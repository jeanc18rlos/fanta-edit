use std::collections::HashSet;

use gpui::SharedString;

use super::*;

/// Host-declared availability of Figma's corner controls.
///
/// Shape topology matters more than a coarse node kind: a closed vector or a
/// Boolean operation containing rectangles can expose radius controls while
/// an open vector cannot. Hosts can therefore override this preset without
/// changing the node classification.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DesignCornerCapabilities {
    pub uniform_radius: bool,
    pub independent_radii: bool,
    pub smoothing: bool,
}

impl DesignCornerCapabilities {
    pub const NONE: Self = Self {
        uniform_radius: false,
        independent_radii: false,
        smoothing: false,
    };

    pub const fn for_node_kind(kind: DesignPanelNodeKind) -> Self {
        let uniform_radius = kind.supports_corner_radius();
        Self {
            uniform_radius,
            independent_radii: matches!(
                kind,
                DesignPanelNodeKind::Frame
                    | DesignPanelNodeKind::Section
                    | DesignPanelNodeKind::Component
                    | DesignPanelNodeKind::ComponentSet
                    | DesignPanelNodeKind::Instance
                    | DesignPanelNodeKind::Slot
                    | DesignPanelNodeKind::Rectangle
                    | DesignPanelNodeKind::Image
                    | DesignPanelNodeKind::Video
            ),
            smoothing: uniform_radius,
        }
    }

    pub const fn has_any(self) -> bool {
        self.uniform_radius || self.independent_radii || self.smoothing
    }
}

/// Canonical Figma `ArcData`.
///
/// Angles are stored in radians and `inner_radius` is a unit interval. The
/// Design panel converts angles to degrees and the radius to a percentage for
/// display and editing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DesignArcData {
    pub starting_angle: f32,
    pub ending_angle: f32,
    pub inner_radius: f32,
}

impl DesignArcData {
    pub const FULL_CIRCLE: Self = Self {
        starting_angle: 0.,
        ending_angle: std::f32::consts::TAU,
        inner_radius: 0.,
    };

    pub fn new(starting_angle: f32, ending_angle: f32, inner_radius: f32) -> Self {
        Self {
            starting_angle,
            ending_angle,
            inner_radius: inner_radius.clamp(0., 1.),
        }
    }

    pub fn starting_degrees(self) -> f32 {
        self.starting_angle.to_degrees()
    }

    pub fn sweep_angle(self) -> f32 {
        self.ending_angle - self.starting_angle
    }

    pub fn sweep_degrees(self) -> f32 {
        self.sweep_angle().to_degrees()
    }

    pub fn ending_degrees(self) -> f32 {
        self.ending_angle.to_degrees()
    }

    /// Whether Figma's current Design panel exposes the Arc controls.
    ///
    /// A plain full ellipse has no Arc section in the current UI. The controls
    /// appear after the ellipse becomes a partial arc or a ring.
    pub fn has_inspector_controls(self) -> bool {
        let sweep = self.sweep_angle().abs();
        self.inner_radius > f32::EPSILON || (sweep - std::f32::consts::TAU).abs() > f32::EPSILON
    }
}

