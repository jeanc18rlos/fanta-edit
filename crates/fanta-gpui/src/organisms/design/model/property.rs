use gpui::SharedString;

use super::*;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignPanelProperty {
    Visible,
    X,
    Y,
    /// Horizontal Space between for an exact multiple-selection snapshot.
    SmartSelectionHorizontalSpacing,
    /// Vertical Space between for an exact multiple-selection snapshot.
    SmartSelectionVerticalSpacing,
    /// X coordinate for the exact host-selected vector vertices.
    VectorVertexX,
    /// Y coordinate for the exact host-selected vector vertices.
    VectorVertexY,
    /// Per-vertex radius for the exact host-selected vector vertices.
    VectorVertexCornerRadius,
    /// Tangent mirroring for the exact host-selected vector vertices.
    VectorHandleMirroring,
    Width,
    Height,
    Rotation,
    LockAspectRatio,
    HorizontalConstraint,
    VerticalConstraint,
    LayoutMode,
    HorizontalSizing,
    VerticalSizing,
    AutoLayoutAlignment,
    /// Compatibility-only per-axis auto-layout field.
    #[deprecated(note = "use DesignPanelProperty::AutoLayoutAlignment")]
    AlignmentX,
    /// Compatibility-only per-axis auto-layout field.
    #[deprecated(note = "use DesignPanelProperty::AutoLayoutAlignment")]
    AlignmentY,
    Wrap,
    Gap,
    ItemSpacingMode,
    CounterAxisAlignContent,
    CounterAxisGap,
    PaddingVertical,
    PaddingHorizontal,
    PaddingShorthand,
    PaddingTop,
    PaddingRight,
    PaddingBottom,
    PaddingLeft,
    ClipContent,
    IncludeStrokes,
    StackingOrder,
    BaselineAlignment,
    MinWidth,
    MaxWidth,
    MinHeight,
    MaxHeight,
    LayoutPositioning,
    LayoutAlignSelf,
    LayoutGrow,
    GridAutoTracks,
    GridItemsPositioning,
    GridColumnCount,
    GridRowCount,
    GridColumnTrack(usize),
    GridRowTrack(usize),
    GridColumnTrackValue(usize),
    GridRowTrackValue(usize),
    GridRowIndex,
    GridColumnIndex,
    GridRowSpan,
    GridColumnSpan,
    GridHorizontalAlignment,
    GridVerticalAlignment,
    Opacity,
    BlendMode,
    CornerRadius,
    CornerRadiusTopLeft,
    CornerRadiusTopRight,
    CornerRadiusBottomRight,
    CornerRadiusBottomLeft,
    IndependentCorners,
    CornerSmoothing,
    FillShowsInExports,
    TypographyStyle,
    FontFamily,
    FontStyle,
    FontWeight,
    FontSize,
    LineHeight,
    LetterSpacing,
    TextLeadingTrim,
    ParagraphSpacing,
    ParagraphIndent,
    ListSpacing,
    TextHangingPunctuation,
    TextHangingLists,
    HorizontalTextAlignment,
    VerticalTextAlignment,
    TextResize,
    TextTruncate,
    TextMaxLines,
    TextDecoration,
    TextDecorationStyle,
    TextDecorationOffset,
    TextDecorationThickness,
    TextDecorationColor,
    TextDecorationSkipInk,
    TextCase,
    TextList,
    TextPathStartSegment,
    TextPathStartPosition,
    TextPathOffset,
    ComponentProperty(usize),
    SlotStretchChildOnInsert(usize),
    SlotDisplayEmpty(usize),
    SlotMinimumInstances(usize),
    SlotMaximumInstances(usize),
    SlotPreferredValuesOnly(usize),
    /// Compatibility-only node media property.
    #[deprecated(note = "use PaintEditRequested with a typed Image/Video paint edit")]
    MediaCropMode,
    /// Compatibility-only node media property.
    #[deprecated(note = "use PaintEditRequested with DesignPaintProperty::MediaFilter")]
    MediaExposure,
    /// Compatibility-only node media property.
    #[deprecated(note = "use PaintEditRequested with DesignPaintProperty::MediaFilter")]
    MediaContrast,
    /// Compatibility-only node media property.
    #[deprecated(note = "use PaintEditRequested with DesignPaintProperty::MediaFilter")]
    MediaSaturation,
    /// Compatibility-only node media property.
    #[deprecated(note = "use PaintEditRequested with DesignPaintProperty::MediaFilter")]
    MediaTemperature,
    /// Compatibility-only node media property.
    #[deprecated(note = "use PaintEditRequested with DesignPaintProperty::MediaFilter")]
    MediaTint,
    /// Compatibility-only node media property.
    #[deprecated(note = "use PaintEditRequested with DesignPaintProperty::MediaFilter")]
    MediaHighlights,
    /// Compatibility-only node media property.
    #[deprecated(note = "use PaintEditRequested with DesignPaintProperty::MediaFilter")]
    MediaShadows,
    PolygonCount,
    StarPointCount,
    StarInnerRadius,
    ArcStartingAngle,
    ArcSweep,
    /// Compatibility-only absolute `ArcData.endingAngle` editor.
    ArcEndingAngle,
    ArcInnerRadius,
    BooleanOperation,
    IsMask,
    MaskType,
    TableRows,
    TableColumns,
    SectionContentsHidden,
    SectionDevStatus,
    TransformRepeatType(usize),
    TransformRepeatAxis(usize),
    TransformRepeatCount(usize),
    TransformRepeatUnit(usize),
    TransformRepeatOffset(usize),
    /// Canonical aggregate color row; not a Fill collection index.
    SelectionColor(usize),
    PaintOpacity {
        collection: DesignPanelCollection,
        index: usize,
    },
    PaintVisible {
        collection: DesignPanelCollection,
        index: usize,
    },
    StrokeWeight,
    StrokeWeightMode,
    StrokeWeightTop,
    StrokeWeightRight,
    StrokeWeightBottom,
    StrokeWeightLeft,
    StrokeAlign,
    StrokeStartCap,
    StrokeEndCap,
    StrokeEndpointCap,
    StrokeDashMode,
    StrokeDashPattern,
    StrokeDashCap,
    StrokeJoin,
    StrokeMiterAngle,
    StrokeVariableWidth,
    StrokeVariableWidthPointPosition(usize),
    StrokeVariableWidthPointWidth(usize),
    StrokeType,
    StrokeStretchBrush,
    StrokeBrushDirection,
    StrokeScatterBrush,
    StrokeScatterGap,
    StrokeScatterWiggle,
    StrokeScatterSizeJitter,
    StrokeScatterAngularJitter,
    StrokeScatterRotation,
    StrokeDynamicFrequency,
    StrokeDynamicWiggle,
    StrokeDynamicSmoothen,
    EffectKind(usize),
    EffectVisible(usize),
    /// Compatibility-only aggregate effect payload.
    #[deprecated(note = "use the typed Effect* leaf properties with EffectEditRequested")]
    EffectSettings(usize),
    EffectShadowColor(usize),
    EffectShadowBlendMode(usize),
    EffectShadowBlur(usize),
    EffectShadowSpread(usize),
    EffectShadowOffsetX(usize),
    EffectShadowOffsetY(usize),
    EffectDropShadowShowBehindNode(usize),
    EffectBlurType(usize),
    EffectBlurRadius(usize),
    EffectProgressiveBlurStartRadius(usize),
    EffectProgressiveBlurEndRadius(usize),
    EffectProgressiveBlurStartOffsetX(usize),
    EffectProgressiveBlurStartOffsetY(usize),
    EffectProgressiveBlurEndOffsetX(usize),
    EffectProgressiveBlurEndOffsetY(usize),
    EffectNoiseType(usize),
    EffectNoisePrimaryColor(usize),
    EffectNoiseSecondaryColor(usize),
    EffectNoiseOpacity(usize),
    EffectNoiseBlendMode(usize),
    EffectNoiseSizeX(usize),
    EffectNoiseSizeY(usize),
    EffectNoiseDensity(usize),
    EffectTextureSizeX(usize),
    EffectTextureSizeY(usize),
    EffectTextureRadius(usize),
    EffectTextureClipToShape(usize),
    EffectGlassLightIntensity(usize),
    EffectGlassLightAngle(usize),
    EffectGlassRefraction(usize),
    EffectGlassDepth(usize),
    EffectGlassDispersion(usize),
    EffectGlassFrost(usize),
    EffectGlassSplay(usize),
    /// One Shader property-definition by stable row identity plus a
    /// compatibility property index. The typed edit intent also carries the
    /// definition ID so hosts never have to trust this index after an echo.
    EffectShaderProperty(usize, usize),
    /// Compatibility alias used by the original compact effects row.
    #[deprecated(note = "use EffectShadowBlur or EffectBlurRadius")]
    EffectBlur(usize),
    /// Compatibility alias used by the original compact effects row.
    #[deprecated(note = "use EffectShadowSpread")]
    EffectSpread(usize),
    /// Compatibility alias used by the original compact effects row.
    #[deprecated(note = "use EffectShadowOffsetX")]
    EffectOffsetX(usize),
    /// Compatibility alias used by the original compact effects row.
    #[deprecated(note = "use EffectShadowOffsetY")]
    EffectOffsetY(usize),
    LayoutGridKind(usize),
    LayoutGridVisible(usize),
    LayoutGridAlignment(usize),
    LayoutGridCount(usize),
    LayoutGridSize(usize),
    LayoutGridOffset(usize),
    LayoutGridGutter(usize),
    LayoutGridMargin(usize),
    LayoutGridColor(usize),
    LayoutGridOpacity(usize),
    ExportSizing(usize),
    /// Compatibility alias for scale-only export rows.
    #[deprecated(note = "use DesignPanelProperty::ExportSizing")]
    ExportScale(usize),
    ExportSuffix(usize),
    ExportFormat(usize),
    /// Compatibility-only auto-layout alignment-cell property.
    #[deprecated(note = "use DesignPanelProperty::AutoLayoutAlignment")]
    AlignSelection,
    /// Compatibility-only selection distribution property.
    #[deprecated(note = "use DesignPanelAction::ArrangeRequested")]
    DistributeSelection,
}

