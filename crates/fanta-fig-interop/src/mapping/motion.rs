//! Figma keyframe motion → [`fanta_doc::MotionLibrary`] (pass 1 collect,
//! pass 4 resolve). Wired like `reactions.rs`: pass 1 stashes the raw material
//! in a [`PendingMotion`] side-table, and [`apply_motion`] resolves it onto
//! `doc.motion` once the scene exists.
//!
//! ## Discovered raw shape (reverse-engineered from a real fixture)
//!
//! Figma's keyframe-motion feature stores animation as three `NodeChange`
//! types plus expression bindings on the animated nodes. Everything below was
//! observed by dumping a real user file's decoded `KiwiValue` tree; fields not
//! listed were not present in the fixture.
//!
//! 1. `ANIMATION_PRESET_INSTANCE` — the clip container. A `NodeChange`
//!    parented under the document's internal-only `CANVAS` (a canvas carrying
//!    `internalOnly: true`, named "Internal Only Canvas"). Observed fields:
//!    `name` = `"motion.preset_name.scale"` (the applied preset's i18n key),
//!    `size` (100×100), identity `transform`, `opacity`, `visible`, `phase`,
//!    plus `backingCodeComponentId` (an `AssetRef {key, version}` naming a
//!    sibling `CODE_COMPONENT` of the same name) and a `codeSnapshot` render
//!    cache — both irrelevant to playback.
//! 2. `KEYFRAME_TRACK` — one animation lane. Parented under the preset
//!    instance via `parentIndex.guid`; sibling order via the usual
//!    `parentIndex.position` fractional index. Observed fields:
//!    `keyframeOperation: enum KeyframeOperation { SET=0, SCALE=1, OFFSET=2 }`
//!    (observed `SCALE`), `name` = "Keyframe Track", `visible`. The track does
//!    NOT name the property it animates — the binding lives on the consuming
//!    node (item 4).
//! 3. `KEYFRAME` — parented under its track. Observed fields:
//!    - `timelinePosition: int64` — playhead time in MICROSECONDS (observed
//!      `500000` = 0.5 s; ABSENT on the t=0 keyframe). The schema also
//!      declares `timelinePositionType: enum { ABSOLUTE, RELATIVE }`
//!      (unobserved; treated as absolute).
//!    - `keyframeValue: KeyframeValueData { value: KeyframeAnyValue
//!      { floatValue | colorValue | textDataValue | vectorValue | boolValue |
//!      circleValue | lineValue | circlePointValue | colorPointValue |
//!      gradientParameterValue }, valueType: enum KeyframeValueType { FLOAT=0,
//!      INVALID, COLOR, TEXT_DATA, VECTOR, BOOL, CIRCLE, LINE, CIRCLE_POINT,
//!      COLOR_POINT, GRADIENT_PAINT } }`. Observed: `FLOAT` only (values 1.2
//!      and 1.0).
//!    - `easingData: EasingData { easingType: enum EasingType { IN_CUBIC=0,
//!      OUT_CUBIC, INOUT_CUBIC, LINEAR, IN_BACK_CUBIC, OUT_BACK_CUBIC,
//!      INOUT_BACK_CUBIC, CUSTOM_CUBIC, SPRING, GENTLE_SPRING, CUSTOM_SPRING,
//!      SPRING_PRESET_ONE, SPRING_PRESET_TWO, SPRING_PRESET_THREE, HOLD,
//!      EASE_IN }, easingValue: TransitionEasingAnyValue { springEasing:
//!      SpringParams { stiffness, damping, mass }, bezierEasing:
//!      BezierHandles { p1x, p1y, p2x, p2y } } }`. Observed: `easingType:
//!      OUT_CUBIC` only; the `easingValue` arm shapes are schema-verified but
//!      value-unobserved.
//! 4. Track → node binding lives on the ANIMATED node, not on the track. The
//!    fixture's consumer (a TEXT) carries IDENTICAL entries in BOTH
//!    `variableConsumptionMap` and `parameterConsumptionMap` (each a
//!    `VariableDataMap { entries: VariableDataMapEntry[] }`). Each entry:
//!    - `variableField: enum VariableField` — observed `MOTION_SCALE_X` (49)
//!      and `MOTION_SCALE_Y` (50). The enum also declares
//!      `MOTION_TRANSLATION_X`/`_Y` (46/47), `MOTION_ROTATION` (48),
//!      `MOTION_SHEAR` (51), vector-valued `MOTION_TRANSLATION_XY` /
//!      `MOTION_SCALE_XY` (69/70), transform-origin channels (73–75),
//!      `OPACITY` (31), and the many non-motion bindable fields.
//!    - `variableData: VariableData { dataType: EXPRESSION, resolvedDataType:
//!      FLOAT, value: VariableAnyValue { expressionValue: Expression {
//!      expressionFunction: KEYFRAME (member 19 of ExpressionFunction),
//!      expressionArguments: [
//!        VariableData { dataType: FLOAT, value.floatValue: <base — observed 1> },
//!        VariableData { dataType: KEYFRAME_TRACK_PARAMETER_DATA,
//!          value.keyframeTrackParameterValue.parameters[0]:
//!            KeyframeTrackParameter { type: ANIMATION_PRESET,
//!              value: KeyframeTrackAnyParameter { animationPreset:
//!                AnimationPresetKeyframeTrackParameter {
//!                  animationPresetId: { guid }, keyframeTrackId: { guid },
//!                  timelineDefId: GUID } } } } ] } } }`.
//!      The schema also declares a `MANUAL` parameter arm (`value.manual:
//!      ManualKeyframeTrackParameter { keyframeTrackId, timelineDefId }`,
//!      unobserved) for keyframe tracks authored outside a preset.
//!    - The consumer additionally carries `animationPresets: AnimationPresets
//!      { presets: [AnimationPresetData { animationPresetId, timelineDefId }] }`.
//! 5. Timeline definitions: the animated node's top-level container (the
//!    SYMBOL master in the fixture) carries `timelineDefinitions:
//!    TimelineDefinitionsMap { entries: [TimelineDefinitionsMapEntry { id:
//!    GUID, data: TimelineData { durationUs: uint64 (observed 0 = auto),
//!    defaultTimeline: bool (observed true), plus unobserved
//!    parentTimelineDefId / playbackStyle / autoplay / keyframeSnappingFps /
//!    quantizePlayback } }] }`. The `timelineDefId` in the bindings points at
//!    these entries; the timeline guid is NOT itself a `NodeChange`.
//!
//! Semantics: the KEYFRAME expression samples the referenced track at the
//! playhead and composes the sampled value with the base float argument per
//! the track's `keyframeOperation` (SET replaces, SCALE multiplies, OFFSET
//! adds — only SCALE observed; the other two follow the enum-member names).
//! The result drives the node's `MOTION_*` channel, which layers on top of
//! the committed layout transform. The fixture animates `MOTION_SCALE_X/Y =
//! KEYFRAME(1.0, track)` with keyframes 1.2 → 1.0 over 0..500 000 µs eased
//! OUT_CUBIC — the "Scale" preset: pop from 120% to 100% in half a second.
//!
//! ## What maps where
//!
//! | Figma                                   | Fantaisa                        |
//! |-----------------------------------------|---------------------------------|
//! | `ANIMATION_PRESET_INSTANCE`             | one [`AnimationClip`] (name = the preset's name) |
//! | `KEYFRAME_TRACK` + its consumer binding | one [`AnimationTrack`] per bound (node, channel) |
//! | `KEYFRAME`                              | one [`Keyframe`] (µs → ms, value composed per operation) |
//! | `MOTION_SCALE_X` / `_Y`                 | [`MotionProperty::ScaleX`] / [`ScaleY`] × the committed scale |
//! | `MOTION_TRANSLATION_X` / `_Y`           | [`MotionProperty::PositionX`] / [`PositionY`] + the committed translation |
//! | `MOTION_ROTATION`                       | [`MotionProperty::Rotation`] + the committed rotation |
//! | `OPACITY`                               | `MotionProperty::Bound(BoundProp::Opacity)` (absolute) |
//! | `HOLD` easing                           | [`Interpolation::Hold`]         |
//! | other easing members                    | [`Easing`] (cubic / bezier / spring), see [`read_keyframe_easing`] |
//! | everything else (shear, 3D, vector-valued channels, non-float values) | dropped + counted in `MapReport::motion_keyframes_dropped` |
//!
//! [`ScaleY`]: MotionProperty::ScaleY
//! [`PositionY`]: MotionProperty::PositionY

