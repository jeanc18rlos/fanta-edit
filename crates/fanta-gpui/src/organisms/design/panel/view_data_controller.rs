//! Canonical host snapshot application and reconciliation for DesignPanel.
//!
//! The public façade keeps source-compatible setters and selectors. This
//! controller is the only implementation boundary that projects host data
//! into retained panel state.

use super::*;

pub(super) trait DesignPanelViewDataController: Sized {
    fn canonical_view_data(&self) -> DesignPanelViewData;

    fn apply_view_data(&mut self, view_data: DesignPanelViewData, cx: &mut Context<Self>);

    fn apply_node(&mut self, node: DesignPanelNode, cx: &mut Context<Self>);

    fn apply_additional_labels(&mut self, enabled: bool, cx: &mut Context<Self>);

    fn apply_nudge_settings(&mut self, settings: DesignNudgeSettings, cx: &mut Context<Self>);

    fn apply_workspace_mode(
        &mut self,
        workspace_mode: DesignPanelWorkspaceMode,
        cx: &mut Context<Self>,
    );

    fn apply_active_surface(&mut self, surface: DesignPanelSurface, cx: &mut Context<Self>)
    -> bool;

    fn apply_inspection_context(
        &mut self,
        context: DesignPanelInspectionContext,
        cx: &mut Context<Self>,
    );

    fn apply_export_view_data(&mut self, view_data: DesignExportViewData, cx: &mut Context<Self>);

    fn apply_clear_export_view_data(&mut self, cx: &mut Context<Self>);

    fn apply_color_style_view_data(
        &mut self,
        view_data: DesignColorStyleViewData,
        cx: &mut Context<Self>,
    );

    fn apply_color_style_sample_view_data(
        &mut self,
        view_data: DesignColorStyleSampleViewData,
        cx: &mut Context<Self>,
    );

    fn apply_color_contrast_view_data(
        &mut self,
        view_data: DesignColorContrastViewData,
        cx: &mut Context<Self>,
    );

    fn apply_paint_variable_view_data(
        &mut self,
        view_data: DesignPaintVariableViewData,
        cx: &mut Context<Self>,
    );

    fn apply_paint_style_view_data(
        &mut self,
        view_data: DesignPaintStyleViewData,
        cx: &mut Context<Self>,
    );

    fn apply_media_paint_view_data(
        &mut self,
        view_data: DesignMediaPaintViewData,
        cx: &mut Context<Self>,
    );

    fn apply_shader_view_data(&mut self, view_data: DesignShaderViewData, cx: &mut Context<Self>);

    fn apply_typography_style_view_data(
        &mut self,
        view_data: DesignTypographyStyleViewData,
        cx: &mut Context<Self>,
    );

    fn apply_font_view_data(&mut self, view_data: DesignFontViewData, cx: &mut Context<Self>);

    fn apply_effect_style_view_data(
        &mut self,
        view_data: DesignEffectStyleViewData,
        cx: &mut Context<Self>,
    );

    fn apply_effect_variable_view_data(
        &mut self,
        view_data: DesignEffectVariableViewData,
        cx: &mut Context<Self>,
    );

    fn apply_property_variable_view_data(
        &mut self,
        view_data: DesignVariableViewData,
        cx: &mut Context<Self>,
    );

    fn apply_component_swap_view_data(
        &mut self,
        view_data: DesignComponentSwapViewData,
        cx: &mut Context<Self>,
    );

    fn apply_layout_grid_style_view_data(
        &mut self,
        view_data: DesignLayoutGridStyleViewData,
        cx: &mut Context<Self>,
    );

    fn apply_layout_grid_variable_view_data(
        &mut self,
        view_data: DesignLayoutGridVariableViewData,
        cx: &mut Context<Self>,
    );

    fn apply_layout_grid_count_variable_view_data(
        &mut self,
        view_data: DesignLayoutGridCountVariableViewData,
        cx: &mut Context<Self>,
    );

    fn apply_add_auto_layout_view_data(
        &mut self,
        view_data: DesignAddAutoLayoutViewData,
        cx: &mut Context<Self>,
    );

    fn apply_clear_add_auto_layout_view_data(&mut self, cx: &mut Context<Self>);

    fn apply_draw_appearance_view_data(
        &mut self,
        view_data: DesignDrawAppearanceViewData,
        cx: &mut Context<Self>,
    ) -> bool;

    fn apply_clear_draw_appearance_view_data(&mut self, cx: &mut Context<Self>);

    fn apply_frame_preset_view_data(
        &mut self,
        view_data: DesignFramePresetViewData,
        cx: &mut Context<Self>,
    );

    fn apply_clear_frame_preset_view_data(&mut self, cx: &mut Context<Self>);

    fn apply_smart_selection_view_data(
        &mut self,
        view_data: DesignSmartSelectionViewData,
        cx: &mut Context<Self>,
    );

    fn apply_clear_smart_selection_view_data(&mut self, cx: &mut Context<Self>);

    fn apply_page_view_data(&mut self, view_data: DesignPageViewData, cx: &mut Context<Self>);

    fn apply_clear_page_view_data(&mut self, cx: &mut Context<Self>);

    fn apply_page_local_styles_view_data(
        &mut self,
        view_data: DesignPageLocalStylesViewData,
        cx: &mut Context<Self>,
    );

    fn apply_clear_page_local_styles_view_data(&mut self, cx: &mut Context<Self>);

    fn apply_variables_entry_point(
        &mut self,
        entry_point: DesignVariablesEntryPoint,
        cx: &mut Context<Self>,
    );

    fn apply_variable_mode_view_data(
        &mut self,
        view_data: DesignVariableModeViewData,
        cx: &mut Context<Self>,
    );

    fn apply_clear_variable_mode_view_data(&mut self, cx: &mut Context<Self>);

    fn apply_viewer_properties_view_data(
        &mut self,
        view_data: DesignViewerPropertiesViewData,
        cx: &mut Context<Self>,
    );

    fn apply_clear_viewer_properties_view_data(&mut self, cx: &mut Context<Self>);

    fn apply_selection_header_view_data(
        &mut self,
        view_data: DesignSelectionHeaderViewData,
        cx: &mut Context<Self>,
    );

    fn apply_selection_header_view_data_for_target(
        &mut self,
        target: DesignPanelTarget,
        view_data: DesignSelectionHeaderViewData,
        cx: &mut Context<Self>,
    );

    fn apply_clear_selection_header_view_data(&mut self, cx: &mut Context<Self>);

    fn apply_property_value_states(
        &mut self,
        states: impl IntoIterator<
            Item = (
                DesignPanelProperty,
                DesignPanelPropertyValueState<DesignPanelValue>,
            ),
        >,
        cx: &mut Context<Self>,
    );

    fn apply_property_value_state(
        &mut self,
        property: DesignPanelProperty,
        state: DesignPanelPropertyValueState<DesignPanelValue>,
        cx: &mut Context<Self>,
    );
}

