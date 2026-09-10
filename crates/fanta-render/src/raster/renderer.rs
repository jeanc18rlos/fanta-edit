//! [`RasterRenderer`] — the renderer struct, its constructors and render
//! entry points, the per-frame [`RenderInputs`] context, and the cross-frame
//! instance-expansion memo ([`InstanceCache`]). The actual scene walk lives in
//! the sibling [`walk`](super::walk) module; this file owns the public surface
//! and the per-frame state plumbing.
//!
//! [`InstanceCache`]: InstanceCache
use super::{
    AlphaType, Arc, AssetResolver, BTreeMap, Canvas, Color, ColorType, ComponentLibrary,
    EncodedImageFormat, ExpandedNode, Fill, Hash, Hasher, IdHashMap, ImageCache, ImageInfo,
    InstanceNode, Instant, LayerCache, LayerEpoch, ModeId, NodeData, NodeId, Rect, RenderCtx,
    Scene, Surface, VariableCollectionId, VariableRegistry, Viewport, render_node, surfaces,
    to_sk_color, visible_world_rect,
};

/// Errors returned by the renderer.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("could not create a {width}x{height} raster surface")]
    SurfaceCreate { width: u32, height: u32 },
    #[error("PNG encode failed")]
    Encode,
}

/// Instrumentation returned by [`RasterRenderer::render`]. Tracks frame time
/// and basic draw counts — the foundation of the perf-budget enforcement
/// described in ARCHITECTURE.md §5.
#[derive(Debug, Clone, Default)]
pub struct RenderMetrics {
    pub frame_micros: u64,
    pub nodes_visited: u32,
    pub nodes_drawn: u32,
    /// Nodes skipped because their world AABB did not intersect the visible
    /// region. A culled group also skips its whole subtree, so the subtree's
    /// nodes are *not* counted here (they were never visited) — this counts
    /// only the cull-decision points, which is the figure that matters for
    /// "how much off-screen work did we avoid."
    pub nodes_culled: u32,
    /// Vector nodes whose Skia paths were (re)built this frame instead of
    /// served from the path cache — the deterministic observable for the
    /// per-node geometry-stamp invalidation: a steady frame reports 0, an
    /// edit to one node reports 1, never the scene size.
    pub paths_built: u32,
    /// Node-level effects save-layers pushed this frame (opacity, drop shadow,
    /// layer blur, blend mode, isolation). Each is an offscreen allocation and,
    /// on the GPU, its own render pass — the count is the observable for the
    /// opacity fold below.
    pub effect_layers: u32,
    /// Leaf vectors whose `opacity < 1` was folded into their single draw's
    /// paint alpha instead of an effects layer (see
    /// `opacity_folds_into_paint`).
    pub opacity_folds: u32,
    /// Effects layers served from the cross-frame layer cache this frame
    /// (drawn as one cached image; no save-layer, no filter pass, and the
    /// node's subtree was not walked). See [`RasterRenderer::set_layer_cache_enabled`].
    pub layer_cache_hits: u32,
    /// Effects layers rendered through the layer cache's offscreen path this
    /// frame (a miss that populated — or, for a volatile layer, tried to
    /// populate — the cache). Direct save-layers are counted only in
    /// `effect_layers`.
    pub layer_cache_misses: u32,
}

/// Serialization controls for the headless SVG canvas.
///
/// SVG rendering goes through the same [`RasterRenderer::render_to_canvas`]
/// scene walk as CPU PNG and host-owned GPU canvases. These options only
/// control how Skia serializes that drawing into XML.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SvgRenderOptions {
    /// Convert shaped text into vector outlines. This makes the SVG independent
    /// of fonts installed on the machine that later opens it, at the cost of
    /// losing inspectable/selectable text.
    pub convert_text_to_paths: bool,
    /// Emit relative path commands where Skia can do so.
    pub relative_path_encoding: bool,
    /// Pretty-print the XML. Enabled by default because this path is primarily
    /// a source-inspection and golden-diff harness.
    pub pretty_xml: bool,
}

impl Default for SvgRenderOptions {
    fn default() -> Self {
        Self {
            convert_text_to_paths: false,
            relative_path_encoding: false,
            pretty_xml: true,
        }
    }
}

/// Headless SVG bytes plus the metrics from the shared renderer walk.
#[derive(Debug, Clone)]
pub struct SvgRenderOutput {
    pub svg: Vec<u8>,
    pub metrics: RenderMetrics,
}

/// The document-level state the renderer needs beyond `(Scene, Viewport)` to
/// resolve **component instances** and **variable bindings**: the component
/// library (master subtrees), the variable registry (collections + per-mode
/// values), and the document's currently-active modes.
///
/// All borrowed for the duration of one render call — the renderer never holds
/// these between frames. `RasterRenderer` keeps its own cross-frame caches
/// (image upload + instance expansion); this struct is just the per-frame
/// inputs the app threads in.
///
/// ## Why a separate context (and not extra `render` params)
///
/// The legacy [`RasterRenderer::render`] / [`render_page`] signatures take only
/// `(&Scene, &Viewport[, page_root])` and are called from `fanta-app`. Rather
/// than break them, the instance/variable-aware path is the new
/// [`RasterRenderer::render_with`] / [`render_page_with`] taking a
/// `&RenderInputs`; the legacy entry points delegate to them with
/// [`RenderInputs::empty`] (no library, no variables, default modes), which
/// reproduces the previous behaviour exactly — instances draw the faint
/// placeholder and no binding is substituted.
///
/// [`render`]: RasterRenderer::render
/// [`render_page`]: RasterRenderer::render_page
/// [`render_with`]: RasterRenderer::render_with
/// [`render_page_with`]: RasterRenderer::render_page_with
/// Ephemeral, per-node playback state the app feeds into the render each frame —
/// NOT part of the document. It drives the live media affordances: the audio
/// playhead + played/remaining split today, with the video progress bar (and,
/// later, the 3D orbit angle) riding the same channel. Absent for headless /
/// golden renders, so those are unaffected.
#[derive(Debug, Clone)]
pub struct MediaPlayback {
    /// Playback position in `0.0..=1.0`.
    pub progress: f32,
    /// The video frame to show at this position (a filmstrip asset id), or
    /// `None` for audio / a paused video (which shows its poster).
    pub frame: Option<fanta_doc::AssetId>,
    /// Borrowed GPU textures must remain alive until this frame finishes rendering.
    pub decoded_frame: Option<skia_safe::Image>,
}

pub struct RenderInputs<'a> {
    /// Component masters, for [`NodeData::Instance`] expansion. Empty → an
    /// instance draws the faint placeholder (the master can't be found).
    pub components: &'a ComponentLibrary,
    /// Variable collections + per-mode values, for the binding overlay.
    pub variables: &'a VariableRegistry,
    /// The document's active mode per collection (the Light/Dark toggle, etc.).
    /// A frame's `explicit_modes` pin overrides this for its subtree — that is
    /// handled inside [`fanta_doc::resolve_effective_mode`], which the binding
    /// overlay calls (via [`resolve_bound_value`]) per node, so callers pass
    /// only the *doc-level* selection here.
    pub active_modes: &'a BTreeMap<VariableCollectionId, ModeId>,
    /// A monotonically-increasing counter the app bumps whenever a variable
    /// value, active mode, frame mode pin, or ancestor relationship affecting
    /// effective modes changes. Folded into the instance-memo key because
    /// alias-backed component properties are resolved into cached expansions.
    /// Failing to advance it can therefore serve a stale component-property
    /// value even though ordinary node bindings are re-resolved every frame.
    pub mode_generation: u64,
    /// A transient motion sample for this frame. Motion is applied to scratch
    /// node clones after variable resolution, so playback never mutates the
    /// authored scene.
    pub motion: Option<&'a fanta_doc::MotionEvaluation>,
    /// Live media playback positions by node id, or `None` when nothing is
    /// playing / for headless renders. Read by the audio + video content arms to
    /// draw the playhead / progress bar. See [`MediaPlayback`].
    pub playback: Option<&'a std::collections::HashMap<NodeId, MediaPlayback>>,
    /// Whether the app is in dark appearance — lets theme-aware node content
    /// (the audio waveform card) pick a card fill + accent that read on the
    /// active theme instead of a hardcoded dark pill. Defaults to `false`
    /// (light) for headless/legacy renders.
    pub dark_ui: bool,
}

