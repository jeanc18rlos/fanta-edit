//! Variable collections, variables, and consumption-map bindings.

use super::*;

// =============================================================================
// Task 3: Variables
// =============================================================================

fn var_set_mode(sid: u32, lid: u32, name: &str) -> KiwiValue {
    o(
        "VariableSetMode",
        vec![
            ("id", guid(sid, lid)),
            ("name", KiwiValue::String(name.to_owned())),
        ],
    )
}

fn var_color_value(r: f32, g: f32, b: f32, a: f32) -> KiwiValue {
    o(
        "VariableData",
        vec![(
            "value",
            o("VariableAnyValue", vec![("colorValue", color(r, g, b, a))]),
        )],
    )
}

fn var_float_value(v: f32) -> KiwiValue {
    o(
        "VariableData",
        vec![(
            "value",
            o(
                "VariableAnyValue",
                vec![("floatValue", KiwiValue::Float(v))],
            ),
        )],
    )
}

fn bound_solid_paint(r: f32, g: f32, b: f32, a: f32, var_sid: u32, var_lid: u32) -> KiwiValue {
    o(
        "Paint",
        vec![
            ("type", KiwiValue::Enum("SOLID".into())),
            ("color", color(r, g, b, a)),
            (
                "boundVariables",
                o(
                    "PaintBoundVariables",
                    vec![("color", variable_id(var_sid, var_lid))],
                ),
            ),
        ],
    )
}

#[test]
fn variable_set_maps_to_collection_with_modes_and_variable_values() {
    // VARIABLE_SET with two modes + a VARIABLE with a per-mode color value.
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("VARIABLE_SET".into())),
                ("name", KiwiValue::String("Theme".to_owned())),
                (
                    "variableSetModes",
                    KiwiValue::Array(vec![
                        var_set_mode(0, 10, "Light"),
                        var_set_mode(0, 11, "Dark"),
                    ]),
                ),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("type", KiwiValue::Enum("VARIABLE".into())),
                ("name", KiwiValue::String("bg".to_owned())),
                ("variableSetID", variable_id(0, 1)),
                ("variableResolvedType", KiwiValue::Enum("COLOR".into())),
                (
                    "variableDataValues",
                    o(
                        "VariableDataValues",
                        vec![(
                            "entries",
                            KiwiValue::Array(vec![
                                o(
                                    "VariableDataValuesEntry",
                                    vec![
                                        ("modeID", guid(0, 10)),
                                        ("variableData", var_color_value(1.0, 1.0, 1.0, 1.0)),
                                    ],
                                ),
                                o(
                                    "VariableDataValuesEntry",
                                    vec![
                                        ("modeID", guid(0, 11)),
                                        ("variableData", var_color_value(0.0, 0.0, 0.0, 1.0)),
                                    ],
                                ),
                            ]),
                        )],
                    ),
                ),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.variable_collections, 1);
    assert_eq!(report.variables, 1);

    let coll = doc.variables.collections.values().next().unwrap();
    assert_eq!(coll.name, "Theme");
    assert_eq!(coll.modes.len(), 2);
    assert_eq!(coll.modes[0].name, "Light");
    assert_eq!(coll.modes[1].name, "Dark");

    let var = doc.variables.variables.values().next().unwrap();
    assert_eq!(var.name, "bg");
    assert_eq!(var.ty, VariableType::Color);
    assert_eq!(var.collection, coll.id);
    // Two per-mode color values resolved onto the collection's mode ids.
    assert_eq!(var.values_by_mode.len(), 2);
    let light_mode = coll.modes[0].id;
    assert_eq!(
        var.values_by_mode.get(&light_mode),
        Some(&VarValue::Color {
            value: Color::rgba(255, 255, 255, 255)
        })
    );
    doc.scene.validate().unwrap();
}

