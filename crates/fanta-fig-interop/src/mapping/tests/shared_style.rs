//! Shared-style references, backgroundPaints fallback, visibility.

use super::*;
use std::borrow::Cow;

// =============================================================================
// Shared-style references + backgroundPaints fallback + visibility (this work)
// =============================================================================

/// A `styleIdFor*` ref object: `{ guid: GUID }`.
fn style_ref(sid: u32, lid: u32) -> KiwiValue {
    o("StyleId", vec![("guid", guid(sid, lid))])
}

/// A standalone shared-style definition NodeChange (`styleType` set) carrying a
/// single solid `fillPaints` paint, keyed by its own `guid`.
fn fill_style_def(sid: u32, lid: u32, r: f32, g: f32, b: f32) -> KiwiValue {
    o(
        "NodeChange",
        vec![
            ("guid", guid(sid, lid)),
            ("type", KiwiValue::Enum("VECTOR".into())),
            ("styleType", KiwiValue::Enum("FILL".into())),
            ("name", KiwiValue::String("dark/bg".to_owned())),
            (
                "fillPaints",
                KiwiValue::Array(vec![solid_paint(r, g, b, 1.0)]),
            ),
        ],
    )
}

fn text_style_def(sid: u32, lid: u32, r: f32, g: f32, b: f32) -> KiwiValue {
    o(
        "NodeChange",
        vec![
            ("guid", guid(sid, lid)),
            ("type", KiwiValue::Enum("TEXT".into())),
            ("styleType", KiwiValue::Enum("TEXT".into())),
            ("name", KiwiValue::String("heading".to_owned())),
            ("fontSize", KiwiValue::Float(18.0)),
            (
                "fillPaints",
                KiwiValue::Array(vec![solid_paint(r, g, b, 1.0)]),
            ),
        ],
    )
}

#[test]
fn style_pre_pass_copies_only_the_changes_it_rewrites() {
    // The pre-pass hands back a copy-on-write view: a change with no
    // `styleIdFor*` ref anywhere the resolvers look stays borrowed from the
    // source, and only the consumers (including those whose ref sits on a
    // symbolOverrides entry or a rich-text run entry) are cloned + rewritten.
    let dark = solid_paint(26.0 / 255.0, 26.0 / 255.0, 26.0 / 255.0, 1.0);
    let changes = vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
        fill_style_def(10, 5, 26.0 / 255.0, 26.0 / 255.0, 26.0 / 255.0),
        // A direct consumer.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("type", KiwiValue::Enum("RECTANGLE".into())),
                ("fillPaints", KiwiValue::Array(vec![])),
                ("styleIdForFill", style_ref(10, 5)),
            ],
        ),
        // No ref anywhere: must come back untouched and unowned.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("type", KiwiValue::Enum("RECTANGLE".into())),
                (
                    "fillPaints",
                    KiwiValue::Array(vec![solid_paint(1.0, 1.0, 1.0, 1.0)]),
                ),
            ],
        ),
        // The ref lives on a symbolOverrides entry only.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 4)),
                ("type", KiwiValue::Enum("INSTANCE".into())),
                (
                    "symbolData",
                    o(
                        "SymbolData",
                        vec![
                            ("symbolID", guid(0, 20)),
                            (
                                "symbolOverrides",
                                KiwiValue::Array(vec![o(
                                    "NodeChange",
                                    vec![
                                        ("guidPath", guid_path(0, 21)),
                                        ("styleIdForFill", style_ref(10, 5)),
                                    ],
                                )]),
                            ),
                        ],
                    ),
                ),
            ],
        ),
        // The ref lives on a rich-text run entry only.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 5)),
                ("type", KiwiValue::Enum("TEXT".into())),
                (
                    "textData",
                    o(
                        "TextData",
                        vec![
                            ("characters", KiwiValue::String("ab".to_owned())),
                            (
                                "styleOverrideTable",
                                KiwiValue::Array(vec![o(
                                    "NodeChange",
                                    vec![
                                        ("styleID", KiwiValue::Uint(1)),
                                        ("styleIdForFill", style_ref(10, 5)),
                                    ],
                                )]),
                            ),
                        ],
                    ),
                ),
            ],
        ),
        // A ref to a guid that is NOT a style def: still counted (and copied),
        // exactly as the resolvers behaved on an owned slice.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 6)),
                ("type", KiwiValue::Enum("RECTANGLE".into())),
                ("styleIdForFill", style_ref(99, 99)),
            ],
        ),
    ];

    let (resolved, report) = resolve_style_references(&changes);
    assert_eq!(resolved.len(), changes.len());
    let owned: Vec<bool> = resolved
        .iter()
        .map(|change| matches!(change, Cow::Owned(_)))
        .collect();
    assert_eq!(
        owned,
        vec![false, false, true, false, true, true, true],
        "only ref-carrying changes are copied"
    );
    for (resolved, original) in resolved.iter().zip(&changes) {
        if let Cow::Borrowed(borrowed) = resolved {
            assert!(std::ptr::eq(*borrowed, original));
        }
    }
    // The direct consumer got the style's paints; the untouched rect is equal.
    assert_eq!(
        resolved[2].get("fillPaints"),
        Some(&KiwiValue::Array(vec![dark.clone()]))
    );
    assert_eq!(*resolved[3], changes[3]);
    // The override entry and the run entry were rewritten in place.
    let override_entry = &resolved[4]
        .get("symbolData")
        .and_then(|sd| sd.get("symbolOverrides"))
        .and_then(KiwiValue::as_array)
        .unwrap()[0];
    assert_eq!(
        override_entry.get("fillPaints"),
        Some(&KiwiValue::Array(vec![dark.clone()]))
    );
    let run_entry = &resolved[5]
        .get("textData")
        .and_then(|td| td.get("styleOverrideTable"))
        .and_then(KiwiValue::as_array)
        .unwrap()[0];
    assert_eq!(
        run_entry.get("fillPaints"),
        Some(&KiwiValue::Array(vec![dark]))
    );
    // The unresolvable ref is counted as an empty-fill ref, never resolved.
    assert!(resolved[6].get("fillPaints").is_none());
    assert_eq!(report.style_def_count, 1);
    assert_eq!(
        report.ref_empty_fill, 4,
        "direct consumer + override entry + run entry + dangling ref"
    );
    assert_eq!(report.resolved_fill, 3);
}