#[allow(deprecated)]
impl DesignPanelViewDataController for DesignPanel {
    fn canonical_view_data(&self) -> DesignPanelViewData {
        DesignPanelViewData {
            inspection_context: self.host.inspection_context.clone(),
            page_node_fallback: self.host.page_node_fallback.clone(),
            navigation: DesignPanelNavigationViewData::new(
                self.host.navigation.editor_surface,
                self.host.navigation.viewer_surface,
                self.host.navigation.workspace_mode,
            ),
            preferences: DesignPanelPreferencesViewData {
                additional_labels: self.preferences.additional_labels,
                nudge_settings: self.preferences.nudge_settings,
                variables_entry_point: self.preferences.variables_entry_point.clone(),
            },
            projections: DesignPanelProjectionViewData {
                export: self.host.projections.export.clone(),
                add_auto_layout: self.host.projections.add_auto_layout.clone(),
                draw_appearance: self.host.projections.draw_appearance.clone(),
                frame_presets: self.host.projections.frame_presets.clone(),
                smart_selection: self.host.projections.smart_selection.clone(),
                page: self.host.projections.page.clone(),
                page_local_styles: self.host.projections.page_local_styles.clone(),
                variable_modes: self.host.projections.variable_modes.clone(),
                viewer_properties: self.host.projections.viewer_properties.clone(),
                selection_header: self
                    .host
                    .projections
                    .selection_header
                    .clone()
                    .zip(self.host.projections.selection_header_target.clone())
                    .map(|(view_data, target)| {
                        DesignPanelTargetedSelectionHeader::new(target, view_data)
                    }),
            },
            resources: DesignPanelResourcesViewData {
                color_styles: self.resources.color_styles.clone(),
                color_style_samples: self.resources.color_style_samples.clone(),
                color_contrast: self.resources.color_contrast.clone(),
                paint_variables: self.resources.paint_variables.clone(),
                paint_styles: self.resources.paint_styles.clone(),
                media_paints: self.resources.media_paints.clone(),
                shaders: self.resources.shaders.clone(),
                typography_styles: self.resources.typography_styles.clone(),
                fonts: self.resources.fonts.clone(),
                effect_styles: self.resources.effect_styles.clone(),
                effect_variables: self.resources.effect_variables.clone(),
                property_variables: self.resources.property_variables.clone(),
                component_swaps: self.resources.component_swaps.clone(),
                layout_grid_styles: self.resources.layout_grid_styles.clone(),
                layout_grid_variables: self.resources.layout_grid_variables.clone(),
                layout_grid_count_variables: self.resources.layout_grid_count_variables.clone(),
            },
            property_states: self.host.property_states.clone(),
        }
    }

    fn apply_view_data(&mut self, view_data: DesignPanelViewData, cx: &mut Context<Self>) {
        let DesignPanelViewData {
            inspection_context,
            page_node_fallback,
            navigation,
            preferences,
            projections,
            resources,
            property_states,
        } = view_data;
        let navigation = navigation.normalized();

        self.apply_inspection_context(inspection_context, cx);
        if self.host.page_node_fallback != page_node_fallback {
            self.host.page_node_fallback = page_node_fallback;
            cx.notify();
        }

        let requested_surface =
            navigation.active_surface(self.host.inspection_context.permissions().can_edit());
        let _ = self.apply_active_surface(requested_surface, cx);
        let retained_surface_changed = self.host.navigation.editor_surface
            != navigation.editor_surface
            || self.host.navigation.viewer_surface != navigation.viewer_surface;
        self.host.navigation.editor_surface = navigation.editor_surface;
        self.host.navigation.viewer_surface = navigation.viewer_surface;
        if retained_surface_changed {
            cx.notify();
        }
        self.apply_workspace_mode(navigation.workspace_mode, cx);

        self.apply_additional_labels(preferences.additional_labels, cx);
        self.apply_nudge_settings(preferences.nudge_settings, cx);
        self.apply_variables_entry_point(preferences.variables_entry_point, cx);

        self.apply_color_style_view_data(resources.color_styles, cx);
        self.apply_color_style_sample_view_data(resources.color_style_samples, cx);
        self.apply_color_contrast_view_data(resources.color_contrast, cx);
        self.apply_paint_variable_view_data(resources.paint_variables, cx);
        self.apply_paint_style_view_data(resources.paint_styles, cx);
        self.apply_media_paint_view_data(resources.media_paints, cx);
        self.apply_shader_view_data(resources.shaders, cx);
        self.apply_typography_style_view_data(resources.typography_styles, cx);
        self.apply_font_view_data(resources.fonts, cx);
        self.apply_effect_style_view_data(resources.effect_styles, cx);
        self.apply_effect_variable_view_data(resources.effect_variables, cx);
        self.apply_property_variable_view_data(resources.property_variables, cx);
        self.apply_component_swap_view_data(resources.component_swaps, cx);
        self.apply_layout_grid_style_view_data(resources.layout_grid_styles, cx);
        self.apply_layout_grid_count_variable_view_data(resources.layout_grid_count_variables, cx);
        self.apply_layout_grid_variable_view_data(resources.layout_grid_variables, cx);

        match projections.export {
            Some(view_data) => self.apply_export_view_data(view_data, cx),
            None => self.apply_clear_export_view_data(cx),
        }
        match projections.add_auto_layout {
            Some(view_data) => self.apply_add_auto_layout_view_data(view_data, cx),
            None => self.apply_clear_add_auto_layout_view_data(cx),
        }
        match projections.draw_appearance {
            Some(view_data) => {
                let _ = self.apply_draw_appearance_view_data(view_data, cx);
            }
            None => self.apply_clear_draw_appearance_view_data(cx),
        }
        match projections.frame_presets {
            Some(view_data) => self.apply_frame_preset_view_data(view_data, cx),
            None => self.apply_clear_frame_preset_view_data(cx),
        }
        match projections.smart_selection {
            Some(view_data) => self.apply_smart_selection_view_data(view_data, cx),
            None => self.apply_clear_smart_selection_view_data(cx),
        }
        match projections.page {
            Some(view_data) => self.apply_page_view_data(view_data, cx),
            None => self.apply_clear_page_view_data(cx),
        }
        match projections.page_local_styles {
            Some(view_data) => self.apply_page_local_styles_view_data(view_data, cx),
            None => self.apply_clear_page_local_styles_view_data(cx),
        }
        match projections.variable_modes {
            Some(view_data) => self.apply_variable_mode_view_data(view_data, cx),
            None => self.apply_clear_variable_mode_view_data(cx),
        }
        match projections.viewer_properties {
            Some(view_data) => self.apply_viewer_properties_view_data(view_data, cx),
            None => self.apply_clear_viewer_properties_view_data(cx),
        }
        match projections.selection_header {
            Some(header) => self.apply_selection_header_view_data_for_target(
                header.target,
                header.view_data,
                cx,
            ),
            None => self.apply_clear_selection_header_view_data(cx),
        }

        self.apply_property_value_states(property_states, cx);
    }

