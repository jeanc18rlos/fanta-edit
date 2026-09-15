use super::super::*;

/// Immutable read model consumed by the Appearance and Mask surfaces.
///
/// Only Appearance-owned values, capabilities, disclosure state, and retained
/// slider handles are projected. Renderers cannot inspect the panel facade;
/// every interaction returns through [`dispatch`] for validation against the
/// latest host snapshot.
#[derive(Clone)]
pub(in super::super) struct AppearanceProjection {
    panel_id: SharedString,
    target: DesignPanelTarget,
    capabilities: AppearanceCapabilities,
    values: AppearanceValues,
    presentation: AppearancePresentation,
}

/// Host-declared access and topology gates used by Appearance.
#[derive(Clone, Copy)]
pub(in super::super) struct AppearanceCapabilities {
    pub(in super::super) can_edit: bool,
    pub(in super::super) visibility_editable: bool,
    pub(in super::super) blend_mode_editable: bool,
    pub(in super::super) supports_visibility: bool,
    pub(in super::super) supports_layer_appearance: bool,
    pub(in super::super) supports_pass_through_blend: bool,
    pub(in super::super) supports_mask_section: bool,
    pub(in super::super) uniform_radius: bool,
    pub(in super::super) independent_radii: bool,
    pub(in super::super) smoothing: bool,
}

/// Host-controlled Appearance values, kept separate from interaction state.
#[derive(Clone, Copy)]
pub(in super::super) struct AppearanceValues {
    pub(in super::super) visible: bool,
    pub(in super::super) visibility: VisibilityControlState,
    pub(in super::super) opacity: f32,
    pub(in super::super) blend_mode: DesignBlendMode,
    pub(in super::super) corner_radii: [f32; 4],
    pub(in super::super) independent_corners: bool,
    pub(in super::super) corner_smoothing: f32,
    pub(in super::super) shape_geometry: DesignShapeGeometry,
    pub(in super::super) mask_type: Option<DesignMaskType>,
}

/// Cross-render presentation continuity needed by Draw and disclosures.
#[derive(Clone)]
pub(in super::super) struct AppearancePresentation {
    pub(in super::super) renders_draw_workspace: bool,
    pub(in super::super) layer_expanded: bool,
    pub(in super::super) details_expanded: bool,
    pub(in super::super) blend_mode_open: bool,
    pub(in super::super) opacity_slider: Entity<SliderState>,
    pub(in super::super) corner_radius_slider: Entity<SliderState>,
    pub(in super::super) draw_corner_radius_range: Option<DesignDrawSliderRange>,
    pub(in super::super) draw_slider_property: Option<DesignPanelProperty>,
    pub(in super::super) property_editor_active: bool,
    pub(in super::super) opacity_scrub_available: bool,
    pub(in super::super) corner_radius_scrub_available: bool,
}

impl AppearanceProjection {
    pub(in super::super) fn new(
        panel_id: SharedString,
        target: DesignPanelTarget,
        capabilities: AppearanceCapabilities,
        values: AppearanceValues,
        presentation: AppearancePresentation,
    ) -> Self {
        Self {
            panel_id,
            target,
            capabilities,
            values,
            presentation,
        }
    }

    fn corner_details_are_visible(&self) -> bool {
        let first = self.values.corner_radii[0];
        self.presentation.details_expanded
            || self.values.independent_corners
            || self.values.corner_smoothing.abs() > f32::EPSILON
            || self
                .values
                .corner_radii
                .iter()
                .skip(1)
                .any(|radius| (*radius - first).abs() > f32::EPSILON)
    }

    fn slider_range(&self, property: DesignPanelProperty) -> Option<DesignDrawSliderRange> {
        match property {
            DesignPanelProperty::Opacity if self.capabilities.supports_layer_appearance => {
                DesignDrawSliderRange::new(0., 100., 1.)
            }
            DesignPanelProperty::CornerRadius if self.capabilities.uniform_radius => {
                self.presentation.draw_corner_radius_range
            }
            _ => None,
        }
    }

    fn slider_is_enabled(&self, property: DesignPanelProperty) -> bool {
        let scrub_available = match property {
            DesignPanelProperty::Opacity => self.presentation.opacity_scrub_available,
            DesignPanelProperty::CornerRadius => self.presentation.corner_radius_scrub_available,
            _ => false,
        };
        self.presentation.renders_draw_workspace
            && self.slider_range(property).is_some()
            && scrub_available
            && (!self.presentation.property_editor_active
                || self.presentation.draw_slider_property == Some(property))
    }
}

/// Snapshot for visibility controls reused by paint/effect collection rows.
#[derive(Clone)]
pub(in super::super) struct AppearanceVisibilityProjection {
    panel_id: SharedString,
    target: DesignPanelTarget,
    property: DesignPanelProperty,
    visibility: VisibilityControlState,
    editable: bool,
}

impl AppearanceVisibilityProjection {
    pub(in super::super) fn new(
        panel_id: SharedString,
        target: DesignPanelTarget,
        property: DesignPanelProperty,
        visibility: VisibilityControlState,
        editable: bool,
    ) -> Self {
        Self {
            panel_id,
            target,
            property,
            visibility,
            editable,
        }
    }
}

