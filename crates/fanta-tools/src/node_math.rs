//! Pure path-editing math for the node-edit (direct selection) tool.
//!
//! Free functions over [`PathData`] — no tool state, no doc access — so every
//! edit the [`crate::node_edit::NodeEditTool`] performs is unit-testable in
//! isolation. The functions decompose a path into per-subpath anchor lists
//! (anchor position + incoming/outgoing Bézier control points), edit that
//! representation, and recompose a fresh [`PathData`].
//!
//! ## Representation notes
//!
//! - A subpath starts at a [`PathSegment::Move`]; [`PathSegment::Close`] marks
//!   it closed (SVG semantics — a later `Move` does NOT close the previous
//!   subpath).
//! - Quadratic segments are degree-elevated to exact cubics on decompose, so
//!   every curved edge carries one control per side. The elevation is exact
//!   (the curve is pointwise identical), which also resolves the "one quad
//!   control shared by two anchors" ambiguity for handle edits.
//! - A closed subpath whose final explicit segment lands back on the start
//!   point (the pen tool's closing-segment style) folds that segment into the
//!   first anchor's incoming handle — no duplicate wraparound anchor.
//! - Recompose emits cubics only for edges that have at least one handle;
//!   handle-free edges stay [`PathSegment::Line`] / rely on `Close`.

use fanta_doc::{PathData, PathSegment};
use glam::DVec2;

/// Two handles are "smooth" (collinear) when their directions differ by at
/// most this many degrees. Matches Illustrator's tolerance for treating an
/// anchor as smooth.
pub const SMOOTH_ANGLE_TOL_DEG: f64 = 1.0;

/// Positions closer than this (in path units) are treated as coincident when
/// folding a pen-style explicit closing segment into the first anchor.
const COINCIDENT_EPS: f64 = 1e-9;

pub fn close_path(path: &PathData) -> Option<PathData> {
    let mut result = path.clone();
    result.segments.clear();
    let mut open = false;
    let mut has_edge = false;
    for segment in &path.segments {
        match segment {
            PathSegment::Move { .. } => {
                if open && has_edge {
                    result.segments.push(PathSegment::Close);
                }
                open = true;
                has_edge = false;
            }
            PathSegment::Close => open = false,
            _ => has_edge = true,
        }
        result.segments.push(*segment);
    }
    if open && has_edge {
        result.segments.push(PathSegment::Close);
    }
    (result != *path).then_some(result)
}

pub fn join_paths(paths: &[PathData]) -> Option<PathData> {
    if paths.len() < 2 {
        return None;
    }
    let mut result = paths.first()?.clone();
    result.segments.clear();
    let mut previous_end: Option<DVec2> = None;
    for path in paths {
        if !path_is_finite(path) {
            return None;
        }
        let (PathSegment::Move { to }, segments) = path.segments.split_first()? else {
            return None;
        };
        if segments.is_empty()
            || segments
                .iter()
                .any(|segment| matches!(segment, PathSegment::Move { .. } | PathSegment::Close))
        {
            return None;
        }
        let start = pt(*to);
        if previous_end.is_none() {
            result.segments.push(PathSegment::Move { to: *to });
        } else if previous_end.is_some_and(|end| (end - start).length() > COINCIDENT_EPS) {
            result.segments.push(PathSegment::Line { to: *to });
        }
        result.segments.extend_from_slice(segments);
        previous_end = segments.last().and_then(|segment| match segment {
            PathSegment::Line { to }
            | PathSegment::Quad { to, .. }
            | PathSegment::Cubic { to, .. } => Some(pt(*to)),
            _ => None,
        });
    }
    Some(result)
}

pub fn simplify_path(path: &PathData, tolerance: f64) -> Option<PathData> {
    if !tolerance.is_finite() || tolerance <= 0. || !path_is_finite(path) {
        return None;
    }
    // Kurbo's fitter can subdivide perfectly straight runs, so collapse
    // redundant line anchors exactly before fitting curved portions.
    let source = collapse_collinear_lines(path);
    let mut bezier = kurbo::BezPath::new();
    let mut open = false;
    for segment in &source.segments {
        match *segment {
            PathSegment::Move { to } => {
                bezier.move_to((to[0], to[1]));
                open = true;
            }
            _ if !open => return None,
            PathSegment::Line { to } => bezier.line_to((to[0], to[1])),
            PathSegment::Quad { ctrl, to } => bezier.quad_to((ctrl[0], ctrl[1]), (to[0], to[1])),
            PathSegment::Cubic { ctrl1, ctrl2, to } => {
                bezier.curve_to((ctrl1[0], ctrl1[1]), (ctrl2[0], ctrl2[1]), (to[0], to[1]))
            }
            PathSegment::Close => {
                bezier.close_path();
                open = false;
            }
        }
    }
    let simplified = kurbo::simplify::simplify_bezpath(
        bezier,
        tolerance,
        &kurbo::simplify::SimplifyOptions::default(),
    );
    let mut result = path.clone();
    result.segments = bezier_segments(&simplified);
    // A degenerate subpath can disappear during fitting; retain the authored
    // path rather than shift the per-subpath fill rules onto another contour.
    let subpath_count = |path: &PathData| {
        path.segments
            .iter()
            .filter(|segment| matches!(segment, PathSegment::Move { .. }))
            .count()
    };
    if subpath_count(&result) != subpath_count(path) || !path_is_finite(&result) {
        return None;
    }
    Some(if result.segments.len() < source.segments.len() {
        result
    } else {
        source
    })
}

fn collapse_collinear_lines(path: &PathData) -> PathData {
    let mut result = path.clone();
    result.segments.clear();
    let mut current = None;
    let mut line_start = None;
    for segment in &path.segments {
        if let PathSegment::Line { to } = segment {
            let end = pt(*to);
            if let (Some(start), Some(previous), Some(PathSegment::Line { to })) =
                (line_start, current, result.segments.last_mut())
            {
                let before: DVec2 = previous - start;
                let after = end - previous;
                if before.dot(after) >= 0.
                    && before.perp_dot(after).abs()
                        <= COINCIDENT_EPS * (before.length() * after.length()).max(1.)
                {
                    *to = end.to_array();
                    current = Some(end);
                    continue;
                }
            }
            line_start = current;
        } else {
            line_start = None;
        }
        current = segment.end_point();
        result.segments.push(*segment);
    }
    result
}

fn bezier_segments(path: &kurbo::BezPath) -> Vec<PathSegment> {
    path.elements()
        .iter()
        .map(|element| match *element {
            kurbo::PathEl::MoveTo(point) => PathSegment::Move {
                to: [point.x, point.y],
            },
            kurbo::PathEl::LineTo(point) => PathSegment::Line {
                to: [point.x, point.y],
            },
            kurbo::PathEl::QuadTo(control, point) => PathSegment::Quad {
                ctrl: [control.x, control.y],
                to: [point.x, point.y],
            },
            kurbo::PathEl::CurveTo(first, second, point) => PathSegment::Cubic {
                ctrl1: [first.x, first.y],
                ctrl2: [second.x, second.y],
                to: [point.x, point.y],
            },
            kurbo::PathEl::ClosePath => PathSegment::Close,
        })
        .collect()
}

fn path_is_finite(path: &PathData) -> bool {
    path.segments.iter().all(|segment| {
        let mut finite = true;
        let mut segment = *segment;
        segment.map_points(|point| {
            finite &= point.into_iter().all(f64::is_finite);
            point
        });
        finite
    })
}

/// Addresses one anchor: `subpath` indexes the decomposed subpath list,
/// `index` the anchor within it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AnchorId {
    pub subpath: usize,
    pub index: usize,
}

/// Which side of an anchor a handle edit targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandleSide {
    /// The control point of the incoming edge (toward the previous anchor).
    In,
    /// The control point of the outgoing edge (toward the next anchor).
    Out,
}

/// One anchor point with its adjacent control points (absolute coordinates).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnchorPt {
    pub pos: DVec2,
    /// Control point of the incoming edge nearest this anchor (cubic `ctrl2`),
    /// `None` when the incoming edge is a straight line (or absent).
    pub ctrl_in: Option<DVec2>,
    /// Control point of the outgoing edge nearest this anchor (cubic `ctrl1`),
    /// `None` when the outgoing edge is a straight line (or absent).
    pub ctrl_out: Option<DVec2>,
}

/// One decomposed subpath: its anchors in draw order plus the closed flag.
#[derive(Debug, Clone, PartialEq)]
pub struct SubPath {
    pub anchors: Vec<AnchorPt>,
    pub closed: bool,
}

/// Flat anchor listing for enumeration / hit-testing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnchorInfo {
    pub id: AnchorId,
    pub pos: DVec2,
    pub ctrl_in: Option<DVec2>,
    pub ctrl_out: Option<DVec2>,
    /// Whether the containing subpath is closed.
    pub closed: bool,
}

fn pt(p: [f64; 2]) -> DVec2 {
    DVec2::new(p[0], p[1])
}

/// A control point that coincides with its anchor is a retracted (absent)
/// handle — normalize it to `None` so a one-sided cubic round-trips as "this
/// side has no handle" instead of growing a phantom zero-length handle.
fn nondegenerate(ctrl: DVec2, anchor: DVec2) -> Option<DVec2> {
    ((ctrl - anchor).length() > COINCIDENT_EPS).then_some(ctrl)
}

// =============================================================================
// Decompose / recompose
// =============================================================================

