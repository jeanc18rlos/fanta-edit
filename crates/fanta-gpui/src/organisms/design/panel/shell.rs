//! Top-level Design inspector shell and section dispatch.

use super::*;

/// Scroll continuity owned by the top-level inspector shell.
///
/// The offset is presentation-only. `reset_after_render` is armed when the
/// shell should return to the top on the next frame, which happens when a new
/// inspection context replaces the content the current offset referred to.
pub(super) struct DesignPanelShellState {
    pub(super) scroll_handle: ScrollHandle,
    pub(super) reset_after_render: bool,
}

impl Default for DesignPanelShellState {
    fn default() -> Self {
        Self {
            scroll_handle: ScrollHandle::new(),
            reset_after_render: true,
        }
    }
}

pub(super) trait DesignPanelShellController: Sized {
    fn header_projection(&self) -> sections::header::HeaderProjection;
    fn toggle_section(&mut self, section: DesignPanelSection, cx: &mut Context<Self>);
    fn render_section_header(
        &self,
        section: DesignPanelSection,
        add: Option<DesignPanelCollection>,
        cx: &mut Context<Self>,
    ) -> AnyElement;
    fn render_section(
        &self,
        section: DesignPanelSection,
        add: Option<DesignPanelCollection>,
        content: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement;
    fn render_media(&self, cx: &mut Context<Self>) -> Option<AnyElement>;
    fn renders_inspector_projection(&self) -> bool;
    fn render_host_owned_surface(&self, cx: &mut Context<Self>) -> AnyElement;
    fn render_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement;
}

impl DesignPanelShellController for DesignPanel {
    fn header_projection(&self) -> sections::header::HeaderProjection {
        let selection = self.host.inspection_context.selection();
        sections::header::HeaderProjection::new(
            self.id.clone(),
            sections::header::HeaderNavigationProjection::new(
                self.available_surfaces().iter().copied(),
                self.active_surface(),
                self.host.navigation.workspace_mode,
                self.host.inspection_context.permissions().can_edit(),
            ),
            sections::header::HeaderSelectionProjection::new(
                selection.kind(),
                selection.len(),
                self.host.inspected_node().name.clone(),
                self.resolved_selection_header_view_data(),
                self.current_selection_header_target(),
                self.overlays.selection_header_overlay().clone(),
            ),
        )
    }

    fn toggle_section(&mut self, section: DesignPanelSection, cx: &mut Context<Self>) {
        self.sections.toggle(section);
        cx.notify();
    }

