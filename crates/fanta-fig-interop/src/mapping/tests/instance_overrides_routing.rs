use super::*;

// =============================================================================
// Task: instance overrides (text/fill/visibility) + auto-width text
// =============================================================================

/// A `GUIDPath` addressing a single master descendant guid.
#[test]
fn symbol_override_text_replaces_master_default_on_expand() {
    // A SYMBOL master with a TEXT child reading the placeholder "Title". An
    // INSTANCE carries a symbolOverride (guidPath → the TEXT child) whose textData
    // sets "Action Bar". Expanding the instance must yield "Action Bar", not the
    // master default.
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("Header".to_owned())),
                ("size", vector(300.0, 80.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("TEXT".into())),
                ("name", KiwiValue::String("Title".to_owned())),
                ("size", vector(300.0, 40.0)),
                ("textData", text_data("Title")),
                ("fontSize", KiwiValue::Float(45.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("type", KiwiValue::Enum("INSTANCE".into())),
                ("name", KiwiValue::String("Header instance".to_owned())),
                ("size", vector(300.0, 80.0)),
                (
                    "symbolData",
                    o(
                        "SymbolData",
                        vec![
                            ("symbolID", guid(0, 1)),
                            (
                                "symbolOverrides",
                                KiwiValue::array(vec![o(
                                    "NodeChange",
                                    vec![
                                        ("guidPath", guid_path(0, 2)),
                                        ("textData", text_data("Action Bar")),
                                    ],
                                )]),
                            ),
                        ],
                    ),
                ),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(
        report.instances_with_overrides, 1,
        "the instance carries an override"
    );
    assert!(report.overrides_applied >= 1);

    let expanded = expand_only_instance(&doc);
    let title = expanded.iter().find_map(|e| match &e.node.data {
        NodeData::Text(t) => Some(t.content.clone()),
        _ => None,
    });
    assert_eq!(
        title.as_deref(),
        Some("Action Bar"),
        "the symbolOverride text replaces the master 'Title' default"
    );
}

#[test]
fn symbol_override_text_alignment_snapshot_does_not_recolor_text() {
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("Cell".to_owned())),
                ("size", vector(120.0, 32.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("TEXT".into())),
                ("name", KiwiValue::String("Value".to_owned())),
                ("size", vector(80.0, 18.0)),
                ("textData", text_data("Row item")),
                (
                    "fillPaints",
                    KiwiValue::array(vec![solid_paint(
                        235.0 / 255.0,
                        235.0 / 255.0,
                        235.0 / 255.0,
                        1.0,
                    )]),
                ),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("type", KiwiValue::Enum("INSTANCE".into())),
                ("name", KiwiValue::String("Cell instance".to_owned())),
                ("size", vector(120.0, 32.0)),
                (
                    "symbolData",
                    o(
                        "SymbolData",
                        vec![
                            ("symbolID", guid(0, 1)),
                            (
                                "symbolOverrides",
                                KiwiValue::array(vec![o(
                                    "NodeChange",
                                    vec![
                                        ("guidPath", guid_path(0, 2)),
                                        ("textAlignHorizontal", KiwiValue::Enum("RIGHT".into())),
                                        (
                                            "fillPaints",
                                            KiwiValue::array(vec![solid_paint(
                                                34.0 / 255.0,
                                                34.0 / 255.0,
                                                34.0 / 255.0,
                                                1.0,
                                            )]),
                                        ),
                                    ],
                                )]),
                            ),
                        ],
                    ),
                ),
            ],
        ),
    ]);

    let (doc, _report, _) = fig_to_doc(&fig).unwrap();
    let expanded = expand_only_instance(&doc);
    let text = expanded
        .iter()
        .find_map(|expanded| match &expanded.node.data {
            NodeData::Text(text) => Some(text),
            _ => None,
        })
        .expect("expanded text node");

    assert_eq!(
        text.style.color,
        Color::rgba(235, 235, 235, 255),
        "text alignment snapshots must not be imported as text color overrides"
    );
}

