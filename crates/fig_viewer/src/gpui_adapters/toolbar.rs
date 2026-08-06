//! EditorToolbar adapter: maps `ToolbarTool`/`ToolbarMode`/`ToolbarCommand`
//! intents onto FigView's existing tool, mode, zoom, and edit entry points,
//! and echoes FigView state back into the toolbar entity.

use fanta_gpui::toolbar::{
    AgentToolbarOptions, DevToolbarOptions, EditorToolbar, MotionToolbarOptions, ToolbarCommand,
    ToolbarMode, ToolbarTool,
};
use gpui::{AppContext as _, Context, Entity, SharedString, Subscription, Window};

use crate::editor_session::EditorMode;
use crate::tools::ToolKind;
use crate::view::FigView;

/// The commands the host actually implements today. Passed through
/// `EditorToolbar::set_commands` so the palette never advertises an
/// unwired command.
pub(crate) const IMPLEMENTED_COMMANDS: &[ToolbarCommand] = &[
    ToolbarCommand::Undo,
    ToolbarCommand::Redo,
    ToolbarCommand::Cut,
    ToolbarCommand::Copy,
    ToolbarCommand::Paste,
    ToolbarCommand::Duplicate,
    ToolbarCommand::Delete,
    ToolbarCommand::ZoomToFit,
    ToolbarCommand::ZoomToSelection,
    ToolbarCommand::Present,
    ToolbarCommand::OpenDesignMode,
    ToolbarCommand::OpenMotionMode,
];

/// The entrance presets fig_viewer's Motion inspector can author — the same
/// catalog as `motion_panel.rs`'s `AnimationProperty` preset labels (Position,
/// Scale, Rotation, Size, Opacity). The document has no per-clip style field
/// yet, so this single host-side list feeds both the toolbar's read model and
/// the accepted-style echo; the `Path` preset is excluded because it needs an
/// authored motion path and cannot be applied as a one-click style.
pub(crate) fn motion_animation_styles() -> Vec<SharedString> {
    ["Slide in", "Scale in", "Rotate in", "Grow", "Fade in"]
        .into_iter()
        .map(Into::into)
        .collect()
}

/// Host state snapshot behind the toolbar's mode-specific option read models,
/// assembled by `FigView` on each render and diffed here before any push.
pub(crate) struct ToolbarOptionInputs {
    pub playing: bool,
    pub looping: bool,
    pub current_time_ms: u32,
    /// Duration of the active motion clip; `None` when the document has none.
    pub duration_ms: Option<u32>,
    /// What the contextual Agent composer would act on right now.
    pub agent_context_label: SharedString,
}

/// Total: every canvas tool has a toolbar face.
pub(crate) fn toolbar_tool(kind: ToolKind) -> ToolbarTool {
    match kind {
        ToolKind::Select => ToolbarTool::Move,
        ToolKind::PathSelect => ToolbarTool::PathSelect,
        ToolKind::NodeEdit => ToolbarTool::NodeEdit,
        ToolKind::Hand => ToolbarTool::Hand,
        ToolKind::Scale => ToolbarTool::Scale,
        ToolKind::Rect => ToolbarTool::Rectangle,
        ToolKind::Ellipse => ToolbarTool::Ellipse,
        ToolKind::Line => ToolbarTool::Line,
        ToolKind::Polygon => ToolbarTool::Polygon,
        ToolKind::Star => ToolbarTool::Star,
        ToolKind::Pen => ToolbarTool::Pen,
        ToolKind::Pencil => ToolbarTool::Pencil,
        ToolKind::Frame => ToolbarTool::Frame,
        ToolKind::Section => ToolbarTool::Section,
        ToolKind::Slice => ToolbarTool::Slice,
        ToolKind::Text => ToolbarTool::Text,
        ToolKind::TextPath => ToolbarTool::TextPath,
        ToolKind::Comment => ToolbarTool::Comment,
    }
}

