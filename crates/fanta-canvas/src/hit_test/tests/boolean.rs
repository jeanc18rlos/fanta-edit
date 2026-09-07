//! Boolean-operation hit-testing: the boolean node is the selection target for
//! its whole folded shape; its operand children are not independently hittable.

use super::*;
use fanta_doc::{BooleanNode, BooleanOp, CanvasNode, NodeData};

/// A boolean node at the root with two rect operands: A at (0,0,40,40) and B at
/// (100,100,40,40).
fn boolean_doc() -> (Doc, NodeId, NodeId) {
    let mut doc = Doc::new();
    let boolean = CanvasNode::new(NodeData::Boolean(BooleanNode {
        op: BooleanOp::Union,
        ..BooleanNode::default()
    }));
    let boolean_id = boolean.id;
    doc.apply(Operation::create_node(boolean)).unwrap();

    let mut a = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        40.0,
        40.0,
        Color::WHITE,
    )));
    a.parent = Some(boolean_id);
    a.index = doc.scene.next_child_index(Some(boolean_id));
    let a_id = a.id;
    doc.apply(Operation::create_node(a)).unwrap();

    let mut b = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        100.0,
        100.0,
        40.0,
        40.0,
        Color::WHITE,
    )));
    b.parent = Some(boolean_id);
    b.index = doc.scene.next_child_index(Some(boolean_id));
    doc.apply(Operation::create_node(b)).unwrap();

    (doc, boolean_id, a_id)
}

#[test]
fn clicking_an_operand_selects_the_boolean_node() {
    let (doc, boolean_id, a_id) = boolean_doc();
    // A point inside operand A resolves to the boolean node, not the operand.
    let hit = hit_test(
        &doc.scene,
        DVec2::new(20.0, 20.0),
        HitPrecision::Bounds,
        None,
    );
    assert_eq!(hit, Some(boolean_id), "the boolean node is the hit target");
    assert_ne!(hit, Some(a_id), "the operand itself is not selectable");
}

#[test]
fn boolean_operands_are_absent_from_deep_hits() {
    use crate::hit_test::hit_test_deep;
    let (doc, boolean_id, a_id) = boolean_doc();
    let hits = hit_test_deep(
        &doc.scene,
        DVec2::new(20.0, 20.0),
        HitPrecision::Bounds,
        None,
    );
    assert!(hits.contains(&boolean_id), "boolean node is a deep hit");
    assert!(
        !hits.contains(&a_id),
        "operands are excluded from deep hits"
    );
}
