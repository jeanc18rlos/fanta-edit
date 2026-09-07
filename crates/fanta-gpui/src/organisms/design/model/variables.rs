use gpui::SharedString;

use super::*;

/// Fully resolved type of a Figma variable after following aliases.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignVariableResolvedType {
    Boolean,
    Color,
    Float,
    String,
}

impl DesignVariableResolvedType {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Boolean => "Boolean",
            Self::Color => "Color",
            Self::Float => "Number",
            Self::String => "String",
        }
    }
}

/// Figma's variable-picker scopes.
///
/// `Opaque` preserves a scope introduced after this crate version without
/// silently widening it to `AllScopes`.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum DesignVariableScope {
    AllScopes,
    TextContent,
    CornerRadius,
    WidthHeight,
    Gap,
    AllFills,
    FrameFill,
    ShapeFill,
    TextFill,
    StrokeColor,
    EffectColor,
    StrokeFloat,
    EffectFloat,
    Opacity,
    FontFamily,
    FontStyle,
    FontWeight,
    FontSize,
    LineHeight,
    LetterSpacing,
    ParagraphSpacing,
    ParagraphIndent,
    Opaque(SharedString),
}

impl DesignVariableScope {
    pub fn api_name(&self) -> &str {
        match self {
            Self::AllScopes => "ALL_SCOPES",
            Self::TextContent => "TEXT_CONTENT",
            Self::CornerRadius => "CORNER_RADIUS",
            Self::WidthHeight => "WIDTH_HEIGHT",
            Self::Gap => "GAP",
            Self::AllFills => "ALL_FILLS",
            Self::FrameFill => "FRAME_FILL",
            Self::ShapeFill => "SHAPE_FILL",
            Self::TextFill => "TEXT_FILL",
            Self::StrokeColor => "STROKE_COLOR",
            Self::EffectColor => "EFFECT_COLOR",
            Self::StrokeFloat => "STROKE_FLOAT",
            Self::EffectFloat => "EFFECT_FLOAT",
            Self::Opacity => "OPACITY",
            Self::FontFamily => "FONT_FAMILY",
            Self::FontStyle => "FONT_STYLE",
            Self::FontWeight => "FONT_WEIGHT",
            Self::FontSize => "FONT_SIZE",
            Self::LineHeight => "LINE_HEIGHT",
            Self::LetterSpacing => "LETTER_SPACING",
            Self::ParagraphSpacing => "PARAGRAPH_SPACING",
            Self::ParagraphIndent => "PARAGRAPH_INDENT",
            Self::Opaque(value) => value.as_ref(),
        }
    }
}

/// Exact Figma `VariableBindableNodeField` names.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignVariableBindableNodeField {
    Height,
    Width,
    Characters,
    ItemSpacing,
    PaddingLeft,
    PaddingRight,
    PaddingTop,
    PaddingBottom,
    Visible,
    CornerRadius,
    TopLeftRadius,
    TopRightRadius,
    BottomLeftRadius,
    BottomRightRadius,
    MinWidth,
    MaxWidth,
    MinHeight,
    MaxHeight,
    CounterAxisSpacing,
    StrokeWeight,
    StrokeTopWeight,
    StrokeRightWeight,
    StrokeBottomWeight,
    StrokeLeftWeight,
    Opacity,
    GridRowGap,
    GridColumnGap,
}

