use super::super::super::DesignViewerPropertySection;
use super::super::*;

/// Narrow immutable projection consumed by the Viewer section renderer.
///
/// It intentionally contains no editable Design-node model. Event handlers
/// return through [`ViewerEventSink`], where the latest target and permissions
/// are validated before an action is emitted.
#[derive(Clone)]
pub(in super::super) struct ViewerProjection {
    panel_id: SharedString,
    can_copy: bool,
    view_data: Option<DesignViewerPropertiesViewData>,
}

impl ViewerProjection {
    pub(in super::super) fn new(
        panel_id: SharedString,
        can_edit: bool,
        can_copy: bool,
        target: DesignPanelTarget,
        view_data: Option<DesignViewerPropertiesViewData>,
    ) -> Self {
        Self {
            panel_id,
            can_copy,
            view_data: view_data.filter(|view_data| {
                !can_edit && view_data.target == target && view_data.is_valid()
            }),
        }
    }
}

fn view_data_for_context(panel: &DesignPanel) -> Option<&DesignViewerPropertiesViewData> {
    let view_data = panel.host.projections.viewer_properties.as_ref()?;
    (!panel.host.inspection_context.permissions().can_edit()
        && view_data.target == panel.command_target()
        && view_data.is_valid())
    .then_some(view_data)
}

pub(in super::super) fn emit_row_copy(
    panel: &mut DesignPanel,
    section_id: SharedString,
    row_id: SharedString,
    property: DesignPanelProperty,
    displayed_value: SharedString,
    cx: &mut Context<DesignPanel>,
) {
    let current = view_data_for_context(panel)
        .and_then(|view_data| view_data.section(section_id.as_ref()))
        .and_then(|section| section.row(row_id.as_ref()))
        .is_some_and(|row| {
            row.property == Some(property) && row.displayed_value == displayed_value
        });
    if current {
        panel.emit_property_copy(property, displayed_value, cx);
    }
}

pub(in super::super) fn emit_section_copy(
    panel: &mut DesignPanel,
    section_id: SharedString,
    copy_value: SharedString,
    cx: &mut Context<DesignPanel>,
) {
    if panel.host.inspection_context.permissions().can_edit()
        || !panel.host.inspection_context.permissions().can_copy()
    {
        return;
    }
    let Some(view_data) = view_data_for_context(panel) else {
        return;
    };
    let Some(section) = view_data.section(section_id.as_ref()) else {
        return;
    };
    if section.copy_value.as_ref() != Some(&copy_value) {
        return;
    }
    cx.emit_design_panel_action(
        panel,
        DesignPanelAction::ViewerSectionCopyRequested {
            target: view_data.target.clone(),
            section_id,
            copy_value,
        },
    );
}

pub(in super::super) fn emit_section_representation_change(
    panel: &mut DesignPanel,
    section_id: SharedString,
    representation: DesignViewerColorRepresentation,
    cx: &mut Context<DesignPanel>,
) {
    if panel.host.inspection_context.permissions().can_edit()
        || !panel.host.inspection_context.permissions().can_copy()
    {
        return;
    }
    let Some(view_data) = view_data_for_context(panel) else {
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
        panel,
        DesignPanelAction::ViewerSectionRepresentationChangeRequested {
            target: view_data.target.clone(),
            section_id,
            representation,
        },
    );
}

#[derive(Clone)]
pub(in super::super) struct ViewerEventSink {
    panel: Entity<DesignPanel>,
}

impl ViewerEventSink {
    pub(in super::super) fn new(panel: Entity<DesignPanel>) -> Self {
        Self { panel }
    }

    fn copy_section(&self, section_id: SharedString, copy_value: SharedString, cx: &mut App) {
        self.panel.update(cx, |panel, cx| {
            emit_section_copy(panel, section_id, copy_value, cx);
        });
    }

    fn copy_row(
        &self,
        section_id: SharedString,
        row_id: SharedString,
        property: DesignPanelProperty,
        displayed_value: SharedString,
        cx: &mut App,
    ) {
        self.panel.update(cx, |panel, cx| {
            emit_row_copy(panel, section_id, row_id, property, displayed_value, cx);
        });
    }

    fn change_representation(
        &self,
        section_id: SharedString,
        representation: DesignViewerColorRepresentation,
        cx: &mut App,
    ) {
        self.panel.update(cx, |panel, cx| {
            emit_section_representation_change(panel, section_id, representation, cx);
        });
    }
}

fn render_property_section(
    projection: &ViewerProjection,
    event_sink: &ViewerEventSink,
    section: DesignViewerPropertySection,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let can_copy = projection.can_copy;
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
        let event_sink = event_sink.clone();
        let copy_section_id = section.id.clone();
        header = header.child(
            Button::new(SharedString::from(format!(
                "{}-viewer-copy-section-{}",
                projection.panel_id, section.id
            )))
            .label(section.copy_label.clone())
            .tooltip(SharedString::from(format!("Copy {section_title}")))
            .xsmall()
            .compact()
            .ghost()
            .on_activate(move |_, _, cx| {
                event_sink.copy_section(copy_section_id.clone(), copy_value.clone(), cx);
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
                let event_sink = event_sink.clone();
                let representation_section_id = section.id.clone();
                representations = representations.child(
                    Button::new(SharedString::from(format!(
                        "{}-viewer-representation-{}-{}",
                        projection.panel_id,
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
                        event_sink.change_representation(
                            representation_section_id.clone(),
                            representation,
                            cx,
                        );
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
                projection.panel_id, section_id, row.id
            )))
            .debug_selector({
                let selector = format!(
                    "{}-viewer-row-{}-{}",
                    projection.panel_id, section_id, row.id
                );
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
            let event_sink = event_sink.clone();
            element = element.on_activate(move |_, _, cx| {
                event_sink.copy_row(
                    section_id.clone(),
                    row_id.clone(),
                    property,
                    displayed_value.clone(),
                    cx,
                );
            });
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

pub(in super::super) fn render(
    projection: ViewerProjection,
    event_sink: ViewerEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let Some(view_data) = projection.view_data.as_ref() else {
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
                .map(|section| render_property_section(&projection, &event_sink, section, cx)),
        )
        .into_any_element()
}
