use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollHandle, SharedString,
    Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, h_flex,
    input::{InputEvent, InputState},
    v_flex,
};

use super::{
    AgentToolbarOptions, CloseToolbarOverlay, ConfirmToolbarTextEntry, DecrementToolbarControl,
    DevToolbarOptions, EnterDevMode, FirstToolbarCommand, IncrementToolbarControl,
    LastToolbarCommand, MotionToolbarOptions, NextToolbarCommand, OpenToolbarActions,
    OpenToolbarAgent, PreviousToolbarCommand, SelectAnnotationTool, SelectArrowTool,
    SelectCommentTool, SelectEllipseTool, SelectFrameTool, SelectHandTool, SelectImageVideoTool,
    SelectLineTool, SelectMeasureTool, SelectMoveTool, SelectPenTool, SelectPencilTool,
    SelectRectangleTool, SelectResourcesTool, SelectScaleTool, SelectSectionTool, SelectSliceTool,
    SelectTextTool, ToolbarAction, ToolbarChromeControl, ToolbarCommand, ToolbarMode,
    ToolbarSecondaryControl, ToolbarTool, ToolbarToolGroup, ZoomCanvasTo100, ZoomCanvasToFit,
    ZoomCanvasToSelection,
    commands::{TOOLBAR_KEY_CONTEXT, TOOLBAR_TEXT_ENTRY_KEY_CONTEXT},
};
use crate::molecules::EdgeFades;

mod overlays;
mod rows;
mod state;

const TOOL_SIZE: f32 = 32.;
const POPOVER_GAP: f32 = 8.;
/// Width of the explicit AI action in the compact primary row. Keeping this
/// fixed also gives the anchored composer a stable right edge.
const AGENT_LAUNCHER_WIDTH: f32 = 72.;
/// Square size of one host chrome control in the utility row's trailing
/// capsule; the capsule adds `px_1` around them and 2 px between them.
const CHROME_CONTROL_SIZE: f32 = 28.;

/// Utility-row width the full row needs, excluding any host chrome capsule:
/// the three-tile mode tray, the − / + steppers, the percent trigger, and
/// the explicit AI action with their dividers, gaps, and row padding. Below
/// it the zoom cluster sheds its steppers (50 px) so the dock reflows before
/// its utility segment has to scroll. The dock is intrinsic, so an
/// unconstrained host always meets this; only a host narrower than the dock's
/// natural width triggers the reflow.
pub const TOOLBAR_ZOOM_STEPPERS_MIN_WIDTH: f32 = 317.;
/// Utility-row width the percent-only row needs, excluding any host chrome
/// capsule. Below it the zoom cluster collapses entirely, leaving the mode
/// tray and explicit AI action (192 px). Zoom stays reachable through the
/// Actions palette, the shift-zoom shortcuts, and the host; below this the
/// row's overflow scroll is the last resort.
pub const TOOLBAR_ZOOM_CLUSTER_MIN_WIDTH: f32 = 267.;

/// How much of the zoom cluster the utility row presents, derived from the
/// measured dock width so narrow hosts reflow instead of overflowing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ZoomClusterTier {
    /// Steppers, the percent menu trigger, and the flyout.
    Full,
    /// The percent menu trigger and its flyout only.
    PercentOnly,
    /// No zoom cluster at all.
    Hidden,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolbarOverlay {
    ToolGroup(ToolbarToolGroup),
    Actions,
    Agent,
    Zoom,
    /// Anchored candidate menu for one secondary chip, always fed by
    /// host-supplied candidates.
    OptionEditor(ToolbarSecondaryControl),
}

/// Fixed Figma-parity entries in the zoom menu. Zoom in/out step the shared
/// zoom ladder; the remaining entries emit the existing zoom commands and
/// typed percent intents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ZoomMenuEntry {
    ZoomIn,
    ZoomOut,
    ZoomToFit,
    ZoomToSelection,
    ZoomTo100,
    ZoomTo50,
}