/// Compatibility-only Design-panel adapter paths and their canonical
/// replacements.
///
/// These paths remain in the public enums so older hosts continue to compile,
/// but the reusable panel and Storybook do not emit or exercise them as
/// canonical behavior.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DesignPanelCompatibilityPath {
    AutoLayoutAxisAlignment,
    AggregateEffectSettings,
    AutoLayoutAlignmentCell,
    SelectionDistribution,
    NodeMediaProperty,
    CompactEffectLeaf,
    ScaleOnlyExportProperty,
    IndexedLayoutGridPropertyAction,
    IndexedLayoutGridRemoveAction,
    CountOnlyLayoutGridVariableAction,
    WholePaintAction,
    ConflatedColorStyleAction,
    NodeMediaReplaceAction,
    SingleExportAction,
}

impl DesignPanelCompatibilityPath {
    /// The canonical host contract replacing this compatibility path.
    pub const fn replacement(self) -> &'static str {
        match self {
            Self::AutoLayoutAxisAlignment | Self::AutoLayoutAlignmentCell => {
                "DesignPanelProperty::AutoLayoutAlignment"
            }
            Self::AggregateEffectSettings => {
                "typed Effect* leaf properties through DesignPanelAction::EffectEditRequested"
            }
            Self::SelectionDistribution => "DesignPanelAction::ArrangeRequested",
            Self::NodeMediaProperty => {
                "DesignPanelAction::PaintEditRequested with a typed Image/Video paint edit"
            }
            Self::CompactEffectLeaf => {
                "EffectShadow* or EffectBlurRadius through DesignPanelAction::EffectEditRequested"
            }
            Self::ScaleOnlyExportProperty => "DesignPanelProperty::ExportSizing",
            Self::IndexedLayoutGridPropertyAction => {
                "DesignPanelAction::LayoutGridPropertyEditRequested"
            }
            Self::IndexedLayoutGridRemoveAction => "DesignPanelAction::LayoutGridRemoveRequested",
            Self::CountOnlyLayoutGridVariableAction => {
                "DesignPanelAction::LayoutGridVariableApplyRequested or LayoutGridVariableDetachRequested"
            }
            Self::WholePaintAction => "DesignPanelAction::PaintEditRequested",
            Self::ConflatedColorStyleAction => {
                "PaintStyle*Requested for whole styles or PaintColorVariable*Requested for leaf variables"
            }
            Self::NodeMediaReplaceAction => "DesignPanelAction::PaintMediaSourceActionRequested",
            Self::SingleExportAction => "DesignPanelAction::ExportAllRequested",
        }
    }
}

