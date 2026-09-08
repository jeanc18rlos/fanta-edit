//! Readable sugar applied at the print/parse (text) boundary. The in-memory
//! [`FnxElement`] tree is ALWAYS canonical — every fold here happens right
//! after parsing (desugar) or on a clone right before printing (sugar), so
//! sidecar fingerprints, reconciliation, and the doc projection never observe
//! a sugared spelling.
//!
//! Three sugars live here:
//!
//! - **Position**: a node's pure-translation `transform` renders as `x`/`y`
//!   attributes instead of a raw 6-element affine array. Lossless and exact.
//!   The doc serializes `transform` as the SVG matrix `[a, b, c, d, tx, ty]`;
//!   only an *exact* `[1, 0, 0, 1, tx, ty]` (identity 2×2, finite translation)
//!   is sugared, and `x`/`y` reconstruct that array byte-for-byte
//!   (`[1, 0, 0, 1, x, y]`). Any transform carrying rotation, scale, or skew
//!   is left verbatim as `transform={[…]}`. Sugaring always emits BOTH `x` and
//!   `y` (even when zero) so the inverse can always rebuild the array.
//! - **Size**: parse-only `width`/`height` folding into `clip_size`/
//!   `local_size` (see [`desugar_size`]).
//! - **Shapes**: `<Rect width={…} height={…}>` / `<Ellipse width={…}
//!   height={…}>` desugar into a canonical `<Vector>` whose `path` is the
//!   exact serde JSON of `fanta_doc::PathData::rect` / `::ellipse`. Printing
//!   re-sugars only when strictly lossless: the recognizer REGENERATES the
//!   path from the implied size and demands `Value` equality, so
//!   print→parse→print is a fixpoint by construction — no float tolerance
//!   anywhere (see [`sugar_shapes`]). A viewport (`local_size`) equal to the
//!   generated shape's own extent rides along on the sugared spelling; any
//!   other viewport crops the shape and blocks the sugar.

use crate::convert::FnxError;
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
///
/// `x`/`y` alongside an explicit `transform` is a hard error, not a merge: the
/// sugar rebuilds the whole matrix, so folding over an existing one would
/// silently drop its rotation/scale/skew, and ignoring `x`/`y` would silently
/// drop the author's position. Neither silent loss is acceptable in a
/// design-as-source file, and the printer never emits both spellings, so no
/// generated file ever hits this.
pub(crate) fn desugar_transform(el: &mut FnxElement) -> Result<(), FnxError> {
    let x = el.attrs.get("x").and_then(Value::as_f64);
    let y = el.attrs.get("y").and_then(Value::as_f64);
    if x.is_some() || y.is_some() {
        if el.attrs.contains_key("transform") {
            return Err(FnxError::Parse(format!(
                "`x`/`y` conflict with an explicit `transform` on {}; put the \
                 translation in the matrix or remove x/y",
                element_ref(el)
            )));
        }
        el.attrs.remove("x");
        el.attrs.remove("y");
        el.attrs.insert(
            "transform".to_owned(),
            json!([1.0, 0.0, 0.0, 1.0, x.unwrap_or(0.0), y.unwrap_or(0.0)]),
        );
    }
    for child in &mut el.children {
        desugar_transform(child)?;
    }
    Ok(())
}

/// Parse-time size sugar: fold `width`/`height` attributes into the node's
/// canonical size field, in place, over the whole subtree. Hand- and
/// agent-authored `.fnx` naturally reaches for `width={…} height={…}` (the JSX
/// idiom), but the doc model stores sizes as `clip_size` (a Frame's box) or
/// `local_size` (every other sized node). Without this fold those attributes
/// were silently ignored for optional-size nodes and made required-size nodes
/// (`<Text>`, media, `<Instance>`) fail to reassemble with
/// "missing field `local_size`".
///
/// An explicit `clip_size`/`local_size` wins; the sugar only fills the gap.
/// `width`/`height` are always consumed so they don't linger as unknown
/// attributes. Print never emits `width`/`height`, so machine-written sources
/// are unaffected.
pub(crate) fn desugar_size(el: &mut FnxElement) {
    // A Frame's box is its clip size; most other tags size via `local_size`.
    // A Vector's `local_size` is its SVG-viewport CLIP, not its geometry —
    // folding there would crop the shape instead of resizing it, so Vector
    // keeps its attrs untouched (unknown fields are ignored downstream).
    let key = match el.tag.as_str() {
        "Frame" => Some("clip_size"),
        "Vector" => None,
        _ => Some("local_size"),
    };
    let width = el.attrs.get("width").and_then(Value::as_f64);
    let height = el.attrs.get("height").and_then(Value::as_f64);
    if let (Some(key), Some(width), Some(height)) = (key, width, height)
        && width.is_finite()
        && height.is_finite()
    {
        el.attrs.remove("width");
        el.attrs.remove("height");
        if !el.attrs.contains_key(key) {
            el.attrs.insert(key.to_owned(), json!([width, height]));
        }
    }
    for child in &mut el.children {
        desugar_size(child);
    }
}

