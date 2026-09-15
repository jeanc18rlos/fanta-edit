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

#[cfg(test)]
mod tests {
    use gpui::{Entity, TestAppContext, VisualTestContext};

    use crate::design::{DesignPanelNodeCapabilities, DesignTypography};
    use crate::test_support::{Mounted, ProbeHost, mount_component};

    use super::*;

    type Host = Entity<ProbeHost<DesignPanel, DesignPanelAction>>;

    /// Every section this shell dispatches for a fully capable node, paired
    /// with the leaves that only paint when the shell actually mounted that
    /// section.
    ///
    /// Section headers themselves carry no debug selector, so each entry names
    /// controls the section body owns. An entry may list alternatives because
    /// the same property renders either as a plain value cell or as a retained
    /// option menu depending on editability; with the one exception below, a
    /// section that stopped being dispatched paints none of them.
    ///
    /// The constraint entry names the labelled cell rather than the cell
    /// itself: an editable option cell is a retained `Select` and only its
    /// labelled presentation carries a debug selector, which is why [`setup`]
    /// turns the Additional labels preference on.
    ///
    /// That exception is the Constraints row, which is not exclusive on its
    /// own: the Position section paints the same cells inline while its own
    /// constraints disclosure is open
    /// (`sections::position::render` -> `render_constraints_controls`), so the
    /// Constraints row only distinguishes the dispatch once that disclosure is
    /// collapsed. Every presence assertion against this table collapses it
    /// first.
    const DISPATCHED_SECTION_LEAVES: &[(DesignPanelSection, &[&str])] = &[
        (DesignPanelSection::Position, &["design-x", "design-y"]),
        (
            DesignPanelSection::Layout,
            &["design-width", "design-height"],
        ),
        (
            DesignPanelSection::Constraints,
            &["design-horizontal-constraint-additional-label"],
        ),
        (DesignPanelSection::Layer, &["design-opacity-value"]),
        (DesignPanelSection::Typography, &["design-font-size"]),
        (DesignPanelSection::Fill, &["design-add-fill"]),
        (DesignPanelSection::Stroke, &["design-add-stroke"]),
        (DesignPanelSection::Effects, &["design-add-effect"]),
        (DesignPanelSection::LayoutGrid, &["design-add-layout-guide"]),
        (DesignPanelSection::Export, &["design-add-export"]),
    ];

    /// Compatibility-only identifiers the shell still resolves but must never
    /// project: `Media` was retired into the paint picker and `Geometry` only
    /// carries legacy table/mask leaves.
    const COMPATIBILITY_SECTIONS: &[DesignPanelSection] =
        &[DesignPanelSection::Geometry, DesignPanelSection::Media];

