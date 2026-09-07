//! Arc-to-cubic-Bezier approximation helpers shared by `ellipse_arc`
//! (core) and SVG `A` command parsing.

use super::PathData;

/// Append the cubic-Bézier approximation of an elliptical arc on the ellipse
/// centered at `(cx, cy)` with radii `(rx, ry)`, sweeping from `start_rad` to
/// `end_rad` (signed — `end < start` sweeps the other way). The current point is
/// assumed to already be at the arc's start; only cubic segments are emitted (no
/// leading Move/Line), so the caller controls how the arc joins the rest of the
/// contour.
///
/// The sweep is split into pieces of at most a quarter turn so each piece is a
/// faithful single cubic, with control-point handle length
/// `k = (4/3)·tan(Δ/4)` — the standard arc-to-Bézier formula, which collapses to
/// the KAPPA constant at Δ = π/2.
pub(super) fn append_arc_cubics(
    p: &mut PathData,
    cx: f64,
    cy: f64,
    rx: f64,
    ry: f64,
    start_rad: f64,
    end_rad: f64,
) {
    use std::f64::consts::FRAC_PI_2;
    let total = end_rad - start_rad;
    if total == 0.0 {
        return;
    }
    // Split into ≤90° pieces. A tiny epsilon before `ceil` keeps a span that is
    // a quarter turn within float rounding (e.g. an f32 π/2 widened to f64) as a
    // single cubic instead of spuriously splitting into two.
    let pieces = (total.abs() / FRAC_PI_2 - 1e-6).ceil().max(1.0) as usize;
    let delta = total / pieces as f64;
    let k = (4.0 / 3.0) * (delta / 4.0).tan();
    let mut a = start_rad;
    for _ in 0..pieces {
        let b = a + delta;
        let (ca, sa) = (a.cos(), a.sin());
        let (cb, sb) = (b.cos(), b.sin());
        // Endpoints on the ellipse.
        let p0 = [cx + rx * ca, cy + ry * sa];
        let p3 = [cx + rx * cb, cy + ry * sb];
        // Control points: P0 + k·tangent(P0), P3 − k·tangent(P3). The unit
        // tangent to the parametric ellipse at angle θ is (−rx·sinθ, ry·cosθ).
        let c1 = [p0[0] - k * rx * sa, p0[1] + k * ry * ca];
        let c2 = [p3[0] + k * rx * sb, p3[1] - k * ry * cb];
        p.cubic_to(c1[0], c1[1], c2[0], c2[1], p3[0], p3[1]);
        a = b;
    }
}

/// Append the cubic-Bézier approximation of an SVG `A`/`a` elliptical-arc command
/// to `p`, whose current point is `start`. Implements the SVG spec F.6
/// endpoint→center conversion: radii `(rx, ry)`, x-axis rotation `phi` (radians),
/// the large-arc/sweep flags, and the `end` point fully determine the arc. Radii
/// are corrected up if too small (F.6.6); a degenerate arc falls back to a line.
/// Handles a rotated ellipse (`phi != 0`), unlike [`append_arc_cubics`].
#[allow(clippy::too_many_arguments)]
pub(super) fn append_svg_arc(
    p: &mut PathData,
    start: [f64; 2],
    mut rx: f64,
    mut ry: f64,
    phi: f64,
    large_arc: bool,
    sweep: bool,
    end: [f64; 2],
) {
    use std::f64::consts::{FRAC_PI_2, PI};
    rx = rx.abs();
    ry = ry.abs();
    if rx < 1e-12 || ry < 1e-12 || (start[0] == end[0] && start[1] == end[1]) {
        p.line_to(end[0], end[1]);
        return;
    }
    let (cosp, sinp) = (phi.cos(), phi.sin());
    // F.6.5 step 1: midpoint-relative, de-rotated.
    let dx = (start[0] - end[0]) / 2.0;
    let dy = (start[1] - end[1]) / 2.0;
    let x1p = cosp * dx + sinp * dy;
    let y1p = -sinp * dx + cosp * dy;
    // F.6.6: ensure the radii are large enough.
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }
    let (rx2, ry2) = (rx * rx, ry * ry);
    // F.6.5 step 2: center in the de-rotated frame.
    let num = (rx2 * ry2 - rx2 * y1p * y1p - ry2 * x1p * x1p).max(0.0);
    let den = rx2 * y1p * y1p + ry2 * x1p * x1p;
    let mut coef = if den > 0.0 { (num / den).sqrt() } else { 0.0 };
    if large_arc == sweep {
        coef = -coef;
    }
    let cxp = coef * (rx * y1p) / ry;
    let cyp = coef * -(ry * x1p) / rx;
    // F.6.5 step 3: center in user space.
    let cx = cosp * cxp - sinp * cyp + (start[0] + end[0]) / 2.0;
    let cy = sinp * cxp + cosp * cyp + (start[1] + end[1]) / 2.0;
    // F.6.5 step 4: start angle and sweep.
    let angle = |ux: f64, uy: f64, vx: f64, vy: f64| -> f64 {
        let dot = ux * vx + uy * vy;
        let len = ((ux * ux + uy * uy) * (vx * vx + vy * vy)).sqrt();
        let mut a = if len > 0.0 {
            (dot / len).clamp(-1.0, 1.0).acos()
        } else {
            0.0
        };
        if ux * vy - uy * vx < 0.0 {
            a = -a;
        }
        a
    };
    let theta1 = angle(1.0, 0.0, (x1p - cxp) / rx, (y1p - cyp) / ry);
    let mut dtheta = angle(
        (x1p - cxp) / rx,
        (y1p - cyp) / ry,
        (-x1p - cxp) / rx,
        (-y1p - cyp) / ry,
    );
    if !sweep && dtheta > 0.0 {
        dtheta -= 2.0 * PI;
    } else if sweep && dtheta < 0.0 {
        dtheta += 2.0 * PI;
    }
    // Emit ≤90° cubic pieces; each point/tangent is on the rotated ellipse.
    let pieces = (dtheta.abs() / FRAC_PI_2 - 1e-6).ceil().max(1.0) as usize;
    let delta = dtheta / pieces as f64;
    let k = (4.0 / 3.0) * (delta / 4.0).tan();
    let on_ellipse = |t: f64| -> ([f64; 2], [f64; 2]) {
        let (ct, st) = (t.cos(), t.sin());
        let point = [
            cosp * (rx * ct) - sinp * (ry * st) + cx,
            sinp * (rx * ct) + cosp * (ry * st) + cy,
        ];
        // Derivative wrt t (the unit tangent direction, scaled).
        let tangent = [
            cosp * (-rx * st) - sinp * (ry * ct),
            sinp * (-rx * st) + cosp * (ry * ct),
        ];
        (point, tangent)
    };
    let mut a = theta1;
    for _ in 0..pieces {
        let b = a + delta;
        let (p0, t0) = on_ellipse(a);
        let (p3, t3) = on_ellipse(b);
        p.cubic_to(
            p0[0] + k * t0[0],
            p0[1] + k * t0[1],
            p3[0] - k * t3[0],
            p3[1] - k * t3[1],
            p3[0],
            p3[1],
        );
        a = b;
    }
}
