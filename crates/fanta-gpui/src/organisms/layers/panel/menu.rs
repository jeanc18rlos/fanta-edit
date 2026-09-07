use gpui::{InteractiveElement as _, MouseDownEvent, deferred, size};

use crate::molecules::{clamp_menu_origin, menu_item, menu_surface};

use super::*;

const MENU_WIDTH: f32 = 224.;
const MENU_ITEM_HEIGHT: f32 = 28.;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MenuEntry {
    pub(super) action: LayersPanelContextAction,
}

impl MenuEntry {
    const fn new(action: LayersPanelContextAction) -> Self {
        Self { action }
    }
}

impl LayersPanel {
    pub(super) fn render_context_menu(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (Some(panel_bounds), Some(menu)) = (self.panel_bounds, self.menu.as_ref()) else {
            return div().into_any_element();
        };
        let sections = menu_sections_for(menu.node.kind);
        let row_count = sections.iter().map(Vec::len).sum::<usize>();
        let separator_count = sections.len().saturating_sub(1);
        let estimated_height =
            px(row_count as f32 * MENU_ITEM_HEIGHT + separator_count as f32 * 9. + 16.);
        // Menus clamp against the window like other Fanta popups (§12), not
        // against the anchoring panel: the deferred surface may extend past a
        // narrow rail but must stay fully on screen.
        let window_size = window.viewport_size();
        let menu_height = estimated_height.min(px(620.)).min(window_size.height);
        let clamped =
            clamp_menu_origin(menu.anchor, window_size, size(px(MENU_WIDTH), menu_height));
        let origin = clamped - panel_bounds.origin;

        let mut surface = menu_surface(
            SharedString::from(format!("{}-context-menu", self.id)),
            origin,
            px(MENU_WIDTH),
            menu_height,
            px(10.),
            cx,
        )
        .debug_selector(|| "layers-context-menu".to_owned())
        .track_scroll(&self.menu_scroll_handle)
        .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _, cx| {
            if this.menu.take().is_some() {
                cx.notify();
            }
        }));

        let mut first = true;
        for (section_index, section) in sections.into_iter().enumerate() {
            if section_index > 0 {
                surface = surface.child(
                    div()
                        .h(px(1.))
                        .w_full()
                        .my_1()
                        .flex_none()
                        .bg(cx.theme().border),
                );
            }
            for entry in section {
                surface = surface.child(self.render_menu_item(entry, first, cx));
                first = false;
            }
        }

        deferred(
            surface.child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .debug_selector(|| "layers-context-menu-scrollbar".to_owned())
                    .child(
                        Scrollbar::vertical(&self.menu_scroll_handle).id(SharedString::from(
                            format!("{}-context-menu-scrollbar", self.id),
                        )),
                    ),
            ),
        )
        .with_priority(4)
        .into_any_element()
    }

    fn render_menu_item(
        &mut self,
        entry: MenuEntry,
        first: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let action = entry.action;
        let selector = format!("layers-menu-{}", action.selector_slug());
        menu_item(
            SharedString::from(format!("{}-menu-{}", self.id, action.selector_slug())),
            px(MENU_ITEM_HEIGHT),
            cx,
        )
        .debug_selector(move || selector)
        .when(first, |item| {
            item.track_focus(&self.menu_focus_handle.clone().tab_index(0).tab_stop(true))
        })
        .on_activate(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            this.activate_menu_action(action, window, cx);
        }))
        .child(div().flex_1().child(action.label()))
        .when_some(action.shortcut(), |row, shortcut| {
            row.child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(shortcut),
            )
        })
        .when(action.has_submenu(), |row| {
            row.child(Icon::new(IconName::ChevronRight).xsmall())
        })
        .into_any_element()
    }
}

