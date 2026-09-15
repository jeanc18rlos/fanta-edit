//! Export section projection, live event validation, and rendering.

use super::super::super::{DesignAnimatedExportViewData, DesignStaticExportCapabilities};
use super::super::*;

/// Host identity and permissions used by Export actions.
#[derive(Clone)]
pub(in super::super) struct ExportTargetProjection {
    panel_id: SharedString,
    target: DesignPanelTarget,
    node_name: SharedString,
    can_export: bool,
    collection_supported: bool,
}

impl ExportTargetProjection {
    pub(in super::super) fn new(
        panel_id: SharedString,
        target: DesignPanelTarget,
        node_name: SharedString,
        can_export: bool,
        collection_supported: bool,
    ) -> Self {
        Self {
            panel_id,
            target,
            node_name,
            can_export,
            collection_supported,
        }
    }
}

/// One host-owned static export configuration plus its local disclosure state.
#[derive(Clone)]
pub(in super::super) struct ExportStaticRowProjection {
    index: usize,
    configuration: DesignExportConfiguration,
    expanded: bool,
}

/// Static Export rows and target-specific capabilities.
#[derive(Clone)]
pub(in super::super) struct ExportStaticProjection {
    rows: Vec<ExportStaticRowProjection>,
    capabilities: DesignStaticExportCapabilities,
}

impl ExportStaticProjection {
    pub(in super::super) fn new(
        configurations: Vec<DesignExportConfiguration>,
        capabilities: DesignStaticExportCapabilities,
        expanded_configuration_ids: &HashSet<SharedString>,
    ) -> Self {
        Self {
            rows: configurations
                .into_iter()
                .enumerate()
                .map(|(index, configuration)| ExportStaticRowProjection {
                    expanded: expanded_configuration_ids.contains(&configuration.id),
                    index,
                    configuration,
                })
                .collect(),
            capabilities,
        }
    }
}

/// Host-controlled mode and animated-export settings.
#[derive(Clone)]
pub(in super::super) struct ExportAnimatedProjection {
    mode: DesignExportMode,
    view_data: Option<DesignAnimatedExportViewData>,
}

impl ExportAnimatedProjection {
    pub(in super::super) fn new(
        mode: DesignExportMode,
        view_data: Option<DesignAnimatedExportViewData>,
    ) -> Self {
        Self { mode, view_data }
    }
}

/// Host preview state combined with the local disclosure state.
#[derive(Clone)]
pub(in super::super) struct ExportPreviewProjection {
    available: bool,
    expanded: bool,
    state: Option<DesignExportPreviewState>,
}

impl ExportPreviewProjection {
    pub(in super::super) fn new(
        available: bool,
        expanded: bool,
        state: Option<DesignExportPreviewState>,
    ) -> Self {
        Self {
            available,
            expanded,
            state,
        }
    }
}

/// Export-local overlay/disclosure presentation state.
#[derive(Clone)]
pub(in super::super) struct ExportPresentationProjection {
    choice_overlay: Option<SharedString>,
}

impl ExportPresentationProjection {
    pub(in super::super) fn new(choice_overlay: Option<SharedString>) -> Self {
        Self { choice_overlay }
    }
}

/// Canonical immutable input to the complete Export renderer.
#[derive(Clone)]
pub(in super::super) struct ExportProjection {
    target: ExportTargetProjection,
    static_export: ExportStaticProjection,
    animated: ExportAnimatedProjection,
    preview: ExportPreviewProjection,
    presentation: ExportPresentationProjection,
}

impl ExportProjection {
    pub(in super::super) fn new(
        target: ExportTargetProjection,
        static_export: ExportStaticProjection,
        animated: ExportAnimatedProjection,
        preview: ExportPreviewProjection,
        presentation: ExportPresentationProjection,
    ) -> Self {
        Self {
            target,
            static_export,
            animated,
            preview,
            presentation,
        }
    }
}

/// Compatibility methods still used by the property-editing façade.
///
/// The complete Export renderer no longer lives on `DesignPanel`; these
/// retained methods are the narrow bridge for edit reconciliation while that
/// controller is migrated independently.
pub(in super::super) trait ExportPanelCompat {
    fn can_export(&self) -> bool;
    fn export_target(&self) -> DesignPanelTarget;
    fn active_export_view_data(&self) -> Option<&DesignExportViewData>;
    fn export_configurations(&self) -> Vec<DesignExportConfiguration>;
    fn export_configuration(&self, index: usize) -> Option<DesignExportConfiguration>;
    fn export_property_index(property: DesignPanelProperty) -> Option<usize>;
    fn export_property_with_index(
        property: DesignPanelProperty,
        index: usize,
    ) -> DesignPanelProperty;
    fn reconcile_export_property_editor(
        &mut self,
        next_view_data: &DesignExportViewData,
        cx: &mut Context<DesignPanel>,
    );
    fn export_preview_available(&self) -> bool;
    fn export_change_for_property(
        &self,
        property: DesignPanelProperty,
        value: DesignPanelValue,
    ) -> Option<(SharedString, DesignExportConfigurationChange)>;
    fn emit_export_configuration_change(
        &mut self,
        configuration_id: SharedString,
        change: DesignExportConfigurationChange,
        phase: DesignPanelEditPhase,
        cx: &mut Context<DesignPanel>,
    );
    fn emit_export_all(&mut self, cx: &mut Context<DesignPanel>);
    fn emit_export_mode_change(&mut self, mode: DesignExportMode, cx: &mut Context<DesignPanel>);
    fn emit_animated_export_change(
        &mut self,
        change: DesignAnimatedExportChange,
        cx: &mut Context<DesignPanel>,
    );
    fn emit_animated_export(&mut self, cx: &mut Context<DesignPanel>);
    fn toggle_export_advanced(
        &mut self,
        configuration_id: SharedString,
        cx: &mut Context<DesignPanel>,
    );
    fn set_export_choice_overlay(
        &mut self,
        overlay: SharedString,
        open: bool,
        cx: &mut Context<DesignPanel>,
    );
    fn toggle_export_preview(&mut self, cx: &mut Context<DesignPanel>);
    fn render_export(&self, cx: &mut Context<DesignPanel>) -> AnyElement;
}

impl ExportPanelCompat for DesignPanel {
    fn can_export(&self) -> bool {
        self.host.inspection_context.permissions().can_export()
    }

    fn export_target(&self) -> DesignPanelTarget {
        self.host
            .projections
            .export
            .as_ref()
            .filter(|view_data| view_data.target == self.command_target())
            .map_or_else(
                || self.command_target(),
                |view_data| view_data.target.clone(),
            )
    }

    fn active_export_view_data(&self) -> Option<&DesignExportViewData> {
        self.host
            .projections
            .export
            .as_ref()
            .filter(|view_data| view_data.target == self.command_target())
    }

    fn export_configurations(&self) -> Vec<DesignExportConfiguration> {
        self.active_export_view_data()
            .map(|view_data| view_data.configurations.clone())
            .unwrap_or_else(|| {
                self.host
                    .inspected_node()
                    .export_settings
                    .iter()
                    .enumerate()
                    .map(|(index, setting)| {
                        DesignExportConfiguration::from_legacy(
                            SharedString::from(format!(
                                "{}-legacy-export-{index}",
                                self.host.inspected_node().id
                            )),
                            setting,
                        )
                    })
                    .collect()
            })
    }

