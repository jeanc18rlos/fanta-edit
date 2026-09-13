use std::cell::Cell;

use fanta_doc::{
    AnimationClip, AnimationClipId, AnimationTrack, AnimationTrackId, BoundProp, CanvasNode, Doc,
    Easing, Interpolation, Keyframe, KeyframeId, MotionProperty, MotionTarget, MotionTransform,
    NodeId, Operation, ResolvedVarValue,
};

use crate::timeline::{TimelineKeyframeSelection, TimelineProperty};

pub(crate) fn motion_property(property: TimelineProperty) -> MotionProperty {
    match property {
        TimelineProperty::PositionX => MotionProperty::PositionX,
        TimelineProperty::PositionY => MotionProperty::PositionY,
        TimelineProperty::Rotation => MotionProperty::Rotation,
        TimelineProperty::ScaleX => MotionProperty::ScaleX,
        TimelineProperty::ScaleY => MotionProperty::ScaleY,
        TimelineProperty::Opacity => MotionProperty::bound(BoundProp::Opacity),
        TimelineProperty::FillColor => MotionProperty::bound(BoundProp::FillColor { index: 0 }),
    }
}

pub(crate) fn motion_value(
    node: &CanvasNode,
    property: MotionProperty,
) -> Option<ResolvedVarValue> {
    match property {
        MotionProperty::Bound { prop } => prop.read_resolved(node),
        MotionProperty::PositionX
        | MotionProperty::PositionY
        | MotionProperty::Rotation
        | MotionProperty::ScaleX
        | MotionProperty::ScaleY => {
            let transform = MotionTransform::decompose(node.transform)?;
            let value = match property {
                MotionProperty::PositionX => transform.position[0],
                MotionProperty::PositionY => transform.position[1],
                MotionProperty::Rotation => transform.rotation_radians,
                MotionProperty::ScaleX => transform.scale[0],
                MotionProperty::ScaleY => transform.scale[1],
                MotionProperty::Bound { .. } => return None,
            };
            Some(ResolvedVarValue::Float { value })
        }
    }
}

fn source_motion_node(doc: &Doc, node: NodeId) -> Option<CanvasNode> {
    let mut node_value = doc.scene.get(node)?.clone();
    for (property, variable) in node_value.bindings.clone() {
        if let Some(value) = fanta_doc::resolve_bound_value(
            &doc.variables,
            &doc.scene,
            node,
            &doc.active_modes,
            variable,
        ) {
            property.apply_resolved(&mut node_value, value);
        }
    }
    Some(node_value)
}

pub(crate) fn evaluated_motion_value(
    doc: &Doc,
    clip: AnimationClipId,
    node: NodeId,
    property: TimelineProperty,
    playhead_ms: u32,
) -> Option<(u32, ResolvedVarValue)> {
    let clip_value = doc.motion.clip(clip)?;
    let time_ms = playhead_ms.min(clip_value.duration_ms);
    let property = motion_property(property);
    let target = MotionTarget::new(node, property);
    let source = source_motion_node(doc, node)?;
    let evaluation = clip_value.evaluate(time_ms);
    let value = evaluation
        .get(target)
        .cloned()
        .or_else(|| motion_value(&source, property))?;
    valid_motion_value(property, &value).then_some((time_ms, value))
}

#[cfg(test)]
fn effective_motion_track(clip: &AnimationClip, target: MotionTarget) -> Option<&AnimationTrack> {
    effective_motion_track_entry(clip, target).map(|(_, track)| track)
}

fn effective_motion_track_entry(
    clip: &AnimationClip,
    target: MotionTarget,
) -> Option<(AnimationTrackId, &AnimationTrack)> {
    clip.tracks
        .iter()
        .rev()
        .find_map(|(map_key, track)| (track.target == target).then_some((*map_key, track)))
}

fn effective_keyframe_at_time(
    track: &AnimationTrack,
    time_ms: u32,
) -> Option<(KeyframeId, &Keyframe)> {
    track
        .keyframes
        .iter()
        .filter(|(_, keyframe)| {
            keyframe.time_ms == time_ms && track.target.property.accepts_value(&keyframe.value)
        })
        .max_by_key(|(map_key, keyframe)| (keyframe.id, **map_key))
        .map(|(map_key, keyframe)| (*map_key, keyframe))
}

fn valid_motion_value(property: MotionProperty, value: &ResolvedVarValue) -> bool {
    if !property.accepts_value(value) {
        return false;
    }
    match (property, value) {
        (
            MotionProperty::Bound {
                prop: BoundProp::Opacity,
            },
            ResolvedVarValue::Float { value },
        ) => value.is_finite() && (0.0..=1.0).contains(value),
        (_, ResolvedVarValue::Float { value }) => value.is_finite(),
        _ => true,
    }
}

fn vacant_track_id(clip: &AnimationClip) -> AnimationTrackId {
    loop {
        let id = AnimationTrackId::new();
        if !clip.tracks.contains_key(&id) {
            return id;
        }
    }
}

fn vacant_keyframe_id(track: Option<&AnimationTrack>) -> KeyframeId {
    loop {
        let id = KeyframeId::new();
        if track.is_none_or(|track| !track.keyframes.contains_key(&id)) {
            return id;
        }
    }
}

fn upsert_track_keyframe(
    original: Option<&AnimationTrack>,
    track: AnimationTrackId,
    target: MotionTarget,
    keyframe: KeyframeId,
    time_ms: u32,
    value: ResolvedVarValue,
) -> AnimationTrack {
    let mut next = original
        .cloned()
        .unwrap_or_else(|| AnimationTrack::new(track, target));
    if let Some(existing) = next.keyframes.get_mut(&keyframe) {
        existing.value = value;
    } else {
        next.keyframes
            .insert(keyframe, Keyframe::new(keyframe, time_ms, value));
    }
    next
}

