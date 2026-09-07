use std::collections::HashSet;

use gpui::SharedString;

use super::*;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignLayoutMode {
    None,
    Horizontal,
    Vertical,
    Grid,
}

impl DesignLayoutMode {
    pub const ALL: [Self; 4] = [Self::None, Self::Horizontal, Self::Vertical, Self::Grid];

    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "Freeform",
            Self::Horizontal => "Horizontal",
            Self::Vertical => "Vertical",
            Self::Grid => "Grid",
        }
    }
}

/// Host-controlled availability for a frame-preset catalog, group, or leaf.
///
/// Figma changes its device and asset templates independently of the document
/// schema. Keeping availability in the read model lets a host preserve the
/// exact current list and explain why a known option cannot be applied.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum DesignFramePresetAvailability {
    #[default]
    Available,
    Disabled {
        reason: SharedString,
    },
}

impl DesignFramePresetAvailability {
    pub const fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }

    pub const fn disabled_reason(&self) -> Option<&SharedString> {
        match self {
            Self::Available => None,
            Self::Disabled { reason } => Some(reason),
        }
    }

    pub fn disabled(reason: impl Into<SharedString>) -> Self {
        Self::Disabled {
            reason: reason.into(),
        }
    }
}

/// One exact frame size supplied by the host.
///
/// `id` is stable only within its group. The pair represented by
/// [`DesignFramePresetSelection`] is the canonical identity returned to the
/// host; labels and dimensions remain presentation/application data.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignFramePreset {
    pub id: SharedString,
    pub label: SharedString,
    pub width: f32,
    pub height: f32,
    pub availability: DesignFramePresetAvailability,
}

impl DesignFramePreset {
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        width: f32,
        height: f32,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            width,
            height,
            availability: DesignFramePresetAvailability::Available,
        }
    }

    pub fn disabled(mut self, reason: impl Into<SharedString>) -> Self {
        self.availability = DesignFramePresetAvailability::disabled(reason);
        self
    }

    pub fn is_valid(&self) -> bool {
        !self.id.is_empty()
            && !self.label.is_empty()
            && self.width.is_finite()
            && self.width > 0.
            && self.height.is_finite()
            && self.height > 0.
            && self
                .availability
                .disabled_reason()
                .is_none_or(|reason| !reason.is_empty())
    }
}

/// Stable identity of one frame preset.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DesignFramePresetSelection {
    pub group_id: SharedString,
    pub preset_id: SharedString,
}

impl DesignFramePresetSelection {
    pub fn new(group_id: impl Into<SharedString>, preset_id: impl Into<SharedString>) -> Self {
        Self {
            group_id: group_id.into(),
            preset_id: preset_id.into(),
        }
    }
}

/// One ordered Figma-style frame-preset section such as Phone or Desktop.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignFramePresetGroup {
    pub id: SharedString,
    pub label: SharedString,
    pub presets: Vec<DesignFramePreset>,
    pub availability: DesignFramePresetAvailability,
}

impl DesignFramePresetGroup {
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        presets: impl IntoIterator<Item = DesignFramePreset>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            presets: presets.into_iter().collect(),
            availability: DesignFramePresetAvailability::Available,
        }
    }

    pub fn disabled(mut self, reason: impl Into<SharedString>) -> Self {
        self.availability = DesignFramePresetAvailability::disabled(reason);
        self
    }

    pub fn selection(&self, preset_id: impl Into<SharedString>) -> DesignFramePresetSelection {
        DesignFramePresetSelection::new(self.id.clone(), preset_id)
    }

    pub fn is_valid(&self) -> bool {
        if self.id.is_empty()
            || self.label.is_empty()
            || self.presets.is_empty()
            || self
                .availability
                .disabled_reason()
                .is_some_and(|reason| reason.is_empty())
        {
            return false;
        }
        let mut preset_ids = HashSet::with_capacity(self.presets.len());
        self.presets
            .iter()
            .all(|preset| preset.is_valid() && preset_ids.insert(preset.id.clone()))
    }
}

/// Exact grouped frame-preset catalog for one selected Frame.
///
/// Binding the catalog to `target_node_id` prevents a late asynchronous
/// response from attaching device sizes to a different selection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DesignFramePresetViewData {
    pub target_node_id: SharedString,
    pub groups: Vec<DesignFramePresetGroup>,
    pub availability: DesignFramePresetAvailability,
}

impl DesignFramePresetViewData {
    pub fn new(
        target_node_id: impl Into<SharedString>,
        groups: impl IntoIterator<Item = DesignFramePresetGroup>,
    ) -> Self {
        Self {
            target_node_id: target_node_id.into(),
            groups: groups.into_iter().collect(),
            availability: DesignFramePresetAvailability::Available,
        }
    }

    pub fn disabled(mut self, reason: impl Into<SharedString>) -> Self {
        self.availability = DesignFramePresetAvailability::disabled(reason);
        self
    }

    pub fn is_valid(&self) -> bool {
        if self.target_node_id.is_empty()
            || self.groups.is_empty()
            || self
                .availability
                .disabled_reason()
                .is_some_and(|reason| reason.is_empty())
        {
            return false;
        }
        let mut group_ids = HashSet::with_capacity(self.groups.len());
        self.groups
            .iter()
            .all(|group| group.is_valid() && group_ids.insert(group.id.clone()))
    }

    pub fn preset(
        &self,
        selection: &DesignFramePresetSelection,
    ) -> Option<(&DesignFramePresetGroup, &DesignFramePreset)> {
        let group = self
            .groups
            .iter()
            .find(|group| group.id == selection.group_id)?;
        let preset = group
            .presets
            .iter()
            .find(|preset| preset.id == selection.preset_id)?;
        Some((group, preset))
    }

    pub fn effective_disabled_reason(
        &self,
        selection: &DesignFramePresetSelection,
    ) -> Option<&SharedString> {
        if let Some(reason) = self.availability.disabled_reason() {
            return Some(reason);
        }
        let (group, preset) = self.preset(selection)?;
        group
            .availability
            .disabled_reason()
            .or_else(|| preset.availability.disabled_reason())
    }

    pub fn can_apply(&self, selection: &DesignFramePresetSelection) -> bool {
        self.is_valid()
            && self.preset(selection).is_some()
            && self.effective_disabled_reason(selection).is_none()
    }

    pub fn matching_dimensions(
        &self,
        width: f32,
        height: f32,
    ) -> Option<DesignFramePresetSelection> {
        self.groups.iter().find_map(|group| {
            group
                .presets
                .iter()
                .find(|preset| preset.width == width && preset.height == height)
                .map(|preset| group.selection(preset.id.clone()))
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignSizingMode {
    Fixed,
    Hug,
    Fill,
}

/// One axis in Figma's Width/Height resizing and min/max disclosure.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignLayoutDimensionAxis {
    Width,
    Height,
}

impl DesignLayoutDimensionAxis {
    pub const ALL: [Self; 2] = [Self::Width, Self::Height];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Width => "Width",
            Self::Height => "Height",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DesignAutoLayoutAlignment {
    pub x: u8,
    pub y: u8,
}

impl DesignAutoLayoutAlignment {
    pub const fn new(x: u8, y: u8) -> Self {
        Self {
            x: if x > 2 { 2 } else { x },
            y: if y > 2 { 2 } else { y },
        }
    }
}

impl DesignSizingMode {
    pub const ALL: [Self; 3] = [Self::Fixed, Self::Hug, Self::Fill];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Fixed => "Fixed",
            Self::Hug => "Hug",
            Self::Fill => "Fill",
        }
    }
}

/// How the primary-axis space between auto-layout children is distributed.
///
/// `Auto` corresponds to Figma's "Auto" gap (space between); [`DesignLayout::gap`]
/// remains the concrete gap used by `Fixed`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignItemSpacingMode {
    Fixed,
    Auto,
}

impl DesignItemSpacingMode {
    pub const ALL: [Self; 2] = [Self::Fixed, Self::Auto];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Fixed => "Fixed",
            Self::Auto => "Auto",
        }
    }
}

