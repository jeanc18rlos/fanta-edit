//! Resolved-scene snapshot — the canonical "what did we actually resolve" view
//! of a document's active page, flattened to a serializable list of nodes.
//!
//! ## Why this exists
//!
//! A `Doc` is a graph of *authoring* state: component instances are one node
//! pointing at a master, fills can be driven by a bound variable whose value
//! depends on the active mode, and a frame can pin a different mode for its
//! subtree. None of that is what ends up on screen. The renderer
//! ([`fanta-render`]) resolves all of it at paint time — expanding instances
//! into transient subtrees and substituting bound values per node in the
//! effective mode.
//!
//! [`SceneSnapshot`] reproduces *exactly that resolution* with no Skia, so a
//! test (or future tooling) can ask "what did Fantaisa actually resolve this
//! page to?" and compare it against a reference. It is the data backbone of the
//! Tier-2 fidelity E2E (snapshot vs. an OpenPencil-derived golden) and a useful
//! diagnostic in its own right.
//!
//! The resolution here is a faithful, renderer-free port of `fanta-render`'s
//! walk:
//! - **Binding overlay** — every `(BoundProp, VariableId)` is resolved via
//!   [`resolve_bound_value`] (which picks the effective mode per collection:
//!   nearest-ancestor frame pin → doc active mode → collection default) and
//!   written onto a scratch clone before its values are read. Mirrors
//!   `fanta-render`'s `resolve_overlay`.
//! - **Instance expansion** — an [`NodeData::Instance`] is expanded via
//!   [`expand_instance`] into its transient subtree; nested instances recurse,
//!   exactly as the renderer does. The expansion root's own transform is
//!   suppressed (the instance node positions it), matching the renderer.
//!
//! Ordering is deterministic: a depth-first, z-order-ascending walk (the same
//! order the renderer paints in), so two runs over the same doc emit the same
//! `Vec` and snapshots diff cleanly.
//!
//! [`fanta-render`]: https://docs.rs/fanta-render

use crate::color::Color;
use crate::node::{CanvasNode, NodeData, NodeFlags};
use crate::resolve::{
    ExpandedNode, InstanceExpansionContext, ResolvedComponentDefRef, expand_instance_with_context,
    resolve_bound_value, resolved_component_with_context,
};
use crate::scene::Scene;
use crate::style::Fill;
use crate::transform::{Bounds, Transform2D};
use crate::{Doc, NodeId};
use serde::{Deserialize, Serialize};

/// A flat, serializable view of one document page after instance expansion and
/// variable resolution. Produced by [`SceneSnapshot::of_active_page`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneSnapshot {
    /// Resolved nodes in deterministic paint order (depth-first, z ascending).
    pub nodes: Vec<NodeSnapshot>,
}

/// Source-addressable companion to [`SceneSnapshot`].
///
/// A snapshot answers "what resolved values were painted"; this trace answers
/// "which authoring node produced each resolved node?" Real scene nodes retain
/// their [`NodeId`]. Component descendants additionally carry the placed
/// instance, the definition-local path, and a stable address that remains
/// distinct when the same master is placed more than once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedSceneTrace {
    pub schema: u16,
    pub coverage: TraceCoverage,
    /// Resolved nodes in the exact same order as [`SceneSnapshot::nodes`].
    pub nodes: Vec<ResolvedNodeTrace>,
}

/// Which traversal produced a [`ResolvedSceneTrace`].
///
/// The initial headless trace follows the renderer-independent resolved tree.
/// It intentionally does not claim to report viewport culling or paint/effect
/// outcomes from a concrete renderer backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceCoverage {
    SemanticResolvedTree,
}

/// One source-addressable resolved node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedNodeTrace {
    /// Stable address for this occurrence in the resolved tree.
    ///
    /// Real nodes use `node/<id>`. Expanded nodes append
    /// `component/<master-root>` and their definition-local path to the stable
    /// address of the instance node that introduced the expansion.
    pub address: String,
    /// Stable authoring id in the page or component master. This is never the
    /// fresh transient clone id produced by instance expansion.
    pub source_node_id: NodeId,
    /// Stable address of the resolved parent, if this is not the selected root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// The outermost placed scene-instance id. Nested component expansions keep
    /// this id so two placements of the same nested component stay distinct.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<NodeId>,
    /// Every component expansion crossed to reach this occurrence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instance_path: Vec<ResolvedInstanceHop>,
    /// Definition-local path for the nearest component expansion. The master
    /// root is represented by an empty path.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub def_path: Vec<NodeId>,
    pub name: String,
    pub kind: String,
    pub abs_bounds: Option<[f64; 4]>,
}

