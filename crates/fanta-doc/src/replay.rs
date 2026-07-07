//! Headless, deterministic replay of a recorded [`SessionJournal`].
//!
//! Replay rebuilds a fresh [`Doc`] from the journal's `init_doc_json`, re-applies
//! each step's op-deltas (or undo/redo) in order, then diffs the resulting
//! active-page [`SceneSnapshot`] against the recorded `final_snapshot`. Because
//! `CreateNode` carries its baked [`NodeId`](crate::NodeId) and the snapshot walk
//! is order-deterministic, a faithful recording replays to a byte-identical
//! snapshot — any [`Divergence`] is a real signal (a non-deterministic op, a
//! dropped/garbled step, or a recorder/replayer bug).
//!
//! Replay is side-effect free: a `from_json_str` doc has no journal sink, so
//! re-applying ops never re-records.

use crate::Doc;
use crate::history::Transaction;
use crate::journal::{Provenance, SessionJournal};
use crate::snapshot::{NodeSnapshot, SceneSnapshot};
use serde::{Deserialize, Serialize};

/// One field-level difference between the recorded final snapshot and the
/// replayed one. Positional (paint-ordered): `index` is the node's position in
/// the flat snapshot, which is deterministic for op-replay.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Divergence {
    pub index: usize,
    pub name: String,
    pub field: String,
    pub expected: String,
    pub got: String,
}

/// Outcome of an op-replay: whether it reproduced the recorded final state, how
/// many steps ran, every divergence found, and the replayed doc as JSON (handy
/// for failure packages).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayResult {
    pub ok: bool,
    pub steps_applied: usize,
    pub divergences: Vec<Divergence>,
    pub replayed_doc_json: String,
}

/// Bounds comparison tolerance. Op-replay is exact, but a hair of slack guards
/// against incidental float-format noise without masking real geometry drift —
/// the same philosophy as `compare_screenshot`'s pixel tolerance.
const BOUNDS_TOL: f64 = 0.01;

/// Replay a journal's op-deltas onto a fresh doc and diff against the recorded
/// final snapshot. Returns an error only when the journal itself is unusable
/// (init doc won't parse, an op fails to apply) — a clean replay with mismatches
/// returns `Ok` with `ok: false` and the divergence list.
pub fn replay_ops(journal: &SessionJournal) -> Result<ReplayResult, String> {
    let mut doc =
        Doc::from_json_str(&journal.init_doc_json).map_err(|e| format!("init doc parse: {e}"))?;

    let mut applied = 0usize;
    for step in &journal.steps {
        match step.provenance {
            Provenance::Undo => {
                doc.undo()
                    .map_err(|e| format!("step {}: undo failed: {e:?}", step.seq))?;
            }
            Provenance::Redo => {
                doc.redo()
                    .map_err(|e| format!("step {}: redo failed: {e:?}", step.seq))?;
            }
            _ => {
                let tx = Transaction {
                    label: step.label.clone(),
                    ops: step.ops.clone(),
                };
                doc.apply_transaction(tx)
                    .map_err(|e| format!("step {}: apply failed: {e:?}", step.seq))?;
            }
        }
        applied += 1;
    }

    let replayed = SceneSnapshot::of_active_page(&doc);
    let divergences = match &journal.final_snapshot {
        Some(expected) => diff_snapshots(expected, &replayed),
        None => Vec::new(),
    };
    let replayed_doc_json = doc
        .to_json_string()
        .map_err(|e| format!("serialize replayed doc: {e}"))?;

    Ok(ReplayResult {
        ok: divergences.is_empty(),
        steps_applied: applied,
        divergences,
        replayed_doc_json,
    })
}

/// Structurally diff two resolved snapshots positionally (paint order is
/// deterministic). Reports a node-count mismatch plus every per-field
/// difference for the overlapping prefix.
pub fn diff_snapshots(expected: &SceneSnapshot, got: &SceneSnapshot) -> Vec<Divergence> {
    let mut out = Vec::new();
    if expected.nodes.len() != got.nodes.len() {
        out.push(Divergence {
            index: expected.nodes.len().min(got.nodes.len()),
            name: String::new(),
            field: "node_count".into(),
            expected: expected.nodes.len().to_string(),
            got: got.nodes.len().to_string(),
        });
    }
    for (i, (e, g)) in expected.nodes.iter().zip(got.nodes.iter()).enumerate() {
        diff_node(i, e, g, &mut out);
    }
    out
}

