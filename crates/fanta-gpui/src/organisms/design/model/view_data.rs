use std::collections::HashMap;

use super::super::{
    DesignPanelInspectionContext, DesignPanelParentLayout, DesignPanelPermissions,
    DesignPanelPropertyValueState,
};
use super::*;

/// Controlled right-sidebar navigation supplied with a Design-panel snapshot.
///
/// Editor and viewer surfaces are retained independently so changing access
/// mode does not discard the host's last accepted surface for the other mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct DesignPanelNavigationViewData {
    pub editor_surface: DesignPanelSurface,
    pub viewer_surface: DesignPanelSurface,
    pub workspace_mode: DesignPanelWorkspaceMode,
}

impl DesignPanelNavigationViewData {
    pub const fn new(
        editor_surface: DesignPanelSurface,
        viewer_surface: DesignPanelSurface,
        workspace_mode: DesignPanelWorkspaceMode,
    ) -> Self {
        Self {
            editor_surface,
            viewer_surface,
            workspace_mode,
        }
    }

    /// Repairs surfaces that belong to the wrong permission set.
    pub const fn normalized(self) -> Self {
        Self {
            editor_surface: if self.editor_surface.is_available(true) {
                self.editor_surface
            } else {
                DesignPanelSurface::Design
            },
            viewer_surface: if self.viewer_surface.is_available(false) {
                self.viewer_surface
            } else {
                DesignPanelSurface::Properties
            },
            workspace_mode: self.workspace_mode,
        }
    }

    pub const fn active_surface(self, can_edit: bool) -> DesignPanelSurface {
        if can_edit {
            self.editor_surface
        } else {
            self.viewer_surface
        }
    }
}

impl Default for DesignPanelNavigationViewData {
    fn default() -> Self {
        Self {
            editor_surface: DesignPanelSurface::Design,
            viewer_surface: DesignPanelSurface::Properties,
            workspace_mode: DesignPanelWorkspaceMode::Design,
        }
    }
}

/// Cross-file presentation preferences consumed by the Design panel.
///
/// None of these values are document state and changing them emits no
/// [`DesignPanelAction`].
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct DesignPanelPreferencesViewData {
    pub additional_labels: bool,
    pub nudge_settings: DesignNudgeSettings,
    pub variables_entry_point: DesignVariablesEntryPoint,
}

/// One selected-node header snapshot bound to its exact ordered target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignPanelTargetedSelectionHeader {
    pub target: DesignPanelTarget,
    pub view_data: DesignSelectionHeaderViewData,
}

impl DesignPanelTargetedSelectionHeader {
    pub const fn new(target: DesignPanelTarget, view_data: DesignSelectionHeaderViewData) -> Self {
        Self { target, view_data }
    }
}

/// Optional host projections whose validity depends on the current target.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct DesignPanelProjectionViewData {
    pub export: Option<DesignExportViewData>,
    pub add_auto_layout: Option<DesignAddAutoLayoutViewData>,
    pub draw_appearance: Option<DesignDrawAppearanceViewData>,
    pub frame_presets: Option<DesignFramePresetViewData>,
    pub smart_selection: Option<DesignSmartSelectionViewData>,
    pub page: Option<DesignPageViewData>,
    pub page_local_styles: Option<DesignPageLocalStylesViewData>,
    pub variable_modes: Option<DesignVariableModeViewData>,
    pub viewer_properties: Option<DesignViewerPropertiesViewData>,
    pub selection_header: Option<DesignPanelTargetedSelectionHeader>,
}

/// Host-owned catalogs and resource metadata used by inspector pickers.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct DesignPanelResourcesViewData {
    /// Compatibility-only color presets retained for existing hosts.
    pub color_styles: DesignColorStyleViewData,
    pub color_style_samples: DesignColorStyleSampleViewData,
    pub color_contrast: DesignColorContrastViewData,
    pub paint_variables: DesignPaintVariableViewData,
    pub paint_styles: DesignPaintStyleViewData,
    pub media_paints: DesignMediaPaintViewData,
    pub shaders: DesignShaderViewData,
    pub typography_styles: DesignTypographyStyleViewData,
    pub fonts: DesignFontViewData,
    pub effect_styles: DesignEffectStyleViewData,
    pub effect_variables: DesignEffectVariableViewData,
    pub property_variables: DesignVariableViewData,
    pub component_swaps: DesignComponentSwapViewData,
    pub layout_grid_styles: DesignLayoutGridStyleViewData,
    pub layout_grid_variables: DesignLayoutGridVariableViewData,
    /// Compatibility snapshot for the former count-only guide-variable API.
    pub layout_grid_count_variables: DesignLayoutGridCountVariableViewData,
}

/// Complete immutable host snapshot for [`DesignPanel`](super::super::DesignPanel).
///
/// This is the canonical bulk-update API. The existing granular setters stay
/// available and apply the same normalization and interaction-cancellation
/// rules. Presentation-only state such as focus, scroll offsets, open menus,
/// and edit drafts is intentionally absent.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct DesignPanelViewData {
    pub inspection_context: DesignPanelInspectionContext,
    /// Compatibility-only node retained for Page/no-selection contexts.
    ///
    /// A Page has no inspected scene node, but the original `DesignPanel`
    /// constructor and `node()` API expose a fallback node and older Page
    /// hosts may derive an implicit target from its opaque ID. Carrying that
    /// value in the complete snapshot makes `view_data()` / `set_view_data()`
    /// lossless without making it a second source for selected-node data.
    pub page_node_fallback: DesignPanelNode,
    pub navigation: DesignPanelNavigationViewData,
    pub preferences: DesignPanelPreferencesViewData,
    pub projections: DesignPanelProjectionViewData,
    pub resources: DesignPanelResourcesViewData,
    pub property_states:
        HashMap<DesignPanelProperty, DesignPanelPropertyValueState<DesignPanelValue>>,
}

impl DesignPanelViewData {
    pub fn new(inspection_context: DesignPanelInspectionContext) -> Self {
        let page_node_fallback = inspection_context
            .selection()
            .items()
            .first()
            .cloned()
            .unwrap_or_else(|| {
                DesignPanelNode::new("design-panel-page", "Page", DesignPanelNodeKind::Frame)
            });
        Self {
            inspection_context,
            page_node_fallback,
            navigation: DesignPanelNavigationViewData::default(),
            preferences: DesignPanelPreferencesViewData::default(),
            projections: DesignPanelProjectionViewData::default(),
            resources: DesignPanelResourcesViewData::default(),
            property_states: HashMap::new(),
        }
    }

    /// Convenience snapshot matching [`DesignPanel::new`](super::super::DesignPanel::new).
    pub fn for_node(node: DesignPanelNode) -> Self {
        Self::new(DesignPanelInspectionContext::single(
            node,
            DesignPanelParentLayout::Freeform,
            DesignPanelPermissions::editor(),
        ))
    }

    pub const fn inspection_context(&self) -> &DesignPanelInspectionContext {
        &self.inspection_context
    }

    pub fn selected_node(&self) -> Option<&DesignPanelNode> {
        self.inspection_context.selection().items().first()
    }
}