/// One component boundary in a [`ResolvedNodeTrace::instance_path`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedInstanceHop {
    /// Stable address of the instance node that introduced this expansion.
    pub instance_address: String,
    /// Authoring id of that instance node. For nested instances this is the id
    /// in the containing component definition.
    pub source_instance_id: NodeId,
    /// Effective component or component-set id on the resolved instance. A
    /// nested instance may already carry an outer placement's swap override.
    pub component_ref: crate::ComponentId,
    /// Exact component definition selected for expansion. This differs from
    /// `component_ref` when the instance references a component set.
    pub resolved_component: crate::ComponentId,
    /// Root authoring id of the resolved component definition (including the
    /// selected member when the instance points at a component set).
    pub component_root_id: NodeId,
}

/// One resolved node: its identity, kind, world-space box, and the paint values
/// the renderer would actually use (after bindings + instance overrides).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeSnapshot {
    /// The node's display name (e.g. `_Header`). Instance descendants carry the
    /// name baked onto their master clone, exactly as the renderer sees them.
    pub name: String,
    /// A short, stable variant tag: `frame`, `vector`, `text`, `instance`,
    /// `bitmap`, … — parallel to OpenPencil's `type`. Owned so the snapshot
    /// round-trips through serde (a distilled golden deserializes back into
    /// this exact type).
    pub kind: String,
    /// World-space axis-aligned bounds `[min_x, min_y, max_x, max_y]`. `None`
    /// for a node with no intrinsic bounds (an empty unclipped group).
    pub abs_bounds: Option<[f64; 4]>,
    /// The resolved color of the node's *first effective* fill (frame
    /// background or `fills[0]` / text color), straight-alpha RGBA. `None` when
    /// the node has no solid fill (unfilled, gradient/image only).
    pub fill_rgba: Option<[u8; 4]>,
    /// Resolved color of the first stroke's solid paint, if any.
    pub stroke_rgba: Option<[u8; 4]>,
    /// Text content for [`NodeData::Text`] nodes (after a `TextContent`
    /// binding), else `None`.
    pub text: Option<String>,
    /// Effective uniform corner radius for rectangle vectors, if set.
    pub corner_radius: Option<f64>,
    /// `true` when this node is a **mask** for its following siblings (Figma
    /// `isMask`). Skipped from serialization when `false` so existing
    /// mask-free goldens round-trip byte-identical.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_mask: bool,
}

impl SceneSnapshot {
    /// Resolve the document's active page into a flat snapshot. Returns an empty
    /// snapshot when the doc has no active page.
    ///
    /// The walk matches the renderer: hidden / fully-transparent nodes are
    /// skipped (they paint nothing), instances expand, and bindings resolve in
    /// each node's effective mode.
    pub fn of_active_page(doc: &Doc) -> Self {
        match doc.active_page() {
            Some(page) => Self::of_page(doc, page),
            None => Self { nodes: Vec::new() },
        }
    }

    /// Resolve a specific page (need not be the doc's active page). Useful when
    /// a test wants a page other than the importer-selected one.
    ///
    /// Auto-layout is solved first (Stage 2): the page subtree is laid out on a
    /// scratch clone of the scene — flow children get their computed positions,
    /// hug frames their content sizes, auto-width labels their measured widths —
    /// so the snapshot's world bounds reflect *laid-out* geometry rather than the
    /// importer's partially-baked transforms. The doc is never mutated (we clone
    /// the scene). Text measurement uses a Skia-free metric approximation here;
    /// the live renderer injects the real `fanta-text` shaper.
    pub fn of_page(doc: &Doc, page: NodeId) -> Self {
        Self::of_page_with_measure(doc, page, &mut metric_measure)
    }

    /// Resolve a page using a caller-provided text measurer.
    ///
    /// Renderer hosts use this to produce a semantic snapshot with the same
    /// font shaping and auto-layout measurements as the visual target while
    /// keeping `fanta-doc` renderer-agnostic. Pure document tests should use
    /// [`Self::of_page`], whose built-in metric approximation has no font or
    /// Skia dependency.
    pub fn of_page_with_measure(
        doc: &Doc,
        page: NodeId,
        measure: &mut impl FnMut(&crate::node::TextNode) -> (f64, f64),
    ) -> Self {
        Self::of_page_with_measure_and_trace(doc, page, measure).0
    }

    /// Resolve a page and return both its semantic snapshot and stable
    /// source-address trace in one traversal.
    pub fn of_page_with_trace(doc: &Doc, page: NodeId) -> (Self, ResolvedSceneTrace) {
        Self::of_page_with_measure_and_trace(doc, page, &mut metric_measure)
    }

