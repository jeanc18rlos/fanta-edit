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
    /// An undo reverted this previously-committed transaction.
    Undo(Transaction),
    /// A redo re-applied this transaction.
    Redo(Transaction),
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
