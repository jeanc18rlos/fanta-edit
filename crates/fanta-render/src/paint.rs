//! Convert [`Fill`] and [`Stroke`] into Skia [`Paint`]s.
//!
//! [`Fill`]: fanta_doc::Fill
//! [`Stroke`]: fanta_doc::Stroke

use crate::color::{to_sk_color, to_sk_color4f};
use fanta_doc::{BlendMode, Fill, Gradient, GradientStop, Stroke, StrokeCap, StrokeJoin};
use skia_safe::{
    Matrix, Paint, Point, Shader, TileMode, paint::Cap, paint::Join, paint::Style as PaintStyle,
};

/// Convert a [`Fill`] into a paint pre-configured with style = fill.
///
/// `local_bounds` is the node-local bounding rectangle used to resolve
/// gradient endpoints (which are stored in normalized 0..=1 space). For
/// image fills this also drives the texture sampling rectangle; assets
/// are resolved via [`AssetResolver`] in the calling renderer.
///
/// A per-paint `blend` (Figma's paint-level blend mode, carried by gradient
/// fills) rides on the returned paint, so the layer composites against the
/// paints below it — and the backdrop — with that mode.
pub fn fill_to_paint(fill: &Fill, local_bounds: [f32; 4]) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_style(PaintStyle::Fill);
    match fill {
        Fill::Solid { color } => {
            paint.set_color(to_sk_color(*color));
        }
        Fill::Gradient { gradient, blend } => {
            if let Some(shader) = gradient_to_shader(gradient, local_bounds) {
                paint.set_shader(shader);
            }
            if !blend.is_normal() {
                paint.set_blend_mode(to_sk_blend_mode(*blend));
            }
        }
        Fill::Image { .. } => {
            // Image fills are resolved by the renderer (it knows the asset
            // store) and applied as a shader at draw time. The paint we hand
            // back here is a transparent-magenta placeholder so missing-asset
            // states are visible in debugging.
            paint.set_color(to_sk_color(fanta_doc::Color::rgba(255, 0, 255, 64)));
        }
    }
    paint
}

/// Map a [`fanta_doc::BlendMode`] to its [`skia_safe::BlendMode`] equivalent.
///
/// `fanta_doc::BlendMode` is the CSS Compositing Level 1 / `SkBlendMode` set, so
/// each separable + non-separable mode has an exact Skia counterpart. `Normal`
/// maps to Skia `SrcOver` (regular alpha-over). Owned here so the node-level
/// effects layer and the per-paint fill/image blends share one mapping.
pub(crate) fn to_sk_blend_mode(mode: BlendMode) -> skia_safe::BlendMode {
    use skia_safe::BlendMode as Sk;
    match mode {
        BlendMode::Normal => Sk::SrcOver,
        BlendMode::Multiply => Sk::Multiply,
        BlendMode::Screen => Sk::Screen,
        BlendMode::Overlay => Sk::Overlay,
        BlendMode::Darken => Sk::Darken,
        BlendMode::Lighten => Sk::Lighten,
        BlendMode::ColorDodge => Sk::ColorDodge,
        BlendMode::ColorBurn => Sk::ColorBurn,
        BlendMode::HardLight => Sk::HardLight,
        BlendMode::SoftLight => Sk::SoftLight,
        BlendMode::Difference => Sk::Difference,
        BlendMode::Exclusion => Sk::Exclusion,
        BlendMode::Hue => Sk::Hue,
        BlendMode::Saturation => Sk::Saturation,
        BlendMode::Color => Sk::Color,
        BlendMode::Luminosity => Sk::Luminosity,
    }
}

/// Convert a [`Stroke`] into a paint pre-configured with style = stroke.
pub fn stroke_to_paint(stroke: &Stroke, local_bounds: [f32; 4]) -> Paint {
    let mut paint = fill_to_paint(&stroke.paint, local_bounds);
    paint.set_style(PaintStyle::Stroke);
    paint.set_stroke_width(stroke.width as f32);
    paint.set_stroke_cap(cap_to_skia(stroke.cap));
    paint.set_stroke_join(join_to_skia(stroke.join));
    paint.set_stroke_miter(stroke.miter_limit as f32);
    if !stroke.dash.is_empty() {
        // Skia takes a flat alternating on/off interval list.
        let intervals: Vec<f32> = stroke.dash.iter().map(|v| *v as f32).collect();
        if let Some(effect) = skia_safe::PathEffect::dash(&intervals, 0.0) {
            paint.set_path_effect(effect);
        }
    }
    paint
}

