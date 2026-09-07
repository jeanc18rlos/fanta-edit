//! Editor hot-path latency gate (ARCHITECTURE.md §8.1).
//!
//! Guards the cost of one non-structural `ArtifactSession::apply` — the
//! per-pointer-move unit of a canvas drag — at editor scale. The gate exists
//! because the clone-then-swap transaction path is deliberately correct-first;
//! this test is the tripwire that keeps "correct" from silently regressing to
//! "unusable" as the session grows features.
//!
//! Run explicitly (wall-clock assertions are meaningless under `--debug` or a
//! loaded CI box):
//!
//! ```sh
//! cargo test -p fanta-format --release --test session_latency_gate -- --ignored --nocapture
//! ```
//!
//! Baselines measured 2026-07-30 on an M-series MacBook (release build),
//! BEFORE the revision-watermark fix: p50 3.4ms @111 nodes, 33.7ms @1k,
//! 328ms @10k, 1.65s @50k — O(scene) because every apply re-projected the
//! full scene twice inside `synchronize_retained_source_with_scene` even when
//! nothing changed. Core `Doc::apply` measured ~200ns flat at every scale.

use fanta_doc::{
    CanvasNode, Color, Doc, GroupNode, NodeData, NodeId, Operation, Transform2D, VectorNode,
};
use fanta_format::{ArtifactId, WorkspaceSession, write_project_tree};
use std::collections::BTreeMap;
use std::time::Instant;

/// p95 budget for one non-structural apply at 10k nodes, in milliseconds.
/// 8ms leaves half a 60fps frame for everything else in a drag tick.
const P95_BUDGET_MS: f64 = 8.0;
const NODES_CLUSTERS: usize = 100;
const NODES_PER_CLUSTER: usize = 100; // 100×100 + clusters + page ≈ 10.1k nodes
const SAMPLES: usize = 40;

/// Spatially-clustered scene with fixed ids (no wall-clock ULIDs) so the
/// projected tree and traversal order are deterministic across runs.
fn clustered_project(clusters: usize, per_cluster: usize) -> (tempfile::TempDir, NodeId) {
    const CLUSTER_SPACING: f64 = 512.0;
    const CELL: f64 = 18.0;
    let mut document = Doc::new();
    let mut next = 1u128;
    let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
    page.id = NodeId::from_u128(next);
    next += 1;
    page.name = "Latency gate".into();
    let page_id = page.id;
    document.apply(Operation::create_node(page)).unwrap();
    document.add_page(page_id);

    let ccols = (clusters as f64).sqrt().ceil() as usize;
    let vcols = (per_cluster as f64).sqrt().ceil() as usize;
    let extent = vcols as f64 * CELL;
    for ci in 0..clusters {
        let mut cluster = CanvasNode::new(NodeData::Group(GroupNode {
            local_size: Some([extent, extent]),
            ..GroupNode::default()
        }));
        cluster.id = NodeId::from_u128(next);
        next += 1;
        cluster.parent = Some(page_id);
        cluster.transform = Transform2D::translation(
            (ci % ccols) as f64 * CLUSTER_SPACING,
            (ci / ccols) as f64 * CLUSTER_SPACING,
        );
        let cluster_id = cluster.id;
        document.apply(Operation::create_node(cluster)).unwrap();
        for vi in 0..per_cluster {
            let mut vector = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
                0.0,
                0.0,
                12.0,
                12.0,
                Color::rgb(0x31, 0x7A, 0xF5),
            )));
            vector.id = NodeId::from_u128(next);
            next += 1;
            vector.parent = Some(cluster_id);
            vector.transform =
                Transform2D::translation((vi % vcols) as f64 * CELL, (vi / vcols) as f64 * CELL);
            document.apply(Operation::create_node(vector)).unwrap();
        }
    }
    let dir = tempfile::tempdir().unwrap();
    write_project_tree(dir.path(), &document, &BTreeMap::new()).unwrap();
    (dir, page_id)
}

#[test]
#[ignore = "wall-clock gate; run with --release --ignored on a quiet machine"]
fn non_structural_apply_p95_stays_within_frame_budget_at_10k_nodes() {
    let (dir, page) = clustered_project(NODES_CLUSTERS, NODES_PER_CLUSTER);
    let mut ws = WorkspaceSession::open(dir.path()).expect("open workspace");
    let id = ArtifactId::Page(page);
    ws.open_artifact(id.clone()).expect("open artifact");

    let target = {
        let scene = &ws.artifact(&id).unwrap().doc().scene;
        scene
            .descendants_of(page)
            .find(|n| matches!(scene.get(*n).map(|n| &n.data), Some(NodeData::Vector(_))))
            .expect("clustered project has vectors")
    };

    // A simulated pointer drag: successive SetTransform ops on one leaf.
    let mut samples_ms = Vec::with_capacity(SAMPLES);
    for i in 0..SAMPLES as u32 {
        let old = ws
            .artifact(&id)
            .unwrap()
            .doc()
            .scene
            .get(target)
            .unwrap()
            .transform;
        let new = Transform2D::translation(f64::from(i), f64::from(i));
        let art = ws.artifact_mut(&id).unwrap();
        let started = Instant::now();
        art.apply(Operation::SetTransform {
            id: target,
            old,
            new,
        })
        .expect("apply transform");
        samples_ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    samples_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = samples_ms[samples_ms.len() / 2];
    let p95 = samples_ms[(samples_ms.len() as f64 * 0.95) as usize];
    println!(
        "non-structural apply @~10k nodes: p50 {p50:.2}ms  p95 {p95:.2}ms  budget {P95_BUDGET_MS}ms"
    );
    assert!(
        p95 <= P95_BUDGET_MS,
        "p95 {p95:.2}ms exceeds the {P95_BUDGET_MS}ms editor frame budget \
         (p50 {p50:.2}ms) — the session edit path has regressed to O(scene)"
    );
}