    /// Replaces the immutable selection data supplied by the host.
    fn apply_node(&mut self, node: DesignPanelNode, cx: &mut Context<Self>) {
        let node_changed = self.host.inspected_node().id != node.id;
        if self.overlays.active_menu_preview().is_some() && *self.host.inspected_node() != node {
            self.cancel_menu_preview(cx);
        }
        let dimension_preview_invalidated = self
            .features
            .layout
            .dimension_limits_preview
            .as_ref()
            .is_some_and(|preview| {
                preview.node_id != node.id
                    || !Self::dimension_limits_are_applicable_for(
                        &node,
                        &self.host.inspection_context,
                    )
                    || Self::dimension_limits_for_node(&node, preview.axis)
                        != (preview.minimum, preview.maximum)
            });
        if node_changed || dimension_preview_invalidated {
            self.cancel_dimension_limits_preview(cx);
        }
        let parent_layout = self.host.inspection_context.parent_layout();
        let permissions = self.host.inspection_context.permissions();
        let edit_mode = self.host.inspection_context.edit_mode();
        let text_range_revision = self.host.inspection_context.text_range_revision();
        let next_context = match self.host.inspection_context.selection().kind() {
            DesignPanelSelectionKind::None => self.host.inspection_context.clone(),
            DesignPanelSelectionKind::Single => {
                let context =
                    DesignPanelInspectionContext::single(node.clone(), parent_layout, permissions)
                        .with_edit_mode(edit_mode)
                        .expect(
                            "a valid single-selection edit mode remains valid after a host echo",
                        );
                match text_range_revision {
                    Some(revision) => context
                        .with_text_range_revision(revision)
                        .expect("a text-range revision remains valid in Text edit mode"),
                    None => context,
                }
            }
            DesignPanelSelectionKind::Multiple => {
                let items = self.host.inspection_context.selection().items();
                DesignPanelInspectionContext::multiple(
                    DesignPanelMultipleSelection::with_remaining(
                        node.clone(),
                        items[1].clone(),
                        items.iter().skip(2).cloned(),
                    ),
                    parent_layout,
                    permissions,
                )
            }
        };
        let next_selection_header_target = match self.host.inspection_context.selection().kind() {
            DesignPanelSelectionKind::None => None,
            DesignPanelSelectionKind::Single => Some(DesignPanelTarget::Nodes {
                node_ids: vec![node.id.clone()],
            }),
            DesignPanelSelectionKind::Multiple => Some(DesignPanelTarget::Nodes {
                node_ids: std::iter::once(node.id.clone())
                    .chain(
                        self.host
                            .inspection_context
                            .selection()
                            .items()
                            .iter()
                            .skip(1)
                            .map(|item| item.id.clone()),
                    )
                    .collect(),
            }),
        };
        if !node_changed
            && *self.host.inspected_node() != node
            && self
                .edit
                .numeric_scrub
                .as_ref()
                .is_some_and(|scrub| !scrub.active)
        {
            self.edit.numeric_scrub = None;
        }
        if !node_changed {
            self.cancel_interactions_invalidated_by_host_echo(&node, &next_context, cx);
        }
        if !node_changed && *self.host.inspected_node() != node {
            self.clear_option_interactions();
        }
        if node_changed {
            self.cancel_host_interactions_for_context_change(true, cx);
            self.cancel_component_authoring_for_workspace_change(cx);
            if self.overlays.component_authoring_dialog_open() {
                self.overlays
                    .discard(DesignOpenOverlay::ComponentPropertyEdit);
                self.component_authoring.dialog_close_pending = true;
            }
            self.overlays
                .discard(DesignOpenOverlay::AuxiliaryColorPicker);
            self.features.layout.padding_editor_mode = PaddingEditorMode::for_node(&node);
            self.overlays.discard(DesignOpenOverlay::PaintPicker);
            self.overlays.discard(DesignOpenOverlay::EffectSettings);
            self.overlays.discard(DesignOpenOverlay::PaintStyle);
            self.overlays.discard(DesignOpenOverlay::EffectStyle);
            self.overlays.discard(DesignOpenOverlay::PropertyVariable);
            self.overlays
                .discard(DesignOpenOverlay::ComponentPropertyVariable);
            self.overlays.discard(DesignOpenOverlay::ComponentSwap);
            self.features.component.swap_hovered = None;
            self.component_authoring.open_slot_limits = None;
            self.overlays
                .discard(DesignOpenOverlay::ComponentPropertyCreateMenu);
            self.component_authoring.create_draft = None;
            self.component_authoring.edit_draft = None;
            self.component_authoring.selected_property = None;
            self.overlays
                .discard(DesignOpenOverlay::ComponentPropertyContextMenu);
            self.overlays.discard(DesignOpenOverlay::LayoutGridStyle);
            self.overlays
                .discard(DesignOpenOverlay::LayoutGridCountVariable);
            self.overlays.discard(DesignOpenOverlay::FramePreset);
            self.overlays.discard(DesignOpenOverlay::PageBackground);
            self.overlays.discard(DesignOpenOverlay::VariableMode);
            self.overlays.discard(DesignOpenOverlay::TypographyStyle);
            self.overlays.discard(DesignOpenOverlay::FontBrowser);
            self.overlays.discard(DesignOpenOverlay::SelectionHeader);
            if self.host.projections.selection_header_target.as_ref()
                != next_selection_header_target.as_ref()
            {
                self.host.projections.selection_header = None;
                self.host.projections.selection_header_target = None;
            }
            self.overlays.discard(DesignOpenOverlay::TypeSettings);
            self.features.typography.settings_tab = TypographySettingsTab::Basics;
            self.overlays.discard(DesignOpenOverlay::GridDimensions);
            self.features
                .layout
                .dimension_limit_fields_disclosed
                .clear();
            self.overlays.discard(DesignOpenOverlay::DimensionMenu);
            self.sections.reset_contextual_disclosures();
            self.overlays
                .discard(DesignOpenOverlay::AppearanceBlendMode);
            self.overlays.discard(DesignOpenOverlay::PreviewOptionMenu);
            self.overlays.discard(DesignOpenOverlay::MenuPreview);
            self.edit.clear_after_cancellation();
            self.shell
                .scroll_handle
                .set_offset(gpui::point(px(0.), px(0.)));
            self.shell.reset_after_render = true;
            self.host.property_states.clear();
            self.resources.media_paints = DesignMediaPaintViewData::default();
            self.paint_picker.update(cx, |picker, cx| {
                picker.set_media_view_data(DesignMediaPaintViewData::default(), cx);
            });
            self.host.projections.export = None;
            self.features.export.expanded_settings.clear();
            self.features.export.preview_expanded = false;
            self.overlays.clear_for_context_change();
        }
        self.host.page_node_fallback = node;
        self.host.inspection_context = next_context;
        self.reconcile_slot_limits_state();
        self.reconcile_component_authoring_state(cx);
        if !self.dimension_limits_are_applicable() {
            self.features
                .layout
                .dimension_limit_fields_disclosed
                .clear();
            self.overlays.discard(DesignOpenOverlay::DimensionMenu);
        }
        if self
            .overlays
            .active_picker()
            .as_ref()
            .is_some_and(|target| self.picker_paint(target).is_none())
        {
            self.overlays.discard(DesignOpenOverlay::PaintPicker);
        }
        if self
            .overlays
            .paint_style_browser_open()
            .is_some_and(|collection| !self.collection_is_supported(collection))
        {
            self.overlays.discard(DesignOpenOverlay::PaintStyle);
        }
        if !self.collection_is_supported(DesignPanelCollection::Effect) {
            self.overlays.discard(DesignOpenOverlay::EffectSettings);
            self.overlays.discard(DesignOpenOverlay::EffectStyle);
        }
        if !self.collection_is_supported(DesignPanelCollection::LayoutGrid) {
            self.overlays.discard(DesignOpenOverlay::LayoutGridStyle);
            self.overlays
                .discard(DesignOpenOverlay::LayoutGridCountVariable);
        }
        if let Some(mut target) = self.overlays.active_effect_settings().clone() {
            let next_index = if target.effect_id.is_empty() {
                self.host
                    .inspected_node()
                    .effects
                    .get(target.index)
                    .map(|_| target.index)
            } else {
                self.host
                    .inspected_node()
                    .effect_index_by_id(target.effect_id.as_ref())
            };
            if let Some(index) = next_index {
                target.index = index;
                self.overlays
                    .replace(DesignOverlayState::EffectSettings(target));
            } else {
                self.overlays.discard(DesignOpenOverlay::EffectSettings);
            }
        }
        self.reconcile_layout_grid_targets();
        if self.host.inspected_node().typography.is_none() {
            self.overlays.discard(DesignOpenOverlay::TypographyStyle);
            self.overlays.discard(DesignOpenOverlay::FontBrowser);
        }
        if self.frame_preset_view_data_for_context().is_none() {
            self.overlays.discard(DesignOpenOverlay::FramePreset);
        }
        cx.notify();
    }

    /// Enables Figma UI3's cross-file “Additional labels” preference.
    ///
    /// This is deliberately presentation-only state. It survives inspection
    /// context changes and does not emit a [`DesignPanelAction`].
    fn apply_additional_labels(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.preferences.additional_labels == enabled {
            return;
        }
        self.preferences.additional_labels = enabled;
        cx.notify();
    }

