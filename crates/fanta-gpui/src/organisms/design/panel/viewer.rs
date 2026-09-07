use super::*;

impl DesignPanel {
    pub(super) fn viewer_properties_view_data_for_context(
        &self,
    ) -> Option<&DesignViewerPropertiesViewData> {
        let view_data = self.viewer_properties_view_data.as_ref()?;
        (!self.inspection_context.permissions().can_edit()
            && view_data.target == self.command_target()
            && view_data.is_valid())
        .then_some(view_data)
    }

    pub(super) fn emit_viewer_row_copy(
        &mut self,
        section_id: SharedString,
        row_id: SharedString,
        property: DesignPanelProperty,
        displayed_value: SharedString,
        cx: &mut Context<Self>,
    ) {
        let current = self
            .viewer_properties_view_data_for_context()
            .and_then(|view_data| view_data.section(section_id.as_ref()))
            .and_then(|section| section.row(row_id.as_ref()))
            .is_some_and(|row| {
                row.property == Some(property) && row.displayed_value == displayed_value
            });
        if current {
            self.emit_property_copy(property, displayed_value, cx);
        }
    }

    pub(super) fn emit_viewer_section_copy(
        &mut self,
        section_id: SharedString,
        copy_value: SharedString,
        cx: &mut Context<Self>,
    ) {
        if self.inspection_context.permissions().can_edit()
            || !self.inspection_context.permissions().can_copy()
        {
            return;
        }
        let Some(view_data) = self.viewer_properties_view_data_for_context() else {
            return;
        };
        let Some(section) = view_data.section(section_id.as_ref()) else {
            return;
        };
        if section.copy_value.as_ref() != Some(&copy_value) {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::ViewerSectionCopyRequested {
                target: view_data.target.clone(),
                section_id,
                copy_value,
            },
        );
    }

    pub(super) fn emit_viewer_section_representation_change(
        &mut self,
        section_id: SharedString,
        representation: DesignViewerColorRepresentation,
        cx: &mut Context<Self>,
    ) {
        if self.inspection_context.permissions().can_edit()
            || !self.inspection_context.permissions().can_copy()
        {
            return;
        }
        let Some(view_data) = self.viewer_properties_view_data_for_context() else {
            return;
        };
        let Some(section) = view_data.section(section_id.as_ref()) else {
            return;
        };
        if section.color_representation.is_none()
            || section.color_representation == Some(representation)
        {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::ViewerSectionRepresentationChangeRequested {
                target: view_data.target.clone(),
                section_id,
                representation,
            },
        );
    }

    pub(super) fn render_viewer_property_section(
        &self,
        section: super::super::DesignViewerPropertySection,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let can_copy = self.inspection_context.permissions().can_copy();
        let section_id = section.id.clone();
        let section_title = section.title.clone();
        let mut header = h_flex()
            .h(px(40.))
            .w_full()
            .px(px(PANEL_PADDING))
            .gap_2()
            .items_center()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .text_sm()
                    .font_semibold()
                    .child(section.title.clone()),
            );
        if can_copy && let Some(copy_value) = section.copy_value.clone() {
            let panel = cx.entity();
            let copy_section_id = section.id.clone();
            header = header.child(
                Button::new(SharedString::from(format!(
                    "{}-viewer-copy-section-{}",
                    self.id, section.id
                )))
                .label(section.copy_label.clone())
                .tooltip(SharedString::from(format!("Copy {section_title}")))
                .xsmall()
                .compact()
                .ghost()
                .on_activate(move |_, _, cx| {
                    panel.update(cx, |this, cx| {
                        this.emit_viewer_section_copy(
                            copy_section_id.clone(),
                            copy_value.clone(),
                            cx,
                        );
                    });
                }),
            );
        }

