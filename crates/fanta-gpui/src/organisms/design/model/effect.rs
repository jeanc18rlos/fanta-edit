use gpui::SharedString;

use super::*;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignEffectKind {
    DropShadow,
    InnerShadow,
    LayerBlur,
    BackgroundBlur,
    Noise,
    Texture,
    Glass,
    Shader,
    /// A future effect type that this version of the inspector cannot edit.
    ///
    /// Hosts retain the original type and payload in
    /// [`DesignEffectSettings::Opaque`] so a newer document never loses data.
    Unsupported,
}

impl DesignEffectKind {
    /// Effect kinds that can be selected in the current Design panel.
    ///
    /// `Unsupported` is intentionally omitted: it is a read-only round-trip
    /// fallback for future document data, not a type that users can create.
    pub const ALL: [Self; 8] = [
        Self::DropShadow,
        Self::InnerShadow,
        Self::LayerBlur,
        Self::BackgroundBlur,
        Self::Noise,
        Self::Texture,
        Self::Glass,
        Self::Shader,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::DropShadow => "Drop shadow",
            Self::InnerShadow => "Inner shadow",
            Self::LayerBlur => "Layer blur",
            Self::BackgroundBlur => "Background blur",
            Self::Noise => "Noise",
            Self::Texture => "Texture",
            Self::Glass => "Glass",
            Self::Shader => "Shader",
            Self::Unsupported => "Unsupported effect",
        }
    }

    /// Maximum instances of this kind that Figma permits on one layer.
    pub const fn maximum_per_node(self) -> u8 {
        match self {
            Self::DropShadow | Self::InnerShadow => 8,
            Self::Noise => 2,
            Self::LayerBlur | Self::BackgroundBlur | Self::Texture | Self::Glass => 1,
            // Figma currently documents no per-node shader limit. The panel
            // therefore delegates any lower product limit to host
            // availability while retaining a finite collection guard.
            Self::Shader => u8::MAX,
            Self::Unsupported => 0,
        }
    }
}

/// Host-authored availability for one effect kind.
///
/// This is intentionally separate from Figma's documented per-kind count
/// limits. A host may need to disable Shader while no compatible shader is
/// imported, or disable a beta effect for a particular document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignEffectKindAvailability {
    pub kind: DesignEffectKind,
    pub available: bool,
    pub disabled_reason: Option<SharedString>,
}

impl DesignEffectKindAvailability {
    pub const fn available(kind: DesignEffectKind) -> Self {
        Self {
            kind,
            available: true,
            disabled_reason: None,
        }
    }

    pub fn unavailable(kind: DesignEffectKind, reason: impl Into<SharedString>) -> Self {
        Self {
            kind,
            available: false,
            disabled_reason: Some(reason.into()),
        }
    }
}

/// Exact node-context capabilities for effects whose applicability cannot be
/// inferred safely by a reusable inspector.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignEffectCapabilities {
    pub kind_availability: Vec<DesignEffectKindAvailability>,
    /// Whether Drop/Inner shadow spread is legal for this exact node.
    pub shadow_spread: bool,
    /// Whether Figma's "Show behind transparent areas" control is applicable
    /// for a Drop shadow on this exact node.
    pub show_shadow_behind_transparent_areas: bool,
}

impl DesignEffectCapabilities {
    pub fn for_node_kind(kind: DesignPanelNodeKind) -> Self {
        Self {
            kind_availability: DesignEffectKind::ALL
                .into_iter()
                .map(|effect_kind| {
                    if effect_kind == DesignEffectKind::Shader {
                        DesignEffectKindAvailability::unavailable(
                            effect_kind,
                            "No compatible shader is imported",
                        )
                    } else {
                        DesignEffectKindAvailability::available(effect_kind)
                    }
                })
                .collect(),
            shadow_spread: matches!(
                kind,
                DesignPanelNodeKind::Rectangle | DesignPanelNodeKind::Ellipse
            ),
            // Applicability depends on the host's resolved fills, strokes,
            // opacity, and blend modes; conservative is the only safe preset.
            show_shadow_behind_transparent_areas: false,
        }
    }

    pub fn availability(&self, kind: DesignEffectKind) -> Option<&DesignEffectKindAvailability> {
        self.kind_availability
            .iter()
            .find(|availability| availability.kind == kind)
    }

    pub fn kind_is_available(&self, kind: DesignEffectKind) -> bool {
        self.availability(kind)
            .is_some_and(|availability| availability.available)
    }

