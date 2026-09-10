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

use std::time::{Duration, Instant};
use std::{fmt, ops::Range};

use fanta_doc::{
    Color, Doc, NodeData, NodeId, Operation, Override, OverridePath, TextAlign, TextAutoResize,
    TextNode, TextPathNode, TextStyleRun, Transform2D, VAlign, Viewport,
};
use fanta_render::{TextPathAffinity, TextPathPosition, TextPathVisualDirection};
use fanta_text::{Caret, Selection, TextBuffer, TextError, TextStyle as EngineTextStyle};
use glam::DVec2;
use gpui::{Subscription, Task};

use crate::instance_text;

/// Half-period of the caret blink, matching the original fanta-app (~2×/sec):
/// the caret is solid for one interval, then hidden for one interval.
pub(crate) const CARET_BLINK_INTERVAL: Duration = Duration::from_millis(530);
const TEXT_PATH_EDIT_HIT_SLOP_PX: f64 = 4.0;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InvalidTextStyleRun {
    pub(crate) index: usize,
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) error: TextError,
}

impl fmt::Display for InvalidTextStyleRun {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "style run {} ({}..{}) is invalid: {}",
            self.index, self.start, self.end, self.error
        )
    }
}

impl std::error::Error for InvalidTextStyleRun {}

/// The pure editing model: the live buffer, the directed selection, and the
/// pre-edit node data the commit/rewind path restores. No GPUI types, so the
/// whole editing behavior is testable headlessly.
pub(crate) struct TextEditSession {
    node_id: NodeId,
    /// Full node data at open time for rewind. For an instance session this is
    /// the resolved text CLONE (content/style reflect existing overrides).
    original: TextNode,
    node_kind: TextEditNodeKind,
    /// `Some` when editing text inside a component instance — the edit becomes
    /// an override rather than a scene write.
    instance: Option<InstanceEdit>,
    /// Live rich text buffer supporting style runs over selections.
    buffer: TextBuffer,
    invalid_style_run: Option<InvalidTextStyleRun>,
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
    /// Visual sides of the selection endpoints for TextPath BiDi boundaries.
    /// The byte selection remains authoritative for edits and styling.
    text_path_affinities: TextPathSelectionAffinities,
    /// IME composition range (byte offsets into `buffer`), when marked text
    /// is pending.
    marked_range: Option<Range<usize>>,
    /// A mouse drag-selection is in flight.
    pub(crate) dragging: bool,
    /// The pointer is currently over the edited node (drives the I-beam).
    pub(crate) pointer_inside: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TextEditNodeKind {
    Text,
    TextPath,
}

#[derive(Clone, Copy)]
struct TextPathSelectionAffinities {
    anchor: TextPathAffinity,
    head: TextPathAffinity,
}

impl TextEditSession {
    pub(crate) fn new(node_id: NodeId, node: &TextNode) -> Self {
        Self::new_with_kind(node_id, node, TextEditNodeKind::Text)
    }

    pub(crate) fn new_text_path(node_id: NodeId, node: &TextPathNode) -> Self {
        let mut text = TextNode::new(node.content.clone(), 1.0, 1.0);
        text.style = node.style.clone();
        text.style_runs = node.style_runs.clone();
        Self::new_with_kind(node_id, &text, TextEditNodeKind::TextPath)
    }

    fn new_with_kind(node_id: NodeId, node: &TextNode, node_kind: TextEditNodeKind) -> Self {
        let caret = node.content.len();
        let caret_affinity = boundary_affinity(caret, node.content.len());
        let (buf, invalid_style_run) =
            buffer_from_doc_text(&node.content, &node.style, &node.style_runs);
        if let Some(error) = invalid_style_run {
            log::error!("cannot construct a lossless text-edit buffer: {error}");
        }
        Self {
            node_id,
            original: node.clone(),
            node_kind,
            instance: None,
            invalid_style_run,
            opened_buffer: buf.clone(),
            buffer: buf,
            selection: Selection::caret(caret),
            text_path_affinities: TextPathSelectionAffinities {
                anchor: caret_affinity,
                head: caret_affinity,
            },
            marked_range: None,
            dragging: false,
            pointer_inside: false,
        }
    }

    pub(crate) fn matches_node_data(&self, data: &NodeData) -> bool {
        matches!(
            (self.node_kind, data),
            (TextEditNodeKind::Text, NodeData::Text(_))
                | (TextEditNodeKind::TextPath, NodeData::TextPath(_))
        )
    }

