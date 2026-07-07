//! Prototype interactions: triggers, navigate actions, transitions, flow start.

use super::*;

// =============================================================================
// Task 4: Prototype
// =============================================================================

fn prototype_event(interaction: &str) -> KiwiValue {
    o(
        "PrototypeEvent",
        vec![("interactionType", KiwiValue::Enum(interaction.to_owned()))],
    )
}

fn navigate_action(target: KiwiValue, transition: Option<&str>) -> KiwiValue {
    let mut fields = vec![("transitionNodeID", target)];
    if let Some(t) = transition {
        fields.push(("transitionType", KiwiValue::Enum(t.to_owned())));
    }
    o("PrototypeAction", fields)
}

#[test]
fn prototype_interaction_maps_to_click_navigate_reaction() {
    // FRAME A has a click → navigate to FRAME B with a dissolve transition.
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("FRAME".to_owned())),
                ("name", KiwiValue::String("A".to_owned())),
                ("size", vector(100.0, 100.0)),
                (
                    "prototypeInteractions",
                    KiwiValue::Array(vec![o(
                        "PrototypeInteraction",
                        vec![
                            ("event", prototype_event("ON_CLICK")),
                            (
                                "actions",
                                KiwiValue::Array(vec![navigate_action(
                                    guid(0, 2),
                                    Some("DISSOLVE"),
                                )]),
                            ),
                        ],
                    )]),
                ),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("type", KiwiValue::Enum("FRAME".to_owned())),
                ("name", KiwiValue::String("B".to_owned())),
                ("size", vector(100.0, 100.0)),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.reactions, 1);

    let a_id = doc
        .scene
        .roots()
        .iter()
        .copied()
        .find(|id| doc.scene.get(*id).unwrap().name == "A")
        .unwrap();
    let b_id = doc
        .scene
        .roots()
        .iter()
        .copied()
        .find(|id| doc.scene.get(*id).unwrap().name == "B")
        .unwrap();
    let node = doc.scene.get(a_id).unwrap();
    assert_eq!(node.reactions.len(), 1);
    let r = &node.reactions[0];
    assert_eq!(r.trigger, Trigger::Click);
    assert_eq!(r.action, Action::Navigate { to: b_id });
    assert!(matches!(
        r.transition,
        Some(Transition {
            style: TransitionStyle::Dissolve,
            ..
        })
    ));
    doc.scene.validate().unwrap();
}

#[test]
fn document_prototype_start_node_sets_flow_start() {
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".to_owned())),
                ("prototypeStartNodeID", guid(0, 1)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("FRAME".to_owned())),
                ("name", KiwiValue::String("Start".to_owned())),
                ("size", vector(100.0, 100.0)),
            ],
        ),
    ]);
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    let start_id = doc
        .scene
        .roots()
        .iter()
        .copied()
        .find(|id| doc.scene.get(*id).unwrap().name == "Start")
        .unwrap();
    assert_eq!(doc.flow_start(), Some(start_id));
}

#[test]
fn after_delay_trigger_converts_seconds_to_ms() {
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("FRAME".to_owned())),
                ("name", KiwiValue::String("A".to_owned())),
                ("size", vector(100.0, 100.0)),
                (
                    "prototypeInteractions",
                    KiwiValue::Array(vec![o(
                        "PrototypeInteraction",
                        vec![
                            (
                                "event",
                                o(
                                    "PrototypeEvent",
                                    vec![
                                        (
                                            "interactionType",
                                            KiwiValue::Enum("AFTER_TIMEOUT".to_owned()),
                                        ),
                                        ("interactionDuration", KiwiValue::Float(0.25)),
                                    ],
                                ),
                            ),
                            (
                                "actions",
                                KiwiValue::Array(vec![navigate_action(guid(0, 2), None)]),
                            ),
                        ],
                    )]),
                ),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("type", KiwiValue::Enum("FRAME".to_owned())),
                ("name", KiwiValue::String("B".to_owned())),
                ("size", vector(100.0, 100.0)),
            ],
        ),
    ]);
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    let a_id = doc
        .scene
        .roots()
        .iter()
        .copied()
        .find(|id| doc.scene.get(*id).unwrap().name == "A")
        .unwrap();
    let r = &doc.scene.get(a_id).unwrap().reactions[0];
    assert_eq!(r.trigger, Trigger::AfterDelay { delay_ms: 250 });
}
