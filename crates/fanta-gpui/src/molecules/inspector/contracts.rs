//! Domain-neutral controlled-value contracts shared by inspector surfaces.

use gpui::{Pixels, SharedString, px};

use crate::atoms::tokens;

/// Presentation of a controlled value supplied by a host.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub enum InspectorValue<T> {
    /// The host has no value for this field.
    #[default]
    Unset,
    /// Multiple targets do not share one value.
    Mixed,
    /// Every target shares the supplied value.
    Uniform(T),
}

impl<T> InspectorValue<T> {
    pub const fn is_unset(&self) -> bool {
        matches!(self, Self::Unset)
    }

    pub const fn is_mixed(&self) -> bool {
        matches!(self, Self::Mixed)
    }

    pub const fn is_uniform(&self) -> bool {
        matches!(self, Self::Uniform(_))
    }

    pub const fn as_uniform(&self) -> Option<&T> {
        match self {
            Self::Uniform(value) => Some(value),
            Self::Unset | Self::Mixed => None,
        }
    }

    pub fn as_ref(&self) -> InspectorValue<&T> {
        match self {
            Self::Unset => InspectorValue::Unset,
            Self::Mixed => InspectorValue::Mixed,
            Self::Uniform(value) => InspectorValue::Uniform(value),
        }
    }

    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> InspectorValue<U> {
        match self {
            Self::Unset => InspectorValue::Unset,
            Self::Mixed => InspectorValue::Mixed,
            Self::Uniform(value) => InspectorValue::Uniform(map(value)),
        }
    }
}

/// Whether a field may accept an edit in the current host projection.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub enum InspectorFieldAccess {
    #[default]
    Editable,
    ReadOnly {
        reason: Option<SharedString>,
    },
    Disabled {
        reason: Option<SharedString>,
    },
}

/// Presentation metadata that is orthogonal to the field's controlled value.
///
/// Bound and invalid are deliberately separate: a bound value may be valid
/// and inspectable, while an invalid draft may still belong to an otherwise
/// editable field. The optional message is presentation-only and never
/// changes access or edit semantics.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct InspectorFieldPresentation {
    pub access: InspectorFieldAccess,
    pub bound: bool,
    pub invalid: bool,
    pub message: Option<SharedString>,
}

impl InspectorFieldPresentation {
    pub fn new(access: InspectorFieldAccess) -> Self {
        Self {
            access,
            ..Default::default()
        }
    }

    pub const fn bound(mut self, bound: bool) -> Self {
        self.bound = bound;
        self
    }

    pub const fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }

    pub fn message(mut self, message: impl Into<Option<SharedString>>) -> Self {
        self.message = message.into();
        self
    }
}

impl InspectorFieldAccess {
    pub fn read_only(reason: impl Into<Option<SharedString>>) -> Self {
        Self::ReadOnly {
            reason: reason.into(),
        }
    }

    pub fn disabled(reason: impl Into<Option<SharedString>>) -> Self {
        Self::Disabled {
            reason: reason.into(),
        }
    }

    pub const fn is_editable(&self) -> bool {
        matches!(self, Self::Editable)
    }

    /// Read-only fields remain inspectable and copyable; disabled fields do
    /// not participate in field interaction.
    pub const fn is_interactive(&self) -> bool {
        !matches!(self, Self::Disabled { .. })
    }

    pub const fn reason(&self) -> Option<&SharedString> {
        match self {
            Self::Editable => None,
            Self::ReadOnly { reason } | Self::Disabled { reason } => reason.as_ref(),
        }
    }
}

/// Lifecycle of one host-controlled edit transaction.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum InspectorEditPhase {
    Begin,
    Preview,
    Commit,
    Cancel,
}

impl InspectorEditPhase {
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Commit | Self::Cancel)
    }
}

/// Domain-neutral candidate emitted by an inspector field.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct InspectorEdit<T> {
    pub value: T,
    pub phase: InspectorEditPhase,
}

/// An edit emitted by a controlled inspector field.
///
/// Keeping the value presentation in the payload means cancellation can
/// faithfully restore an unset or mixed host value; a field never has to
/// invent a document value merely to terminate its transient interaction.
pub type InspectorControlledEdit<T> = InspectorEdit<InspectorValue<T>>;

impl<T> InspectorEdit<T> {
    pub const fn new(value: T, phase: InspectorEditPhase) -> Self {
        Self { value, phase }
    }

