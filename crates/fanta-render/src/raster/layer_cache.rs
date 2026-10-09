//! Cross-frame cache of rendered **effect layers** — see [`LayerCache`].
//!
//! ## Why
//!
//! Every node that composites through an effects save-layer (a visible layer
//! blur or drop shadow, a non-`Normal` blend, a non-foldable opacity, an
//! isolated group) costs, per frame, an offscreen allocation, its filter passes
//! (a Gaussian is two passes) and a composite — on the GPU, its own render
//! passes. Nothing about that work depends on the viewport *offset*: a pan
//! moves the identical pixels by a number of device pixels. Measured on the
//! Agency template Design page at 10% zoom, ~500 such layers re-rendered per
//! pan step were ~65 of a ~150 ms frame; a pan re-blurred every one of them
//! unchanged.
//!
//! ## What is cached
//!
//! For a live-scene node the walk decides needs a layer
//! ([`begin_effects_layer`](super::begin_effects_layer)'s conditions), the
//! FILTERED layer — the node's own content, its subtree, its foreground and
//! inner shadows, with the drop-shadow / layer-blur filter already applied —
//! is rendered once into an offscreen surface at device resolution and kept
//! as an [`Image`]. The composite step (the layer paint's alpha and blend
//! mode) is applied when the image is drawn, exactly as Skia applies it when
//! restoring the save-layer, so the pixels are the same either way.
//!
//! ## Validity
//!
//! Scene identity, render modes, component metadata and asset ownership guard
//! the whole cache. Within that epoch, `Scene::changes_since` distinguishes
//! transforms from data edits: moving a subtree invalidates its strict
//! ancestors, while data edits also invalidate the node and its descendants.
//! Structural, untracked or expired change history drops everything. Node
//! stamps alone cannot make this distinction because transform-only edits do
//! not stamp ordinary geometry. Layers containing cross-tree pattern sources
//! stay volatile, since their dependencies are not scene ancestors.
//!
//! Per entry, the local→device matrix at render time is kept. A lookup is a
//! **hit** only when the current matrix equals it up to an INTEGER device
//! translation (an integer pan at the same zoom): the image is drawn at the
//! shifted origin with no resampling — pixel-identical to re-rendering, as
//! rasterization is invariant under integer device translation. Anything
//! else — a fractional pan, a zoom — is a miss and re-renders (populating the
//! entry when allowed). There is deliberately no approximate path: compositing
//! a cached 8-bit layer at a fractional offset was measured at up to 3 LSB
//! off a fresh render (two 8-bit roundings around the bilinear resample, for
//! any blur sigma), which is not invisible by this renderer's 1-LSB standard.
//! Interactive hosts that pan by fractional device pixels get integer pans —
//! and hits on every step — by enabling `RasterRenderer::set_pixel_snap_pan`.
//!
//! Layers that render differently for reasons no epoch tracks — an image
//! whose asset is still decoding (placeholder), live media, a 3D model whose
//! camera can move — are marked *volatile* while they paint (see
//! [`super::RenderCtx::layer_volatile`]) and are never stored.
//!
//! ## When it fills
//!
//! Entries are keyed by the frame's effective scale and device phase, so a
//! zoom step or a fractional pan misses everything. To keep a continuous
//! zoom — or a host that pans by fractional device pixels — from paying the
//! (costlier) populate path on every frame only to throw the entries away,
//! populating is enabled only on a frame that repeats the previous frame's
//! content epoch and scale and moves the viewport by whole device pixels (see
//! [`LayerCache::begin_frame`]): the first frame at a new zoom renders
//! directly. Edited nodes and their ancestors cannot refill during that
//! frame; unrelated layers can still hit or fill. The following stable
//! whole-pixel frames (a pan, or a settle repaint)
//! fill the cache at most [`LAYER_CACHE_POPULATE_PER_FRAME`] layers per
//! frame, and pans from then on hit.
use super::effects::EffectsLayerPaint;
use super::{
    AlphaType, Bounds, Canvas, IdHashMap, ImageInfo, NodeId, Paint, Rect, RenderCtx,
    padded_layer_rect,
};
use fanta_doc::{ComponentLibrary, Scene};
use skia_safe::{ColorType, IRect, Matrix, SamplingOptions, Surface};
use std::collections::HashSet;

/// The largest layer, in device pixels, the cache will hold as one entry
/// (4 Mpx ≈ 16 MB RGBA). Bigger layers — a page-spanning blurred backdrop
/// at high zoom — render directly; they are few, and one of them would
/// otherwise evict hundreds of the small layers the cache exists for.
pub(crate) const LAYER_CACHE_MAX_ENTRY_PX: u64 = 4_000_000;

/// Default byte budget for cached layer images (device-resolution RGBA).
/// 256 MB holds every layer of the densest measured 10–25% views with room
/// for a second zoom level; least-recently-used entries go first when the
/// budget is exceeded.
pub(crate) const LAYER_CACHE_DEFAULT_BUDGET_BYTES: usize = 256 << 20;

/// How many layers one frame may render through the populate path. Rendering
/// a layer into its own offscreen costs roughly twice a direct save-layer
/// (measured: the frame that populated all ~600 layers of the densest 10%
/// view took 336 ms against 155 ms direct), so filling the cache in one go
/// would turn the first frame after every zoom step into a visible hitch.
/// Capped, the fill is spread over a handful of frames — each at most ~40 ms
/// over the direct cost, each faster than the last as hits accumulate — and
/// the steady state (every pan a hit) is reached within a second of panning.
pub(crate) const LAYER_CACHE_POPULATE_PER_FRAME: u32 = 128;

/// How far (in device pixels) a translation may sit from an integer and still
/// count as an integer pan. Covers the float rounding of `center · zoom ·
/// scale` between two frames whose device offset is nominally whole.
const INTEGER_PAN_EPS: f32 = 1.0 / 512.0;

/// The frame-wide inputs a cached layer's pixels depend on besides the
/// viewport. Any change drops the whole cache (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LayerEpoch {
    pub(crate) scene_instance: u64,
    pub(crate) mode_generation: u64,
    pub(crate) dark_ui: bool,
    pub(crate) asset_resolver: Option<usize>,
}

/// One node's cached, filtered layer.
pub(crate) struct CachedLayer {
    /// The filtered layer at device resolution (premultiplied, transparent
    /// outside the content) — everything the save-layer held right before
    /// its composite.
    image: skia_safe::Image,
    /// The node's local→device matrix at render time (affine; a perspective
    /// matrix is never cached), as `[a, b, c, d, tx, ty]`.
    matrix: [f32; 6],
    /// Device-pixel top-left of `image` at render time.
    origin: (i32, i32),
    /// Whether `image` is a GPU texture (only drawable on a GPU canvas of the
    /// same context) or CPU pixels.
    texture_backed: bool,
    bytes: usize,
    last_used: u64,
}

/// How a lookup resolved — see [`LayerCache::lookup`].
pub(crate) enum LayerLookup {
    /// Not cached (or stale): render the layer; the caller may store it.
    Miss,
    /// The cached image was drawn in place of the layer.
    Hit,
    SampledHit(VerificationSample),
}