    /// Caller-measured counterpart to [`Self::of_page_with_trace`].
    ///
    /// The snapshot and trace are emitted by one walk, so `snapshot.nodes[i]`
    /// and `trace.nodes[i]` always describe the same resolved occurrence.
    pub fn of_page_with_measure_and_trace(
        doc: &Doc,
        page: NodeId,
        measure: &mut impl FnMut(&crate::node::TextNode) -> (f64, f64),
    ) -> (Self, ResolvedSceneTrace) {
        let mut scene = doc.scene.clone();
        crate::layout::solve_auto_layout(&mut scene, page, measure);
        let mut nodes = Vec::new();
        let mut trace = Vec::new();
        visit_scene_node(doc, &scene, page, None, &mut nodes, &mut trace, measure);
        (
            Self { nodes },
            ResolvedSceneTrace {
                schema: 1,
                coverage: TraceCoverage::SemanticResolvedTree,
                nodes: trace,
            },
        )
    }
}

/// A Skia-free text measurer for the oracle: approximate a single line's width as
/// the character count times a fraction of the em size (a coarse average glyph
/// advance), and its height as one line. Good enough for the oracle's bounds
/// matching (a few hundred units of tolerance); the live renderer measures real
/// glyphs. Auto-width labels never wrap, so a single-line estimate is correct.
fn metric_measure(t: &crate::node::TextNode) -> (f64, f64) {
    const AVG_ADVANCE: f64 = 0.52; // mean advance/em across Latin text
    let chars = t.content.chars().filter(|c| *c != '\n').count() as f64;
    let w = chars * t.style.size_px * AVG_ADVANCE;
    let h = t.style.size_px * t.style.line_height;
    (w, h)
}

/// Resolve `node`'s bindings into a scratch clone if it binds anything, else
/// `None` (the common case allocates nothing). A faithful port of
/// `fanta-render`'s `resolve_overlay`: each binding resolves in the node's
/// effective mode and is applied with [`crate::binding::BoundProp::apply_resolved`].
/// Reads `scene` (the laid-out scratch clone) for the ancestor frame-pin walk —
/// structurally identical to `doc.scene`, so mode resolution is unaffected.
fn resolve_overlay(doc: &Doc, scene: &Scene, id: NodeId, node: &CanvasNode) -> Option<CanvasNode> {
    if node.bindings.is_empty() {
        return None;
    }
    let mut scratch = node.clone();
    for (prop, var_id) in &node.bindings {
        if let Some(resolved) =
            resolve_bound_value(&doc.variables, scene, id, &doc.active_modes, *var_id)
        {
            prop.apply_resolved(&mut scratch, resolved);
        }
    }
    Some(scratch)
}

/// Walk one real scene node: apply its binding overlay, skip hidden /
/// transparent nodes, emit its snapshot, then recurse — into the transient
/// expansion for an instance, or into its z-ordered scene children otherwise.
/// `scene` is the auto-layout-solved scratch clone (see [`SceneSnapshot::of_page`]).
fn visit_scene_node(
    doc: &Doc,
    scene: &Scene,
    id: NodeId,
    parent_address: Option<&str>,
    out: &mut Vec<NodeSnapshot>,
    trace: &mut Vec<ResolvedNodeTrace>,
    measure: &mut impl FnMut(&crate::node::TextNode) -> (f64, f64),
) {
    let Some(node) = scene.get(id) else { return };
    let scratch = resolve_overlay(doc, scene, id, node);
    let node: &CanvasNode = scratch.as_ref().unwrap_or(node);

    if node.flags.contains(NodeFlags::HIDDEN) || node.opacity.get() <= 0.0 {
        return;
    }

    // World bounds come from the (solved) scene's memoized computation.
    let abs = scene.world_bounds(id);
    out.push(node_snapshot(node, abs));
    let address = scene_address(id);
    trace.push(node_trace(
        node,
        abs,
        address.clone(),
        id,
        parent_address.map(str::to_owned),
        Vec::new(),
        Vec::new(),
    ));

    if let NodeData::Instance(inst) = &node.data {
        // World transform up to and including the instance node positions the
        // expansion root; descendants compose their own local transforms on top.
        let world = scene.world_transform(id).unwrap_or(Transform2D::IDENTITY);
        let expansion_context =
            InstanceExpansionContext::new(&doc.variables, &doc.active_modes, id);
        let component =
            resolved_component_with_context(scene, &doc.components, inst, &expansion_context);
        let mut expanded =
            expand_instance_with_context(scene, &doc.components, inst, &expansion_context);
        // Figma-derived instances already carry baked per-descendant layout; the
        // renderer only solves component masters that have no derived data.
        if inst.derived.is_empty() {
            crate::layout::solve_expanded(&mut expanded, measure);
        }
        if let Some(component) = component {
            visit_expanded_root(
                doc,
                scene,
                &expanded,
                world,
                id,
                &address,
                id,
                component,
                &[],
                out,
                trace,
                measure,
            );
        }
    } else {
        for &child in scene.children_of(Some(id)) {
            visit_scene_node(doc, scene, child, Some(&address), out, trace, measure);
        }
    }
}