    pub const fn begin(value: T) -> Self {
        Self::new(value, InspectorEditPhase::Begin)
    }

    pub const fn preview(value: T) -> Self {
        Self::new(value, InspectorEditPhase::Preview)
    }

    pub const fn commit(value: T) -> Self {
        Self::new(value, InspectorEditPhase::Commit)
    }

    pub const fn cancel(value: T) -> Self {
        Self::new(value, InspectorEditPhase::Cancel)
    }

    pub const fn is_terminal(&self) -> bool {
        self.phase.is_terminal()
    }
}

/// Shared geometry for dense property inspectors.
///
/// The defaults reproduce the current Design-panel baseline. A host may pass
/// another metrics value to shared molecules without restyling each field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InspectorMetrics {
    pub section_header_height: Pixels,
    pub row_height: Pixels,
    pub horizontal_padding: Pixels,
    pub row_gap: Pixels,
    pub control_gap: Pixels,
    pub label_width: Pixels,
    pub icon_size: Pixels,
    pub radius: Pixels,
    pub compact_feedback_padding: Pixels,
    pub compact_breakpoint: Pixels,
    pub wide_breakpoint: Pixels,
}

/// Density is chosen by the property grid, never by an individual field.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum InspectorDensity {
    #[default]
    Compact,
    Comfortable,
}

/// Placement of labels shared by every row in one property grid.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum InspectorLabelPlacement {
    #[default]
    Leading,
    Stacked,
}

/// Layout policy inherited by rows and labels in one inspector grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InspectorGridLayout {
    pub metrics: InspectorMetrics,
    pub density: InspectorDensity,
    pub label_placement: InspectorLabelPlacement,
}

impl InspectorGridLayout {
    pub const fn new(
        metrics: InspectorMetrics,
        density: InspectorDensity,
        label_placement: InspectorLabelPlacement,
    ) -> Self {
        Self {
            metrics,
            density,
            label_placement,
        }
    }

    pub const fn compact(metrics: InspectorMetrics) -> Self {
        Self::new(
            metrics,
            InspectorDensity::Compact,
            InspectorLabelPlacement::Leading,
        )
    }

    pub fn row_height(self) -> Pixels {
        match self.density {
            InspectorDensity::Compact => self.metrics.row_height,
            InspectorDensity::Comfortable => {
                self.metrics.row_height + self.metrics.row_gap + self.metrics.row_gap
            }
        }
    }

    pub fn grid_gap(self) -> Pixels {
        match self.density {
            InspectorDensity::Compact => self.metrics.row_gap,
            InspectorDensity::Comfortable => self.metrics.control_gap,
        }
    }
}

impl Default for InspectorGridLayout {
    fn default() -> Self {
        Self::compact(InspectorMetrics::default())
    }
}

impl InspectorMetrics {
    /// Canonical numeric geometry used by the current inspector density.
    ///
    /// These constants let retained legacy renderers consume the same source
    /// of truth while they are incrementally migrated to `InspectorMetrics`.
    /// Each one is a named view of the shared geometry scale
    /// ([`crate::atoms::tokens`]); `LABEL_WIDTH` is inspector-specific and so
    /// stays a literal here.
    pub const SECTION_HEADER_HEIGHT: f32 = tokens::RowHeight::SECTION_HEADER;
    pub const ROW_HEIGHT: f32 = tokens::RowHeight::FIELD;
    pub const HORIZONTAL_PADDING: f32 = tokens::Space::LG;
    pub const ROW_GAP: f32 = tokens::Space::XS;
    pub const CONTROL_GAP: f32 = tokens::Space::SM;
    pub const LABEL_WIDTH: f32 = 96.;
    pub const ICON_SIZE: f32 = tokens::IconSize::MD;
    pub const RADIUS: f32 = tokens::Radius::CONTROL;
    pub const COMPACT_FEEDBACK_PADDING: f32 = tokens::Space::MD;
    pub const COMPACT_BREAKPOINT: f32 = tokens::Breakpoint::COMPACT;
    pub const WIDE_BREAKPOINT: f32 = tokens::Breakpoint::WIDE;

