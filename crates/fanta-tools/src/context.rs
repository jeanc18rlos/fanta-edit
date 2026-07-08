//! [`ToolContext`] — the shared bag every tool reads from and writes to.
//!
//! ## Why a single context value
//!
//! Each tool needs access to four things: the live [`Doc`], the [`Viewport`]
//! (for screen↔world conversion), a [`SnapEngine`], and the current screen
//! size. Bundling them into one borrow makes tool signatures uniform — there
//! is exactly one mutable reference threaded through every call — and means
//! the eventual shell in `fanta-app` constructs one of these per gesture
//! instead of passing four arguments through every layer.
//!
//! The doc handle is `&mut Doc` rather than copies of its sub-fields because
//! [`Doc::apply`] is the single chokepoint for undo + history + modified-time
//! bookkeeping. Tools never reach past it.

use fanta_canvas::{SnapEngine, screen_to_world, world_to_screen};
use fanta_doc::{CanvasNode, Color, Doc, NodeId, Transform2D, Viewport};
use glam::DVec2;

/// Neutral fill a shape-creation tool uses when the shell doesn't specify one
/// (e.g. in unit tests that call [`ToolContext::new`] directly).
pub const DEFAULT_NEW_SHAPE_FILL: Color = Color::rgb(0xCC, 0xCC, 0xCC);

/// Per-gesture shared state. Constructed by the shell (`fanta-app`) and
/// re-used across every event in a single drag.
///
/// The fields are public because the tool layer is a peer to its caller — we
/// don't gain anything by hiding them, and tools occasionally need to inspect
/// the snap engine's settings to decide whether to invoke it (e.g. respect a
/// "disable snap while space is held" UI affordance).
pub struct ToolContext<'a> {
    /// Mutable handle on the document. Tools apply ops through
    /// [`Doc::apply`]; never via `doc.scene` directly. Direct scene access
    /// bypasses history and is reserved for migrations.
    pub doc: &'a mut Doc,

    /// Current viewport — mutated by [`crate::hand::HandTool`] and by the
    /// shell's zoom handlers. Other tools read it for screen↔world math.
    pub viewport: &'a mut Viewport,

    /// Snap engine handle. The shell rebuilds this per-frame with the current
    /// viewport zoom so screen-pixel thresholds stay correct.
    pub snap: SnapEngine,

    /// Size of the viewport rectangle in screen pixels. Required for the
    /// world↔screen mapping; not derived from `Viewport` because viewport-as-
    /// world-state is decoupled from window dimensions on purpose (zoom
    /// behavior shouldn't change when the window is resized).
    pub screen_size: DVec2,

    /// Fill color a shape-creation tool (rect / ellipse / line) gives a newly
    /// drawn node. The shell sets this from the active palette swatch before
    /// dispatch, so the `CreateNode` op carries the *final* color — the color
    /// is part of the undoable creation, not a mutation applied afterward.
    /// (Fixes the prior side-channel that recolored outside the history
    /// transaction and lost the color on undo→redo.)
    pub new_shape_fill: Color,

    /// The subtree root the active editor view is scoped to (selection
    /// container-walk, deep hit-test base, and new-node parent). The shell sets
    /// it from `AppState::focus_root()` so a component-edit tab scopes tools to
    /// the component master instead of the page. `None` falls back to
    /// `doc.active_page()` — identical to the project tab's behavior.
    pub scope_root: Option<NodeId>,
}

impl<'a> ToolContext<'a> {
    /// Construct a fresh context. The shell calls this once per gesture; tests
    /// call it directly with synthetic doc / viewport / snap values.
    pub fn new(
        doc: &'a mut Doc,
        viewport: &'a mut Viewport,
        snap: SnapEngine,
        screen_size: DVec2,
    ) -> Self {
        Self {
            doc,
            viewport,
            snap,
            screen_size,
            new_shape_fill: DEFAULT_NEW_SHAPE_FILL,
            scope_root: None,
        }
    }

    /// Builder: set the fill new shapes are created with. The shell calls this
    /// with the active palette color so creation tools bake it into the
    /// `CreateNode` op.
    pub fn with_new_shape_fill(mut self, color: Color) -> Self {
        self.new_shape_fill = color;
        self
    }

    /// Builder: scope the gesture to a subtree root (the active editor view's
    /// `focus_root`). Selection container-walk, deep hit-test, and new-node
    /// parenting use this instead of the active page.
    pub fn with_scope_root(mut self, scope_root: Option<NodeId>) -> Self {
        self.scope_root = scope_root;
        self
    }

