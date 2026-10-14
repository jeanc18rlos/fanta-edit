//! Controlled navigation benchmark for the native canvas pipeline.
//!
//! Drives the production [`GpuCanvas`] paint planner and its Metal render
//! thread on a real project with a scripted, display-paced input stream.
//! GPUI's compositor is not involved: a tick is one paint pass at a fixed
//! 60 Hz cadence, with camera input arriving between paints the way trackpad
//! events arrive between display frames. Every tick records what paint would
//! present; with `FANTA_PERF=1` every render-thread job is captured too.
//!
//! ```text
//! FANTA_PERF=1 FANTA_CAMERA_BENCH_PROJECT=<project dir> \
//! FANTA_CAMERA_BENCH_OUT=<result.json> \
//! cargo test -p fig_viewer --lib navigation_bench -- --ignored --nocapture
//! ```

use super::*;
use crate::document::{FigDocument, fit_bounds, load_project_document, try_page_bounds};
use std::path::PathBuf;

const TICK: Duration = Duration::from_micros(16_667);
/// Trackpad events land shortly after a display frame (the native logs show
/// input ~14 ms old at the next paint).
const INPUT_DELAY: Duration = Duration::from_micros(2_500);
const SETTLE_TIMEOUT: Duration = Duration::from_secs(20);

struct CapturedLog {
    origin: Instant,
    lines: Mutex<Vec<(f64, String)>>,
}

impl log::Log for CapturedLog {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record) {
        let message = record.args().to_string();
        if message.starts_with("fanta_canvas_frame")
            || message.starts_with("fanta_canvas_cancelled")
            || message.starts_with("fanta perf")
            || message.starts_with("retained")
        {
            let at = self.origin.elapsed().as_secs_f64() * 1000.0;
            if let Ok(mut lines) = self.lines.lock() {
                lines.push((at, message));
            }
        }
    }

    fn flush(&self) {}
}

#[derive(serde::Serialize)]
struct TickRecord {
    t_ms: f64,
    moving: bool,
    requested_center: [f64; 2],
    requested_zoom: f64,
    presentation: &'static str,
    frame_request: Option<u64>,
    frame_center: Option<[f64; 2]>,
    frame_zoom: Option<f64>,
    /// Age of the input whose camera the presented frame was rendered for.
    frame_input_age_ms: Option<f64>,
    paint_ms: f64,
    in_flight: bool,
}

#[derive(serde::Serialize)]
struct InputRecord {
    t_ms: f64,
    center: [f64; 2],
    zoom: f64,
}

#[derive(serde::Serialize, Default)]
struct PageSwitchRecord {
    from_page: Option<String>,
    to_page: Option<String>,
    ensure_page_solved_ms: f64,
    scope_scan_ms: f64,
    fit_bounds_ms: f64,
    first_paint_ms: f64,
    first_presented_ms: Option<f64>,
    first_fresh_ms: Option<f64>,
    settled_ms: Option<f64>,
}

#[derive(serde::Serialize, Default)]
struct EditRecord {
    kind: &'static str,
    apply_ms: f64,
    first_paint_ms: f64,
    first_fresh_ms: Option<f64>,
    settled_ms: Option<f64>,
}

#[derive(serde::Serialize)]
struct ScenarioRecord {
    name: String,
    started_ms: f64,
    finished_ms: f64,
    warmup_ms: Option<f64>,
    inputs: Vec<InputRecord>,
    ticks: Vec<TickRecord>,
    page_switch: Option<PageSwitchRecord>,
    edits: Vec<EditRecord>,
    physical_footprint_mib: Option<f64>,
    physical_footprint_peak_mib: Option<f64>,
}

#[derive(serde::Serialize)]
struct BenchmarkRecord {
    project: String,
    page: Option<String>,
    element_logical: (f64, f64),
    render_size: (u32, u32),
    scale_factor: f32,
    load_ms: f64,
    /// Tick and log timestamps share one origin.
    log_offset_ms: f64,
    dependency_compare_us: f64,
    scenarios: Vec<ScenarioRecord>,
    log: Vec<(f64, String)>,
}

