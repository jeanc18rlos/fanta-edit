//! Seeds `gpui_component`'s theme from the active Zed theme so fanta-gpui
//! panels paint with the application palette, and re-applies whenever
//! settings change.
//!
//! Import discipline: never `use` both `ActiveTheme` traits in one file —
//! Zed theme access here is fully qualified, gpui_component's stays behind
//! its own types.

use gpui::App;
use gpui_component::theme::{Theme, ThemeMode};
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