/// Break a path into per-subpath anchor lists. Quads are degree-elevated to
/// exact cubics; a pen-style explicit closing segment (ending on the start
/// point, followed by `Close`) folds into the first anchor's incoming handle.
pub fn decompose(path: &PathData) -> Vec<SubPath> {
    let mut subs: Vec<SubPath> = Vec::new();
    let mut cur: Option<SubPath> = None;
    for seg in &path.segments {
        match *seg {
            PathSegment::Move { to } => {
                push_current_subpath(&mut subs, &mut cur);
                cur = Some(new_subpath(pt(to)));
            }
            PathSegment::Line { to } => push_line(cur.as_mut(), pt(to)),
            PathSegment::Quad { ctrl, to } => push_quad(cur.as_mut(), pt(ctrl), pt(to)),
            PathSegment::Cubic { ctrl1, ctrl2, to } => {
                push_cubic(cur.as_mut(), pt(ctrl1), pt(ctrl2), pt(to));
            }
            PathSegment::Close => close_subpath(cur.as_mut()),
        }
    }
    push_current_subpath(&mut subs, &mut cur);
    subs
}

fn push_current_subpath(subs: &mut Vec<SubPath>, cur: &mut Option<SubPath>) {
    if let Some(sp) = cur.take() {
        subs.push(sp);
    }
}

fn new_subpath(pos: DVec2) -> SubPath {
    SubPath {
        anchors: vec![AnchorPt {
            pos,
            ctrl_in: None,
            ctrl_out: None,
        }],
        closed: false,
    }
}

fn push_line(sp: Option<&mut SubPath>, to: DVec2) {
    if let Some(sp) = sp {
        sp.anchors.push(plain_anchor(to));
    }
}

fn push_quad(sp: Option<&mut SubPath>, ctrl: DVec2, to: DVec2) {
    let Some(sp) = sp else {
        return;
    };
    let Some(last) = sp.anchors.last_mut() else {
        return;
    };
    // Exact degree elevation: c1 = p0 + 2/3*(q - p0),
    // c2 = p1 + 2/3*(q - p1). Pointwise identical curve.
    let p0 = last.pos;
    last.ctrl_out = nondegenerate(p0 + (ctrl - p0) * (2.0 / 3.0), p0);
    sp.anchors.push(AnchorPt {
        pos: to,
        ctrl_in: nondegenerate(to + (ctrl - to) * (2.0 / 3.0), to),
        ctrl_out: None,
    });
}

fn push_cubic(sp: Option<&mut SubPath>, ctrl1: DVec2, ctrl2: DVec2, to: DVec2) {
    if let Some(sp) = sp {
        if let Some(last) = sp.anchors.last_mut() {
            last.ctrl_out = nondegenerate(ctrl1, last.pos);
        }
        sp.anchors.push(AnchorPt {
            pos: to,
            ctrl_in: nondegenerate(ctrl2, to),
            ctrl_out: None,
        });
    }
}

fn plain_anchor(pos: DVec2) -> AnchorPt {
    AnchorPt {
        pos,
        ctrl_in: None,
        ctrl_out: None,
    }
}

fn close_subpath(sp: Option<&mut SubPath>) {
    let Some(sp) = sp else {
        return;
    };
    sp.closed = true;
    fold_duplicate_closing_anchor(sp);
}

fn fold_duplicate_closing_anchor(sp: &mut SubPath) {
    if sp.anchors.len() < 2 {
        return;
    }
    let first_pos = sp.anchors[0].pos;
    let last = *sp.anchors.last().expect("len >= 2");
    if (last.pos - first_pos).length() <= COINCIDENT_EPS {
        sp.anchors[0].ctrl_in = last.ctrl_in;
        sp.anchors.pop();
    }
}

/// Rebuild a [`PathData`] from decomposed subpaths. Inverse of [`decompose`]
/// up to representation normalization (quads become cubics, closing edges
/// with handles become an explicit cubic back to the start + `Close`).
pub fn recompose(subs: &[SubPath]) -> PathData {
    let mut path = PathData::new();
    for sp in subs {
        let Some(first) = sp.anchors.first() else {
            continue;
        };
        path.move_to(first.pos.x, first.pos.y);
        for win in sp.anchors.windows(2) {
            emit_edge(&mut path, &win[0], &win[1]);
        }
        if sp.closed {
            if sp.anchors.len() >= 2 {
                let last = sp.anchors.last().expect("len >= 2");
                // The closing edge runs last → first. Curved closing edges
                // need the explicit segment; straight ones are Close itself.
                if last.ctrl_out.is_some() || first.ctrl_in.is_some() {
                    emit_edge(&mut path, last, first);
                }
            }
            path.close();
        }
    }
    path
}

pub fn reverse_path(path: &PathData) -> Option<PathData> {
    if !path_is_finite(path) {
        return None;
    }
    let mut subpaths = decompose(path);
    if !subpaths.iter().any(|subpath| subpath.anchors.len() >= 2) {
        return None;
    }
    for subpath in &mut subpaths {
        subpath.anchors.reverse();
        if subpath.closed && !subpath.anchors.is_empty() {
            subpath.anchors.rotate_right(1);
        }
        for anchor in &mut subpath.anchors {
            std::mem::swap(&mut anchor.ctrl_in, &mut anchor.ctrl_out);
        }
    }
    let mut result = recompose(&subpaths);
    result.fill_rule = path.fill_rule;
    result.subpath_rules = path.subpath_rules.clone();
    Some(result)
}

pub fn break_at_anchor(path: &PathData, anchor: AnchorId) -> Option<PathData> {
    let mut subpaths = decompose(path);
    let subpath = subpaths.get_mut(anchor.subpath)?;
    subpath.anchors.get(anchor.index)?;
    if subpath.anchors.len() < 2 {
        return None;
    }
    let mut rules = path.subpath_rules.clone();
    if subpath.closed {
        subpath.closed = false;
        subpath.anchors.rotate_left(anchor.index);
        let mut end = *subpath.anchors.first()?;
        end.ctrl_out = None;
        subpath.anchors.first_mut()?.ctrl_in = None;
        subpath.anchors.push(end);
    } else {
        if anchor.index == 0 || anchor.index + 1 == subpath.anchors.len() {
            return None;
        }
        let mut following = SubPath {
            anchors: subpath.anchors.get(anchor.index..)?.to_vec(),
            closed: false,
        };
        subpath.anchors.truncate(anchor.index + 1);
        subpath.anchors.last_mut()?.ctrl_out = None;
        following.anchors.first_mut()?.ctrl_in = None;
        subpaths.insert(anchor.subpath + 1, following);
        if !rules.is_empty() {
            rules.resize(subpaths.len() - 1, path.fill_rule);
            let rule = *rules.get(anchor.subpath)?;
            rules.insert(anchor.subpath + 1, rule);
        }
    }
    let mut result = recompose(&subpaths);
    result.fill_rule = path.fill_rule;
    result.subpath_rules = rules;
    Some(result)
}

pub fn delete_anchors_preserving_shape(
    path: &PathData,
    anchors: &[AnchorId],
    tolerance: f64,
) -> Option<PathData> {
    if !tolerance.is_finite() || tolerance <= 0. || !path_is_finite(path) {
        return None;
    }
    let mut result = path.clone();
    result.segments.clear();
    let mut changed = false;
    for (subpath_index, subpath) in decompose(path).iter().enumerate() {
        let count = subpath.anchors.len();
        let deleted = deletion_mask(subpath_index, count, anchors);
        let delete_count = deletion_count(&deleted);
        if delete_count == 0 || !can_delete_anchors(count, delete_count, subpath.closed) {
            result
                .segments
                .extend(recompose(std::slice::from_ref(subpath)).segments);
            continue;
        }
        changed = true;
        let surviving = deleted
            .iter()
            .enumerate()
            .filter_map(|(index, deleted)| (!deleted).then_some(index))
            .collect::<Vec<_>>();
        let first = *surviving.first()?;
        let start = subpath.anchors.get(first)?;
        result.move_to(start.pos.x, start.pos.y);
        for indices in surviving.windows(2) {
            append_preserved_gap(
                &mut result,
                subpath,
                *indices.first()?,
                *indices.get(1)?,
                tolerance,
            )?;
        }
        if subpath.closed {
            append_preserved_gap(&mut result, subpath, *surviving.last()?, first, tolerance)?;
            result.close();
        }
    }
    (changed && path_is_finite(&result)).then_some(result)
}

fn append_preserved_gap(
    result: &mut PathData,
    subpath: &SubPath,
    start: usize,
    end: usize,
    tolerance: f64,
) -> Option<()> {
    let count = subpath.anchors.len();
    let steps = (end + count - start) % count;
    let first = subpath.anchors.get(start)?;
    let last = subpath.anchors.get(end)?;
    if steps == 1 {
        emit_edge(result, first, last);
        return Some(());
    }
    let mut bezier = kurbo::BezPath::new();
    bezier.move_to((first.pos.x, first.pos.y));
    let mut curved = false;
    for step in 0..steps {
        let first = subpath.anchors.get((start + step) % count)?;
        let second = subpath.anchors.get((start + step + 1) % count)?;
        if first.ctrl_out.is_some() || second.ctrl_in.is_some() {
            curved = true;
            let control_first = first.ctrl_out.unwrap_or(first.pos);
            let control_second = second.ctrl_in.unwrap_or(second.pos);
            bezier.curve_to(
                (control_first.x, control_first.y),
                (control_second.x, control_second.y),
                (second.pos.x, second.pos.y),
            );
        } else {
            bezier.line_to((second.pos.x, second.pos.y));
        }
    }
    if curved {
        let source = kurbo::simplify::SimplifyBezPath::new(bezier);
        let fitted = kurbo::fit_to_bezpath(&source, tolerance);
        let segments = bezier_segments(&fitted);
        result.segments.extend_from_slice(segments.get(1..)?);
    } else {
        result.line_to(last.pos.x, last.pos.y);
    }
    Some(())
}

/// Append the edge `a → b`: a cubic when either side carries a handle, else a
/// straight line.
fn emit_edge(path: &mut PathData, a: &AnchorPt, b: &AnchorPt) {
    match (a.ctrl_out, b.ctrl_in) {
        (None, None) => {
            path.line_to(b.pos.x, b.pos.y);
        }
        (c1, c2) => {
            let c1 = c1.unwrap_or(a.pos);
            let c2 = c2.unwrap_or(b.pos);
            path.cubic_to(c1.x, c1.y, c2.x, c2.y, b.pos.x, b.pos.y);
        }
    }
}

