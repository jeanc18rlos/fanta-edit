//! NodeGraph — model a ComfyUI / Invoke-style workflow as a canvas node.
//!
//! The graph is just data here; execution lives in `fanta-nodes`. The point of
//! these tests is to confirm the doc-level representation survives round-trips
//! and accepts the topologies a real workflow tool needs.

use fanta_doc::{
    CanvasNode, Doc, Link, LinkId, NodeData, NodeGraph, NodeGraphNode, Operation, WorkflowNode,
    WorkflowNodeId,
};

#[test]
fn three_stage_workflow_round_trips() {
    let load = WorkflowNodeId::new();
    let resize = WorkflowNodeId::new();
    let save = WorkflowNodeId::new();

    let mut graph = NodeGraph::default();
    graph.nodes.insert(
        load,
        WorkflowNode {
            id: load,
            kind: "image.load".into(),
            position: [0.0, 0.0],
            params: serde_json::json!({"path": "hero.png"}),
        },
    );
    graph.nodes.insert(
        resize,
        WorkflowNode {
            id: resize,
            kind: "image.resize".into(),
            position: [240.0, 0.0],
            params: serde_json::json!({"w": 1920, "h": 1080, "mode": "fit"}),
        },
    );
    graph.nodes.insert(
        save,
        WorkflowNode {
            id: save,
            kind: "image.save".into(),
            position: [480.0, 0.0],
            params: serde_json::json!({"format": "webp"}),
        },
    );
    graph.links.push(Link {
        id: LinkId::new(),
        from_node: load,
        from_port: "image".into(),
        to_node: resize,
        to_port: "image".into(),
    });
    graph.links.push(Link {
        id: LinkId::new(),
        from_node: resize,
        from_port: "image".into(),
        to_node: save,
        to_port: "image".into(),
    });
    graph.output = Some(save);

    let mut doc = Doc::new();
    let canvas_node = CanvasNode::new(NodeData::NodeGraph(NodeGraphNode {
        local_size: [600.0, 200.0],
        graph,
        preview: None,
    }));
    let canvas_id = canvas_node.id;
    doc.apply(Operation::create_node(canvas_node)).unwrap();

    let json = doc.to_json_string().unwrap();
    let loaded = Doc::from_json_str(&json).unwrap();

    let NodeData::NodeGraph(ng) = &loaded.scene.get(canvas_id).unwrap().data else {
        panic!("not a node graph variant after load");
    };
    assert_eq!(ng.graph.nodes.len(), 3);
    assert_eq!(ng.graph.links.len(), 2);
    assert_eq!(ng.graph.output, Some(save));
    // The output node's kind survives.
    assert_eq!(ng.graph.nodes[&save].kind, "image.save");
}

#[test]
fn an_unconnected_graph_is_still_valid() {
    // A workflow editor lets you place nodes before linking them; the doc
    // model must not require connectivity to be parseable.
    let only_node = WorkflowNodeId::new();
    let mut graph = NodeGraph::default();
    graph.nodes.insert(
        only_node,
        WorkflowNode {
            id: only_node,
            kind: "image.load".into(),
            position: [50.0, 50.0],
            params: serde_json::json!({}),
        },
    );
    let n = CanvasNode::new(NodeData::NodeGraph(NodeGraphNode {
        local_size: [200.0, 200.0],
        graph,
        preview: None,
    }));
    let mut doc = Doc::new();
    let id = n.id;
    doc.apply(Operation::create_node(n)).unwrap();

    let json = doc.to_json_string().unwrap();
    let loaded = Doc::from_json_str(&json).unwrap();
    let NodeData::NodeGraph(ng) = &loaded.scene.get(id).unwrap().data else {
        panic!("variant lost on round-trip");
    };
    assert_eq!(ng.graph.nodes.len(), 1);
    assert!(ng.graph.links.is_empty());
    assert!(ng.graph.output.is_none());
}
