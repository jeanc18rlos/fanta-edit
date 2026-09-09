//! Low-level field readers shared across the mapping passes: geometry, fills,
//! gradients, strokes-by-corner, colors, effects, blurs, transforms, and the
//! shared-style reference pre-pass.

use super::{
    AssetId, BlendMode, Blur, BlurKind, Color, Fill, Gradient, GradientStop, HashMap, ImageAdjust,
    ImageFitMode, KiwiValue, ParametricShape, PathData, Shadow, ShadowKind, Transform2D,
    VectorNode, build_stroke, stable_hash_u128,
};
use std::borrow::Cow;

/// Resolve a Figma `Number { value, units }` to an absolute pixel amount.
pub(crate) fn read_number_px(number: Option<&KiwiValue>, font_px: f64) -> Option<f64> {
    let number = number?;
    let value = number.get("value").and_then(KiwiValue::as_f64)?;
    match number.get("units").and_then(KiwiValue::as_str) {
        Some("PIXELS") => Some(value),
        Some("PERCENT") => Some(value / 100.0 * font_px),
        _ => None,
    }
}

/// A decoded Figma `lineHeight`: either a plain multiple of the font size, or
/// a percentage of the FONT'S INTRINSIC line height (Kiwi PERCENT units — 100
/// is Figma's "auto").
pub(crate) enum LineHeight {
    /// A unitless multiple of `size_px` (Kiwi RAW, or PIXELS ÷ font size).
    Multiple(f64),
    /// Percent of the font's intrinsic (metric) line height. `100.0` = auto.
    IntrinsicPercent(f64),
}

/// Resolve a Figma `lineHeight` `Number`.
///
/// - `PIXELS` → a multiple of the font size (`value / font_px`).
/// - `RAW` → already a unitless multiple of the font size.
/// - `PERCENT` → percent of the font's INTRINSIC line height, NOT of the font
///   size: `100` is exactly Figma's "auto" (ascent+descent metrics, ~1.21× the
///   size for Inter). Verified across the corpus — every default-line-height
///   text node stores `PERCENT 100`.
pub(crate) fn read_line_height(number: Option<&KiwiValue>, font_px: f64) -> Option<LineHeight> {
    let number = number?;
    let value = number.get("value").and_then(KiwiValue::as_f64)?;
    if font_px <= 0.0 {
        return None;
    }
    match number.get("units").and_then(KiwiValue::as_str) {
        Some("PIXELS") => Some(LineHeight::Multiple(value / font_px)),
        Some("PERCENT") => Some(LineHeight::IntrinsicPercent(value)),
        Some("RAW") => Some(LineHeight::Multiple(value)),
        _ => None,
    }
}

/// A rectangle [`VectorNode`] sized `(w, h)` at the local origin, carrying the
/// node's full fill stack and any imported stroke. Used for RECTANGLE /
/// ROUNDED_RECTANGLE and the geometry-decode bbox fallback.
pub(crate) fn make_shape(
    size: (f64, f64),
    fills: &[Fill],
    change: &KiwiValue,
    corner_radius: Option<f64>,
    corner_radii: Option<[f64; 4]>,
) -> VectorNode {
    VectorNode {
        path: PathData::rect(0.0, 0.0, size.0, size.1),
        fills: fills.iter().cloned().collect(),
        strokes: build_stroke(change).into_iter().collect(),
        corner_radius,
        corner_radii,
        corner_smoothing: corner_smoothing(change),
        local_size: viewport(size),
        parametric: None,
    }
}

/// Read `cornerSmoothing` (Figma's iOS-squircle amount, 0..=1). `0.0` when
/// absent or out of range.
pub(crate) fn corner_smoothing(change: &KiwiValue) -> f32 {
    change
        .get("cornerSmoothing")
        .and_then(KiwiValue::as_f64)
        .filter(|s| s.is_finite() && *s > 0.0)
        .map(|s| s.clamp(0.0, 1.0) as f32)
        .unwrap_or(0.0)
}

/// A filled rectangle [`VectorNode`] (single optional fill, no stroke). Kept for
/// the text fallback case where there's no usable `change` shape to stroke.
pub(crate) fn make_rect(
    size: (f64, f64),
    fills: Option<Fill>,
    corner_radius: Option<f64>,
) -> VectorNode {
    VectorNode {
        path: PathData::rect(0.0, 0.0, size.0, size.1),
        fills: fills.into_iter().collect(),
        strokes: Default::default(),
        corner_radius,
        corner_radii: None,
        corner_smoothing: 0.0,
        local_size: viewport(size),
        parametric: None,
    }
}

/// Read a rectangle's corner rounding into `(uniform, per_corner)`:
///
/// - **`per_corner`** `[TL, TR, BR, BL]` is `Some` only when the four corners are
///   not all equal (a mixed-corner card/tab/segmented control). It takes render
///   precedence and is read from `rectangleTopLeftCornerRadius` etc. (active when
///   `rectangleCornerRadiiIndependent` is set, but we also honor the fields when
///   they simply differ, matching op2 `mapCornerRadius`).
/// - **`uniform`** is the single radius for the all-equal case: the explicit
///   `cornerRadius`, else the common per-corner value when all four agree.
///
/// Returns `(None, None)` when the shape is square. Mirrors op2's
/// `converters/common.ts mapCornerRadius`.
pub(crate) fn corner_radii(change: &KiwiValue) -> (Option<f64>, Option<[f64; 4]>) {
    let corner = |k: &str| change.get(k).and_then(KiwiValue::as_f64).unwrap_or(0.0);
    let independent = matches!(
        change.get("rectangleCornerRadiiIndependent"),
        Some(KiwiValue::Bool(true))
    );
    let has_per_corner = independent
        || [
            "rectangleTopLeftCornerRadius",
            "rectangleTopRightCornerRadius",
            "rectangleBottomRightCornerRadius",
            "rectangleBottomLeftCornerRadius",
        ]
        .iter()
        .any(|k| change.get(k).and_then(KiwiValue::as_f64).is_some());

    if has_per_corner {
        let tl = corner("rectangleTopLeftCornerRadius");
        let tr = corner("rectangleTopRightCornerRadius");
        let br = corner("rectangleBottomRightCornerRadius");
        let bl = corner("rectangleBottomLeftCornerRadius");
        if (tl - tr).abs() < f64::EPSILON
            && (tr - br).abs() < f64::EPSILON
            && (br - bl).abs() < f64::EPSILON
        {
            // All four equal → collapse to the uniform value (or square).
            return ((tl > 0.0).then_some(tl), None);
        }
        return (None, Some([tl, tr, br, bl]));
    }
    if let Some(r) = change.get("cornerRadius").and_then(KiwiValue::as_f64) {
        if r > 0.0 {
            return (Some(r), None);
        }
    }
    (None, None)
}