// =============================================================================
// Enumeration
// =============================================================================

/// Flat list of every anchor in the path with its adjacent control points,
/// across all subpaths. Closed-path wraparound is already folded: a closed
/// subpath's first anchor carries the closing edge's incoming control.
pub fn enumerate_anchors(path: &PathData) -> Vec<AnchorInfo> {
    let mut out = Vec::new();
    for (s, sp) in decompose(path).iter().enumerate() {
        for (i, a) in sp.anchors.iter().enumerate() {
            out.push(AnchorInfo {
                id: AnchorId {
                    subpath: s,
                    index: i,
                },
                pos: a.pos,
                ctrl_in: a.ctrl_in,
                ctrl_out: a.ctrl_out,
                closed: sp.closed,
            });
        }
    }
    out
}

fn get_anchor(subs: &[SubPath], id: AnchorId) -> Option<AnchorPt> {
    subs.get(id.subpath)?.anchors.get(id.index).copied()
}

// =============================================================================
// Editing operations
// =============================================================================

/// Move one anchor to `new_pos`, dragging its attached control points along by
/// the same delta. Returns `None` for an out-of-range id.
pub fn move_anchor(path: &PathData, id: AnchorId, new_pos: DVec2) -> Option<PathData> {
    let mut subs = decompose(path);
    let a = subs.get_mut(id.subpath)?.anchors.get_mut(id.index)?;
    let delta = new_pos - a.pos;
    a.pos = new_pos;
    a.ctrl_in = a.ctrl_in.map(|c| c + delta);
    a.ctrl_out = a.ctrl_out.map(|c| c + delta);
    Some(recompose(&subs))
}

/// Set one handle of an anchor to the absolute position `ctrl`. With `mirror`
/// the opposite handle (when present) is rotated to stay collinear — its
/// LENGTH is preserved, only its angle follows (the smooth-anchor drag). Pass
/// `mirror = false` to break the pair into a corner (the Alt drag).
///
/// A handle written onto a straight edge promotes that edge to a cubic; the
/// neighbor side's control defaults to its own anchor (degenerate, renders
/// identically until dragged out).
pub fn set_handle(
    path: &PathData,
    id: AnchorId,
    side: HandleSide,
    ctrl: DVec2,
    mirror: bool,
) -> Option<PathData> {
    let mut subs = decompose(path);
    let a = subs.get_mut(id.subpath)?.anchors.get_mut(id.index)?;
    let dir = ctrl - a.pos;
    match side {
        HandleSide::In => a.ctrl_in = Some(ctrl),
        HandleSide::Out => a.ctrl_out = Some(ctrl),
    }
    if mirror && dir.length() > COINCIDENT_EPS {
        let unit = dir / dir.length();
        match side {
            HandleSide::In => {
                if let Some(out) = a.ctrl_out {
                    let len = (out - a.pos).length();
                    a.ctrl_out = Some(a.pos - unit * len);
                }
            }
            HandleSide::Out => {
                if let Some(inn) = a.ctrl_in {
                    let len = (inn - a.pos).length();
                    a.ctrl_in = Some(a.pos - unit * len);
                }
            }
        }
    }
    Some(recompose(&subs))
}

/// Whether an anchor is "smooth": both handles exist, are non-degenerate, and
/// are collinear within [`SMOOTH_ANGLE_TOL_DEG`].
pub fn is_smooth(a: &AnchorPt) -> bool {
    let (Some(ci), Some(co)) = (a.ctrl_in, a.ctrl_out) else {
        return false;
    };
    let vin = a.pos - ci; // direction of travel INTO the anchor
    let vout = co - a.pos; // direction of travel OUT of the anchor
    if vin.length() <= COINCIDENT_EPS || vout.length() <= COINCIDENT_EPS {
        return false;
    }
    let cos = vin.dot(vout) / (vin.length() * vout.length());
    cos >= SMOOTH_ANGLE_TOL_DEG.to_radians().cos()
}

/// Whether the anchor addressed by `id` is smooth (see [`is_smooth`]).
pub fn anchor_is_smooth(path: &PathData, id: AnchorId) -> bool {
    let subs = decompose(path);
    get_anchor(&subs, id).is_some_and(|a| is_smooth(&a))
}

/// Toggle an anchor between smooth and corner:
///
/// - A smooth anchor becomes a corner: both handles retract (the adjacent
///   edges keep the NEIGHBOR-side controls; a fully handle-free edge demotes
///   back to a line via [`recompose`]).
/// - A corner anchor becomes smooth: handles are rebuilt collinear along the
///   prev→next chord direction, each 1/3 the length of its adjacent chord.
///   Open-path endpoints grow only the one handle their single edge carries.
pub fn toggle_smooth(path: &PathData, id: AnchorId) -> Option<PathData> {
    let mut subs = decompose(path);
    let sp = subs.get(id.subpath)?;
    let n = sp.anchors.len();
    let a = *sp.anchors.get(id.index)?;
    if is_smooth(&a) {
        let am = &mut subs[id.subpath].anchors[id.index];
        am.ctrl_in = None;
        am.ctrl_out = None;
        return Some(recompose(&subs));
    }
    // Corner → smooth. Neighbor positions (wrapping when closed).
    let prev = if sp.closed {
        (n > 1).then(|| sp.anchors[(id.index + n - 1) % n].pos)
    } else {
        id.index.checked_sub(1).map(|i| sp.anchors[i].pos)
    };
    let next = if sp.closed {
        (n > 1).then(|| sp.anchors[(id.index + 1) % n].pos)
    } else {
        sp.anchors.get(id.index + 1).map(|x| x.pos)
    };
    let dir = match (prev, next) {
        (Some(p), Some(q)) => q - p,
        (Some(p), None) => a.pos - p,
        (None, Some(q)) => q - a.pos,
        (None, None) => return Some(recompose(&subs)), // lone anchor: nothing to do
    };
    if dir.length() <= COINCIDENT_EPS {
        return Some(recompose(&subs));
    }
    let unit = dir / dir.length();
    let am = &mut subs[id.subpath].anchors[id.index];
    if let Some(p) = prev {
        am.ctrl_in = Some(a.pos - unit * ((a.pos - p).length() / 3.0));
    }
    if let Some(q) = next {
        am.ctrl_out = Some(a.pos + unit * ((q - a.pos).length() / 3.0));
    }
    Some(recompose(&subs))
}

/// Delete the given anchors, rejoining each gap's surviving neighbors with a
/// straight line. Guards: a subpath must keep at least 2 anchors when open and
/// 3 when closed — deletions that would breach the minimum are skipped for
/// that whole subpath (its anchors stay untouched). Returns `None` when no
/// anchor was deleted at all.
pub fn delete_anchors(path: &PathData, ids: &[AnchorId]) -> Option<PathData> {
    let mut subs = decompose(path);
    let mut any = false;
    for (s, sp) in subs.iter_mut().enumerate() {
        if delete_from_subpath(s, sp, ids) {
            any = true;
        }
    }
    any.then(|| recompose(&subs))
}

fn delete_from_subpath(subpath_index: usize, sp: &mut SubPath, ids: &[AnchorId]) -> bool {
    let n = sp.anchors.len();
    let delete = deletion_mask(subpath_index, n, ids);
    let delete_count = deletion_count(&delete);
    if delete_count == 0 || !can_delete_anchors(n, delete_count, sp.closed) {
        return false;
    }
    sp.anchors = surviving_anchors(sp, &delete, delete_count);
    true
}

fn deletion_mask(subpath_index: usize, anchor_count: usize, ids: &[AnchorId]) -> Vec<bool> {
    let mut delete = vec![false; anchor_count];
    for id in ids {
        if id.subpath == subpath_index && id.index < anchor_count {
            delete[id.index] = true;
        }
    }
    delete
}

fn deletion_count(delete: &[bool]) -> usize {
    delete.iter().filter(|&&marked| marked).count()
}

fn can_delete_anchors(anchor_count: usize, delete_count: usize, closed: bool) -> bool {
    let min_keep = if closed { 3 } else { 2 };
    anchor_count - delete_count >= min_keep
}

fn surviving_anchors(sp: &SubPath, delete: &[bool], delete_count: usize) -> Vec<AnchorPt> {
    let mut out = Vec::with_capacity(sp.anchors.len() - delete_count);
    for i in 0..sp.anchors.len() {
        if let Some(anchor) = survivor_anchor(sp, delete, i) {
            out.push(anchor);
        }
    }
    out
}

fn survivor_anchor(sp: &SubPath, delete: &[bool], index: usize) -> Option<AnchorPt> {
    if delete[index] {
        return None;
    }
    let mut anchor = sp.anchors[index];
    if neighbor_deleted(delete, index, sp.closed, -1) {
        anchor.ctrl_in = None;
    }
    if neighbor_deleted(delete, index, sp.closed, 1) {
        anchor.ctrl_out = None;
    }
    Some(anchor)
}

fn neighbor_deleted(delete: &[bool], index: usize, closed: bool, direction: isize) -> bool {
    let n = delete.len();
    if closed {
        return n > 1 && delete[((index as isize + direction).rem_euclid(n as isize)) as usize];
    }
    match direction {
        -1 => index > 0 && delete[index - 1],
        1 => index + 1 < n && delete[index + 1],
        _ => false,
    }
}

// =============================================================================
// Segment evaluation / insertion (de Casteljau)
// =============================================================================