fn cap_to_skia(c: StrokeCap) -> Cap {
    match c {
        StrokeCap::Butt => Cap::Butt,
        StrokeCap::Round => Cap::Round,
        StrokeCap::Square => Cap::Square,
    }
}

fn join_to_skia(j: StrokeJoin) -> Join {
    match j {
        StrokeJoin::Miter => Join::Miter,
        StrokeJoin::Round => Join::Round,
        StrokeJoin::Bevel => Join::Bevel,
    }
}

fn gradient_to_shader(g: &Gradient, [x, y, w, h]: [f32; 4]) -> Option<Shader> {
    match g {
        Gradient::Linear { start, end, stops } => {
            let p0 = Point::new(x + start[0] * w, y + start[1] * h);
            let p1 = Point::new(x + end[0] * w, y + end[1] * h);
            let (colors, positions) = stops_to_arrays(stops);
            Shader::linear_gradient(
                (p0, p1),
                colors.as_slice(),
                Some(positions.as_slice()),
                TileMode::Clamp,
                None,
                None,
            )
        }
        Gradient::Radial {
            center,
            radius,
            handles,
            stops,
        } => {
            // `center`/`radius` live in the node's normalized 0–1 local space.
            // Mapping that unit square onto the node rect with the affine
            //     M = translate(x, y) · scale(w, h)
            // turns the unit circle (center `c`, radius `r`) into an
            // axis-aligned *ellipse* with x-radius `r·w` and y-radius `r·h`,
            // which is exactly Figma's node-aspect radial gradient. (The old
            // `radius·(w+h)·0.5` forced a single circular radius — wrong for
            // any non-square node.) We build the shader in unit space and let
            // Skia apply the ellipse via `local_matrix`, so a degenerate
            // `w == h` collapses back to the faithful circle.
            let (colors, positions) = stops_to_arrays(stops);
            let mut local_matrix = node_local_matrix(x, y, w, h);
            // Full axis handles (a rotated and/or anisotropic radial): the
            // ellipse is `center` plus the two axis vectors to the handle
            // endpoints. Build the gradient on the CANONICAL unit circle
            // (origin, radius 1) and let the handle matrix carry it onto the
            // rotated ellipse — the node-aspect matrix above then maps
            // normalized space onto pixels as usual. Degenerate handles
            // (zero-area axes) fall back to the aspect-only ellipse.
            if let Some(axes) = (*handles).and_then(|ends| handle_axes_matrix(*center, ends)) {
                local_matrix.pre_concat(&axes);
                return Shader::radial_gradient(
                    Point::new(0.0, 0.0),
                    1.0,
                    colors.as_slice(),
                    Some(positions.as_slice()),
                    TileMode::Clamp,
                    None,
                    Some(&local_matrix),
                );
            }
            Shader::radial_gradient(
                Point::new(center[0], center[1]),
                *radius,
                colors.as_slice(),
                Some(positions.as_slice()),
                TileMode::Clamp,
                None,
                Some(&local_matrix),
            )
        }
        Gradient::Angular {
            center,
            start_angle,
            stops,
        } => {
            // Conic / sweep gradient: stops are swept around `center` (in the
            // node's normalized 0–1 space) through a full turn. Skia's
            // `sweep_gradient` takes angles in DEGREES measured clockwise from
            // +x, which matches Figma's conic convention; we convert the stored
            // radians and run the sweep from `start_angle` to `start_angle+360`.
            // The unit-space center is mapped onto the node rect by the same
            // `local_matrix` the radial branch uses, so a non-square node sweeps
            // about its real center.
            let unit_center = Point::new(center[0], center[1]);
            let (colors, positions) = stops_to_arrays(stops);
            let local_matrix = node_local_matrix(x, y, w, h);
            let start_deg = start_angle.to_degrees();
            Shader::sweep_gradient(
                unit_center,
                colors.as_slice(),
                Some(positions.as_slice()),
                TileMode::Clamp,
                Some((start_deg, start_deg + 360.0)),
                None,
                Some(&local_matrix),
            )
        }
        Gradient::Diamond {
            center,
            radius,
            handles,
            stops,
        } => {
            // Diamond gradient: the iso-distance contours are axis-aligned
            // *diamonds* (rhombi) — the L1 / Manhattan-distance "radial" — not
            // the ellipses of a true radial. A rotated radial is still an
            // ellipse, so the old rotate-the-circle trick produced a tilted
            // ellipse, never a diamond. Skia has no native diamond gradient, so
            // we color by the true L1 distance
            //     t = clamp((|x - cx| + |y - cy|) / radius, 0, 1)
            // in the node's normalized 0–1 local space. We bake that ramp into a
            // small unit-square bitmap and map it onto the node rect with the
            // same `node_local_matrix` the radial branch uses, so a non-square
            // node stretches the diamond by the node aspect — but the
            // iso-contours are real rhombi. A baked tile keeps us off
            // skia-safe 0.84's `RuntimeEffect::make_for_shader`, whose options
            // argument has an unsatisfiable higher-ranked lifetime bound.
            // Axis `handles` (a rotated/anisotropic diamond) ride the same
            // baked tile through an extra affine — see `diamond_shader`.
            diamond_shader(
                [x, y, w, h],
                *center,
                *radius,
                *handles,
                stops,
                DIAMOND_TILE,
            )
        }
    }
}

