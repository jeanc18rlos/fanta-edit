//! Round-trip tests: a subtree of node values must survive
//! `tree_from_nodes → print_doc → parse_doc → nodes_from_tree` unchanged.

use super::*;
use serde_json::{Value, json};

/// Compare two node-value sets ignoring order (nodes are keyed by `id`).
fn assert_same_nodes(a: &[Value], b: &[Value]) {
    let key = |v: &Value| v.get("id").and_then(Value::as_str).unwrap().to_owned();
    let mut a = a.to_vec();
    let mut b = b.to_vec();
    a.sort_by_key(key);
    b.sort_by_key(key);
    assert_eq!(a, b, "round-trip changed the node set");
}

fn round_trip(nodes: &[Value]) -> Vec<Value> {
    let tree = tree_from_nodes(nodes).expect("decompose");
    let text = print_doc("Card", &tree.root);
    let root = parse_doc(&text).expect("parse");
    nodes_from_tree(&root, &tree.sidecar, tree.root_parent.as_deref()).expect("recompose")
}

/// A representative subtree: a frame root with nested frame, text, vector —
/// exercising strings, numbers, bools, arrays, nested objects (incl. a color
/// object), and an opaque `meta` blob.
fn sample() -> Vec<Value> {
    vec![
        json!({
            "type": "group", "id": "ROOT0000000000000000000000", "parent": null, "index": 1.0,
            "name": "Card", "opacity": 0.9, "corner_radius": 8.0,
            "background": { "kind": "solid", "color": { "r": 37, "g": 99, "b": 235, "a": 255 } },
            "meta": { "authoredBy": "test", "z": [1, 2, 3] }
        }),
        json!({
            "type": "text", "id": "TEXT0000000000000000000000", "parent": "ROOT0000000000000000000000",
            "index": 1.0, "name": "Title", "content": "Hello, world",
            "style": { "font_family": "Inter", "size_px": 16.0, "weight": 600, "italic": false }
        }),
        json!({
            "type": "vector", "id": "VEC00000000000000000000000", "parent": "ROOT0000000000000000000000",
            "index": 2.0, "name": "Box",
            "fills": [{ "kind": "solid", "color": { "r": 0, "g": 0, "b": 0, "a": 255 } }],
            "corner_radius": 4.0
        }),
        json!({
            "type": "group", "id": "INNER000000000000000000000", "parent": "ROOT0000000000000000000000",
            "index": 3.0, "name": "Inner", "auto_layout": { "mode": "horizontal", "spacing": 12.0 }
        }),
        json!({
            "type": "instance", "id": "INST0000000000000000000000", "parent": "INNER000000000000000000000",
            "index": 1.0, "name": "Btn", "component": "COMP0000000000000000000000",
            "local_size": [120.0, 40.0]
        }),
    ]
}

#[test]
fn full_subtree_round_trips_losslessly() {
    let nodes = sample();
    let back = round_trip(&nodes);
    assert_same_nodes(&nodes, &back);
}

#[test]
fn printed_source_looks_like_react() {
    let nodes = sample();
    let tree = tree_from_nodes(&nodes).unwrap();
    let text = print_doc("Card", &tree.root);
    assert!(text.contains("export default function Card()"));
    assert!(text.contains("<Frame"));
    assert!(text.contains("<Text"));
    assert!(text.contains("<Vector"));
    assert!(text.contains("<Instance"));
    // Nesting: the inner frame's instance child closes inside it.
    assert!(text.contains("</Frame>"));
    // Color sugar: the root background {r:37,g:99,b:235,a:255} renders as hex,
    // not a verbose JSON object.
    assert!(
        text.contains("#2563EB"),
        "color should render as a hex literal: {text}"
    );
    // The opaque ULIDs are NOT in the readable source — they live in the sidecar.
    assert!(
        !text.contains("ROOT0000000000000000000000"),
        "node ids must not leak into the source: {text}"
    );
    assert_eq!(tree.sidecar.len(), nodes.len());
    assert_eq!(tree.sidecar[0].id, "ROOT0000000000000000000000");
}