        let mut body = v_flex().w_full().px(px(PANEL_PADDING)).pb_3().gap_1();
        if let Some(summary) = section.summary.clone() {
            body = body.child(
                div()
                    .w_full()
                    .min_h(px(ROW_HEIGHT))
                    .px_2()
                    .py_1()
                    .rounded(px(4.))
                    .bg(cx.theme().secondary)
                    .text_xs()
                    .child(summary),
            );
        }

        if let Some(active_representation) = section.color_representation {
            let mut representations = h_flex().w_full().gap_1();
            if can_copy {
                for representation in DesignViewerColorRepresentation::ALL {
                    let panel = cx.entity();
                    let representation_section_id = section.id.clone();
                    representations = representations.child(
                        Button::new(SharedString::from(format!(
                            "{}-viewer-representation-{}-{}",
                            self.id,
                            section.id,
                            representation.label().to_ascii_lowercase()
                        )))
                        .label(representation.label())
                        .tooltip(SharedString::from(format!(
                            "Represent {} as {}",
                            section.title,
                            representation.label()
                        )))
                        .xsmall()
                        .compact()
                        .ghost()
                        .selected(representation == active_representation)
                        .on_activate(move |_, _, cx| {
                            panel.update(cx, |this, cx| {
                                this.emit_viewer_section_representation_change(
                                    representation_section_id.clone(),
                                    representation,
                                    cx,
                                );
                            });
                        }),
                    );
                }
            } else {
                representations = representations.child(
                    div()
                        .h(px(ROW_HEIGHT))
                        .px_2()
                        .flex()
                        .items_center()
                        .rounded(px(4.))
                        .bg(cx.theme().secondary)
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(active_representation.label()),
                );
            }
            body = body.child(representations);
        }

        for row in section.rows {
            let copyable = can_copy && row.property.is_some();
            let row_id = row.id.clone();
            let displayed_value = row.displayed_value.clone();
            let mut element = h_flex()
                .id(SharedString::from(format!(
                    "{}-viewer-row-{}-{}",
                    self.id, section_id, row.id
                )))
                .debug_selector({
                    let selector = format!("{}-viewer-row-{}-{}", self.id, section_id, row.id);
                    move || selector.clone()
                })
                .h(px(ROW_HEIGHT))
                .w_full()
                .px_2()
                .gap_2()
                .rounded(px(4.))
                .bg(cx.theme().secondary)
                .child(
                    div()
                        .w(px(96.))
                        .flex_none()
                        .truncate()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(row.label),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_xs()
                        .child(row.displayed_value),
                )
                .when(copyable, |element| {
                    element
                        .key_context(CONTROL_KEY_CONTEXT)
                        .tab_index(0)
                        .cursor_pointer()
                        .hover(|style| style.bg(cx.theme().sidebar_accent.opacity(0.55)))
                        .focus(|style| {
                            style
                                .bg(cx.theme().sidebar_accent)
                                .border_color(cx.theme().selection)
                        })
                });
            if let Some(property) = row.property.filter(|_| copyable) {
                let section_id = section_id.clone();
                element = element.on_activate(cx.listener(move |this, _, _, cx| {
                    this.emit_viewer_row_copy(
                        section_id.clone(),
                        row_id.clone(),
                        property,
                        displayed_value.clone(),
                        cx,
                    );
                }));
            }
            body = body.child(element);
        }

        v_flex()
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(cx.theme().sidebar_border)
            .child(header)
            .child(body)
            .into_any_element()
    }

    pub(super) fn render_viewer_properties(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(view_data) = self.viewer_properties_view_data_for_context() else {
            return div()
                .w_full()
                .px(px(PANEL_PADDING))
                .py_4()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Supply DesignViewerPropertiesViewData for this exact selection")
                .into_any_element();
        };
        v_flex()
            .w_full()
            .children(
                view_data
                    .sections
                    .clone()
                    .into_iter()
                    .map(|section| self.render_viewer_property_section(section, cx)),
            )
            .into_any_element()
    }
}