struct Bench {
    document: FigDocument,
    gpu: GpuCanvas,
    render_thread: Option<std::thread::JoinHandle<()>>,
    origin: Instant,
    logical: (f64, f64),
    frame_logical: (f64, f64),
    render_size: (u32, u32),
    scale_factor: f32,
    page_index: usize,
    page_root: Option<NodeId>,
    viewport: Viewport,
    inputs: Vec<(Viewport, Instant)>,
    maintenance_timer: Option<(u64, Instant)>,
}

impl Bench {
    fn new(
        document: FigDocument,
        page_index: usize,
        logical: (f64, f64),
        scale_factor: f32,
        origin: Instant,
    ) -> Result<Self> {
        let margin = f64::from(RENDER_MARGIN);
        let frame_logical = (logical.0 + margin * 2.0, logical.1 + margin * 2.0);
        let render_size = (
            (frame_logical.0 * f64::from(scale_factor)).round().max(1.0) as u32,
            (frame_logical.1 * f64::from(scale_factor)).round().max(1.0) as u32,
        );
        let (jobs, job_rx) = mpsc::channel();
        let (reply_tx, replies) = mpsc::channel();
        let signal = Arc::new(WakeSignal::default());
        let render_thread = std::thread::Builder::new()
            .name("fanta-canvas-render".into())
            .spawn(move || render_thread_main(render_size, job_rx, reply_tx, signal))?;
        let gpu = GpuCanvas::with_channels(None, jobs, replies, Task::ready(()), None);
        let mut bench = Self {
            document,
            gpu,
            render_thread: Some(render_thread),
            origin,
            logical,
            frame_logical,
            render_size,
            scale_factor,
            page_index,
            page_root: None,
            viewport: Viewport::default(),
            inputs: Vec::new(),
            maintenance_timer: None,
        };
        bench.switch_page(page_index)?;
        Ok(bench)
    }

    fn now_ms(&self) -> f64 {
        self.origin.elapsed().as_secs_f64() * 1000.0
    }

    fn page_label(&self, index: usize) -> Option<String> {
        self.document.pages.get(index).map(|page| {
            format!(
                "{} ({})",
                page.name,
                page.root.map(|root| root.to_string()).unwrap_or_default()
            )
        })
    }

    /// Mirrors `FigView::select_page` plus the prepaint fit that follows a
    /// page change. Returns the UI-thread cost of each step.
    fn switch_page(&mut self, index: usize) -> Result<PageSwitchRecord> {
        let from_page = self.page_label(self.page_index);
        let started = Instant::now();
        self.document.ensure_page_solved(index);
        let ensure_page_solved_ms = started.elapsed().as_secs_f64() * 1000.0;
        let root = self
            .document
            .pages
            .get(index)
            .and_then(|page| page.root)
            .context("benchmark page has no root")?;
        if self.document.doc.set_active_page(Some(root)) {
            self.document.doc.selection.clear();
        }
        if let Some(prewarm) = self.document.take_page_prewarm(root) {
            let assets = prewarm.len();
            let origin = self.origin;
            std::thread::Builder::new()
                .name("navigation-bench-prewarm".into())
                .spawn(move || {
                    let started = Instant::now();
                    prewarm.run();
                    eprintln!(
                        "prewarmed {assets} page images in {:.1} ms (finished at {:.1} ms)",
                        started.elapsed().as_secs_f64() * 1000.0,
                        origin.elapsed().as_secs_f64() * 1000.0
                    );
                })?;
        }
        let started = Instant::now();
        let _scope = self
            .document
            .doc
            .components
            .defs
            .values()
            .find(|definition| definition.root == root)
            .map(|definition| definition.id);
        let scope_scan_ms = started.elapsed().as_secs_f64() * 1000.0;
        let changed = self.page_root != Some(root);
        self.page_index = index;
        self.page_root = Some(root);
        if changed {
            self.gpu.invalidate_presentation();
        }
        let started = Instant::now();
        let viewport = try_page_bounds(&self.document.doc, Some(root))
            .map(|bounds| {
                fit_bounds(
                    bounds,
                    self.logical,
                    crate::view::RENDER_PADDING,
                    crate::view::MIN_ZOOM,
                    crate::view::MAX_ZOOM,
                )
            })
            .unwrap_or_default();
        let fit_bounds_ms = started.elapsed().as_secs_f64() * 1000.0;
        self.set_camera(viewport);
        Ok(PageSwitchRecord {
            from_page,
            to_page: self.page_label(index),
            ensure_page_solved_ms,
            scope_scan_ms,
            fit_bounds_ms,
            ..Default::default()
        })
    }

