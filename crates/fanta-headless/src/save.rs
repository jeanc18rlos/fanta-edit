//! Saving an edited document back to its project directory the way the
//! editor does: which artifacts an edit touched, and the writer calls that
//! keep every untouched source byte for byte.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    path::Path,
};

use anyhow::{Context as _, Result};
use fanta_doc::{AssetId, Doc, NodeId};
use fanta_format::{ArtifactId, ProjectWriteCache, WorkspaceSession, WriteReport};

/// Write `doc` over the project at `root`, which `session` indexes and
/// `baseline` was last read or saved as. An edit confined to known artifacts
/// rewrites only their files. Anything broader goes through the
/// whole-project writer with the session's retained sources.
#[expect(
    clippy::too_many_arguments,
    reason = "the editor's save inputs, passed apart so callers keep ownership of each"
)]
pub fn save_project(
    root: &Path,
    session: &mut WorkspaceSession,
    doc: &Doc,
    baseline: &Doc,
    raw_assets: &BTreeMap<AssetId, Vec<u8>>,
    assets_changed: bool,
    changed_artifacts: &BTreeSet<ArtifactId>,
    cache: &mut ProjectWriteCache,
) -> Result<WriteReport> {
    if !assets_changed
        && let Some(report) = session.try_save_document_artifacts(
            doc,
            baseline,
            changed_artifacts,
            &BTreeMap::new(),
        )?
    {
        *cache = ProjectWriteCache::default();
        return Ok(report);
    }
    for id in changed_artifacts {
        let Some(indexed_hash) = session.file_index.get(id).copied() else {
            continue;
        };
        if !session.has_indexed_source(id) {
            continue;
        }
        session.open_artifact(id.clone())?;
        anyhow::ensure!(
            session.artifact(id).map(|artifact| artifact.disk_hash) == Some(indexed_hash),
            "{} changed on disk while opening its source",
            id.debug_label(),
        );
    }
    session.adopt_document_shared(doc);
    session.adopt_document_artifacts(doc, changed_artifacts)?;
    let sources = session.validated_source_overrides_for_document(doc)?;
    let expected_disk = session.source_write_preconditions(doc)?;
    let report = fanta_format::write_project_tree_cached_with_sources_checked(
        root,
        doc,
        raw_assets,
        cache,
        &sources,
        &expected_disk,
    )
    .with_context(|| format!("writing Fanta project at {}", root.display()))?;
    let asset_index_hash = report
        .written_hashes
        .get(Path::new("assets/index.json"))
        .copied()
        .or_else(|| session.asset_index_disk_hash())
        .context("the project asset index has no committed hash")?;
    session.accept_written_sources(doc, asset_index_hash, &sources, &report.written_hashes)?;
    if doc
        .pages()
        .iter()
        .any(|id| !session.artifacts.contains_key(&ArtifactId::Page(*id)))
        || doc
            .components
            .defs
            .keys()
            .any(|id| !session.artifacts.contains_key(&ArtifactId::Component(*id)))
    {
        session.reconcile_disk_snapshot()?;
    }
    Ok(report)
}

pub fn all_artifact_ids(document: &Doc) -> BTreeSet<ArtifactId> {
    document
        .pages()
        .iter()
        .copied()
        .map(ArtifactId::Page)
        .chain(
            document
                .components
                .defs
                .keys()
                .copied()
                .map(ArtifactId::Component),
        )
        .collect()
}

pub fn changed_artifact_ids(
    document: &Doc,
    base: Option<&Doc>,
    watermark: Option<(u64, u64)>,
) -> BTreeSet<ArtifactId> {
    let Some((instance, revision)) = watermark else {
        return all_artifact_ids(document);
    };
    if instance != document.scene.instance_id() {
        return all_artifact_ids(document);
    }
    changed_artifact_ids_from_baseline(document, base, document.scene.changes_since(revision))
}