/// Walk the transient subtree produced by [`expand_instance`], reconstructing
/// world transforms from the fresh clone parent→child links. `root_world` is
/// the world transform at the instance node (the expansion root inherits it;
/// the root's *own* transform is suppressed, matching the renderer).
fn visit_expanded_root(
    doc: &Doc,
    scene: &Scene,
    expanded: &[ExpandedNode],
    root_world: Transform2D,
    mode_anchor: NodeId,
    instance_address: &str,
    source_instance_id: NodeId,
    component: ResolvedComponentDefRef,
    prior_instance_path: &[ResolvedInstanceHop],
    out: &mut Vec<NodeSnapshot>,
    trace: &mut Vec<ResolvedNodeTrace>,
    measure: &mut impl FnMut(&crate::node::TextNode) -> (f64, f64),
) {
    let Some(root) = expanded.iter().find(|e| e.def_path.is_empty()) else {
        return;
    };
    let mut instance_path = prior_instance_path.to_vec();
    instance_path.push(ResolvedInstanceHop {
        instance_address: instance_address.to_owned(),
        source_instance_id,
        component_ref: component.component_ref,
        resolved_component: component.resolved_component,
        component_root_id: component.resolved_root,
    });
    visit_expanded(
        doc,
        scene,
        root,
        true,
        root_world,
        expanded,
        mode_anchor,
        instance_address,
        component.resolved_root,
        &instance_path,
        out,
        trace,
        measure,
    );
}

/// Recursive transient-node walk, mirroring `fanta-render`'s `render_expanded`:
/// binding overlay, hidden/transparent skip, world-transform composition, and a
/// nested-instance recursion through [`expand_instance`] again.
fn visit_expanded(
    doc: &Doc,
    scene: &Scene,
    entry: &ExpandedNode,
    is_root: bool,
    parent_world: Transform2D,
    expanded: &[ExpandedNode],
    mode_anchor: NodeId,
    instance_address: &str,
    component_root: NodeId,
    instance_path: &[ResolvedInstanceHop],
    out: &mut Vec<NodeSnapshot>,
    trace: &mut Vec<ResolvedNodeTrace>,
    measure: &mut impl FnMut(&crate::node::TextNode) -> (f64, f64),
) {
    let node = &entry.node;
    // Transient clones aren't in the scene, so binding resolution sees no
    // ancestor frame pins — only the doc-level modes — exactly as the renderer.
    let scratch = resolve_overlay(doc, scene, node.id, node);
    let node: &CanvasNode = scratch.as_ref().unwrap_or(node);

    if node.flags.contains(NodeFlags::HIDDEN) || node.opacity.get() <= 0.0 {
        return;
    }

    // The root's own transform is suppressed (the instance node already
    // positions it via `parent_world`); descendants compose theirs.
    let world = if is_root {
        parent_world
    } else {
        node.transform.then(&parent_world)
    };
    let abs = node.data.local_bounds().map(|b| b.transformed(&world));
    out.push(node_snapshot(node, abs));
    let address = expanded_address(instance_address, component_root, &entry.def_path);
    let parent = if entry.def_path.is_empty() {
        instance_address.to_owned()
    } else {
        expanded_address(
            instance_address,
            component_root,
            &entry.def_path[..entry.def_path.len() - 1],
        )
    };
    let source_node_id = entry.def_path.last().copied().unwrap_or(component_root);
    trace.push(node_trace(
        node,
        abs,
        address.clone(),
        source_node_id,
        Some(parent),
        instance_path.to_vec(),
        entry.def_path.to_vec(),
    ));

    if let NodeData::Instance(inner) = &node.data {
        let expansion_context =
            InstanceExpansionContext::new(&doc.variables, &doc.active_modes, mode_anchor);
        let inner_component =
            resolved_component_with_context(scene, &doc.components, inner, &expansion_context);
        let mut inner_expanded =
            expand_instance_with_context(scene, &doc.components, inner, &expansion_context);
        if inner.derived.is_empty() {
            crate::layout::solve_expanded(&mut inner_expanded, measure);
        }
        if let Some(inner_component) = inner_component {
            visit_expanded_root(
                doc,
                scene,
                &inner_expanded,
                world,
                mode_anchor,
                &address,
                source_node_id,
                inner_component,
                instance_path,
                out,
                trace,
                measure,
            );
        }
    } else {
        for child in expanded.iter().filter(|e| e.node.parent == Some(node.id)) {
            visit_expanded(
                doc,
                scene,
                child,
                false,
                world,
                expanded,
                mode_anchor,
                instance_address,
                component_root,
                instance_path,
                out,
                trace,
                measure,
            );
        }
    }
}