#[test]
fn float_variable_resolves_type_and_value() {
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("VARIABLE_SET".into())),
                ("name", KiwiValue::String("Spacing".to_owned())),
                (
                    "variableSetModes",
                    KiwiValue::Array(vec![var_set_mode(0, 10, "Base")]),
                ),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("type", KiwiValue::Enum("VARIABLE".into())),
                ("name", KiwiValue::String("gap".to_owned())),
                ("variableSetID", variable_id(0, 1)),
                ("variableResolvedType", KiwiValue::Enum("FLOAT".into())),
                (
                    "variableDataValues",
                    o(
                        "VariableDataValues",
                        vec![(
                            "entries",
                            KiwiValue::Array(vec![o(
                                "VariableDataValuesEntry",
                                vec![
                                    ("modeID", guid(0, 10)),
                                    ("variableData", var_float_value(8.0)),
                                ],
                            )]),
                        )],
                    ),
                ),
            ],
        ),
    ]);
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    let var = doc.variables.variables.values().next().unwrap();
    assert_eq!(var.ty, VariableType::Float);
    let mode = doc.variables.collections[&var.collection].modes[0].id;
    assert_eq!(
        var.values_by_mode.get(&mode),
        Some(&VarValue::Float { value: 8.0 })
    );
}

#[test]
fn variable_consumption_map_binds_a_node_property() {
    // A rectangle whose corner radius consumes a variable becomes a node with a
    // BoundProp::CornerRadius binding.
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("ROUNDED_RECTANGLE".into())),
            ("size", vector(50.0, 50.0)),
            ("cornerRadius", KiwiValue::Float(4.0)),
            (
                "variableConsumptionMap",
                o(
                    "VariableDataMap",
                    vec![(
                        "entries",
                        KiwiValue::Array(vec![o(
                            "VariableDataMapEntry",
                            vec![
                                (
                                    "variableData",
                                    o(
                                        "VariableData",
                                        vec![(
                                            "value",
                                            o(
                                                "VariableAnyValue",
                                                vec![("alias", variable_id(0, 99))],
                                            ),
                                        )],
                                    ),
                                ),
                                ("variableField", KiwiValue::Enum("CORNER_RADIUS".into())),
                            ],
                        )]),
                    )],
                ),
            ),
        ],
    )]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.bindings, 1);
    let node = doc.scene.get(doc.scene.roots()[0]).unwrap();
    assert_eq!(node.bindings.len(), 1);
    assert!(node.bindings.contains_key(&BoundProp::CornerRadius));
    doc.scene.validate().unwrap();
}

#[test]
fn paint_bound_variables_bind_vector_fill_and_stroke_colors() {
    let fig = doc_from(vec![o(
        "NodeChange",
        vec![
            ("guid", guid(0, 1)),
            ("type", KiwiValue::Enum("RECTANGLE".into())),
            ("size", vector(50.0, 50.0)),
            (
                "fillPaints",
                KiwiValue::Array(vec![bound_solid_paint(1.0, 0.0, 0.0, 1.0, 0, 99)]),
            ),
            (
                "strokePaints",
                KiwiValue::Array(vec![bound_solid_paint(0.0, 0.0, 0.0, 1.0, 0, 100)]),
            ),
            ("strokeWeight", KiwiValue::Float(2.0)),
        ],
    )]);

    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.bindings, 2);
    let node = doc.scene.get(doc.scene.roots()[0]).unwrap();
    assert_eq!(
        node.bindings.get(&BoundProp::FillColor { index: 0 }),
        Some(&guid_to_variable_id("0:99"))
    );
    assert_eq!(
        node.bindings.get(&BoundProp::StrokeColor { index: 0 }),
        Some(&guid_to_variable_id("0:100"))
    );
}

