//! The workspace item view for Figma documents: canvas chrome (floating tool
//! pill, page picker, project controls), input routing into the tool shell,
//! and the `Item` integration that gives fanta projects text-editor-style
//! dirty tracking and save.

use anyhow::Result;
use fanta_canvas::HitPrecision;
use fanta_doc::{
    AnimationClip, AnimationClipId, AnimationTrack, AnimationTrackId, BoundProp, Easing,
    Interpolation, Keyframe, KeyframeId, MotionEvaluation, MotionProperty, MotionTarget,
    MotionTransform, NodeId, Operation, ResolvedVarValue, Transaction, Viewport,
};
use fanta_tools::{Button as ToolButton, LogicalKey, ToolEvent};
use file_icons::FileIcons;
use glam::DVec2;
use gpui::{
    Action, Anchor, AnyElement, App, Bounds, Context, CursorStyle, DragMoveEvent, Empty, Entity,
    EventEmitter, FocusHandle, Focusable, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PinchEvent, Pixels, Point, Render, ScrollDelta, ScrollWheelEvent, SharedString,
    Subscription, Task, Window, actions, div, px,
};
use language::Capability;
use project::Project;
use settings::{Settings as _, update_settings_file};
use ui::{ContextMenu, ContextMenuEntry, Divider, IconPosition, PopoverMenu, Tooltip, prelude::*};
use util::{ResultExt, paths::PathExt};
use workspace::{
    ItemSettings, Pane,
    item::{Item, ItemEvent, ProjectItem, SaveOptions, TabContentParams},
};

use crate::canvas::{
    CanvasElement, RenderedCanvas, bounds_size, evaluated_hit_test_screen,
    screen_position_in_bounds,
};
use crate::code_workspace::FantaCodeWorkspace;
use crate::comments_panel::{FantaCommentsPanel, document_comment_rows};
use crate::design_panel::FantaDesignPanel;
use crate::document::{DocChange, FigDocument, FigItem, FigItemEvent};
use crate::editor_session::{
    EditorMode, EditorModeTabs, EditorSession, EditorWorkspace, EditorWorkspaceTabs,
};
use crate::motion_edit::{
    MotionKeyframeDragSession, delete_keyframe_operation, rename_clip_operation,
    set_clip_duration_operation,
};
use crate::panel_settings::{FantaDesignPanelSettings, FantaPropertiesPanelSettings};
use crate::properties_panel::FantaPropertiesPanel;
use crate::prototype_panel::FantaPrototypePanel;
use crate::text_edit::CanvasTextEdit;
use crate::timeline::{
    TIMELINE_HEIGHT, TimelineEditPhase, TimelineEvent, TimelineKeyframeSelection,
    TimelineKeyframeViewModel, TimelineProperty, TimelineShell, TimelineTrackViewModel,
    TimelineViewModel,
};
use crate::tools::{
    TOOLBAR_GROUPS, ToolKind, ToolShell, key_event, move_event, pointer_button, press_event,
    release_event, tool_context,
};
use crate::variables_workspace::FantaVariablesWorkspace;

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
        /// Activate the comment tool (click the canvas to pin a comment).
        ActivateCommentTool,
        /// Show or hide the embedded layers sidebar.
        ToggleLayersSidebar,
        /// Show or hide the embedded inspector sidebar.
        ToggleInspectorSidebar,
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
    pub(crate) viewport: Option<Viewport>,
    pan_last_position: Option<Point<Pixels>>,
    primary_pressed: bool,
    /// Space is held: the canvas temporarily pans with any active tool, the
    /// Figma/Illustrator "hold space to pan" gesture. Cleared on key-up.
    space_pan: bool,
    pub(crate) container_bounds: Option<Bounds<Pixels>>,
    pub(crate) rendered_canvas: Option<RenderedCanvas>,
    #[cfg(target_os = "macos")]
    gpu_renderer: Option<MacGpuRenderer>,
    pub(crate) tools: ToolShell,
    pub(crate) comment_state: crate::comments_ui::CommentState,
    /// The last-used tool per toolbar group, so each group's button keeps
    /// showing the member you last picked (Figma behavior). Indexed by group.
    group_faces: Vec<ToolKind>,
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
    _item_subscription: Subscription,
    _editor_session_subscription: Subscription,
    _timeline_subscription: Subscription,
}

