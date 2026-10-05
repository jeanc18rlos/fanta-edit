//! VECTOR geometry: bbox fallback + real path decode from command blobs.

use super::*;

fn import_loss_node(local: u32, kind: &str, parent: Option<u32>) -> KiwiValue {
    let mut node = o(
        "NodeChange",
        vec![
            ("guid", guid(0, local)),
            ("type", KiwiValue::Enum(kind.into())),
            ("size", vector(24.0, 18.0)),
        ],
    );
    if let Some(parent) = parent {
        node.set_field("parentIndex", parent_index(0, parent));
    }
    node
}

#[test]
fn boolean_import_preserves_operations_operands_and_baked_geometry() {
    for (operation, expected) in [
        ("UNION", fanta_doc::BooleanOp::Union),
        ("SUBTRACT", fanta_doc::BooleanOp::Subtract),
        ("INTERSECT", fanta_doc::BooleanOp::Intersect),
        ("XOR", fanta_doc::BooleanOp::Exclude),
    ] {
        for descendants_first in [false, true] {
            let mut parent = import_loss_node(1, "BOOLEAN_OPERATION", None);
            parent.set_field("booleanOperation", KiwiValue::Enum(operation.into()));
            parent.set_field("fillGeometry", KiwiValue::array(vec![fig_path(0, "ODD")]));
            let mut triangle = Vec::new();
            triangle.extend(cmd(1, &[2.0, 2.0]));
            triangle.extend(cmd(2, &[20.0, 2.0]));
            triangle.extend(cmd(2, &[11.0, 16.0]));
            triangle.extend(cmd(0, &[]));
            let mut first = import_loss_node(2, "RECTANGLE", Some(1));
            first
                .get_mut("parentIndex")
                .expect("parent")
                .set_field("position", KiwiValue::String("b".into()));
            let mut second = import_loss_node(3, "ROUNDED_RECTANGLE", Some(1));
            second
                .get_mut("parentIndex")
                .expect("parent")
                .set_field("position", KiwiValue::String("a".into()));
            let mut nodes = vec![parent, first, second];
            if descendants_first {
                nodes.reverse();
            }
            let fig = doc_from_with_blobs(nodes, vec![triangle]);
            let (doc, report, _) = fig_to_doc(&fig).expect("import editable Boolean");
            assert_eq!(report.boolean_operations_flattened, 0, "{operation}");
            assert_eq!(report.boolean_operands_dropped, 0, "{operation}");
            assert_eq!(report.instance_children_dropped, 0, "{operation}");
            assert_eq!(report.non_container_children_dropped, 0, "{operation}");
            assert!(report.boolean_fallbacks_by_reason.is_empty());
            assert!(report.content_loss_summary().is_none());
            assert_eq!(report.mapped, 3);
            assert_eq!(doc.scene.len(), 3);
            let root = *doc.scene.roots().first().expect("Boolean parent");
            let node = doc.scene.get(root).expect("mapped parent");
            let NodeData::Boolean(boolean) = &node.data else {
                panic!("editable Boolean");
            };
            assert_eq!(boolean.op, expected);
            assert_eq!(node.meta["figma_id"], "0:1");
            let baked = boolean.baked.as_ref().expect("original baked geometry");
            assert_eq!(
                baked.vector.path.segments,
                vec![
                    PathSegment::Move { to: [2.0, 2.0] },
                    PathSegment::Line { to: [20.0, 2.0] },
                    PathSegment::Line { to: [11.0, 16.0] },
                    PathSegment::Close,
                ]
            );
            assert_eq!(baked.vector.path.fill_rule, FillRule::EvenOdd);
            assert_eq!(
                baked.source,
                fanta_doc::boolean_geometry_signature(&doc.scene, root).expect("valid geometry")
            );
            let ordered = doc
                .scene
                .children_of(Some(root))
                .iter()
                .map(|id| {
                    doc.scene.get(*id).expect("operand").meta["figma_id"]
                        .as_str()
                        .expect("source identity")
                })
                .collect::<Vec<_>>();
            assert_eq!(ordered, ["0:3", "0:2"]);
            assert_boolean_fnx_roundtrip(&doc, root);
        }
    }
}

