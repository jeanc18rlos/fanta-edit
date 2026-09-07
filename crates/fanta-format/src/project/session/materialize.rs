//! Page / Component materialize profiles (design N3).

use super::error::SessionError;
use super::types::ScopedDoc;
use fanta_doc::{
    CanvasNode, ComponentLibrary, Doc, DocId, History, NodeId, SCHEMA_VERSION, Scene, Selection,
    VariableRegistry, Viewport,
};
use fanta_fnx::{ArtifactIr, ArtifactKind, ImportTarget, import_allowed};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// Materialize a page IR into a scoped `Doc` (page nodes only; no masters in scene).
pub fn materialize_page(
    ir: &ArtifactIr,
    project_id: DocId,
    components: ComponentLibrary,
    variables: VariableRegistry,
    active_modes: BTreeMap<fanta_doc::VariableCollectionId, fanta_doc::ModeId>,
) -> Result<ScopedDoc, SessionError> {
    let nodes = ir.to_nodes()?;
    validate_import_matrix(ArtifactKind::Page, &nodes)?;
    let root = root_id_from_nodes(&nodes)?;
    ensure_page_root_is_group(&nodes, root)?;
    let mut doc = empty_scoped_doc(project_id);
    doc.components = components;
    doc.variables = variables;
    doc.active_modes = active_modes;
    insert_nodes(&mut doc.scene, &nodes)?;
    doc.pages = vec![root];
    doc.active_page = Some(root);
    strip_page_root_size(&mut doc.scene, root);
    Ok(ScopedDoc { doc, root })
}

/// Materialize a component master IR into a scoped `Doc` (master closure only).
pub fn materialize_component(
    ir: &ArtifactIr,
    project_id: DocId,
    def: fanta_doc::ComponentDef,
    variables: VariableRegistry,
    active_modes: BTreeMap<fanta_doc::VariableCollectionId, fanta_doc::ModeId>,
) -> Result<ScopedDoc, SessionError> {
    let nodes = ir.to_nodes()?;
    validate_import_matrix(ArtifactKind::Component, &nodes)?;
    let root = root_id_from_nodes(&nodes)?;
    if def.root != root {
        // Prefer the tree root; def.root should match after a good write.
        // If they diverge, trust the source tree and keep def metadata otherwise.
    }
    let mut doc = empty_scoped_doc(project_id);
    let mut components = ComponentLibrary::new();
    let mut def = def;
    def.root = root;
    components.defs.insert(def.id, def);
    doc.components = components;
    doc.variables = variables;
    doc.active_modes = active_modes;
    insert_nodes(&mut doc.scene, &nodes)?;
    // Component edit scopes the canvas to the master root via active_page.
    doc.pages = vec![root];
    doc.active_page = Some(root);
    Ok(ScopedDoc { doc, root })
}

/// Materialize from a merged node map (after three-way).
pub fn materialize_from_node_map(
    kind: ArtifactKind,
    nodes: &Map<String, Value>,
    header: &Value,
    project_id: DocId,
    components: ComponentLibrary,
    variables: VariableRegistry,
    active_modes: BTreeMap<fanta_doc::VariableCollectionId, fanta_doc::ModeId>,
    fn_name: &str,
) -> Result<(ArtifactIr, ScopedDoc), SessionError> {
    let list: Vec<Value> = nodes.values().cloned().collect();
    let ir = ArtifactIr::from_nodes(kind, fn_name, &list)?;
    let scoped = match kind {
        ArtifactKind::Page => {
            materialize_page(&ir, project_id, components, variables, active_modes)?
        }
        ArtifactKind::Component => {
            let def = parse_component_def(header, &ir)?;
            materialize_component(&ir, project_id, def, variables, active_modes)?
        }
        ArtifactKind::Graphics => {
            super::graphics::materialize_graphics(&ir, project_id, variables, active_modes)?
        }
        other => {
            return Err(SessionError::other(format!(
                "materialize not implemented for kind {}",
                other.label()
            )));
        }
    };
    Ok((ir, scoped))
}

