//! Still exports of a design: PNG, JPEG, SVG and PDF renders of a page or a
//! node, sized by scale or by a target width or height. The editor's export
//! menu and the design API both export through these functions, so they
//! produce the same bytes.

use std::borrow::Cow;
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use fanta_doc::{Bounds, Doc, NodeId, Viewport, resolve_bound_value, resolve_effective_mode};
use fanta_render::{AssetResolver, RasterRenderer, visual_world_bounds};
use image::codecs::jpeg::JpegEncoder;

use crate::render::render_inputs;

/// The longest side a raster export may have.
pub const MAX_EXPORT_PIXELS: u32 = 8192;

/// The most pixels a raster export may hold.
pub const MAX_EXPORT_TOTAL_PIXELS: u64 = 16 * 1024 * 1024;

/// How large a raster export is: a multiple of the design's size, a target
/// width or height that the other side follows, or a cap on the longer side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ExportSizing {
    Scale(f64),
    Width(f64),
    Height(f64),
    /// The design's own size, scaled down when its longer side is over this
    /// many pixels; never scaled up.
    MaxDimension(f64),
}

impl ExportSizing {
    pub fn zoom(self, bounds: Bounds) -> Result<f64> {
        let zoom = match self {
            Self::Scale(scale) => scale,
            Self::Width(width) => width / bounds.width(),
            Self::Height(height) => height / bounds.height(),
            Self::MaxDimension(limit) => (limit / bounds.width().max(bounds.height())).min(1.0),
        };
        if !zoom.is_finite() || zoom <= 0.0 {
            bail!("export size must be a finite positive value");
        }
        Ok(zoom)
    }

    /// How error messages and file names spell the size, such as `2x`.
    pub fn label(self) -> String {
        match self {
            Self::Scale(scale) => format!("{scale}x"),
            Self::Width(width) => format!("{width}w"),
            Self::Height(height) => format!("{height}h"),
            Self::MaxDimension(limit) => format!("{limit}max"),
        }
    }
}

pub fn resolve_export_bindings(document: &Doc) -> Cow<'_, Doc> {
    let bound_nodes = document
        .scene
        .roots()
        .iter()
        .flat_map(|root| document.scene.descendants_of(*root))
        .filter_map(|id| {
            let node = document.scene.get(id)?;
            (!node.bindings.is_empty()).then_some((id, node.bindings.clone()))
        })
        .collect::<Vec<_>>();
    if bound_nodes.is_empty() {
        return Cow::Borrowed(document);
    }

    let mut resolved = document.clone();
    for (id, bindings) in bound_nodes {
        let values = bindings
            .into_iter()
            .filter_map(|(property, variable)| {
                resolve_bound_value(
                    &document.variables,
                    &document.scene,
                    id,
                    &document.active_modes,
                    variable,
                )
                .map(|value| (property, value))
            })
            .collect::<Vec<_>>();
        let Some(node) = resolved.scene.get_mut(id) else {
            continue;
        };
        for (property, value) in values {
            property.apply_resolved(node, value);
        }
    }
    Cow::Owned(resolved)
}

/// The document with `root` lifted to the top level at its world position,
/// so a render of it alone draws it exactly as it appears on its page,
/// themed by the modes it inherits there.
pub fn detach_export_root<'a>(
    document: &'a Doc,
    root: Option<NodeId>,
    name: &str,
) -> Result<Cow<'a, Doc>> {
    let Some(root) = root else {
        return Ok(Cow::Borrowed(document));
    };
    let node = document
        .scene
        .get(root)
        .with_context(|| format!("export target {} no longer exists", name))?;
    if node.parent.is_none() {
        return Ok(Cow::Borrowed(document));
    }

    let world_transform = document
        .scene
        .world_transform(root)
        .with_context(|| format!("{} has no world transform", name))?;
    let inherited_modes = document
        .variables
        .collections
        .values()
        .map(|collection| {
            (
                collection.id,
                resolve_effective_mode(&document.scene, root, collection, &document.active_modes),
            )
        })
        .collect();
    let mut detached = document.clone();
    detached.active_modes = inherited_modes;
    let root_index = detached.scene.next_child_index(None);
    detached
        .scene
        .set_parent(root, None, root_index)
        .with_context(|| format!("detaching {} for export", name))?;
    detached
        .scene
        .get_mut(root)
        .with_context(|| format!("detached target {} disappeared", name))?
        .transform = world_transform;
    Ok(Cow::Owned(detached))
}

