use super::*;
use fanta_gpui::timeline::{
    self as shared, Timeline, TimelineAction as Action, TimelineEasing, TimelinePlayback,
};
use std::collections::{BTreeMap, HashMap};

pub(super) struct TimelineAdapter {
    pub panel: Entity<Timeline>,
    expanded: HashMap<String, bool>,
    selected: Vec<SharedString>,
    zoom: f32,
    height: u16,
    unit: shared::TimelineTimeUnit,
    snapping: bool,
    _subscription: Subscription,
}

pub(super) fn shared_easing(easing: Easing, interpolation: Interpolation) -> TimelineEasing {
    if interpolation == Interpolation::Hold {
        return TimelineEasing::Hold;
    }
    match easing {
        Easing::Linear => TimelineEasing::Linear,
        Easing::EaseIn => TimelineEasing::EaseIn,
        Easing::EaseOut => TimelineEasing::EaseOut,
        Easing::EaseInOut => TimelineEasing::EaseInOut,
        Easing::CubicBezier { x1, y1, x2, y2 } => TimelineEasing::CubicBezier([x1, y1, x2, y2]),
        Easing::Spring {
            mass,
            stiffness,
            damping,
        } => TimelineEasing::Spring {
            bounce: (1. - damping / (2. * (mass * stiffness).sqrt())).clamp(0., 1.),
        },
    }
}
pub(super) fn engine_easing(easing: &TimelineEasing) -> (Easing, Interpolation) {
    let curve = |[x1, y1, x2, y2]: [f32; 4]| Easing::CubicBezier { x1, y1, x2, y2 };
    let easing = match easing {
        TimelineEasing::Linear => Easing::Linear,
        TimelineEasing::EaseIn => Easing::EaseIn,
        TimelineEasing::EaseOut => Easing::EaseOut,
        TimelineEasing::EaseInOut => Easing::EaseInOut,
        TimelineEasing::Hold => return (Easing::Linear, Interpolation::Hold),
        TimelineEasing::CubicBezier(values) => curve(*values),
        TimelineEasing::EaseInBack => curve([0.36, 0., 0.66, -0.56]),
        TimelineEasing::EaseOutBack => curve([0.34, 1.56, 0.64, 1.]),
        TimelineEasing::EaseInOutBack => curve([0.68, -0.6, 0.32, 1.6]),
        TimelineEasing::Gentle => Easing::Spring {
            mass: 1.,
            stiffness: 100.,
            damping: 15.,
        },
        TimelineEasing::Quick => Easing::Spring {
            mass: 1.,
            stiffness: 300.,
            damping: 20.,
        },
        TimelineEasing::Bouncy => Easing::Spring {
            mass: 1.,
            stiffness: 180.,
            damping: 10.,
        },
        TimelineEasing::Slow => Easing::Spring {
            mass: 2.,
            stiffness: 100.,
            damping: 20.,
        },
        TimelineEasing::Spring { bounce } => Easing::Spring {
            mass: 1.,
            stiffness: 100.,
            damping: (20. * (1. - bounce.clamp(0., 1.))).max(0.1),
        },
    };
    (easing, Interpolation::Linear)
}

pub(super) fn property_name(property: MotionProperty) -> &'static str {
    match property {
        MotionProperty::PositionX => "Position X",
        MotionProperty::PositionY => "Position Y",
        MotionProperty::Rotation => "Rotation",
        MotionProperty::ScaleX => "Scale X",
        MotionProperty::ScaleY => "Scale Y",
        MotionProperty::Bound {
            prop: BoundProp::Opacity,
        } => "Opacity",
        MotionProperty::Bound {
            prop: BoundProp::FillColor { .. },
        } => "Fill color",
        MotionProperty::Bound {
            prop: BoundProp::ClipWidth,
        } => "Width",
        MotionProperty::Bound {
            prop: BoundProp::ClipHeight,
        } => "Height",
        MotionProperty::Bound { .. } => "Property",
    }
}

