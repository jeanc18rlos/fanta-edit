//! Vector + frame geometry painting: rect/rounded-rect path construction,
//! stroke alignment (incl. per-side borders), the vector-fill draw path, and
//! the debug placeholder / unresolved-instance outline helpers.
use super::{
    Bounds, Canvas, Color, Fill, Paint, Rect, RenderCtx, draw_image_cached, fill_to_paint,
    stroke_to_paint, to_sk_color, to_sk_path,
};

/// Whether a [`PathData`] is an axis-aligned rectangle (the shape `corner_radius`
/// is allowed to round). Recognizes the canonical `Move + 3×Line + Close`
/// rectangle [`PathData::rect`] emits, requiring every edge to be horizontal or
/// vertical. Anything else (real vector geometry, ellipses) returns false so its
/// `corner_radius` is ignored, matching the doc-model contract.
pub(crate) fn path_is_rect(path: &fanta_doc::PathData) -> bool {
    // Single source of truth lives on `PathData` so the renderer and the
    // auto-layout resizer (`fanta_doc::layout::size`) agree on what counts as a
    // resizable rectangle vs. real vector geometry that must be preserved.
    path.is_rect()
}

/// Build a Skia path for an axis-aligned rect `[x, y, w, h]` (logical px),
/// rounded by an optional uniform `corner_radius` or independent `corner_radii`
/// ([TL, TR, BR, BL]). `corner_radii` takes precedence; absent/zero rounding
/// yields a plain rectangle path. Each radius is clamped to half the smaller box
/// dimension so two adjacent radii never overlap (Skia would otherwise scale
/// them down silently). Shared by the vector-rect path and the frame-box path so
/// a frame's background + border round exactly like a rounded rect.
pub(crate) fn rounded_rect_path(
    bounds_f32: [f32; 4],
    corner_radius: Option<f64>,
    corner_radii: Option<[f64; 4]>,
    smoothing: f32,
) -> skia_safe::Path {
    let [x, y, w, h] = bounds_f32;
    let rect = Rect::from_xywh(x, y, w, h);
    // Clamp any single radius to half the smaller box dimension so two adjacent
    // radii never overlap (Skia would otherwise scale them down).
    let cap = (w.min(h) * 0.5).max(0.0);
    // Corner smoothing → continuous-curvature superellipse corners (only when
    // there's both smoothing AND a radius; otherwise fall through to the exact,
    // byte-identical original construction so smoothing=0 never shifts a pixel).
    if smoothing > 0.001 {
        let radii: Option<[f32; 4]> = match corner_radii {
            Some([tl, tr, br, bl]) => Some([tl as f32, tr as f32, br as f32, bl as f32]),
            None => corner_radius.filter(|r| *r > 0.0).map(|r| [r as f32; 4]),
        };
        if let Some(r) = radii {
            return superellipse_rect_path(rect, r.map(|v| v.clamp(0.0, cap)), smoothing);
        }
    }
    match corner_radii {
        Some([tl, tr, br, bl]) => {
            let c = |v: f64| (v as f32).clamp(0.0, cap);
            // skia_safe::RRect radii order is TL, TR, BR, BL — matching ours.
            let radii = [
                skia_safe::Point::new(c(tl), c(tl)),
                skia_safe::Point::new(c(tr), c(tr)),
                skia_safe::Point::new(c(br), c(br)),
                skia_safe::Point::new(c(bl), c(bl)),
            ];
            let rrect = skia_safe::RRect::new_rect_radii(rect, &radii);
            let mut p = skia_safe::Path::new();
            p.add_rrect(rrect, None);
            p
        }
        None => match corner_radius {
            Some(r) if r > 0.0 => {
                let r = (r as f32).clamp(0.0, cap);
                let mut p = skia_safe::Path::new();
                p.add_round_rect(rect, (r, r), None);
                p
            }
            _ => {
                let mut p = skia_safe::Path::new();
                p.add_rect(rect, None);
                p
            }
        },
    }
}

