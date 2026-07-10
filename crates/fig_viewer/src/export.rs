use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use fanta_doc::{Bounds, Doc, NodeId, Viewport};
use fanta_render::{AssetResolver, RasterRenderer, RenderInputs};

use crate::document::{FigPage, page_bounds};

const MAX_EXPORT_PIXELS: u32 = 8192;
const EXPORT_SCALE: f64 = 2.0;

pub(crate) struct PngExportBatch {
    doc: Doc,
    asset_resolver: Option<Arc<dyn AssetResolver>>,
    targets: Vec<PngExportTarget>,
    project_root: PathBuf,
}

struct PngExportTarget {
    /// Subtree to render: a selected node, the page root, or `None` for every
    /// root in a document with one implicit page.
    root: Option<NodeId>,
    name: String,
    bounds: Bounds,
}

pub(crate) fn prepare_png_export_jobs(
    doc: &Doc,
    asset_resolver: Option<Arc<dyn AssetResolver>>,
    page: Option<&FigPage>,
    project_root: PathBuf,
) -> Result<PngExportBatch> {
    let selected: Vec<_> = doc.selection.iter().copied().collect();
    if selected.is_empty() {
        let root = page.and_then(|page| page.root);
        let name = page
            .map(|page| page.name.to_string())
            .unwrap_or_else(|| "Page".to_string());
        let bounds = page_bounds(&doc, root);
        ensure_exportable_bounds(&name, bounds)?;
        return Ok(PngExportBatch {
            doc: doc.clone(),
            asset_resolver,
            targets: vec![PngExportTarget {
                root,
                name: sanitize_file_name(&name),
                bounds,
            }],
            project_root,
        });
    }

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
        let bounds = doc
            .scene
            .world_bounds(id)
            .with_context(|| format!("{name} has no visible bounds"))?;
        ensure_exportable_bounds(&name, bounds)?;
        targets.push((id, name, bounds));
    }

    let unique_names = unique_export_names(targets.iter().map(|(_, name, _)| name.as_str()));
    let targets = targets
        .into_iter()
        .zip(unique_names)
        .map(|((root, _, bounds), name)| PngExportTarget {
            root: Some(root),
            name,
            bounds,
        })
        .collect();
    Ok(PngExportBatch {
        doc: doc.clone(),
        asset_resolver,
        targets,
        project_root,
    })
}

impl PngExportBatch {
    pub(crate) fn len(&self) -> usize {
        self.targets.len()
    }
}

pub(crate) fn run_png_export_jobs(batch: PngExportBatch) -> Result<Vec<PathBuf>> {
    if batch.targets.is_empty() {
        bail!("there is nothing to export");
    }
    batch
        .targets
        .iter()
        .map(|target| {
            run_png_export(&batch, target).with_context(|| format!("exporting {}", target.name))
        })
        .collect()
}

fn run_png_export(batch: &PngExportBatch, target: &PngExportTarget) -> Result<PathBuf> {
    let width = ((target.bounds.width() * EXPORT_SCALE).ceil() as u32).clamp(1, MAX_EXPORT_PIXELS);
    let height =
        ((target.bounds.height() * EXPORT_SCALE).ceil() as u32).clamp(1, MAX_EXPORT_PIXELS);
    let zoom = (f64::from(width) / target.bounds.width())
        .min(f64::from(height) / target.bounds.height())
        .min(EXPORT_SCALE);

    let mut renderer = RasterRenderer::new(width, height)
        .map_err(|error| anyhow::anyhow!("creating {width}x{height} export surface: {error}"))?;
    if let Some(asset_resolver) = batch.asset_resolver.clone() {
        renderer.set_asset_resolver(asset_resolver);
    }
    let center = target.bounds.center();
    let viewport = Viewport {
        center: [center.x, center.y],
        zoom,
    };
    let inputs = RenderInputs {
        components: &batch.doc.components,
        variables: &batch.doc.variables,
        active_modes: &batch.doc.active_modes,
        mode_generation: 0,
        motion: None,
        playback: None,
        dark_ui: false,
    };
    renderer.render_page_with(&batch.doc.scene, &viewport, target.root, &inputs);
    let png = renderer
        .encode_png()
        .map_err(|error| anyhow::anyhow!("encoding export PNG: {error}"))?;

    let exports_dir = batch.project_root.join("exports");
    std::fs::create_dir_all(&exports_dir)
        .with_context(|| format!("creating {}", exports_dir.display()))?;
    let path = exports_dir.join(format!("{}.png", target.name));
    std::fs::write(&path, png).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

fn ensure_exportable_bounds(name: &str, bounds: Bounds) -> Result<()> {
    if !bounds.is_finite() || bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        bail!("{name} has invalid or empty bounds");
    }
    Ok(())
}

fn unique_export_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut next_suffix_by_base = HashMap::<String, usize>::new();
    let mut used = HashSet::new();
    names
        .into_iter()
        .map(|name| {
            let base = sanitize_file_name(name);
            let next_suffix = next_suffix_by_base.entry(base.clone()).or_insert(1);
            loop {
                let candidate = if *next_suffix == 1 {
                    base.clone()
                } else {
                    format!("{base}-{}", *next_suffix)
                };
                *next_suffix += 1;
                if used.insert(candidate.clone()) {
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
    use fanta_doc::{CanvasNode, Color, Fill, GroupNode, NodeData, Transform2D};
    use image::GenericImageView as _;

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
    fn multiple_selected_layers_get_distinct_jobs_sharing_one_document_snapshot() {
        let mut doc = Doc::new();
        let first = insert(&mut doc, exportable_frame("Card", 0.0));
        let second = insert(&mut doc, exportable_frame("Card", 30.0));
        doc.selection.select_only(first);
        doc.selection.toggle(second);

        let batch = prepare_png_export_jobs(&doc, None, None, PathBuf::from("project"))
            .expect("selection is exportable");
        assert_eq!(batch.len(), 2);
        assert_eq!(batch.targets[0].root, Some(first));
        assert_eq!(batch.targets[1].root, Some(second));
        assert_eq!(batch.targets[0].name, "Card");
        assert_eq!(batch.targets[1].name, "Card-2");
    }

    #[test]
    fn png_export_writes_the_selected_layer_at_two_x() {
        let directory = tempfile::tempdir().expect("temporary export project");
        let mut doc = Doc::new();
        let frame = insert(&mut doc, exportable_frame("Preview/Card", 15.0));
        doc.selection.select_only(frame);
        let batch = prepare_png_export_jobs(&doc, None, None, directory.path().to_path_buf())
            .expect("frame is exportable");

        let paths = run_png_export_jobs(batch).expect("PNG export succeeds");
        assert_eq!(paths.len(), 1);
        assert_eq!(
            paths[0].file_name().and_then(|name| name.to_str()),
            Some("Preview-Card.png")
        );
        let image = image::open(&paths[0]).expect("written PNG is decodable");
        assert_eq!(image.dimensions(), (40, 20));
    }

    #[test]
    fn empty_bounds_are_reported_before_background_export() {
        let mut doc = Doc::new();
        let empty = insert(
            &mut doc,
            CanvasNode::new(NodeData::Group(GroupNode::default())),
        );
        doc.selection.select_only(empty);
        let result = prepare_png_export_jobs(&doc, None, None, PathBuf::from("project"));
        let error = match result {
            Ok(_) => panic!("empty group cannot be exported"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("no visible bounds"));
    }
}