impl FigView {
    pub(super) fn refresh_shared_timeline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.gpui_design.is_none() {
            return;
        }
        if self.gpui_timeline.is_none() {
            let panel = cx.new(|cx| Timeline::new("editor-timeline", Default::default(), cx));
            panel.update(cx, |panel, cx| {
                panel.set_auto_keyframe_available(false, cx);
                panel.set_comments_available(false, cx);
            });
            let subscription = cx.subscribe_in(&panel, window, Self::handle_shared_timeline_action);
            self.gpui_timeline = Some(TimelineAdapter {
                panel,
                expanded: HashMap::new(),
                selected: vec![],
                zoom: 1.,
                height: 300,
                unit: shared::TimelineTimeUnit::Seconds,
                snapping: true,
                _subscription: subscription,
            });
        }
        let Some(adapter) = &mut self.gpui_timeline else {
            return;
        };
        let item = self.item.read(cx);
        let Some(document) = item.document() else {
            adapter
                .panel
                .update(cx, |panel, cx| panel.set_transport_available(false, cx));
            return;
        };
        let transport_available = self
            .active_motion_clip
            .and_then(|id| document.doc.motion.clip(id))
            .is_some();
        let clock = self.timeline_shell.read(cx);
        let mut data = shared::TimelineViewData {
            current_time_ms: (clock.playhead_us().max(0) / 1000) as u32,
            playing: clock.is_playing(),
            looping: clock.loop_playback_enabled(),
            playback: if clock.ping_pong_playback_enabled() {
                TimelinePlayback::PingPong
            } else if clock.loop_playback_enabled() {
                TimelinePlayback::Loop
            } else {
                TimelinePlayback::Once
            },
            read_only: !item.is_editable(),
            zoom: adapter.zoom,
            height: adapter.height,
            time_unit: adapter.unit,
            snapping: adapter.snapping,
            selected_keyframes: adapter.selected.clone(),
            ..Default::default()
        };
        if super::properties_inspector::motion_preset_available(&document.doc, item.is_editable()) {
            data.presets = crate::gpui_adapters::toolbar::motion_animation_styles()
                .into_iter()
                .map(|name| shared::TimelinePreset {
                    id: name.clone(),
                    name,
                })
                .collect();
        }
        if let Some(clip) = self
            .active_motion_clip
            .and_then(|id| document.doc.motion.clip(id))
        {
            data.duration_ms = clip.duration_ms;
            let mut tracks = BTreeMap::<NodeId, shared::TimelineTrack>::new();
            for track in clip.tracks.values() {
                let Some(node) = document.doc.scene.get(track.target.node) else {
                    continue;
                };
                let row = tracks.entry(node.id).or_insert_with(|| {
                    let mut row =
                        shared::TimelineTrack::new(node.id.to_string(), node.name.clone());
                    row.expanded = adapter
                        .expanded
                        .get(&node.id.to_string())
                        .copied()
                        .unwrap_or(true);
                    row.selected = document.doc.selection.contains(node.id);
                    row.visible = !node.flags.contains(fanta_doc::NodeFlags::HIDDEN);
                    row.locked = node.flags.contains(fanta_doc::NodeFlags::LOCKED);
                    row
                });
                let mut keyframes = track
                    .keyframes
                    .values()
                    .map(|key| shared::TimelineKeyframe {
                        id: key.id.to_string().into(),
                        time_ms: key.time_ms,
                        value: match &key.value {
                            ResolvedVarValue::Float { value } => format!("{value:.2}").into(),
                            _ => format!("{:?}", key.value).into(),
                        },
                        easing: shared_easing(key.easing, key.interpolation),
                    })
                    .collect::<Vec<_>>();
                keyframes.sort_by_key(|key| key.time_ms);
                row.properties.push(shared::TimelineProperty {
                    id: track.id.to_string().into(),
                    name: property_name(track.target.property).into(),
                    value: keyframes
                        .first()
                        .map(|key| key.value.clone())
                        .unwrap_or_default(),
                    keyframes,
                });
            }
            data.tracks = tracks.into_values().collect();
            data.selected_keyframes.retain(|id| {
                data.tracks
                    .iter()
                    .flat_map(|track| &track.properties)
                    .flat_map(|property| &property.keyframes)
                    .any(|key| key.id == *id)
            });
        }
        if !data.presets.is_empty()
            && let Some(selected) = super::single_selection(&document.doc)
            && let Some(node) = document.doc.scene.get(selected)
            && !data
                .tracks
                .iter()
                .any(|track| track.id == selected.to_string())
        {
            let mut track = shared::TimelineTrack::new(selected.to_string(), node.name.clone());
            track.selected = true;
            track.visible = !node.flags.contains(fanta_doc::NodeFlags::HIDDEN);
            track.locked = node.flags.contains(fanta_doc::NodeFlags::LOCKED);
            data.tracks.push(track);
        }
        adapter.panel.update(cx, |panel, cx| {
            panel.set_transport_available(transport_available, cx)
        });
        if adapter.panel.read(cx).view_data() != &data {
            adapter
                .panel
                .update(cx, |panel, cx| panel.set_view_data(data, cx));
        }
    }

    pub(super) fn apply_inspector_operations(
        &mut self,
        operations: Vec<Operation>,
        label: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if operations.is_empty() || !self.is_editable(cx) {
            return;
        }
        self.finish_panel_edits(cx);
        let mut transaction = Transaction::new(label);
        transaction.ops = operations;
        let result = self.item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                match document.doc.apply_transaction(transaction) {
                    Ok(()) => (Ok(()), DocChange::Content),
                    Err(error) => (Err(error), DocChange::None),
                }
            })
        });
        if let Some(Err(error)) = result {
            show_canvas_notice(format!("{label} failed: {error:#}"), window, cx);
        }
    }

    pub(super) fn handle_shared_timeline_action(
        &mut self,
        _: &Entity<Timeline>,
        action: &Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            Action::PlayStateChangeRequested { playing } => { self.timeline_shell.update(cx, |clock, cx| clock.set_playing(*playing, cx)); }
            Action::LoopChangeRequested { looping } => { self.timeline_shell.update(cx, |clock, cx| clock.set_loop_playback(*looping, cx)); }
            Action::PlaybackChangeRequested { playback } => {
                if *playback == TimelinePlayback::PingPong { self.timeline_shell.update(cx, |clock, cx| clock.set_ping_pong_playback(cx)); }
                else { self.timeline_shell.update(cx, |clock, cx| clock.set_loop_playback(*playback == TimelinePlayback::Loop, cx)); }
            }
            Action::SeekRequested { time_ms } => { self.timeline_shell.update(cx, |clock, cx| clock.set_playhead(i64::from(*time_ms) * 1000, cx)); }
            Action::DurationChangeRequested { duration_ms } => { if self.active_motion_clip.is_none() { self.create_motion_clip(cx); } self.set_motion_clip_duration(i64::from(*duration_ms) * 1000, cx); }
            Action::ZoomChangeRequested { zoom } => { if let Some(adapter) = &mut self.gpui_timeline { adapter.zoom = *zoom; } }
            Action::HeightChangeRequested { height } => { if let Some(adapter) = &mut self.gpui_timeline { adapter.height = *height; } }
            Action::TimeUnitChangeRequested { unit } => { if let Some(adapter) = &mut self.gpui_timeline { adapter.unit = *unit; } }
            Action::SnappingChangeRequested { enabled } => { if let Some(adapter) = &mut self.gpui_timeline { adapter.snapping = *enabled; } }
            Action::TrackExpansionRequested { track_id, expanded } => { if let Some(adapter) = &mut self.gpui_timeline { adapter.expanded.insert(track_id.to_string(), *expanded); } }
            Action::ExpandAllRequested { expanded } => { if let Some(adapter) = &mut self.gpui_timeline { for track in &adapter.panel.read(cx).view_data().tracks { adapter.expanded.insert(track.id.to_string(), *expanded); } } }
            Action::KeyframeSelectionRequested { keyframe_ids } => { if let Some(adapter) = &mut self.gpui_timeline { adapter.selected = keyframe_ids.clone(); } }
            Action::TrackSelectionRequested { track_ids } => {
                self.item.update(cx, |item, cx| { item.with_document(cx, |document| {
                    let nodes = track_ids.iter().filter_map(|id| id.parse::<NodeId>().ok()).filter(|id| document.doc.scene.contains(*id)).collect::<Vec<_>>();
                    document.doc.selection.clear(); for node in nodes { document.doc.selection.add(node); }
                    ((), DocChange::Selection)
                }); });
            }
            Action::TrackRenameRequested { track_id, name } => {
                if !name.trim().is_empty() {
                    let operation = track_id.parse::<NodeId>().ok().and_then(|id| self.item.read(cx).document()?.doc.scene.get(id).map(|node| Operation::SetName { id, old: node.name.clone(), new: name.trim().to_owned() }));
                    self.apply_inspector_operations(operation.into_iter().collect(), "Rename layer", window, cx);
                }
            }
            Action::TrackVisibilityRequested { track_id, .. } | Action::TrackLockRequested { track_id, .. } => {
                let operation = track_id.parse::<NodeId>().ok().and_then(|id| {
                    let node = self.item.read(cx).document()?.doc.scene.get(id)?;
                    let mut new = node.flags;
                    match action { Action::TrackVisibilityRequested { visible, .. } => new.set(fanta_doc::NodeFlags::HIDDEN, !visible), Action::TrackLockRequested { locked, .. } => new.set(fanta_doc::NodeFlags::LOCKED, *locked), _ => {} }
                    Some(Operation::SetFlags { id, old: node.flags, new })
                });
                self.apply_inspector_operations(operation.into_iter().collect(), "Edit layer visibility or lock", window, cx);
            }
            Action::PropertyKeyframeRequested { property_id, time_ms, .. } => {
                let operation = self.active_motion_clip.and_then(|clip_id| {
                    let doc = &self.item.read(cx).document()?.doc;
                    let clip = doc.motion.clip(clip_id)?;
                    let track = clip.tracks.values().find(|track| track.id.to_string() == property_id.as_ref())?;
                    let time_ms = (*time_ms).min(clip.duration_ms);
                    if track.keyframes.values().any(|keyframe| keyframe.time_ms == time_ms) {
                        return None;
                    }
                    let value = track.evaluate(time_ms)?;
                    let keyframe = Keyframe::new(KeyframeId::new(), time_ms, value);
                    Some(Operation::SetKeyframe { clip: clip_id, track: track.id, target: track.target, keyframe: keyframe.id, old: None, new: Some(keyframe) })
                });
                self.apply_inspector_operations(operation.into_iter().collect(), "Add keyframe", window, cx);
            }
            Action::AddKeyframeRequested { time_ms } => {
                self.timeline_shell.update(cx, |clock, cx| clock.set_playhead(i64::from(*time_ms) * 1000, cx));
                let operations = self.active_motion_clip.and_then(|clip_id| {
                    let doc = &self.item.read(cx).document()?.doc;
                    let clip = doc.motion.clip(clip_id)?;
                    let time_ms = (*time_ms).min(clip.duration_ms);
                    Some(clip.tracks.values().filter(|track| doc.selection.contains(track.target.node)).filter_map(|track| {
                        if track.keyframes.values().any(|keyframe| keyframe.time_ms == time_ms) {
                            return None;
                        }
                        let value = track.evaluate(time_ms)?;
                        let keyframe = Keyframe::new(KeyframeId::new(), time_ms, value);
                        Some(Operation::SetKeyframe { clip: clip_id, track: track.id, target: track.target, keyframe: keyframe.id, old: None, new: Some(keyframe) })
                    }).collect::<Vec<_>>())
                }).unwrap_or_default();
                if operations.is_empty() {
                    let has_selected_track = self.active_motion_clip.is_some_and(|clip_id| {
                        self.item.read(cx).document().is_some_and(|document| {
                            document.doc.motion.clip(clip_id).is_some_and(|clip| {
                                clip.tracks.values().any(|track| {
                                    document.doc.selection.contains(track.target.node)
                                })
                            })
                        })
                    });
                    if has_selected_track {
                        show_canvas_notice("No new keyframes can be added at this time".into(), window, cx);
                    } else {
                        self.add_toolbar_keyframe(window, cx);
                    }
                } else {
                    self.apply_inspector_operations(operations, "Add keyframes", window, cx);
                }
            }
            Action::KeyframesMoveRequested { .. } | Action::KeyframesDeleteRequested { .. } | Action::KeyframesDuplicateRequested { .. } | Action::EasingChangeRequested { .. } | Action::TrackTimingChangeRequested { .. } => {
                let operations = self.timeline_edit_operations(action, cx);
                self.apply_inspector_operations(operations, "Edit animation", window, cx);
            }
            Action::CollapsedChanged { .. } | Action::EmptyStateDismissed => {}
            Action::HelpRequested => show_canvas_notice("Drag keys or timing bars to retime. Double-click a layer name to rename it. Use the + beside a property to add a keyframe.".into(), window, cx),
            Action::AutoKeyframeChangeRequested { .. } => notify_unavailable("Auto keyframe recording", window, cx),
            Action::PresetApplyRequested {
                track_ids,
                preset_id,
                ..
            } => {
                let can_apply = self.is_editable(cx)
                    && crate::gpui_adapters::toolbar::motion_animation_styles()
                        .contains(preset_id)
                    && self.item.read(cx).document().is_some_and(|document| {
                        super::properties_inspector::motion_preset_available(&document.doc, true)
                            && super::single_selection(&document.doc).is_some_and(|selected| {
                                track_ids.len() == 1
                                    && track_ids.first().is_some_and(|track| {
                                        track.as_ref() == selected.to_string().as_str()
                                    })
                            })
                    });
                if can_apply {
                    self.motion_sidebar
                        .update(cx, |panel, cx| panel.apply_inspector_preset(preset_id, cx));
                    self.sync_motion_timeline(cx);
                }
            }
            Action::ClipTimingChangeRequested { .. } | Action::CommentAddRequested { .. } | Action::CommentOpenRequested { .. } | Action::AskAgentRequested => notify_unavailable("This timeline action", window, cx),
        }
        cx.notify();
    }

    fn timeline_edit_operations(&self, action: &Action, cx: &App) -> Vec<Operation> {
        let Some(clip_id) = self.active_motion_clip else {
            return vec![];
        };
        let Some(clip) = self
            .item
            .read(cx)
            .document()
            .and_then(|document| document.doc.motion.clip(clip_id))
        else {
            return vec![];
        };
        let mut operations = vec![];
        for track in clip.tracks.values() {
            let mut changed = track.clone();
            let span = if let Action::TrackTimingChangeRequested {
                track_id,
                start_ms,
                end_ms,
            } = action
            {
                if track.target.node.to_string() != track_id.as_ref() {
                    continue;
                }
                let times = clip
                    .tracks
                    .values()
                    .filter(|other| other.target.node == track.target.node)
                    .flat_map(|other| other.keyframes.values().map(|key| key.time_ms))
                    .collect::<Vec<_>>();
                Some((
                    *times.iter().min().unwrap_or(&0),
                    *times.iter().max().unwrap_or(&0),
                    *start_ms,
                    *end_ms,
                ))
            } else {
                None
            };
            for key in track.keyframes.values() {
                let id = key.id.to_string();
                match action {
                    Action::KeyframesMoveRequested { keyframes } => {
                        if let Some(time) = keyframes.iter().find(|time| time.id.as_ref() == id)
                            && let Some(changed) = changed.keyframes.get_mut(&key.id)
                        {
                            changed.time_ms = time.time_ms.min(clip.duration_ms);
                        }
                    }
                    Action::KeyframesDeleteRequested { keyframe_ids }
                        if keyframe_ids
                            .iter()
                            .any(|candidate| candidate.as_ref() == id) =>
                    {
                        changed.keyframes.remove(&key.id);
                    }
                    Action::KeyframesDuplicateRequested {
                        keyframe_ids,
                        offset_ms,
                    } if keyframe_ids
                        .iter()
                        .any(|candidate| candidate.as_ref() == id) =>
                    {
                        let mut duplicate = key.clone();
                        duplicate.id = KeyframeId::new();
                        duplicate.time_ms = duplicate
                            .time_ms
                            .saturating_add(*offset_ms)
                            .min(clip.duration_ms);
                        changed.keyframes.insert(duplicate.id, duplicate);
                    }
                    Action::EasingChangeRequested {
                        target: shared::TimelineEasingTarget::Keyframes(ids),
                        easing,
                    } if ids.iter().any(|candidate| candidate.as_ref() == id) => {
                        if let Some(changed) = changed.keyframes.get_mut(&key.id) {
                            (changed.easing, changed.interpolation) = engine_easing(easing);
                        }
                    }
                    Action::TrackTimingChangeRequested { .. } => {
                        if let Some((old_start, old_end, new_start, new_end)) = span
                            && let Some(changed) = changed.keyframes.get_mut(&key.id)
                        {
                            let fraction = f64::from(key.time_ms.saturating_sub(old_start))
                                / f64::from(old_end.saturating_sub(old_start).max(1));
                            changed.time_ms = (f64::from(new_start)
                                + fraction * f64::from(new_end.saturating_sub(new_start)))
                            .round()
                            .clamp(0., f64::from(clip.duration_ms))
                                as u32;
                        }
                    }
                    _ => {}
                }
            }
            if changed != *track {
                operations.push(Operation::SetAnimationTrack {
                    clip: clip_id,
                    track: track.id,
                    old: Some(Box::new(track.clone())),
                    new: Some(Box::new(changed)),
                });
            }
        }
        operations
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{
        AnimationClip, AnimationClipId, CanvasNode, Doc, GroupNode, NodeData, TextNode,
    };
    use gpui::{TestAppContext, VisualTestContext};
    use project::{FakeFs, Project};
    use std::path::PathBuf;

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            zlog::init_test();
            assets::Assets.load_test_fonts(cx);
            let store = settings::SettingsStore::test(cx);
            cx.set_global(store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            release_channel::init(semver::Version::new(0, 0, 0), cx);
            editor::init(cx);
            gpui_component::init(cx);
            fanta_gpui::init(cx);
            crate::theme_bridge::init(cx);
        });
    }

    async fn setup(
        cx: &mut TestAppContext,
        with_clip: bool,
    ) -> (Entity<FigView>, Entity<Timeline>, NodeId, VisualTestContext) {
        init_test(cx);
        let project = Project::test(FakeFs::new(cx.executor()), [], cx).await;
        let mut doc = Doc::new();
        let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let page_id = page.id;
        doc.apply(Operation::create_node(page))
            .expect("create page");
        doc.add_page(page_id);
        doc.set_active_page(Some(page_id));
        let mut target = CanvasNode::new(NodeData::Text(TextNode::new("Target", 120., 40.)));
        target.parent = Some(page_id);
        let target_id = target.id;
        doc.apply(Operation::create_node(target))
            .expect("create target");
        doc.selection.select_only(target_id);
        if with_clip {
            let clip_id = AnimationClipId::new();
            doc.motion
                .clips
                .insert(clip_id, AnimationClip::new(clip_id, "Entrance", 1_500));
        }
        doc.history = Default::default();
        let item = crate::document::ready_item_for_test(
            &project,
            PathBuf::from("/tmp/Motion-timeline-presets.fig"),
            doc,
            cx,
        );
        let (view, cx) =
            cx.add_window_view(move |window, cx| FigView::new(item, project, window, cx));
        cx.run_until_parked();
        view.update(cx, |view, cx| view.set_editor_mode(EditorMode::Motion, cx));
        cx.run_until_parked();
        let timeline = view.read_with(cx, |view, _| {
            view.gpui_timeline
                .as_ref()
                .expect("timeline adapter")
                .panel
                .clone()
        });
        (view, timeline, target_id, cx.clone())
    }

    #[gpui::test]
    async fn timeline_transport_requires_an_active_clip(cx: &mut TestAppContext) {
        let (view, timeline, _, mut cx) = setup(cx, false).await;
        let cx = &mut cx;
        assert!(!timeline.read_with(cx, |timeline, _| timeline.transport_available()));

        timeline.update(cx, |_, cx| {
            cx.emit(Action::AddKeyframeRequested { time_ms: 0 });
        });
        cx.run_until_parked();

        assert!(view.read_with(cx, |view, cx| {
            view.item
                .read(cx)
                .document()
                .is_some_and(|document| !document.doc.motion.clips.is_empty())
        }));
        assert!(timeline.read_with(cx, |timeline, _| timeline.transport_available()));
    }

    #[gpui::test]
    async fn timeline_presets_author_tracks_and_reject_invalid_requests(cx: &mut TestAppContext) {
        let (view, timeline, target_id, mut cx) = setup(cx, true).await;
        let cx = &mut cx;
        let data = timeline.read_with(cx, |timeline, _| timeline.view_data().clone());
        assert_eq!(data.presets.len(), 5);
        assert!(
            data.tracks
                .iter()
                .any(|track| track.id == target_id.to_string() && track.selected)
        );
        timeline.update(cx, |_, cx| {
            cx.emit(Action::PresetApplyRequested {
                track_ids: vec![target_id.to_string().into()],
                preset_id: "Slide in".into(),
                time_ms: 0,
            })
        });
        cx.run_until_parked();

        let before = view.read_with(cx, |view, cx| {
            let document = view.item.read(cx).document().expect("document");
            let clip = document
                .doc
                .motion
                .clips
                .values()
                .next()
                .expect("active clip");
            clip.tracks.len()
        });
        assert!(before > 0, "the first preset should create a timeline row");

        timeline.update(cx, |_, cx| {
            cx.emit(Action::PresetApplyRequested {
                track_ids: vec![target_id.to_string().into()],
                preset_id: "Fade in".into(),
                time_ms: 0,
            })
        });
        cx.run_until_parked();
        view.read_with(cx, |view, cx| {
            let document = view.item.read(cx).document().expect("document");
            let clip = document
                .doc
                .motion
                .clips
                .values()
                .next()
                .expect("active clip");
            assert!(clip.tracks.len() > before);
            assert!(clip.tracks.values().any(|track| {
                track.target.node == target_id
                    && matches!(
                        track.target.property,
                        MotionProperty::Bound {
                            prop: BoundProp::Opacity
                        }
                    )
            }));
        });

        let after = view.read_with(cx, |view, cx| {
            view.item
                .read(cx)
                .document()
                .expect("document")
                .doc
                .motion
                .clips
                .values()
                .next()
                .expect("active clip")
                .tracks
                .len()
        });
        for (tracks, preset) in [
            (vec![target_id.to_string().into()], "Unknown"),
            (vec![NodeId::new().to_string().into()], "Grow"),
        ] {
            timeline.update(cx, |_, cx| {
                cx.emit(Action::PresetApplyRequested {
                    track_ids: tracks,
                    preset_id: preset.into(),
                    time_ms: 0,
                })
            });
            cx.run_until_parked();
        }
        view.read_with(cx, |view, cx| {
            let document = view.item.read(cx).document().expect("document");
            let clip = document
                .doc
                .motion
                .clips
                .values()
                .next()
                .expect("active clip");
            assert_eq!(clip.tracks.len(), after);
        });
    }

    #[gpui::test]
    async fn add_keyframe_on_unanimated_timeline_row_creates_opacity_track(
        cx: &mut TestAppContext,
    ) {
        let (view, timeline, target_id, mut cx) = setup(cx, true).await;
        let cx = &mut cx;
        assert!(timeline.read_with(cx, |timeline, _| {
            timeline.view_data().tracks.iter().any(|track| {
                track.id == target_id.to_string() && track.selected && track.properties.is_empty()
            })
        }));

        timeline.update(cx, |_, cx| {
            cx.emit(Action::AddKeyframeRequested { time_ms: 0 });
        });
        cx.run_until_parked();
        view.read_with(cx, |view, cx| {
            let document = view.item.read(cx).document().expect("document");
            let clip = document
                .doc
                .motion
                .clips
                .values()
                .next()
                .expect("active clip");
            assert!(clip.tracks.values().any(|track| {
                track.target.node == target_id
                    && matches!(
                        track.target.property,
                        MotionProperty::Bound {
                            prop: BoundProp::Opacity
                        }
                    )
                    && track
                        .keyframes
                        .values()
                        .any(|keyframe| keyframe.time_ms == 0)
            }));
        });
    }
}
