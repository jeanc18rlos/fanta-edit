use super::super::*;
use super::position::PositionPanelCompat as _;

#[derive(Clone)]
pub(in super::super) struct LayoutIdentity {
    panel_id: SharedString,
    target: DesignPanelTarget,
}

impl LayoutIdentity {
    pub(in super::super) fn new(panel_id: SharedString, target: DesignPanelTarget) -> Self {
        Self { panel_id, target }
    }
}

#[derive(Clone, Copy)]
pub(in super::super) struct LayoutCapabilities {
    can_edit: bool,
    layout_mode_editable: bool,
    text_resize: Option<DesignTextResize>,
    supports_resize_to_fit: bool,
    supports_clip_content: bool,
}

impl LayoutCapabilities {
    pub(in super::super) const fn new(
        can_edit: bool,
        layout_mode_editable: bool,
        text_resize: Option<DesignTextResize>,
        supports_resize_to_fit: bool,
        supports_clip_content: bool,
    ) -> Self {
        Self {
            can_edit,
            layout_mode_editable,
            text_resize,
            supports_resize_to_fit,
            supports_clip_content,
        }
    }
}

#[derive(Clone)]
pub(in super::super) struct LayoutAutoLayoutProjection {
    owns_auto_layout: bool,
    participates_in_auto_layout: bool,
    ignored_by_auto_layout: bool,
    parent_direction: Option<DesignPanelAutoLayoutDirection>,
    add: Option<DesignAddAutoLayoutViewData>,
}

impl LayoutAutoLayoutProjection {
    pub(in super::super) fn new(
        owns_auto_layout: bool,
        participates_in_auto_layout: bool,
        ignored_by_auto_layout: bool,
        parent_direction: Option<DesignPanelAutoLayoutDirection>,
        add: Option<DesignAddAutoLayoutViewData>,
    ) -> Self {
        Self {
            owns_auto_layout,
            participates_in_auto_layout,
            ignored_by_auto_layout,
            parent_direction,
            add,
        }
    }

    fn shows_child_controls(&self) -> bool {
        self.participates_in_auto_layout && self.parent_direction.is_some()
    }

    fn shows_grid_child_tracks(&self) -> bool {
        self.shows_child_controls()
            && self.parent_direction == Some(DesignPanelAutoLayoutDirection::Grid)
    }
}

/// Grid-only snapshot kept separate from general auto-layout capability.
/// The equality check used by the renderer catches projection drift during
/// development without making the retained Grid picker inspect the facade.
#[derive(Clone, Copy)]
pub(in super::super) struct LayoutGridProjection {
    supports_auto_layout: bool,
    dimensions: Option<DesignGridDimensions>,
    automatic_rows: bool,
    column_count: usize,
    row_count: usize,
}

impl LayoutGridProjection {
    pub(in super::super) fn new(supports_auto_layout: bool, layout: &DesignLayout) -> Self {
        Self {
            supports_auto_layout,
            dimensions: layout.grid_dimensions(),
            automatic_rows: layout.grid_auto_tracks == DesignGridAutoTracks::Rows,
            column_count: layout.grid_columns.len(),
            row_count: layout.grid_rows.len(),
        }
    }

    fn matches(&self, layout: &DesignLayout) -> bool {
        self.dimensions == layout.grid_dimensions()
            && self.automatic_rows == (layout.grid_auto_tracks == DesignGridAutoTracks::Rows)
            && self.column_count == layout.grid_columns.len()
            && self.row_count == layout.grid_rows.len()
    }
}

#[derive(Clone, Copy)]
pub(in super::super) struct LayoutAxisLimitsProjection {
    dimension: f32,
    minimum: Option<f32>,
    maximum: Option<f32>,
    show_minimum: bool,
    show_maximum: bool,
}

impl LayoutAxisLimitsProjection {
    pub(in super::super) const fn new(
        dimension: f32,
        minimum: Option<f32>,
        maximum: Option<f32>,
        show_minimum: bool,
        show_maximum: bool,
    ) -> Self {
        Self {
            dimension,
            minimum,
            maximum,
            show_minimum,
            show_maximum,
        }
    }

    const fn is_visible(self) -> bool {
        self.show_minimum || self.show_maximum
    }
}

#[derive(Clone, Copy)]
pub(in super::super) struct LayoutDimensionProjection {
    lock_aspect_ratio_supported: bool,
    lock_aspect_ratio: bool,
    lock_aspect_ratio_editable: bool,
    width_limits: LayoutAxisLimitsProjection,
    height_limits: LayoutAxisLimitsProjection,
}

impl LayoutDimensionProjection {
    pub(in super::super) const fn new(
        lock_aspect_ratio_supported: bool,
        lock_aspect_ratio: bool,
        lock_aspect_ratio_editable: bool,
        width_limits: LayoutAxisLimitsProjection,
        height_limits: LayoutAxisLimitsProjection,
    ) -> Self {
        Self {
            lock_aspect_ratio_supported,
            lock_aspect_ratio,
            lock_aspect_ratio_editable,
            width_limits,
            height_limits,
        }
    }
}

