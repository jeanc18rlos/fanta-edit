//! Regression tests for the Figma-fidelity doc fields the renderer consumes:
//! metric-relative line height, vector corner smoothing, drop-shadow knockout
//! (`show_behind_node`), image-fill tile scale / rotation / per-paint blend,
//! gradient axis handles, per-subpath fill rules, text truncation, paragraph
//! spacing/indent, and `ISOLATED_BLEND` containers.
use super::*;
use fanta_doc::{Doc, GradientStop, Operation, TextNode, VectorNode};
use std::sync::Arc as StdArc;

fn render_doc(doc: &Doc, side: u32) -> Vec<u8> {
    let mut renderer = RasterRenderer::new(side, side).unwrap();
    renderer.render(&doc.scene, &doc.viewport);
    renderer.copy_rgba()
}

// -----------------------------------------------------------------------
// 1. Metric-relative ("auto") line height
// -----------------------------------------------------------------------

#[test]
fn line_height_auto_percent_keys_the_cache_and_overrides_the_scalar() {
    clear_layout_cache();
    let mut auto = TextNode::new("Line one\nLine two", 400.0, 100.0);
    auto.style.line_height = 1.0;
    auto.style.line_height_auto_percent = Some(200.0);
    let mut scalar = auto.clone();
    scalar.style.line_height_auto_percent = None;

    let (_, auto_height) = measure_text_node(&auto);
    let (_, scalar_height) = measure_text_node(&scalar);
    // Two nodes identical except for the auto-percent MUST occupy two cache
    // slots — otherwise the second render paints the first one's layout.
    assert_eq!(
        layout_cache_len(),
        2,
        "line_height_auto_percent must be part of the shape cache key"
    );
    // 200% of Inter's intrinsic metric height (≈1.21 em) is far taller than
    // the 1.0-em scalar the node would otherwise lay out at.
    assert!(
        auto_height > scalar_height * 1.5,
        "metric-relative 200% must out-measure the 1.0 scalar: auto={auto_height} scalar={scalar_height}"
    );
}

// -----------------------------------------------------------------------
// 2. Vector corner smoothing (squircle)
// -----------------------------------------------------------------------

#[test]
fn vector_corner_smoothing_bulges_the_rounded_corner() {
    // Rect world [−50,−50]..[50,50], radius 40; TL rounding pivot at (−10,−10).
    // World (−41,−41) is 43.8 from the pivot — OUTSIDE the circular r=40 arc —
    // but inside the 0.6-smoothed superellipse (whose 45° point sits ~48 out).
    fn corner_pixel(smoothing: f32) -> [u8; 4] {
        let mut vector = VectorNode::rect_solid(-50.0, -50.0, 100.0, 100.0, Color::rgb(0, 0, 255));
        vector.corner_radius = Some(40.0);
        vector.corner_smoothing = smoothing;
        let mut doc = Doc::new();
        doc.apply(Operation::create_node(CanvasNode::new(NodeData::Vector(
            vector,
        ))))
        .unwrap();
        let buf = render_doc(&doc, 128);
        // world (−41,−41) → screen (23, 23) on the 128² origin-centred surface.
        rgba_at(&buf, 128, 23, 23)
    }
    let circular = corner_pixel(0.0);
    let smoothed = corner_pixel(0.6);
    assert!(
        circular[3] < 40,
        "without smoothing the sample sits outside the circular corner: {circular:?}"
    );
    assert!(
        smoothed[3] > 200 && smoothed[2] > 200,
        "0.6 smoothing must bulge the corner outward over the sample: {smoothed:?}"
    );
}

// -----------------------------------------------------------------------
// 3. Drop-shadow knockout (`show_behind_node = false`)
// -----------------------------------------------------------------------

#[test]
fn drop_shadow_is_knocked_out_under_the_node_unless_shown_behind() {
    // A 50%-alpha red rect with a crisp black shadow that lands underneath it
    // (offset small enough that the centre is covered by body AND shadow).
    fn center_pixel(show_behind_node: bool) -> [u8; 4] {
        let mut node = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            -20.0,
            -20.0,
            40.0,
            40.0,
            Color::rgba(255, 0, 0, 128),
        )));
        node.effects.push(Shadow {
            kind: ShadowKind::Drop,
            color: Color::rgb(0, 0, 0),
            blur: 0.0,
            spread: 0.0,
            offset: [2.0, 0.0],
            show_behind_node,
        });
        let mut doc = Doc::new();
        doc.apply(Operation::create_node(node)).unwrap();
        let buf = render_doc(&doc, 64);
        rgba_at(&buf, 64, 32, 32)
    }
    let behind = center_pixel(true);
    let knocked_out = center_pixel(false);
    // Shown behind: the black shadow bleeds through the translucent body —
    // darker red, higher combined alpha.
    // Knocked out: the shadow under the body is (mostly) removed — the pixel
    // is closer to the bare translucent red: brighter and less opaque.
    assert!(
        knocked_out[0] > behind[0] + 20,
        "knockout must brighten the red under the translucent body: knocked={knocked_out:?} behind={behind:?}"
    );
    assert!(
        behind[3] > knocked_out[3] + 20,
        "showing the shadow behind must add coverage under the body: knocked={knocked_out:?} behind={behind:?}"
    );
}

