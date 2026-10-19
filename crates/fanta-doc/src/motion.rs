//! Persistent motion-authoring data and pure playhead evaluation.
//!
//! Motion lives beside the scene rather than inside nodes: one clip may animate
//! many nodes, and a node may participate in many clips. Evaluation returns
//! transient property overrides and never writes those sampled values back into
//! the [`crate::Doc`].

use crate::binding::BoundProp;
use crate::color::Color;
use crate::id::{AnimationClipId, AnimationTrackId, KeyframeId, NodeId};
use crate::node::{CanvasNode, Easing};
use crate::transform::Transform2D;
use crate::value::ResolvedVarValue;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// All authored animation clips in a document.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MotionLibrary {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub clips: BTreeMap<AnimationClipId, AnimationClip>,
}

impl MotionLibrary {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }

    pub fn clip(&self, id: AnimationClipId) -> Option<&AnimationClip> {
        self.clips.get(&id)
    }

    /// Sample `clip` at `playhead_ms` without mutating the library or scene.
    pub fn evaluate(&self, clip: AnimationClipId, playhead_ms: u32) -> Option<MotionEvaluation> {
        self.clips.get(&clip).map(|clip| clip.evaluate(playhead_ms))
    }

    pub fn compile(&self) -> CompiledMotionLibrary {
        CompiledMotionLibrary {
            clips: self
                .clips
                .iter()
                .map(|(identity, clip)| {
                    (
                        *identity,
                        CompiledAnimationClip {
                            identity: *identity,
                            duration_ms: clip.duration_ms,
                            settings: clip.settings.clone(),
                            tracks: clip
                                .tracks
                                .values()
                                .map(|track| CompiledAnimationTrack {
                                    target: track.target,
                                    keyframes: canonical_keyframes(track)
                                        .into_iter()
                                        .cloned()
                                        .collect(),
                                })
                                .collect(),
                        },
                    )
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CompiledMotionLibrary {
    clips: BTreeMap<AnimationClipId, CompiledAnimationClip>,
}

impl CompiledMotionLibrary {
    pub fn evaluate(&self, clip: AnimationClipId, playhead_ms: u32) -> Option<MotionEvaluation> {
        let clip = self.clips.get(&clip)?;
        let playhead_ms = playhead_ms.min(clip.duration_ms);
        let mut overrides = BTreeMap::new();
        for track in &clip.tracks {
            if let Some(value) = evaluate_keyframes(&track.keyframes, playhead_ms) {
                overrides.insert(track.target, value);
            }
        }
        Some(MotionEvaluation {
            clip: clip.identity,
            playhead_ms,
            overrides,
            transform_origins: clip.settings.transform_origins.clone(),
        })
    }

    pub fn settings(&self, clip: AnimationClipId) -> Option<&ClipSettings> {
        self.clips.get(&clip).map(|clip| &clip.settings)
    }
}

#[derive(Debug, Clone)]
struct CompiledAnimationClip {
    identity: AnimationClipId,
    duration_ms: u32,
    settings: ClipSettings,
    tracks: Vec<CompiledAnimationTrack>,
}

#[derive(Debug, Clone)]
struct CompiledAnimationTrack {
    target: MotionTarget,
    keyframes: Vec<Keyframe>,
}

/// How a clip plays when the host does not say otherwise: the authored
/// default the timeline, the runtime and exporters start from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopMode {
    #[default]
    Once,
    Repeat,
    PingPong,
}

impl LoopMode {
    pub fn is_once(&self) -> bool {
        *self == Self::Once
    }
}

/// Clip-level settings that are not the timeline itself. They flatten into
/// the clip's JSON and are all absent at their defaults, so a document that
/// never set them serializes exactly as before; one operation edits them as a
/// unit.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClipSettings {
    /// Frame rate for frame-based export and sampling. `None` leaves the
    /// exporter's default (30).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<u16>,
    #[serde(default, skip_serializing_if = "LoopMode::is_once")]
    pub loop_mode: LoopMode,
    /// The top-level frame this clip animates, the stage an export renders.
    /// Figma keeps one timeline per top-level frame; Fanta records the frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<NodeId>,
    /// Backdrop behind the stage in exports with no frame background.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<Color>,
    /// Per node, the pivot that rotation and scale tracks turn about, as a
    /// fraction of the node's local bounds (`[0.5, 0.5]` is the centre).
    /// Absent, channels keep turning about the local origin.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub transform_origins: BTreeMap<NodeId, [f64; 2]>,
}

/// A named, finite timeline containing independently-addressable tracks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationClip {
    pub id: AnimationClipId,
    pub name: String,
    /// Timeline length. Evaluation clamps a later playhead to this boundary.
    #[serde(default)]
    pub duration_ms: u32,
    #[serde(flatten)]
    pub settings: ClipSettings,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tracks: BTreeMap<AnimationTrackId, AnimationTrack>,
}

impl AnimationClip {
    pub fn new(id: AnimationClipId, name: impl Into<String>, duration_ms: u32) -> Self {
        Self {
            id,
            name: name.into(),
            duration_ms,
            settings: ClipSettings::default(),
            tracks: BTreeMap::new(),
        }
    }

    /// Sample every non-empty track into a deterministic target-to-value map.
    ///
    /// A malformed file can contain two tracks for the same target. Tracks are
    /// visited by stable id order, so the later id wins deterministically rather
    /// than making playback depend on hash-map or import order.
    pub fn evaluate(&self, playhead_ms: u32) -> MotionEvaluation {
        let playhead_ms = playhead_ms.min(self.duration_ms);
        let mut overrides = BTreeMap::new();
        for track in self.tracks.values() {
            if let Some(value) = track.evaluate(playhead_ms) {
                overrides.insert(track.target, value);
            }
        }
        MotionEvaluation {
            clip: self.id,
            playhead_ms,
            overrides,
            transform_origins: self.settings.transform_origins.clone(),
        }
    }

    pub fn track_for_target(&self, target: MotionTarget) -> Option<&AnimationTrack> {
        self.tracks.values().find(|track| track.target == target)
    }

    #[cfg(test)]
    fn compile_settings_for_test(&self) -> ClipSettings {
        let mut library = MotionLibrary::new();
        library.clips.insert(self.id, self.clone());
        library
            .compile()
            .settings(self.id)
            .cloned()
            .unwrap_or_default()
    }
}

/// One timeline lane targeting exactly one property of one node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationTrack {
    pub id: AnimationTrackId,
    pub target: MotionTarget,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub keyframes: BTreeMap<KeyframeId, Keyframe>,
}

