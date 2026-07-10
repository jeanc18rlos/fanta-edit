//! The properties panel's anchored color picker popover: a saturation/value
//! square, a hue strip, an alpha strip, and a hex field. Every interaction
//! emits a live [`ColorPickerEvent::Changed`] so the panel can preview the
//! color transiently; the panel commits one undoable operation when the
//! popover closes.

use editor::{Editor, EditorEvent};
use fanta_doc::{Color as FantaColor, Gradient, GradientStop};
use gpui::prelude::*;
use gpui::{
    App, Bounds, Context, DragMoveEvent, Empty, Entity, EventEmitter, FocusHandle, Focusable, Hsla,
    KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Point, Render, Rgba, Subscription, Window,
    canvas, div, linear_color_stop, linear_gradient, px,
};
use ui::Tooltip;
use ui::prelude::*;
use util::ResultExt as _;

/// Hue/saturation/value/alpha, the picker's working color space. `h` is in
/// degrees (0..360); `s`, `v`, and `a` are normalized 0..=1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Hsva {
    pub h: f32,
    pub s: f32,
    pub v: f32,
    pub a: f32,
}

/// Convert an 8-bit RGBA color into HSV. Gray colors (no chroma) report hue 0.
pub(crate) fn rgb_to_hsv(color: FantaColor) -> Hsva {
    let r = f32::from(color.r) / 255.0;
    let g = f32::from(color.g) / 255.0;
    let b = f32::from(color.b) / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let h = if delta <= f32::EPSILON {
        0.0
    } else if max == r {
        60.0 * (((g - b) / delta).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    Hsva {
        h,
        s: if max <= f32::EPSILON {
            0.0
        } else {
            delta / max
        },
        v: max,
        a: f32::from(color.a) / 255.0,
    }
}

/// Convert HSV back into an 8-bit RGBA color.
pub(crate) fn hsv_to_rgb(hsva: Hsva) -> FantaColor {
    let h = hsva.h.rem_euclid(360.0);
    let s = hsva.s.clamp(0.0, 1.0);
    let v = hsva.v.clamp(0.0, 1.0);
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h {
        h if h < 60.0 => (c, x, 0.0),
        h if h < 120.0 => (x, c, 0.0),
        h if h < 180.0 => (0.0, c, x),
        h if h < 240.0 => (0.0, x, c),
        h if h < 300.0 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let channel = |value: f32| ((value + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    FantaColor::rgba(
        channel(r),
        channel(g),
        channel(b),
        (hsva.a.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

fn fanta_to_rgba(color: FantaColor) -> Rgba {
    Rgba {
        r: f32::from(color.r) / 255.0,
        g: f32::from(color.g) / 255.0,
        b: f32::from(color.b) / 255.0,
        a: f32::from(color.a) / 255.0,
    }
}

// =============================================================================
// Gradient helpers (pure — unit tested)
// =============================================================================

/// The four gradient kinds, decoupled from the geometry-carrying [`Gradient`]
/// enum so the type selector and seeding logic can reason about "just the kind".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GradientKind {
    Linear,
    Radial,
    Angular,
    Diamond,
}

impl GradientKind {
    pub(crate) fn of(gradient: &Gradient) -> Self {
        match gradient {
            Gradient::Linear { .. } => GradientKind::Linear,
            Gradient::Radial { .. } => GradientKind::Radial,
            Gradient::Angular { .. } => GradientKind::Angular,
            Gradient::Diamond { .. } => GradientKind::Diamond,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            GradientKind::Linear => "Linear",
            GradientKind::Radial => "Radial",
            GradientKind::Angular => "Angular",
            GradientKind::Diamond => "Diamond",
        }
    }
}

/// Read a gradient's stops regardless of kind.
pub(crate) fn gradient_stops(gradient: &Gradient) -> &[GradientStop] {
    match gradient {
        Gradient::Linear { stops, .. }
        | Gradient::Radial { stops, .. }
        | Gradient::Angular { stops, .. }
        | Gradient::Diamond { stops, .. } => stops,
    }
}

/// Mutate a gradient's stops regardless of kind.
pub(crate) fn gradient_stops_mut(gradient: &mut Gradient) -> &mut Vec<GradientStop> {
    match gradient {
        Gradient::Linear { stops, .. }
        | Gradient::Radial { stops, .. }
        | Gradient::Angular { stops, .. }
        | Gradient::Diamond { stops, .. } => stops,
    }
}

/// A gradient's angle in degrees measured clockwise from the +x axis (the
/// CSS/Figma convention). Only linear and angular gradients carry a
/// user-facing angle; radial/diamond report `None`.
pub(crate) fn gradient_angle_degrees(gradient: &Gradient) -> Option<f32> {
    match gradient {
        Gradient::Linear { start, end, .. } => {
            let dx = end[0] - start[0];
            let dy = end[1] - start[1];
            if dx.abs() < f32::EPSILON && dy.abs() < f32::EPSILON {
                Some(0.0)
            } else {
                Some(dy.atan2(dx).to_degrees().rem_euclid(360.0))
            }
        }
        Gradient::Angular { start_angle, .. } => Some(start_angle.to_degrees().rem_euclid(360.0)),
        Gradient::Radial { .. } | Gradient::Diamond { .. } => None,
    }
}

/// Rewrite a linear gradient's `start`/`end` so the axis points along `degrees`
/// (clockwise from +x), keeping the axis centered on (0.5, 0.5) and spanning
/// the full 0..1 box. Angular gradients update their `start_angle`. Other kinds
/// are returned unchanged. Pure: takes and returns the geometry.
pub(crate) fn set_gradient_angle(gradient: &Gradient, degrees: f32) -> Gradient {
    let radians = degrees.to_radians();
    match gradient {
        Gradient::Linear { stops, .. } => {
            // Half-axis vector from center; project onto the box edges at the
            // unit-square half-extent so the axis stays inside 0..1.
            let (half_x, half_y) = (0.5 * radians.cos(), 0.5 * radians.sin());
            Gradient::Linear {
                start: [0.5 - half_x, 0.5 - half_y],
                end: [0.5 + half_x, 0.5 + half_y],
                stops: stops.clone(),
            }
        }
        Gradient::Angular { center, stops, .. } => Gradient::Angular {
            center: *center,
            start_angle: radians.rem_euclid(std::f32::consts::TAU),
            stops: stops.clone(),
        },
        other => other.clone(),
    }
}

/// Convert `gradient` to `kind`, preserving its stops and, where possible, its
/// geometry (a linear axis's angle maps to an angular start angle and vice
/// versa; radial/diamond share center + radius). Seeds sensible defaults for
/// geometry a kind gains.
pub(crate) fn convert_gradient_kind(gradient: &Gradient, kind: GradientKind) -> Gradient {
    if GradientKind::of(gradient) == kind {
        return gradient.clone();
    }
    let stops = gradient_stops(gradient).to_vec();
    let angle = gradient_angle_degrees(gradient).unwrap_or(90.0);
    match kind {
        GradientKind::Linear => {
            let radians = angle.to_radians();
            let (half_x, half_y) = (0.5 * radians.cos(), 0.5 * radians.sin());
            Gradient::Linear {
                start: [0.5 - half_x, 0.5 - half_y],
                end: [0.5 + half_x, 0.5 + half_y],
                stops,
            }
        }
        GradientKind::Radial => Gradient::Radial {
            center: [0.5, 0.5],
            radius: 0.5,
            handles: None,
            stops,
        },
        GradientKind::Angular => Gradient::Angular {
            center: [0.5, 0.5],
            start_angle: angle.to_radians().rem_euclid(std::f32::consts::TAU),
            stops,
        },
        GradientKind::Diamond => Gradient::Diamond {
            center: [0.5, 0.5],
            radius: 0.5,
            handles: None,
            stops,
        },
    }
}

/// Seed a fresh linear gradient from a solid color, matching Figma's default:
/// the color at 0% fading to a fully-transparent copy of itself at 100%, on a
/// top-to-bottom axis.
pub(crate) fn seed_gradient_from_color(color: FantaColor) -> Gradient {
    let transparent = FantaColor::rgba(color.r, color.g, color.b, 0);
    Gradient::Linear {
        // Top → bottom (90° clockwise from +x): start above, end below.
        start: [0.5, 0.0],
        end: [0.5, 1.0],
        stops: vec![
            GradientStop {
                position: 0.0,
                color,
            },
            GradientStop {
                position: 1.0,
                color: transparent,
            },
        ],
    }
}

/// The color a solid fill should take when a gradient is flattened back to
/// solid: the first (lowest-position) stop, or black when empty.
pub(crate) fn representative_gradient_color(gradient: &Gradient) -> FantaColor {
    let stops = gradient_stops(gradient);
    let mut lowest: Option<&GradientStop> = None;
    for stop in stops {
        if lowest.is_none_or(|current| stop.position < current.position) {
            lowest = Some(stop);
        }
    }
    lowest.map(|stop| stop.color).unwrap_or(FantaColor::BLACK)
}

/// Insert a stop at normalized `position`, interpolating its color from the
/// gradient's existing stops so a click on the preview bar drops a stop that
/// visually matches the bar under the cursor. Returns the index of the new
/// stop within the position-sorted stop list. Pure — operates on a copy.
pub(crate) fn insert_stop_at(gradient: &mut Gradient, position: f32) -> usize {
    let position = position.clamp(0.0, 1.0);
    let color = sample_gradient_color(gradient, position);
    let stops = gradient_stops_mut(gradient);
    stops.push(GradientStop { position, color });
    sort_stops(stops);
    stops
        .iter()
        .position(|stop| (stop.position - position).abs() < f32::EPSILON && stop.color == color)
        .unwrap_or(0)
}

/// Remove the stop at `index`. A gradient must keep at least two stops, so the
/// removal is refused (returns `false`) when only two remain.
pub(crate) fn remove_stop(gradient: &mut Gradient, index: usize) -> bool {
    let stops = gradient_stops_mut(gradient);
    if stops.len() <= 2 || index >= stops.len() {
        return false;
    }
    stops.remove(index);
    true
}

/// Move the stop at `index` to `position`, re-sorting so the list stays ordered.
/// Returns the stop's new index after the re-sort.
pub(crate) fn move_stop(gradient: &mut Gradient, index: usize, position: f32) -> usize {
    let position = position.clamp(0.0, 1.0);
    let stops = gradient_stops_mut(gradient);
    let Some(stop) = stops.get_mut(index) else {
        return index;
    };
    stop.position = position;
    let moved = *stop;
    sort_stops(stops);
    stops
        .iter()
        .position(|candidate| *candidate == moved)
        .unwrap_or(index)
}

/// Replace the color of the stop at `index`, leaving its position untouched.
pub(crate) fn set_stop_color(gradient: &mut Gradient, index: usize, color: FantaColor) {
    if let Some(stop) = gradient_stops_mut(gradient).get_mut(index) {
        stop.color = color;
    }
}

/// Order stops by position; ties keep their relative order (stable) so a newly
/// dropped stop coinciding with an old one stays adjacent to it.
fn sort_stops(stops: &mut [GradientStop]) {
    stops.sort_by(|a, b| {
        a.position
            .partial_cmp(&b.position)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// Linearly interpolate the gradient's color at normalized `position`, treating
/// the stop list as position-sorted key colors (the same sampling the preview
/// bar approximates). Alpha is interpolated too. Used when dropping a new stop.
pub(crate) fn sample_gradient_color(gradient: &Gradient, position: f32) -> FantaColor {
    let position = position.clamp(0.0, 1.0);
    let mut stops: Vec<GradientStop> = gradient_stops(gradient).to_vec();
    sort_stops(&mut stops);
    let Some(first) = stops.first().copied() else {
        return FantaColor::BLACK;
    };
    if position <= first.position {
        return first.color;
    }
    let Some(last) = stops.last().copied() else {
        return FantaColor::BLACK;
    };
    if position >= last.position {
        return last.color;
    }
    for window in stops.windows(2) {
        let (a, b) = (window[0], window[1]);
        if position >= a.position && position <= b.position {
            let span = b.position - a.position;
            let t = if span <= f32::EPSILON {
                0.0
            } else {
                (position - a.position) / span
            };
            return lerp_color(a.color, b.color, t);
        }
    }
    last.color
}

fn lerp_color(a: FantaColor, b: FantaColor, t: f32) -> FantaColor {
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| {
        (f32::from(x) + (f32::from(y) - f32::from(x)) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    FantaColor::rgba(mix(a.r, b.r), mix(a.g, b.g), mix(a.b, b.b), mix(a.a, b.a))
}

/// What the picker tells its owner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ColorPickerEvent {
    /// The working color changed (drag / hex entry). Preview only — the owner
    /// must not push undo operations for these.
    Changed(FantaColor),
    /// Close the popover keeping the current color (owner commits one op).
    Commit,
    /// Close the popover restoring the pre-open color (no op).
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickerRegion {
    SaturationValue,
    Hue,
    Alpha,
}

/// Drag payload marking an in-flight picker gesture; renders nothing.
#[derive(Clone)]
struct PickerDrag(PickerRegion);

impl Render for PickerDrag {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

pub(crate) struct ColorPicker {
    hsva: Hsva,
    focus_handle: FocusHandle,
    hex_editor: Entity<Editor>,
    sv_bounds: Option<Bounds<Pixels>>,
    hue_bounds: Option<Bounds<Pixels>>,
    alpha_bounds: Option<Bounds<Pixels>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ColorPickerEvent> for ColorPicker {}

impl Focusable for ColorPicker {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ColorPicker {
    pub(crate) fn new(initial: FantaColor, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let hex_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_text(initial.to_hex(), window, cx);
            editor
        });
        let editor_subscription = cx.subscribe_in(
            &hex_editor,
            window,
            |this: &mut Self, _, event: &EditorEvent, window, cx| {
                if matches!(event, EditorEvent::BufferEdited | EditorEvent::Blurred) {
                    this.apply_hex_text(window, cx);
                }
            },
        );
        Self {
            hsva: rgb_to_hsv(initial),
            focus_handle: cx.focus_handle(),
            hex_editor,
            sv_bounds: None,
            hue_bounds: None,
            alpha_bounds: None,
            _subscriptions: vec![editor_subscription],
        }
    }

    /// The picker's current working color.
    pub(crate) fn color(&self) -> FantaColor {
        hsv_to_rgb(self.hsva)
    }

    fn set_hsva(&mut self, hsva: Hsva, window: &mut Window, cx: &mut Context<Self>) {
        self.update_hsva(hsva, true, window, cx);
    }

    fn update_hsva(
        &mut self,
        hsva: Hsva,
        sync_hex_editor: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.hsva == hsva {
            return;
        }
        self.hsva = hsva;
        if sync_hex_editor {
            let hex = self.color().to_hex();
            self.hex_editor.update(cx, |editor, cx| {
                editor.set_text(hex, window, cx);
            });
        }
        cx.emit(ColorPickerEvent::Changed(self.color()));
        cx.notify();
    }

    fn apply_hex_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.hex_editor.read(cx).text(cx);
        let trimmed = text.trim();
        let parsed =
            FantaColor::from_hex(trimmed).or_else(|| FantaColor::from_hex(&format!("#{trimmed}")));
        if let Some(color) = parsed {
            // Keep the user's cursor and partially-entered spelling intact.
            // Pointer-driven changes still synchronize the canonical hex text.
            self.update_hsva(rgb_to_hsv(color), false, window, cx);
        }
    }

    fn apply_position(
        &mut self,
        region: PickerRegion,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let fraction_x = |bounds: Bounds<Pixels>| {
            let width = f32::from(bounds.size.width).max(1.0);
            (f32::from(position.x - bounds.left()) / width).clamp(0.0, 1.0)
        };
        let mut hsva = self.hsva;
        match region {
            PickerRegion::SaturationValue => {
                let Some(bounds) = self.sv_bounds else { return };
                let height = f32::from(bounds.size.height).max(1.0);
                let y = (f32::from(position.y - bounds.top()) / height).clamp(0.0, 1.0);
                hsva.s = fraction_x(bounds);
                hsva.v = 1.0 - y;
            }
            PickerRegion::Hue => {
                let Some(bounds) = self.hue_bounds else {
                    return;
                };
                // 359.999 keeps a full-right drag from wrapping back to red 0°.
                hsva.h = (fraction_x(bounds) * 360.0).min(359.999);
            }
            PickerRegion::Alpha => {
                let Some(bounds) = self.alpha_bounds else {
                    return;
                };
                hsva.a = fraction_x(bounds);
            }
        }
        self.set_hsva(hsva, window, cx);
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.keystroke.key.as_str() {
            "enter" => {
                cx.stop_propagation();
                self.apply_hex_text(window, cx);
                cx.emit(ColorPickerEvent::Commit);
            }
            "escape" => {
                cx.stop_propagation();
                cx.emit(ColorPickerEvent::Cancel);
            }
            _ => {}
        }
    }

    /// A transparent overlay that records its bounds into the picker each
    /// frame, so mouse positions can map back to region fractions.
    fn bounds_probe(
        &self,
        set: fn(&mut Self, Bounds<Pixels>),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let this = cx.weak_entity();
        canvas(
            move |bounds, _, cx| {
                this.update(cx, |this, _| set(this, bounds)).log_err();
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }

    fn region_handlers(
        &self,
        target: gpui::Stateful<gpui::Div>,
        region: PickerRegion,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        target
            .on_drag(PickerDrag(region), |drag, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| drag.clone())
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.apply_position(region, event.position, window, cx);
                }),
            )
    }
}

impl Render for ColorPicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        let hsva = self.hsva;
        let hue_rgba = fanta_to_rgba(hsv_to_rgb(Hsva {
            h: hsva.h,
            s: 1.0,
            v: 1.0,
            a: 1.0,
        }));
        let opaque = {
            let mut color = self.color();
            color.a = 255;
            fanta_to_rgba(color)
        };
        let current = fanta_to_rgba(self.color());
        let white: Hsla = gpui::white();
        let black: Hsla = gpui::black();
        let transparent_white = white.opacity(0.0);
        let transparent_black = black.opacity(0.0);

        let sv_square = self.region_handlers(
            div()
                .id("fanta-color-sv")
                .debug_selector(|| "fanta-color-sv".to_owned())
                .relative()
                .w_full()
                .h(px(132.))
                .rounded_sm()
                .overflow_hidden()
                .cursor_crosshair()
                .bg(hue_rgba)
                .child(div().absolute().inset_0().bg(linear_gradient(
                    270.,
                    linear_color_stop(transparent_white, 0.),
                    linear_color_stop(white, 1.),
                )))
                .child(div().absolute().inset_0().bg(linear_gradient(
                    0.,
                    linear_color_stop(black, 0.),
                    linear_color_stop(transparent_black, 1.),
                )))
                .child(self.bounds_probe(|this, bounds| this.sv_bounds = Some(bounds), cx))
                .child(
                    div()
                        .absolute()
                        .left(gpui::relative(hsva.s))
                        .top(gpui::relative(1.0 - hsva.v))
                        .ml(px(-5.))
                        .mt(px(-5.))
                        .size(px(10.))
                        .rounded_full()
                        .border_2()
                        .border_color(white)
                        .shadow_sm(),
                ),
            PickerRegion::SaturationValue,
            cx,
        );

        // The hue rainbow as six two-stop gradient segments (gpui gradients
        // carry two stops each).
        let hue_segments = [
            (0.0_f32, 60.0_f32),
            (60.0, 120.0),
            (120.0, 180.0),
            (180.0, 240.0),
            (240.0, 300.0),
            (300.0, 360.0),
        ];
        let mut hue_strip_fill = h_flex().absolute().inset_0();
        for (from, to) in hue_segments {
            let from_color = fanta_to_rgba(hsv_to_rgb(Hsva {
                h: from,
                s: 1.0,
                v: 1.0,
                a: 1.0,
            }));
            let to_color = fanta_to_rgba(hsv_to_rgb(Hsva {
                h: to.min(359.999),
                s: 1.0,
                v: 1.0,
                a: 1.0,
            }));
            hue_strip_fill = hue_strip_fill.child(div().flex_1().h_full().bg(linear_gradient(
                270.,
                linear_color_stop(to_color, 0.),
                linear_color_stop(from_color, 1.),
            )));
        }
        let hue_marker = |fraction: f32| {
            div()
                .absolute()
                .left(gpui::relative(fraction))
                .top(px(-1.))
                .ml(px(-3.))
                .w(px(6.))
                .h(px(12.))
                .rounded_sm()
                .border_2()
                .border_color(white)
                .shadow_sm()
        };
        let hue_strip = self.region_handlers(
            div()
                .id("fanta-color-hue")
                .debug_selector(|| "fanta-color-hue".to_owned())
                .relative()
                .w_full()
                .h(px(10.))
                .rounded_sm()
                .cursor_ew_resize()
                .child(hue_strip_fill)
                .child(self.bounds_probe(|this, bounds| this.hue_bounds = Some(bounds), cx))
                .child(hue_marker(hsva.h / 360.0)),
            PickerRegion::Hue,
            cx,
        );

        let alpha_strip = self.region_handlers(
            div()
                .id("fanta-color-alpha")
                .debug_selector(|| "fanta-color-alpha".to_owned())
                .relative()
                .w_full()
                .h(px(10.))
                .rounded_sm()
                .cursor_ew_resize()
                .bg(colors.element_background)
                .child(div().absolute().inset_0().rounded_sm().bg(linear_gradient(
                    270.,
                    linear_color_stop(opaque, 0.),
                    linear_color_stop(Rgba { a: 0.0, ..opaque }, 1.),
                )))
                .child(self.bounds_probe(|this, bounds| this.alpha_bounds = Some(bounds), cx))
                .child(hue_marker(hsva.a)),
            PickerRegion::Alpha,
            cx,
        );

        v_flex()
            .key_context("FantaColorPicker")
            .track_focus(&self.focus_handle)
            .occlude()
            .w(px(216.))
            .p_2()
            .gap_2()
            .rounded_md()
            .border_1()
            .border_color(colors.border)
            .bg(colors.elevated_surface_background)
            .shadow_lg()
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| {
                cx.emit(ColorPickerEvent::Commit);
            }))
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<PickerDrag>, window, cx| {
                    let region = event.drag(cx).0;
                    this.apply_position(region, event.event.position, window, cx);
                }),
            )
            .child(sv_square)
            .child(hue_strip)
            .child(alpha_strip)
            .child(
                h_flex()
                    .gap_1p5()
                    .items_center()
                    .child(
                        div()
                            .size(px(18.))
                            .flex_none()
                            .rounded_sm()
                            .border_1()
                            .border_color(colors.border)
                            .bg(current),
                    )
                    .child(div().flex_1().min_w_0().child(self.hex_editor.clone())),
            )
    }
}

// =============================================================================
// Gradient editor popover
// =============================================================================

/// The number of segments used to approximate a multi-stop gradient as a strip
/// of gpui two-stop `linear_gradient`s (gpui gradients carry only two stops).
const GRADIENT_PREVIEW_SEGMENTS: usize = 24;

/// Build a horizontal preview strip approximating `gradient` as
/// [`GRADIENT_PREVIEW_SEGMENTS`] side-by-side two-stop linear gradients. Used
/// both for the editor's preview bar and the inspector's fill swatch, so the
/// swatch preview matches the editor exactly.
pub(crate) fn gradient_preview_strip(gradient: &Gradient) -> Div {
    let mut strip = h_flex().size_full().overflow_hidden();
    for segment in 0..GRADIENT_PREVIEW_SEGMENTS {
        let from = segment as f32 / GRADIENT_PREVIEW_SEGMENTS as f32;
        let to = (segment + 1) as f32 / GRADIENT_PREVIEW_SEGMENTS as f32;
        let from_color: Hsla = fanta_to_rgba(sample_gradient_color(gradient, from)).into();
        let to_color: Hsla = fanta_to_rgba(sample_gradient_color(gradient, to)).into();
        strip = strip.child(div().flex_1().h_full().bg(linear_gradient(
            90.,
            linear_color_stop(from_color, 0.),
            linear_color_stop(to_color, 1.),
        )));
    }
    strip
}

/// What the gradient editor tells its owner. Mirrors [`ColorPickerEvent`] so the
/// panel drives both with the same preview-then-commit staging.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum GradientEditorEvent {
    /// The working gradient changed. Preview only — no undo op.
    Changed(Gradient),
    /// Close, keeping the current gradient (owner commits one op).
    Commit,
    /// Close, restoring the pre-open gradient (no op).
    Cancel,
}

pub(crate) struct GradientEditor {
    gradient: Gradient,
    /// Index (into the position-sorted stop list) of the stop currently being
    /// edited by the embedded color picker, if the picker is open.
    editing_stop: Option<usize>,
    stop_picker: Option<Entity<ColorPicker>>,
    focus_handle: FocusHandle,
    preview_bounds: Option<Bounds<Pixels>>,
    angle_bounds: Option<Bounds<Pixels>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<GradientEditorEvent> for GradientEditor {}

impl Focusable for GradientEditor {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GradientDragTarget {
    /// Dragging the stop at this position-sorted index along the preview bar.
    Stop(usize),
    /// Dragging the angle wheel.
    Angle,
}

/// Drag payload for an in-flight gradient gesture; renders nothing.
#[derive(Clone)]
struct GradientDrag(GradientDragTarget);

impl Render for GradientDrag {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

const GRADIENT_KINDS: [GradientKind; 4] = [
    GradientKind::Linear,
    GradientKind::Radial,
    GradientKind::Angular,
    GradientKind::Diamond,
];

impl GradientEditor {
    pub(crate) fn new(initial: Gradient, cx: &mut Context<Self>) -> Self {
        Self {
            gradient: initial,
            editing_stop: None,
            stop_picker: None,
            focus_handle: cx.focus_handle(),
            preview_bounds: None,
            angle_bounds: None,
            _subscriptions: Vec::new(),
        }
    }

    /// The editor's current working gradient.
    pub(crate) fn gradient(&self) -> Gradient {
        self.gradient.clone()
    }

    fn emit_changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(GradientEditorEvent::Changed(self.gradient.clone()));
        cx.notify();
    }

    pub(crate) fn set_kind(&mut self, kind: GradientKind, cx: &mut Context<Self>) {
        if GradientKind::of(&self.gradient) == kind {
            return;
        }
        self.gradient = convert_gradient_kind(&self.gradient, kind);
        self.emit_changed(cx);
    }

    fn add_stop(&mut self, position: f32, window: &mut Window, cx: &mut Context<Self>) {
        let index = insert_stop_at(&mut self.gradient, position);
        self.emit_changed(cx);
        self.open_stop_picker(index, window, cx);
    }

    fn remove_stop_at(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.editing_stop == Some(index) {
            self.close_stop_picker(cx);
        }
        if remove_stop(&mut self.gradient, index) {
            self.emit_changed(cx);
        }
    }

    fn move_preview_stop(
        &mut self,
        index: usize,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> usize {
        let Some(bounds) = self.preview_bounds else {
            return index;
        };
        let width = f32::from(bounds.size.width).max(1.0);
        let fraction = (f32::from(position.x - bounds.left()) / width).clamp(0.0, 1.0);
        let new_index = move_stop(&mut self.gradient, index, fraction);
        self.emit_changed(cx);
        new_index
    }

    fn apply_angle(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(bounds) = self.angle_bounds else {
            return;
        };
        let center = bounds.center();
        let dx = f32::from(position.x - center.x);
        let dy = f32::from(position.y - center.y);
        if dx.abs() < f32::EPSILON && dy.abs() < f32::EPSILON {
            return;
        }
        let degrees = dy.atan2(dx).to_degrees().rem_euclid(360.0);
        self.gradient = set_gradient_angle(&self.gradient, degrees);
        self.emit_changed(cx);
    }

    pub(crate) fn open_stop_picker(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing_stop == Some(index)
            && let Some(picker) = self.stop_picker.as_ref()
        {
            picker.read(cx).focus_handle(cx).focus(window, cx);
            return;
        }
        let Some(stop) = gradient_stops(&self.gradient).get(index).copied() else {
            return;
        };
        let picker = cx.new(|cx| ColorPicker::new(stop.color, window, cx));
        let subscription = cx.subscribe(
            &picker,
            move |this, picker, event: &ColorPickerEvent, cx| {
                match event {
                    ColorPickerEvent::Changed(color) => {
                        if let Some(index) = this.editing_stop {
                            set_stop_color(&mut this.gradient, index, *color);
                            this.emit_changed(cx);
                        }
                    }
                    ColorPickerEvent::Commit => {
                        // Adopt the final color, then close the sub-picker
                        // without collapsing the whole gradient editor.
                        if let Some(index) = this.editing_stop {
                            let color = picker.read(cx).color();
                            set_stop_color(&mut this.gradient, index, color);
                            this.emit_changed(cx);
                        }
                        this.defer_close_stop_picker(picker, cx);
                    }
                    ColorPickerEvent::Cancel => this.defer_close_stop_picker(picker, cx),
                }
            },
        );
        picker.read(cx).focus_handle(cx).focus(window, cx);
        self.editing_stop = Some(index);
        self.stop_picker = Some(picker);
        self._subscriptions = vec![subscription];
        cx.notify();
    }

    fn defer_close_stop_picker(&self, closing_picker: Entity<ColorPicker>, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        cx.defer(move |cx| {
            this.update(cx, |this, cx| {
                if this
                    .stop_picker
                    .as_ref()
                    .is_some_and(|picker| picker.entity_id() == closing_picker.entity_id())
                {
                    this.close_stop_picker(cx);
                }
            })
            .log_err();
        });
    }

    fn close_stop_picker(&mut self, cx: &mut Context<Self>) {
        self.editing_stop = None;
        self.stop_picker = None;
        self._subscriptions.clear();
        cx.notify();
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.keystroke.key.as_str() {
            "enter" => {
                cx.stop_propagation();
                cx.emit(GradientEditorEvent::Commit);
            }
            "escape" => {
                cx.stop_propagation();
                if self.stop_picker.is_some() {
                    self.close_stop_picker(cx);
                } else {
                    cx.emit(GradientEditorEvent::Cancel);
                }
            }
            _ => {}
        }
    }

    fn preview_bounds_probe(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.weak_entity();
        canvas(
            move |bounds, _, cx| {
                this.update(cx, |this, _| this.preview_bounds = Some(bounds))
                    .log_err();
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }

    fn angle_bounds_probe(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.weak_entity();
        canvas(
            move |bounds, _, cx| {
                this.update(cx, |this, _| this.angle_bounds = Some(bounds))
                    .log_err();
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }

    fn render_type_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        let current = GradientKind::of(&self.gradient);
        let mut row = h_flex().w_full().gap_1();
        for kind in GRADIENT_KINDS {
            let selected = kind == current;
            let (bg, text_color) = if selected {
                (colors.element_selected, Color::Default)
            } else {
                (colors.element_background, Color::Muted)
            };
            row = row.child(
                div()
                    .id(("fanta-gradient-kind", kind as usize))
                    .flex_1()
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .bg(bg)
                    .cursor_pointer()
                    .hover(|style| style.bg(colors.element_hover))
                    .child(
                        Label::new(kind.label())
                            .size(LabelSize::XSmall)
                            .color(text_color),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.set_kind(kind, cx))),
            );
        }
        row
    }

    fn render_preview_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        let mut stops_overlay = div().absolute().inset_0();
        // The visual stop markers live over the preview bar so a drag on one
        // repositions it; a click on empty bar space adds a stop.
        for (index, stop) in gradient_stops(&self.gradient).iter().enumerate() {
            let selected = self.editing_stop == Some(index);
            // Imported gradients can carry positions outside 0..1 (or NaN);
            // clamp so the marker stays on the bar (`clamp` also maps NaN → 0).
            let marker_fraction = if stop.position.is_finite() {
                stop.position.clamp(0.0, 1.0)
            } else {
                0.0
            };
            let marker = div()
                .id(("fanta-gradient-stop-marker", index))
                .debug_selector(move || format!("fanta-gradient-stop-marker-{index}"))
                .absolute()
                .left(gpui::relative(marker_fraction))
                .top(px(-3.))
                .ml(px(-6.))
                .w(px(12.))
                .h(px(24.))
                .rounded_sm()
                .border_2()
                .border_color(if selected {
                    colors.text_accent
                } else {
                    gpui::white()
                })
                .bg(fanta_to_rgba(stop.color))
                .shadow_sm()
                .cursor_pointer()
                .on_drag(
                    GradientDrag(GradientDragTarget::Stop(index)),
                    |drag, _, _, cx| {
                        cx.stop_propagation();
                        cx.new(|_| drag.clone())
                    },
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.open_stop_picker(index, window, cx);
                    }),
                );
            stops_overlay = stops_overlay.child(marker);
        }
        div()
            .id("fanta-gradient-preview")
            .relative()
            .w_full()
            .h(px(18.))
            .rounded_sm()
            .border_1()
            .border_color(colors.border)
            .overflow_hidden()
            .bg(colors.element_background)
            .cursor_crosshair()
            .child(gradient_preview_strip(&self.gradient).absolute().inset_0())
            .child(self.preview_bounds_probe(cx))
            .child(stops_overlay)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    if let Some(bounds) = this.preview_bounds {
                        let width = f32::from(bounds.size.width).max(1.0);
                        let fraction =
                            (f32::from(event.position.x - bounds.left()) / width).clamp(0.0, 1.0);
                        this.add_stop(fraction, window, cx);
                    }
                }),
            )
    }

    fn render_stop_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        let mut list = v_flex().w_full().gap_1();
        let stop_count = gradient_stops(&self.gradient).len();
        for (index, stop) in gradient_stops(&self.gradient).iter().enumerate() {
            let percent = (stop.position * 100.0).round() as i32;
            let removable = stop_count > 2;
            let row = h_flex()
                .w_full()
                .gap_1p5()
                .items_center()
                .child(
                    div()
                        .id(("fanta-gradient-stop-swatch", index))
                        .size(px(18.))
                        .flex_none()
                        .rounded_sm()
                        .border_1()
                        .border_color(colors.border)
                        .bg(fanta_to_rgba(stop.color))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_stop_picker(index, window, cx);
                        })),
                )
                .child(
                    div().flex_1().min_w_0().child(
                        Label::new(stop.color.to_hex().trim_start_matches('#').to_string())
                            .size(LabelSize::Small),
                    ),
                )
                .child(
                    Label::new(format!("{percent}%"))
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .child(
                    IconButton::new(("fanta-gradient-stop-remove", index), IconName::Close)
                        .icon_size(IconSize::XSmall)
                        .disabled(!removable)
                        .tooltip(Tooltip::text("Remove Stop"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.remove_stop_at(index, cx);
                        })),
                );
            list = list.child(row);
        }
        list
    }

    /// The angle wheel for linear / angular gradients: a small dial whose handle
    /// points along the gradient axis and drags to rotate it.
    fn render_angle_control(&self, degrees: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        // Guard against non-finite geometry from a corrupt import so the dial
        // handle never receives a NaN offset.
        let degrees = if degrees.is_finite() { degrees } else { 0.0 };
        let radians = degrees.to_radians();
        // Handle position on a unit circle within the 28px dial.
        let handle_x = 0.5 + 0.5 * radians.cos();
        let handle_y = 0.5 + 0.5 * radians.sin();
        h_flex()
            .w_full()
            .gap_2()
            .items_center()
            .child(
                Label::new("Angle")
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
            .child(
                div()
                    .id("fanta-gradient-angle")
                    .relative()
                    .size(px(28.))
                    .flex_none()
                    .rounded_full()
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.element_background)
                    .cursor_pointer()
                    .child(self.angle_bounds_probe(cx))
                    .child(
                        div()
                            .absolute()
                            .left(gpui::relative(handle_x))
                            .top(gpui::relative(handle_y))
                            .ml(px(-3.))
                            .mt(px(-3.))
                            .size(px(6.))
                            .rounded_full()
                            .bg(colors.text_accent),
                    )
                    .on_drag(GradientDrag(GradientDragTarget::Angle), |drag, _, _, cx| {
                        cx.stop_propagation();
                        cx.new(|_| drag.clone())
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.apply_angle(event.position, cx);
                        }),
                    ),
            )
            .child(
                Label::new(format!("{}°", degrees.round() as i32))
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
    }
}