    pub fn set_kind_availability(&mut self, availability: DesignEffectKindAvailability) {
        if let Some(current) = self
            .kind_availability
            .iter_mut()
            .find(|current| current.kind == availability.kind)
        {
            *current = availability;
        } else {
            self.kind_availability.push(availability);
        }
    }
}

/// A two-axis value used by effect controls.
///
/// Blur control points use normalized object coordinates. Noise and texture
/// sizes use the same display-space units supplied by the host.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DesignEffectVector {
    pub x: f32,
    pub y: f32,
}

impl DesignEffectVector {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub const fn splat(value: f32) -> Self {
        Self { x: value, y: value }
    }
}

/// Figma effect leaves that can be bound through
/// `setBoundVariableForEffect`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignEffectVariableField {
    Radius,
    Color,
    Spread,
    OffsetX,
    OffsetY,
}

impl DesignEffectVariableField {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Radius => "Blur",
            Self::Color => "Color",
            Self::Spread => "Spread",
            Self::OffsetX => "Offset X",
            Self::OffsetY => "Offset Y",
        }
    }

    pub const fn value_kind(self) -> DesignEffectVariableKind {
        match self {
            Self::Color => DesignEffectVariableKind::Color,
            Self::Radius | Self::Spread | Self::OffsetX | Self::OffsetY => {
                DesignEffectVariableKind::Float
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignEffectVariableKind {
    Float,
    Color,
}

/// One host-supplied variable shown for a bindable effect leaf.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignEffectVariable {
    pub id: SharedString,
    pub name: SharedString,
    pub collection_name: SharedString,
    pub kind: DesignEffectVariableKind,
    pub remote: bool,
}

impl DesignEffectVariable {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        collection_name: impl Into<SharedString>,
        kind: DesignEffectVariableKind,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            collection_name: collection_name.into(),
            kind,
            remote: false,
        }
    }

    pub const fn remote(mut self, remote: bool) -> Self {
        self.remote = remote;
        self
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DesignEffectVariableViewData {
    pub variables: Vec<DesignEffectVariable>,
}

impl DesignEffectVariableViewData {
    pub fn new(variables: impl IntoIterator<Item = DesignEffectVariable>) -> Self {
        Self {
            variables: variables.into_iter().collect(),
        }
    }

    pub fn variable(&self, variable_id: &str) -> Option<&DesignEffectVariable> {
        self.variables
            .iter()
            .find(|variable| variable.id.as_ref() == variable_id)
    }

    pub fn compatible(
        &self,
        field: DesignEffectVariableField,
    ) -> impl Iterator<Item = &DesignEffectVariable> {
        self.variables
            .iter()
            .filter(move |variable| variable.kind == field.value_kind())
    }
}

/// Current binding metadata for one effect leaf.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignEffectVariableBinding {
    pub field: DesignEffectVariableField,
    pub variable_id: SharedString,
    pub variable_name: SharedString,
    pub collection_name: SharedString,
    pub can_detach: bool,
}

impl DesignEffectVariableBinding {
    pub fn new(
        field: DesignEffectVariableField,
        variable_id: impl Into<SharedString>,
        variable_name: impl Into<SharedString>,
        collection_name: impl Into<SharedString>,
    ) -> Self {
        Self {
            field,
            variable_id: variable_id.into(),
            variable_name: variable_name.into(),
            collection_name: collection_name.into(),
            can_detach: true,
        }
    }

    pub const fn detachable(mut self, can_detach: bool) -> Self {
        self.can_detach = can_detach;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignShaderPropertyKind {
    Boolean,
    Text,
    Number,
    Image,
    InstanceSwap,
    Slot,
    Color,
    Point,
    Line,
    Circle,
    CirclePoint,
    ColorPoint,
    Gradient,
    Unsupported,
}

impl DesignShaderPropertyKind {
    pub const ALL: [Self; 14] = [
        Self::Boolean,
        Self::Text,
        Self::Number,
        Self::Image,
        Self::InstanceSwap,
        Self::Slot,
        Self::Color,
        Self::Point,
        Self::Line,
        Self::Circle,
        Self::CirclePoint,
        Self::ColorPoint,
        Self::Gradient,
        Self::Unsupported,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Boolean => "Boolean",
            Self::Text => "Text",
            Self::Number => "Number",
            Self::Image => "Image",
            Self::InstanceSwap => "Instance swap",
            Self::Slot => "Slot",
            Self::Color => "Color",
            Self::Point => "Point",
            Self::Line => "Line",
            Self::Circle => "Circle",
            Self::CirclePoint => "Circle point",
            Self::ColorPoint => "Color point",
            Self::Gradient => "Gradient",
            Self::Unsupported => "Unsupported",
        }
    }
}

/// Exact value leaf targeted by a host-owned Shader-property resource editor.
///
/// The effect and property-definition IDs on the surrounding intent remain
/// authoritative. A gradient-stop index is an ordering hint because Figma's
/// Shader API does not expose a stable stop ID.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignShaderPropertyEditorTarget {
    Value,
    ColorPointColor,
    GradientStopColor(usize),
}

/// Host surface requested for a Shader-property value.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignShaderPropertyEditorKind {
    Resource,
    Variable,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignShaderGradientStop {
    pub position: f32,
    pub color: DesignColor,
    pub variable_id: Option<SharedString>,
}

impl DesignShaderGradientStop {
    pub const fn new(position: f32, color: DesignColor) -> Self {
        Self {
            position,
            color,
            variable_id: None,
        }
    }
}

/// A structured value for one shader property-definition ID.
///
/// The opaque case preserves newly introduced value shapes without requiring
/// this inspector version to understand or rewrite them.
#[derive(Clone, Debug, PartialEq)]
pub enum DesignShaderPropertyValue {
    Boolean(bool),
    Text(SharedString),
    Number(f32),
    AssetId(SharedString),
    Color(DesignColor),
    Point(DesignEffectVector),
    Line {
        start: DesignEffectVector,
        end: DesignEffectVector,
    },
    Circle {
        center: DesignEffectVector,
        radius: f32,
    },
    CirclePoint {
        center: DesignEffectVector,
        radius: f32,
        angle: f32,
    },
    ColorPoint {
        point: DesignEffectVector,
        color: DesignColor,
        variable_id: Option<SharedString>,
    },
    Gradient(Vec<DesignShaderGradientStop>),
    VariableAlias {
        variable_id: SharedString,
    },
    Opaque {
        type_name: SharedString,
        payload: SharedString,
    },
}

impl DesignShaderPropertyValue {
    pub const fn is_compatible_with(&self, kind: DesignShaderPropertyKind) -> bool {
        matches!(
            (self, kind),
            (Self::Boolean(_), DesignShaderPropertyKind::Boolean)
                | (Self::Text(_), DesignShaderPropertyKind::Text)
                | (Self::Number(_), DesignShaderPropertyKind::Number)
                | (
                    Self::AssetId(_),
                    DesignShaderPropertyKind::Image
                        | DesignShaderPropertyKind::InstanceSwap
                        | DesignShaderPropertyKind::Slot
                )
                | (Self::Color(_), DesignShaderPropertyKind::Color)
                | (Self::Point(_), DesignShaderPropertyKind::Point)
                | (Self::Line { .. }, DesignShaderPropertyKind::Line)
                | (Self::Circle { .. }, DesignShaderPropertyKind::Circle)
                | (
                    Self::CirclePoint { .. },
                    DesignShaderPropertyKind::CirclePoint
                )
                | (
                    Self::ColorPoint { .. },
                    DesignShaderPropertyKind::ColorPoint
                )
                | (Self::Gradient(_), DesignShaderPropertyKind::Gradient)
                | (Self::VariableAlias { .. }, _)
                | (Self::Opaque { .. }, DesignShaderPropertyKind::Unsupported)
        )
    }

    pub const fn variable_alias_id(&self) -> Option<&SharedString> {
        match self {
            Self::VariableAlias { variable_id } => Some(variable_id),
            _ => None,
        }
    }

    pub fn summary(&self) -> SharedString {
        match self {
            Self::Boolean(value) => value.to_string().into(),
            Self::Text(value) | Self::AssetId(value) => value.clone(),
            Self::Number(value) => value.to_string().into(),
            Self::Color(color) => format!("#{}", color.hex()).into(),
            Self::Point(point) => format!("{}, {}", point.x, point.y).into(),
            Self::Line { .. } => "Line".into(),
            Self::Circle { radius, .. } => format!("Circle · {radius}").into(),
            Self::CirclePoint { radius, angle, .. } => {
                format!("Circle · {radius} · {angle}°").into()
            }
            Self::ColorPoint { color, .. } => format!("Point · #{}", color.hex()).into(),
            Self::Gradient(stops) => format!("Gradient · {} stops", stops.len()).into(),
            Self::VariableAlias { variable_id } => format!("Variable · {variable_id}").into(),
            Self::Opaque { type_name, .. } => type_name.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignShaderProperty {
    pub definition_id: SharedString,
    pub name: SharedString,
    pub kind: DesignShaderPropertyKind,
    pub value: DesignShaderPropertyValue,
    pub description: Option<SharedString>,
    pub read_only: bool,
}

impl DesignShaderProperty {
    pub fn new(
        definition_id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        kind: DesignShaderPropertyKind,
        value: DesignShaderPropertyValue,
    ) -> Self {
        Self {
            definition_id: definition_id.into(),
            name: name.into(),
            kind,
            value,
            description: None,
            read_only: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignShaderEffect {
    pub shader_id: SharedString,
    pub name: SharedString,
    pub imported: bool,
    pub properties: Vec<DesignShaderProperty>,
}

impl DesignShaderEffect {
    pub fn new(
        shader_id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        properties: impl IntoIterator<Item = DesignShaderProperty>,
    ) -> Self {
        Self {
            shader_id: shader_id.into(),
            name: name.into(),
            imported: true,
            properties: properties.into_iter().collect(),
        }
    }
}

impl Default for DesignShaderEffect {
    fn default() -> Self {
        Self {
            shader_id: SharedString::default(),
            name: "Choose shader".into(),
            imported: false,
            properties: Vec::new(),
        }
    }
}

/// Lossless host payload for an effect type introduced after this inspector.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignOpaqueEffect {
    pub type_name: SharedString,
    pub summary: SharedString,
    pub payload: SharedString,
}

impl DesignOpaqueEffect {
    pub fn new(
        type_name: impl Into<SharedString>,
        summary: impl Into<SharedString>,
        payload: impl Into<SharedString>,
    ) -> Self {
        Self {
            type_name: type_name.into(),
            summary: summary.into(),
            payload: payload.into(),
        }
    }
}

impl Default for DesignOpaqueEffect {
    fn default() -> Self {
        Self::new("UNKNOWN", "Unsupported effect", "")
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignBlurType {
    Normal,
    Progressive,
}

impl DesignBlurType {
    pub const ALL: [Self; 2] = [Self::Normal, Self::Progressive];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Progressive => "Progressive",
        }
    }
}

/// Settings shared by layer blur and background blur.
///
/// Progressive blur positions are normalized to the effected node's bounds.
#[derive(Clone, Debug, PartialEq)]
pub enum DesignBlurEffect {
    Normal {
        radius: f32,
    },
    Progressive {
        start_radius: f32,
        end_radius: f32,
        start_offset: DesignEffectVector,
        end_offset: DesignEffectVector,
    },
}

impl DesignBlurEffect {
    pub const fn normal(radius: f32) -> Self {
        Self::Normal { radius }
    }

    pub const fn progressive(
        start_radius: f32,
        end_radius: f32,
        start_offset: DesignEffectVector,
        end_offset: DesignEffectVector,
    ) -> Self {
        Self::Progressive {
            start_radius,
            end_radius,
            start_offset,
            end_offset,
        }
    }

    pub const fn blur_type(&self) -> DesignBlurType {
        match self {
            Self::Normal { .. } => DesignBlurType::Normal,
            Self::Progressive { .. } => DesignBlurType::Progressive,
        }
    }

    pub const fn end_radius(&self) -> f32 {
        match self {
            Self::Normal { radius } => *radius,
            Self::Progressive { end_radius, .. } => *end_radius,
        }
    }

    pub fn set_end_radius(&mut self, radius: f32) {
        match self {
            Self::Normal {
                radius: current_radius,
            } => *current_radius = radius,
            Self::Progressive { end_radius, .. } => *end_radius = radius,
        }
    }

    /// Switches normal/progressive UI modes while preserving the end radius.
    pub fn set_blur_type(&mut self, blur_type: DesignBlurType) {
        if self.blur_type() == blur_type {
            return;
        }
        let end_radius = self.end_radius();
        *self = match blur_type {
            DesignBlurType::Normal => Self::normal(end_radius),
            DesignBlurType::Progressive => Self::progressive(
                0.,
                end_radius,
                DesignEffectVector::new(0.5, 0.),
                DesignEffectVector::new(0.5, 1.),
            ),
        };
    }
}

impl Default for DesignBlurEffect {
    fn default() -> Self {
        Self::normal(4.)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignNoiseType {
    Monotone,
    Duotone,
    Multitone,
}

impl DesignNoiseType {
    pub const ALL: [Self; 3] = [Self::Monotone, Self::Duotone, Self::Multitone];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Monotone => "Mono",
            Self::Duotone => "Duo",
            Self::Multitone => "Multi",
        }
    }
}

/// The color controls exposed for one of Figma's three noise modes.
#[derive(Clone, Debug, PartialEq)]
pub enum DesignNoiseColors {
    Monotone {
        color: DesignColor,
    },
    Duotone {
        color: DesignColor,
        secondary_color: DesignColor,
    },
    Multitone {
        opacity: f32,
    },
}

impl DesignNoiseColors {
    pub const fn noise_type(&self) -> DesignNoiseType {
        match self {
            Self::Monotone { .. } => DesignNoiseType::Monotone,
            Self::Duotone { .. } => DesignNoiseType::Duotone,
            Self::Multitone { .. } => DesignNoiseType::Multitone,
        }
    }

    pub const fn primary_color(&self) -> Option<DesignColor> {
        match self {
            Self::Monotone { color } | Self::Duotone { color, .. } => Some(*color),
            Self::Multitone { .. } => None,
        }
    }

    /// Switches the number-of-colors mode with representative values while
    /// preserving the existing primary color when one is available.
    pub fn set_noise_type(&mut self, noise_type: DesignNoiseType) {
        if self.noise_type() == noise_type {
            return;
        }
        let primary = self.primary_color().unwrap_or(DesignColor::BLACK);
        *self = match noise_type {
            DesignNoiseType::Monotone => Self::Monotone { color: primary },
            DesignNoiseType::Duotone => Self::Duotone {
                color: primary,
                secondary_color: DesignColor::WHITE,
            },
            DesignNoiseType::Multitone => Self::Multitone { opacity: 1. },
        };
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignDropShadowEffect {
    pub color: DesignColor,
    pub offset: DesignEffectVector,
    pub radius: f32,
    pub spread: f32,
    pub blend_mode: DesignBlendMode,
    pub show_behind_node: bool,
}

impl Default for DesignDropShadowEffect {
    fn default() -> Self {
        Self {
            color: DesignColor::rgba(0, 0, 0, 0x40),
            offset: DesignEffectVector::new(0., 4.),
            radius: 4.,
            spread: 0.,
            blend_mode: DesignBlendMode::Normal,
            show_behind_node: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignInnerShadowEffect {
    pub color: DesignColor,
    pub offset: DesignEffectVector,
    pub radius: f32,
    pub spread: f32,
    pub blend_mode: DesignBlendMode,
}

impl Default for DesignInnerShadowEffect {
    fn default() -> Self {
        Self {
            color: DesignColor::rgba(0, 0, 0, 0x40),
            offset: DesignEffectVector::new(0., 2.),
            radius: 4.,
            spread: 0.,
            blend_mode: DesignBlendMode::Normal,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignNoiseEffect {
    pub colors: DesignNoiseColors,
    pub size: DesignEffectVector,
    pub density: f32,
    pub blend_mode: DesignBlendMode,
}

impl Default for DesignNoiseEffect {
    fn default() -> Self {
        Self {
            colors: DesignNoiseColors::Monotone {
                color: DesignColor::BLACK,
            },
            size: DesignEffectVector::splat(1.),
            density: 0.5,
            blend_mode: DesignBlendMode::Normal,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignTextureEffect {
    pub size: DesignEffectVector,
    pub radius: f32,
    pub clip_to_shape: bool,
}

impl Default for DesignTextureEffect {
    fn default() -> Self {
        Self {
            size: DesignEffectVector::splat(1.),
            radius: 1.,
            clip_to_shape: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignGlassEffect {
    /// Normalized intensity in the inclusive 0–1 range.
    pub light_intensity: f32,
    /// Light direction in degrees.
    pub light_angle: f32,
    /// Normalized refraction amount in the inclusive 0–1 range.
    pub refraction: f32,
    /// Refraction depth. Figma requires this value to be at least one.
    pub depth: f32,
    /// Normalized chromatic dispersion in the inclusive 0–1 range.
    pub dispersion: f32,
    /// Frost radius.
    pub frost: f32,
    /// Highlight spread exposed by the current Design panel.
    pub splay: f32,
}

impl Default for DesignGlassEffect {
    fn default() -> Self {
        Self {
            light_intensity: 0.5,
            light_angle: -45.,
            refraction: 0.5,
            depth: 10.,
            dispersion: 0.,
            frost: 0.,
            splay: 0.5,
        }
    }
}

/// Effect-specific settings stored by a host-controlled inspector node.
///
/// The discriminant deliberately carries the effect kind so fields that Figma
/// does not expose for a particular kind cannot accidentally leak into its UI.
#[derive(Clone, Debug, PartialEq)]
pub enum DesignEffectSettings {
    DropShadow(DesignDropShadowEffect),
    InnerShadow(DesignInnerShadowEffect),
    LayerBlur(DesignBlurEffect),
    BackgroundBlur(DesignBlurEffect),
    Noise(DesignNoiseEffect),
    Texture(DesignTextureEffect),
    Glass(DesignGlassEffect),
    Shader(DesignShaderEffect),
    Opaque(DesignOpaqueEffect),
}

impl DesignEffectSettings {
    pub const fn kind(&self) -> DesignEffectKind {
        match self {
            Self::DropShadow(_) => DesignEffectKind::DropShadow,
            Self::InnerShadow(_) => DesignEffectKind::InnerShadow,
            Self::LayerBlur(_) => DesignEffectKind::LayerBlur,
            Self::BackgroundBlur(_) => DesignEffectKind::BackgroundBlur,
            Self::Noise(_) => DesignEffectKind::Noise,
            Self::Texture(_) => DesignEffectKind::Texture,
            Self::Glass(_) => DesignEffectKind::Glass,
            Self::Shader(_) => DesignEffectKind::Shader,
            Self::Opaque(_) => DesignEffectKind::Unsupported,
        }
    }

    pub fn default_for_kind(kind: DesignEffectKind) -> Self {
        match kind {
            DesignEffectKind::DropShadow => Self::DropShadow(DesignDropShadowEffect::default()),
            DesignEffectKind::InnerShadow => Self::InnerShadow(DesignInnerShadowEffect::default()),
            DesignEffectKind::LayerBlur => Self::LayerBlur(DesignBlurEffect::default()),
            DesignEffectKind::BackgroundBlur => Self::BackgroundBlur(DesignBlurEffect::default()),
            DesignEffectKind::Noise => Self::Noise(DesignNoiseEffect::default()),
            DesignEffectKind::Texture => Self::Texture(DesignTextureEffect::default()),
            DesignEffectKind::Glass => Self::Glass(DesignGlassEffect::default()),
            DesignEffectKind::Shader => Self::Shader(DesignShaderEffect::default()),
            DesignEffectKind::Unsupported => Self::Opaque(DesignOpaqueEffect::default()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignEffect {
    /// Stable host-owned identity. Empty is a compatibility fallback to index.
    pub id: SharedString,
    /// Compatibility summary used by the original compact effects row.
    pub kind: DesignEffectKind,
    pub visible: bool,
    pub color: DesignColor,
    pub blur: f32,
    pub spread: f32,
    pub offset_x: f32,
    pub offset_y: f32,
    /// Authoritative effect-specific settings for the expanded settings UI.
    pub settings: DesignEffectSettings,
    /// Exact variable aliases currently bound to bindable fields on this
    /// effect. The host echoes this list after apply/detach intents.
    pub variable_bindings: Vec<DesignEffectVariableBinding>,
}

impl DesignEffect {
    pub fn new(kind: DesignEffectKind) -> Self {
        Self::from_settings(true, DesignEffectSettings::default_for_kind(kind))
    }

    pub fn drop_shadow(
        color: DesignColor,
        blur: f32,
        spread: f32,
        offset_x: f32,
        offset_y: f32,
    ) -> Self {
        Self::from_settings(
            true,
            DesignEffectSettings::DropShadow(DesignDropShadowEffect {
                color,
                offset: DesignEffectVector::new(offset_x, offset_y),
                radius: blur,
                spread,
                ..DesignDropShadowEffect::default()
            }),
        )
    }

    pub fn from_settings(visible: bool, settings: DesignEffectSettings) -> Self {
        let mut effect = Self {
            id: SharedString::default(),
            kind: settings.kind(),
            visible,
            color: DesignColor::BLACK,
            blur: 0.,
            spread: 0.,
            offset_x: 0.,
            offset_y: 0.,
            settings,
            variable_bindings: Vec::new(),
        };
        effect.sync_compatibility_summary();
        effect
    }

    pub fn with_id(mut self, id: impl Into<SharedString>) -> Self {
        self.id = id.into();
        self
    }

    pub fn with_variable_binding(mut self, binding: DesignEffectVariableBinding) -> Self {
        self.set_variable_binding(binding);
        self
    }

    pub fn variable_binding(
        &self,
        field: DesignEffectVariableField,
    ) -> Option<&DesignEffectVariableBinding> {
        self.variable_bindings
            .iter()
            .find(|binding| binding.field == field)
    }

    pub fn set_variable_binding(&mut self, binding: DesignEffectVariableBinding) {
        if let Some(current) = self
            .variable_bindings
            .iter_mut()
            .find(|current| current.field == binding.field)
        {
            *current = binding;
        } else {
            self.variable_bindings.push(binding);
        }
    }

    pub fn remove_variable_binding(
        &mut self,
        field: DesignEffectVariableField,
    ) -> Option<DesignEffectVariableBinding> {
        self.variable_bindings
            .iter()
            .position(|binding| binding.field == field)
            .map(|index| self.variable_bindings.remove(index))
    }

    pub fn variable_fields(&self, spread_supported: bool) -> &'static [DesignEffectVariableField] {
        const SHADOW: &[DesignEffectVariableField] = &[
            DesignEffectVariableField::Radius,
            DesignEffectVariableField::Color,
            DesignEffectVariableField::Spread,
            DesignEffectVariableField::OffsetX,
            DesignEffectVariableField::OffsetY,
        ];
        const SHADOW_WITHOUT_SPREAD: &[DesignEffectVariableField] = &[
            DesignEffectVariableField::Radius,
            DesignEffectVariableField::Color,
            DesignEffectVariableField::OffsetX,
            DesignEffectVariableField::OffsetY,
        ];
        const BLUR: &[DesignEffectVariableField] = &[DesignEffectVariableField::Radius];
        const NONE: &[DesignEffectVariableField] = &[];
        match &self.settings {
            DesignEffectSettings::DropShadow(_) | DesignEffectSettings::InnerShadow(_) => {
                if spread_supported {
                    SHADOW
                } else {
                    SHADOW_WITHOUT_SPREAD
                }
            }
            DesignEffectSettings::LayerBlur(_) | DesignEffectSettings::BackgroundBlur(_) => BLUR,
            DesignEffectSettings::Noise(_)
            | DesignEffectSettings::Texture(_)
            | DesignEffectSettings::Glass(_)
            | DesignEffectSettings::Shader(_)
            | DesignEffectSettings::Opaque(_) => NONE,
        }
    }

    /// Replaces an effect kind with its representative settings while
    /// preserving the collection row's visibility.
    pub fn set_kind(&mut self, kind: DesignEffectKind) {
        self.settings = DesignEffectSettings::default_for_kind(kind);
        self.variable_bindings.clear();
        self.sync_compatibility_summary();
    }

    pub fn set_settings(&mut self, settings: DesignEffectSettings) {
        if self.settings.kind() != settings.kind() {
            self.variable_bindings.clear();
        }
        self.settings = settings;
        self.sync_compatibility_summary();
    }

    /// Updates the legacy compact blur field and its corresponding typed leaf.
    pub fn set_blur(&mut self, blur: f32) {
        self.blur = blur;
        match &mut self.settings {
            DesignEffectSettings::DropShadow(settings) => settings.radius = blur,
            DesignEffectSettings::InnerShadow(settings) => settings.radius = blur,
            DesignEffectSettings::LayerBlur(settings)
            | DesignEffectSettings::BackgroundBlur(settings) => settings.set_end_radius(blur),
            DesignEffectSettings::Texture(settings) => settings.radius = blur,
            DesignEffectSettings::Glass(settings) => settings.frost = blur,
            DesignEffectSettings::Noise(_)
            | DesignEffectSettings::Shader(_)
            | DesignEffectSettings::Opaque(_) => {}
        }
    }

    pub fn set_spread(&mut self, spread: f32) {
        self.spread = spread;
        match &mut self.settings {
            DesignEffectSettings::DropShadow(settings) => settings.spread = spread,
            DesignEffectSettings::InnerShadow(settings) => settings.spread = spread,
            _ => {}
        }
    }

    pub fn set_offset_x(&mut self, offset_x: f32) {
        self.offset_x = offset_x;
        match &mut self.settings {
            DesignEffectSettings::DropShadow(settings) => settings.offset.x = offset_x,
            DesignEffectSettings::InnerShadow(settings) => settings.offset.x = offset_x,
            _ => {}
        }
    }

    pub fn set_offset_y(&mut self, offset_y: f32) {
        self.offset_y = offset_y;
        match &mut self.settings {
            DesignEffectSettings::DropShadow(settings) => settings.offset.y = offset_y,
            DesignEffectSettings::InnerShadow(settings) => settings.offset.y = offset_y,
            _ => {}
        }
    }

    /// Rebuilds compatibility fields after a host changes typed settings.
    pub fn sync_compatibility_summary(&mut self) {
        self.kind = self.settings.kind();
        let (color, blur, spread, offset) = match &self.settings {
            DesignEffectSettings::DropShadow(settings) => (
                settings.color,
                settings.radius,
                settings.spread,
                settings.offset,
            ),
            DesignEffectSettings::InnerShadow(settings) => (
                settings.color,
                settings.radius,
                settings.spread,
                settings.offset,
            ),
            DesignEffectSettings::LayerBlur(settings)
            | DesignEffectSettings::BackgroundBlur(settings) => (
                DesignColor::BLACK,
                settings.end_radius(),
                0.,
                DesignEffectVector::default(),
            ),
            DesignEffectSettings::Noise(settings) => {
                let color = match &settings.colors {
                    DesignNoiseColors::Monotone { color }
                    | DesignNoiseColors::Duotone { color, .. } => *color,
                    DesignNoiseColors::Multitone { .. } => DesignColor::BLACK,
                };
                (color, 0., 0., DesignEffectVector::default())
            }
            DesignEffectSettings::Texture(settings) => (
                DesignColor::BLACK,
                settings.radius,
                0.,
                DesignEffectVector::default(),
            ),
            DesignEffectSettings::Glass(settings) => (
                DesignColor::WHITE,
                settings.frost,
                0.,
                DesignEffectVector::default(),
            ),
            DesignEffectSettings::Shader(_) | DesignEffectSettings::Opaque(_) => {
                (DesignColor::BLACK, 0., 0., DesignEffectVector::default())
            }
        };
        self.color = color;
        self.blur = blur;
        self.spread = spread;
        self.offset_x = offset.x;
        self.offset_y = offset.y;
    }
}

/// One host-supplied page or library Effect style.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignEffectStyle {
    pub id: SharedString,
    pub name: SharedString,
    /// Ordered preview summary. Applying remains host-owned and does not copy
    /// these values into the panel's controlled node.
    pub effect_kinds: Vec<DesignEffectKind>,
}

impl DesignEffectStyle {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        effect_kinds: impl IntoIterator<Item = DesignEffectKind>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            effect_kinds: effect_kinds.into_iter().collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignEffectStyleLibrary {
    pub id: SharedString,
    pub name: SharedString,
    pub styles: Vec<DesignEffectStyle>,
}

impl DesignEffectStyleLibrary {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        styles: impl IntoIterator<Item = DesignEffectStyle>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            styles: styles.into_iter().collect(),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum DesignEffectStyleSource {
    Page,
    Library { library_id: SharedString },
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DesignEffectStyleSelection {
    pub source: DesignEffectStyleSource,
    pub style_id: SharedString,
}

impl DesignEffectStyleSelection {
    pub fn page(style_id: impl Into<SharedString>) -> Self {
        Self {
            source: DesignEffectStyleSource::Page,
            style_id: style_id.into(),
        }
    }

    pub fn library(library_id: impl Into<SharedString>, style_id: impl Into<SharedString>) -> Self {
        Self {
            source: DesignEffectStyleSource::Library {
                library_id: library_id.into(),
            },
            style_id: style_id.into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DesignEffectStyleViewData {
    pub page_styles: Vec<DesignEffectStyle>,
    pub libraries: Vec<DesignEffectStyleLibrary>,
}

impl DesignEffectStyleViewData {
    pub fn new(
        page_styles: impl IntoIterator<Item = DesignEffectStyle>,
        libraries: impl IntoIterator<Item = DesignEffectStyleLibrary>,
    ) -> Self {
        Self {
            page_styles: page_styles.into_iter().collect(),
            libraries: libraries.into_iter().collect(),
        }
    }

    pub fn style(&self, selection: &DesignEffectStyleSelection) -> Option<&DesignEffectStyle> {
        match &selection.source {
            DesignEffectStyleSource::Page => self
                .page_styles
                .iter()
                .find(|style| style.id == selection.style_id),
            DesignEffectStyleSource::Library { library_id } => self
                .libraries
                .iter()
                .find(|library| library.id == *library_id)?
                .styles
                .iter()
                .find(|style| style.id == selection.style_id),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignEffectStyleBinding {
    pub selection: DesignEffectStyleSelection,
    pub name: SharedString,
    pub can_detach: bool,
}

impl DesignEffectStyleBinding {
    pub fn new(selection: DesignEffectStyleSelection, name: impl Into<SharedString>) -> Self {
        Self {
            selection,
            name: name.into(),
            can_detach: true,
        }
    }

    pub const fn detachable(mut self, can_detach: bool) -> Self {
        self.can_detach = can_detach;
        self
    }
}