/// What a page or node draws, in world units.
pub fn page_visual_bounds(document: &Doc, root: Option<NodeId>) -> Option<Bounds> {
    match root {
        Some(root) => visual_world_bounds(&document.scene, root, 0.0),
        None => document
            .scene
            .roots()
            .iter()
            .filter_map(|root| visual_world_bounds(&document.scene, *root, 0.0))
            .reduce(|left, right| left.union(&right)),
    }
}

pub fn raster_dimensions(
    bounds: Bounds,
    sizing: impl Into<ExportSizing>,
    name: &str,
) -> Result<(u32, u32)> {
    let sizing = sizing.into();
    let multiplier = sizing.zoom(bounds)?;
    let (width, height) = match sizing {
        ExportSizing::Scale(_) | ExportSizing::MaxDimension(_) => (
            (bounds.width() * multiplier).ceil(),
            (bounds.height() * multiplier).ceil(),
        ),
        ExportSizing::Width(width) => (width.ceil(), (bounds.height() * multiplier).ceil()),
        ExportSizing::Height(height) => ((bounds.width() * multiplier).ceil(), height.ceil()),
    };
    let width = width.max(1.0);
    let height = height.max(1.0);
    if width > f64::from(MAX_EXPORT_PIXELS) || height > f64::from(MAX_EXPORT_PIXELS) {
        bail!(
            "{name} at {} would be {width:.0}×{height:.0} pixels; the per-side limit is {MAX_EXPORT_PIXELS}. Choose a smaller scale",
            sizing.label()
        );
    }
    let pixels = (width as u64)
        .checked_mul(height as u64)
        .context("export dimensions overflowed the pixel budget")?;
    if pixels > MAX_EXPORT_TOTAL_PIXELS {
        bail!(
            "{name} at {} would contain {pixels} pixels; the safe limit is {MAX_EXPORT_TOTAL_PIXELS}. Choose a smaller scale",
            sizing.label()
        );
    }
    Ok((width as u32, height as u32))
}

pub fn checked_vector_dimension(value: f64, axis: &str) -> Result<u32> {
    if value > f64::from(i32::MAX) {
        bail!("{axis} is too large to export");
    }
    Ok((value.ceil() as u32).max(1))
}

/// The world region an export shows: a node, a page, or every root.
#[derive(Clone, Copy, Debug)]
pub struct ExportRegion<'a> {
    /// The node or page root drawn; `None` draws every root.
    pub root: Option<NodeId>,
    /// The world rectangle the output covers.
    pub bounds: Bounds,
    /// What error messages call the region.
    pub name: &'a str,
}

pub fn render_png(
    document: &Doc,
    assets: Option<Arc<dyn AssetResolver>>,
    region: ExportRegion<'_>,
    sizing: ExportSizing,
) -> Result<Vec<u8>> {
    let mut renderer = render_raster(document, assets, region, sizing)?;
    renderer
        .encode_png()
        .map_err(|error| anyhow::anyhow!("encoding export PNG: {error}"))
}

/// A JPEG at `quality` (1–100), with transparency composited onto white.
pub fn render_jpeg(
    document: &Doc,
    assets: Option<Arc<dyn AssetResolver>>,
    region: ExportRegion<'_>,
    sizing: ExportSizing,
    quality: u8,
) -> Result<Vec<u8>> {
    let mut renderer = render_raster(document, assets, region, sizing)?;
    let (width, height) = raster_dimensions(region.bounds, sizing, region.name)?;
    let rgba = renderer.copy_rgba();
    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    for pixel in rgba.chunks_exact(4) {
        let alpha = u16::from(pixel[3]);
        for channel in &pixel[..3] {
            let composited = (u16::from(*channel) * alpha + 255 * (255 - alpha) + 127) / 255;
            rgb.push(composited as u8);
        }
    }

    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, quality.clamp(1, 100))
        .encode(&rgb, width, height, image::ExtendedColorType::Rgb8)
        .context("encoding export JPEG")?;
    Ok(jpeg)
}

/// An SVG at the design's own size, with text converted to paths.
pub fn render_svg(
    document: &Doc,
    assets: Option<Arc<dyn AssetResolver>>,
    region: ExportRegion<'_>,
) -> Result<Vec<u8>> {
    let width = checked_vector_dimension(region.bounds.width(), "SVG width")?;
    let height = checked_vector_dimension(region.bounds.height(), "SVG height")?;
    let mut renderer = RasterRenderer::new(width, height)
        .map_err(|error| anyhow::anyhow!("creating SVG renderer: {error}"))?;
    if let Some(assets) = assets {
        renderer.set_asset_resolver(assets);
    }
    let flags = skia_safe::svg::canvas::Flags::CONVERT_TEXT_TO_PATHS;
    let canvas = skia_safe::svg::Canvas::new(
        skia_safe::Rect::from_size((width as f32, height as f32)),
        Some(flags),
    );
    let viewport = region_viewport(region, 1.0);
    let inputs = render_inputs(document);
    renderer.render_to_canvas(
        &canvas,
        width,
        height,
        &document.scene,
        &viewport,
        region.root,
        &inputs,
    );
    Ok(canvas.end().as_bytes().to_vec())
}

