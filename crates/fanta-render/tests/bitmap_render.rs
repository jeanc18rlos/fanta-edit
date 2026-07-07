//! End-to-end bitmap rendering: drive `RasterRenderer` with an
//! `InMemoryAssetResolver` and assert the sampled pixels show the *decoded
//! image* (not the magenta/blue placeholder), honour each `ImageFitMode`,
//! apply `tint`, and degrade gracefully when an asset is missing.
//!
//! These mirror the pixel-sampling style of `raster.rs`'s in-crate tests
//! (`copy_rgba` + index into the buffer) and cover the acceptance criteria in
//! `specs/03-media-3d-and-node-workflows.md` §1.

use std::sync::Arc;

use fanta_doc::{
    AssetId, BitmapNode, CanvasNode, Color, Doc, Fill, ImageFitMode, NodeData, Operation,
    VectorNode,
};
use fanta_render::{DecodedImage, InMemoryAssetResolver, RasterRenderer};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn rgba_at(buf: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let row = (width * 4) as usize;
    let i = (y as usize) * row + (x as usize) * 4;
    [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
}

/// A `w × h` image filled with one straight-alpha RGBA colour.
fn solid_image(w: u32, h: u32, rgba: [u8; 4]) -> DecodedImage {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..(w * h) {
        px.extend_from_slice(&rgba);
    }
    DecodedImage::new(Arc::new(px), w, h)
}

/// A 2×1 image: left column `left`, right column `right`. Useful to prove the
/// fit/crop math samples the correct source region.
fn two_column_image(left: [u8; 4], right: [u8; 4]) -> DecodedImage {
    let mut px = Vec::with_capacity(2 * 4);
    px.extend_from_slice(&left);
    px.extend_from_slice(&right);
    DecodedImage::new(Arc::new(px), 2, 1)
}

/// Build a resolver holding a single asset and return `(resolver, id)`.
fn resolver_with(img: DecodedImage) -> (Arc<InMemoryAssetResolver>, AssetId) {
    let mut r = InMemoryAssetResolver::new();
    let id = AssetId::new();
    r.insert(id, img);
    (Arc::new(r), id)
}

/// A `BitmapNode` centred at the origin (so a centred viewport samples its
/// middle at the surface centre), `size × size` local units.
fn bitmap_node(asset: AssetId, size: f64, fit: ImageFitMode, tint: Option<Color>) -> CanvasNode {
    let mut node = CanvasNode::new(NodeData::Bitmap(BitmapNode {
        asset,
        natural_size: [0, 0], // not consulted by the renderer; resolver carries dims
        local_size: [size, size],
        crop: None,
        fit,
        tint,
    }));
    // Centre the node rect [0,0,size] on the origin so the surface centre lands
    // inside it under the default centred viewport.
    node.transform = fanta_doc::Transform2D::translation(-size * 0.5, -size * 0.5);
    node
}

// ---------------------------------------------------------------------------
// Core: a real asset renders, a missing one falls back
// ---------------------------------------------------------------------------

#[test]
fn registered_red_asset_renders_red_at_center_not_placeholder() {
    let (resolver, id) = resolver_with(solid_image(4, 4, [255, 0, 0, 255]));
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(bitmap_node(
        id,
        20.0,
        ImageFitMode::Fill,
        None,
    )))
    .unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.set_asset_resolver(resolver);
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    let c = rgba_at(&buf, 64, 32, 32);
    assert!(c[0] > 200, "expected red, got {c:?}");
    assert!(c[1] < 40, "expected low green, got {c:?}");
    assert!(c[2] < 40, "expected low blue, got {c:?}");
    assert!(c[3] > 200, "expected opaque, got {c:?}");
    // The placeholder is a light blue (120,200,255) — explicitly NOT this.
    assert!(
        !(c[2] > 200 && c[1] > 150),
        "rendered the placeholder, not the asset: {c:?}"
    );
}

