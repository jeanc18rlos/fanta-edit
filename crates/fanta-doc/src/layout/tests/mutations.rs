use super::*;
use crate::{GridLayout, GridTrack, IndexKey, Scene};

fn assert_unchanged(scene: &Scene, before: &Scene, revision: u64) {
    assert_eq!(
        serde_json::to_value(scene).expect("scene"),
        serde_json::to_value(before).expect("original scene"),
    );
    assert_eq!(scene.revision(), revision);
    assert!(
        scene
            .changes_since(revision)
            .expect("bounded delta")
            .is_empty()
    );
    for root in before.roots() {
        for id in before.descendants_of(*root) {
            assert!(scene.shares_node(before, id), "unchanged node {id}");
        }
    }
}

#[test]
fn unchanged_large_page_layout_preserves_precise_scene_delta() {
    let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
    let page_id = page.id;
    let mut nodes = vec![page];
    for index in 0..2050 {
        let mut node = rect_child(page_id, 10.0, 12.0);
        node.index = IndexKey::from_raw(index as f64 + 1.0);
        nodes.push(node);
    }
    let moved = nodes.last().expect("last child").id;
    let mut scene = Scene::new();
    scene.insert_many(nodes).expect("valid page");
    let original = scene.clone();
    let revision = scene.revision();
    for _ in 0..9 {
        solve_auto_layout(&mut scene, page_id, &mut no_measure);
    }
    assert_unchanged(&scene, &original, revision);

    let transform = Transform2D::translation(32.0, 16.0);
    scene.set_transform(moved, transform).expect("move child");
    let after_move = scene.clone();
    let moved_revision = scene.revision();
    for _ in 0..9 {
        solve_auto_layout(&mut scene, page_id, &mut no_measure);
    }
    assert_unchanged(&scene, &after_move, moved_revision);
    let delta = scene.changes_since(revision).expect("single changed node");
    assert_eq!(delta.transforms, vec![moved]);
    assert!(delta.nodes.is_empty());
    assert_eq!(scene.get(moved).expect("moved child").transform, transform);
}

#[test]
fn text_autoresize_records_only_changed_geometry() {
    let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
    let page_id = page.id;
    let mut fixed = CanvasNode::new(NodeData::Text(TextNode::new("Fixed", 70.0, 80.0)));
    fixed.parent = Some(page_id);
    let fixed_id = fixed.id;
    let mut label = TextNode::new("Ready", 50.0, 20.0);
    label.auto_resize = TextAutoResize::WidthAndHeight;
    let mut label = CanvasNode::new(NodeData::Text(label));
    label.parent = Some(page_id);
    let label_id = label.id;
    let mut paragraph = TextNode::new("Changed", 90.0, 1.0);
    paragraph.auto_resize = TextAutoResize::Height;
    let mut paragraph = CanvasNode::new(NodeData::Text(paragraph));
    paragraph.parent = Some(page_id);
    let paragraph_id = paragraph.id;
    let mut scene = Scene::new();
    scene
        .insert_many([page, fixed, label, paragraph])
        .expect("text page");
    let before = scene.clone();
    let revision = scene.revision();
    let mut measurements = 0;
    let mut measure = |text: &TextNode| {
        assert_ne!(text.auto_resize, TextAutoResize::None);
        measurements += 1;
        (text.content.len() as f64 * 10.0, 20.0)
    };
    solve_auto_layout(&mut scene, page_id, &mut measure);
    let delta = scene.changes_since(revision).expect("text delta");
    assert_eq!(delta.nodes, vec![paragraph_id]);
    assert!(delta.transforms.is_empty());
    for id in [page_id, fixed_id, label_id] {
        assert!(scene.shares_node(&before, id), "unchanged node {id}");
    }
    assert_eq!(
        scene
            .get(paragraph_id)
            .expect("paragraph")
            .data
            .local_size(),
        Some([90.0, 20.0])
    );
    let settled = scene.clone();
    let revision = scene.revision();
    solve_auto_layout(&mut scene, page_id, &mut measure);
    assert_unchanged(&scene, &settled, revision);
    assert_eq!(measurements, 4);
}

#[test]
fn scoped_layout_reaches_changed_text_through_free_groups_without_measuring_other_text() {
    let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
    let page_id = page.id;
    let mut wrapper = CanvasNode::new(NodeData::Group(GroupNode::default()));
    wrapper.parent = Some(page_id);
    let wrapper_id = wrapper.id;
    let mut changed = TextNode::new("Changed", 71.0, 13.0);
    changed.auto_resize = TextAutoResize::WidthAndHeight;
    let mut changed = CanvasNode::new(NodeData::Text(changed));
    changed.parent = Some(wrapper_id);
    let changed_id = changed.id;
    let mut untouched = TextNode::new("Imported", 95.0, 17.0);
    untouched.auto_resize = TextAutoResize::WidthAndHeight;
    let mut untouched = CanvasNode::new(NodeData::Text(untouched));
    untouched.parent = Some(wrapper_id);
    untouched.index = IndexKey::after(changed.index);
    let untouched_id = untouched.id;
    let mut scene = Scene::new();
    scene
        .insert_many([page, wrapper, changed, untouched])
        .expect("text scene");
    let before = scene.clone();
    let revision = scene.revision();
    let mut measured = Vec::new();
    solve_auto_layout_scoped(
        &mut scene,
        page_id,
        &BTreeSet::from([page_id, changed_id]),
        &mut |text| {
            measured.push(text.content.clone());
            (70.0, 20.0)
        },
    );
    assert_eq!(measured, vec!["Changed"]);
    assert_eq!(
        scene
            .get(changed_id)
            .expect("changed text")
            .data
            .local_size(),
        Some([70.0, 20.0])
    );
    assert_eq!(
        scene.changes_since(revision).expect("layout delta").nodes,
        vec![changed_id]
    );
    for id in [page_id, wrapper_id, untouched_id] {
        assert!(scene.shares_node(&before, id), "unaffected geometry {id}");
    }
}