// ---------------------------------------------------------------------------
// Shape sugar: <Rect> / <Ellipse>
// ---------------------------------------------------------------------------

/// The two authoring-sugar shapes ([`crate::model::SUGAR_TAGS`], one kind per
/// entry). Each kind owns its path generator and its inverse (implied-size
/// extraction); the print-side recognizer is defined as *regenerate and
/// compare*, so the generator is the single source of truth for what a
/// sugarable shape is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShapeKind {
    Rect,
    Ellipse,
}

/// The kappa constant, mirrored byte-for-byte from
/// `fanta_doc::PathData::ellipse`: the distance from a quadrant endpoint to
/// its tangent control point that yields a near-perfect circle approximation.
/// A dev-dependency drift test pins the generated JSON against the real
/// constructor, so a change on either side fails the build instead of
/// silently splitting the seam.
const KAPPA: f64 = 0.5522847498307933;

impl ShapeKind {
    fn for_tag(tag: &str) -> Option<Self> {
        match tag {
            "Rect" => Some(Self::Rect),
            "Ellipse" => Some(Self::Ellipse),
            _ => None,
        }
    }

    fn tag(self) -> &'static str {
        match self {
            Self::Rect => "Rect",
            Self::Ellipse => "Ellipse",
        }
    }

    /// The exact serde JSON `fanta_doc::PathData::rect(0.0, 0.0, w, h)` /
    /// `::ellipse(w/2, h/2, w/2, h/2)` serializes to. The arithmetic mirrors
    /// the doc constructors operation-for-operation (IEEE 754 is
    /// deterministic, so identical expressions give bit-identical floats), and
    /// the object shape mirrors `PathData`'s serde attributes: `fill_rule`
    /// (default) and `subpath_rules` (empty) are skipped, `PathSegment` is
    /// internally tagged as `op` in snake_case. This crate has no fanta-doc
    /// dependency by design; the drift tests in `tests.rs` pin byte equality
    /// against the real types via the dev-dependency.
    fn path_value(self, w: f64, h: f64) -> Value {
        match self {
            // Mirrors PathData::rect(x, y, w, h) with x = y = 0.0:
            // move → line → line → line → close, corners clockwise from the
            // top-left. `x + w` is written out (not just `w`) so even the
            // -0.0 edge case matches the doc's arithmetic exactly.
            Self::Rect => {
                let (x, y) = (0.0f64, 0.0f64);
                json!({
                    "segments": [
                        { "op": "move", "to": [x, y] },
                        { "op": "line", "to": [x + w, y] },
                        { "op": "line", "to": [x + w, y + h] },
                        { "op": "line", "to": [x, y + h] },
                        { "op": "close" },
                    ],
                })
            }
            // Mirrors PathData::ellipse(cx, cy, rx, ry) with cx = rx = w/2 and
            // cy = ry = h/2: four kappa cubics, one per quadrant, starting at
            // the left extreme (9 o'clock) and sweeping through the top.
            Self::Ellipse => {
                let (cx, cy) = (w / 2.0, h / 2.0);
                let (rx, ry) = (w / 2.0, h / 2.0);
                let (ox, oy) = (rx * KAPPA, ry * KAPPA);
                json!({
                    "segments": [
                        { "op": "move", "to": [cx - rx, cy] },
                        { "op": "cubic", "ctrl1": [cx - rx, cy - oy], "ctrl2": [cx - ox, cy - ry], "to": [cx, cy - ry] },
                        { "op": "cubic", "ctrl1": [cx + ox, cy - ry], "ctrl2": [cx + rx, cy - oy], "to": [cx + rx, cy] },
                        { "op": "cubic", "ctrl1": [cx + rx, cy + oy], "ctrl2": [cx + ox, cy + ry], "to": [cx, cy + ry] },
                        { "op": "cubic", "ctrl1": [cx - ox, cy + ry], "ctrl2": [cx - rx, cy + oy], "to": [cx - rx, cy] },
                        { "op": "close" },
                    ],
                })
            }
        }
    }

    /// The (w, h) a generator-produced path implies, read off segment
    /// endpoints that are exact by construction (rect: the third point IS
    /// `(w, h)`; ellipse: `cx + rx == w` and `cy + ry == h` — both are the
    /// doc's own arithmetic on the serialized values). Purely a CANDIDATE:
    /// the caller regenerates from it and compares, so a wrong guess on a
    /// non-generator path can never mis-sugar — it just fails the equality.
    fn implied_size(self, path: &Value) -> Option<(f64, f64)> {
        let segments = path.get("segments")?.as_array()?;
        let endpoint = |i: usize| -> Option<[f64; 2]> {
            let to = segments.get(i)?.get("to")?.as_array()?;
            match (to.first()?.as_f64(), to.get(1)?.as_f64()) {
                (Some(x), Some(y)) => Some([x, y]),
                _ => None,
            }
        };
        match self {
            Self::Rect => {
                let corner = endpoint(2)?;
                Some((corner[0], corner[1]))
            }
            Self::Ellipse => {
                let right = endpoint(2)?;
                let bottom = endpoint(3)?;
                Some((right[0], bottom[1]))
            }
        }
    }
}

