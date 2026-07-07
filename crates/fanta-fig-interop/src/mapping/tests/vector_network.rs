//! OpenPencil port: multi-stroke build_stroke + vectorNetworkBlob decode.

use super::*;

// =============================================================================
// OpenPencil port — multiple stroke paints (build_stroke) + vectorNetworkBlob
// fallback decoder (decode_vector_network / parse_vector_network).
// =============================================================================

/// A test segment spec: `(startVertex, tangentStart, endVertex, tangentEnd)`.
type VnSegSpec = (u32, (f32, f32), u32, (f32, f32));
/// A test region spec: `(windingRule, loops)` where each loop is a list of
/// segment indices.
type VnRegionSpec<'a> = (u32, &'a [&'a [u32]]);

/// Encode a `vectorNetworkBlob` byte stream in the confirmed wire format
/// (verified against the Adobe Spectrum fixture + OpenPencil op1):
///   header  : [numVertices:u32, numSegments:u32, numRegions:u32]
///   vertex  : [styleIdx:u32, x:f32, y:f32]
///   segment : [styleIdx:u32, start:u32, tsX:f32, tsY:f32, end:u32, teX:f32, teY:f32]
///   region  : [winding:u32(0=even-odd), numLoops:u32, {numSegs:u32, segIdx:u32 …} …]
fn vn_blob(verts: &[(f32, f32)], segs: &[VnSegSpec], regions: &[VnRegionSpec]) -> Vec<u8> {
    let mut b = Vec::new();
    let push_u32 = |b: &mut Vec<u8>, v: u32| b.extend_from_slice(&v.to_le_bytes());
    let push_f32 = |b: &mut Vec<u8>, v: f32| b.extend_from_slice(&v.to_le_bytes());
    push_u32(&mut b, verts.len() as u32);
    push_u32(&mut b, segs.len() as u32);
    push_u32(&mut b, regions.len() as u32);
    for (x, y) in verts {
        push_u32(&mut b, 0); // styleIdx
        push_f32(&mut b, *x);
        push_f32(&mut b, *y);
    }
    for (start, ts, end, te) in segs {
        push_u32(&mut b, 0); // styleIdx
        push_u32(&mut b, *start);
        push_f32(&mut b, ts.0);
        push_f32(&mut b, ts.1);
        push_u32(&mut b, *end);
        push_f32(&mut b, te.0);
        push_f32(&mut b, te.1);
    }
    for (winding, loops) in regions {
        push_u32(&mut b, *winding);
        push_u32(&mut b, loops.len() as u32);
        for lp in *loops {
            push_u32(&mut b, lp.len() as u32);
            for s in *lp {
                push_u32(&mut b, *s);
            }
        }
    }
    b
}

/// A VECTOR NodeChange that carries only a `vectorData.vectorNetworkBlob` (no
/// `fillGeometry`), plus an optional `normalizedSize` (defaults to `size`).
fn vn_vector_change(
    size: (f64, f64),
    norm: Option<(f64, f64)>,
    blob_index: u32,
    extra: Vec<(&str, KiwiValue)>,
) -> KiwiValue {
    let n = norm.unwrap_or(size);
    let vd = vec![
        ("vectorNetworkBlob", KiwiValue::Uint(blob_index)),
        ("normalizedSize", vector(n.0, n.1)),
    ];
    let mut fields = vec![
        ("guid", guid(0, 2)),
        ("parentIndex", parent_index(0, 1)),
        ("type", KiwiValue::Enum("VECTOR".to_owned())),
        ("name", KiwiValue::String("VN".to_owned())),
        ("size", vector(size.0, size.1)),
        ("vectorData", o("VectorData", vd)),
    ];
    fields.extend(extra);
    o("NodeChange", fields)
}

#[test]
fn parse_vector_network_decodes_a_triangle_region() {
    // Three vertices, three straight segments (zero tangents) forming a closed
    // even-odd region. No scaling (1:1).
    let blob = vn_blob(
        &[(0.0, 0.0), (10.0, 0.0), (5.0, 8.0)],
        &[
            (0, (0.0, 0.0), 1, (0.0, 0.0)),
            (1, (0.0, 0.0), 2, (0.0, 0.0)),
            (2, (0.0, 0.0), 0, (0.0, 0.0)),
        ],
        &[(0 /*even-odd*/, &[&[0, 1, 2]])],
    );
    let net = parse_vector_network(&blob).expect("parses");
    let path = net.to_path(1.0, 1.0).expect("path");
    assert_eq!(path.fill_rule, FillRule::EvenOdd);
    // Move + 3 lines + close.
    let moves = path
        .segments
        .iter()
        .filter(|s| matches!(s, PathSegment::Move { .. }))
        .count();
    let lines = path
        .segments
        .iter()
        .filter(|s| matches!(s, PathSegment::Line { .. }))
        .count();
    let closes = path
        .segments
        .iter()
        .filter(|s| matches!(s, PathSegment::Close))
        .count();
    assert_eq!(
        (moves, lines, closes),
        (1, 3, 1),
        "closed triangle: {:?}",
        path.segments
    );
}

#[test]
fn parse_vector_network_emits_cubic_for_nonzero_tangents() {
    // A single segment with non-zero tangents must become a cubic whose control
    // points are vertex + tangent offset.
    let blob = vn_blob(
        &[(0.0, 0.0), (10.0, 0.0)],
        &[(0, (2.0, 3.0), 1, (-1.0, 4.0))],
        &[(1 /*non-zero*/, &[&[0]])],
    );
    let net = parse_vector_network(&blob).expect("parses");
    let path = net.to_path(1.0, 1.0).expect("path");
    assert_eq!(path.fill_rule, FillRule::NonZero);
    let cubic = path
        .segments
        .iter()
        .find_map(|s| match s {
            PathSegment::Cubic { ctrl1, ctrl2, to } => Some((*ctrl1, *ctrl2, *to)),
            _ => None,
        })
        .expect("a cubic segment");
    // ctrl1 = start + tangentStart = (0+2, 0+3); ctrl2 = end + tangentEnd = (10-1, 0+4).
    assert_eq!(cubic.0, [2.0, 3.0]);
    assert_eq!(cubic.1, [9.0, 4.0]);
    assert_eq!(cubic.2, [10.0, 0.0]);
}

#[test]
fn parse_vector_network_walks_open_chain_without_region() {
    // Two connected straight segments, NO region — should walk as one open chain
    // (Move + 2 Lines, no Close).
    let blob = vn_blob(
        &[(0.0, 0.0), (5.0, 0.0), (10.0, 0.0)],
        &[
            (0, (0.0, 0.0), 1, (0.0, 0.0)),
            (1, (0.0, 0.0), 2, (0.0, 0.0)),
        ],
        &[],
    );
    let net = parse_vector_network(&blob).expect("parses");
    let path = net.to_path(1.0, 1.0).expect("path");
    let moves = path
        .segments
        .iter()
        .filter(|s| matches!(s, PathSegment::Move { .. }))
        .count();
    let lines = path
        .segments
        .iter()
        .filter(|s| matches!(s, PathSegment::Line { .. }))
        .count();
    let closes = path
        .segments
        .iter()
        .filter(|s| matches!(s, PathSegment::Close))
        .count();
    assert_eq!(
        (moves, lines, closes),
        (1, 2, 0),
        "open chain: {:?}",
        path.segments
    );
}

#[test]
fn parse_vector_network_rejects_truncated_and_oob() {
    // Header promises 2 vertices but the blob ends mid-vertex.
    let mut truncated = Vec::new();
    truncated.extend_from_slice(&2u32.to_le_bytes()); // nV
    truncated.extend_from_slice(&0u32.to_le_bytes()); // nS
    truncated.extend_from_slice(&0u32.to_le_bytes()); // nR
    truncated.extend_from_slice(&0u32.to_le_bytes()); // styleIdx of vertex 0
    truncated.extend_from_slice(&1.0f32.to_le_bytes()); // x of vertex 0, then EOF
    assert!(
        parse_vector_network(&truncated).is_none(),
        "truncated must fail"
    );

    // A segment references a vertex index out of range.
    let oob = vn_blob(&[(0.0, 0.0)], &[(0, (0.0, 0.0), 5, (0.0, 0.0))], &[]);
    assert!(
        parse_vector_network(&oob).is_none(),
        "out-of-range vertex index must fail"
    );

    // Garbage bytes must never panic.
    for seed in 0u8..40 {
        let bytes: Vec<u8> = (0..53u8)
            .map(|i| i.wrapping_mul(seed).wrapping_add(3))
            .collect();
        let _ = parse_vector_network(&bytes);
    }
}

#[test]
fn decode_vector_network_scales_from_normalized_size() {
    // normalizedSize (10x10) → node size (20x40): x doubles, y quadruples.
    let blob = vn_blob(
        &[(0.0, 0.0), (10.0, 10.0)],
        &[(0, (0.0, 0.0), 1, (0.0, 0.0))],
        &[],
    );
    let change = vn_vector_change((20.0, 40.0), Some((10.0, 10.0)), 0, Vec::new());
    let path = decode_vector_network(&change, (20.0, 40.0), &[blob]).expect("decodes");
    // The line endpoint (10,10) in normalized space lands at (20, 40) scaled.
    let last_line = path
        .segments
        .iter()
        .rev()
        .find_map(|s| match s {
            PathSegment::Line { to } => Some(*to),
            _ => None,
        })
        .expect("a line");
    assert_eq!(last_line, [20.0, 40.0], "scaled endpoint");
}

#[test]
fn build_vector_uses_vector_network_when_fill_geometry_absent() {
    // End-to-end: a VECTOR node with no fillGeometry but a vectorNetworkBlob gets
    // its real path (tagged meta.geometry = "vector_network"), not a bbox rect.
    let triangle = vn_blob(
        &[(0.0, 0.0), (12.0, 0.0), (6.0, 12.0)],
        &[
            (0, (0.0, 0.0), 1, (0.0, 0.0)),
            (1, (0.0, 0.0), 2, (0.0, 0.0)),
            (2, (0.0, 0.0), 0, (0.0, 0.0)),
        ],
        &[(0, &[&[0, 1, 2]])],
    );
    let fig = doc_from_with_blobs(
        vec![
            o(
                "NodeChange",
                vec![
                    ("guid", guid(0, 0)),
                    ("type", KiwiValue::Enum("DOCUMENT".to_owned())),
                ],
            ),
            o(
                "NodeChange",
                vec![
                    ("guid", guid(0, 1)),
                    ("parentIndex", parent_index(0, 0)),
                    ("type", KiwiValue::Enum("CANVAS".to_owned())),
                    ("name", KiwiValue::String("Page 1".to_owned())),
                ],
            ),
            vn_vector_change((12.0, 12.0), Some((12.0, 12.0)), 0, Vec::new()),
        ],
        vec![triangle],
    );
    let (doc, report, _imgs) = fig_to_doc(&fig).expect("map");
    assert_eq!(report.vector_network_decoded, 1, "one VN fallback decode");
    assert_eq!(report.vectors_recovered, 1);
    // The single VECTOR node is in the scene with a real triangle path.
    let mut found = false;
    for root in doc.scene.roots().to_vec() {
        for id in doc.scene.descendants_of(root) {
            let Some(n) = doc.scene.get(id) else { continue };
            if n.meta.get("geometry").and_then(|g| g.as_str()) == Some("vector_network") {
                found = true;
                let NodeData::Vector(v) = &n.data else {
                    panic!("vector node")
                };
                let moves = v
                    .path
                    .segments
                    .iter()
                    .filter(|s| matches!(s, PathSegment::Move { .. }))
                    .count();
                assert_eq!(moves, 1, "one closed contour");
                assert!(
                    matches!(v.path.segments.last(), Some(PathSegment::Close)),
                    "contour is closed"
                );
            }
        }
    }
    assert!(found, "VN-decoded vector present in scene");
}

#[test]
fn fill_geometry_is_preferred_over_vector_network() {
    // A node carrying BOTH fillGeometry and vectorNetworkBlob must use the
    // (pre-flattened) fillGeometry path and tag meta.geometry = "decoded".
    let mut tri_cmd = Vec::new();
    tri_cmd.extend(cmd(1, &[0.0, 0.0])); // Move
    tri_cmd.extend(cmd(2, &[10.0, 0.0])); // Line
    tri_cmd.extend(cmd(2, &[5.0, 8.0])); // Line
    tri_cmd.extend(cmd(0, &[])); // Close
    let vn = vn_blob(
        &[(0.0, 0.0), (99.0, 0.0)],
        &[(0, (0.0, 0.0), 1, (0.0, 0.0))],
        &[],
    );
    let change = vn_vector_change(
        (10.0, 8.0),
        Some((10.0, 8.0)),
        1, // vectorNetworkBlob index
        vec![(
            "fillGeometry",
            KiwiValue::Array(vec![fig_path(0, "NONZERO")]),
        )],
    );
    let fig = doc_from_with_blobs(
        vec![
            o(
                "NodeChange",
                vec![
                    ("guid", guid(0, 0)),
                    ("type", KiwiValue::Enum("DOCUMENT".to_owned())),
                ],
            ),
            o(
                "NodeChange",
                vec![
                    ("guid", guid(0, 1)),
                    ("parentIndex", parent_index(0, 0)),
                    ("type", KiwiValue::Enum("CANVAS".to_owned())),
                ],
            ),
            change,
        ],
        vec![tri_cmd, vn],
    );
    let (doc, report, _imgs) = fig_to_doc(&fig).expect("map");
    assert_eq!(
        report.vector_network_decoded, 0,
        "fillGeometry wins, no VN fallback"
    );
    assert_eq!(report.geometry_decoded, 1);
    let mut tagged_decoded = false;
    for root in doc.scene.roots().to_vec() {
        for id in doc.scene.descendants_of(root) {
            let Some(n) = doc.scene.get(id) else { continue };
            if matches!(&n.data, NodeData::Vector(_)) {
                assert_eq!(
                    n.meta.get("geometry").and_then(|g| g.as_str()),
                    Some("decoded")
                );
                tagged_decoded = true;
            }
        }
    }
    assert!(tagged_decoded, "vector tagged decoded (not vector_network)");
}

#[test]
fn malformed_vector_network_blob_falls_back_to_bbox() {
    // A garbage vectorNetworkBlob must not regress to a torn path — the node keeps
    // its bbox rectangle (meta.geometry = "bbox_fallback").
    let garbage = vec![0xFFu8; 20];
    let change = vn_vector_change((30.0, 30.0), Some((30.0, 30.0)), 0, Vec::new());
    let fig = doc_from_with_blobs(
        vec![
            o(
                "NodeChange",
                vec![
                    ("guid", guid(0, 0)),
                    ("type", KiwiValue::Enum("DOCUMENT".to_owned())),
                ],
            ),
            o(
                "NodeChange",
                vec![
                    ("guid", guid(0, 1)),
                    ("parentIndex", parent_index(0, 0)),
                    ("type", KiwiValue::Enum("CANVAS".to_owned())),
                ],
            ),
            change,
        ],
        vec![garbage],
    );
    let (doc, report, _imgs) = fig_to_doc(&fig).expect("map");
    assert_eq!(report.vector_network_decoded, 0);
    let mut bbox = false;
    for root in doc.scene.roots().to_vec() {
        for id in doc.scene.descendants_of(root) {
            let Some(n) = doc.scene.get(id) else { continue };
            if matches!(&n.data, NodeData::Vector(_)) {
                assert_eq!(
                    n.meta.get("geometry").and_then(|g| g.as_str()),
                    Some("bbox_fallback")
                );
                bbox = true;
            }
        }
    }
    assert!(bbox, "garbage VN blob → bbox fallback, no panic");
}

#[test]
fn build_stroke_emits_one_stroke_per_visible_paint() {
    // Two visible stroke paints + a hidden one → exactly two strokes, in order,
    // both sharing the node's single weight/align.
    let hidden = {
        let mut p = solid_paint(1.0, 0.0, 0.0, 1.0);
        p.set_field("visible", KiwiValue::Bool(false));
        p
    };
    let change = o(
        "NodeChange",
        vec![
            ("type", KiwiValue::Enum("VECTOR".to_owned())),
            ("size", vector(20.0, 20.0)),
            ("strokeWeight", KiwiValue::Float(3.0)),
            ("strokeAlign", KiwiValue::Enum("OUTSIDE".to_owned())),
            (
                "strokePaints",
                KiwiValue::Array(vec![
                    solid_paint(1.0, 0.0, 0.0, 1.0), // red, bottom
                    hidden,                          // skipped
                    solid_paint(0.0, 0.0, 1.0, 1.0), // blue, top
                ]),
            ),
        ],
    );
    let strokes = build_stroke(&change);
    assert_eq!(strokes.len(), 2, "two visible paints → two strokes");
    for s in &strokes {
        assert_eq!(s.width, 3.0);
        assert_eq!(s.align, StrokeAlign::Outside);
    }
    // Order preserved: red (bottom) first, blue (top) second.
    let c0 = match &strokes[0].paint {
        Fill::Solid { color } => *color,
        _ => panic!("solid"),
    };
    let c1 = match &strokes[1].paint {
        Fill::Solid { color } => *color,
        _ => panic!("solid"),
    };
    assert!(c0.r > c0.b, "first stroke is the red paint");
    assert!(c1.b > c1.r, "second stroke is the blue paint");
}

#[test]
fn build_stroke_defaults_to_black_when_weight_has_no_paints() {
    let change = o(
        "NodeChange",
        vec![
            ("type", KiwiValue::Enum("ROUNDED_RECTANGLE".to_owned())),
            ("size", vector(20.0, 20.0)),
            ("strokeWeight", KiwiValue::Float(1.0)),
        ],
    );

    let strokes = build_stroke(&change);
    assert_eq!(strokes.len(), 1, "positive weight implies a default stroke");
    assert_eq!(strokes[0].width, 1.0);
    assert_eq!(strokes[0].align, StrokeAlign::Inside);
    match &strokes[0].paint {
        Fill::Solid { color } => assert_eq!(*color, Color::BLACK),
        other => panic!("expected solid black stroke, got {other:?}"),
    }
}

#[test]
fn build_stroke_does_not_default_decorative_vector_family_paints() {
    let change = o(
        "NodeChange",
        vec![
            ("type", KiwiValue::Enum("STAR".to_owned())),
            ("size", vector(20.0, 20.0)),
            ("strokeWeight", KiwiValue::Float(1.0)),
        ],
    );

    assert!(
        build_stroke(&change).is_empty(),
        "decorative vector-family nodes need explicit stroke paints"
    );
}

#[test]
fn build_stroke_does_not_default_frame_paints() {
    let change = o(
        "NodeChange",
        vec![
            ("type", KiwiValue::Enum("FRAME".to_owned())),
            ("size", vector(20.0, 20.0)),
            ("strokeWeight", KiwiValue::Float(1.0)),
        ],
    );

    assert!(
        build_stroke(&change).is_empty(),
        "frame-like containers need explicit stroke paints"
    );
}

#[test]
fn build_stroke_drops_all_when_no_visible_paint_or_zero_weight() {
    // All paints hidden → no strokes.
    let hidden = {
        let mut p = solid_paint(0.0, 0.0, 0.0, 1.0);
        p.set_field("visible", KiwiValue::Bool(false));
        p
    };
    let all_hidden = o(
        "NodeChange",
        vec![
            ("type", KiwiValue::Enum("VECTOR".to_owned())),
            ("strokeWeight", KiwiValue::Float(2.0)),
            ("strokePaints", KiwiValue::Array(vec![hidden])),
        ],
    );
    assert!(
        build_stroke(&all_hidden).is_empty(),
        "no visible paint → no stroke"
    );

    // Visible paints but zero weight (and no stroke geometry) → no strokes.
    let zero_weight = o(
        "NodeChange",
        vec![
            ("type", KiwiValue::Enum("VECTOR".to_owned())),
            ("strokeWeight", KiwiValue::Float(0.0)),
            (
                "strokePaints",
                KiwiValue::Array(vec![
                    solid_paint(0.0, 0.0, 0.0, 1.0),
                    solid_paint(1.0, 1.0, 1.0, 1.0),
                ]),
            ),
        ],
    );
    assert!(
        build_stroke(&zero_weight).is_empty(),
        "zero weight → no phantom strokes"
    );
}