    /// Echoes the host's cross-file small/big keyboard nudge preference.
    ///
    /// This preference survives selection changes and emits no document
    /// action. [`DesignNudgeSettings`] guarantees finite positive amounts.
    fn apply_nudge_settings(&mut self, settings: DesignNudgeSettings, cx: &mut Context<Self>) {
        if self.preferences.nudge_settings == settings {
            return;
        }
        self.preferences.nudge_settings = settings;
        cx.notify();
    }

    /// Echoes one host-accepted Design/Draw workspace presentation.
    ///
    /// Switching workspace emits no document action and deliberately
    /// preserves the accepted Design/Prototype surface. Any phased or
    /// transient interaction whose controls are about to be replaced is
    /// balanced or dismissed before the projection changes.
    fn apply_workspace_mode(
        &mut self,
        workspace_mode: DesignPanelWorkspaceMode,
        cx: &mut Context<Self>,
    ) {
        if self.host.navigation.workspace_mode == workspace_mode {
            return;
        }
        self.cancel_dimension_limits_preview(cx);
        self.cancel_host_interactions_for_context_change(true, cx);
        self.cancel_component_authoring_for_workspace_change(cx);
        self.close_editor_only_overlays();
        self.overlays.discard(DesignOpenOverlay::SelectionHeader);
        self.host.navigation.workspace_mode = workspace_mode;
        self.shell
            .scroll_handle
            .set_offset(gpui::point(px(0.), px(0.)));
        self.shell.reset_after_render = true;
        cx.notify();
    }

    /// Echoes one host-accepted right-sidebar surface.
    ///
    /// Returns `false` without changing presentation when `surface` belongs
    /// to the other permission context. Tab activation never calls this
    /// setter directly; it emits [`DesignPanelAction::SurfaceChangeRequested`]
    /// and waits for its host.
    fn apply_active_surface(
        &mut self,
        surface: DesignPanelSurface,
        cx: &mut Context<Self>,
    ) -> bool {
        let can_edit = self.host.inspection_context.permissions().can_edit();
        if !surface.is_available(can_edit) {
            return false;
        }
        if self.active_surface() == surface {
            return true;
        }
        self.cancel_dimension_limits_preview(cx);
        self.cancel_host_interactions_for_context_change(true, cx);
        self.cancel_component_authoring_for_workspace_change(cx);
        self.close_editor_only_overlays();
        self.overlays.discard(DesignOpenOverlay::SelectionHeader);
        if can_edit {
            self.host.navigation.editor_surface = surface;
        } else {
            self.host.navigation.viewer_surface = surface;
        }
        self.shell
            .scroll_handle
            .set_offset(gpui::point(px(0.), px(0.)));
        self.shell.reset_after_render = true;
        cx.notify();
        true
    }

    /// Replaces the complete host-controlled inspection context.
    ///
    /// A multiple selection uses the first supplied node as its aggregate
    /// visual model. It should contain only capabilities and collection/type
    /// leaves with a valid aggregate identity, rather than a wholesale clone
    /// of one real target. Hosts supply uniform/mixed/unset/bound leaf states
    /// with [`Self::set_property_value_states`]. A page/no-selection context
    /// renders page-level controls and never emits document-property edits.
    fn apply_inspection_context(
        &mut self,
        context: DesignPanelInspectionContext,
        cx: &mut Context<Self>,
    ) {
        let previous_selection_header_target =
            Self::selection_header_target_for_context(&self.host.inspection_context);
        let next_selection_header_target = Self::selection_header_target_for_context(&context);
        let selection_header_target_changed =
            previous_selection_header_target != next_selection_header_target;
        let previous_kind = self.host.inspection_context.selection().kind();
        let next_kind = context.selection().kind();
        let edit_mode_changed = self.host.inspection_context.edit_mode() != context.edit_mode();
        let text_range_changed =
            self.host.inspection_context.text_range_revision() != context.text_range_revision();
        let permissions_changed =
            self.host.inspection_context.permissions() != context.permissions();
        let entering_viewer = self.host.inspection_context.permissions().can_edit()
            && !context.permissions().can_edit();
        let next_node = context.selection().items().first().cloned();
        let node_changed = next_node
            .as_ref()
            .is_some_and(|node| node.id != self.host.inspected_node().id);
        self.overlays.discard(DesignOpenOverlay::SelectionHeader);
        if selection_header_target_changed
            && self.host.projections.selection_header_target.as_ref()
                != next_selection_header_target.as_ref()
        {
            self.host.projections.selection_header = None;
            self.host.projections.selection_header_target = None;
        }
        let interaction_target_changed = previous_kind != next_kind
            || node_changed
            || edit_mode_changed
            || text_range_changed
            || selection_header_target_changed;
        if self.overlays.active_menu_preview().is_some()
            && (interaction_target_changed
                || permissions_changed
                || next_node
                    .as_ref()
                    .is_none_or(|node| *node != *self.host.inspected_node()))
        {
            self.cancel_menu_preview(cx);
        }
        let dimension_preview_invalidated = self
            .features
            .layout
            .dimension_limits_preview
            .as_ref()
            .is_some_and(|preview| {
                next_node.as_ref().is_none_or(|node| {
                    preview.node_id != node.id
                        || !Self::dimension_limits_are_applicable_for(node, &context)
                        || Self::dimension_limits_for_node(node, preview.axis)
                            != (preview.minimum, preview.maximum)
                })
            });
        if interaction_target_changed || permissions_changed || dimension_preview_invalidated {
            self.cancel_dimension_limits_preview(cx);
        }
        if !interaction_target_changed
            && next_node
                .as_ref()
                .is_some_and(|node| *node != *self.host.inspected_node())
            && self
                .edit
                .numeric_scrub
                .as_ref()
                .is_some_and(|scrub| !scrub.active)
        {
            self.edit.numeric_scrub = None;
        }
        if interaction_target_changed || permissions_changed {
            self.cancel_host_interactions_for_context_change(interaction_target_changed, cx);
        }
        if permissions_changed && !interaction_target_changed {
            self.cancel_component_authoring_edit_transactions(cx);
        }
        if permissions_changed {
            self.features
                .layout
                .dimension_limit_fields_disclosed
                .clear();
            self.overlays.discard(DesignOpenOverlay::DimensionMenu);
        }
        if entering_viewer {
            self.close_editor_only_overlays();
        }
        if !interaction_target_changed
            && !permissions_changed
            && let Some(node) = next_node.as_ref()
        {
            self.cancel_interactions_invalidated_by_host_echo(node, &context, cx);
        }
        if !interaction_target_changed
            && next_node
                .as_ref()
                .is_some_and(|node| *node != *self.host.inspected_node())
        {
            self.clear_option_interactions();
        }
        if interaction_target_changed {
            self.cancel_component_authoring_for_workspace_change(cx);
            self.overlays
                .discard(DesignOpenOverlay::AuxiliaryColorPicker);
            if let Some(node) = &next_node {
                self.features.layout.padding_editor_mode = PaddingEditorMode::for_node(node);
            }
            self.overlays.discard(DesignOpenOverlay::PaintPicker);
            self.overlays.discard(DesignOpenOverlay::EffectSettings);
            self.overlays.discard(DesignOpenOverlay::PaintStyle);
            self.overlays.discard(DesignOpenOverlay::EffectStyle);
            self.overlays.discard(DesignOpenOverlay::PropertyVariable);
            self.overlays
                .discard(DesignOpenOverlay::ComponentPropertyVariable);
            self.overlays.discard(DesignOpenOverlay::ComponentSwap);
            self.features.component.swap_hovered = None;
            self.component_authoring.open_slot_limits = None;
            self.overlays.discard(DesignOpenOverlay::LayoutGridStyle);
            self.overlays
                .discard(DesignOpenOverlay::LayoutGridCountVariable);
            self.overlays.discard(DesignOpenOverlay::FramePreset);
            self.overlays.discard(DesignOpenOverlay::PageBackground);
            self.overlays.discard(DesignOpenOverlay::VariableMode);
            self.overlays.discard(DesignOpenOverlay::TypographyStyle);
            self.overlays.discard(DesignOpenOverlay::FontBrowser);
            if edit_mode_changed {
                self.host.projections.selection_header = None;
                self.host.projections.selection_header_target = None;
            }
            self.overlays.discard(DesignOpenOverlay::TypeSettings);
            self.features.typography.settings_tab = TypographySettingsTab::Basics;
            self.overlays.discard(DesignOpenOverlay::GridDimensions);
            self.features
                .layout
                .dimension_limit_fields_disclosed
                .clear();
            self.overlays.discard(DesignOpenOverlay::DimensionMenu);
            self.sections.reset_contextual_disclosures();
            self.overlays
                .discard(DesignOpenOverlay::AppearanceBlendMode);
            self.overlays.discard(DesignOpenOverlay::PreviewOptionMenu);
            self.overlays.discard(DesignOpenOverlay::MenuPreview);
            self.edit.clear_after_cancellation();
            self.retained.options.states.clear();
            self.retained.options.subscriptions.clear();
            self.retained.options.snapshots.clear();
            self.host.property_states.clear();
            self.resources.media_paints = DesignMediaPaintViewData::default();
            self.paint_picker.update(cx, |picker, cx| {
                picker.set_media_view_data(DesignMediaPaintViewData::default(), cx);
            });
            self.host.projections.export = None;
            self.features.export.expanded_settings.clear();
            self.features.export.preview_expanded = false;
            self.shell
                .scroll_handle
                .set_offset(gpui::point(px(0.), px(0.)));
            self.shell.reset_after_render = true;
            self.overlays.clear_for_context_change();
        }
        if let Some(node) = next_node {
            self.host.page_node_fallback = node;
        }
        self.host.inspection_context = context;
        self.reconcile_slot_limits_state();
        self.reconcile_component_authoring_state(cx);
        if !self.dimension_limits_are_applicable() {
            self.features
                .layout
                .dimension_limit_fields_disclosed
                .clear();
            self.overlays.discard(DesignOpenOverlay::DimensionMenu);
        }
        self.reconcile_layout_grid_targets();
        if self.frame_preset_view_data_for_context().is_none() {
            self.overlays.discard(DesignOpenOverlay::FramePreset);
        }
        cx.notify();
    }

