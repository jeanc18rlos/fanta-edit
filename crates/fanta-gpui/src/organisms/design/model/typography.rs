use gpui::SharedString;

use super::*;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignTextResize {
    AutoWidth,
    AutoHeight,
    Fixed,
}

impl DesignTextResize {
    pub const ALL: [Self; 3] = [Self::AutoWidth, Self::AutoHeight, Self::Fixed];

    pub const fn label(self) -> &'static str {
        match self {
            Self::AutoWidth => "Auto width",
            Self::AutoHeight => "Auto height",
            Self::Fixed => "Fixed size",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum DesignLineHeight {
    #[default]
    Auto,
    Pixels(f32),
    Percent(f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DesignLetterSpacing {
    Pixels(f32),
    Percent(f32),
}

impl Default for DesignLetterSpacing {
    fn default() -> Self {
        Self::Pixels(0.)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignTextHorizontalAlignment {
    Left,
    Center,
    Right,
    Justified,
}

impl DesignTextHorizontalAlignment {
    pub const ALL: [Self; 4] = [Self::Left, Self::Center, Self::Right, Self::Justified];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Center => "Center",
            Self::Right => "Right",
            Self::Justified => "Justified",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignTextVerticalAlignment {
    Top,
    Center,
    Bottom,
}

impl DesignTextVerticalAlignment {
    pub const ALL: [Self; 3] = [Self::Top, Self::Center, Self::Bottom];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Top => "Top",
            Self::Center => "Center",
            Self::Bottom => "Bottom",
        }
    }
}

/// Figma's `LeadingTrim` value, surfaced as Vertical trim in Type settings.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignTextLeadingTrim {
    None,
    CapHeight,
}

impl DesignTextLeadingTrim {
    pub const ALL: [Self; 2] = [Self::None, Self::CapHeight];

    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::CapHeight => "Cap height",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignTextCase {
    Original,
    Uppercase,
    Lowercase,
    TitleCase,
    SmallCaps,
    SmallCapsForced,
}

impl DesignTextCase {
    pub const ALL: [Self; 6] = [
        Self::Original,
        Self::Uppercase,
        Self::Lowercase,
        Self::TitleCase,
        Self::SmallCaps,
        Self::SmallCapsForced,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Original => "Original",
            Self::Uppercase => "Uppercase",
            Self::Lowercase => "Lowercase",
            Self::TitleCase => "Title Case",
            Self::SmallCaps => "Small Caps",
            Self::SmallCapsForced => "Small Caps Forced",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignTextDecoration {
    None,
    Underline,
    Strikethrough,
}

/// Exact writable `TextNode.textDecorationStyle` values.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignTextDecorationStyle {
    Solid,
    Wavy,
    Dotted,
}

impl DesignTextDecorationStyle {
    pub const ALL: [Self; 3] = [Self::Solid, Self::Wavy, Self::Dotted];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Solid => "Solid",
            Self::Wavy => "Wavy",
            Self::Dotted => "Dotted",
        }
    }
}

/// Lossless writable value for `textDecorationOffset` and
/// `textDecorationThickness`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DesignTextDecorationMetric {
    Auto,
    Pixels(f32),
    Percent(f32),
}

impl DesignTextDecorationMetric {
    pub const fn is_finite(self) -> bool {
        match self {
            Self::Auto => true,
            Self::Pixels(value) | Self::Percent(value) => value.is_finite(),
        }
    }

    pub const fn is_non_negative(self) -> bool {
        match self {
            Self::Auto => true,
            Self::Pixels(value) | Self::Percent(value) => value.is_finite() && value >= 0.,
        }
    }
}

/// Writable `TextNode.textDecorationColor`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DesignTextDecorationColor {
    Auto,
    Solid(DesignColor),
}

/// Advanced underline/strikethrough fields exposed by the current Text API.
///
/// Hosts set this to `None` when no decoration is active or when these fields
/// are unavailable for the current selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DesignTextDecorationDetails {
    pub style: DesignTextDecorationStyle,
    pub offset: DesignTextDecorationMetric,
    pub thickness: DesignTextDecorationMetric,
    pub color: DesignTextDecorationColor,
    pub skip_ink: bool,
}

impl Default for DesignTextDecorationDetails {
    fn default() -> Self {
        Self {
            style: DesignTextDecorationStyle::Solid,
            offset: DesignTextDecorationMetric::Auto,
            thickness: DesignTextDecorationMetric::Auto,
            color: DesignTextDecorationColor::Auto,
            skip_ink: true,
        }
    }
}

impl DesignTextDecoration {
    pub const ALL: [Self; 3] = [Self::None, Self::Underline, Self::Strikethrough];

    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Underline => "Underline",
            Self::Strikethrough => "Strikethrough",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignTextList {
    None,
    Bulleted,
    Numbered,
}

impl DesignTextList {
    pub const ALL: [Self; 3] = [Self::None, Self::Bulleted, Self::Numbered];

    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Bulleted => "Bulleted list",
            Self::Numbered => "Numbered list",
        }
    }
}

