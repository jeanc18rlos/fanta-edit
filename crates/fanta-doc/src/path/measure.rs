use super::{PathData, PathSegment};
use glam::DVec2;
use std::ops::Range;

const DEFAULT_TOLERANCE: f64 = 0.01;
const MAX_SUBDIVISION_DEPTH: u32 = 16;
const LENGTH_EPSILON: f64 = 1e-12;

/// Renderer-independent arc-length approximation of a [`PathData`].
///
/// Each measured segment carries both the canonical drawable-segment index used
/// by text-path starts and the raw `PathData::segments` index used by path tools.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasuredPath {
    segments: Vec<MeasuredPathSegment>,
    contours: Vec<MeasuredPathContour>,
    total_length: f64,
}

/// Cumulative length metadata for one drawable source segment.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasuredPathSegment {
    drawable_segment_index: usize,
    path_segment_index: usize,
    contour_index: usize,
    start_distance: f64,
    length: f64,
    geometry: SegmentGeometry,
    samples: Vec<LengthSample>,
}

/// Cumulative length metadata for one source contour.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasuredPathContour {
    contour_index: usize,
    start_distance: f64,
    length: f64,
    measured_segments: Range<usize>,
    closed: bool,
}

/// A point and unit tangent resolved from an arc-length query.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeasuredPathPoint {
    pub point: [f64; 2],
    pub tangent: [f64; 2],
    pub distance: f64,
    pub contour_index: usize,
    pub drawable_segment_index: usize,
    pub path_segment_index: usize,
    pub segment_position: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SegmentGeometry {
    Line {
        start: DVec2,
        end: DVec2,
    },
    Quad {
        start: DVec2,
        control: DVec2,
        end: DVec2,
    },
    Cubic {
        start: DVec2,
        control1: DVec2,
        control2: DVec2,
        end: DVec2,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct LengthSample {
    position: f64,
    distance: f64,
    point: DVec2,
}

struct ContourBuilder {
    contour_index: usize,
    start_distance: f64,
    first_measured_segment: usize,
    closed: bool,
}

impl MeasuredPath {
    pub fn new(path: &PathData) -> Self {
        Self::with_tolerance(path, DEFAULT_TOLERANCE)
    }

    /// Measure with a caller-selected local-space flatness tolerance.
    ///
    /// A non-finite or non-positive tolerance falls back to the deterministic
    /// default instead of panicking or producing invalid measurements.
    pub fn with_tolerance(path: &PathData, tolerance: f64) -> Self {
        let tolerance = if tolerance.is_finite() && tolerance > 0.0 {
            tolerance
        } else {
            DEFAULT_TOLERANCE
        };
        let mut segments = Vec::new();
        let mut contours = Vec::new();
        let mut total_length = 0.0;
        let mut current = None;
        let mut contour_start = None;
        let mut active_contour: Option<ContourBuilder> = None;
        let mut next_contour_index = 0;
        let mut next_drawable_segment_index = 0;

        for (path_segment_index, segment) in path.segments.iter().copied().enumerate() {
            match segment {
                PathSegment::Move { to } => {
                    finish_contour(
                        &mut active_contour,
                        &mut contours,
                        segments.len(),
                        total_length,
                    );
                    let point = finite_point(to);
                    current = point;
                    contour_start = point;
                    active_contour = Some(ContourBuilder {
                        contour_index: next_contour_index,
                        start_distance: total_length,
                        first_measured_segment: segments.len(),
                        closed: false,
                    });
                    next_contour_index += 1;
                }
                PathSegment::Line { to } => {
                    let drawable_segment_index = next_drawable_segment_index;
                    next_drawable_segment_index += 1;
                    let end = finite_point(to);
                    if let (Some(start), Some(end), Some(contour)) =
                        (current, end, active_contour.as_ref())
                    {
                        append_segment(
                            &mut segments,
                            &mut total_length,
                            drawable_segment_index,
                            path_segment_index,
                            contour.contour_index,
                            SegmentGeometry::Line { start, end },
                            tolerance,
                        );
                    }
                    current = end;
                }
                PathSegment::Quad { ctrl, to } => {
                    let drawable_segment_index = next_drawable_segment_index;
                    next_drawable_segment_index += 1;
                    let control = finite_point(ctrl);
                    let end = finite_point(to);
                    if let (Some(start), Some(control), Some(end), Some(contour)) =
                        (current, control, end, active_contour.as_ref())
                    {
                        append_segment(
                            &mut segments,
                            &mut total_length,
                            drawable_segment_index,
                            path_segment_index,
                            contour.contour_index,
                            SegmentGeometry::Quad {
                                start,
                                control,
                                end,
                            },
                            tolerance,
                        );
                    }
                    current = end;
                }
                PathSegment::Cubic { ctrl1, ctrl2, to } => {
                    let drawable_segment_index = next_drawable_segment_index;
                    next_drawable_segment_index += 1;
                    let control1 = finite_point(ctrl1);
                    let control2 = finite_point(ctrl2);
                    let end = finite_point(to);
                    if let (Some(start), Some(control1), Some(control2), Some(end), Some(contour)) =
                        (current, control1, control2, end, active_contour.as_ref())
                    {
                        append_segment(
                            &mut segments,
                            &mut total_length,
                            drawable_segment_index,
                            path_segment_index,
                            contour.contour_index,
                            SegmentGeometry::Cubic {
                                start,
                                control1,
                                control2,
                                end,
                            },
                            tolerance,
                        );
                    }
                    current = end;
                }
                PathSegment::Close => {
                    let drawable_segment_index = next_drawable_segment_index;
                    next_drawable_segment_index += 1;
                    if let Some(contour) = active_contour.as_mut() {
                        contour.closed = true;
                        if let (Some(start), Some(end)) = (current, contour_start) {
                            append_segment(
                                &mut segments,
                                &mut total_length,
                                drawable_segment_index,
                                path_segment_index,
                                contour.contour_index,
                                SegmentGeometry::Line { start, end },
                                tolerance,
                            );
                            current = Some(end);
                        }
                    }
                }
            }
        }
        finish_contour(
            &mut active_contour,
            &mut contours,
            segments.len(),
            total_length,
        );

        Self {
            segments,
            contours,
            total_length,
        }
    }

    pub fn total_length(&self) -> f64 {
        self.total_length
    }

    pub fn segments(&self) -> &[MeasuredPathSegment] {
        &self.segments
    }

    pub fn contours(&self) -> &[MeasuredPathContour] {
        &self.contours
    }

    pub fn segment(&self, drawable_segment_index: usize) -> Option<&MeasuredPathSegment> {
        self.segments
            .iter()
            .find(|segment| segment.drawable_segment_index == drawable_segment_index)
    }

    /// Look up a measured segment by its raw index in [`PathData::segments`].
    pub fn segment_for_path_segment(
        &self,
        path_segment_index: usize,
    ) -> Option<&MeasuredPathSegment> {
        self.segments
            .iter()
            .find(|segment| segment.path_segment_index == path_segment_index)
    }

    /// Resolve a normalized drawable-segment position to cumulative path distance.
    pub fn distance_at_segment_position(
        &self,
        drawable_segment_index: usize,
        position: f64,
    ) -> Option<f64> {
        if !position.is_finite() || !(0.0..=1.0).contains(&position) {
            return None;
        }
        let segment = self.segment(drawable_segment_index)?;
        Some(segment.start_distance + segment.distance_at_position(position))
    }

    /// Point and forward tangent at cumulative distance over all contours.
    ///
    /// Finite distances are clamped to the path. At a contour boundary the
    /// preceding contour wins, making endpoint queries deterministic.
    pub fn point_tangent_at_distance(&self, distance: f64) -> Option<MeasuredPathPoint> {
        if !distance.is_finite() || self.total_length <= LENGTH_EPSILON {
            return None;
        }
        let distance = distance.clamp(0.0, self.total_length);
        let mut last_nonempty = None;
        for segment in &self.segments {
            if segment.length <= LENGTH_EPSILON {
                continue;
            }
            last_nonempty = Some(segment);
            if distance <= segment.end_distance() {
                return segment.point_tangent_at_distance(distance);
            }
        }
        last_nonempty?.point_tangent_at_distance(distance)
    }

    /// Point and forward tangent at distance local to one contour.
    pub fn point_tangent_on_contour(
        &self,
        contour_index: usize,
        distance: f64,
    ) -> Option<MeasuredPathPoint> {
        if !distance.is_finite() {
            return None;
        }
        let contour = self
            .contours
            .iter()
            .find(|contour| contour.contour_index == contour_index)?;
        if contour.length <= LENGTH_EPSILON {
            return None;
        }
        let distance = contour.start_distance + distance.clamp(0.0, contour.length);
        let mut last_nonempty = None;
        for segment in self.segments.get(contour.measured_segments.clone())? {
            if segment.length <= LENGTH_EPSILON {
                continue;
            }
            last_nonempty = Some(segment);
            if distance <= segment.end_distance() {
                return segment.point_tangent_at_distance(distance);
            }
        }
        last_nonempty?.point_tangent_at_distance(distance)
    }
}

impl MeasuredPathSegment {
    pub fn drawable_segment_index(&self) -> usize {
        self.drawable_segment_index
    }

    pub fn path_segment_index(&self) -> usize {
        self.path_segment_index
    }

    pub fn contour_index(&self) -> usize {
        self.contour_index
    }

    pub fn start_distance(&self) -> f64 {
        self.start_distance
    }

    pub fn length(&self) -> f64 {
        self.length
    }

    pub fn end_distance(&self) -> f64 {
        self.start_distance + self.length
    }

    fn distance_at_position(&self, position: f64) -> f64 {
        let position = position.clamp(0.0, 1.0);
        for pair in self.samples.windows(2) {
            let [left, right] = pair else {
                continue;
            };
            if position <= right.position {
                let span = right.position - left.position;
                if span <= f64::EPSILON {
                    return right.distance;
                }
                let factor = (position - left.position) / span;
                return left.distance + (right.distance - left.distance) * factor;
            }
        }
        self.length
    }

    fn position_at_distance(&self, distance: f64) -> f64 {
        let distance = distance.clamp(0.0, self.length);
        for pair in self.samples.windows(2) {
            let [left, right] = pair else {
                continue;
            };
            if distance <= right.distance {
                let span = right.distance - left.distance;
                if span <= LENGTH_EPSILON {
                    continue;
                }
                let factor = (distance - left.distance) / span;
                return left.position + (right.position - left.position) * factor;
            }
        }
        1.0
    }

    fn point_tangent_at_distance(&self, distance: f64) -> Option<MeasuredPathPoint> {
        let local_distance = (distance - self.start_distance).clamp(0.0, self.length);
        let position = self.position_at_distance(local_distance);
        let point = self.geometry.point(position);
        let tangent = normalized(self.geometry.tangent(position))
            .or_else(|| self.sample_tangent(position))?;
        Some(MeasuredPathPoint {
            point: point.to_array(),
            tangent: tangent.to_array(),
            distance: self.start_distance + local_distance,
            contour_index: self.contour_index,
            drawable_segment_index: self.drawable_segment_index,
            path_segment_index: self.path_segment_index,
            segment_position: position,
        })
    }

    fn sample_tangent(&self, position: f64) -> Option<DVec2> {
        let containing = self.samples.windows(2).find(|pair| {
            pair.first().is_some_and(|left| left.position <= position)
                && pair.last().is_some_and(|right| position <= right.position)
        });
        if let Some(pair) = containing {
            let direction = pair.get(1)?.point - pair.first()?.point;
            if let Some(direction) = normalized(direction) {
                return Some(direction);
            }
        }
        self.samples
            .windows(2)
            .find_map(|pair| normalized(pair.get(1)?.point - pair.first()?.point))
    }
}

impl MeasuredPathContour {
    pub fn contour_index(&self) -> usize {
        self.contour_index
    }

    pub fn start_distance(&self) -> f64 {
        self.start_distance
    }

    pub fn length(&self) -> f64 {
        self.length
    }

    pub fn end_distance(&self) -> f64 {
        self.start_distance + self.length
    }

    pub fn measured_segment_range(&self) -> Range<usize> {
        self.measured_segments.clone()
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

impl SegmentGeometry {
    fn point(self, position: f64) -> DVec2 {
        let position = position.clamp(0.0, 1.0);
        match self {
            Self::Line { start, end } => start.lerp(end, position),
            Self::Quad {
                start,
                control,
                end,
            } => {
                let one_minus = 1.0 - position;
                start * one_minus * one_minus
                    + control * 2.0 * one_minus * position
                    + end * position * position
            }
            Self::Cubic {
                start,
                control1,
                control2,
                end,
            } => {
                let one_minus = 1.0 - position;
                start * one_minus * one_minus * one_minus
                    + control1 * 3.0 * one_minus * one_minus * position
                    + control2 * 3.0 * one_minus * position * position
                    + end * position * position * position
            }
        }
    }

    fn tangent(self, position: f64) -> DVec2 {
        let position = position.clamp(0.0, 1.0);
        match self {
            Self::Line { start, end } => end - start,
            Self::Quad {
                start,
                control,
                end,
            } => (control - start) * (2.0 * (1.0 - position)) + (end - control) * (2.0 * position),
            Self::Cubic {
                start,
                control1,
                control2,
                end,
            } => {
                let one_minus = 1.0 - position;
                (control1 - start) * (3.0 * one_minus * one_minus)
                    + (control2 - control1) * (6.0 * one_minus * position)
                    + (end - control2) * (3.0 * position * position)
            }
        }
    }

    fn samples(self, tolerance: f64) -> Option<Vec<LengthSample>> {
        let mut flattened = vec![(0.0, self.point(0.0))];
        match self {
            Self::Line { end, .. } => flattened.push((1.0, end)),
            Self::Quad {
                start,
                control,
                end,
            } => flatten_quad(start, control, end, 0.0, 1.0, tolerance, 0, &mut flattened),
            Self::Cubic {
                start,
                control1,
                control2,
                end,
            } => flatten_cubic(
                start,
                control1,
                control2,
                end,
                0.0,
                1.0,
                tolerance,
                0,
                &mut flattened,
            ),
        }

        let mut samples = Vec::with_capacity(flattened.len());
        let mut distance = 0.0;
        let mut previous = None;
        for (position, point) in flattened {
            if let Some(previous) = previous {
                let increment = DVec2::distance(previous, point);
                if !increment.is_finite() {
                    return None;
                }
                distance += increment;
            }
            samples.push(LengthSample {
                position,
                distance,
                point,
            });
            previous = Some(point);
        }
        distance.is_finite().then_some(samples)
    }
}

fn append_segment(
    segments: &mut Vec<MeasuredPathSegment>,
    total_length: &mut f64,
    drawable_segment_index: usize,
    path_segment_index: usize,
    contour_index: usize,
    geometry: SegmentGeometry,
    tolerance: f64,
) {
    let Some(samples) = geometry.samples(tolerance) else {
        return;
    };
    let Some(length) = samples.last().map(|sample| sample.distance) else {
        return;
    };
    let end_distance = *total_length + length;
    if !end_distance.is_finite() {
        return;
    }
    segments.push(MeasuredPathSegment {
        drawable_segment_index,
        path_segment_index,
        contour_index,
        start_distance: *total_length,
        length,
        geometry,
        samples,
    });
    *total_length = end_distance;
}

fn finish_contour(
    active: &mut Option<ContourBuilder>,
    contours: &mut Vec<MeasuredPathContour>,
    measured_segment_end: usize,
    total_length: f64,
) {
    let Some(contour) = active.take() else {
        return;
    };
    contours.push(MeasuredPathContour {
        contour_index: contour.contour_index,
        start_distance: contour.start_distance,
        length: total_length - contour.start_distance,
        measured_segments: contour.first_measured_segment..measured_segment_end,
        closed: contour.closed,
    });
}

fn finite_point(point: [f64; 2]) -> Option<DVec2> {
    point
        .into_iter()
        .all(f64::is_finite)
        .then_some(DVec2::from(point))
}

fn normalized(vector: DVec2) -> Option<DVec2> {
    let length = vector.length();
    (length.is_finite() && length > LENGTH_EPSILON).then(|| vector / length)
}

fn midpoint(left: DVec2, right: DVec2) -> DVec2 {
    left * 0.5 + right * 0.5
}

#[allow(clippy::too_many_arguments)]
fn flatten_quad(
    start: DVec2,
    control: DVec2,
    end: DVec2,
    start_position: f64,
    end_position: f64,
    tolerance: f64,
    depth: u32,
    output: &mut Vec<(f64, DVec2)>,
) {
    let start_control = midpoint(start, control);
    let control_end = midpoint(control, end);
    let curve_midpoint = midpoint(start_control, control_end);
    let chord_midpoint = midpoint(start, end);
    let control_length = start.distance(control) + control.distance(end);
    let chord_length = start.distance(end);
    let flatness = control_length - chord_length;
    let parameter_error = curve_midpoint.distance(chord_midpoint);
    if depth >= MAX_SUBDIVISION_DEPTH
        || (flatness.is_finite()
            && parameter_error.is_finite()
            && flatness <= tolerance
            && parameter_error <= tolerance)
    {
        output.push((end_position, end));
        return;
    }

    let middle_position = (start_position + end_position) * 0.5;
    flatten_quad(
        start,
        start_control,
        curve_midpoint,
        start_position,
        middle_position,
        tolerance,
        depth + 1,
        output,
    );
    flatten_quad(
        curve_midpoint,
        control_end,
        end,
        middle_position,
        end_position,
        tolerance,
        depth + 1,
        output,
    );
}

#[allow(clippy::too_many_arguments)]
fn flatten_cubic(
    start: DVec2,
    control1: DVec2,
    control2: DVec2,
    end: DVec2,
    start_position: f64,
    end_position: f64,
    tolerance: f64,
    depth: u32,
    output: &mut Vec<(f64, DVec2)>,
) {
    let start_control = midpoint(start, control1);
    let controls_midpoint = midpoint(control1, control2);
    let control_end = midpoint(control2, end);
    let left_control = midpoint(start_control, controls_midpoint);
    let right_control = midpoint(controls_midpoint, control_end);
    let curve_midpoint = midpoint(left_control, right_control);
    let chord_midpoint = midpoint(start, end);
    let control_length =
        start.distance(control1) + control1.distance(control2) + control2.distance(end);
    let chord_length = start.distance(end);
    let flatness = control_length - chord_length;
    let parameter_error = curve_midpoint.distance(chord_midpoint);
    if depth >= MAX_SUBDIVISION_DEPTH
        || (flatness.is_finite()
            && parameter_error.is_finite()
            && flatness <= tolerance
            && parameter_error <= tolerance)
    {
        output.push((end_position, end));
        return;
    }

    let middle_position = (start_position + end_position) * 0.5;
    flatten_cubic(
        start,
        start_control,
        left_control,
        curve_midpoint,
        start_position,
        middle_position,
        tolerance,
        depth + 1,
        output,
    );
    flatten_cubic(
        curve_midpoint,
        right_control,
        control_end,
        end,
        middle_position,
        end_position,
        tolerance,
        depth + 1,
        output,
    );
}
