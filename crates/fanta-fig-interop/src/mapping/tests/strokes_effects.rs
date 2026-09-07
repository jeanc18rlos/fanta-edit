//! Strokes, per-corner radii, arc ellipses, opacity/blend, shadows, blurs.

use super::*;

#[test]
fn stroke_imports_paint_weight_and_align() {
    let rect = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("RECTANGLE".into())),
            ("size", vector(100.0, 50.0)),
            (
                "fillPaints",
                KiwiValue::Array(vec![solid_paint(1.0, 1.0, 1.0, 1.0)]),
            ),
            (
                "strokePaints",
                KiwiValue::Array(vec![solid_paint(0.0, 0.0, 1.0, 1.0)]),
            ),
            ("strokeWeight", KiwiValue::Float(3.0)),
            ("strokeAlign", KiwiValue::Enum("INSIDE".into())),
        ],
    );
    let (doc, report, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    assert_eq!(report.strokes_imported, 1);
    let v = first_vector(&doc);
    assert_eq!(v.strokes.len(), 1);
    let s = &v.strokes[0];
    assert_eq!(s.width, 3.0);
    assert_eq!(s.align, fanta_doc::style::StrokeAlign::Inside);
    assert_eq!(s.paint, Fill::solid(Color::rgba(0, 0, 255, 255)));
}

#[test]
fn frame_imports_border_stroke_and_corner_radius() {
    // A FRAME carrying a border (strokePaints + strokeWeight + align) AND a
    // uniform corner radius must import BOTH onto its GroupNode — the
    // frame-border fidelity fix. Before this, frames dropped strokes + rounding.
    let frame = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("FRAME".into())),
            ("name", KiwiValue::String("Frame".to_owned())),
            ("size", vector(120.0, 80.0)),
            (
                "fillPaints",
                KiwiValue::Array(vec![solid_paint(1.0, 1.0, 1.0, 1.0)]),
            ),
            (
                "strokePaints",
                KiwiValue::Array(vec![solid_paint(0.0, 0.0, 1.0, 1.0)]),
            ),
            ("strokeWeight", KiwiValue::Float(2.0)),
            ("strokeAlign", KiwiValue::Enum("INSIDE".into())),
            ("cornerRadius", KiwiValue::Float(6.0)),
        ],
    );
    let (doc, report, _) = fig_to_doc(&doc_with_shape(frame)).unwrap();
    assert_eq!(report.frames_with_stroke, 1, "the frame gained a border");
    assert_eq!(report.frames_rounded, 1, "the frame gained corner rounding");
    assert_eq!(
        report.strokes_imported, 1,
        "frame strokes count as imported strokes"
    );
    let g = group_named(&doc, "Frame");
    assert_eq!(g.clip_size, Some([120.0, 80.0]));
    assert_eq!(g.strokes.len(), 1, "the frame's border survived import");
    let s = &g.strokes[0];
    assert_eq!(s.width, 2.0);
    assert_eq!(s.align, fanta_doc::style::StrokeAlign::Inside);
    assert_eq!(s.paint, Fill::solid(Color::rgba(0, 0, 255, 255)));
    assert_eq!(g.corner_radius, Some(6.0), "uniform frame rounding");
    assert_eq!(g.corner_radii, None);
}

#[test]
fn frame_imports_independent_per_corner_radii() {
    // A FRAME with mixed per-corner radii imports them as `corner_radii`
    // ([TL, TR, BR, BL]) on the GroupNode — a rounded card with distinct corners.
    let frame = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("FRAME".into())),
            ("name", KiwiValue::String("Frame".to_owned())),
            ("size", vector(100.0, 60.0)),
            (
                "fillPaints",
                KiwiValue::Array(vec![solid_paint(1.0, 1.0, 1.0, 1.0)]),
            ),
            ("rectangleTopLeftCornerRadius", KiwiValue::Float(8.0)),
            ("rectangleTopRightCornerRadius", KiwiValue::Float(8.0)),
            ("rectangleBottomRightCornerRadius", KiwiValue::Float(0.0)),
            ("rectangleBottomLeftCornerRadius", KiwiValue::Float(0.0)),
        ],
    );
    let (doc, report, _) = fig_to_doc(&doc_with_shape(frame)).unwrap();
    assert_eq!(report.frames_rounded, 1);
    let g = group_named(&doc, "Frame");
    assert_eq!(
        g.corner_radii,
        Some([8.0, 8.0, 0.0, 0.0]),
        "TL, TR, BR, BL order"
    );
    assert!(g.corner_radius.is_none());
}

