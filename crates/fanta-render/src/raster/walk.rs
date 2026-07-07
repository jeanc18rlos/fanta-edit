//! The depth-first scene walk: [`render_node`] (one live-scene node + its
//! children), the shared mask-aware [`paint_child_sequence`], and the
//! variable-binding [`resolve_overlay`]. The transient instance-subtree walk
//! lives in [`instance`](super::instance) and shares these helpers.
use super::{
    AssetResolver, Bounds, Canvas, CanvasNode, ImageCache, InstanceCache, MaskType, NodeData,
    NodeFlags, NodeId, Paint, RenderInputs, RenderMetrics, Scene, apply_background_blur,
    begin_effects_layer, draw_inner_shadows, effects_layer_bounds, paint_node_content,
    render_instance, resolve_bound_value, shadow_expanded_world_bounds, to_sk_matrix,
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
    /// Component library + variable registry + active modes for instance
    /// expansion and the binding overlay.
    pub(crate) inputs: &'a RenderInputs<'a>,
    /// Visible region in world coordinates; nodes whose world AABB (expanded by
    /// any drop shadow) misses this are culled. See [`visible_world_rect`].
    pub(crate) visible: Bounds,
    /// The canvas scale the frame applies: `viewport.zoom · display_scale`.
    /// Threaded so the drop-shadow filter can cap its ON-SCREEN Gaussian sigma
    /// (`world_sigma · effective_scale`) and the cull test can expand a node's
    /// world bounds by its shadow's screen-bounded blur extent.
    pub(crate) effective_scale: f32,
    pub(crate) metrics: &'a mut RenderMetrics,
}

/// Draw one node of the *live scene* (and its scene children), depth-first.
///
/// The geometry/visibility checks and the cull test read the live node; only
/// the *painted* values are taken from a scratch copy carrying the resolved
/// variable bindings (see [`resolve_overlay`]), so the document is never
/// mutated by rendering.
pub(crate) fn render_node(canvas: &Canvas, id: NodeId, ctx: &mut RenderCtx) {
    render_node_with_clip(canvas, id, ctx);
}