    fn set_camera(&mut self, viewport: Viewport) {
        if self.viewport != viewport {
            self.gpu.note_camera_input(viewport);
            self.viewport = viewport;
            self.inputs.push((viewport, Instant::now()));
        }
    }

    fn input_time(&self, center: [f64; 2], zoom: f64) -> Option<Instant> {
        self.inputs
            .iter()
            .rev()
            .find(|(viewport, _)| viewport.center == center && viewport.zoom == zoom)
            .map(|(_, at)| *at)
    }

    /// Mirrors `FigView::schedule_camera_maintenance` and its idle timer.
    fn schedule_maintenance(&mut self) {
        let gpu = &mut self.gpu;
        let Some(latest) = &gpu.latest_request else {
            return;
        };
        if !gpu.newest.as_ref().is_some_and(|frame| {
            frame.key == latest.key
                && frame.progress.complete_detail
                && frame.progress.needs_refresh
        }) || gpu
            .maintenance_request
            .as_ref()
            .is_none_or(|request| request.key != latest.key)
        {
            return;
        }
        let sequence = gpu.camera_mailbox.sequence.load(Ordering::Acquire);
        if gpu.maintenance_sequence == Some(sequence) {
            if let Some((armed, at)) = self.maintenance_timer
                && armed == sequence
                && at.elapsed() >= Duration::from_millis(64)
            {
                self.maintenance_timer = None;
                gpu.maintenance_ready = true;
            }
            return;
        }
        gpu.maintenance_sequence = Some(sequence);
        gpu.maintenance_ready = false;
        self.maintenance_timer = Some((sequence, Instant::now()));
    }

    fn paint(&mut self, moving: bool) -> Result<TickRecord> {
        let t_ms = self.now_ms();
        let started = Instant::now();
        let (frame, _) = self.gpu.paint_frame(
            &self.document,
            self.page_root,
            self.render_size,
            self.frame_logical,
            self.logical,
            self.viewport,
            self.scale_factor,
            None,
            None,
            Vec::new(),
            None,
            false,
        )?;
        let paint_ms = started.elapsed().as_secs_f64() * 1000.0;
        self.schedule_maintenance();
        let presentation = match &frame {
            Some(GpuFrame::Fresh(_)) => "fresh",
            Some(GpuFrame::Reprojected { .. }) => "reprojected",
            Some(GpuFrame::Held(_)) => "held",
            None => "unavailable",
        };
        let newest = self.gpu.newest.as_ref();
        let frame_center = newest.map(|frame| frame.key.viewport_center);
        let frame_zoom = newest.map(|frame| frame.key.viewport_zoom);
        let frame_input_age_ms = frame_center
            .zip(frame_zoom)
            .and_then(|(center, zoom)| self.input_time(center, zoom))
            .map(|at| at.elapsed().as_secs_f64() * 1000.0);
        Ok(TickRecord {
            t_ms,
            moving,
            requested_center: self.viewport.center,
            requested_zoom: self.viewport.zoom,
            presentation,
            frame_request: newest.map(|frame| frame.progress.pipeline.request_id),
            frame_center,
            frame_zoom,
            frame_input_age_ms,
            paint_ms,
            in_flight: self.gpu.in_flight.is_some(),
        })
    }

    fn settled(&self) -> bool {
        self.gpu.in_flight.is_none()
            && self.gpu.newest.as_ref().is_some_and(|frame| {
                self.gpu
                    .latest_request
                    .as_ref()
                    .is_some_and(|latest| latest.key == frame.key)
                    && !frame.progress.needs_refresh
            })
    }

