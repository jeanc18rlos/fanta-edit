//! Cross-tree / worktree sync (host supplies other root).

use super::error::SessionError;
use super::hash::{ContentHash, hash_file_set};
use super::types::{ArtifactDirty, ArtifactId, FsEvent, SessionEvent};
use super::workspace::WorkspaceSession;
use fanta_doc::{CanvasNode, Doc, DocId, NodeId};
use fanta_fnx::{ArtifactKind, artifact_file_names};
use std::collections::{BTreeMap, BTreeSet};
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
    /// Singleton or asset changes that need the full project reader.
    pub requires_full_reload: bool,
}

/// Whether a disk change was applied directly to the running document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncrementalDocApply {
    Applied,
    RequiresFullReload,
}

/// Small, scene-free baseline for comparing a freshly opened disk session.
#[derive(Debug, Clone)]
pub struct WorkspaceDiskSnapshot {
    project_id: DocId,
    workspace_generation: u64,
    manifest_hash: [u8; 32],
    asset_index_hash: Option<[u8; 32]>,
    workspace_fnx_hash: Option<[u8; 32]>,
    other_singleton_hashes: BTreeMap<PathBuf, Option<[u8; 32]>>,
    shared_hash: ContentHash,
    artifacts: BTreeMap<ArtifactId, (PathBuf, ContentHash)>,
}

impl WorkspaceSession {
    pub fn disk_snapshot(&self) -> WorkspaceDiskSnapshot {
        WorkspaceDiskSnapshot {
            project_id: self.project_id,
            workspace_generation: self.workspace_generation,
            manifest_hash: self.manifest_disk_hash,
            asset_index_hash: self.asset_index_disk_hash,
            workspace_fnx_hash: self.workspace_fnx_disk_hash,
            other_singleton_hashes: self.other_singleton_disk_hashes.clone(),
            shared_hash: self.shared.disk_hash,
            artifacts: self
                .artifacts
                .iter()
                .map(|(id, meta)| (id.clone(), (meta.design_dir.clone(), meta.disk_hash)))
                .collect(),
        }
    }

    /// Compare this freshly opened disk session with a scene-free baseline.
    /// The caller can apply the report to a host-owned document on a worker.
    pub fn reconcile_from_disk_snapshot(
        &mut self,
        baseline: &WorkspaceDiskSnapshot,
    ) -> Result<ApplyReport, SessionError> {
        if self.project_id != baseline.project_id {
            return Err(SessionError::InvalidState(
                "project identity changed during branch switch".into(),
            ));
        }
        if !self.open.is_empty() {
            return Err(SessionError::InvalidState(
                "disk snapshot comparison requires a fresh session".into(),
            ));
        }
        let requires_full_reload = self.manifest_disk_hash != baseline.manifest_hash
            || self.asset_index_disk_hash != baseline.asset_index_hash
            || self.workspace_fnx_disk_hash != baseline.workspace_fnx_hash
            || self.other_singleton_disk_hashes != baseline.other_singleton_hashes;
        if requires_full_reload {
            return Ok(ApplyReport {
                requires_full_reload: true,
                ..ApplyReport::default()
            });
        }

        let mut report = ApplyReport::default();
        self.workspace_generation = baseline.workspace_generation;
        if self.shared.disk_hash != baseline.shared_hash {
            self.workspace_generation = self.workspace_generation.wrapping_add(1);
            report.events.push(SessionEvent::WorkspaceGeneration {
                generation: self.workspace_generation,
            });
        }
        let ids: BTreeSet<_> = baseline
            .artifacts
            .keys()
            .chain(self.artifacts.keys())
            .cloned()
            .collect();
        for id in ids {
            let previous = baseline.artifacts.get(&id);
            let current = self.artifacts.get(&id);
            let event = match (previous, current) {
                (None, Some(_)) => Some(SessionEvent::Created { id: id.clone() }),
                (Some(_), None) => Some(SessionEvent::Removed { id: id.clone() }),
                (Some((path, hash)), Some(meta))
                    if path != &meta.design_dir || hash != &meta.disk_hash =>
                {
                    Some(SessionEvent::Reloaded { id: id.clone() })
                }
                _ => None,
            };
            if let Some(event) = event {
                report.changed.push(id);
                report.events.push(event);
            }
        }
        Ok(report)
    }

    /// Splice the changed page/component subtrees into an existing document.
    /// Unsupported artifact kinds request a full reload before any mutation;
    /// conflicts leave the document unchanged for review.
    pub fn apply_report_to_doc(
        &mut self,
        document: &mut Doc,
        report: &ApplyReport,
    ) -> Result<IncrementalDocApply, SessionError> {
        let mut scratch = document.clone_for_persist();
        scratch.selection = document.selection.clone();
        let (result, scratch) = self.apply_report_to_owned_doc(scratch, report)?;
        if result == IncrementalDocApply::Applied {
            *document = scratch;
        }
        Ok(result)
    }

