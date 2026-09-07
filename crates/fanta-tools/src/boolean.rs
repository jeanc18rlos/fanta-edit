//! The "make boolean operation" editor command.
//!
//! Wraps a selection of nodes in a new [`NodeData::Boolean`] container so the
//! renderer folds them under the chosen [`BooleanOp`]. The operands become the
//! boolean node's children (reparented in place, world position preserved); the
//! boolean node inherits the bottom operand's paint, matching Figma's "the
//! result takes the lowest layer's style".

use fanta_doc::{
    BooleanNode, BooleanOp, CanvasNode, Color, Doc, Fill, NodeData, NodeId, Operation, Transform2D,
};
use smallvec::smallvec;

/// Wrap `operands` (bottom-first: `operands[0]` is the lowest layer) in a new
/// boolean-operation node folding them under `op`, and return its id.
///
/// The boolean node is created at the first operand's parent with an identity
/// transform; every operand is reparented under it with its world position
/// preserved. Returns `None` if `operands` is empty or the first operand is not
/// in the scene. Each reparent that Would form a cycle / hit a non-container is
/// skipped (logged), never corrupting the document.
pub fn make_boolean(doc: &mut Doc, operands: &[NodeId], op: BooleanOp) -> Option<NodeId> {
    let first = *operands.first()?;
    let first_node = doc.scene.get(first)?;
    let parent = first_node.parent;

    // Inherit the bottom operand's paint when it is a vector; otherwise fall back
    // to a plain black fill so the result is visible.
    let (fills, strokes) = match &first_node.data {
        NodeData::Vector(v) => (v.fills.clone(), v.strokes.clone()),
        _ => (smallvec![Fill::solid(Color::BLACK)], smallvec![]),
    };

    let mut boolean = CanvasNode::new(NodeData::Boolean(BooleanNode { op, fills, strokes }));
    boolean.parent = parent;
    boolean.index = doc.scene.next_child_index(parent);
    let boolean_id = boolean.id;
    if let Err(e) = doc.apply(Operation::create_node(boolean)) {
        tracing::warn!(target: "fanta-tools.boolean", "create boolean node failed: {e}");
        return None;
    }

    for &id in operands {
        let Some(node) = doc.scene.get(id) else {
            continue;
        };
        let old_parent = node.parent;
        let old_index = node.index;
        let old_local = node.transform;
        let world = doc
            .scene
            .world_transform(id)
            .unwrap_or(Transform2D::IDENTITY);
        let new_index = doc.scene.next_child_index(Some(boolean_id));
        if let Err(e) = doc.apply(Operation::Reparent {
            id,
            old_parent,
            old_index,
            new_parent: Some(boolean_id),
            new_index,
        }) {
            tracing::warn!(target: "fanta-tools.boolean", "reparent operand failed: {e}");
            continue;
        }
        // Rebase into the boolean node's space so the operand keeps its world
        // position (the boolean node is identity at the operands' old parent, so
        // this is usually a no-op — but a cross-parent selection needs it).
        let parent_world = doc
            .scene
            .world_transform(boolean_id)
            .unwrap_or(Transform2D::IDENTITY);
        let new_local = world.then(&parent_world.inverse());
        if new_local != old_local {
            if let Err(e) = doc.apply(Operation::SetTransform {
                id,
                old: old_local,
                new: new_local,
            }) {
                tracing::warn!(target: "fanta-tools.boolean", "operand rebase failed: {e}");
            }
        }
    }

    Some(boolean_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{GroupNode, VectorNode};

    fn rect(x: f64, y: f64, w: f64, h: f64) -> CanvasNode {
        CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            x,
            y,
            w,
            h,
            Color::rgb(10, 20, 30),
        )))
    }

    #[test]
    fn make_boolean_wraps_the_selection_as_operands() {
        let mut doc = Doc::new();
        let page = {
            let g = CanvasNode::new(NodeData::Group(GroupNode {
                clip_size: Some([400.0, 400.0]),
                ..GroupNode::default()
            }));
            let id = g.id;
            doc.apply(Operation::create_node(g)).unwrap();
            id
        };
        let a = {
            let mut n = rect(0.0, 0.0, 100.0, 100.0);
            n.parent = Some(page);
            n.index = doc.scene.next_child_index(Some(page));
            let id = n.id;
            doc.apply(Operation::create_node(n)).unwrap();
            id
        };
        let b = {
            let mut n = rect(50.0, 50.0, 100.0, 100.0);
            n.parent = Some(page);
            n.index = doc.scene.next_child_index(Some(page));
            let id = n.id;
            doc.apply(Operation::create_node(n)).unwrap();
            id
        };

        let boolean = make_boolean(&mut doc, &[a, b], BooleanOp::Subtract).unwrap();

        // The boolean node exists, is a Boolean of the right op, and owns both
        // operands as children.
        let node = doc.scene.get(boolean).unwrap();
        assert_eq!(
            node.data.as_boolean().map(|x| x.op),
            Some(BooleanOp::Subtract)
        );
        assert_eq!(node.parent, Some(page));
        let children = doc.scene.children_of(Some(boolean));
        assert!(
            children.contains(&a) && children.contains(&b),
            "operands reparented"
        );
        assert_eq!(doc.scene.get(a).unwrap().parent, Some(boolean));

        // The boolean inherited the bottom operand's fill.
        assert_eq!(node.data.as_boolean().unwrap().fills.len(), 1);
    }

    #[test]
    fn make_boolean_on_empty_selection_is_none() {
        let mut doc = Doc::new();
        assert!(make_boolean(&mut doc, &[], BooleanOp::Union).is_none());
    }
}
