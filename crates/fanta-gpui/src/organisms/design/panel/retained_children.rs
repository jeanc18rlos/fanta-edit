//! Retained child entities owned by the Design inspector façade.
//!
//! These are GPUI children the panel keeps alive across renders, plus the
//! subscriptions and controlled snapshots that keep them synchronized with the
//! host snapshot. None of them are document data: every value here is either a
//! transient draft surface or a projection of controlled host data, and each is
//! safe to discard when the inspection context changes.
//!
//! Grouping them keeps the façade's field list about ownership tiers rather
//! than about individual widgets; `DesignPanelFactory` remains the only place
//! that assembles them.

use std::collections::HashMap;

use gpui::{Entity, Subscription};
use gpui_component::input::InputState;
use gpui_component::slider::SliderState;

use super::{DesignPanelProperty, PropertyOptionSnapshot, PropertySelectState};

/// Retained children grouped by the concern that owns them.
pub(super) struct DesignPanelRetainedChildren {
    pub(super) inputs: DesignPanelInputStates,
    pub(super) draw_sliders: DesignDrawSliderStates,
    pub(super) options: DesignPanelOptionStates,
}

impl DesignPanelRetainedChildren {
    pub(super) fn new(
        inputs: DesignPanelInputStates,
        draw_sliders: DesignDrawSliderStates,
    ) -> Self {
        Self {
            inputs,
            draw_sliders,
            options: DesignPanelOptionStates::default(),
        }
    }
}

/// Retained text-entry children.
///
/// `property` and `component_multiline` carry uncommitted edit drafts; the
/// remaining inputs hold search queries that only filter presentation.
pub(super) struct DesignPanelInputStates {
    pub(super) property: Entity<InputState>,
    pub(super) property_variable_search: Entity<InputState>,
    pub(super) component_property_variable_search: Entity<InputState>,
    pub(super) component_swap_search: Entity<InputState>,
    pub(super) font_search: Entity<InputState>,
    pub(super) style_browser_search: Entity<InputState>,
    pub(super) component_multiline: Entity<InputState>,
}

/// Draw-workspace Appearance sliders.
///
/// The sliders project controlled host values and emit preview phases through
/// their subscriptions; retaining the subscriptions here keeps them alive for
/// exactly as long as the sliders they drive.
pub(super) struct DesignDrawSliderStates {
    pub(super) opacity: Entity<SliderState>,
    pub(super) corner_radius: Entity<SliderState>,
    _opacity_subscription: Subscription,
    _corner_radius_subscription: Subscription,
}

impl DesignDrawSliderStates {
    pub(super) fn new(
        opacity: Entity<SliderState>,
        opacity_subscription: Subscription,
        corner_radius: Entity<SliderState>,
        corner_radius_subscription: Subscription,
    ) -> Self {
        Self {
            opacity,
            corner_radius,
            _opacity_subscription: opacity_subscription,
            _corner_radius_subscription: corner_radius_subscription,
        }
    }

    /// Rebuilds the corner-radius slider when the host supplies a new legal
    /// range, dropping the previous subscription with the child it observed.
    pub(super) fn replace_corner_radius(
        &mut self,
        state: Entity<SliderState>,
        subscription: Subscription,
    ) {
        self.corner_radius = state;
        self._corner_radius_subscription = subscription;
    }
}

/// Per-property select children and the host data they were last built from.
///
/// `snapshots` records the options and selection a child was synchronized to,
/// so a re-render only rebuilds a child when the controlled data actually
/// changed.
#[derive(Default)]
pub(super) struct DesignPanelOptionStates {
    pub(super) states: HashMap<DesignPanelProperty, Entity<PropertySelectState>>,
    pub(super) subscriptions: HashMap<DesignPanelProperty, Subscription>,
    pub(super) snapshots: HashMap<DesignPanelProperty, PropertyOptionSnapshot>,
}