fn assert_boolean_fnx_roundtrip(doc: &Doc, root: NodeId) {
    let nodes = doc
        .scene
        .descendants_of(root)
        .map(|id| serde_json::to_value(doc.scene.get(id).expect("node")).expect("node JSON"))
        .collect::<Vec<_>>();
    let (source, sidecar) =
        fanta_fnx::encode_subtree(&nodes, "BooleanFixture").expect("encode FNX");
    let decoded = fanta_fnx::decode_subtree(&source, &sidecar).expect("decode FNX");
    let mut scene = fanta_doc::Scene::new();
    scene
        .insert_many(
            decoded
                .into_iter()
                .map(|node| serde_json::from_value::<CanvasNode>(node).expect("typed node")),
        )
        .expect("restore scene");
    assert_eq!(scene.len(), doc.scene.len());
    for id in doc.scene.descendants_of(root) {
        assert_eq!(
            scene.get(id),
            doc.scene.get(id),
            "identity, payload and source order roundtrip"
        );
        assert_eq!(scene.children_of(Some(id)), doc.scene.children_of(Some(id)));
    }
    assert_eq!(
        fanta_doc::boolean_geometry_signature(&scene, root).expect("restored signature"),
        fanta_doc::boolean_geometry_signature(&doc.scene, root).expect("original signature")
    );
}

#[test]
fn boolean_import_preserves_nested_operands_without_baked_parent_geometry() {
    for descendants_first in [false, true] {
        let mut parent = import_loss_node(1, "BOOLEAN_OPERATION", None);
        parent.set_field("booleanOperation", KiwiValue::Enum("SUBTRACT".into()));
        let mut nodes = vec![
            parent,
            import_loss_node(2, "GROUP", Some(1)),
            import_loss_node(3, "RECTANGLE", Some(2)),
        ];
        if descendants_first {
            nodes.reverse();
        }
        let (doc, report, _) = fig_to_doc(&doc_from(nodes)).expect("import nested operand");
        assert_eq!(report.boolean_operations_flattened, 0);
        assert_eq!(report.boolean_operands_dropped, 0);
        assert_eq!(report.mapped, 3);
        assert!(report.content_loss_summary().is_none());
        let root = *doc.scene.roots().first().expect("root");
        assert!(
            matches!(&doc.scene.get(root).expect("Boolean").data, NodeData::Boolean(boolean) if boolean.baked.is_none())
        );
        assert_boolean_fnx_roundtrip(&doc, root);
    }
}

#[test]
fn boolean_import_unknown_or_missing_operation_reports_explicit_fallback() {
    for operation in [None, Some("FUTURE_OPERATION")] {
        for descendants_first in [false, true] {
            let mut parent = import_loss_node(1, "BOOLEAN_OPERATION", None);
            if let Some(operation) = operation {
                parent.set_field("booleanOperation", KiwiValue::Enum(operation.into()));
            }
            let mut nodes = vec![
                parent,
                import_loss_node(2, "GROUP", Some(1)),
                import_loss_node(3, "RECTANGLE", Some(2)),
            ];
            if descendants_first {
                nodes.reverse();
            }
            let (doc, report, _) = fig_to_doc(&doc_from(nodes)).expect("explicit fallback");
            assert_eq!(report.boolean_operations_flattened, 1);
            assert_eq!(report.boolean_operands_dropped, 2);
            assert_eq!(report.instance_children_dropped, 0);
            assert_eq!(report.non_container_children_dropped, 0);
            assert_eq!(report.mapped, doc.scene.len());
            let reason = operation
                .map(|operation| format!("unknown operation {operation}"))
                .unwrap_or_else(|| "missing operation".to_owned());
            assert_eq!(report.boolean_fallbacks_by_reason.get(&reason), Some(&1));
            assert!(
                report
                    .content_loss_summary()
                    .expect("loss warning")
                    .contains(&reason)
            );
            let node = doc
                .scene
                .get(*doc.scene.roots().first().expect("root"))
                .expect("fallback node");
            assert!(matches!(node.data, NodeData::Vector(_)));
            assert_eq!(
                node.meta["figma_boolean_operation"],
                serde_json::json!(operation)
            );
        }
    }
}

