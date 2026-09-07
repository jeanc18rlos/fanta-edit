//! Instance expansion: deep-clone a component master's subtree with fresh ids,
//! apply the instance's sparse [`Override`]s and Figma's baked
//! [`DerivedOverride`](crate::node::DerivedOverride) data, and record each
//! clone's def-local path back to the master.
//!
//! [`expand_instance`] is one level deep — nested instances come back *as*
//! instances (with any swap-override applied) so the renderer recurses by
//! calling it again.

use crate::binding::BoundProp;
use crate::component::{ComponentDef, ComponentLibrary, ComponentPropKind};
use crate::id::{ModeId, NodeId, VariableCollectionId};
use crate::node::{
    CanvasNode, InstanceNode, NodeData, NodeFlags, Override, OverridePath, OverrideValue,
};
use crate::path::PathData;
use crate::scene::Scene;
use crate::value::{ResolvedVarValue, VarValue};
use crate::variables::VariableRegistry;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

use super::variables::resolve_bound_value;

/// One node produced by [`expand_instance`]: a fresh-id clone of a master
/// descendant, plus the def-local path (original master ids, root-first,
/// excluding the master root) identifying which master node it came from.
///
/// `def_path == []` is the master root. The path is what lets the editor map a
/// clicked transient descendant back to an [`Override`]'s `target_path`.
///
/// [`Override`]: crate::node::Override
#[derive(Debug, Clone, PartialEq)]
pub struct ExpandedNode {
    /// The cloned node with a fresh id; `parent` points at another fresh id in
    /// the same batch, or `None` for the expansion root.
    pub node: CanvasNode,
    /// Def-local path of *original* master ids (root-first, root excluded).
    pub def_path: OverridePath,
}

/// Variable state used while expanding a component instance.
///
/// Component-property values live on the placed instance, so aliases resolve at
/// that instance's position in the scene. `mode_anchor` supplies that position
/// for the nearest-ancestor frame-mode lookup. A renderer expanding a nested
/// transient instance should keep using the outer placed instance as the anchor,
/// because transient clone ids do not exist in `Scene`.
#[derive(Debug, Clone, Copy)]
pub struct InstanceExpansionContext<'a> {
    variables: &'a VariableRegistry,
    active_modes: &'a BTreeMap<VariableCollectionId, ModeId>,
    mode_anchor: NodeId,
}

/// Exact component definition selected for an instance.
///
/// `component_ref` is the effective reference carried by the [`InstanceNode`]
/// being resolved and may name a component set. For a nested transient instance
/// this can already include an outer instance's swap override.
/// `resolved_component` is always the concrete member definition whose master
/// subtree is expanded. This is the authoritative metadata for render traces,
/// dependency walkers, and memo keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedComponentDefRef {
    pub component_ref: crate::id::ComponentId,
    pub resolved_component: crate::id::ComponentId,
    pub resolved_root: NodeId,
    pub rev: u64,
}

impl<'a> InstanceExpansionContext<'a> {
    pub fn new(
        variables: &'a VariableRegistry,
        active_modes: &'a BTreeMap<VariableCollectionId, ModeId>,
        mode_anchor: NodeId,
    ) -> Self {
        Self {
            variables,
            active_modes,
            mode_anchor,
        }
    }

    pub fn mode_anchor(&self) -> NodeId {
        self.mode_anchor
    }
}

/// Expand one instance against its component master: deep-clone the master
/// subtree with fresh ids, rewire parents within the clone, apply the instance's
/// overrides (matched by def-local path), and tag each clone with its def path.
///
/// One level only: a clone that is itself an [`NodeData::Instance`] is returned
/// as an instance (with any swap-override already applied), so the renderer
/// recurses by calling `expand_instance` on it. A dangling component id (master
/// deleted) yields an empty `Vec` — the caller renders nothing, matching the
/// model's dangling-ref tolerance.
///
/// A component property with a descendant binding applies the instance value or
/// definition default before explicit overrides, so an explicit per-instance
/// override still wins. Concrete values work through this compatibility API;
/// aliases require [`expand_instance_with_context`] because resolving them needs
/// the document registry and the placed instance's effective modes. Variant
/// properties also participate when `instance.component` names a
/// [`ComponentSet`].
///
/// [`ComponentSet`]: crate::component::ComponentSet
pub fn expand_instance(
    scene: &Scene,
    components: &ComponentLibrary,
    instance: &InstanceNode,
) -> Vec<ExpandedNode> {
    expand_instance_inner(scene, components, instance, None)
}

/// Expand an instance while resolving variable aliases used by its component
/// properties. This is the variable-aware counterpart to [`expand_instance`];
/// the original function remains available for callers that only have concrete
/// component-property values or intentionally lack document variable state.
pub fn expand_instance_with_context(
    scene: &Scene,
    components: &ComponentLibrary,
    instance: &InstanceNode,
    context: &InstanceExpansionContext<'_>,
) -> Vec<ExpandedNode> {
    expand_instance_inner(scene, components, instance, Some(context))
}

