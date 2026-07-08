//! Image-upload-cache tests (population, cross-frame reuse, clear).
use super::*;
use fanta_doc::{Doc, GroupNode, ImageFitMode, Operation, VectorNode};

// -----------------------------------------------------------------------
// Image cache (renderer-level: it survives across frames)
// -----------------------------------------------------------------------

fn solid_resolver(rgba: [u8; 4]) -> (Arc<crate::asset::InMemoryAssetResolver>, fanta_doc::AssetId) {
    use crate::asset::{DecodedImage, InMemoryAssetResolver};
    let mut px = Vec::with_capacity(16);
    for _ in 0..4 {
        px.extend_from_slice(&rgba);
    }
    let img = DecodedImage::new(Arc::new(px), 2, 2);
    let mut res = InMemoryAssetResolver::new();
    let id = fanta_doc::AssetId::new();
    res.insert(id, img);
    (Arc::new(res), id)
}

fn bitmap_doc(asset: fanta_doc::AssetId) -> Doc {
    use fanta_doc::BitmapNode;
    let mut node = CanvasNode::new(NodeData::Bitmap(BitmapNode {
        asset,
        natural_size: [2, 2],
        local_size: [20.0, 20.0],
        crop: None,
        fit: fanta_doc::ImageFitMode::Fill,
        tint: None,
    }));
    node.transform = Transform2D::translation(-10.0, -10.0);
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(node)).unwrap();
    doc
}

fn image_background_frame_doc(asset: fanta_doc::AssetId) -> Doc {
    let mut group = GroupNode {
        clip_size: Some([20.0, 20.0]),
        background: Some(Fill::Image {
            asset,
            mode: ImageFitMode::Stretch,
            opacity: 1.0,
            crop: None,
            scale: None,
            rotation: None,
            blend: fanta_doc::BlendMode::Normal,
        }),
        ..GroupNode::default()
    };
    group.auto_layout = None;
    let mut node = CanvasNode::new(NodeData::Group(group));
    node.transform = Transform2D::translation(-10.0, -10.0);
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(node)).unwrap();
    doc
}

#[test]
fn image_cache_populates_on_first_render_and_reuses_on_second() {
    let (resolver, id) = solid_resolver([200, 30, 30, 255]);
    let doc = bitmap_doc(id);

    let mut r = RasterRenderer::new(48, 48).unwrap();
    r.set_asset_resolver(resolver);
    assert_eq!(r.image_cache_len(), 0, "cache starts empty");

    r.render(&doc.scene, &doc.viewport);
    assert_eq!(
        r.image_cache_len(),
        1,
        "first render uploads and caches the image"
    );

    // A second render must not grow the cache — the same asset id hits.
    r.render(&doc.scene, &doc.viewport);
    assert_eq!(
        r.image_cache_len(),
        1,
        "second render reuses the cached image"
    );
}

#[test]
fn image_cache_reuse_renders_identical_pixels_across_frames() {
    let (resolver, id) = solid_resolver([10, 120, 240, 255]);
    let doc = bitmap_doc(id);

    let mut r = RasterRenderer::new(48, 48).unwrap();
    r.set_asset_resolver(resolver);
    r.render(&doc.scene, &doc.viewport);
    let frame1 = r.copy_rgba();
    // Second frame draws from the cache rather than rebuilding the image.
    r.render(&doc.scene, &doc.viewport);
    let frame2 = r.copy_rgba();
    assert_eq!(
        frame1, frame2,
        "cached-image frame must match the first frame"
    );
    // And it is the real image colour (blue), not the placeholder.
    let c = rgba_at(&frame2, 48, 24, 24);
    assert!(
        c[2] > 150,
        "centre should be blue from the cached image: {c:?}"
    );
}

#[test]
fn image_background_on_frame_renders_real_asset_not_placeholder() {
    let (resolver, id) = solid_resolver([30, 210, 60, 255]);
    let doc = image_background_frame_doc(id);

    let mut r = RasterRenderer::new(48, 48).unwrap();
    r.set_asset_resolver(resolver);
    r.render(&doc.scene, &doc.viewport);
    let frame = r.copy_rgba();
    let c = rgba_at(&frame, 48, 24, 24);
    assert!(
        c[1] > 160 && c[0] < 80 && c[2] < 100,
        "frame image background should draw the green asset, not magenta placeholder: {c:?}"
    );
}

