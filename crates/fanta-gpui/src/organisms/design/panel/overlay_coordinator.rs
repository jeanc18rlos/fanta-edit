//! Central presentation state for transient Design-inspector surfaces.
//!
//! Payloads remain stable host-facing identities, but their openness is owned
//! in one place. The facade is responsible for emitting Cancel intents before
//! clearing an overlay whose child has an active edit transaction.

use super::*;
use crate::molecules::{
    InspectorOverlayDismissCause, InspectorOverlayDismissIntent, InspectorOverlayFocusTarget,
};

const DESIGN_OVERLAY_PRIORITY: [DesignOpenOverlay; 28] = [
    DesignOpenOverlay::ComponentPropertyEdit,
    DesignOpenOverlay::ComponentPropertyContextMenu,
    DesignOpenOverlay::ComponentPropertyCreateMenu,
    DesignOpenOverlay::AuxiliaryColorPicker,
    DesignOpenOverlay::PageBackground,
    DesignOpenOverlay::PageResource,
    DesignOpenOverlay::VariableMode,
    DesignOpenOverlay::FramePreset,
    DesignOpenOverlay::ComponentPropertyVariable,
    DesignOpenOverlay::PropertyVariable,
    DesignOpenOverlay::ComponentSwap,
    DesignOpenOverlay::TypographyStyle,
    DesignOpenOverlay::FontBrowser,
    DesignOpenOverlay::SelectionHeader,
    DesignOpenOverlay::PaintStyle,
    DesignOpenOverlay::EffectStyle,
    DesignOpenOverlay::LayoutGridStyle,
    DesignOpenOverlay::PreviewOptionMenu,
    DesignOpenOverlay::AppearanceBlendMode,
    DesignOpenOverlay::DimensionMenu,
    DesignOpenOverlay::ExportChoice,
    DesignOpenOverlay::EffectSettings,
    DesignOpenOverlay::SelectionColorResource,
    DesignOpenOverlay::LayoutGridCountVariable,
    DesignOpenOverlay::MenuPreview,
    DesignOpenOverlay::PaintPicker,
    DesignOpenOverlay::TypeSettings,
    DesignOpenOverlay::GridDimensions,
];

// These sets deliberately mirror the peers closed by the corresponding
// retained surface before the coordinator existed. Keeping the policy beside
// the overlay slots prevents renderers from growing slightly different lists
// of fields to clear when another trigger is added.
const GRID_DIMENSIONS_CONFLICTS: &[DesignOpenOverlay] = &[
    DesignOpenOverlay::PaintPicker,
    DesignOpenOverlay::AuxiliaryColorPicker,
    DesignOpenOverlay::EffectSettings,
    DesignOpenOverlay::PaintStyle,
    DesignOpenOverlay::EffectStyle,
    DesignOpenOverlay::PropertyVariable,
    DesignOpenOverlay::ComponentPropertyVariable,
    DesignOpenOverlay::ComponentSwap,
    DesignOpenOverlay::LayoutGridStyle,
    DesignOpenOverlay::FramePreset,
    DesignOpenOverlay::TypographyStyle,
    DesignOpenOverlay::FontBrowser,
    DesignOpenOverlay::TypeSettings,
    DesignOpenOverlay::SelectionHeader,
];

const FRAME_PRESET_CONFLICTS: &[DesignOpenOverlay] = &[
    DesignOpenOverlay::PaintPicker,
    DesignOpenOverlay::AuxiliaryColorPicker,
    DesignOpenOverlay::EffectSettings,
    DesignOpenOverlay::EffectStyle,
    DesignOpenOverlay::PropertyVariable,
    DesignOpenOverlay::ComponentPropertyVariable,
    DesignOpenOverlay::ComponentSwap,
    DesignOpenOverlay::TypographyStyle,
    DesignOpenOverlay::FontBrowser,
    DesignOpenOverlay::TypeSettings,
    DesignOpenOverlay::SelectionHeader,
];

