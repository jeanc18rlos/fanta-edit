//! The workspace item view for Figma documents: canvas chrome (floating tool
//! pill, page picker, project controls), input routing into the tool shell,
//! and the `Item` integration that gives fanta projects text-editor-style
//! dirty tracking and save.

use std::collections::HashSet;

use anyhow::{Context as _, Result};
use fanta_canvas::HitPrecision;
use fanta_doc::{
    AnimationClip, AnimationClipId, AnimationTrack, AnimationTrackId, AssetId, BoundProp,
    CanvasNode, Doc, Easing, IndexKey, Interpolation, Keyframe, KeyframeId, MotionEvaluation,
    MotionProperty, MotionTarget, MotionTransform, NodeData, NodeId, Operation, ResolvedVarValue,
    Transaction, Viewport,
};
use fanta_tools::{Button as ToolButton, LogicalKey, ToolEvent};
use file_icons::FileIcons;
use glam::DVec2;
use gpui::{
    Action, Anchor, AnyElement, App, Bounds, ClipboardEntry, ClipboardItem, Context, CursorStyle,
    DragMoveEvent, Empty, Entity, EventEmitter, FocusHandle, Focusable, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PinchEvent, Pixels, Point, Render, RenderImage,
    ScrollDelta, ScrollWheelEvent, SharedString, Subscription, Task, Window, actions, div, px,
};
use language::Capability;
use project::{Project, ProjectEntryId};
use settings::{Settings as _, update_settings_file};
use smallvec::SmallVec;
use ui::{ContextMenu, ContextMenuEntry, Divider, IconPosition, PopoverMenu, Tooltip, prelude::*};
use util::{ResultExt, paths::PathExt};
use workspace::{
    ItemSettings, MultiWorkspace, Pane, Toast,
    item::{Item, ItemEvent, ProjectItem, SaveOptions, TabContentParams},
    notifications::NotificationId,
};

use crate::canvas::{
    CanvasElement, RenderedCanvas, bounds_size, evaluated_hit_test_screen,
    screen_position_in_bounds,
};
use crate::clipboard::{
    CanvasClipboard, ClipboardPlacement, apply_transaction as apply_canvas_transaction,
    create_operations, delete_operations,
};
use crate::code_workspace::FantaCodeWorkspace;
use crate::comments_panel::{FantaCommentsPanel, document_comment_rows};
use crate::design_panel::FantaDesignPanel;
use crate::document::{
    AssetStores, DocChange, FigDocument, FigItem, FigItemEvent, FigScope, MAX_IMAGE_SOURCE_BYTES,
    SaveKind, ScopeRequester,
};
use crate::editor_session::{
    EditorMode, EditorModeTabs, EditorSession, EditorWorkspace, EditorWorkspaceTabs,
};
use crate::motion_edit::{
    MotionKeyframeDragSession, delete_keyframe_operation, rename_clip_operation,
    set_clip_duration_operation, set_keyframe_easing_operation,
    set_keyframe_interpolation_operation,
};
use crate::motion_panel::{FantaMotionPanel, MotionPanelEvent};
use crate::panel_settings::{FantaDesignPanelSettings, FantaPropertiesPanelSettings};
use crate::properties_panel::FantaPropertiesPanel;
use crate::prototype_panel::FantaPrototypePanel;
use crate::prototype_player::PrototypePlayerState;
use crate::text_edit::CanvasTextEdit;
#[cfg(test)]
use crate::timeline::TIMELINE_HEIGHT;
use crate::timeline::{
    TimelineEditPhase, TimelineEvent, TimelineKeyframeSelection, TimelineKeyframeViewModel,
    TimelineProperty, TimelineShell, TimelineTrackViewModel, TimelineViewModel,
};
use crate::tools::{
    TOOLBAR_GROUPS, ToolKind, ToolShell, key_event, move_event, pointer_button, press_event,
    release_event, tool_context,
};
use crate::variables_workspace::FantaVariablesWorkspace;

#[cfg(target_os = "macos")]
use crate::canvas::{CanvasVideoFrame, GpuCanvas};
#[cfg(target_os = "macos")]
use crate::video_playback::VideoPlaybackView;

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
        /// Copy the selected canvas subtrees.
        CopySelection,
        /// Cut the selected canvas subtrees.
        CutSelection,
        /// Paste canvas subtrees from the clipboard.
        PasteSelection,
        /// Duplicate the selected canvas subtrees.
        DuplicateSelection,
        /// Group the selected layers.
        GroupSelection,
        /// Ungroup the selected groups and frames.
        UngroupSelection,
        /// Wrap the selected layers in a frame.
        FrameSelection,
        /// Present the authored prototype from its configured starting frame.
        PlayPrototype,
        /// Leave prototype presentation and return to the editor.
        ExitPrototype,
        /// Restart prototype presentation from its starting frame.
        RestartPrototype,
        /// Show the previous top-level frame in prototype presentation.
        PrototypePreviousFrame,
        /// Show the next top-level frame in prototype presentation.
        PrototypeNextFrame,
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
        /// Proportionally scale selected objects and their contents.
        ActivateScaleTool,
        /// Activate direct selection of vector anchors and segments.
        ActivatePathSelectTool,
        /// Activate the text-on-path tool (placeholder).
        ActivateTextPathTool,
        /// Activate the comment tool (click the canvas to pin a comment).
        ActivateCommentTool,
        /// Show or hide the embedded layers sidebar.
        ToggleLayersSidebar,
        /// Show or hide the embedded inspector sidebar.
        ToggleInspectorSidebar,
        /// Select every top-level node on the current page.
        SelectAll,
        /// Fit the viewport around the current selection.
        ZoomToSelection,
    ]
);

/// The action that represents a tool, so a toolbar dropdown row can display
/// its keybinding while the click path still activates the owning FigView
/// directly.
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
        ToolKind::Comment => Box::new(ActivateCommentTool),
    }
}

pub(crate) const MIN_ZOOM: f32 = 0.1;
pub(crate) const MAX_ZOOM: f32 = 20.0;
const ZOOM_STEP: f32 = 1.1;
const SCROLL_LINE_MULTIPLIER: f32 = 20.0;
pub(crate) const RENDER_PADDING: f64 = 48.0;
const MIN_LAYERS_SIDEBAR_WIDTH: f32 = 220.0;
const MIN_INSPECTOR_SIDEBAR_WIDTH: f32 = 260.0;
const MAX_SIDEBAR_WIDTH: f32 = 560.0;
const SIDEBAR_RESIZE_HANDLE_SIZE: Pixels = px(6.);
/// How long the canvas waits after the last committing edit before writing the
/// project tree. A Fanta project is meant to be readable as source by an agent
/// and reviewable as a diff, so edits reach disk on their own; the delay keeps
/// a burst of edits (or a gesture that commits per step) to one write.
const AUTOSAVE_DEBOUNCE: std::time::Duration = std::time::Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SidebarKind {
    Layers,
    Inspector,
}

#[derive(Clone)]
struct SidebarResizeDrag {
    sidebar: SidebarKind,
}

impl Render for SidebarResizeDrag {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

pub struct FigView {
    pub(crate) item: Entity<FigItem>,
    project: Entity<Project>,
    pub(crate) focus_handle: FocusHandle,
    editor_session: Entity<EditorSession>,
    layers_sidebar: Entity<FantaDesignPanel>,
    inspector_sidebar: Entity<FantaPropertiesPanel>,
    prototype_sidebar: Entity<FantaPrototypePanel>,
    motion_sidebar: Entity<FantaMotionPanel>,
    variables_workspace: Entity<FantaVariablesWorkspace>,
    code_workspace: Entity<FantaCodeWorkspace>,
    timeline_shell: Entity<TimelineShell>,
    active_motion_clip: Option<AnimationClipId>,
    motion_keyframe_drag: Option<MotionKeyframeDragSession>,
    layers_sidebar_visible: bool,
    inspector_sidebar_visible: bool,
    layers_sidebar_width: Pixels,
    inspector_sidebar_width: Pixels,
    selected_page_index: Option<usize>,
    /// Root node of the explicitly selected page, used to re-resolve
    /// `selected_page_index` when a disk reload reorders or removes pages.
    selected_page_root: Option<NodeId>,
    /// The worktree entry this view was opened from (the clicked `page.fnx`,
    /// `master.fnx`, `doc/variables.json`, manifest, or `.fig`). Reported to
    /// the pane so each opened path keeps its OWN tab even though every tab
    /// shares the project's one [`FigItem`].
    opened_entry_id: Option<ProjectEntryId>,
    /// What this tab is scoped to. Re-asserted on focus so the shared
    /// document's single render root follows the focused tab, and updated by
    /// in-tab navigation ([`Self::select_page`]).
    scope: Option<FigScope>,
    is_focused: bool,
    /// The document root this view last acted on: a `ScopeApplied` that
    /// merely restores our own root (switching back to this tab) must not
    /// clobber the saved viewport.
    last_seen_root: Option<NodeId>,
    pub(crate) viewport: Option<Viewport>,
    pan_last_position: Option<Point<Pixels>>,
    primary_pressed: bool,
    /// A pointer button is down on the canvas, from the press until the
    /// release (wherever it lands). Broader than `primary_pressed`, which the
    /// tool paths only set once an event reaches a tool: the autosave must
    /// also stand down for a text-selection drag or a comment click.
    canvas_pointer_down: bool,
    /// The debounced autosave armed by the last committing edit.
    autosave_task: Option<Task<()>>,
    /// The selection handle the cursor is over, resolved on hover so
    /// `render` only reads it — hit-testing eight handles per redraw would
    /// run on every window frame, not just on pointer movement.
    hover_resize_handle: Option<fanta_canvas::ResizeHandle>,
    /// Space is held: the canvas temporarily pans with any active tool, the
    /// Figma/Illustrator "hold space to pan" gesture. Cleared on key-up.
    space_pan: bool,
    pub(crate) container_bounds: Option<Bounds<Pixels>>,
    pub(crate) rendered_canvas: Option<RenderedCanvas>,
    /// Memoized selection-chrome geometry (frame labels, per-node selection
    /// bounds, text baselines), reused across paint frames while the scene
    /// revision, page root, selection, and text-edit target are unchanged —
    /// pans and zooms repaint every frame without touching any of them.
    /// Interior-mutable because the canvas element collects overlays during
    /// paint with only `&App`. Cleared with the rendered-canvas cache: a
    /// reload restarts the scene revision counter, which could collide.
    pub(crate) chrome_cache: std::cell::RefCell<Option<crate::canvas::ChromeCache>>,
    /// The Metal render thread and its newest frame; created on first paint.
    /// `pub(crate)` because the canvas element drives it during paint.
    #[cfg(target_os = "macos")]
    pub(crate) gpu_canvas: Option<GpuCanvas>,
    #[cfg(target_os = "macos")]
    canvas_video: Option<CanvasVideoSession>,
    #[cfg(target_os = "macos")]
    canvas_video_generation: u64,
    #[cfg(target_os = "macos")]
    canvas_video_removed: std::cell::Cell<bool>,
    #[cfg(target_os = "macos")]
    canvas_video_active: std::cell::Cell<bool>,
    pub(crate) tools: ToolShell,
    pub(crate) comment_state: crate::comments_ui::CommentState,
    /// The last-used tool per toolbar group, so each group's button keeps
    /// showing the member you last picked (Figma behavior). Indexed by group.
    group_faces: Vec<ToolKind>,
    #[cfg(feature = "fanta-gpui-ui")]
    gpui_toolbar: Option<crate::gpui_adapters::toolbar::ToolbarAdapter>,
    /// The DesignPanel inspector adapter, mounted for Design mode when the
    /// fanta-gpui runtime and the `FANTA_GPUI_DESIGN` gate allow it; the
    /// legacy `FantaPropertiesPanel` stays the fallback. `pub(crate)` because
    /// the adapter's host methods live in `gpui_adapters::design`.
    #[cfg(feature = "fanta-gpui-ui")]
    pub(crate) gpui_design: Option<crate::gpui_adapters::design::DesignAdapter>,
    /// Set once the document's fonts have been queued for background download,
    /// so the one-shot prewarm doesn't re-fire every frame.
    fonts_prewarmed: bool,
    hovered_node: Option<NodeId>,
    /// The in-place text-editing session, when a text node is being edited.
    pub(crate) text_edit: Option<CanvasTextEdit>,
    /// A text node waiting for a session to open. The text tool commits on
    /// a release delivered through the canvas's window-level mouse listener,
    /// which has no `Window`, so the session is opened on the next render.
    pending_text_edit: Option<NodeId>,
    prototype_player: Option<PrototypePlayerState>,
    prototype_saved_viewport: Option<Viewport>,
    prototype_tick_task: Option<Task<()>>,
    prototype_last_tick: Option<std::time::Instant>,
    prototype_pointer_down: Option<Point<Pixels>>,
    prototype_drag_fired: bool,
    prototype_suppress_click: bool,
    prototype_render_cache: Option<PrototypeRenderCache>,
    prototype_link_notice: Option<PrototypeLinkNotice>,
    _item_subscription: Subscription,
    _editor_session_subscription: Subscription,
    _motion_sidebar_subscription: Subscription,
    _timeline_subscription: Subscription,
}

pub enum FigViewEvent {
    Edited,
    TitleChanged,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CanvasVideoSource {
    scene: u64,
    node: NodeId,
    asset: AssetId,
    assets_identity: usize,
    bytes_identity: Option<(usize, usize)>,
    time_range_us: [i64; 2],
    speed_bits: u32,
}

#[cfg(target_os = "macos")]
struct CanvasVideoSession {
    source: CanvasVideoSource,
    loading: Option<Task<()>>,
    playback: Option<Entity<VideoPlaybackView>>,
    observation: Option<Subscription>,
    error: Option<SharedString>,
    audio: (bool, u32),
    bytes: Option<std::sync::Arc<[u8]>>,
    trim: Option<CanvasVideoTrim>,
}

#[cfg(target_os = "macos")]
struct CanvasVideoTrim {
    source: CanvasVideoSource,
    expected: fanta_doc::VideoNode,
    start: Entity<ui_input::InputField>,
    end: Entity<ui_input::InputField>,
    duration_us: u64,
    task: std::cell::RefCell<Option<Task<()>>>,
    cancelled: std::cell::Cell<bool>,
    error: Option<SharedString>,
}

struct PrototypeRenderCache {
    logical_size: (u32, u32),
    scale_bits: u32,
    image: std::sync::Arc<RenderImage>,
}

#[derive(Clone)]
enum PrototypeLinkNotice {
    Confirm(url::Url),
    Invalid(SharedString),
}

fn validate_prototype_link(raw: &str) -> std::result::Result<url::Url, SharedString> {
    match url::Url::parse(raw) {
        Ok(url) if matches!(url.scheme(), "http" | "https") => Ok(url),
        Ok(url) => Err(format!(
            "Blocked prototype link with unsupported {} scheme.",
            url.scheme()
        )
        .into()),
        Err(error) => Err(format!("Invalid prototype link: {error}").into()),
    }
}

fn prototype_tick_elapsed(
    last_tick: &mut Option<std::time::Instant>,
    now: std::time::Instant,
) -> std::time::Duration {
    last_tick
        .replace(now)
        .map(|previous| now.saturating_duration_since(previous))
        .unwrap_or_else(|| std::time::Duration::from_millis(16))
}

impl EventEmitter<FigViewEvent> for FigView {}

/// How a freshly opened text session seeds its selection: the text tool and
/// enter-to-edit select everything (the first keystroke replaces it), a
/// double-click selects the word under the cursor.
#[derive(Debug)]
pub(crate) enum TextEditSeed {
    SelectAll,
    WordAt(DVec2),
}

impl FigView {
    pub(crate) fn new(
        item: Entity<FigItem>,
        project: Entity<Project>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let item_subscription = Self::subscribe_to_item(&item, cx);
        let (layers_sidebar_visible, layers_sidebar_width) = {
            let settings = FantaDesignPanelSettings::get_global(cx);
            (
                settings.visible,
                clamp_sidebar_width(settings.default_width, SidebarKind::Layers),
            )
        };
        let (inspector_sidebar_visible, inspector_sidebar_width) = {
            let settings = FantaPropertiesPanelSettings::get_global(cx);
            (
                settings.visible,
                clamp_sidebar_width(settings.default_width, SidebarKind::Inspector),
            )
        };
        // The descriptor carries what `FigItem::try_open` learned about THIS
        // open (the clicked entry and the scope its path implies); the shared
        // item cannot, since one item backs every tab of the project.
        let descriptor = crate::document::take_pending_view_descriptor(cx);
        let opened_entry_id = descriptor.and_then(|descriptor| descriptor.entry_id);
        let scope = descriptor.and_then(|descriptor| descriptor.scope);
        let editor_session = cx.new(|_| EditorSession::new());
        // A variables-scoped tab starts in the variables space — the
        // `doc/variables.json` file IS the variables registry, so that's what
        // clicking it shows. A plain open without a descriptor (e.g. a second
        // window) follows the shared item's most recent scope. Later scope
        // changes arrive via `FigItemEvent::ScopeApplied`.
        let starts_in_variables = match scope {
            Some(FigScope::Variables) => true,
            Some(_) => false,
            None => {
                item.read(cx).last_scope() == Some(FigScope::Variables)
                    || item.read(cx).pending_variables_scope()
            }
        };
        if starts_in_variables {
            editor_session.update(cx, |session, cx| {
                session.set_workspace(EditorWorkspace::Variables, cx);
            });
        }
        let editor_session_subscription = cx.observe(&editor_session, |_, _, cx| cx.notify());
        let (layers_sidebar, inspector_sidebar) = Self::new_embedded_sidebars(&project, window, cx);
        let prototype_sidebar = cx.new(|cx| FantaPrototypePanel::new(item.clone(), cx));
        let motion_sidebar = cx.new(|cx| FantaMotionPanel::new(item.clone(), cx));
        let motion_sidebar_subscription =
            cx.subscribe(&motion_sidebar, |_this, _, event: &MotionPanelEvent, cx| {
                let event = *event;
                let view = cx.weak_entity();
                cx.defer(move |cx| {
                    view.update(cx, |view, cx| view.handle_motion_panel_event(event, cx))
                        .log_err();
                });
            });
        let variables_workspace =
            cx.new(|cx| FantaVariablesWorkspace::new(item.clone(), window, cx));
        let code_workspace =
            cx.new(|cx| FantaCodeWorkspace::new(item.clone(), project.clone(), window, cx));
        let timeline_shell = cx.new(|cx| {
            let mut timeline = TimelineShell::new();
            timeline.set_authoring_enabled(false, cx);
            timeline
        });
        let timeline_subscription =
            cx.subscribe(&timeline_shell, |this, _, event: &TimelineEvent, cx| {
                this.handle_timeline_event(event.clone(), cx);
            });
        let focus_handle = cx.focus_handle();
        // The agent's design tools target the most recently opened or focused
        // canvas; keep the registry pointed at this item.
        crate::agent_surface::set_active_item(item.downgrade(), cx);
        cx.on_focus(&focus_handle, window, |this: &mut Self, _, cx| {
            this.is_focused = true;
            crate::agent_surface::set_active_item(this.item.downgrade(), cx);
            // The shared document has ONE render root; re-assert this tab's
            // scope so focusing the tab brings its page/component back.
            // `request_scope` is a cheap no-op when the root is already ours.
            if let Some(scope) = this.scope {
                let view = cx.entity_id();
                this.item.update(cx, |item, cx| {
                    item.request_scope(scope, ScopeRequester::View(view), cx)
                });
            }
        })
        .detach();
        // A space held across a focus change (panel click, window switch, a
        // text session opening) delivers its key-up elsewhere; without this
        // reset `space_pan` stays true and the Select tool pans with a hand
        // cursor until space is pressed again.
        cx.on_focus_out(&focus_handle, window, |this: &mut Self, _, _, cx| {
            this.is_focused = false;
            if this.space_pan || this.pan_last_position.is_some() {
                this.space_pan = false;
                this.pan_last_position = None;
                cx.notify();
            }
        })
        .detach();
        // The root the document currently shows. For a scoped open of an
        // already-ready shared item the scope was applied in `try_open`, so
        // this starts as our own root and the echoing `ScopeApplied` (or a
        // later tab switch back to us) does not reset the viewport.
        let last_seen_root = item
            .read(cx)
            .document()
            .and_then(|document| document.doc.active_page());
        Self {
            item,
            project,
            focus_handle,
            editor_session,
            layers_sidebar,
            inspector_sidebar,
            prototype_sidebar,
            motion_sidebar,
            variables_workspace,
            code_workspace,
            timeline_shell,
            active_motion_clip: None,
            motion_keyframe_drag: None,
            layers_sidebar_visible,
            inspector_sidebar_visible,
            layers_sidebar_width,
            inspector_sidebar_width,
            selected_page_index: None,
            selected_page_root: None,
            opened_entry_id,
            scope,
            is_focused: false,
            last_seen_root,
            viewport: None,
            pan_last_position: None,
            primary_pressed: false,
            canvas_pointer_down: false,
            autosave_task: None,
            hover_resize_handle: None,
            space_pan: false,
            container_bounds: None,
            rendered_canvas: None,
            chrome_cache: std::cell::RefCell::new(None),
            #[cfg(target_os = "macos")]
            gpu_canvas: None,
            #[cfg(target_os = "macos")]
            canvas_video: None,
            #[cfg(target_os = "macos")]
            canvas_video_generation: 0,
            #[cfg(target_os = "macos")]
            canvas_video_removed: std::cell::Cell::new(false),
            #[cfg(target_os = "macos")]
            canvas_video_active: std::cell::Cell::new(true),
            tools: ToolShell::new(),
            comment_state: crate::comments_ui::CommentState::default(),
            group_faces: crate::tools::initial_group_faces(),
            #[cfg(feature = "fanta-gpui-ui")]
            gpui_toolbar: crate::gpui_adapters::runtime_enabled(cx)
                .then(|| crate::gpui_adapters::toolbar::ToolbarAdapter::new(window, cx)),
            #[cfg(feature = "fanta-gpui-ui")]
            gpui_design: (crate::gpui_adapters::runtime_enabled(cx)
                && crate::gpui_adapters::design::design_enabled())
            .then(|| crate::gpui_adapters::design::DesignAdapter::new(window, cx)),
            fonts_prewarmed: false,
            hovered_node: None,
            text_edit: None,
            pending_text_edit: None,
            prototype_player: None,
            prototype_saved_viewport: None,
            prototype_tick_task: None,
            prototype_last_tick: None,
            prototype_pointer_down: None,
            prototype_drag_fired: false,
            prototype_suppress_click: false,
            prototype_render_cache: None,
            prototype_link_notice: None,
            _item_subscription: item_subscription,
            _editor_session_subscription: editor_session_subscription,
            _motion_sidebar_subscription: motion_sidebar_subscription,
            _timeline_subscription: timeline_subscription,
        }
    }

    fn reconcile_opened_entry_with_project_root(&mut self, cx: &App) {
        if let Some(root) = self.item.read(cx).project_root() {
            self.opened_entry_id = self.opened_entry_id.filter(|entry_id| {
                let project = self.project.read(cx);
                project
                    .path_for_entry(*entry_id, cx)
                    .and_then(|path| project.absolute_path(&path, cx))
                    .is_some_and(|path| path.starts_with(root))
            });
        }
    }