    /// One display-paced tick: wait for the next vsync, paint, then deliver
    /// `input` shortly after, as trackpad events arrive between frames.
    fn tick(
        &mut self,
        next_vsync: &mut Instant,
        input: Option<Viewport>,
        ticks: &mut Vec<TickRecord>,
        inputs: &mut Vec<InputRecord>,
    ) -> Result<()> {
        let now = Instant::now();
        if *next_vsync > now {
            std::thread::sleep(*next_vsync - now);
        }
        *next_vsync += TICK;
        if Instant::now() > *next_vsync {
            // A paint overran its frame; realign instead of bursting.
            *next_vsync = Instant::now() + TICK;
        }
        ticks.push(self.paint(input.is_some())?);
        if let Some(viewport) = input {
            std::thread::sleep(INPUT_DELAY);
            self.set_camera(viewport);
            inputs.push(InputRecord {
                t_ms: self.now_ms(),
                center: viewport.center,
                zoom: viewport.zoom,
            });
        }
        Ok(())
    }

    /// Paint at the display cadence without input until the newest frame
    /// matches the camera and needs no refresh. Returns the settle time.
    fn settle(
        &mut self,
        ticks: &mut Vec<TickRecord>,
        inputs: &mut Vec<InputRecord>,
        timeout: Duration,
    ) -> Result<Option<f64>> {
        let started = Instant::now();
        let mut next_vsync = Instant::now();
        while started.elapsed() < timeout {
            self.tick(&mut next_vsync, None, ticks, inputs)?;
            if self.settled() {
                return Ok(Some(started.elapsed().as_secs_f64() * 1000.0));
            }
        }
        Ok(None)
    }

    /// Write the settled frame as a PNG under `FANTA_CAMERA_BENCH_SNAPSHOTS`,
    /// so two builds can be compared pixel for pixel at identical cameras.
    /// With `FANTA_CAMERA_BENCH_REFERENCE`, also write `<name>_reference`: the
    /// first frame after invalidating the camera cache, which draws the scene
    /// directly without recordings or tiles. It clears the cache, so timings
    /// of runs that write references are not comparable.
    fn snapshot(&mut self, name: &str) -> Result<()> {
        let Some(directory) = std::env::var_os("FANTA_CAMERA_BENCH_SNAPSHOTS").map(PathBuf::from)
        else {
            return Ok(());
        };
        if !self.settled() {
            eprintln!("{name}: not settled, no snapshot");
            return Ok(());
        }
        self.save_newest(&directory, name)?;
        if std::env::var_os("FANTA_CAMERA_BENCH_REFERENCE").is_none() {
            return Ok(());
        }
        self.gpu.invalidate();
        let (mut ticks, mut inputs) = (Vec::new(), Vec::new());
        let mut next_vsync = Instant::now();
        let started = Instant::now();
        loop {
            anyhow::ensure!(
                started.elapsed() < SETTLE_TIMEOUT,
                "{name}: no reference frame"
            );
            self.tick(&mut next_vsync, None, &mut ticks, &mut inputs)?;
            if self.gpu.in_flight.is_none()
                && self.gpu.newest.as_ref().is_some_and(|frame| {
                    self.gpu
                        .latest_request
                        .as_ref()
                        .is_some_and(|latest| latest.key == frame.key)
                })
            {
                break;
            }
        }
        let progress = &self
            .gpu
            .newest
            .as_ref()
            .context("no reference frame")?
            .progress;
        eprintln!(
            "{name}: reference scene_rendered={} needs_refresh={}",
            progress.scene_rendered, progress.needs_refresh
        );
        self.save_newest(&directory, &format!("{name}_reference"))
    }

    fn save_newest(&self, directory: &std::path::Path, name: &str) -> Result<()> {
        let frame = self.gpu.newest.as_ref().context("no frame to snapshot")?;
        let buffer = &frame.buffer;
        let (width, height, row_bytes) = (
            buffer.get_width(),
            buffer.get_height(),
            buffer.get_bytes_per_row(),
        );
        let status = buffer.lock_base_address(kCVPixelBufferLock_ReadOnly);
        anyhow::ensure!(status == 0, "locking the presented frame failed: {status}");
        let pixels = (|| {
            let address = unsafe { buffer.get_base_address() };
            anyhow::ensure!(!address.is_null(), "the presented frame has no pixels");
            anyhow::ensure!(row_bytes * height <= buffer.get_data_size(), "short frame");
            // SAFETY: the read-only lock keeps `row_bytes * height` bytes mapped.
            let bytes =
                unsafe { std::slice::from_raw_parts(address.cast::<u8>(), row_bytes * height) };
            let mut rgba = Vec::with_capacity(width * height * 4);
            for row in bytes.chunks_exact(row_bytes).take(height) {
                for pixel in row.get(..width * 4).context("short row")?.chunks_exact(4) {
                    rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
                }
            }
            Ok(rgba)
        })();
        let unlocked = buffer.unlock_base_address(kCVPixelBufferLock_ReadOnly);
        let pixels = pixels?;
        anyhow::ensure!(
            unlocked == 0,
            "unlocking the presented frame failed: {unlocked}"
        );
        std::fs::create_dir_all(directory)?;
        image::RgbaImage::from_raw(width as u32, height as u32, pixels)
            .context("presented frame pixels")?
            .save(directory.join(format!("{name}.png")))?;
        Ok(())
    }