/// How wrapped tracks consume free space on the counter axis.
///
/// This maps directly to Figma's `counterAxisAlignContent` values. It is only
/// applicable to horizontal auto-layout with Wrap enabled.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignCounterAxisAlignContent {
    Auto,
    SpaceBetween,
}

impl DesignCounterAxisAlignContent {
    pub const ALL: [Self; 2] = [Self::Auto, Self::SpaceBetween];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::SpaceBetween => "Space between",
        }
    }
}

/// Whether a direct child participates in its parent's auto-layout flow.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignLayoutPositioning {
    InFlow,
    Absolute,
}

impl DesignLayoutPositioning {
    pub const ALL: [Self; 2] = [Self::InFlow, Self::Absolute];

    pub const fn label(self) -> &'static str {
        match self {
            Self::InFlow => "In auto layout",
            Self::Absolute => "Ignore auto layout",
        }
    }
}

/// Per-child cross-axis sizing inside an auto-layout parent.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignLayoutAlignSelf {
    Inherit,
    Stretch,
}

impl DesignLayoutAlignSelf {
    pub const ALL: [Self; 2] = [Self::Inherit, Self::Stretch];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Inherit => "Auto",
            Self::Stretch => "Stretch",
        }
    }
}

/// Canvas paint order for children whose bounds overlap.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignStackingOrder {
    LastOnTop,
    FirstOnTop,
}

impl DesignStackingOrder {
    pub const ALL: [Self; 2] = [Self::LastOnTop, Self::FirstOnTop];

    pub const fn label(self) -> &'static str {
        match self {
            Self::LastOnTop => "Last on top",
            Self::FirstOnTop => "First on top",
        }
    }
}

/// Whether horizontal auto-layout children align by their bounds or text baseline.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignBaselineAlignment {
    Bounds,
    Baseline,
}

impl DesignBaselineAlignment {
    pub const ALL: [Self; 2] = [Self::Bounds, Self::Baseline];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Bounds => "Bounds",
            Self::Baseline => "Baseline",
        }
    }
}

/// One explicit track axis in a grid auto-layout.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignGridTrackAxis {
    Column,
    Row,
}

impl DesignGridTrackAxis {
    pub const ALL: [Self; 2] = [Self::Column, Self::Row];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Column => "Column",
            Self::Row => "Row",
        }
    }
}

/// Positive row and column counts for one Grid auto-layout container.
///
/// A dimensions change is intentionally atomic: the panel never decomposes a
/// 2D selector choice into a sequence of per-track insertions or deletions.
/// The host owns track creation, removal, sizing, content relocation, and the
/// complete authoritative vectors echoed through [`DesignLayout`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DesignGridDimensions {
    pub columns: usize,
    pub rows: usize,
}

impl DesignGridDimensions {
    pub const fn new(columns: usize, rows: usize) -> Option<Self> {
        if columns == 0 || rows == 0 {
            None
        } else {
            Some(Self { columns, rows })
        }
    }

    pub const fn is_valid(self) -> bool {
        self.columns > 0 && self.rows > 0
    }
}

/// Whether Figma automatically creates and removes empty grid rows.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignGridAutoTracks {
    None,
    Rows,
}

impl DesignGridAutoTracks {
    pub const ALL: [Self; 2] = [Self::None, Self::Rows];

    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "Manual rows",
            Self::Rows => "Auto rows",
        }
    }
}

/// The sizing algorithm for one row or column in a grid auto-layout.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignGridTrackSizing {
    Fixed,
    Fraction,
    Hug,
}

impl DesignGridTrackSizing {
    pub const ALL: [Self; 3] = [Self::Fixed, Self::Fraction, Self::Hug];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Fixed => "Fixed",
            Self::Fraction => "Fraction",
            Self::Hug => "Hug",
        }
    }
}

/// A grid auto-layout track.
///
/// `value` is pixels for [`DesignGridTrackSizing::Fixed`], fractional units for
/// [`DesignGridTrackSizing::Fraction`], and ignored for
/// [`DesignGridTrackSizing::Hug`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DesignGridTrack {
    pub sizing: DesignGridTrackSizing,
    pub value: f32,
}

impl DesignGridTrack {
    pub const fn fixed(pixels: f32) -> Self {
        Self {
            sizing: DesignGridTrackSizing::Fixed,
            value: pixels,
        }
    }

    pub const fn fraction(units: f32) -> Self {
        Self {
            sizing: DesignGridTrackSizing::Fraction,
            value: units,
        }
    }

    pub const fn hug() -> Self {
        Self {
            sizing: DesignGridTrackSizing::Hug,
            value: 0.,
        }
    }

    pub fn is_valid(self) -> bool {
        self.sizing == DesignGridTrackSizing::Hug || (self.value.is_finite() && self.value > 0.)
    }
}

impl Default for DesignGridTrack {
    fn default() -> Self {
        Self::hug()
    }
}

/// How children are assigned to grid cells.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignGridItemsPositioning {
    Manual,
    RowAutoFlow,
}

impl DesignGridItemsPositioning {
    pub const ALL: [Self; 2] = [Self::Manual, Self::RowAutoFlow];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Manual => "Manual",
            Self::RowAutoFlow => "Auto flow",
        }
    }
}

/// Per-item alignment within a grid auto-layout cell.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignGridItemAlignment {
    Auto,
    Start,
    Center,
    End,
}

impl DesignGridItemAlignment {
    pub const ALL: [Self; 4] = [Self::Auto, Self::Start, Self::Center, Self::End];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Start => "Start",
            Self::Center => "Center",
            Self::End => "End",
        }
    }
}

/// Properties that belong to a selected node as an item of an auto-layout parent.
///
/// Grid row and column indices are zero-based, matching Figma's plugin API.
/// Spans are always expected to be at least one.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignAutoLayoutItem {
    pub positioning: DesignLayoutPositioning,
    pub align_self: DesignLayoutAlignSelf,
    pub layout_grow: f32,
    pub min_width: Option<f32>,
    pub max_width: Option<f32>,
    pub min_height: Option<f32>,
    /// For a Text/TextPath item, a numeric Max height is mutually exclusive
    /// with [`DesignTypography::max_lines`].
    pub max_height: Option<f32>,
    pub grid_row_index: usize,
    pub grid_column_index: usize,
    pub grid_row_span: u16,
    pub grid_column_span: u16,
    pub grid_horizontal_alignment: DesignGridItemAlignment,
    pub grid_vertical_alignment: DesignGridItemAlignment,
}

