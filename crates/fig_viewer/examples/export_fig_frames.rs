use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context as _, Result, anyhow};
use fanta_doc::{Bounds, Doc, NodeData, NodeId, Viewport};
use fanta_fig_interop::{fig_to_doc, read_fig};
use fanta_render::{
    AssetResolver, DecodedImage, InMemoryAssetResolver, RasterRenderer, RenderInputs,
    solve_scene_layout,
};

const DEFAULT_MAX_SIDE: u32 = 1600;
const DEFAULT_PADDING: f64 = 48.0;

struct Options {
    fig_path: PathBuf,
    output_dir: PathBuf,
    root_filter: Option<String>,
    only_filter: Option<String>,
    solve_layout: bool,
    max_side: u32,
    padding: f64,
}

fn main() -> Result<()> {
    let options = parse_options()?;
    fs::create_dir_all(&options.output_dir)
        .with_context(|| format!("creating {}", options.output_dir.display()))?;

    let bytes = fs::read(&options.fig_path)
        .with_context(|| format!("reading {}", options.fig_path.display()))?;
    let fig = read_fig(&bytes).context("parsing .fig")?;
    let (mut doc, report, assets) = fig_to_doc(&fig).context("mapping .fig to Fanta document")?;
    eprintln!(
        "mapped {} nodes, {} malformed, {} skipped type buckets",
        report.mapped,
        report.malformed,
        report.skipped_by_type.len()
    );

    let root = select_export_root(&doc, options.root_filter.as_deref())?;
    if options.solve_layout {
        let page = page_for_node(&doc, root).unwrap_or(root);
        solve_scene_layout(&mut doc.scene, page);
    }

    let resolver = decode_assets(assets);
    let asset_resolver =
        (!resolver.is_empty()).then(|| Arc::new(resolver) as Arc<dyn AssetResolver>);
    let children = export_children(&doc, root, options.only_filter.as_deref());
    if children.is_empty() {
        return Err(anyhow!("no child frames matched the requested filters"));
    }

    for (index, child) in children.into_iter().enumerate() {
        export_node(
            &doc,
            child,
            &options.output_dir,
            index,
            options.max_side,
            options.padding,
            asset_resolver.clone(),
        )
        .with_context(|| format!("exporting {}", node_path(&doc, child)))?;
    }

    Ok(())
}

fn parse_options() -> Result<Options> {
    let mut args = env::args().skip(1);
    let fig_path = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage_error("missing .fig path"))?;
    let output_dir = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage_error("missing output directory"))?;

    let mut root_filter = None;
    let mut only_filter = None;
    let mut solve_layout = false;
    let mut max_side = DEFAULT_MAX_SIDE;
    let mut padding = DEFAULT_PADDING;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => {
                root_filter = Some(
                    args.next()
                        .ok_or_else(|| usage_error("--root requires a value"))?,
                );
            }
            "--only" => {
                only_filter = Some(
                    args.next()
                        .ok_or_else(|| usage_error("--only requires a value"))?,
                );
            }
            "--solve-layout" => {
                solve_layout = true;
            }
            "--max-side" => {
                let value = args
                    .next()
                    .ok_or_else(|| usage_error("--max-side requires a value"))?;
                max_side = value
                    .parse()
                    .with_context(|| format!("parsing --max-side {value:?}"))?;
            }
            "--padding" => {
                let value = args
                    .next()
                    .ok_or_else(|| usage_error("--padding requires a value"))?;
                padding = value
                    .parse()
                    .with_context(|| format!("parsing --padding {value:?}"))?;
            }
            "--help" | "-h" => return Err(anyhow!("{}", usage())),
            other => return Err(usage_error(&format!("unknown argument {other:?}"))),
        }
    }

    Ok(Options {
        fig_path,
        output_dir,
        root_filter,
        only_filter,
        solve_layout,
        max_side,
        padding,
    })
}

fn usage_error(message: &str) -> anyhow::Error {
    anyhow!("{message}\n\n{}", usage())
}

fn usage() -> &'static str {
    "usage: cargo run -p fig_viewer --example export_fig_frames -- <file.fig> <out-dir> [--root <path-fragment>] [--only <path-fragment>] [--solve-layout] [--max-side <px>] [--padding <px>]"
}

fn select_export_root(doc: &Doc, root_filter: Option<&str>) -> Result<NodeId> {
    if let Some(filter) = root_filter {
        return find_node_by_path(doc, filter)
            .with_context(|| format!("finding export root containing {filter:?}"));
    }

    doc.active_page()
        .or_else(|| doc.pages().first().copied())
        .or_else(|| doc.scene.roots().first().copied())
        .context("document has no root nodes")
}

fn find_node_by_path(doc: &Doc, filter: &str) -> Option<NodeId> {
    let filter = filter.to_lowercase();
    doc.scene
        .roots()
        .iter()
        .flat_map(|root| doc.scene.descendants_of(*root))
        .find(|node_id| node_path(doc, *node_id).to_lowercase().contains(&filter))
}

fn page_for_node(doc: &Doc, node: NodeId) -> Option<NodeId> {
    if doc.pages().contains(&node) {
        return Some(node);
    }

    doc.scene
        .ancestors_of(node)
        .find(|ancestor| doc.pages().contains(&ancestor.id))
        .map(|ancestor| ancestor.id)
}