    fn subscribe_to_item(item: &Entity<FigItem>, cx: &mut Context<Self>) -> Subscription {
        cx.subscribe(item, |this, _, event: &FigItemEvent, cx| {
            #[cfg(target_os = "macos")]
            if this.canvas_video.as_ref().is_some_and(|session| {
                this.selected_canvas_video_source(cx).map(|source| source.0)
                    != Some(session.source)
            }) {
                this.clear_canvas_video(cx);
            }
            // Echo document state into the DesignPanel inspector. Preview
            // frames are skipped (the panel re-echoes on the committing
            // event); selection and text-selection changes must refresh even
            // though the native panels ignore them.
            #[cfg(feature = "fanta-gpui-ui")]
            if !matches!(event, FigItemEvent::EditedTransient) {
                this.refresh_gpui_design(cx);
            }
            match event {
                FigItemEvent::Edited => {
                    this.invalidate_canvas_cache();
                    this.schedule_autosave(cx);
                    this.sync_motion_timeline(cx);
                    this.hovered_node = None;
                    // The edit may have removed the node under the inline
                    // text editor (undo, layer delete); the overlay must not
                    // outlive its target.
                    this.drop_text_edit_if_target_gone(cx);
                    cx.emit(FigViewEvent::Edited);
                    cx.notify();
                }
                // Preview frames only need a canvas repaint; emitting an item
                // event per pointer move would spam tab updates.
                FigItemEvent::EditedTransient => {
                    this.invalidate_canvas_cache();
                    cx.notify();
                }
                FigItemEvent::SelectionChanged => {
                    // The cached handle belongs to the node that was
                    // selected; a resize cursor over a now-empty selection
                    // would promise a gesture the press would not start.
                    this.hover_resize_handle = None;
                }
                FigItemEvent::TextSelectionChanged => {}
                // A save wrote the document without replacing it, so
                // nothing view-side is stale: only the tab's dirty mark
                // changes. Deliberately NOT `StateChanged`, which every
                // listener reads as a reload and answers by dropping
                // in-flight sessions.
                FigItemEvent::Saved => {
                    this.reconcile_opened_entry_with_project_root(cx);
                    cx.emit(FigViewEvent::TitleChanged);
                }
                FigItemEvent::StateChanged => {
                    this.reconcile_opened_entry_with_project_root(cx);
                    // A reload replaces the document while prototype state
                    // contains node/variable IDs from the previous tree. Drop
                    // the session locally without trying to update the item
                    // from inside its own event callback.
                    if this.prototype_player.take().is_some() {
                        this.prototype_tick_task = None;
                        this.prototype_last_tick = None;
                        this.prototype_pointer_down = None;
                        this.prototype_drag_fired = false;
                        this.prototype_suppress_click = false;
                        this.prototype_link_notice = None;
                        this.viewport = this.prototype_saved_viewport.take();
                    }
                    // The node tree may have been swapped out (disk reload)
                    // or persisted; committing the overlay's stale text into
                    // the new tree could clobber external edits, so drop the
                    // inline text editor without committing.
                    this.text_edit = None;
                    this.pending_text_edit = None;
                    // A (re)loaded document restarts its scene revision
                    // counter, so cached frames keyed by revision must go.
                    this.invalidate_canvas_cache();
                    this.motion_keyframe_drag = None;
                    this.timeline_shell
                        .update(cx, |timeline, cx| timeline.cancel_authoring_gestures(cx));
                    this.active_motion_clip = None;
                    this.sync_motion_timeline(cx);
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
                FigItemEvent::ReloadedFromDisk { merged } => {
                    let source = this.active_page_source_label(cx);
                    let message = if *merged {
                        format!(
                            "{source} changed on disk — merged into your unsaved canvas edits"
                        )
                    } else {
                        format!("{source} changed on disk — canvas updated")
                    };
                    show_canvas_notice_deferred(message, cx);
                }
                FigItemEvent::ConflictChanged => {
                    if this.item.read(cx).has_conflict() {
                        show_canvas_notice_deferred(
                            "External edits conflict with unsaved canvas edits — save or reload to resolve"
                                .to_string(),
                            cx,
                        );
                    }
                    cx.emit(FigViewEvent::TitleChanged);
                }
                FigItemEvent::SourceEditLockChanged => {
                    let source_edit_locked = this.item.read(cx).source_edit_locked();
                    let view = cx.weak_entity();
                    cx.defer(move |cx| {
                        view.update(cx, |view, cx| {
                            if source_edit_locked {
                                view.cancel_canvas_edits_for_source_lock(cx);
                            }
                            view.sync_motion_timeline(cx);
                        })
                        .log_err();
                    });
                    cx.emit(FigViewEvent::Edited);
                }
                FigItemEvent::ScopeApplied(scope, requester) => {
                    // A scoped open, a tab re-asserting its scope on focus, or
                    // the load-time apply re-targeted the shared document. The
                    // refit decision keys on WHO asked — not on `is_focused`,
                    // which is stale between the emit and the next draw (GPUI
                    // fires focus listeners only when a frame is drawn), so
                    // the previous tab could still believe it is focused and
                    // have its saved viewport and page mirrors clobbered by a
                    // sibling's navigation. Only the requesting view follows,
                    // and only onto a root it is not already showing (a tab
                    // switch merely restoring our root must keep the saved
                    // viewport); a scoped open targets a tab that does not
                    // exist yet, so no live view follows; the load-time apply
                    // has no requesting view, so the focused view follows.
                    // Known limitation: the shared document has ONE render
                    // root, so two SPLITS visible at once both render the
                    // focused tab's root.
                    let follows = match requester {
                        ScopeRequester::View(view) => *view == cx.entity_id(),
                        ScopeRequester::Open => false,
                        ScopeRequester::Load => this.is_focused,
                    };
                    let root = this
                        .item
                        .read(cx)
                        .document()
                        .and_then(|document| document.doc.active_page());
                    if follows && this.last_seen_root != root {
                        this.last_seen_root = root;
                        match scope {
                            FigScope::Variables => {
                                this.set_editor_workspace(EditorWorkspace::Variables, cx);
                            }
                            FigScope::Page(_) | FigScope::Component(_) => {
                                if this.editor_workspace(cx) == EditorWorkspace::Variables {
                                    this.set_editor_workspace(EditorWorkspace::Canvas, cx);
                                }
                                let document = this.item.read(cx).document();
                                let selected_page_index = root.and_then(|root| {
                                    document.and_then(|document| {
                                        document
                                            .pages
                                            .iter()
                                            .position(|page| page.root == Some(root))
                                    })
                                });
                                this.selected_page_root = root;
                                this.selected_page_index = selected_page_index;
                                this.viewport = None;
                                this.hovered_node = None;
                                this.invalidate_canvas_cache();
                            }
                        }
                        cx.emit(FigViewEvent::TitleChanged);
                        cx.notify();
                    }
                }
            }
            cx.notify();
        })
    }

    /// Name the file an external reload changed: the active page's source,
    /// relative to the project root. Falls back to the generic "Design" when
    /// the document is not a project on disk, has no page scope, or has no
    /// materialised source for that page yet.
    fn active_page_source_label(&self, cx: &App) -> String {
        let item = self.item.read(cx);
        let fallback = "Design".to_string();
        let Some(root) = item.project_root() else {
            return fallback;
        };
        let Some(page) = item
            .document()
            .and_then(|document| document.doc.active_page())
        else {
            return fallback;
        };
        let Some(source) = fanta_format::locate_page_source(root, page) else {
            return fallback;
        };
        source
            .strip_prefix(root)
            .unwrap_or(&source)
            .to_string_lossy()
            .into_owned()
    }

    fn cancel_canvas_edits_for_source_lock(&mut self, cx: &mut Context<Self>) {
        if !self.item.read(cx).source_edit_locked() {
            return;
        }

        self.cancel_motion_keyframe_drag(cx);
        self.timeline_shell
            .update(cx, |timeline, cx| timeline.set_authoring_enabled(false, cx));
        self.primary_pressed = false;
        self.pending_text_edit = None;
        let text_session = self.text_edit.take().map(|edit| edit.session);
        let mut viewport = self.viewport;
        let screen_size = self.container_bounds.map(|bounds| {
            let (width, height) = bounds_size(bounds);
            DVec2::new(width, height)
        });
        let tools = &mut self.tools;
        self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let revision_before = document.doc.scene.revision();
                if let Some(session) = text_session.as_ref() {
                    crate::text_edit::rewind_preview(&mut document.doc, session);
                }
                if let (Some(viewport), Some(screen_size)) = (viewport.as_mut(), screen_size) {
                    let mut tool_context =
                        tool_context(&mut document.doc, viewport, screen_size, ToolKind::Select);
                    tools.cancel_and_activate(ToolKind::Select, &mut tool_context);
                } else {
                    tools.activate_without_context(ToolKind::Select);
                }
                let change = if document.doc.scene.revision() != revision_before {
                    DocChange::ContentPreview
                } else {
                    DocChange::None
                };
                ((), change)
            });
            item.finish_content_preview(false, cx);
        });
        self.viewport = viewport;
        self.remember_tool_face(ToolKind::Select);
        self.invalidate_canvas_cache();
        cx.notify();
    }

    fn new_embedded_sidebars(
        project: &Entity<Project>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Entity<FantaDesignPanel>, Entity<FantaPropertiesPanel>) {
        let fs = project.read(cx).fs().clone();
        let view = cx.entity();
        (
            FantaDesignPanel::new_embedded(view.clone(), fs.clone(), window, cx),
            FantaPropertiesPanel::new_embedded(view, fs, window, cx),
        )
    }

    pub fn item(&self) -> &Entity<FigItem> {
        &self.item
    }

    pub fn editor_session(&self) -> &Entity<EditorSession> {
        &self.editor_session
    }

    pub fn editor_mode(&self, cx: &App) -> EditorMode {
        self.editor_session.read(cx).mode()
    }

    pub fn editor_workspace(&self, cx: &App) -> EditorWorkspace {
        self.editor_session.read(cx).workspace()
    }

    fn finish_panel_edits(&mut self, cx: &mut Context<Self>) {
        self.inspector_sidebar
            .update(cx, |panel, cx| panel.finish_continuous_edits(cx));
        self.variables_workspace
            .update(cx, |workspace, cx| workspace.finish_value_edit(cx));
        self.prototype_sidebar
            .update(cx, |panel, cx| panel.finish_parameter_edit(cx));
        #[cfg(feature = "fanta-gpui-ui")]
        self.finish_gpui_design_edits(cx);
    }

    fn finish_document_edits(&mut self, cx: &mut Context<Self>) {
        self.finish_panel_edits(cx);
        self.commit_text_edit(cx);
        let clip_edit = self
            .timeline_shell
            .update(cx, |timeline, cx| timeline.finish_clip_edit(cx));
        match clip_edit {
            Some(TimelineEvent::RenameClip(name)) => self.rename_motion_clip(&name, cx),
            Some(TimelineEvent::SetClipDuration(duration_us)) => {
                self.set_motion_clip_duration(duration_us, cx)
            }
            _ => {}
        }
        let easing_edit = self
            .timeline_shell
            .update(cx, |timeline, cx| timeline.finish_easing_edit(cx));
        match easing_edit {
            Some(TimelineEvent::EditKeyframeEasing {
                keyframe,
                easing,
                phase: TimelineEditPhase::Commit,
            }) => {
                if self
                    .motion_keyframe_drag
                    .as_ref()
                    .is_some_and(|session| session.matches(&keyframe))
                {
                    self.commit_motion_keyframe_easing(&keyframe, easing, cx);
                } else {
                    self.set_motion_keyframe_easing(keyframe, easing, cx);
                }
            }
            Some(TimelineEvent::EditKeyframeEasing {
                phase: TimelineEditPhase::Cancel,
                ..
            }) => self.cancel_motion_keyframe_drag(cx),
            _ => self.finish_motion_keyframe_drag(cx),
        }
        self.timeline_shell
            .update(cx, |timeline, cx| timeline.reset_keyframe_drag(cx));
        self.sync_motion_timeline(cx);
    }

    pub(crate) fn finish_document_edits_for_external_change(&mut self, cx: &mut Context<Self>) {
        self.finish_document_edits(cx);
    }

    pub fn set_editor_workspace(&mut self, workspace: EditorWorkspace, cx: &mut Context<Self>) {
        if self.editor_workspace(cx) == workspace {
            return;
        }
        if self.prototype_player.is_some() && workspace != EditorWorkspace::Canvas {
            self.exit_prototype_session(cx);
        }
        self.finish_document_edits(cx);
        if workspace != EditorWorkspace::Canvas {
            self.timeline_shell
                .update(cx, |timeline, cx| timeline.pause(cx));
        }
        self.editor_session
            .update(cx, |session, cx| session.set_workspace(workspace, cx));
        self.sync_motion_timeline(cx);
        self.invalidate_canvas_cache();
        cx.notify();
    }

    pub fn set_editor_mode(&mut self, mode: EditorMode, cx: &mut Context<Self>) {
        if self.editor_mode(cx) == mode {
            return;
        }
        if self.prototype_player.is_some() && mode != EditorMode::Prototype {
            self.exit_prototype_session(cx);
        }
        self.finish_document_edits(cx);
        if mode != EditorMode::Motion {
            self.timeline_shell
                .update(cx, |timeline, cx| timeline.pause(cx));
        }
        if mode != EditorMode::Design {
            self.activate_tool(ToolKind::Select, cx);
        }
        self.editor_session
            .update(cx, |session, cx| session.set_mode(mode, cx));
        self.sync_motion_timeline(cx);
        self.invalidate_canvas_cache();
        cx.notify();
    }

    pub(crate) fn motion_evaluation(
        &self,
        document: &FigDocument,
        cx: &App,
    ) -> Option<MotionEvaluation> {
        if self.editor_mode(cx) != EditorMode::Motion {
            return None;
        }
        let clip = self
            .active_motion_clip
            .or_else(|| document.doc.motion.clips.keys().next().copied())?;
        let playhead_ms = self
            .timeline_shell
            .read(cx)
            .playhead_us()
            .max(0)
            .div_euclid(1_000)
            .min(i64::from(u32::MAX)) as u32;
        document.doc.motion.evaluate(clip, playhead_ms)
    }

    fn handle_timeline_event(&mut self, event: TimelineEvent, cx: &mut Context<Self>) {
        match event {
            TimelineEvent::PlayheadChanged(_) | TimelineEvent::PlaybackChanged(_) => {
                self.invalidate_canvas_cache();
            }
            TimelineEvent::CreateClip => {
                let view = cx.weak_entity();
                cx.defer(move |cx| {
                    view.update(cx, |view, cx| view.create_motion_clip(cx))
                        .log_err();
                });
            }
            TimelineEvent::AddKeyframe(property) => {
                let view = cx.weak_entity();
                cx.defer(move |cx| {
                    view.update(cx, |view, cx| view.add_motion_keyframe(property, cx))
                        .log_err();
                });
            }
            TimelineEvent::KeyframeSelectionChanged(_) => {}
            TimelineEvent::EditKeyframeTime {
                keyframe,
                time_us,
                phase,
            } => {
                let view = cx.weak_entity();
                cx.defer(move |cx| {
                    view.update(cx, |view, cx| {
                        view.edit_motion_keyframe_time(keyframe, time_us, phase, cx)
                    })
                    .log_err();
                });
            }
            TimelineEvent::SetKeyframeInterpolation {
                keyframe,
                interpolation,
            } => {
                let view = cx.weak_entity();
                cx.defer(move |cx| {
                    view.update(cx, |view, cx| {
                        view.set_motion_keyframe_interpolation(keyframe, interpolation, cx)
                    })
                    .log_err();
                });
            }
            TimelineEvent::SetKeyframeEasing { keyframe, easing } => {
                let view = cx.weak_entity();
                cx.defer(move |cx| {
                    view.update(cx, |view, cx| {
                        view.set_motion_keyframe_easing(keyframe, easing, cx)
                    })
                    .log_err();
                });
            }
            TimelineEvent::EditKeyframeEasing {
                keyframe,
                easing,
                phase,
            } => {
                let view = cx.weak_entity();
                cx.defer(move |cx| {
                    view.update(cx, |view, cx| {
                        view.edit_motion_keyframe_easing(keyframe, easing, phase, cx)
                    })
                    .log_err();
                });
            }
            TimelineEvent::DeleteKeyframe(keyframe) => {
                let view = cx.weak_entity();
                cx.defer(move |cx| {
                    view.update(cx, |view, cx| view.delete_motion_keyframe(keyframe, cx))
                        .log_err();
                });
            }
            TimelineEvent::RenameClip(name) => {
                let view = cx.weak_entity();
                cx.defer(move |cx| {
                    view.update(cx, |view, cx| view.rename_motion_clip(&name, cx))
                        .log_err();
                });
            }
            TimelineEvent::SetClipDuration(duration_us) => {
                let view = cx.weak_entity();
                cx.defer(move |cx| {
                    view.update(cx, |view, cx| {
                        view.set_motion_clip_duration(duration_us, cx)
                    })
                    .log_err();
                });
            }
        }
        cx.notify();
    }

    fn handle_motion_panel_event(&mut self, event: MotionPanelEvent, cx: &mut Context<Self>) {
        match event {
            MotionPanelEvent::SelectClip(clip) => {
                let clip_exists = self
                    .item
                    .read(cx)
                    .document()
                    .is_some_and(|document| document.doc.motion.clip(clip).is_some());
                if !clip_exists || self.active_motion_clip == Some(clip) {
                    return;
                }
                self.finish_document_edits(cx);
                self.active_motion_clip = Some(clip);
                self.sync_motion_timeline(cx);
                self.invalidate_canvas_cache();
                cx.notify();
            }
        }
    }

    fn edit_motion_keyframe_time(
        &mut self,
        keyframe: TimelineKeyframeSelection,
        time_us: i64,
        phase: TimelineEditPhase,
        cx: &mut Context<Self>,
    ) {
        match phase {
            TimelineEditPhase::Begin => self.begin_motion_keyframe_drag(&keyframe, cx),
            TimelineEditPhase::Preview => self.preview_motion_keyframe_drag(&keyframe, time_us, cx),
            TimelineEditPhase::Commit => self.commit_motion_keyframe_drag(&keyframe, time_us, cx),
            TimelineEditPhase::Cancel => self.cancel_motion_keyframe_drag(cx),
        }
    }

    fn set_motion_keyframe_interpolation(
        &mut self,
        keyframe: TimelineKeyframeSelection,
        interpolation: Interpolation,
        cx: &mut Context<Self>,
    ) {
        self.finish_motion_keyframe_drag(cx);
        if !self.is_editable(cx) {
            return;
        }
        let Some(clip_id) = self.active_motion_clip else {
            return;
        };
        let operation = self.item.read(cx).document().and_then(|document| {
            set_keyframe_interpolation_operation(&document.doc, clip_id, &keyframe, interpolation)
        });
        self.apply_motion_operation(operation, "setting keyframe interpolation", cx);
    }

    fn set_motion_keyframe_easing(
        &mut self,
        keyframe: TimelineKeyframeSelection,
        easing: Easing,
        cx: &mut Context<Self>,
    ) {
        self.finish_motion_keyframe_drag(cx);
        if !self.is_editable(cx) {
            return;
        }
        let Some(clip_id) = self.active_motion_clip else {
            return;
        };
        let operation = self.item.read(cx).document().and_then(|document| {
            set_keyframe_easing_operation(&document.doc, clip_id, &keyframe, easing)
        });
        self.apply_motion_operation(operation, "setting keyframe easing", cx);
    }

    fn edit_motion_keyframe_easing(
        &mut self,
        keyframe: TimelineKeyframeSelection,
        easing: Easing,
        phase: TimelineEditPhase,
        cx: &mut Context<Self>,
    ) {
        match phase {
            TimelineEditPhase::Begin => self.begin_motion_keyframe_drag(&keyframe, cx),
            TimelineEditPhase::Preview => {
                self.preview_motion_keyframe_easing(&keyframe, easing, cx)
            }
            TimelineEditPhase::Commit => self.commit_motion_keyframe_easing(&keyframe, easing, cx),
            TimelineEditPhase::Cancel => self.cancel_motion_keyframe_drag(cx),
        }
    }

    fn begin_motion_keyframe_drag(
        &mut self,
        keyframe: &TimelineKeyframeSelection,
        cx: &mut Context<Self>,
    ) {
        self.finish_panel_edits(cx);
        self.commit_text_edit(cx);
        self.cancel_motion_keyframe_drag(cx);
        if !self.is_editable(cx) {
            return;
        }
        let Some(clip_id) = self.active_motion_clip else {
            return;
        };
        self.motion_keyframe_drag = self.item.read(cx).document().and_then(|document| {
            MotionKeyframeDragSession::begin(&document.doc, clip_id, keyframe)
        });
    }

    fn preview_motion_keyframe_drag(
        &mut self,
        keyframe: &TimelineKeyframeSelection,
        time_us: i64,
        cx: &mut Context<Self>,
    ) {
        if !self.is_editable(cx) {
            self.cancel_motion_keyframe_drag(cx);
            return;
        }
        let item = self.item.clone();
        let Some(session) = self
            .motion_keyframe_drag
            .as_mut()
            .filter(|session| session.matches(keyframe))
        else {
            return;
        };
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let change = if session.preview(&mut document.doc, time_us) {
                    DocChange::ContentPreview
                } else {
                    DocChange::None
                };
                ((), change)
            });
        });
    }

    fn preview_motion_keyframe_easing(
        &mut self,
        keyframe: &TimelineKeyframeSelection,
        easing: Easing,
        cx: &mut Context<Self>,
    ) {
        if !self.is_editable(cx) {
            self.cancel_motion_keyframe_drag(cx);
            return;
        }
        let item = self.item.clone();
        let Some(session) = self
            .motion_keyframe_drag
            .as_mut()
            .filter(|session| session.matches(keyframe))
        else {
            return;
        };
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let change = if session.preview_easing(&mut document.doc, easing) {
                    DocChange::ContentPreview
                } else {
                    DocChange::None
                };
                ((), change)
            });
        });
    }

    fn commit_motion_keyframe_drag(
        &mut self,
        keyframe: &TimelineKeyframeSelection,
        time_us: i64,
        cx: &mut Context<Self>,
    ) {
        let Some(mut session) = self.motion_keyframe_drag.take() else {
            return;
        };
        if !session.matches(keyframe) || !self.is_editable(cx) {
            self.restore_motion_keyframe_drag(session, cx);
            return;
        }
        self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                session.preview(&mut document.doc, time_us);
                let change = if session.restore(&mut document.doc) {
                    DocChange::ContentPreview
                } else {
                    DocChange::None
                };
                ((), change)
            });
            let Some(operation) = session.operation() else {
                item.finish_content_preview(false, cx);
                return;
            };
            if let Err(error) = item.apply(operation, cx) {
                item.finish_content_preview(false, cx);
                log::error!("moving motion keyframe failed: {error:#}");
            }
        });
    }

    fn commit_motion_keyframe_easing(
        &mut self,
        keyframe: &TimelineKeyframeSelection,
        easing: Easing,
        cx: &mut Context<Self>,
    ) {
        let Some(mut session) = self.motion_keyframe_drag.take() else {
            return;
        };
        if !session.matches(keyframe) || !self.is_editable(cx) {
            self.restore_motion_keyframe_drag(session, cx);
            return;
        }
        self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                session.preview_easing(&mut document.doc, easing);
                let change = if session.restore(&mut document.doc) {
                    DocChange::ContentPreview
                } else {
                    DocChange::None
                };
                ((), change)
            });
            let Some(operation) = session.operation() else {
                item.finish_content_preview(false, cx);
                return;
            };
            if let Err(error) = item.apply(operation, cx) {
                item.finish_content_preview(false, cx);
                log::error!("editing motion keyframe easing failed: {error:#}");
            }
        });
    }

    fn finish_motion_keyframe_drag(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.motion_keyframe_drag.take() else {
            return;
        };
        if !self.is_editable(cx) {
            self.restore_motion_keyframe_drag(session, cx);
            return;
        }
        self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let change = if session.restore(&mut document.doc) {
                    DocChange::ContentPreview
                } else {
                    DocChange::None
                };
                ((), change)
            });
            let Some(operation) = session.operation() else {
                item.finish_content_preview(false, cx);
                return;
            };
            if let Err(error) = item.apply(operation, cx) {
                item.finish_content_preview(false, cx);
                log::error!("finishing motion keyframe drag failed: {error:#}");
            }
        });
    }

    fn cancel_motion_keyframe_drag(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.motion_keyframe_drag.take() else {
            return;
        };
        self.restore_motion_keyframe_drag(session, cx);
    }

    fn restore_motion_keyframe_drag(
        &self,
        session: MotionKeyframeDragSession,
        cx: &mut Context<Self>,
    ) {
        self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let change = if session.restore(&mut document.doc) {
                    DocChange::ContentPreview
                } else {
                    DocChange::None
                };
                ((), change)
            });
            item.finish_content_preview(false, cx);
        });
    }

    fn apply_motion_operation(
        &self,
        operation: Option<Operation>,
        action: &'static str,
        cx: &mut Context<Self>,
    ) {
        let Some(operation) = operation else {
            return;
        };
        self.item.update(cx, |item, cx| {
            if let Err(error) = item.apply(operation, cx) {
                log::error!("{action} failed: {error:#}");
            }
        });
    }

    fn delete_motion_keyframe(
        &mut self,
        keyframe: TimelineKeyframeSelection,
        cx: &mut Context<Self>,
    ) {
        self.finish_motion_keyframe_drag(cx);
        if !self.is_editable(cx) {
            return;
        }
        let Some(clip_id) = self.active_motion_clip else {
            return;
        };
        let operation = self
            .item
            .read(cx)
            .document()
            .and_then(|document| delete_keyframe_operation(&document.doc, clip_id, &keyframe));
        self.apply_motion_operation(operation, "deleting motion keyframe", cx);
    }

    fn rename_motion_clip(&mut self, name: &str, cx: &mut Context<Self>) {
        self.finish_motion_keyframe_drag(cx);
        if !self.is_editable(cx) {
            return;
        }
        let Some(clip_id) = self.active_motion_clip else {
            return;
        };
        let operation = self
            .item
            .read(cx)
            .document()
            .and_then(|document| rename_clip_operation(&document.doc, clip_id, name));
        self.apply_motion_operation(operation, "renaming motion clip", cx);
    }

    fn set_motion_clip_duration(&mut self, duration_us: i64, cx: &mut Context<Self>) {
        self.finish_motion_keyframe_drag(cx);
        if !self.is_editable(cx) {
            return;
        }
        let Some(clip_id) = self.active_motion_clip else {
            return;
        };
        let operation =
            self.item.read(cx).document().and_then(|document| {
                set_clip_duration_operation(&document.doc, clip_id, duration_us)
            });
        self.apply_motion_operation(operation, "changing motion clip duration", cx);
    }

    fn create_motion_clip(&mut self, cx: &mut Context<Self>) {
        if !self.is_editable(cx) {
            return;
        }
        let clip_id = AnimationClipId::new();
        self.active_motion_clip = Some(clip_id);
        self.item.update(cx, |item, cx| {
            if let Err(error) = item.apply(
                Operation::CreateAnimationClip {
                    clip: Box::new(AnimationClip::new(clip_id, "Animation 1", 5_000)),
                },
                cx,
            ) {
                log::error!("creating motion clip failed: {error:#}");
            }
        });
        self.sync_motion_timeline(cx);
    }

    fn add_motion_keyframe(&mut self, property: TimelineProperty, cx: &mut Context<Self>) {
        if !self.is_editable(cx) {
            return;
        }
        let Some(clip_id) = self.active_motion_clip else {
            return;
        };
        let playhead_ms = self
            .timeline_shell
            .read(cx)
            .playhead_us()
            .max(0)
            .div_euclid(1_000)
            .min(i64::from(u32::MAX)) as u32;
        self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let Some(node_id) = single_selection(&document.doc) else {
                    return ((), DocChange::None);
                };
                let Some(node) = motion_source_node(&document.doc, node_id) else {
                    return ((), DocChange::None);
                };
                let motion_property = motion_property(property);
                let Some(value) = motion_value(&node, motion_property) else {
                    return ((), DocChange::None);
                };
                let target = MotionTarget::new(node_id, motion_property);
                let Some(clip) = document.doc.motion.clip(clip_id) else {
                    return ((), DocChange::None);
                };
                let time_ms = playhead_ms.min(clip.duration_ms);
                let mut transaction = Transaction::new("Add Keyframe");
                let track_id = if let Some(track) = clip.track_for_target(target) {
                    track.id
                } else {
                    let track_id = AnimationTrackId::new();
                    transaction.push(Operation::SetAnimationTrack {
                        clip: clip_id,
                        track: track_id,
                        old: None,
                        new: Some(Box::new(AnimationTrack::new(track_id, target))),
                    });
                    track_id
                };
                let existing = clip
                    .tracks
                    .get(&track_id)
                    .and_then(|track| {
                        track
                            .keyframes
                            .values()
                            .find(|keyframe| keyframe.time_ms == time_ms)
                    })
                    .cloned();
                let keyframe_id = existing
                    .as_ref()
                    .map(|keyframe| keyframe.id)
                    .unwrap_or_else(KeyframeId::new);
                transaction.push(Operation::SetKeyframe {
                    clip: clip_id,
                    track: track_id,
                    target,
                    keyframe: keyframe_id,
                    old: existing,
                    new: Some(Keyframe {
                        id: keyframe_id,
                        time_ms,
                        value,
                        interpolation: Interpolation::Linear,
                        easing: Easing::EaseInOut,
                    }),
                });
                match document.doc.apply_transaction(transaction) {
                    Ok(()) => ((), DocChange::Content),
                    Err(error) => {
                        log::error!("adding motion keyframe failed: {error:#}");
                        ((), DocChange::None)
                    }
                }
            });
        });
        self.sync_motion_timeline(cx);
    }

    fn sync_motion_timeline(&mut self, cx: &mut Context<Self>) {
        let authoring_enabled = self.is_editable(cx)
            && self.editor_workspace(cx) == EditorWorkspace::Canvas
            && self.editor_mode(cx) == EditorMode::Motion;
        let model = if let Some(document) = self.item.read(cx).document() {
            if self
                .active_motion_clip
                .is_none_or(|clip| document.doc.motion.clip(clip).is_none())
            {
                self.active_motion_clip = document.doc.motion.clips.keys().next().copied();
            }
            motion_timeline_model(&document.doc, self.active_motion_clip)
        } else {
            self.active_motion_clip = None;
            TimelineViewModel::empty()
        };
        self.timeline_shell.update(cx, |timeline, cx| {
            if timeline.authoring_enabled() != authoring_enabled {
                timeline.set_authoring_enabled(authoring_enabled, cx);
            }
            timeline.set_model(model, cx);
        });
        let active_clip = self.active_motion_clip;
        let motion_sidebar = self.motion_sidebar.downgrade();
        cx.defer(move |cx| {
            motion_sidebar
                .update(cx, |panel, cx| panel.set_active_clip(active_clip, cx))
                .log_err();
        });
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

    pub(crate) fn invalidate_canvas_cache(&mut self) {
        self.rendered_canvas = None;
        self.prototype_render_cache = None;
        self.chrome_cache.replace(None);
        #[cfg(target_os = "macos")]
        if let Some(gpu) = self.gpu_canvas.as_mut() {
            gpu.invalidate();
        }
    }

    pub(crate) fn is_editable(&self, cx: &App) -> bool {
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
        if self.handle_motion_selection_event(event, viewport, cx) {
            return;
        }
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

        let active_tool = self.tools.kind();
        let tools = &mut self.tools;
        let mut wants_exit = false;
        let mut content_changed = false;
        let item = self.item.clone();
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let revision_before = document.doc.scene.revision();
                let selection_before: Vec<NodeId> =
                    document.doc.selection.iter().copied().collect();

                let mut ctx =
                    tool_context(&mut document.doc, &mut viewport, screen_size, active_tool);
                let response = tools.handle_event(&mut ctx, event);
                wants_exit = response.wants_exit;

                let revision_changed = document.doc.scene.revision() != revision_before;
                content_changed = revision_changed;
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
        if content_changed {
            self.invalidate_canvas_cache();
        }
        if wants_exit {
            let was_text_tool = self.tools.kind() == ToolKind::Text;
            let text_node = was_text_tool
                .then(|| self.selection_anchor_text_node(cx))
                .flatten();
            self.activate_tool(ToolKind::Select, cx);
            // The text tool commits its node and selects it before asking to
            // exit; drop the user straight into typing on it, like Figma.
            if let Some(node) = text_node {
                self.pending_text_edit = Some(node);
                cx.notify();
            }
            return;
        }
        // Document and selection changes already notify through the item's
        // event stream; only repaint here when something view-local changed.
        // An unconditional notify would re-render the whole view (and wake
        // observers) on every idle mouse move.
        if content_changed
            || !crate::canvas::same_viewport(viewport_before, viewport)
            || self.tools.overlays != overlays_before
            || self.tools.cursor != cursor_before
        {
            cx.notify();
        }
    }

    fn handle_motion_selection_event(
        &mut self,
        event: ToolEvent,
        viewport: Viewport,
        cx: &mut Context<Self>,
    ) -> bool {
        let ToolEvent::Pointer(fanta_tools::PointerEvent::Press {
            screen,
            button: ToolButton::Primary,
            modifiers,
            ..
        }) = event
        else {
            return false;
        };
        if self.editor_mode(cx) != EditorMode::Motion || self.tools.kind() != ToolKind::Select {
            return false;
        }
        let Some(bounds) = self.container_bounds else {
            return false;
        };
        let hit = {
            let item = self.item.read(cx);
            let Some(document) = item.document() else {
                return false;
            };
            let Some(evaluation) = self.motion_evaluation(document, cx) else {
                return false;
            };
            let (width, height) = bounds_size(bounds);
            evaluated_hit_test_screen(
                &document.doc.scene,
                &evaluation,
                &viewport,
                DVec2::new(width, height),
                DVec2::new(screen[0], screen[1]),
                HitPrecision::Path,
                document.doc.active_page(),
            )
        };
        self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                match hit {
                    Some(node) if modifiers.extend_selection() => {
                        document.doc.selection.toggle(node)
                    }
                    Some(node) => document.doc.selection.select_only(node),
                    None if !modifiers.extend_selection() => document.doc.selection.clear(),
                    None => {}
                }
                ((), DocChange::Selection)
            });
        });
        true
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
        if kind != ToolKind::Comment {
            self.comment_state.draft = None;
            self.comment_state.hovered_pin = None;
        }
        // Switching tools is a document edit boundary for every inspector and
        // inline session, not only text.
        self.finish_document_edits(cx);
        let Some((bounds, viewport)) = self.container_bounds.zip(self.viewport) else {
            self.tools.activate_without_context(kind);
            self.remember_tool_face(kind);
            cx.notify();
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
                let mut ctx = tool_context(&mut document.doc, &mut viewport, screen_size, kind);
                tools.activate(kind, &mut ctx);
                let change = if document.doc.scene.revision() != revision_before {
                    DocChange::Content
                } else {
                    DocChange::None
                };
                ((), change)
            });
        });
        self.remember_tool_face(kind);
        self.viewport = Some(viewport);
        cx.notify();
    }

    fn remember_tool_face(&mut self, kind: ToolKind) {
        if let Some(index) = crate::tools::group_index_of(kind)
            && let Some(face) = self.group_faces.get_mut(index)
        {
            *face = kind;
        }
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
        if event.button == MouseButton::Left {
            // Set before any of the branches below can return early: the
            // autosave stands down for the whole gesture, including the text
            // and comment paths that never reach a tool.
            self.canvas_pointer_down = true;
        }
        if self.prototype_player.is_some() {
            self.focus_handle.focus(window, cx);
            if event.button == MouseButton::Left {
                self.prototype_pointer_down = Some(event.position);
                self.prototype_drag_fired = false;
                let response = self.trigger_prototype_pointer(
                    event.position,
                    fanta_present::PointerEvent::Down,
                    cx,
                );
                self.prototype_suppress_click = response.suppress_click;
            }
            return;
        }
        if event.button == MouseButton::Left {
            self.finish_panel_edits(cx);
        }
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
            && event.click_count >= 2
            && event.click_count.is_multiple_of(2)
            && self.editor_mode(cx) == EditorMode::Design
            && self.tools.kind() == ToolKind::Select
            && self.is_editable(cx)
            && let Some(bounds) = self.container_bounds
        {
            let screen = screen_position_in_bounds(event.position, bounds);
            if let Some(node) = self.text_node_at(screen, cx) {
                // A wrapped text layer first has to be drilled into by the
                // select tool. Only a text layer that is already the sole
                // selection enters editing on this press. Standalone text is
                // selected by the first press in the double-click pair, so it
                // still opens on an ordinary double-click.
                if self.single_selected_text_node(cx) == Some(node) {
                    self.open_text_edit(node, TextEditSeed::WordAt(screen), window, cx);
                    return;
                }
            } else if let Some(target) = self.instance_text_at(screen, cx) {
                // Instance text is virtual and cannot become its own scene
                // selection; selecting the wrapping instance is the equivalent
                // prerequisite before opening an override editor.
                let instance_selected = self.item.read(cx).document().is_some_and(|document| {
                    document.doc.selection.as_slice() == [target.instance_id]
                });
                if instance_selected {
                    self.open_instance_text_edit(target, TextEditSeed::WordAt(screen), window, cx);
                    return;
                }
            }
        }

        self.focus_handle.focus(window, cx);
        let Some(bounds) = self.container_bounds else {
            return;
        };

        // Comments: pin clicks open threads with any tool; in comment mode a
        // canvas click opens the draft composer (nothing hits the doc until
        // Send) — the original fanta flow.
        if event.button == MouseButton::Left
            && self.handle_comment_mouse_down(event.position, window, cx)
        {
            return;
        }

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
        if self.prototype_player.is_some() {
            self.trigger_prototype_key(&event.keystroke.key, cx);
            return;
        }
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
        if event.button == MouseButton::Left {
            self.canvas_pointer_down = false;
        }
        if self.prototype_player.is_some() {
            if event.button == MouseButton::Left {
                let should_click =
                    self.prototype_pointer_down.take().is_some() && !self.prototype_drag_fired;
                let suppress_click = std::mem::take(&mut self.prototype_suppress_click);
                self.prototype_drag_fired = false;
                self.trigger_prototype_pointer(event.position, fanta_present::PointerEvent::Up, cx);
                if should_click && !suppress_click {
                    self.trigger_prototype_click(event.position, cx);
                }
            }
            return;
        }
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
        // The canvas and window listeners share an idempotent release path:
        // whichever runs first ends the gesture, so it never dispatches twice.
        self.handle_window_mouse_up(event, cx);
    }

    /// Element listeners stop firing once the cursor leaves the canvas, so
    /// the window listener must also be able to end an active gesture.
    pub(crate) fn handle_window_mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        if event.button != MouseButton::Left {
            return;
        }
        // Cleared before the `primary_pressed` guard: a release that lands
        // outside the canvas comes through here only, and leaving the flag set
        // would hold the autosave off indefinitely.
        self.canvas_pointer_down = false;
        if !self.primary_pressed {
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
        // A hover with no button held also recovers a release missed while
        // the window was inactive.
        if event.pressed_button.is_none() {
            self.canvas_pointer_down = false;
        }
        if self.prototype_player.is_some() {
            if let Some(origin) = self.prototype_pointer_down {
                let delta = event.position - origin;
                let distance = f64::from(f32::from(delta.x)).hypot(f64::from(f32::from(delta.y)));
                if !self.prototype_drag_fired && distance >= 4.0 {
                    self.prototype_drag_fired = true;
                    self.trigger_prototype_pointer(
                        event.position,
                        fanta_present::PointerEvent::DragStart,
                        cx,
                    );
                }
            } else {
                self.trigger_prototype_pointer(
                    event.position,
                    fanta_present::PointerEvent::Move,
                    cx,
                );
            }
            return;
        }
        self.handle_comment_mouse_move(event.position, cx);
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
        if self.tools.kind() == ToolKind::Scale {
            self.update_hover_resize_handle(screen, cx);
            if self.hovered_node.take().is_some() {
                cx.notify();
            }
            return;
        }
        if self.tools.kind() != ToolKind::Select {
            let had_handle = self.hover_resize_handle.take().is_some();
            if self.hovered_node.take().is_some() || had_handle {
                cx.notify();
            }
            return;
        }
        self.update_hover_resize_handle(screen, cx);
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
                if let Some(evaluation) = self.motion_evaluation(document, cx) {
                    evaluated_hit_test_screen(
                        &document.doc.scene,
                        &evaluation,
                        &viewport,
                        DVec2::new(width, height),
                        screen,
                        HitPrecision::Bounds,
                        document.doc.active_page(),
                    )
                } else {
                    fanta_canvas::hit_test_screen(
                        &document.doc.scene,
                        &viewport,
                        DVec2::new(width, height),
                        screen,
                        HitPrecision::Bounds,
                        document.doc.active_page(),
                    )
                }
            })
        };
        if hovered != self.hovered_node {
            self.hovered_node = hovered;
            cx.notify();
        }
    }

    /// Resolve which selection handle (if any) the cursor is over, so
    /// `render` can show a directional resize cursor by reading one field.
    /// Mirrors the Select tool's own precondition — a single selected node,
    /// the same handle threshold — so the cursor never promises a resize the
    /// press would not start. Rotated nodes are skipped: their handles sit on
    /// the oriented box, which this AABB hit-test would misreport.
    fn update_hover_resize_handle(&mut self, screen: DVec2, cx: &mut Context<Self>) {
        let handle = self.resize_handle_at(screen, cx);
        if handle != self.hover_resize_handle {
            self.hover_resize_handle = handle;
            cx.notify();
        }
    }

    fn resize_handle_at(&self, screen: DVec2, cx: &App) -> Option<fanta_canvas::ResizeHandle> {
        let viewport = self.viewport?;
        let bounds = self.container_bounds?;
        let (width, height) = bounds_size(bounds);
        let document = self.item.read(cx).document()?;
        if self.tools.kind() == ToolKind::Scale {
            let (local, world) =
                fanta_tools::ScaleTool::selection_frame(&document.doc, document.doc.active_page())?;
            return fanta_canvas::handles::hit_test_resize_handle_oriented(
                local,
                &world,
                screen,
                &viewport,
                DVec2::new(width, height),
                fanta_canvas::handles::DEFAULT_HANDLE_THRESHOLD,
            );
        }
        let selection = document.doc.selection.as_slice();
        let [id] = selection else {
            return None;
        };
        let world_transform = document.doc.scene.world_transform(*id)?;
        if fanta_canvas::handles::transform_angle(&world_transform).abs() > 1e-4 {
            return None;
        }
        let world = document.doc.scene.world_bounds(*id)?;
        fanta_canvas::handles::hit_test_resize_handle_screen(
            world,
            screen,
            &viewport,
            DVec2::new(width, height),
            fanta_canvas::handles::DEFAULT_HANDLE_THRESHOLD,
        )
    }

    /// Whether the page the canvas is showing has no children yet. Cheap
    /// enough for `render`: `children_of` hands back a slice.
    fn active_page_is_empty(&self, cx: &App) -> bool {
        self.item.read(cx).document().is_some_and(|document| {
            document
                .doc
                .active_page()
                .is_some_and(|root| document.doc.scene.children_of(Some(root)).is_empty())
        })
    }

    fn handle_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Presentation mode: the wheel scrolls the prototype's scrollable
        // container under the cursor (authored Figma overflow — clamped,
        // fixed/sticky children honored), never the editor viewport.
        if self.prototype_player.is_some() {
            let delta = match event.delta {
                ScrollDelta::Pixels(pixels) => pixels,
                ScrollDelta::Lines(lines) => lines.map(|line| px(line * SCROLL_LINE_MULTIPLIER)),
            };
            let Some(bounds) = self.container_bounds else {
                return;
            };
            let screen = screen_position_in_bounds(event.position, bounds);
            let scroll_delta = [
                -f64::from(f32::from(delta.x)),
                -f64::from(f32::from(delta.y)),
            ];
            if let Some(player) = self.prototype_player.as_mut()
                && player.scroll_by_screen(screen, scroll_delta).is_some()
            {
                self.invalidate_canvas_cache();
                cx.notify();
            }
            return;
        }
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
        self.zoom_to_percent(100, cx);
    }

    fn zoom_to_selection_action(
        &mut self,
        _: &ZoomToSelection,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.zoom_to_selection(cx);
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
        self.finish_document_edits(cx);
        let changed = self.item.update(cx, |item, cx| match item.undo(cx) {
            Ok(changed) => changed,
            Err(error) => {
                log::error!("fig_viewer undo failed: {error:#}");
                false
            }
        });
        if changed {
            self.refresh_tool_overlays(cx);
        }
    }

    fn redo(&mut self, _: &Redo, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_editable(cx) {
            return;
        }
        self.finish_document_edits(cx);
        let changed = self.item.update(cx, |item, cx| match item.redo(cx) {
            Ok(changed) => changed,
            Err(error) => {
                log::error!("fig_viewer redo failed: {error:#}");
                false
            }
        });
        if changed {
            self.refresh_tool_overlays(cx);
        }
    }

    fn refresh_tool_overlays(&mut self, cx: &mut Context<Self>) {
        if let Some(doc) = self.item.read(cx).doc()
            && self.tools.refresh_overlays(doc)
        {
            cx.notify();
        }
    }

    fn cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(target_os = "macos")]
        if self
            .canvas_video
            .as_ref()
            .is_some_and(|session| session.trim.is_some())
        {
            self.cancel_video_trim(cx);
            return;
        }
        if self.prototype_player.is_some() {
            self.exit_prototype_session(cx);
            return;
        }
        // Escape while composing a comment discards the draft and exits the
        // comment tool entirely (Figma-style), before any other cancel.
        if self.cancel_comment_draft(cx) {
            return;
        }
        if self.comment_state.open_thread.is_some() {
            self.comment_state.open_thread = None;
            self.comment_state.reply_editor = None;
            cx.notify();
            return;
        }
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
        if self.prototype_player.is_some() {
            self.trigger_prototype_key("enter", cx);
            return;
        }
        self.finish_panel_edits(cx);
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
        if matches!(self.tools.kind(), ToolKind::NodeEdit | ToolKind::PathSelect) {
            if self.is_editable(cx) {
                self.finish_document_edits(cx);
                self.dispatch_tool_event(key_event(LogicalKey::Delete, window.modifiers()), cx);
            }
        } else {
            self.delete_selected_nodes(cx);
        }
    }

    fn copy_selection(&mut self, _: &CopySelection, _window: &mut Window, cx: &mut Context<Self>) {
        self.copy_selected_nodes(cx);
    }

    fn cut_selection(&mut self, _: &CutSelection, _window: &mut Window, cx: &mut Context<Self>) {
        self.cut_selected_nodes(cx);
    }

    fn paste_selection(
        &mut self,
        _: &PasteSelection,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.paste_selected_nodes(cx);
    }

    fn duplicate_selection(
        &mut self,
        _: &DuplicateSelection,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.duplicate_selected_nodes(cx);
    }

    fn group_selection(&mut self, _: &GroupSelection, window: &mut Window, cx: &mut Context<Self>) {
        self.group_nodes(None, window, cx);
    }

    fn ungroup_selection(
        &mut self,
        _: &UngroupSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ungroup_nodes(None, window, cx);
    }

    fn frame_selection(&mut self, _: &FrameSelection, window: &mut Window, cx: &mut Context<Self>) {
        self.frame_nodes(None, window, cx);
    }

    /// Wrap the structure targets (see [`Self::structure_targets`]) in a new
    /// group and select it.
    pub(crate) fn group_nodes(
        &mut self,
        clicked: Option<NodeId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_structure_edit(
            "Group",
            move |doc| {
                let targets = structure_targets(doc, clicked);
                let grouped = crate::structure::group_operations(doc, &targets, None)?;
                Ok((grouped.operations, vec![grouped.group]))
            },
            window,
            cx,
        );
    }

    /// Wrap the structure targets in a new clipping frame and select it.
    pub(crate) fn frame_nodes(
        &mut self,
        clicked: Option<NodeId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_structure_edit(
            "Frame selection",
            move |doc| {
                let targets = structure_targets(doc, clicked);
                let grouped = crate::structure::frame_selection_operations(doc, &targets, None)?;
                Ok((grouped.operations, vec![grouped.group]))
            },
            window,
            cx,
        );
    }

    /// Dissolve every group or frame among the structure targets and select
    /// the freed children. Targets nested in another target are skipped, as
    /// their parent's ungroup already moves them.
    pub(crate) fn ungroup_nodes(
        &mut self,
        clicked: Option<NodeId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_structure_edit(
            "Ungroup",
            move |doc| {
                let targets = structure_targets(doc, clicked);
                let groups = ungroupable_targets(doc, &targets);
                if groups.is_empty() {
                    anyhow::bail!("select a group or frame to ungroup");
                }
                crate::structure::ungroup_many_operations(doc, &groups)
            },
            window,
            cx,
        );
    }

    /// Run one structural transaction built from the current document and
    /// select `select` afterwards. A refusal (a page in the selection, no
    /// group to ungroup) is shown as a canvas notice: the command came from a
    /// visible control, so silence would read as a broken button.
    fn apply_structure_edit(
        &mut self,
        label: &str,
        build: impl FnOnce(&Doc) -> Result<(Vec<Operation>, Vec<NodeId>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_editable(cx) {
            return;
        }
        self.finish_document_edits(cx);
        let result = self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let (operations, select) = match build(&document.doc) {
                    Ok(built) => built,
                    Err(error) => return (Err(error), DocChange::None),
                };
                match apply_canvas_transaction(&mut document.doc, label, operations) {
                    Ok(true) => {
                        document.doc.selection.replace_with(select);
                        (Ok(()), DocChange::Content)
                    }
                    Ok(false) => (Ok(()), DocChange::None),
                    Err(error) => (Err(error), DocChange::None),
                }
            })
            .unwrap_or_else(|| Err(anyhow::anyhow!("the document is no longer available")))
        });
        if let Err(error) = result {
            log::warn!("{label} on the canvas selection failed: {error:#}");
            show_canvas_notice(format!("{label}: {error:#}"), window, cx);
        }
    }

    /// Select every top-level node of the page the canvas is showing.
    ///
    /// A document with no active page renders all of its roots, and
    /// `children_of(None)` would then hand back the pages themselves —
    /// selecting pages is not what "select all" means, so bail instead.
    fn select_all(&mut self, _: &SelectAll, _window: &mut Window, cx: &mut Context<Self>) {
        // The inline text session owns its own selection; selecting canvas
        // nodes underneath it mid-typing is never what the user asked for.
        if self.text_edit.is_some() {
            return;
        }
        self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let Some(root) = document.doc.active_page() else {
                    return ((), DocChange::None);
                };
                // `children_of` borrows the scene that the selection sits
                // beside, so the ids must be copied out before mutating it.
                let nodes = document.doc.scene.children_of(Some(root)).to_vec();
                document.doc.selection.clear();
                for node in nodes {
                    document.doc.selection.add(node);
                }
                ((), DocChange::Selection)
            });
        });
    }

    pub(crate) fn copy_selected_nodes(&mut self, cx: &mut Context<Self>) {
        self.finish_document_edits(cx);
        let payload = self
            .item
            .read(cx)
            .document()
            .and_then(|document| CanvasClipboard::capture(&document.doc));
        if let Some(payload) = payload {
            cx.write_to_clipboard(ClipboardItem::new_string_with_json_metadata(
                payload.display_text(),
                payload,
            ));
        }
    }

    pub(crate) fn cut_selected_nodes(&mut self, cx: &mut Context<Self>) {
        if !self.is_editable(cx) {
            return;
        }
        self.copy_selected_nodes(cx);
        self.delete_selected_nodes(cx);
    }

    pub(crate) fn delete_selected_nodes(&mut self, cx: &mut Context<Self>) {
        if !self.is_editable(cx) {
            return;
        }
        self.finish_document_edits(cx);
        let result = self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let operations = delete_operations(&document.doc);
                match apply_canvas_transaction(&mut document.doc, "Delete", operations) {
                    Ok(true) => {
                        document.doc.selection.clear();
                        (Ok(true), DocChange::Content)
                    }
                    Ok(false) => (Ok(false), DocChange::None),
                    Err(error) => (Err(error), DocChange::None),
                }
            })
            .unwrap_or_else(|| Err(anyhow::anyhow!("the document is no longer available")))
        });
        if let Err(error) = result {
            log::error!("deleting canvas selection failed: {error:#}");
        }
    }

    /// Paste in priority order: a canvas payload copied from this app, then
    /// image bytes from another app, then image files copied in a file
    /// manager. Text without a canvas payload is not pastable on the canvas.
    pub(crate) fn paste_selected_nodes(&mut self, cx: &mut Context<Self>) {
        if !self.is_editable(cx) {
            return;
        }
        let Some(clipboard) = cx.read_from_clipboard() else {
            return;
        };
        let payload = clipboard.entries.iter().find_map(|entry| match entry {
            ClipboardEntry::String(string) => string.metadata_json::<CanvasClipboard>(),
            ClipboardEntry::Image(_) | ClipboardEntry::ExternalPaths(_) => None,
        });
        if let Some(payload) = payload {
            self.finish_document_edits(cx);
            self.insert_clipboard_payload(&payload, "Paste", 16.0, ClipboardPlacement::Paste, cx);
            return;
        }

        let images = clipboard
            .entries
            .iter()
            .filter_map(|entry| match entry {
                ClipboardEntry::Image(image) => Some(image.bytes().to_vec()),
                ClipboardEntry::String(_) | ClipboardEntry::ExternalPaths(_) => None,
            })
            .collect::<Vec<_>>();
        if !images.is_empty() {
            self.finish_document_edits(cx);
            self.place_pasted_images(images, cx);
            return;
        }

        let paths = clipboard
            .entries
            .iter()
            .filter_map(|entry| match entry {
                ClipboardEntry::ExternalPaths(paths) => Some(paths.paths()),
                ClipboardEntry::String(_) | ClipboardEntry::Image(_) => None,
            })
            .flatten()
            .filter(|path| is_pastable_image_path(path))
            .cloned()
            .collect::<Vec<_>>();
        if paths.is_empty() {
            return;
        }
        self.finish_document_edits(cx);
        let read = cx.background_spawn(async move {
            paths
                .iter()
                .map(|path| read_pastable_image_file(path))
                .collect::<Vec<Result<Vec<u8>>>>()
        });
        cx.spawn(async move |this, cx| {
            let files = read.await;
            this.update(cx, |this, cx| {
                let mut images = Vec::new();
                for file in files {
                    match file {
                        Ok(bytes) => images.push(bytes),
                        Err(error) => {
                            log::warn!("pasting an image file failed: {error:#}");
                            show_canvas_notice_deferred(
                                format!("Pasting an image file failed: {error:#}"),
                                cx,
                            );
                        }
                    }
                }
                if !images.is_empty() {
                    this.place_pasted_images(images, cx);
                }
            })
        })
        .detach_and_log_err(cx);
    }

    /// Ingest every image as a project asset and place them as bitmap layers
    /// centred on the viewport in one transaction, so a multi-file paste is a
    /// single undo step. An image that cannot be ingested (undecodable, or
    /// over the asset cap) is reported and skipped without failing the rest.
    fn place_pasted_images(&mut self, images: Vec<Vec<u8>>, cx: &mut Context<Self>) {
        let viewport = self.viewport.unwrap_or(Viewport {
            center: [0.0, 0.0],
            zoom: 1.0,
        });
        let visible = self
            .container_bounds
            .map(bounds_size)
            .map(|(width, height)| [width / viewport.zoom, height / viewport.zoom])
            .unwrap_or([1024.0, 768.0]);

        let pasted = self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let (doc, mut assets) = document.doc_and_assets();
                match paste_images(doc, &mut assets, images, viewport.center, visible) {
                    Ok(pasted) => {
                        let change = if pasted.placed.is_empty() {
                            DocChange::None
                        } else {
                            DocChange::Content
                        };
                        (Ok(pasted), change)
                    }
                    Err(error) => (Err(error), DocChange::None),
                }
            })
            .unwrap_or_else(|| Err(anyhow::anyhow!("the document is no longer available")))
        });
        let pasted = match pasted {
            Ok(pasted) => pasted,
            Err(error) => {
                log::warn!("pasting images failed: {error:#}");
                show_canvas_notice_deferred(format!("Pasting images failed: {error:#}"), cx);
                return;
            }
        };
        for error in &pasted.skipped {
            log::warn!("pasting an image failed: {error:#}");
            show_canvas_notice_deferred(format!("Pasting an image failed: {error:#}"), cx);
        }
        if pasted.placed.is_empty() {
            return;
        }
        self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.selection.replace_with(pasted.placed);
                ((), DocChange::Selection)
            });
        });
    }

    pub(crate) fn duplicate_selected_nodes(&mut self, cx: &mut Context<Self>) {
        if !self.is_editable(cx) {
            return;
        }
        self.finish_document_edits(cx);
        let payload = self
            .item
            .read(cx)
            .document()
            .and_then(|document| CanvasClipboard::capture(&document.doc));
        if let Some(payload) = payload {
            self.insert_clipboard_payload(
                &payload,
                "Duplicate",
                16.0,
                ClipboardPlacement::Duplicate,
                cx,
            );
        }
    }

    fn insert_clipboard_payload(
        &mut self,
        payload: &CanvasClipboard,
        label: &str,
        offset: f64,
        placement: ClipboardPlacement,
        cx: &mut Context<Self>,
    ) {
        let result = self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let pasted = match payload.instantiate(&document.doc, offset, placement) {
                    Ok(pasted) => pasted,
                    Err(error) => return (Err(error), DocChange::None),
                };
                match apply_canvas_transaction(&mut document.doc, label, create_operations(&pasted))
                {
                    Ok(true) => {
                        document.doc.selection.replace_with(pasted.roots);
                        (Ok(true), DocChange::Content)
                    }
                    Ok(false) => (Ok(false), DocChange::None),
                    Err(error) => (Err(error), DocChange::None),
                }
            })
            .unwrap_or_else(|| Err(anyhow::anyhow!("the document is no longer available")))
        });
        if let Err(error) = result {
            log::error!("{label} canvas selection failed: {error:#}");
        }
    }

    fn nudge(&mut self, key: LogicalKey, window: &mut Window, cx: &mut Context<Self>) {
        if self.prototype_player.is_some() {
            match key {
                LogicalKey::ArrowLeft => self.show_previous_prototype_frame(cx),
                LogicalKey::ArrowRight => self.show_next_prototype_frame(cx),
                LogicalKey::ArrowUp | LogicalKey::ArrowDown => {}
                LogicalKey::Escape | LogicalKey::Enter | LogicalKey::Delete => {}
            }
            return;
        }
        if self.is_editable(cx) {
            self.finish_document_edits(cx);
            self.dispatch_tool_event(key_event(key, window.modifiers()), cx);
        }
    }

    pub(crate) fn is_presenting_prototype(&self) -> bool {
        self.prototype_player.is_some()
    }

    fn play_prototype(&mut self, _: &PlayPrototype, window: &mut Window, cx: &mut Context<Self>) {
        if self.prototype_player.is_some() {
            return;
        }
        self.finish_document_edits(cx);
        let screen_size = self
            .container_bounds
            .map(bounds_size)
            .map(|(width, height)| DVec2::new(width, height))
            .unwrap_or_else(|| DVec2::new(960.0, 640.0));
        let player = self
            .item
            .read(cx)
            .document()
            .ok_or_else(|| anyhow::anyhow!("The design is not ready yet"))
            .and_then(|document| {
                PrototypePlayerState::try_start(
                    &document.doc,
                    document.asset_resolver.clone(),
                    screen_size,
                )
            });
        let player = match player {
            Ok(player) => player,
            Err(error) => {
                let detail = format!("{error:#}");
                drop(window.prompt(
                    gpui::PromptLevel::Warning,
                    "Cannot present prototype",
                    Some(&detail),
                    &["OK"],
                    cx,
                ));
                return;
            }
        };
        if self.editor_workspace(cx) != EditorWorkspace::Canvas {
            self.set_editor_workspace(EditorWorkspace::Canvas, cx);
        }
        if self.editor_mode(cx) != EditorMode::Prototype {
            self.set_editor_mode(EditorMode::Prototype, cx);
        }
        self.prototype_saved_viewport = self.viewport;
        self.prototype_player = Some(player);
        self.prototype_tick_task = None;
        self.prototype_last_tick = Some(std::time::Instant::now());
        self.prototype_pointer_down = None;
        self.prototype_drag_fired = false;
        self.prototype_suppress_click = false;
        self.prototype_link_notice = None;
        self.primary_pressed = false;
        self.pan_last_position = None;
        self.hovered_node = None;
        self.invalidate_canvas_cache();
        self.focus_handle.focus(window, cx);
        self.start_prototype_clock(cx);
        cx.notify();
    }

    fn exit_prototype(&mut self, _: &ExitPrototype, _window: &mut Window, cx: &mut Context<Self>) {
        self.exit_prototype_session(cx);
    }

    fn exit_prototype_session(&mut self, cx: &mut Context<Self>) {
        let Some(player) = self.prototype_player.take() else {
            return;
        };
        drop(player);
        self.prototype_tick_task = None;
        self.prototype_last_tick = None;
        self.prototype_pointer_down = None;
        self.prototype_drag_fired = false;
        self.prototype_suppress_click = false;
        self.prototype_link_notice = None;
        self.viewport = self.prototype_saved_viewport.take();
        self.invalidate_canvas_cache();
        cx.notify();
    }

    fn restart_prototype(
        &mut self,
        _: &RestartPrototype,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(player) = self.prototype_player.as_mut() else {
            return;
        };
        if player.restart() {
            self.prototype_last_tick = Some(std::time::Instant::now());
            self.invalidate_canvas_cache();
            cx.notify();
        }
    }

    fn prototype_previous_frame(
        &mut self,
        _: &PrototypePreviousFrame,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_previous_prototype_frame(cx);
    }

    fn prototype_next_frame(
        &mut self,
        _: &PrototypeNextFrame,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_next_prototype_frame(cx);
    }

    fn show_previous_prototype_frame(&mut self, cx: &mut Context<Self>) {
        let Some(player) = self.prototype_player.as_mut() else {
            return;
        };
        if player.show_previous_frame() {
            self.prototype_last_tick = Some(std::time::Instant::now());
            self.invalidate_canvas_cache();
            cx.notify();
        }
    }

    fn show_next_prototype_frame(&mut self, cx: &mut Context<Self>) {
        let Some(player) = self.prototype_player.as_mut() else {
            return;
        };
        if player.show_next_frame() {
            self.prototype_last_tick = Some(std::time::Instant::now());
            self.invalidate_canvas_cache();
            cx.notify();
        }
    }

    fn trigger_prototype_click(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.trigger_prototype_pointer(position, fanta_present::PointerEvent::Click, cx);
    }

    fn trigger_prototype_pointer(
        &mut self,
        position: Point<Pixels>,
        event: fanta_present::PointerEvent,
        cx: &mut Context<Self>,
    ) -> fanta_present::PresentResponse {
        let Some(bounds) = self.container_bounds else {
            return fanta_present::PresentResponse::default();
        };
        let screen = screen_position_in_bounds(position, bounds);
        let Some(player) = self.prototype_player.as_mut() else {
            return fanta_present::PresentResponse::default();
        };
        let response = player.handle_pointer(screen, event);
        self.handle_prototype_response(response, cx);
        response
    }

    fn leave_prototype_surface(&mut self, cx: &mut Context<Self>) {
        let Some(player) = self.prototype_player.as_mut() else {
            return;
        };
        let response = player.handle_pointer(DVec2::ZERO, fanta_present::PointerEvent::Leave);
        self.prototype_pointer_down = None;
        self.prototype_drag_fired = false;
        self.prototype_suppress_click = false;
        self.handle_prototype_response(response, cx);
    }

    fn trigger_prototype_key(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(player) = self.prototype_player.as_mut() else {
            return;
        };
        let response = player.handle_key(key);
        self.handle_prototype_response(response, cx);
    }

    fn handle_prototype_response(
        &mut self,
        response: fanta_present::PresentResponse,
        cx: &mut Context<Self>,
    ) {
        if let Some(url) = self
            .prototype_player
            .as_mut()
            .and_then(PrototypePlayerState::take_open_url)
            .filter(|url| !url.trim().is_empty())
        {
            self.prototype_link_notice = Some(match validate_prototype_link(&url) {
                Ok(url) => PrototypeLinkNotice::Confirm(url),
                Err(error) => PrototypeLinkNotice::Invalid(error),
            });
            cx.notify();
        }
        if response.exited {
            self.exit_prototype_session(cx);
            return;
        }
        if response.navigated {
            self.prototype_last_tick = Some(std::time::Instant::now());
        }
        if response.needs_redraw || response.navigated || response.media_updated {
            self.invalidate_canvas_cache();
            cx.notify();
        }
    }

    pub(crate) fn render_prototype_rgba(
        &mut self,
        size: (u32, u32),
        display_scale: f64,
    ) -> anyhow::Result<(u32, u32, Vec<u8>)> {
        let player = self
            .prototype_player
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("prototype presentation is not active"))?;
        player.resize(
            DVec2::new(f64::from(size.0), f64::from(size.1)),
            display_scale,
        )?;
        Ok(player.present_rgba())
    }

    pub(crate) fn render_prototype_image(
        &mut self,
        logical_size: (u32, u32),
        display_scale: f32,
    ) -> anyhow::Result<std::sync::Arc<RenderImage>> {
        let scale_bits = display_scale.to_bits();
        if let Some(cache) = &self.prototype_render_cache
            && cache.logical_size == logical_size
            && cache.scale_bits == scale_bits
        {
            return Ok(cache.image.clone());
        }
        let (width, height, pixels) =
            self.render_prototype_rgba(logical_size, f64::from(display_scale))?;
        let image = crate::canvas::render_image_from_rgba(width, height, pixels, display_scale)?;
        self.prototype_render_cache = Some(PrototypeRenderCache {
            logical_size,
            scale_bits,
            image: image.clone(),
        });
        Ok(image)
    }

    fn start_prototype_clock(&mut self, cx: &mut Context<Self>) {
        self.prototype_tick_task = None;
        self.prototype_last_tick = Some(std::time::Instant::now());
        self.prototype_tick_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(16))
                    .await;
                let keep_running = this.update(cx, |this, cx| {
                    let Some(player) = this.prototype_player.as_mut() else {
                        return false;
                    };
                    let now = std::time::Instant::now();
                    let elapsed = prototype_tick_elapsed(&mut this.prototype_last_tick, now);
                    let response = player.tick_elapsed(elapsed);
                    this.handle_prototype_response(response, cx);
                    this.prototype_player.is_some()
                });
                if !matches!(keep_running, Ok(true)) {
                    break;
                }
            }
        }));
    }

    // === Pages ============================================================

    pub fn select_page(&mut self, index: usize, cx: &mut Context<Self>) {
        // The edited node stays behind on the old page; end the session
        // before the canvas stops rendering it.
        self.finish_document_edits(cx);
        let (root, prewarm) = self
            .item
            .update(cx, |item, cx| {
                item.with_document(cx, |document| {
                    document.ensure_page_solved(index);
                    let root = document.pages.get(index).and_then(|page| page.root);
                    if document.doc.set_active_page(root) {
                        document.doc.selection.clear();
                    }
                    let prewarm = root.and_then(|root| document.take_page_prewarm(root));
                    ((root, prewarm), DocChange::Selection)
                })
            })
            .unwrap_or((None, None));
        if let Some(prewarm) = prewarm {
            // Decoding the page's images here would stall the frame that
            // shows it; the render thread picks up whatever has landed.
            cx.background_spawn(async move { prewarm.run() }).detach();
        }
        self.selected_page_index = Some(index);
        self.selected_page_root = root;
        // Focus re-assertion must follow in-tab navigation: this tab now
        // means this page, and the root it shows is already current.
        self.scope = root.map(FigScope::Page);
        self.last_seen_root = root;
        self.viewport = None;
        self.hovered_node = None;
        cx.notify();
    }

    // === Chrome ===========================================================

    /// Arm the debounced autosave. Every committing edit re-arms it, so a
    /// burst of edits (or a gesture that commits per step) costs one write
    /// about a second after the user stops.
    fn schedule_autosave(&mut self, cx: &mut Context<Self>) {
        if !self.autosave_allowed(cx) {
            self.autosave_task = None;
            return;
        }
        self.autosave_task = Some(cx.spawn(async move |view, cx| {
            cx.background_executor().timer(AUTOSAVE_DEBOUNCE).await;
            view.update(cx, |view, cx| view.autosave_now(cx)).log_err();
        }));
    }

    /// Whether writing the document right now is both possible and harmless.
    ///
    /// A bare `.fig` with no materialized project is excluded on purpose: its
    /// first save scaffolds a directory next to the file, and creating one
    /// behind the user's back is not something a timer should do.
    fn autosave_allowed(&self, cx: &App) -> bool {
        let item = self.item.read(cx);
        item.project_root().is_some()
            && item.is_dirty()
            && !item.has_conflict()
            && !item.source_edit_locked()
            // An open text session, a running prototype, or a keyframe drag
            // each hold document state that a write would freeze mid-gesture.
            && self.text_edit.is_none()
            && self.pending_text_edit.is_none()
            && self.prototype_player.is_none()
            && self.motion_keyframe_drag.is_none()
    }

    /// The debounce elapsed: write the document as it stands.
    ///
    /// Deliberately does NOT run `finish_document_edits`: that commits
    /// half-typed inspector values through `finish_panel_edits`. Any in-flight
    /// panel edit emits its own `Edited` when the user commits it, which
    /// re-arms this timer.
    fn autosave_now(&mut self, cx: &mut Context<Self>) {
        self.autosave_task = None;
        if self.canvas_pointer_down {
            // `mark_edited` announces the dirty transition on the FIRST
            // preview frame of a drag from a clean document, so the timer can
            // expire mid-gesture. Writing here would put an intermediate
            // position on disk; wait for the release instead.
            self.schedule_autosave(cx);
            return;
        }
        if !self.autosave_allowed(cx) {
            return;
        }
        let save = self
            .item
            .update(cx, |item, cx| item.save(SaveKind::Auto, cx));
        cx.spawn(async move |_, _| {
            if let Err(error) = save.await {
                log::error!("autosaving the canvas failed: {error:#}");
            }
        })
        .detach();
    }

    /// Commit any in-flight text session and persist the document. Returns the
    /// project directory the save materialized (only on the first save of a
    /// lone `.fig`, which turns it into an on-disk project), or `None` when the
    /// project already existed.
    fn save_document(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Task<Result<Option<std::path::PathBuf>>> {
        if self.prototype_player.is_some() {
            self.exit_prototype_session(cx);
        }
        // Persist committed state, not a transient preview mid-session.
        self.finish_document_edits(cx);
        self.item
            .update(cx, |item, cx| item.save(SaveKind::Explicit, cx))
    }

    fn toggle_layers_sidebar(
        &mut self,
        _: &ToggleLayersSidebar,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.layers_sidebar_visible = !self.layers_sidebar_visible;
        self.persist_sidebar_layout(cx);
        cx.notify();
    }

    fn toggle_inspector_sidebar(
        &mut self,
        _: &ToggleInspectorSidebar,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.inspector_sidebar_visible = !self.inspector_sidebar_visible;
        self.persist_sidebar_layout(cx);
        cx.notify();
    }

    fn persist_sidebar_layout(&self, cx: &mut Context<Self>) {
        let fs = self.project.read(cx).fs().clone();
        let layers_sidebar_visible = self.layers_sidebar_visible;
        let inspector_sidebar_visible = self.inspector_sidebar_visible;
        let layers_sidebar_width = self.layers_sidebar_width.as_f32();
        let inspector_sidebar_width = self.inspector_sidebar_width.as_f32();
        update_settings_file(fs, cx, move |settings, _| {
            let design_panel = settings.fanta_design_panel.get_or_insert_default();
            design_panel.visible = Some(layers_sidebar_visible);
            design_panel.default_width = Some(layers_sidebar_width);

            let properties_panel = settings.fanta_properties_panel.get_or_insert_default();
            properties_panel.visible = Some(inspector_sidebar_visible);
            properties_panel.default_width = Some(inspector_sidebar_width);
        });
    }

    fn handle_sidebar_resize_drag(
        &mut self,
        event: &DragMoveEvent<SidebarResizeDrag>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let drag = event.drag(cx);
        let pointer_x = event.event.position.x.as_f32();
        let bounds_left = event.bounds.origin.x.as_f32();
        let bounds_right = bounds_left + event.bounds.size.width.as_f32();
        match drag.sidebar {
            SidebarKind::Layers => {
                self.layers_sidebar_width =
                    clamp_sidebar_width(px(pointer_x - bounds_left), SidebarKind::Layers);
            }
            SidebarKind::Inspector => {
                self.inspector_sidebar_width =
                    clamp_sidebar_width(px(bounds_right - pointer_x), SidebarKind::Inspector);
            }
        }
        cx.notify();
    }

    fn render_layers_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let sidebar = div()
            .id("fanta-layers-sidebar")
            .relative()
            .h_full()
            .w(self.layers_sidebar_width)
            .flex_shrink_0()
            .border_r_1()
            .border_color(cx.theme().colors().border)
            .child(self.layers_sidebar.clone())
            .child(self.render_sidebar_resize_handle(SidebarKind::Layers));
        #[cfg(test)]
        let sidebar = sidebar.debug_selector(|| "fanta-layers-sidebar".to_owned());
        sidebar.into_any_element()
    }

    fn render_inspector_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mode = self.editor_mode(cx);
        let view = cx.weak_entity();
        let tabs = EditorModeTabs::new(mode, move |mode, _, cx| {
            view.update(cx, |view, cx| view.set_editor_mode(mode, cx))
                .log_err();
        });
        let body = match mode {
            EditorMode::Prototype => self.prototype_sidebar.clone().into_any_element(),
            EditorMode::Comments => self.render_comments_sidebar(cx),
            EditorMode::Motion => self.motion_sidebar.clone().into_any_element(),
            EditorMode::Design => {
                // The fanta-gpui DesignPanel replaces the legacy inspector
                // when its adapter mounted; `FANTA_GPUI_DESIGN=0` (or the
                // process-wide `FANTA_GPUI_UI=0`) keeps the legacy panel.
                #[cfg(feature = "fanta-gpui-ui")]
                let body = match self.gpui_design.as_ref() {
                    Some(adapter) => adapter.panel.clone().into_any_element(),
                    None => self.inspector_sidebar.clone().into_any_element(),
                };
                #[cfg(not(feature = "fanta-gpui-ui"))]
                let body = self.inspector_sidebar.clone().into_any_element();
                body
            }
        };
        let sidebar = div()
            .id("fanta-inspector-sidebar")
            .relative()
            .h_full()
            .w(self.inspector_sidebar_width)
            .flex_shrink_0()
            .border_l_1()
            .border_color(cx.theme().colors().border)
            .child(
                v_flex()
                    .size_full()
                    .overflow_hidden()
                    .child(h_flex().flex_none().justify_center().py_1().child(tabs))
                    .child(div().flex_1().min_h_0().child(body)),
            )
            .child(self.render_sidebar_resize_handle(SidebarKind::Inspector));
        #[cfg(test)]
        let sidebar = sidebar.debug_selector(|| "fanta-inspector-sidebar".to_owned());
        sidebar.into_any_element()
    }

    fn render_workspace_tabs(&self, cx: &mut Context<Self>) -> AnyElement {
        let workspace = self.editor_workspace(cx);
        let view = cx.weak_entity();
        let tabs = EditorWorkspaceTabs::new(workspace, move |workspace, _, cx| {
            view.update(cx, |view, cx| view.set_editor_workspace(workspace, cx))
                .log_err();
        });
        h_flex()
            .absolute()
            .top(px(8.0))
            .left_0()
            .right_0()
            .justify_center()
            .child(div().occlude().child(tabs))
            .into_any_element()
    }

    fn render_comments_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let rows = self
            .item
            .read(cx)
            .document()
            .map(|document| document_comment_rows(&document.doc, &document.pages))
            .unwrap_or_default();
        let view = cx.weak_entity();
        FantaCommentsPanel::new(rows, move |page_index, comment_id, window, cx| {
            view.update(cx, |view, cx| {
                view.select_page(page_index, cx);
                if view.comment_state.open_thread.as_deref() != Some(comment_id.as_str()) {
                    view.toggle_comment_thread(comment_id, window, cx);
                }
            })
            .log_err();
        })
        .into_any_element()
    }

    fn render_source_edit_lock_banner(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .id("fanta-source-edit-lock-banner")
            .absolute()
            .top(px(48.))
            .left_0()
            .right_0()
            .px_4()
            .justify_center()
            .child(
                h_flex()
                    .occlude()
                    .w_full()
                    .min_w_0()
                    .max_w(px(620.))
                    .px_3()
                    .py_1p5()
                    .gap_2()
                    .rounded_md()
                    .border_1()
                    .border_color(Color::Warning.color(cx))
                    .bg(cx.theme().colors().panel_background)
                    .child(
                        Icon::new(IconName::Lock)
                            .size(IconSize::XSmall)
                            .color(Color::Warning),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            Label::new(
                                "Canvas editing is locked while FNX has unsaved changes. Pan and selection remain available; save FNX to resume editing.",
                            )
                            .size(LabelSize::Small)
                            .line_clamp(2),
                        ),
                    ),
            )
            .into_any_element()
    }

    fn render_sidebar_resize_handle(&self, sidebar: SidebarKind) -> AnyElement {
        div()
            .id(match sidebar {
                SidebarKind::Layers => "fanta-layers-sidebar-resize-handle",
                SidebarKind::Inspector => "fanta-inspector-sidebar-resize-handle",
            })
            .absolute()
            .top(px(0.))
            .when(sidebar == SidebarKind::Layers, |this| {
                this.right(-SIDEBAR_RESIZE_HANDLE_SIZE / 2.)
            })
            .when(sidebar == SidebarKind::Inspector, |this| {
                this.left(-SIDEBAR_RESIZE_HANDLE_SIZE / 2.)
            })
            .h_full()
            .w(SIDEBAR_RESIZE_HANDLE_SIZE)
            .cursor_col_resize()
            .on_drag(SidebarResizeDrag { sidebar }, |drag, _, _, cx| {
                cx.new(|_| drag.clone())
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
            .occlude()
            .into_any_element()
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
        let view = cx.weak_entity();

        h_flex()
            .debug_selector(|| "fanta-canvas-toolbar".to_owned())
            .absolute()
            .bottom(px(12.))
            .left_0()
            .right_0()
            .px_3()
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
                                let menu_view = view.clone();
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
                                        let view = menu_view.clone();
                                        Some(ContextMenu::build(
                                            window,
                                            cx,
                                            move |mut menu, _window, _cx| {
                                                for kind in group.iter().copied() {
                                                    let view = view.clone();
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
                                                            .handler(move |_window, cx| {
                                                                if let Err(error) =
                                                                    view.update(cx, |this, cx| {
                                                                        this.activate_tool(kind, cx);
                                                                    })
                                                                {
                                                                    log::debug!(
                                                                        "dropping toolbar tool activation for closed Figma view: {error:#}"
                                                                    );
                                                                }
                                                            })
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
                    )
                    .child(div().h(px(20.)).child(Divider::vertical()))
                    .child(
                        IconButton::new(
                            "fig-toggle-layers-sidebar",
                            if self.layers_sidebar_visible {
                                IconName::ThreadsSidebarLeftOpen
                            } else {
                                IconName::ThreadsSidebarLeftClosed
                            },
                        )
                        .icon_size(IconSize::Small)
                        .toggle_state(self.layers_sidebar_visible)
                        .icon_color(if self.layers_sidebar_visible {
                            Color::Accent
                        } else {
                            Color::Muted
                        })
                        .tooltip(Tooltip::text(if self.layers_sidebar_visible {
                            "Hide Layers"
                        } else {
                            "Show Layers"
                        }))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.toggle_layers_sidebar(&ToggleLayersSidebar, window, cx);
                        })),
                    )
                    .child(
                        IconButton::new(
                            "fig-toggle-inspector-sidebar",
                            if self.inspector_sidebar_visible {
                                IconName::ThreadsSidebarRightOpen
                            } else {
                                IconName::ThreadsSidebarRightClosed
                            },
                        )
                        .icon_size(IconSize::Small)
                        .toggle_state(self.inspector_sidebar_visible)
                        .icon_color(if self.inspector_sidebar_visible {
                            Color::Accent
                        } else {
                            Color::Muted
                        })
                        .tooltip(Tooltip::text(if self.inspector_sidebar_visible {
                            "Hide Inspector"
                        } else {
                            "Show Inspector"
                        }))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.toggle_inspector_sidebar(&ToggleInspectorSidebar, window, cx);
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

/// The centred "draw something" hint shown over an empty page. The keys match
/// the `FigViewer` tool bindings in `assets/keymaps/default-macos.json`.
fn render_empty_page_hint(cx: &App) -> AnyElement {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        // No listeners and no `occlude`, so the press that draws the first
        // shape passes straight through to the canvas container beneath.
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().colors().text_muted)
                .child("R rectangle · F frame · T text"),
        )
        .into_any_element()
}

/// The cursor for a selection handle: each handle resizes along its own axis
/// or diagonal.
fn resize_cursor(handle: fanta_canvas::ResizeHandle) -> CursorStyle {
    use fanta_canvas::ResizeHandle;
    match handle {
        ResizeHandle::North | ResizeHandle::South => CursorStyle::ResizeUpDown,
        ResizeHandle::East | ResizeHandle::West => CursorStyle::ResizeLeftRight,
        ResizeHandle::NorthWest | ResizeHandle::SouthEast => CursorStyle::ResizeUpLeftDownRight,
        ResizeHandle::NorthEast | ResizeHandle::SouthWest => CursorStyle::ResizeUpRightDownLeft,
    }
}

fn clamp_sidebar_width(width: Pixels, sidebar: SidebarKind) -> Pixels {
    let minimum = match sidebar {
        SidebarKind::Layers => MIN_LAYERS_SIDEBAR_WIDTH,
        SidebarKind::Inspector => MIN_INSPECTOR_SIDEBAR_WIDTH,
    };
    px(width.as_f32().clamp(minimum, MAX_SIDEBAR_WIDTH))
}

impl FigView {
    fn render_prototype_play_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.weak_entity();
        div()
            .absolute()
            .top_3()
            .right_3()
            .child(
                Button::new("fanta-prototype-present", "Present")
                    .start_icon(Icon::new(IconName::PlayFilled).size(IconSize::Small))
                    .style(ButtonStyle::Filled)
                    .tooltip(Tooltip::text("Present prototype (⌥⌘↵)"))
                    .on_click(move |_, window, cx| {
                        view.update(cx, |view, cx| {
                            view.play_prototype(&PlayPrototype, window, cx)
                        })
                        .log_err();
                    }),
            )
            .into_any_element()
    }

    fn render_prototype_presentation(&self, cx: &mut Context<Self>) -> AnyElement {
        let (title, position, can_go_back, can_go_forward) = self
            .prototype_player
            .as_ref()
            .map(|player| {
                let title = self
                    .item
                    .read(cx)
                    .document()
                    .and_then(|document| document.doc.scene.get(player.active_frame()))
                    .map(|node| node.name.trim())
                    .filter(|name| !name.is_empty())
                    .unwrap_or("Prototype")
                    .to_string();
                let position = player.frame_position();
                let can_go_back = position.is_some_and(|(index, _)| index > 1);
                let can_go_forward = position.is_some_and(|(index, count)| index < count);
                (title, position, can_go_back, can_go_forward)
            })
            .unwrap_or_else(|| ("Prototype".to_string(), None, false, false));

        let previous_view = cx.weak_entity();
        let next_view = cx.weak_entity();
        let restart_view = cx.weak_entity();
        let exit_view = cx.weak_entity();
        let flow_view = cx.weak_entity();
        // Named flows from the imported document, for the flow picker; a
        // single unnamed flow gets no picker.
        let flows: Vec<(usize, SharedString)> = self
            .prototype_player
            .as_ref()
            .map(|player| {
                player
                    .flows()
                    .iter()
                    .enumerate()
                    .map(|(index, flow)| (index, SharedString::from(flow.name.clone())))
                    .collect()
            })
            .unwrap_or_default();
        let link_notice = self.prototype_link_notice.clone();
        let nav_label = position
            .map(|(index, count)| format!("{index} / {count}"))
            .unwrap_or_else(|| "—".to_string());
        let chrome_border = cx.theme().colors().border;
        let chrome_background = cx.theme().colors().panel_background;
        let chrome = move |child: AnyElement| {
            h_flex()
                .h_9()
                .px_2()
                .gap_1()
                .rounded_lg()
                .border_1()
                .border_color(chrome_border)
                .bg(chrome_background)
                .shadow_sm()
                .child(child)
        };

        div()
            .id("fanta-prototype-presentation")
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(cx.theme().colors().editor_background)
            .child(
                div()
                    .id("fig-prototype-container")
                    .absolute()
                    .inset_0()
                    .overflow_hidden()
                    .cursor_pointer()
                    .on_hover(cx.listener(|view, hovered: &bool, _, cx| {
                        if !*hovered {
                            view.leave_prototype_surface(cx);
                        }
                    }))
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::handle_mouse_down))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::handle_mouse_up))
                    .on_mouse_move(cx.listener(Self::handle_mouse_move))
                    .child(CanvasElement::new(cx.entity())),
            )
            .child(
                h_flex()
                    .absolute()
                    .top_3()
                    .left_3()
                    .right_3()
                    .justify_between()
                    .child(chrome(
                        h_flex()
                            .gap_2()
                            .px_1()
                            .child(Icon::new(IconName::PlayFilled).size(IconSize::Small))
                            .child(Label::new(title).single_line())
                            .when(flows.len() > 1, |row| {
                                let flows = flows.clone();
                                row.child(
                                    PopoverMenu::new("fanta-prototype-flow-picker")
                                        .anchor(Anchor::TopLeft)
                                        .trigger(
                                            IconButton::new(
                                                "fanta-prototype-flow-caret",
                                                IconName::ChevronDown,
                                            )
                                            .icon_size(IconSize::XSmall)
                                            .icon_color(Color::Muted)
                                            .tooltip(Tooltip::text("Switch Flow")),
                                        )
                                        .menu(move |window, cx| {
                                            let flows = flows.clone();
                                            let flow_view = flow_view.clone();
                                            Some(ContextMenu::build(
                                                window,
                                                cx,
                                                move |mut menu, _window, _cx| {
                                                    for (index, name) in flows.iter() {
                                                        let flow_view = flow_view.clone();
                                                        let index = *index;
                                                        menu = menu.entry(
                                                            name.clone(),
                                                            None,
                                                            move |_, cx| {
                                                                flow_view
                                                                    .update(cx, |view, cx| {
                                                                        view.start_prototype_flow(
                                                                            index, cx,
                                                                        );
                                                                    })
                                                                    .ok();
                                                            },
                                                        );
                                                    }
                                                    menu
                                                },
                                            ))
                                        }),
                                )
                            })
                            .into_any_element(),
                    ))
                    .child(chrome(
                        h_flex()
                            .gap_1()
                            .child(
                                IconButton::new(
                                    "fanta-prototype-presentation-restart-top",
                                    IconName::RotateCcw,
                                )
                                .icon_size(IconSize::Small)
                                .tooltip(Tooltip::text("Restart prototype"))
                                .on_click(move |_, _, cx| {
                                    restart_view
                                        .update(cx, |view, cx| {
                                            let Some(player) = view.prototype_player.as_mut()
                                            else {
                                                return;
                                            };
                                            if player.restart() {
                                                view.prototype_last_tick =
                                                    Some(std::time::Instant::now());
                                                view.invalidate_canvas_cache();
                                                cx.notify();
                                            }
                                        })
                                        .log_err();
                                }),
                            )
                            .child(
                                IconButton::new(
                                    "fanta-prototype-presentation-close",
                                    IconName::Close,
                                )
                                .icon_size(IconSize::Small)
                                .tooltip(Tooltip::text("Return to editor"))
                                .on_click(move |_, _, cx| {
                                    exit_view
                                        .update(cx, |view, cx| view.exit_prototype_session(cx))
                                        .log_err();
                                }),
                            )
                            .into_any_element(),
                    )),
            )
            .child(
                h_flex()
                    .absolute()
                    .bottom_4()
                    .left_0()
                    .right_0()
                    .justify_center()
                    .child(chrome(
                        h_flex()
                            .gap_2()
                            .child(
                                IconButton::new(
                                    "fanta-prototype-presentation-previous",
                                    IconName::ArrowLeft,
                                )
                                .icon_size(IconSize::Small)
                                .disabled(!can_go_back)
                                .tooltip(Tooltip::text("Previous frame"))
                                .on_click(move |_, _, cx| {
                                    previous_view
                                        .update(cx, |view, cx| {
                                            view.show_previous_prototype_frame(cx)
                                        })
                                        .log_err();
                                }),
                            )
                            .child(Label::new(nav_label).size(LabelSize::Small))
                            .child(
                                IconButton::new(
                                    "fanta-prototype-presentation-next",
                                    IconName::ArrowRight,
                                )
                                .icon_size(IconSize::Small)
                                .disabled(!can_go_forward)
                                .tooltip(Tooltip::text("Next frame"))
                                .on_click(move |_, _, cx| {
                                    next_view
                                        .update(cx, |view, cx| view.show_next_prototype_frame(cx))
                                        .log_err();
                                }),
                            )
                            .into_any_element(),
                    )),
            )
            .when_some(link_notice, |this, notice| {
                let dismiss_view = cx.weak_entity();
                let content = match notice {
                    PrototypeLinkNotice::Confirm(url) => {
                        let open_view = cx.weak_entity();
                        let url_label = url.as_str().to_string();
                        v_flex()
                            .gap_3()
                            .child(Label::new("Open external link?").size(LabelSize::Large))
                            .child(
                                Label::new(url_label)
                                    .size(LabelSize::Small)
                                    .color(Color::Muted)
                                    .line_clamp(3),
                            )
                            .child(
                                h_flex()
                                    .justify_end()
                                    .gap_2()
                                    .child(
                                        Button::new("fanta-prototype-link-cancel", "Cancel")
                                            .on_click(move |_, _, cx| {
                                                dismiss_view
                                                    .update(cx, |view, cx| {
                                                        view.prototype_link_notice = None;
                                                        cx.notify();
                                                    })
                                                    .log_err();
                                            }),
                                    )
                                    .child(
                                        Button::new("fanta-prototype-link-open", "Open link")
                                            .style(ButtonStyle::Filled)
                                            .on_click(move |_, _, cx| {
                                                cx.open_url(url.as_str());
                                                open_view
                                                    .update(cx, |view, cx| {
                                                        view.prototype_link_notice = None;
                                                        cx.notify();
                                                    })
                                                    .log_err();
                                            }),
                                    ),
                            )
                            .into_any_element()
                    }
                    PrototypeLinkNotice::Invalid(message) => v_flex()
                        .gap_3()
                        .child(Label::new("Link blocked").size(LabelSize::Large))
                        .child(
                            Label::new(message)
                                .size(LabelSize::Small)
                                .color(Color::Muted)
                                .line_clamp(3),
                        )
                        .child(h_flex().justify_end().child(
                            Button::new("fanta-prototype-link-close", "Close").on_click(
                                move |_, _, cx| {
                                    dismiss_view
                                        .update(cx, |view, cx| {
                                            view.prototype_link_notice = None;
                                            cx.notify();
                                        })
                                        .log_err();
                                },
                            ),
                        ))
                        .into_any_element(),
                };
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(gpui::black().opacity(0.45))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            v_flex()
                                .w(px(420.))
                                .max_w_full()
                                .p_4()
                                .rounded_lg()
                                .border_1()
                                .border_color(chrome_border)
                                .bg(chrome_background)
                                .shadow_lg()
                                .child(content),
                        ),
                )
            })
            .into_any_element()
    }
}

