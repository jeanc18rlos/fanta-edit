use std::rc::Rc;

use gpui::{App, IntoElement, Pixels, RenderOnce, SharedString, Window, px, relative};
use ui::Tooltip;
use ui::prelude::*;

pub const TIMELINE_MIN_ZOOM: f32 = 1.0;
pub const TIMELINE_MAX_ZOOM: f32 = 4.0;
pub const TIMELINE_ZOOM_STEP: f32 = 0.5;

pub type TimelineControlHandler = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct TimelineToolbarButton {
    id: &'static str,
    icon: IconName,
    tooltip: &'static str,
    active: bool,
    disabled: bool,
    on_click: TimelineControlHandler,
}

impl TimelineToolbarButton {
    pub fn new(
        id: &'static str,
        icon: IconName,
        tooltip: &'static str,
        on_click: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id,
            icon,
            tooltip,
            active: false,
            disabled: false,
            on_click: Rc::new(on_click),
        }
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl RenderOnce for TimelineToolbarButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let id = self.id;
        let on_click = self.on_click;
        div()
            .id(id)
            .debug_selector(|| format!("FANTA-TIMELINE-CONTROL-{id}"))
            .child(
                IconButton::new(id, self.icon)
                    .icon_size(IconSize::Small)
                    .toggle_state(self.active)
                    .disabled(self.disabled)
                    .aria_label(self.tooltip)
                    .tooltip(Tooltip::text(self.tooltip))
                    .on_click(move |_, window, cx| on_click(window, cx)),
            )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimelineScale {
    duration_us: i64,
    zoom: f32,
}

impl TimelineScale {
    pub fn new(duration_us: i64, zoom: f32) -> Self {
        Self {
            duration_us: duration_us.max(1),
            zoom: zoom.clamp(TIMELINE_MIN_ZOOM, TIMELINE_MAX_ZOOM),
        }
    }

    pub fn duration_us(self) -> i64 {
        self.duration_us
    }

    pub fn zoom(self) -> f32 {
        self.zoom
    }

    pub fn tick_count(self) -> usize {
        (10.0 * self.zoom).round().clamp(10.0, 40.0) as usize
    }

    pub fn tick_time_us(self, index: usize) -> i64 {
        let tick_count = self.tick_count().max(1);
        let index = index.min(tick_count);
        ((self.duration_us as i128 * index as i128) / tick_count as i128) as i64
    }

    pub fn tick_fraction(self, index: usize) -> f32 {
        index.min(self.tick_count()) as f32 / self.tick_count().max(1) as f32
    }
}

#[derive(IntoElement)]
pub struct TimelineTimecode {
    time_us: i64,
}

impl TimelineTimecode {
    pub fn new(time_us: i64) -> Self {
        Self { time_us }
    }
}

impl RenderOnce for TimelineTimecode {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .h(px(26.))
            .min_w(px(92.))
            .px_2()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().colors().border_variant)
            .bg(cx.theme().colors().editor_background)
            .flex()
            .items_center()
            .justify_center()
            .child(
                Label::new(format_timecode(self.time_us))
                    .size(LabelSize::XSmall)
                    .color(Color::Default),
            )
    }
}

#[derive(IntoElement)]
pub struct TimelineRulerHeader {
    width: Pixels,
    title: SharedString,
    unit: SharedString,
}

impl TimelineRulerHeader {
    pub fn new(
        width: Pixels,
        title: impl Into<SharedString>,
        unit: impl Into<SharedString>,
    ) -> Self {
        Self {
            width,
            title: title.into(),
            unit: unit.into(),
        }
    }
}