/// Four-character registered OpenType tags remain distinguishable from
/// opaque future tags supplied by a host.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum DesignOpenTypeFeatureTag {
    Registered(SharedString),
    Opaque(SharedString),
}

impl DesignOpenTypeFeatureTag {
    /// Builds a registered OpenType tag only when the exact UTF-8 value is
    /// four ASCII characters. Unknown four-character tags are still valid.
    pub fn registered(tag: impl Into<SharedString>) -> Option<Self> {
        let tag = tag.into();
        (tag.len() == 4 && tag.is_ascii()).then_some(Self::Registered(tag))
    }

    /// Preserves a host-defined tag without normalizing or interpreting it.
    pub fn opaque(tag: impl Into<SharedString>) -> Self {
        Self::Opaque(tag.into())
    }

    pub const fn api_name(&self) -> &SharedString {
        match self {
            Self::Registered(tag) | Self::Opaque(tag) => tag,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DesignOpenTypeFeatureAvailability {
    Available,
    Unavailable { reason: SharedString },
}

impl DesignOpenTypeFeatureAvailability {
    pub const fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }
}

/// One exact feature record from the selected font.
///
/// Unlike `TextNode.openTypeFeatures`, this record keeps both the font default
/// and current value so the panel never infers omitted-default semantics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignOpenTypeFeature {
    pub tag: DesignOpenTypeFeatureTag,
    pub name: SharedString,
    pub default_enabled: bool,
    pub enabled: bool,
    pub availability: DesignOpenTypeFeatureAvailability,
    pub preview: Option<SharedString>,
}

impl DesignOpenTypeFeature {
    pub fn new(
        tag: DesignOpenTypeFeatureTag,
        name: impl Into<SharedString>,
        default_enabled: bool,
        enabled: bool,
    ) -> Self {
        Self {
            tag,
            name: name.into(),
            default_enabled,
            enabled,
            availability: DesignOpenTypeFeatureAvailability::Available,
            preview: None,
        }
    }

    pub fn unavailable(mut self, reason: impl Into<SharedString>) -> Self {
        self.availability = DesignOpenTypeFeatureAvailability::Unavailable {
            reason: reason.into(),
        };
        self
    }