#[derive(Clone)]
pub(in super::super) struct LayoutPresetProjection {
    catalog: Option<DesignFramePresetViewData>,
}

impl LayoutPresetProjection {
    pub(in super::super) fn new(catalog: Option<DesignFramePresetViewData>) -> Self {
        Self { catalog }
    }
}

#[derive(Clone)]
pub(in super::super) struct LayoutAuxiliaryProjection {
    dimensions: LayoutDimensionProjection,
    presets: LayoutPresetProjection,
}

impl LayoutAuxiliaryProjection {
    pub(in super::super) fn new(
        dimensions: LayoutDimensionProjection,
        presets: LayoutPresetProjection,
    ) -> Self {
        Self {
            dimensions,
            presets,
        }
    }
}

#[derive(Clone, Copy)]
pub(in super::super) struct LayoutPresentation {
    renders_draw_workspace: bool,
    section_expanded: bool,
    padding_editor_mode: PaddingEditorMode,
}

impl LayoutPresentation {
    pub(in super::super) const fn new(
        renders_draw_workspace: bool,
        section_expanded: bool,
        padding_editor_mode: PaddingEditorMode,
    ) -> Self {
        Self {
            renders_draw_workspace,
            section_expanded,
            padding_editor_mode,
        }
    }
}

/// Immutable section-owned state consumed by the Layout renderer.
///
/// `DesignLayout` is the Layout domain's compact value object: it owns the
/// padding, Grid tracks, sizing and auto-layout values rendered by this
/// surface. Catalog, disclosure, capability and presentation state live in
/// focused subprojections. The renderer never reads the Design-panel facade.
#[derive(Clone)]
pub(in super::super) struct LayoutProjection {
    identity: LayoutIdentity,
    layout: DesignLayout,
    capabilities: LayoutCapabilities,
    auto_layout: LayoutAutoLayoutProjection,
    grid: LayoutGridProjection,
    auxiliary: LayoutAuxiliaryProjection,
    presentation: LayoutPresentation,
}

impl LayoutProjection {
    pub(in super::super) fn new(
        identity: LayoutIdentity,
        layout: DesignLayout,
        capabilities: LayoutCapabilities,
        auto_layout: LayoutAutoLayoutProjection,
        grid: LayoutGridProjection,
        auxiliary: LayoutAuxiliaryProjection,
        presentation: LayoutPresentation,
    ) -> Self {
        debug_assert!(grid.matches(&layout));
        Self {
            identity,
            layout,
            capabilities,
            auto_layout,
            grid,
            auxiliary,
            presentation,
        }
    }
}

