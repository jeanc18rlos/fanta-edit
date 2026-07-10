//! In-place WYSIWYG text editing on the Figma canvas.
//!
//! While a session is live, every edit writes the buffer straight into the
//! node's [`TextNode::content`] — a transient, non-undoable preview, exactly
//! like the select tool's drag preview writing transforms through `get_mut`.
//! The canvas renderer then paints the live text with the node's real font,
//! size, alignment, wrapping, and zoom, so the editing view is pixel-identical
//! to the committed result by construction. The only overlay UI is a blinking
//! caret and the selection highlight, whose geometry comes from the same
//! shaped layout the renderer paints (`fanta_render::text_caret_rect` & co.),
//! mapped node-local → world → screen.
//!
//! Committing rewinds the preview to the original data and applies ONE
//! undoable [`Operation::ReplaceData`] (original → final), mirroring the
//! original fanta-app's `state/text_edit.rs` and the select tool's move
//! commit, so a whole typing session undoes in a single step.

use std::ops::Range;
use std::time::{Duration, Instant};

use fanta_doc::{
    Color, Doc, NodeData, NodeId, Operation, Override, OverridePath, TextAlign, TextAutoResize,
    TextNode, TextStyleRun, Transform2D, VAlign, Viewport,
};
use fanta_text::{Caret, Selection, TextBuffer, TextError, TextStyle as EngineTextStyle};
use glam::DVec2;
use gpui::{Subscription, Task};

use crate::instance_text;

/// Half-period of the caret blink, matching the original fanta-app (~2×/sec):
/// the caret is solid for one interval, then hidden for one interval.
pub(crate) const CARET_BLINK_INTERVAL: Duration = Duration::from_millis(530);

/// Phase of the blink after `elapsed` since the last reset: solid for the
/// first [`CARET_BLINK_INTERVAL`], hidden for the next, and so on. Split out so
/// the cadence is unit-testable without a GPUI context.
fn caret_visible_after(elapsed: Duration) -> bool {
    let interval = CARET_BLINK_INTERVAL.as_millis().max(1);
    (elapsed.as_millis() % (interval * 2)) < interval
}

/// The live view-side session: the pure editing model plus the GPUI plumbing
/// (blink repaint task, focus-out commit subscription) that must die with it.
///
/// The blink is **phase-based**, mirroring the original fanta-app: caret
/// visibility is derived from wall-clock time since [`blink_reset_at`] rather
/// than a toggled flag, so it is correct on *every* repaint regardless of
/// whether a scheduled wake landed — a stalled or dropped wake can only slow
/// the blink, never leave the caret stuck hidden. Every edit or caret move
/// resets the phase (via [`Self::reset_blink`]) so the caret is solid the
/// instant you act and only blinks once idle, exactly like Figma.
///
/// [`blink_reset_at`]: Self::blink_reset_at
pub(crate) struct CanvasTextEdit {
    pub(crate) session: TextEditSession,
    /// An inspector field can deliberately take focus while it continues to
    /// edit this session's character selection. In that case the next focus
    /// out must keep the session alive instead of committing it.
    pub(crate) retain_on_next_focus_out: bool,
    /// When the current blink phase last restarted. The caret is solid for the
    /// first [`CARET_BLINK_INTERVAL`] after this instant, then alternates.
    pub(crate) blink_reset_at: Instant,
    /// Bumped on every blink reset so a stale wake task stops itself instead
    /// of fighting the newly scheduled one.
    pub(crate) blink_epoch: usize,
    /// Wakes the view at the next phase boundary so the blink actually
    /// animates; re-arms itself each cycle. Dropping it (session end) stops
    /// the wakeups.
    pub(crate) blink_task: Option<Task<()>>,
    pub(crate) _focus_out_subscription: Subscription,
}

impl CanvasTextEdit {
    pub(crate) fn new(session: TextEditSession, focus_out_subscription: Subscription) -> Self {
        Self {
            session,
            retain_on_next_focus_out: false,
            blink_reset_at: Instant::now(),
            blink_epoch: 0,
            blink_task: None,
            _focus_out_subscription: focus_out_subscription,
        }
    }

    /// Whether the caret is in its visible half-cycle right now — a pure
    /// function of elapsed time, so it is correct on any repaint. Solid for the
    /// first interval after a reset, then alternating.
    pub(crate) fn caret_visible(&self) -> bool {
        caret_visible_after(self.blink_reset_at.elapsed())
    }

    /// Restart the blink phase: the caret shows solid now and the next hide is
    /// a full interval away. Bumps the epoch so any prior wake task retires.
    pub(crate) fn reset_blink(&mut self) -> usize {
        self.blink_reset_at = Instant::now();
        self.blink_epoch += 1;
        self.blink_epoch
    }
}

/// When the edited text lives inside a component instance, the session targets
/// a VIRTUAL clone (no scene node). The edit is persisted as an [`Override`] on
/// the instance, and the caret/selection geometry is projected through the
/// clone's reconstructed world transform rather than a scene lookup.
#[derive(Clone)]
pub(crate) struct InstanceEdit {
    /// The instance scene node whose `overrides` the edit writes.
    pub(crate) instance_id: NodeId,
    /// Def-local path of the text clone (the override's `target_path`).
    pub(crate) def_path: OverridePath,
    /// Absolute world transform of the clone, for caret/selection geometry.
    pub(crate) world: Transform2D,
    /// The instance's pre-edit override vec, restored on rewind before the
    /// undoable commit (mirrors the real-text rewind-then-`ReplaceData` stage).
    pub(crate) base_overrides: Vec<Override>,
}

/// What the properties panel shows for a live text sub-selection: the style
/// at the selection start, with `color_mixed` raised when the selected range
/// spans runs whose glyph colors disagree.
pub(crate) struct SelectionTypography {
    pub(crate) style: EngineTextStyle,
    pub(crate) color_mixed: bool,
}

/// The pure editing model: the live buffer, the directed selection, and the
/// pre-edit node data the commit/rewind path restores. No GPUI types, so the
/// whole editing behavior is testable headlessly.
pub(crate) struct TextEditSession {
    node_id: NodeId,
    /// Full node data at open time for rewind. For an instance session this is
    /// the resolved text CLONE (content/style reflect existing overrides).
    original: TextNode,
    /// `Some` when editing text inside a component instance — the edit becomes
    /// an override rather than a scene write.
    instance: Option<InstanceEdit>,
    /// Live rich text buffer supporting style runs over selections.
    buffer: TextBuffer,
    /// The buffer exactly as built at open. `is_changed` compares the live
    /// buffer against THIS (same construction, same run representation) rather
    /// than re-deriving runs from the node — deriving is lossy for partially
    /// covered `style_runs` (the builder materializes base-style gap runs the
    /// node convention omits), which read as a phantom change and committed a
    /// no-op `ReplaceData` for a session that only opened and closed.
    opened_buffer: TextBuffer,
    /// Directed selection in byte offsets; collapsed (anchor == head) is the
    /// plain caret. The head is the moving end.
    selection: Selection,
    /// IME composition range (byte offsets into `buffer`), when marked text
    /// is pending.
    marked_range: Option<Range<usize>>,
    /// A mouse drag-selection is in flight.
    pub(crate) dragging: bool,
    /// The pointer is currently over the edited node (drives the I-beam).
    pub(crate) pointer_inside: bool,
}

impl TextEditSession {
    pub(crate) fn new(node_id: NodeId, node: &TextNode) -> Self {
        let caret = node.content.len();
        // Build rich buffer from node's base style + any existing style runs.
        let base_style = map_doc_style_to_engine(&node.style);
        let mut buf = if node.content.is_empty() {
            TextBuffer::new()
        } else {
            TextBuffer::from_str(node.content.clone(), base_style.clone())
        };
        for run in &node.style_runs {
            let run_style = map_doc_style_to_engine(&run.style);
            let _ = buf.set_style(run.start..run.end, run_style);
        }
        buf.set_default_style(base_style);
        Self {
            node_id,
            original: node.clone(),
            instance: None,
            opened_buffer: buf.clone(),
            buffer: buf,
            selection: Selection::caret(caret),
            marked_range: None,
            dragging: false,
            pointer_inside: false,
        }
    }

    /// Open a session on text inside a component instance. `target.text` is the
    /// resolved clone; the edit persists as an override on `target.instance_id`.
    pub(crate) fn new_instance(
        target: instance_text::InstanceTextTarget,
        base_overrides: Vec<Override>,
    ) -> Self {
        let mut session = Self::new(target.instance_id, &target.text);
        session.instance = Some(InstanceEdit {
            instance_id: target.instance_id,
            def_path: target.def_path,
            world: target.world,
            base_overrides,
        });
        session
    }

    /// The scene node the session is anchored to: the text node for a real
    /// session, or the wrapping INSTANCE node for an instance session (whose
    /// existence gates the session and whose overrides the edit writes).
    pub(crate) fn node_id(&self) -> NodeId {
        self.node_id
    }

    pub(crate) fn instance(&self) -> Option<&InstanceEdit> {
        self.instance.as_ref()
    }

    /// A text node reflecting the LIVE buffer (content + glyph color), used for
    /// instance-session caret/selection/hit geometry since the clone has no
    /// scene entry. Auto-resizing boxes hug the new glyphs so the geometry
    /// matches what the renderer paints from the previewed override.
    fn live_text(&self) -> TextNode {
        let mut text = self.original.clone();
        text.content = self.buffer.text().to_string();
        text.style = map_engine_style_to_doc(self.buffer.default_style());
        // Instance overrides carry whole-content text + a single glyph color;
        // partial style runs aren't part of the override model.
        text.style_runs.clear();
        hug_auto_resize(&mut text);
        text
    }

    /// The glyph color the buffer currently carries — the value an instance
    /// color override would install. `None` when unchanged from the clone's
    /// original color (so no color override is emitted for a content-only edit).
    fn changed_color(&self) -> Option<Color> {
        let color = self.buffer.default_style().color;
        (color != self.original.style.color).then_some(color)
    }

