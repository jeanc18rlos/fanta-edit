use std::{collections::HashMap, env, fs, path::PathBuf, sync::Arc, time::Instant};

use anyhow::{Context as _, Result, bail, ensure};
use fanta_doc::{AssetId, Bounds, Doc, Fill, NodeData, NodeFlags, NodeId, Transform2D, Viewport};
use fanta_fig_interop::{fig_to_doc, read_fig};
use fanta_render::{
    AssetResolver, DecodedImage, InMemoryAssetResolver, RasterRenderer, RenderInputs,
    RenderMetrics, solve_scene_layout,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

const USAGE: &str = "usage: cargo run -p fig_viewer --example drag_bench -- <file.fig> [--page <name-fragment> | --page-index <zero-based-index>] [--node <depth-first-index>] [--kind any|instance|image|vector] [--size <width>x<height>] [--frames <1..10000>] [--distance <parent-units>] [--no-solve-layout]\n\nRenders a read-only import at fit-all and 100% zoom, moving one node for 120 frames by default. Without a page selector, uses the page with the most scene nodes. Without --node, prefers a visible instance, then image, then vector near the page center. --page-index and --node reuse page.index and moving_node.page_index from a prior JSON result for the same source file and importer (node index 0 is the page itself). JSON is written to stdout. This measures CPU raster rendering, not native GPUI/Metal gesture latency.";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum NodeKind {
    #[default]
    Any,
    Instance,
    Image,
    Vector,
}

#[derive(Debug)]
struct Options {
    path: PathBuf,
    page: Option<String>,
    page_index: Option<usize>,
    node: Option<usize>,
    kind: NodeKind,
    size: [u32; 2],
    frames: usize,
    distance: f64,
    solve_layout: bool,
}

#[derive(Default, Serialize)]
struct MetricTotals {
    nodes_visited: u64,
    nodes_drawn: u64,
    nodes_culled: u64,
    paths_built: u64,
    effect_layers: u64,
    layer_cache_hits: u64,
    layer_cache_misses: u64,
    incomplete_artwork_frames: usize,
    non_artwork_content_frames: usize,
    effect_failed_frames: usize,
}

impl MetricTotals {
    fn record(&mut self, metrics: &RenderMetrics) {
        self.nodes_visited += u64::from(metrics.nodes_visited);
        self.nodes_drawn += u64::from(metrics.nodes_drawn);
        self.nodes_culled += u64::from(metrics.nodes_culled);
        self.paths_built += u64::from(metrics.paths_built);
        self.effect_layers += u64::from(metrics.effect_layers);
        self.layer_cache_hits += u64::from(metrics.layer_cache_hits);
        self.layer_cache_misses += u64::from(metrics.layer_cache_misses);
        self.incomplete_artwork_frames += usize::from(metrics.incomplete_artwork);
        self.non_artwork_content_frames += usize::from(metrics.non_artwork_content);
        self.effect_failed_frames += usize::from(metrics.effect_failed);
    }
}

#[derive(Serialize)]
struct Timing {
    p50_us: f64,
    p95_us: f64,
    max_us: f64,
}

fn summarize(mut samples: Vec<f64>) -> Result<Timing> {
    ensure!(!samples.is_empty(), "cannot summarize zero frames");
    samples.sort_by(f64::total_cmp);
    let percentile = |percent: usize| {
        let index = (samples.len() * percent).div_ceil(100).saturating_sub(1);
        samples.get(index).copied().context("missing timing sample")
    };
    Ok(Timing {
        p50_us: percentile(50)?,
        p95_us: percentile(95)?,
        max_us: percentile(100)?,
    })
}

fn main() -> Result<()> {
    let Some(options) = parse_options(env::args().skip(1))? else {
        println!("{USAGE}");
        return Ok(());
    };
    let started = Instant::now();
    let bytes =
        fs::read(&options.path).with_context(|| format!("reading {}", options.path.display()))?;
    let read_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let source_sha256 = format!("{:x}", Sha256::digest(&bytes));
    let started = Instant::now();
    let fig = read_fig(&bytes).context("parsing .fig")?;
    let parse_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let started = Instant::now();
    let (mut doc, report, assets) = fig_to_doc(&fig).context("mapping .fig")?;
    let map_ms = started.elapsed().as_secs_f64() * 1_000.0;
    drop(fig);
    drop(bytes);
    eprintln!(
        "Imported {} nodes; {} malformed; {} skipped type buckets",
        report.mapped,
        report.malformed,
        report.skipped_by_type.len()
    );
    let page = find_page(&doc, options.page.as_deref(), options.page_index)?;
    let page_index = doc
        .pages()
        .iter()
        .position(|id| *id == page)
        .context("selected page is not in the document")?;
    let started = Instant::now();
    if options.solve_layout {
        solve_scene_layout(&mut doc.scene, page);
    }
    let layout_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let page_bounds =
        page_content_bounds(&doc, page).context("page has no finite visible bounds")?;
    let moving = find_node(&doc, page, page_bounds, &options)?;
    let moving_index = doc
        .scene
        .descendants_of(page)
        .position(|id| id == moving)
        .context("moving node is outside the page")?;
    let node = doc.scene.get(moving).context("selected node disappeared")?;
    let node_name = node.name.clone();
    let node_kind = node.data.kind_tag();
    let original = node.transform;
    let moving_bounds = doc
        .scene
        .world_bounds(moving)
        .context("moving node has no bounds")?;
    let page_name = doc
        .scene
        .get(page)
        .context("page disappeared")?
        .name
        .clone();
    let page_nodes = doc.scene.descendants_of(page).count().saturating_sub(1);
    let started = Instant::now();
    let (resolver, decoded_assets, rejected_assets) = decode_assets(assets);
    let decode_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let resolver: Arc<dyn AssetResolver> = Arc::new(resolver);
    let fit_zoom = (f64::from(options.size[0]) / (page_bounds.width() + 80.0))
        .min(f64::from(options.size[1]) / (page_bounds.height() + 80.0));
    ensure!(
        fit_zoom.is_finite() && fit_zoom > 0.0,
        "page cannot be fitted"
    );
    let mut runs = Vec::new();
    for (label, zoom, center) in [
        ("fit_all", fit_zoom, page_bounds.center()),
        ("100_percent", 1.0, moving_bounds.center()),
    ] {
        doc.scene.set_transform(moving, original)?;
        let viewport = Viewport {
            center: center.to_array(),
            zoom,
        };
        let mut renderer = RasterRenderer::new(options.size[0], options.size[1])
            .context("creating CPU raster surface")?;
        renderer.set_asset_resolver(resolver.clone());
        let inputs = RenderInputs {
            components: &doc.components,
            variables: &doc.variables,
            active_modes: &doc.active_modes,
            mode_generation: 0,
            motion: None,
            playback: None,
            video_fill_frames: None,
            dark_ui: false,
        };
        for _ in 0..3 {
            renderer.render_page_with(&doc.scene, &viewport, Some(page), &inputs);
        }
        eprintln!(
            "{label}: page {page_name:?} ({page_nodes} nodes), moving {node_kind} {node_name:?}, zoom {zoom:.5}, {} frames",
            options.frames
        );
        let mut transform_times = Vec::with_capacity(options.frames);
        let mut render_times = Vec::with_capacity(options.frames);
        let mut frame_times = Vec::with_capacity(options.frames);
        let mut totals = MetricTotals::default();
        for step in 1..=options.frames {
            let offset = options.distance * step as f64 / options.frames as f64;
            let transform = original.then(&Transform2D::translation(offset, 0.0));
            let frame_started = Instant::now();
            doc.scene.set_transform(moving, transform)?;
            transform_times.push(frame_started.elapsed().as_secs_f64() * 1_000_000.0);
            let render_started = Instant::now();
            let metrics = renderer.render_page_with(&doc.scene, &viewport, Some(page), &inputs);
            render_times.push(render_started.elapsed().as_secs_f64() * 1_000_000.0);
            frame_times.push(frame_started.elapsed().as_secs_f64() * 1_000_000.0);
            totals.record(&metrics);
            if step.is_multiple_of(30) || step == options.frames {
                eprintln!("{label}: {step}/{} frames complete", options.frames);
            }
        }
        let render = summarize(render_times)?;
        eprintln!(
            "{label}: CPU render p50={:.2}ms p95={:.2}ms max={:.2}ms; layer hits={} misses={}",
            render.p50_us / 1_000.0,
            render.p95_us / 1_000.0,
            render.max_us / 1_000.0,
            totals.layer_cache_hits,
            totals.layer_cache_misses
        );
        runs.push(serde_json::json!({
            "label": label, "zoom": zoom, "center": center.to_array(),
            "transform": summarize(transform_times)?, "cpu_render": render,
            "transform_plus_cpu_render": summarize(frame_times)?,
            "metric_totals": totals,
        }));
    }
    let result = serde_json::json!({
        "measurement": "CPU raster transform benchmark; excludes native GPUI event dispatch, inspector, Metal GPU presentation and undo/drop layout",
        "debug_assertions": cfg!(debug_assertions),
        "source": options.path,
        "source_sha256": source_sha256,
        "executable": env::current_exe().context("locating benchmark binary")?,
        "page": { "id": page.to_string(), "index": page_index, "name": page_name, "scene_nodes": page_nodes },
        "moving_node": { "id": moving.to_string(), "page_index": moving_index,
            "name": node_name, "kind": node_kind,
            "initial_world_bounds": [moving_bounds.min_x, moving_bounds.min_y, moving_bounds.max_x, moving_bounds.max_y] },
        "surface_pixels": options.size, "warmup_frames": 3, "measured_frames_per_zoom": options.frames,
        "distance_parent_units": options.distance,
        "import": { "file_read_ms": read_ms, "parse_ms": parse_ms, "map_ms": map_ms, "layout_ms": layout_ms, "decode_ms": decode_ms,
            "mapped_nodes": report.mapped, "malformed_nodes": report.malformed, "skipped_type_buckets": report.skipped_by_type.len(),
            "decoded_assets": decoded_assets, "rejected_assets": rejected_assets },
        "runs": runs,
    });
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn parse_options(mut arguments: impl Iterator<Item = String>) -> Result<Option<Options>> {
    let Some(path) = arguments.next() else {
        bail!("missing .fig path\n\n{USAGE}")
    };
    if matches!(path.as_str(), "--help" | "-h") {
        return Ok(None);
    }
    ensure!(!path.starts_with('-'), "expected .fig path, got {path:?}");
    let mut options = Options {
        path: path.into(),
        page: None,
        page_index: None,
        node: None,
        kind: NodeKind::Any,
        size: [1280, 800],
        frames: 120,
        distance: 60.0,
        solve_layout: true,
    };
    while let Some(argument) = arguments.next() {
        if argument == "--no-solve-layout" {
            options.solve_layout = false;
            continue;
        }
        if matches!(argument.as_str(), "--help" | "-h") {
            return Ok(None);
        }
        ensure!(
            matches!(
                argument.as_str(),
                "--page"
                    | "--page-index"
                    | "--node"
                    | "--kind"
                    | "--size"
                    | "--frames"
                    | "--distance"
            ),
            "unknown argument {argument:?}\n\n{USAGE}"
        );
        let value = arguments
            .next()
            .with_context(|| format!("{argument} needs a value"))?;
        match argument.as_str() {
            "--page" => {
                ensure!(!value.trim().is_empty(), "page name cannot be empty");
                options.page = Some(value);
            }
            "--page-index" => {
                options.page_index = Some(value.parse().context("invalid page index")?);
            }
            "--node" => {
                let index = value.parse().context("invalid depth-first node index")?;
                ensure!(index > 0, "node index 0 is the page itself");
                options.node = Some(index);
            }
            "--kind" => {
                options.kind = match value.as_str() {
                    "any" => NodeKind::Any,
                    "instance" => NodeKind::Instance,
                    "image" => NodeKind::Image,
                    "vector" => NodeKind::Vector,
                    _ => bail!("--kind expects any, instance, image or vector"),
                }
            }
            "--size" => {
                let (width, height) = value
                    .split_once('x')
                    .context("--size expects widthxheight")?;
                options.size = [
                    width.parse().context("invalid width")?,
                    height.parse().context("invalid height")?,
                ];
                ensure!(
                    options.size.iter().all(|value| (1..=8192).contains(value)),
                    "surface dimensions must be 1..8192"
                );
            }
            "--frames" => {
                options.frames = value.parse().context("invalid frame count")?;
                ensure!(
                    (1..=10_000).contains(&options.frames),
                    "frames must be 1..10000"
                );
            }
            "--distance" => {
                options.distance = value.parse().context("invalid distance")?;
                ensure!(
                    options.distance.is_finite() && options.distance > 0.0,
                    "distance must be finite and positive"
                );
            }
            _ => bail!("unknown argument {argument:?}"),
        }
    }
    ensure!(
        options.node.is_none() || options.kind == NodeKind::Any,
        "choose --node or --kind, not both"
    );
    ensure!(
        options.page.is_none() || options.page_index.is_none(),
        "choose --page or --page-index, not both"
    );
    Ok(Some(options))
}

fn find_page(doc: &Doc, filter: Option<&str>, index: Option<usize>) -> Result<NodeId> {
    if let Some(index) = index {
        return doc
            .pages()
            .get(index)
            .copied()
            .context("page index is outside the document");
    }
    let Some(filter) = filter else {
        return doc
            .pages()
            .iter()
            .copied()
            .max_by_key(|page| doc.scene.descendants_of(*page).count())
            .context("document contains no pages");
    };
    let filter = filter.to_lowercase();
    let matches = doc
        .pages()
        .iter()
        .copied()
        .filter(|page| {
            doc.scene
                .get(*page)
                .is_some_and(|node| node.name.to_lowercase().contains(&filter))
        })
        .collect::<Vec<_>>();
    let [page] = matches.as_slice() else {
        let names = doc
            .pages()
            .iter()
            .filter_map(|page| doc.scene.get(*page))
            .map(|node| node.name.as_str())
            .collect::<Vec<_>>();
        bail!(
            "page filter {filter:?} matches {} pages; choose a unique fragment or --page-index from this zero-based list: {names:?}",
            matches.len()
        );
    };
    Ok(*page)
}

fn image_node(data: &NodeData) -> bool {
    match data {
        NodeData::Bitmap(_) => true,
        NodeData::Vector(vector) => vector
            .fills
            .iter()
            .any(|fill| matches!(fill, Fill::Image { .. })),
        NodeData::Group(group) => group
            .background
            .iter()
            .chain(&group.background_fills)
            .any(|fill| matches!(fill, Fill::Image { .. })),
        _ => false,
    }
}

fn find_node(doc: &Doc, page: NodeId, bounds: Bounds, options: &Options) -> Result<NodeId> {
    let eligible = |id: NodeId| {
        id != page
            && doc.scene.get(id).is_some_and(|node| {
                !node.flags.intersects(NodeFlags::HIDDEN | NodeFlags::LOCKED)
                    && node.opacity.get() > 0.0
                    && doc.scene.ancestors_of(id).all(|ancestor| {
                        !ancestor
                            .flags
                            .intersects(NodeFlags::HIDDEN | NodeFlags::LOCKED)
                            && ancestor.opacity.get() > 0.0
                    })
                    && doc
                        .scene
                        .ancestors_of(id)
                        .any(|ancestor| ancestor.id == page)
                    && doc.scene.world_bounds(id).is_some_and(|bounds| {
                        bounds.is_finite() && bounds.width() > 0.0 && bounds.height() > 0.0
                    })
            })
    };
    if let Some(index) = options.node {
        // Import assigns new IDs on each run; scene order lets the same source
        // file and importer select the same authored node again.
        let node = doc
            .scene
            .descendants_of(page)
            .nth(index)
            .context("node index is outside the selected page")?;
        ensure!(
            eligible(node),
            "requested node must be visible, unlocked and on the selected page"
        );
        return Ok(node);
    }
    doc.scene
        .descendants_of(page)
        .filter(|id| eligible(*id))
        .filter_map(|id| {
            let node = doc.scene.get(id)?;
            let kind = if matches!(node.data, NodeData::Instance(_)) {
                NodeKind::Instance
            } else if image_node(&node.data) {
                NodeKind::Image
            } else if matches!(node.data, NodeData::Vector(_)) {
                NodeKind::Vector
            } else {
                return None;
            };
            if options.kind != NodeKind::Any && kind != options.kind {
                return None;
            }
            let priority = match kind {
                NodeKind::Instance => 3,
                NodeKind::Image => 2,
                _ => 1,
            };
            let distance = doc
                .scene
                .world_bounds(id)?
                .center()
                .distance_squared(bounds.center());
            Some((id, priority, distance))
        })
        .max_by(|left, right| {
            left.1
                .cmp(&right.1)
                .then_with(|| right.2.total_cmp(&left.2))
        })
        .map(|(id, _, _)| id)
        .context("no visible unlocked node matches the requested kind on this page")
}

fn page_content_bounds(doc: &Doc, page: NodeId) -> Option<Bounds> {
    doc.scene
        .children_of(Some(page))
        .iter()
        .filter_map(|id| {
            let node = doc.scene.get(*id)?;
            if node.flags.contains(NodeFlags::HIDDEN) || node.opacity.get() <= 0.0 {
                return None;
            }
            doc.scene
                .world_bounds(*id)
                .filter(|bounds| bounds.is_finite())
        })
        .reduce(|left, right| left.union(&right))
}

fn decode_assets(assets: HashMap<AssetId, Vec<u8>>) -> (InMemoryAssetResolver, usize, usize) {
    let mut resolver = InMemoryAssetResolver::new();
    let mut decoded = 0;
    let mut rejected = 0;
    for (asset, bytes) in assets {
        match image::load_from_memory(&bytes) {
            Ok(image) => {
                let rgba = image.to_rgba8();
                let (width, height) = rgba.dimensions();
                resolver.insert(
                    asset,
                    DecodedImage::new(Arc::new(rgba.into_raw()), width, height),
                );
                decoded += 1;
            }
            Err(error) => {
                rejected += 1;
                eprintln!("Image asset {asset} could not decode: {error:#}");
            }
        }
    }
    (resolver, decoded, rejected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> Result<Option<Options>> {
        parse_options(arguments.iter().map(|value| (*value).to_owned()))
    }

    #[test]
    fn defaults_measure_120_frames_at_a_fixed_viewport_size() {
        let options = parse(&["fixture.fig"])
            .expect("valid options")
            .expect("options");
        assert_eq!(options.frames, 120);
        assert_eq!(options.size, [1280, 800]);
        assert_eq!(options.kind, NodeKind::Any);
        assert!(parse(&["--help"]).expect("help").is_none());
        assert_eq!(
            parse(&["fixture.fig", "--node", "17"])
                .expect("valid node index")
                .expect("options")
                .node,
            Some(17)
        );
    }

    #[test]
    fn invalid_measurement_arguments_are_rejected() {
        for arguments in [
            vec!["fixture.fig", "--frames", "0"],
            vec!["fixture.fig", "--frames", "10001"],
            vec!["fixture.fig", "--size", "0x800"],
            vec!["fixture.fig", "--size", "1280"],
            vec!["fixture.fig", "--size", "9000x800"],
            vec!["fixture.fig", "--distance", "NaN"],
            vec!["fixture.fig", "--distance", "-1"],
            vec!["fixture.fig", "--kind", "unknown"],
            vec!["fixture.fig", "--page", ""],
            vec!["fixture.fig", "--page-index", "-1"],
            vec!["fixture.fig", "--page", "Name", "--page-index", "0"],
            vec!["fixture.fig", "--node", "not-an-id"],
            vec!["fixture.fig", "--node", "0"],
            vec!["fixture.fig", "--node", "1", "--kind", "image"],
            vec!["fixture.fig", "--frames"],
            vec!["fixture.fig", "--unknown"],
        ] {
            assert!(parse(&arguments).is_err(), "{arguments:?}");
        }
    }

    #[test]
    fn percentiles_use_nearest_rank_and_include_the_slowest_frame() {
        let timing = summarize(vec![50.0, 10.0, 40.0, 20.0, 30.0]).expect("timings");
        assert_eq!(timing.p50_us, 30.0);
        assert_eq!(timing.p95_us, 50.0);
        assert_eq!(timing.max_us, 50.0);
        assert!(summarize(Vec::new()).is_err());
    }
}
