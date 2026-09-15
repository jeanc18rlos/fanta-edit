//! Thin projection adapter for the extracted Export section.

use super::*;

pub(super) fn projection(panel: &DesignPanel) -> sections::export::ExportProjection {
    let command_target = panel.command_target();
    let active_view_data = panel
        .host
        .projections
        .export
        .as_ref()
        .filter(|view_data| view_data.target == command_target);
    let target = active_view_data.map_or_else(
        || command_target.clone(),
        |view_data| view_data.target.clone(),
    );
    let configurations = active_view_data
        .map(|view_data| view_data.configurations.clone())
        .unwrap_or_else(|| {
            panel
                .host
                .inspected_node()
                .export_settings
                .iter()
                .enumerate()
                .map(|(index, setting)| {
                    DesignExportConfiguration::from_legacy(
                        SharedString::from(format!(
                            "{}-legacy-export-{index}",
                            panel.host.inspected_node().id
                        )),
                        setting,
                    )
                })
                .collect()
        });
    let mode = active_view_data.map_or(DesignExportMode::Static, |view_data| view_data.mode);
    let static_capabilities = active_view_data.map_or(Default::default(), |view_data| {
        view_data.static_capabilities
    });
    let animated = active_view_data.and_then(|view_data| view_data.animated.clone());
    let preview_state = active_view_data.and_then(|view_data| view_data.preview.clone());
    let preview_available = panel.host.inspection_context.selection().kind()
        != DesignPanelSelectionKind::Multiple
        && mode == DesignExportMode::Static
        && preview_state.is_some();

    sections::export::ExportProjection::new(
        sections::export::ExportTargetProjection::new(
            panel.id.clone(),
            target,
            panel.host.inspected_node().name.clone(),
            panel.host.inspection_context.permissions().can_export(),
            panel.collection_is_supported(DesignPanelCollection::Export),
        ),
        sections::export::ExportStaticProjection::new(
            configurations,
            static_capabilities,
            &panel.features.export.expanded_settings,
        ),
        sections::export::ExportAnimatedProjection::new(mode, animated),
        sections::export::ExportPreviewProjection::new(
            preview_available,
            panel.features.export.preview_expanded,
            preview_state,
        ),
        sections::export::ExportPresentationProjection::new(
            panel.overlays.export_choice_overlay().clone(),
        ),
    )
}
