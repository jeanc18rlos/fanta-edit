use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use anyhow::{Result, anyhow, ensure};
use core_video::pixel_buffer::CVPixelBuffer;
use futures::future::select;
#[cfg(test)]
use gpui::Entity;
use gpui::{
    AppContext as _, Bounds, Context, DispatchPhase, MouseButton, MouseMoveEvent, MouseUpEvent,
    Pixels, Point, Render, SharedString, Subscription, Task, Window, canvas, surface,
};
use media::video::{
    VideoFrameUpdate, VideoPlayback, VideoPlaybackState, VideoPlaybackStatus,
    prepare_video_playback, video_host_time_seconds,
};
use ui::prelude::*;

const PREPARATION_TIMEOUT: Duration = Duration::from_secs(20);
const LOADING_TIMEOUT: Duration = Duration::from_secs(15);
const SEEK_TIMEOUT: Duration = Duration::from_secs(10);

type SessionFactory = Box<dyn FnOnce() -> Result<Box<dyn PlaybackSession>> + Send>;
type Preparation = Pin<Box<dyn Future<Output = Result<SessionFactory>> + Send>>;

trait PlaybackSession {
    fn play(&mut self) -> Result<()>;
    fn pause(&mut self) -> Result<()>;
    fn seek(&mut self, time_us: u64) -> Result<()>;
    fn set_audio(&mut self, muted: bool, volume: f32) -> Result<()>;
    fn status(&mut self) -> Result<VideoPlaybackStatus>;
    fn frame(&mut self) -> Result<VideoFrameUpdate>;
}

impl PlaybackSession for VideoPlayback {
    fn play(&mut self) -> Result<()> {
        VideoPlayback::play(self)
    }
    fn pause(&mut self) -> Result<()> {
        VideoPlayback::pause(self)
    }
    fn seek(&mut self, time_us: u64) -> Result<()> {
        VideoPlayback::seek(self, time_us)
    }
    fn set_audio(&mut self, muted: bool, volume: f32) -> Result<()> {
        VideoPlayback::set_audio(self, muted, volume)
    }
    fn status(&mut self) -> Result<VideoPlaybackStatus> {
        VideoPlayback::status(self)
    }
    fn frame(&mut self) -> Result<VideoFrameUpdate> {
        self.frame_for_host_time(video_host_time_seconds())
    }
}

pub(crate) struct VideoPlaybackView {
    session: Option<Box<dyn PlaybackSession>>,
    preparation: Option<Task<()>>,
    pending_session: Option<SessionFactory>,
    deadline: Option<Task<()>>,
    deadline_generation: u64,
    frame: Option<CVPixelBuffer>,
    frame_revision: u64,
    status: VideoPlaybackStatus,
    error: Option<SharedString>,
    active: bool,
    window_active: bool,
    closed: bool,
    muted: bool,
    volume: f32,
    wants_play: bool,
    awaiting_frame: bool,
    frame_scheduled: bool,
    scrub_bounds: Option<Bounds<Pixels>>,
    scrubbing: bool,
    activation: Option<Subscription>,
}

impl VideoPlaybackView {
    pub(crate) fn new(bytes: Arc<[u8]>, maximum_dimension: u32, cx: &mut Context<Self>) -> Self {
        Self::with_preparation(
            Box::pin(async move {
                let prepared = prepare_video_playback(bytes, maximum_dimension)?.await?;
                Ok(Box::new(move || {
                    Ok(Box::new(VideoPlayback::new(prepared)?) as Box<dyn PlaybackSession>)
                }) as SessionFactory)
            }),
            cx,
        )
    }

