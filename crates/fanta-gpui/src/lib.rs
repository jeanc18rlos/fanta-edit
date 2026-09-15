#![forbid(unsafe_code)]
//! Engine-decoupled, host-controlled GPUI components for Fanta applications.
//!
//! This crate owns presentation and typed UI intents only. A host supplies
//! immutable view data, keeps domain state, and maps intents to Fanta
//! operations. See the workspace `ARCHITECTURE.md` §2–§4.
//!
//! Source is organized by atomic design tier (ARCHITECTURE.md §16): `atoms/`
//! and `molecules/` hold the shared control layer, `organisms/` the
//! host-facing feature surfaces, `layouts/` the composition shells, and
//! `screens/` complete application-sized workflows. These modules are
//! re-exported at the crate root, so hosts import `fanta_gpui::variables`,
//! never a tier path. The [`atoms`] and
//! [`molecules`] tiers are themselves curated public API: hosts build
//! consistent custom chrome from them instead of re-implementing activation,
//! focus rings, icons, or menu/popup clamping.

pub mod atoms;
mod color;
mod layouts;
pub mod molecules;
mod organisms;
mod screens;
#[cfg(test)]
pub(crate) mod test_support;
mod text_input_fallback;

pub use atoms::ActivateControl;
pub use layouts::{file_inspector, pseudo_editor};
pub use organisms::{design, layers, pages, prototype, timeline, toolbar};
pub use screens::variables;

/// Registers Fanta GPUI commands and their default key bindings.
///
/// Call this once after `gpui_component::init`.
pub fn init(cx: &mut gpui::App) {
    text_input_fallback::init(cx);
    atoms::init(cx);
    design::init(cx);
    layers::init(cx);
    pages::init(cx);
    toolbar::init(cx);
}

