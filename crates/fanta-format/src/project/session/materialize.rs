//! Page / Component materialize profiles (design N3).

use super::error::SessionError;
use super::types::ScopedDoc;
use fanta_doc::{
    CanvasNode, ComponentLibrary, Doc, DocId, History, NodeId, SCHEMA_VERSION, Scene, Selection,
    VariableRegistry, Viewport,
};
use fanta_fnx::{ArtifactIr, ArtifactKind, ImportTarget, import_allowed};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Materialize a page IR into a scoped `Doc` (page nodes only; no masters in scene).
pub fn materialize_page(
    ir: &ArtifactIr,
    project_id: DocId,
    components: ComponentLibrary,
    variables: VariableRegistry,
    active_modes: BTreeMap<fanta_doc::VariableCollectionId, fanta_doc::ModeId>,
) -> Result<ScopedDoc, SessionError> {
    let nodes = scoped_nodes(ir)?;
    validate_import_matrix(ArtifactKind::Page, &nodes)?;
    let root = root_id_from_nodes(&nodes)?;
    ensure_page_root_is_group(&nodes, root)?;
    let mut doc = empty_scoped_doc(project_id);
    doc.pending_layout = crate::project::read::pending_layout_from_nodes(&nodes, Some(root))?;
    doc.components = components;
    doc.variables = variables;
    doc.active_modes = active_modes;
    insert_nodes(&mut doc.scene, &nodes)?;
    doc.pages = vec![root];
    doc.active_page = Some(root);
    strip_page_root_size(&mut doc.scene, root);
    Ok(ScopedDoc { doc, root })
}

/// Scope a page or component out of a whole workspace document without a JSON
/// round trip: the scoped scene shares every node payload with `document`
/// ([`Scene::extract_subtree`]), so the session can later find what an edit
/// touched by pointer. Checks what the IR path checks. `None` for kinds that
/// still go through FNX (graphics).
pub(crate) fn scope_from_document(
    document: &Doc,
    kind: ArtifactKind,
    root: NodeId,
    component: Option<&fanta_doc::ComponentDef>,
) -> Option<Result<ScopedDoc, SessionError>> {
    if !matches!(kind, ArtifactKind::Page | ArtifactKind::Component) {
        return None;
    }
    Some((|| {
        let mut scene = document.scene.extract_subtree(root).ok_or_else(|| {
            SessionError::other(format!("artifact root {root} is not in the document"))
        })?;
        // A registered master owns its subtree even when it is geometrically
        // nested under another page or master. Match the project writer's
        // nearest-component-root partition while retaining the current root.
        for definition in document.components.defs.values() {
            if definition.root != root && scene.contains(definition.root) {
                scene
                    .remove(definition.root)
                    .map_err(|error| SessionError::DocAssemble(error.to_string()))?;
            }
        }
        for id in scene.descendants_of(root) {
            match scene.get(id).map(|node| &node.data) {
                Some(fanta_doc::NodeData::Instance(_))
                    if !import_allowed(kind, ImportTarget::LiveComponent) =>
                {
                    return Err(SessionError::ImportNotAllowed {
                        feature: format!("live Instance in {}", kind.label()),
                    });
                }
                _ => {}
            }
        }
        let mut doc = empty_scoped_doc(document.id);
        doc.variables = document.variables.clone();
        doc.active_modes = document.active_modes.clone();
        match kind {
            ArtifactKind::Page => {
                if !matches!(
                    scene.get(root).map(|node| &node.data),
                    Some(fanta_doc::NodeData::Group(_))
                ) {
                    return Err(SessionError::InvalidSource(
                        "page root must be a Frame (group)".into(),
                    ));
                }
                strip_page_root_size(&mut scene, root);
                doc.components = document.components.clone();
            }
            _ => {
                let mut def = component.cloned().ok_or_else(|| {
                    SessionError::other("component artifact without a definition")
                })?;
                def.root = root;
                let mut components = ComponentLibrary::new();
                components.defs.insert(def.id, def);
                doc.components = components;
            }
        }
        doc.pending_layout = document
            .pending_layout
            .iter()
            .copied()
            .filter(|id| scene.contains(*id))
            .collect();
        doc.scene = scene;
        doc.pages = vec![root];
        doc.active_page = Some(root);
        Ok(ScopedDoc { doc, root })
    })())
}

