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
//! GPU entries also require the same live recorder/context. Hosts must clear
//! caches before replacing a backend: a freed recorder's address can be reused.
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
use fanta_doc::{ComponentDef, ComponentId, ComponentLibrary, ComponentSet, Scene};
use skia_safe::{IRect, Matrix, SamplingOptions};
use std::collections::{BTreeMap, HashSet};

#[derive(Default)]
pub(crate) struct ComponentDependencies {
    definitions: BTreeMap<ComponentId, Option<ComponentDef>>,
    sets: BTreeMap<ComponentId, Option<ComponentSet>>,
    bytes: usize,
    incomplete: bool,
}

pub(crate) struct CacheByteCounter(pub(crate) usize);

impl std::io::Write for CacheByteCounter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl ComponentDependencies {
    pub(crate) fn matches(&self, components: &ComponentLibrary) -> bool {
        !self.incomplete
            && self
                .definitions
                .iter()
                .all(|(id, definition)| definition.as_ref() == components.defs.get(id))
            && self
                .sets
                .iter()
                .all(|(id, set)| set.as_ref() == components.sets.get(id))
    }

    pub(crate) fn changed_references(
        &self,
        components: &ComponentLibrary,
    ) -> Option<HashSet<ComponentId>> {
        if self.incomplete {
            return None;
        }
        let mut changed: HashSet<_> = self
            .definitions
            .iter()
            .filter(|(id, definition)| definition.as_ref() != components.defs.get(id))
            .map(|(id, _)| *id)
            .collect();
        for (id, set) in &self.sets {
            if set.as_ref() != components.sets.get(id)
                || set.as_ref().is_some_and(|set| {
                    set.members
                        .iter()
                        .chain(std::iter::once(&set.default_variant))
                        .any(|member| changed.contains(member))
                })
            {
                changed.insert(*id);
            }
        }
        Some(changed)
    }

    pub(crate) fn record(&mut self, components: &ComponentLibrary, reference: ComponentId) {
        if self.definitions.contains_key(&reference)
            && (components.defs.contains_key(&reference) || self.sets.contains_key(&reference))
        {
            return;
        }
        if !self.record_definition(components, reference) {
            return;
        }
        if components.defs.contains_key(&reference) {
            return;
        }
        let set = components.sets.get(&reference);
        let mut counter = CacheByteCounter(0);
        if serde_json::to_writer(&mut counter, &set).is_err() || !self.reserve(counter.0) {
            self.incomplete = true;
            return;
        }
        self.sets.insert(reference, set.cloned());
        if let Some(set) = components.sets.get(&reference) {
            // Selection reads every member's property schema and membership,
            // including candidates which are not the selected master.
            for id in set
                .members
                .iter()
                .chain(std::iter::once(&set.default_variant))
            {
                if !self.record_definition(components, *id) {
                    break;
                }
            }
        }
    }

    fn record_definition(&mut self, components: &ComponentLibrary, id: ComponentId) -> bool {
        if self.definitions.contains_key(&id) {
            return true;
        }
        let definition = components.defs.get(&id);
        let mut counter = CacheByteCounter(0);
        if serde_json::to_writer(&mut counter, &definition).is_err() || !self.reserve(counter.0) {
            self.incomplete = true;
            return false;
        }
        self.definitions.insert(id, definition.cloned());
        true
    }

    fn reserve(&mut self, serialized_bytes: usize) -> bool {
        let bytes = serialized_bytes.saturating_mul(4).saturating_add(256);
        if self.definitions.len().saturating_add(self.sets.len()) >= 4_096
            || self.bytes.saturating_add(bytes) > 16 << 20
        {
            self.incomplete = true;
            return false;
        }
        self.bytes += bytes;
        true
    }

    pub(crate) fn clear(&mut self) {
        self.definitions.clear();
        self.sets.clear();
        self.bytes = 0;
        self.incomplete = false;
    }
}

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
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LayerCacheDomain {
    Raster,
    #[cfg(feature = "metal")]
    Ganesh(skia_safe::gpu::DirectContextId),
    #[cfg(feature = "metal")]
    Graphite(usize),
}