pub(super) fn menu_sections_for(kind: LayersPanelNodeKind) -> Vec<Vec<MenuEntry>> {
    use LayersPanelContextAction as Action;

    let mut sections = vec![
        entries(&[
            Action::Copy,
            Action::PasteToReplace,
            Action::CopyPasteAs,
            Action::SendToFigmaMake,
            Action::FindSimilarDesigns,
            Action::AddMotion,
        ]),
        entries(&[Action::MoveToPage, Action::BringToFront, Action::SendToBack]),
    ];

    let structural = match kind {
        LayersPanelNodeKind::Section => entries(&[
            Action::ConvertToFrame,
            Action::Rename,
            Action::RenameLayers,
            Action::SetAsThumbnail,
        ]),
        LayersPanelNodeKind::Frame => entries(&[
            Action::ConvertToSection,
            Action::GroupSelection,
            Action::FrameSelection,
            Action::RemoveFrame,
            Action::Rename,
            Action::RenameLayers,
            Action::Flatten,
            Action::OutlineStroke,
            Action::UseAsMask,
            Action::SetAsThumbnail,
        ]),
        LayersPanelNodeKind::Group => entries(&[
            Action::ConvertToFrame,
            Action::ConvertToSection,
            Action::GroupSelection,
            Action::FrameSelection,
            Action::Ungroup,
            Action::Rename,
            Action::RenameLayers,
            Action::Flatten,
            Action::OutlineStroke,
            Action::UseAsMask,
            Action::SetAsThumbnail,
        ]),
        LayersPanelNodeKind::Component | LayersPanelNodeKind::ComponentSet => entries(&[
            Action::GroupSelection,
            Action::FrameSelection,
            Action::Rename,
            Action::RenameLayers,
            Action::Flatten,
            Action::OutlineStroke,
            Action::UseAsMask,
            Action::SetAsThumbnail,
        ]),
        LayersPanelNodeKind::Instance => entries(&[
            Action::GroupSelection,
            Action::FrameSelection,
            Action::Rename,
            Action::RenameLayers,
            Action::Flatten,
            Action::GoToMainComponent,
            Action::DetachInstance,
            Action::ResetInstance,
        ]),
        LayersPanelNodeKind::Text => entries(&[
            Action::EditText,
            Action::GroupSelection,
            Action::FrameSelection,
            Action::Rename,
            Action::RenameLayers,
            Action::Flatten,
            Action::OutlineStroke,
            Action::UseAsMask,
        ]),
        LayersPanelNodeKind::Image => entries(&[
            Action::CropImage,
            Action::ReplaceMedia,
            Action::GroupSelection,
            Action::FrameSelection,
            Action::Rename,
            Action::RenameLayers,
            Action::Flatten,
            Action::UseAsMask,
        ]),
        LayersPanelNodeKind::Video => entries(&[
            Action::ReplaceMedia,
            Action::GroupSelection,
            Action::FrameSelection,
            Action::Rename,
            Action::RenameLayers,
            Action::UseAsMask,
        ]),
        LayersPanelNodeKind::Slice => entries(&[Action::Rename, Action::RenameLayers]),
        LayersPanelNodeKind::Mask => entries(&[
            Action::GroupSelection,
            Action::FrameSelection,
            Action::Ungroup,
            Action::Rename,
            Action::RenameLayers,
            Action::Flatten,
        ]),
        LayersPanelNodeKind::Rectangle
        | LayersPanelNodeKind::Ellipse
        | LayersPanelNodeKind::Polygon
        | LayersPanelNodeKind::Star
        | LayersPanelNodeKind::Line
        | LayersPanelNodeKind::Arrow
        | LayersPanelNodeKind::Vector
        | LayersPanelNodeKind::BooleanOperation
        | LayersPanelNodeKind::Pen
        | LayersPanelNodeKind::Pencil
        | LayersPanelNodeKind::Other => entries(&[
            Action::GroupSelection,
            Action::FrameSelection,
            Action::Rename,
            Action::RenameLayers,
            Action::Flatten,
            Action::OutlineStroke,
            Action::UseAsMask,
        ]),
    };
    sections.push(structural);

    let mut layout = Vec::new();
    if kind.supports_auto_layout() {
        layout.push(MenuEntry::new(Action::AddAutoLayout));
        if kind.is_container() {
            layout.push(MenuEntry::new(Action::MoreLayoutOptions));
        }
    }
    if kind.can_create_component() {
        layout.push(MenuEntry::new(Action::CreateComponent));
    }
    if !layout.is_empty() {
        sections.push(layout);
    }

    sections.push(entries(&[Action::Plugins, Action::Widgets]));
    sections.push(entries(&[Action::ShowHide, Action::LockUnlock]));
    if kind.can_flip() {
        sections.push(entries(&[Action::FlipHorizontal, Action::FlipVertical]));
    }
    sections
}

fn entries(actions: &[LayersPanelContextAction]) -> Vec<MenuEntry> {
    actions.iter().copied().map(MenuEntry::new).collect()
}
