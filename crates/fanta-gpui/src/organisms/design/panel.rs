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

use crate::atoms::{
    ActivateControl, ButtonControlExt as _, CONTROL_KEY_CONTEXT, ControlExt as _, LucideIcon,
    render_lucide_icon, tokens,
};
use crate::color::{parse_hex_rgba, rgba_channels};
use crate::molecules::{
    InspectorEditPhase, InspectorFieldAccess, InspectorFieldPresentation, InspectorMetrics,
    InspectorNumberField, InspectorOverlayDismissIntent, InspectorTextField, InspectorValue,
    popup_height, popup_width,
};

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
    DesignPanelInspectionContext, DesignPanelMultipleSelection, DesignPanelNavigationViewData,
    DesignPanelNode, DesignPanelNodeKind, DesignPanelParentLayout, DesignPanelPermissions,
    DesignPanelPreferencesViewData, DesignPanelProjectionViewData, DesignPanelProperty,
    DesignPanelPropertyValueState, DesignPanelResourcesViewData, DesignPanelSection,
    DesignPanelSelectionKind, DesignPanelSurface, DesignPanelTarget,
    DesignPanelTargetedSelectionHeader, DesignPanelValue, DesignPanelViewData,
    DesignPanelWorkspaceMode, DesignPropertyVariableTarget, DesignRepeatAxis, DesignRepeatMode,
    DesignRepeatType, DesignRowGridAlignment, DesignScatterBrushName, DesignScrubSpeed,
    DesignSectionDevStatusKind, DesignSelectionHeaderCommand, DesignSelectionHeaderCommandAccess,
    DesignSelectionHeaderControl, DesignSelectionHeaderControlIcon,
    DesignSelectionHeaderControlKind, DesignSelectionHeaderViewData,
    DesignShaderPropertyEditorKind, DesignShaderPropertyEditorTarget, DesignShaderPropertyKind,
    DesignShaderPropertyValue, DesignShaderViewData, DesignShapeGeometry, DesignSizingMode,
    DesignSlotSettingsChange, DesignSlotViolation, DesignSmartSelectionAvailability,
    DesignSmartSelectionAxis, DesignSmartSelectionOperation, DesignSmartSelectionSpacingValue,
    DesignSmartSelectionViewData, DesignStackingOrder, DesignStretchBrushName, DesignStrokeAlign,
    DesignStrokeBrushDirection, DesignStrokeCap, DesignStrokeDashMode, DesignStrokeEndpointControl,
    DesignStrokeJoin, DesignStrokeType, DesignStrokeWeightMode, DesignTextCase,
    DesignTextDecoration, DesignTextDecorationColor, DesignTextDecorationMetric,
    DesignTextDecorationStyle, DesignTextHorizontalAlignment, DesignTextLeadingTrim,
    DesignTextList, DesignTextPathOrientation, DesignTextPathStartData, DesignTextResize,
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
mod component_authoring_edit_controller;
mod component_props;
mod drag_preview;
mod edit_controller;
mod effects;
mod export;
mod factory;
mod feature_state;
mod host_state;
mod inspector_fields;
mod inspector_preferences;
mod layout;
mod layout_grids;
mod options;
mod overlay_coordinator;
mod paints;
mod position;
mod properties;
mod resource_catalogs;
mod retained_children;
mod scrub;
mod section_controller;
mod sections;
mod shader_helpers;
mod shell;
mod typography;
mod view_data_controller;

use alignment_grid::*;
use browsers::DesignBrowserController;
use component_authoring_edit_controller::{
    ComponentAuthoringNameEditor, DesignComponentAuthoringEditController,
};
use component_props::DesignComponentController;
use edit_controller::{
    ActivePaintEdit, DesignComponentMultilineEditEvent, DesignGridDimensionsEditEvent,
    DesignGridDimensionsEditTarget, DesignMediaCropEditTarget, DesignPropertyEditController,
    DesignPropertyEditEvent, DesignPropertyEditReconciliation, DesignVariableFontAxisEditEvent,
};
use effects::DesignEffectsController as DesignEffectsControllerExt;
use factory::DesignPanelFactory;
use feature_state::DesignPanelFeatureState;
use host_state::DesignPanelHostState;
use inspector_fields::DesignInspectorFieldRenderer as _;
use inspector_preferences::DesignInspectorPreferences;
use layout::DesignLayoutController as DesignLayoutControllerExt;
use layout_grids::DesignLayoutGridController as DesignLayoutGridControllerExt;
use options::DesignOptionsController as DesignOptionsControllerExt;
use overlay_coordinator::{DesignOpenOverlay, DesignOverlayCoordinator, DesignOverlayState};
use paints::DesignPaintController as DesignPaintControllerExt;
use properties::DesignPropertiesController as DesignPropertiesControllerExt;
use resource_catalogs::DesignPanelResourceCatalogs;
use retained_children::{
    DesignDrawSliderStates, DesignPanelInputStates, DesignPanelRetainedChildren,
};
use scrub::{DesignScrubController as DesignScrubControllerExt, render_numeric_scrub_surface};
use section_controller::DesignSectionController;
use sections::appearance::AppearancePanelCompat as _;
use sections::export::ExportPanelCompat as _;
use sections::header::HeaderPanelCompat as _;
use sections::page::PagePanelController as _;
use sections::position::PositionPanelCompat as _;
use shader_helpers::*;
use shell::{DesignPanelShellController as DesignPanelShellControllerExt, DesignPanelShellState};
use typography::DesignTypographyController as DesignTypographyControllerExt;
use view_data_controller::DesignPanelViewDataController;