/// Build a nested-component `.fig`: an INNER "Chip" SYMBOL (guid 0:1) with a
/// TEXT child (0:2), an OUTER "Card" SYMBOL (0:10) whose child is an INSTANCE
/// (0:11) of Chip, and a top-level INSTANCE (0:20) of Card carrying `extra`
/// override/derived entries. The Card-instance override's guidPath is length 2:
/// `[0:11 (nested instance), 0:2 (inner text)]` — it crosses the nested boundary.
#[test]
fn length2_guidpath_override_routes_onto_nested_instance_and_applies() {
    // The Card instance carries a symbolOverride whose guidPath is length 2
    // (`[nested chip 0:11, inner text 0:2]`). The old terminal-guid-only keying
    // would mis-route or drop it; the full-path resolution must route the
    // remainder onto the nested chip instance so the inner text reads "Routed"
    // after the two-level expansion.
    let fig = nested_component_doc(vec![(
        "symbolData",
        o(
            "SymbolData",
            vec![
                ("symbolID", guid(0, 10)),
                (
                    "symbolOverrides",
                    KiwiValue::array(vec![o(
                        "NodeChange",
                        vec![
                            ("guidPath", guid_path2((0, 11), (0, 2))),
                            ("textData", text_data("Routed")),
                        ],
                    )]),
                ),
            ],
        ),
    )]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.override_path_nested, 1, "one length>1 entry counted");
    assert_eq!(
        report.override_nested_resolved, 1,
        "and it resolved its full path"
    );

    // Find the Card instance (component is "Card") and expand twice.
    let card = doc
        .scene
        .roots()
        .iter()
        .flat_map(|r| doc.scene.descendants_of(*r))
        .find_map(|id| match doc.scene.get(id).map(|n| &n.data) {
            Some(NodeData::Instance(i)) => Some(i.clone()),
            _ => None,
        })
        .expect("card instance present");
    let outer = expand_instance(&doc.scene, &doc.components, &card);
    let nested = outer
        .iter()
        .find_map(|e| match &e.node.data {
            NodeData::Instance(i) => Some(i.clone()),
            _ => None,
        })
        .expect("nested chip instance in the expansion");
    let inner = expand_instance(&doc.scene, &doc.components, &nested);
    let text = inner.iter().find_map(|e| match &e.node.data {
        NodeData::Text(t) => Some(t.content.clone()),
        _ => None,
    });
    assert_eq!(
        text.as_deref(),
        Some("Routed"),
        "the length-2 override applied to the inner text on recursion"
    );
}

#[test]
fn overridden_symbol_id_swaps_the_nested_component_on_import() {
    // The Card instance carries a symbolOverride at the nested chip (guidPath
    // length 1 → the nested instance) with `overriddenSymbolID` pointing at an
    // ALT component. After import + outer expansion, the nested chip clone must
    // point at the Alt component.
    let fig = doc_from(vec![
        // Inner master "Chip".
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("Chip".to_owned())),
                ("size", vector(60.0, 20.0)),
            ],
        ),
        // Outer master "Card" holding an INSTANCE of Chip.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 10)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("Card".to_owned())),
                ("size", vector(120.0, 40.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 11)),
                ("parentIndex", parent_index(0, 10)),
                ("type", KiwiValue::Enum("INSTANCE".into())),
                ("name", KiwiValue::String("Nested chip".to_owned())),
                ("size", vector(60.0, 20.0)),
                (
                    "symbolData",
                    o("SymbolData", vec![("symbolID", guid(0, 1))]),
                ),
            ],
        ),
        // ALT SYMBOL master (guid 0:30) to swap to.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 30)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("Alt".to_owned())),
                ("size", vector(60.0, 20.0)),
            ],
        ),
        // Top-level INSTANCE of Card, with a swap override on the nested chip.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 20)),
                ("type", KiwiValue::Enum("INSTANCE".into())),
                ("name", KiwiValue::String("Card instance".to_owned())),
                ("size", vector(120.0, 40.0)),
                (
                    "symbolData",
                    o(
                        "SymbolData",
                        vec![
                            ("symbolID", guid(0, 10)),
                            (
                                "symbolOverrides",
                                KiwiValue::array(vec![o(
                                    "NodeChange",
                                    vec![
                                        ("guidPath", guid_path(0, 11)),
                                        ("overriddenSymbolID", guid(0, 30)),
                                    ],
                                )]),
                            ),
                        ],
                    ),
                ),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.overridden_symbol_swaps, 1, "one swap seen");
    assert_eq!(
        report.overridden_symbol_resolved, 1,
        "and it resolved a ComponentId"
    );

    // The Alt component's id (resolve by its master name).
    let alt_id = doc
        .components
        .defs
        .iter()
        .find(|(_, d)| d.name == "Alt")
        .map(|(id, _)| *id)
        .expect("Alt component exists");

    let card = doc
        .scene
        .roots()
        .iter()
        .flat_map(|r| doc.scene.descendants_of(*r))
        .find_map(|id| match doc.scene.get(id).map(|n| &n.data) {
            Some(NodeData::Instance(i))
                if doc.components.def(i.component).map(|d| d.name.as_str()) == Some("Card") =>
            {
                Some(i.clone())
            }
            _ => None,
        })
        .expect("card instance present");
    let outer = expand_instance(&doc.scene, &doc.components, &card);
    let nested_comp = outer.iter().find_map(|e| match &e.node.data {
        NodeData::Instance(i) => Some(i.component),
        _ => None,
    });
    assert_eq!(
        nested_comp,
        Some(alt_id),
        "overriddenSymbolID re-pointed the nested chip to Alt"
    );
}