/// Narrow access to the inspector's established, domain-neutral chrome.
pub(in super::super) trait AppearanceInspectorChrome {
    fn appearance_value_cell(
        &self,
        id_suffix: impl Into<SharedString>,
        prefix: &'static str,
        value: impl Into<SharedString>,
        property: DesignPanelProperty,
        next: DesignPanelValue,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn appearance_icon_value_cell(
        &self,
        id_suffix: &'static str,
        icon: ValueFieldIcon,
        value: impl Into<SharedString>,
        property: DesignPanelProperty,
        next: DesignPanelValue,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn appearance_toggle_row(
        &self,
        id_suffix: impl Into<SharedString>,
        label: &'static str,
        value: bool,
        property: DesignPanelProperty,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn appearance_group_label(
        &self,
        label: &'static str,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn appearance_section_header(
        &self,
        section: DesignPanelSection,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn appearance_section(
        &self,
        section: DesignPanelSection,
        content: AnyElement,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn appearance_value_field_icon(
        &self,
        icon: ValueFieldIcon,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;
}

impl AppearanceInspectorChrome for DesignPanel {
    fn appearance_value_cell(
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

    fn appearance_icon_value_cell(
        &self,
        id_suffix: &'static str,
        icon: ValueFieldIcon,
        value: impl Into<SharedString>,
        property: DesignPanelProperty,
        next: DesignPanelValue,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_icon_value_cell(id_suffix, icon, value, property, next, cx)
    }

    fn appearance_toggle_row(
        &self,
        id_suffix: impl Into<SharedString>,
        label: &'static str,
        value: bool,
        property: DesignPanelProperty,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_toggle_row(id_suffix, label, value, property, cx)
    }

    fn appearance_group_label(
        &self,
        label: &'static str,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_group_label(label, cx)
    }

    fn appearance_section_header(
        &self,
        section: DesignPanelSection,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_section_header(section, None, cx)
    }

    fn appearance_section(
        &self,
        section: DesignPanelSection,
        content: AnyElement,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_section(section, None, content, cx)
    }

    fn appearance_value_field_icon(
        &self,
        icon: ValueFieldIcon,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_value_field_icon(icon, cx)
    }
}

#[derive(Clone)]
enum AppearanceEvent {
    ToggleLayer,
    ToggleCornerDetails,
    SetBlendModeOpen(bool),
    PreviewBlendMode {
        mode: DesignBlendMode,
        hovered: bool,
    },
    CommitProperty {
        property: DesignPanelProperty,
        value: DesignPanelValue,
    },
    ClearBlendPreview,
    FinishDrawSlider {
        property: DesignPanelProperty,
        commit: bool,
    },
    StepDrawSlider {
        property: DesignPanelProperty,
        direction: f32,
    },
}

/// Typed event channel retained by Appearance renderers.
#[derive(Clone)]
pub(in super::super) struct AppearanceEventSink {
    panel: Entity<DesignPanel>,
}

impl AppearanceEventSink {
    pub(in super::super) fn new(panel: Entity<DesignPanel>) -> Self {
        Self { panel }
    }

    fn send(
        &self,
        expected_target: &DesignPanelTarget,
        event: AppearanceEvent,
        cx: &mut App,
    ) -> bool {
        self.panel
            .update(cx, |panel, cx| dispatch(panel, expected_target, event, cx))
    }

    fn set_blend_popover_open(
        &self,
        expected_target: &DesignPanelTarget,
        open: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.panel.update(cx, |panel, cx| {
            if open {
                panel.remember_overlay_focus_return(
                    DesignOpenOverlay::AppearanceBlendMode,
                    window,
                    cx,
                );
                dispatch(
                    panel,
                    expected_target,
                    AppearanceEvent::SetBlendModeOpen(true),
                    cx,
                );
            } else if panel.overlays.appearance_blend_mode_open() {
                let _ = panel.dismiss_overlay_from_outside_click(
                    DesignOpenOverlay::AppearanceBlendMode,
                    window,
                    cx,
                );
            }
        });
    }

    fn handle_blend_key(
        &self,
        expected_target: &DesignPanelTarget,
        mode: DesignBlendMode,
        key: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        self.panel.update(cx, |panel, cx| {
            dispatch(
                panel,
                expected_target,
                AppearanceEvent::PreviewBlendMode {
                    mode,
                    hovered: true,
                },
                cx,
            );
            match key {
                "enter" | "space" => {
                    dispatch(
                        panel,
                        expected_target,
                        AppearanceEvent::CommitProperty {
                            property: DesignPanelProperty::BlendMode,
                            value: DesignPanelValue::BlendMode(mode),
                        },
                        cx,
                    );
                    window.prevent_default();
                    true
                }
                "escape" => {
                    let handled = panel.dismiss_overlay_from_escape(
                        DesignOpenOverlay::AppearanceBlendMode,
                        window,
                        cx,
                    );
                    if handled {
                        window.prevent_default();
                    }
                    handled
                }
                "tab" => {
                    dispatch(
                        panel,
                        expected_target,
                        AppearanceEvent::ClearBlendPreview,
                        cx,
                    );
                    false
                }
                _ => false,
            }
        })
    }
}

fn live_target_matches(panel: &DesignPanel, expected_target: &DesignPanelTarget) -> bool {
    panel.command_target() == *expected_target
}

fn live_property_is_editable(
    panel: &DesignPanel,
    expected_target: &DesignPanelTarget,
    property: DesignPanelProperty,
) -> bool {
    live_target_matches(panel, expected_target)
        && panel.can_edit()
        && panel.property_is_editable(property)
}

/// Translate a projection event against the latest host snapshot.
///
/// The return value reports whether a keyboard event was handled. A slider
/// transaction whose target or access changed is cancelled rather than being
/// allowed to commit against a stale selection.
fn dispatch(
    panel: &mut DesignPanel,
    expected_target: &DesignPanelTarget,
    event: AppearanceEvent,
    cx: &mut Context<DesignPanel>,
) -> bool {
    match event {
        AppearanceEvent::ToggleLayer => {
            if !live_target_matches(panel, expected_target)
                || !panel
                    .host
                    .inspected_node()
                    .supports_section(DesignPanelSection::Layer)
            {
                return false;
            }
            panel.toggle_section(DesignPanelSection::Layer, cx);
            true
        }
        AppearanceEvent::ToggleCornerDetails => {
            if !live_target_matches(panel, expected_target)
                || !panel.can_edit()
                || !panel
                    .host
                    .inspected_node()
                    .corner_capabilities
                    .independent_radii
                || !panel
                    .host
                    .inspected_node()
                    .supports_section(DesignPanelSection::Layer)
            {
                return false;
            }
            panel.sections.toggle_appearance_details();
            cx.notify();
            true
        }
        AppearanceEvent::SetBlendModeOpen(open) => {
            if !live_target_matches(panel, expected_target)
                || (open && !panel.property_is_editable(DesignPanelProperty::BlendMode))
            {
                return false;
            }
            panel
                .overlays
                .set_open(DesignOverlayState::AppearanceBlendMode, open);
            if open {
                panel.prepare_paint_picker_for_dismissal(cx);
                panel.cancel_menu_preview(cx);
            } else {
                panel.cancel_menu_preview_for_property(DesignPanelProperty::BlendMode, cx);
            }
            cx.notify();
            true
        }
        AppearanceEvent::PreviewBlendMode { mode, hovered } => {
            if !live_property_is_editable(panel, expected_target, DesignPanelProperty::BlendMode) {
                return false;
            }
            panel.set_property_menu_preview(
                DesignPanelProperty::BlendMode,
                DesignPanelValue::BlendMode(mode),
                hovered,
                cx,
            );
            true
        }
        AppearanceEvent::CommitProperty { property, value } => {
            if !live_property_is_editable(panel, expected_target, property) {
                return false;
            }
            panel.cancel_menu_preview(cx);
            if property == DesignPanelProperty::BlendMode {
                panel
                    .overlays
                    .discard(DesignOpenOverlay::AppearanceBlendMode);
            }
            panel.emit_property(property, value, cx);
            true
        }
        AppearanceEvent::ClearBlendPreview => {
            if !live_target_matches(panel, expected_target) {
                return false;
            }
            panel.cancel_menu_preview(cx);
            true
        }
        AppearanceEvent::FinishDrawSlider { property, commit } => {
            let commit = commit && live_property_is_editable(panel, expected_target, property);
            panel.finish_draw_appearance_slider(property, commit, cx)
        }
        AppearanceEvent::StepDrawSlider {
            property,
            direction,
        } => {
            if !live_property_is_editable(panel, expected_target, property) {
                return false;
            }
            step_draw_slider_edit(panel, property, direction, cx)
        }
    }
}

/// Thin compatibility adapter for the facade and neighboring legacy
/// collection sections. Rendering delegates to [`AppearanceProjection`]
/// functions below; controller methods remain centralized here until the
/// retained slider editor itself becomes a reusable molecule.
pub(in super::super) trait AppearancePanelCompat: Sized {
    fn rebuild_draw_corner_radius_slider(&mut self, cx: &mut Context<Self>);
    #[cfg(test)]
    fn visibility_control_state(
        &self,
        property: DesignPanelProperty,
        fallback_visible: bool,
    ) -> VisibilityControlState;
    fn preview_draw_appearance_slider(
        &mut self,
        property: DesignPanelProperty,
        value: f32,
        cx: &mut Context<Self>,
    );
    fn finish_draw_appearance_slider(
        &mut self,
        property: DesignPanelProperty,
        commit: bool,
        cx: &mut Context<Self>,
    ) -> bool;
    fn sync_draw_appearance_sliders(&mut self, window: &mut Window, cx: &mut Context<Self>);
    #[cfg(test)]
    fn render_appearance_header(&self, cx: &mut Context<Self>) -> AnyElement;
    #[cfg(test)]
    fn appearance_corner_details_are_visible(&self) -> bool;
    fn render_visibility_button(
        &self,
        id_suffix: impl Into<SharedString>,
        fallback_visible: bool,
        property: DesignPanelProperty,
        cx: &mut Context<Self>,
    ) -> AnyElement;
    fn render_shape_appearance_controls(&self, cx: &mut Context<Self>) -> Option<AnyElement>;
    fn render_layer(&self, cx: &mut Context<Self>) -> AnyElement;
    fn render_mask(&self, cx: &mut Context<Self>) -> Option<AnyElement>;
    #[cfg(test)]
    fn draw_appearance_view_data_for_context(&self) -> Option<&DesignDrawAppearanceViewData>;
}

impl AppearancePanelCompat for DesignPanel {
    fn rebuild_draw_corner_radius_slider(&mut self, cx: &mut Context<Self>) {
        let range = self
            .host
            .projections
            .draw_appearance
            .as_ref()
            .map(|view_data| view_data.corner_radius_range)
            .unwrap_or_else(|| {
                DesignDrawSliderRange::new(0., 100., 1.)
                    .expect("the private Draw corner fallback range is valid")
            });
        let state = cx.new(|_| {
            SliderState::new()
                .min(range.min())
                .max(range.max())
                .step(range.step())
                .default_value(range.clamp_and_snap(self.host.inspected_node().corner_radii[0]))
        });
        let subscription = cx.subscribe(&state, |this, _, event: &SliderEvent, cx| {
            let SliderEvent::Change(SliderValue::Single(value)) = event else {
                return;
            };
            this.preview_draw_appearance_slider(DesignPanelProperty::CornerRadius, *value, cx);
        });
        self.retained
            .draw_sliders
            .replace_corner_radius(state, subscription);
    }

    #[cfg(test)]
    fn visibility_control_state(
        &self,
        property: DesignPanelProperty,
        fallback_visible: bool,
    ) -> VisibilityControlState {
        super::super::appearance::visibility_projection(self, property, fallback_visible).visibility
    }

    fn preview_draw_appearance_slider(
        &mut self,
        property: DesignPanelProperty,
        value: f32,
        cx: &mut Context<Self>,
    ) {
        if !self.renders_draw_workspace() || !self.property_is_editable(property) {
            if self.edit.draw_slider_property == Some(property) {
                self.finish_draw_appearance_slider(property, false, cx);
            }
            return;
        }
        let Some(range) = draw_slider_range(self, property) else {
            return;
        };
        let scalar = range.clamp_and_snap(value);
        let value = DesignPanelValue::Number(scalar);
        if self.edit.draw_slider_property.is_none() {
            if self.edit.has_property_edit()
                || self.edit.property.is_some()
                || self.edit.numeric_scrub.is_some()
            {
                return;
            }
            let Some((original, kind, base)) = self.numeric_scrub_seed(property) else {
                return;
            };
            self.cancel_menu_preview(cx);
            self.overlays.discard(DesignOpenOverlay::PaintPicker);
            self.overlays.discard(DesignOpenOverlay::EffectSettings);
            self.overlays.discard(DesignOpenOverlay::GridDimensions);
            self.overlays.discard(DesignOpenOverlay::EffectStyle);
            self.overlays.discard(DesignOpenOverlay::PropertyVariable);
            self.overlays
                .discard(DesignOpenOverlay::ComponentPropertyVariable);
            self.overlays.discard(DesignOpenOverlay::ComponentSwap);
            self.overlays.discard(DesignOpenOverlay::TypeSettings);
            let Some(begin) = self.edit.begin_property_edit(property, original.clone()) else {
                return;
            };
            let mut field = crate::molecules::InspectorSliderField::new(
                crate::molecules::InspectorValue::Uniform(base),
                crate::molecules::InspectorFieldPresentation::new(
                    crate::molecules::InspectorFieldAccess::Editable,
                ),
            );
            let Some(_field_begin) = field.begin(base) else {
                let _ = self.edit.cancel_property_edit();
                return;
            };
            self.edit.focus_return = None;
            self.edit.property = Some(PropertyEditor {
                property,
                layout_grid_target: None,
                export_configuration_id: None,
                original: original.clone(),
                last_preview: None,
                base,
                kind,
                controlled: PropertyEditorControlledField::Compound,
            });
            self.edit.draw_slider_property = Some(property);
            self.edit.draw_slider_field = Some(field);
            self.edit.property_invalid = false;
            self.emit_property_lifecycle_event(begin, cx);
        }
        if self.edit.draw_slider_property != Some(property) {
            return;
        }
        let should_preview = self.edit.property.as_ref().is_some_and(|editor| {
            editor.property == property
                && !(editor.last_preview.is_none() && editor.original == value)
                && editor.last_preview.as_ref() != Some(&value)
        }) && self.layout_property_value_is_applicable(property, &value);
        let field_preview = should_preview
            .then(|| {
                self.edit
                    .draw_slider_field
                    .as_mut()
                    .and_then(|field| field.preview(f64::from(scalar)))
            })
            .flatten();
        let preview = field_preview
            .is_some()
            .then(|| self.edit.preview_property_edit(property, value.clone()))
            .flatten();
        if preview.is_some()
            && let Some(editor) = self.edit.property.as_mut()
        {
            editor.last_preview = Some(value.clone());
        }
        if let Some(event) = preview {
            self.emit_property_lifecycle_event(event, cx);
        }
        cx.notify();
    }

    fn finish_draw_appearance_slider(
        &mut self,
        property: DesignPanelProperty,
        commit: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.edit.draw_slider_property != Some(property) {
            return false;
        }
        self.edit.draw_slider_property = None;
        let field_terminal = if commit {
            self.edit
                .draw_slider_field
                .as_mut()
                .and_then(crate::molecules::InspectorSliderField::commit)
        } else {
            self.edit
                .draw_slider_field
                .as_mut()
                .and_then(crate::molecules::InspectorSliderField::cancel)
        };
        self.edit.draw_slider_field = None;
        let Some(editor) = self
            .edit
            .property
            .take()
            .filter(|editor| editor.property == property)
        else {
            cx.notify();
            return false;
        };
        debug_assert!(
            field_terminal.is_some(),
            "an active Draw slider must terminate its shared inspector session"
        );
        self.edit.vector_target_ids = None;
        self.edit.property_invalid = false;
        self.edit.suppress_property_input_change = false;
        let event = if commit {
            let value = editor
                .last_preview
                .unwrap_or_else(|| editor.original.clone());
            self.edit.commit_property_edit(property, value)
        } else {
            self.edit.cancel_property_edit()
        };
        if let Some(event) = event {
            self.emit_property_lifecycle_event(event, cx);
        }
        cx.notify();
        true
    }

    fn sync_draw_appearance_sliders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (property, state) in [
            (
                DesignPanelProperty::Opacity,
                self.retained.draw_sliders.opacity.clone(),
            ),
            (
                DesignPanelProperty::CornerRadius,
                self.retained.draw_sliders.corner_radius.clone(),
            ),
        ] {
            if self.edit.draw_slider_property == Some(property)
                || draw_slider_range(self, property).is_none()
            {
                continue;
            }
            let Some(DesignPanelValue::Number(value)) = self.resolved_property_value(property)
            else {
                continue;
            };
            let Some(range) = draw_slider_range(self, property) else {
                continue;
            };
            let value = range.clamp_and_snap(value);
            if (state.read(cx).value().end() - value).abs() <= f32::EPSILON {
                continue;
            }
            state.update(cx, |state, cx| {
                state.set_value(value, window, cx);
            });
        }
    }

    #[cfg(test)]
    fn render_appearance_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let projection = super::super::appearance::projection(self);
        let variable_mode_popover = self.render_variable_mode_popover(cx);
        let visibility_variable_button =
            self.render_property_variable_button(DesignPanelProperty::Visible, cx);
        let events = AppearanceEventSink::new(cx.entity());
        render_header(
            &projection,
            variable_mode_popover,
            visibility_variable_button,
            &events,
            cx,
        )
    }

    #[cfg(test)]
    fn appearance_corner_details_are_visible(&self) -> bool {
        super::super::appearance::projection(self).corner_details_are_visible()
    }

    fn render_visibility_button(
        &self,
        id_suffix: impl Into<SharedString>,
        fallback_visible: bool,
        property: DesignPanelProperty,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let projection =
            super::super::appearance::visibility_projection(self, property, fallback_visible);
        let property_variable_button = self.render_property_variable_button(property, cx);
        let events = AppearanceEventSink::new(cx.entity());
        render_visibility_control(
            &projection,
            id_suffix,
            property_variable_button,
            &events,
            cx,
        )
    }

    fn render_shape_appearance_controls(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let projection = super::super::appearance::projection(self);
        render_shape_controls(&projection, self, cx)
    }

    fn render_layer(&self, cx: &mut Context<Self>) -> AnyElement {
        let projection = super::super::appearance::projection(self);
        let applied_component_controls = (!projection.presentation.renders_draw_workspace)
            .then(|| {
                self.render_applied_component_property_controls(
                    DesignComponentPropertyApplicationSurface::Appearance,
                    cx,
                )
            })
            .flatten();
        let variable_mode_popover = (!projection.presentation.renders_draw_workspace)
            .then(|| self.render_variable_mode_popover(cx))
            .flatten();
        let visibility_variable_button =
            self.render_property_variable_button(DesignPanelProperty::Visible, cx);
        let events = AppearanceEventSink::new(cx.entity());
        render(
            &projection,
            self,
            applied_component_controls,
            variable_mode_popover,
            visibility_variable_button,
            &events,
            cx,
        )
    }

    fn render_mask(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let projection = super::super::appearance::projection(self);
        render_mask(&projection, self, cx)
    }

    #[cfg(test)]
    fn draw_appearance_view_data_for_context(&self) -> Option<&DesignDrawAppearanceViewData> {
        let view_data = self.host.projections.draw_appearance.as_ref()?;
        (view_data.is_valid()
            && view_data.target == self.command_target()
            && !self.host.inspection_context.selection().is_empty()
            && multiple_selection_has_uniform_draw_geometry(self)
            && self
                .host
                .inspected_node()
                .supports_section(DesignPanelSection::Layer))
        .then_some(view_data)
    }
}

fn draw_slider_range(
    panel: &DesignPanel,
    property: DesignPanelProperty,
) -> Option<DesignDrawSliderRange> {
    super::super::appearance::projection(panel).slider_range(property)
}

fn step_draw_slider_edit(
    panel: &mut DesignPanel,
    property: DesignPanelProperty,
    direction: f32,
    cx: &mut Context<DesignPanel>,
) -> bool {
    let Some(range) = draw_slider_range(panel, property) else {
        return false;
    };
    let Some((_, _, scalar)) = panel.numeric_scrub_seed(property) else {
        return false;
    };
    let current = scalar as f32;
    let next = range.clamp_and_snap(current + range.step() * direction.signum());
    if (next - current).abs() <= f32::EPSILON {
        return false;
    }
    panel.preview_draw_appearance_slider(property, next, cx);
    panel.finish_draw_appearance_slider(property, true, cx)
}

#[cfg(test)]
fn multiple_selection_has_uniform_draw_geometry(panel: &DesignPanel) -> bool {
    if panel.host.inspection_context.selection().kind() != DesignPanelSelectionKind::Multiple {
        return true;
    }

    [DesignPanelProperty::Width, DesignPanelProperty::Height]
        .into_iter()
        .all(|property| {
            panel
                .host
                .property_states
                .get(&property)
                .and_then(DesignPanelPropertyValueState::resolved)
                .is_some_and(
                    |value| matches!(value, DesignPanelValue::Number(value) if value.is_finite()),
                )
        })
}

fn render_blend_icon(active: bool, cx: &mut Context<DesignPanel>) -> AnyElement {
    let color = if active {
        cx.theme().selection
    } else {
        cx.theme().foreground
    };
    render_lucide_icon(LucideIcon::Blend, color, 16.)
}

fn render_visibility_control(
    projection: &AppearanceVisibilityProjection,
    id_suffix: impl Into<SharedString>,
    property_variable_button: Option<AnyElement>,
    events: &AppearanceEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let visibility = projection.visibility;
    let next_visible = visibility.activation_value();
    let editable = projection.editable;
    let property = projection.property;
    let expected_target = projection.target.clone();
    let icon = match visibility {
        VisibilityControlState::Visible => IconName::Eye,
        VisibilityControlState::Hidden => IconName::EyeOff,
        VisibilityControlState::Mixed => IconName::Minus,
    };
    let tooltip = SharedString::from(visibility.tooltip(editable));
    let selector = format!("{}-{}", projection.panel_id, id_suffix.into());
    let debug_selector = selector.clone();
    let mut button = div()
        .id(SharedString::from(selector))
        .debug_selector(move || debug_selector.clone())
        .size(px(24.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .text_color(match visibility {
            VisibilityControlState::Visible => cx.theme().foreground,
            VisibilityControlState::Hidden => cx.theme().muted_foreground,
            VisibilityControlState::Mixed => cx.theme().selection,
        })
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .when(visibility == VisibilityControlState::Mixed, |button| {
            button.bg(cx.theme().selection.opacity(0.14))
        })
        .when(editable, |button| {
            button
                .key_context(CONTROL_KEY_CONTEXT)
                .tab_index(0)
                .cursor_pointer()
                .hover(|style| style.bg(cx.theme().accent))
                .focus(|style| {
                    style
                        .bg(cx.theme().accent)
                        .border_1()
                        .border_color(cx.theme().selection)
                })
        })
        .when(!editable, |button| button.opacity(0.62));
    if editable {
        let events = events.clone();
        button = button.on_activate(move |_, _, cx| {
            events.send(
                &expected_target,
                AppearanceEvent::CommitProperty {
                    property,
                    value: DesignPanelValue::Bool(next_visible),
                },
                cx,
            );
        });
    }
    h_flex()
        .h(px(24.))
        .gap_0p5()
        .when_some(property_variable_button, |controls, variable| {
            controls.child(variable)
        })
        .child(button.child(Icon::new(icon).xsmall()))
        .into_any_element()
}

fn render_corner_details_control(
    projection: &AppearanceProjection,
    chrome: &impl AppearanceInspectorChrome,
    events: &AppearanceEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let enabled = projection.capabilities.can_edit;
    let selected = projection.corner_details_are_visible();
    let expected_target = projection.target.clone();
    let mut button = div()
        .id(SharedString::from(format!(
            "{}-appearance-corner-details",
            projection.panel_id
        )))
        .size(px(ROW_HEIGHT))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .when(selected, |button| {
            button
                .bg(cx.theme().selection.opacity(0.22))
                .text_color(cx.theme().selection)
        })
        .when(enabled, |button| {
            button
                .key_context(CONTROL_KEY_CONTEXT)
                .tab_index(0)
                .cursor_pointer()
                .hover(|style| style.bg(cx.theme().accent))
                .focus(|style| {
                    style
                        .bg(cx.theme().accent)
                        .border_1()
                        .border_color(cx.theme().selection)
                })
        })
        .when(!enabled, |button| button.opacity(0.38));
    if enabled {
        let events = events.clone();
        button = button.on_activate(move |_, _, cx| {
            events.send(&expected_target, AppearanceEvent::ToggleCornerDetails, cx);
        });
    }
    button
        .child(chrome.appearance_value_field_icon(ValueFieldIcon::Corners, cx))
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn render_draw_slider_row(
    projection: &AppearanceProjection,
    chrome: &impl AppearanceInspectorChrome,
    id_suffix: &'static str,
    label: &'static str,
    icon: ValueFieldIcon,
    property: DesignPanelProperty,
    displayed_value: SharedString,
    next: DesignPanelValue,
    slider: Entity<SliderState>,
    show_slider: bool,
    trailing: Option<AnyElement>,
    events: &AppearanceEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let enabled = projection.slider_is_enabled(property);
    let expected_target_for_key = projection.target.clone();
    let expected_target_for_up = projection.target.clone();
    let expected_target_for_up_out = projection.target.clone();
    let events_for_key = events.clone();
    let events_for_up = events.clone();
    let events_for_up_out = events.clone();
    let selector = SharedString::from(format!("{}-draw-{id_suffix}-slider", projection.panel_id));
    let debug_selector = selector.to_string();
    let slider_element = div()
        .id(selector)
        .debug_selector(move || debug_selector.clone())
        .flex_1()
        .min_w(px(0.))
        .h(px(24.))
        .when(enabled, |track| {
            track
                .key_context(CONTROL_KEY_CONTEXT)
                .tab_index(0)
                .cursor_pointer()
                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                    let direction = match event.keystroke.key.as_str() {
                        "left" | "down" => -1.,
                        "right" | "up" => 1.,
                        _ => return,
                    };
                    let handled = events_for_key.send(
                        &expected_target_for_key,
                        AppearanceEvent::StepDrawSlider {
                            property,
                            direction,
                        },
                        cx,
                    );
                    if handled {
                        window.prevent_default();
                        cx.stop_propagation();
                    }
                })
        })
        .when(!enabled, |track| track.opacity(0.52))
        .on_mouse_up(MouseButton::Left, move |_: &MouseUpEvent, _, cx| {
            events_for_up.send(
                &expected_target_for_up,
                AppearanceEvent::FinishDrawSlider {
                    property,
                    commit: true,
                },
                cx,
            );
        })
        .on_mouse_up_out(MouseButton::Left, move |_: &MouseUpEvent, _, cx| {
            events_for_up_out.send(
                &expected_target_for_up_out,
                AppearanceEvent::FinishDrawSlider {
                    property,
                    commit: true,
                },
                cx,
            );
        })
        .child(Slider::new(&slider).horizontal().disabled(!enabled));

    v_flex()
        .w_full()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .child(
                    div()
                        .w(px(96.))
                        .flex_none()
                        .child(chrome.appearance_icon_value_cell(
                            id_suffix,
                            icon,
                            displayed_value,
                            property,
                            next,
                            cx,
                        )),
                )
                .when(show_slider, |row| row.child(slider_element))
                .when(!show_slider, |row| row.child(div().flex_1().min_w(px(0.))))
                .when_some(trailing, |row, action| row.child(action)),
        )
        .into_any_element()
}

fn render_shape_controls(
    projection: &AppearanceProjection,
    chrome: &impl AppearanceInspectorChrome,
    cx: &mut Context<DesignPanel>,
) -> Option<AnyElement> {
    let controls = match projection.values.shape_geometry {
        DesignShapeGeometry::Polygon(geometry) => v_flex()
            .w_full()
            .gap_2()
            .child(chrome.appearance_value_cell(
                "polygon-count",
                "Sides",
                geometry.point_count.to_string(),
                DesignPanelProperty::PolygonCount,
                DesignPanelValue::Integer(i64::from(
                    geometry.point_count.saturating_add(1).min(60),
                )),
                cx,
            ))
            .into_any_element(),
        DesignShapeGeometry::Star(geometry) => h_flex()
            .w_full()
            .gap_2()
            .child(chrome.appearance_value_cell(
                "star-points",
                "Points",
                geometry.point_count.to_string(),
                DesignPanelProperty::StarPointCount,
                DesignPanelValue::Integer(i64::from(
                    geometry.point_count.saturating_add(1).min(60),
                )),
                cx,
            ))
            .child(chrome.appearance_value_cell(
                "star-inner-radius",
                "%",
                format!("{}%", format_number(geometry.inner_radius * 100.)),
                DesignPanelProperty::StarInnerRadius,
                DesignPanelValue::Ratio((geometry.inner_radius + 0.05).min(1.)),
                cx,
            ))
            .into_any_element(),
        DesignShapeGeometry::Ellipse(arc) if arc.has_inspector_controls() => v_flex()
            .w_full()
            .gap_2()
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(chrome.appearance_value_cell(
                        "arc-start",
                        "↻",
                        format!("{}°", format_number(arc.starting_degrees())),
                        DesignPanelProperty::ArcStartingAngle,
                        DesignPanelValue::AngleRadians(arc.starting_angle + 15_f32.to_radians()),
                        cx,
                    ))
                    .child(chrome.appearance_value_cell(
                        "arc-sweep",
                        "◔",
                        format!("{}°", format_number(arc.sweep_degrees())),
                        DesignPanelProperty::ArcSweep,
                        DesignPanelValue::AngleRadians(arc.sweep_angle() + 15_f32.to_radians()),
                        cx,
                    )),
            )
            .child(chrome.appearance_value_cell(
                "arc-ratio",
                "R",
                format!("{}%", format_number(arc.inner_radius * 100.)),
                DesignPanelProperty::ArcInnerRadius,
                DesignPanelValue::Ratio((arc.inner_radius + 0.1).min(1.)),
                cx,
            ))
            .into_any_element(),
        DesignShapeGeometry::None
        | DesignShapeGeometry::Ellipse(_)
        | DesignShapeGeometry::Boolean(_)
        | DesignShapeGeometry::Table(_) => return None,
    };
    Some(controls)
}

fn render_blend_popover(
    projection: &AppearanceProjection,
    events: &AppearanceEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let current = projection.values.blend_mode;
    let supports_pass_through = projection.capabilities.supports_pass_through_blend;
    let editable = projection.capabilities.blend_mode_editable;
    let blend_open = projection.presentation.blend_mode_open;
    let expected_target_for_keyboard = projection.target.clone();
    let expected_target_for_open = projection.target.clone();
    let events_for_open = events.clone();
    let events_for_keyboard = events.clone();
    let events_for_content = events.clone();
    let target_for_content = projection.target.clone();
    let trigger = Button::new(SharedString::from(format!(
        "{}-appearance-blend-mode",
        projection.panel_id
    )))
    .xsmall()
    .compact()
    .ghost()
    .w(px(24.))
    .h(px(24.))
    .disabled(!editable)
    .on_keyboard_activate(move |_, cx| {
        events_for_keyboard.send(
            &expected_target_for_keyboard,
            AppearanceEvent::SetBlendModeOpen(!blend_open),
            cx,
        );
    })
    .child(render_blend_icon(
        blend_open
            || !matches!(
                current,
                DesignBlendMode::Normal | DesignBlendMode::PassThrough
            ),
        cx,
    ));

    Popover::new(SharedString::from(format!(
        "{}-appearance-blend-mode-popover",
        projection.panel_id
    )))
    .anchor(Anchor::TopRight)
    .open(blend_open)
    .overlay_closable(true)
    .on_open_change(move |open, window, cx| {
        events_for_open.set_blend_popover_open(&expected_target_for_open, *open, window, cx);
    })
    .trigger(trigger)
    .content(move |_, window, cx| {
        let popover = cx.entity();
        v_flex()
            .w(popup_width(window, 176.))
            .max_h(popup_height(window, 336.))
            .overflow_y_scrollbar()
            .gap_1()
            .children(
                DesignBlendMode::ALL
                    .into_iter()
                    .filter(move |mode| {
                        supports_pass_through || *mode != DesignBlendMode::PassThrough
                    })
                    .map(|mode| {
                        let events = events_for_content.clone();
                        let events_for_hover = events.clone();
                        let events_for_key = events.clone();
                        let popover = popover.clone();
                        let hover_target = target_for_content.clone();
                        let key_target = target_for_content.clone();
                        let activate_target = target_for_content.clone();
                        Button::new(SharedString::from(format!(
                            "appearance-blend-mode-{}",
                            mode.label().to_lowercase().replace(' ', "-")
                        )))
                        .label(mode.label())
                        .xsmall()
                        .compact()
                        .ghost()
                        .w_full()
                        .selected(mode == current)
                        .on_hover(move |hovered, _, cx| {
                            events_for_hover.send(
                                &hover_target,
                                AppearanceEvent::PreviewBlendMode {
                                    mode,
                                    hovered: *hovered,
                                },
                                cx,
                            );
                        })
                        .on_key_down(move |event: &KeyDownEvent, window, cx| {
                            if events_for_key.handle_blend_key(
                                &key_target,
                                mode,
                                event.keystroke.key.as_str(),
                                window,
                                cx,
                            ) {
                                cx.stop_propagation();
                            }
                        })
                        .on_activate(move |_, window, cx| {
                            events.send(
                                &activate_target,
                                AppearanceEvent::CommitProperty {
                                    property: DesignPanelProperty::BlendMode,
                                    value: DesignPanelValue::BlendMode(mode),
                                },
                                cx,
                            );
                            popover.update(cx, |popover, cx| {
                                popover.dismiss(window, cx);
                            });
                        })
                    }),
            )
    })
    .into_any_element()
}

fn render_header(
    projection: &AppearanceProjection,
    variable_mode_popover: Option<AnyElement>,
    visibility_variable_button: Option<AnyElement>,
    events: &AppearanceEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let section = DesignPanelSection::Layer;
    let id = SharedString::from(format!("{}-section-appearance", projection.panel_id));
    let expected_target = projection.target.clone();
    let visibility = AppearanceVisibilityProjection {
        panel_id: projection.panel_id.clone(),
        target: projection.target.clone(),
        property: DesignPanelProperty::Visible,
        visibility: projection.values.visibility,
        editable: projection.capabilities.visibility_editable,
    };
    let header_events = events.clone();
    h_flex()
        .id(id)
        .key_context(CONTROL_KEY_CONTEXT)
        .tab_index(0)
        .h(px(40.))
        .w_full()
        .pl(px(PANEL_PADDING))
        .pr_2()
        .gap_1()
        .cursor_pointer()
        .hover(|style| style.bg(cx.theme().sidebar_accent.opacity(0.45)))
        .focus(|style| {
            style
                .bg(cx.theme().sidebar_accent.opacity(0.45))
                .border_color(cx.theme().selection)
        })
        .on_activate(move |_, _, cx| {
            header_events.send(&expected_target, AppearanceEvent::ToggleLayer, cx);
        })
        .child(
            div()
                .flex_1()
                .text_sm()
                .font_semibold()
                .child(section.label()),
        )
        .child(
            h_flex()
                .id(SharedString::from(format!(
                    "{}-appearance-header-actions",
                    projection.panel_id
                )))
                .gap_1()
                .on_click(|_, _, cx| cx.stop_propagation())
                .when_some(variable_mode_popover, |actions, browser| {
                    actions.child(browser)
                })
                .when(projection.capabilities.supports_visibility, |actions| {
                    actions.child(render_visibility_control(
                        &visibility,
                        "appearance-visible",
                        visibility_variable_button,
                        events,
                        cx,
                    ))
                })
                .when(
                    projection.capabilities.supports_layer_appearance,
                    |actions| actions.child(render_blend_popover(projection, events, cx)),
                ),
        )
        .into_any_element()
}

fn render_draw_layer(
    projection: &AppearanceProjection,
    chrome: &impl AppearanceInspectorChrome,
    visibility_variable_button: Option<AnyElement>,
    events: &AppearanceEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let mut content = v_flex()
        .pl(px(PANEL_PADDING))
        .pr_2()
        .pt(px(2.))
        .pb_4()
        .gap_2();
    if projection.capabilities.supports_visibility
        && !projection.capabilities.supports_layer_appearance
    {
        content = content.child(chrome.appearance_toggle_row(
            "draw-appearance-visible",
            "Visible",
            projection.values.visible,
            DesignPanelProperty::Visible,
            cx,
        ));
    }
    if projection.capabilities.supports_layer_appearance {
        let visibility = AppearanceVisibilityProjection {
            panel_id: projection.panel_id.clone(),
            target: projection.target.clone(),
            property: DesignPanelProperty::Visible,
            visibility: projection.values.visibility,
            editable: projection.capabilities.visibility_editable,
        };
        content = content.child(render_draw_slider_row(
            projection,
            chrome,
            "opacity",
            "Opacity",
            ValueFieldIcon::Opacity,
            DesignPanelProperty::Opacity,
            format!("{}%", format_number(projection.values.opacity)).into(),
            DesignPanelValue::Number(if projection.values.opacity <= 20. {
                100.
            } else {
                projection.values.opacity - 10.
            }),
            projection.presentation.opacity_slider.clone(),
            true,
            Some(render_visibility_control(
                &visibility,
                "draw-appearance-visible",
                visibility_variable_button,
                events,
                cx,
            )),
            events,
            cx,
        ));
    }
    if projection.capabilities.uniform_radius {
        content = content.child(render_draw_slider_row(
            projection,
            chrome,
            "corner-radius",
            "Corner radius",
            ValueFieldIcon::Corners,
            DesignPanelProperty::CornerRadius,
            format_number(projection.values.corner_radii[0]).into(),
            DesignPanelValue::Number(projection.values.corner_radii[0] + 4.),
            projection.presentation.corner_radius_slider.clone(),
            projection
                .slider_range(DesignPanelProperty::CornerRadius)
                .is_some(),
            projection
                .capabilities
                .independent_radii
                .then(|| render_corner_details_control(projection, chrome, events, cx)),
            events,
            cx,
        ));
    }

    let corner_details_visible = projection.corner_details_are_visible();
    if corner_details_visible && projection.capabilities.independent_radii {
        content = content
            .child(chrome.appearance_group_label("Corners", cx))
            .child(chrome.appearance_value_cell(
                "draw-independent-corners",
                "Corners",
                if projection.values.independent_corners {
                    "Individual"
                } else {
                    "All corners"
                },
                DesignPanelProperty::IndependentCorners,
                DesignPanelValue::Bool(!projection.values.independent_corners),
                cx,
            ));
    }
    if corner_details_visible && projection.capabilities.smoothing {
        let smoothing = projection.values.corner_smoothing.clamp(0., 1.);
        content = content
            .child(chrome.appearance_group_label("Corner smoothing", cx))
            .child(chrome.appearance_value_cell(
                "draw-corner-smoothing",
                "Smooth",
                format!("{}%", format_number(smoothing * 100.)),
                DesignPanelProperty::CornerSmoothing,
                DesignPanelValue::Ratio((smoothing + 0.1).min(1.)),
                cx,
            ));
    }
    if corner_details_visible
        && projection.capabilities.independent_radii
        && projection.values.independent_corners
    {
        content = content
            .child(chrome.appearance_group_label("Corners", cx))
            .child(
                h_flex()
                    .gap_2()
                    .child(chrome.appearance_value_cell(
                        "draw-corner-top-left",
                        "⌜",
                        format_number(projection.values.corner_radii[0]),
                        DesignPanelProperty::CornerRadiusTopLeft,
                        DesignPanelValue::Number(projection.values.corner_radii[0] + 1.),
                        cx,
                    ))
                    .child(chrome.appearance_value_cell(
                        "draw-corner-top-right",
                        "⌝",
                        format_number(projection.values.corner_radii[1]),
                        DesignPanelProperty::CornerRadiusTopRight,
                        DesignPanelValue::Number(projection.values.corner_radii[1] + 1.),
                        cx,
                    )),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(chrome.appearance_value_cell(
                        "draw-corner-bottom-left",
                        "⌞",
                        format_number(projection.values.corner_radii[3]),
                        DesignPanelProperty::CornerRadiusBottomLeft,
                        DesignPanelValue::Number(projection.values.corner_radii[3] + 1.),
                        cx,
                    ))
                    .child(chrome.appearance_value_cell(
                        "draw-corner-bottom-right",
                        "⌟",
                        format_number(projection.values.corner_radii[2]),
                        DesignPanelProperty::CornerRadiusBottomRight,
                        DesignPanelValue::Number(projection.values.corner_radii[2] + 1.),
                        cx,
                    )),
            );
    }
    if let Some(shape_controls) = render_shape_controls(projection, chrome, cx) {
        content = content.child(shape_controls);
    }
    if projection.capabilities.supports_layer_appearance {
        let next_blend = if projection.values.blend_mode == DesignBlendMode::Normal {
            DesignBlendMode::Multiply
        } else {
            DesignBlendMode::Normal
        };
        content = content
            .child(chrome.appearance_group_label("Blend mode", cx))
            .child(chrome.appearance_value_cell(
                "draw-blend-mode",
                "Blend",
                projection.values.blend_mode.label(),
                DesignPanelProperty::BlendMode,
                DesignPanelValue::BlendMode(next_blend),
                cx,
            ));
    }

    v_flex()
        .w_full()
        .flex_none()
        .border_b_1()
        .border_color(cx.theme().sidebar_border)
        .child(chrome.appearance_section_header(DesignPanelSection::Layer, cx))
        .when(projection.presentation.layer_expanded, |section| {
            section.child(content.into_any_element())
        })
        .into_any_element()
}

fn render_design_layer(
    projection: &AppearanceProjection,
    chrome: &impl AppearanceInspectorChrome,
    applied_component_controls: Option<AnyElement>,
    variable_mode_popover: Option<AnyElement>,
    visibility_variable_button: Option<AnyElement>,
    events: &AppearanceEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let mut content = v_flex()
        .pl(px(PANEL_PADDING))
        .pr_2()
        .pt(px(2.))
        .pb_4()
        .gap_2()
        .when_some(applied_component_controls, |content, applied| {
            content.child(applied)
        });
    if projection.capabilities.supports_layer_appearance || projection.capabilities.uniform_radius {
        let mut summary = h_flex().w_full().items_end().gap_2();
        if projection.capabilities.supports_layer_appearance {
            summary = summary.child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.))
                    .gap_1()
                    .child(chrome.appearance_group_label("Opacity", cx))
                    .child(chrome.appearance_icon_value_cell(
                        "opacity",
                        ValueFieldIcon::Opacity,
                        format!("{}%", format_number(projection.values.opacity)),
                        DesignPanelProperty::Opacity,
                        DesignPanelValue::Number(if projection.values.opacity <= 20. {
                            100.
                        } else {
                            projection.values.opacity - 10.
                        }),
                        cx,
                    )),
            );
        }
        if projection.capabilities.uniform_radius {
            summary = summary.child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.))
                    .gap_1()
                    .child(chrome.appearance_group_label("Corner radius", cx))
                    .child(chrome.appearance_icon_value_cell(
                        "corner-radius",
                        ValueFieldIcon::Corners,
                        format_number(projection.values.corner_radii[0]),
                        DesignPanelProperty::CornerRadius,
                        DesignPanelValue::Number(projection.values.corner_radii[0] + 4.),
                        cx,
                    )),
            );
            summary = if projection.capabilities.independent_radii {
                summary.child(render_corner_details_control(
                    projection, chrome, events, cx,
                ))
            } else {
                summary.child(div().size(px(24.)).flex_none())
            };
        }
        content = content.child(summary);
    }

    let corner_details_visible = projection.corner_details_are_visible();
    if corner_details_visible && projection.capabilities.independent_radii {
        content = content
            .child(chrome.appearance_group_label("Corners", cx))
            .child(chrome.appearance_value_cell(
                "independent-corners",
                "Corners",
                if projection.values.independent_corners {
                    "Individual"
                } else {
                    "All corners"
                },
                DesignPanelProperty::IndependentCorners,
                DesignPanelValue::Bool(!projection.values.independent_corners),
                cx,
            ));
    }
    if corner_details_visible && projection.capabilities.smoothing {
        let smoothing = projection.values.corner_smoothing.clamp(0., 1.);
        content = content
            .child(chrome.appearance_group_label("Corner smoothing", cx))
            .child(chrome.appearance_value_cell(
                "corner-smoothing",
                "Smooth",
                format!("{}%", format_number(smoothing * 100.)),
                DesignPanelProperty::CornerSmoothing,
                DesignPanelValue::Ratio((smoothing + 0.1).min(1.)),
                cx,
            ));
    }
    if corner_details_visible
        && projection.capabilities.independent_radii
        && projection.values.independent_corners
    {
        content = content
            .child(chrome.appearance_group_label("Corners", cx))
            .child(
                h_flex()
                    .gap_2()
                    .child(chrome.appearance_value_cell(
                        "corner-top-left",
                        "⌜",
                        format_number(projection.values.corner_radii[0]),
                        DesignPanelProperty::CornerRadiusTopLeft,
                        DesignPanelValue::Number(projection.values.corner_radii[0] + 1.),
                        cx,
                    ))
                    .child(chrome.appearance_value_cell(
                        "corner-top-right",
                        "⌝",
                        format_number(projection.values.corner_radii[1]),
                        DesignPanelProperty::CornerRadiusTopRight,
                        DesignPanelValue::Number(projection.values.corner_radii[1] + 1.),
                        cx,
                    )),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(chrome.appearance_value_cell(
                        "corner-bottom-left",
                        "⌞",
                        format_number(projection.values.corner_radii[3]),
                        DesignPanelProperty::CornerRadiusBottomLeft,
                        DesignPanelValue::Number(projection.values.corner_radii[3] + 1.),
                        cx,
                    ))
                    .child(chrome.appearance_value_cell(
                        "corner-bottom-right",
                        "⌟",
                        format_number(projection.values.corner_radii[2]),
                        DesignPanelProperty::CornerRadiusBottomRight,
                        DesignPanelValue::Number(projection.values.corner_radii[2] + 1.),
                        cx,
                    )),
            );
    }
    if let Some(shape_controls) = render_shape_controls(projection, chrome, cx) {
        content = content.child(shape_controls);
    }
    v_flex()
        .w_full()
        .flex_none()
        .border_b_1()
        .border_color(cx.theme().sidebar_border)
        .child(render_header(
            projection,
            variable_mode_popover,
            visibility_variable_button,
            events,
            cx,
        ))
        .when(projection.presentation.layer_expanded, |section| {
            section.child(content.into_any_element())
        })
        .into_any_element()
}

