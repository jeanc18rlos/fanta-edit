use super::*;

#[test]
fn component_prop_assignment_text_drives_bound_master_text() {
    // A SYMBOL master with a TEXT child bound to a prop-def via componentPropRefs
    // (TEXT_DATA). An INSTANCE assigns that prop-def the value "Action Bar" via
    // componentPropAssignments. The bound TEXT must read "Action Bar" on expand.
    let prop_def = guid(7, 183);
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
                (
                    "componentPropRefs",
                    KiwiValue::array(vec![o(
                        "ComponentPropRef",
                        vec![
                            ("defID", prop_def.clone()),
                            (
                                "componentPropNodeField",
                                KiwiValue::Enum("TEXT_DATA".into()),
                            ),
                        ],
                    )]),
                ),
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
                    o("SymbolData", vec![("symbolID", guid(0, 1))]),
                ),
                (
                    "componentPropAssignments",
                    KiwiValue::array(vec![o(
                        "ComponentPropAssignment",
                        vec![
                            ("defID", prop_def),
                            ("value", prop_value_text("Action Bar")),
                        ],
                    )]),
                ),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.instances_with_overrides, 1);

    let expanded = expand_only_instance(&doc);
    let title = expanded.iter().find_map(|e| match &e.node.data {
        NodeData::Text(t) => Some(t.content.clone()),
        _ => None,
    });
    assert_eq!(
        title.as_deref(),
        Some("Action Bar"),
        "the prop assignment drives the bound master TEXT content"
    );
}