#[allow(deprecated)]
impl DesignPanelProperty {
    /// Returns the compatibility classification for non-canonical properties.
    pub const fn compatibility_path(self) -> Option<DesignPanelCompatibilityPath> {
        match self {
            Self::AlignmentX | Self::AlignmentY => {
                Some(DesignPanelCompatibilityPath::AutoLayoutAxisAlignment)
            }
            Self::EffectSettings(_) => Some(DesignPanelCompatibilityPath::AggregateEffectSettings),
            Self::AlignSelection => Some(DesignPanelCompatibilityPath::AutoLayoutAlignmentCell),
            Self::DistributeSelection => Some(DesignPanelCompatibilityPath::SelectionDistribution),
            Self::MediaCropMode
            | Self::MediaExposure
            | Self::MediaContrast
            | Self::MediaSaturation
            | Self::MediaTemperature
            | Self::MediaTint
            | Self::MediaHighlights
            | Self::MediaShadows => Some(DesignPanelCompatibilityPath::NodeMediaProperty),
            Self::EffectBlur(_)
            | Self::EffectSpread(_)
            | Self::EffectOffsetX(_)
            | Self::EffectOffsetY(_) => Some(DesignPanelCompatibilityPath::CompactEffectLeaf),
            Self::ExportScale(_) => Some(DesignPanelCompatibilityPath::ScaleOnlyExportProperty),
            _ => None,
        }
    }