use super::{Doc, Easing, HashMap, KiwiValue, MapReport, NodeId, guid_key, stable_hash_u128};
use fanta_doc::id::{AnimationClipId, AnimationTrackId, KeyframeId};
use fanta_doc::motion::{
    AnimationClip, AnimationTrack, Interpolation, Keyframe, MotionProperty, MotionTarget,
    MotionTransform,
};
use fanta_doc::value::ResolvedVarValue;

// =============================================================================
// Pending side-table records (collected in pass 1)
// =============================================================================

/// Raw keyframe-motion material collected during pass 1 and resolved onto
/// [`Doc::motion`] in pass 4 (once the scene and its `NodeId`s exist).
#[derive(Default)]
pub(crate) struct PendingMotion {
    /// `ANIMATION_PRESET_INSTANCE` changes in stream order: (guid, name).
    pub(crate) presets: Vec<(String, String)>,
    /// `KEYFRAME_TRACK` changes in stream order.
    pub(crate) tracks: Vec<PendingMotionTrack>,
    /// `KEYFRAME` changes in stream order.
    pub(crate) keyframes: Vec<PendingMotionKeyframe>,
    /// Consumer bindings (deduped): which scene node's which `VariableField`
    /// each track drives, plus the expression's base float and timeline ref.
    pub(crate) bindings: Vec<PendingKeyframeBinding>,
    /// `timelineDefinitions` entries across all nodes: timeline guid →
    /// `durationUs` (0 = auto, i.e. derive from the last keyframe).
    pub(crate) timeline_durations_us: HashMap<String, u64>,
}

