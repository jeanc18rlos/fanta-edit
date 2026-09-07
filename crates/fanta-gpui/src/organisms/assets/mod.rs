//! Figma-like Assets browser with host-controlled library data.

use gpui::StyledImage as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ObjectFit, ParentElement as _, Render, ScrollHandle,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, img,
    prelude::FluentBuilder as _, px, rgba,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};

use crate::atoms::{
    CONTROL_KEY_CONTEXT, ControlExt as _, ControlIcon, icon_button, render_control_icon,
};

/// Width of the fixed left navigation rail: intentional chrome
/// (ARCHITECTURE.md §5), so it never compresses.
pub const ASSETS_RAIL_WIDTH: f32 = 56.;
/// Narrowest width the panel lays out without clipping: the fixed rail
/// beside a ~200px content column whose rows truncate their labels.
pub const ASSETS_PANEL_MIN_WIDTH: f32 = 260.;
/// Shortest height the panel stays useful at; the library list scrolls.
pub const ASSETS_PANEL_MIN_HEIGHT: f32 = 400.;

/// One destination in the editor's left navigation rail.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum AssetsRailItem {
    File,
    Agents,
    #[default]
    Assets,
    Tools,
    Variables,
}

impl AssetsRailItem {
    const fn label(self) -> &'static str {
        match self {
            Self::File => "File",
            Self::Agents => "Agents",
            Self::Assets => "Assets",
            Self::Tools => "Tools",
            Self::Variables => "Variables",
        }
    }

    const fn slug(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Agents => "agents",
            Self::Assets => "assets",
            Self::Tools => "tools",
            Self::Variables => "variables",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AssetsLibraryKind {
    #[default]
    CurrentFile,
    UiKit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetsLibrary {
    pub id: SharedString,
    pub name: SharedString,
    pub component_count: usize,
    pub kind: AssetsLibraryKind,
    pub accent: u32,
    pub thumbnail_asset: Option<SharedString>,
}

impl AssetsLibrary {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        component_count: usize,
        kind: AssetsLibraryKind,
        accent: u32,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            component_count,
            kind,
            accent,
            thumbnail_asset: None,
        }
    }

    /// Supplies a host-owned embedded image asset for the 71×40 preview.
    pub fn thumbnail_asset(mut self, path: impl Into<SharedString>) -> Self {
        self.thumbnail_asset = Some(path.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetsViewData {
    pub libraries: Vec<AssetsLibrary>,
    pub active_rail_item: AssetsRailItem,
    pub icon_assets: AssetsIconAssets,
}

impl AssetsViewData {
    pub fn new(libraries: impl IntoIterator<Item = AssetsLibrary>) -> Self {
        Self {
            libraries: libraries.into_iter().collect(),
            active_rail_item: AssetsRailItem::default(),
            icon_assets: AssetsIconAssets::default(),
        }
    }

    pub fn active_rail_item(mut self, active_rail_item: AssetsRailItem) -> Self {
        self.active_rail_item = active_rail_item;
        self
    }

    pub fn icon_assets(mut self, icon_assets: AssetsIconAssets) -> Self {
        self.icon_assets = icon_assets;
        self
    }
}

impl Default for AssetsViewData {
    fn default() -> Self {
        Self::new([])
    }
}

/// Optional host-owned artwork for glyphs shown by the Assets panel.
///
/// Consumers can leave every field empty to use the component-library
/// fallbacks. Supplying artwork is useful when a host needs to mirror its
/// editor's exact icon set without coupling this crate to that asset bundle.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AssetsIconAssets {
    pub rail_file: Option<SharedString>,
    pub rail_agents: Option<SharedString>,
    pub rail_assets: Option<SharedString>,
    pub rail_tools: Option<SharedString>,
    pub rail_variables: Option<SharedString>,
    pub header_library: Option<SharedString>,
    pub filters: Option<SharedString>,
    pub ui_kit_badge: Option<SharedString>,
}

impl AssetsIconAssets {
    fn rail(&self, item: AssetsRailItem) -> Option<SharedString> {
        match item {
            AssetsRailItem::File => self.rail_file.clone(),
            AssetsRailItem::Agents => self.rail_agents.clone(),
            AssetsRailItem::Assets => self.rail_assets.clone(),
            AssetsRailItem::Tools => self.rail_tools.clone(),
            AssetsRailItem::Variables => self.rail_variables.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssetsPanelAction {
    SearchQueryChanged { query: SharedString },
    LibrarySelected { library_id: SharedString },
    FiltersRequested,
    AddLibrariesRequested,
    RailItemSelected { item: AssetsRailItem },
}

pub struct AssetsPanel {
    id: SharedString,
    focus_handle: FocusHandle,
    scroll_handle: ScrollHandle,
    view_data: AssetsViewData,
    search_input: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<AssetsPanelAction> for AssetsPanel {}

impl AssetsPanel {
    pub fn new(
        id: impl Into<SharedString>,
        view_data: AssetsViewData,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search all libraries"));
        let subscriptions =
            vec![
                cx.subscribe(&search_input, |_, input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.emit(AssetsPanelAction::SearchQueryChanged {
                            query: input.read(cx).value(),
                        });
                    }
                }),
            ];
        Self {
            id: id.into(),
            focus_handle: cx.focus_handle(),
            scroll_handle: ScrollHandle::new(),
            view_data,
            search_input,
            _subscriptions: subscriptions,
        }
    }

    pub fn set_view_data(&mut self, view_data: AssetsViewData, cx: &mut Context<Self>) {
        self.view_data = view_data;
        cx.notify();
    }

    fn rail_icon(&self, item: AssetsRailItem, cx: &mut Context<Self>) -> AnyElement {
        if let Some(path) = self.view_data.icon_assets.rail(item) {
            return img(path)
                .w(px(32.))
                .h(px(32.))
                .object_fit(ObjectFit::Fill)
                .into_any_element();
        }

        match item {
            AssetsRailItem::File => Icon::new(IconName::File).small().into_any_element(),
            AssetsRailItem::Agents => render_control_icon(
                ControlIcon::Sparkle,
                if self.view_data.active_rail_item == item {
                    cx.theme().sidebar_accent_foreground
                } else {
                    cx.theme().sidebar_foreground
                },
                16.,
            ),
            AssetsRailItem::Assets => div()
                .size(px(16.))
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .border_1()
                .border_color(cx.theme().sidebar_primary)
                .child(
                    Icon::new(IconName::Plus)
                        .xsmall()
                        .text_color(cx.theme().sidebar_primary),
                )
                .into_any_element(),
            AssetsRailItem::Tools => div()
                .relative()
                .w(px(16.))
                .h(px(14.))
                .child(
                    div()
                        .absolute()
                        .left(px(5.))
                        .top_0()
                        .w(px(6.))
                        .h(px(5.))
                        .rounded_t(px(2.))
                        .border_1()
                        .border_color(cx.theme().sidebar_foreground),
                )
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px(4.))
                        .w(px(16.))
                        .h(px(10.))
                        .rounded(px(2.))
                        .border_1()
                        .border_color(cx.theme().sidebar_foreground),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(4.))
                        .top(px(7.))
                        .w(px(8.))
                        .h(px(1.))
                        .bg(cx.theme().sidebar_foreground),
                )
                .into_any_element(),
            AssetsRailItem::Variables => Icon::new(IconName::Settings).small().into_any_element(),
        }
    }

    fn figma_mark(cx: &mut Context<Self>) -> AnyElement {
        div()
            .relative()
            .left(px(-0.5))
            .top(px(10.))
            .w(px(11.))
            .h(px(16.))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .size(px(6.))
                    .rounded(px(3.))
                    .border_1()
                    .border_color(cx.theme().sidebar_foreground),
            )
            .child(
                div()
                    .absolute()
                    .left(px(5.))
                    .top_0()
                    .size(px(6.))
                    .rounded(px(3.))
                    .border_1()
                    .border_color(cx.theme().sidebar_foreground),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(5.))
                    .size(px(6.))
                    .rounded(px(3.))
                    .border_1()
                    .border_color(cx.theme().sidebar_foreground),
            )
            .child(
                div()
                    .absolute()
                    .left(px(5.))
                    .top(px(5.))
                    .size(px(6.))
                    .rounded(px(3.))
                    .border_1()
                    .border_color(cx.theme().sidebar_foreground),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(10.))
                    .size(px(6.))
                    .rounded(px(3.))
                    .border_1()
                    .border_color(cx.theme().sidebar_foreground),
            )
            .into_any_element()
    }

    fn rail_item(&self, item: AssetsRailItem, cx: &mut Context<Self>) -> AnyElement {
        let active = self.view_data.active_rail_item == item;
        let uses_host_artwork = self.view_data.icon_assets.rail(item).is_some();
        v_flex()
            .id(SharedString::from(format!(
                "{}-rail-{}",
                self.id,
                item.slug()
            )))
            .debug_selector(move || format!("assets-rail-{}", item.slug()))
            .key_context(CONTROL_KEY_CONTEXT)
            .tab_index(0)
            .w(px(55.))
            .h(px(54.))
            .items_center()
            .justify_center()
            .gap(px(2.))
            .cursor_pointer()
            .border_1()
            .border_color(cx.theme().transparent)
            .focus(|style| style.border_color(cx.theme().selection))
            .on_activate(cx.listener(move |_, _, _, cx| {
                cx.emit(AssetsPanelAction::RailItemSelected { item });
            }))
            .child(
                div()
                    .flex()
                    .relative()
                    .left(px(-0.5))
                    .top(px(-1.))
                    .size(px(32.))
                    .items_center()
                    .justify_center()
                    .rounded(px(7.))
                    .when(active && !uses_host_artwork, |icon| {
                        icon.bg(cx.theme().sidebar_accent)
                            .text_color(cx.theme().sidebar_accent_foreground)
                    })
                    .hover(|style| style.bg(cx.theme().sidebar_accent))
                    .child(self.rail_icon(item, cx)),
            )
            .child(div().text_size(px(10.)).child(item.label()))
            .into_any_element()
    }

    fn render_rail(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .w(px(ASSETS_RAIL_WIDTH))
            .h_full()
            .flex_none()
            .items_center()
            .pt(px(6.))
            .gap(px(2.))
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .h(px(32.))
                    .items_center()
                    .justify_center()
                    .child(Self::figma_mark(cx)),
            )
            .child(div().w(px(16.)).h(px(1.)).my(px(8.)).bg(cx.theme().border))
            .child(self.rail_item(AssetsRailItem::File, cx))
            .child(self.rail_item(AssetsRailItem::Agents, cx))
            .child(self.rail_item(AssetsRailItem::Assets, cx))
            .child(self.rail_item(AssetsRailItem::Tools, cx))
            .child(
                div()
                    .w(px(16.))
                    .h(px(1.))
                    .mt(px(5.))
                    .mb(px(7.))
                    .bg(cx.theme().border),
            )
            .child(self.rail_item(AssetsRailItem::Variables, cx))
            .into_any_element()
    }

    fn render_thumbnail(&self, library: &AssetsLibrary) -> AnyElement {
        if let Some(path) = library.thumbnail_asset.clone() {
            return img(path)
                .w(px(71.))
                .h(px(40.))
                .flex_none()
                .rounded(px(5.))
                .object_fit(ObjectFit::Cover)
                .into_any_element();
        }
        let accent = rgba((library.accent << 8) | 0xff);
        div()
            .relative()
            .w(px(71.))
            .h(px(40.))
            .flex_none()
            .overflow_hidden()
            .rounded(px(5.))
            .border_1()
            .border_color(rgba(0xffffff22))
            .bg(accent)
            .child(
                div()
                    .absolute()
                    .left(px(-12.))
                    .top(px(15.))
                    .w(px(92.))
                    .h(px(18.))
                    .rounded(px(9.))
                    .bg(rgba(0xffffff33)),
            )
            .child(
                div()
                    .absolute()
                    .left(px(5.))
                    .top(px(4.))
                    .max_w(px(60.))
                    .text_size(px(7.))
                    .font_semibold()
                    .text_color(rgba(0xffffffff))
                    .child(library.name.clone()),
            )
            .into_any_element()
    }

    fn render_library(&self, library: &AssetsLibrary, cx: &mut Context<Self>) -> AnyElement {
        let library_id = library.id.clone();
        let selector_id = library.id.clone();
        let is_ui_kit = library.kind == AssetsLibraryKind::UiKit;
        let title_offset = if is_ui_kit { 3. } else { 1.5 };
        let count_offset = if is_ui_kit { -2. } else { -2.5 };
        h_flex()
            .id(SharedString::from(format!(
                "{}-library-{}",
                self.id, library.id
            )))
            .debug_selector(move || format!("assets-library-{selector_id}"))
            .key_context(CONTROL_KEY_CONTEXT)
            .tab_index(0)
            .w_full()
            .h(px(58.))
            .flex_none()
            .pl(px(1.))
            .gap(px(8.))
            .rounded(px(5.))
            .cursor_pointer()
            .border_1()
            .border_color(cx.theme().transparent)
            .hover(|style| style.bg(cx.theme().accent.opacity(0.7)))
            .focus(|style| style.border_color(cx.theme().selection))
            .on_activate(cx.listener(move |_, _, _, cx| {
                cx.emit(AssetsPanelAction::LibrarySelected {
                    library_id: library_id.clone(),
                });
            }))
            .child(self.render_thumbnail(library))
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.))
                    .gap(px(2.))
                    .child(
                        h_flex()
                            .relative()
                            .top(px(title_offset))
                            .gap(px(5.))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(11.))
                                    .child(library.name.clone()),
                            )
                            .when(is_ui_kit, |row| {
                                row.child(
                                    if let Some(path) =
                                        self.view_data.icon_assets.ui_kit_badge.clone()
                                    {
                                        div()
                                            .relative()
                                            .top(px(-1.))
                                            .w(px(12.))
                                            .h(px(12.))
                                            .flex_none()
                                            .child(
                                                img(path)
                                                    .w(px(12.))
                                                    .h(px(12.))
                                                    .object_fit(ObjectFit::Fill),
                                            )
                                            .into_any_element()
                                    } else {
                                        Icon::new(IconName::BookOpen)
                                            .xsmall()
                                            .text_color(cx.theme().muted_foreground)
                                            .into_any_element()
                                    },
                                )
                            }),
                    )
                    .child(
                        div()
                            .relative()
                            .top(px(count_offset))
                            .text_size(px(11.))
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{} components", library.component_count)),
                    ),
            )
            .into_any_element()
    }

    fn render_content(&self, cx: &mut Context<Self>) -> AnyElement {
        let query = self.search_input.read(cx).value().to_lowercase();
        let current_file = self
            .view_data
            .libraries
            .iter()
            .filter(|library| library.kind == AssetsLibraryKind::CurrentFile)
            .filter(|library| query.is_empty() || library.name.to_lowercase().contains(&query));
        let ui_kits = self
            .view_data
            .libraries
            .iter()
            .filter(|library| library.kind == AssetsLibraryKind::UiKit)
            .filter(|library| query.is_empty() || library.name.to_lowercase().contains(&query));

        let mut current = v_flex().gap(px(2.));
        for library in current_file {
            current = current.child(self.render_library(library, cx));
        }
        let mut kits = v_flex().gap(px(0.));
        for library in ui_kits {
            kits = kits.child(self.render_library(library, cx));
        }

        v_flex()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .child(
                h_flex()
                    .h(px(49.))
                    .px(px(16.))
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().font_semibold().text_size(px(14.)).child("Assets"))
                    .child(div().flex_1())
                    .child(
                        if let Some(path) = self.view_data.icon_assets.header_library.clone() {
                            img(path)
                                .w(px(16.))
                                .h(px(16.))
                                .object_fit(ObjectFit::Fill)
                                .into_any_element()
                        } else {
                            Icon::new(IconName::BookOpen).small().into_any_element()
                        },
                    ),
            )
            .child(
                v_flex()
                    .id(SharedString::from(format!("{}-scroll", self.id)))
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll_handle)
                    .px(px(16.))
                    .pt(px(11.))
                    .pb(px(44.))
                    .gap(px(10.))
                    .child(
                        h_flex()
                            .w_full()
                            .h(px(26.))
                            .flex_none()
                            .mx(px(-1.))
                            .mb(px(1.))
                            .gap(px(6.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .h_full()
                                    .rounded(px(6.))
                                    .border_1()
                                    .border_color(cx.theme().selection)
                                    .bg(cx.theme().secondary)
                                    .child(
                                        Input::new(&self.search_input)
                                            .appearance(false)
                                            .bordered(false)
                                            .focus_bordered(false)
                                            .small()
                                            .px(px(5.))
                                            .text_size(px(11.))
                                            .prefix(Icon::new(IconName::Search).small()),
                                    ),
                            )
                            .child(
                                icon_button(
                                    SharedString::from(format!("{}-filters", self.id)),
                                    px(20.),
                                    px(5.),
                                    cx,
                                )
                                .debug_selector(|| "assets-filters".to_owned())
                                .on_activate(cx.listener(|_, _, _, cx| {
                                    cx.emit(AssetsPanelAction::FiltersRequested);
                                }))
                                .child(
                                    if let Some(path) = self.view_data.icon_assets.filters.clone() {
                                        img(path)
                                            .w(px(18.))
                                            .h(px(18.))
                                            .object_fit(ObjectFit::Fill)
                                            .into_any_element()
                                    } else {
                                        Icon::new(IconName::Settings2).small().into_any_element()
                                    },
                                ),
                            ),
                    )
                    .child(
                        div()
                            .relative()
                            .top(px(-1.))
                            .mb(px(1.5))
                            .font_semibold()
                            .text_size(px(11.))
                            .child("All libraries"),
                    )
                    .child(current.mt(px(-5.)))
                    .child(div().h(px(1.)).w_full().mt(px(-2.)).bg(cx.theme().border))
                    .child(
                        div()
                            .mt(px(3.))
                            .relative()
                            .top(px(1.5))
                            .mb(px(2.))
                            .font_semibold()
                            .text_size(px(11.))
                            .child("UI kits"),
                    )
                    .child(kits.mt(px(-3.)))
                    .child(
                        h_flex()
                            .id(SharedString::from(format!("{}-add-libraries", self.id)))
                            .debug_selector(|| "assets-add-libraries".to_owned())
                            .key_context(CONTROL_KEY_CONTEXT)
                            .tab_index(0)
                            .h(px(24.))
                            .flex_none()
                            .mt(px(2.))
                            .w_full()
                            .justify_center()
                            .rounded(px(5.))
                            .border_1()
                            .border_color(cx.theme().border)
                            .cursor_pointer()
                            .text_size(px(11.))
                            .hover(|style| style.bg(cx.theme().accent))
                            .focus(|style| style.border_color(cx.theme().selection))
                            .on_activate(cx.listener(|_, _, _, cx| {
                                cx.emit(AssetsPanelAction::AddLibrariesRequested);
                            }))
                            .child("Add more libraries"),
                    ),
            )
            .into_any_element()
    }
}

impl Focusable for AssetsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AssetsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .id(self.id.clone())
            .track_focus(&self.focus_handle)
            .size_full()
            .min_h(px(0.))
            .items_start()
            .overflow_hidden()
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .border_1()
            .border_color(cx.theme().border)
            .child(self.render_rail(cx))
            .child(self.render_content(cx))
    }
}

#[cfg(test)]
mod interaction_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assets_view_data_defaults_to_the_assets_destination() {
        let view_data = AssetsViewData::new([]);

        assert_eq!(view_data.active_rail_item, AssetsRailItem::Assets);
        assert!(view_data.libraries.is_empty());
    }

    #[test]
    fn assets_rail_item_builder_preserves_the_typed_destination() {
        let view_data = AssetsViewData::new([]).active_rail_item(AssetsRailItem::Variables);

        assert_eq!(view_data.active_rail_item, AssetsRailItem::Variables);
        assert_eq!(AssetsRailItem::Variables.label(), "Variables");
        assert_eq!(AssetsRailItem::Variables.slug(), "variables");
    }
}
