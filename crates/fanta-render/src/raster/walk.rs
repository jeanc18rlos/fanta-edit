//! The depth-first scene walk: [`render_node`] (one live-scene node + its
//! children), the shared mask-aware [`paint_child_sequence`], and the
//! variable-binding [`resolve_overlay`]. The transient instance-subtree walk
//! lives in [`instance`](super::instance) and shares these helpers.
use super::effects::{effects_layer_paint, group_clips_children};
use super::layer_cache::{LayerCache, LayerLookup, render_layer_via_cache};
use super::{
    AssetResolver, BlendMode, BooleanCache, Bounds, Canvas, CanvasNode, IdHashMap, ImageCache,
    InstanceCache, MaskType, NodeData, NodeFlags, NodeId, Paint, PathCache, RenderInputs,
    RenderMetrics, Scene, Transform2D, apply_background_blur, draw_inner_shadows,
    effects_layer_bounds, opacity_folds_into_paint, padded_layer_rect, paint_node_content,
    paint_node_foreground, render_instance, resolve_bound_value, shadow_expanded_local_bounds,
    to_sk_matrix, visible_effects,
};

// ---------------------------------------------------------------------------
// Tree walker
// ---------------------------------------------------------------------------

/// State threaded through the depth-first scene walk. Bundled into one struct
/// so adding the cull rect, the image cache, the instance cache, and the
/// doc-level inputs did not balloon every recursive call's argument list.
pub(crate) struct RenderCtx<'a> {
    pub(crate) scene: &'a Scene,
    pub(crate) resolver: Option<&'a dyn AssetResolver>,
    pub(crate) cache: &'a mut ImageCache,
    /// Cross-frame instance-expansion memo (see [`InstanceCache`]).
    pub(crate) instance_cache: &'a mut InstanceCache,
    /// Boolean fold cache (expensive PathOp results). See [`BooleanCache`].
    pub(crate) boolean_cache: &'a mut BooleanCache,
    /// Built vector-node path cache. See [`PathCache`].
    pub(crate) path_cache: &'a mut PathCache,
    /// Component library + variable registry + active modes for instance
    /// expansion and the binding overlay.
    pub(crate) inputs: &'a RenderInputs<'a>,
    /// Real scene node whose ancestor chain supplies effective variable modes
    /// while walking a transient component expansion. Nested instance clones
    /// keep the outer placed instance's anchor because their fresh ids do not
    /// exist in `scene`.
    pub(crate) instance_mode_anchor: Option<NodeId>,
    /// Per-frame geometry derived from the variable + motion overlay. Scene
    /// geometry caches deliberately remain authored-state caches; playback
    /// must not mutate or invalidate them on every sample.
    pub(crate) resolved_local_transforms: IdHashMap<NodeId, Transform2D>,
    pub(crate) resolved_world_transforms: IdHashMap<NodeId, Transform2D>,
    pub(crate) resolved_local_bounds: IdHashMap<NodeId, Option<Bounds>>,
    /// Visible region in world coordinates; nodes whose world AABB (expanded by
    /// any drop shadow) misses this are culled. See [`visible_world_rect`].
    pub(crate) visible: Bounds,
    /// The canvas scale the frame applies: `viewport.zoom · display_scale`.
    /// Threaded so the drop-shadow filter can cap its ON-SCREEN Gaussian sigma
    /// (`world_sigma · effective_scale`) and the cull test can expand a node's
    /// world bounds by its shadow's screen-bounded blur extent.
    pub(crate) effective_scale: f32,
    /// The render root whose solid background already cleared the whole
    /// surface (the page canvas color). `paint_group` skips this node's own
    /// background so a translucent page color is not composited twice over
    /// the children-bounds rect.
    pub(crate) page_background_root: Option<NodeId>,
    /// Whether the target can allocate offscreen save-layers. Raster/GPU
    /// surfaces can; Skia's SVG device cannot. The SVG inspection adapter
    /// keeps geometry visible by bypassing layer-only effects and masks, while
    /// the harness reports every such approximation before rendering.
    pub(crate) supports_offscreen_layers: bool,
    /// Alpha multiplied into every Solid/Gradient vector paint while the
    /// current node's own content paints. `1.0` except for a leaf whose node
    /// opacity was folded into its single draw instead of a save-layer (see
    /// [`opacity_folds_into_paint`]); the walk sets it around
    /// `paint_node_content` and restores it after, so it never leaks into
    /// children or siblings.
    pub(crate) paint_alpha: f32,
    /// Cross-frame effect-layer cache (see [`LayerCache`]).
    pub(crate) layer_cache: &'a mut LayerCache,
    /// Whether this frame may serve effect layers from the cache at all
    /// (cache enabled, offscreen layers supported, no motion sample — a
    /// motion overlay moves nodes without any epoch input changing).
    pub(crate) layer_cache_lookups: bool,
    /// Whether this frame may ADD entries to the layer cache (see
    /// [`LayerCache::begin_frame`]). Never true when lookups are off.
    pub(crate) layer_cache_populate: bool,
    /// Set by content painters when what they drew may change without any
    /// [`LayerEpoch`](super::layer_cache::LayerEpoch) input moving — an
    /// unresolved image (placeholder), live media, an orbitable 3D model.
    /// The layer cache reads it after rendering a layer's body and refuses to
    /// store a volatile layer; nested layers propagate it outward.
    pub(crate) layer_volatile: bool,
    pub(crate) metrics: &'a mut RenderMetrics,
}