const NO_OVERLAY_CONFLICTS: &[DesignOpenOverlay] = &[];
const PAGE_RESOURCE_CONFLICTS: &[DesignOpenOverlay] = &[
    DesignOpenOverlay::PageBackground,
    DesignOpenOverlay::VariableMode,
];
const PAGE_BACKGROUND_CONFLICTS: &[DesignOpenOverlay] = &[
    DesignOpenOverlay::PageResource,
    DesignOpenOverlay::VariableMode,
];
const VARIABLE_MODE_CONFLICTS: &[DesignOpenOverlay] = &[
    DesignOpenOverlay::PageBackground,
    DesignOpenOverlay::PageResource,
    DesignOpenOverlay::AppearanceBlendMode,
];
const APPEARANCE_BLEND_MODE_CONFLICTS: &[DesignOpenOverlay] = &[
    DesignOpenOverlay::PaintPicker,
    DesignOpenOverlay::TypeSettings,
];
const TYPE_SETTINGS_CONFLICTS: &[DesignOpenOverlay] = &[
    DesignOpenOverlay::PaintPicker,
    DesignOpenOverlay::TypographyStyle,
    DesignOpenOverlay::FontBrowser,
];
const PROPERTY_VARIABLE_CONFLICTS: &[DesignOpenOverlay] = &[
    DesignOpenOverlay::ComponentPropertyVariable,
    DesignOpenOverlay::ComponentSwap,
    DesignOpenOverlay::PaintPicker,
    DesignOpenOverlay::EffectSettings,
    DesignOpenOverlay::GridDimensions,
    DesignOpenOverlay::EffectStyle,
    DesignOpenOverlay::LayoutGridStyle,
    DesignOpenOverlay::LayoutGridCountVariable,
    DesignOpenOverlay::TypographyStyle,
    DesignOpenOverlay::TypeSettings,
    DesignOpenOverlay::SelectionHeader,
];
const COMPONENT_PROPERTY_VARIABLE_CONFLICTS: &[DesignOpenOverlay] = &[
    DesignOpenOverlay::ComponentSwap,
    DesignOpenOverlay::PropertyVariable,
    DesignOpenOverlay::PaintPicker,
    DesignOpenOverlay::EffectSettings,
    DesignOpenOverlay::TypeSettings,
];
const COMPONENT_SWAP_CONFLICTS: &[DesignOpenOverlay] = &[
    DesignOpenOverlay::ComponentPropertyVariable,
    DesignOpenOverlay::PropertyVariable,
    DesignOpenOverlay::PaintPicker,
    DesignOpenOverlay::EffectSettings,
    DesignOpenOverlay::TypeSettings,
];
const PAINT_PICKER_CONFLICTS: &[DesignOpenOverlay] = &[
    DesignOpenOverlay::AuxiliaryColorPicker,
    DesignOpenOverlay::PaintStyle,
    DesignOpenOverlay::TypographyStyle,
    DesignOpenOverlay::TypeSettings,
];
const AUXILIARY_COLOR_PICKER_CONFLICTS: &[DesignOpenOverlay] = &[
    DesignOpenOverlay::PaintPicker,
    DesignOpenOverlay::PaintStyle,
    DesignOpenOverlay::EffectStyle,
    DesignOpenOverlay::TypographyStyle,
    DesignOpenOverlay::SelectionHeader,
];
const PAINT_STYLE_CONFLICTS: &[DesignOpenOverlay] = &[
    DesignOpenOverlay::AuxiliaryColorPicker,
    DesignOpenOverlay::PaintPicker,
    DesignOpenOverlay::EffectSettings,
    DesignOpenOverlay::EffectStyle,
    DesignOpenOverlay::LayoutGridStyle,
    DesignOpenOverlay::TypographyStyle,
    DesignOpenOverlay::TypeSettings,
    DesignOpenOverlay::SelectionHeader,
];
const EFFECT_STYLE_CONFLICTS: &[DesignOpenOverlay] = &[
    DesignOpenOverlay::PaintStyle,
    DesignOpenOverlay::EffectSettings,
    DesignOpenOverlay::PaintPicker,
    DesignOpenOverlay::TypographyStyle,
    DesignOpenOverlay::TypeSettings,
    DesignOpenOverlay::SelectionHeader,
];
const LAYOUT_GRID_STYLE_CONFLICTS: &[DesignOpenOverlay] = &[
    DesignOpenOverlay::PaintStyle,
    DesignOpenOverlay::LayoutGridCountVariable,
    DesignOpenOverlay::EffectSettings,
    DesignOpenOverlay::EffectStyle,
    DesignOpenOverlay::PaintPicker,
    DesignOpenOverlay::TypographyStyle,
    DesignOpenOverlay::TypeSettings,
    DesignOpenOverlay::SelectionHeader,
];
const LAYOUT_GRID_VARIABLE_CONFLICTS: &[DesignOpenOverlay] = &[
    DesignOpenOverlay::LayoutGridStyle,
    DesignOpenOverlay::EffectSettings,
    DesignOpenOverlay::EffectStyle,
    DesignOpenOverlay::PaintPicker,
    DesignOpenOverlay::TypographyStyle,
    DesignOpenOverlay::TypeSettings,
    DesignOpenOverlay::SelectionHeader,
];
const TYPOGRAPHY_STYLE_CONFLICTS: &[DesignOpenOverlay] = &[
    DesignOpenOverlay::PaintStyle,
    DesignOpenOverlay::FontBrowser,
    DesignOpenOverlay::PaintPicker,
    DesignOpenOverlay::EffectSettings,
    DesignOpenOverlay::EffectStyle,
    DesignOpenOverlay::TypeSettings,
    DesignOpenOverlay::SelectionHeader,
];
const FONT_BROWSER_CONFLICTS: &[DesignOpenOverlay] = &[
    DesignOpenOverlay::PaintStyle,
    DesignOpenOverlay::TypographyStyle,
    DesignOpenOverlay::TypeSettings,
    DesignOpenOverlay::PaintPicker,
    DesignOpenOverlay::EffectSettings,
    DesignOpenOverlay::EffectStyle,
    DesignOpenOverlay::SelectionHeader,
];
const SELECTION_HEADER_CONFLICTS: &[DesignOpenOverlay] = &[
    DesignOpenOverlay::PaintPicker,
    DesignOpenOverlay::EffectSettings,
    DesignOpenOverlay::EffectStyle,
    DesignOpenOverlay::TypeSettings,
    DesignOpenOverlay::AppearanceBlendMode,
];
const EFFECT_SETTINGS_CONFLICTS: &[DesignOpenOverlay] = &[
    DesignOpenOverlay::EffectStyle,
    DesignOpenOverlay::PaintPicker,
    DesignOpenOverlay::TypographyStyle,
];
const SELECTION_COLOR_RESOURCE_CONFLICTS: &[DesignOpenOverlay] = &[
    DesignOpenOverlay::AuxiliaryColorPicker,
    DesignOpenOverlay::PaintPicker,
    DesignOpenOverlay::PaintStyle,
    DesignOpenOverlay::EffectStyle,
    DesignOpenOverlay::LayoutGridStyle,
    DesignOpenOverlay::TypographyStyle,
    DesignOpenOverlay::SelectionHeader,
];

/// Coarse overlay identity used for deterministic dismissal and diagnostics.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum DesignOpenOverlay {
    ComponentPropertyEdit,
    ComponentPropertyContextMenu,
    ComponentPropertyCreateMenu,
    AuxiliaryColorPicker,
    PageBackground,
    PageResource,
    VariableMode,
    FramePreset,
    ComponentPropertyVariable,
    PropertyVariable,
    ComponentSwap,
    TypographyStyle,
    FontBrowser,
    SelectionHeader,
    PaintStyle,
    EffectStyle,
    LayoutGridStyle,
    PreviewOptionMenu,
    AppearanceBlendMode,
    DimensionMenu,
    ExportChoice,
    EffectSettings,
    SelectionColorResource,
    LayoutGridCountVariable,
    MenuPreview,
    PaintPicker,
    TypeSettings,
    GridDimensions,
}

/// Typed payload used for every coordinator-owned open or replacement
/// transition. Callers choose the surface and payload; the coordinator owns
/// slot mutation, mutual exclusion, and stale focus-return cleanup.
#[derive(Clone)]
pub(super) enum DesignOverlayState {
    ComponentPropertyEdit,
    ComponentPropertyContextMenu(SharedString),
    ComponentPropertyCreateMenu,
    AuxiliaryColorPicker(AuxiliaryColorPickerTarget),
    PageBackground,
    PageResource(DesignLocalResourceCategory),
    VariableMode,
    FramePreset,
    ComponentPropertyVariable(DesignComponentPropertyVariableTarget),
    PropertyVariable(DesignPanelProperty),
    ComponentSwap(SharedString),
    TypographyStyle,
    FontBrowser,
    SelectionHeader(SelectionHeaderOverlay),
    PaintStyle(DesignPanelCollection),
    EffectStyle,
    LayoutGridStyle,
    PreviewOptionMenu(DesignPanelProperty),
    AppearanceBlendMode,
    DimensionMenu(DesignLayoutDimensionAxis),
    ExportChoice(SharedString),
    EffectSettings(EffectSettingsTarget),
    SelectionColorResource(SelectionColorResourceTarget),
    LayoutGridCountVariable(DesignLayoutGridVariableTarget),
    MenuPreview(Box<DesignMenuPreview>),
    PaintPicker(PaintPickerTarget),
    TypeSettings,
    GridDimensions(GridDimensionsPicker),
}

