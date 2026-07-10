use std::time::{Duration, Instant};

use editor::{Editor, EditorEvent, actions::SelectAll};
use fanta_doc::{Easing, Interpolation};
use gpui::{
    App, Bounds, Context, Entity, EventEmitter, Focusable, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Render, SharedString, Subscription, Task,
    Window, canvas, px, relative,
};
use ui::prelude::*;
use ui::{ContextMenu, ContextMenuEntry, DropdownMenu, DropdownStyle, Tooltip};
use util::ResultExt;

const DEFAULT_DURATION_US: i64 = 5_000_000;
pub(crate) const TIMELINE_HEIGHT: Pixels = px(188.);
const TRACK_LABEL_WIDTH: Pixels = px(112.);

#[derive(Debug, Clone, PartialEq)]
pub struct TimelineViewModel {
    pub clip_name: Option<SharedString>,
    pub duration_us: i64,
    pub tracks: Vec<TimelineTrackViewModel>,
}

impl TimelineViewModel {
    pub fn new(duration_us: i64, tracks: Vec<TimelineTrackViewModel>) -> Self {
        Self {
            clip_name: None,
            duration_us: duration_us.max(1),
            tracks,
        }
    }

    pub fn for_clip(
        clip_name: impl Into<SharedString>,
        duration_us: i64,
        tracks: Vec<TimelineTrackViewModel>,
    ) -> Self {
        Self {
            clip_name: Some(clip_name.into()),
            duration_us,
            tracks,
        }
    }

    pub fn empty() -> Self {
        Self::new(DEFAULT_DURATION_US, Vec::new())
    }
}