pub(crate) fn upsert_motion_keyframe_operation(
    doc: &Doc,
    clip: AnimationClipId,
    node: NodeId,
    property: TimelineProperty,
    playhead_ms: u32,
    value: ResolvedVarValue,
) -> Option<Operation> {
    let motion_property = motion_property(property);
    if !valid_motion_value(motion_property, &value) {
        return None;
    }
    evaluated_motion_value(doc, clip, node, property, playhead_ms)?;
    let clip_value = doc.motion.clip(clip)?;
    let time_ms = playhead_ms.min(clip_value.duration_ms);
    let target = MotionTarget::new(node, motion_property);
    let effective = effective_motion_track_entry(clip_value, target);
    let track = effective
        .map(|(map_key, _)| map_key)
        .unwrap_or_else(|| vacant_track_id(clip_value));
    let original = effective.map(|(_, track)| track.clone());
    let keyframe = original
        .as_ref()
        .and_then(|track| effective_keyframe_at_time(track, time_ms))
        .map(|(map_key, _)| map_key)
        .unwrap_or_else(|| vacant_keyframe_id(original.as_ref()));
    let next = upsert_track_keyframe(original.as_ref(), track, target, keyframe, time_ms, value);
    (original.as_ref() != Some(&next)).then(|| Operation::SetAnimationTrack {
        clip,
        track,
        old: original.map(Box::new),
        new: Some(Box::new(next)),
    })
}

#[derive(Debug, Clone)]
pub(crate) struct MotionPropertyEditSession {
    scene_instance: u64,
    clip: AnimationClipId,
    track: AnimationTrackId,
    target: MotionTarget,
    keyframe: KeyframeId,
    time_ms: u32,
    start_value: ResolvedVarValue,
    original: Option<AnimationTrack>,
    current: Option<AnimationTrack>,
    commit_allowed: Cell<bool>,
}

impl MotionPropertyEditSession {
    pub(crate) fn begin(
        doc: &Doc,
        clip: AnimationClipId,
        node: NodeId,
        property: TimelineProperty,
        playhead_ms: u32,
    ) -> Option<Self> {
        let (time_ms, start_value) =
            evaluated_motion_value(doc, clip, node, property, playhead_ms)?;
        let target = MotionTarget::new(node, motion_property(property));
        let clip_value = doc.motion.clip(clip)?;
        let effective = effective_motion_track_entry(clip_value, target);
        let track = effective
            .map(|(map_key, _)| map_key)
            .unwrap_or_else(|| vacant_track_id(clip_value));
        let original = effective.map(|(_, track)| track.clone());
        let keyframe = original
            .as_ref()
            .and_then(|track| effective_keyframe_at_time(track, time_ms))
            .map(|(map_key, _)| map_key)
            .unwrap_or_else(|| vacant_keyframe_id(original.as_ref()));
        Some(Self {
            scene_instance: doc.scene.instance_id(),
            clip,
            track,
            target,
            keyframe,
            time_ms,
            start_value,
            current: original.clone(),
            original,
            commit_allowed: Cell::new(true),
        })
    }

    fn live_preview_state_matches(&self, doc: &Doc) -> bool {
        if doc.scene.instance_id() != self.scene_instance || !doc.scene.contains(self.target.node) {
            return false;
        }
        let Some(clip) = doc.motion.clip(self.clip) else {
            return false;
        };
        let live = clip.tracks.get(&self.track);
        if live != self.current.as_ref() {
            return false;
        }
        effective_motion_track_entry(clip, self.target).map(|(map_key, _)| map_key)
            == self.current.as_ref().map(|_| self.track)
    }

    pub(crate) fn preview(&mut self, doc: &mut Doc, value: ResolvedVarValue) -> bool {
        if !self.commit_allowed.get() || !valid_motion_value(self.target.property, &value) {
            return false;
        }
        if !self.live_preview_state_matches(doc) {
            self.commit_allowed.set(false);
            return false;
        }
        let desired = if value == self.start_value {
            self.original.clone()
        } else {
            Some(upsert_track_keyframe(
                self.original.as_ref(),
                self.track,
                self.target,
                self.keyframe,
                self.time_ms,
                value,
            ))
        };
        if desired == self.current {
            return false;
        }
        let Some(clip) = doc.motion.clips.get_mut(&self.clip) else {
            self.commit_allowed.set(false);
            return false;
        };
        match desired.as_ref() {
            Some(track) => {
                clip.tracks.insert(self.track, track.clone());
            }
            None => {
                clip.tracks.remove(&self.track);
            }
        }
        self.current = desired;
        true
    }

    pub(crate) fn preview_original(&mut self, doc: &mut Doc) -> bool {
        let start_value = self.start_value.clone();
        self.preview(doc, start_value)
    }

    pub(crate) fn restore(&self, doc: &mut Doc) -> bool {
        if doc.scene.instance_id() != self.scene_instance {
            self.commit_allowed.set(false);
            return false;
        }
        let node_exists = doc.scene.contains(self.target.node);
        let Some(clip) = doc.motion.clips.get_mut(&self.clip) else {
            self.commit_allowed.set(false);
            return false;
        };
        if clip.tracks.get(&self.track) != self.current.as_ref() {
            self.commit_allowed.set(false);
            return false;
        }
        if !node_exists
            || effective_motion_track_entry(clip, self.target).map(|(map_key, _)| map_key)
                != self.current.as_ref().map(|_| self.track)
        {
            self.commit_allowed.set(false);
        }
        if self.current == self.original {
            return false;
        }
        match self.original.as_ref() {
            Some(track) => {
                clip.tracks.insert(self.track, track.clone());
            }
            None => {
                clip.tracks.remove(&self.track);
            }
        }
        true
    }