    pub(crate) fn buffer(&self) -> &str {
        self.buffer.text()
    }

    #[cfg(test)]
    pub(crate) fn text_buffer(&self) -> &fanta_text::TextBuffer {
        &self.buffer
    }

    pub(crate) fn buffer_snapshot(&self) -> TextBuffer {
        self.buffer.clone()
    }

    pub(crate) fn restore_buffer_snapshot(&mut self, buffer: TextBuffer) {
        self.buffer = buffer;
    }

    /// Whether the user changed anything since the session opened. Compared
    /// against [`opened_buffer`](Self::opened_buffer) — the same construction,
    /// so a session that merely opened and closed is exactly equal, even for a
    /// node whose partial `style_runs` the buffer builder had to materialize
    /// into explicit gap runs.
    pub(crate) fn is_changed(&self) -> bool {
        self.buffer != self.opened_buffer
    }

    pub(crate) fn caret(&self) -> usize {
        self.selection.head
    }

    pub(crate) fn selected_range(&self) -> Range<usize> {
        self.selection.range()
    }

    pub(crate) fn selection_reversed(&self) -> bool {
        self.selection.head < self.selection.anchor
    }

    pub(crate) fn marked_range(&self) -> Option<Range<usize>> {
        self.marked_range.clone()
    }

    pub(crate) fn clear_marked(&mut self) {
        self.marked_range = None;
    }

    pub(crate) fn selected_text(&self) -> Option<&str> {
        let range = self.selected_range();
        if range.is_empty() {
            return None;
        }
        self.buffer.text().get(range)
    }

    /// The effective typography over the current selection, for the properties
    /// panel: the style of the run at the selection start, plus a mixed flag
    /// for the glyph color when the selection spans runs that disagree (the
    /// panel shows that as mixed, like Figma). A collapsed caret reports the
    /// typing style at the caret.
    pub(crate) fn selection_typography(&self) -> SelectionTypography {
        let range = self.selected_range();
        if range.is_empty() {
            return SelectionTypography {
                style: self.buffer.style_at(self.caret()).clone(),
                color_mixed: false,
            };
        }
        let mut styles = self
            .buffer
            .runs()
            .iter()
            .filter(|run| run.start < range.end && run.end > range.start)
            .map(|run| &run.style);
        let Some(first) = styles.next() else {
            return SelectionTypography {
                style: self.buffer.default_style().clone(),
                color_mixed: false,
            };
        };
        let color_mixed = styles.any(|style| style.color != first.color);
        SelectionTypography {
            style: first.clone(),
            color_mixed,
        }
    }

    /// Apply a style change to the current selection (or set default for caret).
    /// This enables changing color, font, weight etc. on only the selected text.
    #[cfg(test)]
    pub(crate) fn apply_style_to_selection(&mut self, style: EngineTextStyle) {
        // An instance override carries a single whole-node glyph color, so a
        // style change on an instance session applies to the whole node (the
        // default) rather than a sub-range run — otherwise the selection-run
        // color would have no override to commit into.
        if self.instance.is_some() {
            self.buffer.set_default_style(style);
            return;
        }
        let range = self.selected_range();
        if !range.is_empty() {
            let _ = self.buffer.set_style(range, style);
        } else {
            self.buffer.set_default_style(style);
        }
    }

    /// Patch one typography property without replacing the other properties
    /// carried by each selected run. A selection may span different font
    /// sizes, weights, or colors; inspector edits must preserve those
    /// differences unless that exact property is being changed.
    pub(crate) fn patch_style_to_selection(
        &mut self,
        patch: impl Fn(&mut EngineTextStyle),
    ) -> Result<(), TextError> {
        if self.instance.is_some() {
            let mut style = self.buffer.default_style().clone();
            patch(&mut style);
            self.buffer.set_default_style(style);
            return Ok(());
        }

        let range = self.selected_range();
        if range.is_empty() {
            let mut style = self.buffer.style_at(self.caret()).clone();
            patch(&mut style);
            self.buffer.set_default_style(style);
            return Ok(());
        }

        let selected_runs: Vec<_> = self
            .buffer
            .runs()
            .iter()
            .filter_map(|run| {
                let start = run.start.max(range.start);
                let end = run.end.min(range.end);
                (start < end).then(|| (start..end, run.style.clone()))
            })
            .collect();
        for (run_range, mut style) in selected_runs {
            patch(&mut style);
            self.buffer.set_style(run_range, style)?;
        }
        Ok(())
    }

    // -- selection / caret movement ----------------------------------------

    pub(crate) fn select_all(&mut self) {
        self.selection = Selection::new(0, self.buffer.len());
    }

    pub(crate) fn move_to(&mut self, byte: usize, extend: bool) {
        let byte = self.clamp_offset(byte);
        if extend {
            self.selection.head = byte;
        } else {
            self.selection = Selection::caret(byte);
        }
    }

    /// A mouse press inside the node: place the caret (shift extends from the
    /// existing anchor).
    pub(crate) fn click(&mut self, byte: usize, extend: bool) {
        self.move_to(byte, extend);
    }

    /// Extend the selection toward `byte` while drag-selecting.
    pub(crate) fn drag_to(&mut self, byte: usize) {
        self.selection.head = self.clamp_offset(byte);
    }

    pub(crate) fn select_word_at(&mut self, byte: usize) {
        let range = word_range(self.buffer.text(), self.clamp_offset(byte));
        if range.is_empty() {
            self.selection = Selection::caret(range.start);
        } else {
            self.selection = Selection::new(range.start, range.end);
        }
    }

    pub(crate) fn move_left(&mut self, extend: bool) {
        if !extend && !self.selection.is_collapsed() {
            self.selection = Selection::caret(self.selection.start());
            return;
        }
        let head = Caret::new(self.selection.head)
            .move_left(self.buffer.text())
            .byte;
        self.move_to(head, extend);
    }

    pub(crate) fn move_right(&mut self, extend: bool) {
        if !extend && !self.selection.is_collapsed() {
            self.selection = Selection::caret(self.selection.end());
            return;
        }
        let head = Caret::new(self.selection.head)
            .move_right(self.buffer.text())
            .byte;
        self.move_to(head, extend);
    }

    pub(crate) fn move_line_start(&mut self, extend: bool) {
        let head = Caret::new(self.selection.head)
            .move_home(self.buffer.text())
            .byte;
        self.move_to(head, extend);
    }

    pub(crate) fn move_line_end(&mut self, extend: bool) {
        let head = Caret::new(self.selection.head)
            .move_end(self.buffer.text())
            .byte;
        self.move_to(head, extend);
    }

    // -- edits ---------------------------------------------------------------

    /// Insert at the caret, replacing any selection first.
    pub(crate) fn insert(&mut self, text: &str) {
        self.replace_range(self.selected_range(), text);
    }

    pub(crate) fn replace_range(&mut self, range: Range<usize>, text: &str) {
        let range = self.clamp_range(range);
        if !range.is_empty() {
            let _ = self.buffer.delete_range(range.clone());
        }
        if !text.is_empty() {
            let _ = self.buffer.insert(range.start, text);
        }
        self.selection = Selection::caret(range.start + text.len());
        self.marked_range = None;
    }

    /// Replace `range` with IME composition text, marking it and placing the
    /// selection where the composition asked (offsets relative to the new
    /// text), defaulting to a caret after it.
    pub(crate) fn replace_and_mark(
        &mut self,
        range: Range<usize>,
        text: &str,
        relative_selection: Option<Range<usize>>,
    ) {
        let range = self.clamp_range(range);
        if !range.is_empty() {
            let _ = self.buffer.delete_range(range.clone());
        }
        if !text.is_empty() {
            let _ = self.buffer.insert(range.start, text);
        }
        self.marked_range = (!text.is_empty()).then(|| range.start..range.start + text.len());
        self.selection = match relative_selection {
            Some(relative) => Selection::new(
                self.clamp_offset(range.start + relative.start),
                self.clamp_offset(range.start + relative.end),
            ),
            None => Selection::caret(range.start + text.len()),
        };
    }

    /// Delete the selection, else the grapheme before the caret.
    pub(crate) fn backspace(&mut self) {
        let range = self.selected_range();
        if !range.is_empty() {
            self.replace_range(range, "");
            return;
        }
        let caret = self.selection.head;
        let previous = Caret::new(caret).move_left(self.buffer.text()).byte;
        if previous < caret {
            self.replace_range(previous..caret, "");
        }
    }

    /// Delete the selection, else the grapheme after the caret.
    pub(crate) fn delete_forward(&mut self) {
        let range = self.selected_range();
        if !range.is_empty() {
            self.replace_range(range, "");
            return;
        }
        let caret = self.selection.head;
        let next = Caret::new(caret).move_right(self.buffer.text()).byte;
        if next > caret {
            self.replace_range(caret..next, "");
        }
    }

    // -- UTF-16 bridging for the platform input handler ----------------------

    pub(crate) fn offset_from_utf16(&self, offset_utf16: usize) -> usize {
        offset_from_utf16(self.buffer.text(), offset_utf16)
    }

    pub(crate) fn offset_to_utf16(&self, offset: usize) -> usize {
        offset_to_utf16(self.buffer.text(), offset)
    }

    pub(crate) fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }

    pub(crate) fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    // -- clamping helpers -----------------------------------------------------

    fn clamp_offset(&self, byte: usize) -> usize {
        let mut byte = byte.min(self.buffer.len());
        while byte > 0 && !self.buffer.text().is_char_boundary(byte) {
            byte -= 1;
        }
        byte
    }

    fn clamp_range(&self, range: Range<usize>) -> Range<usize> {
        let start = self.clamp_offset(range.start);
        let end = self.clamp_offset(range.end).max(start);
        start..end
    }
}