/// A one-page PDF at the design's own size.
pub fn render_pdf(
    document: &Doc,
    assets: Option<Arc<dyn AssetResolver>>,
    region: ExportRegion<'_>,
) -> Result<Vec<u8>> {
    let width = checked_vector_dimension(region.bounds.width(), "PDF width")?;
    let height = checked_vector_dimension(region.bounds.height(), "PDF height")?;
    let mut renderer = RasterRenderer::new(width, height)
        .map_err(|error| anyhow::anyhow!("creating PDF renderer: {error}"))?;
    if let Some(assets) = assets {
        renderer.set_asset_resolver(assets);
    }

    let mut pdf = Vec::new();
    {
        let pdf_document = skia_safe::pdf::new_document(&mut pdf, None);
        let mut page = pdf_document.begin_page((width as f32, height as f32), None);
        let viewport = region_viewport(region, 1.0);
        let inputs = render_inputs(document);
        renderer.render_to_canvas(
            page.canvas(),
            width,
            height,
            &document.scene,
            &viewport,
            region.root,
            &inputs,
        );
        page.end_page().close();
    }
    Ok(pdf)
}

fn render_raster(
    document: &Doc,
    assets: Option<Arc<dyn AssetResolver>>,
    region: ExportRegion<'_>,
    sizing: ExportSizing,
) -> Result<RasterRenderer> {
    let (width, height) = raster_dimensions(region.bounds, sizing, region.name)?;
    let zoom = sizing.zoom(region.bounds)?;
    let mut renderer = RasterRenderer::new(width, height)
        .map_err(|error| anyhow::anyhow!("creating {width}x{height} export surface: {error}"))?;
    if let Some(assets) = assets {
        renderer.set_asset_resolver(assets);
    }
    let viewport = region_viewport(region, zoom);
    let inputs = render_inputs(document);
    renderer.render_page_with(&document.scene, &viewport, region.root, &inputs);
    Ok(renderer)
}

/// The viewport that centres `region` at `zoom`.
pub fn region_viewport(region: ExportRegion<'_>, zoom: f64) -> Viewport {
    let center = region.bounds.center();
    Viewport {
        center: [center.x, center.y],
        zoom,
    }
}

/// What an export is encoded as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportEncoding {
    Png,
    /// JPEG at a quality from 1 to 100.
    Jpeg {
        quality: u8,
    },
    Svg,
    Pdf,
}

/// An exported image and the region it shows.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportedImage {
    pub bytes: Vec<u8>,
    /// Pixels for PNG and JPEG; design units for SVG and PDF.
    pub width: u32,
    pub height: u32,
    /// The world rectangle the image shows.
    pub bounds: Bounds,
}

/// Export the node `root` (a page root exports the page) the way the editor's
/// export menu does: the region is what the node draws, with variable values
/// applied, and the node is drawn alone at its place on the page. `sizing`
/// applies to PNG and JPEG; SVG and PDF use the design's own size.
pub fn export_node(
    document: &Doc,
    assets: Option<Arc<dyn AssetResolver>>,
    root: NodeId,
    encoding: ExportEncoding,
    sizing: ExportSizing,
) -> Result<ExportedImage> {
    let node = document
        .scene
        .get(root)
        .with_context(|| format!("node {root} does not exist"))?;
    let name = if node.name.is_empty() {
        "Untitled"
    } else {
        node.name.as_str()
    };
    let resolved = resolve_export_bindings(document);
    let bounds = page_visual_bounds(&resolved, Some(root))
        .filter(|bounds| bounds.is_finite() && bounds.width() > 0.0 && bounds.height() > 0.0)
        .with_context(|| format!("{name} draws nothing to export"))?;
    let detached = detach_export_root(document, Some(root), name)?;
    let region = ExportRegion {
        root: Some(root),
        bounds,
        name,
    };
    let (bytes, (width, height)) = match encoding {
        ExportEncoding::Png => (
            render_png(&detached, assets, region, sizing)?,
            raster_dimensions(bounds, sizing, name)?,
        ),
        ExportEncoding::Jpeg { quality } => (
            render_jpeg(&detached, assets, region, sizing, quality)?,
            raster_dimensions(bounds, sizing, name)?,
        ),
        ExportEncoding::Svg => (
            render_svg(&detached, assets, region)?,
            vector_dimensions(bounds)?,
        ),
        ExportEncoding::Pdf => (
            render_pdf(&detached, assets, region)?,
            vector_dimensions(bounds)?,
        ),
    };
    Ok(ExportedImage {
        bytes,
        width,
        height,
        bounds,
    })
}