fn expand_instance_inner(
    scene: &Scene,
    components: &ComponentLibrary,
    instance: &InstanceNode,
    context: Option<&InstanceExpansionContext<'_>>,
) -> Vec<ExpandedNode> {
    let Some(def) = resolve_instance_def(scene, components, instance, context) else {
        return Vec::new();
    };
    let root = def.root;
    if scene.get(root).is_none() {
        return Vec::new();
    }

    // `descendants_of` yields the root first, then its subtree.
    let orig_ids: Vec<NodeId> = scene.descendants_of(root).collect();

    // Fresh id per original, so the transient subtree never collides with the
    // live scene and two instances of the same master stay independent.
    let mut remap: HashMap<NodeId, NodeId> = HashMap::with_capacity(orig_ids.len());
    for &o in &orig_ids {
        remap.insert(o, NodeId::new());
    }

    let mut out: Vec<ExpandedNode> = Vec::with_capacity(orig_ids.len());
    for &o in &orig_ids {
        let Some(orig) = scene.get(o) else { continue };
        let mut clone = orig.clone();
        clone.id = remap[&o];
        clone.parent = if o == root {
            None
        } else {
            orig.parent.and_then(|p| remap.get(&p).copied())
        };
        out.push(ExpandedNode {
            node: clone,
            def_path: def_local_path(scene, root, o),
        });
    }

    pin_expansion_root_box(&mut out, instance);
    apply_prop_bindings(scene, &mut out, def, instance, context);
    apply_sparse_overrides(&mut out, instance);

    // Apply Figma's baked per-descendant resolved data (`derivedSymbolData`),
    // matched by the same def-local path. This OVERWRITES the cloned master's
    // transform/size/geometry/text with the values Figma resolved *for this
    // instance*, so a dark-theme instance renders dark (correct geometry, sizes,
    // positions, baked text) instead of the light master's. It runs after the
    // sparse overrides because the baked data is Figma's authoritative final
    // layout — a sparse text override still sets the *content* string, while the
    // derived entry sets the resolved *layout* of that same run. Only the fields
    // an entry populates are written; the rest stay at the master value. Deeper
    // paths route onto the nested instance's `derived` exactly like overrides.
    //
    // Once the user edits the master (`rev != 0`, bumped on every master-subtree
    // edit and omitted from JSON when zero), the baked PAINT is stale: a master
    // fill change must reach the instance. But the baked GEOMETRY (transform,
    // size, resolved path, stroke weight) is the instance's correct per-placement
    // layout — dropping it would resize the vector to the master's box and
    // distort it. So an edited master keeps the baked geometry and only lets the
    // fill fall through to the (edited) master value. `apply_derived` takes the
    // `edited` flag and skips exactly the fill write.
    let edited = def.rev != 0;
    for d in &instance.derived {
        // A sparse override is the user's explicit per-instance edit; the baked
        // derived entry is Figma's import-time snapshot of the SAME channels.
        // The explicit edit must win for exactly the channels it claims (text
        // content, first-fill color) or an edit to instance text is silently
        // reverted on every re-expansion — while the derived entry keeps
        // supplying the resolved layout (transform, size, font metrics).
        let claims = OverrideClaims {
            content: has_override(&instance.overrides, &d.path, &BoundProp::TextContent),
            fill: has_override(
                &instance.overrides,
                &d.path,
                &BoundProp::FillColor { index: 0 },
            ),
        };
        if !apply_at_path(&mut out, &d.path, |node| {
            apply_derived(node, d, edited, claims)
        }) {
            route_nested(&mut out, &d.path, |remainder, inst| {
                let mut nested = d.clone();
                nested.path = remainder.iter().copied().collect();
                inst.derived.push(nested);
            });
        }
    }
    out
}

/// Which channels of a derived entry an explicit [`Override`] on the same
/// def-path has claimed — those writes are skipped so the edit survives.
#[derive(Clone, Copy, Default)]
struct OverrideClaims {
    content: bool,
    fill: bool,
}

fn has_override(overrides: &[Override], path: &OverridePath, prop: &BoundProp) -> bool {
    overrides
        .iter()
        .any(|o| o.target_path == *path && o.target_prop == *prop)
}

/// The revision of the [`ComponentDef`] an `instance` resolves to (its master,
/// or the selected member of a variant set), or `0` when the master is missing.
/// `0` means "never edited since import"; any higher value means the master has
/// been edited, so callers should derive the instance live rather than trust the
/// baked `derivedSymbolData`. See [`expand_instance`].
pub fn resolved_component_rev(components: &ComponentLibrary, instance: &InstanceNode) -> u64 {
    resolved_component(components, instance).map_or(0, |resolved| resolved.rev)
}

/// Resolve an instance to one concrete component definition without variable
/// context. Prefer [`resolved_component_with_context`] when variant properties
/// may contain aliases.
pub fn resolved_component(
    components: &ComponentLibrary,
    instance: &InstanceNode,
) -> Option<ResolvedComponentDefRef> {
    let def = resolve_instance_def_without_context(components, instance)?;
    Some(ResolvedComponentDefRef {
        component_ref: instance.component,
        resolved_component: def.id,
        resolved_root: def.root,
        rev: def.rev,
    })
}

/// Root node of the component definition this instance resolves to.
///
/// This is the renderer-agnostic dependency lookup used by headless exporters
/// and validators. It handles both direct component ids and component-set ids,
/// choosing the set's selected/default member with the same rules as
/// [`expand_instance`]. The returned root may still be absent from `scene`;
/// callers that need a drawable dependency should verify that separately.
pub fn resolved_component_root(
    components: &ComponentLibrary,
    instance: &InstanceNode,
) -> Option<NodeId> {
    resolved_component(components, instance).map(|resolved| resolved.resolved_root)
}