pub enum FigViewEvent {
    Edited,
    TitleChanged,
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
    fn new(
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
        let editor_session = cx.new(|_| EditorSession::new());
        let editor_session_subscription = cx.observe(&editor_session, |_, _, cx| cx.notify());
        let (layers_sidebar, inspector_sidebar) = Self::new_embedded_sidebars(&project, window, cx);
        let prototype_sidebar = cx.new(|cx| FantaPrototypePanel::new(item.clone(), cx));
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
        // A space held across a focus change (panel click, window switch, a
        // text session opening) delivers its key-up elsewhere; without this
        // reset `space_pan` stays true and the Select tool pans with a hand
        // cursor until space is pressed again.
        cx.on_focus_out(&focus_handle, window, |this: &mut Self, _, _, cx| {
            if this.space_pan || this.pan_last_position.is_some() {
                this.space_pan = false;
                this.pan_last_position = None;
                cx.notify();
            }
        })
        .detach();
        Self {
            item,
            project,
            focus_handle,
            editor_session,
            layers_sidebar,
            inspector_sidebar,
            prototype_sidebar,
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
            viewport: None,
            pan_last_position: None,
            primary_pressed: false,
            space_pan: false,
            container_bounds: None,
            rendered_canvas: None,
            #[cfg(target_os = "macos")]
            gpu_renderer: None,
            tools: ToolShell::new(),
            comment_state: crate::comments_ui::CommentState::default(),
            group_faces: crate::tools::initial_group_faces(),
            fonts_prewarmed: false,
            hovered_node: None,
            text_edit: None,
            pending_text_edit: None,
            _item_subscription: item_subscription,
            _editor_session_subscription: editor_session_subscription,
            _timeline_subscription: timeline_subscription,
        }
    }

    fn subscribe_to_item(item: &Entity<FigItem>, cx: &mut Context<Self>) -> Subscription {
        cx.subscribe(item, |this, _, event: &FigItemEvent, cx| {
            match event {
                FigItemEvent::Edited => {
                    this.invalidate_canvas_cache();
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
                FigItemEvent::SelectionChanged | FigItemEvent::TextSelectionChanged => {}
                FigItemEvent::StateChanged => {
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
                FigItemEvent::ConflictChanged => {
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
            }
            cx.notify();
        })
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
                    let mut tool_context = tool_context(&mut document.doc, viewport, screen_size);
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
        self.finish_motion_keyframe_drag(cx);
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
        #[cfg(target_os = "macos")]
        if let Some(renderer) = self.gpu_renderer.as_mut() {
            renderer.invalidate();
        }
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn take_gpu_renderer(&mut self) -> Option<MacGpuRenderer> {
        self.gpu_renderer.take()
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn store_gpu_renderer(&mut self, renderer: MacGpuRenderer) {
        self.gpu_renderer = Some(renderer);
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

        let tools = &mut self.tools;
        let mut wants_exit = false;
        let mut content_changed = false;
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
            && event.click_count == 2
            && self.editor_mode(cx) == EditorMode::Design
            && self.tools.kind() == ToolKind::Select
            && self.is_editable(cx)
            && let Some(bounds) = self.container_bounds
        {
            let screen = screen_position_in_bounds(event.position, bounds);
            if let Some(node) = self.text_node_at(screen, cx) {
                self.open_text_edit(node, TextEditSeed::WordAt(screen), window, cx);
                return;
            }
            // No real text under the cursor — try text inside a component
            // instance (a virtual clone, edited via an override).
            if let Some(target) = self.instance_text_at(screen, cx) {
                self.open_instance_text_edit(target, TextEditSeed::WordAt(screen), window, cx);
                return;
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
        self.finish_document_edits(cx);
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
        self.finish_document_edits(cx);
        self.item.update(cx, |item, cx| {
            if let Err(error) = item.redo(cx) {
                log::error!("fig_viewer redo failed: {error:#}");
            }
        });
    }

    fn cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
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
        if self.is_editable(cx) {
            self.finish_document_edits(cx);
            self.dispatch_tool_event(key_event(LogicalKey::Delete, window.modifiers()), cx);
        }
    }

    fn nudge(&mut self, key: LogicalKey, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_editable(cx) {
            self.finish_document_edits(cx);
            self.dispatch_tool_event(key_event(key, window.modifiers()), cx);
        }
    }

    // === Pages ============================================================

    pub fn select_page(&mut self, index: usize, cx: &mut Context<Self>) {
        // The edited node stays behind on the old page; end the session
        // before the canvas stops rendering it.
        self.finish_document_edits(cx);
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
        self.finish_document_edits(cx);
        self.item.update(cx, |item, cx| item.save(cx))
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
        div()
            .id("fanta-layers-sidebar")
            .relative()
            .h_full()
            .w(self.layers_sidebar_width)
            .flex_shrink_0()
            .border_r_1()
            .border_color(cx.theme().colors().border)
            .child(self.layers_sidebar.clone())
            .child(self.render_sidebar_resize_handle(SidebarKind::Layers))
            .into_any_element()
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
            EditorMode::Design | EditorMode::Motion => {
                self.inspector_sidebar.clone().into_any_element()
            }
        };
        div()
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
            .child(self.render_sidebar_resize_handle(SidebarKind::Inspector))
            .into_any_element()
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
            .justify_center()
            .child(
                h_flex()
                    .occlude()
                    .max_w(px(620.))
                    .mx_4()
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
                        Label::new(
                            "Canvas editing is locked while FNX has unsaved changes. Pan and selection remain available; save FNX to resume editing.",
                        )
                        .size(LabelSize::Small),
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
            .absolute()
            .bottom(if self.editor_mode(cx) == EditorMode::Motion {
                TIMELINE_HEIGHT + px(16.)
            } else {
                px(16.)
            })
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

fn clamp_sidebar_width(width: Pixels, sidebar: SidebarKind) -> Pixels {
    let minimum = match sidebar {
        SidebarKind::Layers => MIN_LAYERS_SIDEBAR_WIDTH,
        SidebarKind::Inspector => MIN_INSPECTOR_SIDEBAR_WIDTH,
    };
    px(width.as_f32().clamp(minimum, MAX_SIDEBAR_WIDTH))
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
                let workspace_body = if editor_workspace == EditorWorkspace::Variables {
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
                            h_flex()
                                .size_full()
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
                                                .on_mouse_move(cx.listener(Self::handle_mouse_move))
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
                                                    c
                                                }),
                                        )
                                        .children(
                                            (editor_mode == EditorMode::Motion)
                                                .then(|| self.timeline_shell.clone()),
                                        ),
                                )
                                .children(
                                    self.inspector_sidebar_visible
                                        .then(|| self.render_inspector_sidebar(cx)),
                                ),
                        )
                        .child(self.render_tool_pill(cx))
                        .children(
                            self.item
                                .read(cx)
                                .source_edit_locked()
                                .then(|| self.render_source_edit_lock_banner(cx)),
                        )
                        .into_any_element()
                };
                this.child(workspace_body)
                    .child(self.render_workspace_tabs(cx))
            })
    }
}

