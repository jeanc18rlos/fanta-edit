//! EditorToolbar adapter: maps `ToolbarTool`/`ToolbarMode`/`ToolbarCommand`
//! intents onto FigView's existing tool, mode, zoom, and edit entry points,
//! and echoes FigView state back into the toolbar entity.

use fanta_gpui::toolbar::{
    AgentToolbarOptions, DevToolbarOptions, EditorToolbar, MotionToolbarOptions,
    ToolbarChromeControl, ToolbarCommand, ToolbarMode, ToolbarTool,
};
use gpui::{AppContext as _, Context, Entity, SharedString, Subscription, Window};
use gpui_component::IconName;

use crate::editor_session::EditorMode;
use crate::tools::ToolKind;
use crate::view::FigView;

/// Chrome control ids the host pushes through `set_chrome_controls`; the
/// toolbar echoes them verbatim in `ToolbarAction::ChromeControlInvoked`.
pub(crate) const CHROME_FIT_TO_VIEW: &str = "fit-to-view";
pub(crate) const CHROME_TOGGLE_LAYERS_SIDEBAR: &str = "toggle-layers-sidebar";
pub(crate) const CHROME_TOGGLE_INSPECTOR_SIDEBAR: &str = "toggle-inspector-sidebar";

/// The commands the host actually implements today. Passed through
/// `EditorToolbar::set_commands` so the palette never advertises an
/// unwired command. Every entry must have its own arm in
/// `FigView::handle_toolbar_action`; anything that reaches that match's
/// fallthrough is answered with a "not in the alpha" notice instead of a
/// silent no-op, so an accidental addition here is visible rather than dead.
pub(crate) const IMPLEMENTED_COMMANDS: &[ToolbarCommand] = &[
    ToolbarCommand::Undo,
    ToolbarCommand::Redo,
    ToolbarCommand::Cut,
    ToolbarCommand::Copy,
    ToolbarCommand::Paste,
    ToolbarCommand::Duplicate,
    ToolbarCommand::Delete,
    ToolbarCommand::SelectAll,
    ToolbarCommand::Group,
    ToolbarCommand::Ungroup,
    ToolbarCommand::FrameSelection,
    ToolbarCommand::ZoomToFit,
    ToolbarCommand::ZoomToSelection,
    ToolbarCommand::Export,
    ToolbarCommand::Present,
    ToolbarCommand::OpenDesignMode,
    ToolbarCommand::OpenMotionMode,
    ToolbarCommand::GenerateImage,
    ToolbarCommand::GenerateVideo,
    ToolbarCommand::GenerateVector,
    ToolbarCommand::GenerateDesign,
    ToolbarCommand::GenerateMasks,
    ToolbarCommand::ReplaceContent,
    ToolbarCommand::RewriteText,
    ToolbarCommand::TranslateText,
    ToolbarCommand::RenameLayers,
    ToolbarCommand::RemoveBackground,
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
    /// Whether the left (pages/layers) sidebar is shown; drives the chrome
    /// toggle's icon, label, and active state.
    pub layers_sidebar_visible: bool,
    /// Whether the right (inspector) sidebar is shown; same role as above.
    pub inspector_sidebar_visible: bool,
    /// The live keymap text for `FitToView` (e.g. "⇧1"), or `None` when the
    /// action is unbound in the current window.
    pub fit_to_view_shortcut: Option<SharedString>,
}

