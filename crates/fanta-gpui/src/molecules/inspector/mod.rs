//! Domain-neutral building blocks for dense property inspectors.
//!
//! These components follow behavioral taxonomy rather than a particular
//! feature model: fields edit controlled values, sections disclose content,
//! action groups invoke commands, collections expose row interactions, and
//! overlays remain transient and anchored. They intentionally know nothing
//! about any host document or organism action enum.

mod contracts;
mod controls;
mod feedback;
mod fields;
mod overlay;
mod structure;

#[cfg(test)]
mod interaction_tests;

pub use contracts::{
    InspectorControlledEdit, InspectorDensity, InspectorEdit, InspectorEditPhase,
    InspectorFieldAccess, InspectorFieldPresentation, InspectorGridLayout, InspectorLabelPlacement,
    InspectorMetrics, InspectorValue,
};
pub use controls::{
    inspector_action_button, inspector_action_group, inspector_collection_row, inspector_segment,
    inspector_segmented_control,
};
pub use feedback::{
    InspectorEmptyState, InspectorFeedback, InspectorFeedbackKind, InspectorFieldMessage,
    InspectorFieldMessageKind,
};
pub use fields::{
    InspectorCheckboxField, InspectorColorField, InspectorColorSwatch, InspectorFieldFrame,
    InspectorNumberDraft, InspectorNumberField, InspectorPickerField, InspectorSliderField,
    InspectorTextField, InspectorToggleField, inspector_checkbox_field, inspector_color_field,
    inspector_color_swatch, inspector_number_field, inspector_picker_field, inspector_slider_field,
    inspector_text_field, inspector_toggle_field,
};
pub use overlay::{
    InspectorOverlayDismissCause, InspectorOverlayDismissIntent, InspectorOverlayFocusTarget,
    InspectorOverlayPlacement, inspector_anchored_menu, inspector_anchored_menu_surface,
    inspector_anchored_overlay, inspector_anchored_popover, inspector_menu_item,
    inspector_menu_surface, inspector_popover_surface,
};
pub use structure::{
    inspector_field_frame, inspector_field_frame_with_presentation, inspector_field_grid,
    inspector_field_grid_with_layout, inspector_field_group, inspector_field_label,
    inspector_grouped_field_frame, inspector_row, inspector_row_with_layout, inspector_section,
    inspector_section_group, inspector_section_header,
};
