use std::collections::HashMap;

use super::{
    DesignAddAutoLayoutViewData, DesignDrawAppearanceViewData, DesignExportViewData,
    DesignFramePresetViewData, DesignPageLocalStylesViewData, DesignPageViewData,
    DesignPanelInspectionContext, DesignPanelNavigationViewData, DesignPanelNode,
    DesignPanelProperty, DesignPanelPropertyValueState, DesignPanelTarget,
    DesignSelectionHeaderViewData, DesignSmartSelectionViewData, DesignVariableModeViewData,
    DesignViewerPropertiesViewData,
};

/// Host-owned state retained by the Design-panel facade.
///
/// The selected-node snapshot remains alongside the canonical inspection
/// context only for compatibility with the legacy node-oriented API. New code
/// should derive selection and command targets from `inspection_context`.
pub(super) struct DesignPanelHostState {
    /// Page/no-selection compatibility value. Selected-node data lives only
    /// in `inspection_context` and is never read from this field.
    pub(super) page_node_fallback: DesignPanelNode,
    pub(super) inspection_context: DesignPanelInspectionContext,
    pub(super) navigation: DesignPanelNavigationViewData,
    pub(super) projections: DesignPanelHostProjections,
    pub(super) property_states: HashMap<DesignPanelProperty, DesignPanelPropertyValueState>,
}

impl DesignPanelHostState {
    pub(super) fn new(
        node: DesignPanelNode,
        inspection_context: DesignPanelInspectionContext,
    ) -> Self {
        Self {
            page_node_fallback: node,
            inspection_context,
            navigation: DesignPanelNavigationViewData::default(),
            projections: DesignPanelHostProjections::default(),
            property_states: HashMap::new(),
        }
    }

    /// Returns inspected-node data from the canonical context.
    ///
    /// `node` is consulted only for the legacy Page/no-selection fallback,
    /// where the context intentionally contains no scene node.
    pub(super) fn inspected_node(&self) -> &DesignPanelNode {
        self.inspection_context
            .selection()
            .items()
            .first()
            .unwrap_or(&self.page_node_fallback)
    }
}

/// Target-sensitive host projections, separate from resource catalogs and
/// transient inspector interaction state.
#[derive(Default)]
pub(super) struct DesignPanelHostProjections {
    pub(super) export: Option<DesignExportViewData>,
    pub(super) add_auto_layout: Option<DesignAddAutoLayoutViewData>,
    pub(super) draw_appearance: Option<DesignDrawAppearanceViewData>,
    pub(super) frame_presets: Option<DesignFramePresetViewData>,
    pub(super) smart_selection: Option<DesignSmartSelectionViewData>,
    pub(super) page: Option<DesignPageViewData>,
    pub(super) page_local_styles: Option<DesignPageLocalStylesViewData>,
    pub(super) variable_modes: Option<DesignVariableModeViewData>,
    pub(super) viewer_properties: Option<DesignViewerPropertiesViewData>,
    pub(super) selection_header: Option<DesignSelectionHeaderViewData>,
    pub(super) selection_header_target: Option<DesignPanelTarget>,
}
