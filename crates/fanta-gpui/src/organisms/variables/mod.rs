//! Host-controlled variables manager with Figma UI3 table geometry.

use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, canvas, div,
    prelude::FluentBuilder as _, px, rgba,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};

use crate::{
    atoms::{CONTROL_KEY_CONTEXT, ControlExt as _, ControlIcon, icon_button, render_control_icon},
    color::parse_hex_rgba,
};

/// Reference design width of the collections/groups sidebar.
const SIDEBAR_WIDTH: f32 = 282.;
/// Floor the sidebar compresses to on narrow pages.
const SIDEBAR_MIN_WIDTH: f32 = 180.;
/// Fixed width of the search/share cluster on the right of the header row.
const HEADER_TOOLS_WIDTH: f32 = 320.;
/// Collection-title width preserved before the sidebar starts compressing.
const HEADER_TITLE_MIN_WIDTH: f32 = 120.;
/// Reference design width of the table's name and value columns.
const NAME_COLUMN_WIDTH: f32 = 201.;
const VALUE_COLUMN_WIDTH: f32 = 201.;
/// Floor the name column compresses to on narrow tables.
const NAME_COLUMN_MIN_WIDTH: f32 = 140.;
/// Fixed width of the add-mode cell at the end of the header row.
const ADD_MODE_WIDTH: f32 = 40.;

/// The smallest width the variables manager reflows to honestly: the
/// floored sidebar, the fixed header search/share cluster, and a usable
/// collection title. Below this the table's mode columns already scroll;
/// narrower pages would clip header chrome instead of compressing it.
pub const VARIABLES_PAGE_MIN_WIDTH: f32 = SIDEBAR_MIN_WIDTH + HEADER_TOOLS_WIDTH + 60.;
/// The smallest height the variables manager reflows to honestly: the
/// page header, the table header, one variable row, the create-variable
/// row, and a short scrollable sidebar/table region.
pub const VARIABLES_PAGE_MIN_HEIGHT: f32 = 400.;

/// A collection listed in the variables manager sidebar.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VariablesCollection {
    pub id: SharedString,
    pub name: SharedString,
    pub variable_count: usize,
}

impl VariablesCollection {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        variable_count: usize,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            variable_count,
        }
    }
}

/// A group filter belonging to the selected collection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VariablesGroup {
    pub id: SharedString,
    pub name: SharedString,
    pub variable_count: usize,
    pub is_aggregate: bool,
}

impl VariablesGroup {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        variable_count: usize,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            variable_count,
            is_aggregate: false,
        }
    }

    /// Marks this host-owned group as the aggregate row.
    pub fn aggregate(mut self) -> Self {
        self.is_aggregate = true;
        self
    }
}

/// One mode column in the variables table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VariablesMode {
    pub id: SharedString,
    pub name: SharedString,
}

impl VariablesMode {
    pub fn new(id: impl Into<SharedString>, name: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
        }
    }
}

/// Visual type shown before a variable name.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VariableKind {
    #[default]
    Color,
    Number,
    String,
    Boolean,
}

/// A value supplied for a particular mode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VariableModeValue {
    pub mode_id: SharedString,
    pub value: SharedString,
    pub color_hex: Option<SharedString>,
}

impl VariableModeValue {
    pub fn new(mode_id: impl Into<SharedString>, value: impl Into<SharedString>) -> Self {
        Self {
            mode_id: mode_id.into(),
            value: value.into(),
            color_hex: None,
        }
    }

    pub fn color(mut self, hex: impl Into<SharedString>) -> Self {
        self.color_hex = Some(hex.into());
        self
    }
}

/// One host-owned row in the variables table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VariableRow {
    pub id: SharedString,
    pub name: SharedString,
    pub group_id: SharedString,
    pub kind: VariableKind,
    pub values: Vec<VariableModeValue>,
}

impl VariableRow {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        group_id: impl Into<SharedString>,
        kind: VariableKind,
        values: impl IntoIterator<Item = VariableModeValue>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            group_id: group_id.into(),
            kind,
            values: values.into_iter().collect(),
        }
    }
}