/// Materialize a component master IR into a scoped `Doc` (master closure only).
pub fn materialize_component(
    ir: &ArtifactIr,
    project_id: DocId,
    def: fanta_doc::ComponentDef,
    variables: VariableRegistry,
    active_modes: BTreeMap<fanta_doc::VariableCollectionId, fanta_doc::ModeId>,
) -> Result<ScopedDoc, SessionError> {
    let nodes = scoped_nodes(ir)?;
    validate_import_matrix(ArtifactKind::Component, &nodes)?;
    let root = root_id_from_nodes(&nodes)?;
    if def.root != root {
        // Prefer the tree root; def.root should match after a good write.
        // If they diverge, trust the source tree and keep def metadata otherwise.
    }
    let mut doc = empty_scoped_doc(project_id);
    doc.pending_layout = crate::project::read::pending_layout_from_nodes(&nodes, None)?;
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

/// An artifact's nodes with its root detached from anything outside the artifact.
///
/// FNX keeps a root's external parent (the sidecar's `root_parent`), so a
/// component master that sits on a page decodes still pointing at that page,
/// both from disk and when a save adopts it from the whole workspace document.
/// The scoped document holds only this artifact, so its root has no parent.
pub(crate) fn scoped_nodes(ir: &ArtifactIr) -> Result<Vec<Value>, SessionError> {
    let mut nodes = ir.to_nodes()?;
    let ids: HashSet<String> = nodes
        .iter()
        .filter_map(|node| node.get("id").and_then(Value::as_str).map(str::to_owned))
        .collect();
    for node in &mut nodes {
        let external = node
            .get("parent")
            .and_then(Value::as_str)
            .is_some_and(|parent| !ids.contains(parent));
        if external {
            node["parent"] = Value::Null;
        }
    }
    Ok(nodes)
}

fn root_id_from_nodes(nodes: &[Value]) -> Result<NodeId, SessionError> {
    subtree_root(nodes)
}

/// A subtree's root: its one node whose parent lies outside the subtree. This
/// is FNX's rule (`fanta_fnx` decodes a subtree the same way), so every
/// session path agrees with the format on which node is the root, whether or
/// not that node still records an external parent.
pub(crate) fn subtree_root(nodes: &[Value]) -> Result<NodeId, SessionError> {
    let ids: HashSet<&str> = nodes
        .iter()
        .filter_map(|node| node.get("id").and_then(Value::as_str))
        .collect();
    let roots: Vec<&Value> = nodes
        .iter()
        .filter(|node| {
            node.get("parent")
                .and_then(Value::as_str)
                .is_none_or(|parent| !ids.contains(parent))
        })
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

pub(crate) fn strip_page_root_size(scene: &mut Scene, root: NodeId) {
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

/// Reject live instances where the artifact kind cannot resolve them.
pub(crate) fn validate_no_live_instance(
    kind: ArtifactKind,
    nodes: &[Value],
) -> Result<(), SessionError> {
    validate_import_matrix(kind, nodes)
}

/// Insert a subtree's nodes into `scene`, parents before children, parsing each
/// node once. A node whose parent lies outside `nodes` hangs off that parent
/// when the scene already holds it (baking under an existing frame) and
/// otherwise becomes a root: FNX keeps a subtree root's external parent (the
/// page a component master sits on), which a scoped or side scene does not
/// contain. Every node is inserted or the call fails; none is dropped.
fn insert_nodes(scene: &mut Scene, nodes: &[Value]) -> Result<(), SessionError> {
    let assemble = |message: String| SessionError::DocAssemble(message);
    let mut parsed: Vec<CanvasNode> = Vec::with_capacity(nodes.len());
    for value in nodes {
        // The same authoring leniency the monodoc reader applies: a hand-written
        // `<Text>`/media element that omits `local_size` (and the width/height
        // sugar) must materialize with an estimated/placeholder box instead of
        // failing the artifact. The session path is the FLAGSHIP authoring path —
        // it cannot be stricter than batch load.
        let mut value = value.clone();
        crate::project::read::backfill_required_geometry(&mut value);
        let node: CanvasNode = serde_json::from_value(value)
            .map_err(|e| assemble(format!("node deserialize: {e}")))?;
        parsed.push(node);
    }
    let mut in_set = HashSet::with_capacity(parsed.len());
    for node in &parsed {
        if !in_set.insert(node.id) {
            return Err(assemble(format!("duplicate node id {}", node.id)));
        }
    }
    let mut children: HashMap<NodeId, Vec<usize>> = HashMap::new();
    let mut roots = Vec::new();
    for (ix, node) in parsed.iter_mut().enumerate() {
        match node.parent {
            Some(parent) if in_set.contains(&parent) => {
                children.entry(parent).or_default().push(ix)
            }
            Some(parent) if scene.get(parent).is_some() => roots.push(ix),
            Some(_) => {
                node.parent = None;
                roots.push(ix);
            }
            None => roots.push(ix),
        }
    }
    let mut order = Vec::with_capacity(parsed.len());
    let mut stack: Vec<usize> = roots.into_iter().rev().collect();
    while let Some(ix) = stack.pop() {
        order.push(ix);
        if let Some(kids) = children.get(&parsed[ix].id) {
            stack.extend(kids.iter().rev());
        }
    }
    if order.len() != parsed.len() {
        return Err(assemble(format!(
            "{} of {} nodes are unreachable from a root (their parents form a cycle)",
            parsed.len() - order.len(),
            parsed.len()
        )));
    }
    let mut slots: Vec<Option<CanvasNode>> = parsed.into_iter().map(Some).collect();
    for ix in order {
        let node = slots[ix]
            .take()
            .ok_or_else(|| assemble("node visited twice".into()))?;
        scene
            .insert(node)
            .map_err(|e| assemble(format!("scene insert: {e}")))?;
    }
    Ok(())
}

// Stored leaf payloads must remain saveable even without a dedicated authoring UI.
fn validate_import_matrix(kind: ArtifactKind, nodes: &[Value]) -> Result<(), SessionError> {
    for n in nodes {
        let ty = n.get("type").and_then(Value::as_str).unwrap_or("");
        if ty == "instance" && !import_allowed(kind, ImportTarget::LiveComponent) {
            return Err(SessionError::ImportNotAllowed {
                feature: format!("live Instance in {}", kind.label()),
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
    let nodes = scoped_nodes(ir)?;
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