    fn with_preparation(preparation: Preparation, cx: &mut Context<Self>) -> Self {
        let work = cx.background_spawn(preparation);
        let timer = cx.background_executor().timer(PREPARATION_TIMEOUT);
        let task = cx.spawn(async move |this, cx| {
            let result = match select(work, timer).await {
                futures::future::Either::Left((result, _)) => result,
                futures::future::Either::Right(_) => Err(anyhow!(
                    "Video preparation took too long. Select the result again to retry."
                )),
            };
            // The factory creates AVPlayer on this foreground thread, never on
            // the worker that writes and validates the local input.
            if let Err(error) = this.update(cx, |this, cx| {
                if this.closed {
                    return;
                }
                this.preparation = None;
                match result {
                    Ok(factory) => {
                        this.pending_session = Some(factory);
                        this.install_prepared_session(cx);
                    }
                    Err(error) => this.fail(error, cx),
                }
            }) {
                log::debug!("Video preparation owner was released: {error}");
            }
        });
        Self {
            session: None,
            preparation: Some(task),
            pending_session: None,
            deadline: None,
            deadline_generation: 0,
            frame: None,
            frame_revision: 0,
            status: VideoPlaybackStatus {
                state: VideoPlaybackState::Loading,
                current_time_us: 0,
                duration_us: 0,
            },
            error: None,
            active: true,
            window_active: true,
            closed: false,
            muted: true,
            volume: 1.,
            wants_play: false,
            awaiting_frame: false,
            frame_scheduled: false,
            scrub_bounds: None,
            scrubbing: false,
            activation: None,
        }
    }

    fn install_prepared_session(&mut self, cx: &mut Context<Self>) {
        if self.closed || !self.active || !self.window_active {
            return;
        }
        let Some(factory) = self.pending_session.take() else {
            return;
        };
        match factory() {
            Ok(mut session) => {
                let setup = session
                    .set_audio(self.muted, self.volume)
                    .and_then(|()| session.seek(0));
                match setup {
                    Ok(()) => {
                        self.session = Some(session);
                        self.awaiting_frame = true;
                        self.arm_deadline(
                            LOADING_TIMEOUT,
                            "The video did not become ready in time.",
                            cx,
                        );
                        cx.notify();
                    }
                    Err(error) => self.fail(error, cx),
                }
            }
            Err(error) => self.fail(error, cx),
        }
    }

