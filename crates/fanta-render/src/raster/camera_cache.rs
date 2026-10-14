use super::layer_cache::immutable_surface_snapshot;
use super::{
    Bounds, Canvas, Color, ComponentLibrary, ImageInfo, NodeId, RasterRenderer, RenderInputs,
    RenderMetrics, Scene, VariableRegistry, Viewport, to_sk_color,
};
use fanta_doc::{ModeId, VariableCollectionId};
use skia_safe::canvas::SrcRectConstraint;
use skia_safe::{AlphaType, BlendMode, Image, Paint, Picture, Rect, SamplingOptions};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

const TILE_SIZE: i32 = 512;
const GUTTER: i32 = 4;
const IMAGE_BUDGET: usize = 96 * 1024 * 1024;
const PICTURE_BUDGET: usize = 64 * 1024 * 1024;
/// Missing visible tiles a frame at an already-presented scale builds instead
/// of replaying its whole recording. A larger gap (a jump, the first pan after
/// a zoom) replays once and fills its tiles over the following frames.
pub(super) const MANDATORY_TILES: usize = 6;
/// CPU time a visible frame may spend building tiles beyond the mandatory
/// ones: the rest of a large gap first, then tiles ahead of the motion.
const TILE_BUDGET: Duration = Duration::from_millis(6);
/// Tiles a moving frame builds ahead of the motion, so a steady pan finds its
/// next tiles ready instead of building them when they become visible.
pub(super) const PREFETCH_TILES: usize = 2;
/// Extra tiles a region recording reaches in the direction of motion, so a
/// steady pan records new regions less often.
const LEAD_TILES: f32 = 2.0;

#[derive(Debug, Clone, Default)]
pub struct CameraCacheMetrics {
    pub overview_drawn: bool,
    pub picture_drawn: bool,
    pub tiles_drawn: u32,
    pub tiles_built: u32,
    pub scene_walks: u32,
    pub scene_walk_wall_us: u64,
    pub retained_bytes: usize,
}

#[derive(Debug, Clone)]
pub struct CameraCacheResult {
    pub metrics: RenderMetrics,
    pub cache: CameraCacheMetrics,
    pub needs_refresh: bool,
    pub complete_detail: bool,
    pub cancelled: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CacheDomain {
    Raster,
    #[cfg(feature = "metal")]
    Ganesh(skia_safe::gpu::DirectContextId),
    #[cfg(feature = "metal")]
    Graphite(usize),
}

#[derive(Clone, PartialEq)]
struct ContentKey {
    scene: (u64, u64),
    generation: u64,
    page: Option<NodeId>,
    mode_generation: u64,
    dark_ui: bool,
    font_generation: Option<u64>,
    background: Color,
    domain: CacheDomain,
    color_info: skia_safe::ColorInfo,
    properties: skia_safe::SurfaceProps,
}

struct Tile {
    image: Image,
    last_used: u64,
}

/// A display list of one device-space region at the level's scale.
struct RegionRecording {
    bounds: Rect,
    picture: Picture,
    last_used: u64,
}

struct DetailLevel {
    scale: f64,
    /// Recordings of the regions this scale has shown or prefetched: the
    /// first view, then each gap a pan exposed, recorded once.
    recordings: Vec<RegionRecording>,
    tiles: BTreeMap<(i32, i32), Tile>,
    /// Tiles whose own recording was refused (backdrop sampling) or whose
    /// image could not be allocated: frames showing them render normally.
    refused_tiles: BTreeSet<(i32, i32)>,
    /// The device translation of the last frame presented at this scale:
    /// `None` until one was, and the source of the motion direction.
    last_transform: Option<(f64, f64)>,
}

impl DetailLevel {
    fn new(scale: f64) -> Self {
        Self {
            scale,
            recordings: Vec::new(),
            tiles: BTreeMap::new(),
            refused_tiles: BTreeSet::new(),
            last_transform: None,
        }
    }

    fn records(&self, rect: Rect) -> bool {
        self.recording_for(rect).is_some()
    }

    /// The most recent recording that contains `rect`.
    fn recording_for(&self, rect: Rect) -> Option<&RegionRecording> {
        self.recordings
            .iter()
            .rev()
            .find(|recording| contains_rect(recording.bounds, rect))
    }

    fn touch_recording(&mut self, rect: Rect, frame: u64) -> Option<Picture> {
        let recording = self
            .recordings
            .iter_mut()
            .rev()
            .find(|recording| contains_rect(recording.bounds, rect))?;
        recording.last_used = frame;
        Some(recording.picture.clone())
    }