fn assert_settled_flow_unchanged(mode: LayoutMode) {
    let mut parent = frame(
        200.0,
        160.0,
        AutoLayout {
            mode,
            spacing: 8.0,
            padding: [4.0; 4],
            counter_align: CounterAlign::Stretch,
            ..Default::default()
        },
    );
    if mode == LayoutMode::Grid {
        let NodeData::Group(group) = &mut parent.data else {
            panic!("frame")
        };
        group.grid = Some(GridLayout {
            columns: vec![
                GridTrack::Fixed { size: 80.0 },
                GridTrack::Fixed { size: 100.0 },
            ],
            rows: vec![GridTrack::Fixed { size: 50.0 }],
            column_gap: 8.0,
            row_gap: 0.0,
        });
    }
    let parent_id = parent.id;
    let first = rect_child(parent_id, 20.0, 30.0);
    let first_id = first.id;
    let mut second = rect_child(parent_id, 40.0, 25.0);
    second.index = IndexKey::after(first.index);
    let second_id = second.id;
    let mut scene = Scene::new();
    scene
        .insert_many([parent, first, second])
        .expect("flow scene");
    solve_auto_layout(&mut scene, parent_id, &mut no_measure);
    for id in [first_id, second_id] {
        assert_ne!(
            scene.get(id).expect("child").transform,
            Transform2D::translation(999.0, 999.0)
        );
    }
    let settled = scene.clone();
    let revision = scene.revision();
    for _ in 0..3 {
        solve_auto_layout(&mut scene, parent_id, &mut no_measure);
    }
    assert_unchanged(&scene, &settled, revision);
}

#[test]
fn settled_horizontal_layout_does_not_rewrite_nodes() {
    assert_settled_flow_unchanged(LayoutMode::Horizontal);
}

#[test]
fn settled_vertical_layout_does_not_rewrite_nodes() {
    assert_settled_flow_unchanged(LayoutMode::Vertical);
}

#[test]
fn settled_grid_layout_does_not_rewrite_nodes() {
    assert_settled_flow_unchanged(LayoutMode::Grid);
}

#[test]
fn stretching_unsized_child_does_not_record_a_size_change() {
    let parent = frame(
        100.0,
        100.0,
        AutoLayout {
            counter_align: CounterAlign::Stretch,
            ..Default::default()
        },
    );
    let parent_id = parent.id;
    let mut child = CanvasNode::new(NodeData::Group(GroupNode::default()));
    child.parent = Some(parent_id);
    let mut scene = Scene::new();
    scene.insert_many([parent, child]).expect("unsized child");
    let before = scene.clone();
    let revision = scene.revision();
    solve_auto_layout(&mut scene, parent_id, &mut no_measure);
    assert_unchanged(&scene, &before, revision);
}

#[test]
fn stretching_vector_preserves_existing_scale_tolerance_and_real_resizes() {
    let parent = frame(
        100.0,
        100.0 + 5e-8,
        AutoLayout {
            counter_align: CounterAlign::Stretch,
            ..Default::default()
        },
    );
    let parent_id = parent.id;
    let mut child = rect_child(parent_id, 100.0, 100.0);
    child.transform = Transform2D::IDENTITY;
    let NodeData::Vector(vector) = &mut child.data else {
        panic!("vector")
    };
    vector.path = crate::PathData::ellipse(50.0, 50.0, 50.0, 50.0);
    let child_id = child.id;
    let mut scene = Scene::new();
    scene.insert_many([parent, child]).expect("vector child");
    let before = scene.clone();
    let revision = scene.revision();
    solve_auto_layout(&mut scene, parent_id, &mut no_measure);
    assert_unchanged(&scene, &before, revision);

    let NodeData::Group(parent) = &mut scene.get_mut(parent_id).expect("parent").data else {
        panic!("frame")
    };
    parent.clip_size = Some([100.0, 150.0]);
    let revision = scene.revision();
    solve_auto_layout(&mut scene, parent_id, &mut no_measure);
    assert_eq!(
        local_box(scene.get(child_id).expect("child")).size,
        [100.0, 150.0]
    );
    assert_eq!(
        scene.changes_since(revision).expect("resized vector").nodes,
        vec![child_id]
    );
    let NodeData::Vector(vector) = &scene.get(child_id).expect("child").data else {
        panic!("vector")
    };
    assert!(!vector.path.is_rect());
    let settled = scene.clone();
    let revision = scene.revision();
    solve_auto_layout(&mut scene, parent_id, &mut no_measure);
    assert_unchanged(&scene, &settled, revision);
}