    pub fn with_preview(mut self, preview: impl Into<SharedString>) -> Self {
        self.preview = Some(preview.into());
        self
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum DesignFontSource {
    Local,
    Library {
        library_id: SharedString,
        library_name: SharedString,
    },
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DesignFontSelection {
    pub source: DesignFontSource,
    pub family_id: SharedString,
    pub style_id: SharedString,
}

impl DesignFontSelection {
    pub fn local(family_id: impl Into<SharedString>, style_id: impl Into<SharedString>) -> Self {
        Self {
            source: DesignFontSource::Local,
            family_id: family_id.into(),
            style_id: style_id.into(),
        }
    }

    pub fn library(
        library_id: impl Into<SharedString>,
        library_name: impl Into<SharedString>,
        family_id: impl Into<SharedString>,
        style_id: impl Into<SharedString>,
    ) -> Self {
        Self {
            source: DesignFontSource::Library {
                library_id: library_id.into(),
                library_name: library_name.into(),
            },
            family_id: family_id.into(),
            style_id: style_id.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DesignFontAvailability {
    /// The font can be loaded and applied immediately.
    Imported,
    /// A library font must be imported before it can be applied.
    Available,
    Missing {
        reason: SharedString,
    },
    Unavailable {
        reason: SharedString,
    },
}

impl DesignFontAvailability {
    pub const fn can_apply(&self) -> bool {
        matches!(self, Self::Imported)
    }

    pub const fn can_import(&self) -> bool {
        matches!(self, Self::Available)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignFontStyle {
    pub id: SharedString,
    pub name: SharedString,
    pub weight: Option<u16>,
    pub italic: bool,
    pub availability: DesignFontAvailability,
    pub preview: Option<SharedString>,
}

impl DesignFontStyle {
    pub fn imported(id: impl Into<SharedString>, name: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            weight: None,
            italic: false,
            availability: DesignFontAvailability::Imported,
            preview: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignFontFamily {
    pub id: SharedString,
    pub name: SharedString,
    pub source: DesignFontSource,
    pub styles: Vec<DesignFontStyle>,
}

impl DesignFontFamily {
    pub fn local(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        styles: impl IntoIterator<Item = DesignFontStyle>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            source: DesignFontSource::Local,
            styles: styles.into_iter().collect(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum DesignFontCatalogState {
    #[default]
    Loading,
    Ready,
    Unavailable {
        reason: SharedString,
    },
}

/// Immutable result of the host's font enumeration/loading adapter.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DesignFontViewData {
    pub state: DesignFontCatalogState,
    pub families: Vec<DesignFontFamily>,
}

impl DesignFontViewData {
    pub fn ready(families: impl IntoIterator<Item = DesignFontFamily>) -> Self {
        Self {
            state: DesignFontCatalogState::Ready,
            families: families.into_iter().collect(),
        }
    }

    pub fn font(
        &self,
        selection: &DesignFontSelection,
    ) -> Option<(&DesignFontFamily, &DesignFontStyle)> {
        let family = self
            .families
            .iter()
            .find(|family| family.id == selection.family_id && family.source == selection.source)?;
        let style = family
            .styles
            .iter()
            .find(|style| style.id == selection.style_id)?;
        Some((family, style))
    }

    pub fn matching(
        &self,
        query: &str,
    ) -> impl Iterator<Item = (&DesignFontFamily, &DesignFontStyle)> {
        let query = query.trim().to_ascii_lowercase();
        self.families.iter().flat_map(move |family| {
            let query = query.clone();
            family
                .styles
                .iter()
                .filter(move |style| {
                    query.is_empty()
                        || family.name.to_ascii_lowercase().contains(&query)
                        || style.name.to_ascii_lowercase().contains(&query)
                })
                .map(move |style| (family, style))
        })
    }
}

/// One immutable host-supplied text style shown in the Typography picker.
///
/// `id` is opaque and stable within its page or library scope. The font
/// summary is presentation data and lets the picker describe a style without
/// synthesizing or applying typography values itself.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignTypographyStyle {
    pub id: SharedString,
    pub name: SharedString,
    pub family: SharedString,
    pub font_style: SharedString,
    pub size: f32,
}

impl DesignTypographyStyle {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        family: impl Into<SharedString>,
        font_style: impl Into<SharedString>,
        size: f32,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            family: family.into(),
            font_style: font_style.into(),
            size,
        }
    }
}

/// A named host-supplied library and its ordered text styles.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignTypographyStyleLibrary {
    pub id: SharedString,
    pub name: SharedString,
    pub styles: Vec<DesignTypographyStyle>,
}

impl DesignTypographyStyleLibrary {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        styles: impl IntoIterator<Item = DesignTypographyStyle>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            styles: styles.into_iter().collect(),
        }
    }
}

/// Stable scope for a text-style selection.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum DesignTypographyStyleSource {
    Page,
    Library { library_id: SharedString },
}

/// Opaque text-style identity returned to the host.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DesignTypographyStyleSelection {
    pub source: DesignTypographyStyleSource,
    pub style_id: SharedString,
}

impl DesignTypographyStyleSelection {
    pub fn page(style_id: impl Into<SharedString>) -> Self {
        Self {
            source: DesignTypographyStyleSource::Page,
            style_id: style_id.into(),
        }
    }

    pub fn library(library_id: impl Into<SharedString>, style_id: impl Into<SharedString>) -> Self {
        Self {
            source: DesignTypographyStyleSource::Library {
                library_id: library_id.into(),
            },
            style_id: style_id.into(),
        }
    }
}

/// Immutable text-style snapshot supplied by a Design-panel host.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DesignTypographyStyleViewData {
    pub page_styles: Vec<DesignTypographyStyle>,
    pub libraries: Vec<DesignTypographyStyleLibrary>,
}

impl DesignTypographyStyleViewData {
    pub fn new(
        page_styles: impl IntoIterator<Item = DesignTypographyStyle>,
        libraries: impl IntoIterator<Item = DesignTypographyStyleLibrary>,
    ) -> Self {
        Self {
            page_styles: page_styles.into_iter().collect(),
            libraries: libraries.into_iter().collect(),
        }
    }

    pub const fn is_empty(&self) -> bool {
        self.page_styles.is_empty() && self.libraries.is_empty()
    }

    pub fn style(
        &self,
        selection: &DesignTypographyStyleSelection,
    ) -> Option<&DesignTypographyStyle> {
        match &selection.source {
            DesignTypographyStyleSource::Page => self
                .page_styles
                .iter()
                .find(|style| style.id == selection.style_id),
            DesignTypographyStyleSource::Library { library_id } => self
                .libraries
                .iter()
                .find(|library| library.id == *library_id)?
                .styles
                .iter()
                .find(|style| style.id == selection.style_id),
        }
    }
}

/// The text style currently bound to a controlled typography snapshot.
///
/// The display name remains available even when the host no longer exposes
/// the style in the current page/library browser.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignTypographyStyleBinding {
    pub selection: DesignTypographyStyleSelection,
    pub name: SharedString,
    pub can_detach: bool,
}

impl DesignTypographyStyleBinding {
    pub fn new(selection: DesignTypographyStyleSelection, name: impl Into<SharedString>) -> Self {
        Self {
            selection,
            name: name.into(),
            can_detach: true,
        }
    }

    pub fn detachable(mut self, can_detach: bool) -> Self {
        self.can_detach = can_detach;
        self
    }
}

/// Whether one host-enumerated variable-font axis can be changed.
///
/// `ReadOnly` keeps a valid, inspectable axis distinct from an axis the
/// current font/style cannot provide. Hosts may include a reason in either
/// state so disabled controls never need to infer policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DesignFontAxisAvailability {
    Available,
    ReadOnly { reason: Option<SharedString> },
    Unavailable { reason: SharedString },
}

impl DesignFontAxisAvailability {
    pub const fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }

    pub const fn disabled_reason(&self) -> Option<&SharedString> {
        match self {
            Self::Available => None,
            Self::ReadOnly { reason } => reason.as_ref(),
            Self::Unavailable { reason } => Some(reason),
        }
    }
}

/// Optional variable provenance for an axis represented by a bindable
/// typography leaf such as the registered `wght`/font-weight axis.
///
/// Variable-font axes and Figma variables are different features. This
/// metadata therefore never implies that every custom axis is bindable. When
/// present, the axis remains inspectable but direct axis editing is disabled;
/// the host exposes binding replacement/detachment through its ordinary
/// typography-variable surface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignFontAxisBinding {
    pub variable_id: SharedString,
    pub variable_name: SharedString,
    pub collection_name: Option<SharedString>,
}

impl DesignFontAxisBinding {
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

/// One exact axis from the active variable-font face.
///
/// `tag` is the stable OpenType axis identity and is authoritative across
/// host echoes and row reorderings. `default` and `step` are authored by the
/// host/font adapter; the panel does not guess registered-axis semantics.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignFontAxis {
    pub tag: SharedString,
    pub name: SharedString,
    pub value: f32,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub step: f32,
    pub availability: DesignFontAxisAvailability,
    pub binding: Option<DesignFontAxisBinding>,
}