impl Default for TimelineViewModel {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimelineTrackViewModel {
    pub id: SharedString,
    pub label: SharedString,
    pub keyframes: Vec<TimelineKeyframeViewModel>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimelineKeyframeViewModel {
    pub id: SharedString,
    pub time_us: i64,
    pub interpolation: Interpolation,
    pub easing: Easing,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TimelineKeyframeSelection {
    pub track_id: SharedString,
    pub keyframe_id: SharedString,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineEditPhase {
    Begin,
    Preview,
    Commit,
    Cancel,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TimelineEvent {
    PlayheadChanged(i64),
    PlaybackChanged(bool),
    CreateClip,
    AddKeyframe(TimelineProperty),
    KeyframeSelectionChanged(Option<TimelineKeyframeSelection>),
    EditKeyframeTime {
        keyframe: TimelineKeyframeSelection,
        time_us: i64,
        phase: TimelineEditPhase,
    },
    SetKeyframeInterpolation {
        keyframe: TimelineKeyframeSelection,
        interpolation: Interpolation,
    },
    SetKeyframeEasing {
        keyframe: TimelineKeyframeSelection,
        easing: Easing,
    },
    EditKeyframeEasing {
        keyframe: TimelineKeyframeSelection,
        easing: Easing,
        phase: TimelineEditPhase,
    },
    DeleteKeyframe(TimelineKeyframeSelection),
    RenameClip(SharedString),
    SetClipDuration(i64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineProperty {
    PositionX,
    PositionY,
    Rotation,
    ScaleX,
    ScaleY,
    Opacity,
    FillColor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TimelineKeyframeDrag {
    keyframe: TimelineKeyframeSelection,
    pointer_offset_us: i64,
    original_time_us: i64,
    current_time_us: i64,
}

#[derive(Debug, Clone, PartialEq)]
struct TimelineEasingEdit {
    keyframe: TimelineKeyframeSelection,
    original: Easing,
    current: Easing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimelineClipField {
    Name,
    Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimelineEasingPreset {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    Custom,
}

pub struct TimelineShell {
    model: TimelineViewModel,
    authoring_enabled: bool,
    playhead_us: i64,
    playing: bool,
    scrubbing: bool,
    selected_keyframe: Option<TimelineKeyframeSelection>,
    keyframe_drag: Option<TimelineKeyframeDrag>,
    ruler_bounds: Option<Bounds<Pixels>>,
    playback_epoch: u64,
    playback_task: Option<Task<()>>,
    editing_clip_field: Option<TimelineClipField>,
    clip_edit_error: Option<SharedString>,
    clip_name_editor: Option<Entity<Editor>>,
    clip_duration_editor: Option<Entity<Editor>>,
    easing_editor: Option<Entity<Editor>>,
    easing_edit: Option<TimelineEasingEdit>,
    easing_edit_error: Option<SharedString>,
    _editor_subscriptions: Vec<Subscription>,
}

impl TimelineShell {
    pub fn new() -> Self {
        Self {
            model: TimelineViewModel::empty(),
            authoring_enabled: true,
            playhead_us: 0,
            playing: false,
            scrubbing: false,
            selected_keyframe: None,
            keyframe_drag: None,
            ruler_bounds: None,
            playback_epoch: 0,
            playback_task: None,
            editing_clip_field: None,
            clip_edit_error: None,
            clip_name_editor: None,
            clip_duration_editor: None,
            easing_editor: None,
            easing_edit: None,
            easing_edit_error: None,
            _editor_subscriptions: Vec::new(),
        }
    }

    pub fn set_model(&mut self, model: TimelineViewModel, cx: &mut Context<Self>) {
        let cannot_play = model.clip_name.is_none() || model.duration_us <= 0;
        if cannot_play {
            self.pause(cx);
        }
        self.model = TimelineViewModel {
            clip_name: model.clip_name,
            duration_us: model.duration_us.max(1),
            tracks: model.tracks,
        };
        if self.easing_edit.as_ref().is_some_and(|edit| {
            model_keyframe_easing(&self.model, &edit.keyframe) != Some(edit.current)
        }) {
            self.cancel_easing_edit(cx);
        }
        if self
            .selected_keyframe
            .as_ref()
            .is_some_and(|keyframe| !model_contains_keyframe(&self.model, keyframe))
        {
            self.selected_keyframe = None;
            self.keyframe_drag = None;
            cx.emit(TimelineEvent::KeyframeSelectionChanged(None));
        }
        self.set_playhead(self.playhead_us, cx);
        cx.notify();
    }

    pub fn view_model(&self) -> &TimelineViewModel {
        &self.model
    }

    pub fn playhead_us(&self) -> i64 {
        self.playhead_us
    }

    pub fn selected_keyframe(&self) -> Option<&TimelineKeyframeSelection> {
        self.selected_keyframe.as_ref()
    }

    pub(crate) fn authoring_enabled(&self) -> bool {
        self.authoring_enabled
    }

    pub(crate) fn set_authoring_enabled(
        &mut self,
        authoring_enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if self.authoring_enabled == authoring_enabled {
            return;
        }
        if !authoring_enabled {
            self.cancel_authoring_gestures(cx);
        }
        self.authoring_enabled = authoring_enabled;
        cx.notify();
    }

    pub(crate) fn cancel_authoring_gestures(&mut self, cx: &mut Context<Self>) {
        let mut changed = self.reset_keyframe_drag_inner();
        changed |= self.cancel_easing_edit(cx);
        self.scrubbing = false;
        if self.editing_clip_field.take().is_some() {
            changed = true;
        }
        if self.clip_edit_error.take().is_some() {
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    pub(crate) fn reset_keyframe_drag(&mut self, cx: &mut Context<Self>) {
        if self.reset_keyframe_drag_inner() {
            cx.notify();
        }
    }

    fn reset_keyframe_drag_inner(&mut self) -> bool {
        if let Some(drag) = self.keyframe_drag.take() {
            if model_keyframe_time(&self.model, &drag.keyframe) == Some(drag.current_time_us) {
                set_model_keyframe_time(&mut self.model, &drag.keyframe, drag.original_time_us);
            }
            true
        } else {
            false
        }
    }

    fn ensure_clip_editors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.clip_name_editor.is_some()
            && self.clip_duration_editor.is_some()
            && self.easing_editor.is_some()
        {
            return;
        }

        let clip_name_editor = cx.new(|cx| Editor::single_line(window, cx));
        let clip_duration_editor = cx.new(|cx| Editor::single_line(window, cx));
        let easing_editor = cx.new(|cx| Editor::single_line(window, cx));
        let name_subscription = cx.subscribe_in(
            &clip_name_editor,
            window,
            |timeline: &mut Self, _, event: &EditorEvent, _window, cx| {
                if timeline.editing_clip_field == Some(TimelineClipField::Name) {
                    if matches!(event, EditorEvent::Edited { .. }) {
                        timeline.clear_clip_edit_error(cx);
                    } else if matches!(event, EditorEvent::Blurred) {
                        timeline.commit_clip_edit(cx);
                    }
                }
            },
        );
        let duration_subscription = cx.subscribe_in(
            &clip_duration_editor,
            window,
            |timeline: &mut Self, _, event: &EditorEvent, _window, cx| {
                if timeline.editing_clip_field == Some(TimelineClipField::Duration) {
                    if matches!(event, EditorEvent::Edited { .. }) {
                        timeline.clear_clip_edit_error(cx);
                    } else if matches!(event, EditorEvent::Blurred) {
                        timeline.commit_clip_edit(cx);
                    }
                }
            },
        );
        let easing_subscription = cx.subscribe_in(
            &easing_editor,
            window,
            |timeline: &mut Self, _, event: &EditorEvent, _window, cx| {
                if timeline.easing_edit.is_some() {
                    if matches!(event, EditorEvent::Edited { .. }) {
                        timeline.preview_easing_edit(cx);
                    } else if matches!(event, EditorEvent::Blurred) {
                        timeline.commit_easing_edit(cx);
                    }
                }
            },
        );
        self.clip_name_editor = Some(clip_name_editor);
        self.clip_duration_editor = Some(clip_duration_editor);
        self.easing_editor = Some(easing_editor);
        self._editor_subscriptions.extend([
            name_subscription,
            duration_subscription,
            easing_subscription,
        ]);
    }

    fn begin_easing_edit(
        &mut self,
        keyframe: TimelineKeyframeSelection,
        easing: Easing,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.authoring_enabled || !matches!(easing, Easing::CubicBezier { .. }) {
            return;
        }
        self.cancel_easing_edit(cx);
        self.ensure_clip_editors(window, cx);
        let Some(editor) = self.easing_editor.clone() else {
            return;
        };
        self.easing_edit = Some(TimelineEasingEdit {
            keyframe: keyframe.clone(),
            original: easing,
            current: easing,
        });
        self.easing_edit_error = None;
        editor.update(cx, |editor, cx| {
            editor.set_text(format_cubic_bezier(easing), window, cx);
            editor.select_all(&SelectAll, window, cx);
        });
        editor.focus_handle(cx).focus(window, cx);
        cx.emit(TimelineEvent::EditKeyframeEasing {
            keyframe,
            easing,
            phase: TimelineEditPhase::Begin,
        });
        cx.notify();
    }

    fn preview_easing_edit(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.easing_editor.as_ref() else {
            return;
        };
        let text = editor.read(cx).text(cx);
        let easing = match parse_cubic_bezier(&text) {
            Ok(easing) => easing,
            Err(error) => {
                self.easing_edit_error = Some(error);
                cx.notify();
                return;
            }
        };
        let Some(edit) = self.easing_edit.as_ref() else {
            return;
        };
        if model_keyframe_easing(&self.model, &edit.keyframe) != Some(edit.current) {
            self.cancel_easing_edit(cx);
            self.easing_edit_error = Some("The keyframe changed; reopen the easing editor".into());
            cx.notify();
            return;
        }
        self.easing_edit_error = None;
        if edit.current == easing {
            cx.notify();
            return;
        }
        let keyframe = edit.keyframe.clone();
        if !set_model_keyframe_easing(&mut self.model, &keyframe, easing) {
            return;
        }
        if let Some(edit) = self.easing_edit.as_mut() {
            edit.current = easing;
        }
        cx.emit(TimelineEvent::EditKeyframeEasing {
            keyframe,
            easing,
            phase: TimelineEditPhase::Preview,
        });
        cx.notify();
    }

    fn commit_easing_edit(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.authoring_enabled {
            self.cancel_easing_edit(cx);
            return false;
        }
        let Some(editor) = self.easing_editor.as_ref() else {
            return false;
        };
        let text = editor.read(cx).text(cx);
        if let Err(error) = parse_cubic_bezier(&text) {
            self.easing_edit_error = Some(error);
            cx.notify();
            return false;
        }
        self.preview_easing_edit(cx);
        if self.easing_edit_error.is_some() {
            return false;
        }
        let Some(edit) = self.easing_edit.take() else {
            return false;
        };
        self.easing_edit_error = None;
        cx.emit(TimelineEvent::EditKeyframeEasing {
            keyframe: edit.keyframe,
            easing: edit.current,
            phase: TimelineEditPhase::Commit,
        });
        cx.notify();
        true
    }

    pub(crate) fn cancel_easing_edit(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(edit) = self.easing_edit.take() else {
            return self.easing_edit_error.take().is_some();
        };
        if model_keyframe_easing(&self.model, &edit.keyframe) == Some(edit.current) {
            set_model_keyframe_easing(&mut self.model, &edit.keyframe, edit.original);
        }
        self.easing_edit_error = None;
        cx.emit(TimelineEvent::EditKeyframeEasing {
            keyframe: edit.keyframe,
            easing: edit.original,
            phase: TimelineEditPhase::Cancel,
        });
        cx.notify();
        true
    }

    fn handle_easing_editor_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.keystroke.key.as_str() {
            "enter" => {
                cx.stop_propagation();
                self.commit_easing_edit(cx);
            }
            "escape" => {
                cx.stop_propagation();
                self.cancel_easing_edit(cx);
            }
            _ => {}
        }
    }

    fn begin_clip_edit(
        &mut self,
        field: TimelineClipField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.authoring_enabled || self.model.clip_name.is_none() {
            return;
        }
        self.ensure_clip_editors(window, cx);
        let (editor, initial) = match field {
            TimelineClipField::Name => {
                let Some(editor) = self.clip_name_editor.clone() else {
                    return;
                };
                let initial = self.model.clip_name.clone().unwrap_or_default();
                (editor, initial)
            }
            TimelineClipField::Duration => {
                let Some(editor) = self.clip_duration_editor.clone() else {
                    return;
                };
                (editor, format_time(self.model.duration_us).into())
            }
        };
        editor.update(cx, |editor, cx| {
            editor.set_text(initial, window, cx);
            editor.select_all(&SelectAll, window, cx);
        });
        self.editing_clip_field = Some(field);
        self.clip_edit_error = None;
        editor.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn clear_clip_edit_error(&mut self, cx: &mut Context<Self>) {
        if self.clip_edit_error.take().is_some() {
            cx.notify();
        }
    }

    fn pending_clip_edit_event(&self, cx: &App) -> Result<Option<TimelineEvent>, SharedString> {
        let Some(field) = self.editing_clip_field else {
            return Ok(None);
        };
        match field {
            TimelineClipField::Name => {
                let Some(editor) = self.clip_name_editor.as_ref() else {
                    return Err("The clip name editor is unavailable".into());
                };
                let name = editor.read(cx).text(cx).trim().to_owned();
                if name.is_empty() {
                    return Err("Name is required".into());
                }
                Ok((self.model.clip_name.as_deref() != Some(name.as_str()))
                    .then(|| TimelineEvent::RenameClip(name.into())))
            }
            TimelineClipField::Duration => {
                let Some(editor) = self.clip_duration_editor.as_ref() else {
                    return Err("The clip duration editor is unavailable".into());
                };
                let text = editor.read(cx).text(cx);
                let Some(duration_us) = parse_duration_us(&text) else {
                    return Err("Enter a duration greater than 0".into());
                };
                Ok((duration_us != self.model.duration_us)
                    .then_some(TimelineEvent::SetClipDuration(duration_us)))
            }
        }
    }

    fn commit_clip_edit(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.authoring_enabled {
            self.cancel_clip_edit(cx);
            return false;
        }
        let event = match self.pending_clip_edit_event(cx) {
            Ok(event) => event,
            Err(error) => {
                self.clip_edit_error = Some(error);
                cx.notify();
                return false;
            }
        };
        self.editing_clip_field = None;
        self.clip_edit_error = None;
        if let Some(event) = event {
            cx.emit(event);
        }
        cx.notify();
        true
    }

    pub(crate) fn finish_clip_edit(&mut self, cx: &mut Context<Self>) -> Option<TimelineEvent> {
        if self.editing_clip_field.is_none() {
            return None;
        }
        if !self.authoring_enabled {
            self.cancel_clip_edit(cx);
            return None;
        }
        let event = self.pending_clip_edit_event(cx).ok().flatten();
        self.editing_clip_field = None;
        self.clip_edit_error = None;
        cx.notify();
        event
    }

    fn cancel_clip_edit(&mut self, cx: &mut Context<Self>) {
        let was_editing = self.editing_clip_field.take().is_some();
        let had_error = self.clip_edit_error.take().is_some();
        if was_editing || had_error {
            cx.notify();
        }
    }

    fn handle_clip_editor_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.keystroke.key.as_str() {
            "enter" => {
                cx.stop_propagation();
                self.commit_clip_edit(cx);
            }
            "escape" => {
                cx.stop_propagation();
                self.cancel_clip_edit(cx);
            }
            _ => {}
        }
    }

    pub(crate) fn pause(&mut self, cx: &mut Context<Self>) {
        let was_playing = self.playing;
        self.playing = false;
        self.playback_epoch = self.playback_epoch.wrapping_add(1);
        self.playback_task = None;
        if was_playing {
            cx.emit(TimelineEvent::PlaybackChanged(false));
            cx.notify();
        }
    }

    fn set_playhead(&mut self, playhead_us: i64, cx: &mut Context<Self>) {
        let playhead_us = playhead_us.clamp(0, self.model.duration_us);
        if playhead_us == self.playhead_us {
            return;
        }
        self.playhead_us = playhead_us;
        cx.emit(TimelineEvent::PlayheadChanged(playhead_us));
        cx.notify();
    }

    fn seek_from_x(&mut self, x: Pixels, cx: &mut Context<Self>) {
        let Some(bounds) = self.ruler_bounds else {
            return;
        };
        self.set_playhead(
            playhead_for_x(
                f64::from(x),
                f64::from(bounds.left()),
                f64::from(bounds.size.width),
                self.model.duration_us,
            ),
            cx,
        );
    }

    fn begin_scrub(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.scrubbing = true;
        self.seek_from_x(event.position.x, cx);
    }

    fn update_scrub(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.scrubbing {
            self.seek_from_x(event.position.x, cx);
        }
    }

    fn end_scrub(&mut self, event: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.scrubbing {
            self.seek_from_x(event.position.x, cx);
            self.scrubbing = false;
        }
    }

    fn time_from_x(&self, x: Pixels) -> Option<i64> {
        let bounds = self.ruler_bounds?;
        Some(playhead_for_x(
            f64::from(x),
            f64::from(bounds.left()),
            f64::from(bounds.size.width),
            self.model.duration_us,
        ))
    }

    fn begin_keyframe_drag(
        &mut self,
        keyframe: TimelineKeyframeSelection,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.authoring_enabled {
            return;
        }
        self.cancel_easing_edit(cx);
        let Some(time_us) = model_keyframe_time(&self.model, &keyframe) else {
            return;
        };
        self.pause(cx);
        let selection_changed = self.selected_keyframe.as_ref() != Some(&keyframe);
        self.selected_keyframe = Some(keyframe.clone());
        let pointer_offset_us = self
            .time_from_x(event.position.x)
            .unwrap_or(time_us)
            .saturating_sub(time_us);
        self.keyframe_drag = Some(TimelineKeyframeDrag {
            keyframe: keyframe.clone(),
            pointer_offset_us,
            original_time_us: time_us,
            current_time_us: time_us,
        });
        self.set_playhead(time_us, cx);
        if selection_changed {
            cx.emit(TimelineEvent::KeyframeSelectionChanged(Some(
                keyframe.clone(),
            )));
        }
        cx.emit(TimelineEvent::EditKeyframeTime {
            keyframe,
            time_us,
            phase: TimelineEditPhase::Begin,
        });
        cx.stop_propagation();
        cx.notify();
    }

    fn keyframe_drag_time(&self, x: Pixels) -> Option<i64> {
        let drag = self.keyframe_drag.as_ref()?;
        Some(
            self.time_from_x(x)?
                .saturating_sub(drag.pointer_offset_us)
                .clamp(0, self.model.duration_us),
        )
    }

    fn update_keyframe_drag(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.authoring_enabled {
            self.cancel_authoring_gestures(cx);
            return;
        }
        let Some(drag) = self.keyframe_drag.clone() else {
            return;
        };
        let Some(time_us) = self.keyframe_drag_time(event.position.x) else {
            return;
        };
        if !set_model_keyframe_time(&mut self.model, &drag.keyframe, time_us) {
            return;
        }
        if let Some(drag) = self.keyframe_drag.as_mut() {
            drag.current_time_us = time_us;
        }
        self.set_playhead(time_us, cx);
        cx.emit(TimelineEvent::EditKeyframeTime {
            keyframe: drag.keyframe,
            time_us,
            phase: TimelineEditPhase::Preview,
        });
        cx.stop_propagation();
        cx.notify();
    }

    fn end_keyframe_drag(
        &mut self,
        event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.authoring_enabled {
            self.cancel_authoring_gestures(cx);
            return;
        }
        let Some(drag) = self.keyframe_drag.take() else {
            return;
        };
        let time_us = self
            .time_from_x(event.position.x)
            .map(|time_us| {
                time_us
                    .saturating_sub(drag.pointer_offset_us)
                    .clamp(0, self.model.duration_us)
            })
            .or_else(|| model_keyframe_time(&self.model, &drag.keyframe))
            .unwrap_or(0);
        set_model_keyframe_time(&mut self.model, &drag.keyframe, time_us);
        self.set_playhead(time_us, cx);
        cx.emit(TimelineEvent::EditKeyframeTime {
            keyframe: drag.keyframe,
            time_us,
            phase: TimelineEditPhase::Commit,
        });
        cx.stop_propagation();
        cx.notify();
    }

    fn delete_selected_keyframe(&mut self, cx: &mut Context<Self>) {
        if !self.authoring_enabled {
            return;
        }
        let Some(keyframe) = self.selected_keyframe.take() else {
            return;
        };
        self.cancel_easing_edit(cx);
        self.keyframe_drag = None;
        cx.emit(TimelineEvent::KeyframeSelectionChanged(None));
        cx.emit(TimelineEvent::DeleteKeyframe(keyframe));
        cx.notify();
    }

    fn toggle_playback(&mut self, cx: &mut Context<Self>) {
        if self.playing {
            self.pause(cx);
            return;
        }
        if self.model.clip_name.is_none() || self.model.duration_us <= 0 {
            return;
        }
        if self.playhead_us >= self.model.duration_us {
            self.set_playhead(0, cx);
        }
        self.playing = true;
        cx.emit(TimelineEvent::PlaybackChanged(true));
        self.playback_epoch = self.playback_epoch.wrapping_add(1);
        let epoch = self.playback_epoch;
        let start_playhead_us = self.playhead_us;
        let started = Instant::now();
        self.playback_task = Some(cx.spawn(async move |timeline, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let keep_playing = timeline.update(cx, |timeline, cx| {
                    if !timeline.playing || timeline.playback_epoch != epoch {
                        return false;
                    }
                    let elapsed_us =
                        i64::try_from(started.elapsed().as_micros()).unwrap_or(i64::MAX);
                    let next = start_playhead_us.saturating_add(elapsed_us);
                    if next >= timeline.model.duration_us {
                        timeline.set_playhead(timeline.model.duration_us, cx);
                        timeline.playing = false;
                        cx.emit(TimelineEvent::PlaybackChanged(false));
                        cx.notify();
                        return false;
                    }
                    timeline.set_playhead(next, cx);
                    true
                });
                if !matches!(keep_playing, Ok(true)) {
                    break;
                }
            }
        }));
        cx.notify();
    }

    fn emit_create_clip(&mut self, cx: &mut Context<Self>) {
        if self.authoring_enabled {
            cx.emit(TimelineEvent::CreateClip);
        }
    }

    fn emit_add_keyframe(&mut self, property: TimelineProperty, cx: &mut Context<Self>) {
        if self.authoring_enabled {
            cx.emit(TimelineEvent::AddKeyframe(property));
        }
    }

    fn set_selected_interpolation(&mut self, interpolation: Interpolation, cx: &mut Context<Self>) {
        if !self.authoring_enabled {
            return;
        }
        let Some(keyframe) = self.selected_keyframe.clone() else {
            return;
        };
        let Some(current) = model_keyframe(&self.model, &keyframe) else {
            return;
        };
        if current.interpolation == interpolation {
            return;
        }
        cx.emit(TimelineEvent::SetKeyframeInterpolation {
            keyframe,
            interpolation,
        });
    }

    fn set_selected_easing(&mut self, preset: TimelineEasingPreset, cx: &mut Context<Self>) {
        if !self.authoring_enabled {
            return;
        }
        let Some(keyframe) = self.selected_keyframe.clone() else {
            return;
        };
        let Some(current) = model_keyframe_easing(&self.model, &keyframe) else {
            return;
        };
        let easing = easing_for_preset(preset, current);
        if current == easing {
            return;
        }
        self.cancel_easing_edit(cx);
        cx.emit(TimelineEvent::SetKeyframeEasing { keyframe, easing });
    }

    fn bounds_probe(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let timeline = cx.weak_entity();
        canvas(
            move |bounds, _, cx| {
                timeline
                    .update(cx, |timeline, _| timeline.ruler_bounds = Some(bounds))
                    .log_err();
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }

    fn render_ruler(&self, cx: &mut Context<Self>) -> AnyElement {
        let progress = progress(self.playhead_us, self.model.duration_us);
        let mut ruler = div()
            .id("fanta-motion-ruler")
            .relative()
            .h(px(30.))
            .ml(TRACK_LABEL_WIDTH)
            .border_b_1()
            .border_color(cx.theme().colors().border_variant)
            .cursor_col_resize()
            .on_mouse_down(MouseButton::Left, cx.listener(Self::begin_scrub))
            .on_mouse_move(cx.listener(Self::update_scrub))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::end_scrub))
            .child(self.bounds_probe(cx));
        for index in 0..=5 {
            let fraction = index as f32 / 5.0;
            let seconds = self.model.duration_us as f64 * f64::from(fraction) / 1_000_000.0;
            ruler = ruler
                .child(
                    div()
                        .absolute()
                        .left(relative(fraction))
                        .top_0()
                        .h(px(7.))
                        .w_px()
                        .bg(cx.theme().colors().border),
                )
                .child(
                    div().absolute().left(relative(fraction)).top(px(9.)).child(
                        Label::new(format!("{seconds:.1}s"))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
                );
        }
        ruler
            .child(
                div()
                    .absolute()
                    .left(relative(progress))
                    .top_0()
                    .bottom_0()
                    .w(px(2.))
                    .bg(cx.theme().colors().text_accent),
            )
            .into_any_element()
    }

    fn render_track(&self, track: &TimelineTrackViewModel, cx: &mut Context<Self>) -> AnyElement {
        let mut lane = div()
            .relative()
            .flex_1()
            .h_full()
            .border_l_1()
            .border_color(cx.theme().colors().border_variant);
        for keyframe in &track.keyframes {
            let selection = TimelineKeyframeSelection {
                track_id: track.id.clone(),
                keyframe_id: keyframe.id.clone(),
            };
            let selected = self.selected_keyframe.as_ref() == Some(&selection);
            let selection_for_drag = selection.clone();
            lane = lane.child(
                div()
                    .id(keyframe.id.clone())
                    .absolute()
                    .left(relative(progress(keyframe.time_us, self.model.duration_us)))
                    .ml(px(-4.))
                    .top(px(8.))
                    .size(px(9.))
                    .rounded(px(2.))
                    .bg(if selected {
                        cx.theme().colors().text_accent
                    } else {
                        cx.theme().colors().text_muted
                    })
                    .when(selected, |marker| {
                        marker.border_1().border_color(cx.theme().colors().text)
                    })
                    .when(self.authoring_enabled, |marker| {
                        marker.cursor_col_resize().on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |timeline, event, window, cx| {
                                timeline.begin_keyframe_drag(
                                    selection_for_drag.clone(),
                                    event,
                                    window,
                                    cx,
                                );
                            }),
                        )
                    }),
            );
        }
        h_flex()
            .id(track.id.clone())
            .h(px(26.))
            .border_b_1()
            .border_color(cx.theme().colors().border_variant)
            .child(
                div().w(TRACK_LABEL_WIDTH).flex_none().px_2().child(
                    Label::new(track.label.clone())
                        .size(LabelSize::Small)
                        .single_line(),
                ),
            )
            .child(lane)
            .into_any_element()
    }

    fn render_clip_name(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.editing_clip_field == Some(TimelineClipField::Name)
            && let Some(editor) = self.clip_name_editor.clone()
        {
            return div()
                .id("fanta-motion-clip-name-editor")
                .w(px(144.))
                .h(px(26.))
                .overflow_hidden()
                .on_key_down(cx.listener(Self::handle_clip_editor_key_down))
                .child(editor)
                .into_any_element();
        }

        let label = self
            .model
            .clip_name
            .clone()
            .unwrap_or_else(|| "Timeline".into());
        div()
            .id("fanta-motion-clip-name")
            .h(px(24.))
            .max_w(px(160.))
            .px_1()
            .rounded_sm()
            .when(self.authoring_enabled, |element| {
                element
                    .cursor_pointer()
                    .hover(|element| element.bg(cx.theme().colors().element_hover))
                    .on_click(cx.listener(|timeline, _, window, cx| {
                        timeline.begin_clip_edit(TimelineClipField::Name, window, cx)
                    }))
            })
            .child(Label::new(label).size(LabelSize::Small).single_line())
            .into_any_element()
    }

    fn render_clip_duration(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.editing_clip_field == Some(TimelineClipField::Duration)
            && let Some(editor) = self.clip_duration_editor.clone()
        {
            return div()
                .id("fanta-motion-clip-duration-editor")
                .w(px(76.))
                .h(px(26.))
                .overflow_hidden()
                .on_key_down(cx.listener(Self::handle_clip_editor_key_down))
                .child(editor)
                .into_any_element();
        }

        div()
            .id("fanta-motion-clip-duration")
            .h(px(24.))
            .px_1()
            .rounded_sm()
            .when(self.authoring_enabled, |element| {
                element
                    .cursor_pointer()
                    .hover(|element| element.bg(cx.theme().colors().element_hover))
                    .on_click(cx.listener(|timeline, _, window, cx| {
                        timeline.begin_clip_edit(TimelineClipField::Duration, window, cx)
                    }))
            })
            .child(
                Label::new(format_time(self.model.duration_us))
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .into_any_element()
    }

    fn render_interpolation_dropdown(
        &self,
        selection: &TimelineKeyframeSelection,
        current: Interpolation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let timeline = cx.weak_entity();
        let menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
            for (interpolation, label) in [
                (Interpolation::Linear, "Linear"),
                (Interpolation::Hold, "Hold"),
            ] {
                let timeline = timeline.clone();
                menu.push_item(
                    ContextMenuEntry::new(label)
                        .toggleable(IconPosition::End, interpolation == current)
                        .handler(move |_, cx| {
                            timeline
                                .update(cx, |timeline, cx| {
                                    timeline.set_selected_interpolation(interpolation, cx)
                                })
                                .log_err();
                        }),
                );
            }
            menu
        });
        DropdownMenu::new(
            format!(
                "fanta-motion-interpolation-{}-{}",
                selection.track_id, selection.keyframe_id
            ),
            interpolation_label(current),
            menu,
        )
        .style(DropdownStyle::Outlined)
        .trigger_size(ButtonSize::Compact)
        .disabled(!self.authoring_enabled)
        .into_any_element()
    }

    fn render_easing_dropdown(
        &self,
        selection: &TimelineKeyframeSelection,
        current: Easing,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let timeline = cx.weak_entity();
        let current_preset = easing_preset(current);
        let menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
            for (preset, label) in [
                (TimelineEasingPreset::Linear, "Linear"),
                (TimelineEasingPreset::EaseIn, "Ease in"),
                (TimelineEasingPreset::EaseOut, "Ease out"),
                (TimelineEasingPreset::EaseInOut, "Ease in and out"),
                (TimelineEasingPreset::Custom, "Custom cubic-bezier"),
            ] {
                let timeline = timeline.clone();
                menu.push_item(
                    ContextMenuEntry::new(label)
                        .toggleable(IconPosition::End, preset == current_preset)
                        .handler(move |_, cx| {
                            timeline
                                .update(cx, |timeline, cx| timeline.set_selected_easing(preset, cx))
                                .log_err();
                        }),
                );
            }
            menu
        });
        DropdownMenu::new(
            format!(
                "fanta-motion-easing-{}-{}",
                selection.track_id, selection.keyframe_id
            ),
            easing_label(current),
            menu,
        )
        .style(DropdownStyle::Outlined)
        .trigger_size(ButtonSize::Compact)
        .disabled(!self.authoring_enabled)
        .into_any_element()
    }

    fn render_custom_easing_editor(
        &self,
        selection: &TimelineKeyframeSelection,
        easing: Easing,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = format!(
            "fanta-motion-cubic-bezier-{}-{}",
            selection.track_id, selection.keyframe_id
        );
        let editing = self
            .easing_edit
            .as_ref()
            .is_some_and(|edit| edit.keyframe == *selection);
        let mut field = div()
            .id(id)
            .w(px(164.))
            .h(px(24.))
            .px_1p5()
            .rounded_sm()
            .border_1()
            .border_color(if editing {
                cx.theme().colors().border_focused
            } else {
                cx.theme().colors().border_variant
            })
            .bg(cx.theme().colors().editor_background)
            .overflow_hidden();
        if editing && let Some(editor) = self.easing_editor.clone() {
            return field
                .on_key_down(cx.listener(Self::handle_easing_editor_key_down))
                .child(editor)
                .into_any_element();
        }
        if self.authoring_enabled {
            let selection = selection.clone();
            field = field
                .cursor_text()
                .on_click(cx.listener(move |timeline, _, window, cx| {
                    timeline.begin_easing_edit(selection.clone(), easing, window, cx)
                }));
        }
        field
            .child(
                Label::new(format_cubic_bezier(easing))
                    .size(LabelSize::XSmall)
                    .single_line(),
            )
            .tooltip(Tooltip::text("Edit x1, y1, x2, y2"))
            .into_any_element()
    }

    fn render_selected_keyframe_controls(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let selection = self.selected_keyframe.as_ref()?;
        let keyframe = model_keyframe(&self.model, selection)?;
        let interpolation =
            self.render_interpolation_dropdown(selection, keyframe.interpolation, window, cx);
        let easing = self.render_easing_dropdown(selection, keyframe.easing, window, cx);
        let mut controls = h_flex()
            .gap_1()
            .items_center()
            .child(
                Label::new("Interpolation")
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
            .child(interpolation)
            .child(
                Label::new("Easing")
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
            .child(easing);
        if matches!(keyframe.easing, Easing::CubicBezier { .. }) {
            controls =
                controls.child(self.render_custom_easing_editor(selection, keyframe.easing, cx));
        }
        Some(controls.into_any_element())
    }
}

impl Render for TimelineShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_clip_editors(window, cx);
        let keyframe_controls = self.render_selected_keyframe_controls(window, cx);
        let mut tracks = v_flex()
            .id("fanta-motion-tracks")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        if self.model.tracks.is_empty() {
            tracks = tracks.child(
                h_flex().h(px(40.)).px_3().child(
                    Label::new("Select a layer and add a keyframe to begin")
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                ),
            );
        } else {
            for track in &self.model.tracks {
                tracks = tracks.child(self.render_track(track, cx));
            }
        }
        let timeline = cx.weak_entity();
        let keyframe_menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
            for (property, label) in [
                (TimelineProperty::PositionX, "Position X"),
                (TimelineProperty::PositionY, "Position Y"),
                (TimelineProperty::Rotation, "Rotation"),
                (TimelineProperty::ScaleX, "Scale X"),
                (TimelineProperty::ScaleY, "Scale Y"),
                (TimelineProperty::Opacity, "Opacity"),
                (TimelineProperty::FillColor, "Fill color"),
            ] {
                let timeline = timeline.clone();
                menu.push_item(ContextMenuEntry::new(label).handler(move |_, cx| {
                    timeline
                        .update(cx, |timeline, cx| timeline.emit_add_keyframe(property, cx))
                        .log_err();
                }));
            }
            menu
        });
        v_flex()
            .id("fanta-motion-timeline")
            .h(TIMELINE_HEIGHT)
            .flex_none()
            .border_t_1()
            .border_color(cx.theme().colors().border)
            .bg(cx.theme().colors().panel_background)
            .on_mouse_move(cx.listener(Self::update_keyframe_drag))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::end_keyframe_drag))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::end_keyframe_drag))
            .child(
                h_flex()
                    .h(px(34.))
                    .px_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().colors().border_variant)
                    .child(
                        IconButton::new(
                            "fanta-motion-play",
                            if self.playing {
                                IconName::DebugPause
                            } else {
                                IconName::PlayFilled
                            },
                        )
                        .icon_size(IconSize::Small)
                        .tooltip(Tooltip::text(if self.playing { "Pause" } else { "Play" }))
                        .on_click(cx.listener(|timeline, _, _, cx| timeline.toggle_playback(cx))),
                    )
                    .child(self.render_clip_name(cx))
                    .when(self.model.clip_name.is_some(), |header| {
                        header.child(self.render_clip_duration(cx))
                    })
                    .child(if self.model.clip_name.is_some() {
                        DropdownMenu::new(
                            "fanta-motion-add-keyframe",
                            "Add keyframe",
                            keyframe_menu,
                        )
                        .style(DropdownStyle::Outlined)
                        .trigger_size(ButtonSize::Compact)
                        .disabled(!self.authoring_enabled)
                        .into_any_element()
                    } else {
                        Button::new("fanta-motion-create-clip", "Create animation")
                            .size(ButtonSize::Compact)
                            .disabled(!self.authoring_enabled)
                            .on_click(
                                cx.listener(|timeline, _, _, cx| timeline.emit_create_clip(cx)),
                            )
                            .into_any_element()
                    })
                    .when_some(keyframe_controls, |header, controls| header.child(controls))
                    .when(self.selected_keyframe.is_some(), |header| {
                        header.child(
                            IconButton::new("fanta-motion-delete-keyframe", IconName::Trash)
                                .icon_size(IconSize::Small)
                                .disabled(!self.authoring_enabled)
                                .tooltip(Tooltip::text("Delete keyframe"))
                                .on_click(cx.listener(|timeline, _, _, cx| {
                                    timeline.delete_selected_keyframe(cx)
                                })),
                        )
                    })
                    .when_some(self.clip_edit_error.clone(), |header, error| {
                        header.child(
                            Label::new(error)
                                .size(LabelSize::XSmall)
                                .color(Color::Error)
                                .single_line(),
                        )
                    })
                    .when_some(self.easing_edit_error.clone(), |header, error| {
                        header.child(
                            Label::new(error)
                                .size(LabelSize::XSmall)
                                .color(Color::Error)
                                .single_line(),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        Label::new(format_time(self.playhead_us))
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            )
            .child(self.render_ruler(cx))
            .child(tracks)
    }
}