impl Default for DesignAutoLayoutItem {
    fn default() -> Self {
        Self {
            positioning: DesignLayoutPositioning::InFlow,
            align_self: DesignLayoutAlignSelf::Inherit,
            layout_grow: 0.,
            min_width: None,
            max_width: None,
            min_height: None,
            max_height: None,
            grid_row_index: 0,
            grid_column_index: 0,
            grid_row_span: 1,
            grid_column_span: 1,
            grid_horizontal_alignment: DesignGridItemAlignment::Auto,
            grid_vertical_alignment: DesignGridItemAlignment::Auto,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignLayout {
    pub mode: DesignLayoutMode,
    pub horizontal_sizing: DesignSizingMode,
    pub vertical_sizing: DesignSizingMode,
    pub alignment_x: u8,
    pub alignment_y: u8,
    pub wrap: bool,
    pub gap: f32,
    pub item_spacing_mode: DesignItemSpacingMode,
    /// Figma's `counterAxisSpacing`: `None` links wrapped-track spacing to the
    /// primary gap, while `Some` stores a positive custom spacing.
    ///
    /// Grid currently reuses this field for its explicit row gap and therefore
    /// always supplies `Some`.
    pub counter_axis_gap: Option<f32>,
    pub counter_axis_align_content: DesignCounterAxisAlignContent,
    /// Top, right, bottom, left.
    pub padding: [f32; 4],
    pub clip_content: bool,
    pub include_strokes: bool,
    pub stacking_order: DesignStackingOrder,
    pub baseline_alignment: DesignBaselineAlignment,
    pub grid_columns: Vec<DesignGridTrack>,
    pub grid_rows: Vec<DesignGridTrack>,
    pub grid_auto_tracks: DesignGridAutoTracks,
    pub grid_items_positioning: DesignGridItemsPositioning,
    pub item: DesignAutoLayoutItem,
}

impl Default for DesignLayout {
    fn default() -> Self {
        Self {
            mode: DesignLayoutMode::None,
            horizontal_sizing: DesignSizingMode::Fixed,
            vertical_sizing: DesignSizingMode::Fixed,
            alignment_x: 0,
            alignment_y: 0,
            wrap: false,
            gap: 8.,
            item_spacing_mode: DesignItemSpacingMode::Fixed,
            counter_axis_gap: None,
            counter_axis_align_content: DesignCounterAxisAlignContent::Auto,
            padding: [16.; 4],
            clip_content: true,
            include_strokes: false,
            stacking_order: DesignStackingOrder::LastOnTop,
            baseline_alignment: DesignBaselineAlignment::Bounds,
            grid_columns: Vec::new(),
            grid_rows: Vec::new(),
            grid_auto_tracks: DesignGridAutoTracks::None,
            grid_items_positioning: DesignGridItemsPositioning::Manual,
            item: DesignAutoLayoutItem::default(),
        }
    }
}

impl DesignLayout {
    /// Applies a flow only when the inspected node supports it.
    ///
    /// Returns `false` without changing the current flow when Figma would
    /// reject the operation (for example Grid on a Slot).
    pub fn set_mode_for_kind(&mut self, kind: DesignPanelNodeKind, mode: DesignLayoutMode) -> bool {
        self.set_mode_for_capabilities(
            kind.supports_auto_layout_container(),
            kind.supports_grid_auto_layout(),
            mode,
        )
    }

    /// Applies a flow against an exact host-authored node capability snapshot.
    pub fn set_mode_for_capabilities(
        &mut self,
        supports_auto_layout: bool,
        supports_grid: bool,
        mode: DesignLayoutMode,
    ) -> bool {
        if mode != DesignLayoutMode::None && !supports_auto_layout {
            return false;
        }
        if mode == DesignLayoutMode::Grid && !supports_grid {
            return false;
        }
        let leaving_grid = self.mode == DesignLayoutMode::Grid && mode != DesignLayoutMode::Grid;
        let entering_grid = self.mode != DesignLayoutMode::Grid && mode == DesignLayoutMode::Grid;
        self.mode = mode;
        if mode != DesignLayoutMode::Horizontal {
            self.wrap = false;
            if mode != DesignLayoutMode::Grid {
                self.counter_axis_align_content = DesignCounterAxisAlignContent::Auto;
                self.counter_axis_gap = None;
            }
        }
        if leaving_grid {
            self.counter_axis_align_content = DesignCounterAxisAlignContent::Auto;
            self.counter_axis_gap = None;
        }
        if entering_grid {
            self.horizontal_sizing = DesignSizingMode::Hug;
            self.vertical_sizing = DesignSizingMode::Hug;
            self.counter_axis_gap = Some(
                self.counter_axis_gap
                    .filter(|gap| gap.is_finite() && *gap >= 0.)
                    .unwrap_or_else(|| {
                        if self.gap.is_finite() {
                            self.gap.max(0.)
                        } else {
                            8.
                        }
                    }),
            );
            self.grid_columns = vec![DesignGridTrack::hug()];
            self.grid_rows = vec![DesignGridTrack::hug()];
            self.grid_auto_tracks = DesignGridAutoTracks::Rows;
            self.grid_items_positioning = DesignGridItemsPositioning::RowAutoFlow;
        }
        true
    }

    /// Applies Figma's horizontal-only Wrap invariant.
    pub fn set_wrap(&mut self, wrap: bool) {
        self.wrap = self.mode == DesignLayoutMode::Horizontal && wrap;
        if !self.wrap && self.mode != DesignLayoutMode::Grid {
            self.counter_axis_align_content = DesignCounterAxisAlignContent::Auto;
            self.counter_axis_gap = None;
        }
    }

    /// Applies wrapped-track distribution and clears spacing that Figma ignores.
    pub fn set_counter_axis_align_content(
        &mut self,
        alignment: DesignCounterAxisAlignContent,
    ) -> bool {
        if self.mode != DesignLayoutMode::Horizontal || !self.wrap {
            return false;
        }
        self.counter_axis_align_content = alignment;
        if alignment == DesignCounterAxisAlignContent::SpaceBetween {
            self.counter_axis_gap = None;
        }
        true
    }

    /// Applies Figma's nullable, positive wrapped-track spacing.
    pub fn set_counter_axis_gap(&mut self, spacing: Option<f32>) -> bool {
        if self.mode != DesignLayoutMode::Horizontal
            || !self.wrap
            || self.counter_axis_align_content != DesignCounterAxisAlignContent::Auto
            || spacing.is_some_and(|spacing| !spacing.is_finite() || spacing <= 0.)
        {
            return false;
        }
        self.counter_axis_gap = spacing;
        true
    }

    /// Canonicalizes imported host data to the node's Figma capabilities.
    pub fn normalize_for_kind(&mut self, kind: DesignPanelNodeKind) {
        if !kind.supports_auto_layout_container()
            || (self.mode == DesignLayoutMode::Grid && !kind.supports_grid_auto_layout())
        {
            self.mode = DesignLayoutMode::None;
        }
        if self.mode != DesignLayoutMode::Horizontal {
            self.wrap = false;
        }
        if self.mode == DesignLayoutMode::Horizontal && self.wrap {
            if self.counter_axis_align_content == DesignCounterAxisAlignContent::SpaceBetween
                || self
                    .counter_axis_gap
                    .is_some_and(|spacing| !spacing.is_finite() || spacing <= 0.)
            {
                self.counter_axis_gap = None;
            }
        } else if self.mode != DesignLayoutMode::Grid {
            self.counter_axis_align_content = DesignCounterAxisAlignContent::Auto;
            self.counter_axis_gap = None;
        }
        if self.mode == DesignLayoutMode::Grid {
            self.counter_axis_gap = Some(
                self.counter_axis_gap
                    .filter(|gap| gap.is_finite() && *gap >= 0.)
                    .unwrap_or_else(|| {
                        if self.gap.is_finite() {
                            self.gap.max(0.)
                        } else {
                            8.
                        }
                    }),
            );
            if self.grid_columns.is_empty() {
                self.grid_columns.push(DesignGridTrack::hug());
            }
            if self.grid_rows.is_empty() {
                self.grid_rows.push(DesignGridTrack::hug());
            }
            for track in &mut self.grid_columns {
                if !track.is_valid()
                    || (self.horizontal_sizing == DesignSizingMode::Hug
                        && track.sizing == DesignGridTrackSizing::Fraction)
                {
                    *track = DesignGridTrack::hug();
                }
            }
            for track in &mut self.grid_rows {
                if !track.is_valid()
                    || (self.vertical_sizing == DesignSizingMode::Hug
                        && track.sizing == DesignGridTrackSizing::Fraction)
                {
                    *track = DesignGridTrack::hug();
                }
            }
        }
    }

    pub fn grid_tracks(&self, axis: DesignGridTrackAxis) -> &[DesignGridTrack] {
        match axis {
            DesignGridTrackAxis::Column => &self.grid_columns,
            DesignGridTrackAxis::Row => &self.grid_rows,
        }
    }

    pub fn grid_track_count(&self, axis: DesignGridTrackAxis) -> usize {
        self.grid_tracks(axis).len()
    }

    pub fn grid_dimensions(&self) -> Option<DesignGridDimensions> {
        (self.mode == DesignLayoutMode::Grid)
            .then(|| DesignGridDimensions::new(self.grid_columns.len(), self.grid_rows.len()))
            .flatten()
    }

    pub fn grid_track_count_is_editable(&self, axis: DesignGridTrackAxis) -> bool {
        self.mode == DesignLayoutMode::Grid
            && !(axis == DesignGridTrackAxis::Row
                && self.grid_auto_tracks == DesignGridAutoTracks::Rows)
    }

    pub fn can_delete_grid_track(&self, axis: DesignGridTrackAxis) -> bool {
        self.grid_track_count_is_editable(axis) && self.grid_track_count(axis) > 1
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignConstraint {
    Left,
    Right,
    LeftAndRight,
    Center,
    Scale,
    Top,
    Bottom,
    TopAndBottom,
}

impl DesignConstraint {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Right => "Right",
            Self::LeftAndRight => "Left & right",
            Self::Center => "Center",
            Self::Scale => "Scale",
            Self::Top => "Top",
            Self::Bottom => "Bottom",
            Self::TopAndBottom => "Top & bottom",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignGridKind {
    Uniform,
    Columns,
    Rows,
}

impl DesignGridKind {
    pub const ALL: [Self; 3] = [Self::Uniform, Self::Columns, Self::Rows];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Uniform => "Grid",
            Self::Columns => "Columns",
            Self::Rows => "Rows",
        }
    }
}

/// Count used by row and column layout guides.
///
/// `Auto` asks the host to derive as many tracks as fit the selected frame.
/// A concrete count is always at least one; constructors normalize zero.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignLayoutGridCount {
    Auto,
    Number(u16),
}

impl DesignLayoutGridCount {
    pub const fn number(count: u16) -> Self {
        Self::Number(if count == 0 { 1 } else { count })
    }

    pub const fn numeric(self) -> Option<u16> {
        match self {
            Self::Auto => None,
            Self::Number(count) => Some(count),
        }
    }

    pub fn label(self) -> SharedString {
        match self {
            Self::Auto => "Auto".into(),
            Self::Number(count) => count.to_string().into(),
        }
    }

    pub const fn is_auto(self) -> bool {
        matches!(self, Self::Auto)
    }
}

/// Exact Figma field on one layout guide that can retain a Number variable.
///
/// `offset` is shared by the fixed-guide Offset control and the stretch-guide
/// Margin control. [`DesignLayoutGridVariableTarget::property`] preserves
/// which inspector leaf was visible when an intent was emitted.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignLayoutGridVariableField {
    SectionSize,
    Count,
    Offset,
    GutterSize,
}

impl DesignLayoutGridVariableField {
    pub const fn api_name(self) -> &'static str {
        match self {
            Self::SectionSize => "sectionSize",
            Self::Count => "count",
            Self::Offset => "offset",
            Self::GutterSize => "gutterSize",
        }
    }
}

/// Stable, lossless target for one variable-bindable layout-guide leaf.
///
/// A non-empty `guide_id` is authoritative. `index` and the index embedded in
/// `property` are compatibility hints that hosts resolve again against their
/// latest ordered guide snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignLayoutGridVariableTarget {
    pub guide_id: SharedString,
    pub index: usize,
    pub property: DesignPanelProperty,
    pub field: DesignLayoutGridVariableField,
}