const HEADER_HEIGHT: f32 = InspectorMetrics::SECTION_HEADER_HEIGHT;
const ROW_HEIGHT: f32 = InspectorMetrics::ROW_HEIGHT;
const PANEL_PADDING: f32 = InspectorMetrics::HORIZONTAL_PADDING;
const NUMERIC_SCRUB_THRESHOLD: f32 = 2.;
const GRID_DIMENSIONS_POPOVER_WIDTH: f32 = tokens::MenuWidth::POPOVER;

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
    /// Domain-neutral exact-value field that owns the transient draft and its
    /// Begin/Preview/Commit/Cancel session. `InputState` remains only the GPUI
    /// renderer, while Design-specific parsing stays in the section codec.
    controlled: PropertyEditorControlledField,
}

#[derive(Clone, Debug, PartialEq)]
enum PropertyEditorControlledField {
    Number(InspectorNumberField),
    Text(InspectorTextField),
    /// Color, list, shader, and other compound codecs retain their existing
    /// editor until a matching domain-neutral field exists.
    Compound,
}

struct PropertyEditorSeed {
    property: DesignPanelProperty,
    layout_grid_target: Option<LayoutGridTarget>,
    export_configuration_id: Option<SharedString>,
    original: DesignPanelValue,
    value: InspectorValue<DesignPanelValue>,
    base: f64,
    kind: PropertyEditorKind,
    draft: String,
}

struct PropertyEditorNudge {
    property: DesignPanelProperty,
    current: f64,
    next: f64,
    value: DesignPanelValue,
    draft: String,
}

impl PropertyEditor {
    fn new(seed: PropertyEditorSeed) -> Self {
        let PropertyEditorSeed {
            property,
            layout_grid_target,
            export_configuration_id,
            original,
            value,
            base,
            kind,
            draft,
        } = seed;
        let presentation = InspectorFieldPresentation::new(InspectorFieldAccess::Editable);
        let controlled = match kind {
            PropertyEditorKind::Number { .. }
            | PropertyEditorKind::OptionalNumber { .. }
            | PropertyEditorKind::AngleDegrees
            | PropertyEditorKind::PercentageRatio
            | PropertyEditorKind::LayoutGridCount
            | PropertyEditorKind::ExportSizing => {
                let value = match value {
                    InspectorValue::Mixed => InspectorValue::Mixed,
                    InspectorValue::Unset => InspectorValue::Unset,
                    InspectorValue::Uniform(value) => Self::numeric_scalar(kind, &value)
                        .map_or(InspectorValue::Unset, InspectorValue::Uniform),
                };
                PropertyEditorControlledField::Number(InspectorNumberField::new(
                    value,
                    presentation,
                ))
            }
            PropertyEditorKind::Text => {
                let value = match value {
                    InspectorValue::Mixed => InspectorValue::Mixed,
                    InspectorValue::Unset => InspectorValue::Unset,
                    InspectorValue::Uniform(DesignPanelValue::Text(value)) => {
                        InspectorValue::Uniform(value)
                    }
                    InspectorValue::Uniform(_) => InspectorValue::Unset,
                };
                PropertyEditorControlledField::Text(InspectorTextField::new(value, presentation))
            }
            PropertyEditorKind::Color
            | PropertyEditorKind::NumberList
            | PropertyEditorKind::Shader { .. } => PropertyEditorControlledField::Compound,
        };
        let mut editor = Self {
            property,
            layout_grid_target,
            export_configuration_id,
            original,
            last_preview: None,
            base,
            kind,
            controlled,
        };
        editor.begin_controlled(&draft);
        editor
    }

    fn numeric_scalar(kind: PropertyEditorKind, value: &DesignPanelValue) -> Option<f64> {
        match (kind, value) {
            (
                PropertyEditorKind::Number { integer: false, .. },
                DesignPanelValue::Number(value),
            ) => Some(f64::from(*value)),
            (
                PropertyEditorKind::Number { integer: true, .. },
                DesignPanelValue::Integer(value),
            ) => Some(*value as f64),
            (
                PropertyEditorKind::OptionalNumber { .. },
                DesignPanelValue::OptionalNumber(Some(value)),
            ) => Some(f64::from(*value)),
            (PropertyEditorKind::AngleDegrees, DesignPanelValue::AngleRadians(value)) => {
                Some(f64::from(value.to_degrees()))
            }
            (PropertyEditorKind::PercentageRatio, DesignPanelValue::Ratio(value)) => {
                Some(f64::from(*value * 100.))
            }
            (
                PropertyEditorKind::LayoutGridCount,
                DesignPanelValue::LayoutGridCount(DesignLayoutGridCount::Number(value)),
            ) => Some(f64::from(*value)),
            (PropertyEditorKind::ExportSizing, DesignPanelValue::ExportSizing(value)) => {
                Some(f64::from(value.value()))
            }
            _ => None,
        }
        .filter(|value| value.is_finite())
    }

