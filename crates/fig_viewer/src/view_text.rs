//! Inline text editing on the canvas: session lifecycle (open, seed, commit),
//! mouse and keyboard routing while a session is live, the caret/selection
//! overlay, and the platform IME input handler for [`FigView`].

use std::ops::Range;

use fanta_canvas::HitPrecision;
use fanta_doc::{NodeData, NodeId};
use glam::DVec2;
use gpui::{
    AnyElement, App, Bounds, ClipboardItem, ContentMask, Context, ElementInputHandler,
    EntityInputHandler, KeyDownEvent, MouseDownEvent, Pixels, Point, UTF16Selection, Window,
    canvas, fill, point, px, size,
};
use ui::prelude::*;

use crate::canvas::{bounds_size, screen_position_in_bounds};
use crate::document::DocChange;
use crate::instance_text;
use crate::text_edit::{self, CARET_BLINK_INTERVAL, CanvasTextEdit, TextEditSession};
use crate::view::{FigView, TextEditSeed};

impl FigView {
    /// The topmost text node under `screen`, if any. The hit test returns
    /// leaves, so a text node is normally hit directly; walk up the ancestor
    /// chain for content nested under one (e.g. instance-expanded children).
    pub(crate) fn text_node_at(&self, screen: DVec2, cx: &App) -> Option<NodeId> {
        let bounds = self.container_bounds?;
        let viewport = self.viewport?;
        let (width, height) = bounds_size(bounds);
        let item = self.item.read(cx);
        let document = item.document()?;
        let doc = &document.doc;
        let hit = fanta_canvas::hit_test_screen(
            &doc.scene,
            &viewport,
            DVec2::new(width, height),
            screen,
            HitPrecision::Path,
            doc.active_page(),
        )?;
        if matches!(doc.scene.get(hit)?.data, NodeData::Text(_)) {
            return Some(hit);
        }
        // Check ancestors (e.g. text containing other hittable nodes)
        if let Some(text) = doc
            .scene
            .ancestors_of(hit)
            .find(|node| matches!(node.data, NodeData::Text(_)))
            .map(|node| node.id)
        {
            return Some(text);
        }
        // Also search descendants: text nodes inside a frame/group that was hit
        // (common case: artboard frame containing text layers). We pick the
        // first Text descendant whose world bounds contain the click point.
        for desc_id in doc.scene.descendants_of(hit) {
            if let Some(node) = doc.scene.get(desc_id) {
                if matches!(node.data, NodeData::Text(_)) {
                    let contains = text_edit::node_contains_screen(
                        doc,
                        desc_id,
                        screen,
                        &viewport,
                        DVec2::new(width, height),
                    );
                    if contains {
                        return Some(desc_id);
                    }
                }
            }
        }
        None
    }

    /// The topmost TEXT clone inside a component instance under `screen`,
    /// resolved for override editing. Instances are real scene nodes whose
    /// descendants are virtual, so the scene hit-test bottoms out at the
    /// instance node; from there we expand it and point-test its text clones.
    pub(crate) fn instance_text_at(
        &self,
        screen: DVec2,
        cx: &App,
    ) -> Option<instance_text::InstanceTextTarget> {
        let bounds = self.container_bounds?;
        let viewport = self.viewport?;
        let (width, height) = bounds_size(bounds);
        let screen_size = DVec2::new(width, height);
        let item = self.item.read(cx);
        let document = item.document()?;
        let doc = &document.doc;
        let hit = fanta_canvas::hit_test_screen(
            &doc.scene,
            &viewport,
            screen_size,
            screen,
            HitPrecision::Path,
            doc.active_page(),
        )?;
        // The instance is the hit node itself, or its nearest instance ancestor.
        let instance_id = if matches!(doc.scene.get(hit)?.data, NodeData::Instance(_)) {
            hit
        } else {
            doc.scene
                .ancestors_of(hit)
                .find(|node| matches!(node.data, NodeData::Instance(_)))
                .map(|node| node.id)?
        };
        let world_point = fanta_canvas::screen_to_world(screen, &viewport, screen_size);
        instance_text::text_target_at(doc, instance_id, world_point)
    }