    fn export_configuration(&self, index: usize) -> Option<DesignExportConfiguration> {
        self.export_configurations().into_iter().nth(index)
    }

    fn export_property_index(property: DesignPanelProperty) -> Option<usize> {
        match property {
            DesignPanelProperty::ExportSizing(index)
            | DesignPanelProperty::ExportScale(index)
            | DesignPanelProperty::ExportSuffix(index)
            | DesignPanelProperty::ExportFormat(index) => Some(index),
            _ => None,
        }
    }

    fn export_property_with_index(
        property: DesignPanelProperty,
        index: usize,
    ) -> DesignPanelProperty {
        match property {
            DesignPanelProperty::ExportSizing(_) => DesignPanelProperty::ExportSizing(index),
            DesignPanelProperty::ExportScale(_) => DesignPanelProperty::ExportScale(index),
            DesignPanelProperty::ExportSuffix(_) => DesignPanelProperty::ExportSuffix(index),
            DesignPanelProperty::ExportFormat(_) => DesignPanelProperty::ExportFormat(index),
            property => property,
        }
    }

    fn reconcile_export_property_editor(
        &mut self,
        next_view_data: &DesignExportViewData,
        cx: &mut Context<DesignPanel>,
    ) {
        let Some(editor) = self.edit.property.as_ref() else {
            return;
        };
        let Some(configuration_id) = editor.export_configuration_id.clone() else {
            return;
        };
        let previous_property = editor.property;
        let next_index = (next_view_data.target == self.command_target())
            .then(|| {
                next_view_data
                    .configurations
                    .iter()
                    .position(|configuration| configuration.id == configuration_id)
            })
            .flatten();
        let Some(next_index) = next_index else {
            self.cancel_property_editor_transaction(cx);
            return;
        };
        let next_property = Self::export_property_with_index(editor.property, next_index);
        let previous_view_data = self.host.projections.export.replace(next_view_data.clone());
        let next_property_is_editable = self.current_property_value(next_property).is_some()
            && self.property_is_editable(next_property);
        self.host.projections.export = previous_view_data;
        if !next_property_is_editable {
            self.cancel_property_editor_transaction(cx);
            return;
        }
        if let Some(editor) = self.edit.property.as_mut() {
            editor.property = next_property;
        }
        self.edit
            .rebase_property_edit(previous_property, next_property);
        if let Some(scrub) = self
            .edit
            .numeric_scrub
            .as_mut()
            .filter(|scrub| scrub.active)
        {
            scrub.property = next_property;
        }
    }

    fn export_preview_available(&self) -> bool {
        self.host.inspection_context.selection().kind() != DesignPanelSelectionKind::Multiple
            && self.active_export_view_data().is_some_and(|view_data| {
                view_data.mode == DesignExportMode::Static && view_data.preview.is_some()
            })
    }

    fn export_change_for_property(
        &self,
        property: DesignPanelProperty,
        value: DesignPanelValue,
    ) -> Option<(SharedString, DesignExportConfigurationChange)> {
        let (index, change) = match (property, value) {
            (DesignPanelProperty::ExportSizing(index), DesignPanelValue::ExportSizing(sizing)) => {
                (index, DesignExportConfigurationChange::Sizing(sizing))
            }
            (DesignPanelProperty::ExportScale(index), DesignPanelValue::Number(scale)) => (
                index,
                DesignExportConfigurationChange::Sizing(DesignExportSizing::Scale(scale)),
            ),
            (DesignPanelProperty::ExportSuffix(index), DesignPanelValue::Text(suffix)) => {
                (index, DesignExportConfigurationChange::Suffix(suffix))
            }
            (DesignPanelProperty::ExportFormat(index), DesignPanelValue::ExportFormat(format)) => {
                (index, DesignExportConfigurationChange::Format(format))
            }
            _ => return None,
        };
        let configuration_id = self
            .edit
            .property
            .as_ref()
            .filter(|editor| editor.property == property)
            .and_then(|editor| editor.export_configuration_id.clone())
            .or_else(|| {
                self.export_configuration(index)
                    .map(|configuration| configuration.id)
            })?;
        Some((configuration_id, change))
    }

    fn emit_export_configuration_change(
        &mut self,
        configuration_id: SharedString,
        change: DesignExportConfigurationChange,
        phase: DesignPanelEditPhase,
        cx: &mut Context<DesignPanel>,
    ) {
        if !self.collection_is_supported(DesignPanelCollection::Export) || !self.can_export() {
            return;
        }
        let Some(configuration) = self
            .export_configurations()
            .into_iter()
            .find(|configuration| configuration.id == configuration_id)
        else {
            return;
        };
        let capabilities = self
            .active_export_view_data()
            .map_or(Default::default(), |view_data| {
                view_data.static_capabilities
            });
        let applicable = match &change {
            DesignExportConfigurationChange::Sizing(_) => {
                configuration.format().supports_custom_sizing()
            }
            DesignExportConfigurationChange::Suffix(_)
            | DesignExportConfigurationChange::Format(_)
            | DesignExportConfigurationChange::ColorProfile(_) => true,
            DesignExportConfigurationChange::IgnoreOverlappingLayers(_) => matches!(
                configuration.format_settings,
                DesignExportFormatSettings::Png(_)
                    | DesignExportFormatSettings::Jpg(_)
                    | DesignExportFormatSettings::Svg(_)
            ),
            DesignExportConfigurationChange::IncludeTextBoundingBox(_) => {
                capabilities.can_include_text_bounding_box
                    && matches!(
                        configuration.format_settings,
                        DesignExportFormatSettings::Png(_) | DesignExportFormatSettings::Jpg(_)
                    )
            }
            DesignExportConfigurationChange::Resampling(_) => matches!(
                configuration.format_settings,
                DesignExportFormatSettings::Png(_)
                    | DesignExportFormatSettings::Jpg(_)
                    | DesignExportFormatSettings::Pdf(_)
            ),
            DesignExportConfigurationChange::Quality(_) => matches!(
                configuration.format_settings,
                DesignExportFormatSettings::Jpg(_) | DesignExportFormatSettings::Pdf(_)
            ),
            DesignExportConfigurationChange::IncludeBounds(_) => {
                capabilities.can_include_svg_bounds
                    && matches!(
                        configuration.format_settings,
                        DesignExportFormatSettings::Svg(_)
                    )
            }
            DesignExportConfigurationChange::IncludeIdAttribute(_)
            | DesignExportConfigurationChange::OutlineText(_)
            | DesignExportConfigurationChange::SimplifyStroke(_) => matches!(
                configuration.format_settings,
                DesignExportFormatSettings::Svg(_)
            ),
        };
        if !self.can_export() || !applicable {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::ExportConfigurationChangeRequested {
                target: self.export_target(),
                configuration_id,
                change,
                phase,
            },
        );
    }

    fn emit_export_all(&mut self, cx: &mut Context<Self>) {
        if self.can_export()
            && self.collection_is_supported(DesignPanelCollection::Export)
            && !self.export_configurations().is_empty()
        {
            cx.emit_design_panel_action(
                self,
                DesignPanelAction::ExportAllRequested {
                    target: self.export_target(),
                },
            );
        }
    }