impl DesignLayoutGridVariableTarget {
    pub fn new(
        guide_id: impl Into<SharedString>,
        index: usize,
        property: DesignPanelProperty,
        field: DesignLayoutGridVariableField,
    ) -> Self {
        Self {
            guide_id: guide_id.into(),
            index,
            property: property.with_layout_grid_index(index),
            field,
        }
    }

    pub const fn with_index(mut self, index: usize) -> Self {
        self.index = index;
        self.property = self.property.with_layout_grid_index(index);
        self
    }
}

/// Typed resolved value used to validate and create layout-guide variables.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DesignLayoutGridVariableValue {
    Number(f32),
    Count(DesignLayoutGridCount),
}

impl DesignLayoutGridVariableValue {
    pub fn label(self) -> SharedString {
        match self {
            Self::Number(value) => {
                let rounded = value.round();
                if (value - rounded).abs() <= f32::EPSILON {
                    format!("{rounded:.0}").into()
                } else {
                    format!("{value:.2}").into()
                }
            }
            Self::Count(value) => value.label(),
        }
    }

    /// Returns a host- or type-authored reason that this Number variable
    /// cannot be applied to the exact layout-guide leaf.
    pub fn compatibility_reason(self, variable: &DesignVariable) -> Option<SharedString> {
        if let Some(reason) = &variable.disabled_reason {
            return Some(reason.clone());
        }
        if variable.resolved_type != DesignVariableResolvedType::Float {
            return Some("Layout guides require a Number variable".into());
        }
        let Some(DesignVariableResolvedValue::Float(resolved)) = variable.resolved_value else {
            return Some("A resolved Number value is required for this guide".into());
        };
        match self {
            Self::Number(_) if resolved.is_finite() && resolved >= 0. => None,
            Self::Number(_) => Some("This guide requires a finite non-negative number".into()),
            Self::Count(DesignLayoutGridCount::Auto)
                if resolved.is_infinite() && resolved.is_sign_positive() =>
            {
                None
            }
            Self::Count(DesignLayoutGridCount::Auto) => {
                Some("This guide requires an Auto count variable".into())
            }
            Self::Count(DesignLayoutGridCount::Number(_))
                if resolved.is_finite()
                    && resolved >= 1.
                    && resolved <= u16::MAX as f32
                    && resolved.fract().abs() <= f32::EPSILON =>
            {
                None
            }
            Self::Count(DesignLayoutGridCount::Number(_)) => {
                Some("This guide requires a positive whole-number count variable".into())
            }
        }
    }

    pub fn is_compatible(self, variable: &DesignVariable) -> bool {
        self.compatibility_reason(variable).is_none()
    }
}

/// Whether the host exposes variable creation for the active guide target.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum DesignLayoutGridVariableCreateState {
    #[default]
    Hidden,
    Enabled,
    Disabled {
        reason: SharedString,
    },
}

impl DesignLayoutGridVariableCreateState {
    pub const fn is_visible(&self) -> bool {
        !matches!(self, Self::Hidden)
    }

    pub const fn is_enabled(&self) -> bool {
        matches!(self, Self::Enabled)
    }

    pub const fn disabled_reason(&self) -> Option<&SharedString> {
        match self {
            Self::Disabled { reason } => Some(reason),
            Self::Hidden | Self::Enabled => None,
        }
    }
}

/// Host-controlled Number-variable catalog shared by every bindable layout
/// guide leaf. Source/import state, collection identity, search metadata, and
/// disabled reasons remain the lossless [`DesignVariable`] contract.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DesignLayoutGridVariableViewData {
    pub variables: Vec<DesignVariable>,
    pub create_state: DesignLayoutGridVariableCreateState,
}

impl DesignLayoutGridVariableViewData {
    pub fn new(variables: impl IntoIterator<Item = DesignVariable>) -> Self {
        Self {
            variables: variables
                .into_iter()
                .filter(|variable| {
                    variable.resolved_type == DesignVariableResolvedType::Float
                        && matches!(
                            variable.resolved_value.as_ref(),
                            None | Some(DesignVariableResolvedValue::Float(_))
                        )
                })
                .collect(),
            create_state: DesignLayoutGridVariableCreateState::Hidden,
        }
    }

    pub fn with_create_state(mut self, create_state: DesignLayoutGridVariableCreateState) -> Self {
        self.create_state = create_state;
        self
    }

    pub fn variable(&self, variable_id: &str) -> Option<&DesignVariable> {
        self.variables.iter().find(|variable| {
            variable.id.as_ref() == variable_id
                && variable.resolved_type == DesignVariableResolvedType::Float
                && matches!(
                    variable.resolved_value.as_ref(),
                    None | Some(DesignVariableResolvedValue::Float(_))
                )
        })
    }