/// The cross-frame effect-layer cache. Owned by the renderer next to the
/// image / path / boolean caches; see the module docs.
pub(crate) struct LayerCache {
    entries: IdHashMap<NodeId, CachedLayer>,
    bytes: usize,
    budget_bytes: usize,
    epoch: Option<LayerEpoch>,
    seen_revision: Option<u64>,
    components: ComponentLibrary,
    hot_nodes: HashSet<NodeId>,
    moved_roots: HashSet<NodeId>,
    /// Frame serial for LRU ordering.
    frame: u64,
    /// The previous frame's effective scale and root device translation —
    /// populating is enabled only when the current frame repeats the scale
    /// and shifts the translation by whole device pixels (see the module
    /// docs and [`Self::begin_frame`]).
    last_frame: Option<(u32, f64, f64)>,
    /// Master switch (renderer API). Off ⇒ every lookup misses and nothing
    /// is stored — the direct save-layer path, byte for byte.
    enabled: bool,
    /// Layers rendered through the populate path so far this frame; capped
    /// at [`LAYER_CACHE_POPULATE_PER_FRAME`] (see [`Self::take_populate_slot`]).
    populated_this_frame: u32,
    /// Whether this frame's canvas is GPU-backed (`Some(true)`), CPU
    /// (`Some(false)`), or could not be probed (`None` — no lookups). A GPU
    /// texture cannot be drawn on a CPU canvas and vice versa; the renderer
    /// serves both kinds, so a lookup only serves entries of the canvas's
    /// kind. Probed per frame by [`Self::begin_frame`].
    canvas_is_gpu: Option<bool>,
    verification: VerificationSchedule,
    #[cfg(test)]
    verification_fault: VerificationFault,
}

impl Default for LayerCache {
    fn default() -> Self {
        Self {
            entries: IdHashMap::default(),
            bytes: 0,
            budget_bytes: LAYER_CACHE_DEFAULT_BUDGET_BYTES,
            epoch: None,
            seen_revision: None,
            components: ComponentLibrary::default(),
            hot_nodes: HashSet::new(),
            moved_roots: HashSet::new(),
            frame: 0,
            last_frame: None,
            enabled: true,
            populated_this_frame: 0,
            canvas_is_gpu: None,
            verification: VerificationSchedule::new(
                std::env::var("FANTA_LAYER_CACHE_VERIFY").is_ok_and(|value| value == "1"),
            ),
            #[cfg(test)]
            verification_fault: VerificationFault::None,
        }
    }
}

impl LayerCache {
    /// Apply the scene delta before any lookup. Unknown history cannot prove
    /// a cached layer is unchanged, so it takes the same cold path as a new epoch.
    ///
    /// Populating is worth its cost (an offscreen per layer, ~2× a direct
    /// save-layer) only when later frames can hit, i.e. when the viewport is
    /// moving by whole device pixels at a fixed zoom. So a frame populates
    /// only if it repeats the previous frame's epoch and `effective_scale`, and its root
    /// device translation `(root_tx, root_ty)` differs from the previous
    /// frame's by an integer (a whole-pixel pan, or no pan). A continuous
    /// zoom, or a host panning by fractional device pixels without
    /// `set_pixel_snap_pan`, therefore never pays for entries it could not
    /// use — the cache is then inert.
    pub(crate) fn begin_frame(
        &mut self,
        canvas: &Canvas,
        scene: &Scene,
        components: &ComponentLibrary,
        epoch: LayerEpoch,
        effective_scale: f32,
        root_translation: (f64, f64),
    ) -> bool {
        let components_changed = self.components != *components;
        let epoch_changed = self.epoch != Some(epoch) || components_changed;
        self.hot_nodes.clear();
        self.moved_roots.clear();
        let delta = (!epoch_changed)
            .then(|| {
                self.seen_revision
                    .and_then(|revision| scene.changes_since(revision))
            })
            .flatten();
        let reset = delta.is_none();
        if reset {
            self.clear();
        } else if let Some(delta) = delta {
            for id in delta.transforms {
                self.moved_roots.insert(id);
                self.hot_nodes.insert(id);
                for ancestor in scene.ancestors_of(id) {
                    self.hot_nodes.insert(ancestor.id);
                    self.remove(ancestor.id);
                }
            }
            for id in delta.nodes {
                for affected in scene
                    .descendants_of(id)
                    .chain(scene.ancestors_of(id).map(|node| node.id))
                {
                    self.hot_nodes.insert(affected);
                    self.remove(affected);
                }
            }
        }
        if components_changed {
            self.components.clone_from(components);
        }
        self.epoch = Some(epoch);
        self.seen_revision = Some(scene.revision());
        self.frame = self.frame.wrapping_add(1);
        self.populated_this_frame = 0;
        let (root_tx, root_ty) = root_translation;
        let scale_bits = effective_scale.to_bits();
        let whole_pixel_pan = |a: f64, b: f64| {
            let d = a - b;
            (d - d.round()).abs() <= f64::from(INTEGER_PAN_EPS)
        };
        let populate = self.enabled
            && !reset
            && self.last_frame.is_some_and(|(bits, tx, ty)| {
                bits == scale_bits && whole_pixel_pan(root_tx, tx) && whole_pixel_pan(root_ty, ty)
            });
        self.last_frame = Some((scale_bits, root_tx, root_ty));
        // Which kind of image this canvas can draw: probe with a 1×1
        // compatible surface (only needed once entries exist to serve; a
        // recording canvas yields no surface and so serves nothing).
        self.canvas_is_gpu = if self.enabled && !self.entries.is_empty() {
            let base = canvas.image_info();
            let info = ImageInfo::new(
                (1, 1),
                base.color_type(),
                AlphaType::Premul,
                base.color_space(),
            );
            canvas
                .new_surface(&info, None)
                .map(|mut s| s.image_snapshot().is_texture_backed())
        } else {
            None
        };
        populate
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
        self.seen_revision = None;
        self.hot_nodes.clear();
        self.moved_roots.clear();
    }

    fn remove(&mut self, id: NodeId) {
        if let Some(entry) = self.entries.remove(&id) {
            self.bytes -= entry.bytes;
        }
    }

    pub(crate) fn may_populate(&self, scene: &Scene, id: NodeId) -> bool {
        !self.hot_nodes.contains(&id)
            && (self.moved_roots.is_empty()
                || !scene
                    .ancestors_of(id)
                    .any(|node| self.moved_roots.contains(&node.id)))
    }

    /// Claim one of this frame's [`LAYER_CACHE_POPULATE_PER_FRAME`] populate
    /// slots; `false` once they are spent (the layer then renders directly
    /// and gets its turn on a later frame).
    pub(crate) fn take_populate_slot(&mut self) -> bool {
        if self.populated_this_frame >= LAYER_CACHE_POPULATE_PER_FRAME {
            return false;
        }
        self.populated_this_frame += 1;
        true
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    pub(crate) fn budget_bytes(&self) -> usize {
        self.budget_bytes
    }

    pub(crate) fn set_budget_bytes(&mut self, bytes: usize) {
        self.budget_bytes = bytes;
        self.evict_to_fit(0);
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.clear();
        }
    }