impl<'a> RenderInputs<'a> {
    /// An inputs context with no components and no variables, in the default
    /// (empty) mode set. Used by the legacy [`RasterRenderer::render`] /
    /// [`render_page`] entry points so they behave exactly as before the
    /// instance/variable support landed.
    ///
    /// The borrowed library/registry are `'static` empties, so this is free.
    ///
    /// [`render`]: RasterRenderer::render
    /// [`render_page`]: RasterRenderer::render_page
    pub fn empty() -> RenderInputs<'static> {
        // `OnceLock`-free `'static` empties: a `const` value can't be
        // referenced as `&'static` for non-`Copy` heap types, but the default
        // `ComponentLibrary` / `VariableRegistry` / map are cheap to leak once
        // for the process. We instead use thread-safe statics built lazily.
        use std::sync::OnceLock;
        static COMPONENTS: OnceLock<ComponentLibrary> = OnceLock::new();
        static VARIABLES: OnceLock<VariableRegistry> = OnceLock::new();
        static MODES: OnceLock<BTreeMap<VariableCollectionId, ModeId>> = OnceLock::new();
        RenderInputs {
            components: COMPONENTS.get_or_init(ComponentLibrary::default),
            variables: VARIABLES.get_or_init(VariableRegistry::default),
            active_modes: MODES.get_or_init(BTreeMap::new),
            mode_generation: 0,
            motion: None,
            playback: None,
            dark_ui: false,
        }
    }

    /// Inputs wired from a [`Doc`]'s own component library, variable registry,
    /// and active modes — the context almost every embedder wants (see the
    /// embedding API in ARCHITECTURE.md §2). Uses `mode_generation: 0`, which
    /// is correct for hosts that don't mutate variables or effective-mode
    /// ancestry between frames (see the field docs); interactive hosts should
    /// build `RenderInputs` themselves with their own counter and playback map.
    ///
    /// [`Doc`]: fanta_doc::Doc
    pub fn for_doc(doc: &'a fanta_doc::Doc) -> RenderInputs<'a> {
        RenderInputs {
            components: &doc.components,
            variables: &doc.variables,
            active_modes: &doc.active_modes,
            mode_generation: 0,
            motion: None,
            playback: None,
            dark_ui: false,
        }
    }
}

/// Memoized instance expansions, keyed per spec 07 §2 P3 by
/// `(instance NodeId, ComponentDef.rev, override-hash, mode generation)`.
///
/// The cached value is the *structural* expansion — the transient subtree of
/// fresh-id [`ExpandedNode`]s as `expand_instance` returns it, before the
/// paint-time binding overlay. That overlay is re-applied every frame (it is
/// cheap and mode-dependent), so the memo only saves the deep-clone +
/// override-application work, which is the expensive part of a deep instance
/// tree.
///
/// Invalidation is automatic: editing a master bumps its `ComponentDef.rev`
/// (see `ComponentLibrary::bump_rev_for_node`), which changes the key; changing
/// the instance's own overrides changes the override-hash; a mode switch bumps
/// `mode_generation`, and a frame mode pin above the instance changes the
/// pin-hash. Entries are keyed by instance id and hold one expansion each, so a
/// key change REPLACES the instance's entry instead of leaving the old one
/// behind. Entries no instance looked up for [`INSTANCE_CACHE_IDLE_FRAMES`]
/// frames are dropped (that is how the fresh-id clones of nested instances,
/// orphaned when their outer expansion re-expands, get freed), and
/// `clear_instance_cache` drops it wholesale (e.g. on document switch).
#[derive(Default)]
pub(crate) struct InstanceCache {
    pub(crate) entries: IdHashMap<NodeId, InstanceCacheEntry>,
    /// [`hash_overrides`] memoized per live-scene instance as
    /// `(node stamp, hash)`: the hash inputs (`component`, `overrides`,
    /// `prop_values`, `derived`) live on the scene node, whose stamp moves on
    /// every data edit, so an unchanged stamp guarantees an unchanged hash and
    /// a steady-state frame serializes nothing. Purged of removed ids when the
    /// scene's removal revision moves.
    override_hashes: IdHashMap<NodeId, (u64, u64)>,
    removal_seen: u64,
    /// Frame serial for idle eviction.
    frame: u64,
}

/// One instance's memoized expansion and the key it was built under.
pub(crate) struct InstanceCacheEntry {
    pub(crate) key: InstanceCacheKey,
    pub(crate) expanded: Arc<Vec<ExpandedNode>>,
    last_used: u64,
}

/// Frames an entry may go without a lookup before it is dropped. Long enough
/// that an instance scrolled off-screen and back within a few seconds keeps
/// its expansion; short enough that orphaned nested-clone entries do not pile
/// up over an editing session.
pub(crate) const INSTANCE_CACHE_IDLE_FRAMES: u64 = 240;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct InstanceCacheKey {
    pub(crate) instance: NodeId,
    pub(crate) rev: u64,
    /// The resolved master's transient-preview revision, which moves while a
    /// drag/text/properties preview writes straight into the scene without
    /// going through history (and so without moving `rev`).
    pub(crate) preview_rev: u64,
    pub(crate) override_hash: u64,
    pub(crate) mode_generation: u64,
    /// Hash of the frame mode pins on the instance's ancestor chain (see
    /// `hash_mode_pins`): alias-backed component properties are resolved in
    /// the instance's effective mode INTO the cached expansion, and a pin on
    /// an enclosing frame changes that mode without touching the variables,
    /// the modes, or the instance node itself.
    pub(crate) mode_pins: u64,
}

impl InstanceCache {
    fn clear(&mut self) {
        self.entries.clear();
        self.override_hashes.clear();
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    /// The memoized expansion for `key`'s instance, if it was built under
    /// exactly this key.
    pub(crate) fn lookup(&mut self, key: &InstanceCacheKey) -> Option<Arc<Vec<ExpandedNode>>> {
        let entry = self.entries.get_mut(&key.instance)?;
        if entry.key != *key {
            return None;
        }
        entry.last_used = self.frame;
        Some(Arc::clone(&entry.expanded))
    }

    pub(crate) fn insert(&mut self, key: InstanceCacheKey, expanded: Arc<Vec<ExpandedNode>>) {
        self.entries.insert(
            key.instance,
            InstanceCacheEntry {
                key,
                expanded,
                last_used: self.frame,
            },
        );
    }

    /// [`hash_overrides`] for `instance`, served from the stamp memo when
    /// `stamp` is `Some` (a live-scene node) and computed directly for a
    /// transient clone, whose fresh id would only ever pin a one-off entry.
    pub(crate) fn override_hash(
        &mut self,
        instance_id: NodeId,
        stamp: Option<u64>,
        instance: &InstanceNode,
    ) -> u64 {
        let Some(stamp) = stamp else {
            return hash_overrides(instance);
        };
        if let Some(&(memo_stamp, hash)) = self.override_hashes.get(&instance_id)
            && memo_stamp == stamp
        {
            return hash;
        }
        let hash = hash_overrides(instance);
        self.override_hashes.insert(instance_id, (stamp, hash));
        hash
    }

    /// Per-frame bookkeeping: advance the idle clock, drop entries from older
    /// `mode_generation`s and entries idle for [`INSTANCE_CACHE_IDLE_FRAMES`],
    /// and purge the override-hash memo of removed nodes (gated on the scene's
    /// removal revision so steady frames never scan it).
    fn begin_frame(&mut self, scene: &Scene, mode_generation: u64) {
        self.frame = self.frame.wrapping_add(1);
        let frame = self.frame;
        if self.entries.values().any(|entry| {
            entry.key.mode_generation != mode_generation
                || frame.wrapping_sub(entry.last_used) > INSTANCE_CACHE_IDLE_FRAMES
        }) {
            self.entries.retain(|_, entry| {
                entry.key.mode_generation == mode_generation
                    && frame.wrapping_sub(entry.last_used) <= INSTANCE_CACHE_IDLE_FRAMES
            });
        }
        let removal = scene.removal_revision();
        if self.removal_seen != removal {
            self.override_hashes.retain(|id, _| scene.contains(*id));
            self.removal_seen = removal;
        }
    }
}

/// Cache for the expensive Skia PathOp fold of a [`NodeData::Boolean`].
///
/// Keyed by boolean node id, tagged with [`Scene::subtree_stamp`] at fold
/// time: the fold bakes operand geometry, order, transforms, and visibility,
/// all of which stamp some node inside the subtree when they change, and the
/// subtree max strictly increases on any stamped change. An edit elsewhere in
/// the document leaves the entry valid — the pre-stamp design keyed on the
/// global revision and re-folded every boolean on every edit.
///
/// A stale-tag lookup rebuilds in place (live ids can never accumulate stale
/// entries); ids removed from the scene are purged only when
/// [`Scene::removal_revision`] moved.
///
/// [`Scene::subtree_stamp`]: fanta_doc::Scene::subtree_stamp
/// [`Scene::removal_revision`]: fanta_doc::Scene::removal_revision
#[derive(Default)]
pub(crate) struct BooleanCache {
    pub(crate) entries: IdHashMap<NodeId, (u64, skia_safe::Path)>,
    pub(crate) removal_seen: u64,
}

impl BooleanCache {
    fn clear(&mut self) {
        self.entries.clear();
    }

