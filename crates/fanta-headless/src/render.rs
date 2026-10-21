//! Raster renders of a page or one node, the way the editor's design tools
//! take screenshots. Skia's CPU rasterizer draws them, so they need no GPU.

use std::{collections::HashSet, sync::Arc};

use anyhow::{Context as _, Result, anyhow, bail};
use fanta_design_ops::ScreenshotTarget;
use fanta_doc::{AssetId, Bounds, Doc, NodeId, Viewport};
use fanta_render::{
    AssetResolver, DecodedImage, RasterRenderer, RenderInputs, visual_world_bounds,
};

/// A PNG and the world rectangle it shows.
pub struct Screenshot {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub bounds: Bounds,
}

/// The document-wide inputs a render reads besides the scene: components,
/// variables and modes, with no motion, playback or UI theme.
pub fn render_inputs(document: &Doc) -> RenderInputs<'_> {
    RenderInputs {
        components: &document.components,
        variables: &document.variables,
        active_modes: &document.active_modes,
        mode_generation: 0,
        motion: None,
        playback: None,
        video_fill_frames: None,
        dark_ui: false,
    }
}

pub fn page_bounds(doc: &Doc, page_root: Option<NodeId>) -> Bounds {
    try_page_bounds(doc, page_root).unwrap_or_else(|| Bounds::from_xywh(0.0, 0.0, 1024.0, 768.0))
}

pub fn try_page_bounds(doc: &Doc, page_root: Option<NodeId>) -> Option<Bounds> {
    let mut bounds: Option<Bounds> = None;
    let mut include = |node_id| {
        if let Some(node_bounds) = doc.scene.world_bounds(node_id)
            && node_bounds.is_finite()
            && node_bounds.width() > 0.0
            && node_bounds.height() > 0.0
        {
            bounds = Some(match bounds {
                Some(bounds) => bounds.union(&node_bounds),
                None => node_bounds,
            });
        }
    };

    if let Some(page_root) = page_root {
        for node_id in doc.scene.descendants_of(page_root) {
            if node_id != page_root {
                include(node_id);
            }
        }
    } else {
        for root in doc.scene.roots() {
            for node_id in doc.scene.descendants_of(*root) {
                include(node_id);
            }
        }
    }

    bounds
}

/// Render `node`, or the whole of `page_root` when there is none, as a PNG
/// no longer than `target.max_dimension` (default 768, at most 1568) on its
/// longer side.
pub fn render_screenshot(
    doc: &Doc,
    asset_resolver: Option<Arc<dyn AssetResolver>>,
    page_root: Option<NodeId>,
    node: Option<NodeId>,
    target: ScreenshotTarget,
) -> Result<Screenshot> {
    // Larger screenshots consume the gateway's request budget without adding
    // detail after the model's image downscaling.
    let max_dimension = f64::from(target.max_dimension.unwrap_or(768).clamp(16, 1568));
    let motion = match target.motion_clip {
        Some(raw) => {
            let clip = raw
                .parse::<fanta_doc::AnimationClipId>()
                .map_err(|_| anyhow!("{raw} is not a valid animation clip id"))?;
            Some(
                doc.motion
                    .evaluate(clip, target.playhead_ms.unwrap_or(0))
                    .with_context(|| format!("animation clip {raw} does not exist"))?,
            )
        }
        None if target.playhead_ms.is_some() => bail!("playhead_ms requires motion_clip"),
        None => None,
    };
    let motion_scene = motion.as_ref().map(|motion| {
        let mut scene = doc.scene.clone();
        let affected = motion
            .overrides
            .keys()
            .map(|target| target.node)
            .collect::<HashSet<_>>();
        for id in affected {
            if let Some(node) = scene.get_mut(id) {
                *node = motion.apply_to_node(node);
            }
        }
        scene
    });
    let bounds_scene = motion_scene.as_ref().unwrap_or(&doc.scene);
    let bounds = match node {
        Some(node) => visual_world_bounds(bounds_scene, node, 0.0)
            .context("the node has no visible bounds")?,
        None if motion_scene.is_some() => page_root
            .and_then(|root| visual_world_bounds(bounds_scene, root, 0.0))
            .unwrap_or_else(|| page_bounds(doc, page_root)),
        None => page_bounds(doc, page_root),
    };
    if !bounds.is_finite() || bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        bail!("the screenshot target has invalid or empty bounds");
    }
    // Fit within the dimension cap; allow mild upscale so small nodes
    // (icons) stay legible without ballooning the surface.
    let zoom = (max_dimension / bounds.width().max(bounds.height())).min(2.0);
    let width = (bounds.width() * zoom).ceil().max(1.0) as u32;
    let height = (bounds.height() * zoom).ceil().max(1.0) as u32;
    let mut renderer = RasterRenderer::new(width, height)
        .map_err(|error| anyhow!("creating {width}x{height} screenshot surface: {error}"))?;
    if let Some(asset_resolver) = asset_resolver {
        renderer.set_asset_resolver(asset_resolver);
    }
    let center = bounds.center();
    let viewport = Viewport {
        center: [center.x, center.y],
        zoom,
    };
    let mut inputs = render_inputs(doc);
    inputs.motion = motion.as_ref();
    renderer.render_page_with(&doc.scene, &viewport, page_root, &inputs);
    let png = renderer
        .encode_png()
        .map_err(|error| anyhow!("encoding screenshot PNG: {error}"))?;
    Ok(Screenshot {
        png,
        width,
        height,
        bounds,
    })
}

/// Decode one embedded asset to the straight-alpha RGBA8 the renderer draws.
/// Installed in a document's `LazyAssetResolver`, which calls it the first
/// time an image is drawn (or prewarmed) and remembers a failure so a corrupt
/// asset is logged once, not once per frame.
pub fn decode_embedded_image(asset_id: AssetId, bytes: &[u8]) -> Option<DecodedImage> {
    match image::load_from_memory(bytes) {
        Ok(image) => {
            // `into_rgba8` moves an RGBA8 decode instead of cloning it, so the
            // copy into the shared buffer is the only one made.
            let image = image.into_rgba8();
            let (width, height) = image.dimensions();
            Some(DecodedImage::new(image.as_raw(), width, height))
        }
        Err(error) => {
            log::warn!("failed to decode embedded .fig image asset {asset_id}: {error}");
            None
        }
    }
}

/// An embedded image's natural size, read from its header. The canvas picks
/// the level to decode from it before decoding anything, and Skia, which the
/// resolver reads sizes with by default, is built without WebP.
pub fn embedded_image_size(bytes: &[u8]) -> Option<[u32; 2]> {
    let (width, height) = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()?;
    Some([width, height])
}