    pub fn variable_mut(&mut self, variable_id: &str) -> Option<&mut DesignVariable> {
        self.variables.iter_mut().find(|variable| {
            variable.id.as_ref() == variable_id
                && variable.resolved_type == DesignVariableResolvedType::Float
                && matches!(
                    variable.resolved_value.as_ref(),
                    None | Some(DesignVariableResolvedValue::Float(_))
                )
        })
    }
}

/// Compatibility-only variable candidate for a row/column guide count.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignLayoutGridCountVariable {
    pub id: SharedString,
    pub name: SharedString,
    pub collection_name: SharedString,
    pub resolved_count: DesignLayoutGridCount,
    /// Host-authored reason that prevents applying this variable.
    pub disabled_reason: Option<SharedString>,
}

impl DesignLayoutGridCountVariable {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        collection_name: impl Into<SharedString>,
        resolved_count: DesignLayoutGridCount,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            collection_name: collection_name.into(),
            resolved_count,
            disabled_reason: None,
        }
    }

    pub fn disabled(mut self, reason: impl Into<SharedString>) -> Self {
        self.disabled_reason = Some(reason.into());
        self
    }

    /// Count variables preserve Figma's Number/Auto discriminant exactly.
    /// A numeric count cannot silently accept an Auto variable or vice versa.
    pub fn compatibility_reason(
        &self,
        current_count: DesignLayoutGridCount,
    ) -> Option<SharedString> {
        if let Some(reason) = &self.disabled_reason {
            return Some(reason.clone());
        }
        if self.resolved_count.is_auto() == current_count.is_auto() {
            None
        } else if current_count.is_auto() {
            Some("This guide requires an Auto count variable".into())
        } else {
            Some("This guide requires a numeric count variable".into())
        }
    }

    pub fn is_compatible(&self, current_count: DesignLayoutGridCount) -> bool {
        self.compatibility_reason(current_count).is_none()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DesignLayoutGridCountVariableViewData {
    pub variables: Vec<DesignLayoutGridCountVariable>,
}

impl DesignLayoutGridCountVariableViewData {
    pub fn new(variables: impl IntoIterator<Item = DesignLayoutGridCountVariable>) -> Self {
        Self {
            variables: variables.into_iter().collect(),
        }
    }

    pub fn variable(&self, variable_id: &str) -> Option<&DesignLayoutGridCountVariable> {
        self.variables
            .iter()
            .find(|variable| variable.id.as_ref() == variable_id)
    }

    pub fn generalized(&self) -> DesignLayoutGridVariableViewData {
        DesignLayoutGridVariableViewData::new(self.variables.iter().cloned().map(|variable| {
            let resolved_value = match variable.resolved_count {
                DesignLayoutGridCount::Auto => f32::INFINITY,
                DesignLayoutGridCount::Number(count) => f32::from(count),
            };
            let mut value = DesignVariable::page(
                variable.id,
                variable.name,
                variable.collection_name.clone(),
                variable.collection_name,
                DesignVariableResolvedType::Float,
            )
            .with_resolved_value(DesignVariableResolvedValue::Float(resolved_value));
            value.disabled_reason = variable.disabled_reason;
            value
        }))
    }
}

/// Opaque variable identity for one Number-valued layout-guide field.
///
/// The panel displays the host-resolved raw value but prevents a direct edit
/// until the host accepts a detach intent. `read_only_reason` and
/// `can_detach` let a host distinguish an inspectable binding from one the
/// current user may replace or detach.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignLayoutGridVariableBinding {
    pub variable_id: SharedString,
    pub variable_name: SharedString,
    pub collection_name: Option<SharedString>,
    pub can_detach: bool,
    pub read_only_reason: Option<SharedString>,
}

impl DesignLayoutGridVariableBinding {
    pub fn new(
        variable_id: impl Into<SharedString>,
        variable_name: impl Into<SharedString>,
    ) -> Self {
        Self {
            variable_id: variable_id.into(),
            variable_name: variable_name.into(),
            collection_name: None,
            can_detach: true,
            read_only_reason: None,
        }
    }

    pub fn with_collection(mut self, collection_name: impl Into<SharedString>) -> Self {
        self.collection_name = Some(collection_name.into());
        self
    }

    pub const fn detachable(mut self, can_detach: bool) -> Self {
        self.can_detach = can_detach;
        self
    }

    pub fn read_only(mut self, reason: impl Into<SharedString>) -> Self {
        self.can_detach = false;
        self.read_only_reason = Some(reason.into());
        self
    }
}

/// One exact Figma field binding on a host-owned layout guide.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignLayoutGridBoundVariable {
    pub field: DesignLayoutGridVariableField,
    pub binding: DesignLayoutGridVariableBinding,
}

