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
//! One [`LayerEpoch`] guards the whole cache: the scene instance + revision,
//! the render inputs' `mode_generation`, and `dark_ui`. Any content edit
//! moves one of them and the cache is dropped wholesale — it is a NAVIGATION
//! cache, correct because nothing a pan or zoom does can change a layer's
//! pixels except its device placement, and cheap because that is the only
//! invalidation it needs. (Per-node stamps are deliberately not used: a
//! transform-only drag does not stamp the moved node — see
//! `Scene::set_transform` — so a subtree stamp would not be a sound key.)
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
//! scale and moves the viewport by whole device pixels (see
//! [`LayerCache::begin_frame`]): the first frame at a new zoom renders
//! directly, the following whole-pixel frames (a pan, or a settle repaint)
//! fill the cache at most [`LAYER_CACHE_POPULATE_PER_FRAME`] layers per
//! frame, and pans from then on hit.
use super::effects::EffectsLayerPaint;
use super::{
    AlphaType, Bounds, Canvas, IdHashMap, ImageInfo, NodeId, Paint, Rect, RenderCtx,
    padded_layer_rect,
};
use skia_safe::{IRect, Matrix, SamplingOptions};

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
    pub(crate) scene_revision: u64,
    pub(crate) mode_generation: u64,
    pub(crate) dark_ui: bool,
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
}

/// The cross-frame effect-layer cache. Owned by the renderer next to the
/// image / path / boolean caches; see the module docs.
pub(crate) struct LayerCache {
    entries: IdHashMap<NodeId, CachedLayer>,
    bytes: usize,
    budget_bytes: usize,
    epoch: Option<LayerEpoch>,
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
}

impl Default for LayerCache {
    fn default() -> Self {
        Self {
            entries: IdHashMap::default(),
            bytes: 0,
            budget_bytes: LAYER_CACHE_DEFAULT_BUDGET_BYTES,
            epoch: None,
            frame: 0,
            last_frame: None,
            enabled: true,
            populated_this_frame: 0,
            canvas_is_gpu: None,
        }
    }
}