// -----------------------------------------------------------------------
// 4 + 5. Image fill: tile scale and quarter-turn rotation
// -----------------------------------------------------------------------

/// A rect vector at world `[−16,−16, 32, 32]` filled with the given image fill.
fn image_fill_doc(fill: Fill) -> Doc {
    let mut vector = VectorNode::rect_solid(-16.0, -16.0, 32.0, 32.0, Color::rgb(0, 0, 0));
    vector.fills = smallvec_of(fill);
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(CanvasNode::new(NodeData::Vector(
        vector,
    ))))
    .unwrap();
    doc
}

#[test]
fn tile_scale_draws_tiles_at_natural_times_scale() {
    use crate::asset::{DecodedImage, InMemoryAssetResolver};
    // 2×2 quadrant image: TL red, TR green, BL blue, BR white.
    let px: Vec<u8> = vec![
        255, 0, 0, 255, 0, 255, 0, 255, //
        0, 0, 255, 255, 255, 255, 255, 255,
    ];
    let mut resolver = InMemoryAssetResolver::new();
    let asset = fanta_doc::AssetId::new();
    resolver.insert(asset, DecodedImage::new(StdArc::new(px), 2, 2));
    let resolver = StdArc::new(resolver);

    let pixel_at = |scale: Option<f32>| -> [u8; 4] {
        let doc = image_fill_doc(Fill::Image {
            asset,
            mode: fanta_doc::ImageFitMode::Tile,
            opacity: 1.0,
            crop: None,
            scale,
            rotation: None,
            blend: fanta_doc::BlendMode::Normal,
        });
        let mut renderer = RasterRenderer::new(64, 64).unwrap();
        renderer.set_asset_resolver(resolver.clone());
        renderer.render(&doc.scene, &doc.viewport);
        let buf = renderer.copy_rgba();
        // Fill-local (3, 3) → screen (19, 19) (rect origin at screen (16, 16)).
        rgba_at(&buf, 64, 19, 19)
    };
    // Native tiles: local (3,3) samples image pixel (3 mod 2, 3 mod 2) = the
    // white bottom-right texel.
    let native = pixel_at(None);
    assert!(
        native[0] > 200 && native[1] > 200 && native[2] > 200,
        "native tiling at (3,3) lands on the white texel: {native:?}"
    );
    // Scaled ×8: one tile spans 16px, so (3,3) is still inside the red
    // top-left texel of the first tile.
    let scaled = pixel_at(Some(8.0));
    assert!(
        scaled[0] > 200 && scaled[1] < 90 && scaled[2] < 90,
        "×8 tiles keep (3,3) inside the red texel: {scaled:?}"
    );
}

#[test]
fn image_rotation_quarter_turns_the_fill() {
    use crate::asset::{DecodedImage, InMemoryAssetResolver};
    // 2×1 image: left red, right blue.
    let px: Vec<u8> = vec![255, 0, 0, 255, 0, 0, 255, 255];
    let mut resolver = InMemoryAssetResolver::new();
    let asset = fanta_doc::AssetId::new();
    resolver.insert(asset, DecodedImage::new(StdArc::new(px), 2, 1));
    let resolver = StdArc::new(resolver);

    let render = |rotation: Option<f32>| -> Vec<u8> {
        let doc = image_fill_doc(Fill::Image {
            asset,
            mode: fanta_doc::ImageFitMode::Stretch,
            opacity: 1.0,
            crop: None,
            scale: None,
            rotation,
            blend: fanta_doc::BlendMode::Normal,
        });
        let mut renderer = RasterRenderer::new(64, 64).unwrap();
        renderer.set_asset_resolver(resolver.clone());
        renderer.render(&doc.scene, &doc.viewport);
        renderer.copy_rgba()
    };
    // Unrotated: red left half, blue right half.
    let plain = render(None);
    let left = rgba_at(&plain, 64, 20, 32);
    let right = rgba_at(&plain, 64, 44, 32);
    assert!(left[0] > 200 && left[2] < 80, "left is red: {left:?}");
    assert!(right[2] > 200 && right[0] < 80, "right is blue: {right:?}");
    // 90° clockwise: the image's left edge becomes the TOP — red on top,
    // blue at the bottom.
    let rotated = render(Some(90.0));
    let top = rgba_at(&rotated, 64, 32, 20);
    let bottom = rgba_at(&rotated, 64, 32, 44);
    assert!(
        top[0] > 200 && top[2] < 80,
        "rotated 90° cw the red half is on top: {top:?}"
    );
    assert!(
        bottom[2] > 200 && bottom[0] < 80,
        "rotated 90° cw the blue half is at the bottom: {bottom:?}"
    );
}