impl EventEmitter<TimelineEvent> for TimelineShell {}

impl Default for TimelineShell {
    fn default() -> Self {
        Self::new()
    }
}

fn progress(time_us: i64, duration_us: i64) -> f32 {
    if duration_us <= 0 {
        return 0.0;
    }
    (time_us as f64 / duration_us as f64).clamp(0.0, 1.0) as f32
}

fn model_contains_keyframe(
    model: &TimelineViewModel,
    selection: &TimelineKeyframeSelection,
) -> bool {
    model_keyframe_time(model, selection).is_some()
}

fn model_keyframe<'a>(
    model: &'a TimelineViewModel,
    selection: &TimelineKeyframeSelection,
) -> Option<&'a TimelineKeyframeViewModel> {
    model
        .tracks
        .iter()
        .find(|track| track.id == selection.track_id)?
        .keyframes
        .iter()
        .find(|keyframe| keyframe.id == selection.keyframe_id)
}

fn model_keyframe_time(
    model: &TimelineViewModel,
    selection: &TimelineKeyframeSelection,
) -> Option<i64> {
    model
        .tracks
        .iter()
        .find(|track| track.id == selection.track_id)?
        .keyframes
        .iter()
        .find(|keyframe| keyframe.id == selection.keyframe_id)
        .map(|keyframe| keyframe.time_us)
}