    fn scenario(&self, name: &str) -> ScenarioRecord {
        ScenarioRecord {
            name: name.to_string(),
            started_ms: self.now_ms(),
            finished_ms: 0.0,
            warmup_ms: None,
            inputs: Vec::new(),
            ticks: Vec::new(),
            page_switch: None,
            edits: Vec::new(),
            physical_footprint_mib: None,
            physical_footprint_peak_mib: None,
        }
    }

    fn finish(&self, mut record: ScenarioRecord) -> ScenarioRecord {
        record.finished_ms = self.now_ms();
        let (current, peak) = physical_footprint();
        record.physical_footprint_mib = current;
        record.physical_footprint_peak_mib = peak;
        record
    }

    /// Warm the start camera, then play `path` one input per tick and paint
    /// until the pipeline settles.
    fn run_motion(
        &mut self,
        name: &str,
        start: Viewport,
        path: &[Viewport],
    ) -> Result<ScenarioRecord> {
        let mut record = self.scenario(name);
        self.set_camera(start);
        let mut warm_ticks = Vec::new();
        let mut warm_inputs = Vec::new();
        record.warmup_ms = self.settle(&mut warm_ticks, &mut warm_inputs, SETTLE_TIMEOUT)?;
        record.started_ms = self.now_ms();
        let mut next_vsync = Instant::now();
        for viewport in path {
            self.tick(
                &mut next_vsync,
                Some(*viewport),
                &mut record.ticks,
                &mut record.inputs,
            )?;
        }
        self.settle(&mut record.ticks, &mut record.inputs, SETTLE_TIMEOUT)?;
        self.snapshot(name)?;
        Ok(self.finish(record))
    }

    fn run_page_switch(&mut self, name: &str, index: usize) -> Result<ScenarioRecord> {
        let mut record = self.scenario(name);
        let mut switch = self.switch_page(index)?;
        let switched = Instant::now();
        let mut next_vsync = Instant::now();
        let mut first = true;
        while switched.elapsed() < SETTLE_TIMEOUT {
            self.tick(&mut next_vsync, None, &mut record.ticks, &mut record.inputs)?;
            let Some(tick) = record.ticks.last() else {
                continue;
            };
            if first {
                switch.first_paint_ms = tick.paint_ms;
                first = false;
            }
            let elapsed = switched.elapsed().as_secs_f64() * 1000.0;
            if tick.presentation != "unavailable" && switch.first_presented_ms.is_none() {
                switch.first_presented_ms = Some(elapsed);
            }
            if tick.presentation == "fresh" && switch.first_fresh_ms.is_none() {
                switch.first_fresh_ms = Some(elapsed);
            }
            if self.settled() {
                switch.settled_ms = Some(elapsed);
                break;
            }
        }
        record.page_switch = Some(switch);
        self.snapshot(name)?;
        Ok(self.finish(record))
    }

