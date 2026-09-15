use std::collections::HashSet;

use gpui::SharedString;

use super::{
    DesignComponentSwapSelection, DesignPanelNode, DesignPanelProperty, DimensionLimitsPreview,
    PaddingEditorMode, StyleBrowserSourceFilter, StyleBrowserViewMode, TypographySettingsTab,
};

/// Feature-local presentation state owned by the Design inspector.
///
/// None of these values are document data. They record disclosure, hover, and
/// other short-lived UI choices that can be discarded when the inspection
/// context changes.
pub(super) struct DesignPanelFeatureState {
    pub(super) page: DesignPageFeatureState,
    pub(super) export: DesignExportFeatureState,
    pub(super) component: DesignComponentFeatureState,
    pub(super) layout: DesignLayoutFeatureState,
    pub(super) typography: DesignTypographyFeatureState,
    pub(super) style_browser: DesignStyleBrowserFeatureState,
}

impl DesignPanelFeatureState {
    pub(super) fn new(node: &DesignPanelNode) -> Self {
        Self {
            page: DesignPageFeatureState::default(),
            export: DesignExportFeatureState::default(),
            component: DesignComponentFeatureState::default(),
            layout: DesignLayoutFeatureState::new(node),
            typography: DesignTypographyFeatureState::default(),
            style_browser: DesignStyleBrowserFeatureState::default(),
        }
    }
}

#[derive(Default)]
pub(super) struct DesignPageFeatureState {
    pub(super) collapsed_local_style_folders: HashSet<SharedString>,
}

#[derive(Default)]
pub(super) struct DesignExportFeatureState {
    pub(super) expanded_settings: HashSet<SharedString>,
    pub(super) preview_expanded: bool,
}

#[derive(Default)]
pub(super) struct DesignComponentFeatureState {
    pub(super) swap_hovered: Option<(SharedString, DesignComponentSwapSelection)>,
}

pub(super) struct DesignLayoutFeatureState {
    pub(super) collapsed_frame_preset_groups: HashSet<SharedString>,
    pub(super) padding_editor_mode: PaddingEditorMode,
    pub(super) dimension_limit_fields_disclosed: HashSet<DesignPanelProperty>,
    pub(super) dimension_limits_preview: Option<DimensionLimitsPreview>,
}

impl DesignLayoutFeatureState {
    fn new(node: &DesignPanelNode) -> Self {
        Self {
            collapsed_frame_preset_groups: HashSet::new(),
            padding_editor_mode: PaddingEditorMode::for_node(node),
            dimension_limit_fields_disclosed: HashSet::new(),
            dimension_limits_preview: None,
        }
    }
}

pub(super) struct DesignTypographyFeatureState {
    pub(super) settings_tab: TypographySettingsTab,
}

impl Default for DesignTypographyFeatureState {
    fn default() -> Self {
        Self {
            settings_tab: TypographySettingsTab::Basics,
        }
    }
}

pub(super) struct DesignStyleBrowserFeatureState {
    pub(super) source_filter: StyleBrowserSourceFilter,
    pub(super) view_mode: StyleBrowserViewMode,
}

impl Default for DesignStyleBrowserFeatureState {
    fn default() -> Self {
        Self {
            source_filter: StyleBrowserSourceFilter::All,
            view_mode: StyleBrowserViewMode::List,
        }
    }
}
