//! Retained editor-state projections for component-property authoring.
//!
//! This module deliberately has no `DesignPanel` dependency. The panel owns
//! GPUI inputs and host coordination; these helpers own the type-specific
//! draft initialization and finalization rules shared by the create and edit
//! surfaces.

use gpui::SharedString;

use crate::organisms::design::{
    DesignComponentPropertyDefinition, DesignComponentPropertyKind, DesignDocumentationLink,
    DesignSlotSettings, DesignSlotValue,
};

use super::super::{ComponentPropertyCreateDraft, ComponentPropertyEditDraft};

pub(super) struct ComponentPropertyCreateEditorSeed {
    pub(super) draft: ComponentPropertyCreateDraft,
    pub(super) name: SharedString,
    pub(super) default_or_description: SharedString,
}

pub(super) struct ComponentPropertyEditEditorSeed {
    pub(super) draft: ComponentPropertyEditDraft,
    pub(super) default_or_description: SharedString,
    pub(super) slot_minimum: SharedString,
    pub(super) slot_maximum: SharedString,
}

pub(super) struct PreparedComponentPropertyCreate {
    pub(super) name: SharedString,
    pub(super) draft: ComponentPropertyCreateDraft,
}

#[derive(Debug)]
pub(super) enum ComponentPropertyEditPreparation {
    Ready(ComponentPropertyEditDraft),
    Invalid(ComponentPropertyEditDraft),
}

pub(super) fn create_editor_seed(
    kind: DesignComponentPropertyKind,
) -> ComponentPropertyCreateEditorSeed {
    let (name, default_or_description, definition) = match kind {
        DesignComponentPropertyKind::Boolean => (
            "Show layer",
            "True",
            DesignComponentPropertyDefinition::Boolean {
                default_value: true,
            },
        ),
        DesignComponentPropertyKind::Text => (
            "Text",
            "Text",
            DesignComponentPropertyDefinition::Text {
                default_value: "Text".into(),
                multiline: false,
            },
        ),
        DesignComponentPropertyKind::InstanceSwap => (
            "Instance",
            "",
            DesignComponentPropertyDefinition::InstanceSwap {
                default_value: None,
                preferred_values: Vec::new(),
            },
        ),
        DesignComponentPropertyKind::Variant => (
            "Property",
            "Default",
            DesignComponentPropertyDefinition::Variant {
                default_value: "Default".into(),
                options: vec!["Default".into()],
            },
        ),
        DesignComponentPropertyKind::Slot => (
            "Slot",
            "",
            DesignComponentPropertyDefinition::Slot {
                default_value: DesignSlotValue::default(),
                settings: DesignSlotSettings::default(),
            },
        ),
    };

    ComponentPropertyCreateEditorSeed {
        draft: ComponentPropertyCreateDraft {
            kind,
            description: None,
            documentation_links: Vec::new(),
            definition,
            default_variable_id: None,
        },
        name: name.into(),
        default_or_description: default_or_description.into(),
    }
}

pub(super) fn edit_editor_seed(
    property_id: SharedString,
    description: Option<SharedString>,
    documentation_links: Vec<DesignDocumentationLink>,
    definition: DesignComponentPropertyDefinition,
) -> ComponentPropertyEditEditorSeed {
    let default_or_description = match &definition {
        DesignComponentPropertyDefinition::Text { default_value, .. } => default_value.clone(),
        DesignComponentPropertyDefinition::Slot { .. } => description.clone().unwrap_or_default(),
        DesignComponentPropertyDefinition::Boolean { .. }
        | DesignComponentPropertyDefinition::InstanceSwap { .. }
        | DesignComponentPropertyDefinition::Variant { .. } => "".into(),
    };
    let (slot_minimum, slot_maximum) = match &definition {
        DesignComponentPropertyDefinition::Slot { settings, .. } => (
            settings
                .minimum_children
                .map(|value| value.to_string())
                .unwrap_or_default()
                .into(),
            settings
                .maximum_children
                .map(|value| value.to_string())
                .unwrap_or_default()
                .into(),
        ),
        DesignComponentPropertyDefinition::Boolean { .. }
        | DesignComponentPropertyDefinition::Text { .. }
        | DesignComponentPropertyDefinition::InstanceSwap { .. }
        | DesignComponentPropertyDefinition::Variant { .. } => ("".into(), "".into()),
    };

    ComponentPropertyEditEditorSeed {
        draft: ComponentPropertyEditDraft {
            property_id,
            expected_description: description.clone(),
            description,
            expected_documentation_links: documentation_links.clone(),
            documentation_links,
            expected_definition: definition.clone(),
            definition,
        },
        default_or_description,
        slot_minimum,
        slot_maximum,
    }
}

