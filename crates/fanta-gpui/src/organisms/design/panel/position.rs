//! Compatibility projection factory for the extracted Position section.

use super::*;

pub(super) fn projection(panel: &DesignPanel) -> sections::position::PositionProjection {
    let parent_layout = panel.host.inspection_context.parent_layout();
    let grid_child = panel.host.inspected_node().supports_auto_layout_child()
        && parent_layout.auto_layout_direction() == Some(DesignPanelAutoLayoutDirection::Grid)
        && parent_layout.participates_in_auto_layout();
    let grid_item = grid_child
        .then(|| {
            panel
                .host
                .inspected_node()
                .layout
                .as_ref()
                .map(|layout| &layout.item)
        })
        .flatten();

    sections::position::PositionProjection::new(
        sections::position::PositionIdentityProjection::new(
            panel.id.clone(),
            panel.command_target(),
        ),
        sections::position::PositionAccessProjection::new(
            panel.can_edit(),
            panel.host.inspected_node().supports_arrange()
                && panel.can_edit()
                && parent_layout != DesignPanelParentLayout::Canvas,
            sections::position::can_show_constraints(panel),
            panel.property_is_editable(DesignPanelProperty::GridHorizontalAlignment),
            panel.property_is_editable(DesignPanelProperty::GridVerticalAlignment),
        ),
        sections::position::PositionCapabilityProjection::new(
            panel.host.inspected_node().supports_arrange(),
            panel.host.inspected_node().supports_position_coordinates(),
            panel.host.inspected_node().supports_transforms(),
        ),
        sections::position::PositionPresentationProjection::new(
            panel.sections.constraints_expanded(),
            panel.renders_draw_workspace(),
        ),
        sections::position::PositionValueProjection::new(
            grid_item.map(|item| item.grid_horizontal_alignment),
            grid_item.map(|item| item.grid_vertical_alignment),
            panel.host.inspected_node().x,
            panel.host.inspected_node().y,
            panel.host.inspected_node().rotation,
            panel.host.inspected_node().horizontal_constraint,
            panel.host.inspected_node().vertical_constraint,
        ),
        sections::position::PositionSelectionProjection::new(
            panel.host.inspection_context.selection().kind() == DesignPanelSelectionKind::Multiple,
            sections::position::smart_selection_view_data_for_context(panel).cloned(),
            panel.host.projections.smart_selection.is_some(),
            panel.active_vector_edit().cloned(),
        ),
    )
}