/// Partial: toolbar faces without a canvas tool (Brush, Lasso, Measure, …)
/// are roadmap items and intentionally return `None`. `Resources` also has no
/// canvas tool — `FigView::handle_toolbar_action` intercepts it as host
/// chrome (the left sidebar toggle) before this mapping is consulted.
pub(crate) fn tool_kind(tool: ToolbarTool) -> Option<ToolKind> {
    Some(match tool {
        ToolbarTool::Move => ToolKind::Select,
        ToolbarTool::PathSelect => ToolKind::PathSelect,
        ToolbarTool::NodeEdit => ToolKind::NodeEdit,
        ToolbarTool::Hand => ToolKind::Hand,
        ToolbarTool::Scale => ToolKind::Scale,
        ToolbarTool::Rectangle => ToolKind::Rect,
        ToolbarTool::Ellipse => ToolKind::Ellipse,
        ToolbarTool::Line => ToolKind::Line,
        ToolbarTool::Polygon => ToolKind::Polygon,
        ToolbarTool::Star => ToolKind::Star,
        ToolbarTool::Pen => ToolKind::Pen,
        ToolbarTool::Pencil => ToolKind::Pencil,
        ToolbarTool::Frame => ToolKind::Frame,
        ToolbarTool::Section => ToolKind::Section,
        ToolbarTool::Slice => ToolKind::Slice,
        ToolbarTool::Text => ToolKind::Text,
        ToolbarTool::TextPath => ToolKind::TextPath,
        ToolbarTool::Comment => ToolKind::Comment,
        _ => return None,
    })
}

/// The editor has no Draw or Dev mode; Prototype and Comments keep the
/// Design strip visible.
pub(crate) fn toolbar_mode(mode: EditorMode) -> ToolbarMode {
    match mode {
        EditorMode::Motion => ToolbarMode::Motion,
        EditorMode::Design | EditorMode::Prototype | EditorMode::Comments => ToolbarMode::Design,
    }
}

pub(crate) struct ToolbarAdapter {
    pub panel: Entity<EditorToolbar>,
    /// Last state pushed into the entity; setters always notify, so the
    /// render-time echo diffs here to avoid notify churn.
    last_pushed: (ToolbarMode, ToolbarTool, u16),
    /// Last option models pushed, same diff guard; `None` until first push.
    last_pushed_motion: Option<MotionToolbarOptions>,
    last_pushed_dev: Option<DevToolbarOptions>,
    last_pushed_agent: Option<AgentToolbarOptions>,
    /// The accepted Motion animation style. Host-side UI state held on the
    /// adapter because the document model has no per-clip style field yet;
    /// `ControlChangeRequested` updates it and the render-time refresh echoes
    /// it back through `set_motion_options`.
    animation_style: SharedString,
    /// Test-only: how many option-model pushes reached the panel, so the
    /// echo tests can prove the diff guard instead of counting notifies.
    #[cfg(test)]
    option_pushes: std::cell::Cell<u32>,
    _subscription: Subscription,
}

