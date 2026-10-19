//! The portable form of one animation clip: a seconds-based, name-addressed
//! document that exporters read, agents write, and other motion tools can map
//! onto their own models.
//!
//! The authored clip ([`AnimationClip`]) stays the editing truth: it keys
//! tracks and keyframes by stable ids and counts time in milliseconds. A
//! [`MotionSpec`] groups the same data by layer (one entry per animated node)
//! and names each track by its property (`translation_x`, `opacity`,
//! `fill_color:1`), so the same document reads naturally as a Figma keyframe
//! track set or as a time-driven Stage. Rotation is in degrees here and in
//! radians in the clip; time is in seconds here and milliseconds there.
//! Importing a spec onto an existing clip keeps the ids of tracks and
//! keyframes it can match, so re-importing an edited spec stays a small diff.

use crate::binding::BoundProp;
use crate::color::Color;
use crate::doc::Doc;
use crate::id::{AnimationClipId, AnimationTrackId, AssetId, KeyframeId, NodeId};
use crate::motion::{
    AnimationClip, AnimationTrack, ClipSettings, Interpolation, Keyframe, LoopMode, MotionProperty,
    MotionTarget,
};
use crate::node::{Easing, TextStyle};
use crate::value::{ResolvedVarValue, VariableType};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const MOTION_SPEC_VERSION: u32 = 1;

/// One clip as layers of named tracks, in seconds and degrees.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MotionSpec {
    pub version: u32,
    pub name: String,
    /// Seconds.
    pub duration: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<u16>,
    #[serde(default, rename = "loop", skip_serializing_if = "SpecLoop::is_once")]
    pub loop_mode: SpecLoop,
    /// The stage size, from the scope frame's local bounds when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<[f64; 2]>,
    /// Backdrop as a hex color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// The stage frame, as a prefixed node id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<SpecLayer>,
}

/// Loop vocabulary shared with Figma timelines and Claude Motion stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecLoop {
    #[default]
    Once,
    Loop,
    Boomerang,
}

impl SpecLoop {
    pub fn is_once(&self) -> bool {
        *self == Self::Once
    }
}

impl From<LoopMode> for SpecLoop {
    fn from(mode: LoopMode) -> Self {
        match mode {
            LoopMode::Once => Self::Once,
            LoopMode::Repeat => Self::Loop,
            LoopMode::PingPong => Self::Boomerang,
        }
    }
}

impl From<SpecLoop> for LoopMode {
    fn from(mode: SpecLoop) -> Self {
        match mode {
            SpecLoop::Once => Self::Once,
            SpecLoop::Loop => Self::Repeat,
            SpecLoop::Boomerang => Self::PingPong,
        }
    }
}