/// The transform that will actually be painted for a live node this frame.
/// Variables cannot target transforms, but going through the shared overlay
/// keeps the ordering contract explicit: bound literals resolve first and the
/// motion sample is applied last.
fn resolved_local_transform(ctx: &mut RenderCtx, id: NodeId) -> Option<Transform2D> {
    if ctx.inputs.motion.is_none() {
        return ctx.scene.get(id).map(|node| node.transform);
    }
    if let Some(transform) = ctx.resolved_local_transforms.get(&id) {
        return Some(*transform);
    }
    let transform = {
        let node = ctx.scene.get(id)?;
        let scratch = resolve_overlay(ctx, id, node);
        scratch.as_deref().unwrap_or(node).transform
    };
    ctx.resolved_local_transforms.insert(id, transform);
    Some(transform)
}

/// Parent-composed transform matching the save/concat sequence used by the
/// painter. Unlike [`Scene::world_transform`], this includes transient motion
/// channels while leaving the authored scene cache untouched.
fn resolved_world_transform(ctx: &mut RenderCtx, id: NodeId) -> Option<Transform2D> {
    if ctx.inputs.motion.is_none() {
        return ctx.scene.world_transform(id);
    }
    if let Some(transform) = ctx.resolved_world_transforms.get(&id) {
        return Some(*transform);
    }
    let parent = ctx.scene.get(id)?.parent;
    let local = resolved_local_transform(ctx, id)?;
    let parent_world = match parent {
        Some(parent) => resolved_world_transform(ctx, parent)?,
        None => Transform2D::IDENTITY,
    };
    let world = local.then(&parent_world);
    ctx.resolved_world_transforms.insert(id, world);
    Some(world)
}