impl DesignFontAxis {
    pub fn new(
        tag: impl Into<SharedString>,
        name: impl Into<SharedString>,
        value: f32,
        min: f32,
        max: f32,
        default: f32,
    ) -> Self {
        Self {
            tag: tag.into(),
            name: name.into(),
            value,
            min,
            max,
            default,
            step: 1.,
            availability: DesignFontAxisAvailability::Available,
            binding: None,
        }
    }

    pub const fn with_step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }

    pub fn read_only(mut self) -> Self {
        self.availability = DesignFontAxisAvailability::ReadOnly { reason: None };
        self
    }

    pub fn read_only_with_reason(mut self, reason: impl Into<SharedString>) -> Self {
        self.availability = DesignFontAxisAvailability::ReadOnly {
            reason: Some(reason.into()),
        };
        self
    }

    pub fn unavailable(mut self, reason: impl Into<SharedString>) -> Self {
        self.availability = DesignFontAxisAvailability::Unavailable {
            reason: reason.into(),
        };
        self
    }

    pub fn with_binding(mut self, binding: DesignFontAxisBinding) -> Self {
        self.binding = Some(binding);
        self
    }

    /// OpenType variation-axis tags are exactly four printable ASCII bytes.
    pub fn has_valid_tag(&self) -> bool {
        self.tag.len() == 4 && self.tag.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
    }

    pub fn has_valid_range(&self) -> bool {
        self.value.is_finite()
            && self.min.is_finite()
            && self.max.is_finite()
            && self.default.is_finite()
            && self.step.is_finite()
            && self.min < self.max
            && self.step > 0.
            && (self.min..=self.max).contains(&self.value)
            && (self.min..=self.max).contains(&self.default)
    }

    pub fn is_editable(&self) -> bool {
        self.has_valid_tag()
            && self.has_valid_range()
            && self.availability.is_available()
            && self.binding.is_none()
    }

    pub fn disabled_reason(&self) -> Option<SharedString> {
        if !self.has_valid_tag() {
            return Some("Variable-font axis tags must be four printable ASCII characters".into());
        }
        if !self.has_valid_range() {
            return Some("The host supplied an invalid variable-font axis range".into());
        }
        if let Some(binding) = &self.binding {
            return Some(format!("Bound to variable {}", binding.variable_name).into());
        }
        self.availability.disabled_reason().cloned().or_else(|| {
            matches!(
                self.availability,
                DesignFontAxisAvailability::ReadOnly { .. }
            )
            .then(|| SharedString::from("This variable-font axis is read-only"))
        })
    }
}

/// Exact `TextPathNode.textPathStartData` value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DesignTextPathStartData {
    /// Zero-based vector segment index.
    pub segment: u32,
    /// Normalized position along `segment`.
    pub position: f32,
}

impl DesignTextPathStartData {
    pub const DEFAULT: Self = Self {
        segment: 0,
        position: 0.,
    };

    pub fn new(segment: u32, position: f32) -> Option<Self> {
        (position.is_finite() && (0. ..=1.).contains(&position))
            .then_some(Self { segment, position })
    }

    pub const fn with_segment(self, segment: u32) -> Self {
        Self { segment, ..self }
    }

    pub fn with_position(self, position: f32) -> Option<Self> {
        Self::new(self.segment, position)
    }
}

impl Default for DesignTextPathStartData {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Host-owned projection of which side of a path currently carries its text.
///
/// Figma exposes a native "Flip text orientation" command in the Typography
/// section, but the public Plugin API does not currently expose a matching
/// orientation property. The inspector therefore treats this as opaque view
/// data and emits a command instead of deriving or mutating vector geometry.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum DesignTextPathOrientation {
    #[default]
    Default,
    Flipped,
}

impl DesignTextPathOrientation {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Flipped => "Flipped",
        }
    }

    pub const fn toggled(self) -> Self {
        match self {
            Self::Default => Self::Flipped,
            Self::Flipped => Self::Default,
        }
    }
}