    fn begin_controlled(&mut self, draft: &str) {
        let phase = match &mut self.controlled {
            PropertyEditorControlledField::Number(field) => {
                field.begin_with(self.base, draft).map(|edit| edit.phase)
            }
            PropertyEditorControlledField::Text(field) => {
                field.begin_with(draft).map(|edit| edit.phase)
            }
            PropertyEditorControlledField::Compound => return,
        };
        debug_assert_eq!(phase, Some(InspectorEditPhase::Begin));
    }

    fn sync_controlled_draft(&mut self, draft: &str) {
        match &mut self.controlled {
            PropertyEditorControlledField::Number(field) => {
                let _ = field.set_text(draft);
            }
            PropertyEditorControlledField::Text(field) => {
                let _ = field.set_text(SharedString::from(draft.to_owned()));
            }
            PropertyEditorControlledField::Compound => {}
        }
    }

    fn controlled_draft(&self) -> Option<&str> {
        match &self.controlled {
            PropertyEditorControlledField::Number(field) => field.draft(),
            PropertyEditorControlledField::Text(field) => field.draft(),
            PropertyEditorControlledField::Compound => None,
        }
    }

    /// Records one accepted Design-domain value through the shared exact-value
    /// field, returning true only when that field produced a distinct Preview.
    fn preview_controlled(&mut self, value: &DesignPanelValue) -> bool {
        if self.last_preview.is_none() && self.original == *value {
            return false;
        }
        let phase = match &mut self.controlled {
            PropertyEditorControlledField::Number(field) => {
                if let Some(scalar) = Self::numeric_scalar(self.kind, value) {
                    field.preview_with(|_| Some(scalar))
                } else if matches!(
                    (self.kind, value),
                    (
                        PropertyEditorKind::OptionalNumber { .. },
                        DesignPanelValue::OptionalNumber(None)
                    ) | (
                        PropertyEditorKind::LayoutGridCount,
                        DesignPanelValue::LayoutGridCount(DesignLayoutGridCount::Auto)
                    )
                ) {
                    field.preview_unset()
                } else {
                    None
                }
                .map(|edit| edit.phase)
            }
            PropertyEditorControlledField::Text(field) => {
                let DesignPanelValue::Text(value) = value else {
                    return false;
                };
                field.preview(value.clone()).map(|edit| edit.phase)
            }
            PropertyEditorControlledField::Compound => {
                (self.last_preview.as_ref() != Some(value)).then_some(InspectorEditPhase::Preview)
            }
        };
        if phase == Some(InspectorEditPhase::Preview) {
            self.last_preview = Some(value.clone());
            true
        } else {
            false
        }
    }

    fn scrub_controlled(&mut self, scalar: f64, value: &DesignPanelValue) -> bool {
        if self.last_preview.is_none() && self.original == *value {
            return false;
        }
        let PropertyEditorControlledField::Number(field) = &mut self.controlled else {
            return false;
        };
        let edit = field.scrub_to(scalar, |value| format_nudge_number(value as f32));
        if edit
            .as_ref()
            .is_some_and(|edit| edit.phase == InspectorEditPhase::Preview)
        {
            self.last_preview = Some(value.clone());
            true
        } else {
            false
        }
    }

    /// Applies a keyboard nudge through the shared numeric draft. Domain
    /// parsing and normalization are injected by the Design codec; the field
    /// owns the draft replacement and preview de-duplication.
    fn nudge_controlled(
        &mut self,
        current: f64,
        next: f64,
        value: &DesignPanelValue,
        formatted: &str,
    ) -> Option<bool> {
        let PropertyEditorControlledField::Number(field) = &mut self.controlled else {
            return None;
        };
        let emitted = field
            .nudge_with(
                next - current,
                |_| Some(current),
                |_| Some(next),
                |_| formatted.to_owned(),
            )
            .is_some_and(|edit| edit.phase == InspectorEditPhase::Preview);
        if emitted {
            self.last_preview = Some(value.clone());
        }
        Some(emitted)
    }

    /// Consumes the shared field transaction exactly once. The returned phase
    /// is used as an invariant check around the unchanged Design action guard.
    fn finish_controlled(
        &mut self,
        commit: bool,
        accepted: Option<&DesignPanelValue>,
    ) -> Option<InspectorEditPhase> {
        match &mut self.controlled {
            PropertyEditorControlledField::Number(field) if commit => if let Some(value) = accepted
            {
                if let Some(scalar) = Self::numeric_scalar(self.kind, value) {
                    field.commit_with(|_| Some(scalar))
                } else if matches!(
                    (self.kind, value),
                    (
                        PropertyEditorKind::OptionalNumber { .. },
                        DesignPanelValue::OptionalNumber(None)
                    ) | (
                        PropertyEditorKind::LayoutGridCount,
                        DesignPanelValue::LayoutGridCount(DesignLayoutGridCount::Auto)
                    )
                ) {
                    field.commit_unset()
                } else {
                    field.cancel()
                }
            } else {
                field.cancel()
            }
            .map(|edit| edit.phase),
            PropertyEditorControlledField::Number(field) => field.cancel().map(|edit| edit.phase),
            PropertyEditorControlledField::Text(field) if commit && accepted.is_some() => {
                field.commit().map(|edit| edit.phase)
            }
            PropertyEditorControlledField::Text(field) => field.cancel().map(|edit| edit.phase),
            PropertyEditorControlledField::Compound => Some(if commit && accepted.is_some() {
                InspectorEditPhase::Commit
            } else {
                InspectorEditPhase::Cancel
            }),
        }
    }
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
    /// Domain-neutral exact numeric field that owns the active draft and its
    /// balanced edit phases. Axis ranges and OpenType identity remain Design
    /// concerns on this controller projection.
    field: InspectorNumberField,
}