/// Narrow adapter to established field editors and complex retained overlays.
///
/// These helpers already validate every command against the current host
/// snapshot. Keeping them behind this surface lets the section renderer be
/// projection-only while the Grid picker, dimension menu and Frame preset
/// browser retain their focus/transaction continuity during migration.
pub(in super::super) trait LayoutInspectorChrome {
    fn layout_add_auto_layout(
        &self,
        view_data: &DesignAddAutoLayoutViewData,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn layout_group_label(&self, label: &'static str, cx: &mut Context<DesignPanel>) -> AnyElement;

    fn layout_text_resize_control(
        &self,
        resize: DesignTextResize,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn layout_frame_preset_browser(
        &self,
        catalog: &DesignFramePresetViewData,
        cx: &mut Context<DesignPanel>,
    ) -> Option<AnyElement>;

    fn layout_dimension_control(
        &self,
        axis: DesignLayoutDimensionAxis,
        layout: &DesignLayout,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn layout_position_action_icon(
        &self,
        icon: PositionActionIcon,
        enabled: bool,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn layout_icon_action_button(
        &self,
        id_suffix: &'static str,
        icon: IconName,
        action: DesignPanelAction,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn layout_value_cell(
        &self,
        id_suffix: impl Into<SharedString>,
        prefix: &'static str,
        value: impl Into<SharedString>,
        property: DesignPanelProperty,
        next: DesignPanelValue,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn layout_alignment_grid(&self, cx: &mut Context<DesignPanel>) -> AnyElement;

    fn layout_counter_axis_spacing_controls(
        &self,
        spacing: Option<f32>,
        primary_gap: f32,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn layout_padding_mode_button(
        &self,
        mode: PaddingEditorMode,
        label: &'static str,
        tooltip: &'static str,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn layout_toggle_row(
        &self,
        id_suffix: impl Into<SharedString>,
        label: &'static str,
        checked: bool,
        property: DesignPanelProperty,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn layout_grid_dimensions_picker(
        &self,
        layout: &DesignLayout,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn layout_grid_auto_rows_toggle(&self, cx: &mut Context<DesignPanel>) -> AnyElement;

    fn layout_grid_track_controls(
        &self,
        axis: DesignGridTrackAxis,
        layout: &DesignLayout,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn layout_checkbox_row(
        &self,
        id_suffix: impl Into<SharedString>,
        label: &'static str,
        checked: bool,
        property: DesignPanelProperty,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn layout_draw_header(&self, cx: &mut Context<DesignPanel>) -> AnyElement;

    fn layout_section(&self, content: AnyElement, cx: &mut Context<DesignPanel>) -> AnyElement;
}

impl LayoutInspectorChrome for DesignPanel {
    fn layout_add_auto_layout(
        &self,
        view_data: &DesignAddAutoLayoutViewData,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_add_auto_layout(view_data, cx)
    }

    fn layout_group_label(&self, label: &'static str, cx: &mut Context<DesignPanel>) -> AnyElement {
        self.render_group_label(label, cx)
    }

    fn layout_text_resize_control(
        &self,
        resize: DesignTextResize,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_text_resize_control(resize, cx)
    }

    fn layout_frame_preset_browser(
        &self,
        _catalog: &DesignFramePresetViewData,
        cx: &mut Context<DesignPanel>,
    ) -> Option<AnyElement> {
        self.render_frame_preset_browser(cx)
    }

    fn layout_dimension_control(
        &self,
        axis: DesignLayoutDimensionAxis,
        layout: &DesignLayout,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_dimension_control(axis, layout, cx)
    }

    fn layout_position_action_icon(
        &self,
        icon: PositionActionIcon,
        enabled: bool,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_position_action_icon(icon, enabled, cx)
    }

    fn layout_icon_action_button(
        &self,
        id_suffix: &'static str,
        icon: IconName,
        action: DesignPanelAction,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_icon_action_button(id_suffix, icon, action, cx)
    }

    fn layout_value_cell(
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

    fn layout_alignment_grid(&self, cx: &mut Context<DesignPanel>) -> AnyElement {
        self.render_alignment_grid(cx)
    }

    fn layout_counter_axis_spacing_controls(
        &self,
        spacing: Option<f32>,
        primary_gap: f32,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_counter_axis_spacing_controls(spacing, primary_gap, cx)
    }

    fn layout_padding_mode_button(
        &self,
        mode: PaddingEditorMode,
        label: &'static str,
        tooltip: &'static str,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_padding_mode_button(mode, label, tooltip, cx)
    }

    fn layout_toggle_row(
        &self,
        id_suffix: impl Into<SharedString>,
        label: &'static str,
        checked: bool,
        property: DesignPanelProperty,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_toggle_row(id_suffix, label, checked, property, cx)
    }

    fn layout_grid_dimensions_picker(
        &self,
        layout: &DesignLayout,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_grid_dimensions_picker(layout, cx)
    }

    fn layout_grid_auto_rows_toggle(&self, cx: &mut Context<DesignPanel>) -> AnyElement {
        self.render_grid_auto_rows_toggle(cx)
    }

    fn layout_grid_track_controls(
        &self,
        axis: DesignGridTrackAxis,
        layout: &DesignLayout,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_grid_track_controls(axis, layout, cx)
    }

    fn layout_checkbox_row(
        &self,
        id_suffix: impl Into<SharedString>,
        label: &'static str,
        checked: bool,
        property: DesignPanelProperty,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_checkbox_row(id_suffix, label, checked, property, cx)
    }

    fn layout_draw_header(&self, cx: &mut Context<DesignPanel>) -> AnyElement {
        self.render_draw_layout_header(cx)
    }

    fn layout_section(&self, content: AnyElement, cx: &mut Context<DesignPanel>) -> AnyElement {
        self.render_section(DesignPanelSection::Layout, None, content, cx)
    }
}

#[derive(Clone)]
enum LayoutEvent {
    Property(DesignPanelProperty, DesignPanelValue),
}

/// Typed event channel retained by the Layout renderer.
#[derive(Clone)]
pub(in super::super) struct LayoutEventSink {
    panel: Entity<DesignPanel>,
}

impl LayoutEventSink {
    pub(in super::super) fn new(panel: Entity<DesignPanel>) -> Self {
        Self { panel }
    }

    fn send(&self, event: LayoutEvent, cx: &mut App) {
        self.panel
            .update(cx, |panel, cx| dispatch(panel, event, cx));
    }
}

fn dispatch(panel: &mut DesignPanel, event: LayoutEvent, cx: &mut Context<DesignPanel>) {
    match event {
        LayoutEvent::Property(property, value) => {
            // `emit_property` validates editability, capability and value shape
            // against the latest host snapshot before emitting the intent.
            panel.emit_property(property, value, cx);
        }
    }
}

fn render_dimension_limit_fields(
    limits: LayoutAxisLimitsProjection,
    axis: DesignLayoutDimensionAxis,
    chrome: &impl LayoutInspectorChrome,
    cx: &mut Context<DesignPanel>,
) -> Option<AnyElement> {
    if !limits.is_visible() {
        return None;
    }
    let (
        minimum_id,
        maximum_id,
        minimum_prefix,
        maximum_prefix,
        minimum_property,
        maximum_property,
    ) = match axis {
        DesignLayoutDimensionAxis::Width => (
            "layout-min-width",
            "layout-max-width",
            "W≥",
            "W≤",
            DesignPanelProperty::MinWidth,
            DesignPanelProperty::MaxWidth,
        ),
        DesignLayoutDimensionAxis::Height => (
            "layout-min-height",
            "layout-max-height",
            "H≥",
            "H≤",
            DesignPanelProperty::MinHeight,
            DesignPanelProperty::MaxHeight,
        ),
    };
    let maximum_next = DesignPanelValue::OptionalNumber(Some(
        limits.dimension.max(limits.minimum.unwrap_or(1.)).max(1.),
    ));
    Some(
        h_flex()
            .w_full()
            .gap_2()
            .when(limits.show_minimum, |row| {
                row.child(chrome.layout_value_cell(
                    minimum_id,
                    minimum_prefix,
                    format_optional_number(limits.minimum),
                    minimum_property,
                    DesignPanelValue::OptionalNumber(Some(1.)),
                    cx,
                ))
            })
            .when(limits.show_maximum, |row| {
                row.child(chrome.layout_value_cell(
                    maximum_id,
                    maximum_prefix,
                    format_optional_number(limits.maximum),
                    maximum_property,
                    maximum_next,
                    cx,
                ))
            })
            .into_any_element(),
    )
}

fn render_lock_aspect_ratio_button(
    projection: &LayoutProjection,
    chrome: &impl LayoutInspectorChrome,
    events: &LayoutEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let enabled = projection.auxiliary.dimensions.lock_aspect_ratio_editable;
    let next = !projection.auxiliary.dimensions.lock_aspect_ratio;
    let mut button = div()
        .id(SharedString::from(format!(
            "{}-lock-aspect-ratio",
            projection.identity.panel_id
        )))
        .h(px(ROW_HEIGHT))
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .border_1()
        .border_color(cx.theme().transparent)
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
        });
    if enabled {
        let events = events.clone();
        button = button.on_activate(move |_, _, cx| {
            events.send(
                LayoutEvent::Property(
                    DesignPanelProperty::LockAspectRatio,
                    DesignPanelValue::Bool(next),
                ),
                cx,
            );
        });
    }
    button
        .child(chrome.layout_position_action_icon(PositionActionIcon::LockAspectRatio, enabled, cx))
        .into_any_element()
}

/// Render the complete Layout section from a stable read model.
pub(in super::super) fn render(
    projection: &LayoutProjection,
    chrome: &impl LayoutInspectorChrome,
    events: &LayoutEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let layout = &projection.layout;
    let direction_buttons = [
        (DesignLayoutMode::None, IconName::Frame),
        (DesignLayoutMode::Horizontal, IconName::ArrowRight),
        (DesignLayoutMode::Vertical, IconName::ArrowDown),
        (DesignLayoutMode::Grid, IconName::LayoutDashboard),
    ];
    let mut directions = h_flex()
        .h(px(ROW_HEIGHT))
        .w_full()
        .rounded(px(4.))
        .bg(cx.theme().secondary);
    for (mode, icon) in direction_buttons
        .into_iter()
        .filter(|(mode, _)| *mode != DesignLayoutMode::Grid || projection.grid.supports_auto_layout)
    {
        let mode_events = events.clone();
        directions = directions.child(
            div()
                .id(SharedString::from(format!(
                    "{}-layout-{}",
                    projection.identity.panel_id,
                    mode.label().to_lowercase()
                )))
                .flex_1()
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .when(
                    !projection.capabilities.can_edit
                        || !projection.capabilities.layout_mode_editable,
                    |button| button.opacity(0.62),
                )
                .when(layout.mode == mode, |button| {
                    button
                        .bg(cx.theme().accent)
                        .border_1()
                        .border_color(cx.theme().selection)
                })
                .when(
                    projection.capabilities.can_edit
                        && projection.capabilities.layout_mode_editable,
                    |button| {
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
                            .on_activate(move |_, _, cx| {
                                mode_events.send(
                                    LayoutEvent::Property(
                                        DesignPanelProperty::LayoutMode,
                                        DesignPanelValue::LayoutMode(mode),
                                    ),
                                    cx,
                                );
                            })
                    },
                )
                .child(Icon::new(icon).xsmall()),
        );
    }

    let mut content = v_flex().pl(px(PANEL_PADDING)).pr_2().pb_4().gap_1();
    if !projection.presentation.renders_draw_workspace
        && let Some(view_data) = projection.auto_layout.add.as_ref()
    {
        content = content.child(chrome.layout_add_auto_layout(view_data, cx));
    }
    if projection.auto_layout.owns_auto_layout {
        if projection.presentation.renders_draw_workspace {
            let flow_id = SharedString::from(format!(
                "{}-draw-layout-flow-label",
                projection.identity.panel_id
            ));
            let flow_selector = flow_id.to_string();
            content = content.child(
                div()
                    .id(flow_id)
                    .debug_selector(move || flow_selector.clone())
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Flow"),
            );
        }
        content = content.child(directions);
    }
    if let Some(resize) = projection.capabilities.text_resize
        && !projection.auto_layout.owns_auto_layout
    {
        content = content
            .child(chrome.layout_group_label("Resizing", cx))
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .child(chrome.layout_text_resize_control(resize, cx)),
                    )
                    .child(div().size(px(24.)).flex_none()),
            );
    }
    if let Some(catalog) = projection.auxiliary.presets.catalog.as_ref()
        && let Some(frame_presets) = chrome.layout_frame_preset_browser(catalog, cx)
    {
        content = content
            .child(chrome.layout_group_label("Frame", cx))
            .child(frame_presets);
    }
    content = content
        .child(chrome.layout_group_label("Dimensions", cx))
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .child(
                    h_flex()
                        .flex_1()
                        .min_w(px(0.))
                        .gap_2()
                        .child(chrome.layout_dimension_control(
                            DesignLayoutDimensionAxis::Width,
                            layout,
                            cx,
                        ))
                        .child(chrome.layout_dimension_control(
                            DesignLayoutDimensionAxis::Height,
                            layout,
                            cx,
                        )),
                )
                .when(
                    projection.auxiliary.dimensions.lock_aspect_ratio_supported,
                    |row| {
                        row.child(
                            div()
                                .w(px(24.))
                                .flex_none()
                                .rounded(px(4.))
                                .when(
                                    projection.auxiliary.dimensions.lock_aspect_ratio,
                                    |button| {
                                        button
                                            .bg(cx.theme().selection.opacity(0.22))
                                            .text_color(cx.theme().selection)
                                    },
                                )
                                .child(render_lock_aspect_ratio_button(
                                    projection, chrome, events, cx,
                                )),
                        )
                    },
                )
                .when(
                    !projection.presentation.renders_draw_workspace
                        && projection.capabilities.supports_resize_to_fit,
                    |row| {
                        row.child(div().w(px(24.)).flex_none().child(
                            chrome.layout_icon_action_button(
                                "resize-to-fit",
                                IconName::Maximize,
                                DesignPanelAction::ResizeToFitRequested {
                                    target: projection.identity.target.clone(),
                                },
                                cx,
                            ),
                        ))
                    },
                ),
        );
    if let Some(width_limits) = render_dimension_limit_fields(
        projection.auxiliary.dimensions.width_limits,
        DesignLayoutDimensionAxis::Width,
        chrome,
        cx,
    ) {
        content = content.child(width_limits);
    }
    if let Some(height_limits) = render_dimension_limit_fields(
        projection.auxiliary.dimensions.height_limits,
        DesignLayoutDimensionAxis::Height,
        chrome,
        cx,
    ) {
        content = content.child(height_limits);
    }
    if projection.auto_layout.owns_auto_layout && layout.mode != DesignLayoutMode::None {
        let gap_controls = h_flex()
            .w_full()
            .gap_2()
            .when(layout.mode != DesignLayoutMode::Grid, |controls| {
                controls.child(chrome.layout_value_cell(
                    "layout-item-spacing-mode",
                    "⇔",
                    layout.item_spacing_mode.label(),
                    DesignPanelProperty::ItemSpacingMode,
                    DesignPanelValue::ItemSpacingMode(match layout.item_spacing_mode {
                        DesignItemSpacingMode::Fixed => DesignItemSpacingMode::Auto,
                        DesignItemSpacingMode::Auto => DesignItemSpacingMode::Fixed,
                    }),
                    cx,
                ))
            })
            .when(
                layout.mode == DesignLayoutMode::Grid
                    || layout.item_spacing_mode == DesignItemSpacingMode::Fixed,
                |controls| {
                    controls.child(chrome.layout_value_cell(
                        "layout-gap",
                        "H",
                        format_number(layout.gap),
                        DesignPanelProperty::Gap,
                        DesignPanelValue::Number(layout.gap + 2.),
                        cx,
                    ))
                },
            )
            .when(layout.mode == DesignLayoutMode::Grid, |controls| {
                controls.child(chrome.layout_value_cell(
                    "layout-counter-axis-gap",
                    "V",
                    format_optional_number(layout.counter_axis_gap),
                    DesignPanelProperty::CounterAxisGap,
                    DesignPanelValue::OptionalNumber(Some(
                        layout.counter_axis_gap.unwrap_or(0.) + 2.,
                    )),
                    cx,
                ))
            });
        if layout.mode == DesignLayoutMode::Grid {
            content = content
                .child(chrome.layout_group_label("Gap", cx))
                .child(gap_controls);
        } else {
            content = content
                .child(
                    h_flex()
                        .w_full()
                        .gap_3()
                        .child(
                            div()
                                .w(px(88.))
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("Alignment"),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("Gap"),
                        ),
                )
                .child(
                    h_flex()
                        .gap_3()
                        .items_start()
                        .child(chrome.layout_alignment_grid(cx))
                        .child(v_flex().flex_1().gap_2().child(gap_controls)),
                );
            if layout.mode == DesignLayoutMode::Horizontal && layout.wrap {
                let next_alignment = match layout.counter_axis_align_content {
                    DesignCounterAxisAlignContent::Auto => {
                        DesignCounterAxisAlignContent::SpaceBetween
                    }
                    DesignCounterAxisAlignContent::SpaceBetween => {
                        DesignCounterAxisAlignContent::Auto
                    }
                };
                content = content
                    .child(chrome.layout_group_label("Wrapped tracks", cx))
                    .child(
                        v_flex()
                            .w_full()
                            .gap_1()
                            .child(chrome.layout_value_cell(
                                "layout-counter-axis-align-content",
                                "V",
                                layout.counter_axis_align_content.label(),
                                DesignPanelProperty::CounterAxisAlignContent,
                                DesignPanelValue::CounterAxisAlignContent(next_alignment),
                                cx,
                            ))
                            .when(
                                layout.counter_axis_align_content
                                    == DesignCounterAxisAlignContent::Auto,
                                |controls| {
                                    controls.child(chrome.layout_counter_axis_spacing_controls(
                                        layout.counter_axis_gap,
                                        layout.gap,
                                        cx,
                                    ))
                                },
                            ),
                    );
            }
        }

        let padding_controls = match projection.presentation.padding_editor_mode {
            PaddingEditorMode::Axes => h_flex()
                .w_full()
                .gap_2()
                .child(chrome.layout_value_cell(
                    "layout-padding-vertical",
                    "V",
                    format_number(layout.padding[0]),
                    DesignPanelProperty::PaddingVertical,
                    DesignPanelValue::Number(layout.padding[0] + 4.),
                    cx,
                ))
                .child(chrome.layout_value_cell(
                    "layout-padding-horizontal",
                    "H",
                    format_number(layout.padding[3]),
                    DesignPanelProperty::PaddingHorizontal,
                    DesignPanelValue::Number(layout.padding[3] + 4.),
                    cx,
                ))
                .into_any_element(),
            PaddingEditorMode::Individual => h_flex()
                .w_full()
                .gap_2()
                .child(chrome.layout_value_cell(
                    "layout-padding-top",
                    "T",
                    format_number(layout.padding[0]),
                    DesignPanelProperty::PaddingTop,
                    DesignPanelValue::Number(layout.padding[0] + 4.),
                    cx,
                ))
                .child(chrome.layout_value_cell(
                    "layout-padding-right",
                    "R",
                    format_number(layout.padding[1]),
                    DesignPanelProperty::PaddingRight,
                    DesignPanelValue::Number(layout.padding[1] + 4.),
                    cx,
                ))
                .child(chrome.layout_value_cell(
                    "layout-padding-bottom",
                    "B",
                    format_number(layout.padding[2]),
                    DesignPanelProperty::PaddingBottom,
                    DesignPanelValue::Number(layout.padding[2] + 4.),
                    cx,
                ))
                .child(chrome.layout_value_cell(
                    "layout-padding-left",
                    "L",
                    format_number(layout.padding[3]),
                    DesignPanelProperty::PaddingLeft,
                    DesignPanelValue::Number(layout.padding[3] + 4.),
                    cx,
                ))
                .into_any_element(),
            PaddingEditorMode::Shorthand => h_flex()
                .w_full()
                .gap_2()
                .child(chrome.layout_value_cell(
                    "layout-padding-shorthand",
                    "CSS",
                    format_padding_shorthand(layout.padding),
                    DesignPanelProperty::PaddingShorthand,
                    DesignPanelValue::NumberList(
                        layout.padding.iter().map(|value| value + 4.).collect(),
                    ),
                    cx,
                ))
                .into_any_element(),
        };
        content = content
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .child(chrome.layout_group_label("Padding", cx))
                    .child(
                        h_flex()
                            .gap_0p5()
                            .child(chrome.layout_padding_mode_button(
                                PaddingEditorMode::Axes,
                                "H/V",
                                "Horizontal and vertical padding",
                                cx,
                            ))
                            .child(chrome.layout_padding_mode_button(
                                PaddingEditorMode::Individual,
                                "TRBL",
                                "Individual top, right, bottom, and left padding",
                                cx,
                            ))
                            .child(chrome.layout_padding_mode_button(
                                PaddingEditorMode::Shorthand,
                                "CSS",
                                "Uniform or CSS shorthand padding",
                                cx,
                            )),
                    ),
            )
            .child(padding_controls)
            .when(layout.mode == DesignLayoutMode::Horizontal, |content| {
                content.child(chrome.layout_toggle_row(
                    "wrap",
                    "Wrap",
                    layout.wrap,
                    DesignPanelProperty::Wrap,
                    cx,
                ))
            })
            .child(chrome.layout_toggle_row(
                "include-strokes",
                "Include strokes in layout",
                layout.include_strokes,
                DesignPanelProperty::IncludeStrokes,
                cx,
            ));
        if layout.mode != DesignLayoutMode::Grid {
            content = content.child(
                h_flex()
                    .gap_2()
                    .child(chrome.layout_value_cell(
                        "stacking-order",
                        "Z",
                        layout.stacking_order.label(),
                        DesignPanelProperty::StackingOrder,
                        DesignPanelValue::StackingOrder(match layout.stacking_order {
                            DesignStackingOrder::LastOnTop => DesignStackingOrder::FirstOnTop,
                            DesignStackingOrder::FirstOnTop => DesignStackingOrder::LastOnTop,
                        }),
                        cx,
                    ))
                    .when(layout.mode == DesignLayoutMode::Horizontal, |row| {
                        row.child(chrome.layout_value_cell(
                            "baseline-alignment",
                            "A",
                            layout.baseline_alignment.label(),
                            DesignPanelProperty::BaselineAlignment,
                            DesignPanelValue::BaselineAlignment(match layout.baseline_alignment {
                                DesignBaselineAlignment::Bounds => {
                                    DesignBaselineAlignment::Baseline
                                }
                                DesignBaselineAlignment::Baseline => {
                                    DesignBaselineAlignment::Bounds
                                }
                            }),
                            cx,
                        ))
                    }),
            );
        }
        if layout.mode == DesignLayoutMode::Grid {
            content = content
                .child(div().pt_1().text_xs().font_semibold().child("Grid"))
                .child(chrome.layout_grid_dimensions_picker(layout, cx))
                .child(chrome.layout_grid_auto_rows_toggle(cx))
                .child(chrome.layout_group_label("Positioning", cx))
                .child(chrome.layout_value_cell(
                    "grid-items-positioning",
                    "↳",
                    layout.grid_items_positioning.label(),
                    DesignPanelProperty::GridItemsPositioning,
                    DesignPanelValue::GridItemsPositioning(match layout.grid_items_positioning {
                        DesignGridItemsPositioning::Manual => {
                            DesignGridItemsPositioning::RowAutoFlow
                        }
                        DesignGridItemsPositioning::RowAutoFlow => {
                            DesignGridItemsPositioning::Manual
                        }
                    }),
                    cx,
                ))
                .child(chrome.layout_grid_track_controls(DesignGridTrackAxis::Column, layout, cx))
                .child(chrome.layout_grid_track_controls(DesignGridTrackAxis::Row, layout, cx));
        }
    }

    if projection.auto_layout.shows_child_controls() {
        content = content
            .child(
                div()
                    .pt_1()
                    .text_xs()
                    .font_semibold()
                    .child("Auto-layout child"),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(chrome.layout_value_cell(
                        "layout-positioning",
                        "P",
                        layout.item.positioning.label(),
                        DesignPanelProperty::LayoutPositioning,
                        DesignPanelValue::LayoutPositioning(match layout.item.positioning {
                            DesignLayoutPositioning::InFlow => DesignLayoutPositioning::Absolute,
                            DesignLayoutPositioning::Absolute => DesignLayoutPositioning::InFlow,
                        }),
                        cx,
                    ))
                    .child(chrome.layout_value_cell(
                        "layout-align-self",
                        "H",
                        layout.item.align_self.label(),
                        DesignPanelProperty::LayoutAlignSelf,
                        DesignPanelValue::LayoutAlignSelf(match layout.item.align_self {
                            DesignLayoutAlignSelf::Inherit => DesignLayoutAlignSelf::Stretch,
                            DesignLayoutAlignSelf::Stretch => DesignLayoutAlignSelf::Inherit,
                        }),
                        cx,
                    )),
            )
            .child(h_flex().gap_2().child(chrome.layout_value_cell(
                "layout-grow",
                "Grow",
                format_number(layout.item.layout_grow),
                DesignPanelProperty::LayoutGrow,
                DesignPanelValue::Number(layout.item.layout_grow + 1.),
                cx,
            )));
        if projection.auto_layout.shows_grid_child_tracks() {
            content = content
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            chrome.layout_value_cell(
                                "grid-row-index",
                                "R",
                                (layout.item.grid_row_index + 1).to_string(),
                                DesignPanelProperty::GridRowIndex,
                                DesignPanelValue::Integer(
                                    i64::try_from(layout.item.grid_row_index)
                                        .unwrap_or(i64::MAX)
                                        .saturating_add(2),
                                ),
                                cx,
                            ),
                        )
                        .child(
                            chrome.layout_value_cell(
                                "grid-column-index",
                                "C",
                                (layout.item.grid_column_index + 1).to_string(),
                                DesignPanelProperty::GridColumnIndex,
                                DesignPanelValue::Integer(
                                    i64::try_from(layout.item.grid_column_index)
                                        .unwrap_or(i64::MAX)
                                        .saturating_add(2),
                                ),
                                cx,
                            ),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(chrome.layout_value_cell(
                            "grid-row-span",
                            "R×",
                            layout.item.grid_row_span.to_string(),
                            DesignPanelProperty::GridRowSpan,
                            DesignPanelValue::Integer(i64::from(
                                layout.item.grid_row_span.saturating_add(1),
                            )),
                            cx,
                        ))
                        .child(chrome.layout_value_cell(
                            "grid-column-span",
                            "C×",
                            layout.item.grid_column_span.to_string(),
                            DesignPanelProperty::GridColumnSpan,
                            DesignPanelValue::Integer(i64::from(
                                layout.item.grid_column_span.saturating_add(1),
                            )),
                            cx,
                        )),
                );
        }
    }
    if projection.auto_layout.ignored_by_auto_layout {
        content = content
            .child(
                div()
                    .pt_1()
                    .text_xs()
                    .font_semibold()
                    .child("Auto-layout child"),
            )
            .child(chrome.layout_value_cell(
                "layout-positioning",
                "P",
                DesignLayoutPositioning::Absolute.label(),
                DesignPanelProperty::LayoutPositioning,
                DesignPanelValue::LayoutPositioning(DesignLayoutPositioning::InFlow),
                cx,
            ));
    }
    if projection.capabilities.supports_clip_content {
        content = content.child(chrome.layout_checkbox_row(
            "clip-content",
            "Clip content",
            layout.clip_content,
            DesignPanelProperty::ClipContent,
            cx,
        ));
    }
    if projection.presentation.renders_draw_workspace {
        v_flex()
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(cx.theme().sidebar_border)
            .child(chrome.layout_draw_header(cx))
            .when(projection.presentation.section_expanded, |section| {
                section.child(content.into_any_element())
            })
            .into_any_element()
    } else {
        chrome.layout_section(content.into_any_element(), cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projection(parent_direction: Option<DesignPanelAutoLayoutDirection>) -> LayoutProjection {
        let layout = DesignLayout::default();
        LayoutProjection::new(
            LayoutIdentity::new(
                "layout-test".into(),
                DesignPanelTarget::Nodes {
                    node_ids: vec!["node".into()],
                },
            ),
            layout.clone(),
            LayoutCapabilities::new(true, true, None, true, true),
            LayoutAutoLayoutProjection::new(
                true,
                parent_direction.is_some(),
                false,
                parent_direction,
                None,
            ),
            LayoutGridProjection::new(true, &layout),
            LayoutAuxiliaryProjection::new(
                LayoutDimensionProjection::new(
                    true,
                    false,
                    true,
                    LayoutAxisLimitsProjection::new(100., None, None, false, false),
                    LayoutAxisLimitsProjection::new(100., None, None, false, false),
                ),
                LayoutPresetProjection::new(None),
            ),
            LayoutPresentation::new(false, true, PaddingEditorMode::Axes),
        )
    }

    #[test]
    fn grid_child_controls_require_current_grid_parent_projection() {
        let grid = projection(Some(DesignPanelAutoLayoutDirection::Grid));
        assert!(grid.auto_layout.shows_child_controls());
        assert!(grid.auto_layout.shows_grid_child_tracks());

        let horizontal = projection(Some(DesignPanelAutoLayoutDirection::Horizontal));
        assert!(horizontal.auto_layout.shows_child_controls());
        assert!(!horizontal.auto_layout.shows_grid_child_tracks());

        let detached = projection(None);
        assert!(!detached.auto_layout.shows_child_controls());
        assert!(!detached.auto_layout.shows_grid_child_tracks());
    }

    #[test]
    fn layout_domain_snapshot_keeps_padding_and_grid_tracks_together() {
        let mut projected = projection(None);
        projected.layout.padding = [1., 2., 3., 4.];
        projected.layout.grid_columns = vec![DesignGridTrack::fixed(120.)];
        projected.layout.grid_rows = vec![DesignGridTrack::fraction(1.)];

        assert_eq!(projected.layout.padding, [1., 2., 3., 4.]);
        assert_eq!(projected.layout.grid_columns.len(), 1);
        assert_eq!(projected.layout.grid_rows.len(), 1);
        assert!(!projected.grid.matches(&projected.layout));

        projected.grid = LayoutGridProjection::new(true, &projected.layout);
        assert!(projected.grid.matches(&projected.layout));
    }

    #[test]
    fn dimension_limit_projection_keeps_values_and_disclosure_atomic() {
        let hidden = LayoutAxisLimitsProjection::new(320., Some(80.), Some(640.), false, false);
        assert!(!hidden.is_visible());

        let disclosed = LayoutAxisLimitsProjection::new(320., Some(80.), Some(640.), true, false);
        assert!(disclosed.is_visible());
        assert_eq!(disclosed.dimension, 320.);
        assert_eq!(disclosed.minimum, Some(80.));
        assert_eq!(disclosed.maximum, Some(640.));
    }
}