fn diff_node(i: usize, e: &NodeSnapshot, g: &NodeSnapshot, out: &mut Vec<Divergence>) {
    let mut push = |field: &str, expected: String, got: String| {
        out.push(Divergence {
            index: i,
            name: e.name.clone(),
            field: field.into(),
            expected,
            got,
        });
    };

    if e.name != g.name {
        push("name", e.name.clone(), g.name.clone());
    }
    if e.kind != g.kind {
        push("kind", e.kind.clone(), g.kind.clone());
    }
    if e.fill_rgba != g.fill_rgba {
        push(
            "fill_rgba",
            format!("{:?}", e.fill_rgba),
            format!("{:?}", g.fill_rgba),
        );
    }
    if e.stroke_rgba != g.stroke_rgba {
        push(
            "stroke_rgba",
            format!("{:?}", e.stroke_rgba),
            format!("{:?}", g.stroke_rgba),
        );
    }
    if e.text != g.text {
        push("text", format!("{:?}", e.text), format!("{:?}", g.text));
    }
    if e.corner_radius != g.corner_radius {
        push(
            "corner_radius",
            format!("{:?}", e.corner_radius),
            format!("{:?}", g.corner_radius),
        );
    }
    if e.is_mask != g.is_mask {
        push("is_mask", e.is_mask.to_string(), g.is_mask.to_string());
    }

    match (e.abs_bounds, g.abs_bounds) {
        (Some(a), Some(b))
            if a.iter()
                .zip(b.iter())
                .any(|(x, y)| (x - y).abs() > BOUNDS_TOL) =>
        {
            push("abs_bounds", format!("{a:?}"), format!("{b:?}"));
        }
        // One side has bounds and the other doesn't — a structural divergence.
        (a, b) if a.is_some() != b.is_some() => {
            push("abs_bounds", format!("{a:?}"), format!("{b:?}"));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::journal::{JournalEvent, JournalSink, SessionStep, now_ms};
    use crate::node::{CanvasNode, GroupNode, NodeData, VectorNode};
    use crate::op::Operation;
    use crate::style::Fill;
    use crate::transform::Transform2D;
    use smallvec::smallvec;
    use std::sync::mpsc;

    /// Build a doc with one clipped frame as the active page, then return it
    /// ready to record edits *into*. The frame setup is part of the baseline
    /// (it precedes recording), exactly like nodes that exist before
    /// `session_start`.
    fn doc_with_page() -> (Doc, crate::NodeId) {
        let mut doc = Doc::new();
        let frame = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([400.0, 300.0]),
            background: Some(Fill::solid(Color::rgb(20, 20, 20))),
            ..Default::default()
        }));
        let frame_id = frame.id;
        doc.apply(Operation::create_node(frame)).unwrap();
        doc.add_page(frame_id);
        doc.set_active_page(Some(frame_id));
        (doc, frame_id)
    }

    fn rect_in(frame: crate::NodeId, x: f64, y: f64, color: Color) -> CanvasNode {
        let mut rect = CanvasNode::new(NodeData::Vector(VectorNode {
            path: crate::path::PathData::rect(0.0, 0.0, 50.0, 50.0),
            fills: smallvec![Fill::solid(color)],
            strokes: smallvec![],
            corner_radius: None,
            corner_radii: None,
        }));
        rect.parent = Some(frame);
        rect.transform = Transform2D::translation(x, y);
        rect
    }

    /// Record a sequence of edits through the sink and assemble a `SessionJournal`
    /// the same way the app recorder would.
    fn record<F: FnOnce(&mut Doc)>(setup: F) -> SessionJournal {
        let (mut doc, _frame) = doc_with_page();
        let init_doc_json = doc.to_json_string().unwrap();
        let init_snapshot = SceneSnapshot::of_active_page(&doc);

        let (tx, rx) = mpsc::channel();
        doc.set_journal_sink(Some(JournalSink::new(tx)));
        setup(&mut doc);
        doc.set_journal_sink(None);

        let mut steps = Vec::new();
        for (seq, event) in rx.try_iter().enumerate() {
            let seq = seq as u64;
            let (provenance, transaction) = match event {
                JournalEvent::Commit(t) => (Provenance::Unknown, t),
                JournalEvent::Undo(t) => (Provenance::Undo, t),
                JournalEvent::Redo(t) => (Provenance::Redo, t),
            };
            steps.push(SessionStep {
                seq,
                revision: seq,
                provenance,
                label: transaction.label.clone(),
                ops: transaction.ops.clone(),
                ts_ms: now_ms(),
            });
        }

        SessionJournal {
            session_id: "test".into(),
            init_doc_json,
            init_snapshot,
            final_snapshot: Some(SceneSnapshot::of_active_page(&doc)),
            steps,
            viewport: None,
            input_events: Vec::new(),
        }
    }

    #[test]
    fn replay_reproduces_recorded_session() {
        let journal = record(|doc| {
            let f = doc.active_page().unwrap();
            doc.apply(Operation::create_node(rect_in(
                f,
                10.0,
                10.0,
                Color::rgb(255, 0, 0),
            )))
            .unwrap();
            doc.apply(Operation::create_node(rect_in(
                f,
                80.0,
                10.0,
                Color::rgb(0, 255, 0),
            )))
            .unwrap();
        });
        assert_eq!(journal.steps.len(), 2, "two creates recorded");

        let result = replay_ops(&journal).unwrap();
        assert!(
            result.ok,
            "clean replay should have zero divergences, got {:?}",
            result.divergences
        );
        assert_eq!(result.steps_applied, 2);
    }

    #[test]
    fn replay_round_trips_undo_redo() {
        let journal = record(|doc| {
            let f = doc.active_page().unwrap();
            doc.apply(Operation::create_node(rect_in(
                f,
                10.0,
                10.0,
                Color::rgb(255, 0, 0),
            )))
            .unwrap();
            doc.undo().unwrap();
            doc.redo().unwrap();
        });
        // commit + undo + redo = 3 events.
        assert_eq!(journal.steps.len(), 3);
        let result = replay_ops(&journal).unwrap();
        assert!(result.ok, "divergences: {:?}", result.divergences);
    }

    #[test]
    fn equal_index_siblings_order_by_node_id_live_and_rebuilt() {
        // Two sibling rects BOTH at the default IndexKey (equal-index siblings) —
        // the exact case that used to make replay non-deterministic (HashMap-order
        // tiebreak after rebuild_child_index). With the (index, NodeId) total
        // order, child order is by NodeId, identical live AND after a deserialize
        // rebuild — so a journal whose init doc has such siblings replays the same
        // every process.
        let (mut doc, frame) = doc_with_page();
        let a = rect_in(frame, 0.0, 0.0, Color::rgb(255, 0, 0));
        let b = rect_in(frame, 0.0, 0.0, Color::rgb(0, 255, 0));
        let (ida, idb) = (a.id, b.id);
        doc.apply(Operation::create_node(a)).unwrap();
        doc.apply(Operation::create_node(b)).unwrap();

        let mut expected = vec![ida, idb];
        expected.sort();
        assert_eq!(
            doc.scene.children_of(Some(frame)).to_vec(),
            expected,
            "live order must be by NodeId for equal-index siblings"
        );
        let rebuilt = Doc::from_json_str(&doc.to_json_string().unwrap()).unwrap();
        assert_eq!(
            rebuilt.scene.children_of(Some(frame)).to_vec(),
            expected,
            "deserialize rebuild must match the live order"
        );
    }

    #[test]
    fn replay_reports_divergence_when_final_mismatches() {
        let mut journal = record(|doc| {
            let f = doc.active_page().unwrap();
            doc.apply(Operation::create_node(rect_in(
                f,
                10.0,
                10.0,
                Color::rgb(255, 0, 0),
            )))
            .unwrap();
            doc.apply(Operation::create_node(rect_in(
                f,
                80.0,
                10.0,
                Color::rgb(0, 255, 0),
            )))
            .unwrap();
        });
        // Simulate a recorder/replayer disagreement: drop the last step so the
        // replayed doc has fewer nodes than the recorded final snapshot.
        journal.steps.pop();

        let result = replay_ops(&journal).unwrap();
        assert!(!result.ok, "a dropped step must surface as divergence");
        assert!(
            result.divergences.iter().any(|d| d.field == "node_count"),
            "expected a node_count divergence, got {:?}",
            result.divergences
        );
    }
}
