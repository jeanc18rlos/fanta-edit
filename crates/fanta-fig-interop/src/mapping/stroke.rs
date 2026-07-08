//! Stroke (border) construction and the optional-clip group helper.

use super::{
    Color, Fill, GroupNode, KiwiValue, Stroke, StrokeAlign, StrokeCap, StrokeJoin, corner_radii,
    corner_smoothing, read_fills, read_paint,
};

/// Build the node's stroke(s) from `strokePaints` + `strokeWeight` (+
/// `strokeAlign`, `strokeCap`, `strokeJoin`, `dashPattern`, `miterLimit`).
///
/// **Every** visible stroke paint (solid OR gradient) becomes its own [`Stroke`]
/// — Figma stacks multiple stroke paints on one node (e.g. a solid edge under a
/// gradient sheen), painting them bottom-to-top in array order. The old mapper
/// kept only the *first* visible paint (`find_map(read_paint)`), silently
/// dropping every additional border layer. We now emit one stroke per visible
/// paint, all sharing the node's single `strokeWeight`/`strokeAlign`/cap/join/
/// dash/miter (Figma stores those once per node, not per paint), preserving
/// array order so the renderer composites them in the same z-order Figma does.
///
/// Each stroke is painted at `strokeWeight` (default 1px) with the node's
/// `strokeAlign`. (We render strokes from the node's path + width, matching how
/// the doc model + renderer expect them; Figma's pre-expanded `strokeGeometry`
/// outline is redundant with our width-based stroking.) We emit strokes only
/// when there is a real positive weight or baked stroke geometry to justify it;
/// missing `strokePaints` falls back to Figma's default black stroke.
///
/// We also carry the line-decoration fields the renderer already honors via
/// [`fanta_render::stroke_to_paint`] (zero renderer change): `strokeCap`
/// (NONE→Butt/ROUND→Round/SQUARE→Square), `strokeJoin` (MITER/BEVEL/ROUND),
/// `dashPattern` (an array of on/off lengths → `dash`), and `miterLimit`
/// → `miter_limit`. Mirrors op2 `figma-stroke-mapper.ts` (its `visibleStrokes`
/// filter, here producing one Stroke per surviving paint).
pub(crate) fn build_stroke(change: &KiwiValue) -> Vec<Stroke> {
    let mut out = Vec::new();
    let has_stroke_geom = change
        .get("strokeGeometry")
        .and_then(KiwiValue::as_array)
        .map(|a| !a.is_empty())
        .unwrap_or(false);
    let weight = change.get("strokeWeight").and_then(KiwiValue::as_f64);
    // A border only paints with positive weight (or baked stroke geometry).
    // Figma frames very commonly carry a stroke paint with `strokeWeight: 0`, or
    // a paint flagged `visible: false` — both mean the border is toggled OFF in
    // the UI. The old `weight.is_some()` gate + 1px default imported those as
    // phantom borders on thousands of frames (now that frames import strokes).
    // Gate on real positive weight or baked stroke geometry so toggled-off
    // borders stay off, while genuine borders still render.
    let positive_weight = weight.is_some_and(|w| w > 0.0);
    if !(has_stroke_geom || positive_weight) {
        return out;
    }
    let width = weight.filter(|w| *w > 0.0).unwrap_or(1.0);
    let align = match change.get("strokeAlign").and_then(KiwiValue::as_str) {
        Some("INSIDE") => StrokeAlign::Inside,
        Some("OUTSIDE") => StrokeAlign::Outside,
        None if defaults_to_inside_stroke(change) => StrokeAlign::Inside,
        _ => StrokeAlign::Center,
    };
    let cap = stroke_cap(change.get("strokeCap").and_then(KiwiValue::as_str));
    let join = stroke_join(change.get("strokeJoin").and_then(KiwiValue::as_str));
    let miter_limit = stroke_miter_limit(change);
    let dash = dash_pattern(change);
    // Figma's "individual" border weights: when `borderStrokeWeightsIndependent`
    // is set the four edges can carry different widths
    // (`borderTop/Right/Bottom/LeftWeight`), e.g. an underline-only border or a
    // table cell. Carried as an additive `per_side` array on every emitted stroke
    // so the renderer can stroke each rectangle edge at its own width; falls back
    // to the uniform `width` for non-rectangle paths. `None` when the node uses a
    // single uniform weight (the overwhelmingly common case).
    let per_side = per_side_weights(change);

    // Collect EVERY visible stroke paint, in array (bottom-to-top) order. A paint
    // is skipped only if it's flagged `visible: false` OR is an unrecognized /
    // empty paint kind (`read_paint` returns `None`) — matching the single-paint
    // gate the old code applied, just no longer stopping at the first hit.
    //
    // Some `.fig` exports omit `strokePaints` for rectangle-like wireframe boxes
    // that still carry a positive `strokeWeight`; Figma renders those as the
    // default black stroke. Keep this fallback narrow: decorative vector-family
    // shapes can also carry a default `strokeWeight` even when no border is
    // visible, so broad fallback invents outlines on stars/icons.
    let paints: Vec<Fill> = match change.get("strokePaints").and_then(KiwiValue::as_array) {
        Some(arr) => arr
            .iter()
            .filter(|p| !matches!(p.get("visible"), Some(KiwiValue::Bool(false))))
            .filter_map(read_paint)
            .collect(),
        None if default_black_stroke(change) => vec![Fill::solid(Color::BLACK)],
        None => Vec::new(),
    };

    for paint in paints {
        out.push(Stroke {
            paint,
            width,
            align,
            cap,
            join,
            // Each stroke owns its own dash vec (the renderer reads it by value);
            // they share the same pattern but must not alias one allocation.
            miter_limit,
            dash: dash.clone(),
            per_side,
        });
    }
    out
}