/// Collect all scene nodes under `root` (inclusive) as JSON values.
pub fn collect_subtree_nodes(doc: &Doc, root: NodeId) -> Result<Vec<Value>, SessionError> {
    let mut out = Vec::new();
    collect_dfs(&doc.scene, root, &mut out)?;
    Ok(out)
}

fn collect_dfs(scene: &Scene, id: NodeId, out: &mut Vec<Value>) -> Result<(), SessionError> {
    let node = scene
        .get(id)
        .ok_or_else(|| SessionError::other(format!("missing node {id}")))?;
    out.push(serde_json::to_value(node).map_err(|e| SessionError::other(e.to_string()))?);
    for &child in scene.children_of(Some(id)) {
        collect_dfs(scene, child, out)?;
    }
    Ok(())
}

/// Project a scoped scene back to a node-map edition + IR.
pub fn project_scene_to_node_map(
    scoped: &ScopedDoc,
    kind: ArtifactKind,
    fn_name: &str,
    header: Value,
) -> Result<(ArtifactIr, crate::project::merge::NodeMapEdition), SessionError> {
    let nodes = collect_subtree_nodes(&scoped.doc, scoped.root)?;
    let ir = ArtifactIr::from_nodes(kind, fn_name, &nodes)?;
    let mut map = Map::new();
    for node in &nodes {
        let id = node
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| SessionError::other("node missing id"))?
            .to_owned();
        map.insert(id, node.clone());
    }
    Ok((ir, crate::project::merge::NodeMapEdition::new(header, map)))
}

fn empty_scoped_doc(project_id: DocId) -> Doc {
    let mut doc = Doc::new();
    doc.id = project_id;
    doc.schema_version = SCHEMA_VERSION;
    doc.history = History::new();
    doc.selection = Selection::new();
    doc.viewport = Viewport::default();
    doc
}

fn root_id_from_nodes(nodes: &[Value]) -> Result<NodeId, SessionError> {
    let roots: Vec<&Value> = nodes
        .iter()
        .filter(|n| n.get("parent").map_or(true, Value::is_null))
        .collect();
    if roots.len() != 1 {
        return Err(SessionError::other(format!(
            "expected single root, found {}",
            roots.len()
        )));
    }
    let id = roots[0]
        .get("id")
        .ok_or_else(|| SessionError::other("root missing id"))?;
    parse_node_id(id)
}

/// Parse a node id from JSON (bare ULID string) or display form (`n_…`).
fn parse_node_id(value: &Value) -> Result<NodeId, SessionError> {
    serde_json::from_value(value.clone())
        .map_err(|e| SessionError::other(format!("bad node id: {e}")))
}

fn ensure_page_root_is_group(nodes: &[Value], root: NodeId) -> Result<(), SessionError> {
    let node = nodes
        .iter()
        .find(|n| n.get("id").and_then(|v| parse_node_id(v).ok()) == Some(root));
    let Some(node) = node else {
        return Err(SessionError::other("root node not in list"));
    };
    match node.get("type").and_then(Value::as_str) {
        Some("group") => Ok(()),
        other => Err(SessionError::InvalidSource(format!(
            "page root must be a Frame (group), found {other:?}"
        ))),
    }
}

fn strip_page_root_size(scene: &mut Scene, root: NodeId) {
    if let Some(node) = scene.get_mut(root)
        && let fanta_doc::NodeData::Group(g) = &mut node.data
    {
        g.clip_size = None;
        g.local_size = None;
    }
}

/// Insert a flat list of node JSON values into a scene (parents before children).
pub(crate) fn insert_nodes_public(scene: &mut Scene, nodes: &[Value]) -> Result<(), SessionError> {
    insert_nodes(scene, nodes)
}