/// Node-local subtree bounds matching the values and transforms that will be
/// painted for the current motion sample. Intrinsic nodes use their resolved
/// geometry directly; unclipped groups union resolved children. Boolean
/// operands remain authored until the path-fold renderer can consume overlays.
fn resolved_local_bounds(ctx: &mut RenderCtx, id: NodeId) -> Option<Bounds> {
    if ctx.inputs.motion.is_none() {
        return ctx.scene.local_bounds(id);
    }
    if let Some(bounds) = ctx.resolved_local_bounds.get(&id) {
        return *bounds;
    }

    let (union_children, mut computed) = {
        let node = ctx.scene.get(id)?;
        let scratch = resolve_overlay(ctx, id, node);
        let node = scratch.as_deref().unwrap_or(node);
        match &node.data {
            NodeData::Group(group) => {
                let clips_children = group.clip_size.is_some()
                    && node
                        .meta
                        .get("clip_content")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(true);
                (
                    !clips_children,
                    group
                        .clip_size
                        .or(group.local_size)
                        .map(|[width, height]| Bounds::from_xywh(0.0, 0.0, width, height)),
                )
            }
            NodeData::Boolean(_) => (false, ctx.scene.local_bounds(id)),
            data => (false, data.local_bounds()),
        }
    };

    if union_children {
        let scene: &Scene = ctx.scene;
        let children: &[NodeId] = scene.children_of(Some(id));
        for &child in children {
            let Some(child_bounds) = resolved_local_bounds(ctx, child) else {
                continue;
            };
            let Some(child_transform) = resolved_local_transform(ctx, child) else {
                continue;
            };
            let Some(in_parent) = child_bounds.try_transformed(&child_transform) else {
                continue;
            };
            computed = Some(match computed {
                Some(current) => current.union(&in_parent),
                None => in_parent,
            });
        }
    }
    ctx.resolved_local_bounds.insert(id, computed);
    computed
}

/// Draw one node of the *live scene* (and its scene children), depth-first.
///
/// Geometry, visibility, culling, and painting read the same transient overlay
/// (see [`resolve_overlay`]); the document is never mutated by rendering.
pub(crate) fn render_node(canvas: &Canvas, id: NodeId, ctx: &mut RenderCtx) {
    render_node_with_clip(canvas, id, ctx);
}

