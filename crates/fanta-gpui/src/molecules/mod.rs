//! Molecules: composite chrome assembled from `crate::atoms`.
//!
//! Popover-menu chrome with two-axis clamping, the anchored-popup recipe for
//! trigger-attached transient surfaces, the selectable list-row baseline, and
//! the edge-fade affordance for horizontally scrollable rows
//! (ARCHITECTURE.md §16).
//!
//! This module is the curated public molecules API: hosts assemble menus,
//! popups, rows, and scroll affordances from these recipes so transient
//! surfaces share the library's clamping, chrome, and focus contracts.

mod list_row;
mod menu;
mod popup;
mod scroll_fade;

pub use list_row::list_row;
pub use menu::{clamp_menu_origin, menu_item, menu_surface};
pub use popup::{
    POPUP_SAFE_MARGIN, anchored_popup, popup_height, popup_max_height, popup_surface, popup_width,
};
pub use scroll_fade::{EdgeFades, horizontal_fade_overlays, track_horizontal_edge_fades};
