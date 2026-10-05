//! Session journal — an ordered, serializable record of every committed edit,
//! tagged with where it came from, plus the runtime plumbing to capture it.
//!
//! The model crate stays pure: it knows how to *emit* committed transactions
//! (via an optional channel sink on [`History`]) and how to *describe* a
//! recorded session ([`SessionJournal`] / [`SessionStep`] / [`Provenance`]). It
//! does **not** know about windows, tools, or the :3846 bridge — the app layer
//! stamps provenance and persists the stream, and [`crate::replay`] consumes a
//! journal headlessly.
//!
//! ## Why a sink, not a poll
//!
//! Every edit funnels through [`History`] commit/undo/redo. A `#[serde(skip)]`
//! channel sender fires one [`JournalEvent`] per committed transaction, in
//! order, regardless of how many land in a single dispatch — so the recorder
//! never has to diff the undo stack (which `undo`/`redo` mutate). When no sink
//! is installed it costs nothing, and a cloned [`Doc`] drops the sink so a
//! throwaway copy can't double-record.
//!
//! [`History`]: crate::history::History

use crate::history::Transaction;
use crate::op::Operation;
use crate::snapshot::SceneSnapshot;
use serde::{Deserialize, Serialize};
use std::sync::mpsc::Sender;

/// A single committed change emitted by [`History`](crate::history::History) to
/// an installed sink. Carries the whole transaction so a recorder can persist
/// its ops without re-reading the undo stack. Provenance is deliberately absent
/// — only the calling boundary (RPC vs. input vs. gesture) knows it.
#[derive(Debug, Clone)]
pub enum JournalEvent {
    /// A fresh edit committed (single-op apply, or a begin/…/commit gesture).
    Commit(Transaction),
    /// Replace the preceding committed transaction with its complete authored
    /// and computed operations. A recorder folds this into the preceding step;
    /// it is not a second edit or an additional undo entry.
    Amend(Transaction),
    /// An undo reverted this previously-committed transaction.
    Undo(Transaction),
    /// A redo re-applied this transaction.
    Redo(Transaction),
}

impl JournalEvent {
    /// Record an edit, folding derived geometry into its authored commit so
    /// replay has the same transaction boundaries as the live undo stack.
    pub fn record(
        self,
        steps: &mut Vec<SessionStep>,
        provenance: Provenance,
        revision: u64,
        ts_ms: u64,
    ) -> Result<(), String> {
        let (provenance, transaction) = match self {
            Self::Commit(transaction) => (provenance, transaction),
            Self::Undo(transaction) => (Provenance::Undo, transaction),
            Self::Redo(transaction) => (Provenance::Redo, transaction),
            Self::Amend(transaction) => {
                let previous = steps
                    .last_mut()
                    .ok_or("layout amendment has no preceding commit")?;
                if matches!(previous.provenance, Provenance::Undo | Provenance::Redo)
                    || previous.label != transaction.label
                    || transaction.ops.len() < previous.ops.len()
                    || serde_json::to_value(&previous.ops).map_err(|error| error.to_string())?
                        != serde_json::to_value(&transaction.ops[..previous.ops.len()])
                            .map_err(|error| error.to_string())?
                {
                    return Err("layout amendment does not extend the preceding commit".into());
                }
                previous.ops = transaction.ops;
                previous.revision = revision;
                return Ok(());
            }
        };
        let seq = steps.last().map_or(0, |step| step.seq.saturating_add(1));
        steps.push(SessionStep {
            seq,
            revision,
            provenance,
            label: transaction.label,
            ops: transaction.ops,
            ts_ms,
        });
        Ok(())
    }
}

/// The sender half installed on a [`History`](crate::history::History). A thin
/// newtype so call sites read clearly and so it can be dropped on clone.
#[derive(Debug, Clone)]
pub struct JournalSink(Sender<JournalEvent>);

impl JournalSink {
    pub fn new(sender: Sender<JournalEvent>) -> Self {
        Self(sender)
    }

    /// Best-effort send. A disconnected/closed channel just means nobody is
    /// draining; recording is instrumentation and must never break editing.
    pub(crate) fn emit(&self, event: JournalEvent) {
        let _ = self.0.send(event);
    }
}

/// Where a recorded step came from. Renderer/tool-agnostic so the model crate
/// carries no dependency on `fanta-tools`; the app serializes concrete events
/// into `detail`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum Provenance {
    /// Real user input dispatched through the tool pipeline. `kind` is
    /// `"pointer"` / `"key"`; `detail` is the serialized event so input-mode
    /// replay can re-feed it.
    Input {
        kind: String,
        detail: serde_json::Value,
    },
    /// A :3846 RPC tool call. `args` is the raw tool arguments.
    Rpc {
        tool: String,
        args: serde_json::Value,
    },
    /// A higher-level semantic gesture (drag/click/marquee) the app synthesized.
    Gesture {
        kind: String,
        detail: serde_json::Value,
    },
    /// History navigation.
    Undo,
    Redo,
    /// Produced by the replay engine while driving the live editor.
    Replay,
    /// A programmatic edit outside any known boundary.
    Unknown,
}

/// One recorded edit: its order, the content-revision after it, where it came
/// from, a human label, and the exact op-deltas to replay it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStep {
    pub seq: u64,
    pub revision: u64,
    pub provenance: Provenance,
    pub label: String,
    pub ops: Vec<Operation>,
    /// Wall-clock millis, display only — never used by replay (which is
    /// logical-order deterministic).
    #[serde(default)]
    pub ts_ms: u64,
}