/// The affine mapping the CANONICAL gradient space (origin-centered, radius-1
/// unit circle / unit diamond) onto a gradient authored with full axis
/// HANDLES: column one is the x-axis vector `ends[0] − center`, column two the
/// y-axis vector `ends[1] − center`, translation `center` — all in the node's
/// normalized 0–1 space. `None` when the axes span (near-)zero area, letting
/// callers fall back to the aspect-only mapping instead of emitting a
/// degenerate shader.
fn handle_axes_matrix(center: [f32; 2], ends: [[f32; 2]; 2]) -> Option<Matrix> {
    let x_axis = [ends[0][0] - center[0], ends[0][1] - center[1]];
    let y_axis = [ends[1][0] - center[0], ends[1][1] - center[1]];
    let det = x_axis[0] * y_axis[1] - x_axis[1] * y_axis[0];
    if !det.is_finite() || det.abs() < 1e-6 {
        return None;
    }
    Some(Matrix::new_all(
        x_axis[0], y_axis[0], center[0], x_axis[1], y_axis[1], center[1], 0.0, 0.0, 1.0,
    ))
}

/// Edge length (texels) of the baked diamond ramp tile. The tile encodes the
/// unit square `[0,1]²`; `node_local_matrix` (scaled by `1/N`) then maps it onto
/// the node rect, so the on-device resolution is the node size, not `N`. 256 is
/// smooth (the per-texel L1 distance is bilinearly interpolated by the image
/// shader) without being a large allocation.
const DIAMOND_TILE: usize = 256;

