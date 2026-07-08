//! The workspace item view for Figma documents: canvas chrome (floating tool
//! pill, page picker, project controls), input routing into the tool shell,
//! and the `Item` integration that gives fanta projects text-editor-style
//! dirty tracking and save.

use std::ops::Range;

use anyhow::Result;
use fanta_canvas::HitPrecision;
use fanta_doc::{NodeData, NodeId, Viewport};
use fanta_tools::{Button as ToolButton, LogicalKey, ToolEvent};
use file_icons::FileIcons;
use glam::DVec2;
use gpui::{
    Action, Anchor, AnyElement, App, Bounds, ClipboardItem, ContentMask, Context, CursorStyle,
    ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PinchEvent, Pixels,
    Point, Render, ScrollDelta, ScrollWheelEvent, SharedString, Subscription, Task, UTF16Selection,
    Window, actions, canvas, div, fill, point, px, size,
};
use language::Capability;
use project::Project;
use settings::Settings as _;
use ui::{ContextMenu, ContextMenuEntry, Divider, IconPosition, PopoverMenu, Tooltip, prelude::*};
use util::paths::PathExt;
use workspace::{
    ItemSettings, Pane,
    item::{Item, ItemEvent, ProjectItem, SaveOptions, TabContentParams},
};

use crate::canvas::{CanvasElement, RenderedCanvas, bounds_size, screen_position_in_bounds};
use crate::document::{DocChange, FigItem, FigItemEvent};
use crate::text_edit::{self, CARET_BLINK_INTERVAL, CanvasTextEdit, TextEditSession};
use crate::tools::{
    TOOLBAR_GROUPS, ToolKind, ToolShell, key_event, move_event, pointer_button, press_event,
    release_event, tool_context,
};

#[cfg(target_os = "macos")]
use crate::canvas::MacGpuRenderer;

actions!(
    fig_viewer,
    [
        /// Zoom in the Figma canvas.
        ZoomIn,
        /// Zoom out the Figma canvas.
        ZoomOut,
        /// Reset canvas zoom to 100%.
        ResetZoom,
        /// Fit the current page to the pane.
        FitToView,
        /// Undo the last canvas operation.
        Undo,
        /// Redo the last undone canvas operation.
        Redo,
        /// Abort the in-flight gesture and clear the selection.
        Cancel,
        /// Confirm the in-flight gesture.
        Confirm,
        /// Delete the selected nodes.
        DeleteSelection,
        /// Nudge the selection left.
        NudgeLeft,
        /// Nudge the selection right.
        NudgeRight,
        /// Nudge the selection up.
        NudgeUp,
        /// Nudge the selection down.
        NudgeDown,
        /// Activate the move/select tool.
        ActivateSelectTool,
        /// Activate the hand (pan) tool.
        ActivateHandTool,
        /// Activate the rectangle tool.
        ActivateRectangleTool,
        /// Activate the ellipse tool.
        ActivateEllipseTool,
        /// Activate the line tool.
        ActivateLineTool,
        /// Activate the polygon tool.
        ActivatePolygonTool,
        /// Activate the star tool.
        ActivateStarTool,
        /// Activate the pen tool.
        ActivatePenTool,
        /// Activate the path editing tool.
        ActivateNodeEditTool,
        /// Activate the frame tool.
        ActivateFrameTool,
        /// Activate the text tool.
        ActivateTextTool,
        /// Activate the pencil (freehand) tool.
        ActivatePencilTool,
        /// Activate the section tool.
        ActivateSectionTool,
        /// Activate the slice tool.
        ActivateSliceTool,
        /// Activate the scale tool (placeholder).
        ActivateScaleTool,
        /// Activate the direct path-selection tool (placeholder).
        ActivatePathSelectTool,
        /// Activate the text-on-path tool (placeholder).
        ActivateTextPathTool,
    ]
);

/// The action that activates a given tool, so a toolbar dropdown row can both
/// dispatch it and display its keybinding.
fn action_for_kind(kind: ToolKind) -> Box<dyn Action> {
    match kind {
        ToolKind::Select => Box::new(ActivateSelectTool),
        ToolKind::PathSelect => Box::new(ActivatePathSelectTool),
        ToolKind::NodeEdit => Box::new(ActivateNodeEditTool),
        ToolKind::Hand => Box::new(ActivateHandTool),
        ToolKind::Scale => Box::new(ActivateScaleTool),
        ToolKind::Rect => Box::new(ActivateRectangleTool),
        ToolKind::Ellipse => Box::new(ActivateEllipseTool),
        ToolKind::Line => Box::new(ActivateLineTool),
        ToolKind::Polygon => Box::new(ActivatePolygonTool),
        ToolKind::Star => Box::new(ActivateStarTool),
        ToolKind::Pen => Box::new(ActivatePenTool),
        ToolKind::Pencil => Box::new(ActivatePencilTool),
        ToolKind::Frame => Box::new(ActivateFrameTool),
        ToolKind::Section => Box::new(ActivateSectionTool),
        ToolKind::Slice => Box::new(ActivateSliceTool),
        ToolKind::Text => Box::new(ActivateTextTool),
        ToolKind::TextPath => Box::new(ActivateTextPathTool),
    }
}

pub(crate) const MIN_ZOOM: f32 = 0.1;
pub(crate) const MAX_ZOOM: f32 = 20.0;
const ZOOM_STEP: f32 = 1.1;
const SCROLL_LINE_MULTIPLIER: f32 = 20.0;
pub(crate) const RENDER_PADDING: f64 = 48.0;

pub struct FigView {
    item: Entity<FigItem>,
    project: Entity<Project>,
    focus_handle: FocusHandle,
    selected_page_index: Option<usize>,
    /// Root node of the explicitly selected page, used to re-resolve
    /// `selected_page_index` when a disk reload reorders or removes pages.
    selected_page_root: Option<NodeId>,
    viewport: Option<Viewport>,
    pan_last_position: Option<Point<Pixels>>,
    primary_pressed: bool,
    /// Space is held: the canvas temporarily pans with any active tool, the
    /// Figma/Illustrator "hold space to pan" gesture. Cleared on key-up.
    space_pan: bool,
    container_bounds: Option<Bounds<Pixels>>,
    pub(crate) rendered_canvas: Option<RenderedCanvas>,
    #[cfg(target_os = "macos")]
    gpu_renderer: Option<MacGpuRenderer>,
    tools: ToolShell,
    /// The last-used tool per toolbar group, so each group's button keeps
    /// showing the member you last picked (Figma behavior). Indexed by group.
    group_faces: Vec<ToolKind>,
    /// Set once the document's fonts have been queued for background download,
    /// so the one-shot prewarm doesn't re-fire every frame.
    fonts_prewarmed: bool,
    hovered_node: Option<NodeId>,
    /// The in-place text-editing session, when a text node is being edited.
    text_edit: Option<CanvasTextEdit>,
    /// A text node waiting for a session to open. The text tool commits on
    /// a release delivered through the canvas's window-level mouse listener,
    /// which has no `Window`, so the session is opened on the next render.
    pending_text_edit: Option<NodeId>,
    _item_subscription: Subscription,
}

pub enum FigViewEvent {
    Edited,
    TitleChanged,
}

impl EventEmitter<FigViewEvent> for FigView {}

/// How a freshly opened text session seeds its selection: the text tool and
/// enter-to-edit select everything (the first keystroke replaces it), a
/// double-click selects the word under the cursor.
enum TextEditSeed {
    SelectAll,
    WordAt(DVec2),
}

