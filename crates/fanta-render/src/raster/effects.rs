//! Node-level compositing effects: drop/inner shadows, layer + background
//! blur, blend-mode mapping, and the on-screen sigma cap + shadow-expanded
//! cull-bounds math that bounds their cost. Shared by both the live-scene
//! walk and the transient instance walk.
use super::{
    BlendMode, Blur, BlurKind, Bounds, Canvas, CanvasNode, NodeData, NodeId, Paint, RenderCtx,
    Scene, Shadow, ShadowKind, bounds_to_f32, path_is_rect, rounded_rect_path, to_sk_color,
    to_sk_path,
};

/// Push a save-layer carrying this node's compositing effects — opacity, drop
/// shadow(s), and blend mode — returning whether one was pushed (so the caller
/// balances it with a `restore`).
///
/// A layer is needed when ANY of these hold:
/// - `opacity < 1.0` (the legacy reason);
/// - the node has ≥1 **drop** shadow (an `ImageFilter` paints the shadow behind
///   the node silhouette);
/// - the node's `blend_mode` is non-`Normal` (the layer composites against the
///   backdrop with that mode).
///
/// When none hold (the overwhelmingly common case: full opacity, no shadows,
/// `Normal` blend) this allocates nothing and returns `false`, so the existing
/// fast path is untouched — no regression, no extra layer.
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
pub(crate) fn begin_effects_layer(
    canvas: &Canvas,
    opacity: f32,
    blend_mode: BlendMode,
    effects: &[Shadow],
    blurs: &[Blur],
    effective_scale: f32,
    content_bounds: Option<Bounds>,
) -> bool {
    // Drop shadow(s) first; then a LAYER blur chained on top so the node's
    // content AND its shadow are Gaussian-blurred together (Figma's layer-blur
    // semantics — the whole composited layer is blurred). Background blurs are
    // NOT a layer-paint effect (they sample the backdrop, not the node) and are
    // handled separately by [`apply_background_blur`].
    let shadow_filter = build_drop_shadow_filter(effects, effective_scale);
    let image_filter = build_layer_blur_filter(blurs, effective_scale, shadow_filter);
    let non_normal_blend = blend_mode != BlendMode::Normal;
    // Nothing to do: full opacity, no drop shadow, no layer blur, normal blend.
    if opacity >= 1.0 && image_filter.is_none() && !non_normal_blend {
        return false;
    }
    let mut paint = Paint::default();
    if opacity < 1.0 {
        paint.set_alpha_f(opacity);
    }
    if let Some(filter) = image_filter {
        paint.set_image_filter(filter);
    }
    if non_normal_blend {
        paint.set_blend_mode(to_sk_blend_mode(blend_mode));
    }
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
    let rec = skia_safe::canvas::SaveLayerRec::default().paint(&paint);
    if let Some(b) = content_bounds {
        // Pad in DEVICE pixels so the safety margin is zoom-independent:
        // covers outside-aligned strokes, anti-aliasing, and small geometry
        // overshoot (glyph overhang, a child stroke at the box edge) at any
        // zoom without re-deriving per-node stroke reach.
        let scale = f64::from(effective_scale);
        let pad = if scale.is_finite() && scale > 0.0 {
            LAYER_BOUNDS_PAD_DEVICE_PX / scale
        } else {
            LAYER_BOUNDS_PAD_DEVICE_PX
        };
        let padded = skia_safe::Rect::new(
            (b.min_x - pad) as f32,
            (b.min_y - pad) as f32,
            (b.max_x + pad) as f32,
            (b.max_y + pad) as f32,
        );
        canvas.save_layer(&rec.bounds(&padded));
    } else {
        canvas.save_layer(&rec);
    }
    true
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
/// - the node's own intrinsic geometry (vector path bounds, text/bitmap/etc.
///   `local_size`) — also valid for TRANSIENT instance-expansion clones that
///   have no scene entry;
/// - the scene's memoized subtree `local_bounds` for a live unclipped group.
pub(crate) fn effects_layer_bounds(
    node: &CanvasNode,
    scene_id: Option<NodeId>,
    scene: &Scene,
) -> Option<Bounds> {
    match &node.data {
        NodeData::Group(g) => match g.clip_size {
            Some([w, h]) => Some(Bounds::from_xywh(0.0, 0.0, w, h)),
            None => scene_id.and_then(|id| scene.local_bounds(id)),
        },
        NodeData::Vector(v) => v.path.rough_bounds(),
        NodeData::Text(t) => Some(Bounds::from_xywh(
            0.0,
            0.0,
            t.local_size[0],
            t.local_size[1],
        )),
        NodeData::Bitmap(b) => Some(Bounds::from_xywh(
            0.0,
            0.0,
            b.local_size[0],
            b.local_size[1],
        )),
        NodeData::Instance(i) => Some(Bounds::from_xywh(
            0.0,
            0.0,
            i.local_size[0],
            i.local_size[1],
        )),
        // Remaining kinds (video/audio/embed/...) are rare effect carriers;
        // the live scene resolves them via local_bounds, transient ones stay
        // unbounded.
        _ => scene_id.and_then(|id| scene.local_bounds(id)),
    }
}

/// Paint the node's **background blur** ("frosted glass", Figma's
/// BACKGROUND_BLUR): the backdrop already composited *behind* this node is
/// sampled, Gaussian-blurred, and drawn back inside the node's silhouette,
/// before the node's own content paints on top. Returns whether anything was
/// frosted (purely informational — this is self-contained and pushes no lasting
/// layer, so the caller does NOT restore anything for it).
///
/// ## Why an explicit sample → blur → blit, not `SaveLayerRec::backdrop`
///
/// Skia's `SaveLayerRec::backdrop(blur)` is the textbook recipe, but on the CPU
/// raster path it is both **fragile and weak** here: a backdrop filter only
/// samples the real device backdrop when the layer is top-level, and even
/// unwrapped it applied a far-smaller-than-requested effective sigma under the
/// frame's viewport CTM + silhouette clip (a sigma-5 blur barely dented a 4px
/// checker) while reading partly-uninitialized layer margin (run-to-run flaky
/// variance). So instead we do the offscreen-sample-and-blit the macOS-GPU notes
/// prescribe for advanced blends: read the backdrop region into an offscreen
/// bitmap, blur it, and composite it back — deterministic, full-strength, and
/// independent of whatever layer nesting the caller is in.
///
/// ## Mechanics
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
    // Combine layer-blur radii in quadrature (independent Gaussians).
    let mut sigma_sq = 0.0f64;
    for blur in blurs {
        if blur.kind != BlurKind::Layer {
            continue;
        }
        let s = blur_world_sigma(blur);
        sigma_sq += s * s;
    }
    if sigma_sq <= 0.0 {
        return input;
    }
    let world_sigma = sigma_sq.sqrt();
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
pub(crate) fn shadow_expanded_world_bounds(
    scene: &Scene,
    id: NodeId,
    effects: &[Shadow],
    blurs: &[Blur],
    world: Bounds,
    effective_scale: f32,
) -> Bounds {
    // Fast path: no drop shadow AND no LAYER blur ⇒ the body bounds are the
    // whole story. (A BACKGROUND blur frosts the backdrop *inside* the node's
    // silhouette and never grows its painted extent, so it doesn't widen the
    // cull box.)
    let has_drop = effects
        .iter()
        .any(|s| s.kind == ShadowKind::Drop && s.color.a != 0);
    let layer_blur_sigma_sq: f64 = blurs
        .iter()
        .filter(|b| b.kind == BlurKind::Layer)
        .map(|b| {
            let s = blur_world_sigma(b);
            s * s
        })
        .sum();
    if !has_drop && layer_blur_sigma_sq <= 0.0 {
        return world;
    }
    // We need the node's LOCAL bounds + world transform to expand in local
    // space and project. If either is unavailable, fall back to the body world
    // bounds (no expansion) — safe, just less precise.
    let (Some(local), Some(world_t)) = (scene.local_bounds(id), scene.world_transform(id)) else {
        return world;
    };

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
    if layer_blur_sigma_sq > 0.0 {
        let sigma = layer_blur_sigma_sq.sqrt().min(world_sigma_cap);
        let reach = sigma * SHADOW_BLUR_REACH_SIGMAS;
        let blur_box = Bounds {
            min_x: local.min_x - reach,
            min_y: local.min_y - reach,
            max_x: local.max_x + reach,
            max_y: local.max_y + reach,
        };
        acc = acc.union(&blur_box);
    }
    // Project the expanded local box to world space (corner-wise AABB).
    acc.transformed(&world_t)
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
        if shadow.kind != ShadowKind::Drop {
            continue; // inner shadows are painted separately (see draw_inner_shadows).
        }
        // A fully-transparent shadow paints nothing — skip it (and don't pay for
        // its layer-sized blur). `color.a` is the 8-bit alpha byte.
        if shadow.color.a == 0 {
            continue;
        }
        // Zoom-out LOD: a shadow whose offset+spread+blur all collapse under a
        // device pixel hides behind the silhouette's AA edge — skip its filter.
        if shadow_is_subpixel(shadow, effective_scale) {
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
            shadow_layers.push(f);
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
/// - **Instance / Bitmap / Video / Text / etc.**: `None` — an inner shadow on a
///   text run or an instance box is rare and its silhouette is ill-defined here
///   (text wants per-glyph, an instance wants its expanded subtree), so we skip
///   rather than ring a wrong box.
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
                rounded_rect_path(local, v.corner_radius, v.corner_radii, 0.0)
            } else {
                to_sk_path(&v.path)
            })
        }
        NodeData::Group(g) => {
            let box_bounds = match g.clip_size {
                Some([w, h]) => Some(Bounds::from_xywh(0.0, 0.0, w, h)),
                None if g.background.is_some() || !g.strokes.is_empty() => {
                    scene_id.and_then(|id| scene.local_bounds(id))
                }
                None => None,
            };
            box_bounds.map(|b| {
                rounded_rect_path(
                    bounds_to_f32(&b),
                    g.corner_radius,
                    g.corner_radii,
                    g.corner_smoothing,
                )
            })
        }
        _ => None,
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

    // Cheap pre-check: nothing to do unless at least one visible inner shadow.
    if !node
        .effects
        .iter()
        .any(|s| s.kind == ShadowKind::Inner && s.color.a != 0)
    {
        return;
    }
    let Some(path) = node_silhouette_path(node, scene_id, ctx.scene) else {
        return;
    };

    for shadow in &node.effects {
        if shadow.kind != ShadowKind::Inner || shadow.color.a == 0 {
            continue;
        }
        // Zoom-out LOD, mirroring the drop-shadow skip: a sub-pixel inner ring
        // is invisible, so don't build its filter DAG.
        if shadow_is_subpixel(shadow, ctx.effective_scale) {
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

/// Map a [`fanta_doc::BlendMode`] to its [`skia_safe::BlendMode`] equivalent.
///
/// `fanta_doc::BlendMode` is the CSS Compositing Level 1 / `SkBlendMode` set, so
/// each separable + non-separable mode has an exact Skia counterpart. `Normal`
/// maps to Skia `SrcOver` (regular alpha-over) — though `Normal` never reaches a
/// layer paint in practice, since [`begin_effects_layer`] skips the layer for it
/// unless opacity/shadow already forced one.
pub(crate) fn to_sk_blend_mode(mode: BlendMode) -> skia_safe::BlendMode {
    use skia_safe::BlendMode as Sk;
    match mode {
        BlendMode::Normal => Sk::SrcOver,
        BlendMode::Multiply => Sk::Multiply,
        BlendMode::Screen => Sk::Screen,
        BlendMode::Overlay => Sk::Overlay,
        BlendMode::Darken => Sk::Darken,
        BlendMode::Lighten => Sk::Lighten,
        BlendMode::ColorDodge => Sk::ColorDodge,
        BlendMode::ColorBurn => Sk::ColorBurn,
        BlendMode::HardLight => Sk::HardLight,
        BlendMode::SoftLight => Sk::SoftLight,
        BlendMode::Difference => Sk::Difference,
        BlendMode::Exclusion => Sk::Exclusion,
        BlendMode::Hue => Sk::Hue,
        BlendMode::Saturation => Sk::Saturation,
        BlendMode::Color => Sk::Color,
        BlendMode::Luminosity => Sk::Luminosity,
    }
}
