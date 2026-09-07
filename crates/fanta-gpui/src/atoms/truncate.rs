//! Width-constrained one-line labels.

use gpui::{Div, ParentElement as _, SharedString, Styled as _, div, px};

/// Flexible label that truncates instead of forcing its row wider.
///
/// Replaces per-panel character-count width heuristics: the label claims the
/// remaining row width (`flex_1` with a zero min width) and ellipsizes
/// overflow on a single line.
pub fn truncating_label(text: impl Into<SharedString>) -> Div {
    div()
        .flex_1()
        .min_w(px(0.))
        .truncate()
        .whitespace_nowrap()
        .overflow_hidden()
        .child(text.into())
}
