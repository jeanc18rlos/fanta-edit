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

const USAGE: &str = "usage: cargo run -p fig_viewer --example drag_bench -- <file.fig> [--page <name-fragment> | --page-index <zero-based-index>] [--node <depth-first-index>] [--kind any|instance|image|vector] [--size <width>x<height>] [--frames <1..10000>] [--distance <parent-units>] [--no-solve-layout] [--metal-retained]\n\nRenders a read-only import at fit-all and 100% zoom, moving one node for 120 frames by default. Without a page selector, uses the page with the most scene nodes. Without --node, prefers a visible instance, then image, then vector near the page center. --page-index and --node reuse page.index and moving_node.page_index from a prior JSON result for the same source file and importer (node index 0 is the page itself). JSON is written to stdout. Default mode measures CPU raster rendering. --metal-retained requires macOS Metal and FANTA_PERF unset; it compares full Metal rendering with the retained GPU API, accepts timing only after complete paired pixel parity, and never activates the live canvas. Neither mode measures native GPUI input latency or displayed FPS.";

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
    metal_retained: bool,
}

#[derive(Default, Serialize)]
struct MetricTotals {
    nodes_visited: u64,
    nodes_drawn: u64,
    nodes_culled: u64,
    paths_built: u64,
    instance_indexes_built: u64,
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
        self.instance_indexes_built += u64::from(metrics.instance_indexes_built);
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
    if options.metal_retained {
        ensure!(
            cfg!(target_os = "macos"),
            "--metal-retained requires macOS Metal"
        );
        ensure!(
            env::var_os("FANTA_PERF").is_none(),
            "unset FANTA_PERF for isolated paired Metal timing"
        );
    }
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
    let node_has_image_fill = image_node(&node.data);
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
    let mut metal_failed = false;
    for (label, zoom, center) in [
        ("fit_all", fit_zoom, page_bounds.center()),
        ("100_percent", 1.0, moving_bounds.center()),
    ] {
        doc.scene.set_transform(moving, original)?;
        let viewport = Viewport {
            center: center.to_array(),
            zoom,
        };
        if options.metal_retained {
            {
                eprintln!(
                    "{label}: paired Metal full/retained, {node_kind} {node_name:?}, zoom {zoom:.5}"
                );
                let result = metal_bench::run(
                    &mut doc, page, moving, original, &viewport, &resolver, &options,
                );
                match result {
                    Ok(report) => {
                        metal_failed |= report
                            .get("timing_valid")
                            .and_then(serde_json::Value::as_bool)
                            != Some(true);
                        runs.push(serde_json::json!({"label":label,"zoom":zoom,"center":center.to_array(),"metal":report}));
                    }
                    Err(error) => {
                        runs.push(serde_json::json!({"label":label,"zoom":zoom,"center":center.to_array(),"metal":{
                            "accepted":false,"parity_passed":false,"timing_valid":false,"error":format!("{error:#}")
                        }}));
                        metal_failed = true;
                    }
                }
            }
            continue;
        }
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
        "measurement": if options.metal_retained {
            "Paired full/retained actual Metal renderer backend wall time through GPU completion; excludes input dispatch, inspector, GPUI presentation and release/Undo layout; no live activation"
        } else {
            "CPU raster transform benchmark; excludes native GPUI event dispatch, inspector, Metal GPU presentation and undo/drop layout"
        },
        "timing_valid": options.metal_retained.then_some(!metal_failed),
        "layout_solved": options.solve_layout,
        "debug_assertions": cfg!(debug_assertions),
        "source": options.path,
        "source_sha256": source_sha256,
        "executable": env::current_exe().context("locating benchmark binary")?,
        "page": { "id": page.to_string(), "index": page_index, "name": page_name, "scene_nodes": page_nodes },
        "moving_node": { "id": moving.to_string(), "page_index": moving_index,
            "name": node_name, "kind": node_kind, "has_image_fill": node_has_image_fill,
            "initial_world_bounds": [moving_bounds.min_x, moving_bounds.min_y, moving_bounds.max_x, moving_bounds.max_y] },
        "surface_pixels": options.size, "warmup_frames": 3, "measured_frames_per_zoom": options.frames,
        "distance_parent_units": options.distance,
        "import": { "file_read_ms": read_ms, "parse_ms": parse_ms, "map_ms": map_ms, "layout_ms": layout_ms, "decode_ms": decode_ms,
            "mapped_nodes": report.mapped, "malformed_nodes": report.malformed, "skipped_type_buckets": report.skipped_by_type.len(),
            "decoded_assets": decoded_assets, "rejected_assets": rejected_assets },
        "runs": runs,
    });
    println!("{}", serde_json::to_string_pretty(&result)?);
    ensure!(
        !metal_failed,
        "Metal retained run refused or failed parity; no timing from the failed run is accepted"
    );
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
        metal_retained: false,
    };
    while let Some(argument) = arguments.next() {
        if argument == "--metal-retained" {
            ensure!(!options.metal_retained, "--metal-retained specified twice");
            options.metal_retained = true;
            continue;
        }
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
        assert!(!options.metal_retained);
        assert!(
            parse(&["fixture.fig", "--metal-retained"])
                .expect("Metal options")
                .expect("options")
                .metal_retained
        );
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
            vec!["fixture.fig", "--metal-retained", "--metal-retained"],
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

#[cfg(target_os = "macos")]
mod metal_bench {
    use super::*;
    use fanta_render::{RetainedGpuTarget, RetainedTranslationSession};
    use metal::foreign_types::ForeignType;
    use skia_safe::gpu::{self, SyncCpu, backend_render_targets, direct_contexts, mtl};
    use skia_safe::{AlphaType, ColorType, ImageInfo};
    use std::io::Write;

    const PARITY_THRESHOLD: u8 = 2;
    const WARMUP_FRAMES: usize = 3;
    const RESOURCE_CACHE_LIMIT: usize = 1024 << 20;

    struct MetalOwner {
        context: gpu::DirectContext,
        _queue: metal::CommandQueue,
        device: metal::Device,
    }

    impl MetalOwner {
        fn new() -> Result<Self> {
            let device =
                metal::Device::system_default().context("actual Metal device unavailable")?;
            let queue = device.new_command_queue();
            // These actual owners outlive the Skia context and every target.
            let backend = unsafe {
                mtl::BackendContext::new(
                    device.as_ptr() as mtl::Handle,
                    queue.as_ptr() as mtl::Handle,
                )
            };
            let mut context = direct_contexts::make_metal(&backend, None)
                .context("creating actual Metal context")?;
            context.set_resource_cache_limit(RESOURCE_CACHE_LIMIT);
            Ok(Self {
                context,
                _queue: queue,
                device,
            })
        }

        fn target(&mut self, size: [u32; 2]) -> Result<Target> {
            let descriptor = metal::TextureDescriptor::new();
            descriptor.set_texture_type(metal::MTLTextureType::D2);
            descriptor.set_pixel_format(metal::MTLPixelFormat::BGRA8Unorm);
            descriptor.set_width(u64::from(size[0]));
            descriptor.set_height(u64::from(size[1]));
            descriptor.set_storage_mode(metal::MTLStorageMode::Private);
            descriptor.set_usage(
                metal::MTLTextureUsage::RenderTarget | metal::MTLTextureUsage::ShaderRead,
            );
            let texture = self.device.new_texture(&descriptor);
            // The texture owner stays alive until after the wrapped target drops.
            let info = unsafe { mtl::TextureInfo::new(texture.as_ptr() as mtl::Handle) };
            let backend = backend_render_targets::make_mtl(
                (i32::try_from(size[0])?, i32::try_from(size[1])?),
                &info,
            );
            let target = RetainedGpuTarget::wrap_top_left(
                &mut self.context,
                &backend,
                ColorType::BGRA8888,
                None,
            )?;
            Ok(Target {
                target,
                _backend: backend,
                _texture: texture,
            })
        }
    }

    struct Target {
        target: RetainedGpuTarget,
        _backend: gpu::BackendRenderTarget,
        _texture: metal::Texture,
    }

    impl Target {
        fn pixels(&mut self, context: &mut gpu::DirectContext) -> Result<Vec<u8>> {
            let image = self.target.image_snapshot()?;
            let size = image.dimensions();
            let info = ImageInfo::new(size, ColorType::RGBA8888, AlphaType::Premul, None);
            let row = usize::try_from(size.width)?
                .checked_mul(4)
                .context("readback row overflow")?;
            let length = row
                .checked_mul(usize::try_from(size.height)?)
                .context("readback size overflow")?;
            let mut pixels = vec![0; length];
            ensure!(
                image.read_pixels_with_context(
                    Some(context),
                    &info,
                    &mut pixels,
                    row,
                    (0, 0),
                    skia_safe::image::CachingHint::Disallow,
                ),
                "actual Metal readback failed"
            );
            Ok(pixels)
        }
    }

    #[derive(Clone, Copy, Serialize)]
    struct Sample {
        encode_us: f64,
        compose_us: f64,
        flush_gpu_complete_us: f64,
        total_gpu_complete_us: f64,
    }

    struct Frame {
        sample: Sample,
        metrics: RenderMetrics,
    }

    fn micros(started: Instant) -> f64 {
        started.elapsed().as_secs_f64() * 1_000_000.0
    }

    fn complete(metrics: &RenderMetrics) -> Result<()> {
        ensure!(
            !metrics.incomplete_artwork && !metrics.effect_failed && !metrics.non_artwork_content,
            "incomplete artwork, failed effects or non-artwork placeholders cannot validate timing"
        );
        Ok(())
    }

    fn normal_frame(
        renderer: &mut RasterRenderer,
        target: &mut Target,
        doc: &Doc,
        page: NodeId,
        viewport: &Viewport,
        size: [u32; 2],
    ) -> Result<Frame> {
        let total_started = Instant::now();
        let encode_started = Instant::now();
        let metrics = renderer.render_to_canvas(
            target.target.canvas()?,
            size[0],
            size[1],
            &doc.scene,
            viewport,
            Some(page),
            &RenderInputs::for_doc(doc),
        );
        let encode_us = micros(encode_started);
        complete(&metrics)?;
        let flush_started = Instant::now();
        target.target.flush_and_submit(SyncCpu::Yes)?;
        let flush_gpu_complete_us = micros(flush_started);
        Ok(Frame {
            sample: Sample {
                encode_us,
                compose_us: 0.0,
                flush_gpu_complete_us,
                total_gpu_complete_us: micros(total_started),
            },
            metrics,
        })
    }

    fn retained_frame(
        session: &mut RetainedTranslationSession,
        renderer: &mut RasterRenderer,
        target: &mut Target,
        doc: &Doc,
        viewport: &Viewport,
        resolver: &dyn AssetResolver,
    ) -> Result<Frame> {
        let total_started = Instant::now();
        let encode_started = Instant::now();
        let frame = session.render_for_target(
            renderer,
            &mut target.target,
            &doc.scene,
            viewport,
            &RenderInputs::for_doc(doc),
            Some(resolver),
            0,
        )?;
        let encode_us = micros(encode_started);
        complete(&frame.metrics.middle)?;
        let compose_started = Instant::now();
        let mut paint = skia_safe::Paint::default();
        paint.set_blend_mode(skia_safe::BlendMode::Src);
        target
            .target
            .canvas()?
            .draw_image(&frame.image, (0, 0), Some(&paint));
        let compose_us = micros(compose_started);
        let flush_started = Instant::now();
        target.target.flush_and_submit(SyncCpu::Yes)?;
        let flush_gpu_complete_us = micros(flush_started);
        // Drop each image before the next frame so the benchmark cannot keep
        // old output snapshots alive and hide their copy-on-write storage cost.
        drop(frame.image);
        Ok(Frame {
            sample: Sample {
                encode_us,
                compose_us,
                flush_gpu_complete_us,
                total_gpu_complete_us: micros(total_started),
            },
            metrics: frame.metrics.middle,
        })
    }

    fn parity(expected: &[u8], actual: &[u8]) -> Result<u8> {
        ensure!(
            expected.len() == actual.len(),
            "full-frame output lengths differ"
        );
        let maximum = expected
            .iter()
            .zip(actual)
            .map(|(left, right)| left.abs_diff(*right))
            .max()
            .context("empty parity output")?;
        ensure!(
            maximum <= PARITY_THRESHOLD,
            "full-frame parity failed: maximum channel difference {maximum} exceeds {PARITY_THRESHOLD}"
        );
        Ok(maximum)
    }

    fn sample_summary(samples: &[Sample]) -> Result<serde_json::Value> {
        Ok(serde_json::json!({
            "encode":summarize(samples.iter().map(|sample|sample.encode_us).collect())?,
            "compose":summarize(samples.iter().map(|sample|sample.compose_us).collect())?,
            "flush_gpu_complete":summarize(samples.iter().map(|sample|sample.flush_gpu_complete_us).collect())?,
            "total_gpu_complete":summarize(samples.iter().map(|sample|sample.total_gpu_complete_us).collect())?,
        }))
    }

    struct HashWriter(Sha256);
    impl Write for HashWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn document_hash(doc: &Doc) -> Result<String> {
        let mut writer = HashWriter(Sha256::new());
        serde_json::to_writer(&mut writer, doc)?;
        Ok(format!("{:x}", writer.0.finalize()))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn run(
        doc: &mut Doc,
        page: NodeId,
        moving: NodeId,
        original: Transform2D,
        viewport: &Viewport,
        resolver: &Arc<dyn AssetResolver>,
        options: &Options,
    ) -> Result<serde_json::Value> {
        let before = document_hash(doc)?;
        let mut evidence = serde_json::json!({
            "accepted":false,"parity_passed":false,"timing_valid":false,
            "stage":"setup","raw_pairs":[],"parity_threshold":PARITY_THRESHOLD,
            "target_allocation_wrapping_rebinding":"outside per-frame totals; reused targets; live IOSurface jobs wrap per job",
            "timing_scope":"diagnostic raw timings are invalid until every pair passes; not native latency"
        });
        let result = metal::objc::rc::autoreleasepool(|| {
            run_inner(
                doc,
                page,
                moving,
                original,
                viewport,
                resolver,
                options,
                &mut evidence,
            )
        });
        doc.scene
            .set_transform(moving, original)
            .context("restoring in-memory benchmark transform")?;
        ensure!(
            document_hash(doc)? == before,
            "benchmark changed authored document data"
        );
        let mut report = match result {
            Ok(report) => report,
            Err(error) => {
                evidence["error"] = format!("{error:#}").into();
                evidence
            }
        };
        report["authored_document_unchanged"] = true.into();
        report["authored_document_sha256"] = before.into();
        Ok(report)
    }

    #[allow(clippy::too_many_arguments)]
    fn run_inner(
        doc: &mut Doc,
        page: NodeId,
        moving: NodeId,
        original: Transform2D,
        viewport: &Viewport,
        resolver: &Arc<dyn AssetResolver>,
        options: &Options,
        evidence: &mut serde_json::Value,
    ) -> Result<serde_json::Value> {
        let setup_started = Instant::now();
        let mut normal_owner = MetalOwner::new()?;
        let mut retained_owner = MetalOwner::new()?;
        ensure!(
            normal_owner.device.registry_id() == retained_owner.device.registry_id(),
            "paired contexts selected different Metal devices"
        );
        let device = normal_owner.device.name().to_owned();
        let mut normal_target = normal_owner.target(options.size)?;
        let mut retained_target = retained_owner.target(options.size)?;
        let mut normal_renderer = RasterRenderer::new(options.size[0], options.size[1])?;
        let mut retained_renderer = RasterRenderer::new(options.size[0], options.size[1])?;
        for renderer in [&mut normal_renderer, &mut retained_renderer] {
            renderer.set_asset_resolver(Arc::clone(resolver));
            renderer.background = fanta_doc::Color::rgb(245, 245, 245);
            renderer.set_pixel_snap_pan(true);
        }
        let setup_us = micros(setup_started);
        evidence["stage"] = "prepare".into();
        evidence["device"] = device.clone().into();
        evidence["setup_both_contexts_targets_renderers_us"] = setup_us.into();
        let prepare_started = Instant::now();
        let mut session = RetainedTranslationSession::prepare_for_target(
            &mut retained_renderer,
            &mut retained_target.target,
            &doc.scene,
            page,
            moving,
            viewport,
            &RenderInputs::for_doc(doc),
            Some(resolver.as_ref()),
            0,
        )
        .context("retained preparation refused")?;
        // Preparation paints private Below storage. Finish that context's work
        // before stopping the outer preparation clock, not after timing begins.
        retained_owner.context.flush(None);
        ensure!(
            retained_owner.context.submit(SyncCpu::Yes),
            "retained preparation GPU submit failed"
        );
        ensure!(
            !retained_owner.context.abandoned(),
            "retained context lost during preparation"
        );
        let prepare_gpu_complete_us = micros(prepare_started);
        evidence["retained_prepare_gpu_complete_us"] = prepare_gpu_complete_us.into();
        let build = session.build_metrics().clone();
        complete(&build.below)?;
        complete(&build.above)?;
        let mut normal_samples = Vec::with_capacity(options.frames);
        let mut retained_samples = Vec::with_capacity(options.frames);
        let mut normal_warmup = Vec::with_capacity(WARMUP_FRAMES);
        let mut retained_warmup = Vec::with_capacity(WARMUP_FRAMES);
        let mut transform_samples = Vec::with_capacity(options.frames);
        let mut normal_metrics = MetricTotals::default();
        let mut retained_metrics = MetricTotals::default();
        let mut maximum_difference = 0;
        let mut parity_frames = 0;
        let mut changed_from_first = false;
        let mut first_pixels_hash = None;
        for index in 0..(WARMUP_FRAMES + options.frames) {
            evidence["stage"] = "paired_render".into();
            evidence["pair_index"] = index.into();
            let measuring = index >= WARMUP_FRAMES;
            let step = index.saturating_sub(WARMUP_FRAMES) + 1;
            let offset = if measuring {
                options.distance * step as f64 / options.frames as f64
            } else {
                0.0
            };
            let transform_started = Instant::now();
            doc.scene.set_transform(
                moving,
                original.then(&Transform2D::translation(offset, 0.0)),
            )?;
            doc.components.bump_preview_for_node(&doc.scene, moving);
            let transform_us = micros(transform_started);
            // Alternate pair order to avoid always giving one path the first
            // slot. Each path finishes GPU work before the other starts.
            let (normal, retained) = if index.is_multiple_of(2) {
                let normal = normal_frame(
                    &mut normal_renderer,
                    &mut normal_target,
                    doc,
                    page,
                    viewport,
                    options.size,
                )?;
                let retained = retained_frame(
                    &mut session,
                    &mut retained_renderer,
                    &mut retained_target,
                    doc,
                    viewport,
                    resolver.as_ref(),
                )?;
                (normal, retained)
            } else {
                let retained = retained_frame(
                    &mut session,
                    &mut retained_renderer,
                    &mut retained_target,
                    doc,
                    viewport,
                    resolver.as_ref(),
                )?;
                let normal = normal_frame(
                    &mut normal_renderer,
                    &mut normal_target,
                    doc,
                    page,
                    viewport,
                    options.size,
                )?;
                (normal, retained)
            };
            // CPU readback and full-image comparison happen only after both
            // GPU-complete timers stop, identically for warmup and measured pairs.
            let expected = normal_target.pixels(&mut normal_owner.context)?;
            let actual = retained_target.pixels(&mut retained_owner.context)?;
            evidence["stage"] = "parity".into();
            let pair_maximum = expected
                .iter()
                .zip(&actual)
                .map(|(left, right)| left.abs_diff(*right))
                .max();
            let differing_channels = expected
                .iter()
                .zip(&actual)
                .filter(|(left, right)| left != right)
                .count();
            evidence["raw_pairs"].as_array_mut().context("paired evidence is not an array")?.push(serde_json::json!({
                "index":index,"warmup":!measuring,"offset":offset,"normal_first":index.is_multiple_of(2),
                "normal":normal.sample,"retained":retained.sample,"transform_us":transform_us,
                "expected_bytes":expected.len(),"actual_bytes":actual.len(),
                "maximum_channel_difference":pair_maximum,"differing_channels":differing_channels,
                "normal_pixels_sha256":format!("{:x}",Sha256::digest(&expected)),
                "retained_pixels_sha256":format!("{:x}",Sha256::digest(&actual))
            }));
            let difference = parity(&expected, &actual)
                .with_context(|| format!("pair {index}, parent-space offset {offset}"))?;
            maximum_difference = maximum_difference.max(difference);
            parity_frames += 1;
            let hash = Sha256::digest(&expected);
            match first_pixels_hash {
                Some(first) => changed_from_first |= first != hash,
                None => first_pixels_hash = Some(hash),
            }
            if measuring {
                transform_samples.push(transform_us);
                normal_metrics.record(&normal.metrics);
                retained_metrics.record(&retained.metrics);
                normal_samples.push(normal.sample);
                retained_samples.push(retained.sample);
                if step.is_multiple_of(30) || step == options.frames {
                    eprintln!(
                        "Metal paired frames {step}/{}; max channel difference {maximum_difference}",
                        options.frames
                    );
                }
            } else {
                normal_warmup.push(normal.sample);
                retained_warmup.push(retained.sample);
            }
        }
        evidence["stage"] = "motion_observability".into();
        evidence["parity_passed"] = true.into();
        evidence["parity"] = serde_json::json!({
            "full_frame_pairs":parity_frames,"threshold":PARITY_THRESHOLD,
            "maximum_channel_difference":maximum_difference,
            "moving_target_changes_pixels":changed_from_first
        });
        ensure!(
            changed_from_first,
            "moving target changed no pixels in this viewport; no valid timing comparison"
        );
        let normal_sum: f64 = normal_samples
            .iter()
            .map(|sample| sample.total_gpu_complete_us)
            .sum();
        let retained_sum: f64 = retained_samples
            .iter()
            .map(|sample| sample.total_gpu_complete_us)
            .sum();
        let savings_per_frame = (normal_sum - retained_sum) / options.frames as f64;
        let break_even_frames =
            (savings_per_frame > 0.0).then(|| (prepare_gpu_complete_us / savings_per_frame).ceil());
        let surface_bytes = u64::from(options.size[0]) * u64::from(options.size[1]) * 4;
        Ok(serde_json::json!({
            "accepted":true,"parity_passed":true,"timing_valid":true,
            "device":device,"device_registry_id":normal_owner.device.registry_id(),
            "backend":"Skia Ganesh Metal; independent contexts on the same device",
            "target":"BGRA8888 premultiplied TopLeft; display scale 1; pixel-snap pan enabled",
            "paired_order":"alternating normal/retained first; GPU-complete before next path",
            "target_allocation_wrapping_rebinding":"outside per-frame totals; targets reused; live IOSurface jobs wrap per job",
            "raw_pairs":evidence.get("raw_pairs"),
            "frames":options.frames,"warmup_frames":WARMUP_FRAMES,
            "parity":{"full_frame_pairs":parity_frames,"threshold":PARITY_THRESHOLD,"maximum_channel_difference":maximum_difference,"moving_target_changes_pixels":changed_from_first},
            "setup_both_contexts_targets_renderers_us":setup_us,
            "retained_prepare_gpu_complete_us":prepare_gpu_complete_us,
            "retained_prepare_encode_us":build.build_micros,
            "retained_prepare_phases_us":{"validation":build.backend_validation_micros,"surface_allocation":build.surface_allocation_micros,"prepare":build.prepare_micros,"below":build.below_micros,"above_record":build.above_record_micros},
            "normal":sample_summary(&normal_samples)?,"retained":sample_summary(&retained_samples)?,
            "transform_and_preview_revision":summarize(transform_samples)?,
            "normal_warmup":normal_warmup,"retained_warmup":retained_warmup,
            "normal_samples":normal_samples,"retained_samples":retained_samples,
            "normal_metric_totals":normal_metrics,"retained_middle_metric_totals":retained_metrics,
            "build_amortization":{
                "normal_measured_total_us":normal_sum,
                "retained_measured_total_us":retained_sum,
                "retained_measured_plus_prepare_us":retained_sum+prepare_gpu_complete_us,
                "normal_including_warmup_us":normal_sum+normal_warmup.iter().map(|sample|sample.total_gpu_complete_us).sum::<f64>(),
                "retained_including_prepare_and_warmup_us":retained_sum+prepare_gpu_complete_us+retained_warmup.iter().map(|sample|sample.total_gpu_complete_us).sum::<f64>(),
                "estimated_frames_to_amortize_prepare_at_measured_mean":break_even_frames,
                "assumption":"fixed-view accepted translation, no invalidation; estimate is not a measured native gesture"
            },
            "memory_accounting":{
                "retained_surface_bytes":build.surface_bytes,
                "frozen_decoded_pixel_bytes":build.frozen_pixel_bytes,
                "approximate_recorded_picture_bytes":build.picture_bytes,
                "accounted_retained_bytes":build.surface_bytes.saturating_add(build.frozen_pixel_bytes).saturating_add(build.picture_bytes),
                "prepared_nodes":build.prepared_nodes,"prepared_instances":build.prepared_instances,
                "external_target_bytes_two_contexts":surface_bytes*2,
                "paired_cpu_readback_bytes":surface_bytes*2,
                "returned_frame_images_max_live":1,"readback_snapshots_max_live_per_context":1,
                "ganesh_resource_cache_limit_per_context":RESOURCE_CACHE_LIMIT,
                "scope":"arithmetic storage estimates, not measured VRAM or peak RSS; excludes driver allocations, font/image caches and scratch surfaces"
            },
            "scope":"GPU-complete standalone renderer backend wall time only; parity readback, importer, source I/O, UI dispatch, inspector, live MacGpuRenderer/render_thread_loop, native composition/presentation and release/Undo layout excluded; no live canvas activation"
        }))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parity_rejects_empty_different_size_and_above_threshold_outputs() {
            assert!(parity(&[], &[]).is_err());
            assert!(parity(&[0, 0, 0, 255], &[0, 0, 0]).is_err());
            assert!(parity(&[0, 0, 0, 255], &[0, 0, 3, 255]).is_err());
            assert_eq!(
                parity(&[0, 0, 0, 255], &[0, 0, 2, 255]).expect("existing threshold"),
                2
            );
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod metal_bench {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    pub(super) fn run(
        _doc: &mut Doc,
        _page: NodeId,
        _moving: NodeId,
        _original: Transform2D,
        _viewport: &Viewport,
        _resolver: &Arc<dyn AssetResolver>,
        _options: &Options,
    ) -> Result<serde_json::Value> {
        bail!("--metal-retained requires macOS Metal")
    }
}
