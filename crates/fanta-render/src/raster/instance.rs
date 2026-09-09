//! Component-instance rendering: expand a master subtree (memoized), run the
//! auto-layout solver over it, and walk the transient clone tree
//! ([`render_expanded`]) mirroring the live-scene walk. Also hosts the public
//! [`solve_scene_layout`] entry point for scene-direct auto-layout frames.
use super::effects::group_clips_children;
use super::{
    Arc, BlendMode, Bounds, Canvas, CanvasNode, ExpandedNode, HashMap, InstanceCacheKey,
    InstanceNode, NodeData, NodeFlags, NodeId, RenderCtx, Scene, apply_background_blur,
    begin_effects_layer, draw_inner_shadows, draw_unresolved_outline, effects_layer_bounds,
    opacity_folds_into_paint, paint_child_sequence, paint_node_content, paint_node_foreground,
    resolve_overlay, shadow_expanded_local_bounds, to_sk_matrix, visible_effects,
    with_shaped_layout,
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
    let mode_anchor = if ctx.scene.get(instance_id).is_some() {
        instance_id
    } else {
        ctx.instance_mode_anchor.unwrap_or(instance_id)
    };
    let expanded = expand_instance_memoized(ctx, instance_id, inst, mode_anchor);
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
        let previous_mode_anchor = ctx.instance_mode_anchor.replace(mode_anchor);
        render_expanded(canvas, &root.node, true, &expanded, &children, ctx);
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
) -> Arc<Vec<ExpandedNode>> {
    let expansion_context = fanta_doc::InstanceExpansionContext::new(
        ctx.inputs.variables,
        ctx.inputs.active_modes,
        mode_anchor,
    );
    // The RESOLVED master's rev (a variant instance's `component` is the *set*
    // id, which is not itself a def — `def(set)` is `None` and would wrongly
    // report rev 0, never invalidating on a member edit). `> 0` also means the
    // master was edited, which drives the live-solve decision below.
    let rev = fanta_doc::resolved_component_rev_with_context(
        ctx.scene,
        ctx.inputs.components,
        inst,
        &expansion_context,
    );
    // Only a live-scene instance has a stamp to memoize its override hash on;
    // a nested transient clone (fresh id, not in the scene) hashes directly.
    let stamp = ctx
        .scene
        .contains(instance_id)
        .then(|| ctx.scene.node_stamp(instance_id));
    let key = InstanceCacheKey {
        instance: instance_id,
        rev,
        override_hash: ctx.instance_cache.override_hash(instance_id, stamp, inst),
        mode_generation: ctx.inputs.mode_generation,
        mode_pins: hash_mode_pins(ctx.scene, mode_anchor),
    };
    if let Some(cached) = ctx.instance_cache.lookup(&key) {
        return cached;
    }
    let mut expanded = fanta_doc::expand_instance_with_context(
        ctx.scene,
        ctx.inputs.components,
        inst,
        &expansion_context,
    );
    // Figma's `derivedSymbolData` is already a resolved per-instance subtree, so
    // re-solving those clones moves icons/carets/text away from the baked
    // geometry — only masters without derived data need our layout pass. An
    // EDITED master keeps its baked geometry (only the fill falls through to the
    // master), so we must NOT re-solve it — doing so would resize its vectors to
    // the master box and distort them.
    if inst.derived.is_empty() {
        fanta_doc::solve_expanded(&mut expanded, &mut measure_text_node);
    }
    let expanded = Arc::new(expanded);
    ctx.instance_cache.insert(key, Arc::clone(&expanded));
    expanded
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
        NodeData::Group(group) if !group_clips_children(node, group) => {
            let mut acc = group
                .clip_size
                .or(group.local_size)
                .map(|[width, height]| Bounds::from_xywh(0.0, 0.0, width, height));
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
    let opacity = node.opacity.get();
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
    if ctx.supports_offscreen_layers {
        apply_background_blur(canvas, node, None, ctx.scene, ctx.effective_scale);
    }

    // Same opacity fold as the live walk: a single-draw leaf vector applies its
    // opacity as paint alpha instead of a layer (pixel-identical), so an icon
    // component's translucent leaves cost no layer inside its instances either.
    let has_children = children.get(&node.id).is_some_and(|c| !c.is_empty());
    // Zoom-effective effects (see `VisibleEffects`): same fold/layer gating as
    // the live walk, so a sub-pixel blur or shadow inside a component neither
    // forces a layer nor blocks the fold in its instances.
    let visible = visible_effects(node, ctx.effective_scale);
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
            ctx.effective_scale,
            // Transient clone: no scene entry, so the box comes from the node's
            // own geometry (clip box / path bounds / local_size); an unclipped
            // transient group falls back to the union of its transient children so
            // its layer is still bounded rather than viewport-sized.
            expanded_subtree_bounds(node, expanded, children, ctx.scene),
        );
    if used_layer {
        ctx.metrics.effect_layers += 1;
    }

    // Transient nodes have no scene id, so a background-only unclipped group
    // paints nothing (no bounds to fall back to) — correct for a transient
    // subtree. Their fresh clone ids also must not key the vector path cache
    // (`path_cache_id: None`): expansion clones rebuild paths directly.
    let content_state = if folded_opacity {
        ctx.metrics.opacity_folds += 1;
        let outer_alpha = std::mem::replace(&mut ctx.paint_alpha, opacity);
        let state = paint_node_content(canvas, node, None, None, ctx);
        ctx.paint_alpha = outer_alpha;
        state
    } else {
        paint_node_content(canvas, node, None, None, ctx)
    };

    if let Some(inner) = node.data.as_instance() {
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
    if ctx.supports_offscreen_layers {
        draw_inner_shadows(canvas, node, None, ctx);
    }

    if used_layer {
        canvas.restore();
    }
    canvas.restore();
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{AutoLayout, Color, GroupNode, Transform2D, VectorNode};

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
        let scene = Scene::new();

        assert_eq!(
            expanded_subtree_bounds(&expanded[0].node, &expanded, &children, &scene),
            Some(Bounds::from_xywh(0.0, -4.0, 35.0, 14.0))
        );
    }
}