#[test]
fn paint_bound_variables_bind_frame_background_and_text_color() {
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("Frame".to_owned())),
                ("size", vector(50.0, 50.0)),
                (
                    "fillPaints",
                    KiwiValue::Array(vec![bound_solid_paint(1.0, 1.0, 1.0, 1.0, 0, 101)]),
                ),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("type", KiwiValue::Enum("TEXT".into())),
                ("name", KiwiValue::String("Label".to_owned())),
                ("size", vector(80.0, 20.0)),
                ("textData", text_data("Label")),
                (
                    "fillPaints",
                    KiwiValue::Array(vec![bound_solid_paint(0.0, 0.0, 0.0, 1.0, 0, 102)]),
                ),
            ],
        ),
    ]);

    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.bindings, 2);

    let node_by_name = |name: &str| {
        doc.scene
            .roots()
            .iter()
            .flat_map(|root| doc.scene.descendants_of(*root))
            .find_map(|id| doc.scene.get(id).filter(|node| node.name == name))
            .unwrap_or_else(|| panic!("missing node {name}"))
    };

    assert_eq!(
        node_by_name("Frame")
            .bindings
            .get(&BoundProp::FillColor { index: 0 }),
        Some(&guid_to_variable_id("0:101"))
    );
    assert_eq!(
        node_by_name("Label")
            .bindings
            .get(&BoundProp::FillColor { index: 0 }),
        Some(&guid_to_variable_id("0:102"))
    );
}

// =============================================================================
// explicitVariableModes — per-frame mode pinning
// =============================================================================

/// A FRAME's `explicitVariableModes` must land on its `GroupNode.explicit_modes`
/// as a `(collection id -> mode id)` pin, so a frame forced to a non-default
/// mode (a card pinned to Dark) resolves bound values in that mode regardless of
/// the document's active mode. Uses the Figma `VariableModeBySet` shape with a
/// BARE `GUID` for `variableSetID`/`modeID`.
#[test]
fn explicit_variable_modes_pin_a_frame_to_a_nondefault_mode() {
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("VARIABLE_SET".into())),
                ("name", KiwiValue::String("Theme".to_owned())),
                (
                    "variableSetModes",
                    KiwiValue::Array(vec![
                        var_set_mode(0, 10, "Light"),
                        var_set_mode(0, 11, "Dark"),
                    ]),
                ),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 5)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("Card".to_owned())),
                ("size", vector(100.0, 100.0)),
                (
                    "explicitVariableModes",
                    KiwiValue::Array(vec![o(
                        "VariableModeBySet",
                        vec![("variableSetID", guid(0, 1)), ("modeID", guid(0, 11))],
                    )]),
                ),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.explicit_modes_imported, 1);

    let coll = doc.variables.collections.values().next().unwrap();
    assert_eq!(coll.modes[1].name, "Dark");
    let dark_mode = coll.modes[1].id;

    let frame = group_named(&doc, "Card");
    assert_eq!(frame.explicit_modes.get(&coll.id), Some(&dark_mode));
    doc.scene.validate().unwrap();
}

/// The reader is lenient about the GUID shape: a `variableSetID` wrapped in a
/// `{ guid: GUID }` holder (the form a VARIABLE node's own `variableSetID` uses)
/// resolves the same as a bare GUID. Guards the dual-shape decode.
#[test]
fn explicit_variable_modes_accept_wrapped_guid_shape() {
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("VARIABLE_SET".into())),
                ("name", KiwiValue::String("Theme".to_owned())),
                (
                    "variableSetModes",
                    KiwiValue::Array(vec![
                        var_set_mode(0, 10, "Light"),
                        var_set_mode(0, 11, "Dark"),
                    ]),
                ),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 5)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("Card".to_owned())),
                ("size", vector(100.0, 100.0)),
                (
                    "explicitVariableModes",
                    KiwiValue::Array(vec![o(
                        "VariableModeBySet",
                        // variableSetID wrapped as `{ guid: GUID }`.
                        vec![
                            ("variableSetID", variable_id(0, 1)),
                            ("modeID", guid(0, 10)),
                        ],
                    )]),
                ),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.explicit_modes_imported, 1);
    let coll = doc.variables.collections.values().next().unwrap();
    let light_mode = coll.modes[0].id;
    let frame = group_named(&doc, "Card");
    assert_eq!(frame.explicit_modes.get(&coll.id), Some(&light_mode));
}
