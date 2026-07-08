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
            ..
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
                    handles,
                    stops,
                },
            ..
        }) => {
            assert!((center[0] - 0.5).abs() < 1e-3 && (center[1] - 0.5).abs() < 1e-3);
            assert!(*radius > 0.0);
            assert_eq!(
                *handles, None,
                "an axis-aligned isotropic radial carries no axis handles"
            );
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
            ..
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
                    handles: _,
                    stops,
                },
            ..
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
            ..
        }) => {
            // 1.0 alpha * 0.5 paint opacity ≈ 128.
            assert_eq!(stops[0].color.a, 128, "stop alpha folds in paint opacity");
        }
        other => panic!("expected gradient, got {other:?}"),
    }
}

#[test]
fn rotated_radial_gradient_keeps_axis_handles() {
    // A rotated (45°) radial: the gradient transform carries off-diagonal
    // terms, so the scalar center+radius form loses the rotation and the
    // second-axis radius. The importer must keep the two axis-handle
    // positions (gradient-space (1,0.5) and (0.5,1) mapped into node space).
    // Inverse of a 45° rotation about (0.5, 0.5) — forward == inverse^T here;
    // this matrix maps node space → gradient space.
    let c = std::f32::consts::FRAC_1_SQRT_2;
    let tx = 0.5 - c * 0.5 - c * 0.5;
    let ty = 0.5 + c * 0.5 - c * 0.5;
    let m = [c, c, tx, -c, c, ty];
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
    let (doc, _, _) = fig_to_doc(&doc_with_shape(rect)).unwrap();
    let v = first_vector(&doc);
    match v.fills.first() {
        Some(Fill::Gradient {
            gradient: Gradient::Radial {
                center, handles, ..
            },
            ..
        }) => {
            assert!((center[0] - 0.5).abs() < 1e-3 && (center[1] - 0.5).abs() < 1e-3);
            let [x_end, y_end] = handles.expect("rotated radial keeps handles");
            // The primary axis points along the rotated +x direction from the
            // center; both handles sit half a unit from the center.
            let dx = (x_end[0] - 0.5, x_end[1] - 0.5);
            let dy = (y_end[0] - 0.5, y_end[1] - 0.5);
            let len = |v: (f32, f32)| (v.0 * v.0 + v.1 * v.1).sqrt();
            assert!((len(dx) - 0.5).abs() < 1e-3, "|x axis| = 0.5, got {dx:?}");
            assert!((len(dy) - 0.5).abs() < 1e-3, "|y axis| = 0.5, got {dy:?}");
            assert!(
                dx.1.abs() > 0.3,
                "rotated x-axis has a significant y component: {dx:?}"
            );
        }
        other => panic!("expected radial gradient fill, got {other:?}"),
    }
}