impl AnimationTrack {
    pub fn new(id: AnimationTrackId, target: MotionTarget) -> Self {
        Self {
            id,
            target,
            keyframes: BTreeMap::new(),
        }
    }

    /// Evaluate the track at `playhead_ms`, clamping outside its keyed range.
    ///
    /// Keyframes are stored by stable identity so timeline edits do not depend
    /// on vector positions. Sampling sorts by `(time, id)`; for duplicate times,
    /// the greatest id at that time wins. Values that do not match the target's
    /// static type are ignored so hand-authored or forward-version data cannot
    /// poison every otherwise valid keyframe in the track.
    pub fn evaluate(&self, playhead_ms: u32) -> Option<ResolvedVarValue> {
        evaluate_keyframes(&canonical_keyframes(self), playhead_ms)
    }
}

fn canonical_keyframes(track: &AnimationTrack) -> Vec<&Keyframe> {
    let mut keyframes: Vec<&Keyframe> = track
        .keyframes
        .values()
        .filter(|keyframe| track.target.property.accepts_value(&keyframe.value))
        .collect();
    keyframes.sort_by_key(|keyframe| (keyframe.time_ms, keyframe.id));
    let mut canonical: Vec<&Keyframe> = Vec::with_capacity(keyframes.len());
    for keyframe in keyframes {
        if let Some(previous) = canonical.last_mut()
            && previous.time_ms == keyframe.time_ms
        {
            *previous = keyframe;
        } else {
            canonical.push(keyframe);
        }
    }
    canonical
}

fn evaluate_keyframes<T: std::borrow::Borrow<Keyframe>>(
    canonical_keyframes: &[T],
    playhead_ms: u32,
) -> Option<ResolvedVarValue> {
    let first = canonical_keyframes.first()?.borrow();
    if playhead_ms < first.time_ms {
        return Some(first.value.clone());
    }

    let upper_index =
        canonical_keyframes.partition_point(|keyframe| keyframe.borrow().time_ms <= playhead_ms);
    if upper_index == canonical_keyframes.len() {
        return canonical_keyframes
            .last()
            .map(|keyframe| keyframe.borrow().value.clone());
    }

    let lower = canonical_keyframes
        .get(upper_index.checked_sub(1)?)?
        .borrow();
    let upper = canonical_keyframes.get(upper_index)?.borrow();
    if lower.time_ms == playhead_ms || lower.interpolation == Interpolation::Hold {
        return Some(lower.value.clone());
    }

    let segment_ms = upper.time_ms.checked_sub(lower.time_ms)?;
    if segment_ms == 0 {
        return Some(upper.value.clone());
    }
    let elapsed_ms = playhead_ms.saturating_sub(lower.time_ms);
    let progress = elapsed_ms as f64 / segment_ms as f64;
    let progress = eased_progress(lower.easing, progress);
    Some(interpolate_value(&lower.value, &upper.value, progress))
}

/// Stable address of one animatable property on one scene node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MotionTarget {
    pub node: NodeId,
    pub property: MotionProperty,
}