    /// The subtree root the gesture is scoped to: the explicit `scope_root`, else
    /// the document's active page (legacy/project behavior).
    pub fn scope(&self) -> Option<NodeId> {
        self.scope_root.or_else(|| self.doc.active_page())
    }

    /// Map a screen-pixel position into world space at the current viewport.
    /// Stable across both the tools and the renderer because both consume the
    /// same `fanta_canvas::viewport` math.
    pub fn screen_to_world(&self, screen: DVec2) -> DVec2 {
        screen_to_world(screen, self.viewport, self.screen_size)
    }

    /// Map a world-space position back into screen pixels. Used by tools that
    /// produce overlays positioned in screen coords (selection handles, marquee
    /// outline) once their underlying data is in world coords.
    pub fn world_to_screen(&self, world: DVec2) -> DVec2 {
        world_to_screen(world, self.viewport, self.screen_size)
    }

    /// Attach a newly-created node to the current scope (the active page, or the
    /// component master root in a component-edit tab), preserving the world
    /// transform it already has. Creation tools author geometry in world space;
    /// imported Figma documents show only `Doc::active_page()` in both the
    /// canvas and Layers, so a root-level shape can exist but appear to vanish.
    ///
    /// The node's existing transform is treated as the desired world transform.
    /// When there is a scope parent, we rebase that transform into the parent's
    /// coordinate space and place the node above its current children.
    /// Legacy/no-page docs keep the old root-level behavior.
    pub fn place_new_node_on_active_page(&self, node: &mut CanvasNode) {
        let parent = self.scope();
        let desired_world = node.transform;
        let parent_world = parent
            .and_then(|p| self.doc.scene.world_transform(p))
            .unwrap_or(Transform2D::IDENTITY);
        node.parent = parent;
        node.index = self.doc.scene.next_child_index(parent);
        node.transform = desired_world.then(&parent_world.inverse());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{CanvasNode, Doc, GroupNode, NodeData, Operation, VectorNode};

    #[test]
    fn screen_to_world_at_identity_viewport_is_centered() {
        let mut doc = Doc::new();
        let mut viewport = Viewport::default();
        let ctx = ToolContext::new(
            &mut doc,
            &mut viewport,
            SnapEngine::default(),
            DVec2::new(800.0, 600.0),
        );
        let center = DVec2::new(400.0, 300.0);
        let world = ctx.screen_to_world(center);
        assert!((world - DVec2::ZERO).length() < 1e-9);
    }

    #[test]
    fn world_to_screen_round_trips_with_screen_to_world() {
        let mut doc = Doc::new();
        let mut viewport = Viewport {
            center: [42.0, -17.0],
            zoom: 2.5,
        };
        let ctx = ToolContext::new(
            &mut doc,
            &mut viewport,
            SnapEngine::default(),
            DVec2::new(1024.0, 768.0),
        );
        let original = DVec2::new(123.0, 456.0);
        let back = ctx.world_to_screen(ctx.screen_to_world(original));
        assert!((back - original).length() < 1e-9);
    }

    #[test]
    fn context_carries_snap_engine_thresholds() {
        let mut doc = Doc::new();
        let mut viewport = Viewport::default();
        let snap = SnapEngine {
            zoom: 2.0,
            ..Default::default()
        };
        let ctx = ToolContext::new(&mut doc, &mut viewport, snap, DVec2::new(800.0, 600.0));
        assert!((ctx.snap.zoom - 2.0).abs() < 1e-9);
    }

    #[test]
    fn place_new_node_parents_to_active_page_without_moving_in_world() {
        let mut doc = Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.transform = Transform2D::translation(500.0, 0.0);
        let page_id = page.id;
        doc.apply(Operation::create_node(page)).unwrap();
        doc.add_page(page_id);
        doc.set_active_page(Some(page_id));

        let mut viewport = Viewport::default();
        let ctx = ToolContext::new(
            &mut doc,
            &mut viewport,
            SnapEngine::default(),
            DVec2::new(800.0, 600.0),
        );
        let mut node = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            10.0,
            20.0,
            30.0,
            40.0,
            DEFAULT_NEW_SHAPE_FILL,
        )));
        ctx.place_new_node_on_active_page(&mut node);
        let id = node.id;
        ctx.doc.apply(Operation::create_node(node)).unwrap();

        assert_eq!(ctx.doc.scene.get(id).unwrap().parent, Some(page_id));
        let bounds = ctx.doc.scene.world_bounds(id).unwrap();
        assert!((bounds.min_x - 10.0).abs() < 1e-9);
        assert!((bounds.min_y - 20.0).abs() < 1e-9);
        assert!((bounds.max_x - 40.0).abs() < 1e-9);
        assert!((bounds.max_y - 60.0).abs() < 1e-9);
    }
}