/// One raw input event captured for input-mode replay — the full pointer/key
/// stream, *including* moves that committed no edit. Distinct from
/// [`SessionStep`] (committed op-deltas): op-replay uses steps, input-replay
/// uses these. `detail` is the serialized `fanta_tools` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedInput {
    /// `"pointer"` or `"key"`.
    pub kind: String,
    pub detail: serde_json::Value,
    #[serde(default)]
    pub ts_ms: u64,
}

/// A complete recorded session: the starting doc, a baseline snapshot, the
/// ordered steps, and (on export) the final snapshot replay must reproduce.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionJournal {
    pub session_id: String,
    /// Canonical `.fant.json` of the doc at `session_start`. Replay rebuilds
    /// from here, so it must capture everything that isn't itself a step
    /// (pages, pre-existing nodes, components, variables).
    pub init_doc_json: String,
    pub init_snapshot: SceneSnapshot,
    pub steps: Vec<SessionStep>,
    /// The active-page snapshot at export. `Some` once a session is finalized;
    /// replay diffs its result against this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_snapshot: Option<SceneSnapshot>,
    /// Viewport + screen captured at start, so input-mode replay resolves
    /// world↔screen coordinates identically. `None` for op-only journals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<JournalViewport>,
    /// Raw input stream for input-mode replay (empty for RPC-only sessions).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_events: Vec<RecordedInput>,
}

/// Minimal viewport state needed to reproduce input-mode replay coordinates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalViewport {
    pub center: [f64; 2],
    pub zoom: f64,
    pub screen: [f64; 2],
}

/// Wall-clock millis since the Unix epoch — display-only timestamps for steps.
pub fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodeId, Transform2D};

    fn authored_transaction() -> Transaction {
        let mut transaction = Transaction::new("Move");
        transaction.push(Operation::SetTransform {
            id: NodeId::new(),
            old: Transform2D::IDENTITY,
            new: Transform2D::translation(20.0, 10.0),
        });
        transaction
    }

    fn with_derived(mut transaction: Transaction) -> Transaction {
        transaction.push(Operation::SetTransform {
            id: NodeId::new(),
            old: Transform2D::IDENTITY,
            new: Transform2D::translation(40.0, 10.0),
        });
        transaction
    }

    #[test]
    fn derived_journal_amendment_folds_without_changing_step_or_provenance() {
        let authored = authored_transaction();
        let first = with_derived(authored.clone());
        let final_transaction = with_derived(first.clone());
        let provenance = Provenance::Gesture {
            kind: "move".into(),
            detail: serde_json::json!({"source": "canvas"}),
        };
        let mut steps = Vec::new();
        JournalEvent::Commit(authored)
            .record(&mut steps, provenance.clone(), 11, 100)
            .expect("record authored edit");
        JournalEvent::Amend(first)
            .record(&mut steps, Provenance::Unknown, 12, 200)
            .expect("record first layout result");
        JournalEvent::Amend(final_transaction.clone())
            .record(&mut steps, Provenance::Unknown, 13, 300)
            .expect("record final layout result");
        assert_eq!(steps.len(), 1);
        let step = steps.first().expect("authored step");
        assert_eq!(step.seq, 0);
        assert_eq!(step.revision, 13);
        assert_eq!(step.ts_ms, 100);
        assert_eq!(step.provenance, provenance);
        assert_eq!(
            serde_json::to_value(&step.ops).expect("step"),
            serde_json::to_value(&final_transaction.ops).expect("final operations")
        );
        JournalEvent::Undo(final_transaction.clone())
            .record(&mut steps, Provenance::Unknown, 14, 400)
            .expect("record undo");
        JournalEvent::Redo(final_transaction)
            .record(&mut steps, Provenance::Unknown, 15, 500)
            .expect("record redo");
        assert_eq!(
            steps.iter().map(|step| step.seq).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(steps.get(1).expect("undo").provenance, Provenance::Undo);
        assert_eq!(steps.get(2).expect("redo").provenance, Provenance::Redo);
    }

    #[test]
    fn derived_journal_amendment_rejects_history_navigation_and_unrelated_commits() {
        let authored = authored_transaction();
        for preceding in [
            JournalEvent::Undo(authored.clone()),
            JournalEvent::Redo(authored.clone()),
            JournalEvent::Commit(authored_transaction()),
        ] {
            let mut steps = Vec::new();
            preceding
                .record(&mut steps, Provenance::Unknown, 1, 1)
                .expect("preceding step");
            let before = serde_json::to_value(&steps).expect("before");
            assert!(
                JournalEvent::Amend(with_derived(authored.clone()))
                    .record(&mut steps, Provenance::Unknown, 2, 2)
                    .is_err()
            );
            assert_eq!(serde_json::to_value(&steps).expect("after"), before);
        }
    }

    #[test]
    fn derived_journal_amendment_rejects_missing_shortened_and_renamed_prefixes() {
        let authored = authored_transaction();
        let mut empty = Vec::new();
        assert!(
            JournalEvent::Amend(with_derived(authored.clone()))
                .record(&mut empty, Provenance::Unknown, 2, 2)
                .is_err()
        );
        assert!(empty.is_empty());
        let mut renamed = with_derived(authored.clone());
        renamed.label = "Different edit".into();
        for invalid in [Transaction::new("Move"), renamed] {
            let mut steps = Vec::new();
            JournalEvent::Commit(authored.clone())
                .record(&mut steps, Provenance::Unknown, 1, 1)
                .expect("authored");
            let before = serde_json::to_value(&steps).expect("before");
            assert!(
                JournalEvent::Amend(invalid)
                    .record(&mut steps, Provenance::Unknown, 2, 2)
                    .is_err()
            );
            assert_eq!(serde_json::to_value(&steps).expect("after"), before);
        }
    }
}