    /// Supplies canonical host-owned export rows for the current command
    /// target. Rows are normalized at the presentation boundary so SVG and
    /// PDF cannot display or emit an unsupported custom size.
    fn apply_export_view_data(
        &mut self,
        mut view_data: DesignExportViewData,
        cx: &mut Context<Self>,
    ) {
        for configuration in &mut view_data.configurations {
            *configuration = configuration.clone().normalized();
        }
        let ids = view_data
            .configurations
            .iter()
            .map(|configuration| configuration.id.clone())
            .collect::<HashSet<_>>();
        self.features
            .export
            .expanded_settings
            .retain(|configuration_id| ids.contains(configuration_id));
        if let Some(animated) = &mut view_data.animated {
            animated.settings = animated.settings.clone().normalized();
        } else {
            view_data.mode = DesignExportMode::Static;
        }
        if view_data.preview.is_none() {
            self.features.export.preview_expanded = false;
        }
        self.reconcile_export_property_editor(&view_data, cx);
        self.overlays.discard(DesignOpenOverlay::ExportChoice);
        self.host.projections.export = Some(view_data);
        self.clear_option_interactions();
        cx.notify();
    }

    /// Clears canonical export input and falls back to adapting the legacy
    /// scale-only records on [`DesignPanelNode`].
    fn apply_clear_export_view_data(&mut self, cx: &mut Context<Self>) {
        if self
            .edit
            .property
            .as_ref()
            .is_some_and(|editor| editor.export_configuration_id.is_some())
        {
            self.cancel_property_editor_transaction(cx);
        }
        self.host.projections.export = None;
        self.features.export.expanded_settings.clear();
        self.overlays.discard(DesignOpenOverlay::ExportChoice);
        self.features.export.preview_expanded = false;
        self.clear_option_interactions();
        cx.notify();
    }

    /// Stores the compatibility-only leaf color-preset snapshot.
    ///
    /// The built-in picker no longer renders or emits this conflated surface.
    /// New hosts should use [`Self::set_paint_variable_view_data`] for leaf
    /// bindings and [`Self::set_paint_style_view_data`] for complete styles.
    fn apply_color_style_view_data(
        &mut self,
        view_data: DesignColorStyleViewData,
        cx: &mut Context<Self>,
    ) {
        if self.resources.color_styles == view_data {
            return;
        }
        self.resources.color_styles = view_data;
        cx.notify();
    }

    /// Supplies sample-only Color-style values rendered beside Color
    /// variables in the retained picker.
    ///
    /// Sampling changes one color leaf; complete Fill/Stroke style identity
    /// remains exclusively controlled by [`Self::set_paint_style_view_data`].
    fn apply_color_style_sample_view_data(
        &mut self,
        view_data: DesignColorStyleSampleViewData,
        cx: &mut Context<Self>,
    ) {
        if self.resources.color_style_samples == view_data {
            return;
        }
        self.resources.color_style_samples = view_data.clone();
        self.paint_picker.update(cx, |picker, cx| {
            picker.set_color_style_sample_view_data(view_data, cx);
        });
        cx.notify();
    }

    /// Supplies exact host-computed color-contrast results for paint
    /// occurrences in the current inspection context.
    ///
    /// The host resolves the effective scene background and nearest compliant
    /// colors. The panel only chooses a transient WCAG category/level and
    /// emits an ordinary paint edit when the user accepts a supplied
    /// correction.
    fn apply_color_contrast_view_data(
        &mut self,
        view_data: DesignColorContrastViewData,
        cx: &mut Context<Self>,
    ) {
        let view_data = DesignColorContrastViewData::new(view_data.paints);
        if self.resources.color_contrast == view_data {
            return;
        }
        self.resources.color_contrast = view_data;
        cx.notify();
    }

    /// Supplies host-filtered Color variables for solid-paint and
    /// gradient-stop bindings. Whole Paint styles use
    /// [`Self::set_paint_style_view_data`] instead.
    fn apply_paint_variable_view_data(
        &mut self,
        view_data: DesignPaintVariableViewData,
        cx: &mut Context<Self>,
    ) {
        if self.resources.paint_variables == view_data {
            return;
        }
        self.resources.paint_variables = view_data.clone();
        self.paint_picker.update(cx, |picker, cx| {
            picker.set_paint_variable_view_data(view_data, cx);
        });
        cx.notify();
    }

    /// Supplies whole-collection Fill/Stroke Paint styles.
    fn apply_paint_style_view_data(
        &mut self,
        view_data: DesignPaintStyleViewData,
        cx: &mut Context<Self>,
    ) {
        if self.resources.paint_styles == view_data {
            return;
        }
        if self.overlays.paint_style_browser_open().is_some()
            || self
                .overlays
                .selection_color_resource_browser()
                .as_ref()
                .is_some_and(|target| target.kind == SelectionColorResourceKind::PaintStyle)
        {
            self.features.style_browser.source_filter = self
                .features
                .style_browser
                .source_filter
                .normalized_for_libraries(
                    view_data
                        .libraries
                        .iter()
                        .map(|library| (&library.id, &library.name)),
                );
        }
        self.resources.paint_styles = view_data;
        cx.notify();
    }

