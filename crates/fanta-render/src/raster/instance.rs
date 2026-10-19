//! Component-instance rendering: expand a master subtree (memoized), run the
//! auto-layout solver over it, and walk the transient clone tree
//! ([`render_expanded`]) mirroring the live-scene walk. Also hosts the public
//! [`solve_scene_layout`] entry point for scene-direct auto-layout frames.
use super::effects::group_clips_children;
use super::renderer::PreparedInstance;
use super::{
    Arc, BlendMode, Bounds, Canvas, CanvasNode, ExpandedNode, HashMap, IdHashMap, InstanceCacheKey,
    InstanceNode, NodeData, NodeFlags, NodeId, RenderCtx, Scene, apply_background_blur,
    begin_effects_layer, draw_inner_shadows, draw_unresolved_outline, opacity_folds_into_paint,
    paint_child_sequence, paint_node_content, paint_node_foreground, resolve_overlay, to_sk_matrix,
    visible_effects, with_shaped_layout,
};
use skia_safe::QuickReject;

// ---------------------------------------------------------------------------
// Component-instance rendering
// ---------------------------------------------------------------------------

/// Expand and draw a component instance's master subtree. The caller has
/// already concatenated the instance's own transform, so the expanded subtree
/// paints in the instance's local space. Content confinement comes from the
/// expansion root itself: its `clip_size` is pinned to the instance box
/// (`pin_expansion_root_box`), so the root's frame clip crops descendants while
/// the root's own border correctly escapes it (Figma frame-stroke semantics).
///
/// The expansion (deep-clone of the master with overrides applied) is memoized
/// across frames; the resulting transient subtree is walked by
/// [`render_expanded`], which re-applies the per-node binding overlay and
/// recurses into nested instances. A dangling/empty expansion draws a faint
/// placeholder so a broken instance stays visible rather than vanishing.
///
/// `instance_id` is the wrapping [`CanvasNode`]'s id, used as the memo key's
/// per-instance identity slot.
pub(crate) fn render_instance(
    canvas: &Canvas,
    instance_id: NodeId,
    inst: &InstanceNode,
    ctx: &mut RenderCtx,
) {
    if super::walk::cancel_camera_work(ctx) {
        return;
    }
    let mode_anchor = if ctx.scene.get(instance_id).is_some() {
        instance_id
    } else {
        ctx.instance_mode_anchor.unwrap_or(instance_id)
    };
    let expanded = if let Some(split) = ctx.split {
        // Preparation closes the entire nested graph; never rebuild against the normal memo.
        let Some(prepared) = split.spec.prepared_instance(instance_id) else {
            ctx.metrics.incomplete_artwork = true;
            ctx.metrics.effect_failed = true;
            return;
        };
        Arc::clone(prepared)
    } else {
        expand_instance_memoized(ctx, instance_id, inst, mode_anchor)
    };
    if super::walk::cancel_camera_work(ctx) {
        return;
    }
    if expanded.nodes.is_empty() {
        ctx.metrics.incomplete_artwork = true;
        // Genuinely unresolvable instance (master deleted / not loaded). Draw a
        // faint dashed outline of the instance box so it stays *locatable*
        // without painting a heavy translucent-blue block over the design.
        draw_unresolved_outline(canvas, inst.local_size);
        ctx.metrics.nodes_drawn += 1;
        return;
    }
    // The expansion root is the entry with an empty def_path; its clone id is
    // the parent of the first level. Draw the root, then recurse via the
    // transient parent→child links reconstructed from the fresh clone ids.
    //
    // The root is drawn with `is_root = true` so its OWN transform is ignored:
    // the master root's transform is its position on the (hidden) Components
    // page, which is irrelevant — the *instance* node's transform (already
    // concatenated by the caller) is what positions this instance. Descendant
    // transforms are relative to the root's local space, so they compose
    // correctly under that identity root.
    if let Some(root) = expanded.root.and_then(|index| expanded.nodes.get(index)) {
        let previous_mode_anchor = ctx.instance_mode_anchor.replace(mode_anchor);
        render_expanded(
            canvas,
            &root.node,
            true,
            &expanded.nodes,
            &expanded.children,
            ctx.inputs
                .motion
                .is_none()
                .then_some(expanded.bounds.as_ref())
                .flatten(),
            ctx,
        );
        ctx.instance_mode_anchor = previous_mode_anchor;
    }
}