fn swap_context_path(locals: &[u32]) -> KiwiValue {
    o(
        "GUIDPath",
        vec![(
            "guids",
            KiwiValue::array(locals.iter().map(|local| guid(0, *local)).collect()),
        )],
    )
}

fn swap_context_override(path: &[u32], component: u32) -> KiwiValue {
    o(
        "NodeChange",
        vec![
            ("guidPath", swap_context_path(path)),
            ("overriddenSymbolID", guid(0, component)),
        ],
    )
}

fn swap_context_instance(
    local: u32,
    parent: Option<u32>,
    component: u32,
    overrides: Vec<KiwiValue>,
    extra: Vec<(&str, KiwiValue)>,
) -> KiwiValue {
    let mut fields = vec![
        ("guid", guid(0, local)),
        ("type", KiwiValue::Enum("INSTANCE".into())),
        ("name", KiwiValue::String(format!("Placement {local}"))),
        ("size", vector(120.0, 40.0)),
        (
            "symbolData",
            o(
                "SymbolData",
                vec![
                    ("symbolID", guid(0, component)),
                    ("symbolOverrides", KiwiValue::array(overrides)),
                ],
            ),
        ),
    ];
    if let Some(parent) = parent {
        fields.push(("parentIndex", parent_index(0, parent)));
    }
    fields.extend(extra);
    o("NodeChange", fields)
}

fn swap_context_components() -> Vec<KiwiValue> {
    let mut nodes = Vec::new();
    for (local, name) in [(1, "A"), (21, "B"), (51, "C"), (10, "Card"), (30, "Board")] {
        nodes.push(o(
            "NodeChange",
            vec![
                ("guid", guid(0, local)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String(name.to_owned())),
                ("size", vector(120.0, 40.0)),
            ],
        ));
    }
    for (local, parent, content) in [(2, 1, "A"), (22, 21, "B"), (52, 51, "C")] {
        nodes.push(o(
            "NodeChange",
            vec![
                ("guid", guid(0, local)),
                ("parentIndex", parent_index(0, parent)),
                ("type", KiwiValue::Enum("TEXT".into())),
                ("name", KiwiValue::String(content.to_owned())),
                ("size", vector(60.0, 16.0)),
                ("textData", text_data(content)),
                ("fontSize", KiwiValue::Float(12.0)),
            ],
        ));
    }
    nodes.push(swap_context_instance(
        11,
        Some(10),
        1,
        Vec::new(),
        vec![(
            "componentPropRefs",
            KiwiValue::array(vec![o(
                "ComponentPropRef",
                vec![
                    ("defID", guid(7, 1)),
                    (
                        "componentPropNodeField",
                        KiwiValue::Enum("OVERRIDDEN_SYMBOL_ID".into()),
                    ),
                ],
            )]),
        )],
    ));
    nodes
}