    /// Supplies host-controlled capabilities, crop-tool state, and video
    /// preview state for stable media paint occurrences.
    fn apply_media_paint_view_data(
        &mut self,
        view_data: DesignMediaPaintViewData,
        cx: &mut Context<Self>,
    ) {
        if self.resources.media_paints == view_data {
            return;
        }
        let active_target = self.overlays.active_picker().clone();
        let crop_interaction_became_unavailable = active_target.as_ref().is_some_and(|target| {
            let Some(index) = self.paint_target_index(target) else {
                return false;
            };
            let previous =
                self.resources
                    .media_paints
                    .paint(target.collection, &target.paint_id, index);
            let next = view_data.paint(target.collection, &target.paint_id, index);
            previous.is_some_and(|view| {
                view.crop_tool.active
                    && next.is_none_or(|next| {
                        !next.crop_tool.active || !next.capabilities.can_edit_properties
                    })
            })
        });
        let active_edit_became_unavailable = self.edit.active_paint_edit().is_some_and(|active| {
            if active.target.node_id != self.host.inspected_node().id {
                return false;
            }
            let target = PaintPickerTarget {
                collection: active.target.collection,
                index: active.target.index,
                paint_id: active.target.paint_id.clone(),
            };
            let Some(index) = self.paint_target_index(&target) else {
                return false;
            };
            self.resources
                .media_paints
                .paint(target.collection, &target.paint_id, index)
                .is_some_and(|previous| {
                    previous.capabilities.can_edit_properties
                        && view_data
                            .paint(target.collection, &target.paint_id, index)
                            .is_none_or(|next| !next.capabilities.can_edit_properties)
                })
        });
        if crop_interaction_became_unavailable && let Some(target) = active_target.as_ref() {
            self.emit_crop_cancel_if_active(target, cx);
        }
        if active_edit_became_unavailable {
            self.prepare_paint_picker_for_dismissal(cx);
            self.cancel_active_paint_edit(cx);
        }
        self.resources.media_paints = view_data.clone();
        let inspected_node_id = self.host.inspected_node().id.clone();
        self.edit
            .reconcile_media_crop_host_snapshot(&inspected_node_id, &view_data);
        self.paint_picker.update(cx, |picker, cx| {
            picker.set_media_view_data(view_data, cx);
        });
        cx.notify();
    }

    /// Supplies host-owned imported/page shaders and discoverable library
    /// shaders. Import and apply remain explicit typed host intents.
    fn apply_shader_view_data(&mut self, view_data: DesignShaderViewData, cx: &mut Context<Self>) {
        if self.resources.shaders == view_data {
            return;
        }
        self.resources.shaders = view_data.clone();
        self.paint_picker.update(cx, |picker, cx| {
            picker.set_shader_view_data(view_data, cx);
        });
        cx.notify();
    }

    /// Supplies the host-owned page text styles and grouped libraries shown
    /// by the retained Typography style picker.
    ///
    /// Applying or detaching a style emits a typed intent. Neither the panel
    /// nor the picker mutates the controlled typography snapshot.
    fn apply_typography_style_view_data(
        &mut self,
        view_data: DesignTypographyStyleViewData,
        cx: &mut Context<Self>,
    ) {
        if self.resources.typography_styles == view_data {
            return;
        }
        self.resources.typography_styles = view_data.clone();
        self.typography_style_picker.update(cx, |picker, cx| {
            picker.set_view_data(view_data, cx);
        });
        cx.notify();
    }

    /// Supplies exact host-controlled font enumeration and loading state.
    fn apply_font_view_data(&mut self, view_data: DesignFontViewData, cx: &mut Context<Self>) {
        if self.resources.fonts == view_data {
            return;
        }
        self.resources.fonts = view_data;
        cx.notify();
    }

    /// Supplies controlled page/library Effect styles. Applying, creating, or
    /// detaching a style only emits a typed host intent.
    fn apply_effect_style_view_data(
        &mut self,
        view_data: DesignEffectStyleViewData,
        cx: &mut Context<Self>,
    ) {
        if self.resources.effect_styles == view_data {
            return;
        }
        if self.overlays.effect_style_browser_open() {
            self.features.style_browser.source_filter = self
                .features
                .style_browser
                .source_filter
                .normalized_for_libraries(
                    view_data
                        .libraries
                        .iter()
                        .map(|library| (&library.id, &library.name)),
                );
        }
        self.resources.effect_styles = view_data;
        cx.notify();
    }

    /// Supplies exact variable candidates for the effect-variable affordances.
    fn apply_effect_variable_view_data(
        &mut self,
        view_data: DesignEffectVariableViewData,
        cx: &mut Context<Self>,
    ) {
        if self.resources.effect_variables == view_data {
            return;
        }
        self.resources.effect_variables = view_data;
        cx.notify();
    }

    /// Supplies the complete page/library variable catalog used by generic
    /// node and text property pickers. Search and open-popover state remain
    /// transient; apply/import/detach only emit host intents.
    fn apply_property_variable_view_data(
        &mut self,
        view_data: DesignVariableViewData,
        cx: &mut Context<Self>,
    ) {
        if self.resources.property_variables == view_data {
            return;
        }
        self.resources.property_variables = view_data;
        cx.notify();
    }

    /// Supplies the complete local/library component catalog used by
    /// instance-swap properties. Open/search/hover state stays transient.
    fn apply_component_swap_view_data(
        &mut self,
        view_data: DesignComponentSwapViewData,
        cx: &mut Context<Self>,
    ) {
        if self.resources.component_swaps == view_data {
            return;
        }
        let hovered_candidate_survives =
            self.features
                .component
                .swap_hovered
                .as_ref()
                .is_none_or(|(_, selection)| {
                    view_data
                        .candidate(selection)
                        .is_some_and(DesignComponentSwapCandidate::can_apply)
                });
        if !hovered_candidate_survives {
            self.cancel_component_swap_preview(cx);
        }
        self.resources.component_swaps = view_data;
        cx.notify();
    }

    /// Supplies page and library Grid styles for the Layout guides header
    /// browser. The panel never imports or applies a style itself.
    fn apply_layout_grid_style_view_data(
        &mut self,
        view_data: DesignLayoutGridStyleViewData,
        cx: &mut Context<Self>,
    ) {
        if self.resources.layout_grid_styles == view_data {
            return;
        }
        if self.overlays.layout_grid_style_browser_open() {
            self.features.style_browser.source_filter = self
                .features
                .style_browser
                .source_filter
                .normalized_for_libraries(
                    view_data
                        .libraries
                        .iter()
                        .map(|library| (&library.id, &library.name)),
                );
        }
        self.resources.layout_grid_styles = view_data;
        cx.notify();
    }

    /// Supplies the host-controlled Number-variable catalog shared by every
    /// supported layout-guide numeric leaf.
    fn apply_layout_grid_variable_view_data(
        &mut self,
        view_data: DesignLayoutGridVariableViewData,
        cx: &mut Context<Self>,
    ) {
        if self.resources.layout_grid_variables == view_data {
            return;
        }
        self.resources.layout_grid_variables = view_data;
        cx.notify();
    }

    /// Compatibility adapter for the former count-only variable catalog.
    fn apply_layout_grid_count_variable_view_data(
        &mut self,
        view_data: DesignLayoutGridCountVariableViewData,
        cx: &mut Context<Self>,
    ) {
        let generalized = view_data.generalized();
        if self.resources.layout_grid_count_variables == view_data
            && self.resources.layout_grid_variables == generalized
        {
            return;
        }
        self.resources.layout_grid_count_variables = view_data;
        self.resources.layout_grid_variables = generalized;
        cx.notify();
    }

    /// Supplies host-resolved structural availability for “Add auto layout”.
    ///
    /// The projection is bound to an exact ordered node target. A late result
    /// for a previous selection may remain stored, but it cannot render or
    /// emit until that same target is current again.
    fn apply_add_auto_layout_view_data(
        &mut self,
        view_data: DesignAddAutoLayoutViewData,
        cx: &mut Context<Self>,
    ) {
        if self.host.projections.add_auto_layout.as_ref() == Some(&view_data) {
            return;
        }
        self.host.projections.add_auto_layout = Some(view_data);
        cx.notify();
    }

