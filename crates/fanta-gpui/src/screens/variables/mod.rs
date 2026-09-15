//! Host-controlled Variables screen with Figma UI3 table geometry.

use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, ParentElement as _, Render,
    ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
    canvas, div, prelude::FluentBuilder as _, px, rgba,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::Button,
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::Scrollbar,
    v_flex,
};

use crate::{
    atoms::{
        ActivateEvent, ButtonControlExt as _, CONTROL_KEY_CONTEXT, ControlExt as _, LucideIcon,
        icon_button, render_lucide_icon, tokens,
    },
    color::parse_hex_rgba,
    molecules::menu_item,
};

/// Reference design width of the collections/groups sidebar.
const SIDEBAR_WIDTH: f32 = 282.;
/// Floor the sidebar compresses to on narrow pages.
const SIDEBAR_MIN_WIDTH: f32 = 180.;
/// Fixed width of the search cluster on the right of the header row.
const HEADER_TOOLS_WIDTH: f32 = 320.;
/// Collection-title width preserved before the sidebar starts compressing.
const HEADER_TITLE_MIN_WIDTH: f32 = 120.;
/// Reference design width of the table's name and value columns.
const NAME_COLUMN_WIDTH: f32 = 201.;
const VALUE_COLUMN_WIDTH: f32 = 201.;
/// Floor the name column compresses to on narrow tables.
const NAME_COLUMN_MIN_WIDTH: f32 = 140.;
const VALUE_COLUMN_MIN_WIDTH: f32 = 160.;
/// Fixed width of the pinned actions column.
const ACTIONS_COLUMN_WIDTH: f32 = 40.;
const TABLE_SCROLLBAR_WIDTH: f32 = 16.;

/// The smallest width the variables manager reflows to honestly: the
/// floored sidebar, the fixed header search cluster, and a usable
/// collection title. Below this the table's mode columns already scroll;
/// narrower pages would clip header chrome instead of compressing it.
pub const VARIABLES_SCREEN_MIN_WIDTH: f32 = SIDEBAR_MIN_WIDTH + HEADER_TOOLS_WIDTH + 60.;
/// The smallest height the variables manager reflows to honestly: the
/// page header, the table header, one variable row, the create-variable
/// row, and a short scrollable sidebar/table region.
pub const VARIABLES_SCREEN_MIN_HEIGHT: f32 = 400.;

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

/// Immutable host snapshot rendered by [`VariablesScreen`].
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
    CollectionRenameRequested {
        collection_id: SharedString,
        name: SharedString,
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
    ImportVariablesRequested,
    AddModeRequested,
    ValueEditRequested {
        variable_id: SharedString,
        mode_id: SharedString,
    },
    VariableSettingsRequested {
        variable_id: SharedString,
    },
    HelpRequested,
}