/// Look up (or compute and cache) the structural expansion of `inst`, keyed per
/// spec 07 §2 P3 by `(instance NodeId, ComponentDef.rev, override-hash, mode
/// generation)`. A dangling component yields an empty `Vec` (still cached, so we
/// don't re-probe a missing master every frame).
pub(crate) fn expand_instance_memoized(
    ctx: &mut RenderCtx,
    instance_id: NodeId,
    inst: &InstanceNode,
    mode_anchor: NodeId,
) -> Arc<PreparedInstance> {
    let expansion_context = fanta_doc::InstanceExpansionContext::new(
        ctx.inputs.variables,
        ctx.inputs.active_modes,
        mode_anchor,
    );
    ctx.instance_cache
        .record_component(ctx.inputs.components, inst.component);
    if ctx.layer_cache_lookups {
        ctx.layer_cache
            .record_component(ctx.inputs.components, inst.component);
    }
    // The RESOLVED master's revisions (a variant instance's `component` is the
    // *set* id, which is not itself a def — `def(set)` is `None` and would
    // wrongly report rev 0, never invalidating on a member edit). `rev` moves
    // for committed edits, `preview_rev` for in-flight previews that bypass
    // history; both must key the memo or a drag inside a master would not
    // refresh its instances.
    let resolved_component = fanta_doc::resolved_component_with_context(
        ctx.scene,
        ctx.inputs.components,
        inst,
        &expansion_context,
    );
    let (rev, preview_rev) = resolved_component
        .as_ref()
        .map_or((0, 0), |resolved| (resolved.rev, resolved.preview_rev));
    // Only a live-scene instance has a stamp to memoize its override hash on;
    // a nested transient clone (fresh id, not in the scene) hashes directly.
    let stamp = ctx
        .scene
        .get(instance_id)
        .and_then(|node| node.data.as_instance())
        .filter(|authored| std::ptr::eq(*authored, inst))
        .map(|_| ctx.scene.node_stamp(instance_id));
    let key = InstanceCacheKey {
        instance: instance_id,
        node_stamp: stamp,
        root: resolved_component
            .as_ref()
            .map(|resolved| resolved.resolved_root)
            .filter(|root| ctx.scene.contains(*root)),
        component: resolved_component
            .as_ref()
            .map(|resolved| resolved.resolved_component),
        rev,
        preview_rev,
        override_hash: ctx.instance_cache.override_hash(instance_id, stamp, inst),
        mode_generation: ctx.inputs.mode_generation,
        mode_pins: hash_mode_pins(ctx.scene, mode_anchor),
    };
    if let Some(budget) = &mut ctx.metrics.sampling_budget
        && let Some(component) = resolved_component
    {
        // The walk sees clones only AFTER expansion/layout. Reserve the whole
        // resolved master first so a wide recursive component cannot allocate
        // or repeatedly walk a wide subtree before the walk reaches its cap.
        // Sharing immutable nodes does not reduce sampling's expansion quota.
        for _ in ctx.scene.descendants_of(component.resolved_root) {
            if !budget.reserve_expanded_node() {
                return Arc::new(PreparedInstance::default());
            }
        }
    }
    if let Some(cached) = ctx.instance_cache.lookup(&key, inst) {
        return cached;
    }
    let expanded = if let Some(should_cancel) = ctx.should_cancel {
        let Some(mut nodes) = fanta_doc::expand_instance_with_context_cancellable(
            ctx.scene,
            ctx.inputs.components,
            inst,
            &expansion_context,
            should_cancel,
        ) else {
            ctx.metrics.camera_cancelled = true;
            return Arc::new(PreparedInstance::default());
        };
        if inst.derived.is_empty()
            && !fanta_doc::solve_expanded_cancellable(
                &mut nodes,
                &mut measure_text_node,
                should_cancel,
            )
        {
            ctx.metrics.camera_cancelled = true;
            return Arc::new(PreparedInstance::default());
        }
        let Some(prepared) = PreparedInstance::new_cancellable(nodes, should_cancel) else {
            ctx.metrics.camera_cancelled = true;
            return Arc::new(PreparedInstance::default());
        };
        Arc::new(prepared)
    } else {
        Arc::new(prepare_instance(
            ctx.scene,
            ctx.inputs.components,
            inst,
            &expansion_context,
        ))
    };
    ctx.metrics.instance_indexes_built += 1;
    ctx.instance_cache.insert(key, inst, Arc::clone(&expanded));
    expanded
}

pub(crate) fn prepare_instance(
    scene: &Scene,
    components: &fanta_doc::ComponentLibrary,
    instance: &InstanceNode,
    context: &fanta_doc::InstanceExpansionContext<'_>,
) -> PreparedInstance {
    let expanded = fanta_doc::expand_instance_with_context(scene, components, instance, context);
    prepare_instance_nodes(instance, expanded)
}

