//! Node-level compositing effects: drop/inner shadows, layer + background
//! blur, blend-mode mapping, and the on-screen sigma cap + shadow-expanded
//! cull-bounds math that bounds their cost. Shared by both the live-scene
//! walk and the transient instance walk.
use super::boolean::fold_operands;
use super::{
    BlendMode, Blur, BlurKind, Bounds, Canvas, CanvasNode, Fill, GroupNode, NodeData, NodeFlags,
    NodeId, Paint, RenderCtx, Scene, Shadow, ShadowKind, bounds_to_f32, frame_box_bounds,
    path_is_rect, rounded_rect_path, text_node_outline, text_path_bounds, text_path_outline,
    to_sk_blend_mode, to_sk_color, to_sk_fill_path,
};

/// Which of a node's authored effects actually PAINT at the frame's
/// `effective_scale` — its zoom-effective effect set.
///
/// Every effect already carries a zoom-out LOD (a layer blur whose on-screen
/// sigma is under [`SIGMA_SCREEN_MIN`] builds no filter, a drop/inner shadow
/// that is sub-pixel per [`shadow_is_subpixel`] is skipped, a zero-alpha shadow
/// paints nothing), but the WALK used to decide "does this node need an
/// effects layer / may its opacity fold" from the *authored* lists. On the
/// Agency template Design page 6,443 vectors carry a layer blur, 81% of them
/// under 0.25 device px at 10% zoom: their blur was (correctly) skipped, yet
/// their opacity still cost a save-layer each because the fold saw a non-empty
/// `blurs` (measured: 2,379 layers / 4 folds → 651 / 1,732 once the fold and
/// layer decisions read this summary instead — ~75 ms of a 230 ms frame).
///
/// Computed once per node per frame by the walk (see `render_node`) and
/// threaded to [`opacity_folds_into_paint`] and the layer decision; the filter
/// builders ([`build_drop_shadow_filter`], [`build_layer_blur_filter`],
/// [`draw_inner_shadows`]) use the SAME predicates ([`drop_shadow_paints`],
/// [`inner_shadow_paints`], [`layer_blur_paints`]) so the summary can never
/// disagree with what is drawn. Background blur is not part of it: it frosts the
/// backdrop BEFORE the node's layer and never influences the layer/fold choice
/// (see [`apply_background_blur`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct VisibleEffects {
    /// ≥1 drop shadow paints (non-zero alpha and not sub-pixel at this scale).
    pub(crate) drop_shadow: bool,
    /// ≥1 inner shadow paints (non-zero alpha and not sub-pixel at this scale).
    pub(crate) inner_shadow: bool,
    /// The combined layer blur has a visible on-screen sigma at this scale.
    pub(crate) layer_blur: bool,
}

impl VisibleEffects {
    /// Whether ANY effect that composites through the node's effects layer
    /// paints — i.e. the node needs a layer for effects (independent of
    /// opacity / blend / isolation), and its opacity therefore cannot fold.
    /// Inner shadows are drawn as a separate pass inside the layer, so they
    /// count too: with a fold, the shadow would paint at full alpha OVER a
    /// translucent body instead of being flattened with it first.
    pub(crate) fn any(self) -> bool {
        self.drop_shadow || self.inner_shadow || self.layer_blur
    }
}

/// The zoom-effective effect summary of `node` at `effective_scale` (see
/// [`VisibleEffects`]). Cheap: a node with no effects and no blurs — the
/// overwhelming majority — returns the all-false default after two `is_empty`
/// checks.
pub(crate) fn visible_effects(node: &CanvasNode, effective_scale: f32) -> VisibleEffects {
    let mut visible = VisibleEffects::default();
    for shadow in &node.effects {
        match shadow.kind {
            ShadowKind::Drop => {
                visible.drop_shadow |= drop_shadow_paints(shadow, effective_scale);
            }
            ShadowKind::Inner => {
                visible.inner_shadow |= inner_shadow_paints(shadow, effective_scale);
            }
        }
    }
    visible.layer_blur = layer_blur_paints(&node.blurs, effective_scale);
    visible
}

/// Whether a drop shadow paints at all at `effective_scale`: a non-zero
/// alpha AND not sub-pixel (see [`shadow_is_subpixel`]). The single predicate
/// [`build_drop_shadow_filter`] and [`visible_effects`] share.
pub(crate) fn drop_shadow_paints(shadow: &Shadow, effective_scale: f32) -> bool {
    shadow.kind == ShadowKind::Drop
        && shadow.color.a != 0
        && !shadow_is_subpixel(shadow, effective_scale)
}

/// Whether an inner shadow paints at all at `effective_scale` — the same
/// alpha + sub-pixel test as a drop shadow's; shared by [`draw_inner_shadows`]
/// and [`visible_effects`].
pub(crate) fn inner_shadow_paints(shadow: &Shadow, effective_scale: f32) -> bool {
    shadow.kind == ShadowKind::Inner
        && shadow.color.a != 0
        && !shadow_is_subpixel(shadow, effective_scale)
}

/// The combined WORLD-space sigma of every LAYER blur in `blurs` (independent
/// Gaussians add in quadrature, `σ² = Σσᵢ²`), or `0` when there is none.
pub(crate) fn layer_blur_world_sigma(blurs: &[Blur]) -> f64 {
    let sigma_sq: f64 = blurs
        .iter()
        .filter(|b| b.kind == BlurKind::Layer)
        .map(|b| {
            let s = blur_world_sigma(b);
            s * s
        })
        .sum();
    if sigma_sq > 0.0 { sigma_sq.sqrt() } else { 0.0 }
}

/// Whether the node's combined layer blur is visible at `effective_scale`,
/// i.e. its capped on-screen sigma clears [`SIGMA_SCREEN_MIN`] (see
/// [`capped_render_sigma`]). Shared by [`build_layer_blur_filter`] and
/// [`visible_effects`].
pub(crate) fn layer_blur_paints(blurs: &[Blur], effective_scale: f32) -> bool {
    let world_sigma = layer_blur_world_sigma(blurs);
    world_sigma > 0.0 && capped_render_sigma(world_sigma, effective_scale) > 0.0
}