/// Variable-aware counterpart to [`resolved_component_rev`]. This must be used
/// with [`expand_instance_with_context`] so an alias-backed variant selection
/// chooses both the expansion and its renderer memo revision from the same
/// resolved component definition.
pub fn resolved_component_rev_with_context(
    scene: &Scene,
    components: &ComponentLibrary,
    instance: &InstanceNode,
    context: &InstanceExpansionContext<'_>,
) -> u64 {
    resolved_component_with_context(scene, components, instance, context)
        .map_or(0, |resolved| resolved.rev)
}

/// Resolve an instance to the concrete component definition selected after
/// applying variable-aware variant properties.
pub fn resolved_component_with_context(
    scene: &Scene,
    components: &ComponentLibrary,
    instance: &InstanceNode,
    context: &InstanceExpansionContext<'_>,
) -> Option<ResolvedComponentDefRef> {
    let def = resolve_instance_def(scene, components, instance, Some(context))?;
    Some(ResolvedComponentDefRef {
        component_ref: instance.component,
        resolved_component: def.id,
        resolved_root: def.root,
        rev: def.rev,
    })
}

/// Variable-aware counterpart to [`resolved_component_root`].
///
/// Variant properties backed by variable aliases resolve in `context`, so this
/// must be used by dependency walkers that want the same component-set member
/// the renderer will expand.
pub fn resolved_component_root_with_context(
    scene: &Scene,
    components: &ComponentLibrary,
    instance: &InstanceNode,
    context: &InstanceExpansionContext<'_>,
) -> Option<NodeId> {
    resolved_component_with_context(scene, components, instance, context)
        .map(|resolved| resolved.resolved_root)
}

/// Drop every instance sparse override whose fill/stroke merely RESTATES the
/// master's own value at that node. Figma bakes each instance's fully-resolved
/// paint state into its overrides, so these snapshots are no-ops at import — but
/// once the user edits the master they would pin the instance to the stale value
/// and stop the edit from ever reaching it. Removing them lets a master paint
/// edit propagate to unmodified instances while a genuine per-instance recolor
/// (a value that differs from the master) is kept.
///
/// Runs once at load over both `.fig` imports and Fanta-project loads, and is
/// idempotent. Bumps the scene revision only for instances it actually trims.
pub fn strip_redundant_instance_overrides(scene: &mut Scene, components: &ComponentLibrary) {
    if components.defs.is_empty() {
        return;
    }
    let all_ids: Vec<NodeId> = scene
        .roots()
        .to_vec()
        .into_iter()
        .flat_map(|root| scene.descendants_of(root).collect::<Vec<_>>())
        .collect();
    let mut trimmed: Vec<(NodeId, Vec<Override>)> = Vec::new();
    for id in all_ids {
        let Some(node) = scene.get(id) else { continue };
        let NodeData::Instance(instance) = &node.data else {
            continue;
        };
        if instance.overrides.is_empty() {
            continue;
        }
        let Some(def) = resolve_instance_def_without_context(components, instance) else {
            continue;
        };
        let master_root = def.root;
        let kept: Vec<Override> = instance
            .overrides
            .iter()
            .filter(|ov| {
                let master_node = ov.target_path.last().copied().unwrap_or(master_root);
                !override_restates_master(scene, master_node, &ov.value)
            })
            .cloned()
            .collect();
        if kept.len() != instance.overrides.len() {
            trimmed.push((id, kept));
        }
    }
    for (id, kept) in trimmed {
        if let Some(node) = scene.get_mut(id)
            && let NodeData::Instance(instance) = &mut node.data
        {
            instance.overrides = kept;
        }
    }
}

/// Backfill an SVG viewport ([`VectorNode::local_size`]) onto any vector that
/// lacks one, inferring its box from the path bounds. This is for documents saved
/// before viewports were recorded (older materialized projects) — and, crucially,
/// for docs in a MIXED state where some vectors have a viewport and others don't
/// (e.g. a project saved mid-migration): each vector is decided INDEPENDENTLY, so
/// a `None` vector is filled even when a sibling already carries a `local_size`.
///
/// Vectors that already have a viewport are left untouched (the importer's exact
/// Figma `size`, or a prior backfill). Only an origin-anchored, non-degenerate
/// path gets a box — one starting left of / above the local origin, or with a
/// zero-area bound (an axis-aligned `LINE`), can't have its authored box inferred
/// from geometry, so it stays unclipped rather than risk a misaligned clip.
///
/// Masters live in the scene (embedded or on the hidden Components page), so this
/// reaches instance sources too: an instance clones the master's `local_size`
/// when it expands.
pub fn backfill_vector_viewports(scene: &mut Scene) {
    let all_ids: Vec<NodeId> = scene
        .roots()
        .to_vec()
        .into_iter()
        .flat_map(|root| scene.descendants_of(root).collect::<Vec<_>>())
        .collect();
    let mut boxes: Vec<(NodeId, [f64; 2])> = Vec::new();
    for id in &all_ids {
        let Some(node) = scene.get(*id) else { continue };
        if let NodeData::Vector(v) = &node.data {
            if v.local_size.is_some() {
                continue; // already has a viewport — decide each vector on its own
            }
            let Some(b) = v.path.rough_bounds() else {
                continue;
            };
            if b.min_x < -0.5 || b.min_y < -0.5 || b.max_x <= 0.0 || b.max_y <= 0.0 {
                continue;
            }
            boxes.push((*id, [b.max_x, b.max_y]));
        }
    }
    for (id, size) in boxes {
        if let Some(node) = scene.get_mut(id)
            && let NodeData::Vector(v) = &mut node.data
        {
            v.local_size = Some(size);
        }
    }
}

