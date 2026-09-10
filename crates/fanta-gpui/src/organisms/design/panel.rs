#![allow(deprecated)]

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use gpui::{
    Anchor, AnyElement, App, AppContext as _, ClickEvent, Context, Entity, EventEmitter,
    ExternalPaths, FocusHandle, Focusable, InteractiveElement as _, IntoElement, KeyDownEvent,
    Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _,
    Render, ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription,
    Window, div, linear_color_stop, linear_gradient, pattern_slash, prelude::FluentBuilder as _,
    px, relative, rgba,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, IndexPath, Selectable as _, Sizable as _,
    StyledExt as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    dialog::Dialog,
    h_flex,
    input::{Input, InputEvent, InputState},
    popover::Popover,
    scroll::{ScrollableElement as _, Scrollbar, ScrollbarAxis},
    select::{Select, SelectEvent, SelectItem, SelectState},
    slider::{Slider, SliderEvent, SliderState, SliderValue},
    switch::Switch,
    tooltip::Tooltip,
    v_flex,
};

use crate::atoms::vector_icon::{render_icon_canvas, render_wide_icon_canvas};
use crate::atoms::{ActivateControl, ButtonControlExt as _, CONTROL_KEY_CONTEXT, ControlExt as _};
use crate::color::{parse_hex_rgba, rgba_channels};
use crate::molecules::{popup_height, popup_width};

use super::field_value::{
    ArrowStep, NumericClamp, evaluate_numeric_expression, parse_decorated_number, round_to_integer,
    step_with_arrow_amount,
};
use super::paint_picker::{
    PaintPicker, PaintPickerEvent, PaintPickerTarget as PickerEventTarget, media_drop_from_paths,
};
use super::typography_style_picker::{TypographyStylePicker, TypographyStylePickerEvent};
use super::{
    CancelDesignInteraction, DESIGN_PANEL_KEY_CONTEXT, DesignAddAutoLayoutViewData,
    DesignAnimatedExportChange, DesignAnimatedExportSettings,
    DesignAppliedComponentPropertyControl, DesignArrangeOperation, DesignAutoLayoutAlignment,
    DesignBaselineAlignment, DesignBlendMode, DesignBlurEffect, DesignBlurType,
    DesignBooleanOperation, DesignColor, DesignColorContrastPaintTarget,
    DesignColorContrastPaintViewData, DesignColorContrastViewData, DesignColorStyleSampleViewData,
    DesignColorStyleViewData, DesignColumnGridAlignment, DesignComplexStroke,
    DesignComponentAuthoringViewData, DesignComponentProperty,
    DesignComponentPropertyApplicationSurface, DesignComponentPropertyDefinition,
    DesignComponentPropertyKind, DesignComponentPropertyOrigin,
    DesignComponentPropertyOverrideState, DesignComponentPropertyPartition,
    DesignComponentPropertyValue, DesignComponentPropertyVariableTarget, DesignComponentRole,
    DesignComponentSource, DesignComponentSwapCandidate, DesignComponentSwapSelection,
    DesignComponentSwapViewData, DesignConstraint, DesignCounterAxisAlignContent,
    DesignDrawAppearanceViewData, DesignDrawSliderRange, DesignEffect, DesignEffectKind,
    DesignEffectSettings, DesignEffectStyleSelection, DesignEffectStyleViewData,
    DesignEffectVariableField, DesignEffectVariableViewData, DesignExportColorProfile,
    DesignExportConfiguration, DesignExportConfigurationChange, DesignExportFormat,
    DesignExportFormatSettings, DesignExportImageQuality, DesignExportImageResampling,
    DesignExportMode, DesignExportPreviewState, DesignExportSizing, DesignExportViewData,
    DesignFontAvailability, DesignFontAxis, DesignFontCatalogState, DesignFontSelection,
    DesignFontViewData, DesignFramePresetSelection, DesignFramePresetViewData, DesignGifExportFps,
    DesignGridAutoTracks, DesignGridDimensions, DesignGridItemAlignment,
    DesignGridItemsPositioning, DesignGridKind, DesignGridTrack, DesignGridTrackAxis,
    DesignGridTrackSizing, DesignHandleMirroring, DesignItemSpacingMode, DesignLayout,
    DesignLayoutAlignSelf, DesignLayoutDimensionAxis, DesignLayoutGridCount,
    DesignLayoutGridCountVariableViewData, DesignLayoutGridSettings,
    DesignLayoutGridStyleImportState, DesignLayoutGridStyleSelection, DesignLayoutGridStyleSource,
    DesignLayoutGridStyleViewData, DesignLayoutGridVariableTarget, DesignLayoutGridVariableValue,
    DesignLayoutGridVariableViewData, DesignLayoutMode, DesignLayoutPositioning,
    DesignLetterSpacing, DesignLineHeight, DesignLocalResourceAvailability,
    DesignLocalResourceCategory, DesignLocalResourceKind, DesignLocalResourceSelection,
    DesignLocalResourceSource, DesignLocalStyleCommand, DesignLocalStyleEntry,
    DesignLocalStyleInsertion, DesignLocalStyleKind, DesignLocalStylePreview,
    DesignLocalStyleTarget, DesignMaskType, DesignMediaCropAction, DesignMediaDroppedFile,
    DesignMediaKind, DesignMediaPaintCapabilities, DesignMediaPaintViewData,
    DesignMediaSourceAction, DesignMenuPreview, DesignMenuPreviewPhase, DesignNoiseColors,
    DesignNoiseType, DesignNudgeSettings, DesignOpenTypeFeatureTag, DesignPageLocalStylesViewData,
    DesignPageViewData, DesignPaint, DesignPaintColorTarget, DesignPaintEdit, DesignPaintPayload,
    DesignPaintProperty, DesignPaintStyleImportState, DesignPaintStyleSelection,
    DesignPaintStyleViewData, DesignPaintTarget, DesignPaintValue, DesignPaintVariableViewData,
    DesignPanelAction, DesignPanelAutoLayoutDirection, DesignPanelAutoLayoutParticipation,
    DesignPanelBindingKind, DesignPanelCollection, DesignPanelEditMode, DesignPanelEditPhase,
    DesignPanelInspectionContext, DesignPanelMultipleSelection, DesignPanelNode,
    DesignPanelNodeKind, DesignPanelParentLayout, DesignPanelPermissions, DesignPanelProperty,
    DesignPanelPropertyValueState, DesignPanelSection, DesignPanelSelectionKind,
    DesignPanelSurface, DesignPanelTarget, DesignPanelValue, DesignPanelWorkspaceMode,
    DesignPropertyVariableTarget, DesignRepeatAxis, DesignRepeatMode, DesignRepeatType,
    DesignRowGridAlignment, DesignScatterBrushName, DesignScrubSpeed, DesignSectionDevStatusKind,
    DesignSelectionHeaderCommand, DesignSelectionHeaderCommandAccess, DesignSelectionHeaderControl,
    DesignSelectionHeaderControlIcon, DesignSelectionHeaderControlKind,
    DesignSelectionHeaderViewData, DesignShaderPropertyEditorKind,
    DesignShaderPropertyEditorTarget, DesignShaderPropertyKind, DesignShaderPropertyValue,
    DesignShaderViewData, DesignShapeGeometry, DesignSizingMode, DesignSlotSettingsChange,
    DesignSlotViolation, DesignSmartSelectionAvailability, DesignSmartSelectionAxis,
    DesignSmartSelectionOperation, DesignSmartSelectionSpacingValue, DesignSmartSelectionViewData,
    DesignStackingOrder, DesignStretchBrushName, DesignStrokeAlign, DesignStrokeBrushDirection,
    DesignStrokeCap, DesignStrokeDashMode, DesignStrokeEndpointControl, DesignStrokeJoin,
    DesignStrokeType, DesignStrokeWeightMode, DesignTextCase, DesignTextDecoration,
    DesignTextDecorationColor, DesignTextDecorationMetric, DesignTextDecorationStyle,
    DesignTextHorizontalAlignment, DesignTextLeadingTrim, DesignTextList,
    DesignTextPathOrientation, DesignTextPathStartData, DesignTextResize,
    DesignTextVerticalAlignment, DesignTransformModifierChange, DesignTransformOperation,
    DesignTransformUnit, DesignTypographyStyleBinding, DesignTypographyStyleViewData,
    DesignTypographyTarget, DesignVariable, DesignVariableImportState, DesignVariableModeViewData,
    DesignVariableScope, DesignVariableSource, DesignVariableViewData, DesignVariableWidthPoint,
    DesignVariableWidthPreset, DesignVariableWidthStroke, DesignVariablesEntryPoint,
    DesignVectorCoordinateAxis, DesignVectorEditViewData, DesignVectorSelectionValue,
    DesignVideoExportFps, DesignViewerColorRepresentation, DesignViewerPropertiesViewData,
    design_panel_section_is_visible_in_workspace, resolve_design_panel_sections_with_export,
};

mod alignment_grid;
mod appearance;
mod browsers;
mod component_props;
mod drag_preview;
mod effects;
mod export;
mod header;
mod layout;
mod layout_grids;
mod options;
mod page;
mod paints;
mod position;
mod properties;
mod scrub;
mod shader_helpers;
mod shape;
mod typography;
mod viewer;

use alignment_grid::*;
use scrub::*;
use shader_helpers::*;

const HEADER_HEIGHT: f32 = 40.;
const ROW_HEIGHT: f32 = 24.;
const PANEL_PADDING: f32 = 16.;
const NUMERIC_SCRUB_THRESHOLD: f32 = 2.;
const GRID_DIMENSIONS_POPOVER_WIDTH: f32 = 248.;

fn component_authoring_orders_have_same_unique_members(
    left: &[SharedString],
    right: &[SharedString],
) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let left_ids = left
        .iter()
        .map(SharedString::as_ref)
        .collect::<HashSet<_>>();
    let right_ids = right
        .iter()
        .map(SharedString::as_ref)
        .collect::<HashSet<_>>();
    left_ids.len() == left.len() && right_ids.len() == right.len() && left_ids == right_ids
}

#[derive(Clone, Debug, Eq)]
struct PaintPickerTarget {
    collection: DesignPanelCollection,
    index: usize,
    paint_id: SharedString,
}

impl PartialEq for PaintPickerTarget {
    fn eq(&self, other: &Self) -> bool {
        self.collection == other.collection
            && if self.paint_id.is_empty() || other.paint_id.is_empty() {
                self.index == other.index
            } else {
                self.paint_id == other.paint_id
            }
    }
}

#[derive(Clone, Debug, Eq)]
struct EffectSettingsTarget {
    index: usize,
    effect_id: SharedString,
}

impl EffectSettingsTarget {
    fn matches(&self, effect: &DesignEffect, index: usize) -> bool {
        if self.effect_id.is_empty() || effect.id.is_empty() {
            self.index == index
        } else {
            self.effect_id == effect.id
        }
    }
}

impl PartialEq for EffectSettingsTarget {
    fn eq(&self, other: &Self) -> bool {
        if self.effect_id.is_empty() || other.effect_id.is_empty() {
            self.index == other.index
        } else {
            self.effect_id == other.effect_id
        }
    }
}

#[derive(Clone, Debug, Eq)]
struct LayoutGridTarget {
    guide_id: SharedString,
    index: usize,
}

