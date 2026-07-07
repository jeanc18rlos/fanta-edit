//! Auto-layout solver — Figma "stack" / flexbox layout, Stage 2.
//!
//! Stage 1 ([`crate::node::AutoLayout`]) imported the per-frame stack config and
//! per-child layout props off the `.fig`. This module is the pass that *consumes*
//! them: for every auto-layout frame it positions the flow children along the
//! primary axis, aligns them on the counter axis, distributes leftover space to
//! FILL children, stretches children that ask for it, hugs the frame to its
//! content where the frame's axis sizing is Hug, and snaps auto-width text to its
//! measured glyph width. Free (non-auto-layout) frames keep their baked relative
//! transforms untouched.
//!
//! ## Why it lives here (Skia-free)
//!
//! The doc layer is renderer-agnostic ([`crate::lib`] crate docs) and must not
//! grow a Skia/text-shaping dependency. Text measurement (the auto-width case)
//! lives in `fanta-text`, which links Skia — so the solver takes an **injected
//! measure closure** ([`Measure`]) instead of importing it. The pure algorithm is
//! unit-testable with a fake metric; the two real consumers
//! ([`crate::snapshot::SceneSnapshot`] and `fanta-render`'s expansion path) inject
//! a measurer (a metric approximation for the oracle, the real shaper for the
//! live render).
//!
//! ## Tree abstraction
//!
//! Both consumers hold a *collection* of [`CanvasNode`]s linked by `parent`: the
//! live [`crate::scene::Scene`] (a map) and an instance's transient expansion (a
//! `Vec<ExpandedNode>`). The solver works against either through the
//! [`LayoutTree`] trait — get a node, get it mutably, list a node's children in
//! z-order — so the single algorithm services both. The walk is BOTTOM-UP
//! (children laid out / hug-sized before parents) so nested hug frames resolve
//! before the frames that contain them.

mod size;

use crate::id::NodeId;
use crate::node::{
    AutoLayout, AxisSizing, CanvasNode, CounterAlign, LayoutMode, NodeData, PrimaryAlign,
    TextAutoResize, TextNode,
};
use crate::transform::Transform2D;
use glam::{DMat2, DVec2};
use size::{LocalBox, local_box, set_size};

/// Measures a text node's single-line glyph extent `(width, height)` — the size
/// an auto-width ([`TextAutoResize::WidthAndHeight`]) label needs so it never
/// wraps. Injected so the doc layer stays Skia-free: the oracle passes a metric
/// approximation, the renderer passes the real `fanta-text` shaper.
pub type Measure<'a> = dyn FnMut(&TextNode) -> (f64, f64) + 'a;

/// A mutable tree of [`CanvasNode`]s the solver can lay out: random-access by id,
/// mutable access by id, and a node's children in ascending z-order. Implemented
/// for the live [`crate::scene::Scene`] and for an instance's transient expansion
/// so the one algorithm services both consumers.
pub trait LayoutTree {
    /// The node for `id`, or `None` if it is not in the tree.
    fn node(&self, id: NodeId) -> Option<&CanvasNode>;
    /// Mutable node for `id`, or `None`.
    fn node_mut(&mut self, id: NodeId) -> Option<&mut CanvasNode>;
    /// Ids of `parent`'s direct children, ascending z-order (paint order).
    fn children(&self, parent: NodeId) -> Vec<NodeId>;
}

/// The live scene is a layout tree: ids map to nodes, `children_of` already
/// returns z-ordered ids. `node_mut` goes through [`crate::scene::Scene::get_mut`]
/// (which clears the world-transform / local-bounds caches), so geometry the
/// solver writes is reflected by the next `world_transform`/`world_bounds` read.
impl LayoutTree for crate::scene::Scene {
    fn node(&self, id: NodeId) -> Option<&CanvasNode> {
        self.get(id)
    }
    fn node_mut(&mut self, id: NodeId) -> Option<&mut CanvasNode> {
        self.get_mut(id)
    }
    fn children(&self, parent: NodeId) -> Vec<NodeId> {
        self.children_of(Some(parent)).to_vec()
    }
}

/// A [`LayoutTree`] over an instance's transient expansion (`&mut [ExpandedNode]`).
/// Children are derived from each clone's `parent` (fresh ids), z-ordered by the
/// node's `index` to match the renderer's paint order. Used by both consumers to
/// solve auto-layout *inside* an expanded component subtree before walking it.
pub struct ExpandedTree<'a> {
    nodes: &'a mut [crate::resolve::ExpandedNode],
}

