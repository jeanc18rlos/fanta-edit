//! The `PathData` type and its core construction/query/transform methods.

use super::arc::append_arc_cubics;
use super::{FillRule, PathSegment};
use serde::{Deserialize, Serialize};

/// A complete path: an ordered list of [`PathSegment`]s plus a [`FillRule`].
///
/// ## On-disk shape (schema v2)
///
/// Was a bare array (`#[serde(transparent)]`) in schema v1; v2 makes it an
/// object `{ "segments": [...], "fill_rule"?: "even-odd" }` so the fill rule
/// can ride along without a second field on every vector node. `fill_rule` is
/// skipped when default, so a v2 doc with no even-odd paths adds nothing beyond
/// the wrapping object. The v1→v2 migration in `fanta-format` rewrites the bare
/// array into `{ "segments": <array> }`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PathData {
    pub segments: Vec<PathSegment>,
    /// Interior-determination rule. Defaults to [`FillRule::NonZero`] and is
    /// omitted from JSON in that case (see [`FillRule::is_default`]).
    #[serde(default, skip_serializing_if = "FillRule::is_default")]
    pub fill_rule: FillRule,
    /// Per-subpath fill rules for paths whose subpaths were authored with
    /// MIXED winding rules (Figma stores a `windingRule` per `fillGeometry`
    /// Path entry). Entry `i` applies to the `i`-th subpath (a subpath starts
    /// at each [`Move`](super::PathSegment::Move); leading segments before any
    /// `Move` form subpath 0). Empty ⇒ every subpath uses [`fill_rule`]
    /// (`Self::fill_rule`) — the overwhelmingly common case, skipped from JSON
    /// so old docs round-trip byte-identical. Renderers that don't split
    /// per-rule may fall back to `fill_rule` alone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subpath_rules: Vec<FillRule>,
}

impl PathData {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a [`Move`] segment.
    ///
    /// [`Move`]: PathSegment::Move
    pub fn move_to(&mut self, x: f64, y: f64) -> &mut Self {
        self.segments.push(PathSegment::Move { to: [x, y] });
        self
    }

    pub fn line_to(&mut self, x: f64, y: f64) -> &mut Self {
        self.segments.push(PathSegment::Line { to: [x, y] });
        self
    }

    pub fn quad_to(&mut self, cx: f64, cy: f64, x: f64, y: f64) -> &mut Self {
        self.segments.push(PathSegment::Quad {
            ctrl: [cx, cy],
            to: [x, y],
        });
        self
    }

    pub fn cubic_to(
        &mut self,
        c1x: f64,
        c1y: f64,
        c2x: f64,
        c2y: f64,
        x: f64,
        y: f64,
    ) -> &mut Self {
        self.segments.push(PathSegment::Cubic {
            ctrl1: [c1x, c1y],
            ctrl2: [c2x, c2y],
            to: [x, y],
        });
        self
    }

    pub fn close(&mut self) -> &mut Self {
        self.segments.push(PathSegment::Close);
        self
    }

    /// Construct a rectangle path. Corners go top-left → top-right → bottom-
    /// right → bottom-left → close. Common starting shape for design tools.
    pub fn rect(x: f64, y: f64, w: f64, h: f64) -> Self {
        let mut p = Self::new();
        p.move_to(x, y)
            .line_to(x + w, y)
            .line_to(x + w, y + h)
            .line_to(x, y + h)
            .close();
        p
    }

    /// Construct an ellipse path approximated by four cubic Béziers. The
    /// `0.5522847498` magic constant is the standard "kappa" — the distance
    /// from a quadrant endpoint to its tangent control point that yields a
    /// near-perfect circle approximation (error < 0.03%).
    pub fn ellipse(cx: f64, cy: f64, rx: f64, ry: f64) -> Self {
        const KAPPA: f64 = 0.5522847498307933;
        let (ox, oy) = (rx * KAPPA, ry * KAPPA);
        let mut p = Self::new();
        p.move_to(cx - rx, cy)
            .cubic_to(cx - rx, cy - oy, cx - ox, cy - ry, cx, cy - ry)
            .cubic_to(cx + ox, cy - ry, cx + rx, cy - oy, cx + rx, cy)
            .cubic_to(cx + rx, cy + oy, cx + ox, cy + ry, cx, cy + ry)
            .cubic_to(cx - ox, cy + ry, cx - rx, cy + oy, cx - rx, cy)
            .close();
        p
    }