pub(crate) fn prepare_instance_nodes(
    instance: &InstanceNode,
    mut expanded: Vec<ExpandedNode>,
) -> PreparedInstance {
    // Figma's `derivedSymbolData` is already a resolved per-instance subtree, so
    // re-solving those clones moves icons/carets/text away from the baked
    // geometry — only masters without derived data need our layout pass. An
    // EDITED master keeps its baked geometry (only the fill falls through to the
    // master), so we must NOT re-solve it — doing so would resize its vectors to
    // the master box and distort them.
    if instance.derived.is_empty() {
        fanta_doc::solve_expanded(&mut expanded, &mut measure_text_node);
    }
    PreparedInstance::new(expanded)
}

/// Hash of every frame mode pin (`GroupNode::explicit_modes`) on `anchor` and
/// its ancestor chain, nearest first. The instance's effective mode — which
/// [`expand_instance_memoized`] bakes into the cached expansion through
/// alias-backed component properties — is decided by the nearest pin, so the
/// ordered sequence of pins is exactly what the expansion depends on beyond
/// the variables and doc-level modes. O(depth), no allocation.
fn hash_mode_pins(scene: &Scene, anchor: NodeId) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut visit = |node: &CanvasNode| {
        if let NodeData::Group(group) = &node.data
            && !group.explicit_modes.is_empty()
        {
            group.explicit_modes.hash(&mut hasher);
        }
    };
    if let Some(node) = scene.get(anchor) {
        visit(node);
    }
    for ancestor in scene.ancestors_of(anchor) {
        visit(ancestor);
    }
    hasher.finish()
}

/// Measure a text node's glyph box with the real `fanta-text` shaper, for the
/// auto-layout solver's auto-width and wrapped auto-height cases, returning `(width, height)`.
///
/// Goes through the **shared shaped-text cache** ([`with_shaped_layout`]) rather
/// than shaping a throwaway paragraph: the cache is keyed identically for measure
/// and draw, so this measure pass PRE-WARMS the very [`TextLayout`]s the
/// subsequent render paints. That turns a first-visit page switch from
/// double-shaping (solver measures every text node, then the cold draw cache
/// re-shapes them all) into shaping each run exactly once — the dominant cost of
/// the switch. The wrap-width/align used for shaping is the same one
/// [`draw_text_node`] would use (auto-width → unbounded single line), so the
/// measured extents and the painted glyphs agree.
pub fn measure_text_node(t: &fanta_doc::TextNode) -> (f64, f64) {
    if t.content.is_empty() {
        return (0.0, 0.0);
    }
    // Height includes the paragraph-spacing bands the draw inserts, so an
    // auto-height box hugs the pixels that actually paint.
    with_shaped_layout(t, |layout| {
        (layout.width(), super::text::painted_text_height(t, layout))
    })
}

/// Lay out the **scene-direct** auto-layout frames under `page` — the page →
/// section → card frames that live in the `Scene` itself (NOT inside a
/// component instance) — in place, so the live render shows Figma's computed
/// stack positions for them, exactly as the snapshot oracle already does.
///
/// Instance interiors are laid out separately when each instance is expanded
/// (see [`expand_instance_memoized`]), so the two passes cover disjoint sets.
/// Auto-width labels are measured with the real `fanta-text` shaper — through
/// the same shared shaped-text cache the renderer paints from, so this measure
/// pass PRE-WARMS those runs for the first render (see [`measure_text_node`]).
/// Every node it writes goes through `Scene::get_mut`, which drops the derived
/// caches and logs a precise per-node change — so culling, hit-testing, and
/// `world_bounds` observe the new geometry, and a render-thread copy of the
/// scene can still be patched past the solve instead of re-copied wholesale.
///
/// Call ONCE after import (and after edits that change auto-layout inputs) — it
/// is `O(n)` over the page subtree, not a per-frame cost.
pub fn solve_scene_layout(scene: &mut Scene, page: NodeId) {
    fanta_doc::solve_auto_layout(scene, page, &mut measure_text_node);
}

/// Resolve text-content and typography bindings before measuring layout, while
/// retaining authored literals and bindings in the document.
pub fn solve_doc_layout(doc: &mut fanta_doc::Doc, page: NodeId) {
    fanta_doc::solve_auto_layout_with_variables(
        &mut doc.scene,
        page,
        &doc.variables,
        &doc.active_modes,
        &mut measure_text_node,
    );
}

/// The local AABB of a transient (expanded-instance) node's subtree, mirroring
/// [`Scene::local_bounds`] for clones that have no scene entry: an unclipped
/// group unions its transient children's boxes (transformed into its space);
/// every other kind reports its intrinsic geometry (via
/// [`effects_layer_bounds`], which never consults the scene when `scene_id` is
/// `None`). Used to bound the effects/mask save-layers of the transient walk —
/// without it an unclipped group inside a component allocates viewport-sized
/// layers per frame (see [`begin_effects_layer`]).
fn expanded_subtree_bounds(
    node: &CanvasNode,
    expanded: &[ExpandedNode],
    children: &HashMap<NodeId, Vec<usize>>,
    cached: Option<&IdHashMap<NodeId, Option<Bounds>>>,
) -> Option<Bounds> {
    expanded_subtree_bounds_cancellable(node, expanded, children, cached, None)
}

