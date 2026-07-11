use std::{collections::BTreeMap, rc::Rc, sync::Arc};

use gpui::{
    Anchor, AnyElement, App, Context, DismissEvent, ElementId, IntoElement, RenderOnce, Role,
    SharedString, Task, Window, point, px,
};
use picker::{Picker, PickerDelegate};
use ui::{ListItem, ListItemSpacing, PopoverMenu, PopoverMenuHandle, Tooltip, prelude::*};

type FontPicker = Picker<FontFamilyPickerDelegate>;
type ChangeHandler = Rc<dyn Fn(SharedString, &mut Window, &mut App)>;
type OpenHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// A compact Zed-style combobox for choosing a font family.
///
/// Typing only filters the popover. The document callback runs once, when a
/// highlighted family is confirmed, so partial search queries can never become
/// document font names.
#[derive(IntoElement)]
pub struct FontFamilyPicker {
    id: ElementId,
    current: SharedString,
    families: Vec<SharedString>,
    disabled: bool,
    on_change: ChangeHandler,
    on_open: Option<OpenHandler>,
}

impl FontFamilyPicker {
    pub fn new(
        id: impl Into<ElementId>,
        current: impl Into<SharedString>,
        families: impl IntoIterator<Item = impl Into<SharedString>>,
        on_change: impl Fn(SharedString, &mut Window, &mut App) + 'static,
    ) -> Self {
        let current = current.into();
        Self {
            id: id.into(),
            families: normalize_font_families(&current, families),
            current,
            disabled: false,
            on_change: Rc::new(on_change),
            on_open: None,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Runs immediately after the popover is deployed and before it receives
    /// focus. Canvas integrations use this to retain an active rich-text range.
    pub fn on_open(mut self, on_open: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_open = Some(Rc::new(on_open));
        self
    }
}

impl RenderOnce for FontFamilyPicker {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let display = if self.current.trim().is_empty() {
            SharedString::from("Mixed")
        } else {
            self.current.clone()
        };
        let handle = PopoverMenuHandle::<FontPicker>::default();
        let show_handle = handle.clone();
        let hide_handle = handle.clone();
        let trigger = Button::new((self.id.clone(), "trigger"), display.clone())
            .aria_role(Role::ComboBox)
            .aria_label(format!("Font family: {display}"))
            .aria_expanded(handle.is_deployed())
            .on_a11y_action(gpui::accesskit::Action::Expand, move |_, window, cx| {
                show_handle.show(window, cx);
            })
            .on_a11y_action(gpui::accesskit::Action::Collapse, move |_, _window, cx| {
                hide_handle.hide(cx);
            })
            .style(ButtonStyle::Outlined)
            .size(ButtonSize::Medium)
            .start_icon(Icon::new(IconName::Font).size(IconSize::XSmall))
            .end_icon(
                Icon::new(IconName::ChevronUpDown)
                    .size(IconSize::XSmall)
                    .color(Color::Muted),
            )
            .truncate(true)
            .full_width()
            .tab_index(0_isize)
            .disabled(self.disabled)
            .tooltip(Tooltip::text(display.to_string()));

        let current = self.current;
        let families = self.families;
        let on_change = self.on_change;
        let mut popover = PopoverMenu::new((self.id, "popover"))
            .trigger(trigger)
            .menu(move |window, cx| {
                let current = current.clone();
                let families = families.clone();
                let on_change = on_change.clone();
                Some(cx.new(move |cx| font_picker(current, families, on_change, window, cx)))
            })
            .anchor(Anchor::TopLeft)
            .offset(point(px(0.), px(2.)))
            .with_handle(handle);
        if let Some(on_open) = self.on_open {
            popover = popover.on_open(on_open);
        }

        let element = div().w_full().child(popover);
        #[cfg(test)]
        let element = element.debug_selector(|| "fanta-font-family-picker".to_string());
        element
    }
}

fn normalize_font_families(
    current: &SharedString,
    families: impl IntoIterator<Item = impl Into<SharedString>>,
) -> Vec<SharedString> {
    let mut by_folded_name = BTreeMap::<String, SharedString>::new();
    for family in families {
        let family = family.into();
        let trimmed = family.trim();
        if !trimmed.is_empty() {
            by_folded_name
                .entry(trimmed.to_lowercase())
                .or_insert_with(|| SharedString::from(trimmed.to_owned()));
        }
    }
    let current = current.trim();
    if !current.is_empty() {
        // Preserve the spelling already authored in the document.
        by_folded_name.insert(
            current.to_lowercase(),
            SharedString::from(current.to_owned()),
        );
    }
    by_folded_name.into_values().collect()
}

fn filtered_font_indices(families: &[SharedString], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    families
        .iter()
        .enumerate()
        .filter_map(|(index, family)| {
            (query.is_empty() || family.to_lowercase().contains(&query)).then_some(index)
        })
        .collect()
}

pub struct FontFamilyPickerDelegate {
    families: Vec<SharedString>,
    filtered_indices: Vec<usize>,
    selected_index: usize,
    current: SharedString,
    on_change: ChangeHandler,
}

impl FontFamilyPickerDelegate {
    fn new(current: SharedString, families: Vec<SharedString>, on_change: ChangeHandler) -> Self {
        let filtered_indices = filtered_font_indices(&families, "");
        let selected_index = filtered_indices
            .iter()
            .position(|index| {
                families[*index]
                    .as_ref()
                    .eq_ignore_ascii_case(current.as_ref())
            })
            .unwrap_or_default();
        Self {
            families,
            filtered_indices,
            selected_index,
            current,
            on_change,
        }
    }
}

impl PickerDelegate for FontFamilyPickerDelegate {
    type ListItem = AnyElement;

