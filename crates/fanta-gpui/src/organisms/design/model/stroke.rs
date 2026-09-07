use gpui::SharedString;

use super::*;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignStrokeAlign {
    Inside,
    Center,
    Outside,
}

impl DesignStrokeAlign {
    pub const ALL: [Self; 3] = [Self::Inside, Self::Center, Self::Outside];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Inside => "Inside",
            Self::Center => "Center",
            Self::Outside => "Outside",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignStrokeCap {
    None,
    Round,
    Square,
    LineArrow,
    TriangleArrow,
    ReverseTriangle,
    Diamond,
    Circle,
}

impl DesignStrokeCap {
    /// The complete Figma `StrokeCap` set.
    pub const ALL: [Self; 8] = [
        Self::None,
        Self::Round,
        Self::Square,
        Self::LineArrow,
        Self::TriangleArrow,
        Self::ReverseTriangle,
        Self::Diamond,
        Self::Circle,
    ];

    /// Compatibility alias for the former abbreviated line-arrow variant.
    #[allow(non_upper_case_globals)]
    #[deprecated(note = "use DesignStrokeCap::LineArrow")]
    pub const Arrow: Self = Self::LineArrow;

    /// Compatibility alias for the former abbreviated outward triangle variant.
    #[allow(non_upper_case_globals)]
    #[deprecated(note = "use DesignStrokeCap::TriangleArrow")]
    pub const Triangle: Self = Self::TriangleArrow;

    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Round => "Round",
            Self::Square => "Square",
            Self::LineArrow => "Line arrow",
            Self::TriangleArrow => "Triangle arrow",
            Self::ReverseTriangle => "Reverse triangle",
            Self::Diamond => "Diamond arrow",
            Self::Circle => "Circle",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignStrokeJoin {
    Miter,
    Bevel,
    Round,
}

impl DesignStrokeJoin {
    pub const ALL: [Self; 3] = [Self::Miter, Self::Bevel, Self::Round];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Miter => "Miter",
            Self::Bevel => "Bevel",
            Self::Round => "Rounded",
        }
    }
}

/// Which stroke-side editor Figma exposes while retaining all four weights.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignStrokeWeightMode {
    All,
    Top,
    Bottom,
    Left,
    Right,
    Custom,
}

impl DesignStrokeWeightMode {
    pub const ALL: [Self; 6] = [
        Self::All,
        Self::Top,
        Self::Bottom,
        Self::Left,
        Self::Right,
        Self::Custom,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Top => "Top",
            Self::Bottom => "Bottom",
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Custom => "Custom",
        }
    }
}

/// Canonical top/right/bottom/left stroke weights.
///
/// The editor mode is presentation metadata; the four side values remain the
/// source of truth, including while the uniform `All` editor is active.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DesignStrokeWeights {
    pub mode: DesignStrokeWeightMode,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl DesignStrokeWeights {
    pub const fn uniform(weight: f32) -> Self {
        Self {
            mode: DesignStrokeWeightMode::All,
            top: weight,
            right: weight,
            bottom: weight,
            left: weight,
        }
    }

    pub const fn custom(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Self {
            mode: DesignStrokeWeightMode::Custom,
            top,
            right,
            bottom,
            left,
        }
    }

    pub const fn active(self) -> f32 {
        match self.mode {
            DesignStrokeWeightMode::All
            | DesignStrokeWeightMode::Top
            | DesignStrokeWeightMode::Custom => self.top,
            DesignStrokeWeightMode::Bottom => self.bottom,
            DesignStrokeWeightMode::Left => self.left,
            DesignStrokeWeightMode::Right => self.right,
        }
    }

    pub fn set_uniform(&mut self, weight: f32) {
        self.top = weight;
        self.right = weight;
        self.bottom = weight;
        self.left = weight;
    }

    pub fn set_mode(&mut self, mode: DesignStrokeWeightMode) {
        let active = self.active();
        self.mode = mode;
        if mode == DesignStrokeWeightMode::All {
            self.set_uniform(active);
        }
    }

    pub fn set_active(&mut self, weight: f32) {
        match self.mode {
            DesignStrokeWeightMode::All => self.set_uniform(weight),
            DesignStrokeWeightMode::Top => self.top = weight,
            DesignStrokeWeightMode::Bottom => self.bottom = weight,
            DesignStrokeWeightMode::Left => self.left = weight,
            DesignStrokeWeightMode::Right => self.right = weight,
            DesignStrokeWeightMode::Custom => self.top = weight,
        }
    }

    pub const fn is_uniform(self) -> bool {
        self.top == self.right && self.top == self.bottom && self.top == self.left
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignStrokeDashMode {
    Solid,
    Dashed,
    Custom,
}

impl DesignStrokeDashMode {
    pub const ALL: [Self; 3] = [Self::Solid, Self::Dashed, Self::Custom];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Solid => "Solid",
            Self::Dashed => "Dashed",
            Self::Custom => "Custom",
        }
    }

    pub const fn next(self) -> Self {
        match self {
            Self::Solid => Self::Dashed,
            Self::Dashed => Self::Custom,
            Self::Custom => Self::Solid,
        }
    }
}