fn vector_dimensions(bounds: Bounds) -> Result<(u32, u32)> {
    Ok((
        checked_vector_dimension(bounds.width(), "vector width")?,
        checked_vector_dimension(bounds.height(), "vector height")?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{
        CanvasNode, GroupNode, NodeData, Operation, Transform2D, VectorNode, color::Color,
    };

    fn doc_with_card() -> (Doc, NodeId) {
        let mut doc = Doc::new();
        let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let page_id = page.id;
        doc.apply(Operation::create_node(page)).expect("page");
        doc.add_page(page_id);
        let mut card = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            100.0,
            50.0,
            Color::rgb(255, 0, 0),
        )));
        card.name = "Card".into();
        card.parent = Some(page_id);
        card.transform = Transform2D::translation(10.0, 20.0);
        let card_id = card.id;
        doc.apply(Operation::create_node(card)).expect("card");
        (doc, card_id)
    }

    #[test]
    fn png_exports_honor_a_target_width() {
        let (doc, card) = doc_with_card();
        let exported = export_node(
            &doc,
            None,
            card,
            ExportEncoding::Png,
            ExportSizing::Width(200.0),
        )
        .expect("export");
        assert_eq!((exported.width, exported.height), (200, 100));
        assert!(exported.bytes.starts_with(b"\x89PNG"));
        let decoded = image::load_from_memory(&exported.bytes).expect("decode");
        assert_eq!((decoded.width(), decoded.height()), (200, 100));
        assert_eq!(decoded.to_rgba8().get_pixel(100, 50).0, [255, 0, 0, 255]);
    }

    #[test]
    fn jpeg_exports_composite_onto_white_at_the_requested_quality() {
        let (doc, card) = doc_with_card();
        let high = export_node(
            &doc,
            None,
            card,
            ExportEncoding::Jpeg { quality: 95 },
            ExportSizing::Scale(1.0),
        )
        .expect("export");
        let low = export_node(
            &doc,
            None,
            card,
            ExportEncoding::Jpeg { quality: 10 },
            ExportSizing::Scale(1.0),
        )
        .expect("export");
        assert!(high.bytes.starts_with(&[0xFF, 0xD8]));
        assert!(low.bytes.len() < high.bytes.len());
        let decoded = image::load_from_memory(&high.bytes)
            .expect("decode")
            .to_rgb8();
        let [red, green, blue] = decoded.get_pixel(50, 25).0;
        assert!(red > 240 && green < 20 && blue < 20, "{red} {green} {blue}");
    }

    #[test]
    fn svg_and_pdf_exports_carry_their_headers() {
        let (doc, card) = doc_with_card();
        let svg = export_node(
            &doc,
            None,
            card,
            ExportEncoding::Svg,
            ExportSizing::Scale(1.0),
        )
        .expect("svg");
        assert!(String::from_utf8_lossy(&svg.bytes).contains("<svg"));
        assert_eq!((svg.width, svg.height), (100, 50));
        let pdf = export_node(
            &doc,
            None,
            card,
            ExportEncoding::Pdf,
            ExportSizing::Scale(1.0),
        )
        .expect("pdf");
        assert!(pdf.bytes.starts_with(b"%PDF"));
    }

    #[test]
    fn oversized_exports_are_refused() {
        let (doc, card) = doc_with_card();
        let error = export_node(
            &doc,
            None,
            card,
            ExportEncoding::Png,
            ExportSizing::Scale(1000.0),
        )
        .expect_err("too large");
        assert!(error.to_string().contains("limit"), "{error}");
    }

    #[test]
    fn a_max_dimension_scales_down_and_never_up() {
        let wide = Bounds::from_xywh(0.0, 0.0, 4000.0, 1000.0);
        assert_eq!(
            raster_dimensions(wide, ExportSizing::MaxDimension(1568.0), "Banner").expect("fits"),
            (1568, 392)
        );
        assert_eq!(
            raster_dimensions(wide, ExportSizing::MaxDimension(8000.0), "Banner").expect("fits"),
            (4000, 1000)
        );
        assert!(raster_dimensions(wide, ExportSizing::MaxDimension(0.0), "Banner").is_err());
    }
}