fn render_node_with_clip(canvas: &Canvas, id: NodeId, ctx: &mut RenderCtx) {
    ctx.metrics.nodes_visited += 1;
    let Some(node) = ctx.scene.get(id) else {
        return;
    };
    let Some(world_transform) = ctx.scene.world_transform(id) else {
        return;
    };
    if !world_transform.is_finite() {
        ctx.metrics.nodes_culled += 1;
        return;
    }

    // Viewport cull on the LIVE node's world bounds, EXPANDED by any drop
    // shadow's reach so a node that is only on-screen *because of its shadow*
    // (its body is off-screen but the offset+blurred shadow reaches in) is kept,
    // and conversely a node whose body AND shadow both miss the viewport is
    // culled. A binding can change a node's color / text / opacity / visibility
    // / clip size but not its transform or effects, so the live geometry +
    // effects are the right cull reference. (ClipWidth / ClipHeight only shrink
    // a frame's own box; `world_bounds` is the union of descendants, which a
    // smaller clip can only over-estimate — safe to keep.)
    //
    // Subtree-skip safety: a group's world bounds are the union of its
    // children's world bounds, so if the union (here expanded by the group's own
    // shadow) misses the viewport every child misses it too. A node with no
    // intrinsic bounds (empty group) returns `None`; we do not cull those
    // (nothing to draw, recursion is cheap). A child's *own* shadow reaches at
    // most `~3·sigma + offset` past the group bounds; descendant shadows are not
    // folded into a parent's cull box, so the per-node expansion below is what
    // keeps each shadowed leaf correct — the dominant case.
    if let Some(world) = ctx.scene.world_bounds(id) {
        let expanded = shadow_expanded_world_bounds(
            ctx.scene,
            id,
            &node.effects,
            &node.blurs,
            world,
            ctx.effective_scale,
        );
        if !expanded.intersects(&ctx.visible) {
            ctx.metrics.nodes_culled += 1;
            return;
        }
    }

    // Variable-resolution overlay: if the node binds any property, resolve each
    // binding in this node's effective mode and write it onto a scratch CLONE,
    // then paint the clone. Unbound nodes paint by reference (no clone). The
    // returned value is borrowed as `&CanvasNode` either way.
    let scratch = resolve_overlay(ctx, id, node);
    let node: &CanvasNode = scratch.as_deref().unwrap_or(node);

    // Visibility + fully-transparent skip read AFTER the overlay so a `Visible`
    // or `Opacity` binding takes effect.
    if node.flags.contains(NodeFlags::HIDDEN) {
        return;
    }
    let opacity = node.opacity.clamp(0.0, 1.0);
    if opacity <= 0.0 {
        return;
    }

    canvas.save();
    canvas.concat(&to_sk_matrix(&node.transform));

    // BACKGROUND blur ("frosted glass"): sample the backdrop already composited
    // behind this node, blur it, and blit it back inside the node's silhouette,
    // before anything else this node draws. Runs BEFORE `begin_effects_layer`
    // deliberately: it reads the top device, which must be the real backdrop and
    // not this node's own (empty) effects layer. Self-contained — frosts nothing
    // and is free when the node carries no visible background blur.
    apply_background_blur(canvas, node, Some(id), ctx.scene, ctx.effective_scale);

    // Wrap this node's contribution in a save-layer when it needs one: opacity
    // < 1.0, ≥1 drop shadow, a LAYER blur, or a non-`Normal` blend mode. The
    // layer composites strokes/fills/children together, then blends the whole
    // node (with its shadow behind it, the layer Gaussian-blurred) at the
    // requested alpha and blend mode. A node with full opacity, no shadows, no
    // layer blur, and `Normal` blend allocates no layer (the hot path).
    let used_layer = begin_effects_layer(
        canvas,
        opacity,
        node.blend_mode,
        &node.effects,
        &node.blurs,
        ctx.effective_scale,
        effects_layer_bounds(node, Some(id), ctx.scene),
    );

    // Paint this node's own content (no children). For a scene Group the
    // background rect can fall back to the scene-computed content bounds, so we
    // pass `Some(id)`; the transient (instance) walk passes `None` and uses the
    // clip box only.
    paint_node_content(canvas, node, Some(id), ctx);

    // Inner shadows ride ON TOP of the node's own fill (and under any children),
    // clipped to its silhouette. Drop shadows already rode the layer paint above.
    draw_inner_shadows(canvas, node, Some(id), ctx);

    if let NodeData::Instance(inst) = &node.data {
        render_instance(canvas, node.id, inst, ctx);
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
            // The mask classification reads the LIVE node: `is_mask`/`mask_type`
            // are structural (a binding never changes them), like visibility. A
            // HIDDEN mask masks nothing (its run paints normally) — matching
            // Figma, where toggling a mask off reveals the full siblings.
            |ctx, &child| {
                ctx.scene
                    .get(child)
                    .filter(|n| n.is_mask && !n.flags.contains(NodeFlags::HIDDEN))
                    .map(|n| n.mask_type)
            },
            |canvas, ctx, &child| render_node(canvas, child, ctx),
        );
    }

    if used_layer {
        canvas.restore();
    }
    canvas.restore();
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
pub(crate) fn paint_child_sequence<C, M, P>(
    canvas: &Canvas,
    ctx: &mut RenderCtx,
    children: &[C],
    mut mask_of: M,
    mut paint: P,
) where
    M: FnMut(&RenderCtx, &C) -> Option<MaskType>,
    P: FnMut(&Canvas, &mut RenderCtx, &C),
{
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

        // (1) Offscreen content layer: paint the masked siblings into it.
        canvas.save_layer(&skia_safe::canvas::SaveLayerRec::default());
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
            canvas.save_layer(&skia_safe::canvas::SaveLayerRec::default().paint(&mask_paint));
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
    if node.bindings.is_empty() {
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
    Some(Box::new(scratch))
}