/// Whether a node carries independent (non-uniform) per-corner radii — used for
/// the [`MapReport`] counter.
pub(crate) fn has_independent_corners(change: &KiwiValue) -> bool {
    corner_radii(change).1.is_some()
}

/// Build the real arc/pie/donut/ring [`PathData`] for an ELLIPSE that carries a
/// non-default `arcData`, or `None` when the ellipse is a plain full disc (no
/// `arcData`, a ≈360° sweep, AND no inner radius) — in which case the caller
/// keeps the full [`PathData::ellipse`].
///
/// Figma's `arcData`:
/// - `startingAngle` / `endingAngle` — radians, 0 = +x (3 o'clock), increasing
///   clockwise (screen y-down). The signed sweep is `endingAngle − startingAngle`.
/// - `innerRadius` — `0..=1` ratio of the outer radius (`0` ⇒ a solid pie,
///   `> 0` ⇒ a donut/ring with a hole).
///
/// Mirrors op2 `shape-converter.ts` `mapFigmaArcData` + `arc-path.ts`
/// `isArcEllipse`, here tessellated into our cubic-only `PathData` rather than an
/// SVG `A` command (we have no arc segment). `size` is the node's box `(w, h)`;
/// the outer radii are `w/2`, `h/2`.
pub(crate) fn arc_ellipse_path(change: &KiwiValue, size: (f64, f64)) -> Option<PathData> {
    Some(read_arc_shape(change)?.to_path(size.0, size.1))
}

/// Read a non-trivial `arcData` as a scrubable [`ParametricShape::Arc`], or
/// `None` for a plain full disc (no `arcData`, a ≈360° sweep, AND no inner
/// radius). [`arc_ellipse_path`] tessellates whatever this returns, so the two
/// share one definition of "is this a real arc" and can't drift.
pub(crate) fn read_arc_shape(change: &KiwiValue) -> Option<ParametricShape> {
    let arc = change.get("arcData")?;
    let start = arc
        .get("startingAngle")
        .and_then(KiwiValue::as_f64)
        .unwrap_or(0.0);
    let end = arc
        .get("endingAngle")
        .and_then(KiwiValue::as_f64)
        .unwrap_or(std::f64::consts::TAU);
    let inner = arc
        .get("innerRadius")
        .and_then(KiwiValue::as_f64)
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let sweep = end - start;
    // A near-full sweep with no hole is just the plain ellipse — not parametric.
    let near_full = sweep.abs() >= std::f64::consts::TAU - 1e-3;
    if near_full && inner <= 1e-3 {
        return None;
    }
    Some(ParametricShape::Arc {
        start_rad: start,
        sweep_rad: sweep,
        inner_ratio: inner,
    })
}

/// A stable string key for a Figma `GUID` value (`sessionID:localID`).
pub(crate) fn guid_key(guid: &KiwiValue) -> Option<String> {
    let session = guid.get("sessionID").and_then(KiwiValue::as_f64)? as u64;
    let local = guid.get("localID").and_then(KiwiValue::as_f64)? as u64;
    Some(format!("{session}:{local}"))
}

/// A vector's SVG-style viewport box for [`VectorNode::local_size`]: the node's
/// declared `size`, used at render time to clip geometry that spills past the box
/// (chiefly a stroke thickened beyond the authored size). `None` for a degenerate
/// (zero width or height) box — a LINE reports one, and clipping to it would erase
/// the whole stroke outline.
pub(crate) fn viewport(size: (f64, f64)) -> Option<[f64; 2]> {
    (size.0 > 0.0 && size.1 > 0.0).then_some([size.0, size.1])
}

/// Read the `size` (a Figma `Vector {x, y}`), defaulting to a 1x1 box when a
/// component is absent OR non-finite (a NaN/Infinity size seen in the wild in
/// a handful of degenerate nodes would otherwise bake into the bbox-fallback
/// path and `clip_size`, which the project-tree writer then serializes as
/// JSON `null` — failing to deserialize back into the plain `f64`/`[f64; 2]`
/// the schema expects).
pub(crate) fn read_size(change: &KiwiValue) -> (f64, f64) {
    let v = change.get("size");
    let component = |key: &str| {
        v.and_then(|v| v.get(key))
            .and_then(KiwiValue::as_f64)
            .filter(|value| value.is_finite())
            .unwrap_or(1.0)
    };
    (component("x"), component("y"))
}

/// Read the `transform` (a Figma `Matrix`) into a [`Transform2D`]. A
/// non-finite component (seen in the wild in a handful of degenerate nodes)
/// falls back to identity rather than baking a NaN/Infinity into the doc: the
/// project-tree writer serializes those as JSON `null`, which then fails to
/// deserialize back into the plain `f64` the schema expects.
pub(crate) fn read_transform(change: &KiwiValue) -> Transform2D {
    let Some(m) = change.get("transform") else {
        return Transform2D::IDENTITY;
    };
    let g = |k: &str, default: f64| m.get(k).and_then(KiwiValue::as_f64).unwrap_or(default);
    let (m00, m01, m02) = (g("m00", 1.0), g("m01", 0.0), g("m02", 0.0));
    let (m10, m11, m12) = (g("m10", 0.0), g("m11", 1.0), g("m12", 0.0));
    let transform = Transform2D::from_components([m00, m10, m01, m11, m02, m12]);
    if transform.is_finite() {
        transform
    } else {
        Transform2D::IDENTITY
    }
}