#[test]
fn single_self_closing_node_round_trips() {
    let nodes = vec![json!({
        "type": "vector", "id": "ONE00000000000000000000000", "parent": null, "index": 1.0,
        "name": "Lonely"
    })];
    assert_same_nodes(&nodes, &round_trip(&nodes));
}

#[test]
fn root_parent_is_preserved() {
    // A component master root whose parent is the (external) components page.
    let nodes = vec![json!({
        "type": "group", "id": "MASTER00000000000000000000", "parent": "PAGEHIDDEN0000000000000000",
        "index": 5.0, "name": "Button"
    })];
    let tree = tree_from_nodes(&nodes).unwrap();
    assert_eq!(
        tree.root_parent.as_deref(),
        Some("PAGEHIDDEN0000000000000000")
    );
    let root = parse_doc(&print_doc("Button", &tree.root)).unwrap();
    let back = nodes_from_tree(&root, &tree.sidecar, tree.root_parent.as_deref()).unwrap();
    assert_same_nodes(&nodes, &back);
}

#[test]
fn every_nodedata_variant_has_a_tag() {
    // The exhaustive match below fails to COMPILE if a `NodeData` variant is
    // added without a matching `TYPE_TAGS` entry — turning a silent
    // unsaveable-document bug into a build error. (The persistence layer also
    // falls back to per-node JSON for an un-tagged type, so this is the
    // early-warning guard, not the only safety net.)
    fn serde_type(n: &fanta_doc::NodeData) -> &'static str {
        use fanta_doc::NodeData::*;
        match n {
            Group(_) => "group",
            Vector(_) => "vector",
            Text(_) => "text",
            Bitmap(_) => "bitmap",
            Video(_) => "video",
            Audio(_) => "audio",
            NodeGraph(_) => "node_graph",
            Model3d(_) => "model3d",
            AiArtifact(_) => "ai_artifact",
            Instance(_) => "instance",
            Embed(_) => "embed",
        }
    }
    let _ = serde_type;
    for ty in [
        "group",
        "vector",
        "text",
        "bitmap",
        "video",
        "audio",
        "node_graph",
        "model3d",
        "ai_artifact",
        "instance",
        "embed",
    ] {
        assert!(tag_for_type(ty).is_some(), "no JSX tag for NodeData `{ty}`");
    }
}

#[test]
fn tag_type_bijection_covers_all_variants() {
    for ty in [
        "group",
        "vector",
        "text",
        "bitmap",
        "video",
        "audio",
        "node_graph",
        "model3d",
        "ai_artifact",
        "instance",
        "embed",
    ] {
        let tag = tag_for_type(ty).expect("tag for type");
        assert_eq!(type_for_tag(tag), Some(ty), "bijection broken for {ty}");
    }
}

#[test]
fn string_attr_with_special_chars_round_trips() {
    let nodes = vec![json!({
        "type": "text", "id": "STR00000000000000000000000", "parent": null, "index": 1.0,
        "name": "Quote \"x\" < & >", "content": "line1\nline2\ttab"
    })];
    assert_same_nodes(&nodes, &round_trip(&nodes));
}

