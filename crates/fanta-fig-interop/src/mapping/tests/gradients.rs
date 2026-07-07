//! Gradient fills: linear, radial, angular, diamond, and per-paint opacity.

use super::*;

// =============================================================================
// Element-fidelity: gradients, strokes, per-corner radius, opacity, effects
// =============================================================================

#[test]
fn linear_gradient_fill_maps_to_fill_gradient_with_endpoints_and_stops() {
    // A 90° rotation transform (the common top→bottom gradient). Inverting it and
    // applying to the canonical handles (0,0)/(1,0) yields a vertical gradient.
    let m = [0.0, 1.0, 0.0, -1.0, 0.0, 1.0];
    let rect = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("RECTANGLE".to_owned())),
            ("size", vector(100.0, 100.0)),
            (
                "fillPaints",
                KiwiValue::Array(vec![gradient_paint(
                    "GRADIENT_LINEAR",
                    m,
                    vec![
                        grad_stop(0.0, 1.0, 0.0, 0.0, 1.0),
                        grad_stop(1.0, 0.0, 0.0, 1.0, 1.0),
                    ],
                )]),
            ),
        ],
    );
    let (doc, report, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    assert_eq!(report.gradients_imported, 1, "one gradient imported");
    let v = first_vector(&doc);
    match v.fills.first() {
        Some(Fill::Gradient {
            gradient: Gradient::Linear { start, end, stops },
        }) => {
            // Inverse of a +90° rotation maps (0,0)→(1,0) and (1,0)→(1,1): a
            // vertical run down the right edge.
            assert!((start[1] - 0.0).abs() < 1e-3, "start y ~0, got {start:?}");
            assert!((end[1] - 1.0).abs() < 1e-3, "end y ~1, got {end:?}");
            assert_eq!(stops.len(), 2);
            assert_eq!(stops[0].color, Color::rgba(255, 0, 0, 255));
            assert_eq!(stops[1].color, Color::rgba(0, 0, 255, 255));
            assert_eq!(stops[0].position, 0.0);
            assert_eq!(stops[1].position, 1.0);
        }
        other => panic!("expected linear gradient fill, got {other:?}"),
    }
    doc.scene.validate().unwrap();
}

#[test]
fn radial_gradient_fill_maps_to_fill_gradient_radial() {
    // Identity transform: center at (0.5,0.5), radius ~0.5.
    let m = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let rect = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("ELLIPSE".to_owned())),
            ("size", vector(80.0, 80.0)),
            (
                "fillPaints",
                KiwiValue::Array(vec![gradient_paint(
                    "GRADIENT_RADIAL",
                    m,
                    vec![
                        grad_stop(0.0, 1.0, 1.0, 1.0, 1.0),
                        grad_stop(1.0, 0.0, 0.0, 0.0, 1.0),
                    ],
                )]),
            ),
        ],
    );
    let (doc, report, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    assert_eq!(report.gradients_imported, 1);
    let v = first_vector(&doc);
    match v.fills.first() {
        Some(Fill::Gradient {
            gradient:
                Gradient::Radial {
                    center,
                    radius,
                    stops,
                },
        }) => {
            assert!((center[0] - 0.5).abs() < 1e-3 && (center[1] - 0.5).abs() < 1e-3);
            assert!(*radius > 0.0);
            assert_eq!(stops.len(), 2);
        }
        other => panic!("expected radial gradient fill, got {other:?}"),
    }
}

#[test]
fn angular_gradient_fill_maps_to_fill_gradient_angular() {
    // Identity transform: center at (0.5,0.5); a conic sweep with start angle 0.
    let m = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let rect = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("ELLIPSE".to_owned())),
            ("size", vector(80.0, 80.0)),
            (
                "fillPaints",
                KiwiValue::Array(vec![gradient_paint(
                    "GRADIENT_ANGULAR",
                    m,
                    vec![
                        grad_stop(0.0, 1.0, 1.0, 1.0, 1.0),
                        grad_stop(1.0, 0.0, 0.0, 0.0, 1.0),
                    ],
                )]),
            ),
        ],
    );
    let (doc, report, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    assert_eq!(report.gradients_imported, 1);
    assert_eq!(report.conic_gradients_imported, 1, "counted as conic");
    let v = first_vector(&doc);
    match v.fills.first() {
        Some(Fill::Gradient {
            gradient: Gradient::Angular { center, stops, .. },
        }) => {
            assert!((center[0] - 0.5).abs() < 1e-3 && (center[1] - 0.5).abs() < 1e-3);
            assert_eq!(stops.len(), 2);
        }
        other => panic!("expected angular gradient fill, got {other:?}"),
    }
}

#[test]
fn diamond_gradient_fill_maps_to_fill_gradient_diamond() {
    let m = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let rect = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("RECTANGLE".to_owned())),
            ("size", vector(80.0, 80.0)),
            (
                "fillPaints",
                KiwiValue::Array(vec![gradient_paint(
                    "GRADIENT_DIAMOND",
                    m,
                    vec![
                        grad_stop(0.0, 1.0, 1.0, 1.0, 1.0),
                        grad_stop(1.0, 0.0, 0.0, 0.0, 1.0),
                    ],
                )]),
            ),
        ],
    );
    let (doc, report, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    assert_eq!(report.gradients_imported, 1);
    assert_eq!(report.diamond_gradients_imported, 1, "counted as diamond");
    let v = first_vector(&doc);
    match v.fills.first() {
        Some(Fill::Gradient {
            gradient:
                Gradient::Diamond {
                    center,
                    radius,
                    stops,
                },
        }) => {
            assert!((center[0] - 0.5).abs() < 1e-3 && (center[1] - 0.5).abs() < 1e-3);
            assert!(*radius > 0.0);
            assert_eq!(stops.len(), 2);
        }
        other => panic!("expected diamond gradient fill, got {other:?}"),
    }
}

#[test]
fn per_paint_opacity_multiplies_gradient_stop_alpha() {
    let m = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let mut paint = gradient_paint(
        "GRADIENT_LINEAR",
        m,
        vec![
            grad_stop(0.0, 1.0, 1.0, 1.0, 1.0),
            grad_stop(1.0, 1.0, 1.0, 1.0, 1.0),
        ],
    );
    if let KiwiValue::Object { fields, .. } = &mut paint {
        fields.insert("opacity".to_owned(), KiwiValue::Float(0.5));
    }
    let rect = o(
        "NodeChange",
        vec![
            ("guid", guid(0, 2)),
            ("parentIndex", parent_index(0, 1)),
            ("type", KiwiValue::Enum("RECTANGLE".to_owned())),
            ("size", vector(10.0, 10.0)),
            ("fillPaints", KiwiValue::Array(vec![paint])),
        ],
    );
    let (doc, _, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    let v = first_vector(&doc);
    match v.fills.first() {
        Some(Fill::Gradient {
            gradient: Gradient::Linear { stops, .. },
        }) => {
            // 1.0 alpha * 0.5 paint opacity ≈ 128.
            assert_eq!(stops[0].color.a, 128, "stop alpha folds in paint opacity");
        }
        other => panic!("expected gradient, got {other:?}"),
    }
}