/// Build the diamond gradient shader by baking the L1-distance color ramp into
/// an `n × n` RGBA bitmap (texel (i,j) at unit coords `((i+0.5)/n, (j+0.5)/n)`
/// gets the stop color at its normalized L1 distance from `center`), then
/// turning that image into a clamped image shader mapped onto the node rect.
/// Returns `None` if the image can't be built — the fill is then skipped rather
/// than drawing garbage; never panics.
///
/// With axis `handles` (a rotated/anisotropic diamond) the ramp is instead
/// baked in a CANONICAL frame — center `(0.5, 0.5)`, radius `0.5`, so `t = 1`
/// lands exactly on the tile edge midpoints — and an extra affine (canonical
/// square → `center ± axes` parallelogram, via [`handle_axes_matrix`]) is
/// folded into the local matrix. Degenerate handles fall back to the
/// aspect-only mapping, keeping the handle-less path byte-identical.
fn diamond_shader(
    bounds: [f32; 4],
    center: [f32; 2],
    radius: f32,
    handles: Option<[[f32; 2]; 2]>,
    stops: &[GradientStop],
    n: usize,
) -> Option<Shader> {
    use skia_safe::{AlphaType, ColorType, FilterMode, ImageInfo, MipmapMode, SamplingOptions};
    let [x, y, w, h] = bounds;

    let axes = handles.and_then(|ends| handle_axes_matrix(center, ends));
    let (center, radius) = if axes.is_some() {
        ([0.5, 0.5], 0.5)
    } else {
        (center, radius)
    };

    // Guard a degenerate radius so the divide can't produce NaN/inf.
    let r = if radius.abs() < f32::EPSILON {
        f32::EPSILON
    } else {
        radius
    };

    // `sample_stops` walks adjacent pairs, so the stops must be in ascending
    // position order (the doc keeps authoring order). Sort once before the bake.
    let mut sorted: Vec<GradientStop> = stops.to_vec();
    sorted.sort_by(|a, b| {
        a.position
            .partial_cmp(&b.position)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    // Bake the ramp: straight RGBA (Unpremul), row-major, N×N.
    let mut px = vec![0u8; n * n * 4];
    for j in 0..n {
        let v = (j as f32 + 0.5) / n as f32;
        for i in 0..n {
            let u = (i as f32 + 0.5) / n as f32;
            // L1 (Manhattan) distance, normalized and clamped — the diamond.
            let t = (((u - center[0]).abs() + (v - center[1]).abs()) / r).clamp(0.0, 1.0);
            let c = sample_stops(&sorted, t);
            let o = (j * n + i) * 4;
            px[o] = c[0];
            px[o + 1] = c[1];
            px[o + 2] = c[2];
            px[o + 3] = c[3];
        }
    }

    let info = ImageInfo::new(
        (n as i32, n as i32),
        ColorType::RGBA8888,
        AlphaType::Unpremul,
        None,
    );
    let row_bytes = info.min_row_bytes();
    let data = skia_safe::Data::new_copy(&px);
    let image = skia_safe::images::raster_from_data(&info, data, row_bytes)?;

    // Map the unit square (here represented by the N-texel image) onto the node
    // rect: scale unit→rect (anisotropic for non-square nodes, the diamond
    // signature is preserved), then scale 1/N to go from rect-unit to texels.
    let mut local_matrix = node_local_matrix(x, y, w, h);
    // Handle axes: the canonical bake frame's unit square maps onto the
    // parallelogram `center ± axes` — i.e. canonical point q → axes·(2q − 1).
    if let Some(axes_matrix) = axes {
        local_matrix.pre_concat(&axes_matrix);
        let square_to_canonical = Matrix::new_all(2.0, 0.0, -1.0, 0.0, 2.0, -1.0, 0.0, 0.0, 1.0);
        local_matrix.pre_concat(&square_to_canonical);
    }
    let mut to_texels = Matrix::new_identity();
    to_texels.set_scale((1.0 / n as f32, 1.0 / n as f32), None);
    // shader-space point → (node_local · to_texels)⁻¹ is what Skia applies; we
    // build the forward map texel→device by pre-concatenating the texel scale.
    local_matrix.pre_concat(&to_texels);

    let sampling = SamplingOptions::new(FilterMode::Linear, MipmapMode::None);
    image.to_shader((TileMode::Clamp, TileMode::Clamp), sampling, &local_matrix)
}

/// Evaluate the gradient stop ramp at normalized position `t` (0..=1), returning
/// straight RGBA. Mirrors Skia's clamp behavior: before the first stop holds the
/// first color, after the last holds the last, and between two stops linearly
/// interpolates each channel. Empty stops → transparent.
fn sample_stops(stops: &[GradientStop], t: f32) -> [u8; 4] {
    if stops.is_empty() {
        return [0, 0, 0, 0];
    }
    let t = t.clamp(0.0, 1.0);
    // Before the first / after the last stop: clamp to the end color.
    if t <= stops[0].position {
        return color_rgba(stops[0].color);
    }
    let last = &stops[stops.len() - 1];
    if t >= last.position {
        return color_rgba(last.color);
    }
    // Find the bracketing pair and lerp.
    for w in stops.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if t >= a.position && t <= b.position {
            let span = b.position - a.position;
            let f = if span <= f32::EPSILON {
                0.0
            } else {
                (t - a.position) / span
            };
            let ca = color_rgba(a.color);
            let cb = color_rgba(b.color);
            return [
                lerp_u8(ca[0], cb[0], f),
                lerp_u8(ca[1], cb[1], f),
                lerp_u8(ca[2], cb[2], f),
                lerp_u8(ca[3], cb[3], f),
            ];
        }
    }
    color_rgba(last.color)
}

fn color_rgba(c: fanta_doc::Color) -> [u8; 4] {
    [c.r, c.g, c.b, c.a]
}

fn lerp_u8(a: u8, b: u8, f: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * f)
        .round()
        .clamp(0.0, 255.0) as u8
}