fn swap_context_content(path: &[u32], content: &str) -> KiwiValue {
    o(
        "NodeChange",
        vec![
            ("guidPath", swap_context_path(path)),
            ("textData", text_data(content)),
        ],
    )
}

fn swap_context_derived(path: &[u32], font_size: f32) -> KiwiValue {
    o(
        "NodeChange",
        vec![
            ("guidPath", swap_context_path(path)),
            ("fontSize", KiwiValue::Float(font_size)),
        ],
    )
}

fn assert_swap_context_expansion(doc: &Doc, depth: usize, expected_master: &str) {
    let mut instance = doc
        .scene
        .roots()
        .iter()
        .flat_map(|root| doc.scene.descendants_of(*root))
        .filter_map(|id| doc.scene.get(id))
        .find_map(|node| match &node.data {
            NodeData::Instance(instance) if node.name == "Placement 40" => Some(instance.clone()),
            _ => None,
        })
        .expect("outer placement");
    for _ in 0..depth {
        let expanded = expand_instance(&doc.scene, &doc.components, &instance);
        instance = expanded
            .iter()
            .find_map(|entry| match &entry.node.data {
                NodeData::Instance(instance) => Some(instance.clone()),
                _ => None,
            })
            .expect("nested placement");
    }
    assert_eq!(
        doc.components
            .def(instance.component)
            .expect("selected master")
            .name,
        expected_master,
    );
    let expanded = expand_instance(&doc.scene, &doc.components, &instance);
    let text = expanded
        .iter()
        .find_map(|entry| match &entry.node.data {
            NodeData::Text(text) => Some(text),
            _ => None,
        })
        .expect("expanded text");
    assert_eq!(text.content, "Authored");
    assert_eq!(text.style.size_px, 27.0);
    for node in doc
        .scene
        .roots()
        .iter()
        .flat_map(|root| doc.scene.descendants_of(*root))
        .filter_map(|id| doc.scene.get(id))
    {
        if let NodeData::Text(text) = &node.data {
            assert_ne!(text.content, "Authored", "masters remain unchanged");
            assert_eq!(text.style.size_px, 12.0);
        }
    }
}

#[test]
fn component_property_swap_routes_nested_content_and_derived_in_any_source_order() {
    for reverse in [false, true] {
        for conflicting_symbol_swap in [false, true] {
            let mut nodes = swap_context_components();
            let mut overrides = Vec::new();
            if conflicting_symbol_swap {
                overrides.push(swap_context_override(&[11], 51));
            }
            overrides.extend([
                swap_context_content(&[11, 22], "Authored"),
                swap_context_content(&[11, 52], "Foreign component"),
            ]);
            nodes.push(swap_context_instance(
                40,
                None,
                10,
                overrides,
                vec![
                    (
                        "componentPropAssignments",
                        KiwiValue::array(vec![o(
                            "ComponentPropAssignment",
                            vec![("defID", guid(7, 1)), ("value", prop_value_guid(0, 21))],
                        )]),
                    ),
                    (
                        "derivedSymbolData",
                        KiwiValue::array(vec![
                            swap_context_derived(&[11, 22], 27.0),
                            swap_context_derived(&[11, 52], 99.0),
                        ]),
                    ),
                ],
            ));
            if reverse {
                nodes.reverse();
            }
            let (doc, report, _) = fig_to_doc(&doc_from(nodes)).expect("import swap fixture");
            assert_eq!(report.prop_instance_swap_resolved, 1);
            assert_eq!(report.override_path_nested, 4);
            assert_eq!(report.override_nested_resolved, 2);
            assert_swap_context_expansion(&doc, 1, "B");
        }
    }
}