    /// Try to satisfy `id`'s effects layer from the cache, drawing the cached
    /// image onto `canvas` (under the current clip, with `composite` — the
    /// layer paint's alpha + blend) when the entry is valid for the current
    /// local→device matrix. See the module docs for the hit conditions.
    pub(crate) fn lookup(&mut self, canvas: &Canvas, id: NodeId, composite: &Paint) -> LayerLookup {
        if !self.enabled {
            return LayerLookup::Miss;
        }
        let Some(entry) = self.entries.get_mut(&id) else {
            return LayerLookup::Miss;
        };
        // A GPU texture cannot be drawn on a CPU canvas and vice versa; the
        // renderer serves both, so treat a mismatch as a stale entry.
        if self.canvas_is_gpu != Some(entry.texture_backed) {
            let stale = self.entries.remove(&id).expect("just found");
            self.bytes -= stale.bytes;
            return LayerLookup::Miss;
        }
        let m = canvas.local_to_device_as_3x3();
        if m.has_perspective() {
            return LayerLookup::Miss;
        }
        let [a, b, c, d, tx, ty] = affine_of(&m);
        let [ea, eb, ec, ed, etx, ety] = entry.matrix;
        if a != ea || b != eb || c != ec || d != ed {
            return LayerLookup::Miss;
        }
        let (dx, dy) = (tx - etx, ty - ety);
        let (rx, ry) = (dx.round(), dy.round());
        if (dx - rx).abs() > INTEGER_PAN_EPS || (dy - ry).abs() > INTEGER_PAN_EPS {
            return LayerLookup::Miss;
        }
        entry.last_used = self.frame;
        // Whole-pixel placement, nearest sampling: a 1:1 copy of the cached
        // pixels — the invariance the hit rests on.
        canvas.save();
        canvas.reset_matrix();
        canvas.draw_image_with_sampling_options(
            &entry.image,
            ((entry.origin.0 as f32) + rx, (entry.origin.1 as f32) + ry),
            SamplingOptions::default(),
            Some(composite),
        );
        canvas.restore();
        if self.verification.nominate(self.frame) {
            LayerLookup::SampledHit(VerificationSample {
                image: entry.image.clone(),
                position: ((entry.origin.0 as f32) + rx, (entry.origin.1 as f32) + ry),
                matrix: affine_of(&m),
                frame: self.frame,
                texture_backed: entry.texture_backed,
                #[cfg(test)]
                fault: self.verification_fault,
            })
        } else {
            LayerLookup::Hit
        }
    }

    /// Store a freshly rendered layer image for `id`. `matrix` is the node's
    /// local→device matrix it was rendered under and `origin` the image's
    /// device top-left. Returns whether it was stored (an over-budget or
    /// oversize image is dropped, and the render simply happens again next
    /// frame).
    pub(crate) fn store(
        &mut self,
        id: NodeId,
        image: skia_safe::Image,
        matrix: [f32; 6],
        origin: (i32, i32),
    ) -> bool {
        if !self.enabled {
            return false;
        }
        let bytes = layer_bytes(image.width(), image.height());
        if bytes > self.budget_bytes {
            return false;
        }
        if let Some(old) = self.entries.remove(&id) {
            self.bytes -= old.bytes;
        }
        if !self.evict_to_fit(bytes) {
            return false;
        }
        let texture_backed = image.is_texture_backed();
        self.entries.insert(
            id,
            CachedLayer {
                image,
                matrix,
                origin,
                texture_backed,
                bytes,
                last_used: self.frame,
            },
        );
        self.bytes += bytes;
        true
    }

    #[cfg(test)]
    pub(super) fn verification_test_policy(
        &mut self,
        enabled: bool,
        every_hit: bool,
        fault: VerificationFault,
    ) {
        self.verification = VerificationSchedule::new(enabled);
        self.verification.every_hit = every_hit;
        self.verification_fault = fault;
    }

    #[cfg(test)]
    pub(super) fn verification_test_state(&self) -> (usize, usize, u32, Vec<(NodeId, u32, u64)>) {
        let mut entries: Vec<_> = self
            .entries
            .iter()
            .map(|(id, entry)| (*id, entry.image.unique_id(), entry.last_used))
            .collect();
        entries.sort_unstable();
        (self.len(), self.bytes, self.populated_this_frame, entries)
    }

    #[cfg(test)]
    pub(super) fn verification_poison(&mut self, id: NodeId, move_extent: bool) {
        let entry = self.entries.get_mut(&id).expect("cached test layer");
        if move_extent {
            entry.origin.0 += 80;
        } else {
            let mut surface =
                skia_safe::surfaces::raster_n32_premul((entry.image.width(), entry.image.height()))
                    .expect("poison surface");
            surface.canvas().clear(skia_safe::Color::MAGENTA);
            entry.image = surface.image_snapshot();
        }
    }

    /// Evict least-recently-used entries NOT used this frame until `incoming`
    /// more bytes fit the budget. Returns whether they fit (false when the
    /// entries used this frame alone exceed the budget).
    fn evict_to_fit(&mut self, incoming: usize) -> bool {
        if self.bytes + incoming <= self.budget_bytes {
            return true;
        }
        let mut victims: Vec<(u64, NodeId, usize)> = self
            .entries
            .iter()
            .filter(|(_, e)| e.last_used != self.frame)
            .map(|(id, e)| (e.last_used, *id, e.bytes))
            .collect();
        victims.sort_unstable();
        for (_, id, bytes) in victims {
            if self.bytes + incoming <= self.budget_bytes {
                break;
            }
            self.entries.remove(&id);
            self.bytes -= bytes;
        }
        self.bytes + incoming <= self.budget_bytes
    }
}

/// Render one node's effects layer through an offscreen surface — the layer's
/// `body` (content + subtree + foreground + inner shadows) inside a
/// save-layer carrying the FILTER half of `layer`, at device resolution — then
/// composite the result onto `canvas` with the ALPHA + BLEND half, exactly
/// the two steps Skia performs when restoring the direct save-layer, and
/// store the image in `ctx.layer_cache` for later frames (unless the body
/// proved volatile). Returns `true` when the layer was rendered this way.
///
/// Returns `false` WITHOUT calling `body` — the caller then takes the direct
/// save-layer path — when the layer cannot be cached faithfully or usefully:
/// - the CTM has perspective (the cache keys on an affine matrix);
/// - the current clip is empty (nothing would composite);
/// - its filtered output would exceed [`LAYER_CACHE_MAX_ENTRY_PX`] or the
///   cache budget, or the canvas cannot make a compatible surface (a
///   recording canvas).
///
/// Why the current clip does not otherwise matter: Skia sizes a direct
/// save-layer's content to what can influence its output inside the clip
/// (the clip bounds grown by the filter's reach, ∩ the bounds hint), applies
/// the clip only to the filtered result, and never applies non-rectangular
/// clip shapes to layer content at all. A complete rendering composited under
/// the same clip therefore reproduces the direct output pixel for pixel — and
/// stays complete after a pan moves the clip (the fidelity tests cover a
/// blurred child crossing its clipping frame's edge).
///
/// While the body renders, viewport culling is disabled (`ctx.visible` is
/// widened to [`UNCULLED_WORLD`]) so the cached image is complete whatever
/// the viewport shows — a hit after a pan must not reveal a hole where an
/// off-screen descendant was culled at render time.
pub(crate) fn render_layer_via_cache(
    canvas: &Canvas,
    id: NodeId,
    layer: &EffectsLayerPaint,
    content_bounds: Bounds,
    ctx: &mut RenderCtx,
    body: impl FnOnce(&Canvas, &mut RenderCtx),
) -> bool {
    let m = canvas.local_to_device_as_3x3();
    if m.has_perspective() {
        return false;
    }
    let padded = padded_layer_rect(&content_bounds, ctx.effective_scale);
    // Nothing to composite under an empty clip (fully clipped subtree).
    if canvas.device_clip_bounds().is_none() {
        return false;
    }
    // The filtered layer's reach: a blur bleeds ~3σ past the content, a drop
    // shadow its offset + blur; both known to the filter. No filter (an
    // opacity / blend / isolation layer) ⇒ the padded box itself.
    let out_local = match &layer.filter {
        None => padded,
        Some(filter) if filter.can_compute_fast_bounds() => filter.compute_fast_bounds(padded),
        // A filter whose reach Skia cannot bound would be cropped by any
        // finite offscreen — render it directly.
        Some(_) => return false,
    };
    let Some(dev_out) = device_rect_of(&m, &out_local) else {
        return false;
    };
    let px = (dev_out.width() as u64) * (dev_out.height() as u64);
    if px > LAYER_CACHE_MAX_ENTRY_PX
        || layer_bytes(dev_out.width(), dev_out.height()) > ctx.layer_cache.budget_bytes()
    {
        return false;
    }
    if !ctx.layer_cache.take_populate_slot() {
        return false;
    }
    // A compatible offscreen: the canvas's own color type / color space (a
    // GPU canvas yields a GPU surface on the same context), premultiplied so
    // the layer's transparent surround composites correctly.
    let base = canvas.image_info();
    let info = ImageInfo::new(
        (dev_out.width(), dev_out.height()),
        base.color_type(),
        AlphaType::Premul,
        base.color_space(),
    );
    // The canvas's own surface props (dither etc.); the save-layer pushed
    // inside picks its pixel geometry the same way a direct save-layer does.
    let props = canvas.top_props();
    let Some(mut surface) = canvas.new_surface(&info, Some(&props)) else {
        return false;
    };
    let mut volatile = false;
    paint_filtered_layer(
        surface.canvas(),
        layer,
        padded,
        &m,
        dev_out,
        ctx,
        |canvas, ctx| {
            let outer_visible = std::mem::replace(&mut ctx.visible, UNCULLED_WORLD);
            let outer_volatile = std::mem::replace(&mut ctx.layer_volatile, false);
            body(canvas, ctx);
            ctx.visible = outer_visible;
            volatile = ctx.layer_volatile;
            ctx.layer_volatile = outer_volatile || volatile;
        },
    );
    let image = surface.image_snapshot();

    // Composite: identity CTM (the image is in device pixels), the layer
    // paint's alpha + blend, under the caller's clip.
    canvas.save();
    canvas.reset_matrix();
    canvas.draw_image(
        &image,
        (dev_out.left as f32, dev_out.top as f32),
        Some(&layer.composite_paint()),
    );
    canvas.restore();

    if !volatile {
        ctx.layer_cache
            .store(id, image, affine_of(&m), (dev_out.left, dev_out.top));
    }
    true
}