#[test]
fn stroke_imports_per_side_border_weights() {
    // F1 — a node with `borderStrokeWeightsIndependent` and differing per-side
    // weights imports them onto the stroke's `per_side` field [T, R, B, L].
    let rect = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("RECTANGLE".into())),
            ("size", vector(100.0, 50.0)),
            (
                "strokePaints",
                KiwiValue::Array(vec![solid_paint(0.0, 0.0, 0.0, 1.0)]),
            ),
            ("strokeWeight", KiwiValue::Float(1.0)),
            ("borderStrokeWeightsIndependent", KiwiValue::Bool(true)),
            ("borderTopWeight", KiwiValue::Float(4.0)),
            ("borderRightWeight", KiwiValue::Float(0.0)),
            ("borderBottomWeight", KiwiValue::Float(2.0)),
            ("borderLeftWeight", KiwiValue::Float(0.0)),
        ],
    );
    let (doc, report, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    assert_eq!(report.per_side_borders, 1, "one node with per-side borders");
    let v = first_vector(&doc);
    assert_eq!(v.strokes.len(), 1);
    assert_eq!(
        v.strokes[0].per_side,
        Some([4.0, 0.0, 2.0, 0.0]),
        "[top, right, bottom, left]"
    );
}

#[test]
fn uniform_independent_border_weights_collapse_to_none() {
    // F1 — `borderStrokeWeightsIndependent` set but all four sides equal: the
    // per_side array collapses to `None` (indistinguishable from a uniform
    // weight), keeping the common case allocation-free / byte-stable.
    let rect = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("RECTANGLE".into())),
            ("size", vector(60.0, 60.0)),
            (
                "strokePaints",
                KiwiValue::Array(vec![solid_paint(0.0, 0.0, 0.0, 1.0)]),
            ),
            ("strokeWeight", KiwiValue::Float(3.0)),
            ("borderStrokeWeightsIndependent", KiwiValue::Bool(true)),
            ("borderTopWeight", KiwiValue::Float(3.0)),
            ("borderRightWeight", KiwiValue::Float(3.0)),
            ("borderBottomWeight", KiwiValue::Float(3.0)),
            ("borderLeftWeight", KiwiValue::Float(3.0)),
        ],
    );
    let (doc, report, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    assert_eq!(report.per_side_borders, 0, "all-equal collapses to uniform");
    let v = first_vector(&doc);
    assert_eq!(v.strokes[0].per_side, None);
}