// -----------------------------------------------------------------------
// 6. Per-paint blend mode (gradient fill layer)
// -----------------------------------------------------------------------

#[test]
fn gradient_fill_blend_composites_against_the_paint_below() {
    // Bottom fill: solid red. Top fill: a flat mid-gray gradient with
    // Multiply — the result must be the darkened red product, not gray.
    let gray = Color::rgb(128, 128, 128);
    let flat_gray = fanta_doc::Gradient::Linear {
        start: [0.0, 0.0],
        end: [1.0, 0.0],
        stops: vec![
            GradientStop {
                position: 0.0,
                color: gray,
            },
            GradientStop {
                position: 1.0,
                color: gray,
            },
        ],
    };
    let mut vector = VectorNode::rect_solid(-16.0, -16.0, 32.0, 32.0, Color::rgb(255, 0, 0));
    vector.fills.push(Fill::Gradient {
        gradient: flat_gray,
        blend: fanta_doc::BlendMode::Multiply,
    });
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(CanvasNode::new(NodeData::Vector(
        vector,
    ))))
    .unwrap();
    let buf = render_doc(&doc, 64);
    let center = rgba_at(&buf, 64, 32, 32);
    assert!(
        center[0] > 100 && center[0] < 160 && center[1] < 40 && center[2] < 40,
        "multiplying gray over red must yield the dark-red product, got {center:?}"
    );
}

// -----------------------------------------------------------------------
// 7. Radial gradient axis handles (rotated / anisotropic ellipse)
// -----------------------------------------------------------------------

#[test]
fn radial_handles_shape_an_anisotropic_ellipse() {
    // Square node, centre (0.5, 0.5): x-axis handle reaches 0.5 out, y-axis
    // handle only 0.25 — a squashed ellipse. Equal world offsets from the
    // centre must therefore land at DIFFERENT ramp positions per axis
    // (an aspect-only radial on a square node would color them identically).
    let gradient = fanta_doc::Gradient::Radial {
        center: [0.5, 0.5],
        radius: 0.5,
        handles: Some([[1.0, 0.5], [0.5, 0.75]]),
        stops: vec![
            GradientStop {
                position: 0.0,
                color: Color::rgb(255, 255, 255),
            },
            GradientStop {
                position: 1.0,
                color: Color::rgb(0, 0, 0),
            },
        ],
    };
    let mut vector = VectorNode::rect_solid(-50.0, -50.0, 100.0, 100.0, Color::rgb(0, 0, 0));
    vector.fills = smallvec_of(Fill::Gradient {
        gradient,
        blend: fanta_doc::BlendMode::Normal,
    });
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(CanvasNode::new(NodeData::Vector(
        vector,
    ))))
    .unwrap();
    let buf = render_doc(&doc, 128);
    // +20 world px along x: t = 0.2/0.5 = 0.4 → light gray (~153).
    let along_x = rgba_at(&buf, 128, 64 + 20, 64);
    // +20 world px along y: t = 0.2/0.25 = 0.8 → dark gray (~51).
    let along_y = rgba_at(&buf, 128, 64, 64 + 20);
    assert!(
        along_x[0] > along_y[0] + 60,
        "the squashed y-axis must ramp to dark much sooner: x={along_x:?} y={along_y:?}"
    );
}

// -----------------------------------------------------------------------
// 8. Per-subpath fill rules (mixed winding)
// -----------------------------------------------------------------------