/// Read every visible paint from `fillPaints` into a stacked [`Fill`] list,
/// bottom-first (Figma's paint order). Each paint becomes a `Fill::Solid`,
/// `Fill::Gradient`, or `Fill::Image` placeholder; per-paint `opacity` multiplies
/// the color/stop alpha. Invisible paints and types we can't decode are skipped.
/// Coverage tallies produced by [`resolve_style_references`], surfaced on the
/// [`MapReport`] for the fixture test / measurement output.
#[derive(Default)]
pub(crate) struct StyleResolveReport {
    /// Count of NodeChanges that are shared-style definitions (`styleType` set).
    pub(crate) style_def_count: usize,
    /// Count of consumers (nodes + override entries) carrying a `styleIdForFill`
    /// while their own `fillPaints` was empty/absent.
    pub(crate) ref_empty_fill: usize,
    /// Of those, how many we resolved to a real paint set via the style map.
    pub(crate) resolved_fill: usize,
}

/// Whether a change/override carries a non-empty `fillPaints` array.
pub(crate) fn has_nonempty_fills(change: &KiwiValue) -> bool {
    change
        .get("fillPaints")
        .and_then(KiwiValue::as_array)
        .map(|a| !a.is_empty())
        .unwrap_or(false)
}

/// Whether a change carries a non-empty array at `field`.
pub(crate) fn has_nonempty_array(change: &KiwiValue, field: &str) -> bool {
    change
        .get(field)
        .and_then(KiwiValue::as_array)
        .map(|a| !a.is_empty())
        .unwrap_or(false)
}

/// Resolve the guid a `styleIdFor*` ref points at: `{ guid: { sessionID, localID } }`.
pub(crate) fn style_ref_guid(change: &KiwiValue, field: &str) -> Option<String> {
    change
        .get(field)
        .and_then(|r| r.get("guid"))
        .and_then(guid_key)
}

/// The shared-style definitions of a document, keyed by guid string. Borrows
/// the ORIGINAL (unresolved) changes: a style def that itself carries a style
/// ref is never chased, matching op2.
pub(crate) type StyleMap<'a> = HashMap<String, &'a KiwiValue>;

/// **Shared-style reference resolution** — mirrors op2's `resolveStyleReferences`
/// (figma-node-mapper.ts:25-90).
///
/// Figma stores shared styles as standalone `NodeChange`s carrying a `styleType`
/// (`FILL` / `TEXT` / `EFFECT`) plus the actual `fillPaints` / `effects` / font
/// fields. A *consuming* node carries only a `styleIdFor{Fill,StrokeFill,Text,
/// Effect}` ref (a `{guid}`) and frequently an EMPTY `fillPaints` array — so the
/// naive reader sees no fill and the dark-theme frame renders as its light
/// master. Here we build a guid → style-change map, then for every consuming node
/// (and every `symbolData.symbolOverrides[]` entry) inline the referenced style's
/// payload into the consumer's own fields, so all the downstream readers
/// (`read_fills`, `build_stroke`, `read_effects`, `build_text`) transparently see
/// real values. Rich text runs have their own mini `NodeChange`s in
/// `textData.styleOverrideTable`, so we resolve those nested refs too.
///
/// Returns the changes as a copy-on-write view over `changes`: a change that
/// carries no `styleIdFor*` ref anywhere the resolvers look is handed back
/// borrowed, and only the consumers are cloned and rewritten. The resolvers
/// never mutate (or count) a change without such a ref, so the borrowed
/// entries are exactly the ones they would have left untouched — and the
/// document is no longer deep-copied wholesale for the handful of styled nodes.
pub(crate) fn resolve_style_references(
    changes: &[KiwiValue],
) -> (Vec<Cow<'_, KiwiValue>>, StyleResolveReport) {
    let mut report = StyleResolveReport::default();

    let mut style_map: StyleMap<'_> = HashMap::new();
    for nc in changes {
        if nc.get("styleType").is_some() {
            report.style_def_count += 1;
            if let Some(guid) = nc.get("guid").and_then(guid_key) {
                style_map.insert(guid, nc);
            }
        }
    }
    let mut resolved: Vec<Cow<'_, KiwiValue>> = changes.iter().map(Cow::Borrowed).collect();
    if style_map.is_empty() {
        return (resolved, report);
    }

    for slot in &mut resolved {
        if !change_carries_style_ref(slot) {
            continue;
        }
        let nc = slot.to_mut();
        resolve_style_on(nc, &style_map, &mut report);

        // Override entries (figma-node-mapper.ts:82-89): dark-theme instances
        // point children at dark FILL styles via per-override `styleIdForFill`.
        // We can't borrow `nc` immutably and mutably at once, so collect the
        // override-array length first, then index in.
        let n_overrides = nc
            .get("symbolData")
            .and_then(|sd| sd.get("symbolOverrides"))
            .and_then(KiwiValue::as_array)
            .map(|a| a.len())
            .unwrap_or(0);
        for i in 0..n_overrides {
            if let Some(ov) = nc
                .get_mut("symbolData")
                .and_then(|sd| sd.get_mut("symbolOverrides"))
                .and_then(|a| match a {
                    KiwiValue::Array(v) => v.get_mut(i),
                    _ => None,
                })
            {
                resolve_style_on_symbol_override(ov, &style_map, &mut report);
            }
        }
    }

    (resolved, report)
}

/// The `styleIdFor*` fields the resolvers act on. Every resolver returns
/// before touching or counting anything unless its ref field is present, so
/// "carries one of these" is the exact precondition for a change needing an
/// owned copy.
const STYLE_REF_FIELDS: [&str; 4] = [
    "styleIdForFill",
    "styleIdForStrokeFill",
    "styleIdForText",
    "styleIdForEffect",
];

/// Whether `resolve_style_references` would rewrite `nc`: a style ref on the
/// change itself, on one of its rich-text run entries, or on one of its
/// `symbolData.symbolOverrides[]` entries (including THEIR run entries). Mirrors
/// the exact shape the resolvers walk.
pub(crate) fn change_carries_style_ref(nc: &KiwiValue) -> bool {
    if consumer_carries_style_ref(nc) {
        return true;
    }
    nc.get("symbolData")
        .and_then(|sd| sd.get("symbolOverrides"))
        .and_then(KiwiValue::as_array)
        .is_some_and(|overrides| overrides.iter().any(consumer_carries_style_ref))
}

