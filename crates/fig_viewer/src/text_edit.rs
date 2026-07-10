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
    Doc, NodeData, NodeId, Operation, TextAlign, TextAutoResize, TextNode, TextStyleRun, VAlign,
    Viewport,
};
use fanta_text::{Caret, Selection, TextBuffer, TextStyle as EngineTextStyle};
use glam::DVec2;
use gpui::{Subscription, Task};

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

/// The pure editing model: the live buffer, the directed selection, and the
/// pre-edit node data the commit/rewind path restores. No GPUI types, so the
/// whole editing behavior is testable headlessly.
pub(crate) struct TextEditSession {
    node_id: NodeId,
    /// Full node data at open time for rewind.
    original: TextNode,
    /// Live rich text buffer supporting style runs over selections.
    buffer: TextBuffer,
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
            buffer: buf,
            selection: Selection::caret(caret),
            marked_range: None,
            dragging: false,
            pointer_inside: false,
        }
    }

    pub(crate) fn node_id(&self) -> NodeId {
        self.node_id
    }

    pub(crate) fn buffer(&self) -> &str {
        self.buffer.text()
    }

    pub(crate) fn text_buffer(&self) -> &fanta_text::TextBuffer {
        &self.buffer
    }

    pub(crate) fn is_changed(&self) -> bool {
        self.buffer.text() != self.original.content
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

    /// Apply a style change to the current selection (or set default for caret).
    /// This enables changing color, font, weight etc. on only the selected text.
    pub(crate) fn apply_style_to_selection(&mut self, style: EngineTextStyle) {
        let range = self.selected_range();
        if !range.is_empty() {
            let _ = self.buffer.set_style(range, style);
        } else {
            self.buffer.set_default_style(style);
        }
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
        let head = Caret::new(self.selection.head).move_left(self.buffer.text()).byte;
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
        let head = Caret::new(self.selection.head).move_home(self.buffer.text()).byte;
        self.move_to(head, extend);
    }

    pub(crate) fn move_line_end(&mut self, extend: bool) {
        let head = Caret::new(self.selection.head).move_end(self.buffer.text()).byte;
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

/// Restore the pre-edit content (and the box size the preview re-hugged) so
/// the committing [`Operation::ReplaceData`] drives the scene from its true
/// old state to the final one through the history chokepoint — the same
/// restore-then-apply staging as the select tool's move commit.
pub(crate) fn rewind_preview(doc: &mut Doc, session: &TextEditSession) {
    let Some(node) = doc.scene.get_mut(session.node_id) else {
        return;
    };
    let NodeData::Text(text) = &mut node.data else {
        return;
    };
    text.content = session.original.content.clone();
    text.local_size = session.original.local_size;
}

/// The single undoable operation committing the session, built against the
/// REWOUND document (so `old` carries the pre-edit content while keeping any
/// concurrent property edits). `None` when nothing changed or the node is
/// gone.
pub(crate) fn commit_operation(doc: &Doc, session: &TextEditSession) -> Option<Operation> {
    if !session.is_changed() {
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
    let world_transform = doc.scene.world_transform(node_id)?;
    let project = |local: DVec2| {
        fanta_canvas::world_to_screen(
            world_transform.transform_point(local),
            viewport,
            screen_size,
        )
    };
    Some((
        project(DVec2::new(x, y + dy)),
        project(DVec2::new(x, y + dy + height)),
    ))
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
    if range.is_empty() {
        return Vec::new();
    }
    let Some(node) = doc.scene.get(node_id) else {
        return Vec::new();
    };
    let NodeData::Text(text) = &node.data else {
        return Vec::new();
    };
    let Some(world_transform) = doc.scene.world_transform(node_id) else {
        return Vec::new();
    };
    let dy = vertical_paint_offset(text);
    let project = |x: f64, y: f64| {
        fanta_canvas::world_to_screen(
            world_transform.transform_point(DVec2::new(x, y)),
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
            let min_x = corners
                .iter()
                .map(|corner| corner.x)
                .fold(f64::INFINITY, f64::min);
            let max_x = corners
                .iter()
                .map(|corner| corner.x)
                .fold(f64::NEG_INFINITY, f64::max);
            let min_y = corners
                .iter()
                .map(|corner| corner.y)
                .fold(f64::INFINITY, f64::min);
            let max_y = corners
                .iter()
                .map(|corner| corner.y)
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
    let world_transform = doc.scene.world_transform(node_id)?;
    let world = fanta_canvas::screen_to_world(screen, viewport, screen_size);
    let local = world_transform.inverse().transform_point(world);
    let dy = vertical_paint_offset(text);
    Some(fanta_render::text_hit_test(text, [local.x, local.y - dy]))
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
            font_variations: style.font_variations.clone(),
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
}
