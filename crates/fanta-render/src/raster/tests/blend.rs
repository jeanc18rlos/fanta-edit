//! Per-paint blend modes: a fill's `blend` composites the paint against what is
//! already on the canvas (the paints below it and the backdrop), so a Multiply
//! solid over a colored node darkens the overlap rather than covering it.

use super::*;
use fanta_doc::{BlendMode, Color, Doc, Fill, IndexKey, Operation, PathData, VectorNode};

/// A solid-filled rect node with an explicit per-paint blend mode and z-index.
fn solid_rect(
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    color: Color,
    blend: BlendMode,
    z: IndexKey,
) -> CanvasNode {
    let mut node = CanvasNode::new(NodeData::Vector(VectorNode {
        path: PathData::rect(x, y, w, h),
        fills: smallvec_of(Fill::Solid { color, blend }),
        strokes: Default::default(),
        corner_radius: None,
        corner_radii: None,
        corner_smoothing: 0.0,
        local_size: None,
        parametric: None,
    }));
    node.index = z;
    node
}

/// Render a bottom orange rect with a top light-blue rect over it, using
/// `top_blend` for the top rect's fill. Returns the RGBA of the overlap pixel.
fn overlap_pixel(top_blend: BlendMode) -> [u8; 4] {
    const N: u32 = 64;
    let mut doc = Doc::new();
    // Bottom: opaque orange covering the whole canvas (world (0,0) → center).
    doc.apply(Operation::create_node(solid_rect(
        -32.0,
        -32.0,
        64.0,
        64.0,
        Color::rgb(200, 100, 50),
        BlendMode::Normal,
        IndexKey::FIRST,
    )))
    .unwrap();
    // Top: light blue over the same region (higher z), blended per `top_blend`.
    doc.apply(Operation::create_node(solid_rect(
        -16.0,
        -16.0,
        32.0,
        32.0,
        Color::rgb(100, 200, 255),
        top_blend,
        IndexKey::after(IndexKey::FIRST),
    )))
    .unwrap();

    let mut r = RasterRenderer::new(N, N).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();
    rgba_at(&buf, N, N / 2, N / 2)
}

#[test]
fn multiply_solid_fill_darkens_the_backdrop() {
    let normal = overlap_pixel(BlendMode::Normal);
    let multiply = overlap_pixel(BlendMode::Multiply);

    // Normal simply shows the top light-blue paint.
    assert!(
        normal[2] > 220,
        "Normal overlap should be the light-blue top paint (blue≈255), got {normal:?}"
    );

    // Multiply = top × bottom / 255 per channel:
    //   R: 100·200/255 ≈ 78,  G: 200·100/255 ≈ 78,  B: 255·50/255 = 50.
    // The blue channel is the clean discriminator: 50 (multiply) vs 255 (normal).
    assert!(
        multiply[2] < 90,
        "Multiply overlap should darken toward the product (blue≈50), got {multiply:?}"
    );
    assert!(
        (multiply[0] as i32 - 78).abs() < 20 && (multiply[1] as i32 - 78).abs() < 20,
        "Multiply overlap should be ~(78,78,50), got {multiply:?}"
    );
    assert!(
        multiply[2] + 100 < normal[2],
        "Multiply must differ from Normal — the per-paint blend field is not applied \
         (multiply {multiply:?} vs normal {normal:?})"
    );
}