/// One `KEYFRAME_TRACK` change.
pub(crate) struct PendingMotionTrack {
    pub(crate) guid: String,
    /// The owning `ANIMATION_PRESET_INSTANCE` guid (`parentIndex.guid`).
    pub(crate) parent_guid: Option<String>,
    pub(crate) operation: KeyframeOperation,
}

/// Figma `KeyframeOperation`: how a sampled track value composes with the
/// KEYFRAME expression's base argument. Only `SCALE` was observed in the
/// fixture; `SET`/`OFFSET` follow the enum-member names (ASSUMPTION).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyframeOperation {
    Set,
    Scale,
    Offset,
}

impl KeyframeOperation {
    fn compose(self, base: Option<f64>, value: f64) -> f64 {
        match self {
            // ASSUMPTION: SET ignores the base (unobserved; per the name).
            Self::Set => value,
            Self::Scale => base.unwrap_or(1.0) * value,
            // ASSUMPTION: OFFSET adds to the base (unobserved; per the name).
            Self::Offset => base.unwrap_or(0.0) + value,
        }
    }
}

/// One `KEYFRAME` change, parsed to the fields the model consumes.
pub(crate) struct PendingMotionKeyframe {
    pub(crate) guid: String,
    /// The owning `KEYFRAME_TRACK` guid.
    pub(crate) parent_guid: Option<String>,
    /// `timelinePosition` in microseconds (absent = 0).
    pub(crate) time_us: i64,
    /// `keyframeValue.value.floatValue` when `valueType` is FLOAT; `None` for
    /// the non-float value kinds (COLOR/TEXT_DATA/VECTOR/... — none observed),
    /// which the resolver counts as dropped rather than guessing a conversion.
    pub(crate) float_value: Option<f64>,
    pub(crate) interpolation: Interpolation,
    pub(crate) easing: Easing,
}

/// One resolved consumer binding: `track` drives `field` of scene node `node`,
/// with the KEYFRAME expression's base float and the timeline the preset
/// binds to (for the clip's duration).
#[derive(PartialEq)]
pub(crate) struct PendingKeyframeBinding {
    pub(crate) node: NodeId,
    pub(crate) track_guid: String,
    /// The `VariableField` enum member name (e.g. `"MOTION_SCALE_X"`).
    pub(crate) field: String,
    pub(crate) base: Option<f64>,
    pub(crate) timeline_def_guid: Option<String>,
}

// =============================================================================
// Pass 1 — collection
// =============================================================================