    /// Purge entries for nodes that left the scene — gated on the scene's
    /// removal revision so steady-state frames never scan the map.
    fn purge_removed(&mut self, scene: &fanta_doc::Scene) {
        let removal = scene.removal_revision();
        if self.removal_seen != removal {
            self.entries.retain(|id, _| scene.contains(*id));
            self.removal_seen = removal;
        }
    }
}

/// Cache of built Skia paths for vector nodes, keyed by
/// `(vector node id in the live scene, scene.revision())`.
///
/// Rebuilding a node's outline ([`vector_outline_sk_path`]) and fill coverage
/// path ([`to_sk_fill_path`]) every frame was the largest steady per-frame
/// allocation: each visible vector re-walked its `PathData` verbs into fresh
/// heap-backed `skia_safe::Path`s per draw. Like [`BooleanCache`], any edit
/// bumps `scene.revision()` and re-keys, so stale entries are never looked up
/// again and are dropped by [`evict_stale`] each render.
///
/// Only *live scene* nodes are cached: transient instance-expansion clones have
/// no scene id (their `paint_node_content` call passes `scene_id: None`) and a
/// node whose variable/motion overlay replaced it this frame is bypassed by the
/// caller, so per-frame geometry never pins a cache entry.
///
/// [`vector_outline_sk_path`]: super::vector_outline_sk_path
/// [`to_sk_fill_path`]: super::to_sk_fill_path
/// [`evict_stale`]: PathCache::evict_stale
#[derive(Default)]
pub(crate) struct PathCache {
    pub(crate) entries: IdHashMap<NodeId, (u64, CachedVectorPaths)>,
    pub(crate) removal_seen: u64,
}

/// The Skia paths [`draw_vector`](super::draw_vector) needs for one vector
/// node: the stroke/clip outline, plus the fill coverage path when it differs
/// (mixed subpath winding rules). `skia_safe::Path` clones are cheap
/// (copy-on-write ref to the immutable path data), so handing out clones of
/// these does not re-allocate the geometry.
#[derive(Clone)]
pub(crate) struct CachedVectorPaths {
    pub(crate) outline: skia_safe::Path,
    /// `None` ⇒ the outline doubles as the fill path (the common case).
    pub(crate) fill: Option<skia_safe::Path>,
    /// Local-path rough bounds, memoized at build time — previously an
    /// O(segments) walk per visible vector per frame.
    pub(crate) rough_bounds: fanta_doc::Bounds,
    /// Whether the path is an axis-aligned rectangle, memoized at build time
    /// — previously a per-frame check that heap-allocated per call.
    pub(crate) is_rect: bool,
}

impl PathCache {
    fn clear(&mut self) {
        self.entries.clear();
    }