    pub(crate) fn operation(&self) -> Option<Operation> {
        (self.commit_allowed.get() && self.original != self.current).then(|| {
            Operation::SetAnimationTrack {
                clip: self.clip,
                track: self.track,
                old: self.original.clone().map(Box::new),
                new: self.current.clone().map(Box::new),
            }
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MotionKeyframeDragSession {
    clip: AnimationClipId,
    track: AnimationTrackId,
    target: MotionTarget,
    keyframe: KeyframeId,
    original: Keyframe,
    current: Keyframe,
    commit_allowed: Cell<bool>,
}

impl MotionKeyframeDragSession {
    pub(crate) fn begin(
        doc: &Doc,
        clip: AnimationClipId,
        selection: &TimelineKeyframeSelection,
    ) -> Option<Self> {
        let track_id = selection.track_id.parse::<AnimationTrackId>().ok()?;
        let keyframe_id = selection.keyframe_id.parse::<KeyframeId>().ok()?;
        let track = doc.motion.clip(clip)?.tracks.get(&track_id)?;
        let keyframe = track.keyframes.get(&keyframe_id)?.clone();
        Some(Self {
            clip,
            track: track_id,
            target: track.target,
            keyframe: keyframe_id,
            original: keyframe.clone(),
            current: keyframe,
            commit_allowed: Cell::new(true),
        })
    }

    pub(crate) fn matches(&self, selection: &TimelineKeyframeSelection) -> bool {
        selection.track_id.as_ref() == self.track.to_string()
            && selection.keyframe_id.as_ref() == self.keyframe.to_string()
    }

    pub(crate) fn preview(&mut self, doc: &mut Doc, time_us: i64) -> bool {
        let Some(duration_ms) = doc
            .motion
            .clips
            .get(&self.clip)
            .map(|clip| clip.duration_ms)
        else {
            self.commit_allowed.set(false);
            return false;
        };
        let time_ms = timeline_us_to_ms(time_us, duration_ms);
        self.preview_update(doc, |keyframe| keyframe.time_ms = time_ms)
    }

    pub(crate) fn preview_easing(&mut self, doc: &mut Doc, easing: Easing) -> bool {
        let Some(easing) = normalize_easing(easing) else {
            return false;
        };
        self.preview_update(doc, |keyframe| keyframe.easing = easing)
    }

    fn preview_update(&mut self, doc: &mut Doc, update: impl FnOnce(&mut Keyframe)) -> bool {
        if !self.commit_allowed.get() {
            return false;
        }
        let Some(clip) = doc.motion.clips.get_mut(&self.clip) else {
            self.commit_allowed.set(false);
            return false;
        };
        let Some(track) = clip.tracks.get_mut(&self.track) else {
            self.commit_allowed.set(false);
            return false;
        };
        if track.target != self.target {
            self.commit_allowed.set(false);
            return false;
        }
        let Some(keyframe) = track.keyframes.get(&self.keyframe) else {
            self.commit_allowed.set(false);
            return false;
        };
        if keyframe != &self.current {
            self.commit_allowed.set(false);
            return false;
        }
        let mut next = self.current.clone();
        update(&mut next);
        if next == self.current {
            return false;
        }
        self.current = next;
        track.keyframes.insert(self.keyframe, self.current.clone());
        true
    }

    pub(crate) fn restore(&self, doc: &mut Doc) -> bool {
        let Some(track) = doc
            .motion
            .clips
            .get_mut(&self.clip)
            .and_then(|clip| clip.tracks.get_mut(&self.track))
        else {
            self.commit_allowed.set(false);
            return false;
        };
        if track.target != self.target {
            self.commit_allowed.set(false);
            return false;
        }
        let Some(keyframe) = track.keyframes.get(&self.keyframe) else {
            self.commit_allowed.set(false);
            return false;
        };
        if self.current == self.original {
            return false;
        }
        if keyframe != &self.current {
            self.commit_allowed.set(false);
            return false;
        }
        track.keyframes.insert(self.keyframe, self.original.clone());
        true
    }

    pub(crate) fn operation(&self) -> Option<Operation> {
        (self.commit_allowed.get() && self.original != self.current).then(|| {
            Operation::SetKeyframe {
                clip: self.clip,
                track: self.track,
                target: self.target,
                keyframe: self.keyframe,
                old: Some(self.original.clone()),
                new: Some(self.current.clone()),
            }
        })
    }
}

pub(crate) fn set_keyframe_interpolation_operation(
    doc: &Doc,
    clip: AnimationClipId,
    selection: &TimelineKeyframeSelection,
    interpolation: Interpolation,
) -> Option<Operation> {
    edit_keyframe_operation(doc, clip, selection, |keyframe| {
        keyframe.interpolation = interpolation
    })
}

pub(crate) fn set_keyframe_easing_operation(
    doc: &Doc,
    clip: AnimationClipId,
    selection: &TimelineKeyframeSelection,
    easing: Easing,
) -> Option<Operation> {
    let easing = normalize_easing(easing)?;
    edit_keyframe_operation(doc, clip, selection, |keyframe| keyframe.easing = easing)
}

fn edit_keyframe_operation(
    doc: &Doc,
    clip: AnimationClipId,
    selection: &TimelineKeyframeSelection,
    edit: impl FnOnce(&mut Keyframe),
) -> Option<Operation> {
    let track_id = selection.track_id.parse::<AnimationTrackId>().ok()?;
    let keyframe_id = selection.keyframe_id.parse::<KeyframeId>().ok()?;
    let track = doc.motion.clip(clip)?.tracks.get(&track_id)?;
    let old = track.keyframes.get(&keyframe_id)?.clone();
    let mut new = old.clone();
    edit(&mut new);
    (new != old).then_some(Operation::SetKeyframe {
        clip,
        track: track_id,
        target: track.target,
        keyframe: keyframe_id,
        old: Some(old),
        new: Some(new),
    })
}

fn normalize_easing(easing: Easing) -> Option<Easing> {
    match easing {
        Easing::CubicBezier { x1, y1, x2, y2 } => {
            if ![x1, y1, x2, y2].into_iter().all(f32::is_finite) {
                return None;
            }
            Some(Easing::CubicBezier {
                x1: x1.clamp(0.0, 1.0),
                y1: y1.clamp(-10.0, 10.0),
                x2: x2.clamp(0.0, 1.0),
                y2: y2.clamp(-10.0, 10.0),
            })
        }
        easing => Some(easing),
    }
}

pub(crate) fn delete_keyframe_operation(
    doc: &Doc,
    clip: AnimationClipId,
    selection: &TimelineKeyframeSelection,
) -> Option<Operation> {
    let track_id = selection.track_id.parse::<AnimationTrackId>().ok()?;
    let keyframe_id = selection.keyframe_id.parse::<KeyframeId>().ok()?;
    let track = doc.motion.clip(clip)?.tracks.get(&track_id)?;
    let keyframe = track.keyframes.get(&keyframe_id)?.clone();
    Some(Operation::SetKeyframe {
        clip,
        track: track_id,
        target: track.target,
        keyframe: keyframe_id,
        old: Some(keyframe),
        new: None,
    })
}

pub(crate) fn rename_clip_operation(
    doc: &Doc,
    clip: AnimationClipId,
    name: &str,
) -> Option<Operation> {
    let clip = doc.motion.clip(clip)?;
    let name = name.trim();
    if name.is_empty() || name == clip.name {
        return None;
    }
    Some(Operation::SetAnimationClipName {
        id: clip.id,
        old: clip.name.clone(),
        new: name.to_owned(),
    })
}

pub(crate) fn set_clip_duration_operation(
    doc: &Doc,
    clip: AnimationClipId,
    duration_us: i64,
) -> Option<Operation> {
    let clip = doc.motion.clip(clip)?;
    let duration_ms = duration_us_to_ms(duration_us);
    if duration_ms == clip.duration_ms {
        return None;
    }
    Some(Operation::SetAnimationClipDuration {
        id: clip.id,
        old: clip.duration_ms,
        new: duration_ms,
    })
}

fn timeline_us_to_ms(time_us: i64, duration_ms: u32) -> u32 {
    let rounded_ms = time_us
        .max(0)
        .saturating_add(500)
        .div_euclid(1_000)
        .min(i64::from(u32::MAX)) as u32;
    rounded_ms.min(duration_ms)
}

fn duration_us_to_ms(duration_us: i64) -> u32 {
    duration_us
        .max(1)
        .saturating_add(500)
        .div_euclid(1_000)
        .clamp(1, i64::from(u32::MAX)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use fanta_doc::{
        Color, Mode, ModeId, NodeData, UnitInterval, VarValue, Variable, VariableCollection,
        VariableCollectionId, VariableId, VariableType, VectorNode,
    };

    fn motion_doc() -> (
        Doc,
        AnimationClipId,
        AnimationTrackId,
        KeyframeId,
        TimelineKeyframeSelection,
    ) {
        let mut doc = Doc::new();
        let clip_id = AnimationClipId::from_u128(1);
        let track_id = AnimationTrackId::from_u128(2);
        let keyframe_id = KeyframeId::from_u128(3);
        let target = MotionTarget::new(fanta_doc::NodeId::from_u128(4), MotionProperty::PositionX);
        let mut track = AnimationTrack::new(track_id, target);
        track.keyframes.insert(
            keyframe_id,
            Keyframe::new(keyframe_id, 250, ResolvedVarValue::Float { value: 10.0 }),
        );
        let mut clip = AnimationClip::new(clip_id, "Entrance", 1_000);
        clip.tracks.insert(track_id, track);
        doc.motion.clips.insert(clip_id, clip);
        let selection = TimelineKeyframeSelection {
            track_id: track_id.to_string().into(),
            keyframe_id: keyframe_id.to_string().into(),
        };
        (doc, clip_id, track_id, keyframe_id, selection)
    }

    fn property_doc() -> (Doc, AnimationClipId, NodeId) {
        let mut doc = Doc::new();
        let mut node = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            20.0,
            20.0,
            Color::rgb(12, 34, 56),
        )));
        node.transform = MotionTransform {
            position: [12.0, 34.0],
            rotation_radians: 0.25,
            scale: [-2.0, 3.0],
        }
        .recompose()
        .expect("representable motion transform");
        node.opacity = UnitInterval::new(0.4);
        let node_id = node.id;
        doc.scene.insert(node).expect("insert motion node");
        let clip_id = AnimationClipId::from_u128(100);
        doc.motion
            .clips
            .insert(clip_id, AnimationClip::new(clip_id, "Property edit", 1_000));
        (doc, clip_id, node_id)
    }

    fn float(value: &ResolvedVarValue) -> f64 {
        match value {
            ResolvedVarValue::Float { value } => *value,
            value => panic!("expected float, got {value:?}"),
        }
    }

    fn insert_track(
        doc: &mut Doc,
        clip: AnimationClipId,
        track_id: u128,
        node: NodeId,
        property: TimelineProperty,
        keyframes: impl IntoIterator<Item = (KeyframeId, Keyframe)>,
    ) -> AnimationTrackId {
        let track_id = AnimationTrackId::from_u128(track_id);
        let mut track =
            AnimationTrack::new(track_id, MotionTarget::new(node, motion_property(property)));
        track.keyframes.extend(keyframes);
        doc.motion
            .clips
            .get_mut(&clip)
            .expect("motion clip")
            .tracks
            .insert(track_id, track);
        track_id
    }

    #[test]
    fn drag_preview_is_transient_and_commit_builds_one_reversible_operation() {
        let (mut doc, clip_id, track_id, keyframe_id, selection) = motion_doc();
        let mut drag = MotionKeyframeDragSession::begin(&doc, clip_id, &selection)
            .expect("keyframe drag session");

        assert!(drag.preview(&mut doc, 700_000));
        assert_eq!(
            doc.motion.clips[&clip_id].tracks[&track_id].keyframes[&keyframe_id].time_ms,
            700
        );
        assert!(!doc.undo().expect("preview does not enter history"));

        let operation = drag.operation().expect("commit operation");
        doc.apply(operation).expect("commit drag");
        assert!(doc.undo().expect("undo drag"));
        assert_eq!(
            doc.motion.clips[&clip_id].tracks[&track_id].keyframes[&keyframe_id].time_ms,
            250
        );
    }

    #[test]
    fn dragging_clamps_to_the_clip_and_preserves_keyframe_metadata() {
        let (mut doc, clip_id, track_id, keyframe_id, selection) = motion_doc();
        let original = doc.motion.clips[&clip_id].tracks[&track_id].keyframes[&keyframe_id].clone();
        let mut drag = MotionKeyframeDragSession::begin(&doc, clip_id, &selection)
            .expect("keyframe drag session");

        assert!(drag.preview(&mut doc, i64::MAX));
        let dragged = &doc.motion.clips[&clip_id].tracks[&track_id].keyframes[&keyframe_id];
        assert_eq!(dragged.time_ms, 1_000);
        assert_eq!(dragged.value, original.value);
        assert_eq!(dragged.interpolation, original.interpolation);
        assert_eq!(dragged.easing, original.easing);
    }

    #[test]
    fn restore_only_rewinds_the_session_last_preview() {
        let (mut doc, clip_id, track_id, keyframe_id, selection) = motion_doc();
        let mut drag = MotionKeyframeDragSession::begin(&doc, clip_id, &selection)
            .expect("keyframe drag session");

        assert!(drag.preview(&mut doc, 700_000));
        assert!(drag.restore(&mut doc));
        assert_eq!(
            doc.motion.clips[&clip_id].tracks[&track_id].keyframes[&keyframe_id].time_ms,
            250
        );
        assert!(drag.operation().is_some());
    }

    #[test]
    fn restore_abandons_a_drag_after_the_live_keyframe_diverges() {
        let (mut doc, clip_id, track_id, keyframe_id, selection) = motion_doc();
        let mut drag = MotionKeyframeDragSession::begin(&doc, clip_id, &selection)
            .expect("keyframe drag session");

        assert!(drag.preview(&mut doc, 700_000));
        doc.motion
            .clips
            .get_mut(&clip_id)
            .expect("live clip")
            .tracks
            .get_mut(&track_id)
            .expect("live track")
            .keyframes
            .get_mut(&keyframe_id)
            .expect("live keyframe")
            .time_ms = 800;

        assert!(!drag.restore(&mut doc));
        assert_eq!(
            doc.motion.clips[&clip_id].tracks[&track_id].keyframes[&keyframe_id].time_ms,
            800
        );
        assert!(
            drag.operation().is_none(),
            "a divergent preview must not be committed over the newer keyframe"
        );
    }

    #[test]
    fn delete_rename_and_duration_use_existing_motion_operations() {
        let (doc, clip_id, track_id, keyframe_id, selection) = motion_doc();
        assert!(matches!(
            delete_keyframe_operation(&doc, clip_id, &selection),
            Some(Operation::SetKeyframe {
                track,
                keyframe,
                new: None,
                ..
            }) if track == track_id && keyframe == keyframe_id
        ));
        assert!(matches!(
            rename_clip_operation(&doc, clip_id, "Loop"),
            Some(Operation::SetAnimationClipName { old, new, .. })
                if old == "Entrance" && new == "Loop"
        ));
        assert!(matches!(
            set_clip_duration_operation(&doc, clip_id, 2_500_000),
            Some(Operation::SetAnimationClipDuration {
                old: 1_000,
                new: 2_500,
                ..
            })
        ));
    }

    #[test]
    fn easing_preview_clamps_points_and_commits_as_one_undo_step() {
        let (mut doc, clip_id, track_id, keyframe_id, selection) = motion_doc();
        let mut edit = MotionKeyframeDragSession::begin(&doc, clip_id, &selection)
            .expect("keyframe easing session");
        let requested = Easing::CubicBezier {
            x1: -2.0,
            y1: -20.0,
            x2: 4.0,
            y2: 20.0,
        };

        assert!(edit.preview_easing(&mut doc, requested));
        let expected = Easing::CubicBezier {
            x1: 0.0,
            y1: -10.0,
            x2: 1.0,
            y2: 10.0,
        };
        assert_eq!(
            doc.motion.clips[&clip_id].tracks[&track_id].keyframes[&keyframe_id].easing,
            expected
        );
        assert!(edit.restore(&mut doc));
        doc.apply(edit.operation().expect("one easing operation"))
            .expect("commit easing");
        assert_eq!(
            doc.motion.clips[&clip_id].tracks[&track_id].keyframes[&keyframe_id].easing,
            expected
        );
        assert!(doc.undo().expect("undo easing"));
        assert_eq!(
            doc.motion.clips[&clip_id].tracks[&track_id].keyframes[&keyframe_id].easing,
            Easing::EaseInOut
        );
        assert!(doc.redo().expect("redo easing"));
        assert_eq!(
            doc.motion.clips[&clip_id].tracks[&track_id].keyframes[&keyframe_id].easing,
            expected
        );
    }

    #[test]
    fn invalid_easing_does_not_poison_a_live_edit_session() {
        let (mut doc, clip_id, track_id, keyframe_id, selection) = motion_doc();
        let mut edit = MotionKeyframeDragSession::begin(&doc, clip_id, &selection)
            .expect("keyframe easing session");

        assert!(!edit.preview_easing(
            &mut doc,
            Easing::CubicBezier {
                x1: f32::NAN,
                y1: 0.0,
                x2: 1.0,
                y2: 1.0,
            }
        ));
        assert_eq!(
            doc.motion.clips[&clip_id].tracks[&track_id].keyframes[&keyframe_id].easing,
            Easing::EaseInOut
        );
        assert!(edit.preview_easing(&mut doc, Easing::EaseOut));
        assert!(edit.operation().is_some());
    }

    #[test]
    fn interpolation_and_preset_easing_build_reversible_keyframe_operations() {
        let (mut doc, clip_id, track_id, keyframe_id, selection) = motion_doc();
        let interpolation =
            set_keyframe_interpolation_operation(&doc, clip_id, &selection, Interpolation::Hold)
                .expect("interpolation operation");
        doc.apply(interpolation).expect("set interpolation");
        assert_eq!(
            doc.motion.clips[&clip_id].tracks[&track_id].keyframes[&keyframe_id].interpolation,
            Interpolation::Hold
        );
        assert!(doc.undo().expect("undo interpolation"));

        let easing = set_keyframe_easing_operation(&doc, clip_id, &selection, Easing::EaseIn)
            .expect("easing operation");
        doc.apply(easing).expect("set easing");
        assert_eq!(
            doc.motion.clips[&clip_id].tracks[&track_id].keyframes[&keyframe_id].easing,
            Easing::EaseIn
        );
        assert!(doc.undo().expect("undo easing"));
    }

    #[test]
    fn property_preview_creates_one_reversible_track_without_preview_history() {
        let (mut doc, clip_id, node_id) = property_doc();
        let mut edit = MotionPropertyEditSession::begin(
            &doc,
            clip_id,
            node_id,
            TimelineProperty::PositionX,
            250,
        )
        .expect("position edit session");

        assert_eq!(float(&edit.start_value), 12.0);
        assert_eq!(edit.clip, clip_id);
        assert_eq!(edit.target.node, node_id);
        assert_eq!(edit.target.property, MotionProperty::PositionX);
        assert_eq!(edit.time_ms, 250);
        assert!(edit.preview(&mut doc, ResolvedVarValue::Float { value: 42.0 }));
        assert_eq!(
            edit.current
                .as_ref()
                .and_then(|track| track.evaluate(edit.time_ms)),
            Some(ResolvedVarValue::Float { value: 42.0 })
        );
        assert_eq!(doc.history.undo_depth(), 0);

        let operation = edit.operation().expect("whole-track operation");
        let track_id = match &operation {
            Operation::SetAnimationTrack {
                track,
                old: None,
                new: Some(track_value),
                ..
            } => {
                assert_eq!(track_value.keyframes.len(), 1);
                assert_eq!(
                    track_value.evaluate(250),
                    Some(ResolvedVarValue::Float { value: 42.0 })
                );
                *track
            }
            operation => panic!("expected a new whole track, got {operation:?}"),
        };

        assert!(edit.restore(&mut doc));
        assert!(!doc.motion.clips[&clip_id].tracks.contains_key(&track_id));
        doc.apply(operation).expect("commit property edit");
        assert_eq!(doc.history.undo_depth(), 1);
        assert!(doc.motion.clips[&clip_id].tracks.contains_key(&track_id));
        assert!(doc.undo().expect("undo property edit"));
        assert!(!doc.motion.clips[&clip_id].tracks.contains_key(&track_id));
        assert!(doc.redo().expect("redo property edit"));
        assert!(doc.motion.clips[&clip_id].tracks.contains_key(&track_id));
    }

    #[test]
    fn whole_track_upsert_preserves_exact_time_identity_metadata_and_siblings() {
        let (mut doc, clip_id, node_id) = property_doc();
        let exact_map_key = KeyframeId::from_u128(201);
        let embedded_id = KeyframeId::from_u128(901);
        let sibling_id = KeyframeId::from_u128(202);
        let easing = Easing::CubicBezier {
            x1: 0.2,
            y1: -0.3,
            x2: 0.8,
            y2: 1.4,
        };
        let exact = Keyframe {
            id: embedded_id,
            time_ms: 400,
            value: ResolvedVarValue::Float { value: 10.0 },
            interpolation: Interpolation::Hold,
            easing,
        };
        let sibling = Keyframe::new(sibling_id, 800, ResolvedVarValue::Float { value: 80.0 });
        let track_id = insert_track(
            &mut doc,
            clip_id,
            200,
            node_id,
            TimelineProperty::PositionX,
            [(exact_map_key, exact), (sibling_id, sibling.clone())],
        );

        let operation = upsert_motion_keyframe_operation(
            &doc,
            clip_id,
            node_id,
            TimelineProperty::PositionX,
            400,
            ResolvedVarValue::Float { value: 22.0 },
        )
        .expect("replace exact keyframe");
        let next = match &operation {
            Operation::SetAnimationTrack {
                track,
                old: Some(old),
                new: Some(new),
                ..
            } => {
                assert_eq!(*track, track_id);
                assert_eq!(old.as_ref(), &doc.motion.clips[&clip_id].tracks[&track_id]);
                new.as_ref()
            }
            operation => panic!("expected existing whole-track replacement, got {operation:?}"),
        };
        let replaced = &next.keyframes[&exact_map_key];
        assert_eq!(replaced.id, embedded_id);
        assert_eq!(replaced.time_ms, 400);
        assert_eq!(replaced.interpolation, Interpolation::Hold);
        assert_eq!(replaced.easing, easing);
        assert_eq!(replaced.value, ResolvedVarValue::Float { value: 22.0 });
        assert_eq!(next.keyframes[&sibling_id], sibling);
    }

    #[test]
    fn property_edit_uses_effective_track_and_effective_valid_duplicate_keyframe() {
        let (mut doc, clip_id, node_id) = property_doc();
        let low_key = KeyframeId::from_u128(1);
        let low_track = insert_track(
            &mut doc,
            clip_id,
            10,
            node_id,
            TimelineProperty::PositionX,
            [(
                low_key,
                Keyframe::new(low_key, 500, ResolvedVarValue::Float { value: 1.0 }),
            )],
        );
        let winning_map_key = KeyframeId::from_u128(301);
        let losing_map_key = KeyframeId::from_u128(302);
        let wrong_type_map_key = KeyframeId::from_u128(303);
        let winning = Keyframe::new(
            KeyframeId::from_u128(500),
            500,
            ResolvedVarValue::Float { value: 5.0 },
        );
        let losing = Keyframe::new(
            KeyframeId::from_u128(400),
            500,
            ResolvedVarValue::Float { value: 4.0 },
        );
        let wrong_type = Keyframe::new(
            KeyframeId::from_u128(600),
            500,
            ResolvedVarValue::Color {
                value: Color::WHITE,
            },
        );
        let high_track = insert_track(
            &mut doc,
            clip_id,
            20,
            node_id,
            TimelineProperty::PositionX,
            [
                (winning_map_key, winning.clone()),
                (losing_map_key, losing.clone()),
                (wrong_type_map_key, wrong_type.clone()),
            ],
        );
        let target = MotionTarget::new(node_id, MotionProperty::PositionX);
        assert_eq!(
            effective_motion_track(&doc.motion.clips[&clip_id], target).map(|track| track.id),
            Some(high_track)
        );

        let mut edit = MotionPropertyEditSession::begin(
            &doc,
            clip_id,
            node_id,
            TimelineProperty::PositionX,
            500,
        )
        .expect("duplicate-safe edit session");
        assert_eq!(float(&edit.start_value), 5.0);
        assert!(edit.preview(&mut doc, ResolvedVarValue::Float { value: 9.0 }));

        let clip = &doc.motion.clips[&clip_id];
        assert_eq!(
            clip.tracks[&low_track].keyframes[&low_key].value,
            ResolvedVarValue::Float { value: 1.0 }
        );
        let high = &clip.tracks[&high_track];
        assert_eq!(
            high.keyframes[&winning_map_key].value,
            ResolvedVarValue::Float { value: 9.0 }
        );
        assert_eq!(high.keyframes[&winning_map_key].id, winning.id);
        assert_eq!(high.keyframes[&losing_map_key], losing);
        assert_eq!(high.keyframes[&wrong_type_map_key], wrong_type);
    }

    #[test]
    fn away_and_back_restores_the_exact_original_track_or_absence() {
        let (mut doc, clip_id, node_id) = property_doc();
        let first_id = KeyframeId::from_u128(401);
        let last_id = KeyframeId::from_u128(402);
        let mut first = Keyframe::new(first_id, 0, ResolvedVarValue::Float { value: 10.0 });
        first.easing = Easing::Linear;
        let track_id = insert_track(
            &mut doc,
            clip_id,
            400,
            node_id,
            TimelineProperty::PositionX,
            [
                (first_id, first),
                (
                    last_id,
                    Keyframe::new(last_id, 1_000, ResolvedVarValue::Float { value: 20.0 }),
                ),
            ],
        );
        let original = doc.motion.clips[&clip_id].tracks[&track_id].clone();
        let mut edit = MotionPropertyEditSession::begin(
            &doc,
            clip_id,
            node_id,
            TimelineProperty::PositionX,
            500,
        )
        .expect("interpolated property session");
        assert_eq!(float(&edit.start_value), 15.0);
        assert!(edit.preview(&mut doc, ResolvedVarValue::Float { value: 30.0 }));
        assert_eq!(
            doc.motion.clips[&clip_id].tracks[&track_id].keyframes.len(),
            3
        );
        assert!(edit.preview(&mut doc, ResolvedVarValue::Float { value: 15.0 }));
        assert_eq!(doc.motion.clips[&clip_id].tracks[&track_id], original);
        assert!(edit.operation().is_none());

        let mut new_track_edit = MotionPropertyEditSession::begin(
            &doc,
            clip_id,
            node_id,
            TimelineProperty::PositionY,
            500,
        )
        .expect("untracked property session");
        assert!(new_track_edit.preview(&mut doc, ResolvedVarValue::Float { value: 50.0 }));
        assert!(new_track_edit.preview(&mut doc, ResolvedVarValue::Float { value: 34.0 }));
        assert!(
            effective_motion_track(
                &doc.motion.clips[&clip_id],
                MotionTarget::new(node_id, MotionProperty::PositionY)
            )
            .is_none()
        );
        assert!(new_track_edit.operation().is_none());
    }

    #[test]
    fn property_edit_rejects_invalid_values_and_never_overwrites_stale_state() {
        let (mut doc, clip_id, node_id) = property_doc();
        let mut edit = MotionPropertyEditSession::begin(
            &doc,
            clip_id,
            node_id,
            TimelineProperty::PositionX,
            250,
        )
        .expect("position edit session");
        assert!(!edit.preview(
            &mut doc,
            ResolvedVarValue::Color {
                value: Color::BLACK,
            }
        ));
        assert!(!edit.preview(&mut doc, ResolvedVarValue::Float { value: f64::NAN }));
        assert!(!edit.preview(
            &mut doc,
            ResolvedVarValue::Float {
                value: f64::INFINITY,
            }
        ));
        assert!(edit.operation().is_none());
        assert!(edit.preview(&mut doc, ResolvedVarValue::Float { value: 20.0 }));
        let target = MotionTarget::new(node_id, MotionProperty::PositionX);
        let track_id = effective_motion_track(&doc.motion.clips[&clip_id], target)
            .expect("preview track")
            .id;
        doc.motion
            .clips
            .get_mut(&clip_id)
            .expect("clip")
            .tracks
            .get_mut(&track_id)
            .expect("track")
            .keyframes
            .values_mut()
            .next()
            .expect("keyframe")
            .value = ResolvedVarValue::Float { value: 99.0 };
        assert!(!edit.preview(&mut doc, ResolvedVarValue::Float { value: 30.0 }));
        assert!(edit.operation().is_none());
        assert!(!edit.restore(&mut doc));
        assert_eq!(
            doc.motion.clips[&clip_id].tracks[&track_id]
                .evaluate(250)
                .expect("stale value"),
            ResolvedVarValue::Float { value: 99.0 }
        );

        let (mut opacity_doc, opacity_clip, opacity_node) = property_doc();
        let mut opacity_edit = MotionPropertyEditSession::begin(
            &opacity_doc,
            opacity_clip,
            opacity_node,
            TimelineProperty::Opacity,
            0,
        )
        .expect("opacity session");
        assert!(!opacity_edit.preview(&mut opacity_doc, ResolvedVarValue::Float { value: -0.1 }));
        assert!(!opacity_edit.preview(&mut opacity_doc, ResolvedVarValue::Float { value: 1.1 }));
        assert!(opacity_edit.preview(&mut opacity_doc, ResolvedVarValue::Float { value: 0.5 }));

        let mut fill_edit = MotionPropertyEditSession::begin(
            &opacity_doc,
            opacity_clip,
            opacity_node,
            TimelineProperty::FillColor,
            0,
        )
        .expect("fill session");
        assert!(!fill_edit.preview(&mut opacity_doc, ResolvedVarValue::Float { value: 0.5 }));
    }

    #[test]
    fn stale_scene_fails_closed_and_deleted_node_preview_is_still_restored() {
        let (mut doc, clip_id, node_id) = property_doc();
        let mut stale = MotionPropertyEditSession::begin(
            &doc,
            clip_id,
            node_id,
            TimelineProperty::PositionX,
            0,
        )
        .expect("position session");
        let mut replacement = doc.clone();
        assert!(!stale.preview(&mut replacement, ResolvedVarValue::Float { value: 40.0 }));
        assert!(stale.operation().is_none());

        let mut deleted = MotionPropertyEditSession::begin(
            &doc,
            clip_id,
            node_id,
            TimelineProperty::PositionY,
            0,
        )
        .expect("position session");
        assert!(deleted.preview(&mut doc, ResolvedVarValue::Float { value: 80.0 }));
        doc.scene.remove(node_id).expect("remove edited node");
        assert!(deleted.restore(&mut doc));
        assert!(deleted.operation().is_none());
        assert!(
            effective_motion_track(
                &doc.motion.clips[&clip_id],
                MotionTarget::new(node_id, MotionProperty::PositionY)
            )
            .is_none()
        );
    }

    #[test]
    fn evaluated_projection_maps_and_reads_all_seven_properties_at_clamped_time() {
        let (mut doc, clip_id, node_id) = property_doc();
        let cases = [
            (
                TimelineProperty::PositionX,
                MotionProperty::PositionX,
                ResolvedVarValue::Float { value: 100.0 },
            ),
            (
                TimelineProperty::PositionY,
                MotionProperty::PositionY,
                ResolvedVarValue::Float { value: 200.0 },
            ),
            (
                TimelineProperty::Rotation,
                MotionProperty::Rotation,
                ResolvedVarValue::Float { value: 7.5 },
            ),
            (
                TimelineProperty::ScaleX,
                MotionProperty::ScaleX,
                ResolvedVarValue::Float { value: 0.0 },
            ),
            (
                TimelineProperty::ScaleY,
                MotionProperty::ScaleY,
                ResolvedVarValue::Float { value: 5.0 },
            ),
            (
                TimelineProperty::Opacity,
                MotionProperty::bound(BoundProp::Opacity),
                ResolvedVarValue::Float { value: 0.25 },
            ),
            (
                TimelineProperty::FillColor,
                MotionProperty::bound(BoundProp::FillColor { index: 0 }),
                ResolvedVarValue::Color {
                    value: Color::rgb(220, 30, 40),
                },
            ),
        ];
        for (position, (timeline_property, motion_property_value, value)) in
            cases.iter().enumerate()
        {
            assert_eq!(motion_property(*timeline_property), *motion_property_value);
            let keyframe_id = KeyframeId::from_u128(700 + position as u128);
            insert_track(
                &mut doc,
                clip_id,
                700 + position as u128,
                node_id,
                *timeline_property,
                [(
                    keyframe_id,
                    Keyframe::new(keyframe_id, 1_000, value.clone()),
                )],
            );
        }

        for (timeline_property, _, expected) in cases {
            let (time_ms, actual) =
                evaluated_motion_value(&doc, clip_id, node_id, timeline_property, 5_000)
                    .expect("evaluated property");
            assert_eq!(time_ms, 1_000);
            match (&actual, &expected) {
                (
                    ResolvedVarValue::Float { value: actual },
                    ResolvedVarValue::Float { value: expected },
                ) => assert!((actual - expected).abs() < 1e-12),
                _ => assert_eq!(&actual, &expected),
            }
        }
    }

    #[test]
    fn evaluated_projection_resolves_bound_values_before_motion_overrides() {
        let (mut doc, clip_id, node_id) = property_doc();
        let collection_id = VariableCollectionId::from_u128(800);
        let mode_id = ModeId::from_u128(801);
        let opacity_id = VariableId::from_u128(802);
        let fill_id = VariableId::from_u128(803);
        doc.variables.collections.insert(
            collection_id,
            VariableCollection {
                id: collection_id,
                name: "Theme".to_owned(),
                modes: vec![Mode {
                    id: mode_id,
                    name: "Default".to_owned(),
                }],
                default_mode: mode_id,
                variable_order: vec![opacity_id, fill_id],
            },
        );
        doc.variables.variables.insert(
            opacity_id,
            Variable {
                id: opacity_id,
                collection: collection_id,
                name: "Opacity".to_owned(),
                ty: VariableType::Float,
                values_by_mode: BTreeMap::from([(mode_id, VarValue::Float { value: 0.35 })]),
                scopes: Vec::new(),
            },
        );
        doc.variables.variables.insert(
            fill_id,
            Variable {
                id: fill_id,
                collection: collection_id,
                name: "Fill".to_owned(),
                ty: VariableType::Color,
                values_by_mode: BTreeMap::from([(
                    mode_id,
                    VarValue::Color {
                        value: Color::rgb(1, 2, 3),
                    },
                )]),
                scopes: Vec::new(),
            },
        );
        let node = doc.scene.get_mut(node_id).expect("bound node");
        node.bindings.insert(BoundProp::Opacity, opacity_id);
        node.bindings
            .insert(BoundProp::FillColor { index: 0 }, fill_id);

        let (time_ms, opacity) =
            evaluated_motion_value(&doc, clip_id, node_id, TimelineProperty::Opacity, 0)
                .expect("resolved opacity");
        assert_eq!(time_ms, 0);
        assert!((float(&opacity) - 0.35).abs() < 1e-6);
        assert_eq!(
            evaluated_motion_value(&doc, clip_id, node_id, TimelineProperty::FillColor, 0),
            Some((
                0,
                ResolvedVarValue::Color {
                    value: Color::rgb(1, 2, 3),
                }
            ))
        );

        let keyframe_id = KeyframeId::from_u128(804);
        insert_track(
            &mut doc,
            clip_id,
            804,
            node_id,
            TimelineProperty::Opacity,
            [(
                keyframe_id,
                Keyframe::new(keyframe_id, 0, ResolvedVarValue::Float { value: 0.8 }),
            )],
        );
        assert_eq!(
            evaluated_motion_value(&doc, clip_id, node_id, TimelineProperty::Opacity, 0),
            Some((0, ResolvedVarValue::Float { value: 0.8 }))
        );
    }
}
