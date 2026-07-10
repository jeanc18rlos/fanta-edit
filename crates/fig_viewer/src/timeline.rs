use std::time::{Duration, Instant};

use gpui::{
    App, Bounds, Context, EventEmitter, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Render, SharedString, Task, Window, canvas, px, relative,
};
use ui::prelude::*;
use ui::{ContextMenu, ContextMenuEntry, DropdownMenu, DropdownStyle, Tooltip};
use util::ResultExt;

const DEFAULT_DURATION_US: i64 = 5_000_000;
pub(crate) const TIMELINE_HEIGHT: Pixels = px(188.);
const TRACK_LABEL_WIDTH: Pixels = px(112.);

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineTrackViewModel {
    pub id: SharedString,
    pub label: SharedString,
    pub keyframes_us: Vec<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineEvent {
    PlayheadChanged(i64),
    PlaybackChanged(bool),
    CreateClip,
    AddKeyframe(TimelineProperty),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineProperty {
    PositionX,
    PositionY,
    Rotation,
    Opacity,
    FillColor,
}

pub struct TimelineShell {
    model: TimelineViewModel,
    playhead_us: i64,
    playing: bool,
    scrubbing: bool,
    ruler_bounds: Option<Bounds<Pixels>>,
    playback_epoch: u64,
    playback_task: Option<Task<()>>,
}

impl TimelineShell {
    pub fn new() -> Self {
        Self {
            model: TimelineViewModel::empty(),
            playhead_us: 0,
            playing: false,
            scrubbing: false,
            ruler_bounds: None,
            playback_epoch: 0,
            playback_task: None,
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
        self.set_playhead(self.playhead_us, cx);
        cx.notify();
    }

    pub fn view_model(&self) -> &TimelineViewModel {
        &self.model
    }

    pub fn playhead_us(&self) -> i64 {
        self.playhead_us
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
        cx.emit(TimelineEvent::CreateClip);
    }

    fn emit_add_keyframe(&mut self, property: TimelineProperty, cx: &mut Context<Self>) {
        cx.emit(TimelineEvent::AddKeyframe(property));
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

    fn render_track(&self, index: usize, track: &TimelineTrackViewModel, cx: &App) -> AnyElement {
        let mut lane = div()
            .relative()
            .flex_1()
            .h_full()
            .border_l_1()
            .border_color(cx.theme().colors().border_variant);
        for keyframe in &track.keyframes_us {
            lane = lane.child(
                div()
                    .absolute()
                    .left(relative(progress(*keyframe, self.model.duration_us)))
                    .top(px(9.))
                    .size(px(7.))
                    .rounded_full()
                    .bg(cx.theme().colors().text_accent),
            );
        }
        h_flex()
            .id(("fanta-motion-track", index))
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
}

impl Render for TimelineShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            for (index, track) in self.model.tracks.iter().enumerate() {
                tracks = tracks.child(self.render_track(index, track, cx));
            }
        }
        let timeline = cx.weak_entity();
        let keyframe_menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
            for (property, label) in [
                (TimelineProperty::PositionX, "Position X"),
                (TimelineProperty::PositionY, "Position Y"),
                (TimelineProperty::Rotation, "Rotation"),
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
                    .child(
                        Label::new(
                            self.model
                                .clip_name
                                .clone()
                                .unwrap_or_else(|| "Timeline".into()),
                        )
                        .size(LabelSize::Small),
                    )
                    .child(if self.model.clip_name.is_some() {
                        DropdownMenu::new(
                            "fanta-motion-add-keyframe",
                            "Add keyframe",
                            keyframe_menu,
                        )
                        .style(DropdownStyle::Outlined)
                        .trigger_size(ButtonSize::Compact)
                        .into_any_element()
                    } else {
                        Button::new("fanta-motion-create-clip", "Create animation")
                            .size(ButtonSize::Compact)
                            .on_click(
                                cx.listener(|timeline, _, _, cx| timeline.emit_create_clip(cx)),
                            )
                            .into_any_element()
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

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
