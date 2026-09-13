//! A composed, simulated editor shell for exercising Fanta GPUI components.

use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{ActiveTheme as _, StyledExt as _, h_flex, v_flex};

use crate::atoms::{
    CONTROL_KEY_CONTEXT, ControlExt as _, ControlIcon, icon_button, render_control_icon,
};
use crate::{
    assets::AssetsPanel, design::DesignPanel, layers::LayersPanel, pages::PagesPanel,
    prototype::PrototypePanel, timeline::Timeline, toolbar::EditorToolbar,
    variables::VariablesPage,
};

/// Reference design width of the left rail (§14).
pub const PSEUDO_EDITOR_LEFT_RAIL_WIDTH: f32 = 337.;
/// Reference design width of the right rail (§14).
pub const PSEUDO_EDITOR_RIGHT_RAIL_WIDTH: f32 = 473.;
/// Floor the left rail compresses to on narrow windows.
pub const PSEUDO_EDITOR_LEFT_RAIL_MIN_WIDTH: f32 = 120.;
/// Floor the right rail compresses to on narrow windows.
pub const PSEUDO_EDITOR_RIGHT_RAIL_MIN_WIDTH: f32 = 140.;
/// The canvas never yields below this width; the rails compress first.
pub const PSEUDO_EDITOR_MIN_CANVAS_WIDTH: f32 = 60.;
/// The canvas never yields below this height; the timeline strip compresses
/// first.
pub const PSEUDO_EDITOR_MIN_CANVAS_HEIGHT: f32 = 96.;
/// Height of the shell's top bar.
const PSEUDO_EDITOR_TOP_BAR_HEIGHT: f32 = 44.;
/// Reference design height of the bottom timeline strip.
pub const PSEUDO_EDITOR_TIMELINE_HEIGHT: f32 = 333.;
/// Floor the timeline strip compresses to when the canvas would otherwise
/// fall below [`PSEUDO_EDITOR_MIN_CANVAS_HEIGHT`].
pub const PSEUDO_EDITOR_TIMELINE_MIN_HEIGHT: f32 = 100.;
/// Narrowest window the shell lays out without clipping: both rails at their
/// floors beside the minimum canvas.
pub const PSEUDO_EDITOR_MIN_WIDTH: f32 = PSEUDO_EDITOR_LEFT_RAIL_MIN_WIDTH
    + PSEUDO_EDITOR_MIN_CANVAS_WIDTH
    + PSEUDO_EDITOR_RIGHT_RAIL_MIN_WIDTH;
/// Shortest window the shell lays out without clipping: the top bar plus the
/// minimum canvas and the minimum timeline strip.
pub const PSEUDO_EDITOR_MIN_HEIGHT: f32 = PSEUDO_EDITOR_TOP_BAR_HEIGHT
    + PSEUDO_EDITOR_MIN_CANVAS_HEIGHT
    + PSEUDO_EDITOR_TIMELINE_MIN_HEIGHT;