/// Canonical active dash data plus the explicit Design-panel presentation mode.
///
/// Figma persists only an ordered `strokeDashes` array. The explicit mode keeps
/// an empty array selectable as Solid while still exposing a transition into
/// the two-value Dashed editor or the arbitrary ordered Custom editor.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignStrokeDashes {
    pub mode: DesignStrokeDashMode,
    pub pattern: Vec<f32>,
}

impl DesignStrokeDashes {
    pub const fn solid() -> Self {
        Self {
            mode: DesignStrokeDashMode::Solid,
            pattern: Vec::new(),
        }
    }

    pub fn dashed(dash: f32, gap: f32) -> Self {
        Self {
            mode: DesignStrokeDashMode::Dashed,
            pattern: vec![dash, gap],
        }
    }

    pub fn custom(pattern: impl IntoIterator<Item = f32>) -> Self {
        Self {
            mode: DesignStrokeDashMode::Custom,
            pattern: pattern.into_iter().collect(),
        }
    }

    pub fn is_valid(&self) -> bool {
        self.pattern
            .iter()
            .all(|value| value.is_finite() && *value >= 0.)
            && match self.mode {
                DesignStrokeDashMode::Solid => self.pattern.is_empty(),
                DesignStrokeDashMode::Dashed => self.pattern.len() == 2,
                DesignStrokeDashMode::Custom => !self.pattern.is_empty(),
            }
    }

    pub fn transition(&mut self, mode: DesignStrokeDashMode) {
        self.mode = mode;
        match mode {
            DesignStrokeDashMode::Solid => self.pattern.clear(),
            DesignStrokeDashMode::Dashed => {
                self.pattern = match self.pattern.as_slice() {
                    [dash, gap, ..]
                        if dash.is_finite() && *dash >= 0. && gap.is_finite() && *gap >= 0. =>
                    {
                        vec![*dash, *gap]
                    }
                    _ => vec![4., 4.],
                };
            }
            DesignStrokeDashMode::Custom => {
                if self.pattern.is_empty()
                    || !self
                        .pattern
                        .iter()
                        .all(|value| value.is_finite() && *value >= 0.)
                {
                    self.pattern = vec![4., 4., 1., 4.];
                }
            }
        }
    }

    pub fn set_pattern(&mut self, pattern: Vec<f32>) -> bool {
        let candidate = Self {
            mode: self.mode,
            pattern,
        };
        if !candidate.is_valid() {
            return false;
        }
        *self = candidate;
        true
    }

    pub fn is_solid(&self) -> bool {
        self.mode == DesignStrokeDashMode::Solid
    }
}