/// Whether a node's `opacity < 1` can be applied as **paint alpha** instead of
/// an opacity save-layer, with pixel-identical output.
///
/// A save-layer at alpha `a` around exactly ONE draw composites the same pixels
/// as that draw with its paint alpha multiplied by `a` (`a · (coverage · color)`
/// either way). That holds only for a leaf `Vector` that emits a single
/// Solid/Gradient draw with a Normal paint blend — one fill and no drawing
/// stroke, or one drawing stroke and no fill (zero-width strokes paint nothing
/// and do not count) — and that needs the layer for nothing else: no drop or
/// inner shadow and no layer blur THAT PAINTS AT THIS ZOOM (`visible`, see
/// [`VisibleEffects`]), Normal node blend, not isolated, no children.
/// Anything with two or more draws must keep the layer, because overlapping
/// draws inside a layer are flattened BEFORE the alpha applies (a stroke fully
/// covers the fill beneath it), whereas per-draw alpha would let the fill show
/// through the stroke. Image paints, per-side borders (four overlapping bands),
/// and per-paint blend modes are excluded for the same reason.
///
/// An authored blur or shadow that is sub-pixel at this scale does not block
/// the fold: its filter is skipped anyway (see [`build_layer_blur_filter`] /
/// [`build_drop_shadow_filter`]), so the layer it used to force was a pure
/// opacity layer around one draw — exactly what the fold replaces,
/// pixel-identically. A background blur never blocks it either: it frosts the
/// backdrop before the node's own draw, outside any layer, so
/// `frost; layer(a){draw}` and `frost; draw·a` composite the same pixels.
///
/// Why this matters: on a zoomed-out icon sheet thousands of translucent leaf
/// vectors each cost a Ganesh render task + Metal render pass for a layer that
/// holds one draw. Folding removes ~70% of that frame's cost (measured on the
/// Agency template Icons page: 145 → ~45 ms at 10% zoom).
pub(crate) fn opacity_folds_into_paint(
    node: &CanvasNode,
    has_children: bool,
    visible: VisibleEffects,
) -> bool {
    if has_children
        || node.blend_mode != BlendMode::Normal
        || visible.any()
        || node.flags.contains(NodeFlags::ISOLATED_BLEND)
    {
        return false;
    }
    let NodeData::Vector(vector) = &node.data else {
        return false;
    };
    let mut draws = 0usize;
    for fill in &vector.fills {
        match fill {
            Fill::Solid { blend, .. } | Fill::Gradient { blend, .. } if blend.is_normal() => {
                draws += 1;
            }
            _ => return false,
        }
    }
    for stroke in &vector.strokes {
        if stroke.per_side.is_some() || !stroke.width.is_finite() {
            return false;
        }
        // A zero/negative-width stroke paints nothing (see `stroke_sk_path`).
        if stroke.width <= 0.0 {
            continue;
        }
        match &stroke.paint {
            Fill::Solid { blend, .. } | Fill::Gradient { blend, .. } if blend.is_normal() => {
                draws += 1;
            }
            _ => return false,
        }
    }
    draws == 1
}

/// Push a save-layer carrying this node's compositing effects — opacity, drop
/// shadow(s), and blend mode — returning whether one was pushed (so the caller
/// balances it with a `restore`).
///
/// A layer is needed when ANY of these hold:
/// - `opacity < 1.0` (the legacy reason);
/// - the node has ≥1 **drop** shadow (an `ImageFilter` paints the shadow behind
///   the node silhouette);
/// - the node's `blend_mode` is non-`Normal` (the layer composites against the
///   backdrop with that mode);
/// - `isolate` is set ([`NodeFlags::ISOLATED_BLEND`](super::NodeFlags)) — the
///   container's children must be flattened into their own group before
///   compositing, so a descendant's blend mode reads the group's contents
///   rather than leaking through to the backdrop (Figma's "Pass through"
///   toggle, inverted). The isolation layer is bounded by the same padded
///   content box as every other effects layer.
///
/// When none hold (the overwhelmingly common case: full opacity, no shadows,
/// `Normal` blend, pass-through) this allocates nothing and returns `false`, so
/// the existing fast path is untouched — no regression, no extra layer.
///
/// The layer's `Paint` carries all three at once: `alpha_f` for opacity, the
/// merged drop-shadow `ImageFilter` (see [`build_drop_shadow_filter`]) so the
/// shadow follows the WHOLE composited node + its subtree, and the mapped Skia
/// blend mode (see [`to_sk_blend_mode`]). Because the filter and blend live on
/// the layer paint, the shadow is computed from the node's full silhouette and
/// then the silhouette + shadow blend against the backdrop together — exactly
/// Figma's node-level effect semantics.
///
/// `effective_scale` (`viewport.zoom · display_scale`) is threaded into the
/// shadow filter so its on-screen Gaussian sigma can be capped — see
/// [`build_drop_shadow_filter`]. If every effect is an inner shadow, a
/// zero-alpha drop shadow, or a sub-pixel blur, `shadow_filter` is `None` and no
/// filter is attached.
///
/// **Inner shadows are NOT a layer effect.** Skia has no direct inner-shadow
/// image filter, and an inner shadow darkens the *inside* of the node's
/// silhouette rather than compositing the whole layer — so it cannot ride on the
/// layer paint here. It is painted separately, on top of the node's own content
/// and clipped to its shape, by [`draw_inner_shadows`]; this function ignores
/// `ShadowKind::Inner` entirely (only drop shadows feed the layer filter).
#[allow(clippy::too_many_arguments)]
pub(crate) fn begin_effects_layer(
    canvas: &Canvas,
    opacity: f32,
    blend_mode: BlendMode,
    effects: &[Shadow],
    blurs: &[Blur],
    isolate: bool,
    effective_scale: f32,
    content_bounds: Option<Bounds>,
) -> bool {
    let Some(layer) = effects_layer_paint(
        opacity,
        blend_mode,
        effects,
        blurs,
        isolate,
        effective_scale,
    ) else {
        return false;
    };
    // Bound the layer to the node's content box (in the CURRENT — node-local —
    // canvas space). Without explicit bounds Skia sizes the layer from the
    // clip, i.e. the whole viewport: on a zoomed-out page where hundreds of
    // small cards carry a drop shadow, that is hundreds of viewport-sized
    // offscreen allocations + filters PER FRAME (measured: a 2,100-node page
    // at fit-zoom took ~3s/frame; bounded, ~30ms). The bounds describe the
    // layer's SOURCE content — Skia derives the filter's output reach (offset/
    // blur/spread) itself — padded generously below for strokes, AA, and
    // modest subtree overshoot. `None` (no resolvable box) keeps the old
    // unbounded behavior.
    let paint = layer.full_paint();
    let rec = skia_safe::canvas::SaveLayerRec::default().paint(&paint);
    if let Some(b) = content_bounds {
        canvas.save_layer(&rec.bounds(&padded_layer_rect(&b, effective_scale)));
    } else {
        canvas.save_layer(&rec);
    }
    true
}

/// The paint an effects layer composites with, split into its two halves:
/// the image FILTER (drop shadows + layer blur), which Skia applies to the
/// layer's pixels at `restore`, and the ALPHA + BLEND, applied when the
/// filtered result is drawn onto the backdrop. [`begin_effects_layer`] uses
/// both on one save-layer paint ([`Self::full_paint`]); the layer cache
/// renders the layer with [`Self::filter_paint`] into an offscreen and later
/// draws the cached image with [`Self::composite_paint`] — the same two steps
/// Skia performs, so the pixels agree (see [`super::layer_cache`]).
pub(crate) struct EffectsLayerPaint {
    /// The merged drop-shadow + layer-blur filter, or `None` when the layer is
    /// needed only for opacity / blend / isolation.
    pub(crate) filter: Option<skia_safe::ImageFilter>,
    pub(crate) opacity: f32,
    pub(crate) blend_mode: BlendMode,
}

impl EffectsLayerPaint {
    /// The single save-layer paint carrying filter + alpha + blend.
    pub(crate) fn full_paint(&self) -> Paint {
        let mut paint = self.composite_paint();
        if let Some(filter) = &self.filter {
            paint.set_image_filter(filter.clone());
        }
        paint
    }

    /// Filter only (alpha 1, Normal blend): what the layer's pixels look
    /// like right before Skia composites them.
    pub(crate) fn filter_paint(&self) -> Paint {
        let mut paint = Paint::default();
        if let Some(filter) = &self.filter {
            paint.set_image_filter(filter.clone());
        }
        paint
    }

