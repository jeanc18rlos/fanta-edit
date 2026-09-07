//! Graphics kind: materialize profile + component-as-recreation bake (N3/N).

use super::artifact::ArtifactSession;
use super::catalog::ComponentCatalog;
use super::error::SessionError;
use super::materialize::{insert_nodes_public, parse_node_id_value, validate_no_live_instance};
use super::types::{ArtifactDirty, ScopedDoc};
use fanta_doc::{
    CanvasNode, ComponentId, Doc, DocId, GroupNode, History, IndexKey, InstanceNode, NodeData,
    NodeId, SCHEMA_VERSION, Selection, VariableRegistry, Viewport, expand_instance,
};
use fanta_fnx::{ArtifactIr, ArtifactKind};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// Materialize a graphics IR (vectors/assets only; no live instances).
pub fn materialize_graphics(
    ir: &ArtifactIr,
    project_id: DocId,
    variables: VariableRegistry,
    active_modes: BTreeMap<fanta_doc::VariableCollectionId, fanta_doc::ModeId>,
) -> Result<ScopedDoc, SessionError> {
    let nodes = ir.to_nodes()?;
    validate_no_live_instance(ArtifactKind::Graphics, &nodes)?;
    let root = root_id(&nodes)?;
    let mut doc = Doc::new();
    doc.id = project_id;
    doc.schema_version = SCHEMA_VERSION;
    doc.history = History::new();
    doc.selection = Selection::new();
    doc.viewport = Viewport::default();
    doc.variables = variables;
    doc.active_modes = active_modes;
    insert_nodes_public(&mut doc.scene, &nodes)?;
    doc.pages = vec![root];
    doc.active_page = Some(root);
    Ok(ScopedDoc { doc, root })
}

fn root_id(nodes: &[Value]) -> Result<NodeId, SessionError> {
    let roots: Vec<&Value> = nodes
        .iter()
        .filter(|n| n.get("parent").map_or(true, Value::is_null))
        .collect();
    if roots.len() != 1 {
        return Err(SessionError::other(format!(
            "graphics expected single root, found {}",
            roots.len()
        )));
    }
    parse_node_id_value(roots[0].get("id").unwrap_or(&Value::Null))
}

/// Bake a component master into static nodes under `parent` (no live Instance).
///
/// Expands against the catalog side scene, strips bindings/reactions, re-ids,
/// and inserts under the graphics root. Marks DirtyCanvas.
pub fn import_component_as_recreation(
    session: &mut ArtifactSession,
    catalog: &mut ComponentCatalog,
    component: ComponentId,
    parent: NodeId,
) -> Result<NodeId, SessionError> {
    if session.kind != ArtifactKind::Graphics {
        return Err(SessionError::ImportNotAllowed {
            feature: "import_component_as_recreation only on Graphics".into(),
        });
    }
    if matches!(
        session.state,
        ArtifactDirty::Conflict(_) | ArtifactDirty::Invalid { .. } | ArtifactDirty::DirtyText
    ) {
        return Err(SessionError::InvalidState(
            "cannot recreate component in this state".into(),
        ));
    }
    if session.scoped.doc.scene.get(parent).is_none() {
        return Err(SessionError::other("parent not in graphics scene"));
    }

    let design_dir = catalog.paths.get(&component).cloned();
    if !catalog.master_scenes.contains_key(&component) {
        return Err(SessionError::other(format!(
            "master {component} not loaded; call WorkspaceSession::ensure_master_loaded first (path={design_dir:?})"
        )));
    }
    if !catalog.defs.defs.contains_key(&component) {
        return Err(SessionError::MasterNotInScope);
    }
    let instance = InstanceNode {
        component,
        overrides: Vec::new(),
        prop_values: BTreeMap::new(),
        derived: Vec::new(),
        local_size: [100.0, 100.0],
    };
    // Expand using side scene + library without holding MasterRef across mut.
    let expanded = {
        let scene = catalog
            .master_scenes
            .get(&component)
            .ok_or(SessionError::MasterNotInScope)?;
        expand_instance(scene, catalog.library(), &instance)
    };
    if expanded.is_empty() {
        return Err(SessionError::other(
            "component expansion empty (missing master nodes in side scene)",
        ));
    }

    // Bake: strip bindings/reactions; nested Instance → leave as group-like by
    // expanding one level only (renderer-style). For v1, convert Instance nodes
    // to empty Groups so graphics stays hermetic.
    let mut baked: Vec<CanvasNode> = Vec::with_capacity(expanded.len());
    let mut root_id = None;
    for exp in expanded {
        let mut node = exp.node;
        node.bindings.clear();
        node.reactions.clear();
        if matches!(node.data, NodeData::Instance(_)) {
            node.data = NodeData::Group(GroupNode::default());
            node.name = format!("{} (static)", node.name);
        }
        if node.parent.is_none() {
            node.parent = Some(parent);
            // Place after existing children.
            let idx = session.scoped.doc.scene.children_of(Some(parent)).len() as u64 + 1;
            node.index = IndexKey::from_raw(idx as f64);
            root_id = Some(node.id);
        }
        // meta recreation tag
        if let Value::Object(meta) = &mut node.meta {
            meta.insert(
                "recreated_from".into(),
                Value::String(component.to_string()),
            );
        } else {
            let mut m = Map::new();
            m.insert(
                "recreated_from".into(),
                Value::String(component.to_string()),
            );
            node.meta = Value::Object(m);
        }
        baked.push(node);
    }

    // Insert parents before children (expanded is typically root-first).
    for node in baked {
        session
            .scoped
            .doc
            .scene
            .insert(node)
            .map_err(|e| SessionError::DocAssemble(format!("bake insert: {e}")))?;
    }
    session.state = ArtifactDirty::DirtyCanvas;
    session.ir_stale = true;
    root_id.ok_or_else(|| SessionError::other("bake produced no root"))
}

/// Load master subtree into a side scene from already-decoded nodes.
pub fn load_master_scene_from_nodes(nodes: &[Value]) -> Result<fanta_doc::Scene, SessionError> {
    let mut scene = fanta_doc::Scene::new();
    insert_nodes_public(&mut scene, nodes)?;
    Ok(scene)
}