impl MotionTarget {
    pub const fn new(node: NodeId, property: MotionProperty) -> Self {
        Self { node, property }
    }
}

/// A property address suitable for animation tracks.
///
/// [`BoundProp`] remains the shared vocabulary for variable-bindable and
/// component-overridable fields. Transform channels deliberately live only
/// here: Figma-style variables do not bind to rotation or scale, while a useful
/// motion system must animate them independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MotionProperty {
    Bound {
        prop: BoundProp,
    },
    /// Parent-space translation in document logical units.
    PositionX,
    /// Parent-space translation in document logical units.
    PositionY,
    /// Rotation in radians, using [`Transform2D::rotation`]'s sign convention.
    Rotation,
    /// Horizontal scale multiplier (`1.0` is identity; negative reflects).
    ScaleX,
    /// Vertical scale multiplier (`1.0` is identity; negative reflects).
    ScaleY,
}

impl MotionProperty {
    pub const fn bound(prop: BoundProp) -> Self {
        Self::Bound { prop }
    }

    /// Whether a keyframe value has the static type required by this property.
    pub fn accepts_value(&self, value: &ResolvedVarValue) -> bool {
        match self {
            Self::PositionX | Self::PositionY | Self::Rotation | Self::ScaleX | Self::ScaleY => {
                matches!(value, ResolvedVarValue::Float { .. })
            }
            Self::Bound { prop } => prop.variable_type() == value.variable_type(),
        }
    }
}

/// Canonical scale/rotation/translation channels used by motion overrides.
///
/// Position is the affine transform's parent-space translation in document
/// logical units, rotation is radians, and scale is a signed multiplier. A
/// reflection is canonicalized onto `scale[0]`, leaving `scale[1]` non-negative.
/// Sheared or degenerate source transforms cannot be represented losslessly and
/// are rejected by [`Self::decompose`] rather than silently losing information.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionTransform {
    pub position: [f64; 2],
    pub rotation_radians: f64,
    pub scale: [f64; 2],
}

impl MotionTransform {
    pub fn decompose(transform: Transform2D) -> Option<Self> {
        let [a, b, c, d, position_x, position_y] = transform.to_components();
        if ![a, b, c, d, position_x, position_y]
            .into_iter()
            .all(f64::is_finite)
        {
            return None;
        }

        let scale_x_magnitude = a.hypot(b);
        let scale_y = c.hypot(d);
        if scale_x_magnitude <= f64::EPSILON || scale_y <= f64::EPSILON {
            return None;
        }

        let normalized_axis_dot = (a * c + b * d) / (scale_x_magnitude * scale_y);
        if normalized_axis_dot.abs() > 1e-9 {
            return None;
        }

        let determinant = a * d - b * c;
        if determinant == 0.0 {
            return None;
        }
        let scale_x = if determinant.is_sign_negative() {
            -scale_x_magnitude
        } else {
            scale_x_magnitude
        };

        Some(Self {
            position: [position_x, position_y],
            rotation_radians: (-c).atan2(d),
            scale: [scale_x, scale_y],
        })
    }

    pub fn recompose(self) -> Option<Transform2D> {
        if ![
            self.position[0],
            self.position[1],
            self.rotation_radians,
            self.scale[0],
            self.scale[1],
        ]
        .into_iter()
        .all(f64::is_finite)
        {
            return None;
        }

        let (sin, cos) = self.rotation_radians.sin_cos();
        Some(Transform2D::from_components([
            cos * self.scale[0],
            sin * self.scale[0],
            -sin * self.scale[1],
            cos * self.scale[1],
            self.position[0],
            self.position[1],
        ]))
    }

    /// Apply one transform-channel value. Returns `false` for bound properties,
    /// non-float values, or non-finite values.
    pub fn set(&mut self, property: MotionProperty, value: &ResolvedVarValue) -> bool {
        let ResolvedVarValue::Float { value } = value else {
            return false;
        };
        if !value.is_finite() {
            return false;
        }
        match property {
            MotionProperty::PositionX => self.position[0] = *value,
            MotionProperty::PositionY => self.position[1] = *value,
            MotionProperty::Rotation => self.rotation_radians = *value,
            MotionProperty::ScaleX => self.scale[0] = *value,
            MotionProperty::ScaleY => self.scale[1] = *value,
            MotionProperty::Bound { .. } => return false,
        }
        true
    }
}

/// One independently editable point on an [`AnimationTrack`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    pub id: KeyframeId,
    pub time_ms: u32,
    pub value: ResolvedVarValue,
    /// Interpolation used from this keyframe to the next one.
    #[serde(default)]
    pub interpolation: Interpolation,
    /// Timing curve used from this keyframe to the next one.
    #[serde(default)]
    pub easing: Easing,
}