/// The cross-frame effect-layer cache. Owned by the renderer next to the
/// image / path / boolean caches; see the module docs.
pub(crate) struct LayerCache {
    entries: IdHashMap<NodeId, CachedLayer>,
    bytes: usize,
    budget_bytes: usize,
    epoch: Option<LayerEpoch>,
    seen_revision: Option<u64>,
    components: ComponentDependencies,
    hot_nodes: HashSet<NodeId>,
    moved_roots: HashSet<NodeId>,
    /// Frame serial for LRU ordering.
    frame: u64,
    /// The previous frame's effective scale and root device translation —
    /// populating is enabled only when the current frame repeats the scale
    /// and shifts the translation by whole device pixels (see the module
    /// docs and [`Self::begin_frame`]).
    last_frame: Option<([u32; 2], f64, f64)>,
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
    domain: Option<LayerCacheDomain>,
}

impl Default for LayerCache {
    fn default() -> Self {
        Self {
            entries: IdHashMap::default(),
            bytes: 0,
            budget_bytes: LAYER_CACHE_DEFAULT_BUDGET_BYTES,
            epoch: None,
            seen_revision: None,
            components: ComponentDependencies::default(),
            hot_nodes: HashSet::new(),
            moved_roots: HashSet::new(),
            frame: 0,
            last_frame: None,
            enabled: true,
            populated_this_frame: 0,
            canvas_is_gpu: None,
            domain: None,
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
        effective_scale: impl Into<super::effects::EffectScale>,
        root_translation: (f64, f64),
    ) -> bool {
        let components_changed = !self.components.matches(components);
        let domain = canvas_cache_domain(canvas);
        let domain_changed = domain.is_none() || self.domain != domain;
        let epoch_changed = self.epoch != Some(epoch) || components_changed || domain_changed;
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
        self.epoch = Some(epoch);
        self.domain = domain;
        self.seen_revision = Some(scene.revision());
        self.frame = self.frame.wrapping_add(1);
        self.populated_this_frame = 0;
        let (root_tx, root_ty) = root_translation;
        let scale_bits = effective_scale.into().axes().map(f32::to_bits);
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
            canvas_backend_is_gpu(canvas, &info)
        } else {
            None
        };
        populate
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
        self.components.clear();
        self.seen_revision = None;
        self.hot_nodes.clear();
        self.moved_roots.clear();
    }

