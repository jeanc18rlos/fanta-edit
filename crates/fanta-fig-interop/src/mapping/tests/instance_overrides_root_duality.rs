use super::*;

// =============================================================================
// Main-vs-published-component duality: a `guidPath` is rooted at the SYMBOL the
// instance references, so a length-1 path whose single guid IS that master root
// addresses the EXPANSION ROOT (def-local path `[]`), and a content override that
// descends through a SWAPPED nested instance addresses the SWAPPED master's
// descendants — not the declared symbolID's. Both used to resolve to nothing
// (`build_master_guid_paths` excludes the root; the descent entered the wrong
// master), silently dropping ~14.5k root + ~1.85k swapped-nested entries in the
// Spectrum fixture.
// =============================================================================

#[test]
fn root_targeted_guidpath_repaints_the_expansion_root_surface() {
    // The MAIN-vs-PUBLISHED ROOT case. A "Card" SYMBOL master is a white-filled
    // frame; its INSTANCE carries a symbolOverride whose `guidPath` is the SINGLE
    // master-root guid (0:1 — exactly the symbolID the instance points at),
    // re-coloring the surface to dark `#1d1d1d`. The override addresses the
    // expansion ROOT, so after expansion the root clone's background must be the
    // override's dark fill, not the master's white. Before the root-case fix the
    // path resolved to `None` (root excluded from the descendant map) and the
    // override was dropped.
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("Card".to_owned())),
                ("size", vector(120.0, 40.0)),
                // The master's own (light) surface fill — makes the root a Group
                // with a background, the surface the override recolors.
                (
                    "fillPaints",
                    KiwiValue::array(vec![solid_paint(1.0, 1.0, 1.0, 1.0)]),
                ),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("type", KiwiValue::Enum("INSTANCE".into())),
                ("name", KiwiValue::String("Card instance".to_owned())),
                ("size", vector(120.0, 40.0)),
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
                                    // guidPath == [master root 0:1] → the root.
                                    vec![
                                        ("guidPath", guid_path(0, 1)),
                                        (
                                            "fillPaints",
                                            KiwiValue::array(vec![solid_paint(
                                                0.114, 0.114, 0.114, 1.0,
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
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(
        report.instances_with_overrides, 1,
        "the root-targeted fill override resolved + attached"
    );

    let expanded = expand_only_instance(&doc);
    let root = expanded
        .iter()
        .find(|e| e.def_path.is_empty())
        .expect("expansion root present");
    let bg = match &root.node.data {
        NodeData::Group(g) => g.background.clone(),
        other => panic!("expansion root should be a Group, got {other:?}"),
    };
    match bg {
        Some(Fill::Solid { color, .. }) => {
            // ~#1d1d1d (0.114 * 255 ≈ 29).
            assert!(
                (color.r as i16 - 29).abs() <= 2 && color.r == color.g && color.g == color.b,
                "root surface must be the override's dark fill, got {color:?}"
            );
        }
        other => panic!("root must carry the override's solid surface fill, got {other:?}"),
    }
}

#[test]
fn swapped_nested_instance_routes_content_override_into_the_swapped_master() {
    // The NESTED main-vs-published duality. A "Card" master holds a nested
    // INSTANCE (0:11) of "ChipA" (its DECLARED symbolID). The Card INSTANCE
    // carries TWO symbolOverrides:
    //   1. a swap at the nested instance (guidPath [0:11], overriddenSymbolID →
    //      "ChipB"), re-pointing it to ChipB;
    //   2. a CONTENT override whose guidPath [0:11, 0:22] descends through the
    //      nested instance to ChipB's OWN text child (0:22) — which lives under
    //      ChipB, NOT under the declared ChipA. Without the swap-redirect the
    //      descent entered ChipA, failed to find 0:22, and dropped the override.
    // After import + expansion, the nested instance is ChipB and its inner text
    // reads "Swapped".
    let fig = doc_from(vec![
        // ChipA master (declared symbolID) with its own text (0:2).
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("ChipA".to_owned())),
                ("size", vector(60.0, 20.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("TEXT".into())),
                ("name", KiwiValue::String("InnerA".to_owned())),
                ("size", vector(60.0, 16.0)),
                ("textData", text_data("A")),
                ("fontSize", KiwiValue::Float(12.0)),
            ],
        ),
        // ChipB master (swap target) with its OWN distinct text (0:22).
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 21)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("ChipB".to_owned())),
                ("size", vector(60.0, 20.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 22)),
                ("parentIndex", parent_index(0, 21)),
                ("type", KiwiValue::Enum("TEXT".into())),
                ("name", KiwiValue::String("InnerB".to_owned())),
                ("size", vector(60.0, 16.0)),
                ("textData", text_data("B")),
                ("fontSize", KiwiValue::Float(12.0)),
            ],
        ),
        // Card master holding a nested INSTANCE of ChipA.
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
        // Top-level Card INSTANCE: swap the nested chip to ChipB, then drive
        // ChipB's inner text via a guidPath that descends through the swap.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 30)),
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
                                KiwiValue::array(vec![
                                    // 1. swap the nested instance to ChipB.
                                    o(
                                        "NodeChange",
                                        vec![
                                            ("guidPath", guid_path(0, 11)),
                                            ("overriddenSymbolID", guid(0, 21)),
                                        ],
                                    ),
                                    // 2. content override at ChipB's text (lives
                                    // under ChipB, not the declared ChipA).
                                    o(
                                        "NodeChange",
                                        vec![
                                            ("guidPath", guid_path2((0, 11), (0, 22))),
                                            ("textData", text_data("Swapped")),
                                        ],
                                    ),
                                ]),
                            ),
                        ],
                    ),
                ),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.overridden_symbol_resolved, 1, "the swap resolved");
    assert_eq!(
        report.override_nested_resolved, 1,
        "the swap-redirected content override resolved its full cross-master path"
    );

    let chipb_id = doc
        .components
        .defs
        .iter()
        .find(|(_, d)| d.name == "ChipB")
        .map(|(id, _)| *id)
        .expect("ChipB component exists");

    // Expand the Card instance, then the (now-ChipB) nested instance.
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
    let nested = outer
        .iter()
        .find_map(|e| match &e.node.data {
            NodeData::Instance(i) => Some(i.clone()),
            _ => None,
        })
        .expect("nested chip instance in the expansion");
    assert_eq!(
        nested.component, chipb_id,
        "the nested instance was swapped to ChipB"
    );
    let inner = expand_instance(&doc.scene, &doc.components, &nested);
    let text = inner.iter().find_map(|e| match &e.node.data {
        NodeData::Text(t) => Some(t.content.clone()),
        _ => None,
    });
    assert_eq!(
        text.as_deref(),
        Some("Swapped"),
        "the content override routed through the SWAPPED master's text, proving \
         the descent entered ChipB (not the declared ChipA)"
    );
}