/// The dock's trailing chrome capsule: fit-to-view plus the two sidebar
/// toggles, exactly the host chrome the pre-capsule trailing cluster held.
/// The Panel icons depict the action (`*Close` while visible, `*Open` while
/// hidden) and `active` mirrors visibility, so the raised tile reads
/// "sidebar is showing".
pub(crate) fn chrome_controls(
    layers_sidebar_visible: bool,
    inspector_sidebar_visible: bool,
    fit_to_view_shortcut: Option<SharedString>,
) -> Vec<ToolbarChromeControl> {
    let mut fit = ToolbarChromeControl::new(CHROME_FIT_TO_VIEW, IconName::Maximize, "Fit to View");
    if let Some(shortcut) = fit_to_view_shortcut {
        fit = fit.shortcut(shortcut);
    }
    vec![
        fit,
        ToolbarChromeControl::new(
            CHROME_TOGGLE_LAYERS_SIDEBAR,
            if layers_sidebar_visible {
                IconName::PanelLeftClose
            } else {
                IconName::PanelLeftOpen
            },
            if layers_sidebar_visible {
                "Hide Layers"
            } else {
                "Show Layers"
            },
        )
        .active(layers_sidebar_visible),
        ToolbarChromeControl::new(
            CHROME_TOGGLE_INSPECTOR_SIDEBAR,
            if inspector_sidebar_visible {
                IconName::PanelRightClose
            } else {
                IconName::PanelRightOpen
            },
            if inspector_sidebar_visible {
                "Hide Inspector"
            } else {
                "Show Inspector"
            },
        )
        .active(inspector_sidebar_visible),
    ]
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

/// Partial: toolbar faces without a canvas tool (Arrow, Measure, Dev and
/// Motion faces, …) are roadmap items and intentionally return `None`.
/// `Resources` also has no canvas tool — `FigView::handle_toolbar_action`
/// intercepts it as host chrome (reveal and focus the left sidebar) before
/// this mapping is consulted.
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

/// The editor has no Dev mode; Prototype and Comments keep the Design strip
/// visible.
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
    last_pushed_chrome: Option<Vec<ToolbarChromeControl>>,
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
            last_pushed_chrome: None,
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

        let agent = agent_options(options.agent_context_label);
        if self.last_pushed_agent.as_ref() != Some(&agent) {
            self.record_option_push();
            self.panel.update(cx, |toolbar, cx| {
                toolbar.set_agent_options(agent.clone(), cx);
            });
            self.last_pushed_agent = Some(agent);
        }

        // Host chrome (§12): the toolbar renders the capsule, the host owns
        // the state; a sidebar toggle round-trips as intent → host flip →
        // this echo, which flips the tile's icon and raised state.
        let chrome = chrome_controls(
            options.layers_sidebar_visible,
            options.inspector_sidebar_visible,
            options.fit_to_view_shortcut,
        );
        if self.last_pushed_chrome.as_ref() != Some(&chrome) {
            self.record_option_push();
            self.panel.update(cx, |toolbar, cx| {
                toolbar.set_chrome_controls(chrome.iter().cloned(), cx);
            });
            self.last_pushed_chrome = Some(chrome);
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

    /// The chrome capsule carries exactly the three controls the retired
    /// trailing cluster held, in the same order, with the Panel icons
    /// depicting the action and `active` mirroring visibility.
    #[test]
    fn chrome_controls_mirror_sidebar_visibility() {
        use fanta_gpui::toolbar::ToolbarChromeControl;
        use gpui_component::IconNamed as _;

        let shown = chrome_controls(true, true, Some("⇧1".into()));
        assert_eq!(
            shown.iter().map(|c| c.id.as_ref()).collect::<Vec<_>>(),
            [
                CHROME_FIT_TO_VIEW,
                CHROME_TOGGLE_LAYERS_SIDEBAR,
                CHROME_TOGGLE_INSPECTOR_SIDEBAR
            ]
        );
        assert_eq!(
            shown[0],
            ToolbarChromeControl::new(CHROME_FIT_TO_VIEW, IconName::Maximize, "Fit to View")
                .shortcut("⇧1")
        );
        assert_eq!(shown[1].icon_path(), IconName::PanelLeftClose.path());
        assert_eq!(shown[1].label, "Hide Layers");
        assert!(shown[1].active);
        assert_eq!(shown[2].icon_path(), IconName::PanelRightClose.path());
        assert_eq!(shown[2].label, "Hide Inspector");
        assert!(shown[2].active);

        let hidden = chrome_controls(false, false, None);
        assert_eq!(hidden[0].shortcut, None);
        assert_eq!(hidden[1].icon_path(), IconName::PanelLeftOpen.path());
        assert_eq!(hidden[1].label, "Show Layers");
        assert!(!hidden[1].active);
        assert_eq!(hidden[2].icon_path(), IconName::PanelRightOpen.path());
        assert_eq!(hidden[2].label, "Show Inspector");
        assert!(!hidden[2].active);
    }

    /// The app's asset source is Zed's `assets::Assets` (embedded
    /// `assets/icons/**`), not gpui-component-assets. Every Lucide icon the
    /// toolbar can name — its own tool/mode faces and the host's chrome
    /// controls — must resolve there or the tile renders blank in the app.
    /// The vendored `IconName::path` table is the exhaustive list of what
    /// `gpui_component::IconName` can name, so scanning it covers the
    /// toolbar's internal choices without pinning them here.
    #[test]
    fn every_gpui_component_icon_resolves_in_the_app_asset_source() {
        use gpui::AssetSource as _;

        let icon_table = include_str!("../../../gpui_component/src/icon.rs");
        let paths: Vec<&str> = icon_table
            .match_indices("\"icons/")
            .map(|(start, _)| {
                let rest = &icon_table[start + 1..];
                let end = rest.find('"').expect("closing quote");
                &rest[..end]
            })
            .collect();
        assert!(
            paths.len() >= 80,
            "the IconName table should list the Lucide set, found {}",
            paths.len()
        );
        for control in chrome_controls(true, false, None) {
            let path = control.icon_path();
            assert!(
                paths.contains(&path.as_ref()),
                "{path} must come from the IconName table"
            );
        }
        for path in paths {
            let bytes = assets::Assets
                .load(path)
                .unwrap_or_else(|error| panic!("{path}: {error:#}"))
                .unwrap_or_else(|| panic!("{path} is missing from assets/icons"));
            assert!(
                std::str::from_utf8(&bytes).is_ok_and(|svg| svg.contains("<svg")),
                "{path} must be an SVG document"
            );
        }
    }

    /// Both zoom commands the component's zoom menu emits are advertised, so
    /// the Actions palette and the menu agree on what the host implements.
    /// Select all rides along: the palette prints its ⌘A shortcut, so the
    /// command has to be advertised or the palette shows a key that works
    /// while the entry claiming it does not exist.
    #[test]
    fn the_zoom_command_family_is_advertised() {
        assert!(IMPLEMENTED_COMMANDS.contains(&ToolbarCommand::ZoomToFit));
        assert!(IMPLEMENTED_COMMANDS.contains(&ToolbarCommand::ZoomToSelection));
        assert!(IMPLEMENTED_COMMANDS.contains(&ToolbarCommand::SelectAll));
    }

    #[test]
    fn implemented_commands_are_unique_and_include_existing_structure_and_text_ai_actions() {
        let unique: std::collections::HashSet<_> = IMPLEMENTED_COMMANDS.iter().copied().collect();
        assert_eq!(
            unique.len(),
            IMPLEMENTED_COMMANDS.len(),
            "the host command registry must not contain duplicate actions"
        );

        for command in [
            ToolbarCommand::Group,
            ToolbarCommand::Ungroup,
            ToolbarCommand::FrameSelection,
            ToolbarCommand::ReplaceContent,
            ToolbarCommand::RewriteText,
            ToolbarCommand::TranslateText,
            ToolbarCommand::RenameLayers,
        ] {
            assert_eq!(
                IMPLEMENTED_COMMANDS
                    .iter()
                    .filter(|registered| **registered == command)
                    .count(),
                1,
                "{command:?} must be advertised exactly once"
            );
        }
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
    use std::{cell::RefCell, path::PathBuf, rc::Rc};

    use fanta_doc::{
        AnimationClip, AnimationClipId, CanvasNode, Doc, GroupNode, NodeData, Operation, TextNode,
        Transform2D, Viewport,
    };
    use fanta_gpui::toolbar::{ToolbarAction, ToolbarControlValue, ToolbarSecondaryControl};
    use gpui::{Bounds, Modifiers, TestAppContext, VisualTestContext, point, px, size};
    use gpui_component::IconNamed as _;
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
        doc.apply(Operation::create_node(page))
            .expect("create page");
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
    async fn path_selection_toolbar_drag_edits_segment_endpoints_and_undoes(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let mut doc = test_doc();
        let mut original = fanta_doc::PathData::new();
        original
            .move_to(0., 0.)
            .line_to(100., 0.)
            .line_to(100., 100.);
        let mut vector = CanvasNode::new(NodeData::Vector(fanta_doc::VectorNode {
            path: original.clone(),
            fills: Default::default(),
            strokes: [fanta_doc::Stroke::solid(fanta_doc::Color::BLACK, 2.)]
                .into_iter()
                .collect(),
            corner_radius: None,
            corner_radii: None,
            corner_smoothing: 0.,
            local_size: None,
            parametric: None,
        }));
        vector.parent = doc.active_page();
        vector.transform = Transform2D::translation(-100., 100.);
        let vector_id = vector.id;
        doc.apply(Operation::create_node(vector))
            .expect("create editable path");
        doc.selection.clear();
        doc.history = Default::default();
        let item = crate::document::ready_item_for_test(
            &project,
            PathBuf::from("/tmp/Path-selection.fig"),
            doc,
            cx,
        );
        let (view, cx) = cx.add_window_view({
            let item = item.clone();
            move |window, cx| FigView::new(item, project, window, cx)
        });
        cx.simulate_resize(size(px(1200.), px(900.)));
        cx.run_until_parked();
        let toolbar = view.read_with(cx, |view, _| {
            view.gpui_toolbar_adapter().expect("toolbar").panel.clone()
        });
        toolbar.update(cx, |_, cx| {
            cx.emit(ToolbarAction::ToolChangeRequested {
                mode: ToolbarMode::Design,
                tool: ToolbarTool::PathSelect,
            });
        });
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |view, _| view.active_tool()),
            ToolKind::PathSelect
        );
        view.update(cx, |view, cx| {
            view.set_viewport_silent(Viewport {
                center: [-50., 150.],
                zoom: 1.,
            });
            cx.notify();
        });
        cx.run_until_parked();
        let bounds = view.read_with(cx, |view, _| view.container_bounds.expect("canvas bounds"));
        let start = bounds.center() + point(px(0.), px(-50.));
        let end = start + point(px(20.), px(30.));
        cx.simulate_mouse_down(start, gpui::MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(end, gpui::MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(end, gpui::MouseButton::Left, Modifiers::none());
        cx.run_until_parked();
        let edited = item.read_with(cx, |item, _| {
            let doc = item.doc().expect("document");
            assert!(doc.selection.contains(vector_id));
            assert_eq!(doc.history.undo_depth(), 1);
            let path = &doc
                .scene
                .get(vector_id)
                .expect("vector")
                .data
                .as_vector()
                .expect("path")
                .path;
            let mut expected = fanta_doc::PathData::new();
            expected
                .move_to(20., 30.)
                .line_to(120., 30.)
                .line_to(100., 100.);
            assert_eq!(path, &expected);
            path.clone()
        });
        for (command, expected) in [
            (ToolbarCommand::Undo, original),
            (ToolbarCommand::Redo, edited),
        ] {
            toolbar.update(cx, |_, cx| {
                cx.emit(ToolbarAction::CommandInvoked { command })
            });
            cx.run_until_parked();
            item.read_with(cx, |item, _| {
                let actual = &item
                    .doc()
                    .expect("document")
                    .scene
                    .get(vector_id)
                    .expect("vector")
                    .data
                    .as_vector()
                    .expect("path")
                    .path;
                assert_eq!(actual, &expected);
            });
        }
    }

    #[gpui::test]
    async fn toolbar_scale_resizes_text_contents_with_undo_redo_and_escape(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let mut doc = Doc::new();
        let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let page_id = page.id;
        doc.apply(Operation::create_node(page))
            .expect("create page");
        doc.add_page(page_id);
        doc.set_active_page(Some(page_id));
        let mut text = TextNode::new("Scale this text", 100., 50.);
        text.style.size_px = 20.;
        let mut layer = CanvasNode::new(NodeData::Text(text));
        layer.parent = Some(page_id);
        let text_id = layer.id;
        doc.apply(Operation::create_node(layer))
            .expect("create text");
        doc.selection.select_only(text_id);
        doc.history = Default::default();
        let item = crate::document::ready_item_for_test(
            &project,
            PathBuf::from("/tmp/Scale-tool.fig"),
            doc,
            cx,
        );
        let (view, cx) = cx.add_window_view({
            let item = item.clone();
            move |window, cx| FigView::new(item, project, window, cx)
        });
        cx.simulate_resize(size(px(1200.), px(900.)));
        cx.run_until_parked();
        let toolbar = view.read_with(cx, |view, _| {
            view.gpui_toolbar_adapter().expect("toolbar").panel.clone()
        });
        toolbar.update(cx, |_, cx| {
            cx.emit(ToolbarAction::ToolChangeRequested {
                mode: ToolbarMode::Design,
                tool: ToolbarTool::Scale,
            });
        });
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |view, _| view.active_tool()),
            ToolKind::Scale
        );
        assert!(!ToolKind::Scale.is_stub());
        view.update(cx, |view, cx| {
            view.set_viewport_silent(Viewport {
                center: [50., 25.],
                zoom: 1.,
            });
            cx.notify();
        });
        cx.run_until_parked();
        let bounds = view.read_with(cx, |view, _| view.container_bounds.expect("canvas bounds"));
        let original_handle = bounds.center() + point(px(50.), px(25.));
        let scaled_handle = original_handle + point(px(100.), px(50.));
        cx.simulate_mouse_down(original_handle, gpui::MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(scaled_handle, gpui::MouseButton::Left, Modifiers::none());
        item.read_with(cx, |item, _| {
            let doc = item.doc().expect("document");
            let NodeData::Text(text) = &doc.scene.get(text_id).expect("text layer").data else {
                panic!("text remains editable text");
            };
            assert_eq!(text.local_size, [200., 100.]);
            assert_eq!(
                text.style.size_px, 40.,
                "Scale changes font size as well as its box"
            );
            assert_eq!(doc.history.undo_depth(), 0);
        });
        cx.simulate_mouse_up(scaled_handle, gpui::MouseButton::Left, Modifiers::none());
        cx.run_until_parked();
        item.read_with(cx, |item, _| {
            assert_eq!(item.doc().expect("document").history.undo_depth(), 1);
        });
        for (command, expected_size, expected_font) in [
            (ToolbarCommand::Undo, [100., 50.], 20.),
            (ToolbarCommand::Redo, [200., 100.], 40.),
        ] {
            toolbar.update(cx, |_, cx| {
                cx.emit(ToolbarAction::CommandInvoked { command })
            });
            cx.run_until_parked();
            item.read_with(cx, |item, _| {
                let doc = item.doc().expect("document");
                let node = doc.scene.get(text_id).expect("text layer");
                let NodeData::Text(text) = &node.data else {
                    panic!("text remains text")
                };
                assert_eq!(text.local_size, expected_size);
                assert_eq!(text.style.size_px, expected_font);
                assert_eq!(node.transform, Transform2D::IDENTITY);
            });
        }
        cx.update(|_, app| {
            let bindings = settings::KeymapFile::load_asset_allow_partial_failure(
                settings::DEFAULT_KEYMAP_PATH,
                app,
            )
            .expect("load shipped keymap");
            app.bind_keys(bindings);
        });
        let enlarged_handle = scaled_handle + point(px(100.), px(50.));
        cx.simulate_mouse_down(scaled_handle, gpui::MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(enlarged_handle, gpui::MouseButton::Left, Modifiers::none());
        item.read_with(cx, |item, _| {
            let NodeData::Text(text) = &item
                .doc()
                .expect("document")
                .scene
                .get(text_id)
                .expect("text")
                .data
            else {
                panic!("text remains text")
            };
            assert_eq!(text.style.size_px, 60.);
        });
        cx.simulate_keystrokes("escape");
        cx.simulate_mouse_up(enlarged_handle, gpui::MouseButton::Left, Modifiers::none());
        item.read_with(cx, |item, _| {
            let doc = item.doc().expect("document");
            let NodeData::Text(text) = &doc.scene.get(text_id).expect("text").data else {
                panic!("text remains text")
            };
            assert_eq!(text.local_size, [200., 100.]);
            assert_eq!(text.style.size_px, 40.);
            assert_eq!(
                doc.history.undo_depth(),
                1,
                "cancelled drag creates no undo step"
            );
        });
    }

    async fn assert_path_edit_viewport_pixels(cx: &mut TestAppContext, delta: f32) {
        init_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let mut doc = Doc::new();
        let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let page_id = page.id;
        doc.apply(Operation::create_node(page))
            .expect("create page");
        doc.add_page(page_id);
        doc.set_active_page(Some(page_id));
        let mut path = fanta_doc::PathData::new();
        path.move_to(0., 160.).quad_to(100., 0., 200., 160.);
        let mut curve = CanvasNode::new(NodeData::Vector(fanta_doc::VectorNode {
            path,
            local_size: Some([200., 160.]),
            strokes: [fanta_doc::Stroke::solid(
                fanta_doc::Color::rgb(46, 125, 255),
                6.,
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        }));
        curve.meta = if delta > 0. {
            serde_json::json!(["opaque", 42])
        } else {
            serde_json::json!("opaque extension")
        };
        curve.parent = Some(page_id);
        curve.transform = Transform2D::translation(60., 60.);
        let curve_id = curve.id;
        let original = curve.clone();
        doc.apply(Operation::create_node(curve))
            .expect("create imported curve");
        doc.selection.select_only(curve_id);
        doc.history = Default::default();
        let item = crate::document::ready_item_for_test(
            &project,
            PathBuf::from("/tmp/Path-viewport.fig"),
            doc,
            cx,
        );
        let (view, cx) = cx.add_window_view({
            let item = item.clone();
            let project = project.clone();
            move |window, cx| FigView::new(item, project, window, cx)
        });
        cx.simulate_resize(size(px(1200.), px(900.)));
        cx.run_until_parked();
        let toolbar = view.read_with(cx, |view, _| {
            view.gpui_toolbar_adapter().expect("toolbar").panel.clone()
        });
        toolbar.update(cx, |_, cx| {
            cx.emit(ToolbarAction::ToolChangeRequested {
                mode: ToolbarMode::Design,
                tool: ToolbarTool::PathSelect,
            });
        });
        cx.run_until_parked();
        view.update(cx, |view, cx| {
            view.set_viewport_silent(Viewport {
                center: [160., 140.],
                zoom: 1.,
            });
            cx.notify();
        });
        cx.run_until_parked();
        let bounds = view.read_with(cx, |view, _| view.container_bounds.expect("canvas bounds"));
        let start = bounds.center();
        let end = start + point(px(delta), px(delta));
        let mut renderer = fanta_render::RasterRenderer::new(400, 400).expect("CPU renderer");
        // At t=0.1, the shifted curve is outside the old viewport: below it for
        // +40 and left of it for -40. A geometry-only assertion misses this clip.
        let samples = if delta > 0. {
            [(120_usize, 231_usize), (298, 260)]
        } else {
            [(40, 151), (218, 180)]
        };
        let assert_pixel = |item: &Entity<crate::document::FigItem>,
                            cx: &VisualTestContext,
                            renderer: &mut fanta_render::RasterRenderer,
                            painted: bool| {
            item.read_with(cx, |item, _| {
                let doc = item.doc().expect("document");
                renderer.render_page(
                    &doc.scene,
                    &Viewport {
                        center: [200., 200.],
                        zoom: 1.,
                    },
                    doc.active_page(),
                );
                let pixels = renderer.copy_rgba();
                for sample in samples {
                    let offset = (sample.1 * 400 + sample.0) * 4;
                    let pixel = pixels.get(offset..offset + 4).expect("sample pixel");
                    let blue = pixel[0] < 100
                        && pixel[1] > 80
                        && pixel[1] < 180
                        && pixel[2] > 220
                        && pixel[3] > 200;
                    assert_eq!(
                        blue, painted,
                        "curve pixel at {sample:?}, delta {delta}: {pixel:?}"
                    );
                }
            });
        };
        assert_pixel(&item, cx, &mut renderer, false);
        cx.simulate_mouse_down(start, gpui::MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(end, gpui::MouseButton::Left, Modifiers::none());
        assert_pixel(&item, cx, &mut renderer, true);
        item.read_with(cx, |item, _| {
            assert_eq!(item.doc().expect("document").history.undo_depth(), 0)
        });
        cx.simulate_mouse_up(end, gpui::MouseButton::Left, Modifiers::none());
        assert_pixel(&item, cx, &mut renderer, true);
        item.read_with(cx, |item, _| {
            let doc = item.doc().expect("document");
            let curve = doc.scene.get(curve_id).expect("curve");
            assert_eq!(curve.transform, original.transform);
            assert_eq!(curve.meta, original.meta);
            assert_eq!(
                curve.flags,
                original.flags | fanta_doc::NodeFlags::UNCLIPPED_VECTOR
            );
            assert_eq!(curve.data.as_vector().expect("vector").local_size, None);
            assert_eq!(doc.history.undo_depth(), 1);
        });
        for (command, painted) in [(ToolbarCommand::Undo, false), (ToolbarCommand::Redo, true)] {
            toolbar.update(cx, |_, cx| {
                cx.emit(ToolbarAction::CommandInvoked { command })
            });
            cx.run_until_parked();
            assert_pixel(&item, cx, &mut renderer, painted);
            if !painted {
                item.read_with(cx, |item, _| {
                    let node = item
                        .doc()
                        .expect("document")
                        .scene
                        .get(curve_id)
                        .expect("restored curve");
                    assert_eq!(node.data, original.data);
                    assert_eq!(node.meta, original.meta);
                    assert_eq!(node.flags, original.flags);
                    assert_eq!(node.transform, original.transform);
                });
            }
        }

        let saved_doc = item.read_with(cx, |item, _| item.doc().expect("saved doc").clone());
        let saved_curve = saved_doc.scene.get(curve_id).expect("saved curve").clone();
        let temporary = tempfile::tempdir().expect("temporary project");
        crate::document::write_project(temporary.path(), &saved_doc, &Default::default())
            .expect("write edited FNX project");
        let (reopened_doc, assets) =
            fanta_format::read_project_tree(temporary.path()).expect("read edited FNX project");
        assert!(assets.is_empty());
        let reopened_item = crate::document::ready_item_for_test(
            &project,
            temporary.path().join("page.fnx"),
            reopened_doc,
            cx,
        );
        reopened_item.read_with(cx, |item, _| {
            let node = item
                .doc()
                .expect("reopened document")
                .scene
                .get(curve_id)
                .expect("reopened curve");
            assert_eq!(
                node.data.as_vector().expect("vector").local_size,
                None,
                "FigDocument load must not backfill an explicitly edited viewport"
            );
            assert_eq!(node.data, saved_curve.data);
            assert_eq!(node.meta, original.meta);
            assert_eq!(node.meta, saved_curve.meta);
            assert_eq!(node.flags, saved_curve.flags);
            assert!(node.flags.contains(fanta_doc::NodeFlags::UNCLIPPED_VECTOR));
            assert_eq!(node.transform, saved_curve.transform);
        });
        assert_pixel(&reopened_item, cx, &mut renderer, true);
    }

    #[gpui::test]
    async fn path_edit_viewport_positive_segment_drag_renders_outside_old_clip(
        cx: &mut TestAppContext,
    ) {
        assert_path_edit_viewport_pixels(cx, 40.).await;
    }

    #[gpui::test]
    async fn path_edit_viewport_negative_segment_drag_renders_outside_old_clip(
        cx: &mut TestAppContext,
    ) {
        assert_path_edit_viewport_pixels(cx, -40.).await;
    }

    #[test]
    fn path_edit_viewport_explicit_flag_overrides_a_remaining_box() {
        let mut doc = Doc::new();
        let mut node = CanvasNode::new(NodeData::Vector(fanta_doc::VectorNode {
            path: fanta_doc::PathData::rect(0., 0., 20., 20.),
            local_size: Some([20., 20.]),
            strokes: [fanta_doc::Stroke::solid(
                fanta_doc::Color::rgb(0, 0, 255),
                16.,
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        }));
        node.flags.insert(fanta_doc::NodeFlags::UNCLIPPED_VECTOR);
        doc.apply(Operation::create_node(node))
            .expect("create flagged SVG viewport");
        let mut renderer = fanta_render::RasterRenderer::new(64, 64).expect("CPU renderer");
        renderer.render(&doc.scene, &doc.viewport);
        let pixels = renderer.copy_rgba();
        let offset = (42 * 64 + 56) * 4;
        let pixel = pixels
            .get(offset..offset + 4)
            .expect("stroke outside viewport");
        assert!(
            pixel[2] > 200 && pixel[3] > 200,
            "explicit flag must override its remaining viewport: {pixel:?}"
        );
    }

    struct CanvasGestureFixture {
        view: Entity<FigView>,
        item: Entity<crate::document::FigItem>,
        toolbar: Entity<EditorToolbar>,
        cx: VisualTestContext,
        vector_id: fanta_doc::NodeId,
        original: fanta_doc::PathData,
    }

    async fn canvas_gesture_fixture(
        cx: &mut TestAppContext,
        tool: ToolbarTool,
    ) -> CanvasGestureFixture {
        init_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let mut doc = Doc::new();
        let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let page_id = page.id;
        doc.apply(Operation::create_node(page))
            .expect("create page");
        doc.add_page(page_id);
        doc.set_active_page(Some(page_id));
        let mut original = fanta_doc::PathData::new();
        original
            .move_to(0., 0.)
            .line_to(100., 0.)
            .line_to(100., 100.)
            .line_to(0., 100.)
            .close();
        let mut vector = CanvasNode::new(NodeData::Vector(fanta_doc::VectorNode {
            path: original.clone(),
            fills: [fanta_doc::Fill::solid(fanta_doc::Color::BLACK)]
                .into_iter()
                .collect(),
            strokes: Default::default(),
            corner_radius: None,
            corner_radii: None,
            corner_smoothing: 0.,
            local_size: None,
            parametric: None,
        }));
        vector.parent = Some(page_id);
        let vector_id = vector.id;
        doc.apply(Operation::create_node(vector))
            .expect("create vector");
        doc.selection.select_only(vector_id);
        doc.history = Default::default();
        let item = crate::document::ready_item_for_test(
            &project,
            PathBuf::from("/tmp/Canvas-gesture.fig"),
            doc,
            cx,
        );
        let (view, cx) = cx.add_window_view({
            let item = item.clone();
            move |window, cx| FigView::new(item, project, window, cx)
        });
        cx.simulate_resize(size(px(1200.), px(900.)));
        cx.run_until_parked();
        let toolbar = view.read_with(cx, |view, _| {
            view.gpui_toolbar_adapter().expect("toolbar").panel.clone()
        });
        toolbar.update(cx, |_, cx| {
            cx.emit(ToolbarAction::ToolChangeRequested {
                mode: ToolbarMode::Design,
                tool,
            });
        });
        view.update(cx, |view, cx| {
            view.set_viewport_silent(Viewport {
                center: [50., 50.],
                zoom: 1.,
            });
            cx.notify();
        });
        cx.run_until_parked();
        CanvasGestureFixture {
            view,
            item,
            toolbar,
            cx: cx.clone(),
            vector_id,
            original,
        }
    }

    async fn assert_path_overlays_follow_history(cx: &mut TestAppContext, tool: ToolbarTool) {
        let CanvasGestureFixture {
            view,
            item,
            toolbar,
            mut cx,
            vector_id,
            ..
        } = canvas_gesture_fixture(cx, tool).await;
        let original = item.update(&mut cx, |item, cx| {
            item.with_document(cx, |document| {
                let node = document
                    .doc
                    .scene
                    .get_mut(vector_id)
                    .expect("curve fixture");
                let vector = node.data.as_vector_mut().expect("vector fixture");
                let mut path = fanta_doc::PathData::new();
                path.move_to(0., 100.).quad_to(50., 0., 100., 100.);
                vector.path = path;
                vector.local_size = Some([100., 100.]);
                node.transform = Transform2D::translation(20., 30.);
                node.meta = serde_json::json!(["preserve", 42]);
                let original = node.clone();
                document.doc.history = Default::default();
                (original, crate::document::DocChange::Content)
            })
            .expect("loaded document")
        });
        view.update(&mut cx, |view, cx| {
            view.set_viewport_silent(Viewport {
                center: [70., 80.],
                zoom: 1.,
            });
            cx.notify();
        });
        cx.update(|_, app| {
            let bindings = settings::KeymapFile::load_asset_allow_partial_failure(
                settings::DEFAULT_KEYMAP_PATH,
                app,
            )
            .expect("shipped key bindings");
            app.bind_keys(bindings);
        });
        cx.run_until_parked();
        let bounds = view.read_with(&cx, |view, _| view.container_bounds.expect("canvas"));
        let start = bounds.center() + point(px(-50.), px(50.));
        let end = start + point(px(20.), px(30.));
        cx.simulate_mouse_down(start, gpui::MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(end, gpui::MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(end, gpui::MouseButton::Left, Modifiers::none());
        let edited = item.read_with(&cx, |item, _| {
            let doc = item.doc().expect("document");
            assert_eq!(doc.history.undo_depth(), 1);
            let node = doc.scene.get(vector_id).expect("edited curve");
            assert_ne!(node.data, original.data);
            node.clone()
        });
        let assert_overlays = |cx: &VisualTestContext| {
            let expected = item.read_with(cx, |item, _| {
                let doc = item.doc().expect("document");
                let node = doc.scene.get(vector_id).expect("curve");
                let anchors = fanta_tools::node_math::enumerate_anchors(
                    &node.data.as_vector().expect("vector").path,
                );
                assert_eq!(anchors.len(), 2);
                let transform = doc
                    .scene
                    .world_transform(vector_id)
                    .expect("world transform");
                let mut expected = Vec::new();
                for anchor in &anchors {
                    for control in [anchor.ctrl_in, anchor.ctrl_out].into_iter().flatten() {
                        expected.push(fanta_tools::ToolOverlay::PathHandle {
                            world_anchor: transform.transform_point(anchor.pos).to_array(),
                            world_ctrl: transform.transform_point(control).to_array(),
                        });
                    }
                }
                for (index, anchor) in anchors.iter().enumerate() {
                    expected.push(fanta_tools::ToolOverlay::PathAnchor {
                        world: transform.transform_point(anchor.pos).to_array(),
                        selected: index == 0,
                    });
                }
                expected
            });
            view.read_with(cx, |view, _| {
                assert_eq!(view.tools().overlays, expected,
                    "every anchor/control overlay must follow document history without pointer input");
            });
        };
        assert_overlays(&cx);
        for from_toolbar in [false, true] {
            for (command, expected_node, expected_depth) in [
                (ToolbarCommand::Undo, &original, 0),
                (ToolbarCommand::Redo, &edited, 1),
            ] {
                if from_toolbar {
                    toolbar.update(&mut cx, |_, cx| {
                        cx.emit(ToolbarAction::CommandInvoked { command });
                    });
                    cx.run_until_parked();
                } else {
                    cx.simulate_keystrokes(match command {
                        ToolbarCommand::Undo => "secondary-z",
                        ToolbarCommand::Redo => "secondary-shift-z",
                        _ => unreachable!(),
                    });
                }
                item.read_with(&cx, |item, _| {
                    let doc = item.doc().expect("document");
                    let node = doc.scene.get(vector_id).expect("restored curve");
                    assert_eq!(node.data, expected_node.data);
                    assert_eq!(node.flags, expected_node.flags);
                    assert_eq!(node.meta, original.meta);
                    assert_eq!(node.transform, original.transform);
                    assert_eq!(doc.history.undo_depth(), expected_depth);
                    assert_eq!(doc.selection.len(), 1);
                    assert!(doc.selection.contains(vector_id));
                });
                assert_overlays(&cx);
            }
        }
    }

    #[gpui::test]
    async fn path_overlays_follow_keyboard_and_toolbar_history_in_path_selection(
        cx: &mut TestAppContext,
    ) {
        assert_path_overlays_follow_history(cx, ToolbarTool::PathSelect).await;
    }

    #[gpui::test]
    async fn path_overlays_follow_keyboard_and_toolbar_history_in_node_edit(
        cx: &mut TestAppContext,
    ) {
        assert_path_overlays_follow_history(cx, ToolbarTool::NodeEdit).await;
    }

    async fn assert_fast_canvas_drag(
        cx: &mut TestAppContext,
        tool: ToolbarTool,
        release_outside: bool,
    ) {
        use gpui::InputEvent as _;

        let CanvasGestureFixture {
            view,
            item,
            toolbar,
            mut cx,
            vector_id,
            original,
        } = canvas_gesture_fixture(cx, tool).await;
        let bounds = view.read_with(&cx, |view, _| view.container_bounds.expect("canvas bounds"));
        let start = if tool == ToolbarTool::PathSelect {
            bounds.center() + point(px(0.), px(-50.))
        } else {
            bounds.center()
        };
        let delta = if release_outside {
            point(bounds.right() - start.x + px(20.), px(30.))
        } else {
            point(px(20.), px(30.))
        };
        let end = start + delta;
        assert_eq!(bounds.contains(&end), !release_outside);
        // simulate_mouse_* pumps the executor after every event, hiding a listener
        // installed only by the repaint following the press.
        cx.update(|window, app| {
            window.dispatch_event(
                gpui::MouseDownEvent {
                    position: start,
                    button: gpui::MouseButton::Left,
                    modifiers: Modifiers::none(),
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                app,
            );
            window.dispatch_event(
                gpui::MouseMoveEvent {
                    position: end,
                    pressed_button: Some(gpui::MouseButton::Left),
                    modifiers: Modifiers::none(),
                }
                .to_platform_input(),
                app,
            );
            window.dispatch_event(
                gpui::MouseUpEvent {
                    position: end,
                    button: gpui::MouseButton::Left,
                    modifiers: Modifiers::none(),
                    click_count: 1,
                }
                .to_platform_input(),
                app,
            );
        });
        cx.run_until_parked();
        let edited = item.read_with(&cx, |item, _| {
            let doc = item.doc().expect("document");
            let node = doc.scene.get(vector_id).expect("dragged vector");
            assert_eq!(
                doc.history.undo_depth(),
                1,
                "one drag must commit one undo entry"
            );
            if tool == ToolbarTool::PathSelect {
                let mut expected = fanta_doc::PathData::new();
                expected
                    .move_to(20., 30.)
                    .line_to(120., 30.)
                    .line_to(100., 100.)
                    .line_to(0., 100.)
                    .close();
                assert_eq!(node.data.as_vector().expect("vector").path, expected);
                assert_eq!(node.transform, Transform2D::IDENTITY);
            } else {
                assert_eq!(node.data.as_vector().expect("vector").path, original);
                assert_eq!(
                    node.transform,
                    Transform2D::translation(
                        f64::from(f32::from(delta.x)),
                        f64::from(f32::from(delta.y))
                    )
                );
            }
            node.clone()
        });
        cx.simulate_mouse_move(
            bounds.center() + point(px(-30.), px(20.)),
            None,
            Modifiers::none(),
        );
        item.read_with(&cx, |item, _| {
            assert_eq!(
                item.doc()
                    .expect("document")
                    .scene
                    .get(vector_id)
                    .expect("vector"),
                &edited,
                "the release must end the gesture, including outside the canvas"
            );
        });
        toolbar.update(&mut cx, |_, cx| {
            cx.emit(ToolbarAction::CommandInvoked {
                command: ToolbarCommand::Undo,
            });
        });
        cx.run_until_parked();
        item.read_with(&cx, |item, _| {
            let node = item
                .doc()
                .expect("document")
                .scene
                .get(vector_id)
                .expect("vector");
            assert_eq!(node.transform, Transform2D::IDENTITY);
            assert_eq!(node.data.as_vector().expect("vector").path, original);
        });
    }

    #[gpui::test]
    async fn fast_drag_without_repaint_moves_selected_layer(cx: &mut TestAppContext) {
        assert_fast_canvas_drag(cx, ToolbarTool::Move, false).await;
    }

    #[gpui::test]
    async fn fast_drag_without_repaint_moves_path_segment(cx: &mut TestAppContext) {
        assert_fast_canvas_drag(cx, ToolbarTool::PathSelect, false).await;
    }

    #[gpui::test]
    async fn fast_drag_without_repaint_releases_outside_canvas(cx: &mut TestAppContext) {
        assert_fast_canvas_drag(cx, ToolbarTool::Move, true).await;
    }

    async fn assert_anchor_delete_uses_editing_tool(cx: &mut TestAppContext, tool: ToolbarTool) {
        let CanvasGestureFixture {
            view,
            item,
            toolbar,
            mut cx,
            vector_id,
            original,
        } = canvas_gesture_fixture(cx, tool).await;
        cx.update(|_, app| {
            let bindings = settings::KeymapFile::load_asset_allow_partial_failure(
                settings::DEFAULT_KEYMAP_PATH,
                app,
            )
            .expect("load the shipped key bindings");
            app.bind_keys(bindings);
        });
        let bounds = view.read_with(&cx, |view, _| view.container_bounds.expect("canvas bounds"));
        let anchor = bounds.center() + point(px(-50.), px(-50.));
        for from_toolbar in [false, true] {
            cx.simulate_click(anchor, Modifiers::none());
            view.read_with(&cx, |view, _| {
                assert_eq!(
                    view.tools()
                        .overlays
                        .iter()
                        .filter(|overlay| matches!(
                            overlay,
                            fanta_tools::ToolOverlay::PathAnchor { selected: true, .. }
                        ))
                        .count(),
                    1,
                    "click must select exactly one anchor"
                );
            });
            if from_toolbar {
                toolbar.update(&mut cx, |_, cx| {
                    cx.emit(ToolbarAction::CommandInvoked {
                        command: ToolbarCommand::Delete,
                    });
                });
                cx.run_until_parked();
            } else {
                cx.simulate_keystrokes("backspace");
            }
            item.read_with(&cx, |item, _| {
                let doc = item.doc().expect("document");
                let node = doc
                    .scene
                    .get(vector_id)
                    .expect("anchor deletion must preserve the layer");
                let anchors = fanta_tools::node_math::enumerate_anchors(
                    &node.data.as_vector().expect("vector").path,
                );
                assert_eq!(anchors.len(), 3);
                assert!(anchors.iter().all(|anchor| anchor.pos != glam::DVec2::ZERO));
                assert_eq!(doc.history.undo_depth(), 1);
            });
            view.read_with(&cx, |view, _| {
                let anchors = view
                    .tools()
                    .overlays
                    .iter()
                    .filter_map(|overlay| match overlay {
                        fanta_tools::ToolOverlay::PathAnchor { world, selected } => {
                            Some((world, selected))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                assert_eq!(
                    anchors.len(),
                    3,
                    "deleted anchor must disappear from overlays"
                );
                assert!(
                    anchors
                        .iter()
                        .all(|(world, selected)| **world != [0., 0.] && !**selected)
                );
            });
            cx.simulate_keystrokes("secondary-z");
            item.read_with(&cx, |item, _| {
                let doc = item.doc().expect("document");
                let node = doc.scene.get(vector_id).expect("restored vector");
                assert_eq!(node.data.as_vector().expect("vector").path, original);
                assert_eq!(doc.history.undo_depth(), 0);
            });
        }
        view.update(&mut cx, |view, cx| view.delete_selected_nodes(cx));
        item.read_with(&cx, |item, _| {
            assert!(
                item.doc().expect("document").scene.get(vector_id).is_none(),
                "explicit layer deletion must retain its object-level meaning"
            );
        });
    }

    #[gpui::test]
    async fn anchor_delete_keymap_and_toolbar_preserve_path_selection_layer(
        cx: &mut TestAppContext,
    ) {
        assert_anchor_delete_uses_editing_tool(cx, ToolbarTool::PathSelect).await;
    }

    #[gpui::test]
    async fn anchor_delete_keymap_and_toolbar_preserve_node_edit_layer(cx: &mut TestAppContext) {
        assert_anchor_delete_uses_editing_tool(cx, ToolbarTool::NodeEdit).await;
    }

    #[gpui::test]
    async fn toolbar_actions_query_keeps_text_input_with_canvas_keybindings(
        cx: &mut TestAppContext,
    ) {
        let (view, toolbar, mut cx) = setup(cx).await;
        let cx = &mut cx;
        let query = Rc::new(RefCell::new(gpui::SharedString::default()));
        let query_changes = query.clone();
        let _subscription = cx.update(|_, app| {
            let bindings = settings::KeymapFile::load_asset_allow_partial_failure(
                settings::DEFAULT_KEYMAP_PATH,
                app,
            )
            .expect("load the app's default key bindings");
            app.bind_keys(bindings);
            app.subscribe(&toolbar, move |_, action: &ToolbarAction, _| {
                if let ToolbarAction::CommandQueryChanged { query } = action {
                    *query_changes.borrow_mut() = query.clone();
                }
            })
        });
        cx.simulate_resize(size(px(1200.), px(800.)));
        cx.run_until_parked();
        let trigger = cx
            .debug_bounds("toolbar-tool-actions")
            .expect("the Actions trigger should render");
        cx.simulate_click(trigger.center(), Modifiers::none());
        cx.run_until_parked();
        cx.simulate_keystrokes("g e n e r a t e space i m a g e");
        cx.run_until_parked();

        assert_eq!(query.borrow().as_ref(), "generate image");
        assert!(cx.debug_bounds("toolbar-actions-palette").is_some());
        assert_eq!(
            view.read_with(cx, |view, _| view.tools().kind()),
            ToolKind::Select
        );

        let palette = cx
            .debug_bounds("toolbar-actions-palette")
            .expect("typing must keep the palette open");
        cx.simulate_click(
            point(palette.left() + px(100.), palette.top() + px(27.)),
            Modifiers::none(),
        );
        cx.simulate_keystrokes("secondary-a r e c t a n g l e");
        cx.run_until_parked();
        assert_eq!(query.borrow().as_ref(), "rectangle");
        assert!(cx.debug_bounds("toolbar-actions-palette").is_some());
        assert_eq!(
            view.read_with(cx, |view, _| view.tools().kind()),
            ToolKind::Select
        );

        cx.simulate_keystrokes("escape r");
        cx.run_until_parked();
        assert!(cx.debug_bounds("toolbar-actions-palette").is_none());
        assert_eq!(
            view.read_with(cx, |view, _| view.tools().kind()),
            ToolKind::Rect
        );
    }

    #[gpui::test]
    async fn generation_actions_are_searchable_and_open_native_workspace_tabs(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let file_system = FakeFs::new(cx.executor());
        let project = Project::test(file_system.clone(), [], cx).await;
        cx.update(|cx| {
            client::Client::set_global(project.read(cx).client(), cx);
            <dyn fs::Fs>::set_global(file_system, cx);
            let bindings = settings::KeymapFile::load_asset_allow_partial_failure(
                settings::DEFAULT_KEYMAP_PATH,
                cx,
            )
            .expect("default key bindings");
            cx.bind_keys(bindings);
        });
        let item = crate::document::ready_item_for_test(
            &project,
            PathBuf::from("/tmp/Generation-toolbar.fig"),
            test_doc(),
            cx,
        );
        let (multi_workspace, cx) = cx.add_window_view(|window, cx| {
            workspace::MultiWorkspace::test_new(project.clone(), window, cx)
        });
        let workspace =
            multi_workspace.read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone());
        let view = cx.update(|window, cx| {
            let view = cx.new(|cx| FigView::new(item, project, window, cx));
            workspace.update(cx, |workspace, cx| {
                workspace.add_item_to_active_pane(Box::new(view.clone()), None, true, window, cx);
            });
            view
        });
        cx.simulate_resize(size(px(1200.), px(900.)));
        cx.run_until_parked();
        let toolbar = view.read_with(cx, |view, _| {
            view.gpui_toolbar_adapter()
                .expect("canvas toolbar")
                .panel
                .clone()
        });
        let invoked = Rc::new(RefCell::new(Vec::new()));
        let _subscription = cx.update(|_, cx| {
            let invoked = invoked.clone();
            cx.subscribe(&toolbar, move |_, event: &ToolbarAction, _| {
                if let ToolbarAction::CommandInvoked { command } = event {
                    invoked.borrow_mut().push(*command);
                }
            })
        });
        for (query, command, selector, expected_tab) in [
            (
                "generate image",
                ToolbarCommand::GenerateImage,
                "toolbar-command-generate-an-image",
                "AI · Image",
            ),
            (
                "GENERATE VIDEO",
                ToolbarCommand::GenerateVideo,
                "toolbar-command-generate-a-video",
                "AI · Video",
            ),
            (
                "generate vector",
                ToolbarCommand::GenerateVector,
                "toolbar-command-generate-vectors",
                "AI · Create from prompt",
            ),
            (
                "generate design",
                ToolbarCommand::GenerateDesign,
                "toolbar-command-generate-a-design",
                "AI · Design",
            ),
            (
                "generate masks",
                ToolbarCommand::GenerateMasks,
                "toolbar-command-generate-masks",
                "AI · Masks",
            ),
            (
                "remove background",
                ToolbarCommand::RemoveBackground,
                "toolbar-command-remove-background",
                "AI · Masks",
            ),
        ] {
            cx.update(|window, cx| {
                workspace.update(cx, |workspace, cx| {
                    assert!(workspace.activate_item(&view, true, true, window, cx));
                });
            });
            cx.run_until_parked();
            let trigger = cx
                .debug_bounds("toolbar-tool-actions")
                .expect("Actions trigger");
            cx.simulate_click(trigger.center(), Modifiers::none());
            cx.simulate_keystrokes("secondary-a");
            cx.simulate_input(query);
            cx.run_until_parked();
            let result = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("'{query}' must offer {command:?}"));
            if command == ToolbarCommand::GenerateImage {
                cx.simulate_keystrokes("enter");
            } else {
                cx.simulate_click(result.center(), Modifiers::none());
            }
            cx.run_until_parked();
            assert_eq!(
                invoked.borrow().last().copied(),
                Some(command),
                "activate the matching result"
            );
            workspace.read_with(cx, |workspace, cx| {
                let item = workspace.active_item(cx).expect("generation tab");
                assert_eq!(
                    item.tab_content_text(0, cx).as_ref(),
                    expected_tab,
                    "'{query}' opens the corresponding native tab"
                );
                assert_ne!(item.item_id(), view.entity_id());
            });
        }
    }

    #[gpui::test]
    async fn structure_and_text_ai_commands_are_searchable_and_emit_their_host_actions(
        cx: &mut TestAppContext,
    ) {
        let (_view, toolbar, mut cx) = setup(cx).await;
        let cx = &mut cx;
        let invoked = Rc::new(RefCell::new(Vec::new()));
        let _subscription = cx.update(|_, cx| {
            let invoked = invoked.clone();
            cx.subscribe(&toolbar, move |_, action: &ToolbarAction, _| {
                if let ToolbarAction::CommandInvoked { command } = action {
                    invoked.borrow_mut().push(*command);
                }
            })
        });
        cx.simulate_resize(size(px(1_200.), px(900.)));
        cx.run_until_parked();

        let commands = [
            (
                "group selection",
                "toolbar-command-group-selection",
                ToolbarCommand::Group,
            ),
            (
                "ungroup selection",
                "toolbar-command-ungroup-selection",
                ToolbarCommand::Ungroup,
            ),
            (
                "frame selection",
                "toolbar-command-frame-selection",
                ToolbarCommand::FrameSelection,
            ),
            (
                "replace content",
                "toolbar-command-replace-content",
                ToolbarCommand::ReplaceContent,
            ),
            (
                "rewrite text",
                "toolbar-command-rewrite-text",
                ToolbarCommand::RewriteText,
            ),
            (
                "translate text",
                "toolbar-command-translate-text",
                ToolbarCommand::TranslateText,
            ),
            (
                "rename layers",
                "toolbar-command-rename-layers",
                ToolbarCommand::RenameLayers,
            ),
        ];
        for (query, selector, command) in commands {
            let trigger = cx
                .debug_bounds("toolbar-tool-actions")
                .expect("Actions trigger");
            cx.simulate_click(trigger.center(), Modifiers::none());
            cx.simulate_keystrokes("secondary-a");
            cx.simulate_input(query);
            cx.run_until_parked();

            let result = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("'{query}' must expose {command:?}"));
            cx.simulate_click(result.center(), Modifiers::none());
            cx.run_until_parked();
            assert_eq!(
                invoked.borrow().last().copied(),
                Some(command),
                "'{query}' must emit its implemented host action"
            );
        }

        assert_eq!(
            invoked.borrow().as_slice(),
            commands.map(|(_, _, command)| command).as_slice(),
            "each newly exposed command must emit once in registry order"
        );
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
            view.set_container_bounds(Bounds::new(point(px(0.), px(0.)), size(px(800.), px(600.))));
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

    fn selection(view: &Entity<FigView>, cx: &mut VisualTestContext) -> Vec<fanta_doc::NodeId> {
        view.read_with(cx, |view, cx| {
            view.item()
                .read(cx)
                .document()
                .map(|document| document.doc.selection.as_slice().to_vec())
                .unwrap_or_default()
        })
    }

    /// End-to-end click-through guard: the dock renders as an absolute
    /// overlay inside the canvas workspace, so before the toolbar occluded
    /// its hitbox a tool press also landed on `fig-container`'s
    /// `on_mouse_down` and the Select tool cleared the selection. Every dock
    /// press below leaves the canvas selection untouched while the toolbar
    /// itself reacts; the closing canvas press proves the probe is live.
    #[gpui::test]
    async fn dock_presses_reach_the_toolbar_but_never_the_canvas(cx: &mut TestAppContext) {
        let (view, toolbar, mut cx) = setup(cx).await;
        let cx = &mut cx;
        cx.simulate_resize(size(px(1_280.), px(800.)));
        cx.run_until_parked();

        let initial = selection(&view, cx);
        assert_eq!(initial.len(), 1, "the fixture starts with Target selected");
        let canvas = cx
            .debug_bounds("fig-container")
            .expect("the canvas container should render");
        let surface = cx
            .debug_bounds("editor-toolbar-surface")
            .expect("the dock surface should render");
        assert!(
            surface.intersects(&canvas),
            "the dock must overlay the canvas for the probe to mean anything: {surface:?} vs {canvas:?}"
        );

        // A tool tile: the toolbar switches the host tool, the canvas sees
        // nothing (a Select press on empty canvas would have deselected).
        let text = cx
            .debug_bounds("toolbar-tool-text")
            .expect("the Text tool tile should render");
        cx.simulate_click(text.center(), Modifiers::default());
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |view, _| view.active_tool()),
            ToolKind::Text,
            "the tool press must reach the toolbar"
        );
        assert_eq!(
            selection(&view, cx),
            initial,
            "the tool press must not reach the canvas"
        );
        assert!(!view.read_with(cx, |view, _| view.primary_pressed()));

        // Dock padding, with the Text tool live: a canvas press here would
        // create a text layer and select it.
        cx.simulate_click(surface.origin + point(px(3.), px(3.)), Modifiers::default());
        cx.run_until_parked();
        assert_eq!(
            selection(&view, cx),
            initial,
            "dock padding must not reach the canvas"
        );
        assert_eq!(
            view.read_with(cx, |view, _| view.active_tool()),
            ToolKind::Text
        );

        // The chrome capsule: fit-to-view reframes the page and toggles the
        // sidebar, still without touching the selection.
        view.update_in(cx, |view, _, cx| {
            view.set_viewport_silent(Viewport {
                center: [-900.0, 700.0],
                zoom: 3.0,
            });
            cx.notify();
        });
        cx.run_until_parked();
        let fit = cx
            .debug_bounds("toolbar-chrome-fit-to-view")
            .expect("the fit-to-view chrome control should render");
        cx.simulate_click(fit.center(), Modifiers::default());
        cx.run_until_parked();
        let viewport = view.read_with(cx, |view, _| view.viewport().expect("viewport"));
        assert!(
            (viewport.center[0] - 260.0).abs() < 1.0 && (viewport.center[1] - 170.0).abs() < 1.0,
            "fit-to-view must reframe the page contents, got {:?}",
            viewport.center
        );
        assert_eq!(selection(&view, cx), initial);

        // Beside the dock the canvas is live: back on Select, an empty-canvas
        // press clears the selection.
        view.update_in(cx, |view, _, cx| view.activate_tool(ToolKind::Select, cx));
        cx.run_until_parked();
        let probe = canvas.origin + point(px(24.), px(24.));
        assert!(
            !surface.contains(&probe),
            "the probe must lie outside the dock"
        );
        cx.simulate_click(probe, Modifiers::default());
        cx.run_until_parked();
        assert!(
            selection(&view, cx).is_empty(),
            "an empty-canvas press beside the dock must reach the Select tool"
        );
        toolbar.read_with(cx, |toolbar, _| {
            assert_eq!(toolbar.active_tool(), ToolbarTool::Move);
        });
    }

    /// The chrome capsule is the §12 round trip for host chrome: the tiles
    /// mirror the sidebars, a press flips the host state, and the echo flips
    /// the tile's icon and raised state; `Resources` reveals (never hides)
    /// the left sidebar.
    #[gpui::test]
    async fn chrome_toggles_round_trip_through_the_sidebars(cx: &mut TestAppContext) {
        let (view, toolbar, mut cx) = setup(cx).await;
        let cx = &mut cx;
        cx.simulate_resize(size(px(1_280.), px(800.)));
        cx.run_until_parked();

        // Mount-time echo: both sidebars start visible.
        toolbar.read_with(cx, |toolbar, _| {
            let controls = toolbar.chrome_controls();
            assert_eq!(
                controls.iter().map(|c| c.id.as_ref()).collect::<Vec<_>>(),
                [
                    CHROME_FIT_TO_VIEW,
                    CHROME_TOGGLE_LAYERS_SIDEBAR,
                    CHROME_TOGGLE_INSPECTOR_SIDEBAR
                ]
            );
            assert!(controls[1].active && controls[2].active);
            assert_eq!(controls[1].icon_path(), IconName::PanelLeftClose.path());
        });
        assert!(cx.debug_bounds("fanta-layers-sidebar").is_some());
        assert!(cx.debug_bounds("fanta-inspector-sidebar").is_some());
        let baseline = view.read_with(cx, |view, _| {
            view.gpui_toolbar_adapter()
                .expect("adapter")
                .option_push_count()
        });

        // Hide the layers sidebar from the dock.
        let layers = cx
            .debug_bounds("toolbar-chrome-toggle-layers-sidebar")
            .expect("the layers chrome control should render");
        cx.simulate_click(layers.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("fanta-layers-sidebar").is_none(),
            "the layers sidebar must hide"
        );
        assert!(cx.debug_bounds("fanta-inspector-sidebar").is_some());
        toolbar.read_with(cx, |toolbar, _| {
            let controls = toolbar.chrome_controls();
            assert!(!controls[1].active, "the echo must flip the tile off");
            assert_eq!(controls[1].icon_path(), IconName::PanelLeftOpen.path());
            assert_eq!(controls[1].label, "Show Layers");
            assert!(controls[2].active);
        });
        assert!(
            cx.debug_bounds("toolbar-chrome-toggle-layers-sidebar-inactive")
                .is_some()
        );
        assert_eq!(
            view.read_with(cx, |view, _| {
                view.gpui_toolbar_adapter()
                    .expect("adapter")
                    .option_push_count()
            }),
            baseline + 1,
            "one flip, one chrome push"
        );

        // Hide the inspector from the dock too.
        let inspector = cx
            .debug_bounds("toolbar-chrome-toggle-inspector-sidebar")
            .expect("the inspector chrome control should render");
        cx.simulate_click(inspector.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("fanta-inspector-sidebar").is_none());
        toolbar.read_with(cx, |toolbar, _| {
            let controls = toolbar.chrome_controls();
            assert!(!controls[2].active);
            assert_eq!(controls[2].icon_path(), IconName::PanelRightOpen.path());
        });

        // Resources reveals the hidden layers sidebar and focuses it …
        toolbar.update_in(cx, |_, _, cx| {
            cx.emit(ToolbarAction::ToolChangeRequested {
                mode: ToolbarMode::Design,
                tool: ToolbarTool::Resources,
            });
        });
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("fanta-layers-sidebar").is_some(),
            "Resources must reveal the layers sidebar"
        );
        toolbar.read_with(cx, |toolbar, _| {
            assert!(toolbar.chrome_controls()[1].active)
        });
        assert!(
            view.update_in(cx, |view, window, cx| view
                .layers_sidebar_is_focused(window, cx)),
            "Resources must move focus into the layers sidebar"
        );

        // … and never hides it: a second press is a no-op for visibility.
        toolbar.update_in(cx, |_, _, cx| {
            cx.emit(ToolbarAction::ToolChangeRequested {
                mode: ToolbarMode::Design,
                tool: ToolbarTool::Resources,
            });
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("fanta-layers-sidebar").is_some());
        toolbar.read_with(cx, |toolbar, _| {
            assert!(toolbar.chrome_controls()[1].active)
        });
    }
}