#[cfg(test)]
impl GradientEditor {
    pub(crate) fn stop_picker_for_test(&self) -> Option<Entity<ColorPicker>> {
        self.stop_picker.clone()
    }
}

#[cfg(test)]
impl ColorPicker {
    /// Drive the working color from a test as if the user moved the picker,
    /// emitting the same `Changed` event a real drag would.
    pub(crate) fn set_test_color(
        &mut self,
        color: FantaColor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_hsva(rgb_to_hsv(color), window, cx);
    }

    /// Emit the `Commit` event a click-outside / Enter would, so a test can
    /// exercise the owner's commit handling.
    pub(crate) fn commit_for_test(&mut self, cx: &mut Context<Self>) {
        cx.emit(ColorPickerEvent::Commit);
    }
}

impl Render for GradientEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        let angle = gradient_angle_degrees(&self.gradient);
        let body = v_flex()
            .key_context("FantaGradientEditor")
            .track_focus(&self.focus_handle)
            .occlude()
            .w(px(224.))
            .p_2()
            .gap_2()
            .rounded_md()
            .border_1()
            .border_color(colors.border)
            .bg(colors.elevated_surface_background)
            .shadow_lg()
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                // A click outside collapses the whole editor (committing),
                // unless a stop sub-picker is capturing that click first.
                if this.stop_picker.is_none() {
                    cx.emit(GradientEditorEvent::Commit);
                }
            }))
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<GradientDrag>, _, cx| {
                    match event.drag(cx).0 {
                        GradientDragTarget::Stop(index) => {
                            // The drag payload's index is fixed at gesture start;
                            // once a stop is dragged across a neighbor the list
                            // re-sorts and that index points at the neighbor.
                            // Track the live index in `editing_stop` (seeded by
                            // the marker's mouse-down `open_stop_picker`) so every
                            // move follows the same stop rather than grabbing its
                            // neighbor.
                            let current = this.editing_stop.unwrap_or(index);
                            let new_index =
                                this.move_preview_stop(current, event.event.position, cx);
                            this.editing_stop = Some(new_index);
                        }
                        GradientDragTarget::Angle => this.apply_angle(event.event.position, cx),
                    }
                }),
            )
            .child(self.render_type_row(cx))
            .child(self.render_preview_bar(cx))
            .when_some(angle, |this, degrees| {
                this.child(self.render_angle_control(degrees, cx))
            })
            .child(self.render_stop_list(cx));

        // Keep the nested picker in the same deferred subtree as the gradient
        // editor. A second deferred/anchored subtree can move the focused
        // picker's dispatch node outside the editor's reused node range, which
        // makes GPUI panic on the next redraw after a color drag.
        if let Some(picker) = &self.stop_picker {
            body.child(picker.clone())
        } else {
            body
        }
    }
}