    /// Returns the compatibility index carried by an ordinary layout-guide
    /// property.
    pub const fn layout_grid_index(self) -> Option<usize> {
        match self {
            Self::LayoutGridKind(index)
            | Self::LayoutGridVisible(index)
            | Self::LayoutGridAlignment(index)
            | Self::LayoutGridCount(index)
            | Self::LayoutGridSize(index)
            | Self::LayoutGridOffset(index)
            | Self::LayoutGridGutter(index)
            | Self::LayoutGridMargin(index)
            | Self::LayoutGridColor(index)
            | Self::LayoutGridOpacity(index) => Some(index),
            _ => None,
        }
    }

    /// Rewrites only the compatibility index of an ordinary layout-guide
    /// property.
    pub const fn with_layout_grid_index(self, index: usize) -> Self {
        match self {
            Self::LayoutGridKind(_) => Self::LayoutGridKind(index),
            Self::LayoutGridVisible(_) => Self::LayoutGridVisible(index),
            Self::LayoutGridAlignment(_) => Self::LayoutGridAlignment(index),
            Self::LayoutGridCount(_) => Self::LayoutGridCount(index),
            Self::LayoutGridSize(_) => Self::LayoutGridSize(index),
            Self::LayoutGridOffset(_) => Self::LayoutGridOffset(index),
            Self::LayoutGridGutter(_) => Self::LayoutGridGutter(index),
            Self::LayoutGridMargin(_) => Self::LayoutGridMargin(index),
            Self::LayoutGridColor(_) => Self::LayoutGridColor(index),
            Self::LayoutGridOpacity(_) => Self::LayoutGridOpacity(index),
            property => property,
        }
    }