/// The per-consumer half of [`change_carries_style_ref`]: what
/// [`resolve_style_on`] / [`resolve_style_on_symbol_override`] look at for one
/// node or override entry — its own ref fields plus its run tables.
fn consumer_carries_style_ref(nc: &KiwiValue) -> bool {
    if STYLE_REF_FIELDS.iter().any(|field| nc.get(field).is_some()) {
        return true;
    }
    ["textData", "derivedTextData"].iter().any(|field| {
        nc.get(field)
            .and_then(|td| td.get("styleOverrideTable"))
            .and_then(KiwiValue::as_array)
            .is_some_and(|table| table.iter().any(consumer_carries_style_ref))
    })
}

/// Inline any referenced style's payload into a single consumer (`nc`), only for
/// the channels the consumer doesn't already populate inline. `report` tallies
/// the fill-resolution coverage.
pub(crate) fn resolve_style_on(
    nc: &mut KiwiValue,
    style_map: &StyleMap<'_>,
    report: &mut StyleResolveReport,
) {
    resolve_fill_style(nc, style_map, report);
    resolve_stroke_fill_style(nc, style_map);
    resolve_text_style(nc, style_map, true);
    resolve_effect_style(nc, style_map);
    resolve_rich_text_run_styles(nc, style_map, report);
}

pub(crate) fn resolve_style_on_symbol_override(
    nc: &mut KiwiValue,
    style_map: &StyleMap<'_>,
    report: &mut StyleResolveReport,
) {
    resolve_fill_style(nc, style_map, report);
    resolve_stroke_fill_style(nc, style_map);
    resolve_text_style(nc, style_map, false);
    resolve_effect_style(nc, style_map);
    resolve_rich_text_run_styles(nc, style_map, report);
}

/// ---- FILL ----
/// op2 `resolveStyleReferences` (figma-node-mapper.ts:44-48) overwrites the
/// consumer's `fillPaints` with the referenced FILL style's paints
/// UNCONDITIONALLY whenever the style has paints — it does NOT preserve the
/// node's own inline `fillPaints`. This is load-bearing for theme fidelity: a
/// dark-theme card `_Header` carries its *master/light default* baked fill
/// (white `#FFFFFF`) inline while its `styleIdForFill` points at the per-page
/// dark FILL style (`darkest/gray/gray-100` → `#1D1D1D`). The style is the
/// authoritative per-page color, so it MUST win over the stale inline paint;
/// resolving only when the inline fill was empty (the old 3a48692 behavior)
/// left the Darkest `_Header` painted white. (On the Light page the same node
/// resolves to `light/gray/gray-50` → `#FFFFFF`, matching its inline paint, so
/// the overwrite is a no-op there — the fix stays per-page correct.)
fn resolve_fill_style(
    nc: &mut KiwiValue,
    style_map: &StyleMap<'_>,
    report: &mut StyleResolveReport,
) {
    let Some(guid) = style_ref_guid(nc, "styleIdForFill") else {
        return;
    };
    let had_empty = !has_nonempty_fills(nc);
    if had_empty {
        report.ref_empty_fill += 1;
    }
    let Some(paints) = style_map
        .get(&guid)
        .filter(|style| has_nonempty_fills(style))
        .and_then(|style| style.get("fillPaints"))
    else {
        return;
    };
    nc.set_field("fillPaints", paints.clone());
    if had_empty {
        report.resolved_fill += 1;
    }
}

/// ---- STROKE FILL ----  (the style's `fillPaints` becomes the stroke paint)
/// Same unconditional-overwrite semantics as FILL (op2 lines 51-54): the
/// referenced stroke style's paints win over the node's own `strokePaints`.
fn resolve_stroke_fill_style(nc: &mut KiwiValue, style_map: &StyleMap<'_>) {
    let Some(guid) = style_ref_guid(nc, "styleIdForStrokeFill") else {
        return;
    };
    let Some(paints) = style_map
        .get(&guid)
        .filter(|style| has_nonempty_fills(style))
        .and_then(|style| style.get("fillPaints"))
    else {
        return;
    };
    nc.set_field("strokePaints", paints.clone());
}

/// ---- TEXT ----  font fields + the text color (style's fillPaints)
fn resolve_text_style(nc: &mut KiwiValue, style_map: &StyleMap<'_>, copy_style_fills: bool) {
    let Some(guid) = style_ref_guid(nc, "styleIdForText") else {
        return;
    };
    let Some(&style) = style_map.get(&guid) else {
        return;
    };
    for field in [
        "fontName",
        "fontSize",
        "lineHeight",
        "letterSpacing",
        "textAlignHorizontal",
        "textCase",
        "textDecoration",
    ] {
        if nc.get(field).is_none() {
            if let Some(v) = style.get(field) {
                nc.set_field(field, v.clone());
            }
        }
    }
    // A TEXT style may also carry the glyph color via its own fillPaints. Symbol
    // override entries are different: Spectrum uses `styleIdForText` there to
    // swap typography while leaving the descendant's authored/theme fill intact.
    if copy_style_fills && !has_nonempty_fills(nc) && has_nonempty_fills(style) {
        if let Some(paints) = style.get("fillPaints") {
            nc.set_field("fillPaints", paints.clone());
        }
    }
}

/// ---- EFFECT ----
fn resolve_effect_style(nc: &mut KiwiValue, style_map: &StyleMap<'_>) {
    if has_nonempty_array(nc, "effects") {
        return;
    }
    let Some(guid) = style_ref_guid(nc, "styleIdForEffect") else {
        return;
    };
    let Some(effects) = style_map
        .get(&guid)
        .filter(|style| has_nonempty_array(style, "effects"))
        .and_then(|style| style.get("effects"))
    else {
        return;
    };
    nc.set_field("effects", effects.clone());
}

/// ---- RICH TEXT RUNS ----
/// Figma stores per-range text styles as compact NodeChange entries under
/// textData.styleOverrideTable. Link/bold spans in the Spectrum introduction
/// carry their own `styleIdForFill`; without resolving those nested refs the
/// range imports with the right font metadata but the wrong glyph color.
fn resolve_rich_text_run_styles(
    nc: &mut KiwiValue,
    style_map: &StyleMap<'_>,
    report: &mut StyleResolveReport,
) {
    for field in ["textData", "derivedTextData"] {
        let n_styles = nc
            .get(field)
            .and_then(|td| td.get("styleOverrideTable"))
            .and_then(KiwiValue::as_array)
            .map(|a| a.len())
            .unwrap_or(0);
        for i in 0..n_styles {
            if let Some(style) = nc
                .get_mut(field)
                .and_then(|td| td.get_mut("styleOverrideTable"))
                .and_then(|a| match a {
                    KiwiValue::Array(v) => v.get_mut(i),
                    _ => None,
                })
            {
                resolve_style_on(style, style_map, report);
            }
        }
    }
}