/// Parse a node id from JSON (bare ULID string) or display form.
pub(crate) fn parse_node_id_value(value: &Value) -> Result<NodeId, SessionError> {
    parse_node_id(value)
}

/// Reject live instances / model3d for the given kind.
pub(crate) fn validate_no_live_instance(
    kind: ArtifactKind,
    nodes: &[Value],
) -> Result<(), SessionError> {
    validate_import_matrix(kind, nodes)
}

fn insert_nodes(scene: &mut Scene, nodes: &[Value]) -> Result<(), SessionError> {
    // Insert parents before children: sort so roots first, then by depth.
    let mut ordered = nodes.to_vec();
    // The same authoring leniency the monodoc reader applies: a hand-written
    // `<Text>`/media element that omits `local_size` (and the width/height
    // sugar) must materialize with an estimated/placeholder box instead of
    // failing the artifact. The session path is the FLAGSHIP authoring path —
    // it cannot be stricter than batch load.
    for value in &mut ordered {
        crate::project::read::backfill_required_geometry(value);
    }
    ordered.sort_by_key(|n| {
        // crude: null parent first
        if n.get("parent").map_or(true, Value::is_null) {
            0u8
        } else {
            1
        }
    });
    // Multi-pass until all inserted (handles deeper trees).
    let mut pending = ordered;
    let mut guard = 0;
    while !pending.is_empty() && guard < 64 {
        guard += 1;
        let batch_len = pending.len();
        let mut next = Vec::new();
        for value in pending {
            let node: CanvasNode = serde_json::from_value(value.clone())
                .map_err(|e| SessionError::DocAssemble(format!("node deserialize: {e}")))?;
            let parent_ok = match node.parent {
                None => true,
                Some(p) => scene.get(p).is_some(),
            };
            if parent_ok {
                // insert with fixed id: Scene::insert uses node's id
                scene
                    .insert(node)
                    .map_err(|e| SessionError::DocAssemble(format!("scene insert: {e}")))?;
            } else {
                next.push(value);
            }
        }
        if next.len() == batch_len && !next.is_empty() {
            // force insert remaining (parent missing — still insert for recovery)
            for value in next {
                let node: CanvasNode = serde_json::from_value(value)
                    .map_err(|e| SessionError::DocAssemble(format!("node deserialize: {e}")))?;
                let _ = scene.insert(node);
            }
            break;
        }
        pending = next;
    }
    Ok(())
}

fn validate_import_matrix(kind: ArtifactKind, nodes: &[Value]) -> Result<(), SessionError> {
    for n in nodes {
        let ty = n.get("type").and_then(Value::as_str).unwrap_or("");
        if ty == "instance" && !import_allowed(kind, ImportTarget::LiveComponent) {
            return Err(SessionError::ImportNotAllowed {
                feature: format!("live Instance in {}", kind.label()),
            });
        }
        if ty == "model3d" {
            // 3D ignored / forbidden in session product surface
            return Err(SessionError::ImportNotAllowed {
                feature: "model3d".into(),
            });
        }
    }
    Ok(())
}

fn parse_component_def(
    header: &Value,
    ir: &ArtifactIr,
) -> Result<fanta_doc::ComponentDef, SessionError> {
    if let Ok(def) = serde_json::from_value::<fanta_doc::ComponentDef>(header.clone()) {
        return Ok(def);
    }
    // Minimal def from tree root.
    let nodes = ir.to_nodes()?;
    let root = root_id_from_nodes(&nodes)?;
    let name = nodes
        .iter()
        .find(|n| n.get("id").and_then(|v| parse_node_id(v).ok()) == Some(root))
        .and_then(|n| n.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("Component")
        .to_owned();
    let id = header
        .get("id")
        .and_then(Value::as_str)
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(fanta_doc::ComponentId::new);
    Ok(fanta_doc::ComponentDef::new(id, root, name))
}