#[test]
fn missing_asset_renders_placeholder_without_panicking() {
    // Resolver exists but does not hold the node's asset id.
    let resolver = Arc::new(InMemoryAssetResolver::new());
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(bitmap_node(
        AssetId::new(),
        20.0,
        ImageFitMode::Fill,
        None,
    )))
    .unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.set_asset_resolver(resolver);
    let metrics = r.render(&doc.scene, &doc.viewport);
    assert!(
        metrics.nodes_drawn >= 1,
        "placeholder still counts as a draw"
    );

    let buf = r.copy_rgba();
    let c = rgba_at(&buf, 64, 32, 32);
    // The placeholder is light blue with alpha ~100 — present, not red.
    assert!(c[3] > 0, "placeholder should be visible, got {c:?}");
    assert!(c[2] >= c[0], "placeholder skews blue, not red: {c:?}");
}

#[test]
fn no_resolver_installed_renders_placeholder() {
    // A renderer with no resolver at all (the pre-asset default) must still
    // draw the placeholder rather than panic or render nothing.
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(bitmap_node(
        AssetId::new(),
        20.0,
        ImageFitMode::Fill,
        None,
    )))
    .unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    let metrics = r.render(&doc.scene, &doc.viewport);
    assert!(metrics.nodes_drawn >= 1);
    let buf = r.copy_rgba();
    assert!(rgba_at(&buf, 64, 32, 32)[3] > 0, "placeholder visible");
}

#[test]
fn malformed_buffer_falls_back_to_placeholder() {
    // Buffer length does not match width*height*4 → the renderer must not hand
    // it to Skia; it falls back to the placeholder.
    let bad = DecodedImage::new(Arc::new(vec![255u8; 7]), 4, 4);
    let (resolver, id) = resolver_with(bad);
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(bitmap_node(
        id,
        20.0,
        ImageFitMode::Fill,
        None,
    )))
    .unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.set_asset_resolver(resolver);
    let metrics = r.render(&doc.scene, &doc.viewport);
    assert!(metrics.nodes_drawn >= 1);
    let buf = r.copy_rgba();
    let c = rgba_at(&buf, 64, 32, 32);
    assert!(
        c[2] >= c[0],
        "malformed asset should show placeholder: {c:?}"
    );
}

// ---------------------------------------------------------------------------
// Fit modes
// ---------------------------------------------------------------------------

#[test]
fn stretch_fills_the_whole_rect() {
    // A 4x4 green image stretched into a 20x20 rect should be green across the
    // whole rect, including a near-corner sample where Fit would letterbox.
    let (resolver, id) = resolver_with(solid_image(4, 4, [0, 200, 0, 255]));
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(bitmap_node(
        id,
        20.0,
        ImageFitMode::Stretch,
        None,
    )))
    .unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.set_asset_resolver(resolver);
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    // Rect spans surface pixels ~[22,42] in both axes. Sample near a corner.
    let corner = rgba_at(&buf, 64, 24, 24);
    assert!(
        corner[1] > 150,
        "stretch should fill the corner green: {corner:?}"
    );
    let center = rgba_at(&buf, 64, 32, 32);
    assert!(center[1] > 150, "stretch center green: {center:?}");
}

#[test]
fn fit_letterboxes_a_wide_image() {
    // A wide 2x1 image (both columns opaque white) fitted into a square leaves
    // transparent letterbox bars top and bottom. Center is white; the
    // top-of-rect band is transparent.
    let (resolver, id) =
        resolver_with(two_column_image([255, 255, 255, 255], [255, 255, 255, 255]));
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(bitmap_node(
        id,
        40.0,
        ImageFitMode::Fit,
        None,
    )))
    .unwrap();

    let mut r = RasterRenderer::new(80, 80).unwrap();
    r.set_asset_resolver(resolver);
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    // Rect spans surface ~[20,60]. Center should be opaque (image band).
    let center = rgba_at(&buf, 80, 40, 40);
    assert!(
        center[3] > 200,
        "fit center should be opaque image: {center:?}"
    );
    // Aspect 2:1 in a square → image band is 40 wide x 20 tall, centered
    // vertically (rows ~30..50). Row 22 (near top of rect) is letterbox.
    let top_band = rgba_at(&buf, 80, 40, 22);
    assert_eq!(top_band[3], 0, "fit should letterbox the top: {top_band:?}");
}

