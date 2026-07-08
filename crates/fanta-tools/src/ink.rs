//! Variable-width pressure ink — clean-room reimplementation.
//!
//! Turns a sequence of `(x, y, pressure?)` pointer samples into a single closed
//! outline polygon whose local half-width varies point-by-point as a function of
//! size, pressure (real or velocity-simulated), and thinning. Filled with a
//! non-zero winding rule, that polygon is true hand-drawn "ink", not a
//! constant-width stroked polyline (which is all `PathData::stroke_to_fill` can
//! produce today).
//!
//! ## Provenance
//!
//! This is an **independent reimplementation** of the public algorithm popularized
//! by the MIT-licensed `perfect-freehand` package (Steve Ruiz). It was written
//! from the documented *interface and behavior* — the option semantics, the
//! `getStrokePoints` → `getStrokeOutlinePoints` pipeline, and the published
//! radius/streamline/pressure formulas — **not** by transcribing that source.
//! Correctness is verified differentially against `perfect-freehand`'s own output
//! as a reference oracle (see `oracle/` and `tests/ink_oracle.rs`); the original
//! never enters this crate or the shipped binary. The algorithm/constants are
//! facts; this Rust expression is our own.
//!
//! Pipeline mirrors the reference contract:
//! `get_stroke(pts) == get_stroke_outline_points(get_stroke_points(pts))`.

use glam::DVec2;
use serde::{Deserialize, Serialize};
use std::f64::consts::{FRAC_PI_2, PI};

/// How quickly simulated pressure tracks pointer speed (reference constant).
const RATE_OF_PRESSURE_CHANGE: f64 = 0.275;
/// Number of segments used to approximate a rounded end cap.
const CAP_SEGMENTS: usize = 16;

/// A raw input sample. `pressure < 0.0` means "no stylus pressure supplied"
/// (mouse/touch), which routes width through the velocity simulator.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InkPoint {
    pub pt: DVec2,
    pub pressure: f64,
}

impl InkPoint {
    /// A sample with no real pressure (the common mouse/touch case).
    pub fn new(x: f64, y: f64) -> Self {
        Self {
            pt: DVec2::new(x, y),
            pressure: -1.0,
        }
    }
    /// A sample carrying a real stylus pressure in `0..=1`.
    pub fn with_pressure(x: f64, y: f64, pressure: f64) -> Self {
        Self {
            pt: DVec2::new(x, y),
            pressure,
        }
    }
}

/// Brush parameters. Defaults match the reference (`size` 16, the rest mid-range).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct StrokeOptions {
    /// Base diameter at full pressure (px).
    pub size: f64,
    /// Pressure→width sensitivity in `-1..=1`. `0` = constant width.
    pub thinning: f64,
    /// Outline corner softness in `0..=1` (drives the min-distance dedup).
    pub smoothing: f64,
    /// Input stabilizer in `0..=1`; pulls each new point toward the last.
    pub streamline: f64,
    /// Synthesize pressure from pointer velocity when no stylus pressure exists.
    pub simulate_pressure: bool,
    /// `true` finalizes at the last input point; `false` (live drag) trails it.
    pub last: bool,
    /// Start taper distance (px) — `0` disables (rounded cap instead).
    pub taper_start: f64,
    /// End taper distance (px) — `0` disables (rounded cap instead).
    pub taper_end: f64,
    /// Draw a rounded cap at the start (ignored when `taper_start > 0`).
    pub cap_start: bool,
    /// Draw a rounded cap at the end (ignored when `taper_end > 0`).
    pub cap_end: bool,
}

impl Default for StrokeOptions {
    fn default() -> Self {
        Self {
            size: 16.0,
            thinning: 0.5,
            smoothing: 0.5,
            streamline: 0.5,
            simulate_pressure: true,
            last: true,
            taper_start: 0.0,
            taper_end: 0.0,
            cap_start: true,
            cap_end: true,
        }
    }
}

/// A processed centerline sample: stabilized position plus derived metadata.
#[derive(Clone, Copy, Debug)]
pub struct StrokePoint {
    pub point: DVec2,
    pub pressure: f64,
    /// Unit vector pointing *back* toward the previous point.
    pub vector: DVec2,
    /// Distance to the previous point.
    pub distance: f64,
    /// Cumulative arc length up to this point.
    pub running_length: f64,
}

#[inline]
fn perp(v: DVec2) -> DVec2 {
    DVec2::new(v.y, -v.x)
}