    /// The selection anchor, when it is a text node — the node the text tool
    /// just committed and selected.
    pub(crate) fn selection_anchor_text_node(&self, cx: &App) -> Option<NodeId> {
        let item = self.item.read(cx);
        let document = item.document()?;
        let anchor = document.doc.selection.anchor()?;
        matches!(document.doc.scene.get(anchor)?.data, NodeData::Text(_)).then_some(anchor)
    }

    /// The single selected node, when it is a text node — the target for
    /// Figma's enter-to-edit.
    pub(crate) fn single_selected_text_node(&self, cx: &App) -> Option<NodeId> {
        let item = self.item.read(cx);
        let document = item.document()?;
        let &[node] = document.doc.selection.as_slice() else {
            return None;
        };
        matches!(document.doc.scene.get(node)?.data, NodeData::Text(_)).then_some(node)
    }

    /// Open an in-place editing session on `node`, committing any session
    /// already in flight on another node.
    pub(crate) fn open_text_edit(
        &mut self,
        node: NodeId,
        seed: TextEditSeed,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_editable(cx) {
            return;
        }
        if let Some(edit) = self.text_edit.as_ref() {
            if edit.session.node_id() == node && edit.session.instance().is_none() {
                return;
            }
            self.commit_text_edit(cx);
        }
        let text = self.item.read(cx).document().and_then(|document| {
            match &document.doc.scene.get(node)?.data {
                NodeData::Text(text) => Some(text.clone()),
                _ => None,
            }
        });
        let Some(text) = text else {
            return;
        };
        let mut session = TextEditSession::new(node, &text);
        self.seed_session(&mut session, seed, cx);
        self.install_text_session(session, window, cx);
    }

    /// Open an in-place editing session on TEXT that lives inside a component
    /// instance. The edit is persisted as an override on the instance, so the
    /// session carries the instance id, the clone's def-local path, and the
    /// clone's world transform (via `target`) rather than a scene text node.
    pub(crate) fn open_instance_text_edit(
        &mut self,
        target: instance_text::InstanceTextTarget,
        seed: TextEditSeed,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_editable(cx) {
            return;
        }
        if let Some(edit) = self.text_edit.as_ref() {
            if edit.session.instance().is_some_and(|inst| {
                inst.instance_id == target.instance_id && inst.def_path == target.def_path
            }) {
                return;
            }
            self.commit_text_edit(cx);
        }
        let Some(base_overrides) =
            self.item.read(cx).document().map(|document| {
                instance_text::snapshot_overrides(&document.doc, target.instance_id)
            })
        else {
            return;
        };
        let mut session = TextEditSession::new_instance(target, base_overrides);
        self.seed_session(&mut session, seed, cx);
        self.install_text_session(session, window, cx);
    }

    /// Apply the opening selection seed to a freshly built session: select all
    /// (enter-to-edit), or select the word under a double-click point.
    fn seed_session(&self, session: &mut TextEditSession, seed: TextEditSeed, cx: &App) {
        match seed {
            TextEditSeed::SelectAll => session.select_all(),
            TextEditSeed::WordAt(screen) => {
                let byte =
                    self.viewport
                        .zip(self.container_bounds)
                        .and_then(|(viewport, bounds)| {
                            let (width, height) = bounds_size(bounds);
                            let document = self.item.read(cx).document()?;
                            text_edit::session_byte_at_screen(
                                &document.doc,
                                session,
                                screen,
                                &viewport,
                                DVec2::new(width, height),
                            )
                        });
                if let Some(byte) = byte {
                    session.select_word_at(byte);
                }
            }
        }
    }