    /// Construct an ellipse **arc** — a pie slice, a donut/ring segment, or a
    /// full ring — as cubic Béziers, for Figma's `arcData` (partial-sweep /
    /// inner-radius ellipses). Coordinates are in the ellipse's local box
    /// `[0, 0, 2·rx, 2·ry]` (center at `(rx, ry)`), matching how a plain
    /// [`Self::ellipse`] is built for an ELLIPSE node.
    ///
    /// - `rx`, `ry` — the OUTER radii (half the node's width/height).
    /// - `start_rad` — start angle in radians, 0 = +x (3 o'clock), increasing
    ///   clockwise (screen y-down), matching Figma's `arcData.startingAngle`.
    /// - `sweep_rad` — signed angular extent in radians
    ///   (`endingAngle - startingAngle`).
    /// - `inner` — inner-radius ratio in `0..=1` (`0` ⇒ a solid pie slice;
    ///   `> 0` ⇒ a donut ring with inner radii `rx·inner`, `ry·inner`).
    ///
    /// The contour is closed:
    /// - **Pie** (`inner == 0`): center → outer-start → outer arc → close (back
    ///   through the center).
    /// - **Donut** (`inner > 0`): outer-start → outer arc (forward) → line to
    ///   inner-end → inner arc (reverse) → close. The reversed inner arc is what
    ///   carves the hole under the non-zero winding rule.
    ///
    /// Each quadrant-or-less span is one cubic via the standard arc-to-Bézier
    /// control-point formula `k = (4/3)·tan(Δ/4)`, so a quarter turn matches the
    /// KAPPA circle approximation [`Self::ellipse`] uses. A near-full sweep
    /// (`|sweep| ≥ 2π − ε`) falls through to the full ring/ellipse so callers
    /// don't have to special-case 360°.
    pub fn ellipse_arc(rx: f64, ry: f64, start_rad: f64, sweep_rad: f64, inner: f64) -> Self {
        let (cx, cy) = (rx, ry);
        let inner = inner.clamp(0.0, 1.0);
        let full = sweep_rad.abs() >= std::f64::consts::TAU - 1e-4;

        // A full 360° ring/ellipse: solid ellipse when no hole, else an outer
        // ellipse with a reversed inner ellipse carving the hole.
        if full {
            if inner <= 1e-3 {
                return Self::ellipse(cx, cy, rx, ry);
            }
            let mut p = Self::ellipse(cx, cy, rx, ry);
            let inner_rev = Self::ellipse(cx, cy, rx * inner, ry * inner).reversed();
            p.segments.extend(inner_rev.segments);
            return p;
        }

        let end_rad = start_rad + sweep_rad;
        let mut p = Self::new();
        if inner <= 1e-3 {
            // Pie slice: center, line out to the arc start, sweep the outer arc,
            // close back through the center.
            p.move_to(cx, cy);
            let (sx, sy) = (cx + rx * start_rad.cos(), cy + ry * start_rad.sin());
            p.line_to(sx, sy);
            append_arc_cubics(&mut p, cx, cy, rx, ry, start_rad, end_rad);
            p.close();
        } else {
            // Donut/ring segment: outer arc forward, line in to the inner-end,
            // inner arc back (reverse direction), close.
            let (irx, iry) = (rx * inner, ry * inner);
            let (osx, osy) = (cx + rx * start_rad.cos(), cy + ry * start_rad.sin());
            p.move_to(osx, osy);
            append_arc_cubics(&mut p, cx, cy, rx, ry, start_rad, end_rad);
            let (iex, iey) = (cx + irx * end_rad.cos(), cy + iry * end_rad.sin());
            p.line_to(iex, iey);
            append_arc_cubics(&mut p, cx, cy, irx, iry, end_rad, start_rad);
            p.close();
        }
        p
    }

