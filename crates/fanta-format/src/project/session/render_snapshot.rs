//! Compose one artifact and its live component dependencies into a renderable
//! document without falling back to the batch monodoc loader.
//!
//! Editor sessions deliberately keep page scenes and component-master scenes
//! separate. The renderer's instance resolver, however, consumes one `Scene`.
//! This module is the renderer-independent bridge: it builds a derived `Doc`
//! through `Operation`s, preserving the scoped sessions as the authoring
//! sources of truth.

use super::artifact::ArtifactSession;
use super::error::SessionError;
use super::hash::{ContentHash, hash_file_set};
use super::types::ArtifactId;
use super::workspace::WorkspaceSession;
use fanta_doc::{
    ComponentId, ComponentLibrary, Doc, History, InstanceExpansionContext, InstanceNode, NodeData,
    NodeId, Operation, Scene, expand_instance_with_context, resolved_component_with_context,
};
use std::collections::{BTreeMap, VecDeque};

/// A scoped artifact plus exactly the component-master subtrees needed to
/// resolve its live instances.
#[derive(Debug, Clone)]
pub struct ArtifactRenderSnapshot {
    pub artifact: ArtifactId,
    pub doc: Doc,
    pub root: NodeId,
    pub revision: ArtifactRenderRevision,
}

/// Content identity for a composed artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactRenderRevision {
    pub artifact_hash: ContentHash,
    pub shared_hash: ContentHash,
    pub component_sets_hash: ContentHash,
    pub workspace_generation: u64,
    pub dependencies: BTreeMap<ComponentId, ContentHash>,
}

#[derive(Clone)]
struct PendingInstance {
    instance: InstanceNode,
    mode_anchor: NodeId,
    component_chain: Vec<NodeId>,
}

impl WorkspaceSession {
    /// Compose one openable Page, Component, or Graphics artifact into a
    /// renderable document.
    ///
    /// Component dependencies are loaded lazily. An already-open component
    /// session wins over its disk master, so unsaved component canvas/text
    /// commits are visible to dependent page previews without being persisted.
    pub fn render_snapshot(
        &mut self,
        artifact: &ArtifactId,
    ) -> Result<ArtifactRenderSnapshot, SessionError> {
        self.open_artifact(artifact.clone())?;
        let (mut doc, root, artifact_hash) = {
            let session = self.open.get(artifact).ok_or(SessionError::NotOpen)?;
            (
                session.scoped.doc.clone(),
                session.scoped.root,
                working_hash(session)?,
            )
        };

        let effective_components = self.effective_component_library();
        install_component_library(&mut doc, &effective_components)?;

        let mut dependencies = BTreeMap::new();
        let mut queue = VecDeque::new();
        enqueue_instances(&doc.scene, root, &mut queue);
        let mut expansion_count = 0usize;

        while let Some(pending) = queue.pop_front() {
            expansion_count += 1;
            if expansion_count > 1_000_000 {
                return Err(SessionError::other(
                    "component dependency expansion exceeded the safety budget",
                ));
            }

            let context = InstanceExpansionContext::new(
                &doc.variables,
                &doc.active_modes,
                pending.mode_anchor,
            );
            let Some(resolved_component) = resolved_component_with_context(
                &doc.scene,
                &doc.components,
                &pending.instance,
                &context,
            ) else {
                // Dangling instances are valid authoring state. The renderer
                // emits its visible unresolved outline and the harness reports
                // the diagnostic.
                continue;
            };
            let master_root = resolved_component.resolved_root;
            if pending.component_chain.contains(&master_root) {
                let mut cycle = pending
                    .component_chain
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>();
                cycle.push(master_root.to_string());
                return Err(SessionError::other(format!(
                    "component dependency cycle: {}",
                    cycle.join(" -> ")
                )));
            }
            let component = resolved_component.resolved_component;

            if doc.scene.get(master_root).is_none() {
                let (master_scene, master_hash) = self.master_scene_and_hash(component)?;
                append_master_subtree(&mut doc, &master_scene, master_root)?;
                dependencies.insert(component, master_hash);
            } else if Some(component) != artifact_component(artifact) {
                dependencies
                    .entry(component)
                    .or_insert(self.component_working_hash(component)?);
            }

            let expansion_context = InstanceExpansionContext::new(
                &doc.variables,
                &doc.active_modes,
                pending.mode_anchor,
            );
            let mut expanded = expand_instance_with_context(
                &doc.scene,
                &doc.components,
                &pending.instance,
                &expansion_context,
            );
            // Dependency discovery does not need layout, but it does need the
            // override-resolved nested Instance nodes in the expansion.
            let mut chain = pending.component_chain;
            chain.push(master_root);
            for entry in expanded.drain(..) {
                if let NodeData::Instance(instance) = entry.node.data {
                    queue.push_back(PendingInstance {
                        instance,
                        mode_anchor: pending.mode_anchor,
                        component_chain: chain.clone(),
                    });
                }
            }
        }

        doc.history = History::new();
        doc.scene.validate().map_err(|error| {
            SessionError::other(format!("composed render scene failed validation: {error}"))
        })?;

        let component_sets = serde_json::to_vec(&doc.components.sets)
            .map_err(|error| SessionError::other(error.to_string()))?;
        let component_sets_hash =
            hash_file_set(&[("components/sets.json", component_sets.as_slice())]);

        Ok(ArtifactRenderSnapshot {
            artifact: artifact.clone(),
            doc,
            root,
            revision: ArtifactRenderRevision {
                artifact_hash,
                shared_hash: self.shared.disk_hash,
                component_sets_hash,
                workspace_generation: self.workspace_generation,
                dependencies,
            },
        })
    }

