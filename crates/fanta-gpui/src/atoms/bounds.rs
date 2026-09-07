//! Bounds-tracking canvas shared by panels that anchor overlays to elements.

use gpui::{Bounds, Entity, IntoElement, Pixels, Styled as _, canvas};

/// Absolute full-size canvas that writes its measured bounds into `entity`
/// state during prepaint.
///
/// Replaces the per-panel copies of the measure-canvas pattern: layer it as a
/// child of the element whose bounds the entity needs (typically the last
/// child of a `relative()` container) and record them in `write`.
pub fn track_bounds<V: 'static>(
    entity: Entity<V>,
    write: impl Fn(&mut V, Bounds<Pixels>) + 'static,
) -> impl IntoElement {
    canvas(
        move |bounds, _, app| {
            entity.update(app, |view, _| write(view, bounds));
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}