fn scene_address(id: NodeId) -> String {
    format!("node/{id}")
}

fn expanded_address(instance_address: &str, component_root: NodeId, def_path: &[NodeId]) -> String {
    let mut address = format!("{instance_address}/component/{component_root}");
    if !def_path.is_empty() {
        address.push_str("/def");
        for id in def_path {
            address.push('/');
            address.push_str(&id.to_string());
        }
    }
    address
}

#[allow(clippy::too_many_arguments)]
fn node_trace(
    node: &CanvasNode,
    abs: Option<Bounds>,
    address: String,
    source_node_id: NodeId,
    parent: Option<String>,
    instance_path: Vec<ResolvedInstanceHop>,
    def_path: Vec<NodeId>,
) -> ResolvedNodeTrace {
    ResolvedNodeTrace {
        address,
        source_node_id,
        parent,
        instance_id: instance_path.first().map(|hop| hop.source_instance_id),
        instance_path,
        def_path,
        name: node.name.clone(),
        kind: node.data.kind_tag().to_owned(),
        abs_bounds: abs.map(|bounds| [bounds.min_x, bounds.min_y, bounds.max_x, bounds.max_y]),
    }
}

/// Build the flat [`NodeSnapshot`] for one already-overlay-resolved node.
fn node_snapshot(node: &CanvasNode, abs: Option<Bounds>) -> NodeSnapshot {
    NodeSnapshot {
        name: node.name.clone(),
        kind: node.data.kind_tag().to_owned(),
        abs_bounds: abs.map(|b| [b.min_x, b.min_y, b.max_x, b.max_y]),
        fill_rgba: effective_fill(node).map(rgba),
        stroke_rgba: effective_stroke(node).map(rgba),
        text: match &node.data {
            NodeData::Text(t) => Some(t.content.clone()),
            _ => None,
        },
        corner_radius: match &node.data {
            // A per-corner array reports its top-left as the representative
            // radius when no uniform radius is set, so a card with mixed corners
            // still surfaces a value.
            NodeData::Vector(v) => v.corner_radius.or(v.corner_radii.map(|r| r[0])),
            // A frame (group) can carry its own rounding too — surface it the
            // same way so a rounded card/panel frame reports a radius.
            NodeData::Group(g) => g.corner_radius.or(g.corner_radii.map(|r| r[0])),
            _ => None,
        },
        is_mask: node.is_mask,
    }
}

/// The node's first *effective* solid fill color: a frame's background, else a
/// vector's first solid fill, else a text node's glyph color. `None` for
/// gradient/image-only or unfilled nodes.
fn effective_fill(node: &CanvasNode) -> Option<Color> {
    match &node.data {
        NodeData::Group(g) => solid_of(g.background.as_ref()),
        NodeData::Vector(v) => v.fills.iter().find_map(|f| solid_of(Some(f))),
        NodeData::Text(t) => Some(t.style.color),
        _ => None,
    }
}

/// The first stroke's solid paint color, if any. Vectors carry shape strokes;
/// a frame (group) can carry a border stroke too, so report it the same way.
fn effective_stroke(node: &CanvasNode) -> Option<Color> {
    match &node.data {
        NodeData::Vector(v) => v.strokes.iter().find_map(|s| solid_of(Some(&s.paint))),
        NodeData::Group(g) => g.strokes.iter().find_map(|s| solid_of(Some(&s.paint))),
        _ => None,
    }
}

/// Pull the solid color out of an optional [`Fill`].
fn solid_of(fill: Option<&Fill>) -> Option<Color> {
    match fill? {
        Fill::Solid { color, .. } => Some(*color),
        _ => None,
    }
}