/// Width at which both rails hold their reference design widths beside a
/// comfortable canvas.
pub const PSEUDO_EDITOR_PREFERRED_WIDTH: f32 =
    PSEUDO_EDITOR_LEFT_RAIL_WIDTH + PSEUDO_EDITOR_RIGHT_RAIL_WIDTH + 430.;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PseudoEditorLeftSurface {
    Pages,
    Layers,
    #[default]
    Assets,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PseudoEditorRightSurface {
    Design,
    #[default]
    Prototype,
}

#[derive(Clone)]
pub struct PseudoEditorChildren {
    pub pages: Entity<PagesPanel>,
    pub layers: Entity<LayersPanel>,
    pub assets: Entity<AssetsPanel>,
    pub design: Entity<DesignPanel>,
    pub prototype: Entity<PrototypePanel>,
    pub timeline: Entity<Timeline>,
    pub toolbar: Entity<EditorToolbar>,
    pub variables: Entity<VariablesPage>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PseudoEditorAction {
    LeftSurfaceChanged { surface: PseudoEditorLeftSurface },
    RightSurfaceChanged { surface: PseudoEditorRightSurface },
    VariablesVisibilityChanged { visible: bool },
    PresentRequested,
    ShareRequested,
}

/// Presentation-only editor shell. Domain data remains owned by each child
/// component's host adapter.
pub struct PseudoEditor {
    id: SharedString,
    focus_handle: FocusHandle,
    children: PseudoEditorChildren,
    left_surface: PseudoEditorLeftSurface,
    right_surface: PseudoEditorRightSurface,
    variables_visible: bool,
}

impl EventEmitter<PseudoEditorAction> for PseudoEditor {}

impl PseudoEditor {
    pub fn new(
        id: impl Into<SharedString>,
        children: PseudoEditorChildren,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            id: id.into(),
            focus_handle: cx.focus_handle(),
            children,
            left_surface: PseudoEditorLeftSurface::default(),
            right_surface: PseudoEditorRightSurface::default(),
            variables_visible: false,
        }
    }

    pub fn set_left_surface(&mut self, surface: PseudoEditorLeftSurface, cx: &mut Context<Self>) {
        self.left_surface = surface;
        cx.notify();
    }

    pub fn set_right_surface(&mut self, surface: PseudoEditorRightSurface, cx: &mut Context<Self>) {
        self.right_surface = surface;
        cx.notify();
    }

    pub fn set_variables_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.variables_visible = visible;
        cx.notify();
    }

    fn small_tab<T: EventEmitter<PseudoEditorAction>>(
        &self,
        id: SharedString,
        selector: &'static str,
        label: &'static str,
        selected: bool,
        on_activate: impl Fn(&mut T, &mut Context<T>) + 'static,
        cx: &mut Context<T>,
    ) -> AnyElement {
        div()
            .id(id)
            .debug_selector(|| selector.to_owned())
            .key_context(CONTROL_KEY_CONTEXT)
            .tab_index(0)
            .h(px(26.))
            .px(px(8.))
            .items_center()
            .rounded(px(5.))
            .cursor_pointer()
            .text_size(px(10.))
            .border_1()
            .border_color(cx.theme().transparent)
            .when(selected, |tab| {
                tab.bg(cx.theme().tab_active)
                    .text_color(cx.theme().tab_active_foreground)
                    .font_semibold()
            })
            .hover(|style| style.bg(cx.theme().accent))
            .focus(|style| style.border_color(cx.theme().selection))
            .on_activate(cx.listener(move |this, _, _, cx| on_activate(this, cx)))
            .child(label)
            .into_any_element()
    }

    fn render_left(&self, cx: &mut Context<Self>) -> AnyElement {
        let panel = match self.left_surface {
            PseudoEditorLeftSurface::Pages => self.children.pages.clone().into_any_element(),
            PseudoEditorLeftSurface::Layers => self.children.layers.clone().into_any_element(),
            PseudoEditorLeftSurface::Assets => self.children.assets.clone().into_any_element(),
        };
        div()
            .debug_selector(|| "pseudo-editor-left-rail".to_owned())
            .w(px(PSEUDO_EDITOR_LEFT_RAIL_WIDTH))
            .min_w(px(PSEUDO_EDITOR_LEFT_RAIL_MIN_WIDTH))
            .h_full()
            .flex_shrink(1.)
            .min_h(px(0.))
            .overflow_hidden()
            .border_r_1()
            .border_color(cx.theme().border)
            .child(panel)
            .into_any_element()
    }

    fn render_right(&self, cx: &mut Context<Self>) -> AnyElement {
        let panel = match self.right_surface {
            PseudoEditorRightSurface::Design => self.children.design.clone().into_any_element(),
            PseudoEditorRightSurface::Prototype => {
                self.children.prototype.clone().into_any_element()
            }
        };
        div()
            .debug_selector(|| "pseudo-editor-right-rail".to_owned())
            .w(px(PSEUDO_EDITOR_RIGHT_RAIL_WIDTH))
            .min_w(px(PSEUDO_EDITOR_RIGHT_RAIL_MIN_WIDTH))
            .h_full()
            .flex_shrink(1.)
            .min_h(px(0.))
            .overflow_hidden()
            .border_l_1()
            .border_color(cx.theme().border)
            .child(panel)
            .into_any_element()
    }

    fn render_canvas(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .debug_selector(|| "pseudo-editor-canvas".to_owned())
            .relative()
            .flex_1()
            .h_full()
            .min_w(px(PSEUDO_EDITOR_MIN_CANVAS_WIDTH))
            .items_center()
            .justify_center()
            .overflow_hidden()
            .bg(cx.theme().muted)
            .child(
                v_flex()
                    .w(px(360.))
                    .max_w_full()
                    .h(px(240.))
                    .items_center()
                    .justify_center()
                    .gap(px(8.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(cx.theme().selection)
                    .bg(cx.theme().background)
                    .text_color(cx.theme().foreground)
                    .when(cx.theme().shadow, |card| card.shadow_lg())
                    .child(div().text_size(px(20.)).font_semibold().child("Fanta"))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(cx.theme().muted_foreground)
                            .child("A simulated editor canvas"),
                    ),
            )
            .child(
                h_flex()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom(px(12.))
                    .px_3()
                    .justify_center()
                    .child(self.children.toolbar.clone()),
            )
            .into_any_element()
    }
}

impl Focusable for PseudoEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PseudoEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let variables = self.children.variables.clone();
        v_flex()
            .id(self.id.clone())
            .track_focus(&self.focus_handle)
            .relative()
            .size_full()
            .min_h(px(0.))
            .overflow_hidden()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                h_flex()
                    .h(px(PSEUDO_EDITOR_TOP_BAR_HEIGHT))
                    .w_full()
                    .flex_none()
                    .px(px(12.))
                    .gap(px(10.))
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().font_semibold().text_size(px(12.)).child("Untitled"))
                    .child(self.small_tab(
                        SharedString::from(format!("{}-left-pages", self.id)),
                        "pseudo-editor-left-pages",
                        "Pages",
                        self.left_surface == PseudoEditorLeftSurface::Pages,
                        |this, cx| {
                            this.left_surface = PseudoEditorLeftSurface::Pages;
                            cx.emit(PseudoEditorAction::LeftSurfaceChanged {
                                surface: PseudoEditorLeftSurface::Pages,
                            });
                            cx.notify();
                        },
                        cx,
                    ))
                    .child(self.small_tab(
                        SharedString::from(format!("{}-left-layers", self.id)),
                        "pseudo-editor-left-layers",
                        "Layers",
                        self.left_surface == PseudoEditorLeftSurface::Layers,
                        |this, cx| {
                            this.left_surface = PseudoEditorLeftSurface::Layers;
                            cx.emit(PseudoEditorAction::LeftSurfaceChanged {
                                surface: PseudoEditorLeftSurface::Layers,
                            });
                            cx.notify();
                        },
                        cx,
                    ))
                    .child(self.small_tab(
                        SharedString::from(format!("{}-left-assets", self.id)),
                        "pseudo-editor-left-assets",
                        "Assets",
                        self.left_surface == PseudoEditorLeftSurface::Assets,
                        |this, cx| {
                            this.left_surface = PseudoEditorLeftSurface::Assets;
                            cx.emit(PseudoEditorAction::LeftSurfaceChanged {
                                surface: PseudoEditorLeftSurface::Assets,
                            });
                            cx.notify();
                        },
                        cx,
                    ))
                    .child(div().flex_1())
                    .child(self.small_tab(
                        SharedString::from(format!("{}-right-design", self.id)),
                        "pseudo-editor-right-design",
                        "Design",
                        self.right_surface == PseudoEditorRightSurface::Design,
                        |this, cx| {
                            this.right_surface = PseudoEditorRightSurface::Design;
                            cx.emit(PseudoEditorAction::RightSurfaceChanged {
                                surface: PseudoEditorRightSurface::Design,
                            });
                            cx.notify();
                        },
                        cx,
                    ))
                    .child(self.small_tab(
                        SharedString::from(format!("{}-right-prototype", self.id)),
                        "pseudo-editor-right-prototype",
                        "Prototype",
                        self.right_surface == PseudoEditorRightSurface::Prototype,
                        |this, cx| {
                            this.right_surface = PseudoEditorRightSurface::Prototype;
                            cx.emit(PseudoEditorAction::RightSurfaceChanged {
                                surface: PseudoEditorRightSurface::Prototype,
                            });
                            cx.notify();
                        },
                        cx,
                    ))
                    .child(
                        div()
                            .id(SharedString::from(format!("{}-variables", self.id)))
                            .debug_selector(|| "pseudo-editor-variables".to_owned())
                            .key_context(CONTROL_KEY_CONTEXT)
                            .tab_index(0)
                            .h(px(28.))
                            .px(px(9.))
                            .items_center()
                            .rounded(px(5.))
                            .cursor_pointer()
                            .text_size(px(10.))
                            .border_1()
                            .border_color(cx.theme().transparent)
                            .hover(|style| style.bg(cx.theme().accent))
                            .focus(|style| style.border_color(cx.theme().selection))
                            .on_activate(cx.listener(|this, _, _, cx| {
                                this.variables_visible = true;
                                cx.emit(PseudoEditorAction::VariablesVisibilityChanged {
                                    visible: true,
                                });
                                cx.notify();
                            }))
                            .child("Variables"),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("{}-present", self.id)))
                            .debug_selector(|| "pseudo-editor-present".to_owned())
                            .key_context(CONTROL_KEY_CONTEXT)
                            .tab_index(0)
                            .h(px(28.))
                            .px(px(9.))
                            .items_center()
                            .rounded(px(5.))
                            .cursor_pointer()
                            .text_size(px(10.))
                            .border_1()
                            .border_color(cx.theme().transparent)
                            .hover(|style| style.bg(cx.theme().accent))
                            .focus(|style| style.border_color(cx.theme().selection))
                            .on_activate(cx.listener(|_, _, _, cx| {
                                cx.emit(PseudoEditorAction::PresentRequested);
                            }))
                            .child("Present"),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("{}-share", self.id)))
                            .debug_selector(|| "pseudo-editor-share".to_owned())
                            .key_context(CONTROL_KEY_CONTEXT)
                            .tab_index(0)
                            .h(px(28.))
                            .px(px(12.))
                            .items_center()
                            .rounded(px(6.))
                            .bg(cx.theme().primary)
                            .text_color(cx.theme().primary_foreground)
                            .cursor_pointer()
                            .font_semibold()
                            .text_size(px(10.))
                            .border_1()
                            .border_color(cx.theme().transparent)
                            .hover(|style| style.bg(cx.theme().primary_hover))
                            .focus(|style| style.border_color(cx.theme().selection))
                            .on_activate(cx.listener(|_, _, _, cx| {
                                cx.emit(PseudoEditorAction::ShareRequested);
                            }))
                            .child("Share"),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h(px(PSEUDO_EDITOR_MIN_CANVAS_HEIGHT))
                    .items_start()
                    .child(self.render_left(cx))
                    .child(self.render_canvas(cx))
                    .child(self.render_right(cx)),
            )
            .child(
                // The timeline strip yields toward its floor before the
                // canvas row shrinks below its minimum height.
                div()
                    .debug_selector(|| "pseudo-editor-timeline-strip".to_owned())
                    .h(px(PSEUDO_EDITOR_TIMELINE_HEIGHT))
                    .min_h(px(PSEUDO_EDITOR_TIMELINE_MIN_HEIGHT))
                    .w_full()
                    .flex_shrink(1.)
                    .overflow_hidden()
                    .child(self.children.timeline.clone()),
            )
            .when(self.variables_visible, |root| {
                root.child(
                    v_flex()
                        .absolute()
                        .left_0()
                        .right_0()
                        .top_0()
                        .bottom_0()
                        .occlude()
                        .bg(cx.theme().background)
                        // The close control lives in an overlay-owned header
                        // strip, anchored by the overlay's own layout instead
                        // of offsets tuned to the Variables page's internal
                        // header geometry.
                        .child(
                            h_flex()
                                .w_full()
                                .flex_none()
                                .items_center()
                                .justify_end()
                                .px(px(10.))
                                .py(px(6.))
                                .border_b_1()
                                .border_color(cx.theme().border)
                                .child(
                                    icon_button(
                                        SharedString::from(format!("{}-close-variables", self.id)),
                                        px(30.),
                                        px(6.),
                                        cx,
                                    )
                                    .debug_selector(|| "pseudo-editor-close-variables".to_owned())
                                    .bg(cx.theme().secondary)
                                    .on_activate(cx.listener(|this, _, _, cx| {
                                        this.variables_visible = false;
                                        cx.emit(PseudoEditorAction::VariablesVisibilityChanged {
                                            visible: false,
                                        });
                                        cx.notify();
                                    }))
                                    .child(
                                        render_control_icon(
                                            ControlIcon::Close,
                                            cx.theme().foreground,
                                            12.,
                                        ),
                                    ),
                                ),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_h(px(0.))
                                .overflow_hidden()
                                .child(variables),
                        ),
                )
            })
    }
}

#[cfg(test)]
mod composed_host_flow_tests;
#[cfg(test)]
mod interaction_tests;