/// The decisive test: round-trip the per-node JSON of a REAL `Doc` (the exact
/// projection `fanta-format`'s project tree writes), proving the codec handles
/// the actual serde shapes (transform `[6]`, flattened `type`, `SmallVec`
/// fills/strokes, `NodeFlags`, fractional `index`) — not just hand-written JSON.
#[test]
fn real_doc_node_projection_round_trips() {
    use fanta_doc::{
        CanvasNode, Color, Doc, GroupNode, NodeData, Operation, Stroke, TextNode, VectorNode,
    };

    let mut doc = Doc::new();
    let mut root = CanvasNode::new(NodeData::Group(GroupNode {
        background: Some(fanta_doc::Fill::solid(Color::WHITE)),
        corner_radius: Some(12.0),
        ..Default::default()
    }));
    root.name = "Card".to_owned();
    root.opacity = 0.95;
    let root_id = root.id;
    doc.apply(Operation::create_node(root)).unwrap();

    let mut rect = CanvasNode::new(NodeData::Vector({
        let mut v = VectorNode::rect_solid(0.0, 0.0, 40.0, 40.0, Color::rgb(0x11, 0x22, 0x33));
        v.corner_radius = Some(4.0);
        v.strokes
            .push(Stroke::solid(Color::rgb(0xAA, 0xBB, 0xCC), 2.0));
        v
    }));
    rect.parent = Some(root_id);
    doc.apply(Operation::create_node(rect)).unwrap();

    let mut text = CanvasNode::new(NodeData::Text(TextNode::new("Hello", 10.0, 10.0)));
    text.parent = Some(root_id);
    doc.apply(Operation::create_node(text)).unwrap();

    // Pull the per-node Values exactly as write_project_tree does.
    let doc_val = serde_json::to_value(&doc).unwrap();
    let nodes: Vec<Value> = doc_val["scene"]["nodes"]
        .as_object()
        .expect("scene.nodes object")
        .values()
        .cloned()
        .collect();
    assert_eq!(nodes.len(), 3);

    let back = round_trip(&nodes);
    assert_same_nodes(&nodes, &back);

    // And the reconstructed values must deserialize back into real CanvasNodes.
    for v in &back {
        serde_json::from_value::<CanvasNode>(v.clone()).expect("reconstructed node deserializes");
    }
}

/// A pure-translation `transform` prints as readable `x`/`y` (not the raw 6-array)
/// and folds back exactly — the position sugar is lossless.
#[test]
fn translation_transform_sugars_to_x_y_and_round_trips() {
    let nodes = vec![json!({
        "type": "vector", "id": "VEC00000000000000000000000", "parent": null, "index": 1.0,
        "name": "Box", "transform": [1.0, 0.0, 0.0, 1.0, 24.0, 36.0]
    })];
    let tree = tree_from_nodes(&nodes).unwrap();
    let text = print_doc("Box", &tree.root);
    assert!(text.contains("x={24.0}"), "x sugar missing:\n{text}");
    assert!(text.contains("y={36.0}"), "y sugar missing:\n{text}");
    assert!(
        !text.contains("transform="),
        "raw transform leaked:\n{text}"
    );

    assert_same_nodes(&nodes, &round_trip(&nodes));
}

/// An identity translation still round-trips (sugars to `x={0} y={0}`), so a
/// node sitting at the origin keeps its `transform` byte-for-byte.
#[test]
fn identity_translation_round_trips() {
    let nodes = vec![json!({
        "type": "group", "id": "ROOT0000000000000000000000", "parent": null, "index": 1.0,
        "name": "Card", "transform": [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
    })];
    assert_same_nodes(&nodes, &round_trip(&nodes));
}

/// A transform carrying rotation/scale (non-identity 2×2) is NOT sugared — it
/// stays a verbatim `transform={[…]}` array and round-trips unchanged.
#[test]
fn non_translation_transform_is_left_verbatim() {
    let nodes = vec![json!({
        "type": "vector", "id": "VEC00000000000000000000000", "parent": null, "index": 1.0,
        "name": "Spun", "transform": [0.0, -1.0, 1.0, 0.0, 5.0, 7.0]
    })];
    let tree = tree_from_nodes(&nodes).unwrap();
    let text = print_doc("Spun", &tree.root);
    assert!(
        text.contains("transform="),
        "rotation should stay a transform array:\n{text}"
    );
    assert!(
        !text.contains("x={"),
        "rotation must not sugar to x/y:\n{text}"
    );

    assert_same_nodes(&nodes, &round_trip(&nodes));
}
