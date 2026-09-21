//! Print an [`FnxElement`] tree as React/TSX-style `.fnx` source.
//!
//! Deterministic: attributes and embedded object keys are sorted explicitly,
//! independent of `serde_json`'s feature-selected map order, and indent is fixed.
//! Identical input ⇒ identical bytes, so git diffs stay clean.

use crate::model::FnxElement;
use crate::refs::RefTable;
use serde_json::{Number, Value};

const GENERATED_MARKER: &str = "// @generated fanta source — the design is the source of truth; ids live in the .ids sidecar\n";

/// Magnitude below which a printed float is arithmetic dust rather than a
/// value anyone authored, and prints as a flush zero.
///
/// `.fnx` numbers are design units — one unit is one CSS pixel, ~0.26 mm — so
/// 1e-9 units is roughly a quarter of a picometre: below anything a designer
/// can place, a renderer can resolve, or a downstream consumer preserves. The
/// noise this clamps sits four more orders of magnitude down (a `.fig`
/// coordinate is an `f32` widened to `f64`, and one auto-layout solve later a
/// flush-left frame reads `1.1368683772161603e-13`), while the smallest
/// quantity a design can actually mean — a sub-pixel nudge, a normalized
/// gradient stop, a barely-there opacity — is many orders above it. So the
/// clamp cannot swallow an intended value, and it cannot leave dust behind.
const NEGLIGIBLE_MAGNITUDE: f64 = 1e-9;

/// Longest run of zeros an expanded plain decimal may open with before the
/// exponent form is left alone, so no attribute can grow unboundedly wide.
/// [`NEGLIGIBLE_MAGNITUDE`] already floors the printer at 1e-9 — eight leading
/// zeros — so this bound only keeps [`to_positional`] total on its own terms.
const MAX_LEADING_ZEROS: usize = 12;

/// Render a single-root element tree as one `.fnx` file. `name` becomes the
/// (cosmetic) function name; the authoritative name is the root's `name`
/// attribute, so the function name is free to be sanitized.
///
/// Equivalent to [`print_doc_with`] with an empty [`RefTable`]: references
/// print as raw ULIDs, exactly as the codec always behaved.
pub fn print_doc(name: &str, root: &FnxElement) -> String {
    print_doc_with(name, root, &RefTable::default())
}

/// [`print_doc`] with a name-emission context: when the table opted into
/// `emit_names`, `component` ids and binding variable ids re-sugar into their
/// unambiguous names / `$Collection/Name` paths (see [`crate::refs`]).
pub fn print_doc_with(name: &str, root: &FnxElement, refs: &RefTable) -> String {
    // Sugar a clone so the caller's tree is untouched: a generator-exact
    // rect/ellipse `Vector` prints as `<Rect>`/`<Ellipse>` and a
    // pure-translation `transform` prints as readable `x`/`y` (see
    // [`crate::sugar`]). Both re-sugar only when strictly lossless — as does
    // the reference sugar below (unambiguous names only, all-or-nothing per
    // bindings attribute).
    let mut root = root.clone();
    crate::sugar::sugar_shapes(&mut root);
    crate::sugar::sugar_transform(&mut root);
    crate::refs::sugar_refs(&mut root, refs);
    let mut out = String::with_capacity(256);
    out.push_str(crate::canonicalize::JSX_RUNTIME_PRAGMA);
    out.push('\n');
    out.push_str(crate::canonicalize::JSX_FACTORY_PRAGMA);
    out.push('\n');
    out.push_str(crate::canonicalize::FNX_TAG_IMPORT);
    out.push('\n');
    out.push_str(GENERATED_MARKER);
    out.push_str("export default function ");
    out.push_str(&fn_ident(name));
    out.push_str("() {\n  return (\n");
    print_element(&root, 2, &mut out);
    out.push_str("  );\n}\n");
    out
}

fn print_element(el: &FnxElement, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    out.push_str(&pad);
    out.push_str(&render_open_tag(el));
    if el.children.is_empty() {
        out.push('\n');
        return;
    }
    out.push('\n');
    for child in &el.children {
        print_element(child, depth + 1, out);
    }
    out.push_str(&pad);
    out.push_str(&render_close_tag(el));
    out.push('\n');
}

pub(crate) fn render_open_tag(el: &FnxElement) -> String {
    let mut out = String::new();
    out.push('<');
    out.push_str(&el.tag);
    for (key, value) in &el.attrs {
        out.push(' ');
        out.push_str(key);
        out.push('=');
        out.push_str(&render_attr(value));
    }
    if el.children.is_empty() {
        out.push_str(" />");
    } else {
        out.push('>');
    }
    out
}

pub(crate) fn render_close_tag(el: &FnxElement) -> String {
    format!("</{}>", el.tag)
}

/// String attributes render as `"…"`; everything else as `{value}`, where the
/// value uses a JSON-ish grammar with one shorthand: a Color object renders as
/// `fnxColor("#RRGGBB[AA]")`. The helper-call spelling keeps generated source
/// valid TSX, allowing standard formatters and language tooling to parse it.
pub(crate) fn render_attr(value: &Value) -> String {
    if value.is_string() {
        return serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned());
    }
    format!("{{{}}}", render_value(value))
}