#[cfg(target_os = "macos")]
impl FigView {
    fn selected_canvas_video_source(&self, cx: &App) -> Option<(CanvasVideoSource, bool, f32)> {
        let document = self.item.read(cx).document()?;
        let node = single_selection(&document.doc)?;
        if !crate::clipboard::node_is_on_active_page(&document.doc, node) {
            return None;
        }
        let NodeData::Video(video) = &document.doc.scene.get(node)?.data else {
            return None;
        };
        Some((
            CanvasVideoSource {
                scene: document.doc.scene.instance_id(),
                node,
                asset: video.asset,
                assets_identity: std::sync::Arc::as_ptr(&document.raw_assets) as usize,
                bytes_identity: document
                    .raw_assets
                    .get(&video.asset)
                    .map(|bytes| (bytes.as_ptr() as usize, bytes.len())),
                time_range_us: video.time_range_us,
                speed_bits: video.speed.to_bits(),
            },
            video.muted,
            video.volume,
        ))
    }

    fn clear_canvas_video(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.canvas_video.take() {
            if let Some(playback) = session.playback {
                playback.update(cx, |playback, cx| playback.close(cx));
            }
            self.canvas_video_generation = self.canvas_video_generation.wrapping_add(1);
            self.rendered_canvas = None;
            cx.notify();
        }
    }

