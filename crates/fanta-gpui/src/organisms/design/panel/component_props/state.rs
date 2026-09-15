//! Feature-local retained interaction state for component-property authoring.

use gpui::{Entity, SharedString};
use gpui_component::input::InputState;

use super::super::{ComponentPropertyCreateDraft, ComponentPropertyEditDraft};

#[cfg(test)]
use super::super::DesignComponentPropertyKind;

/// Presentation-only state retained while the user authors component
/// properties. Host-controlled component definitions remain in
/// `DesignPanelHostState`.
pub(in crate::organisms::design::panel) struct ComponentAuthoringState {
    pub(in crate::organisms::design::panel) open_slot_limits: Option<SharedString>,
    pub(in crate::organisms::design::panel) create_draft: Option<ComponentPropertyCreateDraft>,
    pub(in crate::organisms::design::panel) edit_draft: Option<ComponentPropertyEditDraft>,
    pub(in crate::organisms::design::panel) dialog_close_pending: bool,
    #[cfg(test)]
    pub(in crate::organisms::design::panel) dialog_last_rendered_kind:
        Option<DesignComponentPropertyKind>,
    pub(in crate::organisms::design::panel) selected_property: Option<SharedString>,
    pub(in crate::organisms::design::panel) name_input: Entity<InputState>,
    pub(in crate::organisms::design::panel) default_input: Entity<InputState>,
    pub(in crate::organisms::design::panel) slot_minimum_input: Entity<InputState>,
    pub(in crate::organisms::design::panel) slot_maximum_input: Entity<InputState>,
}

impl ComponentAuthoringState {
    pub(in crate::organisms::design::panel) fn new(
        name_input: Entity<InputState>,
        default_input: Entity<InputState>,
        slot_minimum_input: Entity<InputState>,
        slot_maximum_input: Entity<InputState>,
    ) -> Self {
        Self {
            open_slot_limits: None,
            create_draft: None,
            edit_draft: None,
            dialog_close_pending: false,
            #[cfg(test)]
            dialog_last_rendered_kind: None,
            selected_property: None,
            name_input,
            default_input,
            slot_minimum_input,
            slot_maximum_input,
        }
    }
}
