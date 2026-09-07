use std::collections::HashSet;

use gpui::SharedString;

use super::*;

/// A host-provided color represented in display-space channels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DesignColor {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

impl DesignColor {
    pub const BLACK: Self = Self::rgb(0x1e, 0x1e, 0x1e);
    pub const WHITE: Self = Self::rgb(0xff, 0xff, 0xff);
    pub const BLUE: Self = Self::rgb(0x4c, 0x86, 0xf7);
    pub const PURPLE: Self = Self::rgb(0x97, 0x5f, 0xf7);

    pub const fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self {
            red,
            green,
            blue,
            alpha: u8::MAX,
        }
    }

    pub const fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }

    pub fn hex(self) -> SharedString {
        if self.alpha == u8::MAX {
            format!("{:02X}{:02X}{:02X}", self.red, self.green, self.blue).into()
        } else {
            format!(
                "{:02X}{:02X}{:02X}{:02X}",
                self.red, self.green, self.blue, self.alpha
            )
            .into()
        }
    }
}

/// Figma's six top-level paint classifications.
///
/// Gradient direction remains represented by [`DesignPaintKind`], while this
/// type is suitable for the first level of a paint-type picker.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignPaintType {
    Solid,
    Gradient,
    Pattern,
    Image,
    Video,
    Shader,
    /// A host paint the built-in picker cannot edit without losing data.
    Unsupported,
}

impl DesignPaintType {
    /// Paint types directly creatable by the built-in picker.
    ///
    /// Shader creation starts by choosing a host-supplied definition; the
    /// picker never manufactures a shader id.
    pub const ALL: [Self; 6] = [
        Self::Solid,
        Self::Gradient,
        Self::Pattern,
        Self::Image,
        Self::Video,
        Self::Shader,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Solid => "Solid",
            Self::Gradient => "Gradient",
            Self::Pattern => "Pattern",
            Self::Image => "Image",
            Self::Video => "Video",
            Self::Shader => "Shader",
            Self::Unsupported => "Unsupported",
        }
    }

    /// Returns the representative concrete kind for a newly selected type.
    ///
    /// Existing gradient paints keep their current subtype; Linear is only the
    /// default when creating a gradient from the top-level classification.
    pub const fn default_kind(self) -> DesignPaintKind {
        match self {
            Self::Solid => DesignPaintKind::Solid,
            Self::Gradient => DesignPaintKind::LinearGradient,
            Self::Pattern => DesignPaintKind::Pattern,
            Self::Image => DesignPaintKind::Image,
            Self::Video => DesignPaintKind::Video,
            Self::Shader => DesignPaintKind::Shader,
            Self::Unsupported => DesignPaintKind::Unsupported,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignPaintKind {
    Solid,
    LinearGradient,
    RadialGradient,
    AngularGradient,
    DiamondGradient,
    Pattern,
    Image,
    Video,
    Shader,
    Unsupported,
}

impl DesignPaintKind {
    /// Paint kinds directly creatable by the built-in picker.
    pub const ALL: [Self; 9] = [
        Self::Solid,
        Self::LinearGradient,
        Self::RadialGradient,
        Self::AngularGradient,
        Self::DiamondGradient,
        Self::Pattern,
        Self::Image,
        Self::Video,
        Self::Shader,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Solid => "Solid",
            Self::LinearGradient => "Linear",
            Self::RadialGradient => "Radial",
            Self::AngularGradient => "Angular",
            Self::DiamondGradient => "Diamond",
            Self::Pattern => "Pattern",
            Self::Image => "Image",
            Self::Video => "Video",
            Self::Shader => "Shader",
            Self::Unsupported => "Unsupported",
        }
    }

    pub const fn is_gradient(self) -> bool {
        matches!(
            self,
            Self::LinearGradient
                | Self::RadialGradient
                | Self::AngularGradient
                | Self::DiamondGradient
        )
    }

    /// Collapses concrete gradient subtypes into Figma's top-level paint type.
    pub const fn paint_type(self) -> DesignPaintType {
        match self {
            Self::Solid => DesignPaintType::Solid,
            Self::LinearGradient
            | Self::RadialGradient
            | Self::AngularGradient
            | Self::DiamondGradient => DesignPaintType::Gradient,
            Self::Pattern => DesignPaintType::Pattern,
            Self::Image => DesignPaintType::Image,
            Self::Video => DesignPaintType::Video,
            Self::Shader => DesignPaintType::Shader,
            Self::Unsupported => DesignPaintType::Unsupported,
        }
    }
}

impl From<DesignPaintKind> for DesignPaintType {
    fn from(kind: DesignPaintKind) -> Self {
        kind.paint_type()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignGradientStop {
    /// Stable opaque host identity. Empty retains legacy index targeting.
    pub id: SharedString,
    pub position: f32,
    pub color: DesignColor,
    pub binding: Option<DesignPaintBinding>,
}

impl DesignGradientStop {
    pub fn new(position: f32, color: DesignColor) -> Self {
        Self {
            id: "".into(),
            position,
            color,
            binding: None,
        }
    }

    pub fn with_id(mut self, id: impl Into<SharedString>) -> Self {
        self.id = id.into();
        self
    }

    pub fn with_binding(mut self, binding: DesignPaintBinding) -> Self {
        self.binding = Some(binding);
        self
    }
}

/// Opaque color-variable binding metadata retained at one paint color leaf.
///
/// This is deliberately not a Paint-style binding. Figma stores `fillStyleId`
/// and `strokeStyleId` on the whole ordered paint collection; those identities
/// are represented by [`DesignPaintStyleBinding`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignPaintBinding {
    pub variable_id: SharedString,
    pub variable_name: SharedString,
    pub collection_name: Option<SharedString>,
}

impl DesignPaintBinding {
    pub fn new(
        variable_id: impl Into<SharedString>,
        variable_name: impl Into<SharedString>,
    ) -> Self {
        Self {
            variable_id: variable_id.into(),
            variable_name: variable_name.into(),
            collection_name: None,
        }
    }

    pub fn with_collection(mut self, collection_name: impl Into<SharedString>) -> Self {
        self.collection_name = Some(collection_name.into());
        self
    }
}

/// Compatibility-only host-supplied leaf color preset.
///
/// New hosts should supply [`DesignPaintVariableViewData`] for color-variable
/// rows and [`DesignPaintStyleViewData`] for whole Fill/Stroke styles. This
/// older shape remains available so existing adapters can migrate without
/// conflating the two binding levels.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignColorStyle {
    pub id: SharedString,
    pub name: SharedString,
    pub color: DesignColor,
    pub binding: Option<DesignPaintBinding>,
}

impl DesignColorStyle {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        color: DesignColor,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            color,
            binding: None,
        }
    }

    pub fn with_binding(mut self, binding: DesignPaintBinding) -> Self {
        self.binding = Some(binding);
        self
    }
}