#[test]
fn ellipse_with_arc_data_tessellates_pie() {
    // F3 — an ELLIPSE with a partial-sweep `arcData` (90°, no inner radius)
    // imports as a real pie-slice path (center Move + Line + arc cubics + Close),
    // not a full ellipse. The full ellipse is exactly 6 segments (M + 4 C + Z);
    // a quarter pie is M + L + 1 C + Z = 4 segments and starts at the box center.
    let arc = o(
        "ArcData",
        vec![
            ("startingAngle", KiwiValue::Float(0.0)),
            ("endingAngle", KiwiValue::Float(std::f32::consts::FRAC_PI_2)),
            ("innerRadius", KiwiValue::Float(0.0)),
        ],
    );
    let ellipse = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("ELLIPSE".into())),
            ("size", vector(40.0, 40.0)),
            (
                "fillPaints",
                KiwiValue::Array(vec![solid_paint(1.0, 0.0, 0.0, 1.0)]),
            ),
            ("arcData", arc),
        ],
    );
    let (doc, report, _) = fig_to_doc(&doc_with_shape(ellipse)).unwrap();
    assert_eq!(report.arc_ellipses, 1, "one arc ellipse");
    let v = first_vector(&doc);
    use fanta_doc::path::PathSegment;
    // First segment is a Move to the box center (20, 20).
    assert!(
        matches!(v.path.segments.first(), Some(PathSegment::Move { to }) if (to[0]-20.0).abs()<1e-4 && (to[1]-20.0).abs()<1e-4),
        "pie starts at the box center"
    );
    let cubics = v
        .path
        .segments
        .iter()
        .filter(|s| matches!(s, PathSegment::Cubic { .. }))
        .count();
    assert_eq!(
        cubics, 1,
        "a 90° pie is one cubic, not the 4 of a full ellipse"
    );

    // The arc is also kept as a scrubable ParametricShape::Arc, and regenerating
    // it reproduces the tessellated path exactly.
    match v.parametric {
        Some(fanta_doc::ParametricShape::Arc {
            start_rad,
            sweep_rad,
            inner_ratio,
        }) => {
            assert!((start_rad - 0.0).abs() < 1e-4);
            assert!((sweep_rad - std::f64::consts::FRAC_PI_2).abs() < 1e-4);
            assert_eq!(inner_ratio, 0.0);
        }
        other => panic!("expected a parametric Arc, got {other:?}"),
    }
    assert_eq!(
        v.parametric.unwrap().to_path(40.0, 40.0),
        v.path,
        "regenerating the arc reproduces the imported path"
    );
}

#[test]
fn ellipse_with_donut_arc_data_has_a_hole() {
    // F3 — an ELLIPSE with inner radius > 0 imports as a donut/ring (no center
    // Move; two arc spans for a quarter ring).
    let arc = o(
        "ArcData",
        vec![
            ("startingAngle", KiwiValue::Float(0.0)),
            ("endingAngle", KiwiValue::Float(std::f32::consts::FRAC_PI_2)),
            ("innerRadius", KiwiValue::Float(0.5)),
        ],
    );
    let ellipse = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("ELLIPSE".into())),
            ("size", vector(40.0, 40.0)),
            (
                "fillPaints",
                KiwiValue::Array(vec![solid_paint(1.0, 0.0, 0.0, 1.0)]),
            ),
            ("arcData", arc),
        ],
    );
    let (doc, report, _) = fig_to_doc(&doc_with_shape(ellipse)).unwrap();
    assert_eq!(report.arc_ellipses, 1);
    let v = first_vector(&doc);
    use fanta_doc::path::PathSegment;
    let cubics = v
        .path
        .segments
        .iter()
        .filter(|s| matches!(s, PathSegment::Cubic { .. }))
        .count();
    assert_eq!(cubics, 2, "donut quarter = outer + inner arc span");
    assert!(
        v.path
            .segments
            .iter()
            .any(|s| matches!(s, PathSegment::Line { .. }))
    );
}

#[test]
fn full_ellipse_arc_data_stays_a_full_ellipse() {
    // F3 — a 360° arcData with no inner radius is a plain full disc: it must keep
    // the full-ellipse path (6 segments) and NOT count as an arc ellipse.
    let arc = o(
        "ArcData",
        vec![
            ("startingAngle", KiwiValue::Float(0.0)),
            ("endingAngle", KiwiValue::Float(std::f32::consts::TAU)),
            ("innerRadius", KiwiValue::Float(0.0)),
        ],
    );
    let ellipse = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("ELLIPSE".into())),
            ("size", vector(40.0, 40.0)),
            (
                "fillPaints",
                KiwiValue::Array(vec![solid_paint(1.0, 0.0, 0.0, 1.0)]),
            ),
            ("arcData", arc),
        ],
    );
    let (doc, report, _) = fig_to_doc(&doc_with_shape(ellipse)).unwrap();
    assert_eq!(report.arc_ellipses, 0, "a full disc is not an arc");
    let v = first_vector(&doc);
    assert_eq!(v.path.segments.len(), 6, "full ellipse: M + 4 C + Z");
}