    fn recording_bytes(&self) -> usize {
        self.recordings
            .iter()
            .map(|recording| recording.picture.approximate_bytes_used())
            .sum()
    }
}

enum RegionRecord {
    Recorded,
    /// The recording device cannot certify the region (backdrop sampling).
    Unsupported(Box<RenderMetrics>),
    /// Recorded, but too large to retain or recorded across a font change.
    Unretainable(Box<RenderMetrics>),
    Cancelled(Box<RenderMetrics>),
}

enum TileBuild {
    Built,
    Refused,
    /// No tile could be evicted to make room.
    Full,
    Cancelled(Box<RenderMetrics>),
}

struct ParkedPage {
    key: ContentKey,
    level: Option<DetailLevel>,
    refused: Option<Rect>,
}

#[derive(Default)]
pub(super) struct CameraCache {
    key: Option<ContentKey>,
    components: ComponentLibrary,
    variables: VariableRegistry,
    active_modes: BTreeMap<VariableCollectionId, ModeId>,
    level: Option<DetailLevel>,
    parked: Option<ParkedPage>,
    frame: u64,
    domain_witness: Option<skia_safe::Surface>,
    refused: Option<Rect>,
}

impl CameraCache {
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render(
        &mut self,
        renderer: &mut RasterRenderer,
        canvas: &Canvas,
        width: u32,
        height: u32,
        scene: &Scene,
        viewport: &Viewport,
        page: Option<NodeId>,
        inputs: &RenderInputs,
        generation: u64,
        _interacting: bool,
        should_cancel: &dyn Fn() -> bool,
    ) -> CameraCacheResult {
        let started = Instant::now();
        if should_cancel() {
            return cancelled_result(RenderMetrics::default(), CameraCacheMetrics::default());
        }
        let scale = f64::from((viewport.zoom * renderer.display_scale) as f32);
        let Some(domain) = cache_domain(canvas) else {
            return fallback_cancellable(
                renderer,
                canvas,
                width,
                height,
                scene,
                viewport,
                page,
                inputs,
                should_cancel,
            );
        };
        if width == 0
            || height == 0
            || width > i32::MAX as u32
            || height > i32::MAX as u32
            || !scale.is_finite()
            || scale <= 0.0
            || canvas.image_info().bytes_per_pixel() != 4
            || !renderer.pixel_snap_pan()
            || !viewport.center.iter().all(|value| value.is_finite())
            || !canvas.local_to_device_as_3x3().is_identity()
            || inputs.motion.is_some()
            || inputs.playback.is_some_and(|values| !values.is_empty())
            || inputs
                .video_fill_frames
                .is_some_and(|values| !values.is_empty())
        {
            return fallback_cancellable(
                renderer,
                canvas,
                width,
                height,
                scene,
                viewport,
                page,
                inputs,
                should_cancel,
            );
        }
        let witness_matches = self
            .domain_witness
            .as_mut()
            .is_some_and(|surface| cache_domain(surface.canvas()) == Some(domain));
        if !witness_matches {
            self.clear();
            let Some(surface) = compatible_surface(canvas, 1, 1) else {
                return fallback_cancellable(
                    renderer,
                    canvas,
                    width,
                    height,
                    scene,
                    viewport,
                    page,
                    inputs,
                    should_cancel,
                );
            };
            // Recorder destruction abandons this live device, preventing address reuse
            // from making images belonging to an old GPU context eligible again.
            self.domain_witness = Some(surface);
        }
        let background = renderer.camera_background(scene, page, inputs);
        let key = ContentKey {
            scene: (scene.instance_id(), scene.revision()),
            generation,
            page,
            mode_generation: inputs.mode_generation,
            dark_ui: inputs.dark_ui,
            font_generation: RasterRenderer::current_font_generation(),
            background,
            domain,
            color_info: canvas.image_info().color_info().clone(),
            properties: canvas.top_props(),
        };
        let dependencies_changed = self.components != *inputs.components
            || self.variables != *inputs.variables
            || self.active_modes != *inputs.active_modes;
        if dependencies_changed {
            renderer.clear_camera_dependencies();
            self.key = None;
            self.level = None;
            self.parked = None;
            self.refused = None;
            if self.components != *inputs.components {
                self.components.clone_from(inputs.components);
            }
            if self.variables != *inputs.variables {
                self.variables.clone_from(inputs.variables);
            }
            if self.active_modes != *inputs.active_modes {
                self.active_modes.clone_from(inputs.active_modes);
            }
        }
        if self.key.as_ref() != Some(&key) {
            renderer.clear_camera_dependencies();
            let previous = self.key.take().map(|previous| ParkedPage {
                key: previous,
                level: self.level.take(),
                refused: self.refused.take(),
            });
            let reusable = self.parked.as_ref().is_some_and(|parked| parked.key == key);
            if reusable {
                if let Some(parked) = self.parked.take() {
                    self.level = parked.level;
                    self.refused = parked.refused;
                }
            } else {
                self.level = None;
                self.refused = None;
            }
            // Revised content for the same page replaces its old entry; only
            // another page is retained for a verified return-page snapshot.
            self.parked = previous.filter(|previous| {
                previous.key.page != key.page
                    && previous.key.generation == key.generation
                    && previous.key.domain == key.domain
                    && previous.key.font_generation == key.font_generation
                    && previous.key.mode_generation == key.mode_generation
                    && previous.key.color_info == key.color_info
                    && previous.key.properties == key.properties
                    && previous.key.dark_ui == key.dark_ui
            });
            self.key = Some(key.clone());
        }
        if self.level.as_ref().is_none_or(|level| level.scale != scale) {
            // The bounds include the actual screen-capped filter reach, so their
            // memo must change with scale even though authored content is unchanged.
            renderer.clear_camera_bounds();
            self.level = Some(DetailLevel::new(scale));
            self.refused = None;
        }
        self.frame = self.frame.wrapping_add(1);
        let transform = camera_transform(width, height, viewport, scale);
        let Some(visible) = visible_tiles(width, height, transform) else {
            return fallback_cancellable(
                renderer,
                canvas,
                width,
                height,
                scene,
                viewport,
                page,
                inputs,
                should_cancel,
            );
        };
        let Some(required) = tile_coverage(&visible).filter(|required| valid_bounds(*required))
        else {
            return fallback_cancellable(
                renderer,
                canvas,
                width,
                height,
                scene,
                viewport,
                page,
                inputs,
                should_cancel,
            );
        };
        if self
            .refused
            .is_some_and(|refused| contains_rect(refused, required))
        {
            return fallback_cancellable(
                renderer,
                canvas,
                width,
                height,
                scene,
                viewport,
                page,
                inputs,
                should_cancel,
            );
        }
        let Some(level) = self.level.as_mut() else {
            return fallback_cancellable(
                renderer,
                canvas,
                width,
                height,
                scene,
                viewport,
                page,
                inputs,
                should_cancel,
            );
        };
        if visible
            .iter()
            .any(|position| level.refused_tiles.contains(position))
        {
            return fallback_cancellable(
                renderer,
                canvas,
                width,
                height,
                scene,
                viewport,
                page,
                inputs,
                should_cancel,
            );
        }
        let mut metrics = RenderMetrics::default();
        let mut cache_metrics = CameraCacheMetrics::default();
        for position in &visible {
            if let Some(tile) = level.tiles.get_mut(position) {
                tile.last_used = self.frame;
            }
        }
        // A zoom visits each scale once: its frames render directly, without
        // a recording, a spatial index or tiles nothing would reuse. Tiles
        // start once a scale is presented again; panning then reuses them.
        if level.last_transform.is_none() {
            level.last_transform = Some(transform);
            let mut result = fallback_cancellable(
                renderer,
                canvas,
                width,
                height,
                scene,
                viewport,
                page,
                inputs,
                should_cancel,
            );
            result.needs_refresh = result.complete_detail && !result.cancelled;
            return result;
        }
        let motion = level
            .last_transform
            .map(|previous| (transform.0 - previous.0, transform.1 - previous.1));
        let missing: Vec<(i32, i32)> = visible
            .iter()
            .copied()
            .filter(|position| !level.tiles.contains_key(position))
            .collect();
        // A pan exposes a few tiles at a time: they are built now, from one
        // recording of just the exposed gap. A fresh scale or a large gap
        // replays one recording of the whole view instead.
        let gap = missing.len() <= MANDATORY_TILES;
        let coverage = if missing.is_empty() {
            None
        } else if gap {
            missing
                .iter()
                .map(|position| tile_rect(*position))
                .filter(|rect| !level.records(*rect))
                .reduce(|union, rect| {
                    let mut union = union;
                    union.join(rect);
                    union
                })
                .map(|union| ahead_of(union, motion))
        } else if level.records(required) {
            None
        } else {
            // Coverage is bounded in device pixels. It includes complete tile
            // edges and a nearby fringe, never a page-wide prerequisite.
            Some(ahead_of(
                required.with_outset((TILE_SIZE as f32, TILE_SIZE as f32)),
                motion,
            ))
        };
        if let Some(coverage) = coverage {
            match record_region(
                level,
                &mut self.parked,
                renderer,
                scene,
                page,
                inputs,
                &key,
                coverage,
                self.frame,
                should_cancel,
                &mut cache_metrics,
                &mut metrics,
            ) {
                RegionRecord::Recorded => {}
                RegionRecord::Cancelled(recorded) => {
                    return cancelled_result(*recorded, cache_metrics);
                }
                RegionRecord::Unsupported(recorded) => {
                    // Backdrop sampling cannot be certified by a recording
                    // device. Refuse only this coverage; a distant blur must
                    // not disable a page.
                    self.refused = Some(coverage);
                    let mut result = fallback_cancellable(
                        renderer,
                        canvas,
                        width,
                        height,
                        scene,
                        viewport,
                        page,
                        inputs,
                        should_cancel,
                    );
                    add_walks(&mut result, &cache_metrics, &recorded, started);
                    return result;
                }
                RegionRecord::Unretainable(recorded) => {
                    let mut result = fallback_cancellable(
                        renderer,
                        canvas,
                        width,
                        height,
                        scene,
                        viewport,
                        page,
                        inputs,
                        should_cancel,
                    );
                    add_walks(&mut result, &cache_metrics, &recorded, started);
                    return result;
                }
            }
        }
        if gap {
            for position in &missing {
                match build_tile(
                    level,
                    &mut self.parked,
                    renderer,
                    canvas,
                    scene,
                    page,
                    inputs,
                    &key,
                    *position,
                    tile_rect(*position),
                    &visible,
                    self.frame,
                    should_cancel,
                    &mut cache_metrics,
                    &mut metrics,
                ) {
                    TileBuild::Built => {}
                    TileBuild::Full => break,
                    TileBuild::Cancelled(recorded) => {
                        return cancelled_result(*recorded, cache_metrics);
                    }
                    TileBuild::Refused => {
                        let mut result = fallback_cancellable(
                            renderer,
                            canvas,
                            width,
                            height,
                            scene,
                            viewport,
                            page,
                            inputs,
                            should_cancel,
                        );
                        result.cache.scene_walks += cache_metrics.scene_walks;
                        result.cache.scene_walk_wall_us += cache_metrics.scene_walk_wall_us;
                        return result;
                    }
                }
            }
        }
        {
            // Spend what remains of the frame budget from existing recordings
            // only (never a scene walk): the rest of a large visible gap
            // first, then the tiles the motion reaches next.
            let ahead = motion.map_or_else(Vec::new, |motion| tiles_ahead(&visible, motion));
            let mut prefetched = 0;
            for position in visible.iter().chain(&ahead) {
                if started.elapsed() >= TILE_BUDGET {
                    break;
                }
                let prefetch = !visible.contains(position);
                if prefetch && prefetched >= PREFETCH_TILES {
                    break;
                }
                if level.tiles.contains_key(position)
                    || level.refused_tiles.contains(position)
                    || !level.records(tile_rect(*position))
                    || (prefetch && !has_room(level, self.parked.as_ref()))
                {
                    continue;
                }
                match build_tile(
                    level,
                    &mut self.parked,
                    renderer,
                    canvas,
                    scene,
                    page,
                    inputs,
                    &key,
                    *position,
                    tile_rect(*position),
                    &visible,
                    self.frame,
                    should_cancel,
                    &mut cache_metrics,
                    &mut metrics,
                ) {
                    TileBuild::Built => prefetched += usize::from(prefetch),
                    TileBuild::Full | TileBuild::Refused => break,
                    TileBuild::Cancelled(recorded) => {
                        return cancelled_result(*recorded, cache_metrics);
                    }
                }
            }
        }
        level.last_transform = Some(transform);
        if should_cancel() {
            return cancelled_result(metrics, cache_metrics);
        }
        let gaps: Vec<(i32, i32)> = visible
            .iter()
            .copied()
            .filter(|position| !level.tiles.contains_key(position))
            .collect();
        let complete = gaps.is_empty();
        // Mostly uncovered views replay one recording of the whole view;
        // otherwise only the gaps replay, so the cost follows the uncovered
        // area, not the whole view.
        let whole = (gaps.len() * 2 > visible.len())
            .then(|| level.touch_recording(required, self.frame))
            .flatten();
        canvas.clear(to_sk_color(background));
        if whole.is_none() {
            let mut paint = Paint::default();
            paint.set_blend_mode(BlendMode::Src);
            let source = Rect::from_xywh(
                GUTTER as f32,
                GUTTER as f32,
                TILE_SIZE as f32,
                TILE_SIZE as f32,
            );
            for position in &visible {
                let Some(tile) = level.tiles.get(position) else {
                    continue;
                };
                canvas.draw_image_rect_with_sampling_options(
                    &tile.image,
                    Some((&source, SrcRectConstraint::Strict)),
                    tile_destination(*position, transform),
                    SamplingOptions::default(),
                    &paint,
                );
                cache_metrics.tiles_drawn += 1;
            }
        }
        let mut replays = Vec::new();
        if let Some(picture) = whole {
            replays.push((None, picture));
        } else {
            for position in &gaps {
                let Some(picture) = level.touch_recording(tile_rect(*position), self.frame) else {
                    return fallback_cancellable(
                        renderer,
                        canvas,
                        width,
                        height,
                        scene,
                        viewport,
                        page,
                        inputs,
                        should_cancel,
                    );
                };
                replays.push((Some(tile_destination(*position, transform)), picture));
            }
        }
        for (clip, picture) in &replays {
            canvas.save();
            if let Some(clip) = clip {
                canvas.clip_rect(*clip, skia_safe::ClipOp::Intersect, false);
            }
            canvas.translate((transform.0 as f32, transform.1 as f32));
            canvas.draw_picture(picture, None, None);
            canvas.restore();
            cache_metrics.picture_drawn = true;
        }
        let pending = !complete
            || (has_room(level, self.parked.as_ref())
                && ring_tiles(&visible).iter().any(|position| {
                    !level.tiles.contains_key(position) && !level.refused_tiles.contains(position)
                }));
        cache_metrics.retained_bytes = level_bytes(Some(level))
            + self
                .parked
                .as_ref()
                .map_or(0, |parked| level_bytes(parked.level.as_ref()));
        metrics.frame_micros = started.elapsed().as_micros() as u64;
        CameraCacheResult {
            metrics,
            cache: cache_metrics,
            needs_refresh: pending,
            complete_detail: true,
            cancelled: false,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn maintain(
        &mut self,
        renderer: &mut RasterRenderer,
        canvas: &Canvas,
        width: u32,
        height: u32,
        scene: &Scene,
        viewport: &Viewport,
        page: Option<NodeId>,
        inputs: &RenderInputs,
        generation: u64,
        should_cancel: &dyn Fn() -> bool,
    ) -> CameraCacheResult {
        let started = Instant::now();
        let mut metrics = RenderMetrics::default();
        let mut cache_metrics = CameraCacheMetrics::default();
        if should_cancel() {
            return cancelled_result(metrics, cache_metrics);
        }
        let scale = f64::from((viewport.zoom * renderer.display_scale) as f32);
        let Some(domain) = cache_domain(canvas) else {
            return idle_result(metrics, cache_metrics, false);
        };
        let key = ContentKey {
            scene: (scene.instance_id(), scene.revision()),
            generation,
            page,
            mode_generation: inputs.mode_generation,
            dark_ui: inputs.dark_ui,
            font_generation: RasterRenderer::current_font_generation(),
            background: renderer.camera_background(scene, page, inputs),
            domain,
            color_info: canvas.image_info().color_info().clone(),
            properties: canvas.top_props(),
        };
        // Maintenance only augments an already presented, validated recording;
        // it cannot create a scene generation or paint into the output target.
        if self.key.as_ref() != Some(&key)
            || self.components != *inputs.components
            || self.variables != *inputs.variables
            || self.active_modes != *inputs.active_modes
            || !self
                .domain_witness
                .as_mut()
                .is_some_and(|surface| cache_domain(surface.canvas()) == Some(domain))
            || self.level.as_ref().is_none_or(|level| level.scale != scale)
            || !renderer.pixel_snap_pan()
            || !scale.is_finite()
            || scale <= 0.0
            || width == 0
            || height == 0
            || width > i32::MAX as u32
            || height > i32::MAX as u32
            || !viewport.center.iter().all(|value| value.is_finite())
            || !canvas.local_to_device_as_3x3().is_identity()
            || canvas.image_info().bytes_per_pixel() != 4
            || inputs.motion.is_some()
            || inputs.playback.is_some_and(|values| !values.is_empty())
            || inputs
                .video_fill_frames
                .is_some_and(|values| !values.is_empty())
        {
            return idle_result(metrics, cache_metrics, false);
        }
        let transform = camera_transform(width, height, viewport, scale);
        let Some(visible) = visible_tiles(width, height, transform) else {
            return idle_result(metrics, cache_metrics, false);
        };
        let Some(required) = tile_coverage(&visible).filter(|required| valid_bounds(*required))
        else {
            return idle_result(metrics, cache_metrics, false);
        };
        if self
            .refused
            .is_some_and(|refused| contains_rect(refused, required))
        {
            return idle_result(metrics, cache_metrics, false);
        }
        let Some(level) = self
            .level
            .as_mut()
            .filter(|level| level.last_transform.is_some())
        else {
            return idle_result(metrics, cache_metrics, false);
        };
        if visible
            .iter()
            .any(|position| level.refused_tiles.contains(position))
        {
            return idle_result(metrics, cache_metrics, false);
        }
        // The visible gap first, nearest the center; then the ring around the
        // view, which may need its own small recording — the camera is idle.
        let ring = ring_tiles(&visible);
        let room = has_room(level, self.parked.as_ref());
        let next = visible
            .iter()
            .chain(ring.iter().filter(|_| room))
            .find(|position| {
                !level.tiles.contains_key(position) && !level.refused_tiles.contains(position)
            })
            .copied();
        if let Some(position) = next {
            self.frame = self.frame.wrapping_add(1);
            match build_tile(
                level,
                &mut self.parked,
                renderer,
                canvas,
                scene,
                page,
                inputs,
                &key,
                position,
                // The first visible tile at a scale records the whole view in
                // one walk; later ring tiles record only themselves.
                if visible.contains(&position) {
                    required.with_outset((TILE_SIZE as f32, TILE_SIZE as f32))
                } else {
                    tile_rect(position)
                },
                &visible,
                self.frame,
                should_cancel,
                &mut cache_metrics,
                &mut metrics,
            ) {
                TileBuild::Built | TileBuild::Refused => {}
                TileBuild::Full => return idle_result(metrics, cache_metrics, false),
                TileBuild::Cancelled(recorded) => {
                    return cancelled_result(*recorded, cache_metrics);
                }
            }
        }
        let room = has_room(level, self.parked.as_ref());
        let pending = visible
            .iter()
            .chain(ring.iter().filter(|_| room))
            .any(|position| {
                !level.tiles.contains_key(position) && !level.refused_tiles.contains(position)
            });
        cache_metrics.retained_bytes = level_bytes(Some(level))
            + self
                .parked
                .as_ref()
                .map_or(0, |parked| level_bytes(parked.level.as_ref()));
        metrics.frame_micros = started.elapsed().as_micros() as u64;
        if should_cancel() {
            return cancelled_result(metrics, cache_metrics);
        }
        idle_result(metrics, cache_metrics, pending)
    }
}

/// Render one tile into its own image from a recording that covers it,
/// recording `coverage` (which must contain the tile) first when none does.
/// Evicts the least recently used tile outside `visible` when the image
/// budget is full.
#[allow(clippy::too_many_arguments)]
fn build_tile(
    level: &mut DetailLevel,
    parked: &mut Option<ParkedPage>,
    renderer: &mut RasterRenderer,
    canvas: &Canvas,
    scene: &Scene,
    page: Option<NodeId>,
    inputs: &RenderInputs,
    key: &ContentKey,
    position: (i32, i32),
    coverage: Rect,
    visible: &[(i32, i32)],
    frame: u64,
    should_cancel: &dyn Fn() -> bool,
    cache_metrics: &mut CameraCacheMetrics,
    metrics: &mut RenderMetrics,
) -> TileBuild {
    if !make_room(level, parked, visible) {
        return TileBuild::Full;
    }
    let rect = tile_rect(position);
    if !level.records(rect) {
        let coverage = if contains_rect(coverage, rect) {
            coverage
        } else {
            rect
        };
        match record_region(
            level,
            parked,
            renderer,
            scene,
            page,
            inputs,
            key,
            coverage,
            frame,
            should_cancel,
            cache_metrics,
            metrics,
        ) {
            RegionRecord::Recorded => {}
            RegionRecord::Cancelled(recorded) => return TileBuild::Cancelled(recorded),
            RegionRecord::Unsupported(_) => {
                level.refused_tiles.insert(position);
                return TileBuild::Refused;
            }
            RegionRecord::Unretainable(_) => return TileBuild::Refused,
        }
    }
    let Some(image) = level
        .touch_recording(rect, frame)
        .and_then(|picture| make_tile(canvas, &picture, position, key.background))
    else {
        level.refused_tiles.insert(position);
        return TileBuild::Refused;
    };
    level.tiles.insert(
        position,
        Tile {
            image,
            last_used: frame,
        },
    );
    cache_metrics.tiles_built += 1;
    TileBuild::Built
}

/// Record `coverage` at the level's scale and keep it, evicting the least
/// recently used recordings (then the parked page) to stay in budget.
#[allow(clippy::too_many_arguments)]
fn record_region(
    level: &mut DetailLevel,
    parked: &mut Option<ParkedPage>,
    renderer: &mut RasterRenderer,
    scene: &Scene,
    page: Option<NodeId>,
    inputs: &RenderInputs,
    key: &ContentKey,
    coverage: Rect,
    frame: u64,
    should_cancel: &dyn Fn() -> bool,
    cache_metrics: &mut CameraCacheMetrics,
    metrics: &mut RenderMetrics,
) -> RegionRecord {
    let recorded =
        renderer.record_camera_region(scene, level.scale, page, inputs, coverage, should_cancel);
    let (picture, recorded) = match recorded {
        Ok(recorded) => recorded,
        Err(recorded) => {
            cache_metrics.scene_walks += 1;
            cache_metrics.scene_walk_wall_us += recorded.frame_micros;
            return if recorded.camera_cancelled || should_cancel() {
                RegionRecord::Cancelled(recorded)
            } else {
                RegionRecord::Unsupported(recorded)
            };
        }
    };
    accumulate(metrics, &recorded);
    cache_metrics.scene_walks += 1;
    cache_metrics.scene_walk_wall_us += recorded.frame_micros;
    if should_cancel() {
        return RegionRecord::Cancelled(Box::new(recorded));
    }
    let bytes = picture.approximate_bytes_used();
    if bytes > PICTURE_BUDGET || RasterRenderer::current_font_generation() != key.font_generation {
        return RegionRecord::Unretainable(Box::new(recorded));
    }
    while level.recording_bytes()
        + parked.as_ref().map_or(0, |parked| {
            parked
                .level
                .as_ref()
                .map_or(0, DetailLevel::recording_bytes)
        })
        + bytes
        > PICTURE_BUDGET
    {
        if let Some(oldest) = level
            .recordings
            .iter()
            .enumerate()
            .min_by_key(|(_, recording)| recording.last_used)
            .map(|(index, _)| index)
        {
            level.recordings.remove(oldest);
        } else if parked.is_some() {
            *parked = None;
        } else {
            break;
        }
    }
    level.recordings.push(RegionRecording {
        bounds: coverage,
        picture,
        last_used: frame,
    });
    RegionRecord::Recorded
}

fn add_walks(
    result: &mut CameraCacheResult,
    walks: &CameraCacheMetrics,
    recorded: &RenderMetrics,
    started: Instant,
) {
    result.cache.scene_walks += walks.scene_walks;
    result.cache.scene_walk_wall_us += walks.scene_walk_wall_us;
    result.metrics.bounds_prepared += recorded.bounds_prepared;
    result.metrics.frame_micros = started.elapsed().as_micros() as u64;
}

fn tile_capacity() -> usize {
    IMAGE_BUDGET / tile_bytes()
}

fn retained_tiles(level: &DetailLevel, parked: Option<&ParkedPage>) -> usize {
    level.tiles.len()
        + parked
            .and_then(|parked| parked.level.as_ref())
            .map_or(0, |level| level.tiles.len())
}

/// Whether another tile fits without evicting one; prefetched tiles never
/// evict, so they cannot churn against each other.
fn has_room(level: &DetailLevel, parked: Option<&ParkedPage>) -> bool {
    retained_tiles(level, parked) < tile_capacity()
}

/// Evict least recently used tiles outside `visible` (the parked page's
/// first when older) until another tile fits.
fn make_room(
    level: &mut DetailLevel,
    parked: &mut Option<ParkedPage>,
    visible: &[(i32, i32)],
) -> bool {
    while !has_room(level, parked.as_ref()) {
        let current_oldest = level
            .tiles
            .iter()
            .filter(|(position, _)| !visible.contains(position))
            .min_by_key(|(_, tile)| tile.last_used)
            .map(|(position, tile)| (*position, tile.last_used));
        let parked_oldest = parked
            .as_ref()
            .and_then(|parked| parked.level.as_ref())
            .and_then(|level| level.tiles.iter().min_by_key(|(_, tile)| tile.last_used))
            .map(|(position, tile)| (*position, tile.last_used));
        if let Some((position, _)) = parked_oldest.filter(|(_, used)| {
            current_oldest.is_none_or(|(_, current_used)| *used <= current_used)
        }) {
            if let Some(parked) = parked.as_mut().and_then(|parked| parked.level.as_mut()) {
                parked.tiles.remove(&position);
            }
        } else if let Some((position, _)) = current_oldest {
            level.tiles.remove(&position);
        } else {
            return false;
        }
    }
    true
}

/// The device-space rectangle a tile's image covers, gutter included.
fn tile_rect(position: (i32, i32)) -> Rect {
    Rect::from_xywh(
        (position.0 * TILE_SIZE - GUTTER) as f32,
        (position.1 * TILE_SIZE - GUTTER) as f32,
        (TILE_SIZE + 2 * GUTTER) as f32,
        (TILE_SIZE + 2 * GUTTER) as f32,
    )
}

/// Where a tile lands on the target for the camera `transform`.
fn tile_destination(position: (i32, i32), transform: (f64, f64)) -> Rect {
    Rect::from_xywh(
        (f64::from(position.0) * f64::from(TILE_SIZE) + transform.0) as f32,
        (f64::from(position.1) * f64::from(TILE_SIZE) + transform.1) as f32,
        TILE_SIZE as f32,
        TILE_SIZE as f32,
    )
}

/// `region` extended in the direction the view is moving, so a steady pan
/// records new regions less often.
fn ahead_of(region: Rect, motion: Option<(f64, f64)>) -> Rect {
    let mut region = region;
    if let Some((dx, dy)) = motion {
        // Content moves against the view: a negative device shift means the
        // view advances toward larger tile indices.
        let lead = LEAD_TILES * TILE_SIZE as f32;
        if dx < 0.0 {
            region.right += lead;
        } else if dx > 0.0 {
            region.left -= lead;
        }
        if dy < 0.0 {
            region.bottom += lead;
        } else if dy > 0.0 {
            region.top -= lead;
        }
    }
    region
}

/// The tiles bordering the visible block.
fn ring_tiles(visible: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let (Some(minimum_x), Some(maximum_x), Some(minimum_y), Some(maximum_y)) = (
        visible.iter().map(|position| position.0).min(),
        visible.iter().map(|position| position.0).max(),
        visible.iter().map(|position| position.1).min(),
        visible.iter().map(|position| position.1).max(),
    ) else {
        return Vec::new();
    };
    let center_x = f64::from(minimum_x + maximum_x) * 0.5;
    let center_y = f64::from(minimum_y + maximum_y) * 0.5;
    let mut ring = Vec::new();
    for y in minimum_y.saturating_sub(1)..=maximum_y.saturating_add(1) {
        for x in minimum_x.saturating_sub(1)..=maximum_x.saturating_add(1) {
            if x < minimum_x || x > maximum_x || y < minimum_y || y > maximum_y {
                ring.push((x, y));
            }
        }
    }
    ring.sort_by(|first, second| {
        let distance = |position: &(i32, i32)| {
            (f64::from(position.0) - center_x).powi(2) + (f64::from(position.1) - center_y).powi(2)
        };
        distance(first).total_cmp(&distance(second))
    });
    ring
}

/// Ring tiles on the side the view is moving toward, most direct first.
fn tiles_ahead(visible: &[(i32, i32)], motion: (f64, f64)) -> Vec<(i32, i32)> {
    let (dx, dy) = (-motion.0, -motion.1);
    if dx == 0.0 && dy == 0.0 {
        return Vec::new();
    }
    let (Some(minimum_x), Some(maximum_x), Some(minimum_y), Some(maximum_y)) = (
        visible.iter().map(|position| position.0).min(),
        visible.iter().map(|position| position.0).max(),
        visible.iter().map(|position| position.1).min(),
        visible.iter().map(|position| position.1).max(),
    ) else {
        return Vec::new();
    };
    let center_x = f64::from(minimum_x + maximum_x) * 0.5;
    let center_y = f64::from(minimum_y + maximum_y) * 0.5;
    let mut ahead: Vec<((i32, i32), f64)> = ring_tiles(visible)
        .into_iter()
        .filter_map(|position| {
            let offset_x = f64::from(position.0) - center_x;
            let offset_y = f64::from(position.1) - center_y;
            let alignment = offset_x * dx + offset_y * dy;
            (alignment > 0.0).then_some((position, alignment))
        })
        .collect();
    ahead.sort_by(|first, second| second.1.total_cmp(&first.1));
    ahead.into_iter().map(|(position, _)| position).collect()
}

fn accumulate(total: &mut RenderMetrics, recorded: &RenderMetrics) {
    total.nodes_visited += recorded.nodes_visited;
    total.nodes_drawn += recorded.nodes_drawn;
    total.nodes_culled += recorded.nodes_culled;
    total.paths_built += recorded.paths_built;
    total.bounds_prepared += recorded.bounds_prepared;
    total.instance_indexes_built += recorded.instance_indexes_built;
    total.effect_layers += recorded.effect_layers;
    total.opacity_folds += recorded.opacity_folds;
    total.layer_cache_hits += recorded.layer_cache_hits;
    total.layer_cache_misses += recorded.layer_cache_misses;
    total.incomplete_artwork |= recorded.incomplete_artwork;
    total.non_artwork_content |= recorded.non_artwork_content;
    total.effect_failed |= recorded.effect_failed;
}

fn tile_bytes() -> usize {
    ((TILE_SIZE + 2 * GUTTER) as usize).pow(2) * 4
}

fn level_bytes(level: Option<&DetailLevel>) -> usize {
    level.map_or(0, |level| {
        level.tiles.len() * tile_bytes() + level.recording_bytes()
    })
}

fn idle_result(
    metrics: RenderMetrics,
    cache: CameraCacheMetrics,
    pending: bool,
) -> CameraCacheResult {
    CameraCacheResult {
        metrics,
        cache,
        needs_refresh: pending,
        complete_detail: true,
        cancelled: false,
    }
}

fn cancelled_result(mut metrics: RenderMetrics, cache: CameraCacheMetrics) -> CameraCacheResult {
    metrics.camera_cancelled = true;
    CameraCacheResult {
        metrics,
        cache,
        needs_refresh: false,
        complete_detail: false,
        cancelled: true,
    }
}

fn contains_rect(outer: Rect, inner: Rect) -> bool {
    outer.left <= inner.left
        && outer.top <= inner.top
        && outer.right >= inner.right
        && outer.bottom >= inner.bottom
}

fn tile_coverage(positions: &[(i32, i32)]) -> Option<Rect> {
    let minimum_x = positions.iter().map(|position| position.0).min()?;
    let minimum_y = positions.iter().map(|position| position.1).min()?;
    let maximum_x = positions.iter().map(|position| position.0).max()?;
    let maximum_y = positions.iter().map(|position| position.1).max()?;
    Some(Rect::new(
        minimum_x as f32 * TILE_SIZE as f32 - GUTTER as f32,
        minimum_y as f32 * TILE_SIZE as f32 - GUTTER as f32,
        (f64::from(maximum_x) + 1.0) as f32 * TILE_SIZE as f32 + GUTTER as f32,
        (f64::from(maximum_y) + 1.0) as f32 * TILE_SIZE as f32 + GUTTER as f32,
    ))
}

pub(super) fn begin_camera_node(
    canvas: &Canvas,
    node: &fanta_doc::CanvasNode,
    fallback_bounds: Option<Bounds>,
    metrics: &mut RenderMetrics,
) -> Option<Bounds> {
    if !metrics.camera_recording || metrics.camera_visible.is_some() {
        return None;
    }
    let outer = metrics.camera_bounds.take();
    let local = match &node.data {
        fanta_doc::NodeData::Vector(vector) => {
            crate::bounds::vector_visual_bounds(vector, node.flags)
        }
        fanta_doc::NodeData::Text(text) => super::walk::text_visual_bounds(text),
        fanta_doc::NodeData::TextPath(text) => super::text_path_bounds(text),
        _ => node.data.local_bounds().or(fallback_bounds),
    };
    metrics.camera_bounds = local.and_then(|bounds| {
        let bounds =
            crate::bounds::expand_for_strokes(bounds, node.data.strokes().unwrap_or(&[]), false);
        let (mapped, _) = canvas
            .local_to_device_as_3x3()
            .map_rect(bounds_rect(bounds));
        valid_bounds(mapped).then(|| rect_bounds(mapped))
    });
    outer
}

pub(super) fn end_camera_node(
    canvas: &Canvas,
    node: &fanta_doc::CanvasNode,
    outer: Option<Bounds>,
    effective_scale: f32,
    metrics: &mut RenderMetrics,
) {
    if !metrics.camera_recording || metrics.camera_visible.is_some() {
        return;
    }
    let matrix = canvas.local_to_device_as_3x3();
    if let Some(bounds) = metrics.camera_bounds {
        let Some(inverse) = matrix.invert() else {
            metrics.camera_unsupported = true;
            return;
        };
        let (local, _) = inverse.map_rect(bounds_rect(bounds));
        let padded = super::padded_layer_rect(&rect_bounds(local), effective_scale);
        let layer = super::effects::effects_layer_paint(
            node.opacity.get(),
            node.blend_mode,
            &node.effects,
            &node.blurs,
            node.flags.contains(super::NodeFlags::ISOLATED_BLEND),
            effective_scale,
        );
        // Layer blur is chained after the merged shadow filter, so their reaches add.
        // The same Skia fast-bounds proof used by retained effect layers includes that chain.
        let filtered = match layer.as_ref().and_then(|layer| layer.filter.as_ref()) {
            Some(filter) if filter.can_compute_fast_bounds() => filter.compute_fast_bounds(padded),
            Some(_) => {
                metrics.camera_unsupported = true;
                return;
            }
            None => padded,
        };
        let (expanded, _) = matrix.map_rect(filtered);
        if !valid_bounds(expanded) {
            metrics.camera_unsupported = true;
        }
        let expanded = rect_bounds(expanded);
        metrics.camera_bounds = Some(outer.map_or(expanded, |outer| outer.union(&expanded)));
    } else {
        metrics.camera_bounds = outer;
    }
}

pub(super) fn bounds_rect(bounds: Bounds) -> Rect {
    Rect::new(
        bounds.min_x as f32,
        bounds.min_y as f32,
        bounds.max_x as f32,
        bounds.max_y as f32,
    )
}

fn rect_bounds(bounds: Rect) -> Bounds {
    Bounds {
        min_x: f64::from(bounds.left),
        min_y: f64::from(bounds.top),
        max_x: f64::from(bounds.right),
        max_y: f64::from(bounds.bottom),
    }
}

fn valid_bounds(bounds: Rect) -> bool {
    !bounds.is_empty()
        && bounds.is_finite()
        && bounds.width().is_finite()
        && bounds.height().is_finite()
}

fn camera_transform(width: u32, height: u32, viewport: &Viewport, scale: f64) -> (f64, f64) {
    (
        (f64::from(width) * 0.5 - scale * viewport.center[0]).round(),
        (f64::from(height) * 0.5 - scale * viewport.center[1]).round(),
    )
}

fn make_tile(
    canvas: &Canvas,
    picture: &Picture,
    position: (i32, i32),
    background: Color,
) -> Option<Image> {
    let side = TILE_SIZE + GUTTER * 2;
    let mut surface = compatible_surface(canvas, side, side)?;
    surface.canvas().clear(to_sk_color(background));
    surface.canvas().translate((
        GUTTER as f32 - position.0 as f32 * TILE_SIZE as f32,
        GUTTER as f32 - position.1 as f32 * TILE_SIZE as f32,
    ));
    surface.canvas().draw_picture(picture, None, None);
    immutable_surface_snapshot(&mut surface)
}

fn compatible_surface(canvas: &Canvas, width: i32, height: i32) -> Option<skia_safe::Surface> {
    let info = ImageInfo::new(
        (width, height),
        canvas.image_info().color_type(),
        AlphaType::Premul,
        canvas.image_info().color_space(),
    );
    let properties = canvas.top_props();
    let mut surface = canvas.new_surface(&info, Some(&properties))?;
    if cache_domain(surface.canvas()) != cache_domain(canvas) {
        return None;
    }
    // A compatible image must remain a GPU image on GPU hosts; raster-only tests use
    // the identical composition code, but native frames must never read artwork back.
    #[cfg(feature = "metal")]
    if canvas.recorder().is_some() && surface.canvas().recorder().is_none() {
        return None;
    }
    Some(surface)
}

pub(super) fn is_gpu_canvas(canvas: &Canvas) -> bool {
    #[cfg(feature = "metal")]
    {
        matches!(
            cache_domain(canvas),
            Some(CacheDomain::Ganesh(_) | CacheDomain::Graphite(_))
        )
    }
    #[cfg(not(feature = "metal"))]
    {
        let _canvas = canvas;
        false
    }
}

fn cache_domain(canvas: &Canvas) -> Option<CacheDomain> {
    #[cfg(feature = "metal")]
    {
        use skia_safe::wrapper::PointerWrapper;
        if let Some(recorder) = canvas.recorder() {
            return Some(CacheDomain::Graphite(recorder.inner() as *const _ as usize));
        }
        if let Some(context) = canvas.direct_context() {
            return Some(CacheDomain::Ganesh(context.id()));
        }
        if canvas.recording_context().is_some() {
            return None;
        }
    }
    #[cfg(not(feature = "metal"))]
    let _canvas = canvas;
    Some(CacheDomain::Raster)
}

fn visible_tiles(width: u32, height: u32, transform: (f64, f64)) -> Option<Vec<(i32, i32)>> {
    let left = -transform.0;
    let top = -transform.1;
    let right = f64::from(width) - transform.0;
    let bottom = f64::from(height) - transform.1;
    if right <= left || bottom <= top {
        return Some(Vec::new());
    }
    let minimum_x = (left / f64::from(TILE_SIZE)).floor();
    let minimum_y = (top / f64::from(TILE_SIZE)).floor();
    let maximum_x = (right / f64::from(TILE_SIZE)).ceil();
    let maximum_y = (bottom / f64::from(TILE_SIZE)).ceil();
    if ![minimum_x, minimum_y, maximum_x, maximum_y]
        .iter()
        .all(|value| {
            value.is_finite() && *value >= f64::from(i32::MIN) && *value <= f64::from(i32::MAX)
        })
        || (maximum_x - minimum_x) * (maximum_y - minimum_y)
            > (IMAGE_BUDGET / ((TILE_SIZE + GUTTER * 2) as usize).pow(2) / 4) as f64
    {
        return None;
    }
    let mut positions = Vec::new();
    for y in minimum_y as i32..maximum_y as i32 {
        for x in minimum_x as i32..maximum_x as i32 {
            positions.push((x, y));
        }
    }
    let center_x = (left + right) * 0.5 / f64::from(TILE_SIZE);
    let center_y = (top + bottom) * 0.5 / f64::from(TILE_SIZE);
    positions.sort_by(|first, second| {
        let distance = |position: &(i32, i32)| {
            (f64::from(position.0) + 0.5 - center_x).powi(2)
                + (f64::from(position.1) + 0.5 - center_y).powi(2)
        };
        distance(first).total_cmp(&distance(second))
    });
    Some(positions)
}

#[allow(clippy::too_many_arguments)]
fn fallback_cancellable(
    renderer: &mut RasterRenderer,
    canvas: &Canvas,
    width: u32,
    height: u32,
    scene: &Scene,
    viewport: &Viewport,
    page: Option<NodeId>,
    inputs: &RenderInputs,
    should_cancel: &dyn Fn() -> bool,
) -> CameraCacheResult {
    let metrics = renderer.render_camera_fallback(
        canvas,
        width,
        height,
        scene,
        viewport,
        page,
        inputs,
        should_cancel,
    );
    let cache = CameraCacheMetrics {
        scene_walks: 1,
        scene_walk_wall_us: metrics.frame_micros,
        ..Default::default()
    };
    let cancelled = metrics.camera_cancelled || should_cancel();
    CameraCacheResult {
        metrics,
        cache,
        needs_refresh: false,
        complete_detail: !cancelled,
        cancelled,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn fallback(
    renderer: &mut RasterRenderer,
    canvas: &Canvas,
    width: u32,
    height: u32,
    scene: &Scene,
    viewport: &Viewport,
    page: Option<NodeId>,
    inputs: &RenderInputs,
) -> CameraCacheResult {
    let metrics = renderer.render_to_canvas(canvas, width, height, scene, viewport, page, inputs);
    let cache = CameraCacheMetrics {
        scene_walks: 1,
        scene_walk_wall_us: metrics.frame_micros,
        ..Default::default()
    };
    CameraCacheResult {
        metrics,
        cache,
        needs_refresh: false,
        complete_detail: true,
        cancelled: false,
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use fanta_doc::{
        CanvasNode, ComponentDef, ComponentId, Doc, Fill, GroupNode, InstanceNode, NodeData,
        NodeFlags, Operation, Shadow, ShadowKind, Transform2D, VectorNode,
    };
    use skia_safe::ColorType;

    pub(crate) const WIDTH: u32 = 640;
    pub(crate) const HEIGHT: u32 = 480;

    fn add(doc: &mut Doc, parent: Option<NodeId>, data: NodeData, x: f64, y: f64) -> NodeId {
        let mut node = CanvasNode::new(data);
        node.parent = parent;
        node.index = doc.scene.next_child_index(parent);
        node.transform = Transform2D::translation(x, y);
        let id = node.id;
        doc.apply(Operation::create_node(node))
            .expect("fixture node");
        id
    }

    pub(crate) fn fixture() -> (Doc, NodeId, NodeId) {
        let mut doc = Doc::new();
        let page = add(
            &mut doc,
            None,
            NodeData::Group(GroupNode {
                background: Some(Fill::solid(Color::rgb(16, 18, 22))),
                ..Default::default()
            }),
            0.0,
            0.0,
        );
        let rectangle = add(
            &mut doc,
            Some(page),
            NodeData::Vector(VectorNode::rect_solid(
                0.0,
                0.0,
                680.0,
                450.0,
                Color::rgba(190, 40, 90, 180),
            )),
            340.25,
            20.5,
        );
        let node = doc.scene.get_mut(rectangle).expect("rectangle");
        node.effects.push(Shadow {
            kind: ShadowKind::Drop,
            color: Color::rgba(0, 0, 0, 150),
            blur: 18.0,
            spread: 3.0,
            offset: [13.0, 6.0],
            show_behind_node: false,
        });
        node.blurs.push(fanta_doc::Blur::layer(2.0));
        let frame = add(
            &mut doc,
            Some(page),
            NodeData::Group(GroupNode {
                clip_size: Some([320.0, 150.0]),
                corner_radius: Some(12.0),
                background: Some(Fill::solid(Color::rgba(20, 80, 160, 190))),
                ..Default::default()
            }),
            430.75,
            280.25,
        );
        let masked = add(
            &mut doc,
            Some(frame),
            NodeData::Group(GroupNode::default()),
            0.0,
            0.0,
        );
        let mask = add(
            &mut doc,
            Some(masked),
            NodeData::Vector(VectorNode::rect_solid(0.0, 0.0, 150.0, 110.0, Color::WHITE)),
            40.0,
            5.0,
        );
        doc.scene.get_mut(mask).expect("mask").is_mask = true;
        let foreground = add(
            &mut doc,
            Some(masked),
            NodeData::Vector(VectorNode::rect_solid(
                0.0,
                0.0,
                400.0,
                140.0,
                Color::rgba(50, 210, 90, 150),
            )),
            0.0,
            20.0,
        );
        doc.scene
            .get_mut(foreground)
            .expect("foreground")
            .blend_mode = fanta_doc::BlendMode::Screen;
        (doc, page, rectangle)
    }

    fn target() -> skia_safe::Surface {
        skia_safe::surfaces::raster_n32_premul((WIDTH as i32, HEIGHT as i32)).expect("target")
    }

    fn pixels(surface: &mut skia_safe::Surface) -> Vec<u8> {
        let info = ImageInfo::new(
            (WIDTH as i32, HEIGHT as i32),
            ColorType::RGBA8888,
            AlphaType::Premul,
            None,
        );
        let mut pixels = vec![0; WIDTH as usize * HEIGHT as usize * 4];
        assert!(surface.read_pixels(&info, &mut pixels, WIDTH as usize * 4, (0, 0)));
        pixels
    }

    struct RasterCacheHarness {
        renderer: RasterRenderer,
        cache: CameraCache,
    }

    impl std::ops::Deref for RasterCacheHarness {
        type Target = RasterRenderer;
        fn deref(&self) -> &Self::Target {
            &self.renderer
        }
    }

    impl std::ops::DerefMut for RasterCacheHarness {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.renderer
        }
    }

    impl RasterCacheHarness {
        // These cases exercise cache proofs and scheduling with inspectable raster images;
        // the public API's retained pixel contract is tested on actual Metal canvases.
        #[allow(clippy::too_many_arguments)]
        fn render_camera_to_canvas(
            &mut self,
            canvas: &Canvas,
            width: u32,
            height: u32,
            scene: &Scene,
            viewport: &Viewport,
            page: Option<NodeId>,
            inputs: &RenderInputs,
            generation: u64,
            interacting: bool,
        ) -> CameraCacheResult {
            self.cache.render(
                &mut self.renderer,
                canvas,
                width,
                height,
                scene,
                viewport,
                page,
                inputs,
                generation,
                interacting,
                &|| false,
            )
        }

        fn maintain_camera_cache(
            &mut self,
            canvas: &Canvas,
            doc: &Doc,
            page: NodeId,
            viewport: &Viewport,
            should_cancel: &dyn Fn() -> bool,
        ) -> CameraCacheResult {
            self.cache.maintain(
                &mut self.renderer,
                canvas,
                WIDTH,
                HEIGHT,
                &doc.scene,
                viewport,
                Some(page),
                &RenderInputs::for_doc(doc),
                1,
                should_cancel,
            )
        }
    }

    fn renderer() -> RasterCacheHarness {
        let mut renderer = RasterRenderer::new(WIDTH, HEIGHT).expect("renderer");
        renderer.set_pixel_snap_pan(true);
        RasterCacheHarness {
            renderer,
            cache: CameraCache::default(),
        }
    }

    fn refine(
        renderer: &mut RasterCacheHarness,
        surface: &mut skia_safe::Surface,
        doc: &Doc,
        page: NodeId,
        viewport: &Viewport,
        interacting: bool,
    ) -> CameraCacheResult {
        for _ in 0..64 {
            let result = renderer.render_camera_to_canvas(
                surface.canvas(),
                WIDTH,
                HEIGHT,
                &doc.scene,
                viewport,
                Some(page),
                &RenderInputs::for_doc(doc),
                1,
                interacting,
            );
            let scale = f64::from(viewport.zoom as f32);
            let visible = visible_tiles(
                WIDTH,
                HEIGHT,
                camera_transform(WIDTH, HEIGHT, viewport, scale),
            )
            .map_or(0, |visible| visible.len());
            assert!(
                result.cache.tiles_built as usize <= visible + PREFETCH_TILES,
                "presentation builds at most the visible gap and a bounded prefetch"
            );
            assert!(result.cache.retained_bytes <= IMAGE_BUDGET + PICTURE_BUDGET);
            assert!(
                result.complete_detail && !result.cancelled,
                "every navigation frame is sharp"
            );
            assert!(
                !result.cache.overview_drawn,
                "a page preview must never be presented"
            );
            if !result.needs_refresh {
                return result;
            }
            let retained = pixels(surface);
            let maintenance =
                renderer.maintain_camera_cache(surface.canvas(), doc, page, viewport, &|| false);
            assert!(maintenance.cache.tiles_built <= 1);
            assert!(
                maintenance.cache.scene_walks <= 1,
                "maintenance records at most one tile of its own"
            );
            assert!(!maintenance.cache.picture_drawn);
            assert_eq!(
                pixels(surface),
                retained,
                "maintenance must not repaint the viewport"
            );
        }
        panic!("bounded visible tile set must settle");
    }

    fn assert_parity(
        renderer: &mut RasterCacheHarness,
        surface: &mut skia_safe::Surface,
        doc: &Doc,
        page: NodeId,
        viewport: &Viewport,
    ) -> CameraCacheResult {
        let result = refine(renderer, surface, doc, page, viewport, false);
        assert!(
            !result.cache.overview_drawn && result.cache.tiles_drawn > 0,
            "must exercise retained raster path"
        );
        let mut reference = target();
        let mut normal = self::renderer();
        normal.render_to_canvas(
            reference.canvas(),
            WIDTH,
            HEIGHT,
            &doc.scene,
            viewport,
            Some(page),
            &RenderInputs::for_doc(doc),
        );
        let actual = pixels(surface);
        let expected = pixels(&mut reference);
        let maximum = actual
            .iter()
            .zip(&expected)
            .map(|(actual, expected)| actual.abs_diff(*expected))
            .max()
            .expect("pixels");
        if maximum > 2 {
            let differences: Vec<_> = actual
                .chunks_exact(4)
                .zip(expected.chunks_exact(4))
                .enumerate()
                .filter(|(_, (actual, expected))| {
                    actual
                        .iter()
                        .zip(*expected)
                        .any(|(actual, expected)| actual.abs_diff(*expected) > 2)
                })
                .take(8)
                .map(|(position, (actual, expected))| {
                    (
                        position % WIDTH as usize,
                        position / WIDTH as usize,
                        actual,
                        expected,
                    )
                })
                .collect();
            let mut direct_picture = target();
            let scale = f64::from(viewport.zoom as f32);
            let (picture, _) = normal
                .record_camera_picture(&doc.scene, scale, Some(page), &RenderInputs::for_doc(doc))
                .expect("same recording");
            direct_picture
                .canvas()
                .clear(to_sk_color(normal.camera_background(
                    &doc.scene,
                    Some(page),
                    &RenderInputs::for_doc(doc),
                )));
            let transform = camera_transform(WIDTH, HEIGHT, viewport, scale);
            direct_picture
                .canvas()
                .translate((transform.0 as f32, transform.1 as f32));
            direct_picture.canvas().draw_picture(&picture, None, None);
            let direct_pixels = pixels(&mut direct_picture);
            let direct_maximum = direct_pixels
                .iter()
                .zip(&expected)
                .map(|(actual, expected)| actual.abs_diff(*expected))
                .max()
                .expect("direct pixels");
            let tile_vs_picture = actual
                .iter()
                .zip(&direct_pixels)
                .map(|(actual, expected)| actual.abs_diff(*expected))
                .max()
                .expect("tile pixels");
            panic!(
                "tile composition differs by {maximum} at {viewport:?}: {differences:?}; direct picture difference={direct_maximum}, tile vs picture={tile_vs_picture}"
            );
        }
        result
    }

    /// Raster tiles crop curves device-dependently: a handful of edge pixels
    /// may differ by a few levels from an uncropped draw. GPU tiles are held
    /// to exact parity by the Metal tests.
    fn assert_raster_close(actual: &[u8], expected: &[u8], context: &str) {
        let differing: Vec<(usize, u8)> = actual
            .chunks_exact(4)
            .zip(expected.chunks_exact(4))
            .enumerate()
            .map(|(index, (actual, expected))| (index, max_difference(actual, expected)))
            .filter(|(_, difference)| *difference > 2)
            .collect();
        assert!(
            differing.len() <= 8 && differing.iter().all(|(_, difference)| *difference <= 8),
            "{context}: {} pixels differ, worst {:?}",
            differing.len(),
            differing.iter().max_by_key(|(_, difference)| *difference)
        );
    }

    fn max_difference(first: &[u8], second: &[u8]) -> u8 {
        first
            .iter()
            .zip(second)
            .map(|(first, second)| first.abs_diff(*second))
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn camera_presentation_draws_a_new_scale_directly_then_tiles_it_and_maintenance_never_repaints()
    {
        let (doc, page, _) = fixture();
        let viewport = Viewport {
            center: [512.0, 240.0],
            zoom: 1.0,
        };
        let mut renderer = renderer();
        let mut surface = target();
        let present = |renderer: &mut RasterCacheHarness, surface: &mut skia_safe::Surface| {
            renderer.render_camera_to_canvas(
                surface.canvas(),
                WIDTH,
                HEIGHT,
                &doc.scene,
                &viewport,
                Some(page),
                &RenderInputs::for_doc(&doc),
                1,
                false,
            )
        };
        // A zoom visits each scale once, so its first frame draws directly
        // instead of recording or building tiles nothing will reuse.
        let first = present(&mut renderer, &mut surface);
        assert_eq!(first.cache.tiles_built, 0);
        assert!(first.complete_detail && !first.cache.picture_drawn && first.needs_refresh);
        assert_eq!(first.cache.scene_walks, 1);
        let direct = pixels(&mut surface);
        let cancelled =
            renderer.maintain_camera_cache(surface.canvas(), &doc, page, &viewport, &|| true);
        assert!(cancelled.cancelled && cancelled.cache.tiles_built == 0);
        assert_eq!(pixels(&mut surface), direct);
        let recorded =
            renderer.maintain_camera_cache(surface.canvas(), &doc, page, &viewport, &|| false);
        assert_eq!(recorded.cache.tiles_built, 1);
        assert_eq!(
            recorded.cache.scene_walks, 1,
            "one recording covers the view and its fringe"
        );
        assert!(!recorded.cache.picture_drawn && recorded.cache.tiles_drawn == 0);
        assert_eq!(pixels(&mut surface), direct);
        let second = present(&mut renderer, &mut surface);
        assert!(
            second.cache.tiles_built > 0 && second.cache.tiles_built as usize <= MANDATORY_TILES,
            "a presented scale builds its small visible gap"
        );
        assert_eq!(second.cache.scene_walks, 0, "from the recording");
        assert!(!second.cache.picture_drawn && second.cache.tiles_drawn > 0);
        assert_raster_close(
            &pixels(&mut surface),
            &direct,
            "tiles composite what the direct frame drew",
        );
        assert!(
            second.needs_refresh,
            "the ring around the view is still missing"
        );
        let sharp_pixels = pixels(&mut surface);
        let maintenance =
            renderer.maintain_camera_cache(surface.canvas(), &doc, page, &viewport, &|| false);
        assert_eq!(maintenance.cache.tiles_built, 1);
        assert_eq!(
            maintenance.cache.scene_walks, 0,
            "a ring tile inside the recording needs no walk"
        );
        assert_eq!(maintenance.metrics.nodes_visited, 0);
        assert!(!maintenance.cache.picture_drawn && maintenance.cache.tiles_drawn == 0);
        assert_eq!(pixels(&mut surface), sharp_pixels);
        let checks = std::cell::Cell::new(0);
        let interrupted =
            renderer.maintain_camera_cache(surface.canvas(), &doc, page, &viewport, &|| {
                checks.set(checks.get() + 1);
                checks.get() >= 2
            });
        assert!(interrupted.cancelled && interrupted.cache.tiles_built == 1);
        assert_eq!(pixels(&mut surface), sharp_pixels);
        let resumed = refine(&mut renderer, &mut surface, &doc, page, &viewport, false);
        assert_eq!(resumed.cache.scene_walks, 0);
        assert!(resumed.cache.tiles_drawn > 0);
    }

    #[test]
    fn camera_pan_builds_the_exposed_gap_and_prefetches_ahead_of_motion() {
        let (doc, page, _) = fixture();
        let mut renderer = renderer();
        let mut surface = target();
        let start = Viewport {
            center: [512.0, 240.0],
            zoom: 1.0,
        };
        refine(&mut renderer, &mut surface, &doc, page, &start, false);
        let mut prefetched = 0;
        for step in 1..=8 {
            let viewport = Viewport {
                center: [512.0 + f64::from(step) * 96.0, 240.0],
                zoom: 1.0,
            };
            let result = renderer.render_camera_to_canvas(
                surface.canvas(),
                WIDTH,
                HEIGHT,
                &doc.scene,
                &viewport,
                Some(page),
                &RenderInputs::for_doc(&doc),
                1,
                false,
            );
            assert!(result.complete_detail && !result.cancelled);
            assert!(
                !result.cache.picture_drawn,
                "a steady pan composites tiles instead of replaying the view"
            );
            let level = renderer.cache.level.as_ref().expect("level");
            let transform = camera_transform(WIDTH, HEIGHT, &viewport, 1.0);
            let visible = visible_tiles(WIDTH, HEIGHT, transform).expect("visible");
            prefetched += tiles_ahead(&visible, (-96.0, 0.0))
                .iter()
                .filter(|position| level.tiles.contains_key(position))
                .count();
            let mut reference = target();
            let mut normal = self::renderer();
            normal.render_to_canvas(
                reference.canvas(),
                WIDTH,
                HEIGHT,
                &doc.scene,
                &viewport,
                Some(page),
                &RenderInputs::for_doc(&doc),
            );
            assert_raster_close(
                &pixels(&mut surface),
                &pixels(&mut reference),
                &format!("panned tile composition matches normal rendering at {viewport:?}"),
            );
        }
        assert!(
            prefetched > 0,
            "tiles ahead of the motion are ready before they show"
        );
    }

    #[test]
    fn camera_page_return_reuses_exact_retained_pixels() {
        let (mut doc, page, _) = fixture();
        let other = add(
            &mut doc,
            None,
            NodeData::Group(GroupNode::default()),
            0.0,
            0.0,
        );
        add(
            &mut doc,
            Some(other),
            NodeData::Vector(VectorNode::rect_solid(
                0.0,
                0.0,
                2_000.0,
                2_000.0,
                Color::rgb(20, 180, 60),
            )),
            0.0,
            0.0,
        );
        let viewport = Viewport {
            center: [512.0, 240.0],
            zoom: 1.0,
        };
        let mut renderer = renderer();
        let mut surface = target();
        refine(&mut renderer, &mut surface, &doc, page, &viewport, false);
        let first_page = pixels(&mut surface);
        refine(&mut renderer, &mut surface, &doc, other, &viewport, false);
        assert_ne!(pixels(&mut surface), first_page);
        let returned = renderer.render_camera_to_canvas(
            surface.canvas(),
            WIDTH,
            HEIGHT,
            &doc.scene,
            &viewport,
            Some(page),
            &RenderInputs::for_doc(&doc),
            1,
            false,
        );
        assert_eq!(returned.cache.scene_walks, 0);
        assert_eq!(returned.metrics.nodes_visited, 0);
        assert_eq!(returned.cache.tiles_built, 0);
        assert!(returned.cache.tiles_drawn > 0 && !returned.needs_refresh);
        assert_eq!(pixels(&mut surface), first_page);
        assert!(returned.cache.retained_bytes <= IMAGE_BUDGET + PICTURE_BUDGET);
    }

    #[test]
    fn camera_pages_share_one_global_tile_budget() {
        let mut doc = Doc::new();
        let mut pages = Vec::new();
        for color in [Color::rgb(180, 30, 40), Color::rgb(20, 150, 60)] {
            let page = add(
                &mut doc,
                None,
                NodeData::Group(GroupNode::default()),
                0.0,
                0.0,
            );
            add(
                &mut doc,
                Some(page),
                NodeData::Vector(VectorNode::rect_solid(0.0, 0.0, 40_000.0, 1_000.0, color)),
                0.0,
                0.0,
            );
            pages.push(page);
        }
        let mut renderer = renderer();
        let mut surface = target();
        for page in &pages {
            for step in 0..30 {
                let viewport = Viewport {
                    center: [512.0 + f64::from(step) * 1_024.0, 240.0],
                    zoom: 1.0,
                };
                let retained = refine(&mut renderer, &mut surface, &doc, *page, &viewport, false);
                assert!(retained.cache.retained_bytes <= IMAGE_BUDGET + PICTURE_BUDGET);
                let images = renderer
                    .cache
                    .level
                    .as_ref()
                    .map_or(0, |level| level.tiles.len())
                    + renderer
                        .cache
                        .parked
                        .as_ref()
                        .and_then(|parked| parked.level.as_ref())
                        .map_or(0, |level| level.tiles.len());
                assert!(images * tile_bytes() <= IMAGE_BUDGET);
            }
        }
        let first = pages.first().copied().expect("first page");
        let returned = renderer.render_camera_to_canvas(
            surface.canvas(),
            WIDTH,
            HEIGHT,
            &doc.scene,
            &Viewport {
                center: [512.0 + 29.0 * 1_024.0, 240.0],
                zoom: 1.0,
            },
            Some(first),
            &RenderInputs::for_doc(&doc),
            1,
            false,
        );
        assert!(returned.cache.tiles_drawn > 0 && !returned.needs_refresh);
        assert_eq!(returned.cache.scene_walks, 0);
    }

    #[test]
    fn camera_picture_preserves_clips_masks_blends_and_filters_across_viewports() {
        let (doc, page, _) = fixture();
        for center in [
            [512.0, 240.0],
            [613.25, 285.75],
            [300.5, 280.0],
            [512.0, 240.0],
        ] {
            let viewport = Viewport { center, zoom: 1.0 };
            let mut renderer = renderer();
            let mut normal = target();
            renderer.render_to_canvas(
                normal.canvas(),
                WIDTH,
                HEIGHT,
                &doc.scene,
                &viewport,
                Some(page),
                &RenderInputs::for_doc(&doc),
            );
            let (picture, _) = renderer
                .record_camera_picture(&doc.scene, 1.0, Some(page), &RenderInputs::for_doc(&doc))
                .expect("recording");
            let mut replay = target();
            replay
                .canvas()
                .clear(to_sk_color(renderer.camera_background(
                    &doc.scene,
                    Some(page),
                    &RenderInputs::for_doc(&doc),
                )));
            let transform = camera_transform(WIDTH, HEIGHT, &viewport, 1.0);
            replay
                .canvas()
                .translate((transform.0 as f32, transform.1 as f32));
            replay.canvas().draw_picture(&picture, None, None);
            assert_eq!(
                pixels(&mut replay),
                pixels(&mut normal),
                "whole display-list replay must preserve pixels at {viewport:?}"
            );
        }
    }

    #[test]
    fn camera_raster_entry_preserves_normal_pixels_without_refinement() {
        let (doc, page, _) = fixture();
        let viewport = Viewport {
            center: [512.0, 240.0],
            zoom: 1.0,
        };
        let mut renderer = renderer().renderer;
        let mut surface = target();
        let mut normal = target();
        let mut normal_renderer = self::renderer().renderer;
        for _ in 0..3 {
            normal_renderer.render_to_canvas(
                normal.canvas(),
                WIDTH,
                HEIGHT,
                &doc.scene,
                &viewport,
                Some(page),
                &RenderInputs::for_doc(&doc),
            );
            let result = renderer.render_camera_to_canvas(
                surface.canvas(),
                WIDTH,
                HEIGHT,
                &doc.scene,
                &viewport,
                Some(page),
                &RenderInputs::for_doc(&doc),
                1,
                false,
            );
            assert!(result.complete_detail && !result.needs_refresh);
            assert!(!result.cache.overview_drawn && result.cache.tiles_drawn == 0);
            assert!(result.metrics.nodes_visited > 0);
            assert_eq!(pixels(&mut surface), pixels(&mut normal));
        }
    }

    #[test]
    fn camera_visible_recording_sharply_covers_new_world_before_tiles() {
        let (mut doc, page, _) = fixture();
        add(
            &mut doc,
            Some(page),
            NodeData::Vector(VectorNode::rect_solid(
                0.0,
                0.0,
                600.0,
                600.0,
                Color::rgb(10, 220, 60),
            )),
            10_000.0,
            0.0,
        );
        let mut renderer = renderer();
        let mut surface = target();
        refine(
            &mut renderer,
            &mut surface,
            &doc,
            page,
            &Viewport {
                center: [512.0, 240.0],
                zoom: 1.0,
            },
            false,
        );
        let far = Viewport {
            center: [10_300.0, 300.0],
            zoom: 1.0,
        };
        let result = renderer.render_camera_to_canvas(
            surface.canvas(),
            WIDTH,
            HEIGHT,
            &doc.scene,
            &far,
            Some(page),
            &RenderInputs::for_doc(&doc),
            1,
            false,
        );
        assert!(
            !result.cache.overview_drawn
                && (result.cache.picture_drawn || result.cache.tiles_drawn > 0)
        );
        assert!(result.complete_detail);
        assert!(result.cache.scene_walks >= 1, "the new world is recorded");
        let pixels = pixels(&mut surface);
        let center = ((HEIGHT / 2 * WIDTH + WIDTH / 2) * 4) as usize;
        assert_eq!(
            pixels.get(center..center + 4),
            Some([10, 220, 60, 255].as_slice()),
            "newly uncovered world must display artwork immediately"
        );
        assert_parity(&mut renderer, &mut surface, &doc, page, &far);
    }

    #[test]
    fn camera_exact_library_witness_and_scene_edits_invalidate_retained_pixels() {
        let mut doc = Doc::new();
        let page = add(
            &mut doc,
            None,
            NodeData::Group(GroupNode::default()),
            0.0,
            0.0,
        );
        let red = add(
            &mut doc,
            None,
            NodeData::Vector(VectorNode::rect_solid(
                0.0,
                0.0,
                100.0,
                100.0,
                Color::rgb(220, 20, 30),
            )),
            5000.0,
            0.0,
        );
        let blue = add(
            &mut doc,
            None,
            NodeData::Vector(VectorNode::rect_solid(
                0.0,
                0.0,
                100.0,
                100.0,
                Color::rgb(20, 50, 220),
            )),
            6000.0,
            0.0,
        );
        let component = ComponentId::new();
        doc.components
            .defs
            .insert(component, ComponentDef::new(component, red, "Color"));
        add(
            &mut doc,
            Some(page),
            NodeData::Instance(InstanceNode {
                component,
                overrides: Vec::new(),
                prop_values: Default::default(),
                derived: Vec::new(),
                local_size: [100.0, 100.0],
            }),
            0.0,
            0.0,
        );
        let viewport = Viewport {
            center: [50.0, 50.0],
            zoom: 1.0,
        };
        let mut renderer = renderer();
        let mut surface = target();
        assert_parity(&mut renderer, &mut surface, &doc, page, &viewport);
        let before = pixels(&mut surface);
        let scene_revision = doc.scene.revision();
        let definition = doc.components.defs.get_mut(&component).expect("component");
        let definition_revision = definition.rev;
        definition.root = blue;
        assert_eq!(definition.rev, definition_revision);
        assert_eq!(doc.scene.revision(), scene_revision);
        assert_parity(&mut renderer, &mut surface, &doc, page, &viewport);
        assert_ne!(
            pixels(&mut surface),
            before,
            "same counters must not retain old component pixels"
        );
        doc.scene
            .get_mut(blue)
            .expect("master")
            .data
            .as_vector_mut()
            .expect("vector")
            .fills = [Fill::solid(Color::rgb(20, 220, 50))].into_iter().collect();
        assert_parity(&mut renderer, &mut surface, &doc, page, &viewport);
    }

    #[test]
    fn camera_metadata_changes_refresh_pattern_sources_and_retained_layers() {
        let mut doc = Doc::new();
        let page = add(
            &mut doc,
            None,
            NodeData::Group(GroupNode {
                background: Some(Fill::solid(Color::BLACK)),
                ..Default::default()
            }),
            0.0,
            0.0,
        );
        let red = add(
            &mut doc,
            None,
            NodeData::Vector(VectorNode::rect_solid(
                0.0,
                0.0,
                10.0,
                10.0,
                Color::rgb(220, 20, 30),
            )),
            20_000.0,
            0.0,
        );
        let blue = add(
            &mut doc,
            None,
            NodeData::Vector(VectorNode::rect_solid(
                0.0,
                0.0,
                10.0,
                10.0,
                Color::rgb(20, 50, 220),
            )),
            30_000.0,
            0.0,
        );
        let component = ComponentId::new();
        doc.components.defs.insert(
            component,
            ComponentDef::new(component, red, "Pattern color"),
        );
        let source = add(
            &mut doc,
            None,
            NodeData::Group(GroupNode {
                clip_size: Some([10.0, 10.0]),
                ..Default::default()
            }),
            10_000.0,
            0.0,
        );
        add(
            &mut doc,
            Some(source),
            NodeData::Instance(InstanceNode {
                component,
                overrides: Vec::new(),
                prop_values: Default::default(),
                derived: Vec::new(),
                local_size: [10.0, 10.0],
            }),
            0.0,
            0.0,
        );
        let mut vector = VectorNode::rect_solid(0.0, 0.0, 200.0, 200.0, Color::WHITE);
        vector.fills = [Fill::Pattern {
            pattern: Box::new(fanta_doc::PatternFill {
                source_node_id: source,
                tile_type: fanta_doc::PatternTileType::Rectangular,
                scaling_factor: 1.0,
                spacing: Default::default(),
                horizontal_alignment: fanta_doc::PatternHorizontalAlignment::Start,
            }),
            opacity: 1.0,
            blend: fanta_doc::BlendMode::Normal,
        }]
        .into_iter()
        .collect();
        let target_id = add(&mut doc, Some(page), NodeData::Vector(vector), 0.0, 0.0);
        doc.scene
            .get_mut(target_id)
            .expect("pattern target")
            .opacity = fanta_doc::UnitInterval::new(0.6);
        let viewport = Viewport {
            center: [100.0, 100.0],
            zoom: 1.0,
        };
        let mut renderer = renderer();
        let mut surface = target();
        let baseline = refine(&mut renderer, &mut surface, &doc, page, &viewport, false);
        assert!(baseline.complete_detail);
        let old_pixels = pixels(&mut surface);
        // Warm the older effect-layer path too: it can hold the pattern's already
        // flattened pixels while the source's instance metadata is changed directly.
        for _ in 0..3 {
            renderer.renderer.render_to_canvas(
                surface.canvas(),
                WIDTH,
                HEIGHT,
                &doc.scene,
                &viewport,
                Some(page),
                &RenderInputs::for_doc(&doc),
            );
        }
        let scene_revision = doc.scene.revision();
        let definition = doc.components.defs.get_mut(&component).expect("component");
        let definition_revision = definition.rev;
        definition.root = blue;
        assert_eq!(definition.rev, definition_revision);
        assert_eq!(doc.scene.revision(), scene_revision);
        let first = renderer.render_camera_to_canvas(
            surface.canvas(),
            WIDTH,
            HEIGHT,
            &doc.scene,
            &viewport,
            Some(page),
            &RenderInputs::for_doc(&doc),
            1,
            false,
        );
        assert!(first.complete_detail && !first.cancelled && !first.cache.overview_drawn);
        let first_pixels = pixels(&mut surface);
        let mut cold = self::renderer();
        let mut reference = target();
        cold.render_to_canvas(
            reference.canvas(),
            WIDTH,
            HEIGHT,
            &doc.scene,
            &viewport,
            Some(page),
            &RenderInputs::for_doc(&doc),
        );
        assert_eq!(
            first_pixels,
            pixels(&mut reference),
            "the first visible frame must contain the new pattern color"
        );
        assert_ne!(
            first_pixels, old_pixels,
            "same-counter metadata changes must alter the repeated artwork"
        );
        let final_frame = refine(&mut renderer, &mut surface, &doc, page, &viewport, false);
        assert!(final_frame.complete_detail && !final_frame.needs_refresh);
        cold.render_to_canvas(
            reference.canvas(),
            WIDTH,
            HEIGHT,
            &doc.scene,
            &viewport,
            Some(page),
            &RenderInputs::for_doc(&doc),
        );
        assert_eq!(
            pixels(&mut surface),
            pixels(&mut reference),
            "unsupported retained artwork must preserve current pixels through normal fallback"
        );
    }

    #[test]
    fn camera_zoom_draws_exact_visible_content_without_bucket_or_overview() {
        let (doc, page, _) = fixture();
        let mut renderer = renderer();
        let mut surface = target();
        for zoom in [1.1, 1.2, 0.73] {
            let viewport = Viewport {
                center: [512.0, 240.0],
                zoom,
            };
            let result = renderer.render_camera_to_canvas(
                surface.canvas(),
                WIDTH,
                HEIGHT,
                &doc.scene,
                &viewport,
                Some(page),
                &RenderInputs::for_doc(&doc),
                1,
                true,
            );
            assert!(result.complete_detail && !result.cache.picture_drawn);
            assert!(!result.cache.overview_drawn && result.cache.tiles_built == 0);
            assert_eq!(
                result.cache.scene_walks, 1,
                "a zoom frame draws only the visible content, without a recording"
            );
            let mut reference = target();
            let mut cold = self::renderer();
            cold.render_to_canvas(
                reference.canvas(),
                WIDTH,
                HEIGHT,
                &doc.scene,
                &viewport,
                Some(page),
                &RenderInputs::for_doc(&doc),
            );
            assert_eq!(
                pixels(&mut surface),
                pixels(&mut reference),
                "the first zoom frame must already contain sharp authored pixels"
            );
            refine(&mut renderer, &mut surface, &doc, page, &viewport, true);
            let settled = renderer.render_camera_to_canvas(
                surface.canvas(),
                WIDTH,
                HEIGHT,
                &doc.scene,
                &viewport,
                Some(page),
                &RenderInputs::for_doc(&doc),
                1,
                false,
            );
            assert!(!settled.needs_refresh && settled.cache.tiles_drawn > 0);
            assert_eq!(
                settled.cache.scene_walks, 0,
                "settling quality cannot re-record the same zoom"
            );
        }
    }

    #[test]
    fn camera_visible_recording_culls_offscreen_frames_before_preparing_their_contents() {
        let mut doc = Doc::new();
        let page = add(
            &mut doc,
            None,
            NodeData::Group(GroupNode::default()),
            0.0,
            0.0,
        );
        for index in 0..80 {
            let frame = add(
                &mut doc,
                Some(page),
                NodeData::Group(GroupNode {
                    clip_size: Some([200.0, 200.0]),
                    background: Some(Fill::solid(Color::rgb(30, 40, 50))),
                    ..Default::default()
                }),
                f64::from(index) * 5_000.0,
                0.0,
            );
            for child in 0..16 {
                add(
                    &mut doc,
                    Some(frame),
                    NodeData::Vector(VectorNode::rect_solid(
                        0.0,
                        0.0,
                        8.0,
                        8.0,
                        Color::rgb(80, 120, 180),
                    )),
                    f64::from(child) * 10.0,
                    20.0,
                );
            }
        }
        let mut renderer = renderer();
        let mut surface = target();
        for (center, zoom) in [
            ([100.0, 100.0], 1.0),
            ([5_100.0, 100.0], 1.0),
            ([5_100.0, 100.0], 0.73),
        ] {
            let mut reference = target();
            let mut normal = self::renderer();
            normal.render_to_canvas(
                reference.canvas(),
                WIDTH,
                HEIGHT,
                &doc.scene,
                &Viewport { center, zoom },
                Some(page),
                &RenderInputs::for_doc(&doc),
            );
            // The first frame at a scale draws directly; presenting it again
            // records the visible gap, whose walk must cull the same way.
            for presentation in 0..2 {
                let result = renderer.render_camera_to_canvas(
                    surface.canvas(),
                    WIDTH,
                    HEIGHT,
                    &doc.scene,
                    &Viewport { center, zoom },
                    Some(page),
                    &RenderInputs::for_doc(&doc),
                    1,
                    true,
                );
                assert!(result.complete_detail && !result.cancelled);
                assert!(
                    presentation == 0 || result.cache.tiles_drawn > 0,
                    "a presented scale composites retained tiles"
                );
                assert!(!result.cache.overview_drawn);
                assert!(
                    result.metrics.nodes_visited < 120,
                    "offscreen descendants must not be walked"
                );
                assert!(
                    result.metrics.bounds_prepared < 120,
                    "culling must not conceal whole-page preparation"
                );
                assert_raster_close(
                    &pixels(&mut surface),
                    &pixels(&mut reference),
                    "culled frames match the normal renderer",
                );
            }
        }
    }

    #[test]
    fn camera_cancellation_retains_completed_bounds_without_publishing_partial_pixels() {
        use std::cell::Cell;
        let mut doc = Doc::new();
        let page = add(
            &mut doc,
            None,
            NodeData::Group(GroupNode::default()),
            0.0,
            0.0,
        );
        let group = add(
            &mut doc,
            Some(page),
            NodeData::Group(GroupNode {
                background: Some(Fill::solid(Color::rgb(35, 45, 55))),
                ..Default::default()
            }),
            0.0,
            0.0,
        );
        for index in 0..120 {
            add(
                &mut doc,
                Some(group),
                NodeData::Vector(VectorNode::rect_solid(
                    0.0,
                    0.0,
                    16.0,
                    16.0,
                    Color::rgb(80, 120, 180),
                )),
                f64::from(index) * 20.0,
                0.0,
            );
        }
        let mut renderer = renderer();
        let mut surface = target();
        let viewport = Viewport {
            center: [320.0, 100.0],
            zoom: 1.0,
        };
        // The first frame at a scale draws directly; the next one records.
        let direct = renderer.render_camera_to_canvas(
            surface.canvas(),
            WIDTH,
            HEIGHT,
            &doc.scene,
            &viewport,
            Some(page),
            &RenderInputs::for_doc(&doc),
            1,
            false,
        );
        assert!(direct.complete_detail && direct.cache.tiles_built == 0);
        let checks = Cell::new(0usize);
        let cancelled = renderer.cache.render(
            &mut renderer.renderer,
            surface.canvas(),
            WIDTH,
            HEIGHT,
            &doc.scene,
            &viewport,
            Some(page),
            &RenderInputs::for_doc(&doc),
            1,
            false,
            &|| {
                checks.set(checks.get() + 1);
                checks.get() > 70
            },
        );
        assert!(cancelled.cancelled && !cancelled.complete_detail && !cancelled.needs_refresh);
        assert!(cancelled.metrics.bounds_prepared > 0);
        let retained = renderer.prepared_camera_bounds_count();
        assert!(
            retained > 0 && retained < 120,
            "only finished child unions may survive cancellation"
        );
        let resumed = renderer.render_camera_to_canvas(
            surface.canvas(),
            WIDTH,
            HEIGHT,
            &doc.scene,
            &viewport,
            Some(page),
            &RenderInputs::for_doc(&doc),
            1,
            false,
        );
        assert!(!resumed.cancelled && resumed.complete_detail && resumed.cache.tiles_drawn > 0);
        let mut reference = target();
        let mut normal = self::renderer();
        normal.render_to_canvas(
            reference.canvas(),
            WIDTH,
            HEIGHT,
            &doc.scene,
            &viewport,
            Some(page),
            &RenderInputs::for_doc(&doc),
        );
        assert_eq!(pixels(&mut surface), pixels(&mut reference));
    }

    #[test]
    fn camera_recording_preserves_offscreen_inputs_to_ancestor_filters() {
        for offset_shadow in [false, true] {
            let mut doc = Doc::new();
            let page = add(
                &mut doc,
                None,
                NodeData::Group(GroupNode::default()),
                0.0,
                0.0,
            );
            let parent = doc.scene.get_mut(page).expect("parent");
            parent.blurs.push(fanta_doc::Blur::layer(if offset_shadow {
                64.0
            } else {
                32.0
            }));
            if offset_shadow {
                parent.effects.push(Shadow {
                    kind: ShadowKind::Drop,
                    color: Color::BLACK,
                    blur: 32.0,
                    spread: 0.0,
                    offset: [1_000.0, 0.0],
                    show_behind_node: false,
                });
            }
            add(
                &mut doc,
                Some(page),
                NodeData::Vector(VectorNode::rect_solid(
                    if offset_shadow { -1_050.0 } else { -20.0 },
                    220.0,
                    20.0,
                    40.0,
                    Color::rgb(80, 160, 230),
                )),
                0.0,
                0.0,
            );
            let viewport = Viewport {
                center: [f64::from(WIDTH) * 0.5, f64::from(HEIGHT) * 0.5],
                zoom: 1.0,
            };
            let mut oracle = renderer();
            oracle.set_layer_cache_enabled(false);
            let (picture, _) = oracle
                .record_camera_picture(&doc.scene, 1.0, Some(page), &RenderInputs::for_doc(&doc))
                .expect("complete source oracle");
            let mut reference = target();
            reference.canvas().clear(skia_safe::Color::TRANSPARENT);
            reference.canvas().draw_picture(&picture, None, None);
            let expected = pixels(&mut reference);
            assert!(
                expected.chunks_exact(4).any(|pixel| pixel[3] > 0),
                "an offscreen child must produce visible ancestor-filter pixels"
            );
            let mut normal = renderer();
            normal.set_layer_cache_enabled(false);
            let mut surface = target();
            normal.render_to_canvas(
                surface.canvas(),
                WIDTH,
                HEIGHT,
                &doc.scene,
                &viewport,
                Some(page),
                &RenderInputs::for_doc(&doc),
            );
            assert_eq!(
                pixels(&mut surface),
                expected,
                "viewport culling must retain the ancestor's required source halo"
            );
            let mut cached = renderer();
            // The first frame at a scale draws directly; the second records
            // the visible gap and tiles it.
            for presentation in 0..2 {
                let result = cached.render_camera_to_canvas(
                    surface.canvas(),
                    WIDTH,
                    HEIGHT,
                    &doc.scene,
                    &viewport,
                    Some(page),
                    &RenderInputs::for_doc(&doc),
                    1,
                    false,
                );
                assert!(result.complete_detail && !result.cancelled);
                assert!(presentation == 0 || result.cache.tiles_drawn > 0);
                assert_eq!(
                    pixels(&mut surface),
                    expected,
                    "bounded recording must retain source pixels outside its output coverage"
                );
            }
        }
    }

    #[test]
    fn camera_mask_bounds_preserve_composed_shadow_and_layer_blur_tails() {
        let mut doc = Doc::new();
        let page = add(
            &mut doc,
            None,
            NodeData::Group(GroupNode::default()),
            0.0,
            0.0,
        );
        let mask = add(
            &mut doc,
            Some(page),
            NodeData::Vector(VectorNode::rect_solid(
                0.0,
                0.0,
                f64::from(WIDTH),
                f64::from(HEIGHT),
                Color::WHITE,
            )),
            0.0,
            0.0,
        );
        let mask_node = doc.scene.get_mut(mask).expect("mask");
        mask_node.is_mask = true;
        mask_node.flags.insert(NodeFlags::HIDDEN);
        let content = add(
            &mut doc,
            Some(page),
            NodeData::Vector(VectorNode::rect_solid(
                -2_000.0,
                -1_000.0,
                2_100.0,
                2_000.0,
                Color::WHITE,
            )),
            0.0,
            0.0,
        );
        let content = doc.scene.get_mut(content).expect("filtered content");
        content.effects.push(Shadow {
            kind: ShadowKind::Drop,
            color: Color::WHITE,
            blur: 128.0,
            spread: 0.0,
            offset: [0.0, 0.0],
            show_behind_node: false,
        });
        content.blurs.push(fanta_doc::Blur::layer(128.0));
        let viewport = Viewport {
            center: [f64::from(WIDTH) * 0.5, f64::from(HEIGHT) * 0.5],
            zoom: 1.0,
        };
        let mut unmasked = renderer();
        unmasked.set_layer_cache_enabled(false);
        let mut reference = target();
        unmasked.render_to_canvas(
            reference.canvas(),
            WIDTH,
            HEIGHT,
            &doc.scene,
            &viewport,
            Some(page),
            &RenderInputs::for_doc(&doc),
        );
        let expected = pixels(&mut reference);
        // Each individual filter reaches 192 px, while their composition has
        // visible pixels beyond that reach plus the mask layer's 32 px pad.
        let tail_alpha = (240 * WIDTH as usize + 328) * 4 + 3;
        assert!(
            expected[tail_alpha] > 0,
            "the oracle must paint the composed tail"
        );
        doc.scene
            .get_mut(mask)
            .expect("mask")
            .flags
            .remove(NodeFlags::HIDDEN);
        let mut normal = renderer();
        normal.set_layer_cache_enabled(false);
        let mut surface = target();
        normal.render_to_canvas(
            surface.canvas(),
            WIDTH,
            HEIGHT,
            &doc.scene,
            &viewport,
            Some(page),
            &RenderInputs::for_doc(&doc),
        );
        let masked = pixels(&mut surface);
        assert_eq!(masked[tail_alpha], expected[tail_alpha]);
        assert!(
            masked
                .iter()
                .zip(&expected)
                .all(|(actual, expected)| actual.abs_diff(*expected) <= 1),
            "an opaque viewport mask must preserve the complete filtered source"
        );
        let mut cached = renderer();
        // The first frame at a scale draws directly; the second records the
        // visible gap and tiles it.
        for presentation in 0..2 {
            let result = cached.render_camera_to_canvas(
                surface.canvas(),
                WIDTH,
                HEIGHT,
                &doc.scene,
                &viewport,
                Some(page),
                &RenderInputs::for_doc(&doc),
                1,
                false,
            );
            assert!(result.complete_detail && !result.cancelled);
            assert!(presentation == 0 || result.cache.tiles_drawn > 0);
            assert_eq!(pixels(&mut surface), masked);
        }
    }

    #[test]
    fn camera_cull_bounds_include_the_composed_shadow_and_layer_blur() {
        let mut doc = Doc::new();
        let page = add(
            &mut doc,
            None,
            NodeData::Group(GroupNode::default()),
            0.0,
            0.0,
        );
        let rectangle = add(
            &mut doc,
            Some(page),
            NodeData::Vector(VectorNode::rect_solid(
                0.0,
                0.0,
                60.0,
                60.0,
                Color::rgb(220, 90, 30),
            )),
            500.0,
            120.0,
        );
        let node = doc.scene.get_mut(rectangle).expect("rectangle");
        node.effects.push(Shadow {
            kind: ShadowKind::Drop,
            color: Color::BLACK,
            blur: 128.0,
            spread: 5.0,
            offset: [80.0, -60.0],
            show_behind_node: false,
        });
        node.blurs.push(fanta_doc::Blur::layer(128.0));
        let node = doc.scene.get(rectangle).expect("rectangle");
        let filter = super::super::effects::effects_layer_paint(
            node.opacity.get(),
            node.blend_mode,
            &node.effects,
            &node.blurs,
            false,
            1.0,
        )
        .expect("effects layer")
        .filter
        .expect("composed filter");
        assert!(filter.can_compute_fast_bounds());
        let source = super::super::padded_layer_rect(&Bounds::from_xywh(0.0, 0.0, 60.0, 60.0), 1.0);
        let expected = filter.compute_fast_bounds(source);
        let expected = Rect::new(
            expected.left + 500.0,
            expected.top + 120.0,
            expected.right + 500.0,
            expected.bottom + 120.0,
        );
        let mut renderer = renderer();
        let (_, metrics) = renderer
            .record_camera_picture(&doc.scene, 1.0, Some(page), &RenderInputs::for_doc(&doc))
            .expect("bounded effects");
        // Skia subsequently shrinks its RTree cull to nontransparent draw bounds.
        // The finite coverage supplied before that pass must contain the padded filter input.
        let bounds = bounds_rect(metrics.camera_bounds.expect("recorded complete coverage"));
        assert!(
            bounds.left <= expected.left
                && bounds.top <= expected.top
                && bounds.right >= expected.right
                && bounds.bottom >= expected.bottom,
            "retained coverage {bounds:?} must contain the complete composed-filter output {expected:?}"
        );
        let mut surface = target();
        assert_parity(
            &mut renderer,
            &mut surface,
            &doc,
            page,
            &Viewport {
                center: [512.0, 240.0],
                zoom: 1.0,
            },
        );
    }

    #[test]
    fn camera_recording_refuses_far_backdrops_and_unresolved_assets() {
        let (mut doc, page, rectangle) = fixture();
        let node = doc.scene.get_mut(rectangle).expect("rectangle");
        node.transform = Transform2D::translation(-50_000.0, -50_000.0);
        node.blurs.push(fanta_doc::Blur::background(8.0));
        let mut renderer = renderer();
        assert!(
            renderer
                .record_camera_picture(&doc.scene, 1.0, Some(page), &RenderInputs::for_doc(&doc))
                .is_err(),
            "an empty/nonviewport backdrop read is still unsupported"
        );
        let mut surface = target();
        let viewport = Viewport {
            center: [512.0, 240.0],
            zoom: 1.0,
        };
        let initial = renderer.render_camera_to_canvas(
            surface.canvas(),
            WIDTH,
            HEIGHT,
            &doc.scene,
            &viewport,
            Some(page),
            &RenderInputs::for_doc(&doc),
            1,
            false,
        );
        assert!(initial.complete_detail && initial.cache.tiles_built == 0 && initial.needs_refresh);
        refine(&mut renderer, &mut surface, &doc, page, &viewport, false);
        let near_blur = Viewport {
            center: [-49_600.0, -49_800.0],
            zoom: 1.0,
        };
        let refused = renderer.render_camera_to_canvas(
            surface.canvas(),
            WIDTH,
            HEIGHT,
            &doc.scene,
            &near_blur,
            Some(page),
            &RenderInputs::for_doc(&doc),
            1,
            false,
        );
        assert!(
            refused.cache.scene_walks >= 2,
            "the refused recording is followed by normal rendering"
        );
        assert!(refused.complete_detail && !refused.needs_refresh && !refused.cache.overview_drawn);
        let repeated = renderer.render_camera_to_canvas(
            surface.canvas(),
            WIDTH,
            HEIGHT,
            &doc.scene,
            &near_blur,
            Some(page),
            &RenderInputs::for_doc(&doc),
            1,
            false,
        );
        assert_eq!(
            repeated.cache.scene_walks, 1,
            "only the refused coverage is bypassed"
        );
        let restored = renderer.render_camera_to_canvas(
            surface.canvas(),
            WIDTH,
            HEIGHT,
            &doc.scene,
            &viewport,
            Some(page),
            &RenderInputs::for_doc(&doc),
            1,
            false,
        );
        assert_eq!(restored.cache.scene_walks, 0);
        assert!(
            restored.cache.tiles_drawn > 0,
            "a distant blur cannot disable cached sharp artwork"
        );
        doc.scene
            .get_mut(rectangle)
            .expect("rectangle")
            .blurs
            .clear();
        doc.scene
            .get_mut(rectangle)
            .expect("rectangle")
            .data
            .as_vector_mut()
            .expect("vector")
            .fills = [Fill::Image {
            asset: fanta_doc::AssetId::new(),
            mode: fanta_doc::ImageFitMode::Stretch,
            opacity: 1.0,
            crop: None,
            scale: None,
            rotation: None,
            blend: fanta_doc::BlendMode::Normal,
            adjust: Default::default(),
        }]
        .into_iter()
        .collect();
        assert!(
            renderer
                .record_camera_picture(&doc.scene, 1.0, Some(page), &RenderInputs::for_doc(&doc))
                .is_err(),
            "an unresolved offscreen source must not enter the retained overview"
        );
    }
}