impl<'a> ExpandedTree<'a> {
    /// Wrap an expansion for layout. Solve from its root (the entry with an empty
    /// `def_path`), then read back the laid-out clones.
    pub fn new(nodes: &'a mut [crate::resolve::ExpandedNode]) -> Self {
        Self { nodes }
    }

    /// The id of the expansion root (empty `def_path`), if present.
    pub fn root_id(&self) -> Option<NodeId> {
        self.nodes
            .iter()
            .find(|e| e.def_path.is_empty())
            .map(|e| e.node.id)
    }
}

impl LayoutTree for ExpandedTree<'_> {
    fn node(&self, id: NodeId) -> Option<&CanvasNode> {
        self.nodes.iter().find(|e| e.node.id == id).map(|e| &e.node)
    }
    fn node_mut(&mut self, id: NodeId) -> Option<&mut CanvasNode> {
        self.nodes
            .iter_mut()
            .find(|e| e.node.id == id)
            .map(|e| &mut e.node)
    }
    fn children(&self, parent: NodeId) -> Vec<NodeId> {
        let mut kids: Vec<(crate::index::IndexKey, NodeId)> = self
            .nodes
            .iter()
            .filter(|e| e.node.parent == Some(parent))
            .map(|e| (e.node.index, e.node.id))
            .collect();
        kids.sort_by_key(|a| a.0);
        kids.into_iter().map(|(_, id)| id).collect()
    }
}

/// Solve auto-layout over a whole instance expansion, from its root. A
/// convenience for the consumers: wrap the `Vec`, solve from the root id, leave
/// the laid-out clones in place. No-op if the expansion has no root.
pub fn solve_expanded(nodes: &mut [crate::resolve::ExpandedNode], measure: &mut Measure) {
    let mut tree = ExpandedTree::new(nodes);
    if let Some(root) = tree.root_id() {
        solve_auto_layout(&mut tree, root, measure);
    }
}

/// Run the auto-layout pass over the subtree rooted at `root`, mutating child
/// transforms + sizes (and hug-frame sizes) in place. Processes BOTTOM-UP: every
/// descendant frame is laid out (and hug-sized) before the frame that contains
/// it, so a parent reads already-resolved child extents. Non-auto-layout frames
/// recurse into their children but never reposition them; auto-width text is
/// snapped to its measured width wherever it sits.
///
/// `measure` supplies single-line text extents for auto-width labels. See
/// [`Measure`].
pub fn solve_auto_layout<T: LayoutTree>(tree: &mut T, root: NodeId, measure: &mut Measure) {
    solve_node(tree, root, measure);
}

/// Lay out one node's subtree bottom-up, returning nothing — the node's own box
/// (size/transform) is mutated in place. Children are solved first (so their
/// extents are final), then, if this node is an auto-layout frame, its flow
/// children are positioned/sized and the frame is hugged where asked.
fn solve_node<T: LayoutTree>(tree: &mut T, id: NodeId, measure: &mut Measure) {
    // Snap auto-width text to its measured glyph box first — this is the child's
    // own intrinsic size and must be final before a parent flows it.
    apply_text_autoresize(tree, id, measure);

    // Bottom-up: solve every child's subtree before laying this node out.
    let children = tree.children(id);
    for &child in &children {
        solve_node(tree, child, measure);
    }

    // Only auto-layout frames flow their children. Plain frames/groups keep the
    // children's baked transforms — we already recursed into them above.
    let Some(al) = auto_layout_of(tree, id) else {
        return;
    };
    layout_frame(tree, id, &al, &children);
}

/// The [`AutoLayout`] config of `id` if it is an auto-layout frame, else `None`.
fn auto_layout_of<T: LayoutTree>(tree: &T, id: NodeId) -> Option<AutoLayout> {
    match &tree.node(id)?.data {
        NodeData::Group(g) => g.auto_layout,
        _ => None,
    }
}

