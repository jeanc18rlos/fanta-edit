//! Host-controlled animation timeline with Figma Motion empty-state chrome.

use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, canvas, div, prelude::FluentBuilder as _,
    px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, h_flex, v_flex,
};

use crate::atoms::{
    CONTROL_KEY_CONTEXT, ControlExt as _, ControlIcon, icon_button, render_control_icon,
    track_bounds,
};

/// Width of the gutter before the ruler/track content, in pixels.
const RULER_GUTTER: f32 = 18.;

/// Reference design width of the transport/track rail on the left.
const RAIL_WIDTH: f32 = 296.;
/// Floor the rail compresses to on narrow timelines.
const RAIL_MIN_WIDTH: f32 = 220.;
/// Reference design width of the zoom cluster on the right of the header row.
const ZOOM_WIDTH: f32 = 160.;
/// Floor the zoom cluster compresses to before collapsing to its icon-only
/// trigger.
const ZOOM_MIN_WIDTH: f32 = 96.;
/// Width of the icon-only zoom cluster once the slider collapses.
const ZOOM_COMPACT_WIDTH: f32 = 36.;
/// Chrome around the zoom slider track (panel icon, gap, and side slack);
/// the track gets the rest of the cluster.
const ZOOM_TRACK_CHROME: f32 = 68.;
/// Reference design width of the zoom slider track.
const ZOOM_TRACK_WIDTH: f32 = 92.;
/// Floor of the compressed zoom slider track.
const ZOOM_TRACK_MIN_WIDTH: f32 = 40.;
/// Ruler width preserved before the rail starts compressing.
const RULER_MIN_WIDTH: f32 = 120.;
/// Honest minimum outer width of the timeline: the floored rail, the
/// minimum ruler, the icon-only zoom cluster, and the 1 px frame borders.
/// Below it the ruler clips.
pub const TIMELINE_MIN_WIDTH: f32 = RAIL_MIN_WIDTH + RULER_MIN_WIDTH + ZOOM_COMPACT_WIDTH + 2.;
/// Reference design size of the empty-state card.
const EMPTY_CARD_WIDTH: f32 = 418.;
const EMPTY_CARD_HEIGHT: f32 = 144.;
/// Legal zoom range shared by the ruler scale and the zoom slider thumb so
/// the two mappings cannot drift.
const ZOOM_MIN: f32 = 0.25;
const ZOOM_MAX: f32 = 2.0;

#[derive(Clone, Debug, PartialEq)]
pub struct TimelineViewData {
    pub duration_ms: u32,
    pub current_time_ms: u32,
    pub playing: bool,
    pub looping: bool,
    pub zoom: f32,
}