#[test]
fn boolean_import_unsupported_operand_geometry_is_a_reported_baked_fallback() {
    let mut parent = import_loss_node(1, "BOOLEAN_OPERATION", None);
    parent.set_field("booleanOperation", KiwiValue::Enum("UNION".into()));
    parent.set_field(
        "fillGeometry",
        KiwiValue::array(vec![fig_path(0, "NONZERO")]),
    );
    let mut triangle = Vec::new();
    triangle.extend(cmd(1, &[0.0, 0.0]));
    triangle.extend(cmd(2, &[10.0, 0.0]));
    triangle.extend(cmd(2, &[5.0, 10.0]));
    triangle.extend(cmd(0, &[]));
    let fig = doc_from_with_blobs(
        vec![parent, import_loss_node(2, "VECTOR", Some(1))],
        vec![triangle],
    );
    let (doc, report, _) = fig_to_doc(&fig).expect("unavailable operand path");
    assert_eq!(report.boolean_operations_flattened, 1);
    assert_eq!(report.boolean_operands_dropped, 1);
    assert_eq!(
        report
            .boolean_fallbacks_by_reason
            .get("operand geometry is unavailable"),
        Some(&1)
    );
    assert_eq!(report.mapped, 1);
    let node = doc
        .scene
        .get(*doc.scene.roots().first().expect("root"))
        .expect("baked fallback");
    assert!(matches!(&node.data, NodeData::Vector(vector) if vector.path.segments.len() == 4));
}

#[test]
fn boolean_import_operandless_vector_fallbacks_preserve_winding_and_stroke_outlines() {
    for stroke_only in [false, true] {
        let mut parent = import_loss_node(1, "BOOLEAN_OPERATION", None);
        parent.set_field("booleanOperation", KiwiValue::Enum("SUBTRACT".into()));
        parent.set_field(
            if stroke_only {
                "strokePaints"
            } else {
                "fillPaints"
            },
            KiwiValue::array(vec![solid_paint(1.0, 0.0, 0.0, 1.0)]),
        );
        if stroke_only {
            parent.set_field("strokeWeight", KiwiValue::Float(2.0));
        }
        parent.set_field(
            if stroke_only {
                "strokeGeometry"
            } else {
                "fillGeometry"
            },
            KiwiValue::array(vec![fig_path(0, "ODD")]),
        );
        let mut ring = Vec::new();
        for points in [
            [[0., 0.], [10., 0.], [10., 10.], [0., 10.]],
            [[3., 3.], [7., 3.], [7., 7.], [3., 7.]],
        ] {
            for (index, point) in points.into_iter().enumerate() {
                ring.extend(cmd(if index == 0 { 1 } else { 2 }, &point));
            }
            ring.extend(cmd(0, &[]));
        }
        let (doc, report, _) = fig_to_doc(&doc_from_with_blobs(vec![parent], vec![ring]))
            .expect("operandless baked operation");
        assert!(
            report
                .content_loss_summary()
                .expect("explicit fallback warning")
                .contains("editable operands are unavailable")
        );
        assert_eq!(report.boolean_operations_flattened, 1);
        assert_eq!(report.boolean_operands_dropped, 0);
        assert_eq!(report.geometry_decoded, 1);
        let root = *doc.scene.roots().first().expect("root");
        let node = doc.scene.get(root).expect("fallback vector");
        let NodeData::Vector(vector) = &node.data else {
            panic!("operandless source remains an editable vector");
        };
        assert_eq!(node.meta["figma_boolean_operation"], "SUBTRACT");
        assert_eq!(node.meta["stroke_only_outline"], stroke_only);
        assert_eq!(vector.path.fill_rule, FillRule::EvenOdd);
        assert_eq!(vector.path.segments.len(), 10);
        assert_eq!(
            vector.fills.as_slice(),
            &[Fill::solid(Color::rgb(255, 0, 0))]
        );
        assert!(vector.strokes.is_empty());
    }
}

#[test]
fn boolean_import_nested_live_operands_are_preserved_and_unknown_ones_fallback() {
    for known_nested_operation in [true, false] {
        let mut outer = import_loss_node(1, "BOOLEAN_OPERATION", None);
        outer.set_field("booleanOperation", KiwiValue::Enum("UNION".into()));
        let mut inner = import_loss_node(2, "BOOLEAN_OPERATION", Some(1));
        inner.set_field(
            "booleanOperation",
            KiwiValue::Enum(
                if known_nested_operation {
                    "XOR"
                } else {
                    "UNKNOWN"
                }
                .into(),
            ),
        );
        let (doc, report, _) = fig_to_doc(&doc_from(vec![
            import_loss_node(3, "RECTANGLE", Some(2)),
            inner,
            outer,
        ]))
        .expect("nested Boolean import");
        let root = *doc.scene.roots().first().expect("root");
        if known_nested_operation {
            assert_eq!(doc.scene.len(), 3);
            assert_eq!(report.boolean_operands_dropped, 0);
            assert!(report.content_loss_summary().is_none());
            assert_boolean_fnx_roundtrip(&doc, root);
        } else {
            assert_eq!(doc.scene.len(), 1);
            assert_eq!(report.boolean_operands_dropped, 2);
            assert_eq!(report.boolean_operations_flattened, 1);
            assert_eq!(
                report
                    .boolean_fallbacks_by_reason
                    .get("unsupported nested Boolean operation"),
                Some(&1)
            );
            assert!(matches!(
                doc.scene.get(root).expect("fallback").data,
                NodeData::Vector(_)
            ));
        }
    }
}