/// UTF-16 code-unit offset → UTF-8 byte offset within `text`, clamped to the
/// end. The platform input handler speaks UTF-16; the buffer speaks bytes.
pub(crate) fn offset_from_utf16(text: &str, offset_utf16: usize) -> usize {
    let mut utf8_offset = 0;
    let mut utf16_count = 0;
    for ch in text.chars() {
        if utf16_count >= offset_utf16 {
            break;
        }
        utf16_count += ch.len_utf16();
        utf8_offset += ch.len_utf8();
    }
    utf8_offset
}

/// UTF-8 byte offset → UTF-16 code-unit offset within `text`.
pub(crate) fn offset_to_utf16(text: &str, offset: usize) -> usize {
    let mut utf16_offset = 0;
    let mut utf8_count = 0;
    for ch in text.chars() {
        if utf8_count >= offset {
            break;
        }
        utf8_count += ch.len_utf8();
        utf16_offset += ch.len_utf16();
    }
    utf16_offset
}

/// The word (alphanumeric + `_` run) containing `byte`, for double-click word
/// selection. A non-word position yields an empty range at `byte`.
fn word_range(text: &str, byte: usize) -> Range<usize> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut low = byte.min(text.len());
    let mut high = low;
    while let Some(previous) = text[..low].chars().next_back() {
        if is_word(previous) {
            low -= previous.len_utf8();
        } else {
            break;
        }
    }
    while let Some(next) = text[high..].chars().next() {
        if is_word(next) {
            high += next.len_utf8();
        } else {
            break;
        }
    }
    low..high
}

/// Map fanta_doc::TextStyle -> fanta_text::TextStyle for the live buffer.
fn map_doc_style_to_engine(doc_style: &fanta_doc::TextStyle) -> EngineTextStyle {
    EngineTextStyle {
        font_family: doc_style.font_family.clone(),
        size_px: doc_style.size_px,
        weight: doc_style.weight,
        italic: doc_style.italic,
        underline: doc_style.underline,
        strikethrough: doc_style.strikethrough,
        color: doc_style.color,
        letter_spacing: doc_style.letter_spacing,
        line_height: doc_style.line_height,
        line_height_auto_percent: doc_style.line_height_auto_percent,
        font_variations: doc_style.font_variations.clone(),
    }
}

/// Map back fanta_text::TextStyle -> fanta_doc::TextStyle.
fn map_engine_style_to_doc(engine: &EngineTextStyle) -> fanta_doc::TextStyle {
    fanta_doc::TextStyle {
        font_family: engine.font_family.clone(),
        size_px: engine.size_px,
        weight: engine.weight,
        italic: engine.italic,
        underline: engine.underline,
        strikethrough: engine.strikethrough,
        color: engine.color,
        letter_spacing: engine.letter_spacing,
        line_height: engine.line_height,
        line_height_auto_percent: engine.line_height_auto_percent,
        font_variations: engine.font_variations.clone(),
    }
}

// ---------------------------------------------------------------------------
// Document preview / commit
// ---------------------------------------------------------------------------

/// Push the live buffer into the node's content WITHOUT touching history, so
/// the canvas renders keystrokes as they land. Auto-resizing boxes are
/// re-hugged to the new glyphs (through the renderer's shared shaped-layout
/// cache, so the measure pre-warms the very paragraph the next paint draws).
/// Also pushes the current style runs so partial styling is previewed.
pub(crate) fn apply_preview(doc: &mut Doc, session: &TextEditSession) {
    // Instance session: the edited text is a virtual clone, so the preview is a
    // transient rewrite of the instance's overrides (which re-expands the clone
    // with the new content/color). The caller pairs this with a cache
    // invalidation because a `get_mut` write doesn't bump the scene revision.
    if let Some(instance) = &session.instance {
        let overrides = instance_text::preview_overrides(
            &instance.base_overrides,
            &instance.def_path,
            session.buffer.text(),
            session.changed_color(),
        );
        instance_text::set_overrides_transient(doc, instance.instance_id, overrides);
        return;
    }
    let Some(node) = doc.scene.get_mut(session.node_id) else {
        return;
    };
    let NodeData::Text(text) = &mut node.data else {
        return;
    };
    text.content = session.buffer.text().to_string();
    // Transfer the buffer's default style as the node's base style (for
    // collapsed caret / typing style and overall), plus any explicit runs for
    // partial/selection styling. This ensures color/font changes on a selection
    // (or at caret) are previewed and committed correctly.
    text.style = map_engine_style_to_doc(session.buffer.default_style());
    text.style_runs = session
        .buffer
        .runs()
        .iter()
        .map(|r| TextStyleRun {
            start: r.start,
            end: r.end,
            style: map_engine_style_to_doc(&r.style),
        })
        .collect();
    hug_auto_resize(text);
}

/// Restore the pre-edit content, rich-text styles, and box size so the
/// committing [`Operation::ReplaceData`] drives the scene from its true old
/// state to the final one through the history chokepoint — the same
/// restore-then-apply staging as the select tool's move commit.
pub(crate) fn rewind_preview(doc: &mut Doc, session: &TextEditSession) {
    if let Some(instance) = &session.instance {
        instance_text::set_overrides_transient(
            doc,
            instance.instance_id,
            instance.base_overrides.clone(),
        );
        return;
    }
    let Some(node) = doc.scene.get_mut(session.node_id) else {
        return;
    };
    let NodeData::Text(text) = &mut node.data else {
        return;
    };
    text.content = session.original.content.clone();
    text.local_size = session.original.local_size;
    text.style = session.original.style.clone();
    text.style_runs = session.original.style_runs.clone();
}

/// The single undoable operation committing the session, built against the
/// REWOUND document (so `old` carries the pre-edit text-session fields while
/// keeping concurrent non-glyph property edits). `None` when nothing changed
/// or the node is gone.
pub(crate) fn commit_operation(doc: &Doc, session: &TextEditSession) -> Option<Operation> {
    if !session.is_changed() {
        return None;
    }
    // Instance sessions commit through `commit_ops` (their edit is one or two
    // `SetInstanceOverride`s, not a `ReplaceData`).
    if session.instance.is_some() {
        return None;
    }
    let node = doc.scene.get(session.node_id)?;
    let NodeData::Text(text) = &node.data else {
        return None;
    };
    let mut new_text = text.clone();
    new_text.content = session.buffer.text().to_string();
    // Transfer default + runs for rich text / partial styles in the final
    // undoable ReplaceData (mirrors the preview transfer).
    new_text.style = map_engine_style_to_doc(session.buffer.default_style());
    new_text.style_runs = session
        .buffer
        .runs()
        .iter()
        .map(|r| TextStyleRun {
            start: r.start,
            end: r.end,
            style: map_engine_style_to_doc(&r.style),
        })
        .collect();
    hug_auto_resize(&mut new_text);
    Some(Operation::ReplaceData {
        id: session.node_id,
        old: Box::new(node.data.clone()),
        new: Box::new(NodeData::Text(new_text)),
    })
}

/// The undoable operation(s) committing the session against the REWOUND
/// document. A real-text session yields at most one [`Operation::ReplaceData`];
/// an instance session yields up to two [`Operation::SetInstanceOverride`]s (a
/// text-content override, and a glyph-color override when the color changed).
/// Empty when nothing changed.
pub(crate) fn commit_ops(doc: &Doc, session: &TextEditSession) -> Vec<Operation> {
    if !session.is_changed() {
        return Vec::new();
    }
    let Some(instance) = &session.instance else {
        return commit_operation(doc, session).into_iter().collect();
    };
    let content_changed = session.buffer.text() != session.original.content;
    instance_text::commit_ops(
        doc,
        instance.instance_id,
        &instance.def_path,
        content_changed.then(|| session.buffer.text()),
        session.changed_color(),
    )
}

/// Resize an auto-resizing text box to hug its (non-empty) content, matching
/// Figma: auto-width boxes hug both axes, auto-height boxes hug the height.
fn hug_auto_resize(text: &mut TextNode) {
    if text.content.is_empty() {
        return;
    }
    match text.auto_resize {
        TextAutoResize::WidthAndHeight => {
            let (width, height) = fanta_render::measure_text_node(text);
            if width > 0.0 && height > 0.0 {
                text.local_size = [width, height];
            }
        }
        TextAutoResize::Height => {
            let (_, height) = fanta_render::measure_text_node(text);
            if height > 0.0 {
                text.local_size[1] = height;
            }
        }
        TextAutoResize::None => {}
    }
}

// ---------------------------------------------------------------------------
// Geometry: node-local layout → screen, for the caret/selection overlay
// ---------------------------------------------------------------------------

/// Vertical paint offset the renderer applies for the node's vertical
/// alignment (`draw_text_node` paints the paragraph block Top/Center/Bottom
/// within the box height). Caret and hit-test geometry must shift by the same
/// amount to line up with the painted glyphs.
fn vertical_paint_offset(text: &TextNode) -> f64 {
    let factor = match text.vertical_align {
        VAlign::Top => return 0.0,
        VAlign::Center => 0.5,
        VAlign::Bottom => 1.0,
    };
    let painted_height = if text.content.is_empty() {
        0.0
    } else {
        fanta_render::measure_text_node(text).1
    };
    (text.local_size[1] - painted_height) * factor
}

/// Where an empty node's caret sits horizontally: the alignment edge, since
/// the first typed glyph will appear there.
fn empty_caret_x(text: &TextNode) -> f64 {
    match text.align {
        TextAlign::Left | TextAlign::Justify => 0.0,
        TextAlign::Center => text.local_size[0] * 0.5,
        TextAlign::Right => text.local_size[0],
    }
}