impl DesignOverlayState {
    pub(super) const fn overlay(&self) -> DesignOpenOverlay {
        match self {
            Self::ComponentPropertyEdit => DesignOpenOverlay::ComponentPropertyEdit,
            Self::ComponentPropertyContextMenu(_) => {
                DesignOpenOverlay::ComponentPropertyContextMenu
            }
            Self::ComponentPropertyCreateMenu => DesignOpenOverlay::ComponentPropertyCreateMenu,
            Self::AuxiliaryColorPicker(_) => DesignOpenOverlay::AuxiliaryColorPicker,
            Self::PageBackground => DesignOpenOverlay::PageBackground,
            Self::PageResource(_) => DesignOpenOverlay::PageResource,
            Self::VariableMode => DesignOpenOverlay::VariableMode,
            Self::FramePreset => DesignOpenOverlay::FramePreset,
            Self::ComponentPropertyVariable(_) => DesignOpenOverlay::ComponentPropertyVariable,
            Self::PropertyVariable(_) => DesignOpenOverlay::PropertyVariable,
            Self::ComponentSwap(_) => DesignOpenOverlay::ComponentSwap,
            Self::TypographyStyle => DesignOpenOverlay::TypographyStyle,
            Self::FontBrowser => DesignOpenOverlay::FontBrowser,
            Self::SelectionHeader(_) => DesignOpenOverlay::SelectionHeader,
            Self::PaintStyle(_) => DesignOpenOverlay::PaintStyle,
            Self::EffectStyle => DesignOpenOverlay::EffectStyle,
            Self::LayoutGridStyle => DesignOpenOverlay::LayoutGridStyle,
            Self::PreviewOptionMenu(_) => DesignOpenOverlay::PreviewOptionMenu,
            Self::AppearanceBlendMode => DesignOpenOverlay::AppearanceBlendMode,
            Self::DimensionMenu(_) => DesignOpenOverlay::DimensionMenu,
            Self::ExportChoice(_) => DesignOpenOverlay::ExportChoice,
            Self::EffectSettings(_) => DesignOpenOverlay::EffectSettings,
            Self::SelectionColorResource(_) => DesignOpenOverlay::SelectionColorResource,
            Self::LayoutGridCountVariable(_) => DesignOpenOverlay::LayoutGridCountVariable,
            Self::MenuPreview(_) => DesignOpenOverlay::MenuPreview,
            Self::PaintPicker(_) => DesignOpenOverlay::PaintPicker,
            Self::TypeSettings => DesignOpenOverlay::TypeSettings,
            Self::GridDimensions(_) => DesignOpenOverlay::GridDimensions,
        }
    }
}

#[derive(Default)]
struct DesignOverlayFocusHandles {
    dimension_menu: Option<[FocusHandle; 2]>,
    type_settings: Option<FocusHandle>,
}

#[derive(Default)]
pub(super) struct DesignOverlayCoordinator {
    /// Native component create/edit dialog visibility. Its draft remains in
    /// the feature controller, while ownership, priority, and focus return
    /// live with every other Design-inspector overlay here.
    component_authoring_dialog_open: bool,
    selection_header_overlay: Option<SelectionHeaderOverlay>,
    export_choice_overlay: Option<SharedString>,
    appearance_blend_mode_open: bool,
    preview_option_menu_open: Option<DesignPanelProperty>,
    active_menu_preview: Option<DesignMenuPreview>,
    active_picker: Option<PaintPickerTarget>,
    auxiliary_color_picker: Option<AuxiliaryColorPickerTarget>,
    active_effect_settings: Option<EffectSettingsTarget>,
    paint_style_browser_open: Option<DesignPanelCollection>,
    selection_color_resource_browser: Option<SelectionColorResourceTarget>,
    effect_style_browser_open: bool,
    property_variable_picker: Option<DesignPanelProperty>,
    component_property_variable_picker: Option<DesignComponentPropertyVariableTarget>,
    component_swap_browser: Option<SharedString>,
    component_property_context_menu: Option<SharedString>,
    component_property_create_menu_open: bool,
    layout_grid_style_browser_open: bool,
    layout_grid_count_variable_target: Option<DesignLayoutGridVariableTarget>,
    frame_preset_browser_open: bool,
    page_background_picker_open: bool,
    page_resource_browser: Option<DesignLocalResourceCategory>,
    variable_mode_browser_open: bool,
    typography_style_picker_open: bool,
    font_browser_open: bool,
    type_settings_open: bool,
    grid_dimensions_picker: Option<GridDimensionsPicker>,
    dimension_menu_open: Option<DesignLayoutDimensionAxis>,
    focus_returns: HashMap<DesignOpenOverlay, InspectorOverlayFocusTarget>,
    generations: HashMap<DesignOpenOverlay, u64>,
    next_generation: u64,
    focus_handles: DesignOverlayFocusHandles,
}

impl DesignOverlayCoordinator {
    pub(super) fn with_focus_handles(cx: &mut App) -> Self {
        Self {
            focus_handles: DesignOverlayFocusHandles {
                dimension_menu: Some([
                    cx.focus_handle().tab_index(0).tab_stop(true),
                    cx.focus_handle().tab_index(0).tab_stop(true),
                ]),
                type_settings: Some(cx.focus_handle()),
            },
            ..Self::default()
        }
    }

    pub(super) fn dimension_menu_focus(&self, axis: DesignLayoutDimensionAxis) -> &FocusHandle {
        &self
            .focus_handles
            .dimension_menu
            .as_ref()
            .expect("DesignPanel initializes overlay focus handles")[match axis {
            DesignLayoutDimensionAxis::Width => 0,
            DesignLayoutDimensionAxis::Height => 1,
        }]
    }

    pub(super) fn type_settings_focus(&self) -> &FocusHandle {
        self.focus_handles
            .type_settings
            .as_ref()
            .expect("DesignPanel initializes overlay focus handles")
    }

    pub(super) const fn component_authoring_dialog_open(&self) -> bool {
        self.component_authoring_dialog_open
    }

    pub(super) fn selection_header_overlay(&self) -> Option<SelectionHeaderOverlay> {
        self.selection_header_overlay.clone()
    }

    pub(super) fn export_choice_overlay(&self) -> Option<SharedString> {
        self.export_choice_overlay.clone()
    }

    pub(super) const fn appearance_blend_mode_open(&self) -> bool {
        self.appearance_blend_mode_open
    }

    pub(super) const fn preview_option_menu_open(&self) -> Option<DesignPanelProperty> {
        self.preview_option_menu_open
    }

    pub(super) fn active_menu_preview(&self) -> Option<DesignMenuPreview> {
        self.active_menu_preview.clone()
    }

    pub(super) fn active_picker(&self) -> Option<PaintPickerTarget> {
        self.active_picker.clone()
    }

    pub(super) fn auxiliary_color_picker(&self) -> Option<AuxiliaryColorPickerTarget> {
        self.auxiliary_color_picker.clone()
    }

    pub(super) fn active_effect_settings(&self) -> Option<EffectSettingsTarget> {
        self.active_effect_settings.clone()
    }

    pub(super) const fn paint_style_browser_open(&self) -> Option<DesignPanelCollection> {
        self.paint_style_browser_open
    }