#[test]
fn fill_covers_the_whole_rect_with_no_transparent_gaps() {
    // The same wide image under Fill covers the square (cropping the sides),
    // so the near-top band that letterboxed under Fit is now opaque.
    let (resolver, id) =
        resolver_with(two_column_image([255, 255, 255, 255], [255, 255, 255, 255]));
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(bitmap_node(
        id,
        40.0,
        ImageFitMode::Fill,
        None,
    )))
    .unwrap();

    let mut r = RasterRenderer::new(80, 80).unwrap();
    r.set_asset_resolver(resolver);
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    let top_band = rgba_at(&buf, 80, 40, 22);
    assert!(
        top_band[3] > 200,
        "fill should cover the top band: {top_band:?}"
    );
    let center = rgba_at(&buf, 80, 40, 40);
    assert!(center[3] > 200, "fill center opaque: {center:?}");
}

#[test]
fn tile_repeats_and_fills_the_rect() {
    // A small solid tile repeated still covers the whole rect opaquely.
    let (resolver, id) = resolver_with(solid_image(4, 4, [40, 80, 220, 255]));
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(bitmap_node(
        id,
        40.0,
        ImageFitMode::Tile,
        None,
    )))
    .unwrap();

    let mut r = RasterRenderer::new(80, 80).unwrap();
    r.set_asset_resolver(resolver);
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    let center = rgba_at(&buf, 80, 40, 40);
    assert!(center[3] > 200, "tile should be opaque: {center:?}");
    assert!(center[2] > 150, "tile colour (blue) present: {center:?}");
    // A different position within the rect is also covered (the repeat tiles).
    let off = rgba_at(&buf, 80, 28, 52);
    assert!(off[3] > 200, "tile fills off-center too: {off:?}");
}

// ---------------------------------------------------------------------------
// Tint
// ---------------------------------------------------------------------------

#[test]
fn tint_multiplies_and_shifts_the_rendered_color() {
    // A white image tinted red should render red: multiply(white, red) = red.
    let (resolver, id) = resolver_with(solid_image(4, 4, [255, 255, 255, 255]));
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(bitmap_node(
        id,
        20.0,
        ImageFitMode::Stretch,
        Some(Color::rgb(255, 0, 0)),
    )))
    .unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.set_asset_resolver(resolver);
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    let c = rgba_at(&buf, 64, 32, 32);
    assert!(c[0] > 200, "tinted red channel high: {c:?}");
    assert!(c[1] < 40, "multiply zeroes green: {c:?}");
    assert!(c[2] < 40, "multiply zeroes blue: {c:?}");
}

#[test]
fn untinted_white_stays_white() {
    // Control for the tint test: same white image, no tint, stays white.
    let (resolver, id) = resolver_with(solid_image(4, 4, [255, 255, 255, 255]));
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(bitmap_node(
        id,
        20.0,
        ImageFitMode::Stretch,
        None,
    )))
    .unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.set_asset_resolver(resolver);
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    let c = rgba_at(&buf, 64, 32, 32);
    assert!(
        c[0] > 200 && c[1] > 200 && c[2] > 200,
        "white stays white: {c:?}"
    );
}

// ---------------------------------------------------------------------------
// Crop
// ---------------------------------------------------------------------------

#[test]
fn crop_selects_the_requested_source_region() {
    // 2x1 image: left red, right green. Crop to the right half only, then
    // stretch — the rect should be green, proving crop selected the right
    // column before the fit mapping.
    let img = two_column_image([255, 0, 0, 255], [0, 200, 0, 255]);
    let (resolver, id) = resolver_with(img);
    let mut node = CanvasNode::new(NodeData::Bitmap(BitmapNode {
        asset: id,
        natural_size: [2, 1],
        local_size: [20.0, 20.0],
        crop: Some([0.5, 0.0, 0.5, 1.0]), // right half
        fit: ImageFitMode::Stretch,
        tint: None,
    }));
    node.transform = fanta_doc::Transform2D::translation(-10.0, -10.0);
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(node)).unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.set_asset_resolver(resolver);
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    let c = rgba_at(&buf, 64, 32, 32);
    assert!(c[1] > 150, "cropped-to-right should be green: {c:?}");
    assert!(c[0] < 80, "the red left column was cropped out: {c:?}");
}

