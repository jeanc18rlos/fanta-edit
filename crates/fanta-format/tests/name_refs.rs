//! Full-loop integration tests for name-based references (plan items B3+B4):
//! `component="Button"` and `"$Collection/Name"` binding paths in authored
//! `.fnx` resolve to real ids in the scene, survive canvas edits and saves in
//! their readable spelling, and NEVER leak into the node-map merge substrate.

use fanta_doc::{
    BoundProp, CanvasNode, ComponentDef, ComponentId, Doc, GroupNode, Mode, ModeId, NodeData,
    NodeId, Operation, Variable, VariableCollection, VariableCollectionId, VariableId,
    VariableType, VectorNode,
};
use fanta_format::{ArtifactId, WorkspaceSession, write_project_tree};
use serde_json::Value;
use std::collections::BTreeMap;
use tempfile::tempdir;

struct Fixture {
    dir: tempfile::TempDir,
    page: NodeId,
    button: ComponentId,
    bg_var: VariableId,
}

/// A project with one page, one component master named "Button", and one color
/// variable `Theme/bg` — the vocabulary the authored source below references.
fn fixture() -> Fixture {
    let mut doc = Doc::new();

    let mut page_node = CanvasNode::new(NodeData::Group(GroupNode::default()));
    page_node.name = "Home".into();
    let page = doc.scene.insert(page_node).unwrap();
    doc.add_page(page);

    let mut master = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([120.0, 40.0]),
        ..GroupNode::default()
    }));
    master.name = "Button".into();
    let master_root = doc.scene.insert(master).unwrap();
    let button = ComponentId::new();
    doc.components
        .defs
        .insert(button, ComponentDef::new(button, master_root, "Button"));

    let collection = VariableCollectionId::new();
    let mode = ModeId::new();
    doc.variables.collections.insert(
        collection,
        VariableCollection {
            id: collection,
            name: "Theme".into(),
            modes: vec![Mode {
                id: mode,
                name: "Light".into(),
            }],
            default_mode: mode,
            variable_order: Vec::new(),
        },
    );
    let bg_var = VariableId::new();
    doc.variables.variables.insert(
        bg_var,
        Variable {
            id: bg_var,
            collection,
            name: "bg".into(),
            ty: VariableType::Color,
            values_by_mode: BTreeMap::new(),
            scopes: Vec::new(),
        },
    );

    let dir = tempdir().unwrap();
    write_project_tree(dir.path(), &doc, &BTreeMap::new()).unwrap();
    Fixture {
        dir,
        page,
        button,
        bg_var,
    }
}

/// Author a named `<Instance>` into the page source through the text path.
fn commit_named_source(ws: &mut WorkspaceSession, id: &ArtifactId, component: &str) {
    let project_id = ws.project_id;
    let components = ws.components.defs.clone();
    let variables = ws.shared.variables.clone();
    let modes = ws.shared.active_modes.clone();
    let art = ws.artifact_mut(id).unwrap();
    let text = art.begin_text_edit().unwrap().clone();
    let instance = format!(
        "<Instance name=\"Btn\" component={component:?} local_size={{[120.0, 40.0]}} \
         bindings={{{{\"fill_color\": \"$Theme/bg\"}}}} />"
    );
    // The generated page root is a childless self-closing <Frame …/>; give it
    // the instance as its only child.
    let text = text.replacen("/>", &format!(">{instance}</Frame>"), 1);
    art.set_text(text).unwrap();
    art.commit_text_to_scene(project_id, components, variables, modes)
        .unwrap();
}

fn page_instance(ws: &WorkspaceSession, id: &ArtifactId, page: NodeId) -> (NodeId, CanvasNode) {
    let doc = ws.artifact(id).unwrap().doc();
    let child = *doc
        .scene
        .children_of(Some(page))
        .iter()
        .find(|child| matches!(doc.scene.get(**child).unwrap().data, NodeData::Instance(_)))
        .expect("page has an instance child");
    (child, doc.scene.get(child).unwrap().clone())
}