#[test]
fn frame_without_border_imports_no_stroke() {
    // Back-compat / no-false-positive: a plain FRAME (no strokePaints) imports
    // with an empty stroke list and no rounding — counters stay at zero.
    let frame = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("FRAME".into())),
            ("name", KiwiValue::String("Frame".to_owned())),
            ("size", vector(100.0, 60.0)),
            (
                "fillPaints",
                KiwiValue::Array(vec![solid_paint(1.0, 1.0, 1.0, 1.0)]),
            ),
        ],
    );
    let (doc, report, _) = fig_to_doc(&doc_with_shape(frame)).unwrap();
    assert_eq!(report.frames_with_stroke, 0);
    assert_eq!(report.frames_rounded, 0);
    let g = group_named(&doc, "Frame");
    assert!(g.strokes.is_empty());
    assert_eq!(g.corner_radius, None);
    assert_eq!(g.corner_radii, None);
}

#[test]
fn filled_rectangle_without_stroke_paints_does_not_gain_default_black_stroke() {
    let rect = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("RECTANGLE".into())),
            ("size", vector(100.0, 2.0)),
            (
                "fillPaints",
                KiwiValue::Array(vec![solid_paint(0.3, 0.3, 0.3, 1.0)]),
            ),
            ("strokeWeight", KiwiValue::Float(1.0)),
        ],
    );
    let (doc, report, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    assert_eq!(report.strokes_imported, 0);
    assert!(first_vector(&doc).strokes.is_empty());
}

#[test]
fn unfilled_rectangle_with_weight_can_use_default_black_stroke() {
    let rect = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("RECTANGLE".into())),
            ("size", vector(100.0, 50.0)),
            ("strokeWeight", KiwiValue::Float(1.0)),
        ],
    );
    let (doc, report, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    assert_eq!(report.strokes_imported, 1);
    let strokes = &first_vector(&doc).strokes;
    assert_eq!(strokes.len(), 1);
    assert_eq!(strokes[0].paint, Fill::solid(Color::BLACK));
}

#[test]
fn rounded_rect_imports_uniform_corner_radius() {
    let rect = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("ROUNDED_RECTANGLE".into())),
            ("size", vector(100.0, 40.0)),
            (
                "fillPaints",
                KiwiValue::Array(vec![solid_paint(0.0, 1.0, 0.0, 1.0)]),
            ),
            ("cornerRadius", KiwiValue::Float(8.0)),
        ],
    );
    let (doc, _, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    let v = first_vector(&doc);
    assert_eq!(v.corner_radius, Some(8.0));
}

#[test]
fn mixed_per_corner_radii_import_as_independent_corner_radii() {
    // Differing per-corner fields now round-trip as independent `corner_radii`
    // ([TL, TR, BR, BL]) rather than collapsing to the max corner.
    let rect = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("ROUNDED_RECTANGLE".into())),
            ("size", vector(100.0, 40.0)),
            (
                "fillPaints",
                KiwiValue::Array(vec![solid_paint(0.0, 1.0, 0.0, 1.0)]),
            ),
            ("rectangleTopLeftCornerRadius", KiwiValue::Float(4.0)),
            ("rectangleTopRightCornerRadius", KiwiValue::Float(12.0)),
            ("rectangleBottomRightCornerRadius", KiwiValue::Float(6.0)),
            ("rectangleBottomLeftCornerRadius", KiwiValue::Float(0.0)),
        ],
    );
    let (doc, report, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    let v = first_vector(&doc);
    assert_eq!(
        v.corner_radii,
        Some([4.0, 12.0, 6.0, 0.0]),
        "TL, TR, BR, BL order"
    );
    assert_eq!(
        v.corner_radius, None,
        "independent corners do not set the uniform field"
    );
    assert_eq!(
        report.per_corner_radius, 1,
        "counted as one per-corner shape"
    );
}

