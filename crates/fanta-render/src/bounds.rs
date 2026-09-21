use fanta_doc::{Bounds, NodeData, NodeFlags, NodeId, Scene, StrokeAlign, StrokeJoin};

use crate::raster::shadow_expanded_local_bounds;

/// Conservative world-space bounds of everything that can paint in `root`'s
/// subtree, including outside/center strokes, drop shadows, and layer blurs.
///
/// `effective_scale` is the renderer's world-to-device scale. Pass `0.0` when
/// callers need uncapped authoring bounds that are safe for every export scale.
pub fn visual_world_bounds(scene: &Scene, root: NodeId, effective_scale: f32) -> Option<Bounds> {
    let local = local_visual_bounds(scene, root, effective_scale)?;
    local.try_transformed(&scene.world_transform(root)?)
}

fn local_visual_bounds(scene: &Scene, id: NodeId, effective_scale: f32) -> Option<Bounds> {
    let Some(node) = scene.get(id) else {
        return None;
    };
    if node.flags.contains(NodeFlags::HIDDEN) {
        return None;
    }

    let (mut result, skip_children) = match &node.data {
        NodeData::Group(group) => {
            let box_bounds = group
                .clip_size
                .or(group.local_size)
                .map(|[width, height]| Bounds::from_xywh(0.0, 0.0, width, height))
                .or_else(|| {
                    (group.background.is_some()
                        || !group.background_fills.is_empty()
                        || !group.strokes.is_empty())
                    .then(|| scene.local_bounds(id))
                    .flatten()
                });
            let own = box_bounds.map(|bounds| expand_for_strokes(bounds, &group.strokes, true));
            let clips_children = group.clip_size.is_some()
                && node
                    .meta
                    .get("clip_content")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true);
            (own, clips_children)
        }
        NodeData::Vector(vector) => {
            let own = node
                .data
                .local_bounds()
                .map(|bounds| expand_for_strokes(bounds, &vector.strokes, vector.path.is_rect()));
            let viewport = if node.flags.contains(NodeFlags::UNCLIPPED_VECTOR) {
                None
            } else {
                vector.local_size
            };
            let own = match (own, viewport) {
                (Some(bounds), Some([width, height])) => {
                    intersect_bounds(bounds, Bounds::from_xywh(0.0, 0.0, width, height))
                }
                (bounds, None) => bounds,
                (None, Some(_)) => None,
            };
            (own, false)
        }
        NodeData::Boolean(boolean) => (
            scene
                .local_bounds(id)
                .map(|bounds| expand_for_strokes(bounds, &boolean.strokes, false)),
            true,
        ),
        _ => (node.data.local_bounds(), false),
    };
    if !skip_children {
        for child in scene.children_of(Some(id)) {
            if let (Some(child_bounds), Some(child_node)) = (
                local_visual_bounds(scene, *child, effective_scale),
                scene.get(*child),
            ) && let Some(child_bounds) = child_bounds.try_transformed(&child_node.transform)
            {
                result = Some(match result {
                    Some(existing) => existing.union(&child_bounds),
                    None => child_bounds,
                });
            }
        }
    }
    result.map(|bounds| {
        shadow_expanded_local_bounds(bounds, &node.effects, &node.blurs, effective_scale)
    })
}

fn intersect_bounds(left: Bounds, right: Bounds) -> Option<Bounds> {
    let intersection = Bounds {
        min_x: left.min_x.max(right.min_x),
        min_y: left.min_y.max(right.min_y),
        max_x: left.max_x.min(right.max_x),
        max_y: left.max_y.min(right.max_y),
    };
    (intersection.is_finite()
        && intersection.min_x < intersection.max_x
        && intersection.min_y < intersection.max_y)
        .then_some(intersection)
}

