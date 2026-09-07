//! VECTOR geometry: bbox fallback + real path decode from command blobs.

use super::*;

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
                    KiwiValue::Array(vec![solid_paint(0.0, 0.0, 1.0, 1.0)]),
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
                    KiwiValue::Array(vec![solid_paint(1.0, 0.0, 0.0, 1.0)]),
                ),
                (
                    "fillGeometry",
                    KiwiValue::Array(vec![fig_path(0, "NONZERO")]),
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
                ("fillGeometry", KiwiValue::Array(vec![fig_path(0, "ODD")])),
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
                    KiwiValue::Array(vec![fig_path(0, "NONZERO"), fig_path(1, "NONZERO")]),
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
                    KiwiValue::Array(vec![fig_path(0, "NONZERO")]),
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
                    KiwiValue::Array(vec![fig_path(999, "NONZERO")]),
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
                    KiwiValue::Array(vec![fig_path(0, "NONZERO")]),
                ),
                (
                    "strokeGeometry",
                    KiwiValue::Array(vec![fig_path(1, "NONZERO")]),
                ),
                ("strokeWeight", KiwiValue::Float(2.0)),
                ("strokePaints", KiwiValue::Array(vec![stroke_paint])),
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
                    KiwiValue::Array(vec![solid_paint(0.0, 0.0, 0.0, 1.0)]),
                ),
                ("strokeWeight", KiwiValue::Float(2.0)),
                (
                    "strokeGeometry",
                    KiwiValue::Array(vec![fig_path(0, "NONZERO")]),
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
                    KiwiValue::Array(vec![solid_paint(1.0, 0.0, 0.0, 1.0)]),
                ),
                ("strokeWeight", KiwiValue::Float(1.0)),
                (
                    "strokeGeometry",
                    KiwiValue::Array(vec![fig_path(0, "NONZERO")]),
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
                    KiwiValue::Array(vec![solid_paint(0.0, 1.0, 0.0, 1.0)]),
                ),
                (
                    "strokePaints",
                    KiwiValue::Array(vec![solid_paint(1.0, 0.0, 0.0, 1.0)]),
                ),
                ("strokeWeight", KiwiValue::Float(1.0)),
                (
                    "strokeGeometry",
                    KiwiValue::Array(vec![fig_path(0, "NONZERO")]),
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
                    KiwiValue::Array(vec![solid_paint(0.0, 0.0, 1.0, 1.0)]),
                ),
                (
                    "fillGeometry",
                    KiwiValue::Array(vec![fig_path(0, "NONZERO"), fig_path(1, "ODD")]),
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
                    KiwiValue::Array(vec![fig_path(0, "ODD"), fig_path(1, "ODD")]),
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
