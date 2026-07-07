//! AI artifact lineage — re-rolling produces a sibling chain, not a destructive
//! overwrite. This is the Krea/Invoke design point that makes "AI in the doc"
//! different from "AI in a sidebar."

use fanta_doc::{
    AiArtifactNode, AssetId, CanvasNode, Doc, GenerationStatus, NodeData, NodeId, Operation,
};

fn ai_node(prompt: &str, lineage_parent: Option<NodeId>, seed: u64) -> CanvasNode {
    CanvasNode::new(NodeData::AiArtifact(AiArtifactNode {
        local_size: [512.0, 512.0],
        prompt: prompt.into(),
        model: "flux-pro".into(),
        params: serde_json::json!({"steps": 30}),
        inputs: vec![],
        lineage_parent,
        output: Some(AssetId::new()),
        status: GenerationStatus::Done,
        seed: Some(seed),
    }))
}

#[test]
fn rerolls_form_a_lineage_chain() {
    let mut doc = Doc::new();

    let v1 = ai_node("a robot in the rain", None, 1);
    let v1_id = v1.id;
    doc.apply(Operation::create_node(v1)).unwrap();

    let v2 = ai_node("a robot in the rain", Some(v1_id), 2);
    let v2_id = v2.id;
    doc.apply(Operation::create_node(v2)).unwrap();

    let v3 = ai_node("a robot in the rain", Some(v2_id), 3);
    let v3_id = v3.id;
    doc.apply(Operation::create_node(v3)).unwrap();

    // Walk the lineage from v3 back to v1.
    let mut chain = vec![v3_id];
    let mut cursor = v3_id;
    while let NodeData::AiArtifact(a) = &doc.scene.get(cursor).unwrap().data {
        match a.lineage_parent {
            Some(parent) => {
                chain.push(parent);
                cursor = parent;
            }
            None => break,
        }
    }
    assert_eq!(chain, vec![v3_id, v2_id, v1_id]);

    // Round-trip preserves lineage.
    let json = doc.to_json_string().unwrap();
    let loaded = Doc::from_json_str(&json).unwrap();
    if let NodeData::AiArtifact(a) = &loaded.scene.get(v3_id).unwrap().data {
        assert_eq!(a.lineage_parent, Some(v2_id));
        assert_eq!(a.seed, Some(3));
    } else {
        panic!("variant mismatch");
    }
}

#[test]
fn deleting_a_lineage_parent_does_not_orphan_a_child_visually() {
    // The semantic guarantee we want: even if a parent is deleted, the child
    // still has the parent's id recorded for forensic / re-resurrect flows.
    // The Doc layer does not auto-cascade-delete based on lineage edges; those
    // are weak references.
    let mut doc = Doc::new();

    let parent = ai_node("v1", None, 1);
    let parent_id = parent.id;
    doc.apply(Operation::create_node(parent)).unwrap();
    let child = ai_node("v2", Some(parent_id), 2);
    let child_id = child.id;
    doc.apply(Operation::create_node(child)).unwrap();

    // Capture the descendants snapshot the way a real delete op would.
    let snap: Vec<_> = doc
        .scene
        .descendants_of(parent_id)
        .map(|id| doc.scene.get(id).unwrap().clone())
        .collect();
    doc.apply(Operation::DeleteSubtree { snapshot: snap })
        .unwrap();
    assert!(!doc.scene.contains(parent_id));
    assert!(doc.scene.contains(child_id));
    if let NodeData::AiArtifact(a) = &doc.scene.get(child_id).unwrap().data {
        // Child still records its lineage parent — useful for "the version
        // this came from has been deleted; click to restore" UX.
        assert_eq!(a.lineage_parent, Some(parent_id));
    }
}