impl ZoomMenuEntry {
    const ALL: [Self; 6] = [
        Self::ZoomIn,
        Self::ZoomOut,
        Self::ZoomToFit,
        Self::ZoomToSelection,
        Self::ZoomTo100,
        Self::ZoomTo50,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::ZoomIn => "Zoom in",
            Self::ZoomOut => "Zoom out",
            Self::ZoomToFit => "Zoom to fit",
            Self::ZoomToSelection => "Zoom to selection",
            Self::ZoomTo100 => "Zoom to 100%",
            Self::ZoomTo50 => "Zoom to 50%",
        }
    }

    const fn shortcut(self) -> Option<&'static str> {
        match self {
            Self::ZoomIn => Some("+"),
            Self::ZoomOut => Some("−"),
            Self::ZoomToFit => Some("⇧ 1"),
            Self::ZoomToSelection => Some("⇧ 2"),
            Self::ZoomTo100 => Some("⇧ 0"),
            Self::ZoomTo50 => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum CommandScope {
    #[default]
    All,
    Assets,
    PluginsAndWidgets,
}

impl CommandScope {
    const ALL: [Self; 3] = [Self::All, Self::Assets, Self::PluginsAndWidgets];

    const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Assets => "Assets",
            Self::PluginsAndWidgets => "Plugins & widgets",
        }
    }

    const fn includes(self, command: ToolbarCommand) -> bool {
        match self {
            Self::All => true,
            Self::Assets => matches!(
                command,
                ToolbarCommand::OpenResources | ToolbarCommand::OpenVariables
            ),
            Self::PluginsAndWidgets => matches!(
                command,
                ToolbarCommand::OpenPlugins | ToolbarCommand::OpenWidgets
            ),
        }
    }
}

/// Stateful presentation for a controlled, Figma-like editor toolbar.
///
/// The host owns the active mode, selected tool, zoom, and mode-specific
/// values. The component owns only input drafts, open flyouts, and focus.
pub struct EditorToolbar {
    id: SharedString,
    focus_handle: FocusHandle,
    mode: ToolbarMode,
    active_tool: ToolbarTool,
    zoom_percent: u16,
    dev_options: DevToolbarOptions,
    motion_options: MotionToolbarOptions,
    agent_options: AgentToolbarOptions,
    /// Host chrome rendered in the utility segment's trailing capsule (§12: the
    /// host owns chrome, the toolbar owns the dock).
    chrome_controls: Vec<ToolbarChromeControl>,
    commands: Vec<ToolbarCommand>,
    overlay: Option<ToolbarOverlay>,
    command_scope: CommandScope,
    command_cursor: usize,
    command_scroll_handle: ScrollHandle,
    command_input: Entity<InputState>,
    ai_input: Entity<InputState>,
    menu_cursor: usize,
    menu_scroll_handle: ScrollHandle,
    /// Measured inner width of the dock's utility segment; the zoom cluster
    /// derives its collapse tier from it so the dock reflows on narrow
    /// hosts instead of overflowing.
    utility_width: Option<f32>,
    primary_scroll_handle: ScrollHandle,
    secondary_scroll_handle: ScrollHandle,
    utility_scroll_handle: ScrollHandle,
    primary_fades: EdgeFades,
    secondary_fades: EdgeFades,
    utility_fades: EdgeFades,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ToolbarAction> for EditorToolbar {}

impl EditorToolbar {
    /// Creates a toolbar from the host's current mode, tool, and canvas zoom.
    pub fn new(
        id: impl Into<SharedString>,
        mode: ToolbarMode,
        active_tool: ToolbarTool,
        zoom_percent: u16,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let command_input = cx
            .new(|cx| InputState::new(window, cx).placeholder("Search commands, plugins, and AI…"));
        let ai_input = cx.new(|cx| InputState::new(window, cx).placeholder("Describe your idea"));

        let subscriptions = vec![
            cx.subscribe_in(
                &command_input,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => {
                        this.command_cursor = 0;
                        this.command_scroll_handle.scroll_to_item(0);
                        cx.emit(ToolbarAction::CommandQueryChanged {
                            query: this.command_input.read(cx).value(),
                        });
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => {
                        this.invoke_highlighted_command(window, cx);
                    }
                    InputEvent::Focus | InputEvent::Blur => {}
                },
            ),
            cx.subscribe_in(
                &ai_input,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { .. } => this.submit_ai_prompt(window, cx),
                    InputEvent::Change => cx.notify(),
                    InputEvent::Focus | InputEvent::Blur => {}
                },
            ),
        ];