/// Recursively render an attribute value: Color objects become `fnxColor(...)`,
/// other objects/arrays recurse, numbers go through [`render_number`], and the
/// remaining scalars/strings through `serde_json`.
///
/// Numbers are rendered the same way wherever they sit — a bare `x`, an
/// element of `transform={[a, b, c, d, tx, ty]}` or `local_size={[w, h]}`, a
/// field of a nested object — because every one of them is a coordinate a
/// reader compares against the others, and a `transform` that reads
/// `[1, 0, 0, 1, 8, 1.1368683772161603e-13]` is exactly as unreadable as the
/// sugared `y` would have been.
fn render_value(value: &Value) -> String {
    match value {
        Value::Object(obj) => {
            if let Some(hex) = crate::color::color_obj_to_hex(obj) {
                return format!("fnxColor({hex:?})");
            }
            let mut entries: Vec<_> = obj.iter().collect();
            entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
            let parts: Vec<String> = entries
                .into_iter()
                .map(|(k, v)| {
                    format!(
                        "{}: {}",
                        serde_json::to_string(k).unwrap_or_else(|_| "\"\"".to_owned()),
                        render_value(v)
                    )
                })
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
        Value::Array(arr) => {
            let parts: Vec<String> = arr.iter().map(render_value).collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Number(number) => render_number(number),
        scalar => serde_json::to_string(scalar).unwrap_or_else(|_| "null".to_owned()),
    }
}

/// One JSON number as `.fnx` source text.
///
/// Integers pass through verbatim: routing them via `f64` would round the ones
/// past 2^53, and an integer cannot carry dust in the first place. Floats go
/// through [`render_float`].
fn render_number(number: &Number) -> String {
    match number.as_f64().filter(|_| number.is_f64()) {
        Some(value) => render_float(value),
        None => number.to_string(),
    }
}

/// One float as a plain decimal: dust reads as a flush zero, and nothing at
/// design scale ever reaches the reader as scientific notation.
///
/// Everything that survives the clamp keeps `serde_json`'s shortest
/// round-tripping spelling, digit for digit — expanding an exponent only moves
/// the decimal point through those same digits — so a `.fig` coordinate that
/// needs all 17 significant digits still parses back to the identical `f64`.
pub(crate) fn render_float(value: f64) -> String {
    // Dust is clamped to a real `0.0` and then printed like any other float,
    // rather than short-circuited to the string "0", for two reasons. Zero has
    // to print in exactly ONE spelling or a coordinate that wobbles across the
    // epsilon between two imports rewrites its line for nothing — the churn
    // this clamp exists to stop — and `0.0` is already the spelling an exact
    // zero has here. And a bare `0` reparses as a JSON *integer*, which would
    // change the node's shape across the text boundary and break the crate's
    // round-trip contract for `transform={[0.0, -1.0, 1.0, 0.0, …]}`.
    //
    // A `Value` cannot hold a non-finite float, so folding those in here is
    // belt and braces: NaN and infinity are not coordinates either.
    let value = if value.is_finite() && value.abs() >= NEGLIGIBLE_MAGNITUDE {
        value
    } else {
        0.0
    };
    let Some(shortest) = Number::from_f64(value).map(|number| number.to_string()) else {
        // Unreachable — `from_f64` only rejects the non-finite values the
        // clamp above has already replaced — but the printer must not panic.
        return "0.0".to_owned();
    };
    let positional = shortest
        .split_once(['e', 'E'])
        .and_then(|(mantissa, exponent)| to_positional(mantissa, exponent));
    positional.unwrap_or(shortest)
}

/// Rewrite `<mantissa>e<exponent>` as a plain decimal carrying the very same
/// digits: `1.25e-8` becomes `0.0000000125`.
///
/// Only values below one are expanded, which covers every exponent
/// `serde_json` writes at design scale — it reaches for one below 1e-5. It
/// also uses one above 1e16, but no design holds a coordinate that large, and
/// expanding one would produce a run of digits with no decimal point, which
/// reparses as a JSON *integer* rather than a float. That type flip would be a
/// silent change of shape across the text boundary, so those keep the exponent.
///
/// `None` when the text is not the shape `serde_json` emits, or when the value
/// is outside the expandable range.
fn to_positional(mantissa: &str, exponent: &str) -> Option<String> {
    let exponent: i32 = exponent.trim_start_matches('+').parse().ok()?;
    let (sign, magnitude) = match mantissa.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", mantissa),
    };
    let (whole, fraction) = magnitude.split_once('.').unwrap_or((magnitude, ""));
    if whole.is_empty()
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    // Where the decimal point lands within the digits, counted from their
    // left. At or below zero the value is all fraction, and `-point` is how
    // many zeros stand between the point and the first significant digit.
    let point = i32::try_from(whole.len()).ok()?.checked_add(exponent)?;
    if point > 0 {
        return None;
    }
    let leading_zeros = usize::try_from(point.unsigned_abs()).ok()?;
    if leading_zeros > MAX_LEADING_ZEROS {
        return None;
    }
    Some(format!(
        "{sign}0.{}{whole}{fraction}",
        "0".repeat(leading_zeros)
    ))
}

/// A cosmetic, identifier-ish function name. Lossy on purpose — the real name
/// is the root's `name` attribute, so this never has to round-trip.
fn fn_ident(name: &str) -> String {
    let mut id: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if id.is_empty() || id.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        id.insert(0, '_');
    }
    id
}