impl DesignLayoutGridBoundVariable {
    pub const fn new(
        field: DesignLayoutGridVariableField,
        binding: DesignLayoutGridVariableBinding,
    ) -> Self {
        Self { field, binding }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum DesignLayoutGridStyleSource {
    Page,
    Library { library_id: SharedString },
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DesignLayoutGridStyleSelection {
    pub source: DesignLayoutGridStyleSource,
    pub style_id: SharedString,
}

impl DesignLayoutGridStyleSelection {
    pub fn page(style_id: impl Into<SharedString>) -> Self {
        Self {
            source: DesignLayoutGridStyleSource::Page,
            style_id: style_id.into(),
        }
    }

    pub fn library(library_id: impl Into<SharedString>, style_id: impl Into<SharedString>) -> Self {
        Self {
            source: DesignLayoutGridStyleSource::Library {
                library_id: library_id.into(),
            },
            style_id: style_id.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum DesignLayoutGridStyleImportState {
    #[default]
    Imported,
    Available,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignLayoutGridStyle {
    pub id: SharedString,
    pub name: SharedString,
    /// Complete ordered guide snapshot stored by Figma's `GridStyle`.
    pub layout_grids: Vec<DesignLayoutGrid>,
    pub import_state: DesignLayoutGridStyleImportState,
}

impl DesignLayoutGridStyle {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        layout_grids: impl IntoIterator<Item = DesignLayoutGrid>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            layout_grids: layout_grids.into_iter().collect(),
            import_state: DesignLayoutGridStyleImportState::Imported,
        }
    }

    pub const fn available(mut self) -> Self {
        self.import_state = DesignLayoutGridStyleImportState::Available;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignLayoutGridStyleLibrary {
    pub id: SharedString,
    pub name: SharedString,
    pub styles: Vec<DesignLayoutGridStyle>,
}

impl DesignLayoutGridStyleLibrary {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        styles: impl IntoIterator<Item = DesignLayoutGridStyle>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            styles: styles.into_iter().collect(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DesignLayoutGridStyleViewData {
    pub page_styles: Vec<DesignLayoutGridStyle>,
    pub libraries: Vec<DesignLayoutGridStyleLibrary>,
}

impl DesignLayoutGridStyleViewData {
    pub fn new(
        page_styles: impl IntoIterator<Item = DesignLayoutGridStyle>,
        libraries: impl IntoIterator<Item = DesignLayoutGridStyleLibrary>,
    ) -> Self {
        Self {
            page_styles: page_styles.into_iter().collect(),
            libraries: libraries.into_iter().collect(),
        }
    }

    pub fn style(
        &self,
        selection: &DesignLayoutGridStyleSelection,
    ) -> Option<&DesignLayoutGridStyle> {
        match &selection.source {
            DesignLayoutGridStyleSource::Page => self
                .page_styles
                .iter()
                .find(|style| style.id == selection.style_id),
            DesignLayoutGridStyleSource::Library { library_id } => self
                .libraries
                .iter()
                .find(|library| library.id == *library_id)?
                .styles
                .iter()
                .find(|style| style.id == selection.style_id),
        }
    }
}

/// Opaque node-level layout-guide style identity.
///
/// Figma exposes one `gridStyleId` for the node's complete ordered
/// `layoutGrids` array. Style application remains host-owned.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignLayoutGridStyleBinding {
    pub style_id: SharedString,
    pub style_name: SharedString,
    pub source: DesignLayoutGridStyleSource,
    pub can_detach: bool,
}

impl DesignLayoutGridStyleBinding {
    pub fn new(style_id: impl Into<SharedString>, style_name: impl Into<SharedString>) -> Self {
        Self {
            style_id: style_id.into(),
            style_name: style_name.into(),
            source: DesignLayoutGridStyleSource::Page,
            can_detach: true,
        }
    }

    pub fn library(
        library_id: impl Into<SharedString>,
        style_id: impl Into<SharedString>,
        style_name: impl Into<SharedString>,
    ) -> Self {
        Self {
            style_id: style_id.into(),
            style_name: style_name.into(),
            source: DesignLayoutGridStyleSource::Library {
                library_id: library_id.into(),
            },
            can_detach: true,
        }
    }

    pub fn selection(&self) -> DesignLayoutGridStyleSelection {
        DesignLayoutGridStyleSelection {
            source: self.source.clone(),
            style_id: self.style_id.clone(),
        }
    }

    pub const fn with_detach_allowed(mut self, can_detach: bool) -> Self {
        self.can_detach = can_detach;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignColumnGridAlignment {
    Left,
    Center,
    Right,
    Stretch,
}

impl DesignColumnGridAlignment {
    pub const ALL: [Self; 4] = [Self::Left, Self::Center, Self::Right, Self::Stretch];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Center => "Center",
            Self::Right => "Right",
            Self::Stretch => "Stretch",
        }
    }

    pub const fn is_stretch(self) -> bool {
        matches!(self, Self::Stretch)
    }

    /// Centered fixed guides do not expose an edge offset in Figma.
    pub const fn supports_offset(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignRowGridAlignment {
    Top,
    Center,
    Bottom,
    Stretch,
}

impl DesignRowGridAlignment {
    pub const ALL: [Self; 4] = [Self::Top, Self::Center, Self::Bottom, Self::Stretch];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Top => "Top",
            Self::Center => "Center",
            Self::Bottom => "Bottom",
            Self::Stretch => "Stretch",
        }
    }

    pub const fn is_stretch(self) -> bool {
        matches!(self, Self::Stretch)
    }

    /// Centered fixed guides do not expose an edge offset in Figma.
    pub const fn supports_offset(self) -> bool {
        matches!(self, Self::Top | Self::Bottom)
    }
}

/// Uniform square-grid guide settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DesignUniformLayoutGrid {
    pub size: f32,
}

impl Default for DesignUniformLayoutGrid {
    fn default() -> Self {
        Self { size: 10. }
    }
}

/// Column guide settings.
///
/// `size` and `offset` apply to Left/Center/Right fixed guides. Stretch
/// columns have an implicit Auto size and instead use `margin`; all alignment
/// modes use `gutter`.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignColumnLayoutGrid {
    pub alignment: DesignColumnGridAlignment,
    pub count: DesignLayoutGridCount,
    pub count_binding: Option<DesignLayoutGridVariableBinding>,
    pub size: f32,
    pub offset: f32,
    pub gutter: f32,
    pub margin: f32,
}

impl Default for DesignColumnLayoutGrid {
    fn default() -> Self {
        Self {
            alignment: DesignColumnGridAlignment::Stretch,
            count: DesignLayoutGridCount::number(12),
            count_binding: None,
            size: 72.,
            offset: 0.,
            gutter: 24.,
            margin: 32.,
        }
    }
}

/// Row guide settings.
///
/// `size` and `offset` apply to Top/Center/Bottom fixed guides. Stretch rows
/// have an implicit Auto size and instead use `margin`; all alignment modes
/// use `gutter`.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignRowLayoutGrid {
    pub alignment: DesignRowGridAlignment,
    pub count: DesignLayoutGridCount,
    pub count_binding: Option<DesignLayoutGridVariableBinding>,
    pub size: f32,
    pub offset: f32,
    pub gutter: f32,
    pub margin: f32,
}

impl Default for DesignRowLayoutGrid {
    fn default() -> Self {
        Self {
            alignment: DesignRowGridAlignment::Stretch,
            count: DesignLayoutGridCount::number(8),
            count_binding: None,
            size: 8.,
            offset: 0.,
            gutter: 8.,
            margin: 24.,
        }
    }
}

/// Settings discriminated by Figma layout-guide type.
#[derive(Clone, Debug, PartialEq)]
pub enum DesignLayoutGridSettings {
    Uniform(DesignUniformLayoutGrid),
    Columns(DesignColumnLayoutGrid),
    Rows(DesignRowLayoutGrid),
}

impl DesignLayoutGridSettings {
    pub const fn kind(&self) -> DesignGridKind {
        match self {
            Self::Uniform(_) => DesignGridKind::Uniform,
            Self::Columns(_) => DesignGridKind::Columns,
            Self::Rows(_) => DesignGridKind::Rows,
        }
    }
}

impl From<DesignGridKind> for DesignLayoutGridSettings {
    fn from(kind: DesignGridKind) -> Self {
        match kind {
            DesignGridKind::Uniform => Self::Uniform(DesignUniformLayoutGrid::default()),
            DesignGridKind::Columns => Self::Columns(DesignColumnLayoutGrid::default()),
            DesignGridKind::Rows => Self::Rows(DesignRowLayoutGrid::default()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignLayoutGrid {
    /// Stable opaque host identity. Empty retains the legacy index fallback.
    pub id: SharedString,
    pub visible: bool,
    pub settings: DesignLayoutGridSettings,
    pub color: DesignColor,
    /// Guide opacity as a percentage in the inclusive 0–100 range.
    pub opacity: f32,
    /// Exact Number-variable bindings keyed by Figma layout-grid field.
    ///
    /// The legacy row/column `count_binding` remains a compatibility fallback
    /// when no canonical Count entry is present here.
    pub variable_bindings: Vec<DesignLayoutGridBoundVariable>,
}

impl DesignLayoutGrid {
    pub fn uniform(size: f32, color: DesignColor) -> Self {
        Self {
            id: "".into(),
            visible: true,
            settings: DesignLayoutGridSettings::Uniform(DesignUniformLayoutGrid { size }),
            color,
            opacity: 10.,
            variable_bindings: Vec::new(),
        }
    }

    pub fn columns(settings: DesignColumnLayoutGrid, color: DesignColor) -> Self {
        Self {
            id: "".into(),
            visible: true,
            settings: DesignLayoutGridSettings::Columns(settings),
            color,
            opacity: 10.,
            variable_bindings: Vec::new(),
        }
    }

    pub fn rows(settings: DesignRowLayoutGrid, color: DesignColor) -> Self {
        Self {
            id: "".into(),
            visible: true,
            settings: DesignLayoutGridSettings::Rows(settings),
            color,
            opacity: 10.,
            variable_bindings: Vec::new(),
        }
    }

    pub const fn kind(&self) -> DesignGridKind {
        self.settings.kind()
    }

    pub fn with_id(mut self, id: impl Into<SharedString>) -> Self {
        self.id = id.into();
        self
    }

    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity.clamp(0., 100.);
        self
    }

    pub fn with_variable_binding(
        mut self,
        field: DesignLayoutGridVariableField,
        binding: DesignLayoutGridVariableBinding,
    ) -> Self {
        self.set_variable_binding(field, Some(binding));
        self
    }

    pub fn set_variable_binding(
        &mut self,
        field: DesignLayoutGridVariableField,
        binding: Option<DesignLayoutGridVariableBinding>,
    ) {
        if field == DesignLayoutGridVariableField::Count {
            match &mut self.settings {
                DesignLayoutGridSettings::Columns(settings) => settings.count_binding = None,
                DesignLayoutGridSettings::Rows(settings) => settings.count_binding = None,
                DesignLayoutGridSettings::Uniform(_) => {}
            }
        }
        self.variable_bindings
            .retain(|current| current.field != field);
        if let Some(binding) = binding {
            self.variable_bindings
                .push(DesignLayoutGridBoundVariable::new(field, binding));
        }
    }

    pub fn variable_binding(
        &self,
        field: DesignLayoutGridVariableField,
    ) -> Option<&DesignLayoutGridVariableBinding> {
        self.variable_bindings
            .iter()
            .find(|current| current.field == field)
            .map(|current| &current.binding)
            .or_else(|| {
                (field == DesignLayoutGridVariableField::Count)
                    .then_some(match &self.settings {
                        DesignLayoutGridSettings::Columns(settings) => {
                            settings.count_binding.as_ref()
                        }
                        DesignLayoutGridSettings::Rows(settings) => settings.count_binding.as_ref(),
                        DesignLayoutGridSettings::Uniform(_) => None,
                    })
                    .flatten()
            })
    }

    /// Resolves one visible inspector property onto the exact Figma
    /// variable-bindable guide field and its current typed value.
    pub fn variable_target(
        &self,
        index: usize,
        property: DesignPanelProperty,
    ) -> Option<(
        DesignLayoutGridVariableTarget,
        DesignLayoutGridVariableValue,
    )> {
        let property = property.with_layout_grid_index(index);
        let (field, value) = match (&self.settings, property) {
            (
                DesignLayoutGridSettings::Uniform(settings),
                DesignPanelProperty::LayoutGridSize(_),
            ) => (
                DesignLayoutGridVariableField::SectionSize,
                DesignLayoutGridVariableValue::Number(settings.size),
            ),
            (
                DesignLayoutGridSettings::Columns(settings),
                DesignPanelProperty::LayoutGridCount(_),
            ) => (
                DesignLayoutGridVariableField::Count,
                DesignLayoutGridVariableValue::Count(settings.count),
            ),
            (DesignLayoutGridSettings::Rows(settings), DesignPanelProperty::LayoutGridCount(_)) => {
                (
                    DesignLayoutGridVariableField::Count,
                    DesignLayoutGridVariableValue::Count(settings.count),
                )
            }
            (
                DesignLayoutGridSettings::Columns(settings),
                DesignPanelProperty::LayoutGridSize(_),
            ) if !settings.alignment.is_stretch() => (
                DesignLayoutGridVariableField::SectionSize,
                DesignLayoutGridVariableValue::Number(settings.size),
            ),
            (DesignLayoutGridSettings::Rows(settings), DesignPanelProperty::LayoutGridSize(_))
                if !settings.alignment.is_stretch() =>
            {
                (
                    DesignLayoutGridVariableField::SectionSize,
                    DesignLayoutGridVariableValue::Number(settings.size),
                )
            }
            (
                DesignLayoutGridSettings::Columns(settings),
                DesignPanelProperty::LayoutGridOffset(_),
            ) if settings.alignment.supports_offset() => (
                DesignLayoutGridVariableField::Offset,
                DesignLayoutGridVariableValue::Number(settings.offset),
            ),
            (
                DesignLayoutGridSettings::Rows(settings),
                DesignPanelProperty::LayoutGridOffset(_),
            ) if settings.alignment.supports_offset() => (
                DesignLayoutGridVariableField::Offset,
                DesignLayoutGridVariableValue::Number(settings.offset),
            ),
            (
                DesignLayoutGridSettings::Columns(settings),
                DesignPanelProperty::LayoutGridMargin(_),
            ) if settings.alignment.is_stretch() => (
                DesignLayoutGridVariableField::Offset,
                DesignLayoutGridVariableValue::Number(settings.margin),
            ),
            (
                DesignLayoutGridSettings::Rows(settings),
                DesignPanelProperty::LayoutGridMargin(_),
            ) if settings.alignment.is_stretch() => (
                DesignLayoutGridVariableField::Offset,
                DesignLayoutGridVariableValue::Number(settings.margin),
            ),
            (
                DesignLayoutGridSettings::Columns(settings),
                DesignPanelProperty::LayoutGridGutter(_),
            ) => (
                DesignLayoutGridVariableField::GutterSize,
                DesignLayoutGridVariableValue::Number(settings.gutter),
            ),
            (
                DesignLayoutGridSettings::Rows(settings),
                DesignPanelProperty::LayoutGridGutter(_),
            ) => (
                DesignLayoutGridVariableField::GutterSize,
                DesignLayoutGridVariableValue::Number(settings.gutter),
            ),
            _ => return None,
        };
        Some((
            DesignLayoutGridVariableTarget::new(self.id.clone(), index, property, field),
            value,
        ))
    }
}

impl Default for DesignLayoutGrid {
    fn default() -> Self {
        Self::uniform(10., DesignColor::BLUE)
    }
}

/// Host-owned availability for Figma's native “Add auto layout” operation.
///
/// Structural eligibility cannot be inferred from an aggregate inspector node:
/// groups and arbitrary layer selections may be eligible even though neither
/// owns an auto-layout flow yet. The target binds an asynchronous projection to
/// the exact ordered selection that was evaluated. `disabled_reason` keeps an
/// otherwise eligible affordance visible but inert, for example while one
/// selected layer is locked.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignAddAutoLayoutViewData {
    pub target: DesignPanelTarget,
    pub structurally_eligible: bool,
    pub disabled_reason: Option<SharedString>,
}

impl DesignAddAutoLayoutViewData {
    pub const fn eligible(target: DesignPanelTarget) -> Self {
        Self {
            target,
            structurally_eligible: true,
            disabled_reason: None,
        }
    }

    pub const fn ineligible(target: DesignPanelTarget) -> Self {
        Self {
            target,
            structurally_eligible: false,
            disabled_reason: None,
        }
    }

    pub fn disabled(mut self, reason: impl Into<SharedString>) -> Self {
        self.disabled_reason = Some(reason.into());
        self
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
            && node_ids.iter().all(|node_id| !node_id.is_empty())
            && self
                .disabled_reason
                .as_ref()
                .is_none_or(|reason| !reason.is_empty())
    }

    pub const fn can_request(&self) -> bool {
        self.structurally_eligible && self.disabled_reason.is_none()
    }
}

/// Geometry class of an exact multiple selection for Figma Smart Selection.
///
/// `None` means the selection is not currently evenly spaced. A host can
/// still expose Tidy up or distribution controls that create a Smart
/// Selection. The other variants determine which Space between fields are
/// valid.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum DesignSmartSelectionKind {
    #[default]
    None,
    Horizontal,
    Vertical,
    TwoDimensional,
}

impl DesignSmartSelectionKind {
    pub const fn supports_axis(self, axis: DesignSmartSelectionAxis) -> bool {
        matches!(
            (self, axis),
            (
                Self::Horizontal | Self::TwoDimensional,
                DesignSmartSelectionAxis::Horizontal
            ) | (
                Self::Vertical | Self::TwoDimensional,
                DesignSmartSelectionAxis::Vertical
            )
        )
    }
}

/// Axis of one native Smart Selection Space between field.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignSmartSelectionAxis {
    Horizontal,
    Vertical,
}

impl DesignSmartSelectionAxis {
    pub const ALL: [Self; 2] = [Self::Horizontal, Self::Vertical];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Horizontal => "Horizontal space between",
            Self::Vertical => "Vertical space between",
        }
    }

    pub const fn glyph(self) -> &'static str {
        match self {
            Self::Horizontal => "↔",
            Self::Vertical => "↕",
        }
    }
}

/// Host-owned value shown by one Smart Selection spacing field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DesignSmartSelectionSpacingValue {
    Mixed,
    Uniform(f32),
}

impl DesignSmartSelectionSpacingValue {
    pub const fn is_mixed(self) -> bool {
        matches!(self, Self::Mixed)
    }