        Self {
            id: id.into(),
            focus_handle: cx.focus_handle(),
            mode,
            active_tool,
            zoom_percent: zoom_percent.clamp(1, 3_200),
            dev_options: DevToolbarOptions::default(),
            motion_options: MotionToolbarOptions::default(),
            agent_options: AgentToolbarOptions::default(),
            chrome_controls: Vec::new(),
            commands: ToolbarCommand::ALL.to_vec(),
            overlay: None,
            command_scope: CommandScope::default(),
            command_cursor: 0,
            command_scroll_handle: ScrollHandle::new(),
            command_input,
            ai_input,
            menu_cursor: 0,
            menu_scroll_handle: ScrollHandle::new(),
            utility_width: None,
            primary_scroll_handle: ScrollHandle::new(),
            secondary_scroll_handle: ScrollHandle::new(),
            utility_scroll_handle: ScrollHandle::new(),
            primary_fades: EdgeFades::default(),
            secondary_fades: EdgeFades::default(),
            utility_fades: EdgeFades::default(),
            _subscriptions: subscriptions,
        }
    }

    /// The zoom cluster's presentation at the measured dock width. An
    /// unmeasured dock renders in full; the first frame's measurement
    /// schedules the corrective re-render. Host chrome occupies row width
    /// the zoom cluster cannot use, so its capsule is subtracted first. The
    /// half-pixel tolerance keeps a row that fits exactly from shedding on
    /// layout rounding.
    fn zoom_cluster_tier(&self) -> ZoomClusterTier {
        let Some(width) = self.utility_width else {
            return ZoomClusterTier::Full;
        };
        let available = width - self.chrome_cluster_width() + 0.5;
        if available >= TOOLBAR_ZOOM_STEPPERS_MIN_WIDTH {
            ZoomClusterTier::Full
        } else if available >= TOOLBAR_ZOOM_CLUSTER_MIN_WIDTH {
            ZoomClusterTier::PercentOnly
        } else {
            ZoomClusterTier::Hidden
        }
    }

    /// Row width claimed by the host chrome capsule and its separator: the
    /// capsule's `px_1`, its controls and 2 px gaps, plus the 1 px divider
    /// and the row's two 4 px gaps around it. Zero without chrome.
    fn chrome_cluster_width(&self) -> f32 {
        let count = self.chrome_controls.len();
        if count == 0 {
            return 0.;
        }
        8. + count as f32 * CHROME_CONTROL_SIZE + (count as f32 - 1.) * 2. + 1. + 8.
    }
}