    pub fn current() -> Self {
        Self {
            section_header_height: px(Self::SECTION_HEADER_HEIGHT),
            row_height: px(Self::ROW_HEIGHT),
            horizontal_padding: px(Self::HORIZONTAL_PADDING),
            row_gap: px(Self::ROW_GAP),
            control_gap: px(Self::CONTROL_GAP),
            label_width: px(Self::LABEL_WIDTH),
            icon_size: px(Self::ICON_SIZE),
            radius: px(Self::RADIUS),
            compact_feedback_padding: px(Self::COMPACT_FEEDBACK_PADDING),
            compact_breakpoint: px(Self::COMPACT_BREAKPOINT),
            wide_breakpoint: px(Self::WIDE_BREAKPOINT),
        }
    }

    pub fn compact() -> Self {
        Self::current()
    }

    pub fn is_valid(self) -> bool {
        [
            self.section_header_height,
            self.row_height,
            self.horizontal_padding,
            self.control_gap,
            self.label_width,
            self.icon_size,
            self.radius,
            self.compact_feedback_padding,
            self.compact_breakpoint,
            self.wide_breakpoint,
        ]
        .into_iter()
        .all(|value| value.as_f32().is_finite() && value.as_f32() > 0.)
            && self.row_gap.as_f32().is_finite()
            && self.row_gap.as_f32() >= 0.
            && self.compact_breakpoint < self.wide_breakpoint
    }
}