/// Exact host-authored native TextPath inspector state.
///
/// `show_start_data_debug_controls` is deliberately separate from native
/// orientation support. `textPathStartData` is useful to API hosts and
/// diagnostics, but Figma's native sidebar positions text with a canvas handle
/// rather than exposing segment/normalized-position number fields.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DesignTextPathViewData {
    pub orientation: DesignTextPathOrientation,
    pub can_flip_orientation: bool,
    pub show_start_data_debug_controls: bool,
}

impl DesignTextPathViewData {
    pub const fn new(orientation: DesignTextPathOrientation) -> Self {
        Self {
            orientation,
            can_flip_orientation: true,
            show_start_data_debug_controls: false,
        }
    }

    pub const fn flippable(mut self, can_flip_orientation: bool) -> Self {
        self.can_flip_orientation = can_flip_orientation;
        self
    }

    pub const fn with_start_data_debug_controls(mut self, show: bool) -> Self {
        self.show_start_data_debug_controls = show;
        self
    }
}

impl Default for DesignTextPathViewData {
    fn default() -> Self {
        Self::new(DesignTextPathOrientation::Default)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesignTypography {
    pub style_binding: Option<DesignTypographyStyleBinding>,
    pub family: SharedString,
    pub style: SharedString,
    /// Figma's numeric `fontWeight` value. This remains independent from the
    /// string-valued `fontStyle` field so either leaf can carry its own
    /// variable binding.
    pub weight: f32,
    pub size: f32,
    pub line_height: DesignLineHeight,
    pub letter_spacing: DesignLetterSpacing,
    pub leading_trim: DesignTextLeadingTrim,
    pub paragraph_spacing: f32,
    pub paragraph_indent: f32,
    pub list_spacing: f32,
    pub hanging_punctuation: bool,
    pub hanging_lists: bool,
    pub horizontal_alignment: DesignTextHorizontalAlignment,
    pub vertical_alignment: DesignTextVerticalAlignment,
    pub resize: DesignTextResize,
    /// Whether Figma's ending truncation is enabled for this text layer.
    pub truncate: bool,
    /// Figma's node-level `maxLines`: `None` is native Auto, numeric values
    /// are at least one, and are applicable only under
    /// [`Self::text_max_lines_are_available`].
    pub max_lines: Option<u32>,
    pub decoration: DesignTextDecoration,
    pub decoration_details: Option<DesignTextDecorationDetails>,
    pub case: DesignTextCase,
    pub list: DesignTextList,
    pub open_type_features: Vec<DesignOpenTypeFeature>,
    pub variable_axes: Vec<DesignFontAxis>,
}

impl Default for DesignTypography {
    fn default() -> Self {
        Self {
            style_binding: None,
            family: "Inter".into(),
            style: "Regular".into(),
            weight: 400.,
            size: 16.,
            line_height: DesignLineHeight::Auto,
            letter_spacing: DesignLetterSpacing::Pixels(0.),
            leading_trim: DesignTextLeadingTrim::None,
            paragraph_spacing: 0.,
            paragraph_indent: 0.,
            list_spacing: 0.,
            hanging_punctuation: false,
            hanging_lists: false,
            horizontal_alignment: DesignTextHorizontalAlignment::Left,
            vertical_alignment: DesignTextVerticalAlignment::Top,
            resize: DesignTextResize::AutoHeight,
            truncate: false,
            max_lines: None,
            decoration: DesignTextDecoration::None,
            decoration_details: None,
            case: DesignTextCase::Original,
            list: DesignTextList::None,
            open_type_features: Vec::new(),
            variable_axes: Vec::new(),
        }
    }
}

impl DesignTypography {
    /// Whether Figma exposes the Max lines control for this text-resize state.
    ///
    /// Parent auto-layout sizing is a node-level concern and is intentionally
    /// checked by [`DesignPanelNode::text_max_lines_are_available`].
    pub const fn text_max_lines_are_available(&self) -> bool {
        self.truncate
            && matches!(
                self.resize,
                DesignTextResize::AutoWidth | DesignTextResize::AutoHeight
            )
    }
}