#[test]
fn boolean_import_unmapped_operands_and_invalid_operation_fields_are_reported() {
    let mut parent = import_loss_node(1, "BOOLEAN_OPERATION", None);
    parent.set_field("booleanOperation", KiwiValue::Enum("UNION".into()));
    let (doc, report, _) = fig_to_doc(&doc_from(vec![
        parent.clone(),
        import_loss_node(2, "UNSUPPORTED_OPERAND", Some(1)),
    ]))
    .expect("unmapped operand");
    assert_eq!(
        report
            .boolean_fallbacks_by_reason
            .get("unmapped operand nodes"),
        Some(&1)
    );
    assert_eq!(report.skipped(), 1);
    assert!(matches!(
        doc.scene
            .get(*doc.scene.roots().first().expect("root"))
            .expect("fallback")
            .data,
        NodeData::Vector(_)
    ));
    parent.set_field("booleanOperation", KiwiValue::Float(2.0));
    let (_, report, _) = fig_to_doc(&doc_from(vec![parent])).expect("invalid operation");
    assert_eq!(
        report
            .boolean_fallbacks_by_reason
            .get("invalid operation field"),
        Some(&1)
    );
}

#[test]
fn non_container_import_loss_is_not_reported_as_instance_content() {
    let fig = doc_from(vec![
        import_loss_node(1, "VECTOR", None),
        import_loss_node(2, "RECTANGLE", Some(1)),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).expect("partial import");
    assert_eq!(report.non_container_children_dropped, 1);
    assert_eq!(report.boolean_operations_flattened, 0);
    assert_eq!(report.boolean_operands_dropped, 0);
    assert_eq!(report.instance_children_dropped, 0);
    assert_eq!(report.mapped, doc.scene.len());
    assert!(
        report
            .content_loss_summary()
            .expect("omitted layer warning")
            .contains("layers with unsupported parent relationships were omitted")
    );
}

// =============================================================================
// Task 1: VECTOR geometry — bbox fallback (STEP 1)
// =============================================================================

#[test]
fn vector_family_maps_to_bbox_fallback_vector() {
    // VECTOR/STAR/LINE/BOOLEAN_OPERATION are no longer skipped; each becomes a
    // bbox rectangle vector tagged geometry=bbox_fallback, carrying its fill.
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("VECTOR".into())),
                ("name", KiwiValue::String("Icon path".to_owned())),
                ("size", vector(24.0, 18.0)),
                (
                    "fillPaints",
                    KiwiValue::array(vec![solid_paint(0.0, 0.0, 1.0, 1.0)]),
                ),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("type", KiwiValue::Enum("STAR".into())),
                ("size", vector(30.0, 30.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("type", KiwiValue::Enum("LINE".into())),
                ("size", vector(100.0, 1.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 4)),
                ("type", KiwiValue::Enum("BOOLEAN_OPERATION".into())),
                ("size", vector(40.0, 40.0)),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.mapped, 4, "all four vector-family nodes mapped");
    assert_eq!(report.skipped(), 0, "none skipped");
    assert_eq!(report.vectors_recovered, 4);

    // The VECTOR node: a bbox rect sized 24x18, blue-filled, flagged fallback.
    let vid = doc
        .scene
        .roots()
        .iter()
        .copied()
        .find(|id| doc.scene.get(*id).unwrap().name == "Icon path")
        .expect("vector node present");
    let node = doc.scene.get(vid).unwrap();
    assert_eq!(node.meta["figma_type"], "VECTOR");
    assert_eq!(node.meta["geometry"], "bbox_fallback");
    match &node.data {
        NodeData::Vector(v) => {
            let b = v.path.rough_bounds().unwrap();
            assert_eq!((b.width(), b.height()), (24.0, 18.0));
            assert_eq!(v.fills[0], Fill::solid(Color::rgba(0, 0, 255, 255)));
        }
        other => panic!("VECTOR should map to a Vector, got {other:?}"),
    }
    doc.scene.validate().unwrap();
}

// =============================================================================
// Task 1 (STEP 2): VECTOR geometry — real path decode from command blobs
// =============================================================================

#[test]
fn vector_with_fill_geometry_decodes_real_path() {
    // A VECTOR node whose fillGeometry references a blob holding a triangle
    // (Move/Line/Line/Close) decodes to the real path, tagged geometry=decoded.
    let mut triangle = Vec::new();
    triangle.extend(cmd(1, &[2.0, 2.0])); // Move
    triangle.extend(cmd(2, &[20.0, 2.0])); // Line
    triangle.extend(cmd(2, &[11.0, 16.0])); // Line
    triangle.extend(cmd(0, &[])); // Close

    let fig = doc_from_with_blobs(
        vec![o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("VECTOR".into())),
                ("name", KiwiValue::String("Triangle".to_owned())),
                ("size", vector(24.0, 18.0)),
                (
                    "fillPaints",
                    KiwiValue::array(vec![solid_paint(1.0, 0.0, 0.0, 1.0)]),
                ),
                (
                    "fillGeometry",
                    KiwiValue::array(vec![fig_path(0, "NONZERO")]),
                ),
            ],
        )],
        vec![triangle],
    );

    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.vectors_recovered, 1);
    assert_eq!(report.geometry_decoded, 1, "real geometry decoded");

    let node = doc.scene.get(doc.scene.roots()[0]).unwrap();
    assert_eq!(node.meta["geometry"], "decoded");
    match &node.data {
        NodeData::Vector(v) => {
            // The exact decoded triangle (not a bbox rect).
            assert_eq!(
                v.path.segments,
                vec![
                    PathSegment::Move { to: [2.0, 2.0] },
                    PathSegment::Line { to: [20.0, 2.0] },
                    PathSegment::Line { to: [11.0, 16.0] },
                    PathSegment::Close,
                ]
            );
            assert_eq!(v.path.fill_rule, FillRule::NonZero);
            assert_eq!(v.fills[0], Fill::solid(Color::rgba(255, 0, 0, 255)));
        }
        other => panic!("VECTOR should decode to a Vector, got {other:?}"),
    }
    doc.scene.validate().unwrap();
}