#[test]
fn equal_per_corner_radii_collapse_to_uniform() {
    // When the four per-corner fields are all equal, collapse to a single uniform
    // radius (no need for an independent array).
    let rect = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("ROUNDED_RECTANGLE".into())),
            ("size", vector(100.0, 40.0)),
            (
                "fillPaints",
                KiwiValue::Array(vec![solid_paint(0.0, 1.0, 0.0, 1.0)]),
            ),
            ("rectangleTopLeftCornerRadius", KiwiValue::Float(7.0)),
            ("rectangleTopRightCornerRadius", KiwiValue::Float(7.0)),
            ("rectangleBottomRightCornerRadius", KiwiValue::Float(7.0)),
            ("rectangleBottomLeftCornerRadius", KiwiValue::Float(7.0)),
        ],
    );
    let (doc, _, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    let v = first_vector(&doc);
    assert_eq!(v.corner_radius, Some(7.0));
    assert_eq!(v.corner_radii, None);
}

#[test]
fn node_opacity_blend_and_drop_shadow_import() {
    let shadow = o(
        "Effect",
        vec![
            ("type", KiwiValue::Enum("DROP_SHADOW".into())),
            ("color", color(0.0, 0.0, 0.0, 0.5)),
            ("radius", KiwiValue::Float(4.0)),
            ("spread", KiwiValue::Float(1.0)),
            ("offset", vector(0.0, 2.0)),
            ("visible", KiwiValue::Bool(true)),
        ],
    );
    let rect = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("RECTANGLE".into())),
            ("size", vector(20.0, 20.0)),
            (
                "fillPaints",
                KiwiValue::Array(vec![solid_paint(1.0, 0.0, 0.0, 1.0)]),
            ),
            ("opacity", KiwiValue::Float(0.4)),
            ("blendMode", KiwiValue::Enum("MULTIPLY".into())),
            ("effects", KiwiValue::Array(vec![shadow])),
        ],
    );
    let (doc, report, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    assert_eq!(report.node_opacity_imported, 1);
    assert_eq!(report.blend_modes_imported, 1);
    assert_eq!(report.effects_imported, 1);
    let n = first_node_with_vector(&doc);
    assert!(
        (n.opacity.get() - 0.4).abs() < 1e-4,
        "node opacity carried, got {}",
        n.opacity.get()
    );
    assert_eq!(n.blend_mode, BlendMode::Multiply);
    assert_eq!(n.effects.len(), 1);
    let s = &n.effects[0];
    assert_eq!(s.kind, ShadowKind::Drop);
    assert_eq!(s.blur, 4.0);
    assert_eq!(s.spread, 1.0);
    assert_eq!(s.offset, [0.0, 2.0]);
    assert_eq!(s.color, Color::rgba(0, 0, 0, 128));
}