/// Whether a fill/stroke override value equals the master node's own paint (so
/// the override is a redundant snapshot). Only fill/stroke override kinds can be
/// redundant this way; every other kind is kept.
fn override_restates_master(scene: &Scene, master_node: NodeId, value: &OverrideValue) -> bool {
    let Some(node) = scene.get(master_node) else {
        return false;
    };
    match value {
        OverrideValue::Fills { fills } => match &node.data {
            NodeData::Vector(v) => v.fills.as_slice() == fills.as_slice(),
            NodeData::Group(g) => match &g.background {
                Some(background) => fills.as_slice() == std::slice::from_ref(background),
                None => fills.is_empty(),
            },
            _ => false,
        },
        OverrideValue::Strokes { strokes } => match &node.data {
            NodeData::Vector(v) => v.strokes.as_slice() == strokes.as_slice(),
            NodeData::Group(g) => g.strokes.as_slice() == strokes.as_slice(),
            _ => false,
        },
        _ => false,
    }
}

/// mergeSymbolProps (op2 `mergeSymbolProps`): the expansion root is the placed
/// instance's box, not the component master's original box. Pin clipped roots to
/// `local_size` even when they have no background; otherwise icon components with
/// baked larger descendant geometry keep the master's stale clip and crop their
/// own vectors.
fn pin_expansion_root_box(out: &mut [ExpandedNode], instance: &InstanceNode) {
    if let Some(rooten) = out.iter_mut().find(|e| e.def_path.is_empty()) {
        if let NodeData::Group(g) = &mut rooten.node.data {
            if g.background.is_some() || g.clip_size.is_some() {
                g.clip_size = Some(instance.local_size);
            } else {
                g.local_size = Some(instance.local_size);
            }
        }
    }
}

/// Apply component PROPERTY bindings: for each non-variant prop, resolve the
/// instance's value (or the def's default) and write it into every descendant the
/// prop is bound to (`PropBindingTarget`). This runs BEFORE the explicit overrides
/// loop so a per-instance override still wins. Variant props carry no binding —
/// they're consumed by `resolve_instance_def` (member selection). Bindings to
/// direct master descendants (single-level paths) apply here; deeper paths into
/// nested instances are a follow-up (silently skipped, matching the model's
/// dangling-ref tolerance).
fn apply_prop_bindings(
    scene: &Scene,
    out: &mut [ExpandedNode],
    def: &ComponentDef,
    instance: &InstanceNode,
    context: Option<&InstanceExpansionContext<'_>>,
) {
    for prop in &def.props {
        if matches!(prop.kind, ComponentPropKind::Variant { .. }) || prop.bindings.is_empty() {
            continue;
        }
        let value = instance.prop_values.get(&prop.id).unwrap_or(&prop.default);
        let Some(resolved) = resolve_component_prop_value(Some(scene), value, context) else {
            continue; // unresolved alias — leave the master value in place
        };
        for binding in &prop.bindings {
            let bound = binding.prop;
            let rv = resolved.clone();
            apply_at_path(out, &binding.path, move |node| {
                bound.apply_resolved(node, rv);
            });
        }
    }
}

/// Apply the instance's sparse [`Override`]s, matched by def-local path against
/// the master ids. A single-level path (the common case) addresses a direct
/// master descendant and is applied here. A longer path crosses into a nested
/// instance's own master subtree (Figma's `guidPath` walks nested-instance
/// boundaries); we ROUTE the remainder onto the cloned nested instance's
/// overrides so the renderer's next `expand_instance` recursion applies it — see
/// [`route_nested`].
fn apply_sparse_overrides(out: &mut [ExpandedNode], instance: &InstanceNode) {
    for ov in &instance.overrides {
        if apply_at_path(out, &ov.target_path, |node| apply_override(node, &ov.value)) {
            continue;
        }
        // The exact def-local path didn't match a clone at this level. Before
        // routing deeper, try the ICON-SWAP RECOLOR fallback: a single-level
        // `Fills` override that recolors "the icon's one vector" is keyed by the
        // path of the icon master's *original* vector. When this instance is
        // itself the swapped icon (its `component` was re-pointed by an outer
        // `SwapInstance`), that original vector id no longer exists — the swapped
        // master has a different leaf. Figma recolors whatever icon now occupies
        // the slot, so we re-target the recolor onto the sole vector leaf of the
        // (swapped) master. Guarded to the unambiguous single-vector case (every
        // Spectrum icon is one vector), so a multi-shape master is never
        // mis-recolored.
        if matches!(ov.value, OverrideValue::Fills { .. })
            && ov.target_path.len() <= 1
            && apply_to_sole_vector(out, |node| apply_override(node, &ov.value))
        {
            continue;
        }
        route_nested(out, &ov.target_path, |remainder, inst| {
            inst.overrides.push(Override {
                target_path: remainder.iter().copied().collect(),
                target_prop: ov.target_prop,
                value: ov.value.clone(),
            });
        });
    }
}

