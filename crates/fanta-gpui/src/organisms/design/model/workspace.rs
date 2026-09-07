use std::collections::HashSet;

use gpui::SharedString;

use super::*;

/// Host-owned Draw appearance policy for one exact ordered node target.
///
/// Opacity always uses the fixed `0..=100` percentage range. Corner radius is
/// intentionally target-bound because its useful slider maximum depends on
/// host geometry and product policy. For a multiple selection, the host also
/// supplies explicit uniform Width and Height property states; mixed or
/// unresolved geometry cannot safely share one corner-radius range. Stale,
/// Page, empty, or duplicate targets remain stored but never render or emit.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignDrawAppearanceViewData {
    pub target: DesignPanelTarget,
    pub corner_radius_range: DesignDrawSliderRange,
}

impl DesignDrawAppearanceViewData {
    pub const fn new(
        target: DesignPanelTarget,
        corner_radius_range: DesignDrawSliderRange,
    ) -> Self {
        Self {
            target,
            corner_radius_range,
        }
    }

    pub fn is_valid(&self) -> bool {
        let DesignPanelTarget::Nodes { node_ids } = &self.target else {
            return false;
        };
        let unique_ids = node_ids
            .iter()
            .map(SharedString::as_ref)
            .collect::<HashSet<_>>();
        !node_ids.is_empty()
            && unique_ids.len() == node_ids.len()
            && node_ids.iter().all(|node_id| !node_id.trim().is_empty())
    }
}

/// Host-controlled cross-file preference for keyboard nudging in numeric
/// inspector fields.
///
/// Figma defaults to one resolution-independent point for a normal arrow key
/// and ten points while Shift is held. Construction rejects zero, negative,
/// NaN, and infinite values so a panel never has to repair host preferences.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DesignNudgeSettings {
    small: f32,
    big: f32,
}

impl DesignNudgeSettings {
    pub const DEFAULT_SMALL: f32 = 1.;
    pub const DEFAULT_BIG: f32 = 10.;

    pub fn new(small: f32, big: f32) -> Option<Self> {
        (small.is_finite() && small > 0. && big.is_finite() && big > 0.)
            .then_some(Self { small, big })
    }

    pub const fn small(self) -> f32 {
        self.small
    }

    pub const fn big(self) -> f32 {
        self.big
    }

    pub const fn amount(self, big: bool) -> f32 {
        if big { self.big } else { self.small }
    }
}

impl Default for DesignNudgeSettings {
    fn default() -> Self {
        Self {
            small: Self::DEFAULT_SMALL,
            big: Self::DEFAULT_BIG,
        }
    }
}

/// Host-controlled workspace projection for the editable right sidebar.
///
/// This is deliberately orthogonal to [`DesignPanelSurface`] and
/// [`DesignPanelEditMode`](super::DesignPanelEditMode). Entering Draw changes
/// which inspector presentation is mounted, but it does not replace the
/// host's accepted Design/Prototype surface or the active canvas edit context.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum DesignPanelWorkspaceMode {
    #[default]
    Design,
    Draw,
}

impl DesignPanelWorkspaceMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Design => "Design",
            Self::Draw => "Draw",
        }
    }
}

/// One validated linear range used by a Draw-only appearance slider.
///
/// Figma documents Draw's slider-first presentation but does not publish a
/// universal corner-radius maximum. The host therefore supplies that range
/// for the exact current target instead of the component inventing one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DesignDrawSliderRange {
    min: f32,
    max: f32,
    step: f32,
}

impl DesignDrawSliderRange {
    pub fn new(min: f32, max: f32, step: f32) -> Option<Self> {
        (min.is_finite() && max.is_finite() && step.is_finite() && min < max && step > 0.)
            .then_some(Self { min, max, step })
    }

    pub const fn min(self) -> f32 {
        self.min
    }

    pub const fn max(self) -> f32 {
        self.max
    }

    pub const fn step(self) -> f32 {
        self.step
    }

