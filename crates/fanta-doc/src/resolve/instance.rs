//! Instance expansion: deep-clone a component master's subtree with fresh ids,
//! apply the instance's sparse [`Override`]s and Figma's baked
//! [`DerivedOverride`](crate::node::DerivedOverride) data, and record each
//! clone's def-local path back to the master.
//!
//! [`expand_instance`] is one level deep — nested instances come back *as*
//! instances (with any swap-override applied) so the renderer recurses by
//! calling it again.

use crate::component::{ComponentDef, ComponentLibrary, ComponentPropKind};
use crate::id::NodeId;
use crate::node::{
    CanvasNode, InstanceNode, NodeData, NodeFlags, Override, OverridePath, OverrideValue,
};
use crate::path::PathData;
use crate::scene::Scene;
use crate::value::VarValue;
use std::collections::HashMap;

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
/// `instance.prop_values` are intentionally *not* applied to descendants here: a
/// component property drives a descendant property only once `ComponentPropDef`
/// carries a ref target (populated by the `.fig` importer), which is a follow-up.
/// Explicit overrides — the primary Figma mechanism — are fully applied. The one
/// exception is *variant selection*: when `instance.component` names a
/// [`ComponentSet`] rather than a member def, the variant axis values in
/// `prop_values` (or the set's `default_variant`) pick the member def to expand —
/// see [`resolve_instance_def`].
///
/// [`ComponentSet`]: crate::component::ComponentSet
pub fn expand_instance(
    scene: &Scene,
    components: &ComponentLibrary,
    instance: &InstanceNode,
) -> Vec<ExpandedNode> {
    let Some(def) = resolve_instance_def(components, instance) else {
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

    pin_inherited_root_surface(&mut out, instance);
    apply_prop_bindings(&mut out, def, instance);
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
    for d in &instance.derived {
        if !apply_at_path(&mut out, &d.path, |node| apply_derived(node, d)) {
            route_nested(&mut out, &d.path, |remainder, inst| {
                let mut nested = d.clone();
                nested.path = remainder.iter().copied().collect();
                inst.derived.push(nested);
            });
        }
    }
    out
}

/// mergeSymbolProps (op2 `mergeSymbolProps`): the instance's own surface is its
/// master root's surface — its background fill — but rendered at the *instance's*
/// box. The expansion root IS the master-root clone, so it already carries the
/// master root's `background`; here we additionally pin the root's clip box to the
/// instance's `local_size` when it has a background, so an instance that merely
/// INHERITS its background (rather than overriding it) still paints that
/// background at its own box, and a resized instance clips correctly. Without
/// this, a master root that isn't a clipping frame (`clip_size: None`) paints no
/// background on the transient walk, so the instance renders without its
/// inherited surface.
fn pin_inherited_root_surface(out: &mut [ExpandedNode], instance: &InstanceNode) {
    if let Some(rooten) = out.iter_mut().find(|e| e.def_path.is_empty()) {
        if let NodeData::Group(g) = &mut rooten.node.data {
            if g.background.is_some() {
                g.clip_size = Some(instance.local_size);
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
fn apply_prop_bindings(out: &mut [ExpandedNode], def: &ComponentDef, instance: &InstanceNode) {
    for prop in &def.props {
        if matches!(prop.kind, ComponentPropKind::Variant { .. }) || prop.bindings.is_empty() {
            continue;
        }
        let value = instance.prop_values.get(&prop.id).unwrap_or(&prop.default);
        let Some(resolved) = value.as_resolved() else {
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
fn apply_derived(node: &mut CanvasNode, d: &crate::node::DerivedOverride) {
    // Position / scale: Figma bakes the resolved local transform here.
    if let Some(t) = d.transform {
        node.transform = t;
    }
    if let Some(size) = d.size {
        apply_derived_size(&mut node.data, size);
    }
    apply_derived_geometry(&mut node.data, d);
    // Baked text layout: resolved content + font size / line height / spacing,
    // plus the per-instance resolved theme color / weight / family. The color is
    // the headline fix: a label on a Dark page must render in its real (light)
    // resolved color, not the light master's near-black default.
    if let (Some(dt), NodeData::Text(t)) = (&d.text, &mut node.data) {
        apply_derived_text(t, dt);
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
        }
        NodeData::Text(t) => t.local_size = [w, h],
        NodeData::Bitmap(b) => b.local_size = [w, h],
        NodeData::Instance(i) => i.local_size = [w, h],
        // A frame-like group clips to its size; keep that in sync so the
        // resolved instance clips to the baked box. A non-clipping group
        // (`clip_size: None`) has no box to update.
        NodeData::Group(g) if g.clip_size.is_some() => {
            g.clip_size = Some([w, h]);
        }
        _ => {}
    }
}

/// Write the baked resolved geometry (vector path/fills/stroke, or a frame's
/// background fill) onto the node.
fn apply_derived_geometry(data: &mut NodeData, d: &crate::node::DerivedOverride) {
    match data {
        NodeData::Vector(v) => {
            // Resolved fill geometry replaces the master path outright.
            if let Some(path) = &d.path_data {
                v.path = path.clone();
            }
            // Baked per-instance fills, when present (uncommon — theme fills
            // usually flow through variable/mode resolution, not the baked
            // entry).
            if let Some(fills) = &d.fills {
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
            if let Some(fills) = &d.fills {
                g.background = fills.first().cloned();
            }
        }
        _ => {}
    }
}

/// Write the baked resolved text layout (content + font size / line height /
/// spacing + per-instance theme color / weight / family) onto a text node.
fn apply_derived_text(t: &mut crate::node::TextNode, dt: &crate::node::DerivedText) {
    if let Some(content) = &dt.content {
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
        }
    }
    if let Some(ls) = dt.letter_spacing {
        t.style.letter_spacing = ls;
    }
    if let Some(color) = dt.color {
        t.style.color = color;
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
    components: &'a ComponentLibrary,
    instance: &InstanceNode,
) -> Option<&'a ComponentDef> {
    // Direct def hit — the common case.
    if let Some(def) = components.def(instance.component) {
        return Some(def);
    }
    // Otherwise it may be a component *set*; resolve to a member def.
    let set = components.sets.get(&instance.component)?;
    let member = select_set_variant(components, instance, set);
    components.def(member)
}

/// Choose which member [`ComponentId`](crate::id::ComponentId) of `set` an
/// instance resolves to.
///
/// Builds the instance's selected axis→value map from its `prop_values` (only
/// the def's `Variant { axis }` props with a string value participate), then
/// returns the first member whose membership `axis_values` agree on every
/// selected axis. With no usable selection, or no matching member, returns the
/// set's `default_variant`.
fn select_set_variant(
    components: &ComponentLibrary,
    instance: &InstanceNode,
    set: &crate::component::ComponentSet,
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
                if let Some(VarValue::String { value }) = instance.prop_values.get(&prop.id) {
                    selected.insert(axis.clone(), value.clone());
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
                if let Some(crate::style::Fill::Solid { color }) = fills.first() {
                    t.style.color = *color;
                }
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
        // Opaque escape-hatch overrides are not applied in v1.
        OverrideValue::Field { .. } => {}
    }
}

#[cfg(test)]
#[path = "instance_tests/mod.rs"]
mod tests;
