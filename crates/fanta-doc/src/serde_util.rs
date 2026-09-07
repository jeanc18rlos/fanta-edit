//! Small serde field helpers shared across the crate.
//!
//! These back `#[serde(default = "...", skip_serializing_if = "...")]` on
//! fields so default values are omitted from JSON and old docs round-trip
//! byte-identical. They live at the crate root (rather than under `node`)
//! because both the node modules and `style` — a `node` dependency, so it
//! cannot reach into `node` — need the same predicates.

pub(crate) fn is_false(v: &bool) -> bool {
    !*v
}

pub(crate) fn is_zero_f32(v: &f32) -> bool {
    *v == 0.0
}

pub(crate) fn default_opacity() -> f32 {
    1.0
}

pub(crate) fn is_default_opacity(opacity: &f32) -> bool {
    (*opacity - 1.0).abs() < f32::EPSILON
}