/// Read every visible paint from `fillPaints` (falling back to `backgroundPaints`
/// when `fillPaints` is empty/absent — op2: `mapFigmaFills(fillPaints) ??
/// mapFigmaFills(backgroundPaints)`) into a stacked [`Fill`] list.
pub(crate) fn read_fills(change: &KiwiValue) -> Vec<Fill> {
    let mut out = Vec::new();
    let paints = match change.get("fillPaints").and_then(KiwiValue::as_array) {
        Some(p) if !p.is_empty() => p,
        // CANVAS / FRAME / SECTION backgrounds live under `backgroundPaints`.
        _ => match change.get("backgroundPaints").and_then(KiwiValue::as_array) {
            Some(p) => p,
            None => return out,
        },
    };
    for paint in paints {
        if let Some(fill) = read_paint(paint) {
            out.push(fill);
        }
    }
    out
}

/// The first fill of a stack (the bottom-most paint), for single-fill contexts
/// (a frame background, a text glyph color).
pub(crate) fn first_fill(fills: &[Fill]) -> Option<Fill> {
    fills.first().cloned()
}

/// First visible paint from an arbitrary paint array (e.g. `strokePaints`),
/// solid or gradient. Used to paint a stroke.
pub(crate) fn first_paint_fill(paints: Option<&KiwiValue>) -> Option<Fill> {
    let arr = paints.and_then(KiwiValue::as_array)?;
    arr.iter().find_map(read_paint)
}

/// Convert one Figma `Paint` into a doc [`Fill`]. Recognizes SOLID, the three
/// gradient kinds, and IMAGE (placeholder). Returns `None` for an invisible
/// paint or an unrecognized/empty one.
pub(crate) fn read_paint(paint: &KiwiValue) -> Option<Fill> {
    let visible = paint
        .get("visible")
        .map(|v| matches!(v, KiwiValue::Bool(true)))
        .unwrap_or(true);
    if !visible {
        return None;
    }
    let opacity = paint
        .get("opacity")
        .and_then(KiwiValue::as_f64)
        .unwrap_or(1.0);
    match paint.get("type").and_then(KiwiValue::as_str) {
        Some("SOLID") => {
            let color = paint.get("color").and_then(|c| read_color(c, paint))?;
            Some(Fill::Solid {
                color,
                blend: paint_blend_mode(paint),
            })
        }
        Some(
            t @ ("GRADIENT_LINEAR" | "GRADIENT_RADIAL" | "GRADIENT_ANGULAR" | "GRADIENT_DIAMOND"),
        ) => read_gradient(t, paint, opacity).map(|gradient| Fill::Gradient {
            gradient,
            blend: paint_blend_mode(paint),
        }),
        Some("IMAGE") => {
            // An IMAGE paint references its bitmap by `image.hash` (a sha1 the
            // `.fig` ZIP stores under `images/<hex>`). Mint a *deterministic*
            // `AssetId` from that hash so the same bitmap always keys the same
            // asset (re-import stable, dedup across paints), then emit a real
            // image fill. The bytes are wired into the resolver separately by
            // [`collect_image_assets`]; the renderer falls back to its own
            // placeholder if the asset can't be resolved/decoded, so a paint we
            // can't back with bytes still degrades gracefully (never panics).
            let hash = image_hash_hex(paint)?;
            let asset = asset_id_for_image(&hash);
            let mode = image_fit_mode(paint);
            // `imageTransform` is only *applicable* in Figma's CROP mode (Kiwi
            // `STRETCH`); FILL/FIT/TILE paints keep a stale transform around
            // after the user switches modes, which Figma preserves but ignores
            // — reading it there wrongly cropped those paints.
            let crop = is_crop_scale_mode(paint)
                .then(|| image_crop(paint))
                .flatten();
            // `scale` (plugin `scalingFactor`) is likewise only applicable in
            // TILE mode: each tile draws at natural-size × scale.
            let scale = (mode == ImageFitMode::Tile)
                .then(|| paint.get("scale").and_then(KiwiValue::as_f64))
                .flatten()
                .filter(|s| s.is_finite() && *s > 0.0 && (*s - 1.0).abs() > 1e-6)
                .map(|s| s as f32);
            let rotation = paint
                .get("rotation")
                .and_then(KiwiValue::as_f64)
                .filter(|r| r.is_finite() && r.abs() > 1e-6)
                .map(|r| r as f32);
            Some(Fill::Image {
                asset,
                mode,
                opacity: opacity.clamp(0.0, 1.0) as f32,
                crop: crop.map(Box::new),
                scale,
                rotation,
                blend: paint_blend_mode(paint),
                adjust: read_image_adjust(paint),
            })
        }
        _ => None,
    }
}

/// Read Figma's per-image-paint adjustment `filters` (exposure / contrast /
/// saturation / temperature / tint / highlights / shadows). Absent filters —
/// or unrecognized field names — yield a no-op [`ImageAdjust`], so this is
/// additive: worst case an adjustment is simply not imported, never wrong.
fn read_image_adjust(paint: &KiwiValue) -> ImageAdjust {
    let Some(filters) = paint.get("filters") else {
        return ImageAdjust::default();
    };
    let read = |key: &str| {
        filters
            .get(key)
            .and_then(KiwiValue::as_f64)
            .filter(|v| v.is_finite())
            .unwrap_or(0.0) as f32
    };
    ImageAdjust {
        exposure: read("exposure"),
        contrast: read("contrast"),
        saturation: read("saturation"),
        temperature: read("temperature"),
        tint: read("tint"),
        highlights: read("highlights"),
        shadows: read("shadows"),
    }
}

/// Read a paint's own `blendMode` (per-paint compositing against the paints
/// below it in the stack). Absent / unrecognized / PASS_THROUGH → `Normal`.
pub(crate) fn paint_blend_mode(paint: &KiwiValue) -> BlendMode {
    paint
        .get("blendMode")
        .and_then(KiwiValue::as_str)
        .and_then(blend_mode)
        .unwrap_or(BlendMode::Normal)
}