#[test]
fn inherited_instance_swap_routes_nested_content_and_derived_in_any_source_order() {
    for reverse in [false, true] {
        for outer_swap in [false, true] {
            let mut nodes = swap_context_components();
            nodes.push(swap_context_instance(
                31,
                Some(30),
                10,
                vec![swap_context_override(&[11], 21)],
                Vec::new(),
            ));
            let (target, foreign, expected_master) = if outer_swap {
                (52, 22, "C")
            } else {
                (22, 52, "B")
            };
            let mut overrides = Vec::new();
            if outer_swap {
                overrides.push(swap_context_override(&[31, 11], 51));
            }
            overrides.extend([
                swap_context_content(&[31, 11, target], "Authored"),
                swap_context_content(&[31, 11, foreign], "Foreign component"),
            ]);
            nodes.push(swap_context_instance(
                40,
                None,
                30,
                overrides,
                vec![(
                    "derivedSymbolData",
                    KiwiValue::array(vec![
                        swap_context_derived(&[31, 11, target], 27.0),
                        swap_context_derived(&[31, 11, foreign], 99.0),
                    ]),
                )],
            ));
            if reverse {
                nodes.reverse();
            }
            let (doc, report, _) = fig_to_doc(&doc_from(nodes)).expect("import swap fixture");
            assert_eq!(report.override_path_nested, 4 + usize::from(outer_swap));
            assert_eq!(report.override_nested_resolved, 2 + usize::from(outer_swap));
            assert_swap_context_expansion(&doc, 2, expected_master);
        }
    }
}

#[test]
fn swap_redirect_cannot_cross_a_non_instance_node() {
    let mut nodes = swap_context_components();
    nodes.push(swap_context_instance(
        40,
        None,
        10,
        vec![
            swap_context_override(&[11], 21),
            swap_context_override(&[11, 22], 51),
            swap_context_content(&[11, 22], "Authored"),
            swap_context_content(&[11, 22, 52], "Foreign component"),
        ],
        vec![(
            "derivedSymbolData",
            KiwiValue::array(vec![swap_context_derived(&[11, 22], 27.0)]),
        )],
    ));
    let (doc, report, _) = fig_to_doc(&doc_from(nodes)).expect("import swap fixture");
    assert_eq!(report.override_path_nested, 4);
    assert_eq!(report.override_nested_resolved, 3);
    assert_swap_context_expansion(&doc, 1, "B");
}

fn swap_context_nested_assignment(path: &[u32], component: u32) -> KiwiValue {
    o(
        "NodeChange",
        vec![
            ("guidPath", swap_context_path(path)),
            (
                "componentPropAssignments",
                KiwiValue::array(vec![o(
                    "ComponentPropAssignment",
                    vec![
                        ("defID", guid(7, 1)),
                        ("value", prop_value_guid(0, component)),
                    ],
                )]),
            ),
        ],
    )
}

#[test]
fn nested_property_assignment_preserves_selected_component_and_content_in_source_order() {
    for root_target in [false, true] {
        for reverse in [false, true] {
            for assignment_last in [false, true] {
                let mut nodes = swap_context_components();
                nodes.push(swap_context_instance(
                    31,
                    Some(30),
                    10,
                    Vec::new(),
                    Vec::new(),
                ));
                let assignment =
                    swap_context_nested_assignment(if root_target { &[31, 10] } else { &[31] }, 21);
                let swap = swap_context_override(&[31, 11], 51);
                let (mut overrides, target, foreign, expected_master) = if assignment_last {
                    (vec![swap, assignment], 22, 52, "B")
                } else {
                    (vec![assignment, swap], 52, 22, "C")
                };
                overrides.extend([
                    swap_context_content(&[31, 11, target], "Authored"),
                    swap_context_content(&[31, 11, foreign], "Foreign component"),
                ]);
                nodes.push(swap_context_instance(
                    40,
                    None,
                    30,
                    overrides,
                    vec![(
                        "derivedSymbolData",
                        KiwiValue::array(vec![
                            swap_context_derived(&[31, 11, target], 27.0),
                            swap_context_derived(&[31, 11, foreign], 99.0),
                        ]),
                    )],
                ));
                if reverse {
                    nodes.reverse();
                }
                let (doc, report, _) =
                    fig_to_doc(&doc_from(nodes)).expect("import nested assignment");
                assert_eq!(report.prop_instance_swap_resolved, 1);
                assert_eq!(report.override_path_nested, 5 + usize::from(root_target));
                assert_eq!(
                    report.override_nested_resolved,
                    3 + usize::from(root_target)
                );
                assert_swap_context_expansion(&doc, 2, expected_master);
            }
        }
    }
}