impl ToolbarAdapter {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<FigView>) -> Self {
        let initial = (ToolbarMode::Design, ToolbarTool::Move, 100);
        let panel = cx.new(|cx| {
            let mut toolbar = EditorToolbar::new(
                "fig-gpui-toolbar",
                initial.0,
                initial.1,
                initial.2,
                window,
                cx,
            );
            toolbar.set_commands(IMPLEMENTED_COMMANDS.iter().copied(), cx);
            toolbar
        });
        let subscription = cx.subscribe_in(&panel, window, FigView::handle_toolbar_action);
        let animation_style = motion_animation_styles()
            .first()
            .cloned()
            .unwrap_or_default();
        Self {
            panel,
            last_pushed: initial,
            last_pushed_motion: None,
            last_pushed_dev: None,
            last_pushed_agent: None,
            animation_style,
            #[cfg(test)]
            option_pushes: std::cell::Cell::new(0),
            _subscription: subscription,
        }
    }

    #[cfg(test)]
    pub(crate) fn option_push_count(&self) -> u32 {
        self.option_pushes.get()
    }

    #[cfg(test)]
    fn record_option_push(&self) {
        self.option_pushes.set(self.option_pushes.get() + 1);
    }

    #[cfg(not(test))]
    fn record_option_push(&self) {}

    /// Accepts a `MotionAnimationStyle` control change when the candidate is
    /// in the host catalog. Returns whether the accepted style changed; the
    /// echo happens on the next render-time [`Self::refresh`].
    pub(crate) fn accept_animation_style(&mut self, style: &SharedString) -> bool {
        if self.animation_style == *style || !motion_animation_styles().contains(style) {
            return false;
        }
        self.animation_style = style.clone();
        true
    }

    #[cfg(test)]
    pub(crate) fn accepted_animation_style(&self) -> &SharedString {
        &self.animation_style
    }

    /// Echo current host state into the toolbar entity, diff-guarded. Called
    /// from FigView's render path so every tool/mode/zoom/options mutation
    /// site is covered by the one choke point.
    pub(crate) fn refresh(
        &mut self,
        mode: EditorMode,
        tool: ToolKind,
        zoom_percent: u16,
        options: ToolbarOptionInputs,
        cx: &mut gpui::App,
    ) {
        let next = (toolbar_mode(mode), toolbar_tool(tool), zoom_percent);
        if next != self.last_pushed {
            let last = self.last_pushed;
            self.last_pushed = next;
            self.panel.update(cx, |toolbar, cx| {
                if next.0 != last.0 {
                    toolbar.set_mode(next.0, cx);
                }
                if next.1 != last.1 {
                    toolbar.set_active_tool(next.1, cx);
                }
                if next.2 != last.2 {
                    toolbar.set_zoom_percent(next.2, cx);
                }
            });
        }

        let motion = MotionToolbarOptions {
            playing: options.playing,
            looping: options.looping,
            // fig_viewer has no keyframe-recording mode; the toggle stays off
            // and its intents are logged in `handle_toolbar_control_change`.
            auto_keyframe: false,
            current_time_ms: options.current_time_ms,
            // No clip means nothing can play: report a zero-length transport
            // instead of inventing a duration.
            duration_ms: options.duration_ms.unwrap_or(0),
            animation_style: self.animation_style.clone(),
            available_animation_styles: motion_animation_styles(),
        };
        if self.last_pushed_motion.as_ref() != Some(&motion) {
            self.record_option_push();
            self.panel.update(cx, |toolbar, cx| {
                toolbar.set_motion_options(motion.clone(), cx);
            });
            self.last_pushed_motion = Some(motion);
        }

        // Truthful static: fig_viewer has no ready-for-development model (and
        // no reachable Dev mode), so the readiness chip stays unset.
        let dev = DevToolbarOptions {
            ready_for_development: false,
        };
        if self.last_pushed_dev != Some(dev) {
            self.record_option_push();
            self.panel.update(cx, |toolbar, cx| {
                toolbar.set_dev_options(dev, cx);
            });
            self.last_pushed_dev = Some(dev);
        }

        // Draw options are intentionally never pushed: `EditorMode` has no
        // Draw/ink surface, `toolbar_mode` never yields `ToolbarMode::Draw`,
        // and feeding a fake stroke model would advertise a mode the host
        // cannot enter.

        let agent = agent_options(options.agent_context_label);
        if self.last_pushed_agent.as_ref() != Some(&agent) {
            self.record_option_push();
            self.panel.update(cx, |toolbar, cx| {
                toolbar.set_agent_options(agent.clone(), cx);
            });
            self.last_pushed_agent = Some(agent);
        }
    }
}