/// A named host-supplied library and its available color styles.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignColorStyleLibrary {
    pub id: SharedString,
    pub name: SharedString,
    pub styles: Vec<DesignColorStyle>,
}

impl DesignColorStyleLibrary {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        styles: impl IntoIterator<Item = DesignColorStyle>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            styles: styles.into_iter().collect(),
        }
    }
}

/// Immutable color-style snapshot supplied by a Design-panel host.
///
/// Page styles populate the Custom tab's "On this page" section. Libraries
/// populate the Libraries tab in their supplied order.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DesignColorStyleViewData {
    pub page_styles: Vec<DesignColorStyle>,
    pub libraries: Vec<DesignColorStyleLibrary>,
}

/// One solid-color sample exposed by a page or library Color style.
///
/// Sampling this value in the paint picker changes only the active solid
/// color or gradient-stop leaf. It does not attach the style to the complete
/// Fill/Stroke collection and it never creates a Color-variable binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignColorStyleSample {
    pub id: SharedString,
    pub name: SharedString,
    pub color: DesignColor,
    pub disabled_reason: Option<SharedString>,
}

impl DesignColorStyleSample {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        color: DesignColor,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            color,
            disabled_reason: None,
        }
    }

    pub fn disabled(mut self, reason: impl Into<SharedString>) -> Self {
        self.disabled_reason = Some(reason.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignColorStyleSampleLibrary {
    pub id: SharedString,
    pub name: SharedString,
    pub samples: Vec<DesignColorStyleSample>,
}

impl DesignColorStyleSampleLibrary {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        samples: impl IntoIterator<Item = DesignColorStyleSample>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            samples: samples.into_iter().collect(),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum DesignColorStyleSampleSource {
    Page,
    Library { library_id: SharedString },
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DesignColorStyleSampleSelection {
    pub source: DesignColorStyleSampleSource,
    pub sample_id: SharedString,
}

impl DesignColorStyleSampleSelection {
    pub fn page(sample_id: impl Into<SharedString>) -> Self {
        Self {
            source: DesignColorStyleSampleSource::Page,
            sample_id: sample_id.into(),
        }
    }

    pub fn library(
        library_id: impl Into<SharedString>,
        sample_id: impl Into<SharedString>,
    ) -> Self {
        Self {
            source: DesignColorStyleSampleSource::Library {
                library_id: library_id.into(),
            },
            sample_id: sample_id.into(),
        }
    }
}

/// Host-filtered Color-style samples shown beside Color variables in the
/// retained paint picker.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DesignColorStyleSampleViewData {
    pub page_samples: Vec<DesignColorStyleSample>,
    pub libraries: Vec<DesignColorStyleSampleLibrary>,
}

impl DesignColorStyleSampleViewData {
    pub fn new(
        page_samples: impl IntoIterator<Item = DesignColorStyleSample>,
        libraries: impl IntoIterator<Item = DesignColorStyleSampleLibrary>,
    ) -> Self {
        Self {
            page_samples: page_samples.into_iter().collect(),
            libraries: libraries.into_iter().collect(),
        }
    }

    pub fn sample(
        &self,
        selection: &DesignColorStyleSampleSelection,
    ) -> Option<&DesignColorStyleSample> {
        match &selection.source {
            DesignColorStyleSampleSource::Page => self
                .page_samples
                .iter()
                .find(|sample| sample.id == selection.sample_id),
            DesignColorStyleSampleSource::Library { library_id } => self
                .libraries
                .iter()
                .find(|library| library.id == *library_id)?
                .samples
                .iter()
                .find(|sample| sample.id == selection.sample_id),
        }
    }
}

/// Import state for one Figma Paint style.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum DesignPaintStyleImportState {
    #[default]
    Imported,
    Available,
}

/// One complete ordered Figma Paint style snapshot.
///
/// A style may contain several solid, gradient, image, pattern, video, or
/// shader paints. Applying it replaces the selected Fill or Stroke paint
/// collection as one host-owned operation; it never targets a single color
/// leaf.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignPaintStyle {
    pub id: SharedString,
    pub name: SharedString,
    pub paints: Vec<DesignPaint>,
    pub import_state: DesignPaintStyleImportState,
}

impl DesignPaintStyle {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        paints: impl IntoIterator<Item = DesignPaint>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            paints: paints.into_iter().collect(),
            import_state: DesignPaintStyleImportState::Imported,
        }
    }

    pub const fn with_import_state(mut self, import_state: DesignPaintStyleImportState) -> Self {
        self.import_state = import_state;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignPaintStyleLibrary {
    pub id: SharedString,
    pub name: SharedString,
    pub styles: Vec<DesignPaintStyle>,
}

impl DesignPaintStyleLibrary {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        styles: impl IntoIterator<Item = DesignPaintStyle>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            styles: styles.into_iter().collect(),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum DesignPaintStyleSource {
    Page,
    Library { library_id: SharedString },
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DesignPaintStyleSelection {
    pub source: DesignPaintStyleSource,
    pub style_id: SharedString,
}

impl DesignPaintStyleSelection {
    pub fn page(style_id: impl Into<SharedString>) -> Self {
        Self {
            source: DesignPaintStyleSource::Page,
            style_id: style_id.into(),
        }
    }

    pub fn library(library_id: impl Into<SharedString>, style_id: impl Into<SharedString>) -> Self {
        Self {
            source: DesignPaintStyleSource::Library {
                library_id: library_id.into(),
            },
            style_id: style_id.into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DesignPaintStyleViewData {
    pub page_styles: Vec<DesignPaintStyle>,
    pub libraries: Vec<DesignPaintStyleLibrary>,
}

impl DesignPaintStyleViewData {
    pub fn new(
        page_styles: impl IntoIterator<Item = DesignPaintStyle>,
        libraries: impl IntoIterator<Item = DesignPaintStyleLibrary>,
    ) -> Self {
        Self {
            page_styles: page_styles.into_iter().collect(),
            libraries: libraries.into_iter().collect(),
        }
    }

    pub fn style(&self, selection: &DesignPaintStyleSelection) -> Option<&DesignPaintStyle> {
        match &selection.source {
            DesignPaintStyleSource::Page => self
                .page_styles
                .iter()
                .find(|style| style.id == selection.style_id),
            DesignPaintStyleSource::Library { library_id } => self
                .libraries
                .iter()
                .find(|library| library.id == *library_id)?
                .styles
                .iter()
                .find(|style| style.id == selection.style_id),
        }
    }

    pub fn style_mut(
        &mut self,
        selection: &DesignPaintStyleSelection,
    ) -> Option<&mut DesignPaintStyle> {
        match &selection.source {
            DesignPaintStyleSource::Page => self
                .page_styles
                .iter_mut()
                .find(|style| style.id == selection.style_id),
            DesignPaintStyleSource::Library { library_id } => self
                .libraries
                .iter_mut()
                .find(|library| library.id == *library_id)?
                .styles
                .iter_mut()
                .find(|style| style.id == selection.style_id),
        }
    }
}

/// Whole-collection `fillStyleId` or `strokeStyleId` binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignPaintStyleBinding {
    pub selection: DesignPaintStyleSelection,
    pub name: SharedString,
    pub can_detach: bool,
}

impl DesignPaintStyleBinding {
    pub fn new(selection: DesignPaintStyleSelection, name: impl Into<SharedString>) -> Self {
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

impl DesignColorStyleViewData {
    pub fn new(
        page_styles: impl IntoIterator<Item = DesignColorStyle>,
        libraries: impl IntoIterator<Item = DesignColorStyleLibrary>,
    ) -> Self {
        Self {
            page_styles: page_styles.into_iter().collect(),
            libraries: libraries.into_iter().collect(),
        }
    }

    pub const fn is_empty(&self) -> bool {
        self.page_styles.is_empty() && self.libraries.is_empty()
    }
}

/// Stable scope for a color-style application intent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DesignColorStyleSource {
    Page,
    Library { library_id: SharedString },
}

/// Opaque style selection returned to the host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignColorStyleSelection {
    pub source: DesignColorStyleSource,
    pub style_id: SharedString,
}

impl DesignColorStyleSelection {
    pub fn page(style_id: impl Into<SharedString>) -> Self {
        Self {
            source: DesignColorStyleSource::Page,
            style_id: style_id.into(),
        }
    }

    pub fn library(library_id: impl Into<SharedString>, style_id: impl Into<SharedString>) -> Self {
        Self {
            source: DesignColorStyleSource::Library {
                library_id: library_id.into(),
            },
            style_id: style_id.into(),
        }
    }
}

/// The exact color leaf within a paint that receives a selected style.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DesignPaintColorTarget {
    Solid,
    GradientStop { stop_id: SharedString, index: usize },
}

/// Display-space affine transform used by gradients and source paints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DesignPaintTransform {
    pub m11: f32,
    pub m12: f32,
    pub m21: f32,
    pub m22: f32,
    pub tx: f32,
    pub ty: f32,
}

impl DesignPaintTransform {
    pub const IDENTITY: Self = Self {
        m11: 1.,
        m12: 0.,
        m21: 0.,
        m22: 1.,
        tx: 0.,
        ty: 0.,
    };

    pub fn rotated(self, degrees: f32) -> Self {
        let radians = degrees.to_radians();
        let cosine = radians.cos();
        let sine = radians.sin();
        self.prepend(Self {
            m11: cosine,
            m12: sine,
            m21: -sine,
            m22: cosine,
            tx: 0.,
            ty: 0.,
        })
    }

    pub fn flipped_horizontal(self) -> Self {
        self.prepend(Self {
            m11: -1.,
            m12: 0.,
            m21: 0.,
            m22: 1.,
            tx: 1.,
            ty: 0.,
        })
    }

    pub fn flipped_vertical(self) -> Self {
        self.prepend(Self {
            m11: 1.,
            m12: 0.,
            m21: 0.,
            m22: -1.,
            tx: 0.,
            ty: 1.,
        })
    }

    fn prepend(self, operation: Self) -> Self {
        Self {
            m11: operation.m11 * self.m11 + operation.m21 * self.m12,
            m12: operation.m12 * self.m11 + operation.m22 * self.m12,
            m21: operation.m11 * self.m21 + operation.m21 * self.m22,
            m22: operation.m12 * self.m21 + operation.m22 * self.m22,
            tx: operation.m11 * self.tx + operation.m21 * self.ty + operation.tx,
            ty: operation.m12 * self.tx + operation.m22 * self.ty + operation.ty,
        }
    }
}

impl Default for DesignPaintTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

pub(super) fn paint_transform_is_finite(transform: DesignPaintTransform) -> bool {
    [
        transform.m11,
        transform.m12,
        transform.m21,
        transform.m22,
        transform.tx,
        transform.ty,
    ]
    .into_iter()
    .all(f32::is_finite)
}

/// Figma `PatternPaint.tileType`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignPatternTileType {
    Rectangular,
    HorizontalHexagonal,
    VerticalHexagonal,
}

impl DesignPatternTileType {
    pub const ALL: [Self; 3] = [
        Self::Rectangular,
        Self::HorizontalHexagonal,
        Self::VerticalHexagonal,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Rectangular => "Rectangular",
            Self::HorizontalHexagonal => "Horizontal hex",
            Self::VerticalHexagonal => "Vertical hex",
        }
    }
}

/// Figma `PatternPaint.horizontalAlignment`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignPatternHorizontalAlignment {
    Start,
    Center,
    End,
}