impl DesignVariableBindableNodeField {
    pub const fn api_name(self) -> &'static str {
        match self {
            Self::Height => "height",
            Self::Width => "width",
            Self::Characters => "characters",
            Self::ItemSpacing => "itemSpacing",
            Self::PaddingLeft => "paddingLeft",
            Self::PaddingRight => "paddingRight",
            Self::PaddingTop => "paddingTop",
            Self::PaddingBottom => "paddingBottom",
            Self::Visible => "visible",
            Self::CornerRadius => "cornerRadius",
            Self::TopLeftRadius => "topLeftRadius",
            Self::TopRightRadius => "topRightRadius",
            Self::BottomLeftRadius => "bottomLeftRadius",
            Self::BottomRightRadius => "bottomRightRadius",
            Self::MinWidth => "minWidth",
            Self::MaxWidth => "maxWidth",
            Self::MinHeight => "minHeight",
            Self::MaxHeight => "maxHeight",
            Self::CounterAxisSpacing => "counterAxisSpacing",
            Self::StrokeWeight => "strokeWeight",
            Self::StrokeTopWeight => "strokeTopWeight",
            Self::StrokeRightWeight => "strokeRightWeight",
            Self::StrokeBottomWeight => "strokeBottomWeight",
            Self::StrokeLeftWeight => "strokeLeftWeight",
            Self::Opacity => "opacity",
            Self::GridRowGap => "gridRowGap",
            Self::GridColumnGap => "gridColumnGap",
        }
    }
}

/// Exact Figma `VariableBindableTextField` names.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignVariableBindableTextField {
    FontFamily,
    FontSize,
    FontStyle,
    FontWeight,
    LetterSpacing,
    LineHeight,
    ParagraphSpacing,
    ParagraphIndent,
}

impl DesignVariableBindableTextField {
    pub const fn api_name(self) -> &'static str {
        match self {
            Self::FontFamily => "fontFamily",
            Self::FontSize => "fontSize",
            Self::FontStyle => "fontStyle",
            Self::FontWeight => "fontWeight",
            Self::LetterSpacing => "letterSpacing",
            Self::LineHeight => "lineHeight",
            Self::ParagraphSpacing => "paragraphSpacing",
            Self::ParagraphIndent => "paragraphIndent",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignVariableBindableField {
    Node(DesignVariableBindableNodeField),
    Text(DesignVariableBindableTextField),
}

impl DesignVariableBindableField {
    pub const fn api_name(self) -> &'static str {
        match self {
            Self::Node(field) => field.api_name(),
            Self::Text(field) => field.api_name(),
        }
    }
}

/// Exact property-to-API-field target carried by variable intents.
///
/// Independent-corner nodes intentionally carry four fields for the uniform
/// Corner radius property, matching how Figma records that binding in
/// `boundVariables`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignPropertyVariableTarget {
    pub property: DesignPanelProperty,
    pub fields: Vec<DesignVariableBindableField>,
    pub resolved_type: DesignVariableResolvedType,
    pub compatible_scope: Option<DesignVariableScope>,
    pub typography_target: Option<DesignTypographyTarget>,
}

impl DesignPropertyVariableTarget {
    pub(super) fn node(
        property: DesignPanelProperty,
        fields: impl IntoIterator<Item = DesignVariableBindableNodeField>,
        resolved_type: DesignVariableResolvedType,
        compatible_scope: Option<DesignVariableScope>,
    ) -> Self {
        Self {
            property,
            fields: fields
                .into_iter()
                .map(DesignVariableBindableField::Node)
                .collect(),
            resolved_type,
            compatible_scope,
            typography_target: None,
        }
    }

    pub(super) fn text(
        property: DesignPanelProperty,
        field: DesignVariableBindableTextField,
        resolved_type: DesignVariableResolvedType,
        compatible_scope: DesignVariableScope,
    ) -> Self {
        Self {
            property,
            fields: vec![DesignVariableBindableField::Text(field)],
            resolved_type,
            compatible_scope: Some(compatible_scope),
            typography_target: Some(DesignTypographyTarget::WholeLayer),
        }
    }

    pub const fn with_typography_target(mut self, target: DesignTypographyTarget) -> Self {
        if self.typography_target.is_some() {
            self.typography_target = Some(target);
        }
        self
    }
}

/// Whether a variable lives in this page or a published library.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum DesignVariableSource {
    Page {
        page_id: SharedString,
        page_name: SharedString,
    },
    Library {
        library_id: SharedString,
        library_name: SharedString,
    },
}

