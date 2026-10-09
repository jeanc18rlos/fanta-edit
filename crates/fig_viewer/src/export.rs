use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use fanta_doc::{Bounds, Doc, NodeId, Viewport, resolve_bound_value, resolve_effective_mode};
use fanta_render::{AssetResolver, RasterRenderer, RenderInputs, visual_world_bounds};
use image::codecs::jpeg::JpegEncoder;
use tempfile::NamedTempFile;

use crate::document::{FigPage, page_bounds};

const MAX_EXPORT_PIXELS: u32 = 8192;
const MAX_EXPORT_TOTAL_PIXELS: u64 = 16 * 1024 * 1024;
const JPEG_QUALITY: u8 = 95;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ExportFormat {
    Png,
    Jpeg,
    Svg,
    Pdf,
}

impl ExportFormat {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpeg => "JPG",
            Self::Svg => "SVG",
            Self::Pdf => "PDF",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Svg => "svg",
            Self::Pdf => "pdf",
        }
    }

    pub(crate) fn is_raster(self) -> bool {
        matches!(self, Self::Png | Self::Jpeg)
    }
}

pub(crate) const EXPORT_FORMATS: &[(ExportFormat, &str)] = &[
    (ExportFormat::Png, "PNG"),
    (ExportFormat::Jpeg, "JPG"),
    (ExportFormat::Svg, "SVG"),
    (ExportFormat::Pdf, "PDF"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ExportScale {
    One,
    Two,
    Four,
}

impl ExportScale {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::One => "1x",
            Self::Two => "2x",
            Self::Four => "4x",
        }
    }

    fn multiplier(self) -> f64 {
        match self {
            Self::One => 1.0,
            Self::Two => 2.0,
            Self::Four => 4.0,
        }
    }

    fn file_suffix(self) -> &'static str {
        match self {
            Self::One => "",
            Self::Two => "@2x",
            Self::Four => "@4x",
        }
    }
}