#[test]
fn boolean_operation_carries_winding_rule_from_baked_geometry() {
    // A BOOLEAN_OPERATION's baked `fillGeometry` is the already-combined result
    // (e.g. a SUBTRACT bakes an outer + inner contour). The combined PathData
    // must adopt the geometry's `windingRule` (EVEN_ODD here) so the hole renders
    // — this is what makes a boolean node a real combined shape rather than a
    // bbox. Two subpaths (outer square + inner square) with an EVEN_ODD rule.
    let mut outer = Vec::new();
    outer.extend(cmd(1, &[0.0, 0.0]));
    outer.extend(cmd(2, &[10.0, 0.0]));
    outer.extend(cmd(2, &[10.0, 10.0]));
    outer.extend(cmd(2, &[0.0, 10.0]));
    outer.extend(cmd(0, &[]));
    let mut inner = Vec::new();
    inner.extend(cmd(1, &[3.0, 3.0]));
    inner.extend(cmd(2, &[7.0, 3.0]));
    inner.extend(cmd(2, &[7.0, 7.0]));
    inner.extend(cmd(2, &[3.0, 7.0]));
    inner.extend(cmd(0, &[]));
    let fig = doc_from_with_blobs(
        vec![o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("BOOLEAN_OPERATION".into())),
                ("size", vector(10.0, 10.0)),
                (
                    "fillPaints",
                    KiwiValue::Array(vec![solid_paint(0.0, 0.0, 0.0, 1.0)]),
                ),
                (
                    "fillGeometry",
                    KiwiValue::Array(vec![fig_path(0, "ODD"), fig_path(1, "ODD")]),
                ),
            ],
        )],
        vec![outer, inner],
    );
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(
        report.geometry_decoded, 1,
        "boolean decoded its baked geometry"
    );
    let n = doc.scene.get(doc.scene.roots()[0]).unwrap();
    assert_eq!(
        n.meta.get("figma_type").and_then(|v| v.as_str()),
        Some("BOOLEAN_OPERATION"),
        "tagged as a boolean op"
    );
    assert_eq!(
        n.meta.get("geometry").and_then(|v| v.as_str()),
        Some("decoded"),
        "boolean renders the combined shape, not a bbox/children fallback"
    );
    match &n.data {
        NodeData::Vector(v) => {
            assert_eq!(
                v.path.fill_rule,
                FillRule::EvenOdd,
                "even-odd cuts the hole"
            );
            let moves = v
                .path
                .segments
                .iter()
                .filter(|s| matches!(s, PathSegment::Move { .. }))
                .count();
            assert_eq!(moves, 2, "outer + inner contour combined into one path");
        }
        other => panic!("expected vector, got {other:?}"),
    }
}

#[test]
fn layer_and_background_blur_effects_import() {
    // A rect carrying both a FOREGROUND_BLUR (layer) and a BACKGROUND_BLUR must
    // import two doc `Blur`s of the right kind/radius, while a SHADOW alongside
    // them still goes to `effects` (the two lists stay disjoint).
    let layer_blur = o(
        "Effect",
        vec![
            ("type", KiwiValue::Enum("FOREGROUND_BLUR".into())),
            ("radius", KiwiValue::Float(8.0)),
            ("visible", KiwiValue::Bool(true)),
        ],
    );
    let bg_blur = o(
        "Effect",
        vec![
            ("type", KiwiValue::Enum("BACKGROUND_BLUR".into())),
            ("radius", KiwiValue::Float(12.0)),
            ("visible", KiwiValue::Bool(true)),
        ],
    );
    // An invisible blur and a zero-radius blur must both be skipped.
    let hidden_blur = o(
        "Effect",
        vec![
            ("type", KiwiValue::Enum("LAYER_BLUR".into())),
            ("radius", KiwiValue::Float(20.0)),
            ("visible", KiwiValue::Bool(false)),
        ],
    );
    let zero_blur = o(
        "Effect",
        vec![
            ("type", KiwiValue::Enum("BACKGROUND_BLUR".into())),
            ("radius", KiwiValue::Float(0.0)),
            ("visible", KiwiValue::Bool(true)),
        ],
    );
    let shadow = o(
        "Effect",
        vec![
            ("type", KiwiValue::Enum("DROP_SHADOW".into())),
            ("color", color(0.0, 0.0, 0.0, 0.5)),
            ("radius", KiwiValue::Float(4.0)),
            ("spread", KiwiValue::Float(0.0)),
            ("offset", vector(0.0, 2.0)),
            ("visible", KiwiValue::Bool(true)),
        ],
    );
    let rect = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("RECTANGLE".into())),
            ("size", vector(20.0, 20.0)),
            (
                "fillPaints",
                KiwiValue::Array(vec![solid_paint(1.0, 0.0, 0.0, 1.0)]),
            ),
            (
                "effects",
                KiwiValue::Array(vec![layer_blur, bg_blur, hidden_blur, zero_blur, shadow]),
            ),
        ],
    );
    let (doc, report, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    assert_eq!(
        report.blurs_imported, 2,
        "two visible non-zero blurs imported"
    );
    assert_eq!(
        report.effects_imported, 1,
        "the drop shadow still lands in effects"
    );
    let n = first_node_with_vector(&doc);
    assert_eq!(n.blurs.len(), 2);
    assert_eq!(
        n.blurs[0],
        Blur {
            kind: BlurKind::Layer,
            radius: 8.0
        }
    );
    assert_eq!(
        n.blurs[1],
        Blur {
            kind: BlurKind::Background,
            radius: 12.0
        }
    );
    assert_eq!(n.effects.len(), 1, "shadow effect is separate from blurs");
}