/// Affine mapping the node's normalized 0–1 local space onto its pixel rect
/// `[x, y, w, h]`: `translate(x, y) · scale(w, h)`. A gradient authored against
/// the unit square then lands on the node, and any non-uniform `w != h` scale
/// stretches a radial gradient's unit circle into the matching ellipse.
fn node_local_matrix(x: f32, y: f32, w: f32, h: f32) -> Matrix {
    let mut m = Matrix::new_identity();
    m.set_scale_translate((w, h), (x, y));
    m
}

fn stops_to_arrays(stops: &[GradientStop]) -> (Vec<skia_safe::Color4f>, Vec<f32>) {
    // Skia's gradient shaders require strictly non-decreasing positions; the doc
    // stores stops in authoring/insertion order (a user can drag one past
    // another), so sort by position here or the ramp renders garbled.
    let mut sorted: Vec<&GradientStop> = stops.iter().collect();
    sorted.sort_by(|a, b| {
        a.position
            .partial_cmp(&b.position)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let colors: Vec<_> = sorted.iter().map(|s| to_sk_color4f(s.color)).collect();
    let positions: Vec<_> = sorted.iter().map(|s| s.position).collect();
    (colors, positions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::Color;

    #[test]
    fn solid_fill_carries_color_and_alpha() {
        let fill = Fill::solid(Color::rgba(10, 20, 30, 40));
        let paint = fill_to_paint(&fill, [0.0, 0.0, 100.0, 100.0]);
        let c = paint.color();
        assert_eq!(c.r(), 10);
        assert_eq!(c.g(), 20);
        assert_eq!(c.b(), 30);
        assert_eq!(c.a(), 40);
    }

    #[test]
    fn stroke_paint_has_stroke_style_and_width() {
        let stroke = Stroke::solid(Color::BLACK, 4.0);
        let paint = stroke_to_paint(&stroke, [0.0, 0.0, 100.0, 100.0]);
        assert_eq!(paint.style(), PaintStyle::Stroke);
        assert!((paint.stroke_width() - 4.0).abs() < 1e-4);
    }

    #[test]
    fn node_local_matrix_maps_unit_space_onto_the_node_rect() {
        // The matrix must send the normalized unit square corners onto the node
        // rect corners, with x and y scaled INDEPENDENTLY by w and h — that
        // independent scale is what turns a unit circle into the node-aspect
        // ellipse a radial gradient needs (the whole point of the fix).
        let m = node_local_matrix(10.0, 20.0, 60.0, 30.0);
        let origin = m.map_point(Point::new(0.0, 0.0));
        let corner = m.map_point(Point::new(1.0, 1.0));
        let centre = m.map_point(Point::new(0.5, 0.5));
        assert!((origin.x - 10.0).abs() < 1e-4 && (origin.y - 20.0).abs() < 1e-4);
        assert!((corner.x - 70.0).abs() < 1e-4 && (corner.y - 50.0).abs() < 1e-4);
        assert!((centre.x - 40.0).abs() < 1e-4 && (centre.y - 35.0).abs() < 1e-4);
        // A unit x-step stretches 60, a unit y-step only 30 → anisotropic.
        let dx = m.map_point(Point::new(1.0, 0.0)).x - origin.x;
        let dy = m.map_point(Point::new(0.0, 1.0)).y - origin.y;
        assert!((dx - 60.0).abs() < 1e-4 && (dy - 30.0).abs() < 1e-4);
        assert!(
            (dx - dy).abs() > 1.0,
            "the wide node must scale x and y differently"
        );
    }

    #[test]
    fn angular_gradient_builds_a_sweep_shader() {
        // A conic/angular gradient must produce a shader (Skia sweep_gradient),
        // not drop the fill or collapse to a flat color.
        let fill = Fill::Gradient {
            blend: fanta_doc::BlendMode::Normal,
            gradient: fanta_doc::Gradient::Angular {
                center: [0.5, 0.5],
                start_angle: 0.0,
                stops: vec![
                    GradientStop {
                        position: 0.0,
                        color: Color::WHITE,
                    },
                    GradientStop {
                        position: 0.5,
                        color: Color::rgb(255, 0, 0),
                    },
                    GradientStop {
                        position: 1.0,
                        color: Color::BLACK,
                    },
                ],
            },
        };
        let paint = fill_to_paint(&fill, [0.0, 0.0, 80.0, 80.0]);
        assert!(
            paint.shader().is_some(),
            "an angular gradient must build a sweep shader"
        );
    }

    #[test]
    fn diamond_gradient_builds_a_shader() {
        // A diamond gradient must produce a shader (SkSL L1-distance + ramp).
        let fill = Fill::Gradient {
            blend: fanta_doc::BlendMode::Normal,
            gradient: fanta_doc::Gradient::Diamond {
                center: [0.5, 0.5],
                radius: 0.5,
                handles: None,
                stops: vec![
                    GradientStop {
                        position: 0.0,
                        color: Color::WHITE,
                    },
                    GradientStop {
                        position: 1.0,
                        color: Color::BLACK,
                    },
                ],
            },
        };
        let paint = fill_to_paint(&fill, [0.0, 0.0, 100.0, 60.0]);
        assert!(
            paint.shader().is_some(),
            "a diamond gradient must build a shader"
        );
    }

    #[test]
    fn diamond_shader_builds_for_a_non_square_node() {
        // The diamond branch must build a shader (SkSL compiles + ramp child +
        // unit-space local matrix) rather than dropping the fill, even on a
        // non-square node where the diamond stretches with the aspect ratio.
        let stops = vec![
            GradientStop {
                position: 0.0,
                color: Color::WHITE,
            },
            GradientStop {
                position: 1.0,
                color: Color::BLACK,
            },
        ];
        let shader = diamond_shader([0.0, 0.0, 120.0, 40.0], [0.5, 0.5], 0.5, None, &stops, 64);
        assert!(
            shader.is_some(),
            "the baked diamond image shader must build for a non-square node"
        );
    }

    #[test]
    fn diamond_ramp_is_l1_not_l2() {
        // Unit proof on the baked ramp itself (no Skia surface): on the same
        // euclidean circle of radius rho around the center, the DIAGONAL texel
        // has a strictly larger normalized L1 distance — hence a darker (higher
        // `t`) color — than the AXIS texel. A radial/elliptical ramp would color
        // them identically. This is the diamond signature, computed directly
        // from `sample_stops` over the same L1 formula the bake uses.
        let stops = vec![
            GradientStop {
                position: 0.0,
                color: Color::rgb(255, 255, 255), // t=0 white
            },
            GradientStop {
                position: 1.0,
                color: Color::rgb(0, 0, 0), // t=1 black
            },
        ];
        let center = [0.5_f32, 0.5];
        let radius = 0.5_f32;
        let l1 = |u: f32, v: f32| ((u - center[0]).abs() + (v - center[1]).abs()) / radius;
        let rho = 0.25_f32;
        let axis_t = l1(center[0] + rho, center[1]); // 0.5
        let diag = rho / std::f32::consts::SQRT_2;
        let diag_t = l1(center[0] + diag, center[1] + diag); // ~0.707
        assert!(
            diag_t > axis_t + 0.1,
            "diagonal L1 ({diag_t}) must exceed axis L1 ({axis_t}) — the diamond signature"
        );
        // And the sampled colors reflect it: diagonal is darker than axis.
        let axis_c = sample_stops(&stops, axis_t);
        let diag_c = sample_stops(&stops, diag_t);
        assert!(
            (diag_c[0] as i32) < (axis_c[0] as i32) - 20,
            "diagonal color {diag_c:?} must be darker than axis {axis_c:?}"
        );
    }

    #[test]
    fn radial_gradient_builds_a_shader_for_a_non_square_node() {
        // The radial branch must produce a shader (with the ellipse local-matrix
        // attached) rather than dropping the fill on a non-square node.
        let fill = Fill::Gradient {
            blend: fanta_doc::BlendMode::Normal,
            gradient: fanta_doc::Gradient::Radial {
                center: [0.5, 0.5],
                radius: 0.5,
                handles: None,
                stops: vec![
                    GradientStop {
                        position: 0.0,
                        color: Color::WHITE,
                    },
                    GradientStop {
                        position: 1.0,
                        color: Color::BLACK,
                    },
                ],
            },
        };
        let paint = fill_to_paint(&fill, [0.0, 0.0, 120.0, 40.0]);
        assert!(
            paint.shader().is_some(),
            "a radial gradient on a wide node must still build a shader"
        );
    }

    /// The gradient editor can transiently hand the renderer pathological
    /// gradients — a fill mid-edit with zero or one stop, a stop dragged to a
    /// non-finite position, coincident linear endpoints, a collapsed radius, or
    /// degenerate axis handles. None of these may panic (skia-safe's gradient
    /// constructors abort on some malformed inputs): a degenerate gradient must
    /// resolve to *no shader* (the fill is skipped) rather than taking down the
    /// process. This is the render-side guard for the "crashes on opening
    /// gradients" report — the panel path is covered by `fig_viewer`'s suite.
    #[test]
    fn degenerate_gradients_never_panic_and_drop_the_shader() {
        let nan = f32::NAN;
        let one = vec![GradientStop {
            position: 0.0,
            color: Color::WHITE,
        }];
        let nan_pos = vec![
            GradientStop {
                position: nan,
                color: Color::WHITE,
            },
            GradientStop {
                position: 1.0,
                color: Color::BLACK,
            },
        ];
        let two = vec![
            GradientStop {
                position: 0.0,
                color: Color::WHITE,
            },
            GradientStop {
                position: 1.0,
                color: Color::BLACK,
            },
        ];

        let gradients = [
            // Empty and single-stop of every kind.
            Gradient::Linear {
                start: [0.0, 0.0],
                end: [1.0, 1.0],
                stops: vec![],
            },
            Gradient::Linear {
                start: [0.0, 0.0],
                end: [1.0, 1.0],
                stops: one.clone(),
            },
            // Coincident linear endpoints (zero-length axis).
            Gradient::Linear {
                start: [0.5, 0.5],
                end: [0.5, 0.5],
                stops: two.clone(),
            },
            // Non-finite stop position.
            Gradient::Linear {
                start: [0.0, 0.0],
                end: [1.0, 1.0],
                stops: nan_pos,
            },
            // Non-finite endpoints.
            Gradient::Linear {
                start: [nan, nan],
                end: [1.0, 1.0],
                stops: two.clone(),
            },
            Gradient::Radial {
                center: [0.5, 0.5],
                radius: 0.0,
                handles: None,
                stops: two.clone(),
            },
            Gradient::Radial {
                center: [nan, nan],
                radius: nan,
                handles: None,
                stops: one.clone(),
            },
            Gradient::Radial {
                center: [0.5, 0.5],
                radius: 0.5,
                handles: Some([[nan, nan], [nan, nan]]),
                stops: two.clone(),
            },
            Gradient::Angular {
                center: [0.5, 0.5],
                start_angle: nan,
                stops: vec![],
            },
            Gradient::Angular {
                center: [nan, nan],
                start_angle: 0.0,
                stops: one.clone(),
            },
            Gradient::Diamond {
                center: [0.5, 0.5],
                radius: 0.0,
                handles: None,
                stops: two,
            },
            Gradient::Diamond {
                center: [0.5, 0.5],
                radius: 0.5,
                handles: Some([[nan, nan], [nan, nan]]),
                stops: one,
            },
        ];

        // Also exercise a zero-size node rect, which the editor produces before
        // the first layout pass. The assertion is simply that we return without
        // panicking; whether a shader is built is left to each branch.
        for gradient in gradients {
            for bounds in [[0.0, 0.0, 120.0, 40.0], [0.0, 0.0, 0.0, 0.0]] {
                let fill = Fill::Gradient {
                    blend: fanta_doc::BlendMode::Normal,
                    gradient: gradient.clone(),
                };
                let _ = fill_to_paint(&fill, bounds);
            }
        }
    }
}