/// Apply `f` to the expanded clone whose `def_path` matches `target` exactly
/// (a single-level address within this master). Returns whether a clone matched
/// — `false` means the path is deeper than this master level (it crosses a
/// nested-instance boundary) and should be routed by [`route_nested`].
fn apply_at_path(
    out: &mut [ExpandedNode],
    target: &[NodeId],
    f: impl FnOnce(&mut CanvasNode),
) -> bool {
    if let Some(en) = out.iter_mut().find(|e| e.def_path.as_slice() == target) {
        f(&mut en.node);
        true
    } else {
        false
    }
}

/// Apply `f` to the clone that is the master's SOLE [`NodeData::Vector`], if there
/// is exactly one. Returns whether it applied. Used by the icon-swap recolor
/// fallback: when a single-level `Fills` override can't match its exact (pre-swap)
/// vector path, and the swapped master has a single vector, recolor that vector —
/// the faithful behaviour for a one-shape icon whose component was swapped.
fn apply_to_sole_vector(out: &mut [ExpandedNode], f: impl FnOnce(&mut CanvasNode)) -> bool {
    let mut only: Option<usize> = None;
    for (i, e) in out.iter().enumerate() {
        if matches!(e.node.data, NodeData::Vector(_)) {
            if only.is_some() {
                return false; // ambiguous: more than one vector
            }
            only = Some(i);
        }
    }
    if let Some(i) = only {
        f(&mut out[i].node);
        true
    } else {
        false
    }
}

/// Route a deeper-than-this-level override/derived onto the nested instance it
/// crosses. A leading PREFIX of `target` addresses a nested instance clone in
/// this expansion (the clone whose `def_path` is the longest prefix of `target`
/// while being an [`NodeData::Instance`]); `f` is called with the REMAINDER
/// (`target` minus that prefix) and that clone's [`InstanceNode`], so the
/// renderer's next `expand_instance` recursion applies the remainder against the
/// nested master. Peeling the matched-instance prefix (not a fixed one id) lets
/// a nested instance buried under groups in its parent master still route
/// correctly, and walks an arbitrarily deep `guidPath` level by level. A path
/// with no instance-clone prefix is silently dropped (tolerant: a stale /
/// cross-master path is never fatal).
fn route_nested(
    out: &mut [ExpandedNode],
    target: &[NodeId],
    f: impl FnOnce(&[NodeId], &mut InstanceNode),
) {
    // Longest instance-clone prefix of `target`. A clone's `def_path` is a strict
    // prefix of the full target when the target descends into that instance.
    let mut best: Option<usize> = None;
    for (i, en) in out.iter().enumerate() {
        let dl = en.def_path.len();
        if dl == 0 || dl >= target.len() {
            continue; // root, or not a strict prefix (must leave a remainder)
        }
        if target.starts_with(en.def_path.as_slice())
            && matches!(en.node.data, NodeData::Instance(_))
            && best.map(|b| out[b].def_path.len() < dl).unwrap_or(true)
        {
            best = Some(i);
        }
    }
    if let Some(i) = best {
        let prefix_len = out[i].def_path.len();
        let remainder: OverridePath = target[prefix_len..].iter().copied().collect();
        if let NodeData::Instance(inst) = &mut out[i].node.data {
            f(&remainder, inst);
        }
    }
}

/// Write a baked [`DerivedOverride`] onto an expanded clone, overwriting only the
/// fields the entry actually populated. Type-incompatible targets are tolerated
/// (a geometry payload on a non-vector is ignored), matching the rest of the
/// model's best-effort writes.
///
/// [`DerivedOverride`]: crate::node::DerivedOverride
fn apply_derived(
    node: &mut CanvasNode,
    d: &crate::node::DerivedOverride,
    edited: bool,
    claims: OverrideClaims,
) {
    // Position / scale: Figma bakes the resolved local transform here. Geometry
    // (transform, size, path) is applied regardless of `edited` — it is the
    // instance's correct per-placement layout; only the baked FILL is skipped
    // when the master has been edited, so a master fill change shows through.
    if let Some(t) = d.transform {
        node.transform = t;
    }
    if let Some(size) = d.size {
        apply_derived_size(&mut node.data, size);
    }
    apply_derived_geometry(&mut node.data, d, edited, claims);
    // Baked text layout: resolved content + font size / line height / spacing,
    // plus the per-instance resolved theme color / weight / family. The color is
    // the headline fix: a label on a Dark page must render in its real (light)
    // resolved color, not the light master's near-black default.
    if let (Some(dt), NodeData::Text(t)) = (&d.text, &mut node.data) {
        apply_derived_text(t, dt, claims);
    }
}