    fn emit_export_mode_change(&mut self, mode: DesignExportMode, cx: &mut Context<Self>) {
        if !self.can_export()
            || !self.collection_is_supported(DesignPanelCollection::Export)
            || self
                .active_export_view_data()
                .is_none_or(|view_data| view_data.animated.is_none())
        {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::ExportModeChangeRequested {
                target: self.export_target(),
                mode,
            },
        );
    }

    fn emit_animated_export_change(
        &mut self,
        change: DesignAnimatedExportChange,
        cx: &mut Context<Self>,
    ) {
        let Some(view_data) = self.active_export_view_data() else {
            return;
        };
        let Some(animated) = &view_data.animated else {
            return;
        };
        let mut candidate = animated.settings.clone();
        if !self.can_export()
            || !self.collection_is_supported(DesignPanelCollection::Export)
            || view_data.mode != DesignExportMode::Animated
        {
            return;
        }
        if let DesignAnimatedExportChange::Format(format) = &change
            && !animated.capability.available_formats.contains(format)
        {
            return;
        }
        if !candidate.apply_change(change.clone()) {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::AnimatedExportChangeRequested {
                target: self.export_target(),
                change,
                phase: DesignPanelEditPhase::Commit,
            },
        );
    }

    fn emit_animated_export(&mut self, cx: &mut Context<Self>) {
        let Some(view_data) = self.active_export_view_data() else {
            return;
        };
        let Some(animated) = &view_data.animated else {
            return;
        };
        if !self.can_export()
            || !self.collection_is_supported(DesignPanelCollection::Export)
            || !animated.capability.allows(&animated.settings)
        {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::AnimatedExportRequested {
                target: self.export_target(),
                settings: animated.settings.clone(),
            },
        );
    }

    fn toggle_export_advanced(&mut self, configuration_id: SharedString, cx: &mut Context<Self>) {
        if !self
            .features
            .export
            .expanded_settings
            .remove(&configuration_id)
        {
            self.features
                .export
                .expanded_settings
                .insert(configuration_id);
        }
        cx.notify();
    }

    fn set_export_choice_overlay(
        &mut self,
        overlay: SharedString,
        open: bool,
        cx: &mut Context<Self>,
    ) {
        self.overlays
            .set_open(DesignOverlayState::ExportChoice(overlay), open);
        cx.notify();
    }

    fn toggle_export_preview(&mut self, cx: &mut Context<Self>) {
        if !self.export_preview_available() {
            return;
        }
        self.features.export.preview_expanded = !self.features.export.preview_expanded;
        if self.features.export.preview_expanded {
            cx.emit_design_panel_action(
                self,
                DesignPanelAction::ExportPreviewRequested {
                    target: self.export_target(),
                },
            );
        }
        cx.notify();
    }

    fn render_export(&self, cx: &mut Context<Self>) -> AnyElement {
        let projection = super::super::export::projection(self);
        render(&projection, self, &ExportEventSink::new(cx.entity()), cx)
    }
}

/// Narrow Design-facade chrome needed by the domain-neutral Export renderer.
pub(in super::super) trait ExportInspectorChrome {
    fn export_value_cell(
        &self,
        id_suffix: impl Into<SharedString>,
        prefix: &'static str,
        value: impl Into<SharedString>,
        property: DesignPanelProperty,
        next: DesignPanelValue,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn export_section(
        &self,
        collection: Option<DesignPanelCollection>,
        content: AnyElement,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;
}

impl ExportInspectorChrome for DesignPanel {
    fn export_value_cell(
        &self,
        id_suffix: impl Into<SharedString>,
        prefix: &'static str,
        value: impl Into<SharedString>,
        property: DesignPanelProperty,
        next: DesignPanelValue,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_value_cell(id_suffix, prefix, value, property, next, cx)
    }

    fn export_section(
        &self,
        collection: Option<DesignPanelCollection>,
        content: AnyElement,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_section(DesignPanelSection::Export, collection, content, cx)
    }
}

#[derive(Clone)]
enum ExportEvent {
    ConfigurationChange {
        expected_target: DesignPanelTarget,
        configuration_id: SharedString,
        change: DesignExportConfigurationChange,
        phase: DesignPanelEditPhase,
        dismiss_overlay: bool,
    },
    RemoveConfiguration {
        expected_target: DesignPanelTarget,
        configuration_id: SharedString,
    },
    ModeChange {
        expected_target: DesignPanelTarget,
        mode: DesignExportMode,
    },
    AnimatedChange {
        expected_target: DesignPanelTarget,
        change: DesignAnimatedExportChange,
        dismiss_overlay: bool,
    },
    AnimatedExport {
        expected_target: DesignPanelTarget,
    },
    ExportAll {
        expected_target: DesignPanelTarget,
    },
    ToggleAdvanced {
        configuration_id: SharedString,
    },
    SetChoiceOverlay {
        overlay: SharedString,
        open: bool,
    },
    ToggleChoiceOverlay(SharedString),
    TogglePreview {
        expected_target: DesignPanelTarget,
    },
}

fn target_is_current(panel: &DesignPanel, expected_target: &DesignPanelTarget) -> bool {
    panel.export_target() == expected_target.clone()
}

fn dispatch(panel: &mut DesignPanel, event: ExportEvent, cx: &mut Context<DesignPanel>) {
    match event {
        ExportEvent::ConfigurationChange {
            expected_target,
            configuration_id,
            change,
            phase,
            dismiss_overlay,
        } => {
            if !target_is_current(panel, &expected_target) {
                return;
            }
            if dismiss_overlay {
                panel.overlays.discard(DesignOpenOverlay::ExportChoice);
            }
            panel.emit_export_configuration_change(configuration_id, change, phase, cx);
            if dismiss_overlay {
                cx.notify();
            }
        }
        ExportEvent::RemoveConfiguration {
            expected_target,
            configuration_id,
        } => {
            if !target_is_current(panel, &expected_target)
                || !panel.can_export()
                || !panel.collection_is_supported(DesignPanelCollection::Export)
                || !panel
                    .export_configurations()
                    .iter()
                    .any(|configuration| configuration.id == configuration_id)
            {
                return;
            }
            cx.emit_design_panel_action(
                panel,
                DesignPanelAction::ExportConfigurationRemoveRequested {
                    target: expected_target,
                    configuration_id,
                },
            );
        }
        ExportEvent::ModeChange {
            expected_target,
            mode,
        } => {
            if target_is_current(panel, &expected_target) {
                panel.emit_export_mode_change(mode, cx);
            }
        }
        ExportEvent::AnimatedChange {
            expected_target,
            change,
            dismiss_overlay,
        } => {
            if !target_is_current(panel, &expected_target) {
                return;
            }
            if dismiss_overlay {
                panel.overlays.discard(DesignOpenOverlay::ExportChoice);
            }
            panel.emit_animated_export_change(change, cx);
            if dismiss_overlay {
                cx.notify();
            }
        }
        ExportEvent::AnimatedExport { expected_target } => {
            if target_is_current(panel, &expected_target) {
                panel.emit_animated_export(cx);
            }
        }
        ExportEvent::ExportAll { expected_target } => {
            if target_is_current(panel, &expected_target) {
                panel.emit_export_all(cx);
            }
        }
        ExportEvent::ToggleAdvanced { configuration_id } => {
            if panel
                .export_configurations()
                .iter()
                .any(|configuration| configuration.id == configuration_id)
            {
                panel.toggle_export_advanced(configuration_id, cx);
            }
        }
        ExportEvent::SetChoiceOverlay { overlay, open } => {
            panel.set_export_choice_overlay(overlay, open, cx);
        }
        ExportEvent::ToggleChoiceOverlay(overlay) => {
            let open = panel.overlays.export_choice_overlay().as_ref() != Some(&overlay);
            panel.set_export_choice_overlay(overlay, open, cx);
        }
        ExportEvent::TogglePreview { expected_target } => {
            if target_is_current(panel, &expected_target) {
                panel.toggle_export_preview(cx);
            }
        }
    }
}

#[derive(Clone)]
pub(in super::super) struct ExportEventSink {
    panel: Entity<DesignPanel>,
}

impl ExportEventSink {
    pub(in super::super) fn new(panel: Entity<DesignPanel>) -> Self {
        Self { panel }
    }

    fn dispatch(&self, event: ExportEvent, cx: &mut App) {
        self.panel
            .update(cx, |panel, cx| dispatch(panel, event, cx));
    }

    fn update_choice_overlay_from_popover(
        &self,
        overlay: SharedString,
        open: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.panel.update(cx, |panel, cx| {
            if open {
                panel.remember_overlay_focus_return(DesignOpenOverlay::ExportChoice, window, cx);
                dispatch(
                    panel,
                    ExportEvent::SetChoiceOverlay {
                        overlay,
                        open: true,
                    },
                    cx,
                );
            } else if panel.overlays.export_choice_overlay().as_ref() == Some(&overlay) {
                let _ = panel.dismiss_overlay_from_outside_click(
                    DesignOpenOverlay::ExportChoice,
                    window,
                    cx,
                );
            }
        });
    }
}

struct ExportRenderer<'a, C> {
    projection: &'a ExportProjection,
    chrome: &'a C,
    event_sink: &'a ExportEventSink,
    id: &'a SharedString,
    node_name: &'a SharedString,
}

impl<'a, C: ExportInspectorChrome> ExportRenderer<'a, C> {
    fn can_export(&self) -> bool {
        self.projection.target.can_export
    }