impl DesignVariableSource {
    pub fn page(page_id: impl Into<SharedString>, page_name: impl Into<SharedString>) -> Self {
        Self::Page {
            page_id: page_id.into(),
            page_name: page_name.into(),
        }
    }

    pub fn library(
        library_id: impl Into<SharedString>,
        library_name: impl Into<SharedString>,
    ) -> Self {
        Self::Library {
            library_id: library_id.into(),
            library_name: library_name.into(),
        }
    }

    pub const fn is_remote(&self) -> bool {
        matches!(self, Self::Library { .. })
    }

    pub const fn label(&self) -> &SharedString {
        match self {
            Self::Page { page_name, .. } => page_name,
            Self::Library { library_name, .. } => library_name,
        }
    }
}

/// Import state is separate from source so imported library variables retain
/// their remote identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignVariableImportState {
    Local,
    Imported,
    Available,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DesignVariableSearchMetadata {
    pub path: Vec<SharedString>,
    pub description: Option<SharedString>,
    pub keywords: Vec<SharedString>,
}

impl DesignVariableSearchMetadata {
    pub fn new(
        path: impl IntoIterator<Item = impl Into<SharedString>>,
        keywords: impl IntoIterator<Item = impl Into<SharedString>>,
    ) -> Self {
        Self {
            path: path.into_iter().map(Into::into).collect(),
            description: None,
            keywords: keywords.into_iter().map(Into::into).collect(),
        }
    }

    pub fn described(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// Host-resolved preview value for the active variable mode.
#[derive(Clone, Debug, PartialEq)]
pub enum DesignVariableResolvedValue {
    Boolean(bool),
    Color(DesignColor),
    Float(f32),
    String(SharedString),
}

impl DesignVariableResolvedValue {
    pub const fn resolved_type(&self) -> DesignVariableResolvedType {
        match self {
            Self::Boolean(_) => DesignVariableResolvedType::Boolean,
            Self::Color(_) => DesignVariableResolvedType::Color,
            Self::Float(_) => DesignVariableResolvedType::Float,
            Self::String(_) => DesignVariableResolvedType::String,
        }
    }

    pub fn panel_value(&self) -> DesignPanelValue {
        match self {
            Self::Boolean(value) => DesignPanelValue::Bool(*value),
            Self::Color(value) => DesignPanelValue::Color(*value),
            Self::Float(value) => DesignPanelValue::Number(*value),
            Self::String(value) => DesignPanelValue::Text(value.clone()),
        }
    }
}

/// One lossless host-supplied variable picker row.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignVariable {
    pub id: SharedString,
    pub name: SharedString,
    pub collection_id: SharedString,
    pub collection_name: SharedString,
    pub source: DesignVariableSource,
    pub resolved_type: DesignVariableResolvedType,
    pub scopes: Vec<DesignVariableScope>,
    pub import_state: DesignVariableImportState,
    pub resolved_value: Option<DesignVariableResolvedValue>,
    pub search: DesignVariableSearchMetadata,
    pub disabled_reason: Option<SharedString>,
}

impl DesignVariable {
    pub fn page(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        collection_id: impl Into<SharedString>,
        collection_name: impl Into<SharedString>,
        resolved_type: DesignVariableResolvedType,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            collection_id: collection_id.into(),
            collection_name: collection_name.into(),
            source: DesignVariableSource::page("current-page", "This page"),
            resolved_type,
            scopes: vec![DesignVariableScope::AllScopes],
            import_state: DesignVariableImportState::Local,
            resolved_value: None,
            search: DesignVariableSearchMetadata::default(),
            disabled_reason: None,
        }
    }

    pub fn with_source(mut self, source: DesignVariableSource) -> Self {
        self.source = source;
        self
    }

    pub fn with_scopes(mut self, scopes: impl IntoIterator<Item = DesignVariableScope>) -> Self {
        self.scopes = scopes.into_iter().collect();
        self
    }