/// A `ComponentPropValue` carrying a `boolValue` (a BOOL/VISIBLE prop).
#[test]
fn component_prop_assignments_drive_icon_swap_visibility_and_label() {
    // The Action Bar button shape: a SHARED Button master whose nested icon
    // INSTANCE binds `OVERRIDDEN_SYMBOL_ID` to an "Icon" prop-def, whose Label
    // TEXT binds `TEXT_DATA` to a "Label" prop-def, and whose placeholder VECTOR
    // binds `VISIBLE` to a "Hold" bool prop-def. An outer INSTANCE assigns all
    // three: swap the icon to the Copy symbol, set the label to "Copy", hide the
    // placeholder. Expanding must reflect ALL three — the regression that left
    // every button showing the master's default icon + a spurious ▪ + (for some)
    // an empty label.
    let icon_prop = guid(366, 276);
    let label_prop = guid(366, 548);
    let hold_prop = guid(366, 4);

    let fig = doc_from(vec![
        // Default icon master (the "Edit" pencil), one vector.
        o(
            "NodeChange",
            vec![
                ("guid", guid(41, 35100)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("Edit".to_owned())),
                ("size", vector(18.0, 18.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(41, 35101)),
                ("parentIndex", parent_index(41, 35100)),
                ("type", KiwiValue::Enum("VECTOR".into())),
                ("name", KiwiValue::String("Vector".to_owned())),
                ("size", vector(18.0, 18.0)),
            ],
        ),
        // Swap-target icon master (the "Copy" overlapping-squares), one vector.
        o(
            "NodeChange",
            vec![
                ("guid", guid(41, 35424)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("Copy".to_owned())),
                ("size", vector(18.0, 18.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(41, 35425)),
                ("parentIndex", parent_index(41, 35424)),
                ("type", KiwiValue::Enum("VECTOR".into())),
                ("name", KiwiValue::String("Vector".to_owned())),
                ("size", vector(18.0, 18.0)),
            ],
        ),
        // The Button master, with the three prop-defs declared on it.
        o(
            "NodeChange",
            vec![
                ("guid", guid(366, 43647)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("Button".to_owned())),
                ("size", vector(80.0, 32.0)),
            ],
        ),
        // Nested icon instance (default = Edit), binds OVERRIDDEN_SYMBOL_ID → Icon.
        o(
            "NodeChange",
            vec![
                ("guid", guid(366, 50)),
                ("parentIndex", parent_index(366, 43647)),
                ("type", KiwiValue::Enum("INSTANCE".into())),
                ("name", KiwiValue::String("Edit".to_owned())),
                ("size", vector(18.0, 18.0)),
                (
                    "symbolData",
                    o("SymbolData", vec![("symbolID", guid(41, 35100))]),
                ),
                (
                    "componentPropRefs",
                    KiwiValue::array(vec![o(
                        "ComponentPropRef",
                        vec![
                            ("defID", icon_prop.clone()),
                            (
                                "componentPropNodeField",
                                KiwiValue::Enum("OVERRIDDEN_SYMBOL_ID".into()),
                            ),
                        ],
                    )]),
                ),
            ],
        ),
        // Label text, binds TEXT_DATA → Label.
        o(
            "NodeChange",
            vec![
                ("guid", guid(366, 51)),
                ("parentIndex", parent_index(366, 43647)),
                ("type", KiwiValue::Enum("TEXT".into())),
                ("name", KiwiValue::String("Label".to_owned())),
                ("size", vector(40.0, 18.0)),
                ("textData", text_data("Action")),
                ("fontSize", KiwiValue::Float(14.0)),
                (
                    "componentPropRefs",
                    KiwiValue::array(vec![o(
                        "ComponentPropRef",
                        vec![
                            ("defID", label_prop.clone()),
                            (
                                "componentPropNodeField",
                                KiwiValue::Enum("TEXT_DATA".into()),
                            ),
                        ],
                    )]),
                ),
            ],
        ),
        // Hold-Icon placeholder vector, binds VISIBLE → Hold (the ▪).
        o(
            "NodeChange",
            vec![
                ("guid", guid(366, 52)),
                ("parentIndex", parent_index(366, 43647)),
                ("type", KiwiValue::Enum("VECTOR".into())),
                ("name", KiwiValue::String("Hold Icon".to_owned())),
                ("size", vector(8.0, 8.0)),
                (
                    "componentPropRefs",
                    KiwiValue::array(vec![o(
                        "ComponentPropRef",
                        vec![
                            ("defID", hold_prop.clone()),
                            ("componentPropNodeField", KiwiValue::Enum("VISIBLE".into())),
                        ],
                    )]),
                ),
            ],
        ),
        // The outer button INSTANCE: swap icon → Copy, label → "Copy", hide Hold.
        o(
            "NodeChange",
            vec![
                ("guid", guid(900, 1)),
                ("type", KiwiValue::Enum("INSTANCE".into())),
                ("name", KiwiValue::String("Center Action Button".to_owned())),
                ("size", vector(80.0, 32.0)),
                (
                    "symbolData",
                    o("SymbolData", vec![("symbolID", guid(366, 43647))]),
                ),
                (
                    "componentPropAssignments",
                    KiwiValue::array(vec![
                        o(
                            "ComponentPropAssignment",
                            vec![("defID", icon_prop), ("value", prop_value_guid(41, 35424))],
                        ),
                        o(
                            "ComponentPropAssignment",
                            vec![("defID", label_prop), ("value", prop_value_text("Copy"))],
                        ),
                        o(
                            "ComponentPropAssignment",
                            vec![("defID", hold_prop), ("value", prop_value_bool(false))],
                        ),
                    ]),
                ),
            ],
        ),
    ]);

    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(
        report.prop_instance_swap_resolved, 1,
        "the icon swap resolved"
    );
    assert_eq!(
        report.prop_visible_resolved, 1,
        "the visibility prop resolved"
    );

    // Find and expand the OUTER button instance (not the nested icon instance).
    let inst = doc
        .scene
        .roots()
        .iter()
        .flat_map(|r| doc.scene.descendants_of(*r))
        .find_map(|id| match doc.scene.get(id).map(|n| &n.data) {
            Some(NodeData::Instance(i))
                if doc.scene.get(id).map(|n| n.name.as_str()) == Some("Center Action Button") =>
            {
                Some(i.clone())
            }
            _ => None,
        })
        .expect("outer button instance present");
    let expanded = expand_instance(&doc.scene, &doc.components, &inst);

    // Label resolves to "Copy".
    let label = expanded.iter().find_map(|e| match &e.node.data {
        NodeData::Text(t) => Some(t.content.clone()),
        _ => None,
    });
    assert_eq!(
        label.as_deref(),
        Some("Copy"),
        "label prop drives the bound TEXT"
    );

    // The Hold-Icon vector is hidden.
    let hold_hidden = expanded
        .iter()
        .any(|e| e.node.name == "Hold Icon" && e.node.flags.contains(fanta_doc::NodeFlags::HIDDEN));
    assert!(
        hold_hidden,
        "the bool VISIBLE prop hides the placeholder vector"
    );

    // The nested icon instance is re-pointed to the Copy master (its expansion
    // yields the Copy symbol's vector, not the Edit master's).
    let icon = expanded.iter().find_map(|e| match &e.node.data {
        NodeData::Instance(i) => Some(i.clone()),
        _ => None,
    });
    let icon = icon.expect("nested icon instance clone present");
    let copy_comp = doc
        .components
        .defs
        .values()
        .find(|d| d.name == "Copy")
        .map(|d| d.id)
        .expect("Copy component registered");
    assert_eq!(
        icon.component, copy_comp,
        "the INSTANCE_SWAP assignment re-points the icon"
    );
}