impl Keyframe {
    pub fn new(id: KeyframeId, time_ms: u32, value: ResolvedVarValue) -> Self {
        Self {
            id,
            time_ms,
            value,
            interpolation: Interpolation::default(),
            easing: Easing::default(),
        }
    }
}

/// How a keyframe's outgoing segment changes value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interpolation {
    #[default]
    Linear,
    Hold,
}

/// Read-only result of sampling one clip.
#[derive(Debug, Clone, PartialEq)]
pub struct MotionEvaluation {
    pub clip: AnimationClipId,
    /// The duration-clamped playhead that was actually sampled.
    pub playhead_ms: u32,
    pub overrides: BTreeMap<MotionTarget, ResolvedVarValue>,
    /// The clip's declared pivots (see [`ClipSettings::transform_origins`]).
    pub transform_origins: BTreeMap<NodeId, [f64; 2]>,
}

impl MotionEvaluation {
    pub fn get(&self, target: MotionTarget) -> Option<&ResolvedVarValue> {
        self.overrides.get(&target)
    }

    pub fn is_empty(&self) -> bool {
        self.overrides.is_empty()
    }

    /// Apply this evaluation to a clone of `node` and return the transient node.
    /// The passed node—and therefore the committed [`crate::Doc`]—is untouched.
    ///
    /// Bound properties reuse [`BoundProp::apply_resolved`]. Transform channels
    /// are decomposed and recomposed once as a unit; if the committed transform
    /// contains shear or is degenerate, those channel overrides are ignored so
    /// evaluation never destroys unrepresentable affine data.
    pub fn apply_to_node(&self, node: &CanvasNode) -> CanvasNode {
        let mut evaluated = node.clone();
        let mut transform = MotionTransform::decompose(node.transform);
        let mut transform_changed = false;

        let first = MotionTarget::new(
            node.id,
            MotionProperty::Bound {
                prop: BoundProp::FillColor { index: 0 },
            },
        );
        let last = MotionTarget::new(node.id, MotionProperty::ScaleY);
        let typography = MotionTarget::new(
            node.id,
            MotionProperty::Bound {
                prop: BoundProp::TextStyle,
            },
        );
        if let Some(value) = self.overrides.get(&typography) {
            BoundProp::TextStyle.apply_resolved(&mut evaluated, value.clone());
        }
        // MotionTarget orders the node before its property, so a range avoids scanning other nodes' tracks.
        for (target, value) in self.overrides.range(first..=last) {
            if *target == typography {
                continue;
            }
            match target.property {
                MotionProperty::Bound { prop } => {
                    prop.apply_resolved(&mut evaluated, value.clone());
                }
                property => {
                    if let Some(transform) = &mut transform {
                        transform_changed |= transform.set(property, value);
                    }
                }
            }
        }

        if transform_changed && let Some(animated) = transform {
            let recomposed = match self.transform_origins.get(&node.id) {
                Some(origin) => pivoted_transform(node, animated, *origin),
                None => animated.recompose(),
            };
            if let Some(transform) = recomposed {
                evaluated.transform = transform;
            }
        }
        evaluated
    }
}

/// Recompose animated channels so rotation and scale turn about `origin`, a
/// fraction of the node's local bounds.
///
/// Keyframes stay absolute, so the animated rotation and scale are applied as
/// the change from the authored channels, about the pivot, on top of the
/// authored matrix (with any animated position): `T(pos) · R0 · S0 · T(p) ·
/// R(Δθ) · S(ΔS) · T(-p)`. At the authored values this is exactly the
/// committed transform, so a clip never jumps where it starts.
fn pivoted_transform(
    node: &CanvasNode,
    animated: MotionTransform,
    origin: [f64; 2],
) -> Option<Transform2D> {
    let authored = MotionTransform::decompose(node.transform)?;
    let bounds = node.data.local_bounds();
    let pivot = bounds.map_or([0.0, 0.0], |bounds| {
        [
            bounds.min_x + origin[0] * bounds.width(),
            bounds.min_y + origin[1] * bounds.height(),
        ]
    });
    let delta_rotation = animated.rotation_radians - authored.rotation_radians;
    let delta_scale = [
        animated.scale[0] / authored.scale[0],
        animated.scale[1] / authored.scale[1],
    ];
    if !pivot
        .iter()
        .chain(&delta_scale)
        .all(|value| value.is_finite())
        || !delta_rotation.is_finite()
    {
        return None;
    }
    let base = MotionTransform {
        position: animated.position,
        rotation_radians: authored.rotation_radians,
        scale: authored.scale,
    }
    .recompose()?;
    let local = Transform2D::translation(pivot[0], pivot[1]).0
        * Transform2D::rotation(delta_rotation).0
        * Transform2D::scale_xy(delta_scale[0], delta_scale[1]).0
        * Transform2D::translation(-pivot[0], -pivot[1]).0;
    let transform = Transform2D(base.0 * local);
    transform.is_finite().then_some(transform)
}

