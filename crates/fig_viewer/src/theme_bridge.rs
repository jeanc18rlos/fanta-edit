//! Seeds `gpui_component`'s theme from the active Zed theme so fanta-gpui
//! panels paint with the application palette, and re-applies whenever
//! settings change.
//!
//! Import discipline: never `use` both `ActiveTheme` traits in one file —
//! Zed theme access here is fully qualified, gpui_component's stays behind
//! its own types.

use gpui::{App, Hsla};
use gpui_component::theme::{Colorize, Theme, ThemeColor, ThemeMode};
use settings::SettingsStore;
use theme::theme_settings;

pub fn init(cx: &mut App) {
    apply(cx);
    cx.observe_global::<SettingsStore>(apply).detach();
}

fn apply(cx: &mut App) {
    let zed = theme::ActiveTheme::theme(cx).clone();
    let mode = match zed.appearance() {
        theme::Appearance::Dark => ThemeMode::Dark,
        theme::Appearance::Light => ThemeMode::Light,
    };
    // Reseed every token from gpui_component's own light/dark defaults first
    // so anything not mapped below still matches the mode.
    Theme::change(mode, None, cx);

    let colors = zed.colors().clone();
    let status = zed.status().clone();
    let (ui_family, ui_size, mono_family, mono_size) = {
        let settings = theme_settings(cx);
        (
            settings.ui_font(cx).family.clone(),
            settings.ui_font_size(cx),
            settings.buffer_font(cx).family.clone(),
            settings.buffer_font_size(cx),
        )
    };

    let theme = Theme::global_mut(cx);
    theme.font_family = ui_family.to_string().into();
    theme.font_size = ui_size;
    theme.mono_font_family = mono_family.to_string().into();
    theme.mono_font_size = mono_size;

    let t = &mut theme.colors;
    t.background = colors.surface_background;
    t.foreground = colors.text;
    t.border = colors.border;
    t.input = colors.border;
    t.ring = colors.border_focused;
    t.muted = colors.elevated_surface_background;
    t.muted_foreground = colors.text_muted;
    t.accent = colors.element_hover;
    t.accent_foreground = colors.text;
    t.primary = colors.text_accent;
    t.primary_foreground = colors.background;
    t.primary_hover = colors.element_hover;
    t.primary_active = colors.element_active;
    t.secondary = colors.element_background;
    t.secondary_foreground = colors.text;
    t.secondary_hover = colors.element_hover;
    t.secondary_active = colors.element_active;
    t.danger = status.error;
    t.danger_foreground = colors.background;
    apply_success_palette(t, status.success, colors.background, mode);
    t.popover = colors.elevated_surface_background;
    t.popover_foreground = colors.text;
    t.selection = colors.element_selection_background;
    t.caret = colors.text_accent;
    t.link = colors.text_accent;
    t.sidebar = colors.panel_background;
    t.sidebar_border = colors.border;
    t.sidebar_accent = colors.element_selected;
    t.sidebar_accent_foreground = colors.text;
    t.sidebar_foreground = colors.text;
    t.list = colors.surface_background;
    t.list_hover = colors.element_hover;
    t.list_active = colors.element_selected;
    t.list_active_border = colors.border_selected;
    t.scrollbar = colors.scrollbar_track_background;
    t.scrollbar_thumb = colors.scrollbar_thumb_background;
    t.scrollbar_thumb_hover = colors.scrollbar_thumb_hover_background;
    t.drop_target = colors.drop_target_background;
    t.drag_border = colors.border_selected;
    t.title_bar = colors.title_bar_background;
    t.tab_bar = colors.tab_bar_background;

    cx.refresh_windows();
}

fn apply_success_palette(
    colors: &mut ThemeColor,
    success: Hsla,
    panel_background: Hsla,
    mode: ThemeMode,
) {
    // Success is shared by status text and filled controls, so adjust its full palette.
    colors.success = readable_success(success, panel_background);
    colors.success_foreground = success_foreground(colors.background.blend(colors.success));
    colors.success_hover = readable_success(
        colors.background.blend(colors.success.opacity(0.9)),
        colors.success_foreground,
    );
    colors.success_active = readable_success(
        colors.background.blend(
            colors
                .success
                .darken(if mode.is_dark() { 0.2 } else { 0.1 }),
        ),
        colors.success_foreground,
    );
}