    fn name() -> &'static str {
        "font family picker"
    }

    fn match_count(&self) -> usize {
        self.filtered_indices.len()
    }

    fn selected_index(&self) -> usize {
        self.selected_index
    }

    fn set_selected_index(&mut self, index: usize, _: &mut Window, cx: &mut Context<FontPicker>) {
        self.selected_index = index.min(self.filtered_indices.len().saturating_sub(1));
        cx.notify();
    }

    fn placeholder_text(&self, _window: &mut Window, _cx: &mut App) -> Arc<str> {
        "Search fonts…".into()
    }

    fn update_matches(
        &mut self,
        query: String,
        _window: &mut Window,
        cx: &mut Context<FontPicker>,
    ) -> Task<()> {
        self.filtered_indices = filtered_font_indices(&self.families, &query);
        self.selected_index = if query.trim().is_empty() {
            self.filtered_indices
                .iter()
                .position(|index| self.families[*index].eq_ignore_ascii_case(self.current.as_ref()))
                .unwrap_or_default()
        } else {
            0
        };
        cx.notify();
        Task::ready(())
    }

    fn confirm(&mut self, _secondary: bool, window: &mut Window, cx: &mut Context<FontPicker>) {
        let Some(family_index) = self.filtered_indices.get(self.selected_index) else {
            return;
        };
        let Some(family) = self.families.get(*family_index).cloned() else {
            return;
        };
        if !family.eq_ignore_ascii_case(self.current.as_ref()) {
            (self.on_change)(family.clone(), window, cx);
            self.current = family;
            cx.notify();
        }
    }

    fn dismissed(&mut self, window: &mut Window, cx: &mut Context<FontPicker>) {
        cx.defer_in(window, |picker, window, cx| {
            picker.set_query("", window, cx);
        });
        cx.emit(DismissEvent);
    }

    fn render_match(
        &self,
        index: usize,
        selected: bool,
        _window: &mut Window,
        _cx: &mut Context<FontPicker>,
    ) -> Option<Self::ListItem> {
        let family_index = *self.filtered_indices.get(index)?;
        let family = self.families.get(family_index)?.clone();
        let is_current = family.eq_ignore_ascii_case(self.current.as_ref());
        Some(
            ListItem::new(index)
                .inset(true)
                .spacing(ListItemSpacing::Sparse)
                .toggle_state(selected)
                .aria_role(Role::ListBoxOption)
                .aria_label(family.clone())
                .when(is_current, |item| {
                    item.end_slot(
                        Icon::new(IconName::Check)
                            .size(IconSize::XSmall)
                            .color(Color::Accent),
                    )
                })
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .font_family(family.clone())
                        .child(Label::new(family).size(LabelSize::Small).truncate()),
                )
                .into_any_element(),
        )
    }
}

fn font_picker(
    current: SharedString,
    families: Vec<SharedString>,
    on_change: ChangeHandler,
    window: &mut Window,
    cx: &mut Context<FontPicker>,
) -> FontPicker {
    Picker::uniform_list(
        FontFamilyPickerDelegate::new(current, families, on_change),
        window,
        cx,
    )
    .show_scrollbar(true)
    .initial_width(rems_from_px(248.))
    .max_height(rems(22.))
    .popover()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_are_trimmed_case_insensitively_deduplicated_and_sorted() {
        let choices = normalize_font_families(
            &"Inter".into(),
            [" Zed Sans ", "inter", "Arial", "", "arial"],
        );
        assert_eq!(
            choices,
            vec![
                SharedString::from("Arial"),
                SharedString::from("Inter"),
                SharedString::from("Zed Sans"),
            ]
        );
    }

    #[test]
    fn filtering_is_case_insensitive_and_never_changes_the_source_catalog() {
        let families = vec![
            "Inter".into(),
            "Source Sans 3".into(),
            "Source Serif 4".into(),
        ];
        assert_eq!(filtered_font_indices(&families, "SOURCE"), vec![1, 2]);
        assert_eq!(filtered_font_indices(&families, " serif "), vec![2]);
        assert_eq!(families[0], "Inter");
    }

    #[cfg(feature = "test-support")]
    #[gpui::test]
    fn compact_picker_has_a_deterministic_full_width_trigger(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            assets::Assets.load_test_fonts(cx);
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
        });
        let window = cx.add_window(|_, _| PickerHarness);
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, cx| window.draw(cx).clear());
        let bounds = visual
            .debug_bounds("fanta-font-family-picker")
            .expect("font picker is laid out");
        assert_eq!(bounds.size.width, px(240.));
        assert_eq!(bounds.size.height, px(28.));
    }

    #[cfg(feature = "test-support")]
    struct PickerHarness;

    #[cfg(feature = "test-support")]
    impl gpui::Render for PickerHarness {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().w(px(240.)).child(FontFamilyPicker::new(
                "font-picker-test",
                "Source Sans 3",
                ["Inter", "Source Sans 3", "Source Serif 4"],
                |_, _, _| {},
            ))
        }
    }
}