#[test]
fn style_pre_pass_without_style_defs_borrows_everything() {
    let changes = vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("type", KiwiValue::Enum("RECTANGLE".into())),
                ("styleIdForFill", style_ref(10, 5)),
            ],
        ),
    ];
    let (resolved, report) = resolve_style_references(&changes);
    assert!(
        resolved
            .iter()
            .all(|change| matches!(change, Cow::Borrowed(_))),
        "no style defs ⇒ nothing to inline ⇒ no copies"
    );
    assert_eq!(report.style_def_count, 0);
    assert_eq!(report.ref_empty_fill, 0);
}

#[test]
fn empty_fillpaints_with_style_ref_resolves_to_the_styles_paints() {
    // A rect with an EMPTY fillPaints + a styleIdForFill must inherit the
    // referenced FILL style's paint (the core shared-style unlock). The style is
    // a near-black dark-theme color.
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".into())),
            ],
        ),
        // The shared FILL style def: dark grey #1A1A1A.
        fill_style_def(10, 5, 26.0 / 255.0, 26.0 / 255.0, 26.0 / 255.0),
        // The consumer: empty fillPaints + a ref to style 10:5.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("RECTANGLE".into())),
                ("size", vector(10.0, 10.0)),
                ("fillPaints", KiwiValue::Array(vec![])),
                ("styleIdForFill", style_ref(10, 5)),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    let v = first_vector(&doc);
    assert_eq!(
        v.fills.first(),
        Some(&Fill::solid(Color::rgba(26, 26, 26, 255))),
        "empty-fill node inherits the referenced style's paint"
    );
    assert_eq!(report.style_def_count, 1, "one style def counted");
    assert!(
        report.style_ref_resolved_fill >= 1,
        "at least one ref resolved"
    );
}

#[test]
fn nonempty_fillpaints_with_style_ref_takes_the_styles_paint() {
    // THE DARKEST `_Header` FIX (regression guard). A node can carry BOTH its own
    // non-empty inline `fillPaints` (here white — the light master default) AND a
    // `styleIdForFill` pointing at a per-page FILL style (here dark #1A1A1A). op2
    // (`resolveStyleReferences`) overwrites the inline paint with the style's
    // UNCONDITIONALLY — the style is the authoritative per-page color. The old
    // behavior resolved the style only when the inline fill was empty, leaving the
    // dark-theme surface painted white. After the fix the style wins.
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".into())),
            ],
        ),
        // The shared FILL style def: dark grey #1A1A1A.
        fill_style_def(10, 5, 26.0 / 255.0, 26.0 / 255.0, 26.0 / 255.0),
        // The consumer: its OWN white inline fill + a ref to the dark style 10:5.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("RECTANGLE".into())),
                ("size", vector(10.0, 10.0)),
                (
                    "fillPaints",
                    KiwiValue::Array(vec![solid_paint(1.0, 1.0, 1.0, 1.0)]),
                ),
                ("styleIdForFill", style_ref(10, 5)),
            ],
        ),
    ]);
    let (doc, _report, _) = fig_to_doc(&fig).unwrap();
    let v = first_vector(&doc);
    assert_eq!(
        v.fills.first(),
        Some(&Fill::solid(Color::rgba(26, 26, 26, 255))),
        "the referenced FILL style wins over the node's own inline (white) paint"
    );
}