    pub(crate) fn status(&self) -> VideoPlaybackStatus {
        self.status
    }
    pub(crate) fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }
    pub(crate) fn frame(&self) -> Option<CVPixelBuffer> {
        self.frame.clone()
    }
    pub(crate) fn frame_revision(&self) -> u64 {
        self.frame_revision
    }

    pub(crate) fn set_audio(&mut self, muted: bool, volume: f32, cx: &mut Context<Self>) {
        if self.muted == muted && self.volume == volume {
            return;
        }
        let result = (|| {
            ensure!(
                volume.is_finite() && (0. ..=1.).contains(&volume),
                "Video volume must be between zero and one."
            );
            if let Some(session) = self.session.as_mut() {
                session.set_audio(muted, volume)?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.muted = muted;
                self.volume = volume;
                cx.notify();
            }
            Err(error) => self.fail(error, cx),
        }
    }

    pub(crate) fn play(&mut self, cx: &mut Context<Self>) {
        if self.closed || !self.active || !self.window_active {
            return;
        }
        if let Some(session) = self.session.as_mut() {
            match session.play() {
                Ok(()) => {
                    self.wants_play = true;
                    cx.notify();
                }
                Err(error) => self.fail(error, cx),
            }
        }
    }

    pub(crate) fn pause(&mut self, cx: &mut Context<Self>) {
        self.wants_play = false;
        self.scrubbing = false;
        if let Some(session) = self.session.as_mut() {
            if let Err(error) = session.pause() {
                self.fail(error, cx);
                return;
            }
        }
        if self.status.state == VideoPlaybackState::Playing {
            self.status.state = VideoPlaybackState::Paused;
        }
        cx.notify();
    }

    pub(crate) fn seek(&mut self, time_us: u64, cx: &mut Context<Self>) {
        if self.closed || !self.active || !self.window_active {
            return;
        }
        if let Some(session) = self.session.as_mut() {
            let time_us = time_us.min(self.status.duration_us);
            match session.seek(time_us) {
                Ok(()) => {
                    self.status.current_time_us = time_us;
                    self.status.state = VideoPlaybackState::Seeking;
                    self.awaiting_frame = true;
                    // The native engine retains only the active and latest
                    // requested seek. Do not extend its deadline on every drag.
                    if self.deadline.is_none() {
                        self.arm_deadline(SEEK_TIMEOUT, "Seeking in this video took too long.", cx);
                    }
                    cx.notify();
                }
                Err(error) => self.fail(error, cx),
            }
        }
    }

    pub(crate) fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.active == active || self.closed {
            return;
        }
        self.active = active;
        if active {
            self.install_prepared_session(cx);
        }
        if !active {
            self.pause(cx);
            self.clear_deadline();
        } else if self.awaiting_frame && self.session.is_some() {
            self.arm_deadline(
                LOADING_TIMEOUT,
                "The video did not become ready in time.",
                cx,
            );
        }
        cx.notify();
    }

    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        self.closed = true;
        self.preparation = None;
        self.pending_session = None;
        self.clear_deadline();
        self.session = None;
        self.frame = None;
        self.frame_revision = self.frame_revision.wrapping_add(1);
        self.wants_play = false;
        self.scrubbing = false;
        self.awaiting_frame = false;
        self.status.state = VideoPlaybackState::Paused;
        cx.notify();
    }

    fn fail(&mut self, error: anyhow::Error, cx: &mut Context<Self>) {
        self.close(cx);
        self.error = Some(format!("{error:#}").into());
    }

    fn clear_deadline(&mut self) {
        self.deadline_generation = self.deadline_generation.wrapping_add(1);
        self.deadline = None;
    }

    fn arm_deadline(&mut self, duration: Duration, message: &'static str, cx: &mut Context<Self>) {
        self.clear_deadline();
        let generation = self.deadline_generation;
        let timer = cx.background_executor().timer(duration);
        self.deadline = Some(cx.spawn(async move |this, cx| {
            timer.await;
            if let Err(error) = this.update(cx, |this, cx| {
                if this.deadline_generation == generation && !this.closed {
                    this.fail(anyhow!(message), cx);
                }
            }) {
                log::debug!("Video deadline owner was released: {error}");
            }
        }));
    }

    fn poll_session(&mut self, cx: &mut Context<Self>) {
        if self.closed || !self.active || !self.window_active {
            return;
        }
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let result = session
            .status()
            .and_then(|status| Ok((status, session.frame()?)));
        match result {
            Ok((status, frame)) => {
                let changed = self.status != status;
                self.status = status;
                if status.state == VideoPlaybackState::Ended {
                    self.wants_play = false;
                }
                let changed_frame = match frame {
                    VideoFrameUpdate::Frame(frame) => {
                        self.frame = Some(frame.buffer);
                        self.awaiting_frame = false;
                        true
                    }
                    VideoFrameUpdate::Empty => {
                        self.frame = None;
                        true
                    }
                    VideoFrameUpdate::Unchanged => false,
                };
                if changed_frame {
                    self.frame_revision = self.frame_revision.wrapping_add(1);
                }
                let waiting = self.awaiting_frame
                    || matches!(
                        status.state,
                        VideoPlaybackState::Loading | VideoPlaybackState::Seeking
                    );
                if waiting && self.deadline.is_none() {
                    self.arm_deadline(
                        LOADING_TIMEOUT,
                        "The video stopped responding while loading.",
                        cx,
                    );
                } else if !waiting && self.deadline.is_some() {
                    self.clear_deadline();
                }
                if changed || changed_frame {
                    cx.notify();
                }
            }
            Err(error) => self.fail(error, cx),
        }
    }

    pub(crate) fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.install_prepared_session(cx);
        self.poll_session(cx);
        if !self.frame_scheduled
            && !self.closed
            && self.active
            && self.window_active
            && (self.awaiting_frame
                || self.wants_play
                || matches!(
                    self.status.state,
                    VideoPlaybackState::Loading
                        | VideoPlaybackState::Seeking
                        | VideoPlaybackState::Playing
                ))
        {
            self.frame_scheduled = true;
            let this = cx.entity().downgrade();
            window.on_next_frame(move |window, cx| {
                if let Some(this) = this.upgrade() {
                    this.update(cx, |this, cx| {
                        this.frame_scheduled = false;
                        this.tick(window, cx);
                    });
                }
            });
        }
    }

    fn seek_at(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(bounds) = self.scrub_bounds else {
            return;
        };
        if bounds.size.width <= px(0.) {
            return;
        }
        let ratio = ((position.x - bounds.origin.x) / bounds.size.width).clamp(0., 1.);
        self.seek(
            (ratio as f64 * self.status.duration_us as f64).round() as u64,
            cx,
        );
    }

    pub(crate) fn render_controls(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.activation.is_none() {
            self.activation = Some(cx.observe_window_activation(window, |this, window, cx| {
                this.window_active = window.is_window_active();
                if !this.window_active {
                    this.pause(cx);
                    this.clear_deadline();
                }
                cx.notify();
            }));
        }
        let window_active = window.is_window_active();
        if self.window_active && !window_active {
            self.pause(cx);
            self.clear_deadline();
        }
        self.window_active = window_active;
        self.tick(window, cx);
        let ratio = if self.status.duration_us == 0 {
            0.
        } else {
            self.status.current_time_us as f32 / self.status.duration_us as f32
        };
        let weak = cx.entity().downgrade();
        let disabled = self.session.is_none() || self.closed;
        let seek_bar = div()
            .id("video-seek-bar")
            .relative()
            .w_full()
            .h(px(24.))
            .min_w(px(80.))
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    if this.session.is_some() && !this.closed {
                        this.scrubbing = true;
                        this.seek_at(event.position, cx);
                        cx.stop_propagation();
                    }
                }),
            )
            .child(
                canvas(
                    move |bounds, _, cx| {
                        if let Some(view) = weak.upgrade() {
                            view.update(cx, |this, _| this.scrub_bounds = Some(bounds));
                        }
                    },
                    {
                        let weak = cx.entity().downgrade();
                        move |bounds, _, window, cx| {
                            let track = Bounds {
                                origin: gpui::point(bounds.origin.x, bounds.center().y - px(2.)),
                                size: gpui::size(bounds.size.width, px(4.)),
                            };
                            window.paint_quad(gpui::fill(track, cx.theme().colors().border));
                            let progress = Bounds {
                                size: gpui::size(
                                    track.size.width * ratio.clamp(0., 1.),
                                    track.size.height,
                                ),
                                ..track
                            };
                            window
                                .paint_quad(gpui::fill(progress, cx.theme().colors().text_accent));
                            let thumb = Bounds {
                                origin: gpui::point(
                                    progress.right() - px(5.),
                                    bounds.center().y - px(5.),
                                ),
                                size: gpui::size(px(10.), px(10.)),
                            };
                            window.paint_quad(
                                gpui::fill(thumb, cx.theme().colors().text_accent)
                                    .corner_radii(px(5.)),
                            );
                            let view = weak.clone();
                            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                                if phase == DispatchPhase::Bubble
                                    && let Some(view) = view.upgrade()
                                {
                                    view.update(cx, |this, cx| {
                                        if this.scrubbing {
                                            this.seek_at(event.position, cx);
                                            cx.stop_propagation();
                                        }
                                    });
                                }
                            });
                            let view = weak.clone();
                            window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                                if phase == DispatchPhase::Bubble
                                    && event.button == MouseButton::Left
                                    && let Some(view) = view.upgrade()
                                {
                                    view.update(cx, |this, cx| {
                                        if this.scrubbing {
                                            this.seek_at(event.position, cx);
                                            this.scrubbing = false;
                                            cx.stop_propagation();
                                        }
                                    });
                                }
                            });
                        }
                    },
                )
                .size_full(),
            );
        v_flex()
            .w_full()
            .gap_1()
            .child(
                h_flex()
                    .w_full()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new(
                            "video-toggle-play",
                            if self.wants_play { "Pause" } else { "Play" },
                        )
                        .disabled(disabled)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this.wants_play {
                                this.pause(cx);
                            } else {
                                this.play(cx);
                            }
                        })),
                    )
                    .child(
                        Label::new(format!(
                            "{} / {}",
                            time_label(self.status.current_time_us),
                            time_label(self.status.duration_us)
                        ))
                        .size(LabelSize::Small),
                    )
                    .child(
                        Button::new(
                            "video-toggle-mute",
                            if self.muted { "Unmute" } else { "Mute" },
                        )
                        .disabled(disabled)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_audio(!this.muted, this.volume, cx);
                        })),
                    ),
            )
            .child(seek_bar)
            .when_some(self.error.clone(), |element, error| {
                element.child(Label::new(error).color(Color::Error))
            })
            .when(
                self.error.is_none() && (self.preparation.is_some() || self.awaiting_frame),
                |element| element.child(Label::new("Loading video…").color(Color::Muted)),
            )
            .into_any_element()
    }
}

