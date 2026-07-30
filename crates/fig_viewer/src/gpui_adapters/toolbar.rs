//! EditorToolbar adapter: maps `ToolbarTool`/`ToolbarMode`/`ToolbarCommand`
//! intents onto FigView's existing tool, mode, zoom, and edit entry points,
//! and echoes FigView state back into the toolbar entity.

use fanta_gpui::toolbar::{EditorToolbar, ToolbarCommand, ToolbarMode, ToolbarTool};
use gpui::{AppContext as _, Context, Entity, Subscription, Window};

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
    ToolbarCommand::Present,
    ToolbarCommand::OpenDesignMode,
    ToolbarCommand::OpenMotionMode,
];

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
/// are roadmap items and intentionally return `None`.
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
        Self {
            panel,
            last_pushed: initial,
            _subscription: subscription,
        }
    }

    /// Echo current host state into the toolbar entity, diff-guarded. Called
    /// from FigView's render path so every tool/mode/zoom mutation site is
    /// covered by the one choke point.
    pub(crate) fn refresh(
        &mut self,
        mode: EditorMode,
        tool: ToolKind,
        zoom_percent: u16,
        cx: &mut gpui::App,
    ) {
        let next = (toolbar_mode(mode), toolbar_tool(tool), zoom_percent);
        if next == self.last_pushed {
            return;
        }
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
}