fn model_keyframe_easing(
    model: &TimelineViewModel,
    selection: &TimelineKeyframeSelection,
) -> Option<Easing> {
    model
        .tracks
        .iter()
        .find(|track| track.id == selection.track_id)?
        .keyframes
        .iter()
        .find(|keyframe| keyframe.id == selection.keyframe_id)
        .map(|keyframe| keyframe.easing)
}

fn set_model_keyframe_easing(
    model: &mut TimelineViewModel,
    selection: &TimelineKeyframeSelection,
    easing: Easing,
) -> bool {
    let Some(keyframe) = model
        .tracks
        .iter_mut()
        .find(|track| track.id == selection.track_id)
        .and_then(|track| {
            track
                .keyframes
                .iter_mut()
                .find(|keyframe| keyframe.id == selection.keyframe_id)
        })
    else {
        return false;
    };
    if keyframe.easing == easing {
        return false;
    }
    keyframe.easing = easing;
    true
}

fn set_model_keyframe_time(
    model: &mut TimelineViewModel,
    selection: &TimelineKeyframeSelection,
    time_us: i64,
) -> bool {
    let Some(keyframe) = model
        .tracks
        .iter_mut()
        .find(|track| track.id == selection.track_id)
        .and_then(|track| {
            track
                .keyframes
                .iter_mut()
                .find(|keyframe| keyframe.id == selection.keyframe_id)
        })
    else {
        return false;
    };
    let time_us = time_us.clamp(0, model.duration_us);
    if keyframe.time_us == time_us {
        return false;
    }
    keyframe.time_us = time_us;
    true
}