impl FigView {
    fn new(
        item: Entity<FigItem>,
        project: Entity<Project>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let item_subscription = Self::subscribe_to_item(&item, cx);
        Self {
            item,
            project,
            focus_handle: cx.focus_handle(),
            selected_page_index: None,
            selected_page_root: None,
            viewport: None,
            pan_last_position: None,
            primary_pressed: false,
            space_pan: false,
            container_bounds: None,
            rendered_canvas: None,
            #[cfg(target_os = "macos")]
            gpu_renderer: None,
            tools: ToolShell::new(),
            group_faces: crate::tools::initial_group_faces(),
            fonts_prewarmed: false,
            hovered_node: None,
            text_edit: None,
            pending_text_edit: None,
            _item_subscription: item_subscription,
        }
    }

    fn subscribe_to_item(item: &Entity<FigItem>, cx: &mut Context<Self>) -> Subscription {
        cx.subscribe(item, |this, _, event: &FigItemEvent, cx| {
            match event {
                FigItemEvent::Edited => {
                    this.hovered_node = None;
                    // The edit may have removed the node under the inline
                    // text editor (undo, layer delete); the overlay must not
                    // outlive its target.
                    this.drop_text_edit_if_target_gone(cx);
                    cx.emit(FigViewEvent::Edited);
                }
                // Preview frames only need a canvas repaint; emitting an item
                // event per pointer move would spam tab updates.
                FigItemEvent::EditedTransient => {}
                FigItemEvent::SelectionChanged => {}
                FigItemEvent::StateChanged => {
                    // The node tree may have been swapped out (disk reload)
                    // or persisted; committing the overlay's stale text into
                    // the new tree could clobber external edits, so drop the
                    // inline text editor without committing.
                    this.text_edit = None;
                    this.pending_text_edit = None;
                    // A (re)loaded document restarts its scene revision
                    // counter, so cached frames keyed by revision must go.
                    this.rendered_canvas = None;
                    #[cfg(target_os = "macos")]
                    if let Some(renderer) = this.gpu_renderer.as_mut() {
                        renderer.invalidate();
                    }
                    // A disk reload swaps the node tree out from under the
                    // hover state and may reorder pages, so re-resolve the
                    // selected page by its root node.
                    this.hovered_node = None;
                    if let Some(root) = this.selected_page_root {
                        this.selected_page_index =
                            this.item.read(cx).document().and_then(|document| {
                                document
                                    .pages
                                    .iter()
                                    .position(|page| page.root == Some(root))
                            });
                        if this.selected_page_index.is_none() {
                            this.selected_page_root = None;
                        }
                    }
                    cx.emit(FigViewEvent::TitleChanged);
                }
                FigItemEvent::ConflictChanged => {
                    cx.emit(FigViewEvent::TitleChanged);
                }
            }
            cx.notify();
        })
    }

    pub fn item(&self) -> &Entity<FigItem> {
        &self.item
    }

    pub fn selected_page_index(&self) -> Option<usize> {
        self.selected_page_index
    }

    pub fn viewport(&self) -> Option<Viewport> {
        self.viewport
    }

    pub fn zoom_percent(&self) -> f64 {
        self.viewport
            .map(|viewport| viewport.zoom * 100.0)
            .unwrap_or(100.0)
    }

    pub(crate) fn tools(&self) -> &ToolShell {
        &self.tools
    }

    pub fn active_tool(&self) -> ToolKind {
        self.tools.kind()
    }

    pub(crate) fn hovered_node(&self) -> Option<NodeId> {
        self.hovered_node
    }

    pub(crate) fn is_panning(&self) -> bool {
        self.pan_last_position.is_some()
    }

    pub(crate) fn end_panning(&mut self, cx: &mut Context<Self>) {
        self.pan_last_position = None;
        cx.notify();
    }

    pub(crate) fn set_container_bounds(&mut self, bounds: Bounds<Pixels>) {
        self.container_bounds = Some(bounds);
    }

    pub(crate) fn set_viewport_silent(&mut self, viewport: Viewport) {
        self.viewport = Some(viewport);
    }