/// Collect a keyframe-motion `NodeChange` (`ANIMATION_PRESET_INSTANCE` /
/// `KEYFRAME_TRACK` / `KEYFRAME`) into the side-table. Returns `true` when the
/// change was motion data — the caller then records the guid as structural
/// (not a scene node) and does NOT count it in the skip table.
pub(crate) fn collect_motion_change(
    pending: &mut PendingMotion,
    type_name: &str,
    guid: &str,
    change: &KiwiValue,
) -> bool {
    let parent_guid = change
        .get("parentIndex")
        .and_then(|p| p.get("guid"))
        .and_then(guid_key);
    match type_name {
        "ANIMATION_PRESET_INSTANCE" => {
            let name = change
                .get("name")
                .and_then(KiwiValue::as_str)
                .unwrap_or("Animation Preset")
                .to_owned();
            pending.presets.push((guid.to_owned(), name));
            true
        }
        "KEYFRAME_TRACK" => {
            let operation = match change.get("keyframeOperation").and_then(KiwiValue::as_str) {
                Some("SCALE") => KeyframeOperation::Scale,
                Some("OFFSET") => KeyframeOperation::Offset,
                // ASSUMPTION: an absent `keyframeOperation` defaults to the
                // enum's 0 member (SET). Every observed track carried SCALE.
                _ => KeyframeOperation::Set,
            };
            pending.tracks.push(PendingMotionTrack {
                guid: guid.to_owned(),
                parent_guid,
                operation,
            });
            true
        }
        "KEYFRAME" => {
            let (interpolation, easing) = read_keyframe_easing(change);
            pending.keyframes.push(PendingMotionKeyframe {
                guid: guid.to_owned(),
                parent_guid,
                time_us: change
                    .get("timelinePosition")
                    .and_then(KiwiValue::as_f64)
                    .map(|us| us as i64)
                    .unwrap_or(0),
                float_value: read_keyframe_float_value(change),
                interpolation,
                easing,
            });
            true
        }
        _ => false,
    }
}

/// `keyframeValue.value.floatValue`, gated on `valueType` being FLOAT (or
/// absent — FLOAT is the enum's 0/default member). Non-float kinds return
/// `None` and are counted as dropped by the resolver.
fn read_keyframe_float_value(change: &KiwiValue) -> Option<f64> {
    let keyframe_value = change.get("keyframeValue")?;
    let value_type = keyframe_value
        .get("valueType")
        .and_then(KiwiValue::as_str)
        .unwrap_or("FLOAT");
    if value_type != "FLOAT" {
        return None;
    }
    keyframe_value
        .get("value")
        .and_then(|v| v.get("floatValue"))
        .and_then(KiwiValue::as_f64)
        .filter(|v| v.is_finite())
}

/// Map a keyframe's `easingData` onto the model's
/// ([`Interpolation`], [`Easing`]) pair. `HOLD` is an interpolation mode in
/// our vocabulary, not a curve. The spring triples reuse the standard preset
/// approximations from `read_transition` (reactions.rs); `SPRING_PRESET_ONE/
/// TWO/THREE` get the quick/bouncy/slow triples in that order (ASSUMPTION:
/// unobserved — ordered to mirror Figma's quick/bouncy/slow preset lineup).
pub(crate) fn read_keyframe_easing(change: &KiwiValue) -> (Interpolation, Easing) {
    let Some(data) = change.get("easingData") else {
        // ASSUMPTION: every observed keyframe carried easingData; an absent
        // one keeps the model's default curve.
        return (Interpolation::Linear, Easing::default());
    };
    let easing_value = data.get("easingValue");
    let easing = match data.get("easingType").and_then(KiwiValue::as_str) {
        Some("HOLD") => return (Interpolation::Hold, Easing::Linear),
        Some("LINEAR") => Easing::Linear,
        Some("IN_CUBIC" | "IN_BACK_CUBIC" | "EASE_IN") => Easing::EaseIn,
        Some("OUT_CUBIC" | "OUT_BACK_CUBIC") => Easing::EaseOut,
        Some("INOUT_CUBIC" | "INOUT_BACK_CUBIC") => Easing::EaseInOut,
        Some("CUSTOM_CUBIC") => read_bezier_easing(easing_value),
        Some("SPRING" | "GENTLE_SPRING") => Easing::Spring {
            mass: 1.0,
            stiffness: 100.0,
            damping: 15.0,
        },
        Some("CUSTOM_SPRING") => read_spring_easing(easing_value),
        Some("SPRING_PRESET_ONE") => Easing::Spring {
            mass: 1.0,
            stiffness: 300.0,
            damping: 20.0,
        },
        Some("SPRING_PRESET_TWO") => Easing::Spring {
            mass: 1.0,
            stiffness: 600.0,
            damping: 15.0,
        },
        Some("SPRING_PRESET_THREE") => Easing::Spring {
            mass: 1.0,
            stiffness: 80.0,
            damping: 20.0,
        },
        _ => Easing::default(),
    };
    (Interpolation::Linear, easing)
}