/// Re-flow an auto-layout child whose size its parent just changed (a counter-axis
/// Stretch or a primary-axis FILL/grow).
///
/// Layout is bottom-up: a child is solved at its own INTRINSIC (Hug) size before
/// the parent resizes it, so its flow children are still packed for the old size.
/// That is invisible until the same master is instanced at a different width — a
/// 720-wide instance of a 400-wide composer master stretches the control-row
/// frame to 720 but leaves its send button bunched where the 400-wide solve put
/// it, with dead space on the right. Here we redistribute the child's own flow
/// children across its new extent, forcing both axes to Fixed so the frame keeps
/// the parent-assigned size instead of hugging back to its content. `layout_frame`
/// runs this same pass on grandchildren, so nesting re-flows recursively.
fn reflow_after_resize<T: LayoutTree>(tree: &mut T, id: NodeId) {
    let Some(mut al) = auto_layout_of(tree, id) else {
        return;
    };
    al.primary_sizing = AxisSizing::Fixed;
    al.counter_sizing = AxisSizing::Fixed;
    let children = tree.children(id);
    layout_frame(tree, id, &al, &children);
}

/// Snap an auto-width text node to its measured single-line glyph box (so a label
/// like "Edit"/"Copy" never wraps). `Height`-mode and `None`-mode text keep their
/// authored width; only `WidthAndHeight` (Figma "Auto width") is overwritten.
fn apply_text_autoresize<T: LayoutTree>(tree: &mut T, id: NodeId, measure: &mut Measure) {
    let new_size = match tree.node(id).map(|n| &n.data) {
        Some(NodeData::Text(t)) if t.auto_resize == TextAutoResize::WidthAndHeight => {
            let (w, h) = measure(t);
            Some([w.max(0.0), h.max(0.0)])
        }
        _ => None,
    };
    if let (Some([w, h]), Some(node)) = (new_size, tree.node_mut(id)) {
        if let NodeData::Text(t) = &mut node.data {
            t.local_size = [w, h];
        }
    }
}

// =============================================================================
// Per-frame layout
// =============================================================================

/// Resolved per-child geometry the layout collects before writing it back.
struct ChildInfo {
    id: NodeId,
    /// Pre-transform local box (origin + size) as currently sized.
    bx: LocalBox,
    /// Flex grow factor along the primary axis (0 ⇒ no grow).
    grow: f64,
    /// Counter-axis alignment (the child's `align_self`, else the frame default).
    counter_align: CounterAlign,
    /// Whether the child is absolutely positioned (excluded from the flow).
    absolute: bool,
    /// The child's current transform linear part (matrix2) — preserved so a
    /// scaled/rotated child keeps its orientation while we set its translation.
    matrix: DMat2,
    /// The translation the layout computed for this child (`None` until placed).
    /// `info_transform` rebuilds the final transform from `matrix` + this.
    computed_translation: Option<DVec2>,
}

fn gather_child_infos<T: LayoutTree>(
    tree: &T,
    al: &AutoLayout,
    children: &[NodeId],
) -> Vec<ChildInfo> {
    let mut infos = Vec::with_capacity(children.len());
    if al.flow_reverse {
        for &cid in children.iter().rev() {
            push_child_info(tree, al, cid, &mut infos);
        }
    } else {
        for &cid in children {
            push_child_info(tree, al, cid, &mut infos);
        }
    }
    infos
}

fn push_child_info<T: LayoutTree>(
    tree: &T,
    al: &AutoLayout,
    cid: NodeId,
    infos: &mut Vec<ChildInfo>,
) {
    let Some(node) = tree.node(cid) else { return };
    let lc = if al.child_layout {
        node.layout_child
    } else {
        None
    };
    infos.push(ChildInfo {
        id: cid,
        bx: local_box(node),
        grow: lc.map(|l| l.grow as f64).unwrap_or(0.0),
        counter_align: lc.and_then(|l| l.align_self).unwrap_or(al.counter_align),
        absolute: lc.map(|l| l.absolute).unwrap_or(false),
        matrix: node.transform.0.matrix2,
        computed_translation: None,
    });
}

#[derive(Clone, Copy)]
struct FrameFlow {
    horizontal: bool,
    pad_main_lo: f64,
    pad_main_hi: f64,
    pad_cross_lo: f64,
    pad_cross_hi: f64,
    inner_main: f64,
    inner_cross: f64,
}