/// Whether an IMAGE paint is in Figma's plugin-API **CROP** mode, which the
/// Kiwi schema spells `STRETCH` (the plugin API has no STRETCH member). Only in
/// this mode is `imageTransform` applicable. `CROP` is accepted too for schema
/// variants that spell it out.
fn is_crop_scale_mode(paint: &KiwiValue) -> bool {
    matches!(
        paint.get("imageScaleMode").and_then(KiwiValue::as_str),
        Some("STRETCH" | "CROP")
    )
}

/// Hex-encode an IMAGE paint's `image.hash` (a Kiwi `byte[]`), which is the key
/// the `.fig` ZIP uses for `images/<hex>`. `None` when the paint carries no
/// image / hash (a malformed or thumbnail-only paint), so the caller skips it
/// rather than minting a dangling asset.
pub(crate) fn image_hash_hex(paint: &KiwiValue) -> Option<String> {
    let bytes = paint
        .get("image")
        .and_then(|img| img.get("hash"))
        .and_then(KiwiValue::as_bytes)?;
    if bytes.is_empty() {
        return None;
    }
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes.iter() {
        hex.push(HEX_DIGITS[usize::from(byte >> 4)]);
        hex.push(HEX_DIGITS[usize::from(byte & 0x0f)]);
    }
    Some(hex)
}

const HEX_DIGITS: [char; 16] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f',
];

/// Deterministic [`AssetId`] for an image hash hex string. Stable across runs
/// and across re-imports (it's a pure function of the hash), so two paints that
/// reference the same bitmap share one asset and the resolver caches one decode.
pub(crate) fn asset_id_for_image(hash_hex: &str) -> AssetId {
    // Namespace the hash so an image asset can never collide with a variable id
    // (which hashes a bare guid through the same `stable_hash_u128`).
    AssetId::from_u128(stable_hash_u128(&format!("fig-image:{hash_hex}")))
}

/// Map Figma's `imageScaleMode` enum to the doc's [`ImageFitMode`].
///
/// `FIT`/`TILE`/`STRETCH` map directly. `CROP` maps to [`ImageFitMode::Fill`]:
/// the crop window is carried separately by [`image_crop`] (Figma's
/// `imageTransform`), and the cropped sub-rect is then *filled* into the node —
/// which, because `imageTransform` already encodes the pan/zoom, reproduces
/// Figma's crop. Absent/unknown defaults to `Fill`, matching Figma's default.
pub(crate) fn image_fit_mode(paint: &KiwiValue) -> ImageFitMode {
    match paint.get("imageScaleMode").and_then(KiwiValue::as_str) {
        Some("FIT") => ImageFitMode::Fit,
        Some("STRETCH") => ImageFitMode::Stretch,
        Some("TILE") => ImageFitMode::Tile,
        // FILL, CROP, or anything unrecognized.
        _ => ImageFitMode::Fill,
    }
}

/// Read an image paint's transform into a normalized `[x, y, w, h]` crop rect
/// (0..=1 asset space), the form `Fill::Image`'s `crop` (and the renderer's
/// `crop_to_pixels`) already use.
///
/// Figma has used both `imageTransform` and paint-level `transform` for this
/// matrix, and it can appear on normal FILL image paints as well as explicit
/// CROP paints. For the axis-aligned pan/zoom that image crop produces, the
/// matrix is `[[sx, 0, tx], [0, sy, ty]]`: `tx`/`ty` are the top-left of the
/// visible window in normalized image space and `sx`/`sy` its size. So the crop
/// rect is `[m02, m12, m00, m11]`.
///
/// Returns `None` (no crop → whole asset) unless the paint carries a
/// non-identity, non-degenerate axis-aligned image transform. A skew or rotation
/// (non-zero off-diagonal) is not representable as an axis-aligned crop rect, so
/// we conservatively fall back to `None` (the whole image, filled) rather than
/// emit a wrong sub-rect. The renderer clamps the rect to the asset bounds, so
/// an out-of-range window can never produce an invalid source rect.
pub(crate) fn image_crop(paint: &KiwiValue) -> Option<[f32; 4]> {
    let m = paint
        .get("imageTransform")
        .or_else(|| paint.get("transform"))?;
    let g = |k: &str, default: f64| m.get(k).and_then(KiwiValue::as_f64).unwrap_or(default);
    let (m00, m01, m02) = (g("m00", 1.0), g("m01", 0.0), g("m02", 0.0));
    let (m10, m11, m12) = (g("m10", 0.0), g("m11", 1.0), g("m12", 0.0));

    // Only axis-aligned crops (pure scale + translate) map to a rect.
    const EPS: f64 = 1e-6;
    if m01.abs() > EPS || m10.abs() > EPS {
        return None;
    }
    let (x, y, w, h) = (m02 as f32, m12 as f32, m00 as f32, m11 as f32);
    // A degenerate or identity window carries no useful crop.
    if w <= 0.0 || h <= 0.0 || (x == 0.0 && y == 0.0 && w >= 1.0 && h >= 1.0) {
        return None;
    }
    Some([x, y, w, h])
}