pub(in super::super) fn render(
    projection: &AppearanceProjection,
    chrome: &impl AppearanceInspectorChrome,
    applied_component_controls: Option<AnyElement>,
    variable_mode_popover: Option<AnyElement>,
    visibility_variable_button: Option<AnyElement>,
    events: &AppearanceEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    if projection.presentation.renders_draw_workspace {
        render_draw_layer(projection, chrome, visibility_variable_button, events, cx)
    } else {
        render_design_layer(
            projection,
            chrome,
            applied_component_controls,
            variable_mode_popover,
            visibility_variable_button,
            events,
            cx,
        )
    }
}

pub(in super::super) fn render_mask(
    projection: &AppearanceProjection,
    chrome: &impl AppearanceInspectorChrome,
    cx: &mut Context<DesignPanel>,
) -> Option<AnyElement> {
    if !projection.capabilities.supports_mask_section {
        return None;
    }
    let mask_type = projection.values.mask_type?;
    let next_mask_type = match mask_type {
        DesignMaskType::Alpha => DesignMaskType::Vector,
        DesignMaskType::Vector => DesignMaskType::Luminance,
        DesignMaskType::Luminance => DesignMaskType::Alpha,
    };
    let content =
        v_flex()
            .px(px(PANEL_PADDING))
            .pb_4()
            .gap_2()
            .child(chrome.appearance_value_cell(
                "mask-type",
                "Mask",
                mask_type.label(),
                DesignPanelProperty::MaskType,
                DesignPanelValue::MaskType(next_mask_type),
                cx,
            ));
    Some(chrome.appearance_section(DesignPanelSection::Mask, content.into_any_element(), cx))
}