fn expanded_subtree_bounds_cancellable(
    node: &CanvasNode,
    expanded: &[ExpandedNode],
    children: &HashMap<NodeId, Vec<usize>>,
    cached: Option<&IdHashMap<NodeId, Option<Bounds>>>,
    should_cancel: Option<&dyn Fn() -> bool>,
) -> Option<Bounds> {
    if should_cancel.is_some_and(|should_cancel| should_cancel()) {
        return None;
    }
    if let Some(bounds) = cached.and_then(|cached| cached.get(&node.id)) {
        return *bounds;
    }
    match &node.data {
        NodeData::Group(_) | NodeData::Boolean(_)
            if node.is_boolean()
                || node
                    .data
                    .as_group()
                    .is_some_and(|group| !group_clips_children(node, group)) =>
        {
            let mut acc = match &node.data {
                NodeData::Group(group) => group
                    .clip_size
                    .or(group.local_size)
                    .map(|[width, height]| Bounds::from_xywh(0.0, 0.0, width, height)),
                NodeData::Boolean(boolean) => boolean
                    .baked
                    .as_ref()
                    .and_then(|baked| baked.vector.path.rough_bounds()),
                _ => None,
            };
            for &i in children.get(&node.id).into_iter().flatten() {
                if should_cancel.is_some_and(|should_cancel| should_cancel()) {
                    return None;
                }
                let Some(entry) = expanded.get(i) else {
                    continue;
                };
                let child = &entry.node;
                let Some(b) = expanded_subtree_bounds_cancellable(
                    child,
                    expanded,
                    children,
                    cached,
                    should_cancel,
                ) else {
                    continue;
                };
                let Some(in_parent) = b.try_transformed(&child.transform) else {
                    continue;
                };
                acc = Some(match acc {
                    Some(a) => a.union(&in_parent),
                    None => in_parent,
                });
            }
            acc
        }
        NodeData::TextPath(text_path) => super::text_path_bounds(text_path),
        data => data.local_bounds(),
    }
}

pub(crate) fn prepare_expansion_bounds(
    expanded: &[ExpandedNode],
    children: &HashMap<NodeId, Vec<usize>>,
) -> IdHashMap<NodeId, Option<Bounds>> {
    let mut bounds = IdHashMap::default();
    // Expansion emits parents before descendants. Reverse order makes each
    // container's union use already-computed child bounds, without recursion.
    for entry in expanded.iter().rev() {
        let computed = expanded_subtree_bounds(&entry.node, expanded, children, Some(&bounds));
        bounds.insert(entry.node.id, computed);
    }
    bounds
}

/// Map each expanded clone's parent id → its child clone indices, so the
/// transient subtree can be walked top-down without a scene. Built once per
/// cached expansion and indexed by clone `NodeId`.
pub(crate) fn build_child_index(expanded: &[ExpandedNode]) -> HashMap<NodeId, Vec<usize>> {
    let mut children: HashMap<NodeId, Vec<usize>> = HashMap::new();
    for (i, e) in expanded.iter().enumerate() {
        if let Some(parent) = e.node.parent {
            children.entry(parent).or_default().push(i);
        }
    }
    children
}

/// Walk one transient (expanded-instance) node and its transient children,
/// mirroring [`render_node`] but sourcing children from `children` (the fresh
/// clone parent→child index) instead of the scene. The per-node binding overlay
/// and opacity/visibility/transform handling match the scene walk exactly so an
/// instance renders identically to its master placed in the scene; a nested
/// `NodeData::Instance` recurses through [`render_instance`].
///
/// `is_root` is `true` only for the expansion root: its own transform is the
/// master root's Components-page position and is *ignored* (the instance node's
/// transform, already concatenated, positions the instance). All descendants
/// pass `false` and use their own transforms.
pub(crate) fn render_expanded(
    canvas: &Canvas,
    node: &CanvasNode,
    is_root: bool,
    expanded: &[ExpandedNode],
    children: &HashMap<NodeId, Vec<usize>>,
    cached_bounds: Option<&IdHashMap<NodeId, Option<Bounds>>>,
    ctx: &mut RenderCtx,
) {
    if super::walk::cancel_camera_work(ctx) {
        return;
    }
    if ctx
        .metrics
        .sampling_budget
        .as_mut()
        .is_some_and(|budget| !budget.enter())
    {
        return;
    }
    render_expanded_inner(
        canvas,
        node,
        is_root,
        expanded,
        children,
        cached_bounds,
        ctx,
    );
    if let Some(budget) = &mut ctx.metrics.sampling_budget {
        budget.leave();
    }
}