/// Map a Figma gradient paint to a doc [`Gradient`]. The paint's `transform` is
/// a 2x3 matrix mapping node-normalized `[0,1]²` → gradient space; the canonical
/// gradient handles live at `(0,0)` (start) and `(1,0)` (end) in gradient space,
/// so we invert the transform and apply it to those points to recover the
/// node-local endpoints the renderer expects. `paint_opacity` multiplies every
/// stop's alpha.
///
/// All four Figma gradient kinds now map to a native doc [`Gradient`] variant:
/// LINEAR → [`Gradient::Linear`], RADIAL → [`Gradient::Radial`],
/// ANGULAR → [`Gradient::Angular`] (conic sweep), DIAMOND → [`Gradient::Diamond`].
pub(crate) fn read_gradient(kind: &str, paint: &KiwiValue, paint_opacity: f64) -> Option<Gradient> {
    let stops = read_gradient_stops(paint, paint_opacity);
    if stops.is_empty() {
        return None;
    }
    let m = read_gradient_matrix(paint);
    // Center + radius shared by radial and diamond: center = inverse·(0.5,0.5),
    // a representative edge = inverse·(1,0.5); radius = the distance between them.
    let center_radius = || {
        let center = apply_inverse(&m, 0.5, 0.5);
        let edge = apply_inverse(&m, 1.0, 0.5);
        let radius = (((edge.0 - center.0).powi(2) + (edge.1 - center.1).powi(2)) as f32).sqrt();
        let radius = if radius.is_finite() && radius > 0.0 {
            radius
        } else {
            0.5
        };
        ([center.0 as f32, center.1 as f32], radius)
    };
    match kind {
        "GRADIENT_RADIAL" => {
            let (center, radius) = center_radius();
            Some(Gradient::Radial {
                center,
                radius,
                handles: gradient_axis_handles(&m),
                stops,
            })
        }
        "GRADIENT_DIAMOND" => {
            let (center, radius) = center_radius();
            Some(Gradient::Diamond {
                center,
                radius,
                handles: gradient_axis_handles(&m),
                stops,
            })
        }
        "GRADIENT_ANGULAR" => {
            // Conic sweep about the gradient center. The start direction is the
            // gradient's (0,0)→(1,0) axis in node space; its angle (radians,
            // clockwise from +x) is the sweep start angle the renderer wants.
            let center = apply_inverse(&m, 0.5, 0.5);
            let start = apply_inverse(&m, 0.0, 0.0);
            let end = apply_inverse(&m, 1.0, 0.0);
            let start_angle = ((end.1 - start.1) as f32).atan2((end.0 - start.0) as f32);
            Some(Gradient::Angular {
                center: [center.0 as f32, center.1 as f32],
                start_angle: if start_angle.is_finite() {
                    start_angle
                } else {
                    0.0
                },
                stops,
            })
        }
        // LINEAR (and any unrecognized gradient kind: linear is the safe default).
        _ => {
            let start = apply_inverse(&m, 0.0, 0.0);
            let end = apply_inverse(&m, 1.0, 0.0);
            Some(Gradient::Linear {
                start: [start.0 as f32, start.1 as f32],
                end: [end.0 as f32, end.1 as f32],
                stops,
            })
        }
    }
}

/// The full-ellipse axis handles of a radial/diamond gradient — the node-local
/// positions of gradient-space `(1, 0.5)` (radius handle) and `(0.5, 1)` (width
/// handle) — but only when they carry information the scalar `center + radius`
/// form loses: a rotated/skewed transform (non-zero off-diagonals) or
/// normalized-space anisotropy (the two axis lengths differ). Returns `None`
/// for the plain axis-aligned isotropic case, keeping the common gradient
/// byte-identical on the wire.
pub(crate) fn gradient_axis_handles(m: &[f64; 6]) -> Option<[[f32; 2]; 2]> {
    let center = apply_inverse(m, 0.5, 0.5);
    let x_end = apply_inverse(m, 1.0, 0.5);
    let y_end = apply_inverse(m, 0.5, 1.0);
    let x_axis = (x_end.0 - center.0, x_end.1 - center.1);
    let y_axis = (y_end.0 - center.0, y_end.1 - center.1);
    if !(x_end.0.is_finite() && x_end.1.is_finite() && y_end.0.is_finite() && y_end.1.is_finite()) {
        return None;
    }
    let len_x = (x_axis.0 * x_axis.0 + x_axis.1 * x_axis.1).sqrt();
    let len_y = (y_axis.0 * y_axis.0 + y_axis.1 * y_axis.1).sqrt();
    let rotated = x_axis.1.abs() > 1e-4 || y_axis.0.abs() > 1e-4;
    let anisotropic = (len_x - len_y).abs() > 1e-3 * len_x.max(len_y).max(1e-9);
    (rotated || anisotropic).then(|| {
        [
            [x_end.0 as f32, x_end.1 as f32],
            [y_end.0 as f32, y_end.1 as f32],
        ]
    })
}