#[test]
fn vector_odd_winding_rule_maps_to_even_odd() {
    let mut blob = Vec::new();
    blob.extend(cmd(1, &[0.0, 0.0]));
    blob.extend(cmd(2, &[5.0, 0.0]));
    blob.extend(cmd(2, &[5.0, 5.0]));
    blob.extend(cmd(0, &[]));
    let fig = doc_from_with_blobs(
        vec![o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("VECTOR".into())),
                ("size", vector(5.0, 5.0)),
                ("fillGeometry", KiwiValue::array(vec![fig_path(0, "ODD")])),
            ],
        )],
        vec![blob],
    );
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    match &doc.scene.get(doc.scene.roots()[0]).unwrap().data {
        NodeData::Vector(v) => assert_eq!(v.path.fill_rule, FillRule::EvenOdd),
        other => panic!("expected vector, got {other:?}"),
    }
}

#[test]
fn multiple_fill_geometry_paths_concatenate_into_one_path() {
    // Two Path entries (two blobs) on one node combine into one PathData with
    // both subpaths — a donut/compound icon.
    let mut outer = Vec::new();
    outer.extend(cmd(1, &[0.0, 0.0]));
    outer.extend(cmd(2, &[10.0, 0.0]));
    outer.extend(cmd(2, &[10.0, 10.0]));
    outer.extend(cmd(0, &[]));
    let mut inner = Vec::new();
    inner.extend(cmd(1, &[3.0, 3.0]));
    inner.extend(cmd(2, &[6.0, 3.0]));
    inner.extend(cmd(2, &[6.0, 6.0]));
    inner.extend(cmd(0, &[]));
    let fig = doc_from_with_blobs(
        vec![o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("BOOLEAN_OPERATION".into())),
                ("size", vector(10.0, 10.0)),
                (
                    "fillGeometry",
                    KiwiValue::array(vec![fig_path(0, "NONZERO"), fig_path(1, "NONZERO")]),
                ),
            ],
        )],
        vec![outer, inner],
    );
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.geometry_decoded, 1);
    match &doc.scene.get(doc.scene.roots()[0]).unwrap().data {
        NodeData::Vector(v) => {
            let moves = v
                .path
                .segments
                .iter()
                .filter(|s| matches!(s, PathSegment::Move { .. }))
                .count();
            assert_eq!(moves, 2, "both subpaths concatenated");
        }
        other => panic!("expected vector, got {other:?}"),
    }
}