    pub fn uniform(self) -> Option<f32> {
        match self {
            Self::Mixed => None,
            Self::Uniform(value) if value.is_finite() => Some(value),
            Self::Uniform(_) => None,
        }
    }

    pub fn is_valid(self) -> bool {
        matches!(self, Self::Mixed) || self.uniform().is_some()
    }
}

/// Host-authored availability and disabled explanation for a Smart Selection
/// field or command.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum DesignSmartSelectionAvailability {
    #[default]
    Available,
    Disabled {
        reason: SharedString,
    },
}

impl DesignSmartSelectionAvailability {
    pub const fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }

    pub const fn disabled_reason(&self) -> Option<&SharedString> {
        match self {
            Self::Available => None,
            Self::Disabled { reason } => Some(reason),
        }
    }

    pub fn disabled(reason: impl Into<SharedString>) -> Self {
        Self::Disabled {
            reason: reason.into(),
        }
    }

    fn is_valid(&self) -> bool {
        self.disabled_reason()
            .is_none_or(|reason| !reason.trim().is_empty())
    }
}

/// One host-controlled Smart Selection spacing readout.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignSmartSelectionSpacingViewData {
    pub value: DesignSmartSelectionSpacingValue,
    pub availability: DesignSmartSelectionAvailability,
}