/// Write the baked resolved box size onto whichever node variant carries one.
fn apply_derived_size(data: &mut NodeData, [w, h]: [f64; 2]) {
    match data {
        NodeData::Vector(v) => {
            let Some(bounds) = v.path.rough_bounds() else {
                return;
            };
            if v.path.is_rect() {
                v.path = PathData::rect(bounds.min_x, bounds.min_y, w, h);
            } else {
                let bounds_width = bounds.width();
                let bounds_height = bounds.height();
                let scale_x = if bounds_width > 1e-9 {
                    w / bounds_width
                } else {
                    1.0
                };
                let scale_y = if bounds_height > 1e-9 {
                    h / bounds_height
                } else {
                    1.0
                };
                if (scale_x - 1.0).abs() > 1e-9 || (scale_y - 1.0).abs() > 1e-9 {
                    v.path
                        .scale_about(bounds.min_x, bounds.min_y, scale_x, scale_y);
                }
            }
            // Keep the SVG viewport in sync with the resized path so the clip
            // still matches the instance's box (else an enlarged instance would
            // be cropped to the master's smaller viewport). Only when the master
            // actually carries a viewport — never introduce a clip on a vector
            // that had none.
            if v.local_size.is_some() {
                v.local_size = Some([w, h]);
            }
        }
        // Only the variants the importer bakes derived sizes for; the
        // placeholder variants (video/audio/…) never carry one, so they are
        // deliberately not written here.
        NodeData::Text(_) | NodeData::Bitmap(_) | NodeData::Instance(_) => {
            data.set_local_size(w, h);
        }
        // Keep either kind of authored group box in sync without changing its
        // clipping semantics.
        NodeData::Group(g) if g.clip_size.is_some() => {
            g.clip_size = Some([w, h]);
        }
        NodeData::Group(g) => {
            g.local_size = Some([w, h]);
        }
        _ => {}
    }
}

/// Write the baked resolved geometry (vector path/fills/stroke, or a frame's
/// background fill) onto the node.
fn apply_derived_geometry(
    data: &mut NodeData,
    d: &crate::node::DerivedOverride,
    edited: bool,
    claims: OverrideClaims,
) {
    match data {
        NodeData::Vector(v) => {
            // Resolved fill geometry replaces the master path outright.
            if let Some(path) = &d.path_data {
                v.path = path.clone();
                if d.stroke_path.is_none() && d.stroke_weight.is_none() {
                    v.strokes.clear();
                }
            }
            // Baked per-instance fills, when present (uncommon — theme fills
            // usually flow through variable/mode resolution, not the baked
            // entry). Skipped once the master is edited so a master fill change
            // reaches the instance instead of being overwritten by the snapshot,
            // and skipped when an explicit fill override claimed this node.
            if !edited
                && !claims.fill
                && let Some(fills) = &d.fills
            {
                v.fills = fills.clone();
            }
            // Resolved stroke weight updates the first stroke's width.
            if let Some(w) = d.stroke_weight {
                if let Some(stroke) = v.strokes.first_mut() {
                    stroke.width = w;
                }
            }
        }
        // A frame is a `Group` carrying a single `background` paint, so a baked
        // fill on a frame lands there (a frame's resolved theme background is
        // the literal "white header" bug when this arm is missing).
        NodeData::Group(g) => {
            if !edited
                && !claims.fill
                && let Some(fills) = &d.fills
            {
                g.background = fills.first().cloned();
            }
        }
        _ => {}
    }
}

/// Write the baked resolved text layout (content + font size / line height /
/// spacing + per-instance theme color / weight / family) onto a text node.
fn apply_derived_text(
    t: &mut crate::node::TextNode,
    dt: &crate::node::DerivedText,
    claims: OverrideClaims,
) {
    // An explicit TextContent override on this clone is the user's live edit;
    // the baked characters are the import-time snapshot of the master's text.
    if !claims.content
        && let Some(content) = &dt.content
    {
        t.content = content.clone();
    }
    if let Some(fs) = dt.font_size {
        if fs > 0.0 {
            t.style.size_px = fs;
        }
    }
    if let Some(lh) = dt.line_height {
        if lh > 0.0 {
            t.style.line_height = lh;
            // The derived entry replaces the master's line height wholesale:
            // a plain multiple clears any inherited auto marker, while an
            // auto/metric-relative resolution re-establishes it below.
            t.style.line_height_auto_percent = dt.line_height_auto_percent;
        }
    }
    if let Some(ls) = dt.letter_spacing {
        t.style.letter_spacing = ls;
    }
    // An explicit fill override (routed onto the glyph color) wins over the
    // baked per-instance theme color, like content above.
    if !claims.fill
        && let Some(color) = dt.color
    {
        t.set_glyph_color(color);
    }
    if let Some(weight) = dt.weight {
        t.style.weight = weight;
    }
    if let Some(family) = &dt.family {
        t.style.font_family = family.clone();
    }
}