/// A rounded-rect path whose corners are quarter-superellipses (Figma "corner
/// smoothing" / squircles), sampled as a fine polyline. `radii` is `[TL,TR,BR,BL]`
/// (already clamped); `smoothing` 0..=1 maps to the superellipse exponent
/// (`n = 2` ≈ circle … `n ≈ 6` ≈ Apple squircle). Built clockwise from the top
/// edge so fill/clip/stroke all share one path.
fn superellipse_rect_path(rect: Rect, radii: [f32; 4], smoothing: f32) -> skia_safe::Path {
    let (x0, y0, x1, y1) = (rect.left, rect.top, rect.right, rect.bottom);
    let [tl, tr, br, bl] = radii;
    let n = 2.0 + smoothing.clamp(0.0, 1.0) * 4.0;
    let e = 2.0 / n;
    const SEGS: usize = 16;
    let mut p = skia_safe::Path::new();
    // One corner: line to `SEGS` sampled superellipse points; `dir(c,s)` maps the
    // (cosᵉ, sinᵉ) pair to the unit offset from the corner's rounding pivot.
    let corner =
        |p: &mut skia_safe::Path, px: f32, py: f32, r: f32, dir: fn(f32, f32) -> (f32, f32)| {
            if r <= 0.01 {
                // Sharp corner: the pivot collapses onto the box corner; one point.
                let (dx, dy) = dir(1.0, 0.0);
                p.line_to((px + dx * r, py + dy * r));
                return;
            }
            for i in 1..=SEGS {
                let t = i as f32 / SEGS as f32 * std::f32::consts::FRAC_PI_2;
                let c = t.cos().abs().powf(e);
                let s = t.sin().abs().powf(e);
                let (dx, dy) = dir(c, s);
                p.line_to((px + dx * r, py + dy * r));
            }
        };
    p.move_to((x0 + tl, y0));
    p.line_to((x1 - tr, y0));
    corner(&mut p, x1 - tr, y0 + tr, tr, |c, s| (s, -c)); // TR
    p.line_to((x1, y1 - br));
    corner(&mut p, x1 - br, y1 - br, br, |c, s| (c, s)); // BR
    p.line_to((x0 + bl, y1));
    corner(&mut p, x0 + bl, y1 - bl, bl, |c, s| (-s, c)); // BL
    p.line_to((x0, y0 + tl));
    corner(&mut p, x0 + tl, y0 + tl, tl, |c, s| (-c, -s)); // TL
    p.close();
    p
}

/// Stroke `sk_path` with each [`Stroke`] in `strokes`, honoring Figma stroke
/// alignment. A centered stroke straddles the path edge (half in, half out);
/// Inside/Outside must fall entirely on one side. Skia only strokes centered, so
/// for Inside/Outside we clip to (or against) the path's region and stroke at
/// double width — the clip discards the half we don't want, leaving the
/// remaining half exactly `stroke.width` thick. `local_bounds_f32` is the path's
/// bounding box, used for gradient/image stroke paint mapping. Shared by the
/// vector-shape stroke and the frame-border stroke so a frame's border draws
/// with the same alignment semantics as a shape's. Bumps `nodes_drawn` per stroke.
pub(crate) fn stroke_sk_path(
    canvas: &Canvas,
    sk_path: &skia_safe::Path,
    strokes: &[fanta_doc::Stroke],
    local_bounds_f32: [f32; 4],
    ctx: &mut RenderCtx,
) {
    for stroke in strokes {
        // Per-side border weights (Figma "individual" borders): each rectangle
        // edge is stroked at its own width. Only meaningful on a rectangular box,
        // so when the per-side array is present AND the box is non-degenerate we
        // draw the four edges individually; otherwise fall through to the uniform
        // width path below. (Per-side already self-guards each zero-width edge, so
        // the uniform-width guard below does not apply when `per_side` is set.)
        if let Some(sides) = stroke.per_side {
            if draw_per_side_border(canvas, sk_path, stroke, sides, local_bounds_f32) {
                ctx.metrics.nodes_drawn += 1;
                continue;
            }
        }
        // A zero/negative-width uniform stroke paints NOTHING in Figma — but Skia
        // treats `stroke_width == 0` as a *hairline* (one device pixel wide,
        // regardless of transform/zoom). Imported `.fig` shapes routinely carry a
        // stroke paint with a 0 weight (a disabled/animated-off border, or a paint
        // left attached after the weight was zeroed); drawing those as hairlines
        // sprinkles spurious 1px outlines over the composition that scale-invert
        // when zooming. Skip them so the geometric edge is honored. (Per-side
        // strokes are handled above and never reach here.)
        if stroke.width <= 0.0 {
            continue;
        }
        match stroke.align {
            fanta_doc::StrokeAlign::Center => {
                let paint = stroke_to_paint(stroke, local_bounds_f32);
                canvas.draw_path(sk_path, &paint);
            }
            fanta_doc::StrokeAlign::Inside => {
                let mut paint = stroke_to_paint(stroke, local_bounds_f32);
                paint.set_stroke_width((stroke.width * 2.0) as f32);
                canvas.save();
                // Keep only the part of the doubled stroke that lands inside
                // the region; the inner half == `stroke.width` thick.
                canvas.clip_path(sk_path, skia_safe::ClipOp::Intersect, true);
                canvas.draw_path(sk_path, &paint);
                canvas.restore();
            }
            fanta_doc::StrokeAlign::Outside => {
                let mut paint = stroke_to_paint(stroke, local_bounds_f32);
                paint.set_stroke_width((stroke.width * 2.0) as f32);
                canvas.save();
                // Clip OUT the region so only the outer half of the doubled
                // stroke remains; strokes are drawn after fills, so the fill
                // interior stays intact.
                canvas.clip_path(sk_path, skia_safe::ClipOp::Difference, true);
                canvas.draw_path(sk_path, &paint);
                canvas.restore();
            }
        }
        ctx.metrics.nodes_drawn += 1;
    }
}