#[test]
fn solid_paint_with_a_variable_binding_resolves_to_its_baked_color() {
    // THE `Color Background` SWATCH SHAPE (regression guard). In the real Spectrum
    // file every status-color swatch is a `ROUNDED_RECTANGLE` named "Color
    // Background" whose `fillPaints[0]` is a SOLID paint carrying BOTH a baked
    // `color` (the materialized brand color, e.g. #004087) AND a Figma VARIABLE
    // binding (a Spectrum color token) annotated on the paint. We import zero
    // variables (OpenPencil imports none either); the correct behavior — matching
    // OpenPencil — is to read the baked solid `color` straight off the paint and
    // never drop it just because a variable is also bound. The swatch ALSO carries
    // a `styleIdForFill` whose FILL style holds the same baked color, so the result
    // is identical whether the style pre-pass overwrites or not.
    //
    // We model a SOLID paint with a `colorVar` variable annotation alongside its
    // baked color, plus a matching FILL style ref, and assert the resolved fill is
    // the baked color — not `<none>`.
    let mut bound_solid = solid_paint(0.0, 64.0 / 255.0, 135.0 / 255.0, 1.0); // #004087
    if let KiwiValue::Object { fields, .. } = &mut bound_solid {
        // A variable annotation on the paint (the field name doesn't matter — we
        // never read it; what matters is its presence must not suppress the baked
        // `color`). `colorVar` carries an alias VariableData pointing at a token.
        fields.insert(
            "colorVar".into(),
            o(
                "VariableData",
                vec![(
                    "value",
                    o("VariableAnyValue", vec![("alias", variable_id(99, 1))]),
                )],
            ),
        );
    }
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".into())),
            ],
        ),
        // The FILL style def carries the SAME baked color (the per-page token value).
        fill_style_def(10, 7, 0.0, 64.0 / 255.0, 135.0 / 255.0),
        // The swatch: a ROUNDED_RECTANGLE with a variable-bound baked solid + a
        // styleIdForFill ref, exactly like the real "Color Background" rect.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("ROUNDED_RECTANGLE".into())),
                ("name", KiwiValue::String("Color Background".to_owned())),
                ("size", vector(330.0, 283.0)),
                ("fillPaints", KiwiValue::Array(vec![bound_solid])),
                ("styleIdForFill", style_ref(10, 7)),
            ],
        ),
    ]);
    let (doc, _report, _) = fig_to_doc(&fig).unwrap();
    let v = first_vector(&doc);
    assert_eq!(
        v.fills.first(),
        Some(&Fill::solid(Color::rgba(0, 64, 135, 255))),
        "a SOLID paint with a variable binding resolves to its baked #004087, not <none>"
    );
}

#[test]
fn instance_own_surface_fill_overrides_master_root_background() {
    // THE `mergeSymbolProps` HALF of the Darkest `_Header` fix. A card `_Header`
    // is an INSTANCE: its surface is painted by the expanded MASTER ROOT's
    // background, NOT its own inline `fillPaints`. So overwriting the instance's
    // own fill is not enough — the instance's own (style-resolved, dark) surface
    // must be routed onto the expanded master root. We model a tiny case: a
    // SYMBOL master frame with a WHITE background, and an INSTANCE of it whose own
    // `fillPaints` is dark #1A1A1A. The expanded instance must paint dark.
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".into())),
            ],
        ),
        // SYMBOL master: a frame with a WHITE background (the light master).
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 10)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("Card".to_owned())),
                ("size", vector(100.0, 40.0)),
                (
                    "fillPaints",
                    KiwiValue::Array(vec![solid_paint(1.0, 1.0, 1.0, 1.0)]),
                ),
            ],
        ),
        // INSTANCE of the master, carrying its OWN dark surface fill #1A1A1A.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 20)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("INSTANCE".into())),
                ("name", KiwiValue::String("Card".to_owned())),
                ("size", vector(100.0, 40.0)),
                (
                    "fillPaints",
                    KiwiValue::Array(vec![solid_paint(
                        26.0 / 255.0,
                        26.0 / 255.0,
                        26.0 / 255.0,
                        1.0,
                    )]),
                ),
                (
                    "symbolData",
                    o("SymbolData", vec![("symbolID", guid(0, 10))]),
                ),
            ],
        ),
    ]);
    let (doc, _report, _) = fig_to_doc(&fig).unwrap();
    // Find the instance node and expand it; the expansion root must be dark.
    let expanded = expand_only_instance(&doc);
    let root = expanded
        .iter()
        .find(|e| e.def_path.is_empty())
        .expect("expansion has a root");
    match &root.node.data {
        NodeData::Group(g) => assert_eq!(
            g.background,
            Some(Fill::solid(Color::rgba(26, 26, 26, 255))),
            "the instance's own dark surface overrides the white master-root background"
        ),
        other => panic!("expected the expanded root to be a group, got {other:?}"),
    }
}

