//! [`RasterRenderer`] — the renderer struct, its constructors and render
//! entry points, the per-frame [`RenderInputs`] context, and the cross-frame
//! instance-expansion memo ([`InstanceCache`]). The actual scene walk lives in
//! the sibling [`walk`](super::walk) module; this file owns the public surface
//! and the per-frame state plumbing.
//!
//! [`InstanceCache`]: InstanceCache
use super::{
    AlphaType, Arc, AssetResolver, BTreeMap, Canvas, Color, ColorType, ComponentLibrary,
    EncodedImageFormat, ExpandedNode, Hash, HashMap, Hasher, ImageCache, ImageInfo, InstanceNode,
    Instant, ModeId, NodeId, RenderCtx, Scene, Surface, VariableCollectionId, VariableRegistry,
    Viewport, render_node, surfaces, to_sk_color, visible_world_rect,
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
#[derive(Debug, Clone, Copy)]
pub struct MediaPlayback {
    /// Playback position in `0.0..=1.0`.
    pub progress: f32,
    /// The video frame to show at this position (a filmstrip asset id), or
    /// `None` for audio / a paused video (which shows its poster).
    pub frame: Option<fanta_doc::AssetId>,
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
    /// A monotonically-increasing counter the app bumps whenever `active_modes`
    /// (or any variable value) changes. Folded into the instance-memo key so a
    /// mode switch can't serve a stale expansion. Today instance *structure* is
    /// mode-independent (modes only affect the paint-time binding overlay, not
    /// which nodes exist), so a constant value is correct; the field exists so
    /// the key stays correct if a future change bakes modes into expansion.
    pub mode_generation: u64,
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
/// `mode_generation`. Stale entries are simply never looked up again; the cache
/// is bounded only by distinct live keys, and `clear_instance_cache` drops it
/// wholesale (e.g. on document switch).
#[derive(Default)]
pub(crate) struct InstanceCache {
    pub(crate) entries: HashMap<InstanceCacheKey, Arc<Vec<ExpandedNode>>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct InstanceCacheKey {
    pub(crate) instance: NodeId,
    pub(crate) rev: u64,
    pub(crate) override_hash: u64,
    pub(crate) mode_generation: u64,
}

impl InstanceCache {
    fn clear(&mut self) {
        self.entries.clear();
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Hash an instance's overrides into one `u64` for the memo key. The overrides
/// `Vec<Override>` is not `Hash` (it carries `f64`s via `OverrideValue::Field`
/// JSON and `Fills`), so we route it through its serde JSON projection — stable
/// for a given override set and cheap relative to a full subtree clone. A hash
/// collision would at worst serve a wrong-but-same-shape expansion; the input
/// space (overrides on one instance between two edits) makes that negligible,
/// and any real edit also bumps `rev` or `mode_generation`.
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
        })
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

    /// Number of distinct instance expansions currently memoized. For tests and
    /// memory diagnostics.
    pub fn instance_cache_len(&self) -> usize {
        self.instance_cache.len()
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
        let background = self.background;
        let display_scale = self.display_scale;
        let resolver = self.asset_resolver.clone();
        let resolver_ref = resolver.as_deref();
        let cache = &mut self.image_cache;
        let instance_cache = &mut self.instance_cache;
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
            scene,
            viewport,
            page_root,
            inputs,
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
        let started = Instant::now();
        let mut metrics = RenderMetrics::default();

        let background = self.background;
        let display_scale = self.display_scale;
        let resolver = self.asset_resolver.clone();
        let resolver_ref = resolver.as_deref();
        let cache = &mut self.image_cache;
        let instance_cache = &mut self.instance_cache;

        Self::draw_scene_onto(
            canvas,
            target_w,
            target_h,
            background,
            display_scale,
            resolver_ref,
            cache,
            instance_cache,
            scene,
            viewport,
            page_root,
            inputs,
            &mut metrics,
        );

        metrics.frame_micros = started.elapsed().as_micros() as u64;
        metrics
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
        let display_scale = self.display_scale;
        let resolver = self.asset_resolver.clone();
        let cache = &mut self.image_cache;
        let instance_cache = &mut self.instance_cache;
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
        let display_scale = self.display_scale;
        let resolver = self.asset_resolver.clone();
        let cache = &mut self.image_cache;
        let instance_cache = &mut self.instance_cache;
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
        if let Some(bg) = background {
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
            inputs,
            visible,
            effective_scale,
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
        scene: &Scene,
        viewport: &Viewport,
        page_root: Option<NodeId>,
        inputs: &RenderInputs,
        metrics: &mut RenderMetrics,
    ) {
        // Clear to background.
        canvas.clear(to_sk_color(background));

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
        canvas.save();
        canvas.translate((half_w, half_h));
        canvas.scale((effective_scale, effective_scale));
        canvas.translate((-viewport.center[0] as f32, -viewport.center[1] as f32));

        // Walk roots in z-order. Sub-tree recursion via the helper below.
        let mut ctx = RenderCtx {
            scene,
            resolver,
            cache,
            instance_cache,
            inputs,
            visible,
            effective_scale,
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
        let _ = self.surface.read_pixels(&info, &mut buf, row_bytes, (0, 0));
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