    /// A regular `points`-point **star** inscribed in the box `[0, 0, w, h]`:
    /// the tips lie on the box's inscribed ellipse and the notches at
    /// `inner_ratio` (0..=1) of the radii, starting at the top (−90°) and going
    /// clockwise. Even-odd wound, matching the star tool so a parametric star
    /// regenerates identically. `points` clamps to ≥ 3.
    pub fn star(w: f64, h: f64, points: u32, inner_ratio: f64) -> Self {
        let n = points.max(3);
        let ratio = inner_ratio.clamp(0.0, 1.0);
        let (cx, cy, rx, ry) = (w * 0.5, h * 0.5, w * 0.5, h * 0.5);
        let start = -std::f64::consts::FRAC_PI_2;
        let step = std::f64::consts::TAU / (2.0 * n as f64);
        let mut p = Self::new();
        for i in 0..(2 * n) {
            let theta = start + step * f64::from(i);
            let scale = if i % 2 == 0 { 1.0 } else { ratio };
            let (x, y) = (cx + rx * scale * theta.cos(), cy + ry * scale * theta.sin());
            if i == 0 {
                p.move_to(x, y);
            } else {
                p.line_to(x, y);
            }
        }
        p.close();
        p.fill_rule = FillRule::EvenOdd;
        p
    }

    /// A regular convex `sides`-gon inscribed in the box `[0, 0, w, h]`, first
    /// vertex at the top (−90°), proceeding clockwise. Matches the polygon tool
    /// so a parametric polygon regenerates identically. `sides` clamps to ≥ 3.
    pub fn polygon(w: f64, h: f64, sides: u32) -> Self {
        let n = sides.max(3);
        let (cx, cy, rx, ry) = (w * 0.5, h * 0.5, w * 0.5, h * 0.5);
        let start = -std::f64::consts::FRAC_PI_2;
        let mut p = Self::new();
        for i in 0..n {
            let theta = start + std::f64::consts::TAU * f64::from(i) / f64::from(n);
            let (x, y) = (cx + rx * theta.cos(), cy + ry * theta.sin());
            if i == 0 {
                p.move_to(x, y);
            } else {
                p.line_to(x, y);
            }
        }
        p.close();
        p
    }

    /// Return this path with its drawing direction reversed (winding flipped).
    /// Only the Move/Line/Cubic/Close kinds this module produces are handled (a
    /// `Quad` degrades to a line through its endpoint, which `ellipse` /
    /// `ellipse_arc` never emit). Used to carve a donut hole: a reversed inner
    /// contour flips the winding so the non-zero rule empties it.
    fn reversed(&self) -> Self {
        // Collect the contour's points in order (the per-point control pair, if
        // the segment reaching it was a cubic; the trailing Close is dropped).
        let mut pts: Vec<[f64; 2]> = Vec::new();
        let mut ctrls: Vec<Option<([f64; 2], [f64; 2])>> = Vec::new();
        for seg in &self.segments {
            match seg {
                PathSegment::Move { to }
                | PathSegment::Line { to }
                | PathSegment::Quad { to, .. } => {
                    pts.push(*to);
                    ctrls.push(None);
                }
                PathSegment::Cubic { ctrl1, ctrl2, to } => {
                    pts.push(*to);
                    ctrls.push(Some((*ctrl1, *ctrl2)));
                }
                PathSegment::Close => {}
            }
        }
        let mut out = Self::new();
        let n = pts.len();
        if n == 0 {
            return out;
        }
        out.move_to(pts[n - 1][0], pts[n - 1][1]);
        for i in (1..n).rev() {
            // The original segment pts[i-1] -> pts[i] carried ctrls[i]; reversed
            // it goes pts[i] -> pts[i-1] with the control points swapped.
            match ctrls[i] {
                Some((c1, c2)) => {
                    out.cubic_to(c2[0], c2[1], c1[0], c1[1], pts[i - 1][0], pts[i - 1][1]);
                }
                None => {
                    out.line_to(pts[i - 1][0], pts[i - 1][1]);
                }
            }
        }
        out.close();
        out
    }