// ---------------------------------------------------------------------------
// Idempotence / determinism across frames
// ---------------------------------------------------------------------------

#[test]
fn rendering_twice_with_same_resolver_is_deterministic() {
    let (resolver, id) = resolver_with(solid_image(4, 4, [10, 120, 240, 255]));
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(bitmap_node(
        id,
        20.0,
        ImageFitMode::Fill,
        None,
    )))
    .unwrap();

    let mut r = RasterRenderer::new(48, 48).unwrap();
    r.set_asset_resolver(resolver);
    r.render(&doc.scene, &doc.viewport);
    let frame1 = r.copy_rgba();
    r.render(&doc.scene, &doc.viewport);
    let frame2 = r.copy_rgba();
    assert_eq!(
        frame1, frame2,
        "two frames with an unchanged resolver must match"
    );
}

// ---------------------------------------------------------------------------
// Vector image fill (Fill::Image)
// ---------------------------------------------------------------------------

#[test]
fn vector_image_fill_paints_the_image_clipped_to_the_path() {
    // A rect vector with Fill::Image should show the resolved image inside the
    // path. Center is image-colour; a point well outside the rect is empty.
    let (resolver, id) = resolver_with(solid_image(4, 4, [0, 0, 255, 255]));
    let mut doc = Doc::new();
    let node = CanvasNode::new(NodeData::Vector(VectorNode {
        path: fanta_doc::PathData::rect(-15.0, -15.0, 30.0, 30.0),
        fills: smallvec_one(Fill::Image {
            asset: id,
            mode: ImageFitMode::Fill,
            opacity: 1.0,
            crop: None,
        }),
        strokes: Default::default(),
        corner_radius: None,
        corner_radii: None,
    }));
    doc.apply(Operation::create_node(node)).unwrap();

    let mut r = RasterRenderer::new(80, 80).unwrap();
    r.set_asset_resolver(resolver);
    let metrics = r.render(&doc.scene, &doc.viewport);
    assert!(metrics.nodes_drawn >= 1);
    let buf = r.copy_rgba();

    let center = rgba_at(&buf, 80, 40, 40);
    assert!(
        center[2] > 200,
        "vector image fill should be blue: {center:?}"
    );
    assert!(center[3] > 200, "and opaque: {center:?}");
    // Far corner is outside the 30x30 rect → transparent.
    let corner = rgba_at(&buf, 80, 4, 4);
    assert_eq!(
        corner[3], 0,
        "outside the path stays transparent: {corner:?}"
    );
}

#[test]
fn vector_image_fill_missing_asset_shows_placeholder_paint() {
    // With no matching asset, the vector falls back to fill_to_paint's
    // transparent-magenta placeholder rather than rendering nothing or panicking.
    let resolver = Arc::new(InMemoryAssetResolver::new());
    let mut doc = Doc::new();
    let node = CanvasNode::new(NodeData::Vector(VectorNode {
        path: fanta_doc::PathData::rect(-15.0, -15.0, 30.0, 30.0),
        fills: smallvec_one(Fill::Image {
            asset: AssetId::new(),
            mode: ImageFitMode::Fill,
            opacity: 1.0,
            crop: None,
        }),
        strokes: Default::default(),
        corner_radius: None,
        corner_radii: None,
    }));
    doc.apply(Operation::create_node(node)).unwrap();

    let mut r = RasterRenderer::new(80, 80).unwrap();
    r.set_asset_resolver(resolver);
    let metrics = r.render(&doc.scene, &doc.viewport);
    assert!(metrics.nodes_drawn >= 1, "placeholder paint still draws");
    let buf = r.copy_rgba();
    // Placeholder magenta has alpha 64 → some red+blue, low-ish alpha.
    let center = rgba_at(&buf, 80, 40, 40);
    assert!(
        center[0] > 0 && center[2] > 0,
        "magenta-ish placeholder: {center:?}"
    );
}

/// Build a `SmallVec<[Fill; 1]>` with a single element without importing the
/// crate directly (keeps the test deps minimal).
fn smallvec_one(fill: Fill) -> smallvec::SmallVec<[Fill; 1]> {
    let mut v = smallvec::SmallVec::new();
    v.push(fill);
    v
}