    fn run_edit_undo(&mut self, name: &str, target: NodeId) -> Result<ScenarioRecord> {
        let mut record = self.scenario(name);
        let mut warm_ticks = Vec::new();
        let mut warm_inputs = Vec::new();
        record.warmup_ms = self.settle(&mut warm_ticks, &mut warm_inputs, SETTLE_TIMEOUT)?;
        let old = self
            .document
            .doc
            .scene
            .get(target)
            .map(|node| node.opacity)
            .context("edit target vanished")?;
        for undo in [false, true] {
            let started = Instant::now();
            if undo {
                self.document.undo_for_benchmark()?;
            } else {
                self.document
                    .apply_for_benchmark(fanta_doc::Operation::SetOpacity {
                        id: target,
                        old,
                        new: fanta_doc::UnitInterval::new(old.get() * 0.5),
                    })?;
            }
            // FigItemEvent::Edited → FigView::invalidate_canvas_cache.
            self.gpu.invalidate();
            let mut edit = EditRecord {
                kind: if undo { "undo" } else { "apply" },
                apply_ms: started.elapsed().as_secs_f64() * 1000.0,
                ..Default::default()
            };
            let mut next_vsync = Instant::now();
            let mut first = true;
            while started.elapsed() < SETTLE_TIMEOUT {
                self.tick(&mut next_vsync, None, &mut record.ticks, &mut record.inputs)?;
                let Some(tick) = record.ticks.last() else {
                    continue;
                };
                if first {
                    edit.first_paint_ms = tick.paint_ms;
                    first = false;
                }
                let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                if tick.presentation == "fresh" && edit.first_fresh_ms.is_none() {
                    edit.first_fresh_ms = Some(elapsed);
                }
                if self.settled() {
                    edit.settled_ms = Some(elapsed);
                    break;
                }
            }
            self.snapshot(&format!("{name}_{}", edit.kind))?;
            record.edits.push(edit);
        }
        Ok(self.finish(record))
    }
}

impl Drop for Bench {
    fn drop(&mut self) {
        // Closing the job channel ends the render thread after its job.
        let (jobs, _) = mpsc::channel();
        self.gpu.jobs = jobs;
        if let Some(thread) = self.render_thread.take()
            && thread.join().is_err()
        {
            log::warn!("navigation benchmark render thread panicked");
        }
    }
}

// Runs between scenarios on the benchmark's own thread, outside any executor
// and any timed section, so blocking on vmmap is harmless.
#[allow(clippy::disallowed_methods)]
fn physical_footprint() -> (Option<f64>, Option<f64>) {
    let output = std::process::Command::new("vmmap")
        .args(["-summary", &std::process::id().to_string()])
        .output();
    let Ok(output) = output else {
        return (None, None);
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let parse = |label: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(label))
            .and_then(|value| {
                let value = value.trim();
                let (number, unit) = value.split_at(value.find(|c: char| c.is_ascii_alphabetic())?);
                let number: f64 = number.trim().parse().ok()?;
                Some(match unit.trim() {
                    "K" => number / 1024.0,
                    "M" => number,
                    "G" => number * 1024.0,
                    _ => number / (1024.0 * 1024.0),
                })
            })
    };
    (
        parse("Physical footprint:"),
        parse("Physical footprint (peak):"),
    )
}

/// The world-space center of the page's top-level child with the most
/// descendants: a content-dense camera anchor.
fn dense_anchor(document: &FigDocument, page: NodeId) -> Option<[f64; 2]> {
    let scene = &document.doc.scene;
    scene
        .children_of(Some(page))
        .iter()
        .filter_map(|child| {
            let bounds = scene.world_bounds(*child)?;
            (bounds.is_finite() && bounds.width() > 0.0 && bounds.height() > 0.0)
                .then(|| (scene.descendants_of(*child).count(), bounds))
        })
        .max_by_key(|(count, _)| *count)
        .map(|(_, bounds)| {
            [
                bounds.min_x + bounds.width() * 0.5,
                bounds.min_y + bounds.height() * 0.5,
            ]
        })
}

fn pan_path(start: Viewport, ticks: usize, logical_per_tick: (f64, f64)) -> Vec<Viewport> {
    (1..=ticks)
        .map(|step| Viewport {
            center: [
                start.center[0] + logical_per_tick.0 * step as f64 / start.zoom,
                start.center[1] + logical_per_tick.1 * step as f64 / start.zoom,
            ],
            zoom: start.zoom,
        })
        .collect()
}

fn there_and_back(start: Viewport, ticks: usize, logical_per_tick: (f64, f64)) -> Vec<Viewport> {
    let out = pan_path(start, ticks, logical_per_tick);
    let back = out
        .iter()
        .rev()
        .skip(1)
        .copied()
        .chain(std::iter::once(start));
    out.iter().copied().chain(back).collect()
}