impl Default for TimelineViewData {
    fn default() -> Self {
        Self {
            duration_ms: 2_000,
            current_time_ms: 0,
            playing: false,
            looping: true,
            zoom: 0.5,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum TimelineAction {
    PlayStateChangeRequested { playing: bool },
    LoopChangeRequested { looping: bool },
    AddKeyframeRequested { time_ms: u32 },
    SeekRequested { time_ms: u32 },
    ZoomChangeRequested { zoom: f32 },
    AskAgentRequested,
    EmptyStateDismissed,
    HelpRequested,
}

pub struct Timeline {
    id: SharedString,
    focus_handle: FocusHandle,
    view_data: TimelineViewData,
    show_empty_state: bool,
    ruler_bounds: Option<Bounds<Pixels>>,
    /// Measured width of the whole strip; both rows derive the shared rail
    /// width from it so their vertical border stays aligned while the rail
    /// compresses on narrow timelines.
    strip_width: Option<f32>,
}

impl EventEmitter<TimelineAction> for Timeline {}

impl Timeline {
    pub fn new(
        id: impl Into<SharedString>,
        view_data: TimelineViewData,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            id: id.into(),
            focus_handle: cx.focus_handle(),
            view_data,
            show_empty_state: true,
            ruler_bounds: None,
            strip_width: None,
        }
    }

    pub fn set_view_data(&mut self, view_data: TimelineViewData, cx: &mut Context<Self>) {
        self.view_data = view_data;
        cx.notify();
    }

    pub fn restore_empty_state(&mut self, cx: &mut Context<Self>) {
        self.show_empty_state = true;
        cx.notify();
    }

    /// Zoom-scaled width of one ruler segment; the ruler spans ten segments.
    fn segment_width(&self) -> f32 {
        280. * self.view_data.zoom.clamp(ZOOM_MIN, ZOOM_MAX)
    }

    /// Width of the zoom cluster: the design width while the strip is wide
    /// enough, compressing toward [`ZOOM_MIN_WIDTH`] once the floored rail
    /// and the minimum ruler would no longer fit beside it, and collapsing
    /// to the icon-only [`ZOOM_COMPACT_WIDTH`] below that floor.
    fn zoom_cluster_width(&self) -> f32 {
        self.strip_width.map_or(ZOOM_WIDTH, |width| {
            let available = width - RAIL_MIN_WIDTH - RULER_MIN_WIDTH;
            if available < ZOOM_MIN_WIDTH {
                ZOOM_COMPACT_WIDTH
            } else {
                available.clamp(ZOOM_MIN_WIDTH, ZOOM_WIDTH)
            }
        })
    }

    /// Shared width of the transport and track rails: the design width while
    /// the strip is wide enough, compressing toward [`RAIL_MIN_WIDTH`] once
    /// the ruler would fall below [`RULER_MIN_WIDTH`]. The zoom cluster
    /// compresses and collapses first, so its freed width flows back here.
    fn rail_width(&self) -> f32 {
        self.strip_width.map_or(RAIL_WIDTH, |width| {
            (width - self.zoom_cluster_width() - RULER_MIN_WIDTH).clamp(RAIL_MIN_WIDTH, RAIL_WIDTH)
        })
    }

    /// Playhead x-offset in the shared ruler/track coordinate system: content
    /// starts after the gutter and spans ten zoom-scaled segments. The seek
    /// click handler inverts exactly this mapping so clicking under a tick
    /// seeks to that tick's time.
    fn playhead_left(&self) -> f32 {
        RULER_GUTTER
            + if self.view_data.duration_ms == 0 {
                0.
            } else {
                self.view_data
                    .current_time_ms
                    .min(self.view_data.duration_ms) as f32
                    / self.view_data.duration_ms as f32
                    * self.segment_width()
                    * 10.
            }
    }

    fn render_transport(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .debug_selector(|| "timeline-transport-rail".to_owned())
            .w(px(self.rail_width()))
            .h(px(50.))
            .flex_none()
            .overflow_hidden()
            .px(px(8.))
            .gap(px(8.))
            .border_r_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                icon_button(
                    SharedString::from(format!("{}-play", self.id)),
                    px(22.),
                    px(4.),
                    cx,
                )
                .debug_selector(|| "timeline-play".to_owned())
                .on_activate(cx.listener(|this, _, _, cx| {
                    cx.emit(TimelineAction::PlayStateChangeRequested {
                        playing: !this.view_data.playing,
                    });
                }))
                .child(render_control_icon(
                    if self.view_data.playing {
                        ControlIcon::Pause
                    } else {
                        ControlIcon::Play
                    },
                    cx.theme().foreground,
                    12.,
                )),
            )
            .child(
                icon_button(
                    SharedString::from(format!("{}-keyframe", self.id)),
                    px(22.),
                    px(4.),
                    cx,
                )
                .debug_selector(|| "timeline-keyframe".to_owned())
                .on_activate(cx.listener(|this, _, _, cx| {
                    cx.emit(TimelineAction::AddKeyframeRequested {
                        time_ms: this.view_data.current_time_ms,
                    });
                }))
                .child(render_control_icon(
                    ControlIcon::KeyframeDiamond,
                    cx.theme().foreground,
                    12.,
                )),
            )
            .child(
                h_flex()
                    .h(px(24.))
                    .rounded(px(4.))
                    .bg(cx.theme().secondary)
                    .text_size(px(10.))
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        div()
                            .px(px(7.))
                            .child(format!("{:04}", self.view_data.current_time_ms)),
                    )
                    .child(
                        div()
                            .h_full()
                            .px(px(7.))
                            .items_center()
                            .border_l_1()
                            .border_color(cx.theme().border)
                            .child(format!("{:04}", self.view_data.duration_ms)),
                    )
                    .child(
                        div()
                            .h_full()
                            .px(px(6.))
                            .items_center()
                            .border_l_1()
                            .border_color(cx.theme().border)
                            .child("ms"),
                    ),
            )
            .child(
                icon_button(
                    SharedString::from(format!("{}-loop", self.id)),
                    px(22.),
                    px(4.),
                    cx,
                )
                .debug_selector(|| "timeline-loop".to_owned())
                .when(self.view_data.looping, |button| {
                    button.bg(cx.theme().secondary)
                })
                .on_activate(cx.listener(|this, _, _, cx| {
                    cx.emit(TimelineAction::LoopChangeRequested {
                        looping: !this.view_data.looping,
                    });
                }))
                .child(render_control_icon(
                    ControlIcon::Loop,
                    cx.theme().foreground,
                    12.,
                )),
            )
            .into_any_element()
    }

