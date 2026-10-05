#![cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
    time::{Duration, Instant},
};

use fanta_render::RenderMetrics;
use serde::Serialize;

pub(crate) type SharedGesturePerf = Rc<RefCell<GesturePerf>>;

const UI_STAGE_COUNT: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UiStage {
    ToolDispatch,
    ViewRender,
    DesignRefresh,
    ContentPreviewPostprocess,
}

impl UiStage {
    fn index(self) -> usize {
        match self {
            Self::ToolDispatch => 0,
            Self::ViewRender => 1,
            Self::DesignRefresh => 2,
            Self::ContentPreviewPostprocess => 3,
        }
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize)]
struct UiCost {
    calls: u64,
    inclusive_wall_total_us: u64,
    inclusive_wall_max_us: u64,
}

impl UiCost {
    fn record(&mut self, elapsed: Duration) {
        let micros = duration_micros(elapsed);
        self.calls = self.calls.saturating_add(1);
        self.inclusive_wall_total_us = self.inclusive_wall_total_us.saturating_add(micros);
        self.inclusive_wall_max_us = self.inclusive_wall_max_us.max(micros);
    }
}

#[derive(Serialize)]
struct UiCosts {
    membership: &'static str,
    timing: &'static str,
    tool_dispatch: UiCost,
    view_render: UiCost,
    design_refresh: UiCost,
    content_preview_postprocess: UiCost,
}

impl UiCosts {
    fn from_stages(stages: [UiCost; UI_STAGE_COUNT]) -> Self {
        Self {
            membership: "synchronous_scopes_started_in_this_gesture; includes_release_dispatch; excludes_initial_press_setup_and_post_release_settling",
            timing: "inclusive_wall_time; nested_stages_overlap_and_must_not_be_summed; not_cpu_time_or_input_latency; render_is_element_construction_not_layout_or_paint",
            tool_dispatch: stages[UiStage::ToolDispatch.index()],
            view_render: stages[UiStage::ViewRender.index()],
            design_refresh: stages[UiStage::DesignRefresh.index()],
            content_preview_postprocess: stages[UiStage::ContentPreviewPostprocess.index()],
        }
    }
}

#[derive(Clone)]
struct UiContext {
    perf: SharedGesturePerf,
    owner: u64,
    gesture: u64,
}

thread_local! {
    // Document postprocessing is synchronous and receives its view's owner ID,
    // so this avoids coupling document entities to a particular view's metrics.
    static CURRENT_UI_SCOPE: RefCell<Option<UiContext>> = const { RefCell::new(None) };
}

#[must_use]
pub(crate) struct UiSpan {
    context: Option<UiContext>,
    previous: Option<UiContext>,
    stage: UiStage,
    started: Option<Instant>,
}

impl UiSpan {
    fn start(context: Option<UiContext>, stage: UiStage, clock: impl FnOnce() -> Instant) -> Self {
        let context =
            context.filter(|context| context.perf.borrow_mut().reserve_ui_span(context.gesture));
        let started = optional_timer_with(context.is_some(), clock);
        // Even an inactive view masks its caller's scope, so its document work
        // cannot be charged to a different active view during a nested callback.
        let previous = CURRENT_UI_SCOPE.with(|current| current.replace(context.clone()));
        Self {
            context,
            previous,
            stage,
            started,
        }
    }
}

impl Drop for UiSpan {
    fn drop(&mut self) {
        drop(CURRENT_UI_SCOPE.with(|current| current.replace(self.previous.take())));
        if let Some(context) = self.context.take()
            && let Some(started) = self.started.take()
        {
            context.perf.borrow_mut().complete_ui_span(
                context.gesture,
                self.stage,
                started.elapsed(),
            );
        }
    }
}

pub(crate) fn ui_span(perf: Option<&SharedGesturePerf>, owner: u64, stage: UiStage) -> UiSpan {
    let context = perf.and_then(|perf| {
        let gesture = perf.borrow().active?;
        Some(UiContext {
            perf: Rc::clone(perf),
            owner,
            gesture,
        })
    });
    UiSpan::start(context, stage, Instant::now)
}