fn playhead_for_x(x: f64, left: f64, width: f64, duration_us: i64) -> i64 {
    if width <= 0.0 || duration_us <= 0 {
        return 0;
    }
    (((x - left) / width).clamp(0.0, 1.0) * duration_us as f64).round() as i64
}

fn format_time(time_us: i64) -> String {
    let seconds = time_us.max(0) as f64 / 1_000_000.0;
    format!("{seconds:.2}s")
}

fn parse_duration_us(text: &str) -> Option<i64> {
    let seconds = text.trim().trim_end_matches(['s', 'S']).trim();
    let seconds = seconds.parse::<f64>().ok()?;
    if !seconds.is_finite() || seconds <= 0.0 {
        return None;
    }
    let duration_us = (seconds * 1_000_000.0).round();
    if duration_us > i64::MAX as f64 {
        return Some(i64::MAX);
    }
    Some((duration_us as i64).max(1))
}

fn format_cubic_bezier(easing: Easing) -> String {
    let Easing::CubicBezier { x1, y1, x2, y2 } = easing else {
        return "0.42, 0, 0.58, 1".to_owned();
    };
    format!("{x1:.3}, {y1:.3}, {x2:.3}, {y2:.3}")
}

fn interpolation_label(interpolation: Interpolation) -> &'static str {
    match interpolation {
        Interpolation::Linear => "Linear",
        Interpolation::Hold => "Hold",
    }
}