    pub const fn effect_index(self) -> Option<usize> {
        match self {
            Self::EffectKind(index)
            | Self::EffectVisible(index)
            | Self::EffectSettings(index)
            | Self::EffectShadowColor(index)
            | Self::EffectShadowBlendMode(index)
            | Self::EffectShadowBlur(index)
            | Self::EffectShadowSpread(index)
            | Self::EffectShadowOffsetX(index)
            | Self::EffectShadowOffsetY(index)
            | Self::EffectDropShadowShowBehindNode(index)
            | Self::EffectBlurType(index)
            | Self::EffectBlurRadius(index)
            | Self::EffectProgressiveBlurStartRadius(index)
            | Self::EffectProgressiveBlurEndRadius(index)
            | Self::EffectProgressiveBlurStartOffsetX(index)
            | Self::EffectProgressiveBlurStartOffsetY(index)
            | Self::EffectProgressiveBlurEndOffsetX(index)
            | Self::EffectProgressiveBlurEndOffsetY(index)
            | Self::EffectNoiseType(index)
            | Self::EffectNoisePrimaryColor(index)
            | Self::EffectNoiseSecondaryColor(index)
            | Self::EffectNoiseOpacity(index)
            | Self::EffectNoiseBlendMode(index)
            | Self::EffectNoiseSizeX(index)
            | Self::EffectNoiseSizeY(index)
            | Self::EffectNoiseDensity(index)
            | Self::EffectTextureSizeX(index)
            | Self::EffectTextureSizeY(index)
            | Self::EffectTextureRadius(index)
            | Self::EffectTextureClipToShape(index)
            | Self::EffectGlassLightIntensity(index)
            | Self::EffectGlassLightAngle(index)
            | Self::EffectGlassRefraction(index)
            | Self::EffectGlassDepth(index)
            | Self::EffectGlassDispersion(index)
            | Self::EffectGlassFrost(index)
            | Self::EffectGlassSplay(index)
            | Self::EffectBlur(index)
            | Self::EffectSpread(index)
            | Self::EffectOffsetX(index)
            | Self::EffectOffsetY(index)
            | Self::EffectShaderProperty(index, _) => Some(index),
            _ => None,
        }
    }