pub(super) fn prepare_create(
    mut draft: ComponentPropertyCreateDraft,
    name: SharedString,
    default_or_description: SharedString,
    slot_minimum: SharedString,
    slot_maximum: SharedString,
) -> Option<PreparedComponentPropertyCreate> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }

    draft.definition = match draft.kind {
        DesignComponentPropertyKind::Boolean | DesignComponentPropertyKind::InstanceSwap => {
            draft.definition
        }
        DesignComponentPropertyKind::Text => DesignComponentPropertyDefinition::Text {
            default_value: default_or_description,
            multiline: false,
        },
        DesignComponentPropertyKind::Variant => {
            let default: SharedString = if default_or_description.trim().is_empty() {
                "Default".into()
            } else {
                default_or_description.trim().to_owned().into()
            };
            DesignComponentPropertyDefinition::Variant {
                default_value: default.clone(),
                options: vec![default],
            }
        }
        DesignComponentPropertyKind::Slot => {
            let (minimum_children, maximum_children) =
                parse_slot_limits(slot_minimum, slot_maximum)?;
            let DesignComponentPropertyDefinition::Slot {
                default_value,
                mut settings,
            } = draft.definition
            else {
                return None;
            };
            settings.minimum_children = minimum_children;
            settings.maximum_children = maximum_children;
            draft.description = (!default_or_description.trim().is_empty())
                .then(|| SharedString::from(default_or_description.trim().to_owned()));
            DesignComponentPropertyDefinition::Slot {
                default_value,
                settings,
            }
        }
    };

    Some(PreparedComponentPropertyCreate {
        name: name.to_owned().into(),
        draft,
    })
}

pub(super) fn prepare_edit(
    mut draft: ComponentPropertyEditDraft,
    default_or_description: SharedString,
    slot_minimum: SharedString,
    slot_maximum: SharedString,
) -> ComponentPropertyEditPreparation {
    match &mut draft.definition {
        DesignComponentPropertyDefinition::Text { default_value, .. } => {
            *default_value = default_or_description;
        }
        DesignComponentPropertyDefinition::Slot { settings, .. } => {
            let Some((minimum_children, maximum_children)) =
                parse_slot_limits(slot_minimum, slot_maximum)
            else {
                return ComponentPropertyEditPreparation::Invalid(draft);
            };
            settings.minimum_children = minimum_children;
            settings.maximum_children = maximum_children;
            draft.description = (!default_or_description.trim().is_empty())
                .then(|| SharedString::from(default_or_description.trim().to_owned()));
        }
        DesignComponentPropertyDefinition::Boolean { .. }
        | DesignComponentPropertyDefinition::InstanceSwap { .. }
        | DesignComponentPropertyDefinition::Variant { .. } => {}
    }
    ComponentPropertyEditPreparation::Ready(draft)
}

pub(super) fn edit_has_changes(draft: &ComponentPropertyEditDraft) -> bool {
    draft.description != draft.expected_description
        || draft.documentation_links != draft.expected_documentation_links
        || draft.definition != draft.expected_definition
}