    pub const fn with_import_state(mut self, import_state: DesignVariableImportState) -> Self {
        self.import_state = import_state;
        self
    }

    pub fn with_resolved_value(mut self, value: DesignVariableResolvedValue) -> Self {
        debug_assert_eq!(value.resolved_type(), self.resolved_type);
        self.resolved_value = Some(value);
        self
    }

    pub fn with_search(mut self, search: DesignVariableSearchMetadata) -> Self {
        self.search = search;
        self
    }

    pub fn disabled(mut self, reason: impl Into<SharedString>) -> Self {
        self.disabled_reason = Some(reason.into());
        self
    }

    pub fn is_compatible_with(&self, target: &DesignPropertyVariableTarget) -> bool {
        if self.resolved_type != target.resolved_type {
            return false;
        }
        let Some(scope) = &target.compatible_scope else {
            return true;
        };
        self.scopes.is_empty()
            || self.scopes.contains(&DesignVariableScope::AllScopes)
            || self.scopes.contains(scope)
    }

    pub fn matches_search(&self, query: &str) -> bool {
        let tokens = query
            .split_whitespace()
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>();
        if tokens.is_empty() {
            return true;
        }
        let mut haystack = format!(
            "{} {} {} {} {}",
            self.id,
            self.name,
            self.collection_id,
            self.collection_name,
            self.source.label()
        )
        .to_ascii_lowercase();
        for segment in &self.search.path {
            haystack.push(' ');
            haystack.push_str(&segment.to_ascii_lowercase());
        }
        if let Some(description) = &self.search.description {
            haystack.push(' ');
            haystack.push_str(&description.to_ascii_lowercase());
        }
        for keyword in &self.search.keywords {
            haystack.push(' ');
            haystack.push_str(&keyword.to_ascii_lowercase());
        }
        tokens.iter().all(|token| haystack.contains(token))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DesignVariableViewData {
    pub variables: Vec<DesignVariable>,
}

impl DesignVariableViewData {
    pub fn new(variables: impl IntoIterator<Item = DesignVariable>) -> Self {
        Self {
            variables: variables.into_iter().collect(),
        }
    }

    pub fn variable(&self, variable_id: &str) -> Option<&DesignVariable> {
        self.variables
            .iter()
            .find(|variable| variable.id.as_ref() == variable_id)
    }

    pub fn compatible<'a>(
        &'a self,
        target: &'a DesignPropertyVariableTarget,
        query: &'a str,
    ) -> impl Iterator<Item = &'a DesignVariable> + 'a {
        self.variables.iter().filter(move |variable| {
            variable.is_compatible_with(target) && variable.matches_search(query)
        })
    }
}

/// Host-filtered color variables available to solid paints and gradient stops.
///
/// The host supplies only candidates compatible with the active document
/// context. The panel additionally rejects non-Color values and never derives
/// a variable identity from a resolved color. Import and apply remain separate
/// operations because an available library variable cannot be bound yet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DesignPaintVariableViewData {
    pub variables: Vec<DesignVariable>,
}

impl DesignPaintVariableViewData {
    pub fn new(variables: impl IntoIterator<Item = DesignVariable>) -> Self {
        Self {
            variables: variables
                .into_iter()
                .filter(|variable| {
                    variable.resolved_type == DesignVariableResolvedType::Color
                        && matches!(
                            variable.resolved_value.as_ref(),
                            None | Some(DesignVariableResolvedValue::Color(_))
                        )
                })
                .collect(),
        }
    }

    pub fn variable(&self, variable_id: &str) -> Option<&DesignVariable> {
        self.variables.iter().find(|variable| {
            variable.id.as_ref() == variable_id
                && variable.resolved_type == DesignVariableResolvedType::Color
                && matches!(
                    variable.resolved_value.as_ref(),
                    None | Some(DesignVariableResolvedValue::Color(_))
                )
        })
    }

