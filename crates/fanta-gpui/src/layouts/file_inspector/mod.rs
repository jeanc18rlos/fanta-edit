//! A unified left sidebar that composes the Pages and Layers panels.

use gpui::{
    App, Context, Entity, FocusHandle, Focusable, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Window, div, px,
};
use gpui_component::{ActiveTheme as _, v_flex};

use crate::{layers::LayersPanel, pages::PagesPanel};

/// Narrowest width at which both child panels keep their supported layout.
pub const FILE_INSPECTOR_MIN_WIDTH: f32 = 240.;
/// Shortest useful combined sidebar viewport. Both child lists remain
/// independently scrollable below their natural content height.
pub const FILE_INSPECTOR_MIN_HEIGHT: f32 = 400.;

/// Presentation-only composition of the host-controlled Pages and Layers
/// panels. Child intents continue to be emitted by their original entities;
/// this layout only owns placement and focus routing.
pub struct FileInspectorSidebar {
    id: SharedString,
    focus_handle: FocusHandle,
    pages: Entity<PagesPanel>,
    layers: Entity<LayersPanel>,
}

impl FileInspectorSidebar {
    pub fn new(
        id: impl Into<SharedString>,
        pages: Entity<PagesPanel>,
        layers: Entity<LayersPanel>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            id: id.into(),
            focus_handle: cx.focus_handle(),
            pages,
            layers,
        }
    }
}

impl Focusable for FileInspectorSidebar {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for FileInspectorSidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id(self.id.clone())
            .debug_selector(|| "file-inspector-sidebar".to_owned())
            .track_focus(&self.focus_handle)
            .size_full()
            .min_w(px(0.))
            .min_h(px(0.))
            .overflow_hidden()
            .bg(cx.theme().sidebar)
            .child(
                div()
                    .debug_selector(|| "file-inspector-pages".to_owned())
                    .w_full()
                    .flex_none()
                    .child(self.pages.clone()),
            )
            // Overlap the two one-pixel panel borders so the shared seam is
            // visually one rule instead of a double border.
            .child(
                div()
                    .debug_selector(|| "file-inspector-layers".to_owned())
                    .mt(px(-1.))
                    .w_full()
                    .flex_1()
                    .min_h(px(0.))
                    .child(self.layers.clone()),
            )
    }
}

#[cfg(test)]
mod interaction_tests;
