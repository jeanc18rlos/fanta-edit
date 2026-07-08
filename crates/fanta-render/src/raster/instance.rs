//! Component-instance rendering: expand a master subtree (memoized), run the
//! auto-layout solver over it, and walk the transient clone tree
//! ([`render_expanded`]) mirroring the live-scene walk. Also hosts the public
//! [`solve_scene_layout`] entry point for scene-direct auto-layout frames.
use super::{
    Arc, Bounds, Canvas, CanvasNode, ExpandedNode, HashMap, InstanceCacheKey, InstanceNode,
    NodeData, NodeFlags, NodeId, RenderCtx, Scene, apply_background_blur, begin_effects_layer,
    draw_inner_shadows, draw_unresolved_outline, effects_layer_bounds, expand_instance,
    hash_overrides, paint_child_sequence, paint_node_content, paint_node_foreground,
    resolve_overlay, shadow_expanded_local_bounds, to_sk_matrix, with_shaped_layout,
};

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
    let expanded = expand_instance_memoized(ctx, instance_id, inst);
    if expanded.is_empty() {
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
    let children = build_child_index(&expanded);
    if let Some(root) = expanded.iter().find(|e| e.def_path.is_empty()) {
        render_expanded(canvas, &root.node, true, &expanded, &children, ctx);
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
) -> Arc<Vec<ExpandedNode>> {
    let rev = ctx
        .inputs
        .components
        .def(inst.component)
        .map(|d| d.rev)
        .unwrap_or(0);
    let key = InstanceCacheKey {
        instance: instance_id,
        rev,
        override_hash: hash_overrides(inst),
        mode_generation: ctx.inputs.mode_generation,
    };
    if let Some(cached) = ctx.instance_cache.entries.get(&key) {
        return Arc::clone(cached);
    }
    let mut expanded = expand_instance(ctx.scene, ctx.inputs.components, inst);
    // Figma's `derivedSymbolData` is already a resolved per-instance subtree.
    // Re-solving those clones moves icons/carets/text away from the baked
    // geometry; only masters without derived data need our layout pass.
    if inst.derived.is_empty() {
        fanta_doc::solve_expanded(&mut expanded, &mut measure_text_node);
    }
    let expanded = Arc::new(expanded);
    ctx.instance_cache
        .entries
        .insert(key, Arc::clone(&expanded));
    expanded
}

/// Measure a text node's glyph box with the real `fanta-text` shaper, for the
/// auto-layout solver's auto-width case, returning its `(width, height)`.
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
/// Mutates node transforms/sizes, then invalidates the scene's world-bounds
/// cache so culling, hit-testing, and `world_bounds` observe the new geometry.
///
/// Call ONCE after import (and after edits that change auto-layout inputs) — it
/// is `O(n)` over the page subtree, not a per-frame cost.
pub fn solve_scene_layout(scene: &mut Scene, page: NodeId) {
    fanta_doc::solve_auto_layout(scene, page, &mut measure_text_node);
    scene.invalidate_world_cache();
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
    scene: &Scene,
) -> Option<Bounds> {
    match &node.data {
        NodeData::Group(g) if g.clip_size.is_none() => {
            let mut acc: Option<Bounds> = None;
            for &i in children.get(&node.id).into_iter().flatten() {
                let Some(entry) = expanded.get(i) else {
                    continue;
                };
                let child = &entry.node;
                let Some(b) = expanded_subtree_bounds(child, expanded, children, scene) else {
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
        _ => effects_layer_bounds(node, None, scene),
    }
}

/// Map each expanded clone's parent id → its child clone indices, so the
/// transient subtree can be walked top-down without a scene. Built once per
/// instance render and indexed by clone `NodeId`.
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
    let opacity = node.opacity.clamp(0.0, 1.0);
    if opacity <= 0.0 {
        return;
    }

    canvas.save();
    // The root's own transform is suppressed (see fn docs); descendants apply
    // theirs as usual.
    if !is_root {
        canvas.concat(&to_sk_matrix(&node.transform));
    }

    // Background blur on a transient node — same treatment as the live walk:
    // sample+blur+blit the backdrop inside the silhouette BEFORE the effects
    // layer (so it reads the real backdrop, not this node's own layer). No scene
    // id, so a background-only group has no fallback silhouette and won't frost.
    apply_background_blur(canvas, node, None, ctx.scene, ctx.effective_scale);

    let used_layer = begin_effects_layer(
        canvas,
        opacity,
        node.blend_mode,
        &node.effects,
        &node.blurs,
        node.flags.contains(NodeFlags::ISOLATED_BLEND),
        ctx.effective_scale,
        // Transient clone: no scene entry, so the box comes from the node's
        // own geometry (clip box / path bounds / local_size); an unclipped
        // transient group falls back to the union of its transient children so
        // its layer is still bounded rather than viewport-sized.
        effects_layer_bounds(node, None, ctx.scene)
            .or_else(|| expanded_subtree_bounds(node, expanded, children, ctx.scene)),
    );

    // Transient nodes have no scene id, so a background-only unclipped group
    // paints nothing (no bounds to fall back to) — correct for a transient
    // subtree.
    let content_state = paint_node_content(canvas, node, None, ctx);

    if let NodeData::Instance(inner) = &node.data {
        // Nested instance: recurse one expansion level (a swap-override on it
        // was already applied during the parent's expansion). The clone's fresh
        // id keys its own memo entry — distinct from the outer instance and from
        // other clones of the same master.
        render_instance(canvas, node.id, inner, ctx);
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
            // A transient child subtree's painted extent in this node's local
            // space, mirroring the live walk's bounds closure: subtree box,
            // shadow-expanded, mapped through the child's transform.
            |ctx, &i| {
                let child = &expanded.get(i)?.node;
                let local = expanded_subtree_bounds(child, expanded, children, ctx.scene)?;
                shadow_expanded_local_bounds(
                    local,
                    &child.effects,
                    &child.blurs,
                    ctx.effective_scale,
                )
                .try_transformed(&child.transform)
            },
            |canvas, ctx, &i| {
                render_expanded(canvas, &expanded[i].node, false, expanded, children, ctx)
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
    draw_inner_shadows(canvas, node, None, ctx);

    if used_layer {
        canvas.restore();
    }
    canvas.restore();
}
