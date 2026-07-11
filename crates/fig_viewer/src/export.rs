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
const JPEG_QUALITY: u8 = 90;

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
    presets: Vec<ExportPreset>,
    project_root: PathBuf,
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
    if presets.is_empty() {
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

    validate_export_requests(&targets, presets)?;

    Ok(ExportBatch {
        doc: doc.clone(),
        asset_resolver,
        targets,
        presets: presets.to_vec(),
        project_root,
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
    pub(crate) fn len(&self) -> usize {
        self.targets.len().saturating_mul(self.presets.len())
    }

    pub(crate) fn format_summary(&self) -> String {
        let mut labels = Vec::new();
        for preset in &self.presets {
            let label = if preset.format.is_raster() {
                format!("{} {}", preset.format.label(), preset.scale.label())
            } else {
                preset.format.label().to_string()
            };
            if !labels.contains(&label) {
                labels.push(label);
            }
        }
        labels.join(", ")
    }
}

pub(crate) fn run_export_jobs(batch: ExportBatch) -> Result<Vec<PathBuf>> {
    if batch.targets.is_empty() || batch.presets.is_empty() {
        bail!("there is nothing to export");
    }

    let exports_dir = batch.project_root.join("exports");
    std::fs::create_dir_all(&exports_dir)
        .with_context(|| format!("creating {}", exports_dir.display()))?;
    let file_names = output_file_names(&batch.targets, &batch.presets);
    let mut file_names = file_names.into_iter();
    let mut paths = Vec::with_capacity(batch.len());

    for target in &batch.targets {
        let document = render_document_for_target(&batch.doc, target)?;
        for preset in &batch.presets {
            let file_name = file_names
                .next()
                .context("export filename generation was incomplete")?;
            let bytes = render_export(&batch, &document, target, *preset).with_context(|| {
                format!("exporting {} as {}", target.name, preset.format.label())
            })?;
            let path = write_export_atomically(&exports_dir, &file_name, &bytes)
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
    preset: ExportPreset,
) -> Result<Vec<u8>> {
    match preset.format {
        ExportFormat::Png => render_png(batch, document, target, preset.scale),
        ExportFormat::Jpeg => render_jpeg(batch, document, target, preset.scale),
        ExportFormat::Svg => render_svg(batch, document, target),
        ExportFormat::Pdf => render_pdf(batch, document, target),
    }
}

fn render_png(
    batch: &ExportBatch,
    document: &Doc,
    target: &ExportTarget,
    scale: ExportScale,
) -> Result<Vec<u8>> {
    let mut renderer = render_raster(batch, document, target, scale)?;
    renderer
        .encode_png()
        .map_err(|error| anyhow::anyhow!("encoding export PNG: {error}"))
}

fn render_jpeg(
    batch: &ExportBatch,
    document: &Doc,
    target: &ExportTarget,
    scale: ExportScale,
) -> Result<Vec<u8>> {
    let mut renderer = render_raster(batch, document, target, scale)?;
    let (width, height) = raster_dimensions(target.bounds, scale, &target.name)?;
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
    scale: ExportScale,
) -> Result<RasterRenderer> {
    let (width, height) = raster_dimensions(target.bounds, scale, &target.name)?;
    let zoom = scale.multiplier();
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

fn validate_export_requests(targets: &[ExportTarget], presets: &[ExportPreset]) -> Result<()> {
    for target in targets {
        for preset in presets {
            if preset.format.is_raster() {
                raster_dimensions(target.bounds, preset.scale, &target.name)?;
            } else {
                checked_vector_dimension(target.bounds.width(), "vector width")?;
                checked_vector_dimension(target.bounds.height(), "vector height")?;
            }
        }
    }
    Ok(())
}

fn raster_dimensions(bounds: Bounds, scale: ExportScale, name: &str) -> Result<(u32, u32)> {
    let multiplier = scale.multiplier();
    let width = (bounds.width() * multiplier).ceil().max(1.0);
    let height = (bounds.height() * multiplier).ceil().max(1.0);
    if width > f64::from(MAX_EXPORT_PIXELS) || height > f64::from(MAX_EXPORT_PIXELS) {
        bail!(
            "{name} at {} would be {width:.0}×{height:.0} pixels; the per-side limit is {MAX_EXPORT_PIXELS}. Choose a smaller scale",
            scale.label()
        );
    }
    let pixels = (width as u64)
        .checked_mul(height as u64)
        .context("export dimensions overflowed the pixel budget")?;
    if pixels > MAX_EXPORT_TOTAL_PIXELS {
        bail!(
            "{name} at {} would contain {pixels} pixels; the safe limit is {MAX_EXPORT_TOTAL_PIXELS}. Choose a smaller scale",
            scale.label()
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

fn output_file_names(targets: &[ExportTarget], presets: &[ExportPreset]) -> Vec<String> {
    targets
        .iter()
        .flat_map(|target| {
            presets.iter().map(move |preset| {
                let suffix = if preset.format.is_raster() {
                    preset.scale.file_suffix()
                } else {
                    ""
                };
                format!("{}{suffix}.{}", target.name, preset.format.extension())
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