    /// The panel facade's own harness (`panel::tests::setup`) is private to
    /// that module, so the shell mounts the real `DesignPanel` through the
    /// shared crate probe host instead. Same contract: one real panel in a
    /// rooted window, rendered at least once before the test looks at it.
    fn setup(
        node: DesignPanelNode,
        cx: &mut TestAppContext,
    ) -> Mounted<'_, DesignPanel, DesignPanelAction> {
        let (host, actions, visual_cx) = mount_component(cx, move |window, cx| {
            DesignPanel::new("design", node, window, cx)
        });
        visual_cx.run_until_parked();
        // Additional labels only add a label beside each control, so every
        // other selector in the table is unchanged; the constraint cells are
        // observable at no other setting. See DISPATCHED_SECTION_LEAVES.
        let component = visual_cx.read(|app| host.read(app).component.clone());
        component.update(visual_cx, |panel, cx| panel.set_additional_labels(true, cx));
        visual_cx.run_until_parked();
        (host, actions, visual_cx)
    }

    fn panel(host: &Host, cx: &VisualTestContext) -> Entity<DesignPanel> {
        cx.read(|app| host.read(app).component.clone())
    }

    /// A node whose host capabilities claim every band the shell can
    /// dispatch, including the two compatibility identifiers.
    fn full_capability_node() -> DesignPanelNode {
        let sections = DISPATCHED_SECTION_LEAVES
            .iter()
            .map(|(section, _)| *section)
            .chain(COMPATIBILITY_SECTIONS.iter().copied());
        let mut node = DesignPanelNode::new("full", "Full capability", DesignPanelNodeKind::Frame);
        node.typography = Some(DesignTypography::default());
        node.with_capabilities(
            DesignPanelNodeCapabilities::for_node_kind(DesignPanelNodeKind::Frame)
                .with_sections(sections)
                .with_dimensions(true)
                .with_position_coordinates(true)
                .with_constraints(true)
                .with_visibility(true)
                .with_layer_appearance(true)
                .with_fill(true)
                .with_stroke(true)
                .with_effects(true)
                .with_layout_guides(true),
        )
    }

    /// The same fixture with the Constraints band withheld from the host's
    /// ordered section list. The node can still be constrained, so the only
    /// remaining painter of the constraint cells is the Position section's own
    /// inline disclosure.
    fn node_without_the_constraints_band() -> DesignPanelNode {
        let mut node = full_capability_node();
        let capabilities = node
            .capabilities
            .clone()
            .expect("the fixture carries an explicit capability snapshot");
        let sections = capabilities
            .sections
            .iter()
            .copied()
            .filter(|section| *section != DesignPanelSection::Constraints)
            .collect::<Vec<_>>();
        node.capabilities = Some(capabilities.with_sections(sections));
        node
    }

    fn resolved_sections(
        panel: &Entity<DesignPanel>,
        cx: &VisualTestContext,
    ) -> Vec<DesignPanelSection> {
        panel.read_with(cx, |panel, _| {
            panel
                .sections
                .resolve(
                    panel.host.inspected_node(),
                    panel.workspace_mode(),
                    panel.can_export(),
                )
                .into_iter()
                .map(|projection| projection.section)
                .collect()
        })
    }

    /// Closes the Position section's own constraints disclosure so the
    /// constraint cells can only come from the dispatched Constraints section.
    ///
    /// Without this the Constraints row of `DISPATCHED_SECTION_LEAVES` proves
    /// nothing: `sections::position::render` paints the very same cells inline
    /// whenever the node can be constrained and that disclosure is open.
    fn collapse_position_constraints_disclosure(
        panel: &Entity<DesignPanel>,
        cx: &mut VisualTestContext,
    ) {
        panel.update(cx, |panel, cx| {
            assert!(
                panel.sections.constraints_expanded(),
                "the disclosure starts open, so this really is the state that would let the \
                 Position section answer for the Constraints section",
            );
            panel.sections.toggle_constraints();
            cx.notify();
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn shell_renders_every_resolved_section_for_a_full_capability_node(cx: &mut TestAppContext) {
        let (host, _actions, visual_cx) = setup(full_capability_node(), cx);
        let panel = panel(&host, visual_cx);

        let expected = DISPATCHED_SECTION_LEAVES
            .iter()
            .map(|(section, _)| *section)
            .chain(COMPATIBILITY_SECTIONS.iter().copied())
            .collect::<Vec<_>>();
        assert_eq!(
            resolved_sections(&panel, visual_cx),
            expected,
            "the fixture must really resolve every dispatched band, otherwise the render \
             assertions below are vacuous",
        );

        collapse_position_constraints_disclosure(&panel, visual_cx);

        for (section, candidates) in DISPATCHED_SECTION_LEAVES {
            assert!(
                candidates
                    .iter()
                    .copied()
                    .any(|selector| visual_cx.debug_bounds(selector).is_some()),
                "the shell resolved the {} section but mounted none of {candidates:?}",
                section.label(),
            );
        }

        panel.update(visual_cx, |panel, cx| {
            assert!(
                DesignPanelShellController::render_media(&*panel, cx).is_none(),
                "the retired Media path stays inert: image and video controls belong to the \
                 canonical paint picker, so the panel must not project node-level media here",
            );
            let shape = sections::shape::ShapeProjection::from_node(
                panel.id.clone(),
                panel.can_edit(),
                panel.host.inspected_node(),
            );
            assert!(
                sections::shape::render_geometry(&shape, &*panel, cx).is_none(),
                "a resolved Geometry identifier with no legacy table or mask leaf must project \
                 nothing rather than an empty section",
            );
        });
    }

    #[gpui::test]
    fn constraint_cells_follow_the_position_disclosure_when_the_band_is_not_dispatched(
        cx: &mut TestAppContext,
    ) {
        let (host, _actions, visual_cx) = setup(node_without_the_constraints_band(), cx);
        let panel = panel(&host, visual_cx);

        assert!(
            !resolved_sections(&panel, visual_cx).contains(&DesignPanelSection::Constraints),
            "the fixture must really withhold the band, otherwise the shell's own dispatch would \
             answer for the cells below",
        );
        assert!(
            visual_cx
                .debug_bounds("design-horizontal-constraint-additional-label")
                .is_some(),
            "a constrainable node paints the constraint cells from the Position section's inline \
             disclosure even with no Constraints band dispatched: that shared leaf is exactly why \
             the Constraints row of DISPATCHED_SECTION_LEAVES is only read with the disclosure \
             collapsed",
        );

        collapse_position_constraints_disclosure(&panel, visual_cx);

        let (_, constraint_leaves) = DISPATCHED_SECTION_LEAVES
            .iter()
            .find(|(section, _)| *section == DesignPanelSection::Constraints)
            .expect("the table names the leaves the Constraints dispatch must own");
        for selector in constraint_leaves.iter().copied() {
            assert!(
                visual_cx.debug_bounds(selector).is_none(),
                "`{selector}` survived with neither the Constraints dispatch nor the Position \
                 disclosure left to paint it, so its presence can never prove either one",
            );
        }
        assert!(
            visual_cx.debug_bounds("design-x").is_some(),
            "only the constraint block closed: the rest of the Position section still renders",
        );
    }

    #[gpui::test]
    fn host_owned_surface_renders_no_inspector_projection(cx: &mut TestAppContext) {
        let (host, _actions, visual_cx) = setup(full_capability_node(), cx);
        let panel = panel(&host, visual_cx);

        assert!(
            panel.read_with(visual_cx, |panel, _| panel.renders_inspector_projection()),
            "the editor's Design surface is the inspector projection",
        );
        collapse_position_constraints_disclosure(&panel, visual_cx);
        for (section, candidates) in DISPATCHED_SECTION_LEAVES {
            assert!(
                candidates
                    .iter()
                    .copied()
                    .any(|selector| visual_cx.debug_bounds(selector).is_some()),
                "the {} section must be mounted before the surface change, otherwise its \
                 absence afterwards proves nothing",
                section.label(),
            );
        }

        panel.update(visual_cx, |panel, cx| {
            assert!(panel.set_active_surface(DesignPanelSurface::Prototype, cx));
        });
        visual_cx.run_until_parked();

        assert!(
            !panel.read_with(visual_cx, |panel, _| panel.renders_inspector_projection()),
            "a host-owned surface is not an inspector projection",
        );
        assert!(
            visual_cx
                .debug_bounds("design-surface-prototype-host-owned")
                .is_some(),
            "the shell must say the surface is host owned rather than render an empty body",
        );
        assert!(
            visual_cx
                .debug_bounds("design-tab-prototype-active")
                .is_some(),
            "the shared sidebar header survives: only the body below it is surrendered",
        );
        for (section, candidates) in DISPATCHED_SECTION_LEAVES {
            for selector in candidates.iter().copied() {
                assert!(
                    visual_cx.debug_bounds(selector).is_none(),
                    "`{selector}` from the {} section leaked into a host-owned surface",
                    section.label(),
                );
            }
        }
    }

    #[gpui::test]
    fn scroll_reset_arms_once_per_context_change(cx: &mut TestAppContext) {
        let first = DesignPanelNode::new("first", "First", DesignPanelNodeKind::Rectangle);
        let second = DesignPanelNode::new("second", "Second", DesignPanelNodeKind::Rectangle);
        let (host, _actions, visual_cx) = setup(first, cx);
        let panel = panel(&host, visual_cx);

        assert!(
            !panel.read_with(visual_cx, |panel, _| panel.shell.reset_after_render),
            "the mount frame consumes the initial arm; a panel that is already at the top must \
             not keep asking to be scrolled there",
        );

        panel.update(visual_cx, |panel, cx| {
            panel.set_inspection_context(
                DesignPanelInspectionContext::single(
                    second,
                    DesignPanelParentLayout::Freeform,
                    DesignPanelPermissions::editor(),
                ),
                cx,
            );
            assert!(
                panel.shell.reset_after_render,
                "a new inspection context invalidates the content the offset referred to, so \
                 the shell must arm the return to the top",
            );
        });
        visual_cx.run_until_parked();

        assert!(
            !panel.read_with(visual_cx, |panel, _| panel.shell.reset_after_render),
            "the armed reset is spent by the frame that scheduled it",
        );

        panel.update(visual_cx, |_, cx| cx.notify());
        visual_cx.run_until_parked();
        assert!(
            !panel.read_with(visual_cx, |panel, _| panel.shell.reset_after_render),
            "later frames must not re-arm: a shell that stayed armed would drag the reader back \
             to the top of the inspector on every unrelated redraw",
        );
    }
}