    pub(crate) fn clear_rendered_canvas(&mut self) {
        self.rendered_canvas = None;
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn take_gpu_renderer(&mut self) -> Option<MacGpuRenderer> {
        self.gpu_renderer.take()
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn store_gpu_renderer(&mut self, renderer: MacGpuRenderer) {
        self.gpu_renderer = Some(renderer);
    }

    fn is_editable(&self, cx: &App) -> bool {
        self.item.read(cx).is_editable()
    }

    fn set_viewport(&mut self, viewport: Viewport, cx: &mut Context<Self>) {
        self.viewport = Some(viewport);
        cx.notify();
    }

    /// Scroll the canvas so `id` is visible, centering it when it is currently
    /// off-screen — the canvas half of the layers-panel ↔ canvas sync (clicking
    /// a layer row reveals its node). Keeps the current zoom, and leaves the
    /// viewport untouched when the node is already fully visible so browsing the
    /// layer list doesn't jerk the canvas around.
    pub(crate) fn reveal_node_in_canvas(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let Some(viewport) = self.viewport else {
            return;
        };
        let Some(bounds) = self.container_bounds else {
            return;
        };
        let (width, height) = bounds_size(bounds);
        let screen_size = DVec2::new(width, height);
        let Some(node_bounds) = self
            .item
            .read(cx)
            .document()
            .and_then(|document| document.doc.scene.world_bounds(id))
        else {
            return;
        };
        if !node_bounds.is_finite() {
            return;
        }
        let visible_min = fanta_canvas::screen_to_world(DVec2::ZERO, &viewport, screen_size);
        let visible_max = fanta_canvas::screen_to_world(screen_size, &viewport, screen_size);
        let fully_visible = node_bounds.min_x >= visible_min.x
            && node_bounds.min_y >= visible_min.y
            && node_bounds.max_x <= visible_max.x
            && node_bounds.max_y <= visible_max.y;
        if fully_visible {
            return;
        }
        let center = node_bounds.center();
        self.set_viewport(
            Viewport {
                center: [center.x, center.y],
                zoom: viewport.zoom,
            },
            cx,
        );
    }

    /// Center the canvas on `id` and pick a zoom that fits it (never past 1:1
    /// for a small node), computing a fresh viewport rather than nudging the
    /// current one — the caller may have just switched pages, which clears the
    /// viewport. Used to jump to a component master after navigating to its
    /// (possibly hidden) page.
    pub(crate) fn focus_node(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let Some(bounds) = self.container_bounds else {
            return;
        };
        let (width, height) = bounds_size(bounds);
        let screen_size = DVec2::new(width, height);
        let Some(node_bounds) = self
            .item
            .read(cx)
            .document()
            .and_then(|document| document.doc.scene.world_bounds(id))
        else {
            return;
        };
        if !node_bounds.is_finite() {
            return;
        }
        let fit = fanta_canvas::fit_bounds(node_bounds, screen_size, RENDER_PADDING);
        let zoom = fit.zoom.min(1.0).clamp(MIN_ZOOM as f64, MAX_ZOOM as f64);
        let center = node_bounds.center();
        self.set_viewport(
            Viewport {
                center: [center.x, center.y],
                zoom,
            },
            cx,
        );
    }

    // === Tool routing =====================================================

    /// Send one event through the active tool, tracking whether it changed
    /// document content, only the selection, or nothing.
    fn dispatch_tool_event(&mut self, event: ToolEvent, cx: &mut Context<Self>) {
        let Some(bounds) = self.container_bounds else {
            return;
        };
        let Some(viewport) = self.viewport else {
            return;
        };
        let editable = self.is_editable(cx);
        if !editable && self.tools.kind() != ToolKind::Hand {
            self.handle_read_only_event(event, cx);
            return;
        }

        let screen_size = {
            let (width, height) = bounds_size(bounds);
            DVec2::new(width, height)
        };
        let viewport_before = viewport;
        let mut viewport = viewport;
        let is_preview_move = matches!(
            event,
            ToolEvent::Pointer(fanta_tools::PointerEvent::Move { .. })
        );
        let overlays_before = self.tools.overlays.clone();
        let cursor_before = self.tools.cursor;

        let tools = &mut self.tools;
        let mut wants_exit = false;
        let item = self.item.clone();
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let revision_before = document.doc.scene.revision();
                let selection_before: Vec<NodeId> =
                    document.doc.selection.iter().copied().collect();

                let mut ctx = tool_context(&mut document.doc, &mut viewport, screen_size);
                let response = tools.handle_event(&mut ctx, event);
                wants_exit = response.wants_exit;

                let revision_changed = document.doc.scene.revision() != revision_before;
                let selection_changed = !document
                    .doc
                    .selection
                    .iter()
                    .copied()
                    .eq(selection_before.iter().copied());
                let change = if revision_changed {
                    if is_preview_move {
                        DocChange::ContentPreview
                    } else {
                        DocChange::Content
                    }
                } else if selection_changed {
                    DocChange::Selection
                } else {
                    DocChange::None
                };
                ((), change)
            });
        });

        self.viewport = Some(viewport);
        if wants_exit {
            let was_text_tool = self.tools.kind() == ToolKind::Text;
            self.activate_tool(ToolKind::Select, cx);
            // The text tool commits its node and selects it before asking to
            // exit; drop the user straight into typing on it, like Figma.
            if was_text_tool && let Some(node) = self.selection_anchor_text_node(cx) {
                self.pending_text_edit = Some(node);
                cx.notify();
            }
            return;
        }
        // Document and selection changes already notify through the item's
        // event stream; only repaint here when something view-local changed.
        // An unconditional notify would re-render the whole view (and wake
        // observers) on every idle mouse move.
        if !crate::canvas::same_viewport(viewport_before, viewport)
            || self.tools.overlays != overlays_before
            || self.tools.cursor != cursor_before
        {
            cx.notify();
        }
    }

    /// Read-only documents still support click selection so the properties
    /// and layers panels stay useful before a project exists.
    fn handle_read_only_event(&mut self, event: ToolEvent, cx: &mut Context<Self>) {
        let ToolEvent::Pointer(fanta_tools::PointerEvent::Press {
            screen, modifiers, ..
        }) = event
        else {
            return;
        };
        let Some(bounds) = self.container_bounds else {
            return;
        };
        let Some(viewport) = self.viewport else {
            return;
        };
        let (width, height) = bounds_size(bounds);
        let screen_size = DVec2::new(width, height);
        let screen_point = DVec2::new(screen[0], screen[1]);
        let extend = modifiers.extend_selection();

        self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let hit = fanta_canvas::hit_test_screen(
                    &document.doc.scene,
                    &viewport,
                    screen_size,
                    screen_point,
                    HitPrecision::Path,
                    document.doc.active_page(),
                );
                match hit {
                    Some(node) if extend => document.doc.selection.toggle(node),
                    Some(node) => document.doc.selection.select_only(node),
                    None if !extend => document.doc.selection.clear(),
                    None => {}
                }
                ((), DocChange::Selection)
            });
        });
    }

    pub fn activate_tool(&mut self, kind: ToolKind, cx: &mut Context<Self>) {
        if kind.requires_editing() && !self.is_editable(cx) {
            return;
        }
        // Switching tools (toolbar click) while typing ends the session the
        // way any click-away does.
        self.commit_text_edit(cx);
        let Some(bounds) = self.container_bounds else {
            return;
        };
        let Some(viewport) = self.viewport else {
            return;
        };
        let (width, height) = bounds_size(bounds);
        let screen_size = DVec2::new(width, height);
        let mut viewport = viewport;

        let tools = &mut self.tools;
        let item = self.item.clone();
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let revision_before = document.doc.scene.revision();
                let mut ctx = tool_context(&mut document.doc, &mut viewport, screen_size);
                tools.activate(kind, &mut ctx);
                let change = if document.doc.scene.revision() != revision_before {
                    DocChange::Content
                } else {
                    DocChange::None
                };
                ((), change)
            });
        });
        // Remember this tool as its group's face so the group button keeps
        // showing it after switching to another group (Figma behavior).
        if let Some(index) = crate::tools::group_index_of(kind)
            && let Some(face) = self.group_faces.get_mut(index)
        {
            *face = kind;
        }
        self.viewport = Some(viewport);
        cx.notify();
    }

    /// Once the document is loaded, kick off a background download of every
    /// font family it uses (Figma-style auto-fetch) so a missing family is
    /// cached off the paint thread instead of stalling the first render that
    /// shapes it. Fires at most once per view.
    fn maybe_prewarm_fonts(&mut self, cx: &mut Context<Self>) {
        if self.fonts_prewarmed {
            return;
        }
        let Some(document) = self.item.read(cx).document() else {
            return; // document still loading — retry on a later render
        };
        self.fonts_prewarmed = true;
        let families = document.used_font_families();
        if families.is_empty() {
            return;
        }
        cx.background_spawn(async move {
            let fetched = fanta_text::prewarm_font_downloads(families.iter().map(String::as_str));
            if !fetched.is_empty() {
                log::info!("prewarmed {} document font(s): {fetched:?}", fetched.len());
            }
        })
        .detach();
    }

    // === Mouse handling ===================================================

    fn handle_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // While a text session is live, a left press inside the edited node
        // repositions the caret (double-click selects the word, shift
        // extends, and a drag from here selects); a press outside commits
        // the session and falls through so the click still selects whatever
        // was hit — Figma's click-away behavior.
        if event.button == MouseButton::Left
            && self.text_edit.is_some()
            && self.handle_text_edit_mouse_down(event, cx)
        {
            self.focus_handle.focus(window, cx);
            return;
        }

        // A double-click with the select tool on a text node opens the
        // in-place text editor instead of reaching the tool — Figma's
        // enter-text-edit gesture. The pair's first press already ran
        // selection through the tool; this press is consumed here.
        if event.button == MouseButton::Left
            && event.click_count == 2
            && self.tools.kind() == ToolKind::Select
            && self.is_editable(cx)
            && let Some(bounds) = self.container_bounds
        {
            let screen = screen_position_in_bounds(event.position, bounds);
            if let Some(node) = self.text_node_at(screen, cx) {
                self.open_text_edit(node, TextEditSeed::WordAt(screen), window, cx);
                return;
            }
        }

        self.focus_handle.focus(window, cx);
        let Some(bounds) = self.container_bounds else {
            return;
        };

        // Middle-drag, or a left-drag while space is held, pans regardless of
        // the active tool.
        if event.button == MouseButton::Middle
            || (event.button == MouseButton::Left && self.space_pan)
        {
            self.pan_last_position = Some(event.position);
            cx.notify();
            return;
        }

        let Some(button) = pointer_button(event.button) else {
            return;
        };
        if button == ToolButton::Primary {
            self.primary_pressed = true;
        }
        let screen = screen_position_in_bounds(event.position, bounds);
        self.dispatch_tool_event(
            press_event(screen, button, event.modifiers, event.click_count),
            cx,
        );
    }

    /// Track the space bar for hold-to-pan. Returns whether the key was the
    /// space bar (so the caller can stop it reaching tool shortcuts).
    fn handle_canvas_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "space" && self.text_edit.is_none() && !self.space_pan {
            self.space_pan = true;
            cx.notify();
        }
    }

    fn handle_canvas_key_up(
        &mut self,
        event: &gpui::KeyUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "space" && self.space_pan {
            self.space_pan = false;
            // A space-pan drag in flight ends with the key, not the button.
            if self.pan_last_position.is_some() {
                self.pan_last_position = None;
            }
            cx.notify();
        }
    }

    fn handle_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button == MouseButton::Middle {
            self.end_panning(cx);
        }
        // A space-pan drag ends on button release but space may still be held
        // for the next drag; end the pan, keep `space_pan`.
        if event.button == MouseButton::Left && self.space_pan && self.is_panning() {
            self.end_panning(cx);
            return;
        }
        if event.button == MouseButton::Left
            && let Some(edit) = self.text_edit.as_mut()
            && edit.session.dragging
        {
            edit.session.dragging = false;
        }
        // Primary releases are normally handled by the window-level listener
        // the canvas installs while a drag is live — but that listener only
        // exists after the paint FOLLOWING the press. A fast click can
        // release before that paint, which would strand `primary_pressed`
        // and turn every later hover move into a drag. Both paths funnel
        // through `handle_window_mouse_up`, which no-ops once the flag is
        // cleared, so a release never dispatches twice.
        self.handle_window_mouse_up(event, cx);
    }

    /// Window-level fallback installed by the canvas while a primary drag is
    /// in flight: element listeners stop firing once the cursor leaves the
    /// canvas, which would strand the tool mid-gesture.
    pub(crate) fn handle_window_mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        if !self.primary_pressed || event.button != MouseButton::Left {
            return;
        }
        let Some(bounds) = self.container_bounds else {
            return;
        };
        self.primary_pressed = false;
        let screen = screen_position_in_bounds(event.position, bounds);
        self.dispatch_tool_event(
            release_event(screen, ToolButton::Primary, event.modifiers),
            cx,
        );
    }

    pub(crate) fn handle_window_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) {
        if !self.primary_pressed {
            return;
        }
        let Some(bounds) = self.container_bounds else {
            return;
        };
        let screen = screen_position_in_bounds(event.position, bounds);
        self.dispatch_tool_event(move_event(screen, event.modifiers), cx);
    }

    pub(crate) fn primary_pressed(&self) -> bool {
        self.primary_pressed
    }

    fn handle_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_panning() {
            if let Some(last_position) = self.pan_last_position {
                let delta = event.position - last_position;
                if let Some(viewport) = self.viewport {
                    let delta_x = f64::from(f32::from(delta.x));
                    let delta_y = f64::from(f32::from(delta.y));
                    self.set_viewport(pan_viewport(viewport, (delta_x, delta_y)), cx);
                }
            }
            self.pan_last_position = Some(event.position);
            return;
        }
        // Moves during a primary drag are handled by the window-level
        // listener so the gesture keeps tracking outside the canvas.
        if self.primary_pressed {
            return;
        }
        // While space is held the canvas is in pan mode; suppress tool hover
        // so the cursor stays the grab hand with no selection highlights.
        if self.space_pan {
            return;
        }

        let Some(bounds) = self.container_bounds else {
            return;
        };
        let screen = screen_position_in_bounds(event.position, bounds);
        if self.text_edit.is_some() && self.handle_text_edit_mouse_move(screen, cx) {
            return;
        }
        self.dispatch_tool_event(move_event(screen, event.modifiers), cx);
        self.update_hover(screen, cx);
    }

    fn update_hover(&mut self, screen: DVec2, cx: &mut Context<Self>) {
        if self.tools.kind() != ToolKind::Select {
            if self.hovered_node.take().is_some() {
                cx.notify();
            }
            return;
        }
        let Some(bounds) = self.container_bounds else {
            return;
        };
        let Some(viewport) = self.viewport else {
            return;
        };
        let (width, height) = bounds_size(bounds);
        let hovered = {
            let item = self.item.read(cx);
            item.document().and_then(|document| {
                fanta_canvas::hit_test_screen(
                    &document.doc.scene,
                    &viewport,
                    DVec2::new(width, height),
                    screen,
                    HitPrecision::Bounds,
                    document.doc.active_page(),
                )
            })
        };
        if hovered != self.hovered_node {
            self.hovered_node = hovered;
            cx.notify();
        }
    }

    fn handle_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.modifiers.control || event.modifiers.platform {
            let delta: f32 = match event.delta {
                ScrollDelta::Pixels(pixels) => pixels.y.into(),
                ScrollDelta::Lines(lines) => lines.y * SCROLL_LINE_MULTIPLIER,
            };
            let zoom_factor = if delta > 0.0 {
                1.0 + delta.abs() * 0.01
            } else {
                1.0 / (1.0 + delta.abs() * 0.01)
            };
            self.zoom_by(f64::from(zoom_factor), Some(event.position), cx);
        } else {
            let delta = match event.delta {
                ScrollDelta::Pixels(pixels) => pixels,
                ScrollDelta::Lines(lines) => lines.map(|line| px(line * SCROLL_LINE_MULTIPLIER)),
            };
            if let Some(viewport) = self.viewport {
                let delta_x = f64::from(f32::from(delta.x));
                let delta_y = f64::from(f32::from(delta.y));
                self.set_viewport(pan_viewport(viewport, (delta_x, delta_y)), cx);
            }
        }
    }

    fn handle_pinch(&mut self, event: &PinchEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.zoom_by(f64::from(1.0 + event.delta), Some(event.position), cx);
    }

    // === Zoom =============================================================

    fn zoom_in(&mut self, _: &ZoomIn, _window: &mut Window, cx: &mut Context<Self>) {
        self.zoom_by(f64::from(ZOOM_STEP), None, cx);
    }

    fn zoom_out(&mut self, _: &ZoomOut, _window: &mut Window, cx: &mut Context<Self>) {
        self.zoom_by(1.0 / f64::from(ZOOM_STEP), None, cx);
    }

    fn reset_zoom(&mut self, _: &ResetZoom, _window: &mut Window, cx: &mut Context<Self>) {
        self.fit_page_to_view(cx);
    }

    fn fit_to_view(&mut self, _: &FitToView, _window: &mut Window, cx: &mut Context<Self>) {
        self.fit_page_to_view(cx);
    }

    fn fit_page_to_view(&mut self, cx: &mut Context<Self>) {
        let viewport = self
            .container_bounds
            .zip(self.item.read(cx).document())
            .and_then(|(bounds, document)| {
                let page = document.page(self.selected_page_index)?;
                // Compute bounds live: the cached page bounds are only
                // refreshed on load and page solve, not after every edit.
                let page_bounds = crate::document::page_bounds(&document.doc, page.root);
                Some(crate::document::fit_bounds(
                    page_bounds,
                    bounds_size(bounds),
                    RENDER_PADDING,
                    MIN_ZOOM,
                    MAX_ZOOM,
                ))
            });
        if let Some(viewport) = viewport {
            self.set_viewport(viewport, cx);
        }
    }

    fn zoom_by(&mut self, factor: f64, anchor: Option<Point<Pixels>>, cx: &mut Context<Self>) {
        let Some((viewport, bounds)) = self.viewport.zip(self.container_bounds) else {
            return;
        };
        let anchor = anchor
            .map(|position| {
                let screen = screen_position_in_bounds(position, bounds);
                (screen.x, screen.y)
            })
            .unwrap_or_else(|| {
                let size = bounds_size(bounds);
                (size.0 * 0.5, size.1 * 0.5)
            });
        self.set_viewport(
            zoom_viewport_at(viewport, anchor, factor, bounds_size(bounds)),
            cx,
        );
    }

    // === Edit actions =====================================================

    fn undo(&mut self, _: &Undo, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_editable(cx) {
            return;
        }
        self.item.update(cx, |item, cx| {
            if let Err(error) = item.undo(cx) {
                log::error!("fig_viewer undo failed: {error:#}");
            }
        });
    }

    fn redo(&mut self, _: &Redo, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_editable(cx) {
            return;
        }
        self.item.update(cx, |item, cx| {
            if let Err(error) = item.redo(cx) {
                log::error!("fig_viewer redo failed: {error:#}");
            }
        });
    }

    fn cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        // Escape while editing text commits and exits the session (Figma
        // commits on escape); this normally arrives through the key-down
        // listener since the FigViewer keymap context is renamed while
        // editing, but guard here too for programmatic dispatch.
        if self.text_edit.is_some() {
            self.commit_text_edit(cx);
            return;
        }
        if self.is_editable(cx) {
            self.dispatch_tool_event(key_event(LogicalKey::Escape, window.modifiers()), cx);
        }
        // Escape clears whatever selection the gesture left behind, matching
        // Figma.
        self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                if document.doc.selection.is_empty() {
                    ((), DocChange::None)
                } else {
                    document.doc.selection.clear();
                    ((), DocChange::Selection)
                }
            });
        });
        cx.notify();
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if self.text_edit.is_some() {
            self.commit_text_edit(cx);
            return;
        }
        // Enter with a single text node selected drops into editing it with
        // everything selected — Figma's enter-to-edit.
        if self.tools.kind() == ToolKind::Select
            && let Some(node) = self.single_selected_text_node(cx)
        {
            self.open_text_edit(node, TextEditSeed::SelectAll, window, cx);
            return;
        }
        if self.is_editable(cx) {
            self.dispatch_tool_event(key_event(LogicalKey::Enter, window.modifiers()), cx);
        }
    }

    fn delete_selection(
        &mut self,
        _: &DeleteSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_editable(cx) {
            self.dispatch_tool_event(key_event(LogicalKey::Delete, window.modifiers()), cx);
        }
    }

    fn nudge(&mut self, key: LogicalKey, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_editable(cx) {
            self.dispatch_tool_event(key_event(key, window.modifiers()), cx);
        }
    }

    // === Inline text editing ==============================================

    /// The topmost text node under `screen`, if any. The hit test returns
    /// leaves, so a text node is normally hit directly; walk up the ancestor
    /// chain for content nested under one (e.g. instance-expanded children).
    fn text_node_at(&self, screen: DVec2, cx: &App) -> Option<NodeId> {
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
        doc.scene
            .ancestors_of(hit)
            .find(|node| matches!(node.data, NodeData::Text(_)))
            .map(|node| node.id)
    }

    /// The selection anchor, when it is a text node — the node the text tool
    /// just committed and selected.
    fn selection_anchor_text_node(&self, cx: &App) -> Option<NodeId> {
        let item = self.item.read(cx);
        let document = item.document()?;
        let anchor = document.doc.selection.anchor()?;
        matches!(document.doc.scene.get(anchor)?.data, NodeData::Text(_)).then_some(anchor)
    }

    /// The single selected node, when it is a text node — the target for
    /// Figma's enter-to-edit.
    fn single_selected_text_node(&self, cx: &App) -> Option<NodeId> {
        let item = self.item.read(cx);
        let document = item.document()?;
        let &[node] = document.doc.selection.as_slice() else {
            return None;
        };
        matches!(document.doc.scene.get(node)?.data, NodeData::Text(_)).then_some(node)
    }

    /// Open an in-place editing session on `node`, committing any session
    /// already in flight on another node.
    fn open_text_edit(
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
            if edit.session.node_id() == node {
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
        match seed {
            TextEditSeed::SelectAll => session.select_all(),
            TextEditSeed::WordAt(screen) => {
                let byte =
                    self.viewport
                        .zip(self.container_bounds)
                        .and_then(|(viewport, bounds)| {
                            let (width, height) = bounds_size(bounds);
                            let document = self.item.read(cx).document()?;
                            text_edit::byte_at_screen(
                                &document.doc,
                                node,
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
        self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                if document.doc.selection.as_slice() == [node] {
                    ((), DocChange::None)
                } else {
                    document.doc.selection.select_only(node);
                    ((), DocChange::Selection)
                }
            });
        });
        // Focus leaving the canvas (panel field, pane switch) commits the
        // session, matching Figma's click-away semantics.
        let focus_out = cx.on_focus_out(&self.focus_handle, window, |this, _, _, cx| {
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
        let operation = self
            .item
            .update(cx, |item, cx| {
                item.with_document(cx, |document| {
                    text_edit::rewind_preview(&mut document.doc, &session);
                    let operation = text_edit::commit_operation(&document.doc, &session);
                    // The rewind never reaches the screen: the apply below
                    // repaints with the final content in the same cycle.
                    (operation, DocChange::None)
                })
            })
            .flatten();
        let Some(operation) = operation else {
            return;
        };
        self.item.update(cx, |item, cx| {
            if let Err(error) = item.apply(operation, cx) {
                log::error!("fig_viewer text edit failed to commit: {error:#}");
            }
        });
    }

    fn drop_text_edit_if_target_gone(&mut self, cx: &App) {
        let Some(edit) = self.text_edit.as_ref() else {
            return;
        };
        let target_exists = self.item.read(cx).document().is_some_and(|document| {
            document
                .doc
                .scene
                .get(edit.session.node_id())
                .is_some_and(|node| matches!(node.data, NodeData::Text(_)))
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
    fn handle_text_edit_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((viewport, bounds)) = self.viewport.zip(self.container_bounds) else {
            return false;
        };
        let Some(node) = self.text_edit.as_ref().map(|edit| edit.session.node_id()) else {
            return false;
        };
        let (width, height) = bounds_size(bounds);
        let screen_size = DVec2::new(width, height);
        let screen = screen_position_in_bounds(event.position, bounds);
        let byte = {
            let Some(document) = self.item.read(cx).document() else {
                return false;
            };
            let doc = &document.doc;
            if text_edit::node_contains_screen(doc, node, screen, &viewport, screen_size) {
                text_edit::byte_at_screen(doc, node, screen, &viewport, screen_size)
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
        cx.notify();
        true
    }

    /// Pointer movement while a session is live: track whether the cursor is
    /// over the edited node (for the I-beam) and extend a drag-selection.
    /// Returns true when the move was consumed by a drag-selection.
    fn handle_text_edit_mouse_move(&mut self, screen: DVec2, cx: &mut Context<Self>) -> bool {
        let Some((viewport, bounds)) = self.viewport.zip(self.container_bounds) else {
            return false;
        };
        let Some((node, dragging)) = self
            .text_edit
            .as_ref()
            .map(|edit| (edit.session.node_id(), edit.session.dragging))
        else {
            return false;
        };
        let (width, height) = bounds_size(bounds);
        let screen_size = DVec2::new(width, height);
        let (inside, byte) = {
            let Some(document) = self.item.read(cx).document() else {
                return false;
            };
            let doc = &document.doc;
            let inside = text_edit::node_contains_screen(doc, node, screen, &viewport, screen_size);
            let byte = if dragging {
                text_edit::byte_at_screen(doc, node, screen, &viewport, screen_size)
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
        cx.notify();
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
        self.reset_caret_blink(cx);
        cx.notify();
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
            text_edit::vertical_move_target(
                &document.doc,
                edit.session.node_id(),
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
    fn handle_text_edit_key_down(
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
    fn render_text_edit_overlay(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let edit = self.text_edit.as_ref()?;
        let bounds = self.container_bounds?;
        let viewport = self.viewport?;
        let (width, height) = bounds_size(bounds);
        let screen_size = DVec2::new(width, height);
        let session = &edit.session;
        let node = session.node_id();
        let document = self.item.read(cx).document()?;
        let doc = &document.doc;

        // A translucent highlight behind the glyphs and an opaque caret bar,
        // mirroring the original fanta-app (selection ~27% alpha, solid caret).
        // `alpha` sets an ABSOLUTE alpha, so the highlight is visible under any
        // loaded theme regardless of the `selection` swatch's own alpha (a
        // theme whose selection color is near-transparent would otherwise make
        // the highlight invisible); the caret is forced fully opaque.
        let selection_color = cx.theme().players().local().selection.alpha(0.3);
        let caret_color = cx.theme().players().local().cursor.alpha(1.0);

        let to_bounds = |[x, y, w, h]: [f64; 4]| Bounds {
            origin: point(px(x as f32), px(y as f32)),
            size: size(px(w as f32), px(h as f32)),
        };
        let selection_rects: Vec<Bounds<Pixels>> = text_edit::selection_screen_rects(
            doc,
            node,
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
                text_edit::caret_screen_segment(doc, node, session.caret(), &viewport, screen_size)
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
                            let offset = canvas_bounds.origin;
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
            .absolute()
            .size_full()
            .into_any_element(),
        )
    }

    // === Pages ============================================================

    pub fn select_page(&mut self, index: usize, cx: &mut Context<Self>) {
        // The edited node stays behind on the old page; end the session
        // before the canvas stops rendering it.
        self.commit_text_edit(cx);
        let root = self
            .item
            .update(cx, |item, cx| {
                item.with_document(cx, |document| {
                    document.ensure_page_solved(index);
                    let root = document.pages.get(index).and_then(|page| page.root);
                    document.doc.set_active_page(root);
                    (root, DocChange::Selection)
                })
            })
            .flatten();
        self.selected_page_index = Some(index);
        self.selected_page_root = root;
        self.viewport = None;
        self.hovered_node = None;
        cx.notify();
    }

    // === Chrome ===========================================================

    /// Commit any in-flight text session and persist the document. Returns the
    /// project directory the save materialized (only on the first save of a
    /// lone `.fig`, which turns it into an on-disk project), or `None` when the
    /// project already existed.
    fn save_document(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Task<Result<Option<std::path::PathBuf>>> {
        // Persist committed state, not a transient preview mid-session.
        self.commit_text_edit(cx);
        self.item.update(cx, |item, cx| item.save(cx))
    }

    fn render_tool_pill(&self, cx: &mut Context<Self>) -> AnyElement {
        let editable = self.is_editable(cx);
        let active = self.tools.kind();
        // Before any interaction the viewport is initialized silently during
        // paint, so fall back to the fit zoom the canvas will use rather than
        // showing a wrong 100%.
        let zoom = self
            .viewport
            .map(|viewport| viewport.zoom)
            .or_else(|| {
                let bounds = self.container_bounds?;
                let item = self.item.read(cx);
                let document = item.document()?;
                let page = document.page(self.selected_page_index)?;
                Some(
                    crate::document::fit_bounds(
                        page.bounds,
                        bounds_size(bounds),
                        RENDER_PADDING,
                        MIN_ZOOM,
                        MAX_ZOOM,
                    )
                    .zoom,
                )
            })
            .unwrap_or(1.0);
        let zoom_label: SharedString = format!("{:.0}%", zoom * 100.0).into();
        // The last-used tool per group drives each group button's face.
        let faces = self.group_faces.clone();

        h_flex()
            .absolute()
            .bottom_4()
            .left_0()
            .right_0()
            .justify_center()
            .child(
                h_flex()
                    .occlude()
                    .gap_1()
                    .px_1p5()
                    .py_1()
                    .rounded_xl()
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .elevation_2(cx)
                    .children(
                        TOOLBAR_GROUPS
                            .iter()
                            .enumerate()
                            .flat_map(|(group_index, group)| {
                                let group: &'static [ToolKind] = group;
                                let mut children: Vec<AnyElement> = Vec::new();
                                if group_index > 0 {
                                    children.push(
                                        div()
                                            .h(px(20.))
                                            .child(Divider::vertical())
                                            .into_any_element(),
                                    );
                                }
                                // The group's face = the active tool if it belongs
                                // to this group, else the last-used member.
                                let group_active = group.contains(&active);
                                let face_kind = if group_active {
                                    active
                                } else {
                                    faces.get(group_index).copied().unwrap_or(group[0])
                                };
                                let face_disabled = face_kind.requires_editing() && !editable;
                                let face_btn = IconButton::new(
                                    ("fig-tool-face", group_index),
                                    face_kind.icon(),
                                )
                                .toggle_state(group_active)
                                .icon_color(if group_active {
                                    Color::Accent
                                } else {
                                    Color::Default
                                })
                                .disabled(face_disabled)
                                .tooltip(Tooltip::text(face_kind.label()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.activate_tool(face_kind, cx);
                                }));
                                if group.len() <= 1 {
                                    children.push(face_btn.into_any_element());
                                    return children;
                                }
                                // Multi-tool group: face button + caret dropdown.
                                let caret = PopoverMenu::new(("fig-tool-group", group_index))
                                    .anchor(Anchor::BottomLeft)
                                    .trigger(
                                        IconButton::new(
                                            ("fig-tool-caret", group_index),
                                            IconName::ChevronDown,
                                        )
                                        .icon_size(IconSize::XSmall)
                                        .icon_color(Color::Muted),
                                    )
                                    .menu(move |window, cx| {
                                        Some(ContextMenu::build(
                                            window,
                                            cx,
                                            move |mut menu, _window, _cx| {
                                                for kind in group.iter().copied() {
                                                    let mut label = kind.label().to_string();
                                                    if kind.is_stub() {
                                                        label.push_str("  ·  soon");
                                                    }
                                                    let disabled =
                                                        kind.requires_editing() && !editable;
                                                    menu = menu.item(
                                                        ContextMenuEntry::new(label)
                                                            .icon(kind.icon())
                                                            .icon_position(IconPosition::Start)
                                                            .toggleable(
                                                                IconPosition::End,
                                                                kind == active,
                                                            )
                                                            .action(action_for_kind(kind))
                                                            .disabled(disabled),
                                                    );
                                                }
                                                menu
                                            },
                                        ))
                                    });
                                children.push(
                                    h_flex()
                                        .items_center()
                                        .child(face_btn)
                                        .child(caret)
                                        .into_any_element(),
                                );
                                children
                            }),
                    )
                    .child(div().h(px(20.)).child(Divider::vertical()))
                    .child(
                        IconButton::new("fig-zoom-out", IconName::Dash)
                            .icon_size(IconSize::Small)
                            .tooltip(Tooltip::for_action_title("Zoom Out", &ZoomOut))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.zoom_out(&ZoomOut, window, cx);
                            })),
                    )
                    .child(
                        Button::new("fig-zoom-label", zoom_label)
                            .size(ButtonSize::Compact)
                            .label_size(LabelSize::Small)
                            .color(Color::Muted)
                            .tooltip(Tooltip::for_action_title("Fit to View", &FitToView))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.fit_to_view(&FitToView, window, cx);
                            })),
                    )
                    .child(
                        IconButton::new("fig-zoom-in", IconName::Plus)
                            .icon_size(IconSize::Small)
                            .tooltip(Tooltip::for_action_title("Zoom In", &ZoomIn))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.zoom_in(&ZoomIn, window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }
}

struct FigViewSnapshot {
    loading_message: Option<SharedString>,
    error: Option<std::sync::Arc<anyhow::Error>>,
}

impl Render for FigView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Deferred open from the text tool's commit, which arrives through a
        // window-level mouse listener without a `Window`. Everything is
        // selected so the first keystroke replaces the placeholder.
        if let Some(node) = self.pending_text_edit.take() {
            self.open_text_edit(node, TextEditSeed::SelectAll, window, cx);
        }
        self.maybe_prewarm_fonts(cx);
        let snapshot = {
            let item = self.item.read(cx);
            FigViewSnapshot {
                loading_message: item.document.loading_message(),
                error: item.document.error(),
            }
        };
        let has_error = snapshot.error.is_some();
        let is_loading = snapshot.loading_message.is_some();
        let cursor_style = match &self.text_edit {
            // The I-beam over the edited text, an arrow elsewhere — clicking
            // away commits.
            Some(edit) if edit.session.pointer_inside => CursorStyle::IBeam,
            Some(_) => CursorStyle::Arrow,
            // Space-hold pan shows the grab/grabbing hand over any tool.
            None if self.space_pan && self.is_panning() => CursorStyle::ClosedHand,
            None if self.space_pan => CursorStyle::OpenHand,
            None => self.tools.cursor_style(self.is_panning()),
        };

        div()
            .track_focus(&self.focus_handle(cx))
            // While a text session is live the FigViewer keymap context must
            // not match: it binds bare letters (tool shortcuts), enter,
            // escape, backspace, and arrows, all of which belong to the text
            // session. `FigViewerTextEdit` has no bindings, so those keys
            // fall through to the session's key-down listener below and
            // printable characters continue to the platform input handler.
            .key_context(if self.text_edit.is_some() {
                "FigViewerTextEdit"
            } else {
                "FigViewer"
            })
            .when(self.text_edit.is_some(), |this| {
                this.on_key_down(cx.listener(Self::handle_text_edit_key_down))
            })
            .when(self.text_edit.is_none(), |this| {
                // Space-hold pan: tracked only outside text editing, where a
                // space is a literal character.
                this.on_key_down(cx.listener(Self::handle_canvas_key_down))
                    .on_key_up(cx.listener(Self::handle_canvas_key_up))
            })
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::reset_zoom))
            .on_action(cx.listener(Self::fit_to_view))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::delete_selection))
            .on_action(cx.listener(|this, _: &NudgeLeft, window, cx| {
                this.nudge(LogicalKey::ArrowLeft, window, cx)
            }))
            .on_action(cx.listener(|this, _: &NudgeRight, window, cx| {
                this.nudge(LogicalKey::ArrowRight, window, cx)
            }))
            .on_action(cx.listener(|this, _: &NudgeUp, window, cx| {
                this.nudge(LogicalKey::ArrowUp, window, cx)
            }))
            .on_action(cx.listener(|this, _: &NudgeDown, window, cx| {
                this.nudge(LogicalKey::ArrowDown, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ActivateSelectTool, _, cx| {
                this.activate_tool(ToolKind::Select, cx)
            }))
            .on_action(cx.listener(|this, _: &ActivateHandTool, _, cx| {
                this.activate_tool(ToolKind::Hand, cx)
            }))
            .on_action(cx.listener(|this, _: &ActivateRectangleTool, _, cx| {
                this.activate_tool(ToolKind::Rect, cx)
            }))
            .on_action(cx.listener(|this, _: &ActivateEllipseTool, _, cx| {
                this.activate_tool(ToolKind::Ellipse, cx)
            }))
            .on_action(cx.listener(|this, _: &ActivateLineTool, _, cx| {
                this.activate_tool(ToolKind::Line, cx)
            }))
            .on_action(cx.listener(|this, _: &ActivatePolygonTool, _, cx| {
                this.activate_tool(ToolKind::Polygon, cx)
            }))
            .on_action(cx.listener(|this, _: &ActivateStarTool, _, cx| {
                this.activate_tool(ToolKind::Star, cx)
            }))
            .on_action(
                cx.listener(|this, _: &ActivatePenTool, _, cx| {
                    this.activate_tool(ToolKind::Pen, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &ActivateNodeEditTool, _, cx| {
                this.activate_tool(ToolKind::NodeEdit, cx)
            }))
            .on_action(cx.listener(|this, _: &ActivateFrameTool, _, cx| {
                this.activate_tool(ToolKind::Frame, cx)
            }))
            .on_action(cx.listener(|this, _: &ActivateTextTool, _, cx| {
                this.activate_tool(ToolKind::Text, cx)
            }))
            .on_action(cx.listener(|this, _: &ActivatePencilTool, _, cx| {
                this.activate_tool(ToolKind::Pencil, cx)
            }))
            .on_action(cx.listener(|this, _: &ActivateSectionTool, _, cx| {
                this.activate_tool(ToolKind::Section, cx)
            }))
            .on_action(cx.listener(|this, _: &ActivateSliceTool, _, cx| {
                this.activate_tool(ToolKind::Slice, cx)
            }))
            .on_action(cx.listener(|this, _: &ActivateScaleTool, _, cx| {
                this.activate_tool(ToolKind::Scale, cx)
            }))
            .on_action(cx.listener(|this, _: &ActivatePathSelectTool, _, cx| {
                this.activate_tool(ToolKind::PathSelect, cx)
            }))
            .on_action(cx.listener(|this, _: &ActivateTextPathTool, _, cx| {
                this.activate_tool(ToolKind::TextPath, cx)
            }))
            .size_full()
            .relative()
            .bg(cx.theme().colors().editor_background)
            .when_some(snapshot.error, |this, error| {
                this.child(
                    v_flex()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .child(Label::new("Could not open Figma file").size(LabelSize::Large))
                        .child(Label::new(error.to_string()).color(Color::Muted)),
                )
            })
            .when_some(snapshot.loading_message, |this, message| {
                this.child(
                    v_flex()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .child(Label::new(message).size(LabelSize::Large))
                        .child(
                            Label::new("The canvas will appear as soon as parsing finishes.")
                                .color(Color::Muted),
                        ),
                )
            })
            .when(!has_error && !is_loading, |this| {
                this.child(
                    div()
                        .id("fig-container")
                        .size_full()
                        .overflow_hidden()
                        .cursor(cursor_style)
                        .on_scroll_wheel(cx.listener(Self::handle_scroll_wheel))
                        .on_pinch(cx.listener(Self::handle_pinch))
                        .on_mouse_down(MouseButton::Left, cx.listener(Self::handle_mouse_down))
                        .on_mouse_down(MouseButton::Middle, cx.listener(Self::handle_mouse_down))
                        .on_mouse_up(MouseButton::Left, cx.listener(Self::handle_mouse_up))
                        .on_mouse_up(MouseButton::Middle, cx.listener(Self::handle_mouse_up))
                        .on_mouse_move(cx.listener(Self::handle_mouse_move))
                        .child(CanvasElement::new(cx.entity())),
                )
                .child(self.render_tool_pill(cx))
                .children(self.render_text_edit_overlay(cx))
            })
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
        let (top, bottom) = text_edit::caret_screen_segment(
            &document.doc,
            session.node_id(),
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
        let byte = text_edit::byte_at_screen(
            &document.doc,
            edit.session.node_id(),
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

impl Item for FigView {
    type Event = FigViewEvent;

    fn to_item_events(event: &Self::Event, f: &mut dyn FnMut(ItemEvent)) {
        match event {
            FigViewEvent::Edited => {
                f(ItemEvent::Edit);
                f(ItemEvent::UpdateTab);
            }
            FigViewEvent::TitleChanged => {
                f(ItemEvent::UpdateTab);
            }
        }
    }

    fn for_each_project_item(
        &self,
        cx: &App,
        f: &mut dyn FnMut(gpui::EntityId, &dyn project::ProjectItem),
    ) {
        f(self.item.entity_id(), self.item.read(cx));
    }

    fn tab_content_text(&self, _: usize, cx: &App) -> SharedString {
        self.item.read(cx).title()
    }

    fn tab_tooltip_text(&self, cx: &App) -> Option<SharedString> {
        Some(
            self.item
                .read(cx)
                .abs_path()
                .compact()
                .to_string_lossy()
                .into_owned()
                .into(),
        )
    }

    fn tab_icon(&self, _: &Window, cx: &App) -> Option<Icon> {
        let item = self.item.read(cx);
        let path = item.abs_path();
        ItemSettings::get_global(cx)
            .file_icons
            .then(|| FileIcons::get_icon(path, cx))
            .flatten()
            .map(Icon::from_path)
    }

    fn buffer_kind(&self, _: &App) -> workspace::item::ItemBufferKind {
        workspace::item::ItemBufferKind::Singleton
    }

    fn capability(&self, cx: &App) -> Capability {
        if self.item.read(cx).is_editable() {
            Capability::ReadWrite
        } else {
            Capability::ReadOnly
        }
    }

    fn is_dirty(&self, cx: &App) -> bool {
        self.item.read(cx).is_dirty()
    }

    fn has_conflict(&self, cx: &App) -> bool {
        self.item.read(cx).has_conflict()
    }

    fn can_save(&self, cx: &App) -> bool {
        self.item.read(cx).is_editable()
    }

    fn save(
        &mut self,
        _options: SaveOptions,
        _project: Entity<Project>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let task = self.save_document(cx);
        let project = self.project.clone();
        cx.spawn(async move |_, cx| {
            let Some(root) = task.await? else {
                return Ok(());
            };
            log::info!("materialized Fanta project at {}", root.display());
            // First-class Zed integration: surface the freshly written project
            // as a *visible* worktree so its fanta.json / `.fnx` / asset files
            // show up in the project panel and open as ordinary text. A failure
            // here (e.g. a remote project that can't adopt a local folder) must
            // not fail the save — the project already exists on disk.
            let worktree = project.update(cx, |project, cx| {
                project.find_or_create_worktree(root.clone(), true, cx)
            });
            if let Err(error) = worktree.await {
                log::error!(
                    "adding materialized Fanta project {} to the workspace failed: {error:#}",
                    root.display()
                );
            }
            Ok(())
        })
    }

    fn reload(
        &mut self,
        _project: Entity<Project>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        self.item.update(cx, |item, cx| item.reload_from_disk(cx))
    }

    fn can_split(&self) -> bool {
        true
    }

    fn clone_on_split(
        &self,
        _workspace_id: Option<workspace::WorkspaceId>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<Option<Entity<Self>>>
    where
        Self: Sized,
    {
        let item = self.item.clone();
        let project = self.project.clone();
        let viewport = self.viewport;
        let selected_page_index = self.selected_page_index;
        let selected_page_root = self.selected_page_root;
        Task::ready(Some(cx.new(|cx| {
            let item_subscription = Self::subscribe_to_item(&item, cx);
            Self {
                item,
                project,
                focus_handle: cx.focus_handle(),
                selected_page_index,
                selected_page_root,
                viewport,
                pan_last_position: None,
                primary_pressed: false,
                space_pan: false,
                container_bounds: None,
                rendered_canvas: None,
                #[cfg(target_os = "macos")]
                gpu_renderer: None,
                tools: ToolShell::new(),
                group_faces: crate::tools::initial_group_faces(),
                fonts_prewarmed: false,
                hovered_node: None,
                text_edit: None,
                pending_text_edit: None,
                _item_subscription: item_subscription,
            }
        })))
    }

    fn tab_content(&self, params: TabContentParams, _window: &Window, cx: &App) -> AnyElement {
        Label::new(self.tab_content_text(params.detail.unwrap_or_default(), cx))
            .single_line()
            .color(params.text_color())
            .when(params.preview, |this| this.italic())
            .into_any_element()
    }
}

impl ProjectItem for FigView {
    type Item = FigItem;

    fn for_project_item(
        project: Entity<Project>,
        _: Option<&Pane>,
        item: Entity<Self::Item>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self
    where
        Self: Sized,
    {
        Self::new(item, project, window, cx)
    }
}

impl Focusable for FigView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ToolKind {
    fn requires_editing(self) -> bool {
        !matches!(self, Self::Select | Self::Hand)
    }
}

fn pan_viewport(viewport: Viewport, screen_delta: (f64, f64)) -> Viewport {
    let inv_zoom = 1.0 / viewport.zoom.max(f64::EPSILON);
    Viewport {
        center: [
            viewport.center[0] - screen_delta.0 * inv_zoom,
            viewport.center[1] - screen_delta.1 * inv_zoom,
        ],
        zoom: viewport.zoom,
    }
}

fn screen_to_world(screen: (f64, f64), viewport: Viewport, screen_size: (f64, f64)) -> (f64, f64) {
    let inv_zoom = 1.0 / viewport.zoom.max(f64::EPSILON);
    (
        (screen.0 - screen_size.0 * 0.5) * inv_zoom + viewport.center[0],
        (screen.1 - screen_size.1 * 0.5) * inv_zoom + viewport.center[1],
    )
}

fn zoom_viewport_at(
    viewport: Viewport,
    screen_anchor: (f64, f64),
    factor: f64,
    screen_size: (f64, f64),
) -> Viewport {
    let world_before = screen_to_world(screen_anchor, viewport, screen_size);
    let provisional = Viewport {
        center: viewport.center,
        zoom: (viewport.zoom * factor).clamp(f64::from(MIN_ZOOM), f64::from(MAX_ZOOM)),
    };
    let world_after = screen_to_world(screen_anchor, provisional, screen_size);
    Viewport {
        center: [
            provisional.center[0] - (world_after.0 - world_before.0),
            provisional.center[1] - (world_after.1 - world_before.1),
        ],
        zoom: provisional.zoom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zooming_at_an_anchor_keeps_the_anchored_world_point_fixed() {
        let viewport = Viewport {
            center: [100.0, -50.0],
            zoom: 2.0,
        };
        let screen_size = (800.0, 600.0);
        let anchor = (120.0, 90.0);
        let world_before = screen_to_world(anchor, viewport, screen_size);

        let zoomed = zoom_viewport_at(viewport, anchor, 1.5, screen_size);
        let world_after = screen_to_world(anchor, zoomed, screen_size);

        assert!((world_before.0 - world_after.0).abs() < 1e-9);
        assert!((world_before.1 - world_after.1).abs() < 1e-9);
        assert!((zoomed.zoom - 3.0).abs() < 1e-9);
    }

    #[test]
    fn zoom_clamps_to_viewer_limits() {
        let viewport = Viewport {
            center: [0.0, 0.0],
            zoom: 1.0,
        };
        let screen_size = (800.0, 600.0);
        let zoomed_out = zoom_viewport_at(viewport, (400.0, 300.0), 1e-6, screen_size);
        assert!((zoomed_out.zoom - f64::from(MIN_ZOOM)).abs() < 1e-9);
        let zoomed_in = zoom_viewport_at(viewport, (400.0, 300.0), 1e6, screen_size);
        assert!((zoomed_in.zoom - f64::from(MAX_ZOOM)).abs() < 1e-9);
    }

    #[test]
    fn panning_moves_the_center_against_the_screen_delta() {
        let viewport = Viewport {
            center: [10.0, 20.0],
            zoom: 2.0,
        };
        let panned = pan_viewport(viewport, (30.0, -10.0));
        assert!((panned.center[0] - (10.0 - 15.0)).abs() < 1e-9);
        assert!((panned.center[1] - (20.0 + 5.0)).abs() < 1e-9);
        assert!((panned.zoom - viewport.zoom).abs() < 1e-9);
    }
}
