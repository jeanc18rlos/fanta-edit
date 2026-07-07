//! Shared test fixtures + helpers used across more than one raster test module.
//!
//! Per `docs/coding-practices.md` §4, fixtures several test modules need live in
//! one `test_support`-style module rather than being copy-pasted. These are the
//! genuinely cross-cutting ones; section-specific builders stay next to the
//! tests that use them.

use super::*;
use fanta_doc::{CanvasNode, Doc, NodeData, Operation, VectorNode};

pub(crate) fn red_rect_doc(w: f64, h: f64) -> Doc {
    let mut doc = Doc::new();
    let n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -w * 0.5,
        -h * 0.5,
        w,
        h,
        Color::rgb(255, 0, 0),
    )));
    doc.apply(Operation::create_node(n)).unwrap();
    doc
}

/// Build a one-element `SmallVec<[Fill; 1]>` without importing the type name.
pub(crate) fn smallvec_of(f: Fill) -> smallvec::SmallVec<[Fill; 1]> {
    let mut v = smallvec::SmallVec::new();
    v.push(f);
    v
}

/// Read the straight-RGBA pixel at `(x, y)` from a `copy_rgba` buffer of the
/// given `width`.
pub(crate) fn rgba_at(buf: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let row = (width * 4) as usize;
    let i = (y as usize) * row + (x as usize) * 4;
    [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
}

/// Count straight-alpha-opaque pixels in a `copy_rgba` buffer.
pub(crate) fn opaque_pixel_count(buf: &[u8]) -> usize {
    buf.chunks_exact(4).filter(|px| px[3] != 0).count()
}