fn zoom_path(center: [f64; 2], from: f64, to: f64, ticks: usize) -> Vec<Viewport> {
    (1..=ticks)
        .map(|step| Viewport {
            center,
            zoom: from * (to / from).powf(step as f64 / ticks as f64),
        })
        .collect()
}

fn parse_size(value: &str) -> Option<(f64, f64)> {
    let (width, height) = value.split_once('x')?;
    Some((width.trim().parse().ok()?, height.trim().parse().ok()?))
}

#[test]
#[ignore = "benchmark: needs Metal and FANTA_CAMERA_BENCH_PROJECT; run explicitly with --ignored"]
fn navigation_benchmark() -> Result<()> {
    let Some(project) = std::env::var_os("FANTA_CAMERA_BENCH_PROJECT").map(PathBuf::from) else {
        eprintln!("FANTA_CAMERA_BENCH_PROJECT is unset; skipping the navigation benchmark");
        return Ok(());
    };
    anyhow::ensure!(
        project.join(".claude-benchmark-fixture").is_file(),
        "the benchmark only runs on a marked disposable project copy"
    );
    let output = std::env::var_os("FANTA_CAMERA_BENCH_OUT")
        .map(PathBuf::from)
        .context("FANTA_CAMERA_BENCH_OUT is unset")?;
    let captured: &'static CapturedLog = Box::leak(Box::new(CapturedLog {
        origin: Instant::now(),
        lines: Mutex::new(Vec::new()),
    }));
    let logging = log::set_logger(captured).is_ok();
    if logging {
        log::set_max_level(log::LevelFilter::Info);
    }
    let logical = std::env::var("FANTA_CAMERA_BENCH_SIZE")
        .ok()
        .and_then(|value| parse_size(&value))
        .unwrap_or((336.0, 986.0));
    let selected: Option<Vec<String>> = std::env::var("FANTA_CAMERA_BENCH_SCENARIOS")
        .ok()
        .map(|value| value.split(',').map(str::to_string).collect());
    let wants = |name: &str| {
        selected.as_ref().is_none_or(|names| {
            names
                .iter()
                .any(|selected| name.starts_with(selected.as_str()))
        })
    };

    let load_started = Instant::now();
    let document = load_project_document(&project)?;
    let load_ms = load_started.elapsed().as_secs_f64() * 1000.0;
    let dependency_compare_us = {
        // What CameraCache::render pays per frame to prove its retained
        // pixels still match the component library, variables and modes.
        let components = document.doc.components.clone();
        let variables = document.doc.variables.clone();
        let modes = std::hint::black_box(document.doc.active_modes.clone());
        let started = Instant::now();
        let mut equal = true;
        for _ in 0..100 {
            equal &= std::hint::black_box(&components) == &document.doc.components
                && std::hint::black_box(&variables) == &document.doc.variables
                && std::hint::black_box(&modes) == &document.doc.active_modes;
        }
        anyhow::ensure!(equal, "cloned dependencies compare equal");
        started.elapsed().as_secs_f64() * 1e6 / 100.0
    };
    eprintln!(
        "dependency comparison: {dependency_compare_us:.1} us per frame for {} component definitions, {} variables",
        document.doc.components.defs.len(),
        document.doc.variables.variables.len()
    );
    let families = document.used_font_families();
    let fetched = fanta_text::prewarm_font_downloads(families.iter().map(String::as_str));
    eprintln!("loaded in {load_ms:.0} ms; prewarmed fonts {fetched:?}");

    let page_id = std::env::var("FANTA_CAMERA_BENCH_PAGE")
        .unwrap_or_else(|_| "n_01M4V5T70HDF94V5REGZK2D19P".to_string());
    let page_index = document
        .pages
        .iter()
        .position(|page| page.root.is_some_and(|root| root.to_string() == page_id))
        .with_context(|| format!("page {page_id} is not in the project"))?;
    let other_page_id = std::env::var("FANTA_CAMERA_BENCH_OTHER_PAGE")
        .unwrap_or_else(|_| "n_01M4V5T70CQ5WYW74NF0PY6RF3".to_string());
    let other_index = document
        .pages
        .iter()
        .position(|page| {
            page.root
                .is_some_and(|root| root.to_string() == other_page_id)
        })
        .with_context(|| format!("page {other_page_id} is not in the project"))?;

    let mut bench = Bench::new(document, page_index, logical, 2.0, captured.origin)?;
    let page = bench.page_root.context("benchmark page")?;
    let anchor = dense_anchor(&bench.document, page).unwrap_or(bench.viewport.center);
    let fit = bench.viewport;
    eprintln!(
        "page {page_id}: fit {:?} anchor {anchor:?} render size {:?}",
        fit, bench.render_size
    );
    let mut scenarios = Vec::new();
    let mut cold = bench.scenario("cold_open");
    cold.warmup_ms = bench.settle(&mut cold.ticks, &mut cold.inputs, SETTLE_TIMEOUT)?;
    bench.snapshot("cold_open")?;
    scenarios.push(bench.finish(cold));

    for (name, zoom, speed) in [
        ("pan_z0.10", 0.1, 24.0),
        ("pan_z0.25", 0.25, 16.0),
        ("pan_z0.50", 0.5, 16.0),
        ("pan_z1.00", 1.0, 12.0),
    ] {
        if !wants(name) {
            continue;
        }
        let start = Viewport {
            center: anchor,
            zoom,
        };
        let path = there_and_back(start, 150, (speed, speed * 0.25));
        eprintln!("running {name}");
        scenarios.push(bench.run_motion(name, start, &path)?);
    }
    if wants("fling_uncovered") {
        let start = Viewport {
            center: anchor,
            zoom: 0.5,
        };
        let path = pan_path(start, 60, (0.0, 48.0));
        eprintln!("running fling_uncovered");
        scenarios.push(bench.run_motion("fling_uncovered", start, &path)?);
    }
    if wants("zoom_continuous") {
        let start = Viewport {
            center: anchor,
            zoom: 0.1,
        };
        let mut path = zoom_path(anchor, 0.1, 1.0, 120);
        path.extend(zoom_path(anchor, 1.0, 0.1, 120));
        eprintln!("running zoom_continuous");
        scenarios.push(bench.run_motion("zoom_continuous", start, &path)?);
    }
    if wants("page_cold") {
        eprintln!("running page_cold");
        scenarios.push(bench.run_page_switch("page_cold", other_index)?);
    }
    if wants("page_return") {
        eprintln!("running page_return");
        scenarios.push(bench.run_page_switch("page_return", page_index)?);
        scenarios.push(bench.run_page_switch("page_return_again", other_index)?);
        scenarios.push(bench.run_page_switch("page_return_back", page_index)?);
    }
    if wants("edit_undo") {
        let start = Viewport {
            center: anchor,
            zoom: 0.25,
        };
        bench.set_camera(start);
        let target = bench
            .document
            .doc
            .scene
            .children_of(Some(page))
            .iter()
            .copied()
            .filter_map(|child| {
                let bounds = bench.document.doc.scene.world_bounds(child)?;
                bounds
                    .contains_point(glam::DVec2::new(anchor[0], anchor[1]))
                    .then(|| {
                        (
                            child,
                            bench.document.doc.scene.descendants_of(child).count(),
                        )
                    })
            })
            .max_by_key(|(_, count)| *count)
            .map(|(child, _)| child)
            .context("no top-level node under the anchor")?;
        eprintln!("running edit_undo on {target}");
        scenarios.push(bench.run_edit_undo("edit_undo", target)?);
    }

    let log = if logging {
        captured
            .lines
            .lock()
            .map(|lines| lines.clone())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let record = BenchmarkRecord {
        project: project.display().to_string(),
        page: bench.page_label(page_index),
        element_logical: bench.logical,
        render_size: bench.render_size,
        scale_factor: bench.scale_factor,
        load_ms,
        log_offset_ms: 0.0,
        dependency_compare_us,
        scenarios,
        log,
    };
    std::fs::write(&output, serde_json::to_vec(&record)?)
        .with_context(|| format!("writing {}", output.display()))?;
    eprintln!("wrote {}", output.display());
    Ok(())
}