#[test]
fn named_source_resolves_and_spellings_survive_saves() {
    let f = fixture();
    let mut ws = WorkspaceSession::open(f.dir.path()).unwrap();
    let id = ArtifactId::Page(f.page);
    ws.open_artifact(id.clone()).unwrap();
    commit_named_source(&mut ws, &id, "Button");

    // The scene holds RESOLVED ids, not names.
    let (_, node) = page_instance(&ws, &id, f.page);
    match &node.data {
        NodeData::Instance(instance) => assert_eq!(instance.component, f.button),
        other => panic!("expected instance, got {other:?}"),
    }
    assert_eq!(
        node.bindings.get(&BoundProp::FillColor { index: 0 }),
        Some(&f.bg_var),
        "the $Theme/bg path must resolve to the registry's VariableId"
    );

    // First save writes the author's spelling verbatim (text commit keeps the
    // buffer as the retained source).
    ws.save_artifact(id.clone()).unwrap();
    let fnx_path = f.dir.path().join("pages/home/page.fnx");
    let on_disk = std::fs::read_to_string(&fnx_path).unwrap();
    assert!(
        on_disk.contains("component=\"Button\"")
            && on_disk.contains("bindings={{\"fill_color\": \"$Theme/bg\"}}"),
        "authored spellings must reach disk: {on_disk}"
    );

    // A STRUCTURAL canvas op forces a full canonical reprint — which must
    // re-emit the readable spellings (project layout v4 ⇒ emit_names).
    let mut vector = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        10.0,
        10.0,
        fanta_doc::Color::WHITE,
    )));
    vector.parent = Some(f.page);
    ws.artifact_mut(&id)
        .unwrap()
        .apply(Operation::create_node(vector))
        .unwrap();
    ws.save_artifact(id.clone()).unwrap();
    let reprinted = std::fs::read_to_string(&fnx_path).unwrap();
    assert!(
        reprinted.contains("component=\"Button\"")
            && reprinted.contains("bindings={{\"fill_color\": \"$Theme/bg\"}}")
            && !reprinted.contains(&f.button.0.to_string())
            && !reprinted.contains(&f.bg_var.0.to_string()),
        "canonical reprint must keep names: {reprinted}"
    );

    // Fresh session: the named source loads (names resolve on decode), the
    // scene carries the same resolved ids, and a re-projection of the clean
    // session is byte-identical to disk — the second save is a no-op.
    let mut ws2 = WorkspaceSession::open(f.dir.path()).unwrap();
    ws2.open_artifact(id.clone()).unwrap();
    let (_, node) = page_instance(&ws2, &id, f.page);
    match &node.data {
        NodeData::Instance(instance) => assert_eq!(instance.component, f.button),
        other => panic!("expected instance, got {other:?}"),
    }
    assert_eq!(
        node.bindings.get(&BoundProp::FillColor { index: 0 }),
        Some(&f.bg_var)
    );
    let projected = ws2.artifact(&id).unwrap().project_to_files().unwrap();
    let projected_fnx = projected
        .iter()
        .find(|(name, _)| name == "page.fnx")
        .map(|(_, bytes)| String::from_utf8(bytes.clone()).unwrap())
        .unwrap();
    assert_eq!(
        projected_fnx, reprinted,
        "second save must be byte-stable (print∘parse fixpoint through the session)"
    );

    // And the monolithic reader agrees with the session on the resolved ids.
    let (mono, _) = fanta_format::read_project_tree(f.dir.path()).unwrap();
    let instance_node = mono
        .scene
        .children_of(Some(f.page))
        .iter()
        .find_map(|child| {
            let node = mono.scene.get(*child)?;
            matches!(node.data, NodeData::Instance(_)).then(|| node.clone())
        })
        .expect("read_project_tree kept the instance");
    match &instance_node.data {
        NodeData::Instance(instance) => assert_eq!(instance.component, f.button),
        other => panic!("expected instance, got {other:?}"),
    }
}

