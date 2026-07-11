use std::rc::Rc;

use gpui::{App, ElementId, FontWeight, IntoElement, RenderOnce, SharedString, Window};
use ui::Tooltip;
use ui::prelude::*;

pub type AnimationControlHandler = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AnimationProperty {
    Position,
    Scale,
    Rotation,
    Size,
    Opacity,
    Path,
}

impl AnimationProperty {
    pub const ALL: [Self; 6] = [
        Self::Position,
        Self::Scale,
        Self::Rotation,
        Self::Size,
        Self::Opacity,
        Self::Path,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Position => "Position",
            Self::Scale => "Scale",
            Self::Rotation => "Rotation",
            Self::Size => "Size",
            Self::Opacity => "Opacity",
            Self::Path => "Path",
        }
    }

    pub const fn icon(self) -> IconName {
        match self {
            Self::Position => IconName::Crosshair,
            Self::Scale => IconName::ToolScale,
            Self::Rotation => IconName::RotateCw,
            Self::Size => IconName::Maximize,
            Self::Opacity => IconName::Eye,
            Self::Path => IconName::ToolPathSelect,
        }
    }

    pub const fn supports_direction(self) -> bool {
        matches!(self, Self::Position)
    }

    pub const fn supports_distance(self) -> bool {
        matches!(
            self,
            Self::Position | Self::Rotation | Self::Scale | Self::Size
        )
    }
}

#[derive(IntoElement)]
pub struct AnimationPropertyRow {
    id: ElementId,
    property: AnimationProperty,
    subtitle: Option<SharedString>,
    selected: bool,
    enabled: bool,
    removable: bool,
    on_select: Option<AnimationControlHandler>,
    on_remove: Option<AnimationControlHandler>,
}

impl AnimationPropertyRow {
    pub fn new(id: impl Into<ElementId>, property: AnimationProperty) -> Self {
        Self {
            id: id.into(),
            property,
            subtitle: None,
            selected: false,
            enabled: true,
            removable: true,
            on_select: None,
            on_remove: None,
        }
    }

    pub fn subtitle(mut self, subtitle: impl Into<SharedString>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn removable(mut self, removable: bool) -> Self {
        self.removable = removable;
        self
    }

    pub fn on_select(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(handler));
        self
    }

    pub fn on_remove(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_remove = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for AnimationPropertyRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let property = self.property;
        let enabled = self.enabled;
        let on_select = self.on_select.filter(|_| enabled);
        let on_remove = self.on_remove.filter(|_| self.removable);
        let property_selector = format!("FANTA-ANIMATION-PROPERTY-{property:?}");
        h_flex()
            .id(self.id)
            .debug_selector(|| property_selector)
            .mx_2()
            .h_10()
            .px_2()
            .gap_2()
            .items_center()
            .rounded_md()
            .when(self.selected, |row| {
                row.bg(cx.theme().colors().element_selected)
            })
            .when(!self.selected, |row| {
                row.hover(|row| row.bg(cx.theme().colors().element_hover))
            })
            .when(enabled, |row| row.cursor_pointer())
            .when_some(on_select, |row, handler| {
                row.on_click(move |_, window, cx| handler(window, cx))
            })
            .child(
                Icon::new(property.icon())
                    .size(IconSize::Small)
                    .color(if enabled {
                        Color::Default
                    } else {
                        Color::Disabled
                    }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        Label::new(property.label())
                            .size(LabelSize::Small)
                            .color(if enabled {
                                Color::Default
                            } else {
                                Color::Disabled
                            }),
                    )
                    .children(self.subtitle.map(|subtitle| {
                        Label::new(subtitle)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                    })),
            )
            .children(on_remove.map(|handler| {
                div()
                    .debug_selector(|| format!("FANTA-ANIMATION-REMOVE-{property:?}"))
                    .child(
                        IconButton::new(
                            format!("fanta-animation-remove-{}", property.label()),
                            IconName::Close,
                        )
                        .icon_size(IconSize::XSmall)
                        .tooltip(Tooltip::text("Remove animation"))
                        .disabled(!enabled)
                        .on_click(move |_, window, cx| handler(window, cx)),
                    )
            }))
    }
}

#[derive(IntoElement)]
pub struct AnimationDetailHeader {
    property: AnimationProperty,
    on_close: Option<AnimationControlHandler>,
}

impl AnimationDetailHeader {
    pub fn new(property: AnimationProperty) -> Self {
        Self {
            property,
            on_close: None,
        }
    }

    pub fn on_close(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_close = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for AnimationDetailHeader {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        h_flex()
            .h_11()
            .px_3()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(cx.theme().colors().border_variant)
            .child(Icon::new(self.property.icon()).size(IconSize::Small))
            .child(Label::new(self.property.label()).weight(FontWeight::SEMIBOLD))
            .child(div().flex_1())
            .children(self.on_close.map(|handler| {
                div()
                    .debug_selector(|| "FANTA-ANIMATION-DETAIL-CLOSE".to_owned())
                    .child(
                        IconButton::new("fanta-animation-detail-close", IconName::Close)
                            .icon_size(IconSize::Small)
                            .tooltip(Tooltip::text("Close animation details"))
                            .on_click(move |_, window, cx| handler(window, cx)),
                    )
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::{AnimationProperty, AnimationPropertyRow};

    #[test]
    fn animation_property_capabilities_match_the_editor_contract() {
        assert!(AnimationProperty::Position.supports_direction());
        assert!(AnimationProperty::Position.supports_distance());
        assert!(!AnimationProperty::Opacity.supports_direction());
        assert!(!AnimationProperty::Opacity.supports_distance());
        assert_eq!(AnimationProperty::ALL.len(), 6);
    }

    #[test]
    fn read_only_animation_rows_remain_selectable_without_a_remove_control() {
        let row = AnimationPropertyRow::new("read-only-position", AnimationProperty::Position)
            .removable(false)
            .on_select(|_, _| {})
            .on_remove(|_, _| {});

        assert!(row.enabled);
        assert!(row.on_select.is_some());
        assert!(!row.removable);
        assert!(row.on_remove.is_some());
    }
}