    pub const fn with_effect_index(self, index: usize) -> Self {
        match self {
            Self::EffectKind(_) => Self::EffectKind(index),
            Self::EffectVisible(_) => Self::EffectVisible(index),
            Self::EffectSettings(_) => Self::EffectSettings(index),
            Self::EffectShadowColor(_) => Self::EffectShadowColor(index),
            Self::EffectShadowBlendMode(_) => Self::EffectShadowBlendMode(index),
            Self::EffectShadowBlur(_) => Self::EffectShadowBlur(index),
            Self::EffectShadowSpread(_) => Self::EffectShadowSpread(index),
            Self::EffectShadowOffsetX(_) => Self::EffectShadowOffsetX(index),
            Self::EffectShadowOffsetY(_) => Self::EffectShadowOffsetY(index),
            Self::EffectDropShadowShowBehindNode(_) => Self::EffectDropShadowShowBehindNode(index),
            Self::EffectBlurType(_) => Self::EffectBlurType(index),
            Self::EffectBlurRadius(_) => Self::EffectBlurRadius(index),
            Self::EffectProgressiveBlurStartRadius(_) => {
                Self::EffectProgressiveBlurStartRadius(index)
            }
            Self::EffectProgressiveBlurEndRadius(_) => Self::EffectProgressiveBlurEndRadius(index),
            Self::EffectProgressiveBlurStartOffsetX(_) => {
                Self::EffectProgressiveBlurStartOffsetX(index)
            }
            Self::EffectProgressiveBlurStartOffsetY(_) => {
                Self::EffectProgressiveBlurStartOffsetY(index)
            }
            Self::EffectProgressiveBlurEndOffsetX(_) => {
                Self::EffectProgressiveBlurEndOffsetX(index)
            }
            Self::EffectProgressiveBlurEndOffsetY(_) => {
                Self::EffectProgressiveBlurEndOffsetY(index)
            }
            Self::EffectNoiseType(_) => Self::EffectNoiseType(index),
            Self::EffectNoisePrimaryColor(_) => Self::EffectNoisePrimaryColor(index),
            Self::EffectNoiseSecondaryColor(_) => Self::EffectNoiseSecondaryColor(index),
            Self::EffectNoiseOpacity(_) => Self::EffectNoiseOpacity(index),
            Self::EffectNoiseBlendMode(_) => Self::EffectNoiseBlendMode(index),
            Self::EffectNoiseSizeX(_) => Self::EffectNoiseSizeX(index),
            Self::EffectNoiseSizeY(_) => Self::EffectNoiseSizeY(index),
            Self::EffectNoiseDensity(_) => Self::EffectNoiseDensity(index),
            Self::EffectTextureSizeX(_) => Self::EffectTextureSizeX(index),
            Self::EffectTextureSizeY(_) => Self::EffectTextureSizeY(index),
            Self::EffectTextureRadius(_) => Self::EffectTextureRadius(index),
            Self::EffectTextureClipToShape(_) => Self::EffectTextureClipToShape(index),
            Self::EffectGlassLightIntensity(_) => Self::EffectGlassLightIntensity(index),
            Self::EffectGlassLightAngle(_) => Self::EffectGlassLightAngle(index),
            Self::EffectGlassRefraction(_) => Self::EffectGlassRefraction(index),
            Self::EffectGlassDepth(_) => Self::EffectGlassDepth(index),
            Self::EffectGlassDispersion(_) => Self::EffectGlassDispersion(index),
            Self::EffectGlassFrost(_) => Self::EffectGlassFrost(index),
            Self::EffectGlassSplay(_) => Self::EffectGlassSplay(index),
            Self::EffectShaderProperty(_, property_index) => {
                Self::EffectShaderProperty(index, property_index)
            }
            Self::EffectBlur(_) => Self::EffectBlur(index),
            Self::EffectSpread(_) => Self::EffectSpread(index),
            Self::EffectOffsetX(_) => Self::EffectOffsetX(index),
            Self::EffectOffsetY(_) => Self::EffectOffsetY(index),
            property => property,
        }
    }

    pub const fn is_typography(self) -> bool {
        matches!(
            self,
            Self::TypographyStyle
                | Self::FontFamily
                | Self::FontStyle
                | Self::FontWeight
                | Self::FontSize
                | Self::LineHeight
                | Self::LetterSpacing
                | Self::TextLeadingTrim
                | Self::ParagraphSpacing
                | Self::ParagraphIndent
                | Self::ListSpacing
                | Self::TextHangingPunctuation
                | Self::TextHangingLists
                | Self::HorizontalTextAlignment
                | Self::VerticalTextAlignment
                | Self::TextResize
                | Self::TextTruncate
                | Self::TextMaxLines
                | Self::TextDecoration
                | Self::TextDecorationStyle
                | Self::TextDecorationOffset
                | Self::TextDecorationThickness
                | Self::TextDecorationColor
                | Self::TextDecorationSkipInk
                | Self::TextCase
                | Self::TextList
        )
    }