/// The merge substrate stays ULID: no name or `$path` may ever reach a
/// `NodeMapEdition` — merges diff canonical JSON, and a name there would make
/// spelling changes look like semantic conflicts.
#[test]
fn node_map_editions_never_contain_names() {
    let f = fixture();
    let mut ws = WorkspaceSession::open(f.dir.path()).unwrap();
    let id = ArtifactId::Page(f.page);
    ws.open_artifact(id.clone()).unwrap();
    commit_named_source(&mut ws, &id, "Button");

    let assert_canonical = |map: &fanta_format::NodeMapEdition, which: &str| {
        for (key, node) in &map.nodes {
            if let Some(component) = node.get("component") {
                assert_eq!(
                    component,
                    &Value::String(f.button.0.to_string()),
                    "{which}: node {key} carries a non-ULID component ref"
                );
            }
            if let Some(bindings) = node.get("bindings") {
                let text = bindings.to_string();
                assert!(
                    !text.contains('$') && text.contains(&f.bg_var.0.to_string()),
                    "{which}: node {key} bindings are not canonical: {text}"
                );
            }
        }
    };

    let session = ws.artifact(&id).unwrap();
    assert_canonical(&session.projected_node_map().unwrap(), "working projection");
    assert_canonical(&session.base_nodes, "base edition");

    ws.save_artifact(id.clone()).unwrap();
    let session = ws.artifact(&id).unwrap();
    assert_canonical(&session.base_nodes, "post-save base edition");
}

#[test]
fn unresolved_component_name_errors_with_suggestion_in_both_paths() {
    let f = fixture();

    // Text-commit path: the parse error surfaces through the session with the
    // nearest-name suggestion, and the artifact enters Invalid (not a crash).
    {
        let mut ws = WorkspaceSession::open(f.dir.path()).unwrap();
        let id = ArtifactId::Page(f.page);
        ws.open_artifact(id.clone()).unwrap();
        let project_id = ws.project_id;
        let components = ws.components.defs.clone();
        let variables = ws.shared.variables.clone();
        let modes = ws.shared.active_modes.clone();
        let art = ws.artifact_mut(&id).unwrap();
        let text = art.begin_text_edit().unwrap().clone();
        let text = text.replacen(
            "/>",
            "><Instance name=\"Btn\" component=\"Buton\" local_size={[10.0, 10.0]} /></Frame>",
            1,
        );
        art.set_text(text).unwrap();
        let error = art
            .commit_text_to_scene(project_id, components, variables, modes)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("unknown component \"Buton\"")
                && error.contains("did you mean \"Button\"?"),
            "unhelpful commit error: {error}"
        );
    }

    // Disk-load path: a hand-edited page.fnx with the same typo fails to open
    // with the same suggestion (both open_artifact and read_project_tree).
    let fnx_path = f.dir.path().join("pages/home/page.fnx");
    let source = std::fs::read_to_string(&fnx_path).unwrap();
    let source = source.replacen(
        "/>",
        ">\n      <Instance name=\"Btn\" component=\"Buton\" local_size={[10.0, 10.0]} />\n    </Frame>",
        1,
    );
    std::fs::write(&fnx_path, source).unwrap();

    let mut ws = WorkspaceSession::open(f.dir.path()).unwrap();
    let error = ws
        .open_artifact(ArtifactId::Page(f.page))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("unknown component \"Buton\"") && error.contains("did you mean \"Button\"?"),
        "unhelpful open error: {error}"
    );
    let error = fanta_format::read_project_tree(f.dir.path())
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("unknown component \"Buton\"") && error.contains("did you mean \"Button\"?"),
        "unhelpful read error: {error}"
    );
}