#[test]
fn vector_with_garbage_blob_keeps_bbox_fallback() {
    // A fillGeometry index pointing at a malformed blob must NOT regress: the
    // node keeps the bbox fallback (and is not counted as decoded).
    let garbage = vec![0xFF, 0x01, 0x02, 0x03, 0x04, 0x05];
    let fig = doc_from_with_blobs(
        vec![o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("VECTOR".into())),
                ("size", vector(12.0, 12.0)),
                (
                    "fillGeometry",
                    KiwiValue::array(vec![fig_path(0, "NONZERO")]),
                ),
            ],
        )],
        vec![garbage],
    );
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.vectors_recovered, 1);
    assert_eq!(report.geometry_decoded, 0, "garbage blob does not decode");
    let node = doc.scene.get(doc.scene.roots()[0]).unwrap();
    assert_eq!(node.meta["geometry"], "bbox_fallback");
    match &node.data {
        NodeData::Vector(v) => {
            let b = v.path.rough_bounds().unwrap();
            assert_eq!((b.width(), b.height()), (12.0, 12.0));
        }
        other => panic!("expected vector, got {other:?}"),
    }
    doc.scene.validate().unwrap();
}

#[test]
fn vector_with_out_of_range_blob_index_keeps_bbox_fallback() {
    // commandsBlob points past the blob table — must fall back, not panic.
    let fig = doc_from_with_blobs(
        vec![o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("VECTOR".into())),
                ("size", vector(8.0, 8.0)),
                (
                    "fillGeometry",
                    KiwiValue::array(vec![fig_path(999, "NONZERO")]),
                ),
            ],
        )],
        vec![vec![]], // only one (empty) blob; index 999 is out of range
    );
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.geometry_decoded, 0);
    assert_eq!(
        doc.scene.get(doc.scene.roots()[0]).unwrap().meta["geometry"],
        "bbox_fallback"
    );
}

#[test]
fn vector_no_fill_with_stroke_is_stroke_only_outline_not_a_width_stroke() {
    // A fill-less VECTOR whose `fillGeometry` is the already-expanded stroke
    // outline (Lucide/Feather icon) is the Gap A case: it must paint the outline
    // AS a fill (the stroke paint), NOT decode the outline AND add a width stroke
    // on top (which double-strokes / blobs the glyph).
    let mut fill = Vec::new();
    fill.extend(cmd(1, &[0.0, 0.0]));
    fill.extend(cmd(2, &[10.0, 0.0]));
    fill.extend(cmd(2, &[10.0, 10.0]));
    fill.extend(cmd(0, &[]));
    // strokeGeometry just needs to be present; its blob outline is not stored.
    let mut stroke_outline = Vec::new();
    stroke_outline.extend(cmd(1, &[0.0, 0.0]));
    stroke_outline.extend(cmd(2, &[10.0, 0.0]));
    stroke_outline.extend(cmd(0, &[]));

    let stroke_paint = o(
        "Paint",
        vec![
            ("type", KiwiValue::Enum("SOLID".into())),
            ("color", color(0.0, 0.0, 0.0, 1.0)),
        ],
    );
    let fig = doc_from_with_blobs(
        vec![o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("VECTOR".into())),
                ("size", vector(10.0, 10.0)),
                (
                    "fillGeometry",
                    KiwiValue::array(vec![fig_path(0, "NONZERO")]),
                ),
                (
                    "strokeGeometry",
                    KiwiValue::array(vec![fig_path(1, "NONZERO")]),
                ),
                ("strokeWeight", KiwiValue::Float(2.0)),
                ("strokePaints", KiwiValue::array(vec![stroke_paint])),
            ],
        )],
        vec![fill, stroke_outline],
    );
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.geometry_decoded, 1);
    assert_eq!(
        report.stroke_only_vectors, 1,
        "detected as a stroke-only outline"
    );
    match &doc.scene.get(doc.scene.roots()[0]).unwrap().data {
        NodeData::Vector(v) => {
            assert!(v.strokes.is_empty(), "no width stroke (no double-stroke)");
            assert_eq!(
                v.fills.first().cloned(),
                Some(Fill::solid(Color::rgba(0, 0, 0, 255))),
                "the stroke paint is painted as the outline fill"
            );
        }
        other => panic!("expected vector, got {other:?}"),
    }
}