    fn effective_component_library(&self) -> ComponentLibrary {
        let mut library = self.components.defs.clone();
        for (id, session) in &self.open {
            let ArtifactId::Component(component) = id else {
                continue;
            };
            if let Some(def) = session.scoped.doc.components.defs.get(component) {
                library.defs.insert(*component, def.clone());
            }
        }
        library
    }

    fn master_scene_and_hash(
        &mut self,
        component: ComponentId,
    ) -> Result<(Scene, ContentHash), SessionError> {
        if let Some(session) = self.open.get(&ArtifactId::Component(component)) {
            return Ok((session.scoped.doc.scene.clone(), working_hash(session)?));
        }
        let scene = self.ensure_master_loaded(component)?.scene.clone();
        let hash = self.component_working_hash(component)?;
        Ok((scene, hash))
    }

    fn component_working_hash(&self, component: ComponentId) -> Result<ContentHash, SessionError> {
        let id = ArtifactId::Component(component);
        if let Some(session) = self.open.get(&id) {
            working_hash(session)
        } else {
            self.file_index
                .get(&id)
                .copied()
                .ok_or(SessionError::MasterNotInScope)
        }
    }
}

fn working_hash(session: &ArtifactSession) -> Result<ContentHash, SessionError> {
    if !session.state.is_dirty() {
        return Ok(session.disk_hash);
    }
    let files = session.project_to_files()?;
    let refs = files
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect::<Vec<_>>();
    Ok(hash_file_set(&refs))
}

fn install_component_library(
    doc: &mut Doc,
    effective: &ComponentLibrary,
) -> Result<(), SessionError> {
    let current_defs = doc.components.defs.clone();
    for (id, current) in current_defs {
        match effective.defs.get(&id) {
            Some(next) if next == &current => {}
            Some(next) => {
                doc.apply(Operation::DeleteComponent {
                    id,
                    def: Box::new(current),
                })
                .map_err(|error| SessionError::other(format!("remove component: {error}")))?;
                doc.apply(Operation::DefineComponent {
                    def: Box::new(next.clone()),
                })
                .map_err(|error| SessionError::other(format!("install component: {error}")))?;
            }
            None => {
                doc.apply(Operation::DeleteComponent {
                    id,
                    def: Box::new(current),
                })
                .map_err(|error| SessionError::other(format!("remove component: {error}")))?;
            }
        }
    }
    for (id, def) in &effective.defs {
        if !doc.components.defs.contains_key(id) {
            doc.apply(Operation::DefineComponent {
                def: Box::new(def.clone()),
            })
            .map_err(|error| SessionError::other(format!("install component: {error}")))?;
        }
    }

    let current_sets = doc.components.sets.clone();
    for (id, current) in current_sets {
        match effective.sets.get(&id) {
            Some(next) if next == &current => {}
            Some(next) => {
                doc.apply(Operation::SetComponentSet {
                    id,
                    old: Box::new(current),
                    new: Box::new(next.clone()),
                })
                .map_err(|error| SessionError::other(format!("install component set: {error}")))?;
            }
            None => {
                doc.apply(Operation::DeleteComponentSet {
                    id,
                    set: Box::new(current),
                })
                .map_err(|error| SessionError::other(format!("remove component set: {error}")))?;
            }
        }
    }
    for (id, set) in &effective.sets {
        if !doc.components.sets.contains_key(id) {
            doc.apply(Operation::DefineComponentSet {
                set: Box::new(set.clone()),
            })
            .map_err(|error| SessionError::other(format!("install component set: {error}")))?;
        }
    }
    Ok(())
}

fn append_master_subtree(
    doc: &mut Doc,
    master_scene: &Scene,
    root: NodeId,
) -> Result<(), SessionError> {
    if master_scene.get(root).is_none() {
        return Err(SessionError::other(format!(
            "component master root {root} is absent from its side scene"
        )));
    }
    let nodes = master_scene
        .descendants_of(root)
        .filter_map(|id| master_scene.get(id).cloned())
        .collect::<Vec<_>>();
    for node in nodes {
        if let Some(existing) = doc.scene.get(node.id) {
            if existing != &node {
                return Err(SessionError::other(format!(
                    "duplicate node id {} differs while composing component masters",
                    node.id
                )));
            }
            continue;
        }
        doc.apply(Operation::create_node(node))
            .map_err(|error| SessionError::other(format!("compose master node: {error}")))?;
    }
    Ok(())
}

fn enqueue_instances(scene: &Scene, root: NodeId, queue: &mut VecDeque<PendingInstance>) {
    for id in scene.descendants_of(root) {
        let Some(NodeData::Instance(instance)) = scene.get(id).map(|node| &node.data) else {
            continue;
        };
        queue.push_back(PendingInstance {
            instance: instance.clone(),
            mode_anchor: id,
            component_chain: Vec::new(),
        });
    }
}

fn artifact_component(artifact: &ArtifactId) -> Option<ComponentId> {
    match artifact {
        ArtifactId::Component(id) => Some(*id),
        _ => None,
    }
}