impl FrameFlow {
    fn new(al: &AutoLayout, frame: LocalBox) -> Self {
        let horizontal = al.mode == LayoutMode::Horizontal;
        let [pad_t, pad_r, pad_b, pad_l] = al.padding;
        let (pad_main_lo, pad_main_hi, pad_cross_lo, pad_cross_hi) = if horizontal {
            (pad_l, pad_r, pad_t, pad_b)
        } else {
            (pad_t, pad_b, pad_l, pad_r)
        };
        let frame_main = main_of(frame.size, horizontal);
        let frame_cross = cross_of(frame.size, horizontal);
        Self {
            horizontal,
            pad_main_lo,
            pad_main_hi,
            pad_cross_lo,
            pad_cross_hi,
            inner_main: (frame_main - pad_main_lo - pad_main_hi).max(0.0),
            inner_cross: (frame_cross - pad_cross_lo - pad_cross_hi).max(0.0),
        }
    }

    fn target(self, main: f64, cross: f64) -> DVec2 {
        if self.horizontal {
            DVec2::new(main, cross)
        } else {
            DVec2::new(cross, main)
        }
    }
}

#[derive(Clone, Copy)]
struct RunMetrics {
    content_main: f64,
    packed: f64,
}

fn collect_flow_indices(infos: &[ChildInfo]) -> Vec<usize> {
    (0..infos.len()).filter(|&i| !infos[i].absolute).collect()
}

fn run_metrics(infos: &[ChildInfo], run: &[usize], horizontal: bool, spacing: f64) -> RunMetrics {
    let content_main: f64 = run
        .iter()
        .map(|&i| main_of(infos[i].bx.size, horizontal))
        .sum();
    let packed = content_main + spacing * run.len().saturating_sub(1) as f64;
    RunMetrics {
        content_main,
        packed,
    }
}

fn run_max_cross(infos: &[ChildInfo], run: &[usize], horizontal: bool) -> f64 {
    run.iter()
        .map(|&i| cross_of(infos[i].bx.size, horizontal))
        .fold(0.0_f64, f64::max)
}

fn counter_alignment_span(
    al: &AutoLayout,
    flow: FrameFlow,
    infos: &[ChildInfo],
    run: &[usize],
) -> f64 {
    if al.counter_sizing == AxisSizing::Hug {
        run_max_cross(infos, run, flow.horizontal)
    } else {
        flow.inner_cross
    }
}

fn distribute_grow_on_run(
    infos: &mut [ChildInfo],
    run: &[usize],
    flow: FrameFlow,
    spacing: f64,
    enabled: bool,
) {
    if !enabled {
        return;
    }
    let grow_count = run.iter().filter(|&&i| infos[i].grow > 0.0).count() as f64;
    if grow_count <= 0.0 {
        return;
    }

    let fixed_used: f64 = run
        .iter()
        .filter(|&&i| infos[i].grow <= 0.0)
        .map(|&i| main_of(infos[i].bx.size, flow.horizontal))
        .sum();
    let gaps = spacing * run.len().saturating_sub(1) as f64;
    let each = (flow.inner_main - fixed_used - gaps).max(0.0) / grow_count;
    for &i in run {
        if infos[i].grow > 0.0 {
            set_main(&mut infos[i].bx.size, flow.horizontal, each);
        }
    }
}

fn stretch_run_counter(
    infos: &mut [ChildInfo],
    run: &[usize],
    horizontal: bool,
    counter_size: f64,
) {
    for &i in run {
        if infos[i].counter_align == CounterAlign::Stretch {
            set_cross(&mut infos[i].bx.size, horizontal, counter_size);
        }
    }
}

fn primary_cursor(
    al: &AutoLayout,
    flow: FrameFlow,
    metrics: RunMetrics,
    count: usize,
) -> (f64, f64) {
    let align_main = if al.primary_sizing == AxisSizing::Hug {
        metrics.packed
    } else {
        flow.inner_main
    };
    let free_main = (align_main - metrics.packed).max(0.0);
    match al.primary_align {
        PrimaryAlign::Start => (flow.pad_main_lo, al.spacing),
        PrimaryAlign::Center => (flow.pad_main_lo + free_main * 0.5, al.spacing),
        PrimaryAlign::End => (flow.pad_main_lo + free_main, al.spacing),
        PrimaryAlign::SpaceBetween if count > 1 => {
            let free = (align_main - metrics.content_main).max(0.0);
            (flow.pad_main_lo, free / (count - 1) as f64)
        }
        PrimaryAlign::SpaceBetween => (flow.pad_main_lo, al.spacing),
    }
}