/// A 2x2 image with four distinct quadrant colours (row-major, straight RGBA):
/// top-left red, top-right green, bottom-left blue, bottom-right white.
fn quad_resolver() -> (Arc<crate::asset::InMemoryAssetResolver>, fanta_doc::AssetId) {
    use crate::asset::{DecodedImage, InMemoryAssetResolver};
    let px: Vec<u8> = vec![
        255, 0, 0, 255, // (0,0) red
        0, 255, 0, 255, // (1,0) green
        0, 0, 255, 255, // (0,1) blue
        255, 255, 255, 255, // (1,1) white
    ];
    let img = DecodedImage::new(Arc::new(px), 2, 2);
    let mut res = InMemoryAssetResolver::new();
    let id = fanta_doc::AssetId::new();
    res.insert(id, img);
    (Arc::new(res), id)
}

/// A vector rectangle (centred on the origin) filled with one image paint.
fn image_fill_rect_doc(asset: fanta_doc::AssetId, crop: Option<[f32; 4]>) -> Doc {
    image_fill_rect_doc_with_opacity(asset, crop, 1.0)
}

fn image_fill_rect_doc_with_opacity(
    asset: fanta_doc::AssetId,
    crop: Option<[f32; 4]>,
    opacity: f32,
) -> Doc {
    let mut node = VectorNode::rect_solid(-10.0, -10.0, 20.0, 20.0, Color::rgb(0, 0, 0));
    node.fills = smallvec_of(Fill::Image {
        asset,
        mode: ImageFitMode::Stretch,
        opacity,
        crop: crop.map(Box::new),
        scale: None,
        rotation: None,
        blend: fanta_doc::BlendMode::Normal,
    });
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(CanvasNode::new(NodeData::Vector(
        node,
    ))))
    .unwrap();
    doc
}

#[test]
fn image_fill_opacity_attenuates_resolved_bitmap_alpha() {
    let (resolver, id) = solid_resolver([255, 0, 0, 255]);
    let doc = image_fill_rect_doc_with_opacity(id, None, 0.5);

    let mut r = RasterRenderer::new(40, 40).unwrap();
    r.set_asset_resolver(resolver);
    r.render(&doc.scene, &doc.viewport);
    let frame = r.copy_rgba();
    let c = rgba_at(&frame, 40, 20, 20);
    assert!(
        c[0] > 200 && c[1] < 40 && c[2] < 40 && (110..=150).contains(&c[3]),
        "50% image fill opacity should keep red but halve alpha, got {c:?}"
    );
}

#[test]
fn cropped_image_fill_on_a_vector_samples_only_the_crop_subrect() {
    // Crop the top-left quadrant ([0, 0, 0.5, 0.5]) of the 2x2 quad image and
    // STRETCH it across the whole rect: the entire fill must read RED, because
    // only the red texel is in the cropped source window. The uncropped control
    // shows the bottom-right white texel at the same probe point, proving the
    // crop genuinely narrows the sampled region (model → render).
    let (resolver, id) = quad_resolver();

    let mut r = RasterRenderer::new(40, 40).unwrap();
    r.set_asset_resolver(resolver);

    // Cropped: top-left quadrant only.
    let cropped = image_fill_rect_doc(id, Some([0.0, 0.0, 0.5, 0.5]));
    r.render(&cropped.scene, &cropped.viewport);
    let frame = r.copy_rgba();
    // Probe the bottom-right of the on-screen rect — uncropped this would be the
    // white texel; cropped to the red quadrant it must be red.
    let c = rgba_at(&frame, 40, 28, 28);
    assert!(
        c[0] > 200 && c[1] < 80 && c[2] < 80,
        "cropped fill samples only the red top-left quadrant: {c:?}"
    );

    // Control: the same probe with NO crop shows the white bottom-right texel.
    r.clear_image_cache();
    let whole = image_fill_rect_doc(id, None);
    r.render(&whole.scene, &whole.viewport);
    let frame2 = r.copy_rgba();
    let c2 = rgba_at(&frame2, 40, 28, 28);
    assert!(
        c2[0] > 200 && c2[1] > 200 && c2[2] > 200,
        "uncropped fill shows the white bottom-right texel at the same probe: {c2:?}"
    );
}

#[test]
fn clear_image_cache_empties_it_then_repopulates() {
    let (resolver, id) = solid_resolver([0, 200, 0, 255]);
    let doc = bitmap_doc(id);

    let mut r = RasterRenderer::new(48, 48).unwrap();
    r.set_asset_resolver(resolver);
    r.render(&doc.scene, &doc.viewport);
    assert_eq!(r.image_cache_len(), 1);
    r.clear_image_cache();
    assert_eq!(r.image_cache_len(), 0, "clear drops cached images");
    // Re-rendering re-populates it (the asset is still resolvable).
    r.render(&doc.scene, &doc.viewport);
    assert_eq!(r.image_cache_len(), 1, "render after clear repopulates");
}