#[test]
fn instance_own_surface_fill_equal_to_master_records_no_override() {
    // The flip side of the `_Header` fix: when the instance's own surface fill
    // is the SAME as the master's (a Figma snapshot no-op, e.g. a plain white
    // card over a white master), the importer must NOT bake a redundant fill
    // override. Otherwise editing the master's fill would never reach the
    // instance — the pin would keep re-applying the old value.
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".into())),
            ],
        ),
        // SYMBOL master: a frame with a WHITE background.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 10)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("Card".to_owned())),
                ("size", vector(100.0, 40.0)),
                (
                    "fillPaints",
                    KiwiValue::Array(vec![solid_paint(1.0, 1.0, 1.0, 1.0)]),
                ),
            ],
        ),
        // INSTANCE whose own surface fill is ALSO white — equal to the master.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 20)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("INSTANCE".into())),
                ("name", KiwiValue::String("Card".to_owned())),
                ("size", vector(100.0, 40.0)),
                (
                    "fillPaints",
                    KiwiValue::Array(vec![solid_paint(1.0, 1.0, 1.0, 1.0)]),
                ),
                (
                    "symbolData",
                    o("SymbolData", vec![("symbolID", guid(0, 10))]),
                ),
            ],
        ),
    ]);
    let (doc, _report, _) = fig_to_doc(&fig).unwrap();
    let instance = doc
        .scene
        .roots()
        .iter()
        .flat_map(|root| doc.scene.descendants_of(*root).collect::<Vec<_>>())
        .find_map(|id| match doc.scene.get(id).map(|node| &node.data) {
            Some(NodeData::Instance(inst)) => Some(inst.clone()),
            _ => None,
        })
        .expect("an instance node");
    assert!(
        !instance
            .overrides
            .iter()
            .any(|ov| matches!(ov.value, fanta_doc::OverrideValue::Fills { .. })),
        "a surface fill equal to the master must not be baked as an override, \
         got: {:?}",
        instance.overrides,
    );
}

#[test]
fn frame_with_only_background_paints_uses_them_as_background() {
    // A FRAME with NO fillPaints but a `backgroundPaints` must paint that as its
    // group background (op2's `mapFigmaFills(fillPaints) ?? backgroundPaints`).
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("size", vector(100.0, 100.0)),
                // No fillPaints; only backgroundPaints (a dark navy).
                (
                    "backgroundPaints",
                    KiwiValue::Array(vec![solid_paint(0.1, 0.1, 0.2, 1.0)]),
                ),
            ],
        ),
    ]);
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    let frame_id = doc.scene.children_of(Some(doc.scene.roots()[0]))[0];
    match &doc.scene.get(frame_id).unwrap().data {
        NodeData::Group(g) => assert_eq!(
            g.background,
            Some(Fill::solid(Color::rgba(26, 26, 51, 255))),
            "frame background comes from backgroundPaints fallback"
        ),
        other => panic!("expected group, got {other:?}"),
    }
}

#[test]
fn invisible_node_is_marked_hidden() {
    // A `visible:false` node must carry NodeFlags::HIDDEN so the renderer skips it
    // (kills hidden light-master layers leaking behind dark ones).
    let rect = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("RECTANGLE".into())),
            ("size", vector(10.0, 10.0)),
            (
                "fillPaints",
                KiwiValue::Array(vec![solid_paint(1.0, 1.0, 1.0, 1.0)]),
            ),
            ("visible", KiwiValue::Bool(false)),
        ],
    );
    let (doc, _, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    let node = first_node_with_vector(&doc);
    assert!(
        node.flags.contains(NodeFlags::HIDDEN),
        "visible:false node must be HIDDEN"
    );
}