pub(crate) fn current_ui_span(owner: Option<u64>, stage: UiStage) -> UiSpan {
    let context = CURRENT_UI_SCOPE.with(|current| {
        current
            .borrow()
            .as_ref()
            .filter(|context| Some(context.owner) == owner)
            .cloned()
    });
    UiSpan::start(context, stage, Instant::now)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RenderSourceKind {
    Live,
    SnapshotFresh,
    SnapshotReuse,
    Patch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RequestTag {
    gesture: u64,
    request: u64,
    source: RenderSourceKind,
}

#[derive(Serialize)]
pub(crate) struct GestureContext {
    pub view_id: String,
    pub page_root: Option<String>,
    pub tool: String,
    pub selected_nodes: Vec<String>,
    pub viewport_zoom: Option<f64>,
    pub logical_canvas_size: Option<(f64, f64)>,
    pub display_scale: f32,
}

#[derive(Default, Serialize)]
struct MetricSums {
    nodes_visited: u64,
    nodes_drawn: u64,
    nodes_culled: u64,
    cached_node_paths_built: u64,
    instance_indexes_built: u64,
    effect_layers: u64,
    opacity_folds: u64,
    layer_cache_hits: u64,
    layer_cache_misses: u64,
    incomplete_artwork_frames: u64,
    non_artwork_content_frames: u64,
    effect_failed_frames: u64,
}

impl MetricSums {
    fn add(&mut self, metrics: &RenderMetrics) {
        self.nodes_visited += u64::from(metrics.nodes_visited);
        self.nodes_drawn += u64::from(metrics.nodes_drawn);
        self.nodes_culled += u64::from(metrics.nodes_culled);
        // RenderMetrics.paths_built omits uncached expanded/overlay vectors.
        self.cached_node_paths_built += u64::from(metrics.paths_built);
        self.instance_indexes_built += u64::from(metrics.instance_indexes_built);
        self.effect_layers += u64::from(metrics.effect_layers);
        self.opacity_folds += u64::from(metrics.opacity_folds);
        self.layer_cache_hits += u64::from(metrics.layer_cache_hits);
        self.layer_cache_misses += u64::from(metrics.layer_cache_misses);
        self.incomplete_artwork_frames += u64::from(metrics.incomplete_artwork);
        self.non_artwork_content_frames += u64::from(metrics.non_artwork_content);
        self.effect_failed_frames += u64::from(metrics.effect_failed);
    }
}

pub(crate) struct FrameSample {
    pub metrics: RenderMetrics,
    pub flush_sync_wall_us: u64,
    pub render_wall_us: u64,
    pub physical_size: (u32, u32),
    pub viewport_zoom: f64,
    pub display_scale: f32,
}

#[derive(Serialize)]
struct CompletedSample {
    request_id: u64,
    source: RenderSourceKind,
    scene_walk_wall_us: u64,
    flush_sync_wall_us: u64,
    render_wall_us: u64,
    physical_size: (u32, u32),
    viewport_zoom: f64,
    display_scale: f32,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct Distribution {
    count: usize,
    p50: Option<u64>,
    p95: Option<u64>,
    max: Option<u64>,
    total: u64,
}

impl Distribution {
    fn from_values(values: impl Iterator<Item = u64>) -> Self {
        let mut values: Vec<_> = values.collect();
        values.sort_unstable();
        let rank = |percent: usize| {
            let index = (values.len() * percent).div_ceil(100).checked_sub(1)?;
            values.get(index).copied()
        };
        Self {
            count: values.len(),
            p50: rank(50),
            p95: rank(95),
            max: values.last().copied(),
            total: values.iter().sum(),
        }
    }
}

#[derive(Serialize)]
struct GestureSummary {
    version: u32,
    gesture_id: u64,
    context: GestureContext,
    backend: &'static str,
    membership: &'static str,
    end_reason: &'static str,
    pointer_moves: u64,
    content_preview_moves: u64,
    submitted: u64,
    completed: usize,
    failed: u64,
    incomplete: u64,
    dispatch_errors: u64,
    cpu_fallback_paints: u64,
    present_reuse_paints: u64,
    reproject_paints: u64,
    source_kind_counts: BTreeMap<RenderSourceKind, u64>,
    failure_counts: BTreeMap<&'static str, u64>,
    scene_walk_wall_us: Distribution,
    flush_sync_wall_us: Distribution,
    render_wall_us: Distribution,
    metric_sums: MetricSums,
    ui_costs: UiCosts,
    frames: Vec<CompletedSample>,
}

struct GestureRecord {
    context: GestureContext,
    end_reason: Option<&'static str>,
    pointer_moves: u64,
    content_preview_moves: u64,
    submitted: u64,
    failed: u64,
    incomplete: u64,
    dispatch_errors: u64,
    cpu_fallback_paints: u64,
    present_reuse_paints: u64,
    reproject_paints: u64,
    pending: BTreeSet<u64>,
    source_kind_counts: BTreeMap<RenderSourceKind, u64>,
    failure_counts: BTreeMap<&'static str, u64>,
    metric_sums: MetricSums,
    frames: Vec<CompletedSample>,
    ui_costs: [UiCost; UI_STAGE_COUNT],
    pending_ui_spans: usize,
}

impl GestureRecord {
    fn new(context: GestureContext) -> Self {
        Self {
            context,
            end_reason: None,
            pointer_moves: 0,
            content_preview_moves: 0,
            submitted: 0,
            failed: 0,
            incomplete: 0,
            dispatch_errors: 0,
            cpu_fallback_paints: 0,
            present_reuse_paints: 0,
            reproject_paints: 0,
            pending: BTreeSet::new(),
            source_kind_counts: BTreeMap::new(),
            failure_counts: BTreeMap::new(),
            metric_sums: MetricSums::default(),
            frames: Vec::new(),
            ui_costs: [UiCost::default(); UI_STAGE_COUNT],
            pending_ui_spans: 0,
        }
    }

    fn summary(self, gesture_id: u64, end_reason: &'static str) -> GestureSummary {
        GestureSummary {
            version: 2,
            gesture_id,
            context: self.context,
            backend: match (self.frames.is_empty(), self.cpu_fallback_paints > 0) {
                (true, true) => "cpu_fallback_unmeasured",
                (true, false) => "no_completed_native_frames",
                (false, true) => "skia_metal_worker_with_cpu_fallback",
                (false, false) => "skia_metal_worker",
            },
            membership: "requests_dispatched_while_primary_tool_pressed; excludes_post_release_settling",
            end_reason,
            pointer_moves: self.pointer_moves,
            content_preview_moves: self.content_preview_moves,
            submitted: self.submitted,
            completed: self.frames.len(),
            failed: self.failed,
            incomplete: self.incomplete,
            dispatch_errors: self.dispatch_errors,
            cpu_fallback_paints: self.cpu_fallback_paints,
            present_reuse_paints: self.present_reuse_paints,
            reproject_paints: self.reproject_paints,
            source_kind_counts: self.source_kind_counts,
            failure_counts: self.failure_counts,
            scene_walk_wall_us: Distribution::from_values(
                self.frames.iter().map(|frame| frame.scene_walk_wall_us),
            ),
            flush_sync_wall_us: Distribution::from_values(
                self.frames.iter().map(|frame| frame.flush_sync_wall_us),
            ),
            render_wall_us: Distribution::from_values(
                self.frames.iter().map(|frame| frame.render_wall_us),
            ),
            metric_sums: self.metric_sums,
            ui_costs: UiCosts::from_stages(self.ui_costs),
            frames: self.frames,
        }
    }
}

#[derive(Default)]
pub(crate) struct GesturePerf {
    next_gesture: u64,
    next_request: u64,
    active: Option<u64>,
    records: BTreeMap<u64, GestureRecord>,
    #[cfg(test)]
    summaries: Vec<GestureSummary>,
}

impl GesturePerf {
    pub fn shared() -> SharedGesturePerf {
        Rc::new(RefCell::new(Self::default()))
    }

    pub fn begin(&mut self, context: GestureContext) {
        self.end("superseded_by_press");
        self.next_gesture += 1;
        self.active = Some(self.next_gesture);
        self.records
            .insert(self.next_gesture, GestureRecord::new(context));
    }

    pub fn end(&mut self, reason: &'static str) {
        let Some(gesture) = self.active.take() else {
            return;
        };
        if let Some(record) = self.records.get_mut(&gesture) {
            record.end_reason = Some(reason);
        }
        self.finish_ready(gesture);
    }

    fn reserve_ui_span(&mut self, gesture: u64) -> bool {
        let Some(record) = self.records.get_mut(&gesture) else {
            return false;
        };
        record.pending_ui_spans += 1;
        true
    }

    fn complete_ui_span(&mut self, gesture: u64, stage: UiStage, elapsed: Duration) {
        let Some(record) = self.records.get_mut(&gesture) else {
            return;
        };
        record.ui_costs[stage.index()].record(elapsed);
        record.pending_ui_spans = record.pending_ui_spans.saturating_sub(1);
        self.finish_ready(gesture);
    }

    fn active_record(&mut self) -> Option<&mut GestureRecord> {
        self.records.get_mut(&self.active?)
    }

    pub fn pointer_move(&mut self) {
        if let Some(record) = self.active_record() {
            record.pointer_moves += 1;
        }
    }

    pub fn content_preview_move(&mut self) {
        if let Some(record) = self.active_record() {
            record.content_preview_moves += 1;
        }
    }

    pub fn cpu_fallback_paint(&mut self) {
        if let Some(record) = self.active_record() {
            record.cpu_fallback_paints += 1;
        }
    }

    pub fn reused_paint(&mut self, reprojected: bool) {
        if let Some(record) = self.active_record() {
            if reprojected {
                record.reproject_paints += 1;
            } else {
                record.present_reuse_paints += 1;
            }
        }
    }

    pub fn reserve(&mut self, source: RenderSourceKind) -> Option<RequestTag> {
        let gesture = self.active?;
        self.next_request += 1;
        Some(RequestTag {
            gesture,
            request: self.next_request,
            source,
        })
    }

    pub fn submitted(&mut self, tag: RequestTag) {
        if let Some(record) = self.records.get_mut(&tag.gesture)
            && record.pending.insert(tag.request)
        {
            record.submitted += 1;
            *record.source_kind_counts.entry(tag.source).or_default() += 1;
        }
    }

    pub fn complete(&mut self, tag: RequestTag, sample: Result<FrameSample, &'static str>) -> bool {
        let Some(record) = self.records.get_mut(&tag.gesture) else {
            return false;
        };
        if !record.pending.remove(&tag.request) {
            return false;
        }
        match sample {
            Ok(sample) => {
                record.metric_sums.add(&sample.metrics);
                record.frames.push(CompletedSample {
                    request_id: tag.request,
                    source: tag.source,
                    scene_walk_wall_us: sample.metrics.frame_micros,
                    flush_sync_wall_us: sample.flush_sync_wall_us,
                    render_wall_us: sample.render_wall_us,
                    physical_size: sample.physical_size,
                    viewport_zoom: sample.viewport_zoom,
                    display_scale: sample.display_scale,
                });
            }
            Err(reason) => {
                record.failed += 1;
                *record.failure_counts.entry(reason).or_default() += 1;
            }
        }
        self.finish_ready(tag.gesture);
        true
    }

    pub fn dispatch_failed(&mut self, reason: &'static str) {
        if let Some(record) = self.active_record() {
            record.dispatch_errors += 1;
            *record.failure_counts.entry(reason).or_default() += 1;
        }
    }

    pub fn fail_pending(&mut self, reason: &'static str) {
        let gestures: Vec<_> = self.records.keys().copied().collect();
        for gesture in gestures {
            if let Some(record) = self.records.get_mut(&gesture) {
                let count = record.pending.len() as u64;
                record.failed += count;
                record.incomplete += count;
                if count > 0 {
                    *record.failure_counts.entry(reason).or_default() += count;
                }
                record.pending.clear();
            }
            self.finish_ready(gesture);
        }
    }

    fn finish_ready(&mut self, gesture: u64) {
        let reason = self.records.get(&gesture).and_then(|record| {
            (record.pending.is_empty() && record.pending_ui_spans == 0)
                .then_some(record.end_reason)
                .flatten()
        });
        let Some(reason) = reason else { return };
        let Some(record) = self.records.remove(&gesture) else {
            return;
        };
        let summary = record.summary(gesture, reason);
        match serde_json::to_string(&summary) {
            Ok(json) => log::info!("fanta_gesture_perf {json}"),
            Err(error) => log::warn!("serializing canvas gesture diagnostics failed: {error}"),
        }
        #[cfg(test)]
        self.summaries.push(summary);
    }

    #[cfg(test)]
    pub fn summary_count(&self) -> usize {
        self.summaries.len()
    }

    #[cfg(test)]
    pub fn last_ui_dispatch_calls(&self) -> Option<u64> {
        self.summaries
            .last()
            .map(|summary| summary.ui_costs.tool_dispatch.calls)
    }

    #[cfg(test)]
    pub fn has_active_gesture(&self) -> bool {
        self.active.is_some()
    }
}

impl Drop for GesturePerf {
    fn drop(&mut self) {
        self.end("view_dropped");
        self.fail_pending("view_dropped");
    }
}

pub(crate) fn optional_timer(enabled: bool) -> Option<Instant> {
    optional_timer_with(enabled, Instant::now)
}

fn optional_timer_with<T>(enabled: bool, clock: impl FnOnce() -> T) -> Option<T> {
    enabled.then(clock)
}

pub(crate) fn duration_micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> GestureContext {
        GestureContext {
            view_id: "test".into(),
            page_root: Some("page".into()),
            tool: "Select".into(),
            selected_nodes: vec!["instance".into()],
            viewport_zoom: Some(0.27),
            logical_canvas_size: Some((800.0, 600.0)),
            display_scale: 2.0,
        }
    }

    fn sample(walk: u64) -> FrameSample {
        let mut metrics = RenderMetrics::default();
        metrics.frame_micros = walk;
        metrics.nodes_visited = 7;
        metrics.layer_cache_hits = 3;
        metrics.layer_cache_misses = 2;
        FrameSample {
            metrics,
            flush_sync_wall_us: 4,
            render_wall_us: walk + 6,
            physical_size: (1600, 1200),
            viewport_zoom: 0.27,
            display_scale: 2.0,
        }
    }

    #[test]
    fn gesture_perf_distributions_use_nearest_rank_and_null_empty_values() {
        assert_eq!(
            Distribution::from_values(std::iter::empty()),
            Distribution {
                count: 0,
                p50: None,
                p95: None,
                max: None,
                total: 0,
            }
        );
        let distribution = Distribution::from_values((1..=20).rev());
        assert_eq!(
            distribution,
            Distribution {
                count: 20,
                p50: Some(10),
                p95: Some(19),
                max: Some(20),
                total: 210
            }
        );
        assert_eq!(Distribution::from_values([12].into_iter()).p95, Some(12));
    }

    #[test]
    fn gesture_perf_late_completion_stays_with_original_gesture() {
        let mut perf = GesturePerf::default();
        perf.begin(context());
        let first = perf.reserve(RenderSourceKind::Patch).expect("first tag");
        perf.submitted(first);
        perf.pointer_move();
        perf.content_preview_move();
        perf.end("released");
        assert!(perf.summaries.is_empty());
        perf.begin(context());
        let second = perf
            .reserve(RenderSourceKind::SnapshotFresh)
            .expect("second tag");
        perf.submitted(second);
        assert!(perf.complete(first, Ok(sample(10))));
        assert!(!perf.complete(first, Ok(sample(900))));
        assert_eq!(perf.summaries.len(), 1);
        assert!(perf.has_active_gesture());
        assert!(perf.complete(second, Ok(sample(20))));
        assert_eq!(perf.summaries.len(), 1);
        perf.end("released");
        assert_eq!(perf.summaries.len(), 2);
        let first = &perf.summaries[0];
        assert_eq!(
            (
                first.gesture_id,
                first.pointer_moves,
                first.content_preview_moves
            ),
            (1, 1, 1)
        );
        assert_eq!(first.scene_walk_wall_us.p95, Some(10));
        assert_eq!(first.metric_sums.nodes_visited, 7);
        assert_eq!(first.metric_sums.layer_cache_hits, 3);
        assert_eq!(first.source_kind_counts[&RenderSourceKind::Patch], 1);
        assert_eq!(perf.summaries[1].scene_walk_wall_us.p95, Some(20));
    }

    #[test]
    fn gesture_perf_reuse_and_cpu_fallback_do_not_invent_native_samples() {
        let mut perf = GesturePerf::default();
        perf.begin(context());
        perf.reused_paint(false);
        perf.reused_paint(true);
        perf.cpu_fallback_paint();
        perf.end("released");
        perf.end("released");
        assert_eq!(perf.summaries.len(), 1);
        let summary = &perf.summaries[0];
        assert_eq!((summary.submitted, summary.completed), (0, 0));
        assert_eq!(summary.render_wall_us.p95, None);
        assert_eq!(summary.backend, "cpu_fallback_unmeasured");
        assert_eq!(
            (
                summary.present_reuse_paints,
                summary.reproject_paints,
                summary.cpu_fallback_paints
            ),
            (1, 1, 1)
        );
    }

    #[test]
    fn gesture_perf_failed_and_lost_replies_finish_once_without_samples() {
        for reason in ["render_error", "snapshot_lost"] {
            let mut perf = GesturePerf::default();
            perf.begin(context());
            let tag = perf.reserve(RenderSourceKind::Live).expect("tag");
            perf.submitted(tag);
            perf.end("released");
            assert!(perf.complete(tag, Err(reason)));
            assert!(!perf.complete(tag, Err(reason)));
            let summary = &perf.summaries[0];
            assert_eq!(
                (
                    summary.submitted,
                    summary.failed,
                    summary.completed,
                    summary.incomplete
                ),
                (1, 1, 0, 0)
            );
            assert_eq!(summary.failure_counts[reason], 1);
        }
    }

    #[test]
    fn gesture_perf_disconnect_marks_missing_reply_incomplete() {
        let mut perf = GesturePerf::default();
        perf.begin(context());
        let tag = perf.reserve(RenderSourceKind::SnapshotReuse).expect("tag");
        perf.submitted(tag);
        perf.end("cancelled");
        perf.fail_pending("worker_disconnected");
        perf.fail_pending("worker_disconnected");
        assert_eq!(perf.summaries.len(), 1);
        let summary = &perf.summaries[0];
        assert_eq!(
            (
                summary.submitted,
                summary.failed,
                summary.completed,
                summary.incomplete
            ),
            (1, 1, 0, 1)
        );
        assert_eq!(summary.end_reason, "cancelled");
    }

    #[test]
    fn gesture_perf_unsent_request_does_not_count_as_a_submitted_frame() {
        let mut perf = GesturePerf::default();
        perf.begin(context());
        let unsent = perf.reserve(RenderSourceKind::Patch).expect("reserved tag");
        perf.dispatch_failed("worker_send_failed");
        perf.fail_pending("worker_send_failed");
        assert!(!perf.complete(unsent, Ok(sample(10))));
        perf.end("released");
        let summary = &perf.summaries[0];
        assert_eq!(
            (summary.submitted, summary.completed, summary.failed),
            (0, 0, 0)
        );
        assert_eq!(summary.dispatch_errors, 1);
        assert_eq!(summary.failure_counts["worker_send_failed"], 1);
        assert!(summary.source_kind_counts.is_empty());
        assert_eq!(summary.render_wall_us.p95, None);
    }

    #[test]
    fn gesture_perf_active_canvas_drop_finishes_after_release_without_duplicate_failure() {
        let mut perf = GesturePerf::default();
        perf.begin(context());
        let pending = perf
            .reserve(RenderSourceKind::SnapshotFresh)
            .expect("reserved tag");
        perf.submitted(pending);
        perf.fail_pending("gpu_canvas_dropped");
        perf.fail_pending("gpu_canvas_dropped");
        assert!(perf.summaries.is_empty());
        assert!(!perf.complete(pending, Ok(sample(10))));
        perf.end("released");
        let summary = &perf.summaries[0];
        assert_eq!(
            (summary.submitted, summary.failed, summary.incomplete),
            (1, 1, 1)
        );
        assert_eq!(summary.completed, 0);
        assert_eq!(summary.failure_counts["gpu_canvas_dropped"], 1);
        assert_eq!(summary.render_wall_us.p95, None);
    }

    #[test]
    fn gesture_perf_disabled_timer_does_not_read_clock() {
        assert_eq!(
            optional_timer_with(false, || panic!("clock must not run")),
            None::<usize>
        );
        assert_eq!(optional_timer_with(true, || 42), Some(42));
    }

    #[test]
    fn gesture_perf_ui_costs_are_exact_fixed_size_inclusive_aggregates() {
        let mut record = GestureRecord::new(context());
        for micros in 1..=10_000 {
            record.ui_costs[UiStage::ToolDispatch.index()].record(Duration::from_micros(micros));
        }
        record.ui_costs[UiStage::ContentPreviewPostprocess.index()]
            .record(Duration::from_micros(17));
        let summary = record.summary(1, "released");
        assert_eq!(
            summary.ui_costs.tool_dispatch,
            UiCost {
                calls: 10_000,
                inclusive_wall_total_us: 50_005_000,
                inclusive_wall_max_us: 10_000,
            }
        );
        assert_eq!(summary.ui_costs.content_preview_postprocess.calls, 1);
        assert_eq!(
            summary
                .ui_costs
                .content_preview_postprocess
                .inclusive_wall_total_us,
            17
        );
        assert_eq!(summary.ui_costs.view_render, UiCost::default());
        assert!(summary.ui_costs.timing.contains("must_not_be_summed"));
        assert_eq!(summary.version, 2);
    }

    #[test]
    fn gesture_perf_ui_disabled_and_stale_scopes_never_start_a_clock() {
        let disabled = UiSpan::start(None, UiStage::ToolDispatch, || panic!("disabled clock"));
        assert!(disabled.started.is_none());
        drop(disabled);
        let perf = GesturePerf::shared();
        let inactive = ui_span(Some(&perf), 11, UiStage::ViewRender);
        assert!(inactive.started.is_none());
        drop(inactive);
        let stale = UiSpan::start(
            Some(UiContext {
                perf: Rc::clone(&perf),
                owner: 11,
                gesture: 42,
            }),
            UiStage::ToolDispatch,
            || panic!("stale clock"),
        );
        assert!(stale.started.is_none());
        drop(stale);
        assert_eq!(Rc::strong_count(&perf), 1);
        assert!(CURRENT_UI_SCOPE.with(|current| current.borrow().is_none()));
    }

    #[test]
    fn gesture_perf_ui_release_waits_for_its_span_and_worker_reply() {
        let perf = GesturePerf::shared();
        perf.borrow_mut().begin(context());
        let tag = perf
            .borrow_mut()
            .reserve(RenderSourceKind::Patch)
            .expect("request");
        perf.borrow_mut().submitted(tag);
        let release = ui_span(Some(&perf), 11, UiStage::ToolDispatch);
        perf.borrow_mut().end("released");
        assert_eq!(perf.borrow().summary_count(), 0);
        assert!(perf.borrow_mut().complete(tag, Ok(sample(12))));
        assert_eq!(
            perf.borrow().summary_count(),
            0,
            "release callback is still running"
        );
        drop(release);
        let perf = perf.borrow();
        assert_eq!(perf.summary_count(), 1);
        assert_eq!(perf.summaries[0].ui_costs.tool_dispatch.calls, 1);
        assert_eq!(perf.summaries[0].completed, 1);
        assert!(
            perf.summaries[0]
                .ui_costs
                .membership
                .contains("includes_release_dispatch")
        );
    }

    #[test]
    fn gesture_perf_ui_late_span_stays_with_its_original_gesture() {
        let perf = GesturePerf::shared();
        perf.borrow_mut().begin(context());
        let first = ui_span(Some(&perf), 11, UiStage::ToolDispatch);
        perf.borrow_mut().end("released");
        perf.borrow_mut().begin(context());
        {
            let _second = ui_span(Some(&perf), 11, UiStage::ViewRender);
            let _nested = current_ui_span(Some(11), UiStage::ContentPreviewPostprocess);
        }
        drop(first);
        perf.borrow_mut().end("released");
        let perf = perf.borrow();
        assert_eq!(perf.summaries.len(), 2);
        assert_eq!(perf.summaries[0].ui_costs.tool_dispatch.calls, 1);
        assert_eq!(
            perf.summaries[0].ui_costs.content_preview_postprocess.calls,
            0
        );
        assert_eq!(perf.summaries[1].ui_costs.view_render.calls, 1);
        assert_eq!(
            perf.summaries[1].ui_costs.content_preview_postprocess.calls,
            1
        );
        assert_eq!(perf.summaries[1].ui_costs.tool_dispatch.calls, 0);
    }

    #[test]
    fn gesture_perf_ui_nested_views_and_inactive_views_cannot_mix_costs() {
        let first = GesturePerf::shared();
        let second = GesturePerf::shared();
        first.borrow_mut().begin(context());
        second.borrow_mut().begin(context());
        {
            let _outer = ui_span(Some(&first), 11, UiStage::ToolDispatch);
            {
                let _own = current_ui_span(Some(11), UiStage::ContentPreviewPostprocess);
            }
            {
                let _other_view = ui_span(Some(&second), 22, UiStage::DesignRefresh);
                let wrong_owner = current_ui_span(Some(11), UiStage::ContentPreviewPostprocess);
                assert!(wrong_owner.started.is_none());
                drop(wrong_owner);
                let _own = current_ui_span(Some(22), UiStage::ContentPreviewPostprocess);
            }
            {
                let _inactive_view = ui_span(None, 33, UiStage::ViewRender);
                let masked = current_ui_span(Some(11), UiStage::ContentPreviewPostprocess);
                assert!(masked.started.is_none());
            }
            let _restored = current_ui_span(Some(11), UiStage::ContentPreviewPostprocess);
        }
        first.borrow_mut().end("released");
        second.borrow_mut().end("released");
        assert_eq!(
            first.borrow().summaries[0]
                .ui_costs
                .content_preview_postprocess
                .calls,
            2
        );
        assert_eq!(first.borrow().summaries[0].ui_costs.design_refresh.calls, 0);
        assert_eq!(
            second.borrow().summaries[0]
                .ui_costs
                .content_preview_postprocess
                .calls,
            1
        );
        assert_eq!(second.borrow().summaries[0].ui_costs.tool_dispatch.calls, 0);
        assert!(CURRENT_UI_SCOPE.with(|current| current.borrow().is_none()));
    }

    #[test]
    fn gesture_perf_ui_early_return_restores_the_callers_scope() {
        fn stop_early(perf: &SharedGesturePerf) -> Option<()> {
            let _span = ui_span(Some(perf), 22, UiStage::DesignRefresh);
            let missing: Option<()> = None;
            missing?;
            Some(())
        }
        let first = GesturePerf::shared();
        let second = GesturePerf::shared();
        first.borrow_mut().begin(context());
        second.borrow_mut().begin(context());
        {
            let _outer = ui_span(Some(&first), 11, UiStage::ToolDispatch);
            assert_eq!(stop_early(&second), None);
            let _restored = current_ui_span(Some(11), UiStage::ContentPreviewPostprocess);
        }
        first.borrow_mut().end("released");
        second.borrow_mut().end("released");
        assert_eq!(
            first.borrow().summaries[0]
                .ui_costs
                .content_preview_postprocess
                .calls,
            1
        );
        assert_eq!(
            second.borrow().summaries[0].ui_costs.design_refresh.calls,
            1
        );
        assert!(CURRENT_UI_SCOPE.with(|current| current.borrow().is_none()));
    }
}
