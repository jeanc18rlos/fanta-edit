//! Renders one page of a .fig file to a PNG for fidelity inspection.
//!
//! usage: cargo run -p fig_viewer --example render_fig_page -- \
//!     <file.fig> <page-name-fragment> <out.png> \
//!     [--max-side <px>] [--region <x,y,w,h>] [--no-solve-layout]

use std::{collections::HashMap, env, fs, path::PathBuf, sync::Arc};

use anyhow::{Context as _, Result, anyhow};
use fanta_doc::{Bounds, Doc, NodeId, Viewport};
use fanta_fig_interop::{fig_to_doc, read_fig};
use fanta_render::{
    AssetResolver, DecodedImage, InMemoryAssetResolver, RasterRenderer, RenderInputs,
    solve_scene_layout,
};

const DEFAULT_MAX_SIDE: u32 = 1800;

struct Options {
    fig_path: PathBuf,
    page_filter: String,
    output_path: PathBuf,
    max_side: u32,
    region: Option<Bounds>,
    solve_passes: u32,
}

fn main() -> Result<()> {
    let options = parse_options()?;

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

    let page = find_page(&doc, &options.page_filter)?;
    for _ in 0..options.solve_passes {
        solve_scene_layout(&mut doc.scene, page);
    }

    let bounds = match options.region {
        Some(region) => region,
        None => page_content_bounds(&doc, page)
            .context("page has no children with finite world bounds")?,
    };
    eprintln!(
        "rendering page {:?} bounds ({:.1}, {:.1}) {:.1}x{:.1}",
        doc.scene.get(page).map(|node| node.name.as_str()),
        bounds.min_x,
        bounds.min_y,
        bounds.width(),
        bounds.height()
    );

    let resolver = decode_assets(assets);
    let asset_resolver =
        (!resolver.is_empty()).then(|| Arc::new(resolver) as Arc<dyn AssetResolver>);

    let (width, height, viewport) = fit_render_size(bounds, options.max_side);
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
    renderer.render_page_with(&doc.scene, &viewport, Some(page), &inputs);

    fs::write(
        &options.output_path,
        renderer.encode_png().context("encoding PNG")?,
    )
    .with_context(|| format!("writing {}", options.output_path.display()))?;
    eprintln!(
        "{}: {}x{} zoom {:.4}",
        options.output_path.display(),
        width,
        height,
        viewport.zoom
    );

    Ok(())
}

fn parse_options() -> Result<Options> {
    let mut args = env::args().skip(1);
    let fig_path = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage_error("missing .fig path"))?;
    let page_filter = args
        .next()
        .ok_or_else(|| usage_error("missing page name"))?;
    let output_path = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage_error("missing output png path"))?;

    let mut max_side = DEFAULT_MAX_SIDE;
    let mut region = None;
    let mut solve_passes = 1;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--max-side" => {
                let value = args
                    .next()
                    .ok_or_else(|| usage_error("--max-side requires a value"))?;
                max_side = value
                    .parse()
                    .with_context(|| format!("parsing --max-side {value:?}"))?;
            }
            "--region" => {
                let value = args
                    .next()
                    .ok_or_else(|| usage_error("--region requires x,y,w,h"))?;
                region = Some(parse_region(&value)?);
            }
            "--no-solve-layout" => solve_passes = 0,
            "--solve-passes" => {
                let value = args
                    .next()
                    .ok_or_else(|| usage_error("--solve-passes requires a value"))?;
                solve_passes = value
                    .parse()
                    .with_context(|| format!("parsing --solve-passes {value:?}"))?;
            }
            "--help" | "-h" => return Err(anyhow!("{}", usage())),
            other => return Err(usage_error(&format!("unknown argument {other:?}"))),
        }
    }

    Ok(Options {
        fig_path,
        page_filter,
        output_path,
        max_side,
        region,
        solve_passes,
    })
}

fn parse_region(value: &str) -> Result<Bounds> {
    let parts = value
        .split(',')
        .map(|part| part.trim().parse::<f64>())
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("parsing --region {value:?}"))?;
    let [x, y, w, h] = parts[..] else {
        return Err(usage_error("--region requires x,y,w,h"));
    };
    Ok(Bounds::from_xywh(x, y, w, h))
}

fn usage_error(message: &str) -> anyhow::Error {
    anyhow!("{message}\n\n{}", usage())
}

fn usage() -> &'static str {
    "usage: cargo run -p fig_viewer --example render_fig_page -- <file.fig> <page-name-fragment> <out.png> [--max-side <px>] [--region <x,y,w,h>] [--no-solve-layout] [--solve-passes <n>]"
}

fn find_page(doc: &Doc, filter: &str) -> Result<NodeId> {
    let filter = filter.to_lowercase();
    let pages = doc.pages();
    let names = pages
        .iter()
        .map(|page| {
            doc.scene
                .get(*page)
                .map(|node| node.name.clone())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    pages
        .iter()
        .zip(&names)
        .find(|(_, name)| name.to_lowercase().contains(&filter))
        .map(|(page, _)| *page)
        .ok_or_else(|| anyhow!("no page matching {filter:?}; pages: {names:?}"))
}

fn page_content_bounds(doc: &Doc, page: NodeId) -> Option<Bounds> {
    let mut union: Option<Bounds> = None;
    for child in doc.scene.children_of(Some(page)) {
        let Some(bounds) = doc.scene.world_bounds(*child) else {
            continue;
        };
        if !bounds.is_finite() {
            continue;
        }
        union = Some(match union {
            Some(current) => current.union(&bounds),
            None => bounds,
        });
    }
    union
}

fn fit_render_size(bounds: Bounds, max_side: u32) -> (u32, u32, Viewport) {
    let content_width = bounds.width().max(1.0);
    let content_height = bounds.height().max(1.0);
    let max_side = max_side.max(1) as f64;
    let scale = (max_side / content_width.max(content_height)).min(1.0);
    let width = (content_width * scale).ceil().max(1.0) as u32;
    let height = (content_height * scale).ceil().max(1.0) as u32;
    let zoom = (f64::from(width) / content_width).min(f64::from(height) / content_height);
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
