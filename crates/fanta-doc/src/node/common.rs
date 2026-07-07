//! Small serde helper predicates shared across the [`crate::node`] submodules.
//!
//! These back `#[serde(skip_serializing_if = "...")]` on several node fields so
//! default values are omitted from JSON and old docs round-trip byte-identical.
//! `pub(crate)` because more than one sibling submodule references them.

pub(crate) fn is_zero_f32(v: &f32) -> bool {
    *v == 0.0
}

pub(crate) fn is_false(v: &bool) -> bool {
    !*v
}