/// `easingValue.bezierEasing: BezierHandles { p1x, p1y, p2x, p2y }` →
/// [`Easing::CubicBezier`]. Field names are schema-verified (message 622);
/// values were unobserved, so a missing handle falls back to the standard
/// ease-in-out control points.
fn read_bezier_easing(easing_value: Option<&KiwiValue>) -> Easing {
    let handles = easing_value.and_then(|v| v.get("bezierEasing"));
    let read = |key: &str, fallback: f32| {
        handles
            .and_then(|h| h.get(key))
            .and_then(KiwiValue::as_f64)
            .map_or(fallback, |v| v as f32)
    };
    Easing::CubicBezier {
        x1: read("p1x", 0.42),
        y1: read("p1y", 0.0),
        x2: read("p2x", 0.58),
        y2: read("p2y", 1.0),
    }
}

/// `easingValue.springEasing: SpringParams { stiffness, damping, mass }` →
/// [`Easing::Spring`]. Field names are schema-verified (message 521); a
/// missing parameter falls back to the gentle triple.
fn read_spring_easing(easing_value: Option<&KiwiValue>) -> Easing {
    let params = easing_value.and_then(|v| v.get("springEasing"));
    let read = |key: &str, fallback: f32| {
        params
            .and_then(|p| p.get(key))
            .and_then(KiwiValue::as_f64)
            .map_or(fallback, |v| v as f32)
    };
    Easing::Spring {
        mass: read("mass", 1.0),
        stiffness: read("stiffness", 100.0),
        damping: read("damping", 15.0),
    }
}

/// Collect a scene node's keyframe-track bindings (KEYFRAME expressions in its
/// `variableConsumptionMap` / `parameterConsumptionMap`) and any
/// `timelineDefinitions` durations it carries. Called from pass 1 for every
/// node that produced a scene node.
pub(crate) fn collect_motion_consumers(
    pending: &mut PendingMotion,
    node: NodeId,
    change: &KiwiValue,
) {
    // The fixture carries IDENTICAL entries in both maps; scanning both and
    // deduping on (track, node, field) keeps a file that authors only one of
    // them working without double-importing the other. A binding carries its
    // `node`, and this runs once per node, so a duplicate can only be among
    // the bindings THIS call pushed: comparing against those alone is the
    // same check without rescanning every earlier node's bindings per entry.
    let first_binding_of_this_node = pending.bindings.len();
    for map_name in ["variableConsumptionMap", "parameterConsumptionMap"] {
        let Some(entries) = change
            .get(map_name)
            .and_then(|m| m.get("entries"))
            .and_then(KiwiValue::as_array)
        else {
            continue;
        };
        for entry in entries {
            let Some(binding) = read_keyframe_binding(node, entry) else {
                continue;
            };
            let already_pushed = pending
                .bindings
                .get(first_binding_of_this_node..)
                .is_some_and(|own| own.contains(&binding));
            if !already_pushed {
                pending.bindings.push(binding);
            }
        }
    }

    if let Some(entries) = change
        .get("timelineDefinitions")
        .and_then(|m| m.get("entries"))
        .and_then(KiwiValue::as_array)
    {
        for entry in entries {
            let Some(timeline_guid) = entry.get("id").and_then(guid_key) else {
                continue;
            };
            let duration_us = entry
                .get("data")
                .and_then(|d| d.get("durationUs"))
                .and_then(KiwiValue::as_f64)
                .map(|us| us.max(0.0) as u64)
                .unwrap_or(0);
            pending
                .timeline_durations_us
                .entry(timeline_guid)
                .or_insert(duration_us);
        }
    }
}