    fn export_configurations(&self) -> Vec<DesignExportConfiguration> {
        self.projection
            .static_export
            .rows
            .iter()
            .map(|row| row.configuration.clone())
            .collect()
    }

    fn export_mode(&self) -> DesignExportMode {
        self.projection.animated.mode
    }

    fn export_preview_available(&self) -> bool {
        self.projection.preview.available
    }

    fn render_value_cell(
        &self,
        id_suffix: impl Into<SharedString>,
        prefix: &'static str,
        value: impl Into<SharedString>,
        property: DesignPanelProperty,
        next: DesignPanelValue,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.chrome
            .export_value_cell(id_suffix, prefix, value, property, next, cx)
    }

    fn render_section(
        &self,
        _section: DesignPanelSection,
        collection: Option<DesignPanelCollection>,
        content: AnyElement,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.chrome.export_section(collection, content, cx)
    }

    fn render_remove_button(
        &self,
        id_suffix: impl Into<SharedString>,
        _collection: DesignPanelCollection,
        index: usize,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        let Some(row) = self.projection.static_export.rows.get(index) else {
            return div().size(px(24.)).flex_none().into_any_element();
        };
        if !self.projection.target.can_export || !self.projection.target.collection_supported {
            return div().size(px(24.)).flex_none().into_any_element();
        }
        let event_sink = self.event_sink.clone();
        let expected_target = self.projection.target.target.clone();
        let configuration_id = row.configuration.id.clone();
        div()
            .id(SharedString::from(format!(
                "{}-{}",
                self.id,
                id_suffix.into()
            )))
            .key_context(CONTROL_KEY_CONTEXT)
            .tab_index(0)
            .size(px(24.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .cursor_pointer()
            .hover(|style| style.bg(cx.theme().accent))
            .focus(|style| {
                style
                    .bg(cx.theme().accent)
                    .border_1()
                    .border_color(cx.theme().selection)
            })
            .on_activate(move |_, _, cx| {
                event_sink.dispatch(
                    ExportEvent::RemoveConfiguration {
                        expected_target: expected_target.clone(),
                        configuration_id: configuration_id.clone(),
                    },
                    cx,
                );
            })
            .child(Icon::new(IconName::Minus).xsmall())
            .into_any_element()
    }

    fn render_export_toggle(
        &self,
        id_suffix: impl Into<SharedString>,
        label: &'static str,
        checked: bool,
        configuration_id: SharedString,
        change: DesignExportConfigurationChange,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        let enabled = self.can_export();
        let event_sink = self.event_sink.clone();
        let expected_target = self.projection.target.target.clone();
        h_flex()
            .id(SharedString::from(format!(
                "{}-{}",
                self.id,
                id_suffix.into()
            )))
            .h(px(ROW_HEIGHT))
            .w_full()
            .justify_between()
            .rounded(px(5.))
            .when(enabled, |row| {
                row.key_context(CONTROL_KEY_CONTEXT)
                    .tab_index(0)
                    .cursor_pointer()
                    .hover(|style| style.bg(cx.theme().accent.opacity(0.55)))
                    .focus(|style| {
                        style
                            .bg(cx.theme().accent)
                            .border_1()
                            .border_color(cx.theme().selection)
                    })
            })
            .when(!enabled, |row| {
                row.text_color(cx.theme().muted_foreground).opacity(0.62)
            })
            .when(enabled, |row| {
                row.on_activate(move |_, _, cx| {
                    event_sink.dispatch(
                        ExportEvent::ConfigurationChange {
                            expected_target: expected_target.clone(),
                            configuration_id: configuration_id.clone(),
                            change: change.clone(),
                            phase: DesignPanelEditPhase::Commit,
                            dismiss_overlay: false,
                        },
                        cx,
                    );
                })
            })
            .child(div().text_xs().child(label))
            .child(
                h_flex()
                    .w(px(30.))
                    .h(px(18.))
                    .p(px(2.))
                    .justify_end()
                    .when(!checked, |toggle| toggle.justify_start())
                    .rounded(px(9.))
                    .bg(if checked {
                        cx.theme().selection
                    } else {
                        cx.theme().border
                    })
                    .child(
                        div()
                            .size(px(14.))
                            .rounded(px(7.))
                            .bg(cx.theme().background),
                    ),
            )
            .into_any_element()
    }

    fn render_export_choice(
        &self,
        id_suffix: impl Into<SharedString>,
        label: &'static str,
        value: impl Into<SharedString>,
        configuration_id: SharedString,
        options: Vec<(SharedString, DesignExportConfigurationChange)>,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        let enabled = self.can_export();
        let expected_target = self.projection.target.target.clone();
        let id_suffix = id_suffix.into();
        let value = value.into();
        let overlay = SharedString::from(format!("export-choice-{id_suffix}"));
        let overlay_for_open = overlay.clone();
        let sink_for_open = self.event_sink.clone();
        let sink_for_content = self.event_sink.clone();
        let panel_id = self.id.clone();
        let sink_for_keyboard = sink_for_open.clone();
        let overlay_for_keyboard = overlay.clone();
        let trigger = Button::new(SharedString::from(format!("{}-{id_suffix}", self.id)))
            .tooltip(label)
            .xsmall()
            .compact()
            .ghost()
            .w_full()
            .h(px(ROW_HEIGHT))
            .disabled(!enabled || options.is_empty())
            .on_keyboard_activate(move |_, cx| {
                sink_for_keyboard.dispatch(
                    ExportEvent::ToggleChoiceOverlay(overlay_for_keyboard.clone()),
                    cx,
                );
            })
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .child(div().text_xs().child(label))
                    .child(
                        h_flex()
                            .gap_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(value)
                            .child(Icon::new(IconName::ChevronRight).xsmall()),
                    ),
            );
        Popover::new(SharedString::from(format!("{}-{id_suffix}-menu", self.id)))
            .anchor(Anchor::TopRight)
            .open(self.projection.presentation.choice_overlay.as_ref() == Some(&overlay))
            .overlay_closable(true)
            .on_open_change(move |open, window, cx| {
                sink_for_open.update_choice_overlay_from_popover(
                    overlay_for_open.clone(),
                    *open,
                    window,
                    cx,
                );
            })
            .trigger(trigger)
            .content(move |_, window, cx| {
                let popover = cx.entity();
                v_flex().w(popup_width(window, 188.)).gap_1().children(
                    options.clone().into_iter().enumerate().map(
                        |(option_index, (option_label, change))| {
                            let event_sink = sink_for_content.clone();
                            let popover = popover.clone();
                            let configuration_id = configuration_id.clone();
                            let expected_target = expected_target.clone();
                            Button::new(SharedString::from(format!(
                                "{panel_id}-{id_suffix}-option-{option_index}"
                            )))
                            .label(option_label)
                            .xsmall()
                            .compact()
                            .ghost()
                            .w_full()
                            .on_activate(move |_, window, cx| {
                                event_sink.dispatch(
                                    ExportEvent::ConfigurationChange {
                                        expected_target: expected_target.clone(),
                                        configuration_id: configuration_id.clone(),
                                        change: change.clone(),
                                        phase: DesignPanelEditPhase::Commit,
                                        dismiss_overlay: true,
                                    },
                                    cx,
                                );
                                popover.update(cx, |popover, cx| {
                                    popover.dismiss(window, cx);
                                });
                            })
                        },
                    ),
                )
            })
            .into_any_element()
    }

    fn render_export_advanced(
        &self,
        index: usize,
        configuration: &DesignExportConfiguration,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        let configuration_id = configuration.id.clone();
        let capabilities = self.projection.static_export.capabilities;
        let mut controls = v_flex().w_full().gap_1().pt_1().child(
            self.render_export_choice(
                format!("export-color-profile-{index}"),
                "Color profile",
                configuration.common.color_profile.label(),
                configuration_id.clone(),
                DesignExportColorProfile::ALL
                    .into_iter()
                    .map(|profile| {
                        (
                            profile.label().into(),
                            DesignExportConfigurationChange::ColorProfile(profile),
                        )
                    })
                    .collect(),
                cx,
            ),
        );

        controls = match configuration.format_settings {
            DesignExportFormatSettings::Png(settings) => controls
                .child(self.render_export_toggle(
                    format!("export-ignore-overlap-{index}"),
                    "Ignore overlapping layers",
                    settings.ignore_overlapping_layers,
                    configuration_id.clone(),
                    DesignExportConfigurationChange::IgnoreOverlappingLayers(
                        !settings.ignore_overlapping_layers,
                    ),
                    cx,
                ))
                .when(capabilities.can_include_text_bounding_box, |controls| {
                    controls.child(self.render_export_toggle(
                        format!("export-bounds-{index}"),
                        "Include bounding box",
                        settings.include_text_bounding_box,
                        configuration_id.clone(),
                        DesignExportConfigurationChange::IncludeTextBoundingBox(
                            !settings.include_text_bounding_box,
                        ),
                        cx,
                    ))
                })
                .child(
                    self.render_export_choice(
                        format!("export-resampling-{index}"),
                        "Image resampling",
                        settings.resampling.label(),
                        configuration_id,
                        DesignExportImageResampling::ALL
                            .into_iter()
                            .map(|resampling| {
                                (
                                    resampling.label().into(),
                                    DesignExportConfigurationChange::Resampling(resampling),
                                )
                            })
                            .collect(),
                        cx,
                    ),
                ),
            DesignExportFormatSettings::Jpg(settings) => controls
                .child(self.render_export_toggle(
                    format!("export-ignore-overlap-{index}"),
                    "Ignore overlapping layers",
                    settings.ignore_overlapping_layers,
                    configuration_id.clone(),
                    DesignExportConfigurationChange::IgnoreOverlappingLayers(
                        !settings.ignore_overlapping_layers,
                    ),
                    cx,
                ))
                .when(capabilities.can_include_text_bounding_box, |controls| {
                    controls.child(self.render_export_toggle(
                        format!("export-bounds-{index}"),
                        "Include bounding box",
                        settings.include_text_bounding_box,
                        configuration_id.clone(),
                        DesignExportConfigurationChange::IncludeTextBoundingBox(
                            !settings.include_text_bounding_box,
                        ),
                        cx,
                    ))
                })
                .child(
                    self.render_export_choice(
                        format!("export-resampling-{index}"),
                        "Image resampling",
                        settings.resampling.label(),
                        configuration_id.clone(),
                        DesignExportImageResampling::ALL
                            .into_iter()
                            .map(|resampling| {
                                (
                                    resampling.label().into(),
                                    DesignExportConfigurationChange::Resampling(resampling),
                                )
                            })
                            .collect(),
                        cx,
                    ),
                )
                .child(
                    self.render_export_choice(
                        format!("export-quality-{index}"),
                        "Image quality",
                        settings.quality.label(),
                        configuration_id,
                        DesignExportImageQuality::ALL
                            .into_iter()
                            .map(|quality| {
                                (
                                    quality.label().into(),
                                    DesignExportConfigurationChange::Quality(quality),
                                )
                            })
                            .collect(),
                        cx,
                    ),
                ),
            DesignExportFormatSettings::Svg(settings) => controls
                .child(self.render_export_toggle(
                    format!("export-ignore-overlap-{index}"),
                    "Ignore overlapping layers",
                    settings.ignore_overlapping_layers,
                    configuration_id.clone(),
                    DesignExportConfigurationChange::IgnoreOverlappingLayers(
                        !settings.ignore_overlapping_layers,
                    ),
                    cx,
                ))
                .when(capabilities.can_include_svg_bounds, |controls| {
                    controls.child(self.render_export_toggle(
                        format!("export-include-bounds-{index}"),
                        "Include bounding box",
                        settings.include_bounds,
                        configuration_id.clone(),
                        DesignExportConfigurationChange::IncludeBounds(!settings.include_bounds),
                        cx,
                    ))
                })
                .child(self.render_export_toggle(
                    format!("export-include-id-{index}"),
                    "Include \"id\" attribute",
                    settings.include_id_attribute,
                    configuration_id.clone(),
                    DesignExportConfigurationChange::IncludeIdAttribute(
                        !settings.include_id_attribute,
                    ),
                    cx,
                ))
                .child(self.render_export_toggle(
                    format!("export-outline-text-{index}"),
                    "Outline text",
                    settings.outline_text,
                    configuration_id.clone(),
                    DesignExportConfigurationChange::OutlineText(!settings.outline_text),
                    cx,
                ))
                .child(self.render_export_toggle(
                    format!("export-simplify-stroke-{index}"),
                    "Simplify stroke",
                    settings.simplify_stroke,
                    configuration_id,
                    DesignExportConfigurationChange::SimplifyStroke(!settings.simplify_stroke),
                    cx,
                )),
            DesignExportFormatSettings::Pdf(settings) => controls
                .child(
                    self.render_export_choice(
                        format!("export-resampling-{index}"),
                        "Image resampling",
                        settings.resampling.label(),
                        configuration_id.clone(),
                        DesignExportImageResampling::ALL
                            .into_iter()
                            .map(|resampling| {
                                (
                                    resampling.label().into(),
                                    DesignExportConfigurationChange::Resampling(resampling),
                                )
                            })
                            .collect(),
                        cx,
                    ),
                )
                .child(
                    self.render_export_choice(
                        format!("export-quality-{index}"),
                        "Image quality",
                        settings.quality.label(),
                        configuration_id,
                        DesignExportImageQuality::ALL
                            .into_iter()
                            .map(|quality| {
                                (
                                    quality.label().into(),
                                    DesignExportConfigurationChange::Quality(quality),
                                )
                            })
                            .collect(),
                        cx,
                    ),
                ),
        };
        controls.into_any_element()
    }

    fn render_export_mode_switch(&self, cx: &mut Context<DesignPanel>) -> Option<AnyElement> {
        self.projection.animated.view_data.as_ref()?;
        let event_sink = self.event_sink.clone();
        let expected_target = self.projection.target.target.clone();
        Some(
            h_flex()
                .w_full()
                .p(px(2.))
                .gap_1()
                .rounded(px(6.))
                .bg(cx.theme().secondary)
                .children(DesignExportMode::ALL.into_iter().map(|mode| {
                    let event_sink = event_sink.clone();
                    let expected_target = expected_target.clone();
                    Button::new(SharedString::from(format!(
                        "{}-export-mode-{}",
                        self.id,
                        mode.label().to_ascii_lowercase()
                    )))
                    .label(mode.label())
                    .xsmall()
                    .compact()
                    .ghost()
                    .flex_1()
                    .selected(mode == self.projection.animated.mode)
                    .disabled(!self.can_export())
                    .on_activate(move |_, _, cx| {
                        let expected_target = expected_target.clone();
                        event_sink.dispatch(
                            ExportEvent::ModeChange {
                                expected_target,
                                mode,
                            },
                            cx,
                        );
                    })
                }))
                .into_any_element(),
        )
    }

    fn render_animated_export_choice(
        &self,
        id_suffix: impl Into<SharedString>,
        label: impl Into<SharedString>,
        value: impl Into<SharedString>,
        options: Vec<(SharedString, DesignAnimatedExportChange)>,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        let id_suffix = id_suffix.into();
        let label = label.into();
        let value = value.into();
        let overlay = SharedString::from(format!("animated-export-choice-{id_suffix}"));
        let overlay_for_open = overlay.clone();
        let expected_target = self.projection.target.target.clone();
        let sink_for_open = self.event_sink.clone();
        let sink_for_content = self.event_sink.clone();
        let panel_id = self.id.clone();
        let sink_for_keyboard = sink_for_open.clone();
        let overlay_for_keyboard = overlay.clone();
        let trigger = Button::new(SharedString::from(format!("{}-{id_suffix}", self.id)))
            .tooltip(label.clone())
            .xsmall()
            .compact()
            .ghost()
            .w_full()
            .h(px(ROW_HEIGHT))
            .disabled(!self.can_export() || options.is_empty())
            .on_keyboard_activate(move |_, cx| {
                sink_for_keyboard.dispatch(
                    ExportEvent::ToggleChoiceOverlay(overlay_for_keyboard.clone()),
                    cx,
                );
            })
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .child(div().text_xs().child(label))
                    .child(
                        h_flex()
                            .gap_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(value)
                            .child(Icon::new(IconName::ChevronRight).xsmall()),
                    ),
            );
        Popover::new(SharedString::from(format!("{}-{id_suffix}-menu", self.id)))
            .anchor(Anchor::TopRight)
            .open(self.projection.presentation.choice_overlay.as_ref() == Some(&overlay))
            .overlay_closable(true)
            .on_open_change(move |open, window, cx| {
                sink_for_open.update_choice_overlay_from_popover(
                    overlay_for_open.clone(),
                    *open,
                    window,
                    cx,
                );
            })
            .trigger(trigger)
            .content(move |_, window, cx| {
                let popover = cx.entity();
                v_flex().w(popup_width(window, 188.)).gap_1().children(
                    options.clone().into_iter().enumerate().map(
                        |(option_index, (option_label, change))| {
                            let event_sink = sink_for_content.clone();
                            let popover = popover.clone();
                            let expected_target = expected_target.clone();
                            Button::new(SharedString::from(format!(
                                "{panel_id}-{id_suffix}-option-{option_index}"
                            )))
                            .label(option_label)
                            .xsmall()
                            .compact()
                            .ghost()
                            .w_full()
                            .on_activate(move |_, window, cx| {
                                event_sink.dispatch(
                                    ExportEvent::AnimatedChange {
                                        expected_target: expected_target.clone(),
                                        change: change.clone(),
                                        dismiss_overlay: true,
                                    },
                                    cx,
                                );
                                popover.update(cx, |popover, cx| {
                                    popover.dismiss(window, cx);
                                });
                            })
                        },
                    ),
                )
            })
            .into_any_element()
    }