impl Default for DesignArcData {
    fn default() -> Self {
        Self::FULL_CIRCLE
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignBooleanOperation {
    Union,
    Subtract,
    Intersect,
    Exclude,
}

impl DesignBooleanOperation {
    pub const ALL: [Self; 4] = [Self::Union, Self::Subtract, Self::Intersect, Self::Exclude];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Union => "Union selection",
            Self::Subtract => "Subtract selection",
            Self::Intersect => "Intersect selection",
            Self::Exclude => "Exclude selection",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DesignPolygonGeometry {
    pub point_count: u16,
}

impl DesignPolygonGeometry {
    pub const fn new(point_count: u16) -> Self {
        Self {
            point_count: if point_count < 3 {
                3
            } else if point_count > 60 {
                60
            } else {
                point_count
            },
        }
    }
}

impl Default for DesignPolygonGeometry {
    fn default() -> Self {
        Self::new(3)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DesignStarGeometry {
    pub point_count: u16,
    /// Figma `innerRadius`, normalized to `0..=1`.
    pub inner_radius: f32,
}

impl DesignStarGeometry {
    pub fn new(point_count: u16, inner_radius: f32) -> Self {
        Self {
            point_count: point_count.clamp(3, 60),
            inner_radius: inner_radius.clamp(0., 1.),
        }
    }
}

impl Default for DesignStarGeometry {
    fn default() -> Self {
        Self::new(5, 0.38)
    }
}

/// Read-only counts shown when a host inspects a FigJam table through this
/// reusable surface. Figma Design does not provide table-structure mutations
/// in its Design properties panel.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DesignTableGeometry {
    pub row_count: u16,
    pub column_count: u16,
}

impl DesignTableGeometry {
    pub const fn new(row_count: u16, column_count: u16) -> Self {
        Self {
            row_count,
            column_count,
        }
    }
}

impl Default for DesignTableGeometry {
    fn default() -> Self {
        Self::new(4, 3)
    }
}

/// One mutually-exclusive native shape payload.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum DesignShapeGeometry {
    #[default]
    None,
    Polygon(DesignPolygonGeometry),
    Star(DesignStarGeometry),
    Ellipse(DesignArcData),
    Boolean(DesignBooleanOperation),
    Table(DesignTableGeometry),
}

impl DesignShapeGeometry {
    pub const fn for_node_kind(kind: DesignPanelNodeKind) -> Self {
        match kind {
            DesignPanelNodeKind::Polygon => Self::Polygon(DesignPolygonGeometry::new(3)),
            DesignPanelNodeKind::Star => Self::Star(DesignStarGeometry {
                point_count: 5,
                inner_radius: 0.38,
            }),
            DesignPanelNodeKind::Ellipse => Self::Ellipse(DesignArcData::FULL_CIRCLE),
            DesignPanelNodeKind::BooleanOperation => Self::Boolean(DesignBooleanOperation::Union),
            DesignPanelNodeKind::Table => Self::Table(DesignTableGeometry::new(4, 3)),
            _ => Self::None,
        }
    }

    pub fn has_controls(self) -> bool {
        match self {
            Self::None => false,
            Self::Ellipse(arc) => arc.has_inspector_controls(),
            Self::Polygon(_) | Self::Star(_) | Self::Boolean(_) | Self::Table(_) => true,
        }
    }

    /// Whether this payload contributes native rows to Figma Design's
    /// Appearance section.
    ///
    /// Boolean operations live in the selected-node header and Tables are a
    /// FigJam compatibility projection, so neither is canonical Appearance
    /// content.
    pub fn has_appearance_controls(self) -> bool {
        match self {
            Self::Polygon(_) | Self::Star(_) => true,
            Self::Ellipse(arc) => arc.has_inspector_controls(),
            Self::None | Self::Boolean(_) | Self::Table(_) => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignRepeatAxis {
    Horizontal,
    Vertical,
}

impl DesignRepeatAxis {
    pub const ALL: [Self; 2] = [Self::Horizontal, Self::Vertical];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Horizontal => "Horizontal",
            Self::Vertical => "Vertical",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignRepeatType {
    Linear,
    Radial,
}

impl DesignRepeatType {
    pub const ALL: [Self; 2] = [Self::Linear, Self::Radial];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Linear => "Linear repeat",
            Self::Radial => "Radial repeat",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignRepeatMode {
    Linear(DesignRepeatAxis),
    Radial,
}

impl DesignRepeatMode {
    pub const fn repeat_type(self) -> DesignRepeatType {
        match self {
            Self::Linear(_) => DesignRepeatType::Linear,
            Self::Radial => DesignRepeatType::Radial,
        }
    }

    pub const fn axis(self) -> Option<DesignRepeatAxis> {
        match self {
            Self::Linear(axis) => Some(axis),
            Self::Radial => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignTransformUnit {
    Relative,
    Pixels,
}

impl DesignTransformUnit {
    pub const ALL: [Self; 2] = [Self::Relative, Self::Pixels];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Relative => "Relative",
            Self::Pixels => "Pixels",
        }
    }

    pub const fn suffix(self) -> &'static str {
        match self {
            Self::Relative => "%",
            Self::Pixels => " px",
        }
    }
}

/// Figma's currently-supported `REPEAT` transform modifier.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignRepeatModifier {
    /// Stable host identity retained while the modifier array is reordered.
    pub id: SharedString,
    pub mode: DesignRepeatMode,
    pub count: u32,
    pub unit: DesignTransformUnit,
    pub offset: f32,
}

impl DesignRepeatModifier {
    pub fn linear(id: impl Into<SharedString>, axis: DesignRepeatAxis) -> Self {
        Self {
            id: id.into(),
            mode: DesignRepeatMode::Linear(axis),
            count: 5,
            unit: DesignTransformUnit::Relative,
            offset: 100.,
        }
    }

    pub fn radial(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            mode: DesignRepeatMode::Radial,
            count: 8,
            unit: DesignTransformUnit::Pixels,
            offset: 96.,
        }
    }
}

/// One host-applied mutation to a repeat modifier.
///
/// Mode changes are fully discriminated so switching a radial modifier back
/// to linear also carries the axis Figma requires.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DesignTransformModifierChange {
    Mode(DesignRepeatMode),
    Count(u32),
    Unit(DesignTransformUnit),
    Offset(f32),
}

/// Exact writable values of Figma's `HandleMirroring` Plugin API type.
///
/// Mixed selections are represented by [`DesignVectorSelectionValue::Mixed`],
/// not by an invented writable value.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignHandleMirroring {
    None,
    Angle,
    AngleAndLength,
}

impl DesignHandleMirroring {
    pub const ALL: [Self; 3] = [Self::None, Self::Angle, Self::AngleAndLength];

    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Angle => "Mirror angle",
            Self::AngleAndLength => "Mirror angle & length",
        }
    }

    pub const fn api_name(self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::Angle => "ANGLE",
            Self::AngleAndLength => "ANGLE_AND_LENGTH",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignVectorCoordinateAxis {
    X,
    Y,
}

/// Host-resolved state for one property across the selected vector vertices.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignVectorSelectionValue<T> {
    Unset,
    Uniform(T),
    Mixed,
}

impl<T> DesignVectorSelectionValue<T> {
    pub const fn is_unset(&self) -> bool {
        matches!(self, Self::Unset)
    }

    pub const fn is_mixed(&self) -> bool {
        matches!(self, Self::Mixed)
    }

    pub const fn uniform(&self) -> Option<&T> {
        match self {
            Self::Uniform(value) => Some(value),
            Self::Unset | Self::Mixed => None,
        }
    }
}

impl<T: PartialEq> DesignVectorSelectionValue<T> {
    fn from_values(values: impl IntoIterator<Item = T>) -> Self {
        let mut values = values.into_iter();
        let Some(first) = values.next() else {
            return Self::Unset;
        };
        if values.all(|value| value == first) {
            Self::Uniform(first)
        } else {
            Self::Mixed
        }
    }
}

/// Topology of one stable host vertex in a vector network.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignVectorVertexTopology {
    Endpoint,
    Interior,
    Branch,
}

impl DesignVectorVertexTopology {
    /// Branch vertices can join more than two segments, so Figma's single
    /// tangent-mirroring and per-vertex rounding controls are ambiguous there.
    pub const fn supports_single_path_controls(self) -> bool {
        !matches!(self, Self::Branch)
    }
}

/// Immutable coordinate supplied by the host for one vector vertex.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DesignVectorPoint {
    pub x: f32,
    pub y: f32,
}

impl DesignVectorPoint {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn is_valid(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

/// One stable, host-owned vertex exposed while editing a Vector or TextPath.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignVectorVertexViewData {
    /// Opaque identity authored by the host; never inferred from list order.
    pub id: SharedString,
    pub position: DesignVectorPoint,
    /// `None` means this vertex does not expose per-vertex rounding.
    pub corner_radius: Option<f32>,
    /// `None` means this vertex has no editable tangent mirroring control.
    pub handle_mirroring: Option<DesignHandleMirroring>,
    pub topology: DesignVectorVertexTopology,
    pub selected: bool,
    pub read_only: bool,
}

impl DesignVectorVertexViewData {
    pub fn new(id: impl Into<SharedString>, x: f32, y: f32) -> Self {
        Self {
            id: id.into(),
            position: DesignVectorPoint::new(x, y),
            corner_radius: Some(0.),
            handle_mirroring: Some(DesignHandleMirroring::None),
            topology: DesignVectorVertexTopology::Interior,
            selected: false,
            read_only: false,
        }
    }