/// Caret geometry projected through an explicit `world` transform (so the
/// instance path can pass a virtual clone's reconstructed transform instead of
/// a scene lookup). See [`caret_screen_segment`] for the scene-node wrapper.
fn caret_segment_core(
    text: &TextNode,
    world: Transform2D,
    byte: usize,
    viewport: &Viewport,
    screen_size: DVec2,
) -> Option<(DVec2, DVec2)> {
    let (x, y, height) = if text.content.is_empty() {
        (
            empty_caret_x(text),
            0.0,
            fanta_render::text_line_height(text),
        )
    } else {
        let [x, y, _, height] = fanta_render::text_caret_rect(text, byte);
        let height = if height > 0.0 {
            height
        } else {
            fanta_render::text_line_height(text)
        };
        (x, y, height)
    };
    let dy = vertical_paint_offset(text);
    let project = |local: DVec2| {
        fanta_canvas::world_to_screen(world.transform_point(local), viewport, screen_size)
    };
    Some((
        project(DVec2::new(x, y + dy)),
        project(DVec2::new(x, y + dy + height)),
    ))
}

/// Selection rects projected through an explicit `world` transform.
fn selection_rects_core(
    text: &TextNode,
    world: Transform2D,
    range: Range<usize>,
    viewport: &Viewport,
    screen_size: DVec2,
) -> Vec<[f64; 4]> {
    if range.is_empty() {
        return Vec::new();
    }
    let dy = vertical_paint_offset(text);
    let project = |x: f64, y: f64| {
        fanta_canvas::world_to_screen(
            world.transform_point(DVec2::new(x, y)),
            viewport,
            screen_size,
        )
    };
    fanta_render::text_selection_rects(text, range.start, range.end)
        .into_iter()
        .map(|[x, y, width, height]| {
            let corners = [
                project(x, y + dy),
                project(x + width, y + dy),
                project(x + width, y + dy + height),
                project(x, y + dy + height),
            ];
            let min_x = corners.iter().map(|c| c.x).fold(f64::INFINITY, f64::min);
            let max_x = corners
                .iter()
                .map(|c| c.x)
                .fold(f64::NEG_INFINITY, f64::max);
            let min_y = corners.iter().map(|c| c.y).fold(f64::INFINITY, f64::min);
            let max_y = corners
                .iter()
                .map(|c| c.y)
                .fold(f64::NEG_INFINITY, f64::max);
            [
                min_x,
                min_y,
                (max_x - min_x).max(0.0),
                (max_y - min_y).max(0.0),
            ]
        })
        .collect()
}

/// Byte offset for a screen point via an explicit `world` transform.
fn byte_at_core(
    text: &TextNode,
    world: Transform2D,
    screen: DVec2,
    viewport: &Viewport,
    screen_size: DVec2,
) -> usize {
    let world_point = fanta_canvas::screen_to_world(screen, viewport, screen_size);
    let local = world.inverse().transform_point(world_point);
    let dy = vertical_paint_offset(text);
    fanta_render::text_hit_test(text, [local.x, local.y - dy])
}

/// Whether `screen` falls within the text box `[0,0]..local_size` transformed
/// by `world` (axis-aligned box of the projected corners).
fn contains_core(
    text: &TextNode,
    world: Transform2D,
    screen: DVec2,
    viewport: &Viewport,
    screen_size: DVec2,
) -> bool {
    let [w, h] = text.local_size;
    let corners = [
        DVec2::new(0.0, 0.0),
        DVec2::new(w, 0.0),
        DVec2::new(w, h),
        DVec2::new(0.0, h),
    ]
    .map(|c| fanta_canvas::world_to_screen(world.transform_point(c), viewport, screen_size));
    let min_x = corners.iter().map(|c| c.x).fold(f64::INFINITY, f64::min);
    let max_x = corners
        .iter()
        .map(|c| c.x)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_y = corners.iter().map(|c| c.y).fold(f64::INFINITY, f64::min);
    let max_y = corners
        .iter()
        .map(|c| c.y)
        .fold(f64::NEG_INFINITY, f64::max);
    screen.x >= min_x && screen.x <= max_x && screen.y >= min_y && screen.y <= max_y
}

/// The caret at `byte` as a screen-space segment `(top, bottom)`, projected
/// through the node's world transform and the viewport — so it lands exactly
/// on the painted glyph edge at any zoom, pan, or node transform.
pub(crate) fn caret_screen_segment(
    doc: &Doc,
    node_id: NodeId,
    byte: usize,
    viewport: &Viewport,
    screen_size: DVec2,
) -> Option<(DVec2, DVec2)> {
    let node = doc.scene.get(node_id)?;
    let NodeData::Text(text) = &node.data else {
        return None;
    };
    let world = doc.scene.world_transform(node_id)?;
    caret_segment_core(text, world, byte, viewport, screen_size)
}

fn baseline_segment_core(
    text: &TextNode,
    world: Transform2D,
    viewport: &Viewport,
    screen_size: DVec2,
) -> (DVec2, DVec2) {
    let baseline = fanta_render::text_first_baseline(text);
    let width = text.local_size[0].max(1.0);
    let project = |local: DVec2| {
        fanta_canvas::world_to_screen(world.transform_point(local), viewport, screen_size)
    };
    (
        project(DVec2::new(0.0, baseline)),
        project(DVec2::new(width, baseline)),
    )
}

pub(crate) fn baseline_screen_segment(
    doc: &Doc,
    node_id: NodeId,
    viewport: &Viewport,
    screen_size: DVec2,
) -> Option<(DVec2, DVec2)> {
    let node = doc.scene.get(node_id)?;
    let NodeData::Text(text) = &node.data else {
        return None;
    };
    let world = doc.scene.world_transform(node_id)?;
    Some(baseline_segment_core(text, world, viewport, screen_size))
}

/// Selection highlight as screen-space rects `[x, y, w, h]`. Axis-aligned:
/// exact under translate + scale, a bounding box under rotation.
pub(crate) fn selection_screen_rects(
    doc: &Doc,
    node_id: NodeId,
    range: Range<usize>,
    viewport: &Viewport,
    screen_size: DVec2,
) -> Vec<[f64; 4]> {
    let Some(node) = doc.scene.get(node_id) else {
        return Vec::new();
    };
    let NodeData::Text(text) = &node.data else {
        return Vec::new();
    };
    let Some(world) = doc.scene.world_transform(node_id) else {
        return Vec::new();
    };
    selection_rects_core(text, world, range, viewport, screen_size)
}

/// Map a screen point to a byte offset in the node's text — what turns a
/// click into a caret. Uses the renderer's shaped layout, so the hit-test
/// agrees with the painted glyphs.
pub(crate) fn byte_at_screen(
    doc: &Doc,
    node_id: NodeId,
    screen: DVec2,
    viewport: &Viewport,
    screen_size: DVec2,
) -> Option<usize> {
    let node = doc.scene.get(node_id)?;
    let NodeData::Text(text) = &node.data else {
        return None;
    };
    let world = doc.scene.world_transform(node_id)?;
    Some(byte_at_core(text, world, screen, viewport, screen_size))
}

/// Whether `screen` falls within the node's world bounds — distinguishes a
/// click *inside* the edited text (place caret) from a click-away (commit).
pub(crate) fn node_contains_screen(
    doc: &Doc,
    node_id: NodeId,
    screen: DVec2,
    viewport: &Viewport,
    screen_size: DVec2,
) -> bool {
    let Some(bounds) = doc.scene.world_bounds(node_id) else {
        return false;
    };
    let a = fanta_canvas::world_to_screen(
        DVec2::new(bounds.min_x, bounds.min_y),
        viewport,
        screen_size,
    );
    let b = fanta_canvas::world_to_screen(
        DVec2::new(bounds.max_x, bounds.max_y),
        viewport,
        screen_size,
    );
    screen.x >= a.x.min(b.x)
        && screen.x <= a.x.max(b.x)
        && screen.y >= a.y.min(b.y)
        && screen.y <= a.y.max(b.y)
}

// ---------------------------------------------------------------------------
// Session-dispatched geometry: real sessions read the scene node; instance
// sessions project the live buffer through the clone's reconstructed transform.
// ---------------------------------------------------------------------------

pub(crate) fn session_caret_segment(
    doc: &Doc,
    session: &TextEditSession,
    byte: usize,
    viewport: &Viewport,
    screen_size: DVec2,
) -> Option<(DVec2, DVec2)> {
    match session.instance() {
        Some(inst) => caret_segment_core(
            &session.live_text(),
            inst.world,
            byte,
            viewport,
            screen_size,
        ),
        None => caret_screen_segment(doc, session.node_id(), byte, viewport, screen_size),
    }
}

pub(crate) fn session_baseline_segment(
    doc: &Doc,
    session: &TextEditSession,
    viewport: &Viewport,
    screen_size: DVec2,
) -> Option<(DVec2, DVec2)> {
    match session.instance() {
        Some(instance) => Some(baseline_segment_core(
            &session.live_text(),
            instance.world,
            viewport,
            screen_size,
        )),
        None => baseline_screen_segment(doc, session.node_id(), viewport, screen_size),
    }
}

pub(crate) fn session_selection_rects(
    doc: &Doc,
    session: &TextEditSession,
    range: Range<usize>,
    viewport: &Viewport,
    screen_size: DVec2,
) -> Vec<[f64; 4]> {
    match session.instance() {
        Some(inst) => selection_rects_core(
            &session.live_text(),
            inst.world,
            range,
            viewport,
            screen_size,
        ),
        None => selection_screen_rects(doc, session.node_id(), range, viewport, screen_size),
    }
}

pub(crate) fn session_byte_at_screen(
    doc: &Doc,
    session: &TextEditSession,
    screen: DVec2,
    viewport: &Viewport,
    screen_size: DVec2,
) -> Option<usize> {
    match session.instance() {
        Some(inst) => Some(byte_at_core(
            &session.live_text(),
            inst.world,
            screen,
            viewport,
            screen_size,
        )),
        None => byte_at_screen(doc, session.node_id(), screen, viewport, screen_size),
    }
}