fn render_node_with_clip(canvas: &Canvas, id: NodeId, ctx: &mut RenderCtx) {
    ctx.metrics.nodes_visited += 1;
    let Some(world_transform) = resolved_world_transform(ctx, id) else {
        return;
    };
    if !world_transform.is_finite() {
        ctx.metrics.nodes_culled += 1;
        return;
    }

    let local_bounds = resolved_local_bounds(ctx, id);
    let Some(node) = ctx.scene.get(id) else {
        return;
    };

    // Resolve variables first, then motion, before every property-dependent
    // decision. This makes motion keyframes final overrides while keeping the
    // authored node untouched.
    let scratch = resolve_overlay(ctx, id, node);
    let node: &CanvasNode = scratch.as_deref().unwrap_or(node);

    if node.flags.contains(NodeFlags::HIDDEN) {
        return;
    }
    let opacity = node.opacity.get();
    if opacity <= 0.0 {
        return;
    }

    // Viewport cull on the RESOLVED node's world bounds, EXPANDED by any drop
    // shadow's reach so a node that is only on-screen *because of its shadow*
    // (its body is off-screen but the offset+blurred shadow reaches in) is kept,
    // and conversely a node whose body AND shadow both miss the viewport is
    // culled. A binding can change a node's color / text / opacity / visibility
    // / clip size. Motion can also replace transform channels, so both the
    // local subtree bounds and the parent-composed transform come from the
    // frame-local resolved caches above.
    //
    // Subtree-skip safety: a group's world bounds are the union of its
    // children's world bounds, so if the union (here expanded by the group's own
    // shadow) misses the viewport every child misses it too. A node with no
    // intrinsic bounds (empty group) returns `None`; we do not cull those
    // (nothing to draw, recursion is cheap). A child's *own* shadow reaches at
    // most `~3·sigma + offset` past the group bounds; descendant shadows are not
    // folded into a parent's cull box, so the per-node expansion below is what
    // keeps each shadowed leaf correct — the dominant case.
    //
    // The world box is derived from the local bounds and world transform
    // fetched above rather than re-asking the scene (`Scene::world_bounds`
    // repeats both memo lookups; motion-free it is exactly
    // `local.try_transformed(&world_transform)`), which trims two hash lookups
    // per visited node from the walk.
    if let Some(local) = local_bounds {
        let expanded =
            shadow_expanded_local_bounds(local, &node.effects, &node.blurs, ctx.effective_scale)
                .try_transformed(&world_transform);
        if expanded.is_some_and(|expanded| !expanded.intersects(&ctx.visible)) {
            ctx.metrics.nodes_culled += 1;
            return;
        }
    }

    canvas.save();
    canvas.concat(&to_sk_matrix(&node.transform));

    // BACKGROUND blur ("frosted glass"): sample the backdrop already composited
    // behind this node, blur it, and blit it back inside the node's silhouette,
    // before anything else this node draws. Runs BEFORE `begin_effects_layer`
    // deliberately: it reads the top device, which must be the real backdrop and
    // not this node's own (empty) effects layer. Self-contained — frosts nothing
    // and is free when the node carries no visible background blur.
    if ctx.supports_offscreen_layers {
        apply_background_blur(canvas, node, Some(id), ctx.scene, ctx.effective_scale);
    }

    // The node's ZOOM-EFFECTIVE effects: which authored shadows / layer blur
    // actually paint at this frame's scale (a sub-pixel blur or shadow builds
    // no filter). Computed once here and threaded to both the fold and the
    // layer decision below, so an invisible effect neither forces a layer nor
    // blocks the fold (see `VisibleEffects`).
    let visible = visible_effects(node, ctx.effective_scale);

    // Opacity fold: a leaf vector that emits exactly one Solid/Gradient draw and
    // needs a layer for nothing else applies its opacity as paint alpha instead
    // of allocating an opacity save-layer — pixel-identical for a single draw
    // (see `opacity_folds_into_paint`) and one fewer offscreen + GPU render
    // pass per translucent leaf. The alpha is threaded through `ctx.paint_alpha`
    // for exactly this node's own content and restored right after.
    let folded_opacity = opacity < 1.0
        && opacity_folds_into_paint(node, !ctx.scene.children_of(Some(id)).is_empty(), visible);

    // Wrap this node's contribution in a save-layer when it needs one: opacity
    // < 1.0 (unless folded above), ≥1 visible drop shadow, a visible LAYER
    // blur, a non-`Normal` blend mode, or an ISOLATED_BLEND container (children
    // flatten into their own group so a descendant's blend mode stops at the
    // group instead of the backdrop). The layer composites strokes/fills/
    // children together, then blends the whole node (with its shadow behind
    // it, the layer Gaussian-blurred) at the requested alpha and blend mode. A
    // node with full opacity, no visible shadows, no visible layer blur,
    // `Normal` blend, and pass-through allocates no layer (the hot path). The
    // layer's content box is only computed when a layer is actually needed:
    // `effects_layer_bounds` walks a vector's path segments (`rough_bounds`),
    // which was pure per-node overhead for every full-opacity leaf.
    let needs_layer = !folded_opacity
        && ctx.supports_offscreen_layers
        && (opacity < 1.0
            || node.blend_mode != BlendMode::Normal
            || visible.drop_shadow
            || visible.layer_blur
            || node.flags.contains(NodeFlags::ISOLATED_BLEND));
    let layer = if needs_layer {
        effects_layer_paint(
            opacity,
            node.blend_mode,
            &node.effects,
            &node.blurs,
            node.flags.contains(NodeFlags::ISOLATED_BLEND),
            ctx.effective_scale,
        )
    } else {
        None
    };

    // Vector paths may be cached across frames only when the overlay did NOT
    // replace the node this frame — a variable or motion override can alter
    // geometry the scene revision doesn't track, so an overlaid node rebuilds
    // its paths directly. The same condition gates the effect-layer cache.
    let path_cache_id = if scratch.is_none() { Some(id) } else { None };

    match layer {
        None => {
            // No layer: paint the body straight onto the canvas, threading the
            // folded opacity (if any) through `ctx.paint_alpha` for exactly
            // this node's own content.
            if folded_opacity {
                ctx.metrics.opacity_folds += 1;
                let outer_alpha = std::mem::replace(&mut ctx.paint_alpha, opacity);
                paint_node_body(canvas, id, node, path_cache_id, ctx);
                ctx.paint_alpha = outer_alpha;
            } else {
                paint_node_body(canvas, id, node, path_cache_id, ctx);
            }
        }
        Some(layer) => {
            let content_bounds = match &node.data {
                NodeData::Group(group) if !group_clips_children(node, group) => local_bounds,
                _ => effects_layer_bounds(node, Some(id), ctx.scene),
            };
            // Effect-layer cache (see `layer_cache`): a live, un-overlaid node
            // whose layer has a resolvable content box may be served from the
            // cache — pixel-identical, for a whole-device-pixel pan at the
            // same zoom — or rendered through an offscreen that populates it.
            // Everything else takes the direct save-layer path.
            let cacheable = ctx.layer_cache_lookups && path_cache_id.is_some();
            let served = cacheable
                && matches!(
                    ctx.layer_cache.lookup(canvas, id, &layer.composite_paint()),
                    LayerLookup::Hit
                );
            if served {
                ctx.metrics.layer_cache_hits += 1;
            } else {
                let via_cache = cacheable
                    && ctx.layer_cache_populate
                    && content_bounds.is_some_and(|content| {
                        render_layer_via_cache(canvas, id, &layer, content, ctx, |c, ctx| {
                            paint_node_body(c, id, node, path_cache_id, ctx);
                        })
                    });
                if via_cache {
                    ctx.metrics.effect_layers += 1;
                    ctx.metrics.layer_cache_misses += 1;
                } else {
                    // Direct: one save-layer carrying filter + alpha + blend,
                    // bounded to the padded content box (see
                    // `begin_effects_layer` for why the bounds matter).
                    ctx.metrics.effect_layers += 1;
                    let paint = layer.full_paint();
                    let rec = skia_safe::canvas::SaveLayerRec::default().paint(&paint);
                    match content_bounds {
                        Some(b) => canvas
                            .save_layer(&rec.bounds(&padded_layer_rect(&b, ctx.effective_scale))),
                        None => canvas.save_layer(&rec),
                    };
                    paint_node_body(canvas, id, node, path_cache_id, ctx);
                    canvas.restore();
                }
            }
        }
    }

    canvas.restore();
}