/// Attributes a `Vector` may not carry if it is to re-print as shape sugar.
/// `parametric` / `fill_rule` / `subpath_rules` mark imported or parametric
/// vectors whose extra state the sugar spelling cannot express; `width` /
/// `height` ride along as unknown attrs on a Vector (size sugar deliberately
/// skips Vectors) and would be clobbered by the sugar's own `width`/`height`.
///
/// `local_size` is NOT here: it is handled by value in [`sugared_form`],
/// because a `.fig`-imported (or viewport-backfilled) rectangle carries one
/// and would otherwise degrade to raw path data on every reload.
const SUGAR_BLOCKING_ATTRS: &[&str] = &[
    "parametric",
    "fill_rule",
    "subpath_rules",
    "width",
    "height",
];

/// The sugared spelling of a canonical element, iff strictly lossless: a
/// childless `Vector`, none of [`SUGAR_BLOCKING_ATTRS`], and a `path` that is
/// `Value`-equal to what [`ShapeKind::path_value`] regenerates from the
/// implied size. Because recognize = regenerate + compare, an element either
/// re-sugars to a spelling that desugars back to the *identical* canonical
/// tree, or it stays a verbatim `<Vector path={…}>`.
fn sugared_form(el: &FnxElement, kind: ShapeKind) -> Option<FnxElement> {
    if el.tag != "Vector" || !el.children.is_empty() {
        return None;
    }
    if SUGAR_BLOCKING_ATTRS
        .iter()
        .any(|attr| el.attrs.contains_key(*attr))
    {
        return None;
    }
    let path = el.attrs.get("path")?;
    let (w, h) = kind.implied_size(path)?;
    if kind.path_value(w, h) != *path {
        return None;
    }
    // A Vector's `local_size` is its SVG viewport: rendering is clipped to
    // `[0, 0, w, h]`, so a box that differs from the generated shape's own
    // extent CROPS it (a thick stroke is the usual victim) and no `<Rect>`
    // spelling can express that — those stay canonical. A box that is exactly
    // the shape's extent crops nothing, and it rides along verbatim as its own
    // attribute (`desugar_shapes` never touches `local_size`), so the sugared
    // and canonical spellings still desugar to bit-identical trees. Without
    // this, every `.fig`-imported rectangle — and every rectangle whose
    // viewport the doc backfilled on load — degraded to raw path data on the
    // first reload and produced a spurious diff on the next save.
    if el
        .attrs
        .get("local_size")
        .is_some_and(|size| *size != json!([w, h]))
    {
        return None;
    }
    let mut sugared = el.clone();
    sugared.tag = kind.tag().to_owned();
    sugared.attrs.remove("path");
    sugared.attrs.insert("width".to_owned(), json!(w));
    sugared.attrs.insert("height".to_owned(), json!(h));
    Some(sugared)
}