impl RenderOnce for TimelineRulerHeader {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        h_flex()
            .h_full()
            .w(self.width)
            .flex_none()
            .px_3()
            .justify_between()
            .border_r_1()
            .border_color(cx.theme().colors().border_variant)
            .child(
                Label::new(self.title)
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
            .child(
                Label::new(self.unit)
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
    }
}

#[derive(IntoElement)]
pub struct TimelineGridLine {
    fraction: f32,
    major: bool,
}

impl TimelineGridLine {
    pub fn new(fraction: f32, major: bool) -> Self {
        Self {
            fraction: fraction.clamp(0.0, 1.0),
            major,
        }
    }
}

impl RenderOnce for TimelineGridLine {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .absolute()
            .left(relative(self.fraction))
            .top_0()
            .bottom_0()
            .w_px()
            .bg(if self.major {
                cx.theme().colors().border_variant
            } else {
                cx.theme().colors().border_variant.opacity(0.45)
            })
    }
}

#[derive(IntoElement)]
pub struct TimelineRulerTick {
    fraction: f32,
    label: SharedString,
}

impl TimelineRulerTick {
    pub fn new(fraction: f32, label: impl Into<SharedString>) -> Self {
        Self {
            fraction: fraction.clamp(0.0, 1.0),
            label: label.into(),
        }
    }
}

impl RenderOnce for TimelineRulerTick {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .absolute()
            .left(relative(self.fraction))
            .top(px(9.))
            .ml(px(5.))
            .child(
                Label::new(self.label)
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
    }
}

#[derive(IntoElement)]
pub struct TimelinePlayhead {
    fraction: f32,
    cap: bool,
}

impl TimelinePlayhead {
    pub fn new(fraction: f32) -> Self {
        Self {
            fraction: fraction.clamp(0.0, 1.0),
            cap: false,
        }
    }

    pub fn with_cap(mut self, cap: bool) -> Self {
        self.cap = cap;
        self
    }
}

impl RenderOnce for TimelinePlayhead {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .absolute()
            .left(relative(self.fraction))
            .top_0()
            .bottom_0()
            .w(px(1.))
            .bg(cx.theme().colors().text_accent)
            .when(self.cap, |playhead| {
                playhead.child(
                    div()
                        .absolute()
                        .left(px(-4.))
                        .top_0()
                        .w(px(9.))
                        .h(px(7.))
                        .rounded_b_md()
                        .bg(cx.theme().colors().text_accent),
                )
            })
    }
}

pub fn format_timecode(time_us: i64) -> String {
    let total_milliseconds = time_us.max(0) / 1_000;
    let minutes = total_milliseconds / 60_000;
    let seconds = (total_milliseconds / 1_000) % 60;
    let milliseconds = total_milliseconds % 1_000;
    format!("{minutes:02}:{seconds:02}.{milliseconds:03}")
}

pub fn format_ruler_time(time_us: i64) -> String {
    let milliseconds = time_us.max(0) as f64 / 1_000.0;
    if milliseconds < 10_000.0 {
        format!("{milliseconds:.0}")
    } else {
        format!("{:.1}s", milliseconds / 1_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_increases_detail_without_changing_time_extent() {
        let fit = TimelineScale::new(2_000_000, 1.0);
        assert_eq!(fit.tick_count(), 10);
        assert_eq!(fit.tick_time_us(1), 200_000);
        assert_eq!(fit.tick_time_us(fit.tick_count()), 2_000_000);
        assert_eq!(fit.tick_fraction(fit.tick_count()), 1.0);

        let zoomed = TimelineScale::new(2_000_000, 2.0);
        assert_eq!(zoomed.tick_count(), 20);
        assert_eq!(zoomed.tick_time_us(1), 100_000);
        assert_eq!(zoomed.tick_time_us(zoomed.tick_count()), 2_000_000);
        assert_eq!(TimelineScale::new(1, 100.0).zoom(), TIMELINE_MAX_ZOOM);
    }

    #[test]
    fn time_formats_are_stable_at_unit_boundaries() {
        assert_eq!(format_timecode(1_250_000), "00:01.250");
        assert_eq!(format_timecode(61_005_000), "01:01.005");
        assert_eq!(format_ruler_time(200_000), "200");
        assert_eq!(format_ruler_time(12_500_000), "12.5s");
    }
}