pub(crate) fn stroke_box_path(
    canvas: &Canvas,
    local_bounds_f32: [f32; 4],
    corner_radius: Option<f64>,
    corner_radii: Option<[f64; 4]>,
    corner_smoothing: f32,
    strokes: &[fanta_doc::Stroke],
    ctx: &mut RenderCtx,
) {
    let original_path = || {
        rounded_rect_path(
            local_bounds_f32,
            corner_radius,
            corner_radii,
            corner_smoothing,
        )
    };

    for stroke in strokes {
        if let Some(sides) = stroke.per_side {
            let box_path = original_path();
            if draw_per_side_border(canvas, &box_path, stroke, sides, local_bounds_f32) {
                ctx.metrics.nodes_drawn += 1;
                continue;
            }
        }

        let stroke_width = stroke.width as f32;
        if !stroke_width.is_finite() || stroke_width <= 0.0 {
            continue;
        }

        let (offset, radius_delta) = match stroke.align {
            fanta_doc::StrokeAlign::Center => (0.0, 0.0),
            fanta_doc::StrokeAlign::Inside => (stroke_width * 0.5, -stroke_width * 0.5),
            fanta_doc::StrokeAlign::Outside => (-stroke_width * 0.5, stroke_width * 0.5),
        };

        let Some(stroke_bounds) = offset_box_bounds(local_bounds_f32, offset) else {
            let box_path = original_path();
            stroke_sk_path(
                canvas,
                &box_path,
                std::slice::from_ref(stroke),
                local_bounds_f32,
                ctx,
            );
            continue;
        };

        let stroke_path = rounded_rect_path(
            stroke_bounds,
            offset_corner_radius(corner_radius, radius_delta),
            offset_corner_radii(corner_radii, radius_delta),
            corner_smoothing,
        );
        let paint = stroke_to_paint(stroke, local_bounds_f32);
        canvas.draw_path(&stroke_path, &paint);
        ctx.metrics.nodes_drawn += 1;
    }
}

fn offset_box_bounds(bounds: [f32; 4], inset: f32) -> Option<[f32; 4]> {
    let [x, y, w, h] = bounds;
    let width = w - inset * 2.0;
    let height = h - inset * 2.0;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    Some([x + inset, y + inset, width, height])
}

fn offset_corner_radius(radius: Option<f64>, delta: f32) -> Option<f64> {
    radius.map(|radius| (radius + f64::from(delta)).max(0.0))
}

fn offset_corner_radii(radii: Option<[f64; 4]>, delta: f32) -> Option<[f64; 4]> {
    radii.map(|radii| radii.map(|radius| (radius + f64::from(delta)).max(0.0)))
}