pub(crate) const EXPORT_SCALES: &[(ExportScale, &str)] = &[
    (ExportScale::One, "1x"),
    (ExportScale::Two, "2x"),
    (ExportScale::Four, "4x"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ExportPreset {
    pub(crate) format: ExportFormat,
    pub(crate) scale: ExportScale,
}

impl Default for ExportPreset {
    fn default() -> Self {
        Self {
            format: ExportFormat::Png,
            scale: ExportScale::Two,
        }
    }
}

pub(crate) struct ExportBatch {
    doc: Doc,
    asset_resolver: Option<Arc<dyn AssetResolver>>,
    targets: Vec<ExportTarget>,
    requests: Vec<ExportRequest>,
    output_directory: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ExportSizing {
    Scale(f64),
    Width(f64),
    Height(f64),
}

impl From<ExportScale> for ExportSizing {
    fn from(scale: ExportScale) -> Self {
        Self::Scale(scale.multiplier())
    }
}

impl ExportSizing {
    fn zoom(self, bounds: Bounds) -> Result<f64> {
        let zoom = match self {
            Self::Scale(scale) => scale,
            Self::Width(width) => width / bounds.width(),
            Self::Height(height) => height / bounds.height(),
        };
        if !zoom.is_finite() || zoom <= 0.0 {
            bail!("export size must be a finite positive value");
        }
        Ok(zoom)
    }

    fn label(self) -> String {
        match self {
            Self::Scale(scale) => format!("{scale}x"),
            Self::Width(width) => format!("{width}w"),
            Self::Height(height) => format!("{height}h"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ExportRequest {
    pub(crate) format: ExportFormat,
    pub(crate) sizing: ExportSizing,
    pub(crate) suffix: String,
}

struct ExportTarget {
    root: Option<NodeId>,
    name: String,
    bounds: Bounds,
}

pub(crate) fn prepare_export_jobs(
    doc: &Doc,
    asset_resolver: Option<Arc<dyn AssetResolver>>,
    page: Option<&FigPage>,
    project_root: PathBuf,
    presets: &[ExportPreset],
) -> Result<ExportBatch> {
    let requests = presets
        .iter()
        .map(|preset| ExportRequest {
            format: preset.format,
            sizing: preset.scale.into(),
            suffix: if preset.format.is_raster() {
                preset.scale.file_suffix().to_string()
            } else {
                String::new()
            },
        })
        .collect::<Vec<_>>();
    prepare_export_requests(
        doc,
        asset_resolver,
        page,
        project_root.join("exports"),
        &requests,
    )
}

pub(crate) fn prepare_export_requests(
    doc: &Doc,
    asset_resolver: Option<Arc<dyn AssetResolver>>,
    page: Option<&FigPage>,
    output_directory: PathBuf,
    requests: &[ExportRequest],
) -> Result<ExportBatch> {
    if requests.is_empty() {
        bail!("add at least one export setting");
    }

    let resolved_bounds_document = resolve_export_bindings(doc);
    let bounds_document = resolved_bounds_document.as_ref();
    let selected: Vec<_> = doc.selection.iter().copied().collect();
    let targets = if selected.is_empty() {
        let root = page.and_then(|page| page.root);
        let name = page
            .map(|page| page.name.to_string())
            .unwrap_or_else(|| "Page".to_string());
        let bounds = page_visual_bounds(bounds_document, root)
            .unwrap_or_else(|| page_bounds(bounds_document, root));
        ensure_exportable_bounds(&name, bounds)?;
        vec![ExportTarget {
            root,
            name: sanitize_file_name(&name),
            bounds,
        }]
    } else {
        let mut targets = Vec::with_capacity(selected.len());
        for id in selected {
            let node = doc
                .scene
                .get(id)
                .with_context(|| format!("selected layer {id} no longer exists"))?;
            let name = if node.name.is_empty() {
                "Untitled".to_string()
            } else {
                node.name.clone()
            };
            let bounds = visual_world_bounds(&bounds_document.scene, id, 0.0)
                .with_context(|| format!("{name} has no visible bounds"))?;
            ensure_exportable_bounds(&name, bounds)?;
            targets.push((id, name, bounds));
        }

        let unique_names = unique_export_names(targets.iter().map(|(_, name, _)| name.as_str()));
        targets
            .into_iter()
            .zip(unique_names)
            .map(|((root, _, bounds), name)| ExportTarget {
                root: Some(root),
                name,
                bounds,
            })
            .collect()
    };

    validate_export_requests(&targets, requests)?;

    Ok(ExportBatch {
        doc: doc.clone(),
        asset_resolver,
        targets,
        requests: requests.to_vec(),
        output_directory,
    })
}

fn resolve_export_bindings(document: &Doc) -> Cow<'_, Doc> {
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

impl ExportBatch {
    pub(crate) fn with_output_directory(mut self, output_directory: PathBuf) -> Self {
        self.output_directory = output_directory;
        self
    }

    pub(crate) fn len(&self) -> usize {
        self.targets.len().saturating_mul(self.requests.len())
    }

    pub(crate) fn format_summary(&self) -> String {
        let mut labels = Vec::new();
        for request in &self.requests {
            let label = if request.format.is_raster() {
                format!("{} {}", request.format.label(), request.sizing.label())
            } else {
                request.format.label().to_string()
            };
            if !labels.contains(&label) {
                labels.push(label);
            }
        }
        labels.join(", ")
    }
}

pub(crate) fn run_export_jobs(batch: ExportBatch) -> Result<Vec<PathBuf>> {
    if batch.targets.is_empty() || batch.requests.is_empty() {
        bail!("there is nothing to export");
    }

    let exports_dir = &batch.output_directory;
    std::fs::create_dir_all(&exports_dir)
        .with_context(|| format!("creating {}", exports_dir.display()))?;
    let file_names = output_file_names(&batch.targets, &batch.requests);
    let mut file_names = file_names.into_iter();
    let mut paths = Vec::with_capacity(batch.len());

    for target in &batch.targets {
        let document = render_document_for_target(&batch.doc, target)?;
        for request in &batch.requests {
            let file_name = file_names
                .next()
                .context("export filename generation was incomplete")?;
            let bytes = render_export(&batch, &document, target, request).with_context(|| {
                format!("exporting {} as {}", target.name, request.format.label())
            })?;
            let path = write_export_atomically(exports_dir, &file_name, &bytes)
                .with_context(|| format!("writing {file_name}"))?;
            paths.push(path);
        }
    }
    Ok(paths)
}

fn render_export(
    batch: &ExportBatch,
    document: &Doc,
    target: &ExportTarget,
    request: &ExportRequest,
) -> Result<Vec<u8>> {
    match request.format {
        ExportFormat::Png => render_png(batch, document, target, request.sizing),
        ExportFormat::Jpeg => render_jpeg(batch, document, target, request.sizing),
        ExportFormat::Svg => render_svg(batch, document, target),
        ExportFormat::Pdf => render_pdf(batch, document, target),
    }
}

fn render_png(
    batch: &ExportBatch,
    document: &Doc,
    target: &ExportTarget,
    sizing: ExportSizing,
) -> Result<Vec<u8>> {
    let mut renderer = render_raster(batch, document, target, sizing)?;
    renderer
        .encode_png()
        .map_err(|error| anyhow::anyhow!("encoding export PNG: {error}"))
}

fn render_jpeg(
    batch: &ExportBatch,
    document: &Doc,
    target: &ExportTarget,
    sizing: ExportSizing,
) -> Result<Vec<u8>> {
    let mut renderer = render_raster(batch, document, target, sizing)?;
    let (width, height) = raster_dimensions(target.bounds, sizing, &target.name)?;
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
    JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY)
        .encode(&rgb, width, height, image::ExtendedColorType::Rgb8)
        .context("encoding export JPEG")?;
    Ok(jpeg)
}

fn render_raster(
    batch: &ExportBatch,
    document: &Doc,
    target: &ExportTarget,
    sizing: ExportSizing,
) -> Result<RasterRenderer> {
    let (width, height) = raster_dimensions(target.bounds, sizing, &target.name)?;
    let zoom = sizing.zoom(target.bounds)?;
    let mut renderer = RasterRenderer::new(width, height)
        .map_err(|error| anyhow::anyhow!("creating {width}x{height} export surface: {error}"))?;
    if let Some(asset_resolver) = batch.asset_resolver.clone() {
        renderer.set_asset_resolver(asset_resolver);
    }
    let viewport = target_viewport(target, zoom);
    let inputs = render_inputs(document);
    renderer.render_page_with(&document.scene, &viewport, target.root, &inputs);
    Ok(renderer)
}

fn render_svg(batch: &ExportBatch, document: &Doc, target: &ExportTarget) -> Result<Vec<u8>> {
    let width = checked_vector_dimension(target.bounds.width(), "SVG width")?;
    let height = checked_vector_dimension(target.bounds.height(), "SVG height")?;
    let mut renderer = RasterRenderer::new(width, height)
        .map_err(|error| anyhow::anyhow!("creating SVG renderer: {error}"))?;
    if let Some(asset_resolver) = batch.asset_resolver.clone() {
        renderer.set_asset_resolver(asset_resolver);
    }
    let flags = skia_safe::svg::canvas::Flags::CONVERT_TEXT_TO_PATHS;
    let canvas = skia_safe::svg::Canvas::new(
        skia_safe::Rect::from_size((width as f32, height as f32)),
        Some(flags),
    );
    let viewport = target_viewport(target, 1.0);
    let inputs = render_inputs(document);
    renderer.render_to_canvas(
        &canvas,
        width,
        height,
        &document.scene,
        &viewport,
        target.root,
        &inputs,
    );
    Ok(canvas.end().as_bytes().to_vec())
}

fn render_pdf(batch: &ExportBatch, document: &Doc, target: &ExportTarget) -> Result<Vec<u8>> {
    let width = checked_vector_dimension(target.bounds.width(), "PDF width")?;
    let height = checked_vector_dimension(target.bounds.height(), "PDF height")?;
    let mut renderer = RasterRenderer::new(width, height)
        .map_err(|error| anyhow::anyhow!("creating PDF renderer: {error}"))?;
    if let Some(asset_resolver) = batch.asset_resolver.clone() {
        renderer.set_asset_resolver(asset_resolver);
    }

    let mut pdf = Vec::new();
    {
        let pdf_document = skia_safe::pdf::new_document(&mut pdf, None);
        let mut page = pdf_document.begin_page((width as f32, height as f32), None);
        let viewport = target_viewport(target, 1.0);
        let inputs = render_inputs(document);
        renderer.render_to_canvas(
            page.canvas(),
            width,
            height,
            &document.scene,
            &viewport,
            target.root,
            &inputs,
        );
        page.end_page().close();
    }
    Ok(pdf)
}

pub(crate) fn render_inputs(document: &Doc) -> RenderInputs<'_> {
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

fn target_viewport(target: &ExportTarget, zoom: f64) -> Viewport {
    let center = target.bounds.center();
    Viewport {
        center: [center.x, center.y],
        zoom,
    }
}

fn render_document_for_target<'a>(
    document: &'a Doc,
    target: &ExportTarget,
) -> Result<Cow<'a, Doc>> {
    let Some(root) = target.root else {
        return Ok(Cow::Borrowed(document));
    };
    let node = document
        .scene
        .get(root)
        .with_context(|| format!("export target {} no longer exists", target.name))?;
    if node.parent.is_none() {
        return Ok(Cow::Borrowed(document));
    }

    let world_transform = document
        .scene
        .world_transform(root)
        .with_context(|| format!("{} has no world transform", target.name))?;
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
        .with_context(|| format!("detaching {} for export", target.name))?;
    detached
        .scene
        .get_mut(root)
        .with_context(|| format!("detached target {} disappeared", target.name))?
        .transform = world_transform;
    Ok(Cow::Owned(detached))
}

fn page_visual_bounds(document: &Doc, root: Option<NodeId>) -> Option<Bounds> {
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

fn validate_export_requests(targets: &[ExportTarget], requests: &[ExportRequest]) -> Result<()> {
    for target in targets {
        for request in requests {
            if request.format.is_raster() {
                raster_dimensions(target.bounds, request.sizing, &target.name)?;
            } else {
                checked_vector_dimension(target.bounds.width(), "vector width")?;
                checked_vector_dimension(target.bounds.height(), "vector height")?;
            }
        }
    }
    Ok(())
}

fn raster_dimensions(
    bounds: Bounds,
    sizing: impl Into<ExportSizing>,
    name: &str,
) -> Result<(u32, u32)> {
    let sizing = sizing.into();
    let multiplier = sizing.zoom(bounds)?;
    let (width, height) = match sizing {
        ExportSizing::Scale(_) => (
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

fn checked_vector_dimension(value: f64, axis: &str) -> Result<u32> {
    if value > f64::from(i32::MAX) {
        bail!("{axis} is too large to export");
    }
    Ok((value.ceil() as u32).max(1))
}

fn write_export_atomically(directory: &Path, file_name: &str, bytes: &[u8]) -> Result<PathBuf> {
    let mut temporary = NamedTempFile::new_in(directory)
        .with_context(|| format!("creating a temporary export in {}", directory.display()))?;
    if let Err(error) = temporary.write_all(bytes) {
        return Err(discard_failed_temporary(
            temporary,
            anyhow::Error::new(error).context("writing the temporary export"),
        ));
    }
    if let Err(error) = temporary.as_file().sync_all() {
        return Err(discard_failed_temporary(
            temporary,
            anyhow::Error::new(error).context("syncing the temporary export"),
        ));
    }

    let mut attempt = 1_usize;
    loop {
        let destination = match collision_path(directory, file_name, attempt) {
            Ok(destination) => destination,
            Err(error) => return Err(discard_failed_temporary(temporary, error)),
        };
        match temporary.persist_noclobber(&destination) {
            Ok(_) => return Ok(destination),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                temporary = error.file;
                attempt = match attempt.checked_add(1) {
                    Some(attempt) => attempt,
                    None => {
                        return Err(discard_failed_temporary(
                            temporary,
                            anyhow::anyhow!("too many colliding export filenames"),
                        ));
                    }
                };
            }
            Err(error) => {
                let primary = anyhow::Error::new(error.error)
                    .context(format!("publishing {}", destination.display()));
                return Err(discard_failed_temporary(error.file, primary));
            }
        }
    }
}

fn discard_failed_temporary(temporary: NamedTempFile, primary: anyhow::Error) -> anyhow::Error {
    match temporary.close() {
        Ok(()) => primary,
        Err(cleanup) => {
            anyhow::anyhow!("{primary:#}; also failed to remove the temporary export: {cleanup}")
        }
    }
}

fn collision_path(directory: &Path, file_name: &str, attempt: usize) -> Result<PathBuf> {
    if attempt == 1 {
        return Ok(directory.join(file_name));
    }
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .context("export filename has no valid stem")?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .context("export filename has no valid extension")?;
    Ok(directory.join(format!("{stem}-{attempt}.{extension}")))
}

fn ensure_exportable_bounds(name: &str, bounds: Bounds) -> Result<()> {
    if !bounds.is_finite() || bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        bail!("{name} has invalid or empty bounds");
    }
    Ok(())
}

fn output_file_names(targets: &[ExportTarget], requests: &[ExportRequest]) -> Vec<String> {
    targets
        .iter()
        .flat_map(|target| {
            requests.iter().map(move |request| {
                let suffix = if request.suffix.is_empty() {
                    String::new()
                } else {
                    sanitize_file_name(&request.suffix)
                };
                format!("{}{suffix}.{}", target.name, request.format.extension())
            })
        })
        .collect()
}

fn unique_export_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut next_suffix_by_base = HashMap::<String, usize>::new();
    let mut used = HashSet::new();
    names
        .into_iter()
        .map(|name| {
            let base = sanitize_file_name(name);
            let key = base.to_lowercase();
            let next_suffix = next_suffix_by_base.entry(key).or_insert(1);
            loop {
                let candidate = if *next_suffix == 1 {
                    base.clone()
                } else {
                    format!("{base}-{}", *next_suffix)
                };
                *next_suffix += 1;
                if used.insert(candidate.to_lowercase()) {
                    break candidate;
                }
            }
        })
        .collect()
}

fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
                )
            {
                '-'
            } else {
                character
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.');
    if cleaned.is_empty() {
        "export".to_string()
    } else {
        cleaned.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{
        Blur, BoundProp, CanvasNode, Color, Fill, GroupNode, Mode, ModeId, NodeData, Shadow,
        ShadowKind, Stroke, StrokeAlign, StrokeJoin, Transform2D, VarValue, Variable,
        VariableCollection, VariableCollectionId, VariableId, VariableType, VectorNode,
    };
    use image::GenericImageView as _;
    use std::collections::BTreeMap;

    fn exportable_frame(name: &str, x: f64) -> CanvasNode {
        let mut node = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([20.0, 10.0]),
            background: Some(Fill::solid(Color::rgb(20, 120, 240))),
            ..GroupNode::default()
        }));
        node.name = name.to_string();
        node.transform = Transform2D::translation(x, 5.0);
        node
    }

    fn insert(doc: &mut Doc, mut node: CanvasNode) -> NodeId {
        node.index = doc.scene.next_child_index(node.parent);
        let id = node.id;
        doc.scene.insert(node).expect("test layer is valid");
        id
    }

    #[test]
    fn multiple_selected_layers_and_presets_get_distinct_jobs() {
        let directory = tempfile::tempdir().expect("temporary export project");
        let mut doc = Doc::new();
        let first = insert(&mut doc, exportable_frame("Card", 0.0));
        let second = insert(&mut doc, exportable_frame("card", 30.0));
        doc.selection.select_only(first);
        doc.selection.toggle(second);
        let presets = [
            ExportPreset {
                format: ExportFormat::Png,
                scale: ExportScale::One,
            },
            ExportPreset::default(),
            ExportPreset::default(),
            ExportPreset {
                format: ExportFormat::Svg,
                scale: ExportScale::Four,
            },
        ];

        let batch = prepare_export_jobs(&doc, None, None, directory.path().to_path_buf(), &presets)
            .expect("selection is exportable");
        assert_eq!(batch.len(), 8);
        assert_eq!(batch.targets[0].name, "Card");
        assert_eq!(batch.targets[1].name, "card-2");
        let paths = run_export_jobs(batch).expect("batch exports without collisions");
        assert_eq!(
            paths
                .iter()
                .map(|path| path.file_name().expect("filename").to_string_lossy())
                .collect::<Vec<_>>(),
            [
                "Card.png",
                "Card@2x.png",
                "Card@2x-2.png",
                "Card.svg",
                "card-2.png",
                "card-2@2x.png",
                "card-2@2x-2.png",
                "card-2.svg",
            ]
        );
    }

    #[test]
    fn raster_presets_write_expected_dimensions_and_jpeg_is_opaque() {
        let directory = tempfile::tempdir().expect("temporary export project");
        let mut doc = Doc::new();
        let frame = insert(&mut doc, exportable_frame("Preview/Card", 15.0));
        doc.selection.select_only(frame);
        let presets = [
            ExportPreset {
                format: ExportFormat::Png,
                scale: ExportScale::One,
            },
            ExportPreset {
                format: ExportFormat::Jpeg,
                scale: ExportScale::Four,
            },
        ];
        let batch = prepare_export_jobs(&doc, None, None, directory.path().to_path_buf(), &presets)
            .expect("frame is exportable");

        let paths = run_export_jobs(batch).expect("raster export succeeds");
        assert_eq!(paths.len(), 2);
        assert_eq!(
            paths[0].file_name().and_then(|name| name.to_str()),
            Some("Preview-Card.png")
        );
        assert_eq!(
            paths[1].file_name().and_then(|name| name.to_str()),
            Some("Preview-Card@4x.jpg")
        );
        assert_eq!(
            image::open(&paths[0])
                .expect("PNG is decodable")
                .dimensions(),
            (20, 10)
        );
        assert_eq!(
            image::open(&paths[1])
                .expect("JPEG is decodable")
                .dimensions(),
            (80, 40)
        );
        assert_eq!(
            image::open(&paths[1]).expect("JPEG is decodable").color(),
            image::ColorType::Rgb8
        );
    }

    #[test]
    fn bitmap_jpeg_center_cover_preserves_source_flat_regions() -> Result<()> {
        use fanta_doc::{AssetId, BitmapNode, ImageFitMode};
        use fanta_render::{DecodedImage, InMemoryAssetResolver};
        use sha2::{Digest as _, Sha256};

        let encoded = include_bytes!("../tests/fixtures/export-bitmap-center-cover.png");
        assert_eq!(
            format!("{:x}", Sha256::digest(encoded)),
            "93487f26aef753b48185a775d007fc39ecb767e685860b6e54905a39db5f5ba0"
        );
        let source = image::load_from_memory(encoded)?.to_rgba8();
        assert_eq!(source.dimensions(), (320, 180));
        let asset: AssetId = "a_01M455JDQQ9YXJBQBZ37RX7D59".parse()?;
        let pixels = Arc::new(source.as_raw().clone());
        let mut resolver = InMemoryAssetResolver::new();
        assert!(
            resolver
                .insert(asset, DecodedImage::new(Arc::clone(&pixels), 320, 180))
                .is_none()
        );
        let resolver = Arc::new(resolver);

        let mut doc = Doc::new();
        let mut bitmap = CanvasNode::new(NodeData::Bitmap(BitmapNode {
            asset,
            natural_size: [320, 180],
            local_size: [220.0, 160.0],
            crop: None,
            fit: ImageFitMode::Fill,
            tint: None,
        }));
        bitmap.id = "n_01M455JDQS8E4SX5C51Z2PK9S0".parse()?;
        bitmap.name = "B01 BITMAP 100% - mixed target".into();
        bitmap.transform = Transform2D::translation(320.0, 120.0);
        bitmap.opacity = 1.0.into();
        let bitmap = insert(&mut doc, bitmap);
        insert(&mut doc, exportable_frame("Unselected", 700.0));
        doc.selection.select_only(bitmap);
        let original_document = serde_json::to_value(&doc)?;

        let directory = tempfile::tempdir()?;
        let exports = directory.path().join("exports");
        std::fs::create_dir(&exports)?;
        let existing = exports.join("B01 BITMAP 100% - mixed target.jpg");
        std::fs::write(&existing, b"existing export must survive")?;
        let batch = prepare_export_jobs(
            &doc,
            Some(resolver.clone()),
            None,
            directory.path().to_path_buf(),
            &[
                ExportPreset {
                    format: ExportFormat::Png,
                    scale: ExportScale::One,
                },
                ExportPreset {
                    format: ExportFormat::Jpeg,
                    scale: ExportScale::One,
                },
            ],
        )?;
        assert_eq!(batch.targets.len(), 1);
        assert_eq!(
            batch.targets.first().context("bitmap target")?.bounds,
            Bounds::from_xywh(320.0, 120.0, 220.0, 160.0)
        );
        let paths = run_export_jobs(batch)?;
        let png_path = exports.join("B01 BITMAP 100% - mixed target.png");
        let jpeg_path = exports.join("B01 BITMAP 100% - mixed target-2.jpg");
        assert_eq!(paths, [png_path.clone(), jpeg_path.clone()]);
        let png = image::open(&png_path)?.to_rgba8();
        let jpeg = image::open(&jpeg_path)?;
        assert_eq!(png.dimensions(), (220, 160));
        assert_eq!(jpeg.dimensions(), (220, 160));
        assert_eq!(jpeg.color(), image::ColorType::Rgb8);
        let jpeg = jpeg.to_rgba8();
        assert!(png.pixels().all(|pixel| pixel[3] == 255));
        assert!(jpeg.pixels().all(|pixel| pixel[3] == 255));

        let mut checked_pixels = Vec::new();
        let mut jpeg_failures = Vec::new();
        for y in 0..160 {
            for x in 0..220 {
                // Preserve the original pointwise mask, including mixed DCT blocks:
                // center-cover crops source x=36.25..283.75 at scale 8/9.
                let source_x = (580 + (2 * x + 1) * 9) / 16;
                let source_y = (2 * y + 1) * 9 / 16;
                if !(5..315).contains(&source_x) || !(5..175).contains(&source_y) {
                    continue;
                }
                let expected = source.get_pixel(source_x, source_y);
                if !(source_y - 5..=source_y + 5).all(|neighbor_y| {
                    (source_x - 5..=source_x + 5)
                        .all(|neighbor_x| source.get_pixel(neighbor_x, neighbor_y) == expected)
                }) {
                    continue;
                }
                checked_pixels.push((x, y));
                let lossless = png.get_pixel(x, y);
                assert!(
                    lossless.0[..3]
                        .iter()
                        .zip(&expected.0[..3])
                        .all(|(actual, expected)| actual.abs_diff(*expected) <= 2),
                    "PNG crop/color mismatch at ({x}, {y}): {lossless:?} versus {expected:?}"
                );
                let actual = jpeg.get_pixel(x, y);
                if actual.0[..3]
                    .iter()
                    .zip(&expected.0[..3])
                    .any(|(actual, expected)| actual.abs_diff(*expected) > 10)
                {
                    jpeg_failures.push((x, y, *expected, *actual));
                }
            }
        }
        assert_eq!(checked_pixels.len(), 13_248);
        for original_failure in [(105, 7), (118, 7), (105, 48), (118, 48)] {
            assert!(checked_pixels.contains(&original_failure));
        }
        assert_eq!(serde_json::to_value(&doc)?, original_document);
        assert_eq!(pixels.as_ref(), source.as_raw());
        assert_eq!(std::fs::read(&existing)?, b"existing export must survive");
        let mut actual_paths = std::fs::read_dir(&exports)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        actual_paths.sort();
        let mut expected_paths = vec![existing, png_path, jpeg_path];
        expected_paths.sort();
        assert_eq!(actual_paths, expected_paths);
        assert!(
            jpeg_failures.is_empty(),
            "{} of 13,248 source-flat pixels exceed JPEG tolerance 10; first: {:?}",
            jpeg_failures.len(),
            jpeg_failures.iter().take(8).collect::<Vec<_>>()
        );
        Ok(())
    }

    #[test]
    fn inspector_requests_export_custom_sizes_and_suffixes_into_chosen_directory() {
        let directory = tempfile::tempdir().expect("temporary export directory");
        let output_directory = directory.path().join("chosen");
        let mut doc = Doc::new();
        let frame = insert(&mut doc, exportable_frame("Badge", 0.0));
        doc.selection.select_only(frame);
        let requests = [
            ExportRequest {
                format: ExportFormat::Png,
                sizing: ExportSizing::Width(40.0),
                suffix: "-wide".into(),
            },
            ExportRequest {
                format: ExportFormat::Jpeg,
                sizing: ExportSizing::Height(30.0),
                suffix: "-tall".into(),
            },
            ExportRequest {
                format: ExportFormat::Svg,
                sizing: ExportSizing::Scale(1.0),
                suffix: "-vector".into(),
            },
            ExportRequest {
                format: ExportFormat::Pdf,
                sizing: ExportSizing::Scale(1.0),
                suffix: String::new(),
            },
        ];
        let batch = prepare_export_requests(&doc, None, None, output_directory.clone(), &requests)
            .expect("inspector requests are valid");
        let paths = run_export_jobs(batch).expect("each requested file is written");
        assert_eq!(
            paths,
            [
                output_directory.join("Badge-wide.png"),
                output_directory.join("Badge-tall.jpg"),
                output_directory.join("Badge-vector.svg"),
                output_directory.join("Badge.pdf"),
            ]
        );
        assert_eq!(
            image::open(&paths[0])
                .expect("PNG is decodable")
                .dimensions(),
            (40, 20)
        );
        assert_eq!(
            image::open(&paths[1])
                .expect("JPEG is decodable")
                .dimensions(),
            (60, 30)
        );
        assert!(
            std::fs::read_to_string(&paths[2])
                .expect("SVG is readable")
                .contains("<svg")
        );
        assert!(
            std::fs::read(&paths[3])
                .expect("PDF is readable")
                .starts_with(b"%PDF-")
        );
    }

    #[test]
    fn inspector_export_rejects_invalid_and_oversized_sizing() {
        let bounds = Bounds::from_xywh(0.0, 0.0, 20.0, 10.0);
        for sizing in [
            ExportSizing::Scale(0.0),
            ExportSizing::Width(f64::NAN),
            ExportSizing::Height(-1.0),
        ] {
            assert!(raster_dimensions(bounds, sizing, "Badge").is_err());
        }
        assert!(
            raster_dimensions(bounds, ExportSizing::Width(100_000.0), "Badge")
                .expect_err("large exports are rejected")
                .to_string()
                .contains("per-side limit")
        );
    }

    #[test]
    fn export_bounds_resolve_active_variable_modes_before_sizing_the_surface() {
        let directory = tempfile::tempdir().expect("temporary export project");
        let collection = VariableCollectionId::new();
        let mode = ModeId::new();
        let width = VariableId::new();
        let mut doc = Doc::new();
        doc.variables.collections.insert(
            collection,
            VariableCollection {
                id: collection,
                name: "Layout".to_string(),
                modes: vec![Mode {
                    id: mode,
                    name: "Wide".to_string(),
                }],
                default_mode: mode,
                variable_order: vec![width],
            },
        );
        doc.variables.variables.insert(
            width,
            Variable {
                id: width,
                collection,
                name: "Frame width".to_string(),
                ty: VariableType::Float,
                values_by_mode: BTreeMap::from([(mode, VarValue::Float { value: 60.0 })]),
                scopes: Vec::new(),
            },
        );
        doc.active_modes.insert(collection, mode);
        let mut frame = exportable_frame("Bound frame", 0.0);
        frame.bindings.insert(BoundProp::ClipWidth, width);
        let frame = insert(&mut doc, frame);
        doc.selection.select_only(frame);

        let batch = prepare_export_jobs(
            &doc,
            None,
            None,
            directory.path().to_path_buf(),
            &[ExportPreset {
                format: ExportFormat::Png,
                scale: ExportScale::One,
            }],
        )
        .expect("bound frame is exportable");
        assert_eq!(batch.targets[0].bounds.width(), 60.0);
        let paths = run_export_jobs(batch).expect("bound frame export succeeds");
        assert_eq!(
            image::open(&paths[0])
                .expect("bound frame PNG is decodable")
                .dimensions(),
            (60, 10)
        );
    }

    #[test]
    fn svg_and_pdf_use_vector_output_and_ignore_raster_scale() {
        let directory = tempfile::tempdir().expect("temporary export project");
        let mut doc = Doc::new();
        let frame = insert(&mut doc, exportable_frame("Vector", 15.0));
        doc.selection.select_only(frame);
        let batch = prepare_export_jobs(
            &doc,
            None,
            None,
            directory.path().to_path_buf(),
            &[
                ExportPreset {
                    format: ExportFormat::Svg,
                    scale: ExportScale::Four,
                },
                ExportPreset {
                    format: ExportFormat::Pdf,
                    scale: ExportScale::Four,
                },
            ],
        )
        .expect("frame is exportable");

        let paths = run_export_jobs(batch).expect("SVG export succeeds");
        assert_eq!(
            paths[0].file_name().and_then(|name| name.to_str()),
            Some("Vector.svg")
        );
        let svg = std::fs::read_to_string(&paths[0]).expect("SVG is UTF-8");
        assert!(svg.contains("<svg"));
        assert!(svg.contains("width=\"20\""));
        assert!(svg.contains("height=\"10\""));
        assert!(!svg.contains("<image"), "solid frames remain vector output");
        assert_eq!(
            paths[1].file_name().and_then(|name| name.to_str()),
            Some("Vector.pdf")
        );
        let pdf = std::fs::read(&paths[1]).expect("PDF is readable");
        assert!(pdf.starts_with(b"%PDF-"));
    }

    #[test]
    fn drawn_arrow_survives_project_reopen_and_exports_every_segment() {
        use fanta_tools::{
            Button, LineTool, ModifierKeys, PointerEvent, Tool, ToolContext, ToolEvent,
        };
        use glam::DVec2;

        let directory = tempfile::tempdir().expect("arrow project");
        let mut document = Doc::new();
        let page = insert(
            &mut document,
            CanvasNode::new(NodeData::Group(GroupNode::default())),
        );
        document.add_page(page);
        document.set_active_page(Some(page));
        let mut viewport = Viewport::default();
        let mut context = ToolContext::new(
            &mut document,
            &mut viewport,
            fanta_canvas::SnapEngine {
                targets: fanta_canvas::SnapTargets::empty(),
                ..Default::default()
            },
            DVec2::new(800.0, 600.0),
        );
        let mut arrow = LineTool::arrow();
        arrow.handle_event(
            &mut context,
            ToolEvent::Pointer(PointerEvent::Press {
                screen: [550.0, 420.0],
                button: Button::Primary,
                modifiers: ModifierKeys::empty(),
                count: 1,
            }),
        );
        arrow.handle_event(
            &mut context,
            ToolEvent::Pointer(PointerEvent::Release {
                screen: [250.0, 180.0],
                button: Button::Primary,
                modifiers: ModifierKeys::empty(),
            }),
        );
        let arrow_id = *document.selection.iter().next().expect("selected arrow");
        let original = document.scene.get(arrow_id).expect("drawn arrow").clone();
        fanta_format::write_project_tree(directory.path(), &document, &BTreeMap::new())
            .expect("save arrow project");
        let (mut reopened, _) =
            fanta_format::read_project_tree(directory.path()).expect("read saved arrow project");
        let restored = reopened.scene.get(arrow_id).expect("reopened arrow");
        assert_eq!(restored.data, original.data);
        assert_eq!(restored.transform, original.transform);
        assert_eq!(restored.name, "Arrow");
        reopened.selection.select_only(arrow_id);

        let batch = prepare_export_jobs(
            &reopened,
            None,
            None,
            directory.path().to_path_buf(),
            &[
                ExportPreset {
                    format: ExportFormat::Png,
                    scale: ExportScale::Two,
                },
                ExportPreset {
                    format: ExportFormat::Svg,
                    scale: ExportScale::One,
                },
            ],
        )
        .expect("arrow has exportable bounds");
        let target = batch.targets.first().expect("arrow export target");
        let export_viewport = target_viewport(target, 2.0);
        let paths = run_export_jobs(batch).expect("export arrow");
        let raster = image::open(paths.first().expect("PNG output"))
            .expect("decode exported arrow")
            .to_rgba8();
        let NodeData::Vector(vector) = &original.data else {
            panic!("arrow is a vector");
        };
        let mut current = DVec2::ZERO;
        let mut lines = 0;
        for segment in &vector.path.segments {
            match segment {
                fanta_doc::PathSegment::Move { to } => current = DVec2::from(*to),
                fanta_doc::PathSegment::Line { to } => {
                    let end = DVec2::from(*to);
                    let midpoint = original.transform.transform_point((current + end) / 2.0);
                    let pixel = fanta_canvas::world_to_screen(
                        midpoint,
                        &export_viewport,
                        DVec2::new(raster.width() as f64, raster.height() as f64),
                    );
                    let painted = (-2..=2).any(|offset_x| {
                        (-2..=2).any(|offset_y| {
                            let x = pixel.x.round() as i64 + offset_x;
                            let y = pixel.y.round() as i64 + offset_y;
                            x >= 0
                                && y >= 0
                                && raster.get_pixel_checked(x as u32, y as u32).is_some_and(
                                    |pixel| {
                                        pixel[3] > 64
                                            && pixel[0] < 50
                                            && pixel[1] < 50
                                            && pixel[2] < 50
                                    },
                                )
                        })
                    });
                    assert!(painted, "arrow segment {lines} is absent from PNG export");
                    lines += 1;
                    current = end;
                }
                _ => panic!("arrow consists of open straight segments"),
            }
        }
        assert_eq!(lines, 3);
        let svg =
            std::fs::read_to_string(paths.get(1).expect("SVG output")).expect("read vector arrow");
        assert!(svg.contains("<path"));
        assert!(!svg.contains("<image"));
    }

    #[test]
    fn persistent_measurements_leave_png_pixels_svg_paint_and_export_bounds_unchanged() {
        use crate::measurements::{Measurement, create_measurement_op, read_measurements};

        let directory = tempfile::tempdir().expect("measurement export fixture");
        let mut document = Doc::new();
        let mut page_node = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page_node.name = "Measured page".into();
        page_node.transform = Transform2D::translation(120.0, -70.0);
        let page = insert(&mut document, page_node);
        document.add_page(page);
        document.set_active_page(Some(page));
        let mut art = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            48.0,
            24.0,
            Color::rgb(24, 96, 200),
        )));
        art.parent = Some(page);
        art.transform = Transform2D::translation(8.0, 12.0);
        if let NodeData::Vector(vector) = &mut art.data {
            let mut stroke = Stroke::solid(Color::BLACK, 4.0);
            stroke.align = StrokeAlign::Outside;
            vector.strokes.push(stroke);
        }
        insert(&mut document, art);
        let art_count = document.scene.len();
        let export = |document: &Doc, name: &str| {
            let page = FigPage {
                root: Some(page),
                name: "Measured page".into(),
                bounds: Bounds::ZERO,
                hidden: false,
            };
            let batch = prepare_export_jobs(
                document,
                None,
                Some(&page),
                directory.path().join(name),
                &[
                    ExportPreset {
                        format: ExportFormat::Png,
                        scale: ExportScale::Two,
                    },
                    ExportPreset {
                        format: ExportFormat::Svg,
                        scale: ExportScale::One,
                    },
                ],
            )
            .expect("prepare actual page export");
            let bounds = batch.targets.first().expect("page export target").bounds;
            let files = run_export_jobs(batch).expect("write PNG and SVG");
            let raster = image::open(files.first().expect("PNG file"))
                .expect("decode exported PNG")
                .to_rgba8();
            let svg = std::fs::read_to_string(files.get(1).expect("SVG file"))
                .expect("read exported SVG");
            (bounds, raster, svg)
        };
        let before = export(&document, "before");
        assert_eq!(
            before.1.dimensions(),
            (112, 64),
            "baseline includes the outside stroke"
        );
        assert!(
            before
                .1
                .pixels()
                .any(|pixel| pixel[3] == 255 && pixel[2] > pixel[0]),
            "fixture paints blue artwork"
        );
        assert!(
            before.2.contains("<path") || before.2.contains("<rect"),
            "SVG fixture includes vector paint"
        );
        assert!(!before.2.contains("<image"));

        let inside = Measurement::new([12.0, 16.0], [48.0, 28.0], "Designer".into(), 42)
            .expect("overlapping measurement");
        let outside = Measurement::new(
            [-1_000_000.0, 1_000_000.0],
            [1_000_000.0, -1_000_000.0],
            "Designer".into(),
            43,
        )
        .expect("measurement far outside art bounds");
        for measurement in [&inside, &outside] {
            document
                .apply(
                    create_measurement_op(&document, page, measurement)
                        .expect("create measurement"),
                )
                .expect("apply metadata");
        }
        assert_eq!(
            document.scene.len(),
            art_count,
            "measurement IDs do not become scene nodes"
        );
        assert_eq!(
            read_measurements(&document, page)
                .expect("read marks")
                .len(),
            2
        );
        let after = export(&document, "after");
        assert_eq!(
            after.0, before.0,
            "far-away measurement cannot inflate export bounds"
        );
        assert_eq!(after.1.dimensions(), before.1.dimensions());
        assert_eq!(
            after.1.as_raw(),
            before.1.as_raw(),
            "marks never enter art PNG pixels"
        );
        assert_eq!(after.2, before.2, "SVG paint stays exactly the same");
        assert!(!after.2.contains(&inside.id));
        assert!(!after.2.contains(&outside.id));

        let project = directory.path().join("saved-project");
        fanta_format::write_project_tree(&project, &document, &BTreeMap::new())
            .expect("save measured project");
        let (reopened, _) =
            fanta_format::read_project_tree(&project).expect("reopen measured page FNX");
        assert_eq!(
            read_measurements(&reopened, page)
                .expect("restored records")
                .len(),
            2
        );
        let restored = export(&reopened, "after-reopen");
        assert_eq!(restored.0, before.0);
        assert_eq!(
            restored.1.as_raw(),
            before.1.as_raw(),
            "reopened measurements remain excluded from PNG"
        );
        assert_eq!(
            restored.2, before.2,
            "reopened measurements remain excluded from SVG"
        );
    }

    #[test]
    fn persistent_annotations_preserve_export_pixels_bounds_and_metadata_after_reopen() {
        use crate::annotations::{DeveloperAnnotation, create_annotation_op, read_annotations};

        let directory = tempfile::tempdir().expect("annotation export fixture");
        let mut document = Doc::new();
        let mut page_node = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page_node.name = "Annotated page".into();
        page_node.transform = Transform2D::translation(120.0, -70.0);
        page_node.meta = serde_json::json!({
            "annotations": [{"version": 2, "id": "future-note", "opaque": [1, "keep"]}],
            "integration": {"keep": true}
        });
        let page = insert(&mut document, page_node);
        document.add_page(page);
        document.set_active_page(Some(page));
        let mut art = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            48.0,
            24.0,
            Color::rgb(24, 96, 200),
        )));
        art.parent = Some(page);
        art.transform = Transform2D::translation(8.0, 12.0);
        if let NodeData::Vector(vector) = &mut art.data {
            let mut stroke = Stroke::solid(Color::BLACK, 4.0);
            stroke.align = StrokeAlign::Outside;
            vector.strokes.push(stroke);
        }
        let art = insert(&mut document, art);
        let art_snapshot = document.scene.get(art).cloned().expect("painted artwork");
        let scene_count = document.scene.len();
        let export = |document: &Doc, name: &str| {
            let page = FigPage {
                root: Some(page),
                name: "Annotated page".into(),
                bounds: Bounds::ZERO,
                hidden: false,
            };
            let batch = prepare_export_jobs(
                document,
                None,
                Some(&page),
                directory.path().join(name),
                &[
                    ExportPreset {
                        format: ExportFormat::Png,
                        scale: ExportScale::Two,
                    },
                    ExportPreset {
                        format: ExportFormat::Svg,
                        scale: ExportScale::One,
                    },
                ],
            )
            .expect("prepare actual page export");
            let bounds = batch.targets.first().expect("page export target").bounds;
            let files = run_export_jobs(batch).expect("write annotated PNG and SVG");
            let raster = image::open(files.first().expect("PNG file"))
                .expect("decode exported PNG")
                .to_rgba8();
            let svg = std::fs::read_to_string(files.get(1).expect("SVG file"))
                .expect("read exported SVG");
            (bounds, raster, svg)
        };
        let before = export(&document, "before");
        assert_eq!(before.1.dimensions(), (112, 64));
        assert!(
            before
                .1
                .pixels()
                .any(|pixel| pixel[3] == 255 && pixel[2] > pixel[0]),
            "baseline export must contain opaque blue artwork"
        );
        assert!(before.2.contains("<path") || before.2.contains("<rect"));
        assert!(!before.2.contains("<image"));

        let inside = DeveloperAnnotation::new(
            [12.0, 16.0],
            "Use 24 px spacing.\nKeep the résumé label.".into(),
            "Designer".into(),
            42,
        )
        .expect("note over artwork");
        let outside = DeveloperAnnotation::new(
            [-1_000_000.0, 1_000_000.0],
            "Far outside the exported page".into(),
            "Reviewer".into(),
            43,
        )
        .expect("note far outside art bounds");
        for annotation in [&inside, &outside] {
            let operation = create_annotation_op(&document, page, annotation)
                .expect("create annotation metadata");
            document.apply(operation).expect("apply annotation");
        }
        let metadata = document.scene.get(page).expect("page").meta.clone();
        assert_eq!(
            metadata["annotations"]
                .as_array()
                .expect("all raw notes")
                .len(),
            3
        );
        assert_eq!(
            metadata["annotations"][0]["opaque"],
            serde_json::json!([1, "keep"])
        );
        assert_eq!(metadata["integration"], serde_json::json!({"keep": true}));
        assert_eq!(document.scene.len(), scene_count);
        assert_eq!(document.scene.get(art), Some(&art_snapshot));
        let after = export(&document, "after");
        assert_eq!(after.0, before.0, "off-art notes must not inflate bounds");
        assert_eq!(after.1.dimensions(), before.1.dimensions());
        assert_eq!(
            after.1.as_raw(),
            before.1.as_raw(),
            "notes cannot paint PNG pixels"
        );
        assert_eq!(after.2, before.2, "notes cannot enter vector SVG paint");
        for annotation in [&inside, &outside] {
            assert!(!after.2.contains(&annotation.id));
            assert!(!after.2.contains(&annotation.text));
        }

        let project = directory.path().join("saved-project");
        fanta_format::write_project_tree(&project, &document, &BTreeMap::new())
            .expect("save annotated project");
        let (reopened, _) = fanta_format::read_project_tree(&project).expect("reopen page FNX");
        assert_eq!(
            reopened.scene.get(page).expect("reopened page").meta,
            metadata
        );
        assert_eq!(reopened.scene.get(art), Some(&art_snapshot));
        assert_eq!(reopened.scene.len(), scene_count);
        let restored = read_annotations(&reopened, page).expect("restored annotations");
        assert_eq!(restored.len(), 2, "future-version metadata stays opaque");
        assert_eq!(restored.first().expect("inside note").annotation(), &inside);
        assert_eq!(
            restored.get(1).expect("outside note").annotation(),
            &outside
        );
        let restored = export(&reopened, "after-reopen");
        assert_eq!(restored.0, before.0);
        assert_eq!(restored.1.dimensions(), before.1.dimensions());
        assert_eq!(restored.1.as_raw(), before.1.as_raw());
        assert_eq!(restored.2, before.2);
    }

    #[test]
    fn empty_bounds_and_empty_presets_are_reported_before_background_export() {
        let mut doc = Doc::new();
        let empty = insert(
            &mut doc,
            CanvasNode::new(NodeData::Group(GroupNode::default())),
        );
        doc.selection.select_only(empty);
        let no_presets = match prepare_export_jobs(&doc, None, None, PathBuf::from("project"), &[])
        {
            Ok(_) => panic!("an empty preset list cannot export"),
            Err(error) => error,
        };
        assert!(
            no_presets
                .to_string()
                .contains("at least one export setting")
        );

        let result = prepare_export_jobs(
            &doc,
            None,
            None,
            PathBuf::from("project"),
            &[ExportPreset::default()],
        );
        let error = match result {
            Ok(_) => panic!("empty group cannot be exported"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("no visible bounds"));
    }

    #[test]
    fn oversized_raster_requests_are_rejected_instead_of_silently_clamped() {
        let side_error = raster_dimensions(
            Bounds::from_xywh(0.0, 0.0, 10_000.0, 5_000.0),
            ExportScale::Four,
            "Hero",
        )
        .expect_err("oversized sides are rejected");
        assert!(side_error.to_string().contains("40000×20000"));
        assert!(side_error.to_string().contains("smaller scale"));

        let budget_error = raster_dimensions(
            Bounds::from_xywh(0.0, 0.0, 5_000.0, 4_000.0),
            ExportScale::One,
            "Photo",
        )
        .expect_err("oversized total pixel count is rejected");
        assert!(budget_error.to_string().contains("safe limit"));

        assert_eq!(
            raster_dimensions(
                Bounds::from_xywh(0.0, 0.0, 0.1, 0.1),
                ExportScale::One,
                "Dot"
            )
            .expect("tiny export remains valid"),
            (1, 1)
        );
    }

    #[test]
    fn nested_selected_target_is_detached_with_its_full_world_transform() {
        let mut doc = Doc::new();
        let mut parent = CanvasNode::new(NodeData::Group(GroupNode::default()));
        parent.transform = Transform2D::translation(100.0, 50.0).then(&Transform2D::rotation(0.35));
        let parent = insert(&mut doc, parent);
        let mut child = exportable_frame("Nested", 7.0);
        child.parent = Some(parent);
        child.transform = Transform2D::translation(7.0, 9.0);
        let child = insert(&mut doc, child);
        doc.selection.select_only(child);
        let original_world = doc.scene.world_transform(child).expect("world transform");
        let batch = prepare_export_jobs(
            &doc,
            None,
            None,
            PathBuf::from("project"),
            &[ExportPreset {
                format: ExportFormat::Svg,
                scale: ExportScale::One,
            }],
        )
        .expect("nested selection is exportable");

        let detached = render_document_for_target(&batch.doc, &batch.targets[0])
            .expect("target can be detached");
        let detached_node = detached.scene.get(child).expect("detached node exists");
        assert_eq!(detached_node.parent, None);
        assert_eq!(
            detached.scene.world_transform(child),
            Some(original_world),
            "exporting a nested node must preserve every ancestor transform"
        );
    }

    #[test]
    fn prepared_targets_include_outer_stroke_shadow_and_layer_blur_bounds() {
        let directory = tempfile::tempdir().expect("temporary export project");
        let mut doc = Doc::new();
        let mut node = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            10.0,
            10.0,
            Color::WHITE,
        )));
        let NodeData::Vector(vector) = &mut node.data else {
            unreachable!();
        };
        let mut stroke = Stroke::solid(Color::BLACK, 4.0);
        stroke.align = StrokeAlign::Outside;
        stroke.join = StrokeJoin::Round;
        vector.strokes.push(stroke);
        node.effects.push(Shadow {
            kind: ShadowKind::Drop,
            color: Color::BLACK,
            blur: 4.0,
            spread: 0.0,
            offset: [5.0, 0.0],
            show_behind_node: false,
        });
        node.blurs.push(Blur::layer(4.0));
        let node = insert(&mut doc, node);
        doc.selection.select_only(node);

        let batch = prepare_export_jobs(
            &doc,
            None,
            None,
            directory.path().to_path_buf(),
            &[ExportPreset {
                format: ExportFormat::Png,
                scale: ExportScale::One,
            }],
        )
        .expect("painted extents are exportable");
        assert_eq!(
            batch.targets[0].bounds,
            Bounds::from_xywh(-10.0, -10.0, 35.0, 30.0)
        );
        let paths = run_export_jobs(batch).expect("paint-bounds export succeeds");
        assert_eq!(
            image::open(&paths[0])
                .expect("paint-bounds PNG is decodable")
                .dimensions(),
            (35, 30),
            "the export surface must include every painted extent"
        );
    }

    #[test]
    fn existing_exports_are_never_overwritten_and_temporary_files_are_cleaned() {
        let directory = tempfile::tempdir().expect("temporary export project");
        let exports = directory.path().join("exports");
        std::fs::create_dir(&exports).expect("exports directory");
        std::fs::write(exports.join("Frame.png"), b"keep me").expect("existing export");
        let mut doc = Doc::new();
        let frame = insert(&mut doc, exportable_frame("Frame", 0.0));
        doc.selection.select_only(frame);
        let batch = prepare_export_jobs(
            &doc,
            None,
            None,
            directory.path().to_path_buf(),
            &[ExportPreset {
                format: ExportFormat::Png,
                scale: ExportScale::One,
            }],
        )
        .expect("frame is exportable");

        let paths = run_export_jobs(batch).expect("collision-safe export succeeds");
        assert_eq!(paths, [exports.join("Frame-2.png")]);
        assert_eq!(
            std::fs::read(exports.join("Frame.png")).expect("original remains"),
            b"keep me"
        );
        let mut names = std::fs::read_dir(&exports)
            .expect("read exports")
            .map(|entry| {
                entry
                    .expect("directory entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(names, ["Frame-2.png", "Frame.png"]);
    }
}

#[cfg(feature = "fanta-gpui-ui")]
pub(crate) fn render_layer(
    doc: &Doc,
    resolver: Option<Arc<dyn AssetResolver>>,
    id: NodeId,
    format: ExportFormat,
) -> Result<Vec<u8>> {
    let mut selection = doc.clone();
    selection.selection.replace_with([id]);
    let preset = ExportPreset {
        format,
        scale: ExportScale::One,
    };
    let batch = prepare_export_jobs(&selection, resolver, None, PathBuf::new(), &[preset])?;
    let target = batch
        .targets
        .first()
        .context("The layer has no export target")?;
    let resolved = resolve_export_bindings(&batch.doc);
    let request = batch
        .requests
        .first()
        .context("The layer has no export request")?;
    render_export(&batch, resolved.as_ref(), target, request)
}

#[cfg(feature = "fanta-gpui-ui")]
pub(crate) fn render_thumbnail(
    doc: &Doc,
    resolver: Option<Arc<dyn AssetResolver>>,
    id: NodeId,
) -> Result<Vec<u8>> {
    let document = resolve_export_bindings(doc);
    let bounds = visual_world_bounds(&document.scene, id, 0.)
        .context("The thumbnail layer has no bounds")?;
    ensure_exportable_bounds("Project thumbnail", bounds)?;
    let zoom = (512. / bounds.width()).min(512. / bounds.height()).min(1.);
    let width = (bounds.width() * zoom).ceil().max(1.) as u32;
    let height = (bounds.height() * zoom).ceil().max(1.) as u32;
    let target = ExportTarget {
        root: Some(id),
        name: "Project thumbnail".into(),
        bounds,
    };
    let mut renderer = RasterRenderer::new(width, height)
        .map_err(|error| anyhow::anyhow!("Creating thumbnail surface: {error}"))?;
    if let Some(resolver) = resolver {
        renderer.set_asset_resolver(resolver);
    }
    renderer.render_page_with(
        &document.scene,
        &target_viewport(&target, zoom),
        Some(id),
        &render_inputs(&document),
    );
    renderer
        .encode_png()
        .map_err(|error| anyhow::anyhow!("Encoding thumbnail: {error}"))
}