impl VariableFontAxisEditor {
    fn new(tag: SharedString, target: DesignTypographyTarget, axis: &DesignFontAxis) -> Self {
        Self {
            tag,
            target,
            original: axis.value,
            last_preview: None,
            min: axis.min,
            max: axis.max,
            default: axis.default,
            step: axis.step,
            field: InspectorNumberField::new(
                InspectorValue::Uniform(f64::from(axis.value)),
                InspectorFieldPresentation::new(InspectorFieldAccess::Editable),
            ),
        }
    }
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
    /// The GPUI auto-growing input renders this domain-neutral controlled
    /// draft; document ownership remains with the host.
    field: InspectorTextField,
}

impl ComponentMultilineEditor {
    fn new(property_id: SharedString, value: SharedString) -> Self {
        Self {
            property_id,
            original: DesignComponentPropertyValue::Text(value.clone()),
            last_preview: None,
            field: InspectorTextField::new(
                InspectorValue::Uniform(value),
                InspectorFieldPresentation::new(InspectorFieldAccess::Editable),
            ),
        }
    }
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
    host: DesignPanelHostState,
    resources: DesignPanelResourceCatalogs,
    preferences: DesignInspectorPreferences,
    features: DesignPanelFeatureState,
    paint_picker: Entity<PaintPicker>,
    typography_style_picker: Entity<TypographyStylePicker>,
    overlays: DesignOverlayCoordinator,
    sections: DesignSectionController,
    component_authoring: component_props::ComponentAuthoringState,
    /// Transient drafts and phased edit sessions. Document values remain
    /// authoritative in the host snapshot.
    edit: DesignPropertyEditController,
    /// Retained GPUI children, their subscriptions, and the controlled
    /// snapshots they were last synchronized to.
    retained: DesignPanelRetainedChildren,
    /// Scroll continuity owned by the top-level shell.
    shell: DesignPanelShellState,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DesignPanelAction> for DesignPanel {}

trait DesignPanelActionEmitter {
    fn emit_design_panel_action(&mut self, panel: &DesignPanel, action: DesignPanelAction);
}

impl DesignPanelActionEmitter for Context<'_, DesignPanel> {
    fn emit_design_panel_action(&mut self, panel: &DesignPanel, mut action: DesignPanelAction) {
        if panel.host.inspection_context.selection().kind() == DesignPanelSelectionKind::Multiple
            && action.legacy_node_id_mut().is_some()
        {
            action = DesignPanelAction::for_selection_target(panel.command_target(), action);
        }
        self.emit(action);
    }
}

#[allow(deprecated)]
impl DesignPanel {
    /// Current Figma scrub band while either numeric scrub transaction is
    /// active. Pre-threshold click candidates deliberately return `None`.
    pub fn active_scrub_speed(&self) -> Option<DesignScrubSpeed> {
        self.active_scrub_speed_from_controller()
    }

    pub fn new(
        id: impl Into<SharedString>,
        node: DesignPanelNode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        DesignPanelFactory::assemble(id, node, window, cx)
    }

    /// Creates a panel directly from a complete inspection context.
    pub fn new_with_context(
        id: impl Into<SharedString>,
        inspection_context: DesignPanelInspectionContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        DesignPanelFactory::assemble_with_context(id, inspection_context, window, cx)
    }

    /// Returns the first inspected node from the authoritative context.
    ///
    /// A Page/no-selection context has no inspected node, so the legacy node
    /// supplied through [`Self::set_node`] remains the compatibility fallback.
    pub fn node(&self) -> &DesignPanelNode {
        self.host.inspected_node()
    }

    /// Whether compact inspector controls include their explanatory labels.
    pub const fn additional_labels(&self) -> bool {
        self.preferences.additional_labels
    }

    pub const fn nudge_settings(&self) -> DesignNudgeSettings {
        self.preferences.nudge_settings
    }