impl Default for DesignStrokeDashes {
    fn default() -> Self {
        Self::solid()
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignVariableWidthPreset {
    Uniform,
    Wedge,
    Taper,
    QuarterTaper,
    Eye,
    MirroredTaper,
}

impl DesignVariableWidthPreset {
    pub const ALL: [Self; 6] = [
        Self::Uniform,
        Self::Wedge,
        Self::Taper,
        Self::QuarterTaper,
        Self::Eye,
        Self::MirroredTaper,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Uniform => "Uniform",
            Self::Wedge => "Wedge",
            Self::Taper => "Taper",
            Self::QuarterTaper => "Quarter taper",
            Self::Eye => "Eye",
            Self::MirroredTaper => "Mirrored taper",
        }
    }

    pub const fn api_name(self) -> &'static str {
        match self {
            Self::Uniform => "UNIFORM",
            Self::Wedge => "WEDGE",
            Self::Taper => "TAPER",
            Self::QuarterTaper => "QUARTER_TAPER",
            Self::Eye => "EYE",
            Self::MirroredTaper => "MIRRORED_TAPER",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DesignVariableWidthPoint {
    pub position: f32,
    pub width: f32,
}

impl DesignVariableWidthPoint {
    pub const fn new(position: f32, width: f32) -> Self {
        Self { position, width }
    }

    pub fn is_valid(self) -> bool {
        self.position.is_finite()
            && (0. ..=1.).contains(&self.position)
            && self.width.is_finite()
            && self.width >= 0.
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum DesignVariableWidthStroke {
    Preset(DesignVariableWidthPreset),
    Custom {
        /// Host order is canonical and is never sorted by the panel.
        points: Vec<DesignVariableWidthPoint>,
    },
}

impl DesignVariableWidthStroke {
    pub fn custom(points: impl IntoIterator<Item = DesignVariableWidthPoint>) -> Self {
        Self::Custom {
            points: points.into_iter().collect(),
        }
    }

    pub const fn label(&self) -> &'static str {
        match self {
            Self::Preset(preset) => preset.label(),
            Self::Custom { .. } => "Custom",
        }
    }

    pub fn points(&self) -> Option<&[DesignVariableWidthPoint]> {
        match self {
            Self::Preset(_) => None,
            Self::Custom { points } => Some(points),
        }
    }

    pub fn is_valid(&self) -> bool {
        match self {
            Self::Preset(_) => true,
            Self::Custom { points } => {
                !points.is_empty() && points.iter().all(|point| point.is_valid())
            }
        }
    }

    pub fn set_point_position(&mut self, index: usize, position: f32) -> bool {
        let Self::Custom { points } = self else {
            return false;
        };
        let Some(point) = points.get_mut(index) else {
            return false;
        };
        let candidate = DesignVariableWidthPoint { position, ..*point };
        if !candidate.is_valid() {
            return false;
        }
        *point = candidate;
        true
    }

    pub fn set_point_width(&mut self, index: usize, width: f32) -> bool {
        let Self::Custom { points } = self else {
            return false;
        };
        let Some(point) = points.get_mut(index) else {
            return false;
        };
        let candidate = DesignVariableWidthPoint { width, ..*point };
        if !candidate.is_valid() {
            return false;
        }
        *point = candidate;
        true
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignStrokeType {
    Basic,
    StretchBrush,
    ScatterBrush,
    Dynamic,
    Opaque,
}

impl DesignStrokeType {
    pub const EDITABLE: [Self; 4] = [
        Self::Basic,
        Self::StretchBrush,
        Self::ScatterBrush,
        Self::Dynamic,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Basic => "Basic",
            Self::StretchBrush => "Stretch brush",
            Self::ScatterBrush => "Scatter brush",
            Self::Dynamic => "Dynamic",
            Self::Opaque => "Custom",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignStrokeBrushDirection {
    Forward,
    Backward,
}

impl DesignStrokeBrushDirection {
    pub const ALL: [Self; 2] = [Self::Forward, Self::Backward];

    #[allow(non_upper_case_globals)]
    #[deprecated(note = "use DesignStrokeBrushDirection::Backward")]
    pub const Reverse: Self = Self::Backward;

    pub const fn label(self) -> &'static str {
        match self {
            Self::Forward => "Forward",
            Self::Backward => "Backward",
        }
    }

    pub const fn api_name(self) -> &'static str {
        match self {
            Self::Forward => "FORWARD",
            Self::Backward => "BACKWARD",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignStretchBrushName {
    Heist,
    Blockbuster,
    Grindhouse,
    Biopic,
    SpaghettiWestern,
    Slasher,
    Hardboiled,
    Verite,
    Epic,
    Screwball,
    RomCom,
    Noir,
    Propaganda,
    Melodrama,
    NewWave,
}

impl DesignStretchBrushName {
    pub const ALL: [Self; 15] = [
        Self::Heist,
        Self::Blockbuster,
        Self::Grindhouse,
        Self::Biopic,
        Self::SpaghettiWestern,
        Self::Slasher,
        Self::Hardboiled,
        Self::Verite,
        Self::Epic,
        Self::Screwball,
        Self::RomCom,
        Self::Noir,
        Self::Propaganda,
        Self::Melodrama,
        Self::NewWave,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Heist => "Heist",
            Self::Blockbuster => "Blockbuster",
            Self::Grindhouse => "Grindhouse",
            Self::Biopic => "Biopic",
            Self::SpaghettiWestern => "Spaghetti western",
            Self::Slasher => "Slasher",
            Self::Hardboiled => "Hardboiled",
            Self::Verite => "Verite",
            Self::Epic => "Epic",
            Self::Screwball => "Screwball",
            Self::RomCom => "Rom com",
            Self::Noir => "Noir",
            Self::Propaganda => "Propaganda",
            Self::Melodrama => "Melodrama",
            Self::NewWave => "New wave",
        }
    }

    pub const fn api_name(self) -> &'static str {
        match self {
            Self::Heist => "HEIST",
            Self::Blockbuster => "BLOCKBUSTER",
            Self::Grindhouse => "GRINDHOUSE",
            Self::Biopic => "BIOPIC",
            Self::SpaghettiWestern => "SPAGHETTI_WESTERN",
            Self::Slasher => "SLASHER",
            Self::Hardboiled => "HARDBOILED",
            Self::Verite => "VERITE",
            Self::Epic => "EPIC",
            Self::Screwball => "SCREWBALL",
            Self::RomCom => "ROM_COM",
            Self::Noir => "NOIR",
            Self::Propaganda => "PROPAGANDA",
            Self::Melodrama => "MELODRAMA",
            Self::NewWave => "NEW_WAVE",
        }
    }

    pub const fn next(self) -> Self {
        match self {
            Self::Heist => Self::Blockbuster,
            Self::Blockbuster => Self::Grindhouse,
            Self::Grindhouse => Self::Biopic,
            Self::Biopic => Self::SpaghettiWestern,
            Self::SpaghettiWestern => Self::Slasher,
            Self::Slasher => Self::Hardboiled,
            Self::Hardboiled => Self::Verite,
            Self::Verite => Self::Epic,
            Self::Epic => Self::Screwball,
            Self::Screwball => Self::RomCom,
            Self::RomCom => Self::Noir,
            Self::Noir => Self::Propaganda,
            Self::Propaganda => Self::Melodrama,
            Self::Melodrama => Self::NewWave,
            Self::NewWave => Self::Heist,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignScatterBrushName {
    Bubblegum,
    WitchHouse,
    Shoegaze,
    HonkyTonk,
    Screamo,
    Drone,
    DooWop,
    SpokenWord,
    Vaporwave,
    Oi,
}

impl DesignScatterBrushName {
    pub const ALL: [Self; 10] = [
        Self::Bubblegum,
        Self::WitchHouse,
        Self::Shoegaze,
        Self::HonkyTonk,
        Self::Screamo,
        Self::Drone,
        Self::DooWop,
        Self::SpokenWord,
        Self::Vaporwave,
        Self::Oi,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Bubblegum => "Bubblegum",
            Self::WitchHouse => "Witch house",
            Self::Shoegaze => "Shoegaze",
            Self::HonkyTonk => "Honky tonk",
            Self::Screamo => "Screamo",
            Self::Drone => "Drone",
            Self::DooWop => "Doo wop",
            Self::SpokenWord => "Spoken word",
            Self::Vaporwave => "Vaporwave",
            Self::Oi => "Oi",
        }
    }

    pub const fn api_name(self) -> &'static str {
        match self {
            Self::Bubblegum => "BUBBLEGUM",
            Self::WitchHouse => "WITCH_HOUSE",
            Self::Shoegaze => "SHOEGAZE",
            Self::HonkyTonk => "HONKY_TONK",
            Self::Screamo => "SCREAMO",
            Self::Drone => "DRONE",
            Self::DooWop => "DOO_WOP",
            Self::SpokenWord => "SPOKEN_WORD",
            Self::Vaporwave => "VAPORWAVE",
            Self::Oi => "OI",
        }
    }

    pub const fn next(self) -> Self {
        match self {
            Self::Bubblegum => Self::WitchHouse,
            Self::WitchHouse => Self::Shoegaze,
            Self::Shoegaze => Self::HonkyTonk,
            Self::HonkyTonk => Self::Screamo,
            Self::Screamo => Self::Drone,
            Self::Drone => Self::DooWop,
            Self::DooWop => Self::SpokenWord,
            Self::SpokenWord => Self::Vaporwave,
            Self::Vaporwave => Self::Oi,
            Self::Oi => Self::Bubblegum,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DesignStretchBrushStroke {
    pub brush: DesignStretchBrushName,
    pub direction: DesignStrokeBrushDirection,
}

impl Default for DesignStretchBrushStroke {
    fn default() -> Self {
        Self {
            brush: DesignStretchBrushName::Heist,
            direction: DesignStrokeBrushDirection::Forward,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DesignScatterBrushStroke {
    pub brush: DesignScatterBrushName,
    pub gap: f32,
    pub wiggle: f32,
    pub size_jitter: f32,
    pub angular_jitter: f32,
    pub rotation: f32,
}

impl DesignScatterBrushStroke {
    pub fn is_valid(self) -> bool {
        self.gap.is_finite()
            && self.gap >= 0.25
            && self.wiggle.is_finite()
            && self.wiggle >= 0.
            && self.size_jitter.is_finite()
            && (0. ..=3.).contains(&self.size_jitter)
            && self.angular_jitter.is_finite()
            && (-180. ..=180.).contains(&self.angular_jitter)
            && self.rotation.is_finite()
            && (-180. ..=180.).contains(&self.rotation)
    }
}

impl Default for DesignScatterBrushStroke {
    fn default() -> Self {
        Self {
            brush: DesignScatterBrushName::Bubblegum,
            gap: 0.25,
            wiggle: 0.,
            size_jitter: 0.,
            angular_jitter: 0.,
            rotation: 0.,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DesignDynamicStroke {
    pub frequency: f32,
    pub wiggle: f32,
    pub smoothen: f32,
}

impl DesignDynamicStroke {
    pub fn is_valid(self) -> bool {
        self.frequency.is_finite()
            && (0.01..=20.).contains(&self.frequency)
            && self.wiggle.is_finite()
            && self.wiggle >= 0.
            && self.smoothen.is_finite()
            && (0. ..=1.).contains(&self.smoothen)
    }
}

impl Default for DesignDynamicStroke {
    fn default() -> Self {
        Self {
            frequency: 1.,
            wiggle: 0.,
            smoothen: 0.5,
        }
    }
}

/// Lossless host payload for custom brushes and future complex-stroke forms.
///
/// Figma returns `brushName: "CUSTOM"` but does not allow plugins to set that
/// brush. The panel therefore displays this payload without emitting edits.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignOpaqueComplexStroke {
    pub type_name: SharedString,
    pub label: SharedString,
    pub raw: SharedString,
}

impl DesignOpaqueComplexStroke {
    pub fn new(
        type_name: impl Into<SharedString>,
        label: impl Into<SharedString>,
        raw: impl Into<SharedString>,
    ) -> Self {
        Self {
            type_name: type_name.into(),
            label: label.into(),
            raw: raw.into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum DesignComplexStroke {
    #[default]
    Basic,
    StretchBrush(DesignStretchBrushStroke),
    ScatterBrush(DesignScatterBrushStroke),
    Dynamic(DesignDynamicStroke),
    Opaque(DesignOpaqueComplexStroke),
}

impl DesignComplexStroke {
    pub const fn kind(&self) -> DesignStrokeType {
        match self {
            Self::Basic => DesignStrokeType::Basic,
            Self::StretchBrush(_) => DesignStrokeType::StretchBrush,
            Self::ScatterBrush(_) => DesignStrokeType::ScatterBrush,
            Self::Dynamic(_) => DesignStrokeType::Dynamic,
            Self::Opaque(_) => DesignStrokeType::Opaque,
        }
    }

    pub fn label(&self) -> SharedString {
        match self {
            Self::Opaque(opaque) => opaque.label.clone(),
            _ => self.kind().label().into(),
        }
    }

    pub const fn is_basic(&self) -> bool {
        matches!(self, Self::Basic)
    }

    pub const fn is_dynamic(&self) -> bool {
        matches!(self, Self::Dynamic(_))
    }

    pub const fn is_opaque(&self) -> bool {
        matches!(self, Self::Opaque(_))
    }

    pub fn editable_default(kind: DesignStrokeType) -> Option<Self> {
        match kind {
            DesignStrokeType::Basic => Some(Self::Basic),
            DesignStrokeType::StretchBrush => {
                Some(Self::StretchBrush(DesignStretchBrushStroke::default()))
            }
            DesignStrokeType::ScatterBrush => {
                Some(Self::ScatterBrush(DesignScatterBrushStroke::default()))
            }
            DesignStrokeType::Dynamic => Some(Self::Dynamic(DesignDynamicStroke::default())),
            DesignStrokeType::Opaque => None,
        }
    }

    pub fn is_valid(&self) -> bool {
        match self {
            Self::Basic | Self::StretchBrush(_) | Self::Opaque(_) => true,
            Self::ScatterBrush(stroke) => stroke.is_valid(),
            Self::Dynamic(stroke) => stroke.is_valid(),
        }
    }
}

/// Topology supplied by the document host for endpoint-sensitive controls.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignStrokePathTopology {
    Closed,
    Open,
    Branching,
}

/// Vector-edit context needed to place endpoint controls correctly.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DesignStrokeEditContext {
    pub topology: DesignStrokePathTopology,
    pub endpoint_count: u16,
    pub selected_vertices: u16,
    pub selected_endpoint_vertices: u16,
}

impl DesignStrokeEditContext {
    pub const fn closed() -> Self {
        Self {
            topology: DesignStrokePathTopology::Closed,
            endpoint_count: 0,
            selected_vertices: 0,
            selected_endpoint_vertices: 0,
        }
    }

    pub const fn open(endpoint_count: u16) -> Self {
        Self {
            topology: DesignStrokePathTopology::Open,
            endpoint_count,
            selected_vertices: 0,
            selected_endpoint_vertices: 0,
        }
    }

    pub const fn branching(endpoint_count: u16) -> Self {
        Self {
            topology: DesignStrokePathTopology::Branching,
            endpoint_count,
            selected_vertices: 0,
            selected_endpoint_vertices: 0,
        }
    }

    /// Compatibility shorthand for a selection made entirely of endpoints.
    pub const fn with_selected_vertices(mut self, selected_vertices: u16) -> Self {
        self.selected_vertices = selected_vertices;
        self.selected_endpoint_vertices = selected_vertices;
        self
    }

    /// Supplies the exact vector-edit selection and how many selected vertices
    /// are endpoints. Interior-only selections must not expose endpoint caps.
    pub const fn with_vertex_selection(
        mut self,
        selected_vertices: u16,
        selected_endpoint_vertices: u16,
    ) -> Self {
        self.selected_vertices = selected_vertices;
        self.selected_endpoint_vertices = if selected_endpoint_vertices > selected_vertices {
            selected_vertices
        } else {
            selected_endpoint_vertices
        };
        self
    }

    pub const fn endpoint_control(self) -> DesignStrokeEndpointControl {
        if self.endpoint_count == 0 {
            DesignStrokeEndpointControl::None
        } else if self.selected_endpoint_vertices > 0 {
            DesignStrokeEndpointControl::SelectedVertices
        } else if self.selected_vertices > 0 {
            DesignStrokeEndpointControl::None
        } else if matches!(self.topology, DesignStrokePathTopology::Open)
            && self.endpoint_count == 2
        {
            DesignStrokeEndpointControl::StartAndEnd
        } else {
            DesignStrokeEndpointControl::Aggregate
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignStrokeEndpointControl {
    None,
    StartAndEnd,
    Aggregate,
    SelectedVertices,
}

/// Host-declared support matrix for geometry controls.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DesignStrokeCapabilities {
    pub position: bool,
    pub individual_weights: bool,
    pub variable_width: bool,
    pub joins: bool,
    pub complex_stroke: bool,
}

impl DesignStrokeCapabilities {
    pub const fn for_node_kind(kind: DesignPanelNodeKind) -> Self {
        Self {
            // Figma fixes open Line strokes to Center and does not expose the
            // Inside/Center/Outside position control. `Arrow` is the
            // compatibility alias for a Line with an arrow endpoint.
            position: !matches!(kind, DesignPanelNodeKind::Line | DesignPanelNodeKind::Arrow),
            individual_weights: matches!(
                kind,
                DesignPanelNodeKind::Rectangle
                    | DesignPanelNodeKind::Frame
                    | DesignPanelNodeKind::Component
                    | DesignPanelNodeKind::ComponentSet
                    | DesignPanelNodeKind::Instance
                    | DesignPanelNodeKind::Slot
            ),
            variable_width: !matches!(kind, DesignPanelNodeKind::Pencil),
            joins: !matches!(kind, DesignPanelNodeKind::Line),
            complex_stroke: !matches!(
                kind,
                DesignPanelNodeKind::Image | DesignPanelNodeKind::Video
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignStroke {
    pub paints: Vec<DesignPaint>,
    pub weights: DesignStrokeWeights,
    pub align: DesignStrokeAlign,
    pub start_cap: DesignStrokeCap,
    pub end_cap: DesignStrokeCap,
    pub endpoint_cap: DesignStrokeCap,
    pub dashes: DesignStrokeDashes,
    pub dash_cap: DesignStrokeCap,
    pub join: DesignStrokeJoin,
    pub miter_angle: f32,
    pub variable_width: Option<DesignVariableWidthStroke>,
    pub complex_stroke: DesignComplexStroke,
    pub capabilities: DesignStrokeCapabilities,
    pub edit_context: DesignStrokeEditContext,
}

impl DesignStroke {
    pub fn for_node(
        kind: DesignPanelNodeKind,
        paint: DesignPaint,
        weight: f32,
        align: DesignStrokeAlign,
    ) -> Self {
        let capabilities = DesignStrokeCapabilities::for_node_kind(kind);
        let edit_context = if matches!(
            kind,
            DesignPanelNodeKind::Line
                | DesignPanelNodeKind::Arrow
                | DesignPanelNodeKind::Vector
                | DesignPanelNodeKind::Pen
                | DesignPanelNodeKind::Pencil
        ) {
            DesignStrokeEditContext::open(2)
        } else {
            DesignStrokeEditContext::closed()
        };
        Self {
            paints: vec![paint],
            weights: DesignStrokeWeights::uniform(weight),
            align: if capabilities.position {
                align
            } else {
                DesignStrokeAlign::Center
            },
            start_cap: DesignStrokeCap::None,
            end_cap: DesignStrokeCap::None,
            endpoint_cap: DesignStrokeCap::None,
            dashes: DesignStrokeDashes::solid(),
            dash_cap: DesignStrokeCap::None,
            join: DesignStrokeJoin::Miter,
            miter_angle: 90.,
            variable_width: None,
            complex_stroke: DesignComplexStroke::default(),
            capabilities,
            edit_context,
        }
    }

    /// Compatibility constructor for hosts that formerly supplied one paint.
    pub fn from_paint(paint: DesignPaint, weight: f32, align: DesignStrokeAlign) -> Self {
        Self::for_node(DesignPanelNodeKind::Rectangle, paint, weight, align)
    }

    pub fn alignment_options(&self) -> &'static [DesignStrokeAlign] {
        const CENTER_ONLY: &[DesignStrokeAlign] = &[DesignStrokeAlign::Center];
        const ALL: &[DesignStrokeAlign] = &DesignStrokeAlign::ALL;
        if self.capabilities.position && self.complex_stroke.is_basic() {
            ALL
        } else {
            CENTER_ONLY
        }
    }

    pub fn supports_variable_width(&self) -> bool {
        self.capabilities.variable_width
            && !self.complex_stroke.is_dynamic()
            && !self.complex_stroke.is_opaque()
            && self.edit_context.topology != DesignStrokePathTopology::Branching
    }

    pub fn set_type(&mut self, kind: DesignStrokeType) -> bool {
        if kind == DesignStrokeType::Opaque {
            return false;
        }
        if self.complex_stroke.kind() == kind {
            return true;
        }
        let Some(complex_stroke) = DesignComplexStroke::editable_default(kind) else {
            return false;
        };
        self.complex_stroke = complex_stroke;
        if kind != DesignStrokeType::Basic {
            self.align = DesignStrokeAlign::Center;
            self.dashes = DesignStrokeDashes::solid();
        }
        if kind == DesignStrokeType::Dynamic {
            self.variable_width = None;
        }
        true
    }

    pub fn set_dash_mode(&mut self, mode: DesignStrokeDashMode) -> bool {
        if !self.complex_stroke.is_basic() {
            return false;
        }
        self.dashes.transition(mode);
        true
    }

    pub fn set_dash_pattern(&mut self, dash_pattern: Vec<f32>) -> bool {
        self.complex_stroke.is_basic() && self.dashes.set_pattern(dash_pattern)
    }

    pub fn set_variable_width(
        &mut self,
        variable_width: Option<DesignVariableWidthStroke>,
    ) -> bool {
        if !self.supports_variable_width()
            || variable_width
                .as_ref()
                .is_some_and(|properties| !properties.is_valid())
        {
            return false;
        }
        self.variable_width = variable_width;
        true
    }

    pub fn add_paint(&mut self, paint: DesignPaint) {
        self.paints.push(paint);
    }

    pub fn remove_paint(&mut self, index: usize) -> Option<DesignPaint> {
        (index < self.paints.len()).then(|| self.paints.remove(index))
    }

    pub fn move_paint(&mut self, from: usize, to: usize) -> bool {
        if from >= self.paints.len() || to >= self.paints.len() || from == to {
            return from == to && from < self.paints.len();
        }
        let paint = self.paints.remove(from);
        self.paints.insert(to, paint);
        true
    }
}