/// The on-curve point of `path.segments[seg_index]` at parameter `t ∈ [0, 1]`.
/// `Close` evaluates the implied straight closing line; `Move` has no extent
/// and returns `None`.
pub fn eval_segment(path: &PathData, seg_index: usize, t: f64) -> Option<DVec2> {
    let (start, sp_start) = segment_start(path, seg_index)?;
    match *path.segments.get(seg_index)? {
        PathSegment::Move { .. } => None,
        PathSegment::Line { to } => Some(start.lerp(pt(to), t)),
        PathSegment::Quad { ctrl, to } => {
            let (q, c) = (pt(ctrl), pt(to));
            let u = 1.0 - t;
            Some(start * (u * u) + q * (2.0 * u * t) + c * (t * t))
        }
        PathSegment::Cubic { ctrl1, ctrl2, to } => {
            let (c1, c2, p1) = (pt(ctrl1), pt(ctrl2), pt(to));
            let u = 1.0 - t;
            Some(
                start * (u * u * u)
                    + c1 * (3.0 * u * u * t)
                    + c2 * (3.0 * u * t * t)
                    + p1 * (t * t * t),
            )
        }
        PathSegment::Close => Some(start.lerp(sp_start, t)),
    }
}

pub fn bend_segment(
    path: &PathData,
    segment_index: usize,
    parameter: f64,
    delta: DVec2,
) -> Option<PathData> {
    if !parameter.is_finite()
        || parameter <= 0.
        || parameter >= 1.
        || !delta.is_finite()
        || !path_is_finite(path)
    {
        return None;
    }
    let (start, subpath_start) = segment_start(path, segment_index)?;
    let segment = *path.segments.get(segment_index)?;
    let (first, second, end) = match segment {
        PathSegment::Move { .. } => return None,
        PathSegment::Line { to } => {
            let end = pt(to);
            (start.lerp(end, 1. / 3.), start.lerp(end, 2. / 3.), end)
        }
        PathSegment::Quad { ctrl, to } => {
            let end = pt(to);
            let control = pt(ctrl);
            (
                start + (control - start) * (2. / 3.),
                end + (control - end) * (2. / 3.),
                end,
            )
        }
        PathSegment::Cubic { ctrl1, ctrl2, to } => (pt(ctrl1), pt(ctrl2), pt(to)),
        PathSegment::Close => (
            start.lerp(subpath_start, 1. / 3.),
            start.lerp(subpath_start, 2. / 3.),
            subpath_start,
        ),
    };
    if delta == DVec2::ZERO {
        return Some(path.clone());
    }
    // The two cubic control weights sum to 3t(1-t). Moving both controls
    // by this amount keeps the grabbed point under the pointer.
    let control_delta = delta / (3. * parameter * (1. - parameter));
    let first = first + control_delta;
    let second = second + control_delta;
    if !first.is_finite() || !second.is_finite() {
        return None;
    }
    let mut result = path.clone();
    let cubic = PathSegment::Cubic {
        ctrl1: first.to_array(),
        ctrl2: second.to_array(),
        to: end.to_array(),
    };
    if segment == PathSegment::Close {
        result.segments.insert(segment_index, cubic);
    } else {
        *result.segments.get_mut(segment_index)? = cubic;
    }
    Some(result)
}

pub(crate) fn segment_anchors(path: &PathData, segment_index: usize) -> Option<[AnchorId; 2]> {
    let subpaths = decompose(path);
    let mut subpath_index: Option<usize> = None;
    let mut anchor_index = 0;
    for (index, segment) in path.segments.iter().enumerate() {
        match segment {
            PathSegment::Move { .. } => {
                subpath_index = Some(subpath_index.map_or(0, |index| index + 1));
                anchor_index = 0;
                if index == segment_index {
                    return None;
                }
            }
            _ => {
                let subpath = subpath_index?;
                let anchors = subpaths.get(subpath)?;
                let start = anchor_index;
                let end = if matches!(segment, PathSegment::Close) {
                    0
                } else {
                    anchor_index + 1
                };
                if index == segment_index {
                    // Decomposition folds an explicit closing endpoint into anchor zero.
                    let end = if end == anchors.anchors.len()
                        && anchors.closed
                        && matches!(path.segments.get(index + 1), Some(PathSegment::Close))
                    {
                        0
                    } else {
                        end
                    };
                    if start >= anchors.anchors.len()
                        || end >= anchors.anchors.len()
                        || start == end
                    {
                        return None;
                    }
                    return Some([
                        AnchorId {
                            subpath,
                            index: start,
                        },
                        AnchorId {
                            subpath,
                            index: end,
                        },
                    ]);
                }
                anchor_index = end;
            }
        }
    }
    None
}

/// The current point just BEFORE `seg_index`, and the start point of its
/// subpath (the closing-line target). `None` when the index is out of range
/// or no current point exists (e.g. the segment IS the first `Move`).
fn segment_start(path: &PathData, seg_index: usize) -> Option<(DVec2, DVec2)> {
    if seg_index >= path.segments.len() {
        return None;
    }
    let mut cur: Option<DVec2> = None;
    let mut sp_start = DVec2::ZERO;
    for seg in &path.segments[..seg_index] {
        match *seg {
            PathSegment::Move { to } => {
                sp_start = pt(to);
                cur = Some(pt(to));
            }
            PathSegment::Line { to }
            | PathSegment::Quad { to, .. }
            | PathSegment::Cubic { to, .. } => cur = Some(pt(to)),
            PathSegment::Close => cur = Some(sp_start),
        }
    }
    cur.map(|c| (c, sp_start))
}

/// Split `path.segments[seg_index]` at parameter `t`, inserting a new on-curve
/// anchor there. Lines lerp; quads/cubics split exactly via de Casteljau (both
/// halves reproduce the original curve). `Close` splits the implied closing
/// line by materializing a `Line` to the split point before it. Returns the
/// new path and the world position of the inserted anchor; `None` for `Move`,
/// out-of-range indices, or a `Close` with no extent.
pub fn insert_at(path: &PathData, seg_index: usize, t: f64) -> Option<(PathData, DVec2)> {
    let t = t.clamp(0.0, 1.0);
    let (start, sp_start) = segment_start(path, seg_index)?;
    let seg = *path.segments.get(seg_index)?;
    let (replacement, point): (Vec<PathSegment>, DVec2) = match seg {
        PathSegment::Move { .. } => return None,
        PathSegment::Line { to } => {
            let m = start.lerp(pt(to), t);
            (
                vec![
                    PathSegment::Line { to: [m.x, m.y] },
                    PathSegment::Line { to },
                ],
                m,
            )
        }
        PathSegment::Quad { ctrl, to } => {
            let (q0, q1, q2) = (start, pt(ctrl), pt(to));
            let a1 = q0.lerp(q1, t);
            let b1 = q1.lerp(q2, t);
            let m = a1.lerp(b1, t);
            (
                vec![
                    PathSegment::Quad {
                        ctrl: [a1.x, a1.y],
                        to: [m.x, m.y],
                    },
                    PathSegment::Quad {
                        ctrl: [b1.x, b1.y],
                        to,
                    },
                ],
                m,
            )
        }
        PathSegment::Cubic { ctrl1, ctrl2, to } => {
            let (p0, p1, p2, p3) = (start, pt(ctrl1), pt(ctrl2), pt(to));
            let p01 = p0.lerp(p1, t);
            let p12 = p1.lerp(p2, t);
            let p23 = p2.lerp(p3, t);
            let p012 = p01.lerp(p12, t);
            let p123 = p12.lerp(p23, t);
            let m = p012.lerp(p123, t);
            (
                vec![
                    PathSegment::Cubic {
                        ctrl1: [p01.x, p01.y],
                        ctrl2: [p012.x, p012.y],
                        to: [m.x, m.y],
                    },
                    PathSegment::Cubic {
                        ctrl1: [p123.x, p123.y],
                        ctrl2: [p23.x, p23.y],
                        to,
                    },
                ],
                m,
            )
        }
        PathSegment::Close => {
            if (sp_start - start).length() <= COINCIDENT_EPS {
                return None; // closing line has no extent
            }
            let m = start.lerp(sp_start, t);
            // Materialize the first half; `Close` still draws m → start.
            (
                vec![PathSegment::Line { to: [m.x, m.y] }, PathSegment::Close],
                m,
            )
        }
    };
    let mut segments = Vec::with_capacity(path.segments.len() + 1);
    segments.extend_from_slice(&path.segments[..seg_index]);
    segments.extend(replacement);
    segments.extend_from_slice(&path.segments[seg_index + 1..]);
    Some((
        PathData {
            segments,
            fill_rule: path.fill_rule,
            // A hand edit invalidates any imported per-subpath rule alignment;
            // fall back to the single fill_rule.
            subpath_rules: Vec::new(),
        },
        point,
    ))
}

// =============================================================================
// Proximity queries
// =============================================================================

/// Nearest on-curve point of the whole path to `p` (path-local coordinates).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathHit {
    /// Index into `path.segments` of the matched segment.
    pub seg_index: usize,
    /// Curve parameter of the closest point on that segment.
    pub t: f64,
    /// The closest on-curve point.
    pub pos: DVec2,
    /// Distance from `p` to `pos`.
    pub dist: f64,
}

const CLOSEST_POINT_COARSE_STEPS: usize = 24;
const CLOSEST_POINT_REFINE_STEPS: usize = 24;

/// Find the closest on-curve point across every drawable segment (lines,
/// quads, cubics, and the implied `Close` line) by coarse sampling plus local
/// refinement. Accurate to well under a pixel at editing zoom levels.
pub fn closest_point_on_path(path: &PathData, p: DVec2) -> Option<PathHit> {
    path.segments
        .iter()
        .enumerate()
        .filter_map(|(i, seg)| closest_point_on_segment(path, i, *seg, p))
        .min_by(|a, b| a.dist.total_cmp(&b.dist))
}

fn closest_point_on_segment(
    path: &PathData,
    seg_index: usize,
    seg: PathSegment,
    p: DVec2,
) -> Option<PathHit> {
    if matches!(seg, PathSegment::Move { .. }) {
        return None;
    }
    let coarse_t = coarse_closest_t(path, seg_index, p)?;
    let t = refine_closest_t(path, seg_index, p, coarse_t);
    let pos = eval_segment(path, seg_index, t)?;
    Some(PathHit {
        seg_index,
        t,
        pos,
        dist: (pos - p).length(),
    })
}