/// Resolve the [`ComponentDef`] an instance should expand to, transparently
/// handling the case where `instance.component` names a [`ComponentSet`] rather
/// than a member def.
///
/// - **Def id**: returned directly.
/// - **Set id**: pick the member that matches the instance's *variant selection*
///   (its `prop_values` for the def's `Variant { axis }` props, matched against
///   each member's [`ComponentSetMembership::axis_values`]); if no member matches
///   every selected axis, or the instance selects nothing, fall back to the set's
///   `default_variant`. A set with a `default_variant` that isn't a real def (an
///   empty/degenerate set) yields `None`.
/// - **Neither**: `None` (dangling — the caller renders the unresolved outline).
///
/// This is what lets an instance of a variant *group* render the right (or at
/// least the default) variant instead of nothing.
///
/// [`ComponentSet`]: crate::component::ComponentSet
/// [`ComponentSetMembership::axis_values`]: crate::component::ComponentSetMembership::axis_values
fn resolve_instance_def<'a>(
    scene: &Scene,
    components: &'a ComponentLibrary,
    instance: &InstanceNode,
    context: Option<&InstanceExpansionContext<'_>>,
) -> Option<&'a ComponentDef> {
    // Direct def hit — the common case.
    if let Some(def) = components.def(instance.component) {
        return Some(def);
    }
    // Otherwise it may be a component *set*; resolve to a member def.
    let set = components.sets.get(&instance.component)?;
    let member = select_set_variant(Some(scene), components, instance, set, context);
    components.def(member)
}

fn resolve_instance_def_without_context<'a>(
    components: &'a ComponentLibrary,
    instance: &InstanceNode,
) -> Option<&'a ComponentDef> {
    if let Some(def) = components.def(instance.component) {
        return Some(def);
    }
    let set = components.sets.get(&instance.component)?;
    let member = select_set_variant(None, components, instance, set, None);
    components.def(member)
}

/// Choose which member [`ComponentId`](crate::id::ComponentId) of `set` an
/// instance resolves to.
///
/// Builds the instance's selected axis→value map from its `prop_values`, then
/// returns the first member whose membership `axis_values` agree on every
/// selected axis. With no usable selection, or no matching member, returns the
/// set's `default_variant`.
fn select_set_variant(
    scene: Option<&Scene>,
    components: &ComponentLibrary,
    instance: &InstanceNode,
    set: &crate::component::ComponentSet,
    context: Option<&InstanceExpansionContext<'_>>,
) -> crate::id::ComponentId {
    // Map the instance's variant prop selections (axis name → chosen value).
    // `prop_values` is keyed by ComponentPropId; we read each member-def-agnostic
    // selection off the *set's* member defs' prop schema. In practice the variant
    // props live on the member defs; we scan members for a `Variant { axis }`
    // prop whose id the instance set, and record the chosen string value.
    let mut selected: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    for &member_id in &set.members {
        let Some(def) = components.def(member_id) else {
            continue;
        };
        for prop in &def.props {
            if let ComponentPropKind::Variant { axis } = &prop.kind {
                if let Some(value) = instance
                    .prop_values
                    .get(&prop.id)
                    .and_then(|value| resolve_component_prop_value(scene, value, context))
                    .as_ref()
                    .and_then(variant_value_string)
                {
                    selected.insert(axis.clone(), value);
                }
            }
        }
    }

    if !selected.is_empty() {
        // First member agreeing on EVERY selected axis — the exact variant.
        if let Some(&m) = set.members.iter().find(|&&m| {
            components
                .def(m)
                .and_then(|d| d.variant_of.as_ref())
                .map(|vm| {
                    selected
                        .iter()
                        .all(|(axis, val)| vm.axis_values.get(axis) == Some(val))
                })
                .unwrap_or(false)
        }) {
            return m;
        }
        // No exact match (the instance selected a combination no member carries —
        // a stale/partial selection, or a member set narrower than the axes).
        // Figma still renders the *closest* variant rather than nothing, so fall
        // back to the member agreeing on the MOST selected axes (ties → the first,
        // matching `find`'s order). Only when at least one axis agrees; otherwise
        // the selection is unrelated to this set and the default is the right pick.
        let best = set
            .members
            .iter()
            .filter_map(|&m| {
                let vm = components.def(m).and_then(|d| d.variant_of.as_ref())?;
                let score = selected
                    .iter()
                    .filter(|(axis, val)| vm.axis_values.get(*axis) == Some(*val))
                    .count();
                (score > 0).then_some((score, m))
            })
            .max_by_key(|(score, _)| *score);
        if let Some((_, m)) = best {
            return m;
        }
    }
    set.default_variant
}

fn resolve_component_prop_value(
    scene: Option<&Scene>,
    value: &VarValue,
    context: Option<&InstanceExpansionContext<'_>>,
) -> Option<ResolvedVarValue> {
    match (value, scene, context) {
        (
            VarValue::Alias { variable },
            Some(scene),
            Some(InstanceExpansionContext {
                variables,
                active_modes,
                mode_anchor,
            }),
        ) => resolve_bound_value(variables, scene, *mode_anchor, active_modes, *variable),
        _ => value.as_resolved(),
    }
}

fn variant_value_string(value: &ResolvedVarValue) -> Option<String> {
    match value {
        ResolvedVarValue::String { value } => Some(value.clone()),
        ResolvedVarValue::Boolean { value } => {
            Some(if *value { "True" } else { "False" }.to_owned())
        }
        _ => None,
    }
}

/// The def-local path of `node` within the master rooted at `root`: original
/// ids from the root's child down to `node` (root excluded). `node == root`
/// gives `[]`.
/// The def-local path from a master `root` to a descendant `node`: original
/// master ids, root-first, root excluded (`[]` when `node == root`). This is the
/// exact address space [`Override::target_path`](crate::node::Override) and
/// [`PropBindingTarget`](crate::component::PropBindingTarget) use, so the editor
/// authors binding targets that [`expand_instance`] will match.
pub fn def_local_path(scene: &Scene, root: NodeId, node: NodeId) -> OverridePath {
    if node == root {
        return OverridePath::new();
    }
    // Ancestors are yielded bottom-up (parent first); stop before the root.
    let mut up: Vec<NodeId> = Vec::new();
    for anc in scene.ancestors_of(node) {
        if anc.id == root {
            break;
        }
        up.push(anc.id);
    }
    up.reverse();
    let mut path: OverridePath = up.into_iter().collect();
    path.push(node);
    path
}