fn place_run(
    infos: &mut [ChildInfo],
    run: &[usize],
    al: &AutoLayout,
    flow: FrameFlow,
    cross_origin: f64,
    cross_size: f64,
) {
    let metrics = run_metrics(infos, run, flow.horizontal, al.spacing);
    let (mut cursor, gap) = primary_cursor(al, flow, metrics, run.len());
    for &i in run {
        let child_cross = cross_of(infos[i].bx.size, flow.horizontal);
        let cross_pos =
            cross_origin + counter_offset(infos[i].counter_align, cross_size, child_cross);
        place_child(&mut infos[i], flow.target(cursor, cross_pos));
        cursor += main_of(infos[i].bx.size, flow.horizontal) + gap;
    }
}

fn break_wrap_lines(
    infos: &[ChildInfo],
    flow_indices: &[usize],
    flow: FrameFlow,
    spacing: f64,
) -> Vec<Vec<usize>> {
    let mut lines: Vec<Vec<usize>> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    let mut cur_main = 0.0_f64;
    for &i in flow_indices {
        let child_main = main_of(infos[i].bx.size, flow.horizontal);
        if cur.is_empty() {
            cur.push(i);
            cur_main = child_main;
            continue;
        }
        let next = cur_main + spacing + child_main;
        if flow.inner_main > 0.0 && next > flow.inner_main + 1e-6 {
            lines.push(std::mem::take(&mut cur));
            cur.push(i);
            cur_main = child_main;
        } else {
            cur.push(i);
            cur_main = next;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

fn distribute_grow_on_lines(
    infos: &mut [ChildInfo],
    lines: &[Vec<usize>],
    flow: FrameFlow,
    spacing: f64,
    enabled: bool,
) {
    for line in lines {
        distribute_grow_on_run(infos, line, flow, spacing, enabled);
    }
}

fn line_thicknesses(infos: &[ChildInfo], lines: &[Vec<usize>], horizontal: bool) -> Vec<f64> {
    lines
        .iter()
        .map(|line| run_max_cross(infos, line, horizontal))
        .collect()
}

fn stretch_lines_counter(
    infos: &mut [ChildInfo],
    lines: &[Vec<usize>],
    line_thickness: &[f64],
    horizontal: bool,
) {
    for (line, &thickness) in lines.iter().zip(line_thickness) {
        stretch_run_counter(infos, line, horizontal, thickness);
    }
}

fn place_wrap_lines(
    infos: &mut [ChildInfo],
    lines: &[Vec<usize>],
    line_thickness: &[f64],
    al: &AutoLayout,
    flow: FrameFlow,
) {
    let mut cross_cursor = flow.pad_cross_lo;
    for (line, &thickness) in lines.iter().zip(line_thickness) {
        place_run(infos, line, al, flow, cross_cursor, thickness);
        cross_cursor += thickness + al.counter_spacing;
    }
}

fn widest_line_main(
    infos: &[ChildInfo],
    lines: &[Vec<usize>],
    horizontal: bool,
    spacing: f64,
) -> f64 {
    lines
        .iter()
        .map(|line| run_metrics(infos, line, horizontal, spacing).packed)
        .fold(0.0_f64, f64::max)
}

fn total_line_cross(line_thickness: &[f64], counter_spacing: f64) -> f64 {
    line_thickness.iter().sum::<f64>()
        + counter_spacing * line_thickness.len().saturating_sub(1) as f64
}

fn write_flow_children<T: LayoutTree>(tree: &mut T, infos: &[ChildInfo], flow_indices: &[usize]) {
    for &i in flow_indices {
        let info = &infos[i];
        if let Some(node) = tree.node_mut(info.id) {
            set_size(node, info.bx.size[0], info.bx.size[1]);
            node.transform = info_transform(info);
        }
    }
}

fn reflow_resized_flow_children<T: LayoutTree>(
    tree: &mut T,
    infos: &[ChildInfo],
    flow_indices: &[usize],
) {
    for &i in flow_indices {
        if infos[i].counter_align == CounterAlign::Stretch || infos[i].grow > 0.0 {
            reflow_after_resize(tree, infos[i].id);
        }
    }
}

fn set_hugged_frame_size<T: LayoutTree>(
    tree: &mut T,
    frame_id: NodeId,
    frame_size: [f64; 2],
    al: &AutoLayout,
    flow: FrameFlow,
    content_main: f64,
    content_cross: f64,
) {
    let mut new_frame = frame_size;
    if al.primary_sizing == AxisSizing::Hug {
        let hug_main = content_main + flow.pad_main_lo + flow.pad_main_hi;
        set_main(&mut new_frame, flow.horizontal, hug_main);
    }
    if al.counter_sizing == AxisSizing::Hug {
        let hug_cross = content_cross + flow.pad_cross_lo + flow.pad_cross_hi;
        set_cross(&mut new_frame, flow.horizontal, hug_cross);
    }
    if new_frame != frame_size {
        if let Some(node) = tree.node_mut(frame_id) {
            set_frame_size(node, new_frame);
        }
    }
}

/// Lay out one auto-layout frame's flow children: place + size them along both
/// axes per the [`AutoLayout`] config, distribute FILL space, stretch where
/// asked, then hug the frame to its content on any Hug axis.
fn layout_frame<T: LayoutTree>(
    tree: &mut T,
    frame_id: NodeId,
    al: &AutoLayout,
    children: &[NodeId],
) {
    // Wrapping frames flow onto multiple rows/columns — a distinct enough
    // algorithm that it lives in its own function. A non-wrap frame (the common
    // case) continues through the single-line pipeline below.
    if al.wrap {
        layout_frame_wrap(tree, frame_id, al, children);
        return;
    }

    // Gather the child boxes once. Absolute children are recorded (so we can skip
    // them) but never flowed.
    let mut infos = gather_child_infos(tree, al, children);

    // Frame inner box. Padding is [top, right, bottom, left].
    let frame = frame_box(tree, frame_id);
    let flow = FrameFlow::new(al, frame);

    // Flow children only.
    let flow_indices = collect_flow_indices(&infos);

    // On a HUG counter axis the frame's authored counter extent is not what the
    // children align against — the frame *becomes* its content. Resolve the inner
    // counter size to the max child extent up front so Center/End/Stretch place
    // correctly relative to the hugged box (Start is unaffected). The symmetric
    // HUG-primary case (no free space to justify into) is handled at primary
    // placement below via `align_main`.
    let inner_cross = counter_alignment_span(al, flow, &infos, &flow_indices);

    // ---- FILL distribution on the primary axis ------------------------------
    // grow>0 children share the primary space left over by the FIXED (non-grow)
    // children equally, and each grow child is *assigned* that share as its final
    // primary extent — its own authored base extent is discarded. This matches
    // Figma/Yoga/OpenPencil (`fillSize = remainingMain / fillCount` in the oracle,
    // where `remainingMain = avail - fixedTotal - gaps` counts only the non-fill
    // children): two FILL children sharing a frame end up the SAME size regardless
    // of their authored bases, not `base_i + share` (which would preserve a stale
    // base difference). Distributing by count, not grow weight, also mirrors Figma.
    //
    // FILL needs a fixed primary extent to fill into: on a HUG primary axis the
    // frame collapses to its content, so there is no slack and Figma treats grow
    // as a no-op (the child keeps its base extent). Skip distribution there —
    // otherwise the junk authored main extent would balloon the child.
    distribute_grow_on_run(
        &mut infos,
        &flow_indices,
        flow,
        al.spacing,
        al.primary_sizing != AxisSizing::Hug,
    );

    // ---- Counter-axis Stretch -----------------------------------------------
    // A Stretch child fills the frame's counter inner size (origin pinned at the
    // counter padding-start, so its box spans the inner cross extent).
    stretch_run_counter(&mut infos, &flow_indices, flow.horizontal, inner_cross);

    // ---- Primary-axis placement ---------------------------------------------
    // On a HUG primary axis the frame's authored main extent is junk — the frame
    // *becomes* its packed content, so there is no free space to distribute.
    // `place_run` collapses Center/End/SpaceBetween to Start in that case,
    // rather than placing children against the junk authored extent.
    place_run(
        &mut infos,
        &flow_indices,
        al,
        flow,
        flow.pad_cross_lo,
        inner_cross,
    );
    let metrics = run_metrics(&infos, &flow_indices, flow.horizontal, al.spacing);

    // ---- Write back: sizes (FILL/Stretch may have changed them) + transforms.
    write_flow_children(tree, &infos, &flow_indices);

    // ---- Re-flow resized auto-layout children at their new size. -------------
    // A child we just Stretched or FILLed that is itself an auto-layout frame had
    // its flow children packed for its old (Hug) extent; redistribute them across
    // the new size so e.g. a wider instance pushes its trailing controls to the
    // far edge instead of leaving them bunched. See [`reflow_after_resize`].
    reflow_resized_flow_children(tree, &infos, &flow_indices);

    // ---- Hug sizing: resize the frame to its content on any Hug axis. --------
    // Primary hug = packed run + both primary paddings. Counter hug = max child
    // counter extent + both counter paddings. Children were already positioned
    // above; for primary hug the run starts at pad_main_lo (Start packing), which
    // is exactly what a hug frame uses.
    let max_cross = run_max_cross(&infos, &flow_indices, flow.horizontal);
    set_hugged_frame_size(
        tree,
        frame_id,
        frame.size,
        al,
        flow,
        metrics.packed,
        max_cross,
    );
}

/// Lay out a WRAPPING auto-layout frame (Figma `stackWrap == WRAP`): flow
/// children along the primary axis, breaking to a new row (horizontal) or column
/// (vertical) whenever the next child would overflow the frame's inner primary
/// extent. Rows/columns ("lines") stack on the counter axis separated by
/// [`AutoLayout::counter_spacing`].
///
/// Semantics mirror Figma / CSS flex-wrap:
/// - **Line breaking** is greedy: a child starts a new line when placing it (plus
///   the inter-child `spacing`) would push the line's packed primary run past
///   `inner_main`. The first child of a line never breaks (a single oversized
///   child occupies its own line).
/// - **FILL (`grow`)** distributes each line's leftover primary space among that
///   line's growing children (per-line, like flexbox).
/// - **Primary alignment** ([`PrimaryAlign`]) is applied independently within each
///   line against `inner_main`.
/// - **Counter alignment** ([`CounterAlign`]) positions a child within its OWN
///   line's thickness (the max child cross extent on that line); `Stretch` fills
///   the line thickness.
/// - **Hug**: primary hug → the widest line's packed run; counter hug → the sum of
///   line thicknesses plus the `counter_spacing` gaps.
///
/// Absolute children are excluded from the flow (left untouched), exactly as in
/// the non-wrap path.
fn layout_frame_wrap<T: LayoutTree>(
    tree: &mut T,
    frame_id: NodeId,
    al: &AutoLayout,
    children: &[NodeId],
) {
    let mut infos = gather_child_infos(tree, al, children);

    let frame = frame_box(tree, frame_id);
    let flow = FrameFlow::new(al, frame);

    let flow_indices = collect_flow_indices(&infos);

    // ---- Break the flow into greedy lines on the primary axis. --------------
    // A line is a contiguous run of `flow` indices whose packed primary extent
    // (children + inter-child spacing) fits within `inner_main`. When primary hug
    // is set the frame has no fixed primary extent to wrap against, so everything
    // stays on one line (a hug-primary wrap frame is degenerate — Figma disables
    // wrap unless the primary axis is Fixed); we treat `inner_main <= 0` the same.
    let lines = break_wrap_lines(&infos, &flow_indices, flow, al.spacing);

    // ---- Per-line FILL distribution on the primary axis. --------------------
    // Each line's grow children are *assigned* an equal share of that line's
    // leftover (after its fixed children + gaps), discarding their authored base —
    // identical semantics to the non-wrap path, applied per line like flexbox.
    // Skip on a HUG primary axis: with no fixed extent there is no slack to FILL,
    // and the junk authored extent would otherwise balloon growing children.
    distribute_grow_on_lines(
        &mut infos,
        &lines,
        flow,
        al.spacing,
        al.primary_sizing != AxisSizing::Hug,
    );

    // ---- Each line's thickness (max child cross extent on that line). -------
    let line_thickness = line_thicknesses(&infos, &lines, flow.horizontal);

    // ---- Counter-axis Stretch: a stretched child fills its OWN line's
    // thickness (so each row's stretch children are the same height as that row).
    stretch_lines_counter(&mut infos, &lines, &line_thickness, flow.horizontal);

    // ---- Place each line: counter offset accumulates down the lines; within a
    // line the primary placement honors PrimaryAlign against inner_main.
    place_wrap_lines(&mut infos, &lines, &line_thickness, al, flow);

    // ---- Write back sizes + transforms for the flow children. ---------------
    write_flow_children(tree, &infos, &flow_indices);

    // Re-flow resized auto-layout children at their new size (see the non-wrap
    // path and [`reflow_after_resize`]).
    reflow_resized_flow_children(tree, &infos, &flow_indices);

    // ---- Hug sizing. Primary hug → widest line's packed run; counter hug →
    // total line thickness + counter gaps. Both plus their paddings.
    set_hugged_frame_size(
        tree,
        frame_id,
        frame.size,
        al,
        flow,
        widest_line_main(&infos, &lines, flow.horizontal, al.spacing),
        total_line_cross(&line_thickness, al.counter_spacing),
    );
}

/// The frame's own box (size in local space). A frame always carries a
/// `clip_size` from the importer; if it somehow doesn't, fall back to its
/// content-free zero box (the layout then degenerates to placing at the origin).
fn frame_box<T: LayoutTree>(tree: &T, frame_id: NodeId) -> LocalBox {
    tree.node(frame_id)
        .map(local_box)
        .unwrap_or(LocalBox::zero())
}

/// Set a frame's own size. A frame is a clipped group; resize its `clip_size`.
fn set_frame_size(node: &mut CanvasNode, size: [f64; 2]) {
    if let NodeData::Group(g) = &mut node.data {
        g.clip_size = Some(size);
    } else {
        set_size(node, size[0], size[1]);
    }
}

/// Compute the child's transform so its local-box origin lands at `target` (in
/// the frame's local space), preserving the child's linear part (scale/rotation).
/// translation = target − M·origin, where M is the child's matrix2 — so a child
/// whose path is offset from the origin (a vector) still lands its top-left at
/// `target`.
fn place_child(info: &mut ChildInfo, target: DVec2) {
    let origin = DVec2::new(info.bx.origin[0], info.bx.origin[1]);
    info.computed_translation = Some(target - info.matrix.mul_vec2(origin));
}

/// Rebuild a placed child's final transform from its preserved linear part and
/// the translation the layout computed (identity translation if never placed).
fn info_transform(info: &ChildInfo) -> Transform2D {
    Transform2D(glam::DAffine2 {
        matrix2: info.matrix,
        translation: info.computed_translation.unwrap_or(DVec2::ZERO),
    })
}

// =============================================================================
// Axis helpers
// =============================================================================

#[inline]
fn main_of(size: [f64; 2], horizontal: bool) -> f64 {
    if horizontal { size[0] } else { size[1] }
}

#[inline]
fn cross_of(size: [f64; 2], horizontal: bool) -> f64 {
    if horizontal { size[1] } else { size[0] }
}

#[inline]
fn set_main(size: &mut [f64; 2], horizontal: bool, v: f64) {
    if horizontal {
        size[0] = v;
    } else {
        size[1] = v;
    }
}

#[inline]
fn set_cross(size: &mut [f64; 2], horizontal: bool, v: f64) {
    if horizontal {
        size[1] = v;
    } else {
        size[0] = v;
    }
}

/// Counter-axis offset of a child of cross extent `child_cross` within a cross
/// reference extent `extent` (the inner cross box for a flat frame, the line
/// thickness for a wrapped line), per its [`CounterAlign`].
///
/// `Start`/`Stretch` pin to the cross-start (a stretched child already fills the
/// extent, so its offset is 0). `Center` centers; `End` pins to the cross-end.
///
/// `Baseline` matches the OpenPencil/Yoga oracle, whose layout engine has no
/// font-metrics pipeline and so normalizes `BASELINE` counter-alignment to
/// **end** (`normalizeAlignItems` in `pen-core/src/layout/engine.ts`): for the
/// canonical "big number + small unit" row, bottom-pinning both children to the
/// cross-end is visually indistinguishable from true baseline alignment, whereas
/// the old top-pin (Start) was plainly wrong. We mirror that mapping exactly so
/// a `.fig` importing `stackCounterAlignItems: "BASELINE"` lays out identically.
#[inline]
fn counter_offset(align: CounterAlign, extent: f64, child_cross: f64) -> f64 {
    match align {
        CounterAlign::Start | CounterAlign::Stretch => 0.0,
        CounterAlign::Center => (extent - child_cross) * 0.5,
        CounterAlign::End | CounterAlign::Baseline => extent - child_cross,
    }
}

#[cfg(test)]
mod tests;