    pub(super) fn selection_color_resource_browser(&self) -> Option<SelectionColorResourceTarget> {
        self.selection_color_resource_browser.clone()
    }

    pub(super) const fn effect_style_browser_open(&self) -> bool {
        self.effect_style_browser_open
    }

    pub(super) const fn property_variable_picker(&self) -> Option<DesignPanelProperty> {
        self.property_variable_picker
    }

    pub(super) fn component_property_variable_picker(
        &self,
    ) -> Option<DesignComponentPropertyVariableTarget> {
        self.component_property_variable_picker.clone()
    }

    pub(super) fn component_swap_browser(&self) -> Option<SharedString> {
        self.component_swap_browser.clone()
    }

    pub(super) fn component_property_context_menu(&self) -> Option<SharedString> {
        self.component_property_context_menu.clone()
    }

    pub(super) const fn component_property_create_menu_open(&self) -> bool {
        self.component_property_create_menu_open
    }

    pub(super) const fn layout_grid_style_browser_open(&self) -> bool {
        self.layout_grid_style_browser_open
    }

    pub(super) fn layout_grid_count_variable_target(
        &self,
    ) -> Option<DesignLayoutGridVariableTarget> {
        self.layout_grid_count_variable_target.clone()
    }

    pub(super) const fn page_background_picker_open(&self) -> bool {
        self.page_background_picker_open
    }

    pub(super) const fn page_resource_browser(&self) -> Option<DesignLocalResourceCategory> {
        self.page_resource_browser
    }

    pub(super) const fn variable_mode_browser_open(&self) -> bool {
        self.variable_mode_browser_open
    }

    pub(super) const fn typography_style_picker_open(&self) -> bool {
        self.typography_style_picker_open
    }

    pub(super) const fn font_browser_open(&self) -> bool {
        self.font_browser_open
    }

    pub(super) const fn type_settings_open(&self) -> bool {
        self.type_settings_open
    }

    pub(super) const fn dimension_menu_open(&self) -> Option<DesignLayoutDimensionAxis> {
        self.dimension_menu_open
    }

    // Legacy panel interaction tests seed otherwise host-driven overlay
    // states directly. Keep those fixtures behind cfg(test) while production
    // code is restricted to the typed transitions below.
    #[cfg(test)]
    pub(super) fn active_picker_test_slot(&mut self) -> &mut Option<PaintPickerTarget> {
        &mut self.active_picker
    }

    #[cfg(test)]
    pub(super) fn auxiliary_color_picker_test_slot(
        &mut self,
    ) -> &mut Option<AuxiliaryColorPickerTarget> {
        &mut self.auxiliary_color_picker
    }

    #[cfg(test)]
    pub(super) fn selection_header_overlay_test_slot(
        &mut self,
    ) -> &mut Option<SelectionHeaderOverlay> {
        &mut self.selection_header_overlay
    }

    #[cfg(test)]
    pub(super) fn appearance_blend_mode_open_test_slot(&mut self) -> &mut bool {
        &mut self.appearance_blend_mode_open
    }

    #[cfg(test)]
    pub(super) fn type_settings_open_test_slot(&mut self) -> &mut bool {
        &mut self.type_settings_open
    }

    #[cfg(test)]
    pub(super) fn component_property_context_menu_test_slot(
        &mut self,
    ) -> &mut Option<SharedString> {
        &mut self.component_property_context_menu
    }

    #[cfg(test)]
    pub(super) fn page_resource_browser_test_slot(
        &mut self,
    ) -> &mut Option<DesignLocalResourceCategory> {
        &mut self.page_resource_browser
    }

    #[cfg(test)]
    pub(super) fn export_choice_overlay_test_slot(&mut self) -> &mut Option<SharedString> {
        &mut self.export_choice_overlay
    }

    #[cfg(test)]
    pub(super) fn preview_option_menu_open_test_slot(
        &mut self,
    ) -> &mut Option<DesignPanelProperty> {
        &mut self.preview_option_menu_open
    }

    #[cfg(test)]
    pub(super) fn page_background_picker_open_test_slot(&mut self) -> &mut bool {
        &mut self.page_background_picker_open
    }

    #[cfg(test)]
    pub(super) fn active_effect_settings_test_slot(&mut self) -> &mut Option<EffectSettingsTarget> {
        &mut self.active_effect_settings
    }