#[test]
fn zero_opacity_node_is_marked_hidden() {
    let rect = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("RECTANGLE".into())),
            ("size", vector(10.0, 10.0)),
            (
                "fillPaints",
                KiwiValue::Array(vec![solid_paint(1.0, 1.0, 1.0, 1.0)]),
            ),
            ("opacity", KiwiValue::Float(0.0)),
        ],
    );
    let (doc, _, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    let node = first_node_with_vector(&doc);
    assert!(
        node.flags.contains(NodeFlags::HIDDEN),
        "opacity<=0 node must be HIDDEN"
    );
}

#[test]
fn frame_fillpaints_set_group_background_through_full_import() {
    // End-to-end Phase-1b sanity: a FRAME with explicit fillPaints paints its
    // group background (a frame is a Group with `background`, not a vec of fills).
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("size", vector(100.0, 100.0)),
                (
                    "fillPaints",
                    KiwiValue::Array(vec![solid_paint(0.0, 0.0, 0.0, 1.0)]),
                ),
            ],
        ),
    ]);
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    let frame_id = doc.scene.children_of(Some(doc.scene.roots()[0]))[0];
    match &doc.scene.get(frame_id).unwrap().data {
        NodeData::Group(g) => assert_eq!(
            g.background,
            Some(Fill::solid(Color::rgba(0, 0, 0, 255))),
            "frame fillPaints land in group background"
        ),
        other => panic!("expected group, got {other:?}"),
    }
}

#[test]
fn symbol_override_style_ref_resolves_to_dark_fill_on_expand() {
    // Phase 2c: a dark-theme INSTANCE whose symbolOverride points the master
    // RECTANGLE at a dark FILL style (the override carries EMPTY fillPaints + a
    // styleIdForFill). The pre-pass must inline the style's dark paint into the
    // override entry, so on expand the child renders dark, not the white master.
    let fig = doc_from(vec![
        // Dark FILL style def #101820, guid 9:42.
        fill_style_def(9, 42, 16.0 / 255.0, 24.0 / 255.0, 32.0 / 255.0),
        // SYMBOL master with a white RECTANGLE child.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("Badge".to_owned())),
                ("size", vector(80.0, 24.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("RECTANGLE".into())),
                ("name", KiwiValue::String("bg".to_owned())),
                ("size", vector(80.0, 24.0)),
                (
                    "fillPaints",
                    KiwiValue::Array(vec![solid_paint(1.0, 1.0, 1.0, 1.0)]),
                ),
            ],
        ),
        // INSTANCE: symbolOverride on the rect = empty fillPaints + dark style ref.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("type", KiwiValue::Enum("INSTANCE".into())),
                ("name", KiwiValue::String("Badge / dark".to_owned())),
                ("size", vector(80.0, 24.0)),
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
                                        ("fillPaints", KiwiValue::Array(vec![])),
                                        ("styleIdForFill", style_ref(9, 42)),
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
        "the instance carries a fill override"
    );
    assert!(report.overrides_applied >= 1);

    let expanded = expand_only_instance(&doc);
    let rect_fill = expanded.iter().find_map(|e| match &e.node.data {
        NodeData::Vector(v) => v.fills.first().cloned(),
        _ => None,
    });
    assert_eq!(
        rect_fill,
        Some(Fill::solid(Color::rgba(16, 24, 32, 255))),
        "dark style ref resolves through the symbolOverride to the master child"
    );
}

#[test]
fn symbol_override_text_style_ref_does_not_synthesize_fill_override() {
    let fig = doc_from(vec![
        text_style_def(9, 77, 0.0, 0.0, 0.0),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("Breadcrumb title".to_owned())),
                ("size", vector(180.0, 32.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("TEXT".into())),
                ("name", KiwiValue::String("Label".to_owned())),
                ("size", vector(120.0, 20.0)),
                ("textData", text_data("Master label")),
                ("fontSize", KiwiValue::Float(16.0)),
                (
                    "fillPaints",
                    KiwiValue::Array(vec![solid_paint(1.0, 1.0, 1.0, 1.0)]),
                ),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("type", KiwiValue::Enum("INSTANCE".into())),
                (
                    "name",
                    KiwiValue::String("Breadcrumb title / dark".to_owned()),
                ),
                ("size", vector(180.0, 32.0)),
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
                                        ("styleIdForText", style_ref(9, 77)),
                                        ("textData", text_data("Avatar: The Way of Water")),
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
        .find_map(|entry| match &entry.node.data {
            NodeData::Text(text) => Some(text),
            _ => None,
        })
        .expect("expanded instance contains text");

    assert_eq!(text.content, "Avatar: The Way of Water");
    assert_eq!(
        text.style.color,
        Color::rgba(255, 255, 255, 255),
        "a text style ref inside a symbolOverride must not turn into a fill override"
    );
}
