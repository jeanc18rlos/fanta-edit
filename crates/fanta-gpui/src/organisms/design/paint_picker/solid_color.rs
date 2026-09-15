//! Color entry, conversion, and contrast helpers for the retained paint picker.

use gpui::{Bounds, Hsla, Pixels, Point, rgba};

use crate::color::{parse_hex_rgba, rgba_channels};

use super::super::{DesignColor, DesignColorContrastCategory, DesignColorContrastLevel};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Hsv {
    pub(super) hue: f32,
    pub(super) saturation: f32,
    pub(super) value: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Hsl {
    pub(super) hue: f32,
    pub(super) saturation: f32,
    pub(super) lightness: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ParsedColorInput {
    pub(super) color: DesignColor,
    pub(super) explicit_alpha: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum ColorFormat {
    #[default]
    Hex,
    Rgb,
    Css,
    Hsl,
    Hsb,
}

impl ColorFormat {
    pub(super) const ALL: [Self; 5] = [Self::Hex, Self::Rgb, Self::Css, Self::Hsl, Self::Hsb];

    pub(super) const fn index(self) -> usize {
        match self {
            Self::Hex => 0,
            Self::Rgb => 1,
            Self::Css => 2,
            Self::Hsl => 3,
            Self::Hsb => 4,
        }
    }

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Hex => "Hex",
            Self::Rgb => "RGB",
            Self::Css => "CSS",
            Self::Hsl => "HSL",
            Self::Hsb => "HSB",
        }
    }

    pub(super) const fn channel_labels(self) -> [&'static str; 3] {
        match self {
            Self::Hex | Self::Css => ["", "", ""],
            Self::Rgb => ["R", "G", "B"],
            Self::Hsl => ["H", "S", "L"],
            Self::Hsb => ["H", "S", "B"],
        }
    }
}

pub(super) fn rgb_hex(color: DesignColor) -> String {
    format!("{:02X}{:02X}{:02X}", color.red, color.green, color.blue)
}

pub(super) fn rgba_hex(color: DesignColor) -> String {
    if color.alpha == u8::MAX {
        rgb_hex(color)
    } else {
        format!(
            "{:02X}{:02X}{:02X}{:02X}",
            color.red, color.green, color.blue, color.alpha
        )
    }
}

pub(super) fn css_rgba(color: DesignColor) -> String {
    format!(
        "rgba({}, {}, {}, {})",
        color.red,
        color.green,
        color.blue,
        format_css_alpha(color.alpha)
    )
}

fn format_css_alpha(alpha: u8) -> String {
    let alpha = f32::from(alpha) / 255.;
    if alpha <= f32::EPSILON {
        "0".to_owned()
    } else if (alpha - 1.).abs() <= f32::EPSILON {
        "1".to_owned()
    } else {
        format!("{alpha:.3}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}

pub(super) fn color_channel_values(format: ColorFormat, color: DesignColor) -> [String; 3] {
    match format {
        ColorFormat::Hex | ColorFormat::Css => [String::new(), String::new(), String::new()],
        ColorFormat::Rgb => [
            color.red.to_string(),
            color.green.to_string(),
            color.blue.to_string(),
        ],
        ColorFormat::Hsl => {
            let hsl = rgb_to_hsl(color);
            [
                format_decimal(hsl.hue),
                format_decimal(hsl.saturation * 100.),
                format_decimal(hsl.lightness * 100.),
            ]
        }
        ColorFormat::Hsb => {
            let hsv = rgb_to_hsv(color);
            [
                format_decimal(hsv.hue),
                format_decimal(hsv.saturation * 100.),
                format_decimal(hsv.value * 100.),
            ]
        }
    }
}

pub(super) fn parse_color_channel_value(
    format: ColorFormat,
    channel: usize,
    input: &str,
) -> Option<f32> {
    if matches!(format, ColorFormat::Hex | ColorFormat::Css) || channel >= 3 {
        return None;
    }
    let value = input.trim().parse::<f32>().ok()?;
    if !value.is_finite() {
        return None;
    }
    let maximum = match format {
        ColorFormat::Rgb => 255.,
        ColorFormat::Hsl | ColorFormat::Hsb if channel == 0 => 360.,
        ColorFormat::Hsl | ColorFormat::Hsb => 100.,
        ColorFormat::Hex | ColorFormat::Css => return None,
    };
    (0. ..=maximum).contains(&value).then_some(value)
}

pub(super) fn parse_color_channels(format: ColorFormat, inputs: [&str; 3]) -> Option<DesignColor> {
    let values = [
        parse_color_channel_value(format, 0, inputs[0])?,
        parse_color_channel_value(format, 1, inputs[1])?,
        parse_color_channel_value(format, 2, inputs[2])?,
    ];
    match format {
        ColorFormat::Hex | ColorFormat::Css => None,
        ColorFormat::Rgb => Some(DesignColor::rgb(
            values[0].round() as u8,
            values[1].round() as u8,
            values[2].round() as u8,
        )),
        ColorFormat::Hsl => Some(hsl_to_color(
            Hsl {
                hue: values[0],
                saturation: values[1] / 100.,
                lightness: values[2] / 100.,
            },
            u8::MAX,
        )),
        ColorFormat::Hsb => Some(hsv_to_color(
            Hsv {
                hue: values[0],
                saturation: values[1] / 100.,
                value: values[2] / 100.,
            },
            u8::MAX,
        )),
    }
}

pub(super) fn parse_hex_color_input(input: &str) -> Option<ParsedColorInput> {
    let trimmed = input.trim();
    let hex = trimmed.strip_prefix('#').unwrap_or(trimmed);
    let explicit_alpha = matches!(hex.len(), 4 | 8);
    let [red, green, blue, alpha] = rgba_channels(parse_hex_rgba(hex)?);
    Some(ParsedColorInput {
        color: DesignColor::rgba(red, green, blue, alpha),
        explicit_alpha,
    })
}

#[cfg(test)]
pub(super) fn parse_hex_color(input: &str) -> Option<DesignColor> {
    parse_hex_color_input(input).map(|parsed| parsed.color)
}

pub(super) fn parse_css_color_input(input: &str) -> Option<ParsedColorInput> {
    let input = input.trim();
    let open = input.find('(')?;
    let close = input.rfind(')')?;
    if close != input.len().saturating_sub(1) || !input[..open].trim().eq_ignore_ascii_case("rgba")
    {
        return None;
    }
    let mut components = input[open + 1..close].split(',').map(str::trim);
    let red = parse_css_rgb_channel(components.next()?)?;
    let green = parse_css_rgb_channel(components.next()?)?;
    let blue = parse_css_rgb_channel(components.next()?)?;
    let alpha = components.next()?.parse::<f32>().ok()?;
    if components.next().is_some() || !alpha.is_finite() || !(0. ..=1.).contains(&alpha) {
        return None;
    }
    Some(ParsedColorInput {
        color: DesignColor::rgba(red, green, blue, channel_from_unit(alpha)),
        explicit_alpha: true,
    })
}

fn parse_css_rgb_channel(input: &str) -> Option<u8> {
    let value = input.parse::<f32>().ok()?;
    (value.is_finite() && (0. ..=255.).contains(&value)).then(|| value.round() as u8)
}

pub(super) fn parse_opacity(input: &str) -> Option<f32> {
    let input = input.trim().strip_suffix('%').unwrap_or(input.trim());
    let value = input.trim().parse::<f32>().ok()?;
    value.is_finite().then(|| value.clamp(0., 100.))
}

pub(super) fn parse_stop_position(input: &str) -> Option<f32> {
    parse_opacity(input).map(|position| position / 100.)
}

pub(super) fn format_decimal(value: f32) -> String {
    super::super::format::format_compact_number(value)
}

pub(super) fn fraction_x(bounds: Bounds<Pixels>, position: Point<Pixels>) -> f32 {
    let width = f32::from(bounds.size.width);
    if width <= 0. {
        return 0.;
    }
    (f32::from(position.x - bounds.origin.x) / width).clamp(0., 1.)
}

pub(super) fn fraction_y(bounds: Bounds<Pixels>, position: Point<Pixels>) -> f32 {
    let height = f32::from(bounds.size.height);
    if height <= 0. {
        return 0.;
    }
    (f32::from(position.y - bounds.origin.y) / height).clamp(0., 1.)
}

pub(super) fn color_to_hsla(color: DesignColor) -> Hsla {
    let encoded = u32::from_be_bytes([color.red, color.green, color.blue, color.alpha]);
    rgba(encoded).into()
}

pub(super) fn composite_color(foreground: DesignColor, background: DesignColor) -> DesignColor {
    let foreground_alpha = f32::from(foreground.alpha) / 255.;
    let background_alpha = f32::from(background.alpha) / 255.;
    let output_alpha = foreground_alpha + background_alpha * (1. - foreground_alpha);
    if output_alpha <= f32::EPSILON {
        return DesignColor::rgba(0, 0, 0, 0);
    }
    let channel = |foreground: u8, background: u8| {
        let value = (f32::from(foreground) * foreground_alpha
            + f32::from(background) * background_alpha * (1. - foreground_alpha))
            / output_alpha;
        value.clamp(0., 255.).round() as u8
    };
    DesignColor::rgba(
        channel(foreground.red, background.red),
        channel(foreground.green, background.green),
        channel(foreground.blue, background.blue),
        channel_from_unit(output_alpha),
    )
}

pub(super) fn opaque_color(color: DesignColor) -> DesignColor {
    composite_color(color, DesignColor::WHITE)
}

pub(super) fn wcag_relative_luminance(color: DesignColor) -> f32 {
    let linear = |channel: u8| {
        let encoded = f32::from(channel) / 255.;
        if encoded <= 0.04045 {
            encoded / 12.92
        } else {
            ((encoded + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.red) + 0.7152 * linear(color.green) + 0.0722 * linear(color.blue)
}

pub(super) fn wcag_contrast_ratio(foreground: DesignColor, background: DesignColor) -> f32 {
    let background = opaque_color(background);
    let foreground = composite_color(foreground, background);
    let foreground_luminance = wcag_relative_luminance(foreground);
    let background_luminance = wcag_relative_luminance(background);
    let lighter = foreground_luminance.max(background_luminance);
    let darker = foreground_luminance.min(background_luminance);
    (lighter + 0.05) / (darker + 0.05)
}

pub(super) fn contrast_threshold(
    category: DesignColorContrastCategory,
    level: DesignColorContrastLevel,
) -> Option<f32> {
    match (category, level) {
        (DesignColorContrastCategory::NormalText, DesignColorContrastLevel::Aa) => Some(4.5),
        (DesignColorContrastCategory::NormalText, DesignColorContrastLevel::Aaa) => Some(7.),
        (DesignColorContrastCategory::LargeText, DesignColorContrastLevel::Aa) => Some(3.),
        (DesignColorContrastCategory::LargeText, DesignColorContrastLevel::Aaa) => Some(4.5),
        (DesignColorContrastCategory::Graphics, DesignColorContrastLevel::Aa) => Some(3.),
        (DesignColorContrastCategory::Auto, _)
        | (DesignColorContrastCategory::Graphics, DesignColorContrastLevel::Aaa) => None,
    }
}

pub(super) fn contrast_value_crossings(
    hue: f32,
    saturation: f32,
    alpha: u8,
    background: DesignColor,
    threshold: f32,
) -> Vec<f32> {
    const VALUE_STEPS: usize = 96;
    let passes = |value: f32| {
        let color = hsv_to_color(
            Hsv {
                hue,
                saturation,
                value,
            },
            alpha,
        );
        wcag_contrast_ratio(color, background) + f32::EPSILON >= threshold
    };
    let mut crossings = Vec::with_capacity(2);
    let mut previous_value = 0.;
    let mut previous_passes = passes(previous_value);
    for step in 1..=VALUE_STEPS {
        let value = step as f32 / VALUE_STEPS as f32;
        let current_passes = passes(value);
        if current_passes != previous_passes {
            let mut low = previous_value;
            let mut high = value;
            for _ in 0..8 {
                let middle = (low + high) / 2.;
                if passes(middle) == previous_passes {
                    low = middle;
                } else {
                    high = middle;
                }
            }
            crossings.push((low + high) / 2.);
        }
        previous_value = value;
        previous_passes = current_passes;
    }
    crossings
}

pub(super) fn rgb_to_hsv(color: DesignColor) -> Hsv {
    let red = f32::from(color.red) / 255.;
    let green = f32::from(color.green) / 255.;
    let blue = f32::from(color.blue) / 255.;
    let maximum = red.max(green.max(blue));
    let minimum = red.min(green.min(blue));
    let delta = maximum - minimum;

    let hue = if delta <= f32::EPSILON {
        0.
    } else if maximum == red {
        (60. * ((green - blue) / delta)).rem_euclid(360.)
    } else if maximum == green {
        60. * (((blue - red) / delta) + 2.)
    } else {
        60. * (((red - green) / delta) + 4.)
    };
    let saturation = if maximum <= f32::EPSILON {
        0.
    } else {
        delta / maximum
    };

    Hsv {
        hue,
        saturation,
        value: maximum,
    }
}

fn rgb_to_hsl(color: DesignColor) -> Hsl {
    let red = f32::from(color.red) / 255.;
    let green = f32::from(color.green) / 255.;
    let blue = f32::from(color.blue) / 255.;
    let maximum = red.max(green.max(blue));
    let minimum = red.min(green.min(blue));
    let delta = maximum - minimum;
    let lightness = (maximum + minimum) / 2.;
    let saturation = if delta <= f32::EPSILON {
        0.
    } else {
        delta / (1. - (2. * lightness - 1.).abs())
    };

    Hsl {
        hue: rgb_to_hsv(color).hue,
        saturation,
        lightness,
    }
}

fn hsl_to_color(hsl: Hsl, alpha: u8) -> DesignColor {
    let hue = hsl.hue.rem_euclid(360.);
    let saturation = hsl.saturation.clamp(0., 1.);
    let lightness = hsl.lightness.clamp(0., 1.);
    let chroma = (1. - (2. * lightness - 1.).abs()) * saturation;
    let x = chroma * (1. - ((hue / 60.).rem_euclid(2.) - 1.).abs());
    let offset = lightness - chroma / 2.;
    let (red, green, blue) = match hue {
        hue if hue < 60. => (chroma, x, 0.),
        hue if hue < 120. => (x, chroma, 0.),
        hue if hue < 180. => (0., chroma, x),
        hue if hue < 240. => (0., x, chroma),
        hue if hue < 300. => (x, 0., chroma),
        _ => (chroma, 0., x),
    };

    DesignColor::rgba(
        channel_from_unit(red + offset),
        channel_from_unit(green + offset),
        channel_from_unit(blue + offset),
        alpha,
    )
}

pub(super) fn hsv_to_color(hsv: Hsv, alpha: u8) -> DesignColor {
    let hue = hsv.hue.rem_euclid(360.);
    let saturation = hsv.saturation.clamp(0., 1.);
    let value = hsv.value.clamp(0., 1.);
    let chroma = value * saturation;
    let x = chroma * (1. - ((hue / 60.).rem_euclid(2.) - 1.).abs());
    let offset = value - chroma;
    let (red, green, blue) = match hue {
        hue if hue < 60. => (chroma, x, 0.),
        hue if hue < 120. => (x, chroma, 0.),
        hue if hue < 180. => (0., chroma, x),
        hue if hue < 240. => (0., x, chroma),
        hue if hue < 300. => (x, 0., chroma),
        _ => (chroma, 0., x),
    };

    DesignColor::rgba(
        channel_from_unit(red + offset),
        channel_from_unit(green + offset),
        channel_from_unit(blue + offset),
        alpha,
    )
}

pub(super) fn channel_from_unit(value: f32) -> u8 {
    (value.clamp(0., 1.) * 255.).round() as u8
}

pub(super) fn mix_color(left: DesignColor, right: DesignColor, amount: f32) -> DesignColor {
    let amount = amount.clamp(0., 1.);
    let channel = |left: u8, right: u8| {
        (f32::from(left) + (f32::from(right) - f32::from(left)) * amount).round() as u8
    };
    DesignColor::rgba(
        channel(left.red, right.red),
        channel(left.green, right.green),
        channel(left.blue, right.blue),
        channel(left.alpha, right.alpha),
    )
}

// Retained exact-color entry and direct-manipulation UI lives beside the
// domain-neutral conversion helpers.
use super::*;

impl PaintPicker {
    pub(super) fn set_color_format(
        &mut self,
        format: ColorFormat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_text_input_edit_sessions(cx);
        self.color_format = format;
        self.close_nested_overlay(PaintPickerOverlay::ColorFormat, window, cx);
        self.hex_invalid = false;
        self.color_channel_invalid = [false; 3];
        self.sync_inputs(window, cx, false, false, false);
        cx.notify();
    }

    pub(super) fn open_color_format_menu_from_keyboard(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.color_format_menu_index = self.color_format.index();
        self.open_nested_overlay(PaintPickerOverlay::ColorFormat, window, cx);
        let focus_handle = self.color_format_menu_focus_handle.clone();
        window.defer(cx, move |window, cx| {
            focus_handle.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn close_color_format_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_nested_overlay(PaintPickerOverlay::ColorFormat, window, cx);
    }

    pub(super) fn handle_color_format_menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = ColorFormat::ALL.len();
        match event.keystroke.key.as_str() {
            "up" => {
                self.color_format_menu_index = (self.color_format_menu_index + count - 1) % count;
                cx.notify();
            }
            "down" => {
                self.color_format_menu_index = (self.color_format_menu_index + 1) % count;
                cx.notify();
            }
            "home" => {
                self.color_format_menu_index = 0;
                cx.notify();
            }
            "end" => {
                self.color_format_menu_index = count - 1;
                cx.notify();
            }
            "enter" | "space" => self.commit_color_format_menu(window, cx),
            "escape" => {
                if !self.dismiss_nested_overlay(
                    PaintPickerOverlay::ColorFormat,
                    InspectorOverlayDismissCause::Escape,
                    window,
                    cx,
                ) {
                    return;
                }
            }
            _ => return,
        }
        window.prevent_default();
        cx.stop_propagation();
    }

    /// Commits the highlighted color format; bound to the shared
    /// `ActivateControl` command on the roving menu surface.
    pub(super) fn commit_color_format_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let count = ColorFormat::ALL.len();
        let format = ColorFormat::ALL[self.color_format_menu_index.min(count - 1)];
        self.set_color_format(format, window, cx);
    }

    pub(super) fn color_editing_disabled(&self) -> bool {
        self.editing_disabled()
            || (self.color_only_title.is_some() && !self.color_only_color_editable)
            || self
                .paint
                .as_ref()
                .is_some_and(|paint| paint_color_locked(paint, self.selected_stop))
    }

    pub(super) fn opacity_editing_disabled(&self) -> bool {
        self.editing_disabled()
            || (self.color_only_title.is_some() && !self.color_only_opacity_editable)
            || self.paint.as_ref().is_some_and(|paint| {
                paint.kind.is_gradient() && paint_color_locked(paint, self.selected_stop)
            })
    }

    pub(super) fn remember_current_hue(&mut self) {
        let Some(color) = self.current_color() else {
            return;
        };
        let hsv = rgb_to_hsv(color);
        if hsv.saturation > f32::EPSILON {
            self.remembered_hue = hsv.hue;
        }
    }

    pub(super) fn current_color(&self) -> Option<DesignColor> {
        self.paint
            .as_ref()
            .map(|paint| selected_color(paint, self.selected_stop))
    }

    pub(super) fn current_picker_opacity(&self) -> Option<f32> {
        self.paint
            .as_ref()
            .map(|paint| picker_opacity(paint, self.selected_stop))
    }

    pub(super) fn parse_color_text_input(&self, value: &str) -> Option<ParsedColorInput> {
        match self.color_format {
            ColorFormat::Hex => parse_hex_color_input(value),
            ColorFormat::Css => parse_css_color_input(value),
            ColorFormat::Rgb | ColorFormat::Hsl | ColorFormat::Hsb => None,
        }
    }

    pub(super) fn color_text_terminal_edit(
        &self,
        parsed: ParsedColorInput,
    ) -> Option<DesignPaintEdit> {
        let paint = self.paint.as_ref()?;
        if parsed.explicit_alpha && !paint.kind.is_gradient() {
            Some(picker_opacity_edit(
                paint,
                self.selected_stop,
                f32::from(parsed.color.alpha) / 255. * 100.,
            ))
        } else {
            Some(color_input_edit(paint, self.selected_stop, parsed))
        }
    }

    pub(super) fn preview_color_text_input(
        &self,
        parsed: ParsedColorInput,
        cx: &mut Context<Self>,
    ) {
        let Some(paint) = self.paint.as_ref() else {
            return;
        };
        let _ = self.emit_edit(
            color_input_edit(paint, self.selected_stop, parsed),
            DesignPanelEditPhase::Preview,
            cx,
        );
        // Figma's solid-paint alpha is the paint opacity leaf, while a
        // gradient stop owns alpha on that exact color leaf. An 8/4-digit Hex
        // or CSS rgba edit therefore previews two typed solid-paint leaves
        // inside one whole-paint transaction, but only one gradient leaf.
        if parsed.explicit_alpha && !paint.kind.is_gradient() {
            let _ = self.emit_edit(
                picker_opacity_edit(
                    paint,
                    self.selected_stop,
                    f32::from(parsed.color.alpha) / 255. * 100.,
                ),
                DesignPanelEditPhase::Preview,
                cx,
            );
        }
    }

    pub(super) fn current_hsv(&self) -> Option<Hsv> {
        let mut hsv = rgb_to_hsv(self.current_color()?);
        if hsv.saturation <= f32::EPSILON {
            hsv.hue = self.remembered_hue;
        }
        Some(hsv)
    }

    pub(super) fn selected_color_target(&self) -> Option<DesignPaintColorTarget> {
        paint_color_target(self.paint.as_ref()?, self.selected_stop)
    }

    pub(super) fn current_color_text_edit(&self) -> Option<DesignPaintEdit> {
        let paint = self.paint.as_ref()?;
        Some(color_edit(
            paint,
            self.selected_stop,
            selected_color(paint, self.selected_stop),
        ))
    }

    pub(super) fn rgb_color_text_edit(&self, color: DesignColor) -> Option<DesignPaintEdit> {
        let paint = self.paint.as_ref()?;
        Some(color_edit(
            paint,
            self.selected_stop,
            rgb_edit_color(paint, self.selected_stop, color),
        ))
    }

    pub(super) fn current_opacity_text_edit(&self) -> Option<DesignPaintEdit> {
        let paint = self.paint.as_ref()?;
        Some(picker_opacity_edit(
            paint,
            self.selected_stop,
            picker_opacity(paint, self.selected_stop),
        ))
    }

    pub(super) fn begin_text_input_edit(
        &self,
        edit: DesignPaintEdit,
        cx: &mut Context<Self>,
    ) -> Option<DesignPaintEdit> {
        self.emit_edit(edit.clone(), DesignPanelEditPhase::Begin, cx)
            .map(|_| edit)
    }

    /// Emits the one unconditional terminal paired with a text-field Begin.
    ///
    /// Unlike ordinary one-shot edits, an unchanged Commit cannot be elided:
    /// the host still needs a terminal to release its transaction snapshot.
    pub(super) fn finish_text_input_edit(
        &self,
        session: DesignPaintEdit,
        candidate: Option<DesignPaintEdit>,
        cx: &mut Context<Self>,
    ) {
        let candidate = candidate.filter(|edit| {
            self.paint
                .clone()
                .is_some_and(|mut paint| paint.apply_edit(edit))
        });
        let (edit, phase) = candidate.map_or((session, DesignPanelEditPhase::Cancel), |edit| {
            (edit, DesignPanelEditPhase::Commit)
        });
        let Some(target) = self.target.clone() else {
            return;
        };
        cx.emit(PaintPickerEvent::Edit {
            target,
            edit: Box::new(edit),
            phase,
        });
    }

    pub(super) fn edit_color(
        &mut self,
        color: DesignColor,
        phase: DesignPanelEditPhase,
        cx: &mut Context<Self>,
    ) {
        if self.color_editing_disabled() {
            return;
        }
        let Some(paint) = self.paint.as_ref() else {
            return;
        };
        self.remembered_hue = rgb_to_hsv(color).hue;
        let edit = color_edit(paint, self.selected_stop, color);
        let _ = self.emit_edit(edit, phase, cx);
    }

    pub(super) fn edit_rgb_color(
        &mut self,
        color: DesignColor,
        phase: DesignPanelEditPhase,
        cx: &mut Context<Self>,
    ) {
        if self.color_editing_disabled() {
            return;
        }
        let Some(paint) = self.paint.as_ref() else {
            return;
        };
        let color = rgb_edit_color(paint, self.selected_stop, color);
        self.edit_color(color, phase, cx);
    }

    pub(super) fn set_saturation_value(
        &mut self,
        saturation: f32,
        value: f32,
        cx: &mut Context<Self>,
    ) {
        if self.color_editing_disabled() {
            return;
        }
        let Some(paint) = self.paint.as_ref() else {
            return;
        };
        let current = selected_color(paint, self.selected_stop);
        let color = rgb_edit_color(
            paint,
            self.selected_stop,
            hsv_to_color(
                Hsv {
                    hue: self.remembered_hue,
                    saturation: saturation.clamp(0., 1.),
                    value: value.clamp(0., 1.),
                },
                current.alpha,
            ),
        );
        let edit = color_edit(paint, self.selected_stop, color);
        if self.continuous_edit.is_some() {
            self.preview_continuous_edit(edit, cx);
        } else {
            let _ = self.emit_edit(edit, DesignPanelEditPhase::Commit, cx);
        }
    }

    pub(super) fn set_hue(&mut self, hue: f32, cx: &mut Context<Self>) {
        if self.color_editing_disabled() {
            return;
        }
        let Some(paint) = self.paint.as_ref() else {
            return;
        };
        self.remembered_hue = hue.rem_euclid(360.);
        let current = selected_color(paint, self.selected_stop);
        let mut hsv = rgb_to_hsv(current);
        hsv.hue = self.remembered_hue;
        let color = rgb_edit_color(paint, self.selected_stop, hsv_to_color(hsv, current.alpha));
        let edit = color_edit(paint, self.selected_stop, color);
        if self.continuous_edit.is_some() {
            self.preview_continuous_edit(edit, cx);
        } else {
            let _ = self.emit_edit(edit, DesignPanelEditPhase::Commit, cx);
        }
        cx.notify();
    }

    pub(super) fn set_alpha(&mut self, alpha: f32, cx: &mut Context<Self>) {
        if self.opacity_editing_disabled() {
            return;
        }
        let Some(paint) = self.paint.as_ref() else {
            return;
        };
        let edit = picker_opacity_edit(paint, self.selected_stop, alpha * 100.);
        if self.continuous_edit.is_some() {
            self.preview_continuous_edit(edit, cx);
        } else {
            let _ = self.emit_edit(edit, DesignPanelEditPhase::Commit, cx);
        }
    }

    pub(super) fn set_opacity(
        &self,
        opacity: f32,
        phase: DesignPanelEditPhase,
        cx: &mut Context<Self>,
    ) {
        if self.opacity_editing_disabled() {
            return;
        }
        let Some(paint) = self.paint.as_ref() else {
            return;
        };
        let _ = self.emit_edit(
            picker_opacity_edit(paint, self.selected_stop, opacity),
            phase,
            cx,
        );
    }

    pub(super) fn update_from_pointer(
        &mut self,
        control: ContinuousControl,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.paint.is_none()
            || match control {
                ContinuousControl::Alpha => self.opacity_editing_disabled(),
                ContinuousControl::ColorArea | ContinuousControl::Hue => {
                    self.color_editing_disabled()
                }
            }
        {
            return;
        }
        match control {
            ContinuousControl::ColorArea => {
                let x = fraction_x(self.color_area_bounds, position);
                let y = fraction_y(self.color_area_bounds, position);
                self.set_saturation_value(x, 1. - y, cx);
            }
            ContinuousControl::Hue => {
                self.set_hue(fraction_x(self.hue_bounds, position) * 360., cx);
            }
            ContinuousControl::Alpha => {
                self.set_alpha(fraction_x(self.alpha_bounds, position), cx);
            }
        }
    }

    pub(super) fn handle_continuous_key(
        &mut self,
        control: ContinuousControl,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.paint.is_none()
            || match control {
                ContinuousControl::Alpha => self.opacity_editing_disabled(),
                ContinuousControl::ColorArea | ContinuousControl::Hue => {
                    self.color_editing_disabled()
                }
            }
        {
            return;
        }
        let key = event.keystroke.key.as_str();
        let shift = event.keystroke.modifiers.shift;
        let handled = match control {
            ContinuousControl::ColorArea => {
                let Some(hsv) = self.current_hsv() else {
                    return;
                };
                let step = if shift {
                    KEYBOARD_COARSE_STEP
                } else {
                    KEYBOARD_FINE_STEP
                };
                match key {
                    "left" => {
                        self.set_saturation_value(hsv.saturation - step, hsv.value, cx);
                        true
                    }
                    "right" => {
                        self.set_saturation_value(hsv.saturation + step, hsv.value, cx);
                        true
                    }
                    "up" => {
                        self.set_saturation_value(hsv.saturation, hsv.value + step, cx);
                        true
                    }
                    "down" => {
                        self.set_saturation_value(hsv.saturation, hsv.value - step, cx);
                        true
                    }
                    _ => false,
                }
            }
            ContinuousControl::Hue => {
                let step = if shift {
                    HUE_COARSE_STEP
                } else {
                    HUE_FINE_STEP
                };
                match key {
                    "left" | "down" => {
                        self.set_hue(self.remembered_hue - step, cx);
                        true
                    }
                    "right" | "up" => {
                        self.set_hue(self.remembered_hue + step, cx);
                        true
                    }
                    _ => false,
                }
            }
            ContinuousControl::Alpha => {
                let Some(opacity) = self.current_picker_opacity() else {
                    return;
                };
                let step = if shift {
                    KEYBOARD_COARSE_STEP
                } else {
                    KEYBOARD_FINE_STEP
                };
                let alpha = opacity / 100.;
                match key {
                    "left" | "down" => {
                        self.set_alpha(alpha - step, cx);
                        true
                    }
                    "right" | "up" => {
                        self.set_alpha(alpha + step, cx);
                        true
                    }
                    _ => false,
                }
            }
        };
        if handled {
            window.prevent_default();
            cx.stop_propagation();
        }
    }

    pub(super) fn handle_hex_input(
        &mut self,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Focus) {
            self.dismissal_event_guard = false;
        }
        if matches!(event, InputEvent::Change) && self.ignore_next_hex_change {
            self.ignore_next_hex_change = false;
            return;
        }
        if self.dismissal_event_guard
            || self.suppress_input_events
            || self.color_editing_disabled()
            || self.paint.is_none()
        {
            return;
        }
        match event {
            InputEvent::Change => {
                if !self.hex_input.focus_handle(cx).is_focused(window) {
                    return;
                }
                if self.hex_edit_session.is_none()
                    && let Some(edit) = self.current_color_text_edit()
                {
                    self.hex_edit_session = self.begin_text_input_edit(edit, cx);
                }
                let value = self.hex_input.read(cx).value();
                if let Some(parsed) = self.parse_color_text_input(value.as_ref()) {
                    self.hex_invalid = false;
                    self.preview_color_text_input(parsed, cx);
                } else {
                    self.hex_invalid = true;
                    cx.notify();
                }
            }
            InputEvent::PressEnter { .. } => {
                window.prevent_default();
                let session = self.hex_edit_session.take();
                let candidate = if self.hex_invalid {
                    self.sync_inputs(window, cx, false, true, true);
                    self.ignore_next_hex_change = true;
                    self.hex_invalid = false;
                    cx.notify();
                    None
                } else {
                    let value = self.hex_input.read(cx).value();
                    self.parse_color_text_input(value.as_ref())
                        .and_then(|parsed| self.color_text_terminal_edit(parsed))
                };
                if let Some(session) = session {
                    self.finish_text_input_edit(session, candidate, cx);
                }
            }
            InputEvent::Blur => {
                let session = self.hex_edit_session.take();
                let candidate = if self.hex_invalid {
                    self.sync_inputs(window, cx, false, true, true);
                    self.ignore_next_hex_change = true;
                    self.hex_invalid = false;
                    cx.notify();
                    None
                } else {
                    let value = self.hex_input.read(cx).value();
                    self.parse_color_text_input(value.as_ref())
                        .and_then(|parsed| self.color_text_terminal_edit(parsed))
                };
                if let Some(session) = session {
                    self.finish_text_input_edit(session, candidate, cx);
                }
            }
            InputEvent::Focus => {
                if self.hex_edit_session.is_none()
                    && let Some(edit) = self.current_color_text_edit()
                {
                    self.hex_edit_session = self.begin_text_input_edit(edit, cx);
                }
            }
        }
    }

    pub(super) fn color_from_channel_inputs(&self, cx: &App) -> Option<DesignColor> {
        let values = self
            .color_channel_inputs
            .each_ref()
            .map(|input| input.read(cx).value().to_string());
        parse_color_channels(self.color_format, [&values[0], &values[1], &values[2]])
    }

    pub(super) fn handle_color_channel_input(
        &mut self,
        channel: usize,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Focus) {
            self.dismissal_event_guard = false;
        }
        if channel < self.ignore_next_color_channel_changes.len()
            && matches!(event, InputEvent::Change)
            && self.ignore_next_color_channel_changes[channel]
        {
            self.ignore_next_color_channel_changes[channel] = false;
            return;
        }
        if self.dismissal_event_guard
            || self.suppress_input_events
            || self.color_editing_disabled()
            || self.paint.is_none()
            || channel >= self.color_channel_inputs.len()
            || matches!(self.color_format, ColorFormat::Hex | ColorFormat::Css)
        {
            return;
        }

        let value = self.color_channel_inputs[channel].read(cx).value();
        self.color_channel_invalid[channel] =
            parse_color_channel_value(self.color_format, channel, value.as_ref()).is_none();

        match event {
            InputEvent::Change => {
                if !self.color_channel_inputs[channel]
                    .focus_handle(cx)
                    .is_focused(window)
                {
                    return;
                }
                if self.color_channel_edit_sessions[channel].is_none()
                    && let Some(edit) = self.current_color_text_edit()
                {
                    self.color_channel_edit_sessions[channel] =
                        self.begin_text_input_edit(edit, cx);
                }
                if !self.color_channel_invalid.iter().any(|invalid| *invalid)
                    && let Some(color) = self.color_from_channel_inputs(cx)
                {
                    self.edit_rgb_color(color, DesignPanelEditPhase::Preview, cx);
                }
                cx.notify();
            }
            InputEvent::PressEnter { .. } => {
                window.prevent_default();
                let session = self.color_channel_edit_sessions[channel].take();
                let candidate = if self.color_channel_invalid.iter().any(|invalid| *invalid) {
                    self.sync_inputs(window, cx, true, true, false);
                    self.ignore_next_color_channel_changes = [true; 3];
                    self.color_channel_invalid = [false; 3];
                    cx.notify();
                    None
                } else {
                    self.color_from_channel_inputs(cx)
                        .and_then(|color| self.rgb_color_text_edit(color))
                };
                if let Some(session) = session {
                    self.finish_text_input_edit(session, candidate, cx);
                }
            }
            InputEvent::Blur => {
                let session = self.color_channel_edit_sessions[channel].take();
                let candidate = if self.color_channel_invalid.iter().any(|invalid| *invalid) {
                    self.sync_inputs(window, cx, true, true, false);
                    self.ignore_next_color_channel_changes = [true; 3];
                    self.color_channel_invalid = [false; 3];
                    cx.notify();
                    None
                } else {
                    self.color_from_channel_inputs(cx)
                        .and_then(|color| self.rgb_color_text_edit(color))
                };
                if let Some(session) = session {
                    self.finish_text_input_edit(session, candidate, cx);
                }
            }
            InputEvent::Focus => {
                if self.color_channel_edit_sessions[channel].is_none()
                    && let Some(edit) = self.current_color_text_edit()
                {
                    self.color_channel_edit_sessions[channel] =
                        self.begin_text_input_edit(edit, cx);
                }
            }
        }
    }

    pub(super) fn handle_opacity_input(
        &mut self,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Focus) {
            self.dismissal_event_guard = false;
        }
        if matches!(event, InputEvent::Change) && self.ignore_next_opacity_change {
            self.ignore_next_opacity_change = false;
            return;
        }
        if self.dismissal_event_guard
            || self.suppress_input_events
            || self.opacity_editing_disabled()
            || self.paint.is_none()
        {
            return;
        }
        match event {
            InputEvent::Change => {
                if !self.opacity_input.focus_handle(cx).is_focused(window) {
                    return;
                }
                if self.opacity_edit_session.is_none()
                    && let Some(edit) = self.current_opacity_text_edit()
                {
                    self.opacity_edit_session = self.begin_text_input_edit(edit, cx);
                }
                let value = self.opacity_input.read(cx).value();
                if let Some(opacity) = parse_opacity(value.as_ref()) {
                    self.opacity_invalid = false;
                    self.set_opacity(opacity, DesignPanelEditPhase::Preview, cx);
                } else {
                    self.opacity_invalid = true;
                    cx.notify();
                }
            }
            InputEvent::PressEnter { .. } => {
                window.prevent_default();
                let session = self.opacity_edit_session.take();
                let candidate = if self.opacity_invalid {
                    self.sync_inputs(window, cx, true, false, true);
                    self.ignore_next_opacity_change = true;
                    self.opacity_invalid = false;
                    cx.notify();
                    None
                } else {
                    let value = self.opacity_input.read(cx).value();
                    parse_opacity(value.as_ref()).and_then(|opacity| {
                        let paint = self.paint.as_ref()?;
                        Some(picker_opacity_edit(paint, self.selected_stop, opacity))
                    })
                };
                if let Some(session) = session {
                    self.finish_text_input_edit(session, candidate, cx);
                }
            }
            InputEvent::Blur => {
                let session = self.opacity_edit_session.take();
                let candidate = if self.opacity_invalid {
                    self.sync_inputs(window, cx, true, false, true);
                    self.ignore_next_opacity_change = true;
                    self.opacity_invalid = false;
                    cx.notify();
                    None
                } else {
                    let value = self.opacity_input.read(cx).value();
                    parse_opacity(value.as_ref()).and_then(|opacity| {
                        let paint = self.paint.as_ref()?;
                        Some(picker_opacity_edit(paint, self.selected_stop, opacity))
                    })
                };
                if let Some(session) = session {
                    self.finish_text_input_edit(session, candidate, cx);
                }
            }
            InputEvent::Focus => {
                if self.opacity_edit_session.is_none()
                    && let Some(edit) = self.current_opacity_text_edit()
                {
                    self.opacity_edit_session = self.begin_text_input_edit(edit, cx);
                }
            }
        }
    }

    pub(super) fn sync_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        preserve_hex_draft: bool,
        preserve_opacity_draft: bool,
        preserve_color_channel_drafts: bool,
    ) {
        let color_text = self.paint.as_ref().map_or_else(String::new, |paint| {
            color_text_value(self.color_format, paint, self.selected_stop)
        });
        let color_channel_texts = self.current_color().map_or_else(
            || [String::new(), String::new(), String::new()],
            |color| color_channel_values(self.color_format, color),
        );
        let opacity_text = self
            .current_picker_opacity()
            .map_or_else(String::new, format_decimal);
        let stop_position_text = self
            .selected_gradient_stop()
            .map_or_else(String::new, |stop| {
                format_decimal(stop.position.clamp(0., 1.) * 100.)
            });
        let preserve_stop_position_draft =
            self.stop_position_input.focus_handle(cx).is_focused(window);
        let preserve_color_channels = self.color_channel_inputs.each_ref().map(|input| {
            preserve_color_channel_drafts && input.focus_handle(cx).is_focused(window)
        });

        self.suppress_input_events = true;
        if !preserve_hex_draft {
            self.hex_input.update(cx, |input, cx| {
                input.set_value(color_text, window, cx);
            });
        }
        if !preserve_opacity_draft {
            self.opacity_input.update(cx, |input, cx| {
                input.set_value(opacity_text, window, cx);
            });
        }
        for (channel, text) in color_channel_texts.into_iter().enumerate() {
            if !preserve_color_channels[channel] {
                self.color_channel_inputs[channel].update(cx, |input, cx| {
                    input.set_value(text, window, cx);
                });
                self.color_channel_invalid[channel] = false;
            }
        }
        if !preserve_stop_position_draft {
            self.stop_position_input.update(cx, |input, cx| {
                input.set_value(stop_position_text, window, cx);
            });
        }
        self.suppress_input_events = false;
    }

    pub(super) fn render_color_area(
        &self,
        color: DesignColor,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let disabled = self.color_editing_disabled();
        let hsv = self.current_hsv().unwrap_or_default();
        let hue_color = hsv_to_color(
            Hsv {
                hue: self.remembered_hue,
                saturation: 1.,
                value: 1.,
            },
            u8::MAX,
        );
        let id = SharedString::from(format!("{}-color-area", self.id));

        div()
            .id(id)
            .relative()
            .tab_index(0)
            .w_full()
            .h(px(COLOR_AREA_HEIGHT))
            .overflow_hidden()
            .rounded(px(7.))
            .border_1()
            .border_color(cx.theme().border)
            .bg(color_to_hsla(hue_color))
            .cursor_crosshair()
            .focus(|style| style.border_color(cx.theme().selection))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    if !this.color_editing_disabled()
                        && let (Some(paint), Some(color)) =
                            (this.paint.as_ref(), this.current_color())
                    {
                        this.begin_continuous_edit(
                            color_edit(paint, this.selected_stop, color),
                            cx,
                        );
                    }
                    this.update_from_pointer(ContinuousControl::ColorArea, event.position, cx);
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if event.dragging() {
                    this.update_from_pointer(ContinuousControl::ColorArea, event.position, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.commit_continuous_edit(cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.commit_continuous_edit(cx);
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.handle_continuous_key(ContinuousControl::ColorArea, event, window, cx);
            }))
            .child(div().absolute().size_full().bg(linear_gradient(
                90.,
                linear_color_stop(gpui::white(), 0.),
                linear_color_stop(gpui::transparent_white(), 1.),
            )))
            .child(div().absolute().size_full().bg(linear_gradient(
                180.,
                linear_color_stop(gpui::transparent_black(), 0.),
                linear_color_stop(gpui::black(), 1.),
            )))
            .children(self.render_contrast_boundary(cx))
            .child(
                div()
                    .absolute()
                    .left(relative(hsv.saturation))
                    .top(relative(1. - hsv.value))
                    .ml(px(-6.))
                    .mt(px(-6.))
                    .size(px(12.))
                    .rounded(px(6.))
                    .border_2()
                    .border_color(cx.theme().background)
                    .shadow_sm()
                    .bg(color_to_hsla(color)),
            )
            .child(track_bounds(cx.entity(), |this, bounds| {
                this.color_area_bounds = bounds;
            }))
            .when(disabled, |area| area.opacity(0.56))
            .into_any_element()
    }

    pub(super) fn render_contrast_boundary(&self, _cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.contrast_checker_open {
            return None;
        }
        let leaf = self.current_contrast_leaf()?;
        let category = self.resolved_contrast_category()?;
        let level = self.normalized_contrast_level();
        let threshold = contrast_threshold(category, level)?;
        let foreground_alpha =
            picker_color_with_opacity(self.paint.as_ref()?, self.selected_stop).alpha;
        let hue = self.remembered_hue;
        let background = leaf.effective_background;
        let stroke = if wcag_relative_luminance(opaque_color(background)) > 0.45 {
            gpui::black().opacity(0.78)
        } else {
            gpui::white().opacity(0.86)
        };

        Some(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    const SATURATION_STEPS: usize = 48;
                    let width = f32::from(bounds.size.width);
                    let height = f32::from(bounds.size.height);
                    if width <= 0. || height <= 0. {
                        return;
                    }
                    let mut previous = Vec::<f32>::new();
                    let mut path = gpui::PathBuilder::stroke(px(1.15));
                    for step in 0..=SATURATION_STEPS {
                        let saturation = step as f32 / SATURATION_STEPS as f32;
                        let crossings = contrast_value_crossings(
                            hue,
                            saturation,
                            foreground_alpha,
                            background,
                            threshold,
                        );
                        let x = bounds.origin.x + px(width * saturation);
                        for (branch, value) in crossings.iter().copied().enumerate() {
                            let y = bounds.origin.y + px(height * (1. - value));
                            if let Some(previous_value) = previous.get(branch).copied() {
                                let previous_saturation =
                                    (step.saturating_sub(1)) as f32 / SATURATION_STEPS as f32;
                                let previous_x = bounds.origin.x + px(width * previous_saturation);
                                let previous_y =
                                    bounds.origin.y + px(height * (1. - previous_value));
                                path.move_to(point(previous_x, previous_y));
                                path.line_to(point(x, y));
                            }
                        }
                        previous = crossings;
                    }
                    if let Ok(path) = path.build() {
                        window.paint_path(path, stroke);
                    }
                },
            )
            .absolute()
            .size_full()
            .into_any_element(),
        )
    }

    pub(super) fn render_hue_control(&self, cx: &mut Context<Self>) -> AnyElement {
        let disabled = self.color_editing_disabled();
        let colors = [
            DesignColor::rgb(0xff, 0x00, 0x00),
            DesignColor::rgb(0xff, 0xff, 0x00),
            DesignColor::rgb(0x00, 0xff, 0x00),
            DesignColor::rgb(0x00, 0xff, 0xff),
            DesignColor::rgb(0x00, 0x00, 0xff),
            DesignColor::rgb(0xff, 0x00, 0xff),
            DesignColor::rgb(0xff, 0x00, 0x00),
        ];
        let mut spectrum = h_flex().absolute().size_full();
        for pair in colors.windows(2) {
            spectrum = spectrum.child(div().h_full().flex_1().bg(linear_gradient(
                90.,
                linear_color_stop(color_to_hsla(pair[0]), 0.),
                linear_color_stop(color_to_hsla(pair[1]), 1.),
            )));
        }

        div()
            .id(SharedString::from(format!("{}-hue", self.id)))
            .relative()
            .tab_index(0)
            .w_full()
            .h(px(CONTROL_HEIGHT))
            .overflow_hidden()
            .rounded(px(CONTROL_HEIGHT / 2.))
            .border_1()
            .border_color(cx.theme().border)
            .focus(|style| style.border_color(cx.theme().selection))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    if !this.color_editing_disabled()
                        && let (Some(paint), Some(color)) =
                            (this.paint.as_ref(), this.current_color())
                    {
                        this.begin_continuous_edit(
                            color_edit(paint, this.selected_stop, color),
                            cx,
                        );
                    }
                    this.update_from_pointer(ContinuousControl::Hue, event.position, cx);
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if event.dragging() {
                    this.update_from_pointer(ContinuousControl::Hue, event.position, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.commit_continuous_edit(cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.commit_continuous_edit(cx);
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.handle_continuous_key(ContinuousControl::Hue, event, window, cx);
            }))
            .child(spectrum)
            .child(
                div()
                    .absolute()
                    .left(relative(self.remembered_hue / 360.))
                    .ml(px(-5.))
                    .top(px(3.))
                    .size(px(10.))
                    .rounded(px(5.))
                    .border_2()
                    .border_color(cx.theme().background)
                    .shadow_sm(),
            )
            .child(track_bounds(cx.entity(), |this, bounds| {
                this.hue_bounds = bounds;
            }))
            .when(disabled, |control| control.opacity(0.56))
            .into_any_element()
    }

    pub(super) fn render_alpha_control(
        &self,
        color: DesignColor,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let disabled = self.opacity_editing_disabled();
        let mut transparent = color;
        transparent.alpha = 0;
        let mut opaque = color;
        opaque.alpha = u8::MAX;
        let alpha = self.current_picker_opacity().unwrap_or(100.) / 100.;
        let mut thumb_color = color;
        thumb_color.alpha = channel_from_unit(alpha);

        div()
            .id(SharedString::from(format!("{}-alpha", self.id)))
            .relative()
            .tab_index(0)
            .w_full()
            .h(px(CONTROL_HEIGHT))
            .overflow_hidden()
            .rounded(px(CONTROL_HEIGHT / 2.))
            .border_1()
            .border_color(cx.theme().border)
            .bg(pattern_slash(
                cx.theme().muted_foreground.opacity(0.22),
                0.35,
                0.35,
            ))
            .focus(|style| style.border_color(cx.theme().selection))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    if !this.opacity_editing_disabled()
                        && let Some(edit) = this.paint.as_ref().map(|paint| {
                            picker_opacity_edit(
                                paint,
                                this.selected_stop,
                                picker_opacity(paint, this.selected_stop),
                            )
                        })
                    {
                        this.begin_continuous_edit(edit, cx);
                    }
                    this.update_from_pointer(ContinuousControl::Alpha, event.position, cx);
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if event.dragging() {
                    this.update_from_pointer(ContinuousControl::Alpha, event.position, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.commit_continuous_edit(cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.commit_continuous_edit(cx);
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.handle_continuous_key(ContinuousControl::Alpha, event, window, cx);
            }))
            .child(div().absolute().size_full().bg(linear_gradient(
                90.,
                linear_color_stop(color_to_hsla(transparent), 0.),
                linear_color_stop(color_to_hsla(opaque), 1.),
            )))
            .child(
                div()
                    .absolute()
                    .left(relative(alpha))
                    .ml(px(-5.))
                    .top(px(3.))
                    .size(px(10.))
                    .rounded(px(5.))
                    .border_2()
                    .border_color(cx.theme().background)
                    .shadow_sm()
                    .bg(color_to_hsla(thumb_color)),
            )
            .child(track_bounds(cx.entity(), |this, bounds| {
                this.alpha_bounds = bounds;
            }))
            .when(disabled, |control| control.opacity(0.56))
            .into_any_element()
    }

    pub(super) fn render_color_format_selector(&self, cx: &mut Context<Self>) -> AnyElement {
        let picker_for_open = cx.entity();
        let picker_for_trigger = picker_for_open.clone();
        let picker_for_content = picker_for_open.clone();
        let picker_id = self.id.clone();
        let selected_format = self.color_format;
        let highlighted_index = self.color_format_menu_index;
        let menu_focus_handle = self.color_format_menu_focus_handle.clone();
        let menu_focus_for_content = menu_focus_handle.clone();
        let trigger = Button::new(SharedString::from(format!("{}-color-format", self.id)))
            .label(selected_format.label())
            .dropdown_caret(true)
            .tooltip("Color format")
            .xsmall()
            .compact()
            .outline()
            .w(px(62.))
            .h(px(26.))
            .on_keyboard_activate(move |window, cx| {
                picker_for_trigger.update(cx, |this, cx| {
                    if this.nested_overlay_is_open(PaintPickerOverlay::ColorFormat) {
                        this.close_color_format_menu(window, cx);
                    } else {
                        this.open_color_format_menu_from_keyboard(window, cx);
                    }
                });
            });

        Popover::new(SharedString::from(format!("{}-color-format-menu", self.id)))
            .anchor(Anchor::BottomLeft)
            .open(self.nested_overlay_is_open(PaintPickerOverlay::ColorFormat))
            .track_focus(&menu_focus_handle)
            .overlay_closable(true)
            .on_open_change(move |open, window, cx| {
                picker_for_open.update(cx, |this, cx| {
                    if *open {
                        this.open_nested_overlay(PaintPickerOverlay::ColorFormat, window, cx);
                        this.color_format_menu_index = this.color_format.index();
                        cx.notify();
                    } else {
                        this.dismiss_nested_overlay(
                            PaintPickerOverlay::ColorFormat,
                            InspectorOverlayDismissCause::OutsideClick,
                            window,
                            cx,
                        );
                    }
                });
            })
            .trigger(trigger)
            .content(move |_, window, _| {
                v_flex()
                    .id(SharedString::from(format!(
                        "{picker_id}-color-format-options"
                    )))
                    .key_context(CONTROL_KEY_CONTEXT)
                    .track_focus(&menu_focus_for_content.clone().tab_index(0).tab_stop(true))
                    .on_action({
                        let picker = picker_for_content.clone();
                        move |_: &ActivateControl, window, cx| {
                            picker.update(cx, |this, cx| {
                                this.commit_color_format_menu(window, cx);
                            });
                        }
                    })
                    .on_key_down({
                        let picker = picker_for_content.clone();
                        move |event: &KeyDownEvent, window, cx| {
                            picker.update(cx, |this, cx| {
                                this.handle_color_format_menu_key(event, window, cx);
                            });
                        }
                    })
                    .w(popup_width(window, 88.))
                    .gap_1()
                    .children(
                        ColorFormat::ALL
                            .into_iter()
                            .enumerate()
                            .map(|(index, format)| {
                                let picker = picker_for_content.clone();
                                Button::new(SharedString::from(format!(
                                    "{}-color-format-{}",
                                    picker_id,
                                    format.label().to_lowercase()
                                )))
                                .label(format.label())
                                .tooltip(format.label())
                                .xsmall()
                                .compact()
                                .ghost()
                                .w_full()
                                .tab_stop(false)
                                .selected(highlighted_index == index)
                                .when(selected_format == format, |button| {
                                    button.child(Icon::new(IconName::Check).xsmall())
                                })
                                .on_activate(
                                    move |_, window, cx| {
                                        picker.update(cx, |this, cx| {
                                            this.set_color_format(format, window, cx);
                                        });
                                    },
                                )
                            }),
                    )
            })
            .w(px(62.))
            .h(px(26.))
            .into_any_element()
    }

    pub(super) fn render_color_channel_fields(&self, cx: &mut Context<Self>) -> AnyElement {
        let labels = self.color_format.channel_labels();
        let disabled = self.color_editing_disabled();
        let mut fields = h_flex().flex_1().min_w(px(0.)).gap_1();
        for (channel, label) in labels.into_iter().enumerate() {
            let percentage =
                matches!(self.color_format, ColorFormat::Hsl | ColorFormat::Hsb) && channel > 0;
            fields = fields.child(
                Input::new(&self.color_channel_inputs[channel])
                    .prefix(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(label),
                    )
                    .when(percentage, |input| input.suffix(div().text_xs().child("%")))
                    .xsmall()
                    .h(px(26.))
                    .flex_1()
                    .min_w(px(0.))
                    .disabled(disabled)
                    .when(self.color_channel_invalid[channel], |input| {
                        input.border_color(cx.theme().red)
                    }),
            );
        }
        fields.into_any_element()
    }

    pub(super) fn render_color_value_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let color_disabled = self.color_editing_disabled();
        let opacity_disabled = self.opacity_editing_disabled();
        let opacity = Input::new(&self.opacity_input)
            .suffix(div().text_xs().child("%"))
            .xsmall()
            .h(px(26.))
            .w(px(62.))
            .disabled(opacity_disabled)
            .when(self.opacity_invalid, |input| {
                input.border_color(cx.theme().red)
            });

        if matches!(self.color_format, ColorFormat::Hex | ColorFormat::Css) {
            let controls = h_flex()
                .w_full()
                .gap_1()
                .child(self.render_color_format_selector(cx))
                .child(
                    Input::new(&self.hex_input)
                        .xsmall()
                        .h(px(26.))
                        .flex_1()
                        .min_w(px(0.))
                        .disabled(color_disabled)
                        .when(self.hex_invalid, |input| input.border_color(cx.theme().red)),
                );
            return controls
                .when(self.color_format == ColorFormat::Hex, |controls| {
                    controls.child(opacity)
                })
                .into_any_element();
        }

        v_flex()
            .w_full()
            .gap_1()
            .child(
                h_flex()
                    .w_full()
                    .gap_1()
                    .child(self.render_color_format_selector(cx))
                    .child(self.render_color_channel_fields(cx)),
            )
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Opacity"),
                    )
                    .child(opacity),
            )
            .into_any_element()
    }

    pub(super) fn render_color_editor(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let color = self.current_color()?;
        Some(
            v_flex()
                .w_full()
                .gap_2()
                .child(self.render_color_area(color, cx))
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            Button::new(SharedString::from(format!("{}-eyedropper", self.id)))
                                .icon(IconName::Inspector)
                                .tooltip("Pick color from canvas")
                                .xsmall()
                                .compact()
                                .ghost()
                                .w(px(24.))
                                .disabled(self.color_editing_disabled())
                                .on_activate(cx.listener(|this, _, _, cx| {
                                    this.request_eyedropper(cx);
                                })),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w(px(0.))
                                .gap_2()
                                .child(self.render_hue_control(cx))
                                .child(self.render_alpha_control(color, cx)),
                        ),
                )
                .child(self.render_color_value_controls(cx))
                .into_any_element(),
        )
    }

    pub(super) fn render_opacity_only(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .w_full()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Paint opacity"),
            )
            .child(
                Input::new(&self.opacity_input)
                    .suffix(div().text_xs().child("%"))
                    .xsmall()
                    .h(px(26.))
                    .disabled(self.editing_disabled())
                    .when(self.opacity_invalid, |input| {
                        input.border_color(cx.theme().red)
                    }),
            )
            .into_any_element()
    }
}
