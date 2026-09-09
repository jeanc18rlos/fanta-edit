//! Run with `cargo run --locked --release -p fanta-doc --example scene_snapshot`.
//! Measure process peak memory separately with `/usr/bin/time -l` on macOS.

use fanta_doc::color::Color;
use fanta_doc::index::IndexKey;
use fanta_doc::node::{CanvasNode, NodeData, VectorNode};
use fanta_doc::scene::Scene;
use fanta_doc::transform::Transform2D;
use std::hint::black_box;
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    const NODE_COUNT: usize = 10_000;
    const SEGMENTS_PER_NODE: usize = 256;
    const SNAPSHOT_COUNT: usize = 3;
    const EDIT_COUNT: usize = 12;

    let nodes = (0..NODE_COUNT).map(|index| {
        let mut vector = VectorNode::rect_solid(0.0, 0.0, 100.0, 100.0, Color::WHITE);
        vector.path.segments.clear();
        vector.path.move_to(0.0, 0.0);
        for segment in 0..SEGMENTS_PER_NODE {
            let coordinate = segment as f64;
            vector
                .path
                .cubic_to(coordinate, 0.0, coordinate, 100.0, coordinate + 1.0, 50.0);
        }
        let mut node = CanvasNode::new(NodeData::Vector(vector));
        node.index = IndexKey::from_raw(index as f64);
        node
    });
    let mut scene = Scene::new();
    let ids = scene.insert_many(nodes)?;
    let mut snapshots: Vec<Scene> = (0..SNAPSHOT_COUNT).map(|_| scene.clone()).collect();
    let mut snapshot_times = Vec::with_capacity(EDIT_COUNT);
    let mut edit_times = Vec::with_capacity(EDIT_COUNT);

    for iteration in 0..EDIT_COUNT {
        let started = Instant::now();
        if let Some(&id) = ids.get(iteration) {
            scene.set_transform(id, Transform2D::translation(iteration as f64, 1.0))?;
        }
        let mut rectangle = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            10.0,
            10.0,
            Color::WHITE,
        )));
        rectangle.index = scene.next_root_index();
        scene.insert(rectangle)?;
        edit_times.push(started.elapsed());

        let started = Instant::now();
        let snapshot = scene.clone();
        snapshot_times.push(started.elapsed());
        if let Some(previous) = snapshots.get_mut(iteration % SNAPSHOT_COUNT) {
            *previous = snapshot;
        }
        black_box(&snapshots);
    }

    snapshot_times.sort();
    edit_times.sort();
    let milliseconds = |times: &[Duration], position: usize| {
        times
            .get(position)
            .map_or(0.0, |time| time.as_secs_f64() * 1_000.0)
    };
    println!(
        "nodes={NODE_COUNT} cubic_segments_per_node={SEGMENTS_PER_NODE} retained_snapshots={SNAPSHOT_COUNT} edits={EDIT_COUNT}"
    );
    println!(
        "snapshot_p50_ms={:.3} snapshot_max_ms={:.3} edit_p50_ms={:.3} edit_max_ms={:.3}",
        milliseconds(&snapshot_times, EDIT_COUNT / 2),
        milliseconds(&snapshot_times, EDIT_COUNT - 1),
        milliseconds(&edit_times, EDIT_COUNT / 2),
        milliseconds(&edit_times, EDIT_COUNT - 1),
    );
    black_box((scene, snapshots));
    Ok(())
}