/// The sugared spelling of `el` under the specific sugar tag an author wrote
/// (`"Rect"` / `"Ellipse"`), or `None` when `tag` is not a sugar tag or the
/// element no longer qualifies. The source mirror uses this to keep an
/// authored `<Rect …>` spelling across canvas edits — and to detect the
/// moment the shape stops matching its generator, at which point the mirror
/// falls back to a canonical `<Vector …/>` reprint of that one tag.
pub(crate) fn shape_sugar_spelling(el: &FnxElement, tag: &str) -> Option<FnxElement> {
    sugared_form(el, ShapeKind::for_tag(tag)?)
}

/// Parse-time shape sugar: rewrite each `<Rect>`/`<Ellipse>` into its
/// canonical `<Vector path={…}>`, in place, over the whole subtree. MUST run
/// before [`desugar_size`]: it consumes `width`/`height` here, and a sugar
/// tag is neither `Frame` nor `Vector`, so desugar_size would otherwise fold
/// those attributes into a bogus `local_size`.
pub(crate) fn desugar_shapes(el: &mut FnxElement) -> Result<(), FnxError> {
    if let Some(kind) = ShapeKind::for_tag(&el.tag) {
        // An explicit `path` is contradictory, not mergeable: the sugar tag's
        // whole meaning is "the path is generated". Erroring (instead of
        // preferring either side) keeps the file unambiguous.
        if el.attrs.contains_key("path") {
            return Err(FnxError::Parse(format!(
                "{} generates its own path; use `<Vector>` for explicit path data",
                element_ref(el)
            )));
        }
        let width = el.attrs.get("width").and_then(Value::as_f64);
        let height = el.attrs.get("height").and_then(Value::as_f64);
        let (Some(width), Some(height)) = (width, height) else {
            return Err(FnxError::Parse(format!(
                "{} requires numeric `width` and `height` attributes",
                element_ref(el)
            )));
        };
        if !width.is_finite() || !height.is_finite() {
            return Err(FnxError::Parse(format!(
                "{} requires finite `width` and `height` attributes",
                element_ref(el)
            )));
        }
        el.attrs.remove("width");
        el.attrs.remove("height");
        el.attrs
            .insert("path".to_owned(), kind.path_value(width, height));
        // NOT setting `local_size`: on a Vector that field is the SVG-viewport
        // clip, not the geometry box — setting it would crop, not size. An
        // explicit one the author (or the printer, re-spelling a clipped
        // rectangle) wrote is left exactly as it is.
        el.tag = "Vector".to_owned();
    }
    for child in &mut el.children {
        desugar_shapes(child)?;
    }
    Ok(())
}

/// Print-time shape sugar: re-spell qualifying `Vector`s as `<Rect>`/
/// `<Ellipse>`, in place, over the whole subtree. Applied to a clone just
/// before printing (like [`sugar_transform`]). Rect is tried first; the
/// generators emit disjoint segment kinds (lines vs cubics), so at most one
/// kind can ever match.
pub(crate) fn sugar_shapes(el: &mut FnxElement) {
    for kind in [ShapeKind::Rect, ShapeKind::Ellipse] {
        if let Some(sugared) = sugared_form(el, kind) {
            *el = sugared;
            break;
        }
    }
    for child in &mut el.children {
        sugar_shapes(child);
    }
}

/// A human-readable element reference for post-parse errors. Desugaring runs
/// on the parsed tree, so char positions are gone; tag + `name` is the most
/// precise address left, and it is how authors identify layers anyway.
fn element_ref(el: &FnxElement) -> String {
    match el.attrs.get("name").and_then(Value::as_str) {
        Some(name) => format!("`<{} name={name:?}>`", el.tag),
        None => format!("`<{}>`", el.tag),
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
