//! [`DocMutGuard`] — tool-friendly `&mut Doc` that restores workspace vars (N13).

use super::artifact::ArtifactSession;
use super::workspace::WorkspaceSharedState;
use fanta_doc::Doc;

/// RAII guard for tool paths that need `&mut Doc`.
///
/// On drop:
/// 1. Restores `doc.variables` / `doc.active_modes` from the workspace if they
///    diverged (never promotes replica mutations to workspace dirty).
/// 2. Marks the artifact `DirtyCanvas` and synchronizes its retained FNX
///    projection when history depth or `modified_at` changed during the
///    borrow.
pub struct DocMutGuard<'a> {
    session: &'a mut ArtifactSession,
    ws: &'a WorkspaceSharedState,
    enter_history_depth: usize,
    enter_modified_at: i64,
}

impl<'a> DocMutGuard<'a> {
    pub fn new(session: &'a mut ArtifactSession, ws: &'a WorkspaceSharedState) -> Self {
        let enter_history_depth = session.scoped.doc.history.undo_depth();
        let enter_modified_at = session.scoped.doc.metadata.modified_at;
        Self {
            session,
            ws,
            enter_history_depth,
            enter_modified_at,
        }
    }

    pub fn doc_mut(&mut self) -> &mut Doc {
        &mut self.session.scoped.doc
    }

    pub fn doc(&self) -> &Doc {
        &self.session.scoped.doc
    }

    /// The simultaneous borrows a `ToolContext` needs: the scoped document and
    /// the session's viewport (N8 — the one presence field that lives outside
    /// `doc`). Splitting through the guard keeps both borrows tied to the
    /// guard's lifetime, so the drop-time revalidation still runs after the
    /// tool gesture releases them:
    ///
    /// ```ignore
    /// let (doc, viewport) = guard.tool_parts();
    /// let ctx = ToolContext { doc, viewport, .. };
    /// ```
    pub fn tool_parts(&mut self) -> (&mut Doc, &mut fanta_doc::Viewport) {
        (&mut self.session.scoped.doc, &mut self.session.viewport)
    }
}

impl Drop for DocMutGuard<'_> {
    fn drop(&mut self) {
        // N13 hard guarantee: tools may call Doc::apply with var ops; restore.
        if self.session.scoped.doc.variables != self.ws.variables
            || self.session.scoped.doc.active_modes != self.ws.active_modes
        {
            self.session.scoped.doc.variables = self.ws.variables.clone();
            self.session.scoped.doc.active_modes = self.ws.active_modes.clone();
        }
        let history_changed =
            self.session.scoped.doc.history.undo_depth() != self.enter_history_depth;
        let modified_changed =
            self.session.scoped.doc.metadata.modified_at != self.enter_modified_at;
        if history_changed || modified_changed {
            if !matches!(
                self.session.state,
                super::types::ArtifactDirty::Conflict(_)
                    | super::types::ArtifactDirty::Invalid { .. }
            ) {
                self.session.state = super::types::ArtifactDirty::DirtyCanvas;
                self.session.ir_stale = true;
                if let Err(error) = self.session.synchronize_retained_source_with_scene() {
                    // Keep the scene that failed to project as the paint
                    // fallback: the gesture's edits are still the user's most
                    // recent work, and `Invalid` is defined as "last good scene
                    // for paint if any" (design doc, SoT table). Discarding it
                    // here would blank the canvas on top of the sync failure.
                    let last_good = Some(Box::new(self.session.scoped.clone()));
                    self.session.state = super::types::ArtifactDirty::Invalid {
                        error: format!("tool/source synchronization failed: {error}"),
                        last_good,
                    };
                }
            }
        }
    }
}

/// Borrow helper for non-persistent presence state (not a second store — N8).
pub struct PresenceView<'a> {
    pub viewport: &'a mut fanta_doc::Viewport,
    pub selection: &'a mut fanta_doc::Selection,
}

impl ArtifactSession {
    /// Tools: `ToolContext { doc: guard.doc_mut(), viewport: &mut session.viewport }`.
    pub fn doc_mut_guard<'w>(&'w mut self, ws: &'w WorkspaceSharedState) -> DocMutGuard<'w> {
        DocMutGuard::new(self, ws)
    }

    pub fn presence_view(&mut self) -> PresenceView<'_> {
        PresenceView {
            viewport: &mut self.viewport,
            selection: &mut self.scoped.doc.selection,
        }
    }
}
