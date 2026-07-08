//! Readable position sugar applied at the print/parse (text) boundary: a node's
//! pure-translation `transform` renders as `x`/`y` attributes instead of a raw
//! 6-element affine array, so positions read like coordinates.
//!
//! Lossless and exact. The doc serializes `transform` as the SVG matrix
//! `[a, b, c, d, tx, ty]`; only an *exact* `[1, 0, 0, 1, tx, ty]` (identity 2×2,
//! finite translation) is sugared, and `x`/`y` reconstruct that array
//! byte-for-byte (`[1, 0, 0, 1, x, y]`). Any transform carrying rotation, scale,
//! or skew is left verbatim as `transform={[…]}`. Sugaring always emits BOTH
//! `x` and `y` (even when zero) so the inverse can always rebuild the array.

use crate::model::FnxElement;
use serde_json::{Value, json};

/// Rewrite each element's pure-translation `transform` into `x`/`y`, in place,
/// over the whole subtree. Applied to a clone just before printing.
pub(crate) fn sugar_transform(el: &mut FnxElement) {
    if let Some((tx, ty)) = el.attrs.get("transform").and_then(pure_translation) {
        el.attrs.remove("transform");
        el.attrs.insert("x".to_owned(), json!(tx));
        el.attrs.insert("y".to_owned(), json!(ty));
    }
    for child in &mut el.children {
        sugar_transform(child);
    }
}

/// Inverse of [`sugar_transform`]: fold `x`/`y` back into a `transform` array,
/// in place, over the whole subtree. Applied just after parsing.
pub(crate) fn desugar_transform(el: &mut FnxElement) {
    let x = el.attrs.get("x").and_then(Value::as_f64);
    let y = el.attrs.get("y").and_then(Value::as_f64);
    if x.is_some() || y.is_some() {
        el.attrs.remove("x");
        el.attrs.remove("y");
        el.attrs.insert(
            "transform".to_owned(),
            json!([1.0, 0.0, 0.0, 1.0, x.unwrap_or(0.0), y.unwrap_or(0.0)]),
        );
    }
    for child in &mut el.children {
        desugar_transform(child);
    }
}

/// `Some((tx, ty))` iff `value` is exactly `[1, 0, 0, 1, tx, ty]` with finite
/// translation — a pure translation with an identity 2×2 block.
fn pure_translation(value: &Value) -> Option<(f64, f64)> {
    let arr = value.as_array()?;
    if arr.len() != 6 {
        return None;
    }
    let c: Vec<f64> = arr.iter().map(Value::as_f64).collect::<Option<_>>()?;
    let identity_2x2 = c[0] == 1.0 && c[1] == 0.0 && c[2] == 0.0 && c[3] == 1.0;
    (identity_2x2 && c[4].is_finite() && c[5].is_finite()).then_some((c[4], c[5]))
}