    fn render_animated_export(&self, cx: &mut Context<DesignPanel>) -> AnyElement {
        let Some(animated) = self.projection.animated.view_data.as_ref() else {
            return div().into_any_element();
        };
        let settings = &animated.settings;
        let capability = &animated.capability;
        let mut content = v_flex().w_full().gap_2();
        content = content.child(
            self.render_animated_export_choice(
                "animated-export-format",
                "Format",
                settings.format().label(),
                capability
                    .available_formats
                    .iter()
                    .copied()
                    .map(|format| {
                        (
                            format.label().into(),
                            DesignAnimatedExportChange::Format(format),
                        )
                    })
                    .collect(),
                cx,
            ),
        );

        match settings {
            DesignAnimatedExportSettings::Mp4 {
                sizing,
                fps,
                quality,
            }
            | DesignAnimatedExportSettings::WebM {
                sizing,
                fps,
                quality,
            } => {
                content = content
                    .child(
                        self.render_animated_export_choice(
                            "animated-export-size",
                            "Size",
                            sizing.to_string(),
                            [0.5, 0.75, 1., 1.5, 2., 3., 4.]
                                .into_iter()
                                .map(|scale| {
                                    let sizing = DesignExportSizing::Scale(scale);
                                    (
                                        sizing.to_string().into(),
                                        DesignAnimatedExportChange::Sizing(sizing),
                                    )
                                })
                                .collect(),
                            cx,
                        ),
                    )
                    .child(
                        self.render_animated_export_choice(
                            "animated-export-fps",
                            "Frame rate",
                            fps.label(),
                            DesignVideoExportFps::ALL
                                .into_iter()
                                .map(|fps| (fps.label(), DesignAnimatedExportChange::VideoFps(fps)))
                                .collect(),
                            cx,
                        ),
                    )
                    .child(
                        self.render_animated_export_choice(
                            "animated-export-quality",
                            "Quality",
                            quality.label(),
                            DesignExportImageQuality::ALL
                                .into_iter()
                                .map(|quality| {
                                    (
                                        quality.label().into(),
                                        DesignAnimatedExportChange::Quality(quality),
                                    )
                                })
                                .collect(),
                            cx,
                        ),
                    );
            }
            DesignAnimatedExportSettings::Gif {
                sizing,
                fps,
                loop_count,
            } => {
                let current_loop_count = *loop_count;
                content = content
                    .child(
                        self.render_animated_export_choice(
                            "animated-export-size",
                            "Size",
                            sizing.to_string(),
                            [0.5, 0.75, 1., 1.5, 2., 3., 4.]
                                .into_iter()
                                .map(|scale| {
                                    let sizing = DesignExportSizing::Scale(scale);
                                    (
                                        sizing.to_string().into(),
                                        DesignAnimatedExportChange::Sizing(sizing),
                                    )
                                })
                                .collect(),
                            cx,
                        ),
                    )
                    .child(
                        self.render_animated_export_choice(
                            "animated-export-fps",
                            "Frame rate",
                            fps.label(),
                            DesignGifExportFps::ALL
                                .into_iter()
                                .map(|fps| (fps.label(), DesignAnimatedExportChange::GifFps(fps)))
                                .collect(),
                            cx,
                        ),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .h(px(ROW_HEIGHT))
                            .justify_between()
                            .child(div().text_xs().child("Loop count"))
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child({
                                        let event_sink = self.event_sink.clone();
                                        let expected_target =
                                            self.projection.target.target.clone();
                                        Button::new(SharedString::from(format!(
                                            "{}-animated-loop-decrement",
                                            self.id
                                        )))
                                        .icon(IconName::Minus)
                                        .xsmall()
                                        .compact()
                                        .ghost()
                                        .disabled(!self.can_export() || current_loop_count == 0)
                                        .on_activate(
                                            move |_, _, cx| {
                                                event_sink.dispatch(
                                                    ExportEvent::AnimatedChange {
                                                        expected_target: expected_target.clone(),
                                                        change: DesignAnimatedExportChange::GifLoopCount(
                                                            current_loop_count.saturating_sub(1),
                                                        ),
                                                        dismiss_overlay: false,
                                                    },
                                                    cx,
                                                );
                                            },
                                        )
                                    })
                                    .child(
                                        div()
                                            .w(px(56.))
                                            .h(px(ROW_HEIGHT))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded(px(4.))
                                            .bg(cx.theme().secondary)
                                            .text_xs()
                                            .child(if current_loop_count == 0 {
                                                SharedString::from("Forever")
                                            } else {
                                                SharedString::from(current_loop_count.to_string())
                                            }),
                                    )
                                    .child({
                                        let event_sink = self.event_sink.clone();
                                        let expected_target =
                                            self.projection.target.target.clone();
                                        Button::new(SharedString::from(format!(
                                            "{}-animated-loop-increment",
                                            self.id
                                        )))
                                        .icon(IconName::Plus)
                                        .xsmall()
                                        .compact()
                                        .ghost()
                                        .disabled(!self.can_export() || current_loop_count >= 1000)
                                        .on_activate(
                                            move |_, _, cx| {
                                                event_sink.dispatch(
                                                    ExportEvent::AnimatedChange {
                                                        expected_target: expected_target.clone(),
                                                        change: DesignAnimatedExportChange::GifLoopCount(
                                                            current_loop_count.saturating_add(1),
                                                        ),
                                                        dismiss_overlay: false,
                                                    },
                                                    cx,
                                                );
                                            },
                                        )
                                    }),
                            ),
                    );
            }
            DesignAnimatedExportSettings::Svg { options } => {
                for option in options {
                    content = content.child(
                        self.render_animated_export_choice(
                            SharedString::from(format!("animated-svg-option-{}", option.id)),
                            option.label.clone(),
                            option.selected.clone(),
                            option
                                .choices
                                .iter()
                                .cloned()
                                .map(|value| {
                                    (
                                        value.clone(),
                                        DesignAnimatedExportChange::SvgOption {
                                            option_id: option.id.clone(),
                                            value,
                                        },
                                    )
                                })
                                .collect(),
                            cx,
                        ),
                    );
                }
                if options.is_empty() {
                    content = content.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Animated SVG settings are supplied by the host."),
                    );
                }
            }
        }

