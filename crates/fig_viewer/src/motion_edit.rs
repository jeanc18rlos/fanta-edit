use std::cell::Cell;

use fanta_doc::{
    AnimationClipId, AnimationTrackId, Doc, Keyframe, KeyframeId, MotionTarget, Operation,
};

use crate::timeline::TimelineKeyframeSelection;

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
        if !self.commit_allowed.get() {
            return false;
        }
        let Some(clip) = doc.motion.clips.get_mut(&self.clip) else {
            self.commit_allowed.set(false);
            return false;
        };
        let time_ms = timeline_us_to_ms(time_us, clip.duration_ms);
        if self.current.time_ms == time_ms {
            return false;
        }
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
        self.current.time_ms = time_ms;
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
    use fanta_doc::{AnimationClip, AnimationTrack, MotionProperty, ResolvedVarValue};

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
}