    fn set_canvas_video_active(&self, active: bool, cx: &mut Context<Self>) {
        self.canvas_video_active.set(active);
        if !active {
            self.cancel_pending_video_trim();
        }
        if let Some(playback) = self
            .canvas_video
            .as_ref()
            .and_then(|session| session.playback.as_ref())
        {
            playback.update(cx, |playback, cx| playback.set_active(active, cx));
        }
    }

    fn sync_canvas_video(&mut self, window_active: bool, cx: &mut Context<Self>) {
        if self.canvas_video_removed.get()
            || self.prototype_player.is_some()
            || self.editor_workspace(cx) != EditorWorkspace::Canvas
        {
            self.set_canvas_video_active(false, cx);
            return;
        }
        let source = self.selected_canvas_video_source(cx);
        if self.canvas_video.as_ref().map(|session| session.source) != source.map(|source| source.0)
        {
            self.clear_canvas_video(cx);
        }
        let Some((source, muted, volume)) = source else {
            return;
        };
        self.canvas_video_active.set(window_active);
        if let Some(session) = self.canvas_video.as_mut() {
            if session.audio != (muted, volume.to_bits()) {
                session.audio = (muted, volume.to_bits());
                if let Some(playback) = session.playback.as_ref() {
                    playback.update(cx, |playback, cx| playback.set_audio(muted, volume, cx));
                }
            }
            self.set_canvas_video_active(window_active, cx);
            return;
        }
        let unsupported = source.time_range_us[0] < 0
            || source.time_range_us[1] <= source.time_range_us[0]
            || source.speed_bits != 1_f32.to_bits();
        self.canvas_video_generation = self.canvas_video_generation.wrapping_add(1);
        let generation = self.canvas_video_generation;
        self.canvas_video = Some(CanvasVideoSession {
            source,
            loading: None,
            playback: None,
            observation: None,
            error: unsupported.then(|| "Inline playback and trimming support normal-speed video with a valid source range.".into()),
            audio: (muted, volume.to_bits()),
            bytes: None,
            trim: None,
        });
        if unsupported {
            return;
        }
        let Some(document) = self.item.read(cx).document() else {
            return;
        };
        let raw_assets = document.raw_assets.clone();
        let resolver = document.asset_resolver.clone();
        // Newly placed videos live in raw_assets before the load-time resolver
        // knows about them. Copy the bounded source on a worker, not each paint.
        let loading = cx.background_spawn(async move {
            let resolved;
            let bytes = if let Some(bytes) = raw_assets.get(&source.asset) {
                bytes.as_slice()
            } else {
                resolved = resolver
                    .as_ref()
                    .and_then(|resolver| resolver.resolve_bytes(source.asset))
                    .context("The video source is missing from this project.")?;
                resolved.as_slice()
            };
            anyhow::ensure!(
                !bytes.is_empty() && bytes.len() <= 100 * 1024 * 1024,
                "The video must be nonempty and no larger than 100 MiB."
            );
            Ok(std::sync::Arc::<[u8]>::from(bytes))
        });
        let task = cx.spawn(async move |this, cx| {
            let result = loading.await;
            if let Err(error) = this.update(cx, |this, cx| {
                this.finish_canvas_video_load(source, generation, result, cx);
            }) {
                log::debug!("Canvas video owner was released: {error}");
            }
        });
        if let Some(session) = self.canvas_video.as_mut() {
            session.loading = Some(task);
        }
    }

    fn finish_canvas_video_load(
        &mut self,
        source: CanvasVideoSource,
        generation: u64,
        result: Result<std::sync::Arc<[u8]>>,
        cx: &mut Context<Self>,
    ) {
        if self.canvas_video_removed.get()
            || self.canvas_video_generation != generation
            || self.selected_canvas_video_source(cx).map(|source| source.0) != Some(source)
            || self.canvas_video.as_ref().map(|session| session.source) != Some(source)
        {
            return;
        }
        let playback = match result {
            Ok(bytes) => {
                if let Some(session) = self.canvas_video.as_mut() {
                    session.bytes = Some(bytes.clone());
                }
                Some(cx.new(|cx| {
                    VideoPlaybackView::new_with_range(
                        bytes,
                        2048,
                        [
                            source.time_range_us[0] as u64,
                            source.time_range_us[1] as u64,
                        ],
                        cx,
                    )
                }))
            }
            Err(error) => {
                if let Some(session) = self.canvas_video.as_mut() {
                    session.loading = None;
                    session.error = Some(format!("Could not load video: {error:#}").into());
                }
                cx.notify();
                None
            }
        };
        let Some(playback) = playback else {
            return;
        };
        playback.update(cx, |playback, cx| {
            playback.set_active(
                self.canvas_video_active.get()
                    && self.editor_workspace(cx) == EditorWorkspace::Canvas
                    && self.prototype_player.is_none(),
                cx,
            )
        });
        let observation = cx.observe(&playback, |_, _, cx| cx.notify());
        if let Some(session) = self.canvas_video.as_mut() {
            let (muted, volume) = session.audio;
            playback.update(cx, |playback, cx| {
                playback.set_audio(muted, f32::from_bits(volume), cx)
            });
            session.loading = None;
            session.playback = Some(playback);
            session.observation = Some(observation);
            cx.notify();
        }
    }

    pub(crate) fn canvas_video_frame(&self, cx: &App) -> Option<CanvasVideoFrame> {
        let session = self.canvas_video.as_ref()?;
        if self.canvas_video_removed.get()
            || self.selected_canvas_video_source(cx).map(|source| source.0) != Some(session.source)
        {
            return None;
        }
        let playback = session.playback.as_ref()?.read(cx);
        let status = playback.status();
        if !canvas_video_duration_supported(
            session.source.time_range_us,
            playback.source_duration_us(),
        ) {
            return None;
        }
        Some(CanvasVideoFrame {
            node_id: session.source.node,
            buffer: playback.frame()?,
            progress: status.current_time_us as f32 / status.duration_us as f32,
            revision: playback.frame_revision(),
            session_revision: self.canvas_video_generation,
        })
    }

    fn cancel_pending_video_trim(&self) {
        if let Some(trim) = self
            .canvas_video
            .as_ref()
            .and_then(|session| session.trim.as_ref())
        {
            trim.cancelled.set(true);
            trim.task.borrow_mut().take();
        }
    }

    fn begin_video_trim(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_editable(cx) || self.canvas_video_removed.get() {
            return;
        }
        self.finish_document_edits(cx);
        let Some(session) = self.canvas_video.as_ref() else {
            return;
        };
        let Some(playback) = session.playback.clone() else {
            return;
        };
        let duration_us = playback.read(cx).source_duration_us();
        if duration_us == 0
            || session.bytes.is_none()
            || session.source.speed_bits != 1_f32.to_bits()
        {
            return;
        }
        let source = session.source;
        let Some(NodeData::Video(expected)) = self
            .item
            .read(cx)
            .document()
            .and_then(|document| document.doc.scene.get(source.node))
            .map(|node| &node.data)
        else {
            return;
        };
        let expected = expected.clone();
        let start = cx.new(|cx| {
            ui_input::InputField::new(window, cx, "0")
                .label("Start (seconds)")
                .label_min_width(px(0.))
        });
        let end = cx.new(|cx| {
            ui_input::InputField::new(window, cx, "0")
                .label("End (seconds)")
                .label_min_width(px(0.))
        });
        start.update(cx, |input, cx| {
            input.set_text(&video_trim_time_text(expected.time_range_us[0]), window, cx)
        });
        end.update(cx, |input, cx| {
            input.set_text(&video_trim_time_text(expected.time_range_us[1]), window, cx)
        });
        playback.update(cx, |playback, cx| playback.pause(cx));
        self.canvas_video_generation = self.canvas_video_generation.wrapping_add(1);
        if let Some(session) = self.canvas_video.as_mut() {
            session.trim = Some(CanvasVideoTrim {
                source,
                expected,
                start,
                end,
                duration_us,
                task: Default::default(),
                cancelled: std::cell::Cell::new(false),
                error: None,
            });
        }
        cx.notify();
    }

    fn cancel_video_trim(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.canvas_video.as_mut() {
            session.trim = None;
        }
        cx.notify();
    }

    fn apply_video_trim(&mut self, cx: &mut Context<Self>) {
        self.apply_video_trim_with(
            |bytes, range| Box::pin(crate::generation_media::prepare_video_trim(bytes, range)),
            cx,
        );
    }

    fn apply_video_trim_with(
        &mut self,
        prepare: impl FnOnce(
            std::sync::Arc<[u8]>,
            [i64; 2],
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<crate::generation_media::PreparedVideoTrim>>
                    + Send,
            >,
        >,
        cx: &mut Context<Self>,
    ) {
        if !self.is_editable(cx)
            || self.canvas_video_removed.get()
            || !self.canvas_video_active.get()
        {
            return;
        }
        self.finish_document_edits(cx);
        let Some(session) = self.canvas_video.as_ref() else {
            return;
        };
        let Some(trim) = session.trim.as_ref() else {
            return;
        };
        if trim.task.borrow().is_some() {
            return;
        }
        let parsed = (|| {
            let range = [
                parse_video_trim_time(&trim.start.read(cx).text(cx))?,
                parse_video_trim_time(&trim.end.read(cx).text(cx))?,
            ];
            crate::generation_media::validate_video_trim(range, i64::try_from(trim.duration_us)?)?;
            Ok::<_, anyhow::Error>(range)
        })();
        let range = match parsed {
            Ok(range) => range,
            Err(error) => {
                if let Some(trim) = self
                    .canvas_video
                    .as_mut()
                    .and_then(|session| session.trim.as_mut())
                {
                    trim.error = Some(format!("{error:#}").into());
                }
                cx.notify();
                return;
            }
        };
        if range == trim.expected.time_range_us {
            self.cancel_video_trim(cx);
            return;
        }
        let Some(bytes) = session.bytes.clone() else {
            return;
        };
        let source = trim.source;
        let generation = self.canvas_video_generation;
        let expected = self
            .item
            .read(cx)
            .document()
            .and_then(|document| document.doc.scene.get(source.node))
            .and_then(|node| match &node.data {
                NodeData::Video(video) => Some(video.clone()),
                _ => None,
            });
        let Some(expected) = expected else { return };
        if let Some(trim) = self
            .canvas_video
            .as_mut()
            .and_then(|session| session.trim.as_mut())
        {
            trim.expected = expected;
            trim.cancelled.set(false);
        }
        let work = cx.background_spawn(prepare(bytes, range));
        let timer = cx
            .background_executor()
            .timer(std::time::Duration::from_secs(20));
        let task = cx.spawn(async move |this, cx| {
            let result = match futures::future::select(work, timer).await {
                futures::future::Either::Left((result, _)) => result,
                futures::future::Either::Right(_) => Err(anyhow::anyhow!(
                    "Preparing the trim preview took too long. The video was not changed."
                )),
            };
            if let Err(error) = this.update(cx, |this, cx| {
                this.finish_video_trim(source, generation, result, cx)
            }) {
                log::debug!("Video trim owner was released: {error}");
            }
        });
        if let Some(trim) = self
            .canvas_video
            .as_mut()
            .and_then(|session| session.trim.as_mut())
        {
            trim.error = None;
            *trim.task.borrow_mut() = Some(task);
        }
        cx.notify();
    }

    fn finish_video_trim(
        &mut self,
        source: CanvasVideoSource,
        generation: u64,
        result: Result<crate::generation_media::PreparedVideoTrim>,
        cx: &mut Context<Self>,
    ) {
        if self.canvas_video_removed.get()
            || !self.canvas_video_active.get()
            || generation != self.canvas_video_generation
            || self.selected_canvas_video_source(cx).map(|source| source.0) != Some(source)
            || self.editor_workspace(cx) != EditorWorkspace::Canvas
        {
            return;
        }
        let Some(trim) = self
            .canvas_video
            .as_ref()
            .and_then(|session| session.trim.as_ref())
        else {
            return;
        };
        if trim.cancelled.get() || trim.source != source {
            return;
        }
        trim.task.borrow_mut().take();
        let expected = trim.expected.clone();
        let requested_range = (|| {
            Ok::<_, anyhow::Error>([
                parse_video_trim_time(&trim.start.read(cx).text(cx))?,
                parse_video_trim_time(&trim.end.read(cx).text(cx))?,
            ])
        })();
        let result = result.and_then(|prepared| {
            anyhow::ensure!(
                requested_range? == prepared.range_us,
                "The trim times changed while the preview was loading. Apply the trim again."
            );
            anyhow::ensure!(
                self.is_editable(cx),
                "This document is currently read-only."
            );
            self.item.update(cx, |item, cx| {
                item.with_document(cx, |document| {
                    crate::generation_media::trim_video(document, source.node, &expected, prepared)
                })
                .context("The video document was closed.")?
            })
        });
        match result {
            Ok(()) => {
                self.clear_canvas_video(cx);
                self.invalidate_canvas_cache();
            }
            Err(error) => {
                if let Some(trim) = self
                    .canvas_video
                    .as_mut()
                    .and_then(|session| session.trim.as_mut())
                {
                    trim.error = Some(format!("Could not trim video: {error:#}").into());
                }
            }
        }
        cx.notify();
    }

    fn render_video_trim_controls(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let session = self.canvas_video.as_ref()?;
        if let Some(trim) = session.trim.as_ref() {
            let busy = trim.task.borrow().is_some();
            let error = trim.error.clone();
            return Some(
                v_flex()
                    .gap_2()
                    .w_full()
                    .min_w_0()
                    .child(
                        Label::new(format!(
                            "Trim original video · {} seconds",
                            video_trim_time_text(trim.duration_us as i64)
                        ))
                        .color(Color::Muted),
                    )
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_2()
                            .w_full()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(120.))
                                    .debug_selector(|| "video-trim-start".to_owned())
                                    .child(trim.start.clone()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(120.))
                                    .debug_selector(|| "video-trim-end".to_owned())
                                    .child(trim.end.clone()),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(
                                Button::new(
                                    "video-trim-apply",
                                    if busy {
                                        "Preparing preview…"
                                    } else {
                                        "Apply trim"
                                    },
                                )
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| this.apply_video_trim(cx))),
                            )
                            .child(
                                div()
                                    .debug_selector(|| "video-trim-cancel-target".to_owned())
                                    .child(Button::new("video-trim-cancel", "Cancel").on_click(
                                        cx.listener(|this, _, _, cx| this.cancel_video_trim(cx)),
                                    )),
                            ),
                    )
                    .when_some(error, |element, error| {
                        element.child(Label::new(error).color(Color::Error))
                    })
                    .into_any_element(),
            );
        }
        let enabled = self.is_editable(cx)
            && session.bytes.is_some()
            && session.source.speed_bits == 1_f32.to_bits()
            && session.playback.as_ref().is_some_and(|playback| {
                playback.read(cx).source_duration_us() > 0 && playback.read(cx).error().is_none()
            });
        Some(
            div()
                .debug_selector(|| "video-trim-open-target".to_owned())
                .child(
                    Button::new("video-trim", "Trim video")
                        .disabled(!enabled)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.begin_video_trim(window, cx)),
                        ),
                )
                .into_any_element(),
        )
    }

    fn render_canvas_video_controls(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let session = self.canvas_video.as_ref()?;
        let playback = session.playback.clone();
        let mut error = session.error.clone();
        let range = session.source.time_range_us;
        let mut controls = None;
        if let Some(playback) = playback {
            let rendered = playback.update(cx, |playback, cx| playback.render_controls(window, cx));
            let duration = playback.read(cx).source_duration_us();
            if duration > 0 && !canvas_video_duration_supported(range, duration) {
                playback.update(cx, |playback, cx| playback.close(cx));
                error =
                    Some("This video range extends beyond the original source duration.".into());
                if let Some(session) = self.canvas_video.as_mut() {
                    session.error = error.clone();
                    session.playback = None;
                    session.observation = None;
                }
            } else {
                controls = Some(rendered);
            }
        }
        let trim_controls = self.render_video_trim_controls(window, cx);
        Some(
            v_flex()
                .id("canvas-video-controls")
                .debug_selector(|| "canvas-video-controls".to_owned())
                .w_full()
                .flex_none()
                .min_w_0()
                .p_2()
                .gap_1()
                .border_t_1()
                .border_color(cx.theme().colors().border)
                .bg(cx.theme().colors().panel_background)
                .children(controls)
                .children(trim_controls)
                .when_some(error, |element, error| {
                    element.child(Label::new(error).color(Color::Error))
                })
                .when(
                    self.canvas_video
                        .as_ref()
                        .is_some_and(|session| session.loading.is_some()),
                    |element| element.child(Label::new("Loading video…").color(Color::Muted)),
                )
                .into_any_element(),
        )
    }
}

#[cfg(target_os = "macos")]
fn parse_video_trim_time(text: &str) -> Result<i64> {
    let (seconds, fraction) = text.trim().split_once('.').unwrap_or((text.trim(), ""));
    anyhow::ensure!(
        !seconds.is_empty()
            && seconds.bytes().all(|byte| byte.is_ascii_digit())
            && fraction.len() <= 6
            && fraction.bytes().all(|byte| byte.is_ascii_digit()),
        "Enter seconds with up to six decimal places, such as 1.25."
    );
    let seconds: i64 = seconds.parse().context("The time is too large.")?;
    let fraction: i64 = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<i64>()? * 10_i64.pow(6 - fraction.len() as u32)
    };
    seconds
        .checked_mul(1_000_000)
        .and_then(|time| time.checked_add(fraction))
        .context("The time is too large.")
}

#[cfg(target_os = "macos")]
fn video_trim_time_text(time: i64) -> String {
    if time % 1_000_000 == 0 {
        return (time / 1_000_000).to_string();
    }
    format!("{}.{:06}", time / 1_000_000, time % 1_000_000)
        .trim_end_matches('0')
        .to_owned()
}