        let reason = capability
            .reason_for(settings)
            .map(|reason| SharedString::from(reason.to_owned()));
        let enabled = self.can_export() && reason.is_none();
        content
            .when_some(reason, |content, reason| {
                content.child(
                    div()
                        .px_2()
                        .py_1()
                        .rounded(px(5.))
                        .bg(cx.theme().secondary)
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(reason),
                )
            })
            .child({
                let event_sink = self.event_sink.clone();
                let expected_target = self.projection.target.target.clone();
                Button::new(SharedString::from(format!("{}-animated-export", self.id)))
                    .label(format!(
                        "Export {} as {}",
                        self.node_name,
                        settings.format().label()
                    ))
                    .xsmall()
                    .compact()
                    .w_full()
                    .disabled(!enabled)
                    .on_activate(move |_, _, cx| {
                        event_sink.dispatch(
                            ExportEvent::AnimatedExport {
                                expected_target: expected_target.clone(),
                            },
                            cx,
                        );
                    })
            })
            .into_any_element()
    }

    fn render_export_preview_state(&self, cx: &mut Context<DesignPanel>) -> AnyElement {
        let state = self.projection.preview.state.as_ref();
        match state {
            Some(DesignExportPreviewState::Idle) => div()
                .px_2()
                .pb_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Open Preview to request a host-rendered thumbnail.")
                .into_any_element(),
            Some(DesignExportPreviewState::Loading) => div()
                .px_2()
                .pb_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Rendering preview…")
                .into_any_element(),
            Some(DesignExportPreviewState::Ready(preview)) => v_flex()
                .px_2()
                .pb_2()
                .gap_1()
                .child(
                    div()
                        .h(px(96.))
                        .w_full()
                        .rounded(px(5.))
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().secondary)
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(preview.thumbnail_id.clone().unwrap_or_else(|| {
                            SharedString::from("Host-rendered export thumbnail")
                        })),
                )
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!(
                            "{} × {} px",
                            preview.pixel_width, preview.pixel_height
                        ))
                        .when_some(preview.estimated_output.clone(), |row, output| {
                            row.child(output)
                        }),
                )
                .into_any_element(),
            Some(DesignExportPreviewState::Error { message }) => div()
                .px_2()
                .pb_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(message.clone())
                .into_any_element(),
            None => div().into_any_element(),
        }
    }

    fn render_export(&self, cx: &mut Context<DesignPanel>) -> AnyElement {
        let configurations = self.export_configurations();
        let mode_switch = self.render_export_mode_switch(cx);
        if self.export_mode() == DesignExportMode::Animated {
            let mut content = v_flex().px(px(PANEL_PADDING)).pb_4().gap_2();
            if let Some(mode_switch) = mode_switch {
                content = content.child(mode_switch);
            }
            content = content.child(self.render_animated_export(cx));
            return self.render_section(
                DesignPanelSection::Export,
                None,
                content.into_any_element(),
                cx,
            );
        }
        if configurations.is_empty() {
            let mut content = v_flex().px(px(PANEL_PADDING)).pb_4().gap_2();
            if let Some(mode_switch) = mode_switch {
                content = content.child(mode_switch);
            }
            return self.render_section(
                DesignPanelSection::Export,
                Some(DesignPanelCollection::Export),
                content.into_any_element(),
                cx,
            );
        }
        let mut content = v_flex().px(px(PANEL_PADDING)).pb_4().gap_2();
        if let Some(mode_switch) = mode_switch {
            content = content.child(mode_switch);
        }
        for row_projection in &self.projection.static_export.rows {
            let index = row_projection.index;
            let configuration = &row_projection.configuration;
            let configuration_id = configuration.id.clone();
            let expanded = row_projection.expanded;
            let event_sink = self.event_sink.clone();
            let suffix = if configuration.common.suffix.is_empty() {
                SharedString::from("No suffix")
            } else {
                configuration.common.suffix.clone()
            };
            let mut row = v_flex().w_full().gap_1().child(
                h_flex()
                    .h(px(ROW_HEIGHT))
                    .gap_1()
                    .child(self.render_value_cell(
                        format!("export-sizing-{index}"),
                        "Size",
                        configuration.sizing.to_string(),
                        DesignPanelProperty::ExportSizing(index),
                        DesignPanelValue::ExportSizing(configuration.sizing),
                        cx,
                    ))
                    .child(self.render_value_cell(
                        format!("export-format-{index}"),
                        "Format",
                        configuration.format().label(),
                        DesignPanelProperty::ExportFormat(index),
                        DesignPanelValue::ExportFormat(configuration.format()),
                        cx,
                    ))
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "{}-export-advanced-{index}",
                                self.id
                            )))
                            .key_context(CONTROL_KEY_CONTEXT)
                            .tab_index(0)
                            .size(px(24.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .hover(|style| style.bg(cx.theme().accent))
                            .focus(|style| {
                                style
                                    .bg(cx.theme().accent)
                                    .border_1()
                                    .border_color(cx.theme().selection)
                            })
                            .on_activate(move |_, _, cx| {
                                event_sink.dispatch(
                                    ExportEvent::ToggleAdvanced {
                                        configuration_id: configuration_id.clone(),
                                    },
                                    cx,
                                );
                            })
                            .child(
                                Icon::new(if expanded {
                                    IconName::ChevronUp
                                } else {
                                    IconName::Ellipsis
                                })
                                .xsmall(),
                            ),
                    )
                    .child(self.render_remove_button(
                        format!("remove-export-{index}"),
                        DesignPanelCollection::Export,
                        index,
                        cx,
                    )),
            );
            if expanded {
                row = row
                    .child(self.render_value_cell(
                        format!("export-suffix-{index}"),
                        "S",
                        suffix,
                        DesignPanelProperty::ExportSuffix(index),
                        DesignPanelValue::Text(configuration.common.suffix.clone()),
                        cx,
                    ))
                    .child(self.render_export_advanced(index, configuration, cx));
            }
            content = content.child(row);
        }

        let enabled = self.can_export()
            && self.projection.target.collection_supported
            && !configurations.is_empty();
        let export_all_sink = self.event_sink.clone();
        let export_all_target = self.projection.target.target.clone();
        content = content.child(
            div()
                .id(SharedString::from(format!("{}-export-all", self.id)))
                .h(px(ROW_HEIGHT))
                .w_full()
                .px_2()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.))
                .border_1()
                .border_color(cx.theme().border)
                .text_xs()
                .when(enabled, |button| {
                    button
                        .key_context(CONTROL_KEY_CONTEXT)
                        .tab_index(0)
                        .cursor_pointer()
                        .hover(|style| style.bg(cx.theme().accent))
                        .focus(|style| {
                            style
                                .bg(cx.theme().accent)
                                .border_color(cx.theme().selection)
                        })
                })
                .when(!enabled, |button| {
                    button.text_color(cx.theme().muted_foreground).opacity(0.62)
                })
                .when(enabled, |button| {
                    button.on_activate(move |_, _, cx| {
                        export_all_sink.dispatch(
                            ExportEvent::ExportAll {
                                expected_target: export_all_target.clone(),
                            },
                            cx,
                        );
                    })
                })
                .child(format!("Export {}", self.node_name)),
        );

        if self.export_preview_available() {
            let preview_sink = self.event_sink.clone();
            let preview_target = self.projection.target.target.clone();
            content = content.child(
                v_flex()
                    .w_full()
                    .child(
                        h_flex()
                            .id(SharedString::from(format!("{}-export-preview", self.id)))
                            .key_context(CONTROL_KEY_CONTEXT)
                            .tab_index(0)
                            .h(px(ROW_HEIGHT))
                            .w_full()
                            .px_2()
                            .gap_2()
                            .cursor_pointer()
                            .hover(|style| style.bg(cx.theme().accent))
                            .focus(|style| {
                                style
                                    .bg(cx.theme().accent)
                                    .border_color(cx.theme().selection)
                            })
                            .on_activate(move |_, _, cx| {
                                preview_sink.dispatch(
                                    ExportEvent::TogglePreview {
                                        expected_target: preview_target.clone(),
                                    },
                                    cx,
                                );
                            })
                            .child(
                                Icon::new(if self.projection.preview.expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .xsmall(),
                            )
                            .child(div().text_xs().child("Preview")),
                    )
                    .when(self.projection.preview.expanded, |preview| {
                        preview.child(self.render_export_preview_state(cx))
                    }),
            );
        }

        self.render_section(
            DesignPanelSection::Export,
            Some(DesignPanelCollection::Export),
            content.into_any_element(),
            cx,
        )
    }
}

