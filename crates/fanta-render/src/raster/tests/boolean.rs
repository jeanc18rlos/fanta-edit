//! Boolean-operation rendering: the node's shape is the fold of its operand
//! children, painted with the node's own fill. Verified by probing pixels that
//! land inside vs. outside the folded region for Subtract and Intersect.

use super::*;
use fanta_doc::{BooleanNode, BooleanOp, CanvasNode, Doc, NodeData, Operation, VectorNode};

/// A boolean node with two overlapping rect operands: A = [-20,-20 .. 20,20]
/// (covers the center), B = [0,0 .. 20,20] (the bottom-right quadrant). The
/// node paints red. With an identity viewport on a 64×64 surface, world `(x, y)`
/// maps to pixel `(32 + x, 32 + y)`.
fn two_rect_boolean(op: BooleanOp) -> Doc {
    let mut doc = Doc::new();
    let boolean = CanvasNode::new(NodeData::Boolean(BooleanNode {
        op,
        fills: smallvec_of(Fill::solid(Color::rgb(255, 0, 0))),
        strokes: Default::default(),
    }));
    let boolean_id = boolean.id;
    doc.apply(Operation::create_node(boolean)).unwrap();

    for (x, y, w, h) in [(-20.0, -20.0, 40.0, 40.0), (0.0, 0.0, 20.0, 20.0)] {
        let mut operand = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            x,
            y,
            w,
            h,
            Color::rgb(0, 255, 0), // operand paint is unused — the fold is painted
        )));
        operand.parent = Some(boolean_id);
        operand.index = doc.scene.next_child_index(Some(boolean_id));
        doc.apply(Operation::create_node(operand)).unwrap();
    }
    doc
}

#[test]
fn subtract_cuts_the_second_operand_out_of_the_first() {
    let doc = two_rect_boolean(BooleanOp::Subtract);
    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    // Inside A, outside B (world -10,-10 → pixel 22,22): kept, red.
    let kept = rgba_at(&buf, 64, 22, 22);
    assert!(
        kept[0] > 200 && kept[3] > 200,
        "kept region should be opaque red, got {kept:?}"
    );
    // Inside A AND B (world 10,10 → pixel 42,42): subtracted away, transparent.
    let hole = rgba_at(&buf, 64, 42, 42);
    assert_eq!(
        hole[3], 0,
        "subtracted hole should be transparent, got {hole:?}"
    );
}

#[test]
fn intersect_keeps_only_the_overlap() {
    let doc = two_rect_boolean(BooleanOp::Intersect);
    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    // The overlap is exactly B = [0,0 .. 20,20]; its interior (world 10,10 →
    // pixel 42,42) is filled, and A-only (world -10,-10 → pixel 22,22) is empty.
    let overlap = rgba_at(&buf, 64, 42, 42);
    assert!(
        overlap[0] > 200 && overlap[3] > 200,
        "overlap should be opaque red, got {overlap:?}"
    );
    let outside = rgba_at(&buf, 64, 22, 22);
    assert_eq!(
        outside[3], 0,
        "outside the overlap should be transparent, got {outside:?}"
    );
}

#[test]
fn empty_boolean_paints_nothing() {
    // A boolean with no operands folds to nothing and must not panic.
    let mut doc = Doc::new();
    let boolean = CanvasNode::new(NodeData::Boolean(BooleanNode {
        op: BooleanOp::Union,
        fills: smallvec_of(Fill::solid(Color::rgb(255, 0, 0))),
        strokes: Default::default(),
    }));
    doc.apply(Operation::create_node(boolean)).unwrap();
    let mut r = RasterRenderer::new(16, 16).unwrap();
    r.render(&doc.scene, &doc.viewport);
    assert_eq!(
        opaque_pixel_count(&r.copy_rgba()),
        0,
        "no operands ⇒ nothing painted"
    );
}

#[test]
fn boolean_cache_populates_and_clears() {
    let doc = two_rect_boolean(BooleanOp::Union);
    let mut r = RasterRenderer::new(64, 64).unwrap();
    assert_eq!(r.boolean_cache_len(), 0, "starts empty");
    r.render(&doc.scene, &doc.viewport);
    let after = r.boolean_cache_len();
    assert!(
        after > 0,
        "render of boolean should populate cache, got {}",
        after
    );
    r.clear_boolean_cache();
    assert_eq!(r.boolean_cache_len(), 0, "clear empties it");
    r.render(&doc.scene, &doc.viewport);
    assert!(r.boolean_cache_len() > 0, "re-render repopulates");
}