    /// Apply to a disposable document snapshot. A failed splice may leave this
    /// snapshot partially changed, so errors discard it instead of exposing it
    /// to the running editor.
    pub fn apply_report_to_owned_doc(
        &mut self,
        mut document: Doc,
        report: &ApplyReport,
    ) -> Result<(IncrementalDocApply, Doc), SessionError> {
        let result = self.apply_report_to_scratch_doc(&mut document, report)?;
        Ok((result, document))
    }

    fn apply_report_to_scratch_doc(
        &mut self,
        document: &mut Doc,
        report: &ApplyReport,
    ) -> Result<IncrementalDocApply, SessionError> {
        if report.requires_full_reload {
            return Ok(IncrementalDocApply::RequiresFullReload);
        }
        if report
            .changed
            .iter()
            .any(|id| !matches!(id, ArtifactId::Page(_) | ArtifactId::Component(_)))
        {
            return Ok(IncrementalDocApply::RequiresFullReload);
        }
        if let Some(id) = report.events.iter().find_map(|event| match event {
            SessionEvent::EnteredConflict { id, .. } | SessionEvent::Invalidated { id, .. } => {
                Some(id)
            }
            _ => None,
        }) {
            return Err(SessionError::InvalidState(format!(
                "{} needs review before updating the canvas",
                id.debug_label()
            )));
        }

        let managed_roots: BTreeSet<_> = document
            .pages()
            .iter()
            .copied()
            .chain(document.components.defs.values().map(|def| def.root))
            .collect();
        for id in &report.changed {
            let old_root = match id {
                ArtifactId::Page(page) => Some(*page),
                ArtifactId::Component(component) => {
                    document.components.defs.get(component).map(|def| def.root)
                }
                _ => None,
            };
            if let Some(root) = old_root.filter(|root| document.scene.contains(*root))
                && document
                    .scene
                    .descendants_of(root)
                    .skip(1)
                    .any(|descendant| managed_roots.contains(&descendant))
            {
                return Ok(IncrementalDocApply::RequiresFullReload);
            }
        }

        let mut replacements: Vec<(ArtifactId, NodeId, Vec<CanvasNode>)> = Vec::new();
        for id in &report.changed {
            if !self.artifacts.contains_key(id) {
                continue;
            }
            self.open_artifact(id.clone())?;
            let session = self.open.get(id).ok_or(SessionError::NotOpen)?;
            if matches!(
                session.state,
                ArtifactDirty::Conflict(_) | ArtifactDirty::Invalid { .. }
            ) {
                return Err(SessionError::InvalidState(format!(
                    "{} needs review before updating the canvas",
                    id.debug_label()
                )));
            }
            let nodes = super::materialize::collect_subtree_nodes(session.doc(), session.root())?
                .into_iter()
                .map(|value| {
                    serde_json::from_value(value)
                        .map_err(|error| SessionError::DocAssemble(error.to_string()))
                })
                .collect::<Result<Vec<CanvasNode>, _>>()?;
            replacements.push((id.clone(), session.root(), nodes));
        }

        let metadata = serde_json::from_value(self.shared.metadata.clone())
            .map_err(|error| SessionError::DocAssemble(error.to_string()))?;
        if report.changed.is_empty() {
            document.components = self.components.defs.clone();
            document.variables = self.shared.variables.clone();
            document.active_modes = self.shared.active_modes.clone();
            document.metadata = metadata;
            return Ok(IncrementalDocApply::Applied);
        }

        for id in &report.changed {
            let old_root = match id {
                ArtifactId::Page(page) => Some(*page),
                ArtifactId::Component(component) => {
                    document.components.defs.get(component).map(|def| def.root)
                }
                _ => None,
            };
            if let Some(root) = old_root
                && document.scene.get(root).is_some()
            {
                document
                    .scene
                    .remove(root)
                    .map_err(|error| SessionError::DocAssemble(error.to_string()))?;
            }
        }
        let replacement_nodes = replacements
            .into_iter()
            .flat_map(|(_, _, nodes)| nodes)
            .collect::<Vec<_>>();
        document
            .scene
            .insert_many(replacement_nodes)
            .map_err(|error| SessionError::DocAssemble(error.to_string()))?;

        document.components = self.components.defs.clone();
        document.variables = self.shared.variables.clone();
        document.active_modes = self.shared.active_modes.clone();
        document.metadata = metadata;
        if report
            .changed
            .iter()
            .any(|id| matches!(id, ArtifactId::Page(_)))
        {
            let mut pages = Vec::new();
            for (id, meta) in &self.artifacts {
                if let ArtifactId::Page(page) = id {
                    let header: serde_json::Value = crate::project::layout::read_json_file(
                        &self
                            .root
                            .join(&meta.design_dir)
                            .join(crate::project::layout::PAGE_JSON),
                    )?;
                    let order = header
                        .get("order")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(u64::MAX);
                    pages.push((order, meta.slug.clone(), *page));
                }
            }
            pages.sort_unstable();
            document.pages = pages.into_iter().map(|(_, _, id)| id).collect();
        }
        if document.active_page.is_some_and(|page| {
            !document.pages.contains(&page) && !document.is_component_root(page)
        }) {
            document.active_page = document.pages.first().copied();
        }
        let missing_selection: Vec<NodeId> = document
            .selection
            .iter()
            .copied()
            .filter(|id| document.scene.get(*id).is_none())
            .collect();
        for id in missing_selection {
            document.selection.remove(id);
        }
        Ok(IncrementalDocApply::Applied)
    }