fn expand_for_strokes(
    mut bounds: Bounds,
    strokes: &[fanta_doc::Stroke],
    rectangular: bool,
) -> Bounds {
    let mut reach = 0.0_f64;
    for stroke in strokes {
        let width = stroke
            .per_side
            .map(|sides| sides.into_iter().fold(stroke.width, f64::max))
            .unwrap_or(stroke.width)
            .max(0.0);
        if !width.is_finite() {
            continue;
        }
        let alignment = match stroke.align {
            StrokeAlign::Inside => 0.0,
            StrokeAlign::Center => 0.5,
            StrokeAlign::Outside => 1.0,
        };
        let join = if !rectangular && stroke.join == StrokeJoin::Miter {
            stroke.miter_limit.max(1.0)
        } else {
            1.0
        };
        if join.is_finite() {
            reach = reach.max(width * alignment * join);
        }
    }
    if reach > 0.0 {
        bounds.min_x -= reach;
        bounds.min_y -= reach;
        bounds.max_x += reach;
        bounds.max_y += reach;
    }
    bounds
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{
        Blur, BooleanNode, CanvasNode, Color, Doc, GroupNode, NodeData, Shadow, ShadowKind, Stroke,
        Transform2D, VectorNode,
    };

    fn insert(doc: &mut Doc, mut node: CanvasNode) -> NodeId {
        node.index = doc.scene.next_child_index(node.parent);
        let id = node.id;
        doc.scene.insert(node).expect("valid test node");
        id
    }

    fn rectangle() -> CanvasNode {
        CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            10.0,
            10.0,
            Color::WHITE,
        )))
    }

    #[test]
    fn visual_bounds_include_outside_strokes() {
        let mut doc = Doc::new();
        let mut rectangle = rectangle();
        let NodeData::Vector(vector) = &mut rectangle.data else {
            unreachable!();
        };
        let mut stroke = Stroke::solid(Color::BLACK, 4.0);
        stroke.align = StrokeAlign::Outside;
        stroke.join = StrokeJoin::Round;
        vector.strokes.push(stroke);
        let id = insert(&mut doc, rectangle);

        assert_eq!(
            visual_world_bounds(&doc.scene, id, 0.0),
            Some(Bounds::from_xywh(-4.0, -4.0, 18.0, 18.0))
        );
    }

    #[test]
    fn visual_bounds_include_drop_shadows_and_layer_blurs() {
        let mut doc = Doc::new();
        let mut rectangle = rectangle();
        rectangle.effects.push(Shadow {
            kind: ShadowKind::Drop,
            color: Color::BLACK,
            blur: 4.0,
            spread: 0.0,
            offset: [5.0, 0.0],
            show_behind_node: false,
        });
        rectangle.blurs.push(Blur::layer(4.0));
        let id = insert(&mut doc, rectangle);

        let bounds = visual_world_bounds(&doc.scene, id, 0.0).expect("paint bounds");
        assert_eq!(bounds, Bounds::from_xywh(-6.0, -6.0, 27.0, 22.0));
    }

    #[test]
    fn visual_bounds_compose_nested_world_transforms() {
        let mut doc = Doc::new();
        let mut group = CanvasNode::new(NodeData::Group(GroupNode::default()));
        group.transform = Transform2D::translation(100.0, 50.0);
        let group = insert(&mut doc, group);
        let mut rectangle = rectangle();
        rectangle.parent = Some(group);
        rectangle.transform = Transform2D::translation(7.0, 9.0);
        let rectangle = insert(&mut doc, rectangle);

        assert_eq!(
            visual_world_bounds(&doc.scene, rectangle, 0.0),
            Some(Bounds::from_xywh(107.0, 59.0, 10.0, 10.0))
        );
    }

    #[test]
    fn parent_effects_expand_the_already_painted_child_subtree() {
        let mut doc = Doc::new();
        let mut group = CanvasNode::new(NodeData::Group(GroupNode::default()));
        group.effects.push(Shadow {
            kind: ShadowKind::Drop,
            color: Color::BLACK,
            blur: 0.0,
            spread: 0.0,
            offset: [5.0, 0.0],
            show_behind_node: false,
        });
        let group = insert(&mut doc, group);
        let mut rectangle = rectangle();
        rectangle.parent = Some(group);
        let NodeData::Vector(vector) = &mut rectangle.data else {
            unreachable!();
        };
        let mut stroke = Stroke::solid(Color::BLACK, 4.0);
        stroke.align = StrokeAlign::Outside;
        stroke.join = StrokeJoin::Round;
        vector.strokes.push(stroke);
        insert(&mut doc, rectangle);

        assert_eq!(
            visual_world_bounds(&doc.scene, group, 0.0),
            Some(Bounds::from_xywh(-4.0, -4.0, 23.0, 18.0))
        );
    }

    #[test]
    fn clipped_frame_ignores_invisible_child_overflow() {
        let mut doc = Doc::new();
        let frame = insert(
            &mut doc,
            CanvasNode::new(NodeData::Group(GroupNode {
                clip_size: Some([20.0, 10.0]),
                ..GroupNode::default()
            })),
        );
        let mut child = rectangle();
        child.parent = Some(frame);
        child.transform = Transform2D::translation(100.0, 50.0);
        insert(&mut doc, child);

        assert_eq!(
            visual_world_bounds(&doc.scene, frame, 0.0),
            Some(Bounds::from_xywh(0.0, 0.0, 20.0, 10.0))
        );
    }

    #[test]
    fn frame_with_clipping_disabled_includes_child_overflow() {
        let mut doc = Doc::new();
        let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([20.0, 10.0]),
            ..GroupNode::default()
        }));
        frame.meta = serde_json::json!({ "clip_content": false });
        let frame = insert(&mut doc, frame);
        let mut child = rectangle();
        child.parent = Some(frame);
        child.transform = Transform2D::translation(100.0, 50.0);
        insert(&mut doc, child);

        assert_eq!(
            visual_world_bounds(&doc.scene, frame, 0.0),
            Some(Bounds::from_xywh(0.0, 0.0, 110.0, 60.0))
        );
    }

    #[test]
    fn vector_viewport_crops_geometry_and_strokes() {
        let mut doc = Doc::new();
        let mut vector = rectangle();
        let NodeData::Vector(vector_data) = &mut vector.data else {
            unreachable!();
        };
        vector_data.path = VectorNode::rect_solid(0.0, 0.0, 100.0, 50.0, Color::WHITE).path;
        vector_data.local_size = Some([20.0, 10.0]);
        let mut stroke = Stroke::solid(Color::BLACK, 8.0);
        stroke.align = StrokeAlign::Outside;
        vector_data.strokes.push(stroke);
        let vector = insert(&mut doc, vector);

        assert_eq!(
            visual_world_bounds(&doc.scene, vector, 0.0),
            Some(Bounds::from_xywh(0.0, 0.0, 20.0, 10.0))
        );
    }

    #[test]
    fn path_edit_viewport_flag_preserves_geometry_and_stroke_visual_bounds() {
        let mut doc = Doc::new();
        let mut node = rectangle();
        let vector = node.data.as_vector_mut().expect("vector fixture");
        vector.path = VectorNode::rect_solid(0., 0., 100., 50., Color::WHITE).path;
        vector.local_size = Some([20., 10.]);
        let mut stroke = Stroke::solid(Color::BLACK, 8.);
        stroke.align = StrokeAlign::Outside;
        vector.strokes.push(stroke);
        node.flags.insert(NodeFlags::UNCLIPPED_VECTOR);
        let id = insert(&mut doc, node);
        assert_eq!(
            visual_world_bounds(&doc.scene, id, 0.),
            Some(Bounds::from_xywh(-8., -8., 116., 66.)),
            "explicitly unclipped vectors need their full export bounds, even with a remaining box"
        );
    }

    #[test]
    fn sizeless_group_and_boolean_include_their_own_outside_strokes() {
        let mut doc = Doc::new();
        let mut group_data = GroupNode::default();
        let mut group_stroke = Stroke::solid(Color::BLACK, 4.0);
        group_stroke.align = StrokeAlign::Outside;
        group_data.strokes.push(group_stroke);
        let group = insert(&mut doc, CanvasNode::new(NodeData::Group(group_data)));
        let mut group_child = rectangle();
        group_child.parent = Some(group);
        insert(&mut doc, group_child);

        let mut boolean_data = BooleanNode::default();
        let mut boolean_stroke = Stroke::solid(Color::BLACK, 4.0);
        boolean_stroke.align = StrokeAlign::Outside;
        boolean_stroke.join = StrokeJoin::Round;
        boolean_data.strokes.push(boolean_stroke);
        let boolean = insert(&mut doc, CanvasNode::new(NodeData::Boolean(boolean_data)));
        let mut boolean_child = rectangle();
        boolean_child.parent = Some(boolean);
        insert(&mut doc, boolean_child);

        let expected = Some(Bounds::from_xywh(-4.0, -4.0, 18.0, 18.0));
        assert_eq!(visual_world_bounds(&doc.scene, group, 0.0), expected);
        assert_eq!(visual_world_bounds(&doc.scene, boolean, 0.0), expected);
    }
}