impl Render for VideoPlaybackView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let controls = self.render_controls(window, cx);
        v_flex()
            .size_full()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .w_full()
                    .overflow_hidden()
                    .when_some(self.frame.clone(), |element, frame| {
                        element.child(surface(frame).size_full())
                    }),
            )
            .child(controls)
    }
}

fn time_label(time_us: u64) -> String {
    let seconds = time_us / 1_000_000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(test)]
pub(super) fn fake_playback(cx: &mut App) -> Entity<VideoPlaybackView> {
    tests::fake_view(cx).0
}

#[cfg(test)]
mod tests {
    use super::*;
    use media::video::VideoPlaybackFrame;
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Clone, Default)]
    pub(super) struct FakeState {
        commands: Vec<String>,
        dropped: usize,
        current_time_us: u64,
        playing: bool,
        pending: bool,
        frame_pending: bool,
        empty: bool,
        stall: bool,
        fail: bool,
        volume: f32,
    }

    fn snapshot(state: &Arc<Mutex<FakeState>>) -> FakeState {
        state.lock().expect("fake state").clone()
    }

    struct FakeSession(Arc<Mutex<FakeState>>);
    impl Drop for FakeSession {
        fn drop(&mut self) {
            match self.0.lock() {
                Ok(mut state) => state.dropped += 1,
                Err(poisoned) => {
                    log::error!("Playback fixture state was poisoned before cleanup");
                    poisoned.into_inner().dropped += 1;
                }
            }
        }
    }
    impl PlaybackSession for FakeSession {
        fn play(&mut self) -> Result<()> {
            let mut state = self.0.lock().expect("fake state");
            state.commands.push("play".into());
            state.playing = true;
            Ok(())
        }
        fn pause(&mut self) -> Result<()> {
            let mut state = self.0.lock().expect("fake state");
            state.commands.push("pause".into());
            state.playing = false;
            Ok(())
        }
        fn seek(&mut self, time_us: u64) -> Result<()> {
            let mut state = self.0.lock().expect("fake state");
            state.commands.push(format!("seek:{time_us}"));
            state.current_time_us = time_us;
            state.pending = true;
            state.frame_pending = true;
            Ok(())
        }
        fn set_audio(&mut self, muted: bool, volume: f32) -> Result<()> {
            let mut state = self.0.lock().expect("fake state");
            state.commands.push(format!("muted:{muted}"));
            state.volume = volume;
            Ok(())
        }
        fn status(&mut self) -> Result<VideoPlaybackStatus> {
            let mut state = self.0.lock().expect("fake state");
            if state.fail {
                return Err(anyhow!("fixture playback failure"));
            }
            if !state.stall {
                state.pending = false;
            }
            Ok(VideoPlaybackStatus {
                state: if state.pending {
                    VideoPlaybackState::Seeking
                } else if state.playing {
                    VideoPlaybackState::Playing
                } else {
                    VideoPlaybackState::Paused
                },
                current_time_us: state.current_time_us,
                duration_us: 10_000_000,
            })
        }
        fn frame(&mut self) -> Result<VideoFrameUpdate> {
            let mut state = self.0.lock().expect("fake state");
            if state.empty {
                state.empty = false;
                return Ok(VideoFrameUpdate::Empty);
            }
            if state.pending || !state.frame_pending {
                return Ok(VideoFrameUpdate::Unchanged);
            }
            state.frame_pending = false;
            let buffer = CVPixelBuffer::new(
                core_video::pixel_buffer::kCVPixelFormatType_32BGRA,
                2,
                2,
                None,
            )
            .map_err(|status| anyhow!("fixture pixel buffer failed: {status}"))?;
            Ok(VideoFrameUpdate::Frame(VideoPlaybackFrame {
                buffer,
                presentation_time_us: state.current_time_us,
            }))
        }
    }

    pub(super) fn fake_view(cx: &mut App) -> (Entity<VideoPlaybackView>, Arc<Mutex<FakeState>>) {
        let state = Arc::new(Mutex::new(FakeState::default()));
        let session_state = state.clone();
        let view = cx.new(|cx| {
            VideoPlaybackView::with_preparation(
                Box::pin(async move {
                    Ok(Box::new(move || {
                        Ok(Box::new(FakeSession(session_state)) as Box<dyn PlaybackSession>)
                    }) as SessionFactory)
                }),
                cx,
            )
        });
        (view, state)
    }

    fn ready(cx: &mut gpui::TestAppContext) -> (Entity<VideoPlaybackView>, Arc<Mutex<FakeState>>) {
        let result = cx.update(fake_view);
        cx.run_until_parked();
        result.0.update(cx, |view, cx| view.poll_session(cx));
        assert!(result.0.read_with(cx, |view, _| view.frame().is_some()));
        result
    }

    #[gpui::test]
    fn inline_video_audio_configuration_survives_async_preparation(cx: &mut gpui::TestAppContext) {
        let (view, state) = cx.update(fake_view);
        view.update(cx, |view, cx| view.set_audio(false, 0.25, cx));
        cx.run_until_parked();
        view.update(cx, |view, cx| view.poll_session(cx));
        assert_eq!(snapshot(&state).volume, 0.25);
        assert_eq!(
            snapshot(&state).commands.first().map(String::as_str),
            Some("muted:false")
        );
        view.update(cx, |view, cx| view.set_audio(false, f32::NAN, cx));
        assert!(view.read_with(cx, |view, _| view.error().is_some()));
        assert_eq!(snapshot(&state).dropped, 1);
    }

    #[gpui::test]
    fn inline_video_inactive_preparation_defers_native_commands(cx: &mut gpui::TestAppContext) {
        let (view, state) = cx.update(fake_view);
        view.update(cx, |view, cx| view.set_active(false, cx));
        cx.run_until_parked();
        assert!(snapshot(&state).commands.is_empty());
        view.read_with(cx, |view, _| {
            assert!(view.session.is_none());
            assert!(view.pending_session.is_some());
            assert!(view.deadline.is_none());
        });
        view.update(cx, |view, cx| {
            view.set_active(true, cx);
            view.poll_session(cx);
            assert_eq!(view.status().state, VideoPlaybackState::Paused);
            assert!(view.frame().is_some());
        });
        assert_eq!(snapshot(&state).commands, ["muted:true", "seek:0"]);
        view.update(cx, |view, cx| view.close(cx));
    }

    #[gpui::test]
    fn inline_video_controls_preserve_latest_seek_pause_and_mute(cx: &mut gpui::TestAppContext) {
        let (view, state) = ready(cx);
        view.update(cx, |view, cx| {
            view.play(cx);
            view.seek(8_000_000, cx);
            view.seek(2_000_000, cx);
            view.pause(cx);
            view.poll_session(cx);
            assert_eq!(view.status().state, VideoPlaybackState::Paused);
            assert_eq!(view.status().current_time_us, 2_000_000);
            assert!(view.deadline.is_none());
        });
        let commands = state.lock().expect("fake state").commands.clone();
        assert_eq!(
            commands,
            [
                "muted:true",
                "seek:0",
                "play",
                "seek:8000000",
                "seek:2000000",
                "pause"
            ]
        );
        view.update(cx, |view, cx| view.close(cx));
        assert_eq!(snapshot(&state).dropped, 1);
    }

    #[gpui::test]
    fn inline_video_inactive_pauses_without_resuming_or_owning_old_frames(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, state) = ready(cx);
        let first = view.read_with(cx, |view, _| view.frame_revision());
        view.update(cx, |view, cx| {
            view.play(cx);
            view.set_active(false, cx);
            view.play(cx);
            assert!(!view.wants_play);
            view.set_active(true, cx);
            view.poll_session(cx);
            assert_eq!(view.status().state, VideoPlaybackState::Paused);
        });
        state.lock().expect("fake state").empty = true;
        view.update(cx, |view, cx| {
            view.poll_session(cx);
            assert!(view.frame().is_none());
            assert!(view.frame_revision() > first);
            let empty_revision = view.frame_revision();
            view.close(cx);
            assert!(view.frame_revision() > empty_revision);
        });
        assert_eq!(
            snapshot(&state)
                .commands
                .iter()
                .filter(|command| *command == "play")
                .count(),
            1
        );
    }

    #[gpui::test]
    async fn inline_video_seek_timeout_releases_native_session(cx: &mut gpui::TestAppContext) {
        let (view, state) = ready(cx);
        state.lock().expect("fake state").stall = true;
        view.update(cx, |view, cx| view.seek(3_000_000, cx));
        cx.run_until_parked();
        cx.executor()
            .advance_clock(SEEK_TIMEOUT - Duration::from_millis(1));
        view.update(cx, |view, cx| view.seek(4_000_000, cx));
        cx.executor().advance_clock(Duration::from_millis(2));
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(view.error().expect("seek error").contains("Seeking"));
            assert!(view.frame().is_none());
            assert!(view.session.is_none());
        });
        assert_eq!(snapshot(&state).dropped, 1);
    }

    struct DropProbe(Arc<AtomicUsize>);
    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[gpui::test]
    async fn inline_video_preparation_deadline_and_close_cancel_work(
        cx: &mut gpui::TestAppContext,
    ) {
        for cancel in [false, true] {
            let dropped = Arc::new(AtomicUsize::new(0));
            let probe = DropProbe(dropped.clone());
            let view = cx.new(|cx| {
                VideoPlaybackView::with_preparation(
                    Box::pin(async move {
                        let _probe = probe;
                        futures::future::pending::<Result<SessionFactory>>().await
                    }),
                    cx,
                )
            });
            cx.run_until_parked();
            if cancel {
                view.update(cx, |view, cx| view.close(cx));
            } else {
                cx.executor()
                    .advance_clock(PREPARATION_TIMEOUT + Duration::from_millis(1));
            }
            cx.run_until_parked();
            assert_eq!(dropped.load(Ordering::SeqCst), 1);
            view.read_with(cx, |view, _| {
                assert!(view.session.is_none());
                assert_eq!(view.error().is_none(), cancel);
            });
        }
    }

    #[gpui::test]
    async fn inline_video_closed_owner_rejects_late_prepared_session(
        cx: &mut gpui::TestAppContext,
    ) {
        let (sender, receiver) = futures::channel::oneshot::channel::<SessionFactory>();
        let state = Arc::new(Mutex::new(FakeState::default()));
        let factory_calls = Arc::new(AtomicUsize::new(0));
        let view = cx.new(|cx| {
            VideoPlaybackView::with_preparation(
                Box::pin(async move {
                    receiver
                        .await
                        .map_err(|error| anyhow!("fixture preparation: {error}"))
                }),
                cx,
            )
        });
        cx.run_until_parked();
        let retained_task = view.update(cx, |view, cx| {
            let task = view.preparation.take().expect("pending preparation");
            view.close(cx);
            task
        });
        let calls = factory_calls.clone();
        sender
            .send(Box::new(move || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(Box::new(FakeSession(state)) as Box<dyn PlaybackSession>)
            }))
            .unwrap_or_else(|_| panic!("retained preparation receiver"));
        retained_task.await;
        assert_eq!(factory_calls.load(Ordering::SeqCst), 0);
        assert!(view.read_with(cx, |view, _| view.session.is_none()));
    }

    #[gpui::test]
    async fn inline_video_loading_deadline_remains_bounded_without_render(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, state) = cx.update(fake_view);
        state.lock().expect("fake state").stall = true;
        cx.run_until_parked();
        cx.executor()
            .advance_clock(LOADING_TIMEOUT + Duration::from_millis(1));
        cx.run_until_parked();
        assert!(view.read_with(cx, |view, _| {
            view.error().expect("loading deadline").contains("ready")
        }));
        assert_eq!(snapshot(&state).dropped, 1);
    }

    #[gpui::test]
    fn inline_video_dropped_entity_releases_session(cx: &mut gpui::TestAppContext) {
        let (view, state) = ready(cx);
        let weak = view.downgrade();
        view.update(cx, |view, cx| view.play(cx));
        // GPUI queues the backing entity for release until App::update
        // flushes effects; an idle executor alone does not flush that queue.
        cx.update(|_| drop(view));
        cx.run_until_parked();
        assert!(weak.upgrade().is_none());
        assert_eq!(snapshot(&state).dropped, 1);
    }

    #[gpui::test]
    fn inline_video_native_error_clears_displayed_frame(cx: &mut gpui::TestAppContext) {
        let (view, state) = ready(cx);
        state.lock().expect("fake state").fail = true;
        view.update(cx, |view, cx| view.poll_session(cx));
        view.read_with(cx, |view, _| {
            assert!(
                view.error()
                    .expect("playback error")
                    .contains("fixture playback failure")
            );
            assert!(view.frame().is_none());
        });
    }

    #[gpui::test]
    fn inline_video_seek_bar_uses_final_pointer_position(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            assets::Assets.load_test_fonts(cx);
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
        });
        let state = Arc::new(Mutex::new(FakeState::default()));
        let session_state = state.clone();
        let (view, cx) = cx.add_window_view(|_, cx| {
            VideoPlaybackView::with_preparation(
                Box::pin(async move {
                    Ok(Box::new(move || {
                        Ok(Box::new(FakeSession(session_state)) as Box<dyn PlaybackSession>)
                    }) as SessionFactory)
                }),
                cx,
            )
        });
        cx.simulate_resize(gpui::size(px(240.), px(300.)));
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        let bounds = view.read_with(cx, |view, _| {
            view.scrub_bounds.expect("painted seek control")
        });
        assert!(
            bounds.size.width >= px(80.) && bounds.right() <= px(240.),
            "seek control fits a narrow preview: {bounds:?}"
        );
        let position = |fraction| {
            gpui::point(
                bounds.origin.x + bounds.size.width * fraction,
                bounds.center().y,
            )
        };
        cx.simulate_mouse_down(position(0.2), MouseButton::Left, gpui::Modifiers::none());
        cx.simulate_mouse_move(position(0.5), MouseButton::Left, gpui::Modifiers::none());
        cx.simulate_mouse_up(position(0.8), MouseButton::Left, gpui::Modifiers::none());
        cx.run_until_parked();
        let state = snapshot(&state);
        assert!(
            (state.current_time_us as i64 - 8_000_000).abs() < 1000,
            "final seek: {}",
            state.current_time_us
        );
        assert!(!view.read_with(cx, |view, _| view.scrubbing));
        view.update(cx, |view, cx| view.play(cx));
        cx.deactivate_window();
        assert!(!view.read_with(cx, |view, _| view.wants_play));
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        assert_eq!(
            view.read_with(cx, |view, _| view.status().state),
            VideoPlaybackState::Paused
        );
        view.update(cx, |view, cx| view.close(cx));
    }
}
