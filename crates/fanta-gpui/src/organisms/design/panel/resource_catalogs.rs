use super::{
    DesignColorContrastViewData, DesignColorStyleSampleViewData, DesignColorStyleViewData,
    DesignComponentSwapViewData, DesignEffectStyleViewData, DesignEffectVariableViewData,
    DesignFontViewData, DesignLayoutGridCountVariableViewData, DesignLayoutGridStyleViewData,
    DesignLayoutGridVariableViewData, DesignMediaPaintViewData, DesignPaintStyleViewData,
    DesignPaintVariableViewData, DesignShaderViewData, DesignTypographyStyleViewData,
    DesignVariableViewData,
};

/// Host-owned resource catalogs used by inspector browsers and pickers.
///
/// Catalog data is deliberately independent from the current selection so a
/// host can refresh resources without rebuilding interaction state.
#[derive(Default)]
pub(super) struct DesignPanelResourceCatalogs {
    pub(super) color_styles: DesignColorStyleViewData,
    pub(super) color_style_samples: DesignColorStyleSampleViewData,
    pub(super) color_contrast: DesignColorContrastViewData,
    pub(super) paint_variables: DesignPaintVariableViewData,
    pub(super) paint_styles: DesignPaintStyleViewData,
    pub(super) media_paints: DesignMediaPaintViewData,
    pub(super) shaders: DesignShaderViewData,
    pub(super) typography_styles: DesignTypographyStyleViewData,
    pub(super) fonts: DesignFontViewData,
    pub(super) effect_styles: DesignEffectStyleViewData,
    pub(super) effect_variables: DesignEffectVariableViewData,
    pub(super) property_variables: DesignVariableViewData,
    pub(super) component_swaps: DesignComponentSwapViewData,
    pub(super) layout_grid_styles: DesignLayoutGridStyleViewData,
    pub(super) layout_grid_variables: DesignLayoutGridVariableViewData,
    /// Compatibility catalog for the former count-only guide-variable API.
    pub(super) layout_grid_count_variables: DesignLayoutGridCountVariableViewData,
}