#[test]
fn line_with_stroke_geometry_fills_the_baked_outline() {
    // LINE nodes never carry fillGeometry; Figma bakes the stroked segment —
    // width, caps (incl. arrowheads), dashes — into `strokeGeometry`. The
    // importer paints that outline as a FILL with the stroke's paint and emits
    // no width stroke (the old bbox fallback stroked a degenerate h=0 rect:
    // no caps, dashes traversing the perimeter twice).
    let mut outline = Vec::new();
    outline.extend(cmd(1, &[0.0, 0.0]));
    outline.extend(cmd(2, &[100.0, 0.0]));
    outline.extend(cmd(2, &[100.0, 2.0]));
    outline.extend(cmd(2, &[0.0, 2.0]));
    outline.extend(cmd(0, &[]));

    let fig = doc_from_with_blobs(
        vec![o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("LINE".into())),
                ("name", KiwiValue::String("Divider".to_owned())),
                ("size", vector(100.0, 0.0)),
                (
                    "strokePaints",
                    KiwiValue::array(vec![solid_paint(0.0, 0.0, 0.0, 1.0)]),
                ),
                ("strokeWeight", KiwiValue::Float(2.0)),
                (
                    "strokeGeometry",
                    KiwiValue::array(vec![fig_path(0, "NONZERO")]),
                ),
            ],
        )],
        vec![outline],
    );
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    let node = doc.scene.get(doc.scene.roots()[0]).unwrap();
    assert_eq!(node.meta["geometry"], "stroke_geometry");
    match &node.data {
        NodeData::Vector(v) => {
            assert_eq!(
                v.fills[0],
                Fill::solid(Color::BLACK),
                "the stroke paint becomes the outline fill"
            );
            assert!(
                v.strokes.is_empty(),
                "no width stroke on top of the outline"
            );
            let b = v.path.rough_bounds().unwrap();
            assert_eq!((b.width(), b.height()), (100.0, 2.0));
        }
        other => panic!("expected vector, got {other:?}"),
    }
}

#[test]
fn boolean_operation_without_fill_geometry_uses_stroke_geometry() {
    // A stroke-only BOOLEAN_OPERATION (no fillGeometry at all) must render its
    // baked strokeGeometry result, not a stroked bounding box.
    let mut ring = Vec::new();
    ring.extend(cmd(1, &[0.0, 0.0]));
    ring.extend(cmd(2, &[10.0, 0.0]));
    ring.extend(cmd(2, &[10.0, 10.0]));
    ring.extend(cmd(2, &[0.0, 10.0]));
    ring.extend(cmd(0, &[]));
    let fig = doc_from_with_blobs(
        vec![o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("BOOLEAN_OPERATION".into())),
                ("size", vector(10.0, 10.0)),
                (
                    "strokePaints",
                    KiwiValue::array(vec![solid_paint(1.0, 0.0, 0.0, 1.0)]),
                ),
                ("strokeWeight", KiwiValue::Float(1.0)),
                (
                    "strokeGeometry",
                    KiwiValue::array(vec![fig_path(0, "NONZERO")]),
                ),
            ],
        )],
        vec![ring],
    );
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    let node = doc.scene.get(doc.scene.roots()[0]).unwrap();
    assert_eq!(node.meta["geometry"], "stroke_geometry");
    match &node.data {
        NodeData::Vector(v) => {
            assert_eq!(v.fills[0], Fill::solid(Color::rgb(255, 0, 0)));
            assert!(v.strokes.is_empty());
        }
        other => panic!("expected vector, got {other:?}"),
    }
}