fn readable_success(color: Hsla, background: Hsla) -> Hsla {
    let displayed = background.blend(color);
    if contrast_ratio(displayed, background) >= 4.5 {
        return color;
    }
    // Start from the composited appearance: retaining a low alpha can make 4.5 unreachable.
    let target = success_foreground(background);
    let mut lower = 0.0;
    let mut upper = 1.0;
    let mut result = target;
    for _ in 0..16 {
        let amount = (lower + upper) / 2.0;
        let candidate = displayed.blend(target.opacity(amount));
        if contrast_ratio(candidate, background) >= 4.5 {
            upper = amount;
            result = candidate;
        } else {
            lower = amount;
        }
    }
    result
}

fn success_foreground(background: Hsla) -> Hsla {
    if contrast_ratio(gpui::black(), background) >= contrast_ratio(gpui::white(), background) {
        gpui::black()
    } else {
        gpui::white()
    }
}

fn contrast_ratio(first: Hsla, second: Hsla) -> f64 {
    let luminance = |color: Hsla| {
        let color = color.to_rgb();
        let linear = |channel: f32| {
            let channel = f64::from(channel);
            if channel <= 0.04045 {
                channel / 12.92
            } else {
                ((channel + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
    };
    let first = luminance(first);
    let second = luminance(second);
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_success_contrast(colors: &ThemeColor, panel_background: Hsla) {
        assert!(contrast_ratio(panel_background.blend(colors.success), panel_background) >= 4.5);
        for background in [colors.success, colors.success_hover, colors.success_active] {
            assert!(
                contrast_ratio(
                    colors.background.blend(background),
                    colors.success_foreground
                ) >= 4.5
            );
            for channel in [background.h, background.s, background.l, background.a] {
                assert!(channel.is_finite() && (0.0..=1.0).contains(&channel));
            }
        }
        assert_eq!(colors.success_foreground.a, 1.0);
    }

    #[test]
    fn success_palette_preserves_readable_one_dark_status() {
        let success = gpui::rgb(0xa1c181).into();
        let panel_background = gpui::rgb(0x3b414d).into();
        let mut colors = ThemeColor {
            background: gpui::rgb(0x2f343e).into(),
            ..ThemeColor::default()
        };
        apply_success_palette(&mut colors, success, panel_background, ThemeMode::Dark);
        assert_eq!(colors.success, success);
        assert_eq!(colors.success_foreground, gpui::black());
        assert_success_contrast(&colors, panel_background);
    }

    #[test]
    fn success_palette_adjusts_one_light_status_and_all_filled_states() {
        let success = gpui::rgb(0x669f59).into();
        let panel_background = gpui::rgb(0xdcdcdd).into();
        let mut colors = ThemeColor {
            background: gpui::rgb(0xebebec).into(),
            ..ThemeColor::default()
        };
        assert!(contrast_ratio(success, panel_background) < 2.4);
        apply_success_palette(&mut colors, success, panel_background, ThemeMode::Light);
        assert_ne!(colors.success, success);
        assert_eq!(colors.success_foreground, gpui::white());
        let adjusted = colors.success.to_rgb();
        assert!(adjusted.g > adjusted.r && adjusted.g > adjusted.b);
        assert_success_contrast(&colors, panel_background);
    }

    #[test]
    fn success_palette_composites_translucent_status_before_contrast_adjustment() {
        for (mode, panel, surface) in [
            (ThemeMode::Dark, 0x3b414d, 0x2f343e),
            (ThemeMode::Light, 0xdcdcdd, 0xebebec),
        ] {
            let panel_background = gpui::rgb(panel).into();
            for alpha in [0.0, 0.05, 0.5, 1.0] {
                let mut colors = ThemeColor {
                    background: gpui::rgb(surface).into(),
                    ..ThemeColor::default()
                };
                let success = Hsla::from(gpui::rgb(0x669f59)).opacity(alpha);
                apply_success_palette(&mut colors, success, panel_background, mode);
                assert_success_contrast(&colors, panel_background);
            }
        }
    }
}