/// Stateful presentation for the host-controlled variables manager.
pub struct VariablesScreen {
    id: SharedString,
    focus_handle: FocusHandle,
    view_data: VariablesViewData,
    search_input: Entity<InputState>,
    collection_name_input: Entity<InputState>,
    renaming_collection_id: Option<SharedString>,
    filter_menu_open: bool,
    visible_kinds: [bool; 4],
    table_scroll_handle: ScrollHandle,
    table_horizontal_scroll_handle: ScrollHandle,
    sidebar_visible: bool,
    /// Measured width of the whole page; the header row and the body derive
    /// shared sidebar and name-column widths from it so their vertical
    /// borders stay aligned while both compress on narrow pages.
    page_width: Option<f32>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<VariablesAction> for VariablesScreen {}

impl VariablesScreen {
    pub fn new(
        id: impl Into<SharedString>,
        view_data: VariablesViewData,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let collection_name_input = cx.new(|cx| InputState::new(window, cx));
        let subscriptions = vec![
            cx.subscribe(&search_input, |_, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.emit(VariablesAction::SearchQueryChanged {
                        query: input.read(cx).value(),
                    });
                }
            }),
            cx.subscribe(
                &collection_name_input,
                |this, input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                        let Some(collection_id) = this.renaming_collection_id.take() else {
                            return;
                        };
                        let name = input.read(cx).value();
                        if !name.trim().is_empty() {
                            cx.emit(VariablesAction::CollectionRenameRequested {
                                collection_id,
                                name,
                            });
                        }
                        cx.notify();
                    }
                },
            ),
        ];
        Self {
            id: id.into(),
            focus_handle: cx.focus_handle(),
            view_data,
            search_input,
            collection_name_input,
            renaming_collection_id: None,
            filter_menu_open: false,
            visible_kinds: [true; 4],
            table_scroll_handle: ScrollHandle::new(),
            table_horizontal_scroll_handle: ScrollHandle::new(),
            sidebar_visible: true,
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
        (self.table_viewport_width() * 0.25).clamp(NAME_COLUMN_MIN_WIDTH, NAME_COLUMN_WIDTH)
    }

    fn table_viewport_width(&self) -> f32 {
        self.page_width.map_or(
            SIDEBAR_WIDTH + NAME_COLUMN_WIDTH + VALUE_COLUMN_WIDTH + ACTIONS_COLUMN_WIDTH,
            |width| {
                width
                    - if self.sidebar_visible {
                        self.sidebar_width()
                    } else {
                        0.
                    }
            },
        )
    }

    fn mode_column_width(&self) -> f32 {
        let count = self.view_data.modes.len().max(1) as f32;
        ((self.table_viewport_width() - self.name_column_width() - ACTIONS_COLUMN_WIDTH) / count)
            .max(VALUE_COLUMN_MIN_WIDTH)
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_visible = !self.sidebar_visible;
        cx.notify();
    }

    const fn kind_index(kind: VariableKind) -> usize {
        match kind {
            VariableKind::Color => 0,
            VariableKind::Number => 1,
            VariableKind::String => 2,
            VariableKind::Boolean => 3,
        }
    }

    fn begin_collection_rename(
        &mut self,
        collection_id: SharedString,
        name: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.renaming_collection_id = Some(collection_id);
        self.collection_name_input
            .update(cx, |input, cx| input.set_value(name, window, cx));
        self.collection_name_input
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
        cx.notify();
    }

    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.visible_kinds = [true; 4];
        self.search_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    fn render_collection(
        &self,
        collection: &VariablesCollection,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = collection.id == self.view_data.selected_collection_id;
        let collection_id = collection.id.clone();
        let rename_collection_id = collection.id.clone();
        let rename_collection_name = collection.name.clone();
        let renaming = self.renaming_collection_id.as_ref() == Some(&collection.id);
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
            .text_size(px(tokens::TypeScale::BODY))
            .when(selected, |row| {
                row.bg(cx.theme().secondary)
                    .text_color(cx.theme().secondary_foreground)
            })
            .hover(|style| style.bg(cx.theme().accent))
            .focus(|style| style.border_color(cx.theme().selection))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    if event.click_count >= 2 {
                        this.begin_collection_rename(
                            rename_collection_id.clone(),
                            rename_collection_name.clone(),
                            window,
                            cx,
                        );
                    }
                }),
            )
            .on_activate(cx.listener(move |this, event: &ActivateEvent, _, cx| {
                if event.keyboard && this.renaming_collection_id.as_ref() == Some(&collection_id) {
                    this.renaming_collection_id = None;
                    let name = this.collection_name_input.read(cx).value();
                    if !name.trim().is_empty() {
                        cx.emit(VariablesAction::CollectionRenameRequested {
                            collection_id: collection_id.clone(),
                            name,
                        });
                    }
                    cx.notify();
                } else if this.renaming_collection_id.is_none() {
                    cx.emit(VariablesAction::CollectionSelected {
                        collection_id: collection_id.clone(),
                    });
                }
            }))
            .when(!renaming, |row| {
                row.child(
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
            })
            .when(renaming, |row| {
                row.child(
                    div().flex_1().min_w(px(0.)).child(
                        Input::new(&self.collection_name_input)
                            .appearance(false)
                            .bordered(false)
                            .focus_bordered(false)
                            .small(),
                    ),
                )
            })
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
            .text_size(px(tokens::TypeScale::BODY))
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
                            .text_size(px(tokens::TypeScale::CAPTION))
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
                            .text_size(px(tokens::TypeScale::CAPTION))
                            .child("Groups"),
                    )
                    .child(groups),
            )
            .into_any_element()
    }

    fn render_kind_glyph(kind: VariableKind, cx: &mut Context<Self>) -> AnyElement {
        let icon = match kind {
            VariableKind::Color => LucideIcon::Palette,
            VariableKind::Number => LucideIcon::Hash,
            VariableKind::String => LucideIcon::TypeIcon,
            VariableKind::Boolean => LucideIcon::ToggleLeft,
        };
        render_lucide_icon(icon, cx.theme().muted_foreground, 12.)
    }

    fn parse_hex(value: &str) -> Option<gpui::Hsla> {
        parse_hex_rgba(value).map(|value| rgba(value).into())
    }

    fn render_value(
        &self,
        variable: &VariableRow,
        mode: &VariablesMode,
        mode_width: f32,
        draw_right_border: bool,
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
            .w(px(mode_width))
            .h_full()
            .flex_none()
            .px(px(16.))
            .gap(px(4.))
            .border_b_1()
            .border_color(cx.theme().border)
            .when(draw_right_border, |cell| cell.border_r_1())
            .cursor_pointer()
            .hover(|style| {
                style
                    .bg(cx.theme().accent)
                    .text_color(cx.theme().accent_foreground)
            })
            .focus(|style| {
                style
                    .bg(cx.theme().accent)
                    .text_color(cx.theme().accent_foreground)
            })
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
                    .text_size(px(tokens::TypeScale::BODY))
                    .child(value.map_or_else(SharedString::default, |value| value.value.clone())),
            )
            .into_any_element()
    }

    fn render_filter_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut menu = v_flex()
            .debug_selector(|| "variables-filter-menu".to_owned())
            .absolute()
            .top(px(42.))
            .right(px(8.))
            .w(px(210.))
            .py_2()
            .rounded(px(10.))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .shadow_lg()
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _, cx| {
                this.filter_menu_open = false;
                cx.notify();
            }));
        let all_selected = self.visible_kinds.iter().all(|selected| *selected);
        menu = menu.child(
            menu_item(
                SharedString::from(format!("{}-filter-all", self.id)),
                px(30.),
                cx,
            )
            .debug_selector(|| "variables-filter-all".to_owned())
            .on_activate(cx.listener(|this, _, _, cx| {
                this.visible_kinds = [true; 4];
                cx.notify();
            }))
            .child(div().w(px(16.)).when(all_selected, |slot| {
                slot.child(Icon::new(IconName::Check).xsmall())
            }))
            .child("All"),
        );
        for (kind, label) in [
            (VariableKind::Color, "Colors"),
            (VariableKind::Number, "Numbers"),
            (VariableKind::String, "Strings"),
            (VariableKind::Boolean, "Booleans"),
        ] {
            let index = Self::kind_index(kind);
            let selected = self.visible_kinds[index];
            menu = menu.child(
                menu_item(
                    SharedString::from(format!("{}-filter-kind-{index}", self.id)),
                    px(30.),
                    cx,
                )
                .debug_selector(move || format!("variables-filter-kind-{index}"))
                .on_activate(cx.listener(move |this, _, _, cx| {
                    this.visible_kinds[index] = !this.visible_kinds[index];
                    cx.notify();
                }))
                .child(div().w(px(16.)).when(selected && !all_selected, |slot| {
                    slot.child(Icon::new(IconName::Check).xsmall())
                }))
                .child(Self::render_kind_glyph(kind, cx))
                .child(label),
            );
        }
        menu.into_any_element()
    }

    fn render_empty_state(&self, search_empty: bool, cx: &mut Context<Self>) -> AnyElement {
        let page = cx.entity();
        let mut state = v_flex()
            .debug_selector(|| "variables-empty-state".to_owned())
            .absolute()
            .top(px(41.))
            .right_0()
            .bottom(px(41.))
            .left_0()
            .p(px(16.))
            .items_center()
            .justify_center()
            .gap_4()
            .bg(cx.theme().background)
            .occlude()
            .child(
                div()
                    .debug_selector(|| "variables-empty-title".to_owned())
                    .max_w_full()
                    .text_center()
                    .text_size(px(tokens::TypeScale::TITLE))
                    .font_semibold()
                    .child(if search_empty {
                        "No variables match search"
                    } else {
                        "No variables in this collection"
                    }),
            )
            .child(
                div()
                    .debug_selector(|| "variables-empty-description".to_owned())
                    .w_full()
                    .max_w(px(330.))
                    .text_center()
                    .text_size(px(tokens::TypeScale::BODY))
                    .line_height(px(16.))
                    .text_color(cx.theme().muted_foreground)
                    .child(if search_empty {
                        "Variables that don’t match the current search and filters are hidden."
                    } else {
                        "Create or import variables to reuse values across your file."
                    }),
            );
        if search_empty {
            let page = page.clone();
            state = state.child(
                Button::new(SharedString::from(format!(
                    "{}-clear-empty-search",
                    self.id
                )))
                .debug_selector(|| "variables-clear-empty-search".to_owned())
                .mt(px(8.))
                .label("Clear search")
                .icon(IconName::CircleX)
                .small()
                .compact()
                .outline()
                .on_activate(move |_, window, cx| {
                    page.update(cx, |this, cx| this.clear_search(window, cx));
                }),
            );
        } else {
            let page_for_create = page.clone();
            let page_for_import = page;
            state = state.child(
                h_flex()
                    .max_w_full()
                    .gap_2()
                    .flex_wrap()
                    .justify_center()
                    .child(
                        Button::new(SharedString::from(format!("{}-empty-create", self.id)))
                            .debug_selector(|| "variables-empty-create".to_owned())
                            .label("Create")
                            .icon(IconName::Plus)
                            .small()
                            .compact()
                            .on_activate(move |_, _, cx| {
                                page_for_create.update(cx, |_, cx| {
                                    cx.emit(VariablesAction::CreateVariableRequested);
                                });
                            }),
                    )
                    .child(
                        Button::new(SharedString::from(format!("{}-empty-import", self.id)))
                            .debug_selector(|| "variables-empty-import".to_owned())
                            .label("Import")
                            .small()
                            .compact()
                            .outline()
                            .on_activate(move |_, _, cx| {
                                page_for_import.update(cx, |_, cx| {
                                    cx.emit(VariablesAction::ImportVariablesRequested);
                                });
                            }),
                    ),
            );
        }
        state.into_any_element()
    }

    fn render_table(&self, cx: &mut Context<Self>) -> AnyElement {
        let name_width = self.name_column_width();
        let mode_width = self.mode_column_width();
        let modes_width = mode_width * self.view_data.modes.len() as f32;
        let name_header = h_flex()
            .debug_selector(|| "variables-name-header".to_owned())
            .w_full()
            .h(px(41.))
            .flex_none()
            .px(px(16.))
            .border_r_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .font_semibold()
            .text_size(px(tokens::TypeScale::CAPTION))
            .child("Name");
        let mut names = v_flex().w(px(name_width)).flex_none();
        let mut modes = v_flex().w(px(modes_width)).flex_none();
        let mut mode_headers = h_flex().w(px(modes_width)).h(px(41.)).flex_none();
        for (index, mode) in self.view_data.modes.iter().enumerate() {
            mode_headers = mode_headers.child(
                h_flex()
                    .debug_selector({
                        let mode_id = mode.id.clone();
                        move || format!("variables-mode-header-{mode_id}")
                    })
                    .w(px(mode_width))
                    .h_full()
                    .flex_none()
                    .px(px(16.))
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .when(index + 1 < self.view_data.modes.len(), |header| {
                        header.border_r_1()
                    })
                    .font_semibold()
                    .text_size(px(tokens::TypeScale::CAPTION))
                    .child(mode.name.clone()),
            );
        }
        let action_header = h_flex()
            .id(SharedString::from(format!("{}-add-mode", self.id)))
            .debug_selector(|| "variables-add-mode".to_owned())
            .key_context(CONTROL_KEY_CONTEXT)
            .tab_index(0)
            .w_full()
            .h(px(41.))
            .flex_none()
            .items_center()
            .justify_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .cursor_pointer()
            .hover(|style| style.bg(cx.theme().accent))
            .focus(|style| style.border_1().border_color(cx.theme().selection))
            .on_activate(cx.listener(|_, _, _, cx| {
                cx.emit(VariablesAction::AddModeRequested);
            }))
            .child(Icon::new(IconName::Plus).small());
        let mut actions = v_flex().w(px(ACTIONS_COLUMN_WIDTH)).flex_none();

        let query = self.search_input.read(cx).value().to_lowercase();
        let selected_group_is_aggregate = self
            .view_data
            .groups
            .iter()
            .find(|group| group.id == self.view_data.selected_group_id)
            .is_some_and(|group| group.is_aggregate);
        let group_variables = self
            .view_data
            .variables
            .iter()
            .filter(|variable| {
                selected_group_is_aggregate || variable.group_id == self.view_data.selected_group_id
            })
            .collect::<Vec<_>>();
        let matching_variables = group_variables
            .iter()
            .copied()
            .filter(|variable| {
                self.visible_kinds[Self::kind_index(variable.kind)]
                    && (query.is_empty() || variable.name.to_lowercase().contains(&query))
            })
            .collect::<Vec<_>>();
        for variable in &matching_variables {
            names = names.child(
                h_flex()
                    .debug_selector({
                        let variable_id = variable.id.clone();
                        move || format!("variables-name-cell-{variable_id}")
                    })
                    .w_full()
                    .h(px(41.))
                    .flex_none()
                    .px(px(16.))
                    .gap_3()
                    .border_r_1()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .text_size(px(tokens::TypeScale::BODY))
                    .child(
                        div()
                            .w(px(16.))
                            .text_color(cx.theme().muted_foreground)
                            .child(Self::render_kind_glyph(variable.kind, cx)),
                    )
                    .child(div().truncate().child(variable.name.clone())),
            );

            let mut mode_cells = h_flex().w(px(modes_width)).h(px(41.)).flex_none();
            for (index, mode) in self.view_data.modes.iter().enumerate() {
                mode_cells = mode_cells.child(self.render_value(
                    variable,
                    mode,
                    mode_width,
                    index + 1 < self.view_data.modes.len(),
                    cx,
                ));
            }
            modes = modes.child(mode_cells);

            let variable_id = variable.id.clone();
            actions = actions.child(
                h_flex()
                    .id(SharedString::from(format!(
                        "{}-variable-settings-{}",
                        self.id, variable.id
                    )))
                    .debug_selector({
                        let variable_id = variable.id.clone();
                        move || format!("variables-variable-settings-{variable_id}")
                    })
                    .key_context(CONTROL_KEY_CONTEXT)
                    .tab_index(0)
                    .w_full()
                    .h(px(41.))
                    .flex_none()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|style| style.bg(cx.theme().accent))
                    .focus(|style| style.border_1().border_color(cx.theme().selection))
                    .on_activate(cx.listener(move |_, _, _, cx| {
                        cx.emit(VariablesAction::VariableSettingsRequested {
                            variable_id: variable_id.clone(),
                        });
                    }))
                    .child(Icon::new(IconName::Settings2).xsmall()),
            );
        }

        v_flex()
            .relative()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .child(
                h_flex()
                    .flex_1()
                    .min_h(px(0.))
                    .items_start()
                    .child(
                        v_flex()
                            .w(px(name_width))
                            .h_full()
                            .flex_none()
                            .child(name_header)
                            .child(
                                div()
                                    .id(SharedString::from(format!(
                                        "{}-table-names-scroll",
                                        self.id
                                    )))
                                    .flex_1()
                                    .min_h(px(0.))
                                    .overflow_y_scroll()
                                    .track_scroll(&self.table_scroll_handle)
                                    .child(names),
                            ),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "{}-table-horizontal-scroll",
                                self.id
                            )))
                            .flex_1()
                            .min_w(px(0.))
                            .h_full()
                            .overflow_x_scroll()
                            .track_scroll(&self.table_horizontal_scroll_handle)
                            .child(
                                v_flex()
                                    .w(px(modes_width))
                                    .h_full()
                                    .flex_none()
                                    .child(mode_headers)
                                    .child(
                                        div()
                                            .id(SharedString::from(format!(
                                                "{}-table-scroll",
                                                self.id
                                            )))
                                            .flex_1()
                                            .min_h(px(0.))
                                            .overflow_y_scroll()
                                            .track_scroll(&self.table_scroll_handle)
                                            .child(modes),
                                    ),
                            ),
                    )
                    .child(
                        v_flex()
                            .w(px(ACTIONS_COLUMN_WIDTH))
                            .h_full()
                            .flex_none()
                            .border_l_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().background)
                            .occlude()
                            .child(action_header)
                            .child(
                                div()
                                    .id(SharedString::from(format!(
                                        "{}-table-actions-scroll",
                                        self.id
                                    )))
                                    .flex_1()
                                    .min_h(px(0.))
                                    .overflow_y_scroll()
                                    .track_scroll(&self.table_scroll_handle)
                                    .child(actions),
                            ),
                    ),
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
                    .child(
                        div()
                            .text_size(px(tokens::TypeScale::CAPTION))
                            .child("Create variable"),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .left(px(name_width))
                    .right(px(ACTIONS_COLUMN_WIDTH))
                    .bottom(px(41.))
                    .h(px(TABLE_SCROLLBAR_WIDTH))
                    .child(Scrollbar::horizontal(&self.table_horizontal_scroll_handle)),
            )
            .when(group_variables.is_empty(), |table| {
                table.child(self.render_empty_state(false, cx))
            })
            .when(
                !group_variables.is_empty() && matching_variables.is_empty(),
                |table| table.child(self.render_empty_state(true, cx)),
            )
            .into_any_element()
    }
}

