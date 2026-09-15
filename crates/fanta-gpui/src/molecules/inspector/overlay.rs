//! Inspector-specific placement, chrome, dismissal, and focus-return policy.

use gpui::{
    Anchor, AnyElement, App, Context, Div, ElementId, FocusHandle, InteractiveElement as _, Pixels,
    Point, Stateful, StatefulInteractiveElement as _, Styled as _, Window,
    prelude::FluentBuilder as _,
};
use gpui_component::{ActiveTheme as _, h_flex, v_flex};

use super::{InspectorFieldAccess, InspectorMetrics};
use crate::molecules::{anchored_popup, menu_item};

/// Shared placement contract for trigger-anchored inspector surfaces.
///
/// Menus and popovers intentionally use the same window snapping and deferred
/// layer priority. Their content anatomy remains distinct.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InspectorOverlayPlacement {
    anchor: Anchor,
    offset: Point<Pixels>,
    priority: usize,
}

impl InspectorOverlayPlacement {
    pub const fn new(anchor: Anchor, offset: Point<Pixels>, priority: usize) -> Self {
        Self {
            anchor,
            offset,
            priority,
        }
    }

    pub const fn anchor(self) -> Anchor {
        self.anchor
    }

    pub const fn offset(self) -> Point<Pixels> {
        self.offset
    }

    pub const fn priority(self) -> usize {
        self.priority
    }
}

/// Why the retained owner is being asked to dismiss an inspector overlay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InspectorOverlayDismissCause {
    Escape,
    OutsideClick,
}

/// A retained focus handle captured from the trigger that opened an overlay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InspectorOverlayFocusTarget {
    handle: FocusHandle,
}

impl InspectorOverlayFocusTarget {
    pub fn new(handle: FocusHandle) -> Self {
        Self { handle }
    }

    pub fn handle(&self) -> &FocusHandle {
        &self.handle
    }

    /// Restores focus after the owner has removed the overlay from its next
    /// render. Deferral avoids focusing a trigger while the overlay's focus
    /// scope is still mounted.
    pub fn restore<C: 'static>(&self, window: &mut Window, cx: &mut Context<C>) {
        let handle = self.handle.clone();
        cx.defer_in(window, move |_, window, cx| {
            handle.focus(window, cx);
        });
    }
}

/// Domain-neutral dismissal request produced by a retained overlay owner.
///
/// The intent does not mutate open state. The owner first balances any active
/// edit transaction, clears its overlay state, then calls [`Self::restore_focus`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InspectorOverlayDismissIntent<Overlay> {
    overlay: Overlay,
    cause: InspectorOverlayDismissCause,
    focus_return: Option<InspectorOverlayFocusTarget>,
    generation: Option<u64>,
}

impl<Overlay> InspectorOverlayDismissIntent<Overlay> {
    pub fn new(
        overlay: Overlay,
        cause: InspectorOverlayDismissCause,
        focus_return: Option<InspectorOverlayFocusTarget>,
    ) -> Self {
        Self {
            overlay,
            cause,
            focus_return,
            generation: None,
        }
    }

    /// Creates a dismissal tied to one retained opening generation.
    ///
    /// Owners use this for callbacks that can be delivered after a surface
    /// has already closed and reopened. A generation-less intent remains the
    /// compatibility contract for owners that do not need stale-callback
    /// protection.
    pub fn new_versioned(
        overlay: Overlay,
        cause: InspectorOverlayDismissCause,
        focus_return: Option<InspectorOverlayFocusTarget>,
        generation: u64,
    ) -> Self {
        Self {
            overlay,
            cause,
            focus_return,
            generation: Some(generation),
        }
    }

    pub fn overlay(&self) -> &Overlay {
        &self.overlay
    }

    pub const fn cause(&self) -> InspectorOverlayDismissCause {
        self.cause
    }

    pub fn focus_return(&self) -> Option<&InspectorOverlayFocusTarget> {
        self.focus_return.as_ref()
    }

    pub const fn generation(&self) -> Option<u64> {
        self.generation
    }

    pub fn restore_focus<C: 'static>(&self, window: &mut Window, cx: &mut Context<C>) -> bool {
        let Some(target) = &self.focus_return else {
            return false;
        };
        target.restore(window, cx);
        true
    }
}

