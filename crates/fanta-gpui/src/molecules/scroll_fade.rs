//! Edge-fade affordance for horizontally scrollable rows.
//!
//! Rows that `overflow_x_scroll` clip silently, so off-screen controls are
//! undiscoverable. This module owns the shared recipe once: a prepaint canvas
//! that derives per-edge fade visibility from a row's tracked scroll state and
//! notifies only when it changes, plus the pointer-transparent gradient
//! overlays a caller layers over the clipped edges.

use gpui::{
    AnyElement, Div, Entity, Hsla, InteractiveElement as _, IntoElement, Pixels, ScrollHandle,
    Styled as _, canvas, div, linear_color_stop, linear_gradient, px,
};

/// Which edges of a scrollable row currently clip content.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EdgeFades {
    /// Content is clipped before the visible range (scrolled away from start).
    pub start: bool,
    /// Content is clipped after the visible range.
    pub end: bool,
}

/// Derives horizontal fade visibility from a tracked scroll handle.
///
/// Scroll offsets run from `0` at the start to `-max_offset` at the end; a
/// half-pixel tolerance absorbs the rounding the layout engine applies to the
/// scroll maximum.
pub(crate) fn horizontal_edge_fades(handle: &ScrollHandle) -> EdgeFades {
    let offset = handle.offset().x;
    let max = handle.max_offset().x;
    EdgeFades {
        start: offset < px(-0.5),
        end: max > px(0.5) && offset > px(0.5) - max,
    }
}

/// Absolute full-size canvas that keeps an entity's stored [`EdgeFades`] in
/// sync with a row's tracked scroll state.
///
/// Layer it as the last child of the `relative()` wrapper that also hosts the
/// scroll viewport, after the viewport so the handle's offsets are fresh. The
/// entity update is deferred out of the draw so its notify schedules a real
/// redraw, and it notifies only when visibility changes, so the extra frame
/// converges.
pub fn track_horizontal_edge_fades<V: 'static>(
    entity: Entity<V>,
    handle: ScrollHandle,
    read: impl Fn(&V) -> EdgeFades + 'static,
    write: impl Fn(&mut V, EdgeFades) + 'static,
) -> impl IntoElement {
    canvas(
        move |_, _, app| {
            let fades = horizontal_edge_fades(&handle);
            app.defer(move |app| {
                entity.update(app, |view, cx| {
                    if read(view) != fades {
                        write(view, fades);
                        cx.notify();
                    }
                });
            });
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// Pointer-transparent gradient overlays for a row's clipped edges.
///
/// Returns one absolutely positioned overlay per visible fade, fading from the
/// row's `background` color at the clipped edge to transparent toward the
/// content. The caller adds them to the same `relative()` wrapper as the
/// scroll viewport; they carry `{selector_prefix}-fade-start` and
/// `{selector_prefix}-fade-end` debug selectors.
pub fn horizontal_fade_overlays(
    fades: EdgeFades,
    width: Pixels,
    background: Hsla,
    selector_prefix: &'static str,
) -> Vec<AnyElement> {
    let overlay = |edge_is_start: bool| -> AnyElement {
        let base: Div = div()
            .absolute()
            .top_0()
            .bottom_0()
            .w(width)
            .bg(linear_gradient(
                if edge_is_start { 90. } else { 270. },
                linear_color_stop(background, 0.),
                linear_color_stop(background.opacity(0.), 1.),
            ));
        if edge_is_start {
            base.left_0()
                .debug_selector(move || format!("{selector_prefix}-fade-start"))
        } else {
            base.right_0()
                .debug_selector(move || format!("{selector_prefix}-fade-end"))
        }
        .into_any_element()
    };
    let mut overlays = Vec::new();
    if fades.start {
        overlays.push(overlay(true));
    }
    if fades.end {
        overlays.push(overlay(false));
    }
    overlays
}