/// Parse one `VariableDataMapEntry` into a [`PendingKeyframeBinding`] when it
/// is a KEYFRAME expression; `None` for every other entry kind (plain aliases
/// are `apply_bindings`' business).
fn read_keyframe_binding(node: NodeId, entry: &KiwiValue) -> Option<PendingKeyframeBinding> {
    let field = entry
        .get("variableField")
        .and_then(KiwiValue::as_str)?
        .to_owned();
    let expression = entry
        .get("variableData")
        .and_then(|d| d.get("value"))
        .and_then(|v| v.get("expressionValue"))?;
    if expression
        .get("expressionFunction")
        .and_then(KiwiValue::as_str)
        != Some("KEYFRAME")
    {
        return None;
    }
    let arguments = expression
        .get("expressionArguments")
        .and_then(KiwiValue::as_array)?;
    // ASSUMPTION: the observed argument order is [base FLOAT, track ref], but
    // both are matched by shape rather than position so a reordering still
    // resolves.
    let base = arguments.iter().find_map(|argument| {
        argument
            .get("value")
            .and_then(|v| v.get("floatValue"))
            .and_then(KiwiValue::as_f64)
    });
    let (track_guid, timeline_def_guid) = arguments.iter().find_map(read_track_parameter)?;
    Some(PendingKeyframeBinding {
        node,
        track_guid,
        field,
        base,
        timeline_def_guid,
    })
}

/// Extract `(keyframeTrackId guid, timelineDefId guid)` from an expression
/// argument's `keyframeTrackParameterValue`. Accepts both the observed
/// `ANIMATION_PRESET` parameter arm and the schema's `MANUAL` arm (unobserved
/// — same two ids per the schema). ASSUMPTION: only the first parameter with
/// a track id is used; the observed array always had exactly one.
fn read_track_parameter(argument: &KiwiValue) -> Option<(String, Option<String>)> {
    let parameters = argument
        .get("value")
        .and_then(|v| v.get("keyframeTrackParameterValue"))
        .and_then(|k| k.get("parameters"))
        .and_then(KiwiValue::as_array)?;
    parameters.iter().find_map(|parameter| {
        let arm = parameter.get("value")?;
        let inner = arm.get("animationPreset").or_else(|| arm.get("manual"))?;
        let track = inner
            .get("keyframeTrackId")
            .and_then(|t| t.get("guid"))
            .and_then(guid_key)?;
        let timeline = inner.get("timelineDefId").and_then(guid_key);
        Some((track, timeline))
    })
}

// =============================================================================
// Pass 4 — resolution onto doc.motion
// =============================================================================

/// Resolve the collected motion material onto [`Doc::motion`]: one
/// [`AnimationClip`] per `ANIMATION_PRESET_INSTANCE` (or per timeline for the
/// schema's not-yet-observed preset-less tracks), one [`AnimationTrack`] per
/// (track, bound node, channel), one [`Keyframe`] per `KEYFRAME`.
pub(crate) fn apply_motion(doc: &mut Doc, report: &mut MapReport, pending: &PendingMotion) {
    let mut keyframes_by_track: HashMap<&str, Vec<&PendingMotionKeyframe>> = HashMap::new();
    for keyframe in &pending.keyframes {
        let Some(parent) = keyframe.parent_guid.as_deref() else {
            // An orphan keyframe has no track to sample through.
            report.motion_keyframes_dropped += 1;
            continue;
        };
        keyframes_by_track.entry(parent).or_default().push(keyframe);
    }
    let mut bindings_by_track: HashMap<&str, Vec<&PendingKeyframeBinding>> = HashMap::new();
    for binding in &pending.bindings {
        bindings_by_track
            .entry(binding.track_guid.as_str())
            .or_default()
            .push(binding);
    }
    let preset_names: HashMap<&str, &str> = pending
        .presets
        .iter()
        .map(|(guid, name)| (guid.as_str(), name.as_str()))
        .collect();

    // Group tracks into clips, preserving stream order. The observed grouping
    // is by owning ANIMATION_PRESET_INSTANCE; a track without a preset parent
    // (the schema's MANUAL arm, unobserved) groups by its bindings'
    // timelineDefId, and failing that stands alone (ASSUMPTION).
    let mut clip_order: Vec<String> = Vec::new();
    let mut clip_tracks: HashMap<String, Vec<&PendingMotionTrack>> = HashMap::new();
    for track in &pending.tracks {
        let key = clip_key_for_track(track, &preset_names, &bindings_by_track);
        if !clip_tracks.contains_key(&key) {
            clip_order.push(key.clone());
        }
        clip_tracks.entry(key).or_default().push(track);
    }

    for clip_key in &clip_order {
        let Some(tracks) = clip_tracks.get(clip_key.as_str()) else {
            continue;
        };
        let clip_name = clip_name_for_key(clip_key, &preset_names);
        let clip_id =
            AnimationClipId::from_u128(stable_hash_u128(&format!("figmotion:clip:{clip_key}")));
        let mut clip = AnimationClip::new(clip_id, clip_name, 0);
        let mut duration_ms: u32 = 0;

        for track in tracks {
            let keyframes = keyframes_by_track
                .get(track.guid.as_str())
                .map_or(&[][..], Vec::as_slice);
            let Some(bindings) = bindings_by_track.get(track.guid.as_str()) else {
                // No consumer references this track: nothing to animate.
                report.motion_keyframes_dropped += keyframes.len();
                continue;
            };
            for binding in bindings {
                resolve_track_binding(
                    doc,
                    report,
                    &mut clip,
                    &mut duration_ms,
                    track,
                    binding,
                    keyframes,
                    &pending.timeline_durations_us,
                );
            }
        }

        if clip.tracks.is_empty() {
            continue;
        }
        clip.duration_ms = duration_ms;
        report.motion_tracks_imported += clip.tracks.len();
        report.motion_clips_imported += 1;
        doc.motion.clips.insert(clip.id, clip);
    }

    // Presets whose tracks all failed to resolve (or that own no tracks at
    // all) produced no clip: surface the loss.
    report.motion_presets_dropped += pending
        .presets
        .iter()
        .filter(|(guid, _)| {
            let id = AnimationClipId::from_u128(stable_hash_u128(&format!(
                "figmotion:clip:preset:{guid}"
            )));
            !doc.motion.clips.contains_key(&id)
        })
        .count();
}