#[test]
fn mixed_subpath_rules_fill_each_group_by_its_own_rule() {
    // Subpath 0 (NonZero): a solid left square. Subpaths 1+2 (EvenOdd): a
    // same-winding nested pair on the right — even-odd carves the hole that
    // the path-level NonZero rule would fill solid.
    let mut path = fanta_doc::PathData::new();
    path.move_to(-30.0, -10.0)
        .line_to(-10.0, -10.0)
        .line_to(-10.0, 10.0)
        .line_to(-30.0, 10.0)
        .close();
    path.move_to(5.0, -15.0)
        .line_to(35.0, -15.0)
        .line_to(35.0, 15.0)
        .line_to(5.0, 15.0)
        .close();
    path.move_to(15.0, -5.0)
        .line_to(25.0, -5.0)
        .line_to(25.0, 5.0)
        .line_to(15.0, 5.0)
        .close();
    path.subpath_rules = vec![
        fanta_doc::FillRule::NonZero,
        fanta_doc::FillRule::EvenOdd,
        fanta_doc::FillRule::EvenOdd,
    ];
    let node = CanvasNode::new(NodeData::Vector(VectorNode {
        path,
        fills: smallvec_of(Fill::solid(Color::rgb(0, 200, 0))),
        strokes: Default::default(),
        corner_radius: None,
        corner_radii: None,
        corner_smoothing: 0.0,
    }));
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(node)).unwrap();
    let buf = render_doc(&doc, 64);
    // Left square centre: world (−20, 0) → screen (12, 32): filled.
    let left = rgba_at(&buf, 64, 12, 32);
    assert!(
        left[1] > 180 && left[3] > 200,
        "NonZero subpath fills the left square: {left:?}"
    );
    // Right ring band: world (10, 0) → screen (42, 32): filled.
    let band = rgba_at(&buf, 64, 42, 32);
    assert!(
        band[1] > 180 && band[3] > 200,
        "even-odd ring band stays filled: {band:?}"
    );
    // Right ring HOLE: world (20, 0) → screen (52, 32): carved empty. Without
    // the per-subpath split the path-level NonZero rule fills it solid.
    let hole = rgba_at(&buf, 64, 52, 32);
    assert!(
        hole[3] < 40,
        "the even-odd subpaths must carve their hole: {hole:?}"
    );
}

// -----------------------------------------------------------------------
// 9. max_lines + truncate
// -----------------------------------------------------------------------

#[test]
fn truncate_clamps_lines_at_max_lines_and_at_the_box_height() {
    clear_layout_cache();
    let mut node = TextNode::new(
        "one two three four five six seven eight nine ten eleven twelve",
        60.0,
        1000.0,
    );
    node.style.size_px = 16.0;
    let unclamped_lines = with_shaped_layout(&node, |layout| layout.line_count());
    assert!(
        unclamped_lines >= 4,
        "fixture must wrap to several lines, got {unclamped_lines}"
    );
    let line_height = with_shaped_layout(&node, |layout| {
        layout
            .lines()
            .first()
            .map(|line| line.height)
            .unwrap_or(0.0)
    });
    assert!(line_height > 0.0);

    // Explicit clamp.
    let mut clamped = node.clone();
    clamped.max_lines = Some(2);
    clamped.truncate = true;
    assert_eq!(
        with_shaped_layout(&clamped, |layout| layout.line_count()),
        2,
        "max_lines must clamp the shaped line count"
    );

    // Box-height clamp: truncation with no explicit max_lines on a fixed box
    // sized for exactly two lines.
    let mut box_clamped = node;
    box_clamped.truncate = true;
    box_clamped.local_size = [60.0, line_height * 2.0 + 1.0];
    assert_eq!(
        with_shaped_layout(&box_clamped, |layout| layout.line_count()),
        2,
        "truncation without max_lines must clamp at the box height"
    );
    // And the differently-truncated variants each keyed their own cache entry
    // (unclamped, explicit clamp, box clamp).
    assert_eq!(layout_cache_len(), 3);
}

// -----------------------------------------------------------------------
// 10. paragraph_spacing + paragraph_indent
// -----------------------------------------------------------------------

/// Lowest screen row with any ink, or 0 when nothing painted.
fn lowest_ink_row(buf: &[u8], side: u32) -> u32 {
    let mut lowest = 0;
    for y in 0..side {
        for x in 0..side {
            if rgba_at(buf, side, x, y)[3] != 0 {
                lowest = y;
            }
        }
    }
    lowest
}

/// Leftmost screen column with any ink, or `side` when nothing painted.
fn leftmost_ink_column(buf: &[u8], side: u32) -> u32 {
    for x in 0..side {
        for y in 0..side {
            if rgba_at(buf, side, x, y)[3] != 0 {
                return x;
            }
        }
    }
    side
}