/// Draw a rectangle's four borders at independent per-side widths (Figma's
/// "individual" border weights), honoring the stroke's [`StrokeAlign`]. Returns
/// `true` when it drew (a non-degenerate box), `false` when the box is degenerate
/// and the caller should fall back to the uniform-width stroke path.
///
/// `sides` is `[top, right, bottom, left]` in logical px (the doc's `per_side`).
/// Each edge is a filled rectangle band hugging that side of the box; its inner
/// extent depends on the alignment:
/// - **Inside**: the band lies entirely inside the box edge.
/// - **Outside**: entirely outside.
/// - **Center**: straddles the edge (half in, half out).
///
/// A zero-width side draws nothing (an edge can be toggled off). Edges are drawn
/// as rects rather than stroked lines so adjacent edges of different widths meet
/// without a miter artifact and so each edge's alignment is exact. The paint
/// (color / gradient) is taken from the stroke via [`stroke_to_paint`], reusing
/// the same `local_bounds_f32` mapping a uniform stroke uses.
///
/// **Rounded corners.** `shape_path` is the node's *actual* outline — the
/// rounded-rect path the fill/uniform stroke use (or a plain rect when there is
/// no rounding). For an **Inside**-aligned border (Figma's default for shapes and
/// the way individual border weights are authored) the rounded outline IS the
/// silhouette the bands must stay within, so each band is clipped to it: at a
/// rounded corner the square band is shaved to follow the curve instead of poking
/// a hard square corner past the rounded fill.
///
/// **Center/Outside** bands extend *past* the inner outline (their outer half /
/// whole lies outside the shape), so clipping them to the inner outline would
/// wrongly erase them. They are instead clipped to the **outward-offset**
/// silhouette of `shape_path` — the outline grown outward by the band's outer
/// reach via [`outset_silhouette`], whose corners are the concentric (so still
/// rounded) parallel curve of the original corners. That shaves the square corner
/// overshoot a Center/Outside band would otherwise poke past a rounded rect
/// (Figma rounds an outside border's outer corner to `radius + width`) while
/// leaving every band's straight outer edge untouched. The offset clip is skipped
/// when `shape_path` is a plain rectangle (`Path::is_rect`) — its outward offset
/// is just a larger rectangle the bands already fit inside, so the common
/// hard-edged square-border path is byte-identical and pays no path-op cost.
pub(crate) fn draw_per_side_border(
    canvas: &Canvas,
    shape_path: &skia_safe::Path,
    stroke: &fanta_doc::Stroke,
    sides: [f64; 4],
    local_bounds_f32: [f32; 4],
) -> bool {
    let [x, y, w, h] = local_bounds_f32;
    if w <= 0.0 || h <= 0.0 {
        return false;
    }
    // A fill paint sampled from the stroke's paint over the box bounds (so a
    // gradient border maps across the whole box just like a uniform stroke).
    let mut paint = stroke_to_paint(stroke, local_bounds_f32);
    paint.set_style(skia_safe::paint::Style::Fill);
    paint.set_path_effect(None); // edges are solid bands, not dashed lines

    // For each side, `(outer_off, inner_off)` are how far the band extends past
    // the box edge outward and inward, by alignment.
    let (out_frac, in_frac) = match stroke.align {
        fanta_doc::StrokeAlign::Inside => (0.0_f32, 1.0_f32),
        fanta_doc::StrokeAlign::Outside => (1.0, 0.0),
        fanta_doc::StrokeAlign::Center => (0.5, 0.5),
    };
    let [t, r, b, l] = sides.map(|v| v.max(0.0) as f32);
    // Inside bands stay within the rounded outline, so clip them to it (rounded
    // corners follow the curve). Center/Outside bands extend *beyond* the outline,
    // so instead of the inner outline they clip to the outward-OFFSET silhouette
    // (the outline grown by the band's outer reach), whose rounded corners are the
    // concentric parallel curve of the original corners — shaving the square
    // corner overshoot while keeping each straight outer edge. The clip path is
    // built once here and reused for all four bands.
    //
    // The offset is the largest per-side outer reach: a band stroked at its own
    // width never overshoots a silhouette grown by the *max* width's outer reach,
    // so one silhouette confines all four bands. A degenerate / plain-rect shape
    // (where the offset is just a larger rect the bands already fit) yields `None`
    // and we skip the clip — keeping the square-box path byte-identical.
    let max_width = t.max(r).max(b).max(l);
    let clip_path: Option<skia_safe::Path> = match stroke.align {
        fanta_doc::StrokeAlign::Inside => Some(shape_path.clone()),
        fanta_doc::StrokeAlign::Center | fanta_doc::StrokeAlign::Outside => {
            outset_silhouette(shape_path, max_width * out_frac)
        }
    };
    // The clip is anti-aliased to match the soft fill/stroke edges the rest of the
    // renderer draws. Scoped by the save/restore pair around all four bands so it
    // never leaks to siblings.
    let clip_to_shape = clip_path.is_some();
    if let Some(ref cp) = clip_path {
        canvas.save();
        canvas.clip_path(cp, skia_safe::ClipOp::Intersect, true);
    }
    let x0 = x;
    let y0 = y;
    let x1 = x + w;
    let y1 = y + h;

    // Top edge: a horizontal band along y0, spanning the full (outer) width so it
    // covers the corners with the adjacent vertical edges (last-drawn wins, which
    // is fine for a single-color border; for differing widths the corner is owned
    // by whichever edge is wider, matching Figma's overlap).
    let mut drew = false;
    if t > 0.0 {
        let rect = Rect::from_ltrb(
            x0 - l * out_frac,
            y0 - t * out_frac,
            x1 + r * out_frac,
            y0 + t * in_frac,
        );
        canvas.draw_rect(rect, &paint);
        drew = true;
    }
    if b > 0.0 {
        let rect = Rect::from_ltrb(
            x0 - l * out_frac,
            y1 - b * in_frac,
            x1 + r * out_frac,
            y1 + b * out_frac,
        );
        canvas.draw_rect(rect, &paint);
        drew = true;
    }
    if l > 0.0 {
        let rect = Rect::from_ltrb(
            x0 - l * out_frac,
            y0 - t * out_frac,
            x0 + l * in_frac,
            y1 + b * out_frac,
        );
        canvas.draw_rect(rect, &paint);
        drew = true;
    }
    if r > 0.0 {
        let rect = Rect::from_ltrb(
            x1 - r * in_frac,
            y0 - t * out_frac,
            x1 + r * out_frac,
            y1 + b * out_frac,
        );
        canvas.draw_rect(rect, &paint);
        drew = true;
    }
    if clip_to_shape {
        canvas.restore();
    }
    drew
}

