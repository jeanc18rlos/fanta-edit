//! Prototype interactions: triggers, navigate actions, transitions, flow start.

use super::*;

// =============================================================================
// Task 4: Prototype
// =============================================================================

fn prototype_event(interaction: &str) -> KiwiValue {
    o(
        "PrototypeEvent",
        vec![("interactionType", KiwiValue::Enum(interaction.into()))],
    )
}

fn navigate_action(target: KiwiValue, transition: Option<&str>) -> KiwiValue {
    let mut fields = vec![("transitionNodeID", target)];
    if let Some(t) = transition {
        fields.push(("transitionType", KiwiValue::Enum(t.into())));
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
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("A".to_owned())),
                ("size", vector(100.0, 100.0)),
                (
                    "prototypeInteractions",
                    KiwiValue::array(vec![o(
                        "PrototypeInteraction",
                        vec![
                            ("event", prototype_event("ON_CLICK")),
                            (
                                "actions",
                                KiwiValue::array(vec![navigate_action(
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
                ("type", KiwiValue::Enum("FRAME".into())),
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
                ("type", KiwiValue::Enum("DOCUMENT".into())),
                ("prototypeStartNodeID", guid(0, 1)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("FRAME".into())),
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
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("A".to_owned())),
                ("size", vector(100.0, 100.0)),
                (
                    "prototypeInteractions",
                    KiwiValue::array(vec![o(
                        "PrototypeInteraction",
                        vec![
                            (
                                "event",
                                o(
                                    "PrototypeEvent",
                                    vec![
                                        (
                                            "interactionType",
                                            KiwiValue::Enum("AFTER_TIMEOUT".into()),
                                        ),
                                        ("interactionDuration", KiwiValue::Float(0.25)),
                                    ],
                                ),
                            ),
                            (
                                "actions",
                                KiwiValue::array(vec![navigate_action(guid(0, 2), None)]),
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
                ("type", KiwiValue::Enum("FRAME".into())),
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

#[test]
fn smart_animate_transition_is_imported() {
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("A".to_owned())),
                ("size", vector(100.0, 100.0)),
                (
                    "prototypeInteractions",
                    KiwiValue::array(vec![o(
                        "PrototypeInteraction",
                        vec![
                            ("event", prototype_event("ON_CLICK")),
                            (
                                "actions",
                                KiwiValue::array(vec![navigate_action(
                                    guid(0, 2),
                                    Some("SMART_ANIMATE"),
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
                ("type", KiwiValue::Enum("FRAME".into())),
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
    let r = &doc.scene.get(a_id).unwrap().reactions[0];
    assert!(matches!(
        r.transition,
        Some(Transition {
            style: TransitionStyle::SmartAnimate,
            ..
        })
    ));
}

#[test]
fn update_variant_action_uses_real_component_member_id() {
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("Button".to_owned())),
                ("isStateGroup", KiwiValue::Bool(true)),
                ("size", vector(120.0, 40.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("State=Default".to_owned())),
                ("size", vector(120.0, 40.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("State=Hover".to_owned())),
                ("size", vector(120.0, 40.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 10)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("Prototype".to_owned())),
                ("size", vector(100.0, 100.0)),
                (
                    "prototypeInteractions",
                    KiwiValue::array(vec![o(
                        "PrototypeInteraction",
                        vec![
                            ("event", prototype_event("ON_CLICK")),
                            (
                                "actions",
                                KiwiValue::array(vec![o(
                                    "PrototypeAction",
                                    vec![
                                        ("variantNodeID", guid(0, 3)),
                                        ("variantValue", KiwiValue::String("Hover".to_owned())),
                                    ],
                                )]),
                            ),
                        ],
                    )]),
                ),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.component_sets, 1);
    assert_eq!(report.reactions, 1);

    let hover_member = doc
        .components
        .defs
        .iter()
        .find_map(|(id, def)| (def.name == "State=Hover").then_some(*id))
        .expect("hover member def");
    let frame = doc
        .scene
        .roots()
        .iter()
        .flat_map(|root| doc.scene.descendants_of(*root))
        .find(|id| {
            doc.scene
                .get(*id)
                .is_some_and(|node| node.name == "Prototype")
        })
        .expect("prototype frame");
    let reaction = &doc.scene.get(frame).unwrap().reactions[0];
    assert_eq!(
        reaction.action,
        Action::UpdateVariant {
            component: hover_member,
            variant: "Hover".to_owned(),
        },
        "UpdateVariant should target the imported component member id, not a hashed fallback"
    );
}

#[test]
fn key_trigger_imports_key_codes() {
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("A".to_owned())),
                ("size", vector(100.0, 100.0)),
                (
                    "prototypeInteractions",
                    KiwiValue::array(vec![o(
                        "PrototypeInteraction",
                        vec![
                            (
                                "event",
                                o(
                                    "PrototypeEvent",
                                    vec![
                                        ("interactionType", KiwiValue::Enum("ON_KEY_DOWN".into())),
                                        (
                                            "keyCodes",
                                            KiwiValue::array(vec![
                                                KiwiValue::String("Space".to_owned()),
                                                KiwiValue::String("Enter".to_owned()),
                                            ]),
                                        ),
                                    ],
                                ),
                            ),
                            (
                                "actions",
                                KiwiValue::array(vec![navigate_action(guid(0, 2), None)]),
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
                ("type", KiwiValue::Enum("FRAME".into())),
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
    let r = &doc.scene.get(a_id).unwrap().reactions[0];
    assert_eq!(
        r.trigger,
        Trigger::Key {
            keys: vec!["Space".to_string(), "Enter".to_string()]
        }
    );
}

#[test]
fn other_actions_imported() {
    // Test Back, Close, OpenOverlay, ScrollTo.
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("A".to_owned())),
                ("size", vector(100.0, 100.0)),
                (
                    "prototypeInteractions",
                    KiwiValue::array(vec![
                        o(
                            "PrototypeInteraction",
                            vec![
                                ("event", prototype_event("ON_CLICK")),
                                (
                                    "actions",
                                    KiwiValue::array(vec![o(
                                        "PrototypeAction",
                                        vec![("type", KiwiValue::Enum("BACK".into()))],
                                    )]),
                                ),
                            ],
                        ),
                        o(
                            "PrototypeInteraction",
                            vec![
                                ("event", prototype_event("ON_CLICK")),
                                (
                                    "actions",
                                    KiwiValue::array(vec![o(
                                        "PrototypeAction",
                                        vec![("type", KiwiValue::Enum("CLOSE".into()))],
                                    )]),
                                ),
                            ],
                        ),
                        o(
                            "PrototypeInteraction",
                            vec![
                                ("event", prototype_event("ON_CLICK")),
                                (
                                    "actions",
                                    KiwiValue::array(vec![o(
                                        "PrototypeAction",
                                        vec![
                                            ("overlayNodeID", guid(0, 3)),
                                            ("overlayPosition", KiwiValue::Enum("CENTER".into())),
                                        ],
                                    )]),
                                ),
                            ],
                        ),
                        o(
                            "PrototypeInteraction",
                            vec![
                                ("event", prototype_event("ON_CLICK")),
                                (
                                    "actions",
                                    KiwiValue::array(vec![o(
                                        "PrototypeAction",
                                        vec![("targetNodeID", guid(0, 4))],
                                    )]),
                                ),
                            ],
                        ),
                    ]),
                ),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("B".to_owned())),
                ("size", vector(100.0, 100.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("Overlay".to_owned())),
                ("size", vector(100.0, 100.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 4)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("ScrollTarget".to_owned())),
                ("size", vector(100.0, 100.0)),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.reactions, 4);

    let a_id = doc
        .scene
        .roots()
        .iter()
        .copied()
        .find(|id| doc.scene.get(*id).unwrap().name == "A")
        .unwrap();
    let rs = &doc.scene.get(a_id).unwrap().reactions;
    assert_eq!(rs[0].action, Action::Back);
    assert_eq!(rs[1].action, Action::Close);
    match &rs[2].action {
        Action::OpenOverlay { .. } => {}
        _ => panic!("expected OpenOverlay"),
    }
    match &rs[3].action {
        Action::ScrollTo { .. } => {}
        _ => panic!("expected ScrollTo"),
    }
}

#[test]
fn slide_transition_and_empty_key_import() {
    // Slide + a key trigger with no keyCodes (should still produce Key with empty vec).
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("Src".to_owned())),
                ("size", vector(100.0, 100.0)),
                (
                    "prototypeInteractions",
                    KiwiValue::array(vec![
                        o(
                            "PrototypeInteraction",
                            vec![
                                ("event", prototype_event("ON_CLICK")),
                                (
                                    "actions",
                                    KiwiValue::array(vec![navigate_action(
                                        guid(0, 2),
                                        Some("SLIDE_FROM_LEFT"),
                                    )]),
                                ),
                            ],
                        ),
                        o(
                            "PrototypeInteraction",
                            vec![
                                (
                                    "event",
                                    o(
                                        "PrototypeEvent",
                                        vec![(
                                            "interactionType",
                                            KiwiValue::Enum("ON_KEY_DOWN".into()),
                                        )],
                                    ),
                                ),
                                (
                                    "actions",
                                    KiwiValue::array(vec![navigate_action(guid(0, 3), None)]),
                                ),
                            ],
                        ),
                    ]),
                ),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("Dst".to_owned())),
                ("size", vector(100.0, 100.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("Other".to_owned())),
                ("size", vector(100.0, 100.0)),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.reactions, 2);

    let src_id = doc
        .scene
        .roots()
        .iter()
        .copied()
        .find(|id| doc.scene.get(*id).unwrap().name == "Src")
        .unwrap();
    let reactions = &doc.scene.get(src_id).unwrap().reactions;

    // First reaction: click -> slide
    assert!(matches!(
        reactions[0].transition,
        Some(Transition {
            style: TransitionStyle::SlideIn {
                direction: fanta_doc::Direction::Left,
            },
            ..
        })
    ));

    // Second: key down with no codes
    assert_eq!(reactions[1].trigger, Trigger::Key { keys: vec![] });
}

// =============================================================================
// PR-5..PR-8: reaction completeness (multi-action, out-transitions, springs,
// trigger map)
// =============================================================================

/// Wrap a list of `PrototypeAction`s in one ON_CLICK interaction on frame A,
/// alongside destination frames B (guid 0:2) and C (guid 0:3).
fn frame_a_with_actions(actions: Vec<KiwiValue>) -> FigDocument {
    frame_a_with_interactions(vec![o(
        "PrototypeInteraction",
        vec![
            ("event", prototype_event("ON_CLICK")),
            ("actions", KiwiValue::array(actions)),
        ],
    )])
}

fn frame_a_with_interactions(interactions: Vec<KiwiValue>) -> FigDocument {
    doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("A".to_owned())),
                ("size", vector(100.0, 100.0)),
                ("prototypeInteractions", KiwiValue::array(interactions)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("B".to_owned())),
                ("size", vector(100.0, 100.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("C".to_owned())),
                ("size", vector(100.0, 100.0)),
            ],
        ),
    ])
}

fn reactions_of<'d>(doc: &'d Doc, name: &str) -> &'d [Reaction] {
    let id = doc
        .scene
        .roots()
        .iter()
        .copied()
        .find(|id| doc.scene.get(*id).unwrap().name == name)
        .unwrap();
    &doc.scene.get(id).unwrap().reactions
}

#[test]
fn pr5_multi_action_interaction_imports_every_action() {
    // A Figma "set variable then navigate" pair: previously only the first
    // action survived (the navigate was dropped entirely).
    let fig = frame_a_with_actions(vec![
        o(
            "PrototypeAction",
            vec![
                ("targetVariableID", guid(0, 77)),
                ("targetVariableData", var_value_bool(true)),
            ],
        ),
        navigate_action(guid(0, 2), Some("DISSOLVE")),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.reactions, 1);
    assert_eq!(report.reactions_multi_action, 1);

    let reactions = reactions_of(&doc, "A");
    assert_eq!(reactions.len(), 1);
    let reaction = &reactions[0];
    assert!(
        matches!(reaction.action, Action::SetVariable { .. }),
        "first action stays primary: {:?}",
        reaction.action
    );
    assert_eq!(reaction.extra_actions.len(), 1);
    assert!(matches!(reaction.extra_actions[0], Action::Navigate { .. }));
    assert_eq!(reaction.actions().count(), 2);
    // The pair's transition is authored on the NAVIGATE (SetVariable carries
    // none); the reaction adopts it rather than losing the dissolve.
    assert!(matches!(
        reaction.transition,
        Some(Transition {
            style: TransitionStyle::Dissolve,
            ..
        })
    ));
}

#[test]
fn pr6_out_transitions_and_scroll_animate_map() {
    let fig = frame_a_with_interactions(vec![
        o(
            "PrototypeInteraction",
            vec![
                ("event", prototype_event("ON_CLICK")),
                (
                    "actions",
                    // Figma's UI spelling ("slide out TO left").
                    KiwiValue::array(vec![navigate_action(guid(0, 2), Some("SLIDE_OUT_TO_LEFT"))]),
                ),
            ],
        ),
        o(
            "PrototypeInteraction",
            vec![
                ("event", prototype_event("ON_CLICK")),
                (
                    "actions",
                    // The shorter spelling must map identically (ASSUMPTION in
                    // reactions.rs — internal member name unpinned).
                    KiwiValue::array(vec![navigate_action(guid(0, 2), Some("MOVE_OUT_RIGHT"))]),
                ),
            ],
        ),
        o(
            "PrototypeInteraction",
            vec![
                ("event", prototype_event("ON_CLICK")),
                (
                    "actions",
                    KiwiValue::array(vec![o(
                        "PrototypeAction",
                        vec![
                            ("targetNodeID", guid(0, 3)),
                            ("transitionType", KiwiValue::Enum("SCROLL_ANIMATE".into())),
                            ("transitionDuration", KiwiValue::Float(0.4)),
                        ],
                    )]),
                ),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.reactions, 3);

    let reactions = reactions_of(&doc, "A");
    assert!(matches!(
        reactions[0].transition,
        Some(Transition {
            style: TransitionStyle::SlideOut {
                direction: fanta_doc::Direction::Left,
            },
            ..
        })
    ));
    assert!(matches!(
        reactions[1].transition,
        Some(Transition {
            style: TransitionStyle::MoveOut {
                direction: fanta_doc::Direction::Right,
            },
            ..
        })
    ));
    assert!(matches!(reactions[2].action, Action::ScrollTo { .. }));
    assert!(matches!(
        reactions[2].transition,
        Some(Transition {
            style: TransitionStyle::ScrollAnimate,
            duration_ms: 400,
            ..
        })
    ));
}

#[test]
fn pr7_spring_preset_and_custom_params_map() {
    let fig = frame_a_with_interactions(vec![
        o(
            "PrototypeInteraction",
            vec![
                ("event", prototype_event("ON_CLICK")),
                (
                    "actions",
                    KiwiValue::array(vec![o(
                        "PrototypeAction",
                        vec![
                            ("transitionNodeID", guid(0, 2)),
                            ("transitionType", KiwiValue::Enum("DISSOLVE".into())),
                            ("easingType", KiwiValue::Enum("BOUNCY".into())),
                        ],
                    )]),
                ),
            ],
        ),
        o(
            "PrototypeInteraction",
            vec![
                ("event", prototype_event("ON_CLICK")),
                (
                    "actions",
                    KiwiValue::array(vec![o(
                        "PrototypeAction",
                        vec![
                            ("transitionNodeID", guid(0, 2)),
                            ("transitionType", KiwiValue::Enum("DISSOLVE".into())),
                            ("easingType", KiwiValue::Enum("GENTLE_SPRING".into())),
                            (
                                "spring",
                                o(
                                    "SpringParams",
                                    vec![
                                        ("mass", KiwiValue::Float(2.0)),
                                        ("stiffness", KiwiValue::Float(150.0)),
                                        ("damping", KiwiValue::Float(12.0)),
                                    ],
                                ),
                            ),
                        ],
                    )]),
                ),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.reactions, 2);

    let reactions = reactions_of(&doc, "A");
    // Preset triple (standard approximation pending fixture pinning).
    assert_eq!(
        reactions[0].transition.as_ref().unwrap().easing,
        Easing::Spring {
            mass: 1.0,
            stiffness: 600.0,
            damping: 15.0,
        }
    );
    // Explicit params override the preset.
    assert_eq!(
        reactions[1].transition.as_ref().unwrap().easing,
        Easing::Spring {
            mass: 2.0,
            stiffness: 150.0,
            damping: 12.0,
        }
    );
}

#[test]
fn pr8_hover_family_triggers_map_distinctly() {
    let interaction = |trigger: &str| {
        o(
            "PrototypeInteraction",
            vec![
                ("event", prototype_event(trigger)),
                (
                    "actions",
                    KiwiValue::array(vec![navigate_action(guid(0, 2), None)]),
                ),
            ],
        )
    };
    let fig = frame_a_with_interactions(vec![
        interaction("ON_HOVER"),
        interaction("MOUSE_IN"),
        interaction("MOUSE_LEAVE"),
        interaction("MOUSE_OUT"),
        interaction("WHILE_HOVERING"),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.reactions, 5);
    assert_eq!(report.reactions_dropped_unknown_trigger, 0);

    let reactions = reactions_of(&doc, "A");
    assert_eq!(reactions[0].trigger, Trigger::Hover);
    assert_eq!(reactions[1].trigger, Trigger::MouseEnter);
    // The PR-8 fix: leave triggers no longer conflate to Hover (which fires
    // on ENTRY — a leave-triggered close would fire the moment the pointer
    // arrived).
    assert_eq!(reactions[2].trigger, Trigger::MouseLeave);
    assert_eq!(reactions[3].trigger, Trigger::MouseLeave);
    assert_eq!(reactions[4].trigger, Trigger::WhileHovering);
}

#[test]
fn pr8_unknown_trigger_drops_the_reaction_and_counts_it() {
    let fig = frame_a_with_interactions(vec![
        o(
            "PrototypeInteraction",
            vec![
                ("event", prototype_event("ON_MEDIA_END")),
                (
                    "actions",
                    KiwiValue::array(vec![navigate_action(guid(0, 2), None)]),
                ),
            ],
        ),
        o(
            "PrototypeInteraction",
            vec![
                ("event", prototype_event("ON_CLICK")),
                (
                    "actions",
                    KiwiValue::array(vec![navigate_action(guid(0, 3), None)]),
                ),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    // The media trigger is DROPPED (not defaulted to Click — firing an
    // on-media-end navigation on click is worse than not firing) and the loss
    // is counted; the recognized interaction still imports.
    assert_eq!(report.reactions, 1);
    assert_eq!(report.reactions_dropped_unknown_trigger, 1);

    let reactions = reactions_of(&doc, "A");
    assert_eq!(reactions.len(), 1);
    assert_eq!(reactions[0].trigger, Trigger::Click);
}