const VERIFY_MAX_PIXELS: usize = 262_144;
const VERIFY_MAX_BYTES: usize = 4 << 20;

struct VerificationSchedule {
    enabled: bool,
    hits: u64,
    attempts: u32,
    last_frame: Option<u64>,
    logs: u32,
    #[cfg(test)]
    every_hit: bool,
}

impl VerificationSchedule {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            hits: 0,
            attempts: 0,
            last_frame: None,
            logs: 0,
            #[cfg(test)]
            every_hit: false,
        }
    }

    fn nominate(&mut self, frame: u64) -> bool {
        if !self.enabled || self.attempts >= 64 {
            return false;
        }
        self.hits = self.hits.wrapping_add(1);
        #[cfg(test)]
        let forced = self.every_hit;
        #[cfg(not(test))]
        let forced = false;
        if !forced
            && (!self.hits.is_multiple_of(128)
                || self
                    .last_frame
                    .is_some_and(|last| frame.wrapping_sub(last) < 30))
        {
            return false;
        }
        self.last_frame = Some(frame);
        self.attempts += 1;
        true
    }
}

pub(crate) struct VerificationSample {
    image: skia_safe::Image,
    position: (f32, f32),
    matrix: [f32; 6],
    frame: u64,
    texture_backed: bool,
    #[cfg(test)]
    fault: VerificationFault,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum VerificationFault {
    #[default]
    None,
    Surface,
    Buffer,
    Readback,
    Volatile,
    Incomplete,
    Effect,
    NonArtwork,
    Budget,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VerificationFailure {
    Bounds,
    Budget,
    Format,
    Surface,
    Buffer,
    Readback,
    Artwork,
}

#[derive(Debug)]
struct PixelComparison {
    pixels: u64,
    differing_pixels: u64,
    differing_channels: u64,
    max_delta: u8,
    first_mismatch: Option<(usize, usize)>,
}

#[derive(Clone, Copy, Debug)]
struct VerificationRegion {
    bounds: IRect,
    width: usize,
    row_bytes: usize,
    buffer_bytes: usize,
}

impl VerificationRegion {
    fn new(current: IRect, position: (f32, f32), size: (i32, i32)) -> Option<Self> {
        let integer = |value: f32| {
            let value = f64::from(value);
            (value.is_finite()
                && value.fract() == 0.0
                && value >= f64::from(i32::MIN)
                && value <= f64::from(i32::MAX))
            .then_some(value as i64)
        };
        if size.0 <= 0
            || size.1 <= 0
            || current.left >= current.right
            || current.top >= current.bottom
        {
            return None;
        }
        let left = integer(position.0)?;
        let top = integer(position.1)?;
        let right = left.checked_add(i64::from(size.0))?;
        let bottom = top.checked_add(i64::from(size.1))?;
        let left = left.min(i64::from(current.left));
        let top = top.min(i64::from(current.top));
        let right = right.max(i64::from(current.right));
        let bottom = bottom.max(i64::from(current.bottom));
        let width = usize::try_from(right.checked_sub(left)?).ok()?;
        let height = usize::try_from(bottom.checked_sub(top)?).ok()?;
        let pixels = width.checked_mul(height)?;
        let row_bytes = width.checked_mul(4)?;
        let buffer_bytes = row_bytes.checked_mul(height)?;
        if pixels == 0
            || pixels > VERIFY_MAX_PIXELS
            || buffer_bytes.checked_mul(4)? > VERIFY_MAX_BYTES
        {
            return None;
        }
        Some(Self {
            bounds: IRect::from_ltrb(
                i32::try_from(left).ok()?,
                i32::try_from(top).ok()?,
                i32::try_from(right).ok()?,
                i32::try_from(bottom).ok()?,
            ),
            width,
            row_bytes,
            buffer_bytes,
        })
    }
}

fn checked_device_rect(matrix: &Matrix, local: &Rect) -> Option<IRect> {
    let (device, _) = matrix.map_rect(local);
    if !device.is_finite() || matrix.has_perspective() {
        return None;
    }
    let coordinate = |value: f32, lower: bool| {
        let rounded = if lower {
            f64::from(value).floor() - 1.0
        } else {
            f64::from(value).ceil() + 1.0
        };
        (rounded >= f64::from(i32::MIN) && rounded <= f64::from(i32::MAX)).then_some(rounded as i32)
    };
    let result = IRect::from_ltrb(
        coordinate(device.left, true)?,
        coordinate(device.top, true)?,
        coordinate(device.right, false)?,
        coordinate(device.bottom, false)?,
    );
    (result.left < result.right && result.top < result.bottom).then_some(result)
}

fn paint_filtered_layer(
    canvas: &Canvas,
    layer: &EffectsLayerPaint,
    padded: Rect,
    matrix: &Matrix,
    bounds: IRect,
    ctx: &mut RenderCtx,
    body: impl FnOnce(&Canvas, &mut RenderCtx),
) {
    canvas.clear(skia_safe::Color::TRANSPARENT);
    canvas.translate((-(bounds.left as f32), -(bounds.top as f32)));
    canvas.concat(matrix);
    let filter_paint = layer.filter_paint();
    let rec = skia_safe::canvas::SaveLayerRec::default()
        .paint(&filter_paint)
        .bounds(&padded);
    canvas.save_layer(&rec);
    body(canvas, ctx);
    canvas.restore();
}

fn supported_canvas(canvas: &Canvas, texture_backed: bool) -> bool {
    if !texture_backed {
        return canvas.peek_pixels().is_some();
    }
    #[cfg(feature = "metal")]
    {
        canvas.recording_context().is_some()
    }
    #[cfg(not(feature = "metal"))]
    {
        false
    }
}

fn compatible_surface(
    canvas: &Canvas,
    info: &ImageInfo,
    texture_backed: bool,
) -> Result<Surface, VerificationFailure> {
    let props = canvas.top_props();
    let mut surface = canvas
        .new_surface(info, Some(&props))
        .ok_or(VerificationFailure::Surface)?;
    if surface.image_info() != *info
        || surface.recording_context().is_some() != texture_backed
        || (!texture_backed && surface.peek_pixels().is_none())
    {
        return Err(VerificationFailure::Surface);
    }
    Ok(surface)
}

fn comparison_pixels(
    surface: &mut Surface,
    info: &ImageInfo,
    region: VerificationRegion,
) -> Result<Vec<u8>, VerificationFailure> {
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(region.buffer_bytes)
        .map_err(|_| VerificationFailure::Buffer)?;
    pixels.resize(region.buffer_bytes, 0);
    if !surface.read_pixels(info, &mut pixels, region.row_bytes, (0, 0)) {
        return Err(VerificationFailure::Readback);
    }
    Ok(pixels)
}

fn compare_pixels(cached: &[u8], reference: &[u8], width: usize) -> PixelComparison {
    let mut comparison = PixelComparison {
        pixels: (cached.len() / 4) as u64,
        differing_pixels: 0,
        differing_channels: 0,
        max_delta: 0,
        first_mismatch: None,
    };
    for (index, (left, right)) in cached
        .chunks_exact(4)
        .zip(reference.chunks_exact(4))
        .enumerate()
    {
        if left != right {
            comparison.differing_pixels += 1;
            comparison
                .first_mismatch
                .get_or_insert((index % width, index / width));
            for (left, right) in left.iter().zip(right) {
                comparison.differing_channels += u64::from(left != right);
                comparison.max_delta = comparison.max_delta.max(left.abs_diff(*right));
            }
        }
    }
    comparison
}

fn compare_layer(
    canvas: &Canvas,
    layer: &EffectsLayerPaint,
    content_bounds: Option<Bounds>,
    sample: &VerificationSample,
    ctx: &mut RenderCtx,
    body: impl FnOnce(&Canvas, &mut RenderCtx),
) -> Result<PixelComparison, VerificationFailure> {
    let base = canvas.image_info();
    if !matches!(base.color_type(), ColorType::RGBA8888 | ColorType::BGRA8888)
        || base.bytes_per_pixel() != 4
        || !supported_canvas(canvas, sample.texture_backed)
    {
        return Err(VerificationFailure::Format);
    }
    let padded = padded_layer_rect(
        &content_bounds.ok_or(VerificationFailure::Bounds)?,
        ctx.effective_scale,
    );
    if !padded.is_finite() {
        return Err(VerificationFailure::Bounds);
    }
    let output = match &layer.filter {
        None => padded,
        Some(filter) if filter.can_compute_fast_bounds() => filter.compute_fast_bounds(padded),
        Some(_) => return Err(VerificationFailure::Bounds),
    };
    let matrix = canvas.local_to_device_as_3x3();
    let current = checked_device_rect(&matrix, &output).ok_or(VerificationFailure::Bounds)?;
    let region = VerificationRegion::new(
        current,
        sample.position,
        (sample.image.width(), sample.image.height()),
    )
    .ok_or(VerificationFailure::Bounds)?;
    let info = ImageInfo::new(
        (region.bounds.width(), region.bounds.height()),
        base.color_type(),
        AlphaType::Premul,
        base.color_space(),
    );
    #[cfg(test)]
    if sample.fault == VerificationFault::Surface {
        return Err(VerificationFailure::Surface);
    }
    let mut cached = compatible_surface(canvas, &info, sample.texture_backed)?;
    let mut reference = compatible_surface(canvas, &info, sample.texture_backed)?;
    cached.canvas().clear(skia_safe::Color::TRANSPARENT);
    cached.canvas().draw_image_with_sampling_options(
        &sample.image,
        (
            sample.position.0 - region.bounds.left as f32,
            sample.position.1 - region.bounds.top as f32,
        ),
        SamplingOptions::default(),
        None,
    );

    let outer_metrics = std::mem::replace(
        ctx.metrics,
        super::RenderMetrics {
            sampling_budget: Some(crate::sampling::SamplingWorkBudget::default()),
            ..Default::default()
        },
    );
    let outer_visible = std::mem::replace(&mut ctx.visible, UNCULLED_WORLD);
    let outer_lookup = std::mem::replace(&mut ctx.layer_cache_lookups, false);
    let outer_populate = std::mem::replace(&mut ctx.layer_cache_populate, false);
    let outer_volatile = std::mem::replace(&mut ctx.layer_volatile, false);
    paint_filtered_layer(
        reference.canvas(),
        layer,
        padded,
        &matrix,
        region.bounds,
        ctx,
        |canvas, ctx| {
            let entered = ctx
                .metrics
                .sampling_budget
                .as_mut()
                .is_some_and(|budget| budget.enter());
            if entered {
                body(canvas, ctx);
                if let Some(budget) = &mut ctx.metrics.sampling_budget {
                    budget.leave();
                }
            }
        },
    );
    #[cfg(test)]
    match sample.fault {
        VerificationFault::Volatile => ctx.layer_volatile = true,
        VerificationFault::Incomplete => ctx.metrics.incomplete_artwork = true,
        VerificationFault::Effect => ctx.metrics.effect_failed = true,
        VerificationFault::NonArtwork => ctx.metrics.non_artwork_content = true,
        VerificationFault::Budget => {
            if let Some(budget) = &mut ctx.metrics.sampling_budget {
                budget.exhausted = true;
            }
        }
        _ => {}
    }
    let reference_volatile = std::mem::replace(&mut ctx.layer_volatile, outer_volatile);
    ctx.visible = outer_visible;
    ctx.layer_cache_lookups = outer_lookup;
    ctx.layer_cache_populate = outer_populate;
    let reference_metrics = std::mem::replace(ctx.metrics, outer_metrics);
    if reference_metrics
        .sampling_budget
        .as_ref()
        .is_some_and(|budget| budget.exhausted || budget.expansion_exhausted)
    {
        return Err(VerificationFailure::Budget);
    }
    if reference_volatile
        || reference_metrics.incomplete_artwork
        || reference_metrics.non_artwork_content
        || reference_metrics.effect_failed
    {
        return Err(VerificationFailure::Artwork);
    }
    #[cfg(test)]
    if sample.fault == VerificationFault::Buffer {
        return Err(VerificationFailure::Buffer);
    }
    let read_info = ImageInfo::new(
        (region.bounds.width(), region.bounds.height()),
        ColorType::RGBA8888,
        AlphaType::Premul,
        base.color_space(),
    );
    let cached_pixels = comparison_pixels(&mut cached, &read_info, region)?;
    #[cfg(test)]
    if sample.fault == VerificationFault::Readback {
        return Err(VerificationFailure::Readback);
    }
    let reference_pixels = comparison_pixels(&mut reference, &read_info, region)?;
    Ok(compare_pixels(
        &cached_pixels,
        &reference_pixels,
        region.width,
    ))
}

pub(crate) fn verify_layer_hit(
    canvas: &Canvas,
    id: NodeId,
    layer: &EffectsLayerPaint,
    content_bounds: Option<Bounds>,
    sample: VerificationSample,
    ctx: &mut RenderCtx,
    body: impl FnOnce(&Canvas, &mut RenderCtx),
) {
    let result = compare_layer(canvas, layer, content_bounds, &sample, ctx, body);
    ctx.metrics.layer_cache_verify_attempted += 1;
    let failed = match &result {
        Ok(comparison) => {
            ctx.metrics.layer_cache_verify_completed += 1;
            ctx.metrics.layer_cache_verify_pixels += comparison.pixels;
            ctx.metrics.layer_cache_verify_differing_pixels += comparison.differing_pixels;
            ctx.metrics.layer_cache_verify_differing_channels += comparison.differing_channels;
            ctx.metrics.layer_cache_verify_max_delta = ctx
                .metrics
                .layer_cache_verify_max_delta
                .max(comparison.max_delta);
            if comparison.differing_pixels == 0 {
                ctx.metrics.layer_cache_verify_equal += 1;
                false
            } else {
                ctx.metrics.layer_cache_verify_mismatch += 1;
                true
            }
        }
        Err(VerificationFailure::Bounds | VerificationFailure::Budget) => {
            ctx.metrics.layer_cache_verify_skipped += 1;
            true
        }
        Err(_) => {
            ctx.metrics.layer_cache_verify_unavailable += 1;
            true
        }
    };
    if let Err(reason) = &result {
        let index = match reason {
            VerificationFailure::Bounds => 0,
            VerificationFailure::Budget => 1,
            VerificationFailure::Format => 2,
            VerificationFailure::Surface => 3,
            VerificationFailure::Buffer => 4,
            VerificationFailure::Readback => 5,
            VerificationFailure::Artwork => 6,
        };
        ctx.metrics.layer_cache_verify_reasons[index] += 1;
    }
    if failed && ctx.layer_cache.verification.logs < 8 {
        ctx.layer_cache.verification.logs += 1;
        tracing::warn!(node = %id, frame = sample.frame, revision = ctx.scene.revision(),
            gpu = sample.texture_backed, matrix = ?sample.matrix, position = ?sample.position,
            content_bounds = ?content_bounds, result = ?result, "sampled layer cache verification");
    }
}

/// Bytes a `w × h` RGBA8 layer image occupies.
pub(crate) fn layer_bytes(w: i32, h: i32) -> usize {
    (w.max(0) as usize) * (h.max(0) as usize) * 4
}

/// The affine components `[a, b, c, d, tx, ty]` of a (non-perspective) matrix.
pub(crate) fn affine_of(m: &Matrix) -> [f32; 6] {
    [
        m.scale_x(),
        m.skew_y(),
        m.skew_x(),
        m.scale_y(),
        m.translate_x(),
        m.translate_y(),
    ]
}

/// The integer device rect covering `local` (a node-local rect) under `m`,
/// rounded outward with a one-pixel guard on every side for anti-aliasing
/// and mapping round-off. `None` for a degenerate / non-finite result.
pub(crate) fn device_rect_of(m: &Matrix, local: &Rect) -> Option<IRect> {
    let (dev, _) = m.map_rect(local);
    if !dev.is_finite() {
        return None;
    }
    let out = IRect::from_ltrb(
        dev.left.floor() as i32 - 1,
        dev.top.floor() as i32 - 1,
        dev.right.ceil() as i32 + 1,
        dev.bottom.ceil() as i32 + 1,
    );
    (out.width() > 0 && out.height() > 0).then_some(out)
}

/// A world rect no node can miss: used to disable viewport culling while a
/// layer renders into its offscreen, so the cached image is complete
/// regardless of what the viewport currently shows.
pub(crate) const UNCULLED_WORLD: Bounds = Bounds {
    min_x: -1.0e18,
    min_y: -1.0e18,
    max_x: 1.0e18,
    max_y: 1.0e18,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn with_verification_context(test: impl FnOnce(&mut RenderCtx)) {
        let scene = Scene::new();
        let inputs = super::super::RenderInputs::empty();
        let mut images = super::super::ImageCache::default();
        let mut instances = super::super::InstanceCache::default();
        let mut booleans = super::super::BooleanCache::default();
        let mut paths = super::super::PathCache::default();
        let mut patterns = super::super::PatternCache::default();
        let mut layers = LayerCache::default();
        layers.verification = VerificationSchedule::new(false);
        let mut metrics = super::super::RenderMetrics {
            nodes_visited: 19,
            layer_cache_hits: 7,
            incomplete_artwork: true,
            sampling_budget: Some(crate::sampling::SamplingWorkBudget::default()),
            ..Default::default()
        };
        if let Some(budget) = &mut metrics.sampling_budget {
            assert!(budget.enter());
            assert!(budget.reserve_expanded_node());
        }
        let mut ctx = RenderCtx {
            split: None,
            scene: &scene,
            resolver: None,
            cache: &mut images,
            instance_cache: &mut instances,
            boolean_cache: &mut booleans,
            path_cache: &mut paths,
            pattern_cache: &mut patterns,
            pattern_stack: Vec::new(),
            inputs: &inputs,
            instance_mode_anchor: None,
            resolved_local_transforms: IdHashMap::default(),
            resolved_world_transforms: IdHashMap::default(),
            resolved_local_bounds: IdHashMap::default(),
            visible: Bounds {
                min_x: 1.0,
                min_y: 2.0,
                max_x: 3.0,
                max_y: 4.0,
            },
            effective_scale: 1.0,
            page_background_root: None,
            supports_offscreen_layers: true,
            paint_alpha: 1.0,
            layer_cache: &mut layers,
            layer_cache_lookups: true,
            layer_cache_populate: true,
            live_video_fill_subtrees: IdHashMap::default(),
            layer_volatile: true,
            metrics: &mut metrics,
        };
        test(&mut ctx);
    }

    fn verification_sample(fault: VerificationFault) -> VerificationSample {
        VerificationSample {
            image: raster_image(8, 8),
            position: (0.0, 0.0),
            matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            frame: 1,
            texture_backed: false,
            fault,
        }
    }

    #[test]
    fn layer_verification_schedule_and_checked_region_bound_all_attempts() {
        let mut off = VerificationSchedule::new(false);
        for frame in 0..256 {
            assert!(!off.nominate(frame));
        }
        assert_eq!((off.hits, off.attempts, off.last_frame), (0, 0, None));
        let mut schedule = VerificationSchedule::new(true);
        for hit in 1..=128 {
            assert_eq!(schedule.nominate(1), hit == 128);
        }
        for _ in 0..128 {
            assert!(!schedule.nominate(30));
        }
        for hit in 1..=128 {
            assert_eq!(schedule.nominate(31), hit == 128);
        }
        for attempt in 2..64 {
            for hit in 1..=128 {
                assert_eq!(schedule.nominate(attempt * 30 + 1), hit == 128);
            }
        }
        for frame in 2_000..2_500 {
            assert!(!schedule.nominate(frame));
        }
        assert_eq!(schedule.attempts, 64);
        let mut cache = LayerCache::default();
        cache.verification = schedule;
        cache.clear();
        cache.set_enabled(false);
        cache.set_enabled(true);
        assert_eq!(cache.verification.attempts, 64);
        assert!(!cache.verification.nominate(3_000));

        let maximum =
            VerificationRegion::new(IRect::from_ltrb(0, 0, 512, 512), (0.0, 0.0), (512, 512))
                .expect("maximum union");
        assert_eq!(maximum.buffer_bytes * 4, VERIFY_MAX_BYTES);
        for (bounds, position, size) in [
            (IRect::from_ltrb(0, 0, 513, 512), (0.0, 0.0), (1, 1)),
            (
                IRect::from_ltrb(i32::MIN, 0, i32::MAX, 1),
                (0.0, 0.0),
                (1, 1),
            ),
            (IRect::from_ltrb(0, 0, 1, 1), (f32::INFINITY, 0.0), (1, 1)),
            (IRect::from_ltrb(0, 0, 1, 1), (0.5, 0.0), (1, 1)),
            (IRect::from_ltrb(0, 0, 1, 1), (0.0, 0.0), (0, 1)),
            (IRect::from_ltrb(0, 0, 0, 1), (0.0, 0.0), (1, 1)),
        ] {
            assert!(VerificationRegion::new(bounds, position, size).is_none());
        }
        assert!(
            checked_device_rect(
                &Matrix::new_identity(),
                &Rect::from_ltrb(-f32::MAX, 0.0, f32::MAX, 1.0)
            )
            .is_none()
        );
    }

    #[test]
    fn layer_verification_restores_original_metrics_budget_and_flags_on_every_outcome() {
        for outer_volatile in [false, true] {
            for fault in [
                VerificationFault::None,
                VerificationFault::Surface,
                VerificationFault::Buffer,
                VerificationFault::Readback,
                VerificationFault::Volatile,
                VerificationFault::Incomplete,
                VerificationFault::NonArtwork,
                VerificationFault::Effect,
                VerificationFault::Budget,
            ] {
                with_verification_context(|ctx| {
                    ctx.layer_volatile = outer_volatile;
                    let before_metrics = format!("{:?}", ctx.metrics);
                    let before_visible = ctx.visible;
                    let before_cache = ctx.layer_cache.verification_test_state();
                    let mut surface =
                        skia_safe::surfaces::raster_n32_premul((16, 16)).expect("surface");
                    surface
                        .canvas()
                        .clip_rect(Rect::from_xywh(0.0, 0.0, 1.0, 1.0), None, false);
                    let sample = verification_sample(fault);
                    let layer = EffectsLayerPaint {
                        filter: None,
                        opacity: 0.5,
                        blend_mode: super::super::BlendMode::Multiply,
                    };
                    let result = compare_layer(
                        surface.canvas(),
                        &layer,
                        Some(Bounds {
                            min_x: 0.0,
                            min_y: 0.0,
                            max_x: 8.0,
                            max_y: 8.0,
                        }),
                        &sample,
                        ctx,
                        |canvas, ctx| {
                            assert!(
                                !ctx.layer_cache_lookups
                                    && !ctx.layer_cache_populate
                                    && !ctx.layer_volatile
                            );
                            assert_eq!(ctx.visible, UNCULLED_WORLD);
                            assert!(!ctx.metrics.incomplete_artwork);
                            assert_eq!(ctx.metrics.nodes_visited, 0);
                            assert!(ctx.metrics.sampling_budget.is_some());
                            ctx.metrics.nodes_visited += 11;
                            canvas.clear(skia_safe::Color::BLUE);
                        },
                    );
                    match fault {
                        VerificationFault::None => assert!(
                            result.expect("completed comparison").differing_pixels > 1,
                            "comparison must see pixels outside the original one-pixel clip"
                        ),
                        VerificationFault::Surface => {
                            assert!(matches!(result, Err(VerificationFailure::Surface)))
                        }
                        VerificationFault::Buffer => {
                            assert!(matches!(result, Err(VerificationFailure::Buffer)))
                        }
                        VerificationFault::Readback => {
                            assert!(matches!(result, Err(VerificationFailure::Readback)))
                        }
                        VerificationFault::Budget => {
                            assert!(matches!(result, Err(VerificationFailure::Budget)))
                        }
                        _ => assert!(matches!(result, Err(VerificationFailure::Artwork))),
                    }
                    assert_eq!(format!("{:?}", ctx.metrics), before_metrics);
                    assert_eq!(ctx.visible, before_visible);
                    assert!(ctx.layer_cache_lookups && ctx.layer_cache_populate);
                    assert_eq!(ctx.layer_volatile, outer_volatile);
                    assert_eq!(ctx.layer_cache.verification_test_state(), before_cache);
                });
            }
        }
    }

    #[test]
    fn layer_verification_rejects_high_precision_before_paint_and_balances_real_budget_exhaustion()
    {
        with_verification_context(|ctx| {
            let before = format!("{:?}", ctx.metrics);
            let info = ImageInfo::new((8, 8), ColorType::RGBAF16, AlphaType::Premul, None);
            let mut surface =
                skia_safe::surfaces::raster(&info, None, None).expect("high precision surface");
            let layer = EffectsLayerPaint {
                filter: None,
                opacity: 1.0,
                blend_mode: super::super::BlendMode::Normal,
            };
            let bounds = Some(Bounds {
                min_x: 0.0,
                min_y: 0.0,
                max_x: 8.0,
                max_y: 8.0,
            });
            let result = compare_layer(
                surface.canvas(),
                &layer,
                bounds,
                &verification_sample(VerificationFault::None),
                ctx,
                |_, _| panic!("unsupported format must refuse before painting/allocation"),
            );
            assert!(matches!(result, Err(VerificationFailure::Format)));
            let mut surface = skia_safe::surfaces::raster_n32_premul((8, 8)).expect("surface");
            let result = compare_layer(
                surface.canvas(),
                &layer,
                bounds,
                &verification_sample(VerificationFault::None),
                ctx,
                |_, ctx| {
                    let budget = ctx.metrics.sampling_budget.as_mut().expect("fresh budget");
                    let mut entered = 0;
                    while budget.enter() {
                        entered += 1;
                    }
                    assert_eq!(entered, 63, "reference root occupies the first depth slot");
                    for _ in 0..entered {
                        budget.leave();
                    }
                },
            );
            assert!(matches!(result, Err(VerificationFailure::Budget)));
            assert_eq!(format!("{:?}", ctx.metrics), before);
        });
    }

    #[test]
    fn layer_verification_preserves_near_integer_hit_eligibility() {
        let mut surface = skia_safe::surfaces::raster_n32_premul((16, 16)).expect("surface");
        let canvas = surface.canvas();
        let scene = Scene::new();
        let components = ComponentLibrary::default();
        let mut cache = LayerCache::default();
        cache.verification = VerificationSchedule::new(true);
        cache.verification.every_hit = true;
        cache.begin_frame(
            canvas,
            &scene,
            &components,
            epoch(&scene, 0),
            1.0,
            (0.0, 0.0),
        );
        let id = NodeId::new();
        assert!(cache.store(
            id,
            raster_image(8, 8),
            [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            (0, 0)
        ));
        cache.begin_frame(
            canvas,
            &scene,
            &components,
            epoch(&scene, 0),
            1.0,
            (0.0, 0.0),
        );
        canvas.translate((1.0 + INTEGER_PAN_EPS * 0.5, 0.0));
        assert!(matches!(
            cache.lookup(canvas, id, &Paint::default()),
            LayerLookup::SampledHit(_)
        ));
        canvas.reset_matrix();
        canvas.translate((1.0 + INTEGER_PAN_EPS * 2.0, 0.0));
        assert!(matches!(
            cache.lookup(canvas, id, &Paint::default()),
            LayerLookup::Miss
        ));
        assert_eq!(cache.verification.attempts, 1);
    }

    #[test]
    fn layer_verification_limits_failure_logs_without_losing_counts() {
        with_verification_context(|ctx| {
            let mut surface = skia_safe::surfaces::raster_n32_premul((8, 8)).expect("surface");
            let layer = EffectsLayerPaint {
                filter: None,
                opacity: 1.0,
                blend_mode: super::super::BlendMode::Normal,
            };
            for _ in 0..12 {
                verify_layer_hit(
                    surface.canvas(),
                    NodeId::new(),
                    &layer,
                    Some(Bounds {
                        min_x: 0.0,
                        min_y: 0.0,
                        max_x: 8.0,
                        max_y: 8.0,
                    }),
                    verification_sample(VerificationFault::Surface),
                    ctx,
                    |_, _| panic!("failed allocation"),
                );
            }
            assert_eq!(ctx.layer_cache.verification.logs, 8);
            assert_eq!(ctx.metrics.layer_cache_verify_attempted, 12);
            assert_eq!(ctx.metrics.layer_cache_verify_unavailable, 12);
            assert_eq!(ctx.metrics.layer_cache_verify_reasons[3], 12);
            assert_eq!(ctx.metrics.layer_cache_verify_completed, 0);
        });
    }

    fn raster_image(w: i32, h: i32) -> skia_safe::Image {
        let info = ImageInfo::new_n32_premul((w, h), None);
        let mut surface = skia_safe::surfaces::raster(&info, None, None).unwrap();
        surface.canvas().clear(skia_safe::Color::RED);
        surface.image_snapshot()
    }

    fn epoch(scene: &Scene, mode_generation: u64) -> LayerEpoch {
        LayerEpoch {
            scene_instance: scene.instance_id(),
            mode_generation,
            dark_ui: false,
            asset_resolver: None,
        }
    }

    /// The budget is honoured by evicting the least-recently-used entries
    /// that were NOT used this frame; an entry larger than the budget is
    /// refused; entries touched this frame are never evicted for a newcomer.
    #[test]
    fn budget_evicts_least_recently_used_entries_first() {
        let mut probe = skia_safe::surfaces::raster_n32_premul((4, 4)).unwrap();
        let canvas = probe.canvas();
        let mut cache = LayerCache::default();
        let scene = Scene::new();
        let components = ComponentLibrary::default();
        cache.set_budget_bytes(3 * layer_bytes(10, 10));
        let ids: Vec<NodeId> = (0..4).map(|_| NodeId::new()).collect();
        let m = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

        for id in &ids[..3] {
            cache.begin_frame(
                canvas,
                &scene,
                &components,
                epoch(&scene, 1),
                1.0,
                (0.0, 0.0),
            );
            assert!(cache.store(*id, raster_image(10, 10), m, (0, 0)));
        }
        assert_eq!(cache.len(), 3);
        // A fourth entry evicts the least recently used until it fits: only
        // ids[0] (frame 1) must go.
        cache.begin_frame(
            canvas,
            &scene,
            &components,
            epoch(&scene, 1),
            1.0,
            (0.0, 0.0),
        );
        assert!(cache.store(ids[3], raster_image(10, 10), m, (0, 0)));
        assert_eq!(cache.len(), 3);
        assert!(!cache.entries.contains_key(&ids[0]));
        assert!(cache.entries.contains_key(&ids[1]));
        assert_eq!(cache.bytes(), 3 * layer_bytes(10, 10));

        // Larger than the whole budget: refused, nothing evicted.
        assert!(!cache.store(NodeId::new(), raster_image(40, 40), m, (0, 0)));
        assert_eq!(cache.len(), 3);

        // Entries used THIS frame are not evicted to make room: touch all
        // three via lookups (a hit needs the canvas kind probe, so start a
        // frame with entries present), then a newcomer that needs space is
        // refused rather than evicting a live entry.
        cache.begin_frame(
            canvas,
            &scene,
            &components,
            epoch(&scene, 1),
            1.0,
            (0.0, 0.0),
        );
        let paint = Paint::default();
        for id in &ids[1..4] {
            assert!(matches!(
                cache.lookup(canvas, *id, &paint),
                LayerLookup::Hit
            ));
        }
        assert!(!cache.store(NodeId::new(), raster_image(10, 10), m, (0, 0)));
        assert_eq!(cache.len(), 3);

        // A new epoch drops everything.
        cache.begin_frame(
            canvas,
            &scene,
            &components,
            epoch(&scene, 2),
            1.0,
            (0.0, 0.0),
        );
        assert_eq!((cache.len(), cache.bytes()), (0, 0));
    }

    /// Each frame hands out at most `LAYER_CACHE_POPULATE_PER_FRAME` populate
    /// slots, and a new frame refills them.
    #[test]
    fn populate_slots_are_capped_per_frame() {
        let mut probe = skia_safe::surfaces::raster_n32_premul((4, 4)).unwrap();
        let canvas = probe.canvas();
        let mut cache = LayerCache::default();
        let scene = Scene::new();
        let components = ComponentLibrary::default();
        cache.begin_frame(
            canvas,
            &scene,
            &components,
            epoch(&scene, 1),
            1.0,
            (0.0, 0.0),
        );
        for _ in 0..LAYER_CACHE_POPULATE_PER_FRAME {
            assert!(cache.take_populate_slot());
        }
        assert!(!cache.take_populate_slot());
        cache.begin_frame(
            canvas,
            &scene,
            &components,
            epoch(&scene, 1),
            1.0,
            (0.0, 0.0),
        );
        assert!(cache.take_populate_slot());
    }

    /// Populating is allowed only on a frame whose scale repeats the previous
    /// frame's; disabling clears and blocks stores.
    #[test]
    fn populate_requires_a_repeated_scale_and_enable_gates_everything() {
        let mut probe = skia_safe::surfaces::raster_n32_premul((4, 4)).unwrap();
        let canvas = probe.canvas();
        let mut cache = LayerCache::default();
        let scene = Scene::new();
        let components = ComponentLibrary::default();
        assert!(!cache.begin_frame(
            canvas,
            &scene,
            &components,
            epoch(&scene, 1),
            0.5,
            (10.0, 20.0)
        ));
        assert!(cache.begin_frame(
            canvas,
            &scene,
            &components,
            epoch(&scene, 1),
            0.5,
            (10.0, 20.0)
        ));
        // Whole-pixel pans keep populating; a fractional one does not.
        assert!(cache.begin_frame(
            canvas,
            &scene,
            &components,
            epoch(&scene, 1),
            0.5,
            (13.0, 18.0)
        ));
        assert!(!cache.begin_frame(
            canvas,
            &scene,
            &components,
            epoch(&scene, 1),
            0.5,
            (13.5, 18.0)
        ));
        assert!(!cache.begin_frame(
            canvas,
            &scene,
            &components,
            epoch(&scene, 1),
            0.5,
            (13.5, 18.25)
        ));
        assert!(cache.begin_frame(
            canvas,
            &scene,
            &components,
            epoch(&scene, 1),
            0.5,
            (14.5, 17.25)
        ));
        // A zoom step: direct first, then populate again.
        assert!(!cache.begin_frame(
            canvas,
            &scene,
            &components,
            epoch(&scene, 1),
            0.25,
            (14.5, 17.25)
        ));
        assert!(cache.begin_frame(
            canvas,
            &scene,
            &components,
            epoch(&scene, 1),
            0.25,
            (14.5, 17.25)
        ));
        let m = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        assert!(cache.store(NodeId::new(), raster_image(2, 2), m, (0, 0)));
        cache.set_enabled(false);
        assert_eq!(cache.len(), 0);
        assert!(!cache.begin_frame(
            canvas,
            &scene,
            &components,
            epoch(&scene, 1),
            0.25,
            (14.5, 17.25)
        ));
        assert!(!cache.store(NodeId::new(), raster_image(2, 2), m, (0, 0)));
    }

    #[test]
    fn integer_pan_epsilon_is_sub_pixel_but_covers_float_rounding() {
        // 80 device px of pan computed as 0.2 · 400 in f32 lands within eps.
        let panned = 0.2f32 * 400.0;
        assert!((panned - panned.round()).abs() <= INTEGER_PAN_EPS);
        assert!(INTEGER_PAN_EPS < 0.01);
    }

    #[test]
    fn device_rect_rounds_out_with_a_pixel_guard() {
        let m = Matrix::scale((2.0, 2.0));
        let r = device_rect_of(&m, &Rect::from_xywh(1.25, 2.5, 3.0, 4.0)).unwrap();
        // 2.5..8.5 → floor 2 − 1 .. ceil 9 + 1 ; 5..13 → 4..14
        assert_eq!((r.left, r.top, r.right, r.bottom), (1, 4, 10, 14));
        assert!(device_rect_of(&m, &Rect::from_xywh(0.0, 0.0, 0.0, 0.0)).is_some());
        assert!(device_rect_of(&m, &Rect::from_xywh(f32::NAN, 0.0, 1.0, 1.0)).is_none());
    }
}