#[cfg(target_os = "macos")]
fn canvas_video_duration_supported(range: [i64; 2], duration: u64) -> bool {
    range[0] >= 0
        && range[0] < range[1]
        && duration > 0
        && u64::try_from(range[1]).is_ok_and(|end| end <= duration)
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
        let editor_mode = self.editor_mode(cx);
        let editor_workspace = self.editor_workspace(cx);
        #[cfg(target_os = "macos")]
        self.sync_canvas_video(window.is_window_active(), cx);
        #[cfg(feature = "fanta-gpui-ui")]
        {
            let tool = self.tools.kind();
            let zoom_percent = self.current_zoom_percent(cx);
            let options = self.toolbar_option_inputs(window, cx);
            if let Some(adapter) = self.gpui_toolbar.as_mut() {
                adapter.refresh(editor_mode, tool, zoom_percent, options, cx);
            }
            // Covers state that was already ready before the first item
            // event (a preloaded document); memoized, so later frames skip.
            self.refresh_gpui_design(cx);
        }
        let cursor_style = match &self.text_edit {
            // The I-beam over the edited text, an arrow elsewhere — clicking
            // away commits.
            Some(edit) if edit.session.pointer_inside => CursorStyle::IBeam,
            Some(_) => CursorStyle::Arrow,
            // Space-hold pan shows the grab/grabbing hand over any tool.
            None if self.space_pan && self.is_panning() => CursorStyle::ClosedHand,
            None if self.space_pan => CursorStyle::OpenHand,
            // A directional cursor over a selection handle, Figma-style. The
            // handle was resolved on the last hover move, so this is a field
            // read: `render` runs on every window redraw, hit-testing would
            // not belong here.
            None => match self.hover_resize_handle {
                Some(handle) if self.prototype_player.is_none() => resize_cursor(handle),
                _ => self.tools.cursor_style(self.is_panning()),
            },
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
            } else if self.prototype_player.is_some() {
                "FigViewerPrototype"
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
            })
            // The space key-UP must be seen even if a text session opened (or
            // any other state change swapped listeners) while the key was held.
            // Gating it like the key-down left `space_pan` stuck when the
            // release landed elsewhere — the Select tool then panned with a
            // hand cursor until the user pressed space again.
            .on_key_up(cx.listener(Self::handle_canvas_key_up))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::reset_zoom))
            .on_action(cx.listener(Self::fit_to_view))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::delete_selection))
            .on_action(cx.listener(Self::copy_selection))
            .on_action(cx.listener(Self::cut_selection))
            .on_action(cx.listener(Self::paste_selection))
            .on_action(cx.listener(Self::duplicate_selection))
            .on_action(cx.listener(Self::group_selection))
            .on_action(cx.listener(Self::ungroup_selection))
            .on_action(cx.listener(Self::frame_selection))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::zoom_to_selection_action))
            // The application Edit menu dispatches `editor::actions::*`, which
            // nothing on the canvas would otherwise answer. Each forwarder
            // stands aside while the inline text session is live: that session
            // is a `CanvasTextEdit`, not an `Editor`, so without the guard
            // Edit > Undo mid-typing would commit and close the session and
            // then undo an unrelated node operation.
            .on_action(cx.listener(|this, _: &editor::actions::Undo, window, cx| {
                if this.text_edit.is_some() {
                    cx.propagate();
                    return;
                }
                this.undo(&Undo, window, cx);
            }))
            .on_action(cx.listener(|this, _: &editor::actions::Redo, window, cx| {
                if this.text_edit.is_some() {
                    cx.propagate();
                    return;
                }
                this.redo(&Redo, window, cx);
            }))
            .on_action(cx.listener(|this, _: &editor::actions::Cut, window, cx| {
                if this.text_edit.is_some() {
                    cx.propagate();
                    return;
                }
                this.cut_selection(&CutSelection, window, cx);
            }))
            .on_action(cx.listener(|this, _: &editor::actions::Copy, window, cx| {
                if this.text_edit.is_some() {
                    cx.propagate();
                    return;
                }
                this.copy_selection(&CopySelection, window, cx);
            }))
            .on_action(cx.listener(|this, _: &editor::actions::Paste, window, cx| {
                if this.text_edit.is_some() {
                    cx.propagate();
                    return;
                }
                this.paste_selection(&PasteSelection, window, cx);
            }))
            .on_action(
                cx.listener(|this, _: &editor::actions::SelectAll, window, cx| {
                    if this.text_edit.is_some() {
                        cx.propagate();
                        return;
                    }
                    this.select_all(&SelectAll, window, cx);
                }),
            )
            .on_action(cx.listener(Self::play_prototype))
            .on_action(cx.listener(Self::exit_prototype))
            .on_action(cx.listener(Self::restart_prototype))
            .on_action(cx.listener(Self::prototype_previous_frame))
            .on_action(cx.listener(Self::prototype_next_frame))
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
            .on_action(cx.listener(|this, _: &ActivateCommentTool, _, cx| {
                this.activate_tool(ToolKind::Comment, cx)
            }))
            .on_action(cx.listener(|this, _: &ActivateTextPathTool, _, cx| {
                this.activate_tool(ToolKind::TextPath, cx)
            }))
            .on_action(cx.listener(Self::toggle_layers_sidebar))
            .on_action(cx.listener(Self::toggle_inspector_sidebar))
            .on_drag_move::<SidebarResizeDrag>(cx.listener(Self::handle_sidebar_resize_drag))
            .on_drop(cx.listener(|this, _: &SidebarResizeDrag, _, cx| {
                this.persist_sidebar_layout(cx);
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
                        // `{:#}` prints the whole context chain, so the real
                        // loader failure (auth, parse, missing file) is shown
                        // instead of just the outermost wrapper.
                        .child(Label::new(format!("{error:#}")).color(Color::Muted)),
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
                let presenting_prototype = self.prototype_player.is_some();
                let workspace_body = if presenting_prototype {
                    self.render_prototype_presentation(cx)
                } else if editor_workspace == EditorWorkspace::Variables {
                    div()
                        .id("fanta-variables-workspace-body")
                        .size_full()
                        .overflow_hidden()
                        .child(self.variables_workspace.clone())
                        .into_any_element()
                } else if editor_workspace == EditorWorkspace::Code {
                    div()
                        .id("fanta-code-workspace-body")
                        .size_full()
                        .overflow_hidden()
                        .child(self.code_workspace.clone())
                        .into_any_element()
                } else {
                    div()
                        .id("fanta-canvas-workspace-body")
                        .size_full()
                        .relative()
                        .overflow_hidden()
                        .child(
                            v_flex()
                                .size_full()
                                .overflow_hidden()
                                .child(
                                    h_flex()
                                        .flex_1()
                                        .min_h_0()
                                        .w_full()
                                        .overflow_hidden()
                                        .children(
                                            self.layers_sidebar_visible
                                                .then(|| self.render_layers_sidebar(cx)),
                                        )
                                        .child(
                                            v_flex()
                                                .flex_1()
                                                .min_w_0()
                                                .h_full()
                                                .overflow_hidden()
                                                .child(
                                                    div()
                                                        .id("fig-container")
                                                        // Release no-op; the
                                                        // toolbar occlusion
                                                        // test probes it.
                                                        .debug_selector(|| {
                                                            "fig-container".to_owned()
                                                        })
                                                        .flex_1()
                                                        .min_h_0()
                                                        .w_full()
                                                        .overflow_hidden()
                                                        .relative()
                                                        .cursor(cursor_style)
                                                        .on_scroll_wheel(
                                                            cx.listener(Self::handle_scroll_wheel),
                                                        )
                                                        .on_pinch(cx.listener(Self::handle_pinch))
                                                        .on_mouse_down(
                                                            MouseButton::Left,
                                                            cx.listener(Self::handle_mouse_down),
                                                        )
                                                        .on_mouse_down(
                                                            MouseButton::Middle,
                                                            cx.listener(Self::handle_mouse_down),
                                                        )
                                                        .on_mouse_up(
                                                            MouseButton::Left,
                                                            cx.listener(Self::handle_mouse_up),
                                                        )
                                                        .on_mouse_up(
                                                            MouseButton::Middle,
                                                            cx.listener(Self::handle_mouse_up),
                                                        )
                                                        .on_mouse_move(
                                                            cx.listener(Self::handle_mouse_move),
                                                        )
                                                        .children({
                                                            // Paint order = z-order: the rendered scene
                                                            // surface goes on the BOTTOM, and the text-edit
                                                            // overlay (caret bar + selection highlight) on
                                                            // TOP — otherwise the opaque canvas covers the
                                                            // caret/selection and they never show.
                                                            let mut c: Vec<AnyElement> = vec![];
                                                            c.push(
                                                                CanvasElement::new(cx.entity())
                                                                    .into_any_element(),
                                                            );
                                                            if let Some(ov) =
                                                                self.render_text_edit_overlay(cx)
                                                            {
                                                                c.push(ov);
                                                            }
                                                            if let Some(comments) =
                                                                self.render_comment_overlay(cx)
                                                            {
                                                                c.push(comments);
                                                            }
                                                            if editor_mode == EditorMode::Prototype
                                                            {
                                                                let play_button = self
                                                                    .render_prototype_play_button(
                                                                        cx,
                                                                    );
                                                                c.push(play_button);
                                                            }
                                                            // A loading or
                                                            // failed document
                                                            // is not an empty
                                                            // page, and a
                                                            // running
                                                            // prototype is not
                                                            // an invitation to
                                                            // draw.
                                                            if !is_loading
                                                                && !has_error
                                                                && self.prototype_player.is_none()
                                                                && self.active_page_is_empty(cx)
                                                            {
                                                                c.push(render_empty_page_hint(cx));
                                                            }
                                                            c
                                                        })
                                                        .child(self.render_toolbar_slot(cx)),
                                                )
                                                .children({
                                                    #[cfg(target_os = "macos")]
                                                    {
                                                        self.render_canvas_video_controls(
                                                            window, cx,
                                                        )
                                                    }
                                                    #[cfg(not(target_os = "macos"))]
                                                    {
                                                        None::<AnyElement>
                                                    }
                                                }),
                                        )
                                        .children(
                                            self.inspector_sidebar_visible
                                                .then(|| self.render_inspector_sidebar(cx)),
                                        ),
                                )
                                .children(
                                    (editor_mode == EditorMode::Motion)
                                        .then(|| self.timeline_shell.clone()),
                                ),
                        )
                        .children(
                            self.item
                                .read(cx)
                                .source_edit_locked()
                                .then(|| self.render_source_edit_lock_banner(cx)),
                        )
                        .into_any_element()
                };
                this.child(workspace_body)
                    .children((!presenting_prototype).then(|| self.render_workspace_tabs(cx)))
            })
    }
}

fn single_selection(doc: &fanta_doc::Doc) -> Option<NodeId> {
    let mut selection = doc.selection.iter().copied();
    let node = selection.next()?;
    selection.next().is_none().then_some(node)
}

/// The layers a structural command (group, frame, ungroup) acts on: the
/// selection, unless the command came from a layer row that is not part of
/// it, in which case only that row's node. Scoped navigation can retain the
/// selection, so layers left behind on another page are dropped here — a
/// group built from them would land out of view, or pull them onto this page.
fn structure_targets(doc: &Doc, clicked: Option<NodeId>) -> Vec<NodeId> {
    let candidates = match clicked {
        Some(clicked) if !doc.selection.contains(clicked) => vec![clicked],
        _ => doc.selection.as_slice().to_vec(),
    };
    candidates
        .into_iter()
        .filter(|id| crate::clipboard::node_is_on_active_page(doc, *id))
        .collect()
}

/// The structure targets ungrouping dissolves: groups and frames, skipping
/// page roots, component masters (Figma leaves those intact), and targets
/// nested in another target, whose parent's ungroup already moves them.
fn ungroupable_targets(doc: &Doc, targets: &[NodeId]) -> Vec<NodeId> {
    let requested = targets.iter().copied().collect::<HashSet<_>>();
    targets
        .iter()
        .copied()
        .filter(|id| {
            matches!(
                doc.scene.get(*id).map(|node| &node.data),
                Some(NodeData::Group(_))
            ) && !doc.pages().contains(id)
                && !doc.is_component_root(*id)
                && !doc
                    .scene
                    .ancestors_of(*id)
                    .any(|ancestor| requested.contains(&ancestor.id))
        })
        .collect()
}

fn is_pastable_image_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tiff" | "tif"
            )
        })
}

/// Read an image file for pasting, refusing one over the asset cap before its
/// bytes are read or decoded.
fn read_pastable_image_file(path: &std::path::Path) -> Result<Vec<u8>> {
    let length = std::fs::metadata(path)
        .with_context(|| format!("reading {}", path.display()))?
        .len();
    if usize::try_from(length)
        .ok()
        .is_none_or(|length| length > MAX_IMAGE_SOURCE_BYTES)
    {
        anyhow::bail!(
            "{} is {length} bytes; the limit is {MAX_IMAGE_SOURCE_BYTES} bytes",
            path.display()
        );
    }
    std::fs::read(path).with_context(|| format!("reading {}", path.display()))
}

/// What a paste of several images produced.
struct PastedImages {
    placed: Vec<NodeId>,
    /// Images that could not be ingested (undecodable, or over the asset
    /// cap); they are skipped rather than failing their neighbours.
    skipped: Vec<anyhow::Error>,
}

/// Ingest `images` as assets, then create one bitmap layer per image on the
/// active page in a single transaction: each centred on `center` (world) at
/// natural size, shrunk to fit 80% of the `visible` world-space viewport when
/// larger, and cascaded by 16 world units so a batch stays distinguishable.
/// Every ingested asset is dropped again when the layers cannot be created,
/// so a failed paste leaves no orphan bytes.
fn paste_images(
    doc: &mut Doc,
    assets: &mut AssetStores<'_>,
    images: Vec<Vec<u8>>,
    center: [f64; 2],
    visible: [f64; 2],
) -> Result<PastedImages> {
    let mut ingested = Vec::new();
    let mut skipped = Vec::new();
    for bytes in images {
        match assets.add_image(bytes) {
            Ok(image) => ingested.push(image),
            Err(error) => skipped.push(error),
        }
    }
    match place_ingested_images(doc, &ingested, center, visible) {
        Ok(placed) => Ok(PastedImages { placed, skipped }),
        Err(error) => {
            for (asset, _) in ingested {
                assets.remove(asset);
            }
            Err(error)
        }
    }
}

fn place_ingested_images(
    doc: &mut Doc,
    ingested: &[(AssetId, [u32; 2])],
    center: [f64; 2],
    visible: [f64; 2],
) -> Result<Vec<NodeId>> {
    let mut nodes: Vec<CanvasNode> = Vec::with_capacity(ingested.len());
    for (position, (asset, natural_size)) in ingested.iter().copied().enumerate() {
        let natural = [
            f64::from(natural_size[0].max(1)),
            f64::from(natural_size[1].max(1)),
        ];
        let mut fit = 1.0_f64;
        for axis in 0..2 {
            if visible[axis].is_finite() && visible[axis] > 0.0 {
                fit = fit.min(visible[axis] * 0.8 / natural[axis]);
            }
        }
        let size = [natural[0] * fit, natural[1] * fit];
        let offset = position as f64 * 16.0;
        let x = center[0] - size[0] * 0.5 + offset;
        let y = center[1] - size[1] * 0.5 + offset;
        let mut node =
            crate::structure::image_layer_node(doc, asset, natural_size, size, None, x, y, None)?;
        // `image_layer_node` mints its key from the scene, which does not see
        // the layers built earlier in this loop until the transaction applies,
        // so later images stack above the ones before them explicitly.
        if let Some(previous) = nodes.last() {
            node.index = IndexKey::after(previous.index);
        }
        nodes.push(node);
    }
    if nodes.is_empty() {
        return Ok(Vec::new());
    }
    let placed: Vec<NodeId> = nodes.iter().map(|node| node.id).collect();
    let label = if nodes.len() == 1 {
        "Paste image"
    } else {
        "Paste images"
    };
    let operations = nodes.into_iter().map(Operation::create_node).collect();
    apply_canvas_transaction(doc, label, operations)?;
    Ok(placed)
}

fn motion_property(property: TimelineProperty) -> MotionProperty {
    match property {
        TimelineProperty::PositionX => MotionProperty::PositionX,
        TimelineProperty::PositionY => MotionProperty::PositionY,
        TimelineProperty::Rotation => MotionProperty::Rotation,
        TimelineProperty::ScaleX => MotionProperty::ScaleX,
        TimelineProperty::ScaleY => MotionProperty::ScaleY,
        TimelineProperty::Opacity => MotionProperty::bound(BoundProp::Opacity),
        TimelineProperty::FillColor => MotionProperty::bound(BoundProp::FillColor { index: 0 }),
    }
}

fn motion_value(
    node: &fanta_doc::CanvasNode,
    property: MotionProperty,
) -> Option<ResolvedVarValue> {
    match property {
        MotionProperty::Bound { prop } => prop.read_resolved(node),
        MotionProperty::PositionX
        | MotionProperty::PositionY
        | MotionProperty::Rotation
        | MotionProperty::ScaleX
        | MotionProperty::ScaleY => {
            let transform = MotionTransform::decompose(node.transform)?;
            let value = match property {
                MotionProperty::PositionX => transform.position[0],
                MotionProperty::PositionY => transform.position[1],
                MotionProperty::Rotation => transform.rotation_radians,
                MotionProperty::ScaleX => transform.scale[0],
                MotionProperty::ScaleY => transform.scale[1],
                MotionProperty::Bound { .. } => return None,
            };
            Some(ResolvedVarValue::Float { value })
        }
    }
}

fn motion_source_node(doc: &fanta_doc::Doc, node_id: NodeId) -> Option<fanta_doc::CanvasNode> {
    let mut node = doc.scene.get(node_id)?.clone();
    for (property, variable) in node.bindings.clone() {
        if let Some(value) = fanta_doc::resolve_bound_value(
            &doc.variables,
            &doc.scene,
            node_id,
            &doc.active_modes,
            variable,
        ) {
            property.apply_resolved(&mut node, value);
        }
    }
    Some(node)
}

fn motion_property_label(property: MotionProperty) -> &'static str {
    match property {
        MotionProperty::PositionX => "Position X",
        MotionProperty::PositionY => "Position Y",
        MotionProperty::Rotation => "Rotation",
        MotionProperty::ScaleX => "Scale X",
        MotionProperty::ScaleY => "Scale Y",
        MotionProperty::Bound {
            prop: BoundProp::Opacity,
        } => "Opacity",
        MotionProperty::Bound {
            prop: BoundProp::FillColor { .. },
        } => "Fill color",
        MotionProperty::Bound { .. } => "Property",
    }
}

fn motion_timeline_model(
    doc: &fanta_doc::Doc,
    clip_id: Option<AnimationClipId>,
) -> TimelineViewModel {
    let Some(clip) = clip_id.and_then(|clip| doc.motion.clip(clip)) else {
        return TimelineViewModel::empty();
    };
    let mut tracks = clip
        .tracks
        .values()
        .map(|track| {
            let node_name = doc
                .scene
                .get(track.target.node)
                .map(|node| node.name.as_str())
                .filter(|name| !name.is_empty())
                .unwrap_or("Layer");
            let mut keyframes = track
                .keyframes
                .values()
                .map(|keyframe| TimelineKeyframeViewModel {
                    id: keyframe.id.to_string().into(),
                    time_us: i64::from(keyframe.time_ms) * 1_000,
                    interpolation: keyframe.interpolation,
                    easing: keyframe.easing,
                })
                .collect::<Vec<_>>();
            keyframes.sort_by(|left, right| {
                left.time_us
                    .cmp(&right.time_us)
                    .then_with(|| left.id.as_ref().cmp(right.id.as_ref()))
            });
            TimelineTrackViewModel {
                id: track.id.to_string().into(),
                node_id: track.target.node,
                label: format!(
                    "{node_name} · {}",
                    motion_property_label(track.target.property)
                )
                .into(),
                selected: doc.selection.contains(track.target.node),
                keyframes,
            }
        })
        .collect::<Vec<_>>();
    tracks.sort_by(|left, right| {
        left.label
            .as_ref()
            .cmp(right.label.as_ref())
            .then_with(|| left.id.as_ref().cmp(right.id.as_ref()))
    });
    TimelineViewModel::for_clip(
        clip.name.clone(),
        i64::from(clip.duration_ms) * 1_000,
        tracks,
    )
}

impl Item for FigView {
    type Event = FigViewEvent;

    fn added_to_workspace(
        &mut self,
        _: &mut workspace::Workspace,
        _: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        #[cfg(target_os = "macos")]
        if self.canvas_video_removed.replace(false) {
            self.clear_canvas_video(_cx);
        }
    }

    fn deactivated(&mut self, _: &mut Window, _cx: &mut Context<Self>) {
        #[cfg(target_os = "macos")]
        self.set_canvas_video_active(false, _cx);
    }

    fn workspace_deactivated(&mut self, _: &mut Window, _cx: &mut Context<Self>) {
        #[cfg(target_os = "macos")]
        self.set_canvas_video_active(false, _cx);
    }

    fn on_removed(&self, _cx: &mut Context<Self>) {
        #[cfg(target_os = "macos")]
        {
            self.canvas_video_removed.set(true);
            self.cancel_pending_video_trim();
            if let Some(playback) = self
                .canvas_video
                .as_ref()
                .and_then(|session| session.playback.as_ref())
            {
                playback.update(_cx, |playback, cx| playback.close(cx));
            }
        }
    }

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

    fn project_entry_ids(&self, cx: &App) -> SmallVec<[ProjectEntryId; 3]> {
        // The pane dedupes opens by entry. One shared FigItem backs every tab
        // of a project, so the default (the item's own entry — its FIRST-open
        // path) would collapse every scoped open onto that first tab; report
        // the entry this view was actually opened from instead.
        match self.opened_entry_id {
            Some(entry_id) => [entry_id].into_iter().collect(),
            None => {
                let project = self.project.read(cx);
                project
                    .find_project_path(self.item.read(cx).abs_path(), cx)
                    .and_then(|path| project.entry_for_path(&path, cx))
                    .map(|entry| entry.id)
                    .or_else(|| project::ProjectItem::entry_id(self.item.read(cx), cx))
                    .into_iter()
                    .collect()
            }
        }
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
        if self.item.read(cx).has_ready_document() {
            Capability::ReadWrite
        } else {
            Capability::ReadOnly
        }
    }

    fn is_dirty(&self, cx: &App) -> bool {
        self.item.read(cx).is_dirty() || self.code_workspace.read(cx).source_is_dirty(cx)
    }

    fn has_conflict(&self, cx: &App) -> bool {
        self.code_workspace.read(cx).has_source_conflict(cx)
    }

    fn can_save(&self, cx: &App) -> bool {
        self.item.read(cx).has_ready_document()
    }

    fn can_save_as(&self, cx: &App) -> bool {
        self.item.read(cx).has_ready_document() && self.project.read(cx).is_local()
    }

    fn suggested_filename(&self, cx: &App) -> SharedString {
        format!("{} Copy", self.item.read(cx).title()).into()
    }

    fn suggested_save_as_directory(&self, cx: &App) -> Option<std::path::PathBuf> {
        let item = self.item.read(cx);
        item.project_root()
            .unwrap_or_else(|| item.abs_path())
            .parent()
            .map(std::path::Path::to_path_buf)
    }

    fn validate_save_as(&self, path: std::path::PathBuf, cx: &App) -> Task<Result<()>> {
        let source = self
            .item
            .read(cx)
            .project_root()
            .map(std::path::Path::to_path_buf);
        cx.background_spawn(async move {
            crate::document::validate_project_copy_destination(&path, source.as_deref())?;
            Ok(())
        })
    }

    fn save_as(
        &mut self,
        project: Entity<Project>,
        path: project::ProjectPath,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        if self.prototype_player.is_some() {
            self.exit_prototype_session(cx);
        }
        self.finish_document_edits(cx);
        self.autosave_task = None;
        self.item
            .update(cx, |item, cx| item.save_as(project, path, cx))
    }

    fn save(
        &mut self,
        _options: SaveOptions,
        _project: Entity<Project>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        if self.prototype_player.is_some() {
            self.exit_prototype_session(cx);
        }
        let source_is_dirty = self.code_workspace.read(cx).source_is_dirty(cx);
        let canvas_is_dirty = self.item.read(cx).is_dirty();
        if source_is_dirty && canvas_is_dirty {
            if self.editor_workspace(cx) == EditorWorkspace::Code {
                let discard_canvas = self.item.update(cx, |item, cx| {
                    item.discard_canvas_edits_for_source_resolution(cx)
                });
                let code_workspace = self.code_workspace.clone();
                return cx.spawn(async move |_, cx| {
                    discard_canvas.await?;
                    let save_source = code_workspace.update(cx, |workspace, cx| {
                        workspace
                            .save_source_edit(cx)
                            .unwrap_or_else(|| Task::ready(Ok(())))
                    });
                    save_source.await
                });
            }

            self.finish_document_edits(cx);
            let discard_source = self
                .code_workspace
                .update(cx, |workspace, cx| workspace.discard_source_edit(cx));
            let item = self.item.clone();
            let project = self.project.clone();
            return cx.spawn(async move |_, cx| {
                discard_source.await?;
                let save_document = item.update(cx, |item, cx| item.save(SaveKind::Explicit, cx));
                let Some(root) = save_document.await? else {
                    return Ok(());
                };
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
            });
        }
        if let Some(source_save) = self
            .code_workspace
            .update(cx, |workspace, cx| workspace.save_source_edit(cx))
        {
            return source_save;
        }
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
        if self.prototype_player.is_some() {
            self.exit_prototype_session(cx);
        }
        self.finish_document_edits(cx);
        let discard_source = self
            .code_workspace
            .update(cx, |workspace, cx| workspace.discard_source_edit(cx));
        let item = self.item.clone();
        cx.spawn(async move |_, cx| {
            discard_source.await?;
            item.update(cx, |item, cx| item.reload_from_disk(cx)).await
        })
    }

    fn can_split(&self) -> bool {
        true
    }