fn interpolate_value(
    from: &ResolvedVarValue,
    to: &ResolvedVarValue,
    progress: f64,
) -> ResolvedVarValue {
    match (from, to) {
        (ResolvedVarValue::Float { value: from }, ResolvedVarValue::Float { value: to }) => {
            ResolvedVarValue::Float {
                value: from + (to - from) * progress,
            }
        }
        (ResolvedVarValue::Color { value: from }, ResolvedVarValue::Color { value: to }) => {
            ResolvedVarValue::Color {
                value: interpolate_color(*from, *to, progress),
            }
        }
        // Boolean, string, and composite typography tracks are discrete. A
        // mismatched pair is also held rather than manufacturing a conversion.
        _ => from.clone(),
    }
}

fn interpolate_color(from: Color, to: Color, progress: f64) -> Color {
    let channel = |from: u8, to: u8| -> u8 {
        (from as f64 + (to as f64 - from as f64) * progress)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color::rgba(
        channel(from.r, to.r),
        channel(from.g, to.g),
        channel(from.b, to.b),
        channel(from.a, to.a),
    )
}

fn eased_progress(easing: Easing, progress: f64) -> f64 {
    let progress = progress.clamp(0.0, 1.0);
    match easing {
        Easing::Linear => progress,
        Easing::EaseIn => cubic_bezier(progress, 0.42, 0.0, 1.0, 1.0),
        Easing::EaseOut => cubic_bezier(progress, 0.0, 0.0, 0.58, 1.0),
        Easing::EaseInOut => cubic_bezier(progress, 0.42, 0.0, 0.58, 1.0),
        Easing::CubicBezier { x1, y1, x2, y2 } => cubic_bezier(
            progress,
            f64::from(x1),
            f64::from(y1),
            f64::from(x2),
            f64::from(y2),
        ),
        // Springs share the one closed-form sampler with present-mode
        // transitions (see `node::prototype::spring_progress`). The overshoot
        // an under-damped spring produces intentionally passes through — the
        // interpolators extrapolate past their keyframe values, which is what
        // makes a bouncy position track actually bounce.
        Easing::Spring {
            mass,
            stiffness,
            damping,
        } => crate::node::spring_progress(mass, stiffness, damping, progress),
    }
}

fn cubic_bezier(progress: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    if progress <= 0.0 || progress >= 1.0 {
        return progress;
    }

    let mut parameter = progress;
    for _ in 0..8 {
        let error = bezier_coordinate(parameter, x1, x2) - progress;
        if error.abs() <= 1e-7 {
            return bezier_coordinate(parameter, y1, y2);
        }
        let derivative = bezier_derivative(parameter, x1, x2);
        if derivative.abs() <= 1e-7 {
            break;
        }
        let next = parameter - error / derivative;
        if !(0.0..=1.0).contains(&next) {
            break;
        }
        parameter = next;
    }

    // CSS timing curves require monotonic x handles. Bisection is the stable
    // fallback when Newton's derivative is too small near either endpoint.
    let (mut low, mut high) = (0.0, 1.0);
    for _ in 0..24 {
        parameter = (low + high) * 0.5;
        if bezier_coordinate(parameter, x1, x2) < progress {
            low = parameter;
        } else {
            high = parameter;
        }
    }
    bezier_coordinate(parameter, y1, y2)
}

fn bezier_coordinate(parameter: f64, first: f64, second: f64) -> f64 {
    let inverse = 1.0 - parameter;
    3.0 * inverse * inverse * parameter * first
        + 3.0 * inverse * parameter * parameter * second
        + parameter * parameter * parameter
}

fn bezier_derivative(parameter: f64, first: f64, second: f64) -> f64 {
    let inverse = 1.0 - parameter;
    3.0 * inverse * inverse * first
        + 6.0 * inverse * parameter * (second - first)
        + 3.0 * parameter * parameter * (1.0 - second)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::UnitInterval;
    use crate::{CanvasNode, Doc, NodeData, VectorNode};

    fn float_keyframe(id: u128, time_ms: u32, value: f64) -> Keyframe {
        Keyframe {
            id: KeyframeId::from_u128(id),
            time_ms,
            value: ResolvedVarValue::Float { value },
            interpolation: Interpolation::Linear,
            easing: Easing::Linear,
        }
    }

    fn position_clip(node: NodeId) -> (AnimationClip, MotionTarget) {
        let clip_id = AnimationClipId::from_u128(1);
        let track_id = AnimationTrackId::from_u128(2);
        let target = MotionTarget::new(node, MotionProperty::PositionX);
        let mut track = AnimationTrack::new(track_id, target);
        let first = float_keyframe(3, 0, 0.0);
        let second = float_keyframe(4, 1_000, 100.0);
        track.keyframes.insert(first.id, first);
        track.keyframes.insert(second.id, second);
        let mut clip = AnimationClip::new(clip_id, "Entrance", 1_000);
        clip.tracks.insert(track.id, track);
        (clip, target)
    }

    #[test]
    fn clip_settings_round_trip_and_stay_off_the_wire_when_default() -> serde_json::Result<()> {
        let node = NodeId::from_u128(20);
        let (clip, _) = position_clip(node);
        let plain = serde_json::to_value(&clip)?;
        for key in [
            "fps",
            "loop_mode",
            "scope",
            "background",
            "transform_origins",
        ] {
            assert!(
                plain.get(key).is_none(),
                "{key} must be absent at its default"
            );
        }

        let mut clip = clip;
        clip.settings = ClipSettings {
            fps: Some(24),
            loop_mode: LoopMode::PingPong,
            scope: Some(NodeId::from_u128(9)),
            background: Some(Color::BLACK),
            transform_origins: BTreeMap::from([(node, [0.5, 0.5])]),
        };
        let json = serde_json::to_value(&clip)?;
        assert_eq!(json["fps"], 24);
        assert_eq!(json["loop_mode"], "ping_pong");
        let key = serde_json::to_value(node)?;
        let key = key.as_str().expect("ids serialize as strings");
        assert_eq!(
            json["transform_origins"][key],
            serde_json::json!([0.5, 0.5])
        );
        let restored: AnimationClip = serde_json::from_value(json)?;
        assert_eq!(restored, clip);
        assert_eq!(clip.compile_settings_for_test(), clip.settings);
        Ok(())
    }

    #[test]
    fn rotation_about_a_declared_origin_keeps_the_pivot_in_place() {
        use crate::node::GroupNode;
        use glam::DVec2;

        let mut node = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([100.0, 50.0]),
            ..GroupNode::default()
        }));
        node.transform = Transform2D::translation(10.0, 20.0);
        let clip_id = AnimationClipId::from_u128(1);
        let target = MotionTarget::new(node.id, MotionProperty::Rotation);
        let mut track = AnimationTrack::new(AnimationTrackId::from_u128(2), target);
        let turned = float_keyframe(3, 0, std::f64::consts::FRAC_PI_2);
        track.keyframes.insert(turned.id, turned);
        let mut clip = AnimationClip::new(clip_id, "Spin", 1_000);
        clip.tracks.insert(track.id, track);

        let centre = DVec2::new(50.0, 25.0);
        let before = node.transform.transform_point(centre);
        let about_origin = clip.evaluate(0).apply_to_node(&node);
        let moved = about_origin.transform.transform_point(centre);
        assert!(
            (moved - before).length() > 1.0,
            "turning about the local origin moves the centre: {moved:?}"
        );

        clip.settings.transform_origins.insert(node.id, [0.5, 0.5]);
        let about_centre = clip.evaluate(0).apply_to_node(&node);
        let kept = about_centre.transform.transform_point(centre);
        assert!(
            (kept - before).length() < 1e-9,
            "turning about the centre keeps it at {before:?}, got {kept:?}"
        );
        let corner = about_centre.transform.transform_point(DVec2::ZERO);
        assert!(
            (corner - DVec2::new(85.0, -5.0)).length() < 1e-9,
            "a quarter turn about the centre puts the corner at (85, -5), got {corner:?}"
        );
        let mut library = MotionLibrary::new();
        library.clips.insert(clip_id, clip);
        assert_eq!(
            library
                .compile()
                .evaluate(clip_id, 0)
                .map(|evaluation| evaluation.apply_to_node(&node).transform),
            Some(about_centre.transform),
            "the compiled library applies the same pivot"
        );
    }

    #[test]
    fn motion_library_round_trips_with_stable_ids() -> serde_json::Result<()> {
        let node = NodeId::from_u128(20);
        let (clip, _) = position_clip(node);
        let mut library = MotionLibrary::new();
        library.clips.insert(clip.id, clip);

        let json = serde_json::to_string(&library)?;
        let restored: MotionLibrary = serde_json::from_str(&json)?;
        assert_eq!(restored, library);
        Ok(())
    }

    #[test]
    fn keyframe_defaults_preserve_additive_serde_compatibility() -> serde_json::Result<()> {
        let id = KeyframeId::from_u128(7);
        let json = serde_json::json!({
            "id": serde_json::to_value(id)?,
            "time_ms": 125,
            "value": { "kind": "float", "value": 4.0 }
        });
        let keyframe: Keyframe = serde_json::from_value(json)?;
        assert_eq!(keyframe.interpolation, Interpolation::Linear);
        assert_eq!(keyframe.easing, Easing::EaseInOut);
        Ok(())
    }

    #[test]
    fn empty_library_is_compact_and_old_docs_default_to_empty() -> serde_json::Result<()> {
        let library_json = serde_json::to_string(&MotionLibrary::new())?;
        assert_eq!(library_json, "{}");

        let doc = Doc::new();
        let json = doc.to_json_string()?;
        assert!(!json.contains("\"motion\""));
        let restored = Doc::from_json_str(&json).map_err(|error| {
            serde_json::Error::io(std::io::Error::new(std::io::ErrorKind::InvalidData, error))
        })?;
        assert!(restored.motion.is_empty());
        Ok(())
    }

    #[test]
    fn linear_tracks_interpolate_and_clip_duration_clamps() {
        let node = NodeId::from_u128(20);
        let (clip, target) = position_clip(node);

        let halfway = clip.evaluate(500);
        assert_eq!(
            halfway.get(target),
            Some(&ResolvedVarValue::Float { value: 50.0 })
        );

        let after_end = clip.evaluate(5_000);
        assert_eq!(after_end.playhead_ms, 1_000);
        assert_eq!(
            after_end.get(target),
            Some(&ResolvedVarValue::Float { value: 100.0 })
        );
    }

    #[test]
    fn easing_curves_remap_segment_progress() {
        let node = NodeId::from_u128(20);
        let (mut clip, target) = position_clip(node);
        let Some(track) = clip.tracks.values_mut().next() else {
            panic!("position fixture must include one track");
        };
        let Some(first) = track.keyframes.values_mut().next() else {
            panic!("position fixture must include a starting keyframe");
        };
        first.easing = Easing::EaseIn;

        let halfway = clip.evaluate(500);
        let Some(ResolvedVarValue::Float { value }) = halfway.get(target) else {
            panic!("position track must evaluate to a float override");
        };
        assert!(*value > 0.0 && *value < 50.0);
    }

    #[test]
    fn duplicate_times_resolve_by_stable_keyframe_id_before_interpolation() {
        let target = MotionTarget::new(NodeId::from_u128(20), MotionProperty::PositionX);
        let mut track = AnimationTrack::new(AnimationTrackId::from_u128(1), target);
        let earlier_id = float_keyframe(2, 100, 1.0);
        let winning_id = float_keyframe(3, 100, 2.0);
        let end = float_keyframe(4, 200, 4.0);
        track.keyframes.insert(earlier_id.id, earlier_id);
        track.keyframes.insert(winning_id.id, winning_id);
        track.keyframes.insert(end.id, end);

        assert_eq!(
            track.evaluate(0),
            Some(ResolvedVarValue::Float { value: 2.0 })
        );
        assert_eq!(
            track.evaluate(150),
            Some(ResolvedVarValue::Float { value: 3.0 })
        );
    }

    #[test]
    fn hold_and_color_interpolation_have_predictable_segment_semantics() {
        let node = NodeId::from_u128(20);
        let target = MotionTarget::new(
            node,
            MotionProperty::bound(BoundProp::FillColor { index: 0 }),
        );
        let mut track = AnimationTrack::new(AnimationTrackId::from_u128(2), target);
        let first = Keyframe {
            id: KeyframeId::from_u128(3),
            time_ms: 0,
            value: ResolvedVarValue::Color {
                value: Color::BLACK,
            },
            interpolation: Interpolation::Linear,
            easing: Easing::Linear,
        };
        let second = Keyframe {
            id: KeyframeId::from_u128(4),
            time_ms: 1_000,
            value: ResolvedVarValue::Color {
                value: Color::WHITE,
            },
            interpolation: Interpolation::Hold,
            easing: Easing::Linear,
        };
        track.keyframes.insert(first.id, first);
        track.keyframes.insert(second.id, second);

        assert_eq!(
            track.evaluate(500),
            Some(ResolvedVarValue::Color {
                value: Color::rgb(128, 128, 128)
            })
        );
        assert_eq!(
            track.evaluate(1_000),
            Some(ResolvedVarValue::Color {
                value: Color::WHITE
            })
        );
    }

    #[test]
    fn transform_channels_use_document_units_radians_and_scale_multipliers() {
        let node = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            10.0,
            10.0,
            Color::WHITE,
        )));
        let float = |value| ResolvedVarValue::Float { value };
        let overrides = BTreeMap::from([
            (
                MotionTarget::new(node.id, MotionProperty::PositionX),
                float(12.0),
            ),
            (
                MotionTarget::new(node.id, MotionProperty::PositionY),
                float(34.0),
            ),
            (
                MotionTarget::new(node.id, MotionProperty::Rotation),
                float(std::f64::consts::FRAC_PI_2),
            ),
            (
                MotionTarget::new(node.id, MotionProperty::ScaleX),
                float(2.0),
            ),
            (
                MotionTarget::new(node.id, MotionProperty::ScaleY),
                float(3.0),
            ),
        ]);
        let evaluation = MotionEvaluation {
            clip: AnimationClipId::from_u128(1),
            playhead_ms: 0,
            overrides,
            transform_origins: BTreeMap::new(),
        };

        let evaluated = evaluation.apply_to_node(&node);
        let [a, b, c, d, x, y] = evaluated.transform.to_components();
        assert!(a.abs() < 1e-12);
        assert!((b - 2.0).abs() < 1e-12);
        assert!((c + 3.0).abs() < 1e-12);
        assert!(d.abs() < 1e-12);
        assert_eq!([x, y], [12.0, 34.0]);
        assert_eq!(node.transform, Transform2D::IDENTITY);

        let channels = MotionTransform::decompose(evaluated.transform);
        assert!(channels.is_some_and(|channels| {
            channels.position == [12.0, 34.0]
                && (channels.rotation_radians - std::f64::consts::FRAC_PI_2).abs() < 1e-12
                && (channels.scale[0] - 2.0).abs() < 1e-12
                && (channels.scale[1] - 3.0).abs() < 1e-12
        }));

        let shear = Transform2D::from_components([1.0, 0.0, 0.5, 1.0, 0.0, 0.0]);
        assert!(MotionTransform::decompose(shear).is_none());
    }

    #[test]
    fn evaluation_never_mutates_committed_node_state() -> Result<(), crate::scene::SceneError> {
        let mut doc = Doc::new();
        let node = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            10.0,
            10.0,
            Color::WHITE,
        )));
        let node_id = node.id;
        doc.scene.insert(node)?;

        let target = MotionTarget::new(node_id, MotionProperty::bound(BoundProp::Opacity));
        let mut track = AnimationTrack::new(AnimationTrackId::from_u128(2), target);
        let first = float_keyframe(3, 0, 0.0);
        let second = float_keyframe(4, 1_000, 1.0);
        track.keyframes.insert(first.id, first);
        track.keyframes.insert(second.id, second);
        let mut clip = AnimationClip::new(AnimationClipId::from_u128(1), "Fade", 1_000);
        clip.tracks.insert(track.id, track);
        doc.motion.clips.insert(clip.id, clip);

        let before = doc
            .scene
            .get(node_id)
            .map(|node| node.opacity)
            .unwrap_or(UnitInterval::ZERO);
        let Some(evaluation) = doc.motion.evaluate(AnimationClipId::from_u128(1), 500) else {
            return Err(crate::scene::SceneError::InvariantViolated(
                "motion fixture clip disappeared".into(),
            ));
        };
        let Some(committed) = doc.scene.get(node_id) else {
            return Err(crate::scene::SceneError::NotFound(node_id));
        };
        let evaluated = evaluation.apply_to_node(committed);
        assert_eq!(evaluated.opacity, UnitInterval::new(0.5));
        assert_eq!(
            doc.scene.get(node_id).map(|node| node.opacity),
            Some(before)
        );
        assert_eq!(before, UnitInterval::ONE);
        Ok(())
    }

    #[test]
    fn clip_operations_participate_in_doc_undo_and_redo() -> Result<(), crate::scene::SceneError> {
        let mut doc = Doc::new();
        let clip_id = AnimationClipId::from_u128(50);
        doc.apply(crate::Operation::CreateAnimationClip {
            clip: Box::new(AnimationClip::new(clip_id, "Loop", 500)),
        })?;
        assert!(doc.motion.clip(clip_id).is_some());

        assert!(doc.undo()?);
        assert!(doc.motion.clip(clip_id).is_none());
        assert!(doc.redo()?);
        assert!(doc.motion.clip(clip_id).is_some());
        Ok(())
    }
    #[test]
    fn compiled_motion_matches_authoring_evaluation_with_duplicate_times() {
        let mut library = MotionLibrary::new();
        let target = MotionTarget::new(NodeId::from_u128(7), MotionProperty::PositionX);
        let mut track = AnimationTrack::new(AnimationTrackId::from_u128(8), target);
        for keyframe in [
            float_keyframe(10, 100, 30.0),
            float_keyframe(11, 0, 0.0),
            float_keyframe(12, 100, 100.0),
        ] {
            track.keyframes.insert(keyframe.id, keyframe);
        }
        let mut clip = AnimationClip::new(AnimationClipId::from_u128(9), "Move", 100);
        clip.tracks.insert(track.id, track);
        library.clips.insert(clip.id, clip);
        let compiled = library.compile();
        for playhead in [0, 1, 20, 50, 99, 100, 101, u32::MAX] {
            assert_eq!(
                compiled.evaluate(AnimationClipId::from_u128(9), playhead),
                library.evaluate(AnimationClipId::from_u128(9), playhead)
            );
        }
        library.clips.clear();
        assert!(
            compiled
                .evaluate(AnimationClipId::from_u128(9), 50)
                .is_some()
        );
    }
}