/// Every track of one node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpecLayer {
    /// Prefixed node id (`n_…`).
    pub node: String,
    /// The node's name at export time; informational on import.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Rotation and scale pivot as a fraction of the node's local bounds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<[f64; 2]>,
    /// Tracks by property key, keyframes in time order.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tracks: BTreeMap<String, Vec<SpecKeyframe>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpecKeyframe {
    /// Seconds from the clip start.
    pub t: f64,
    /// A number, a hex color, text, a boolean, a text-style object, an asset
    /// id, or null for "no image"; the track's property decides the type.
    pub v: Value,
    /// Timing of the segment that leaves this keyframe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub easing: Option<SpecEasing>,
    /// The value jumps at the next keyframe instead of interpolating.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hold: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SpecEasing {
    Named(SpecEasingName),
    Curve(SpecEasingCurve),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecEasingName {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    /// Figma's `EASE_IN_AND_OUT_BACK`; Fanta stores it as its cubic curve.
    EaseInOutBack,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecEasingCurve {
    /// CSS `cubic-bezier(x1, y1, x2, y2)`.
    CubicBezier([f32; 4]),
    Spring {
        mass: f32,
        stiffness: f32,
        damping: f32,
    },
}

/// The cubic Figma uses for `EASE_IN_AND_OUT_BACK`.
const EASE_IN_OUT_BACK: [f32; 4] = [0.68, -0.6, 0.32, 1.6];

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MotionSpecError {
    #[error("unsupported motion spec version {0}; this build reads version {MOTION_SPEC_VERSION}")]
    Version(u32),
    #[error("the clip duration must be a finite number of seconds, not {0}")]
    Duration(f64),
    #[error("{0:?} is not a node id")]
    NodeId(String),
    #[error("{0:?} is not an asset id")]
    AssetId(String),
    #[error("{0:?} is not an animatable track")]
    TrackKey(String),
    #[error("keyframe {index} of {track:?} on {node:?}: {reason}")]
    Keyframe {
        node: String,
        track: String,
        index: usize,
        reason: String,
    },
    #[error("{0:?} is not a hex color")]
    Color(String),
    #[error("a text style could not be serialized: {0}")]
    Serialize(String),
}

impl MotionSpec {
    /// Project a clip into its portable form. Layer names come from the
    /// document's nodes; a node the document no longer has keeps its id and an
    /// empty name.
    pub fn from_clip(doc: &Doc, clip: &AnimationClip) -> Result<Self, MotionSpecError> {
        let mut layers: BTreeMap<NodeId, SpecLayer> = BTreeMap::new();
        for track in clip.tracks.values() {
            let node = track.target.node;
            let layer = layers.entry(node).or_insert_with(|| SpecLayer {
                node: node.to_string(),
                name: doc
                    .scene
                    .get(node)
                    .map(|node| node.name.clone())
                    .unwrap_or_default(),
                origin: clip.settings.transform_origins.get(&node).copied(),
                tracks: BTreeMap::new(),
            });
            let mut keyframes: Vec<&Keyframe> = track
                .keyframes
                .values()
                .filter(|keyframe| track.target.property.accepts_value(&keyframe.value))
                .collect();
            keyframes.sort_by_key(|keyframe| (keyframe.time_ms, keyframe.id));
            let keyframes = keyframes
                .into_iter()
                .map(|keyframe| {
                    Ok(SpecKeyframe {
                        t: f64::from(keyframe.time_ms) / 1_000.0,
                        v: value_to_spec(track.target.property, &keyframe.value)?,
                        easing: Some(easing_to_spec(keyframe.easing)),
                        hold: keyframe.interpolation == Interpolation::Hold,
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            layer
                .tracks
                .insert(track_key(track.target.property), keyframes);
        }
        let size = clip
            .settings
            .scope
            .and_then(|scope| doc.scene.get(scope))
            .and_then(|node| node.data.local_bounds())
            .map(|bounds| [bounds.width(), bounds.height()]);
        Ok(Self {
            version: MOTION_SPEC_VERSION,
            name: clip.name.clone(),
            duration: f64::from(clip.duration_ms) / 1_000.0,
            fps: clip.settings.fps,
            loop_mode: clip.settings.loop_mode.into(),
            size,
            background: clip.settings.background.map(Color::to_hex),
            scope: clip.settings.scope.map(|scope| scope.to_string()),
            layers: layers.into_values().collect(),
        })
    }

    /// Build the clip this spec describes. With `existing`, tracks for the
    /// same target and keyframes at the same time keep their ids, so the
    /// result diffs minimally against the clip it replaces.
    pub fn to_clip(
        &self,
        id: AnimationClipId,
        existing: Option<&AnimationClip>,
    ) -> Result<AnimationClip, MotionSpecError> {
        if self.version != MOTION_SPEC_VERSION {
            return Err(MotionSpecError::Version(self.version));
        }
        if !self.duration.is_finite() || self.duration < 0.0 {
            return Err(MotionSpecError::Duration(self.duration));
        }
        let mut clip = AnimationClip::new(id, self.name.clone(), seconds_to_ms(self.duration));
        clip.settings = ClipSettings {
            fps: self.fps,
            loop_mode: self.loop_mode.into(),
            scope: self.scope.as_deref().map(parse_node_id).transpose()?,
            background: self
                .background
                .as_deref()
                .map(|hex| {
                    Color::from_hex(hex)
                        .or_else(|| Color::from_hex(&format!("#{hex}")))
                        .ok_or_else(|| MotionSpecError::Color(hex.to_owned()))
                })
                .transpose()?,
            transform_origins: BTreeMap::new(),
        };
        for layer in &self.layers {
            let node = parse_node_id(&layer.node)?;
            if let Some(origin) = layer.origin {
                clip.settings.transform_origins.insert(node, origin);
            }
            for (key, keyframes) in &layer.tracks {
                let property =
                    parse_track_key(key).ok_or_else(|| MotionSpecError::TrackKey(key.clone()))?;
                let target = MotionTarget::new(node, property);
                let previous = existing.and_then(|clip| clip.track_for_target(target));
                let mut track = AnimationTrack::new(
                    previous.map_or_else(AnimationTrackId::new, |track| track.id),
                    target,
                );
                for (index, keyframe) in keyframes.iter().enumerate() {
                    let error = |reason: String| MotionSpecError::Keyframe {
                        node: layer.node.clone(),
                        track: key.clone(),
                        index,
                        reason,
                    };
                    if !keyframe.t.is_finite() || keyframe.t < 0.0 {
                        return Err(error(format!(
                            "time {} is not a non-negative number",
                            keyframe.t
                        )));
                    }
                    let time_ms = seconds_to_ms(keyframe.t);
                    let value = value_from_spec(property, &keyframe.v).map_err(error)?;
                    let id = previous
                        .and_then(|track| {
                            track
                                .keyframes
                                .values()
                                .find(|candidate| candidate.time_ms == time_ms)
                        })
                        .map_or_else(KeyframeId::new, |keyframe| keyframe.id);
                    let mut built = Keyframe::new(id, time_ms, value);
                    built.interpolation = if keyframe.hold {
                        Interpolation::Hold
                    } else {
                        Interpolation::Linear
                    };
                    if let Some(easing) = &keyframe.easing {
                        built.easing = easing_from_spec(easing);
                    }
                    track.keyframes.insert(built.id, built);
                }
                clip.tracks.insert(track.id, track);
            }
        }
        Ok(clip)
    }
}

fn seconds_to_ms(seconds: f64) -> u32 {
    (seconds * 1_000.0).round().clamp(0.0, f64::from(u32::MAX)) as u32
}

fn parse_node_id(text: &str) -> Result<NodeId, MotionSpecError> {
    text.parse::<NodeId>()
        .map_err(|_| MotionSpecError::NodeId(text.to_owned()))
}

/// The spec key for a property: transform channels by their Figma-style
/// names, bound properties by their serde tag, with `:index` for a paint slot
/// other than the first.
pub fn track_key(property: MotionProperty) -> String {
    let indexed = |name: &str, index: u16| {
        if index == 0 {
            name.to_owned()
        } else {
            format!("{name}:{index}")
        }
    };
    match property {
        MotionProperty::PositionX => "translation_x".to_owned(),
        MotionProperty::PositionY => "translation_y".to_owned(),
        MotionProperty::Rotation => "rotation".to_owned(),
        MotionProperty::ScaleX => "scale_x".to_owned(),
        MotionProperty::ScaleY => "scale_y".to_owned(),
        MotionProperty::Bound { prop } => match prop {
            BoundProp::FillColor { index } => indexed("fill_color", index),
            BoundProp::StrokeColor { index } => indexed("stroke_color", index),
            BoundProp::StrokeWidth { index } => indexed("stroke_width", index),
            BoundProp::ImageFill { index } => indexed("image_fill", index),
            BoundProp::CornerRadius => "corner_radius".to_owned(),
            BoundProp::Opacity => "opacity".to_owned(),
            BoundProp::Visible => "visible".to_owned(),
            BoundProp::TextContent => "text_content".to_owned(),
            BoundProp::TextStyle => "text_style".to_owned(),
            BoundProp::ClipWidth => "clip_width".to_owned(),
            BoundProp::ClipHeight => "clip_height".to_owned(),
        },
    }
}

pub fn parse_track_key(key: &str) -> Option<MotionProperty> {
    let (name, index) = match key.split_once(':') {
        Some((name, index)) => (name, Some(index.parse::<u16>().ok()?)),
        None => (key, None),
    };
    let bound = |prop: BoundProp| Some(MotionProperty::Bound { prop });
    let slot = index.unwrap_or(0);
    match (name, index) {
        ("translation_x", None) => Some(MotionProperty::PositionX),
        ("translation_y", None) => Some(MotionProperty::PositionY),
        ("rotation", None) => Some(MotionProperty::Rotation),
        ("scale_x", None) => Some(MotionProperty::ScaleX),
        ("scale_y", None) => Some(MotionProperty::ScaleY),
        ("fill_color", _) => bound(BoundProp::FillColor { index: slot }),
        ("stroke_color", _) => bound(BoundProp::StrokeColor { index: slot }),
        ("stroke_width", _) => bound(BoundProp::StrokeWidth { index: slot }),
        ("image_fill", _) => bound(BoundProp::ImageFill { index: slot }),
        ("corner_radius", None) => bound(BoundProp::CornerRadius),
        ("opacity", None) => bound(BoundProp::Opacity),
        ("visible", None) => bound(BoundProp::Visible),
        ("text_content", None) => bound(BoundProp::TextContent),
        ("text_style", None) => bound(BoundProp::TextStyle),
        ("clip_width", None) => bound(BoundProp::ClipWidth),
        ("clip_height", None) => bound(BoundProp::ClipHeight),
        _ => None,
    }
}

/// Degrees, rounded so "nice" angles survive a round trip.
fn radians_to_spec_degrees(radians: f64) -> f64 {
    (radians.to_degrees() * 1e6).round() / 1e6
}

fn value_to_spec(
    property: MotionProperty,
    value: &ResolvedVarValue,
) -> Result<Value, MotionSpecError> {
    Ok(match (property, value) {
        (MotionProperty::Rotation, ResolvedVarValue::Float { value }) => {
            json!(radians_to_spec_degrees(*value))
        }
        (_, ResolvedVarValue::Float { value }) => json!(value),
        (_, ResolvedVarValue::Color { value }) => json!(value.to_hex()),
        (_, ResolvedVarValue::String { value }) | (_, ResolvedVarValue::Enum { value }) => {
            json!(value)
        }
        (_, ResolvedVarValue::Boolean { value }) => json!(value),
        (_, ResolvedVarValue::TextStyle { value }) => serde_json::to_value(value)
            .map_err(|error| MotionSpecError::Serialize(error.to_string()))?,
        (_, ResolvedVarValue::Image { value }) => {
            value.map_or(Value::Null, |asset| json!(asset.to_string()))
        }
    })
}

fn value_from_spec(property: MotionProperty, value: &Value) -> Result<ResolvedVarValue, String> {
    let kind = match property {
        MotionProperty::Bound { prop } => prop.variable_type(),
        _ => VariableType::Float,
    };
    let number = || {
        value
            .as_f64()
            .filter(|number| number.is_finite())
            .ok_or_else(|| format!("{value} is not a finite number"))
    };
    Ok(match kind {
        VariableType::Float => {
            let number = number()?;
            ResolvedVarValue::Float {
                value: if property == MotionProperty::Rotation {
                    number.to_radians()
                } else {
                    number
                },
            }
        }
        VariableType::Color => {
            let hex = value
                .as_str()
                .ok_or_else(|| format!("{value} is not a hex color"))?;
            ResolvedVarValue::Color {
                value: Color::from_hex(hex)
                    .or_else(|| Color::from_hex(&format!("#{hex}")))
                    .ok_or_else(|| format!("{hex:?} is not a hex color"))?,
            }
        }
        VariableType::String => ResolvedVarValue::String {
            value: value
                .as_str()
                .ok_or_else(|| format!("{value} is not text"))?
                .to_owned(),
        },
        VariableType::Enum => ResolvedVarValue::Enum {
            value: value
                .as_str()
                .ok_or_else(|| format!("{value} is not an option name"))?
                .to_owned(),
        },
        VariableType::Boolean => ResolvedVarValue::Boolean {
            value: value
                .as_bool()
                .ok_or_else(|| format!("{value} is not true or false"))?,
        },
        VariableType::Typography => ResolvedVarValue::TextStyle {
            value: serde_json::from_value::<TextStyle>(value.clone())
                .map_err(|error| format!("not a text style: {error}"))?,
        },
        VariableType::Image => ResolvedVarValue::Image {
            value: match value {
                Value::Null => None,
                Value::String(text) => Some(
                    text.parse::<AssetId>()
                        .map_err(|_| format!("{text:?} is not an asset id"))?,
                ),
                other => return Err(format!("{other} is not an asset id or null")),
            },
        },
    })
}

pub fn easing_to_spec(easing: Easing) -> SpecEasing {
    match easing {
        Easing::Linear => SpecEasing::Named(SpecEasingName::Linear),
        Easing::EaseIn => SpecEasing::Named(SpecEasingName::EaseIn),
        Easing::EaseOut => SpecEasing::Named(SpecEasingName::EaseOut),
        Easing::EaseInOut => SpecEasing::Named(SpecEasingName::EaseInOut),
        Easing::CubicBezier { x1, y1, x2, y2 } if [x1, y1, x2, y2] == EASE_IN_OUT_BACK => {
            SpecEasing::Named(SpecEasingName::EaseInOutBack)
        }
        Easing::CubicBezier { x1, y1, x2, y2 } => {
            SpecEasing::Curve(SpecEasingCurve::CubicBezier([x1, y1, x2, y2]))
        }
        Easing::Spring {
            mass,
            stiffness,
            damping,
        } => SpecEasing::Curve(SpecEasingCurve::Spring {
            mass,
            stiffness,
            damping,
        }),
    }
}

pub fn easing_from_spec(easing: &SpecEasing) -> Easing {
    match easing {
        SpecEasing::Named(SpecEasingName::Linear) => Easing::Linear,
        SpecEasing::Named(SpecEasingName::EaseIn) => Easing::EaseIn,
        SpecEasing::Named(SpecEasingName::EaseOut) => Easing::EaseOut,
        SpecEasing::Named(SpecEasingName::EaseInOut) => Easing::EaseInOut,
        SpecEasing::Named(SpecEasingName::EaseInOutBack) => {
            let [x1, y1, x2, y2] = EASE_IN_OUT_BACK;
            Easing::CubicBezier { x1, y1, x2, y2 }
        }
        SpecEasing::Curve(SpecEasingCurve::CubicBezier([x1, y1, x2, y2])) => Easing::CubicBezier {
            x1: *x1,
            y1: *y1,
            x2: *x2,
            y2: *y2,
        },
        SpecEasing::Curve(SpecEasingCurve::Spring {
            mass,
            stiffness,
            damping,
        }) => Easing::Spring {
            mass: *mass,
            stiffness: *stiffness,
            damping: *damping,
        },
    }
}

/// The Figma Plugin API keyframe field for a spec track, for the tracks Figma
/// keys the same way. Paint, text and geometry tracks have no field here.
pub fn figma_track_field(key: &str) -> Option<&'static str> {
    Some(match key {
        "translation_x" => "TRANSLATION_X",
        "translation_y" => "TRANSLATION_Y",
        "rotation" => "ROTATION",
        "scale_x" => "SCALE_X",
        "scale_y" => "SCALE_Y",
        "opacity" => "OPACITY",
        _ => return None,
    })
}

/// A spec keyframe's timing as the Figma Plugin API's easing object.
pub fn figma_easing(easing: &SpecEasing, hold: bool) -> Value {
    if hold {
        return json!({ "type": "HOLD" });
    }
    match easing {
        SpecEasing::Named(SpecEasingName::Linear) => json!({ "type": "LINEAR" }),
        SpecEasing::Named(SpecEasingName::EaseIn) => json!({ "type": "EASE_IN" }),
        SpecEasing::Named(SpecEasingName::EaseOut) => json!({ "type": "EASE_OUT" }),
        SpecEasing::Named(SpecEasingName::EaseInOut) => json!({ "type": "EASE_IN_AND_OUT" }),
        SpecEasing::Named(SpecEasingName::EaseInOutBack) => {
            json!({ "type": "EASE_IN_AND_OUT_BACK" })
        }
        SpecEasing::Curve(SpecEasingCurve::CubicBezier([x1, y1, x2, y2])) => json!({
            "type": "CUSTOM_CUBIC_BEZIER",
            "easingFunctionCubicBezier": { "x1": x1, "y1": y1, "x2": x2, "y2": y2 },
        }),
        SpecEasing::Curve(SpecEasingCurve::Spring {
            mass,
            stiffness,
            damping,
        }) => json!({
            "type": "CUSTOM_SPRING",
            "easingFunctionSpring": { "mass": mass, "stiffness": stiffness, "damping": damping },
        }),
    }
}

/// The inverse of [`figma_easing`]: the spec easing and hold flag for a Figma
/// easing object, or `None` for a shape this build does not know.
pub fn spec_easing_from_figma(value: &Value) -> Option<(SpecEasing, bool)> {
    let kind = value.get("type")?.as_str()?;
    let named = |name| Some((SpecEasing::Named(name), false));
    match kind {
        "HOLD" => Some((SpecEasing::Named(SpecEasingName::Linear), true)),
        "LINEAR" => named(SpecEasingName::Linear),
        "EASE_IN" => named(SpecEasingName::EaseIn),
        "EASE_OUT" => named(SpecEasingName::EaseOut),
        "EASE_IN_AND_OUT" => named(SpecEasingName::EaseInOut),
        "EASE_IN_AND_OUT_BACK" => named(SpecEasingName::EaseInOutBack),
        "CUSTOM_CUBIC_BEZIER" => {
            let curve = value.get("easingFunctionCubicBezier")?;
            let point = |key: &str| curve.get(key)?.as_f64().map(|number| number as f32);
            Some((
                SpecEasing::Curve(SpecEasingCurve::CubicBezier([
                    point("x1")?,
                    point("y1")?,
                    point("x2")?,
                    point("y2")?,
                ])),
                false,
            ))
        }
        "CUSTOM_SPRING" => {
            let spring = value.get("easingFunctionSpring")?;
            let field = |key: &str| spring.get(key)?.as_f64().map(|number| number as f32);
            Some((
                SpecEasing::Curve(SpecEasingCurve::Spring {
                    mass: field("mass")?,
                    stiffness: field("stiffness")?,
                    damping: field("damping")?,
                }),
                false,
            ))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{CanvasNode, GroupNode, NodeData};
    use crate::op::Operation;

    fn keyframe(id: u128, time_ms: u32, value: ResolvedVarValue, easing: Easing) -> Keyframe {
        let mut keyframe = Keyframe::new(KeyframeId::from_u128(id), time_ms, value);
        keyframe.easing = easing;
        keyframe
    }

    fn stage_doc() -> (Doc, NodeId, NodeId) {
        let mut doc = Doc::new();
        let mut stage = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([390.0, 844.0]),
            ..GroupNode::default()
        }));
        stage.name = "Hero".into();
        let stage_id = stage.id;
        doc.apply(Operation::create_node(stage)).expect("stage");
        let mut card = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([200.0, 100.0]),
            ..GroupNode::default()
        }));
        card.name = "Card".into();
        let card_id = card.id;
        doc.apply(Operation::create_node(card)).expect("card");
        (doc, stage_id, card_id)
    }

    fn sample_clip(stage: NodeId, card: NodeId) -> AnimationClip {
        let mut clip = AnimationClip::new(AnimationClipId::from_u128(1), "Hero", 1_200);
        clip.settings = ClipSettings {
            fps: Some(30),
            loop_mode: LoopMode::PingPong,
            scope: Some(stage),
            background: Some(Color::rgb(0x11, 0x22, 0x33)),
            transform_origins: BTreeMap::from([(card, [0.5, 0.5])]),
        };
        let tracks = [
            (
                2,
                MotionProperty::PositionX,
                vec![
                    keyframe(
                        10,
                        0,
                        ResolvedVarValue::Float { value: 0.0 },
                        Easing::EaseOut,
                    ),
                    keyframe(
                        11,
                        400,
                        ResolvedVarValue::Float { value: 120.0 },
                        Easing::Linear,
                    ),
                ],
            ),
            (
                3,
                MotionProperty::Rotation,
                vec![
                    keyframe(
                        12,
                        0,
                        ResolvedVarValue::Float { value: 0.0 },
                        Easing::EaseInOut,
                    ),
                    keyframe(
                        13,
                        1_200,
                        ResolvedVarValue::Float {
                            value: 90f64.to_radians(),
                        },
                        Easing::Spring {
                            mass: 1.0,
                            stiffness: 100.0,
                            damping: 15.0,
                        },
                    ),
                ],
            ),
            (
                4,
                MotionProperty::Bound {
                    prop: BoundProp::Opacity,
                },
                vec![
                    keyframe(
                        14,
                        0,
                        ResolvedVarValue::Float { value: 0.0 },
                        Easing::CubicBezier {
                            x1: 0.2,
                            y1: 0.0,
                            x2: 0.0,
                            y2: 1.0,
                        },
                    ),
                    keyframe(
                        15,
                        300,
                        ResolvedVarValue::Float { value: 1.0 },
                        Easing::EaseInOut,
                    ),
                ],
            ),
            (
                5,
                MotionProperty::Bound {
                    prop: BoundProp::FillColor { index: 1 },
                },
                vec![{
                    let mut held = keyframe(
                        16,
                        0,
                        ResolvedVarValue::Color {
                            value: Color::rgb(255, 0, 0),
                        },
                        Easing::Linear,
                    );
                    held.interpolation = Interpolation::Hold;
                    held
                }],
            ),
        ];
        for (track_id, property, keyframes) in tracks {
            let mut track = AnimationTrack::new(
                AnimationTrackId::from_u128(track_id),
                MotionTarget::new(card, property),
            );
            for keyframe in keyframes {
                track.keyframes.insert(keyframe.id, keyframe);
            }
            clip.tracks.insert(track.id, track);
        }
        clip
    }

    #[test]
    fn spec_round_trips_clip_without_loss_and_keeps_ids() {
        let (doc, stage, card) = stage_doc();
        let clip = sample_clip(stage, card);
        let spec = MotionSpec::from_clip(&doc, &clip).expect("spec");
        assert_eq!(spec.duration, 1.2);
        assert_eq!(spec.loop_mode, SpecLoop::Boomerang);
        assert_eq!(spec.size, Some([390.0, 844.0]));
        assert_eq!(spec.background.as_deref(), Some("#112233"));
        assert_eq!(spec.scope.as_deref(), Some(stage.to_string().as_str()));
        assert_eq!(spec.layers.len(), 1);
        let layer = &spec.layers[0];
        assert_eq!(layer.name, "Card");
        assert_eq!(layer.origin, Some([0.5, 0.5]));
        assert_eq!(layer.tracks["rotation"][1].v, json!(90.0));
        assert_eq!(layer.tracks["fill_color:1"][0].v, json!("#FF0000"));
        assert!(layer.tracks["fill_color:1"][0].hold);
        assert_eq!(layer.tracks["translation_x"][1].t, 0.4);

        let json = serde_json::to_string_pretty(&spec).expect("json");
        let parsed: MotionSpec = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed, spec);

        let rebuilt = parsed.to_clip(clip.id, Some(&clip)).expect("clip");
        assert_eq!(rebuilt.settings, clip.settings);
        assert_eq!(rebuilt.duration_ms, clip.duration_ms);
        assert_eq!(
            rebuilt.tracks.keys().collect::<Vec<_>>(),
            clip.tracks.keys().collect::<Vec<_>>()
        );
        for (track_id, track) in &clip.tracks {
            let rebuilt_track = &rebuilt.tracks[track_id];
            assert_eq!(rebuilt_track.target, track.target);
            for (keyframe_id, keyframe) in &track.keyframes {
                let rebuilt_keyframe = &rebuilt_track.keyframes[keyframe_id];
                assert_eq!(rebuilt_keyframe.time_ms, keyframe.time_ms);
                assert_eq!(rebuilt_keyframe.easing, keyframe.easing);
                assert_eq!(rebuilt_keyframe.interpolation, keyframe.interpolation);
                match (&rebuilt_keyframe.value, &keyframe.value) {
                    (
                        ResolvedVarValue::Float { value: a },
                        ResolvedVarValue::Float { value: b },
                    ) if track.target.property == MotionProperty::Rotation => {
                        assert!((a - b).abs() < 1e-9, "rotation {a} vs {b}");
                    }
                    (a, b) => assert_eq!(a, b),
                }
            }
        }

        // Without the previous clip, the structure is the same under fresh ids.
        let fresh = spec
            .to_clip(AnimationClipId::new(), None)
            .expect("fresh clip");
        assert_eq!(fresh.tracks.len(), clip.tracks.len());
        assert!(fresh.tracks.keys().all(|id| !clip.tracks.contains_key(id)));
    }

    #[test]
    fn track_keys_cover_every_motion_property() {
        let properties = [
            MotionProperty::PositionX,
            MotionProperty::PositionY,
            MotionProperty::Rotation,
            MotionProperty::ScaleX,
            MotionProperty::ScaleY,
            MotionProperty::bound(BoundProp::FillColor { index: 0 }),
            MotionProperty::bound(BoundProp::FillColor { index: 2 }),
            MotionProperty::bound(BoundProp::StrokeColor { index: 1 }),
            MotionProperty::bound(BoundProp::StrokeWidth { index: 0 }),
            MotionProperty::bound(BoundProp::ImageFill { index: 0 }),
            MotionProperty::bound(BoundProp::CornerRadius),
            MotionProperty::bound(BoundProp::Opacity),
            MotionProperty::bound(BoundProp::Visible),
            MotionProperty::bound(BoundProp::TextContent),
            MotionProperty::bound(BoundProp::TextStyle),
            MotionProperty::bound(BoundProp::ClipWidth),
            MotionProperty::bound(BoundProp::ClipHeight),
        ];
        for property in properties {
            let key = track_key(property);
            assert_eq!(parse_track_key(&key), Some(property), "{key}");
        }
        assert_eq!(
            track_key(MotionProperty::bound(BoundProp::FillColor { index: 2 })),
            "fill_color:2"
        );
        assert_eq!(parse_track_key("opacity:1"), None);
        assert_eq!(parse_track_key("wobble"), None);
    }

    #[test]
    fn figma_easing_mapping_is_bijective_for_every_shape() {
        let easings = [
            SpecEasing::Named(SpecEasingName::Linear),
            SpecEasing::Named(SpecEasingName::EaseIn),
            SpecEasing::Named(SpecEasingName::EaseOut),
            SpecEasing::Named(SpecEasingName::EaseInOut),
            SpecEasing::Named(SpecEasingName::EaseInOutBack),
            SpecEasing::Curve(SpecEasingCurve::CubicBezier([0.2, 0.0, 0.0, 1.0])),
            SpecEasing::Curve(SpecEasingCurve::Spring {
                mass: 1.0,
                stiffness: 120.0,
                damping: 12.0,
            }),
        ];
        for easing in &easings {
            let figma = figma_easing(easing, false);
            assert_eq!(
                spec_easing_from_figma(&figma),
                Some((easing.clone(), false)),
                "{figma}"
            );
            let doc_easing = easing_from_spec(easing);
            assert_eq!(easing_to_spec(doc_easing), *easing);
        }
        assert_eq!(
            spec_easing_from_figma(&figma_easing(&easings[0], true)),
            Some((SpecEasing::Named(SpecEasingName::Linear), true))
        );
        assert_eq!(
            easing_from_spec(&SpecEasing::Named(SpecEasingName::EaseInOutBack)),
            Easing::CubicBezier {
                x1: 0.68,
                y1: -0.6,
                x2: 0.32,
                y2: 1.6
            }
        );
        assert_eq!(figma_track_field("translation_x"), Some("TRANSLATION_X"));
        assert_eq!(figma_track_field("fill_color"), None);
    }

    #[test]
    fn rotation_and_time_units_convert_both_ways() {
        let (doc, stage, card) = stage_doc();
        let clip = sample_clip(stage, card);
        let spec = MotionSpec::from_clip(&doc, &clip).expect("spec");
        let rotation = &spec.layers[0].tracks["rotation"];
        assert_eq!(rotation[1].t, 1.2);
        assert_eq!(rotation[1].v, json!(90.0));
        let rebuilt = spec.to_clip(clip.id, None).expect("clip");
        let track = rebuilt
            .track_for_target(MotionTarget::new(card, MotionProperty::Rotation))
            .expect("rotation track");
        let last = track
            .keyframes
            .values()
            .max_by_key(|keyframe| keyframe.time_ms)
            .expect("last");
        assert_eq!(last.time_ms, 1_200);
        assert!(
            matches!(last.value, ResolvedVarValue::Float { value } if (value - 90f64.to_radians()).abs() < 1e-12)
        );
    }

    #[test]
    fn spec_rejects_unknown_tracks_bad_ids_and_wrong_value_types() {
        let (_, _, card) = stage_doc();
        let mut spec = MotionSpec {
            version: MOTION_SPEC_VERSION,
            name: "Bad".into(),
            duration: 1.0,
            fps: None,
            loop_mode: SpecLoop::Once,
            size: None,
            background: None,
            scope: None,
            layers: vec![SpecLayer {
                node: card.to_string(),
                name: String::new(),
                origin: None,
                tracks: BTreeMap::from([("wobble".to_owned(), Vec::new())]),
            }],
        };
        assert!(matches!(
            spec.to_clip(AnimationClipId::new(), None),
            Err(MotionSpecError::TrackKey(key)) if key == "wobble"
        ));
        spec.layers[0].tracks = BTreeMap::from([(
            "opacity".to_owned(),
            vec![SpecKeyframe {
                t: 0.0,
                v: json!("#FF0000"),
                easing: None,
                hold: false,
            }],
        )]);
        assert!(matches!(
            spec.to_clip(AnimationClipId::new(), None),
            Err(MotionSpecError::Keyframe { track, index: 0, .. }) if track == "opacity"
        ));
        spec.layers[0].node = "not-a-node".into();
        assert!(matches!(
            spec.to_clip(AnimationClipId::new(), None),
            Err(MotionSpecError::NodeId(_))
        ));
        spec.version = 2;
        assert!(matches!(
            spec.to_clip(AnimationClipId::new(), None),
            Err(MotionSpecError::Version(2))
        ));
    }
}