/// Build the silhouette of `shape_path` grown **outward** by `offset` logical px,
/// or `None` when no offset clip is needed.
///
/// Used to confine Center/Outside per-side border bands so their corners follow
/// the outline's *parallel curve* (a rounded rect's outward offset has corner
/// radius `r + offset`) instead of poking a hard square corner past a rounded
/// shape. The offset shape is `shape ∪ ring`, where `ring` is `shape_path`
/// stroked centered at width `2·offset` and converted to its filled equivalent
/// via [`skia_safe::path_utils::fill_path_with_paint`]: the outer edge of that
/// centered ring sits exactly `offset` outside the outline, and unioning it with
/// the filled outline yields the outline grown by `offset` everywhere — corners
/// included, rounded where the original corners are.
///
/// Returns `None` (caller skips the clip) when:
/// - `offset` is non-positive (a zero/Inside outer reach needs no growth); or
/// - `shape_path` is a plain rectangle ([`skia_safe::Path::is_rect`]) — its
///   outward offset is just a larger rectangle the bands already fit inside, so
///   clipping would be a no-op. This keeps the common hard-edged square-border
///   path byte-identical and pays no path-op cost; or
/// - the Skia stroke→fill or union step fails (degenerate geometry) — falling
///   back to the un-clipped band is safe (it just keeps the prior square-corner
///   behaviour for that one odd shape rather than dropping the border).
fn outset_silhouette(shape_path: &skia_safe::Path, offset: f32) -> Option<skia_safe::Path> {
    if offset <= 0.0 {
        return None;
    }
    // A plain rectangle's outward offset is a larger rectangle the axis-aligned
    // bands already fit within — nothing to round, so skip (byte-identical path).
    if shape_path.is_rect().is_some() {
        return None;
    }
    // The ring whose OUTER edge is `offset` outside the outline: stroke the
    // outline centered at `2·offset`, then take its filled equivalent.
    let mut ring_paint = Paint::default();
    ring_paint.set_style(skia_safe::paint::Style::Stroke);
    ring_paint.set_stroke_width(offset * 2.0);
    // Round joins so the offset corner is the concentric arc (the parallel curve
    // of a rounded corner), matching Figma's `radius + width` outer rounding.
    ring_paint.set_stroke_join(skia_safe::paint::Join::Round);
    let mut ring = skia_safe::Path::new();
    if !skia_safe::path_utils::fill_path_with_paint(shape_path, &ring_paint, &mut ring, None, None)
    {
        return None;
    }
    // shape ∪ ring = the outline grown outward by `offset` (corners rounded).
    shape_path.op(&ring, skia_safe::PathOp::Union)
}