pub(crate) fn session_contains_screen(
    doc: &Doc,
    session: &TextEditSession,
    screen: DVec2,
    viewport: &Viewport,
    screen_size: DVec2,
) -> bool {
    match session.instance() {
        Some(inst) => contains_core(
            &session.live_text(),
            inst.world,
            screen,
            viewport,
            screen_size,
        ),
        None => node_contains_screen(doc, session.node_id(), screen, viewport, screen_size),
    }
}

/// The byte the caret lands on when moved one visual line up or down, via
/// caret-rect + hit-test through the shaped layout (so wrapped lines work).
/// On the first/last line it clamps to the text start/end, the standard
/// editor behavior. `None` when the node is gone or empty.
pub(crate) fn vertical_move_target(
    doc: &Doc,
    node_id: NodeId,
    byte: usize,
    down: bool,
) -> Option<usize> {
    let node = doc.scene.get(node_id)?;
    let NodeData::Text(text) = &node.data else {
        return None;
    };
    vertical_move_target_core(text, byte, down)
}

/// [`vertical_move_target`] dispatched by session: an instance session probes
/// the live buffer's text (its clone has no scene node).
pub(crate) fn session_vertical_move_target(
    doc: &Doc,
    session: &TextEditSession,
    byte: usize,
    down: bool,
) -> Option<usize> {
    match session.instance() {
        Some(_) => vertical_move_target_core(&session.live_text(), byte, down),
        None => vertical_move_target(doc, session.node_id(), byte, down),
    }
}