impl PartialEq for LayoutGridTarget {
    fn eq(&self, other: &Self) -> bool {
        if self.guide_id.is_empty() || other.guide_id.is_empty() {
            self.index == other.index
        } else {
            self.guide_id == other.guide_id
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum AuxiliaryColorPickerTarget {
    PageBackground {
        page_id: SharedString,
    },
    TextDecoration {
        node_id: SharedString,
        typography_target: DesignTypographyTarget,
    },
    Effect {
        node_id: SharedString,
        effect_id: SharedString,
        index: usize,
        property: DesignPanelProperty,
    },
    LayoutGrid {
        node_id: SharedString,
        guide_id: SharedString,
        index: usize,
    },
    SelectionColor {
        target: DesignPanelTarget,
        selection_color_id: SharedString,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SelectionColorResourceKind {
    PaintStyle,
    ColorVariable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SelectionColorResourceTarget {
    selection_color_id: SharedString,
    kind: SelectionColorResourceKind,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
enum StyleBrowserSourceFilter {
    #[default]
    All,
    ThisPage,
    Libraries,
    Library {
        library_id: SharedString,
        library_name: SharedString,
    },
}

impl StyleBrowserSourceFilter {
    fn general() -> [Self; 3] {
        [Self::All, Self::ThisPage, Self::Libraries]
    }

    fn library(library_id: impl Into<SharedString>, library_name: impl Into<SharedString>) -> Self {
        Self::Library {
            library_id: library_id.into(),
            library_name: library_name.into(),
        }
    }

    fn label(&self) -> SharedString {
        match self {
            Self::All => "All".into(),
            Self::ThisPage => "This page".into(),
            Self::Libraries => "Libraries".into(),
            Self::Library { library_name, .. } => library_name.clone(),
        }
    }

    const fn includes_page(&self) -> bool {
        matches!(self, Self::All | Self::ThisPage)
    }

    fn includes_library(&self, library_id: &str) -> bool {
        match self {
            Self::All | Self::Libraries => true,
            Self::Library {
                library_id: selected,
                ..
            } => selected.as_ref() == library_id,
            Self::ThisPage => false,
        }
    }

    fn normalized_for_libraries<'a>(
        &self,
        libraries: impl IntoIterator<Item = (&'a SharedString, &'a SharedString)>,
    ) -> Self {
        let Self::Library {
            library_id: selected,
            ..
        } = self
        else {
            return self.clone();
        };
        libraries
            .into_iter()
            .find(|(library_id, _)| *library_id == selected)
            .map(|(library_id, library_name)| {
                Self::library(library_id.clone(), library_name.clone())
            })
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum StyleBrowserViewMode {
    #[default]
    List,
    Grid,
}

impl StyleBrowserViewMode {
    const ALL: [Self; 2] = [Self::List, Self::Grid];

    const fn label(self) -> &'static str {
        match self {
            Self::List => "List view",
            Self::Grid => "Grid view",
        }
    }

    const fn icon(self) -> IconName {
        match self {
            Self::List => IconName::Menu,
            Self::Grid => IconName::LayoutDashboard,
        }
    }
}

impl AuxiliaryColorPickerTarget {
    const PICKER_NODE_ID: &'static str = "__design-color-picker";

    fn picker_paint_id(&self) -> SharedString {
        match self {
            Self::PageBackground { page_id } => format!("page-background:{page_id}").into(),
            Self::TextDecoration {
                node_id,
                typography_target,
            } => format!("text-decoration:{node_id}:{typography_target:?}").into(),
            Self::Effect {
                node_id,
                effect_id,
                index,
                property,
            } => format!("effect:{node_id}:{effect_id}:{index}:{property:?}").into(),
            Self::LayoutGrid {
                node_id,
                guide_id,
                index,
            } => format!("layout-grid:{node_id}:{guide_id}:{index}").into(),
            Self::SelectionColor {
                target,
                selection_color_id,
                ..
            } => format!("selection-color:{target:?}:{selection_color_id}").into(),
        }
    }

    const fn title(&self) -> &'static str {
        match self {
            Self::PageBackground { .. } => "Page background",
            Self::TextDecoration { .. } => "Decoration color",
            Self::Effect { .. } => "Effect color",
            Self::LayoutGrid { .. } => "Layout guide color",
            Self::SelectionColor { .. } => "Selection color",
        }
    }

    fn matches_picker_target(&self, target: &PickerEventTarget) -> bool {
        target.node_id.as_ref() == Self::PICKER_NODE_ID
            && target.collection == DesignPanelCollection::Fill
            && target.index == 0
            && target.paint_id == self.picker_paint_id()
    }

    fn selection_color_id(&self) -> Option<&SharedString> {
        match self {
            Self::SelectionColor {
                selection_color_id, ..
            } => Some(selection_color_id),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
struct ActivePaintEdit {
    target: PickerEventTarget,
    edit: DesignPaintEdit,
}

#[derive(Clone, Debug)]
struct EffectDrag {
    effect_id: SharedString,
    from_index: usize,
    label: SharedString,
}

struct EffectDragPreview {
    drag: EffectDrag,
}

#[derive(Clone, Debug)]
struct PaintDrag {
    collection: DesignPanelCollection,
    from_index: usize,
    label: SharedString,
    paint: DesignPaint,
}

struct PaintDragPreview {
    drag: PaintDrag,
}

#[derive(Clone, Debug)]
struct LocalStyleDrag {
    target: DesignLocalStyleTarget,
    label: SharedString,
}

struct LocalStyleDragPreview {
    drag: LocalStyleDrag,
}

#[derive(Clone, Debug)]
struct ComponentPropertyDefinitionDrag {
    node_id: SharedString,
    property_id: SharedString,
    property_name: SharedString,
    partition: DesignComponentPropertyPartition,
    original_order: Vec<SharedString>,
}

struct ComponentPropertyDefinitionDragPreview {
    drag: ComponentPropertyDefinitionDrag,
}

#[derive(Clone, Debug)]
struct ComponentVariantOptionDrag {
    node_id: SharedString,
    property_id: SharedString,
    option_id: SharedString,
    option_name: SharedString,
    original_order: Vec<SharedString>,
}

struct ComponentVariantOptionDragPreview {
    drag: ComponentVariantOptionDrag,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TypographySettingsTab {
    Basics,
    Details,
    Variable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PaddingEditorMode {
    Axes,
    Individual,
    Shorthand,
}

impl PaddingEditorMode {
    fn for_node(node: &DesignPanelNode) -> Self {
        let Some(layout) = &node.layout else {
            return Self::Axes;
        };
        if layout.padding[0] != layout.padding[2] || layout.padding[1] != layout.padding[3] {
            Self::Individual
        } else {
            Self::Axes
        }
    }
}

impl TypographySettingsTab {
    const fn label(self) -> &'static str {
        match self {
            Self::Basics => "Basics",
            Self::Details => "Details",
            Self::Variable => "Variable",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TypographyAlignmentIcon {
    Horizontal(DesignTextHorizontalAlignment),
    Vertical(DesignTextVerticalAlignment),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TextResizeIcon {
    AutoWidth,
    AutoHeight,
    FixedSize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PositionActionIcon {
    AlignLeft,
    AlignHorizontalCenter,
    AlignRight,
    AlignTop,
    AlignVerticalCenter,
    AlignBottom,
    RotateClockwise90,
    FlipHorizontal,
    FlipVertical,
    LockAspectRatio,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ValueFieldIcon {
    Opacity,
    Corners,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VisibilityControlState {
    Visible,
    Hidden,
    Mixed,
}

impl VisibilityControlState {
    const fn from_visible(visible: bool) -> Self {
        if visible { Self::Visible } else { Self::Hidden }
    }

    const fn activation_value(self) -> bool {
        match self {
            Self::Visible => false,
            Self::Hidden | Self::Mixed => true,
        }
    }

    const fn tooltip(self, editable: bool) -> &'static str {
        match (self, editable) {
            (Self::Visible, true) => "Visible · Hide",
            (Self::Hidden, true) => "Hidden · Show",
            (Self::Mixed, true) => "Mixed visibility · Show all",
            (Self::Visible, false) => "Visible · Read only",
            (Self::Hidden, false) => "Hidden · Read only",
            (Self::Mixed, false) => "Mixed visibility · Read only",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SelectionHeaderOverlay {
    Title,
    Control(SharedString),
    More,
}

#[derive(Clone, Debug, PartialEq)]
struct PropertyOption {
    label: SharedString,
    value: DesignPanelValue,
}

impl SelectItem for PropertyOption {
    type Value = DesignPanelValue;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

type PropertySelectState = SelectState<Vec<PropertyOption>>;

#[derive(Clone, Debug, PartialEq)]
struct PropertyOptionSnapshot {
    options: Vec<PropertyOption>,
    selected: Option<DesignPanelValue>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum PropertyEditorKind {
    Number {
        integer: bool,
        clamp: Option<NumericClamp>,
    },
    OptionalNumber {
        clamp: Option<NumericClamp>,
    },
    AngleDegrees,
    PercentageRatio,
    LayoutGridCount,
    Color,
    Text,
    NumberList,
    ExportSizing,
    Shader {
        field: ShaderPropertyEditorField,
        input: ShaderPropertyEditorInput,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShaderPropertyEditorField {
    Text,
    Number,
    Color,
    PointX,
    PointY,
    LineStartX,
    LineStartY,
    LineEndX,
    LineEndY,
    CircleCenterX,
    CircleCenterY,
    CircleRadius,
    CirclePointCenterX,
    CirclePointCenterY,
    CirclePointRadius,
    CirclePointAngle,
    ColorPointX,
    ColorPointY,
    ColorPointColor,
    GradientStopPosition(usize),
    GradientStopColor(usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ShaderPropertyEditorInput {
    Text,
    Number { clamp: Option<NumericClamp> },
    Color,
}

#[derive(Clone, Debug, PartialEq)]
struct PropertyEditor {
    property: DesignPanelProperty,
    /// Stable target captured before Begin for an ordinary layout-guide edit.
    layout_grid_target: Option<LayoutGridTarget>,
    /// Stable host export-row identity captured before Begin.
    export_configuration_id: Option<SharedString>,
    original: DesignPanelValue,
    last_preview: Option<DesignPanelValue>,
    base: f64,
    kind: PropertyEditorKind,
}

#[derive(Clone, Debug, PartialEq)]
struct NumericPropertyScrub {
    property: DesignPanelProperty,
    original: DesignPanelValue,
    kind: PropertyEditorKind,
    scalar: f64,
    origin_x: f32,
    origin_y: f32,
    last_x: f32,
    last_y: f32,
    active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum EditorFocusOrigin {
    ValueCell(DesignPanelProperty),
    ShaderField {
        property: DesignPanelProperty,
        field: ShaderPropertyEditorField,
    },
    TypeSetting(DesignPanelProperty),
    ComponentMultiline(SharedString),
}

#[derive(Clone, Debug, PartialEq)]
struct EditorFocusReturn {
    origin: EditorFocusOrigin,
    handle: FocusHandle,
}

#[derive(Clone, Debug, PartialEq)]
struct VariableFontAxisEditor {
    /// Stable OpenType tag; authoritative across host row reorderings.
    tag: SharedString,
    /// Exact layer/range identity captured when Begin was emitted.
    target: DesignTypographyTarget,
    original: f32,
    last_preview: Option<f32>,
    min: f32,
    max: f32,
    default: f32,
    step: f32,
}

#[derive(Clone, Debug, PartialEq)]
struct VariableFontAxisScrub {
    tag: SharedString,
    original: f32,
    scalar: f32,
    origin_x: f32,
    origin_y: f32,
    last_x: f32,
    last_y: f32,
    active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GridDimensionsPicker {
    /// Exact host node captured when the retained picker opened.
    node_id: SharedString,
    candidate: DesignGridDimensions,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GridDimensionsEdit {
    /// Exact host node captured before Begin.
    node_id: SharedString,
    original: DesignGridDimensions,
}

#[derive(Clone, Debug, PartialEq)]
struct DimensionLimitsPreview {
    node_id: SharedString,
    axis: DesignLayoutDimensionAxis,
    minimum: Option<f32>,
    maximum: Option<f32>,
}

#[derive(IntoElement)]
struct DimensionMenuTrigger {
    base: gpui::Stateful<gpui::Div>,
    selected: bool,
}

impl gpui_component::Selectable for DimensionMenuTrigger {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl gpui::RenderOnce for DimensionMenuTrigger {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        self.base.when(self.selected, |trigger| {
            trigger
                .bg(cx.theme().accent)
                .border_1()
                .border_color(cx.theme().selection)
                .text_color(cx.theme().selection)
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ComponentMultilineEditor {
    property_id: SharedString,
    original: DesignComponentPropertyValue,
    last_preview: Option<DesignComponentPropertyValue>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ComponentPropertyCreateDraft {
    kind: DesignComponentPropertyKind,
    description: Option<SharedString>,
    documentation_links: Vec<super::DesignDocumentationLink>,
    definition: DesignComponentPropertyDefinition,
    default_variable_id: Option<SharedString>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ComponentPropertyEditDraft {
    property_id: SharedString,
    expected_description: Option<SharedString>,
    description: Option<SharedString>,
    expected_documentation_links: Vec<super::DesignDocumentationLink>,
    documentation_links: Vec<super::DesignDocumentationLink>,
    expected_definition: DesignComponentPropertyDefinition,
    definition: DesignComponentPropertyDefinition,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ComponentAuthoringNameEditor {
    Property {
        property_id: SharedString,
        original_name: SharedString,
        last_preview: Option<SharedString>,
    },
    VariantOption {
        property_id: SharedString,
        option_id: SharedString,
        original_name: SharedString,
        last_preview: Option<SharedString>,
    },
    NewVariantOption {
        property_id: SharedString,
        after_option_id: Option<SharedString>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ComponentPropertyReorderSession {
    property_id: SharedString,
    partition: DesignComponentPropertyPartition,
    original_order: Vec<SharedString>,
    last_before_property_id: Option<SharedString>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ComponentVariantOptionReorderSession {
    property_id: SharedString,
    option_id: SharedString,
    original_order: Vec<SharedString>,
    last_before_option_id: Option<SharedString>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SlotLimitGuidelineKind {
    MinimumLayers,
    MaximumLayers,
    PreferredInstancesOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SlotLimitGuidelineStatus {
    Met,
    Unmet,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SlotLimitGuideline {
    kind: SlotLimitGuidelineKind,
    label: SharedString,
    status: SlotLimitGuidelineStatus,
}

fn slot_limit_guidelines(property: &DesignComponentProperty) -> Vec<SlotLimitGuideline> {
    let Some(settings) = property.slot_settings() else {
        return Vec::new();
    };
    let actual = match property.effective_value() {
        DesignComponentPropertyValue::Slot(value) => {
            u32::try_from(value.children.len()).unwrap_or(u32::MAX)
        }
        _ => return Vec::new(),
    };
    let status = |kind| {
        let Some(state) = property.slot_state.as_ref() else {
            return SlotLimitGuidelineStatus::Unknown;
        };
        let unmet = state.violations.iter().any(|violation| match kind {
            SlotLimitGuidelineKind::MinimumLayers => {
                matches!(violation, DesignSlotViolation::BelowMinimum { .. })
            }
            SlotLimitGuidelineKind::MaximumLayers => {
                matches!(violation, DesignSlotViolation::AboveMaximum { .. })
            }
            SlotLimitGuidelineKind::PreferredInstancesOnly => matches!(
                violation,
                DesignSlotViolation::NonPreferredValue { .. }
                    | DesignSlotViolation::NonPreferredChild { .. }
                    | DesignSlotViolation::MissingMainComponent { .. }
            ),
        });
        if unmet {
            SlotLimitGuidelineStatus::Unmet
        } else {
            SlotLimitGuidelineStatus::Met
        }
    };

    let mut guidelines = Vec::with_capacity(3);
    if let Some(minimum) = settings.minimum_children {
        let kind = SlotLimitGuidelineKind::MinimumLayers;
        guidelines.push(SlotLimitGuideline {
            kind,
            label: format!("Minimum layers: {minimum} · Currently {actual}").into(),
            status: status(kind),
        });
    }
    if let Some(maximum) = settings.maximum_children {
        let kind = SlotLimitGuidelineKind::MaximumLayers;
        guidelines.push(SlotLimitGuideline {
            kind,
            label: format!("Maximum layers: {maximum} · Currently {actual}").into(),
            status: status(kind),
        });
    }
    if settings.preferred_values_only {
        let kind = SlotLimitGuidelineKind::PreferredInstancesOnly;
        guidelines.push(SlotLimitGuideline {
            kind,
            label: "Only allow preferred instances".into(),
            status: status(kind),
        });
    }
    guidelines
}

fn slot_preferred_violation_layer_ids(property: &DesignComponentProperty) -> Vec<SharedString> {
    if !property
        .slot_settings()
        .is_some_and(|settings| settings.preferred_values_only)
    {
        return Vec::new();
    }
    let Some(state) = property.slot_state.as_ref() else {
        return Vec::new();
    };
    let violating_ids = state
        .violations
        .iter()
        .filter_map(|violation| match violation {
            DesignSlotViolation::NonPreferredValue { instance_id, .. }
            | DesignSlotViolation::MissingMainComponent { instance_id } => {
                Some(instance_id.clone())
            }
            DesignSlotViolation::NonPreferredChild { child_id, .. } => Some(child_id.clone()),
            DesignSlotViolation::BelowMinimum { .. } | DesignSlotViolation::AboveMaximum { .. } => {
                None
            }
        })
        .collect::<HashSet<_>>();
    let DesignComponentPropertyValue::Slot(value) = property.effective_value() else {
        return Vec::new();
    };
    value
        .children
        .iter()
        .filter(|child| violating_ids.contains(&child.node_id))
        .map(|child| child.node_id.clone())
        .collect()
}

#[derive(Clone)]
struct PropertyVariableButtonState {
    target: DesignPropertyVariableTarget,
    panel_id: SharedString,
    search_input: Entity<InputState>,
    active: bool,
    can_change: bool,
    can_edit: bool,
    state: Option<DesignPanelPropertyValueState>,
    variable_view_data: DesignVariableViewData,
}

/// Stateful presentation for a host-controlled Figma-like Design panel.
///
/// The host owns the selected node and every document-facing property. The
/// panel owns only collapsible-section, scroll, focus, and open-picker state.
pub struct DesignPanel {
    id: SharedString,
    focus_handle: FocusHandle,
    node: DesignPanelNode,
    inspection_context: DesignPanelInspectionContext,
    /// Last host echo for each mutually-exclusive permission surface set.
    editor_surface: DesignPanelSurface,
    viewer_surface: DesignPanelSurface,
    /// Orthogonal host-controlled Design/Draw presentation projection.
    workspace_mode: DesignPanelWorkspaceMode,
    property_value_states: HashMap<DesignPanelProperty, DesignPanelPropertyValueState>,
    export_view_data: Option<DesignExportViewData>,
    color_style_view_data: DesignColorStyleViewData,
    color_style_sample_view_data: DesignColorStyleSampleViewData,
    color_contrast_view_data: DesignColorContrastViewData,
    paint_variable_view_data: DesignPaintVariableViewData,
    paint_style_view_data: DesignPaintStyleViewData,
    media_paint_view_data: DesignMediaPaintViewData,
    shader_view_data: DesignShaderViewData,
    typography_style_view_data: DesignTypographyStyleViewData,
    font_view_data: DesignFontViewData,
    effect_style_view_data: DesignEffectStyleViewData,
    effect_variable_view_data: DesignEffectVariableViewData,
    property_variable_view_data: DesignVariableViewData,
    component_swap_view_data: DesignComponentSwapViewData,
    layout_grid_style_view_data: DesignLayoutGridStyleViewData,
    layout_grid_variable_view_data: DesignLayoutGridVariableViewData,
    /// Compatibility snapshot returned by the deprecated count-only getter.
    layout_grid_count_variable_view_data: DesignLayoutGridCountVariableViewData,
    add_auto_layout_view_data: Option<DesignAddAutoLayoutViewData>,
    draw_appearance_view_data: Option<DesignDrawAppearanceViewData>,
    frame_preset_view_data: Option<DesignFramePresetViewData>,
    smart_selection_view_data: Option<DesignSmartSelectionViewData>,
    page_view_data: Option<DesignPageViewData>,
    page_local_styles_view_data: Option<DesignPageLocalStylesViewData>,
    collapsed_local_style_folders: HashSet<SharedString>,
    variables_entry_point: DesignVariablesEntryPoint,
    variable_mode_view_data: Option<DesignVariableModeViewData>,
    viewer_properties_view_data: Option<DesignViewerPropertiesViewData>,
    selection_header_view_data: Option<DesignSelectionHeaderViewData>,
    selection_header_view_data_target: Option<DesignPanelTarget>,
    selection_header_overlay: Option<SelectionHeaderOverlay>,
    /// Cross-file UI preference supplied by the host. This changes only the
    /// inspector's presentation and never participates in document intents.
    additional_labels: bool,
    /// Cross-file keyboard preference supplied by the host.
    nudge_settings: DesignNudgeSettings,
    expanded_export_settings: HashSet<SharedString>,
    export_choice_overlay: Option<SharedString>,
    export_preview_expanded: bool,
    paint_picker: Entity<PaintPicker>,
    typography_style_picker: Entity<TypographyStylePicker>,
    expanded_sections: HashSet<DesignPanelSection>,
    constraints_expanded: bool,
    appearance_blend_mode_open: bool,
    appearance_corner_details_open: bool,
    preview_option_menu_open: Option<DesignPanelProperty>,
    active_menu_preview: Option<DesignMenuPreview>,
    active_picker: Option<PaintPickerTarget>,
    auxiliary_color_picker: Option<AuxiliaryColorPickerTarget>,
    active_paint_edit: Option<ActivePaintEdit>,
    active_effect_settings: Option<EffectSettingsTarget>,
    paint_style_browser_open: Option<DesignPanelCollection>,
    selection_color_resource_browser: Option<SelectionColorResourceTarget>,
    effect_style_browser_open: bool,
    property_variable_picker: Option<DesignPanelProperty>,
    component_property_variable_picker: Option<DesignComponentPropertyVariableTarget>,
    component_swap_browser: Option<SharedString>,
    component_swap_hovered: Option<(SharedString, DesignComponentSwapSelection)>,
    open_slot_limits: Option<SharedString>,
    component_property_create_menu_open: bool,
    component_property_create_draft: Option<ComponentPropertyCreateDraft>,
    component_property_edit_modal: Option<ComponentPropertyEditDraft>,
    component_authoring_dialog_open: bool,
    component_authoring_dialog_close_pending: bool,
    #[cfg(test)]
    component_authoring_dialog_last_rendered_kind: Option<DesignComponentPropertyKind>,
    component_property_selected: Option<SharedString>,
    component_property_context_menu: Option<SharedString>,
    component_authoring_name_editor: Option<ComponentAuthoringNameEditor>,
    component_property_reorder: Option<ComponentPropertyReorderSession>,
    component_variant_option_reorder: Option<ComponentVariantOptionReorderSession>,
    layout_grid_style_browser_open: bool,
    layout_grid_count_variable_target: Option<DesignLayoutGridVariableTarget>,
    frame_preset_browser_open: bool,
    collapsed_frame_preset_groups: HashSet<SharedString>,
    page_background_picker_open: bool,
    /// Deprecated local-resource browser state retained only for compatibility
    /// with hosts that still call the legacy Page resource intents.
    page_resource_browser: Option<DesignLocalResourceCategory>,
    variable_mode_browser_open: bool,
    typography_style_picker_open: bool,
    font_browser_open: bool,
    type_settings_open: bool,
    type_settings_tab: TypographySettingsTab,
    padding_editor_mode: PaddingEditorMode,
    grid_dimensions_picker: Option<GridDimensionsPicker>,
    active_grid_dimensions_edit: Option<GridDimensionsEdit>,
    dimension_limit_fields_disclosed: HashSet<DesignPanelProperty>,
    dimension_menu_open: Option<DesignLayoutDimensionAxis>,
    dimension_limits_preview: Option<DimensionLimitsPreview>,
    dimension_menu_focus: [FocusHandle; 2],
    type_settings_focus: FocusHandle,
    property_input: Entity<InputState>,
    property_variable_search: Entity<InputState>,
    component_property_variable_search: Entity<InputState>,
    component_swap_search: Entity<InputState>,
    component_authoring_name_input: Entity<InputState>,
    component_authoring_default_input: Entity<InputState>,
    component_authoring_slot_minimum_input: Entity<InputState>,
    component_authoring_slot_maximum_input: Entity<InputState>,
    font_search: Entity<InputState>,
    style_browser_search: Entity<InputState>,
    style_browser_source_filter: StyleBrowserSourceFilter,
    style_browser_view_mode: StyleBrowserViewMode,
    component_multiline_input: Entity<InputState>,
    property_editor: Option<PropertyEditor>,
    numeric_property_scrub: Option<NumericPropertyScrub>,
    /// Logical control identity plus its retained GPUI handle. Explicit
    /// keyboard/scrub exits restore it after rendering; input blur never does.
    editor_focus_return: Option<EditorFocusReturn>,
    draw_appearance_slider_property: Option<DesignPanelProperty>,
    draw_opacity_slider: Entity<SliderState>,
    draw_corner_radius_slider: Entity<SliderState>,
    _draw_opacity_slider_subscription: Subscription,
    _draw_corner_radius_slider_subscription: Subscription,
    variable_font_axis_editor: Option<VariableFontAxisEditor>,
    variable_font_axis_scrub: Option<VariableFontAxisScrub>,
    component_multiline_editor: Option<ComponentMultilineEditor>,
    /// Stable host vertex identities captured when a numeric vector edit begins.
    vector_edit_target_ids: Option<Vec<SharedString>>,
    property_editor_invalid: bool,
    variable_font_axis_editor_invalid: bool,
    suppress_property_input_change: bool,
    suppress_next_control_activation: bool,
    option_states: HashMap<DesignPanelProperty, Entity<PropertySelectState>>,
    option_subscriptions: HashMap<DesignPanelProperty, Subscription>,
    option_snapshots: HashMap<DesignPanelProperty, PropertyOptionSnapshot>,
    scroll_handle: ScrollHandle,
    reset_scroll_after_render: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DesignPanelAction> for DesignPanel {}

trait DesignPanelActionEmitter {
    fn emit_design_panel_action(&mut self, panel: &DesignPanel, action: DesignPanelAction);
}

impl DesignPanelActionEmitter for Context<'_, DesignPanel> {
    fn emit_design_panel_action(&mut self, panel: &DesignPanel, mut action: DesignPanelAction) {
        if action.paint_target().is_some() && !panel.node_capability_allows_action(&action) {
            return;
        }
        if panel.inspection_context.selection().kind() == DesignPanelSelectionKind::Multiple
            && action.legacy_node_id_mut().is_some()
        {
            action = DesignPanelAction::for_selection_target(panel.command_target(), action);
        }
        self.emit(action);
    }
}

#[allow(deprecated)]
impl DesignPanel {
    pub fn new(
        id: impl Into<SharedString>,
        node: DesignPanelNode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let id = id.into();
        let property_input = cx.new(|cx| InputState::new(window, cx));
        let subscription = cx.subscribe_in(
            &property_input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    if this.variable_font_axis_editor.is_some() {
                        this.validate_variable_font_axis_draft(cx);
                    } else {
                        this.validate_property_draft(cx);
                    }
                }
                InputEvent::PressEnter { .. } => {
                    this.suppress_next_control_activation = true;
                    window.prevent_default();
                    if this.variable_font_axis_editor.is_some() {
                        this.finish_variable_font_axis_edit(true, window, cx);
                    } else {
                        this.finish_property_edit(true, window, cx);
                    }
                }
                InputEvent::Blur => {
                    if this.variable_font_axis_editor.is_some() {
                        this.finish_variable_font_axis_edit(true, window, cx);
                    } else {
                        this.finish_property_edit_after_input_blur(true, window, cx);
                    }
                }
                InputEvent::Focus => {}
            },
        );
        let property_variable_search = cx.new(|cx| InputState::new(window, cx));
        let property_variable_search_subscription =
            cx.subscribe(&property_variable_search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });
        let component_property_variable_search = cx.new(|cx| InputState::new(window, cx));
        let component_property_variable_search_subscription = cx.subscribe(
            &component_property_variable_search,
            |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        );
        let component_swap_search = cx.new(|cx| InputState::new(window, cx));
        let component_swap_search_subscription =
            cx.subscribe(&component_swap_search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });
        let component_authoring_name_input = cx.new(|cx| InputState::new(window, cx));
        let component_authoring_name_input_subscription = cx.subscribe_in(
            &component_authoring_name_input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => this.preview_component_authoring_name(cx),
                InputEvent::PressEnter { .. } => {
                    window.prevent_default();
                    if this.component_authoring_name_editor.is_some() {
                        this.finish_component_authoring_name_edit(true, window, cx);
                    } else if this.component_property_create_draft.is_some() {
                        this.submit_component_property_create(window, cx);
                    }
                }
                InputEvent::Blur => {
                    this.finish_component_authoring_name_edit(true, window, cx);
                }
                InputEvent::Focus => {}
            },
        );
        let component_authoring_default_input = cx.new(|cx| InputState::new(window, cx));
        let component_authoring_default_input_subscription = cx.subscribe(
            &component_authoring_default_input,
            |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        );
        let component_authoring_slot_minimum_input = cx.new(|cx| InputState::new(window, cx));
        let component_authoring_slot_minimum_input_subscription = cx.subscribe(
            &component_authoring_slot_minimum_input,
            |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        );
        let component_authoring_slot_maximum_input = cx.new(|cx| InputState::new(window, cx));
        let component_authoring_slot_maximum_input_subscription = cx.subscribe(
            &component_authoring_slot_maximum_input,
            |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        );
        let font_search = cx.new(|cx| InputState::new(window, cx));
        let font_search_subscription =
            cx.subscribe(&font_search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });
        let style_browser_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search styles"));
        let style_browser_search_subscription =
            cx.subscribe(&style_browser_search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });
        let component_multiline_input = cx.new(|cx| InputState::new(window, cx).auto_grow(3, 8));
        let component_multiline_input_subscription = cx.subscribe(
            &component_multiline_input,
            |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.preview_component_multiline(cx);
                }
            },
        );
        let paint_picker = cx.new(|cx| {
            PaintPicker::new(SharedString::from(format!("{id}-paint-picker")), window, cx)
        });
        let paint_picker_subscription = cx.subscribe_in(
            &paint_picker,
            window,
            |this, _, event: &PaintPickerEvent, _, cx| match event {
                PaintPickerEvent::Edit {
                    target,
                    edit,
                    phase,
                } if this
                    .auxiliary_color_picker
                    .as_ref()
                    .is_some_and(|active| active.matches_picker_target(target)) =>
                {
                    let leaf_is_editable = this
                        .auxiliary_color_picker
                        .as_ref()
                        .is_some_and(|active| this.auxiliary_paint_editable(active, edit));
                    if (matches!(phase, DesignPanelEditPhase::Cancel) || leaf_is_editable)
                        && this.track_paint_edit(target, edit, *phase)
                    {
                        this.emit_auxiliary_color_edit(edit, *phase, cx);
                    }
                }
                PaintPickerEvent::Edit {
                    target,
                    edit,
                    phase,
                } if target.node_id == this.node.id
                    && this.active_picker
                        == Some(PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        }) =>
                {
                    if this.track_paint_edit(target, edit, *phase) {
                        this.emit_paint_edit(
                            PaintPickerTarget {
                                collection: target.collection,
                                index: target.index,
                                paint_id: target.paint_id.clone(),
                            },
                            edit.as_ref().clone(),
                            *phase,
                            cx,
                        );
                    }
                }
                PaintPickerEvent::Edit { .. } => {}
                PaintPickerEvent::BlendModePreview {
                    target,
                    original,
                    candidate,
                    phase,
                } => match phase {
                    DesignMenuPreviewPhase::Begin => {
                        this.begin_paint_blend_mode_preview(target, *original, *candidate, cx)
                    }
                    DesignMenuPreviewPhase::End => {
                        this.end_paint_blend_mode_preview(target, *original, *candidate, cx)
                    }
                },
                PaintPickerEvent::SourceReplaceRequested { target }
                    if target.node_id == this.node.id
                        && this.active_picker
                            == Some(PaintPickerTarget {
                                collection: target.collection,
                                index: target.index,
                                paint_id: target.paint_id.clone(),
                            }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    if let Some(index) = this.paint_target_index(&panel_target)
                        && this.can_edit()
                        && this.paint_style_binding(target.collection).is_none()
                        && this
                            .paint_collection(target.collection)
                            .and_then(|paints| paints.get(index))
                            .is_some_and(|paint| {
                                !paint.read_only
                                    && matches!(&paint.payload, DesignPaintPayload::Pattern(_))
                            })
                    {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintSourceReplaceRequested {
                                node_id: this.node.id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                            },
                        );
                    }
                }
                PaintPickerEvent::SourceReplaceRequested { .. } => {}
                PaintPickerEvent::MediaSourceActionRequested {
                    target,
                    source_id,
                    action,
                } if target.node_id == this.node.id
                    && this.active_picker
                        == Some(PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        }) =>
                {
                    this.emit_media_source_action(
                        PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        },
                        source_id,
                        *action,
                        cx,
                    );
                }
                PaintPickerEvent::MediaSourceActionRequested { .. } => {}
                PaintPickerEvent::MediaSourceDropRequested {
                    target,
                    expected_source_id,
                    expected_media_kind,
                    file,
                } if target.node_id == this.node.id
                    && this.active_picker
                        == Some(PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        }) =>
                {
                    this.emit_media_source_drop(
                        PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        },
                        expected_source_id,
                        *expected_media_kind,
                        file.clone(),
                        cx,
                    );
                }
                PaintPickerEvent::MediaSourceDropRequested { .. } => {}
                PaintPickerEvent::MediaCropActionRequested { target, action }
                    if target.node_id == this.node.id
                        && this.active_picker
                            == Some(PaintPickerTarget {
                                collection: target.collection,
                                index: target.index,
                                paint_id: target.paint_id.clone(),
                            }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    if let Some(index) = this.paint_target_index(&panel_target)
                        && this.can_edit()
                    {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintMediaCropActionRequested {
                                node_id: this.node.id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                                action: action.clone(),
                            },
                        );
                    }
                }
                PaintPickerEvent::MediaCropActionRequested { .. } => {}
                PaintPickerEvent::VideoPreviewActionRequested { target, action }
                    if target.node_id == this.node.id
                        && this.active_picker
                            == Some(PaintPickerTarget {
                                collection: target.collection,
                                index: target.index,
                                paint_id: target.paint_id.clone(),
                            }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    if let Some(index) = this.paint_target_index(&panel_target) {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintVideoPreviewActionRequested {
                                node_id: this.node.id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                                action: action.clone(),
                            },
                        );
                    }
                }
                PaintPickerEvent::VideoPreviewActionRequested { .. } => {}
                PaintPickerEvent::ShaderImportRequested { target, shader }
                    if target.node_id == this.node.id
                        && this.active_picker
                            == Some(PaintPickerTarget {
                                collection: target.collection,
                                index: target.index,
                                paint_id: target.paint_id.clone(),
                            }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    if let Some(index) = this.paint_target_index(&panel_target)
                        && this.can_edit()
                        && this
                            .shader_view_data
                            .shader(shader)
                            .is_some_and(|definition| !definition.imported)
                    {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintShaderImportRequested {
                                node_id: this.node.id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                                shader: shader.clone(),
                            },
                        );
                    }
                }
                PaintPickerEvent::ShaderImportRequested { .. } => {}
                PaintPickerEvent::ShaderApplyRequested { target, shader }
                    if target.node_id == this.node.id
                        && this.active_picker
                            == Some(PaintPickerTarget {
                                collection: target.collection,
                                index: target.index,
                                paint_id: target.paint_id.clone(),
                            }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    if let Some(index) = this.paint_target_index(&panel_target)
                        && this.can_edit()
                        && this
                            .shader_view_data
                            .shader(shader)
                            .is_some_and(|definition| definition.imported)
                    {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintShaderApplyRequested {
                                node_id: this.node.id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                                shader: shader.clone(),
                            },
                        );
                    }
                }
                PaintPickerEvent::ShaderApplyRequested { .. } => {}
                PaintPickerEvent::ShaderPropertyBindRequested {
                    target,
                    definition_id,
                } if target.node_id == this.node.id
                    && this.active_picker
                        == Some(PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    if let Some(index) = this.paint_target_index(&panel_target)
                        && this.can_edit()
                    {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintShaderPropertyBindRequested {
                                node_id: this.node.id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                                definition_id: definition_id.clone(),
                            },
                        );
                    }
                }
                PaintPickerEvent::ShaderPropertyBindRequested { .. } => {}
                PaintPickerEvent::ShaderPropertyEditorRequested {
                    target,
                    definition_id,
                } if target.node_id == this.node.id
                    && this.active_picker
                        == Some(PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    if let Some(index) = this.paint_target_index(&panel_target)
                        && this.can_edit()
                    {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintShaderPropertyEditorRequested {
                                node_id: this.node.id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                                definition_id: definition_id.clone(),
                            },
                        );
                    }
                }
                PaintPickerEvent::ShaderPropertyEditorRequested { .. } => {}
                PaintPickerEvent::ShaderPropertyDetachRequested {
                    target,
                    definition_id,
                    variable_id,
                } if target.node_id == this.node.id
                    && this.active_picker
                        == Some(PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    if let Some(index) = this.paint_target_index(&panel_target)
                        && this.can_edit()
                    {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintShaderPropertyDetachRequested {
                                node_id: this.node.id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                                definition_id: definition_id.clone(),
                                variable_id: variable_id.clone(),
                            },
                        );
                    }
                }
                PaintPickerEvent::ShaderPropertyDetachRequested { .. } => {}
                PaintPickerEvent::ColorVariableApplyRequested {
                    target,
                    color_target,
                    variable_id,
                } if target.node_id == this.node.id
                    && this.active_picker
                        == Some(PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    let variable = this.paint_variable_view_data.variable(variable_id.as_ref());
                    if let Some(index) = this.paint_target_index(&panel_target)
                        && this.can_edit()
                        && variable.is_some_and(|variable| {
                            variable.disabled_reason.is_none()
                                && matches!(
                                    variable.import_state,
                                    DesignVariableImportState::Local
                                        | DesignVariableImportState::Imported
                                )
                        })
                        && let Some(color_target) =
                            this.resolve_paint_color_target(&panel_target, color_target, true)
                    {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintColorVariableApplyRequested {
                                node_id: this.node.id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                                color_target,
                                variable_id: variable_id.clone(),
                            },
                        );
                    }
                }
                PaintPickerEvent::ColorVariableApplyRequested { .. } => {}
                PaintPickerEvent::ColorVariableImportRequested {
                    target,
                    color_target,
                    variable_id,
                } if target.node_id == this.node.id
                    && this.active_picker
                        == Some(PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    let variable = this.paint_variable_view_data.variable(variable_id.as_ref());
                    if let Some(index) = this.paint_target_index(&panel_target)
                        && this.can_edit()
                        && variable.is_some_and(|variable| {
                            variable.disabled_reason.is_none()
                                && variable.import_state == DesignVariableImportState::Available
                        })
                        && let Some(color_target) =
                            this.resolve_paint_color_target(&panel_target, color_target, true)
                    {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintColorVariableImportRequested {
                                node_id: this.node.id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                                color_target,
                                variable_id: variable_id.clone(),
                            },
                        );
                    }
                }
                PaintPickerEvent::ColorVariableImportRequested { .. } => {}
                PaintPickerEvent::ColorVariableDetachRequested {
                    target,
                    color_target,
                    variable_id,
                } if target.node_id == this.node.id
                    && this.active_picker
                        == Some(PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    if let Some(index) = this.paint_target_index(&panel_target)
                        && this.can_edit()
                        && let Some(color_target) =
                            this.resolve_paint_color_target(&panel_target, color_target, true)
                        && this
                            .paint_color_binding(&panel_target, &color_target)
                            .is_some_and(|binding| {
                                binding.variable_id.as_ref() == variable_id.as_ref()
                            })
                    {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintColorVariableDetachRequested {
                                node_id: this.node.id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                                color_target,
                                variable_id: variable_id.clone(),
                            },
                        );
                    }
                }
                PaintPickerEvent::ColorVariableDetachRequested { .. } => {}
                PaintPickerEvent::ColorVariableCreateRequested {
                    target,
                    color_target,
                    color,
                } if target.node_id == this.node.id
                    && this.active_picker
                        == Some(PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    if let Some(index) = this.paint_target_index(&panel_target)
                        && this.can_edit()
                        && let Some(color_target) =
                            this.resolve_paint_color_target(&panel_target, color_target, true)
                    {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintColorVariableCreateRequested {
                                node_id: this.node.id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                                color_target,
                                color: *color,
                            },
                        );
                    }
                }
                PaintPickerEvent::ColorVariableCreateRequested { .. } => {}
                PaintPickerEvent::PaintStyleCreateRequested {
                    target,
                    color_target,
                } if target.node_id == this.node.id
                    && this.active_picker
                        == Some(PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    if this.paint_target_index(&panel_target).is_some()
                        && this.can_edit()
                        && this
                            .resolve_paint_color_target(&panel_target, color_target, true)
                            .is_some()
                    {
                        this.emit_paint_style_create(target.collection, cx);
                    }
                }
                PaintPickerEvent::PaintStyleCreateRequested { .. } => {}
                PaintPickerEvent::ColorStyleSampleRequested {
                    target,
                    color_target,
                    sample,
                } if target.node_id == this.node.id
                    && this.active_picker
                        == Some(PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    if let Some(index) = this.paint_target_index(&panel_target)
                        && this.can_edit()
                        && this
                            .color_style_sample_view_data
                            .sample(sample)
                            .is_some_and(|sample| sample.disabled_reason.is_none())
                        && let Some(color_target) =
                            this.resolve_paint_color_target(&panel_target, color_target, true)
                    {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintColorStyleSampleRequested {
                                node_id: this.node.id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                                color_target,
                                sample: sample.clone(),
                            },
                        );
                    }
                }
                PaintPickerEvent::ColorStyleSampleRequested { .. } => {}
                PaintPickerEvent::EyedropperRequested {
                    target,
                    color_target,
                } if target.node_id == this.node.id
                    && this.active_picker
                        == Some(PaintPickerTarget {
                            collection: target.collection,
                            index: target.index,
                            paint_id: target.paint_id.clone(),
                        }) =>
                {
                    let panel_target = PaintPickerTarget {
                        collection: target.collection,
                        index: target.index,
                        paint_id: target.paint_id.clone(),
                    };
                    if let Some(index) = this.paint_target_index(&panel_target)
                        && this.can_edit()
                        && let Some(color_target) =
                            this.resolve_paint_color_target(&panel_target, color_target, false)
                    {
                        cx.emit_design_panel_action(
                            this,
                            DesignPanelAction::PaintEyedropperRequested {
                                node_id: this.node.id.clone(),
                                collection: target.collection,
                                target: this.paint_target(target.collection),
                                paint_id: target.paint_id.clone(),
                                index,
                                color_target,
                            },
                        );
                    }
                }
                PaintPickerEvent::EyedropperRequested { .. } => {}
            },
        );
        let typography_style_picker = cx.new(|cx| {
            TypographyStylePicker::new(
                SharedString::from(format!("{id}-typography-style-picker")),
                window,
                cx,
            )
        });
        let typography_style_picker_subscription = cx.subscribe(
            &typography_style_picker,
            |this, _, event: &TypographyStylePickerEvent, cx| match event {
                TypographyStylePickerEvent::ApplyRequested { style }
                    if this.property_is_editable(DesignPanelProperty::TypographyStyle)
                        && this.typography_style_view_data.style(style).is_some() =>
                {
                    cx.emit_design_panel_action(
                        this,
                        DesignPanelAction::TypographyStyleApplyRequested {
                            node_id: this.node.id.clone(),
                            target: this.typography_target(DesignPanelProperty::TypographyStyle),
                            style: style.clone(),
                        },
                    );
                }
                TypographyStylePickerEvent::DetachRequested { style }
                    if this.property_is_editable(DesignPanelProperty::TypographyStyle)
                        && this
                            .node
                            .typography
                            .as_ref()
                            .and_then(|typography| typography.style_binding.as_ref())
                            .is_some_and(|binding| {
                                binding.can_detach && binding.selection == *style
                            }) =>
                {
                    cx.emit_design_panel_action(
                        this,
                        DesignPanelAction::TypographyStyleDetachRequested {
                            node_id: this.node.id.clone(),
                            target: this.typography_target(DesignPanelProperty::TypographyStyle),
                            style: style.clone(),
                        },
                    );
                }
                TypographyStylePickerEvent::ApplyRequested { .. }
                | TypographyStylePickerEvent::DetachRequested { .. } => {}
            },
        );
        let draw_opacity_slider = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(100.)
                .step(1.)
                .default_value(node.opacity)
        });
        let draw_opacity_slider_subscription =
            cx.subscribe(&draw_opacity_slider, |this, _, event: &SliderEvent, cx| {
                let SliderEvent::Change(SliderValue::Single(value)) = event else {
                    return;
                };
                this.preview_draw_appearance_slider(DesignPanelProperty::Opacity, *value, cx);
            });
        let draw_corner_radius_slider = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(100.)
                .step(1.)
                .default_value(node.corner_radii[0].clamp(0., 100.))
        });
        let draw_corner_radius_slider_subscription = cx.subscribe(
            &draw_corner_radius_slider,
            |this, _, event: &SliderEvent, cx| {
                let SliderEvent::Change(SliderValue::Single(value)) = event else {
                    return;
                };
                this.preview_draw_appearance_slider(DesignPanelProperty::CornerRadius, *value, cx);
            },
        );
        let inspection_context = DesignPanelInspectionContext::single(
            node.clone(),
            DesignPanelParentLayout::Freeform,
            DesignPanelPermissions::editor(),
        );
        let padding_editor_mode = PaddingEditorMode::for_node(&node);
        Self {
            id,
            focus_handle: cx.focus_handle(),
            node,
            inspection_context,
            editor_surface: DesignPanelSurface::Design,
            viewer_surface: DesignPanelSurface::Properties,
            workspace_mode: DesignPanelWorkspaceMode::Design,
            property_value_states: HashMap::new(),
            export_view_data: None,
            color_style_view_data: DesignColorStyleViewData::default(),
            color_style_sample_view_data: DesignColorStyleSampleViewData::default(),
            color_contrast_view_data: DesignColorContrastViewData::default(),
            paint_variable_view_data: DesignPaintVariableViewData::default(),
            paint_style_view_data: DesignPaintStyleViewData::default(),
            media_paint_view_data: DesignMediaPaintViewData::default(),
            shader_view_data: DesignShaderViewData::default(),
            typography_style_view_data: DesignTypographyStyleViewData::default(),
            font_view_data: DesignFontViewData::default(),
            effect_style_view_data: DesignEffectStyleViewData::default(),
            effect_variable_view_data: DesignEffectVariableViewData::default(),
            property_variable_view_data: DesignVariableViewData::default(),
            component_swap_view_data: DesignComponentSwapViewData::default(),
            layout_grid_style_view_data: DesignLayoutGridStyleViewData::default(),
            layout_grid_variable_view_data: DesignLayoutGridVariableViewData::default(),
            layout_grid_count_variable_view_data: DesignLayoutGridCountVariableViewData::default(),
            add_auto_layout_view_data: None,
            draw_appearance_view_data: None,
            frame_preset_view_data: None,
            smart_selection_view_data: None,
            page_view_data: None,
            page_local_styles_view_data: None,
            collapsed_local_style_folders: HashSet::new(),
            variables_entry_point: DesignVariablesEntryPoint::default(),
            variable_mode_view_data: None,
            viewer_properties_view_data: None,
            selection_header_view_data: None,
            selection_header_view_data_target: None,
            selection_header_overlay: None,
            additional_labels: false,
            nudge_settings: DesignNudgeSettings::default(),
            expanded_export_settings: HashSet::new(),
            export_choice_overlay: None,
            export_preview_expanded: false,
            paint_picker,
            typography_style_picker,
            expanded_sections: [
                DesignPanelSection::Selection,
                DesignPanelSection::Component,
                DesignPanelSection::Instance,
                DesignPanelSection::Position,
                DesignPanelSection::Layout,
                DesignPanelSection::Constraints,
                DesignPanelSection::Layer,
                DesignPanelSection::Section,
                DesignPanelSection::Transform,
                DesignPanelSection::Geometry,
                DesignPanelSection::Mask,
                DesignPanelSection::Typography,
                DesignPanelSection::Media,
                DesignPanelSection::Fill,
                DesignPanelSection::Stroke,
                DesignPanelSection::Effects,
                DesignPanelSection::LayoutGrid,
                DesignPanelSection::Export,
            ]
            .into_iter()
            .collect(),
            constraints_expanded: true,
            appearance_blend_mode_open: false,
            appearance_corner_details_open: false,
            preview_option_menu_open: None,
            active_menu_preview: None,
            active_picker: None,
            auxiliary_color_picker: None,
            active_paint_edit: None,
            active_effect_settings: None,
            paint_style_browser_open: None,
            selection_color_resource_browser: None,
            effect_style_browser_open: false,
            property_variable_picker: None,
            component_property_variable_picker: None,
            component_swap_browser: None,
            component_swap_hovered: None,
            open_slot_limits: None,
            component_property_create_menu_open: false,
            component_property_create_draft: None,
            component_property_edit_modal: None,
            component_authoring_dialog_open: false,
            component_authoring_dialog_close_pending: false,
            #[cfg(test)]
            component_authoring_dialog_last_rendered_kind: None,
            component_property_selected: None,
            component_property_context_menu: None,
            component_authoring_name_editor: None,
            component_property_reorder: None,
            component_variant_option_reorder: None,
            layout_grid_style_browser_open: false,
            layout_grid_count_variable_target: None,
            frame_preset_browser_open: false,
            collapsed_frame_preset_groups: HashSet::new(),
            page_background_picker_open: false,
            page_resource_browser: None,
            variable_mode_browser_open: false,
            typography_style_picker_open: false,
            font_browser_open: false,
            type_settings_open: false,
            type_settings_tab: TypographySettingsTab::Basics,
            padding_editor_mode,
            grid_dimensions_picker: None,
            active_grid_dimensions_edit: None,
            dimension_limit_fields_disclosed: HashSet::new(),
            dimension_menu_open: None,
            dimension_limits_preview: None,
            dimension_menu_focus: [
                cx.focus_handle().tab_index(0).tab_stop(true),
                cx.focus_handle().tab_index(0).tab_stop(true),
            ],
            type_settings_focus: cx.focus_handle(),
            property_input,
            property_variable_search,
            component_property_variable_search,
            component_swap_search,
            component_authoring_name_input,
            component_authoring_default_input,
            component_authoring_slot_minimum_input,
            component_authoring_slot_maximum_input,
            font_search,
            style_browser_search,
            style_browser_source_filter: StyleBrowserSourceFilter::All,
            style_browser_view_mode: StyleBrowserViewMode::List,
            component_multiline_input,
            property_editor: None,
            numeric_property_scrub: None,
            editor_focus_return: None,
            draw_appearance_slider_property: None,
            draw_opacity_slider,
            draw_corner_radius_slider,
            _draw_opacity_slider_subscription: draw_opacity_slider_subscription,
            _draw_corner_radius_slider_subscription: draw_corner_radius_slider_subscription,
            variable_font_axis_editor: None,
            variable_font_axis_scrub: None,
            component_multiline_editor: None,
            vector_edit_target_ids: None,
            property_editor_invalid: false,
            variable_font_axis_editor_invalid: false,
            suppress_property_input_change: false,
            suppress_next_control_activation: false,
            option_states: HashMap::new(),
            option_subscriptions: HashMap::new(),
            option_snapshots: HashMap::new(),
            scroll_handle: ScrollHandle::new(),
            reset_scroll_after_render: true,
            _subscriptions: vec![
                subscription,
                property_variable_search_subscription,
                component_property_variable_search_subscription,
                component_swap_search_subscription,
                component_authoring_name_input_subscription,
                component_authoring_default_input_subscription,
                component_authoring_slot_minimum_input_subscription,
                component_authoring_slot_maximum_input_subscription,
                font_search_subscription,
                style_browser_search_subscription,
                component_multiline_input_subscription,
                paint_picker_subscription,
                typography_style_picker_subscription,
            ],
        }
    }

    /// Creates a panel directly from a complete inspection context.
    pub fn new_with_context(
        id: impl Into<SharedString>,
        inspection_context: DesignPanelInspectionContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let node = inspection_context
            .selection()
            .items()
            .first()
            .cloned()
            .unwrap_or_else(|| {
                DesignPanelNode::new("design-panel-page", "Page", DesignPanelNodeKind::Frame)
            });
        let mut panel = Self::new(id, node, window, cx);
        panel.inspection_context = inspection_context;
        panel
    }

    /// Replaces the immutable selection data supplied by the host.
    pub fn set_node(&mut self, node: DesignPanelNode, cx: &mut Context<Self>) {
        let node_changed = self.node.id != node.id;
        if self.active_menu_preview.is_some() && self.node != node {
            self.cancel_menu_preview(cx);
        }
        let dimension_preview_invalidated =
            self.dimension_limits_preview
                .as_ref()
                .is_some_and(|preview| {
                    preview.node_id != node.id
                        || !Self::dimension_limits_are_applicable_for(
                            &node,
                            &self.inspection_context,
                        )
                        || Self::dimension_limits_for_node(&node, preview.axis)
                            != (preview.minimum, preview.maximum)
                });
        if node_changed || dimension_preview_invalidated {
            self.cancel_dimension_limits_preview(cx);
        }
        let parent_layout = self.inspection_context.parent_layout();
        let permissions = self.inspection_context.permissions();
        let edit_mode = self.inspection_context.edit_mode();
        let text_range_revision = self.inspection_context.text_range_revision();
        let next_context = match self.inspection_context.selection().kind() {
            DesignPanelSelectionKind::None => self.inspection_context.clone(),
            DesignPanelSelectionKind::Single => {
                let context =
                    DesignPanelInspectionContext::single(node.clone(), parent_layout, permissions)
                        .with_edit_mode(edit_mode)
                        .expect(
                            "a valid single-selection edit mode remains valid after a host echo",
                        );
                match text_range_revision {
                    Some(revision) => context
                        .with_text_range_revision(revision)
                        .expect("a text-range revision remains valid in Text edit mode"),
                    None => context,
                }
            }
            DesignPanelSelectionKind::Multiple => {
                let items = self.inspection_context.selection().items();
                DesignPanelInspectionContext::multiple(
                    DesignPanelMultipleSelection::with_remaining(
                        node.clone(),
                        items[1].clone(),
                        items.iter().skip(2).cloned(),
                    ),
                    parent_layout,
                    permissions,
                )
            }
        };
        let next_selection_header_target = match self.inspection_context.selection().kind() {
            DesignPanelSelectionKind::None => None,
            DesignPanelSelectionKind::Single => Some(DesignPanelTarget::Nodes {
                node_ids: vec![node.id.clone()],
            }),
            DesignPanelSelectionKind::Multiple => Some(DesignPanelTarget::Nodes {
                node_ids: std::iter::once(node.id.clone())
                    .chain(
                        self.inspection_context
                            .selection()
                            .items()
                            .iter()
                            .skip(1)
                            .map(|item| item.id.clone()),
                    )
                    .collect(),
            }),
        };
        if !node_changed
            && self.node != node
            && self
                .numeric_property_scrub
                .as_ref()
                .is_some_and(|scrub| !scrub.active)
        {
            self.numeric_property_scrub = None;
        }
        let rebased_property = (!node_changed)
            .then(|| self.cancel_interactions_invalidated_by_host_echo(&node, &next_context, cx))
            .flatten();
        if !node_changed && self.node != node {
            self.clear_option_interactions();
        }
        if node_changed {
            self.cancel_host_interactions_for_context_change(true, cx);
            if self.component_authoring_dialog_open {
                self.component_authoring_dialog_open = false;
                self.component_authoring_dialog_close_pending = true;
            }
            self.auxiliary_color_picker = None;
            self.padding_editor_mode = PaddingEditorMode::for_node(&node);
            self.active_picker = None;
            self.active_effect_settings = None;
            self.paint_style_browser_open = None;
            self.effect_style_browser_open = false;
            self.property_variable_picker = None;
            self.component_property_variable_picker = None;
            self.component_swap_browser = None;
            self.component_swap_hovered = None;
            self.open_slot_limits = None;
            self.component_property_create_menu_open = false;
            self.component_property_create_draft = None;
            self.component_property_edit_modal = None;
            self.component_property_selected = None;
            self.component_property_context_menu = None;
            self.component_authoring_name_editor = None;
            self.component_property_reorder = None;
            self.component_variant_option_reorder = None;
            self.component_multiline_editor = None;
            self.layout_grid_style_browser_open = false;
            self.layout_grid_count_variable_target = None;
            self.frame_preset_browser_open = false;
            self.page_background_picker_open = false;
            self.variable_mode_browser_open = false;
            self.typography_style_picker_open = false;
            self.font_browser_open = false;
            self.selection_header_overlay = None;
            if self.selection_header_view_data_target.as_ref()
                != next_selection_header_target.as_ref()
            {
                self.selection_header_view_data = None;
                self.selection_header_view_data_target = None;
            }
            self.type_settings_open = false;
            self.type_settings_tab = TypographySettingsTab::Basics;
            self.grid_dimensions_picker = None;
            self.dimension_limit_fields_disclosed.clear();
            self.dimension_menu_open = None;
            self.constraints_expanded = true;
            self.appearance_blend_mode_open = false;
            self.appearance_corner_details_open = false;
            self.preview_option_menu_open = None;
            self.active_menu_preview = None;
            self.property_editor = None;
            self.numeric_property_scrub = None;
            self.variable_font_axis_editor = None;
            self.variable_font_axis_scrub = None;
            self.vector_edit_target_ids = None;
            self.property_editor_invalid = false;
            self.variable_font_axis_editor_invalid = false;
            self.suppress_property_input_change = false;
            self.suppress_next_control_activation = false;
            self.scroll_handle.set_offset(gpui::point(px(0.), px(0.)));
            self.reset_scroll_after_render = true;
            self.property_value_states.clear();
            self.media_paint_view_data = DesignMediaPaintViewData::default();
            self.paint_picker.update(cx, |picker, cx| {
                picker.set_media_view_data(DesignMediaPaintViewData::default(), cx);
            });
            self.export_view_data = None;
            self.expanded_export_settings.clear();
            self.export_choice_overlay = None;
            self.export_preview_expanded = false;
        }
        self.node = node;
        self.inspection_context = next_context;
        self.reconcile_slot_limits_state();
        self.reconcile_component_authoring_state();
        if !self.dimension_limits_are_applicable() {
            self.dimension_limit_fields_disclosed.clear();
            self.dimension_menu_open = None;
        }
        if let Some(property) = rebased_property {
            if let Some(previous) = self
                .property_editor
                .as_ref()
                .map(|editor| editor.property)
                .or_else(|| {
                    self.numeric_property_scrub
                        .as_ref()
                        .map(|scrub| scrub.property)
                })
            {
                self.rebase_editor_focus_origin(previous, property);
            }
            if let Some(editor) = self.property_editor.as_mut() {
                editor.property = property;
            }
            if let Some(scrub) = self.numeric_property_scrub.as_mut() {
                scrub.property = property;
            }
        }
        if self
            .active_picker
            .as_ref()
            .is_some_and(|target| self.picker_paint(target).is_none())
        {
            self.active_picker = None;
        }
        if self
            .paint_style_browser_open
            .is_some_and(|collection| !self.collection_is_supported(collection))
        {
            self.paint_style_browser_open = None;
        }
        if !self.collection_is_supported(DesignPanelCollection::Effect) {
            self.active_effect_settings = None;
            self.effect_style_browser_open = false;
        }
        if !self.collection_is_supported(DesignPanelCollection::LayoutGrid) {
            self.layout_grid_style_browser_open = false;
            self.layout_grid_count_variable_target = None;
        }
        if let Some(target) = self.active_effect_settings.as_mut() {
            let next_index = if target.effect_id.is_empty() {
                self.node.effects.get(target.index).map(|_| target.index)
            } else {
                self.node.effect_index_by_id(target.effect_id.as_ref())
            };
            if let Some(index) = next_index {
                target.index = index;
            } else {
                self.active_effect_settings = None;
            }
        }
        self.reconcile_layout_grid_targets();
        if self.node.typography.is_none() {
            self.typography_style_picker_open = false;
            self.font_browser_open = false;
        }
        if self.frame_preset_view_data_for_context().is_none() {
            self.frame_preset_browser_open = false;
        }
        cx.notify();
    }

    pub fn node(&self) -> &DesignPanelNode {
        &self.node
    }

    /// Enables Figma UI3's cross-file “Additional labels” preference.
    ///
    /// This is deliberately presentation-only state. It survives inspection
    /// context changes and does not emit a [`DesignPanelAction`].
    pub fn set_additional_labels(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.additional_labels == enabled {
            return;
        }
        self.additional_labels = enabled;
        cx.notify();
    }

    /// Whether compact inspector controls include their explanatory labels.
    pub const fn additional_labels(&self) -> bool {
        self.additional_labels
    }

    /// Echoes the host's cross-file small/big keyboard nudge preference.
    ///
    /// This preference survives selection changes and emits no document
    /// action. [`DesignNudgeSettings`] guarantees finite positive amounts.
    pub fn set_nudge_settings(&mut self, settings: DesignNudgeSettings, cx: &mut Context<Self>) {
        if self.nudge_settings == settings {
            return;
        }
        self.nudge_settings = settings;
        cx.notify();
    }

    pub const fn nudge_settings(&self) -> DesignNudgeSettings {
        self.nudge_settings
    }

    /// Returns the exhaustive right-sidebar surfaces valid for the current
    /// permission context.
    pub fn available_surfaces(&self) -> &'static [DesignPanelSurface] {
        DesignPanelSurface::available(self.inspection_context.permissions().can_edit())
    }

    /// Returns the last surface accepted by the host for the current
    /// permission context.
    pub const fn active_surface(&self) -> DesignPanelSurface {
        if self.inspection_context.permissions().can_edit() {
            self.editor_surface
        } else {
            self.viewer_surface
        }
    }

    /// Returns the host-controlled editable-workspace presentation.
    ///
    /// This value is independent from [`Self::active_surface`] and the
    /// inspection context's [`DesignPanelEditMode`].
    pub const fn workspace_mode(&self) -> DesignPanelWorkspaceMode {
        self.workspace_mode
    }

    /// Echoes one host-accepted Design/Draw workspace presentation.
    ///
    /// Switching workspace emits no document action and deliberately
    /// preserves the accepted Design/Prototype surface. Any phased or
    /// transient interaction whose controls are about to be replaced is
    /// balanced or dismissed before the projection changes.
    pub fn set_workspace_mode(
        &mut self,
        workspace_mode: DesignPanelWorkspaceMode,
        cx: &mut Context<Self>,
    ) {
        if self.workspace_mode == workspace_mode {
            return;
        }
        self.cancel_dimension_limits_preview(cx);
        self.cancel_host_interactions_for_context_change(true, cx);
        self.cancel_component_authoring_for_workspace_change(cx);
        self.close_editor_only_overlays();
        self.selection_header_overlay = None;
        self.workspace_mode = workspace_mode;
        self.scroll_handle.set_offset(gpui::point(px(0.), px(0.)));
        self.reset_scroll_after_render = true;
        cx.notify();
    }

    fn renders_draw_workspace(&self) -> bool {
        self.workspace_mode == DesignPanelWorkspaceMode::Draw
            && self.inspection_context.permissions().can_edit()
    }

    /// Echoes one host-accepted right-sidebar surface.
    ///
    /// Returns `false` without changing presentation when `surface` belongs
    /// to the other permission context. Tab activation never calls this
    /// setter directly; it emits [`DesignPanelAction::SurfaceChangeRequested`]
    /// and waits for its host.
    pub fn set_active_surface(
        &mut self,
        surface: DesignPanelSurface,
        cx: &mut Context<Self>,
    ) -> bool {
        let can_edit = self.inspection_context.permissions().can_edit();
        if !surface.is_available(can_edit) {
            return false;
        }
        if self.active_surface() == surface {
            return true;
        }
        self.cancel_dimension_limits_preview(cx);
        self.cancel_host_interactions_for_context_change(true, cx);
        self.cancel_component_authoring_for_workspace_change(cx);
        self.close_editor_only_overlays();
        self.selection_header_overlay = None;
        if can_edit {
            self.editor_surface = surface;
        } else {
            self.viewer_surface = surface;
        }
        self.scroll_handle.set_offset(gpui::point(px(0.), px(0.)));
        self.reset_scroll_after_render = true;
        cx.notify();
        true
    }

    /// Replaces the complete host-controlled inspection context.
    ///
    /// A multiple selection uses the first supplied node as its aggregate
    /// visual model. It should contain only capabilities and collection/type
    /// leaves with a valid aggregate identity, rather than a wholesale clone
    /// of one real target. Hosts supply uniform/mixed/unset/bound leaf states
    /// with [`Self::set_property_value_states`]. A page/no-selection context
    /// renders page-level controls and never emits document-property edits.
    pub fn set_inspection_context(
        &mut self,
        context: DesignPanelInspectionContext,
        cx: &mut Context<Self>,
    ) {
        let previous_selection_header_target =
            Self::selection_header_target_for_context(&self.inspection_context);
        let next_selection_header_target = Self::selection_header_target_for_context(&context);
        let selection_header_target_changed =
            previous_selection_header_target != next_selection_header_target;
        let previous_kind = self.inspection_context.selection().kind();
        let next_kind = context.selection().kind();
        let edit_mode_changed = self.inspection_context.edit_mode() != context.edit_mode();
        let text_range_changed =
            self.inspection_context.text_range_revision() != context.text_range_revision();
        let permissions_changed = self.inspection_context.permissions() != context.permissions();
        let entering_viewer =
            self.inspection_context.permissions().can_edit() && !context.permissions().can_edit();
        let next_node = context.selection().items().first().cloned();
        let node_changed = next_node
            .as_ref()
            .is_some_and(|node| node.id != self.node.id);
        self.selection_header_overlay = None;
        if selection_header_target_changed
            && self.selection_header_view_data_target.as_ref()
                != next_selection_header_target.as_ref()
        {
            self.selection_header_view_data = None;
            self.selection_header_view_data_target = None;
        }
        let interaction_target_changed = previous_kind != next_kind
            || node_changed
            || edit_mode_changed
            || text_range_changed
            || selection_header_target_changed;
        if self.active_menu_preview.is_some()
            && (interaction_target_changed
                || permissions_changed
                || next_node.as_ref().is_none_or(|node| *node != self.node))
        {
            self.cancel_menu_preview(cx);
        }
        let dimension_preview_invalidated =
            self.dimension_limits_preview
                .as_ref()
                .is_some_and(|preview| {
                    next_node.as_ref().is_none_or(|node| {
                        preview.node_id != node.id
                            || !Self::dimension_limits_are_applicable_for(node, &context)
                            || Self::dimension_limits_for_node(node, preview.axis)
                                != (preview.minimum, preview.maximum)
                    })
                });
        if interaction_target_changed || permissions_changed || dimension_preview_invalidated {
            self.cancel_dimension_limits_preview(cx);
        }
        if !interaction_target_changed
            && next_node.as_ref().is_some_and(|node| *node != self.node)
            && self
                .numeric_property_scrub
                .as_ref()
                .is_some_and(|scrub| !scrub.active)
        {
            self.numeric_property_scrub = None;
        }
        if interaction_target_changed || permissions_changed {
            self.cancel_host_interactions_for_context_change(interaction_target_changed, cx);
        }
        if permissions_changed {
            self.dimension_limit_fields_disclosed.clear();
            self.dimension_menu_open = None;
        }
        if entering_viewer {
            self.close_editor_only_overlays();
        }
        let rebased_property = if interaction_target_changed || permissions_changed {
            None
        } else {
            next_node.as_ref().and_then(|node| {
                self.cancel_interactions_invalidated_by_host_echo(node, &context, cx)
            })
        };
        if !interaction_target_changed && next_node.as_ref().is_some_and(|node| *node != self.node)
        {
            self.clear_option_interactions();
        }
        if interaction_target_changed {
            self.auxiliary_color_picker = None;
            if let Some(node) = &next_node {
                self.padding_editor_mode = PaddingEditorMode::for_node(node);
            }
            self.active_picker = None;
            self.active_effect_settings = None;
            self.paint_style_browser_open = None;
            self.effect_style_browser_open = false;
            self.property_variable_picker = None;
            self.component_property_variable_picker = None;
            self.component_swap_browser = None;
            self.component_swap_hovered = None;
            self.open_slot_limits = None;
            self.component_multiline_editor = None;
            self.layout_grid_style_browser_open = false;
            self.layout_grid_count_variable_target = None;
            self.frame_preset_browser_open = false;
            self.page_background_picker_open = false;
            self.variable_mode_browser_open = false;
            self.typography_style_picker_open = false;
            self.font_browser_open = false;
            if edit_mode_changed {
                self.selection_header_view_data = None;
                self.selection_header_view_data_target = None;
            }
            self.type_settings_open = false;
            self.type_settings_tab = TypographySettingsTab::Basics;
            self.grid_dimensions_picker = None;
            self.dimension_limit_fields_disclosed.clear();
            self.dimension_menu_open = None;
            self.constraints_expanded = true;
            self.appearance_blend_mode_open = false;
            self.appearance_corner_details_open = false;
            self.preview_option_menu_open = None;
            self.active_menu_preview = None;
            self.property_editor = None;
            self.numeric_property_scrub = None;
            self.variable_font_axis_editor = None;
            self.variable_font_axis_scrub = None;
            self.vector_edit_target_ids = None;
            self.property_editor_invalid = false;
            self.variable_font_axis_editor_invalid = false;
            self.suppress_property_input_change = false;
            self.suppress_next_control_activation = false;
            self.option_states.clear();
            self.option_subscriptions.clear();
            self.option_snapshots.clear();
            self.property_value_states.clear();
            self.media_paint_view_data = DesignMediaPaintViewData::default();
            self.paint_picker.update(cx, |picker, cx| {
                picker.set_media_view_data(DesignMediaPaintViewData::default(), cx);
            });
            self.export_view_data = None;
            self.expanded_export_settings.clear();
            self.export_choice_overlay = None;
            self.export_preview_expanded = false;
            self.scroll_handle.set_offset(gpui::point(px(0.), px(0.)));
            self.reset_scroll_after_render = true;
        }
        if let Some(node) = next_node {
            self.node = node;
        }
        self.inspection_context = context;
        self.reconcile_slot_limits_state();
        if !self.dimension_limits_are_applicable() {
            self.dimension_limit_fields_disclosed.clear();
            self.dimension_menu_open = None;
        }
        if let Some(property) = rebased_property {
            if let Some(previous) = self
                .property_editor
                .as_ref()
                .map(|editor| editor.property)
                .or_else(|| {
                    self.numeric_property_scrub
                        .as_ref()
                        .map(|scrub| scrub.property)
                })
            {
                self.rebase_editor_focus_origin(previous, property);
            }
            if let Some(editor) = self.property_editor.as_mut() {
                editor.property = property;
            }
            if let Some(scrub) = self.numeric_property_scrub.as_mut() {
                scrub.property = property;
            }
        }
        self.reconcile_layout_grid_targets();
        if self.frame_preset_view_data_for_context().is_none() {
            self.frame_preset_browser_open = false;
        }
        cx.notify();
    }

    pub const fn inspection_context(&self) -> &DesignPanelInspectionContext {
        &self.inspection_context
    }

    /// Supplies canonical host-owned export rows for the current command
    /// target. Rows are normalized at the presentation boundary so SVG and
    /// PDF cannot display or emit an unsupported custom size.
    pub fn set_export_view_data(
        &mut self,
        mut view_data: DesignExportViewData,
        cx: &mut Context<Self>,
    ) {
        for configuration in &mut view_data.configurations {
            *configuration = configuration.clone().normalized();
        }
        let ids = view_data
            .configurations
            .iter()
            .map(|configuration| configuration.id.clone())
            .collect::<HashSet<_>>();
        self.expanded_export_settings
            .retain(|configuration_id| ids.contains(configuration_id));
        if let Some(animated) = &mut view_data.animated {
            animated.settings = animated.settings.clone().normalized();
        } else {
            view_data.mode = DesignExportMode::Static;
        }
        if view_data.preview.is_none() {
            self.export_preview_expanded = false;
        }
        self.reconcile_export_property_editor(&view_data, cx);
        self.export_choice_overlay = None;
        self.export_view_data = Some(view_data);
        self.clear_option_interactions();
        cx.notify();
    }

    /// Clears canonical export input and falls back to adapting the legacy
    /// scale-only records on [`DesignPanelNode`].
    pub fn clear_export_view_data(&mut self, cx: &mut Context<Self>) {
        if self
            .property_editor
            .as_ref()
            .is_some_and(|editor| editor.export_configuration_id.is_some())
        {
            self.cancel_property_editor_transaction(cx);
        }
        self.export_view_data = None;
        self.expanded_export_settings.clear();
        self.export_choice_overlay = None;
        self.export_preview_expanded = false;
        self.clear_option_interactions();
        cx.notify();
    }

    /// Stores the compatibility-only leaf color-preset snapshot.
    ///
    /// The built-in picker no longer renders or emits this conflated surface.
    /// New hosts should use [`Self::set_paint_variable_view_data`] for leaf
    /// bindings and [`Self::set_paint_style_view_data`] for complete styles.
    pub fn set_color_style_view_data(
        &mut self,
        view_data: DesignColorStyleViewData,
        cx: &mut Context<Self>,
    ) {
        if self.color_style_view_data == view_data {
            return;
        }
        self.color_style_view_data = view_data;
        cx.notify();
    }

    pub const fn color_style_view_data(&self) -> &DesignColorStyleViewData {
        &self.color_style_view_data
    }

    /// Supplies sample-only Color-style values rendered beside Color
    /// variables in the retained picker.
    ///
    /// Sampling changes one color leaf; complete Fill/Stroke style identity
    /// remains exclusively controlled by [`Self::set_paint_style_view_data`].
    pub fn set_color_style_sample_view_data(
        &mut self,
        view_data: DesignColorStyleSampleViewData,
        cx: &mut Context<Self>,
    ) {
        if self.color_style_sample_view_data == view_data {
            return;
        }
        self.color_style_sample_view_data = view_data.clone();
        self.paint_picker.update(cx, |picker, cx| {
            picker.set_color_style_sample_view_data(view_data, cx);
        });
        cx.notify();
    }

    pub const fn color_style_sample_view_data(&self) -> &DesignColorStyleSampleViewData {
        &self.color_style_sample_view_data
    }

    /// Supplies exact host-computed color-contrast results for paint
    /// occurrences in the current inspection context.
    ///
    /// The host resolves the effective scene background and nearest compliant
    /// colors. The panel only chooses a transient WCAG category/level and
    /// emits an ordinary paint edit when the user accepts a supplied
    /// correction.
    pub fn set_color_contrast_view_data(
        &mut self,
        view_data: DesignColorContrastViewData,
        cx: &mut Context<Self>,
    ) {
        let view_data = DesignColorContrastViewData::new(view_data.paints);
        if self.color_contrast_view_data == view_data {
            return;
        }
        self.color_contrast_view_data = view_data;
        cx.notify();
    }

    pub const fn color_contrast_view_data(&self) -> &DesignColorContrastViewData {
        &self.color_contrast_view_data
    }

    /// Supplies host-filtered Color variables for solid-paint and
    /// gradient-stop bindings. Whole Paint styles use
    /// [`Self::set_paint_style_view_data`] instead.
    pub fn set_paint_variable_view_data(
        &mut self,
        view_data: DesignPaintVariableViewData,
        cx: &mut Context<Self>,
    ) {
        if self.paint_variable_view_data == view_data {
            return;
        }
        self.paint_variable_view_data = view_data.clone();
        self.paint_picker.update(cx, |picker, cx| {
            picker.set_paint_variable_view_data(view_data, cx);
        });
        cx.notify();
    }

    pub const fn paint_variable_view_data(&self) -> &DesignPaintVariableViewData {
        &self.paint_variable_view_data
    }

    /// Supplies whole-collection Fill/Stroke Paint styles.
    pub fn set_paint_style_view_data(
        &mut self,
        view_data: DesignPaintStyleViewData,
        cx: &mut Context<Self>,
    ) {
        if self.paint_style_view_data == view_data {
            return;
        }
        if self.paint_style_browser_open.is_some()
            || self
                .selection_color_resource_browser
                .as_ref()
                .is_some_and(|target| target.kind == SelectionColorResourceKind::PaintStyle)
        {
            self.style_browser_source_filter =
                self.style_browser_source_filter.normalized_for_libraries(
                    view_data
                        .libraries
                        .iter()
                        .map(|library| (&library.id, &library.name)),
                );
        }
        self.paint_style_view_data = view_data;
        cx.notify();
    }

    pub const fn paint_style_view_data(&self) -> &DesignPaintStyleViewData {
        &self.paint_style_view_data
    }

    /// Supplies host-controlled capabilities, crop-tool state, and video
    /// preview state for stable media paint occurrences.
    pub fn set_media_paint_view_data(
        &mut self,
        view_data: DesignMediaPaintViewData,
        cx: &mut Context<Self>,
    ) {
        if self.media_paint_view_data == view_data {
            return;
        }
        let active_target = self.active_picker.clone();
        let crop_interaction_became_unavailable = active_target.as_ref().is_some_and(|target| {
            let Some(index) = self.paint_target_index(target) else {
                return false;
            };
            let previous =
                self.media_paint_view_data
                    .paint(target.collection, &target.paint_id, index);
            let next = view_data.paint(target.collection, &target.paint_id, index);
            previous.is_some_and(|view| {
                view.crop_tool.active
                    && next.is_none_or(|next| {
                        !next.crop_tool.active || !next.capabilities.can_edit_properties
                    })
            })
        });
        let active_edit_became_unavailable =
            self.active_paint_edit.as_ref().is_some_and(|active| {
                if active.target.node_id != self.node.id {
                    return false;
                }
                let target = PaintPickerTarget {
                    collection: active.target.collection,
                    index: active.target.index,
                    paint_id: active.target.paint_id.clone(),
                };
                let Some(index) = self.paint_target_index(&target) else {
                    return false;
                };
                self.media_paint_view_data
                    .paint(target.collection, &target.paint_id, index)
                    .is_some_and(|previous| {
                        previous.capabilities.can_edit_properties
                            && view_data
                                .paint(target.collection, &target.paint_id, index)
                                .is_none_or(|next| !next.capabilities.can_edit_properties)
                    })
            });
        if crop_interaction_became_unavailable && let Some(target) = active_target.as_ref() {
            self.emit_crop_cancel_if_active(target, cx);
        }
        if active_edit_became_unavailable {
            self.prepare_paint_picker_for_dismissal(cx);
            self.cancel_active_paint_edit(cx);
        }
        self.media_paint_view_data = view_data.clone();
        self.paint_picker.update(cx, |picker, cx| {
            picker.set_media_view_data(view_data, cx);
        });
        cx.notify();
    }

    pub const fn media_paint_view_data(&self) -> &DesignMediaPaintViewData {
        &self.media_paint_view_data
    }

    /// Supplies host-owned imported/page shaders and discoverable library
    /// shaders. Import and apply remain explicit typed host intents.
    pub fn set_shader_view_data(
        &mut self,
        view_data: DesignShaderViewData,
        cx: &mut Context<Self>,
    ) {
        if self.shader_view_data == view_data {
            return;
        }
        self.shader_view_data = view_data.clone();
        self.paint_picker.update(cx, |picker, cx| {
            picker.set_shader_view_data(view_data, cx);
        });
        cx.notify();
    }

    pub const fn shader_view_data(&self) -> &DesignShaderViewData {
        &self.shader_view_data
    }

    /// Supplies the host-owned page text styles and grouped libraries shown
    /// by the retained Typography style picker.
    ///
    /// Applying or detaching a style emits a typed intent. Neither the panel
    /// nor the picker mutates the controlled typography snapshot.
    pub fn set_typography_style_view_data(
        &mut self,
        view_data: DesignTypographyStyleViewData,
        cx: &mut Context<Self>,
    ) {
        if self.typography_style_view_data == view_data {
            return;
        }
        self.typography_style_view_data = view_data.clone();
        self.typography_style_picker.update(cx, |picker, cx| {
            picker.set_view_data(view_data, cx);
        });
        cx.notify();
    }

    pub const fn typography_style_view_data(&self) -> &DesignTypographyStyleViewData {
        &self.typography_style_view_data
    }

    /// Supplies exact host-controlled font enumeration and loading state.
    pub fn set_font_view_data(&mut self, view_data: DesignFontViewData, cx: &mut Context<Self>) {
        if self.font_view_data == view_data {
            return;
        }
        self.font_view_data = view_data;
        cx.notify();
    }

    pub const fn font_view_data(&self) -> &DesignFontViewData {
        &self.font_view_data
    }

    /// Supplies controlled page/library Effect styles. Applying, creating, or
    /// detaching a style only emits a typed host intent.
    pub fn set_effect_style_view_data(
        &mut self,
        view_data: DesignEffectStyleViewData,
        cx: &mut Context<Self>,
    ) {
        if self.effect_style_view_data == view_data {
            return;
        }
        if self.effect_style_browser_open {
            self.style_browser_source_filter =
                self.style_browser_source_filter.normalized_for_libraries(
                    view_data
                        .libraries
                        .iter()
                        .map(|library| (&library.id, &library.name)),
                );
        }
        self.effect_style_view_data = view_data;
        cx.notify();
    }

    pub const fn effect_style_view_data(&self) -> &DesignEffectStyleViewData {
        &self.effect_style_view_data
    }

    /// Supplies exact variable candidates for the effect-variable affordances.
    pub fn set_effect_variable_view_data(
        &mut self,
        view_data: DesignEffectVariableViewData,
        cx: &mut Context<Self>,
    ) {
        if self.effect_variable_view_data == view_data {
            return;
        }
        self.effect_variable_view_data = view_data;
        cx.notify();
    }

    pub const fn effect_variable_view_data(&self) -> &DesignEffectVariableViewData {
        &self.effect_variable_view_data
    }

    /// Supplies the complete page/library variable catalog used by generic
    /// node and text property pickers. Search and open-popover state remain
    /// transient; apply/import/detach only emit host intents.
    pub fn set_property_variable_view_data(
        &mut self,
        view_data: DesignVariableViewData,
        cx: &mut Context<Self>,
    ) {
        if self.property_variable_view_data == view_data {
            return;
        }
        self.property_variable_view_data = view_data;
        cx.notify();
    }

    pub const fn property_variable_view_data(&self) -> &DesignVariableViewData {
        &self.property_variable_view_data
    }

    /// Supplies the complete local/library component catalog used by
    /// instance-swap properties. Open/search/hover state stays transient.
    pub fn set_component_swap_view_data(
        &mut self,
        view_data: DesignComponentSwapViewData,
        cx: &mut Context<Self>,
    ) {
        if self.component_swap_view_data == view_data {
            return;
        }
        let hovered_candidate_survives =
            self.component_swap_hovered
                .as_ref()
                .is_none_or(|(_, selection)| {
                    view_data
                        .candidate(selection)
                        .is_some_and(DesignComponentSwapCandidate::can_apply)
                });
        if !hovered_candidate_survives {
            self.cancel_component_swap_preview(cx);
        }
        self.component_swap_view_data = view_data;
        cx.notify();
    }

    pub const fn component_swap_view_data(&self) -> &DesignComponentSwapViewData {
        &self.component_swap_view_data
    }

    /// Supplies page and library Grid styles for the Layout guides header
    /// browser. The panel never imports or applies a style itself.
    pub fn set_layout_grid_style_view_data(
        &mut self,
        view_data: DesignLayoutGridStyleViewData,
        cx: &mut Context<Self>,
    ) {
        if self.layout_grid_style_view_data == view_data {
            return;
        }
        if self.layout_grid_style_browser_open {
            self.style_browser_source_filter =
                self.style_browser_source_filter.normalized_for_libraries(
                    view_data
                        .libraries
                        .iter()
                        .map(|library| (&library.id, &library.name)),
                );
        }
        self.layout_grid_style_view_data = view_data;
        cx.notify();
    }

    pub const fn layout_grid_style_view_data(&self) -> &DesignLayoutGridStyleViewData {
        &self.layout_grid_style_view_data
    }

    /// Supplies the host-controlled Number-variable catalog shared by every
    /// supported layout-guide numeric leaf.
    pub fn set_layout_grid_variable_view_data(
        &mut self,
        view_data: DesignLayoutGridVariableViewData,
        cx: &mut Context<Self>,
    ) {
        if self.layout_grid_variable_view_data == view_data {
            return;
        }
        self.layout_grid_variable_view_data = view_data;
        cx.notify();
    }

    pub const fn layout_grid_variable_view_data(&self) -> &DesignLayoutGridVariableViewData {
        &self.layout_grid_variable_view_data
    }

    /// Compatibility adapter for the former count-only variable catalog.
    #[deprecated(note = "use set_layout_grid_variable_view_data")]
    pub fn set_layout_grid_count_variable_view_data(
        &mut self,
        view_data: DesignLayoutGridCountVariableViewData,
        cx: &mut Context<Self>,
    ) {
        let generalized = view_data.generalized();
        if self.layout_grid_count_variable_view_data == view_data
            && self.layout_grid_variable_view_data == generalized
        {
            return;
        }
        self.layout_grid_count_variable_view_data = view_data;
        self.layout_grid_variable_view_data = generalized;
        cx.notify();
    }

    #[deprecated(note = "use layout_grid_variable_view_data")]
    pub const fn layout_grid_count_variable_view_data(
        &self,
    ) -> &DesignLayoutGridCountVariableViewData {
        &self.layout_grid_count_variable_view_data
    }

    /// Supplies host-resolved structural availability for “Add auto layout”.
    ///
    /// The projection is bound to an exact ordered node target. A late result
    /// for a previous selection may remain stored, but it cannot render or
    /// emit until that same target is current again.
    pub fn set_add_auto_layout_view_data(
        &mut self,
        view_data: DesignAddAutoLayoutViewData,
        cx: &mut Context<Self>,
    ) {
        if self.add_auto_layout_view_data.as_ref() == Some(&view_data) {
            return;
        }
        self.add_auto_layout_view_data = Some(view_data);
        cx.notify();
    }

    pub fn clear_add_auto_layout_view_data(&mut self, cx: &mut Context<Self>) {
        if self.add_auto_layout_view_data.take().is_some() {
            cx.notify();
        }
    }

    pub const fn add_auto_layout_view_data(&self) -> Option<&DesignAddAutoLayoutViewData> {
        self.add_auto_layout_view_data.as_ref()
    }

    /// Supplies the target-bound corner-radius slider policy for Draw.
    ///
    /// Opacity needs no host data because it is always a percentage. Invalid
    /// Page/empty/duplicate targets are rejected without replacing the last
    /// valid projection.
    pub fn set_draw_appearance_view_data(
        &mut self,
        view_data: DesignDrawAppearanceViewData,
        cx: &mut Context<Self>,
    ) -> bool {
        if !view_data.is_valid() {
            return false;
        }
        if self.draw_appearance_view_data.as_ref() == Some(&view_data) {
            return true;
        }
        if self.draw_appearance_slider_property == Some(DesignPanelProperty::CornerRadius) {
            self.cancel_property_editor_transaction(cx);
        }
        self.draw_appearance_view_data = Some(view_data);
        self.rebuild_draw_corner_radius_slider(cx);
        cx.notify();
        true
    }

    pub fn clear_draw_appearance_view_data(&mut self, cx: &mut Context<Self>) {
        if self.draw_appearance_view_data.is_none() {
            return;
        }
        if self.draw_appearance_slider_property == Some(DesignPanelProperty::CornerRadius) {
            self.cancel_property_editor_transaction(cx);
        }
        self.draw_appearance_view_data = None;
        self.rebuild_draw_corner_radius_slider(cx);
        cx.notify();
    }

    pub const fn draw_appearance_view_data(&self) -> Option<&DesignDrawAppearanceViewData> {
        self.draw_appearance_view_data.as_ref()
    }

    /// Supplies the exact ordered Frame-preset catalog for one selected Frame.
    ///
    /// The catalog is target-bound, so a late result for a previous selection
    /// stays inert. Opening and closing the grouped chooser is transient; an
    /// enabled leaf emits [`DesignPanelAction::FramePresetApplyRequested`].
    pub fn set_frame_preset_view_data(
        &mut self,
        view_data: DesignFramePresetViewData,
        cx: &mut Context<Self>,
    ) {
        if self.frame_preset_view_data.as_ref() == Some(&view_data) {
            return;
        }
        let target_changed = self
            .frame_preset_view_data
            .as_ref()
            .is_some_and(|current| current.target_node_id != view_data.target_node_id);
        let group_ids = view_data
            .groups
            .iter()
            .map(|group| group.id.clone())
            .collect::<HashSet<_>>();
        self.frame_preset_view_data = Some(view_data);
        if target_changed {
            self.collapsed_frame_preset_groups.clear();
        } else {
            self.collapsed_frame_preset_groups
                .retain(|group_id| group_ids.contains(group_id));
        }
        if target_changed || self.frame_preset_view_data_for_context().is_none() {
            self.frame_preset_browser_open = false;
        }
        cx.notify();
    }

    pub fn clear_frame_preset_view_data(&mut self, cx: &mut Context<Self>) {
        if self.frame_preset_view_data.take().is_some() {
            self.frame_preset_browser_open = false;
            self.collapsed_frame_preset_groups.clear();
            cx.notify();
        }
    }

    pub const fn frame_preset_view_data(&self) -> Option<&DesignFramePresetViewData> {
        self.frame_preset_view_data.as_ref()
    }

    /// Supplies Smart Selection geometry and command availability bound to an
    /// exact ordered multiple-selection target.
    ///
    /// A changed host echo terminates an in-progress spacing draft before the
    /// new snapshot is installed. A stale target may remain stored, but it
    /// cannot render or emit against another selection.
    pub fn set_smart_selection_view_data(
        &mut self,
        view_data: DesignSmartSelectionViewData,
        cx: &mut Context<Self>,
    ) {
        if self.smart_selection_view_data.as_ref() == Some(&view_data) {
            return;
        }
        if self
            .property_editor
            .as_ref()
            .is_some_and(|editor| Self::smart_selection_axis(editor.property).is_some())
        {
            self.cancel_property_editor_transaction(cx);
        }
        self.smart_selection_view_data = Some(view_data);
        cx.notify();
    }

    pub fn clear_smart_selection_view_data(&mut self, cx: &mut Context<Self>) {
        if self.smart_selection_view_data.is_none() {
            return;
        }
        if self
            .property_editor
            .as_ref()
            .is_some_and(|editor| Self::smart_selection_axis(editor.property).is_some())
        {
            self.cancel_property_editor_transaction(cx);
        }
        self.smart_selection_view_data = None;
        cx.notify();
    }

    pub const fn smart_selection_view_data(&self) -> Option<&DesignSmartSelectionViewData> {
        self.smart_selection_view_data.as_ref()
    }

    /// Supplies the host-owned Page/no-selection background and resource
    /// catalog. The panel keeps only picker/browser presentation state.
    pub fn set_page_view_data(&mut self, view_data: DesignPageViewData, cx: &mut Context<Self>) {
        if self.page_view_data.as_ref() == Some(&view_data) {
            return;
        }
        let page_changed = self
            .page_view_data
            .as_ref()
            .is_some_and(|current| current.page_id != view_data.page_id);
        let active_background_became_read_only = !page_changed
            && view_data.background.read_only
            && self.auxiliary_color_picker.as_ref().is_some_and(|target| {
                matches!(
                    target,
                    AuxiliaryColorPickerTarget::PageBackground { page_id }
                        if *page_id == view_data.page_id
                )
            });
        if page_changed {
            self.cancel_host_interactions_for_context_change(true, cx);
            self.auxiliary_color_picker = None;
        } else if active_background_became_read_only {
            self.prepare_paint_picker_for_dismissal(cx);
            self.cancel_active_paint_edit(cx);
        }
        self.page_view_data = Some(view_data);
        if page_changed {
            self.page_background_picker_open = false;
            self.collapsed_local_style_folders.clear();
        }
        cx.notify();
    }

    pub fn clear_page_view_data(&mut self, cx: &mut Context<Self>) {
        if self.page_view_data.is_some() {
            self.cancel_host_interactions_for_context_change(true, cx);
            self.auxiliary_color_picker = None;
        }
        if self.page_view_data.take().is_some() {
            self.page_background_picker_open = false;
            self.collapsed_local_style_folders.clear();
            cx.notify();
        }
    }

    pub const fn page_view_data(&self) -> Option<&DesignPageViewData> {
        self.page_view_data.as_ref()
    }

    /// Supplies the exact current-file local-style tree for one Page.
    ///
    /// Invalid or stale snapshots remain inspectable through the getter but
    /// never render or emit. Hosts can therefore replace Page/context data in
    /// either order without a transient command targeting the wrong file.
    pub fn set_page_local_styles_view_data(
        &mut self,
        view_data: DesignPageLocalStylesViewData,
        cx: &mut Context<Self>,
    ) {
        if self.page_local_styles_view_data.as_ref() == Some(&view_data) {
            return;
        }
        let target_changed = self
            .page_local_styles_view_data
            .as_ref()
            .is_some_and(|current| current.target != view_data.target);
        if target_changed {
            self.collapsed_local_style_folders.clear();
        } else {
            fn collect_folder_ids(
                entries: &[DesignLocalStyleEntry],
                ids: &mut HashSet<SharedString>,
            ) {
                for entry in entries {
                    if let DesignLocalStyleEntry::Folder {
                        id,
                        entries: children,
                        ..
                    } = entry
                    {
                        ids.insert(id.clone());
                        collect_folder_ids(children, ids);
                    }
                }
            }
            let mut current_folder_ids = HashSet::new();
            for section in &view_data.sections {
                collect_folder_ids(&section.entries, &mut current_folder_ids);
            }
            self.collapsed_local_style_folders
                .retain(|id| current_folder_ids.contains(id));
        }
        self.page_local_styles_view_data = Some(view_data);
        cx.notify();
    }

    pub fn clear_page_local_styles_view_data(&mut self, cx: &mut Context<Self>) {
        if self.page_local_styles_view_data.take().is_some() {
            self.collapsed_local_style_folders.clear();
            cx.notify();
        }
    }

    pub const fn page_local_styles_view_data(&self) -> Option<&DesignPageLocalStylesViewData> {
        self.page_local_styles_view_data.as_ref()
    }

    /// Selects whether the compatibility Variables navigation row is visible.
    /// The default is [`DesignVariablesEntryPoint::NavigationBarOnly`].
    pub fn set_variables_entry_point(
        &mut self,
        entry_point: DesignVariablesEntryPoint,
        cx: &mut Context<Self>,
    ) {
        if self.variables_entry_point == entry_point {
            return;
        }
        self.variables_entry_point = entry_point;
        cx.notify();
    }

    pub const fn variables_entry_point(&self) -> &DesignVariablesEntryPoint {
        &self.variables_entry_point
    }

    /// Supplies the resolved and explicit mode projection for the current page
    /// or one selected scene node.
    pub fn set_variable_mode_view_data(
        &mut self,
        view_data: DesignVariableModeViewData,
        cx: &mut Context<Self>,
    ) {
        if self.variable_mode_view_data.as_ref() == Some(&view_data) {
            return;
        }
        let target_changed = self
            .variable_mode_view_data
            .as_ref()
            .is_some_and(|current| current.target != view_data.target);
        self.variable_mode_view_data = Some(view_data);
        if target_changed {
            self.variable_mode_browser_open = false;
        }
        cx.notify();
    }

    pub fn clear_variable_mode_view_data(&mut self, cx: &mut Context<Self>) {
        if self.variable_mode_view_data.take().is_some() {
            self.variable_mode_browser_open = false;
            cx.notify();
        }
    }

    pub const fn variable_mode_view_data(&self) -> Option<&DesignVariableModeViewData> {
        self.variable_mode_view_data.as_ref()
    }

    /// Supplies the exact view-only Properties projection for one ordered
    /// selection target.
    ///
    /// Content, section summaries, copy payloads, and Borders representation
    /// stay host owned. A snapshot for an older selection may be stored, but
    /// it cannot render or emit while its exact target is not current.
    pub fn set_viewer_properties_view_data(
        &mut self,
        view_data: DesignViewerPropertiesViewData,
        cx: &mut Context<Self>,
    ) {
        if self.viewer_properties_view_data.as_ref() == Some(&view_data) {
            return;
        }
        self.viewer_properties_view_data = Some(view_data);
        cx.notify();
    }

    pub fn clear_viewer_properties_view_data(&mut self, cx: &mut Context<Self>) {
        if self.viewer_properties_view_data.take().is_some() {
            cx.notify();
        }
    }

    pub const fn viewer_properties_view_data(&self) -> Option<&DesignViewerPropertiesViewData> {
        self.viewer_properties_view_data.as_ref()
    }

    fn selection_header_target_for_context(
        context: &DesignPanelInspectionContext,
    ) -> Option<DesignPanelTarget> {
        (!context.selection().is_empty()).then(|| DesignPanelTarget::Nodes {
            node_ids: context
                .selection()
                .items()
                .iter()
                .map(|node| node.id.clone())
                .collect(),
        })
    }

    fn current_selection_header_target(&self) -> Option<DesignPanelTarget> {
        Self::selection_header_target_for_context(&self.inspection_context)
    }

    fn selection_header_view_data_for_context(&self) -> Option<&DesignSelectionHeaderViewData> {
        let target = self.current_selection_header_target()?;
        if self.selection_header_view_data_target.as_ref() == Some(&target) {
            self.selection_header_view_data.as_ref()
        } else {
            None
        }
    }

    /// Supplies the complete ordered selected-node header model.
    ///
    /// This call binds the data to the current selection's exact ordered node
    /// IDs. It remains authoritative only while that target is unchanged,
    /// including its cardinality. The panel keeps only which
    /// title/control/More popover is open; every leaf activation emits
    /// [`DesignPanelAction::SelectionHeaderCommandRequested`].
    pub fn set_selection_header_view_data(
        &mut self,
        view_data: DesignSelectionHeaderViewData,
        cx: &mut Context<Self>,
    ) {
        if let Some(target) = self.current_selection_header_target() {
            self.set_selection_header_view_data_for_target(target, view_data, cx);
        } else {
            self.clear_selection_header_view_data(cx);
        }
    }

    /// Supplies selected-node header data bound to an explicit command target.
    ///
    /// Prefer this setter when header data is resolved asynchronously. A
    /// response for an older target can be stored but cannot render or emit
    /// while another selection is active. `set_inspection_context` also
    /// preserves data pre-bound to the incoming target.
    pub fn set_selection_header_view_data_for_target(
        &mut self,
        target: DesignPanelTarget,
        view_data: DesignSelectionHeaderViewData,
        cx: &mut Context<Self>,
    ) {
        if self.selection_header_view_data.as_ref() == Some(&view_data)
            && self.selection_header_view_data_target.as_ref() == Some(&target)
        {
            return;
        }
        self.selection_header_view_data = Some(view_data);
        self.selection_header_view_data_target = Some(target);
        self.selection_header_overlay = None;
        cx.notify();
    }

    /// Clears host-owned header data and restores the compatibility preset.
    ///
    /// A single selection uses its kind preset. A multiple selection uses the
    /// kind-neutral aggregate preset and never inherits the first member's
    /// title menu or commands. New integrations should normally keep
    /// supplying explicit target-bound data.
    pub fn clear_selection_header_view_data(&mut self, cx: &mut Context<Self>) {
        let had_view_data = self.selection_header_view_data.take().is_some();
        let had_target = self.selection_header_view_data_target.take().is_some();
        let had_overlay = self.selection_header_overlay.take().is_some();
        if had_view_data || had_target || had_overlay {
            cx.notify();
        }
    }

    pub fn selection_header_view_data(&self) -> Option<&DesignSelectionHeaderViewData> {
        self.selection_header_view_data_for_context()
    }

    fn resolved_selection_header_view_data(&self) -> DesignSelectionHeaderViewData {
        self.selection_header_view_data_for_context()
            .cloned()
            .unwrap_or_else(|| match self.inspection_context.selection().kind() {
                DesignPanelSelectionKind::None => {
                    DesignSelectionHeaderViewData::new("Page", std::iter::empty())
                }
                DesignPanelSelectionKind::Single => {
                    DesignSelectionHeaderViewData::for_node_kind(self.node.kind)
                }
                DesignPanelSelectionKind::Multiple => {
                    DesignSelectionHeaderViewData::for_multiple_selection(
                        self.inspection_context.selection().len(),
                    )
                }
            })
    }

    /// Supplies exact host-resolved display states for individual properties.
    ///
    /// States not present here fall back to the uniform value in `node`.
    pub fn set_property_value_states(
        &mut self,
        states: impl IntoIterator<
            Item = (
                DesignPanelProperty,
                DesignPanelPropertyValueState<DesignPanelValue>,
            ),
        >,
        cx: &mut Context<Self>,
    ) {
        let next_states: HashMap<_, _> = states.into_iter().collect();
        if self.active_menu_preview.is_some() && self.property_value_states != next_states {
            self.cancel_menu_preview(cx);
        }
        if self
            .numeric_property_scrub
            .as_ref()
            .is_some_and(|scrub| !scrub.active)
        {
            self.numeric_property_scrub = None;
        }
        let active_editor_became_non_editable =
            self.property_editor.as_ref().is_some_and(|editor| {
                next_states
                    .get(&editor.property)
                    .is_some_and(|state| state.is_read_only() || state.binding().is_some())
            });
        let active_auxiliary_became_non_editable = self
            .active_auxiliary_state_property()
            .and_then(|property| next_states.get(&property))
            .is_some_and(|state| state.is_read_only() || state.binding().is_some());
        if active_editor_became_non_editable {
            self.cancel_property_editor_transaction(cx);
        }
        if active_auxiliary_became_non_editable {
            self.prepare_paint_picker_for_dismissal(cx);
            self.cancel_active_paint_edit(cx);
        }
        self.property_value_states = next_states;
        self.option_snapshots.clear();
        cx.notify();
    }

    pub fn set_property_value_state(
        &mut self,
        property: DesignPanelProperty,
        state: DesignPanelPropertyValueState<DesignPanelValue>,
        cx: &mut Context<Self>,
    ) {
        if self.active_menu_preview.as_ref().is_some_and(|preview| {
            matches!(
                preview,
                DesignMenuPreview::NodeProperty {
                    property: active_property,
                    ..
                } | DesignMenuPreview::EffectProperty {
                    property: active_property,
                    ..
                } if *active_property == property
            )
        }) {
            self.cancel_menu_preview(cx);
        }
        if self
            .numeric_property_scrub
            .as_ref()
            .is_some_and(|scrub| !scrub.active && scrub.property == property)
        {
            self.numeric_property_scrub = None;
        }
        if self
            .property_editor
            .as_ref()
            .is_some_and(|editor| editor.property == property)
            && (state.is_read_only() || state.binding().is_some())
        {
            self.cancel_property_editor_transaction(cx);
        }
        if (state.is_read_only() || state.binding().is_some())
            && self.active_auxiliary_state_property() == Some(property)
        {
            self.prepare_paint_picker_for_dismissal(cx);
            self.cancel_active_paint_edit(cx);
        }
        self.property_value_states.insert(property, state);
        self.option_snapshots.remove(&property);
        cx.notify();
    }

    fn command_target(&self) -> DesignPanelTarget {
        match self.inspection_context.selection().kind() {
            DesignPanelSelectionKind::None => DesignPanelTarget::Page {
                page_id: self
                    .page_view_data_for_context()
                    .map_or_else(|| self.node.id.clone(), |page| page.page_id.clone()),
            },
            DesignPanelSelectionKind::Single | DesignPanelSelectionKind::Multiple => {
                DesignPanelTarget::Nodes {
                    node_ids: self
                        .inspection_context
                        .selection()
                        .items()
                        .iter()
                        .map(|node| node.id.clone())
                        .collect(),
                }
            }
        }
    }

    fn toggle_section(&mut self, section: DesignPanelSection, cx: &mut Context<Self>) {
        if !self.expanded_sections.remove(&section) {
            self.expanded_sections.insert(section);
        }
        cx.notify();
    }

    fn render_section_header(
        &self,
        section: DesignPanelSection,
        add: Option<DesignPanelCollection>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let can_edit = self.can_edit();
        let section_empty = match section {
            DesignPanelSection::Fill => self.node.fills.is_empty(),
            DesignPanelSection::Stroke => self
                .node
                .stroke
                .as_ref()
                .is_none_or(|stroke| stroke.paints.is_empty()),
            DesignPanelSection::Effects => self.node.effects.is_empty(),
            DesignPanelSection::Export => self
                .export_view_data
                .as_ref()
                .is_none_or(|view_data| view_data.configurations.is_empty()),
            _ => false,
        };
        let can_add = add.is_some_and(|collection| {
            if !self.collection_is_supported(collection)
                || !self
                    .node
                    .paint_collection_edit_mode(collection)
                    .allows_full_controls()
            {
                false
            } else if collection == DesignPanelCollection::Export {
                self.can_export()
            } else if collection == DesignPanelCollection::Effect {
                can_edit
                    && self.node.effect_style_binding.is_none()
                    && self.node.first_addable_effect_kind().is_some()
            } else if collection == DesignPanelCollection::LayoutGrid {
                can_edit && self.node.layout_grid_style_binding.is_none()
            } else if matches!(
                collection,
                DesignPanelCollection::Fill | DesignPanelCollection::Stroke
            ) {
                can_edit && self.paint_style_binding(collection).is_none()
            } else {
                can_edit
            }
        });
        let id = SharedString::from(format!(
            "{}-section-{}",
            self.id,
            section.label().to_lowercase().replace(' ', "-")
        ));
        h_flex()
            .id(id)
            .key_context(CONTROL_KEY_CONTEXT)
            .tab_index(0)
            .h(px(40.))
            .w_full()
            .pl(px(PANEL_PADDING))
            .pr_2()
            .gap_1()
            .cursor_pointer()
            .hover(|style| style.bg(cx.theme().sidebar_accent.opacity(0.45)))
            .focus(|style| {
                style
                    .bg(cx.theme().sidebar_accent.opacity(0.45))
                    .border_color(cx.theme().selection)
            })
            .on_activate(cx.listener(move |this, _, _, cx| {
                this.toggle_section(section, cx);
            }))
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .font_semibold()
                    .when(section_empty, |title| {
                        title.text_color(cx.theme().muted_foreground)
                    })
                    .child(section.label()),
            )
            .when(section == DesignPanelSection::Typography, |header| {
                header.child(self.render_typography_style_button(cx))
            })
            .when(section == DesignPanelSection::Effects, |header| {
                header.child(self.render_effect_style_button(cx))
            })
            .when(
                section == DesignPanelSection::LayoutGrid
                    && (!self.node.layout_grids.is_empty()
                        || self.node.layout_grid_style_binding.is_some()),
                |header| header.child(self.render_layout_grid_style_button(cx)),
            )
            .when(
                matches!(
                    section,
                    DesignPanelSection::Fill | DesignPanelSection::Stroke
                ) && add.is_some_and(|collection| self.collection_is_supported(collection)),
                |header| {
                    header.child(self.render_paint_style_button(
                        add.expect("paint section has a collection"),
                        cx,
                    ))
                },
            )
            .when_some(add.filter(|_| can_add), |header, collection| {
                let add_selector = format!(
                    "{}-add-{}",
                    self.id,
                    collection.label().to_lowercase().replace(' ', "-")
                );
                header.child(
                    div()
                        .id(SharedString::from(add_selector.clone()))
                        .debug_selector(move || add_selector.clone())
                        .key_context(CONTROL_KEY_CONTEXT)
                        .tab_index(0)
                        .size(px(24.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(section_empty, |button| {
                            button.text_color(cx.theme().muted_foreground)
                        })
                        .rounded(px(4.))
                        .hover(|style| style.bg(cx.theme().accent))
                        .focus(|style| {
                            style
                                .bg(cx.theme().accent)
                                .border_color(cx.theme().selection)
                        })
                        .on_activate(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.emit_add(collection, cx);
                        }))
                        .child(Icon::new(IconName::Plus).xsmall()),
                )
            })
            .into_any_element()
    }

    fn render_section(
        &self,
        section: DesignPanelSection,
        add: Option<DesignPanelCollection>,
        content: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let expanded = self.expanded_sections.contains(&section);
        v_flex()
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(cx.theme().sidebar_border)
            .child(self.render_section_header(section, add, cx))
            .when(expanded, |section| section.child(content))
            .into_any_element()
    }

    fn render_media(&self, _cx: &mut Context<Self>) -> Option<AnyElement> {
        // Compatibility-only section identifier. Image/video controls are
        // rendered by the canonical paint picker, so the panel must not emit
        // node-level media intents from this retired path.
        None
    }

    fn renders_inspector_projection(&self) -> bool {
        if self.renders_draw_workspace() {
            return true;
        }
        matches!(
            (
                self.inspection_context.permissions().can_edit(),
                self.active_surface(),
            ),
            (true, DesignPanelSurface::Design) | (false, DesignPanelSurface::Properties)
        )
    }

    fn render_host_owned_surface(&self, cx: &mut Context<Self>) -> AnyElement {
        let surface = self.active_surface();
        let selector = format!("{}-surface-{}-host-owned", self.id, surface.slug());
        div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .w_full()
            .p_4()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(format!(
                "{} is a host-owned surface. DesignPanel retains the shared sidebar header and does not project Design or Properties content here.",
                surface.label()
            ))
            .into_any_element()
    }
}

impl Focusable for DesignPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DesignPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.component_authoring_dialog_close_pending {
            self.component_authoring_dialog_close_pending = false;
            window.on_next_frame(|window, cx| {
                if window.has_active_dialog(cx) {
                    window.close_dialog(cx);
                }
            });
        }
        if self.reset_scroll_after_render {
            self.reset_scroll_after_render = false;
            let scroll_handle = self.scroll_handle.clone();
            window.on_next_frame(move |window, _| {
                scroll_handle.set_offset(gpui::point(px(0.), px(0.)));
                window.refresh();
            });
        }
        let selection_kind = self.inspection_context.selection().kind();
        let renders_inspector_projection = self.renders_inspector_projection();
        if self.renders_draw_workspace() {
            self.sync_draw_appearance_sliders(window, cx);
        }
        if renders_inspector_projection
            && (selection_kind != DesignPanelSelectionKind::None || self.can_export())
        {
            self.sync_option_states(window, cx);
        }
        let viewer_projection = !self.inspection_context.permissions().can_edit();
        if renders_inspector_projection && !viewer_projection {
            self.sync_typography_style_picker(cx);
            self.sync_paint_picker(window, cx);
        }
        let mut body = v_flex().w_full();
        if !renders_inspector_projection {
            body = body.child(self.render_host_owned_surface(cx));
        } else if selection_kind == DesignPanelSelectionKind::None {
            body = body.child(self.render_page_context(cx));
            if self.can_export() {
                body = body.child(self.render_export(cx));
            }
        } else if viewer_projection {
            body = body.child(self.render_viewer_properties(cx));
            if self.can_export() {
                body = body.child(self.render_export(cx));
            }
        } else {
            for section in resolve_design_panel_sections_with_export(&self.node, self.can_export())
                .into_iter()
                .filter(|section| {
                    design_panel_section_is_visible_in_workspace(*section, self.workspace_mode)
                })
            {
                let rendered = match section {
                    DesignPanelSection::Selection => self.render_selection_colors(cx),
                    DesignPanelSection::Component | DesignPanelSection::Instance => {
                        self.render_component(cx)
                    }
                    DesignPanelSection::Position => Some(self.render_position(cx)),
                    DesignPanelSection::Layout => Some(self.render_layout(cx)),
                    DesignPanelSection::Constraints => Some(self.render_constraints(cx)),
                    DesignPanelSection::Layer => Some(self.render_layer(cx)),
                    DesignPanelSection::Section => self.render_section_properties(cx),
                    DesignPanelSection::Transform => self.render_transform_modifiers(cx),
                    DesignPanelSection::Geometry => self.render_geometry(cx),
                    DesignPanelSection::Mask => self.render_mask(cx),
                    DesignPanelSection::Typography => self.render_typography(cx),
                    DesignPanelSection::Media => self.render_media(cx),
                    DesignPanelSection::Fill => self.render_fill(cx),
                    DesignPanelSection::Stroke => self.render_stroke(cx),
                    DesignPanelSection::Effects => Some(self.render_effects(cx)),
                    DesignPanelSection::LayoutGrid => self.render_layout_grids(cx),
                    DesignPanelSection::Export => Some(self.render_export(cx)),
                };
                if let Some(rendered) = rendered {
                    body = body.child(rendered);
                }
            }
        }

        let scroll_handle = self.scroll_handle.clone();
        v_flex()
            .id(self.id.clone())
            .key_context(DESIGN_PANEL_KEY_CONTEXT)
            .relative()
            .size_full()
            .min_h(px(0.))
            .track_focus(&self.focus_handle)
            .on_action(
                cx.listener(|this, _: &CancelDesignInteraction, window, cx| {
                    if this.component_authoring_name_editor.is_some() {
                        this.finish_component_authoring_name_edit(false, window, cx);
                    } else if this.component_property_reorder.is_some() {
                        this.finish_component_property_reorder(false, cx);
                    } else if this.component_variant_option_reorder.is_some() {
                        this.finish_component_variant_option_reorder(false, cx);
                    } else if this.component_property_create_draft.is_some() {
                        this.cancel_component_property_create(window, cx);
                    } else if this.component_property_context_menu.take().is_some()
                        || this.component_property_edit_modal.take().is_some()
                        || this.component_property_create_menu_open
                    {
                        this.component_property_create_menu_open = false;
                        this.close_component_authoring_dialog(window, cx);
                        this.focus_handle.focus(window, cx);
                        cx.notify();
                    } else if let Some(property) = this.draw_appearance_slider_property {
                        this.finish_draw_appearance_slider(property, false, cx);
                    } else if this.variable_font_axis_editor.is_some() {
                        this.finish_variable_font_axis_edit(false, window, cx);
                    } else if this.variable_font_axis_scrub.is_some() {
                        this.variable_font_axis_scrub = None;
                        cx.notify();
                    } else if this.property_editor.is_some() {
                        this.finish_property_edit(false, window, cx);
                    } else if this.component_multiline_editor.is_some() {
                        this.finish_component_multiline_editor(false, window, cx);
                    } else if this.auxiliary_color_picker.is_some() {
                        this.prepare_paint_picker_for_dismissal(cx);
                        this.cancel_active_paint_edit(cx);
                        this.auxiliary_color_picker = None;
                        this.page_background_picker_open = false;
                        cx.notify();
                    } else if this.page_background_picker_open {
                        this.page_background_picker_open = false;
                        cx.notify();
                    } else if this.page_resource_browser.take().is_some() {
                        cx.notify();
                    } else if this.variable_mode_browser_open {
                        this.variable_mode_browser_open = false;
                        cx.notify();
                    } else if this.frame_preset_browser_open {
                        this.frame_preset_browser_open = false;
                        cx.notify();
                    } else if this.property_variable_picker.take().is_some()
                        || this.component_property_variable_picker.take().is_some()
                    {
                        cx.notify();
                    } else if let Some(property_id) = this.component_swap_browser.take() {
                        this.clear_component_swap_preview(property_id.as_ref(), cx);
                        cx.notify();
                    } else if this.typography_style_picker_open {
                        this.typography_style_picker_open = false;
                        cx.notify();
                    } else if this.font_browser_open {
                        this.font_browser_open = false;
                        cx.notify();
                    } else if this.selection_header_overlay.take().is_some()
                        || this.paint_style_browser_open.take().is_some()
                    {
                        cx.notify();
                    } else if this.active_menu_preview.is_some()
                        || this.preview_option_menu_open.take().is_some()
                        || this.appearance_blend_mode_open
                        || this.dimension_menu_open.is_some()
                    {
                        this.cancel_menu_preview(cx);
                        this.appearance_blend_mode_open = false;
                        this.dimension_menu_open = None;
                        cx.notify();
                    } else if let Some(target) = this.active_picker.clone() {
                        this.prepare_paint_picker_for_dismissal(cx);
                        this.cancel_active_paint_edit(cx);
                        this.emit_crop_cancel_if_active(&target, cx);
                        this.active_picker = None;
                        cx.notify();
                    } else if this.type_settings_open {
                        this.type_settings_open = false;
                        this.type_settings_tab = TypographySettingsTab::Basics;
                        cx.notify();
                    } else {
                        cx.propagate();
                    }
                }),
            )
            .on_key_down(cx.listener(Self::handle_property_key_down))
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .text_sm()
            .child(self.render_header(cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h(px(0.))
                    .child(
                        body.id(SharedString::from(format!("{}-scroll", self.id)))
                            .size_full()
                            .min_h(px(0.))
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll_handle),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .h_full()
                            .child(Scrollbar::new(&scroll_handle).axis(ScrollbarAxis::Vertical)),
                    )
                    .when_some(self.render_scrub_speed_cue(cx), |container, cue| {
                        container.child(cue)
                    }),
            )
    }
}

fn color_hsla(color: DesignColor) -> gpui::Hsla {
    rgba(
        (u32::from(color.red) << 24)
            | (u32::from(color.green) << 16)
            | (u32::from(color.blue) << 8)
            | u32::from(color.alpha),
    )
    .into()
}

fn parse_design_color(input: &str) -> Option<DesignColor> {
    let [red, green, blue, alpha] = rgba_channels(parse_hex_rgba(input)?);
    Some(DesignColor::rgba(red, green, blue, alpha))
}

fn format_number(value: f32) -> String {
    super::format::format_compact_number(value)
}

/// Keeps host-configured fractional nudge amounts round-trippable through the
/// expression parser instead of quantizing them to the one-decimal display
/// format used by passive inspector labels.
fn format_nudge_number(value: f32) -> String {
    if value == 0. {
        "0".to_owned()
    } else {
        value.to_string()
    }
}

fn format_vector_number_state(state: DesignVectorSelectionValue<f32>) -> SharedString {
    match state {
        DesignVectorSelectionValue::Unset => "Unavailable".into(),
        DesignVectorSelectionValue::Uniform(value) => format_number(value).into(),
        DesignVectorSelectionValue::Mixed => "Mixed".into(),
    }
}

fn format_vector_handle_state(
    state: DesignVectorSelectionValue<DesignHandleMirroring>,
) -> SharedString {
    match state {
        DesignVectorSelectionValue::Unset => "Unavailable".into(),
        DesignVectorSelectionValue::Uniform(mirroring) => mirroring.label().into(),
        DesignVectorSelectionValue::Mixed => "Mixed".into(),
    }
}

fn format_padding_shorthand([top, right, bottom, left]: [f32; 4]) -> String {
    if top == right && right == bottom && bottom == left {
        format_number(top)
    } else if top == bottom && right == left {
        format!("{}, {}", format_number(top), format_number(right))
    } else if right == left {
        format!(
            "{}, {}, {}",
            format_number(top),
            format_number(right),
            format_number(bottom)
        )
    } else {
        format!(
            "{}, {}, {}, {}",
            format_number(top),
            format_number(right),
            format_number(bottom),
            format_number(left)
        )
    }
}

fn normalize_rotation(value: f32) -> f32 {
    let normalized = (value + 180.).rem_euclid(360.) - 180.;
    if normalized == -0. { 0. } else { normalized }
}

fn format_optional_number(value: Option<f32>) -> String {
    value.map_or_else(|| "—".to_owned(), format_number)
}

fn format_text_max_lines(value: Option<u32>) -> String {
    value.map_or_else(|| "Auto".to_owned(), |lines| lines.to_string())
}

fn format_line_height(line_height: DesignLineHeight) -> String {
    match line_height {
        DesignLineHeight::Auto => "Auto".to_owned(),
        DesignLineHeight::Pixels(value) => format!("{} px", format_number(value)),
        DesignLineHeight::Percent(value) => format!("{}%", format_number(value)),
    }
}

fn next_line_height(line_height: DesignLineHeight) -> DesignLineHeight {
    match line_height {
        DesignLineHeight::Auto => DesignLineHeight::Pixels(24.),
        DesignLineHeight::Pixels(_) => DesignLineHeight::Percent(150.),
        DesignLineHeight::Percent(_) => DesignLineHeight::Auto,
    }
}

fn format_letter_spacing(letter_spacing: DesignLetterSpacing) -> String {
    match letter_spacing {
        DesignLetterSpacing::Pixels(value) => format!("{} px", format_number(value)),
        DesignLetterSpacing::Percent(value) => format!("{}%", format_number(value)),
    }
}

fn next_letter_spacing(letter_spacing: DesignLetterSpacing) -> DesignLetterSpacing {
    match letter_spacing {
        DesignLetterSpacing::Pixels(value) => DesignLetterSpacing::Percent(value),
        DesignLetterSpacing::Percent(value) => DesignLetterSpacing::Pixels(value),
    }
}

fn format_grid_track(track: DesignGridTrack) -> String {
    match track.sizing {
        DesignGridTrackSizing::Fixed => format!("{} px", format_number(track.value)),
        DesignGridTrackSizing::Fraction => format!("{} fr", format_number(track.value)),
        DesignGridTrackSizing::Hug => "Hug".to_owned(),
    }
}

fn next_grid_track(track: DesignGridTrack) -> DesignGridTrack {
    match track.sizing {
        DesignGridTrackSizing::Fixed => DesignGridTrack::fraction(1.),
        DesignGridTrackSizing::Fraction => DesignGridTrack::hug(),
        DesignGridTrackSizing::Hug => DesignGridTrack::fixed(100.),
    }
}

fn next_horizontal_constraint(constraint: super::DesignConstraint) -> super::DesignConstraint {
    use super::DesignConstraint as Constraint;
    match constraint {
        Constraint::Left => Constraint::Right,
        Constraint::Right => Constraint::LeftAndRight,
        Constraint::LeftAndRight => Constraint::Center,
        Constraint::Center => Constraint::Scale,
        _ => Constraint::Left,
    }
}

fn next_vertical_constraint(constraint: super::DesignConstraint) -> super::DesignConstraint {
    use super::DesignConstraint as Constraint;
    match constraint {
        Constraint::Top => Constraint::Bottom,
        Constraint::Bottom => Constraint::TopAndBottom,
        Constraint::TopAndBottom => Constraint::Center,
        Constraint::Center => Constraint::Scale,
        _ => Constraint::Top,
    }
}

fn empty_collection(label: &'static str, cx: &mut Context<DesignPanel>) -> AnyElement {
    div()
        .h(px(ROW_HEIGHT))
        .w_full()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .border_1()
        .border_color(cx.theme().border)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(label)
        .into_any_element()
}

#[cfg(test)]
mod tests;
