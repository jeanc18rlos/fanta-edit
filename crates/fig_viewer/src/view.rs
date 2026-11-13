//! The workspace item view for Figma documents: canvas chrome (floating tool
//! pill, page picker, project controls), input routing into the tool shell,
//! and the `Item` integration that gives fanta projects text-editor-style
//! dirty tracking and save.

mod agent_presence;
#[path = "view_annotations.rs"]
pub(crate) mod annotations_host;
#[path = "view_dev_mode.rs"]
mod dev_mode;
#[path = "view_measurements.rs"]
mod measurements_host;
#[path = "view_media.rs"]
mod media_host;
#[cfg(feature = "fanta-gpui-ui")]
mod properties_inspector;
mod serialization;
#[cfg(feature = "fanta-gpui-ui")]
mod timeline_adapter;

use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    rc::Rc,
    sync::Arc,
};

use anyhow::{Context as _, Result};
use fanta_canvas::HitPrecision;
use fanta_doc::{
    AnimationClip, AnimationClipId, AssetId, BoundProp, CanvasNode, Doc, Easing, IndexKey,
    Interpolation, Keyframe, KeyframeId, MotionEvaluation, MotionProperty, NodeData, NodeId,
    Operation, ProjectAssetKind, ResolvedVarValue, Scope, Transaction, Viewport,
};
// Only the macOS video fills and tests read paint kinds here.
#[cfg(any(test, target_os = "macos"))]
use fanta_doc::Fill;
use fanta_tools::{Button as ToolButton, LogicalKey, ToolEvent};
use file_icons::FileIcons;
use glam::DVec2;
use gpui::{
    Action, Anchor, AnyElement, AnyWindowHandle, App, Bounds, ClipboardEntry, ClipboardItem,
    Context, CursorStyle, DragMoveEvent, Empty, Entity, EventEmitter, FocusHandle, Focusable,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PinchEvent, Pixels, Point, Render,
    RenderImage, ScrollDelta, ScrollWheelEvent, SharedString, Subscription, Task, Window, actions,
    div, px,
};
use language::Capability;
use project::{Project, ProjectEntryId, ProjectPath};
use settings::{Settings as _, update_settings_file};
use smallvec::SmallVec;
use ui::{ContextMenu, ContextMenuEntry, Divider, IconPosition, PopoverMenu, Tooltip, prelude::*};
use util::{ResultExt, paths::PathExt};
use workspace::{
    ItemSettings, MultiWorkspace, Pane, Toast, Workspace,
    item::{Item, ItemEvent, ProjectItem, SaveOptions, TabContentParams},
    notifications::{NotificationId, save_io_error_detail},
};

use crate::canvas::{
    CanvasElement, RenderedCanvas, authored_local_bounds, bounds_size, evaluated_hit_test_screen,
    inspect_hit_test_screen, precise_hit_test_screen, screen_position_in_bounds,
};
use crate::clipboard::{
    CanvasClipboard, ClipboardPlacement, apply_transaction as apply_canvas_transaction,
    clone_component_operations, paste_operations,
};
use crate::code_workspace::{CodeWorkspaceFile, FantaCodeWorkspace};
use crate::comments::MotionCommentAnchor;
use crate::comments_panel::CommentsPanel;
use crate::design_panel::{FantaDesignPanel, FileInspectorVisibilityChanged};
use crate::document::{
    AssetStores, DocChange, FigDocument, FigItem, FigItemEvent, FigScope, MAX_IMAGE_SOURCE_BYTES,
    PreparedImage, SaveKind, ScopeRequester,
};
use crate::editor_session::{
    EditorMode, EditorModeTabs, EditorSession, EditorWorkspace, EditorWorkspaceTabs,
};
use crate::generation_media::{self, PreparedCanvasAsset};
use crate::measurements::{
    MeasurementCommit, MeasurementController, MeasurementDragKind, MeasurementGeometry,
    MeasurementHit, MeasurementOverlay, MeasurementProjection, MeasurementRecord,
    delete_measurement_op, read_measurements,
};
use crate::motion_edit::{
    MotionKeyframeDragSession, delete_keyframe_operation, evaluated_motion_value,
    rename_clip_operation, set_clip_duration_operation, set_clip_loop_mode_operation,
    set_keyframe_easing_operation, set_keyframe_interpolation_operation,
    upsert_motion_keyframe_operation,
};
use crate::motion_panel::{FantaMotionPanel, MotionPanelEvent};
use crate::panel_settings::{FantaDesignPanelSettings, FantaPropertiesPanelSettings};
use crate::properties_panel::FantaPropertiesPanel;
use crate::prototype_panel::FantaPrototypePanel;
use crate::prototype_player::PrototypePlayerState;
use crate::text_edit::CanvasTextEdit;
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
use crate::canvas::{CanvasVideoFillFrame, CanvasVideoFrame, GpuCanvas};
#[cfg(target_os = "macos")]
use crate::generation_workspace::AudioPlaybackView;
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
        /// Switch between authoring a template and filling it in.
        ToggleTemplateFill,
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
        EnterDevMode,
        OpenSavedCode,
        PlaceLocalMedia,
        ActivateSelectTool,
        /// Inspect layers without editing artwork.
        ActivateInspectTool,
        /// Measure a distance on the canvas.
        ActivateMeasureTool,
        /// Add an annotation to the canvas.
        ActivateAnnotationTool,
        /// Activate the hand (pan) tool.
        ActivateHandTool,
        /// Activate the rectangle tool.
        ActivateRectangleTool,
        /// Activate the ellipse tool.
        ActivateEllipseTool,
        /// Activate the line tool.
        ActivateLineTool,
        /// Activate the arrow tool.
        ActivateArrowTool,
        /// Activate the polygon tool.
        ActivatePolygonTool,
        /// Activate the star tool.
        ActivateStarTool,
        /// Activate the pen tool.
        ActivatePenTool,
        /// Activate the path editing tool.
        ActivateNodeEditTool,
        /// Reverse the direction of selected vector paths.
        ReversePath,
        /// Break the path at the selected anchor.
        BreakPathAtAnchor,
        /// Activate the frame tool.
        ActivateFrameTool,
        /// Activate the text tool.
        ActivateTextTool,
        /// Activate the pencil (freehand) tool.
        ActivatePencilTool,
        /// Activate the vector brush tool.
        ActivateBrushTool,
        /// Activate the vector stroke eraser.
        ActivateEraserTool,
        /// Activate the section tool.
        ActivateSectionTool,
        /// Activate the slice tool.
        ActivateSliceTool,
        /// Proportionally scale selected objects and their contents.
        ActivateScaleTool,
        /// Activate direct selection of vector anchors and segments.
        ActivatePathSelectTool,
        /// Select vector layers with a drawn rectangle.
        ActivateRectangleSelectTool,
        /// Select vector and image content inside an ellipse.
        ActivateEllipseSelectTool,
        /// Select vector and image content with a freehand outline.
        ActivateLassoTool,
        /// Select vector and image content with clicked polygon corners.
        ActivatePolygonalLassoTool,
        /// Select matching vector paints and image assets.
        ActivateMagicWandTool,
        /// Crop vector and image content with a rectangular clip.
        ActivateCropTool,
        /// Convert the selected text and vector into text on a path.
        ActivateTextPathTool,
        /// Activate the comment tool (click the canvas to pin a comment).
        ActivateCommentTool,
        /// Post the comment or reply being composed on the canvas.
        SubmitComment,
        /// Export the selected animation as portable Motion JSON.
        ExportMotionJson,
        /// Export the selected animation as Figma Motion JSON.
        ExportFigmaMotionJson,
        /// Export the selected animation as a Claude Motion Stage project.
        ExportMotionStage,
        /// Export the selected animation as an MP4 video.
        ExportMotionVideo,
        /// Export the selected animation as an animated GIF.
        ExportMotionGif,
        /// Export the selected animation as a folder of PNG frames.
        ExportMotionFrames,
        /// Apply the suggested design change in the open comment thread.
        ApplySuggestedFix,
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
        ToolKind::Inspect => Box::new(ActivateInspectTool),
        ToolKind::PathSelect => Box::new(ActivatePathSelectTool),
        ToolKind::RectangleSelect => Box::new(ActivateRectangleSelectTool),
        ToolKind::EllipseSelect => Box::new(ActivateEllipseSelectTool),
        ToolKind::Lasso => Box::new(ActivateLassoTool),
        ToolKind::PolygonalLasso => Box::new(ActivatePolygonalLassoTool),
        ToolKind::MagicWand => Box::new(ActivateMagicWandTool),
        ToolKind::Crop => Box::new(ActivateCropTool),
        ToolKind::NodeEdit => Box::new(ActivateNodeEditTool),
        ToolKind::Hand => Box::new(ActivateHandTool),
        ToolKind::Scale => Box::new(ActivateScaleTool),
        ToolKind::Rect => Box::new(ActivateRectangleTool),
        ToolKind::Ellipse => Box::new(ActivateEllipseTool),
        ToolKind::Line => Box::new(ActivateLineTool),
        ToolKind::Arrow => Box::new(ActivateArrowTool),
        ToolKind::Polygon => Box::new(ActivatePolygonTool),
        ToolKind::Star => Box::new(ActivateStarTool),
        ToolKind::Pen => Box::new(ActivatePenTool),
        ToolKind::Brush => Box::new(ActivateBrushTool),
        ToolKind::Pencil => Box::new(ActivatePencilTool),
        ToolKind::Eraser => Box::new(ActivateEraserTool),
        ToolKind::Frame => Box::new(ActivateFrameTool),
        ToolKind::Section => Box::new(ActivateSectionTool),
        ToolKind::Slice => Box::new(ActivateSliceTool),
        ToolKind::Text => Box::new(ActivateTextTool),
        ToolKind::TextPath => Box::new(ActivateTextPathTool),
        ToolKind::Comment => Box::new(ActivateCommentTool),
        ToolKind::Measure => Box::new(ActivateMeasureTool),
        ToolKind::Annotation => Box::new(ActivateAnnotationTool),
    }
}

pub(crate) const MIN_ZOOM: f32 = 0.1;
pub(crate) const MAX_ZOOM: f32 = 20.0;
const ZOOM_STEP: f32 = 1.1;
const SCROLL_LINE_MULTIPLIER: f32 = 20.0;
pub(crate) const RENDER_PADDING: f64 = 48.0;
const MIN_LAYERS_SIDEBAR_WIDTH: f32 = 220.0;
#[cfg(feature = "fanta-gpui-ui")]
const MIN_INSPECTOR_SIDEBAR_WIDTH: f32 =
    fanta_gpui::properties_inspector::PROPERTIES_INSPECTOR_MIN_WIDTH;
#[cfg(not(feature = "fanta-gpui-ui"))]
const MIN_INSPECTOR_SIDEBAR_WIDTH: f32 = 260.0;
const MAX_SIDEBAR_WIDTH: f32 = 560.0;
const MIN_PANE_CANVAS_WIDTH: f32 = 240.0;
const SIDEBAR_RESIZE_HANDLE_SIZE: Pixels = px(6.);
const SIDEBAR_BORDER_WIDTH: Pixels = px(1.);
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

#[derive(Clone, PartialEq, Eq)]
struct LocalMediaOrigin {
    item: gpui::EntityId,
    scene: u64,
    page: NodeId,
    scope: Option<FigScope>,
    mode: EditorMode,
    path: std::path::PathBuf,
    generation: u64,
}

#[derive(Clone)]
struct MeasurementSelection {
    document: fanta_doc::DocId,
    scene: u64,
    page: NodeId,
    id: String,
}

struct MeasurementCache {
    scene: u64,
    revision: u64,
    page: NodeId,
    records: Vec<MeasurementRecord>,
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

const CLICK_DIAGNOSTIC_LIMIT: u16 = 256;
const CLICK_DIAGNOSTIC_SELECTION_LIMIT: usize = 16;

struct ClickDiagnostics {
    reserved: u16,
    #[cfg(test)]
    records: Vec<serde_json::Value>,
}

impl ClickDiagnostics {
    fn from_setting(value: Option<&std::ffi::OsStr>) -> Option<Self> {
        (value == Some(std::ffi::OsStr::new("1"))).then(|| Self {
            reserved: 0,
            #[cfg(test)]
            records: Vec::new(),
        })
    }

    fn capture<T>(state: Option<&mut Self>, payload: impl FnOnce() -> T) -> Option<(u16, T)> {
        let state = state?;
        if state.reserved >= CLICK_DIAGNOSTIC_LIMIT {
            return None;
        }
        state.reserved += 1;
        Some((state.reserved, payload()))
    }
}

#[derive(serde::Serialize)]
struct ClickDiagnosticSnapshot {
    selection: Vec<NodeId>,
    selection_count: usize,
    selection_truncated: bool,
    tool: String,
    mode: String,
    primary_pressed: bool,
    canvas_pointer_down: bool,
    space_pan: bool,
    text_edit_active: bool,
}

struct PendingClickDiagnostic {
    sequence: u16,
    stage: &'static str,
    input: serde_json::Value,
    before: ClickDiagnosticSnapshot,
}

fn click_diagnostic_modifiers(modifiers: gpui::Modifiers) -> serde_json::Value {
    serde_json::json!({
        "shift": modifiers.shift,
        "command": modifiers.platform,
        "control": modifiers.control,
        "alt": modifiers.alt,
        "function": modifiers.function,
    })
}

pub struct FigView {
    pub(crate) item: Entity<FigItem>,
    project: Entity<Project>,
    pub(crate) focus_handle: FocusHandle,
    native_toolbar_focus: FocusHandle,
    editor_session: Entity<EditorSession>,
    layers_sidebar: Entity<FantaDesignPanel>,
    _file_inspector_subscription: Subscription,
    inspector_sidebar: Entity<FantaPropertiesPanel>,
    prototype_sidebar: Entity<FantaPrototypePanel>,
    comments_sidebar: Entity<CommentsPanel>,
    motion_sidebar: Entity<FantaMotionPanel>,
    variables_workspace: Entity<FantaVariablesWorkspace>,
    code_workspace: Entity<FantaCodeWorkspace>,
    timeline_shell: Entity<TimelineShell>,
    active_motion_clip: Option<AnimationClipId>,
    motion_auto_keyframe: bool,
    motion_keyframe_drag: Option<MotionKeyframeDragSession>,
    layers_sidebar_visible: bool,
    inspector_sidebar_visible: bool,
    layers_sidebar_width: Pixels,
    inspector_sidebar_width: Pixels,
    pane_width: Option<Pixels>,
    revealed_compact_sidebar: Option<SidebarKind>,
    inspector_has_focus: bool,
    inspector_focus_handle: FocusHandle,
    selected_page_index: Option<usize>,
    /// Root node of the explicitly selected page, used to re-resolve
    /// `selected_page_index` when a disk reload reorders or removes pages.
    selected_page_root: Option<NodeId>,
    /// The worktree entry this view was opened from (the clicked `page.fnx`,
    /// `master.fnx`, `doc/variables.json`, manifest, or `.fig`). Reported to
    /// the pane so each opened path keeps its OWN tab even though every tab
    /// shares the project's one [`FigItem`].
    opened_entry_id: Option<ProjectEntryId>,
    opened_project_path: Option<ProjectPath>,
    /// What this tab is scoped to. Re-asserted on focus so the shared
    /// document's single render root follows the focused tab, and updated by
    /// in-tab navigation ([`Self::select_page`]).
    scope: Option<FigScope>,
    is_focused: bool,
    agent_navigation_focus: Option<(
        String,
        Option<String>,
        Option<usize>,
        design_surface::AgentWorkspace,
    )>,
    /// The document root this view last acted on: a `ScopeApplied` that
    /// merely restores our own root (switching back to this tab) must not
    /// clobber the saved viewport.
    last_seen_root: Option<NodeId>,
    pub(crate) viewport: Option<Viewport>,
    pan_last_position: Option<Point<Pixels>>,
    primary_pressed: bool,
    click_diagnostics: Option<ClickDiagnostics>,
    pub(crate) gesture_perf: Option<crate::gesture_perf::SharedGesturePerf>,
    /// A pointer button is down on the canvas, from the press until the
    /// release (wherever it lands). Broader than `primary_pressed`, which the
    /// tool paths only set once an event reaches a tool: the autosave must
    /// also stand down for a text-selection drag or a comment click.
    canvas_pointer_down: bool,
    /// The debounced autosave armed by the last committing edit.
    autosave_task: Option<Task<()>>,
    /// The save error the user has been shown and not yet seen cleared.
    reported_save_failure: Option<String>,
    save_failure_notice: Rc<RefCell<Option<SharedString>>>,
    /// The selection handle the cursor is over, resolved on hover so
    /// `render` only reads it — hit-testing eight handles per redraw would
    /// run on every window frame, not just on pointer movement.
    hover_resize_handle: Option<fanta_canvas::ResizeHandle>,
    /// Space is held: the canvas temporarily pans with any active tool, the
    /// Figma/Illustrator "hold space to pan" gesture. Cleared on key-up.
    space_pan: bool,
    pub(crate) container_bounds: Option<Bounds<Pixels>>,
    pub(crate) rendered_canvas: Option<RenderedCanvas>,
    /// The CPU canvas or prototype frame painted last. GPUI keeps a painted image in the
    /// window's sprite atlas until it is dropped explicitly, and these paths render a new image
    /// for every pan, zoom, edit and animation frame.
    painted_canvas_image: Option<Arc<RenderImage>>,
    /// Memoized selection-chrome geometry (frame labels, per-node selection
    /// bounds, text baselines), reused across paint frames while the scene
    /// revision, page root, selection, and text-edit target are unchanged —
    /// pans and zooms repaint every frame without touching any of them.
    /// Interior-mutable because the canvas element collects overlays during
    /// paint with only `&App`. Cleared with the rendered-canvas cache: a
    /// reload restarts the scene revision counter, which could collide.
    pub(crate) chrome_cache: std::cell::RefCell<Option<crate::canvas::ChromeCache>>,
    /// The text node the last paint drew with its text variables as typed.
    pub(crate) painted_raw_text_node: Option<NodeId>,
    /// The Metal render thread and its newest frame; created on first paint.
    /// `pub(crate)` because the canvas element drives it during paint.
    #[cfg(target_os = "macos")]
    pub(crate) gpu_canvas: Option<GpuCanvas>,
    #[cfg(target_os = "macos")]
    canvas_video: Option<CanvasVideoSession>,
    #[cfg(target_os = "macos")]
    canvas_audio: Option<CanvasAudioSession>,
    #[cfg(target_os = "macos")]
    canvas_video_fills: HashMap<AssetId, CanvasVideoFillSession>,
    #[cfg(target_os = "macos")]
    canvas_video_generation: u64,
    #[cfg(target_os = "macos")]
    canvas_video_fill_generation: u64,
    #[cfg(target_os = "macos")]
    canvas_video_removed: std::cell::Cell<bool>,
    #[cfg(target_os = "macos")]
    canvas_video_active: std::cell::Cell<bool>,
    pub(crate) tools: ToolShell,
    annotation_state: annotations_host::AnnotationHostState,
    measurement_controller: MeasurementController,
    measurement_origin: Option<LocalMediaOrigin>,
    measurement_selection: Option<MeasurementSelection>,
    measurement_cache: std::cell::RefCell<Option<MeasurementCache>>,
    media_import_generation: std::cell::Cell<u64>,
    media_import_task: Option<Task<()>>,
    wand_asset_map: Option<Arc<BTreeMap<AssetId, Vec<u8>>>>,
    wand_decode_cache: HashMap<AssetId, Arc<image::RgbaImage>>,
    wand_decode_order: VecDeque<AssetId>,
    wand_generation: u64,
    wand_override: Option<(NodeId, fanta_tools::select::DrawSelectionShape)>,
    pub(crate) comment_state: crate::comments_ui::CommentState,
    /// The last-used tool per toolbar group, so each group's button keeps
    /// showing the member you last picked (Figma behavior). Indexed by group.
    group_faces: Vec<ToolKind>,
    /// Fill mode for this window: the user is filling a template in, so
    /// canvas gestures and selection commands keep to what each layer's
    /// editing rules allow. Off while authoring, and never saved.
    pub(crate) template_fill: bool,
    #[cfg(feature = "fanta-gpui-ui")]
    gpui_toolbar: Option<crate::gpui_adapters::toolbar::ToolbarAdapter>,
    /// The DesignPanel inspector adapter, mounted for Design mode when the
    /// fanta-gpui runtime and the `FANTA_GPUI_DESIGN` gate allow it; the
    /// legacy `FantaPropertiesPanel` stays the fallback. `pub(crate)` because
    /// the adapter's host methods live in `gpui_adapters::design`.
    #[cfg(feature = "fanta-gpui-ui")]
    pub(crate) gpui_design: Option<crate::gpui_adapters::design::DesignAdapter>,
    #[cfg(feature = "fanta-gpui-ui")]
    gpui_properties: Option<properties_inspector::PropertiesAdapter>,
    #[cfg(feature = "fanta-gpui-ui")]
    gpui_timeline: Option<timeline_adapter::TimelineAdapter>,
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
    pub(crate) prototype_player: Option<PrototypePlayerState>,
    pub(crate) canvas_context_menu: Option<crate::view_context_menu::CanvasContextMenu>,
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
    _agent_activity_subscription: Subscription,
    workspace_activation_subscription: Option<Subscription>,
}

pub enum FigViewEvent {
    Edited,
    TitleChanged,
    PropertyDraftChanged,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CanvasAudioSource {
    scene: u64,
    node: NodeId,
    asset: AssetId,
    bytes_identity: Option<(usize, usize)>,
    time_range_us: [i64; 2],
    muted: bool,
    volume_bits: u32,
}

#[cfg(target_os = "macos")]
struct CanvasAudioSession {
    source: CanvasAudioSource,
    playback: Entity<AudioPlaybackView>,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CanvasVideoFillSource {
    scene: u64,
    node: NodeId,
    asset: AssetId,
    assets_identity: usize,
    bytes_identity: Option<(usize, usize)>,
}

#[cfg(target_os = "macos")]
struct CanvasVideoFillSession {
    source: CanvasVideoFillSource,
    generation: u64,
    loading: Option<Task<()>>,
    playback: Option<Entity<VideoPlaybackView>>,
    observation: Option<Subscription>,
    error: Option<SharedString>,
}

#[cfg(target_os = "macos")]
fn video_fill_assets(node: &CanvasNode) -> Vec<AssetId> {
    let mut assets = HashSet::new();
    let mut add = |fill: &Fill| {
        if let Fill::Video { video, .. } = fill {
            assets.insert(video.asset);
        }
    };
    match &node.data {
        NodeData::Vector(vector) => {
            vector.fills.iter().for_each(&mut add);
            vector.strokes.iter().for_each(|stroke| add(&stroke.paint));
        }
        NodeData::Boolean(boolean) => {
            boolean.fills.iter().for_each(&mut add);
            boolean.strokes.iter().for_each(|stroke| add(&stroke.paint));
        }
        NodeData::Group(group) => {
            if let Some(background) = &group.background {
                add(background);
            }
            group.background_fills.iter().for_each(&mut add);
            group.strokes.iter().for_each(|stroke| add(&stroke.paint));
        }
        _ => {}
    }
    let mut assets = assets.into_iter().collect::<Vec<_>>();
    assets.sort_unstable();
    assets
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

enum CanvasDoubleClickAction {
    Text(NodeId),
    Tool(ToolKind),
    Instance,
    Inspector,
}

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
        let view = cx.entity_id();
        cx.on_release(move |_, cx| crate::agent_surface::clear_active_view(view, cx))
            .detach();
        cx.on_release(Self::release_painted_canvas_image).detach();
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
        let opened_entry_id = descriptor
            .as_ref()
            .and_then(|descriptor| descriptor.entry_id);
        let scope = descriptor.as_ref().and_then(|descriptor| descriptor.scope);
        let opened_project_path = descriptor.map(|descriptor| descriptor.path);
        let save_failure_notice = item.read(cx).save_failure_notice.clone();
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
        let file_inspector_subscription = cx.subscribe(
            &layers_sidebar,
            |this, _, event: &FileInspectorVisibilityChanged, cx| {
                this.set_layers_sidebar_visible(event.0, cx);
                cx.notify();
            },
        );
        let prototype_sidebar = cx.new(|cx| FantaPrototypePanel::new(item.clone(), cx));
        let comments_sidebar = {
            let view = cx.weak_entity();
            cx.new(|cx| CommentsPanel::new(item.clone(), view, cx))
        };
        let timeline_shell = cx.new(|cx| {
            let mut timeline = TimelineShell::new();
            timeline.set_authoring_enabled(false, cx);
            timeline
        });
        let motion_sidebar =
            cx.new(|cx| FantaMotionPanel::new(item.clone(), timeline_shell.clone(), window, cx));
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
        let timeline_subscription =
            cx.subscribe(&timeline_shell, |this, _, event: &TimelineEvent, cx| {
                this.handle_timeline_event(event.clone(), cx);
            });
        let focus_handle = cx.focus_handle();
        cx.on_focus(&focus_handle, window, |this: &mut Self, _, cx| {
            this.is_focused = true;
            this.activate_design_scope(cx);
        })
        .detach();
        // A space held across a focus change (panel click, window switch, a
        // text session opening) delivers its key-up elsewhere; without this
        // reset `space_pan` stays true and the Select tool pans with a hand
        // cursor until space is pressed again.
        cx.on_focus_out(&focus_handle, window, |this: &mut Self, _, _, cx| {
            this.is_focused = false;
            this.freeze_annotation_move(cx);
            if this.space_pan || this.pan_last_position.is_some() {
                this.space_pan = false;
                this.pan_last_position = None;
                cx.notify();
            }
        })
        .detach();
        // A background scoped open leaves the shared root unchanged. The
        // activation event applies this tab's scope and fits its own viewport.
        let last_seen_root = item
            .read(cx)
            .document()
            .and_then(|document| document.doc.active_page());
        Self {
            item,
            project,
            focus_handle,
            native_toolbar_focus: cx.focus_handle(),
            editor_session,
            layers_sidebar,
            _file_inspector_subscription: file_inspector_subscription,
            inspector_sidebar,
            prototype_sidebar,
            comments_sidebar,
            motion_sidebar,
            variables_workspace,
            code_workspace,
            timeline_shell,
            active_motion_clip: None,
            motion_auto_keyframe: false,
            motion_keyframe_drag: None,
            layers_sidebar_visible,
            inspector_sidebar_visible,
            layers_sidebar_width,
            inspector_sidebar_width,
            pane_width: None,
            revealed_compact_sidebar: None,
            inspector_has_focus: false,
            inspector_focus_handle: cx.focus_handle(),
            selected_page_index: None,
            selected_page_root: None,
            opened_entry_id,
            opened_project_path,
            scope,
            is_focused: false,
            agent_navigation_focus: None,
            last_seen_root,
            viewport: None,
            pan_last_position: None,
            primary_pressed: false,
            click_diagnostics: ClickDiagnostics::from_setting(
                std::env::var_os("FANTA_INPUT_DIAGNOSTICS").as_deref(),
            ),
            gesture_perf: crate::perf_enabled().then(crate::gesture_perf::GesturePerf::shared),
            canvas_pointer_down: false,
            autosave_task: None,
            reported_save_failure: None,
            save_failure_notice,
            hover_resize_handle: None,
            space_pan: false,
            container_bounds: None,
            rendered_canvas: None,
            painted_canvas_image: None,
            chrome_cache: std::cell::RefCell::new(None),
            painted_raw_text_node: None,
            #[cfg(target_os = "macos")]
            gpu_canvas: None,
            #[cfg(target_os = "macos")]
            canvas_video: None,
            #[cfg(target_os = "macos")]
            canvas_audio: None,
            #[cfg(target_os = "macos")]
            canvas_video_fills: HashMap::new(),
            #[cfg(target_os = "macos")]
            canvas_video_generation: 0,
            #[cfg(target_os = "macos")]
            canvas_video_fill_generation: 0,
            #[cfg(target_os = "macos")]
            canvas_video_removed: std::cell::Cell::new(false),
            #[cfg(target_os = "macos")]
            canvas_video_active: std::cell::Cell::new(true),
            tools: ToolShell::new(),
            annotation_state: annotations_host::AnnotationHostState::default(),
            measurement_controller: MeasurementController::default(),
            measurement_origin: None,
            measurement_selection: None,
            measurement_cache: std::cell::RefCell::new(None),
            media_import_generation: std::cell::Cell::new(0),
            media_import_task: None,
            wand_asset_map: None,
            wand_decode_cache: HashMap::new(),
            wand_decode_order: VecDeque::new(),
            wand_generation: 0,
            wand_override: None,
            comment_state: crate::comments_ui::CommentState::default(),
            group_faces: crate::tools::initial_group_faces(),
            template_fill: false,
            #[cfg(feature = "fanta-gpui-ui")]
            gpui_toolbar: crate::gpui_adapters::runtime_enabled(cx)
                .then(|| crate::gpui_adapters::toolbar::ToolbarAdapter::new(window, cx)),
            #[cfg(feature = "fanta-gpui-ui")]
            gpui_design: (crate::gpui_adapters::runtime_enabled(cx)
                && crate::gpui_adapters::design::design_enabled())
            .then(|| crate::gpui_adapters::design::DesignAdapter::new(window, cx)),
            #[cfg(feature = "fanta-gpui-ui")]
            gpui_properties: None,
            #[cfg(feature = "fanta-gpui-ui")]
            gpui_timeline: None,
            fonts_prewarmed: false,
            hovered_node: None,
            text_edit: None,
            pending_text_edit: None,
            prototype_player: None,
            canvas_context_menu: None,
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
            _agent_activity_subscription: Self::observe_agent_activity(window, cx),
            workspace_activation_subscription: None,
        }
    }

    fn activate_design_scope(&mut self, cx: &mut Context<Self>) {
        crate::agent_surface::set_active_item_for_view(self.item.downgrade(), cx.entity_id(), cx);
        if let Some(scope) = self.scope {
            let requester = ScopeRequester::View(cx.entity_id());
            self.item.update(cx, |item, cx| {
                item.request_scope(scope, requester, cx);
            });
        }
    }