    pub(crate) fn record_component(
        &mut self,
        components: &ComponentLibrary,
        reference: ComponentId,
    ) {
        self.components.record(components, reference);
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
/// Returns `false` before painting when the layer cannot be cached faithfully
/// or usefully; a failed snapshot after offscreen painting also returns `false`
/// so the caller can paint the complete layer directly:
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
    let padded = padded_layer_rect(&content_bounds, ctx.effective_scale_axes);
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
    let props = top_surface_props(canvas);
    let Some(mut surface) = compatible_offscreen_surface(canvas, &info, Some(&props)) else {
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
    let Some(image) = immutable_surface_snapshot(&mut surface) else {
        return false;
    };

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

fn compatible_offscreen_surface(
    canvas: &Canvas,
    info: &ImageInfo,
    properties: Option<&skia_safe::SurfaceProps>,
) -> Option<skia_safe::Surface> {
    let surface = canvas.new_surface(info, properties)?;
    #[cfg(feature = "metal")]
    {
        use skia_safe::wrapper::PointerWrapper;
        let mut surface = surface;
        match (canvas.recorder(), surface.canvas().recorder()) {
            (Some(parent), Some(child)) if std::ptr::eq(parent.inner(), child.inner()) => {}
            (None, None) => {}
            _ => return None,
        }
        Some(surface)
    }
    #[cfg(not(feature = "metal"))]
    Some(surface)
}

fn canvas_backend_is_gpu(canvas: &Canvas, info: &ImageInfo) -> Option<bool> {
    #[cfg(feature = "metal")]
    if canvas.recorder().is_some() {
        return Some(true);
    }
    let mut surface = compatible_offscreen_surface(canvas, info, None)?;
    immutable_surface_snapshot(&mut surface).map(|image| image.is_texture_backed())
}

fn canvas_cache_domain(_canvas: &Canvas) -> Option<LayerCacheDomain> {
    #[cfg(feature = "metal")]
    {
        let canvas = _canvas;
        use skia_safe::wrapper::PointerWrapper;
        if let Some(recorder) = canvas.recorder() {
            return Some(LayerCacheDomain::Graphite(
                recorder.inner() as *const _ as usize
            ));
        }
        if let Some(context) = canvas.direct_context() {
            return Some(LayerCacheDomain::Ganesh(context.id()));
        }
        if canvas.recording_context().is_some() {
            return None;
        }
    }
    Some(LayerCacheDomain::Raster)
}

/// The surface properties of the canvas's top layer.
pub(crate) fn top_surface_props(canvas: &Canvas) -> skia_safe::SurfaceProps {
    // skia-safe's `top_props` calls `SkCanvas::getTopProps`, a C++ method that returns a
    // struct by value. MSVC passes the result pointer after `this` and the binding passes it
    // first, so on Windows the call reads a garbage canvas and crashes. `props` writes through
    // a pointer instead. The base layer's properties it returns differ from the top layer's
    // only in pixel geometry inside a save layer, and Fanta's raster surfaces set none.
    #[cfg(all(target_os = "windows", target_env = "msvc"))]
    {
        canvas.props().unwrap_or_default()
    }
    #[cfg(not(all(target_os = "windows", target_env = "msvc")))]
    {
        canvas.top_props()
    }
}

pub(crate) fn immutable_surface_snapshot(
    surface: &mut skia_safe::Surface,
) -> Option<skia_safe::Image> {
    #[cfg(feature = "metal")]
    if surface.canvas().recorder().is_some() {
        // A Graphite image sharing this surface's texture would change when
        // the surface is reused. Cached layers and retained frames need a copy.
        return skia_safe::gpu::graphite::surfaces::as_image_copy(
            surface,
            None,
            skia_safe::gpu::Mipmapped::No,
        );
    }
    let bounds = IRect::from_wh(surface.width(), surface.height());
    surface.image_snapshot_with_bounds(bounds)
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

    #[test]
    fn immutable_cpu_layer_snapshot_survives_source_surface_reuse() {
        let mut source = skia_safe::surfaces::raster_n32_premul((8, 8)).expect("source");
        source.canvas().clear(skia_safe::Color::RED);
        let snapshot = immutable_surface_snapshot(&mut source).expect("snapshot");
        source.canvas().clear(skia_safe::Color::BLUE);
        let mut output = skia_safe::surfaces::raster_n32_premul((8, 8)).expect("output");
        output.canvas().draw_image(&snapshot, (0, 0), None);
        let info = ImageInfo::new(
            (8, 8),
            skia_safe::ColorType::RGBA8888,
            AlphaType::Premul,
            None,
        );
        let mut pixels = vec![0; 8 * 8 * 4];
        assert!(output.read_pixels(&info, &mut pixels, 8 * 4, (0, 0)));
        assert!(
            pixels
                .chunks_exact(4)
                .all(|pixel| pixel == [255, 0, 0, 255])
        );
    }

    #[test]
    fn snapshot_and_backend_probe_refuse_nonrendering_surfaces() {
        let mut surface = skia_safe::surfaces::null((8, 8)).expect("null surface");
        let info = ImageInfo::new_n32_premul((1, 1), None);
        assert!(immutable_surface_snapshot(&mut surface).is_none());
        assert!(canvas_backend_is_gpu(surface.canvas(), &info).is_none());
    }

    #[cfg(all(target_os = "macos", feature = "metal"))]
    #[test]
    #[ignore = "requires actual macOS Graphite/Metal; execute explicitly with --features metal --ignored"]
    #[allow(unsafe_code)]
    fn graphite_layer_snapshot_keeps_immutable_pixels_and_compatible_recorder()
    -> Result<(), Box<dyn std::error::Error>> {
        use metal::foreign_types::ForeignType;
        use skia_safe::gpu::graphite::{self, mtl};
        let device = metal::Device::system_default().ok_or("Metal device is required")?;
        let queue = device.new_command_queue();
        // SAFETY: these handles come from owned Metal objects, which remain
        // alive until every context, recorder and surface below has dropped.
        let backend = unsafe {
            mtl::BackendContext::new(
                device.as_ptr() as mtl::Handle,
                queue.as_ptr() as mtl::Handle,
            )
        };
        let mut context =
            mtl::context_factory::make_metal(&backend, None).ok_or("Graphite context")?;
        let mut recorder = context.make_recorder(None).ok_or("Graphite recorder")?;
        let info = ImageInfo::new(
            (8, 8),
            skia_safe::ColorType::RGBA8888,
            AlphaType::Premul,
            None,
        );
        let mut source = graphite::surfaces::render_target(
            &mut recorder,
            &info,
            skia_safe::gpu::Mipmapped::No,
            None,
            None,
        )
        .ok_or("source surface")?;
        let mut output = compatible_offscreen_surface(source.canvas(), &info, None)
            .ok_or("compatible offscreen")?;
        assert!(output.canvas().recorder().is_some());
        assert_eq!(canvas_backend_is_gpu(output.canvas(), &info), Some(true));
        source.canvas().clear(skia_safe::Color::RED);
        let snapshot = immutable_surface_snapshot(&mut source).ok_or("immutable snapshot")?;
        assert!(snapshot.is_texture_backed());
        let scene = Scene::new();
        let components = ComponentLibrary::new();
        let mut cache = LayerCache::default();
        let id = NodeId::new();
        cache.begin_frame(
            output.canvas(),
            &scene,
            &components,
            epoch(&scene, 0),
            1.0,
            (0.0, 0.0),
        );
        assert!(cache.store(id, snapshot, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], (0, 0)));
        for color in [skia_safe::Color::BLUE, skia_safe::Color::GREEN] {
            source.canvas().clear(color);
            output.canvas().clear(skia_safe::Color::TRANSPARENT);
            cache.begin_frame(
                output.canvas(),
                &scene,
                &components,
                epoch(&scene, 0),
                1.0,
                (0.0, 0.0),
            );
            assert!(matches!(
                cache.lookup(output.canvas(), id, &Paint::default()),
                LayerLookup::Hit
            ));
            let mut recording = recorder.snap().ok_or("snap recording")?;
            let status =
                context.insert_recording(&graphite::InsertRecordingInfo::new(&mut recording));
            if status != graphite::InsertStatus::Success {
                return Err(format!("insert recording: {status:?}").into());
            }
            if !context.submit_and_wait() {
                return Err("Graphite submit failed".into());
            }
            let mut pixels = vec![0; 8 * 8 * 4];
            assert!(context.read_pixels(&mut output, &info, &mut pixels, 8 * 4, (0, 0)));
            assert!(
                pixels
                    .chunks_exact(4)
                    .all(|pixel| pixel == [255, 0, 0, 255])
            );
            assert_eq!(cache.len(), 1);
        }
        let mut foreign_context =
            mtl::context_factory::make_metal(&backend, None).ok_or("foreign Graphite context")?;
        let mut foreign_recorder = foreign_context
            .make_recorder(None)
            .ok_or("foreign Graphite recorder")?;
        let mut foreign = graphite::surfaces::render_target(
            &mut foreign_recorder,
            &info,
            skia_safe::gpu::Mipmapped::No,
            None,
            None,
        )
        .ok_or("foreign surface")?;
        assert!(canvas_cache_domain(output.canvas()) != canvas_cache_domain(foreign.canvas()));
        assert!(!cache.begin_frame(
            foreign.canvas(),
            &scene,
            &components,
            epoch(&scene, 0),
            1.0,
            (0.0, 0.0),
        ));
        assert_eq!(cache.len(), 0);
        assert!(matches!(
            cache.lookup(foreign.canvas(), id, &Paint::default()),
            LayerLookup::Miss
        ));
        let snapshot = immutable_surface_snapshot(&mut foreign).ok_or("foreign snapshot")?;
        assert!(cache.store(id, snapshot, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], (0, 0)));
        // SAFETY: the same owned device and queue outlive the Ganesh context
        // and its surface as well as the two still-live Graphite contexts.
        let ganesh_backend = unsafe {
            skia_safe::gpu::mtl::BackendContext::new(device.as_ptr().cast(), queue.as_ptr().cast())
        };
        let mut ganesh_context = skia_safe::gpu::direct_contexts::make_metal(&ganesh_backend, None)
            .ok_or("Ganesh context")?;
        let mut ganesh = skia_safe::gpu::surfaces::render_target(
            &mut ganesh_context,
            skia_safe::gpu::Budgeted::No,
            &info,
            None,
            skia_safe::gpu::SurfaceOrigin::TopLeft,
            None,
            None,
            None,
        )
        .ok_or("Ganesh surface")?;
        assert!(canvas_cache_domain(foreign.canvas()) != canvas_cache_domain(ganesh.canvas()));
        assert!(!cache.begin_frame(
            ganesh.canvas(),
            &scene,
            &components,
            epoch(&scene, 0),
            1.0,
            (0.0, 0.0),
        ));
        assert_eq!(cache.len(), 0);
        assert!(matches!(
            cache.lookup(ganesh.canvas(), id, &Paint::default()),
            LayerLookup::Miss
        ));
        Ok(())
    }

    #[test]
    fn component_dependencies_ignore_unused_definitions_and_guard_variant_candidates() {
        let referenced = ComponentId::new();
        let unused = ComponentId::new();
        let missing = ComponentId::new();
        let mut library = ComponentLibrary::new();
        library.defs.insert(
            referenced,
            ComponentDef::new(referenced, NodeId::new(), "Used"),
        );
        library
            .defs
            .insert(unused, ComponentDef::new(unused, NodeId::new(), "Unused"));
        let mut dependencies = ComponentDependencies::default();
        dependencies.record(&library, referenced);
        library
            .defs
            .get_mut(&unused)
            .expect("unused")
            .name
            .push('!');
        assert!(dependencies.matches(&library));
        library
            .defs
            .get_mut(&referenced)
            .expect("referenced")
            .preview_rev += 1;
        assert!(!dependencies.matches(&library));

        let set = ComponentId::new();
        library.sets.insert(
            set,
            ComponentSet {
                id: set,
                name: "Variants".into(),
                axes: Vec::new(),
                members: vec![referenced, missing],
                default_variant: referenced,
                root: None,
            },
        );
        dependencies.clear();
        dependencies.record(&library, set);
        assert!(dependencies.matches(&library));
        library.defs.insert(
            missing,
            ComponentDef::new(missing, NodeId::new(), "New candidate"),
        );
        assert!(
            !dependencies.matches(&library),
            "a previously missing candidate may now match"
        );
        dependencies.clear();
        dependencies.record(&library, set);
        library
            .defs
            .get_mut(&missing)
            .expect("candidate")
            .name
            .push('!');
        assert!(
            !dependencies.matches(&library),
            "even an unselected candidate is read during resolution"
        );
        dependencies.clear();
        dependencies.record(&library, set);
        library.sets.get_mut(&set).expect("set").default_variant = missing;
        assert!(!dependencies.matches(&library));
    }

    #[test]
    fn unused_library_metadata_changes_keep_cached_layer_pixels() {
        let mut surface = skia_safe::surfaces::raster_n32_premul((4, 4)).expect("surface");
        let scene = Scene::new();
        let mut library = ComponentLibrary::new();
        let unused = ComponentId::new();
        library
            .defs
            .insert(unused, ComponentDef::new(unused, NodeId::new(), "Unused"));
        let mut cache = LayerCache::default();
        cache.begin_frame(
            surface.canvas(),
            &scene,
            &library,
            epoch(&scene, 0),
            1.0,
            (0.0, 0.0),
        );
        let id = NodeId::new();
        assert!(cache.store(
            id,
            raster_image(2, 2),
            [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            (0, 0)
        ));
        library.defs.get_mut(&unused).expect("unused").rev += 1;
        assert!(cache.begin_frame(
            surface.canvas(),
            &scene,
            &library,
            epoch(&scene, 0),
            1.0,
            (0.0, 0.0)
        ));
        assert!(matches!(
            cache.lookup(surface.canvas(), id, &Paint::default()),
            LayerLookup::Hit
        ));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn oversized_component_metadata_requires_a_cold_frame_without_retaining_the_payload() {
        let id = ComponentId::new();
        let mut library = ComponentLibrary::new();
        let definition = ComponentDef::new(id, NodeId::new(), "x".repeat(5 << 20));
        library.defs.insert(id, definition);
        let mut dependencies = ComponentDependencies::default();
        dependencies.record(&library, id);
        assert!(dependencies.incomplete);
        assert!(dependencies.definitions.is_empty());
        assert_eq!(dependencies.bytes, 0);
        assert!(!dependencies.matches(&library));
        dependencies.clear();
        assert!(dependencies.matches(&library));
    }

    #[test]
    fn candidate_definition_witness_does_not_hide_a_later_set_reference() {
        let parent = ComponentId::new();
        let child = ComponentId::new();
        let member = ComponentId::new();
        let mut library = ComponentLibrary::new();
        library
            .defs
            .insert(member, ComponentDef::new(member, NodeId::new(), "Member"));
        for (id, members) in [(parent, vec![member, child]), (child, vec![member])] {
            library.sets.insert(
                id,
                ComponentSet {
                    id,
                    name: "Set".into(),
                    axes: Vec::new(),
                    members,
                    default_variant: member,
                    root: None,
                },
            );
        }
        let mut dependencies = ComponentDependencies::default();
        dependencies.record(&library, parent);
        assert!(dependencies.definitions.contains_key(&child));
        assert!(!dependencies.sets.contains_key(&child));
        dependencies.record(&library, child);
        assert!(dependencies.sets.contains_key(&child));
        library
            .sets
            .get_mut(&child)
            .expect("child set")
            .default_variant = ComponentId::new();
        assert!(!dependencies.matches(&library));
        assert!(
            dependencies
                .changed_references(&library)
                .expect("complete")
                .contains(&child)
        );
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