impl DesignSmartSelectionSpacingViewData {
    pub const fn new(value: DesignSmartSelectionSpacingValue) -> Self {
        Self {
            value,
            availability: DesignSmartSelectionAvailability::Available,
        }
    }

    pub const fn uniform(value: f32) -> Self {
        Self::new(DesignSmartSelectionSpacingValue::Uniform(value))
    }

    pub const fn mixed() -> Self {
        Self::new(DesignSmartSelectionSpacingValue::Mixed)
    }

    pub fn disabled(mut self, reason: impl Into<SharedString>) -> Self {
        self.availability = DesignSmartSelectionAvailability::disabled(reason);
        self
    }

    pub fn is_valid(&self) -> bool {
        self.value.is_valid() && self.availability.is_valid()
    }
}

/// Arrange command whose eligibility is resolved against one exact Smart
/// Selection snapshot.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignSmartSelectionOperation {
    DistributeHorizontal,
    DistributeVertical,
    TidyUp,
}

impl DesignSmartSelectionOperation {
    pub const ALL: [Self; 3] = [
        Self::DistributeHorizontal,
        Self::DistributeVertical,
        Self::TidyUp,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::DistributeHorizontal => "Distribute horizontal spacing",
            Self::DistributeVertical => "Distribute vertical spacing",
            Self::TidyUp => "Tidy up",
        }
    }
}

/// Exact ordered multiple-selection projection used by Smart Selection.
///
/// The target order is part of the snapshot identity. Optional command
/// availability controls whether a native arrange/tidy control is present;
/// a present disabled value remains visible and explains why it cannot run.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignSmartSelectionViewData {
    pub target: DesignPanelTarget,
    pub kind: DesignSmartSelectionKind,
    pub horizontal_spacing: Option<DesignSmartSelectionSpacingViewData>,
    pub vertical_spacing: Option<DesignSmartSelectionSpacingViewData>,
    pub distribute_horizontal: Option<DesignSmartSelectionAvailability>,
    pub distribute_vertical: Option<DesignSmartSelectionAvailability>,
    pub tidy_up: Option<DesignSmartSelectionAvailability>,
    pub read_only_reason: Option<SharedString>,
}

impl DesignSmartSelectionViewData {
    pub fn new(target: DesignPanelTarget, kind: DesignSmartSelectionKind) -> Self {
        Self {
            target,
            kind,
            horizontal_spacing: None,
            vertical_spacing: None,
            distribute_horizontal: None,
            distribute_vertical: None,
            tidy_up: None,
            read_only_reason: None,
        }
    }

    pub fn horizontal(
        target: DesignPanelTarget,
        spacing: DesignSmartSelectionSpacingViewData,
    ) -> Self {
        let mut view_data = Self::new(target, DesignSmartSelectionKind::Horizontal);
        view_data.horizontal_spacing = Some(spacing);
        view_data
    }

    pub fn vertical(
        target: DesignPanelTarget,
        spacing: DesignSmartSelectionSpacingViewData,
    ) -> Self {
        let mut view_data = Self::new(target, DesignSmartSelectionKind::Vertical);
        view_data.vertical_spacing = Some(spacing);
        view_data
    }

    pub fn two_dimensional(
        target: DesignPanelTarget,
        horizontal_spacing: DesignSmartSelectionSpacingViewData,
        vertical_spacing: DesignSmartSelectionSpacingViewData,
    ) -> Self {
        let mut view_data = Self::new(target, DesignSmartSelectionKind::TwoDimensional);
        view_data.horizontal_spacing = Some(horizontal_spacing);
        view_data.vertical_spacing = Some(vertical_spacing);
        view_data
    }

    pub fn with_operation(
        mut self,
        operation: DesignSmartSelectionOperation,
        availability: DesignSmartSelectionAvailability,
    ) -> Self {
        *self.operation_availability_mut(operation) = Some(availability);
        self
    }

    pub fn read_only(mut self, reason: impl Into<SharedString>) -> Self {
        self.read_only_reason = Some(reason.into());
        self
    }

    pub const fn spacing(
        &self,
        axis: DesignSmartSelectionAxis,
    ) -> Option<&DesignSmartSelectionSpacingViewData> {
        match axis {
            DesignSmartSelectionAxis::Horizontal => self.horizontal_spacing.as_ref(),
            DesignSmartSelectionAxis::Vertical => self.vertical_spacing.as_ref(),
        }
    }

    pub const fn operation_availability(
        &self,
        operation: DesignSmartSelectionOperation,
    ) -> Option<&DesignSmartSelectionAvailability> {
        match operation {
            DesignSmartSelectionOperation::DistributeHorizontal => {
                self.distribute_horizontal.as_ref()
            }
            DesignSmartSelectionOperation::DistributeVertical => self.distribute_vertical.as_ref(),
            DesignSmartSelectionOperation::TidyUp => self.tidy_up.as_ref(),
        }
    }

    fn operation_availability_mut(
        &mut self,
        operation: DesignSmartSelectionOperation,
    ) -> &mut Option<DesignSmartSelectionAvailability> {
        match operation {
            DesignSmartSelectionOperation::DistributeHorizontal => &mut self.distribute_horizontal,
            DesignSmartSelectionOperation::DistributeVertical => &mut self.distribute_vertical,
            DesignSmartSelectionOperation::TidyUp => &mut self.tidy_up,
        }
    }

    pub fn spacing_is_editable(&self, axis: DesignSmartSelectionAxis) -> bool {
        self.read_only_reason.is_none()
            && self
                .spacing(axis)
                .is_some_and(|spacing| spacing.availability.is_available())
    }

    pub fn operation_is_available(&self, operation: DesignSmartSelectionOperation) -> bool {
        self.read_only_reason.is_none()
            && self
                .operation_availability(operation)
                .is_some_and(DesignSmartSelectionAvailability::is_available)
    }

    pub fn is_valid(&self) -> bool {
        let DesignPanelTarget::Nodes { node_ids } = &self.target else {
            return false;
        };
        let mut unique_ids = HashSet::with_capacity(node_ids.len());
        if node_ids.len() < 2
            || node_ids
                .iter()
                .any(|node_id| node_id.trim().is_empty() || !unique_ids.insert(node_id))
            || self
                .read_only_reason
                .as_ref()
                .is_some_and(|reason| reason.trim().is_empty())
        {
            return false;
        }

        for axis in DesignSmartSelectionAxis::ALL {
            let spacing = self.spacing(axis);
            if self.kind.supports_axis(axis) != spacing.is_some()
                || spacing.is_some_and(|spacing| !spacing.is_valid())
            {
                return false;
            }
        }
        DesignSmartSelectionOperation::ALL
            .into_iter()
            .filter_map(|operation| self.operation_availability(operation))
            .all(DesignSmartSelectionAvailability::is_valid)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignArrangeOperation {
    AlignLeft,
    AlignHorizontalCenter,
    AlignRight,
    AlignTop,
    AlignVerticalCenter,
    AlignBottom,
    DistributeHorizontal,
    DistributeVertical,
    TidyUp,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignTransformOperation {
    RotateClockwise90,
    FlipHorizontal,
    FlipVertical,
}