#[inline]
fn unit(v: DVec2) -> DVec2 {
    let len = v.length();
    if len > 0.0 { v / len } else { DVec2::ZERO }
}

/// Half-width at a point. `size * (0.5 - thinning * (0.5 - pressure))`.
/// (Linear easing; matches the reference for `thinning == 0` → `size / 2`.)
#[inline]
pub fn get_stroke_radius(size: f64, thinning: f64, pressure: f64) -> f64 {
    size * (0.5 - thinning * (0.5 - pressure))
}

/// Default start-taper easing `t * (2 - t)` (ease-out).
#[inline]
fn ease_taper_start(t: f64) -> f64 {
    t * (2.0 - t)
}

/// Default end-taper easing `(t - 1)^3 + 1` (ease-out cubic).
#[inline]
fn ease_taper_end(t: f64) -> f64 {
    let u = t - 1.0;
    u * u * u + 1.0
}

/// Stage 1: stabilize input and attach per-point metadata.
pub fn get_stroke_points(points: &[InkPoint], opts: &StrokeOptions) -> Vec<StrokePoint> {
    let t = 0.15 + (1.0 - opts.streamline) * 0.85;

    let mut pts: Vec<InkPoint> = points.to_vec();
    if pts.is_empty() {
        return Vec::new();
    }
    if pts.len() == 1 {
        // Duplicate with a tiny offset so a single tap still has a direction.
        let p0 = pts[0];
        pts.push(InkPoint {
            pt: p0.pt + DVec2::new(1.0, 1.0),
            pressure: p0.pressure,
        });
    }

    let mut out: Vec<StrokePoint> = Vec::with_capacity(pts.len());
    out.push(StrokePoint {
        point: pts[0].pt,
        pressure: if pts[0].pressure >= 0.0 {
            pts[0].pressure
        } else {
            0.25
        },
        vector: DVec2::new(1.0, 1.0),
        distance: 0.0,
        running_length: 0.0,
    });

    let mut has_min_length = false;
    let mut running = 0.0;
    let mut prev_point = out[0].point;
    let max = pts.len() - 1;

    for (i, raw) in pts.iter().enumerate().skip(1) {
        let point = if opts.last && i == max {
            raw.pt
        } else {
            prev_point + (raw.pt - prev_point) * t
        };
        if point == prev_point {
            continue;
        }
        let distance = (point - prev_point).length();
        running += distance;

        if i < max && !has_min_length {
            if running < opts.size {
                continue;
            }
            has_min_length = true;
        }

        out.push(StrokePoint {
            point,
            pressure: if raw.pressure >= 0.0 {
                raw.pressure
            } else {
                0.5
            },
            vector: unit(prev_point - point),
            distance,
            running_length: running,
        });
        prev_point = point;
    }

    if out.len() >= 2 {
        out[0].vector = out[1].vector;
    }
    out
}