/// Resolve one (track, binding) pair into an [`AnimationTrack`] on `clip`,
/// counting imported/dropped keyframes and stretching the clip duration.
#[allow(clippy::too_many_arguments)]
fn resolve_track_binding(
    doc: &Doc,
    report: &mut MapReport,
    clip: &mut AnimationClip,
    duration_ms: &mut u32,
    track: &PendingMotionTrack,
    binding: &PendingKeyframeBinding,
    keyframes: &[&PendingMotionKeyframe],
    timeline_durations_us: &HashMap<String, u64>,
) {
    let Some(property) = motion_property_for_field(&binding.field) else {
        // Channel kinds the model can't address (MOTION_SHEAR, the 3D and
        // vector-valued channels, non-motion fields riding a KEYFRAME
        // expression): loud, counted degradation.
        report.motion_keyframes_dropped += keyframes.len();
        return;
    };
    let Some(node) = doc.scene.get(binding.node) else {
        // The consumer was dropped (virtual instance content).
        report.motion_keyframes_dropped += keyframes.len();
        return;
    };
    // MOTION_* channels layer on the committed layout transform, so bake the
    // committed channel value into each keyframe (our evaluator SETS the
    // decomposed channel). A sheared/degenerate committed transform has no
    // channel decomposition — playback would ignore the override anyway, so
    // the keyframes are dropped and counted instead of imported dead.
    let committed = MotionTransform::decompose(node.transform);
    if requires_transform_baseline(property) && committed.is_none() {
        report.motion_keyframes_dropped += keyframes.len();
        return;
    }

    let track_seed = format!(
        "figmotion:track:{}:{:032x}:{}",
        track.guid,
        binding.node.to_u128(),
        binding.field
    );
    let target = MotionTarget::new(binding.node, property);
    let mut animation_track = AnimationTrack::new(
        AnimationTrackId::from_u128(stable_hash_u128(&track_seed)),
        target,
    );

    for keyframe in keyframes {
        let Some(raw_value) = keyframe.float_value else {
            report.motion_keyframes_dropped += 1;
            continue;
        };
        let channel = track.operation.compose(binding.base, raw_value);
        let value = bake_channel_value(property, committed.as_ref(), channel);
        let time_ms = us_to_ms(keyframe.time_us);
        let id = KeyframeId::from_u128(stable_hash_u128(&format!(
            "figmotion:kf:{}:{track_seed}",
            keyframe.guid
        )));
        animation_track.keyframes.insert(
            id,
            Keyframe {
                id,
                time_ms,
                value: ResolvedVarValue::Float { value },
                interpolation: keyframe.interpolation,
                easing: keyframe.easing,
            },
        );
        *duration_ms = (*duration_ms).max(time_ms);
        report.motion_keyframes_imported += 1;
    }

    if animation_track.keyframes.is_empty() {
        return;
    }
    if let Some(timeline_ms) = binding
        .timeline_def_guid
        .as_deref()
        .and_then(|guid| timeline_durations_us.get(guid))
        .filter(|&&us| us > 0)
        .map(|&us| us.div_ceil(1_000).min(u32::MAX as u64) as u32)
    {
        *duration_ms = (*duration_ms).max(timeline_ms);
    }
    clip.tracks.insert(animation_track.id, animation_track);
}