impl DesignPatternHorizontalAlignment {
    pub const ALL: [Self; 3] = [Self::Start, Self::Center, Self::End];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Center => "Center",
            Self::End => "End",
        }
    }
}

/// Figma `Vector` spacing for a pattern tile.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DesignPatternSpacing {
    pub x: f32,
    pub y: f32,
}

impl DesignPatternSpacing {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignSolidPaint {
    pub color: DesignColor,
    pub binding: Option<DesignPaintBinding>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignGradientPaint {
    pub kind: DesignPaintKind,
    pub stops: Vec<DesignGradientStop>,
    pub transform: DesignPaintTransform,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignPatternPaint {
    /// Exact Figma scene-node id used by `setFillsAsync`/`setStrokesAsync`.
    pub source_node_id: SharedString,
    pub tile_type: DesignPatternTileType,
    pub scaling_factor: f32,
    pub spacing: DesignPatternSpacing,
    pub horizontal_alignment: DesignPatternHorizontalAlignment,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignImagePaint {
    pub source: DesignPaintSource,
    pub placement: DesignMediaPaintPlacement,
    pub filters: DesignImageFilters,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignVideoPaint {
    pub source: DesignPaintSource,
    pub placement: DesignMediaPaintPlacement,
    pub filters: DesignImageFilters,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignShaderPropertyDefinition {
    /// Definition id, not the author-facing property name.
    pub id: SharedString,
    pub name: SharedString,
    pub kind: DesignShaderPropertyKind,
    pub default_value: Option<DesignShaderPropertyValue>,
    pub description: Option<SharedString>,
}

impl DesignShaderPropertyDefinition {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        kind: DesignShaderPropertyKind,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            kind,
            default_value: None,
            description: None,
        }
    }

    pub fn with_default(mut self, value: DesignShaderPropertyValue) -> Self {
        debug_assert!(value.is_compatible_with(self.kind));
        self.default_value = Some(value);
        self
    }

    pub fn with_description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignShaderPropertyAssignment {
    pub definition_id: SharedString,
    pub value: DesignShaderPropertyValue,
}

impl DesignShaderPropertyAssignment {
    pub fn new(definition_id: impl Into<SharedString>, value: DesignShaderPropertyValue) -> Self {
        Self {
            definition_id: definition_id.into(),
            value,
        }
    }
}

/// One fill shader returned by the host's shader discovery/import layer.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignShaderDefinition {
    pub id: SharedString,
    pub name: SharedString,
    pub imported: bool,
    /// Present after import. Empty while an available library shader is only
    /// discoverable, matching Figma's `listAvailableShaders` contract.
    pub property_definitions: Vec<DesignShaderPropertyDefinition>,
}

impl DesignShaderDefinition {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        imported: bool,
        property_definitions: impl IntoIterator<Item = DesignShaderPropertyDefinition>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            imported,
            property_definitions: property_definitions.into_iter().collect(),
        }
    }

    pub fn property(&self, definition_id: &str) -> Option<&DesignShaderPropertyDefinition> {
        self.property_definitions
            .iter()
            .find(|definition| definition.id == definition_id)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignShaderLibrary {
    pub id: SharedString,
    pub name: SharedString,
    pub shaders: Vec<DesignShaderDefinition>,
}

impl DesignShaderLibrary {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        shaders: impl IntoIterator<Item = DesignShaderDefinition>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            shaders: shaders.into_iter().collect(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DesignShaderViewData {
    pub page_shaders: Vec<DesignShaderDefinition>,
    pub libraries: Vec<DesignShaderLibrary>,
}

impl DesignShaderViewData {
    pub fn new(
        page_shaders: impl IntoIterator<Item = DesignShaderDefinition>,
        libraries: impl IntoIterator<Item = DesignShaderLibrary>,
    ) -> Self {
        Self {
            page_shaders: page_shaders.into_iter().collect(),
            libraries: libraries.into_iter().collect(),
        }
    }

    pub fn shader(&self, selection: &DesignShaderSelection) -> Option<&DesignShaderDefinition> {
        match &selection.source {
            DesignShaderSource::Page => self
                .page_shaders
                .iter()
                .find(|shader| shader.id == selection.shader_id),
            DesignShaderSource::Library { library_id } => self
                .libraries
                .iter()
                .find(|library| library.id == *library_id)?
                .shaders
                .iter()
                .find(|shader| shader.id == selection.shader_id),
        }
    }

    pub fn definition(&self, shader_id: &str) -> Option<&DesignShaderDefinition> {
        self.page_shaders
            .iter()
            .chain(
                self.libraries
                    .iter()
                    .flat_map(|library| library.shaders.iter()),
            )
            .find(|shader| shader.id == shader_id)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DesignShaderSource {
    Page,
    Library { library_id: SharedString },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignShaderSelection {
    pub source: DesignShaderSource,
    pub shader_id: SharedString,
}

impl DesignShaderSelection {
    pub fn page(shader_id: impl Into<SharedString>) -> Self {
        Self {
            source: DesignShaderSource::Page,
            shader_id: shader_id.into(),
        }
    }

    pub fn library(
        library_id: impl Into<SharedString>,
        shader_id: impl Into<SharedString>,
    ) -> Self {
        Self {
            source: DesignShaderSource::Library {
                library_id: library_id.into(),
            },
            shader_id: shader_id.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignShaderPaint {
    /// Shader id returned by Figma discovery/import, distinct from the stable
    /// identity of the paint row that owns this payload.
    pub shader_id: SharedString,
    pub name: SharedString,
    /// Current assignments keyed by definition id.
    pub properties: Vec<DesignShaderPropertyAssignment>,
}

impl DesignShaderPaint {
    pub fn new(shader_id: impl Into<SharedString>, name: impl Into<SharedString>) -> Self {
        Self {
            shader_id: shader_id.into(),
            name: name.into(),
            properties: Vec::new(),
        }
    }

    pub fn with_properties(
        mut self,
        properties: impl IntoIterator<Item = DesignShaderPropertyAssignment>,
    ) -> Self {
        self.properties = properties.into_iter().collect();
        self
    }

    /// Builds the paint assignments Figma exposes after applying an imported
    /// shader, including every author default keyed by definition id.
    pub fn from_definition(definition: &DesignShaderDefinition) -> Option<Self> {
        if !definition.imported {
            return None;
        }
        Some(
            Self::new(definition.id.clone(), definition.name.clone()).with_properties(
                definition
                    .property_definitions
                    .iter()
                    .filter_map(|property| {
                        property.default_value.clone().map(|value| {
                            DesignShaderPropertyAssignment::new(property.id.clone(), value)
                        })
                    }),
            ),
        )
    }

    pub fn property(&self, definition_id: &str) -> Option<&DesignShaderPropertyValue> {
        self.properties
            .iter()
            .find(|assignment| assignment.definition_id == definition_id)
            .map(|assignment| &assignment.value)
    }

    pub fn property_assignment(
        &self,
        definition_id: &str,
    ) -> Option<&DesignShaderPropertyAssignment> {
        self.properties
            .iter()
            .find(|assignment| assignment.definition_id == definition_id)
    }
}

/// Lossless payload for paint kinds unknown to the built-in picker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignOpaquePaint {
    pub type_name: SharedString,
    pub payload: SharedString,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DesignPaintPayload {
    Solid(DesignSolidPaint),
    Gradient(DesignGradientPaint),
    Pattern(DesignPatternPaint),
    Image(DesignImagePaint),
    Video(DesignVideoPaint),
    Shader(DesignShaderPaint),
    Unsupported(DesignOpaquePaint),
}

impl DesignPaintPayload {
    pub const fn kind(&self) -> DesignPaintKind {
        match self {
            Self::Solid(_) => DesignPaintKind::Solid,
            Self::Gradient(gradient) => gradient.kind,
            Self::Pattern(_) => DesignPaintKind::Pattern,
            Self::Image(_) => DesignPaintKind::Image,
            Self::Video(_) => DesignPaintKind::Video,
            Self::Shader(_) => DesignPaintKind::Shader,
            Self::Unsupported(_) => DesignPaintKind::Unsupported,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignPaint {
    /// Stable opaque host identity. Empty retains legacy index targeting.
    pub id: SharedString,
    /// Compatibility projection of [`Self::payload`].
    pub kind: DesignPaintKind,
    /// Compatibility projection of the solid color or first gradient stop.
    pub color: DesignColor,
    pub opacity: f32,
    pub visible: bool,
    pub blend_mode: DesignBlendMode,
    /// Compatibility projection of gradient payload stops.
    pub gradient_stops: Vec<DesignGradientStop>,
    /// Canonical type-specific paint data.
    pub payload: DesignPaintPayload,
    /// Host-declared read-only state independent of document permissions.
    pub read_only: bool,
}

impl DesignPaint {
    pub const fn paint_type(&self) -> DesignPaintType {
        self.kind.paint_type()
    }

    pub fn solid(color: DesignColor) -> Self {
        Self {
            id: "".into(),
            kind: DesignPaintKind::Solid,
            color,
            opacity: 100.,
            visible: true,
            blend_mode: DesignBlendMode::Normal,
            gradient_stops: Vec::new(),
            payload: DesignPaintPayload::Solid(DesignSolidPaint {
                color,
                binding: None,
            }),
            read_only: false,
        }
    }

    pub fn gradient(kind: DesignPaintKind, stops: Vec<DesignGradientStop>) -> Self {
        debug_assert!(kind.is_gradient());
        let mut paint = Self {
            id: "".into(),
            kind,
            color: stops
                .first()
                .map(|stop| stop.color)
                .unwrap_or(DesignColor::BLACK),
            opacity: 100.,
            visible: true,
            blend_mode: DesignBlendMode::Normal,
            gradient_stops: stops.clone(),
            payload: DesignPaintPayload::Gradient(DesignGradientPaint {
                kind,
                stops,
                transform: DesignPaintTransform::IDENTITY,
            }),
            read_only: false,
        };
        paint.sync_legacy_projection();
        paint
    }

    pub fn pattern(source_node_id: impl Into<SharedString>) -> Self {
        Self::from_payload(DesignPaintPayload::Pattern(DesignPatternPaint {
            source_node_id: source_node_id.into(),
            tile_type: DesignPatternTileType::Rectangular,
            scaling_factor: 1.,
            spacing: DesignPatternSpacing::default(),
            horizontal_alignment: DesignPatternHorizontalAlignment::Center,
        }))
    }

    pub fn image(source: DesignPaintSource) -> Self {
        Self::from_payload(DesignPaintPayload::Image(DesignImagePaint {
            source,
            placement: DesignMediaPaintPlacement::default(),
            filters: DesignImageFilters::default(),
        }))
    }

    pub fn video(source: DesignPaintSource) -> Self {
        Self::from_payload(DesignPaintPayload::Video(DesignVideoPaint {
            source,
            placement: DesignMediaPaintPlacement::default(),
            filters: DesignImageFilters::default(),
        }))
    }

    pub fn shader(shader_id: impl Into<SharedString>, name: impl Into<SharedString>) -> Self {
        Self::from_payload(DesignPaintPayload::Shader(DesignShaderPaint::new(
            shader_id, name,
        )))
    }

    pub fn unsupported(
        type_name: impl Into<SharedString>,
        payload: impl Into<SharedString>,
    ) -> Self {
        Self::from_payload(DesignPaintPayload::Unsupported(DesignOpaquePaint {
            type_name: type_name.into(),
            payload: payload.into(),
        }))
    }

    pub fn from_payload(payload: DesignPaintPayload) -> Self {
        let mut paint = Self {
            id: "".into(),
            kind: payload.kind(),
            color: DesignColor::BLACK,
            opacity: 100.,
            visible: true,
            blend_mode: DesignBlendMode::Normal,
            gradient_stops: Vec::new(),
            payload,
            read_only: false,
        };
        paint.sync_legacy_projection();
        paint
    }

    pub fn with_id(mut self, id: impl Into<SharedString>) -> Self {
        self.id = id.into();
        self
    }

    pub fn with_blend_mode(mut self, blend_mode: DesignBlendMode) -> Self {
        self.blend_mode = blend_mode;
        self
    }

    pub const fn with_read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    pub fn is_bound(&self) -> bool {
        match &self.payload {
            DesignPaintPayload::Solid(solid) => solid.binding.is_some(),
            DesignPaintPayload::Gradient(gradient) => {
                gradient.stops.iter().any(|stop| stop.binding.is_some())
            }
            DesignPaintPayload::Shader(shader) => shader
                .properties
                .iter()
                .any(|property| property.value.variable_alias_id().is_some()),
            DesignPaintPayload::Pattern(_)
            | DesignPaintPayload::Image(_)
            | DesignPaintPayload::Video(_)
            | DesignPaintPayload::Unsupported(_) => false,
        }
    }

    pub fn selected_color_is_bound(&self, stop_index: usize) -> bool {
        match &self.payload {
            DesignPaintPayload::Solid(solid) => solid.binding.is_some(),
            DesignPaintPayload::Gradient(gradient) => gradient
                .stops
                .get(stop_index.min(gradient.stops.len().saturating_sub(1)))
                .is_some_and(|stop| stop.binding.is_some()),
            _ => false,
        }
    }

    pub fn sync_legacy_projection(&mut self) {
        self.kind = self.payload.kind();
        match &self.payload {
            DesignPaintPayload::Solid(solid) => {
                self.color = solid.color;
                self.gradient_stops.clear();
            }
            DesignPaintPayload::Gradient(gradient) => {
                self.gradient_stops = gradient.stops.clone();
                self.color = gradient
                    .stops
                    .first()
                    .map_or(DesignColor::BLACK, |stop| stop.color);
            }
            DesignPaintPayload::Pattern(_)
            | DesignPaintPayload::Image(_)
            | DesignPaintPayload::Video(_)
            | DesignPaintPayload::Shader(_)
            | DesignPaintPayload::Unsupported(_) => {
                self.gradient_stops.clear();
            }
        }
    }

    pub fn apply_edit(&mut self, edit: &DesignPaintEdit) -> bool {
        let changed = match (&edit.property, &edit.value) {
            (DesignPaintProperty::Opacity, DesignPaintValue::Number(value)) => {
                self.opacity = value.clamp(0., 100.);
                true
            }
            (DesignPaintProperty::Visible, DesignPaintValue::Bool(value)) => {
                self.visible = *value;
                true
            }
            (DesignPaintProperty::BlendMode, DesignPaintValue::BlendMode(value)) => {
                self.blend_mode = *value;
                true
            }
            (DesignPaintProperty::Payload, DesignPaintValue::Payload(payload)) => {
                let mut payload = payload.clone();
                let valid = match &mut payload {
                    DesignPaintPayload::Image(image) => {
                        if let Some(filters) = image.filters.normalized() {
                            image.filters = filters;
                            image.placement.is_valid()
                        } else {
                            false
                        }
                    }
                    DesignPaintPayload::Video(video) => {
                        if let Some(filters) = video.filters.normalized() {
                            video.filters = filters;
                            video.placement.is_valid()
                        } else {
                            false
                        }
                    }
                    _ => true,
                };
                if valid {
                    self.payload = payload;
                }
                valid
            }
            (DesignPaintProperty::Color, DesignPaintValue::Color(color)) => match &mut self.payload
            {
                DesignPaintPayload::Solid(solid) if solid.binding.is_none() => {
                    solid.color = *color;
                    true
                }
                _ => false,
            },
            (DesignPaintProperty::GradientKind, DesignPaintValue::PaintKind(kind))
                if kind.is_gradient() =>
            {
                match &mut self.payload {
                    DesignPaintPayload::Gradient(gradient) => {
                        gradient.kind = *kind;
                        true
                    }
                    _ => false,
                }
            }
            (DesignPaintProperty::GradientTransform, DesignPaintValue::Transform(transform)) => {
                match &mut self.payload {
                    DesignPaintPayload::Gradient(gradient) => {
                        gradient.transform = *transform;
                        true
                    }
                    _ => false,
                }
            }
            (
                DesignPaintProperty::GradientStopColor { stop_id, index },
                DesignPaintValue::Color(color),
            ) => match &mut self.payload {
                DesignPaintPayload::Gradient(gradient) => {
                    if let Some(stop) = find_gradient_stop_mut(&mut gradient.stops, stop_id, *index)
                        && stop.binding.is_none()
                    {
                        stop.color = *color;
                        true
                    } else {
                        false
                    }
                }
                _ => false,
            },
            (
                DesignPaintProperty::GradientStopPosition { stop_id, index },
                DesignPaintValue::Number(position),
            ) => match &mut self.payload {
                DesignPaintPayload::Gradient(gradient) => {
                    if let Some(stop) = find_gradient_stop_mut(&mut gradient.stops, stop_id, *index)
                    {
                        stop.position = position.clamp(0., 1.);
                        gradient.stops.sort_by(|left, right| {
                            left.position
                                .partial_cmp(&right.position)
                                .unwrap_or(std::cmp::Ordering::Equal)
                        });
                        true
                    } else {
                        false
                    }
                }
                _ => false,
            },
            (DesignPaintProperty::GradientStopAdd, DesignPaintValue::GradientStop(stop)) => {
                match &mut self.payload {
                    DesignPaintPayload::Gradient(gradient) => {
                        let mut stop = stop.clone();
                        stop.position = stop.position.clamp(0., 1.);
                        gradient.stops.push(stop);
                        gradient.stops.sort_by(|left, right| {
                            left.position
                                .partial_cmp(&right.position)
                                .unwrap_or(std::cmp::Ordering::Equal)
                        });
                        true
                    }
                    _ => false,
                }
            }
            (
                DesignPaintProperty::GradientStopRemove { stop_id, index },
                DesignPaintValue::None,
            ) => match &mut self.payload {
                DesignPaintPayload::Gradient(gradient) if gradient.stops.len() > 2 => {
                    if let Some(index) = find_gradient_stop_index(&gradient.stops, stop_id, *index)
                    {
                        gradient.stops.remove(index);
                        true
                    } else {
                        false
                    }
                }
                _ => false,
            },
            (DesignPaintProperty::Source, DesignPaintValue::Source(source)) => {
                match &mut self.payload {
                    DesignPaintPayload::Image(image) => {
                        image.source = source.clone();
                        true
                    }
                    DesignPaintPayload::Video(video) => {
                        video.source = source.clone();
                        true
                    }
                    _ => false,
                }
            }
            (
                DesignPaintProperty::PatternSourceNode,
                DesignPaintValue::PatternSourceNode(source_node_id),
            ) => match &mut self.payload {
                DesignPaintPayload::Pattern(pattern) => {
                    pattern.source_node_id = source_node_id.clone();
                    true
                }
                _ => false,
            },
            (DesignPaintProperty::MediaScaleMode, DesignPaintValue::MediaScaleMode(mode)) => {
                match &mut self.payload {
                    DesignPaintPayload::Image(image) => {
                        image.placement = image.placement.with_mode(*mode);
                        true
                    }
                    DesignPaintPayload::Video(video) => {
                        video.placement = video.placement.with_mode(*mode);
                        true
                    }
                    _ => false,
                }
            }
            (DesignPaintProperty::MediaCropTransform, DesignPaintValue::Transform(transform))
                if paint_transform_is_finite(*transform) =>
            {
                match &mut self.payload {
                    DesignPaintPayload::Image(DesignImagePaint {
                        placement: DesignMediaPaintPlacement::Crop { transform: current },
                        ..
                    }) => {
                        *current = *transform;
                        true
                    }
                    DesignPaintPayload::Video(DesignVideoPaint {
                        placement: DesignMediaPaintPlacement::Crop { transform: current },
                        ..
                    }) => {
                        *current = *transform;
                        true
                    }
                    _ => false,
                }
            }
            (
                DesignPaintProperty::MediaTileScalingFactor,
                DesignPaintValue::Number(scaling_factor),
            ) if scaling_factor.is_finite() && *scaling_factor > 0. => match &mut self.payload {
                DesignPaintPayload::Image(DesignImagePaint {
                    placement:
                        DesignMediaPaintPlacement::Tile {
                            scaling_factor: current,
                            ..
                        },
                    ..
                }) => {
                    *current = *scaling_factor;
                    true
                }
                DesignPaintPayload::Video(DesignVideoPaint {
                    placement:
                        DesignMediaPaintPlacement::Tile {
                            scaling_factor: current,
                            ..
                        },
                    ..
                }) => {
                    *current = *scaling_factor;
                    true
                }
                _ => false,
            },
            (
                DesignPaintProperty::MediaQuarterTurn,
                DesignPaintValue::MediaQuarterTurn(rotation),
            ) => match &mut self.payload {
                DesignPaintPayload::Image(DesignImagePaint {
                    placement:
                        DesignMediaPaintPlacement::Fill { rotation: current }
                        | DesignMediaPaintPlacement::Fit { rotation: current }
                        | DesignMediaPaintPlacement::Tile {
                            rotation: current, ..
                        },
                    ..
                }) => {
                    *current = *rotation;
                    true
                }
                DesignPaintPayload::Video(DesignVideoPaint {
                    placement:
                        DesignMediaPaintPlacement::Fill { rotation: current }
                        | DesignMediaPaintPlacement::Fit { rotation: current }
                        | DesignMediaPaintPlacement::Tile {
                            rotation: current, ..
                        },
                    ..
                }) => {
                    *current = *rotation;
                    true
                }
                _ => false,
            },
            (DesignPaintProperty::MediaFilter(filter), DesignPaintValue::Number(value))
                if value.is_finite() =>
            {
                match &mut self.payload {
                    DesignPaintPayload::Image(image) => image.filters.set_value(*filter, *value),
                    DesignPaintPayload::Video(video) => video.filters.set_value(*filter, *value),
                    _ => false,
                }
            }
            (
                DesignPaintProperty::PatternTileType,
                DesignPaintValue::PatternTileType(tile_type),
            ) => match &mut self.payload {
                DesignPaintPayload::Pattern(pattern) => {
                    pattern.tile_type = *tile_type;
                    true
                }
                _ => false,
            },
            (DesignPaintProperty::PatternScalingFactor, DesignPaintValue::Number(scale)) => {
                match &mut self.payload {
                    DesignPaintPayload::Pattern(pattern) => {
                        pattern.scaling_factor = scale.max(0.);
                        true
                    }
                    _ => false,
                }
            }
            (DesignPaintProperty::PatternSpacing, DesignPaintValue::PatternSpacing(spacing)) => {
                match &mut self.payload {
                    DesignPaintPayload::Pattern(pattern) => {
                        pattern.spacing = *spacing;
                        true
                    }
                    _ => false,
                }
            }
            (
                DesignPaintProperty::PatternHorizontalAlignment,
                DesignPaintValue::PatternHorizontalAlignment(alignment),
            ) => match &mut self.payload {
                DesignPaintPayload::Pattern(pattern) => {
                    pattern.horizontal_alignment = *alignment;
                    true
                }
                _ => false,
            },
            (
                DesignPaintProperty::ShaderProperty { definition_id },
                DesignPaintValue::ShaderProperty(value),
            ) => match &mut self.payload {
                DesignPaintPayload::Shader(shader) => {
                    if let Some(assignment) = shader
                        .properties
                        .iter_mut()
                        .find(|assignment| assignment.definition_id == *definition_id)
                    {
                        assignment.value = value.clone();
                    } else {
                        shader.properties.push(DesignShaderPropertyAssignment::new(
                            definition_id.clone(),
                            value.clone(),
                        ));
                    }
                    true
                }
                _ => false,
            },
            _ => false,
        };
        if changed {
            self.sync_legacy_projection();
        }
        changed
    }
}

fn find_gradient_stop_index(
    stops: &[DesignGradientStop],
    stop_id: &SharedString,
    fallback_index: usize,
) -> Option<usize> {
    if !stop_id.is_empty() {
        return stops.iter().position(|stop| stop.id == *stop_id);
    }
    (fallback_index < stops.len()).then_some(fallback_index)
}

fn find_gradient_stop_mut<'a>(
    stops: &'a mut [DesignGradientStop],
    stop_id: &SharedString,
    fallback_index: usize,
) -> Option<&'a mut DesignGradientStop> {
    let index = find_gradient_stop_index(stops, stop_id, fallback_index)?;
    stops.get_mut(index)
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum DesignPaintProperty {
    Payload,
    Opacity,
    Visible,
    BlendMode,
    Color,
    GradientKind,
    GradientTransform,
    GradientStopColor { stop_id: SharedString, index: usize },
    GradientStopPosition { stop_id: SharedString, index: usize },
    GradientStopAdd,
    GradientStopRemove { stop_id: SharedString, index: usize },
    Source,
    MediaScaleMode,
    MediaCropTransform,
    MediaTileScalingFactor,
    MediaQuarterTurn,
    MediaFilter(DesignImageFilter),
    PatternSourceNode,
    PatternTileType,
    PatternScalingFactor,
    PatternSpacing,
    PatternHorizontalAlignment,
    ShaderProperty { definition_id: SharedString },
}

#[derive(Clone, Debug, PartialEq)]
pub enum DesignPaintValue {
    None,
    Bool(bool),
    Number(f32),
    Color(DesignColor),
    BlendMode(DesignBlendMode),
    PaintKind(DesignPaintKind),
    Transform(DesignPaintTransform),
    GradientStop(DesignGradientStop),
    Source(DesignPaintSource),
    MediaScaleMode(DesignMediaPaintScaleMode),
    MediaQuarterTurn(DesignMediaQuarterTurn),
    PatternSourceNode(SharedString),
    PatternTileType(DesignPatternTileType),
    PatternSpacing(DesignPatternSpacing),
    PatternHorizontalAlignment(DesignPatternHorizontalAlignment),
    ShaderProperty(DesignShaderPropertyValue),
    Payload(DesignPaintPayload),
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignPaintEdit {
    pub property: DesignPaintProperty,
    pub value: DesignPaintValue,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignBlendMode {
    PassThrough,
    Normal,
    Darken,
    Multiply,
    LinearBurn,
    ColorBurn,
    Lighten,
    Screen,
    LinearDodge,
    ColorDodge,
    Overlay,
    SoftLight,
    HardLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl DesignBlendMode {
    /// The complete current Figma `BlendMode` set, in menu order.
    pub const ALL: [Self; 19] = [
        Self::PassThrough,
        Self::Normal,
        Self::Darken,
        Self::Multiply,
        Self::LinearBurn,
        Self::ColorBurn,
        Self::Lighten,
        Self::Screen,
        Self::LinearDodge,
        Self::ColorDodge,
        Self::Overlay,
        Self::SoftLight,
        Self::HardLight,
        Self::Difference,
        Self::Exclusion,
        Self::Hue,
        Self::Saturation,
        Self::Color,
        Self::Luminosity,
    ];

    /// Blend choices for paints and effects, where Pass through is not valid.
    pub const NON_PASS_THROUGH: [Self; 18] = [
        Self::Normal,
        Self::Darken,
        Self::Multiply,
        Self::LinearBurn,
        Self::ColorBurn,
        Self::Lighten,
        Self::Screen,
        Self::LinearDodge,
        Self::ColorDodge,
        Self::Overlay,
        Self::SoftLight,
        Self::HardLight,
        Self::Difference,
        Self::Exclusion,
        Self::Hue,
        Self::Saturation,
        Self::Color,
        Self::Luminosity,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::PassThrough => "Pass through",
            Self::Normal => "Normal",
            Self::Darken => "Darken",
            Self::Multiply => "Multiply",
            Self::LinearBurn => "Plus darker",
            Self::ColorBurn => "Color burn",
            Self::Lighten => "Lighten",
            Self::Screen => "Screen",
            Self::LinearDodge => "Plus lighter",
            Self::ColorDodge => "Color dodge",
            Self::Overlay => "Overlay",
            Self::SoftLight => "Soft light",
            Self::HardLight => "Hard light",
            Self::Difference => "Difference",
            Self::Exclusion => "Exclusion",
            Self::Hue => "Hue",
            Self::Saturation => "Saturation",
            Self::Color => "Color",
            Self::Luminosity => "Luminosity",
        }
    }
}

/// Native audience classification used by Figma's color-contrast checker.
///
/// `Auto` is a transient picker choice. Every host snapshot also supplies the
/// concrete category that Auto resolves to for the exact inspected paint
/// occurrence.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum DesignColorContrastCategory {
    #[default]
    Auto,
    LargeText,
    NormalText,
    Graphics,
}

impl DesignColorContrastCategory {
    pub const ALL: [Self; 4] = [
        Self::Auto,
        Self::LargeText,
        Self::NormalText,
        Self::Graphics,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::LargeText => "Large text",
            Self::NormalText => "Normal text",
            Self::Graphics => "Graphics",
        }
    }

    pub const fn is_concrete(self) -> bool {
        !matches!(self, Self::Auto)
    }
}

/// WCAG conformance level selected in Figma's color-contrast checker.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum DesignColorContrastLevel {
    #[default]
    Aa,
    Aaa,
}

impl DesignColorContrastLevel {
    pub const ALL: [Self; 2] = [Self::Aa, Self::Aaa];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Aa => "AA",
            Self::Aaa => "AAA",
        }
    }
}

/// One host-computed nearest compliant color for an exact checker mode.
///
/// The panel intentionally does not guess Figma's correction algorithm. It
/// emits the supplied color through the ordinary phased paint-edit contract
/// and waits for the host to echo the accepted paint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DesignColorContrastCorrection {
    pub category: DesignColorContrastCategory,
    pub level: DesignColorContrastLevel,
    pub color: DesignColor,
}

impl DesignColorContrastCorrection {
    pub const fn new(
        category: DesignColorContrastCategory,
        level: DesignColorContrastLevel,
        color: DesignColor,
    ) -> Self {
        Self {
            category,
            level,
            color,
        }
    }

    pub const fn is_valid(self) -> bool {
        self.category.is_concrete()
            && !(matches!(self.category, DesignColorContrastCategory::Graphics)
                && matches!(self.level, DesignColorContrastLevel::Aaa))
    }
}

/// Host-owned contrast result for one Solid color or one stable gradient stop.
///
/// `ratio` is authoritative. Supplying it, rather than recomputing from a
/// guessed white/black canvas, lets the host account for the exact effective
/// background, opacity, blending, and nested scene context.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignColorContrastLeafViewData {
    pub color_target: DesignPaintColorTarget,
    pub effective_background: DesignColor,
    pub ratio: f32,
    pub automatic_category: DesignColorContrastCategory,
    pub corrections: Vec<DesignColorContrastCorrection>,
    pub disabled_reason: Option<SharedString>,
}

impl DesignColorContrastLeafViewData {
    pub fn new(
        color_target: DesignPaintColorTarget,
        effective_background: DesignColor,
        ratio: f32,
        automatic_category: DesignColorContrastCategory,
    ) -> Self {
        Self {
            color_target,
            effective_background,
            ratio,
            automatic_category,
            corrections: Vec::new(),
            disabled_reason: None,
        }
    }

    pub fn with_corrections(
        mut self,
        corrections: impl IntoIterator<Item = DesignColorContrastCorrection>,
    ) -> Self {
        self.corrections = corrections.into_iter().collect();
        self
    }

    pub fn disabled(mut self, reason: impl Into<SharedString>) -> Self {
        self.disabled_reason = Some(reason.into());
        self
    }

    pub fn is_valid(&self) -> bool {
        if !self.ratio.is_finite()
            || !(1. ..=21.).contains(&self.ratio)
            || !self.automatic_category.is_concrete()
        {
            return false;
        }
        let mut modes = HashSet::with_capacity(self.corrections.len());
        self.corrections.iter().all(|correction| {
            correction.is_valid() && modes.insert((correction.category, correction.level))
        })
    }

    pub fn correction(
        &self,
        category: DesignColorContrastCategory,
        level: DesignColorContrastLevel,
    ) -> Option<DesignColor> {
        self.corrections
            .iter()
            .find(|correction| correction.category == category && correction.level == level)
            .map(|correction| correction.color)
    }
}

/// Stable document target for one paint represented by contrast view data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DesignColorContrastPaintTarget {
    Paint {
        node_id: SharedString,
        collection: DesignPanelCollection,
        paint_id: SharedString,
        index: usize,
    },
    SelectionColor {
        target: DesignPanelTarget,
        selection_color_id: SharedString,
    },
}

impl DesignColorContrastPaintTarget {
    pub fn paint(
        node_id: impl Into<SharedString>,
        collection: DesignPanelCollection,
        paint_id: impl Into<SharedString>,
        index: usize,
    ) -> Self {
        Self::Paint {
            node_id: node_id.into(),
            collection,
            paint_id: paint_id.into(),
            index,
        }
    }

    pub fn selection_color(
        target: DesignPanelTarget,
        selection_color_id: impl Into<SharedString>,
    ) -> Self {
        Self::SelectionColor {
            target,
            selection_color_id: selection_color_id.into(),
        }
    }
}

/// Every contrast leaf for one stable paint occurrence.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignColorContrastPaintViewData {
    pub target: DesignColorContrastPaintTarget,
    pub leaves: Vec<DesignColorContrastLeafViewData>,
}

impl DesignColorContrastPaintViewData {
    pub fn new(
        target: DesignColorContrastPaintTarget,
        leaves: impl IntoIterator<Item = DesignColorContrastLeafViewData>,
    ) -> Self {
        Self {
            target,
            leaves: leaves.into_iter().collect(),
        }
    }

    pub fn is_valid(&self) -> bool {
        if self.leaves.is_empty() || !self.leaves.iter().all(|leaf| leaf.is_valid()) {
            return false;
        }
        let mut solid = false;
        let mut stop_ids = HashSet::with_capacity(self.leaves.len());
        self.leaves.iter().all(|leaf| match &leaf.color_target {
            DesignPaintColorTarget::Solid => {
                if solid || self.leaves.len() != 1 {
                    false
                } else {
                    solid = true;
                    true
                }
            }
            DesignPaintColorTarget::GradientStop { stop_id, index } => {
                let identity = if stop_id.is_empty() {
                    format!("#{index}").into()
                } else {
                    stop_id.clone()
                };
                !solid && stop_ids.insert(identity)
            }
        })
    }

    pub fn leaf(
        &self,
        color_target: &DesignPaintColorTarget,
    ) -> Option<&DesignColorContrastLeafViewData> {
        self.leaves.iter().find(|leaf| {
            leaf.color_target == *color_target
                || matches!(
                    (&leaf.color_target, color_target),
                    (
                        DesignPaintColorTarget::GradientStop {
                            stop_id: left_id,
                            index: left_index,
                        },
                        DesignPaintColorTarget::GradientStop {
                            stop_id: right_id,
                            index: right_index,
                        },
                    ) if (!left_id.is_empty() && left_id == right_id)
                        || (left_id.is_empty() && right_id.is_empty() && left_index == right_index)
                )
        })
    }
}

/// Host-owned contrast catalog scoped to the current inspector selection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DesignColorContrastViewData {
    pub paints: Vec<DesignColorContrastPaintViewData>,
}

impl DesignColorContrastViewData {
    pub fn new(paints: impl IntoIterator<Item = DesignColorContrastPaintViewData>) -> Self {
        let mut view_data = Self {
            paints: paints.into_iter().collect(),
        };
        view_data
            .paints
            .retain(DesignColorContrastPaintViewData::is_valid);
        view_data
    }

    pub fn paint(
        &self,
        target: &DesignColorContrastPaintTarget,
    ) -> Option<&DesignColorContrastPaintViewData> {
        self.paints.iter().find(|paint| paint.target == *target)
    }
}