    /// Select the session's anchor scene node (the text node, or the wrapping
    /// instance node for an instance session), wire the focus-out commit, and
    /// make the session live.
    fn install_text_session(
        &mut self,
        session: TextEditSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let select_id = session.node_id();
        self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                if document.doc.selection.as_slice() == [select_id] {
                    ((), DocChange::None)
                } else {
                    document.doc.selection.select_only(select_id);
                    ((), DocChange::Selection)
                }
            });
        });
        // Focus leaving the canvas (panel field, pane switch) commits the
        // session, matching Figma's click-away semantics.
        let focus_out = cx.on_focus_out(&self.focus_handle, window, |this, _, _, cx| {
            if let Some(edit) = this.text_edit.as_mut()
                && std::mem::take(&mut edit.retain_on_next_focus_out)
            {
                return;
            }
            this.commit_text_edit(cx);
        });
        self.text_edit = Some(CanvasTextEdit::new(session, focus_out));
        self.focus_handle.focus(window, cx);
        self.reset_caret_blink(cx);
        cx.notify();
    }

    /// End the session and, when the text changed, write it back as ONE
    /// undoable operation. The scene already holds the final content (the
    /// transient preview wrote it per keystroke), so this stages the commit
    /// like the select tool's drag: rewind to the pre-edit data, then apply
    /// `ReplaceData { old: original, new: final }` through history.
    pub(crate) fn commit_text_edit(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.text_edit.take() else {
            return;
        };
        cx.notify();
        if !self.is_editable(cx) {
            return;
        }
        let session = edit.session;
        let operations = self
            .item
            .update(cx, |item, cx| {
                item.with_document(cx, |document| {
                    text_edit::rewind_preview(&mut document.doc, &session);
                    let operations = text_edit::commit_ops(&document.doc, &session);
                    // The rewind never reaches the screen: the apply below
                    // repaints with the final content in the same cycle.
                    (operations, DocChange::None)
                })
            })
            .unwrap_or_default();
        if operations.is_empty() {
            return;
        }
        self.item.update(cx, |item, cx| {
            for operation in operations {
                if let Err(error) = item.apply(operation, cx) {
                    log::error!("fig_viewer text edit failed to commit: {error:#}");
                }
            }
        });
    }

    pub(crate) fn drop_text_edit_if_target_gone(&mut self, cx: &App) {
        let Some(edit) = self.text_edit.as_ref() else {
            return;
        };
        // A real session's node must still be a text node; an instance session
        // just needs its wrapping instance node to survive (the edited text is a
        // virtual clone, so `node_id` is the instance).
        let is_instance = edit.session.instance().is_some();
        let target_exists = self.item.read(cx).document().is_some_and(|document| {
            document
                .doc
                .scene
                .get(edit.session.node_id())
                .is_some_and(|node| is_instance || matches!(node.data, NodeData::Text(_)))
        });
        if !target_exists {
            // The node (and with it the preview content) is gone; there is
            // nothing to rewind or commit.
            self.text_edit = None;
        }
    }

    /// A left press while a session is live. Returns true when the press was
    /// consumed (it landed inside the edited node and moved the caret);
    /// false lets the press fall through to the tools — after committing the
    /// session if the press was outside the node.
    pub(crate) fn handle_text_edit_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((viewport, bounds)) = self.viewport.zip(self.container_bounds) else {
            return false;
        };
        if self.text_edit.is_none() {
            return false;
        }
        let (width, height) = bounds_size(bounds);
        let screen_size = DVec2::new(width, height);
        let screen = screen_position_in_bounds(event.position, bounds);
        let byte = {
            let Some(document) = self.item.read(cx).document() else {
                return false;
            };
            let doc = &document.doc;
            let Some(session) = self.text_edit.as_ref().map(|edit| &edit.session) else {
                return false;
            };
            if text_edit::session_contains_screen(doc, session, screen, &viewport, screen_size) {
                text_edit::session_byte_at_screen(doc, session, screen, &viewport, screen_size)
            } else {
                None
            }
        };
        let Some(byte) = byte else {
            self.commit_text_edit(cx);
            return false;
        };
        if let Some(edit) = self.text_edit.as_mut() {
            if event.click_count >= 2 {
                edit.session.select_word_at(byte);
            } else {
                edit.session.click(byte, event.modifiers.shift);
            }
            edit.session.dragging = true;
        }
        self.reset_caret_blink(cx);
        self.notify_text_selection_changed(cx);
        cx.notify();
        true
    }

    /// Pointer movement while a session is live: track whether the cursor is
    /// over the edited node (for the I-beam) and extend a drag-selection.
    /// Returns true when the move was consumed by a drag-selection.
    pub(crate) fn handle_text_edit_mouse_move(
        &mut self,
        screen: DVec2,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((viewport, bounds)) = self.viewport.zip(self.container_bounds) else {
            return false;
        };
        let Some(dragging) = self.text_edit.as_ref().map(|edit| edit.session.dragging) else {
            return false;
        };
        let (width, height) = bounds_size(bounds);
        let screen_size = DVec2::new(width, height);
        let (inside, byte) = {
            let Some(document) = self.item.read(cx).document() else {
                return false;
            };
            let doc = &document.doc;
            let Some(session) = self.text_edit.as_ref().map(|edit| &edit.session) else {
                return false;
            };
            let inside =
                text_edit::session_contains_screen(doc, session, screen, &viewport, screen_size);
            let byte = if dragging {
                text_edit::session_byte_at_screen(doc, session, screen, &viewport, screen_size)
            } else {
                None
            };
            (inside, byte)
        };
        let Some(edit) = self.text_edit.as_mut() else {
            return false;
        };
        if inside != edit.session.pointer_inside {
            edit.session.pointer_inside = inside;
            cx.notify();
        }
        if !dragging {
            return false;
        }
        if let Some(byte) = byte {
            edit.session.drag_to(byte);
            self.reset_caret_blink(cx);
            self.notify_text_selection_changed(cx);
            cx.notify();
        }
        true
    }

    /// Mutate the session's text, then push the new buffer into the document
    /// as a transient preview frame — the canvas repaints it through the
    /// normal renderer, which is what makes the editing view WYSIWYG.
    fn with_text_session_edit(
        &mut self,
        cx: &mut Context<Self>,
        edit_session: impl FnOnce(&mut TextEditSession),
    ) {
        {
            let Some(edit) = self.text_edit.as_mut() else {
                return;
            };
            edit_session(&mut edit.session);
        }
        self.sync_text_preview(cx);
    }

    /// Mutate only the session's caret/selection — no document write needed.
    fn with_text_session_move(
        &mut self,
        cx: &mut Context<Self>,
        move_session: impl FnOnce(&mut TextEditSession),
    ) {
        {
            let Some(edit) = self.text_edit.as_mut() else {
                return;
            };
            move_session(&mut edit.session);
        }
        self.reset_caret_blink(cx);
        self.notify_text_selection_changed(cx);
        cx.notify();
    }

    /// Nudge panel observers through the item's event stream after the text
    /// session's caret/selection moved. The properties panel deliberately does
    /// not observe the view (every canvas repaint would re-render it), so a
    /// sub-selection change must arrive as the same `SelectionChanged` event a
    /// node-selection change emits for it to re-read the selection typography.
    fn notify_text_selection_changed(&mut self, cx: &mut Context<Self>) {
        self.item.update(cx, |_, cx| {
            cx.emit(crate::document::FigItemEvent::TextSelectionChanged);
        });
    }

    fn sync_text_preview(&mut self, cx: &mut Context<Self>) {
        let item = self.item.clone();
        if let Some(edit) = self.text_edit.as_ref() {
            item.update(cx, |item, cx| {
                item.with_document(cx, |document| {
                    text_edit::apply_preview(&mut document.doc, &edit.session);
                    ((), DocChange::ContentPreview)
                });
            });
        }
        // Force the canvas caches (revision-keyed rendered images + GPU
        // surfaces) to be discarded. Pure style/color changes on text runs
        // during preview don't change geometry, so GPUI + our image caches
        // can otherwise reuse a stale frame until a move or other delta
        // dirties the canvas element. Clearing here makes the new color take
        // effect on the very next paint.
        self.invalidate_canvas_cache();
        self.reset_caret_blink(cx);
        self.notify_text_selection_changed(cx);
        cx.notify();
    }

    /// The live text session's sub-selection typography for the properties
    /// panel, when a session is open on `node`. `None` outside a session (the
    /// panel then shows the node's own style).
    pub(crate) fn text_selection_typography(
        &self,
        node: fanta_doc::NodeId,
    ) -> Option<crate::text_edit::SelectionTypography> {
        let edit = self.text_edit.as_ref()?;
        (edit.session.node_id() == node).then(|| edit.session.selection_typography())
    }

    pub(crate) fn text_selection_buffer(
        &self,
        node: fanta_doc::NodeId,
    ) -> Option<fanta_text::TextBuffer> {
        let edit = self.text_edit.as_ref()?;
        (edit.session.node_id() == node).then(|| edit.session.buffer_snapshot())
    }

    pub(crate) fn retain_text_selection_on_next_focus_out(
        &mut self,
        node: fanta_doc::NodeId,
    ) -> bool {
        let Some(edit) = self.text_edit.as_mut() else {
            return false;
        };
        if edit.session.node_id() != node {
            return false;
        }
        edit.retain_on_next_focus_out = true;
        true
    }

    pub(crate) fn restore_text_selection_buffer(
        &mut self,
        node: fanta_doc::NodeId,
        buffer: fanta_text::TextBuffer,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(edit) = self.text_edit.as_mut() else {
            return false;
        };
        if edit.session.node_id() != node {
            return false;
        }
        edit.session.restore_buffer_snapshot(buffer);
        self.sync_text_preview(cx);
        true
    }

    /// If there is an active text edit session, apply style change only to the
    /// current selection (for rich text). Returns true if it was applied to a
    /// selection (caller can skip whole-node mutate). Use from properties for
    /// color/font etc on partial text.
    pub(crate) fn with_text_selection_style(
        &mut self,
        cx: &mut Context<Self>,
        patch: impl Fn(&mut fanta_text::TextStyle),
    ) -> bool {
        if let Some(edit) = self.text_edit.as_mut() {
            if let Err(error) = edit.session.patch_style_to_selection(patch) {
                log::error!("fig_viewer failed to style the text selection: {error}");
                return false;
            }
            self.sync_text_preview(cx);
            return true;
        }
        false
    }

    fn text_edit_insert(&mut self, text: &str, cx: &mut Context<Self>) {
        self.with_text_session_edit(cx, |session| session.insert(text));
    }

    fn text_edit_vertical_move(&mut self, down: bool, extend: bool, cx: &mut Context<Self>) {
        let target = {
            let Some(edit) = self.text_edit.as_ref() else {
                return;
            };
            let Some(document) = self.item.read(cx).document() else {
                return;
            };
            text_edit::session_vertical_move_target(
                &document.doc,
                &edit.session,
                edit.session.caret(),
                down,
            )
        };
        if let Some(target) = target {
            self.with_text_session_move(cx, |session| session.move_to(target, extend));
        }
    }

    fn text_edit_copy(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self
            .text_edit
            .as_ref()
            .and_then(|edit| edit.session.selected_text())
        {
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
        }
    }

    fn text_edit_cut(&mut self, cx: &mut Context<Self>) {
        self.text_edit_copy(cx);
        let has_selection = self
            .text_edit
            .as_ref()
            .is_some_and(|edit| !edit.session.selected_range().is_empty());
        if has_selection {
            self.text_edit_insert("", cx);
        }
    }

    fn text_edit_paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        self.text_edit_insert(&text, cx);
    }

    /// Restart the caret blink phase (solid now, then blinking) and schedule
    /// the repaints that animate it. Visibility itself is derived from elapsed
    /// time in [`CanvasTextEdit::caret_visible`]; this task exists only to wake
    /// the view at each phase boundary so the derived value is re-rendered. It
    /// re-arms every cycle, so a single missed wake just slows the blink rather
    /// than stalling it — and because the caret is solid until the first
    /// boundary, it is always visible the moment editing starts, even if the
    /// wake never fires.
    fn reset_caret_blink(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.text_edit.as_mut() else {
            return;
        };
        let epoch = edit.reset_blink();
        edit.blink_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(CARET_BLINK_INTERVAL).await;
                let still_blinking = this.update(cx, |this, cx| {
                    let Some(edit) = this.text_edit.as_ref() else {
                        return false;
                    };
                    if edit.blink_epoch != epoch {
                        return false;
                    }
                    // The derived visibility flipped across this boundary;
                    // repaint so the caret shows/hides.
                    cx.notify();
                    true
                });
                if !matches!(still_blinking, Ok(true)) {
                    break;
                }
            }
        }));
    }

    /// Keys the session resolves itself. These arrive here only when no key
    /// binding claimed them: the FigViewer keymap context is renamed to
    /// `FigViewerTextEdit` while a session is live (see `render`), which has
    /// no bindings, so tool shortcuts and canvas keys are suppressed and
    /// everything falls through — printable characters continue on to the
    /// platform input handler (`EntityInputHandler`), which also carries IME
    /// composition.
    pub(crate) fn handle_text_edit_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.text_edit.is_none() {
            return;
        }
        let keystroke = &event.keystroke;
        let shift = keystroke.modifiers.shift;
        let command = keystroke.modifiers.platform;
        let handled = match keystroke.key.as_str() {
            // Escape commits and exits — Figma commits on escape.
            "escape" => {
                self.commit_text_edit(cx);
                true
            }
            "enter" if command => {
                self.commit_text_edit(cx);
                true
            }
            "enter" => {
                self.text_edit_insert("\n", cx);
                true
            }
            "backspace" => {
                self.with_text_session_edit(cx, |session| session.backspace());
                true
            }
            "delete" => {
                self.with_text_session_edit(cx, |session| session.delete_forward());
                true
            }
            "left" if command => {
                self.with_text_session_move(cx, |session| session.move_line_start(shift));
                true
            }
            "right" if command => {
                self.with_text_session_move(cx, |session| session.move_line_end(shift));
                true
            }
            "home" => {
                self.with_text_session_move(cx, |session| session.move_line_start(shift));
                true
            }
            "end" => {
                self.with_text_session_move(cx, |session| session.move_line_end(shift));
                true
            }
            "left" => {
                self.with_text_session_move(cx, |session| session.move_left(shift));
                true
            }
            "right" => {
                self.with_text_session_move(cx, |session| session.move_right(shift));
                true
            }
            "up" if command => {
                self.with_text_session_move(cx, |session| session.move_to(0, shift));
                true
            }
            "down" if command => {
                self.with_text_session_move(cx, |session| {
                    session.move_to(session.buffer().len(), shift)
                });
                true
            }
            "up" => {
                self.text_edit_vertical_move(false, shift, cx);
                true
            }
            "down" => {
                self.text_edit_vertical_move(true, shift, cx);
                true
            }
            "a" if command => {
                self.with_text_session_move(cx, |session| session.select_all());
                true
            }
            "c" if command => {
                self.text_edit_copy(cx);
                true
            }
            "x" if command => {
                self.text_edit_cut(cx);
                true
            }
            "v" if command => {
                self.text_edit_paste(cx);
                true
            }
            // Swallow document undo/redo while typing: the session has no
            // per-keystroke history and a document undo would fight the
            // transient preview.
            "z" if command => true,
            _ => false,
        };
        if handled {
            cx.stop_propagation();
        }
    }

    /// The caret + selection overlay, plus the element that registers this
    /// view as the window's text-input handler while a session is live. The
    /// glyphs themselves are painted by the canvas renderer from the live
    /// (previewed) node content; only the caret bar and the selection
    /// highlight are drawn here, at geometry measured from the same shaped
    /// layout the renderer paints.
    pub(crate) fn render_text_edit_overlay(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let edit = self.text_edit.as_ref()?;
        let bounds = self.container_bounds?;
        let viewport = self.viewport?;
        let (width, height) = bounds_size(bounds);
        let screen_size = DVec2::new(width, height);
        let session = &edit.session;
        let document = self.item.read(cx).document()?;
        let doc = &document.doc;

        // A translucent highlight behind the glyphs and an opaque caret bar,
        // mirroring the original fanta-app (selection ~27% alpha, solid caret).
        // `alpha` sets an ABSOLUTE alpha, so the highlight is visible under any
        // loaded theme regardless of the `selection` swatch's own alpha (a
        // theme whose selection color is near-transparent would otherwise make
        // the highlight invisible); the caret is forced fully opaque.
        // Force a clearly visible highlight for text selection (blue-ish, semi
        // transparent) so it always shows during edit, independent of theme
        // player colors. Caret is solid accent.
        let selection_color = gpui::hsla(0.6, 0.85, 0.55, 0.45);
        let caret_color = cx.theme().players().local().cursor.alpha(1.0);

        let to_bounds = |[x, y, w, h]: [f64; 4]| Bounds {
            origin: point(px(x as f32), px(y as f32)),
            size: size(px(w as f32), px(h as f32)),
        };
        let selection_rects: Vec<Bounds<Pixels>> = text_edit::session_selection_rects(
            doc,
            session,
            session.selected_range(),
            &viewport,
            screen_size,
        )
        .into_iter()
        .map(to_bounds)
        .collect();
        let caret = edit
            .caret_visible()
            .then(|| {
                text_edit::session_caret_segment(
                    doc,
                    session,
                    session.caret(),
                    &viewport,
                    screen_size,
                )
            })
            .flatten()
            .map(|(top, bottom)| {
                // A hairline bar that thickens slightly with zoom, like Figma.
                let caret_width = (viewport.zoom * 1.5).clamp(1.0, 3.0);
                to_bounds([
                    top.x - caret_width * 0.5,
                    top.y.min(bottom.y),
                    caret_width,
                    (bottom.y - top.y).abs().max(1.0),
                ])
            });

        // The selection/caret rects from the helpers are in "viewport screen"
        // space (0,0 at top-left of the container's content area). The overlay
        // element is absolute full-size at the outer level, so its canvas_bounds
        // origin is the outer editor origin. Use the container's origin so the
        // highlight quads land exactly over the text glyphs.
        let container_origin = self
            .container_bounds
            .map(|b| b.origin)
            .unwrap_or(point(px(0.), px(0.)));

        let entity = cx.entity();
        let focus_handle = self.focus_handle.clone();
        Some(
            canvas(
                |_, _, _| {},
                move |canvas_bounds, _, window, cx| {
                    window.handle_input(
                        &focus_handle,
                        ElementInputHandler::new(canvas_bounds, entity),
                        cx,
                    );
                    window.with_content_mask(
                        Some(ContentMask {
                            bounds: canvas_bounds,
                        }),
                        |window| {
                            let offset = container_origin;
                            for rect in selection_rects {
                                let rect = Bounds {
                                    origin: offset + rect.origin,
                                    size: rect.size,
                                };
                                window.paint_quad(fill(rect, selection_color));
                            }
                            if let Some(rect) = caret {
                                let rect = Bounds {
                                    origin: offset + rect.origin,
                                    size: rect.size,
                                };
                                window.paint_quad(fill(rect, caret_color));
                            }
                        },
                    );
                },
            )
            // `absolute` alone resolves to the element's STATIC position — which,
            // stacked after `CanvasElement`, lands a full viewport below the
            // canvas. The overlay then content-masks its own quads away and the
            // caret/selection never appear. Pinning the insets anchors it to the
            // container's top-left, so its bounds coincide with the canvas.
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .into_any_element(),
        )
    }
}