    pub const fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub const fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    pub const fn with_topology(mut self, topology: DesignVectorVertexTopology) -> Self {
        self.topology = topology;
        self
    }

    pub const fn with_corner_radius(mut self, corner_radius: Option<f32>) -> Self {
        self.corner_radius = corner_radius;
        self
    }

    pub const fn with_handle_mirroring(
        mut self,
        handle_mirroring: Option<DesignHandleMirroring>,
    ) -> Self {
        self.handle_mirroring = handle_mirroring;
        self
    }

    pub fn is_valid(&self) -> bool {
        !self.id.is_empty()
            && self.position.is_valid()
            && self
                .corner_radius
                .is_none_or(|radius| radius.is_finite() && radius >= 0.)
    }

    pub const fn supports_corner_radius(&self) -> bool {
        self.topology.supports_single_path_controls() && self.corner_radius.is_some()
    }

    pub const fn supports_handle_mirroring(&self) -> bool {
        self.topology.supports_single_path_controls() && self.handle_mirroring.is_some()
    }
}

/// Host-controlled sub-selection presented while a Vector or TextPath is in
/// vector-edit mode.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DesignVectorEditViewData {
    pub vertices: Vec<DesignVectorVertexViewData>,
    pub read_only: bool,
    pub disabled_reason: Option<SharedString>,
}

impl DesignVectorEditViewData {
    pub fn new(vertices: impl IntoIterator<Item = DesignVectorVertexViewData>) -> Self {
        Self {
            vertices: vertices.into_iter().collect(),
            read_only: false,
            disabled_reason: None,
        }
    }