    /// Alpha + blend only: how the filtered pixels meet the backdrop.
    pub(crate) fn composite_paint(&self) -> Paint {
        let mut paint = Paint::default();
        if self.opacity < 1.0 {
            paint.set_alpha_f(self.opacity);
        }
        if self.blend_mode != BlendMode::Normal {
            paint.set_blend_mode(to_sk_blend_mode(self.blend_mode));
        }
        paint
    }
}

/// The effects-layer paint for a node with these compositing inputs, or
/// `None` when no layer is needed (full opacity, no visible drop shadow, no
/// visible layer blur, `Normal` blend, no isolation) — the conditions
/// [`begin_effects_layer`] documents. `effective_scale` caps the filters'
/// on-screen sigmas exactly as there.
pub(crate) fn effects_layer_paint(
    opacity: f32,
    blend_mode: BlendMode,
    effects: &[Shadow],
    blurs: &[Blur],
    isolate: bool,
    effective_scale: f32,
) -> Option<EffectsLayerPaint> {
    // Drop shadow(s) first; then a LAYER blur chained on top so the node's
    // content AND its shadow are Gaussian-blurred together (Figma's layer-blur
    // semantics — the whole composited layer is blurred). Background blurs are
    // NOT a layer-paint effect (they sample the backdrop, not the node) and are
    // handled separately by [`apply_background_blur`].
    let shadow_filter = build_drop_shadow_filter(effects, effective_scale);
    let filter = build_layer_blur_filter(blurs, effective_scale, shadow_filter);
    let non_normal_blend = blend_mode != BlendMode::Normal;
    // Nothing to do: full opacity, no drop shadow, no layer blur, normal
    // blend, and no isolation requested.
    if opacity >= 1.0 && filter.is_none() && !non_normal_blend && !isolate {
        return None;
    }
    Some(EffectsLayerPaint {
        filter,
        opacity,
        blend_mode,
    })
}

/// The save-layer bounds rect for a content box `b` in the CURRENT (node-local)
/// canvas space: `b` padded by [`LAYER_BOUNDS_PAD_DEVICE_PX`] converted through
/// `effective_scale`. Padding in DEVICE pixels keeps the safety margin
/// zoom-independent: it covers outside-aligned strokes, anti-aliasing, and
/// small geometry overshoot (glyph overhang, a child stroke at the box edge)
/// at any zoom without re-deriving per-node stroke reach. Shared by the
/// effects layer and the mask layers so both bound their offscreens
/// identically.
pub(crate) fn padded_layer_rect(b: &Bounds, effective_scale: f32) -> skia_safe::Rect {
    let scale = f64::from(effective_scale);
    let pad = if scale.is_finite() && scale > 0.0 {
        LAYER_BOUNDS_PAD_DEVICE_PX / scale
    } else {
        LAYER_BOUNDS_PAD_DEVICE_PX
    };
    skia_safe::Rect::new(
        (b.min_x - pad) as f32,
        (b.min_y - pad) as f32,
        (b.max_x + pad) as f32,
        (b.max_y + pad) as f32,
    )
}

/// Device-pixel padding added around an effects layer's content bounds (see
/// [`begin_effects_layer`]). Covers what the node's local-bounds box does not:
/// outside-aligned strokes, anti-aliased edges, and small subtree overshoot
/// like glyph overhang. 32 device px each side keeps even a zoomed-in 64px
/// stroke inside the layer while remaining orders of magnitude smaller than
/// the unbounded (viewport-sized) layer it replaces.
pub(crate) const LAYER_BOUNDS_PAD_DEVICE_PX: f64 = 32.0;

/// The node-LOCAL content box an effects layer should be bounded to, or `None`
/// when no reliable box exists (then the layer stays unbounded — correct, just
/// slower). Priority order:
/// - a clipped frame's `clip_size` box — overlay-accurate (a binding can
///   resize the clip, and descendants are clipped to it anyway);
/// - the node's own intrinsic geometry (vector path bounds, shaped text-path
///   glyph bounds, or text/bitmap/media `local_size`) — also valid for
///   TRANSIENT instance-expansion clones that have no scene entry;
/// - the scene's memoized subtree `local_bounds` for a live unclipped group
///   (a TRANSIENT unclipped group resolves to `None` here; the instance walk
///   falls back to the union of its transient children).
pub(crate) fn effects_layer_bounds(
    node: &CanvasNode,
    scene_id: Option<NodeId>,
    scene: &Scene,
) -> Option<Bounds> {
    match &node.data {
        // A live unclipped group falls back to the scene's memoized subtree
        // bounds; a transient one resolves to `None` (the instance walk unions
        // its transient children instead).
        NodeData::Group(group) if !group_clips_children(node, group) => {
            scene_id.and_then(|id| scene.local_bounds(id))
        }
        NodeData::TextPath(text_path) => text_path_bounds(text_path),
        data => data.local_bounds(),
    }
}

pub(crate) fn group_clips_children(node: &CanvasNode, group: &GroupNode) -> bool {
    group.clip_size.is_some()
        && node
            .meta
            .get("clip_content")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true)
}

