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
                ("type", KiwiValue::Enum("SYMBOL".to_owned())),
                ("name", KiwiValue::String("Header".to_owned())),
                ("size", vector(300.0, 80.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("TEXT".to_owned())),
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
                ("type", KiwiValue::Enum("INSTANCE".to_owned())),
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
                                KiwiValue::Array(vec![o(
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
                    KiwiValue::Array(vec![o(
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
                ("type", KiwiValue::Enum("SYMBOL".to_owned())),
                ("name", KiwiValue::String("Chip".to_owned())),
                ("size", vector(60.0, 20.0)),
            ],
        ),
        // Outer master "Card" holding an INSTANCE of Chip.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 10)),
                ("type", KiwiValue::Enum("SYMBOL".to_owned())),
                ("name", KiwiValue::String("Card".to_owned())),
                ("size", vector(120.0, 40.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 11)),
                ("parentIndex", parent_index(0, 10)),
                ("type", KiwiValue::Enum("INSTANCE".to_owned())),
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
                ("type", KiwiValue::Enum("SYMBOL".to_owned())),
                ("name", KiwiValue::String("Alt".to_owned())),
                ("size", vector(60.0, 20.0)),
            ],
        ),
        // Top-level INSTANCE of Card, with a swap override on the nested chip.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 20)),
                ("type", KiwiValue::Enum("INSTANCE".to_owned())),
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
                                KiwiValue::Array(vec![o(
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
