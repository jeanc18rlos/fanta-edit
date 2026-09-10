//! Direct selection of existing vector anchors, handles, and segment endpoints.

use crate::context::ToolContext;
use crate::event::ToolEvent;
use crate::node_edit::NodeEditTool;
use crate::tool::{Tool, ToolOverlay, ToolResponse};
use fanta_doc::Doc;

pub struct PathSelectTool {
    editor: NodeEditTool,
}

impl Default for PathSelectTool {
    fn default() -> Self {
        Self::new()
    }
}

impl PathSelectTool {
    pub fn new() -> Self {
        Self {
            editor: NodeEditTool::for_path_selection(),
        }
    }
}

impl Tool for PathSelectTool {
    fn name(&self) -> &'static str {
        "path_select"
    }

    fn handle_event(&mut self, ctx: &mut ToolContext, event: ToolEvent) -> ToolResponse {
        self.editor.handle_event(ctx, event)
    }

    fn overlays_after_document_change(&self, doc: &Doc) -> Option<Vec<ToolOverlay>> {
        self.editor.overlays_after_document_change(doc)
    }

    fn activate(&mut self, ctx: &mut ToolContext) {
        self.editor.activate(ctx);
    }

    fn deactivate(&mut self, ctx: &mut ToolContext) {
        self.editor.deactivate(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Button, KeyEvent, LogicalKey, ModifierKeys, PointerEvent, ToolOverlay};
    use fanta_canvas::SnapEngine;
    use fanta_doc::{
        CanvasNode, Color, Doc, Fill, GroupNode, NodeData, NodeFlags, NodeId, Operation,
        ParametricShape, PathData, Stroke, Transform2D, VectorNode, Viewport,
    };
    use glam::DVec2;

    fn polyline() -> PathData {
        let mut path = PathData::new();
        path.move_to(0., 0.).line_to(100., 0.).line_to(100., 100.);
        path
    }

    fn add_vector(
        doc: &mut Doc,
        path: PathData,
        parent: Option<NodeId>,
        transform: Transform2D,
    ) -> NodeId {
        let mut node = CanvasNode::new(NodeData::Vector(VectorNode {
            path,
            fills: Default::default(),
            strokes: Default::default(),
            corner_radius: None,
            corner_radii: None,
            corner_smoothing: 0.,
            local_size: None,
            parametric: None,
        }));
        node.parent = parent;
        node.transform = transform;
        let id = node.id;
        doc.apply(Operation::create_node(node))
            .expect("create vector");
        id
    }

    fn path(doc: &Doc, id: NodeId) -> PathData {
        doc.scene
            .get(id)
            .and_then(|node| node.data.as_vector())
            .expect("vector")
            .path
            .clone()
    }

    fn context<'a>(doc: &'a mut Doc, viewport: &'a mut Viewport) -> ToolContext<'a> {
        ToolContext::new(doc, viewport, SnapEngine::default(), DVec2::new(800., 600.))
    }

    fn press(
        tool: &mut PathSelectTool,
        ctx: &mut ToolContext,
        world: DVec2,
        modifiers: ModifierKeys,
    ) -> ToolResponse {
        let screen = ctx.world_to_screen(world).to_array();
        tool.handle_event(
            ctx,
            ToolEvent::Pointer(PointerEvent::Press {
                screen,
                button: Button::Primary,
                modifiers,
                count: 1,
            }),
        )
    }

    fn move_to(
        tool: &mut PathSelectTool,
        ctx: &mut ToolContext,
        world: DVec2,
        modifiers: ModifierKeys,
    ) -> ToolResponse {
        let screen = ctx.world_to_screen(world).to_array();
        tool.handle_event(
            ctx,
            ToolEvent::Pointer(PointerEvent::Move { screen, modifiers }),
        )
    }

    fn release(
        tool: &mut PathSelectTool,
        ctx: &mut ToolContext,
        world: DVec2,
        modifiers: ModifierKeys,
    ) -> ToolResponse {
        let screen = ctx.world_to_screen(world).to_array();
        tool.handle_event(
            ctx,
            ToolEvent::Pointer(PointerEvent::Release {
                screen,
                button: Button::Primary,
                modifiers,
            }),
        )
    }

    fn click(
        tool: &mut PathSelectTool,
        ctx: &mut ToolContext,
        world: DVec2,
        modifiers: ModifierKeys,
    ) -> ToolResponse {
        press(tool, ctx, world, modifiers);
        release(tool, ctx, world, modifiers)
    }

    fn selected_points(response: &ToolResponse) -> Vec<DVec2> {
        response
            .overlays
            .iter()
            .filter_map(|overlay| match overlay {
                ToolOverlay::PathAnchor {
                    world,
                    selected: true,
                } => Some(DVec2::from(*world)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn path_selection_acquires_unselected_straight_strokes_with_screen_tolerance() {
        for zoom in [0.5, 1., 4.] {
            for vertical in [false, true] {
                for offset in [1., 5., 7.] {
                    let mut doc = Doc::new();
                    let end = if vertical {
                        DVec2::new(0., 100.)
                    } else {
                        DVec2::new(100., 0.)
                    };
                    let normal = if vertical { DVec2::X } else { DVec2::Y };
                    let mut original = PathData::new();
                    original.move_to(0., 0.).line_to(end.x, end.y);
                    let id = add_vector(&mut doc, original.clone(), None, Transform2D::IDENTITY);
                    doc.scene
                        .get_mut(id)
                        .expect("vector")
                        .data
                        .as_vector_mut()
                        .expect("vector")
                        .strokes
                        .push(Stroke::solid(Color::BLACK, 1.));
                    doc.history = Default::default();
                    let mut viewport = Viewport {
                        zoom,
                        ..Viewport::default()
                    };
                    let mut ctx = context(&mut doc, &mut viewport);
                    let mut tool = PathSelectTool::new();
                    let response = click(
                        &mut tool,
                        &mut ctx,
                        end * 0.5 + normal * (offset / zoom),
                        ModifierKeys::empty(),
                    );
                    if offset < 6. {
                        assert_eq!(
                            selected_points(&response),
                            vec![DVec2::ZERO, end],
                            "zoom {zoom}, offset {offset}, vertical {vertical}"
                        );
                        assert!(ctx.doc.selection.contains(id));
                    } else {
                        assert!(selected_points(&response).is_empty());
                        assert!(ctx.doc.selection.is_empty());
                    }
                    assert_eq!(path(ctx.doc, id), original);
                    assert_eq!(ctx.doc.history.undo_depth(), 0);
                }
            }
        }
    }

    #[test]
    fn path_selection_segment_tolerance_survives_non_uniform_scale() {
        let mut doc = Doc::new();
        let mut original = PathData::new();
        original.move_to(0., 0.).line_to(100., 100.);
        let transform = Transform2D::scale_xy(10., 0.1);
        let id = add_vector(&mut doc, original.clone(), None, transform);
        doc.history = Default::default();
        let mut viewport = Viewport {
            zoom: 2.,
            ..Viewport::default()
        };
        let mut ctx = context(&mut doc, &mut viewport);
        let mut tool = PathSelectTool::new();
        let response = click(
            &mut tool,
            &mut ctx,
            DVec2::new(500., 7.),
            ModifierKeys::empty(),
        );
        assert_eq!(
            selected_points(&response),
            vec![DVec2::ZERO, DVec2::new(1000., 10.)]
        );
        assert!(ctx.doc.selection.contains(id));
        assert_eq!(path(ctx.doc, id), original);
        assert_eq!(ctx.doc.history.undo_depth(), 0);
    }

    #[test]
    fn path_selection_acquires_a_curve_behind_an_unfilled_vectors_bounds() {
        let mut doc = Doc::new();
        let mut curve = PathData::new();
        curve.move_to(0., 0.).quad_to(50., 100., 100., 0.);
        let id = add_vector(&mut doc, curve.clone(), None, Transform2D::IDENTITY);
        let mut covering_bounds = PathData::new();
        covering_bounds
            .move_to(0., 0.)
            .line_to(100., 0.)
            .line_to(100., 100.);
        let foreground = add_vector(
            &mut doc,
            covering_bounds.clone(),
            None,
            Transform2D::IDENTITY,
        );
        doc.scene.get_mut(foreground).expect("foreground").index =
            fanta_doc::IndexKey::from_raw(2.);
        doc.scene.rebuild_child_index();
        assert_eq!(
            fanta_canvas::hit_test(
                &doc.scene,
                DVec2::new(50., 50.),
                fanta_canvas::HitPrecision::Bounds,
                None
            ),
            Some(foreground)
        );
        doc.history = Default::default();
        let mut viewport = Viewport::default();
        let mut ctx = context(&mut doc, &mut viewport);
        let mut tool = PathSelectTool::new();
        let response = click(
            &mut tool,
            &mut ctx,
            DVec2::new(50., 53.),
            ModifierKeys::empty(),
        );
        assert_eq!(
            selected_points(&response),
            vec![DVec2::ZERO, DVec2::new(100., 0.)]
        );
        assert!(ctx.doc.selection.contains(id));
        assert!(!ctx.doc.selection.contains(foreground));
        assert_eq!(path(ctx.doc, id), curve);
        assert_eq!(path(ctx.doc, foreground), covering_bounds);
        assert_eq!(ctx.doc.history.undo_depth(), 0);
    }

    #[test]
    fn path_selection_does_not_acquire_a_path_outside_a_clipping_parent() {
        let mut doc = Doc::new();
        let parent = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([100., 100.]),
            ..Default::default()
        }));
        let parent_id = parent.id;
        doc.apply(Operation::create_node(parent)).expect("parent");
        let mut original = PathData::new();
        original.move_to(0., 120.).line_to(100., 120.);
        let id = add_vector(
            &mut doc,
            original.clone(),
            Some(parent_id),
            Transform2D::IDENTITY,
        );
        doc.history = Default::default();
        let mut viewport = Viewport::default();
        let mut ctx = context(&mut doc, &mut viewport);
        let mut tool = PathSelectTool::new();
        let response = click(
            &mut tool,
            &mut ctx,
            DVec2::new(50., 120.),
            ModifierKeys::empty(),
        );
        assert!(selected_points(&response).is_empty());
        assert!(ctx.doc.selection.is_empty());
        assert_eq!(path(ctx.doc, id), original);
        assert_eq!(ctx.doc.history.undo_depth(), 0);
    }

    #[test]
    fn path_selection_acquires_overflow_when_frame_clipping_is_disabled() {
        let mut doc = Doc::new();
        let mut parent = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([100., 100.]),
            ..Default::default()
        }));
        parent.meta = serde_json::json!({ "clip_content": false });
        let parent_id = parent.id;
        doc.apply(Operation::create_node(parent)).expect("parent");
        let mut original = PathData::new();
        original.move_to(0., 120.).line_to(100., 120.);
        let id = add_vector(
            &mut doc,
            original.clone(),
            Some(parent_id),
            Transform2D::IDENTITY,
        );
        let mut viewport = Viewport::default();
        let mut ctx = context(&mut doc, &mut viewport);
        let mut tool = PathSelectTool::new();
        let response = click(
            &mut tool,
            &mut ctx,
            DVec2::new(50., 120.),
            ModifierKeys::empty(),
        );
        assert_eq!(
            selected_points(&response),
            vec![DVec2::new(0., 120.), DVec2::new(100., 120.)]
        );
        assert!(ctx.doc.selection.contains(id));
        assert_eq!(path(ctx.doc, id), original);
    }

    #[test]
    fn path_selection_filled_body_drag_preserves_style_and_undo_restores_parametric_shape() {
        let mut doc = Doc::new();
        let descriptor = ParametricShape::Polygon { points: 4 };
        let original = descriptor.to_path(100., 100.);
        let id = add_vector(&mut doc, original.clone(), None, Transform2D::IDENTITY);
        let vector = doc
            .scene
            .get_mut(id)
            .expect("vector")
            .data
            .as_vector_mut()
            .expect("vector");
        vector.parametric = Some(descriptor);
        vector.fills.push(Fill::solid(Color::BLACK));
        vector.strokes.push(Stroke::solid(Color::BLACK, 2.));
        let original_data = vector.clone();
        doc.history = Default::default();
        let mut viewport = Viewport::default();
        let mut ctx = context(&mut doc, &mut viewport);
        let mut tool = PathSelectTool::new();
        let response = press(
            &mut tool,
            &mut ctx,
            DVec2::new(50., 50.),
            ModifierKeys::empty(),
        );
        assert_eq!(selected_points(&response).len(), 4);
        move_to(
            &mut tool,
            &mut ctx,
            DVec2::new(60., 70.),
            ModifierKeys::empty(),
        );
        release(
            &mut tool,
            &mut ctx,
            DVec2::new(60., 70.),
            ModifierKeys::empty(),
        );
        let edited_data = ctx
            .doc
            .scene
            .get(id)
            .expect("vector")
            .data
            .as_vector()
            .expect("vector")
            .clone();
        assert_ne!(edited_data.path, original);
        assert_eq!(edited_data.parametric, None);
        assert_eq!(edited_data.fills, original_data.fills);
        assert_eq!(edited_data.strokes, original_data.strokes);
        assert_eq!(ctx.doc.history.undo_depth(), 1);
        ctx.doc.undo().expect("undo");
        assert_eq!(
            ctx.doc.scene.get(id).expect("vector").data.as_vector(),
            Some(&original_data)
        );
        ctx.doc.redo().expect("redo");
        assert_eq!(
            ctx.doc.scene.get(id).expect("vector").data.as_vector(),
            Some(&edited_data)
        );
    }

    #[test]
    fn path_selection_segment_click_selects_endpoints_without_inserting_or_history() {
        let mut doc = Doc::new();
        let original = polyline();
        let id = add_vector(&mut doc, original.clone(), None, Transform2D::IDENTITY);
        doc.history = Default::default();
        let mut viewport = Viewport::default();
        let mut ctx = context(&mut doc, &mut viewport);
        let mut tool = PathSelectTool::new();
        tool.activate(&mut ctx);
        let middle = DVec2::new(50., 0.);
        let response = click(&mut tool, &mut ctx, middle, ModifierKeys::empty());
        assert_eq!(
            selected_points(&response),
            vec![DVec2::ZERO, DVec2::new(100., 0.)]
        );
        assert!(ctx.doc.selection.contains(id));
        assert_eq!(path(ctx.doc, id), original);
        assert_eq!(ctx.doc.history.undo_depth(), 0);
        let hover = move_to(&mut tool, &mut ctx, middle, ModifierKeys::empty());
        assert!(
            !hover
                .overlays
                .iter()
                .any(|overlay| matches!(overlay, ToolOverlay::PathInsertHint { .. }))
        );
        let toggled = click(&mut tool, &mut ctx, middle, ModifierKeys::SHIFT);
        assert!(selected_points(&toggled).is_empty());
        assert_eq!(ctx.doc.history.undo_depth(), 0);
    }

    #[test]
    fn path_selection_closed_and_multiple_subpaths_select_the_correct_segment_endpoints() {
        for explicit_close in [false, true] {
            let mut original = PathData::new();
            original.move_to(-200., -200.).line_to(-100., -200.);
            original
                .move_to(0., 0.)
                .line_to(100., 0.)
                .line_to(100., 100.);
            if explicit_close {
                original.line_to(0., 0.);
            }
            original.close();
            let mut doc = Doc::new();
            let id = add_vector(&mut doc, original.clone(), None, Transform2D::IDENTITY);
            doc.selection.select_only(id);
            doc.history = Default::default();
            let mut viewport = Viewport::default();
            let mut ctx = context(&mut doc, &mut viewport);
            let mut tool = PathSelectTool::new();
            let response = click(
                &mut tool,
                &mut ctx,
                DVec2::new(50., 50.),
                ModifierKeys::empty(),
            );
            assert_eq!(
                selected_points(&response),
                vec![DVec2::ZERO, DVec2::new(100., 100.)]
            );
            assert_eq!(path(ctx.doc, id), original);
            assert_eq!(ctx.doc.history.undo_depth(), 0);
        }
    }

    #[test]
    fn path_selection_curved_segment_click_preserves_the_authored_curve() {
        for cubic in [false, true] {
            let mut original = PathData::new();
            original.move_to(0., 0.);
            if cubic {
                original.cubic_to(0., 60., 100., 60., 100., 0.);
            } else {
                original.quad_to(50., 80., 100., 0.);
            }
            let middle = crate::node_math::eval_segment(&original, 1, 0.5).expect("curve midpoint");
            let mut doc = Doc::new();
            let id = add_vector(&mut doc, original.clone(), None, Transform2D::IDENTITY);
            doc.history = Default::default();
            let mut viewport = Viewport::default();
            let mut ctx = context(&mut doc, &mut viewport);
            let mut tool = PathSelectTool::new();
            let response = click(&mut tool, &mut ctx, middle, ModifierKeys::empty());
            assert_eq!(
                selected_points(&response),
                vec![DVec2::ZERO, DVec2::new(100., 0.)]
            );
            assert_eq!(path(ctx.doc, id), original);
            assert_eq!(ctx.doc.history.undo_depth(), 0);
        }
    }

    #[test]
    fn path_selection_marquee_and_escape_change_only_point_selection() {
        let mut doc = Doc::new();
        let original = polyline();
        let id = add_vector(&mut doc, original.clone(), None, Transform2D::IDENTITY);
        doc.selection.select_only(id);
        doc.history = Default::default();
        let mut viewport = Viewport::default();
        let mut ctx = context(&mut doc, &mut viewport);
        let mut tool = PathSelectTool::new();
        press(
            &mut tool,
            &mut ctx,
            DVec2::new(-20., -20.),
            ModifierKeys::empty(),
        );
        move_to(
            &mut tool,
            &mut ctx,
            DVec2::new(120., 20.),
            ModifierKeys::empty(),
        );
        let response = release(
            &mut tool,
            &mut ctx,
            DVec2::new(120., 20.),
            ModifierKeys::empty(),
        );
        assert_eq!(
            selected_points(&response),
            vec![DVec2::ZERO, DVec2::new(100., 0.)]
        );
        let response = tool.handle_event(
            &mut ctx,
            ToolEvent::Key(KeyEvent::press(LogicalKey::Escape)),
        );
        assert!(selected_points(&response).is_empty());
        assert!(!response.wants_exit);
        let response = tool.handle_event(
            &mut ctx,
            ToolEvent::Key(KeyEvent::press(LogicalKey::Escape)),
        );
        assert!(response.wants_exit);
        assert_eq!(path(ctx.doc, id), original);
        assert_eq!(ctx.doc.history.undo_depth(), 0);
    }

    #[test]
    fn path_selection_segment_drag_respects_parent_transform_and_undo_redo() {
        let mut doc = Doc::new();
        let mut parent = CanvasNode::new(NodeData::Group(GroupNode::default()));
        parent.transform = Transform2D::rotation(0.6).then(&Transform2D::translation(30., 40.));
        let parent_id = parent.id;
        doc.apply(Operation::create_node(parent)).expect("parent");
        let original = polyline();
        let local_transform = Transform2D::scale_xy(2., 0.5);
        let id = add_vector(&mut doc, original.clone(), Some(parent_id), local_transform);
        doc.selection.select_only(id);
        doc.history = Default::default();
        let world = doc.scene.world_transform(id).expect("world transform");
        let mut viewport = Viewport {
            center: [12., -8.],
            zoom: 2.3,
        };
        let mut ctx = context(&mut doc, &mut viewport);
        let mut tool = PathSelectTool::new();
        let start = world.transform_point(DVec2::new(50., 0.));
        let end = world.transform_point(DVec2::new(60., 20.));
        press(&mut tool, &mut ctx, start, ModifierKeys::empty());
        move_to(&mut tool, &mut ctx, end, ModifierKeys::empty());
        assert_eq!(ctx.doc.history.undo_depth(), 0);
        release(&mut tool, &mut ctx, end, ModifierKeys::empty());
        let edited = path(ctx.doc, id);
        let anchors = crate::node_math::enumerate_anchors(&edited);
        for (anchor, expected) in anchors.iter().zip([
            DVec2::new(10., 20.),
            DVec2::new(110., 20.),
            DVec2::new(100., 100.),
        ]) {
            assert!((anchor.pos - expected).length() < 1e-8);
        }
        assert_eq!(
            ctx.doc.scene.get(id).expect("node").transform,
            local_transform
        );
        assert_eq!(ctx.doc.history.undo_depth(), 1);
        assert!(ctx.doc.undo().expect("undo"));
        assert_eq!(path(ctx.doc, id), original);
        assert!(ctx.doc.redo().expect("redo"));
        assert_eq!(path(ctx.doc, id), edited);
    }

    #[test]
    fn path_selection_escape_and_deactivation_restore_transient_edits() {
        for deactivate in [false, true] {
            let mut doc = Doc::new();
            let original = polyline();
            let id = add_vector(&mut doc, original.clone(), None, Transform2D::IDENTITY);
            doc.selection.select_only(id);
            doc.history = Default::default();
            let mut viewport = Viewport::default();
            let mut ctx = context(&mut doc, &mut viewport);
            let mut tool = PathSelectTool::new();
            press(&mut tool, &mut ctx, DVec2::ZERO, ModifierKeys::empty());
            move_to(
                &mut tool,
                &mut ctx,
                DVec2::new(20., 30.),
                ModifierKeys::empty(),
            );
            assert_ne!(path(ctx.doc, id), original);
            if deactivate {
                tool.deactivate(&mut ctx);
            } else {
                tool.handle_event(
                    &mut ctx,
                    ToolEvent::Key(KeyEvent::press(LogicalKey::Escape)),
                );
            }
            assert_eq!(path(ctx.doc, id), original);
            assert_eq!(ctx.doc.history.undo_depth(), 0);
            release(
                &mut tool,
                &mut ctx,
                DVec2::new(20., 30.),
                ModifierKeys::empty(),
            );
            assert_eq!(path(ctx.doc, id), original);
            assert_eq!(ctx.doc.history.undo_depth(), 0);
        }
    }

    #[test]
    fn path_selection_retargets_another_vector_and_keeps_shift_anchor_selection() {
        let mut doc = Doc::new();
        let first = add_vector(&mut doc, polyline(), None, Transform2D::IDENTITY);
        let second = add_vector(
            &mut doc,
            polyline(),
            None,
            Transform2D::translation(200., 0.),
        );
        doc.selection.select_only(first);
        doc.history = Default::default();
        let mut viewport = Viewport::default();
        let mut ctx = context(&mut doc, &mut viewport);
        let mut tool = PathSelectTool::new();
        click(&mut tool, &mut ctx, DVec2::ZERO, ModifierKeys::empty());
        let response = click(
            &mut tool,
            &mut ctx,
            DVec2::new(200., 0.),
            ModifierKeys::empty(),
        );
        assert_eq!(selected_points(&response), vec![DVec2::new(200., 0.)]);
        assert_eq!(
            ctx.doc.selection.iter().copied().collect::<Vec<_>>(),
            vec![second]
        );
        let response = click(
            &mut tool,
            &mut ctx,
            DVec2::new(300., 0.),
            ModifierKeys::SHIFT,
        );
        assert_eq!(
            selected_points(&response),
            vec![DVec2::new(200., 0.), DVec2::new(300., 0.)]
        );
        let response = click(
            &mut tool,
            &mut ctx,
            DVec2::new(200., 0.),
            ModifierKeys::SHIFT,
        );
        assert_eq!(selected_points(&response), vec![DVec2::new(300., 0.)]);
        assert_eq!(path(ctx.doc, first), polyline());
        assert_eq!(path(ctx.doc, second), polyline());
        assert_eq!(ctx.doc.history.undo_depth(), 0);
    }

    #[test]
    fn path_selection_respects_scope_hidden_locked_and_singular_targets() {
        for (flags, ancestor, out_of_scope, singular) in [
            (NodeFlags::LOCKED, false, false, false),
            (NodeFlags::HIDDEN, false, false, false),
            (NodeFlags::LOCKED, true, false, false),
            (NodeFlags::HIDDEN, true, false, false),
            (NodeFlags::empty(), false, true, false),
            (NodeFlags::empty(), false, false, true),
        ] {
            let mut doc = Doc::new();
            let parent = CanvasNode::new(NodeData::Group(GroupNode::default()));
            let parent_id = parent.id;
            doc.apply(Operation::create_node(parent)).expect("parent");
            let id = add_vector(
                &mut doc,
                polyline(),
                Some(parent_id),
                if singular {
                    Transform2D::scale(0.)
                } else {
                    Transform2D::IDENTITY
                },
            );
            doc.scene
                .get_mut(if ancestor { parent_id } else { id })
                .expect("flag target")
                .flags = flags;
            let other_page = CanvasNode::new(NodeData::Group(GroupNode::default()));
            let other_page_id = other_page.id;
            doc.apply(Operation::create_node(other_page))
                .expect("other page");
            doc.selection.select_only(id);
            doc.history = Default::default();
            let mut viewport = Viewport::default();
            let mut ctx = context(&mut doc, &mut viewport).with_scope_root(Some(if out_of_scope {
                other_page_id
            } else {
                parent_id
            }));
            let mut tool = PathSelectTool::new();
            tool.activate(&mut ctx);
            let response = press(&mut tool, &mut ctx, DVec2::ZERO, ModifierKeys::empty());
            assert!(response.overlays.is_empty());
            move_to(
                &mut tool,
                &mut ctx,
                DVec2::new(20., 30.),
                ModifierKeys::empty(),
            );
            release(
                &mut tool,
                &mut ctx,
                DVec2::new(20., 30.),
                ModifierKeys::empty(),
            );
            assert_eq!(path(ctx.doc, id), polyline());
            assert_eq!(ctx.doc.history.undo_depth(), 0);
        }
    }

    #[test]
    fn path_selection_lock_during_drag_cancels_the_preview() {
        let mut doc = Doc::new();
        let original = polyline();
        let id = add_vector(&mut doc, original.clone(), None, Transform2D::IDENTITY);
        doc.selection.select_only(id);
        doc.history = Default::default();
        let mut viewport = Viewport::default();
        let mut ctx = context(&mut doc, &mut viewport);
        let mut tool = PathSelectTool::new();
        press(&mut tool, &mut ctx, DVec2::ZERO, ModifierKeys::empty());
        move_to(
            &mut tool,
            &mut ctx,
            DVec2::new(20., 30.),
            ModifierKeys::empty(),
        );
        assert_ne!(path(ctx.doc, id), original);
        ctx.doc
            .scene
            .get_mut(id)
            .expect("lock node")
            .flags
            .insert(NodeFlags::LOCKED);
        release(
            &mut tool,
            &mut ctx,
            DVec2::new(20., 30.),
            ModifierKeys::empty(),
        );
        assert_eq!(path(ctx.doc, id), original);
        assert_eq!(ctx.doc.history.undo_depth(), 0);
    }

    #[test]
    fn path_selection_handle_drag_preserves_smooth_pair_and_alt_breaks_it() {
        let mut original = PathData::new();
        original
            .move_to(0., 0.)
            .cubic_to(0., 50., 100., 50., 100., 0.)
            .cubic_to(100., -50., 200., -50., 200., 0.);
        let mut doc = Doc::new();
        let id = add_vector(&mut doc, original.clone(), None, Transform2D::IDENTITY);
        doc.selection.select_only(id);
        doc.history = Default::default();
        let mut viewport = Viewport::default();
        let mut ctx = context(&mut doc, &mut viewport);
        let mut tool = PathSelectTool::new();
        click(
            &mut tool,
            &mut ctx,
            DVec2::new(100., 0.),
            ModifierKeys::empty(),
        );
        press(
            &mut tool,
            &mut ctx,
            DVec2::new(100., -50.),
            ModifierKeys::empty(),
        );
        move_to(
            &mut tool,
            &mut ctx,
            DVec2::new(130., -40.),
            ModifierKeys::empty(),
        );
        release(
            &mut tool,
            &mut ctx,
            DVec2::new(130., -40.),
            ModifierKeys::empty(),
        );
        let anchors = crate::node_math::enumerate_anchors(&path(ctx.doc, id));
        let anchor = anchors.get(1).expect("middle anchor");
        assert_eq!(anchor.ctrl_out, Some(DVec2::new(130., -40.)));
        let opposite = anchor.ctrl_in.expect("incoming handle");
        assert!((opposite - DVec2::new(70., 40.)).length() < 1e-8);
        press(
            &mut tool,
            &mut ctx,
            DVec2::new(130., -40.),
            ModifierKeys::ALT,
        );
        move_to(
            &mut tool,
            &mut ctx,
            DVec2::new(140., -20.),
            ModifierKeys::ALT,
        );
        release(
            &mut tool,
            &mut ctx,
            DVec2::new(140., -20.),
            ModifierKeys::ALT,
        );
        let anchors = crate::node_math::enumerate_anchors(&path(ctx.doc, id));
        let anchor = anchors.get(1).expect("middle anchor");
        assert_eq!(anchor.ctrl_in, Some(opposite));
        assert_eq!(anchor.ctrl_out, Some(DVec2::new(140., -20.)));
        assert_eq!(ctx.doc.history.undo_depth(), 2);
        assert!(ctx.doc.undo().expect("undo handle"));
        assert!(ctx.doc.undo().expect("undo handle"));
        assert_eq!(path(ctx.doc, id), original);
    }
}