/// Common imports for Fanta GPUI hosts.
pub mod prelude {
    pub use crate::ActivateControl;
    // Shared control atoms (ARCHITECTURE.md §16).
    pub use crate::atoms::{
        ActivateEvent, ButtonControlExt, CONTROL_KEY_CONTEXT, ControlExt, LucideIcon, icon_button,
        render_lucide_icon, track_bounds, truncating_label,
    };
    pub use crate::design::*;
    pub use crate::file_inspector::{
        FILE_INSPECTOR_MIN_HEIGHT, FILE_INSPECTOR_MIN_WIDTH, FileInspectorSidebar,
    };
    pub use crate::layers::{
        CloseLayersOverlay, CollapseLayer, ConfirmLayersTextEntry, ExpandLayer, FocusNextLayer,
        FocusPreviousLayer, LAYERS_PANEL_MIN_HEIGHT, LAYERS_PANEL_MIN_WIDTH, LayersDropValidator,
        LayersPanel, LayersPanelAction, LayersPanelContextAction, LayersPanelDropPosition,
        LayersPanelItem, LayersPanelNodeKind, LayersPanelSelectionMode, OpenLayerContextMenu,
        ToggleLayerLock, ToggleLayerVisibility,
    };
    // Shared chrome molecules (ARCHITECTURE.md §16).
    pub use crate::molecules::{
        EdgeFades, InspectorCheckboxField, InspectorColorField, InspectorColorSwatch,
        InspectorControlledEdit, InspectorDensity, InspectorEdit, InspectorEditPhase,
        InspectorEmptyState, InspectorFeedback, InspectorFeedbackKind, InspectorFieldAccess,
        InspectorFieldFrame, InspectorFieldMessage, InspectorFieldMessageKind,
        InspectorFieldPresentation, InspectorGridLayout, InspectorLabelPlacement, InspectorMetrics,
        InspectorNumberDraft, InspectorNumberField, InspectorOverlayDismissCause,
        InspectorOverlayDismissIntent, InspectorOverlayFocusTarget, InspectorOverlayPlacement,
        InspectorPickerField, InspectorSliderField, InspectorTextField, InspectorToggleField,
        InspectorValue, POPUP_SAFE_MARGIN, anchored_popup, clamp_menu_origin,
        horizontal_fade_overlays, inspector_action_button, inspector_action_group,
        inspector_anchored_menu, inspector_anchored_menu_surface, inspector_anchored_overlay,
        inspector_anchored_popover, inspector_checkbox_field, inspector_collection_row,
        inspector_color_field, inspector_color_swatch, inspector_field_frame,
        inspector_field_frame_with_presentation, inspector_field_grid,
        inspector_field_grid_with_layout, inspector_field_group, inspector_field_label,
        inspector_grouped_field_frame, inspector_menu_item, inspector_menu_surface,
        inspector_number_field, inspector_picker_field, inspector_popover_surface, inspector_row,
        inspector_row_with_layout, inspector_section, inspector_section_group,
        inspector_section_header, inspector_segment, inspector_segmented_control,
        inspector_slider_field, inspector_text_field, inspector_toggle_field, list_row, menu_item,
        menu_surface, popup_height, popup_max_height, popup_surface, popup_width,
        track_horizontal_edge_fades,
    };
    pub use crate::pages::{
        AddPage, ClosePagesSearch, ConfirmPagesTextEntry, FindInPages, FocusFirstPage,
        FocusLastPage, FocusNextPage, FocusPreviousPage, NextSearchResult, OpenPageContextMenu,
        PAGES_PANEL_MIN_HEIGHT, PAGES_PANEL_MIN_WIDTH, PagesPanel, PagesPanelAction,
        PagesPanelElementCount, PagesPanelElementKind, PagesPanelItem, PagesPanelResultDirection,
        PagesPanelSearchRequest, PagesPanelSearchResult, PagesPanelSearchResults,
        PagesPanelSearchScope, PreviousSearchResult, ReplaceAllResults, ReplaceCurrentResult,
        TogglePagesPanel, ToggleSearchSettings,
    };
    pub use crate::prototype::{
        PROTOTYPE_PANEL_MIN_HEIGHT, PROTOTYPE_PANEL_MIN_WIDTH, PrototypeHint, PrototypePanel,
        PrototypePanelAction, PrototypePanelSurface, PrototypeViewData,
    };
    pub use crate::pseudo_editor::{
        PSEUDO_EDITOR_MIN_HEIGHT, PSEUDO_EDITOR_MIN_WIDTH, PSEUDO_EDITOR_PREFERRED_WIDTH,
        PseudoEditor, PseudoEditorAction, PseudoEditorChildren, PseudoEditorLeftSurface,
        PseudoEditorRightSurface,
    };
    pub use crate::timeline::{TIMELINE_MIN_WIDTH, Timeline, TimelineAction, TimelineViewData};
    pub use crate::toolbar::{
        AgentToolbarOptions, CloseToolbarOverlay, ConfirmToolbarTextEntry, DecrementToolbarControl,
        DevToolbarOptions, EditorToolbar, EnterDevMode, FirstToolbarCommand,
        IncrementToolbarControl, LastToolbarCommand, MotionToolbarOptions, NextToolbarCommand,
        OpenToolbarActions, OpenToolbarAgent, PreviousToolbarCommand, SelectAnnotationTool,
        SelectArrowTool, SelectCommentTool, SelectEllipseTool, SelectFrameTool, SelectHandTool,
        SelectImageVideoTool, SelectLineTool, SelectMeasureTool, SelectMoveTool, SelectPenTool,
        SelectPencilTool, SelectRectangleTool, SelectResourcesTool, SelectScaleTool,
        SelectSectionTool, SelectSliceTool, SelectTextTool, TOOLBAR_ZOOM_CLUSTER_MIN_WIDTH,
        TOOLBAR_ZOOM_STEPPERS_MIN_WIDTH, ToolbarAction, ToolbarChromeControl, ToolbarCommand,
        ToolbarControlValue, ToolbarItem, ToolbarMode, ToolbarSecondaryControl, ToolbarTool,
        ToolbarToolGroup, ZoomCanvasTo100, ZoomCanvasToFit, ZoomCanvasToSelection,
    };
    pub use crate::variables::{
        VARIABLES_SCREEN_MIN_HEIGHT, VARIABLES_SCREEN_MIN_WIDTH, VariableKind, VariableModeValue,
        VariableRow, VariablesAction, VariablesCollection, VariablesGroup, VariablesMode,
        VariablesScreen, VariablesViewData,
    };
}
