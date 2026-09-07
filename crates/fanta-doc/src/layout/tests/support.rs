//! Shared fixtures for the auto-layout suite — the in-memory `VecTree`, the
//! fake/panicking measurers, node builders, and placement assertions used
//! across more than one section. Reached via `use super::*;` (re-exported by
//! `tests/mod.rs`).
#![allow(dead_code)]

use super::*;

/// A trivial in-memory tree: nodes in a flat vec, children derived from each
/// node's `parent` in insertion order (insertion order = z-order for the test).
pub(crate) struct VecTree {
    nodes: Vec<CanvasNode>,
}

impl VecTree {
    pub(crate) fn new() -> Self {
        Self { nodes: Vec::new() }
    }
    pub(crate) fn push(&mut self, n: CanvasNode) -> NodeId {
        let id = n.id;
        self.nodes.push(n);
        id
    }
    pub(crate) fn get(&self, id: NodeId) -> &CanvasNode {
        self.nodes.iter().find(|n| n.id == id).unwrap()
    }
}

impl LayoutTree for VecTree {
    fn node(&self, id: NodeId) -> Option<&CanvasNode> {
        self.nodes.iter().find(|n| n.id == id)
    }
    fn node_mut(&mut self, id: NodeId) -> Option<&mut CanvasNode> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }
    fn children(&self, parent: NodeId) -> Vec<NodeId> {
        self.nodes
            .iter()
            .filter(|n| n.parent == Some(parent))
            .map(|n| n.id)
            .collect()
    }
}

/// A fixed-extent "measurer": every text node measures to `[w_per_char *
/// chars, line_h]` so auto-width tests are deterministic without a shaper.
pub(crate) fn fake_measure(w_per_char: f64, line_h: f64) -> impl FnMut(&TextNode) -> (f64, f64) {
    move |t: &TextNode| (t.content.chars().count() as f64 * w_per_char, line_h)
}

/// Never called: a measurer that panics, to prove a test path touches no text.
pub(crate) fn no_measure(_: &TextNode) -> (f64, f64) {
    panic!("measure should not be called in this test");
}

/// A frame (clipped group) with the given size + auto-layout config.
pub(crate) fn frame(w: f64, h: f64, al: AutoLayout) -> CanvasNode {
    CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([w, h]),
        auto_layout: Some(al),
        ..Default::default()
    }))
}

/// A rectangle child of `parent`, sized `w x h`, at an arbitrary baked transform
/// (the solver should overwrite its position).
pub(crate) fn rect_child(parent: NodeId, w: f64, h: f64) -> CanvasNode {
    let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        w,
        h,
        Color::BLACK,
    )));
    n.parent = Some(parent);
    n.transform = Transform2D::translation(999.0, 999.0); // junk; must be replaced
    n
}

/// The world-free local-box origin of a node after layout (transform applied to
/// its local box origin) — i.e. the child's top-left in the frame's local space.
pub(crate) fn placed_origin(tree: &VecTree, id: NodeId) -> [f64; 2] {
    let n = tree.get(id);
    let b = super::size::local_box(n);
    let o = n
        .transform
        .transform_point(glam::DVec2::new(b.origin[0], b.origin[1]));
    [o.x, o.y]
}

pub(crate) fn placed_size(tree: &VecTree, id: NodeId) -> [f64; 2] {
    super::size::local_box(tree.get(id)).size
}

/// The WORLD-space top-left of a node's local box after layout: compose every
/// ancestor's transform (root-last) down to the node, then map its local-box
/// origin. Lets a nested-grid test assert a leaf's absolute position across
/// multiple auto-layout levels (a single [`placed_origin`] is local-only).
pub(crate) fn world_origin(tree: &VecTree, id: NodeId) -> [f64; 2] {
    // Walk up to the root, collecting the node and every ancestor (bottom-up).
    let mut chain: Vec<NodeId> = vec![id];
    let mut cur = tree.get(id).parent;
    while let Some(p) = cur {
        chain.push(p);
        cur = tree.get(p).parent;
    }
    // Compose root→node: world = T_root ∘ … ∘ T_node.
    let mut world = Transform2D::IDENTITY;
    for &nid in chain.iter().rev() {
        world = tree.get(nid).transform.then(&world);
    }
    let b = super::size::local_box(tree.get(id));
    let o = world.transform_point(glam::DVec2::new(b.origin[0], b.origin[1]));
    [o.x, o.y]
}

pub(crate) const EPS: f64 = 1e-6;

pub(crate) fn approx(a: [f64; 2], b: [f64; 2]) {
    assert!(
        (a[0] - b[0]).abs() < EPS && (a[1] - b[1]).abs() < EPS,
        "expected {b:?}, got {a:?}"
    );
}
