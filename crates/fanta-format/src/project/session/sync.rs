//! Cross-tree / worktree sync (host supplies other root).

use super::error::SessionError;
use super::hash::hash_file_set;
use super::types::{ArtifactId, FsEvent, SessionEvent};
use super::workspace::WorkspaceSession;
use fanta_fnx::{ArtifactKind, artifact_file_names};
use std::path::{Path, PathBuf};

/// Report of applying another tree onto the open session.
#[derive(Debug, Clone, Default)]
pub struct ApplyReport {
    /// Artifacts whose on-disk hash differed and were routed through reload/merge.
    pub changed: Vec<ArtifactId>,
    /// Session events produced while applying.
    pub events: Vec<SessionEvent>,
    /// Paths present only in `other_root` (not yet in this project index).
    pub unknown_in_other: Vec<PathBuf>,
}

impl WorkspaceSession {
    /// Diff `other_root` against this project by content hash and route each
    /// changed artifact through the same path as [`Self::notify_fs_event`].
    ///
    /// Host owns git/worktree checkout; engine only reconciles design file sets.
    pub fn sync_from_tree(&mut self, other_root: &Path) -> Result<ApplyReport, SessionError> {
        let other = other_root.canonicalize()?;
        if !crate::project::layout::is_project_dir(&other) {
            return Err(SessionError::NotAProject { path: other });
        }
        let mut report = ApplyReport::default();
        let ids: Vec<ArtifactId> = self.artifacts.keys().cloned().collect();
        for id in ids {
            let Some(meta) = self.artifacts.get(&id).cloned() else {
                continue;
            };
            let ours = self.root.join(&meta.design_dir);
            let theirs = other.join(&meta.design_dir);
            if !theirs.is_dir() {
                // Deleted in other — simulate remove event on our copy if open/dirty
                let events = self.notify_fs_event(FsEvent::Removed {
                    path: ours.join(first_file_name(meta.kind)),
                });
                if !events.is_empty() {
                    report.changed.push(id);
                    report.events.extend(events);
                }
                continue;
            }
            let our_hash = hash_dir_file_set(&ours, meta.kind)?;
            let their_hash = hash_dir_file_set(&theirs, meta.kind)?;
            if our_hash == their_hash {
                continue;
            }
            // Copy other bytes into our design dir (host may have already done this;
            // when called with a pure worktree path we materialize their content).
            copy_file_set(&theirs, &ours, meta.kind)?;
            let events = self.notify_fs_event(FsEvent::Modified {
                path: ours.join(first_file_name(meta.kind)),
            });
            report.changed.push(id);
            report.events.extend(events);
        }
        Ok(report)
    }
}

fn first_file_name(kind: ArtifactKind) -> &'static str {
    artifact_file_names(kind)
        .first()
        .copied()
        .unwrap_or("page.fnx")
}

fn hash_dir_file_set(
    dir: &Path,
    kind: ArtifactKind,
) -> Result<super::hash::ContentHash, SessionError> {
    let mut pairs: Vec<(String, Vec<u8>)> = Vec::new();
    for name in artifact_file_names(kind) {
        let path = dir.join(name);
        if path.is_file() {
            pairs.push(((*name).to_owned(), std::fs::read(path)?));
        }
    }
    Ok(hash_file_set(
        &pairs
            .iter()
            .map(|(n, b)| (n.as_str(), b.as_slice()))
            .collect::<Vec<_>>(),
    ))
}

fn copy_file_set(from: &Path, to: &Path, kind: ArtifactKind) -> Result<(), SessionError> {
    std::fs::create_dir_all(to)?;
    for name in artifact_file_names(kind) {
        let src = from.join(name);
        if src.is_file() {
            std::fs::copy(&src, to.join(name))?;
        }
    }
    Ok(())
}
