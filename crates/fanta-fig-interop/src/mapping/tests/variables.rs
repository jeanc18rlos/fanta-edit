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
                    KiwiValue::array(vec![
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
                            KiwiValue::array(vec![
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
                    KiwiValue::array(vec![var_set_mode(0, 10, "Base")]),
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
                            KiwiValue::array(vec![o(
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
                        KiwiValue::array(vec![o(
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
                KiwiValue::array(vec![bound_solid_paint(1.0, 0.0, 0.0, 1.0, 0, 99)]),
            ),
            (
                "strokePaints",
                KiwiValue::array(vec![bound_solid_paint(0.0, 0.0, 0.0, 1.0, 0, 100)]),
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
                    KiwiValue::array(vec![bound_solid_paint(1.0, 1.0, 1.0, 1.0, 0, 101)]),
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
                    KiwiValue::array(vec![bound_solid_paint(0.0, 0.0, 0.0, 1.0, 0, 102)]),
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
                    KiwiValue::array(vec![
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
                    KiwiValue::array(vec![o(
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
                    KiwiValue::array(vec![
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
                    KiwiValue::array(vec![o(
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

// =============================================================================
// Remote-library collection stubs
// =============================================================================

/// A VARIABLE_SET stub for a collection whose variables live in a subscribed
/// library: it carries a name and modes but no VARIABLE resolves into it. A real
/// file carries one per consumed library VERSION, which is why the same name
/// repeats.
fn remote_collection_stub(local_id: u32, name: &str) -> KiwiValue {
    o(
        "NodeChange",
        vec![
            ("guid", guid(0, local_id)),
            ("type", KiwiValue::Enum("VARIABLE_SET".into())),
            ("name", KiwiValue::String(name.to_owned())),
            (
                "variableSetModes",
                KiwiValue::array(vec![var_set_mode(0, 100 + local_id, "Mode 1")]),
            ),
        ],
    )
}

/// A FLOAT VARIABLE owned by the VARIABLE_SET at `0:set_local_id`.
fn float_variable(local_id: u32, set_local_id: u32, name: &str, value: f32) -> KiwiValue {
    o(
        "NodeChange",
        vec![
            ("guid", guid(0, local_id)),
            ("type", KiwiValue::Enum("VARIABLE".into())),
            ("name", KiwiValue::String(name.to_owned())),
            ("variableSetID", variable_id(0, set_local_id)),
            ("variableResolvedType", KiwiValue::Enum("FLOAT".into())),
            (
                "variableDataValues",
                o(
                    "VariableDataValues",
                    vec![(
                        "entries",
                        KiwiValue::array(vec![o(
                            "VariableDataValuesEntry",
                            vec![
                                ("modeID", guid(0, 100 + set_local_id)),
                                ("variableData", var_float_value(value)),
                            ],
                        )]),
                    )],
                ),
            ),
        ],
    )
}

/// A file that subscribes to shared libraries carries a VARIABLE_SET change for
/// every remote collection it consumes — repeated per library version, so the
/// same name appears many times — while the variables stay in the publishing
/// library. Minting a collection per VARIABLE_SET buried the file's real
/// collections under dozens of identically-named empty ones (Figma's UI kit
/// imported ~40 collections of which ~35 were empty). A stub that resolves no
/// variable and that nothing pins must not reach the document.
#[test]
fn remote_library_collection_stubs_do_not_import_as_empty_duplicates() {
    let fig = doc_from(vec![
        remote_collection_stub(1, "Colors"),
        remote_collection_stub(2, "Colors"),
        remote_collection_stub(3, "Sizing"),
        remote_collection_stub(4, "Typography"),
        // Only the first "Colors" set actually owns a variable in this file.
        float_variable(20, 1, "space/md", 16.0),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();

    assert_eq!(doc.variables.collections.len(), 1);
    assert_eq!(report.variable_collections, 1);
    assert_eq!(report.variable_collections_pruned, 3);
    let coll = doc.variables.collections.values().next().unwrap();
    assert_eq!(coll.name, "Colors");
    assert_eq!(coll.variable_order.len(), 1);
    // The surviving variable still resolves through its collection.
    let var_id = guid_to_variable_id("0:20");
    assert_eq!(
        doc.variables.collection_of(var_id).map(|c| c.id),
        Some(coll.id)
    );
}

/// Pruning keys off emptiness, never off the NAME: two collections that both
/// hold variables survive even when they are called the same thing (a file can
/// legitimately consume two different libraries' "Colors").
#[test]
fn distinct_collections_sharing_a_name_both_survive() {
    let fig = doc_from(vec![
        remote_collection_stub(1, "Colors"),
        remote_collection_stub(2, "Colors"),
        float_variable(20, 1, "a", 1.0),
        float_variable(21, 2, "b", 2.0),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();

    assert_eq!(doc.variables.collections.len(), 2);
    assert_eq!(report.variable_collections, 2);
    assert_eq!(report.variable_collections_pruned, 0);
    for coll in doc.variables.collections.values() {
        assert_eq!(coll.name, "Colors");
        assert_eq!(coll.variable_order.len(), 1);
    }
}

/// A collection with no variables of its own that a frame PINS a mode of must
/// survive: the pin names it by id, and dropping it would leave the frame's
/// `explicit_modes` entry pointing at nothing.
#[test]
fn empty_collection_referenced_by_a_mode_pin_survives() {
    let fig = doc_from(vec![
        remote_collection_stub(1, "Theme"),
        // Same name, same emptiness, but nothing points at this one.
        remote_collection_stub(2, "Theme"),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 5)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("Card".to_owned())),
                ("size", vector(100.0, 100.0)),
                (
                    "explicitVariableModes",
                    KiwiValue::array(vec![o(
                        "VariableModeBySet",
                        vec![("variableSetID", guid(0, 1)), ("modeID", guid(0, 101))],
                    )]),
                ),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();

    assert_eq!(report.explicit_modes_imported, 1);
    assert_eq!(doc.variables.collections.len(), 1);
    assert_eq!(report.variable_collections, 1);
    assert_eq!(report.variable_collections_pruned, 1);

    let coll = doc.variables.collections.values().next().unwrap();
    assert!(coll.variable_order.is_empty());
    let frame = group_named(&doc, "Card");
    assert_eq!(frame.explicit_modes.get(&coll.id), Some(&coll.modes[0].id));
}

/// Opt-in: every collection a real export imports must earn its place — hold a
/// variable, or be pinned by a frame. Guards the UI-kit defect (≈35 empty
/// duplicate collections) against any file. Run with:
///   FANTA_FIG_FIXTURE=/path/to/file.fig \
///     cargo test -p fanta-fig-interop empty_collections -- --ignored --nocapture
#[test]
#[ignore = "requires FANTA_FIG_FIXTURE pointing at a real .fig"]
fn imports_real_fig_fixture_without_empty_collections() {
    let Ok(path) = std::env::var("FANTA_FIG_FIXTURE") else {
        eprintln!("FANTA_FIG_FIXTURE not set; skipping");
        return;
    };
    let bytes = std::fs::read(&path).expect("read fixture");
    let fig = crate::fig::read_fig(&bytes).expect("real .fig must parse");
    let (doc, report, _assets) = fig_to_doc(&fig).expect("map to doc");

    let mut pinned: std::collections::HashSet<fanta_doc::VariableCollectionId> =
        std::collections::HashSet::new();
    for root in doc.scene.roots() {
        for id in std::iter::once(*root).chain(doc.scene.descendants_of(*root)) {
            if let Some(node) = doc.scene.get(id)
                && let NodeData::Group(group) = &node.data
            {
                pinned.extend(group.explicit_modes.keys().copied());
            }
        }
    }
    eprintln!(
        "  VARIABLE COLLECTIONS: {} kept, {} pruned as empty+unreferenced, \
             {} variables, {} frames pin a mode",
        report.variable_collections,
        report.variable_collections_pruned,
        report.variables,
        pinned.len(),
    );
    for coll in doc.variables.collections.values() {
        assert!(
            !coll.variable_order.is_empty() || pinned.contains(&coll.id),
            "collection {:?} ({}) is empty and unreferenced",
            coll.name,
            coll.id
        );
    }
    assert_eq!(report.variable_collections, doc.variables.collections.len());
    for variable in doc.variables.variables.values() {
        let collection = &doc.variables.collections[&variable.collection];
        assert!(
            variable
                .values_by_mode
                .keys()
                .all(|mode| collection.has_mode(*mode)),
            "{} has a value mapped to another collection's mode",
            variable.name
        );
    }
}

#[test]
fn reused_library_mode_guids_stay_scoped_to_their_collection() {
    let mut doc = Doc::new();
    let mut report = MapReport::default();
    let collections = ["local", "library"].map(|name| PendingCollection {
        guid: name.into(),
        name: name.into(),
        modes: vec![
            ("shared-light".into(), "Light".into()),
            ("shared-dark".into(), "Dark".into()),
        ],
    });
    let variables = ["local", "library"].map(|name| PendingVariable {
        guid: format!("{name}-color"),
        name: "yellow".into(),
        set_guid: Some(name.into()),
        ty: VariableType::Color,
        values: vec![
            (
                "shared-light".into(),
                VarValue::Color {
                    value: Color::rgba(255, 251, 235, 255),
                },
            ),
            (
                "shared-dark".into(),
                VarValue::Color {
                    value: Color::rgba(253, 247, 221, 255),
                },
            ),
        ],
    });
    let maps = build_variables(&mut doc, &mut report, &collections, &variables);
    let frame = CanvasNode::new(NodeData::Group(GroupNode::default()));
    let frame_id = frame.id;
    doc.scene.insert(frame).expect("insert frame");
    apply_explicit_modes(
        &mut doc,
        &mut report,
        &[(frame_id, vec![("local".into(), "shared-dark".into())])],
        &maps,
    );
    for variable in doc.variables.variables.values() {
        let collection = &doc.variables.collections[&variable.collection];
        assert!(
            variable
                .values_by_mode
                .keys()
                .all(|mode| collection.has_mode(*mode))
        );
        assert_eq!(
            variable.values_by_mode.get(&collection.default_mode),
            Some(&VarValue::Color {
                value: Color::rgba(255, 251, 235, 255)
            })
        );
    }
    let local = maps.coll_guid_to_id["local"];
    let frame = doc.scene.get(frame_id).expect("frame exists");
    let NodeData::Group(group) = &frame.data else {
        panic!("frame must be a group")
    };
    let mode = group.explicit_modes[&local];
    assert!(doc.variables.collections[&local].has_mode(mode));
    assert_eq!(
        doc.variables.collections[&local]
            .modes
            .iter()
            .find(|m| m.id == mode)
            .expect("owned mode")
            .name,
        "Dark"
    );
}