    /// Conservative AABB over finite endpoints and Bézier control points.
    /// A Bézier curve lies within its control-point convex hull; endpoint-only
    /// bounds can exclude visible curve bodies from picking and culling.
    pub fn rough_bounds(&self) -> Option<crate::transform::Bounds> {
        let mut bounds: Option<crate::transform::Bounds> = None;
        for mut segment in self.segments.iter().copied() {
            segment.map_points(|point| {
                let position = glam::DVec2::from(point);
                if position.is_finite() {
                    let point_bounds = crate::transform::Bounds::from_min_max(position, position);
                    bounds =
                        Some(bounds.map_or(point_bounds, |bounds| bounds.union(&point_bounds)));
                }
                point
            });
        }
        bounds
    }

    // ---- SVG `d=` interop ---------------------------------------------------
    //
    // F0 owns the single round-trip implementation of the SVG path grammar that
    // we support (M/L/Q/C/Z, absolute and relative) so that `fanta-export`
    // (writing SVG) and `fanta-fig-interop` (reading geometry) share exactly one
    // parser/serializer and can never drift. Arcs (`A`) and shorthand smooth
    // curves (`S`/`T`) are intentionally out of scope here — every producer we
    // emit to approximates arcs with cubics, so we never need to *write* one,
    // and an importer that hands us one should pre-flatten it (matching the
    // "no arcs" note at the top of this module). Horizontal/vertical line
    // shorthands (`H`/`V`) ARE accepted on read because real-world SVG uses them
    // heavily; they normalize to full `Line` segments.

    /// Whether this path is a single axis-aligned rectangle: a `Move` followed by
    /// `Line`s (and an optional `Close`) tracing exactly four corners whose edges
    /// are all horizontal or vertical. This is the canonical check the renderer
    /// and the auto-layout resizer share so a real rectangle (button / divider /
    /// background) is treated as resizable box geometry, while any other path
    /// (icon, ellipse, star, custom vector) is preserved as authored.
    pub fn is_rect(&self) -> bool {
        let mut pts: Vec<[f64; 2]> = match self.segments.first() {
            Some(PathSegment::Move { to }) => vec![*to],
            _ => return false,
        };
        for seg in &self.segments[1..] {
            match seg {
                PathSegment::Line { to } => pts.push(*to),
                PathSegment::Close => {}
                _ => return false, // any curve or a second subpath => not a plain rect
            }
        }
        if pts.len() != 4 {
            return false;
        }
        (0..4).all(|i| {
            let a = pts[i];
            let b = pts[(i + 1) % 4];
            (a[1] - b[1]).abs() < 1e-6 || (a[0] - b[0]).abs() < 1e-6
        })
    }

    /// Apply `f` to every coordinate of every segment (anchors and Bézier
    /// control points), in place. The per-segment point layout lives once in
    /// [`PathSegment::map_points`]; each point-wise transform below is just a
    /// closure over this.
    pub fn map_points_mut(&mut self, mut f: impl FnMut([f64; 2]) -> [f64; 2]) -> &mut Self {
        for seg in &mut self.segments {
            seg.map_points(&mut f);
        }
        self
    }

    /// Scale every coordinate (anchors and Bézier control points) about the point
    /// `(ox, oy)` by `(sx, sy)`. Used to resize real vector geometry to a new box
    /// *without* flattening it to a rectangle — e.g. an icon stretched by an
    /// auto-layout FILL child, or `create_icon` baking its display scale into the
    /// path so the layout engine measures the icon at its true on-screen size.
    pub fn scale_about(&mut self, ox: f64, oy: f64, sx: f64, sy: f64) -> &mut Self {
        self.map_points_mut(|p| [ox + (p[0] - ox) * sx, oy + (p[1] - oy) * sy])
    }

    /// Translate every coordinate by `(dx, dy)`. Pairs with [`Self::scale_about`]
    /// to map an SVG `viewBox` into local space (translate the box origin to 0,
    /// then scale to the display size).
    pub fn translate(&mut self, dx: f64, dy: f64) -> &mut Self {
        self.map_points_mut(|p| [p[0] + dx, p[1] + dy])
    }