    fn clone_on_split(
        &self,
        _workspace_id: Option<workspace::WorkspaceId>,
        window: &mut Window,
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
        let opened_entry_id = self.opened_entry_id;
        let scope = self.scope;
        let last_seen_root = self.last_seen_root;
        let layers_sidebar_visible = self.layers_sidebar_visible;
        let inspector_sidebar_visible = self.inspector_sidebar_visible;
        let layers_sidebar_width = self.layers_sidebar_width;
        let inspector_sidebar_width = self.inspector_sidebar_width;
        let editor_mode = self.editor_mode(cx);
        let editor_workspace = self.editor_workspace(cx);
        let timeline_authoring_enabled = self.is_editable(cx)
            && editor_workspace == EditorWorkspace::Canvas
            && editor_mode == EditorMode::Motion;
        let active_motion_clip = self.active_motion_clip;
        let timeline_model = self.timeline_shell.read(cx).view_model().clone();
        Task::ready(Some(cx.new(|cx| {
            let item_subscription = Self::subscribe_to_item(&item, cx);
            let editor_session =
                cx.new(|_| EditorSession::with_state(editor_mode, editor_workspace));
            let editor_session_subscription = cx.observe(&editor_session, |_, _, cx| cx.notify());
            let (layers_sidebar, inspector_sidebar) =
                Self::new_embedded_sidebars(&project, window, cx);
            let prototype_sidebar = cx.new(|cx| FantaPrototypePanel::new(item.clone(), cx));
            let motion_sidebar = cx.new(|cx| FantaMotionPanel::new(item.clone(), cx));
            motion_sidebar.update(cx, |panel, cx| {
                panel.set_active_clip(active_motion_clip, cx)
            });
            let motion_sidebar_subscription =
                cx.subscribe(&motion_sidebar, |_this, _, event: &MotionPanelEvent, cx| {
                    let event = *event;
                    let view = cx.weak_entity();
                    cx.defer(move |cx| {
                        view.update(cx, |view, cx| view.handle_motion_panel_event(event, cx))
                            .log_err();
                    });
                });
            let variables_workspace =
                cx.new(|cx| FantaVariablesWorkspace::new(item.clone(), window, cx));
            let code_workspace =
                cx.new(|cx| FantaCodeWorkspace::new(item.clone(), project.clone(), window, cx));
            let timeline_shell = cx.new(|cx| {
                let mut timeline = TimelineShell::new();
                timeline.set_authoring_enabled(timeline_authoring_enabled, cx);
                timeline.set_model(timeline_model, cx);
                timeline
            });
            let timeline_subscription =
                cx.subscribe(&timeline_shell, |this, _, event: &TimelineEvent, cx| {
                    this.handle_timeline_event(event.clone(), cx);
                });
            Self {
                item,
                project,
                focus_handle: cx.focus_handle(),
                editor_session,
                layers_sidebar,
                inspector_sidebar,
                prototype_sidebar,
                motion_sidebar,
                variables_workspace,
                code_workspace,
                timeline_shell,
                active_motion_clip,
                motion_keyframe_drag: None,
                layers_sidebar_visible,
                inspector_sidebar_visible,
                layers_sidebar_width,
                inspector_sidebar_width,
                selected_page_index,
                selected_page_root,
                opened_entry_id,
                scope,
                is_focused: false,
                last_seen_root,
                viewport,
                pan_last_position: None,
                primary_pressed: false,
                canvas_pointer_down: false,
                autosave_task: None,
                hover_resize_handle: None,
                space_pan: false,
                container_bounds: None,
                rendered_canvas: None,
                chrome_cache: std::cell::RefCell::new(None),
                #[cfg(target_os = "macos")]
                gpu_canvas: None,
                #[cfg(target_os = "macos")]
                canvas_video: None,
                #[cfg(target_os = "macos")]
                canvas_video_generation: 0,
                #[cfg(target_os = "macos")]
                canvas_video_removed: std::cell::Cell::new(false),
                #[cfg(target_os = "macos")]
                canvas_video_active: std::cell::Cell::new(true),
                tools: ToolShell::new(),
                comment_state: crate::comments_ui::CommentState::default(),
                group_faces: crate::tools::initial_group_faces(),
                #[cfg(feature = "fanta-gpui-ui")]
                gpui_toolbar: crate::gpui_adapters::runtime_enabled(cx)
                    .then(|| crate::gpui_adapters::toolbar::ToolbarAdapter::new(window, cx)),
                #[cfg(feature = "fanta-gpui-ui")]
                gpui_design: (crate::gpui_adapters::runtime_enabled(cx)
                    && crate::gpui_adapters::design::design_enabled())
                .then(|| crate::gpui_adapters::design::DesignAdapter::new(window, cx)),
                fonts_prewarmed: false,
                hovered_node: None,
                text_edit: None,
                pending_text_edit: None,
                prototype_player: None,
                prototype_saved_viewport: None,
                prototype_tick_task: None,
                prototype_last_tick: None,
                prototype_pointer_down: None,
                prototype_drag_fired: false,
                prototype_suppress_click: false,
                prototype_render_cache: None,
                prototype_link_notice: None,
                _item_subscription: item_subscription,
                _editor_session_subscription: editor_session_subscription,
                _motion_sidebar_subscription: motion_sidebar_subscription,
                _timeline_subscription: timeline_subscription,
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
    use std::collections::BTreeMap;

    use fanta_doc::{
        CanvasNode, Color, GroupNode, Mode, ModeId, NodeData, Operation, TextNode, Transform2D,
        VarValue, Variable, VariableCollection, VariableCollectionId, VariableId, VariableType,
        VectorNode,
    };
    use gpui::{TestAppContext, point, size};
    use project::FakeFs;
    use settings::SettingsStore;

    #[cfg(target_os = "macos")]
    async fn canvas_video_fixture(
        cx: &mut TestAppContext,
    ) -> (tempfile::TempDir, Entity<FigItem>, Entity<FigView>) {
        let (directory, _, item, view) = autosave_fixture(cx).await;
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let mut video = CanvasNode::new(NodeData::Video(fanta_doc::VideoNode {
                    asset: AssetId::new(),
                    natural_size: [16, 16],
                    local_size: [80., 80.],
                    time_range_us: [0, 3_000_000],
                    speed: 1.,
                    muted: true,
                    volume: 1.,
                    poster_frame_us: None,
                    poster: None,
                    fit: fanta_doc::ImageFitMode::Fit,
                }));
                video.parent = document.doc.active_page();
                let node = video.id;
                document
                    .doc
                    .apply(Operation::create_node(video))
                    .expect("create video");
                document.doc.selection.replace_with([node]);
                ((), DocChange::Selection)
            });
        });
        (directory, item, view)
    }

    #[cfg(target_os = "macos")]
    async fn video_trim_view_fixture(
        cx: &mut TestAppContext,
    ) -> (
        tempfile::TempDir,
        Entity<FigItem>,
        gpui::WindowHandle<FigView>,
    ) {
        let (directory, item, previous_view) = canvas_video_fixture(cx).await;
        let project = previous_view.read_with(cx, |view, _| view.project.clone());
        let playback = cx.update(crate::video_playback::fake_playback);
        cx.run_until_parked();
        let window = cx.add_window(|window, cx| FigView::new(item.clone(), project, window, cx));
        window
            .update(cx, |_, window, _| window.activate_window())
            .expect("activate trim window");
        cx.run_until_parked();
        window
            .update(cx, |view, window, cx| {
                playback.update(cx, |playback, cx| playback.tick(window, cx));
                let (source, muted, volume) = view.selected_canvas_video_source(cx).expect("video");
                view.canvas_video = Some(CanvasVideoSession {
                    source,
                    loading: None,
                    playback: Some(playback),
                    observation: None,
                    error: None,
                    audio: (muted, volume.to_bits()),
                    bytes: Some(std::sync::Arc::from(&b"original MP4 bytes"[..])),
                    trim: None,
                });
                view.begin_video_trim(window, cx);
                let trim = view
                    .canvas_video
                    .as_ref()
                    .and_then(|session| session.trim.as_ref())
                    .expect("trim controls");
                trim.start
                    .update(cx, |input, cx| input.set_text("0.5", window, cx));
                trim.end
                    .update(cx, |input, cx| input.set_text("2", window, cx));
            })
            .expect("trim controls");
        (directory, item, window)
    }

    #[cfg(target_os = "macos")]
    fn prepared_trim_for_view(range_us: [i64; 2]) -> crate::generation_media::PreparedVideoTrim {
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([225, 0, 225, 255]),
        ))
        .write_to(&mut png, image::ImageFormat::Png)
        .expect("poster");
        crate::generation_media::PreparedVideoTrim {
            range_us,
            source_duration_us: 3_000_000,
            poster: crate::generation_media::VideoPoster {
                png: png.into_inner().into(),
                time_us: range_us[0],
            },
        }
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    async fn video_trim_apply_waits_for_poster_and_commits_one_history_step(
        cx: &mut TestAppContext,
    ) {
        let (_directory, item, window) = video_trim_view_fixture(cx).await;
        let before = item.read_with(cx, |item, _| {
            let doc = &item.document().expect("document").doc;
            let id = single_selection(doc).expect("video");
            (
                id,
                doc.scene.get(id).expect("video").clone(),
                doc.history.undo_depth(),
            )
        });
        let (send, receive) = futures::channel::oneshot::channel();
        window
            .update(cx, |view, _, cx| {
                view.apply_video_trim_with(
                    |bytes, range| {
                        assert_eq!(bytes.as_ref(), b"original MP4 bytes");
                        assert_eq!(range, [500_000, 2_000_000]);
                        Box::pin(async move { receive.await.context("poster response")? })
                    },
                    cx,
                )
            })
            .expect("apply");
        cx.run_until_parked();
        item.read_with(cx, |item, _| {
            let doc = &item.document().expect("document").doc;
            assert_eq!(
                doc.scene.get(before.0),
                Some(&before.1),
                "no partial range mutation before the poster"
            );
            assert_eq!(doc.history.undo_depth(), before.2);
        });
        send.send(Ok(prepared_trim_for_view([500_000, 2_000_000])))
            .unwrap_or_else(|_| panic!("pending poster"));
        cx.run_until_parked();
        item.update(cx, |item, cx| {
            let doc = &item.document().expect("document").doc;
            let NodeData::Video(video) = &doc.scene.get(before.0).expect("video").data else {
                panic!("video")
            };
            assert_eq!(video.time_range_us, [500_000, 2_000_000]);
            assert_eq!(video.poster_frame_us, Some(500_000));
            assert!(video.poster.is_some());
            assert_eq!(doc.history.undo_depth(), before.2 + 1);
            assert!(item.undo(cx).expect("undo trim"));
            assert_eq!(
                item.document().expect("document").doc.scene.get(before.0),
                Some(&before.1)
            );
            assert!(item.redo(cx).expect("redo trim"));
            assert!(item.is_dirty());
        });
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    async fn video_trim_cancel_error_timeout_and_stale_target_preserve_document(
        cx: &mut TestAppContext,
    ) {
        for case in [
            "cancel",
            "error",
            "timeout",
            "deactivate",
            "remove",
            "source-change",
            "input-change",
        ] {
            let (_directory, item, window) = video_trim_view_fixture(cx).await;
            let before = item.read_with(cx, |item, _| {
                let document = item.document().expect("document");
                let id = single_selection(&document.doc).expect("video");
                (
                    id,
                    document.doc.scene.get(id).expect("video").clone(),
                    document.raw_assets.clone(),
                    document.doc.history.undo_depth(),
                )
            });
            let (send, receive) = futures::channel::oneshot::channel::<
                Result<crate::generation_media::PreparedVideoTrim>,
            >();
            let (source, generation) = window
                .update(cx, |view, _, cx| {
                    view.apply_video_trim_with(
                        |_, _| Box::pin(async move { receive.await.context("poster response")? }),
                        cx,
                    );
                    (
                        view.canvas_video.as_ref().expect("session").source,
                        view.canvas_video_generation,
                    )
                })
                .expect("apply");
            cx.run_until_parked();
            match case {
                "cancel" => window
                    .update(cx, |view, _, cx| view.cancel_video_trim(cx))
                    .expect("cancel"),
                "deactivate" => window
                    .update(cx, |view, _, cx| view.set_canvas_video_active(false, cx))
                    .expect("deactivate"),
                "remove" => window
                    .update(cx, |view, _, cx| Item::on_removed(view, cx))
                    .expect("remove"),
                "timeout" => {
                    cx.executor()
                        .advance_clock(std::time::Duration::from_secs(21));
                    cx.run_until_parked();
                }
                "input-change" => window
                    .update(cx, |view, window, cx| {
                        let trim = view
                            .canvas_video
                            .as_ref()
                            .and_then(|session| session.trim.as_ref())
                            .expect("trim");
                        trim.start
                            .update(cx, |input, cx| input.set_text("1", window, cx));
                    })
                    .expect("new trim time"),
                "source-change" => item.update(cx, |item, cx| {
                    item.with_document(cx, |document| {
                        document.doc.selection.clear();
                        ((), DocChange::Selection)
                    });
                }),
                "error" => {
                    send.send(Err(anyhow::anyhow!("injected poster decode failure")))
                        .unwrap_or_else(|_| panic!("pending poster"));
                    cx.run_until_parked();
                }
                _ => unreachable!(),
            }
            if case != "error" && case != "timeout" {
                // Even an already queued completion must not mutate a cancelled or rebound view.
                window
                    .update(cx, |view, _, cx| {
                        view.finish_video_trim(
                            source,
                            generation,
                            Ok(prepared_trim_for_view([500_000, 2_000_000])),
                            cx,
                        )
                    })
                    .expect("late completion");
            }
            if matches!(case, "error" | "timeout" | "input-change") {
                window
                    .update(cx, |view, _, _| {
                        let trim = view
                            .canvas_video
                            .as_ref()
                            .and_then(|session| session.trim.as_ref())
                            .expect("retryable trim inputs");
                        assert!(trim.error.is_some(), "{case}");
                        assert!(trim.task.borrow().is_none(), "{case}");
                    })
                    .expect("visible failure");
            }
            item.read_with(cx, |item, _| {
                let document = item.document().expect("document");
                assert_eq!(document.doc.scene.get(before.0), Some(&before.1), "{case}");
                assert_eq!(document.raw_assets, before.2, "{case}");
                assert_eq!(document.doc.history.undo_depth(), before.3, "{case}");
            });
        }
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    async fn video_trim_controls_open_cancel_and_fit_the_canvas_footer(cx: &mut TestAppContext) {
        let (_directory, item, window) = video_trim_view_fixture(cx).await;
        let view = window.entity(cx).expect("view");
        let depth = item.read_with(cx, |item, _| {
            item.document().expect("document").doc.history.undo_depth()
        });
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        for width in [1_000., 1_200.] {
            visual.simulate_resize(size(px(width), px(900.)));
            visual.update(|window, cx| window.draw(cx).clear());
            let footer = visual
                .debug_bounds("canvas-video-controls")
                .expect("video controls");
            for selector in [
                "video-trim-start",
                "video-trim-end",
                "video-trim-cancel-target",
            ] {
                let bounds = visual.debug_bounds(selector).expect("visible trim control");
                assert!(
                    bounds.left() >= footer.left() && bounds.right() <= footer.right(),
                    "{selector}: {bounds:?} outside {footer:?}"
                );
                assert!(
                    bounds.top() >= footer.top() && bounds.bottom() <= footer.bottom(),
                    "{selector}"
                );
            }
        }
        let cancel = visual
            .debug_bounds("video-trim-cancel-target")
            .expect("cancel");
        visual.simulate_click(cancel.center(), gpui::Modifiers::none());
        visual.update(|window, cx| window.draw(cx).clear());
        view.read_with(&visual, |view, _| {
            assert!(view.canvas_video.as_ref().expect("session").trim.is_none())
        });
        let open = visual
            .debug_bounds("video-trim-open-target")
            .expect("trim button");
        visual.simulate_click(open.center(), gpui::Modifiers::none());
        visual.update(|window, cx| window.draw(cx).clear());
        assert!(visual.debug_bounds("video-trim-start").is_some());
        item.read_with(&visual, |item, _| {
            assert_eq!(
                item.document().expect("document").doc.history.undo_depth(),
                depth
            )
        });
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    async fn video_trim_invalid_input_never_starts_preparation(cx: &mut TestAppContext) {
        let (_directory, item, window) = video_trim_view_fixture(cx).await;
        let before = item.read_with(cx, |item, _| {
            item.document().expect("document").doc.history.undo_depth()
        });
        for (start, end) in [
            ("NaN", "2"),
            ("-1", "2"),
            ("2", "2"),
            ("2", "1"),
            ("0", "11"),
        ] {
            window
                .update(cx, |view, window, cx| {
                    let trim = view
                        .canvas_video
                        .as_ref()
                        .and_then(|session| session.trim.as_ref())
                        .expect("trim");
                    trim.start
                        .update(cx, |input, cx| input.set_text(start, window, cx));
                    trim.end
                        .update(cx, |input, cx| input.set_text(end, window, cx));
                    view.apply_video_trim_with(|_, _| panic!("invalid input reached decoder"), cx);
                    let trim = view
                        .canvas_video
                        .as_ref()
                        .and_then(|session| session.trim.as_ref())
                        .expect("trim");
                    assert!(trim.error.is_some());
                    assert!(trim.task.borrow().is_none());
                })
                .expect("validate input");
        }
        assert_eq!(
            item.read_with(cx, |item, _| item
                .document()
                .expect("document")
                .doc
                .history
                .undo_depth()),
            before
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn video_trim_decimal_times_preserve_microseconds_without_float_rounding() {
        for (text, expected) in [
            ("0", 0),
            ("1.25", 1_250_000),
            ("0.000001", 1),
            ("9223372036854.775807", i64::MAX),
        ] {
            assert_eq!(parse_video_trim_time(text).expect("valid time"), expected);
            assert_eq!(
                parse_video_trim_time(&video_trim_time_text(expected)).expect("round trip"),
                expected
            );
        }
        for text in ["NaN", "-0.1", "1e3", "1.0000001", "9223372036854.775808"] {
            assert!(parse_video_trim_time(text).is_err(), "{text}");
        }
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    async fn video_trim_authored_range_starts_canvas_loading(cx: &mut TestAppContext) {
        let (_directory, item, view) = canvas_video_fixture(cx).await;
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let id = single_selection(&document.doc).expect("selected video");
                let old = document.doc.scene.get(id).expect("video").data.clone();
                let mut new = old.clone();
                let NodeData::Video(video) = &mut new else {
                    panic!("video")
                };
                video.time_range_us = [500_000, 2_000_000];
                document
                    .doc
                    .apply(Operation::ReplaceData {
                        id,
                        old: Box::new(old),
                        new: Box::new(new),
                    })
                    .expect("author source range");
                ((), DocChange::Content)
            });
        });
        view.update(cx, |view, cx| {
            view.sync_canvas_video(true, cx);
            let session = view.canvas_video.as_ref().expect("video session");
            assert!(
                session.error.is_none(),
                "valid authored trim must be playable"
            );
            assert!(
                session.loading.is_some(),
                "load the original source without rewriting it"
            );
            view.clear_canvas_video(cx);
        });
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    async fn canvas_video_controls_remain_below_the_design_toolbar(cx: &mut TestAppContext) {
        init_visual_test(cx);
        #[cfg(feature = "fanta-gpui-ui")]
        cx.update(|cx| {
            gpui_component::init(cx);
            fanta_gpui::init(cx);
            crate::theme_bridge::init(cx);
        });
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let mut document = doc_with_one_page();
        let mut video = CanvasNode::new(NodeData::Video(fanta_doc::VideoNode {
            asset: AssetId::new(),
            natural_size: [16, 16],
            local_size: [80., 80.],
            time_range_us: [0, 10_000_000],
            speed: 1.,
            muted: true,
            volume: 1.,
            poster_frame_us: None,
            poster: None,
            fit: fanta_doc::ImageFitMode::Fit,
        }));
        video.parent = document.active_page();
        let node = video.id;
        document
            .apply(Operation::create_node(video))
            .expect("video");
        document.selection.replace_with([node]);
        let item = crate::document::ready_item_for_test(
            &project,
            std::path::PathBuf::from("/tmp/Video-layout.fig"),
            document,
            cx,
        );
        let window = cx.add_window(move |window, cx| FigView::new(item, project, window, cx));
        let view = window.entity(cx).expect("fig view");
        #[cfg(feature = "fanta-gpui-ui")]
        assert!(view.read_with(cx, |view, _| view.gpui_toolbar.is_some()));
        let playback = cx.update(crate::video_playback::fake_playback);
        cx.run_until_parked();
        view.update(cx, |view, cx| {
            let (source, muted, volume) = view.selected_canvas_video_source(cx).expect("video");
            view.canvas_video = Some(CanvasVideoSession {
                source,
                loading: None,
                playback: Some(playback),
                observation: None,
                error: None,
                audio: (muted, volume.to_bits()),
                bytes: None,
                trim: None,
            });
        });
        let mut visual_context = gpui::VisualTestContext::from_window(window.into(), cx);
        for width in [1_000., 1_400.] {
            visual_context.simulate_resize(size(px(width), px(800.)));
            visual_context.update(|window, cx| window.draw(cx).clear());
            let canvas = visual_context
                .debug_bounds("fig-container")
                .expect("canvas");
            let controls = visual_context
                .debug_bounds("canvas-video-controls")
                .expect("controls");
            let toolbar = visual_context
                .debug_bounds("fanta-canvas-toolbar")
                .expect("toolbar");
            assert!(
                toolbar.bottom() <= canvas.bottom(),
                "toolbar must stay in the canvas: {toolbar:?} vs {canvas:?}"
            );
            assert!(
                toolbar.bottom() < controls.top(),
                "toolbar covers video controls: {toolbar:?} vs {controls:?}"
            );
            #[cfg(feature = "fanta-gpui-ui")]
            {
                let surface = visual_context
                    .debug_bounds("editor-toolbar-surface")
                    .expect("compact toolbar surface");
                assert!(surface.is_contained_within(&canvas));
                assert!(
                    (f32::from(surface.center().x) - f32::from(canvas.center().x)).abs() <= 0.5,
                    "toolbar surface must stay centered in the canvas: {surface:?} vs {canvas:?}"
                );
                assert!(
                    surface.bottom() + px(12.) <= controls.top(),
                    "canvas inset must keep the surface clear of video controls: {surface:?} vs {controls:?}"
                );
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    async fn canvas_video_rejects_late_loading_and_reports_missing_sources(
        cx: &mut TestAppContext,
    ) {
        let (_directory, item, view) = canvas_video_fixture(cx).await;
        let (source, generation) = view.update(cx, |view, cx| {
            view.sync_canvas_video(true, cx);
            (
                view.canvas_video.as_ref().expect("loading video").source,
                view.canvas_video_generation,
            )
        });
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            let session = view.canvas_video.as_ref().expect("failed video");
            assert!(session.playback.is_none());
            assert!(session.loading.is_none());
            assert!(
                session
                    .error
                    .as_ref()
                    .expect("visible error")
                    .contains("missing")
            );
        });
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.selection.replace_with([]);
                ((), DocChange::Selection)
            });
        });
        cx.run_until_parked();
        view.update(cx, |view, cx| {
            assert!(view.canvas_video.is_none());
            view.finish_canvas_video_load(
                source,
                generation,
                Ok(std::sync::Arc::from([1_u8].as_slice())),
                cx,
            );
            assert!(
                view.canvas_video.is_none(),
                "old source cannot install after deselection"
            );
        });
    }

    #[cfg(target_os = "macos")]
    #[gpui::test]
    async fn canvas_video_removal_closes_retained_player_without_editing_document(
        cx: &mut TestAppContext,
    ) {
        let (_directory, item, view) = canvas_video_fixture(cx).await;
        let before = item.read_with(cx, |item, _cx| {
            (
                serde_json::to_value(&item.document().expect("document").doc).expect("serialize"),
                item.is_dirty(),
            )
        });
        let playback = cx.update(crate::video_playback::fake_playback);
        cx.run_until_parked();
        let original_revision = playback.read_with(cx, |playback, _| playback.frame_revision());
        view.update(cx, |view, cx| {
            let (source, muted, volume) = view
                .selected_canvas_video_source(cx)
                .expect("selected video");
            view.canvas_video = Some(CanvasVideoSession {
                source,
                loading: None,
                playback: Some(playback.clone()),
                observation: None,
                error: None,
                audio: (muted, volume.to_bits()),
                bytes: None,
                trim: None,
            });
            Item::on_removed(view, cx);
            assert!(view.canvas_video_frame(cx).is_none());
            view.finish_canvas_video_load(
                source,
                view.canvas_video_generation,
                Ok(std::sync::Arc::from([1_u8].as_slice())),
                cx,
            );
            assert_eq!(
                view.canvas_video
                    .as_ref()
                    .and_then(|session| session.playback.as_ref())
                    .map(Entity::entity_id),
                Some(playback.entity_id())
            );
        });
        playback.read_with(cx, |playback, _| {
            assert!(playback.frame().is_none());
            assert!(
                playback.frame_revision() > original_revision,
                "retained player was closed"
            );
        });
        let project = view.read_with(cx, |view, _| view.project.clone());
        let scratch = cx.add_window(|_, _| gpui::Empty);
        scratch
            .update(cx, |_, window, cx| {
                let workspace = cx.new(|cx| workspace::Workspace::test_new(project, window, cx));
                workspace.update(cx, |workspace, cx| {
                    view.update(cx, |view, cx| {
                        Item::added_to_workspace(view, workspace, window, cx);
                        assert!(
                            view.canvas_video.is_none(),
                            "moving a tab must discard its closed player"
                        );
                        assert!(!view.canvas_video_removed.get());
                        view.sync_canvas_video(true, cx);
                        assert!(
                            view.canvas_video
                                .as_ref()
                                .is_some_and(|session| session.loading.is_some()),
                            "re-added tab can prepare the selected source again"
                        );
                    });
                });
            })
            .expect("re-add the same canvas tab");
        cx.run_until_parked();
        item.read_with(cx, |item, _| {
            assert_eq!(
                serde_json::to_value(&item.document().expect("document").doc).expect("serialize"),
                before.0
            );
            assert_eq!(item.is_dirty(), before.1);
        });
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn video_trim_source_validation_accepts_bounded_ranges_and_rejects_unknown_duration() {
        assert!(canvas_video_duration_supported([0, 3_000_000], 3_000_000));
        assert!(canvas_video_duration_supported(
            [1_000_000, 3_000_000],
            3_000_000
        ));
        assert!(canvas_video_duration_supported([0, 2_000_000], 3_000_000));
        assert!(!canvas_video_duration_supported([0, 4_000_000], 3_000_000));
        assert!(!canvas_video_duration_supported([-1, 2_000_000], 3_000_000));
        assert!(!canvas_video_duration_supported(
            [2_000_000, 2_000_000],
            3_000_000
        ));
        assert!(!canvas_video_duration_supported([0, -1], 3_000_000));
        assert!(!canvas_video_duration_supported([0, 3_000_000], 0));
    }

    #[test]
    fn prototype_links_allow_web_urls_and_block_local_or_executable_schemes() {
        assert!(validate_prototype_link("https://example.com/design").is_ok());
        assert!(validate_prototype_link("http://example.com").is_ok());
        assert!(validate_prototype_link("file:///tmp/private").is_err());
        assert!(validate_prototype_link("javascript:alert(1)").is_err());
        assert!(validate_prototype_link("not a url").is_err());
    }

    #[test]
    fn prototype_clock_uses_measured_elapsed_time_and_resets_cleanly() {
        let start = std::time::Instant::now();
        let mut last_tick = Some(start);
        assert_eq!(
            prototype_tick_elapsed(&mut last_tick, start + std::time::Duration::from_millis(73)),
            std::time::Duration::from_millis(73)
        );
        last_tick = None;
        assert_eq!(
            prototype_tick_elapsed(&mut last_tick, start),
            std::time::Duration::from_millis(16)
        );
    }

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
        });
    }

    fn init_visual_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            zlog::init_test();
            assets::Assets.load_test_fonts(cx);
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            release_channel::init(semver::Version::new(0, 0, 0), cx);
            editor::init(cx);
        });
    }

    fn doc_with_one_page() -> fanta_doc::Doc {
        let mut doc = fanta_doc::Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "Page 1".to_owned();
        let root = page.id;
        doc.apply(Operation::create_node(page))
            .expect("create page root node");
        doc.add_page(root);
        doc.set_active_page(Some(root));
        doc
    }

    fn text_selection_doc(wrapped: bool) -> (fanta_doc::Doc, NodeId, Option<NodeId>) {
        let mut doc = doc_with_one_page();
        let page = doc.active_page().expect("active page");
        let frame = wrapped.then(|| {
            let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
                clip_size: Some([200.0, 100.0]),
                ..GroupNode::default()
            }));
            frame.parent = Some(page);
            frame.transform = Transform2D::translation(-100.0, -50.0);
            let id = frame.id;
            doc.apply(Operation::create_node(frame))
                .expect("create text wrapper");
            id
        });
        let mut text = CanvasNode::new(NodeData::Text(TextNode::new("Hello world", 120.0, 40.0)));
        text.parent = Some(frame.unwrap_or(page));
        text.transform = if wrapped {
            Transform2D::translation(20.0, 20.0)
        } else {
            Transform2D::translation(-60.0, -20.0)
        };
        let text_id = text.id;
        doc.apply(Operation::create_node(text))
            .expect("create text layer");
        (doc, text_id, frame)
    }

    fn send_canvas_click(
        scratch: gpui::WindowHandle<gpui::Empty>,
        view: &Entity<FigView>,
        position: Point<Pixels>,
        click_count: usize,
        cx: &mut TestAppContext,
    ) {
        scratch
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| {
                    view.handle_mouse_down(
                        &MouseDownEvent {
                            button: MouseButton::Left,
                            position,
                            modifiers: gpui::Modifiers::default(),
                            click_count,
                            first_mouse: false,
                        },
                        window,
                        cx,
                    );
                    view.handle_mouse_up(
                        &MouseUpEvent {
                            button: MouseButton::Left,
                            position,
                            modifiers: gpui::Modifiers::default(),
                            click_count,
                        },
                        window,
                        cx,
                    );
                });
            })
            .expect("dispatch canvas click");
    }

    /// A project on disk with one page and no children, plus the view that
    /// owns its autosave timer.
    async fn autosave_fixture(
        cx: &mut TestAppContext,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        Entity<FigItem>,
        Entity<FigView>,
    ) {
        init_visual_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().join("Design");
        let item = crate::document::ready_item_with_root_for_test(
            &project,
            dir.path().join("Design.fig"),
            Some(root.clone()),
            doc_with_one_page(),
            cx,
        );
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let view = scratch
            .update(cx, |_, window, cx| {
                cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
            })
            .expect("create fig view");
        (dir, root, item, view)
    }

    fn add_rect(item: &Entity<FigItem>, cx: &mut TestAppContext) {
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let page = document.doc.active_page().expect("active page");
                let mut rect = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
                    0.0,
                    0.0,
                    10.0,
                    10.0,
                    Color::BLACK,
                )));
                rect.parent = Some(page);
                document
                    .doc
                    .apply(Operation::create_node(rect))
                    .expect("create rect");
                ((), DocChange::Content)
            });
        });
    }

    #[cfg(feature = "fanta-gpui-ui")]
    #[gpui::test]
    async fn toolbar_export_writes_the_default_preset_and_shows_canvas_feedback(
        cx: &mut TestAppContext,
    ) {
        init_visual_test(cx);
        cx.executor().allow_parking();
        cx.update(project::DisableAiSettings::register);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let directory = tempfile::tempdir().expect("temporary export directory");
        let project_root = directory.path().join("Design");
        let item = crate::document::ready_item_with_root_for_test(
            &project,
            directory.path().join("Design.fig"),
            Some(project_root.clone()),
            doc_with_one_page(),
            cx,
        );
        add_rect(&item, cx);

        let window =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let view = window
            .update(cx, |_, window, cx| {
                cx.new(|cx| FigView::new(item, project, window, cx))
            })
            .expect("create canvas in workspace window");
        cx.run_until_parked();

        window
            .update(cx, |_, window, cx| {
                window.activate_window();
                view.update(cx, |view, cx| {
                    view.inspector_sidebar_visible = false;
                    view.export_from_toolbar(window, cx);
                });
            })
            .expect("invoke toolbar export");
        cx.run_until_parked();

        assert!(project_root.join("exports/Page 1@2x.png").is_file());
        assert!(!view.read_with(cx, |view, _| view.inspector_sidebar_visible));
        let workspace = window
            .read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone())
            .expect("workspace window remains open");
        assert_eq!(
            workspace.read_with(cx, |workspace, _| workspace.notification_ids()),
            [NotificationId::named(CANVAS_NOTICE_ID.into())]
        );
    }

    #[cfg(feature = "fanta-gpui-ui")]
    #[gpui::test]
    async fn toolbar_export_reports_an_unsaved_canvas_on_the_canvas_surface(
        cx: &mut TestAppContext,
    ) {
        init_visual_test(cx);
        cx.executor().allow_parking();
        cx.update(project::DisableAiSettings::register);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let item = crate::document::ready_item_for_test(
            &project,
            std::path::PathBuf::from("/tmp/Unsaved-export.fig"),
            doc_with_one_page(),
            cx,
        );
        let window =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let view = window
            .update(cx, |_, window, cx| {
                cx.new(|cx| FigView::new(item, project, window, cx))
            })
            .expect("create unsaved canvas in workspace window");
        cx.run_until_parked();

        window
            .update(cx, |_, window, cx| {
                window.activate_window();
                view.update(cx, |view, cx| {
                    view.inspector_sidebar_visible = false;
                    view.export_from_toolbar(window, cx);
                });
            })
            .expect("invoke toolbar export");
        cx.run_until_parked();

        assert!(!view.read_with(cx, |view, _| view.inspector_sidebar_visible));
        let workspace = window
            .read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone())
            .expect("workspace window remains open");
        assert_eq!(
            workspace.read_with(cx, |workspace, _| workspace.notification_ids()),
            [NotificationId::named(CANVAS_NOTICE_ID.into())]
        );
    }

    #[gpui::test]
    async fn save_as_updates_shared_views_and_their_project_entries(cx: &mut TestAppContext) {
        let result: Result<()> = async {
            init_visual_test(cx);
            let directory = tempfile::tempdir()?;
            let original = directory.path().join("Original");
            let copy = directory.path().join("Copy");
            let document = doc_with_one_page();
            crate::document::write_project(&original, &document, &BTreeMap::new())?;
            let file_system = FakeFs::new(cx.executor());
            file_system
                .insert_tree(
                    directory.path(),
                    serde_json::json!({"Original": {"fanta.json": "{}"}}),
                )
                .await;
            let project = Project::test(file_system.clone(), [directory.path()], cx).await;
            let original_entry = project.read_with(cx, |project, cx| {
                let path = project
                    .find_project_path(original.join("fanta.json"), cx)
                    .expect("original path");
                project
                    .entry_for_path(&path, cx)
                    .expect("original entry")
                    .id
            });
            let destination = project
                .read_with(cx, |project, cx| project.find_project_path(&copy, cx))
                .context("copy path")?;
            let item = crate::document::ready_item_with_root_for_test(
                &project,
                original.join("fanta.json"),
                Some(original.clone()),
                document,
                cx,
            );
            let scratch = cx.add_window(|_, _| gpui::Empty);
            let views = scratch.update(cx, |_, window, cx| {
                (0..2)
                    .map(|_| {
                        cx.new(|cx| {
                            let mut view = FigView::new(item.clone(), project.clone(), window, cx);
                            view.opened_entry_id = Some(original_entry);
                            view
                        })
                    })
                    .collect::<Vec<_>>()
            })?;
            let view = views.first().context("first view")?;
            view.read_with(cx, |view, cx| {
                assert!(view.can_save_as(cx));
                assert_eq!(view.suggested_filename(cx).as_ref(), "Original Copy");
            });
            add_rect(&item, cx);
            scratch
                .update(cx, |_, window, cx| {
                    view.update(cx, |view, cx| {
                        Item::save_as(view, project.clone(), destination, window, cx)
                    })
                })?
                .await?;
            file_system
                .insert_tree(&copy, serde_json::json!({"fanta.json": "{}"}))
                .await;
            cx.run_until_parked();
            let copied_entry = project.read_with(cx, |project, cx| {
                let path = project
                    .find_project_path(copy.join("fanta.json"), cx)
                    .expect("copied path");
                project.entry_for_path(&path, cx).expect("copied entry").id
            });
            for view in &views {
                view.read_with(cx, |view, cx| {
                    assert_eq!(view.tab_content_text(0, cx).as_ref(), "Copy");
                    assert_eq!(view.item.entity_id(), item.entity_id());
                    assert_eq!(view.project_entry_ids(cx).as_slice(), &[copied_entry]);
                    assert_ne!(copied_entry, original_entry);
                    assert!(!view.is_dirty(cx));
                });
            }
            let (original_document, _) = fanta_format::read_project_tree(&original)?;
            let (copied_document, _) = fanta_format::read_project_tree(&copy)?;
            assert_eq!(original_document.scene.len(), 1);
            assert_eq!(copied_document.scene.len(), 2);
            Ok(())
        }
        .await;
        result.expect("Save As updates shared views and their project entries");
    }

    #[gpui::test]
    async fn save_as_native_picker_starts_beside_the_original_project(cx: &mut TestAppContext) {
        let result: Result<()> = async {
            init_visual_test(cx);
            let directory = tempfile::tempdir()?;
            let original = directory.path().join("Original");
            let document = doc_with_one_page();
            crate::document::write_project(&original, &document, &BTreeMap::new())?;
            let file_system = FakeFs::new(cx.executor());
            file_system
                .insert_tree(&original, serde_json::json!({"fanta.json": "{}"}))
                .await;
            let project = Project::test(file_system, [original.as_path()], cx).await;
            let item = crate::document::ready_item_with_root_for_test(
                &project,
                original.join("fanta.json"),
                Some(original.clone()),
                document,
                cx,
            );
            item.update(cx, |item, cx| {
                item.path = project
                    .read(cx)
                    .find_project_path(item.abs_path(), cx)
                    .expect("original path");
            });
            let scratch = cx.add_window(|_, _| gpui::Empty);
            let (workspace, view) = scratch.update(cx, |_, window, cx| {
                let workspace =
                    cx.new(|cx| workspace::Workspace::test_new(project.clone(), window, cx));
                let view = cx.new(|cx| FigView::new(item.clone(), project, window, cx));
                workspace.update(cx, |workspace, cx| {
                    workspace.add_item_to_active_pane(
                        Box::new(view.clone()),
                        None,
                        true,
                        window,
                        cx,
                    )
                });
                (workspace, view)
            })?;
            assert_eq!(
                view.read_with(cx, |view, cx| view.suggested_filename(cx)),
                "Original Copy"
            );
            let save = scratch.update(cx, |_, window, cx| {
                workspace.update(cx, |workspace, cx| {
                    workspace.save_active_item(workspace::SaveIntent::SaveAs, window, cx)
                })
            })?;
            cx.run_until_parked();
            assert!(
                cx.did_prompt_for_new_path(),
                "Save As reaches the native path prompt"
            );
            cx.simulate_new_path_selection(|initial_directory| {
                assert_eq!(
                    initial_directory,
                    directory.path(),
                    "a copied design belongs beside its original, not inside it"
                );
                None
            });
            save.await?;
            assert_eq!(
                item.read_with(cx, |item, _| item
                    .project_root()
                    .map(std::path::Path::to_path_buf)),
                Some(original)
            );
            assert!(
                !directory.path().join("Original Copy").exists(),
                "cancel does not create a copy"
            );
            Ok(())
        }
        .await;
        result.expect("Save As uses the project parent as its native initial directory");
    }

    #[gpui::test]
    async fn save_as_rejects_existing_design_before_opening_its_worktree(cx: &mut TestAppContext) {
        let result: Result<()> = async {
            init_visual_test(cx);
            cx.update(|cx| {
                workspace::register_project_item::<FigView>(cx);
                crate::workspace_hooks::init(cx);
            });
            let directory = tempfile::tempdir()?;
            let original = directory.path().join("Original");
            let copy = directory.path().join("Original Copy");
            let document = doc_with_one_page();
            let page = document.active_page().context("active page")?;
            for root in [&original, &copy] {
                crate::document::write_project(root, &document, &BTreeMap::new())?;
            }
            let original_manifest = std::fs::read(original.join("fanta.json"))?;
            let original_source =
                fanta_format::locate_page_source(&original, page).context("original page")?;
            let original_bytes = std::fs::read(&original_source)?;
            let copy_source =
                fanta_format::locate_page_source(&copy, page).context("copied page")?;
            let copy_bytes = std::fs::read(&copy_source)?;
            let file_system = FakeFs::new(cx.executor());
            file_system
                .insert_tree(
                    directory.path(),
                    serde_json::json!({
                        "Original": {"fanta.json": std::fs::read_to_string(original.join("fanta.json"))?},
                        "Original Copy": {"fanta.json": std::fs::read_to_string(copy.join("fanta.json"))?},
                    }),
                )
                .await;
            let project = Project::test(file_system, [copy.as_path()], cx).await;
            let item = crate::document::ready_item_with_root_for_test(
                &project,
                copy.join("fanta.json"),
                Some(copy.clone()),
                document,
                cx,
            );
            item.update(cx, |item, cx| {
                item.path = project
                    .read(cx)
                    .find_project_path(item.abs_path(), cx)
                    .expect("copied design path");
            });
            let scratch = cx.add_window(|_, _| gpui::Empty);
            let (workspace, view) = scratch.update(cx, |_, window, cx| {
                let workspace =
                    cx.new(|cx| workspace::Workspace::test_new(project.clone(), window, cx));
                let view = cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx));
                workspace.update(cx, |workspace, cx| {
                    workspace.add_item_to_active_pane(
                        Box::new(view.clone()),
                        None,
                        true,
                        window,
                        cx,
                    );
                });
                (workspace, view)
            })?;
            cx.run_until_parked();
            assert_eq!(
                project.read_with(cx, |project, cx| project.worktrees(cx).count()),
                1
            );
            assert_eq!(
                workspace.read_with(cx, |workspace, cx| workspace.items_of_type::<FigView>(cx).count()),
                1
            );
            add_rect(&item, cx);
            let save = scratch.update(cx, |_, window, cx| {
                workspace.update(cx, |workspace, cx| {
                    workspace.save_active_item(workspace::SaveIntent::SaveAs, window, cx)
                })
            })?;
            cx.run_until_parked();
            assert!(cx.did_prompt_for_new_path());
            cx.simulate_new_path_selection(|_| Some(original.clone()));
            let error = save.await.expect_err("an existing design rejects Save As");
            assert!(error.to_string().contains("is not empty"));
            cx.run_until_parked();
            project.read_with(cx, |project, cx| {
                assert_eq!(
                    project.worktrees(cx).count(),
                    1,
                    "a rejected destination must not become a worktree"
                );
                assert!(project.find_project_path(&original, cx).is_none());
            });
            workspace.read_with(cx, |workspace, cx| {
                assert_eq!(
                    workspace.items_of_type::<FigView>(cx).count(),
                    1,
                    "the destination must not open a tab"
                );
                assert_eq!(
                    workspace.active_item(cx).map(|item| item.item_id()),
                    Some(view.entity_id())
                );
            });
            assert_eq!(std::fs::read(original.join("fanta.json"))?, original_manifest);
            assert_eq!(std::fs::read(original_source)?, original_bytes);
            assert_eq!(std::fs::read(copy_source)?, copy_bytes);
            view.read_with(cx, |view, cx| {
                assert_eq!(view.item.read(cx).project_root(), Some(copy.as_path()));
                assert!(view.is_dirty(cx));
                assert!(view.autosave_task.is_some());
            });
            cx.executor().advance_clock(AUTOSAVE_DEBOUNCE * 2);
            cx.run_until_parked();
            assert_eq!(fanta_format::read_project_tree(&copy)?.0.scene.len(), 2);
            assert_eq!(fanta_format::read_project_tree(&original)?.0.scene.len(), 1);
            Ok(())
        }
        .await;
        result
            .expect("rejected Save As preserves the active design without opening its destination");
    }

    #[gpui::test]
    async fn save_as_failure_resumes_autosave_on_the_original_design(cx: &mut TestAppContext) {
        let result: Result<()> = async {
            init_visual_test(cx);
            let directory = tempfile::tempdir()?;
            let original = directory.path().join("Original");
            let occupied = directory.path().join("Occupied");
            let document = doc_with_one_page();
            crate::document::write_project(&original, &document, &BTreeMap::new())?;
            std::fs::create_dir(&occupied)?;
            std::fs::write(occupied.join("keep.txt"), "untouched")?;
            let file_system = FakeFs::new(cx.executor());
            file_system
                .insert_tree(
                    directory.path(),
                    serde_json::json!({"Original": {"fanta.json": "{}"}, "Occupied": {}}),
                )
                .await;
            let project = Project::test(file_system, [directory.path()], cx).await;
            let destination = project
                .read_with(cx, |project, cx| project.find_project_path(&occupied, cx))
                .context("occupied path")?;
            let item = crate::document::ready_item_with_root_for_test(
                &project,
                original.join("fanta.json"),
                Some(original.clone()),
                document,
                cx,
            );
            let scratch = cx.add_window(|_, _| gpui::Empty);
            let view = scratch.update(cx, |_, window, cx| {
                cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
            })?;
            add_rect(&item, cx);
            let save_as = scratch.update(cx, |_, window, cx| {
                view.update(cx, |view, cx| {
                    Item::save_as(view, project, destination, window, cx)
                })
            })?;
            assert!(
                save_as.await.is_err(),
                "the occupied destination must reject Save As"
            );
            cx.run_until_parked();
            view.read_with(cx, |view, cx| {
                assert!(
                    view.autosave_task.is_some(),
                    "the failed copy re-arms autosave"
                );
                assert!(view.is_dirty(cx));
                assert_eq!(view.item.read(cx).project_root(), Some(original.as_path()));
            });
            assert_eq!(fanta_format::read_project_tree(&original)?.0.scene.len(), 1);
            cx.executor().advance_clock(AUTOSAVE_DEBOUNCE * 2);
            cx.run_until_parked();
            assert_eq!(fanta_format::read_project_tree(&original)?.0.scene.len(), 2);
            assert!(!item.read_with(cx, |item, _| item.is_dirty()));
            assert_eq!(
                std::fs::read_to_string(occupied.join("keep.txt"))?,
                "untouched"
            );
            assert!(!occupied.join("fanta.json").exists());
            Ok(())
        }
        .await;
        result.expect("a failed Save As must not disable autosaving the original design");
    }

    /// The hero promise: an edit reaches the project tree on its own, so
    /// `git diff` shows it without the user pressing cmd-s.
    #[gpui::test]
    async fn an_edit_autosaves_the_project_after_the_debounce(cx: &mut TestAppContext) {
        let (_dir, root, item, view) = autosave_fixture(cx).await;
        add_rect(&item, cx);
        item.read_with(cx, |item, _| assert!(item.is_dirty()));

        cx.executor().advance_clock(AUTOSAVE_DEBOUNCE * 2);
        cx.run_until_parked();

        item.read_with(cx, |item, _| {
            assert!(
                !item.is_dirty(),
                "the debounce elapsed and the canvas saved"
            )
        });
        assert!(
            root.join("fanta.json").is_file(),
            "the project tree was written to disk"
        );
        view.read_with(cx, |view, _| assert!(view.autosave_task.is_none()));
    }

    /// A drag longer than the debounce must not have an intermediate position
    /// written under it: `mark_edited` announces the dirty transition on the
    /// first preview frame, so the timer expires mid-gesture.
    #[gpui::test]
    async fn the_autosave_waits_for_the_pointer_to_come_up(cx: &mut TestAppContext) {
        let (_dir, root, item, view) = autosave_fixture(cx).await;
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let position = point(px(10.), px(10.));
        scratch
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| {
                    view.handle_mouse_down(
                        &MouseDownEvent {
                            button: MouseButton::Left,
                            position,
                            modifiers: gpui::Modifiers::default(),
                            click_count: 1,
                            first_mouse: false,
                        },
                        window,
                        cx,
                    );
                });
            })
            .expect("press the canvas");
        add_rect(&item, cx);

        cx.executor().advance_clock(AUTOSAVE_DEBOUNCE * 2);
        cx.run_until_parked();
        item.read_with(cx, |item, _| {
            assert!(item.is_dirty(), "a save mid-gesture would disrupt the drag")
        });
        assert!(
            !root.exists(),
            "nothing was written while the pointer was down"
        );

        scratch
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| {
                    view.handle_mouse_up(
                        &MouseUpEvent {
                            button: MouseButton::Left,
                            position,
                            modifiers: gpui::Modifiers::default(),
                            click_count: 1,
                        },
                        window,
                        cx,
                    );
                });
            })
            .expect("release the canvas");
        cx.executor().advance_clock(AUTOSAVE_DEBOUNCE * 2);
        cx.run_until_parked();

        item.read_with(cx, |item, _| {
            assert!(
                !item.is_dirty(),
                "the release let the pending autosave through"
            )
        });
        assert!(root.join("fanta.json").is_file());
    }

    /// A bare `.fig` has no project directory; the first save would scaffold
    /// one next to it, which a timer must never do behind the user's back.
    #[gpui::test]
    async fn a_fig_without_a_project_is_never_autosaved(cx: &mut TestAppContext) {
        init_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let dir = tempfile::tempdir().expect("temp dir");
        let item = crate::document::ready_item_for_test(
            &project,
            dir.path().join("Design.fig"),
            doc_with_one_page(),
            cx,
        );
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let view = scratch
            .update(cx, |_, window, cx| {
                cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
            })
            .expect("create fig view");
        add_rect(&item, cx);

        cx.executor().advance_clock(AUTOSAVE_DEBOUNCE * 2);
        cx.run_until_parked();

        item.read_with(cx, |item, _| assert!(item.is_dirty()));
        assert!(
            !dir.path().join("Design").exists(),
            "the autosave must not materialize a project directory"
        );
        view.read_with(cx, |view, _| assert!(view.autosave_task.is_none()));
    }

    /// The cursor must promise only what a press would actually start, and
    /// the hover result is cached for `render` — never recomputed per frame.
    #[gpui::test]
    async fn hovering_a_handle_of_the_single_selected_node_caches_a_resize_cursor(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let mut doc = doc_with_one_page();
        let page = doc.active_page().expect("active page");
        let mut rect = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            100.0,
            50.0,
            Color::BLACK,
        )));
        rect.parent = Some(page);
        let rect_id = rect.id;
        doc.apply(Operation::create_node(rect))
            .expect("create rect");
        doc.selection.add(rect_id);
        let item = crate::document::ready_item_for_test(
            &project,
            std::path::PathBuf::from("/tmp/Design.fig"),
            doc,
            cx,
        );
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let view = scratch
            .update(cx, |_, window, cx| {
                cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
            })
            .expect("create fig view");

        let screen_size = DVec2::new(800.0, 600.0);
        let viewport = Viewport {
            center: [0.0, 0.0],
            zoom: 1.0,
        };
        view.update(cx, |view, _| {
            view.viewport = Some(viewport);
            view.container_bounds = Some(Bounds {
                origin: point(px(0.), px(0.)),
                size: size(px(800.), px(600.)),
            });
        });
        let world = item
            .read_with(cx, |item, _| {
                item.doc().and_then(|doc| doc.scene.world_bounds(rect_id))
            })
            .expect("world bounds");

        for handle in fanta_canvas::ResizeHandle::ALL {
            let at = fanta_canvas::handles::handle_screen_position(
                handle,
                world,
                &viewport,
                screen_size,
            );
            view.update(cx, |view, cx| view.update_hover_resize_handle(at, cx));
            view.read_with(cx, |view, _| {
                assert_eq!(view.hover_resize_handle, Some(handle));
            });
        }

        // The middle of the node is not a handle.
        let north_west = fanta_canvas::handles::handle_screen_position(
            fanta_canvas::ResizeHandle::NorthWest,
            world,
            &viewport,
            screen_size,
        );
        let south_east = fanta_canvas::handles::handle_screen_position(
            fanta_canvas::ResizeHandle::SouthEast,
            world,
            &viewport,
            screen_size,
        );
        view.update(cx, |view, cx| {
            view.update_hover_resize_handle((north_west + south_east) / 2.0, cx)
        });
        view.read_with(cx, |view, _| assert!(view.hover_resize_handle.is_none()));

        // Two selected nodes: the Select tool would move them, not resize.
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let page = document.doc.active_page().expect("active page");
                document.doc.selection.add(page);
                ((), DocChange::Selection)
            });
        });
        let corner = fanta_canvas::handles::handle_screen_position(
            fanta_canvas::ResizeHandle::NorthWest,
            world,
            &viewport,
            screen_size,
        );
        view.update(cx, |view, cx| view.update_hover_resize_handle(corner, cx));
        view.read_with(cx, |view, _| assert!(view.hover_resize_handle.is_none()));
    }

    /// The hint is the only thing on an empty canvas, so it must disappear the
    /// moment the page has content — and never claim an unloaded document is
    /// empty.
    #[gpui::test]
    async fn the_empty_page_hint_tracks_the_active_pages_children(cx: &mut TestAppContext) {
        init_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let item = crate::document::ready_item_for_test(
            &project,
            std::path::PathBuf::from("/tmp/Design.fig"),
            doc_with_one_page(),
            cx,
        );
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let view = scratch
            .update(cx, |_, window, cx| {
                cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
            })
            .expect("create fig view");

        view.read_with(cx, |view, cx| assert!(view.active_page_is_empty(cx)));
        add_rect(&item, cx);
        view.read_with(cx, |view, cx| assert!(!view.active_page_is_empty(cx)));
    }

    #[test]
    fn every_selection_handle_maps_to_its_own_resize_cursor() {
        use fanta_canvas::ResizeHandle;
        assert_eq!(
            resize_cursor(ResizeHandle::North),
            CursorStyle::ResizeUpDown
        );
        assert_eq!(
            resize_cursor(ResizeHandle::South),
            CursorStyle::ResizeUpDown
        );
        assert_eq!(
            resize_cursor(ResizeHandle::East),
            CursorStyle::ResizeLeftRight
        );
        assert_eq!(
            resize_cursor(ResizeHandle::West),
            CursorStyle::ResizeLeftRight
        );
        assert_eq!(
            resize_cursor(ResizeHandle::NorthWest),
            CursorStyle::ResizeUpLeftDownRight
        );
        assert_eq!(
            resize_cursor(ResizeHandle::SouthEast),
            CursorStyle::ResizeUpLeftDownRight
        );
        assert_eq!(
            resize_cursor(ResizeHandle::NorthEast),
            CursorStyle::ResizeUpRightDownLeft
        );
        assert_eq!(
            resize_cursor(ResizeHandle::SouthWest),
            CursorStyle::ResizeUpRightDownLeft
        );
    }

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

    #[test]
    fn motion_values_are_sampled_from_authored_node_state() {
        let mut node = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            20.0,
            20.0,
            Color::rgb(12, 34, 56),
        )));
        node.transform = fanta_doc::Transform2D::translation(12.0, 34.0);

        assert_eq!(
            motion_value(&node, MotionProperty::PositionX),
            Some(ResolvedVarValue::Float { value: 12.0 })
        );
        assert_eq!(
            motion_property(TimelineProperty::ScaleX),
            MotionProperty::ScaleX
        );
        assert_eq!(
            motion_property(TimelineProperty::ScaleY),
            MotionProperty::ScaleY
        );
        assert_eq!(
            motion_value(
                &node,
                MotionProperty::bound(BoundProp::FillColor { index: 0 })
            ),
            Some(ResolvedVarValue::Color {
                value: Color::rgb(12, 34, 56)
            })
        );
    }

    #[test]
    fn motion_keyframes_sample_the_variable_resolved_value() {
        let collection_id = VariableCollectionId::new();
        let mode_id = ModeId::new();
        let variable_id = VariableId::new();
        let mut doc = fanta_doc::Doc::new();
        doc.variables.collections.insert(
            collection_id,
            VariableCollection {
                id: collection_id,
                name: "Theme".to_owned(),
                modes: vec![Mode {
                    id: mode_id,
                    name: "Default".to_owned(),
                }],
                default_mode: mode_id,
                variable_order: vec![variable_id],
            },
        );
        doc.variables.variables.insert(
            variable_id,
            Variable {
                id: variable_id,
                collection: collection_id,
                name: "Accent".to_owned(),
                ty: VariableType::Color,
                values_by_mode: BTreeMap::from([(
                    mode_id,
                    VarValue::Color {
                        value: Color::rgb(255, 0, 0),
                    },
                )]),
                scopes: Vec::new(),
            },
        );
        let mut node = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            20.0,
            20.0,
            Color::WHITE,
        )));
        node.bindings
            .insert(BoundProp::FillColor { index: 0 }, variable_id);
        let node_id = node.id;
        doc.apply(Operation::create_node(node))
            .expect("create bound node");

        let resolved = motion_source_node(&doc, node_id).expect("resolved node");
        assert_eq!(
            motion_value(
                &resolved,
                MotionProperty::bound(BoundProp::FillColor { index: 0 })
            ),
            Some(ResolvedVarValue::Color {
                value: Color::rgb(255, 0, 0)
            })
        );
    }

    #[test]
    fn timeline_model_projects_engine_milliseconds_to_ui_microseconds() {
        let mut doc = fanta_doc::Doc::new();
        let mut node = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            20.0,
            20.0,
            Color::WHITE,
        )));
        node.name = "Star".to_owned();
        let node_id = node.id;
        doc.apply(Operation::create_node(node))
            .expect("create motion target");

        let clip_id = AnimationClipId::new();
        let track_id = AnimationTrackId::new();
        let mut track = AnimationTrack::new(
            track_id,
            MotionTarget::new(node_id, MotionProperty::PositionX),
        );
        let keyframe_id = KeyframeId::new();
        let mut keyframe = Keyframe::new(keyframe_id, 250, ResolvedVarValue::Float { value: 10.0 });
        keyframe.interpolation = Interpolation::Hold;
        keyframe.easing = Easing::CubicBezier {
            x1: 0.25,
            y1: -0.5,
            x2: 0.75,
            y2: 1.5,
        };
        track.keyframes.insert(keyframe_id, keyframe);
        let early_keyframe_id = KeyframeId::new();
        track.keyframes.insert(
            early_keyframe_id,
            Keyframe::new(
                early_keyframe_id,
                100,
                ResolvedVarValue::Float { value: 2.0 },
            ),
        );
        let mut clip = AnimationClip::new(clip_id, "Entrance", 1_500);
        clip.tracks.insert(track_id, track);
        doc.motion.clips.insert(clip_id, clip);

        let model = motion_timeline_model(&doc, Some(clip_id));
        assert_eq!(model.clip_name.as_deref(), Some("Entrance"));
        assert_eq!(model.duration_us, 1_500_000);
        assert_eq!(model.tracks.len(), 1);
        assert_eq!(model.tracks[0].node_id, node_id);
        assert_eq!(model.tracks[0].label.as_ref(), "Star · Position X");
        assert_eq!(model.tracks[0].keyframes.len(), 2);
        assert_eq!(
            model.tracks[0].keyframes[0].id.as_ref(),
            early_keyframe_id.to_string()
        );
        assert_eq!(model.tracks[0].keyframes[0].time_us, 100_000);
        assert_eq!(
            model.tracks[0].keyframes[1].id.as_ref(),
            keyframe_id.to_string()
        );
        assert_eq!(model.tracks[0].keyframes[1].time_us, 250_000);
        assert_eq!(
            model.tracks[0].keyframes[1].interpolation,
            Interpolation::Hold
        );
        assert_eq!(
            model.tracks[0].keyframes[1].easing,
            Easing::CubicBezier {
                x1: 0.25,
                y1: -0.5,
                x2: 0.75,
                y2: 1.5,
            }
        );
    }

    fn doc_with_two_pages() -> (fanta_doc::Doc, NodeId, NodeId) {
        let mut doc = doc_with_one_page();
        let page_one = doc.active_page().expect("page one");
        let mut second = CanvasNode::new(NodeData::Group(GroupNode::default()));
        second.name = "Page 2".to_owned();
        let page_two = second.id;
        doc.apply(Operation::create_node(second))
            .expect("create page two");
        doc.add_page(page_two);
        (doc, page_one, page_two)
    }

    fn group_under(doc: &mut fanta_doc::Doc, parent: NodeId) -> NodeId {
        let mut group = CanvasNode::new(NodeData::Group(GroupNode::default()));
        group.parent = Some(parent);
        let id = group.id;
        doc.apply(Operation::create_node(group))
            .expect("create group");
        id
    }

    #[test]
    fn structure_commands_ignore_layers_left_on_another_page() {
        let (mut doc, page_one, page_two) = doc_with_two_pages();
        let on_page_one = group_under(&mut doc, page_one);
        doc.selection.replace_with(vec![on_page_one]);
        assert_eq!(structure_targets(&doc, None), vec![on_page_one]);

        doc.set_active_page(Some(page_two));
        assert!(
            structure_targets(&doc, None).is_empty(),
            "a selection left on page one is not grouped from page two"
        );
        assert!(structure_targets(&doc, Some(on_page_one)).is_empty());

        let on_page_two = group_under(&mut doc, page_two);
        doc.selection.replace_with(vec![on_page_one, on_page_two]);
        assert_eq!(
            structure_targets(&doc, None),
            vec![on_page_two],
            "a selection spanning pages keeps only this page's layers"
        );
    }

    #[test]
    fn ungrouping_skips_component_masters_and_page_roots() {
        let (mut doc, page_one, _page_two) = doc_with_two_pages();
        let plain = group_under(&mut doc, page_one);
        let master = group_under(&mut doc, page_one);
        let component = fanta_doc::ComponentId::new();
        doc.components.defs.insert(
            component,
            fanta_doc::ComponentDef::new(component, master, "Card"),
        );
        assert_eq!(ungroupable_targets(&doc, &[plain, master]), vec![plain]);
        assert!(ungroupable_targets(&doc, &[master, page_one]).is_empty());
    }

    /// The headline new-tab walk: tab A shows page 1 zoomed and still reports
    /// `is_focused` when the scoped open of `pages/<p2>/page.fnx` re-targets
    /// the shared document (focus listeners only run at the next draw, so the
    /// flag is stale at event delivery). Tab A must keep its viewport and
    /// page mirrors — both at the scoped open and when focus later returns
    /// and re-asserts its own scope.
    #[gpui::test]
    async fn a_scoped_open_for_a_new_tab_never_clobbers_the_previous_tabs_viewport(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let (doc, page_one, page_two) = doc_with_two_pages();
        let item = crate::document::ready_item_for_test(
            &project,
            std::path::PathBuf::from("/tmp/Design.fig"),
            doc,
            cx,
        );
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let tab_a = scratch
            .update(cx, |_, window, cx| {
                cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
            })
            .expect("tab A");
        let zoomed = Viewport {
            center: [120.0, -40.0],
            zoom: 3.0,
        };
        tab_a.update(cx, |view, _| {
            view.is_focused = true;
            view.scope = Some(FigScope::Page(page_one));
            view.selected_page_index = Some(0);
            view.selected_page_root = Some(page_one);
            view.viewport = Some(zoomed);
        });

        // The scoped open's `try_open` runs before any draw delivers tab A's
        // focus-out, so A still reports `is_focused` when this event lands.
        item.update(cx, |item, cx| {
            item.request_scope(FigScope::Page(page_two), ScopeRequester::Open, cx);
        });
        cx.run_until_parked();
        tab_a.read_with(cx, |view, _| {
            assert_eq!(
                view.viewport,
                Some(zoomed),
                "a sibling's scoped open must not clobber tab A's viewport"
            );
            assert_eq!(view.last_seen_root, Some(page_one));
            assert_eq!(view.selected_page_index, Some(0));
            assert_eq!(view.selected_page_root, Some(page_one));
        });

        // Focus returns to tab A: its on-focus re-assert re-roots the shared
        // document back to page 1. Restoring our OWN root must keep the
        // saved viewport.
        tab_a.update(cx, |view, cx| {
            let requester = ScopeRequester::View(cx.entity_id());
            view.item.update(cx, |item, cx| {
                item.request_scope(FigScope::Page(page_one), requester, cx)
            });
        });
        cx.run_until_parked();
        tab_a.read_with(cx, |view, cx| {
            assert_eq!(
                view.viewport,
                Some(zoomed),
                "switching back to tab A must keep its zoom"
            );
            assert_eq!(
                view.item.read(cx).doc().and_then(|doc| doc.active_page()),
                Some(page_one),
                "the re-assert re-roots the shared document to tab A's page"
            );
        });
    }

    /// The load-time apply carries no requesting view (the scoped tab was
    /// created while the document was still loading), so the focused view
    /// must still follow it.
    #[gpui::test]
    async fn the_load_time_scope_apply_still_refits_the_focused_view(cx: &mut TestAppContext) {
        init_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let (doc, _page_one, page_two) = doc_with_two_pages();
        let item = crate::document::ready_item_for_test(
            &project,
            std::path::PathBuf::from("/tmp/Design.fig"),
            doc,
            cx,
        );
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let view = scratch
            .update(cx, |_, window, cx| {
                cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
            })
            .expect("view");
        view.update(cx, |view, _| {
            view.is_focused = true;
            view.viewport = Some(Viewport {
                center: [10.0, 20.0],
                zoom: 2.0,
            });
        });

        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.set_active_page(Some(page_two));
                ((), DocChange::Selection)
            });
            cx.emit(FigItemEvent::ScopeApplied(
                FigScope::Page(page_two),
                ScopeRequester::Load,
            ));
        });
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert_eq!(
                view.viewport, None,
                "the focused view refits to the load-applied scope"
            );
            assert_eq!(view.last_seen_root, Some(page_two));
            assert_eq!(view.selected_page_root, Some(page_two));
            assert_eq!(view.selected_page_index, Some(1));
        });
    }

    #[gpui::test]
    async fn motion_panel_clip_selection_synchronizes_the_timeline(cx: &mut TestAppContext) {
        init_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let mut doc = doc_with_one_page();
        let first_clip = AnimationClipId::from_u128(100);
        let second_clip = AnimationClipId::from_u128(200);
        doc.motion.clips.insert(
            first_clip,
            AnimationClip::new(first_clip, "Entrance", 1_000),
        );
        doc.motion
            .clips
            .insert(second_clip, AnimationClip::new(second_clip, "Exit", 2_000));
        let item = crate::document::ready_item_for_test(
            &project,
            std::path::PathBuf::from("/tmp/Design.fig"),
            doc,
            cx,
        );
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let view = scratch
            .update(cx, |_, window, cx| {
                cx.new(|cx| FigView::new(item, project, window, cx))
            })
            .expect("scratch window");
        let motion_sidebar = view.read_with(cx, |view, _| view.motion_sidebar.clone());

        motion_sidebar.update(cx, |_, cx| {
            cx.emit(MotionPanelEvent::SelectClip(second_clip));
        });
        cx.run_until_parked();

        view.read_with(cx, |view, cx| {
            assert_eq!(view.active_motion_clip, Some(second_clip));
            assert_eq!(
                view.timeline_shell
                    .read(cx)
                    .view_model()
                    .clip_name
                    .as_deref(),
                Some("Exit")
            );
        });
    }

    #[gpui::test]
    async fn timeline_drag_previews_realtime_and_commits_one_undo_step(cx: &mut TestAppContext) {
        init_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let mut doc = doc_with_one_page();
        let clip_id = AnimationClipId::from_u128(100);
        let track_id = AnimationTrackId::from_u128(101);
        let keyframe_id = KeyframeId::from_u128(102);
        let target = MotionTarget::new(
            doc.active_page().expect("active page"),
            MotionProperty::PositionX,
        );
        let mut track = AnimationTrack::new(track_id, target);
        track.keyframes.insert(
            keyframe_id,
            Keyframe::new(keyframe_id, 250, ResolvedVarValue::Float { value: 10.0 }),
        );
        let mut clip = AnimationClip::new(clip_id, "Entrance", 1_000);
        clip.tracks.insert(track_id, track);
        doc.motion.clips.insert(clip_id, clip);
        doc.history = Default::default();
        let item = crate::document::ready_item_for_test(
            &project,
            std::path::PathBuf::from("/tmp/Design.fig"),
            doc,
            cx,
        );
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let view = scratch
            .update(cx, |_, window, cx| {
                cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
            })
            .expect("scratch window");
        let selection = TimelineKeyframeSelection {
            track_id: track_id.to_string().into(),
            keyframe_id: keyframe_id.to_string().into(),
        };

        view.update(cx, |view, cx| {
            view.active_motion_clip = Some(clip_id);
            view.begin_motion_keyframe_drag(&selection, cx);
            view.preview_motion_keyframe_drag(&selection, 700_000, cx);
        });
        item.read_with(cx, |item, _| {
            let doc = item.doc().expect("ready document");
            assert_eq!(doc.history.undo_depth(), 0);
            assert_eq!(
                doc.motion.clips[&clip_id].tracks[&track_id].keyframes[&keyframe_id].time_ms,
                700
            );
        });

        view.update(cx, |view, cx| {
            view.commit_motion_keyframe_drag(&selection, 700_000, cx)
        });
        item.update(cx, |item, cx| {
            let doc = item.doc().expect("ready document");
            assert_eq!(doc.history.undo_depth(), 1);
            assert_eq!(
                doc.motion.clips[&clip_id].tracks[&track_id].keyframes[&keyframe_id].time_ms,
                700
            );
            assert!(item.undo(cx).expect("undo motion drag"));
            let doc = item.doc().expect("ready document");
            assert_eq!(
                doc.motion.clips[&clip_id].tracks[&track_id].keyframes[&keyframe_id].time_ms,
                250
            );
        });

        view.update(cx, |view, cx| {
            view.set_editor_mode(EditorMode::Motion, cx);
            view.begin_motion_keyframe_drag(&selection, cx);
            view.preview_motion_keyframe_drag(&selection, 800_000, cx);
        });
        item.update(cx, |item, cx| item.set_source_edit_locked(true, cx));
        cx.run_until_parked();
        view.read_with(cx, |view, cx| {
            assert!(view.motion_keyframe_drag.is_none());
            assert!(!view.timeline_shell.read(cx).authoring_enabled());
        });
        item.read_with(cx, |item, _| {
            assert_eq!(
                item.doc().expect("ready document").motion.clips[&clip_id].tracks[&track_id]
                    .keyframes[&keyframe_id]
                    .time_ms,
                250
            );
        });
        item.update(cx, |item, cx| item.set_source_edit_locked(false, cx));
        cx.run_until_parked();
        view.read_with(cx, |view, cx| {
            assert!(view.timeline_shell.read(cx).authoring_enabled());
        });
    }

    #[gpui::test]
    async fn mode_and_workspace_switches_finish_custom_easing_edits(cx: &mut TestAppContext) {
        init_visual_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let mut doc = doc_with_one_page();
        let clip_id = AnimationClipId::from_u128(200);
        let track_id = AnimationTrackId::from_u128(201);
        let keyframe_id = KeyframeId::from_u128(202);
        let target = MotionTarget::new(
            doc.active_page().expect("active page"),
            MotionProperty::PositionX,
        );
        let original = Easing::CubicBezier {
            x1: 0.42,
            y1: 0.0,
            x2: 0.58,
            y2: 1.0,
        };
        let committed = Easing::CubicBezier {
            x1: 0.1,
            y1: -2.0,
            x2: 0.9,
            y2: 3.0,
        };
        let mut keyframe = Keyframe::new(keyframe_id, 250, ResolvedVarValue::Float { value: 10.0 });
        keyframe.easing = original;
        let mut track = AnimationTrack::new(track_id, target);
        track.keyframes.insert(keyframe_id, keyframe);
        let mut clip = AnimationClip::new(clip_id, "Entrance", 1_000);
        clip.tracks.insert(track_id, track);
        doc.motion.clips.insert(clip_id, clip);
        doc.history = Default::default();
        let item = crate::document::ready_item_for_test(
            &project,
            std::path::PathBuf::from("/tmp/Custom-easing.fig"),
            doc,
            cx,
        );
        let item_for_window = item.clone();
        let window =
            cx.add_window(move |window, cx| FigView::new(item_for_window, project, window, cx));
        let view = window.entity(cx).expect("fig view");
        let selection = TimelineKeyframeSelection {
            track_id: track_id.to_string().into(),
            keyframe_id: keyframe_id.to_string().into(),
        };

        window
            .update(cx, |view, window, cx| {
                view.active_motion_clip = Some(clip_id);
                view.set_editor_mode(EditorMode::Motion, cx);
                let timeline = view.timeline_shell.clone();
                timeline.update(cx, |timeline, cx| {
                    timeline.begin_easing_edit_for_test(selection.clone(), original, window, cx)
                });
            })
            .expect("begin valid easing edit");
        cx.run_until_parked();
        window
            .update(cx, |view, window, cx| {
                view.timeline_shell.update(cx, |timeline, cx| {
                    assert!(timeline.set_easing_edit_text_for_test("0.1, -2, 0.9, 3", window, cx));
                });
            })
            .expect("preview valid easing edit");
        cx.run_until_parked();
        view.update(cx, |view, cx| view.set_editor_mode(EditorMode::Design, cx));

        item.read_with(cx, |item, _| {
            let doc = item.doc().expect("ready document");
            assert_eq!(
                doc.motion.clips[&clip_id].tracks[&track_id].keyframes[&keyframe_id].easing,
                committed
            );
            assert_eq!(doc.history.undo_depth(), 1);
        });

        window
            .update(cx, |view, window, cx| {
                view.set_editor_mode(EditorMode::Motion, cx);
                let timeline = view.timeline_shell.clone();
                timeline.update(cx, |timeline, cx| {
                    timeline.begin_easing_edit_for_test(selection.clone(), committed, window, cx)
                });
            })
            .expect("begin invalid easing edit");
        cx.run_until_parked();
        window
            .update(cx, |view, window, cx| {
                view.timeline_shell.update(cx, |timeline, cx| {
                    assert!(timeline.set_easing_edit_text_for_test("invalid", window, cx));
                });
                view.set_editor_workspace(EditorWorkspace::Variables, cx);
            })
            .expect("finish invalid easing edit on workspace switch");

        item.read_with(cx, |item, _| {
            let doc = item.doc().expect("ready document");
            assert_eq!(
                doc.motion.clips[&clip_id].tracks[&track_id].keyframes[&keyframe_id].easing,
                committed
            );
            assert_eq!(doc.history.undo_depth(), 1);
        });
        view.read_with(cx, |view, cx| {
            assert!(view.motion_keyframe_drag.is_none());
            assert_eq!(view.editor_workspace(cx), EditorWorkspace::Variables);
        });
    }

    #[gpui::test]
    async fn motion_timeline_spans_below_both_editor_sidebars(cx: &mut TestAppContext) {
        init_visual_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let item = crate::document::ready_item_for_test(
            &project,
            std::path::PathBuf::from("/tmp/Motion-layout.fig"),
            doc_with_one_page(),
            cx,
        );
        let window = cx.add_window(move |window, cx| FigView::new(item, project, window, cx));
        let view = window.entity(cx).expect("fig view");
        view.update(cx, |view, cx| {
            view.set_editor_mode(EditorMode::Motion, cx);
        });

        let mut visual_context = gpui::VisualTestContext::from_window(window.into(), cx);
        visual_context.simulate_resize(size(px(1_200.), px(800.)));
        visual_context.update(|window, cx| window.draw(cx).clear());

        let layers = visual_context
            .debug_bounds("fanta-layers-sidebar")
            .expect("layers sidebar");
        let inspector = visual_context
            .debug_bounds("fanta-inspector-sidebar")
            .expect("inspector sidebar");
        let timeline = visual_context
            .debug_bounds("fanta-motion-timeline-shell")
            .expect("motion timeline");
        let motion_panel = visual_context
            .debug_bounds("fanta-motion-panel")
            .expect("motion inspector panel");
        let close = |left: Pixels, right: Pixels| (left - right).abs() <= px(1.);

        assert!(close(timeline.left(), layers.left()));
        assert!(close(timeline.right(), inspector.right()));
        assert!(close(timeline.top(), layers.bottom()));
        assert!(close(timeline.top(), inspector.bottom()));
        assert_eq!(timeline.size.height, TIMELINE_HEIGHT);
        assert!(close(motion_panel.left(), inspector.left()));
        assert!(close(motion_panel.right(), inspector.right()));
        assert!(close(motion_panel.bottom(), inspector.bottom()));
        assert!(motion_panel.top() > inspector.top());
    }

    /// Select All takes the active page's own children — never the page
    /// roots, which is what an unscoped `children_of(None)` would return.
    #[gpui::test]
    async fn select_all_selects_the_active_pages_top_level_nodes(cx: &mut TestAppContext) {
        init_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let mut doc = doc_with_one_page();
        let page = doc.active_page().expect("active page");
        let mut first = CanvasNode::new(NodeData::Group(GroupNode::default()));
        first.parent = Some(page);
        let first_id = first.id;
        doc.apply(Operation::create_node(first))
            .expect("create first node");
        let mut second = CanvasNode::new(NodeData::Group(GroupNode::default()));
        second.parent = Some(page);
        let second_id = second.id;
        doc.apply(Operation::create_node(second))
            .expect("create second node");
        let item = crate::document::ready_item_for_test(
            &project,
            std::path::PathBuf::from("/tmp/Design.fig"),
            doc,
            cx,
        );
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let view = scratch
            .update(cx, |_, window, cx| {
                cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
            })
            .expect("create fig view");

        scratch
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| view.select_all(&SelectAll, window, cx));
            })
            .expect("select all");

        view.read_with(cx, |view, cx| {
            let mut selection = view
                .item
                .read(cx)
                .doc()
                .expect("document")
                .selection
                .as_slice()
                .to_vec();
            let mut expected = vec![first_id, second_id];
            // The selection does not promise insertion order, so compare as sets.
            selection.sort();
            expected.sort();
            assert_eq!(selection, expected);
        });
    }

    #[gpui::test]
    async fn tool_activation_updates_before_canvas_viewport_exists(cx: &mut TestAppContext) {
        init_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let item = crate::document::ready_item_for_test(
            &project,
            std::path::PathBuf::from("/tmp/Design.fig"),
            doc_with_one_page(),
            cx,
        );
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let view = scratch
            .update(cx, |_, window, cx| {
                cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
            })
            .unwrap();

        view.update(cx, |view, cx| {
            assert_eq!(view.active_tool(), ToolKind::Select);
            view.activate_tool(ToolKind::Text, cx);
            assert_eq!(view.active_tool(), ToolKind::Text);
        });
    }

    #[gpui::test]
    async fn prototype_presentation_navigates_and_restores_the_editor_viewport(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let mut doc = doc_with_one_page();
        let page = doc.active_page().expect("active page");
        let mut first = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([320.0, 180.0]),
            ..GroupNode::default()
        }));
        first.parent = Some(page);
        first.name = "First".into();
        doc.apply(Operation::create_node(first))
            .expect("create first prototype frame");
        let mut second = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([320.0, 180.0]),
            ..GroupNode::default()
        }));
        second.parent = Some(page);
        second.name = "Second".into();
        second.transform = Transform2D::translation(400.0, 0.0);
        doc.apply(Operation::create_node(second))
            .expect("create second prototype frame");
        let ordered_frames = doc.scene.children_of(Some(page)).to_vec();
        let start_id = ordered_frames[0];
        let next_id = ordered_frames[1];
        doc.apply(Operation::SetFlowStart {
            old: doc.flow_start(),
            new: Some(start_id),
        })
        .expect("set prototype starting point");
        doc.history = Default::default();
        let item = crate::document::ready_item_for_test(
            &project,
            std::path::PathBuf::from("/tmp/Prototype.fig"),
            doc,
            cx,
        );
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let view = scratch
            .update(cx, |_, window, cx| {
                cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
            })
            .expect("create fig view");
        let editor_viewport = Viewport {
            center: [42.0, 24.0],
            zoom: 1.5,
        };

        scratch
            .update(cx, |_, window, cx| {
                view.update(cx, |view, cx| {
                    view.viewport = Some(editor_viewport);
                    view.play_prototype(&PlayPrototype, window, cx);
                });
            })
            .expect("start prototype presentation");
        view.read_with(cx, |view, cx| {
            assert_eq!(view.editor_mode(cx), EditorMode::Prototype);
            assert_eq!(
                view.prototype_player
                    .as_ref()
                    .map(PrototypePlayerState::current_frame),
                Some(start_id)
            );
        });

        view.update(cx, |view, cx| view.show_next_prototype_frame(cx));
        view.read_with(cx, |view, _| {
            assert_eq!(
                view.prototype_player
                    .as_ref()
                    .map(PrototypePlayerState::current_frame),
                Some(next_id)
            );
        });

        view.update(cx, |view, cx| view.exit_prototype_session(cx));
        view.read_with(cx, |view, _| {
            assert!(view.prototype_player.is_none());
            let restored = view.viewport.expect("restored editor viewport");
            assert_eq!(restored.center, editor_viewport.center);
            assert_eq!(restored.zoom, editor_viewport.zoom);
        });
    }

    #[gpui::test]
    async fn canvas_copy_cut_paste_and_duplicate_preserve_subtrees_and_history(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let mut doc = doc_with_one_page();
        let page = doc.active_page().expect("active page");
        let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([160.0, 90.0]),
            ..Default::default()
        }));
        frame.parent = Some(page);
        frame.transform = Transform2D::translation(30.0, 40.0);
        let frame_id = frame.id;
        doc.apply(Operation::create_node(frame)).unwrap();
        let mut text = CanvasNode::new(NodeData::Text(TextNode::new("Clipboard", 100.0, 30.0)));
        text.parent = Some(frame_id);
        text.transform = Transform2D::translation(12.0, 16.0);
        doc.apply(Operation::create_node(text)).unwrap();
        doc.selection.select_only(frame_id);
        doc.history = Default::default();
        let item = crate::document::ready_item_for_test(
            &project,
            std::path::PathBuf::from("/tmp/Clipboard.fig"),
            doc,
            cx,
        );
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let view = scratch
            .update(cx, |_, window, cx| {
                cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
            })
            .expect("create fig view");

        view.update(cx, |view, cx| view.copy_selected_nodes(cx));
        let copied = cx
            .read_from_clipboard()
            .and_then(|clipboard| {
                clipboard.entries.into_iter().find_map(|entry| match entry {
                    ClipboardEntry::String(string) => string.metadata_json::<CanvasClipboard>(),
                    ClipboardEntry::Image(_) | ClipboardEntry::ExternalPaths(_) => None,
                })
            })
            .expect("canvas clipboard metadata");
        item.read_with(cx, |item, _| {
            assert_eq!(copied.display_text(), "Group");
            assert_eq!(item.doc().unwrap().scene.len(), 3);
        });

        view.update(cx, |view, cx| view.duplicate_selected_nodes(cx));
        let duplicate_id = item.read_with(cx, |item, _| {
            let doc = item.doc().unwrap();
            assert_eq!(doc.scene.len(), 5);
            assert_eq!(doc.history.undo_depth(), 1);
            doc.selection.as_slice()[0]
        });
        assert_ne!(duplicate_id, frame_id);
        item.update(cx, |item, cx| {
            assert!(item.undo(cx).expect("undo duplicate"));
            let doc = item.doc().unwrap();
            assert!(!doc.scene.contains(duplicate_id));
            assert!(
                doc.selection.is_empty(),
                "undo prunes deleted selection ids"
            );
            assert!(item.redo(cx).expect("redo duplicate"));
            assert!(item.doc().unwrap().scene.contains(duplicate_id));
        });

        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.selection.select_only(frame_id);
                ((), DocChange::Selection)
            });
        });
        view.update(cx, |view, cx| view.cut_selected_nodes(cx));
        item.read_with(cx, |item, _| {
            assert!(!item.doc().unwrap().scene.contains(frame_id));
        });
        view.update(cx, |view, cx| view.paste_selected_nodes(cx));
        item.read_with(cx, |item, _| {
            let doc = item.doc().unwrap();
            assert_eq!(doc.selection.len(), 1);
            let pasted = doc.selection.as_slice()[0];
            assert_ne!(pasted, frame_id);
            assert_eq!(doc.scene.children_of(Some(pasted)).len(), 1);
        });
    }

    #[gpui::test]
    async fn standalone_text_selects_on_single_click_and_edits_on_double_click(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let (doc, text_id, _) = text_selection_doc(false);
        let item = crate::document::ready_item_for_test(
            &project,
            std::path::PathBuf::from("/tmp/StandaloneText.fig"),
            doc,
            cx,
        );
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let view = scratch
            .update(cx, |_, window, cx| {
                cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
            })
            .expect("create fig view");
        view.update(cx, |view, _| {
            view.set_container_bounds(Bounds {
                origin: point(px(0.0), px(0.0)),
                size: size(px(800.0), px(600.0)),
            });
            view.set_viewport_silent(Viewport::default());
        });
        let position = point(px(360.0), px(295.0));

        send_canvas_click(scratch, &view, position, 1, cx);
        item.read_with(cx, |item, _| {
            assert_eq!(
                item.doc().expect("document").selection.as_slice(),
                &[text_id]
            );
        });
        view.read_with(cx, |view, _| assert!(view.text_edit.is_none()));

        send_canvas_click(scratch, &view, position, 2, cx);
        view.read_with(cx, |view, _| {
            let edit = view.text_edit.as_ref().expect("text editor opened");
            let range = edit.session.selected_range();
            assert!(range.start < range.end, "double-click selects a word");
            assert!(range.end - range.start < "Hello world".len());
        });
    }

    #[gpui::test]
    async fn wrapped_text_requires_drill_in_before_a_second_double_click_edits(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let (doc, text_id, frame) = text_selection_doc(true);
        let frame = frame.expect("text wrapper");
        let item = crate::document::ready_item_for_test(
            &project,
            std::path::PathBuf::from("/tmp/WrappedText.fig"),
            doc,
            cx,
        );
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let view = scratch
            .update(cx, |_, window, cx| {
                cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
            })
            .expect("create fig view");
        view.update(cx, |view, _| {
            view.set_container_bounds(Bounds {
                origin: point(px(0.0), px(0.0)),
                size: size(px(800.0), px(600.0)),
            });
            view.set_viewport_silent(Viewport::default());
        });
        let position = point(px(340.0), px(285.0));

        send_canvas_click(scratch, &view, position, 1, cx);
        item.read_with(cx, |item, _| {
            assert_eq!(item.doc().expect("document").selection.as_slice(), &[frame]);
        });

        send_canvas_click(scratch, &view, position, 2, cx);
        item.read_with(cx, |item, _| {
            assert_eq!(
                item.doc().expect("document").selection.as_slice(),
                &[text_id]
            );
        });
        view.read_with(cx, |view, _| {
            assert!(
                view.text_edit.is_none(),
                "the first double-click only drills into the wrapper"
            );
        });

        send_canvas_click(scratch, &view, position, 3, cx);
        item.read_with(cx, |item, _| {
            assert_eq!(
                item.doc().expect("document").selection.as_slice(),
                &[text_id]
            );
        });
        view.read_with(cx, |view, _| {
            assert!(
                view.text_edit.is_none(),
                "the first click in the second pair keeps the text selected"
            );
        });

        send_canvas_click(scratch, &view, position, 4, cx);
        view.read_with(cx, |view, _| {
            let edit = view
                .text_edit
                .as_ref()
                .expect("second double-click opens text editing");
            let range = edit.session.selected_range();
            assert!(range.start < range.end, "word selection stays intact");
        });
    }

    #[gpui::test]
    async fn text_tool_click_creates_text_and_queues_inline_edit(cx: &mut TestAppContext) {
        init_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let item = crate::document::ready_item_for_test(
            &project,
            std::path::PathBuf::from("/tmp/Design.fig"),
            doc_with_one_page(),
            cx,
        );
        let item_for_assert = item.clone();
        let scratch = cx.add_window(|_, _| gpui::Empty);
        let view = scratch
            .update(cx, |_, window, cx| {
                cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
            })
            .unwrap();

        view.update(cx, |view, cx| {
            view.set_container_bounds(Bounds {
                origin: point(px(0.0), px(0.0)),
                size: size(px(800.0), px(600.0)),
            });
            view.set_viewport_silent(Viewport::default());
            view.activate_tool(ToolKind::Text, cx);
            view.dispatch_tool_event(
                press_event(
                    DVec2::new(400.0, 300.0),
                    ToolButton::Primary,
                    gpui::Modifiers::default(),
                    1,
                ),
                cx,
            );
            view.dispatch_tool_event(
                release_event(
                    DVec2::new(400.0, 300.0),
                    ToolButton::Primary,
                    gpui::Modifiers::default(),
                ),
                cx,
            );

            assert_eq!(view.active_tool(), ToolKind::Select);
            assert!(
                view.pending_text_edit.is_some(),
                "new text should be queued for inline editing"
            );
        });

        item_for_assert.read_with(cx, |item, _cx| {
            let doc = &item.document().expect("ready document").doc;
            let selected = doc.selection.as_slice();
            assert_eq!(selected.len(), 1);
            let text = doc
                .scene
                .get(selected[0])
                .and_then(|node| node.data.as_text())
                .expect("selected node should be newly created text");
            assert_eq!(text.content, fanta_tools::text::PLACEHOLDER);
        });
    }

    #[gpui::test]
    async fn canvas_create_rectangle_and_undo_restores_state(cx: &mut TestAppContext) {
        // Covers core BDD scenarios: create shapes, undo across visual ops.
        init_visual_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let initial_doc = doc_with_one_page();
        let page_id = initial_doc.active_page().expect("page");

        // Seed one page with nothing extra
        let item = crate::document::ready_item_for_test(
            &project,
            std::path::PathBuf::from("/tmp/CanvasUndo.fig"),
            initial_doc,
            cx,
        );

        let scratch = cx.add_window(|_, _| gpui::Empty);
        let _view = scratch
            .update(cx, |_, window, cx| {
                cx.new(|cx| FigView::new(item.clone(), project.clone(), window, cx))
            })
            .expect("create view");

        // Simulate a simple create via document (as higher level tools would)
        let created = cx.update(|cx| {
            let mut rect = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
                10.0,
                20.0,
                100.0,
                50.0,
                Color::rgb(100, 150, 200),
            )));
            rect.parent = Some(page_id);
            let id = rect.id;
            item.update(cx, |it, cx| {
                it.apply(Operation::create_node(rect), cx)
                    .expect("apply create");
            });
            id
        });

        // Verify node exists
        item.read_with(cx, |it, _| {
            let doc = &it.document().expect("doc").doc;
            assert!(
                doc.scene.contains(created),
                "node should exist after create"
            );
        });

        // Undo
        let did_undo = cx.update(|cx| item.update(cx, |it, cx| it.undo(cx).expect("undo")));
        assert!(did_undo, "undo should succeed");

        item.read_with(cx, |it, _| {
            let doc = &it.document().expect("doc").doc;
            assert!(
                !doc.scene.contains(created),
                "node should be gone after undo"
            );
        });
    }
}