pub(in super::super) fn render(
    projection: &ExportProjection,
    chrome: &impl ExportInspectorChrome,
    event_sink: &ExportEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    ExportRenderer {
        projection,
        chrome,
        event_sink,
        id: &projection.target.panel_id,
        node_name: &projection.target.node_name,
    }
    .render_export(cx)
}

#[cfg(test)]
mod projection_tests {
    use super::*;

    #[test]
    fn static_projection_preserves_stable_ids_and_local_disclosures() {
        let first = DesignExportConfiguration::new("first", DesignExportFormat::Png);
        let second = DesignExportConfiguration::new("second", DesignExportFormat::Svg);
        let expanded = HashSet::from([SharedString::from("second")]);

        let projection = ExportStaticProjection::new(
            vec![first, second],
            DesignStaticExportCapabilities::default(),
            &expanded,
        );

        assert_eq!(projection.rows[0].index, 0);
        assert_eq!(projection.rows[0].configuration.id.as_ref(), "first");
        assert!(!projection.rows[0].expanded);
        assert_eq!(projection.rows[1].index, 1);
        assert_eq!(projection.rows[1].configuration.id.as_ref(), "second");
        assert!(projection.rows[1].expanded);
    }

    #[test]
    fn preview_projection_keeps_host_and_local_state_separate() {
        let projection =
            ExportPreviewProjection::new(true, true, Some(DesignExportPreviewState::Loading));

        assert!(projection.available);
        assert!(projection.expanded);
        assert!(matches!(
            projection.state,
            Some(DesignExportPreviewState::Loading)
        ));
    }
}