/// Write an override's value onto an expanded clone. Type-incompatible targets
/// (e.g. a text override on a vector) are ignored, like the rest of the model's
/// best-effort writes.
fn apply_override(node: &mut CanvasNode, value: &OverrideValue) {
    match value {
        OverrideValue::Text { value } => {
            if let NodeData::Text(t) = &mut node.data {
                t.content = value.clone();
            }
        }
        OverrideValue::Fills { fills } => match &mut node.data {
            NodeData::Vector(v) => {
                v.fills = fills.clone();
            }
            // A frame (a `Group` with a `background`) carries its resolved fill
            // there, not in a `fills` vec — without this arm a frame override
            // fill is silently dropped and the frame renders as its master.
            NodeData::Group(g) => {
                g.background = fills.first().cloned();
            }
            // A TEXT node carries its single glyph color on `style.color`, not in
            // a `fills` vec. A themed instance pins its descendant text's per-page
            // color via a `styleIdForFill` symbolOverride (e.g. a Darkest card link
            // pins `darkest/gray/gray-700` = #D0D0D0 while its light-master clone
            // is #000000 / #222222). Without this arm the fill override is silently
            // dropped and the text renders in its light-master color — dark glyphs
            // on a dark page. Only the first *solid* paint maps to a glyph color; a
            // gradient/image text fill isn't modelled, so we leave the master color
            // untouched there. This is the text-channel analog of the `_Header`
            // surface fix (the per-page fill override must win over the master).
            NodeData::Text(t) => {
                if let Some(crate::style::Fill::Solid { color, .. }) = fills.first() {
                    t.set_glyph_color(*color);
                }
            }
            _ => {}
        },
        OverrideValue::Strokes { strokes } => match &mut node.data {
            NodeData::Vector(v) => {
                v.strokes = strokes.clone();
            }
            NodeData::Group(g) => {
                g.strokes = strokes.clone();
            }
            _ => {}
        },
        OverrideValue::Visible { value } => {
            node.flags.set(NodeFlags::HIDDEN, !*value);
        }
        OverrideValue::SwapInstance { component } => {
            if let NodeData::Instance(i) = &mut node.data {
                i.component = *component;
            }
        }
        // Generic escape hatch: the value is a PARTIAL CanvasNode JSON object
        // that serde-merges onto the clone. One arm covers every present and
        // future property (F1 in docs/research/figma-parity-divergence.md —
        // this being a no-op was OV-9, the blocker for the generic import fix).
        OverrideValue::Field { value } => apply_field_override(node, value),
    }
}

/// Top-level keys a [`OverrideValue::Field`] object may never write. Identity
/// and hierarchy belong to the expansion itself (every clone gets a fresh `id`,
/// a rewired `parent`, and keeps its master z-`index`), and `type` is the
/// flattened [`NodeData`] variant tag — an override can restyle a master node
/// but never change what KIND of node it is (structure is not an override).
const FIELD_STRUCTURAL_KEYS: [&str; 4] = ["id", "parent", "index", "type"];

/// Apply a generic [`OverrideValue::Field`] payload onto an expanded clone:
/// serialize the clone to JSON, shallow-merge the payload's keys over it, and
/// deserialize back into a [`CanvasNode`].
///
/// Shallow (top-level key REPLACEMENT), not a deep merge, on purpose: an
/// `effects` list or a text `style` object swaps wholesale — exactly Figma's
/// override semantics for those channels — and absent-vs-default handling stays
/// in serde, where it already lives. Because [`NodeData`] is `#[serde(flatten)]`
/// on the wrapper, variant payload fields (`corner_radius`, `style_runs`, …)
/// are top-level keys here too, so one mechanism reaches both.
///
/// Tolerance matches the typed arms' best-effort writes: a non-object payload,
/// a clone that can't round-trip (a non-finite float serializes to JSON null),
/// or a payload whose value shapes don't deserialize all leave the clone at its
/// master state — never half-applied, never a panic. A key the target variant
/// doesn't model is simply ignored by serde (the type-mismatch tolerance the
/// typed arms have).
fn apply_field_override(node: &mut CanvasNode, patch: &serde_json::Value) {
    let Some(patch) = patch.as_object() else {
        return;
    };
    if patch.is_empty() {
        return;
    }
    let Ok(serde_json::Value::Object(mut merged)) = serde_json::to_value(&*node) else {
        return;
    };
    for (key, value) in patch {
        if FIELD_STRUCTURAL_KEYS.contains(&key.as_str()) {
            continue;
        }
        merged.insert(key.clone(), value.clone());
    }
    if let Ok(applied) = serde_json::from_value::<CanvasNode>(serde_json::Value::Object(merged)) {
        *node = applied;
    }
}

#[cfg(test)]
#[path = "instance_tests/mod.rs"]
mod tests;