    pub fn clamp_and_snap(self, value: f32) -> f32 {
        if !value.is_finite() {
            return self.min;
        }
        let snapped = self.min + ((value - self.min) / self.step).round() * self.step;
        snapped.clamp(self.min, self.max)
    }
}

/// Vertical displacement at or beyond 80 px toward the screen top for 2x.
pub const DESIGN_SCRUB_DOUBLE_SPEED_Y_THRESHOLD: f32 = -80.;

/// Vertical displacement at or beyond 80 px toward the screen bottom for 1/2.
pub const DESIGN_SCRUB_HALF_SPEED_Y_THRESHOLD: f32 = 80.;

/// Vertical displacement at or beyond 160 px toward the screen bottom for 1/4.
pub const DESIGN_SCRUB_QUARTER_SPEED_Y_THRESHOLD: f32 = 160.;

/// Figma's four numeric-field scrub speed notifications.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum DesignScrubSpeed {
    Double,
    #[default]
    Normal,
    Half,
    Quarter,
}

impl DesignScrubSpeed {
    /// Resolves a deterministic band from pointer Y minus the gesture origin.
    ///
    /// Figma documents the direction and four values but not exact screen
    /// distances. Fanta uses −80 px for 2x, +80 px for 1/2, and +160 px for
    /// 1/4; the interval between −80 and +80 remains 1x.
    pub fn from_vertical_displacement(displacement_y: f32) -> Self {
        if !displacement_y.is_finite() {
            Self::Normal
        } else if displacement_y <= DESIGN_SCRUB_DOUBLE_SPEED_Y_THRESHOLD {
            Self::Double
        } else if displacement_y >= DESIGN_SCRUB_QUARTER_SPEED_Y_THRESHOLD {
            Self::Quarter
        } else if displacement_y >= DESIGN_SCRUB_HALF_SPEED_Y_THRESHOLD {
            Self::Half
        } else {
            Self::Normal
        }
    }

    pub const fn multiplier(self) -> f32 {
        match self {
            Self::Double => 2.,
            Self::Normal => 1.,
            Self::Half => 0.5,
            Self::Quarter => 0.25,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Double => "2x",
            Self::Normal => "1x",
            Self::Half => "1/2",
            Self::Quarter => "1/4",
        }
    }

    /// Width of the transient status cue, mirroring Figma's wider cursor at
    /// faster bands without replacing the platform resize cursor.
    pub const fn cue_width(self) -> f32 {
        match self {
            Self::Double => 28.,
            Self::Normal => 22.,
            Self::Half => 16.,
            Self::Quarter => 10.,
        }
    }
}

/// One host-controlled right-sidebar surface.
///
/// Editor and viewer permissions expose different, exhaustive surface sets.
/// The reusable panel renders the Design and Properties projections; Prototype
/// and Comment remain host-owned surfaces behind the same navigation contract.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum DesignPanelSurface {
    #[default]
    Design,
    Prototype,
    Comment,
    Properties,
}

impl DesignPanelSurface {
    pub const EDITOR: [Self; 2] = [Self::Design, Self::Prototype];
    pub const VIEWER: [Self; 2] = [Self::Comment, Self::Properties];

    pub const fn available(can_edit: bool) -> &'static [Self] {
        if can_edit {
            &Self::EDITOR
        } else {
            &Self::VIEWER
        }
    }

    pub const fn is_available(self, can_edit: bool) -> bool {
        matches!(
            (can_edit, self),
            (true, Self::Design | Self::Prototype) | (false, Self::Comment | Self::Properties)
        )
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Design => "Design",
            Self::Prototype => "Prototype",
            Self::Comment => "Comment",
            Self::Properties => "Properties",
        }
    }

    pub const fn slug(self) -> &'static str {
        match self {
            Self::Design => "design",
            Self::Prototype => "prototype",
            Self::Comment => "comment",
            Self::Properties => "properties",
        }
    }
}