fn coarse_closest_t(path: &PathData, seg_index: usize, p: DVec2) -> Option<f64> {
    let mut best_t = 0.0;
    let mut best_dist = f64::INFINITY;
    for k in 0..=CLOSEST_POINT_COARSE_STEPS {
        let t = k as f64 / CLOSEST_POINT_COARSE_STEPS as f64;
        let dist = segment_distance(path, seg_index, t, p)?;
        if dist < best_dist {
            best_dist = dist;
            best_t = t;
        }
    }
    best_dist.is_finite().then_some(best_t)
}

fn refine_closest_t(path: &PathData, seg_index: usize, p: DVec2, coarse_t: f64) -> f64 {
    let mut lo = (coarse_t - 1.0 / CLOSEST_POINT_COARSE_STEPS as f64).max(0.0);
    let mut hi = (coarse_t + 1.0 / CLOSEST_POINT_COARSE_STEPS as f64).min(1.0);
    for _ in 0..CLOSEST_POINT_REFINE_STEPS {
        let m1 = lo + (hi - lo) / 3.0;
        let m2 = hi - (hi - lo) / 3.0;
        if segment_distance_or_inf(path, seg_index, m1, p)
            < segment_distance_or_inf(path, seg_index, m2, p)
        {
            hi = m2;
        } else {
            lo = m1;
        }
    }
    (lo + hi) * 0.5
}

fn segment_distance(path: &PathData, seg_index: usize, t: f64, p: DVec2) -> Option<f64> {
    eval_segment(path, seg_index, t).map(|q| (q - p).length())
}