    fn render_ruler(&self, cx: &mut Context<Self>) -> AnyElement {
        let segment_width = self.segment_width();
        let mut ruler = h_flex()
            .id(SharedString::from(format!("{}-ruler", self.id)))
            .debug_selector(|| "timeline-ruler".to_owned())
            .relative()
            .h(px(50.))
            .flex_1()
            .overflow_hidden()
            .border_b_1()
            .border_color(cx.theme().border)
            .cursor_pointer()
            .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                let Some(bounds) = this.ruler_bounds else {
                    return;
                };
                let Some(position) = event.mouse_position() else {
                    cx.emit(TimelineAction::SeekRequested {
                        time_ms: this.view_data.current_time_ms,
                    });
                    return;
                };
                // Invert playhead_left's mapping so a click under a tick seeks
                // to that tick's labeled time regardless of width and zoom.
                let content_width = (this.segment_width() * 10.).max(1.);
                let offset = f32::from(position.x - bounds.left()) - RULER_GUTTER;
                let ratio = (offset / content_width).clamp(0., 1.);
                cx.emit(TimelineAction::SeekRequested {
                    time_ms: (this.view_data.duration_ms as f32 * ratio).round() as u32,
                });
            }))
            .child(track_bounds(cx.entity(), |this, bounds| {
                this.ruler_bounds = Some(bounds);
            }))
            .child(div().w(px(RULER_GUTTER)).h_full().flex_none());
        for index in 0..=10 {
            let label = self
                .view_data
                .duration_ms
                .saturating_mul(index)
                .checked_div(10)
                .unwrap_or_default();
            ruler = ruler.child(
                v_flex()
                    .w(px(segment_width))
                    .h_full()
                    .flex_none()
                    .items_start()
                    .child(
                        div()
                            .ml(px(segment_width - 1.))
                            .w(px(1.))
                            .h(px(8.))
                            .bg(cx.theme().border),
                    )
                    .child(
                        div()
                            .mt(px(13.))
                            .text_size(px(10.))
                            .text_color(cx.theme().muted_foreground)
                            .child(label.to_string()),
                    ),
            );
        }
        let playhead_left = self.playhead_left();
        ruler
            .child(
                div()
                    .absolute()
                    .left(px(playhead_left))
                    .top_0()
                    .bottom_0()
                    .w(px(1.))
                    .bg(cx.theme().selection),
            )
            .child(
                div()
                    .absolute()
                    .left(px(playhead_left - 8.))
                    .top_0()
                    .w(px(17.))
                    .h(px(14.))
                    .rounded_b(px(4.))
                    .bg(cx.theme().selection),
            )
            .into_any_element()
    }

    fn render_zoom(&self, cx: &mut Context<Self>) -> AnyElement {
        let cluster_width = self.zoom_cluster_width();
        let compact = cluster_width < ZOOM_MIN_WIDTH;
        let cluster = h_flex()
            .debug_selector(|| "timeline-zoom-cluster".to_owned())
            .w(px(cluster_width))
            .h(px(50.))
            .flex_none()
            .gap(px(9.))
            .justify_center()
            .border_l_1()
            .border_b_1()
            .border_color(cx.theme().border);
        // The trigger cycles the host zoom from pointer and keyboard in both
        // presentations; compact keeps the id, focus ring, and activation.
        let trigger = div()
            .id(SharedString::from(format!("{}-zoom", self.id)))
            .debug_selector(|| "timeline-zoom".to_owned())
            .key_context(CONTROL_KEY_CONTEXT)
            .tab_index(0)
            .relative()
            .rounded(px(4.))
            .cursor_pointer()
            .border_1()
            .border_color(cx.theme().transparent)
            .focus(|style| style.border_color(cx.theme().selection))
            .on_activate(cx.listener(|this, _, _, cx| {
                let zoom = if this.view_data.zoom >= ZOOM_MAX {
                    ZOOM_MIN
                } else {
                    (this.view_data.zoom + 0.25).min(ZOOM_MAX)
                };
                cx.emit(TimelineAction::ZoomChangeRequested { zoom });
            }));
        if compact {
            return cluster
                .child(
                    trigger
                        .size(px(24.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(Icon::new(IconName::PanelBottom).small()),
                )
                .into_any_element();
        }
        let track_width =
            (cluster_width - ZOOM_TRACK_CHROME).clamp(ZOOM_TRACK_MIN_WIDTH, ZOOM_TRACK_WIDTH);
        let zoom = self.view_data.zoom.clamp(ZOOM_MIN, ZOOM_MAX);
        let thumb_left = (zoom - ZOOM_MIN) / (ZOOM_MAX - ZOOM_MIN) * (track_width - 30.);
        let fill_width = thumb_left + 6.;
        cluster
            .child(
                trigger
                    .w(px(track_width))
                    .h(px(18.))
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .top(px(8.))
                            .h(px(4.))
                            .rounded(px(2.))
                            .bg(cx.theme().secondary),
                    )
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top(px(8.))
                            .w(px(fill_width))
                            .h(px(4.))
                            .rounded(px(2.))
                            .bg(cx.theme().selection),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(thumb_left))
                            .top(px(4.))
                            .size(px(12.))
                            .rounded(px(6.))
                            .bg(cx.theme().foreground),
                    ),
            )
            .child(Icon::new(IconName::PanelBottom).small())
            .into_any_element()
    }

    fn render_empty_state(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .debug_selector(|| "timeline-empty-card".to_owned())
            .relative()
            .w(px(EMPTY_CARD_WIDTH))
            // The card keeps its fixed design at normal sizes and caps to the
            // available width on narrow timelines.
            .max_w_full()
            .h(px(EMPTY_CARD_HEIGHT))
            .items_center()
            .justify_center()
            .gap(px(6.))
            .rounded(px(13.))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                icon_button(
                    SharedString::from(format!("{}-dismiss-empty", self.id)),
                    px(22.),
                    px(4.),
                    cx,
                )
                .debug_selector(|| "timeline-dismiss-empty".to_owned())
                .absolute()
                .right(px(10.))
                .top(px(16.5))
                .on_activate(cx.listener(|this, _, _, cx| {
                    this.show_empty_state = false;
                    cx.emit(TimelineAction::EmptyStateDismissed);
                    cx.notify();
                }))
                .child(Icon::new(IconName::Close).small()),
            )
            .child(
                div()
                    .relative()
                    .top(px(11.))
                    .font_semibold()
                    .text_size(px(14.))
                    .child("No animations in timeline"),
            )
            .child(
                div()
                    .relative()
                    .top(px(4.5))
                    .max_w(px(330.))
                    .text_center()
                    .text_size(px(12.))
                    .line_height(px(16.))
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        "Select objects on the canvas to create an animation, or ask the Figma agent to create an idea from scratch.",
                    ),
            )
            .child(
                h_flex()
                    .id(SharedString::from(format!("{}-ask-agent", self.id)))
                    .debug_selector(|| "timeline-ask-agent".to_owned())
                    .key_context(CONTROL_KEY_CONTEXT)
                    .tab_index(0)
                    .mt(px(8.))
                    .relative()
                    .top(px(6.))
                    .h(px(24.))
                    .px(px(7.))
                    .gap(px(5.))
                    .rounded(px(5.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .cursor_pointer()
                    .text_size(px(12.))
                    .hover(|style| style.bg(cx.theme().accent))
                    .focus(|style| style.border_color(cx.theme().selection))
                    .on_activate(cx.listener(|_, _, _, cx| {
                        cx.emit(TimelineAction::AskAgentRequested);
                    }))
                    .child(render_control_icon(
                        ControlIcon::Sparkle,
                        cx.theme().foreground,
                        12.,
                    ))
                    .child("Ask agent"),
            )
            .into_any_element()
    }
}