fn export_children(doc: &Doc, root: NodeId, only_filter: Option<&str>) -> Vec<NodeId> {
    let only_filter = only_filter.map(str::to_lowercase);
    doc.scene
        .children_of(Some(root))
        .iter()
        .copied()
        .filter(|node_id| exportable_node(doc, *node_id))
        .filter(|node_id| {
            only_filter
                .as_ref()
                .is_none_or(|filter| node_path(doc, *node_id).to_lowercase().contains(filter))
        })
        .collect()
}

fn exportable_node(doc: &Doc, node: NodeId) -> bool {
    let Some(canvas_node) = doc.scene.get(node) else {
        return false;
    };
    if matches!(canvas_node.data, NodeData::Group(_) | NodeData::Instance(_)) {
        return doc.scene.world_bounds(node).is_some_and(|bounds| {
            bounds.is_finite() && bounds.width() > 0.0 && bounds.height() > 0.0
        });
    }
    false
}

fn export_node(
    doc: &Doc,
    node: NodeId,
    output_dir: &Path,
    index: usize,
    max_side: u32,
    padding: f64,
    asset_resolver: Option<Arc<dyn AssetResolver>>,
) -> Result<()> {
    let bounds = doc
        .scene
        .world_bounds(node)
        .with_context(|| format!("missing bounds for {}", node_path(doc, node)))?;
    let (width, height, viewport) = fit_render_size(bounds, max_side, padding);
    let mut renderer =
        RasterRenderer::new(width, height).context("creating Skia raster surface")?;
    if let Some(asset_resolver) = asset_resolver {
        renderer.set_asset_resolver(asset_resolver);
    }

    let inputs = RenderInputs {
        components: &doc.components,
        variables: &doc.variables,
        active_modes: &doc.active_modes,
        mode_generation: 0,
        motion: None,
        playback: None,
        dark_ui: false,
    };
    renderer.render_page_with(&doc.scene, &viewport, page_for_node(doc, node), &inputs);

    let path = output_dir.join(format!(
        "{:03}-{}.png",
        index + 1,
        sanitize_filename(&node_path(doc, node))
    ));
    fs::write(&path, renderer.encode_png().context("encoding PNG")?)
        .with_context(|| format!("writing {}", path.display()))?;
    eprintln!(
        "{}: {}x{} zoom {:.4} {}",
        path.display(),
        width,
        height,
        viewport.zoom,
        node_path(doc, node)
    );

    Ok(())
}

fn fit_render_size(bounds: Bounds, max_side: u32, padding: f64) -> (u32, u32, Viewport) {
    let content_width = bounds.width().max(1.0);
    let content_height = bounds.height().max(1.0);
    let padded_width = content_width + padding * 2.0;
    let padded_height = content_height + padding * 2.0;
    let max_side = max_side.max(1) as f64;
    let scale = (max_side / padded_width.max(padded_height)).min(1.0);
    let width = (padded_width * scale).ceil().max(1.0) as u32;
    let height = (padded_height * scale).ceil().max(1.0) as u32;
    let usable_width = (f64::from(width) - padding * 2.0 * scale).max(1.0);
    let usable_height = (f64::from(height) - padding * 2.0 * scale).max(1.0);
    let zoom = (usable_width / content_width)
        .min(usable_height / content_height)
        .max(f64::EPSILON);
    let center = bounds.center();
    (
        width,
        height,
        Viewport {
            center: [center.x, center.y],
            zoom,
        },
    )
}

fn node_path(doc: &Doc, node: NodeId) -> String {
    let mut parts = doc
        .scene
        .ancestors_of(node)
        .map(|ancestor| ancestor.name.clone())
        .collect::<Vec<_>>();
    parts.reverse();
    if let Some(node) = doc.scene.get(node) {
        parts.push(node.name.clone());
    }
    parts.join(" / ")
}

fn sanitize_filename(value: &str) -> String {
    let mut sanitized = String::with_capacity(value.len());
    let mut last_was_separator = false;
    for character in value.chars() {
        let next = if character.is_ascii_alphanumeric() {
            last_was_separator = false;
            Some(character)
        } else if last_was_separator {
            None
        } else {
            last_was_separator = true;
            Some('_')
        };
        if let Some(character) = next {
            sanitized.push(character);
        }
    }
    let sanitized = sanitized.trim_matches('_');
    if sanitized.is_empty() {
        "frame".to_string()
    } else {
        sanitized.to_string()
    }
}

fn decode_assets(assets: HashMap<fanta_doc::AssetId, Vec<u8>>) -> InMemoryAssetResolver {
    let mut resolver = InMemoryAssetResolver::new();
    for (asset_id, bytes) in assets {
        match image::load_from_memory(&bytes) {
            Ok(image) => {
                let rgba = image.to_rgba8();
                let (width, height) = rgba.dimensions();
                resolver.insert(
                    asset_id,
                    DecodedImage::new(Arc::new(rgba.into_raw()), width, height),
                );
            }
            Err(error) => {
                eprintln!("skipping undecodable image asset {asset_id}: {error:#}");
            }
        }
    }
    resolver
}