/// Read a gradient paint's `stops` into doc [`GradientStop`]s, multiplying each
/// stop's alpha by the paint opacity.
pub(crate) fn read_gradient_stops(paint: &KiwiValue, paint_opacity: f64) -> Vec<GradientStop> {
    let Some(stops) = paint.get("stops").and_then(KiwiValue::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(stops.len());
    for s in stops {
        let pos = s.get("position").and_then(KiwiValue::as_f64).unwrap_or(0.0);
        // A stop carries its own color; fold the paint opacity into the alpha by
        // reusing read_color with the paint's opacity as the multiplier.
        if let Some(color) = s
            .get("color")
            .and_then(|c| read_color_with_opacity(c, paint_opacity))
        {
            out.push(GradientStop {
                position: pos.clamp(0.0, 1.0) as f32,
                color,
            });
        }
    }
    out
}

/// Read a gradient paint's `transform` 2x3 matrix as `[m00,m01,m02,m10,m11,m12]`,
/// defaulting to identity.
pub(crate) fn read_gradient_matrix(paint: &KiwiValue) -> [f64; 6] {
    let t = paint.get("transform");
    let g = |k: &str, d: f64| {
        t.and_then(|m| m.get(k))
            .and_then(KiwiValue::as_f64)
            .unwrap_or(d)
    };
    [
        g("m00", 1.0),
        g("m01", 0.0),
        g("m02", 0.0),
        g("m10", 0.0),
        g("m11", 1.0),
        g("m12", 0.0),
    ]
}

/// Apply the inverse of a 2x3 affine matrix `[m00,m01,m02,m10,m11,m12]` to the
/// point `(x, y)`. Used to map canonical gradient-space handles back into
/// node-normalized space. Falls back to the input point if the matrix is
/// singular (degenerate gradient).
pub(crate) fn apply_inverse(m: &[f64; 6], x: f64, y: f64) -> (f64, f64) {
    let [a, b, c, d, e, f] = *m; // [[a b c],[d e f]]
    let det = a * e - b * d;
    if det.abs() < 1e-12 {
        return (x, y);
    }
    let inv_det = 1.0 / det;
    // Inverse linear part.
    let ia = e * inv_det;
    let ib = -b * inv_det;
    let id = -d * inv_det;
    let ie = a * inv_det;
    // Inverse translation: -(inv_linear · translation).
    let itx = -(ia * c + ib * f);
    let ity = -(id * c + ie * f);
    (ia * x + ib * y + itx, id * x + ie * y + ity)
}

/// Read node-level effects (drop/inner shadow) into doc [`Shadow`]s. Blurs and
/// other effect kinds we don't model are skipped. Invisible effects are skipped.
pub(crate) fn read_effects(change: &KiwiValue) -> Vec<Shadow> {
    let Some(arr) = change.get("effects").and_then(KiwiValue::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in arr {
        let visible = e
            .get("visible")
            .map(|v| matches!(v, KiwiValue::Bool(true)))
            .unwrap_or(true);
        if !visible {
            continue;
        }
        let kind = match e.get("type").and_then(KiwiValue::as_str) {
            Some("DROP_SHADOW") => ShadowKind::Drop,
            Some("INNER_SHADOW") => ShadowKind::Inner,
            // Blur effects aren't a node-level Shadow; they are read separately
            // by `read_blurs` into the node's `blurs` list.
            _ => continue,
        };
        let color = e
            .get("color")
            .and_then(|c| read_color_with_opacity(c, 1.0))
            .unwrap_or(Color::rgba(0, 0, 0, 64));
        let blur = e.get("radius").and_then(KiwiValue::as_f64).unwrap_or(0.0);
        let spread = e.get("spread").and_then(KiwiValue::as_f64).unwrap_or(0.0);
        let ox = e
            .get("offset")
            .and_then(|o| o.get("x"))
            .and_then(KiwiValue::as_f64)
            .unwrap_or(0.0);
        let oy = e
            .get("offset")
            .and_then(|o| o.get("y"))
            .and_then(KiwiValue::as_f64)
            .unwrap_or(0.0);
        // Figma's default is `false` (drop shadow knocked out under the node's
        // own — possibly translucent — body), so an absent flag reads false.
        let show_behind_node = matches!(e.get("showShadowBehindNode"), Some(KiwiValue::Bool(true)));
        out.push(Shadow {
            kind,
            color,
            blur,
            spread,
            offset: [ox, oy],
            show_behind_node,
        });
    }
    out
}

/// Read a node's **blur** effects into doc [`Blur`]s. Maps Figma's
/// `FOREGROUND_BLUR` (a.k.a. `LAYER_BLUR`) → [`BlurKind::Layer`] and
/// `BACKGROUND_BLUR` → [`BlurKind::Background`]. The blur amount is the effect's
/// `radius`. Invisible effects (`visible: false`) and zero/negative-radius
/// effects are skipped (a 0-radius blur is a render no-op). Shadow effects are
/// handled by [`read_effects`]; everything else is ignored.
pub(crate) fn read_blurs(change: &KiwiValue) -> Vec<Blur> {
    let Some(arr) = change.get("effects").and_then(KiwiValue::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in arr {
        let visible = e
            .get("visible")
            .map(|v| matches!(v, KiwiValue::Bool(true)))
            .unwrap_or(true);
        if !visible {
            continue;
        }
        let kind = match e.get("type").and_then(KiwiValue::as_str) {
            // Figma's layer blur has been spelled both ways across versions.
            Some("FOREGROUND_BLUR") | Some("LAYER_BLUR") => BlurKind::Layer,
            // GLASS (Figma 2025's liquid-glass effect) is APPROXIMATED as its
            // dominant visual — a backdrop blur through the node's silhouette
            // at the effect's `radius`. The refraction/specular/chromatic
            // components (`refractionRadius`, `specularIntensity`, …) are not
            // modeled; without this mapping the whole effect silently dropped
            // and a frosted panel rendered fully crisp (verified against
            // Figma's own render — parity doc RE-4). Counted per node in
            // `MapReport::effects_glass_approximated` so the loss is loud.
            Some("BACKGROUND_BLUR") | Some("GLASS") => BlurKind::Background,
            _ => continue,
        };
        let radius = e.get("radius").and_then(KiwiValue::as_f64).unwrap_or(0.0);
        if !(radius.is_finite() && radius > 0.0) {
            continue;
        }
        out.push(Blur { kind, radius });
    }
    out
}

/// Map a Figma `BlendMode` enum member to a doc [`BlendMode`]. `PASS_THROUGH`
/// (group blend) and `NORMAL` map to `Normal`; unrecognized members return
/// `None` (leave the node's default).
pub(crate) fn blend_mode(member: &str) -> Option<BlendMode> {
    Some(match member {
        "NORMAL" | "PASS_THROUGH" => BlendMode::Normal,
        "MULTIPLY" => BlendMode::Multiply,
        "SCREEN" => BlendMode::Screen,
        "OVERLAY" => BlendMode::Overlay,
        "DARKEN" => BlendMode::Darken,
        "LIGHTEN" => BlendMode::Lighten,
        "COLOR_DODGE" => BlendMode::ColorDodge,
        "COLOR_BURN" => BlendMode::ColorBurn,
        "HARD_LIGHT" => BlendMode::HardLight,
        "SOFT_LIGHT" => BlendMode::SoftLight,
        "DIFFERENCE" => BlendMode::Difference,
        "EXCLUSION" => BlendMode::Exclusion,
        "HUE" => BlendMode::Hue,
        "SATURATION" => BlendMode::Saturation,
        "COLOR" => BlendMode::Color,
        "LUMINOSITY" => BlendMode::Luminosity,
        _ => return None,
    })
}

/// Convert a Figma `Color {r,g,b,a}` (floats in 0..=1) plus the paint's own
/// `opacity` into a doc [`Color`] (u8 channels).
pub(crate) fn read_color(color: &KiwiValue, paint: &KiwiValue) -> Option<Color> {
    let paint_opacity = paint
        .get("opacity")
        .and_then(KiwiValue::as_f64)
        .unwrap_or(1.0);
    read_color_with_opacity(color, paint_opacity)
}

/// Convert a Figma `Color {r,g,b,a}` (floats in 0..=1) with an explicit alpha
/// multiplier into a doc [`Color`] (u8 channels).
pub(crate) fn read_color_with_opacity(color: &KiwiValue, opacity: f64) -> Option<Color> {
    let r = color.get("r").and_then(KiwiValue::as_f64)?;
    let g = color.get("g").and_then(KiwiValue::as_f64)?;
    let b = color.get("b").and_then(KiwiValue::as_f64)?;
    let a = color.get("a").and_then(KiwiValue::as_f64).unwrap_or(1.0);
    let to_u8 = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    Some(Color::rgba(
        to_u8(r),
        to_u8(g),
        to_u8(b),
        to_u8(a * opacity.clamp(0.0, 1.0)),
    ))
}