impl Focusable for Timeline {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Timeline {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let playhead_left = self.playhead_left();
        let timeline = cx.entity();
        v_flex()
            .id(self.id.clone())
            .track_focus(&self.focus_handle)
            .relative()
            .size_full()
            .min_h(px(0.))
            .overflow_hidden()
            .rounded_b(px(13.))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                // Measures the strip so both rows share one computed rail
                // width. The write is deferred past the draw so the changed
                // width schedules a re-render (notifying mid-draw is a no-op).
                canvas(
                    move |bounds, _, app| {
                        let width = f32::from(bounds.size.width);
                        let known = timeline.read(app).strip_width;
                        if known.is_none_or(|known| (known - width).abs() > 0.5) {
                            let timeline = timeline.clone();
                            app.defer(move |app| {
                                timeline.update(app, |this, cx| {
                                    this.strip_width = Some(width);
                                    cx.notify();
                                });
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .left_0()
                .top_0()
                .size_full(),
            )
            .child(
                h_flex()
                    .h(px(50.))
                    .w_full()
                    .flex_none()
                    .child(self.render_transport(cx))
                    .child(self.render_ruler(cx))
                    .child(self.render_zoom(cx)),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h(px(0.))
                    .items_start()
                    .child(
                        div()
                            .debug_selector(|| "timeline-track-rail".to_owned())
                            .w(px(self.rail_width()))
                            .h_full()
                            .flex_none()
                            .border_r_1()
                            .border_color(cx.theme().border),
                    )
                    .child(
                        v_flex()
                            .relative()
                            .flex_1()
                            .h_full()
                            .child(
                                div()
                                    .absolute()
                                    .left(px(playhead_left))
                                    .top_0()
                                    .bottom_0()
                                    .w(px(1.))
                                    .bg(cx.theme().selection),
                            )
                            .child(
                                icon_button(
                                    SharedString::from(format!("{}-help", self.id)),
                                    px(32.),
                                    px(16.),
                                    cx,
                                )
                                .debug_selector(|| "timeline-help".to_owned())
                                .absolute()
                                .right(px(24.))
                                .bottom(px(20.))
                                .border_color(cx.theme().border)
                                .bg(cx.theme().background)
                                .focus(|style| style.border_color(cx.theme().selection))
                                .on_activate(cx.listener(|_, _, _, cx| {
                                    cx.emit(TimelineAction::HelpRequested);
                                }))
                                .child(render_control_icon(
                                    ControlIcon::Help,
                                    cx.theme().foreground,
                                    14.,
                                )),
                            ),
                    ),
            )
            .when(self.show_empty_state, |root| {
                root.child(
                    h_flex()
                        .absolute()
                        .left_0()
                        .right_0()
                        .top(px(50.))
                        .bottom_0()
                        .px(px(12.))
                        .items_center()
                        .justify_center()
                        .child(self.render_empty_state(cx)),
                )
            })
    }
}

#[cfg(test)]
mod interaction_tests;
