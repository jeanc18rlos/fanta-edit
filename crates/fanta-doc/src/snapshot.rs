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
use crate::resolve::{ExpandedNode, expand_instance, resolve_bound_value};
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
        let mut scene = doc.scene.clone();
        crate::layout::solve_auto_layout(&mut scene, page, &mut metric_measure);
        let mut nodes = Vec::new();
        visit_scene_node(doc, &scene, page, &mut nodes);
        Self { nodes }
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
fn visit_scene_node(doc: &Doc, scene: &Scene, id: NodeId, out: &mut Vec<NodeSnapshot>) {
    let Some(node) = scene.get(id) else { return };
    let scratch = resolve_overlay(doc, scene, id, node);
    let node: &CanvasNode = scratch.as_ref().unwrap_or(node);

    if node.flags.contains(NodeFlags::HIDDEN) || node.opacity <= 0.0 {
        return;
    }

    // World bounds come from the (solved) scene's memoized computation.
    let abs = scene.world_bounds(id);
    out.push(node_snapshot(node, abs));

    if let NodeData::Instance(inst) = &node.data {
        // World transform up to and including the instance node positions the
        // expansion root; descendants compose their own local transforms on top.
        let world = scene.world_transform(id).unwrap_or(Transform2D::IDENTITY);
        let mut expanded = expand_instance(scene, &doc.components, inst);
        // Figma-derived instances already carry baked per-descendant layout; the
        // renderer only solves component masters that have no derived data.
        if inst.derived.is_empty() {
            crate::layout::solve_expanded(&mut expanded, &mut metric_measure);
        }
        visit_expanded_root(doc, scene, &expanded, world, out);
    } else {
        for &child in scene.children_of(Some(id)) {
            visit_scene_node(doc, scene, child, out);
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
    out: &mut Vec<NodeSnapshot>,
) {
    let Some(root) = expanded.iter().find(|e| e.def_path.is_empty()) else {
        return;
    };
    visit_expanded(doc, scene, &root.node, true, root_world, expanded, out);
}

/// Recursive transient-node walk, mirroring `fanta-render`'s `render_expanded`:
/// binding overlay, hidden/transparent skip, world-transform composition, and a
/// nested-instance recursion through [`expand_instance`] again.
fn visit_expanded(
    doc: &Doc,
    scene: &Scene,
    node: &CanvasNode,
    is_root: bool,
    parent_world: Transform2D,
    expanded: &[ExpandedNode],
    out: &mut Vec<NodeSnapshot>,
) {
    // Transient clones aren't in the scene, so binding resolution sees no
    // ancestor frame pins — only the doc-level modes — exactly as the renderer.
    let scratch = resolve_overlay(doc, scene, node.id, node);
    let node: &CanvasNode = scratch.as_ref().unwrap_or(node);

    if node.flags.contains(NodeFlags::HIDDEN) || node.opacity <= 0.0 {
        return;
    }

    // The root's own transform is suppressed (the instance node already
    // positions it via `parent_world`); descendants compose theirs.
    let world = if is_root {
        parent_world
    } else {
        node.transform.then(&parent_world)
    };
    let abs = local_bounds_of(node).map(|b| b.transformed(&world));
    out.push(node_snapshot(node, abs));

    if let NodeData::Instance(inner) = &node.data {
        let mut inner_expanded = expand_instance(scene, &doc.components, inner);
        if inner.derived.is_empty() {
            crate::layout::solve_expanded(&mut inner_expanded, &mut metric_measure);
        }
        visit_expanded_root(doc, scene, &inner_expanded, world, out);
    } else {
        for child in expanded.iter().filter(|e| e.node.parent == Some(node.id)) {
            visit_expanded(doc, scene, &child.node, false, world, expanded, out);
        }
    }
}

/// Intrinsic local-space bounds of a single node, independent of any scene
/// index (transient clones aren't indexed). For a group this is its clip box if
/// set, else `None` — a transient group's content bounds would require walking
/// the (already-emitted) children, which the caller does via composition.
fn local_bounds_of(node: &CanvasNode) -> Option<Bounds> {
    match &node.data {
        NodeData::Group(g) => g.clip_size.map(|[w, h]| Bounds::from_xywh(0.0, 0.0, w, h)),
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
        NodeData::Video(v) => Some(Bounds::from_xywh(
            0.0,
            0.0,
            v.local_size[0],
            v.local_size[1],
        )),
        NodeData::Audio(a) => Some(Bounds::from_xywh(
            0.0,
            0.0,
            a.local_size[0],
            a.local_size[1],
        )),
        NodeData::NodeGraph(n) => Some(Bounds::from_xywh(
            0.0,
            0.0,
            n.local_size[0],
            n.local_size[1],
        )),
        NodeData::Model3d(m) => Some(Bounds::from_xywh(
            0.0,
            0.0,
            m.local_size[0],
            m.local_size[1],
        )),
        NodeData::AiArtifact(a) => Some(Bounds::from_xywh(
            0.0,
            0.0,
            a.local_size[0],
            a.local_size[1],
        )),
        NodeData::Instance(i) => Some(Bounds::from_xywh(
            0.0,
            0.0,
            i.local_size[0],
            i.local_size[1],
        )),
        NodeData::Embed(e) => Some(Bounds::from_xywh(
            0.0,
            0.0,
            e.local_size[0],
            e.local_size[1],
        )),
    }
}

/// Build the flat [`NodeSnapshot`] for one already-overlay-resolved node.
fn node_snapshot(node: &CanvasNode, abs: Option<Bounds>) -> NodeSnapshot {
    NodeSnapshot {
        name: node.name.clone(),
        kind: kind_tag(&node.data).to_owned(),
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
        Fill::Solid { color } => Some(*color),
        _ => None,
    }
}

fn rgba(c: Color) -> [u8; 4] {
    [c.r, c.g, c.b, c.a]
}

/// Short stable variant tag. A group with a clip box reads as a `frame` (Figma's
/// distinction), an unclipped group as `group`.
fn kind_tag(data: &NodeData) -> &'static str {
    match data {
        NodeData::Group(g) => {
            if g.clip_size.is_some() {
                "frame"
            } else {
                "group"
            }
        }
        NodeData::Vector(_) => "vector",
        NodeData::Text(_) => "text",
        NodeData::Bitmap(_) => "bitmap",
        NodeData::Video(_) => "video",
        NodeData::Audio(_) => "audio",
        NodeData::NodeGraph(_) => "node_graph",
        NodeData::Model3d(_) => "model3d",
        NodeData::AiArtifact(_) => "ai_artifact",
        NodeData::Instance(_) => "instance",
        NodeData::Embed(_) => "embed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{GroupNode, TextNode, VectorNode};
    use crate::op::Operation;
    use smallvec::smallvec;

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
}