fn easing_label(easing: Easing) -> &'static str {
    match easing {
        Easing::Linear => "Linear",
        Easing::EaseIn => "Ease in",
        Easing::EaseOut => "Ease out",
        Easing::EaseInOut => "Ease in/out",
        Easing::CubicBezier { .. } => "Custom",
    }
}

fn easing_preset(easing: Easing) -> TimelineEasingPreset {
    match easing {
        Easing::Linear => TimelineEasingPreset::Linear,
        Easing::EaseIn => TimelineEasingPreset::EaseIn,
        Easing::EaseOut => TimelineEasingPreset::EaseOut,
        Easing::EaseInOut => TimelineEasingPreset::EaseInOut,
        Easing::CubicBezier { .. } => TimelineEasingPreset::Custom,
    }
}

fn easing_for_preset(preset: TimelineEasingPreset, current: Easing) -> Easing {
    match preset {
        TimelineEasingPreset::Linear => Easing::Linear,
        TimelineEasingPreset::EaseIn => Easing::EaseIn,
        TimelineEasingPreset::EaseOut => Easing::EaseOut,
        TimelineEasingPreset::EaseInOut => Easing::EaseInOut,
        TimelineEasingPreset::Custom => match current {
            Easing::CubicBezier { .. } => current,
            Easing::Linear => Easing::CubicBezier {
                x1: 0.0,
                y1: 0.0,
                x2: 1.0,
                y2: 1.0,
            },
            Easing::EaseIn => Easing::CubicBezier {
                x1: 0.42,
                y1: 0.0,
                x2: 1.0,
                y2: 1.0,
            },
            Easing::EaseOut => Easing::CubicBezier {
                x1: 0.0,
                y1: 0.0,
                x2: 0.58,
                y2: 1.0,
            },
            Easing::EaseInOut => Easing::CubicBezier {
                x1: 0.42,
                y1: 0.0,
                x2: 0.58,
                y2: 1.0,
            },
        },
    }
}