/// Map a Figma `VariableField` motion channel to a [`MotionProperty`].
/// Returns `None` for channels the model can't address — the caller counts
/// their keyframes in [`MapReport::motion_keyframes_dropped`].
fn motion_property_for_field(field: &str) -> Option<MotionProperty> {
    Some(match field {
        "MOTION_SCALE_X" => MotionProperty::ScaleX,
        "MOTION_SCALE_Y" => MotionProperty::ScaleY,
        // ASSUMPTION: only the scale channels were observed in the fixture.
        // Translation/rotation follow the same channel semantics (layered on
        // the committed transform); their value units are the document's
        // logical px for translation and — per Figma's authoring surfaces —
        // degrees for rotation (converted below).
        "MOTION_TRANSLATION_X" => MotionProperty::PositionX,
        "MOTION_TRANSLATION_Y" => MotionProperty::PositionY,
        "MOTION_ROTATION" => MotionProperty::Rotation,
        // Plain node opacity riding a KEYFRAME expression: absolute 0..1 in
        // both vocabularies.
        "OPACITY" => MotionProperty::bound(fanta_doc::binding::BoundProp::Opacity),
        _ => return None,
    })
}

/// Whether the property's keyframe values must be baked against the committed
/// transform's decomposed channels.
fn requires_transform_baseline(property: MotionProperty) -> bool {
    !matches!(property, MotionProperty::Bound { .. })
}

/// Bake a composed channel value into the absolute property value our
/// evaluator SETS: scale multiplies the committed scale, translation adds to
/// the committed translation, rotation adds to the committed rotation
/// (ASSUMPTION: Figma authors rotation in degrees — converted to the model's
/// radians), and bound properties (opacity) pass through absolute.
fn bake_channel_value(
    property: MotionProperty,
    committed: Option<&MotionTransform>,
    channel: f64,
) -> f64 {
    let Some(committed) = committed else {
        return channel;
    };
    match property {
        MotionProperty::ScaleX => committed.scale[0] * channel,
        MotionProperty::ScaleY => committed.scale[1] * channel,
        MotionProperty::PositionX => committed.position[0] + channel,
        MotionProperty::PositionY => committed.position[1] + channel,
        MotionProperty::Rotation => committed.rotation_radians + channel.to_radians(),
        MotionProperty::Bound { .. } => channel,
    }
}

/// Microseconds → whole milliseconds (round-to-nearest, clamped at 0).
fn us_to_ms(us: i64) -> u32 {
    let us = us.max(0) as u64;
    ((us + 500) / 1_000).min(u32::MAX as u64) as u32
}

/// The clip-grouping key for a track: its owning preset, else its bindings'
/// timeline, else itself.
fn clip_key_for_track(
    track: &PendingMotionTrack,
    preset_names: &HashMap<&str, &str>,
    bindings_by_track: &HashMap<&str, Vec<&PendingKeyframeBinding>>,
) -> String {
    if let Some(parent) = track.parent_guid.as_deref() {
        if preset_names.contains_key(parent) {
            return format!("preset:{parent}");
        }
    }
    if let Some(timeline) = bindings_by_track
        .get(track.guid.as_str())
        .and_then(|bindings| bindings.iter().find_map(|b| b.timeline_def_guid.clone()))
    {
        return format!("timeline:{timeline}");
    }
    format!("track:{}", track.guid)
}

/// Human-facing clip name for a grouping key: the preset instance's own name
/// (kept verbatim — e.g. `"motion.preset_name.scale"` — rather than a
/// prettified guess), or "Timeline" for the unobserved preset-less groupings.
fn clip_name_for_key(clip_key: &str, preset_names: &HashMap<&str, &str>) -> String {
    if let Some(guid) = clip_key.strip_prefix("preset:") {
        if let Some(name) = preset_names.get(guid) {
            return (*name).to_owned();
        }
    }
    "Timeline".to_owned()
}