#[cfg(test)]
mod render_tests {
    //! Headless window/render coverage for the gradient popover. These draw the
    //! same elements the properties panel mounts when a selected node has a
    //! gradient fill, so a paint-time panic in the gradient path (the crash
    //! reported when opening gradients) reproduces here instead of in the app.
    use super::*;
    use gpui::TestAppContext;

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            zlog::init_test();
            assets::Assets.load_test_fonts(cx);
            let store = settings::SettingsStore::test(cx);
            cx.set_global(store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            release_channel::init(semver::Version::new(0, 0, 0), cx);
            editor::init(cx);
        });
    }

    /// Every gradient kind plus the degenerate shapes a real imported `.fig`
    /// can carry: no stops, a single stop, and duplicate-position stops.
    fn gradient_fixtures() -> Vec<(&'static str, Gradient)> {
        let stop = |position: f32, color: FantaColor| GradientStop { position, color };
        vec![
            (
                "linear_two_stops",
                Gradient::Linear {
                    start: [0.0, 0.0],
                    end: [1.0, 0.0],
                    stops: vec![stop(0.0, FantaColor::BLACK), stop(1.0, FantaColor::WHITE)],
                },
            ),
            (
                "linear_zero_stops",
                Gradient::Linear {
                    start: [0.5, 0.0],
                    end: [0.5, 1.0],
                    stops: vec![],
                },
            ),
            (
                "linear_one_stop",
                Gradient::Linear {
                    start: [0.5, 0.0],
                    end: [0.5, 1.0],
                    stops: vec![stop(0.5, FantaColor::rgb(10, 20, 30))],
                },
            ),
            (
                "linear_degenerate_axis",
                Gradient::Linear {
                    start: [0.5, 0.5],
                    end: [0.5, 0.5],
                    stops: vec![stop(0.0, FantaColor::BLACK), stop(1.0, FantaColor::WHITE)],
                },
            ),
            (
                "radial_duplicate_positions",
                Gradient::Radial {
                    center: [0.5, 0.5],
                    radius: 0.5,
                    handles: None,
                    stops: vec![
                        stop(0.5, FantaColor::BLACK),
                        stop(0.5, FantaColor::WHITE),
                        stop(0.5, FantaColor::rgb(1, 2, 3)),
                    ],
                },
            ),
            (
                "angular_normal",
                Gradient::Angular {
                    center: [0.5, 0.5],
                    start_angle: 0.0,
                    stops: vec![
                        stop(0.0, FantaColor::rgb(200, 30, 30)),
                        stop(1.0, FantaColor::rgb(30, 30, 200)),
                    ],
                },
            ),
            (
                "diamond_one_stop",
                Gradient::Diamond {
                    center: [0.5, 0.5],
                    radius: 0.5,
                    handles: None,
                    stops: vec![stop(0.0, FantaColor::rgb(90, 90, 90))],
                },
            ),
            (
                // A real imported `.fig` can carry stop positions outside 0..1.
                "linear_out_of_range_positions",
                Gradient::Linear {
                    start: [0.0, 0.0],
                    end: [1.0, 0.0],
                    stops: vec![stop(-1.0, FantaColor::BLACK), stop(2.0, FantaColor::WHITE)],
                },
            ),
            (
                // NaN stop positions / geometry from a corrupt import.
                "linear_nan_positions_and_axis",
                Gradient::Linear {
                    start: [f32::NAN, f32::NAN],
                    end: [f32::NAN, f32::NAN],
                    stops: vec![
                        stop(f32::NAN, FantaColor::BLACK),
                        stop(1.0, FantaColor::WHITE),
                    ],
                },
            ),
        ]
    }

    #[gpui::test]
    fn gradient_preview_strip_draws_for_every_kind_and_degenerate(cx: &mut TestAppContext) {
        init_test(cx);
        for (name, gradient) in gradient_fixtures() {
            struct StripRoot(Gradient);
            impl Render for StripRoot {
                fn render(
                    &mut self,
                    _window: &mut Window,
                    _cx: &mut Context<Self>,
                ) -> impl IntoElement {
                    div()
                        .size_full()
                        .child(gradient_preview_strip(&self.0).h(px(18.)))
                }
            }
            let window = cx.add_window(|_, _| StripRoot(gradient.clone()));
            // Drawing exercises the real paint path (layout + gradient packing),
            // where a paint-time panic in the gradient render would surface.
            cx.update_window(window.into(), |_, window, cx| {
                window.draw(cx).clear();
            })
            .unwrap_or_else(|error| panic!("drawing swatch strip for {name} failed: {error:#}"));
        }
    }

    #[gpui::test]
    fn gradient_editor_opens_and_draws_for_every_kind(cx: &mut TestAppContext) {
        init_test(cx);
        for (name, gradient) in gradient_fixtures() {
            let window = cx.add_window(|_, cx| GradientEditor::new(gradient.clone(), cx));
            // Draw the editor popover (type row, preview bar, angle wheel, stops).
            cx.update_window(window.into(), |_, window, cx| {
                window.draw(cx).clear();
            })
            .unwrap_or_else(|error| panic!("drawing gradient editor for {name} failed: {error:#}"));
            // Open the nested stop color picker on the first stop and redraw,
            // exercising the anchored sub-popover path.
            window
                .update(cx, |editor, window, cx| {
                    editor.open_stop_picker(0, window, cx);
                })
                .unwrap();
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                window.draw(cx).clear();
            })
            .unwrap_or_else(|error| {
                panic!("drawing gradient editor + stop picker for {name} failed: {error:#}")
            });

            // Committing the stop sub-picker drops the subscription whose
            // callback is mid-flush (`close_stop_picker` clears `_subscriptions`
            // from inside its own event handler) — a re-entrant drop would panic.
            let picker = window
                .read_with(cx, |editor, _| editor.stop_picker_for_test())
                .unwrap();
            if let Some(picker) = picker {
                picker.update(cx, |picker, cx| picker.commit_for_test(cx));
                cx.run_until_parked();
                cx.update_window(window.into(), |_, window, cx| {
                    window.draw(cx).clear();
                })
                .unwrap_or_else(|error| {
                    panic!("drawing after stop-picker commit for {name} failed: {error:#}")
                });
            }
        }
    }

    #[gpui::test]
    fn valid_hex_input_updates_the_picker_before_blur(cx: &mut TestAppContext) {
        init_test(cx);
        let window = cx.add_window(|window, cx| ColorPicker::new(FantaColor::BLACK, window, cx));
        window
            .update(cx, |picker, window, cx| {
                let editor = picker.hex_editor.clone();
                editor.update(cx, |editor, cx| {
                    editor.set_text("#123456", window, cx);
                });
            })
            .unwrap();
        cx.run_until_parked();
        let color = window.read_with(cx, |picker, _| picker.color()).unwrap();
        assert_eq!(color, FantaColor::rgb(0x12, 0x34, 0x56));
    }

    #[gpui::test]
    fn stale_stop_picker_close_does_not_close_its_replacement(cx: &mut TestAppContext) {
        init_test(cx);
        let gradient = Gradient::Linear {
            start: [0.0, 0.0],
            end: [1.0, 0.0],
            stops: vec![
                GradientStop {
                    position: 0.0,
                    color: FantaColor::BLACK,
                },
                GradientStop {
                    position: 1.0,
                    color: FantaColor::WHITE,
                },
            ],
        };
        let window = cx.add_window(|_, cx| GradientEditor::new(gradient, cx));
        let first_picker = window
            .update(cx, |editor, window, cx| {
                editor.open_stop_picker(0, window, cx);
                editor
                    .stop_picker_for_test()
                    .expect("first stop picker is open")
            })
            .expect("open first stop picker");

        let replacement = window
            .update(cx, |editor, window, cx| {
                editor.defer_close_stop_picker(first_picker.clone(), cx);
                editor.open_stop_picker(1, window, cx);
                editor
                    .stop_picker_for_test()
                    .expect("replacement stop picker is open")
            })
            .expect("open replacement stop picker");
        cx.run_until_parked();

        let current = window
            .read_with(cx, |editor, _| {
                editor
                    .stop_picker_for_test()
                    .expect("stale close must not dismiss replacement")
            })
            .expect("read gradient editor");
        assert_eq!(current.entity_id(), replacement.entity_id());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_through_hsv() {
        let cases = [
            FantaColor::rgb(255, 0, 0),
            FantaColor::rgb(0, 255, 0),
            FantaColor::rgb(0, 0, 255),
            FantaColor::rgb(255, 255, 255),
            FantaColor::rgb(0, 0, 0),
            FantaColor::rgb(128, 128, 128),
            FantaColor::rgb(12, 34, 56),
            FantaColor::rgb(217, 217, 217),
            FantaColor::rgba(255, 128, 0, 64),
            FantaColor::rgba(1, 2, 3, 0),
        ];
        for color in cases {
            let round_tripped = hsv_to_rgb(rgb_to_hsv(color));
            assert_eq!(round_tripped, color, "round-trip failed for {color:?}");
        }
    }

    #[test]
    fn hsv_conversion_matches_known_values() {
        let hsva = rgb_to_hsv(FantaColor::rgb(255, 0, 0));
        assert!((hsva.h - 0.0).abs() < 1e-3);
        assert!((hsva.s - 1.0).abs() < 1e-6);
        assert!((hsva.v - 1.0).abs() < 1e-6);

        let hsva = rgb_to_hsv(FantaColor::rgb(0, 255, 255));
        assert!((hsva.h - 180.0).abs() < 1e-3);

        let hsva = rgb_to_hsv(FantaColor::rgb(128, 128, 128));
        assert!((hsva.s - 0.0).abs() < 1e-6);
        assert!((hsva.v - 128.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn full_hue_sweep_round_trips() {
        for h in 0..360 {
            let hsva = Hsva {
                h: h as f32,
                s: 1.0,
                v: 1.0,
                a: 1.0,
            };
            let color = hsv_to_rgb(hsva);
            let back = rgb_to_hsv(color);
            let error = (back.h - hsva.h).abs().min((back.h - hsva.h + 360.0).abs());
            assert!(error < 2.0, "hue {h} drifted to {}", back.h);
        }
    }

    // --- Gradient helpers -------------------------------------------------

    fn two_stop_linear() -> Gradient {
        Gradient::Linear {
            start: [0.0, 0.0],
            end: [1.0, 0.0],
            stops: vec![
                GradientStop {
                    position: 0.0,
                    color: FantaColor::BLACK,
                },
                GradientStop {
                    position: 1.0,
                    color: FantaColor::WHITE,
                },
            ],
        }
    }

    #[test]
    fn seed_gradient_from_color_makes_color_to_transparent_pair() {
        let color = FantaColor::rgb(0x3F, 0xA9, 0xF5);
        let gradient = seed_gradient_from_color(color);
        let stops = gradient_stops(&gradient);
        assert_eq!(stops.len(), 2);
        assert_eq!(stops[0].position, 0.0);
        assert_eq!(stops[0].color, color);
        assert_eq!(stops[1].position, 1.0);
        // Same RGB, zero alpha — Figma's solid→gradient default.
        assert_eq!(stops[1].color, FantaColor::rgba(0x3F, 0xA9, 0xF5, 0));
        assert_eq!(GradientKind::of(&gradient), GradientKind::Linear);
    }

    #[test]
    fn representative_color_is_lowest_position_stop() {
        let gradient = Gradient::Radial {
            center: [0.5, 0.5],
            radius: 0.5,
            handles: None,
            stops: vec![
                GradientStop {
                    position: 0.8,
                    color: FantaColor::WHITE,
                },
                GradientStop {
                    position: 0.2,
                    color: FantaColor::rgb(10, 20, 30),
                },
            ],
        };
        assert_eq!(
            representative_gradient_color(&gradient),
            FantaColor::rgb(10, 20, 30)
        );
    }

    #[test]
    fn insert_stop_keeps_list_sorted_and_interpolates_color() {
        let mut gradient = two_stop_linear();
        let index = insert_stop_at(&mut gradient, 0.5);
        let stops = gradient_stops(&gradient);
        assert_eq!(stops.len(), 3);
        // The new stop lands between the two originals (position-sorted).
        assert_eq!(index, 1);
        assert!((stops[1].position - 0.5).abs() < 1e-6);
        // Halfway between black and white ⇒ mid-gray.
        assert_eq!(stops[1].color, FantaColor::rgb(128, 128, 128));
        // Positions are monotonic non-decreasing.
        assert!(stops[0].position <= stops[1].position);
        assert!(stops[1].position <= stops[2].position);
    }

    #[test]
    fn remove_stop_refuses_below_two() {
        let mut gradient = two_stop_linear();
        assert!(!remove_stop(&mut gradient, 0));
        assert_eq!(gradient_stops(&gradient).len(), 2);

        insert_stop_at(&mut gradient, 0.5);
        assert!(remove_stop(&mut gradient, 1));
        assert_eq!(gradient_stops(&gradient).len(), 2);
        // Out-of-range removal is a no-op.
        assert!(!remove_stop(&mut gradient, 9));
    }

    #[test]
    fn move_stop_reorders_and_reports_new_index() {
        // Three stops so a drag can cross a neighbor and change ordering.
        let mut gradient = two_stop_linear();
        insert_stop_at(&mut gradient, 0.5); // now black@0, gray@0.5, white@1
        // Drag the white stop (index 2, at 1.0) down below the gray stop.
        let new_index = move_stop(&mut gradient, 2, 0.25);
        let stops = gradient_stops(&gradient);
        assert_eq!(stops.len(), 3);
        // White now sits at 0.25, so it is the middle stop.
        assert_eq!(new_index, 1);
        assert_eq!(stops[1].color, FantaColor::WHITE);
        assert!((stops[1].position - 0.25).abs() < 1e-6);
        // Positions stay monotonic non-decreasing after the reorder.
        assert!(stops[0].position <= stops[1].position);
        assert!(stops[1].position <= stops[2].position);
        // Out-of-range clamps to 0..1.
        let clamped = move_stop(&mut gradient, 0, 5.0);
        assert!((gradient_stops(&gradient)[clamped].position - 1.0).abs() < 1e-6);
    }

    #[test]
    fn set_stop_color_leaves_position() {
        let mut gradient = two_stop_linear();
        set_stop_color(&mut gradient, 1, FantaColor::rgb(1, 2, 3));
        let stops = gradient_stops(&gradient);
        assert_eq!(stops[1].color, FantaColor::rgb(1, 2, 3));
        assert!((stops[1].position - 1.0).abs() < 1e-6);
    }

    #[test]
    fn sample_gradient_clamps_and_interpolates() {
        let gradient = two_stop_linear();
        assert_eq!(sample_gradient_color(&gradient, -1.0), FantaColor::BLACK);
        assert_eq!(sample_gradient_color(&gradient, 2.0), FantaColor::WHITE);
        assert_eq!(
            sample_gradient_color(&gradient, 0.5),
            FantaColor::rgb(128, 128, 128)
        );
    }

    #[test]
    fn linear_angle_round_trips_through_geometry() {
        // Every angle set on a linear gradient reads back within rounding.
        for degrees in [0.0_f32, 30.0, 45.0, 90.0, 135.0, 200.0, 315.0] {
            let gradient = set_gradient_angle(&two_stop_linear(), degrees);
            let back = gradient_angle_degrees(&gradient).expect("linear has an angle");
            let error = (back - degrees).abs().min((back - degrees + 360.0).abs());
            assert!(error < 0.5, "angle {degrees} drifted to {back}");
        }
    }

    #[test]
    fn angular_angle_round_trips() {
        let base = Gradient::Angular {
            center: [0.5, 0.5],
            start_angle: 0.0,
            stops: gradient_stops(&two_stop_linear()).to_vec(),
        };
        let rotated = set_gradient_angle(&base, 120.0);
        let back = gradient_angle_degrees(&rotated).expect("angular has an angle");
        assert!(
            (back - 120.0).abs() < 0.5,
            "angular angle drifted to {back}"
        );
    }

    #[test]
    fn radial_and_diamond_have_no_angle() {
        let radial = Gradient::Radial {
            center: [0.5, 0.5],
            radius: 0.5,
            handles: None,
            stops: gradient_stops(&two_stop_linear()).to_vec(),
        };
        assert!(gradient_angle_degrees(&radial).is_none());
    }

    #[test]
    fn convert_kind_preserves_stops_and_angle() {
        let linear = set_gradient_angle(&two_stop_linear(), 45.0);
        let original_stops = gradient_stops(&linear).to_vec();

        // Linear → angular keeps stops and maps the axis angle to start angle.
        let angular = convert_gradient_kind(&linear, GradientKind::Angular);
        assert_eq!(GradientKind::of(&angular), GradientKind::Angular);
        assert_eq!(gradient_stops(&angular), original_stops.as_slice());
        let angle = gradient_angle_degrees(&angular).expect("angular has an angle");
        assert!((angle - 45.0).abs() < 0.5, "converted angle is {angle}");

        // Angular → radial keeps stops, drops the angle.
        let radial = convert_gradient_kind(&angular, GradientKind::Radial);
        assert_eq!(GradientKind::of(&radial), GradientKind::Radial);
        assert_eq!(gradient_stops(&radial), original_stops.as_slice());
        assert!(gradient_angle_degrees(&radial).is_none());

        // Radial → diamond and back to linear keeps stops throughout.
        let diamond = convert_gradient_kind(&radial, GradientKind::Diamond);
        assert_eq!(GradientKind::of(&diamond), GradientKind::Diamond);
        let back_to_linear = convert_gradient_kind(&diamond, GradientKind::Linear);
        assert_eq!(GradientKind::of(&back_to_linear), GradientKind::Linear);
        assert_eq!(gradient_stops(&back_to_linear), original_stops.as_slice());

        // Converting to the same kind is a no-op clone.
        assert_eq!(convert_gradient_kind(&linear, GradientKind::Linear), linear);
    }
}