/// Platform text input (typing and IME composition) while a text session is
/// live. Registered on the window by the overlay element during paint (see
/// `render_text_edit_overlay`), so printable keystrokes, dead keys, and
/// multi-stroke IME input all land in the session and preview onto the
/// canvas.
impl EntityInputHandler for FigView {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let session = &self.text_edit.as_ref()?.session;
        let range = session.range_from_utf16(&range_utf16);
        *adjusted_range = Some(session.range_to_utf16(&range));
        session.buffer().get(range).map(str::to_string)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let session = &self.text_edit.as_ref()?.session;
        Some(UTF16Selection {
            range: session.range_to_utf16(&session.selected_range()),
            reversed: session.selection_reversed(),
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let session = &self.text_edit.as_ref()?.session;
        session
            .marked_range()
            .map(|range| session.range_to_utf16(&range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        if let Some(edit) = self.text_edit.as_mut() {
            edit.session.clear_marked();
        }
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        {
            let Some(edit) = self.text_edit.as_mut() else {
                return;
            };
            let range = range_utf16
                .map(|range| edit.session.range_from_utf16(&range))
                .or_else(|| edit.session.marked_range())
                .unwrap_or_else(|| edit.session.selected_range());
            edit.session.replace_range(range, text);
        }
        self.sync_text_preview(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        {
            let Some(edit) = self.text_edit.as_mut() else {
                return;
            };
            let range = range_utf16
                .map(|range| edit.session.range_from_utf16(&range))
                .or_else(|| edit.session.marked_range())
                .unwrap_or_else(|| edit.session.selected_range());
            // The composition's requested selection is relative to the new
            // marked text, so its UTF-16 offsets resolve against `new_text`.
            let relative_selection = new_selected_range_utf16.map(|relative| {
                text_edit::offset_from_utf16(new_text, relative.start)
                    ..text_edit::offset_from_utf16(new_text, relative.end)
            });
            edit.session
                .replace_and_mark(range, new_text, relative_selection);
        }
        self.sync_text_preview(cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let edit = self.text_edit.as_ref()?;
        let session = &edit.session;
        let byte = session.offset_from_utf16(range_utf16.start);
        let viewport = self.viewport?;
        let bounds = self.container_bounds?;
        let (width, height) = bounds_size(bounds);
        let document = self.item.read(cx).document()?;
        let (top, bottom) = text_edit::session_caret_segment(
            &document.doc,
            session,
            byte,
            &viewport,
            DVec2::new(width, height),
        )?;
        Some(Bounds::from_corners(
            point(
                element_bounds.origin.x + px(top.x as f32),
                element_bounds.origin.y + px(top.y.min(bottom.y) as f32),
            ),
            point(
                element_bounds.origin.x + px(top.x as f32 + 2.0),
                element_bounds.origin.y + px(top.y.max(bottom.y) as f32),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let edit = self.text_edit.as_ref()?;
        let viewport = self.viewport?;
        let bounds = self.container_bounds?;
        let (width, height) = bounds_size(bounds);
        let screen = screen_position_in_bounds(position, bounds);
        let document = self.item.read(cx).document()?;
        let byte = text_edit::session_byte_at_screen(
            &document.doc,
            &edit.session,
            screen,
            &viewport,
            DVec2::new(width, height),
        )?;
        Some(edit.session.offset_to_utf16(byte))
    }

    fn accepts_text_input(&self, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        self.text_edit.is_some()
    }
}