impl Focusable for VariablesScreen {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for VariablesScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = cx.entity();
        v_flex()
            .id(self.id.clone())
            .track_focus(&self.focus_handle)
            .relative()
            .size_full()
            .min_h(px(0.))
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
                    .when(self.sidebar_visible, |header| {
                        header.child(
                            h_flex()
                                .w(px(self.sidebar_width()))
                                .h_full()
                                .flex_none()
                                .px(px(20.))
                                .border_r_1()
                                .border_color(cx.theme().border)
                                .font_semibold()
                                .text_size(px(tokens::TypeScale::LABEL))
                                .child(self.view_data.document_name.clone())
                                .child(div().flex_1())
                                .child(
                                    icon_button(
                                        SharedString::from(format!("{}-toggle-sidebar", self.id)),
                                        px(24.),
                                        px(4.),
                                        cx,
                                    )
                                    .debug_selector(|| "variables-toggle-sidebar".to_owned())
                                    .on_activate(
                                        cx.listener(|this, _, _, cx| this.toggle_sidebar(cx)),
                                    )
                                    .child(Icon::new(IconName::PanelLeft).small()),
                                ),
                        )
                    })
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w(px(0.))
                            .px(px(20.))
                            .font_semibold()
                            .text_size(px(tokens::TypeScale::LABEL))
                            .gap_2()
                            .when(!self.sidebar_visible, |title| {
                                title.child(
                                    icon_button(
                                        SharedString::from(format!(
                                            "{}-toggle-sidebar-collapsed",
                                            self.id
                                        )),
                                        px(24.),
                                        px(4.),
                                        cx,
                                    )
                                    .debug_selector(|| {
                                        "variables-toggle-sidebar-collapsed".to_owned()
                                    })
                                    .on_activate(
                                        cx.listener(|this, _, _, cx| this.toggle_sidebar(cx)),
                                    )
                                    .child(Icon::new(IconName::PanelLeft).small()),
                                )
                            })
                            .child(
                                div().min_w(px(0.)).truncate().child(
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
                            ),
                    )
                    .child(
                        h_flex()
                            .relative()
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
                                                .cleanable(true)
                                                .small()
                                                .px(px(6.))
                                                .text_size(px(tokens::TypeScale::BODY))
                                                .prefix(Icon::new(IconName::Search).small()),
                                        ),
                                    )
                                    .child(
                                        h_flex()
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
                                            .on_activate(cx.listener(|this, _, _, cx| {
                                                this.filter_menu_open = !this.filter_menu_open;
                                                cx.emit(VariablesAction::SearchOptionsRequested);
                                                cx.notify();
                                            }))
                                            .child(Icon::new(IconName::Settings2).xsmall()),
                                    ),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h(px(0.))
                    .items_start()
                    .when(self.sidebar_visible, |body| {
                        body.child(self.render_sidebar(cx))
                    })
                    .child(self.render_table(cx)),
            )
            .when(self.filter_menu_open, |page| {
                page.child(self.render_filter_menu(cx))
            })
    }
}

#[cfg(test)]
mod interaction_tests;