fn render_expanded_inner(
    canvas: &Canvas,
    node: &CanvasNode,
    is_root: bool,
    expanded: &[ExpandedNode],
    children: &HashMap<NodeId, Vec<usize>>,
    cached_bounds: Option<&IdHashMap<NodeId, Option<Bounds>>>,
    ctx: &mut RenderCtx,
) {
    ctx.metrics.nodes_visited += 1;

    // Binding overlay on the transient clone. The clone is not in the scene, so
    // `resolve_effective_mode` walks no ancestors for it — it sees the doc-level
    // active modes (and the collection default). That matches Figma: an
    // instance's bound values follow the document/page theme, not a frame pin
    // that lives inside the master.
    let scratch = resolve_overlay(ctx, node.id, node);
    let node: &CanvasNode = scratch.as_deref().unwrap_or(node);

    if node.flags.contains(NodeFlags::HIDDEN) {
        return;
    }
    let opacity = node.opacity.get();
    if opacity <= 0.0 {
        return;
    }
    let boolean_shape = node
        .is_boolean()
        .then(|| super::boolean::prepare_expanded_boolean(node, expanded, children, ctx));
    let boolean_silhouette = boolean_shape.as_ref().and_then(|shape| shape.silhouette());

    canvas.save();
    // The root's own transform is suppressed (see fn docs); descendants apply
    // theirs as usual.
    if !is_root {
        canvas.concat(&to_sk_matrix(&node.transform));
    }

    if ctx.metrics.camera_visible.is_some()
        && !ctx.filtered_ancestor
        && ((!children
            .get(&node.id)
            .is_some_and(|children| !children.is_empty())
            && matches!(node.data, NodeData::Vector(_) | NodeData::Text(_)))
            || node
                .data
                .as_group()
                .is_some_and(|group| group_clips_children(node, group)))
    {
        let local = match &node.data {
            NodeData::Vector(vector) => crate::bounds::vector_visual_bounds(vector, node.flags),
            NodeData::Text(text) => super::walk::text_visual_bounds(text),
            NodeData::Group(_) => node.data.local_bounds(),
            _ => None,
        };
        if let Some(local) = local {
            let painted = super::walk::visual_local_bounds(node, local, ctx.effective_scale_axes)
                .map(super::camera_cache::bounds_rect);
            if painted.is_some_and(|painted| painted.is_finite() && canvas.quick_reject(&painted)) {
                ctx.metrics.nodes_culled += 1;
                canvas.restore();
                return;
            }
        }
    }

    let outer_camera_bounds = if ctx.metrics.camera_recording {
        super::camera_cache::begin_camera_node(
            canvas,
            node,
            expanded_subtree_bounds_cancellable(
                node,
                expanded,
                children,
                cached_bounds,
                ctx.should_cancel,
            ),
            ctx.metrics,
        )
    } else {
        None
    };

    if ctx.metrics.camera_recording
        && matches!(&node.data, NodeData::Text(text)
            if !text.content.is_empty() && super::walk::text_visual_bounds(text).is_none())
    {
        ctx.metrics.camera_unsupported = true;
    }

    if ctx.metrics.camera_recording
        && node
            .blurs
            .iter()
            .any(|blur| blur.kind == super::BlurKind::Background && blur.radius != 0.0)
    {
        // A recording has no real backdrop device; a no-op readback is not proof
        // that replaying a retained raster would preserve this dependency.
        ctx.metrics.camera_unsupported = true;
    }

    // Background blur on a transient node — same treatment as the live walk:
    // sample+blur+blit the backdrop inside the silhouette BEFORE the effects
    // layer (so it reads the real backdrop, not this node's own layer). No scene
    // id, so a background-only group has no fallback silhouette and won't frost.
    let group_silhouette = if node
        .blurs
        .iter()
        .any(|blur| blur.kind == super::BlurKind::Background && blur.radius > 0.0)
    {
        node.data
            .as_group()
            .filter(|group| {
                group.clip_size.is_none()
                    && group.local_size.is_none()
                    && (group.background.is_some()
                        || !group.background_fills.is_empty()
                        || !group.strokes.is_empty())
            })
            .and_then(|group| {
                expanded_subtree_bounds_cancellable(
                    node,
                    expanded,
                    children,
                    cached_bounds,
                    ctx.should_cancel,
                )
                .map(|bounds| {
                    super::rounded_rect_path(
                        super::bounds_to_f32(&bounds),
                        group.corner_radius,
                        group.corner_radii,
                        group.corner_smoothing,
                    )
                })
            })
    } else {
        None
    };
    if ctx.supports_offscreen_layers {
        if apply_background_blur(
            canvas,
            node,
            None,
            ctx.scene,
            ctx.effective_scale_axes,
            boolean_silhouette.as_ref().or(group_silhouette.as_ref()),
        ) == super::effects::BackgroundBlurOutcome::Failed
        {
            ctx.metrics.effect_failed = true;
        }
    }

    // Same opacity fold as the live walk: a single-draw leaf vector applies its
    // opacity as paint alpha instead of a layer (pixel-identical), so an icon
    // component's translucent leaves cost no layer inside its instances either.
    let has_children = children.get(&node.id).is_some_and(|c| !c.is_empty());
    // Zoom-effective effects (see `VisibleEffects`): same fold/layer gating as
    // the live walk, so a sub-pixel blur or shadow inside a component neither
    // forces a layer nor blocks the fold in its instances.
    let visible = visible_effects(node, ctx.effective_scale_axes);
    let folded_opacity = opacity < 1.0 && opacity_folds_into_paint(node, has_children, visible);
    let needs_layer = !folded_opacity
        && ctx.supports_offscreen_layers
        && (opacity < 1.0
            || node.blend_mode != BlendMode::Normal
            || visible.drop_shadow
            || visible.layer_blur
            || node.flags.contains(NodeFlags::ISOLATED_BLEND));
    let used_layer = needs_layer
        && begin_effects_layer(
            canvas,
            opacity,
            node.blend_mode,
            &node.effects,
            &node.blurs,
            node.flags.contains(NodeFlags::ISOLATED_BLEND),
            ctx.effective_scale_axes,
            // Transient clone: no scene entry, so the box comes from the node's
            // own geometry (clip box / path bounds / local_size); an unclipped
            // transient group falls back to the union of its transient children so
            // its layer is still bounded rather than viewport-sized.
            boolean_shape
                .as_ref()
                .and_then(|shape| shape.bounds(node))
                .or_else(|| {
                    expanded_subtree_bounds_cancellable(
                        node,
                        expanded,
                        children,
                        cached_bounds,
                        ctx.should_cancel,
                    )
                }),
        );
    if used_layer {
        ctx.metrics.effect_layers += 1;
    }
    let previous_filtered_ancestor = ctx.filtered_ancestor;
    ctx.filtered_ancestor |= used_layer && (visible.drop_shadow || visible.layer_blur);

    // Transient nodes have no scene id, so a background-only unclipped group
    // paints nothing (no bounds to fall back to) — correct for a transient
    // subtree. Their fresh clone ids also must not key the vector path cache
    // (`path_cache_id: None`): expansion clones rebuild paths directly.
    let content_state = if folded_opacity {
        ctx.metrics.opacity_folds += 1;
        let outer_alpha = std::mem::replace(&mut ctx.paint_alpha, opacity);
        let state = match &boolean_shape {
            Some(shape) => shape.paint(canvas, node, ctx),
            None => paint_node_content(canvas, node, None, None, ctx),
        };
        ctx.paint_alpha = outer_alpha;
        state
    } else {
        match &boolean_shape {
            Some(shape) => shape.paint(canvas, node, ctx),
            None => paint_node_content(canvas, node, None, None, ctx),
        }
    };

    if let Some(inner) = node.data.as_instance() {
        // Nested instance: recurse one expansion level (a swap-override on it
        // was already applied during the parent's expansion). The clone's fresh
        // id keys its own memo entry — distinct from the outer instance and from
        // other clones of the same master.
        render_instance(canvas, node.id, inner, ctx);
    } else if node.is_boolean() {
        // Operand paint is consumed by the fold, never drawn independently.
    } else if let Some(child_idx) = children.get(&node.id) {
        // Same mask semantics as the live-scene walk: a transient child flagged
        // `is_mask` masks its following siblings — so masks authored inside a
        // component render correctly in its instances. The mask flag/type live on
        // the clone (`expanded[i].node`), which is what `mask_of` reads.
        let child_idx = child_idx.clone();
        paint_child_sequence(
            canvas,
            ctx,
            &child_idx,
            |_ctx, &i| {
                let n = &expanded[i].node;
                (n.is_mask && !n.flags.contains(NodeFlags::HIDDEN)).then_some(n.mask_type)
            },
            // The transient source needs the same composed filter extent as
            // the live walk to keep masked blur tails inside its layer.
            |ctx, &i| {
                let child = &expanded.get(i)?.node;
                let scratch = resolve_overlay(ctx, child.id, child);
                let child = scratch.as_deref().unwrap_or(child);
                // Raw expansion boxes omit descendant effect and glyph overflow.
                // An unbounded hint preserves the current clip when no finite
                // masked source extent has been proved.
                if child
                    .data
                    .as_group()
                    .is_some_and(|group| !group_clips_children(child, group))
                    || child.is_boolean()
                {
                    return None;
                }
                let local = match &child.data {
                    NodeData::Vector(vector) => {
                        crate::bounds::vector_visual_bounds(vector, child.flags)
                    }
                    NodeData::Text(text) => super::walk::text_visual_bounds(text),
                    _ => child.data.local_bounds(),
                }?;
                super::walk::visual_local_bounds(child, local, ctx.effective_scale_axes)?
                    .try_transformed(&child.transform)
            },
            |canvas, ctx, &i| {
                render_expanded(
                    canvas,
                    &expanded[i].node,
                    false,
                    expanded,
                    children,
                    cached_bounds,
                    ctx,
                )
            },
        );
    }

    if content_state.restore_child_clip {
        canvas.restore();
    }
    paint_node_foreground(canvas, node, None, ctx);

    // Inner shadows composite over the node's flattened content (fill, border,
    // children), mirroring the live-scene walk's ordering — with no scene id, so
    // a background-only group has no fallback box (matches `paint_node_content`).
    if ctx.supports_offscreen_layers {
        draw_inner_shadows(canvas, node, None, ctx, boolean_silhouette.as_ref());
    }

    if used_layer {
        canvas.restore();
    }
    ctx.filtered_ancestor = previous_filtered_ancestor;
    super::camera_cache::end_camera_node(
        canvas,
        node,
        outer_camera_bounds,
        ctx.effective_scale_axes,
        ctx.metrics,
    );
    canvas.restore();
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{AutoLayout, Color, GroupNode, Transform2D, VectorNode};

    #[test]
    fn expanded_unclipped_group_background_blur_frosts_its_content_box() {
        use super::super::{RasterRenderer, RenderInputs};
        use fanta_doc::{Blur, ComponentDef, ComponentId, Doc, Fill, Viewport};
        fn insert(scene: &mut Scene, parent: Option<NodeId>, data: NodeData) -> NodeId {
            let mut node = CanvasNode::new(data);
            node.parent = parent;
            node.index = scene.next_child_index(parent);
            scene.insert(node).expect("fixture node")
        }
        fn render(blurred: bool) -> Vec<u8> {
            let mut doc = Doc::new();
            let master = insert(
                &mut doc.scene,
                None,
                NodeData::Group(GroupNode {
                    clip_size: Some([128.0, 128.0]),
                    background: Some(Fill::solid(Color::WHITE)),
                    ..Default::default()
                }),
            );
            for row in 0..32 {
                for column in 0..32 {
                    if (row + column) % 2 == 0 {
                        insert(
                            &mut doc.scene,
                            Some(master),
                            NodeData::Vector(VectorNode::rect_solid(
                                f64::from(column) * 4.0,
                                f64::from(row) * 4.0,
                                4.0,
                                4.0,
                                Color::BLACK,
                            )),
                        );
                    }
                }
            }
            let panel = insert(
                &mut doc.scene,
                Some(master),
                NodeData::Group(GroupNode {
                    background: Some(Fill::solid(Color::rgba(255, 255, 255, 0))),
                    ..Default::default()
                }),
            );
            if blurred {
                doc.scene
                    .get_mut(panel)
                    .expect("panel")
                    .blurs
                    .push(Blur::background(8.0));
            }
            insert(
                &mut doc.scene,
                Some(panel),
                NodeData::Vector(VectorNode::rect_solid(
                    32.0,
                    32.0,
                    64.0,
                    64.0,
                    Color::TRANSPARENT,
                )),
            );
            let component = ComponentId::new();
            doc.components
                .defs
                .insert(component, ComponentDef::new(component, master, "Frost"));
            let placed = insert(
                &mut doc.scene,
                None,
                NodeData::Instance(InstanceNode {
                    component,
                    overrides: Vec::new(),
                    prop_values: Default::default(),
                    derived: Vec::new(),
                    local_size: [128.0, 128.0],
                }),
            );
            let mut renderer = RasterRenderer::new(128, 128).expect("renderer");
            let metrics = renderer.render_page_with(
                &doc.scene,
                &Viewport {
                    center: [64.0, 64.0],
                    zoom: 1.0,
                },
                Some(placed),
                &RenderInputs::for_doc(&doc),
            );
            assert!(!metrics.effect_failed && !metrics.incomplete_artwork);
            renderer.copy_rgba()
        }
        let plain = render(false);
        let frosted = render(true);
        let variance = |pixels: &[u8]| {
            let mut sum = 0.0;
            let mut squares = 0.0;
            for y in 48..80 {
                for x in 48..80 {
                    let value = f64::from(pixels[(y * 128 + x) * 4]);
                    sum += value;
                    squares += value * value;
                }
            }
            squares / 1024.0 - (sum / 1024.0).powi(2)
        };
        assert!(
            variance(&plain) > 1000.0,
            "the unblurred checker must remain sharp"
        );
        assert!(
            variance(&frosted) < variance(&plain) * 0.6,
            "a component's un-sized group must apply its authored backdrop blur"
        );
        for y in 0..24 {
            for x in 0..128 {
                let offset = (y * 128 + x) * 4;
                assert_eq!(
                    &frosted[offset..offset + 4],
                    &plain[offset..offset + 4],
                    "frost must remain inside the resolved content box"
                );
            }
        }
    }

    #[test]
    fn solving_scene_layout_keeps_the_scene_patchable() {
        let mut scene = Scene::new();
        let frame = scene
            .insert(CanvasNode::new(NodeData::Group(GroupNode {
                auto_layout: Some(AutoLayout::default()),
                local_size: Some([100.0, 40.0]),
                ..GroupNode::default()
            })))
            .unwrap();
        let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            30.0,
            20.0,
            Color::WHITE,
        )));
        child.parent = Some(frame);
        child.transform = Transform2D::translation(50.0, 50.0);
        let child = scene.insert(child).unwrap();

        let before = scene.revision();
        solve_scene_layout(&mut scene, frame);
        let delta = scene
            .changes_since(before)
            .expect("a layout solve must log its writes precisely, never as Unknown");
        assert!(
            delta.nodes.contains(&child) || delta.transforms.contains(&child),
            "the solver's write to the child is in the delta: {delta:?}"
        );
    }

    #[test]
    fn mode_pin_hash_follows_the_ancestor_chain_pins() {
        let mut scene = Scene::new();
        let outer = scene
            .insert(CanvasNode::new(NodeData::Group(GroupNode::default())))
            .unwrap();
        let mut inner = CanvasNode::new(NodeData::Group(GroupNode::default()));
        inner.parent = Some(outer);
        let inner = scene.insert(inner).unwrap();
        let mut leaf = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            1.0,
            1.0,
            Color::WHITE,
        )));
        leaf.parent = Some(inner);
        let leaf = scene.insert(leaf).unwrap();
        let sibling = scene
            .insert(CanvasNode::new(NodeData::Group(GroupNode::default())))
            .unwrap();

        let unpinned = hash_mode_pins(&scene, leaf);
        assert_eq!(
            unpinned,
            hash_mode_pins(&scene, sibling),
            "no pins anywhere"
        );

        let collection = fanta_doc::VariableCollectionId::new();
        let dark = fanta_doc::ModeId::new();
        if let Some(NodeData::Group(group)) = scene.get_mut(outer).map(|node| &mut node.data) {
            group.explicit_modes.insert(collection, dark);
        }
        let pinned_above = hash_mode_pins(&scene, leaf);
        assert_ne!(
            unpinned, pinned_above,
            "a pin on an ancestor changes the hash"
        );
        assert_eq!(
            unpinned,
            hash_mode_pins(&scene, sibling),
            "a pin elsewhere leaves unrelated chains alone"
        );
        assert_eq!(
            pinned_above,
            hash_mode_pins(&scene, leaf),
            "stable across calls"
        );

        let light = fanta_doc::ModeId::new();
        if let Some(NodeData::Group(group)) = scene.get_mut(inner).map(|node| &mut node.data) {
            group.explicit_modes.insert(collection, light);
        }
        assert_ne!(
            pinned_above,
            hash_mode_pins(&scene, leaf),
            "a nearer pin changes the hash"
        );
    }

    #[test]
    fn expanded_non_clipping_group_bounds_union_box_and_overflow_children() {
        let root = CanvasNode::new(NodeData::Group(GroupNode {
            local_size: Some([20.0, 10.0]),
            ..Default::default()
        }));
        let root_id = root.id;
        let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            5.0,
            6.0,
            Color::WHITE,
        )));
        child.parent = Some(root_id);
        child.transform = Transform2D::translation(30.0, -4.0);
        let expanded = vec![
            ExpandedNode {
                node: root,
                def_path: Default::default(),
            },
            ExpandedNode {
                node: child,
                def_path: Default::default(),
            },
        ];
        let children = build_child_index(&expanded);
        assert_eq!(
            expanded_subtree_bounds(&expanded[0].node, &expanded, &children, None),
            Some(Bounds::from_xywh(0.0, -4.0, 35.0, 14.0))
        );
        let prepared = PreparedInstance::new(expanded);
        assert_eq!(
            prepared
                .bounds
                .as_ref()
                .and_then(|bounds| bounds.get(&root_id)),
            Some(&Some(Bounds::from_xywh(0.0, -4.0, 35.0, 14.0)))
        );
    }
}