#[test]
fn nested_property_assignment_binds_only_inside_the_selected_component() {
    let mut nodes = swap_context_components();
    nodes.push(o(
        "NodeChange",
        vec![
            ("guid", guid(0, 70)),
            ("type", KiwiValue::Enum("SYMBOL".into())),
            ("name", KiwiValue::String("Alternate card".to_owned())),
            ("size", vector(120.0, 40.0)),
        ],
    ));
    nodes.push(swap_context_instance(
        71,
        Some(70),
        1,
        Vec::new(),
        vec![(
            "componentPropRefs",
            KiwiValue::array(vec![o(
                "ComponentPropRef",
                vec![
                    ("defID", guid(7, 1)),
                    (
                        "componentPropNodeField",
                        KiwiValue::Enum("OVERRIDDEN_SYMBOL_ID".into()),
                    ),
                ],
            )]),
        )],
    ));
    nodes.push(swap_context_instance(
        31,
        Some(30),
        10,
        Vec::new(),
        Vec::new(),
    ));
    nodes.push(swap_context_instance(
        40,
        None,
        30,
        vec![
            swap_context_nested_assignment(&[31], 21),
            swap_context_override(&[31], 70),
            swap_context_content(&[31, 71, 22], "Authored"),
            swap_context_content(&[31, 11, 22], "Old component"),
        ],
        vec![(
            "derivedSymbolData",
            KiwiValue::array(vec![swap_context_derived(&[31, 71, 22], 27.0)]),
        )],
    ));
    let (doc, report, _) = fig_to_doc(&doc_from(nodes)).expect("import nested assignment");
    assert_eq!(report.prop_instance_swap_resolved, 1);
    assert_eq!(report.override_path_nested, 3);
    assert_eq!(report.override_nested_resolved, 2);
    assert_swap_context_expansion(&doc, 2, "B");
}

#[test]
fn nested_property_assignment_resolves_ancestor_root_before_child_properties() {
    let mut nodes = swap_context_components();
    let text = nodes
        .iter_mut()
        .find(|node| node.get("guid") == Some(&guid(0, 22)))
        .expect("B text");
    text.set_field(
        "componentPropRefs",
        KiwiValue::array(
            [(2, "TEXT_DATA"), (3, "VISIBLE")]
                .into_iter()
                .map(|(property, field)| {
                    o(
                        "ComponentPropRef",
                        vec![
                            ("defID", guid(7, property)),
                            ("componentPropNodeField", KiwiValue::Enum(field.into())),
                        ],
                    )
                })
                .collect(),
        ),
    );
    nodes.push(swap_context_instance(
        31,
        Some(30),
        10,
        Vec::new(),
        Vec::new(),
    ));
    nodes.push(swap_context_instance(
        40,
        None,
        30,
        vec![
            o(
                "NodeChange",
                vec![
                    ("guidPath", swap_context_path(&[31, 11])),
                    (
                        "componentPropAssignments",
                        KiwiValue::array(vec![
                            o(
                                "ComponentPropAssignment",
                                vec![
                                    ("defID", guid(7, 2)),
                                    ("value", prop_value_text("Authored")),
                                ],
                            ),
                            o(
                                "ComponentPropAssignment",
                                vec![("defID", guid(7, 3)), ("value", prop_value_bool(false))],
                            ),
                        ]),
                    ),
                ],
            ),
            swap_context_nested_assignment(&[31, 10], 21),
        ],
        vec![(
            "derivedSymbolData",
            KiwiValue::array(vec![swap_context_derived(&[31, 11, 22], 27.0)]),
        )],
    ));
    let (doc, report, _) = fig_to_doc(&doc_from(nodes)).expect("import nested properties");
    assert_eq!(report.prop_instance_swap_resolved, 1);
    assert_eq!(report.prop_visible_resolved, 1);
    assert_eq!(report.override_nested_resolved, 3);
    assert_swap_context_expansion(&doc, 2, "B");
}