    fn apply_clear_add_auto_layout_view_data(&mut self, cx: &mut Context<Self>) {
        if self.host.projections.add_auto_layout.take().is_some() {
            cx.notify();
        }
    }

    /// Supplies the target-bound corner-radius slider policy for Draw.
    ///
    /// Opacity needs no host data because it is always a percentage. Invalid
    /// Page/empty/duplicate targets are rejected without replacing the last
    /// valid projection.
    fn apply_draw_appearance_view_data(
        &mut self,
        view_data: DesignDrawAppearanceViewData,
        cx: &mut Context<Self>,
    ) -> bool {
        if !view_data.is_valid() {
            return false;
        }
        if self.host.projections.draw_appearance.as_ref() == Some(&view_data) {
            return true;
        }
        if self.edit.draw_slider_property == Some(DesignPanelProperty::CornerRadius) {
            self.cancel_property_editor_transaction(cx);
        }
        self.host.projections.draw_appearance = Some(view_data);
        self.rebuild_draw_corner_radius_slider(cx);
        cx.notify();
        true
    }

    fn apply_clear_draw_appearance_view_data(&mut self, cx: &mut Context<Self>) {
        if self.host.projections.draw_appearance.is_none() {
            return;
        }
        if self.edit.draw_slider_property == Some(DesignPanelProperty::CornerRadius) {
            self.cancel_property_editor_transaction(cx);
        }
        self.host.projections.draw_appearance = None;
        self.rebuild_draw_corner_radius_slider(cx);
        cx.notify();
    }

    /// Supplies the exact ordered Frame-preset catalog for one selected Frame.
    ///
    /// The catalog is target-bound, so a late result for a previous selection
    /// stays inert. Opening and closing the grouped chooser is transient; an
    /// enabled leaf emits [`DesignPanelAction::FramePresetApplyRequested`].
    fn apply_frame_preset_view_data(
        &mut self,
        view_data: DesignFramePresetViewData,
        cx: &mut Context<Self>,
    ) {
        if self.host.projections.frame_presets.as_ref() == Some(&view_data) {
            return;
        }
        let target_changed = self
            .host
            .projections
            .frame_presets
            .as_ref()
            .is_some_and(|current| current.target_node_id != view_data.target_node_id);
        let group_ids = view_data
            .groups
            .iter()
            .map(|group| group.id.clone())
            .collect::<HashSet<_>>();
        self.host.projections.frame_presets = Some(view_data);
        if target_changed {
            self.features.layout.collapsed_frame_preset_groups.clear();
        } else {
            self.features
                .layout
                .collapsed_frame_preset_groups
                .retain(|group_id| group_ids.contains(group_id));
        }
        if target_changed || self.frame_preset_view_data_for_context().is_none() {
            self.overlays.discard(DesignOpenOverlay::FramePreset);
        }
        cx.notify();
    }

    fn apply_clear_frame_preset_view_data(&mut self, cx: &mut Context<Self>) {
        if self.host.projections.frame_presets.take().is_some() {
            self.overlays.discard(DesignOpenOverlay::FramePreset);
            self.features.layout.collapsed_frame_preset_groups.clear();
            cx.notify();
        }
    }

    /// Supplies Smart Selection geometry and command availability bound to an
    /// exact ordered multiple-selection target.
    ///
    /// A changed host echo terminates an in-progress spacing draft before the
    /// new snapshot is installed. A stale target may remain stored, but it
    /// cannot render or emit against another selection.
    fn apply_smart_selection_view_data(
        &mut self,
        view_data: DesignSmartSelectionViewData,
        cx: &mut Context<Self>,
    ) {
        if self.host.projections.smart_selection.as_ref() == Some(&view_data) {
            return;
        }
        if self
            .edit
            .property
            .as_ref()
            .is_some_and(|editor| Self::smart_selection_axis(editor.property).is_some())
        {
            self.cancel_property_editor_transaction(cx);
        }
        self.host.projections.smart_selection = Some(view_data);
        cx.notify();
    }

    fn apply_clear_smart_selection_view_data(&mut self, cx: &mut Context<Self>) {
        if self.host.projections.smart_selection.is_none() {
            return;
        }
        if self
            .edit
            .property
            .as_ref()
            .is_some_and(|editor| Self::smart_selection_axis(editor.property).is_some())
        {
            self.cancel_property_editor_transaction(cx);
        }
        self.host.projections.smart_selection = None;
        cx.notify();
    }

    /// Supplies the host-owned Page/no-selection background and resource
    /// catalog. The panel keeps only picker/browser presentation state.
    fn apply_page_view_data(&mut self, view_data: DesignPageViewData, cx: &mut Context<Self>) {
        if self.host.projections.page.as_ref() == Some(&view_data) {
            return;
        }
        let page_changed = self
            .host
            .projections
            .page
            .as_ref()
            .is_some_and(|current| current.page_id != view_data.page_id);
        let active_background_became_read_only = !page_changed
            && view_data.background.read_only
            && self
                .overlays
                .auxiliary_color_picker()
                .as_ref()
                .is_some_and(|target| {
                    matches!(
                        target,
                        AuxiliaryColorPickerTarget::PageBackground { page_id }
                            if *page_id == view_data.page_id
                    )
                });
        if page_changed {
            self.cancel_host_interactions_for_context_change(true, cx);
            self.overlays
                .discard(DesignOpenOverlay::AuxiliaryColorPicker);
        } else if active_background_became_read_only {
            self.prepare_paint_picker_for_dismissal(cx);
            self.cancel_active_paint_edit(cx);
        }
        self.host.projections.page = Some(view_data);
        if page_changed {
            self.overlays.discard(DesignOpenOverlay::PageBackground);
            self.features.page.collapsed_local_style_folders.clear();
        }
        cx.notify();
    }

    fn apply_clear_page_view_data(&mut self, cx: &mut Context<Self>) {
        if self.host.projections.page.is_some() {
            self.cancel_host_interactions_for_context_change(true, cx);
            self.overlays
                .discard(DesignOpenOverlay::AuxiliaryColorPicker);
        }
        if self.host.projections.page.take().is_some() {
            self.overlays.discard(DesignOpenOverlay::PageBackground);
            self.features.page.collapsed_local_style_folders.clear();
            cx.notify();
        }
    }

    /// Supplies the exact current-file local-style tree for one Page.
    ///
    /// Invalid or stale snapshots remain inspectable through the getter but
    /// never render or emit. Hosts can therefore replace Page/context data in
    /// either order without a transient command targeting the wrong file.
    fn apply_page_local_styles_view_data(
        &mut self,
        view_data: DesignPageLocalStylesViewData,
        cx: &mut Context<Self>,
    ) {
        if self.host.projections.page_local_styles.as_ref() == Some(&view_data) {
            return;
        }
        let target_changed = self
            .host
            .projections
            .page_local_styles
            .as_ref()
            .is_some_and(|current| current.target != view_data.target);
        if target_changed {
            self.features.page.collapsed_local_style_folders.clear();
        } else {
            fn collect_folder_ids(
                entries: &[DesignLocalStyleEntry],
                ids: &mut HashSet<SharedString>,
            ) {
                for entry in entries {
                    if let DesignLocalStyleEntry::Folder {
                        id,
                        entries: children,
                        ..
                    } = entry
                    {
                        ids.insert(id.clone());
                        collect_folder_ids(children, ids);
                    }
                }
            }
            let mut current_folder_ids = HashSet::new();
            for section in &view_data.sections {
                collect_folder_ids(&section.entries, &mut current_folder_ids);
            }
            self.features
                .page
                .collapsed_local_style_folders
                .retain(|id| current_folder_ids.contains(id));
        }
        self.host.projections.page_local_styles = Some(view_data);
        cx.notify();
    }