/// Immutable host snapshot rendered by [`VariablesPage`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VariablesViewData {
    pub document_name: SharedString,
    pub collections: Vec<VariablesCollection>,
    pub selected_collection_id: SharedString,
    pub groups: Vec<VariablesGroup>,
    pub selected_group_id: SharedString,
    pub modes: Vec<VariablesMode>,
    pub variables: Vec<VariableRow>,
}

/// Requests emitted by the variables manager.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VariablesAction {
    CollectionSelected {
        collection_id: SharedString,
    },
    GroupSelected {
        group_id: SharedString,
    },
    SearchQueryChanged {
        query: SharedString,
    },
    SearchOptionsRequested,
    CreateCollectionRequested,
    CreateVariableRequested,
    AddModeRequested,
    ValueEditRequested {
        variable_id: SharedString,
        mode_id: SharedString,
    },
    ShareRequested,
    HelpRequested,
}

/// Stateful presentation for the host-controlled variables manager.
pub struct VariablesPage {
    id: SharedString,
    focus_handle: FocusHandle,
    view_data: VariablesViewData,
    search_input: Entity<InputState>,
    table_scroll_handle: ScrollHandle,
    /// Measured width of the whole page; the header row and the body derive
    /// shared sidebar and name-column widths from it so their vertical
    /// borders stay aligned while both compress on narrow pages.
    page_width: Option<f32>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<VariablesAction> for VariablesPage {}

impl VariablesPage {
    pub fn new(
        id: impl Into<SharedString>,
        view_data: VariablesViewData,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let subscriptions =
            vec![
                cx.subscribe(&search_input, |_, input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.emit(VariablesAction::SearchQueryChanged {
                            query: input.read(cx).value(),
                        });
                    }
                }),
            ];
        Self {
            id: id.into(),
            focus_handle: cx.focus_handle(),
            view_data,
            search_input,
            table_scroll_handle: ScrollHandle::new(),
            page_width: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn set_view_data(&mut self, view_data: VariablesViewData, cx: &mut Context<Self>) {
        self.view_data = view_data;
        cx.notify();
    }

    /// Shared sidebar width used by the header row and the body: the design
    /// width while the page is wide enough, compressing toward
    /// [`SIDEBAR_MIN_WIDTH`] once the header tools and title would clip.
    fn sidebar_width(&self) -> f32 {
        self.page_width.map_or(SIDEBAR_WIDTH, |width| {
            (width - HEADER_TOOLS_WIDTH - HEADER_TITLE_MIN_WIDTH)
                .clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_WIDTH)
        })
    }

    /// Shared name-column width used by the table header and rows: the design
    /// width while the table is wide enough, compressing toward
    /// [`NAME_COLUMN_MIN_WIDTH`] before the mode columns start scrolling.
    fn name_column_width(&self) -> f32 {
        self.page_width.map_or(NAME_COLUMN_WIDTH, |width| {
            let table_width = width - self.sidebar_width();
            (table_width - VALUE_COLUMN_WIDTH * self.view_data.modes.len() as f32 - ADD_MODE_WIDTH)
                .clamp(NAME_COLUMN_MIN_WIDTH, NAME_COLUMN_WIDTH)
        })
    }

    fn render_collection(
        &self,
        collection: &VariablesCollection,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = collection.id == self.view_data.selected_collection_id;
        let collection_id = collection.id.clone();
        h_flex()
            .id(SharedString::from(format!(
                "{}-collection-{}",
                self.id, collection.id
            )))
            .debug_selector({
                let collection_id = collection.id.clone();
                move || format!("variables-collection-{collection_id}")
            })
            .key_context(CONTROL_KEY_CONTEXT)
            .tab_index(0)
            .h(px(24.))
            .my(px(3.))
            .w_full()
            .px_2()
            .rounded(px(5.))
            .cursor_pointer()
            .border_1()
            .border_color(cx.theme().transparent)
            .text_size(px(12.))
            .when(selected, |row| {
                row.bg(cx.theme().secondary)
                    .text_color(cx.theme().secondary_foreground)
            })
            .hover(|style| style.bg(cx.theme().accent))
            .focus(|style| style.border_color(cx.theme().selection))
            .on_activate(cx.listener(move |_, _, _, cx| {
                cx.emit(VariablesAction::CollectionSelected {
                    collection_id: collection_id.clone(),
                });
            }))
            .child(
                div()
                    .id(SharedString::from(format!(
                        "{}-collection-name-{}",
                        self.id, collection.id
                    )))
                    .flex_1()
                    .truncate()
                    .when(selected, |name| name.font_semibold())
                    .child(collection.name.clone()),
            )
            .child(
                div()
                    .text_color(if selected {
                        cx.theme().secondary_foreground
                    } else {
                        cx.theme().muted_foreground
                    })
                    .child(collection.variable_count.to_string()),
            )
            .into_any_element()
    }