    /// Reconcile a branch switch that already changed this project's files on
    /// disk. Identity comes from artifact headers, so a directory rename keeps
    /// the existing artifact session and its unsaved state.
    pub fn reconcile_disk_snapshot(&mut self) -> Result<ApplyReport, SessionError> {
        let root = self.root.clone();
        crate::project::layout::with_project_read_lock(&root, || {
            self.reconcile_disk_snapshot_locked()
        })
    }

    fn reconcile_disk_snapshot_locked(&mut self) -> Result<ApplyReport, SessionError> {
        let snapshot = WorkspaceSession::open(&self.root)?;
        if snapshot.project_id != self.project_id {
            return Err(SessionError::InvalidState(
                "project identity changed during branch switch".into(),
            ));
        }
        let requires_full_reload = snapshot.manifest_disk_hash != self.manifest_disk_hash
            || snapshot.asset_index_disk_hash != self.asset_index_disk_hash
            || snapshot.workspace_fnx_disk_hash != self.workspace_fnx_disk_hash
            || snapshot.other_singleton_disk_hashes != self.other_singleton_disk_hashes;
        if requires_full_reload {
            return Ok(ApplyReport {
                requires_full_reload: true,
                ..ApplyReport::default()
            });
        }
        self.manifest = snapshot.manifest.clone();
        self.schema_version = snapshot.schema_version;
        let mut changed = BTreeSet::new();
        let mut events = Vec::new();
        if snapshot.shared.disk_hash != self.shared.disk_hash {
            events.extend(self.notify_fs_event(FsEvent::Modified {
                path: self.root.join("doc/variables.json"),
            }));
        }
        let removed: Vec<_> = self
            .artifacts
            .iter()
            .filter(|(id, _)| !snapshot.artifacts.contains_key(*id))
            .map(|(id, meta)| (id.clone(), meta.design_dir.clone()))
            .collect();
        for (id, design_dir) in removed {
            changed.insert(id);
            events.extend(self.notify_fs_event(FsEvent::Removed {
                path: self.root.join(design_dir),
            }));
        }
        for (id, current) in &snapshot.artifacts {
            if self.artifacts.get(id).is_none_or(|previous| {
                previous.design_dir != current.design_dir || previous.disk_hash != current.disk_hash
            }) {
                changed.insert(id.clone());
                events.extend(self.notify_fs_event(FsEvent::Modified {
                    path: self.root.join(&current.design_dir),
                }));
            }
        }
        Ok(ApplyReport {
            changed: changed.into_iter().collect(),
            events,
            unknown_in_other: Vec::new(),
            requires_full_reload,
        })
    }

    /// Diff `other_root` against this project by content hash and route each
    /// changed artifact through the same path as [`Self::notify_fs_event`].
    ///
    /// Host owns git/worktree checkout; engine only reconciles design file sets.
    pub fn sync_from_tree(&mut self, other_root: &Path) -> Result<ApplyReport, SessionError> {
        let other = other_root.canonicalize()?;
        if !crate::project::layout::is_project_dir(&other) {
            return Err(SessionError::NotAProject { path: other });
        }
        if other == self.root {
            return self.reconcile_disk_snapshot();
        }
        let mut report = ApplyReport::default();
        let snapshot = WorkspaceSession::open(&other)?;
        report.unknown_in_other = snapshot
            .artifacts
            .iter()
            .filter(|(id, _)| !self.artifacts.contains_key(*id))
            .map(|(_, meta)| meta.design_dir.clone())
            .collect();
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

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{ComponentDef, ComponentId, GroupNode, NodeData};

    #[test]
    fn changed_page_with_nested_component_master_requires_full_reload() {
        let directory = tempfile::tempdir().expect("project directory");
        let mut document = Doc::new();
        let page = document
            .scene
            .insert(CanvasNode::new(NodeData::Group(GroupNode::default())))
            .expect("page");
        document.add_page(page);

        let mut master = CanvasNode::new(NodeData::Group(GroupNode::default()));
        master.parent = Some(page);
        let master_root = document.scene.insert(master).expect("component master");
        let component_id = ComponentId::new();
        document.components.defs.insert(
            component_id,
            ComponentDef::new(component_id, master_root, "Button"),
        );

        crate::write_project_tree(directory.path(), &document, &BTreeMap::new())
            .expect("project files");
        let mut session = WorkspaceSession::open(directory.path()).expect("project session");
        let report = ApplyReport {
            changed: vec![ArtifactId::Page(page)],
            ..ApplyReport::default()
        };

        let (result, unchanged) = session
            .apply_report_to_owned_doc(document, &report)
            .expect("incremental decision");
        assert_eq!(result, IncrementalDocApply::RequiresFullReload);
        assert!(unchanged.scene.contains(master_root));
    }
}