fn rgba(c: Color) -> [u8; 4] {
    [c.r, c.g, c.b, c.a]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::{
        ComponentDef, ComponentPropDef, ComponentPropKind, ComponentSet, ComponentSetMembership,
        VariantAxis,
    };
    use crate::node::{GroupNode, TextNode, VectorNode};
    use crate::op::Operation;
    use crate::{ComponentId, ComponentPropId, VarValue};
    use smallvec::smallvec;
    use std::collections::BTreeMap;

    /// Build a doc with a single clipped frame (green bg) holding one red rect
    /// and one text node, and make the frame the active page.
    fn doc_with_frame() -> (Doc, NodeId) {
        let mut doc = Doc::new();
        let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([200.0, 100.0]),
            background: Some(Fill::solid(Color::rgb(0, 200, 0))),
            explicit_modes: Default::default(),
            ..Default::default()
        }));
        frame.name = "Frame".into();
        frame.transform = Transform2D::translation(10.0, 20.0);
        let frame_id = frame.id;
        doc.apply(Operation::create_node(frame)).unwrap();
        doc.add_page(frame_id);
        doc.set_active_page(Some(frame_id));

        let mut rect = CanvasNode::new(NodeData::Vector(VectorNode {
            path: crate::path::PathData::rect(0.0, 0.0, 40.0, 40.0),
            fills: smallvec![Fill::solid(Color::rgb(200, 0, 0))],
            strokes: smallvec![crate::style::Stroke::solid(Color::rgb(0, 0, 255), 2.0)],
            corner_radius: Some(8.0),
            corner_radii: None,
            corner_smoothing: 0.0,
            local_size: None,
            parametric: None,
        }));
        rect.name = "Rect".into();
        rect.parent = Some(frame_id);
        doc.apply(Operation::create_node(rect)).unwrap();

        let mut text = CanvasNode::new(NodeData::Text(TextNode::new("Hello", 80.0, 20.0)));
        text.name = "Label".into();
        text.parent = Some(frame_id);
        doc.apply(Operation::create_node(text)).unwrap();

        (doc, frame_id)
    }

    #[test]
    fn snapshot_captures_frame_rect_and_text() {
        let (doc, _frame) = doc_with_frame();
        let snap = SceneSnapshot::of_active_page(&doc);
        // Frame + rect + text = 3 nodes, deterministic order (frame first).
        assert_eq!(snap.nodes.len(), 3);
        assert_eq!(snap.nodes[0].name, "Frame");
        assert_eq!(snap.nodes[0].kind, "frame");
        assert_eq!(snap.nodes[0].fill_rgba, Some([0, 200, 0, 255]));
        // Frame world bounds = clip box translated by (10,20).
        assert_eq!(snap.nodes[0].abs_bounds, Some([10.0, 20.0, 210.0, 120.0]));

        let rect = snap.nodes.iter().find(|n| n.name == "Rect").unwrap();
        assert_eq!(rect.kind, "vector");
        assert_eq!(rect.fill_rgba, Some([200, 0, 0, 255]));
        assert_eq!(rect.stroke_rgba, Some([0, 0, 255, 255]));
        assert_eq!(rect.corner_radius, Some(8.0));

        let text = snap.nodes.iter().find(|n| n.name == "Label").unwrap();
        assert_eq!(text.kind, "text");
        assert_eq!(text.text.as_deref(), Some("Hello"));
    }

    #[test]
    fn hidden_node_is_omitted() {
        let (mut doc, frame_id) = doc_with_frame();
        // Hide the rect child (found by name, since z-order is not insertion order).
        let rect_id = doc
            .scene
            .children_of(Some(frame_id))
            .iter()
            .copied()
            .find(|id| doc.scene.get(*id).map(|n| n.name.as_str()) == Some("Rect"))
            .unwrap();
        doc.scene
            .get_mut(rect_id)
            .unwrap()
            .flags
            .insert(NodeFlags::HIDDEN);
        let snap = SceneSnapshot::of_active_page(&doc);
        assert!(snap.nodes.iter().all(|n| n.name != "Rect"));
    }

    #[test]
    fn empty_doc_yields_empty_snapshot() {
        let doc = Doc::new();
        let snap = SceneSnapshot::of_active_page(&doc);
        assert!(snap.nodes.is_empty());
    }

    #[test]
    fn snapshot_is_deterministic() {
        let (doc, _) = doc_with_frame();
        let a = SceneSnapshot::of_active_page(&doc);
        let b = SceneSnapshot::of_active_page(&doc);
        assert_eq!(a, b);
    }

    #[test]
    fn trace_disambiguates_two_placements_without_transient_ids() {
        let mut doc = Doc::new();
        let mut master = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            40.0,
            24.0,
            Color::rgb(220, 40, 40),
        )));
        master.name = "Master".into();
        let master_root = master.id;
        doc.apply(Operation::create_node(master)).unwrap();
        let component = crate::ComponentId::new();
        doc.apply(Operation::DefineComponent {
            def: Box::new(ComponentDef::new(component, master_root, "Master")),
        })
        .unwrap();

        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "Page".into();
        let page_id = page.id;
        doc.apply(Operation::create_node(page)).unwrap();
        doc.add_page(page_id);

        let mut placed_ids = Vec::new();
        for x in [10.0, 80.0] {
            let mut instance = CanvasNode::new(NodeData::Instance(crate::InstanceNode {
                component,
                overrides: Vec::new(),
                prop_values: BTreeMap::new(),
                derived: Vec::new(),
                local_size: [40.0, 24.0],
            }));
            instance.parent = Some(page_id);
            instance.transform = Transform2D::translation(x, 8.0);
            placed_ids.push(instance.id);
            doc.apply(Operation::CreateInstance {
                node: Box::new(instance),
            })
            .unwrap();
        }

        let (_, trace) = SceneSnapshot::of_page_with_trace(&doc, page_id);
        let occurrences = trace
            .nodes
            .iter()
            .filter(|node| node.source_node_id == master_root && node.instance_id.is_some())
            .collect::<Vec<_>>();
        assert_eq!(occurrences.len(), 2);
        assert_ne!(occurrences[0].address, occurrences[1].address);
        assert_eq!(occurrences[0].def_path, Vec::<NodeId>::new());
        assert_eq!(occurrences[1].def_path, Vec::<NodeId>::new());
        assert!(placed_ids.contains(&occurrences[0].instance_id.unwrap()));
        assert!(placed_ids.contains(&occurrences[1].instance_id.unwrap()));
        assert_eq!(
            trace,
            SceneSnapshot::of_page_with_trace(&doc, page_id).1,
            "fresh transient clone ids must never enter the trace"
        );
    }

    #[test]
    fn trace_records_component_set_reference_and_selected_member() {
        let mut doc = Doc::new();
        let axis_prop = ComponentPropId::new();
        let set_id = ComponentId::new();
        let small_id = ComponentId::new();
        let large_id = ComponentId::new();

        let mut make_variant = |component: ComponentId, label: &str, axis_value: &str| {
            let mut master = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
                0.0,
                0.0,
                40.0,
                24.0,
                Color::rgb(40, 80, 220),
            )));
            master.name = label.into();
            let root = master.id;
            doc.apply(Operation::create_node(master)).unwrap();

            let mut def = ComponentDef::new(component, root, label);
            def.variant_of = Some(ComponentSetMembership {
                set: set_id,
                axis_values: BTreeMap::from([("Size".into(), axis_value.into())]),
            });
            def.props.push(ComponentPropDef {
                id: axis_prop,
                name: "Size".into(),
                kind: ComponentPropKind::Variant {
                    axis: "Size".into(),
                },
                formatter: Default::default(),
                default: VarValue::String {
                    value: axis_value.into(),
                },
                bindings: Vec::new(),
            });
            doc.apply(Operation::DefineComponent { def: Box::new(def) })
                .unwrap();
            root
        };
        let small_root = make_variant(small_id, "Small", "Small");
        let large_root = make_variant(large_id, "Large", "Large");
        doc.apply(Operation::DefineComponentSet {
            set: Box::new(ComponentSet {
                id: set_id,
                name: "Size".into(),
                axes: vec![VariantAxis {
                    name: "Size".into(),
                    values: vec!["Small".into(), "Large".into()],
                }],
                members: vec![small_id, large_id],
                default_variant: small_id,
            }),
        })
        .unwrap();

        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "Page".into();
        let page_id = page.id;
        doc.apply(Operation::create_node(page)).unwrap();
        doc.add_page(page_id);

        let mut instance = CanvasNode::new(NodeData::Instance(crate::InstanceNode {
            component: set_id,
            overrides: Vec::new(),
            prop_values: BTreeMap::from([(
                axis_prop,
                VarValue::String {
                    value: "Large".into(),
                },
            )]),
            derived: Vec::new(),
            local_size: [40.0, 24.0],
        }));
        instance.parent = Some(page_id);
        let instance_id = instance.id;
        doc.apply(Operation::CreateInstance {
            node: Box::new(instance),
        })
        .unwrap();

        let live_instance = doc
            .scene
            .get(instance_id)
            .and_then(|node| node.data.as_instance())
            .expect("placed set instance");
        let context = InstanceExpansionContext::new(&doc.variables, &doc.active_modes, instance_id);
        let resolved =
            resolved_component_with_context(&doc.scene, &doc.components, live_instance, &context)
                .expect("selected component-set member");
        assert_eq!(resolved.component_ref, set_id);
        assert_eq!(resolved.resolved_component, large_id);
        assert_eq!(resolved.resolved_root, large_root);

        let (_, trace) = SceneSnapshot::of_page_with_trace(&doc, page_id);
        let expanded_root = trace
            .nodes
            .iter()
            .find(|node| {
                node.source_node_id == large_root
                    && node.instance_id == Some(instance_id)
                    && node.def_path.is_empty()
            })
            .expect("selected variant root must be traced");
        let hop = expanded_root
            .instance_path
            .last()
            .expect("expanded root must carry one component hop");
        assert_eq!(hop.component_ref, set_id);
        assert_eq!(hop.resolved_component, large_id);
        assert_eq!(hop.component_root_id, large_root);
        assert!(
            trace
                .nodes
                .iter()
                .all(|node| node.source_node_id != small_root || node.instance_id.is_none()),
            "the unselected default member must not appear in the resolved occurrence trace"
        );
    }

    #[test]
    fn nested_trace_is_stable_and_distinct_per_outer_placement() {
        let mut doc = Doc::new();

        let mut inner_master = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            16.0,
            16.0,
            Color::rgb(20, 160, 90),
        )));
        inner_master.name = "Inner leaf".into();
        let inner_root = inner_master.id;
        doc.apply(Operation::create_node(inner_master)).unwrap();
        let inner_component = ComponentId::new();
        doc.apply(Operation::DefineComponent {
            def: Box::new(ComponentDef::new(inner_component, inner_root, "Inner")),
        })
        .unwrap();

        let mut outer_master = CanvasNode::new(NodeData::Group(GroupNode::default()));
        outer_master.name = "Outer master".into();
        let outer_root = outer_master.id;
        doc.apply(Operation::create_node(outer_master)).unwrap();
        let mut nested = CanvasNode::new(NodeData::Instance(crate::InstanceNode {
            component: inner_component,
            overrides: Vec::new(),
            prop_values: BTreeMap::new(),
            derived: Vec::new(),
            local_size: [16.0, 16.0],
        }));
        nested.name = "Nested slot".into();
        nested.parent = Some(outer_root);
        let nested_source_id = nested.id;
        doc.apply(Operation::CreateInstance {
            node: Box::new(nested),
        })
        .unwrap();
        let outer_component = ComponentId::new();
        doc.apply(Operation::DefineComponent {
            def: Box::new(ComponentDef::new(outer_component, outer_root, "Outer")),
        })
        .unwrap();

        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "Page".into();
        let page_id = page.id;
        doc.apply(Operation::create_node(page)).unwrap();
        doc.add_page(page_id);

        let mut placements = Vec::new();
        for x in [0.0, 48.0] {
            let mut instance = CanvasNode::new(NodeData::Instance(crate::InstanceNode {
                component: outer_component,
                overrides: Vec::new(),
                prop_values: BTreeMap::new(),
                derived: Vec::new(),
                local_size: [16.0, 16.0],
            }));
            instance.parent = Some(page_id);
            instance.transform = Transform2D::translation(x, 0.0);
            placements.push(instance.id);
            doc.apply(Operation::CreateInstance {
                node: Box::new(instance),
            })
            .unwrap();
        }

        let (_, trace) = SceneSnapshot::of_page_with_trace(&doc, page_id);
        let inner_occurrences = trace
            .nodes
            .iter()
            .filter(|node| {
                node.source_node_id == inner_root
                    && node.instance_path.len() == 2
                    && node.def_path.is_empty()
            })
            .collect::<Vec<_>>();
        assert_eq!(inner_occurrences.len(), 2);
        assert_ne!(inner_occurrences[0].address, inner_occurrences[1].address);

        let mut traced_placements = inner_occurrences
            .iter()
            .map(|node| node.instance_id.expect("outer placement id"))
            .collect::<Vec<_>>();
        traced_placements.sort();
        placements.sort();
        assert_eq!(traced_placements, placements);
        for occurrence in inner_occurrences {
            let outer_hop = &occurrence.instance_path[0];
            let inner_hop = &occurrence.instance_path[1];
            assert_eq!(outer_hop.component_ref, outer_component);
            assert_eq!(outer_hop.resolved_component, outer_component);
            assert_eq!(outer_hop.component_root_id, outer_root);
            assert_eq!(inner_hop.source_instance_id, nested_source_id);
            assert_eq!(inner_hop.component_ref, inner_component);
            assert_eq!(inner_hop.resolved_component, inner_component);
            assert_eq!(inner_hop.component_root_id, inner_root);
            assert_eq!(
                occurrence.parent.as_deref(),
                Some(inner_hop.instance_address.as_str()),
                "the nested expansion root must parent to its stable instance occurrence"
            );
        }

        assert_eq!(
            trace,
            SceneSnapshot::of_page_with_trace(&doc, page_id).1,
            "nested addresses must not depend on fresh expansion clone ids"
        );
    }
}
