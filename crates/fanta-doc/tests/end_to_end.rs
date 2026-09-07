//! End-to-end: build a non-trivial scene through the public API, exercise
//! undo/redo, round-trip through JSON, and verify state survives load.
//!
//! These are the "would a real session work?" smoke tests. If they pass, the
//! doc model is suitable for a tool layer to start consuming.

use fanta_doc::{
    AiArtifactNode, AssetId, BitmapNode, CanvasNode, Color, Doc, GenerationStatus, GroupNode,
    ImageFitMode, IndexKey, NodeData, Operation, Transform2D, VectorNode,
};

fn rect(w: f64, h: f64, color: Color) -> CanvasNode {
    CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0, 0.0, w, h, color,
    )))
}

#[test]
fn full_session_round_trips_through_json() {
    let mut doc = Doc::new();

    // Build: one group with a header rect, a body image, and an AI artifact.
    let mut group = CanvasNode::new(NodeData::Group(GroupNode::default()));
    let group_id = group.id;
    group.name = "Hero".into();
    doc.apply(Operation::create_node(group)).unwrap();

    let mut header = rect(800.0, 100.0, Color::rgb(255, 128, 0));
    header.parent = Some(group_id);
    header.index = IndexKey::from_raw(1.0);
    let header_id = header.id;
    doc.apply(Operation::create_node(header)).unwrap();

    let mut hero_img = CanvasNode::new(NodeData::Bitmap(BitmapNode {
        asset: AssetId::new(),
        natural_size: [1920, 1080],
        local_size: [800.0, 450.0],
        crop: None,
        fit: ImageFitMode::Fill,
        tint: None,
    }));
    hero_img.parent = Some(group_id);
    hero_img.index = IndexKey::from_raw(2.0);
    hero_img.transform = Transform2D::translation(0.0, 110.0);
    hero_img.name = "Hero Image".into();
    let img_id = hero_img.id;
    doc.apply(Operation::create_node(hero_img)).unwrap();

    let mut ai = CanvasNode::new(NodeData::AiArtifact(AiArtifactNode {
        local_size: [512.0, 512.0],
        prompt: "a serene mountain landscape, golden hour".into(),
        model: "flux-pro".into(),
        params: serde_json::json!({"steps": 30, "cfg": 7.5}),
        inputs: vec![],
        lineage_parent: None,
        output: Some(AssetId::new()),
        status: GenerationStatus::Done,
        seed: Some(20_260_528),
    }));
    ai.parent = Some(group_id);
    ai.index = IndexKey::from_raw(3.0);
    ai.transform = Transform2D::translation(900.0, 110.0);
    let ai_id = ai.id;
    doc.apply(Operation::create_node(ai)).unwrap();

    // Move the AI artifact. `Doc::apply` builds the `OpCtx` and records a
    // single-op transaction — the same path a real drag commits through.
    doc.apply(Operation::SetTransform {
        id: ai_id,
        old: Transform2D::translation(900.0, 110.0),
        new: Transform2D::translation(950.0, 130.0),
    })
    .unwrap();

    assert_eq!(doc.scene.len(), 4);
    assert_eq!(doc.scene.children_of(Some(group_id)).len(), 3);

    // Round-trip through JSON.
    let json = doc.to_json_pretty().unwrap();
    let loaded = Doc::from_json_str(&json).expect("valid load");
    assert_eq!(loaded.id, doc.id);
    assert_eq!(loaded.scene.len(), 4);
    assert!(loaded.scene.contains(group_id));
    assert!(loaded.scene.contains(header_id));
    assert!(loaded.scene.contains(img_id));
    assert!(loaded.scene.contains(ai_id));
    let kids = loaded.scene.children_of(Some(group_id));
    assert_eq!(kids, &[header_id, img_id, ai_id]);
}

#[test]
fn undo_then_redo_drives_state_back_and_forth() {
    let mut doc = Doc::new();
    let r1 = rect(10.0, 10.0, Color::BLACK);
    let r1_id = r1.id;
    doc.apply(Operation::create_node(r1)).unwrap();
    let r2 = rect(20.0, 20.0, Color::WHITE);
    let r2_id = r2.id;
    doc.apply(Operation::create_node(r2)).unwrap();
    assert_eq!(doc.scene.len(), 2);

    doc.undo().unwrap();
    assert_eq!(doc.scene.len(), 1);
    assert!(doc.scene.contains(r1_id));
    assert!(!doc.scene.contains(r2_id));

    doc.undo().unwrap();
    assert_eq!(doc.scene.len(), 0);

    doc.redo().unwrap();
    doc.redo().unwrap();
    assert_eq!(doc.scene.len(), 2);
    assert!(doc.scene.contains(r1_id));
    assert!(doc.scene.contains(r2_id));
}

#[test]
fn reparent_round_trips_state() {
    let mut doc = Doc::new();
    let group_a = CanvasNode::new(NodeData::Group(GroupNode::default()));
    let a_id = group_a.id;
    doc.apply(Operation::create_node(group_a)).unwrap();
    let group_b = CanvasNode::new(NodeData::Group(GroupNode::default()));
    let b_id = group_b.id;
    doc.apply(Operation::create_node(group_b)).unwrap();

    let mut item = rect(10.0, 10.0, Color::WHITE);
    item.parent = Some(a_id);
    let item_id = item.id;
    doc.apply(Operation::create_node(item)).unwrap();

    // Move item from a to b.
    doc.apply(Operation::Reparent {
        id: item_id,
        old_parent: Some(a_id),
        old_index: IndexKey::FIRST,
        new_parent: Some(b_id),
        new_index: IndexKey::FIRST,
    })
    .unwrap();
    assert_eq!(doc.scene.children_of(Some(a_id)), &[]);
    assert_eq!(doc.scene.children_of(Some(b_id)), &[item_id]);

    // Undo brings it back to a.
    doc.undo().unwrap();
    assert_eq!(doc.scene.children_of(Some(a_id)), &[item_id]);
    assert_eq!(doc.scene.children_of(Some(b_id)), &[]);
}