    /// Bake a stroke of `width` (with the given cap/join) into a filled outline
    /// path — the "Outline Stroke" operation. The result is a single filled shape
    /// that reproduces the stroked appearance, so an icon authored as strokes
    /// becomes a robust filled vector: no live-stroke artifacts, crisp at any
    /// scale, and boolean-able. Uses `kurbo`'s stroke expansion; the output is a
    /// nonzero-winding fill (the default for `PathData`).
    pub fn stroke_to_fill(
        &self,
        width: f64,
        cap: crate::style::StrokeCap,
        join: crate::style::StrokeJoin,
    ) -> PathData {
        use crate::style::{StrokeCap, StrokeJoin};
        let mut bez = kurbo::BezPath::new();
        for seg in &self.segments {
            match *seg {
                PathSegment::Move { to } => bez.move_to((to[0], to[1])),
                PathSegment::Line { to } => bez.line_to((to[0], to[1])),
                PathSegment::Quad { ctrl, to } => bez.quad_to((ctrl[0], ctrl[1]), (to[0], to[1])),
                PathSegment::Cubic { ctrl1, ctrl2, to } => {
                    bez.curve_to((ctrl1[0], ctrl1[1]), (ctrl2[0], ctrl2[1]), (to[0], to[1]))
                }
                PathSegment::Close => bez.close_path(),
            }
        }
        let kcap = match cap {
            StrokeCap::Butt => kurbo::Cap::Butt,
            StrokeCap::Round => kurbo::Cap::Round,
            StrokeCap::Square => kurbo::Cap::Square,
        };
        let kjoin = match join {
            StrokeJoin::Miter => kurbo::Join::Miter,
            StrokeJoin::Round => kurbo::Join::Round,
            StrokeJoin::Bevel => kurbo::Join::Bevel,
        };
        let style = kurbo::Stroke::new(width).with_caps(kcap).with_join(kjoin);
        // Tolerance is in path units; ~0.1 keeps curve flattening crisp at icon scale.
        let outline = kurbo::stroke(bez.iter(), &style, &kurbo::StrokeOpts::default(), 0.1);
        let mut out = PathData::new();
        for el in outline.elements() {
            match *el {
                kurbo::PathEl::MoveTo(p) => {
                    out.move_to(p.x, p.y);
                }
                kurbo::PathEl::LineTo(p) => {
                    out.line_to(p.x, p.y);
                }
                kurbo::PathEl::QuadTo(c, p) => {
                    out.quad_to(c.x, c.y, p.x, p.y);
                }
                kurbo::PathEl::CurveTo(c1, c2, p) => {
                    out.cubic_to(c1.x, c1.y, c2.x, c2.y, p.x, p.y);
                }
                kurbo::PathEl::ClosePath => {
                    out.close();
                }
            }
        }
        out
    }

    /// Serialize to an SVG `d=` attribute string. Round-trips with
    /// [`Self::from_svg_d`] for the M/L/Q/C/Z subset. Coordinates use `{}`
    /// (Rust's shortest round-trippable float formatting), absolute commands.
    pub fn to_svg_d(&self) -> String {
        let mut out = String::new();
        let fmt2 = |out: &mut String, p: [f64; 2]| {
            out.push_str(&format!("{} {}", p[0], p[1]));
        };
        for (i, seg) in self.segments.iter().enumerate() {
            if i > 0 && !out.is_empty() {
                out.push(' ');
            }
            match seg {
                PathSegment::Move { to } => {
                    out.push_str("M ");
                    fmt2(&mut out, *to);
                }
                PathSegment::Line { to } => {
                    out.push_str("L ");
                    fmt2(&mut out, *to);
                }
                PathSegment::Quad { ctrl, to } => {
                    out.push_str("Q ");
                    fmt2(&mut out, *ctrl);
                    out.push(' ');
                    fmt2(&mut out, *to);
                }
                PathSegment::Cubic { ctrl1, ctrl2, to } => {
                    out.push_str("C ");
                    fmt2(&mut out, *ctrl1);
                    out.push(' ');
                    fmt2(&mut out, *ctrl2);
                    out.push(' ');
                    fmt2(&mut out, *to);
                }
                PathSegment::Close => out.push('Z'),
            }
        }
        out
    }
}