    pub const fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    pub fn with_disabled_reason(mut self, reason: impl Into<SharedString>) -> Self {
        self.disabled_reason = Some(reason.into());
        self
    }

    pub fn is_valid(&self) -> bool {
        let mut ids = HashSet::with_capacity(self.vertices.len());
        self.vertices
            .iter()
            .all(|vertex| vertex.is_valid() && ids.insert(vertex.id.clone()))
    }

    pub fn vertex(&self, vertex_id: &str) -> Option<&DesignVectorVertexViewData> {
        (!vertex_id.is_empty()).then_some(())?;
        self.vertices
            .iter()
            .find(|vertex| vertex.id.as_ref() == vertex_id)
    }

    pub fn selected_vertices(&self) -> impl Iterator<Item = &DesignVectorVertexViewData> {
        self.vertices.iter().filter(|vertex| vertex.selected)
    }

    pub fn selected_vertex_ids(&self) -> Vec<SharedString> {
        self.selected_vertices()
            .map(|vertex| vertex.id.clone())
            .collect()
    }

    pub fn has_selection(&self) -> bool {
        self.selected_vertices().next().is_some()
    }

    pub fn selected_x(&self) -> DesignVectorSelectionValue<f32> {
        DesignVectorSelectionValue::from_values(
            self.selected_vertices().map(|vertex| vertex.position.x),
        )
    }

    pub fn selected_y(&self) -> DesignVectorSelectionValue<f32> {
        DesignVectorSelectionValue::from_values(
            self.selected_vertices().map(|vertex| vertex.position.y),
        )
    }

    pub fn selected_corner_radius(&self) -> DesignVectorSelectionValue<f32> {
        if !self
            .selected_vertices()
            .all(DesignVectorVertexViewData::supports_corner_radius)
        {
            return DesignVectorSelectionValue::Unset;
        }
        DesignVectorSelectionValue::from_values(
            self.selected_vertices()
                .filter_map(|vertex| vertex.corner_radius),
        )
    }

    pub fn selected_handle_mirroring(&self) -> DesignVectorSelectionValue<DesignHandleMirroring> {
        if !self
            .selected_vertices()
            .all(DesignVectorVertexViewData::supports_handle_mirroring)
        {
            return DesignVectorSelectionValue::Unset;
        }
        DesignVectorSelectionValue::from_values(
            self.selected_vertices()
                .filter_map(|vertex| vertex.handle_mirroring),
        )
    }

    pub fn can_select_vertices(&self) -> bool {
        self.is_valid() && !self.read_only
    }

    pub fn can_edit_coordinates(&self) -> bool {
        self.can_edit_selected(|_| true)
    }

    pub fn can_edit_corner_radius(&self) -> bool {
        self.can_edit_selected(DesignVectorVertexViewData::supports_corner_radius)
    }

    pub fn can_edit_handle_mirroring(&self) -> bool {
        self.can_edit_selected(DesignVectorVertexViewData::supports_handle_mirroring)
    }

    pub fn contains_exact_vertices(&self, vertex_ids: &[SharedString]) -> bool {
        !vertex_ids.is_empty()
            && vertex_ids.iter().all(|id| self.vertex(id).is_some())
            && vertex_ids.iter().collect::<HashSet<_>>().len() == vertex_ids.len()
    }

    fn can_edit_selected(&self, supports: impl Fn(&DesignVectorVertexViewData) -> bool) -> bool {
        self.is_valid()
            && !self.read_only
            && self.has_selection()
            && self
                .selected_vertices()
                .all(|vertex| !vertex.read_only && supports(vertex))
    }
}