impl Focusable for EditorToolbar {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for EditorToolbar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let key_context = if matches!(
            self.overlay,
            Some(ToolbarOverlay::Actions | ToolbarOverlay::Agent)
        ) {
            TOOLBAR_TEXT_ENTRY_KEY_CONTEXT
        } else {
            TOOLBAR_KEY_CONTEXT
        };
        div()
            .id(self.id.clone())
            .debug_selector(|| "editor-toolbar".to_owned())
            .key_context(key_context)
            .track_focus(&self.focus_handle)
            .relative()
            .flex_shrink_1()
            .min_w(px(0.))
            .max_w_full()
            .on_action(cx.listener(|this, _: &CloseToolbarOverlay, window, cx| {
                this.dismiss_overlay(window, cx);
            }))
            .on_action(cx.listener(|this, _: &OpenToolbarActions, window, cx| {
                this.open_actions(window, cx);
            }))
            .on_action(cx.listener(|this, _: &OpenToolbarAgent, window, cx| {
                this.open_agent(window, cx);
            }))
            // The focused Input already emitted PressEnter (which invokes the
            // highlighted command or submits the Agent prompt) before
            // propagating Enter; consuming the propagated action here keeps
            // the keystroke's "\n" key_char out of the single-line field.
            .on_action(cx.listener(|_, _: &ConfirmToolbarTextEntry, _, _| {}))
            .on_action(cx.listener(|this, _: &NextToolbarCommand, _, cx| {
                this.move_command_cursor(1, cx);
            }))
            .on_action(cx.listener(|this, _: &PreviousToolbarCommand, _, cx| {
                this.move_command_cursor(-1, cx);
            }))
            .on_action(cx.listener(|this, _: &FirstToolbarCommand, _, cx| {
                this.move_cursor_to_boundary(false, cx);
            }))
            .on_action(cx.listener(|this, _: &LastToolbarCommand, _, cx| {
                this.move_cursor_to_boundary(true, cx);
            }))
            .on_action(cx.listener(|this, _: &IncrementToolbarControl, _, cx| {
                this.adjust_option_control(1, cx);
            }))
            .on_action(cx.listener(|this, _: &DecrementToolbarControl, _, cx| {
                this.adjust_option_control(-1, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectMoveTool, _, cx| {
                this.request_tool(ToolbarTool::Move, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectHandTool, _, cx| {
                this.request_tool(ToolbarTool::Hand, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectScaleTool, _, cx| {
                this.request_tool(ToolbarTool::Scale, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectFrameTool, _, cx| {
                this.request_tool(ToolbarTool::Frame, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectSectionTool, _, cx| {
                this.request_tool(ToolbarTool::Section, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectSliceTool, _, cx| {
                this.request_tool(ToolbarTool::Slice, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectRectangleTool, _, cx| {
                this.request_tool(ToolbarTool::Rectangle, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectLineTool, _, cx| {
                this.request_tool(ToolbarTool::Line, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectArrowTool, _, cx| {
                this.request_tool(ToolbarTool::Arrow, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectEllipseTool, _, cx| {
                this.request_tool(ToolbarTool::Ellipse, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectImageVideoTool, _, cx| {
                this.request_tool(ToolbarTool::ImageVideo, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectPenTool, _, cx| {
                this.request_tool(ToolbarTool::Pen, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectPencilTool, _, cx| {
                this.request_tool(ToolbarTool::Pencil, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectTextTool, _, cx| {
                this.request_tool(ToolbarTool::Text, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectCommentTool, _, cx| {
                this.request_tool(ToolbarTool::Comment, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectAnnotationTool, _, cx| {
                this.request_tool(ToolbarTool::Annotation, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectMeasureTool, _, cx| {
                this.request_tool(ToolbarTool::Measure, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectResourcesTool, _, cx| {
                this.request_tool(ToolbarTool::Resources, cx);
            }))
            .on_action(cx.listener(|this, _: &ZoomCanvasToFit, _, cx| {
                this.request_zoom_command(ToolbarCommand::ZoomToFit, cx);
            }))
            .on_action(cx.listener(|this, _: &ZoomCanvasToSelection, _, cx| {
                this.request_zoom_command(ToolbarCommand::ZoomToSelection, cx);
            }))
            .on_action(cx.listener(|this, _: &ZoomCanvasTo100, _, cx| {
                this.request_zoom(100, cx);
            }))
            .on_action(cx.listener(|this, _: &EnterDevMode, _, cx| {
                this.request_mode(ToolbarMode::Dev, cx);
            }))
            .child(
                v_flex()
                    .id(SharedString::from(format!("{}-surface", self.id)))
                    .debug_selector(|| "editor-toolbar-surface".to_owned())
                    // The dock floats over the host canvas: every pointer
                    // event on it — clicks, presses, hovers, and wheel — stops
                    // here so a tool press never also reaches the canvas
                    // underneath. The dock's own overflow rows sit above this
                    // hitbox and keep scrolling; the transient popups block
                    // on their own surfaces.
                    .occlude()
                    .relative()
                    .min_w(px(0.))
                    .max_w_full()
                    .p_1()
                    .gap_1()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().popover)
                    .text_color(cx.theme().popover_foreground)
                    .when(cx.theme().shadow, |surface| surface.shadow_sm())
                    .when(self.mode != ToolbarMode::Design, |surface| {
                        surface.child(self.render_secondary_toolbar(window, cx))
                    })
                    .child(
                        h_flex()
                            .debug_selector(|| "toolbar-primary-dock-row".to_owned())
                            .h(px(40.))
                            .max_w_full()
                            .min_w(px(0.))
                            .child(self.render_main_toolbar(window, cx))
                            .child(div().w(px(1.)).h(px(24.)).flex_none().bg(cx.theme().border))
                            .child(self.render_utility_toolbar(window, cx)),
                    ),
            )
    }
}

#[cfg(test)]
mod interaction_tests;