    fn render_section_header(
        &self,
        section: DesignPanelSection,
        add: Option<DesignPanelCollection>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let can_edit = self.can_edit();
        let section_empty = match section {
            DesignPanelSection::Fill => self.host.inspected_node().fills.is_empty(),
            DesignPanelSection::Stroke => self
                .host
                .inspected_node()
                .stroke
                .as_ref()
                .is_none_or(|stroke| stroke.paints.is_empty()),
            DesignPanelSection::Effects => self.host.inspected_node().effects.is_empty(),
            DesignPanelSection::Export => self
                .host
                .projections
                .export
                .as_ref()
                .is_none_or(|view_data| view_data.configurations.is_empty()),
            _ => false,
        };
        let can_add = add.is_some_and(|collection| {
            if !self.collection_is_supported(collection) {
                false
            } else if collection == DesignPanelCollection::Export {
                self.can_export()
            } else if collection == DesignPanelCollection::Effect {
                can_edit
                    && self.host.inspected_node().effect_style_binding.is_none()
                    && self
                        .host
                        .inspected_node()
                        .first_addable_effect_kind()
                        .is_some()
            } else if collection == DesignPanelCollection::LayoutGrid {
                can_edit
                    && self
                        .host
                        .inspected_node()
                        .layout_grid_style_binding
                        .is_none()
            } else if matches!(
                collection,
                DesignPanelCollection::Fill | DesignPanelCollection::Stroke
            ) {
                can_edit && self.paint_style_binding(collection).is_none()
            } else {
                can_edit
            }
        });
        let id = SharedString::from(format!(
            "{}-section-{}",
            self.id,
            section.label().to_lowercase().replace(' ', "-")
        ));
        crate::molecules::inspector_section_header(
            id,
            crate::molecules::InspectorMetrics::default(),
            cx,
        )
        .pl(px(PANEL_PADDING))
        .pr_2()
        .gap_1()
        .on_activate(cx.listener(move |this, _, _, cx| {
            this.toggle_section(section, cx);
        }))
        .child(
            div()
                .flex_1()
                .text_sm()
                .font_semibold()
                .when(section_empty, |title| {
                    title.text_color(cx.theme().muted_foreground)
                })
                .child(section.label()),
        )
        .when(section == DesignPanelSection::Typography, |header| {
            header.child(self.render_typography_style_button(cx))
        })
        .when(section == DesignPanelSection::Effects, |header| {
            header.child(self.render_effect_style_button(cx))
        })
        .when(
            section == DesignPanelSection::LayoutGrid
                && (!self.host.inspected_node().layout_grids.is_empty()
                    || self
                        .host
                        .inspected_node()
                        .layout_grid_style_binding
                        .is_some()),
            |header| header.child(self.render_layout_grid_style_button(cx)),
        )
        .when(
            matches!(
                section,
                DesignPanelSection::Fill | DesignPanelSection::Stroke
            ) && add.is_some_and(|collection| self.collection_is_supported(collection)),
            |header| {
                header.child(
                    self.render_paint_style_button(
                        add.expect("paint section has a collection"),
                        cx,
                    ),
                )
            },
        )
        .when_some(add.filter(|_| can_add), |header, collection| {
            let add_selector = format!(
                "{}-add-{}",
                self.id,
                collection.label().to_lowercase().replace(' ', "-")
            );
            header.child(
                crate::molecules::inspector_action_button(
                    SharedString::from(add_selector.clone()),
                    &crate::molecules::InspectorFieldAccess::Editable,
                    crate::molecules::InspectorMetrics::default(),
                    cx,
                )
                .debug_selector(move || add_selector.clone())
                .size(px(24.))
                .when(section_empty, |button| {
                    button.text_color(cx.theme().muted_foreground)
                })
                .on_activate(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.emit_add(collection, cx);
                }))
                .child(Icon::new(IconName::Plus).xsmall()),
            )
        })
        .into_any_element()
    }

    fn render_section(
        &self,
        section: DesignPanelSection,
        add: Option<DesignPanelCollection>,
        content: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let expanded = self.sections.is_expanded(section);
        crate::molecules::inspector_section(cx)
            .child(self.render_section_header(section, add, cx))
            .when(expanded, |section| section.child(content))
            .into_any_element()
    }

    fn render_media(&self, _cx: &mut Context<Self>) -> Option<AnyElement> {
        // Compatibility-only section identifier. Image/video controls are
        // rendered by the canonical paint picker, so the panel must not emit
        // node-level media intents from this retired path.
        None
    }

    fn renders_inspector_projection(&self) -> bool {
        if self.renders_draw_workspace() {
            return true;
        }
        matches!(
            (
                self.host.inspection_context.permissions().can_edit(),
                self.active_surface(),
            ),
            (true, DesignPanelSurface::Design) | (false, DesignPanelSurface::Properties)
        )
    }

    fn render_host_owned_surface(&self, cx: &mut Context<Self>) -> AnyElement {
        let surface = self.active_surface();
        let selector = format!("{}-surface-{}-host-owned", self.id, surface.slug());
        div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .w_full()
            .p_4()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(format!(
                "{} is a host-owned surface. DesignPanel retains the shared sidebar header and does not project Design or Properties content here.",
                surface.label()
            ))
            .into_any_element()
    }

    fn render_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.component_authoring.dialog_close_pending {
            self.component_authoring.dialog_close_pending = false;
            window.on_next_frame(|window, cx| {
                if window.has_active_dialog(cx) {
                    window.close_dialog(cx);
                }
            });
        }
        if self.shell.reset_after_render {
            self.shell.reset_after_render = false;
            let scroll_handle = self.shell.scroll_handle.clone();
            window.on_next_frame(move |window, _| {
                scroll_handle.set_offset(gpui::point(px(0.), px(0.)));
                window.refresh();
            });
        }
        let selection_kind = self.host.inspection_context.selection().kind();
        let renders_inspector_projection = self.renders_inspector_projection();
        if self.renders_draw_workspace() {
            self.sync_draw_appearance_sliders(window, cx);
        }
        if renders_inspector_projection
            && (selection_kind != DesignPanelSelectionKind::None || self.can_export())
        {
            self.sync_option_states(window, cx);
        }
        let viewer_projection = !self.host.inspection_context.permissions().can_edit();
        if renders_inspector_projection && !viewer_projection {
            self.sync_typography_style_picker(cx);
            self.sync_paint_picker(window, cx);
        }
        let mut body = v_flex().w_full();
        if !renders_inspector_projection {
            body = body.child(self.render_host_owned_surface(cx));
        } else if selection_kind == DesignPanelSelectionKind::None {
            body = body.child(self.render_page(cx));
            if self.can_export() {
                body = body.child(self.render_export(cx));
            }
        } else if viewer_projection {
            let permissions = self.host.inspection_context.permissions();
            let viewer = sections::viewer::ViewerProjection::new(
                self.id.clone(),
                permissions.can_edit(),
                permissions.can_copy(),
                self.command_target(),
                self.host.projections.viewer_properties.clone(),
            );
            body = body.child(sections::viewer::render(
                viewer,
                sections::viewer::ViewerEventSink::new(cx.entity()),
                cx,
            ));
            if self.can_export() {
                body = body.child(self.render_export(cx));
            }
        } else {
            let shape_projection = sections::shape::ShapeProjection::from_node(
                self.id.clone(),
                self.can_edit(),
                self.host.inspected_node(),
            );
            let shape_event_sink = sections::shape::ShapeEventSink::new(cx.entity());
            for projection in self.sections.resolve(
                self.host.inspected_node(),
                self.host.navigation.workspace_mode,
                self.can_export(),
            ) {
                let _family = projection.family;
                let rendered = match projection.section {
                    DesignPanelSection::Selection => self.render_selection_colors(cx),
                    DesignPanelSection::Component | DesignPanelSection::Instance => {
                        self.render_component(cx)
                    }
                    DesignPanelSection::Position => {
                        let projection = position::projection(self);
                        let events = sections::position::PositionEventSink::new(cx.entity());
                        Some(sections::position::render(&projection, self, &events, cx))
                    }
                    DesignPanelSection::Layout => Some(self.render_layout(cx)),
                    DesignPanelSection::Constraints => {
                        let projection = position::projection(self);
                        Some(sections::position::render_constraints(
                            &projection,
                            self,
                            cx,
                        ))
                    }
                    DesignPanelSection::Layer => Some(self.render_layer(cx)),
                    DesignPanelSection::Section => sections::shape::render_section_properties(
                        &shape_projection,
                        self,
                        shape_event_sink.clone(),
                        cx,
                    ),
                    DesignPanelSection::Transform => sections::shape::render_transform_modifiers(
                        &shape_projection,
                        self,
                        shape_event_sink.clone(),
                        cx,
                    ),
                    DesignPanelSection::Geometry => {
                        sections::shape::render_geometry(&shape_projection, self, cx)
                    }
                    DesignPanelSection::Mask => self.render_mask(cx),
                    DesignPanelSection::Typography => self.render_typography(cx),
                    DesignPanelSection::Media => self.render_media(cx),
                    DesignPanelSection::Fill => self.render_fill(cx),
                    DesignPanelSection::Stroke => self.render_stroke(cx),
                    DesignPanelSection::Effects => Some(self.render_effects(cx)),
                    DesignPanelSection::LayoutGrid => self.render_layout_grids(cx),
                    DesignPanelSection::Export => Some(self.render_export(cx)),
                };
                if let Some(rendered) = rendered {
                    body = body.child(rendered);
                }
            }
        }

        let scroll_handle = self.shell.scroll_handle.clone();
        v_flex()
            .id(self.id.clone())
            .key_context(DESIGN_PANEL_KEY_CONTEXT)
            .relative()
            .size_full()
            .min_h(px(0.))
            .track_focus(&self.focus_handle)
            .on_action(
                cx.listener(|this, _: &CancelDesignInteraction, window, cx| {
                    if this.edit.component_authoring.has_name_editor() {
                        this.finish_component_authoring_name_edit(false, window, cx);
                    } else if this.edit.component_authoring.has_property_reorder() {
                        this.finish_component_property_reorder(false, cx);
                    } else if this.edit.component_authoring.has_variant_option_reorder() {
                        this.finish_component_variant_option_reorder(false, cx);
                    } else if this.component_authoring.create_draft.is_some() {
                        this.cancel_component_property_create(window, cx);
                    } else if let Some(property) = this.edit.draw_slider_property {
                        this.finish_draw_appearance_slider(property, false, cx);
                    } else if this.edit.variable_font_axis.is_some() {
                        this.finish_variable_font_axis_edit(false, window, cx);
                    } else if this.edit.variable_font_axis_scrub.is_some() {
                        this.edit.variable_font_axis_scrub = None;
                        cx.notify();
                    } else if this.edit.property.is_some() {
                        this.finish_property_edit(false, window, cx);
                    } else if this.edit.component_multiline.is_some() {
                        this.finish_component_multiline_editor(false, window, cx);
                    } else if !this.dismiss_topmost_overlay(window, cx) {
                        cx.propagate();
                    }
                }),
            )
            .on_key_down(cx.listener(Self::handle_property_key_down))
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .text_sm()
            .child(self.render_header(cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h(px(0.))
                    .child(
                        body.id(SharedString::from(format!("{}-scroll", self.id)))
                            .size_full()
                            .min_h(px(0.))
                            .overflow_y_scroll()
                            .track_scroll(&self.shell.scroll_handle),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .h_full()
                            .child(Scrollbar::new(&scroll_handle).axis(ScrollbarAxis::Vertical)),
                    )
                    .when_some(self.render_scrub_speed_cue(cx), |container, cue| {
                        container.child(cue)
                    }),
            )
            .into_any_element()
    }
}