    /// Whether a typography property can target the active character range.
    ///
    /// Resizing, truncation, maximum lines, and vertical alignment describe
    /// the text layer's container and must remain whole-layer operations even
    /// while the host is editing characters.
    pub const fn supports_selected_text_range(self) -> bool {
        self.is_typography()
            && !matches!(
                self,
                Self::VerticalTextAlignment
                    | Self::TextResize
                    | Self::TextTruncate
                    | Self::TextMaxLines
            )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum DesignPanelValue {
    Number(f32),
    /// A canonical radian value edited and displayed as degrees.
    AngleRadians(f32),
    /// A canonical `0..=1` ratio edited and displayed as a percentage.
    Ratio(f32),
    OptionalNumber(Option<f32>),
    Integer(i64),
    Bool(bool),
    Text(SharedString),
    TypographyStyle(Option<DesignTypographyStyleSelection>),
    LayoutMode(DesignLayoutMode),
    SizingMode(DesignSizingMode),
    AutoLayoutAlignment(DesignAutoLayoutAlignment),
    ItemSpacingMode(DesignItemSpacingMode),
    CounterAxisAlignContent(DesignCounterAxisAlignContent),
    LayoutPositioning(DesignLayoutPositioning),
    LayoutAlignSelf(DesignLayoutAlignSelf),
    StackingOrder(DesignStackingOrder),
    BaselineAlignment(DesignBaselineAlignment),
    GridAutoTracks(DesignGridAutoTracks),
    GridItemsPositioning(DesignGridItemsPositioning),
    GridTrack(DesignGridTrack),
    GridItemAlignment(DesignGridItemAlignment),
    Constraint(DesignConstraint),
    BooleanOperation(DesignBooleanOperation),
    MaskType(DesignMaskType),
    SectionDevStatus(Option<DesignSectionDevStatusKind>),
    RepeatType(DesignRepeatType),
    RepeatAxis(DesignRepeatAxis),
    TransformUnit(DesignTransformUnit),
    HandleMirroring(DesignHandleMirroring),
    BlendMode(DesignBlendMode),
    TextResize(DesignTextResize),
    LineHeight(DesignLineHeight),
    LetterSpacing(DesignLetterSpacing),
    TextHorizontalAlignment(DesignTextHorizontalAlignment),
    TextVerticalAlignment(DesignTextVerticalAlignment),
    TextLeadingTrim(DesignTextLeadingTrim),
    TextDecoration(DesignTextDecoration),
    TextDecorationStyle(DesignTextDecorationStyle),
    TextDecorationMetric(DesignTextDecorationMetric),
    TextDecorationColor(DesignTextDecorationColor),
    TextCase(DesignTextCase),
    TextList(DesignTextList),
    StrokeAlign(DesignStrokeAlign),
    StrokeCap(DesignStrokeCap),
    StrokeWeightMode(DesignStrokeWeightMode),
    StrokeJoin(DesignStrokeJoin),
    StrokeDashMode(DesignStrokeDashMode),
    StrokeVariableWidth(Option<DesignVariableWidthStroke>),
    StrokeType(DesignStrokeType),
    StrokeStretchBrush(DesignStretchBrushName),
    StrokeBrushDirection(DesignStrokeBrushDirection),
    StrokeScatterBrush(DesignScatterBrushName),
    EffectKind(DesignEffectKind),
    EffectSettings(DesignEffectSettings),
    EffectBlurType(DesignBlurType),
    EffectNoiseType(DesignNoiseType),
    ShaderProperty(DesignShaderPropertyValue),
    Color(DesignColor),
    GridKind(DesignGridKind),
    LayoutGridCount(DesignLayoutGridCount),
    ColumnGridAlignment(DesignColumnGridAlignment),
    RowGridAlignment(DesignRowGridAlignment),
    ExportSizing(DesignExportSizing),
    ExportFormat(DesignExportFormat),
    NumberList(Vec<f32>),
}

/// Lifecycle phase for a continuous inspector edit.
///
/// Hosts can use these phases to group keyboard stepping, typing, and pointer
/// scrubbing into one undoable document operation while the panel keeps only a
/// transient draft. A host should snapshot the controlled value at `Begin` and
/// restore that snapshot at `Cancel`; cancellation does not require the
/// accompanying candidate value to be the original.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesignPanelEditPhase {
    Begin,
    Preview,
    Commit,
    Cancel,
}