fn default_black_stroke(change: &KiwiValue) -> bool {
    matches!(
        change.get("type").and_then(KiwiValue::as_str),
        Some("RECTANGLE" | "ROUNDED_RECTANGLE")
    ) && read_fills(change).is_empty()
}

fn defaults_to_inside_stroke(change: &KiwiValue) -> bool {
    matches!(
        change.get("type").and_then(KiwiValue::as_str),
        Some(
            "RECTANGLE"
                | "ROUNDED_RECTANGLE"
                | "FRAME"
                | "SECTION"
                | "SYMBOL"
                | "COMPONENT"
                | "COMPONENT_SET"
        )
    )
}

/// Read Figma's individual per-side border weights `[top, right, bottom, left]`,
/// or `None` when the node uses a single uniform `strokeWeight`.
///
/// Returns `Some` only when `borderStrokeWeightsIndependent` is set AND the four
/// resolved weights are not all equal — an all-equal "individual" border is
/// indistinguishable from a uniform one, so we collapse it to `None` and let the
/// uniform `strokeWeight` path drive it (keeping the common case allocation-free
/// in the renderer and the JSON byte-stable). Mirrors op2
/// `figma-stroke-mapper.ts` `borderStrokeWeightsIndependent` branch.
pub(crate) fn per_side_weights(change: &KiwiValue) -> Option<[f64; 4]> {
    let independent = matches!(
        change.get("borderStrokeWeightsIndependent"),
        Some(KiwiValue::Bool(true))
    );
    if !independent {
        return None;
    }
    let w = |k: &str| change.get(k).and_then(KiwiValue::as_f64).unwrap_or(0.0);
    let sides = [
        w("borderTopWeight"),
        w("borderRightWeight"),
        w("borderBottomWeight"),
        w("borderLeftWeight"),
    ];
    let all_equal = sides.iter().all(|s| (s - sides[0]).abs() < f64::EPSILON);
    if all_equal { None } else { Some(sides) }
}

/// Map Figma's `strokeCap` enum to the doc [`StrokeCap`]. NONE (Figma's default,
/// a butt cap) and any unrecognized value fall back to `Butt`.
pub(crate) fn stroke_cap(cap: Option<&str>) -> StrokeCap {
    match cap {
        Some("ROUND") => StrokeCap::Round,
        Some("SQUARE") => StrokeCap::Square,
        // NONE, the arrow caps (ARROW_LINES/ARROW_EQUILATERAL — no doc analog),
        // or absent: a plain butt cap.
        _ => StrokeCap::Butt,
    }
}

/// Map Figma's `strokeJoin` enum to the doc [`StrokeJoin`]. MITER (Figma's
/// default) and any unrecognized value fall back to `Miter`.
pub(crate) fn stroke_join(join: Option<&str>) -> StrokeJoin {
    match join {
        Some("BEVEL") => StrokeJoin::Bevel,
        Some("ROUND") => StrokeJoin::Round,
        // MITER or absent.
        _ => StrokeJoin::Miter,
    }
}

/// Read Figma's `miterLimit` — already an SVG-style miter-limit RATIO, exactly
/// what the doc model + renderer expect. (An earlier version read a
/// `strokeMiterAngle` field and converted degrees → ratio, but that field name
/// never occurs in real `.fig` files — the Kiwi schema stores
/// `NodeChange.miterLimit`, e.g. 16 on the Spectrum "Tab Unit" frames.) Absent
/// or out-of-range values keep the doc default (4.0).
pub(crate) fn stroke_miter_limit(change: &KiwiValue) -> f64 {
    const DEFAULT_MITER_LIMIT: f64 = 4.0;
    change
        .get("miterLimit")
        .and_then(KiwiValue::as_f64)
        .filter(|limit| limit.is_finite() && *limit >= 1.0)
        .map(|limit| limit.clamp(1.0, 1.0e6))
        .unwrap_or(DEFAULT_MITER_LIMIT)
}

/// Read Figma's `dashPattern` (an array of alternating on/off dash lengths) into
/// the doc's `dash` vec. Empty/absent → solid stroke (empty vec). Non-finite or
/// negative entries are dropped; an all-zero pattern collapses to solid.
pub(crate) fn dash_pattern(change: &KiwiValue) -> Vec<f64> {
    let Some(arr) = change.get("dashPattern").and_then(KiwiValue::as_array) else {
        return Vec::new();
    };
    let dashes: Vec<f64> = arr
        .iter()
        .filter_map(KiwiValue::as_f64)
        .filter(|v| v.is_finite() && *v >= 0.0)
        .collect();
    if dashes.iter().all(|v| *v == 0.0) {
        return Vec::new();
    }
    dashes
}

/// A [`GroupNode`] with a real, non-default box when the source node has size.
/// `frameMaskDisabled` disables the render-time child clip via node meta, but
/// the box itself still exists for background, border, bounds, and layout.
pub(crate) fn group_with_optional_clip(
    change: &KiwiValue,
    size: (f64, f64),
    background: Option<Fill>,
) -> GroupNode {
    let has_size = change.get("size").is_some() && (size.0 > 1.0 || size.1 > 1.0);
    let (uniform, per_corner) = corner_radii(change);
    GroupNode {
        clip_size: has_size.then_some([size.0, size.1]),
        background,
        background_fills: read_fills(change).into_iter().skip(1).collect(),
        explicit_modes: Default::default(),
        // `build_node` fills this from the change's `stack*` fields afterward.
        auto_layout: None,
        // A component/instance frame can carry its own border + rounding.
        strokes: build_stroke(change).into_iter().collect(),
        corner_radius: uniform,
        corner_radii: per_corner,
        corner_smoothing: corner_smoothing(change),
    }
}