/// The effective Skia outline of a vector node — exactly the path
/// [`draw_vector`] fills, strokes, and clips against. A `corner_radius`/
/// `corner_radii` only applies to rectangle-shaped paths (doc-model contract):
/// when set on a rect, the outline is the rounded-rect instead of the square
/// path — this is what makes imported Figma buttons/badges (13k+ rounded rects
/// in the Spectrum file) read as pills/cards rather than hard boxes.
/// Independent `corner_radii` ([TL, TR, BR, BL]) take precedence and build the
/// RRect via `RRect::new_rect_radii` (a distinct (rx, ry) per corner) so
/// mixed-corner cards/tabs/segmented controls round correctly.
///
/// Public (and shared with `draw_vector`) so the app's geometry operations —
/// flatten transforms / outline stroke / simplify (track svg-prod) — operate on
/// exactly the outline the canvas draws and the two can never drift.
pub fn vector_outline_sk_path(
    path: &fanta_doc::PathData,
    corner_radius: Option<f64>,
    corner_radii: Option<[f64; 4]>,
) -> skia_safe::Path {
    if path_is_rect(path) {
        let bounds = path.rough_bounds().unwrap_or(Bounds::ZERO);
        // Vector shapes carry no corner-smoothing field yet (frames do), so 0.0.
        rounded_rect_path(bounds_to_f32(&bounds), corner_radius, corner_radii, 0.0)
    } else {
        to_sk_path(path)
    }
}

pub(crate) fn draw_vector(
    canvas: &Canvas,
    path: &fanta_doc::PathData,
    fills: &[fanta_doc::Fill],
    strokes: &[fanta_doc::Stroke],
    corner_radius: Option<f64>,
    corner_radii: Option<[f64; 4]>,
    ctx: &mut RenderCtx,
) {
    let bounds = path.rough_bounds().unwrap_or(Bounds::ZERO);
    let local_bounds_f32 = bounds_to_f32(&bounds);
    let sk_path = vector_outline_sk_path(path, corner_radius, corner_radii);

    for fill in fills {
        // Image fills route through the same decode/fit path as BitmapNode:
        // clip to the vector path, then draw the image fitted to the path's
        // bounding box. `paint.rs` cannot do this (it has no resolver), so the
        // renderer owns it. A missing asset falls back to the magenta
        // placeholder paint `fill_to_paint` already produces.
        if let Fill::Image {
            asset,
            mode,
            opacity,
            crop,
        } = fill
        {
            // Hoisted so the resolve closure captures a local, not `ctx`,
            // which `ctx.cache` borrows mutably.
            let resolver = ctx.resolver;
            canvas.save();
            canvas.clip_path(&sk_path, None, true);
            // The image fits the path's bounding box; the clip restricts it
            // to the actual path shape. `crop` carries Figma's CROP-mode
            // `imageTransform` (a normalized sub-rect of the asset); the
            // renderer's `crop_to_pixels`/`blit_sk_image` already sample only
            // that window before `mode` fits it into the rect — identical to
            // the BitmapNode crop path. No tint on the vector-fill path
            // (BitmapNode is the tinting surface).
            let local_size = [bounds.width(), bounds.height()];
            // Translate into the bounds origin so a non-origin path (e.g.
            // a rect at (10, 20)) fills correctly.
            canvas.translate((bounds.min_x as f32, bounds.min_y as f32));
            let crop_rect = crop.as_deref().copied();
            let drawn = draw_image_cached(
                canvas,
                ctx.cache,
                *asset,
                || resolver.and_then(|r| r.resolve(*asset)),
                local_size,
                crop_rect,
                *mode,
                None,
                *opacity,
            );
            canvas.restore();
            if drawn {
                ctx.metrics.nodes_drawn += 1;
                continue;
            }
            // Fall through to the placeholder paint for a missing asset.
        }
        let paint = fill_to_paint(fill, local_bounds_f32);
        canvas.draw_path(&sk_path, &paint);
        ctx.metrics.nodes_drawn += 1;
    }
    if path_is_rect(path) {
        stroke_box_path(
            canvas,
            local_bounds_f32,
            corner_radius,
            corner_radii,
            0.0,
            strokes,
            ctx,
        );
    } else {
        stroke_sk_path(canvas, &sk_path, strokes, local_bounds_f32, ctx);
    }
}