/// Paint the node's **background blur** ("frosted glass", Figma's
/// BACKGROUND_BLUR): the backdrop already composited *behind* this node is
/// sampled, Gaussian-blurred, and drawn back inside the node's silhouette,
/// before the node's own content paints on top. Returns whether anything was
/// frosted (purely informational — this is self-contained and pushes no lasting
/// layer, so the caller does NOT restore anything for it).
///
/// ## Two strategies: backdrop save-layer (default) vs. sample → blur → blit
///
/// The default frost is Skia's textbook `SaveLayerRec::backdrop(blur)`: the
/// backdrop copy and the Gaussian stay inside the ordinary draw stream (on a
/// GPU canvas, entirely GPU-side). The alternative — reading the backdrop
/// pixels back — costs, on a GPU canvas, a full flush + `waitUntilCompleted` +
/// CPU copy + texture re-upload PER frosted node, which was 77% of the frame
/// in a 10%-zoom pan over the Agency template's 28 in-frame frosted panels;
/// it stays available as a diagnostic (`FANTA_BGBLUR_READBACK=1`, see
/// [`background_blur_uses_backdrop_layer`]).
///
/// The two are byte-identical for a frosted node whose ancestors push no
/// layer (the common case, tested at several zooms). They differ only for a
/// frosted node INSIDE an ancestor's layer, and there each is right for a
/// different nesting: the backdrop layer sees the ancestor layer's content
/// drawn so far (correct inside a masked group — a frosted caption bar over a
/// mask-clipped photo frosts the photo — and inside an opaque shadowed card,
/// where it frosts the card body) but not the page below an unfinished
/// translucent group; the readback reads the ROOT device — the page below,
/// but none of the ancestor layer's own content. Masks and shadowed cards
/// are the frequent nesting, so the backdrop layer is the default.
///
/// The backdrop layer was first rejected on the CPU raster path as "fragile
/// and weak" (a sigma-5 blur barely dented a 4px checker, flaky variance);
/// that was the filter being handed the DEVICE sigma under a scaled CTM. With
/// the world-space sigma it now receives, the CPU suite renders the two
/// strategies pixel-identically (see the effects tests).
///
/// ## Mechanics (readback strategy)
///
/// [`Canvas::read_pixels`] copies the **device** pixels (ignoring matrix/clip),
/// i.e. the current top device — which, crucially, must be the backdrop and not
/// this node's own (empty) effects layer, so the caller invokes this BEFORE
/// `begin_effects_layer`. We:
/// 1. map the silhouette's local bounds to device space and pad by the blur's
///    ~3σ reach (so the kernel samples real backdrop, never decal, at the
///    silhouette edge), clamped to the surface;
/// 2. read that device rect into a bitmap (in the canvas's own color/alpha type);
/// 3. clip to the silhouette, reset the CTM to identity (the read pixels are
///    device-space), and draw the bitmap back through a Gaussian-blur image
///    filter at the **on-screen** sigma. The surrounding `save`/`restore`
///    balances the clip + matrix.
///
/// Returns `false` (frosting nothing) when the node has no background blur, when
/// the combined on-screen sigma is sub-pixel, when the node has no resolvable
/// silhouette, when the backdrop is not readable (e.g. a recording canvas), or
/// when the silhouette is fully off-surface — so a node without the effect pays
/// nothing.
pub(crate) fn apply_background_blur(
    canvas: &Canvas,
    node: &CanvasNode,
    scene_id: Option<NodeId>,
    scene: &Scene,
    effective_scale: f32,
) -> bool {
    use skia_safe::{Bitmap, IRect, M44, image_filters};
    // Combine background-blur radii in quadrature, like layer blurs.
    let mut sigma_sq = 0.0f64;
    for blur in &node.blurs {
        if blur.kind != BlurKind::Background {
            continue;
        }
        let s = blur_world_sigma(blur);
        sigma_sq += s * s;
    }
    if sigma_sq <= 0.0 {
        return false;
    }
    // The ON-SCREEN (device-pixel) sigma. The blit below draws the read pixels at
    // an identity CTM, so the filter runs in device space and wants the on-screen
    // sigma directly — not the world sigma `capped_render_sigma` returns (which
    // assumes the canvas re-applies `effective_scale`). Same clamp + sub-pixel
    // floor as every other blur so the cost stays bounded as you zoom.
    let scale = if effective_scale.is_finite() && effective_scale > 0.0 {
        effective_scale
    } else {
        1.0
    };
    let sigma = (sigma_sq.sqrt() as f32 * scale).clamp(0.0, SIGMA_SCREEN_MAX);
    if sigma < SIGMA_SCREEN_MIN {
        return false;
    }
    // Confine the frosted region to the node's shape so only the backdrop behind
    // it is blurred (a background blur with no silhouette is ill-defined — skip).
    let Some(path) = node_silhouette_path(node, scene_id, scene) else {
        return false;
    };

    // Device-space rect to sample: the silhouette bounds projected through the
    // current CTM, padded by the blur's ~3σ reach so the kernel always samples
    // real backdrop (not decal) right up to the silhouette edge, then clamped to
    // the surface. A degenerate (off-surface / empty) rect frosts nothing.
    let (dev_rect, _) = canvas.local_to_device_as_3x3().map_rect(path.bounds());
    let pad = (sigma * SHADOW_BLUR_REACH_SIGMAS as f32).ceil() as i32;
    let info = canvas.image_info();
    let surface_rect = IRect::from_wh(info.width(), info.height());
    let want = IRect::from_ltrb(
        dev_rect.left.floor() as i32 - pad,
        dev_rect.top.floor() as i32 - pad,
        dev_rect.right.ceil() as i32 + pad,
        dev_rect.bottom.ceil() as i32 + pad,
    );
    let Some(src) = IRect::intersect(&want, &surface_rect) else {
        return false;
    };
    if src.width() <= 0 || src.height() <= 0 {
        return false;
    }

    if background_blur_uses_backdrop_layer(canvas) {
        // Default: Skia's native backdrop save-layer. The backdrop copy and
        // the Gaussian both run inside the ordinary draw stream (GPU-side on
        // a GPU canvas). The readback path below instead forces, PER
        // background-blur node, a full GPU flush + `waitUntilCompleted` + CPU
        // copy + texture re-upload (measured: 77% of main-thread samples in a
        // 10%-zoom pan loop on the Agency template — 28 frosted nodes in
        // frame, walk 176 → 29 ms once removed). The layer's filter runs under
        // the CTM, so it takes the WORLD-space sigma. The silhouette clip
        // confines the frost exactly like the readback path's clip does; the
        // layer is bounded to the silhouette so the copy is node-sized, not
        // viewport-sized. Restoring the (empty) layer composites the blurred
        // backdrop copy back in place — nothing else is drawn into it.
        let world_sigma = sigma / scale;
        let Some(blur) = image_filters::blur(
            (world_sigma, world_sigma),
            skia_safe::TileMode::Clamp,
            None,
            None,
        ) else {
            return false;
        };
        canvas.save();
        canvas.clip_path(&path, skia_safe::ClipOp::Intersect, true);
        let rec = skia_safe::canvas::SaveLayerRec::default()
            .bounds(&path.bounds())
            .backdrop(&blur);
        canvas.save_layer(&rec);
        canvas.restore();
        canvas.restore();
        return true;
    }

    // Snapshot that device rect into a bitmap in the canvas's own pixel format
    // (no conversion), then turn it into an image. `read_pixels_to_bitmap`
    // ignores matrix/clip and reads the top device — the backdrop, since the
    // caller runs this before pushing this node's effects layer.
    let read_info = info.with_dimensions((src.width(), src.height()));
    let mut bitmap = Bitmap::new();
    if !bitmap.set_info(&read_info, None) || !bitmap.try_alloc_pixels() {
        return false;
    }
    if !canvas.read_pixels_to_bitmap(&mut bitmap, (src.left, src.top)) {
        return false;
    }
    bitmap.set_immutable(); // share the pixels with the image rather than copy.
    let image = bitmap.as_image();

    // Clamp tile so the blur samples the snapshot's edge pixels (a frosted panel
    // flush against the surface edge stays frosted rather than fading out).
    let Some(blur) = image_filters::blur((sigma, sigma), skia_safe::TileMode::Clamp, None, None)
    else {
        return false;
    };
    let mut paint = Paint::default();
    paint.set_image_filter(blur);

    canvas.save();
    // Confine the frosted backdrop to the silhouette (clip is set in the current
    // CTM, i.e. local space), then drop to identity so the device-space snapshot
    // blits 1:1. The clip persists across the matrix reset (Skia stores it in
    // device space); save/restore balances both.
    canvas.clip_path(&path, skia_safe::ClipOp::Intersect, true);
    canvas.set_matrix(&M44::new_identity());
    canvas.draw_image(&image, (src.left as f32, src.top as f32), Some(&paint));
    canvas.restore();
    true
}

/// Whether [`apply_background_blur`] frosts through Skia's backdrop
/// save-layer (`true`, the default) or the explicit read-pixels → blur → blit
/// path (`false`).
///
/// Environment override for diagnosis: `FANTA_BGBLUR_READBACK=1` forces the
/// readback (the pre-wave-3 behavior) so the two can be A/B'd from the shell.
/// Tests pin a strategy per thread via [`with_background_blur_backdrop`].
/// `canvas` is unused today — the strategy does not depend on the canvas kind
/// — but is threaded so a per-backend choice needs no call-site change.
pub(crate) fn background_blur_uses_backdrop_layer(canvas: &Canvas) -> bool {
    let _ = canvas;
    #[cfg(test)]
    if let Some(forced) = BACKDROP_OVERRIDE.with(|o| o.get()) {
        return forced;
    }
    static ENV: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENV.get_or_init(|| std::env::var_os("FANTA_BGBLUR_READBACK").is_none())
}