    fn conflicts_for(overlay: DesignOpenOverlay) -> &'static [DesignOpenOverlay] {
        match overlay {
            DesignOpenOverlay::GridDimensions => GRID_DIMENSIONS_CONFLICTS,
            DesignOpenOverlay::FramePreset => FRAME_PRESET_CONFLICTS,
            DesignOpenOverlay::PageResource => PAGE_RESOURCE_CONFLICTS,
            DesignOpenOverlay::PageBackground => PAGE_BACKGROUND_CONFLICTS,
            DesignOpenOverlay::VariableMode => VARIABLE_MODE_CONFLICTS,
            DesignOpenOverlay::AppearanceBlendMode => APPEARANCE_BLEND_MODE_CONFLICTS,
            DesignOpenOverlay::TypeSettings => TYPE_SETTINGS_CONFLICTS,
            DesignOpenOverlay::PropertyVariable => PROPERTY_VARIABLE_CONFLICTS,
            DesignOpenOverlay::ComponentPropertyVariable => COMPONENT_PROPERTY_VARIABLE_CONFLICTS,
            DesignOpenOverlay::ComponentSwap => COMPONENT_SWAP_CONFLICTS,
            DesignOpenOverlay::PaintPicker => PAINT_PICKER_CONFLICTS,
            DesignOpenOverlay::AuxiliaryColorPicker => AUXILIARY_COLOR_PICKER_CONFLICTS,
            DesignOpenOverlay::PaintStyle => PAINT_STYLE_CONFLICTS,
            DesignOpenOverlay::EffectStyle => EFFECT_STYLE_CONFLICTS,
            DesignOpenOverlay::LayoutGridStyle => LAYOUT_GRID_STYLE_CONFLICTS,
            DesignOpenOverlay::LayoutGridCountVariable => LAYOUT_GRID_VARIABLE_CONFLICTS,
            DesignOpenOverlay::TypographyStyle => TYPOGRAPHY_STYLE_CONFLICTS,
            DesignOpenOverlay::FontBrowser => FONT_BROWSER_CONFLICTS,
            DesignOpenOverlay::SelectionHeader => SELECTION_HEADER_CONFLICTS,
            DesignOpenOverlay::EffectSettings => EFFECT_SETTINGS_CONFLICTS,
            DesignOpenOverlay::SelectionColorResource => SELECTION_COLOR_RESOURCE_CONFLICTS,
            _ => NO_OVERLAY_CONFLICTS,
        }
    }

    /// Opens a typed surface after applying its coordinator-owned mutual
    /// exclusion policy.
    pub(super) fn open(&mut self, state: DesignOverlayState) {
        let overlay = state.overlay();
        self.discard_many(Self::conflicts_for(overlay));
        self.replace(state);
    }

    /// Replaces a payload without applying peer exclusion. This is reserved
    /// for host echo/reconciliation of an already-open logical surface.
    pub(super) fn replace(&mut self, state: DesignOverlayState) {
        let overlay = state.overlay();
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        self.generations.insert(overlay, self.next_generation);
        match state {
            DesignOverlayState::ComponentPropertyEdit => {
                self.component_authoring_dialog_open = true;
            }
            DesignOverlayState::ComponentPropertyContextMenu(value) => {
                self.component_property_context_menu = Some(value);
            }
            DesignOverlayState::ComponentPropertyCreateMenu => {
                self.component_property_create_menu_open = true;
            }
            DesignOverlayState::AuxiliaryColorPicker(value) => {
                self.auxiliary_color_picker = Some(value);
            }
            DesignOverlayState::PageBackground => self.page_background_picker_open = true,
            DesignOverlayState::PageResource(value) => self.page_resource_browser = Some(value),
            DesignOverlayState::VariableMode => self.variable_mode_browser_open = true,
            DesignOverlayState::FramePreset => self.frame_preset_browser_open = true,
            DesignOverlayState::ComponentPropertyVariable(value) => {
                self.component_property_variable_picker = Some(value);
            }
            DesignOverlayState::PropertyVariable(value) => {
                self.property_variable_picker = Some(value);
            }
            DesignOverlayState::ComponentSwap(value) => {
                self.component_swap_browser = Some(value);
            }
            DesignOverlayState::TypographyStyle => self.typography_style_picker_open = true,
            DesignOverlayState::FontBrowser => self.font_browser_open = true,
            DesignOverlayState::SelectionHeader(value) => {
                self.selection_header_overlay = Some(value);
            }
            DesignOverlayState::PaintStyle(value) => {
                self.paint_style_browser_open = Some(value);
            }
            DesignOverlayState::EffectStyle => self.effect_style_browser_open = true,
            DesignOverlayState::LayoutGridStyle => self.layout_grid_style_browser_open = true,
            DesignOverlayState::PreviewOptionMenu(value) => {
                self.preview_option_menu_open = Some(value);
            }
            DesignOverlayState::AppearanceBlendMode => self.appearance_blend_mode_open = true,
            DesignOverlayState::DimensionMenu(value) => self.dimension_menu_open = Some(value),
            DesignOverlayState::ExportChoice(value) => self.export_choice_overlay = Some(value),
            DesignOverlayState::EffectSettings(value) => {
                self.active_effect_settings = Some(value);
            }
            DesignOverlayState::SelectionColorResource(value) => {
                self.selection_color_resource_browser = Some(value);
            }
            DesignOverlayState::LayoutGridCountVariable(value) => {
                self.layout_grid_count_variable_target = Some(value);
            }
            DesignOverlayState::MenuPreview(value) => self.active_menu_preview = Some(*value),
            DesignOverlayState::PaintPicker(value) => self.active_picker = Some(value),
            DesignOverlayState::TypeSettings => self.type_settings_open = true,
            DesignOverlayState::GridDimensions(value) => {
                self.grid_dimensions_picker = Some(value);
            }
        }
    }

    pub(super) fn set_open(&mut self, state: DesignOverlayState, open: bool) {
        if open {
            self.open(state);
        } else {
            self.discard(state.overlay());
        }
    }

    pub(super) fn toggle(&mut self, state: DesignOverlayState) -> bool {
        let overlay = state.overlay();
        if self.is_open(overlay) {
            self.discard(overlay);
            false
        } else {
            self.open(state);
            true
        }
    }

    pub(super) fn take_menu_preview(&mut self) -> Option<DesignMenuPreview> {
        self.active_menu_preview.take()
    }

    pub(super) fn is_open(&self, overlay: DesignOpenOverlay) -> bool {
        match overlay {
            DesignOpenOverlay::ComponentPropertyEdit => self.component_authoring_dialog_open,
            DesignOpenOverlay::ComponentPropertyContextMenu => {
                self.component_property_context_menu.is_some()
            }
            DesignOpenOverlay::ComponentPropertyCreateMenu => {
                self.component_property_create_menu_open
            }
            DesignOpenOverlay::AuxiliaryColorPicker => self.auxiliary_color_picker.is_some(),
            DesignOpenOverlay::PageBackground => self.page_background_picker_open,
            DesignOpenOverlay::PageResource => self.page_resource_browser.is_some(),
            DesignOpenOverlay::VariableMode => self.variable_mode_browser_open,
            DesignOpenOverlay::FramePreset => self.frame_preset_browser_open,
            DesignOpenOverlay::ComponentPropertyVariable => {
                self.component_property_variable_picker.is_some()
            }
            DesignOpenOverlay::PropertyVariable => self.property_variable_picker.is_some(),
            DesignOpenOverlay::ComponentSwap => self.component_swap_browser.is_some(),
            DesignOpenOverlay::TypographyStyle => self.typography_style_picker_open,
            DesignOpenOverlay::FontBrowser => self.font_browser_open,
            DesignOpenOverlay::SelectionHeader => self.selection_header_overlay.is_some(),
            DesignOpenOverlay::PaintStyle => self.paint_style_browser_open.is_some(),
            DesignOpenOverlay::EffectStyle => self.effect_style_browser_open,
            DesignOpenOverlay::LayoutGridStyle => self.layout_grid_style_browser_open,
            DesignOpenOverlay::PreviewOptionMenu => self.preview_option_menu_open.is_some(),
            DesignOpenOverlay::AppearanceBlendMode => self.appearance_blend_mode_open,
            DesignOpenOverlay::DimensionMenu => self.dimension_menu_open.is_some(),
            DesignOpenOverlay::ExportChoice => self.export_choice_overlay.is_some(),
            DesignOpenOverlay::EffectSettings => self.active_effect_settings.is_some(),
            DesignOpenOverlay::SelectionColorResource => {
                self.selection_color_resource_browser.is_some()
            }
            DesignOpenOverlay::LayoutGridCountVariable => {
                self.layout_grid_count_variable_target.is_some()
            }
            DesignOpenOverlay::MenuPreview => self.active_menu_preview.is_some(),
            DesignOpenOverlay::PaintPicker => self.active_picker.is_some(),
            DesignOpenOverlay::TypeSettings => self.type_settings_open,
            DesignOpenOverlay::GridDimensions => self.grid_dimensions_picker.is_some(),
        }
    }

    fn highest_priority_open(&self) -> Option<DesignOpenOverlay> {
        DESIGN_OVERLAY_PRIORITY
            .iter()
            .copied()
            .find(|overlay| self.is_open(*overlay))
    }

    /// Returns the same topmost order used by the facade's Escape handler.
    pub(super) fn topmost(&self) -> Option<DesignOpenOverlay> {
        self.escape_dismissal_intent()
            .map(|intent| *intent.overlay())
    }

    pub(super) const fn frame_preset_is_open(&self) -> bool {
        self.frame_preset_browser_open
    }

    pub(super) const fn grid_dimensions_picker(&self) -> Option<&GridDimensionsPicker> {
        self.grid_dimensions_picker.as_ref()
    }

    /// Opens the atomic Grid dimensions chooser and applies its complete
    /// mutual-exclusion policy as one state transition.
    pub(super) fn open_grid_dimensions(&mut self, picker: GridDimensionsPicker) {
        self.open(DesignOverlayState::GridDimensions(picker));
    }

    pub(super) fn update_grid_dimensions_candidate(
        &mut self,
        candidate: DesignGridDimensions,
    ) -> bool {
        let Some(picker) = self.grid_dimensions_picker.as_mut() else {
            return false;
        };
        if picker.candidate == candidate {
            return false;
        }
        picker.candidate = candidate;
        true
    }

    /// Opens or closes the Frame preset browser. Opening is one atomic
    /// transition that also closes the exact peer surfaces it supersedes.
    pub(super) fn set_frame_preset_open(&mut self, open: bool) {
        self.set_open(DesignOverlayState::FramePreset, open);
    }

    /// Clears a retained surface without restoring focus. This is the path
    /// for replacement, host echo, and context invalidation. User dismissal
    /// goes through [`Self::finish_dismissal`] so its captured focus target is
    /// returned to the facade.
    pub(super) fn discard(&mut self, overlay: DesignOpenOverlay) -> bool {
        let was_open = self.is_open(overlay);
        self.clear_slot(overlay);
        self.focus_returns.remove(&overlay);
        self.generations.remove(&overlay);
        was_open
    }

    pub(super) fn discard_many(&mut self, overlays: &[DesignOpenOverlay]) {
        for overlay in overlays {
            self.discard(*overlay);
        }
    }

    fn clear_slot(&mut self, overlay: DesignOpenOverlay) {
        match overlay {
            DesignOpenOverlay::ComponentPropertyEdit => {
                self.component_authoring_dialog_open = false;
            }
            DesignOpenOverlay::ComponentPropertyContextMenu => {
                self.component_property_context_menu = None;
            }
            DesignOpenOverlay::ComponentPropertyCreateMenu => {
                self.component_property_create_menu_open = false;
            }
            DesignOpenOverlay::AuxiliaryColorPicker => self.auxiliary_color_picker = None,
            DesignOpenOverlay::PageBackground => self.page_background_picker_open = false,
            DesignOpenOverlay::PageResource => self.page_resource_browser = None,
            DesignOpenOverlay::VariableMode => self.variable_mode_browser_open = false,
            DesignOpenOverlay::FramePreset => self.frame_preset_browser_open = false,
            DesignOpenOverlay::ComponentPropertyVariable => {
                self.component_property_variable_picker = None;
            }
            DesignOpenOverlay::PropertyVariable => self.property_variable_picker = None,
            DesignOpenOverlay::ComponentSwap => self.component_swap_browser = None,
            DesignOpenOverlay::TypographyStyle => self.typography_style_picker_open = false,
            DesignOpenOverlay::FontBrowser => self.font_browser_open = false,
            DesignOpenOverlay::SelectionHeader => self.selection_header_overlay = None,
            DesignOpenOverlay::PaintStyle => self.paint_style_browser_open = None,
            DesignOpenOverlay::EffectStyle => self.effect_style_browser_open = false,
            DesignOpenOverlay::LayoutGridStyle => self.layout_grid_style_browser_open = false,
            DesignOpenOverlay::PreviewOptionMenu => self.preview_option_menu_open = None,
            DesignOpenOverlay::AppearanceBlendMode => self.appearance_blend_mode_open = false,
            DesignOpenOverlay::DimensionMenu => self.dimension_menu_open = None,
            DesignOpenOverlay::ExportChoice => self.export_choice_overlay = None,
            DesignOpenOverlay::EffectSettings => self.active_effect_settings = None,
            DesignOpenOverlay::SelectionColorResource => {
                self.selection_color_resource_browser = None;
            }
            DesignOpenOverlay::LayoutGridCountVariable => {
                self.layout_grid_count_variable_target = None;
            }
            DesignOpenOverlay::MenuPreview => self.active_menu_preview = None,
            DesignOpenOverlay::PaintPicker => self.active_picker = None,
            DesignOpenOverlay::TypeSettings => self.type_settings_open = false,
            DesignOpenOverlay::GridDimensions => self.grid_dimensions_picker = None,
        }
    }

    pub(super) fn remember_focus_return(
        &mut self,
        overlay: DesignOpenOverlay,
        handle: FocusHandle,
    ) {
        self.focus_returns
            .insert(overlay, InspectorOverlayFocusTarget::new(handle));
    }

    /// Captures the current trigger focus, falling back to the stable panel
    /// root for pointer-opened surfaces. Keeping the fallback rule here makes
    /// focus restoration part of the same policy as priority and dismissal.
    pub(super) fn capture_focus_return(
        &mut self,
        overlay: DesignOpenOverlay,
        window: &Window,
        fallback: &FocusHandle,
        cx: &App,
    ) {
        let handle = window.focused(cx).unwrap_or_else(|| fallback.clone());
        self.remember_focus_return(overlay, handle);
    }

    pub(super) fn forget_focus_return(&mut self, overlay: DesignOpenOverlay) {
        self.focus_returns.remove(&overlay);
    }

    pub(super) fn escape_dismissal_intent(
        &self,
    ) -> Option<InspectorOverlayDismissIntent<DesignOpenOverlay>> {
        let overlay = self.highest_priority_open()?;
        self.escape_dismissal_intent_for(overlay)
    }

    /// Produces an Escape intent only when `overlay` is the topmost open
    /// surface. Item-level keyboard handlers use this exact-overlay variant so
    /// a covered popover cannot clear its own slot or steal focus restoration
    /// from the surface that actually owns Escape.
    pub(super) fn escape_dismissal_intent_for(
        &self,
        overlay: DesignOpenOverlay,
    ) -> Option<InspectorOverlayDismissIntent<DesignOpenOverlay>> {
        (self.highest_priority_open() == Some(overlay)).then(|| {
            InspectorOverlayDismissIntent::new_versioned(
                overlay,
                InspectorOverlayDismissCause::Escape,
                self.focus_returns.get(&overlay).cloned(),
                self.generations.get(&overlay).copied().unwrap_or_default(),
            )
        })
    }

    /// Produces an outside-click intent only for the topmost open surface.
    /// A click observed by a covered ancestor must not dismiss two overlays.
    pub(super) fn outside_click_dismissal_intent(
        &self,
        overlay: DesignOpenOverlay,
    ) -> Option<InspectorOverlayDismissIntent<DesignOpenOverlay>> {
        (self.topmost() == Some(overlay)).then(|| {
            InspectorOverlayDismissIntent::new_versioned(
                overlay,
                InspectorOverlayDismissCause::OutsideClick,
                self.focus_returns.get(&overlay).cloned(),
                self.generations.get(&overlay).copied().unwrap_or_default(),
            )
        })
    }

    pub(super) fn dismissal_is_current(
        &self,
        intent: &InspectorOverlayDismissIntent<DesignOpenOverlay>,
    ) -> bool {
        let overlay = *intent.overlay();
        let current_generation = self.generations.get(&overlay).copied().unwrap_or_default();
        self.is_open(overlay) && intent.generation() == Some(current_generation)
    }

    /// Completes focus bookkeeping after the facade has balanced child edits
    /// and cleared the overlay slot. Context changes clear this metadata, so a
    /// stale dismissal intent cannot restore focus into a stale projection.
    pub(super) fn finish_dismissal(
        &mut self,
        intent: &InspectorOverlayDismissIntent<DesignOpenOverlay>,
    ) -> Option<InspectorOverlayFocusTarget> {
        if !self.dismissal_is_current(intent) {
            return None;
        }
        self.clear_slot(*intent.overlay());
        self.generations.remove(intent.overlay());
        self.focus_returns.remove(intent.overlay())
    }

    /// Clears every overlay whose target is derived from the inspected page,
    /// selection, text range, or node capabilities.
    ///
    /// Transaction cancellation must happen before this method is called so
    /// the facade can emit terminal intents while the exact targets are still
    /// available. Unlike [`Self::clear_editor_only`], this also dismisses the
    /// permission-neutral legacy page-resource browser because its page target
    /// is stale after an inspection-context change.
    pub(super) fn clear_for_context_change(&mut self) {
        self.clear_editor_only();
        self.discard_many(&[
            DesignOpenOverlay::SelectionHeader,
            DesignOpenOverlay::ExportChoice,
            DesignOpenOverlay::PageResource,
        ]);
        self.focus_returns.clear();
        self.generations.clear();
    }

    pub(super) fn clear_editor_only(&mut self) {
        for overlay in DESIGN_OVERLAY_PRIORITY {
            if !matches!(
                overlay,
                DesignOpenOverlay::SelectionHeader
                    | DesignOpenOverlay::ExportChoice
                    | DesignOpenOverlay::PageResource
            ) {
                self.discard(overlay);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::organisms::design::{
        DesignComponentPropertyVariableField, DesignLayoutGridVariableField,
        DesignVariableResolvedType,
    };

    fn open(overlays: &mut DesignOverlayCoordinator, overlay: DesignOpenOverlay) {
        match overlay {
            DesignOpenOverlay::ComponentPropertyEdit => {
                overlays.component_authoring_dialog_open = true;
            }
            DesignOpenOverlay::ComponentPropertyContextMenu => {
                overlays.component_property_context_menu = Some("property".into());
            }
            DesignOpenOverlay::ComponentPropertyCreateMenu => {
                overlays.component_property_create_menu_open = true;
            }
            DesignOpenOverlay::AuxiliaryColorPicker => {
                overlays.auxiliary_color_picker =
                    Some(AuxiliaryColorPickerTarget::PageBackground {
                        page_id: "page".into(),
                    });
            }
            DesignOpenOverlay::PageBackground => overlays.page_background_picker_open = true,
            DesignOpenOverlay::PageResource => {
                overlays.page_resource_browser = Some(DesignLocalResourceCategory::Styles);
            }
            DesignOpenOverlay::VariableMode => overlays.variable_mode_browser_open = true,
            DesignOpenOverlay::FramePreset => overlays.frame_preset_browser_open = true,
            DesignOpenOverlay::ComponentPropertyVariable => {
                overlays.component_property_variable_picker =
                    Some(DesignComponentPropertyVariableTarget {
                        property_id: "property".into(),
                        property_name: "Property".into(),
                        property_kind: DesignComponentPropertyKind::Boolean,
                        field: DesignComponentPropertyVariableField::InstanceValue,
                        resolved_type: DesignVariableResolvedType::Boolean,
                    });
            }
            DesignOpenOverlay::PropertyVariable => {
                overlays.property_variable_picker = Some(DesignPanelProperty::X);
            }
            DesignOpenOverlay::ComponentSwap => {
                overlays.component_swap_browser = Some("property".into());
            }
            DesignOpenOverlay::TypographyStyle => overlays.typography_style_picker_open = true,
            DesignOpenOverlay::FontBrowser => overlays.font_browser_open = true,
            DesignOpenOverlay::SelectionHeader => {
                overlays.selection_header_overlay = Some(SelectionHeaderOverlay::More);
            }
            DesignOpenOverlay::PaintStyle => {
                overlays.paint_style_browser_open = Some(DesignPanelCollection::Fill);
            }
            DesignOpenOverlay::EffectStyle => overlays.effect_style_browser_open = true,
            DesignOpenOverlay::LayoutGridStyle => overlays.layout_grid_style_browser_open = true,
            DesignOpenOverlay::PreviewOptionMenu => {
                overlays.preview_option_menu_open = Some(DesignPanelProperty::Opacity);
            }
            DesignOpenOverlay::AppearanceBlendMode => {
                overlays.appearance_blend_mode_open = true;
            }
            DesignOpenOverlay::DimensionMenu => {
                overlays.dimension_menu_open = Some(DesignLayoutDimensionAxis::Width);
            }
            DesignOpenOverlay::ExportChoice => {
                overlays.export_choice_overlay = Some("export".into());
            }
            DesignOpenOverlay::EffectSettings => {
                overlays.active_effect_settings = Some(EffectSettingsTarget {
                    index: 0,
                    effect_id: "effect".into(),
                });
            }
            DesignOpenOverlay::SelectionColorResource => {
                overlays.selection_color_resource_browser = Some(SelectionColorResourceTarget {
                    selection_color_id: "selection-color".into(),
                    kind: SelectionColorResourceKind::PaintStyle,
                });
            }
            DesignOpenOverlay::LayoutGridCountVariable => {
                overlays.layout_grid_count_variable_target =
                    Some(DesignLayoutGridVariableTarget::new(
                        "guide",
                        0,
                        DesignPanelProperty::LayoutGridCount(0),
                        DesignLayoutGridVariableField::Count,
                    ));
            }
            DesignOpenOverlay::MenuPreview => {
                overlays.active_menu_preview = Some(DesignMenuPreview::NodeProperty {
                    target: DesignPanelTarget::Nodes {
                        node_ids: vec!["node".into()],
                    },
                    property: DesignPanelProperty::Opacity,
                    original: DesignPanelValue::Number(1.),
                    candidate: DesignPanelValue::Number(0.5),
                });
            }
            DesignOpenOverlay::PaintPicker => {
                overlays.active_picker = Some(PaintPickerTarget {
                    collection: DesignPanelCollection::Fill,
                    index: 0,
                    paint_id: "paint".into(),
                });
            }
            DesignOpenOverlay::TypeSettings => overlays.type_settings_open = true,
            DesignOpenOverlay::GridDimensions => {
                overlays.grid_dimensions_picker = Some(GridDimensionsPicker {
                    node_id: "node".into(),
                    candidate: DesignGridDimensions {
                        columns: 2,
                        rows: 2,
                    },
                });
            }
        }
    }

    #[test]
    fn every_overlay_slot_has_one_deterministic_priority() {
        for (index, expected) in DESIGN_OVERLAY_PRIORITY.into_iter().enumerate() {
            let mut alone = DesignOverlayCoordinator::default();
            open(&mut alone, expected);
            assert_eq!(alone.topmost(), Some(expected));
            let intent = alone
                .escape_dismissal_intent()
                .expect("an open overlay must produce an Escape intent");
            assert_eq!(intent.overlay(), &expected);
            assert_eq!(intent.cause(), InspectorOverlayDismissCause::Escape);

            for lower_priority in DESIGN_OVERLAY_PRIORITY.iter().copied().skip(index + 1) {
                let mut pair = DesignOverlayCoordinator::default();
                open(&mut pair, expected);
                open(&mut pair, lower_priority);
                assert_eq!(
                    pair.topmost(),
                    Some(expected),
                    "{expected:?} must dismiss before {lower_priority:?}",
                );
            }
        }
    }

    #[test]
    fn outside_click_only_targets_the_topmost_open_overlay() {
        let mut overlays = DesignOverlayCoordinator::default();
        open(&mut overlays, DesignOpenOverlay::PropertyVariable);
        open(
            &mut overlays,
            DesignOpenOverlay::ComponentPropertyCreateMenu,
        );

        assert!(
            overlays
                .outside_click_dismissal_intent(DesignOpenOverlay::PropertyVariable)
                .is_none(),
            "a covered ancestor must not dismiss alongside the topmost overlay"
        );
        let intent = overlays
            .outside_click_dismissal_intent(DesignOpenOverlay::ComponentPropertyCreateMenu)
            .expect("the topmost surface owns the outside click");
        assert_eq!(
            intent.overlay(),
            &DesignOpenOverlay::ComponentPropertyCreateMenu
        );
        assert_eq!(intent.cause(), InspectorOverlayDismissCause::OutsideClick);
    }

    #[test]
    fn exact_escape_only_targets_the_topmost_open_overlay() {
        let mut overlays = DesignOverlayCoordinator::default();
        open(&mut overlays, DesignOpenOverlay::AppearanceBlendMode);
        open(&mut overlays, DesignOpenOverlay::PreviewOptionMenu);

        assert!(
            overlays
                .escape_dismissal_intent_for(DesignOpenOverlay::AppearanceBlendMode)
                .is_none(),
            "a covered item-level handler must leave Escape to the topmost surface"
        );
        let intent = overlays
            .escape_dismissal_intent_for(DesignOpenOverlay::PreviewOptionMenu)
            .expect("the exact topmost surface owns Escape");
        assert_eq!(intent.overlay(), &DesignOpenOverlay::PreviewOptionMenu);
        assert_eq!(intent.cause(), InspectorOverlayDismissCause::Escape);
    }

    #[test]
    fn grid_dimensions_open_is_one_exact_mutual_exclusion_transition() {
        let mut overlays = DesignOverlayCoordinator::default();
        for conflict in GRID_DIMENSIONS_CONFLICTS {
            open(&mut overlays, *conflict);
        }
        open(&mut overlays, DesignOpenOverlay::PageResource);

        overlays.open_grid_dimensions(GridDimensionsPicker {
            node_id: "grid".into(),
            candidate: DesignGridDimensions {
                columns: 3,
                rows: 2,
            },
        });

        assert!(overlays.grid_dimensions_picker().is_some());
        for conflict in GRID_DIMENSIONS_CONFLICTS {
            assert!(
                !overlays.is_open(*conflict),
                "Grid dimensions must atomically replace {conflict:?}"
            );
        }
        assert!(
            overlays.is_open(DesignOpenOverlay::PageResource),
            "the transition must preserve permission-neutral page resources"
        );
    }

    #[test]
    fn frame_preset_open_and_close_use_the_coordinator_policy() {
        let mut overlays = DesignOverlayCoordinator::default();
        for conflict in FRAME_PRESET_CONFLICTS {
            open(&mut overlays, *conflict);
        }
        open(&mut overlays, DesignOpenOverlay::PageResource);

        overlays.set_frame_preset_open(true);

        assert!(overlays.frame_preset_is_open());
        for conflict in FRAME_PRESET_CONFLICTS {
            assert!(
                !overlays.is_open(*conflict),
                "Frame presets must atomically replace {conflict:?}"
            );
        }
        assert!(overlays.is_open(DesignOpenOverlay::PageResource));

        overlays.set_frame_preset_open(false);
        assert!(!overlays.frame_preset_is_open());
    }

    #[test]
    fn completing_dismissal_clears_the_coordinator_owned_slot() {
        let mut overlays = DesignOverlayCoordinator::default();
        overlays.open_grid_dimensions(GridDimensionsPicker {
            node_id: "grid".into(),
            candidate: DesignGridDimensions {
                columns: 2,
                rows: 2,
            },
        });
        let intent = overlays
            .escape_dismissal_intent_for(DesignOpenOverlay::GridDimensions)
            .expect("Grid dimensions is topmost");

        assert!(overlays.finish_dismissal(&intent).is_none());
        assert!(overlays.grid_dimensions_picker().is_none());
        assert_eq!(overlays.topmost(), None);
    }

    #[test]
    fn editor_overlay_reset_keeps_compatibility_page_resource_state() {
        let mut overlays = DesignOverlayCoordinator {
            type_settings_open: true,
            page_resource_browser: Some(DesignLocalResourceCategory::Styles),
            ..Default::default()
        };
        overlays.clear_editor_only();
        assert_eq!(
            overlays.topmost(),
            Some(DesignOpenOverlay::PageResource),
            "the deprecated page resource surface is permission-neutral"
        );
    }

    #[test]
    fn context_change_reset_dismisses_every_overlay_slot() {
        let mut overlays = DesignOverlayCoordinator::default();
        for overlay in DESIGN_OVERLAY_PRIORITY {
            open(&mut overlays, overlay);
        }

        overlays.clear_for_context_change();

        assert_eq!(overlays.topmost(), None);
    }

    #[test]
    fn stale_dismissal_cannot_close_a_reopened_overlay_generation() {
        let mut overlays = DesignOverlayCoordinator::default();
        overlays.open(DesignOverlayState::TypeSettings);
        let stale = overlays
            .escape_dismissal_intent_for(DesignOpenOverlay::TypeSettings)
            .expect("the first generation is open");

        overlays.discard(DesignOpenOverlay::TypeSettings);
        overlays.open(DesignOverlayState::TypeSettings);

        assert!(!overlays.dismissal_is_current(&stale));
        assert!(overlays.finish_dismissal(&stale).is_none());
        assert!(overlays.type_settings_open());
    }
}