impl Default for InspectorMetrics {
    fn default() -> Self {
        Self::current()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn controlled_values_preserve_empty_and_mixed_states() {
        let unset = InspectorValue::<u8>::default();
        assert!(unset.is_unset());
        assert!(!unset.is_mixed());
        assert!(!unset.is_uniform());
        assert_eq!(unset.as_uniform(), None);

        let mixed = InspectorValue::<u8>::Mixed;
        assert!(!mixed.is_unset());
        assert!(mixed.is_mixed());
        assert!(!mixed.is_uniform());
        assert_eq!(mixed.as_uniform(), None);

        let uniform = InspectorValue::Uniform(2);
        assert!(!uniform.is_unset());
        assert!(!uniform.is_mixed());
        assert!(uniform.is_uniform());
        assert_eq!(uniform.as_uniform(), Some(&2));
        assert_eq!(uniform.as_ref(), InspectorValue::Uniform(&2));
        assert_eq!(uniform.map(|value| value * 3), InspectorValue::Uniform(6));

        let mapped = Cell::new(false);
        let unset = InspectorValue::<u8>::Unset.map(|value| {
            mapped.set(true);
            value
        });
        assert_eq!(unset, InspectorValue::Unset);
        assert!(!mapped.get());
        let mixed = InspectorValue::<u8>::Mixed.map(|value| {
            mapped.set(true);
            value
        });
        assert_eq!(mixed, InspectorValue::Mixed);
        assert!(!mapped.get());
    }

    #[test]
    fn field_access_distinguishes_copyable_from_disabled() {
        let editable = InspectorFieldAccess::Editable;
        assert!(editable.is_interactive());
        assert!(editable.is_editable());
        assert_eq!(editable.reason(), None);

        let read_only = InspectorFieldAccess::read_only(Some(SharedString::from("Locked")));
        assert!(read_only.is_interactive());
        assert!(!read_only.is_editable());
        assert_eq!(read_only.reason().map(SharedString::as_ref), Some("Locked"));
        let disabled = InspectorFieldAccess::disabled(Some(SharedString::from("Unavailable")));
        assert!(!disabled.is_interactive());
        assert!(!disabled.is_editable());
        assert_eq!(
            disabled.reason().map(SharedString::as_ref),
            Some("Unavailable")
        );

        assert_eq!(InspectorFieldAccess::read_only(None).reason(), None);
        assert_eq!(InspectorFieldAccess::disabled(None).reason(), None);
    }

    #[test]
    fn edit_lifecycle_has_exact_terminal_phases() {
        let begin = InspectorEdit::begin(1);
        let preview = InspectorEdit::preview(2);
        let commit = InspectorEdit::commit(3);
        let cancel = InspectorEdit::cancel(1);
        assert_eq!(begin.phase, InspectorEditPhase::Begin);
        assert_eq!(preview.phase, InspectorEditPhase::Preview);
        assert_eq!(commit.phase, InspectorEditPhase::Commit);
        assert_eq!(cancel.phase, InspectorEditPhase::Cancel);
        assert!(!begin.is_terminal());
        assert!(!preview.is_terminal());
        assert!(commit.is_terminal());
        assert!(cancel.is_terminal());
    }

    #[test]
    fn default_metrics_match_the_existing_inspector_baseline() {
        let metrics = InspectorMetrics::default();
        assert_eq!(metrics.section_header_height, px(40.));
        assert_eq!(metrics.row_height, px(24.));
        assert_eq!(metrics.horizontal_padding, px(16.));
        assert_eq!(metrics.label_width, px(96.));
        assert_eq!(metrics.icon_size, px(16.));
        assert_eq!(metrics.compact_breakpoint, px(360.));
        assert_eq!(metrics.wide_breakpoint, px(440.));
        assert!(metrics.is_valid());
    }

    #[test]
    fn inspector_metrics_current_matches_the_shared_geometry_tokens() {
        let metrics = InspectorMetrics::current();
        assert_eq!(
            metrics.section_header_height,
            px(tokens::RowHeight::SECTION_HEADER)
        );
        assert_eq!(metrics.row_height, px(tokens::RowHeight::FIELD));
        assert_eq!(metrics.horizontal_padding, px(tokens::Space::LG));
        assert_eq!(metrics.row_gap, px(tokens::Space::XS));
        assert_eq!(metrics.control_gap, px(tokens::Space::SM));
        // Label width is inspector-specific geometry with no shared token.
        assert_eq!(metrics.label_width, px(96.));
        assert_eq!(metrics.icon_size, px(tokens::IconSize::MD));
        assert_eq!(metrics.radius, px(tokens::Radius::CONTROL));
        assert_eq!(metrics.compact_feedback_padding, px(tokens::Space::MD));
        assert_eq!(metrics.compact_breakpoint, px(tokens::Breakpoint::COMPACT));
        assert_eq!(metrics.wide_breakpoint, px(tokens::Breakpoint::WIDE));
    }

    #[test]
    fn metrics_reject_non_finite_or_non_positive_geometry() {
        let fields = [
            |metrics: &mut InspectorMetrics, value| metrics.section_header_height = value,
            |metrics: &mut InspectorMetrics, value| metrics.row_height = value,
            |metrics: &mut InspectorMetrics, value| metrics.horizontal_padding = value,
            |metrics: &mut InspectorMetrics, value| metrics.control_gap = value,
            |metrics: &mut InspectorMetrics, value| metrics.label_width = value,
            |metrics: &mut InspectorMetrics, value| metrics.icon_size = value,
            |metrics: &mut InspectorMetrics, value| metrics.radius = value,
            |metrics: &mut InspectorMetrics, value| metrics.compact_feedback_padding = value,
            |metrics: &mut InspectorMetrics, value| metrics.compact_breakpoint = value,
            |metrics: &mut InspectorMetrics, value| metrics.wide_breakpoint = value,
        ];
        for set in fields {
            for value in [0., -1., f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                let mut metrics = InspectorMetrics::current();
                set(&mut metrics, px(value));
                assert!(!metrics.is_valid(), "{value:?} must be invalid");
            }
        }

        for value in [-1., f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut metrics = InspectorMetrics::current();
            metrics.row_gap = px(value);
            assert!(!metrics.is_valid(), "row gap {value:?} must be invalid");
        }
        let mut metrics = InspectorMetrics::current();
        metrics.row_gap = px(0.);
        assert!(metrics.is_valid());
    }

    #[test]
    fn field_presentation_keeps_binding_and_validation_independent() {
        let presentation = InspectorFieldPresentation::new(InspectorFieldAccess::read_only(Some(
            SharedString::from("Token bound"),
        )))
        .bound(true)
        .invalid(true)
        .message(Some(SharedString::from("Resolve the source token")));
        assert!(presentation.bound);
        assert!(presentation.invalid);
        assert!(presentation.access.is_interactive());
        assert_eq!(
            presentation.message.as_deref(),
            Some("Resolve the source token")
        );
    }

    #[test]
    fn grid_layout_owns_density_and_label_placement() {
        let metrics = InspectorMetrics::default();
        let compact = InspectorGridLayout::compact(metrics);
        assert_eq!(compact.row_height(), metrics.row_height);
        assert_eq!(compact.grid_gap(), metrics.row_gap);
        assert_eq!(compact.label_placement, InspectorLabelPlacement::Leading);

        let comfortable = InspectorGridLayout::new(
            metrics,
            InspectorDensity::Comfortable,
            InspectorLabelPlacement::Stacked,
        );
        assert!(comfortable.row_height() > compact.row_height());
        assert_eq!(comfortable.grid_gap(), metrics.control_gap);
    }
}
