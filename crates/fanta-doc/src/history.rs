//! Undo/redo via grouped transactions.
//!
//! Operations are grouped into [`Transaction`]s so a multi-op gesture
//! (move-and-resize, paste multiple nodes) is a single undo step from the
//! user's perspective.
//!
//! ## Open transactions
//!
//! The typical UX pattern is "begin transaction → tool drives a drag → end
//! transaction." Mid-drag mutations are recorded onto an open transaction; the
//! transaction commits on pointer-up. [`History::begin`] / [`History::commit`]
//! /  [`History::abort`] support that flow.

use crate::journal::{JournalEvent, JournalSink};
use crate::op::{OpCtx, Operation};
use crate::scene::{Scene, SceneError};
use serde::{Deserialize, Serialize};

/// A grouped, atomic unit of edits — a single undo step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    /// User-visible label ("Move", "Create rectangle", "Generate"). Falls back
    /// to the first op's label when not explicitly set.
    pub label: String,
    /// Ordered ops. Revert applies them in reverse order.
    pub ops: Vec<Operation>,
}

impl Transaction {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            ops: Vec::new(),
        }
    }

    pub fn push(&mut self, op: Operation) {
        if self.label.is_empty() {
            self.label = op.label().to_owned();
        }
        self.ops.push(op);
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

/// The undo/redo stack. Bounded depth keeps memory predictable on long
/// sessions; older transactions drop off the bottom of the undo stack.
///
/// `Clone` is implemented by hand (not derived) so a cloned `History` drops its
/// [`JournalSink`] — a throwaway copy of a [`Doc`](crate::Doc) (export, render
/// snapshot, …) must never record into the live session's channel.
#[derive(Debug, Serialize, Deserialize)]
pub struct History {
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
    /// Soft cap on undo depth. Default 500 — enough for any plausible session,
    /// small enough to keep memory bounded.
    max_depth: usize,
    /// The transaction currently being assembled. `Some` between
    /// `begin` and `commit` / `abort`.
    #[serde(skip)]
    open: Option<Transaction>,
    /// Optional session-journal sink. When `Some`, each committed transaction
    /// (and each undo/redo) is emitted to a recorder. Never serialized, dropped
    /// on clone.
    #[serde(skip)]
    sink: Option<JournalSink>,
}

impl Clone for History {
    fn clone(&self) -> Self {
        Self {
            undo: self.undo.clone(),
            redo: self.redo.clone(),
            max_depth: self.max_depth,
            open: self.open.clone(),
            // A clone must not record to the original's channel.
            sink: None,
        }
    }
}

impl Default for History {
    fn default() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            max_depth: 500,
            open: None,
            sink: None,
        }
    }
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_max_depth(max_depth: usize) -> Self {
        Self {
            max_depth,
            ..Self::default()
        }
    }

    /// Install (or clear) the session-journal sink. Called once by the app after
    /// it owns the live doc, and again after any doc swap (open/checkout).
    pub fn set_journal_sink(&mut self, sink: Option<JournalSink>) {
        self.sink = sink;
    }

    /// Whether a journal sink is currently installed.
    pub fn is_journaling(&self) -> bool {
        self.sink.is_some()
    }

    /// Fire one journal event to the installed sink, if any. Best-effort.
    fn emit(&self, event: JournalEvent) {
        if let Some(sink) = &self.sink {
            sink.emit(event);
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    /// Peek the label of the next undo step, for menu / tooltip wiring.
    pub fn next_undo_label(&self) -> Option<&str> {
        self.undo.last().map(|t| t.label.as_str())
    }

    pub fn next_redo_label(&self) -> Option<&str> {
        self.redo.last().map(|t| t.label.as_str())
    }

    // ---- transaction lifecycle ----------------------------------------------

    /// Open a transaction. If one is already open, it is committed first so
    /// that the caller's intent ("a new gesture starts here") is honored
    /// without dropping in-flight work.
    ///
    /// Infallible: `begin`/`commit` are transaction *markers*, they never
    /// replay ops against the scene. `scene` is threaded through only to keep
    /// the call-site signature stable for tools/app. Returning `Result` here
    /// used to invite `let _ = history.begin(..)` at the call sites, which is
    /// indistinguishable from discarding a real error.
    pub fn begin(&mut self, label: impl Into<String>, scene: &mut Scene) {
        if self.open.is_some() {
            self.commit(scene);
        }
        self.open = Some(Transaction::new(label));
    }

    /// Commit the open transaction. No-op if no transaction is open. Empty
    /// transactions (begin-with-no-ops) are silently discarded.
    ///
    /// Infallible, for the same reason as [`begin`](Self::begin).
    pub fn commit(&mut self, _scene: &mut Scene) {
        if let Some(tx) = self.open.take() {
            if !tx.is_empty() {
                self.emit(JournalEvent::Commit(tx.clone()));
                self.push_undo(tx);
                self.redo.clear();
            }
        }
    }

    /// Discard the open transaction, reverting any ops it has already applied
    /// to `scene`. Used when a tool cancels mid-drag (Esc).
    ///
    /// Stays `&mut Scene` (a marker signature tools/app already call). An open
    /// transaction only ever holds scene-only ops (a drag's `SetTransform`), so
    /// reverting them through an [`OpCtx`] with empty doc-level registries is
    /// safe — those ops never touch components/variables/modes/flow-start.
    pub fn abort(&mut self, scene: &mut Scene) -> Result<(), SceneError> {
        if let Some(mut tx) = self.open.take() {
            let mut components = crate::component::ComponentLibrary::new();
            let mut variables = crate::variables::VariableRegistry::new();
            let mut active_modes = std::collections::BTreeMap::new();
            let mut motion = crate::motion::MotionLibrary::new();
            let mut flow_start = None;
            let mut ctx = OpCtx {
                scene,
                components: &mut components,
                variables: &mut variables,
                active_modes: &mut active_modes,
                motion: &mut motion,
                flow_start: &mut flow_start,
            };
            while let Some(op) = tx.ops.pop() {
                op.revert(&mut ctx)?;
            }
        }
        Ok(())
    }

    /// Apply an operation: append to the open transaction if any; otherwise
    /// wrap it in a fresh single-op transaction and commit immediately.
    /// Either way the redo stack is cleared because the timeline branched.
    ///
    /// Threads an [`OpCtx`] (not bare `&mut Scene`) because an op may touch the
    /// component library / variable registry / mode map / flow start, not only
    /// the scene. `begin`/`commit`/`abort` stay scene-only — they are markers,
    /// they never replay ops — so tool/app call sites are unchanged.
    pub fn apply(&mut self, op: Operation, ctx: &mut OpCtx) -> Result<(), SceneError> {
        op.apply(ctx)?;
        ctx.bump_revs(&op);
        match &mut self.open {
            Some(tx) => tx.push(op),
            None => {
                let mut tx = Transaction::new(op.label());
                tx.push(op);
                self.emit(JournalEvent::Commit(tx.clone()));
                self.push_undo(tx);
                self.redo.clear();
            }
        }
        Ok(())
    }

    /// Apply a whole pre-built transaction as a single undo entry. Used by the
    /// replay engine so a recorded multi-op transaction re-collapses to one undo
    /// step (matching the original) rather than one step per op. Commits any
    /// open transaction first, then emits a `Commit` event like any other edit.
    pub fn apply_transaction(
        &mut self,
        tx: Transaction,
        ctx: &mut OpCtx,
    ) -> Result<(), SceneError> {
        if self.open.is_some() {
            self.commit(ctx.scene);
        }
        if tx.is_empty() {
            return Ok(());
        }
        for op in &tx.ops {
            op.apply(ctx)?;
            ctx.bump_revs(op);
        }
        self.record_applied_transaction(tx);
        Ok(())
    }

    /// Record a transaction whose operations have already been applied to a
    /// validated replacement document.
    ///
    /// This is the commit half of clone-then-swap transaction flows. Callers
    /// must not use it unless every operation in `tx` is already reflected in
    /// the scene and document registries. Unlike cloning [`History`], it keeps
    /// the live journal sink attached.
    pub fn record_applied_transaction(&mut self, tx: Transaction) {
        if let Some(open) = self.open.take()
            && !open.is_empty()
        {
            self.emit(JournalEvent::Commit(open.clone()));
            self.push_undo(open);
            self.redo.clear();
        }
        if tx.is_empty() {
            return;
        }
        self.emit(JournalEvent::Commit(tx.clone()));
        self.push_undo(tx);
        self.redo.clear();
    }

    // ---- undo / redo --------------------------------------------------------

    pub fn undo(&mut self, ctx: &mut OpCtx) -> Result<bool, SceneError> {
        if self.open.is_some() {
            // Implicit commit; "undo while dragging" is a hard UX call. We
            // commit the partial gesture and then undo it, which feels least
            // surprising. `commit` only needs the scene.
            self.commit(ctx.scene);
        }
        let Some(mut tx) = self.undo.pop() else {
            return Ok(false);
        };
        for op in tx.ops.iter().rev() {
            op.revert(ctx)?;
            ctx.bump_revs(op);
        }
        // Drain into redo so an immediate redo replays in original order.
        // We swap-and-restore the ops vector to avoid double-clone.
        let label = std::mem::take(&mut tx.label);
        let tx = Transaction { label, ops: tx.ops };
        self.emit(JournalEvent::Undo(tx.clone()));
        self.redo.push(tx);
        Ok(true)
    }

    pub fn redo(&mut self, ctx: &mut OpCtx) -> Result<bool, SceneError> {
        let Some(tx) = self.redo.pop() else {
            return Ok(false);
        };
        for op in &tx.ops {
            op.apply(ctx)?;
            ctx.bump_revs(op);
        }
        self.emit(JournalEvent::Redo(tx.clone()));
        self.push_undo(tx);
        Ok(true)
    }

    fn push_undo(&mut self, tx: Transaction) {
        self.undo.push(tx);
        // Drop from the bottom if we exceed depth.
        if self.undo.len() > self.max_depth {
            let excess = self.undo.len() - self.max_depth;
            self.undo.drain(..excess);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::component::ComponentLibrary;
    use crate::id::{ModeId, NodeId, VariableCollectionId};
    use crate::node::{CanvasNode, NodeData, VectorNode};
    use crate::transform::Transform2D;
    use crate::variables::VariableRegistry;
    use std::collections::BTreeMap;

    /// Owns the doc slices an [`OpCtx`] borrows. `ctx()` rebuilds the borrow per
    /// call so `history.apply(..., &mut td.ctx())` reads naturally.
    struct TestDoc {
        scene: Scene,
        components: ComponentLibrary,
        variables: VariableRegistry,
        active_modes: BTreeMap<VariableCollectionId, ModeId>,
        motion: crate::motion::MotionLibrary,
        flow_start: Option<NodeId>,
    }
    impl TestDoc {
        fn new() -> Self {
            Self {
                scene: Scene::new(),
                components: ComponentLibrary::new(),
                variables: VariableRegistry::new(),
                active_modes: BTreeMap::new(),
                motion: crate::motion::MotionLibrary::new(),
                flow_start: None,
            }
        }
        fn ctx(&mut self) -> OpCtx<'_> {
            OpCtx {
                scene: &mut self.scene,
                components: &mut self.components,
                variables: &mut self.variables,
                active_modes: &mut self.active_modes,
                motion: &mut self.motion,
                flow_start: &mut self.flow_start,
            }
        }
    }

    fn rect_node() -> CanvasNode {
        CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            10.0,
            10.0,
            Color::WHITE,
        )))
    }

    #[test]
    fn apply_then_undo_then_redo() {
        let mut td = TestDoc::new();
        let mut history = History::new();
        let node = rect_node();
        let id = node.id;

        history
            .apply(Operation::create_node(node), &mut td.ctx())
            .unwrap();
        assert!(td.scene.contains(id));

        assert!(history.undo(&mut td.ctx()).unwrap());
        assert!(!td.scene.contains(id));

        assert!(history.redo(&mut td.ctx()).unwrap());
        assert!(td.scene.contains(id));
    }

    #[test]
    fn transaction_groups_ops_into_one_undo() {
        let mut td = TestDoc::new();
        let mut history = History::new();
        let node = rect_node();
        let id = node.id;
        td.scene.insert(node).unwrap();

        // `begin`/`commit` stay scene-only — proves those signatures are
        // unchanged for the tool/app call sites.
        history.begin("Drag", &mut td.scene);
        history
            .apply(
                Operation::SetTransform {
                    id,
                    old: Transform2D::IDENTITY,
                    new: Transform2D::translation(10.0, 0.0),
                },
                &mut td.ctx(),
            )
            .unwrap();
        history
            .apply(
                Operation::SetTransform {
                    id,
                    old: Transform2D::translation(10.0, 0.0),
                    new: Transform2D::translation(20.0, 5.0),
                },
                &mut td.ctx(),
            )
            .unwrap();
        history.commit(&mut td.scene);

        assert_eq!(history.undo_depth(), 1);
        history.undo(&mut td.ctx()).unwrap();
        assert_eq!(td.scene.get(id).unwrap().transform, Transform2D::IDENTITY);
    }

    #[test]
    fn abort_reverts_partial_gesture() {
        let mut td = TestDoc::new();
        let mut history = History::new();
        let node = rect_node();
        let id = node.id;
        td.scene.insert(node).unwrap();

        history.begin("Drag", &mut td.scene);
        history
            .apply(
                Operation::SetTransform {
                    id,
                    old: Transform2D::IDENTITY,
                    new: Transform2D::translation(50.0, 0.0),
                },
                &mut td.ctx(),
            )
            .unwrap();
        history.abort(&mut td.scene).unwrap();
        // Transform reverted; no entry in undo stack.
        assert_eq!(td.scene.get(id).unwrap().transform, Transform2D::IDENTITY);
        assert_eq!(history.undo_depth(), 0);
    }

    #[test]
    fn new_op_clears_redo_stack() {
        let mut td = TestDoc::new();
        let mut history = History::new();
        let n1 = rect_node();
        let n1_id = n1.id;
        history
            .apply(Operation::create_node(n1), &mut td.ctx())
            .unwrap();
        history.undo(&mut td.ctx()).unwrap();
        assert_eq!(history.redo_depth(), 1);

        let n2 = rect_node();
        history
            .apply(Operation::create_node(n2), &mut td.ctx())
            .unwrap();
        assert_eq!(history.redo_depth(), 0);
        // n1 stays undone (we never redid it before branching).
        assert!(!td.scene.contains(n1_id));
    }

    #[test]
    fn undo_stack_respects_max_depth() {
        let mut td = TestDoc::new();
        let mut history = History::with_max_depth(3);
        for _ in 0..5 {
            let n = rect_node();
            history
                .apply(Operation::create_node(n), &mut td.ctx())
                .unwrap();
        }
        assert_eq!(history.undo_depth(), 3);
    }
}