impl FigView {
    /// The zoom the toolbar should display: live viewport zoom, or the fit
    /// zoom the canvas will initialize with before first interaction.
    #[cfg(feature = "fanta-gpui-ui")]
    fn current_zoom_percent(&self, cx: &App) -> u16 {
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
        (zoom * 100.0).round().clamp(1.0, u16::MAX as f64) as u16
    }

    /// Host state behind the toolbar's Motion/Dev/Agent option read models
    /// and its chrome capsule, snapshotted per render for the adapter's
    /// diff-guarded push.
    #[cfg(feature = "fanta-gpui-ui")]
    fn toolbar_option_inputs(
        &self,
        window: &Window,
        cx: &App,
    ) -> crate::gpui_adapters::toolbar::ToolbarOptionInputs {
        let timeline = self.timeline_shell.read(cx);
        let current_time_ms = timeline
            .playhead_us()
            .max(0)
            .div_euclid(1_000)
            .min(i64::from(u32::MAX)) as u32;
        let duration_ms = self.item.read(cx).document().and_then(|document| {
            let clip = self.active_motion_clip?;
            document.doc.motion.clip(clip).map(|clip| clip.duration_ms)
        });
        crate::gpui_adapters::toolbar::ToolbarOptionInputs {
            playing: timeline.is_playing(),
            looping: timeline.loop_playback_enabled(),
            current_time_ms,
            duration_ms,
            agent_context_label: self.toolbar_agent_context_label(cx),
            layers_sidebar_visible: self.layers_sidebar_visible,
            inspector_sidebar_visible: self.inspector_sidebar_visible,
            // Live keymap text, like the tooltip the old native button had.
            fit_to_view_shortcut: ui::text_for_action(&FitToView, window, cx).map(Into::into),
        }
    }