impl LayerCache {
    /// Start a frame: drop everything if the epoch moved, advance the LRU
    /// clock, and report whether this frame may POPULATE the cache. Lookups
    /// are allowed regardless.
    ///
    /// Populating is worth its cost (an offscreen per layer, ~2× a direct
    /// save-layer) only when later frames can hit, i.e. when the viewport is
    /// moving by whole device pixels at a fixed zoom. So a frame populates
    /// only if it repeats the previous frame's `effective_scale` AND its root
    /// device translation `(root_tx, root_ty)` differs from the previous
    /// frame's by an integer (a whole-pixel pan, or no pan). A continuous
    /// zoom, or a host panning by fractional device pixels without
    /// `set_pixel_snap_pan`, therefore never pays for entries it could not
    /// use — the cache is then inert.
    pub(crate) fn begin_frame(
        &mut self,
        canvas: &Canvas,
        epoch: LayerEpoch,
        effective_scale: f32,
        root_tx: f64,
        root_ty: f64,
    ) -> bool {
        if self.epoch != Some(epoch) {
            self.clear();
            self.epoch = Some(epoch);
        }
        self.frame = self.frame.wrapping_add(1);
        self.populated_this_frame = 0;
        let scale_bits = effective_scale.to_bits();
        let whole_pixel_pan = |a: f64, b: f64| {
            let d = a - b;
            (d - d.round()).abs() <= f64::from(INTEGER_PAN_EPS)
        };
        let populate = self.enabled
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
        LayerLookup::Hit
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
    let offscreen = surface.canvas();
    offscreen.clear(skia_safe::Color::TRANSPARENT);
    // The node's local space, shifted so the output rect's corner is (0, 0).
    offscreen.translate((-(dev_out.left as f32), -(dev_out.top as f32)));
    offscreen.concat(&m);
    let filter_paint = layer.filter_paint();
    let rec = skia_safe::canvas::SaveLayerRec::default()
        .paint(&filter_paint)
        .bounds(&padded);
    offscreen.save_layer(&rec);
    let outer_visible = std::mem::replace(&mut ctx.visible, UNCULLED_WORLD);
    let outer_volatile = std::mem::replace(&mut ctx.layer_volatile, false);
    body(offscreen, ctx);
    ctx.visible = outer_visible;
    let volatile = ctx.layer_volatile;
    ctx.layer_volatile = outer_volatile || volatile;
    offscreen.restore();
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

    fn raster_image(w: i32, h: i32) -> skia_safe::Image {
        let info = ImageInfo::new_n32_premul((w, h), None);
        let mut surface = skia_safe::surfaces::raster(&info, None, None).unwrap();
        surface.canvas().clear(skia_safe::Color::RED);
        surface.image_snapshot()
    }

    fn epoch(revision: u64) -> LayerEpoch {
        LayerEpoch {
            scene_instance: 1,
            scene_revision: revision,
            mode_generation: 0,
            dark_ui: false,
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
        cache.set_budget_bytes(3 * layer_bytes(10, 10));
        let ids: Vec<NodeId> = (0..4).map(|_| NodeId::new()).collect();
        let m = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

        for id in &ids[..3] {
            cache.begin_frame(canvas, epoch(1), 1.0, 0.0, 0.0);
            assert!(cache.store(*id, raster_image(10, 10), m, (0, 0)));
        }
        assert_eq!(cache.len(), 3);
        // A fourth entry evicts the least recently used until it fits: only
        // ids[0] (frame 1) must go.
        cache.begin_frame(canvas, epoch(1), 1.0, 0.0, 0.0);
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
        cache.begin_frame(canvas, epoch(1), 1.0, 0.0, 0.0);
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
        cache.begin_frame(canvas, epoch(2), 1.0, 0.0, 0.0);
        assert_eq!((cache.len(), cache.bytes()), (0, 0));
    }

    /// Each frame hands out at most `LAYER_CACHE_POPULATE_PER_FRAME` populate
    /// slots, and a new frame refills them.
    #[test]
    fn populate_slots_are_capped_per_frame() {
        let mut probe = skia_safe::surfaces::raster_n32_premul((4, 4)).unwrap();
        let canvas = probe.canvas();
        let mut cache = LayerCache::default();
        cache.begin_frame(canvas, epoch(1), 1.0, 0.0, 0.0);
        for _ in 0..LAYER_CACHE_POPULATE_PER_FRAME {
            assert!(cache.take_populate_slot());
        }
        assert!(!cache.take_populate_slot());
        cache.begin_frame(canvas, epoch(1), 1.0, 0.0, 0.0);
        assert!(cache.take_populate_slot());
    }

    /// Populating is allowed only on a frame whose scale repeats the previous
    /// frame's; disabling clears and blocks stores.
    #[test]
    fn populate_requires_a_repeated_scale_and_enable_gates_everything() {
        let mut probe = skia_safe::surfaces::raster_n32_premul((4, 4)).unwrap();
        let canvas = probe.canvas();
        let mut cache = LayerCache::default();
        assert!(!cache.begin_frame(canvas, epoch(1), 0.5, 10.0, 20.0));
        assert!(cache.begin_frame(canvas, epoch(1), 0.5, 10.0, 20.0));
        // Whole-pixel pans keep populating; a fractional one does not.
        assert!(cache.begin_frame(canvas, epoch(1), 0.5, 13.0, 18.0));
        assert!(!cache.begin_frame(canvas, epoch(1), 0.5, 13.5, 18.0));
        assert!(!cache.begin_frame(canvas, epoch(1), 0.5, 13.5, 18.25));
        assert!(cache.begin_frame(canvas, epoch(1), 0.5, 14.5, 17.25));
        // A zoom step: direct first, then populate again.
        assert!(!cache.begin_frame(canvas, epoch(1), 0.25, 14.5, 17.25));
        assert!(cache.begin_frame(canvas, epoch(1), 0.25, 14.5, 17.25));
        let m = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        assert!(cache.store(NodeId::new(), raster_image(2, 2), m, (0, 0)));
        cache.set_enabled(false);
        assert_eq!(cache.len(), 0);
        assert!(!cache.begin_frame(canvas, epoch(1), 0.25, 14.5, 17.25));
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