    /// Returns the exhaustive right-sidebar surfaces valid for the current
    /// permission context.
    pub fn available_surfaces(&self) -> &'static [DesignPanelSurface] {
        DesignPanelSurface::available(self.host.inspection_context.permissions().can_edit())
    }

    /// Returns the last surface accepted by the host for the current
    /// permission context.
    pub const fn active_surface(&self) -> DesignPanelSurface {
        if self.host.inspection_context.permissions().can_edit() {
            self.host.navigation.editor_surface
        } else {
            self.host.navigation.viewer_surface
        }
    }

    /// Returns the host-controlled editable-workspace presentation.
    ///
    /// This value is independent from [`Self::active_surface`] and the
    /// inspection context's [`DesignPanelEditMode`].
    pub const fn workspace_mode(&self) -> DesignPanelWorkspaceMode {
        self.host.navigation.workspace_mode
    }

    fn renders_draw_workspace(&self) -> bool {
        self.host.navigation.workspace_mode == DesignPanelWorkspaceMode::Draw
            && self.host.inspection_context.permissions().can_edit()
    }

    pub const fn inspection_context(&self) -> &DesignPanelInspectionContext {
        &self.host.inspection_context
    }

    /// Returns the complete host-controlled snapshot currently retained by
    /// the panel.
    ///
    /// Focus, scroll, open overlays, drafts, and other presentation-only state
    /// are deliberately excluded. The returned value can therefore be fed to
    /// another panel through [`Self::set_view_data`] without transferring UI
    /// interaction state.
    pub fn view_data(&self) -> DesignPanelViewData {
        DesignPanelViewDataController::canonical_view_data(self)
    }

    /// Atomically applies one complete host-controlled Design-panel snapshot.
    ///
    /// Existing granular setters remain source-compatible and share this
    /// method's normalization, target validation, and interaction-cancellation
    /// behavior. Context and navigation are applied first so every target-bound
    /// projection is reconciled against the incoming selection rather than the
    /// outgoing one.
    pub fn set_view_data(&mut self, view_data: DesignPanelViewData, cx: &mut Context<Self>) {
        DesignPanelViewDataController::apply_view_data(self, view_data, cx);
    }

    pub const fn color_style_view_data(&self) -> &DesignColorStyleViewData {
        &self.resources.color_styles
    }

    pub const fn color_style_sample_view_data(&self) -> &DesignColorStyleSampleViewData {
        &self.resources.color_style_samples
    }

    pub const fn color_contrast_view_data(&self) -> &DesignColorContrastViewData {
        &self.resources.color_contrast
    }

    pub const fn paint_variable_view_data(&self) -> &DesignPaintVariableViewData {
        &self.resources.paint_variables
    }

    pub const fn paint_style_view_data(&self) -> &DesignPaintStyleViewData {
        &self.resources.paint_styles
    }

    pub const fn media_paint_view_data(&self) -> &DesignMediaPaintViewData {
        &self.resources.media_paints
    }

    pub const fn shader_view_data(&self) -> &DesignShaderViewData {
        &self.resources.shaders
    }

    pub const fn typography_style_view_data(&self) -> &DesignTypographyStyleViewData {
        &self.resources.typography_styles
    }

    pub const fn font_view_data(&self) -> &DesignFontViewData {
        &self.resources.fonts
    }

    pub const fn effect_style_view_data(&self) -> &DesignEffectStyleViewData {
        &self.resources.effect_styles
    }

    pub const fn effect_variable_view_data(&self) -> &DesignEffectVariableViewData {
        &self.resources.effect_variables
    }

    pub const fn property_variable_view_data(&self) -> &DesignVariableViewData {
        &self.resources.property_variables
    }

    pub const fn component_swap_view_data(&self) -> &DesignComponentSwapViewData {
        &self.resources.component_swaps
    }

    pub const fn layout_grid_style_view_data(&self) -> &DesignLayoutGridStyleViewData {
        &self.resources.layout_grid_styles
    }

    pub const fn layout_grid_variable_view_data(&self) -> &DesignLayoutGridVariableViewData {
        &self.resources.layout_grid_variables
    }

    #[deprecated(note = "use layout_grid_variable_view_data")]
    pub const fn layout_grid_count_variable_view_data(
        &self,
    ) -> &DesignLayoutGridCountVariableViewData {
        &self.resources.layout_grid_count_variables
    }

    pub const fn add_auto_layout_view_data(&self) -> Option<&DesignAddAutoLayoutViewData> {
        self.host.projections.add_auto_layout.as_ref()
    }

    pub const fn draw_appearance_view_data(&self) -> Option<&DesignDrawAppearanceViewData> {
        self.host.projections.draw_appearance.as_ref()
    }

    pub const fn frame_preset_view_data(&self) -> Option<&DesignFramePresetViewData> {
        self.host.projections.frame_presets.as_ref()
    }

    pub const fn smart_selection_view_data(&self) -> Option<&DesignSmartSelectionViewData> {
        self.host.projections.smart_selection.as_ref()
    }

    pub const fn page_view_data(&self) -> Option<&DesignPageViewData> {
        self.host.projections.page.as_ref()
    }

    pub const fn page_local_styles_view_data(&self) -> Option<&DesignPageLocalStylesViewData> {
        self.host.projections.page_local_styles.as_ref()
    }

    pub const fn variables_entry_point(&self) -> &DesignVariablesEntryPoint {
        &self.preferences.variables_entry_point
    }

    pub const fn variable_mode_view_data(&self) -> Option<&DesignVariableModeViewData> {
        self.host.projections.variable_modes.as_ref()
    }

    pub const fn viewer_properties_view_data(&self) -> Option<&DesignViewerPropertiesViewData> {
        self.host.projections.viewer_properties.as_ref()
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
        Self::selection_header_target_for_context(&self.host.inspection_context)
    }

    fn selection_header_view_data_for_context(&self) -> Option<&DesignSelectionHeaderViewData> {
        let target = self.current_selection_header_target()?;
        if self.host.projections.selection_header_target.as_ref() == Some(&target) {
            self.host.projections.selection_header.as_ref()
        } else {
            None
        }
    }

    pub fn selection_header_view_data(&self) -> Option<&DesignSelectionHeaderViewData> {
        self.selection_header_view_data_for_context()
    }

    fn resolved_selection_header_view_data(&self) -> DesignSelectionHeaderViewData {
        self.selection_header_view_data_for_context()
            .cloned()
            .unwrap_or_else(|| match self.host.inspection_context.selection().kind() {
                DesignPanelSelectionKind::None => {
                    DesignSelectionHeaderViewData::new("Page", std::iter::empty())
                }
                DesignPanelSelectionKind::Single => {
                    DesignSelectionHeaderViewData::for_node_kind(self.host.inspected_node().kind)
                }
                DesignPanelSelectionKind::Multiple => {
                    DesignSelectionHeaderViewData::for_multiple_selection(
                        self.host.inspection_context.selection().len(),
                    )
                }
            })
    }

    fn command_target(&self) -> DesignPanelTarget {
        match self.host.inspection_context.selection().kind() {
            DesignPanelSelectionKind::None => DesignPanelTarget::Page {
                page_id: self.page_view_data_for_context().map_or_else(
                    || self.host.inspected_node().id.clone(),
                    |page| page.page_id.clone(),
                ),
            },
            DesignPanelSelectionKind::Single | DesignPanelSelectionKind::Multiple => {
                DesignPanelTarget::Nodes {
                    node_ids: self
                        .host
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

    /// Applies one coordinator-owned dismissal after balancing any retained
    /// child transaction attached to the surface. Escape and outside-click
    /// paths converge here, so overlay priority, cleanup, and focus return
    /// cannot drift between individual renderers.
    fn dismiss_overlay(
        &mut self,
        intent: InspectorOverlayDismissIntent<DesignOpenOverlay>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.overlays.dismissal_is_current(&intent) {
            return;
        }
        match *intent.overlay() {
            DesignOpenOverlay::ComponentPropertyEdit => {
                self.component_authoring.create_draft = None;
                self.close_component_authoring_dialog(window, cx);
            }
            DesignOpenOverlay::ComponentPropertyContextMenu => {
                self.focus_handle.focus(window, cx);
            }
            DesignOpenOverlay::ComponentPropertyCreateMenu => {
                self.focus_handle.focus(window, cx);
            }
            DesignOpenOverlay::AuxiliaryColorPicker => {
                self.prepare_paint_picker_for_dismissal(cx);
                self.cancel_active_paint_edit(cx);
                self.overlays.discard(DesignOpenOverlay::PageBackground);
            }
            DesignOpenOverlay::ComponentSwap => {
                if let Some(property_id) = self.overlays.component_swap_browser().clone() {
                    self.clear_component_swap_preview(property_id.as_ref(), cx);
                }
            }
            DesignOpenOverlay::PreviewOptionMenu => {
                self.cancel_menu_preview(cx);
            }
            DesignOpenOverlay::AppearanceBlendMode => {
                self.cancel_menu_preview(cx);
            }
            DesignOpenOverlay::DimensionMenu => {
                self.cancel_menu_preview(cx);
            }
            DesignOpenOverlay::ExportChoice => {
                self.cancel_menu_preview(cx);
            }
            DesignOpenOverlay::EffectSettings => {
                self.cancel_menu_preview(cx);
            }
            DesignOpenOverlay::SelectionColorResource => {
                self.cancel_menu_preview(cx);
            }
            DesignOpenOverlay::LayoutGridCountVariable => {
                self.cancel_menu_preview(cx);
            }
            DesignOpenOverlay::MenuPreview => {
                self.cancel_menu_preview(cx);
            }
            DesignOpenOverlay::PaintPicker => {
                let target = self
                    .overlays
                    .active_picker()
                    .clone()
                    .expect("the active overlay is a paint picker");
                self.prepare_paint_picker_for_dismissal(cx);
                self.cancel_active_paint_edit(cx);
                self.emit_crop_cancel_if_active(&target, cx);
            }
            DesignOpenOverlay::TypeSettings => {
                self.features.typography.settings_tab = TypographySettingsTab::Basics;
            }
            DesignOpenOverlay::PageBackground
            | DesignOpenOverlay::PageResource
            | DesignOpenOverlay::VariableMode
            | DesignOpenOverlay::FramePreset
            | DesignOpenOverlay::ComponentPropertyVariable
            | DesignOpenOverlay::PropertyVariable
            | DesignOpenOverlay::TypographyStyle
            | DesignOpenOverlay::FontBrowser
            | DesignOpenOverlay::SelectionHeader
            | DesignOpenOverlay::PaintStyle
            | DesignOpenOverlay::EffectStyle
            | DesignOpenOverlay::LayoutGridStyle
            | DesignOpenOverlay::GridDimensions => {}
        }

        if let Some(focus_return) = self.overlays.finish_dismissal(&intent) {
            focus_return.restore(window, cx);
        }
        cx.notify();
    }

    fn dismiss_topmost_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(intent) = self.overlays.escape_dismissal_intent() else {
            return false;
        };
        self.dismiss_overlay(intent, window, cx);
        true
    }

    /// Dismisses `overlay` for an item-level Escape handler only when it is
    /// currently the coordinator's topmost surface. This keeps nested overlay
    /// priority and trigger focus restoration identical to root-level Escape.
    fn dismiss_overlay_from_escape(
        &mut self,
        overlay: DesignOpenOverlay,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(intent) = self.overlays.escape_dismissal_intent_for(overlay) else {
            return false;
        };
        self.dismiss_overlay(intent, window, cx);
        true
    }

    /// Captures the currently focused trigger before an inspector-owned
    /// surface moves focus into its contents. Renderers use this common entry
    /// point so Escape and outside-click dismissal share the same focus-return
    /// policy. The panel root is a stable fallback for pointer-opened controls
    /// that did not own a dedicated focus handle.
    fn remember_overlay_focus_return(
        &mut self,
        overlay: DesignOpenOverlay,
        window: &Window,
        cx: &App,
    ) {
        self.overlays
            .capture_focus_return(overlay, window, &self.focus_handle, cx);
    }

    fn dismiss_overlay_from_outside_click(
        &mut self,
        overlay: DesignOpenOverlay,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(intent) = self.overlays.outside_click_dismissal_intent(overlay) else {
            return false;
        };
        self.dismiss_overlay(intent, window, cx);
        true
    }

    // Compatibility update surface ---------------------------------------------------------
    //
    // These granular APIs remain public for existing hosts, but they are deliberately
    // one-line adapters into the same private apply operations used by `set_view_data`.
    // Keeping the mutation logic on the private side makes the grouped snapshot the
    // canonical host-to-panel update path without changing source compatibility.

    /// Compatibility wrapper for replacing the legacy selected-node projection.
    pub fn set_node(&mut self, node: DesignPanelNode, cx: &mut Context<Self>) {
        self.apply_node(node, cx);
    }

    /// Compatibility wrapper for the Additional labels preference.
    pub fn set_additional_labels(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.apply_additional_labels(enabled, cx);
    }

    /// Compatibility wrapper for host-controlled nudge settings.
    pub fn set_nudge_settings(&mut self, settings: DesignNudgeSettings, cx: &mut Context<Self>) {
        self.apply_nudge_settings(settings, cx);
    }

    /// Compatibility wrapper for the Design/Draw workspace projection.
    pub fn set_workspace_mode(
        &mut self,
        workspace_mode: DesignPanelWorkspaceMode,
        cx: &mut Context<Self>,
    ) {
        self.apply_workspace_mode(workspace_mode, cx);
    }

    /// Compatibility wrapper for the active sidebar surface.
    pub fn set_active_surface(
        &mut self,
        surface: DesignPanelSurface,
        cx: &mut Context<Self>,
    ) -> bool {
        self.apply_active_surface(surface, cx)
    }

    /// Compatibility wrapper for replacing the inspection context.
    pub fn set_inspection_context(
        &mut self,
        context: DesignPanelInspectionContext,
        cx: &mut Context<Self>,
    ) {
        self.apply_inspection_context(context, cx);
    }

    pub fn set_export_view_data(
        &mut self,
        view_data: DesignExportViewData,
        cx: &mut Context<Self>,
    ) {
        self.apply_export_view_data(view_data, cx);
    }

    pub fn clear_export_view_data(&mut self, cx: &mut Context<Self>) {
        self.apply_clear_export_view_data(cx);
    }

    pub fn set_color_style_view_data(
        &mut self,
        view_data: DesignColorStyleViewData,
        cx: &mut Context<Self>,
    ) {
        self.apply_color_style_view_data(view_data, cx);
    }

    pub fn set_color_style_sample_view_data(
        &mut self,
        view_data: DesignColorStyleSampleViewData,
        cx: &mut Context<Self>,
    ) {
        self.apply_color_style_sample_view_data(view_data, cx);
    }

    pub fn set_color_contrast_view_data(
        &mut self,
        view_data: DesignColorContrastViewData,
        cx: &mut Context<Self>,
    ) {
        self.apply_color_contrast_view_data(view_data, cx);
    }

    pub fn set_paint_variable_view_data(
        &mut self,
        view_data: DesignPaintVariableViewData,
        cx: &mut Context<Self>,
    ) {
        self.apply_paint_variable_view_data(view_data, cx);
    }

    pub fn set_paint_style_view_data(
        &mut self,
        view_data: DesignPaintStyleViewData,
        cx: &mut Context<Self>,
    ) {
        self.apply_paint_style_view_data(view_data, cx);
    }

    pub fn set_media_paint_view_data(
        &mut self,
        view_data: DesignMediaPaintViewData,
        cx: &mut Context<Self>,
    ) {
        self.apply_media_paint_view_data(view_data, cx);
    }

    pub fn set_shader_view_data(
        &mut self,
        view_data: DesignShaderViewData,
        cx: &mut Context<Self>,
    ) {
        self.apply_shader_view_data(view_data, cx);
    }

    pub fn set_typography_style_view_data(
        &mut self,
        view_data: DesignTypographyStyleViewData,
        cx: &mut Context<Self>,
    ) {
        self.apply_typography_style_view_data(view_data, cx);
    }

    pub fn set_font_view_data(&mut self, view_data: DesignFontViewData, cx: &mut Context<Self>) {
        self.apply_font_view_data(view_data, cx);
    }

    pub fn set_effect_style_view_data(
        &mut self,
        view_data: DesignEffectStyleViewData,
        cx: &mut Context<Self>,
    ) {
        self.apply_effect_style_view_data(view_data, cx);
    }

    pub fn set_effect_variable_view_data(
        &mut self,
        view_data: DesignEffectVariableViewData,
        cx: &mut Context<Self>,
    ) {
        self.apply_effect_variable_view_data(view_data, cx);
    }

    pub fn set_property_variable_view_data(
        &mut self,
        view_data: DesignVariableViewData,
        cx: &mut Context<Self>,
    ) {
        self.apply_property_variable_view_data(view_data, cx);
    }

    pub fn set_component_swap_view_data(
        &mut self,
        view_data: DesignComponentSwapViewData,
        cx: &mut Context<Self>,
    ) {
        self.apply_component_swap_view_data(view_data, cx);
    }

    pub fn set_layout_grid_style_view_data(
        &mut self,
        view_data: DesignLayoutGridStyleViewData,
        cx: &mut Context<Self>,
    ) {
        self.apply_layout_grid_style_view_data(view_data, cx);
    }

    pub fn set_layout_grid_variable_view_data(
        &mut self,
        view_data: DesignLayoutGridVariableViewData,
        cx: &mut Context<Self>,
    ) {
        self.apply_layout_grid_variable_view_data(view_data, cx);
    }

    /// Compatibility adapter for the former count-only variable catalog.
    #[deprecated(note = "use set_layout_grid_variable_view_data")]
    pub fn set_layout_grid_count_variable_view_data(
        &mut self,
        view_data: DesignLayoutGridCountVariableViewData,
        cx: &mut Context<Self>,
    ) {
        self.apply_layout_grid_count_variable_view_data(view_data, cx);
    }

    pub fn set_add_auto_layout_view_data(
        &mut self,
        view_data: DesignAddAutoLayoutViewData,
        cx: &mut Context<Self>,
    ) {
        self.apply_add_auto_layout_view_data(view_data, cx);
    }

    pub fn clear_add_auto_layout_view_data(&mut self, cx: &mut Context<Self>) {
        self.apply_clear_add_auto_layout_view_data(cx);
    }

    pub fn set_draw_appearance_view_data(
        &mut self,
        view_data: DesignDrawAppearanceViewData,
        cx: &mut Context<Self>,
    ) -> bool {
        self.apply_draw_appearance_view_data(view_data, cx)
    }

    pub fn clear_draw_appearance_view_data(&mut self, cx: &mut Context<Self>) {
        self.apply_clear_draw_appearance_view_data(cx);
    }

    pub fn set_frame_preset_view_data(
        &mut self,
        view_data: DesignFramePresetViewData,
        cx: &mut Context<Self>,
    ) {
        self.apply_frame_preset_view_data(view_data, cx);
    }

    pub fn clear_frame_preset_view_data(&mut self, cx: &mut Context<Self>) {
        self.apply_clear_frame_preset_view_data(cx);
    }

    pub fn set_smart_selection_view_data(
        &mut self,
        view_data: DesignSmartSelectionViewData,
        cx: &mut Context<Self>,
    ) {
        self.apply_smart_selection_view_data(view_data, cx);
    }

    pub fn clear_smart_selection_view_data(&mut self, cx: &mut Context<Self>) {
        self.apply_clear_smart_selection_view_data(cx);
    }

    pub fn set_page_view_data(&mut self, view_data: DesignPageViewData, cx: &mut Context<Self>) {
        self.apply_page_view_data(view_data, cx);
    }

    pub fn clear_page_view_data(&mut self, cx: &mut Context<Self>) {
        self.apply_clear_page_view_data(cx);
    }

    pub fn set_page_local_styles_view_data(
        &mut self,
        view_data: DesignPageLocalStylesViewData,
        cx: &mut Context<Self>,
    ) {
        self.apply_page_local_styles_view_data(view_data, cx);
    }

    pub fn clear_page_local_styles_view_data(&mut self, cx: &mut Context<Self>) {
        self.apply_clear_page_local_styles_view_data(cx);
    }

    pub fn set_variables_entry_point(
        &mut self,
        entry_point: DesignVariablesEntryPoint,
        cx: &mut Context<Self>,
    ) {
        self.apply_variables_entry_point(entry_point, cx);
    }

    pub fn set_variable_mode_view_data(
        &mut self,
        view_data: DesignVariableModeViewData,
        cx: &mut Context<Self>,
    ) {
        self.apply_variable_mode_view_data(view_data, cx);
    }

    pub fn clear_variable_mode_view_data(&mut self, cx: &mut Context<Self>) {
        self.apply_clear_variable_mode_view_data(cx);
    }

    pub fn set_viewer_properties_view_data(
        &mut self,
        view_data: DesignViewerPropertiesViewData,
        cx: &mut Context<Self>,
    ) {
        self.apply_viewer_properties_view_data(view_data, cx);
    }

    pub fn clear_viewer_properties_view_data(&mut self, cx: &mut Context<Self>) {
        self.apply_clear_viewer_properties_view_data(cx);
    }

    pub fn set_selection_header_view_data(
        &mut self,
        view_data: DesignSelectionHeaderViewData,
        cx: &mut Context<Self>,
    ) {
        self.apply_selection_header_view_data(view_data, cx);
    }

    pub fn set_selection_header_view_data_for_target(
        &mut self,
        target: DesignPanelTarget,
        view_data: DesignSelectionHeaderViewData,
        cx: &mut Context<Self>,
    ) {
        self.apply_selection_header_view_data_for_target(target, view_data, cx);
    }

    pub fn clear_selection_header_view_data(&mut self, cx: &mut Context<Self>) {
        self.apply_clear_selection_header_view_data(cx);
    }

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
        self.apply_property_value_states(states, cx);
    }

    pub fn set_property_value_state(
        &mut self,
        property: DesignPanelProperty,
        state: DesignPanelPropertyValueState<DesignPanelValue>,
        cx: &mut Context<Self>,
    ) {
        self.apply_property_value_state(property, state, cx);
    }
}

impl Focusable for DesignPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DesignPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_shell(window, cx)
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