    /// Purge entries for nodes that left the scene — gated on the scene's
    /// removal revision so steady-state frames never scan the map.
    fn purge_removed(&mut self, scene: &fanta_doc::Scene) {
        let removal = scene.removal_revision();
        if self.removal_seen != removal {
            self.entries.retain(|id, _| scene.contains(*id));
            self.removal_seen = removal;
        }
    }
}

/// Hash every field of an instance that expansion reads — `component`,
/// `overrides`, `prop_values`, `derived` and `local_size` — into one `u64` for
/// the memo key. The overrides `Vec<Override>` is not `Hash` (it carries
/// `f64`s via `OverrideValue::Field` JSON and `Fills`), so we route it through
/// its serde JSON projection — stable for a given override set and cheap
/// relative to a full subtree clone. A hash collision would at worst serve a
/// wrong-but-same-shape expansion; the input space (overrides on one instance
/// between two edits) makes that negligible.
/// Three serializations per call, so the renderer memoizes it per live
/// instance behind the node's geometry stamp ([`InstanceCache::override_hash`]).
pub(crate) fn hash_overrides(instance: &InstanceNode) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    // `component` participates so a swap (which `expand` keys off) re-expands
    // even if the rev/override set is unchanged.
    instance.component.hash(&mut hasher);
    if let Ok(json) = serde_json::to_string(&instance.overrides) {
        json.hash(&mut hasher);
    }
    // `prop_values` drive descendant bindings in expansion; include them so a
    // prop change re-expands.
    if let Ok(json) = serde_json::to_string(&instance.prop_values) {
        json.hash(&mut hasher);
    }
    // Baked per-instance `derived` data overwrites master values in expansion,
    // so it MUST participate — otherwise clearing it (e.g. "Push to instances")
    // would not invalidate the memo and the stale baked layout would persist.
    if let Ok(json) = serde_json::to_string(&instance.derived) {
        json.hash(&mut hasher);
    }
    // `pin_expansion_root_box` bakes the instance's own box into the expanded
    // root, so a resize with no other change must miss the memo too.
    for extent in instance.local_size {
        extent.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

/// CPU-backed Skia surface renderer. Resize in place via [`resize`] — it keeps
/// the image/instance caches (they are size-independent) and only the owned
/// CPU surface, if any, is re-created at the new size.
///
/// [`resize`]: Self::resize
pub struct RasterRenderer {
    width: u32,
    height: u32,
    /// The owned CPU pixel surface. Starts as a draw-discarding NULL surface
    /// (no pixel memory) and is upgraded to a real `width × height` raster
    /// surface by the first path that actually draws to or reads it
    /// ([`render_page_with`], [`render_tile_self`], [`encode_png`],
    /// [`canvas`]) — so a caller that only ever renders onto an external
    /// canvas ([`render_to_canvas`] / [`render_tile_onto`], the GPU present
    /// path) never allocates `width × height × 4` bytes it does not read, and
    /// never re-allocates them on resize.
    ///
    /// [`render_page_with`]: Self::render_page_with
    /// [`render_tile_self`]: Self::render_tile_self
    /// [`encode_png`]: Self::encode_png
    /// [`canvas`]: Self::canvas
    /// [`render_to_canvas`]: Self::render_to_canvas
    /// [`render_tile_onto`]: Self::render_tile_onto
    surface: Surface,
    /// Whether `surface` is still the draw-discarding null placeholder. See
    /// [`Self::ensure_raster_surface`].
    surface_is_placeholder: bool,
    /// Color the canvas is cleared to before each frame (transparent by
    /// default; the app sets a dark or light "infinite canvas" color).
    pub background: Color,
    /// DPI scale — the multiplier from logical pixels (world units at
    /// `zoom = 1`) to physical pixels in `surface`. 1.0 on standard displays,
    /// 2.0 on Retina, etc. Default 1.0 so existing tests keep their pixel
    /// arithmetic; `fanta-app` sets it per-window when the renderer is
    /// constructed or resized.
    ///
    /// Concretely: the scene transform inside [`render`] applies a scale of
    /// `viewport.zoom * display_scale`. World coordinates therefore mean
    /// "logical pixels at zoom = 1" — the same units the viewport / hit-test
    /// / snap / overlay layers all reason in — and the renderer is the only
    /// layer that has to translate to physical pixels.
    ///
    /// [`render`]: Self::render
    pub display_scale: f64,

    /// Resolves an [`AssetId`] to decoded pixels for [`NodeData::Bitmap`] and
    /// image fills. `None` until the app installs one via
    /// [`set_asset_resolver`] — with no resolver every bitmap draws the
    /// placeholder, which is exactly the pre-asset behaviour, so existing
    /// callers and tests keep working unchanged.
    ///
    /// Behind an [`Arc<dyn AssetResolver>`] so the same cache can be shared by
    /// the render thread and a background decode pool, and so swapping it is a
    /// pointer write rather than a renderer rebuild.
    ///
    /// [`AssetId`]: fanta_doc::AssetId
    /// [`set_asset_resolver`]: Self::set_asset_resolver
    asset_resolver: Option<Arc<dyn AssetResolver>>,

    /// Uploaded-image cache, keyed by [`AssetId`]. Bitmap nodes and image fills
    /// build their [`skia_safe::Image`] once and reuse it across frames instead
    /// of re-uploading the raw bytes every frame. Owned by the renderer because
    /// it is tied to this surface's lifetime; cleared via [`clear_image_cache`].
    ///
    /// [`AssetId`]: fanta_doc::AssetId
    /// [`clear_image_cache`]: Self::clear_image_cache
    image_cache: ImageCache,

    /// Cross-frame memo of component-instance expansions, keyed by
    /// `(instance id, ComponentDef.rev, override-hash, mode generation)`. Saves
    /// re-running [`expand_instance`]'s deep-clone for an unchanged instance
    /// tree every frame. See [`InstanceCache`]. Dropped by
    /// [`clear_instance_cache`].
    ///
    /// [`clear_instance_cache`]: Self::clear_instance_cache
    instance_cache: InstanceCache,

    /// Cache of folded boolean-operation paths (Skia PathOp results). See
    /// [`BooleanCache`]. Invalidation is via `scene.revision()`.
    boolean_cache: BooleanCache,

    /// Cache of built vector-node Skia paths. See [`PathCache`]. Invalidation
    /// is via `scene.revision()`, like the boolean cache.
    path_cache: PathCache,

    /// Cross-frame cache of rendered effect layers (blur / shadow / blend /
    /// opacity-group save-layers), keyed per node and dropped wholesale on
    /// any content edit. See [`LayerCache`] and
    /// [`Self::set_layer_cache_enabled`].
    layer_cache: LayerCache,

    /// Whether the viewport's device translation is rounded to whole device
    /// pixels (see [`Self::set_pixel_snap_pan`]). Off by default.
    pixel_snap_pan: bool,

    /// The [`Scene::instance_id`] of the last scene rendered, or `None` before
    /// the first render. `scene.revision()` is only meaningful *within* one
    /// scene instance — a reloaded document restarts its counter while node
    /// ids persist, so a structurally-identical reparse can land on exactly
    /// the `(NodeId, revision)` keys of the scene it replaced while carrying
    /// different geometry. Every render entry point compares this against the
    /// incoming scene's id and wholesale-clears the revision/graph-derived
    /// caches (path, boolean, instance) on a mismatch, so an embedder that
    /// swaps the scene under a persistent renderer can never be served stale
    /// content. See [`Self::sync_scene_instance`].
    ///
    /// [`Scene::instance_id`]: fanta_doc::Scene::instance_id
    last_scene_instance_id: Option<u64>,
}

impl RasterRenderer {
    /// Construct a renderer for `width` × `height` pixels. Returns `Err` for a
    /// zero dimension.
    ///
    /// The pixel-backed CPU surface is NOT allocated here: construction starts
    /// with a draw-discarding null surface (no pixel memory), upgraded lazily
    /// by the first path that draws to or reads the owned surface — so an
    /// external-canvas caller ([`render_to_canvas`]) never pays for a
    /// `width × height × 4` buffer it does not use.
    ///
    /// [`render_to_canvas`]: Self::render_to_canvas
    pub fn new(width: u32, height: u32) -> Result<Self, RenderError> {
        // Null-surface creation fails exactly for a non-positive size, which
        // doubles as the dimension validation the old eager allocation did.
        let surface = surfaces::null((width as i32, height as i32))
            .ok_or(RenderError::SurfaceCreate { width, height })?;
        Ok(Self {
            width,
            height,
            surface,
            surface_is_placeholder: true,
            background: Color::TRANSPARENT,
            display_scale: 1.0,
            asset_resolver: None,
            image_cache: ImageCache::new(),
            instance_cache: InstanceCache::default(),
            boolean_cache: BooleanCache::default(),
            path_cache: PathCache::default(),
            layer_cache: LayerCache::default(),
            pixel_snap_pan: false,
            last_scene_instance_id: None,
        })
    }

    /// Drop every cache whose entries are only valid for one scene instance
    /// when `scene` is a different instance than the last one rendered.
    ///
    /// The path and boolean caches key on `(NodeId, scene.revision())`, and
    /// the instance memo bakes the scene's ancestry-resolved modes into its
    /// expansions — none of which survive the scene being *replaced* (as
    /// opposed to mutated): a fresh instance restarts its revision counter,
    /// so equal keys would no longer guarantee equal content. Called at the
    /// top of every render entry point, before the per-frame `evict_stale`
    /// passes (which can only reason within one instance's revision stream).
    fn sync_scene_instance(&mut self, scene: &Scene) {
        let incoming = scene.instance_id();
        if self.last_scene_instance_id != Some(incoming) {
            // Path/boolean entries are tagged with process-unique geometry
            // stamps and self-revalidate on lookup, so they survive scene
            // instance swaps — the session rebuilds its projection (a fresh
            // clone) after every edit, and clearing here rebuilt every Skia
            // path per edit. The instance memo still keys on per-doc state,
            // so it alone resets.
            self.instance_cache.clear();
            self.last_scene_instance_id = Some(incoming);
        }
    }

    /// Change the render target size in place, KEEPING the image and instance
    /// caches — they are size-independent, and rebuilding the renderer on
    /// every window resize threw them away along with a full surface
    /// re-allocation. The owned CPU surface reverts to the (pixel-free) null
    /// placeholder and is re-created lazily at the new size by the paths that
    /// use it; an external-canvas caller pays nothing here. Returns `Err` for
    /// a zero dimension, like [`new`].
    ///
    /// [`new`]: Self::new
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), RenderError> {
        if (width, height) == (self.width, self.height) {
            return Ok(());
        }
        let surface = surfaces::null((width as i32, height as i32))
            .ok_or(RenderError::SurfaceCreate { width, height })?;
        self.width = width;
        self.height = height;
        self.surface = surface;
        self.surface_is_placeholder = true;
        Ok(())
    }

    /// Swap the null placeholder for a real `width × height` raster surface
    /// (idempotent). Returns `Err` when Skia cannot allocate the pixels —
    /// memory exhaustion, since the dimensions were validated at
    /// construction/resize — which the old eager constructor surfaced as its
    /// `Err`; callers that cannot propagate degrade to a blank frame instead.
    fn ensure_raster_surface(&mut self) -> Result<(), RenderError> {
        if !self.surface_is_placeholder {
            return Ok(());
        }
        let surface = surfaces::raster_n32_premul((self.width as i32, self.height as i32)).ok_or(
            RenderError::SurfaceCreate {
                width: self.width,
                height: self.height,
            },
        )?;
        self.surface = surface;
        self.surface_is_placeholder = false;
        Ok(())
    }

    /// Install the asset resolver used to turn an `AssetId` into decoded pixels
    /// for bitmap nodes and image fills. Replaces any previously-set resolver.
    /// Cheap — stores an [`Arc`] clone; no decode happens here.
    pub fn set_asset_resolver(&mut self, resolver: Arc<dyn AssetResolver>) {
        self.asset_resolver = Some(resolver);
    }

    /// Drop every cached uploaded image. Assets are content-addressed and
    /// immutable, so the cache never *needs* invalidation for correctness, but
    /// callers can call this to reclaim image memory (e.g. when switching
    /// documents) or after swapping the asset resolver for an unrelated store.
    pub fn clear_image_cache(&mut self) {
        self.image_cache.clear();
    }

    /// Number of images currently held in the upload cache. Primarily for
    /// tests and memory diagnostics.
    pub fn image_cache_len(&self) -> usize {
        self.image_cache.len()
    }

    /// Drop every cached uploaded image whose asset id is NOT in `keep`,
    /// returning how many were dropped. Called by the app's asset garbage
    /// collector so an orphaned asset's pixels (this cache holds the single
    /// long-lived CPU copy) are reclaimed together with its raw bytes.
    pub fn retain_image_cache(
        &mut self,
        keep: &std::collections::HashSet<fanta_doc::AssetId>,
    ) -> usize {
        self.image_cache.retain(keep)
    }

    /// Drop ONE asset's cached image so it re-renders next frame. Used when a 3D
    /// model's camera changes (orbit): the GPU mesh render is cached by asset id,
    /// so a new angle must invalidate — NOT re-key — that entry (the cache also
    /// feeds the asset GC, which is keyed by asset id). Returns whether an entry
    /// was present.
    pub fn invalidate_image(&mut self, id: fanta_doc::AssetId) -> bool {
        self.image_cache.remove(id)
    }

    /// Drop every memoized instance expansion. Correctness never *requires*
    /// this (the memo key carries `ComponentDef.rev` + override-hash + mode
    /// generation, so a stale entry is simply never looked up), but callers can
    /// reclaim memory after switching documents or doing a big component edit.
    pub fn clear_instance_cache(&mut self) {
        self.instance_cache.clear();
    }

    /// Drop all cached boolean-operation folded paths. They will be recomputed
    /// on the next render that hits a boolean node. Call on major document
    /// switches if you want to reclaim memory eagerly.
    pub fn clear_boolean_cache(&mut self) {
        self.boolean_cache.clear();
    }

    /// Drop all cached vector-node Skia paths. They will be rebuilt on the
    /// next render that hits a vector node. Call on major document switches if
    /// you want to reclaim memory eagerly.
    pub fn clear_path_cache(&mut self) {
        self.path_cache.clear();
    }

    /// Drop every cached effect layer (they rebuild lazily as layers are
    /// rendered). Correctness never requires this — the cache drops itself on
    /// any content edit — but call it to reclaim GPU/CPU memory eagerly, e.g.
    /// on a document switch.
    pub fn clear_layer_cache(&mut self) {
        self.layer_cache.clear();
    }

    /// Enable / disable the cross-frame effect-layer cache (default: enabled).
    ///
    /// When enabled, a node whose compositing needs an effects save-layer
    /// (visible layer blur or drop shadow, non-`Normal` blend, non-foldable
    /// opacity, isolation) is rendered once per zoom into an offscreen and
    /// re-used across pans, pixel-identically, whenever the pan is a whole
    /// number of device pixels (see `raster::layer_cache`). Everything is
    /// dropped on any content edit. Disabling clears the cache and restores
    /// the direct save-layer path byte for byte. Interactive hosts that pan
    /// by fractional device pixels get the benefit by also enabling
    /// [`Self::set_pixel_snap_pan`], which makes every pan a whole-pixel pan.
    pub fn set_layer_cache_enabled(&mut self, enabled: bool) {
        self.layer_cache.set_enabled(enabled);
    }

    /// Whether the effect-layer cache is enabled.
    pub fn layer_cache_enabled(&self) -> bool {
        self.layer_cache.is_enabled()
    }

    /// Byte budget for cached effect-layer images (default 256 MB);
    /// least-recently-used layers are evicted past it.
    pub fn set_layer_cache_budget_bytes(&mut self, bytes: usize) {
        self.layer_cache.set_budget_bytes(bytes);
    }

    /// Number of effect layers currently cached, and their total bytes. For
    /// tests and memory diagnostics.
    pub fn layer_cache_stats(&self) -> (usize, usize) {
        (self.layer_cache.len(), self.layer_cache.bytes())
    }

    /// Round the viewport's device-space translation to whole device pixels
    /// (default: off). The whole scene shifts by at most half a device pixel
    /// from the exact viewport; nothing else changes.
    ///
    /// Why an interactive canvas wants this: a trackpad pan moves the
    /// viewport by fractional logical pixels, so every frame lands the scene
    /// on a different sub-pixel phase — text and hairlines shimmer, and the
    /// effect-layer cache (which is exact only for INTEGER device pans) misses
    /// on every step. With the translation snapped, consecutive pan frames
    /// differ by whole device pixels: cached layers hit and glyphs keep their
    /// phase. Left off for exports / one-shot renders, whose exact placement
    /// matters more than frame-to-frame coherence.
    pub fn set_pixel_snap_pan(&mut self, snap: bool) {
        self.pixel_snap_pan = snap;
    }

    /// Whether the viewport translation is snapped to device pixels.
    pub fn pixel_snap_pan(&self) -> bool {
        self.pixel_snap_pan
    }

    /// Clear image, instance, boolean, vector-path, and effect-layer caches in
    /// one call.
    pub fn clear_caches(&mut self) {
        self.clear_image_cache();
        self.clear_instance_cache();
        self.clear_boolean_cache();
        self.clear_path_cache();
        self.clear_layer_cache();
    }

    /// Number of distinct instance expansions currently memoized. For tests and
    /// memory diagnostics.
    pub fn instance_cache_len(&self) -> usize {
        self.instance_cache.len()
    }

    /// Number of cached boolean folds. For tests and memory diagnostics.
    pub fn boolean_cache_len(&self) -> usize {
        self.boolean_cache.entries.len()
    }

    /// Number of cached vector-node paths. For tests and memory diagnostics.
    pub fn path_cache_len(&self) -> usize {
        self.path_cache.entries.len()
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Direct access to the underlying Skia canvas. Used by `fanta-app` to
    /// draw post-scene overlays (selection outlines, marquee, snap guides,
    /// status text). Call after [`render`]; the canvas is in screen-space —
    /// no viewport transform is active.
    ///
    /// Allocates the owned raster surface on first use. If that allocation
    /// fails (memory exhaustion — this signature has no error channel), the
    /// returned canvas is the draw-discarding placeholder, matching the blank
    /// frame the render paths degrade to in the same state.
    ///
    /// [`render`]: Self::render
    pub fn canvas(&mut self) -> &skia_safe::Canvas {
        match self.ensure_raster_surface() {
            Ok(()) => {}
            // No error channel in this signature: keep the placeholder —
            // draws are discarded, reads see blank, no panic.
            Err(_) => {}
        }
        self.surface.canvas()
    }

    /// Render the scene through the viewport. Returns metrics — the rendered
    /// pixels are available via [`copy_rgba`] and [`encode_png`]. Equivalent to
    /// [`render_page`] with `None` (every root drawn).
    ///
    /// Legacy entry point: it has no component library / variable registry, so
    /// component instances draw the faint placeholder and bound properties are
    /// not substituted. For instance + variable-mode-aware rendering, call
    /// [`render_with`] with a [`RenderInputs`].
    ///
    /// [`copy_rgba`]: Self::copy_rgba
    /// [`encode_png`]: Self::encode_png
    /// [`render_page`]: Self::render_page
    /// [`render_with`]: Self::render_with
    pub fn render(&mut self, scene: &Scene, viewport: &Viewport) -> RenderMetrics {
        self.render_page_with(scene, viewport, None, &RenderInputs::empty())
    }

    /// Render a single page through the viewport, or every root when
    /// `page_root` is `None`. Legacy entry point — see [`render`] for the
    /// instance/variable caveat; call [`render_page_with`] for full support.
    ///
    /// [`render`]: Self::render
    /// [`render_page_with`]: Self::render_page_with
    pub fn render_page(
        &mut self,
        scene: &Scene,
        viewport: &Viewport,
        page_root: Option<NodeId>,
    ) -> RenderMetrics {
        self.render_page_with(scene, viewport, page_root, &RenderInputs::empty())
    }

    /// Render the scene through the viewport with full component-instance and
    /// variable-binding support. Equivalent to [`render_page_with`] with `None`
    /// (every root drawn).
    ///
    /// [`render_page_with`]: Self::render_page_with
    pub fn render_with(
        &mut self,
        scene: &Scene,
        viewport: &Viewport,
        inputs: &RenderInputs,
    ) -> RenderMetrics {
        self.render_page_with(scene, viewport, None, inputs)
    }

    /// Render a single page (or every root when `page_root` is `None`) with the
    /// component library + variable registry + active modes in `inputs`.
    ///
    /// This is the full-fidelity path:
    /// - **Component instances** expand via [`expand_instance`] and draw their
    ///   master subtree clipped to the instance's `local_size`; the expansion
    ///   is memoized across frames (keyed by instance id + master `rev` +
    ///   override-hash + `mode_generation`).
    /// - **Variable bindings** are substituted per node in the node's effective
    ///   mode onto a scratch copy before painting — the live scene is never
    ///   mutated.
    ///
    /// A document's pages are independent root subtrees that share one coordinate
    /// origin (each Figma `CANVAS` imports to one), so drawing them all at once
    /// overlaps them. The app's page switcher passes `doc.active_page()` here to
    /// show exactly one canvas; `None` keeps the legacy all-roots behavior for
    /// single-page / hand-authored docs.
    ///
    /// `Some(id)` for a node that isn't in the scene draws an empty frame rather
    /// than panicking, so a stale active-page id degrades gracefully.
    pub fn render_page_with(
        &mut self,
        scene: &Scene,
        viewport: &Viewport,
        page_root: Option<NodeId>,
        inputs: &RenderInputs,
    ) -> RenderMetrics {
        // The CPU path draws onto the renderer's owned raster surface at its
        // own physical pixel size. We split `self`'s disjoint fields by hand —
        // `surface` (→ canvas) is distinct from `image_cache` / `instance_cache`
        // / the scalar config — and delegate the clear + viewport transform +
        // z-order walk to `draw_scene_onto`, the routine the GPU present also
        // calls (with a Ganesh-backed canvas). Same code, two surfaces.
        let (width, height) = (self.width, self.height);
        let started = Instant::now();
        let mut metrics = RenderMetrics::default();

        // First CPU render allocates the owned surface. Allocation failure
        // (memory exhaustion; the dimensions were validated at construction)
        // degrades to a skipped, blank frame — this signature has no error
        // channel, and the old eager constructor surfaced the same failure.
        if self.ensure_raster_surface().is_err() {
            return metrics;
        }
        self.sync_scene_instance(scene);
        self.instance_cache
            .begin_frame(scene, inputs.mode_generation);
        self.boolean_cache.purge_removed(scene);
        self.path_cache.purge_removed(scene);
        let background = self.background;
        let display_scale = self.display_scale;
        let resolver = self.asset_resolver.clone();
        let resolver_ref = resolver.as_deref();
        let cache = &mut self.image_cache;
        let instance_cache = &mut self.instance_cache;
        let boolean_cache = &mut self.boolean_cache;
        let path_cache = &mut self.path_cache;
        let layer_cache = &mut self.layer_cache;
        let pixel_snap_pan = self.pixel_snap_pan;
        let canvas = self.surface.canvas();

        Self::draw_scene_onto(
            canvas,
            width,
            height,
            background,
            display_scale,
            resolver_ref,
            cache,
            instance_cache,
            boolean_cache,
            path_cache,
            layer_cache,
            pixel_snap_pan,
            scene,
            viewport,
            page_root,
            inputs,
            true,
            &mut metrics,
        );

        metrics.frame_micros = started.elapsed().as_micros() as u64;
        metrics
    }

    /// Draw the scene through the viewport onto an **arbitrary** Skia canvas of
    /// `target_w` × `target_h` physical pixels, clearing it to `background`
    /// first. This is the single, shared scene-draw entry point: the CPU
    /// [`render_page_with`] calls it with the renderer's owned raster surface
    /// canvas, and the live-window GPU present (`fanta-app`) calls it with a
    /// Ganesh/Metal-backed surface canvas — so the scene-walk + viewport
    /// transform are defined in exactly one place and the two paths are
    /// pixel-identical.
    ///
    /// `target_w`/`target_h` are the canvas's physical pixel size (NOT
    /// necessarily this renderer's `width`/`height`), so the GPU path can pass
    /// the live drawable size. `display_scale` is the logical→physical
    /// multiplier (Retina = 2.0) and combines with `viewport.zoom` exactly as
    /// the CPU path expects.
    ///
    /// Returns [`RenderMetrics`] with `frame_micros` set; callers that want to
    /// time additional work (e.g. a GPU flush) can ignore it and time
    /// themselves.
    ///
    /// [`render_page_with`]: Self::render_page_with
    #[allow(clippy::too_many_arguments)]
    pub fn render_to_canvas(
        &mut self,
        canvas: &Canvas,
        target_w: u32,
        target_h: u32,
        scene: &Scene,
        viewport: &Viewport,
        page_root: Option<NodeId>,
        inputs: &RenderInputs,
    ) -> RenderMetrics {
        self.render_to_canvas_configured(
            canvas, target_w, target_h, scene, viewport, page_root, inputs, true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_to_canvas_configured(
        &mut self,
        canvas: &Canvas,
        target_w: u32,
        target_h: u32,
        scene: &Scene,
        viewport: &Viewport,
        page_root: Option<NodeId>,
        inputs: &RenderInputs,
        supports_offscreen_layers: bool,
    ) -> RenderMetrics {
        let started = Instant::now();
        let mut metrics = RenderMetrics::default();

        self.sync_scene_instance(scene);
        self.instance_cache
            .begin_frame(scene, inputs.mode_generation);
        self.boolean_cache.purge_removed(scene);
        self.path_cache.purge_removed(scene);
        let background = self.background;
        let display_scale = self.display_scale;
        let resolver = self.asset_resolver.clone();
        let resolver_ref = resolver.as_deref();
        let cache = &mut self.image_cache;
        let instance_cache = &mut self.instance_cache;
        let boolean_cache = &mut self.boolean_cache;
        let path_cache = &mut self.path_cache;
        let layer_cache = &mut self.layer_cache;
        let pixel_snap_pan = self.pixel_snap_pan;

        Self::draw_scene_onto(
            canvas,
            target_w,
            target_h,
            background,
            display_scale,
            resolver_ref,
            cache,
            instance_cache,
            boolean_cache,
            path_cache,
            layer_cache,
            pixel_snap_pan,
            scene,
            viewport,
            page_root,
            inputs,
            supports_offscreen_layers,
            &mut metrics,
        );

        metrics.frame_micros = started.elapsed().as_micros() as u64;
        metrics
    }

    /// Render one page (or all roots when `page_root` is `None`) to an
    /// inspectable SVG document without allocating the renderer's CPU pixel
    /// surface.
    ///
    /// This is a target adapter over the same scene walk used by
    /// [`Self::render_to_canvas`], so SVG, PNG, and host-owned GPU surfaces
    /// share culling, instance expansion, variable resolution, and traversal.
    /// Skia's SVG device cannot allocate the offscreen save-layers required by
    /// opacity, filters, masks, and some blend/effect combinations. The SVG
    /// adapter therefore bypasses those layers so inspectable geometry remains
    /// visible; callers must treat the result as approximate whenever those
    /// features occur. The headless harness reports them and rejects them in
    /// strict mode.
    pub fn render_page_svg_with(
        &mut self,
        scene: &Scene,
        viewport: &Viewport,
        page_root: Option<NodeId>,
        inputs: &RenderInputs,
        options: SvgRenderOptions,
    ) -> SvgRenderOutput {
        use skia_safe::svg::canvas::Flags;

        let mut flags = Flags::empty();
        if options.convert_text_to_paths {
            flags |= Flags::CONVERT_TEXT_TO_PATHS;
        }
        if options.relative_path_encoding {
            flags |= Flags::RELATIVE_PATH_ENCODING;
        }
        if !options.pretty_xml {
            flags |= Flags::NO_PRETTY_XML;
        }

        let svg_canvas = skia_safe::svg::Canvas::new(
            Rect::from_size((self.width as f32, self.height as f32)),
            flags,
        );
        let metrics = self.render_to_canvas_configured(
            &svg_canvas,
            self.width,
            self.height,
            scene,
            viewport,
            page_root,
            inputs,
            false,
        );
        let data = svg_canvas.end();
        SvgRenderOutput {
            svg: data.as_bytes().to_vec(),
            metrics,
        }
    }

    /// Render `page_root`'s subtree into a single TILE sub-rect of this
    /// renderer's OWN surface (physical px) WITHOUT clearing it — the CPU
    /// analogue of [`render_tile_onto`], for compositing onto the owned raster
    /// surface (the softbuffer present path) where the caller cannot hold an
    /// external canvas handle alongside a `&mut self`. Splits `surface` from the
    /// caches by hand so the borrows are disjoint, then delegates to the shared
    /// tile draw.
    ///
    /// [`render_tile_onto`]: Self::render_tile_onto
    #[allow(clippy::too_many_arguments)]
    pub fn render_tile_self(
        &mut self,
        tile_x: f32,
        tile_y: f32,
        tile_w: f32,
        tile_h: f32,
        background: Option<Color>,
        scene: &Scene,
        viewport: &Viewport,
        page_root: Option<NodeId>,
        inputs: &RenderInputs,
    ) {
        if tile_w < 1.0 || tile_h < 1.0 {
            return;
        }
        // See `render_page_with`: allocation failure degrades to a skipped
        // tile rather than a panic.
        if self.ensure_raster_surface().is_err() {
            return;
        }
        self.sync_scene_instance(scene);
        let display_scale = self.display_scale;
        let resolver = self.asset_resolver.clone();
        let cache = &mut self.image_cache;
        let instance_cache = &mut self.instance_cache;
        let boolean_cache = &mut self.boolean_cache;
        let path_cache = &mut self.path_cache;
        let layer_cache = &mut self.layer_cache;
        let canvas = self.surface.canvas();
        Self::draw_tile_onto(
            canvas,
            tile_x,
            tile_y,
            tile_w,
            tile_h,
            background,
            display_scale,
            resolver.as_deref(),
            cache,
            instance_cache,
            boolean_cache,
            path_cache,
            layer_cache,
            scene,
            viewport,
            page_root,
            inputs,
        );
    }

    /// Render `page_root`'s subtree into a single TILE sub-rect of `canvas`
    /// (physical px) WITHOUT clearing the whole surface — for compositing a grid
    /// of previews. Clips to the tile, optionally fills `background` behind the
    /// subtree, and centers the viewport transform on the tile. Each call is a
    /// self-contained `save`/`restore`, so tiles never wipe each other.
    #[allow(clippy::too_many_arguments)]
    pub fn render_tile_onto(
        &mut self,
        canvas: &Canvas,
        tile_x: f32,
        tile_y: f32,
        tile_w: f32,
        tile_h: f32,
        background: Option<Color>,
        scene: &Scene,
        viewport: &Viewport,
        page_root: Option<NodeId>,
        inputs: &RenderInputs,
    ) {
        if tile_w < 1.0 || tile_h < 1.0 {
            return;
        }
        self.sync_scene_instance(scene);
        let display_scale = self.display_scale;
        let resolver = self.asset_resolver.clone();
        let cache = &mut self.image_cache;
        let instance_cache = &mut self.instance_cache;
        let boolean_cache = &mut self.boolean_cache;
        let path_cache = &mut self.path_cache;
        let layer_cache = &mut self.layer_cache;
        Self::draw_tile_onto(
            canvas,
            tile_x,
            tile_y,
            tile_w,
            tile_h,
            background,
            display_scale,
            resolver.as_deref(),
            cache,
            instance_cache,
            boolean_cache,
            path_cache,
            layer_cache,
            scene,
            viewport,
            page_root,
            inputs,
        );
    }

    /// The shared tile clip + viewport transform + subtree walk, on already-split
    /// field references so it serves both the external-canvas
    /// ([`render_tile_onto`]) and owned-surface ([`render_tile_self`]) tile draws.
    ///
    /// [`render_tile_onto`]: Self::render_tile_onto
    /// [`render_tile_self`]: Self::render_tile_self
    #[allow(clippy::too_many_arguments)]
    fn draw_tile_onto(
        canvas: &Canvas,
        tile_x: f32,
        tile_y: f32,
        tile_w: f32,
        tile_h: f32,
        background: Option<Color>,
        display_scale: f64,
        resolver: Option<&dyn AssetResolver>,
        cache: &mut ImageCache,
        instance_cache: &mut InstanceCache,
        boolean_cache: &mut BooleanCache,
        path_cache: &mut PathCache,
        layer_cache: &mut LayerCache,
        scene: &Scene,
        viewport: &Viewport,
        page_root: Option<NodeId>,
        inputs: &RenderInputs,
    ) {
        if tile_w < 1.0 || tile_h < 1.0 {
            return;
        }
        let mut metrics = RenderMetrics::default();
        let tile = skia_safe::Rect::from_xywh(tile_x, tile_y, tile_w, tile_h);
        canvas.save();
        canvas.clip_rect(tile, None, true);
        // Same page-color-fills-the-viewport rule as `draw_scene_onto`: a
        // solid page background paints the whole tile, not the children AABB.
        let page_background = page_background_color(scene, page_root, inputs);
        if let Some(bg) = page_background.or(background) {
            let mut paint = skia_safe::Paint::default();
            paint.set_color(to_sk_color(bg));
            paint.set_anti_alias(false);
            canvas.draw_rect(tile, &paint);
        }
        let effective_scale = (viewport.zoom * display_scale) as f32;
        let visible = visible_world_rect(tile_w as u32, tile_h as u32, display_scale, viewport);
        canvas.translate((tile_x + tile_w * 0.5, tile_y + tile_h * 0.5));
        canvas.scale((effective_scale, effective_scale));
        canvas.translate((-viewport.center[0] as f32, -viewport.center[1] as f32));
        let mut ctx = RenderCtx {
            scene,
            resolver,
            cache,
            instance_cache,
            boolean_cache,
            path_cache,
            inputs,
            instance_mode_anchor: None,
            resolved_local_transforms: IdHashMap::default(),
            resolved_world_transforms: IdHashMap::default(),
            resolved_local_bounds: IdHashMap::default(),
            visible,
            effective_scale,
            page_background_root: page_background.and(page_root),
            supports_offscreen_layers: true,
            paint_alpha: 1.0,
            // Tiles are one-shot previews at their own scale/offset: the
            // layer cache is neither consulted nor populated for them.
            layer_cache,
            layer_cache_lookups: false,
            layer_cache_populate: false,
            layer_volatile: false,
            metrics: &mut metrics,
        };
        match page_root {
            Some(root) => render_node(canvas, root, &mut ctx),
            None => {
                for &root in scene.roots() {
                    render_node(canvas, root, &mut ctx);
                }
            }
        }
        canvas.restore();
    }

    /// The actual scene clear + viewport transform + z-order walk, working on
    /// already-split field references so it can serve both the owned-surface
    /// (CPU) and externally-owned (GPU) canvases without re-borrowing `self`.
    ///
    /// See [`page_background_color`] for how a page's canvas color becomes the
    /// full-viewport clear instead of a children-bounded group fill.
    #[allow(clippy::too_many_arguments)]
    fn draw_scene_onto(
        canvas: &Canvas,
        target_w: u32,
        target_h: u32,
        background: Color,
        display_scale: f64,
        resolver: Option<&dyn AssetResolver>,
        cache: &mut ImageCache,
        instance_cache: &mut InstanceCache,
        boolean_cache: &mut BooleanCache,
        path_cache: &mut PathCache,
        layer_cache: &mut LayerCache,
        pixel_snap_pan: bool,
        scene: &Scene,
        viewport: &Viewport,
        page_root: Option<NodeId>,
        inputs: &RenderInputs,
        supports_offscreen_layers: bool,
        metrics: &mut RenderMetrics,
    ) {
        // A page root carrying a solid background is the document's canvas
        // color: it must fill the whole viewport, not the union of its
        // children's bounds (which is all an unclipped group's fill can cover
        // in the walk). Resolve it here and clear the surface with it; the
        // walk then skips that root's own background paint so a translucent
        // page color is not composited twice.
        let page_background = page_background_color(scene, page_root, inputs);
        canvas.clear(to_sk_color(page_background.unwrap_or(background)));

        let half_w = target_w as f32 * 0.5;
        let half_h = target_h as f32 * 0.5;
        let effective_scale = (viewport.zoom * display_scale) as f32;

        // Derive the region of WORLD space the surface currently shows by
        // inverting exactly the transform applied below. See
        // [`visible_world_rect`] for the algebra; nodes whose world AABB misses
        // this rect are culled before they (or their subtree) are drawn.
        let visible = visible_world_rect(target_w, target_h, display_scale, viewport);

        // Apply the viewport transform: translate so `viewport.center` maps to
        // the (physical) screen center, then scale by `viewport.zoom *
        // display_scale`. The `display_scale` multiplier is what makes a
        // 100-world-unit shape land at 100 logical pixels (= 200 physical on
        // a 2x Retina display) instead of 100 physical pixels.
        //
        // The device translation `half − scale · center` is what carries a
        // pan's sub-pixel phase; with `pixel_snap_pan` it is rounded to whole
        // device pixels first (see `set_pixel_snap_pan`) — computed in f64 so
        // the rounding sees the same value on every frame.
        canvas.save();
        let device_tx = f64::from(half_w) - f64::from(effective_scale) * viewport.center[0];
        let device_ty = f64::from(half_h) - f64::from(effective_scale) * viewport.center[1];
        let (root_tx, root_ty) = if pixel_snap_pan {
            (device_tx.round(), device_ty.round())
        } else {
            (device_tx, device_ty)
        };
        if pixel_snap_pan {
            canvas.translate((root_tx as f32, root_ty as f32));
            canvas.scale((effective_scale, effective_scale));
        } else {
            canvas.translate((half_w, half_h));
            canvas.scale((effective_scale, effective_scale));
            canvas.translate((-viewport.center[0] as f32, -viewport.center[1] as f32));
        }

        // The effect-layer cache's frame bookkeeping: everything it holds is
        // dropped when the content epoch moves; a frame may populate it only
        // when it repeats the previous frame's scale and pans by whole device
        // pixels (see `LayerCache::begin_frame`). Motion playback moves nodes
        // without touching any epoch input, so a motion frame neither
        // consults nor fills the cache.
        let layer_cache_populate = layer_cache.begin_frame(
            canvas,
            LayerEpoch {
                scene_instance: scene.instance_id(),
                scene_revision: scene.revision(),
                mode_generation: inputs.mode_generation,
                dark_ui: inputs.dark_ui,
            },
            effective_scale,
            root_tx,
            root_ty,
        );
        let layer_cache_lookups =
            supports_offscreen_layers && inputs.motion.is_none() && layer_cache.is_enabled();

        // Walk roots in z-order. Sub-tree recursion via the helper below.
        let mut ctx = RenderCtx {
            scene,
            resolver,
            cache,
            instance_cache,
            boolean_cache,
            path_cache,
            inputs,
            instance_mode_anchor: None,
            resolved_local_transforms: IdHashMap::default(),
            resolved_world_transforms: IdHashMap::default(),
            resolved_local_bounds: IdHashMap::default(),
            visible,
            effective_scale,
            page_background_root: page_background.and(page_root),
            supports_offscreen_layers,
            paint_alpha: 1.0,
            layer_cache,
            layer_cache_lookups,
            layer_cache_populate: layer_cache_lookups && layer_cache_populate,
            layer_volatile: false,
            metrics,
        };
        match page_root {
            // Single page: walk just that root's subtree. Culling and the
            // save/restore stack work identically to the all-roots path.
            Some(root) => render_node(canvas, root, &mut ctx),
            // All roots in z-order (legacy / single-implicit-page behavior).
            None => {
                for &root in scene.roots() {
                    render_node(canvas, root, &mut ctx);
                }
            }
        }

        canvas.restore();
    }

    /// Copy the current pixels into a freshly-allocated RGBA8 buffer
    /// (row-major, 8 bits per channel, **straight alpha**). Skia handles the
    /// conversion from its native premultiplied native-32 layout — we ask for
    /// `RGBA_8888` explicitly so callers don't have to think about platform
    /// endianness.
    ///
    /// Use after [`render`].
    ///
    /// [`render`]: Self::render
    pub fn copy_rgba(&mut self) -> Vec<u8> {
        let info = ImageInfo::new(
            (self.width as i32, self.height as i32),
            ColorType::RGBA8888,
            AlphaType::Unpremul,
            None,
        );
        let row_bytes = info.min_row_bytes();
        let mut buf = vec![0u8; row_bytes * self.height as usize];
        // A never-rendered renderer still holds the pixel-free placeholder;
        // its `read_pixels` reads nothing, leaving the zeroed buffer — byte-
        // identical to reading the freshly-zeroed surface the eager
        // constructor used to allocate, without forcing the allocation here.
        // On a real (materialized) surface the read must succeed; a failure
        // there would silently return a blank frame, so assert the invariant.
        let read = self.surface.read_pixels(&info, &mut buf, row_bytes, (0, 0));
        debug_assert!(
            read || self.surface_is_placeholder,
            "read_pixels failed on a materialized surface; copy_rgba would return a blank frame"
        );
        buf
    }

    /// Encode the current surface contents as PNG. Use after [`render`].
    ///
    /// [`render`]: Self::render
    pub fn encode_png(&mut self) -> Result<Vec<u8>, RenderError> {
        // Snapshotting the pixel-free placeholder is undefined (Skia returns
        // no image for a null surface), so materialize the raster surface
        // first — a never-rendered renderer encodes a blank PNG, as before.
        self.ensure_raster_surface()?;
        let image = self.surface.image_snapshot();
        // CPU surfaces don't need a GPU `DirectContext`; passing `None` is the
        // documented shape for raster encode.
        let data = image
            .encode(None, EncodedImageFormat::PNG, 100)
            .ok_or(RenderError::Encode)?;
        Ok(data.as_bytes().to_vec())
    }
}

/// The solid canvas color a page root contributes to the whole viewport, or
/// `None` when the render root is not a plain page-style group with a solid
/// background. A page (Figma CANVAS) is an unclipped, unsized group; its
/// `background` is the document's canvas color and must cover the entire
/// visible surface. Sized/clipped groups (frames rendered as a root) keep the
/// ordinary walk-painted background. Non-solid page fills (gradient/image)
/// also fall back to the walk, which at least paints the content bounds.
fn page_background_color(
    scene: &Scene,
    page_root: Option<NodeId>,
    inputs: &RenderInputs,
) -> Option<Color> {
    let id = page_root?;
    let root = scene.get(id)?;
    // Match the walk's semantics: a HIDDEN root paints nothing (the clear
    // must fall back), and a translucent root composites through an opacity
    // layer the clear can't reproduce — leave both to the walk-painted
    // (bounded) background instead of flooding the viewport.
    if root.flags.contains(super::NodeFlags::HIDDEN) || root.opacity.get() < 1.0 {
        return None;
    }
    // Resolve variable bindings the same way the walk's overlay does, so a
    // canvas color bound to a theme variable clears with the resolved value,
    // not the stale literal. (Motion overrides are not applied here — a
    // motion-animated page background falls back to the walk.)
    let resolved;
    let root = if root.bindings.is_empty() {
        root
    } else {
        let mut scratch = root.clone();
        for (prop, var_id) in &root.bindings {
            if let Some(value) = super::resolve_bound_value(
                inputs.variables,
                scene,
                id,
                inputs.active_modes,
                *var_id,
            ) {
                prop.apply_resolved(&mut scratch, value);
            }
        }
        resolved = scratch;
        &resolved
    };
    match &root.data {
        NodeData::Group(group) if group.clip_size.is_none() && group.local_size.is_none() => {
            match &group.background {
                Some(Fill::Solid { color, .. }) => Some(*color),
                _ => None,
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod instance_cache_tests {
    use super::*;
    use fanta_doc::{ComponentId, InstanceNode, Override, OverrideValue};

    fn instance(component: ComponentId) -> InstanceNode {
        InstanceNode {
            component,
            overrides: Vec::new(),
            prop_values: Default::default(),
            derived: Vec::new(),
            local_size: [10.0, 10.0],
        }
    }

    fn key(instance: NodeId, override_hash: u64, mode_generation: u64) -> InstanceCacheKey {
        InstanceCacheKey {
            instance,
            rev: 1,
            preview_rev: 0,
            override_hash,
            mode_generation,
            mode_pins: 0,
        }
    }

    #[test]
    fn override_hash_is_served_from_the_stamp_memo_until_the_stamp_moves() {
        let mut cache = InstanceCache::default();
        let id = NodeId::new();
        let plain = instance(ComponentId::new());
        let mut hidden = plain.clone();
        hidden.overrides.push(Override {
            target_path: Default::default(),
            target_prop: fanta_doc::BoundProp::Visible,
            value: OverrideValue::SwapInstance {
                component: ComponentId::new(),
            },
        });
        assert_eq!(
            cache.override_hash(id, Some(1), &plain),
            hash_overrides(&plain)
        );
        // Same stamp: the memo answers, whatever the node now says — the
        // contract is that a data edit always moves the stamp.
        assert_eq!(
            cache.override_hash(id, Some(1), &hidden),
            hash_overrides(&plain)
        );
        assert_eq!(
            cache.override_hash(id, Some(2), &hidden),
            hash_overrides(&hidden)
        );
        // A transient clone (no stamp) is always hashed directly.
        assert_eq!(
            cache.override_hash(id, None, &plain),
            hash_overrides(&plain)
        );
    }

    #[test]
    fn the_override_hash_covers_the_instance_box() {
        let plain = instance(ComponentId::new());
        let mut resized = plain.clone();
        resized.local_size = [20.0, 10.0];
        assert_ne!(
            hash_overrides(&plain),
            hash_overrides(&resized),
            "an instance resize alone must re-key its expansion"
        );
        assert_eq!(hash_overrides(&plain), hash_overrides(&plain.clone()));
    }

    #[test]
    fn a_changed_key_replaces_the_instances_entry_instead_of_leaking() {
        let mut cache = InstanceCache::default();
        let id = NodeId::new();
        cache.insert(key(id, 1, 0), Arc::new(Vec::new()));
        assert!(cache.lookup(&key(id, 1, 0)).is_some());
        assert!(
            cache.lookup(&key(id, 2, 0)).is_none(),
            "a new override hash misses"
        );
        cache.insert(key(id, 2, 0), Arc::new(Vec::new()));
        assert_eq!(cache.len(), 1, "one entry per instance");
        assert!(cache.lookup(&key(id, 1, 0)).is_none());
        assert!(cache.lookup(&key(id, 2, 0)).is_some());
    }

    #[test]
    fn entries_from_another_mode_generation_or_idle_too_long_are_evicted() {
        let scene = Scene::new();
        let mut cache = InstanceCache::default();
        let live = NodeId::new();
        let stale_mode = NodeId::new();
        cache.begin_frame(&scene, 7);
        cache.insert(key(live, 1, 7), Arc::new(Vec::new()));
        cache.insert(key(stale_mode, 1, 6), Arc::new(Vec::new()));
        cache.begin_frame(&scene, 7);
        assert!(cache.lookup(&key(stale_mode, 1, 6)).is_none());
        assert!(cache.lookup(&key(live, 1, 7)).is_some());

        let idle = NodeId::new();
        cache.insert(key(idle, 1, 7), Arc::new(Vec::new()));
        for _ in 0..INSTANCE_CACHE_IDLE_FRAMES {
            cache.begin_frame(&scene, 7);
            // `live` is looked up every frame and must survive.
            assert!(cache.lookup(&key(live, 1, 7)).is_some());
        }
        // Probe the map directly: a lookup would count as a use.
        assert!(cache.entries.contains_key(&idle), "still inside the window");
        cache.begin_frame(&scene, 7);
        assert!(
            !cache.entries.contains_key(&idle),
            "one frame past the window"
        );
        assert!(cache.lookup(&key(live, 1, 7)).is_some());
    }

    #[test]
    fn override_hash_memo_is_purged_of_removed_nodes_when_the_scene_says_so() {
        let mut scene = Scene::new();
        let node =
            fanta_doc::CanvasNode::new(fanta_doc::NodeData::Instance(instance(ComponentId::new())));
        let id = node.id;
        scene.insert(node).unwrap();
        let mut cache = InstanceCache::default();
        cache.begin_frame(&scene, 0);
        let Some(fanta_doc::NodeData::Instance(inst)) = scene.get(id).map(|node| &node.data) else {
            panic!("instance node");
        };
        cache.override_hash(id, Some(scene.node_stamp(id)), inst);
        assert_eq!(cache.override_hashes.len(), 1);
        cache.begin_frame(&scene, 0);
        assert_eq!(cache.override_hashes.len(), 1, "no removal, no scan");
        scene.remove(id).unwrap();
        cache.begin_frame(&scene, 0);
        assert!(cache.override_hashes.is_empty());
    }
}
