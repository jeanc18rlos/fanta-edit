//! Shared (non-test-crate) support for the legacy OpenPencil-derived
//! comprehensive matcher: the golden JSON types and the normalized view of
//! our resolved nodes. Lives under tests/common/ so cargo treats it as a
//! shared module rather than its own test crate. Only the legacy report file
//! consumes these; they are `pub` so that sibling test crate can see them.

use std::path::Path;

use fanta_doc::Color;
use fanta_doc::snapshot::NodeSnapshot;
use serde::Deserialize;

/// A node in the comprehensive (full) golden.
#[derive(Debug, Clone, Deserialize)]
pub struct FullNode {
    /// Stable key: the source z-order index in OpenPencil's flat scene.
    #[allow(dead_code)]
    pub k: usize,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    /// `[x, y, w, h]` integer absolute page coordinates.
    pub abs: [f64; 4],
    /// Solid fill hex (`#rrggbb`) or `None`.
    pub fill: Option<String>,
    #[allow(dead_code)]
    pub stroke: bool,
    pub text: Option<String>,
    #[allow(dead_code)]
    pub cr: Option<f64>,
}

impl FullNode {
    pub fn size(&self) -> (f64, f64) {
        (self.abs[2], self.abs[3])
    }
    pub fn area(&self) -> f64 {
        self.abs[2] * self.abs[3]
    }
    pub fn fill_color(&self) -> Option<Color> {
        self.fill.as_deref().and_then(Color::from_hex)
    }
}

#[derive(Debug, Deserialize)]
pub struct FullGolden {
    pub page: String,
    pub status_frame: StatusFrame,
    #[allow(dead_code)]
    pub node_count: usize,
    pub nodes: Vec<FullNode>,
}

#[derive(Debug, Deserialize)]
pub struct StatusFrame {
    #[allow(dead_code)]
    pub name: String,
    /// `[x, y, w, h]`.
    pub abs: [f64; 4],
}

pub fn load_full_golden(file: &str) -> FullGolden {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(file);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read full golden {}: {e}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("parse full golden {}: {e}", path.display()))
}

/// A normalized view of one of OUR resolved nodes for matching: center, size,
/// and the paint/text values to compare. We drop pure `instance` placeholder
/// nodes (OpenPencil's flat scene has no instance node — only the expanded
/// result), so they don't create spurious EXTRA-node noise.
#[derive(Clone)]
pub struct OurNode {
    pub idx: usize,
    pub name: String,
    pub kind: String,
    /// World-space top-left (min corner). We match on top-left rather than
    /// center because both layouts grow from the top-left, so a node whose
    /// content measured slightly narrower still shares its origin with the
    /// golden node — center-matching, by contrast, would shift by half the
    /// width delta and miss.
    pub x0: f64,
    pub y0: f64,
    pub w: f64,
    pub h: f64,
    pub fill: Option<Color>,
    pub text: Option<String>,
}

pub fn collect_our_nodes(snap: &[NodeSnapshot]) -> Vec<OurNode> {
    snap.iter()
        .enumerate()
        .filter(|(_, n)| n.abs_bounds.is_some() && n.kind != "instance")
        .map(|(idx, n)| {
            let b = n.abs_bounds.unwrap();
            OurNode {
                idx,
                name: n.name.clone(),
                kind: n.kind.clone(),
                x0: b[0],
                y0: b[1],
                w: b[2] - b[0],
                h: b[3] - b[1],
                fill: n.fill_rgba.map(|[r, g, b, a]| Color::rgba(r, g, b, a)),
                text: n.text.clone(),
            }
        })
        .collect()
}

/// Translation aligning golden page space to ours, derived from the Status
/// frame top-left present in both. `to_ours(golden_x) = golden_x - dx`.
pub struct Align {
    pub dx: f64,
    pub dy: f64,
}

impl Align {
    pub fn derive(golden: &FullGolden, ours: &[OurNode]) -> Self {
        // Golden Status top-left from the committed status_frame.
        let (gx, gy) = (golden.status_frame.abs[0], golden.status_frame.abs[1]);
        // Our Status: the largest-area node named "Status" (the page-bg frame).
        let our_status = ours
            .iter()
            .filter(|n| n.name == "Status")
            .max_by(|a, b| (a.w * a.h).partial_cmp(&(b.w * b.h)).unwrap())
            .expect("our snapshot resolves a Status frame");
        Align {
            dx: gx - our_status.x0,
            dy: gy - our_status.y0,
        }
    }
    /// Golden top-left mapped into our world space.
    pub fn map(&self, g: &FullNode) -> (f64, f64) {
        (g.abs[0] - self.dx, g.abs[1] - self.dy)
    }
}