#[test]
fn image_paint_without_a_hash_is_dropped_not_a_phantom_fill() {
    // An IMAGE paint that carries no `image.hash` (malformed / hash-less) can't
    // reference a bitmap, so it's skipped rather than producing a fill that
    // would resolve to nothing. Must not panic, and leaves the node fill-less.
    let image_paint = o(
        "Paint",
        vec![
            ("type", KiwiValue::Enum("IMAGE".into())),
            ("opacity", KiwiValue::Float(1.0)),
        ],
    );
    let rect = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("RECTANGLE".into())),
            ("size", vector(50.0, 50.0)),
            ("fillPaints", KiwiValue::Array(vec![image_paint])),
        ],
    );
    let (doc, report, assets) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    assert_eq!(
        report.images_imported, 0,
        "a hash-less image paint is not counted"
    );
    assert!(assets.is_empty());
    let v = first_vector(&doc);
    assert!(
        v.fills.is_empty(),
        "no fill for an unreferenceable image paint"
    );
}

#[test]
fn invisible_paint_is_skipped() {
    let mut hidden = solid_paint(1.0, 0.0, 0.0, 1.0);
    if let KiwiValue::Object { fields, .. } = &mut hidden {
        fields.insert("visible".into(), KiwiValue::Bool(false));
    }
    let rect = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("RECTANGLE".into())),
            ("size", vector(10.0, 10.0)),
            (
                "fillPaints",
                KiwiValue::Array(vec![hidden, solid_paint(0.0, 1.0, 0.0, 1.0)]),
            ),
        ],
    );
    let (doc, _, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    let v = first_vector(&doc);
    // Only the visible (green) paint survives, as the bottom-most fill.
    assert_eq!(v.fills.len(), 1);
    assert_eq!(v.fills[0], Fill::solid(Color::rgba(0, 255, 0, 255)));
}

#[test]
fn drop_shadow_show_behind_node_imports() {
    // Figma's default is `false` (the drop shadow is knocked out under the
    // node's own body); an explicit `true` paints it behind the whole node.
    let shadow_with = |behind: Option<bool>| {
        let mut fields = vec![
            ("type", KiwiValue::Enum("DROP_SHADOW".into())),
            ("color", color(0.0, 0.0, 0.0, 0.5)),
            ("radius", KiwiValue::Float(4.0)),
            ("offset", vector(0.0, 2.0)),
            ("visible", KiwiValue::Bool(true)),
        ];
        if let Some(b) = behind {
            fields.push(("showShadowBehindNode", KiwiValue::Bool(b)));
        }
        o("Effect", fields)
    };
    for (behind_field, expected) in [(Some(true), true), (Some(false), false), (None, false)] {
        let rect = o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("RECTANGLE".into())),
                ("size", vector(20.0, 20.0)),
                (
                    "fillPaints",
                    KiwiValue::Array(vec![solid_paint(1.0, 0.0, 0.0, 0.5)]),
                ),
                ("effects", KiwiValue::Array(vec![shadow_with(behind_field)])),
            ],
        );
        let (doc, _, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
        let n = first_node_with_vector(&doc);
        assert_eq!(
            n.effects[0].show_behind_node, expected,
            "showShadowBehindNode={behind_field:?} imports as {expected}"
        );
    }
}