    fn project_path_for_opened_entry(&self, cx: &App) -> Option<ProjectPath> {
        self.opened_entry_id
            .and_then(|entry| self.project.read(cx).path_for_entry(entry, cx))
            .or_else(|| self.opened_project_path.clone())
    }

    fn reconcile_opened_entry_with_project_root(&mut self, cx: &App) {
        self.opened_project_path = self.project_path_for_opened_entry(cx);
        if let Some(root) = self.item.read(cx).project_root() {
            self.opened_entry_id = self.opened_entry_id.filter(|entry_id| {
                let project = self.project.read(cx);
                project
                    .path_for_entry(*entry_id, cx)
                    .and_then(|path| project.absolute_path(&path, cx))
                    .is_some_and(|path| path.starts_with(root))
            });
            self.opened_project_path = self.opened_project_path.take().filter(|path| {
                self.project
                    .read(cx)
                    .absolute_path(path, cx)
                    .is_some_and(|path| path.starts_with(root))
            });
        }
    }

    fn subscribe_to_item(item: &Entity<FigItem>, cx: &mut Context<Self>) -> Subscription {
        cx.subscribe(item, |this, _, event: &FigItemEvent, cx| {
            if matches!(event, FigItemEvent::StateChanged) {
                this.end_gesture_perf("document_state_changed");
            }
            #[cfg(target_os = "macos")]
            if this.canvas_video.as_ref().is_some_and(|session| {
                this.selected_canvas_video_source(cx).map(|source| source.0)
                    != Some(session.source)
            }) {
                this.clear_canvas_video(cx);
            }
            #[cfg(feature = "fanta-gpui-ui")]
            if matches!(event, FigItemEvent::StateChanged) {
                this.discard_gpui_design_edits();
            }
            // Echo document state into the DesignPanel inspector. Preview
            // frames are skipped (the panel re-echoes on the committing
            // event); selection and text-selection changes must refresh even
            // though the native panels ignore them.
            this.reconcile_annotations(cx);
            if this.measurement_selection.is_some()
                && (this.selected_measurement(cx).is_none()
                    || this.item.read(cx).doc().is_some_and(|doc| !doc.selection.is_empty()))
            {
                this.measurement_selection = None;
                let view = cx.weak_entity();
                cx.defer(move |cx| {
                    view.update(cx, |view, cx| view.sync_measurement_edit_barrier(cx)).log_err();
                });
            }
            if matches!(event, FigItemEvent::StateChanged) && this.measurement_controller.has_pending_authoring() {
                this.measurement_controller.freeze_after_release_error();
                this.primary_pressed = false;
                this.canvas_pointer_down = false;
                show_canvas_notice_deferred("The document changed during measurement. Its draft was kept; cancel it before starting again.".into(), cx);
            }
            #[cfg(feature = "fanta-gpui-ui")]
            if !matches!(event, FigItemEvent::EditedTransient) {
                this.refresh_gpui_design(cx);
            }
            match event {
                FigItemEvent::PageRegistryChanged => {
                    this.sync_page_registry_mirrors(false, cx);
                    this.refresh_tool_overlays(cx);
                },
                FigItemEvent::Edited => {
                    this.invalidate_canvas_cache();
                    this.schedule_autosave(cx);
                    this.sync_motion_timeline(cx);
                    this.hovered_node = None;
                    // The edit may have removed the node under the inline
                    // text editor (undo, layer delete); the overlay must not
                    // outlive its target.
                    this.drop_text_edit_if_target_gone(cx);
                    this.reconcile_annotations(cx);
                    if this.selected_measurement(cx).is_none() { this.measurement_selection = None; }
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
                    this.reconcile_annotations(cx);
                    if this.item.read(cx).doc().is_some_and(|doc| !doc.selection.is_empty()) {
                        this.measurement_selection = None;
                    }
                    // The cached handle belongs to the node that was
                    // selected; a resize cursor over a now-empty selection
                    // would promise a gesture the press would not start.
                    this.hover_resize_handle = None;
                    #[cfg(feature = "fanta-gpui-ui")]
                    {
                        // Panel and workspace selection changes can arrive
                        // while the item is already updating. Restore the
                        // property preview after that update releases its
                        // entity borrow, while leaving the new selection intact.
                        let view = cx.weak_entity();
                        cx.defer(move |cx| {
                            view.update(cx, |view, cx| {
                                view.finish_gpui_design_edits(cx);
                                // An already-dirty preview only emitted
                                // EditedTransient, so its rollback has no
                                // later heavyweight event to refresh the panel.
                                view.refresh_gpui_design(cx);
                            })
                            .log_err();
                        });
                    }
                }
                FigItemEvent::TextSelectionChanged => {}
                FigItemEvent::ViewportChanged(requester) => {
                    // MCP can navigate before a tab's deferred focus callbacks run.
                    if requester.map_or(this.is_focused, |view| view == cx.entity_id())
                        && let Some(viewport) = this.item.read(cx).doc().map(|doc| doc.viewport)
                    {
                        this.set_viewport(viewport, cx);
                    }
                }
                // A save wrote the document without replacing it, so
                // nothing view-side is stale: only the tab's dirty mark
                // changes. Deliberately NOT `StateChanged`, which every
                // listener reads as a reload and answers by dropping
                // in-flight sessions.
                FigItemEvent::Saved => {
                    this.reconcile_opened_entry_with_project_root(cx);
                    this.clear_save_failure(cx);
                    cx.emit(FigViewEvent::TitleChanged);
                }
                FigItemEvent::StateChanged => {
                    this.measurement_selection = None;
                    this.annotation_state.selection = None;
                    this.freeze_measurement_drag(cx);
                    this.freeze_annotation_move(cx);
                    this.invalidate_local_media_origin();
                    this.comment_state.clear_pending_motion_anchor();
                    this.motion_auto_keyframe = false;
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
                    this.motion_sidebar.update(cx, |panel, cx| panel.discard_continuous_edits(cx));
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
                FigItemEvent::SourceReloadFailed => {
                    if let Some(message) = this.item.read(cx).source_reload_error() {
                        show_canvas_notice_deferred(message.to_string(), cx);
                    }
                    cx.emit(FigViewEvent::TitleChanged);
                }
                FigItemEvent::ConflictChanged => {
                    if this.item.read(cx).has_conflict() && this.item.read(cx).source_reload_error().is_none() {
                        show_canvas_notice_deferred(
                            "External edits conflict with unsaved canvas edits — save or reload to resolve"
                                .to_string(),
                            cx,
                        );
                    }
                    cx.emit(FigViewEvent::TitleChanged);
                }
                FigItemEvent::SourceEditLockChanged => {
                    this.invalidate_local_media_origin();
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
                    this.invalidate_local_media_origin();
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
                    this.freeze_measurement_drag(cx);
                    this.freeze_annotation_move(cx);
                    this.reconcile_annotations(cx);
                    let root = this
                        .item
                        .read(cx)
                        .document()
                        .and_then(|document| document.doc.active_page());
                    if follows && this.last_seen_root != root {
                        this.last_seen_root = root;
                        this.comment_state.clear_pending_motion_anchor();
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
                                this.invalidate_canvas_presentation();
                            }
                        }
                        cx.emit(FigViewEvent::TitleChanged);
                        cx.notify();
                    }
                }
                FigItemEvent::RevealNodes { nodes, view } => {
                    let follows = match view {
                        Some(view) => *view == cx.entity_id(),
                        None => this.is_focused,
                    };
                    if follows {
                        this.reveal_nodes(nodes, cx);
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
        self.reconcile_annotations(cx);
        if !self.item.read(cx).source_edit_locked() {
            return;
        }
        if self.measurement_controller.has_pending_authoring() {
            self.measurement_controller.freeze_after_release_error();
            self.end_gesture_perf("pointer_state_reset");
            self.primary_pressed = false;
            self.canvas_pointer_down = false;
            show_canvas_notice_deferred("The document became read-only. The measurement draft was kept; cancel it before starting again.".into(), cx);
            return;
        }
        if self.is_inspecting() {
            return;
        }

        self.comment_state.clear_pending_motion_anchor();
        #[cfg(feature = "fanta-gpui-ui")]
        self.finish_gpui_design_edits(cx);
        self.motion_sidebar
            .update(cx, |panel, cx| panel.cancel_continuous_edits(cx));
        self.set_motion_auto_keyframe_state(false, cx);
        self.cancel_motion_keyframe_drag(cx);
        self.timeline_shell
            .update(cx, |timeline, cx| timeline.set_authoring_enabled(false, cx));
        self.end_gesture_perf("pointer_state_reset");
        self.primary_pressed = false;
        self.pending_text_edit = None;
        let text_session = self.text_edit.take().map(|edit| edit.session);
        let mut viewport = self.viewport;
        let screen_size = self.container_bounds.map(|bounds| {
            let (width, height) = bounds_size(bounds);
            DVec2::new(width, height)
        });
        let tools = &mut self.tools;
        let preview_owner = cx.entity_id();
        self.item.update(cx, |item, cx| {
            item.with_document_for_preview_owner(preview_owner, cx, |document| {
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
            item.finish_content_preview(preview_owner, false, cx);
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

    pub(crate) fn finish_panel_edits(&mut self, cx: &mut Context<Self>) {
        if self
            .inspector_sidebar
            .read(cx)
            .finishing_edits_requires_text_commit(cx)
        {
            self.commit_text_edit(cx);
        }
        self.inspector_sidebar
            .update(cx, |panel, cx| panel.finish_continuous_edits_from_view(cx));
        self.motion_sidebar
            .update(cx, |panel, cx| panel.finish_continuous_edits(cx));
        self.prototype_sidebar
            .update(cx, |panel, cx| panel.finish_parameter_edit(cx));
        #[cfg(feature = "fanta-gpui-ui")]
        self.finish_gpui_design_edits(cx);
    }

    fn finish_document_edits(&mut self, cx: &mut Context<Self>) {
        self.commit_text_edit(cx);
        if self.pending_bound_text_error(cx).is_some() {
            return;
        }
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
        self.cancel_tool_gesture_if_active(cx);
        self.sync_motion_timeline(cx);
    }

    fn cancel_tool_gesture_if_active(&mut self, cx: &mut Context<Self>) {
        // Select can be interrupted before its first preview. Reset that
        // press too, so the next hover cannot promote it into a drag.
        let pending_selection = self.primary_pressed && self.tools.kind() == ToolKind::Select;
        if !pending_selection && !self.item.read(cx).content_preview_active() {
            return;
        }
        let preview_owner = cx.entity_id();
        let mut viewport = self.viewport;
        let screen_size = self.container_bounds.map(|bounds| {
            let (width, height) = bounds_size(bounds);
            DVec2::new(width, height)
        });
        let tools = &mut self.tools;
        let restored = self.item.update(cx, |item, cx| {
            let restored = item
                .with_document_for_preview_owner(preview_owner, cx, |document| {
                    let revision_before = document.doc.scene.revision();
                    if let (Some(viewport), Some(screen_size)) = (viewport.as_mut(), screen_size) {
                        let mut tool_context = tool_context(
                            &mut document.doc,
                            viewport,
                            screen_size,
                            ToolKind::Select,
                        );
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
                })
                .is_some();
            if restored {
                item.finish_content_preview(preview_owner, false, cx);
            }
            restored
        });
        if restored {
            self.viewport = viewport;
            self.end_gesture_perf("pointer_state_reset");
            self.primary_pressed = false;
            self.canvas_pointer_down = false;
            self.remember_tool_face(ToolKind::Select);
            self.invalidate_canvas_cache();
            cx.notify();
        }
    }

    pub(crate) fn pending_bound_text_error(&self, cx: &App) -> Option<String> {
        self.text_edit.as_ref().and_then(|edit| {
            self.item.read(cx).document().and_then(|document| crate::text_edit::content_edit_error(&document.doc, &edit.session))
        }).or_else(|| self.inspector_sidebar.read(cx).pending_bound_text_error(cx))
            .map(|error| format!("{error} Your draft was kept. Copy it if needed, then press Escape to cancel before continuing."))
    }

    pub(crate) fn finish_document_edits_for_external_change(&mut self, cx: &mut Context<Self>) {
        self.finish_document_edits(cx);
    }

    #[cfg(test)]
    pub(crate) fn code_workspace_for_test(&self) -> Entity<FantaCodeWorkspace> {
        self.code_workspace.clone()
    }

    #[cfg(test)]
    pub(crate) fn space_pan_for_test(&self) -> bool {
        self.space_pan
    }

    pub fn set_editor_workspace(&mut self, workspace: EditorWorkspace, cx: &mut Context<Self>) {
        if self.is_dev_mode(cx) {
            self.set_dev_workspace(workspace, cx);
            return;
        }
        if self.refuse_pending_annotation(cx) || self.refuse_pending_measurement(cx) {
            return;
        }
        if self.editor_workspace(cx) == workspace {
            return;
        }
        if self.is_inspecting()
            || matches!(self.tools.kind(), ToolKind::Measure | ToolKind::Annotation)
            || self.measurement_selection.is_some()
            || self.annotation_state.selection.is_some()
        {
            self.activate_tool(ToolKind::Select, cx);
        }
        self.invalidate_local_media_origin();
        self.comment_state.clear_pending_motion_anchor();
        if self.prototype_player.is_some() && workspace != EditorWorkspace::Canvas {
            self.exit_prototype_session(cx);
        }
        self.finish_document_edits(cx);
        if workspace != EditorWorkspace::Canvas {
            self.set_motion_auto_keyframe_state(false, cx);
            self.timeline_shell
                .update(cx, |timeline, cx| timeline.pause(cx));
        }
        if workspace == EditorWorkspace::Code {
            self.prefer_code_source(cx);
        }
        self.editor_session
            .update(cx, |session, cx| session.set_workspace(workspace, cx));
        self.sync_motion_timeline(cx);
        self.invalidate_canvas_cache();
        cx.notify();
    }

    /// The Code tab shows what the user came from: `variables.json` from the
    /// Variables workspace (or a view opened on it), otherwise the design's
    /// own source.
    fn prefer_code_source(&mut self, cx: &mut Context<Self>) {
        let file = if self.editor_workspace(cx) == EditorWorkspace::Variables
            || self.scope == Some(FigScope::Variables)
        {
            CodeWorkspaceFile::Variables
        } else {
            CodeWorkspaceFile::Fnx
        };
        self.code_workspace
            .update(cx, |workspace, cx| workspace.prefer_source(file, cx));
    }

    pub fn set_editor_mode(&mut self, mode: EditorMode, cx: &mut Context<Self>) {
        if mode == EditorMode::Dev || self.is_dev_mode(cx) {
            self.set_dev_mode_boundary(mode, cx);
            return;
        }
        if self.refuse_pending_annotation(cx) || self.refuse_pending_measurement(cx) {
            return;
        }
        self.comment_state.clear_pending_motion_anchor();
        if mode != EditorMode::Motion {
            self.set_motion_auto_keyframe_state(false, cx);
        }
        let previous_mode = self.editor_mode(cx);
        if previous_mode == mode {
            return;
        }
        if previous_mode == EditorMode::Draw && mode != EditorMode::Draw {
            self.tools.clear_draw_selection_region();
        }
        if self.is_inspecting()
            || matches!(self.tools.kind(), ToolKind::Measure | ToolKind::Annotation)
            || self.measurement_selection.is_some()
            || self.annotation_state.selection.is_some()
        {
            self.activate_tool(ToolKind::Select, cx);
        }
        self.invalidate_local_media_origin();
        if self.prototype_player.is_some() && mode != EditorMode::Prototype {
            self.exit_prototype_session(cx);
        }
        self.finish_document_edits(cx);
        if mode != EditorMode::Motion {
            self.timeline_shell
                .update(cx, |timeline, cx| timeline.pause(cx));
        }
        match mode {
            EditorMode::Draw => self.activate_tool(ToolKind::Brush, cx),
            EditorMode::Design if previous_mode == EditorMode::Draw => {
                self.activate_tool(ToolKind::Select, cx)
            }
            EditorMode::Design => {}
            _ => self.activate_tool(ToolKind::Select, cx),
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
            TimelineEvent::LoopModeChanged(loop_mode) => {
                self.persist_motion_clip_loop_mode(loop_mode, cx);
            }
            TimelineEvent::PlayheadChanged(_) => {
                let current_anchor =
                    self.comment_state
                        .pending_motion_anchor()
                        .is_some_and(|anchor| {
                            let current_time_ms = self
                                .timeline_shell
                                .read(cx)
                                .playhead_us()
                                .max(0)
                                .div_euclid(1_000)
                                .min(i64::from(u32::MAX))
                                as u32;
                            self.active_motion_clip == Some(anchor.clip)
                                && current_time_ms == anchor.time_ms
                        });
                if !current_anchor {
                    self.comment_state.clear_pending_motion_anchor();
                }
                self.invalidate_canvas_cache();
            }
            TimelineEvent::PlaybackChanged(playing) => {
                if playing && self.timeline_shell.read(cx).is_playing() {
                    self.motion_sidebar
                        .update(cx, |panel, cx| panel.finish_continuous_edits(cx));
                    self.comment_state.clear_pending_motion_anchor();
                }
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
                self.comment_state.clear_pending_motion_anchor();
                self.active_motion_clip = Some(clip);
                self.sync_motion_timeline(cx);
                self.comment_state.hovered_pin = None;
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
        self.finish_panel_edits(cx);
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
        self.finish_panel_edits(cx);
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
        let owner = cx.entity_id();
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
            item.with_document_for_preview_owner(owner, cx, |document| {
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
        let owner = cx.entity_id();
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
            item.with_document_for_preview_owner(owner, cx, |document| {
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
        let owner = cx.entity_id();
        let Some(mut session) = self.motion_keyframe_drag.take() else {
            return;
        };
        if !session.matches(keyframe) || !self.is_editable(cx) {
            self.restore_motion_keyframe_drag(session, cx);
            return;
        }
        self.item.update(cx, |item, cx| {
            item.with_document_for_preview_owner(owner, cx, |document| {
                session.preview(&mut document.doc, time_us);
                let change = if session.restore(&mut document.doc) {
                    DocChange::ContentPreview
                } else {
                    DocChange::None
                };
                ((), change)
            });
            let Some(operation) = session.operation() else {
                item.finish_content_preview(owner, false, cx);
                return;
            };
            if let Err(error) = item.apply_for_preview_owner(owner, operation, cx) {
                item.finish_content_preview(owner, false, cx);
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
        let owner = cx.entity_id();
        let Some(mut session) = self.motion_keyframe_drag.take() else {
            return;
        };
        if !session.matches(keyframe) || !self.is_editable(cx) {
            self.restore_motion_keyframe_drag(session, cx);
            return;
        }
        self.item.update(cx, |item, cx| {
            item.with_document_for_preview_owner(owner, cx, |document| {
                session.preview_easing(&mut document.doc, easing);
                let change = if session.restore(&mut document.doc) {
                    DocChange::ContentPreview
                } else {
                    DocChange::None
                };
                ((), change)
            });
            let Some(operation) = session.operation() else {
                item.finish_content_preview(owner, false, cx);
                return;
            };
            if let Err(error) = item.apply_for_preview_owner(owner, operation, cx) {
                item.finish_content_preview(owner, false, cx);
                log::error!("editing motion keyframe easing failed: {error:#}");
            }
        });
    }

    fn finish_motion_keyframe_drag(&mut self, cx: &mut Context<Self>) {
        let owner = cx.entity_id();
        let Some(session) = self.motion_keyframe_drag.take() else {
            return;
        };
        if !self.is_editable(cx) {
            self.restore_motion_keyframe_drag(session, cx);
            return;
        }
        self.item.update(cx, |item, cx| {
            item.with_document_for_preview_owner(owner, cx, |document| {
                let change = if session.restore(&mut document.doc) {
                    DocChange::ContentPreview
                } else {
                    DocChange::None
                };
                ((), change)
            });
            let Some(operation) = session.operation() else {
                item.finish_content_preview(owner, false, cx);
                return;
            };
            if let Err(error) = item.apply_for_preview_owner(owner, operation, cx) {
                item.finish_content_preview(owner, false, cx);
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
        let owner = cx.entity_id();
        self.item.update(cx, |item, cx| {
            item.with_document_for_preview_owner(owner, cx, |document| {
                let change = if session.restore(&mut document.doc) {
                    DocChange::ContentPreview
                } else {
                    DocChange::None
                };
                ((), change)
            });
            item.finish_content_preview(owner, false, cx);
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
        let preview_owner = cx.entity_id();
        self.item.update(cx, |item, cx| {
            if let Err(error) = item.apply_for_preview_owner(preview_owner, operation, cx) {
                log::error!("{action} failed: {error:#}");
            }
        });
    }

    fn delete_motion_keyframe(
        &mut self,
        keyframe: TimelineKeyframeSelection,
        cx: &mut Context<Self>,
    ) {
        self.finish_panel_edits(cx);
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
        self.finish_panel_edits(cx);
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
        self.finish_panel_edits(cx);
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

    /// The timeline's loop toggles are the clip's authored loop mode, so the
    /// runtime and exports play it the way the author previewed it.
    fn persist_motion_clip_loop_mode(
        &mut self,
        loop_mode: fanta_doc::LoopMode,
        cx: &mut Context<Self>,
    ) {
        if !self.is_editable(cx) {
            return;
        }
        let Some(clip_id) = self.active_motion_clip else {
            return;
        };
        let operation =
            self.item.read(cx).document().and_then(|document| {
                set_clip_loop_mode_operation(&document.doc, clip_id, loop_mode)
            });
        self.apply_motion_operation(operation, "changing motion clip loop mode", cx);
    }

    fn create_motion_clip(&mut self, cx: &mut Context<Self>) {
        self.finish_panel_edits(cx);
        self.finish_motion_keyframe_drag(cx);
        if !self.is_editable(cx) {
            return;
        }
        let clip_id = AnimationClipId::new();
        self.comment_state.clear_pending_motion_anchor();
        self.active_motion_clip = Some(clip_id);
        let preview_owner = cx.entity_id();
        self.item.update(cx, |item, cx| {
            if let Err(error) = item.apply_for_preview_owner(
                preview_owner,
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
        if let Err(error) = self.try_add_motion_keyframe(property, cx) {
            show_canvas_notice_deferred(
                format!("Could not add {} keyframe: {error:#}", property.label()),
                cx,
            );
        }
    }

    fn try_add_motion_keyframe(
        &mut self,
        property: TimelineProperty,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.finish_document_edits(cx);
        if !self.is_editable(cx) {
            anyhow::bail!("the canvas is not editable while another edit is active");
        }
        if self.item.read(cx).content_preview_active() {
            anyhow::bail!("finish or cancel the active canvas preview before adding a keyframe");
        }
        let clip_id = self
            .active_motion_clip
            .context("create an animation before adding a keyframe")?;
        let playhead_ms = self
            .timeline_shell
            .read(cx)
            .playhead_us()
            .max(0)
            .div_euclid(1_000)
            .min(i64::from(u32::MAX)) as u32;
        let preview_owner = cx.entity_id();
        let result = self.item.update(cx, |item, cx| {
            item.with_document_for_preview_owner(preview_owner, cx, |document| {
                let result: Result<bool> = (|| {
                    let node_id = single_selection(&document.doc)
                        .context("select one layer before adding a keyframe")?;
                    let (_, value) = evaluated_motion_value(
                        &document.doc,
                        clip_id,
                        node_id,
                        property,
                        playhead_ms,
                    )
                    .with_context(|| {
                        format!(
                            "{} is not available for the selected layer",
                            property.label()
                        )
                    })?;
                    let Some(operation) = upsert_motion_keyframe_operation(
                        &document.doc,
                        clip_id,
                        node_id,
                        property,
                        playhead_ms,
                        value,
                    ) else {
                        return Ok(false);
                    };
                    document
                        .doc
                        .apply(operation)
                        .context("applying the keyframe")?;
                    Ok(true)
                })();
                let change = if result.as_ref().is_ok_and(|changed| *changed) {
                    DocChange::Content
                } else {
                    DocChange::None
                };
                (result, change)
            })
        });
        result.context(
            "finish the current Save As operation or canvas preview before adding a keyframe",
        )??;
        self.sync_motion_timeline(cx);
        Ok(())
    }

    fn sync_motion_timeline(&mut self, cx: &mut Context<Self>) {
        let previous_clip = self.active_motion_clip;
        let authoring_enabled = self.is_editable(cx)
            && self.editor_workspace(cx) == EditorWorkspace::Canvas
            && self.editor_mode(cx) == EditorMode::Motion;
        let (model, loop_mode) = if let Some(document) = self.item.read(cx).document() {
            if self
                .active_motion_clip
                .is_none_or(|clip| document.doc.motion.clip(clip).is_none())
            {
                self.active_motion_clip = document.doc.motion.clips.keys().next().copied();
            }
            let loop_mode = self
                .active_motion_clip
                .and_then(|clip| document.doc.motion.clip(clip))
                .map(|clip| clip.settings.loop_mode)
                .unwrap_or_default();
            (
                motion_timeline_model(&document.doc, self.active_motion_clip),
                loop_mode,
            )
        } else {
            self.active_motion_clip = None;
            (TimelineViewModel::empty(), fanta_doc::LoopMode::Once)
        };
        if !authoring_enabled || self.active_motion_clip.is_none() {
            self.motion_auto_keyframe = false;
        }
        if self.active_motion_clip != previous_clip {
            self.comment_state.clear_pending_motion_anchor();
            self.comment_state.hovered_pin = None;
        }
        self.timeline_shell.update(cx, |timeline, cx| {
            if timeline.authoring_enabled() != authoring_enabled {
                timeline.set_authoring_enabled(authoring_enabled, cx);
            }
            timeline.set_model(model, cx);
            timeline.set_loop_mode(loop_mode, cx);
        });
        let active_clip = self.active_motion_clip;
        let auto_keyframe = self.motion_auto_keyframe;
        let motion_sidebar = self.motion_sidebar.downgrade();
        cx.defer(move |cx| {
            motion_sidebar
                .update(cx, |panel, cx| {
                    panel.set_active_clip(active_clip, cx);
                    panel.set_auto_keyframe(auto_keyframe, cx);
                })
                .log_err();
        });
    }

    fn set_motion_auto_keyframe_state(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let changed = self.motion_auto_keyframe != enabled;
        self.motion_auto_keyframe = enabled;
        self.motion_sidebar
            .update(cx, |panel, cx| panel.set_auto_keyframe(enabled, cx));
        if changed {
            cx.notify();
        }
    }

    fn set_motion_auto_keyframe(&mut self, enabled: bool, cx: &mut Context<Self>) -> Result<()> {
        if !enabled {
            self.motion_sidebar
                .update(cx, |panel, cx| panel.finish_continuous_edits(cx));
            self.set_motion_auto_keyframe_state(false, cx);
            return Ok(());
        }
        if self.motion_auto_keyframe {
            return Ok(());
        }

        self.finish_document_edits(cx);
        anyhow::ensure!(
            self.editor_workspace(cx) == EditorWorkspace::Canvas
                && self.editor_mode(cx) == EditorMode::Motion,
            "open the Motion canvas first"
        );
        let active_clip = self
            .active_motion_clip
            .context("create an animation clip first")?;
        let panel_owner = self.motion_sidebar.entity_id();
        {
            let item = self.item.read(cx);
            anyhow::ensure!(item.is_editable(), "the canvas is read-only");
            anyhow::ensure!(
                item.can_preview_for_owner(panel_owner) && !item.content_preview_active(),
                "finish the active canvas preview first"
            );
            let document = item.document().context("the document is still loading")?;
            anyhow::ensure!(
                document.doc.motion.clip(active_clip).is_some(),
                "the active animation no longer exists"
            );
            let node = single_selection(&document.doc)
                .context("select one layer before turning on Auto key")?;
            anyhow::ensure!(
                document.doc.scene.get(node).is_some(),
                "the selected layer no longer exists"
            );
        }

        self.timeline_shell
            .update(cx, |timeline, cx| timeline.pause(cx));
        self.set_motion_auto_keyframe_state(true, cx);
        self.sync_motion_timeline(cx);
        Ok(())
    }

    pub(crate) fn active_motion_comment_clip(&self, cx: &App) -> Option<AnimationClipId> {
        if self.editor_mode(cx) != EditorMode::Motion {
            return None;
        }
        let clip = self.active_motion_clip?;
        self.item
            .read(cx)
            .document()
            .and_then(|document| document.doc.motion.clip(clip))
            .map(|_| clip)
    }

    pub(crate) fn active_motion_clip_id(&self) -> Option<AnimationClipId> {
        self.active_motion_clip
    }

    #[cfg(feature = "fanta-gpui-ui")]
    fn arm_motion_time_comment(&mut self, cx: &mut Context<Self>) -> Result<u32> {
        self.finish_document_edits(cx);
        if self.comment_state.draft.is_some() {
            anyhow::bail!("send or cancel the current comment draft before starting another one");
        }
        if self.has_unsent_comment_reply(cx) {
            anyhow::bail!("send or clear the current reply before starting a time comment");
        }
        if self.editor_workspace(cx) != EditorWorkspace::Canvas
            || self.editor_mode(cx) != EditorMode::Motion
        {
            anyhow::bail!("open the Motion canvas before adding a time comment");
        }

        let preview_owner = cx.entity_id();
        let (clip, time_ms) = {
            let item = self.item.read(cx);
            if !item.can_preview_for_owner(preview_owner) || item.content_preview_active() {
                anyhow::bail!(
                    "finish the current Save As operation or canvas preview before adding a time comment"
                );
            }
            let document = item.document().context("the document is still loading")?;
            let page = document
                .doc
                .active_page()
                .context("open a page before adding a time comment")?;
            if !document.doc.pages().contains(&page) {
                anyhow::bail!("open a page before adding a time comment");
            }
            let clip = self
                .active_motion_clip
                .context("create an animation before adding a time comment")?;
            let duration_ms = document
                .doc
                .motion
                .clip(clip)
                .context("the active animation no longer exists")?
                .duration_ms;
            let time_ms = self
                .timeline_shell
                .read(cx)
                .playhead_us()
                .max(0)
                .div_euclid(1_000)
                .min(i64::from(duration_ms)) as u32;
            (clip, time_ms)
        };

        self.timeline_shell
            .update(cx, |timeline, cx| timeline.pause(cx));
        self.activate_tool(ToolKind::Comment, cx);
        if !self.arm_motion_comment_anchor(MotionCommentAnchor { clip, time_ms }, cx) {
            self.activate_tool(ToolKind::Select, cx);
            anyhow::bail!("open a page before adding a time comment");
        }
        cx.notify();
        Ok(time_ms)
    }

    pub(crate) fn navigate_to_motion_comment(
        &mut self,
        anchor: MotionCommentAnchor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_dev_mode(cx) {
            return;
        }
        self.finish_document_edits(cx);
        self.comment_state.clear_pending_motion_anchor();
        let Some(duration_ms) = self.item.read(cx).document().and_then(|document| {
            document
                .doc
                .motion
                .clip(anchor.clip)
                .map(|clip| clip.duration_ms)
        }) else {
            self.set_editor_workspace(EditorWorkspace::Canvas, cx);
            self.hovered_node = None;
            self.comment_state.hovered_pin = None;
            self.invalidate_canvas_cache();
            cx.notify();
            show_canvas_notice(
                "This comment's animation is no longer available.".to_string(),
                window,
                cx,
            );
            return;
        };

        self.active_motion_clip = Some(anchor.clip);
        self.set_editor_workspace(EditorWorkspace::Canvas, cx);
        self.set_editor_mode(EditorMode::Motion, cx);
        self.sync_motion_timeline(cx);
        self.timeline_shell.update(cx, |timeline, cx| {
            timeline.seek_to(i64::from(anchor.time_ms).saturating_mul(1_000), cx)
        });
        self.hovered_node = None;
        self.comment_state.hovered_pin = None;
        self.invalidate_canvas_cache();
        cx.notify();

        if anchor.time_ms > duration_ms {
            show_canvas_notice(
                format!(
                    "This comment was anchored at {}; the animation now ends at {}, so the timeline moved to its end.",
                    crate::comments::motion_comment_time_label(anchor.time_ms),
                    crate::comments::motion_comment_time_label(duration_ms)
                ),
                window,
                cx,
            );
        }
    }

    pub(crate) fn is_art_read_only(&self, cx: &App) -> bool {
        self.is_dev_mode(cx)
            || self.is_inspecting()
            || matches!(self.tools.kind(), ToolKind::Measure | ToolKind::Annotation)
            || self.measurement_selection.is_some()
            || self.annotation_state.selection.is_some()
            || self.annotation_state.controller.has_pending_authoring()
    }

    pub(crate) fn is_inspecting(&self) -> bool {
        self.tools.kind() == ToolKind::Inspect
    }

    pub(crate) fn draft_preserving_toolbar_focus_scope(&self, cx: &App) -> FocusHandle {
        #[cfg(feature = "fanta-gpui-ui")]
        if let Some(toolbar) = &self.gpui_toolbar {
            return toolbar.panel.focus_handle(cx);
        }
        let _ = cx;
        self.native_toolbar_focus.clone()
    }

    fn invalidate_local_media_origin(&self) {
        self.media_import_generation
            .set(self.media_import_generation.get().wrapping_add(1));
    }

    fn has_pending_authoring(&self, cx: &App) -> bool {
        self.primary_pressed
            || self.canvas_pointer_down
            || self.has_pending_authoring_except_pointer(cx)
    }

    fn has_pending_authoring_except_pointer(&self, cx: &App) -> bool {
        let inspector = self.inspector_sidebar.read(cx);
        let pending_edit = self.annotation_state.controller.has_pending_authoring()
            || self.measurement_controller.has_pending_authoring()
            || self.item.read(cx).content_preview_active()
            || self.text_edit.is_some()
            || self.pending_text_edit.is_some()
            || self.motion_keyframe_drag.is_some()
            || inspector.editing_field.is_some()
            || inspector.scrub.is_some()
            || inspector.picker.is_some()
            || inspector.gradient_editor.is_some()
            || self.motion_sidebar.read(cx).has_continuous_edit()
            || self.timeline_shell.read(cx).has_pending_authoring()
            || self.prototype_sidebar.read(cx).has_pending_parameter_edit()
            || self.layers_sidebar.read(cx).has_pending_authoring(cx)
            || self.tools.overlays.iter().any(|overlay| {
                matches!(
                    overlay,
                    fanta_tools::ToolOverlay::PreviewLine { .. }
                        | fanta_tools::ToolOverlay::PreviewRect { .. }
                        | fanta_tools::ToolOverlay::PreviewEllipse { .. }
                )
            });
        #[cfg(target_os = "macos")]
        let pending_edit = pending_edit
            || self
                .canvas_video
                .as_ref()
                .is_some_and(|session| session.trim.is_some());
        #[cfg(feature = "fanta-gpui-ui")]
        let pending_edit = pending_edit
            || self
                .gpui_design
                .as_ref()
                .is_some_and(|adapter| adapter.session.is_some());
        pending_edit
    }

    fn after_property_draft_commit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        resume: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) -> Task<Result<()>> + 'static,
    ) -> Task<Result<()>> {
        #[cfg(feature = "fanta-gpui-ui")]
        if let Some(adapter) = self.gpui_design.as_ref() {
            match adapter
                .panel
                .update(cx, |panel, cx| panel.try_commit_property_draft(window, cx))
            {
                Err(error) => return Task::ready(Err(error.into())),
                Ok(true) => {
                    let (sender, receiver) = futures::channel::oneshot::channel();
                    cx.defer_in(window, move |view, window, cx| {
                        let save = resume(view, window, cx);
                        if sender.send(save).is_err() {
                            log::debug!(
                                "property save was cancelled before its edit intent settled"
                            );
                        }
                    });
                    return cx.spawn(async move |_, _| {
                        receiver
                            .await
                            .context("property edit interrupted before save")?
                            .await
                    });
                }
                Ok(false) => {}
            }
        }
        resume(self, window, cx)
    }

    pub(crate) fn defer_after_preserved_design_draft(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
        _resume: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> bool {
        #[cfg(feature = "fanta-gpui-ui")]
        if self.gpui_design.as_ref().is_some_and(|adapter| {
            adapter.panel.update(_cx, |panel, cx| {
                panel.finish_preserved_property_draft(_window, cx)
            })
        }) {
            // The panel emits the commit; its host subscription must receive it
            // before the next action closes the remaining document edit sessions.
            _cx.defer_in(_window, _resume);
            return true;
        }
        false
    }

    fn activate_tool_from_action(
        &mut self,
        kind: ToolKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_dev_mode(cx) {
            self.activate_tool(kind, cx);
            return;
        }
        if self.refuse_pending_annotation(cx) {
            return;
        }
        if !matches!(
            kind,
            ToolKind::Inspect | ToolKind::Measure | ToolKind::Annotation
        ) && self.defer_after_preserved_design_draft(window, cx, move |this, _, cx| {
            this.activate_tool(kind, cx);
        }) {
            return;
        }
        self.activate_tool(kind, cx);
    }

    fn set_editor_mode_from_action(
        &mut self,
        mode: EditorMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if mode == EditorMode::Dev || self.is_dev_mode(cx) {
            self.set_editor_mode(mode, cx);
            return;
        }
        if self.refuse_pending_annotation(cx) {
            return;
        }
        if self.defer_after_preserved_design_draft(window, cx, move |this, _, cx| {
            this.set_editor_mode(mode, cx);
        }) {
            return;
        }
        self.set_editor_mode(mode, cx);
    }

    #[cfg(test)]
    pub(crate) fn inspector_for_test(&self) -> Entity<FantaPropertiesPanel> {
        self.inspector_sidebar.clone()
    }

    #[cfg(test)]
    pub(crate) fn scope_for_test(&self) -> Option<FigScope> {
        self.scope
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
        if self
            .container_bounds
            .is_some_and(|current| current != bounds)
        {
            self.annotation_state.controller.freeze_move();
            self.measurement_controller.freeze_after_release_error();
            self.end_gesture_perf("pointer_state_reset");
            self.primary_pressed = false;
            self.canvas_pointer_down = false;
        }
        self.container_bounds = Some(bounds);
    }

    pub(crate) fn set_viewport_silent(&mut self, viewport: Viewport) {
        // Canvas prepaint repeats this setter on every frame; only a changed
        // projection invalidates an active drag.
        if self.viewport != Some(viewport)
            && (self.annotation_state.controller.is_moving()
                || self.measurement_controller.is_dragging())
        {
            self.annotation_state.controller.freeze_move();
            self.measurement_controller.freeze_after_release_error();
            self.end_gesture_perf("pointer_state_reset");
            self.primary_pressed = false;
            self.canvas_pointer_down = false;
        }
        self.viewport = Some(viewport);
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn clear_rendered_canvas(&mut self) {
        self.rendered_canvas = None;
    }

    /// Records the image the canvas element painted this frame and drops the one it painted
    /// before from the sprite atlas.
    pub(crate) fn replace_painted_canvas_image(
        &mut self,
        painted: &Arc<RenderImage>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if self
            .painted_canvas_image
            .as_ref()
            .is_some_and(|previous| Arc::ptr_eq(previous, painted))
        {
            return;
        }
        if let Some(previous) = self.painted_canvas_image.replace(painted.clone()) {
            cx.drop_image(previous, Some(window));
        }
    }

    fn release_painted_canvas_image(&mut self, cx: &mut App) {
        if let Some(image) = self.painted_canvas_image.take() {
            cx.drop_image(image, None);
        }
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

    fn invalidate_canvas_presentation(&mut self) {
        self.rendered_canvas = None;
        self.prototype_render_cache = None;
        self.chrome_cache.replace(None);
        #[cfg(target_os = "macos")]
        if let Some(gpu) = self.gpu_canvas.as_mut() {
            gpu.invalidate_presentation();
        }
    }

    pub(crate) fn is_editable(&self, cx: &App) -> bool {
        !self.is_art_read_only(cx) && self.item.read(cx).is_editable()
    }

    fn set_viewport(&mut self, viewport: Viewport, cx: &mut Context<Self>) {
        #[cfg(target_os = "macos")]
        if self.viewport != Some(viewport)
            && let Some(gpu) = &self.gpu_canvas
        {
            gpu.note_camera_input(viewport);
        }
        if self.annotation_state.controller.is_moving() || self.measurement_controller.is_dragging()
        {
            self.annotation_state.controller.freeze_move();
            self.measurement_controller.freeze_after_release_error();
            self.end_gesture_perf("pointer_state_reset");
            self.primary_pressed = false;
            self.canvas_pointer_down = false;
        }
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

    /// Select `nodes` and frame them, switching to their page first: what
    /// clicking an Agent mention of them does. Nodes that no longer exist are
    /// skipped.
    pub(crate) fn reveal_nodes(&mut self, nodes: &[NodeId], cx: &mut Context<Self>) {
        let selected_page = self.selected_page_index();
        let target = self.item.read(cx).document().and_then(|document| {
            let nodes = nodes
                .iter()
                .copied()
                .filter(|node| document.doc.scene.contains(*node))
                .collect::<Vec<_>>();
            let page = document.page_index_of_node(*nodes.first()?);
            Some((nodes, page, document.page_index(selected_page)))
        });
        let Some((nodes, page, current_page)) = target else {
            show_canvas_notice_deferred(
                "These layers are no longer in this design.".to_string(),
                cx,
            );
            return;
        };
        if let Some(page) = page
            && Some(page) != current_page
        {
            if self.item.read(cx).source_edit_locked() {
                show_canvas_notice_deferred(
                    "Save or discard source edits before showing layers on another page."
                        .to_string(),
                    cx,
                );
                return;
            }
            self.select_page(page, cx);
        }
        self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.selection.replace_with(nodes.clone());
                ((), DocChange::Selection)
            });
        });
        match nodes.as_slice() {
            [node] => self.focus_node(*node, cx),
            _ => self.zoom_to_selection(cx),
        }
    }

    // === Tool routing =====================================================

    fn defer_bitmap_wand_press(
        &mut self,
        event: ToolEvent,
        viewport: Viewport,
        screen_size: DVec2,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.tools.kind() != ToolKind::MagicWand
            || self.editor_mode(cx) != EditorMode::Draw
            || self.wand_override.is_some()
        {
            return false;
        }
        let ToolEvent::Pointer(fanta_tools::PointerEvent::Press {
            screen,
            button: ToolButton::Primary,
            ..
        }) = event
        else {
            return false;
        };
        self.wand_generation = self.wand_generation.wrapping_add(1);
        let generation = self.wand_generation;
        let world_click =
            fanta_canvas::screen_to_world(DVec2::from(screen), &viewport, screen_size);
        let Some((target, bitmap, transform, revision, raw_assets)) =
            self.item.read(cx).document().and_then(|document| {
                let doc = &document.doc;
                let target = fanta_canvas::hit_test_deep(
                    &doc.scene,
                    world_click,
                    HitPrecision::Path,
                    doc.active_page(),
                )
                .into_iter()
                .find_map(|leaf| {
                    fanta_tools::select::draw_content_target(&doc.scene, leaf, doc.active_page())
                })?;
                let node = doc.scene.get(target)?;
                let NodeData::Bitmap(bitmap) = &node.data else {
                    return None;
                };
                Some((
                    target,
                    bitmap.clone(),
                    doc.scene.world_transform(target)?,
                    doc.scene.revision(),
                    document.raw_assets.clone(),
                ))
            })
        else {
            return false;
        };

        if self
            .wand_asset_map
            .as_ref()
            .is_none_or(|previous| !Arc::ptr_eq(previous, &raw_assets))
        {
            self.wand_asset_map = Some(raw_assets.clone());
            self.wand_decode_cache.clear();
            self.wand_decode_order.clear();
        }
        let cached = self.wand_decode_cache.get(&bitmap.asset).cloned();
        let asset = bitmap.asset;
        #[cfg(feature = "fanta-gpui-ui")]
        let (tolerance, contiguous) = self
            .gpui_toolbar
            .as_ref()
            .map(|toolbar| {
                (
                    toolbar.draw_options.tolerance,
                    toolbar.draw_options.contiguous,
                )
            })
            .unwrap_or((32, true));
        #[cfg(not(feature = "fanta-gpui-ui"))]
        let (tolerance, contiguous) = (32, true);
        let worker_assets = raw_assets.clone();
        let work = cx.background_spawn(async move {
            let pixels = match cached {
                Some(pixels) => pixels,
                None => Arc::new(crate::draw_wand::decode_wand_thumbnail(
                    worker_assets
                        .get(&asset)
                        .context("The selected image data is unavailable")?,
                )?),
            };
            let region = crate::draw_wand::bitmap_wand_region(
                &pixels,
                &bitmap,
                transform,
                world_click,
                tolerance,
                contiguous,
            )?;
            Ok::<_, anyhow::Error>((pixels, region))
        });
        cx.spawn(async move |this, cx| {
            let computed = work.await;
            this.update(cx, |this, cx| {
                if this.wand_generation != generation
                    || this.tools.kind() != ToolKind::MagicWand
                    || this.editor_mode(cx) != EditorMode::Draw
                    || !this.item.read(cx).document().is_some_and(|document| {
                        Arc::ptr_eq(&document.raw_assets, &raw_assets)
                            && document.doc.scene.revision() == revision
                            && document.doc.scene.get(target).is_some_and(|node| {
                                matches!(&node.data, NodeData::Bitmap(bitmap) if bitmap.asset == asset)
                            })
                    })
                {
                    return;
                }
                match computed {
                    Ok((pixels, region)) => {
                        if !this.wand_decode_cache.contains_key(&asset) {
                            if this.wand_decode_order.len() >= 8 {
                                if let Some(oldest) = this.wand_decode_order.pop_front() {
                                    this.wand_decode_cache.remove(&oldest);
                                }
                            }
                            this.wand_decode_order.push_back(asset);
                            this.wand_decode_cache.insert(asset, pixels);
                        }
                        if let Some(region) = region {
                            this.wand_override = Some((target, region));
                            this.dispatch_tool_event(event, cx);
                        }
                    }
                    Err(error) => {
                        show_canvas_notice_deferred(
                            format!("Magic Wand selection failed: {error:#}"),
                            cx,
                        );
                    }
                }
            })
        })
        .detach_and_log_err(cx);
        true
    }

    fn click_diagnostic_snapshot(&self, cx: &App) -> ClickDiagnosticSnapshot {
        let selection = self.item.read(cx).doc().map(|doc| &doc.selection);
        let selection_count = selection.map_or(0, |selection| selection.len());
        ClickDiagnosticSnapshot {
            selection: selection
                .into_iter()
                .flat_map(|selection| selection.iter().copied())
                .take(CLICK_DIAGNOSTIC_SELECTION_LIMIT)
                .collect(),
            selection_count,
            selection_truncated: selection_count > CLICK_DIAGNOSTIC_SELECTION_LIMIT,
            tool: format!("{:?}", self.tools.kind()),
            mode: format!("{:?}", self.editor_mode(cx)),
            primary_pressed: self.primary_pressed,
            canvas_pointer_down: self.canvas_pointer_down,
            space_pan: self.space_pan,
            text_edit_active: self.text_edit.is_some(),
        }
    }

    fn begin_click_diagnostic(
        &mut self,
        stage: &'static str,
        input: impl FnOnce() -> serde_json::Value,
        cx: &App,
    ) -> Option<PendingClickDiagnostic> {
        let (sequence, input) = ClickDiagnostics::capture(self.click_diagnostics.as_mut(), input)?;
        Some(PendingClickDiagnostic {
            sequence,
            stage,
            input,
            before: self.click_diagnostic_snapshot(cx),
        })
    }

    fn finish_click_diagnostic(
        &mut self,
        pending: Option<PendingClickDiagnostic>,
        route: &'static str,
        cx: &Context<Self>,
    ) {
        let Some(pending) = pending else {
            return;
        };
        let record = serde_json::json!({
            "version": 1,
            "pid": std::process::id(),
            "view_id": cx.entity_id().as_u64(),
            "sequence": pending.sequence,
            "record_limit": CLICK_DIAGNOSTIC_LIMIT,
            "stage": pending.stage,
            "input": pending.input,
            "route": route,
            "before": pending.before,
            "after": self.click_diagnostic_snapshot(cx),
        });
        match serde_json::to_string(&record) {
            Ok(json) => log::info!("fanta_click_diagnostic {json}"),
            Err(error) => log::warn!("serializing canvas click diagnostics failed: {error}"),
        }
        #[cfg(test)]
        if let Some(diagnostics) = &mut self.click_diagnostics {
            diagnostics.records.push(record);
        }
    }

    /// Send one event through the active tool, tracking whether it changed
    /// document content, only the selection, or nothing.
    fn dispatch_tool_event(&mut self, event: ToolEvent, cx: &mut Context<Self>) {
        let diagnostic = match event {
            ToolEvent::Pointer(
                pointer @ (fanta_tools::PointerEvent::Press { .. }
                | fanta_tools::PointerEvent::Release { .. }),
            ) => self.begin_click_diagnostic(
                "tool_dispatch",
                || serde_json::json!({"coordinate_space":"canvas", "pointer":pointer}),
                cx,
            ),
            _ => None,
        };
        let route = self.dispatch_tool_event_inner(event, cx);
        self.finish_click_diagnostic(diagnostic, route, cx);
    }

    fn dispatch_tool_event_inner(
        &mut self,
        event: ToolEvent,
        cx: &mut Context<Self>,
    ) -> &'static str {
        let _perf_span = crate::gesture_perf::ui_span(
            self.gesture_perf.as_ref(),
            cx.entity_id().as_u64(),
            crate::gesture_perf::UiStage::ToolDispatch,
        );
        let owner = cx.entity_id();
        if self.handle_annotation_tool_event(event, cx)
            || self.handle_measurement_tool_event(event, cx)
        {
            return "annotation_or_measurement";
        }
        let Some(bounds) = self.container_bounds else {
            return "missing_bounds";
        };
        let Some(viewport) = self.viewport else {
            return "missing_viewport";
        };
        if self.handle_motion_selection_event(event, viewport, cx) {
            return "motion_selection";
        }
        let editable = self.is_editable(cx);
        if !editable && self.tools.kind() != ToolKind::Hand {
            self.handle_read_only_event(event, cx);
            return "read_only_selection";
        }

        let screen_size = {
            let (width, height) = bounds_size(bounds);
            DVec2::new(width, height)
        };
        if self.defer_bitmap_wand_press(event, viewport, screen_size, cx) {
            return "deferred_bitmap_wand";
        }
        let viewport_before = viewport;
        let mut viewport = viewport;
        let is_preview_move = matches!(
            event,
            ToolEvent::Pointer(fanta_tools::PointerEvent::Move { .. })
        );
        let overlays_before = self.tools.overlays.clone();
        let cursor_before = self.tools.cursor;
        let crop_ready_before = self.tools.can_apply_crop();

        let active_tool = self.tools.kind();
        #[cfg(feature = "fanta-gpui-ui")]
        let draw_stroke = (self.editor_mode(cx) == EditorMode::Draw
            && matches!(
                active_tool,
                ToolKind::Brush | ToolKind::Pencil | ToolKind::Eraser
            ))
        .then(|| {
            let draw = self
                .gpui_properties
                .as_ref()?
                .draw
                .read(cx)
                .view_data()
                .clone();
            let color = fanta_doc::Color::from_hex(&format!("#{}", draw.color_hex)).map(|color| {
                let opacity = u16::from(color.a) * u16::from(draw.options.opacity) / 100;
                fanta_doc::Color::rgba(color.r, color.g, color.b, opacity as u8)
            });
            let blend = match draw.blend_mode.as_ref() {
                "multiply" => fanta_doc::BlendMode::Multiply,
                "screen" => fanta_doc::BlendMode::Screen,
                "overlay" => fanta_doc::BlendMode::Overlay,
                _ => fanta_doc::BlendMode::Normal,
            };
            Some((
                color,
                f64::from(draw.options.size),
                0.2 + f64::from(draw.options.smoothing) * 0.058,
                blend,
                draw.options.hardness,
                draw.options.flow,
                match draw.options.brush_tip.as_ref() {
                    "Flat" => fanta_tools::BrushStyle::Flat,
                    "Ink" => fanta_tools::BrushStyle::Ink,
                    _ => fanta_tools::BrushStyle::Round,
                },
            ))
        })
        .flatten();
        let draw_content_only = self.editor_mode(cx) == EditorMode::Draw;
        #[cfg(feature = "fanta-gpui-ui")]
        let draw_selection_settings = ((draw_content_only
            && matches!(
                active_tool,
                ToolKind::RectangleSelect
                    | ToolKind::EllipseSelect
                    | ToolKind::Lasso
                    | ToolKind::PolygonalLasso
                    | ToolKind::MagicWand
                    | ToolKind::Crop
            ))
            || (self.editor_mode(cx) == EditorMode::Design && active_tool == ToolKind::Crop))
            .then(|| {
                let options = &self.gpui_toolbar.as_ref()?.draw_options;
                let operation = match options.selection_operation {
                    fanta_gpui::toolbar::DrawSelectionOperation::Replace => {
                        fanta_tools::select::RectangleSelectionOperation::Replace
                    }
                    fanta_gpui::toolbar::DrawSelectionOperation::Add => {
                        fanta_tools::select::RectangleSelectionOperation::Add
                    }
                    fanta_gpui::toolbar::DrawSelectionOperation::Subtract => {
                        fanta_tools::select::RectangleSelectionOperation::Subtract
                    }
                    fanta_gpui::toolbar::DrawSelectionOperation::Intersect => {
                        fanta_tools::select::RectangleSelectionOperation::Intersect
                    }
                };
                Some((
                    operation,
                    options.tolerance,
                    options.contiguous,
                    options.crop_ratio.to_string(),
                ))
            })
            .flatten();
        let wand_override = self.wand_override.take();
        let template_fill = self.template_fill;
        let tools = &mut self.tools;
        let mut wants_exit = false;
        let mut content_changed = false;
        let item = self.item.clone();
        item.update(cx, |item, cx| {
            let handle_event = |document: &mut FigDocument| {
                let revision_before = document.doc.scene.revision();
                let selection_before: Vec<NodeId> =
                    document.doc.selection.iter().copied().collect();

                let mut ctx =
                    tool_context(&mut document.doc, &mut viewport, screen_size, active_tool);
                ctx.draw_content_only = draw_content_only;
                ctx.bitmap_wand_override = wand_override;
                ctx.template_fill = template_fill;
                #[cfg(feature = "fanta-gpui-ui")]
                if let Some((color, width, smoothing, blend, hardness, flow, brush_style)) =
                    draw_stroke
                {
                    if let Some(color) = color {
                        ctx.new_shape_fill = color;
                    }
                    ctx.new_stroke_width = width;
                    ctx.stroke_smoothing = smoothing;
                    ctx.new_blend_mode = blend;
                    ctx.brush_hardness = hardness;
                    ctx.brush_flow = flow;
                    ctx.brush_style = brush_style;
                }
                #[cfg(feature = "fanta-gpui-ui")]
                if let Some((operation, tolerance, contiguous, ratio)) = &draw_selection_settings {
                    ctx.rectangle_selection_operation = *operation;
                    ctx.selection_tolerance = *tolerance;
                    ctx.selection_contiguous = *contiguous;
                    ctx.crop_aspect_ratio = match ratio.as_str() {
                        "1:1" => Some(1.0),
                        "4:3" => Some(4.0 / 3.0),
                        "3:2" => Some(3.0 / 2.0),
                        "16:9" => Some(16.0 / 9.0),
                        "Original" => ctx
                            .doc
                            .selection
                            .iter()
                            .next()
                            .and_then(|id| ctx.doc.scene.world_bounds(*id))
                            .filter(|bounds| bounds.height() > 0.0)
                            .map(|bounds| bounds.width() / bounds.height()),
                        _ => None,
                    };
                }
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
            };
            if is_preview_move {
                item.with_document_for_preview_owner(owner, cx, handle_event);
            } else {
                item.with_document_for_owner(owner, cx, handle_event);
            }
        });

        #[cfg(target_os = "macos")]
        if self.viewport != Some(viewport)
            && let Some(gpu) = &self.gpu_canvas
        {
            gpu.note_camera_input(viewport);
        }
        self.viewport = Some(viewport);
        if is_preview_move
            && content_changed
            && let Some(perf) = &self.gesture_perf
        {
            perf.borrow_mut().content_preview_move();
        }
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
            return "tool_requested_exit";
        }
        // Document and selection changes already notify through the item's
        // event stream; only repaint here when something view-local changed.
        // An unconditional notify would re-render the whole view (and wake
        // observers) on every idle mouse move.
        if content_changed
            || !crate::canvas::same_viewport(viewport_before, viewport)
            || self.tools.overlays != overlays_before
            || self.tools.cursor != cursor_before
            || self.tools.can_apply_crop() != crop_ready_before
        {
            cx.notify();
        }
        "tool_handled"
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
            screen,
            modifiers,
            button: ToolButton::Primary,
            ..
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
        let inspecting = self.is_inspecting();

        self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let hit = if inspecting {
                    inspect_hit_test_screen(
                        &document.doc.scene,
                        &viewport,
                        screen_size,
                        screen_point,
                        document.doc.active_page(),
                    )
                } else {
                    precise_hit_test_screen(
                        &document.doc.scene,
                        &viewport,
                        screen_size,
                        screen_point,
                        HitPrecision::Path,
                        document.doc.active_page(),
                    )
                };
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
        if self.is_dev_mode(cx) && self.refuse_dev_transition(cx) {
            return;
        }
        if self.tools.kind() != kind {
            self.wand_generation = self.wand_generation.wrapping_add(1);
            self.wand_override = None;
        }
        let kind = if self.is_dev_mode(cx) {
            match kind {
                ToolKind::Select => ToolKind::Inspect,
                ToolKind::Inspect | ToolKind::Hand | ToolKind::Measure | ToolKind::Annotation => {
                    kind
                }
                _ => {
                    show_canvas_notice_deferred("Switch to Design to edit artwork.".into(), cx);
                    return;
                }
            }
        } else {
            kind
        };
        if self.refuse_pending_annotation(cx) {
            return;
        }
        if self.measurement_controller.has_pending_authoring() {
            show_canvas_notice_deferred(
                "Finish or cancel the measurement drag before changing tools.".into(),
                cx,
            );
            return;
        }
        if matches!(kind, ToolKind::Measure | ToolKind::Annotation) {
            if self.tools.kind() == kind {
                return;
            }
            if self.has_pending_authoring(cx)
                || self.comment_state.draft.is_some()
                || self.has_unsent_comment_reply(cx)
            {
                show_canvas_notice_deferred(
                    format!(
                        "Finish or cancel the current edit before using {}.",
                        kind.label()
                    ),
                    cx,
                );
                return;
            }
            if !self.item.read(cx).is_editable() || self.mark_page(cx).is_none() {
                show_canvas_notice_deferred(
                    "Choose a visible, editable page for measurements and annotations.".into(),
                    cx,
                );
                return;
            }
            self.annotation_state.selection = None;
            self.measurement_selection = None;
            self.invalidate_local_media_origin();
            self.comment_state.close_thread();
            self.end_gesture_perf("tool_change");
            self.tools.activate_without_context(kind);
            self.remember_tool_face(kind);
            self.hovered_node = None;
            self.hover_resize_handle = None;
            self.sync_measurement_edit_barrier(cx);
            self.reveal_node_properties(cx);
            cx.notify();
            return;
        }
        if kind == ToolKind::Inspect {
            if self.is_inspecting() {
                return;
            }
            if self.has_pending_authoring(cx)
                || self.comment_state.draft.is_some()
                || self.has_unsent_comment_reply(cx)
            {
                show_canvas_notice_deferred(
                    "Finish or cancel the current edit before inspecting layers.".into(),
                    cx,
                );
                return;
            }
            if !self.is_design_canvas_mode(cx)
                || self.editor_workspace(cx) != EditorWorkspace::Canvas
                || self.prototype_player.is_some()
            {
                show_canvas_notice_deferred(
                    "Return to the Design canvas to inspect layers.".into(),
                    cx,
                );
                return;
            }
            self.invalidate_local_media_origin();
            self.comment_state.close_thread();
            self.end_gesture_perf("tool_change");
            self.tools.activate_without_context(kind);
            self.remember_tool_face(kind);
            self.hovered_node = None;
            self.hover_resize_handle = None;
            self.inspector_sidebar
                .update(cx, |panel, cx| panel.set_inspecting(true, cx));
            self.layers_sidebar
                .update(cx, |panel, cx| panel.set_inspecting(true, cx));
            self.reveal_node_properties(cx);
            self.invalidate_canvas_cache();
            cx.notify();
            return;
        }
        self.comment_state.clear_pending_motion_anchor();
        if kind.requires_editing() && !self.item.read(cx).is_editable() {
            return;
        }
        self.end_gesture_perf("tool_change");
        self.measurement_selection = None;
        self.annotation_state.selection = None;
        if kind != ToolKind::Comment {
            self.comment_state.draft = None;
            self.comment_state.hovered_pin = None;
        }
        // Switching tools is a document edit boundary for every inspector and
        // inline session, not only text.
        self.finish_document_edits(cx);
        let read_only = self.is_dev_mode(cx);
        self.inspector_sidebar
            .update(cx, |panel, cx| panel.set_inspecting(read_only, cx));
        self.layers_sidebar
            .update(cx, |panel, cx| panel.set_inspecting(read_only, cx));
        self.hover_resize_handle = None;
        self.hovered_node = None;
        let activated_kind = if kind == ToolKind::TextPath {
            ToolKind::Select
        } else {
            kind
        };
        let Some((bounds, viewport)) = self.container_bounds.zip(self.viewport) else {
            self.tools.activate_without_context(activated_kind);
            self.remember_tool_face(kind);
            if kind == ToolKind::TextPath {
                self.convert_selection_to_text_path(cx);
            }
            cx.notify();
            return;
        };
        let (width, height) = bounds_size(bounds);
        let screen_size = DVec2::new(width, height);
        let mut viewport = viewport;

        let owner = cx.entity_id();
        let tools = &mut self.tools;
        let item = self.item.clone();
        item.update(cx, |item, cx| {
            item.with_document_for_owner(owner, cx, |document| {
                let revision_before = document.doc.scene.revision();
                let mut ctx = tool_context(
                    &mut document.doc,
                    &mut viewport,
                    screen_size,
                    activated_kind,
                );
                tools.activate(activated_kind, &mut ctx);
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
        if kind == ToolKind::TextPath {
            self.convert_selection_to_text_path(cx);
        }
        cx.notify();
    }

    fn convert_selection_to_text_path(&mut self, cx: &mut Context<Self>) {
        let conversion = self
            .item
            .read(cx)
            .document()
            .map(|document| fanta_tools::text_path_conversion(&document.doc));
        let Some(conversion) = conversion else {
            show_canvas_notice_deferred("The document is still loading.".to_owned(), cx);
            return;
        };
        let (node_id, operation) = match conversion {
            Ok(conversion) => conversion,
            Err(error) => {
                show_canvas_notice_deferred(error.to_string(), cx);
                return;
            }
        };
        let preview_owner = cx.entity_id();
        let result = self.item.update(cx, |item, cx| {
            item.apply_for_preview_owner(preview_owner, operation, cx)
        });
        match result {
            Ok(()) => {
                self.pending_text_edit = Some(node_id);
                self.invalidate_canvas_cache();
            }
            Err(error) => {
                show_canvas_notice_deferred(
                    format!("Text on Path could not be created: {error:#}"),
                    cx,
                );
            }
        }
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

    pub(crate) fn reveal_node_properties(&mut self, cx: &mut Context<Self>) {
        self.inspector_sidebar_visible = true;
        if self.pane_sidebar_layout().inspector.is_none() {
            self.revealed_compact_sidebar = Some(SidebarKind::Inspector);
        }
        cx.notify();
    }

    fn handle_node_double_click(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if event.button != MouseButton::Left
            || event.click_count < 2
            || !event.click_count.is_multiple_of(2)
            || event.modifiers != gpui::Modifiers::default()
            || self.space_pan
            || self.editor_mode(cx) != EditorMode::Design
            || self.tools.kind() != ToolKind::Select
            || !self.is_editable(cx)
        {
            return false;
        }
        let Some((bounds, viewport)) = self.container_bounds.zip(self.viewport) else {
            return false;
        };
        let screen = screen_position_in_bounds(event.position, bounds);
        let (width, height) = bounds_size(bounds);
        let screen_size = DVec2::new(width, height);
        let action = {
            let Some(doc) = self.item.read(cx).doc() else {
                return false;
            };
            let Some(selected) = single_selection(doc) else {
                return false;
            };
            let mut hit = precise_hit_test_screen(
                &doc.scene,
                &viewport,
                screen_size,
                screen,
                HitPrecision::Path,
                doc.active_page(),
            )
            .or_else(|| {
                fanta_tools::NodeEditTool::vector_at_screen_with_refiner(
                    doc,
                    &viewport,
                    screen_size,
                    doc.active_page(),
                    screen,
                    Some(crate::canvas::accepts_authoring_hit),
                )
            });
            while let Some(container) = hit
                && doc
                    .scene
                    .get(container)
                    .is_some_and(|node| matches!(node.data, NodeData::Boolean(_)))
                && doc
                    .scene
                    .ancestors_of(selected)
                    .any(|node| node.id == container)
            {
                let inner = precise_hit_test_screen(
                    &doc.scene,
                    &viewport,
                    screen_size,
                    screen,
                    HitPrecision::Path,
                    Some(container),
                );
                if inner == hit {
                    break;
                }
                hit = inner;
            }
            // The first pair enters a container; only a later pair can edit
            // its selected child. Hit testing also keeps covered or locked
            // selections from opening through a different visible layer.
            if hit != Some(selected) {
                return false;
            }
            let Some(node) = doc.scene.get(selected) else {
                return false;
            };
            match &node.data {
                NodeData::Group(_) | NodeData::Boolean(_) => return false,
                NodeData::Text(_) | NodeData::TextPath(_) => {
                    CanvasDoubleClickAction::Text(selected)
                }
                NodeData::Vector(_) => CanvasDoubleClickAction::Tool(ToolKind::NodeEdit),
                NodeData::Bitmap(_) => CanvasDoubleClickAction::Tool(ToolKind::Crop),
                NodeData::Instance(_) => CanvasDoubleClickAction::Instance,
                NodeData::Video(_)
                | NodeData::Audio(_)
                | NodeData::NodeGraph(_)
                | NodeData::Model3d(_)
                | NodeData::AiArtifact(_)
                | NodeData::Embed(_) => CanvasDoubleClickAction::Inspector,
            }
        };
        self.focus_handle.focus(window, cx);
        match action {
            CanvasDoubleClickAction::Text(node) => {
                self.open_text_edit(node, TextEditSeed::WordAt(screen), window, cx);
            }
            CanvasDoubleClickAction::Tool(kind) => self.activate_tool(kind, cx),
            CanvasDoubleClickAction::Instance => {
                if let Some(target) = self.instance_text_at(screen, cx) {
                    self.open_instance_text_edit(target, TextEditSeed::WordAt(screen), window, cx);
                } else {
                    self.reveal_node_properties(cx);
                }
            }
            CanvasDoubleClickAction::Inspector => self.reveal_node_properties(cx),
        }
        cx.notify();
        true
    }

    fn begin_gesture_perf(&self, window: &Window, cx: &Context<Self>) {
        let Some(perf) = &self.gesture_perf else {
            return;
        };
        let document = self.item.read(cx).doc();
        let context = crate::gesture_perf::GestureContext {
            view_id: format!("{:?}", cx.entity_id()),
            page_root: document
                .and_then(|doc| doc.active_page())
                .map(|id| id.to_string()),
            tool: self.tools.kind().label().to_owned(),
            selected_nodes: document
                .map(|doc| doc.selection.iter().map(ToString::to_string).collect())
                .unwrap_or_default(),
            viewport_zoom: self.viewport.map(|viewport| viewport.zoom),
            logical_canvas_size: self.container_bounds.map(bounds_size),
            display_scale: window.scale_factor(),
        };
        perf.borrow_mut().begin(context);
    }

    fn end_gesture_perf(&self, reason: &'static str) {
        if let Some(perf) = &self.gesture_perf {
            perf.borrow_mut().end(reason);
        }
    }

    fn handle_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let diagnostic = self.begin_click_diagnostic(
            "canvas_down",
            || {
                serde_json::json!({
                    "button":format!("{:?}", event.button),
                    "position":[f32::from(event.position.x), f32::from(event.position.y)],
                    "coordinate_space":"window",
                    "click_count":event.click_count,
                    "first_mouse":event.first_mouse,
                    "modifiers":click_diagnostic_modifiers(event.modifiers),
                    "window_modifiers":click_diagnostic_modifiers(window.modifiers()),
                })
            },
            cx,
        );
        let route = self.handle_mouse_down_inner(event, window, cx);
        self.finish_click_diagnostic(diagnostic, route, cx);
    }

    fn handle_mouse_down_inner(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> &'static str {
        if event.button == MouseButton::Left {
            // Set before any of the branches below can return early: the
            // autosave stands down for the whole gesture, including the text
            // and comment paths that never reach a tool.
            self.end_gesture_perf("superseded_by_press");
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
            return "prototype";
        }
        if event.button == MouseButton::Left && self.is_editable(cx) {
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
            return "text_edit";
        }

        if self.handle_annotation_mouse_down(event, window, cx)
            || self.handle_measurement_mouse_down(event, window, cx)
            || self.handle_node_double_click(event, window, cx)
        {
            return "annotation_measurement_or_double_click";
        }

        self.focus_handle.focus(window, cx);
        let Some(bounds) = self.container_bounds else {
            return "missing_bounds";
        };

        // Comments: pin clicks open threads with any tool; in comment mode a
        // canvas click opens the draft composer (nothing hits the doc until
        // Send) — the original fanta flow.
        if event.button == MouseButton::Left
            && !self.is_inspecting()
            && self.handle_comment_mouse_down(event.position, window, cx)
        {
            return "comment";
        }

        // Middle-drag, or a left-drag while space is held, pans regardless of
        // the active tool.
        if event.button == MouseButton::Middle
            || (event.button == MouseButton::Left && self.space_pan)
        {
            self.pan_last_position = Some(event.position);
            cx.notify();
            return "pan";
        }

        let Some(button) = pointer_button(event.button) else {
            return "unsupported_button";
        };
        if button == ToolButton::Primary {
            self.primary_pressed = true;
        }
        let screen = screen_position_in_bounds(event.position, bounds);
        self.dispatch_tool_event(
            press_event(screen, button, event.modifiers, event.click_count),
            cx,
        );
        if button == ToolButton::Primary && self.primary_pressed {
            self.begin_gesture_perf(window, cx);
        }
        "tool_press"
    }

    /// Track the space bar for hold-to-pan. Returns whether the key was the
    /// space bar (so the caller can stop it reaching tool shortcuts).
    fn handle_canvas_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.prototype_player.is_some() {
            self.trigger_prototype_key(&event.keystroke.key, cx);
            return;
        }
        // Text fields on the canvas (comment composers, annotation notes)
        // bubble their keys here; a typed space must stay a character.
        let typing = window
            .context_stack()
            .iter()
            .any(|context| context.contains("Editor") || context.contains("Input"));
        if event.keystroke.key == "space" && self.text_edit.is_none() && !typing && !self.space_pan
        {
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let diagnostic = self.begin_click_diagnostic(
            "canvas_up",
            || {
                serde_json::json!({
                    "button":format!("{:?}", event.button),
                    "position":[f32::from(event.position.x), f32::from(event.position.y)],
                    "coordinate_space":"window",
                    "click_count":event.click_count,
                    "modifiers":click_diagnostic_modifiers(event.modifiers),
                    "window_modifiers":click_diagnostic_modifiers(window.modifiers()),
                })
            },
            cx,
        );
        let route = self.handle_mouse_up_inner(event, cx);
        self.finish_click_diagnostic(diagnostic, route, cx);
    }

    fn handle_mouse_up_inner(
        &mut self,
        event: &MouseUpEvent,
        cx: &mut Context<Self>,
    ) -> &'static str {
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
            return "prototype";
        }
        if event.button == MouseButton::Middle {
            self.end_panning(cx);
        }
        // A space-pan drag ends on button release but space may still be held
        // for the next drag; end the pan, keep `space_pan`.
        if event.button == MouseButton::Left && self.space_pan && self.is_panning() {
            self.end_panning(cx);
            return "space_pan";
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
        "shared_release"
    }

    /// Element listeners stop firing once the cursor leaves the canvas, so
    /// the window listener must also be able to end an active gesture.
    pub(crate) fn handle_window_mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        let diagnostic = self.begin_click_diagnostic(
            "shared_up",
            || {
                serde_json::json!({
                    "button":format!("{:?}", event.button),
                    "position":[f32::from(event.position.x), f32::from(event.position.y)],
                    "coordinate_space":"window",
                    "click_count":event.click_count,
                    "modifiers":click_diagnostic_modifiers(event.modifiers),
                })
            },
            cx,
        );
        let route = self.handle_window_mouse_up_inner(event, cx);
        self.finish_click_diagnostic(diagnostic, route, cx);
    }

    fn handle_window_mouse_up_inner(
        &mut self,
        event: &MouseUpEvent,
        cx: &mut Context<Self>,
    ) -> &'static str {
        if event.button != MouseButton::Left {
            return "non_left_button";
        }
        // Cleared before the `primary_pressed` guard: a release that lands
        // outside the canvas comes through here only, and leaving the flag set
        // would hold the autosave off indefinitely.
        self.canvas_pointer_down = false;
        if !self.primary_pressed {
            return "primary_not_pressed";
        }
        let Some(bounds) = self.container_bounds else {
            self.end_gesture_perf("released_without_canvas_bounds");
            return "missing_bounds";
        };
        self.primary_pressed = false;
        let screen = screen_position_in_bounds(event.position, bounds);
        self.dispatch_tool_event(
            release_event(screen, ToolButton::Primary, event.modifiers),
            cx,
        );
        self.end_gesture_perf("released");
        "tool_release"
    }

    pub(crate) fn handle_window_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) {
        if self.annotation_state.controller.is_moving()
            && event.pressed_button != Some(MouseButton::Left)
        {
            self.freeze_annotation_move(cx);
            return;
        }
        if self.measurement_controller.is_dragging()
            && event.pressed_button != Some(MouseButton::Left)
        {
            self.freeze_annotation_move(cx);
            self.freeze_measurement_drag(cx);
            return;
        }

        if !self.primary_pressed {
            return;
        }
        if self.tools.kind() == ToolKind::Select && event.pressed_button != Some(MouseButton::Left)
        {
            self.end_gesture_perf("pointer_button_lost");
            self.cancel_tool_gesture_if_active(cx);
            return;
        }
        let Some(bounds) = self.container_bounds else {
            return;
        };
        if let Some(perf) = &self.gesture_perf {
            let mut perf = perf.borrow_mut();
            if event.pressed_button != Some(MouseButton::Left) {
                perf.end("pointer_button_lost");
            } else {
                perf.pointer_move();
            }
        }
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
        if !matches!(self.tools.kind(), ToolKind::Select | ToolKind::Inspect) {
            let had_handle = self.hover_resize_handle.take().is_some();
            if self.hovered_node.take().is_some() || had_handle {
                cx.notify();
            }
            return;
        }
        if self.is_inspecting() {
            self.hover_resize_handle = None;
        } else {
            self.update_hover_resize_handle(screen, cx);
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
                if self.is_inspecting() {
                    inspect_hit_test_screen(
                        &document.doc.scene,
                        &viewport,
                        DVec2::new(width, height),
                        screen,
                        document.doc.active_page(),
                    )
                } else if let Some(evaluation) = self.motion_evaluation(document, cx) {
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
                    precise_hit_test_screen(
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
        if self.is_inspecting() {
            return None;
        }
        let viewport = self.viewport?;
        let bounds = self.container_bounds?;
        let (width, height) = bounds_size(bounds);
        let document = self.item.read(cx).document()?;
        if self.tools.kind() == ToolKind::Scale {
            let (local, world) = fanta_tools::ScaleTool::selection_frame_with_resolver(
                &document.doc,
                document.doc.active_page(),
                Some(authored_local_bounds),
            )?;
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
        let local = authored_local_bounds(&document.doc.scene, *id)?;
        let world = local.try_transformed(&world_transform)?;
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

    fn undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_dev_mode(cx) && !self.dev_history_allowed(false, cx) {
            show_canvas_notice_deferred(
                "Dev can undo page marks only. Switch to Design for other history.".into(),
                cx,
            );
            return;
        }
        if (!self.is_editable(cx) && !self.can_edit_measurements(cx))
            || self.refuse_pending_annotation(cx)
            || self.refuse_pending_measurement(cx)
        {
            return;
        }
        if self.defer_after_preserved_design_draft(window, cx, |view, window, cx| {
            view.undo(&Undo, window, cx)
        }) {
            return;
        }
        self.finish_document_edits(cx);
        let previous_pages = self.item.read(cx).doc().map(|doc| doc.pages().to_vec());
        let changed = self.item.update(cx, |item, cx| match item.undo(cx) {
            Ok(changed) => changed,
            Err(error) => {
                log::error!("fig_viewer undo failed: {error:#}");
                false
            }
        });
        if changed {
            if previous_pages.as_deref() != self.item.read(cx).doc().map(|doc| doc.pages()) {
                self.sync_page_registry_mirrors(true, cx);
            }
            self.refresh_tool_overlays(cx);
            self.reconcile_annotations(cx);
        }
    }

    fn redo(&mut self, _: &Redo, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_dev_mode(cx) && !self.dev_history_allowed(true, cx) {
            show_canvas_notice_deferred(
                "Dev can redo page marks only. Switch to Design for other history.".into(),
                cx,
            );
            return;
        }
        if (!self.is_editable(cx) && !self.can_edit_measurements(cx))
            || self.refuse_pending_annotation(cx)
            || self.refuse_pending_measurement(cx)
        {
            return;
        }
        if self.defer_after_preserved_design_draft(window, cx, |view, window, cx| {
            view.redo(&Redo, window, cx)
        }) {
            return;
        }
        self.finish_document_edits(cx);
        let previous_pages = self.item.read(cx).doc().map(|doc| doc.pages().to_vec());
        let changed = self.item.update(cx, |item, cx| match item.redo(cx) {
            Ok(changed) => changed,
            Err(error) => {
                log::error!("fig_viewer redo failed: {error:#}");
                false
            }
        });
        if changed {
            if previous_pages.as_deref() != self.item.read(cx).doc().map(|doc| doc.pages()) {
                self.sync_page_registry_mirrors(true, cx);
            }
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
        self.end_gesture_perf("escape");
        if self.cancel_annotation(None, cx) {
            return;
        }
        if self.annotation_state.selection.take().is_some() {
            self.sync_measurement_edit_barrier(cx);
            cx.notify();
            return;
        }
        if self.cancel_measurement_drag(cx) {
            return;
        }
        if self.measurement_selection.take().is_some() {
            self.sync_measurement_edit_barrier(cx);
            cx.notify();
            return;
        }
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
            self.comment_state.close_thread();
            self.comment_state.reply_editor = None;
            cx.notify();
            return;
        }
        // Escape while editing text commits and exits the session (Figma
        // commits on escape). The dedicated text-edit key context routes
        // Escape here before Workspace actions can consume it.
        if self.text_edit.is_some() {
            if !self.cancel_bound_text_edit(cx) {
                self.commit_text_edit(cx);
            }
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
        if self.annotation_state.controller.has_pending_authoring()
            || self.tools.kind() == ToolKind::Annotation
            || self.annotation_state.selection.is_some()
        {
            return;
        }
        if self.measurement_controller.has_pending_authoring() {
            let result = self
                .item
                .read(cx)
                .doc()
                .context("The document is no longer available.")
                .and_then(|doc| self.measurement_controller.commit_intent(doc))
                .and_then(|intent| self.apply_measurement_intent(intent, cx));
            if let Err(error) = result {
                show_canvas_notice_deferred(format!("Measurement: {error:#}"), cx);
            }
            return;
        }
        if self.tools.kind() == ToolKind::Measure || self.measurement_selection.is_some() {
            return;
        }
        if self.is_inspecting() {
            return;
        }
        if self.prototype_player.is_some() {
            self.trigger_prototype_key("enter", cx);
            return;
        }
        if self.defer_after_preserved_design_draft(window, cx, |this, window, cx| {
            this.confirm(&Confirm, window, cx);
        }) {
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
        if self.tools.kind() == ToolKind::Select
            && self.editor_mode(cx) == EditorMode::Design
            && self.is_editable(cx)
            && self
                .item
                .read(cx)
                .doc()
                .and_then(fanta_tools::NodeEditTool::selected_editable_vector)
                .is_some()
        {
            self.activate_tool(ToolKind::NodeEdit, cx);
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
        if self.defer_after_preserved_design_draft(window, cx, |this, window, cx| {
            this.delete_selection(&DeleteSelection, window, cx);
        }) {
            return;
        }
        if matches!(self.tools.kind(), ToolKind::NodeEdit | ToolKind::PathSelect) {
            if self.is_editable(cx) {
                self.finish_document_edits(cx);
                self.dispatch_tool_event(key_event(LogicalKey::Delete, window.modifiers()), cx);
            }
        } else {
            self.delete_selected_nodes(cx);
        }
    }

    fn copy_selection(&mut self, _: &CopySelection, window: &mut Window, cx: &mut Context<Self>) {
        if self.defer_after_preserved_design_draft(window, cx, |this, window, cx| {
            this.copy_selection(&CopySelection, window, cx);
        }) {
            return;
        }
        self.copy_selected_nodes(cx);
    }

    fn cut_selection(&mut self, _: &CutSelection, window: &mut Window, cx: &mut Context<Self>) {
        if self.defer_after_preserved_design_draft(window, cx, |this, window, cx| {
            this.cut_selection(&CutSelection, window, cx);
        }) {
            return;
        }
        self.cut_selected_nodes(cx);
    }

    fn paste_selection(&mut self, _: &PasteSelection, window: &mut Window, cx: &mut Context<Self>) {
        if self.defer_after_preserved_design_draft(window, cx, |this, window, cx| {
            this.paste_selection(&PasteSelection, window, cx);
        }) {
            return;
        }
        self.paste_selected_nodes(cx);
    }

    fn duplicate_selection(
        &mut self,
        _: &DuplicateSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_after_preserved_design_draft(window, cx, |this, window, cx| {
            this.duplicate_selection(&DuplicateSelection, window, cx);
        }) {
            return;
        }
        self.duplicate_selected_nodes(cx);
    }

    fn group_selection(&mut self, _: &GroupSelection, window: &mut Window, cx: &mut Context<Self>) {
        if self.defer_after_preserved_design_draft(window, cx, |this, window, cx| {
            this.group_selection(&GroupSelection, window, cx);
        }) {
            return;
        }
        self.group_nodes(None, window, cx);
    }

    fn ungroup_selection(
        &mut self,
        _: &UngroupSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.defer_after_preserved_design_draft(window, cx, |this, window, cx| {
            this.ungroup_selection(&UngroupSelection, window, cx);
        }) {
            return;
        }
        self.ungroup_nodes(None, window, cx);
    }

    fn frame_selection(&mut self, _: &FrameSelection, window: &mut Window, cx: &mut Context<Self>) {
        if self.defer_after_preserved_design_draft(window, cx, |this, window, cx| {
            this.frame_selection(&FrameSelection, window, cx);
        }) {
            return;
        }
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
    pub(crate) fn apply_structure_edit(
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
    fn select_all(&mut self, _: &SelectAll, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.tools.kind(), ToolKind::Measure | ToolKind::Annotation)
            || self.measurement_controller.has_pending_authoring()
            || self.annotation_state.selection.is_some()
            || self.annotation_state.controller.has_pending_authoring()
        {
            return;
        }

        // The inline text session owns its own selection; selecting canvas
        // nodes underneath it mid-typing is never what the user asked for.
        if self.text_edit.is_some() {
            return;
        }
        if self.defer_after_preserved_design_draft(window, cx, |this, window, cx| {
            this.select_all(&SelectAll, window, cx);
        }) {
            return;
        }
        #[cfg(feature = "fanta-gpui-ui")]
        if self.editor_mode(cx) == EditorMode::Draw {
            self.select_all_draw_content(cx);
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
        if let Some(record) = self.selected_annotation(cx) {
            self.copy_annotation(&record, cx);
            return;
        }
        if let Some(record) = self.selected_measurement(cx) {
            self.copy_measurement(&record, cx);
            return;
        }
        if self.annotation_state.controller.has_pending_authoring()
            || self.tools.kind() == ToolKind::Annotation
            || self.tools.kind() == ToolKind::Measure
            || self.measurement_controller.has_pending_authoring()
        {
            return;
        }

        if !self.is_inspecting() {
            self.finish_document_edits(cx);
        }
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

    fn selected_nodes_are_editable(&self, cx: &App) -> bool {
        self.is_editable(cx)
            && self.item.read(cx).doc().is_some_and(|doc| {
                doc.selection
                    .iter()
                    .all(|id| crate::layer_context_ops::editable(doc, *id))
            })
    }

    pub(crate) fn cut_selected_nodes(&mut self, cx: &mut Context<Self>) {
        if !self.is_editable(cx) {
            return;
        }
        if self.selected_annotation(cx).is_some() || self.selected_measurement(cx).is_some() {
            self.copy_selected_nodes(cx);
            self.delete_selected_nodes(cx);
            return;
        }
        if self.annotation_state.controller.has_pending_authoring()
            || self.tools.kind() == ToolKind::Annotation
            || self.tools.kind() == ToolKind::Measure
            || self.measurement_controller.has_pending_authoring()
        {
            return;
        }
        self.remove_selected_nodes(true, cx);
    }

    pub(crate) fn delete_selected_nodes(&mut self, cx: &mut Context<Self>) {
        if let Some(record) = self.selected_annotation(cx) {
            self.delete_annotation(&record, cx);
            return;
        }
        if let Some(record) = self.selected_measurement(cx) {
            self.delete_measurement(&record, cx);
            return;
        }
        self.remove_selected_nodes(false, cx);
    }

    fn remove_selected_nodes(&mut self, cut: bool, cx: &mut Context<Self>) {
        if !self.selected_nodes_are_editable(cx) {
            return;
        }
        let action = if cut {
            "cut these layers"
        } else {
            "delete these layers"
        };
        if !self.template_allows_selection(Scope::LifecycleDestroy, true, action, cx) {
            return;
        }
        self.finish_document_edits(cx);
        let label = if cut { "Cut" } else { "Delete" };
        let result: Result<Option<CanvasClipboard>> = self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let Some(target) = crate::clipboard::editable_selection_roots(&document.doc)
                    .first()
                    .copied()
                else {
                    return (Ok(None), DocChange::None);
                };
                let operations =
                    match crate::layer_context_ops::delete_layers(&document.doc, target) {
                        Ok(operations) => operations,
                        Err(error) => return (Err(error), DocChange::None),
                    };
                let payload = if cut {
                    let Some(payload) = CanvasClipboard::capture(&document.doc) else {
                        return (Ok(None), DocChange::None);
                    };
                    if let Err(error) = payload.validate_cut() {
                        return (Err(error), DocChange::None);
                    }
                    Some(payload)
                } else {
                    None
                };
                match apply_canvas_transaction(&mut document.doc, label, operations) {
                    Ok(true) => {
                        document.doc.selection.clear();
                        (Ok(payload), DocChange::Content)
                    }
                    Ok(false) => (Ok(None), DocChange::None),
                    Err(error) => (Err(error), DocChange::None),
                }
            })
            .unwrap_or_else(|| Err(anyhow::anyhow!("the document is no longer available")))
        });
        match result {
            Ok(Some(payload)) => {
                cx.write_to_clipboard(ClipboardItem::new_string_with_json_metadata(
                    payload.display_text(),
                    payload,
                ));
            }
            Ok(None) => {}
            Err(error) => {
                log::error!("{label} canvas selection failed: {error:#}");
                show_canvas_notice_deferred(format!("{label} failed: {error:#}"), cx);
            }
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
        if !self.is_editable(cx) {
            show_canvas_notice_deferred(
                "Could not paste images: the canvas is read-only.".into(),
                cx,
            );
            return;
        }
        let viewport = self.viewport.unwrap_or(Viewport {
            center: [0.0, 0.0],
            zoom: 1.0,
        });
        let visible = self
            .container_bounds
            .map(bounds_size)
            .map(|(width, height)| [width / viewport.zoom, height / viewport.zoom])
            .unwrap_or([1024.0, 768.0]);
        let target_page = self
            .item
            .read(cx)
            .document()
            .and_then(|document| document.doc.active_page());
        let preparation = cx.background_spawn(async move {
            images
                .into_iter()
                .map(PreparedImage::new)
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let prepared = preparation.await;
            this.update(cx, |this, cx| {
                this.place_prepared_images(prepared, viewport.center, visible, target_page, cx)
            })
        })
        .detach_and_log_err(cx);
    }

    pub(crate) fn place_project_asset(
        &mut self,
        asset: AssetId,
        page: NodeId,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        anyhow::ensure!(
            self.is_editable(cx),
            "Save or discard source edits before placing an asset."
        );
        anyhow::ensure!(
            self.editor_workspace(cx) == EditorWorkspace::Canvas,
            "Open the canvas before placing a project asset."
        );
        let (page_index, bytes, kind, name) = {
            let item = self.item.read(cx);
            let document = item.document().context("The project is still loading.")?;
            let page_index = document
                .pages
                .iter()
                .position(|candidate| candidate.root == Some(page) && !candidate.hidden)
                .context("Choose an available page for this asset.")?;
            let bytes = document
                .raw_assets
                .get(&asset)
                .context("This project asset is no longer available.")?;
            let kind = document
                .doc
                .asset_library
                .get(&asset)
                .map(|entry| entry.kind)
                .or_else(|| generation_media::project_asset_kind(bytes))
                .context("This asset type cannot be placed on the canvas.")?;
            anyhow::ensure!(
                generation_media::project_asset_can_place(bytes, kind),
                "This asset format cannot be placed on the canvas."
            );
            let name = document
                .doc
                .asset_library
                .get(&asset)
                .map(|entry| entry.name.clone())
                .unwrap_or_else(|| {
                    match kind {
                        ProjectAssetKind::Image => "Project image",
                        ProjectAssetKind::Svg => "Project vector",
                        ProjectAssetKind::Video => "Project video",
                        ProjectAssetKind::Audio => "Project audio",
                    }
                    .to_owned()
                });
            (page_index, Arc::<[u8]>::from(bytes.as_slice()), kind, name)
        };
        self.select_page(page_index, cx);
        anyhow::ensure!(
            self.item
                .read(cx)
                .document()
                .and_then(|document| document.doc.active_page())
                == Some(page),
            "The selected page is unavailable while editing its source."
        );
        let viewport = self.viewport.unwrap_or(Viewport {
            center: [0.0, 0.0],
            zoom: 1.0,
        });
        let visible = self
            .container_bounds
            .map(bounds_size)
            .map(|(width, height)| [width / viewport.zoom, height / viewport.zoom])
            .unwrap_or([1024.0, 768.0]);
        let preparation = cx.background_spawn(async move {
            generation_media::prepare_canvas_asset(kind, bytes).await
        });
        cx.spawn(async move |this, cx| {
            let prepared = preparation.await;
            this.update(cx, |this, cx| {
                let result: Result<()> = (|| {
                    let media = prepared?;
                    anyhow::ensure!(
                        this.item
                            .read(cx)
                            .document()
                            .and_then(|document| document.doc.active_page())
                            == Some(page),
                        "The target page changed while preparing the asset. Choose it again."
                    );
                    anyhow::ensure!(
                        this.editor_workspace(cx) == EditorWorkspace::Canvas,
                        "Open the canvas before placing a project asset."
                    );
                    let natural = media.dimensions();
                    let mut scale = 1.0_f64;
                    for axis in 0..2 {
                        if visible[axis].is_finite() && visible[axis] > 0.0 {
                            scale = scale.min(visible[axis] * 0.8 / natural[axis]);
                        }
                    }
                    let size = [natural[0] * scale, natural[1] * scale];
                    let x = viewport.center[0] - size[0] * 0.5;
                    let y = viewport.center[1] - size[1] * 0.5;
                    this.item.update(cx, |item, cx| {
                        anyhow::ensure!(
                            item.is_editable(),
                            "Save or discard source edits before placing an asset."
                        );
                        item.with_document(cx, |document| {
                            if !document.raw_assets.contains_key(&asset) {
                                return (
                                    Err(anyhow::anyhow!(
                                        "This project asset was removed while it was loading."
                                    )),
                                    DocChange::None,
                                );
                            }
                            let (result, change) = match media {
                                PreparedCanvasAsset::Image { width, height } => {
                                    let result = (|| {
                                        let mut node = crate::structure::image_layer_node(
                                            &document.doc,
                                            asset,
                                            [width, height],
                                            size,
                                            None,
                                            x,
                                            y,
                                            None,
                                        )?;
                                        node.name = name.clone();
                                        let id = node.id;
                                        document.doc.apply(Operation::create_node(node))?;
                                        document.doc.selection.select_only(id);
                                        Ok(())
                                    })();
                                    let change = if result.is_ok() {
                                        DocChange::Content
                                    } else {
                                        DocChange::None
                                    };
                                    (result, change)
                                }
                                PreparedCanvasAsset::Svg(artwork) => {
                                    generation_media::place_svg_named(
                                        document,
                                        artwork,
                                        x,
                                        y,
                                        None,
                                        Some(&name),
                                        scale,
                                    )
                                }
                                PreparedCanvasAsset::Video(video) => {
                                    generation_media::place_video_named(
                                        document,
                                        video,
                                        x,
                                        y,
                                        None,
                                        Some(&name),
                                        Some(size),
                                    )
                                }
                                PreparedCanvasAsset::Audio { duration_us } => {
                                    generation_media::place_audio(
                                        document,
                                        asset,
                                        duration_us,
                                        &name,
                                        x,
                                        y,
                                        size,
                                    )
                                }
                            };
                            if result.is_ok() && kind != ProjectAssetKind::Image {
                                if let Some(id) =
                                    document.doc.scene.children_of(Some(page)).last().copied()
                                {
                                    document.doc.selection.select_only(id);
                                }
                            }
                            (result, change)
                        })
                        .context("The project is still loading.")?
                    })
                })();
                if let Err(error) = result {
                    log::warn!("Placing project asset failed: {error:#}");
                    show_canvas_notice_deferred(format!("Could not place asset: {error:#}"), cx);
                }
            })
            .log_err();
        })
        .detach();
        Ok(())
    }

    fn place_prepared_images(
        &mut self,
        images: Vec<Result<PreparedImage>>,
        center: [f64; 2],
        visible: [f64; 2],
        target_page: Option<NodeId>,
        cx: &mut Context<Self>,
    ) {
        if self
            .item
            .read(cx)
            .document()
            .and_then(|document| document.doc.active_page())
            != target_page
        {
            show_canvas_notice_deferred(
                "The active page changed before paste completed.".into(),
                cx,
            );
            return;
        }

        let pasted = self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let (doc, mut assets) = document.doc_and_assets();
                match paste_prepared_images(doc, &mut assets, images, center, visible) {
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
        if !self.selected_nodes_are_editable(cx) {
            return;
        }
        if !self.template_allows_selection(
            Scope::LifecycleDuplicate,
            false,
            "duplicate these layers",
            cx,
        ) {
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
        if !self.is_editable(cx) {
            return;
        }
        let result = self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let mut pasted = match payload.instantiate(&document.doc, offset, placement) {
                    Ok(pasted) => pasted,
                    Err(error) => return (Err(error), DocChange::None),
                };
                let operations = match placement {
                    ClipboardPlacement::Paste => {
                        match paste_operations(&document.doc, payload, &mut pasted) {
                            Ok(operations) => operations,
                            Err(error) => return (Err(error), DocChange::None),
                        }
                    }
                    ClipboardPlacement::Duplicate => {
                        match clone_component_operations(&document.doc, payload, &mut pasted) {
                            Ok(operations) => operations,
                            Err(error) => return (Err(error), DocChange::None),
                        }
                    }
                };
                match apply_canvas_transaction(&mut document.doc, label, operations) {
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
            show_canvas_notice_deferred(format!("{label} failed: {error:#}"), cx);
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
            let modifiers = window.modifiers();
            if self.defer_after_preserved_design_draft(window, cx, move |this, _, cx| {
                if this.is_editable(cx) {
                    this.finish_document_edits(cx);
                    this.dispatch_tool_event(key_event(key, modifiers), cx);
                }
            }) {
                return;
            }
            self.finish_document_edits(cx);
            self.dispatch_tool_event(key_event(key, modifiers), cx);
        }
    }

    pub(crate) fn is_presenting_prototype(&self) -> bool {
        self.prototype_player.is_some()
    }

    fn toggle_template_fill(
        &mut self,
        _: &ToggleTemplateFill,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.template_fill = !self.template_fill;
        let message = if self.template_fill {
            "Filling in the template: layers keep to what it allows."
        } else {
            "Editing the template."
        };
        show_canvas_notice_deferred(message.into(), cx);
        cx.notify();
    }

    /// Whether Fill mode lets a command change, in `scope`, every selected
    /// layer and, with `inside`, every layer within them. Says why not.
    fn template_allows_selection(
        &self,
        scope: Scope,
        inside: bool,
        action: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.template_fill {
            return true;
        }
        let allowed = self.item.read(cx).document().is_none_or(|document| {
            let doc = &document.doc;
            let allows = |id: NodeId| {
                doc.scene
                    .get(id)
                    .is_none_or(|node| node.editing_allows(scope))
            };
            doc.selection.iter().all(|selected| {
                if inside {
                    doc.scene.descendants_of(*selected).all(allows)
                } else {
                    allows(*selected)
                }
            })
        });
        if !allowed {
            show_canvas_notice_deferred(format!("The template doesn't let you {action}."), cx);
        }
        allowed
    }

    /// Whether Fill mode lets a command change `node` in `scope`. Says why
    /// not.
    pub(crate) fn template_allows_node(
        &self,
        node: NodeId,
        scope: Scope,
        action: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.template_fill {
            return true;
        }
        let allowed = self
            .item
            .read(cx)
            .doc()
            .and_then(|doc| doc.scene.get(node))
            .is_none_or(|node| node.editing_allows(scope));
        if !allowed {
            show_canvas_notice_deferred(format!("The template doesn't let you {action}."), cx);
        }
        allowed
    }

    fn play_prototype(&mut self, _: &PlayPrototype, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_dev_mode(cx) {
            show_canvas_notice_deferred("Switch to Design to present the prototype.".into(), cx);
            return;
        }
        if self.prototype_player.is_some() {
            return;
        }
        if self.defer_after_preserved_design_draft(window, cx, |this, window, cx| {
            this.play_prototype(&PlayPrototype, window, cx);
        }) {
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
        self.end_gesture_perf("pointer_state_reset");
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

    fn sync_page_registry_mirrors(&mut self, follow_active: bool, cx: &mut Context<Self>) {
        if self.scope == Some(FigScope::Variables) {
            return;
        }
        let Some(document) = self.item.read(cx).document() else {
            return;
        };
        let root = self.selected_page_root.or(self.last_seen_root);
        let root_valid = root.is_some_and(|root| {
            document.doc.scene.contains(root)
                && (document.doc.pages().contains(&root) || document.doc.is_component_root(root))
        });
        let next_root = if follow_active || !root_valid {
            document.doc.active_page()
        } else {
            root
        };
        let next_index = document
            .pages
            .iter()
            .position(|page| page.root == next_root);
        let next_scope = next_root.and_then(|root| {
            if document.doc.pages().contains(&root) {
                Some(FigScope::Page(root))
            } else {
                document
                    .doc
                    .components
                    .defs
                    .values()
                    .find(|definition| definition.root == root)
                    .map(|definition| FigScope::Component(definition.id))
            }
        });
        self.selected_page_index = next_index;
        self.selected_page_root = next_root;
        if root != next_root {
            self.scope = next_scope;
            self.last_seen_root = next_root;
            self.viewport = None;
            self.hovered_node = None;
            self.measurement_selection = None;
            self.freeze_annotation_move(cx);
            self.freeze_measurement_drag(cx);
            self.comment_state.clear_pending_motion_anchor();
            self.invalidate_local_media_origin();
            self.invalidate_canvas_presentation();
            cx.emit(FigViewEvent::TitleChanged);
        }
        cx.notify();
    }

    pub fn select_page(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(error) = self.pending_bound_text_error(cx) {
            show_canvas_notice_deferred(error, cx);
            return;
        }
        if self.refuse_pending_annotation(cx) || self.refuse_pending_measurement(cx) {
            return;
        }
        self.invalidate_local_media_origin();
        self.measurement_selection = None;
        self.annotation_state.selection = None;
        self.comment_state.clear_pending_motion_anchor();

        let item = self.item.read(cx);
        let Some(document) = item.document() else {
            return;
        };
        let Some(page) = document.pages.get(index) else {
            return;
        };
        if item.source_edit_locked() && document.doc.active_page() != page.root {
            return;
        }
        self.end_gesture_perf("page_change");
        // The edited node stays behind on the old page; end the session
        // before the canvas stops rendering it.
        self.finish_document_edits(cx);
        let Some((root, prewarm)) = self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.ensure_page_solved(index);
                let root = document.pages.get(index).and_then(|page| page.root);
                let changed = document.doc.set_active_page(root);
                if changed {
                    document.doc.selection.clear();
                }
                let prewarm = root.and_then(|root| document.take_page_prewarm(root));
                (
                    (root, prewarm),
                    if changed {
                        DocChange::Selection
                    } else {
                        DocChange::None
                    },
                )
            })
        }) else {
            return;
        };
        if let Some(prewarm) = prewarm {
            // Start the page's image decodes before its first frame asks for
            // them (see `PagePrewarm::run`).
            cx.background_spawn(async move { prewarm.run() }).detach();
        }
        let changed_for_view = self.last_seen_root != root;
        self.selected_page_index = Some(index);
        self.selected_page_root = root;
        // Focus re-assertion must follow in-tab navigation: this tab now
        // means this page, and the root it shows is already current.
        self.scope = root.map(|root| {
            self.item
                .read(cx)
                .doc()
                .and_then(|doc| {
                    doc.components
                        .defs
                        .values()
                        .find(|definition| definition.root == root)
                        .map(|definition| FigScope::Component(definition.id))
                })
                .unwrap_or(FigScope::Page(root))
        });
        self.last_seen_root = root;
        if changed_for_view {
            self.viewport = None;
            self.hovered_node = None;
            self.invalidate_canvas_presentation();
        }
        self.sync_measurement_edit_barrier(cx);
        if changed_for_view {
            cx.emit(FigViewEvent::TitleChanged);
        }
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
            && !self.has_pending_view_edits(cx)
            && item.is_dirty()
            && !item.unchanged_autosave_failed()
            && !item.has_conflict()
            && !item.external_reconciliation_pending()
            && !item.content_preview_active()
            && !item.source_edit_locked()
            // An open text session, a running prototype, or a keyframe drag
            // each hold document state that a write would freeze mid-gesture.
            && self.text_edit.is_none()
            && self.pending_text_edit.is_none()
            && self.prototype_player.is_none()
            && self.motion_keyframe_drag.is_none()
            && !self.measurement_controller.has_pending_authoring()
            && !self.annotation_state.controller.has_pending_authoring()
    }

    /// The debounce elapsed: write the document as it stands.
    ///
    /// Deliberately does NOT run `finish_document_edits`: that commits
    /// half-typed inspector values through `finish_panel_edits`. Any in-flight
    /// panel edit emits its own `Edited` when the user commits it, which
    /// re-arms this timer.
    /// Tell the user their work is not on disk. The notice is raised once and
    /// stays up until a save succeeds ([`Self::clear_save_failure`]); debounced
    /// retries that keep failing do not raise it again (their errors differ
    /// only in details such as the transaction's temp directory).
    fn report_save_failure(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        if self.save_failure_notice.borrow().is_some() {
            self.reported_save_failure
                .get_or_insert_with(|| format!("{error:#}"));
            return;
        }
        let item = self.item.read(cx);
        let name = item
            .project_root()
            .unwrap_or(&item.abs_path)
            .file_name()
            .map(|name| name.to_string_lossy().chars().take(48).collect::<String>())
            .unwrap_or_else(|| "Design".into());
        let reason = save_io_error_detail(error).unwrap_or_else(|| {
            "The save failed. Your changes are still open. Check the log for details and try Save again.".into()
        });
        *self.save_failure_notice.borrow_mut() =
            Some(format!("{name} could not be saved. {reason}").into());
        self.reported_save_failure
            .get_or_insert_with(|| format!("{error:#}"));
        sync_save_failure_notice_deferred(
            self.item.entity_id(),
            self.save_failure_notice.clone(),
            cx,
        );
    }

    fn clear_save_failure(&mut self, cx: &mut Context<Self>) {
        self.reported_save_failure = None;
        // Saved subscriptions may run while the shared item is borrowed.
        let visible = self.save_failure_notice.borrow_mut().take().is_some();
        if visible {
            sync_save_failure_notice_deferred(
                self.item.entity_id(),
                self.save_failure_notice.clone(),
                cx,
            );
        }
    }

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
        cx.spawn(async move |view, cx| {
            if let Err(error) = save.await {
                log::error!("autosaving the canvas failed: {error:#}");
                view.update(cx, |this, cx| {
                    this.report_save_failure(&error, cx);
                })
                .log_err();
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
        if let Some(error) = self.pending_bound_text_error(cx) {
            return Task::ready(Err(anyhow::anyhow!(error)));
        }
        if self.annotation_state.controller.has_pending_authoring() {
            return Task::ready(Err(anyhow::anyhow!(
                "Add, save or cancel the annotation first. Its draft was kept."
            )));
        }
        if self.measurement_controller.has_pending_authoring() {
            return Task::ready(Err(anyhow::anyhow!(
                "Finish or cancel the measurement drag before saving. Its preview was kept."
            )));
        }

        if self.prototype_player.is_some() {
            self.exit_prototype_session(cx);
        }
        // Persist committed state, not a transient preview mid-session.
        self.finish_document_edits(cx);
        self.item
            .update(cx, |item, cx| item.save(SaveKind::Explicit, cx))
    }

    pub fn toggle_layers_sidebar(
        &mut self,
        _: &ToggleLayersSidebar,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_layers_sidebar_visible(!self.layers_sidebar_expanded(), cx);
        cx.notify();
    }

    pub fn toggle_inspector_sidebar(
        &mut self,
        _: &ToggleInspectorSidebar,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(feature = "fanta-gpui-ui")]
        if let Some(adapter) = &self.gpui_properties {
            adapter
                .design
                .update(cx, |design, cx| design.deactivate(window, cx));
        }
        self.finish_panel_edits(cx);
        self.set_inspector_sidebar_visible(!self.inspector_sidebar_expanded(cx), window, cx);
        cx.notify();
    }

    fn pane_sidebar_layout(&self) -> PaneSidebarLayout {
        pane_sidebar_layout(
            self.pane_width,
            self.layers_sidebar_visible
                .then_some(self.layers_sidebar_width),
            self.inspector_sidebar_visible
                .then_some(self.inspector_sidebar_width),
        )
    }

    fn layers_sidebar_expanded(&self) -> bool {
        self.pane_sidebar_layout().layers.is_some()
            || (self.layers_sidebar_visible
                && self.revealed_compact_sidebar == Some(SidebarKind::Layers))
    }

    pub(crate) fn inspector_sidebar_expanded(&self, _cx: &App) -> bool {
        let keep_draft = self.inspector_has_focus;
        #[cfg(feature = "fanta-gpui-ui")]
        let keep_draft = keep_draft
            || self.gpui_design.as_ref().is_some_and(|adapter| {
                adapter.panel.read(_cx).property_draft_state(_cx)
                    != fanta_gpui::design::DesignPropertyDraftState::None
            });
        self.pane_sidebar_layout().inspector.is_some()
            || (self.inspector_sidebar_visible
                && (self.revealed_compact_sidebar == Some(SidebarKind::Inspector) || keep_draft))
    }

    fn displayed_sidebar_width(&self, sidebar: SidebarKind) -> Pixels {
        let layout = self.pane_sidebar_layout();
        let (docked, preferred) = match sidebar {
            SidebarKind::Layers => (layout.layers, self.layers_sidebar_width),
            SidebarKind::Inspector => (layout.inspector, self.inspector_sidebar_width),
        };
        docked.unwrap_or_else(|| {
            let width = preferred.min(self.pane_width.unwrap_or(preferred));
            match sidebar {
                SidebarKind::Layers => width,
                SidebarKind::Inspector => width.max(px(MIN_INSPECTOR_SIDEBAR_WIDTH)),
            }
        })
    }

    fn set_layers_sidebar_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.layers_sidebar_visible && self.pane_sidebar_layout().layers.is_none() {
            self.revealed_compact_sidebar = visible.then_some(SidebarKind::Layers);
        } else {
            self.layers_sidebar_visible = visible;
            if visible && self.pane_sidebar_layout().layers.is_none() {
                self.revealed_compact_sidebar = Some(SidebarKind::Layers);
            }
            self.persist_sidebar_layout(cx);
        }
    }

    fn set_inspector_sidebar_visible(
        &mut self,
        visible: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.inspector_sidebar_visible && self.pane_sidebar_layout().inspector.is_none() {
            self.revealed_compact_sidebar = visible.then_some(SidebarKind::Inspector);
            if !visible {
                self.inspector_has_focus = false;
                self.focus_handle.focus(window, cx);
            }
        } else {
            self.inspector_sidebar_visible = visible;
            if visible && self.pane_sidebar_layout().inspector.is_none() {
                self.revealed_compact_sidebar = Some(SidebarKind::Inspector);
            }
            self.persist_sidebar_layout(cx);
        }
    }

    fn pane_width_probe(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.weak_entity();
        gpui::canvas(
            move |bounds, _, cx| {
                view.update(cx, |view, cx| {
                    if view.pane_width != Some(bounds.size.width) {
                        view.pane_width = Some(bounds.size.width);
                        let layout = view.pane_sidebar_layout();
                        if matches!(view.revealed_compact_sidebar, Some(SidebarKind::Layers))
                            && layout.layers.is_some()
                            || matches!(view.revealed_compact_sidebar, Some(SidebarKind::Inspector))
                                && layout.inspector.is_some()
                        {
                            view.revealed_compact_sidebar = None;
                        }
                        cx.notify();
                    }
                })
                .log_err();
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
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
        self.layers_sidebar.update(cx, |panel, cx| {
            panel.set_file_inspector_collapsed(!self.layers_sidebar_expanded(), cx);
        });
        let sidebar = div()
            .id("fanta-layers-sidebar")
            .relative()
            .when(self.layers_sidebar_expanded(), |sidebar| {
                sidebar
                    .h_full()
                    .w(self.displayed_sidebar_width(SidebarKind::Layers))
                    .flex_shrink_0()
                    .border_r(SIDEBAR_BORDER_WIDTH)
                    .border_color(cx.theme().colors().border)
            })
            .when(!self.layers_sidebar_expanded(), |sidebar| {
                sidebar
                    .absolute()
                    .top_0()
                    .left_0()
                    .w(px(256.))
                    .h(px(80.))
                    .occlude()
            })
            .child(self.layers_sidebar.clone())
            .when(self.layers_sidebar_expanded(), |sidebar| {
                sidebar.child(self.render_sidebar_resize_handle(SidebarKind::Layers, cx))
            });
        #[cfg(test)]
        let sidebar = sidebar.when(self.layers_sidebar_expanded(), |sidebar| {
            sidebar.debug_selector(|| "fanta-layers-sidebar".to_owned())
        });
        sidebar.into_any_element()
    }

    /// Whether the inspector shows the annotation panel in place of the
    /// mode's own panel.
    pub(crate) fn annotation_panel_active(&self, cx: &App) -> bool {
        self.annotation_state.controller.has_pending_authoring()
            || self.selected_annotation(cx).is_some()
            || self.tools.kind() == ToolKind::Annotation
    }

    fn render_inspector_sidebar(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let mode = self.editor_mode(cx);
        let view = cx.weak_entity();
        let tabs = EditorModeTabs::new(mode, move |mode, window, cx| {
            view.update(cx, |view, cx| {
                view.set_editor_mode_from_action(mode, window, cx)
            })
            .log_err();
        });
        if self.annotation_panel_active(cx) {
            return self.render_inspector_body(self.render_annotation_panel(cx), tabs, cx);
        }
        let body = match mode {
            EditorMode::Prototype => self.prototype_sidebar.clone().into_any_element(),
            EditorMode::Comments => self
                .render_comment_thread_page(window, cx)
                .unwrap_or_else(|| self.comments_sidebar.clone().into_any_element()),
            EditorMode::Motion => self.motion_sidebar.clone().into_any_element(),
            EditorMode::Design | EditorMode::Draw | EditorMode::Code | EditorMode::Dev => {
                let records = self.page_measurements(cx);
                let selected = self.selected_measurement(cx);
                if selected.is_some() || self.tools.kind() == ToolKind::Measure {
                    let rows = records
                        .iter()
                        .enumerate()
                        .map(|(index, record)| {
                            let label = record
                                .measurement()
                                .label()
                                .unwrap_or_else(|error| error.to_string());
                            (
                                SharedString::from(record.measurement().id.clone()),
                                SharedString::from(format!("Measurement {} · {label}", index + 1)),
                            )
                        })
                        .collect();
                    let view = cx.weak_entity();
                    let list = crate::properties_panel::MeasurementList::new(
                        rows,
                        selected
                            .as_ref()
                            .map(|record| record.measurement().id.clone().into()),
                        move |id, _, cx| {
                            if let Some(record) = records
                                .iter()
                                .find(|record| record.measurement().id.as_str() == id.as_ref())
                                .cloned()
                            {
                                view.update(cx, |view, cx| view.select_measurement(record, cx))
                                    .log_err();
                            }
                        },
                    );
                    let mut body = v_flex().size_full().overflow_hidden();
                    if let Some(record) = selected {
                        let view = cx.weak_entity();
                        let copy_record = record.clone();
                        let delete_view = view.clone();
                        let delete_record = record.clone();
                        let properties = crate::properties_panel::MeasurementProperties::new(
                            record
                                .measurement()
                                .label()
                                .unwrap_or_else(|error| error.to_string())
                                .into(),
                            record.measurement().start,
                            record.measurement().end,
                            self.can_edit_measurements(cx)
                                && !self.measurement_controller.has_pending_authoring(),
                            move |_, cx| {
                                view.update(cx, |view, cx| view.copy_measurement(&copy_record, cx))
                                    .log_err();
                            },
                            move |_, cx| {
                                delete_view
                                    .update(cx, |view, cx| {
                                        view.delete_measurement(&delete_record, cx)
                                    })
                                    .log_err();
                            },
                        );
                        body = body.child(properties);
                    }
                    return self.render_inspector_body(
                        body.child(list).into_any_element(),
                        tabs,
                        cx,
                    );
                }
                // The fanta-gpui DesignPanel replaces the legacy inspector
                // when its adapter mounted; `FANTA_GPUI_DESIGN=0` (or the
                // process-wide `FANTA_GPUI_UI=0`) keeps the legacy panel.
                #[cfg(feature = "fanta-gpui-ui")]
                let body = self.render_properties_content(mode, cx).unwrap_or_else(|| {
                    match self.gpui_design.as_ref() {
                        Some(adapter) => adapter.panel.clone().into_any_element(),
                        None => self.inspector_sidebar.clone().into_any_element(),
                    }
                });
                #[cfg(not(feature = "fanta-gpui-ui"))]
                let body = self.inspector_sidebar.clone().into_any_element();
                if records.is_empty() {
                    body
                } else {
                    let rows = records
                        .iter()
                        .enumerate()
                        .map(|(index, record)| {
                            let label = record
                                .measurement()
                                .label()
                                .unwrap_or_else(|error| error.to_string());
                            (
                                SharedString::from(record.measurement().id.clone()),
                                SharedString::from(format!("Measurement {} · {label}", index + 1)),
                            )
                        })
                        .collect();
                    let view = cx.weak_entity();
                    let list = crate::properties_panel::MeasurementList::new(
                        rows,
                        None,
                        move |id, _, cx| {
                            if let Some(record) = records
                                .iter()
                                .find(|record| record.measurement().id.as_str() == id.as_ref())
                                .cloned()
                            {
                                view.update(cx, |view, cx| view.select_measurement(record, cx))
                                    .log_err();
                            }
                        },
                    );
                    v_flex()
                        .size_full()
                        .min_w_0()
                        .child(div().flex_1().min_h_0().min_w_0().child(body))
                        .child(
                            div()
                                .debug_selector(|| "fanta-inspector-measurements".to_owned())
                                .h(px(180.0))
                                .flex_shrink_0()
                                .child(list),
                        )
                        .into_any_element()
                }
            }
        };
        let body = if self.is_design_canvas_mode(cx) && !self.page_annotations(cx).is_empty() {
            v_flex()
                .size_full()
                .min_w_0()
                .child(div().flex_1().min_h_0().min_w_0().child(body))
                .child(
                    div()
                        .debug_selector(|| "fanta-inspector-annotations".to_owned())
                        .h(px(150.0))
                        .flex_shrink_0()
                        .child(self.render_annotation_list(cx)),
                )
                .into_any_element()
        } else {
            body
        };
        self.render_inspector_body(body, tabs, cx)
    }

    fn render_inspector_body(
        &self,
        body: AnyElement,
        tabs: EditorModeTabs,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let sidebar = div()
            .id("fanta-inspector-sidebar")
            .track_focus(&self.inspector_focus_handle)
            .relative()
            .h_full()
            .w(self.displayed_sidebar_width(SidebarKind::Inspector))
            .flex_shrink_0()
            .border_l(SIDEBAR_BORDER_WIDTH)
            .border_color(cx.theme().colors().border)
            .child(
                v_flex()
                    .size_full()
                    .min_w_0()
                    .overflow_hidden()
                    .child(self.render_inspector_toolbar(false, cx))
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .flex_none()
                            .justify_center()
                            .py_1()
                            .border_t_1()
                            .border_b_1()
                            .border_color(cx.theme().colors().border)
                            .child(tabs),
                    )
                    .child(div().flex_1().min_h_0().min_w_0().child(body)),
            )
            .child(self.render_sidebar_resize_handle(SidebarKind::Inspector, cx));
        #[cfg(test)]
        let sidebar = sidebar.debug_selector(|| "fanta-inspector-sidebar".to_owned());
        sidebar.into_any_element()
    }

    fn render_inspector_toolbar(&self, collapsed: bool, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.weak_entity();
        h_flex()
            .id("fanta-inspector-controls")
            .flex_none()
            .gap_1()
            .px_2()
            .py_1()
            .bg(cx.theme().colors().panel_background)
            .when(collapsed, |toolbar| {
                toolbar
                    .debug_selector(|| "properties-inspector-floating-card".to_owned())
                    .rounded_xl()
                    .shadow_md()
                    .occlude()
            })
            .when(!collapsed, |toolbar| toolbar.w_full())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .on_pinch(|_, _, cx| cx.stop_propagation())
            .child(
                IconButton::new("inspector-zoom-out", IconName::Dash)
                    .tooltip(Tooltip::for_action_title("Zoom Out", &ZoomOut))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.zoom_out(&ZoomOut, window, cx);
                    })),
            )
            .child(
                PopoverMenu::new("inspector-zoom-menu")
                    .trigger(
                        Button::new(
                            "inspector-zoom-percent",
                            format!("{}%", self.current_zoom_percent(cx)),
                        )
                        .end_icon(Icon::new(IconName::ChevronDown).size(IconSize::XSmall)),
                    )
                    .menu(move |window, cx| {
                        let view = view.clone();
                        Some(ContextMenu::build(window, cx, move |mut menu, _, _| {
                            for percent in [25, 50, 100, 200] {
                                let view = view.clone();
                                menu = menu.entry(format!("{percent}%"), None, move |_, cx| {
                                    view.update(cx, |view, cx| view.zoom_to_percent(percent, cx))
                                        .log_err();
                                });
                            }
                            let selection_view = view.clone();
                            menu.separator()
                                .entry("Fit to View", None, move |window, cx| {
                                    view.update(cx, |view, cx| {
                                        view.fit_to_view(&FitToView, window, cx)
                                    })
                                    .log_err();
                                })
                                .entry("Fit to Selection", None, move |_, cx| {
                                    selection_view
                                        .update(cx, |view, cx| view.zoom_to_selection(cx))
                                        .log_err();
                                })
                        }))
                    }),
            )
            .child(
                IconButton::new("inspector-zoom-in", IconName::Plus)
                    .tooltip(Tooltip::for_action_title("Zoom In", &ZoomIn))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.zoom_in(&ZoomIn, window, cx);
                    })),
            )
            .child(div().h_5().mx_1().child(Divider::vertical()))
            .child(
                div().debug_selector(|| "zoombar-fit".to_owned()).child(
                    IconButton::new("inspector-fit-view", IconName::Maximize)
                        .tooltip(Tooltip::for_action_title("Fit to View", &FitToView))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.fit_to_view(&FitToView, window, cx);
                        })),
                ),
            )
            .when(!collapsed, |toolbar| toolbar.child(div().flex_1()))
            .when(self.editor_mode(cx) == EditorMode::Prototype, |toolbar| {
                let can_present = self.item.read(cx).document().is_some_and(|document| {
                    crate::prototype_player::prototype_entry_frame(&document.doc).is_some()
                });
                toolbar.child(
                    IconButton::new("inspector-play-prototype", IconName::PlayOutlined)
                        .disabled(!can_present)
                        .tooltip(Tooltip::for_action_title("Play Prototype", &PlayPrototype))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.play_prototype(&PlayPrototype, window, cx);
                        })),
                )
            })
            .child(
                IconButton::new("inspector-template-fill", IconName::Lock)
                    .toggle_state(self.template_fill)
                    .tooltip(Tooltip::for_action_title(
                        if self.template_fill {
                            "Edit Template"
                        } else {
                            "Fill In Template"
                        },
                        &ToggleTemplateFill,
                    ))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_template_fill(&ToggleTemplateFill, window, cx);
                    })),
            )
            .child(
                div()
                    .debug_selector(|| "properties-inspector-toggle".to_owned())
                    .child(
                        IconButton::new(
                            "inspector-toggle",
                            if collapsed {
                                IconName::ThreadsSidebarRightClosed
                            } else {
                                IconName::ThreadsSidebarRightOpen
                            },
                        )
                        .tooltip(Tooltip::text(if collapsed {
                            "Show Properties"
                        } else {
                            "Collapse Properties"
                        }))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.toggle_inspector_sidebar(&ToggleInspectorSidebar, window, cx);
                        })),
                    ),
            )
            .into_any_element()
    }

    fn render_workspace_tabs(&self, cx: &mut Context<Self>) -> AnyElement {
        let workspace = self.editor_workspace(cx);
        let view = cx.weak_entity();
        let tabs = EditorWorkspaceTabs::new(workspace, move |workspace, window, cx| {
            view.update(cx, |view, cx| {
                let previous_workspace = view.editor_workspace(cx);
                view.set_editor_workspace(workspace, cx);
                if previous_workspace == EditorWorkspace::Code
                    && workspace == EditorWorkspace::Canvas
                    && view.editor_workspace(cx) == EditorWorkspace::Canvas
                {
                    view.focus_handle(cx).focus(window, cx);
                }
            })
            .log_err();
        })
        .variables_visible(!self.is_dev_mode(cx));
        h_flex()
            .absolute()
            .top(px(8.0))
            .left_0()
            .right_0()
            .when(workspace == EditorWorkspace::Canvas, |tabs| {
                let layout = self.pane_sidebar_layout();
                tabs.left(layout.layers.unwrap_or_default())
                    .right(layout.inspector.unwrap_or_default())
                    .when(layout.layers.is_none(), |tabs| tabs.top(px(88.0)))
            })
            .justify_center()
            .child(div().occlude().child(tabs))
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
                                "Canvas editing is locked while source has unsaved changes. Pan and selection remain available; save or discard the source edits to resume editing.",
                            )
                            .size(LabelSize::Small)
                            .line_clamp(2),
                        ),
                    ),
            )
            .into_any_element()
    }

    fn render_sidebar_resize_handle(
        &self,
        sidebar: SidebarKind,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.weak_entity();
        let handle = div()
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
            .on_drag(SidebarResizeDrag { sidebar }, move |drag, offset, _, cx| {
                // GPUI starts dragging during bubble dispatch, after this move's
                // capture-phase resize listener. Apply that first movement here.
                view.update(cx, |view, cx| {
                    // The absolute handle is positioned against the sidebar's
                    // padding box, inside its one-pixel border.
                    let half_handle = SIDEBAR_RESIZE_HANDLE_SIZE / 2.;
                    match sidebar {
                        SidebarKind::Layers => {
                            view.layers_sidebar_width = clamp_sidebar_width(
                                view.layers_sidebar_width - SIDEBAR_BORDER_WIDTH - half_handle
                                    + offset.x,
                                sidebar,
                            );
                        }
                        SidebarKind::Inspector => {
                            view.inspector_sidebar_width = clamp_sidebar_width(
                                view.inspector_sidebar_width - SIDEBAR_BORDER_WIDTH + half_handle
                                    - offset.x,
                                sidebar,
                            );
                        }
                    }
                    cx.notify();
                })
                .log_err();
                cx.new(|_| drag.clone())
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
            .occlude();
        #[cfg(test)]
        let handle = handle.debug_selector(move || match sidebar {
            SidebarKind::Layers => "fanta-layers-sidebar-resize-handle".to_owned(),
            SidebarKind::Inspector => "fanta-inspector-sidebar-resize-handle".to_owned(),
        });
        handle.into_any_element()
    }

    fn render_tool_pill(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.is_dev_mode(cx) {
            return self.render_native_dev_toolbar(cx);
        }

        let editable = self.item.read(cx).is_editable();
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
            .track_focus(&self.native_toolbar_focus)
            .debug_selector(|| "fanta-canvas-toolbar".to_owned())
            .absolute()
            .bottom(px(16.))
            .left_0()
            .right_0()
            .justify_center()
            .child(
                h_flex()
                    .id("fanta-native-tool-pill")
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
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.activate_tool_from_action(face_kind, window, cx);
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
                                                            .handler(move |window, cx| {
                                                                if let Err(error) =
                                                                    view.update(cx, |this, cx| {
                                                                        this.activate_tool_from_action(kind, window, cx);
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
                            if self.layers_sidebar_expanded() {
                                IconName::ThreadsSidebarLeftOpen
                            } else {
                                IconName::ThreadsSidebarLeftClosed
                            },
                        )
                        .icon_size(IconSize::Small)
                        .toggle_state(self.layers_sidebar_expanded())
                        .icon_color(if self.layers_sidebar_expanded() {
                            Color::Accent
                        } else {
                            Color::Muted
                        })
                        .tooltip(Tooltip::text(if self.layers_sidebar_expanded() {
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
                            if self.inspector_sidebar_expanded(cx) {
                                IconName::ThreadsSidebarRightOpen
                            } else {
                                IconName::ThreadsSidebarRightClosed
                            },
                        )
                        .icon_size(IconSize::Small)
                        .toggle_state(self.inspector_sidebar_expanded(cx))
                        .icon_color(if self.inspector_sidebar_expanded(cx) {
                            Color::Accent
                        } else {
                            Color::Muted
                        })
                        .tooltip(Tooltip::text(if self.inspector_sidebar_expanded(cx) {
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

#[derive(Clone, Copy, Debug, PartialEq)]
struct PaneSidebarLayout {
    layers: Option<Pixels>,
    inspector: Option<Pixels>,
}

fn pane_sidebar_layout(
    pane_width: Option<Pixels>,
    mut layers: Option<Pixels>,
    mut inspector: Option<Pixels>,
) -> PaneSidebarLayout {
    let preferred_inspector = inspector;
    if let Some(pane_width) = pane_width {
        let budget = (pane_width - px(MIN_PANE_CANVAS_WIDTH)).max(px(0.0));
        let total = layers.unwrap_or_default() + inspector.unwrap_or_default();
        if total > budget {
            if let Some(width) = layers.as_mut() {
                *width = (*width - (total - budget)).max(px(MIN_LAYERS_SIDEBAR_WIDTH));
            }
            let remaining = budget - layers.unwrap_or_default();
            if let Some(width) = inspector.as_mut() {
                *width = (*width).min(remaining.max(px(MIN_INSPECTOR_SIDEBAR_WIDTH)));
            }
            if layers.unwrap_or_default() + inspector.unwrap_or_default() > budget {
                layers = None;
                inspector = preferred_inspector
                    .filter(|_| budget >= px(MIN_INSPECTOR_SIDEBAR_WIDTH))
                    .map(|width| budget.min(width));
            }
        }
    }
    PaneSidebarLayout { layers, inspector }
}

fn clamp_sidebar_width(width: Pixels, sidebar: SidebarKind) -> Pixels {
    let minimum = match sidebar {
        SidebarKind::Layers => MIN_LAYERS_SIDEBAR_WIDTH,
        SidebarKind::Inspector => MIN_INSPECTOR_SIDEBAR_WIDTH,
    };
    px(width.as_f32().clamp(minimum, MAX_SIDEBAR_WIDTH))
}

impl FigView {
    fn render_prototype_presentation(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
        let presentation_background = self
            .prototype_player
            .as_ref()
            .and_then(PrototypePlayerState::presentation)
            .and_then(|config| config.frame_color)
            .map(|color| {
                gpui::rgb(
                    (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b),
                )
                .into()
            })
            .unwrap_or_else(|| cx.theme().colors().editor_background);
        let device_size = fitted_presentation_device_size(
            self.prototype_player
                .as_ref()
                .and_then(PrototypePlayerState::presentation),
            (
                f32::from(window.viewport_size().width),
                f32::from(window.viewport_size().height),
            ),
        );
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
            .debug_selector(|| "fanta-prototype-presentation".to_owned())
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(presentation_background)
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .id("fig-prototype-container")
                            .debug_selector(|| "fig-prototype-container".to_owned())
                            .relative()
                            .overflow_hidden()
                            .cursor_pointer()
                            .when_some(device_size, |surface, (width, height)| {
                                surface
                                    .w(px(width))
                                    .h(px(height))
                                    .rounded_lg()
                                    .border_1()
                                    .border_color(chrome_border)
                                    .shadow_lg()
                            })
                            .when(device_size.is_none(), |surface| surface.size_full())
                            .on_hover(cx.listener(|view, hovered: &bool, _, cx| {
                                if !*hovered {
                                    view.leave_prototype_surface(cx);
                                }
                            }))
                            .on_scroll_wheel(cx.listener(Self::handle_scroll_wheel))
                            .on_mouse_down(MouseButton::Left, cx.listener(Self::handle_mouse_down))
                            .on_mouse_up(MouseButton::Left, cx.listener(Self::handle_mouse_up))
                            .on_mouse_move(cx.listener(Self::handle_mouse_move))
                            .child(CanvasElement::new(cx.entity())),
                    ),
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
    fn selected_canvas_audio_source(&self, cx: &App) -> Option<CanvasAudioSource> {
        let document = self.item.read(cx).document()?;
        let node = single_selection(&document.doc)?;
        if !crate::clipboard::node_is_on_active_page(&document.doc, node) {
            return None;
        }
        let NodeData::Audio(audio) = &document.doc.scene.get(node)?.data else {
            return None;
        };
        Some(CanvasAudioSource {
            scene: document.doc.scene.instance_id(),
            node,
            asset: audio.asset,
            bytes_identity: document
                .raw_assets
                .get(&audio.asset)
                .map(|bytes| (bytes.as_ptr() as usize, bytes.len())),
            time_range_us: audio.time_range_us,
            muted: audio.muted,
            volume_bits: audio.volume.to_bits(),
        })
    }

    fn clear_canvas_audio(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.canvas_audio.take() {
            session
                .playback
                .update(cx, |playback, cx| playback.close(cx));
            cx.notify();
        }
    }

    fn sync_canvas_audio(&mut self, window_active: bool, cx: &mut Context<Self>) {
        if self.canvas_video_removed.get()
            || self.prototype_player.is_some()
            || self.editor_workspace(cx) != EditorWorkspace::Canvas
            || !window_active
            || !matches!(
                self.editor_mode(cx),
                EditorMode::Motion | EditorMode::Prototype
            )
        {
            self.clear_canvas_audio(cx);
            return;
        }
        let source = self.selected_canvas_audio_source(cx);
        if self.canvas_audio.as_ref().map(|session| session.source) == source {
            return;
        }
        self.clear_canvas_audio(cx);
        let Some(source) = source else {
            return;
        };
        let (bytes, title) = {
            let item = self.item.read(cx);
            let Some(document) = item.document() else {
                return;
            };
            let Some(bytes) = document.raw_assets.get(&source.asset) else {
                return;
            };
            let Some(node) = document.doc.scene.get(source.node) else {
                return;
            };
            (
                Arc::<[u8]>::from(bytes.as_slice()),
                SharedString::from(node.name.clone()),
            )
        };
        let playback = cx.new(|_| {
            AudioPlaybackView::new(bytes, title)
                .with_levels(f32::from_bits(source.volume_bits), source.muted)
        });
        self.canvas_audio = Some(CanvasAudioSession { source, playback });
        cx.notify();
    }

    fn render_canvas_audio_controls(&self, cx: &App) -> Option<AnyElement> {
        let session = self.canvas_audio.as_ref()?;
        Some(
            div()
                .id("canvas-audio-controls")
                .debug_selector(|| "canvas-audio-controls".to_owned())
                .w_full()
                .h(px(190.))
                .flex_none()
                .border_t_1()
                .border_color(cx.theme().colors().border)
                .child(session.playback.clone())
                .into_any_element(),
        )
    }

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

    fn selected_canvas_video_fill_sources(&self, cx: &App) -> Vec<CanvasVideoFillSource> {
        let Some(document) = self.item.read(cx).document() else {
            return Vec::new();
        };
        let Some(node_id) = single_selection(&document.doc) else {
            return Vec::new();
        };
        if !crate::clipboard::node_is_on_active_page(&document.doc, node_id) {
            return Vec::new();
        }
        let Some(node) = document.doc.scene.get(node_id) else {
            return Vec::new();
        };
        video_fill_assets(node)
            .into_iter()
            .map(|asset| CanvasVideoFillSource {
                scene: document.doc.scene.instance_id(),
                node: node_id,
                asset,
                assets_identity: Arc::as_ptr(&document.raw_assets) as usize,
                bytes_identity: document
                    .raw_assets
                    .get(&asset)
                    .map(|bytes| (bytes.as_ptr() as usize, bytes.len())),
            })
            .collect()
    }

    pub(crate) fn selected_canvas_video_fill_assets(&self, cx: &App) -> Vec<AssetId> {
        self.selected_canvas_video_fill_sources(cx)
            .into_iter()
            .map(|source| source.asset)
            .collect()
    }

    fn remove_canvas_video_fill(&mut self, asset: AssetId, cx: &mut Context<Self>) {
        if let Some(session) = self.canvas_video_fills.remove(&asset) {
            if let Some(playback) = session.playback {
                playback.update(cx, |playback, cx| playback.close(cx));
            }
            self.rendered_canvas = None;
            cx.notify();
        }
    }

    fn clear_canvas_video_fills(&mut self, cx: &mut Context<Self>) {
        let assets = self.canvas_video_fills.keys().copied().collect::<Vec<_>>();
        for asset in assets {
            self.remove_canvas_video_fill(asset, cx);
        }
    }

    fn start_canvas_video_fill(&mut self, source: CanvasVideoFillSource, cx: &mut Context<Self>) {
        self.canvas_video_fill_generation = self.canvas_video_fill_generation.wrapping_add(1);
        let generation = self.canvas_video_fill_generation;
        let Some(document) = self.item.read(cx).document() else {
            return;
        };
        let raw_assets = document.raw_assets.clone();
        let resolver = document.asset_resolver.clone();
        let loading = cx.background_spawn(async move {
            let resolved;
            let bytes = if let Some(bytes) = raw_assets.get(&source.asset) {
                bytes.as_slice()
            } else {
                resolved = resolver
                    .as_ref()
                    .and_then(|resolver| resolver.resolve_bytes(source.asset))
                    .context("The video fill source is missing from this project.")?;
                resolved.as_slice()
            };
            anyhow::ensure!(
                !bytes.is_empty() && bytes.len() <= 100 * 1024 * 1024,
                "The video fill must be nonempty and no larger than 100 MiB."
            );
            Ok(Arc::<[u8]>::from(bytes))
        });
        let task = cx.spawn(async move |this, cx| {
            let result = loading.await;
            if let Err(error) = this.update(cx, |this, cx| {
                this.finish_canvas_video_fill_load(source, generation, result, cx);
            }) {
                log::debug!("Canvas video fill owner was released: {error}");
            }
        });
        self.canvas_video_fills.insert(
            source.asset,
            CanvasVideoFillSession {
                source,
                generation,
                loading: Some(task),
                playback: None,
                observation: None,
                error: None,
            },
        );
        cx.notify();
    }

    fn finish_canvas_video_fill_load(
        &mut self,
        source: CanvasVideoFillSource,
        generation: u64,
        result: Result<Arc<[u8]>>,
        cx: &mut Context<Self>,
    ) {
        if self.canvas_video_removed.get()
            || !self
                .selected_canvas_video_fill_sources(cx)
                .contains(&source)
            || !self
                .canvas_video_fills
                .get(&source.asset)
                .is_some_and(|session| session.source == source && session.generation == generation)
        {
            return;
        }
        let bytes = match result {
            Ok(bytes) => bytes,
            Err(error) => {
                if let Some(session) = self.canvas_video_fills.get_mut(&source.asset) {
                    session.loading = None;
                    session.error = Some(format!("Could not load video fill: {error:#}").into());
                }
                cx.notify();
                return;
            }
        };
        let playback = cx.new(|cx| VideoPlaybackView::new(bytes, 2048, cx));
        playback.update(cx, |playback, cx| {
            playback.set_audio(true, 0.0, cx);
            playback.set_active(self.canvas_video_active.get(), cx);
        });
        let observation = cx.observe(&playback, |_, _, cx| cx.notify());
        if let Some(session) = self.canvas_video_fills.get_mut(&source.asset) {
            session.loading = None;
            session.playback = Some(playback);
            session.observation = Some(observation);
            cx.notify();
        }
    }

    fn sync_canvas_video_fills(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.canvas_video_removed.get()
            || self.prototype_player.is_some()
            || self.editor_workspace(cx) != EditorWorkspace::Canvas
            || !matches!(
                self.editor_mode(cx),
                EditorMode::Motion | EditorMode::Prototype
            )
        {
            self.clear_canvas_video_fills(cx);
            return;
        }
        let sources = self.selected_canvas_video_fill_sources(cx);
        let desired = sources
            .iter()
            .map(|source| (source.asset, *source))
            .collect::<HashMap<_, _>>();
        let stale = self
            .canvas_video_fills
            .iter()
            .filter_map(|(&asset, session)| {
                (desired.get(&asset) != Some(&session.source)).then_some(asset)
            })
            .collect::<Vec<_>>();
        for asset in stale {
            self.remove_canvas_video_fill(asset, cx);
        }
        self.canvas_video_active.set(window.is_window_active());
        for source in sources {
            if !self.canvas_video_fills.contains_key(&source.asset) {
                self.start_canvas_video_fill(source, cx);
            }
        }
        for session in self.canvas_video_fills.values() {
            if let Some(playback) = &session.playback {
                playback.update(cx, |playback, cx| {
                    playback.set_active(window.is_window_active(), cx);
                    playback.tick(window, cx);
                });
            }
        }
    }

    pub(crate) fn ensure_video_fill_playback(&mut self, asset: AssetId, cx: &mut Context<Self>) {
        if !matches!(
            self.editor_mode(cx),
            EditorMode::Motion | EditorMode::Prototype
        ) {
            return;
        }
        if self.canvas_video_fills.contains_key(&asset) {
            return;
        }
        if let Some(source) = self
            .selected_canvas_video_fill_sources(cx)
            .into_iter()
            .find(|source| source.asset == asset)
        {
            self.start_canvas_video_fill(source, cx);
        }
    }

    pub(crate) fn video_fill_playback(&self, asset: AssetId) -> Option<Entity<VideoPlaybackView>> {
        self.canvas_video_fills.get(&asset)?.playback.clone()
    }

    pub(crate) fn video_fill_error(&self, asset: AssetId) -> Option<SharedString> {
        self.canvas_video_fills.get(&asset)?.error.clone()
    }

    pub(crate) fn video_fill_loading(&self, asset: AssetId) -> bool {
        self.canvas_video_fills.get(&asset).is_some_and(|session| {
            session.error.is_none() && (session.loading.is_some() || session.playback.is_none())
        })
    }

    pub(crate) fn canvas_video_fill_frames(&self, cx: &App) -> Vec<CanvasVideoFillFrame> {
        if !self.canvas_video_active.get() || self.canvas_video_removed.get() {
            return Vec::new();
        }
        let selected = self
            .selected_canvas_video_fill_sources(cx)
            .into_iter()
            .map(|source| source.asset)
            .collect::<HashSet<_>>();
        let mut frames = self
            .canvas_video_fills
            .iter()
            .filter_map(|(&asset, session)| {
                if !selected.contains(&asset) {
                    return None;
                }
                let playback = session.playback.as_ref()?.read(cx);
                Some(CanvasVideoFillFrame {
                    asset,
                    buffer: playback.frame()?,
                    session_revision: session.generation,
                    revision: playback.frame_revision(),
                })
            })
            .collect::<Vec<_>>();
        frames.sort_unstable_by_key(|frame| frame.asset);
        frames
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
        for session in self.canvas_video_fills.values() {
            if let Some(playback) = &session.playback {
                playback.update(cx, |playback, cx| playback.set_active(active, cx));
            }
        }
    }

    fn sync_canvas_video(&mut self, window_active: bool, cx: &mut Context<Self>) {
        if self.canvas_video_removed.get()
            || self.prototype_player.is_some()
            || self.editor_workspace(cx) != EditorWorkspace::Canvas
            || !matches!(
                self.editor_mode(cx),
                EditorMode::Motion | EditorMode::Prototype
            )
        {
            self.clear_canvas_video(cx);
            self.clear_canvas_video_fills(cx);
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
        if !matches!(
            self.editor_mode(cx),
            EditorMode::Motion | EditorMode::Prototype
        ) {
            return None;
        }
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
        let _perf_span = crate::gesture_perf::ui_span(
            self.gesture_perf.as_ref(),
            cx.entity_id().as_u64(),
            crate::gesture_perf::UiStage::ViewRender,
        );
        self.inspector_has_focus = self.inspector_focus_handle.contains_focused(window, cx);
        // Deferred open from the text tool's commit, which arrives through a
        // window-level mouse listener without a `Window`. Everything is
        // selected so the first keystroke replaces the placeholder.
        if let Some(node) = self.pending_text_edit.take() {
            self.open_text_edit(node, TextEditSeed::SelectAll, window, cx);
        }
        let development_read_only = self.is_dev_mode(cx);
        let editor_workspace = self.editor_workspace(cx);
        self.code_workspace.update(cx, |workspace, cx| {
            workspace.set_development_read_only(development_read_only, cx);
            workspace.set_visible(editor_workspace == EditorWorkspace::Code, window, cx);
        });
        if editor_workspace == EditorWorkspace::Canvas {
            self.maybe_prewarm_fonts(cx);
        }
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
        #[cfg(target_os = "macos")]
        self.sync_canvas_video(window.is_window_active(), cx);
        #[cfg(target_os = "macos")]
        self.sync_canvas_audio(window.is_window_active(), cx);
        #[cfg(target_os = "macos")]
        self.sync_canvas_video_fills(window, cx);
        #[cfg(feature = "fanta-gpui-ui")]
        if editor_workspace == EditorWorkspace::Canvas {
            // Hidden canvas inspectors must not project document data on table scrolls.
            let tool = self.tools.kind();
            let zoom_percent = self.current_zoom_percent(cx);
            let options = self.toolbar_option_inputs(window, cx);
            if let Some(adapter) = self.gpui_toolbar.as_mut() {
                adapter.refresh(editor_mode, tool, zoom_percent, options, cx);
            }
            // Covers state that was already ready before the first item
            // event (a preloaded document); memoized, so later frames skip.
            self.refresh_gpui_design(cx);
            self.refresh_properties_inspector(window, cx);
            if editor_mode == EditorMode::Motion {
                self.refresh_shared_timeline(window, cx);
            }
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
            // session. Only Escape is bound in the macOS text-edit context,
            // before Workspace actions. Other keys fall through
            // to the session listener or platform input handler.
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
            .on_action(cx.listener(Self::toggle_template_fill))
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
            .on_action(cx.listener(|this, _: &ActivateSelectTool, window, cx| {
                this.activate_tool_from_action(ToolKind::Select, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ActivateInspectTool, window, cx| {
                this.activate_tool_from_action(ToolKind::Inspect, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ActivateMeasureTool, window, cx| {
                this.activate_tool_from_action(ToolKind::Measure, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ActivateAnnotationTool, window, cx| {
                this.activate_tool_from_action(ToolKind::Annotation, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ActivateHandTool, window, cx| {
                this.activate_tool_from_action(ToolKind::Hand, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ActivateRectangleTool, window, cx| {
                this.activate_tool_from_action(ToolKind::Rect, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ActivateEllipseTool, window, cx| {
                this.activate_tool_from_action(ToolKind::Ellipse, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ActivateLineTool, window, cx| {
                this.activate_tool_from_action(ToolKind::Line, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ActivateArrowTool, window, cx| {
                this.activate_tool_from_action(ToolKind::Arrow, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ActivatePolygonTool, window, cx| {
                this.activate_tool_from_action(ToolKind::Polygon, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ActivateStarTool, window, cx| {
                this.activate_tool_from_action(ToolKind::Star, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ActivatePenTool, window, cx| {
                this.activate_tool_from_action(ToolKind::Pen, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ActivateNodeEditTool, window, cx| {
                this.activate_tool_from_action(ToolKind::NodeEdit, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ReversePath, window, cx| {
                this.run_path_action(
                    crate::layer_context_ops::PathAction::Reverse,
                    None,
                    window,
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &BreakPathAtAnchor, _, cx| {
                if this.is_editable(cx)
                    && matches!(this.tools.kind(), ToolKind::NodeEdit | ToolKind::PathSelect)
                {
                    this.dispatch_tool_event(
                        fanta_tools::ToolEvent::Key(fanta_tools::KeyEvent::with_modifiers(
                            LogicalKey::Enter,
                            fanta_tools::ModifierKeys::SHIFT,
                        )),
                        cx,
                    );
                }
            }))
            .on_action(cx.listener(|this, _: &ActivateFrameTool, window, cx| {
                this.activate_tool_from_action(ToolKind::Frame, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ActivateTextTool, window, cx| {
                this.activate_tool_from_action(ToolKind::Text, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ActivatePencilTool, window, cx| {
                this.activate_tool_from_action(ToolKind::Pencil, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ActivateBrushTool, window, cx| {
                this.activate_tool_from_action(ToolKind::Brush, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ActivateEraserTool, window, cx| {
                this.activate_tool_from_action(ToolKind::Eraser, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ActivateSectionTool, window, cx| {
                this.activate_tool_from_action(ToolKind::Section, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ActivateSliceTool, window, cx| {
                this.activate_tool_from_action(ToolKind::Slice, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ActivateScaleTool, window, cx| {
                this.activate_tool_from_action(ToolKind::Scale, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ActivatePathSelectTool, window, cx| {
                this.activate_tool_from_action(ToolKind::PathSelect, window, cx);
            }))
            .on_action(
                cx.listener(|this, _: &ActivateRectangleSelectTool, window, cx| {
                    this.activate_tool_from_action(ToolKind::RectangleSelect, window, cx);
                }),
            )
            .on_action(
                cx.listener(|this, _: &ActivateEllipseSelectTool, window, cx| {
                    this.activate_tool_from_action(ToolKind::EllipseSelect, window, cx);
                }),
            )
            .on_action(cx.listener(|this, _: &ActivateLassoTool, window, cx| {
                this.activate_tool_from_action(ToolKind::Lasso, window, cx);
            }))
            .on_action(
                cx.listener(|this, _: &ActivatePolygonalLassoTool, window, cx| {
                    this.activate_tool_from_action(ToolKind::PolygonalLasso, window, cx);
                }),
            )
            .on_action(cx.listener(|this, _: &ActivateMagicWandTool, window, cx| {
                this.activate_tool_from_action(ToolKind::MagicWand, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ActivateCropTool, window, cx| {
                this.activate_tool_from_action(ToolKind::Crop, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ExportMotionJson, _, cx| {
                this.export_selected_motion(crate::export::ExportFormat::MotionJson, cx);
            }))
            .on_action(cx.listener(|this, _: &ExportFigmaMotionJson, _, cx| {
                this.export_selected_motion(crate::export::ExportFormat::FigmaMotionJson, cx);
            }))
            .on_action(cx.listener(|this, _: &ExportMotionStage, _, cx| {
                this.export_selected_motion(crate::export::ExportFormat::MotionStage, cx);
            }))
            .on_action(cx.listener(|this, _: &ExportMotionVideo, _, cx| {
                this.export_selected_motion(crate::export::ExportFormat::MotionVideo, cx);
            }))
            .on_action(cx.listener(|this, _: &ExportMotionGif, _, cx| {
                this.export_selected_motion(crate::export::ExportFormat::MotionGif, cx);
            }))
            .on_action(cx.listener(|this, _: &ExportMotionFrames, _, cx| {
                this.export_selected_motion(crate::export::ExportFormat::MotionFrames, cx);
            }))
            .on_action(cx.listener(|this, _: &ApplySuggestedFix, _, cx| {
                this.apply_open_comment_suggested_fix(cx);
            }))
            .on_action(cx.listener(|this, _: &ActivateCommentTool, window, cx| {
                this.activate_tool_from_action(ToolKind::Comment, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ActivateTextPathTool, window, cx| {
                this.activate_tool_from_action(ToolKind::TextPath, window, cx);
            }))
            .on_action(cx.listener(|view, _: &EnterDevMode, window, cx| {
                view.set_editor_mode_from_action(EditorMode::Dev, window, cx)
            }))
            .on_action(cx.listener(|view, _: &OpenSavedCode, _, cx| {
                view.set_editor_workspace(EditorWorkspace::Code, cx)
            }))
            .on_action(cx.listener(|view, _: &PlaceLocalMedia, _, cx| view.choose_local_media(cx)))
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
                        .child(Label::new("Could not open design").size(LabelSize::Large))
                        // `{:#}` prints the whole context chain, so the real
                        // loader failure (auth, parse, missing file) is shown
                        // instead of just the outermost wrapper.
                        .child(Label::new(format!("{error:#}")).color(Color::Muted)),
                )
            })
            .when_some(snapshot.loading_message, |this, message| {
                this.child(crate::loading_indicator(
                    message,
                    "Your design will appear when this action finishes.",
                ))
            })
            .when(!has_error && !is_loading, |this| {
                let presenting_prototype = self.prototype_player.is_some();
                let workspace_body =
                    if presenting_prototype {
                        self.render_prototype_presentation(window, cx)
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
                        .child(self.pane_width_probe(cx))
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
                                            self.pane_sidebar_layout().layers
                                                .map(|_| self.render_layers_sidebar(cx)),
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
                                                        .on_mouse_down(
                                                            MouseButton::Right,
                                                            cx.listener(
                                                                Self::deploy_canvas_context_menu,
                                                            ),
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
                                                            if let Some(comments) = self
                                                                .render_comment_overlay(
                                                                    window, cx,
                                                                )
                                                            {
                                                                c.push(comments);
                                                            }
                                                            if let Some(activity) =
                                                                self.render_agent_presence(cx)
                                                            {
                                                                c.push(activity);
                                                            }
                                                            if let Some(menu) =
                                                                self.render_canvas_context_menu()
                                                            {
                                                                c.push(menu);
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
                                                })
                                                .children({
                                                    #[cfg(target_os = "macos")]
                                                    {
                                                        self.render_canvas_audio_controls(cx)
                                                    }
                                                    #[cfg(not(target_os = "macos"))]
                                                    {
                                                        None::<AnyElement>
                                                    }
                                                }),
                                        )
                                        .children(
                                            self.pane_sidebar_layout().inspector
                                                .map(|_| self.render_inspector_sidebar(window, cx)),
                                        ),
                                )
                                .children((editor_mode == EditorMode::Motion).then(|| {
                                    #[cfg(feature = "fanta-gpui-ui")]
                                    if let Some(adapter) = &self.gpui_timeline {
                                        return adapter.panel.clone().into_any_element();
                                    }
                                    self.timeline_shell.clone().into_any_element()
                                })),
                        )
                        .children({
                            #[cfg(feature = "fanta-gpui-ui")]
                            {
                                self.gpui_properties
                                    .as_ref()
                                    .filter(|_| !self.inspector_sidebar_expanded(cx))
                                    .map(|_| {
                                        gpui::deferred(
                                            div()
                                                .absolute()
                                                .top_0()
                                                .right_0()
                                                .p_2()
                                                .child(self.render_inspector_toolbar(true, cx)),
                                        )
                                        .with_priority(3)
                                    })
                            }
                            #[cfg(not(feature = "fanta-gpui-ui"))]
                            {
                                None::<AnyElement>
                            }
                        })
                        .children(self.pane_sidebar_layout().layers.is_none().then(|| {
                            gpui::deferred(
                                div().absolute().top_0().bottom_0().left_0()
                                    .when(self.layers_sidebar_expanded(), |sidebar| {
                                        sidebar.w(self.displayed_sidebar_width(SidebarKind::Layers))
                                            .bg(cx.theme().colors().panel_background).occlude()
                                    })
                                    .child(self.render_layers_sidebar(cx)),
                            ).with_priority(3)
                        }))
                        .children((self.pane_sidebar_layout().inspector.is_none()
                            && self.inspector_sidebar_expanded(cx)).then(|| {
                            gpui::deferred(
                                div().id("fanta-inspector-overlay")
                                    .debug_selector(|| "fanta-inspector-overlay".to_owned())
                                    .absolute().top_0().bottom_0().right_0()
                                    .w(self.pane_width.unwrap_or(self.inspector_sidebar_width)
                                        .min(self.displayed_sidebar_width(SidebarKind::Inspector)))
                                    .overflow_x_scroll()
                                    .bg(cx.theme().colors().panel_background).occlude()
                                    .child(self.render_inspector_sidebar(window, cx)),
                            ).with_priority(4)
                        }))
                        .children(
                            self.item
                                .read(cx)
                                .source_edit_locked()
                                .then(|| self.render_source_edit_lock_banner(cx)),
                        )
                        .into_any_element()
                    };
                let workspace_body = div()
                    .size_full()
                    .relative()
                    .child(workspace_body)
                    .children(
                        (editor_workspace != EditorWorkspace::Canvas && !presenting_prototype)
                            .then(|| self.render_agent_presence(cx))
                            .flatten(),
                    )
                    .children(self.item.read(cx).source_reload_error().map(|message| {
                        div()
                            .id("fanta-source-reload-error")
                            .absolute()
                            .top(px(96.0))
                            .left_4()
                            .right_4()
                            .p_3()
                            .rounded_md()
                            .bg(cx.theme().colors().elevated_surface_background)
                            .border_1()
                            .border_color(cx.theme().status().error_border)
                            .text_sm()
                            .child(message.to_string())
                    }));
                this.child(workspace_body)
                    .children((!presenting_prototype).then(|| self.render_workspace_tabs(cx)))
            })
    }
}

fn fitted_presentation_device_size(
    presentation: Option<&fanta_doc::PresentationConfig>,
    viewport_size: (f32, f32),
) -> Option<(f32, f32)> {
    let presentation = presentation?;
    let [mut width, mut height] = presentation.device_size?;
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return None;
    }
    if presentation.landscape && height > width {
        std::mem::swap(&mut width, &mut height);
    }
    let available_width = (f64::from(viewport_size.0) - 160.0).max(1.0);
    let available_height = (f64::from(viewport_size.1) - 160.0).max(1.0);
    let scale = (available_width / width)
        .min(available_height / height)
        .min(1.0);
    Some(((width * scale) as f32, (height * scale) as f32))
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
fn paste_prepared_images(
    doc: &mut Doc,
    assets: &mut AssetStores<'_>,
    images: Vec<Result<PreparedImage>>,
    center: [f64; 2],
    visible: [f64; 2],
) -> Result<PastedImages> {
    let mut ingested = Vec::new();
    let mut skipped = Vec::new();
    for image in images {
        match image.and_then(|image| assets.add_prepared_image_tracked(image)) {
            Ok(image) => ingested.push(image),
            Err(error) => skipped.push(error),
        }
    }
    match place_ingested_images(doc, &ingested, center, visible) {
        Ok(placed) => Ok(PastedImages { placed, skipped }),
        Err(error) => {
            for (asset, _, inserted) in ingested {
                if inserted {
                    assets.remove(asset);
                }
            }
            Err(error)
        }
    }
}

fn place_ingested_images(
    doc: &mut Doc,
    ingested: &[(AssetId, [u32; 2], bool)],
    center: [f64; 2],
    visible: [f64; 2],
) -> Result<Vec<NodeId>> {
    let mut nodes: Vec<CanvasNode> = Vec::with_capacity(ingested.len());
    for (position, (asset, natural_size, _)) in ingested.iter().copied().enumerate() {
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

#[cfg(test)]
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
        workspace: &mut workspace::Workspace,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.save_failure_notice.borrow().is_some() {
            sync_save_failure_notice_deferred(
                self.item.entity_id(),
                self.save_failure_notice.clone(),
                cx,
            );
        }
        if let Some(workspace_entity) = workspace.weak_handle().upgrade() {
            self.workspace_activation_subscription = Some(cx.subscribe(
                &workspace_entity,
                |view, workspace, event, cx| {
                    if matches!(event, workspace::Event::ActiveItemChanged)
                        && workspace
                            .read(cx)
                            .active_item(cx)
                            .is_some_and(|item| item.item_id() == cx.entity_id())
                    {
                        view.activate_design_scope(cx);
                    }
                },
            ));
        }
        if workspace
            .active_item(cx)
            .is_some_and(|item| item.item_id() == cx.entity_id())
        {
            self.activate_design_scope(cx);
        }
        #[cfg(target_os = "macos")]
        if self.canvas_video_removed.replace(false) {
            self.clear_canvas_video(cx);
            self.clear_canvas_video_fills(cx);
        }
    }

    fn deactivated(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.invalidate_local_media_origin();
        self.freeze_annotation_move(cx);
        self.freeze_measurement_drag(cx);
        self.finish_document_edits(cx);
        #[cfg(target_os = "macos")]
        self.set_canvas_video_active(false, cx);
        #[cfg(target_os = "macos")]
        if let Some(session) = self.canvas_audio.as_ref() {
            session
                .playback
                .update(cx, |playback, cx| playback.pause(cx));
        }
    }

    fn workspace_deactivated(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        // The native media picker deactivates its own workspace window. Page,
        // tab, and document changes still invalidate the pending destination.
        if self.media_import_task.is_none() {
            self.invalidate_local_media_origin();
        }
        self.freeze_annotation_move(cx);
        self.freeze_measurement_drag(cx);
        self.finish_document_edits(cx);
        #[cfg(target_os = "macos")]
        self.set_canvas_video_active(false, cx);
        #[cfg(target_os = "macos")]
        if let Some(session) = self.canvas_audio.as_ref() {
            session
                .playback
                .update(cx, |playback, cx| playback.pause(cx));
        }
    }

    fn on_removed(&self, _cx: &mut Context<Self>) {
        if self.save_failure_notice.borrow().is_some() {
            sync_save_failure_notice_deferred(
                self.item.entity_id(),
                self.save_failure_notice.clone(),
                _cx,
            );
        }
        self.invalidate_local_media_origin();
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
            for session in self.canvas_video_fills.values() {
                if let Some(playback) = &session.playback {
                    playback.update(_cx, |playback, cx| playback.close(cx));
                }
            }
            if let Some(session) = &self.canvas_audio {
                session
                    .playback
                    .update(_cx, |playback, cx| playback.close(cx));
            }
        }
    }

    fn to_item_events(event: &Self::Event, f: &mut dyn FnMut(ItemEvent)) {
        match event {
            FigViewEvent::Edited => {
                f(ItemEvent::Edit);
                f(ItemEvent::UpdateTab);
            }
            FigViewEvent::TitleChanged | FigViewEvent::PropertyDraftChanged => {
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
        let opened_entry_id = self
            .project_path_for_opened_entry(cx)
            .and_then(|path| self.project.read(cx).entry_for_path(&path, cx))
            .map(|entry| entry.id)
            .or(self.opened_entry_id);
        match opened_entry_id {
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
        let item = self.item.read(cx);
        let name = item.document().and_then(|document| match self.scope {
            Some(FigScope::Page(root)) => {
                document.doc.scene.get(root).map(|node| node.name.clone())
            }
            Some(FigScope::Component(component)) => document
                .doc
                .components
                .defs
                .get(&component)
                .map(|definition| definition.name.clone()),
            Some(FigScope::Variables) => Some("Variables".into()),
            None => None,
        });
        name.filter(|name| !name.is_empty())
            .map(SharedString::from)
            .unwrap_or_else(|| item.title())
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
        let project = self.project.read(cx);
        let source_path = self
            .project_path_for_opened_entry(cx)
            .and_then(|path| project.absolute_path(&path, cx));
        let path = source_path.as_deref().unwrap_or_else(|| item.abs_path());
        ItemSettings::get_global(cx)
            .file_icons
            .then(|| FileIcons::get_icon(path, cx))
            .flatten()
            .map(Icon::from_path)
    }

    fn tab_color(&self, cx: &App) -> Option<gpui::Hsla> {
        self.agent_tab_color(cx)
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

    fn close_blocker(&self, _cx: &App) -> Option<SharedString> {
        if self.annotation_state.controller.has_pending_authoring() {
            return Some("Add, save or cancel the annotation before closing this canvas. Its draft was kept.".into());
        }
        self.measurement_controller
            .has_pending_authoring()
            .then(|| {
                "Finish or cancel the measurement before closing this canvas. Its draft was kept."
                    .into()
            })
    }

    fn has_pending_view_edits(&self, _cx: &App) -> bool {
        #[cfg(feature = "fanta-gpui-ui")]
        if let Some(adapter) = self.gpui_design.as_ref() {
            return matches!(
                adapter.panel.read(_cx).property_draft_state(_cx),
                fanta_gpui::design::DesignPropertyDraftState::Valid
                    | fanta_gpui::design::DesignPropertyDraftState::Invalid
            );
        }
        false
    }

    fn prepare_for_save_or_close(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        #[cfg(feature = "fanta-gpui-ui")]
        if let Some(adapter) = self.gpui_design.as_ref() {
            let unchanged = adapter.panel.update(_cx, |panel, cx| {
                if panel.property_draft_state(cx)
                    == fanta_gpui::design::DesignPropertyDraftState::Unchanged
                {
                    match panel.try_commit_property_draft(_window, cx) {
                        Ok(_) => true,
                        Err(error) => {
                            log::error!("unchanged property could not finish: {error}");
                            panel.preserve_property_draft_for_host_action();
                            false
                        }
                    }
                } else {
                    panel.preserve_property_draft_for_host_action();
                    false
                }
            });
            if unchanged {
                self.finish_gpui_design_edits(_cx);
            }
        }
    }

    fn is_dirty(&self, cx: &App) -> bool {
        self.has_pending_view_edits(cx)
            || self.item.read(cx).is_dirty()
            || self.code_workspace.read(cx).source_is_dirty(cx)
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        self.after_property_draft_commit(window, cx, move |view, _window, cx| {
            if let Some(error) = view.pending_bound_text_error(cx) {
                return Task::ready(Err(anyhow::anyhow!(error)));
            }
            if view.annotation_state.controller.has_pending_authoring() {
                return Task::ready(Err(anyhow::anyhow!(
                    "Add, save or cancel the annotation first. Its draft was kept."
                )));
            }
            if view.measurement_controller.has_pending_authoring() {
                return Task::ready(Err(anyhow::anyhow!(
                    "Finish or cancel the measurement drag first. Its preview was kept."
                )));
            }
            if view.prototype_player.is_some() {
                view.exit_prototype_session(cx);
            }
            view.finish_document_edits(cx);
            view.autosave_task = None;
            view.item
                .update(cx, |item, cx| item.save_as(project, path, cx))
        })
    }

    fn save(
        &mut self,
        options: SaveOptions,
        _project: Entity<Project>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        if options.autosave && self.has_pending_view_edits(cx) {
            return Task::ready(Ok(()));
        }
        self.after_property_draft_commit(window, cx, move |view, _window, cx| {
            if let Some(error) = view.pending_bound_text_error(cx) {
                return Task::ready(Err(anyhow::anyhow!(error)));
            }
            if view.annotation_state.controller.has_pending_authoring() {
                return Task::ready(Err(anyhow::anyhow!(
                    "Add, save or cancel the annotation first. Its draft was kept."
                )));
            }
            if view.measurement_controller.has_pending_authoring() {
                return Task::ready(Err(anyhow::anyhow!(
                    "Finish or cancel the measurement drag first. Its preview was kept."
                )));
            }

            if view.prototype_player.is_some() {
                view.exit_prototype_session(cx);
            }
            let source_is_dirty = view.code_workspace.read(cx).source_is_dirty(cx);
            let canvas_is_dirty = view.item.read(cx).is_dirty();
            if source_is_dirty && canvas_is_dirty {
                if view.editor_workspace(cx) == EditorWorkspace::Code {
                    let discard_canvas = view.item.update(cx, |item, cx| {
                        item.discard_canvas_edits_for_source_resolution(cx)
                    });
                    let code_workspace = view.code_workspace.clone();
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

                view.finish_document_edits(cx);
                let discard_source = view
                    .code_workspace
                    .update(cx, |workspace, cx| workspace.discard_source_edit(cx));
                let item = view.item.clone();
                let project = view.project.clone();
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
            if let Some(source_save) = view
                .code_workspace
                .update(cx, |workspace, cx| workspace.save_source_edit(cx))
            {
                return source_save;
            }
            let task = view.save_document(cx);
            let project = view.project.clone();
            cx.spawn(async move |_, cx| {
                let Some(root) = task.await? else {
                    return Ok(());
                };
                log::info!("materialized Fanta project at {}", root.display());
                // First-class Fanta integration: surface the freshly written project
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
        })
    }

    fn reload(
        &mut self,
        _project: Entity<Project>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        if self.annotation_state.controller.has_pending_authoring() {
            return Task::ready(Err(anyhow::anyhow!(
                "Add, save or cancel the annotation first. Its draft was kept."
            )));
        }
        if self.measurement_controller.has_pending_authoring() {
            return Task::ready(Err(anyhow::anyhow!(
                "Finish or cancel the measurement drag first. Its preview was kept."
            )));
        }

        if self.prototype_player.is_some() {
            self.exit_prototype_session(cx);
        }
        #[cfg(feature = "fanta-gpui-ui")]
        if let Some(adapter) = self.gpui_properties.as_ref() {
            adapter
                .design
                .update(cx, |design, cx| design.deactivate(_window, cx));
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
        let save_failure_notice = self.save_failure_notice.clone();
        let project = self.project.clone();
        let viewport = self.viewport;
        let selected_page_index = self.selected_page_index;
        let selected_page_root = self.selected_page_root;
        let opened_entry_id = self.opened_entry_id;
        let opened_project_path = self.project_path_for_opened_entry(cx);
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
            let view = cx.entity_id();
            cx.on_release(move |_, cx| crate::agent_surface::clear_active_view(view, cx))
                .detach();
            cx.on_release(Self::release_painted_canvas_image).detach();
            let item_subscription = Self::subscribe_to_item(&item, cx);
            let editor_session =
                cx.new(|_| EditorSession::with_state(editor_mode, editor_workspace));
            let editor_session_subscription = cx.observe(&editor_session, |_, _, cx| cx.notify());
            let (layers_sidebar, inspector_sidebar) =
                Self::new_embedded_sidebars(&project, window, cx);
            let file_inspector_subscription = cx.subscribe(
                &layers_sidebar,
                |this, _, event: &FileInspectorVisibilityChanged, cx| {
                    this.set_layers_sidebar_visible(event.0, cx);
                    cx.notify();
                },
            );
            let prototype_sidebar = cx.new(|cx| FantaPrototypePanel::new(item.clone(), cx));
            let comments_sidebar = {
                let view = cx.weak_entity();
                cx.new(|cx| CommentsPanel::new(item.clone(), view, cx))
            };
            let timeline_shell = cx.new(|cx| {
                let mut timeline = TimelineShell::new();
                timeline.set_authoring_enabled(timeline_authoring_enabled, cx);
                timeline.set_model(timeline_model, cx);
                timeline
            });
            let motion_sidebar = cx
                .new(|cx| FantaMotionPanel::new(item.clone(), timeline_shell.clone(), window, cx));
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
            let timeline_subscription =
                cx.subscribe(&timeline_shell, |this, _, event: &TimelineEvent, cx| {
                    this.handle_timeline_event(event.clone(), cx);
                });
            let mut tools = ToolShell::new();
            if editor_mode == EditorMode::Dev {
                tools.activate_without_context(ToolKind::Inspect);
            }
            Self {
                item,
                project,
                focus_handle: cx.focus_handle(),
                native_toolbar_focus: cx.focus_handle(),
                editor_session,
                layers_sidebar,
                _file_inspector_subscription: file_inspector_subscription,
                inspector_sidebar,
                prototype_sidebar,
                comments_sidebar,
                motion_sidebar,
                variables_workspace,
                code_workspace,
                timeline_shell,
                active_motion_clip,
                motion_auto_keyframe: false,
                motion_keyframe_drag: None,
                layers_sidebar_visible,
                inspector_sidebar_visible,
                layers_sidebar_width,
                inspector_sidebar_width,
                pane_width: None,
                revealed_compact_sidebar: None,
                inspector_has_focus: false,
                inspector_focus_handle: cx.focus_handle(),
                selected_page_index,
                selected_page_root,
                opened_entry_id,
                opened_project_path,
                scope,
                is_focused: false,
                agent_navigation_focus: None,
                last_seen_root,
                viewport,
                pan_last_position: None,
                primary_pressed: false,
                click_diagnostics: ClickDiagnostics::from_setting(
                    std::env::var_os("FANTA_INPUT_DIAGNOSTICS").as_deref(),
                ),
                gesture_perf: crate::perf_enabled().then(crate::gesture_perf::GesturePerf::shared),
                canvas_pointer_down: false,
                autosave_task: None,
                reported_save_failure: None,
                save_failure_notice,
                hover_resize_handle: None,
                space_pan: false,
                container_bounds: None,
                rendered_canvas: None,
                painted_canvas_image: None,
                chrome_cache: std::cell::RefCell::new(None),
                painted_raw_text_node: None,
                #[cfg(target_os = "macos")]
                gpu_canvas: None,
                #[cfg(target_os = "macos")]
                canvas_video: None,
                #[cfg(target_os = "macos")]
                canvas_audio: None,
                #[cfg(target_os = "macos")]
                canvas_video_fills: HashMap::new(),
                #[cfg(target_os = "macos")]
                canvas_video_generation: 0,
                #[cfg(target_os = "macos")]
                canvas_video_fill_generation: 0,
                #[cfg(target_os = "macos")]
                canvas_video_removed: std::cell::Cell::new(false),
                #[cfg(target_os = "macos")]
                canvas_video_active: std::cell::Cell::new(true),
                tools,
                annotation_state: annotations_host::AnnotationHostState::default(),
                measurement_controller: MeasurementController::default(),
                measurement_origin: None,
                measurement_selection: None,
                measurement_cache: std::cell::RefCell::new(None),
                media_import_generation: std::cell::Cell::new(0),
                media_import_task: None,
                wand_asset_map: None,
                wand_decode_cache: HashMap::new(),
                wand_decode_order: VecDeque::new(),
                wand_generation: 0,
                wand_override: None,
                comment_state: crate::comments_ui::CommentState::default(),
                group_faces: crate::tools::initial_group_faces(),
                template_fill: false,
                #[cfg(feature = "fanta-gpui-ui")]
                gpui_toolbar: crate::gpui_adapters::runtime_enabled(cx)
                    .then(|| crate::gpui_adapters::toolbar::ToolbarAdapter::new(window, cx)),
                #[cfg(feature = "fanta-gpui-ui")]
                gpui_design: (crate::gpui_adapters::runtime_enabled(cx)
                    && crate::gpui_adapters::design::design_enabled())
                .then(|| crate::gpui_adapters::design::DesignAdapter::new(window, cx)),
                #[cfg(feature = "fanta-gpui-ui")]
                gpui_properties: None,
                #[cfg(feature = "fanta-gpui-ui")]
                gpui_timeline: None,
                fonts_prewarmed: false,
                hovered_node: None,
                text_edit: None,
                pending_text_edit: None,
                prototype_player: None,
                canvas_context_menu: None,
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
                _agent_activity_subscription: Self::observe_agent_activity(window, cx),
                workspace_activation_subscription: None,
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
        !matches!(self, Self::Select | Self::Inspect | Self::Hand)
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
mod tests;

impl FigView {
    /// The zoom the toolbar should display: live viewport zoom, or the fit
    /// zoom the canvas will initialize with before first interaction.
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
            auto_keyframe: self.motion_auto_keyframe,
            time_comment_armed: self.comment_state.motion_time_comment_active(),
            layers_sidebar_visible: self.layers_sidebar_expanded(),
            inspector_sidebar_visible: self.inspector_sidebar_expanded(cx),
            // Live keymap text, like the tooltip the old native button had.
            fit_to_view_shortcut: ui::text_for_action(&FitToView, window, cx).map(Into::into),
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

    fn render_toolbar_slot(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.is_dev_mode(cx) {
            return self.render_native_dev_toolbar(cx);
        }
        #[cfg(feature = "fanta-gpui-ui")]
        if self.gpui_toolbar.is_some() {
            return self.render_gpui_toolbar(cx);
        }
        self.render_tool_pill(cx)
    }

    /// Center the toolbar independently of the zoom and sidebar headers.
    #[cfg(feature = "fanta-gpui-ui")]
    fn render_gpui_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(adapter) = self.gpui_toolbar.as_ref() else {
            return self.render_tool_pill(cx);
        };
        h_flex()
            .debug_selector(|| "fanta-canvas-toolbar".to_owned())
            .absolute()
            .bottom(px(16.))
            .left_0()
            .right_0()
            .justify_center()
            .track_focus(&self.native_toolbar_focus)
            .child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .when(self.design_crop_controls_visible(cx), |toolbar| {
                        toolbar.child(
                            h_flex()
                                .w_full()
                                .min_w_0()
                                .px_2()
                                .justify_center()
                                .child(self.render_design_crop_controls(cx)),
                        )
                    })
                    .child(adapter.panel.clone()),
            )
            .into_any_element()
    }

    #[cfg(feature = "fanta-gpui-ui")]
    fn design_crop_controls_visible(&self, cx: &App) -> bool {
        self.editor_mode(cx) == EditorMode::Design
            && self.tools.kind() == ToolKind::Crop
            && self.is_editable(cx)
    }

    #[cfg(feature = "fanta-gpui-ui")]
    fn handle_design_crop_toolbar_action(
        &mut self,
        action: &fanta_gpui::toolbar::ToolbarAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.design_crop_controls_visible(cx) {
            return;
        }
        let Some(panel) = self
            .gpui_toolbar
            .as_ref()
            .map(|adapter| adapter.panel.clone())
        else {
            return;
        };
        self.focus_handle.focus(window, cx);
        self.handle_toolbar_action(&panel, action, window, cx);
    }

    #[cfg(feature = "fanta-gpui-ui")]
    fn render_design_crop_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        use fanta_gpui::toolbar::{DrawToolbarAction, ToolbarAction};
        let Some(adapter) = self.gpui_toolbar.as_ref() else {
            return div().into_any_element();
        };
        let view = cx.weak_entity();
        let ratios = adapter.draw_options.crop_ratios.clone();
        let ratio = adapter.draw_options.crop_ratio.clone();
        let can_apply = self.tools.can_apply_crop();
        let show_hint = !can_apply
            && !self
                .tools
                .overlays
                .iter()
                .any(|overlay| matches!(overlay, fanta_tools::ToolOverlay::PreviewRect { .. }));
        let ratio_menu = PopoverMenu::new("design-crop-ratio-menu")
            .trigger(
                Button::new("design-crop-ratio-button", format!("Ratio: {ratio}"))
                    .size(ButtonSize::Compact)
                    .end_icon(Icon::new(IconName::ChevronDown).size(IconSize::XSmall)),
            )
            .menu(move |window, cx| {
                let view = view.clone();
                let ratios = ratios.clone();
                Some(ContextMenu::build(window, cx, move |mut menu, _, _| {
                    for ratio in ratios {
                        let view = view.clone();
                        menu = menu.entry(ratio.clone(), None, move |window, cx| {
                            view.update(cx, |view, cx| {
                                let Some(adapter) = view.gpui_toolbar.as_ref() else {
                                    return;
                                };
                                let mut options = adapter.draw_options.clone();
                                options.crop_ratio = ratio.clone();
                                view.handle_design_crop_toolbar_action(
                                    &ToolbarAction::DrawOptionsChangeRequested { options },
                                    window,
                                    cx,
                                );
                            })
                            .log_err();
                        });
                    }
                    menu
                }))
            });
        h_flex()
            .id("design-crop-controls")
            .debug_selector(|| "design-crop-controls".to_owned())
            .min_w_0()
            .max_w_full()
            .flex_wrap()
            .justify_center()
            .occlude()
            .gap_2()
            .px_2()
            .py_1()
            .rounded_lg()
            .shadow_md()
            .bg(cx.theme().colors().panel_background)
            .when(show_hint, |row| {
                row.child(
                    div()
                        .debug_selector(|| "design-crop-hint".to_owned())
                        .flex_none()
                        .max_w_full()
                        .child(
                            Label::new("Drag on the canvas to select a crop")
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        ),
                )
            })
            .child(
                div()
                    .debug_selector(|| "design-crop-ratio".to_owned())
                    .flex_none()
                    .child(ratio_menu),
            )
            .child(
                div()
                    .debug_selector(|| "design-crop-cancel".to_owned())
                    .flex_none()
                    .child(
                        Button::new("design-crop-cancel-button", "Cancel")
                            .size(ButtonSize::Compact)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.handle_design_crop_toolbar_action(
                                    &ToolbarAction::DrawActionInvoked {
                                        action: DrawToolbarAction::CancelCrop,
                                    },
                                    window,
                                    cx,
                                );
                            })),
                    ),
            )
            .child(
                div()
                    .debug_selector(|| "design-crop-apply".to_owned())
                    .flex_none()
                    .child(
                        Button::new("design-crop-apply-button", "Apply crop")
                            .size(ButtonSize::Compact)
                            .disabled(!can_apply)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.handle_design_crop_toolbar_action(
                                    &ToolbarAction::DrawActionInvoked {
                                        action: DrawToolbarAction::ApplyCrop,
                                    },
                                    window,
                                    cx,
                                );
                            })),
                    ),
            )
            .into_any_element()
    }

    /// Resources (Figma's ⇧I panel) reveals the left pages/layers sidebar and
    /// moves focus into it. It never hides the sidebar: the dock's chrome
    /// capsule owns show/hide, and a "Resources" tile that closed the panel
    /// every other press would read as broken.
    #[cfg(feature = "fanta-gpui-ui")]
    fn reveal_layers_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.layers_sidebar_expanded() {
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
            CHROME_ADD_CANVAS_CONTEXT, CHROME_FIT_TO_VIEW, CHROME_RUN_AI_REVIEW,
            CHROME_TOGGLE_INSPECTOR_SIDEBAR, CHROME_TOGGLE_LAYERS_SIDEBAR,
        };
        match id {
            CHROME_ADD_CANVAS_CONTEXT => self.open_toolbar_agent_attachment(window, cx),
            CHROME_RUN_AI_REVIEW => {
                window.dispatch_action(Box::new(fanta_actions::agent::ReviewDesign), cx)
            }
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
        use fanta_gpui::toolbar::{
            ToolbarAction, ToolbarCommand, ToolbarMode, ToolbarSecondaryControl, ToolbarTool,
        };
        if self.is_dev_mode(cx) {
            let allowed = match action {
                ToolbarAction::ToolChangeRequested { tool, .. } => matches!(
                    tool,
                    ToolbarTool::Move
                        | ToolbarTool::Inspect
                        | ToolbarTool::Hand
                        | ToolbarTool::Measure
                        | ToolbarTool::Annotation
                        | ToolbarTool::Code
                ),
                ToolbarAction::CommandInvoked { command } => {
                    crate::gpui_adapters::toolbar::DEV_COMMANDS.contains(command)
                }
                ToolbarAction::SecondaryControlInvoked { control, .. } => matches!(
                    control,
                    ToolbarSecondaryControl::DevInspect
                        | ToolbarSecondaryControl::DevMeasure
                        | ToolbarSecondaryControl::DevAnnotate
                ),
                ToolbarAction::ControlChangeRequested { .. } => false,
                _ => true,
            };
            if !allowed {
                show_canvas_notice_deferred(
                    "This control is unavailable in Dev. Switch to Design for artwork edits."
                        .into(),
                    cx,
                );
                return;
            }
        }
        match action {
            // Resources is host chrome, not a canvas tool: Figma's ⇧I panel
            // corresponds to the left pages/layers sidebar here.
            ToolbarAction::ToolChangeRequested {
                tool: ToolbarTool::Resources,
                ..
            } => self.reveal_layers_sidebar(window, cx),
            ToolbarAction::ToolChangeRequested {
                tool: ToolbarTool::MotionSelect,
                ..
            } => self.activate_tool(ToolKind::Select, cx),
            ToolbarAction::ToolChangeRequested {
                tool: ToolbarTool::Code,
                ..
            } => self.set_editor_workspace(EditorWorkspace::Code, cx),
            ToolbarAction::ToolChangeRequested {
                tool: ToolbarTool::Variables,
                ..
            } => self.set_editor_workspace(EditorWorkspace::Variables, cx),
            ToolbarAction::ToolChangeRequested {
                tool: ToolbarTool::ImageVideo,
                ..
            } => self.choose_local_media(cx),
            ToolbarAction::ToolChangeRequested {
                tool: ToolbarTool::ColorPicker,
                ..
            } => self.open_toolbar_color_picker(window, cx),
            ToolbarAction::ToolChangeRequested {
                tool: ToolbarTool::Inspect,
                ..
            } => self.activate_tool_from_action(ToolKind::Inspect, window, cx),
            ToolbarAction::ToolChangeRequested {
                tool: ToolbarTool::AddKeyframe,
                ..
            } => self.add_toolbar_keyframe(window, cx),
            ToolbarAction::ToolChangeRequested {
                tool: ToolbarTool::PlayPreview,
                ..
            } => {
                let playing = !self.timeline_shell.read(cx).is_playing();
                self.set_toolbar_motion_playing(playing, window, cx);
            }
            ToolbarAction::ToolChangeRequested { tool, .. } => {
                match crate::gpui_adapters::toolbar::tool_kind(*tool) {
                    Some(kind) => self.activate_tool_from_action(kind, window, cx),
                    None => notify_unavailable(tool.label(), window, cx),
                }
            }
            ToolbarAction::ChromeControlInvoked { id } => {
                self.handle_toolbar_chrome_control(id, window, cx);
            }
            ToolbarAction::ModeChangeRequested { mode } => match mode {
                ToolbarMode::Design => {
                    self.set_editor_mode_from_action(EditorMode::Design, window, cx)
                }
                ToolbarMode::Motion => {
                    self.set_editor_mode_from_action(EditorMode::Motion, window, cx)
                }
                ToolbarMode::Dev => self.set_editor_mode_from_action(EditorMode::Dev, window, cx),
                ToolbarMode::Draw => self.set_editor_mode(EditorMode::Draw, cx),
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
                | ToolbarCommand::GenerateAudio
                | ToolbarCommand::GenerateVector => {
                    use crate::generation_workspace::GenerationMode;
                    let mode = match command {
                        ToolbarCommand::GenerateVideo => GenerationMode::Video,
                        ToolbarCommand::GenerateAudio => GenerationMode::Audio,
                        ToolbarCommand::GenerateVector => GenerationMode::Vector,
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
                ToolbarCommand::DeselectAll => self.deselect_canvas(cx),
                ToolbarCommand::PlaceImageVideo => self.choose_local_media(cx),
                ToolbarCommand::OpenVariables => {
                    self.set_editor_workspace(EditorWorkspace::Variables, cx)
                }
                ToolbarCommand::OpenDrawMode => self.set_editor_mode(EditorMode::Draw, cx),
                ToolbarCommand::OpenDevMode => {
                    self.set_editor_mode_from_action(EditorMode::Dev, window, cx)
                }
                ToolbarCommand::ZoomToFit => self.fit_to_view(&FitToView, window, cx),
                ToolbarCommand::ZoomToSelection => self.zoom_to_selection(cx),
                ToolbarCommand::Present => self.play_prototype(&PlayPrototype, window, cx),
                ToolbarCommand::OpenDesignMode => {
                    self.set_editor_mode_from_action(EditorMode::Design, window, cx)
                }
                ToolbarCommand::OpenMotionMode => {
                    self.set_editor_mode_from_action(EditorMode::Motion, window, cx)
                }
                ToolbarCommand::Export => self.export_from_toolbar(window, cx),
                ToolbarCommand::Group => self.group_selection(&GroupSelection, window, cx),
                ToolbarCommand::Ungroup => self.ungroup_selection(&UngroupSelection, window, cx),
                ToolbarCommand::FrameSelection => self.frame_selection(&FrameSelection, window, cx),
                ToolbarCommand::AddAutoLayout => self.apply_toolbar_layer_command(
                    fanta_gpui::layers::LayersPanelContextAction::AddAutoLayout,
                    window,
                    cx,
                ),
                ToolbarCommand::CreateComponent => self.apply_toolbar_layer_command(
                    fanta_gpui::layers::LayersPanelContextAction::CreateComponent,
                    window,
                    cx,
                ),
                ToolbarCommand::DetachInstance => self.detach_toolbar_instance(window, cx),
                ToolbarCommand::MakePrototype => self.set_editor_mode(EditorMode::Prototype, cx),
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
            ToolbarAction::DrawOptionsChangeRequested { options } => {
                let options = options.clone().normalized();
                if let Some(adapter) = self.gpui_toolbar.as_mut() {
                    adapter.draw_options = options.clone();
                    adapter.panel.update(cx, |toolbar, cx| {
                        toolbar.set_draw_options(options.clone(), cx)
                    });
                }
                if let Some(adapter) = &self.gpui_properties {
                    let mut draw = adapter.draw.read(cx).view_data().clone();
                    if draw.options != options {
                        draw.options = options;
                        adapter
                            .draw
                            .update(cx, |panel, cx| panel.set_view_data(draw, cx));
                    }
                }
                cx.notify();
            }
            ToolbarAction::DrawActionInvoked { action } => {
                use fanta_gpui::toolbar::DrawToolbarAction;
                match action {
                    DrawToolbarAction::SelectAll => self.select_all_draw_content(cx),
                    DrawToolbarAction::Deselect => self.deselect_canvas(cx),
                    DrawToolbarAction::InvertSelection => self.invert_draw_selection(cx),
                    DrawToolbarAction::ClosePath
                    | DrawToolbarAction::JoinPaths
                    | DrawToolbarAction::SimplifyPath => {
                        use crate::layer_context_ops::PathAction;
                        let action = match action {
                            DrawToolbarAction::ClosePath => PathAction::Close,
                            DrawToolbarAction::JoinPaths => PathAction::Join,
                            _ => PathAction::Simplify,
                        };
                        self.run_path_action(action, None, window, cx);
                    }
                    DrawToolbarAction::ApplyCrop if self.tools.kind() == ToolKind::Crop => {
                        if self.tools.can_apply_crop() {
                            self.dispatch_tool_event(
                                key_event(LogicalKey::Enter, window.modifiers()),
                                cx,
                            );
                        }
                    }
                    DrawToolbarAction::CancelCrop if self.tools.kind() == ToolKind::Crop => {
                        self.dispatch_tool_event(
                            key_event(LogicalKey::Escape, window.modifiers()),
                            cx,
                        );
                    }
                    other => notify_unavailable(&format!("{other:?}"), window, cx),
                }
            }
            ToolbarAction::CommandQueryChanged { .. } => {}
        }
    }

    pub(crate) fn draw_path_simplification_tolerance(&self, cx: &App) -> f64 {
        #[cfg(feature = "fanta-gpui-ui")]
        let smoothing = self
            .gpui_properties
            .as_ref()
            .map(|adapter| adapter.draw.read(cx).view_data().options.smoothing)
            .or_else(|| {
                self.gpui_toolbar
                    .as_ref()
                    .map(|adapter| adapter.draw_options.smoothing)
            })
            .unwrap_or_default();
        #[cfg(not(feature = "fanta-gpui-ui"))]
        let smoothing = {
            let _ = cx;
            0
        };
        0.2 + f64::from(smoothing) * 0.058
    }

    #[cfg(feature = "fanta-gpui-ui")]
    fn deselect_canvas(&mut self, cx: &mut Context<Self>) {
        self.tools.clear_draw_selection_region();
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
    }

    #[cfg(feature = "fanta-gpui-ui")]
    fn invert_draw_selection(&mut self, cx: &mut Context<Self>) {
        if self
            .item
            .read(cx)
            .document()
            .is_some_and(|document| self.tools.invert_draw_selection_region(&document.doc))
        {
            cx.notify();
            return;
        }
        self.tools.clear_draw_selection_region();
        self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let candidates = fanta_tools::select::draw_content_candidates(
                    &document.doc.scene,
                    document.doc.active_page(),
                );
                let inverted = candidates
                    .into_iter()
                    .filter(|id| !document.doc.selection.contains(*id))
                    .collect::<Vec<_>>();
                document.doc.selection.replace_with(inverted);
                ((), DocChange::Selection)
            });
        });
    }

    #[cfg(feature = "fanta-gpui-ui")]
    fn select_all_draw_content(&mut self, cx: &mut Context<Self>) {
        self.tools.clear_draw_selection_region();
        self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                let candidates = fanta_tools::select::draw_content_candidates(
                    &document.doc.scene,
                    document.doc.active_page(),
                );
                document.doc.selection.replace_with(candidates);
                ((), DocChange::Selection)
            });
        });
    }

    #[cfg(feature = "fanta-gpui-ui")]
    fn add_toolbar_keyframe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_editable(cx) {
            return;
        }
        let has_single_selection = self
            .item
            .read(cx)
            .document()
            .and_then(|document| single_selection(&document.doc))
            .is_some();
        if !has_single_selection {
            show_canvas_notice(
                "Select one layer before adding a keyframe.".into(),
                window,
                cx,
            );
            return;
        }
        if self.active_motion_clip.is_none() {
            self.create_motion_clip(cx);
        }
        self.add_motion_keyframe(TimelineProperty::Opacity, cx);
    }

    #[cfg(feature = "fanta-gpui-ui")]
    fn set_toolbar_motion_playing(
        &mut self,
        playing: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if playing {
            self.finish_document_edits(cx);
        }
        self.timeline_shell
            .update(cx, |clock, cx| clock.set_playing(playing, cx));
        if playing && !self.timeline_shell.read(cx).is_playing() {
            show_canvas_notice(
                "Add an animation to a layer before playing the Motion preview.".into(),
                window,
                cx,
            );
        }
        cx.notify();
    }

    #[cfg(feature = "fanta-gpui-ui")]
    fn apply_toolbar_layer_command(
        &mut self,
        action: fanta_gpui::layers::LayersPanelContextAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_structure_edit(
            action.label(),
            move |doc| {
                let id = single_selection(doc).context("select exactly one layer")?;
                let operations = crate::layer_context_ops::simple(doc, id, action)?;
                let roots = crate::layer_context_ops::created_roots(&operations);
                let selection = if roots.is_empty() { vec![id] } else { roots };
                Ok((operations, selection))
            },
            window,
            cx,
        );
    }

    #[cfg(feature = "fanta-gpui-ui")]
    fn detach_toolbar_instance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_structure_edit(
            "Detach instance",
            |doc| {
                let id = single_selection(doc).context("select exactly one component instance")?;
                anyhow::ensure!(
                    doc.scene
                        .get(id)
                        .is_some_and(|node| matches!(&node.data, NodeData::Instance(_))),
                    "select a component instance"
                );
                anyhow::ensure!(
                    crate::layer_context_ops::editable(doc, id),
                    "the selected instance is locked"
                );
                let operations = crate::properties_ops::detach_instance_operations(doc, id)?;
                anyhow::ensure!(!operations.is_empty(), "the instance cannot be detached");
                Ok((operations, vec![id]))
            },
            window,
            cx,
        );
    }

    #[cfg(feature = "fanta-gpui-ui")]
    fn open_toolbar_color_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor_mode(cx) == EditorMode::Draw {
            let color = self.item.read(cx).document().and_then(|document| {
                let page = document
                    .page(self.selected_page_index)
                    .and_then(|page| page.root);
                crate::gpui_adapters::toolbar::draw_sample_color(&document.doc, page)
            });
            let Some(color) = color else {
                show_canvas_notice(
                    "Select one layer with a solid color, or deselect to sample the page background."
                        .into(),
                    window,
                    cx,
                );
                return;
            };
            if !self.inspector_sidebar_expanded(cx) {
                self.toggle_inspector_sidebar(&ToggleInspectorSidebar, window, cx);
            }
            self.refresh_properties_inspector(window, cx);
            let Some(draw) = self
                .gpui_properties
                .as_ref()
                .map(|adapter| adapter.draw.clone())
            else {
                show_canvas_notice("The Draw inspector is unavailable.".into(), window, cx);
                return;
            };
            draw.update(cx, |inspector, cx| {
                let mut data = inspector.view_data().clone();
                data.color_hex = color.to_hex().trim_start_matches('#').to_owned().into();
                inspector.set_view_data(data, cx);
            });
            return;
        }
        self.set_editor_mode(EditorMode::Design, cx);
        if !self.inspector_sidebar_expanded(cx) {
            self.toggle_inspector_sidebar(&ToggleInspectorSidebar, window, cx);
        }
        self.refresh_properties_inspector(window, cx);
        let Some(inspector) = self
            .gpui_properties
            .as_ref()
            .map(|adapter| adapter.design.downgrade())
        else {
            show_canvas_notice("The design inspector is unavailable.".into(), window, cx);
            return;
        };
        match inspector.update(cx, |inspector, cx| {
            inspector
                .controller()
                .update(cx, |panel, cx| panel.open_color_picker(window, cx))
        }) {
            Ok(true) => {}
            Ok(false) => show_canvas_notice(
                "Select a page or a layer with a fill to edit its color.".into(),
                window,
                cx,
            ),
            Err(error) => log::warn!("opening the design color picker failed: {error:#}"),
        }
    }

    /// The toolbar uses the active Design export settings when the new
    /// inspector is mounted, including the same destination and format rows.
    #[cfg(feature = "fanta-gpui-ui")]
    fn export_from_toolbar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.gpui_design.is_some() {
            self.export_design_selection(window, cx);
            return;
        }
        if !self.inspector_sidebar_expanded(cx) {
            self.toggle_inspector_sidebar(&ToggleInspectorSidebar, window, cx);
        }
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
    fn open_toolbar_agent_attachment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.add_selection_to_agent(window, cx) {
            return;
        }
        let Some(workspace) = window
            .root::<MultiWorkspace>()
            .flatten()
            .map(|multi_workspace| multi_workspace.read(cx).workspace().clone())
        else {
            return;
        };
        if let Err(error) = agent_ui::open_agent_add_context_menu(workspace, window, cx) {
            log::error!("opening the toolbar Agent attachment workflow failed: {error:#}");
            show_canvas_notice(
                format!("The Agent attachment workflow could not be opened: {error:#}"),
                window,
                cx,
            );
        }
    }

    /// Mention the selected layers in the Agent Panel's draft: a link that
    /// selects them again when clicked, carrying a snapshot the agent reads.
    /// Returns false, having done nothing, when nothing is selected.
    pub(crate) fn add_selection_to_agent(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let selection = match self.agent_canvas_selection_snapshot(cx) {
            Ok(Some(selection)) => selection,
            Ok(None) => return false,
            Err(error) => {
                log::error!("snapshotting the canvas selection failed: {error:#}");
                show_canvas_notice(
                    format!("The canvas selection could not be attached: {error:#}"),
                    window,
                    cx,
                );
                return true;
            }
        };
        let Some(workspace) = window
            .root::<MultiWorkspace>()
            .flatten()
            .map(|multi_workspace| multi_workspace.read(cx).workspace().clone())
        else {
            show_canvas_notice(
                "This window has no workspace for the Agent Panel.".to_string(),
                window,
                cx,
            );
            return true;
        };
        if let Err(error) = agent_ui::attach_canvas_selection_for_review(
            workspace,
            selection.name,
            selection.nodes,
            selection.content,
            window,
            cx,
        ) {
            log::error!("attaching the canvas selection failed: {error:#}");
            show_canvas_notice(
                format!("The canvas selection could not be attached: {error:#}"),
                window,
                cx,
            );
        }
        true
    }

    pub(crate) fn agent_canvas_selection_snapshot(
        &self,
        cx: &App,
    ) -> anyhow::Result<Option<AgentCanvasSelection>> {
        let item = self.item.read(cx);
        let Some(document) = item.document() else {
            return Ok(None);
        };
        let doc = &document.doc;
        if doc.selection.is_empty() {
            return Ok(None);
        }
        let active_root = doc
            .active_page()
            .context("the canvas has no active page or component root")?;
        let active_root_node = doc
            .scene
            .get(active_root)
            .with_context(|| format!("the active canvas root {active_root} no longer exists"))?;
        let (scope_kind, scope_name) = if let Some(page) = document
            .pages
            .iter()
            .find(|page| page.root == Some(active_root))
        {
            let live_name = active_root_node.name.as_str();
            (
                "page",
                if live_name.is_empty() {
                    page.name.as_ref()
                } else {
                    live_name
                },
            )
        } else if let Some(component) = doc
            .components
            .defs
            .values()
            .find(|component| component.root == active_root)
        {
            ("component", component.name.as_str())
        } else {
            anyhow::bail!(
                "the active canvas root {active_root} is not a document page or component"
            );
        };
        let scope_name = crate::agent_surface::truncate_summary_string(
            if scope_name.is_empty() {
                "Untitled"
            } else {
                scope_name
            },
            crate::agent_surface::SUMMARY_LABEL_CHARS,
        );
        for node_id in doc.selection.iter().copied() {
            if doc.scene.get(node_id).is_none() {
                anyhow::bail!("selected node {node_id} no longer exists");
            }
            let inside_active_root = node_id == active_root
                || doc
                    .scene
                    .ancestors_of(node_id)
                    .any(|ancestor| ancestor.id == active_root);
            if !inside_active_root {
                anyhow::bail!(
                    "selected node {node_id} is outside the active {scope_kind} root {active_root}"
                );
            }
        }
        let item_title = item.title();
        let source = serde_json::json!({
            "item_title": crate::agent_surface::truncate_summary_string(
                item_title.as_ref(),
                crate::agent_surface::SUMMARY_LABEL_CHARS,
            ),
            "document_id": doc.id.to_string(),
            "document_path": item.abs_path().display().to_string(),
            "project_root": item.project_root().map(|root| root.display().to_string()),
            "active_root_id": active_root.to_string(),
            "scope_kind": scope_kind,
            "scope_name": scope_name,
        });
        let mut nodes: Vec<_> = doc
            .selection
            .iter()
            .take(AGENT_ATTACHMENT_CONTEXT_LAYERS)
            .map(|node_id| crate::agent_surface::node_summary(doc, *node_id, Some(0), true))
            .collect();
        let selected_count = doc.selection.len();
        let content = loop {
            let included_count = nodes.len();
            let snapshot = serde_json::json!({
                "kind": "fanta_canvas_selection",
                "source": &source,
                "scope": { "kind": scope_kind, "name": &scope_name },
                "selected_count": selected_count,
                "included_count": included_count,
                "omitted_count": selected_count.saturating_sub(included_count),
                "nodes": &nodes,
                "usage": "This is an immutable attach-time snapshot. Before using its node ids with live design tools, call design_state and verify document_id, document_path, project_root, and active_root_id against source. If any differ, ask the user to focus the source canvas; never apply these ids to another document or canvas root.",
            });
            let content = serde_json::to_string_pretty(&snapshot)?;
            if content.len() <= AGENT_ATTACHMENT_MAX_BYTES {
                break content;
            }
            if nodes.pop().is_none() {
                anyhow::bail!(
                    "the canvas selection metadata exceeds the {AGENT_ATTACHMENT_MAX_BYTES}-byte attachment limit"
                );
            }
        };
        let name = match doc.selection.as_slice() {
            [node] => doc
                .scene
                .get(*node)
                .map(|node| node.name.trim())
                .filter(|name| !name.is_empty())
                .map_or_else(
                    || "Untitled layer".to_string(),
                    |name| {
                        crate::agent_surface::truncate_summary_string(
                            name,
                            crate::agent_surface::SUMMARY_LABEL_CHARS,
                        )
                    },
                ),
            _ => format!("{selected_count} layers on {scope_name}"),
        };
        let included = nodes.len();
        let nodes = doc
            .selection
            .iter()
            .take(included)
            .map(|node| node.to_string())
            .collect();
        Ok(Some(AgentCanvasSelection {
            name,
            nodes,
            content,
        }))
    }

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
             design_edit (change) and design_screenshot (verify). Prefer editing the real .fnx source with file tools, then verify on the canvas. Read canvas comments with design_comments and reply in their original threads.",
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
                self.set_toolbar_motion_playing(*playing, window, cx);
            }
            (ToolbarSecondaryControl::MotionLoop, ToolbarControlValue::Toggle(looping)) => {
                self.timeline_shell
                    .update(cx, |timeline, cx| timeline.set_loop_playback(*looping, cx));
                cx.notify();
            }
            (ToolbarSecondaryControl::MotionAddKeyframe, ToolbarControlValue::Choice(label)) => {
                let Some(property) = TimelineProperty::from_label(label.as_ref()) else {
                    show_canvas_notice(
                        "Choose a keyframe property from the toolbar menu.".into(),
                        window,
                        cx,
                    );
                    return;
                };
                if let Err(error) = self.try_add_motion_keyframe(property, cx) {
                    show_canvas_notice(
                        format!("Could not add {} keyframe: {error:#}", property.label()),
                        window,
                        cx,
                    );
                }
            }
            (ToolbarSecondaryControl::MotionAnimationStyle, ToolbarControlValue::Choice(style)) => {
                let result = self.motion_sidebar.update(cx, |panel, cx| {
                    panel.apply_toolbar_animation_style(self.active_motion_clip, style, cx)
                });
                match result {
                    Ok(_) => {
                        if let Some(adapter) = self.gpui_toolbar.as_mut() {
                            adapter.accept_animation_style(style);
                        }
                        self.sync_motion_timeline(cx);
                        cx.notify();
                    }
                    Err(error) => show_canvas_notice(error.user_message(style), window, cx),
                }
            }
            (ToolbarSecondaryControl::MotionAutoKeyframe, ToolbarControlValue::Toggle(enabled)) => {
                if let Err(error) = self.set_motion_auto_keyframe(*enabled, cx) {
                    show_canvas_notice(error.to_string(), window, cx);
                }
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
                self.add_toolbar_keyframe(window, cx);
            }
            ToolbarSecondaryControl::MotionTimeline => {
                show_canvas_notice(
                    "The Motion timeline is always visible in Motion mode.".to_string(),
                    window,
                    cx,
                );
            }
            ToolbarSecondaryControl::MotionTimeComment => match self.arm_motion_time_comment(cx) {
                Ok(time_ms) => show_canvas_notice(
                    format!(
                        "Click the canvas to place a comment at {}.",
                        crate::comments::motion_comment_time_label(time_ms)
                    ),
                    window,
                    cx,
                ),
                Err(error) => {
                    show_canvas_notice(format!("Could not add time comment: {error:#}"), window, cx)
                }
            },
            ToolbarSecondaryControl::DevInspect => {
                self.activate_tool_from_action(ToolKind::Inspect, window, cx)
            }
            ToolbarSecondaryControl::DevAnnotate => {
                self.activate_tool_from_action(ToolKind::Annotation, window, cx)
            }
            ToolbarSecondaryControl::DevMeasure => {
                self.activate_tool_from_action(ToolKind::Measure, window, cx)
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
pub(crate) const CANVAS_NOTICE_ID: &str = "fanta-canvas-notice";

/// How many selected layers a toolbar Agent prompt lists by id.
#[cfg(feature = "fanta-gpui-ui")]
const AGENT_PROMPT_CONTEXT_LAYERS: usize = 8;
const AGENT_ATTACHMENT_CONTEXT_LAYERS: usize = 64;
const AGENT_ATTACHMENT_MAX_BYTES: usize = 128 * 1024;

/// The selection the Agent Panel's draft mentions: its label, the layers a
/// click on the mention reveals, and the snapshot the agent reads.
pub(crate) struct AgentCanvasSelection {
    pub(crate) name: String,
    pub(crate) nodes: Vec<String>,
    pub(crate) content: String,
}

/// The draft prompt a text-editing toolbar command opens in the Agent Panel.
/// Generation commands open the dedicated generation workspace.
#[cfg(feature = "fanta-gpui-ui")]
fn toolbar_agent_prompt_template(
    command: fanta_gpui::toolbar::ToolbarCommand,
) -> Option<&'static str> {
    use fanta_gpui::toolbar::ToolbarCommand;
    match command {
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
///
/// Code that is already updating the workspace uses
/// [`show_workspace_canvas_notice`] instead.
pub(crate) fn show_canvas_notice(message: String, window: &mut Window, cx: &mut App) {
    show_workspace_toast(CANVAS_NOTICE_ID, message, true, window, cx);
}

/// [`show_canvas_notice`] for a workspace action handler or a
/// `workspace.update` closure: finding the window's workspace and updating it
/// again from there would panic, because GPUI leases an entity to one update
/// at a time.
pub(crate) fn show_workspace_canvas_notice(
    workspace: &mut Workspace,
    message: String,
    cx: &mut Context<Workspace>,
) {
    workspace.show_toast(
        Toast::new(NotificationId::named(CANVAS_NOTICE_ID.into()), message).autohide(),
        cx,
    );
}

fn show_workspace_toast(
    id: &'static str,
    message: String,
    autohide: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(workspace) = window
        .root::<MultiWorkspace>()
        .flatten()
        .map(|multi_workspace| multi_workspace.read(cx).workspace().clone())
    else {
        log::warn!("fanta: no workspace is available to show a notice in: {message}");
        return;
    };
    workspace.update(cx, |workspace, cx| {
        let toast = Toast::new(NotificationId::named(id.into()), message);
        workspace.show_toast(if autohide { toast.autohide() } else { toast }, cx);
    });
}

/// The window a notice raised without a `Window` lands in: the active one,
/// or — when Fanta is in the background, which is exactly when a debounced
/// save finishes unwatched — the first workspace window.
fn notice_window(cx: &App) -> Option<AnyWindowHandle> {
    cx.active_window()
        .filter(|window| window.downcast::<MultiWorkspace>().is_some())
        .or_else(|| {
            cx.windows()
                .into_iter()
                .find(|window| window.downcast::<MultiWorkspace>().is_some())
        })
}

fn show_toast_deferred(id: &'static str, message: String, autohide: bool, cx: &mut App) {
    cx.defer(move |cx| {
        let Some(window) = notice_window(cx) else {
            log::warn!("fanta: no workspace window to show a notice in: {message}");
            return;
        };
        window
            .update(cx, |_, window, cx| {
                show_workspace_toast(id, message, autohide, window, cx)
            })
            .log_err();
    });
}

/// Say which designs a load left out. It stays until dismissed: the user has
/// files to repair, and nothing else on screen shows the design is missing.
pub(crate) fn show_unavailable_designs_notice_deferred(message: String, cx: &mut App) {
    show_toast_deferred("fanta-unavailable-designs", message, false, cx);
}

/// Say what the project check found after an outside change. It stays until
/// dismissed, and a newer result replaces it.
pub(crate) fn show_external_findings_notice_deferred(message: String, cx: &mut App) {
    show_toast_deferred("fanta-external-findings", message, false, cx);
}

/// Show a canvas notice from a place that holds no `Window` — an item event
/// subscription. The notice lands at the next effect flush, which is also
/// when the reload it announces has finished applying.
pub(crate) fn show_canvas_notice_deferred(message: String, cx: &mut App) {
    show_toast_deferred(CANVAS_NOTICE_ID, message, true, cx);
}

fn save_failure_notice_id(item: gpui::EntityId) -> NotificationId {
    NotificationId::composite::<FigItem>(("save-failure", item))
}

fn sync_save_failure_notice_deferred(
    item: gpui::EntityId,
    state: Rc<RefCell<Option<SharedString>>>,
    cx: &mut App,
) {
    cx.defer(move |cx| {
        let mut workspaces = Vec::new();
        for window in cx.windows() {
            if let Some(window) = window.downcast::<MultiWorkspace>() {
                if let Some(owners) = window
                    .update(cx, |multi_workspace, _, _| {
                        multi_workspace.workspaces().cloned().collect::<Vec<_>>()
                    })
                    .log_err()
                {
                    workspaces.extend(owners);
                }
            }
        }
        let owners = workspaces
            .into_iter()
            .map(|workspace| {
                let owns_item = workspace
                    .read(cx)
                    .items_of_type::<FigView>(cx)
                    .any(|view| view.read(cx).item.entity_id() == item);
                (workspace, owns_item)
            })
            .collect::<Vec<_>>();
        if !owners.iter().any(|(_, owns_item)| *owns_item) {
            state.borrow_mut().take();
        }
        let message = state.borrow().clone();
        let id = save_failure_notice_id(item);
        for (workspace, owns_item) in owners {
            workspace.update(cx, |workspace, cx| {
                if let Some(message) = message.as_ref().filter(|_| owns_item) {
                    workspace.show_toast(Toast::new(id.clone(), message.to_string()), cx);
                } else {
                    workspace.dismiss_toast(&id, cx);
                }
            });
        }
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
