//! Shared fixtures for the hit-test suite — only the genuinely cross-cutting
//! ones (used by more than one section); section-specific builders live next to
//! the tests that use them. Reached via `use super::*;` (re-exported by
//! `tests/mod.rs`).
#![allow(dead_code)]

use super::*;
use fanta_doc::{CanvasNode, Color, Doc, NodeData, Operation, VectorNode};

pub(crate) fn rect_doc(x: f64, y: f64, w: f64, h: f64) -> (Doc, NodeId) {
    let mut doc = Doc::new();
    let n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        x,
        y,
        w,
        h,
        Color::WHITE,
    )));
    let id = n.id;
    doc.apply(Operation::create_node(n)).unwrap();
    (doc, id)
}