fn single_selection(doc: &fanta_doc::Doc) -> Option<NodeId> {
    let mut selection = doc.selection.iter().copied();
    let node = selection.next()?;
    selection.next().is_none().then_some(node)
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
    let tracks = clip
        .tracks
        .values()
        .map(|track| {
            let node_name = doc
                .scene
                .get(track.target.node)
                .map(|node| node.name.as_str())
                .filter(|name| !name.is_empty())
                .unwrap_or("Layer");
            TimelineTrackViewModel {
                id: track.id.to_string().into(),
                label: format!(
                    "{node_name} · {}",
                    motion_property_label(track.target.property)
                )
                .into(),
                keyframes: track
                    .keyframes
                    .values()
                    .map(|keyframe| TimelineKeyframeViewModel {
                        id: keyframe.id.to_string().into(),
                        time_us: i64::from(keyframe.time_ms) * 1_000,
                    })
                    .collect(),
            }
        })
        .collect();
    TimelineViewModel::for_clip(
        clip.name.clone(),
        i64::from(clip.duration_ms) * 1_000,
        tracks,
    )
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
        self.item.read(cx).has_conflict()
    }

    fn can_save(&self, cx: &App) -> bool {
        self.item.read(cx).has_ready_document()
    }

    fn save(
        &mut self,
        _options: SaveOptions,
        _project: Entity<Project>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
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
        self.finish_document_edits(cx);
        self.item.update(cx, |item, cx| item.reload_from_disk(cx))
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
                viewport,
                pan_last_position: None,
                primary_pressed: false,
                space_pan: false,
                container_bounds: None,
                rendered_canvas: None,
                #[cfg(target_os = "macos")]
                gpu_renderer: None,
                tools: ToolShell::new(),
                comment_state: crate::comments_ui::CommentState::default(),
                group_faces: crate::tools::initial_group_faces(),
                fonts_prewarmed: false,
                hovered_node: None,
                text_edit: None,
                pending_text_edit: None,
                _item_subscription: item_subscription,
                _editor_session_subscription: editor_session_subscription,
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
        CanvasNode, Color, GroupNode, Mode, ModeId, NodeData, Operation, VarValue, Variable,
        VariableCollection, VariableCollectionId, VariableId, VariableType, VectorNode,
    };
    use gpui::{TestAppContext, point, size};
    use project::FakeFs;
    use settings::SettingsStore;

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
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
        track.keyframes.insert(
            keyframe_id,
            Keyframe::new(keyframe_id, 250, ResolvedVarValue::Float { value: 10.0 }),
        );
        let mut clip = AnimationClip::new(clip_id, "Entrance", 1_500);
        clip.tracks.insert(track_id, track);
        doc.motion.clips.insert(clip_id, clip);

        let model = motion_timeline_model(&doc, Some(clip_id));
        assert_eq!(model.clip_name.as_deref(), Some("Entrance"));
        assert_eq!(model.duration_us, 1_500_000);
        assert_eq!(model.tracks.len(), 1);
        assert_eq!(model.tracks[0].label.as_ref(), "Star · Position X");
        assert_eq!(model.tracks[0].keyframes.len(), 1);
        assert_eq!(
            model.tracks[0].keyframes[0].id.as_ref(),
            keyframe_id.to_string()
        );
        assert_eq!(model.tracks[0].keyframes[0].time_us, 250_000);
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
}
