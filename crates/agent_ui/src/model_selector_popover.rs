use std::rc::Rc;

use acp_thread::{AgentModelId, AgentModelInfo, AgentModelSelector};
use gpui::{Entity, FocusHandle};
use picker::popover_menu::PickerPopoverMenu;
use ui::{ButtonLike, PopoverMenuHandle, Tooltip, prelude::*};

use crate::{
    ModelSelector,
    model_selector::{ModelMenuModels, acp_model_selector},
};

/// The searchable model picker. The composer's model menu is its visible
/// trigger, so this renders only the anchor the picker opens from, beside
/// that menu's button; `toggle` opens it (Search Models…).
pub struct ModelSelectorPopover {
    selector: Entity<ModelSelector>,
    menu_handle: PopoverMenuHandle<ModelSelector>,
}

impl ModelSelectorPopover {
    pub(crate) fn new(
        selector: Rc<dyn AgentModelSelector>,
        menu_handle: PopoverMenuHandle<ModelSelector>,
        focus_handle: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            selector: cx
                .new(move |cx| acp_model_selector(selector, focus_handle.clone(), window, cx)),
            menu_handle,
        }
    }

    pub fn toggle(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu_handle.toggle(window, cx);
    }

    #[cfg(test)]
    pub fn is_picker_open(&self) -> bool {
        self.menu_handle.is_deployed()
    }

    pub fn active_model<'a>(&self, cx: &'a App) -> Option<&'a AgentModelInfo> {
        self.selector.read(cx).delegate.active_model()
    }

    pub fn favorites_count(&self, cx: &App) -> usize {
        self.selector.read(cx).delegate.favorites_count()
    }

    pub fn menu_models(&self, cx: &App) -> ModelMenuModels {
        self.selector.read(cx).delegate.menu_models()
    }

    pub fn select_model(&self, id: &AgentModelId, window: &mut Window, cx: &mut Context<Self>) {
        self.selector.update(cx, |selector, cx| {
            selector.delegate.select_model_with_id(id, window, cx)
        });
        cx.notify();
    }

    pub fn cycle_favorite_models(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.selector.update(cx, |selector, cx| {
            selector.delegate.cycle_favorite_models(window, cx);
        });
    }
}

impl Render for ModelSelectorPopover {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        PickerPopoverMenu::new(
            self.selector.clone(),
            ButtonLike::new("model-picker-anchor")
                .style(ButtonStyle::Transparent)
                .size(ButtonSize::None)
                .disabled(true),
            Tooltip::text("Search Models"),
            gpui::Anchor::BottomRight,
            cx,
        )
        .with_handle(self.menu_handle.clone())
        .render(window, cx)
    }
}
