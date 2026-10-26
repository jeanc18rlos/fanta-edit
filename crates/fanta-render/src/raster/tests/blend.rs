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
    blended_pixel(
        Color::rgb(200, 100, 50),
        Color::rgb(100, 200, 255),
        top_blend,
    )
}

/// The overlap pixel of a `top` rect blended by `top_blend` over an opaque
/// `bottom` rect that covers the canvas.
fn blended_pixel(bottom: Color, top: Color, top_blend: BlendMode) -> [u8; 4] {
    const N: u32 = 64;
    let mut doc = Doc::new();
    // Bottom: covers the whole canvas (world (0,0) → center).
    doc.apply(Operation::create_node(solid_rect(
        -32.0,
        -32.0,
        64.0,
        64.0,
        bottom,
        BlendMode::Normal,
        IndexKey::FIRST,
    )))
    .unwrap();
    // Top: over the same region (higher z), blended per `top_blend`.
    doc.apply(Operation::create_node(solid_rect(
        -16.0,
        -16.0,
        32.0,
        32.0,
        top,
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

#[test]
fn linear_burn_and_dodge_add_the_channels() {
    // Bottom orange (200, 100, 50), top light blue (100, 200, 255).
    // Linear Burn: max(0, s + d - 255) = (45, 45, 50).
    let burn = overlap_pixel(BlendMode::LinearBurn);
    for (channel, expected) in [(0, 45), (1, 45), (2, 50)] {
        assert!(
            (i32::from(burn[channel]) - expected).abs() <= 2,
            "Linear Burn channel {channel}: expected ≈{expected}, got {burn:?}"
        );
    }
    assert_eq!(burn[3], 255);
    // Linear Dodge: min(255, s + d) saturates every channel here.
    let dodge = overlap_pixel(BlendMode::LinearDodge);
    assert!(dodge[..3].iter().all(|&value| value >= 253), "{dodge:?}");
}

#[test]
fn linear_burn_composites_a_translucent_source_by_its_alpha() {
    const N: u32 = 64;
    let mut doc = Doc::new();
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
    doc.apply(Operation::create_node(solid_rect(
        -16.0,
        -16.0,
        32.0,
        32.0,
        Color::rgba(100, 200, 255, 128),
        BlendMode::LinearBurn,
        IndexKey::after(IndexKey::FIRST),
    )))
    .unwrap();
    let mut r = RasterRenderer::new(N, N).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let pixel = rgba_at(&r.copy_rgba(), N, N / 2, N / 2);
    // W3C separable compositing over an opaque backdrop: co = d·(1 - as) + as·B
    // with B = (45, 45, 50) and as ≈ 0.5, so ≈ (122, 72, 50), still opaque. A
    // plain `src + dst - 1` blend would also burn the alpha.
    for (channel, expected) in [(0, 122), (1, 72), (2, 50)] {
        assert!(
            (i32::from(pixel[channel]) - expected).abs() <= 3,
            "channel {channel}: expected ≈{expected}, got {pixel:?}"
        );
    }
    assert_eq!(pixel[3], 255);
}

#[test]
fn photo_editor_modes_follow_their_formulas() {
    let orange = Color::rgb(200, 100, 50);
    let light_blue = Color::rgb(100, 200, 255);
    let violet = Color::rgb(100, 50, 200);
    let green = Color::rgb(0, 200, 0);
    let blue = Color::rgb(0, 0, 255);
    let magenta = Color::rgb(200, 0, 50);
    for (mode, bottom, top, expected) in [
        // Every expected pixel differs from the source color, so a mode drawn
        // as Normal fails; these four keep the backdrop. The color with the
        // lower channel sum (350 < 555), unlike Darken's per-channel minimum
        // (100, 100, 50).
        (BlendMode::DarkerColor, orange, light_blue, [200, 100, 50]),
        // The higher channel sum, unlike Lighten's (200, 200, 255).
        (BlendMode::LighterColor, light_blue, orange, [100, 200, 255]),
        // By channel sum (200 < 255), not luminosity, which ranks blue darker.
        (BlendMode::DarkerColor, green, blue, [0, 200, 0]),
        (BlendMode::LighterColor, blue, green, [0, 0, 255]),
        // Red burns by 2·s (1 - 55/200), green dodges by 2·s - 1
        // (100/110), and blue dodges by 1.
        (BlendMode::VividLight, orange, light_blue, [185, 232, 255]),
        // d + 2·s - 1, clamped.
        (BlendMode::LinearLight, orange, light_blue, [145, 245, 255]),
        // min(d, 2·s) for red, max(d, 2·s - 1) for green and blue.
        (BlendMode::PinLight, orange, light_blue, [200, 145, 255]),
        // 1 where s + d reaches 1: only red (300 of 255).
        (BlendMode::HardMix, orange, violet, [255, 0, 0]),
        (BlendMode::Subtract, orange, light_blue, [100, 0, 0]),
        (BlendMode::Divide, orange, light_blue, [255, 128, 50]),
        // A black source turns a lit backdrop channel white and keeps a black one.
        (BlendMode::Divide, magenta, Color::BLACK, [255, 0, 255]),
    ] {
        let pixel = blended_pixel(bottom, top, mode);
        for (channel, expected) in expected.into_iter().enumerate() {
            assert!(
                (i32::from(pixel[channel]) - expected).abs() <= 3,
                "{mode:?} channel {channel}: expected ≈{expected}, got {pixel:?}"
            );
        }
        assert_eq!(pixel[3], 255, "{mode:?}");
    }
}