/// Draw the marker for an instance that genuinely could not be resolved (its
/// component is dangling — the master was deleted or never loaded).
///
/// Unlike [`draw_placeholder`], this paints **no fill** — only a faint 1px gray
/// dashed outline of the instance's `local_size`. The old translucent-blue block
/// (`rgba(120, 170, 255, 90)`) was a visual eyesore that read as design intent
/// and swamped real content on a page full of instances; a hairline outline
/// keeps a broken instance *locatable* in the editor without painting over the
/// composition. A zero-area box draws nothing.
pub(crate) fn draw_unresolved_outline(canvas: &Canvas, size: [f64; 2]) {
    if size[0] <= 0.0 || size[1] <= 0.0 {
        return;
    }
    let rect = Rect::from_xywh(0.0, 0.0, size[0] as f32, size[1] as f32);
    let mut outline = Paint::default();
    // Faint neutral gray, low alpha — visible against both light and dark
    // canvases but never mistaken for a filled shape.
    outline.set_color(to_sk_color(Color::rgba(140, 140, 140, 120)));
    outline.set_anti_alias(true);
    outline.set_style(skia_safe::paint::Style::Stroke);
    outline.set_stroke_width(1.0);
    if let Some(effect) = skia_safe::PathEffect::dash(&[4.0, 3.0], 0.0) {
        outline.set_path_effect(effect);
    }
    canvas.draw_rect(rect, &outline);
}

pub(crate) fn draw_placeholder(canvas: &Canvas, size: [f64; 2], color: Color) {
    let rect = Rect::from_xywh(0.0, 0.0, size[0] as f32, size[1] as f32);
    let mut paint = Paint::default();
    paint.set_color(to_sk_color(color));
    paint.set_anti_alias(true);
    canvas.draw_rect(rect, &paint);

    // Dashed outline so the placeholder reads as "unrendered" rather than
    // "design intent."
    let mut outline = Paint::default();
    outline.set_color(to_sk_color(Color::rgba(0, 0, 0, 180)));
    outline.set_style(skia_safe::paint::Style::Stroke);
    outline.set_stroke_width(1.0);
    if let Some(effect) = skia_safe::PathEffect::dash(&[6.0, 4.0], 0.0) {
        outline.set_path_effect(effect);
    }
    canvas.draw_rect(rect, &outline);
}

pub(crate) fn bounds_to_f32(b: &Bounds) -> [f32; 4] {
    [
        b.min_x as f32,
        b.min_y as f32,
        b.width() as f32,
        b.height() as f32,
    ]
}

#[cfg(test)]
mod icon_rect_tests {
    use super::*;
    #[test]
    fn real_icons_are_not_rects_but_a_frame_is() {
        // Regression: every built-in icon must report NOT-a-rect so the layout
        // resizer preserves its geometry instead of flattening it to a square.
        let plus = fanta_doc::PathData::from_svg_d("M 12 5 V 19 M 5 12 H 19").unwrap();
        let sparkles = fanta_doc::PathData::from_svg_d(
            "M 12 3 L 14 9 L 20 11 L 14 13 L 12 19 L 10 13 L 4 11 L 10 9 Z M 19 3 V 7 M 17 5 H 21",
        )
        .unwrap();
        let frame = fanta_doc::PathData::from_svg_d("M 5 5 H 19 V 19 H 5 Z").unwrap();
        assert!(!path_is_rect(&plus));
        assert!(!path_is_rect(&sparkles));
        assert!(path_is_rect(&frame));
    }
}
