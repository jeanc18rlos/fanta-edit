//! Compatibility projection factory for the extracted Appearance section.

use super::*;

pub(super) fn projection(panel: &DesignPanel) -> sections::appearance::AppearanceProjection {
    let target = panel.command_target();
    let multiple_selection_has_uniform_draw_geometry =
        panel.host.inspection_context.selection().kind() != DesignPanelSelectionKind::Multiple
            || [DesignPanelProperty::Width, DesignPanelProperty::Height]
                .into_iter()
                .all(|property| {
                    panel
                        .host
                        .property_states
                        .get(&property)
                        .and_then(DesignPanelPropertyValueState::resolved)
                        .is_some_and(|value| {
                            matches!(value, DesignPanelValue::Number(value) if value.is_finite())
                        })
                });
    let draw_corner_radius_range = panel
        .host
        .projections
        .draw_appearance
        .as_ref()
        .filter(|view_data| {
            view_data.is_valid()
                && view_data.target == target
                && !panel.host.inspection_context.selection().is_empty()
                && multiple_selection_has_uniform_draw_geometry
                && panel
                    .host
                    .inspected_node()
                    .supports_section(DesignPanelSection::Layer)
        })
        .map(|view_data| view_data.corner_radius_range);
    let node = &panel.host.inspected_node();
    let corner_capabilities = node.corner_capabilities;
    let visibility = match panel
        .host
        .property_states
        .get(&DesignPanelProperty::Visible)
    {
        Some(state) if state.is_mixed() => VisibilityControlState::Mixed,
        Some(state) => match state.resolved() {
            Some(DesignPanelValue::Bool(visible)) => VisibilityControlState::from_visible(*visible),
            _ => VisibilityControlState::from_visible(node.visible),
        },
        None => VisibilityControlState::from_visible(node.visible),
    };

    sections::appearance::AppearanceProjection::new(
        panel.id.clone(),
        target,
        sections::appearance::AppearanceCapabilities {
            can_edit: panel.can_edit(),
            visibility_editable: panel.property_is_editable(DesignPanelProperty::Visible),
            blend_mode_editable: panel.property_is_editable(DesignPanelProperty::BlendMode),
            supports_visibility: node.supports_visibility(),
            supports_layer_appearance: node.supports_layer_appearance(),
            supports_pass_through_blend: node.supports_pass_through_blend(),
            supports_mask_section: node.supports_section(DesignPanelSection::Mask),
            uniform_radius: corner_capabilities.uniform_radius,
            independent_radii: corner_capabilities.independent_radii,
            smoothing: corner_capabilities.smoothing,
        },
        sections::appearance::AppearanceValues {
            visible: node.visible,
            visibility,
            opacity: node.opacity,
            blend_mode: node.blend_mode,
            corner_radii: node.corner_radii,
            independent_corners: node.independent_corners,
            corner_smoothing: node.corner_smoothing,
            shape_geometry: node.shape_geometry,
            mask_type: node.effective_mask_type(),
        },
        sections::appearance::AppearancePresentation {
            renders_draw_workspace: panel.renders_draw_workspace(),
            layer_expanded: panel.sections.is_expanded(DesignPanelSection::Layer),
            details_expanded: panel.sections.appearance_details_expanded(),
            blend_mode_open: panel.overlays.appearance_blend_mode_open(),
            opacity_slider: panel.retained.draw_sliders.opacity.clone(),
            corner_radius_slider: panel.retained.draw_sliders.corner_radius.clone(),
            draw_corner_radius_range,
            draw_slider_property: panel.edit.draw_slider_property,
            property_editor_active: panel.edit.property.is_some(),
            opacity_scrub_available: panel
                .numeric_scrub_seed(DesignPanelProperty::Opacity)
                .is_some(),
            corner_radius_scrub_available: panel
                .numeric_scrub_seed(DesignPanelProperty::CornerRadius)
                .is_some(),
        },
    )
}

pub(super) fn visibility_projection(
    panel: &DesignPanel,
    property: DesignPanelProperty,
    fallback_visible: bool,
) -> sections::appearance::AppearanceVisibilityProjection {
    let visibility = match panel.host.property_states.get(&property) {
        Some(state) if state.is_mixed() => VisibilityControlState::Mixed,
        Some(state) => match state.resolved() {
            Some(DesignPanelValue::Bool(visible)) => VisibilityControlState::from_visible(*visible),
            _ => VisibilityControlState::from_visible(fallback_visible),
        },
        None => VisibilityControlState::from_visible(fallback_visible),
    };
    sections::appearance::AppearanceVisibilityProjection::new(
        panel.id.clone(),
        panel.command_target(),
        property,
        visibility,
        panel.property_is_editable(property),
    )
}