    /// What the Agent composer's context chip names: the selected layer, a
    /// selection count, or the current page when nothing is selected.
    #[cfg(feature = "fanta-gpui-ui")]
    fn toolbar_agent_context_label(&self, cx: &App) -> SharedString {
        let Some(document) = self.item.read(cx).document() else {
            return "Canvas".into();
        };
        match document.doc.selection.as_slice() {
            [] => document
                .page(self.selected_page_index)
                .map(|page| page.name.clone())
                .unwrap_or_else(|| "Canvas".into()),
            [node] => document
                .doc
                .scene
                .get(*node)
                .filter(|node| !node.name.is_empty())
                .map(|node| SharedString::from(node.name.clone()))
                .unwrap_or_else(|| "1 layer".into()),
            selection => format!("{} layers", selection.len()).into(),
        }
    }

    /// Apply an absolute zoom percentage through the same viewport path the
    /// scroll and keyboard zooms use, anchored at the canvas center. The
    /// factor comes from the live viewport zoom — not the rounded display
    /// percent — so ladder steps and the 100% entry land exactly.
    fn zoom_to_percent(&mut self, percent: u16, cx: &mut Context<Self>) {
        let Some(viewport) = self.viewport else {
            return;
        };
        if viewport.zoom <= 0.0 {
            return;
        }
        let target = f64::from(percent.clamp(1, 3_200)) / 100.0;
        self.zoom_by(target / viewport.zoom, None, cx);
    }

    /// Fit the viewport around the current selection: `fit_page_to_view`'s
    /// framing applied to the union of the selected nodes' world bounds.
    /// No-op when nothing is selected or no selected node has finite bounds.
    fn zoom_to_selection(&mut self, cx: &mut Context<Self>) {
        let viewport = self
            .container_bounds
            .zip(self.item.read(cx).document())
            .and_then(|(bounds, document)| {
                let mut selection_bounds: Option<fanta_doc::Bounds> = None;
                for node in document.doc.selection.iter() {
                    if let Some(node_bounds) = document.doc.scene.world_bounds(*node)
                        && node_bounds.is_finite()
                    {
                        selection_bounds = Some(match selection_bounds {
                            Some(current) => current.union(&node_bounds),
                            None => node_bounds,
                        });
                    }
                }
                Some(crate::document::fit_bounds(
                    selection_bounds?,
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

    /// Test-only view of the mounted toolbar adapter, for the echo tests in
    /// `gpui_adapters::toolbar`.
    #[cfg(all(test, feature = "fanta-gpui-ui"))]
    pub(crate) fn gpui_toolbar_adapter(
        &self,
    ) -> Option<&crate::gpui_adapters::toolbar::ToolbarAdapter> {
        self.gpui_toolbar.as_ref()
    }

    /// Test-only: whether keyboard focus sits inside the left sidebar (the
    /// Resources tile's reveal-and-focus contract).
    #[cfg(all(test, feature = "fanta-gpui-ui"))]
    pub(crate) fn layers_sidebar_is_focused(&self, window: &Window, cx: &App) -> bool {
        self.layers_sidebar
            .focus_handle(cx)
            .contains_focused(window, cx)
    }

    fn render_toolbar_slot(&self, cx: &mut Context<Self>) -> AnyElement {
        #[cfg(feature = "fanta-gpui-ui")]
        if self.gpui_toolbar.is_some() {
            return self.render_gpui_toolbar(cx);
        }
        self.render_tool_pill(cx)
    }

    /// The dock is the whole bottom overlay: fit-to-view and the sidebar
    /// toggles ride inside it as chrome controls (pushed by the adapter's
    /// refresh), and the dock surface occludes the canvas beneath it, so a
    /// press on any part of the toolbar never reaches `handle_mouse_down`.
    #[cfg(feature = "fanta-gpui-ui")]
    fn render_gpui_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(adapter) = self.gpui_toolbar.as_ref() else {
            return self.render_tool_pill(cx);
        };
        h_flex()
            .debug_selector(|| "fanta-canvas-toolbar".to_owned())
            .absolute()
            .bottom(px(12.))
            .left_0()
            .right_0()
            .px_3()
            .justify_center()
            .child(adapter.panel.clone())
            .into_any_element()
    }

    /// Resources (Figma's ⇧I panel) reveals the left pages/layers sidebar and
    /// moves focus into it. It never hides the sidebar: the dock's chrome
    /// capsule owns show/hide, and a "Resources" tile that closed the panel
    /// every other press would read as broken.
    #[cfg(feature = "fanta-gpui-ui")]
    fn reveal_layers_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.layers_sidebar_visible {
            self.toggle_layers_sidebar(&ToggleLayersSidebar, window, cx);
        }
        self.layers_sidebar.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    /// Routes a dock chrome control to the same host action the retired
    /// trailing cluster invoked; the new sidebar state echoes back through
    /// the adapter's render-time refresh.
    #[cfg(feature = "fanta-gpui-ui")]
    fn handle_toolbar_chrome_control(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::gpui_adapters::toolbar::{
            CHROME_FIT_TO_VIEW, CHROME_TOGGLE_INSPECTOR_SIDEBAR, CHROME_TOGGLE_LAYERS_SIDEBAR,
        };
        match id {
            CHROME_FIT_TO_VIEW => self.fit_to_view(&FitToView, window, cx),
            CHROME_TOGGLE_LAYERS_SIDEBAR => {
                self.toggle_layers_sidebar(&ToggleLayersSidebar, window, cx)
            }
            CHROME_TOGGLE_INSPECTOR_SIDEBAR => {
                self.toggle_inspector_sidebar(&ToggleInspectorSidebar, window, cx)
            }
            other => log::warn!("fanta-gpui toolbar: unknown chrome control {other:?}"),
        }
    }

    #[cfg(feature = "fanta-gpui-ui")]
    pub(crate) fn handle_toolbar_action(
        &mut self,
        _toolbar: &Entity<fanta_gpui::toolbar::EditorToolbar>,
        action: &fanta_gpui::toolbar::ToolbarAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use fanta_gpui::toolbar::{ToolbarAction, ToolbarCommand, ToolbarMode, ToolbarTool};
        match action {
            // Resources is host chrome, not a canvas tool: Figma's ⇧I panel
            // corresponds to the left pages/layers sidebar here.
            ToolbarAction::ToolChangeRequested {
                tool: ToolbarTool::Resources,
                ..
            } => self.reveal_layers_sidebar(window, cx),
            ToolbarAction::ToolChangeRequested { tool, .. } => {
                match crate::gpui_adapters::toolbar::tool_kind(*tool) {
                    // Text-on-path has a canvas tool object but no
                    // behavior, so activating it would
                    // arm a face that silently swallows every drag. The
                    // vendored toolbar has no host-side API to hide a tool
                    // (see `EditorToolbar`'s setters), so say so instead.
                    Some(kind) if kind.is_stub() => notify_unavailable(tool.label(), window, cx),
                    Some(kind) => self.activate_tool(kind, cx),
                    None => notify_unavailable(tool.label(), window, cx),
                }
            }
            ToolbarAction::ChromeControlInvoked { id } => {
                self.handle_toolbar_chrome_control(id, window, cx);
            }
            ToolbarAction::ModeChangeRequested { mode } => match mode {
                ToolbarMode::Design => self.set_editor_mode(EditorMode::Design, cx),
                ToolbarMode::Motion => self.set_editor_mode(EditorMode::Motion, cx),
                ToolbarMode::Dev => notify_unavailable("Dev mode", window, cx),
            },
            // The +/- steppers, the zoom menu's percent entries, typed
            // percentages, and the ZoomCanvasTo100 command action all arrive
            // here, so they share one absolute canvas zoom path.
            ToolbarAction::ZoomChangeRequested { percent } => {
                self.zoom_to_percent(*percent, cx);
            }
            ToolbarAction::CommandInvoked { command } => match command {
                ToolbarCommand::GenerateImage
                | ToolbarCommand::GenerateVideo
                | ToolbarCommand::GenerateVector
                | ToolbarCommand::GenerateMasks
                | ToolbarCommand::RemoveBackground
                | ToolbarCommand::GenerateDesign => {
                    use crate::generation_workspace::GenerationMode;
                    let mode = match command {
                        ToolbarCommand::GenerateVideo => GenerationMode::Video,
                        ToolbarCommand::GenerateVector => GenerationMode::Vector,
                        ToolbarCommand::GenerateMasks | ToolbarCommand::RemoveBackground => {
                            GenerationMode::Masks
                        }
                        ToolbarCommand::GenerateDesign => GenerationMode::Design,
                        _ => GenerationMode::Image,
                    };
                    crate::generation_workspace::open_from_canvas(
                        mode,
                        self.item.downgrade(),
                        window,
                        cx,
                    );
                }
                ToolbarCommand::Undo => self.undo(&Undo, window, cx),
                ToolbarCommand::Redo => self.redo(&Redo, window, cx),
                ToolbarCommand::Cut => self.cut_selection(&CutSelection, window, cx),
                ToolbarCommand::Copy => self.copy_selection(&CopySelection, window, cx),
                ToolbarCommand::Paste => self.paste_selection(&PasteSelection, window, cx),
                ToolbarCommand::Duplicate => {
                    self.duplicate_selection(&DuplicateSelection, window, cx)
                }
                ToolbarCommand::Delete => self.delete_selection(&DeleteSelection, window, cx),
                ToolbarCommand::SelectAll => self.select_all(&SelectAll, window, cx),
                ToolbarCommand::ZoomToFit => self.fit_to_view(&FitToView, window, cx),
                ToolbarCommand::ZoomToSelection => self.zoom_to_selection(cx),
                ToolbarCommand::Present => self.play_prototype(&PlayPrototype, window, cx),
                ToolbarCommand::OpenDesignMode => self.set_editor_mode(EditorMode::Design, cx),
                ToolbarCommand::OpenMotionMode => self.set_editor_mode(EditorMode::Motion, cx),
                ToolbarCommand::Export => self.export_from_toolbar(window, cx),
                ToolbarCommand::Group => self.group_selection(&GroupSelection, window, cx),
                ToolbarCommand::Ungroup => self.ungroup_selection(&UngroupSelection, window, cx),
                ToolbarCommand::FrameSelection => self.frame_selection(&FrameSelection, window, cx),
                other => match toolbar_agent_prompt_template(*other) {
                    Some(template) => self.route_toolbar_agent_prompt(template, window, cx),
                    None => notify_unavailable(other.label(), window, cx),
                },
            },
            ToolbarAction::ControlChangeRequested { control, value, .. } => {
                self.handle_toolbar_control_change(*control, value, window, cx);
            }
            ToolbarAction::SecondaryControlInvoked { control, .. } => {
                self.handle_toolbar_secondary_control(*control, window, cx);
            }
            ToolbarAction::AiPromptSubmitted { prompt } => {
                self.route_toolbar_agent_prompt(prompt.as_ref(), window, cx);
            }
            ToolbarAction::AgentAttachmentRequested => {
                notify_unavailable("Attaching a file to the Agent from the toolbar", window, cx)
            }
            ToolbarAction::AgentVoiceInputRequested => {
                notify_unavailable("Voice input", window, cx)
            }
            // The palette's query text and the composer's own show/hide are
            // component-internal state; the host has nothing to do for them.
            ToolbarAction::CommandQueryChanged { .. }
            | ToolbarAction::AgentVisibilityChanged { .. } => {}
        }
    }

    /// The toolbar's Export command runs the inspector's export flow — the
    /// same presets and the same `exports/` destination — rather than a second,
    /// divergent export path. Deferred because that flow reads this view,
    /// which is leased for the duration of the toolbar event.
    #[cfg(feature = "fanta-gpui-ui")]
    fn export_from_toolbar(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        // The shipped Design inspector replaces the legacy panel that owns
        // the export engine. Mirror its status to canvas notices so progress,
        // written paths, and failures remain visible without changing the
        // user's sidebar layout.
        let inspector = self.inspector_sidebar.downgrade();
        cx.defer(move |cx| {
            inspector
                .update(cx, |inspector, cx| {
                    inspector.export_selection_with_canvas_feedback(cx)
                })
                .log_err();
        });
    }

    /// Send the toolbar's built-in AI box to the Agent Panel as a draft the
    /// user still has to send — the same review boundary canvas comments go
    /// through. The active page and selection are prefixed because the agent
    /// otherwise has no idea what "make this blue" refers to.
    #[cfg(feature = "fanta-gpui-ui")]
    fn route_toolbar_agent_prompt(
        &mut self,
        prompt: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let prompt = prompt.trim();
        if prompt.is_empty() {
            return;
        }
        let prompt = format!("{}\n\n{prompt}", self.agent_prompt_context(cx));
        let workspace = window
            .root::<MultiWorkspace>()
            .flatten()
            .map(|multi_workspace| multi_workspace.read(cx).workspace().clone());
        let Some(workspace) = workspace else {
            show_canvas_notice(
                "This window has no workspace for the Agent Panel.".to_string(),
                window,
                cx,
            );
            return;
        };
        if let Err(error) =
            agent_ui::open_external_prompt_for_review(workspace, &prompt, window, cx)
        {
            log::error!("routing the toolbar Agent prompt failed: {error:#}");
            show_canvas_notice(
                format!("The Agent prompt could not be opened: {error:#}"),
                window,
                cx,
            );
        }
    }

    /// The "what the user is looking at" header prefixed onto a toolbar
    /// Agent prompt: the page, the selection, and the first selected layers
    /// with their exact ids so the agent can act without a discovery round
    /// trip.
    #[cfg(feature = "fanta-gpui-ui")]
    fn agent_prompt_context(&self, cx: &App) -> String {
        let Some(document) = self.item.read(cx).document() else {
            return "Fanta canvas: no document is open.".to_string();
        };
        let page = document
            .page(self.selected_page_index)
            .map(|page| page.name.to_string())
            .unwrap_or_else(|| "Untitled".to_string());
        let doc = &document.doc;
        let mut context = match doc.selection.len() {
            0 => format!("Fanta canvas — page \"{page}\", nothing selected."),
            1 => format!("Fanta canvas — page \"{page}\", 1 layer selected."),
            count => format!("Fanta canvas — page \"{page}\", {count} layers selected."),
        };
        let listed = doc
            .selection
            .iter()
            .filter_map(|id| doc.scene.get(*id))
            .take(AGENT_PROMPT_CONTEXT_LAYERS);
        for node in listed {
            let geometry = doc
                .scene
                .world_bounds(node.id)
                .filter(|bounds| bounds.is_finite())
                .map(|bounds| {
                    format!(
                        ", {:.0}x{:.0} at {:.0},{:.0}",
                        bounds.width(),
                        bounds.height(),
                        bounds.min_x,
                        bounds.min_y
                    )
                })
                .unwrap_or_default();
            context.push_str(&format!(
                "\n- {} ({}, id {}{geometry})",
                node.name,
                node.data.kind_tag(),
                node.id
            ));
        }
        let remaining = doc
            .selection
            .len()
            .saturating_sub(AGENT_PROMPT_CONTEXT_LAYERS);
        if remaining > 0 {
            context.push_str(&format!("\n- …and {remaining} more"));
        }
        context.push_str(
            "\nThe ids above are exact node ids. The canvas tools are design_state (read), \
             design_edit (change) and design_screenshot (verify).",
        );
        context
    }

    /// §12 contract: an accepted control value is applied to host state and
    /// echoed back through the options setters. The echo flows through the
    /// render-time `ToolbarAdapter::refresh`, the same diff-guarded choke
    /// point every other host mutation uses.
    #[cfg(feature = "fanta-gpui-ui")]
    fn handle_toolbar_control_change(
        &mut self,
        control: fanta_gpui::toolbar::ToolbarSecondaryControl,
        value: &fanta_gpui::toolbar::ToolbarControlValue,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use fanta_gpui::toolbar::{ToolbarControlValue, ToolbarSecondaryControl};
        match (control, value) {
            (ToolbarSecondaryControl::MotionPlayPause, ToolbarControlValue::Toggle(playing)) => {
                self.timeline_shell
                    .update(cx, |timeline, cx| timeline.set_playing(*playing, cx));
                cx.notify();
            }
            (ToolbarSecondaryControl::MotionLoop, ToolbarControlValue::Toggle(looping)) => {
                self.timeline_shell
                    .update(cx, |timeline, cx| timeline.set_loop_playback(*looping, cx));
                cx.notify();
            }
            (ToolbarSecondaryControl::MotionAnimationStyle, ToolbarControlValue::Choice(style)) => {
                let active_clip = self.active_motion_clip;
                let result = self.motion_sidebar.update(cx, |panel, cx| {
                    panel.apply_toolbar_animation_style(active_clip, style, cx)
                });
                match result {
                    Ok(_) => {
                        if let Some(adapter) = self.gpui_toolbar.as_mut() {
                            adapter.remember_animation_style(style);
                        }
                        self.sync_motion_timeline(cx);
                        show_canvas_notice(format!("{style} animation added."), window, cx);
                        cx.notify();
                    }
                    Err(error) => show_canvas_notice(error.user_message(style), window, cx),
                }
            }
            (ToolbarSecondaryControl::MotionAutoKeyframe, _) => {
                // Echoing "recording" without a recorder would lie; leave the
                // chip off until a keyframe-recording mode exists.
                notify_unavailable("Auto keyframe recording", window, cx);
            }
            (ToolbarSecondaryControl::DevReadyForDevelopment, _) => {
                notify_unavailable("Marking a design ready for dev", window, cx);
            }
            (control, value) => {
                log::info!("fanta-gpui toolbar: control change {control:?} = {value:?}");
                notify_unavailable(control.label(), window, cx);
            }
        }
    }

    /// Routes press-style secondary controls to matching host actions. The
    /// transport and option chips arrive as `ControlChangeRequested`; the
    /// press-only chips below are the whole `SecondaryControlInvoked`
    /// surface, and each unwired one is logged individually.
    #[cfg(feature = "fanta-gpui-ui")]
    fn handle_toolbar_secondary_control(
        &mut self,
        control: fanta_gpui::toolbar::ToolbarSecondaryControl,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use fanta_gpui::toolbar::ToolbarSecondaryControl;
        match control {
            ToolbarSecondaryControl::MotionAddKeyframe => {
                // `add_motion_keyframe` needs a `TimelineProperty`; the chip
                // carries none, and inventing one would author a keyframe the
                // user did not ask for. The timeline's per-track controls own
                // that flow.
                show_canvas_notice(
                    "Add keyframe needs a track: use the timeline's per-track controls."
                        .to_string(),
                    window,
                    cx,
                );
            }
            ToolbarSecondaryControl::MotionTimeline => {
                show_canvas_notice(
                    "The Motion timeline is always visible in Motion mode.".to_string(),
                    window,
                    cx,
                );
            }
            ToolbarSecondaryControl::MotionTimeComment => {
                notify_unavailable("Time-anchored comments", window, cx);
            }
            ToolbarSecondaryControl::DevInspect
            | ToolbarSecondaryControl::DevAnnotate
            | ToolbarSecondaryControl::DevMeasure => {
                notify_unavailable(control.label(), window, cx);
            }
            other => notify_unavailable(other.label(), window, cx),
        }
    }
}

impl FigView {
    /// Restart the presentation at a named flow's entry frame (the flow
    /// picker in the presentation chrome).
    pub(crate) fn start_prototype_flow(&mut self, flow: usize, cx: &mut Context<Self>) {
        if let Some(player) = self.prototype_player.as_mut()
            && player.start_flow(flow)
        {
            self.invalidate_canvas_cache();
            cx.notify();
        }
    }
}

/// One id shared by every canvas notice, so a second click replaces the
/// standing message instead of stacking a queue of them.
const CANVAS_NOTICE_ID: &str = "fanta-canvas-notice";

/// How many selected layers a toolbar Agent prompt lists by id.
#[cfg(feature = "fanta-gpui-ui")]
const AGENT_PROMPT_CONTEXT_LAYERS: usize = 8;

/// The draft prompt a text-oriented toolbar AI command opens in the Agent Panel
/// for the user to complete and review. Media and Design commands are routed to
/// their dedicated generation workspaces before this helper is reached.
#[cfg(feature = "fanta-gpui-ui")]
fn toolbar_agent_prompt_template(
    command: fanta_gpui::toolbar::ToolbarCommand,
) -> Option<&'static str> {
    use fanta_gpui::toolbar::ToolbarCommand;
    match command {
        ToolbarCommand::GenerateDesign => Some(
            "Design <describe the screen> as a new frame on this page, using auto layout, \
             a consistent type scale and the page's existing colours; verify with \
             design_screenshot.",
        ),
        ToolbarCommand::ReplaceContent => Some(
            "Replace the placeholder content in the selection with realistic content for \
             <describe the product>.",
        ),
        ToolbarCommand::RewriteText => {
            Some("Rewrite the text of the selected text layers to: <describe the tone or goal>")
        }
        ToolbarCommand::TranslateText => {
            Some("Translate the selected text layers to <language>, keeping the layout intact.")
        }
        ToolbarCommand::RenameLayers => Some(
            "Rename the selected layers with clear, descriptive names based on their content \
             and role (use design_state to read them, then design_edit set_props name changes \
             in one batch).",
        ),
        _ => None,
    }
}

/// Show `message` in this window's workspace notification surface.
///
/// The vendored fanta-gpui surfaces (the editor toolbar, the pages and layers
/// panels) emit intents for far more affordances than this build implements.
/// Every one of those routes through here, because a click that only produces
/// a log line is indistinguishable from a broken button.
pub(crate) fn show_canvas_notice(message: String, window: &mut Window, cx: &mut App) {
    let Some(workspace) = window
        .root::<MultiWorkspace>()
        .flatten()
        .map(|multi_workspace| multi_workspace.read(cx).workspace().clone())
    else {
        log::warn!("fanta: no workspace is available to show a notice in: {message}");
        return;
    };
    workspace.update(cx, |workspace, cx| {
        workspace.show_toast(
            Toast::new(NotificationId::named(CANVAS_NOTICE_ID.into()), message).autohide(),
            cx,
        );
    });
}

/// Show a canvas notice from a path that holds no `Window`. The notice lands
/// on the active window at the next effect flush.
pub(crate) fn show_canvas_notice_deferred(message: String, cx: &mut App) {
    cx.defer(move |cx| {
        let Some(window) = cx.active_window() else {
            log::warn!("fanta: no active window to show a canvas notice in: {message}");
            return;
        };
        window
            .update(cx, |_, window, cx| show_canvas_notice(message, window, cx))
            .log_err();
    });
}

/// Tell the user, by name, that the thing they just clicked is not in this
/// build. `what` is the affordance's own label so the message points at the
/// control the user actually pressed.
///
/// Gated with its callers: every unwired affordance belongs to a vendored
/// fanta-gpui surface, so a `--no-default-features` diagnostic build has
/// nothing to decline.
#[cfg(feature = "fanta-gpui-ui")]
pub(crate) fn notify_unavailable(what: &str, window: &mut Window, cx: &mut App) {
    log::info!("fanta: {what} is not available in the alpha");
    show_canvas_notice(
        format!("{what} is not available in the Fanta alpha yet."),
        window,
        cx,
    );
}
