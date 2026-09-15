//! Shared geometry scale for Fanta chrome.
//!
//! This module is GEOMETRY ONLY. Color, the body font, and surface radius come
//! from `gpui_component::Theme` (`radius`, `radius_lg`, `font_size`) — not from
//! here. A token that would restate a theme value does not belong in this file.
//!
//! Every constant is a bare `f32`, matching the existing
//! [`InspectorMetrics`](crate::molecules::InspectorMetrics) convention, so call
//! sites stay `px(RowHeight::FIELD)` and the text-scanning architecture tests
//! keep matching them. Hosts reach the scale at
//! `fanta_gpui::atoms::tokens::RowHeight::FIELD`.

/// Padding and gap steps shared by rows, sections, and panel chrome.
pub struct Space;

impl Space {
    pub const XS: f32 = 4.;
    pub const SM: f32 = 8.;
    pub const MD: f32 = 12.;
    pub const LG: f32 = 16.;
}

/// Fixed heights for the crate's horizontal strips: inspector rows, list rows,
/// menu items, page entries, flyouts, and section headers.
pub struct RowHeight;

impl RowHeight {
    pub const FIELD: f32 = 24.;
    pub const LIST: f32 = 28.;
    /// Off-step by one: deliberately preserved shipping geometry, pending a
    /// post-ship polish pass.
    pub const MENU: f32 = 30.;
    pub const PAGE: f32 = 32.;
    /// Off-step by two: deliberately preserved shipping geometry, pending a
    /// post-ship polish pass.
    pub const FLYOUT: f32 = 38.;
    pub const SECTION_HEADER: f32 = 40.;
}

/// Square hit targets for controls: inline affordances inside a field, panel
/// chrome buttons, and toolbar tools.
pub struct ControlSize;

impl ControlSize {
    pub const INLINE: f32 = 16.;
    pub const CHROME: f32 = 28.;
    pub const TOOL: f32 = 32.;
}

/// Corner radii owned by Fanta chrome. Surface radius that a host theme should
/// drive comes from `Theme::radius`/`radius_lg` instead.
pub struct Radius;

impl Radius {
    pub const CONTROL: f32 = 4.;
    pub const MENU: f32 = 12.;
}

/// Rendered glyph sizes for [`LucideIcon`](crate::atoms::LucideIcon) sites.
pub struct IconSize;

impl IconSize {
    pub const XS: f32 = 12.;
    /// Off-step by two: deliberately preserved shipping geometry, pending a
    /// post-ship polish pass.
    pub const SM: f32 = 14.;
    pub const MD: f32 = 16.;
}

/// Text sizes passed to `text_size(px(..))`. Body copy that should follow the
/// host theme uses `Theme::font_size` instead.
pub struct TypeScale;

impl TypeScale {
    pub const MICRO: f32 = 10.;
    pub const CAPTION: f32 = 11.;
    pub const BODY: f32 = 12.;
    pub const LABEL: f32 = 13.;
    /// Off-step by one: deliberately preserved shipping geometry, pending a
    /// post-ship polish pass.
    pub const TITLE: f32 = 14.;
    pub const DISPLAY: f32 = 20.;
}

/// Widths of transient surfaces: dropdown menus, popovers, and pickers.
pub struct MenuWidth;

impl MenuWidth {
    pub const NARROW: f32 = 168.;
    pub const STANDARD: f32 = 224.;
    pub const POPOVER: f32 = 248.;
    pub const PICKER: f32 = 280.;
    pub const PICKER_WIDE: f32 = 304.;
}

/// Container widths at which a panel switches layout density.
pub struct Breakpoint;

impl Breakpoint {
    pub const COMPACT: f32 = 360.;
    pub const WIDE: f32 = 440.;
}