pub fn changed_artifact_ids_from_baseline(
    document: &Doc,
    base: Option<&Doc>,
    scene_delta: Option<fanta_doc::scene::SceneDelta>,
) -> BTreeSet<ArtifactId> {
    let all = || all_artifact_ids(document);
    let Some(base) = base else {
        return all();
    };
    if base.pages() != document.pages()
        || base.variables != document.variables
        || base.active_modes != document.active_modes
        || base.components.sets != document.components.sets
        || base.components.defs.len() != document.components.defs.len()
    {
        return all();
    }
    let mut changed = BTreeSet::new();
    for (id, definition) in &document.components.defs {
        let Some(previous) = base.components.defs.get(id) else {
            return all();
        };
        if previous == definition {
            continue;
        }
        // Master edits bump expansion revisions without changing reference names.
        // They do not require retaining source/IR sessions for unrelated masters.
        let mut previous = previous.clone();
        previous.rev = definition.rev;
        previous.preview_rev = definition.preview_rev;
        if &previous != definition {
            return all();
        }
        changed.insert(ArtifactId::Component(*id));
    }
    if let Some(delta) = scene_delta {
        extend_artifact_owners(
            document,
            &mut changed,
            delta.nodes.into_iter().chain(delta.transforms),
        );
    } else {
        // Master placement and registry changes can affect source ownership
        // outside a changed subtree; retain the conservative path for them.
        for definition in document.components.defs.values() {
            match (
                base.scene.get(definition.root),
                document.scene.get(definition.root),
            ) {
                (Some(previous), Some(current))
                    if previous.parent == current.parent && previous.index == current.index => {}
                _ => return all(),
            }
        }
        let Some(previous_nodes) = changed_structural_nodes(base, document) else {
            return all();
        };
        let Some(current_nodes) = changed_structural_nodes(document, base) else {
            return all();
        };
        // Deleted and moved layers still belong to their previous source as
        // well as their destination. Only looking at the live scene misses it.
        extend_artifact_owners(base, &mut changed, previous_nodes);
        extend_artifact_owners(document, &mut changed, current_nodes);
    }
    changed
}

fn changed_structural_nodes(document: &Doc, other: &Doc) -> Option<Vec<NodeId>> {
    let artifact_roots: HashSet<_> = document
        .pages()
        .iter()
        .copied()
        .chain(
            document
                .components
                .defs
                .values()
                .map(|definition| definition.root),
        )
        .collect();
    if artifact_roots
        .iter()
        .any(|root| !document.scene.contains(*root))
    {
        return None;
    }
    let mut pending = Vec::new();
    for root in document.scene.roots() {
        if !artifact_roots.contains(root) {
            return None;
        }
        pending.push((*root, None));
    }
    let mut visited = HashSet::with_capacity(document.scene.len());
    let mut changed = Vec::new();
    while let Some((id, parent)) = pending.pop() {
        let node = document.scene.get(id)?;
        if node.id != id || node.parent != parent || !visited.insert(id) {
            return None;
        }
        if !document.scene.shares_node(&other.scene, id) {
            changed.push(id);
        }
        if !node.can_have_children() && !document.scene.children_of(Some(id)).is_empty() {
            return None;
        }
        pending.extend(
            document
                .scene
                .children_of(Some(id))
                .iter()
                .map(|child| (*child, Some(id))),
        );
    }
    (visited.len() == document.scene.len()).then_some(changed)
}

pub fn extend_artifact_owners(
    document: &Doc,
    changed: &mut BTreeSet<ArtifactId>,
    nodes: impl IntoIterator<Item = NodeId>,
) {
    let component_roots: HashMap<NodeId, fanta_doc::ComponentId> = document
        .components
        .defs
        .iter()
        .map(|(id, definition)| (definition.root, *id))
        .collect();
    let page_roots: HashSet<NodeId> = document.pages().iter().copied().collect();
    for node in nodes {
        let mut current = Some(node);
        while let Some(id) = current {
            if let Some(component) = component_roots.get(&id) {
                changed.insert(ArtifactId::Component(*component));
                break;
            }
            if page_roots.contains(&id) {
                changed.insert(ArtifactId::Page(id));
                break;
            }
            current = document.scene.get(id).and_then(|node| node.parent);
        }
    }
}