/// Paint one live node's BODY — its own content, its subtree (instance
/// expansion / children with mask semantics), its foreground, and its inner
/// shadows — onto `canvas`, whose CTM is the node's local space. Everything
/// an effects layer wraps: called directly when the node needs no layer,
/// inside the direct save-layer, or into the layer cache's offscreen.
fn paint_node_body(
    canvas: &Canvas,
    id: NodeId,
    node: &CanvasNode,
    path_cache_id: Option<NodeId>,
    ctx: &mut RenderCtx,
) {
    // Paint this node's own content (no children). For a scene Group the
    // background rect can fall back to the scene-computed content bounds, so we
    // pass `Some(id)`; the transient (instance) walk passes `None` and uses the
    // clip box only.
    let content_state = paint_node_content(canvas, node, Some(id), path_cache_id, ctx);

    if let Some(inst) = node.data.as_instance() {
        render_instance(canvas, node.id, inst, ctx);
    } else if node.data.as_boolean().is_some() {
        // A boolean node's children are its operands: they were already folded
        // into a single path and painted by `paint_node_content`. Descending
        // would paint the raw operands on top of the folded result, so we don't.
    } else {
        // Figma/default mode: frame clips pushed by `paint_node_content` stay on
        // the Skia stack while descendants render, so nested clips accumulate.
        // Recurse into children (groups + future nesting), honoring masks: a
        // child flagged `is_mask` masks its following siblings until the next
        // mask (or the end). The child slice is borrowed from the scene
        // (lifetime `'a`), independent of the `&mut ctx` re-borrow below — we
        // copy the `&'a Scene` reference out of `ctx` first so the slice borrow
        // doesn't pin `ctx` across the recursive `render_node` calls. Only the
        // rare reverse-z auto-layout container pays for a reversed copy; the
        // common path is allocation-free (this walk runs once per container per
        // frame, so a per-node `to_vec` was pure hot-path garbage).
        let scene: &Scene = ctx.scene;
        let children: &[NodeId] = scene.children_of(Some(id));
        let reversed: Vec<NodeId>;
        let children: &[NodeId] = if matches!(
            &node.data,
            NodeData::Group(g) if g.auto_layout.as_ref().is_some_and(|al| al.reverse_z)
        ) {
            reversed = children.iter().rev().copied().collect();
            &reversed
        } else {
            children
        };
        paint_child_sequence(
            canvas,
            ctx,
            children,
            // Mask identity/type are structural, but visibility is resolved: a
            // variable or motion keyframe can turn a mask off for this frame.
            |ctx, &child| {
                let node = ctx.scene.get(child)?;
                let scratch = resolve_overlay(ctx, child, node);
                let node = scratch.as_deref().unwrap_or(node);
                (node.is_mask && !node.flags.contains(NodeFlags::HIDDEN)).then_some(node.mask_type)
            },
            // One child subtree's painted extent in THIS node's local space:
            // the frame-resolved subtree bounds, expanded by the child's own
            // drop-shadow / layer-blur reach (descendant shadows are absorbed
            // by the shared device-pixel pad, like the cull box), mapped
            // through the child's transform.
            |ctx, &child| {
                let local = resolved_local_bounds(ctx, child)?;
                let node = ctx.scene.get(child)?;
                let scratch = resolve_overlay(ctx, child, node);
                let node = scratch.as_deref().unwrap_or(node);
                shadow_expanded_local_bounds(local, &node.effects, &node.blurs, ctx.effective_scale)
                    .try_transformed(&node.transform)
            },
            |canvas, ctx, &child| render_node(canvas, child, ctx),
        );
    }

    if content_state.restore_child_clip {
        canvas.restore();
    }
    paint_node_foreground(canvas, node, Some(id), ctx);

    // Inner shadows composite over the node's flattened content — fill, border,
    // AND children — clipped to its silhouette (Figma applies node-level effects
    // to the composited node, so a frame's inner shadow darkens edge-touching
    // children instead of hiding beneath them). Painted after the child clip is
    // restored (its own silhouette clip still confines it) and inside the
    // effects layer so opacity/blend apply to the shadow too. Drop shadows
    // already rode the layer paint above.
    if ctx.supports_offscreen_layers {
        draw_inner_shadows(canvas, node, Some(id), ctx);
    }
}