    pub(crate) fn invalid_style_run(&self) -> Option<&InvalidTextStyleRun> {
        self.invalid_style_run.as_ref()
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

    fn caret_position(&self) -> TextPathPosition {
        TextPathPosition {
            byte: self.selection.head,
            affinity: self.text_path_affinities.head,
        }
    }

    fn anchor_position(&self) -> TextPathPosition {
        TextPathPosition {
            byte: self.selection.anchor,
            affinity: self.text_path_affinities.anchor,
        }
    }

    pub(crate) fn is_text_path(&self) -> bool {
        self.node_kind == TextEditNodeKind::TextPath
    }

    fn position_for_byte(&self, byte: usize) -> TextPathPosition {
        if byte == self.selection.head {
            return self.caret_position();
        }
        let affinity = if byte == self.selection.anchor {
            self.text_path_affinities.anchor
        } else {
            boundary_affinity(byte, self.buffer.len())
        };
        TextPathPosition { byte, affinity }
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
    pub(crate) fn apply_style_to_selection(
        &mut self,
        style: EngineTextStyle,
    ) -> Result<(), TextError> {
        // An instance override carries a single whole-node glyph color, so a
        // style change on an instance session applies to the whole node (the
        // default) rather than a sub-range run — otherwise the selection-run
        // color would have no override to commit into.
        if self.instance.is_some() {
            self.buffer.set_default_style(style);
            return Ok(());
        }
        let range = self.selected_range();
        if !range.is_empty() {
            self.buffer.set_style(range, style)?;
        } else {
            self.buffer.set_default_style(style);
        }
        Ok(())
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
        self.set_selection(0, self.buffer.len());
    }

    pub(crate) fn move_to(&mut self, byte: usize, extend: bool) {
        let byte = self.clamp_offset(byte);
        let affinity = if extend && byte == self.selection.anchor {
            self.text_path_affinities.anchor
        } else if byte < self.selection.head {
            TextPathAffinity::Downstream
        } else if byte > self.selection.head {
            TextPathAffinity::Upstream
        } else {
            self.text_path_affinities.head
        };
        self.move_to_position(TextPathPosition { byte, affinity }, extend);
    }

    pub(crate) fn move_to_position(&mut self, position: TextPathPosition, extend: bool) {
        let byte = self.clamp_offset(position.byte);
        let affinity = if byte == position.byte {
            position.affinity
        } else {
            boundary_affinity(byte, self.buffer.len())
        };
        if extend {
            self.selection.head = byte;
            self.text_path_affinities.head = affinity;
        } else {
            self.set_caret(byte, affinity);
        }
    }

    pub(crate) fn click_position(&mut self, position: TextPathPosition, extend: bool) {
        self.move_to_position(position, extend);
    }

    pub(crate) fn drag_to_position(&mut self, position: TextPathPosition) {
        self.move_to_position(position, true);
    }

    pub(crate) fn select_word_at(&mut self, byte: usize) {
        let range = word_range(self.buffer.text(), self.clamp_offset(byte));
        if range.is_empty() {
            self.set_caret(
                range.start,
                boundary_affinity(range.start, self.buffer.len()),
            );
        } else {
            self.set_selection(range.start, range.end);
        }
    }

    pub(crate) fn move_left(&mut self, extend: bool) {
        if !extend && !self.selection.is_collapsed() {
            let byte = self.selection.start();
            let affinity = self.affinity_at_endpoint(byte);
            self.set_caret(byte, affinity);
            return;
        }
        let head = Caret::new(self.selection.head)
            .move_left(self.buffer.text())
            .byte;
        let affinity = if extend && head == self.selection.anchor {
            self.text_path_affinities.anchor
        } else {
            TextPathAffinity::Downstream
        };
        self.move_to_position(
            TextPathPosition {
                byte: head,
                affinity,
            },
            extend,
        );
    }

    pub(crate) fn move_right(&mut self, extend: bool) {
        if !extend && !self.selection.is_collapsed() {
            let byte = self.selection.end();
            let affinity = self.affinity_at_endpoint(byte);
            self.set_caret(byte, affinity);
            return;
        }
        let head = Caret::new(self.selection.head)
            .move_right(self.buffer.text())
            .byte;
        let affinity = if extend && head == self.selection.anchor {
            self.text_path_affinities.anchor
        } else {
            TextPathAffinity::Upstream
        };
        self.move_to_position(
            TextPathPosition {
                byte: head,
                affinity,
            },
            extend,
        );
    }

    pub(crate) fn move_line_start(&mut self, extend: bool) {
        let head = Caret::new(self.selection.head)
            .move_home(self.buffer.text())
            .byte;
        let affinity = if extend && head == self.selection.anchor {
            self.text_path_affinities.anchor
        } else {
            TextPathAffinity::Downstream
        };
        self.move_to_position(
            TextPathPosition {
                byte: head,
                affinity,
            },
            extend,
        );
    }

    pub(crate) fn move_line_end(&mut self, extend: bool) {
        let head = Caret::new(self.selection.head)
            .move_end(self.buffer.text())
            .byte;
        let affinity = if extend && head == self.selection.anchor {
            self.text_path_affinities.anchor
        } else {
            TextPathAffinity::Upstream
        };
        self.move_to_position(
            TextPathPosition {
                byte: head,
                affinity,
            },
            extend,
        );
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
        let caret = range.start + text.len();
        self.set_caret(caret, boundary_affinity(caret, self.buffer.len()));
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
        match relative_selection {
            Some(relative) => {
                let anchor = self.clamp_offset(range.start + relative.start);
                let head = self.clamp_offset(range.start + relative.end);
                self.set_selection(anchor, head);
            }
            None => {
                let caret = range.start + text.len();
                self.set_caret(caret, boundary_affinity(caret, self.buffer.len()));
            }
        }
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

    fn set_caret(&mut self, byte: usize, affinity: TextPathAffinity) {
        self.selection = Selection::caret(byte);
        self.text_path_affinities = TextPathSelectionAffinities {
            anchor: affinity,
            head: affinity,
        };
    }

    fn set_selection(&mut self, anchor: usize, head: usize) {
        self.selection = Selection::new(anchor, head);
        if anchor == head {
            let affinity = boundary_affinity(head, self.buffer.len());
            self.text_path_affinities = TextPathSelectionAffinities {
                anchor: affinity,
                head: affinity,
            };
        } else {
            self.text_path_affinities = TextPathSelectionAffinities {
                anchor: if anchor < head {
                    TextPathAffinity::Downstream
                } else {
                    TextPathAffinity::Upstream
                },
                head: if head > anchor {
                    TextPathAffinity::Upstream
                } else {
                    TextPathAffinity::Downstream
                },
            };
        }
    }

    fn affinity_at_endpoint(&self, byte: usize) -> TextPathAffinity {
        if byte == self.selection.head {
            self.text_path_affinities.head
        } else if byte == self.selection.anchor {
            self.text_path_affinities.anchor
        } else {
            boundary_affinity(byte, self.buffer.len())
        }
    }
}

fn boundary_affinity(byte: usize, len: usize) -> TextPathAffinity {
    if byte == 0 || len == 0 {
        TextPathAffinity::Downstream
    } else {
        TextPathAffinity::Upstream
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

fn buffer_from_doc_text(
    content: &str,
    base_style: &fanta_doc::TextStyle,
    style_runs: &[TextStyleRun],
) -> (TextBuffer, Option<InvalidTextStyleRun>) {
    let base_style = map_doc_style_to_engine(base_style);
    let mut buffer = if content.is_empty() {
        TextBuffer::new()
    } else {
        TextBuffer::from_str(content.to_owned(), base_style.clone())
    };
    let mut invalid_style_run = None;
    for (index, run) in style_runs.iter().enumerate() {
        let run_style = map_doc_style_to_engine(&run.style);
        if let Err(error) = buffer.set_style(run.start..run.end, run_style)
            && invalid_style_run.is_none()
        {
            invalid_style_run = Some(InvalidTextStyleRun {
                index,
                start: run.start,
                end: run.end,
                error,
            });
        }
    }
    buffer.set_default_style(base_style);
    (buffer, invalid_style_run)
}

pub(crate) fn replace_styled_text_ranges(
    content: &str,
    base_style: &fanta_doc::TextStyle,
    style_runs: &[TextStyleRun],
    ranges: &[Range<usize>],
    replacement: &str,
) -> anyhow::Result<(String, Vec<TextStyleRun>)> {
    let (mut buffer, invalid_style_run) = buffer_from_doc_text(content, base_style, style_runs);
    if let Some(error) = invalid_style_run {
        return Err(error.into());
    }

    let mut preceding_start = content.len();
    for range in ranges.iter().rev() {
        if range.start >= range.end || range.end > preceding_start {
            anyhow::bail!(
                "replacement range {}..{} is empty, overlapping, or out of order",
                range.start,
                range.end
            );
        }
        let replacement_style = buffer.style_at(range.start).clone();
        buffer.delete_range(range.clone())?;
        if !replacement.is_empty() {
            buffer.insert(range.start, replacement)?;
            buffer.set_style(
                range.start..range.start + replacement.len(),
                replacement_style,
            )?;
        }
        preceding_start = range.start;
    }

    let style_runs = buffer
        .runs()
        .iter()
        .filter(|run| &run.style != buffer.default_style())
        .map(|run| TextStyleRun {
            start: run.start,
            end: run.end,
            style: map_engine_style_to_doc(&run.style),
        })
        .collect();
    Ok((buffer.text().to_owned(), style_runs))
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
    if session.invalid_style_run.is_some() {
        return;
    }
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
    let style = map_engine_style_to_doc(session.buffer.default_style());
    let style_runs: Vec<_> = session
        .buffer
        .runs()
        .iter()
        .map(|r| TextStyleRun {
            start: r.start,
            end: r.end,
            style: map_engine_style_to_doc(&r.style),
        })
        .collect();
    let Some(node) = doc.scene.get_mut(session.node_id) else {
        return;
    };
    match (&mut node.data, session.node_kind) {
        (NodeData::Text(text), TextEditNodeKind::Text) => {
            text.content = session.buffer.text().to_string();
            text.style = style;
            text.style_runs = style_runs;
            hug_auto_resize(text);
        }
        (NodeData::TextPath(text_path), TextEditNodeKind::TextPath) => {
            text_path.content = session.buffer.text().to_string();
            text_path.style = style;
            text_path.style_runs = style_runs;
        }
        _ => return,
    }
    // A `get_mut` write bypasses `Doc::apply`, so the master revision that
    // keys memoized instance expansions must move by hand or every instance
    // of an edited master keeps drawing the pre-edit text until the commit.
    // The preview revision, since nothing is committed yet — `rev` is persisted
    // and doubles as the "master was edited" flag.
    doc.components
        .bump_preview_for_node(&doc.scene, session.node_id);
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
    match (&mut node.data, session.node_kind) {
        (NodeData::Text(text), TextEditNodeKind::Text) => {
            text.content = session.original.content.clone();
            text.local_size = session.original.local_size;
            text.style = session.original.style.clone();
            text.style_runs = session.original.style_runs.clone();
        }
        (NodeData::TextPath(text_path), TextEditNodeKind::TextPath) => {
            text_path.content = session.original.content.clone();
            text_path.style = session.original.style.clone();
            text_path.style_runs = session.original.style_runs.clone();
        }
        _ => return,
    }
    doc.components
        .bump_preview_for_node(&doc.scene, session.node_id);
}

/// The single undoable operation committing the session, built against the
/// REWOUND document (so `old` carries the pre-edit text-session fields while
/// keeping concurrent non-glyph property edits). `None` when nothing changed
/// or the node is gone.
pub(crate) fn commit_operation(doc: &Doc, session: &TextEditSession) -> Option<Operation> {
    if session.invalid_style_run.is_some() || !session.is_changed() {
        return None;
    }
    // Instance sessions commit through `commit_ops` (their edit is one or two
    // `SetInstanceOverride`s, not a `ReplaceData`).
    if session.instance.is_some() {
        return None;
    }
    let node = doc.scene.get(session.node_id)?;
    let style = map_engine_style_to_doc(session.buffer.default_style());
    let style_runs: Vec<_> = session
        .buffer
        .runs()
        .iter()
        .map(|r| TextStyleRun {
            start: r.start,
            end: r.end,
            style: map_engine_style_to_doc(&r.style),
        })
        .collect();
    let new = match (&node.data, session.node_kind) {
        (NodeData::Text(text), TextEditNodeKind::Text) => {
            let mut new_text = text.clone();
            new_text.content = session.buffer.text().to_string();
            new_text.style = style;
            new_text.style_runs = style_runs;
            hug_auto_resize(&mut new_text);
            NodeData::Text(new_text)
        }
        (NodeData::TextPath(text_path), TextEditNodeKind::TextPath) => {
            let mut new_text_path = text_path.clone();
            new_text_path.content = session.buffer.text().to_string();
            new_text_path.style = style;
            new_text_path.style_runs = style_runs;
            NodeData::TextPath(new_text_path)
        }
        _ => return None,
    };
    Some(Operation::ReplaceData {
        id: session.node_id,
        old: Box::new(node.data.clone()),
        new: Box::new(new),
    })
}

/// The undoable operation(s) committing the session against the REWOUND
/// document. A real-text session yields at most one [`Operation::ReplaceData`];
/// an instance session yields up to two [`Operation::SetInstanceOverride`]s (a
/// text-content override, and a glyph-color override when the color changed).
/// Empty when nothing changed.
pub(crate) fn commit_ops(doc: &Doc, session: &TextEditSession) -> Vec<Operation> {
    if session.invalid_style_run.is_some() || !session.is_changed() {
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

/// Selection quadrilaterals projected through an explicit `world` transform.
fn selection_quads_core(
    text: &TextNode,
    world: Transform2D,
    range: Range<usize>,
    viewport: &Viewport,
    screen_size: DVec2,
) -> Vec<[DVec2; 4]> {
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
            [
                project(x, y + dy),
                project(x + width, y + dy),
                project(x + width, y + dy + height),
                project(x, y + dy + height),
            ]
        })
        .collect()
}

#[cfg(test)]
fn quad_bounds(quad: [DVec2; 4]) -> [f64; 4] {
    let min_x = quad
        .iter()
        .map(|point| point.x)
        .fold(f64::INFINITY, f64::min);
    let max_x = quad
        .iter()
        .map(|point| point.x)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_y = quad
        .iter()
        .map(|point| point.y)
        .fold(f64::INFINITY, f64::min);
    let max_y = quad
        .iter()
        .map(|point| point.y)
        .fold(f64::NEG_INFINITY, f64::max);
    [
        min_x,
        min_y,
        (max_x - min_x).max(0.0),
        (max_y - min_y).max(0.0),
    ]
}

fn screen_point_near_quad(point: DVec2, quad: [DVec2; 4], padding: f64) -> bool {
    if !point.is_finite()
        || !padding.is_finite()
        || padding < 0.0
        || quad.iter().any(|corner| !corner.is_finite())
    {
        return false;
    }
    let min_x = quad
        .iter()
        .map(|corner| corner.x)
        .fold(f64::INFINITY, f64::min);
    let max_x = quad
        .iter()
        .map(|corner| corner.x)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_y = quad
        .iter()
        .map(|corner| corner.y)
        .fold(f64::INFINITY, f64::min);
    let max_y = quad
        .iter()
        .map(|corner| corner.y)
        .fold(f64::NEG_INFINITY, f64::max);
    if point.x < min_x - padding
        || point.x > max_x + padding
        || point.y < min_y - padding
        || point.y > max_y + padding
    {
        return false;
    }

    let mut positive_cross = false;
    let mut negative_cross = false;
    let mut twice_area = 0.0;
    for index in 0..quad.len() {
        let start = quad[index];
        let end = quad[(index + 1) % quad.len()];
        let edge = end - start;
        let relative = point - start;
        let cross = edge.x * relative.y - edge.y * relative.x;
        positive_cross |= cross > 1e-9;
        negative_cross |= cross < -1e-9;
        twice_area += start.x * end.y - start.y * end.x;
    }
    if twice_area.abs() > 1e-9 && !(positive_cross && negative_cross) {
        return true;
    }

    (0..quad.len()).any(|index| {
        screen_distance_squared_to_segment(point, quad[index], quad[(index + 1) % quad.len()])
            <= padding * padding
    })
}

fn screen_distance_squared_to_segment(point: DVec2, start: DVec2, end: DVec2) -> f64 {
    let segment = end - start;
    let length_squared = segment.length_squared();
    if length_squared <= f64::EPSILON {
        return (point - start).length_squared();
    }
    let projection = ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0);
    (point - (start + segment * projection)).length_squared()
}

fn local_point_at_screen(
    world: Transform2D,
    screen: DVec2,
    viewport: &Viewport,
    screen_size: DVec2,
) -> Option<DVec2> {
    let [a, b, c, d, _, _] = world.to_components();
    let determinant = a * d - b * c;
    if !world.is_finite() || !determinant.is_finite() || determinant.abs() <= f64::EPSILON {
        return None;
    }
    let world_point = fanta_canvas::screen_to_world(screen, viewport, screen_size);
    Some(world.inverse().transform_point(world_point))
}

/// Byte offset for a screen point via an explicit `world` transform.
fn byte_at_core(
    text: &TextNode,
    world: Transform2D,
    screen: DVec2,
    viewport: &Viewport,
    screen_size: DVec2,
) -> Option<usize> {
    let local = local_point_at_screen(world, screen, viewport, screen_size)?;
    let dy = vertical_paint_offset(text);
    Some(fanta_render::text_hit_test(text, [local.x, local.y - dy]))
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
#[cfg(test)]
pub(crate) fn caret_screen_segment(
    doc: &Doc,
    node_id: NodeId,
    byte: usize,
    viewport: &Viewport,
    screen_size: DVec2,
) -> Option<(DVec2, DVec2)> {
    caret_screen_segment_at(
        doc,
        node_id,
        TextPathPosition {
            byte,
            affinity: TextPathAffinity::Downstream,
        },
        viewport,
        screen_size,
    )
}

fn caret_screen_segment_at(
    doc: &Doc,
    node_id: NodeId,
    position: TextPathPosition,
    viewport: &Viewport,
    screen_size: DVec2,
) -> Option<(DVec2, DVec2)> {
    let node = doc.scene.get(node_id)?;
    let world = doc.scene.world_transform(node_id)?;
    match &node.data {
        NodeData::Text(text) => {
            caret_segment_core(text, world, position.byte, viewport, screen_size)
        }
        NodeData::TextPath(text_path) => {
            let caret = fanta_render::text_path_caret_segment_at(text_path, position)?;
            let project = |point: [f64; 2]| {
                fanta_canvas::world_to_screen(
                    world.transform_point(DVec2::from_array(point)),
                    viewport,
                    screen_size,
                )
            };
            Some((project(caret.start), project(caret.end)))
        }
        _ => None,
    }
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
    let world = doc.scene.world_transform(node_id)?;
    match &node.data {
        NodeData::Text(text) => Some(baseline_segment_core(text, world, viewport, screen_size)),
        // A TextPath baseline is curved; a single straight guide would be
        // misleading, while its caret and selection already follow the path.
        NodeData::TextPath(_) => None,
        _ => None,
    }
}

/// Selection highlights as screen-space quadrilaterals. TextPath clusters and
/// transformed text boxes retain their rotation instead of being expanded to
/// axis-aligned rectangles.
pub(crate) fn selection_screen_quads(
    doc: &Doc,
    node_id: NodeId,
    range: Range<usize>,
    viewport: &Viewport,
    screen_size: DVec2,
) -> Vec<[DVec2; 4]> {
    let Some(node) = doc.scene.get(node_id) else {
        return Vec::new();
    };
    let Some(world) = doc.scene.world_transform(node_id) else {
        return Vec::new();
    };
    match &node.data {
        NodeData::Text(text) => selection_quads_core(text, world, range, viewport, screen_size),
        NodeData::TextPath(text_path) => fanta_render::text_path_selection_quads(text_path, range)
            .into_iter()
            .map(|quad| {
                quad.points.map(|point| {
                    fanta_canvas::world_to_screen(
                        world.transform_point(DVec2::from_array(point)),
                        viewport,
                        screen_size,
                    )
                })
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
pub(crate) fn selection_screen_rects(
    doc: &Doc,
    node_id: NodeId,
    range: Range<usize>,
    viewport: &Viewport,
    screen_size: DVec2,
) -> Vec<[f64; 4]> {
    selection_screen_quads(doc, node_id, range, viewport, screen_size)
        .into_iter()
        .map(quad_bounds)
        .collect()
}

/// Map a screen point to a byte offset in the node's text — what turns a
/// click into a caret. Uses the renderer's shaped layout, so the hit-test
/// agrees with the painted glyphs.
#[cfg(test)]
pub(crate) fn byte_at_screen(
    doc: &Doc,
    node_id: NodeId,
    screen: DVec2,
    viewport: &Viewport,
    screen_size: DVec2,
) -> Option<usize> {
    position_at_screen(doc, node_id, screen, viewport, screen_size).map(|position| position.byte)
}

pub(crate) fn position_at_screen(
    doc: &Doc,
    node_id: NodeId,
    screen: DVec2,
    viewport: &Viewport,
    screen_size: DVec2,
) -> Option<TextPathPosition> {
    let node = doc.scene.get(node_id)?;
    let world = doc.scene.world_transform(node_id)?;
    match &node.data {
        NodeData::Text(text) => {
            byte_at_core(text, world, screen, viewport, screen_size).map(|byte| TextPathPosition {
                byte,
                affinity: TextPathAffinity::Downstream,
            })
        }
        NodeData::TextPath(text_path) => {
            let local = local_point_at_screen(world, screen, viewport, screen_size)?;
            fanta_render::text_path_hit_test_position(text_path, local.to_array())
        }
        _ => None,
    }
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
    let Some(node) = doc.scene.get(node_id) else {
        return false;
    };
    if let NodeData::TextPath(text_path) = &node.data {
        let Some(world) = doc.scene.world_transform(node_id) else {
            return false;
        };
        let Some(local) = local_point_at_screen(world, screen, viewport, screen_size) else {
            return false;
        };
        if !text_path.content.is_empty() {
            return fanta_render::text_path_contains_point(text_path, local.to_array());
        }
    }
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
        None => caret_screen_segment_at(
            doc,
            session.node_id(),
            session.position_for_byte(byte),
            viewport,
            screen_size,
        ),
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

pub(crate) fn session_selection_quads(
    doc: &Doc,
    session: &TextEditSession,
    range: Range<usize>,
    viewport: &Viewport,
    screen_size: DVec2,
) -> Vec<[DVec2; 4]> {
    match session.instance() {
        Some(inst) => selection_quads_core(
            &session.live_text(),
            inst.world,
            range,
            viewport,
            screen_size,
        ),
        None => selection_screen_quads(doc, session.node_id(), range, viewport, screen_size),
    }
}

pub(crate) fn session_byte_at_screen(
    doc: &Doc,
    session: &TextEditSession,
    screen: DVec2,
    viewport: &Viewport,
    screen_size: DVec2,
) -> Option<usize> {
    session_position_at_screen(doc, session, screen, viewport, screen_size)
        .map(|position| position.byte)
}

pub(crate) fn session_position_at_screen(
    doc: &Doc,
    session: &TextEditSession,
    screen: DVec2,
    viewport: &Viewport,
    screen_size: DVec2,
) -> Option<TextPathPosition> {
    match session.instance() {
        Some(inst) => byte_at_core(
            &session.live_text(),
            inst.world,
            screen,
            viewport,
            screen_size,
        )
        .map(|byte| TextPathPosition {
            byte,
            affinity: TextPathAffinity::Downstream,
        }),
        None => position_at_screen(doc, session.node_id(), screen, viewport, screen_size),
    }
}

pub(crate) fn session_text_path_horizontal_target(
    doc: &Doc,
    session: &TextEditSession,
    direction: TextPathVisualDirection,
    extend: bool,
) -> Option<TextPathPosition> {
    if !session.is_text_path() || session.instance().is_some() {
        return None;
    }
    let node = doc.scene.get(session.node_id())?;
    let NodeData::TextPath(text_path) = &node.data else {
        return None;
    };
    if !extend && !session.selection.is_collapsed() {
        fanta_render::text_path_visual_selection_edge(
            text_path,
            session.anchor_position(),
            session.caret_position(),
            direction,
        )
    } else {
        fanta_render::text_path_visual_neighbor(text_path, session.caret_position(), direction)
    }
}

pub(crate) fn session_text_path_visual_line_edge(
    doc: &Doc,
    session: &TextEditSession,
    direction: TextPathVisualDirection,
) -> Option<TextPathPosition> {
    if !session.is_text_path() || session.instance().is_some() {
        return None;
    }
    let node = doc.scene.get(session.node_id())?;
    let NodeData::TextPath(text_path) = &node.data else {
        return None;
    };
    fanta_render::text_path_visual_line_edge(text_path, direction)
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
        None => match doc.scene.get(session.node_id()).map(|node| &node.data) {
            Some(NodeData::TextPath(text_path)) => text_path_edit_contains_screen(
                doc,
                session,
                text_path.content.len(),
                screen,
                viewport,
                screen_size,
            ),
            _ => node_contains_screen(doc, session.node_id(), screen, viewport, screen_size),
        },
    }
}

fn text_path_edit_contains_screen(
    doc: &Doc,
    session: &TextEditSession,
    content_len: usize,
    screen: DVec2,
    viewport: &Viewport,
    screen_size: DVec2,
) -> bool {
    if content_len > 0
        && node_contains_screen(doc, session.node_id(), screen, viewport, screen_size)
    {
        return true;
    }
    if selection_screen_quads(
        doc,
        session.node_id(),
        0..content_len,
        viewport,
        screen_size,
    )
    .into_iter()
    .any(|quad| screen_point_near_quad(screen, quad, TEXT_PATH_EDIT_HIT_SLOP_PX))
    {
        return true;
    }
    session_caret_segment(doc, session, session.caret(), viewport, screen_size).is_some_and(
        |(start, end)| {
            screen_distance_squared_to_segment(screen, start, end)
                <= TEXT_PATH_EDIT_HIT_SLOP_PX * TEXT_PATH_EDIT_HIT_SLOP_PX
        },
    )
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
    match &node.data {
        NodeData::Text(text) => vertical_move_target_core(text, byte, down),
        NodeData::TextPath(text_path) if !text_path.content.is_empty() => {
            Some(if down { text_path.content.len() } else { 0 })
        }
        _ => None,
    }
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
    use fanta_doc::{
        CanvasNode, PathData, TextPathAlignment, TextPathDirection, TextPathSide, TextPathStart,
        Transform2D,
    };

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

    fn doc_with_text_path_node(content: &str) -> (Doc, NodeId, TextPathNode) {
        let mut path = PathData::new();
        path.move_to(0.0, 10.0)
            .cubic_to(30.0, -10.0, 70.0, 30.0, 100.0, 10.0);
        let mut text_path = TextPathNode::new(path, content);
        text_path.start = TextPathStart::new(0, 0.25).expect("valid path start");
        text_path.alignment = TextPathAlignment::Center;
        text_path.direction = TextPathDirection::Reverse;
        text_path.side = TextPathSide::Flipped;
        let original = text_path.clone();
        let mut doc = Doc::new();
        let node = CanvasNode::new(NodeData::TextPath(text_path));
        let node_id = node.id;
        doc.apply(Operation::create_node(node))
            .expect("creating a text-path node");
        doc.history = Default::default();
        (doc, node_id, original)
    }

    fn doc_with_straight_text_path_node(content: &str) -> (Doc, NodeId, TextPathNode) {
        let mut path = PathData::new();
        path.move_to(0.0, 40.0).line_to(600.0, 40.0);
        let mut text_path = TextPathNode::new(path, content);
        text_path.style.size_px = 32.0;
        let original = text_path.clone();
        let mut doc = Doc::new();
        let node = CanvasNode::new(NodeData::TextPath(text_path));
        let node_id = node.id;
        doc.apply(Operation::create_node(node))
            .expect("creating a straight text-path node");
        doc.history = Default::default();
        (doc, node_id, original)
    }

    fn distinct_bidi_boundary(
        text_path: &TextPathNode,
    ) -> (
        usize,
        fanta_render::TextPathCaretSegment,
        fanta_render::TextPathCaretSegment,
    ) {
        for byte in text_path
            .content
            .char_indices()
            .map(|(byte, _)| byte)
            .chain(std::iter::once(text_path.content.len()))
        {
            let upstream = fanta_render::text_path_caret_segment_at(
                text_path,
                TextPathPosition {
                    byte,
                    affinity: TextPathAffinity::Upstream,
                },
            );
            let downstream = fanta_render::text_path_caret_segment_at(
                text_path,
                TextPathPosition {
                    byte,
                    affinity: TextPathAffinity::Downstream,
                },
            );
            if let (Some(upstream), Some(downstream)) = (upstream, downstream) {
                let upstream_midpoint =
                    (DVec2::from_array(upstream.start) + DVec2::from_array(upstream.end)) * 0.5;
                let downstream_midpoint =
                    (DVec2::from_array(downstream.start) + DVec2::from_array(downstream.end)) * 0.5;
                if (upstream_midpoint - downstream_midpoint).length() > 1.0 {
                    return (byte, upstream, downstream);
                }
            }
        }
        panic!("mixed-direction text should expose a split-affinity boundary");
    }

    fn text_path_node(doc: &Doc, id: NodeId) -> TextPathNode {
        match &doc.scene.get(id).expect("node exists").data {
            NodeData::TextPath(text_path) => text_path.clone(),
            _ => panic!("expected a text-path node"),
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
        session
            .apply_style_to_selection(style)
            .expect("instance style range is valid");
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
        session
            .apply_style_to_selection(styled)
            .expect("selected typography range is valid");

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
        session
            .apply_style_to_selection(styled)
            .expect("selected color range is valid");

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
    fn invalid_saved_style_runs_are_reported_without_preview_or_commit() {
        let (mut doc, id) = doc_with_text_node("é");
        {
            let node = doc.scene.get_mut(id).expect("text node");
            let NodeData::Text(text) = &mut node.data else {
                panic!("expected text");
            };
            text.style_runs.push(TextStyleRun {
                start: 0,
                end: 1,
                style: text.style.clone(),
            });
        }
        let original = text_node(&doc, id);
        let mut session = TextEditSession::new(id, &original);
        assert_eq!(
            session.invalid_style_run(),
            Some(&InvalidTextStyleRun {
                index: 0,
                start: 0,
                end: 1,
                error: TextError::NotCharBoundary { index: 1 },
            })
        );
        session.insert("x");
        apply_preview(&mut doc, &session);
        assert_eq!(text_node(&doc, id), original);
        assert!(commit_ops(&doc, &session).is_empty());

        let (mut path_doc, path_id, _) = doc_with_text_path_node("AB");
        {
            let node = path_doc.scene.get_mut(path_id).expect("TextPath node");
            let NodeData::TextPath(text_path) = &mut node.data else {
                panic!("expected TextPath");
            };
            text_path.style_runs.push(TextStyleRun {
                start: 1,
                end: 3,
                style: text_path.style.clone(),
            });
        }
        let original_path = text_path_node(&path_doc, path_id);
        let mut path_session = TextEditSession::new_text_path(path_id, &original_path);
        assert_eq!(
            path_session.invalid_style_run(),
            Some(&InvalidTextStyleRun {
                index: 0,
                start: 1,
                end: 3,
                error: TextError::OutOfBounds { index: 3, len: 2 },
            })
        );
        path_session.insert("x");
        apply_preview(&mut path_doc, &path_session);
        assert_eq!(text_path_node(&path_doc, path_id), original_path);
        assert!(commit_ops(&path_doc, &path_session).is_empty());
    }

    #[test]
    fn styled_replacement_shifts_later_runs_by_utf8_byte_length() {
        let base_style = fanta_doc::TextStyle::default();
        let mut emphasized = base_style.clone();
        emphasized.weight = 700;
        let style_runs = [TextStyleRun {
            start: 3,
            end: 4,
            style: emphasized.clone(),
        }];

        let (content, style_runs) =
            replace_styled_text_ranges("AéB", &base_style, &style_runs, &[1..3], "🦀x")
                .expect("valid UTF-8 replacement");

        assert_eq!(content, "A🦀xB");
        assert_eq!(style_runs.len(), 1);
        assert_eq!(style_runs[0].start, 6);
        assert_eq!(style_runs[0].end, 7);
        assert_eq!(style_runs[0].style, emphasized);
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
    fn text_path_session_previews_and_commits_only_text_fields() {
        let (mut doc, node_id, original) = doc_with_text_path_node("Orbit");
        let mut session = TextEditSession::new_text_path(node_id, &original);
        session.select_all();
        session.insert("Around");

        apply_preview(&mut doc, &session);
        let preview = text_path_node(&doc, node_id);
        assert_eq!(preview.content, "Around");
        assert_eq!(preview.path, original.path);
        assert_eq!(preview.start, original.start);
        assert_eq!(preview.alignment, original.alignment);
        assert_eq!(preview.direction, original.direction);
        assert_eq!(preview.side, original.side);

        rewind_preview(&mut doc, &session);
        assert_eq!(text_path_node(&doc, node_id), original);
        let undo_depth = doc.history.undo_depth();
        let operation = commit_operation(&doc, &session).expect("changed text path");
        doc.apply(operation).expect("commit text path");
        assert_eq!(text_path_node(&doc, node_id).content, "Around");
        assert_eq!(doc.history.undo_depth(), undo_depth + 1);

        assert!(doc.undo().expect("undo text path edit"));
        assert_eq!(text_path_node(&doc, node_id), original);
        assert!(doc.redo().expect("redo text path edit"));
        assert_eq!(text_path_node(&doc, node_id).content, "Around");
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
    fn text_path_caret_hit_and_selection_share_curved_geometry() {
        let (doc, id, _) = doc_with_text_path_node("Orbit");
        let viewport = Viewport {
            center: [0.0, 0.0],
            zoom: 2.0,
        };
        let screen_size = DVec2::new(800.0, 600.0);
        let byte = 3;
        let (start, end) = caret_screen_segment(&doc, id, byte, &viewport, screen_size)
            .expect("text-path caret geometry");
        assert!((end - start).length() > 1.0);

        let hit = byte_at_screen(&doc, id, (start + end) * 0.5, &viewport, screen_size)
            .expect("text-path hit test");
        assert_eq!(hit, byte);

        let quads = selection_screen_quads(&doc, id, 1..4, &viewport, screen_size);
        assert!(!quads.is_empty());
        assert!(quads.iter().all(|quad| {
            quad.iter().all(|point| point.is_finite())
                && (quad[1] - quad[0]).length() > 0.0
                && (quad[3] - quad[0]).length() > 0.0
        }));
    }

    #[test]
    fn text_path_screen_hit_preserves_renderer_affinity() {
        let (doc, id, text_path) = doc_with_straight_text_path_node("abc אבג xyz");
        let (_, upstream, downstream) = distinct_bidi_boundary(&text_path);
        let viewport = Viewport {
            center: [0.0, 0.0],
            zoom: 1.0,
        };
        let screen_size = DVec2::new(800.0, 600.0);
        let session = TextEditSession::new_text_path(id, &text_path);

        for segment in [upstream, downstream] {
            let local = (DVec2::from_array(segment.start) + DVec2::from_array(segment.end)) * 0.5;
            let expected = fanta_render::text_path_hit_test_position(&text_path, local.to_array())
                .expect("renderer hit position");
            let screen = fanta_canvas::world_to_screen(local, &viewport, screen_size);
            let actual = session_position_at_screen(&doc, &session, screen, &viewport, screen_size)
                .expect("viewer session hit position");
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn text_path_session_preserves_split_bidi_caret_sides() {
        let (doc, id, text_path) = doc_with_straight_text_path_node("abc אבג xyz");
        let (byte, _, _) = distinct_bidi_boundary(&text_path);
        let viewport = Viewport {
            center: [0.0, 0.0],
            zoom: 1.0,
        };
        let screen_size = DVec2::new(800.0, 600.0);
        let upstream = TextPathPosition {
            byte,
            affinity: TextPathAffinity::Upstream,
        };
        let downstream = TextPathPosition {
            byte,
            affinity: TextPathAffinity::Downstream,
        };
        let mut session = TextEditSession::new_text_path(id, &text_path);

        session.click_position(upstream, false);
        assert_eq!(session.caret_position(), upstream);
        let upstream_screen =
            session_caret_segment(&doc, &session, session.caret(), &viewport, screen_size)
                .expect("upstream session caret");

        session.drag_to_position(downstream);
        assert_eq!(session.caret_position(), downstream);
        assert_eq!(
            session.text_path_affinities.anchor,
            TextPathAffinity::Upstream
        );
        assert_eq!(
            session.text_path_affinities.head,
            TextPathAffinity::Downstream
        );
        let downstream_screen =
            session_caret_segment(&doc, &session, session.caret(), &viewport, screen_size)
                .expect("downstream session caret");

        let upstream_midpoint = (upstream_screen.0 + upstream_screen.1) * 0.5;
        let downstream_midpoint = (downstream_screen.0 + downstream_screen.1) * 0.5;
        assert!((upstream_midpoint - downstream_midpoint).length() > 1.0);
    }

    #[test]
    fn text_path_arrow_movement_and_selection_collapse_follow_visual_order() {
        let (doc, id, text_path) = doc_with_straight_text_path_node("abc אבג xyz");
        let mut session = TextEditSession::new_text_path(id, &text_path);
        let visual_start =
            session_text_path_visual_line_edge(&doc, &session, TextPathVisualDirection::Previous)
                .expect("visual line start");
        let visual_end =
            session_text_path_visual_line_edge(&doc, &session, TextPathVisualDirection::Next)
                .expect("visual line end");
        session.move_to_position(visual_start, false);

        let mut positions = vec![visual_start];
        for _ in 0..text_path.content.len().saturating_mul(2).saturating_add(2) {
            let Some(next) = session_text_path_horizontal_target(
                &doc,
                &session,
                TextPathVisualDirection::Next,
                false,
            ) else {
                break;
            };
            session.move_to_position(next, false);
            positions.push(next);
        }
        assert_eq!(
            positions.last().copied(),
            Some(visual_end),
            "Right reaches the shaped visual-line end"
        );
        let rtl_pair = positions
            .windows(2)
            .find(|pair| pair[1].byte < pair[0].byte)
            .expect("visual traversal crosses the RTL run")
            .to_vec();

        session.click_position(rtl_pair[1], false);
        session.drag_to_position(rtl_pair[0]);
        assert!(!session.selected_range().is_empty());
        assert_eq!(
            session_text_path_horizontal_target(
                &doc,
                &session,
                TextPathVisualDirection::Previous,
                false,
            ),
            Some(rtl_pair[0]),
            "Left collapses to the visually previous endpoint even when its byte is greater"
        );
        assert_eq!(
            session_text_path_horizontal_target(
                &doc,
                &session,
                TextPathVisualDirection::Next,
                false,
            ),
            Some(rtl_pair[1]),
            "Right collapses to the visually next endpoint even when its byte is smaller"
        );
    }

    #[test]
    fn text_path_keyboard_navigation_follows_reverse_path_traversal() {
        let mut path = PathData::new();
        path.move_to(0.0, 40.0).line_to(600.0, 40.0);
        let mut text_path = TextPathNode::new(path, "abc");
        text_path.style.size_px = 32.0;
        text_path.start = TextPathStart::new(0, 0.8).expect("valid path position");
        text_path.direction = TextPathDirection::Reverse;
        let mut doc = Doc::new();
        let node = CanvasNode::new(NodeData::TextPath(text_path.clone()));
        let id = node.id;
        doc.apply(Operation::create_node(node))
            .expect("creating a reverse text-path node");
        doc.history = Default::default();

        let mut session = TextEditSession::new_text_path(id, &text_path);
        assert_eq!(
            session_text_path_visual_line_edge(&doc, &session, TextPathVisualDirection::Previous,)
                .expect("Home follows reverse traversal")
                .byte,
            text_path.content.len()
        );
        assert_eq!(
            session_text_path_visual_line_edge(&doc, &session, TextPathVisualDirection::Next)
                .expect("End follows reverse traversal")
                .byte,
            0
        );

        session.move_to(2, false);
        assert_eq!(
            session_text_path_horizontal_target(
                &doc,
                &session,
                TextPathVisualDirection::Previous,
                false,
            )
            .expect("Left follows reverse traversal")
            .byte,
            3
        );
        assert_eq!(
            session_text_path_horizontal_target(
                &doc,
                &session,
                TextPathVisualDirection::Next,
                false,
            )
            .expect("Right follows reverse traversal")
            .byte,
            1
        );
    }

    #[test]
    fn text_path_keyboard_navigation_keeps_the_clipped_source_suffix() {
        let mut path = PathData::new();
        path.move_to(0.0, 20.0).line_to(42.0, 20.0);
        let mut text_path = TextPathNode::new(path, "ABCDEFGHIJ🦀");
        text_path.style.size_px = 20.0;
        let mut doc = Doc::new();
        let node = CanvasNode::new(NodeData::TextPath(text_path.clone()));
        let id = node.id;
        doc.apply(Operation::create_node(node))
            .expect("creating a clipped text-path node");
        doc.history = Default::default();

        let mut expected_bytes = vec![0];
        while expected_bytes.last().copied() != Some(text_path.content.len()) {
            let next = Caret::new(*expected_bytes.last().expect("source start"))
                .move_right(&text_path.content)
                .byte;
            assert!(
                next > *expected_bytes.last().expect("source start"),
                "grapheme traversal must advance"
            );
            expected_bytes.push(next);
        }

        let mut session = TextEditSession::new_text_path(id, &text_path);
        let visual_start =
            session_text_path_visual_line_edge(&doc, &session, TextPathVisualDirection::Previous)
                .expect("Home keeps the full source start");
        let visual_end =
            session_text_path_visual_line_edge(&doc, &session, TextPathVisualDirection::Next)
                .expect("End keeps the full source end");
        assert_eq!(visual_start.byte, 0);
        assert_eq!(visual_end.byte, text_path.content.len());

        let previous = session_text_path_horizontal_target(
            &doc,
            &session,
            TextPathVisualDirection::Previous,
            false,
        )
        .expect("Left enters the clipped suffix");
        assert_eq!(
            previous.byte,
            Caret::new(text_path.content.len())
                .move_left(&text_path.content)
                .byte,
            "one Left moves exactly one source grapheme"
        );
        assert_eq!(
            previous.byte,
            text_path.content.find('🦀').expect("crab byte")
        );
        session.move_to_position(previous, false);
        assert_eq!(
            session_text_path_horizontal_target(
                &doc,
                &session,
                TextPathVisualDirection::Next,
                false,
            ),
            Some(visual_end),
            "Right returns through the clipped suffix"
        );

        session.move_to_position(visual_start, false);
        let mut actual_bytes = vec![visual_start.byte];
        while let Some(next) = session_text_path_horizontal_target(
            &doc,
            &session,
            TextPathVisualDirection::Next,
            false,
        ) {
            assert!(
                actual_bytes.len() < expected_bytes.len(),
                "visual traversal must not cycle"
            );
            session.move_to_position(next, false);
            actual_bytes.push(next.byte);
        }
        assert_eq!(actual_bytes, expected_bytes);
    }

    #[test]
    fn ordinary_text_keeps_logical_horizontal_navigation() {
        let (doc, id) = doc_with_text_node("abc");
        let text = text_node(&doc, id);
        let mut session = TextEditSession::new(id, &text);

        assert!(!session.is_text_path());
        assert!(
            session_text_path_horizontal_target(
                &doc,
                &session,
                TextPathVisualDirection::Previous,
                false,
            )
            .is_none()
        );
        session.move_left(false);
        assert_eq!(session.caret(), 2);
        session.move_right(false);
        assert_eq!(session.caret(), 3);
    }

    #[test]
    fn text_path_session_accepts_inner_and_trailing_whitespace_quads() {
        let viewport = Viewport {
            center: [0.0, 0.0],
            zoom: 1.0,
        };
        let screen_size = DVec2::new(800.0, 600.0);

        for content in ["A A", "A "] {
            let (doc, id, text_path) = doc_with_straight_text_path_node(content);
            let whitespace_range = 1..2;
            let quad =
                selection_screen_quads(&doc, id, whitespace_range.clone(), &viewport, screen_size)
                    .into_iter()
                    .next()
                    .expect("whitespace selection quad");
            let screen = (quad[0] + quad[1] + quad[2] + quad[3]) * 0.25;
            let mut session = TextEditSession::new_text_path(id, &text_path);

            assert!(
                !node_contains_screen(&doc, id, screen, &viewport, screen_size),
                "whitespace in {content:?} has no glyph ink"
            );
            assert!(
                session_contains_screen(&doc, &session, screen, &viewport, screen_size),
                "the active editor accepts whitespace in {content:?}"
            );
            let position =
                session_position_at_screen(&doc, &session, screen, &viewport, screen_size)
                    .expect("whitespace caret position");
            assert!(
                (whitespace_range.start..=whitespace_range.end).contains(&position.byte),
                "whitespace hit resolved to byte {} in {content:?}",
                position.byte
            );
            session.click_position(position, false);
            assert_eq!(session.caret_position(), position);
        }
    }

    #[test]
    fn empty_text_path_session_hit_region_is_bounded_to_its_caret() {
        let (doc, id, text_path) = doc_with_straight_text_path_node("");
        let viewport = Viewport {
            center: [0.0, 0.0],
            zoom: 1.0,
        };
        let screen_size = DVec2::new(800.0, 600.0);
        let session = TextEditSession::new_text_path(id, &text_path);
        let (start, end) =
            session_caret_segment(&doc, &session, session.caret(), &viewport, screen_size)
                .expect("empty text-path caret");
        let caret_midpoint = (start + end) * 0.5;
        let near_caret = caret_midpoint + DVec2::new(TEXT_PATH_EDIT_HIT_SLOP_PX - 1.0, 0.0);
        assert!(session_contains_screen(
            &doc,
            &session,
            near_caret,
            &viewport,
            screen_size,
        ));

        let conservative = doc
            .scene
            .world_bounds(id)
            .expect("empty text path conservative bounds");
        let far_inside_band = fanta_canvas::world_to_screen(
            DVec2::new(conservative.max_x - 1.0, conservative.max_y - 1.0),
            &viewport,
            screen_size,
        );
        assert!(node_contains_screen(
            &doc,
            id,
            far_inside_band,
            &viewport,
            screen_size,
        ));
        assert!(!session_contains_screen(
            &doc,
            &session,
            far_inside_band,
            &viewport,
            screen_size,
        ));
    }

    #[test]
    fn empty_text_path_has_an_editable_caret() {
        let (doc, id, _) = doc_with_text_path_node("");
        let viewport = Viewport {
            center: [0.0, 0.0],
            zoom: 1.0,
        };
        let screen_size = DVec2::new(800.0, 600.0);
        let (start, end) = caret_screen_segment(&doc, id, 0, &viewport, screen_size)
            .expect("empty text path caret");
        assert!(start.is_finite() && end.is_finite());
        assert!((end - start).length() > 0.0);
    }

    #[test]
    fn text_path_containment_accepts_glyph_ink_not_its_conservative_band() {
        let (doc, id, text_path) = doc_with_text_path_node("Orbit");
        let exact =
            fanta_render::text_path_visual_bounds(&text_path).expect("text path visual bounds");
        let local_hit = (0..=32)
            .flat_map(|row| {
                (0..=64).map(move |column| {
                    DVec2::new(
                        exact.min_x + exact.width() * f64::from(column) / 64.0,
                        exact.min_y + exact.height() * f64::from(row) / 32.0,
                    )
                })
            })
            .find(|point| fanta_render::text_path_contains_point(&text_path, point.to_array()))
            .expect("at least one sampled point should intersect glyph ink");
        let conservative = doc
            .scene
            .local_bounds(id)
            .expect("conservative scene bounds");
        let local_miss = [
            DVec2::new(conservative.min_x, conservative.min_y),
            DVec2::new(conservative.max_x, conservative.min_y),
            DVec2::new(conservative.max_x, conservative.max_y),
            DVec2::new(conservative.min_x, conservative.max_y),
        ]
        .into_iter()
        .find(|point| !fanta_render::text_path_contains_point(&text_path, point.to_array()))
        .expect("conservative band should contain space outside glyph ink");
        let viewport = Viewport {
            center: [0.0, 0.0],
            zoom: 1.0,
        };
        let screen_size = DVec2::new(800.0, 600.0);
        let to_screen = |local| fanta_canvas::world_to_screen(local, &viewport, screen_size);

        assert!(node_contains_screen(
            &doc,
            id,
            to_screen(local_hit),
            &viewport,
            screen_size,
        ));
        assert!(!node_contains_screen(
            &doc,
            id,
            to_screen(local_miss),
            &viewport,
            screen_size,
        ));
    }

    #[test]
    fn text_path_vertical_movement_clamps_to_its_single_baseline() {
        let (doc, id, text_path) = doc_with_text_path_node("Orbit");
        assert_eq!(vertical_move_target(&doc, id, 2, false), Some(0));
        assert_eq!(
            vertical_move_target(&doc, id, 2, true),
            Some(text_path.content.len())
        );
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
        session
            .apply_style_to_selection(s)
            .expect("selected partial-style range is valid");

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