/// Stage 2: offset the centerline into a closed, capped outline polygon.
pub fn get_stroke_outline_points(points: &[StrokePoint], opts: &StrokeOptions) -> Vec<[f64; 2]> {
    let n = points.len();
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return dot_outline(points[0].point, points[0].pressure, opts);
    }

    let total_length = points[n - 1].running_length;
    let min_dist = (opts.size * opts.smoothing).powi(2);

    // Seed pressure with a smoothed average over the leading points.
    let mut prev_pressure = {
        let mut acc = points[0].pressure;
        for sp in points.iter().take(10) {
            let mut pressure = sp.pressure;
            if opts.simulate_pressure {
                let speed = (sp.distance / opts.size).min(1.0);
                let rest = (1.0 - speed).min(1.0);
                pressure = (acc + (rest - acc) * (speed * RATE_OF_PRESSURE_CHANGE)).min(1.0);
            }
            acc = (acc + pressure) / 2.0;
        }
        acc
    };

    let mut left: Vec<DVec2> = Vec::new();
    let mut right: Vec<DVec2> = Vec::new();
    let mut tl = points[0].point;
    let mut tr = points[0].point;

    for (i, sp) in points.iter().enumerate() {
        // Skip points crowding the very end (avoids a wobbling tip).
        if i < n - 1 && (total_length - sp.running_length) < 3.0 {
            continue;
        }

        let mut pressure = sp.pressure;
        let mut radius = opts.size / 2.0;
        if opts.thinning != 0.0 {
            if opts.simulate_pressure {
                let speed = (sp.distance / opts.size).min(1.0);
                let rest = (1.0 - speed).min(1.0);
                pressure = (prev_pressure
                    + (rest - prev_pressure) * (speed * RATE_OF_PRESSURE_CHANGE))
                    .min(1.0);
            }
            radius = get_stroke_radius(opts.size, opts.thinning, pressure);
        }

        // Taper toward the ends if requested.
        let ts = if opts.taper_start > 0.0 && sp.running_length < opts.taper_start {
            ease_taper_start(sp.running_length / opts.taper_start)
        } else {
            1.0
        };
        let from_end = total_length - sp.running_length;
        let te = if opts.taper_end > 0.0 && from_end < opts.taper_end {
            ease_taper_end(from_end / opts.taper_end)
        } else {
            1.0
        };
        radius = (radius * ts.min(te)).max(0.01);

        let offset = perp(sp.vector) * radius;
        let pl = sp.point + offset;
        let pr = sp.point - offset;

        if i <= 1 || (pl - tl).length_squared() > min_dist {
            left.push(pl);
            tl = pl;
        }
        if i <= 1 || (pr - tr).length_squared() > min_dist {
            right.push(pr);
            tr = pr;
        }

        prev_pressure = pressure;
    }

    if left.is_empty() || right.is_empty() {
        return Vec::new();
    }

    // End cap: arc from the last left point around to the last right point,
    // bulging in the direction of travel (forward = -vector at the last point).
    let end_cap = if opts.cap_end && opts.taper_end == 0.0 {
        arc_between(
            *left.last().unwrap(),
            *right.last().unwrap(),
            -points[n - 1].vector,
        )
    } else {
        Vec::new()
    };
    // Start cap: arc from right[0] around to left[0], bulging backward (+vector0).
    let start_cap = if opts.cap_start && opts.taper_start == 0.0 {
        arc_between(right[0], left[0], points[0].vector)
    } else {
        Vec::new()
    };

    let mut out: Vec<[f64; 2]> =
        Vec::with_capacity(left.len() + right.len() + end_cap.len() + start_cap.len());
    out.extend(left.iter().map(|p| [p.x, p.y]));
    out.extend(end_cap.iter().map(|p| [p.x, p.y]));
    out.extend(right.iter().rev().map(|p| [p.x, p.y]));
    out.extend(start_cap.iter().map(|p| [p.x, p.y]));
    out
}

/// Full pipeline: `get_stroke_outline_points(get_stroke_points(points))`.
///
/// A single input point is *not* special-cased here: `get_stroke_points`
/// duplicates it with a tiny offset, so a lone tap renders as the same short
/// round-capped capsule the reference produces (a near-circular blob), keeping
/// this faithful to the reference pipeline.
pub fn get_stroke(points: &[InkPoint], opts: &StrokeOptions) -> Vec<[f64; 2]> {
    let sps = get_stroke_points(points, opts);
    get_stroke_outline_points(&sps, opts)
}

/// A single tap renders as a filled disk of the pressure-derived radius.
fn dot_outline(center: DVec2, pressure: f64, opts: &StrokeOptions) -> Vec<[f64; 2]> {
    let pressure = if pressure >= 0.0 { pressure } else { 0.25 };
    let r = get_stroke_radius(opts.size, opts.thinning, pressure).max(0.01);
    let n = 24;
    (0..n)
        .map(|k| {
            let a = (k as f64 / n as f64) * PI * 2.0;
            [center.x + a.cos() * r, center.y + a.sin() * r]
        })
        .collect()
}

/// Interior points of a semicircular cap from `from` to `to` (both diametric
/// about their midpoint), bulging toward `bulge`. Endpoints are excluded — they
/// are already present in the left/right arrays.
fn arc_between(from: DVec2, to: DVec2, bulge: DVec2) -> Vec<DVec2> {
    let center = (from + to) * 0.5;
    let r = (from - center).length();
    if r <= f64::EPSILON {
        return Vec::new();
    }
    let a0 = (from.y - center.y).atan2(from.x - center.x);
    // Choose the sweep direction whose midpoint leans toward `bulge`.
    let mid = |s: f64| {
        let a = a0 + s * FRAC_PI_2;
        DVec2::new(a.cos(), a.sin()) * r
    };
    let sign = if mid(1.0).dot(bulge) >= mid(-1.0).dot(bulge) {
        1.0
    } else {
        -1.0
    };

    (1..CAP_SEGMENTS)
        .map(|k| {
            let t = k as f64 / CAP_SEGMENTS as f64;
            let a = a0 + sign * PI * t;
            center + DVec2::new(a.cos(), a.sin()) * r
        })
        .collect()
}