    fn render_group(&self, group: &VariablesGroup, cx: &mut Context<Self>) -> AnyElement {
        let selected = group.id == self.view_data.selected_group_id;
        let group_id = group.id.clone();
        h_flex()
            .id(SharedString::from(format!(
                "{}-group-{}",
                self.id, group.id
            )))
            .debug_selector({
                let group_id = group.id.clone();
                move || format!("variables-group-{group_id}")
            })
            .key_context(CONTROL_KEY_CONTEXT)
            .tab_index(0)
            .h(px(24.))
            .my(px(3.))
            .w_full()
            .px_2()
            .rounded(px(5.))
            .cursor_pointer()
            .border_1()
            .border_color(cx.theme().transparent)
            .text_size(px(12.))
            .when(selected, |row| {
                row.bg(cx.theme().selection.opacity(0.2))
                    .text_color(cx.theme().foreground)
            })
            .hover(|style| style.bg(cx.theme().accent))
            .focus(|style| style.border_color(cx.theme().selection))
            .on_activate(cx.listener(move |_, _, _, cx| {
                cx.emit(VariablesAction::GroupSelected {
                    group_id: group_id.clone(),
                });
            }))
            .child(
                div()
                    .flex_1()
                    .when(selected, |name| name.font_semibold())
                    .child(group.name.clone()),
            )
            .child(
                div()
                    .text_color(if selected {
                        cx.theme().foreground
                    } else {
                        cx.theme().muted_foreground
                    })
                    .child(group.variable_count.to_string()),
            )
            .into_any_element()
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut collections = v_flex().w_full();
        for collection in &self.view_data.collections {
            collections = collections.child(self.render_collection(collection, cx));
        }
        let mut groups = v_flex().w_full().mt(px(7.));
        for group in &self.view_data.groups {
            groups = groups.child(self.render_group(group, cx));
        }
        v_flex()
            .debug_selector(|| "variables-sidebar".to_owned())
            .w(px(self.sidebar_width()))
            .h_full()
            .flex_none()
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                v_flex()
                    .px(px(8.))
                    .py(px(10.))
                    .gap_1()
                    .child(
                        h_flex()
                            .h(px(28.))
                            .px_1()
                            .font_semibold()
                            .text_size(px(11.))
                            .child("Collections")
                            .child(div().flex_1())
                            .child(
                                icon_button(
                                    SharedString::from(format!("{}-create-collection", self.id)),
                                    px(24.),
                                    px(4.),
                                    cx,
                                )
                                .debug_selector(|| "variables-create-collection".to_owned())
                                .on_activate(cx.listener(|_, _, _, cx| {
                                    cx.emit(VariablesAction::CreateCollectionRequested);
                                }))
                                .child(Icon::new(IconName::Plus).xsmall()),
                            ),
                    )
                    .child(collections),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h(px(0.))
                    .px(px(8.))
                    .pt(px(8.))
                    .gap_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .px_1()
                            .font_semibold()
                            .text_size(px(11.))
                            .child("Groups"),
                    )
                    .child(groups),
            )
            .into_any_element()
    }

    fn render_kind_glyph(kind: VariableKind, cx: &mut Context<Self>) -> AnyElement {
        let icon = match kind {
            VariableKind::Color => ControlIcon::VariableColor,
            VariableKind::Number => ControlIcon::VariableNumber,
            VariableKind::String => ControlIcon::VariableText,
            VariableKind::Boolean => ControlIcon::VariableBoolean,
        };
        render_control_icon(icon, cx.theme().muted_foreground, 12.)
    }

    fn parse_hex(value: &str) -> Option<gpui::Hsla> {
        parse_hex_rgba(value).map(|value| rgba(value).into())
    }

    fn render_value(
        &self,
        variable: &VariableRow,
        mode: &VariablesMode,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let value = variable
            .values
            .iter()
            .find(|value| value.mode_id == mode.id);
        let variable_id = variable.id.clone();
        let mode_id = mode.id.clone();
        h_flex()
            .id(SharedString::from(format!(
                "{}-value-{}-{}",
                self.id, variable.id, mode.id
            )))
            .debug_selector({
                let variable_id = variable.id.clone();
                let mode_id = mode.id.clone();
                move || format!("variables-value-{variable_id}-{mode_id}")
            })
            .key_context(CONTROL_KEY_CONTEXT)
            .tab_index(0)
            .w(px(VALUE_COLUMN_WIDTH))
            .h(px(41.))
            .flex_none()
            .px(px(16.))
            .gap(px(4.))
            .border_r_1()
            .border_color(cx.theme().border)
            .cursor_pointer()
            .hover(|style| style.bg(cx.theme().accent.opacity(0.55)))
            .focus(|style| style.border_1().border_color(cx.theme().selection))
            .on_activate(cx.listener(move |_, _, _, cx| {
                cx.emit(VariablesAction::ValueEditRequested {
                    variable_id: variable_id.clone(),
                    mode_id: mode_id.clone(),
                });
            }))
            .when_some(
                value.and_then(|value| value.color_hex.as_ref()),
                |cell, hex| {
                    cell.child(
                        div()
                            .size(px(16.))
                            .flex_none()
                            .rounded(px(3.))
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(Self::parse_hex(hex).unwrap_or(cx.theme().background)),
                    )
                },
            )
            .child(
                div()
                    .truncate()
                    .text_size(px(12.))
                    .child(value.map_or_else(SharedString::default, |value| value.value.clone())),
            )
            .into_any_element()
    }

    fn render_table(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut header = h_flex()
            .h(px(41.))
            .w_full()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .debug_selector(|| "variables-name-header".to_owned())
                    .w(px(self.name_column_width()))
                    .h_full()
                    .flex_none()
                    .px(px(16.))
                    .border_r_1()
                    .border_color(cx.theme().border)
                    .font_semibold()
                    .text_size(px(11.))
                    .child("Name"),
            );
        // The mode headers clip inside a shrinkable region so the add-mode
        // cell stays pinned inside the page on narrow tables (the body's
        // value columns scroll horizontally below).
        let mut mode_headers = h_flex().h_full().flex_1().min_w(px(0.)).overflow_hidden();
        for mode in &self.view_data.modes {
            mode_headers = mode_headers.child(
                h_flex()
                    .w(px(VALUE_COLUMN_WIDTH))
                    .h_full()
                    .flex_none()
                    .px(px(16.))
                    .border_r_1()
                    .border_color(cx.theme().border)
                    .font_semibold()
                    .text_size(px(11.))
                    .child(mode.name.clone()),
            );
        }
        header = header.child(mode_headers).child(
            div()
                .id(SharedString::from(format!("{}-add-mode", self.id)))
                .debug_selector(|| "variables-add-mode".to_owned())
                .key_context(CONTROL_KEY_CONTEXT)
                .tab_index(0)
                .size(px(40.))
                .flex_none()
                .items_center()
                .justify_center()
                .border_l_1()
                .border_color(cx.theme().border)
                .cursor_pointer()
                .hover(|style| style.bg(cx.theme().accent))
                .focus(|style| style.border_1().border_color(cx.theme().selection))
                .on_activate(cx.listener(|_, _, _, cx| {
                    cx.emit(VariablesAction::AddModeRequested);
                }))
                .child(Icon::new(IconName::Plus).small()),
        );

        let query = self.search_input.read(cx).value().to_lowercase();
        let selected_group_is_aggregate = self
            .view_data
            .groups
            .iter()
            .find(|group| group.id == self.view_data.selected_group_id)
            .is_some_and(|group| group.is_aggregate);
        let mut rows = v_flex().w_full();
        for variable in self.view_data.variables.iter().filter(|variable| {
            (selected_group_is_aggregate || variable.group_id == self.view_data.selected_group_id)
                && (query.is_empty() || variable.name.to_lowercase().contains(&query))
        }) {
            let mut row = h_flex()
                .h(px(41.))
                .w_full()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(
                    h_flex()
                        .debug_selector({
                            let variable_id = variable.id.clone();
                            move || format!("variables-name-cell-{variable_id}")
                        })
                        .w(px(self.name_column_width()))
                        .h_full()
                        .flex_none()
                        .px(px(16.))
                        .gap_3()
                        .border_r_1()
                        .border_color(cx.theme().border)
                        .text_size(px(12.))
                        .child(
                            div()
                                .w(px(16.))
                                .text_color(cx.theme().muted_foreground)
                                .child(Self::render_kind_glyph(variable.kind, cx)),
                        )
                        .child(div().truncate().child(variable.name.clone())),
                );
            for mode in &self.view_data.modes {
                row = row.child(self.render_value(variable, mode, cx));
            }
            rows = rows.child(row);
        }

        v_flex()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .child(header)
            .child(
                div()
                    .id(SharedString::from(format!("{}-table-scroll", self.id)))
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_scroll()
                    .track_scroll(&self.table_scroll_handle)
                    .child(rows),
            )
            .child(
                h_flex()
                    .id(SharedString::from(format!("{}-create-variable", self.id)))
                    .debug_selector(|| "variables-create-variable".to_owned())
                    .key_context(CONTROL_KEY_CONTEXT)
                    .tab_index(0)
                    .h(px(41.))
                    .w_full()
                    .px(px(16.))
                    .gap_3()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .cursor_pointer()
                    .hover(|style| style.bg(cx.theme().accent))
                    .focus(|style| style.border_1().border_color(cx.theme().selection))
                    .on_activate(cx.listener(|_, _, _, cx| {
                        cx.emit(VariablesAction::CreateVariableRequested);
                    }))
                    .child(Icon::new(IconName::Plus).small())
                    .child(div().text_size(px(11.)).child("Create variable")),
            )
            .into_any_element()
    }
}