/// Paint an ordered sequence of children, applying **mask** semantics: a child
/// flagged as a mask (via `mask_of`) masks the FOLLOWING siblings — up to the
/// next mask child or the end of the sequence — by the mask shape's alpha
/// ([`MaskType::Alpha`]) or luminance ([`MaskType::Luminance`]). The mask node
/// itself is not painted as normal content; only its shape drives the mask.
///
/// Shared by the live-scene walk ([`render_node`]) and the transient
/// instance-subtree walk ([`render_expanded`]) so masks inside components render
/// identically to masks placed directly in the scene. `mask_of` returns
/// `Some(mask_type)` for a mask child and `None` otherwise; `paint` draws one
/// child's full subtree (the same call the non-masked path uses).
///
/// ## Compositing (Skia `DstIn` + optional luma filter)
///
/// For a mask + its masked run we:
/// 1. `save_layer` an OFFSCREEN content layer and paint the masked siblings into
///    it (so the mask is applied to their combined result, not each one).
/// 2. `save_layer` a second layer whose paint blends with [`DstIn`] (and, for a
///    LUMINANCE mask, carries a luma→alpha [`ColorFilter`] so brighter mask
///    pixels reveal more), and paint the MASK's content into it. `DstIn` keeps
///    the content (destination) only where the mask (source) has alpha — exactly
///    Figma's mask. Restoring the mask layer composites it onto the content;
///    restoring the content layer blends the masked result against the backdrop.
///
/// A mask with no following siblings paints nothing (there is nothing to mask) —
/// matching Figma, where a mask at the end of a parent's children is invisible.
///
/// Non-mask children take the fast path (a direct `paint`), so a parent with no
/// masks allocates no extra layer.
///
/// ## Consecutive masks intersect
///
/// A *run* of two or more masks in a row (mask, mask, …, then the first
/// non-mask sibling) compounds: the following content is clipped to the
/// INTERSECTION of every mask in the run — exactly Figma, where stacking two
/// masks over a layer shows it only where BOTH masks cover. Each mask in the
/// run keeps its own [`MaskType`] (an alpha mask and a luminance mask can be
/// stacked). The intersection is built by applying each mask's silhouette as a
/// successive [`DstIn`] layer over the same content: `DstIn` multiplies the
/// destination alpha by the source's (luma-)alpha, so `content · m₁ · m₂ · …`
/// survives only where every mask has coverage. A run of exactly one mask
/// reduces to a single `DstIn` layer (byte-identical to the single-mask path).
///
/// Nested masks recurse naturally because each subtree paints through its own
/// `paint_child_sequence`.
///
/// ## Layer bounds
///
/// Without explicit bounds Skia sizes each of these offscreen layers from the
/// clip — the whole viewport — so at high zoom every masked run costs a stack
/// of viewport-sized allocations per frame (the same pathology
/// [`begin_effects_layer`] bounds away). `bounds_of` reports one child
/// subtree's painted extent in the CURRENT canvas space (shadow-expanded, so a
/// child's drop shadow is not cropped); the union over the masked run, padded
/// like the effects layer, bounds the content layer. Every `DstIn` mask layer
/// uses the SAME rect deliberately: a mask layer smaller than the content
/// layer would leave whatever it doesn't cover un-multiplied — i.e. visible
/// UNmasked — at restore time. If any child's extent is unknown (`None`) the
/// run falls back to unbounded layers — correct, just slower.
pub(crate) fn paint_child_sequence<C, M, B, P>(
    canvas: &Canvas,
    ctx: &mut RenderCtx,
    children: &[C],
    mut mask_of: M,
    mut bounds_of: B,
    mut paint: P,
) where
    M: FnMut(&RenderCtx, &C) -> Option<MaskType>,
    B: FnMut(&mut RenderCtx, &C) -> Option<Bounds>,
    P: FnMut(&Canvas, &mut RenderCtx, &C),
{
    if !ctx.supports_offscreen_layers {
        // Skia's SVG device cannot create the offscreen devices required for a
        // mask layer. Keep the inspection document useful by omitting the mask
        // shapes and painting their content siblings unmasked. The headless
        // preflight marks this as raster-only, and strict mode rejects it.
        for child in children {
            if mask_of(ctx, child).is_none() {
                paint(canvas, ctx, child);
            }
        }
        return;
    }

    let mut i = 0;
    while i < children.len() {
        if mask_of(ctx, &children[i]).is_none() {
            // Plain child: paint directly, no layer.
            paint(canvas, ctx, &children[i]);
            i += 1;
            continue;
        }

        // A run of one OR MORE consecutive masks at `i` masks the following
        // siblings. The masks themselves occupy [mask_start, mask_end); the
        // masked content run is [content_start, content_end), where
        // content_end stops at the next mask (or the end). Consecutive masks
        // compound into an intersection over the same content run.
        let mask_start = i;
        let mut mask_end = i;
        while mask_end < children.len() && mask_of(ctx, &children[mask_end]).is_some() {
            mask_end += 1;
        }
        let content_start = mask_end;
        let mut content_end = content_start;
        while content_end < children.len() && mask_of(ctx, &children[content_end]).is_none() {
            content_end += 1;
        }

        // Nothing to mask (the run of masks reaches the end of the children):
        // the masks themselves are not painted, so skip them.
        if content_start == content_end {
            i = content_end;
            continue;
        }

        // The shared bounds for the content layer AND every mask layer of this
        // run (see the fn docs for why they must match): the padded union of
        // the masked children's extents, or `None` (unbounded) when any extent
        // is unknown.
        let layer_bounds = {
            let mut acc: Option<Bounds> = None;
            let mut all_known = true;
            for child in &children[content_start..content_end] {
                match bounds_of(ctx, child) {
                    Some(b) => {
                        acc = Some(match acc {
                            Some(a) => a.union(&b),
                            None => b,
                        });
                    }
                    None => {
                        all_known = false;
                        break;
                    }
                }
            }
            match (all_known, acc) {
                (true, Some(b)) => Some(padded_layer_rect(&b, ctx.effective_scale)),
                _ => None,
            }
        };

        // (1) Offscreen content layer: paint the masked siblings into it.
        let content_rec = skia_safe::canvas::SaveLayerRec::default();
        let content_rec = match layer_bounds.as_ref() {
            Some(rect) => content_rec.bounds(rect),
            None => content_rec,
        };
        canvas.save_layer(&content_rec);
        for child in &children[content_start..content_end] {
            paint(canvas, ctx, child);
        }

        // (2) One DstIn mask layer PER mask in the run, innermost-first. Each
        // layer paints its mask's silhouette and blends with DstIn (plus a
        // luma filter for a LUMINANCE mask), keeping the content only where
        // that mask has (luma-)alpha. Stacking the layers multiplies their
        // coverages, so the content survives only in the INTERSECTION of all
        // the masks. A single mask in the run pushes exactly one such layer —
        // identical to the prior single-mask path.
        for mask_child in &children[mask_start..mask_end] {
            let mask_type = mask_of(ctx, mask_child).unwrap_or_default();
            let mut mask_paint = Paint::default();
            mask_paint.set_blend_mode(skia_safe::BlendMode::DstIn);
            if mask_type == MaskType::Luminance {
                mask_paint.set_color_filter(skia_safe::ColorFilter::luma());
            }
            let mask_rec = skia_safe::canvas::SaveLayerRec::default().paint(&mask_paint);
            let mask_rec = match layer_bounds.as_ref() {
                Some(rect) => mask_rec.bounds(rect),
                None => mask_rec,
            };
            canvas.save_layer(&mask_rec);
            paint(canvas, ctx, mask_child);
        }
        // Restore each mask layer (DstIn composited onto the layer below), then
        // the content layer (blended against the backdrop).
        for _ in mask_start..mask_end {
            canvas.restore();
        }
        canvas.restore(); // content layer → blended against backdrop

        i = content_end;
    }
}