fn inspector_overlay_chrome(
    id: impl Into<ElementId>,
    metrics: InspectorMetrics,
    cx: &App,
) -> Stateful<Div> {
    v_flex()
        .id(id)
        .block_mouse_except_scroll()
        .rounded(metrics.radius)
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().popover)
        .overflow_y_scroll()
}

/// Standard inspector popover surface. Dismissal and focus return remain with
/// the retained component that owns the open state.
pub fn inspector_popover_surface(
    id: impl Into<ElementId>,
    metrics: InspectorMetrics,
    cx: &App,
) -> Stateful<Div> {
    inspector_overlay_chrome(id, metrics, cx)
        .text_color(cx.theme().popover_foreground)
        .when(cx.theme().shadow, |surface| surface.shadow_lg())
}

/// Menu chrome for an anchored inspector overlay. Unlike the compatibility
/// [`inspector_menu_surface`], placement is supplied by
/// [`inspector_anchored_menu`] rather than baked into the surface.
pub fn inspector_anchored_menu_surface(
    id: impl Into<ElementId>,
    width: Pixels,
    max_height: Pixels,
    metrics: InspectorMetrics,
    cx: &App,
) -> Stateful<Div> {
    inspector_overlay_chrome(id, metrics, cx)
        .w(width)
        .max_h(max_height)
        .py_2()
        .shadow_lg()
}

pub fn inspector_anchored_overlay(
    placement: InspectorOverlayPlacement,
    surface: impl gpui::IntoElement,
) -> AnyElement {
    anchored_popup(
        placement.anchor,
        placement.offset,
        placement.priority,
        surface,
    )
}

/// Compatibility entry point for existing inspector popovers.
pub fn inspector_anchored_popover(
    anchor: Anchor,
    offset: Point<Pixels>,
    priority: usize,
    surface: impl gpui::IntoElement,
) -> AnyElement {
    inspector_anchored_overlay(
        InspectorOverlayPlacement::new(anchor, offset, priority),
        surface,
    )
}

pub fn inspector_anchored_menu(
    placement: InspectorOverlayPlacement,
    surface: impl gpui::IntoElement,
) -> AnyElement {
    inspector_anchored_overlay(placement, surface)
}

pub fn inspector_menu_surface(
    id: impl Into<ElementId>,
    origin: Point<Pixels>,
    width: Pixels,
    max_height: Pixels,
    metrics: InspectorMetrics,
    cx: &App,
) -> Stateful<Div> {
    inspector_overlay_chrome(id, metrics, cx)
        .absolute()
        .left(origin.x)
        .top(origin.y)
        .w(width)
        .max_h(max_height)
        .py_2()
        .shadow_lg()
}

pub fn inspector_menu_item(
    id: impl Into<ElementId>,
    access: &InspectorFieldAccess,
    metrics: InspectorMetrics,
    cx: &App,
) -> Stateful<Div> {
    use gpui::Styled as _;
    if access.is_interactive() {
        menu_item(id, metrics.row_height, cx)
    } else {
        h_flex()
            .id(id)
            .h(metrics.row_height)
            .flex_none()
            .mx_2()
            .px_2()
            .gap_2()
            .rounded(metrics.radius)
            .text_xs()
            .text_color(gpui_component::ActiveTheme::theme(cx).muted_foreground)
            .opacity(0.62)
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Anchor, point, px};

    use super::*;

    #[test]
    fn placement_is_shared_without_losing_anchor_metadata() {
        let placement =
            InspectorOverlayPlacement::new(Anchor::BottomRight, point(px(-4.), px(6.)), 17);
        assert_eq!(placement.anchor(), Anchor::BottomRight);
        assert_eq!(placement.offset(), point(px(-4.), px(6.)));
        assert_eq!(placement.priority(), 17);
    }

    #[test]
    fn dismissal_intent_preserves_identity_and_cause_without_focus() {
        let intent = InspectorOverlayDismissIntent::new(
            "menu",
            InspectorOverlayDismissCause::OutsideClick,
            None,
        );
        assert_eq!(intent.overlay(), &"menu");
        assert_eq!(intent.cause(), InspectorOverlayDismissCause::OutsideClick);
        assert!(intent.focus_return().is_none());
    }
}