impl Focusable for VariablesPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for VariablesPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = cx.entity();
        v_flex()
            .id(self.id.clone())
            .track_focus(&self.focus_handle)
            .relative()
            .size_full()
            .min_h(px(0.))
            .pt(px(1.))
            .pb(px(1.))
            .pl(px(5.))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                // Measures the page so the header row and the body share the
                // computed sidebar and name-column widths. The write is
                // deferred past the draw so the changed width schedules a
                // re-render (notifying mid-draw is a no-op).
                canvas(
                    move |bounds, _, app| {
                        let width = f32::from(bounds.size.width);
                        let known = page.read(app).page_width;
                        if known.is_none_or(|known| (known - width).abs() > 0.5) {
                            let page = page.clone();
                            app.defer(move |app| {
                                page.update(app, |this, cx| {
                                    this.page_width = Some(width);
                                    cx.notify();
                                });
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .left_0()
                .top_0()
                .size_full(),
            )
            .child(
                h_flex()
                    .h(px(50.))
                    .w_full()
                    .flex_none()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .w(px(self.sidebar_width()))
                            .h_full()
                            .flex_none()
                            .px(px(20.))
                            .border_r_1()
                            .border_color(cx.theme().border)
                            .font_semibold()
                            .text_size(px(13.))
                            .child(self.view_data.document_name.clone())
                            .child(div().flex_1())
                            .child(Icon::new(IconName::PanelLeft).small()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .px(px(20.))
                            .font_semibold()
                            .text_size(px(13.))
                            .truncate()
                            .child(
                                self.view_data
                                    .collections
                                    .iter()
                                    .find(|collection| {
                                        collection.id == self.view_data.selected_collection_id
                                    })
                                    .map_or_else(SharedString::default, |collection| {
                                        collection.name.clone()
                                    }),
                            ),
                    )
                    .child(
                        h_flex()
                            .w(px(HEADER_TOOLS_WIDTH))
                            .pl(px(7.))
                            .pr(px(8.))
                            .gap(px(18.5))
                            .child(
                                h_flex()
                                    .h(px(24.))
                                    .flex_1()
                                    .min_w(px(0.))
                                    .overflow_hidden()
                                    .rounded(px(6.))
                                    .bg(cx.theme().secondary)
                                    .child(
                                        div().flex_1().min_w(px(0.)).child(
                                            Input::new(&self.search_input)
                                                .appearance(false)
                                                .bordered(false)
                                                .focus_bordered(false)
                                                .small()
                                                .px(px(6.))
                                                .text_size(px(12.))
                                                .prefix(Icon::new(IconName::Search).small()),
                                        ),
                                    )
                                    .child(
                                        div()
                                            .id(SharedString::from(format!(
                                                "{}-search-options",
                                                self.id
                                            )))
                                            .debug_selector(|| {
                                                "variables-search-options".to_owned()
                                            })
                                            .key_context(CONTROL_KEY_CONTEXT)
                                            .tab_index(0)
                                            .w(px(26.))
                                            .h_full()
                                            .items_center()
                                            .justify_center()
                                            .border_l_1()
                                            .border_color(cx.theme().border)
                                            .cursor_pointer()
                                            .hover(|style| style.bg(cx.theme().accent))
                                            .focus(|style| {
                                                style.border_1().border_color(cx.theme().selection)
                                            })
                                            .on_activate(cx.listener(|_, _, _, cx| {
                                                cx.emit(VariablesAction::SearchOptionsRequested);
                                            }))
                                            .child(Icon::new(IconName::Settings2).xsmall()),
                                    ),
                            )
                            .child(Icon::new(IconName::Maximize).small())
                            .child(
                                div()
                                    .id(SharedString::from(format!("{}-share", self.id)))
                                    .debug_selector(|| "variables-share".to_owned())
                                    .key_context(CONTROL_KEY_CONTEXT)
                                    .tab_index(0)
                                    .h(px(32.))
                                    .px(px(11.))
                                    .items_center()
                                    .rounded(px(6.))
                                    .border_1()
                                    .border_color(cx.theme().transparent)
                                    .bg(cx.theme().primary)
                                    .text_color(cx.theme().primary_foreground)
                                    .cursor_pointer()
                                    .font_semibold()
                                    .text_size(px(11.))
                                    .hover(|style| style.bg(cx.theme().primary_hover))
                                    .focus(|style| style.border_color(cx.theme().selection))
                                    .on_activate(cx.listener(|_, _, _, cx| {
                                        cx.emit(VariablesAction::ShareRequested);
                                    }))
                                    .child("Share"),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h(px(0.))
                    .items_start()
                    .child(self.render_sidebar(cx))
                    .child(self.render_table(cx)),
            )
            .child(
                icon_button(
                    SharedString::from(format!("{}-help", self.id)),
                    px(40.),
                    px(20.),
                    cx,
                )
                .debug_selector(|| "variables-help".to_owned())
                .absolute()
                .right(px(24.))
                .bottom(px(20.))
                .occlude()
                .border_color(cx.theme().border)
                .bg(cx.theme().background)
                .shadow_md()
                .focus(|style| style.border_color(cx.theme().selection))
                .on_activate(cx.listener(|_, _, _, cx| {
                    cx.emit(VariablesAction::HelpRequested);
                }))
                .child(render_control_icon(
                    ControlIcon::Help,
                    cx.theme().foreground,
                    16.,
                )),
            )
            .child(
                div()
                    .absolute()
                    .left(px(5.))
                    .top(px(1.))
                    .bottom(px(1.))
                    .w(px(1.))
                    .bg(cx.theme().border),
            )
            .child(
                div()
                    .absolute()
                    .left(px(5.))
                    .right_0()
                    .top(px(1.))
                    .h(px(1.))
                    .bg(cx.theme().border),
            )
            .child(
                div()
                    .absolute()
                    .left(px(5.))
                    .right_0()
                    .bottom(px(1.))
                    .h(px(1.))
                    .bg(cx.theme().border),
            )
            .child(
                div()
                    .absolute()
                    .right_0()
                    .top(px(1.))
                    .bottom(px(1.))
                    .w(px(1.))
                    .bg(cx.theme().border),
            )
    }
}

#[cfg(test)]
mod interaction_tests;