    fn apply_clear_page_local_styles_view_data(&mut self, cx: &mut Context<Self>) {
        if self.host.projections.page_local_styles.take().is_some() {
            self.features.page.collapsed_local_style_folders.clear();
            cx.notify();
        }
    }

    /// Selects whether the compatibility Variables navigation row is visible.
    /// The default is [`DesignVariablesEntryPoint::NavigationBarOnly`].
    fn apply_variables_entry_point(
        &mut self,
        entry_point: DesignVariablesEntryPoint,
        cx: &mut Context<Self>,
    ) {
        if self.preferences.variables_entry_point == entry_point {
            return;
        }
        self.preferences.variables_entry_point = entry_point;
        cx.notify();
    }

    /// Supplies the resolved and explicit mode projection for the current page
    /// or one selected scene node.
    fn apply_variable_mode_view_data(
        &mut self,
        view_data: DesignVariableModeViewData,
        cx: &mut Context<Self>,
    ) {
        if self.host.projections.variable_modes.as_ref() == Some(&view_data) {
            return;
        }
        let target_changed = self
            .host
            .projections
            .variable_modes
            .as_ref()
            .is_some_and(|current| current.target != view_data.target);
        self.host.projections.variable_modes = Some(view_data);
        if target_changed {
            self.overlays.discard(DesignOpenOverlay::VariableMode);
        }
        cx.notify();
    }

    fn apply_clear_variable_mode_view_data(&mut self, cx: &mut Context<Self>) {
        if self.host.projections.variable_modes.take().is_some() {
            self.overlays.discard(DesignOpenOverlay::VariableMode);
            cx.notify();
        }
    }

    /// Supplies the exact view-only Properties projection for one ordered
    /// selection target.
    ///
    /// Content, section summaries, copy payloads, and Borders representation
    /// stay host owned. A snapshot for an older selection may be stored, but
    /// it cannot render or emit while its exact target is not current.
    fn apply_viewer_properties_view_data(
        &mut self,
        view_data: DesignViewerPropertiesViewData,
        cx: &mut Context<Self>,
    ) {
        if self.host.projections.viewer_properties.as_ref() == Some(&view_data) {
            return;
        }
        self.host.projections.viewer_properties = Some(view_data);
        cx.notify();
    }

    fn apply_clear_viewer_properties_view_data(&mut self, cx: &mut Context<Self>) {
        if self.host.projections.viewer_properties.take().is_some() {
            cx.notify();
        }
    }

    /// Supplies the complete ordered selected-node header model.
    ///
    /// This call binds the data to the current selection's exact ordered node
    /// IDs. It remains authoritative only while that target is unchanged,
    /// including its cardinality. The panel keeps only which
    /// title/control/More popover is open; every leaf activation emits
    /// [`DesignPanelAction::SelectionHeaderCommandRequested`].
    fn apply_selection_header_view_data(
        &mut self,
        view_data: DesignSelectionHeaderViewData,
        cx: &mut Context<Self>,
    ) {
        if let Some(target) = self.current_selection_header_target() {
            self.apply_selection_header_view_data_for_target(target, view_data, cx);
        } else {
            self.apply_clear_selection_header_view_data(cx);
        }
    }

    /// Supplies selected-node header data bound to an explicit command target.
    ///
    /// Prefer this setter when header data is resolved asynchronously. A
    /// response for an older target can be stored but cannot render or emit
    /// while another selection is active. `set_inspection_context` also
    /// preserves data pre-bound to the incoming target.
    fn apply_selection_header_view_data_for_target(
        &mut self,
        target: DesignPanelTarget,
        view_data: DesignSelectionHeaderViewData,
        cx: &mut Context<Self>,
    ) {
        if self.host.projections.selection_header.as_ref() == Some(&view_data)
            && self.host.projections.selection_header_target.as_ref() == Some(&target)
        {
            return;
        }
        self.host.projections.selection_header = Some(view_data);
        self.host.projections.selection_header_target = Some(target);
        self.overlays.discard(DesignOpenOverlay::SelectionHeader);
        cx.notify();
    }

    /// Clears host-owned header data and restores the compatibility preset.
    ///
    /// A single selection uses its kind preset. A multiple selection uses the
    /// kind-neutral aggregate preset and never inherits the first member's
    /// title menu or commands. New integrations should normally keep
    /// supplying explicit target-bound data.
    fn apply_clear_selection_header_view_data(&mut self, cx: &mut Context<Self>) {
        let had_view_data = self.host.projections.selection_header.take().is_some();
        let had_target = self
            .host
            .projections
            .selection_header_target
            .take()
            .is_some();
        let had_overlay = self.overlays.discard(DesignOpenOverlay::SelectionHeader);
        if had_view_data || had_target || had_overlay {
            cx.notify();
        }
    }

    /// Supplies exact host-resolved display states for individual properties.
    ///
    /// States not present here fall back to the uniform value in `node`.
    fn apply_property_value_states(
        &mut self,
        states: impl IntoIterator<
            Item = (
                DesignPanelProperty,
                DesignPanelPropertyValueState<DesignPanelValue>,
            ),
        >,
        cx: &mut Context<Self>,
    ) {
        let next_states: HashMap<_, _> = states.into_iter().collect();
        if self.overlays.active_menu_preview().is_some() && self.host.property_states != next_states
        {
            self.cancel_menu_preview(cx);
        }
        if self
            .edit
            .numeric_scrub
            .as_ref()
            .is_some_and(|scrub| !scrub.active)
        {
            self.edit.numeric_scrub = None;
        }
        let active_editor_became_non_editable = self.edit.property.as_ref().is_some_and(|editor| {
            next_states
                .get(&editor.property)
                .is_some_and(|state| state.is_read_only() || state.binding().is_some())
        });
        let active_auxiliary_became_non_editable = self
            .active_auxiliary_state_property()
            .and_then(|property| next_states.get(&property))
            .is_some_and(|state| state.is_read_only() || state.binding().is_some());
        if active_editor_became_non_editable {
            self.cancel_property_editor_transaction(cx);
        }
        if active_auxiliary_became_non_editable {
            self.prepare_paint_picker_for_dismissal(cx);
            self.cancel_active_paint_edit(cx);
        }
        self.host.property_states = next_states;
        self.reconcile_grid_dimensions_for_current_host(cx);
        self.retained.options.snapshots.clear();
        cx.notify();
    }

    fn apply_property_value_state(
        &mut self,
        property: DesignPanelProperty,
        state: DesignPanelPropertyValueState<DesignPanelValue>,
        cx: &mut Context<Self>,
    ) {
        if self
            .overlays
            .active_menu_preview()
            .as_ref()
            .is_some_and(|preview| {
                matches!(
                    preview,
                    DesignMenuPreview::NodeProperty {
                        property: active_property,
                        ..
                    } | DesignMenuPreview::EffectProperty {
                        property: active_property,
                        ..
                    } if *active_property == property
                )
            })
        {
            self.cancel_menu_preview(cx);
        }
        if self
            .edit
            .numeric_scrub
            .as_ref()
            .is_some_and(|scrub| !scrub.active && scrub.property == property)
        {
            self.edit.numeric_scrub = None;
        }
        if self
            .edit
            .property
            .as_ref()
            .is_some_and(|editor| editor.property == property)
            && (state.is_read_only() || state.binding().is_some())
        {
            self.cancel_property_editor_transaction(cx);
        }
        if (state.is_read_only() || state.binding().is_some())
            && self.active_auxiliary_state_property() == Some(property)
        {
            self.prepare_paint_picker_for_dismissal(cx);
            self.cancel_active_paint_edit(cx);
        }
        self.host.property_states.insert(property, state);
        self.reconcile_grid_dimensions_for_current_host(cx);
        self.retained.options.snapshots.remove(&property);
        cx.notify();
    }
}