/// Build the scratch copy of `node` with its variable bindings resolved in the
/// node's effective mode, or `None` if the node binds nothing (the common case,
/// so the hot path allocates nothing).
///
/// Each `(BoundProp, VariableId)` is resolved via [`resolve_bound_value`] —
/// which itself picks the effective mode per collection (nearest-ancestor
/// frame pin → doc active mode → collection default) — and applied with
/// [`fanta_doc::BoundProp::apply_resolved`]. A binding that doesn't resolve
/// (dangling variable, type mismatch) is skipped, leaving the literal in place.
pub(crate) fn resolve_overlay(
    ctx: &RenderCtx,
    id: NodeId,
    node: &CanvasNode,
) -> Option<Box<CanvasNode>> {
    let has_motion = ctx
        .inputs
        .motion
        .is_some_and(|motion| motion.overrides.keys().any(|target| target.node == id));
    if node.bindings.is_empty() && !has_motion {
        return None;
    }
    let mut scratch = node.clone();
    for (prop, var_id) in &node.bindings {
        if let Some(resolved) = resolve_bound_value(
            ctx.inputs.variables,
            ctx.scene,
            id,
            ctx.inputs.active_modes,
            *var_id,
        ) {
            prop.apply_resolved(&mut scratch, resolved);
        }
    }
    if let Some(motion) = ctx.inputs.motion {
        scratch = motion.apply_to_node(&scratch);
    }
    Some(Box::new(scratch))
}