fn segment_distance_or_inf(path: &PathData, seg_index: usize, t: f64, p: DVec2) -> f64 {
    segment_distance(path, seg_index, t, p).unwrap_or(f64::INFINITY)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f64, y: f64) -> DVec2 {
        DVec2::new(x, y)
    }

    /// Open polyline: M(0,0) L(100,0) L(100,100).
    fn open_polyline() -> PathData {
        let mut p = PathData::new();
        p.move_to(0.0, 0.0)
            .line_to(100.0, 0.0)
            .line_to(100.0, 100.0);
        p
    }

    /// Closed triangle via implicit closing line: M(0,0) L(100,0) L(50,80) Z.
    fn closed_triangle() -> PathData {
        let mut p = PathData::new();
        p.move_to(0.0, 0.0)
            .line_to(100.0, 0.0)
            .line_to(50.0, 80.0)
            .close();
        p
    }

    /// Pen-style closed path: the last cubic returns to the start, then Close.
    fn pen_style_closed() -> PathData {
        let mut p = PathData::new();
        p.move_to(0.0, 0.0)
            .cubic_to(10.0, -20.0, 40.0, -20.0, 50.0, 0.0)
            .line_to(50.0, 50.0)
            .cubic_to(40.0, 70.0, 10.0, 70.0, 0.0, 0.0)
            .close();
        p
    }

    /// Two subpaths: an open V and a closed square.
    fn multi_subpath() -> PathData {
        let mut p = PathData::new();
        p.move_to(0.0, 0.0).line_to(10.0, 10.0).line_to(20.0, 0.0);
        p.move_to(50.0, 50.0)
            .line_to(60.0, 50.0)
            .line_to(60.0, 60.0)
            .line_to(50.0, 60.0)
            .close();
        p
    }

    // -- enumerate_anchors ----------------------------------------------------

    #[test]
    fn enumerate_open_polyline_yields_three_plain_anchors() {
        let a = enumerate_anchors(&open_polyline());
        assert_eq!(a.len(), 3);
        assert_eq!(a[0].pos, v(0.0, 0.0));
        assert_eq!(a[1].pos, v(100.0, 0.0));
        assert_eq!(a[2].pos, v(100.0, 100.0));
        for x in &a {
            assert!(x.ctrl_in.is_none() && x.ctrl_out.is_none());
            assert!(!x.closed);
            assert_eq!(x.id.subpath, 0);
        }
    }

    #[test]
    fn enumerate_multi_subpath_separates_and_flags_closed() {
        let a = enumerate_anchors(&multi_subpath());
        assert_eq!(a.len(), 7); // 3 open + 4 closed
        assert_eq!(a.iter().filter(|x| x.id.subpath == 0).count(), 3);
        assert_eq!(a.iter().filter(|x| x.id.subpath == 1).count(), 4);
        assert!(a.iter().filter(|x| x.id.subpath == 0).all(|x| !x.closed));
        assert!(a.iter().filter(|x| x.id.subpath == 1).all(|x| x.closed));
    }

    #[test]
    fn enumerate_cubic_attaches_controls_to_both_sides() {
        let mut p = PathData::new();
        p.move_to(0.0, 0.0)
            .cubic_to(10.0, 20.0, 40.0, 20.0, 50.0, 0.0);
        let a = enumerate_anchors(&p);
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].ctrl_out, Some(v(10.0, 20.0)));
        assert_eq!(a[0].ctrl_in, None);
        assert_eq!(a[1].ctrl_in, Some(v(40.0, 20.0)));
        assert_eq!(a[1].ctrl_out, None);
    }

    #[test]
    fn enumerate_quad_degree_elevates_exactly() {
        let mut p = PathData::new();
        p.move_to(0.0, 0.0).quad_to(30.0, 60.0, 60.0, 0.0);
        let a = enumerate_anchors(&p);
        assert_eq!(a.len(), 2);
        // c1 = p0 + 2/3 (q - p0) = (20, 40); c2 = p1 + 2/3 (q - p1) = (40, 40).
        let c1 = a[0].ctrl_out.expect("ctrl_out");
        let c2 = a[1].ctrl_in.expect("ctrl_in");
        assert!((c1 - v(20.0, 40.0)).length() < 1e-9);
        assert!((c2 - v(40.0, 40.0)).length() < 1e-9);
        // The elevated cubic is pointwise identical to the original quad.
        let cubic = recompose(&decompose(&p));
        for k in 0..=20 {
            let t = k as f64 / 20.0;
            let q = eval_segment(&p, 1, t).expect("quad eval");
            let c = eval_segment(&cubic, 1, t).expect("cubic eval");
            assert!((q - c).length() < 1e-9, "t={t}: {q:?} vs {c:?}");
        }
    }

    #[test]
    fn enumerate_pen_style_closed_folds_wraparound_anchor() {
        let a = enumerate_anchors(&pen_style_closed());
        // M + cubic-end + line-end (the explicit return-to-start endpoint folds
        // into anchor 0).
        assert_eq!(a.len(), 3);
        assert!(a.iter().all(|x| x.closed));
        // Anchor 0 carries the closing cubic's incoming control.
        assert_eq!(a[0].ctrl_in, Some(v(10.0, 70.0)));
        assert_eq!(a[0].ctrl_out, Some(v(10.0, -20.0)));
    }

    #[test]
    fn decompose_recompose_round_trips_geometry() {
        for p in [
            open_polyline(),
            closed_triangle(),
            pen_style_closed(),
            multi_subpath(),
        ] {
            let rt = recompose(&decompose(&p));
            // Anchor sets must match exactly.
            let a0 = enumerate_anchors(&p);
            let a1 = enumerate_anchors(&rt);
            assert_eq!(a0, a1, "anchors drifted through round-trip");
        }
    }

    // -- move_anchor ----------------------------------------------------------

    #[test]
    fn move_anchor_translates_point_and_attached_controls() {
        let mut p = PathData::new();
        p.move_to(0.0, 0.0)
            .cubic_to(10.0, 20.0, 40.0, 20.0, 50.0, 0.0)
            .cubic_to(60.0, -20.0, 90.0, -20.0, 100.0, 0.0);
        let moved = move_anchor(
            &p,
            AnchorId {
                subpath: 0,
                index: 1,
            },
            v(50.0, 10.0),
        )
        .expect("move");
        let a = enumerate_anchors(&moved);
        assert_eq!(a[1].pos, v(50.0, 10.0));
        // Both attached controls rode along by (0, +10).
        assert_eq!(a[1].ctrl_in, Some(v(40.0, 30.0)));
        assert_eq!(a[1].ctrl_out, Some(v(60.0, -10.0)));
        // The far controls of the neighboring segments did NOT move.
        assert_eq!(a[0].ctrl_out, Some(v(10.0, 20.0)));
        assert_eq!(a[2].ctrl_in, Some(v(90.0, -20.0)));
    }

    #[test]
    fn move_anchor_on_line_endpoints_keeps_lines() {
        let moved = move_anchor(
            &open_polyline(),
            AnchorId {
                subpath: 0,
                index: 0,
            },
            v(-10.0, -10.0),
        )
        .expect("move");
        let a = enumerate_anchors(&moved);
        assert_eq!(a[0].pos, v(-10.0, -10.0));
        assert!(
            moved
                .segments
                .iter()
                .all(|s| !matches!(s, PathSegment::Cubic { .. })),
            "no spurious curves from moving a line anchor"
        );
    }

    #[test]
    fn move_anchor_out_of_range_is_none() {
        assert!(
            move_anchor(
                &open_polyline(),
                AnchorId {
                    subpath: 0,
                    index: 9
                },
                v(0.0, 0.0)
            )
            .is_none()
        );
        assert!(
            move_anchor(
                &open_polyline(),
                AnchorId {
                    subpath: 3,
                    index: 0
                },
                v(0.0, 0.0)
            )
            .is_none()
        );
    }

    #[test]
    fn move_first_anchor_of_closed_path_moves_closing_edge_control() {
        let moved = move_anchor(
            &pen_style_closed(),
            AnchorId {
                subpath: 0,
                index: 0,
            },
            v(5.0, 5.0),
        )
        .expect("move");
        let a = enumerate_anchors(&moved);
        assert_eq!(a[0].pos, v(5.0, 5.0));
        // ctrl_in (10,70) and ctrl_out (10,-20) translated by (+5,+5).
        assert_eq!(a[0].ctrl_in, Some(v(15.0, 75.0)));
        assert_eq!(a[0].ctrl_out, Some(v(15.0, -15.0)));
    }

    // -- set_handle -----------------------------------------------------------

    #[test]
    fn set_handle_mirror_keeps_opposite_length_and_flips_angle() {
        let mut p = PathData::new();
        p.move_to(0.0, 0.0)
            .cubic_to(10.0, 20.0, 30.0, 20.0, 50.0, 0.0)
            .cubic_to(70.0, -20.0, 90.0, -20.0, 100.0, 0.0);
        let id = AnchorId {
            subpath: 0,
            index: 1,
        };
        // Drag the OUT handle of the middle anchor to (50+30, 30).
        let edited = set_handle(&p, id, HandleSide::Out, v(80.0, 30.0), true).expect("set");
        let a = enumerate_anchors(&edited)[1];
        assert_eq!(a.ctrl_out, Some(v(80.0, 30.0)));
        let ci = a.ctrl_in.expect("mirrored in-handle");
        // Length preserved: original in-handle was (30,20), len = |(−20,20)|.
        let orig_len = (v(30.0, 20.0) - v(50.0, 0.0)).length();
        assert!(((ci - a.pos).length() - orig_len).abs() < 1e-9);
        // Collinear, opposite side: in-handle direction = −out direction.
        let out_dir = (v(80.0, 30.0) - a.pos).normalize();
        let in_dir = (ci - a.pos).normalize();
        assert!((out_dir + in_dir).length() < 1e-9, "handles not collinear");
        assert!(is_smooth(&AnchorPt {
            pos: a.pos,
            ctrl_in: a.ctrl_in,
            ctrl_out: a.ctrl_out,
        }));
    }

    #[test]
    fn set_handle_without_mirror_breaks_to_corner() {
        let mut p = PathData::new();
        p.move_to(0.0, 0.0)
            .cubic_to(10.0, 20.0, 30.0, 20.0, 50.0, 0.0)
            .cubic_to(70.0, -20.0, 90.0, -20.0, 100.0, 0.0);
        let id = AnchorId {
            subpath: 0,
            index: 1,
        };
        let edited = set_handle(&p, id, HandleSide::Out, v(80.0, 30.0), false).expect("set");
        let a = enumerate_anchors(&edited)[1];
        assert_eq!(a.ctrl_out, Some(v(80.0, 30.0)));
        // The opposite handle did not move.
        assert_eq!(a.ctrl_in, Some(v(30.0, 20.0)));
    }

    #[test]
    fn set_handle_on_straight_edge_promotes_to_cubic() {
        let edited = set_handle(
            &open_polyline(),
            AnchorId {
                subpath: 0,
                index: 1,
            },
            HandleSide::In,
            v(50.0, -30.0),
            false,
        )
        .expect("set");
        assert!(
            edited
                .segments
                .iter()
                .any(|s| matches!(s, PathSegment::Cubic { .. })),
            "edge promoted to a curve"
        );
        let a = enumerate_anchors(&edited)[1];
        assert_eq!(a.ctrl_in, Some(v(50.0, -30.0)));
        // The other edge of the anchor stays a line.
        assert_eq!(a.ctrl_out, None);
    }

    // -- is_smooth / toggle_smooth ---------------------------------------------

    #[test]
    fn is_smooth_respects_one_degree_tolerance() {
        let pos = v(0.0, 0.0);
        let smooth = AnchorPt {
            pos,
            ctrl_in: Some(v(-10.0, 0.0)),
            ctrl_out: Some(v(10.0, 0.0)),
        };
        assert!(is_smooth(&smooth));
        // 0.5° off — still smooth.
        let half_deg = 0.5_f64.to_radians();
        let nearly = AnchorPt {
            pos,
            ctrl_in: Some(v(-10.0, 0.0)),
            ctrl_out: Some(v(10.0 * half_deg.cos(), 10.0 * half_deg.sin())),
        };
        assert!(is_smooth(&nearly));
        // 5° off — corner.
        let five_deg = 5.0_f64.to_radians();
        let bent = AnchorPt {
            pos,
            ctrl_in: Some(v(-10.0, 0.0)),
            ctrl_out: Some(v(10.0 * five_deg.cos(), 10.0 * five_deg.sin())),
        };
        assert!(!is_smooth(&bent));
        // Missing a handle — corner.
        assert!(!is_smooth(&AnchorPt {
            pos,
            ctrl_in: None,
            ctrl_out: Some(v(10.0, 0.0)),
        }));
    }

    #[test]
    fn toggle_smooth_corner_to_smooth_builds_third_chord_handles() {
        let p = open_polyline(); // corner at (100, 0) between two 100-long edges
        let id = AnchorId {
            subpath: 0,
            index: 1,
        };
        let sm = toggle_smooth(&p, id).expect("toggle");
        let a = enumerate_anchors(&sm)[1];
        let ci = a.ctrl_in.expect("in handle");
        let co = a.ctrl_out.expect("out handle");
        // Direction = next − prev = (100,100) − (0,0), normalized.
        let dir = v(100.0, 100.0).normalize();
        // Lengths: 1/3 of each adjacent chord (both chords are 100).
        assert!((ci - (a.pos - dir * (100.0 / 3.0))).length() < 1e-9);
        assert!((co - (a.pos + dir * (100.0 / 3.0))).length() < 1e-9);
        assert!(anchor_is_smooth(&sm, id));
    }

    #[test]
    fn toggle_smooth_smooth_to_corner_retracts_handles_and_demotes_to_lines() {
        let p = open_polyline();
        let id = AnchorId {
            subpath: 0,
            index: 1,
        };
        let sm = toggle_smooth(&p, id).expect("smooth");
        let back = toggle_smooth(&sm, id).expect("corner");
        let a = enumerate_anchors(&back)[1];
        assert!(a.ctrl_in.is_none() && a.ctrl_out.is_none());
        // Fully handle-free edges demote back to plain lines.
        assert!(
            back.segments
                .iter()
                .all(|s| !matches!(s, PathSegment::Cubic { .. } | PathSegment::Quad { .. }))
        );
    }

    #[test]
    fn toggle_smooth_open_endpoint_grows_only_inner_handle() {
        let p = open_polyline();
        let sm = toggle_smooth(
            &p,
            AnchorId {
                subpath: 0,
                index: 0,
            },
        )
        .expect("toggle");
        let a = enumerate_anchors(&sm)[0];
        assert!(a.ctrl_in.is_none(), "no incoming edge on an open start");
        let co = a.ctrl_out.expect("outgoing handle");
        // Chord (0,0)→(100,0): handle = pos + dir·(100/3).
        assert!((co - v(100.0 / 3.0, 0.0)).length() < 1e-9);
    }

    #[test]
    fn toggle_smooth_closed_path_wraps_neighbors() {
        let p = closed_triangle();
        let id = AnchorId {
            subpath: 0,
            index: 0,
        }; // start anchor: prev wraps to (50,80), next is (100,0)
        let sm = toggle_smooth(&p, id).expect("toggle");
        let a = enumerate_anchors(&sm)[0];
        let ci = a.ctrl_in.expect("wrap in-handle");
        let co = a.ctrl_out.expect("out-handle");
        let dir = (v(100.0, 0.0) - v(50.0, 80.0)).normalize();
        let prev_chord = (v(0.0, 0.0) - v(50.0, 80.0)).length();
        let next_chord = 100.0;
        assert!((ci - (a.pos - dir * (prev_chord / 3.0))).length() < 1e-9);
        assert!((co - (a.pos + dir * (next_chord / 3.0))).length() < 1e-9);
    }

    // -- insert_at ------------------------------------------------------------

    #[test]
    fn insert_on_line_lerps_and_keeps_endpoints() {
        let p = open_polyline();
        let (split, m) = insert_at(&p, 1, 0.25).expect("insert");
        assert!((m - v(25.0, 0.0)).length() < 1e-9);
        let a = enumerate_anchors(&split);
        assert_eq!(a.len(), 4);
        assert_eq!(a[1].pos, v(25.0, 0.0));
        assert_eq!(a[2].pos, v(100.0, 0.0));
    }

    #[test]
    fn insert_on_cubic_halves_reproduce_original_curve() {
        let mut p = PathData::new();
        p.move_to(0.0, 0.0)
            .cubic_to(20.0, 50.0, 80.0, -50.0, 100.0, 0.0);
        let t_split = 0.37;
        let (split, m) = insert_at(&p, 1, t_split).expect("insert");
        assert_eq!(split.segments.len(), 3); // Move + two cubics
        // The split point lies on the original curve.
        let orig_at_split = eval_segment(&p, 1, t_split).expect("eval");
        assert!((m - orig_at_split).length() < 1e-9);
        // Sample both halves against the original parameterization.
        for k in 0..=32 {
            let t = k as f64 / 32.0;
            let orig = eval_segment(&p, 1, t).expect("orig eval");
            let (idx, local_t) = if t <= t_split {
                (1, t / t_split)
            } else {
                (2, (t - t_split) / (1.0 - t_split))
            };
            let half = eval_segment(&split, idx, local_t).expect("half eval");
            assert!(
                (orig - half).length() < 1e-9,
                "t={t}: original {orig:?} vs split {half:?}"
            );
        }
    }

    #[test]
    fn insert_on_quad_halves_reproduce_original_curve() {
        let mut p = PathData::new();
        p.move_to(0.0, 0.0).quad_to(50.0, 80.0, 100.0, 0.0);
        let t_split = 0.6;
        let (split, m) = insert_at(&p, 1, t_split).expect("insert");
        let orig_at_split = eval_segment(&p, 1, t_split).expect("eval");
        assert!((m - orig_at_split).length() < 1e-9);
        for k in 0..=32 {
            let t = k as f64 / 32.0;
            let orig = eval_segment(&p, 1, t).expect("orig eval");
            let (idx, local_t) = if t <= t_split {
                (1, t / t_split)
            } else {
                (2, (t - t_split) / (1.0 - t_split))
            };
            let half = eval_segment(&split, idx, local_t).expect("half eval");
            assert!((orig - half).length() < 1e-9, "t={t}");
        }
    }

    #[test]
    fn insert_on_close_materializes_split_of_implied_line() {
        let p = closed_triangle();
        // Close is segments[3]; the implied line runs (50,80) → (0,0).
        let (split, m) = insert_at(&p, 3, 0.5).expect("insert");
        assert!((m - v(25.0, 40.0)).length() < 1e-9);
        let a = enumerate_anchors(&split);
        assert_eq!(a.len(), 4);
        assert_eq!(a[3].pos, v(25.0, 40.0));
        assert!(matches!(split.segments.last(), Some(PathSegment::Close)));
    }

    #[test]
    fn insert_rejects_move_and_out_of_range() {
        let p = open_polyline();
        assert!(insert_at(&p, 0, 0.5).is_none(), "Move has no extent");
        assert!(insert_at(&p, 9, 0.5).is_none());
    }

    // -- delete_anchors ---------------------------------------------------------

    #[test]
    fn delete_middle_anchor_rejoins_neighbors_with_line() {
        let mut p = PathData::new();
        p.move_to(0.0, 0.0)
            .cubic_to(10.0, 20.0, 40.0, 20.0, 50.0, 0.0)
            .cubic_to(60.0, -20.0, 90.0, -20.0, 100.0, 0.0);
        let del = delete_anchors(
            &p,
            &[AnchorId {
                subpath: 0,
                index: 1,
            }],
        )
        .expect("delete");
        let a = enumerate_anchors(&del);
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].pos, v(0.0, 0.0));
        assert_eq!(a[1].pos, v(100.0, 0.0));
        // The rejoined edge is a straight line (facing controls cleared).
        assert!(a[0].ctrl_out.is_none());
        assert!(a[1].ctrl_in.is_none());
        assert!(matches!(del.segments[1], PathSegment::Line { .. }));
    }

    #[test]
    fn delete_first_anchor_of_open_path_advances_the_move() {
        let del = delete_anchors(
            &open_polyline(),
            &[AnchorId {
                subpath: 0,
                index: 0,
            }],
        )
        .expect("delete");
        let a = enumerate_anchors(&del);
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].pos, v(100.0, 0.0));
        assert!(matches!(del.segments[0], PathSegment::Move { .. }));
    }

    #[test]
    fn delete_open_guard_keeps_minimum_two_anchors() {
        // 3 anchors open: deleting two would leave 1 < 2 → whole request skipped.
        let p = open_polyline();
        let result = delete_anchors(
            &p,
            &[
                AnchorId {
                    subpath: 0,
                    index: 0,
                },
                AnchorId {
                    subpath: 0,
                    index: 1,
                },
            ],
        );
        assert!(result.is_none(), "guard blocks dropping below 2 anchors");
        // Deleting ONE is fine (3 − 1 = 2).
        assert!(
            delete_anchors(
                &p,
                &[AnchorId {
                    subpath: 0,
                    index: 1
                }]
            )
            .is_some()
        );
    }

    #[test]
    fn delete_closed_guard_keeps_minimum_three_anchors() {
        // The triangle has exactly 3 — any deletion would breach the minimum.
        let result = delete_anchors(
            &closed_triangle(),
            &[AnchorId {
                subpath: 0,
                index: 1,
            }],
        );
        assert!(result.is_none(), "closed paths keep at least 3 anchors");
        // A closed square (4 anchors) can lose one.
        let sq = {
            let mut p = PathData::new();
            p.move_to(0.0, 0.0)
                .line_to(10.0, 0.0)
                .line_to(10.0, 10.0)
                .line_to(0.0, 10.0)
                .close();
            p
        };
        let del = delete_anchors(
            &sq,
            &[AnchorId {
                subpath: 0,
                index: 2,
            }],
        )
        .expect("delete");
        assert_eq!(enumerate_anchors(&del).len(), 3);
    }

    #[test]
    fn delete_applies_per_subpath_with_independent_guards() {
        // Subpath 0 (open V, 3 anchors): delete 1 — allowed.
        // Subpath 1 (closed square, 4 anchors): delete 2 — would leave 2 < 3,
        // so that subpath is left untouched.
        let p = multi_subpath();
        let del = delete_anchors(
            &p,
            &[
                AnchorId {
                    subpath: 0,
                    index: 1,
                },
                AnchorId {
                    subpath: 1,
                    index: 0,
                },
                AnchorId {
                    subpath: 1,
                    index: 2,
                },
            ],
        )
        .expect("delete");
        let a = enumerate_anchors(&del);
        assert_eq!(a.iter().filter(|x| x.id.subpath == 0).count(), 2);
        assert_eq!(a.iter().filter(|x| x.id.subpath == 1).count(), 4);
    }

    #[test]
    fn delete_first_anchor_of_closed_square_keeps_it_closed() {
        let sq = {
            let mut p = PathData::new();
            p.move_to(0.0, 0.0)
                .line_to(10.0, 0.0)
                .line_to(10.0, 10.0)
                .line_to(0.0, 10.0)
                .close();
            p
        };
        let del = delete_anchors(
            &sq,
            &[AnchorId {
                subpath: 0,
                index: 0,
            }],
        )
        .expect("delete");
        let a = enumerate_anchors(&del);
        assert_eq!(a.len(), 3);
        assert!(a.iter().all(|x| x.closed));
        assert_eq!(a[0].pos, v(10.0, 0.0));
    }

    // -- eval / closest point ----------------------------------------------------

    #[test]
    fn eval_segment_close_walks_implied_line() {
        let p = closed_triangle();
        let mid = eval_segment(&p, 3, 0.5).expect("close eval");
        assert!((mid - v(25.0, 40.0)).length() < 1e-9);
    }

    #[test]
    fn closest_point_on_path_finds_line_projection() {
        let p = open_polyline();
        let hit = closest_point_on_path(&p, v(40.0, 7.0)).expect("hit");
        assert_eq!(hit.seg_index, 1);
        assert!((hit.pos - v(40.0, 0.0)).length() < 1e-3);
        assert!((hit.dist - 7.0).abs() < 1e-3);
        assert!((hit.t - 0.4).abs() < 1e-3);
    }

    #[test]
    fn closest_point_on_path_reaches_the_implied_close_edge() {
        let p = closed_triangle();
        // A point near the closing edge (50,80) → (0,0), e.g. near (25,40).
        let hit = closest_point_on_path(&p, v(20.0, 42.0)).expect("hit");
        assert_eq!(hit.seg_index, 3, "matched the Close segment");
        assert!(hit.dist < 6.0);
    }

    #[test]
    fn closest_point_on_cubic_is_on_curve() {
        let mut p = PathData::new();
        p.move_to(0.0, 0.0)
            .cubic_to(20.0, 60.0, 80.0, 60.0, 100.0, 0.0);
        let probe = v(50.0, 50.0);
        let hit = closest_point_on_path(&p, probe).expect("hit");
        // Verify it's a true local minimum: nudging t either way is no closer.
        for dt in [-0.01, 0.01] {
            let q = eval_segment(&p, hit.seg_index, (hit.t + dt).clamp(0.0, 1.0)).expect("eval");
            assert!((q - probe).length() >= hit.dist - 1e-9);
        }
        // And on the curve at t=0.5 the apex (50, 45) is the nearest region.
        assert!((hit.pos.x - 50.0).abs() < 2.0);
    }

    #[test]
    fn close_path_closes_open_contours_and_preserves_fill_rules_and_curves() {
        let mut path = PathData::new();
        path.move_to(0., 0.).quad_to(5., 10., 10., 0.);
        path.move_to(20., 0.)
            .line_to(30., 0.)
            .line_to(25., 10.)
            .close();
        path.move_to(40., 0.).line_to(50., 0.);
        path.fill_rule = fanta_doc::FillRule::EvenOdd;
        path.subpath_rules = vec![fanta_doc::FillRule::EvenOdd, fanta_doc::FillRule::NonZero];
        let closed = close_path(&path).expect("close open contours");
        assert_eq!(closed.fill_rule, path.fill_rule);
        assert_eq!(closed.subpath_rules, path.subpath_rules);
        assert_eq!(closed.segments.get(1), path.segments.get(1));
        assert_eq!(
            closed
                .segments
                .iter()
                .filter(|segment| matches!(segment, PathSegment::Close))
                .count(),
            3
        );
        assert!(decompose(&closed).iter().all(|subpath| subpath.closed));
        assert!(close_path(&closed).is_none());
    }

    #[test]
    fn close_path_does_not_close_an_isolated_anchor() {
        let mut path = PathData::new();
        path.move_to(1., 2.);
        assert!(close_path(&path).is_none());
    }

    #[test]
    fn join_paths_connects_end_to_start_without_changing_curve_handles() {
        let mut first = PathData::new();
        first.move_to(0., 0.).quad_to(5., 10., 10., 0.);
        first.fill_rule = fanta_doc::FillRule::EvenOdd;
        let mut second = PathData::new();
        second
            .move_to(20., 0.)
            .cubic_to(25., 10., 30., -10., 40., 0.);
        let joined = join_paths(&[first.clone(), second.clone()]).expect("join paths");
        assert_eq!(joined.segments.get(..2), Some(first.segments.as_slice()));
        assert_eq!(
            joined.segments.get(2),
            Some(&PathSegment::Line { to: [20., 0.] })
        );
        assert_eq!(joined.segments.last(), second.segments.last());
        assert_eq!(joined.fill_rule, first.fill_rule);
        assert_eq!(decompose(&joined).len(), 1);
        assert!(!decompose(&joined).first().expect("subpath").closed);
    }

    #[test]
    fn join_paths_omits_coincident_connector_and_rejects_closed_or_multiple_contours() {
        let mut first = PathData::new();
        first.move_to(0., 0.).line_to(10., 0.);
        let mut second = PathData::new();
        second.move_to(10., 0.).line_to(20., 0.);
        let joined = join_paths(&[first.clone(), second.clone()]).expect("join paths");
        assert_eq!(joined.segments.len(), 3);
        assert!(join_paths(&[first.clone()]).is_none());
        second.close();
        assert!(join_paths(&[first.clone(), second]).is_none());
        let mut multiple = first.clone();
        multiple.move_to(30., 0.).line_to(40., 0.);
        assert!(join_paths(&[first, multiple]).is_none());
    }

    #[test]
    fn simplify_path_reduces_collinear_anchors_and_preserves_corners() {
        let mut path = PathData::new();
        path.move_to(0., 0.)
            .line_to(10., 0.)
            .line_to(20., 0.)
            .line_to(30., 0.)
            .line_to(30., 20.);
        let simplified = simplify_path(&path, 0.2).expect("simplify");
        assert!(
            simplified.segments.len() < path.segments.len(),
            "{simplified:?}"
        );
        let anchors = enumerate_anchors(&simplified);
        assert_eq!(anchors.first().expect("start").pos, v(0., 0.));
        assert!(anchors.iter().any(|anchor| anchor.pos == v(30., 0.)));
        assert_eq!(anchors.last().expect("end").pos, v(30., 20.));
    }

    #[test]
    fn simplify_path_preserves_closed_contours_and_mixed_fill_rules() {
        let mut path = PathData::rect(0., 0., 20., 20.);
        path.segments
            .extend(PathData::rect(5., 5., 10., 10.).segments);
        path.fill_rule = fanta_doc::FillRule::EvenOdd;
        path.subpath_rules = vec![fanta_doc::FillRule::NonZero, fanta_doc::FillRule::EvenOdd];
        let simplified = simplify_path(&path, 0.2).expect("simplify");
        assert_eq!(simplified.fill_rule, path.fill_rule);
        assert_eq!(simplified.subpath_rules, path.subpath_rules);
        assert_eq!(decompose(&simplified).len(), 2);
        assert!(decompose(&simplified).iter().all(|subpath| subpath.closed));
    }

    #[test]
    fn simplify_path_rejects_invalid_input_and_degenerate_contour_loss() {
        let mut malformed = PathData::new();
        malformed.line_to(10., 0.);
        assert!(simplify_path(&malformed, 0.2).is_none());
        let mut path = PathData::new();
        path.move_to(0., 0.).line_to(10., 0.);
        for tolerance in [0., -1., f64::NAN, f64::INFINITY] {
            assert!(simplify_path(&path, tolerance).is_none());
        }
        let mut nonfinite = path.clone();
        nonfinite.line_to(f64::NAN, 0.);
        assert!(simplify_path(&nonfinite, 0.2).is_none());
        assert!(join_paths(&[path.clone(), nonfinite]).is_none());
        path.move_to(20., 0.);
        assert!(simplify_path(&path, 0.2).is_none());
    }

    #[test]
    fn bend_segment_moves_the_grabbed_point_and_keeps_all_anchors_fixed() {
        for parameter in [0.2, 0.5, 0.8] {
            let mut path = PathData::new();
            path.move_to(0., 0.).line_to(100., 0.).line_to(100., 100.);
            let original_anchors = enumerate_anchors(&path)
                .into_iter()
                .map(|anchor| anchor.pos)
                .collect::<Vec<_>>();
            let delta = v(0., 30.);
            let bent = bend_segment(&path, 1, parameter, delta).expect("bend line");
            let anchors = enumerate_anchors(&bent)
                .into_iter()
                .map(|anchor| anchor.pos)
                .collect::<Vec<_>>();
            assert_eq!(anchors, original_anchors);
            let expected = eval_segment(&path, 1, parameter).expect("original point") + delta;
            assert!(
                (eval_segment(&bent, 1, parameter).expect("bent point") - expected).length() < 1e-9
            );
            assert_eq!(bent.segments.last(), path.segments.last());
        }
    }

    #[test]
    fn bend_segment_preserves_curved_closure_and_fill_metadata() {
        let mut path = closed_triangle();
        path.fill_rule = fanta_doc::FillRule::EvenOdd;
        path.subpath_rules = vec![fanta_doc::FillRule::EvenOdd];
        let bent = bend_segment(&path, 3, 0.5, v(-20., 10.)).expect("bend closing edge");
        assert!(matches!(bent.segments.last(), Some(PathSegment::Close)));
        assert_eq!(
            enumerate_anchors(&bent).len(),
            enumerate_anchors(&path).len()
        );
        assert!(decompose(&bent).first().expect("subpath").closed);
        assert_eq!(bent.fill_rule, path.fill_rule);
        assert_eq!(bent.subpath_rules, path.subpath_rules);
        assert_eq!(bend_segment(&path, 3, 0.5, DVec2::ZERO), Some(path.clone()));
        assert!(bend_segment(&path, 0, 0.5, v(0., 10.)).is_none());
        assert!(bend_segment(&path, 1, 0., v(0., 10.)).is_none());
    }

    #[test]
    fn reversing_paths_swaps_handles_and_preserves_closed_start_and_fill_rules() {
        let mut path = PathData::new();
        path.move_to(0., 0.).cubic_to(20., 60., 80., -40., 100., 0.);
        path.fill_rule = fanta_doc::FillRule::EvenOdd;
        path.subpath_rules = vec![fanta_doc::FillRule::EvenOdd];
        let reversed = reverse_path(&path).expect("reverse");
        for sample in 0..=32 {
            let parameter = f64::from(sample) / 32.;
            let original = eval_segment(&path, 1, 1. - parameter).expect("original point");
            let backwards = eval_segment(&reversed, 1, parameter).expect("reversed point");
            assert!((original - backwards).length() < 1e-9);
        }
        assert_eq!(reverse_path(&reversed), Some(path));
        let closed = pen_style_closed();
        let reversed = reverse_path(&closed).expect("reverse closed");
        assert_eq!(
            enumerate_anchors(&reversed).first().expect("start").pos,
            enumerate_anchors(&closed).first().expect("start").pos
        );
        assert!(decompose(&reversed).first().expect("subpath").closed);
        assert_eq!(reverse_path(&reversed), Some(closed));
    }

    #[test]
    fn breaking_an_open_path_duplicates_the_anchor_and_its_fill_rule() {
        let mut path = PathData::new();
        path.move_to(0., 0.)
            .cubic_to(10., 20., 20., 20., 30., 0.)
            .cubic_to(40., -20., 50., -20., 60., 0.);
        path.segments
            .extend(PathData::rect(100., 0., 20., 20.).segments);
        path.fill_rule = fanta_doc::FillRule::EvenOdd;
        path.subpath_rules = vec![fanta_doc::FillRule::EvenOdd, fanta_doc::FillRule::NonZero];
        let broken = break_at_anchor(
            &path,
            AnchorId {
                subpath: 0,
                index: 1,
            },
        )
        .expect("break");
        assert_eq!(
            broken.subpath_rules,
            vec![
                fanta_doc::FillRule::EvenOdd,
                fanta_doc::FillRule::EvenOdd,
                fanta_doc::FillRule::NonZero
            ]
        );
        let subpaths = decompose(&broken);
        assert_eq!(subpaths.len(), 3);
        assert_eq!(
            subpaths
                .first()
                .expect("first")
                .anchors
                .last()
                .expect("end")
                .pos,
            v(30., 0.)
        );
        assert_eq!(
            subpaths
                .get(1)
                .expect("second")
                .anchors
                .first()
                .expect("start")
                .pos,
            v(30., 0.)
        );
        assert_eq!(broken.segments.get(1), path.segments.get(1));
        assert_eq!(broken.segments.get(3), path.segments.get(2));
        assert!(
            break_at_anchor(
                &path,
                AnchorId {
                    subpath: 0,
                    index: 0
                }
            )
            .is_none()
        );
    }

    #[test]
    fn breaking_a_closed_path_opens_it_without_dropping_the_closing_curve() {
        let path = pen_style_closed();
        let broken = break_at_anchor(
            &path,
            AnchorId {
                subpath: 0,
                index: 1,
            },
        )
        .expect("break closed");
        let subpaths = decompose(&broken);
        let subpath = subpaths.first().expect("open path");
        assert!(!subpath.closed);
        assert_eq!(subpath.anchors.len(), enumerate_anchors(&path).len() + 1);
        assert_eq!(
            subpath.anchors.first().expect("start").pos,
            subpath.anchors.last().expect("end").pos
        );
        for index in 0..path.segments.len() {
            for sample in 0..=16 {
                if let Some(point) = eval_segment(&path, index, f64::from(sample) / 16.) {
                    assert!(
                        closest_point_on_path(&broken, point)
                            .expect("point on broken path")
                            .dist
                            < 0.01
                    );
                }
            }
        }
    }

    #[test]
    fn deleting_a_split_curve_anchor_preserves_the_curve_instead_of_flattening_it() {
        let mut path = PathData::new();
        path.move_to(0., 0.).cubic_to(30., 80., 70., 80., 100., 0.);
        let (split, _) = insert_at(&path, 1, 0.5).expect("split curve");
        let removed = delete_anchors_preserving_shape(
            &split,
            &[AnchorId {
                subpath: 0,
                index: 1,
            }],
            0.05,
        )
        .expect("remove curve anchor");
        assert_eq!(enumerate_anchors(&removed).len(), 2);
        assert!(matches!(
            removed.segments.get(1),
            Some(PathSegment::Cubic { .. })
        ));
        for sample in 0..=64 {
            let point = eval_segment(&path, 1, f64::from(sample) / 64.).expect("original point");
            assert!(
                closest_point_on_path(&removed, point)
                    .expect("fitted point")
                    .dist
                    < 0.1
            );
        }
        assert!(
            delete_anchors_preserving_shape(
                &split,
                &[AnchorId {
                    subpath: 0,
                    index: 1
                }],
                0.
            )
            .is_none()
        );
    }
}