#[test]
fn paragraph_spacing_grows_the_measure_and_shifts_later_paragraphs() {
    clear_layout_cache();
    let mut node = TextNode::new("Top\nBottom", 200.0, 200.0);
    node.style.size_px = 20.0;
    let (_, plain_height) = measure_text_node(&node);
    let mut spaced = node.clone();
    spaced.paragraph_spacing = 24.0;
    let (_, spaced_height) = measure_text_node(&spaced);
    assert!(
        (spaced_height - plain_height - 24.0).abs() < 0.01,
        "one hard break adds exactly the spacing: plain={plain_height} spaced={spaced_height}"
    );

    // Pixel proof: the second paragraph's ink moves down by the spacing.
    let render_text = |text_node: TextNode| -> Vec<u8> {
        let mut canvas_node = CanvasNode::new(NodeData::Text(text_node));
        canvas_node.transform = Transform2D::translation(-60.0, -60.0);
        let mut doc = Doc::new();
        doc.apply(Operation::create_node(canvas_node)).unwrap();
        render_doc(&doc, 128)
    };
    let plain_ink = lowest_ink_row(&render_text(node), 128);
    let spaced_ink = lowest_ink_row(&render_text(spaced), 128);
    let shift = spaced_ink as i64 - plain_ink as i64;
    assert!(
        (shift - 24).abs() <= 2,
        "second paragraph must paint ~24px lower, moved {shift}px"
    );
}

#[test]
fn paragraph_indent_insets_the_first_line() {
    clear_layout_cache();
    let mut node = TextNode::new("Hi", 200.0, 100.0);
    node.style.size_px = 20.0;
    let render_text = |text_node: TextNode| -> Vec<u8> {
        let mut canvas_node = CanvasNode::new(NodeData::Text(text_node));
        canvas_node.transform = Transform2D::translation(-60.0, -30.0);
        let mut doc = Doc::new();
        doc.apply(Operation::create_node(canvas_node)).unwrap();
        render_doc(&doc, 128)
    };
    let plain_left = leftmost_ink_column(&render_text(node.clone()), 128);
    node.paragraph_indent = 16.0;
    let indented_left = leftmost_ink_column(&render_text(node), 128);
    let shift = indented_left as i64 - plain_left as i64;
    assert!(
        (shift - 16).abs() <= 2,
        "the indent must inset the first line's ink by ~16px, moved {shift}px"
    );
}

// -----------------------------------------------------------------------
// 11. ISOLATED_BLEND containers
// -----------------------------------------------------------------------

#[test]
fn isolated_blend_confines_a_child_blend_to_the_group() {
    fn center_pixel(isolate: bool) -> [u8; 4] {
        let mut doc = Doc::new();
        // Backdrop below the group: a solid red rect. Sibling z-order is the
        // fractional index (equal indexes tie-break by random id), so the two
        // roots get explicit indexes: backdrop below, group above.
        let mut backdrop = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            -20.0,
            -20.0,
            40.0,
            40.0,
            Color::rgb(255, 0, 0),
        )));
        backdrop.index = fanta_doc::IndexKey::from_raw(1.0);
        doc.apply(Operation::create_node(backdrop)).unwrap();
        // Unclipped group holding one gray Multiply child.
        let mut group = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode::default()));
        group.index = fanta_doc::IndexKey::from_raw(2.0);
        if isolate {
            group.flags |= NodeFlags::ISOLATED_BLEND;
        }
        let group_id = group.id;
        doc.apply(Operation::create_node(group)).unwrap();
        let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            -10.0,
            -10.0,
            20.0,
            20.0,
            Color::rgb(128, 128, 128),
        )));
        child.parent = Some(group_id);
        child.blend_mode = BlendMode::Multiply;
        doc.apply(Operation::create_node(child)).unwrap();
        let buf = render_doc(&doc, 64);
        rgba_at(&buf, 64, 32, 32)
    }
    // Pass-through (no isolation): the gray child multiplies against the red
    // backdrop → the dark-red product.
    let pass_through = center_pixel(false);
    assert!(
        pass_through[0] > 100 && pass_through[0] < 160 && pass_through[1] < 40,
        "pass-through multiply must darken the backdrop: {pass_through:?}"
    );
    // Isolated: the multiply resolves against the group's own (transparent)
    // contents, and the flattened group composites normally → plain gray.
    let isolated = center_pixel(true);
    assert!(
        isolated[0] > 100 && isolated[1] > 100 && isolated[2] > 100,
        "isolation must stop the multiply from reaching the backdrop: {isolated:?}"
    );
}