#[test]
fn node_with_fills_keeps_bbox_fallback_over_stroke_geometry() {
    // The strokeGeometry outline path only applies to stroke-only nodes: a
    // FILLED node with no fillGeometry keeps the bbox fallback (filling its
    // stroke outline would drop the fill).
    let mut outline = Vec::new();
    outline.extend(cmd(1, &[0.0, 0.0]));
    outline.extend(cmd(2, &[10.0, 0.0]));
    outline.extend(cmd(2, &[10.0, 10.0]));
    outline.extend(cmd(0, &[]));
    let fig = doc_from_with_blobs(
        vec![o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("VECTOR".into())),
                ("size", vector(10.0, 10.0)),
                (
                    "fillPaints",
                    KiwiValue::array(vec![solid_paint(0.0, 1.0, 0.0, 1.0)]),
                ),
                (
                    "strokePaints",
                    KiwiValue::array(vec![solid_paint(1.0, 0.0, 0.0, 1.0)]),
                ),
                ("strokeWeight", KiwiValue::Float(1.0)),
                (
                    "strokeGeometry",
                    KiwiValue::array(vec![fig_path(0, "NONZERO")]),
                ),
            ],
        )],
        vec![outline],
    );
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    let node = doc.scene.get(doc.scene.roots()[0]).unwrap();
    assert_eq!(node.meta["geometry"], "bbox_fallback");
}

#[test]
fn mixed_winding_rules_are_preserved_per_subpath() {
    // Figma fills each fillGeometry Path with its OWN windingRule. Two paths —
    // a NONZERO outer square, then an ODD path with two subpaths — must keep
    // per-subpath rules so the ODD holes survive.
    let mut outer = Vec::new();
    outer.extend(cmd(1, &[0.0, 0.0]));
    outer.extend(cmd(2, &[10.0, 0.0]));
    outer.extend(cmd(2, &[10.0, 10.0]));
    outer.extend(cmd(0, &[]));
    let mut holes = Vec::new();
    holes.extend(cmd(1, &[2.0, 2.0]));
    holes.extend(cmd(2, &[4.0, 2.0]));
    holes.extend(cmd(2, &[4.0, 4.0]));
    holes.extend(cmd(0, &[]));
    holes.extend(cmd(1, &[6.0, 6.0]));
    holes.extend(cmd(2, &[8.0, 6.0]));
    holes.extend(cmd(2, &[8.0, 8.0]));
    holes.extend(cmd(0, &[]));

    let fig = doc_from_with_blobs(
        vec![o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("VECTOR".into())),
                ("size", vector(10.0, 10.0)),
                (
                    "fillPaints",
                    KiwiValue::array(vec![solid_paint(0.0, 0.0, 1.0, 1.0)]),
                ),
                (
                    "fillGeometry",
                    KiwiValue::array(vec![fig_path(0, "NONZERO"), fig_path(1, "ODD")]),
                ),
            ],
        )],
        vec![outer, holes],
    );
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    match &doc.scene.get(doc.scene.roots()[0]).unwrap().data {
        NodeData::Vector(v) => {
            assert_eq!(v.path.fill_rule, FillRule::NonZero, "first path's rule");
            assert_eq!(
                v.path.subpath_rules,
                vec![FillRule::NonZero, FillRule::EvenOdd, FillRule::EvenOdd],
                "one rule per subpath: NONZERO outer, ODD hole path's two subpaths"
            );
        }
        other => panic!("expected vector, got {other:?}"),
    }
}

#[test]
fn uniform_winding_rules_leave_subpath_rules_empty() {
    // The common all-same-rule case must not populate subpath_rules (keeps the
    // serialized form byte-identical to pre-feature docs).
    let mut a = Vec::new();
    a.extend(cmd(1, &[0.0, 0.0]));
    a.extend(cmd(2, &[10.0, 0.0]));
    a.extend(cmd(0, &[]));
    let mut b = Vec::new();
    b.extend(cmd(1, &[3.0, 3.0]));
    b.extend(cmd(2, &[6.0, 3.0]));
    b.extend(cmd(0, &[]));
    let fig = doc_from_with_blobs(
        vec![o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("VECTOR".into())),
                ("size", vector(10.0, 10.0)),
                (
                    "fillGeometry",
                    KiwiValue::array(vec![fig_path(0, "ODD"), fig_path(1, "ODD")]),
                ),
            ],
        )],
        vec![a, b],
    );
    let (doc, _, _) = fig_to_doc(&fig).unwrap();
    match &doc.scene.get(doc.scene.roots()[0]).unwrap().data {
        NodeData::Vector(v) => {
            assert_eq!(v.path.fill_rule, FillRule::EvenOdd);
            assert!(v.path.subpath_rules.is_empty());
        }
        other => panic!("expected vector, got {other:?}"),
    }
}