fn vertical_move_target_core(text: &TextNode, byte: usize, down: bool) -> Option<usize> {
    if text.content.is_empty() {
        return None;
    }
    let [x, y, _, height] = fanta_render::text_caret_rect(text, byte);
    let height = if height > 0.0 {
        height
    } else {
        fanta_render::text_line_height(text)
    };
    let probe_y = if down {
        y + height * 1.5
    } else {
        y - height * 0.5
    };
    let target = fanta_render::text_hit_test(text, [x, probe_y]);
    if target == byte {
        return Some(if down { text.content.len() } else { 0 });
    }
    Some(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{CanvasNode, Transform2D};

    fn doc_with_text_node(content: &str) -> (Doc, NodeId) {
        let mut doc = Doc::new();
        let node = CanvasNode::new(NodeData::Text(TextNode::new(content, 200.0, 40.0)));
        let id = node.id;
        doc.apply(Operation::create_node(node))
            .expect("creating a text node in an empty doc");
        (doc, id)
    }

    fn text_node(doc: &Doc, id: NodeId) -> TextNode {
        match &doc.scene.get(id).expect("node exists").data {
            NodeData::Text(text) => text.clone(),
            _ => panic!("expected a text node"),
        }
    }

    fn content(doc: &Doc, id: NodeId) -> String {
        text_node(doc, id).content
    }

    /// A doc with a component master (group root + one text child at local
    /// offset (8,12), box 160x40) and one instance of it placed with
    /// `instance_transform`; returns the doc and the instance node id.
    fn doc_with_instance_transform(
        master_text: &str,
        instance_transform: Transform2D,
    ) -> (Doc, NodeId) {
        use fanta_doc::{ComponentDef, ComponentId, GroupNode, InstanceNode};
        use std::collections::BTreeMap;
        let mut doc = Doc::new();

        let mut root = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let root_id = root.id;
        root.transform = Transform2D::translation(2000.0, 0.0);
        doc.apply(Operation::create_node(root)).expect("root");

        let mut text = CanvasNode::new(NodeData::Text(TextNode::new(master_text, 160.0, 40.0)));
        text.parent = Some(root_id);
        text.transform = Transform2D::translation(8.0, 12.0);
        doc.apply(Operation::create_node(text)).expect("text");

        let component = ComponentId::new();
        doc.components
            .defs
            .insert(component, ComponentDef::new(component, root_id, "Card"));

        let mut instance = CanvasNode::new(NodeData::Instance(InstanceNode {
            component,
            overrides: Vec::new(),
            prop_values: BTreeMap::new(),
            derived: Vec::new(),
            local_size: [160.0, 40.0],
        }));
        let instance_id = instance.id;
        instance.transform = instance_transform;
        doc.apply(Operation::create_node(instance))
            .expect("instance");
        doc.add_page(instance_id);
        (doc, instance_id)
    }

    /// A doc with a component master (group root + one text child) and one
    /// instance of it; returns the doc and the instance node id.
    fn doc_with_instance(master_text: &str) -> (Doc, NodeId) {
        doc_with_instance_transform(master_text, Transform2D::translation(400.0, 200.0))
    }

    fn instance_resolved_text(doc: &Doc, instance_id: NodeId) -> String {
        instance_text::text_target_at(doc, instance_id, DVec2::new(420.0, 220.0))
            .expect("a text clone inside the instance")
            .text
            .content
    }

    /// The full instance-text-editing flow the view drives: open a session on a
    /// clone, type, preview (transient override), rewind, then commit the
    /// undoable `SetInstanceOverride` — and undo back to the master text.
    #[test]
    fn instance_session_edits_content_through_an_override() {
        let (mut doc, instance_id) = doc_with_instance("Master");
        let target = instance_text::text_target_at(&doc, instance_id, DVec2::new(420.0, 220.0))
            .expect("hit the text clone");
        let base = instance_text::snapshot_overrides(&doc, instance_id);
        let mut session = TextEditSession::new_instance(target, base);
        assert!(session.instance().is_some());
        assert!(!session.is_changed());

        session.select_all();
        session.insert("Edited");
        assert!(session.is_changed());

        // Preview writes a transient override; the clone now resolves to "Edited".
        apply_preview(&mut doc, &session);
        assert_eq!(instance_resolved_text(&doc, instance_id), "Edited");

        // Rewind restores the master text (no override yet).
        rewind_preview(&mut doc, &session);
        assert_eq!(instance_resolved_text(&doc, instance_id), "Master");

        // Commit the undoable override op(s).
        let undo_before = doc.history.undo_depth();
        let ops = commit_ops(&doc, &session);
        assert_eq!(ops.len(), 1, "content-only edit is one override op");
        for op in ops {
            doc.apply(op).expect("apply commit");
        }
        assert_eq!(instance_resolved_text(&doc, instance_id), "Edited");
        assert_eq!(doc.history.undo_depth(), undo_before + 1);

        // Undo returns to the master text.
        assert!(doc.undo().expect("undo"));
        assert_eq!(instance_resolved_text(&doc, instance_id), "Master");
    }

    /// Changing the glyph color of instance text (whole-node, as the override
    /// model requires) previews and commits as a single color override, and
    /// undoes back to the master color.
    #[test]
    fn instance_session_edits_color_through_an_override() {
        let (mut doc, instance_id) = doc_with_instance("Label");
        let target = instance_text::text_target_at(&doc, instance_id, DVec2::new(420.0, 220.0))
            .expect("hit the text clone");
        let base = instance_text::snapshot_overrides(&doc, instance_id);
        let mut session = TextEditSession::new_instance(target, base);

        let red = Color::rgb(255, 0, 0);
        let mut style = session.text_buffer().style_at(session.caret()).clone();
        style.color = red;
        session.apply_style_to_selection(style);
        assert_eq!(session.changed_color(), Some(red));

        apply_preview(&mut doc, &session);
        let previewed = instance_text::text_target_at(&doc, instance_id, DVec2::new(420.0, 220.0))
            .expect("clone")
            .text;
        assert_eq!(previewed.content, "Label", "content unchanged");
        assert_eq!(previewed.style.color, red);

        rewind_preview(&mut doc, &session);
        let ops = commit_ops(&doc, &session);
        assert_eq!(ops.len(), 1, "color-only edit is one override op");
        for op in ops {
            doc.apply(op).expect("apply");
        }
        let committed = instance_text::text_target_at(&doc, instance_id, DVec2::new(420.0, 220.0))
            .expect("clone")
            .text;
        assert_eq!(committed.style.color, red);

        assert!(doc.undo().expect("undo"));
        let reverted = instance_text::text_target_at(&doc, instance_id, DVec2::new(420.0, 220.0))
            .expect("clone")
            .text;
        assert_ne!(reverted.style.color, red, "undo restores the master color");
    }

    /// Deleting the instance node out from under a live session must make the
    /// rewind and commit no-ops rather than panics — the view drops such
    /// sessions eagerly, but a commit can race the deletion (e.g. a focus-out
    /// commit after an undo removed the instance).
    #[test]
    fn commit_on_a_deleted_instance_is_empty() {
        let (mut doc, instance_id) = doc_with_instance("Master");
        let target = instance_text::text_target_at(&doc, instance_id, DVec2::new(420.0, 220.0))
            .expect("hit the text clone");
        let base = instance_text::snapshot_overrides(&doc, instance_id);
        let mut session = TextEditSession::new_instance(target, base);
        session.select_all();
        session.insert("Edited");
        assert!(session.is_changed());

        let snapshot = vec![doc.scene.get(instance_id).expect("instance").clone()];
        doc.apply(Operation::DeleteSubtree { snapshot })
            .expect("delete the instance");
        assert!(doc.scene.get(instance_id).is_none());

        rewind_preview(&mut doc, &session);
        assert!(
            commit_ops(&doc, &session).is_empty(),
            "a session whose instance is gone has nothing to commit"
        );
    }

    /// The same race for a plain text session: the node vanished, so the
    /// commit yields nothing instead of a `ReplaceData` against a ghost.
    #[test]
    fn commit_on_a_deleted_text_node_is_empty() {
        let (mut doc, id) = doc_with_text_node("hello");
        let mut session = TextEditSession::new(id, &text_node(&doc, id));
        session.insert("!");
        assert!(session.is_changed());

        let snapshot = vec![doc.scene.get(id).expect("node").clone()];
        doc.apply(Operation::DeleteSubtree { snapshot })
            .expect("delete the text node");

        rewind_preview(&mut doc, &session);
        assert!(commit_operation(&doc, &session).is_none());
        assert!(commit_ops(&doc, &session).is_empty());
    }

    /// `session_contains_screen` on an instance whose transform includes a
    /// rotation must project the clone's box through the full transform (the
    /// projected-corner bounding box), not assume an axis-aligned placement.
    #[test]
    fn rotated_instance_containment_agrees_with_obvious_points() {
        let instance_transform = Transform2D::rotation(std::f64::consts::FRAC_PI_4)
            .then(&Transform2D::translation(400.0, 200.0));
        let (doc, instance_id) = doc_with_instance_transform("Spin", instance_transform);

        // The clone's world transform: text-local offset under the rotated
        // instance placement (the master root's transform is suppressed).
        let clone_world = Transform2D::translation(8.0, 12.0).then(&instance_transform);
        let center_world = clone_world.transform_point(DVec2::new(80.0, 20.0));

        let target = instance_text::text_target_at(&doc, instance_id, center_world)
            .expect("hit the rotated text clone at its center");
        assert!(
            (target.world.transform_point(DVec2::new(80.0, 20.0)) - center_world).length() < 1e-6,
            "the resolved clone transform matches the composed expectation"
        );
        let base = instance_text::snapshot_overrides(&doc, instance_id);
        let session = TextEditSession::new_instance(target, base);

        let viewport = Viewport {
            center: [0.0, 0.0],
            zoom: 1.0,
        };
        let screen_size = DVec2::new(800.0, 600.0);
        let to_screen = |world: DVec2| fanta_canvas::world_to_screen(world, &viewport, screen_size);

        for local in [
            DVec2::new(80.0, 20.0),
            DVec2::new(20.0, 20.0),
            DVec2::new(140.0, 20.0),
        ] {
            let screen = to_screen(clone_world.transform_point(local));
            assert!(
                session_contains_screen(&doc, &session, screen, &viewport, screen_size),
                "point at clone-local {local:?} must be inside"
            );
        }
        for local in [DVec2::new(-200.0, -200.0), DVec2::new(400.0, 300.0)] {
            let screen = to_screen(clone_world.transform_point(local));
            assert!(
                !session_contains_screen(&doc, &session, screen, &viewport, screen_size),
                "point at clone-local {local:?} must be outside"
            );
        }
        // The rotation took effect: a point the UNROTATED box would contain
        // (near its far right edge, world ≈ (560, 216)) lies outside the
        // rotated footprint's projected bounds (x ≤ ~510).
        let unrotated_reach = to_screen(DVec2::new(560.0, 216.0));
        assert!(!session_contains_screen(
            &doc,
            &session,
            unrotated_reach,
            &viewport,
            screen_size
        ));
    }

    /// Mixed typography in one paragraph: a size + family patch on a selection
    /// commits as a style run over just that range (the panel's font-size /
    /// family / line-height / letter-spacing fields route here during a
    /// session), while the rest keeps the base style.
    #[test]
    fn mixed_typography_runs_commit_per_run() {
        let (mut doc, id) = doc_with_text_node("Hello world");
        let base_size = text_node(&doc, id).style.size_px;
        let mut session = TextEditSession::new(id, &text_node(&doc, id));
        session.move_to(6, false);
        session.move_to(11, true);

        let mut styled = session.text_buffer().style_at(6).clone();
        styled.size_px = 40.0;
        styled.font_family = "Menlo".to_string();
        session.apply_style_to_selection(styled);

        apply_preview(&mut doc, &session);
        rewind_preview(&mut doc, &session);
        let ops = commit_ops(&doc, &session);
        assert!(!ops.is_empty());
        for op in ops {
            doc.apply(op).expect("apply mixed typography commit");
        }

        let committed = text_node(&doc, id);
        assert_eq!(committed.style.size_px, base_size, "base style untouched");
        let styled_run = committed
            .style_runs
            .iter()
            .find(|run| run.start == 6 && run.end == 11)
            .expect("the selection produced a dedicated run");
        assert_eq!(styled_run.style.size_px, 40.0);
        assert_eq!(styled_run.style.font_family, "Menlo");
        let prefix_run = committed
            .style_runs
            .iter()
            .find(|run| run.start == 0)
            .expect("prefix run");
        assert_eq!(prefix_run.style.size_px, base_size);
    }

    /// The panel's sub-selection binding: a selection inside a colored run
    /// reports that run's color, a selection spanning disagreeing runs raises
    /// `color_mixed`, and a collapsed caret reports the typing style.
    #[test]
    fn selection_typography_tracks_the_selected_runs() {
        let (doc, id) = doc_with_text_node("yyyyyyyy");
        let mut session = TextEditSession::new(id, &text_node(&doc, id));
        let red = fanta_doc::Color::rgb(255, 0, 0);
        session.move_to(2, false);
        session.move_to(5, true);
        let mut styled = session.text_buffer().style_at(2).clone();
        styled.color = red;
        session.apply_style_to_selection(styled);

        // Inside the red run: red, not mixed.
        session.move_to(3, false);
        session.move_to(4, true);
        let inside = session.selection_typography();
        assert_eq!(inside.style.color, red);
        assert!(!inside.color_mixed);

        // Spanning black + red runs: mixed.
        session.move_to(0, false);
        session.move_to(6, true);
        let spanning = session.selection_typography();
        assert!(spanning.color_mixed);

        // Collapsed caret in the black prefix: black typing style, not mixed.
        session.move_to(1, false);
        let caret = session.selection_typography();
        assert_ne!(caret.style.color, red);
        assert!(!caret.color_mixed);
    }

    #[test]
    fn successive_selection_patches_preserve_prior_and_mixed_properties() {
        let (doc, id) = doc_with_text_node("abcdef");
        let mut session = TextEditSession::new(id, &text_node(&doc, id));
        let red = fanta_doc::Color::rgb(255, 0, 0);

        // A forward selection whose moving edge sits at a run seam used to
        // sample the suffix style on every action. Changing size after color
        // therefore restored the old color.
        session.move_to(1, false);
        session.move_to(3, true);
        session
            .patch_style_to_selection(|style| style.color = red)
            .expect("patching selection color");
        session
            .patch_style_to_selection(|style| style.size_px = 40.0)
            .expect("patching selection size");
        session
            .patch_style_to_selection(|style| style.strikethrough = true)
            .expect("patching selection decoration");

        let selected = session.text_buffer().style_at(1);
        assert_eq!(selected.color, red);
        assert_eq!(selected.size_px, 40.0);
        assert!(selected.strikethrough);
        assert_ne!(session.text_buffer().style_at(4).color, red);

        // Patching one field across mixed runs must not flatten fields the
        // user did not touch.
        session.move_to(3, false);
        session.move_to(5, true);
        session
            .patch_style_to_selection(|style| style.size_px = 18.0)
            .expect("creating a second size run");
        session.move_to(1, false);
        session.move_to(5, true);
        session
            .patch_style_to_selection(|style| style.color = red)
            .expect("coloring mixed-size runs");
        assert_eq!(session.text_buffer().style_at(1).size_px, 40.0);
        assert_eq!(session.text_buffer().style_at(3).size_px, 18.0);
        assert_eq!(session.text_buffer().style_at(1).color, red);
        assert_eq!(session.text_buffer().style_at(3).color, red);
    }

    /// A net-zero instance edit (type then delete back to the original) commits
    /// nothing — no stray override is written.
    #[test]
    fn unchanged_instance_session_commits_nothing() {
        let (doc, instance_id) = doc_with_instance("Hello");
        let target = instance_text::text_target_at(&doc, instance_id, DVec2::new(420.0, 220.0))
            .expect("hit the text clone");
        let base = instance_text::snapshot_overrides(&doc, instance_id);
        let mut session = TextEditSession::new_instance(target, base);
        session.move_to(session.buffer().len(), false);
        session.insert("!");
        session.backspace();
        assert!(!session.is_changed());
        assert!(commit_ops(&doc, &session).is_empty());
    }

    #[test]
    fn session_opens_with_caret_at_end() {
        let (doc, id) = doc_with_text_node("hello");
        let session = TextEditSession::new(id, &text_node(&doc, id));
        assert_eq!(session.buffer(), "hello");
        assert_eq!(session.caret(), 5);
        assert!(session.selected_range().is_empty());
        assert!(!session.is_changed());
    }

    #[test]
    fn insert_replaces_the_selection() {
        let (doc, id) = doc_with_text_node("hello");
        let mut session = TextEditSession::new(id, &text_node(&doc, id));
        session.select_all();
        session.insert("y");
        assert_eq!(session.buffer(), "y");
        assert_eq!(session.caret(), 1);
        assert!(session.selected_range().is_empty());
    }

    #[test]
    fn backspace_and_delete_are_grapheme_aware() {
        let (doc, id) = doc_with_text_node("a🦀b");
        let mut session = TextEditSession::new(id, &text_node(&doc, id));
        session.move_left(false); // before 'b'
        session.backspace(); // removes the whole 4-byte crab
        assert_eq!(session.buffer(), "ab");
        session.move_to(0, false);
        session.delete_forward();
        assert_eq!(session.buffer(), "b");
    }

    #[test]
    fn shift_movement_extends_and_collapse_lands_on_the_edge() {
        let (doc, id) = doc_with_text_node("hello");
        let mut session = TextEditSession::new(id, &text_node(&doc, id));
        session.move_left(true);
        session.move_left(true);
        assert_eq!(session.selected_text(), Some("lo"));
        assert!(session.selection_reversed());
        session.move_right(false); // collapse to the right edge
        assert_eq!(session.caret(), 5);
        assert!(session.selected_range().is_empty());
    }

    #[test]
    fn line_start_and_end_respect_hard_newlines() {
        let (doc, id) = doc_with_text_node("ab\ncde");
        let mut session = TextEditSession::new(id, &text_node(&doc, id));
        session.move_to(4, false);
        session.move_line_start(false);
        assert_eq!(session.caret(), 3);
        session.move_line_end(false);
        assert_eq!(session.caret(), 6);
    }

    #[test]
    fn double_click_selects_the_word() {
        let (doc, id) = doc_with_text_node("hello  world");
        let mut session = TextEditSession::new(id, &text_node(&doc, id));
        session.select_word_at(9);
        assert_eq!(session.selected_text(), Some("world"));
        // At a word edge the adjacent word is selected.
        session.select_word_at(5);
        assert_eq!(session.selected_text(), Some("hello"));
        // Between the two spaces: caret placed, nothing selected.
        session.select_word_at(6);
        assert!(session.selected_range().is_empty());
        assert_eq!(session.caret(), 6);
    }

    #[test]
    fn marked_text_replaces_and_remarks() {
        let (doc, id) = doc_with_text_node("");
        let mut session = TextEditSession::new(id, &text_node(&doc, id));
        session.replace_and_mark(0..0, "ni", None);
        assert_eq!(session.marked_range(), Some(0..2));
        session.replace_and_mark(0..2, "に", None);
        assert_eq!(session.buffer(), "に");
        assert_eq!(session.marked_range(), Some(0..3));
        session.replace_range(0..3, "に");
        assert_eq!(session.marked_range(), None);
        assert_eq!(session.caret(), 3);
    }

    #[test]
    fn utf16_offsets_round_trip_through_wide_characters() {
        let (doc, id) = doc_with_text_node("a🦀b");
        let session = TextEditSession::new(id, &text_node(&doc, id));
        // '🦀' is 4 UTF-8 bytes and 2 UTF-16 units.
        assert_eq!(session.offset_to_utf16(5), 3);
        assert_eq!(session.offset_from_utf16(3), 5);
        assert_eq!(session.range_from_utf16(&(1..3)), 1..5);
        assert_eq!(session.range_to_utf16(&(1..5)), 1..3);
    }

    #[test]
    fn preview_writes_the_scene_and_rewind_restores_it() {
        let (mut doc, id) = doc_with_text_node("hello");
        let mut session = TextEditSession::new(id, &text_node(&doc, id));
        session.insert("!");
        apply_preview(&mut doc, &session);
        assert_eq!(content(&doc, id), "hello!");
        rewind_preview(&mut doc, &session);
        assert_eq!(content(&doc, id), "hello");
    }

    #[test]
    fn commit_is_one_undoable_step_restoring_the_pre_edit_text() {
        let (mut doc, id) = doc_with_text_node("hello");
        let mut session = TextEditSession::new(id, &text_node(&doc, id));
        session.select_all();
        session.insert("world");
        session.insert("!");
        apply_preview(&mut doc, &session);
        assert_eq!(content(&doc, id), "world!");

        let undo_depth_before = doc.history.undo_depth();
        rewind_preview(&mut doc, &session);
        let operation = commit_operation(&doc, &session).expect("content changed");
        doc.apply(operation).expect("commit applies");
        assert_eq!(content(&doc, id), "world!");
        assert_eq!(doc.history.undo_depth(), undo_depth_before + 1);

        assert!(doc.undo().expect("undo succeeds"));
        assert_eq!(content(&doc, id), "hello");
        assert_eq!(doc.history.undo_depth(), undo_depth_before);

        assert!(doc.redo().expect("redo succeeds"));
        assert_eq!(content(&doc, id), "world!");
    }

    /// A node whose `style_runs` only partially cover the content (the doc
    /// convention leaves uncovered spans on the base style) opens into a buffer
    /// with materialized gap runs. Merely opening and closing such a session
    /// must not read as a change — the false-positive committed a no-op
    /// `ReplaceData` and polluted undo history.
    #[test]
    fn untouched_partially_styled_session_commits_nothing() {
        let (mut doc, id) = doc_with_text_node("Hello world");
        {
            let node = doc.scene.get_mut(id).expect("node");
            let NodeData::Text(text) = &mut node.data else {
                panic!("expected text");
            };
            let mut styled = text.style.clone();
            styled.color = fanta_doc::Color::rgb(255, 0, 0);
            text.style_runs.push(TextStyleRun {
                start: 6,
                end: 11,
                style: styled,
            });
        }
        let session = TextEditSession::new(id, &text_node(&doc, id));
        assert!(!session.is_changed(), "an untouched session is unchanged");
        assert!(commit_ops(&doc, &session).is_empty());
    }

    #[test]
    fn unchanged_session_commits_nothing() {
        let (mut doc, id) = doc_with_text_node("hello");
        let mut session = TextEditSession::new(id, &text_node(&doc, id));
        session.insert("!");
        session.backspace();
        apply_preview(&mut doc, &session);
        rewind_preview(&mut doc, &session);
        assert!(commit_operation(&doc, &session).is_none());
        assert_eq!(content(&doc, id), "hello");
    }

    #[test]
    fn preview_hugs_auto_width_boxes_to_the_new_content() {
        let mut doc = Doc::new();
        let mut text = TextNode::new("Hi", 140.0, 28.0);
        text.auto_resize = TextAutoResize::WidthAndHeight;
        let node = CanvasNode::new(NodeData::Text(text));
        let id = node.id;
        doc.apply(Operation::create_node(node))
            .expect("creating a text node");

        let mut session = TextEditSession::new(id, &text_node(&doc, id));
        session.move_to(session.buffer().len(), false);
        session.insert(" there, canvas");
        apply_preview(&mut doc, &session);
        let hugged = text_node(&doc, id);
        let (expected_width, expected_height) = fanta_render::measure_text_node(&hugged);
        assert!(expected_width > 0.0);
        assert_eq!(hugged.local_size, [expected_width, expected_height]);

        rewind_preview(&mut doc, &session);
        assert_eq!(text_node(&doc, id).local_size, [140.0, 28.0]);
    }

    #[test]
    fn caret_geometry_matches_the_fanta_text_layout() {
        let mut doc = Doc::new();
        let text = TextNode::new("Hi", 200.0, 40.0);
        let style = text.style.clone();
        let mut node = CanvasNode::new(NodeData::Text(text));
        node.transform = Transform2D::translation(10.0, 20.0);
        let id = node.id;
        doc.apply(Operation::create_node(node))
            .expect("creating a text node");

        // Independent expectation straight from fanta-text — the same engine
        // fanta-render shapes with — at the node's wrap width (fixed box →
        // local width) and alignment (left).
        let engine_style = fanta_text::TextStyle {
            font_family: style.font_family.clone(),
            size_px: style.size_px,
            weight: style.weight,
            italic: style.italic,
            underline: style.underline,
            strikethrough: style.strikethrough,
            color: style.color,
            letter_spacing: style.letter_spacing,
            line_height: style.line_height,
            line_height_auto_percent: style.line_height_auto_percent,
            font_variations: style.font_variations,
        };
        let buffer = fanta_text::TextBuffer::from_str("Hi", engine_style);
        let layout = fanta_text::LayoutEngine::new().layout(&buffer, 200.0);
        let [local_x, local_y, _, local_height] = layout.caret_rect(1);
        assert!(local_height > 0.0);

        let viewport = Viewport {
            center: [0.0, 0.0],
            zoom: 2.0,
        };
        let screen_size = DVec2::new(800.0, 600.0);
        let (top, bottom) = caret_screen_segment(&doc, id, 1, &viewport, screen_size)
            .expect("caret geometry for a live text node");

        // world = node translation + local caret; screen = world*zoom + half.
        let expected_top = DVec2::new(
            (10.0 + local_x) * 2.0 + 400.0,
            (20.0 + local_y) * 2.0 + 300.0,
        );
        let expected_bottom = DVec2::new(
            expected_top.x,
            (20.0 + local_y + local_height) * 2.0 + 300.0,
        );
        assert!(
            (top - expected_top).length() < 1e-6,
            "top {top:?} vs {expected_top:?}"
        );
        assert!(
            (bottom - expected_bottom).length() < 1e-6,
            "bottom {bottom:?} vs {expected_bottom:?}"
        );
    }

    #[test]
    fn baseline_segment_uses_text_width_and_full_node_transform() {
        let mut doc = Doc::new();
        let mut text = TextNode::new("Guide", 120.0, 48.0);
        text.style.size_px = 18.0;
        let baseline = fanta_render::text_first_baseline(&text);
        let mut node = CanvasNode::new(NodeData::Text(text));
        node.transform = Transform2D::translation(15.0, -8.0);
        let id = node.id;
        doc.apply(Operation::create_node(node))
            .expect("creating a text node");
        let viewport = Viewport {
            center: [0.0, 0.0],
            zoom: 2.0,
        };
        let screen_size = DVec2::new(800.0, 600.0);

        let (start, end) = baseline_screen_segment(&doc, id, &viewport, screen_size)
            .expect("text baseline geometry");

        assert!((start.x - 430.0).abs() < 1e-6);
        assert!((end.x - 670.0).abs() < 1e-6);
        let expected_y = 300.0 + (-8.0 + baseline) * 2.0;
        assert!((start.y - expected_y).abs() < 1e-6);
        assert!((end.y - expected_y).abs() < 1e-6);
    }

    #[test]
    fn click_hit_test_and_caret_agree() {
        let (doc, id) = doc_with_text_node("Hello world");
        let viewport = Viewport {
            center: [0.0, 0.0],
            zoom: 1.0,
        };
        let screen_size = DVec2::new(800.0, 600.0);
        // Project the caret after "Hello" to the screen, then hit-test that
        // point back: it must land on the same byte.
        let (top, bottom) =
            caret_screen_segment(&doc, id, 5, &viewport, screen_size).expect("caret geometry");
        let probe = DVec2::new(top.x, (top.y + bottom.y) * 0.5);
        let byte =
            byte_at_screen(&doc, id, probe, &viewport, screen_size).expect("hit test resolves");
        assert_eq!(byte, 5);
    }

    #[test]
    fn vertical_movement_crosses_hard_lines_and_clamps_at_the_ends() {
        let (doc, id) = doc_with_text_node("ab\ncd");
        let down = vertical_move_target(&doc, id, 0, true).expect("target below");
        assert!(down >= 3, "down from line 1 lands on line 2, got {down}");
        let up = vertical_move_target(&doc, id, down, false).expect("target above");
        assert!(up <= 2, "up from line 2 lands on line 1, got {up}");
        // First line up → start; last line down → end.
        assert_eq!(vertical_move_target(&doc, id, 1, false), Some(0));
        assert_eq!(vertical_move_target(&doc, id, 4, true), Some(5));
    }

    #[test]
    fn node_containment_projects_world_bounds() {
        let (doc, id) = doc_with_text_node("hello");
        let viewport = Viewport {
            center: [0.0, 0.0],
            zoom: 1.0,
        };
        let screen_size = DVec2::new(800.0, 600.0);
        // Node box is (0,0)..(200,40) in world space → (400,300)..(600,340).
        assert!(node_contains_screen(
            &doc,
            id,
            DVec2::new(450.0, 320.0),
            &viewport,
            screen_size
        ));
        assert!(!node_contains_screen(
            &doc,
            id,
            DVec2::new(390.0, 320.0),
            &viewport,
            screen_size
        ));
    }

    #[test]
    fn word_range_handles_edges_and_unicode() {
        assert_eq!(word_range("hello world", 2), 0..5);
        assert_eq!(word_range("hello world", 11), 6..11);
        assert_eq!(word_range("héllo", 1), 0..6);
        assert_eq!(word_range("a  b", 2), 2..2);
    }

    /// The caret is solid for the first interval after a reset (so it never
    /// vanishes the instant editing starts), hidden for the second, and solid
    /// again in the third — the "solid while acting, blink when idle" cadence.
    #[test]
    fn caret_blink_is_solid_then_alternates() {
        let interval = CARET_BLINK_INTERVAL;
        // Just after a reset: solid.
        assert!(caret_visible_after(Duration::ZERO));
        assert!(caret_visible_after(interval / 2));
        // Into the second half: hidden.
        assert!(!caret_visible_after(interval + interval / 2));
        // Third half-cycle: solid again.
        assert!(caret_visible_after(interval * 2 + interval / 2));
    }

    /// A multi-glyph selection projects to at least one on-screen,
    /// non-degenerate rect — the overlay's selection highlight. This mirrors
    /// exactly what `render_text_edit_overlay` feeds `paint_quad`, so an empty
    /// or off-screen result here is an invisible selection on the canvas.
    #[test]
    fn selection_rects_are_on_screen_and_non_degenerate() {
        let (doc, id) = doc_with_text_node("Hello world");
        let viewport = Viewport {
            center: [100.0, 20.0],
            zoom: 1.5,
        };
        let screen_size = DVec2::new(800.0, 600.0);
        let rects = selection_screen_rects(&doc, id, 0..5, &viewport, screen_size);
        assert!(
            !rects.is_empty(),
            "a five-glyph selection must yield at least one highlight rect"
        );
        for [x, y, w, h] in &rects {
            assert!(
                *w > 0.0 && *h > 0.0,
                "rect {:?} is degenerate",
                [x, y, w, h]
            );
            assert!(
                *x + *w > 0.0 && *x < screen_size.x && *y + *h > 0.0 && *y < screen_size.y,
                "rect {:?} is off-screen for {screen_size:?}",
                [x, y, w, h]
            );
        }
    }

    /// The caret projects to an on-screen, non-zero-height segment — the
    /// overlay's blinking bar. A zero-length or off-screen segment is an
    /// invisible caret.
    #[test]
    fn caret_segment_is_on_screen_and_non_zero() {
        let (doc, id) = doc_with_text_node("Hello world");
        let viewport = Viewport {
            center: [100.0, 20.0],
            zoom: 1.5,
        };
        let screen_size = DVec2::new(800.0, 600.0);
        let (top, bottom) =
            caret_screen_segment(&doc, id, 5, &viewport, screen_size).expect("caret geometry");
        assert!(
            (bottom.y - top.y).abs() > 1.0,
            "caret segment height {} is too small",
            (bottom.y - top.y).abs()
        );
        for point in [top, bottom] {
            assert!(
                point.x >= 0.0
                    && point.x <= screen_size.x
                    && point.y >= 0.0
                    && point.y <= screen_size.y,
                "caret endpoint {point:?} is off-screen for {screen_size:?}"
            );
        }
    }

    /// Shift-extension diverges anchor from head, so `selected_range` is
    /// non-empty and feeds the highlight — the precondition for any selection
    /// to be drawable at all.
    #[test]
    fn shift_extension_diverges_the_selection_range() {
        let (doc, id) = doc_with_text_node("Hello world");
        let mut session = TextEditSession::new(id, &text_node(&doc, id));
        session.move_to(0, false);
        assert!(session.selected_range().is_empty());
        session.move_right(true);
        session.move_right(true);
        session.move_right(true);
        let range = session.selected_range();
        assert_eq!(
            range,
            0..3,
            "three shift-rights select the first three bytes"
        );
        assert!(!range.is_empty());
        // And that range projects to a visible highlight.
        let viewport = Viewport {
            center: [0.0, 0.0],
            zoom: 1.0,
        };
        let rects = selection_screen_rects(&doc, id, range, &viewport, DVec2::new(800.0, 600.0));
        assert!(!rects.is_empty(), "the extended selection must be drawable");
    }

    /// The overlay projects node-local caret geometry the same way the working
    /// selection-handle overlay (`canvas::paint_overlays`) projects world
    /// bounds: `world_to_screen(world_transform * local)` then add the element
    /// origin. This test pins that the caret lands inside the node's projected
    /// screen bounds, so it can never be painted outside the glyphs it marks.
    #[test]
    fn caret_lands_within_the_node_screen_bounds() {
        let (doc, id) = doc_with_text_node("Hello world");
        let viewport = Viewport {
            center: [0.0, 0.0],
            zoom: 1.0,
        };
        let screen_size = DVec2::new(800.0, 600.0);
        let (top, bottom) =
            caret_screen_segment(&doc, id, 3, &viewport, screen_size).expect("caret geometry");
        let bounds = doc.scene.world_bounds(id).expect("node bounds");
        let min = fanta_canvas::world_to_screen(
            DVec2::new(bounds.min_x, bounds.min_y),
            &viewport,
            screen_size,
        );
        let max = fanta_canvas::world_to_screen(
            DVec2::new(bounds.max_x, bounds.max_y),
            &viewport,
            screen_size,
        );
        for point in [top, bottom] {
            assert!(
                point.x >= min.x - 1.0 && point.x <= max.x + 1.0,
                "caret x {} outside node screen bounds [{}, {}]",
                point.x,
                min.x,
                max.x
            );
        }
    }

    /// End-to-end harness for the text editor's partial styling (the core of
    /// "modify only selection" in the inspector while a text session is live).
    /// This exercises:
    /// - Opening a session on a TextNode
    /// - Setting a non-empty character selection
    /// - Applying a style patch (color) only to the selection via the same
    ///   path the properties panel + view use
    /// - Preview updating the doc's style + style_runs
    /// - Selection rects being non-empty (so the overlay would draw visible highlight)
    /// - Commit producing the correct runs in the final doc
    /// Run this with `cargo test -p fig_viewer text_editor_partial_style_harness`
    /// and iterate ("loop") on failures.
    #[test]
    fn text_editor_partial_style_harness() {
        let (mut doc, id) = doc_with_text_node("Hello world");
        let original_style = text_node(&doc, id).style;

        let mut session = TextEditSession::new(id, &text_node(&doc, id));
        // Select "world" (bytes 6..11)
        session.move_to(6, false);
        session.move_to(11, true);
        assert_eq!(session.selected_text(), Some("world"));
        assert!(!session.selected_range().is_empty());

        // Simulate inspector changing color on the *selection only*.
        // (Mimics with_text_selection_style + apply_style_to_selection)
        let red = fanta_doc::Color::rgb(255, 0, 0);
        let mut s = session.text_buffer().style_at(session.caret()).clone();
        s.color = red;
        session.apply_style_to_selection(s);

        // Preview should write runs (but keep base style)
        apply_preview(&mut doc, &session);
        let previewed = text_node(&doc, id);
        assert_eq!(previewed.content, "Hello world");
        assert_eq!(previewed.style.color, original_style.color); // base unchanged
        // set_style on "world" (6..11) of "Hello world" (len 11) splits into before(0..6) + selected; no after text
        assert_eq!(
            previewed.style_runs.len(),
            2,
            "runs: {:?}",
            previewed.style_runs
        );
        let red_run = previewed
            .style_runs
            .iter()
            .find(|r| r.style.color == red)
            .expect("red run present");
        assert_eq!(red_run.start, 6);
        assert_eq!(red_run.end, 11);
        assert_eq!(red_run.style.color, red);

        // The overlay should see a drawable selection
        let viewport = Viewport {
            center: [0.0, 0.0],
            zoom: 1.0,
        };
        let screen_size = DVec2::new(800.0, 600.0);
        let rects = selection_screen_rects(&doc, id, 6..11, &viewport, screen_size);
        assert!(
            !rects.is_empty(),
            "partial selection must produce highlight rects for the overlay"
        );
        assert!(rects.iter().any(|r| r[2] > 0.0 && r[3] > 0.0));

        // Commit path (rewind + ReplaceData) must preserve the partial style
        rewind_preview(&mut doc, &session);
        let op = commit_operation(&doc, &session).expect("should have changed");
        doc.apply(op).expect("apply commit");
        let committed = text_node(&doc, id);
        assert_eq!(committed.style_runs.len(), 2);
        let red_run = committed
            .style_runs
            .iter()
            .find(|r| r.style.color == red)
            .expect("red run present after commit");
        assert_eq!(red_run.style.color, red);
        assert_eq!(committed.style.color, original_style.color);
    }
}