pub(super) fn parse_slot_limits(
    minimum: SharedString,
    maximum: SharedString,
) -> Option<(Option<u32>, Option<u32>)> {
    let parse = |value: SharedString| {
        let value = value.trim();
        if value.is_empty() {
            Some(None)
        } else {
            value.parse::<u32>().ok().map(Some)
        }
    };
    let minimum = parse(minimum)?;
    let maximum = parse(maximum)?;
    if minimum
        .zip(maximum)
        .is_some_and(|(minimum, maximum)| minimum > maximum)
    {
        return None;
    }
    Some((minimum, maximum))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_seed_keeps_type_specific_defaults_together() {
        let text = create_editor_seed(DesignComponentPropertyKind::Text);
        assert_eq!(text.name, "Text");
        assert_eq!(text.default_or_description, "Text");
        assert_eq!(
            text.draft.definition,
            DesignComponentPropertyDefinition::Text {
                default_value: "Text".into(),
                multiline: false,
            }
        );

        let slot = create_editor_seed(DesignComponentPropertyKind::Slot);
        assert_eq!(slot.name, "Slot");
        assert_eq!(slot.default_or_description, "");
        assert!(matches!(
            slot.draft.definition,
            DesignComponentPropertyDefinition::Slot { .. }
        ));
    }

    #[test]
    fn slot_limits_reject_invalid_or_inverted_ranges() {
        assert_eq!(parse_slot_limits("".into(), "".into()), Some((None, None)));
        assert_eq!(
            parse_slot_limits("2".into(), "5".into()),
            Some((Some(2), Some(5)))
        );
        assert_eq!(parse_slot_limits("6".into(), "5".into()), None);
        assert_eq!(parse_slot_limits("two".into(), "5".into()), None);
    }

    #[test]
    fn prepare_create_normalizes_variant_and_slot_inputs() {
        let variant = create_editor_seed(DesignComponentPropertyKind::Variant);
        let prepared = prepare_create(
            variant.draft,
            "  Size  ".into(),
            "   ".into(),
            "".into(),
            "".into(),
        )
        .expect("valid Variant draft");
        assert_eq!(prepared.name, "Size");
        assert_eq!(
            prepared.draft.definition,
            DesignComponentPropertyDefinition::Variant {
                default_value: "Default".into(),
                options: vec!["Default".into()],
            }
        );

        let slot = create_editor_seed(DesignComponentPropertyKind::Slot);
        let prepared = prepare_create(
            slot.draft,
            "Content".into(),
            "  Main content  ".into(),
            "1".into(),
            "4".into(),
        )
        .expect("valid Slot draft");
        assert_eq!(prepared.draft.description, Some("Main content".into()));
        let DesignComponentPropertyDefinition::Slot { settings, .. } = prepared.draft.definition
        else {
            panic!("expected Slot definition");
        };
        assert_eq!(settings.minimum_children, Some(1));
        assert_eq!(settings.maximum_children, Some(4));
    }

    #[test]
    fn edit_seed_and_finalize_keep_invalid_slot_draft_available() {
        let definition = DesignComponentPropertyDefinition::Slot {
            default_value: DesignSlotValue::default(),
            settings: DesignSlotSettings {
                minimum_children: Some(1),
                maximum_children: Some(3),
                ..DesignSlotSettings::default()
            },
        };
        let seed = edit_editor_seed(
            "content".into(),
            Some("Original".into()),
            Vec::new(),
            definition,
        );
        assert_eq!(seed.default_or_description, "Original");
        assert_eq!(seed.slot_minimum, "1");
        assert_eq!(seed.slot_maximum, "3");

        let ComponentPropertyEditPreparation::Invalid(draft) =
            prepare_edit(seed.draft, "Changed".into(), "4".into(), "2".into())
        else {
            panic!("inverted limits must preserve the editor draft");
        };
        assert_eq!(draft.description, Some("Original".into()));

        let ComponentPropertyEditPreparation::Ready(draft) =
            prepare_edit(draft, "  Changed  ".into(), "2".into(), "4".into())
        else {
            panic!("valid limits must finalize the editor draft");
        };
        assert_eq!(draft.description, Some("Changed".into()));
        assert!(edit_has_changes(&draft));
    }
}