/// Host copy for the contextual Agent composer: the context chip names what
/// the prompt would act on, and the suggestions stay within what fig_viewer's
/// agent design tools (read, edit, screenshot the active canvas) could do.
fn agent_options(context_label: SharedString) -> AgentToolbarOptions {
    AgentToolbarOptions::new(context_label)
        .mention_hint("Describe a change to the current selection")
        .suggestions([
            "Rename the selected layers",
            "Suggest a color palette",
            "Describe this design",
        ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_kind_round_trips_through_the_toolbar() {
        for kind in [
            ToolKind::Select,
            ToolKind::PathSelect,
            ToolKind::NodeEdit,
            ToolKind::Hand,
            ToolKind::Scale,
            ToolKind::Rect,
            ToolKind::Ellipse,
            ToolKind::Line,
            ToolKind::Polygon,
            ToolKind::Star,
            ToolKind::Pen,
            ToolKind::Pencil,
            ToolKind::Frame,
            ToolKind::Section,
            ToolKind::Slice,
            ToolKind::Text,
            ToolKind::TextPath,
            ToolKind::Comment,
        ] {
            assert_eq!(
                tool_kind(toolbar_tool(kind)),
                Some(kind),
                "{kind:?} must survive the round trip"
            );
        }
    }

    /// The exact set of toolbar faces with no canvas tool today. A new
    /// `ToolbarTool` variant lands here on purpose or gets a mapping —
    /// never silently.
    #[test]
    fn unmapped_toolbar_tools_are_exactly_the_roadmap_set() {
        let unmapped: Vec<ToolbarTool> = ToolbarTool::ALL
            .iter()
            .copied()
            .filter(|tool| tool_kind(*tool).is_none())
            .collect();
        assert_eq!(
            unmapped,
            vec![
                ToolbarTool::Arrow,
                ToolbarTool::ImageVideo,
                ToolbarTool::Annotation,
                ToolbarTool::Measure,
                ToolbarTool::Resources,
                ToolbarTool::Actions,
                ToolbarTool::Brush,
                ToolbarTool::PaintBucket,
                ToolbarTool::ShapeBuilder,
                ToolbarTool::Lasso,
                ToolbarTool::VariableWidth,
                ToolbarTool::Inspect,
                ToolbarTool::ColorPicker,
                ToolbarTool::Code,
                ToolbarTool::Variables,
                ToolbarTool::ReadyForDev,
                ToolbarTool::MotionSelect,
                ToolbarTool::AddKeyframe,
                ToolbarTool::MotionPath,
                ToolbarTool::AnimationStyle,
                ToolbarTool::TimeComment,
                ToolbarTool::AutoKeyframe,
                ToolbarTool::PlayPreview,
            ]
        );
    }

    #[test]
    fn every_editor_mode_maps_to_a_toolbar_mode() {
        assert_eq!(toolbar_mode(EditorMode::Design), ToolbarMode::Design);
        assert_eq!(toolbar_mode(EditorMode::Motion), ToolbarMode::Motion);
        assert_eq!(toolbar_mode(EditorMode::Prototype), ToolbarMode::Design);
        assert_eq!(toolbar_mode(EditorMode::Comments), ToolbarMode::Design);
    }

    /// Both zoom commands the component's zoom menu emits are advertised, so
    /// the Actions palette and the menu agree on what the host implements.
    #[test]
    fn the_zoom_command_family_is_advertised() {
        assert!(IMPLEMENTED_COMMANDS.contains(&ToolbarCommand::ZoomToFit));
        assert!(IMPLEMENTED_COMMANDS.contains(&ToolbarCommand::ZoomToSelection));
    }

    /// The style catalog mirrors motion_panel.rs's entrance presets (minus
    /// Path, which needs an authored motion path). A new preset lands in both
    /// places or this fails.
    #[test]
    fn animation_style_catalog_mirrors_the_motion_panel_presets() {
        assert_eq!(
            motion_animation_styles(),
            vec![
                gpui::SharedString::from("Slide in"),
                "Scale in".into(),
                "Rotate in".into(),
                "Grow".into(),
                "Fade in".into(),
            ]
        );
    }
}

#[cfg(test)]
mod echo_tests {
    use std::path::PathBuf;

    use fanta_doc::{
        AnimationClip, AnimationClipId, CanvasNode, Doc, GroupNode, NodeData, Operation, TextNode,
        Transform2D, Viewport,
    };
    use fanta_gpui::toolbar::{ToolbarAction, ToolbarControlValue, ToolbarSecondaryControl};
    use gpui::{Bounds, TestAppContext, VisualTestContext, point, px, size};
    use project::{FakeFs, Project};

    use super::*;
    use crate::view::FigView;

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            zlog::init_test();
            assets::Assets.load_test_fonts(cx);
            let store = settings::SettingsStore::test(cx);
            cx.set_global(store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            release_channel::init(semver::Version::new(0, 0, 0), cx);
            editor::init(cx);
            gpui_component::init(cx);
            fanta_gpui::init(cx);
            crate::theme_bridge::init(cx);
        });
    }

    /// One page holding two text layers — one at the origin, one offset, so
    /// fit-to-page and fit-to-selection land on different centers. The offset
    /// layer starts selected and the document carries one 1.5 s motion clip.
    fn test_doc() -> Doc {
        let mut doc = Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "Page 1".to_owned();
        let page_root = page.id;
        doc.apply(Operation::create_node(page)).expect("create page");
        doc.add_page(page_root);
        doc.set_active_page(Some(page_root));

        let mut origin_text = CanvasNode::new(NodeData::Text(TextNode::new("Origin", 120.0, 40.0)));
        origin_text.parent = Some(page_root);
        doc.apply(Operation::create_node(origin_text))
            .expect("create origin layer");

        let mut target = CanvasNode::new(NodeData::Text(TextNode::new("Target", 120.0, 40.0)));
        target.name = "Target".to_owned();
        target.parent = Some(page_root);
        target.transform = Transform2D::translation(400.0, 300.0);
        let target_id = target.id;
        doc.apply(Operation::create_node(target))
            .expect("create target layer");

        let clip_id = AnimationClipId::from_u128(7);
        doc.motion
            .clips
            .insert(clip_id, AnimationClip::new(clip_id, "Entrance", 1_500));
        doc.selection.select_only(target_id);
        doc.history = Default::default();
        doc
    }

    async fn setup(
        cx: &mut TestAppContext,
    ) -> (Entity<FigView>, Entity<EditorToolbar>, VisualTestContext) {
        init_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let item = crate::document::ready_item_for_test(
            &project,
            PathBuf::from("/tmp/Toolbar.fig"),
            test_doc(),
            cx,
        );
        let (view, cx) =
            cx.add_window_view(move |window, cx| FigView::new(item, project, window, cx));
        cx.run_until_parked();
        let toolbar = view.read_with(cx, |view, _| {
            view.gpui_toolbar_adapter()
                .expect("the toolbar adapter should mount in a themed window")
                .panel
                .clone()
        });
        let cx = cx.clone();
        (view, toolbar, cx)
    }

    #[gpui::test]
    async fn option_models_reach_the_panel_and_pushes_are_diff_guarded(cx: &mut TestAppContext) {
        let (view, toolbar, mut cx) = setup(cx).await;
        let cx = &mut cx;

        // Mount-time push: truthful Motion/Dev/Agent read models.
        toolbar.read_with(cx, |toolbar, _| {
            assert!(!toolbar.dev_options().ready_for_development);
            assert_eq!(toolbar.agent_options().context_label, "Target");
            let motion = toolbar.motion_options();
            assert!(!motion.playing);
            assert!(!motion.looping);
            assert!(!motion.auto_keyframe);
            assert_eq!(motion.available_animation_styles, motion_animation_styles());
            assert_eq!(motion.animation_style, "Slide in");
            // No active clip yet: a zero-length transport, not an invented one.
            assert_eq!(motion.duration_ms, 0);
        });

        // Entering Motion mode adopts the document's clip; its real duration
        // reaches the transport read model.
        view.update_in(cx, |view, _, cx| {
            view.set_editor_mode(crate::editor_session::EditorMode::Motion, cx)
        });
        cx.run_until_parked();
        toolbar.read_with(cx, |toolbar, _| {
            assert_eq!(toolbar.mode(), ToolbarMode::Motion);
            assert_eq!(toolbar.motion_options().duration_ms, 1_500);
        });

        // Diff guard: host renders without state changes push nothing.
        let baseline = view.read_with(cx, |view, _| {
            view.gpui_toolbar_adapter()
                .expect("adapter")
                .option_push_count()
        });
        view.update_in(cx, |_, _, cx| cx.notify());
        cx.run_until_parked();
        view.update_in(cx, |_, _, cx| cx.notify());
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |view, _| {
                view.gpui_toolbar_adapter()
                    .expect("adapter")
                    .option_push_count()
            }),
            baseline,
            "no-change renders must not push options into the panel"
        );

        // The loop chip's accepted value lands on the timeline and echoes
        // back through exactly one options push.
        toolbar.update_in(cx, |_, _, cx| {
            cx.emit(ToolbarAction::ControlChangeRequested {
                mode: ToolbarMode::Motion,
                control: ToolbarSecondaryControl::MotionLoop,
                value: ToolbarControlValue::Toggle(true),
            });
        });
        cx.run_until_parked();
        assert!(
            toolbar.read_with(cx, |toolbar, _| toolbar.motion_options().looping),
            "the accepted loop value must round-trip into the echoed options"
        );
        assert_eq!(
            view.read_with(cx, |view, _| {
                view.gpui_toolbar_adapter()
                    .expect("adapter")
                    .option_push_count()
            }),
            baseline + 1,
            "one change, one push"
        );
    }

    #[gpui::test]
    async fn animation_style_choice_round_trips_and_unknown_candidates_are_refused(
        cx: &mut TestAppContext,
    ) {
        let (view, toolbar, mut cx) = setup(cx).await;
        let cx = &mut cx;

        toolbar.update_in(cx, |_, _, cx| {
            cx.emit(ToolbarAction::ControlChangeRequested {
                mode: ToolbarMode::Motion,
                control: ToolbarSecondaryControl::MotionAnimationStyle,
                value: ToolbarControlValue::Choice("Fade in".into()),
            });
        });
        cx.run_until_parked();
        toolbar.read_with(cx, |toolbar, _| {
            assert_eq!(toolbar.motion_options().animation_style, "Fade in");
        });
        view.read_with(cx, |view, _| {
            assert_eq!(
                view.gpui_toolbar_adapter()
                    .expect("adapter")
                    .accepted_animation_style(),
                &SharedString::from("Fade in"),
            );
        });

        // A candidate outside the host catalog is refused, not echoed.
        toolbar.update_in(cx, |_, _, cx| {
            cx.emit(ToolbarAction::ControlChangeRequested {
                mode: ToolbarMode::Motion,
                control: ToolbarSecondaryControl::MotionAnimationStyle,
                value: ToolbarControlValue::Choice("Bounce".into()),
            });
        });
        cx.run_until_parked();
        toolbar.read_with(cx, |toolbar, _| {
            assert_eq!(toolbar.motion_options().animation_style, "Fade in");
        });
    }

    #[gpui::test]
    async fn zoom_commands_apply_through_the_canvas_zoom_path(cx: &mut TestAppContext) {
        let (view, toolbar, mut cx) = setup(cx).await;
        let cx = &mut cx;

        view.update_in(cx, |view, _, cx| {
            view.set_viewport_silent(Viewport {
                center: [0.0, 0.0],
                zoom: 1.0,
            });
            view.set_container_bounds(Bounds::new(
                point(px(0.), px(0.)),
                size(px(800.), px(600.)),
            ));
            cx.notify();
        });
        cx.run_until_parked();

        // The +/- steppers and the zoom menu's percent entries arrive as
        // ZoomChangeRequested; ZoomCanvasTo100 arrives the same way.
        toolbar.update_in(cx, |_, _, cx| {
            cx.emit(ToolbarAction::ZoomChangeRequested { percent: 200 });
        });
        cx.run_until_parked();
        let zoom = view.read_with(cx, |view, _| view.zoom_percent());
        assert!((zoom - 200.0).abs() < 1e-6, "expected 200%, got {zoom}");
        assert_eq!(
            toolbar.read_with(cx, |toolbar, _| toolbar.zoom_percent()),
            200,
            "the applied zoom must echo into the toolbar display"
        );

        toolbar.update_in(cx, |_, _, cx| {
            cx.emit(ToolbarAction::ZoomChangeRequested { percent: 100 });
        });
        cx.run_until_parked();
        let zoom = view.read_with(cx, |view, _| view.zoom_percent());
        assert!(
            (zoom - 100.0).abs() < 1e-6,
            "the 100% entry must land exactly, got {zoom}"
        );

        // Zoom to selection frames the selected offset layer (120×40 at
        // 400,300 → center 460,320), not the page.
        toolbar.update_in(cx, |_, _, cx| {
            cx.emit(ToolbarAction::CommandInvoked {
                command: ToolbarCommand::ZoomToSelection,
            });
        });
        cx.run_until_parked();
        let viewport = view.read_with(cx, |view, _| view.viewport().expect("viewport"));
        assert!(
            (viewport.center[0] - 460.0).abs() < 1.0 && (viewport.center[1] - 320.0).abs() < 1.0,
            "zoom-to-selection must center the selection, got {:?}",
            viewport.center
        );
        assert!(viewport.zoom > 1.0);

        // Zoom to fit frames the whole page: both layers' union
        // (0,0)–(520,340) → center 260,170.
        toolbar.update_in(cx, |_, _, cx| {
            cx.emit(ToolbarAction::CommandInvoked {
                command: ToolbarCommand::ZoomToFit,
            });
        });
        cx.run_until_parked();
        let viewport = view.read_with(cx, |view, _| view.viewport().expect("viewport"));
        assert!(
            (viewport.center[0] - 260.0).abs() < 1.0 && (viewport.center[1] - 170.0).abs() < 1.0,
            "zoom-to-fit must center the page contents, got {:?}",
            viewport.center
        );
    }
}
