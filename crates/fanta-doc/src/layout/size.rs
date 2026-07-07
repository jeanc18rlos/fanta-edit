//! Per-variant size + local-origin accessors for the auto-layout solver.
//!
//! The solver reasons about each child as a box: a `(width, height)` extent and
//! a local-space origin (the top-left of that box in the child's own coordinate
//! system, before its `transform` is applied). For most variants the origin is
//! `(0, 0)` and the size lives in a `local_size` field; a vector's box is its
//! path's bounding rect, which need not start at the origin. Keeping these two
//! concerns in one place means [`super::solve`] never matches on [`NodeData`].

use crate::node::{CanvasNode, NodeData};
use crate::path::PathData;

/// The size + local origin of a node's content box, in its own local space
/// *before* its `transform`. `size` is `[w, h]`; `origin` is the local-space
/// top-left of that box (usually `[0, 0]`, but a vector path can be offset).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LocalBox {
    pub origin: [f64; 2],
    pub size: [f64; 2],
}

impl LocalBox {
    pub fn zero() -> Self {
        Self {
            origin: [0.0, 0.0],
            size: [0.0, 0.0],
        }
    }
}

/// Read a node's local box (extent + origin). A frame uses its `clip_size`; a
/// vector its path's rough bounds; the box-shaped variants their `local_size`.
/// A group with no `clip_size` reports a zero box — the solver only flows
/// auto-layout *frames* (which always carry a `clip_size` from the importer),
/// so a sizeless plain group never participates as a measurable child here.
pub(crate) fn local_box(node: &CanvasNode) -> LocalBox {
    match &node.data {
        NodeData::Group(g) => match g.clip_size {
            Some([w, h]) => LocalBox {
                origin: [0.0, 0.0],
                size: [w, h],
            },
            None => LocalBox::zero(),
        },
        NodeData::Vector(v) => match v.path.rough_bounds() {
            Some(b) => LocalBox {
                origin: [b.min_x, b.min_y],
                size: [b.width(), b.height()],
            },
            None => LocalBox::zero(),
        },
        NodeData::Text(t) => box_from_size(t.local_size),
        NodeData::Bitmap(b) => box_from_size(b.local_size),
        NodeData::Video(v) => box_from_size(v.local_size),
        NodeData::Audio(a) => box_from_size(a.local_size),
        NodeData::NodeGraph(n) => box_from_size(n.local_size),
        NodeData::Model3d(m) => box_from_size(m.local_size),
        NodeData::AiArtifact(a) => box_from_size(a.local_size),
        NodeData::Instance(i) => box_from_size(i.local_size),
        NodeData::Embed(e) => box_from_size(e.local_size),
    }
}

fn box_from_size(size: [f64; 2]) -> LocalBox {
    LocalBox {
        origin: [0.0, 0.0],
        size,
    }
}

/// Overwrite a node's box size along both axes, keeping its local origin fixed.
/// Used for FILL (grow) on the primary axis and Stretch on the counter axis. A
/// vector is re-laid as an axis-aligned rect at its existing origin (auto-layout
/// children that stretch/grow are overwhelmingly rectangles — buttons, dividers,
/// backgrounds — so re-rectangling is faithful and keeps the box exact); a frame
/// updates its `clip_size`; box variants their `local_size`.
pub(crate) fn set_size(node: &mut CanvasNode, w: f64, h: f64) {
    match &mut node.data {
        NodeData::Group(g) => {
            // Only a sized (frame) group is resized; an unclipped group has no
            // box and is left alone.
            if g.clip_size.is_some() {
                g.clip_size = Some([w, h]);
            }
        }
        NodeData::Vector(v) => {
            let Some(b) = v.path.rough_bounds() else {
                return;
            };
            if v.path.is_rect() {
                // A real rectangle (button / divider / background) resizes as a
                // rectangle — faithful, and keeps the box exact.
                v.path = PathData::rect(b.min_x, b.min_y, w, h);
            } else {
                // Any other vector (icon, ellipse, star, custom path) keeps its
                // geometry: scale it to the requested box about its top-left so a
                // FILL/stretch child resizes the SHAPE instead of being flattened
                // to a rectangle. A fixed/hug child is measured at its own bounds,
                // so this is a no-op (scale ≈ 1) and the path stays byte-stable.
                let (bw, bh) = (b.width(), b.height());
                let sx = if bw > 1e-9 { w / bw } else { 1.0 };
                let sy = if bh > 1e-9 { h / bh } else { 1.0 };
                if (sx - 1.0).abs() > 1e-9 || (sy - 1.0).abs() > 1e-9 {
                    v.path.scale_about(b.min_x, b.min_y, sx, sy);
                }
            }
        }
        NodeData::Text(t) => t.local_size = [w, h],
        NodeData::Bitmap(b) => b.local_size = [w, h],
        NodeData::Video(v) => v.local_size = [w, h],
        NodeData::Audio(a) => a.local_size = [w, h],
        NodeData::NodeGraph(n) => n.local_size = [w, h],
        NodeData::Model3d(m) => m.local_size = [w, h],
        NodeData::AiArtifact(a) => a.local_size = [w, h],
        NodeData::Instance(i) => i.local_size = [w, h],
        NodeData::Embed(e) => e.local_size = [w, h],
    }
}