    pub fn variable_mut(&mut self, variable_id: &str) -> Option<&mut DesignVariable> {
        self.variables.iter_mut().find(|variable| {
            variable.id.as_ref() == variable_id
                && variable.resolved_type == DesignVariableResolvedType::Color
                && matches!(
                    variable.resolved_value.as_ref(),
                    None | Some(DesignVariableResolvedValue::Color(_))
                )
        })
    }
}

/// Whether the historic Variables shortcut is projected in the right
/// sidebar. Figma's canonical default exposes Variables from the navigation
/// bar, so the Page surface omits a variables row unless a host explicitly
/// requests the compatibility entry point.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum DesignVariablesEntryPoint {
    #[default]
    NavigationBarOnly,
    LegacyRightSidebar {
        disabled_reason: Option<SharedString>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignVariableMode {
    pub id: SharedString,
    pub name: SharedString,
    pub disabled_reason: Option<SharedString>,
}

impl DesignVariableMode {
    pub fn new(id: impl Into<SharedString>, name: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            disabled_reason: None,
        }
    }

    pub fn disabled(mut self, reason: impl Into<SharedString>) -> Self {
        self.disabled_reason = Some(reason.into());
        self
    }
}

/// One variable collection's default, resolved, and explicitly selected mode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignVariableModeCollection {
    pub id: SharedString,
    pub name: SharedString,
    pub source: DesignLocalResourceSource,
    pub modes: Vec<DesignVariableMode>,
    pub default_mode_id: SharedString,
    pub resolved_mode_id: SharedString,
    pub explicit_mode_id: Option<SharedString>,
    pub disabled_reason: Option<SharedString>,
}

impl DesignVariableModeCollection {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        source: DesignLocalResourceSource,
        modes: impl IntoIterator<Item = DesignVariableMode>,
        default_mode_id: impl Into<SharedString>,
        resolved_mode_id: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            source,
            modes: modes.into_iter().collect(),
            default_mode_id: default_mode_id.into(),
            resolved_mode_id: resolved_mode_id.into(),
            explicit_mode_id: None,
            disabled_reason: None,
        }
    }

    pub fn explicit(mut self, mode_id: impl Into<SharedString>) -> Self {
        let mode_id = mode_id.into();
        self.resolved_mode_id = mode_id.clone();
        self.explicit_mode_id = Some(mode_id);
        self
    }

    pub fn disabled(mut self, reason: impl Into<SharedString>) -> Self {
        self.disabled_reason = Some(reason.into());
        self
    }

    pub fn mode(&self, mode_id: &str) -> Option<&DesignVariableMode> {
        self.modes.iter().find(|mode| mode.id.as_ref() == mode_id)
    }

    pub fn resolved_mode(&self) -> Option<&DesignVariableMode> {
        self.mode(self.resolved_mode_id.as_ref())
    }

    pub fn explicit_mode(&self) -> Option<&DesignVariableMode> {
        self.explicit_mode_id
            .as_ref()
            .and_then(|mode_id| self.mode(mode_id.as_ref()))
    }

    pub fn mode_disabled_reason(&self, mode_id: &str) -> Option<SharedString> {
        if let Some(reason) = &self.disabled_reason {
            return Some(reason.clone());
        }
        self.mode(mode_id)
            .and_then(|mode| mode.disabled_reason.clone())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignVariableModeViewData {
    pub target: DesignPanelTarget,
    pub collections: Vec<DesignVariableModeCollection>,
}

impl DesignVariableModeViewData {
    pub fn new(
        target: DesignPanelTarget,
        collections: impl IntoIterator<Item = DesignVariableModeCollection>,
    ) -> Self {
        Self {
            target,
            collections: collections.into_iter().collect(),
        }
    }

    pub fn collection(&self, collection_id: &str) -> Option<&DesignVariableModeCollection> {
        self.collections
            .iter()
            .find(|collection| collection.id.as_ref() == collection_id)
    }
}