#[cfg(test)]
thread_local! {
    static BACKDROP_OVERRIDE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Run `f` with the background-blur strategy pinned on this thread: `true` =
/// backdrop save-layer, `false` = readback (regardless of canvas kind). Lets
/// the CPU test suite exercise the GPU-default path.
#[cfg(test)]
pub(crate) fn with_background_blur_backdrop<T>(backdrop: bool, f: impl FnOnce() -> T) -> T {
    let previous = BACKDROP_OVERRIDE.with(|o| o.replace(Some(backdrop)));
    let out = f();
    BACKDROP_OVERRIDE.with(|o| o.set(previous));
    out
}

/// Maximum on-SCREEN (device-pixel) Gaussian sigma we will ever ask Skia to
/// blur a drop shadow by. With the `blur/2` sigma convention this corresponds to
/// a ~256 px on-screen blur *radius* — squarely in the "larger is visually
/// indistinguishable" range: a Gaussian's visible support is ≈3σ, so 128 px of
/// sigma already spreads a shadow over ~384 device px in each direction, well
/// past any plausible card/button silhouette, and its tails are below 8-bit
/// alpha quantization.
///
/// Capping here bounds the worst case: the CPU Gaussian's cost scales with the
/// kernel area (∝ sigma²), and because the canvas is scaled by
/// `effective_scale`, the *on-screen* sigma is `world_sigma · effective_scale`,
/// which explodes as you zoom in. Clamping the on-screen sigma to this constant
/// makes a high-zoom frame's shadow cost flat instead of quadratic.
///
/// The cap only bites when the on-screen blur is already enormous (i.e. you have
/// zoomed in far past where the extra blur is perceptible), so normal-zoom
/// shadows are byte-for-byte unchanged.
pub(crate) const SIGMA_SCREEN_MAX: f32 = 128.0;

/// Below this on-screen sigma a Gaussian blur is a sub-pixel no-op — the shadow
/// is effectively a hard offset copy and the blur term contributes nothing
/// visible. We treat anything under it as "no blur" (sigma 0), which lets Skia
/// take its crisp-shadow fast path instead of building a degenerate kernel.
pub(crate) const SIGMA_SCREEN_MIN: f32 = 0.25;

/// A shadow whose ENTIRE painted deviation from the bare silhouette — offset,
/// spread, and the blur's ~3σ reach combined — lands under this many device
/// pixels is imperceptible: it hides behind the silhouette's own anti-aliased
/// edge. Zoomed out far enough, that is true of every ordinary card/button
/// shadow, and skipping them removes their filter + layer entirely (the
/// zoom-out LOD that keeps a node-heavy page scrollable).
pub(crate) const SHADOW_SUBPIXEL_MAX_PX: f64 = 0.75;

/// Whether `shadow`'s painted deviation from the bare silhouette is sub-pixel
/// at `effective_scale` (see [`SHADOW_SUBPIXEL_MAX_PX`]). Uses the blur's 3σ
/// visible support and |spread| (negative spread shrinks the silhouette — also
/// invisible once sub-pixel). A degenerate scale never skips (safe default).
pub(crate) fn shadow_is_subpixel(shadow: &Shadow, effective_scale: f32) -> bool {
    let scale = f64::from(effective_scale);
    if !scale.is_finite() || scale <= 0.0 {
        return false;
    }
    let offset = shadow.offset[0].abs().max(shadow.offset[1].abs());
    let reach = offset
        + shadow_world_spread(shadow).abs()
        + shadow_world_sigma(shadow) * SHADOW_BLUR_REACH_SIGMAS;
    reach * scale < SHADOW_SUBPIXEL_MAX_PX
}

/// The world-space Gaussian sigma a [`Shadow`] maps to (before the on-screen
/// cap). Factored out so the shadow-expanded cull bounds and the filter builder
/// agree on the blur extent.
///
/// Per the implementation note, the **blur sigma** is `blur / 2`: Figma/CSS
/// express blur as a diameter-ish radius, while Skia's Gaussian takes a standard
/// deviation, and `radius/2` is the conventional, visually-faithful mapping.
///
/// `spread` is NOT folded in here — it is a *geometric* term (it grows or
/// shrinks the shadow's silhouette before the blur, the way Figma/OpenPencil
/// inflate/deflate the shadow rect by `±spread`), not a blur term. The filter
/// builders ([`build_drop_shadow_filter`] / [`draw_inner_shadows`]) apply it as
/// a morphology (dilate/erode) on the silhouette via [`shadow_world_spread`];
/// the cull-bounds math grows its box by it directly. Folding spread into the
/// blur (the old behavior) made the shadow *blurrier* instead of *bigger* and
/// silently dropped negative spread.
pub(crate) fn shadow_world_sigma(shadow: &Shadow) -> f64 {
    (shadow.blur * 0.5).max(0.0)
}

/// The world-space silhouette **spread** a [`Shadow`] maps to: a positive value
/// expands the shadow's shape, a negative one contracts it (Figma's `spread`).
///
/// For a DROP shadow this is applied by dilating (spread &gt; 0) or eroding
/// (spread &lt; 0) the node's alpha silhouette before the offset+blur, so the
/// soft shadow grows/shrinks outward — matching OpenPencil's
/// `rect(x-spread, y-spread, x+w+spread, y+h+spread)` inflate. For an INNER
/// shadow the morphology is *inverted* (positive spread chokes the shape
/// inward, thickening the inner ring), see [`draw_inner_shadows`].
///
/// Factored out so the filter builders and the shadow-expanded cull bounds agree
/// on the geometric extent of `spread` exactly like they agree on the blur via
/// [`shadow_world_sigma`].
pub(crate) fn shadow_world_spread(shadow: &Shadow) -> f64 {
    shadow.spread
}

/// The world-space Gaussian sigma a [`Blur`] effect maps to (before the
/// on-screen cap). Uses the SAME `radius / 2` convention as a shadow's blur:
/// Figma/CSS express the blur as a diameter-ish radius while Skia's Gaussian
/// takes a standard deviation. Factored out so the layer-blur filter and the
/// background-blur backdrop filter agree on the mapping, and both run it through
/// [`capped_render_sigma`] so the on-screen sigma stays bounded as you zoom.
pub(crate) fn blur_world_sigma(blur: &Blur) -> f64 {
    blur.radius.max(0.0) * 0.5
}

/// Build a single Skia Gaussian-blur `ImageFilter` covering every LAYER blur in
/// `blurs`, chained on top of `input` (typically the drop-shadow filter so the
/// node's shadow is blurred along with its content), or return `input`
/// unchanged when there is no visible layer blur.
///
/// Multiple layer blurs are uncommon, but if present their sigmas add (a stack
/// of independent Gaussians convolves to a wider Gaussian, `σ² = Σσᵢ²`), so we
/// combine them into one filter rather than nesting N blurs — same visual,
/// cheaper. A blur whose capped on-screen sigma is sub-pixel contributes nothing
/// and is skipped (so a 0-radius blur is a no-op and the fast path is kept).
///
/// `effective_scale` is `viewport.zoom · display_scale`; the sigma is capped via
/// [`capped_render_sigma`] exactly like a shadow's, so a high-zoom layer blur
/// can't blow up the CPU Gaussian's cost.
pub(crate) fn build_layer_blur_filter(
    blurs: &[Blur],
    effective_scale: f32,
    input: Option<skia_safe::ImageFilter>,
) -> Option<skia_safe::ImageFilter> {
    use skia_safe::image_filters;
    // Combine layer-blur radii in quadrature (independent Gaussians). The
    // sub-pixel skip below is the same test `layer_blur_paints` applies for
    // the walk's layer/fold decision, so the two never disagree.
    let world_sigma = layer_blur_world_sigma(blurs);
    if world_sigma <= 0.0 {
        return input;
    }
    let sigma = capped_render_sigma(world_sigma, effective_scale);
    if sigma <= 0.0 {
        return input;
    }
    // Decal tile mode (Skia's default) so the layer fades at its edge rather
    // than smearing the boundary texel outward.
    image_filters::blur((sigma, sigma), None, input, None)
}

/// A Gaussian's visible support is ≈3σ (≈99.7% of the contribution); beyond it
/// the tail is below 8-bit alpha quantization. We use 3σ as the blur reach when
/// expanding a node's cull bounds so the box covers everywhere the shadow could
/// paint a visible pixel.
pub(crate) const SHADOW_BLUR_REACH_SIGMAS: f64 = 3.0;

/// Expand a node's `world` AABB by the world-space reach of its own drop
/// shadow(s), so the viewport cull keeps a node whose body is off-screen but
/// whose offset+blurred shadow reaches the visible region (and culls a node
/// whose body *and* shadow both miss it).
///
/// A drop shadow's silhouette in the node's LOCAL space is the node's local
/// bounds translated by `offset`, inflated by `spread` on every side (see
/// [`shadow_world_spread`]), then grown by `~3·sigma` for the blur. We compute
/// that expanded LOCAL box (the union over all drop shadows, plus the
/// un-shadowed body) and transform it to world space through the node's world
/// transform — transforming the expanded box (rather than padding the world
/// AABB) keeps the expansion correct under a rotated/scaled node.
///
/// The blur reach matches what actually paints: the **capped** on-screen sigma
/// (see [`SIGMA_SCREEN_MAX`]) converted back to world units, i.e.
/// `min(world_sigma, SIGMA_SCREEN_MAX / effective_scale)`. Using the capped
/// reach makes the cull box exactly cover the painted shadow — never
/// under-estimating it (so a visible shadow is never wrongly culled) and not
/// over-estimating it at high zoom (so a distant node whose capped shadow no
/// longer reaches the screen is correctly dropped). Inner shadows and zero-alpha
/// drop shadows add nothing.
///
/// `effective_scale` is the frame's `viewport.zoom · display_scale`. The node's
/// own world-space scale further multiplies the on-screen sigma, but folding it
/// in would require decomposing the world transform; omitting it makes the reach
/// slightly *more* generous (the cap converts back through the smaller frame
/// scale), which stays on the safe side of never culling a visible shadow.
#[cfg(test)]
pub(crate) fn shadow_expanded_world_bounds(
    scene: &Scene,
    id: NodeId,
    effects: &[Shadow],
    blurs: &[Blur],
    world: Bounds,
    effective_scale: f32,
) -> Bounds {
    if !shadow_expansion_needed(effects, blurs) {
        return world;
    }
    // We need the node's LOCAL bounds + world transform to expand in local
    // space and project. If either is unavailable, fall back to the body world
    // bounds (no expansion) — safe, just less precise.
    let (Some(local), Some(world_t)) = (scene.local_bounds(id), scene.world_transform(id)) else {
        return world;
    };
    // Project the expanded local box to world space (corner-wise AABB).
    shadow_expanded_local_bounds(local, effects, blurs, effective_scale).transformed(&world_t)
}

/// Fast pre-check for the shadow expansions: no visible drop shadow AND no
/// LAYER blur ⇒ the body bounds are the whole story. (A BACKGROUND blur frosts
/// the backdrop *inside* the node's silhouette and never grows its painted
/// extent, so it doesn't widen any box.)
fn shadow_expansion_needed(effects: &[Shadow], blurs: &[Blur]) -> bool {
    effects
        .iter()
        .any(|s| s.kind == ShadowKind::Drop && s.color.a != 0)
        || blurs
            .iter()
            .any(|b| b.kind == BlurKind::Layer && blur_world_sigma(b) > 0.0)
}

/// Expand a node's LOCAL bounds by the world-space reach of its own drop
/// shadow(s) and layer blur — the local-space core of
/// [`shadow_expanded_world_bounds`], shared with the mask-layer bounds (which
/// need the same expansion in the parent's local space rather than world
/// space). Returns `local` unchanged when nothing expands.
pub(crate) fn shadow_expanded_local_bounds(
    local: Bounds,
    effects: &[Shadow],
    blurs: &[Blur],
    effective_scale: f32,
) -> Bounds {
    if !shadow_expansion_needed(effects, blurs) {
        return local;
    }
    let layer_blur_sigma = layer_blur_world_sigma(blurs);

    // Convert the on-screen sigma cap back to world units (the ceiling on a
    // shadow's/blur's painted sigma in this frame). A degenerate scale disables
    // the ceiling (infinite), so the reach falls back to the uncapped sigma.
    let scale = effective_scale as f64;
    let world_sigma_cap = if scale.is_finite() && scale > 0.0 {
        SIGMA_SCREEN_MAX as f64 / scale
    } else {
        f64::INFINITY
    };

    // Accumulate the expanded local box: start with the un-shadowed body so a
    // node always at least covers itself, then union each drop shadow's
    // offset+blurred local silhouette.
    let mut acc = local;
    for shadow in effects {
        if shadow.kind != ShadowKind::Drop || shadow.color.a == 0 {
            continue;
        }
        let sigma = shadow_world_sigma(shadow).min(world_sigma_cap);
        // The blur's ~3·sigma reach PLUS the positive spread (which dilates the
        // silhouette outward); a negative spread shrinks the shape but can only
        // make the painted shadow *smaller*, so it never needs a wider cull box —
        // clamp it to 0 here to stay on the safe side of never culling a visible
        // shadow.
        let reach = sigma * SHADOW_BLUR_REACH_SIGMAS + shadow_world_spread(shadow).max(0.0);
        let shadow_box = Bounds {
            min_x: local.min_x + shadow.offset[0] - reach,
            min_y: local.min_y + shadow.offset[1] - reach,
            max_x: local.max_x + shadow.offset[0] + reach,
            max_y: local.max_y + shadow.offset[1] + reach,
        };
        acc = acc.union(&shadow_box);
    }
    // A LAYER blur grows the node's painted silhouette uniformly by ~3·sigma on
    // every side (the Gaussian's visible support), so expand the body box by the
    // combined (capped) layer-blur reach.
    if layer_blur_sigma > 0.0 {
        let sigma = layer_blur_sigma.min(world_sigma_cap);
        let reach = sigma * SHADOW_BLUR_REACH_SIGMAS;
        let blur_box = Bounds {
            min_x: local.min_x - reach,
            min_y: local.min_y - reach,
            max_x: local.max_x + reach,
            max_y: local.max_y + reach,
        };
        acc = acc.union(&blur_box);
    }
    acc
}

/// The WORLD-space Gaussian sigma Skia's drop-shadow filter should be given for
/// a shadow whose faithful world sigma is `world_sigma`, under canvas scale
/// `effective_scale` (`viewport.zoom · display_scale`).
///
/// The canvas re-applies `effective_scale`, so the ON-SCREEN sigma Skia ends up
/// blurring by is `returned_sigma · effective_scale`. We therefore:
/// 1. compute the on-screen sigma `world_sigma · scale`;
/// 2. clamp it to [`SIGMA_SCREEN_MAX`] (the cost-bounding cap — only bites when
///    the blur is already perceptually saturated, so normal zoom is unchanged);
/// 3. treat anything below [`SIGMA_SCREEN_MIN`] as a crisp shadow (sigma 0);
/// 4. convert the result back to world units (`/ scale`).
///
/// A non-positive / non-finite scale (degenerate zoom) falls back to 1.0 so the
/// cap still bounds the kernel. Pure + side-effect-free so the cap is unit
/// testable without building a Skia filter.
pub(crate) fn capped_render_sigma(world_sigma: f64, effective_scale: f32) -> f32 {
    let scale = if effective_scale.is_finite() && effective_scale > 0.0 {
        effective_scale
    } else {
        1.0
    };
    let screen_sigma = (world_sigma as f32 * scale).clamp(0.0, SIGMA_SCREEN_MAX);
    if screen_sigma < SIGMA_SCREEN_MIN {
        0.0
    } else {
        screen_sigma / scale
    }
}

/// Build a single Skia `ImageFilter` that paints all of `effects`' **drop**
/// shadows behind the layer's content, or `None` if there are no drop shadows
/// (or every one is negligible at this zoom — see below).
///
/// Each drop shadow is built as a **shadow-only** layer derived from the node's
/// own silhouette (the layer's source bitmap), then all of them are `merge`d
/// *under* the source so the node always draws over its own shadows — a
/// node-level effect: the silhouette + its shadows blend against the backdrop as
/// a unit. The shadows are merged so the FIRST shadow in `effects` sits on top
/// of later ones (Figma's panel order: the topmost effect composites last),
/// which is why the list is walked in reverse into the merge.
///
/// **`spread`** is honored geometrically, the way Figma/OpenPencil do it: a
/// positive spread DILATES the silhouette and a negative spread ERODES it
/// *before* the offset+blur (via [`shadow_world_spread`] morphology), so the
/// soft shadow grows/shrinks outward rather than just getting blurrier. The old
/// path folded spread into the blur sigma, which made a spread shadow fuzzier
/// instead of bigger and silently dropped negative spread.
///
/// `effective_scale` is `viewport.zoom · display_scale` — the canvas scale the
/// caller has applied. The blur is specified in WORLD units but Skia applies the
/// Gaussian in the scaled device space, so the *on-screen* sigma is
/// `world_sigma · effective_scale`. We clamp the world sigma so that on-screen
/// sigma never exceeds [`SIGMA_SCREEN_MAX`]: this leaves normal-zoom shadows
/// identical (the cap only engages once the on-screen blur is already enormous)
/// while bounding the CPU Gaussian's quadratic blow-up as you zoom in. A shadow
/// whose on-screen sigma is below [`SIGMA_SCREEN_MIN`] is rendered crisp (sigma
/// 0); a shadow with zero color alpha contributes nothing and is skipped. The
/// spread morphology is given in WORLD units too and likewise rides the canvas
/// CTM into device space.
///
/// A zero-everything shadow still draws a crisp offset copy, which is correct.
///
/// Inner shadows are skipped (see [`begin_effects_layer`]).
pub(crate) fn build_drop_shadow_filter(
    effects: &[Shadow],
    effective_scale: f32,
) -> Option<skia_safe::ImageFilter> {
    use skia_safe::image_filters;

    // One shadow-only filter per visible drop shadow, in `effects` order.
    let mut shadow_layers: Vec<skia_safe::ImageFilter> = Vec::new();
    for shadow in effects {
        // Inner shadows are painted separately (see `draw_inner_shadows`); a
        // fully-transparent shadow paints nothing (don't pay for its
        // layer-sized blur); and the zoom-out LOD skips a shadow whose
        // offset+spread+blur all collapse under a device pixel (it hides
        // behind the silhouette's AA edge). One predicate — shared with the
        // walk's `visible_effects` — decides all three.
        if !drop_shadow_paints(shadow, effective_scale) {
            continue;
        }
        // The WORLD-space sigma Skia's filter is given, after the on-screen cap
        // and the negligible-blur floor (see `capped_render_sigma`). Below the
        // cap this equals the faithful blur/2 sigma, so normal-zoom output is
        // byte-for-byte unchanged.
        let sigma = capped_render_sigma(shadow_world_sigma(shadow), effective_scale);
        // Spread dilates/erodes the silhouette before the offset+blur. The
        // morphology takes the source bitmap (`None` input) and feeds the
        // grown/shrunk silhouette into the shadow filter. A zero spread skips the
        // morphology so the un-spread fast path is untouched.
        let spread_input = spread_morphology_filter(shadow_world_spread(shadow), false);
        // `drop_shadow_only` so we composite the shadows + content ourselves
        // (via the merge below) rather than re-stamping the content per shadow.
        if let Some(f) = image_filters::drop_shadow_only(
            (shadow.offset[0] as f32, shadow.offset[1] as f32),
            (sigma, sigma),
            crate::color::to_sk_color4f(shadow.color),
            None,         // default color space
            spread_input, // the spread-morphed silhouette (or the source bitmap)
            None,         // no crop rect — shadow may extend past the node bounds
        ) {
            // Figma's `showShadowBehindNode = false` (its default): the shadow
            // is knocked out wherever the node's own coverage sits, so nothing
            // shows through a translucent or hollow body. `DstOut` with the
            // source silhouette (`None` foreground) multiplies the shadow by
            // `1 − source alpha` — for a fully opaque body this is invisible
            // (the body repaints over the shadow anyway), for a translucent
            // one it is exactly the Figma knockout.
            let layer = if shadow.show_behind_node {
                f
            } else {
                match image_filters::blend(skia_safe::BlendMode::DstOut, f.clone(), None, None) {
                    Some(knocked_out) => knocked_out,
                    None => f,
                }
            };
            shadow_layers.push(layer);
        }
    }
    if shadow_layers.is_empty() {
        return None;
    }
    // Composite: all shadows BEHIND the source, with the FIRST listed shadow on
    // top of later ones. `merge` draws its inputs front-of-list = bottom, so we
    // push the shadows back-to-front, then the source (`None`) last = on top.
    let mut inputs: Vec<Option<skia_safe::ImageFilter>> =
        Vec::with_capacity(shadow_layers.len() + 1);
    for layer in shadow_layers.into_iter().rev() {
        inputs.push(Some(layer));
    }
    inputs.push(None); // the node's own content, drawn over its shadows.
    image_filters::merge(inputs, None)
}

/// A Skia morphology `ImageFilter` that grows (dilate) or shrinks (erode) the
/// silhouette of its input (the source bitmap, since `input` is `None`) by
/// `spread` WORLD units, or `None` for a zero/negligible spread (so the
/// no-spread fast path attaches no extra filter).
///
/// `invert` flips the direction, for the INNER-shadow case where a positive
/// spread chokes the shape *inward* (thickening the inner ring) — i.e. positive
/// inner spread erodes the offset copy rather than dilating it. For a DROP shadow
/// `invert` is `false`: positive spread dilates, negative erodes — matching
/// Figma's outward inflate.
fn spread_morphology_filter(spread: f64, invert: bool) -> Option<skia_safe::ImageFilter> {
    use skia_safe::image_filters;
    // Below ~half a world pixel the morphology is a sub-pixel no-op.
    if spread.abs() < 0.5 {
        return None;
    }
    let radius = spread.abs() as f32;
    let dilate = (spread > 0.0) != invert;
    if dilate {
        image_filters::dilate((radius, radius), None, None)
    } else {
        image_filters::erode((radius, radius), None, None)
    }
}

/// The node-LOCAL silhouette path an inner shadow is clipped to (and computed
/// from), or `None` for a node with no fillable interior at this level.
///
/// Mirrors the shape each node kind actually paints in [`paint_node_content`] so
/// the inner shadow rings the *same* boundary the fill draws:
/// - **Vector**: the rounded-rect box for a rect-shaped path (honoring
///   `corner_radius`/`corner_radii`, exactly like `draw_vector`), else the raw
///   vector path.
/// - **Group / frame**: the rounded box of the frame's `clip_size`, else the
///   scene-computed content bounds for a background/border-only group.
/// - **Text / text path**: the shaped glyph outlines, preserving counters while
///   leaving whitespace transparent, so inner shadows follow glyph alpha.
/// - **Boolean**: the folded operand path, matching the geometry actually drawn.
/// - **Instance / Bitmap / Video / media**: the node's `local_size` box — the
///   same box `effects_layer_bounds` uses. A degenerate box yields `None`.
pub(crate) fn node_silhouette_path(
    node: &CanvasNode,
    scene_id: Option<NodeId>,
    scene: &Scene,
) -> Option<skia_safe::Path> {
    match &node.data {
        NodeData::Vector(v) => {
            let bounds = v.path.rough_bounds().unwrap_or(Bounds::ZERO);
            let local = bounds_to_f32(&bounds);
            Some(if path_is_rect(&v.path) {
                rounded_rect_path(local, v.corner_radius, v.corner_radii, v.corner_smoothing)
            } else {
                to_sk_fill_path(&v.path)
            })
        }
        NodeData::Text(text) => text_node_outline(text).map(|path| to_sk_fill_path(&path)),
        NodeData::TextPath(text_path) => text_path_outline(text_path).into_exact(),
        NodeData::Boolean(boolean) => fold_operands(scene, scene_id?, boolean.op),
        NodeData::Group(g) => frame_box_bounds(g, scene_id, scene).map(|b| {
            rounded_rect_path(
                bounds_to_f32(&b),
                g.corner_radius,
                g.corner_radii,
                g.corner_smoothing,
            )
        }),
        data => {
            let [w, h] = data.local_size()?;
            if w <= 0.0 || h <= 0.0 {
                return None;
            }
            let mut path = skia_safe::Path::new();
            path.add_rect(
                skia_safe::Rect::from_xywh(0.0, 0.0, w as f32, h as f32),
                None,
            );
            Some(path)
        }
    }
}

/// Paint every [`ShadowKind::Inner`] effect of `node`, clipped to its silhouette
/// so a shadow only ever darkens the *inside* of the shape and never bleeds past
/// its edges (the drop shadow's job, and the invariant the
/// `inner_shadow_does_not_bleed_outside_the_node_rect` test guards).
///
/// Skia has no single inner-shadow filter, so we build the same DAG Figma's
/// canvas uses, drawn into a shape-clipped save-layer:
///
/// 1. **Clip** the canvas to the silhouette path (anti-aliased Intersect). This
///    alone guarantees no outside bleed.
/// 2. Draw the silhouette filled with the shadow color, under an `ImageFilter`
///    that subtracts an offset+blurred copy of itself:
///    `blend(SrcOut, background = blur(offset(source)), foreground = source)`.
///    `SrcOut` keeps the colored silhouette only where the offset/blurred copy
///    has *receded* — i.e. the soft band hugging the inner edges away from the
///    offset direction. The blur uses Skia's default **decal** tile mode, so the
///    offset copy fades to transparent at the shape edge and the ring softens
///    inward — the "difference + decal blur" recipe.
///
/// The blur sigma reuses [`capped_render_sigma`] / [`shadow_world_sigma`] so an
/// inner shadow's on-screen blur is bounded and floored identically to a drop
/// shadow's (faithful at normal zoom, cost-capped when zoomed far in). A
/// zero-alpha or geometry-less shadow is skipped.
///
/// **`spread`** is honored geometrically and *inverted* relative to a drop
/// shadow: a positive spread chokes the shape inward (a thicker inner ring), a
/// negative one loosens it. It is applied as a morphology on the offset copy
/// (the `background`) via [`spread_morphology_filter`] with `invert = true`, so
/// positive spread erodes that copy — vacating more interior for the `SrcOut`
/// color to fill — exactly mirroring how a drop shadow's positive spread dilates
/// outward. The old path folded spread into the blur sigma (fuzzier, not
/// thicker) and dropped negative spread.
pub(crate) fn draw_inner_shadows(
    canvas: &Canvas,
    node: &CanvasNode,
    scene_id: Option<NodeId>,
    ctx: &mut RenderCtx,
) {
    use skia_safe::image_filters;

    // Cheap pre-check: nothing to do unless at least one inner shadow paints
    // at this scale (non-zero alpha, not sub-pixel — the same predicate the
    // walk's `visible_effects` uses, so the fold/layer decision agrees with
    // what is drawn here). Also skips the silhouette build for a node whose
    // inner shadows are all zoom-out-skipped.
    if !node
        .effects
        .iter()
        .any(|s| inner_shadow_paints(s, ctx.effective_scale))
    {
        return;
    }
    let Some(path) = node_silhouette_path(node, scene_id, ctx.scene) else {
        return;
    };

    for shadow in &node.effects {
        // Zoom-out LOD, mirroring the drop-shadow skip: a sub-pixel inner ring
        // is invisible, so don't build its filter DAG.
        if !inner_shadow_paints(shadow, ctx.effective_scale) {
            continue;
        }
        let sigma = capped_render_sigma(shadow_world_sigma(shadow), ctx.effective_scale);
        // background = the silhouette (source), optionally spread-morphed, then
        // offset and blurred. Inner spread is INVERTED (positive ⇒ erode the
        // copy so more interior is vacated for the SrcOut color to fill). `offset`
        // with a `None`/morphed input shifts the silhouette; `blur` then softens
        // it with the default decal tile so it fades at the shape edge.
        let spread_input = spread_morphology_filter(shadow_world_spread(shadow), true);
        let offset = image_filters::offset(
            (shadow.offset[0] as f32, shadow.offset[1] as f32),
            spread_input,
            None,
        );
        let background = image_filters::blur((sigma, sigma), None, offset, None);
        // foreground = the source silhouette itself (drawn in the shadow color
        // below), so `SrcOut` paints that color in the ring the background has
        // vacated. `None` foreground == source bitmap.
        let Some(filter) =
            image_filters::blend(skia_safe::BlendMode::SrcOut, background, None, None)
        else {
            continue;
        };

        canvas.save();
        // (1) Confine everything to the shape — no outside bleed, ever.
        canvas.clip_path(&path, skia_safe::ClipOp::Intersect, true);
        // (2) Draw the silhouette in the shadow color through the SrcOut DAG.
        let mut paint = Paint::default();
        paint.set_anti_alias(true);
        paint.set_color(to_sk_color(shadow.color));
        paint.set_image_filter(filter);
        canvas.draw_path(&path, &paint);
        canvas.restore();
        ctx.metrics.nodes_drawn += 1;
    }
}