fn parse_cubic_bezier(text: &str) -> Result<Easing, SharedString> {
    let trimmed = text.trim();
    let values = if let Some(inner) = trimmed
        .strip_prefix("cubic-bezier(")
        .and_then(|inner| inner.strip_suffix(')'))
    {
        inner
    } else {
        trimmed
    };
    let mut values = values.split(',').map(str::trim);
    let parse_value =
        |value: Option<&str>, minimum: f32, maximum: f32| -> Result<f32, SharedString> {
            let Some(value) = value else {
                return Err("Enter four comma-separated control points".into());
            };
            let value = value
                .parse::<f32>()
                .map_err(|_| SharedString::from("Control points must be numbers"))?;
            if !value.is_finite() {
                return Err("Control points must be finite".into());
            }
            Ok(value.clamp(minimum, maximum))
        };
    let x1 = parse_value(values.next(), 0.0, 1.0)?;
    let y1 = parse_value(values.next(), -10.0, 10.0)?;
    let x2 = parse_value(values.next(), 0.0, 1.0)?;
    let y2 = parse_value(values.next(), -10.0, 10.0)?;
    if values.next().is_some() {
        return Err("Enter exactly four control points".into());
    }
    Ok(Easing::CubicBezier { x1, y1, x2, y2 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use settings::SettingsStore;

    struct TimelineEventRecorder {
        events: Vec<TimelineEvent>,
        _subscription: Subscription,
    }

    fn init_editor_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            zlog::init_test();
            assets::Assets.load_test_fonts(cx);
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            release_channel::init(semver::Version::new(0, 0, 0), cx);
            editor::init(cx);
        });
    }

    fn easing_model(selection: &TimelineKeyframeSelection, easing: Easing) -> TimelineViewModel {
        TimelineViewModel::for_clip(
            "Entrance",
            1_000_000,
            vec![TimelineTrackViewModel {
                id: selection.track_id.clone(),
                label: "Layer · Position X".into(),
                keyframes: vec![TimelineKeyframeViewModel {
                    id: selection.keyframe_id.clone(),
                    time_us: 250_000,
                    interpolation: Interpolation::Linear,
                    easing,
                }],
            }],
        )
    }

    #[test]
    fn pointer_positions_map_to_clamped_microsecond_time() {
        assert_eq!(playhead_for_x(50.0, 0.0, 100.0, 4_000_000), 2_000_000);
        assert_eq!(playhead_for_x(-20.0, 0.0, 100.0, 4_000_000), 0);
        assert_eq!(playhead_for_x(120.0, 0.0, 100.0, 4_000_000), 4_000_000);
        assert_eq!(playhead_for_x(50.0, 0.0, 0.0, 4_000_000), 0);
    }

    #[test]
    fn view_model_normalizes_invalid_duration() {
        let model = TimelineViewModel::new(0, Vec::new());
        assert_eq!(model.duration_us, 1);
        assert_eq!(progress(10, model.duration_us), 1.0);
        assert_eq!(format_time(1_250_000), "1.25s");
    }

    #[test]
    fn moving_a_keyframe_preserves_its_stable_identity() {
        let selection = TimelineKeyframeSelection {
            track_id: "track-1".into(),
            keyframe_id: "keyframe-1".into(),
        };
        let mut model = TimelineViewModel::for_clip(
            "Entrance",
            1_000_000,
            vec![TimelineTrackViewModel {
                id: selection.track_id.clone(),
                label: "Layer · Position X".into(),
                keyframes: vec![TimelineKeyframeViewModel {
                    id: selection.keyframe_id.clone(),
                    time_us: 250_000,
                    interpolation: Interpolation::Linear,
                    easing: Easing::EaseInOut,
                }],
            }],
        );

        assert!(set_model_keyframe_time(&mut model, &selection, 750_000));
        assert_eq!(model_keyframe_time(&model, &selection), Some(750_000));
        assert!(model_contains_keyframe(&model, &selection));
    }

    #[test]
    fn duration_field_accepts_seconds_and_rejects_non_positive_values() {
        assert_eq!(parse_duration_us("2.5s"), Some(2_500_000));
        assert_eq!(parse_duration_us("0"), None);
        assert_eq!(parse_duration_us("not a duration"), None);
    }

    #[test]
    fn cubic_bezier_parser_clamps_x_and_preserves_safe_y_overshoot() {
        assert_eq!(
            parse_cubic_bezier("-2, -3, 4, 5"),
            Ok(Easing::CubicBezier {
                x1: 0.0,
                y1: -3.0,
                x2: 1.0,
                y2: 5.0,
            })
        );
        assert_eq!(
            parse_cubic_bezier("0.2, -20, 0.8, 20"),
            Ok(Easing::CubicBezier {
                x1: 0.2,
                y1: -10.0,
                x2: 0.8,
                y2: 10.0,
            })
        );
        assert!(parse_cubic_bezier("NaN, 0, 1, 1").is_err());
        assert!(parse_cubic_bezier("0, 1, 1").is_err());
    }

    #[gpui::test]
    fn disabling_authoring_rewinds_the_active_local_keyframe_drag(cx: &mut TestAppContext) {
        let selection = TimelineKeyframeSelection {
            track_id: "track-1".into(),
            keyframe_id: "keyframe-1".into(),
        };
        let timeline = cx.new(|_| TimelineShell::new());
        timeline.update(cx, |timeline, cx| {
            timeline.set_model(
                TimelineViewModel::for_clip(
                    "Entrance",
                    1_000_000,
                    vec![TimelineTrackViewModel {
                        id: selection.track_id.clone(),
                        label: "Layer · Position X".into(),
                        keyframes: vec![TimelineKeyframeViewModel {
                            id: selection.keyframe_id.clone(),
                            time_us: 250_000,
                            interpolation: Interpolation::Linear,
                            easing: Easing::EaseInOut,
                        }],
                    }],
                ),
                cx,
            );
            timeline.keyframe_drag = Some(TimelineKeyframeDrag {
                keyframe: selection.clone(),
                pointer_offset_us: 0,
                original_time_us: 250_000,
                current_time_us: 700_000,
            });
            assert!(set_model_keyframe_time(
                &mut timeline.model,
                &selection,
                700_000
            ));
            timeline.editing_clip_field = Some(TimelineClipField::Name);
            timeline.clip_edit_error = Some("Name is required".into());

            timeline.reset_keyframe_drag(cx);

            assert!(timeline.authoring_enabled());
            assert!(timeline.keyframe_drag.is_none());
            assert_eq!(
                model_keyframe_time(&timeline.model, &selection),
                Some(250_000)
            );
            assert_eq!(timeline.editing_clip_field, Some(TimelineClipField::Name));
            assert_eq!(
                timeline.clip_edit_error.as_deref(),
                Some("Name is required")
            );

            timeline.keyframe_drag = Some(TimelineKeyframeDrag {
                keyframe: selection.clone(),
                pointer_offset_us: 0,
                original_time_us: 250_000,
                current_time_us: 700_000,
            });
            assert!(set_model_keyframe_time(
                &mut timeline.model,
                &selection,
                700_000
            ));

            timeline.set_authoring_enabled(false, cx);

            assert!(!timeline.authoring_enabled());
            assert!(timeline.keyframe_drag.is_none());
            assert_eq!(
                model_keyframe_time(&timeline.model, &selection),
                Some(250_000)
            );
            assert!(timeline.editing_clip_field.is_none());
            assert!(timeline.clip_edit_error.is_none());
        });
    }

    #[gpui::test]
    fn invalid_clip_edits_stay_open_and_report_validation(cx: &mut TestAppContext) {
        init_editor_test(cx);
        let timeline = cx.add_window(|_, _| TimelineShell::new());
        let timeline_entity = timeline.entity(cx).expect("timeline entity");
        let recorder = cx.new(|cx| TimelineEventRecorder {
            events: Vec::new(),
            _subscription: cx.subscribe(
                &timeline_entity,
                |recorder: &mut TimelineEventRecorder, _, event: &TimelineEvent, _| {
                    recorder.events.push(event.clone())
                },
            ),
        });
        timeline
            .update(cx, |timeline, window, cx| {
                timeline.set_model(
                    TimelineViewModel::for_clip("Entrance", 1_000_000, Vec::new()),
                    cx,
                );
                timeline.begin_clip_edit(TimelineClipField::Name, window, cx);
                let name_editor = timeline.clip_name_editor.clone().expect("clip name editor");
                name_editor.update(cx, |editor, cx| editor.set_text("", window, cx));

                assert!(!timeline.commit_clip_edit(cx));
                assert_eq!(timeline.editing_clip_field, Some(TimelineClipField::Name));
                assert_eq!(
                    timeline.clip_edit_error.as_deref(),
                    Some("Name is required")
                );

                timeline.cancel_clip_edit(cx);
                assert!(timeline.editing_clip_field.is_none());
                assert!(timeline.clip_edit_error.is_none());
                timeline.begin_clip_edit(TimelineClipField::Duration, window, cx);
                let duration_editor = timeline
                    .clip_duration_editor
                    .clone()
                    .expect("clip duration editor");
                duration_editor.update(cx, |editor, cx| {
                    editor.set_text("not a duration", window, cx)
                });

                assert!(!timeline.commit_clip_edit(cx));
                assert_eq!(
                    timeline.editing_clip_field,
                    Some(TimelineClipField::Duration)
                );
                assert_eq!(
                    timeline.clip_edit_error.as_deref(),
                    Some("Enter a duration greater than 0")
                );
                assert_eq!(timeline.finish_clip_edit(cx), None);
                assert!(timeline.editing_clip_field.is_none());
                assert!(timeline.clip_edit_error.is_none());

                timeline.begin_clip_edit(TimelineClipField::Name, window, cx);
                let name_editor = timeline.clip_name_editor.clone().expect("clip name editor");
                name_editor.update(cx, |editor, cx| editor.set_text("Renamed clip", window, cx));
                assert!(matches!(
                    timeline.finish_clip_edit(cx),
                    Some(TimelineEvent::RenameClip(name)) if name.as_ref() == "Renamed clip"
                ));
                assert!(timeline.editing_clip_field.is_none());
            })
            .expect("update timeline window");
        assert!(recorder.read_with(cx, |recorder, _| recorder.events.is_empty()));
    }

    #[gpui::test]
    fn custom_easing_editor_keeps_invalid_text_and_emits_one_live_gesture(cx: &mut TestAppContext) {
        init_editor_test(cx);
        let selection = TimelineKeyframeSelection {
            track_id: "track-1".into(),
            keyframe_id: "keyframe-1".into(),
        };
        let original = Easing::CubicBezier {
            x1: 0.42,
            y1: 0.0,
            x2: 0.58,
            y2: 1.0,
        };
        let timeline = cx.add_window(|_, _| TimelineShell::new());
        let timeline_entity = timeline.entity(cx).expect("timeline entity");
        let recorder = cx.new(|cx| TimelineEventRecorder {
            events: Vec::new(),
            _subscription: cx.subscribe(
                &timeline_entity,
                |recorder: &mut TimelineEventRecorder, _, event: &TimelineEvent, _| {
                    recorder.events.push(event.clone())
                },
            ),
        });

        timeline
            .update(cx, |timeline, window, cx| {
                timeline.set_model(easing_model(&selection, original), cx);
                timeline.selected_keyframe = Some(selection.clone());
                timeline.begin_easing_edit(selection.clone(), original, window, cx);
                let editor = timeline.easing_editor.clone().expect("easing editor");
                editor.update(cx, |editor, cx| editor.set_text("NaN, 0, 1, 1", window, cx));
                timeline.preview_easing_edit(cx);
                assert_eq!(
                    timeline.easing_edit_error.as_deref(),
                    Some("Control points must be finite")
                );
                assert_eq!(
                    model_keyframe_easing(&timeline.model, &selection),
                    Some(original)
                );
                assert_eq!(editor.read(cx).text(cx), "NaN, 0, 1, 1");

                editor.update(cx, |editor, cx| editor.set_text("-2, -3, 4, 5", window, cx));
                timeline.preview_easing_edit(cx);
                let expected = Easing::CubicBezier {
                    x1: 0.0,
                    y1: -3.0,
                    x2: 1.0,
                    y2: 5.0,
                };
                assert_eq!(
                    model_keyframe_easing(&timeline.model, &selection),
                    Some(expected)
                );
                assert!(timeline.commit_easing_edit(cx));
                assert!(timeline.easing_edit.is_none());
                assert!(timeline.easing_edit_error.is_none());
            })
            .expect("update timeline window");

        let phases = recorder.read_with(cx, |recorder, _| {
            recorder
                .events
                .iter()
                .filter_map(|event| match event {
                    TimelineEvent::EditKeyframeEasing { phase, .. } => Some(*phase),
                    _ => None,
                })
                .collect::<Vec<_>>()
        });
        assert_eq!(
            phases,
            vec![
                TimelineEditPhase::Begin,
                TimelineEditPhase::Preview,
                TimelineEditPhase::Commit,
            ]
        );
    }

    #[gpui::test]
    fn disabling_authoring_cancels_easing_without_clobbering_divergence(cx: &mut TestAppContext) {
        init_editor_test(cx);
        let selection = TimelineKeyframeSelection {
            track_id: "track-1".into(),
            keyframe_id: "keyframe-1".into(),
        };
        let original = Easing::CubicBezier {
            x1: 0.42,
            y1: 0.0,
            x2: 0.58,
            y2: 1.0,
        };
        let external = Easing::EaseOut;
        let timeline = cx.add_window(|_, _| TimelineShell::new());

        timeline
            .update(cx, |timeline, window, cx| {
                timeline.set_model(easing_model(&selection, original), cx);
                timeline.selected_keyframe = Some(selection.clone());
                timeline.begin_easing_edit(selection.clone(), original, window, cx);
                let editor = timeline.easing_editor.clone().expect("easing editor");
                editor.update(cx, |editor, cx| editor.set_text("0, -2, 1, 3", window, cx));
                timeline.preview_easing_edit(cx);
                assert_ne!(
                    model_keyframe_easing(&timeline.model, &selection),
                    Some(original)
                );

                assert!(set_model_keyframe_easing(
                    &mut timeline.model,
                    &selection,
                    external
                ));
                timeline.set_authoring_enabled(false, cx);

                assert!(timeline.easing_edit.is_none());
                assert_eq!(
                    model_keyframe_easing(&timeline.model, &selection),
                    Some(external)
                );
            })
            .expect("update timeline window");
    }

    #[gpui::test]
    fn replacing_a_playing_clip_with_an_empty_model_stops_playback(cx: &mut TestAppContext) {
        let timeline = cx.new(|_| TimelineShell::new());
        timeline.update(cx, |timeline, cx| {
            timeline.set_model(
                TimelineViewModel::for_clip("Entrance", 1_000_000, Vec::new()),
                cx,
            );
            timeline.toggle_playback(cx);
            assert!(timeline.playing);

            timeline.set_model(TimelineViewModel::empty(), cx);
            assert!(!timeline.playing);
            assert!(timeline.playback_task.is_none());
        });
    }

    #[gpui::test]
    fn invalid_duration_stops_playback_and_cancels_the_task(cx: &mut TestAppContext) {
        let timeline = cx.new(|_| TimelineShell::new());
        timeline.update(cx, |timeline, cx| {
            timeline.set_model(
                TimelineViewModel::for_clip("Entrance", 1_000_000, Vec::new()),
                cx,
            );
            timeline.set_playhead(500_000, cx);
            timeline.toggle_playback(cx);
            timeline.set_model(TimelineViewModel::for_clip("Entrance", 0, Vec::new()), cx);
            assert!(!timeline.playing);
            assert_eq!(timeline.view_model().duration_us, 1);
            assert!(timeline.playback_task.is_none());
        });
    }
}
