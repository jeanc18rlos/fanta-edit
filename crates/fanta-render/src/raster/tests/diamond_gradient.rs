//! DIAMOND-gradient render fidelity: the iso-distance contours must be real
//! axis-aligned diamonds (L1 / Manhattan distance), not the circles/ellipses of
//! a radial gradient.
//!
//! The discriminator is geometric. Take two probe points at the *same* euclidean
//! distance `rho` from the gradient center: an AXIS point `center + (rho, 0)` has
//! L1 distance `rho`, while a DIAGONAL point `center + (rho/√2, rho/√2)` has L1
//! distance `rho·√2` (≈ 1.414·rho). Both lie on one circle, so a radial gradient
//! colors them *identically*. A true diamond colors the diagonal point noticeably
//! darker (larger L1 → larger `t` → closer to the edge color). That single
//! inequality is exactly what the old rotate-the-radial approximation could never
//! satisfy, so this test fails against the buggy code and passes against the fix.

use super::*;
use fanta_doc::{Doc, GradientStop, Operation, PathData, VectorNode};

/// White-center → black-edge diamond gradient on a square rect centered at the
/// world origin. `half` is the rect half-extent in world units.
fn diamond_doc(half: f64) -> Doc {
    let gradient = fanta_doc::Gradient::Diamond {
        center: [0.5, 0.5],
        radius: 0.5,
        handles: None,
        stops: vec![
            GradientStop {
                position: 0.0,
                color: Color::rgb(255, 255, 255), // center: white
            },
            GradientStop {
                position: 1.0,
                color: Color::rgb(0, 0, 0), // edge: black
            },
        ],
    };
    let node = VectorNode {
        path: PathData::rect(-half, -half, half * 2.0, half * 2.0),
        fills: smallvec_of(Fill::Gradient {
            gradient,
            blend: fanta_doc::BlendMode::Normal,
        }),
        strokes: Default::default(),
        corner_radius: None,
        corner_radii: None,
        corner_smoothing: 0.0,
        local_size: None,
        parametric: None,
    };
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(CanvasNode::new(NodeData::Vector(
        node,
    ))))
    .unwrap();
    doc
}

/// Luminance proxy (the gradient is grayscale white→black, so the red channel
/// tracks `1 - t`).
fn luma(buf: &[u8], width: u32, x: u32, y: u32) -> u8 {
    rgba_at(buf, width, x, y)[0]
}

#[test]
fn diamond_gradient_has_diamond_iso_contours_not_circular() {
    // Canvas + node geometry. At zoom 1, world (0,0) lands at the canvas center
    // and world units map 1:1 to pixels, so the rect spans pixels
    // (cx-half .. cx+half) in both axes.
    const N: u32 = 96;
    let cx = (N / 2) as i32;
    let cy = (N / 2) as i32;
    let half = 36.0_f64;

    let doc = diamond_doc(half);
    let mut r = RasterRenderer::new(N, N).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    // Euclidean probe distance from center (in pixels). Stay well inside the
    // rect and below the diamond clamp on the axis (axis L1 = rho/half < 1).
    let rho = 18.0_f64;
    let diag = (rho / std::f64::consts::SQRT_2).round() as i32;
    let axis = rho.round() as i32;

    let center_l = luma(&buf, N, cx as u32, cy as u32);

    // Axis probes (East / North): L1 distance == rho.
    let east = luma(&buf, N, (cx + axis) as u32, cy as u32);
    let north = luma(&buf, N, cx as u32, (cy - axis) as u32);

    // Diagonal probes (NE / SW): same euclidean rho, but L1 == rho·√2.
    let ne = luma(&buf, N, (cx + diag) as u32, (cy - diag) as u32);
    let sw = luma(&buf, N, (cx - diag) as u32, (cy + diag) as u32);

    // Sanity: the gradient actually runs (center near white, axis darker).
    assert!(
        center_l > 230,
        "center should be ~white, got luma {center_l}"
    );
    assert!(
        east < center_l && north < center_l,
        "axis points must be darker than center (east {east}, north {north}, center {center_l})"
    );

    // THE DIAMOND TEST: on the same euclidean circle, the diagonal (larger L1)
    // must be clearly DARKER than the axis point. A radial/elliptical gradient
    // colors them equal — this inequality is the diamond signature and is what
    // the old rotated-radial approximation fails.
    let margin = 25_i32; // far above AA noise; circle would give ~0 difference.
    assert!(
        (east as i32) - (ne as i32) > margin,
        "DIAMOND broken: NE diagonal ({ne}) not darker than E axis ({east}) at equal \
         euclidean radius — this is the circular-radial bug"
    );
    assert!(
        (north as i32) - (sw as i32) > margin,
        "DIAMOND broken: SW diagonal ({sw}) not darker than N axis ({north}) at equal \
         euclidean radius"
    );

    // Iso-contour symmetry: the four axis edge-midpoints share one L1 distance,
    // so they must share a color (a real diamond is symmetric under the axes).
    let west = luma(&buf, N, (cx - axis) as u32, cy as u32);
    let south = luma(&buf, N, cx as u32, (cy + axis) as u32);
    let axis_spread = [east, west, north, south]
        .iter()
        .map(|&v| v as i32)
        .collect::<Vec<_>>();
    let amax = *axis_spread.iter().max().unwrap();
    let amin = *axis_spread.iter().min().unwrap();
    assert!(
        amax - amin <= 12,
        "the four axis edge-midpoints lie on one diamond iso-contour and must \
         share a color: E{east} W{west} N{north} S{south}"
    );
}
