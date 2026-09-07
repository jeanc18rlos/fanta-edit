//! Unit tests for [`Scene`] — structural ops, world geometry, the spatial
//! index parity oracle, and cache-invalidation contracts.
//!
//! A child module of [`crate::scene::graph`] (`use super::*`), so it can reach
//! the `pub(crate)` storage fields the geometry/parity tests probe directly.

use super::*;
use crate::color::Color;
use crate::node::{GroupNode, NodeData, VectorNode};
use glam::DVec2;

fn rect_node(x: f64, y: f64, w: f64, h: f64) -> CanvasNode {
    CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        x,
        y,
        w,
        h,
        Color::WHITE,
    )))
}

fn group_node() -> CanvasNode {
    CanvasNode::new(NodeData::Group(GroupNode::default()))
}

#[test]
fn insert_and_get() {
    let mut scene = Scene::new();
    let n = rect_node(0.0, 0.0, 10.0, 10.0);
    let id = n.id;
    scene.insert(n).unwrap();
    assert!(scene.contains(id));
    assert_eq!(scene.len(), 1);
    assert!(scene.get(id).is_some());
}

#[test]
fn duplicate_id_is_rejected() {
    let mut scene = Scene::new();
    let n = rect_node(0.0, 0.0, 10.0, 10.0);
    let clone = n.clone();
    scene.insert(n).unwrap();
    assert!(matches!(scene.insert(clone), Err(SceneError::Duplicate(_))));
}

#[test]
fn child_under_non_container_parent_is_rejected() {
    let mut scene = Scene::new();
    let rect = rect_node(0.0, 0.0, 10.0, 10.0);
    let rect_id = rect.id;
    scene.insert(rect).unwrap();
    let mut child = rect_node(0.0, 0.0, 5.0, 5.0);
    child.parent = Some(rect_id);
    assert!(matches!(
        scene.insert(child),
        Err(SceneError::ParentNotContainer(_))
    ));
}

#[test]
fn next_root_index_climbs_above_existing_siblings() {
    let mut scene = Scene::new();
    // Empty scene → FIRST.
    assert_eq!(scene.next_root_index(), IndexKey::FIRST);
    // Insert one node at FIRST; the next index must sort strictly above it.
    let mut a = rect_node(0.0, 0.0, 10.0, 10.0);
    a.index = scene.next_root_index();
    scene.insert(a).unwrap();
    let second = scene.next_root_index();
    assert!(second > IndexKey::FIRST);
    // A node at `second` lands last (top) in z-order.
    let mut b = rect_node(0.0, 0.0, 10.0, 10.0);
    b.index = second;
    let b_id = b.id;
    scene.insert(b).unwrap();
    assert_eq!(*scene.roots().last().unwrap(), b_id);
}

#[test]
fn z_order_follows_index_ascending() {
    let mut scene = Scene::new();
    let mut a = rect_node(0.0, 0.0, 10.0, 10.0);
    let mut b = rect_node(0.0, 0.0, 10.0, 10.0);
    let mut c = rect_node(0.0, 0.0, 10.0, 10.0);
    a.index = IndexKey::from_raw(2.0);
    b.index = IndexKey::from_raw(1.0);
    c.index = IndexKey::from_raw(3.0);
    let (a_id, b_id, c_id) = (a.id, b.id, c.id);
    scene.insert(a).unwrap();
    scene.insert(b).unwrap();
    scene.insert(c).unwrap();
    assert_eq!(scene.roots(), &[b_id, a_id, c_id]);
}

#[test]
fn reparent_to_descendant_creates_cycle_error() {
    let mut scene = Scene::new();
    let g_outer = group_node();
    let g_inner = {
        let mut g = group_node();
        g.parent = Some(g_outer.id);
        g
    };
    let outer_id = g_outer.id;
    let inner_id = g_inner.id;
    scene.insert(g_outer).unwrap();
    scene.insert(g_inner).unwrap();
    // Try to make outer a child of inner — cycle.
    assert!(matches!(
        scene.set_parent(outer_id, Some(inner_id), IndexKey::FIRST),
        Err(SceneError::Cycle { .. })
    ));
}

#[test]
fn remove_takes_descendants_with_it() {
    let mut scene = Scene::new();
    let g = group_node();
    let g_id = g.id;
    scene.insert(g).unwrap();
    for _ in 0..3 {
        let mut r = rect_node(0.0, 0.0, 10.0, 10.0);
        r.parent = Some(g_id);
        scene.insert(r).unwrap();
    }
    assert_eq!(scene.len(), 4);
    scene.remove(g_id).unwrap();
    assert_eq!(scene.len(), 0);
}

#[test]
fn descendants_iterates_in_dfs() {
    let mut scene = Scene::new();
    let g = group_node();
    let g_id = g.id;
    scene.insert(g).unwrap();
    let child_ids: Vec<NodeId> = (0..3)
        .map(|i| {
            let mut r = rect_node(0.0, 0.0, 10.0, 10.0);
            r.parent = Some(g_id);
            r.index = IndexKey::from_raw(i as f64);
            let id = r.id;
            scene.insert(r).unwrap();
            id
        })
        .collect();
    let visited: Vec<NodeId> = scene.descendants_of(g_id).collect();
    assert_eq!(visited[0], g_id);
    assert_eq!(&visited[1..], &child_ids[..]);
}

#[test]
fn hit_test_returns_topmost_visible_node() {
    let mut scene = Scene::new();
    let mut bottom = rect_node(0.0, 0.0, 100.0, 100.0);
    bottom.index = IndexKey::from_raw(1.0);
    let mut top = rect_node(0.0, 0.0, 50.0, 50.0);
    top.index = IndexKey::from_raw(2.0);
    let (bot_id, top_id) = (bottom.id, top.id);
    scene.insert(bottom).unwrap();
    scene.insert(top).unwrap();
    // Point (25, 25) is under both; top wins.
    assert_eq!(scene.hit_test(DVec2::new(25.0, 25.0)), Some(top_id));
    // Point (75, 75) is only under the bottom rect.
    assert_eq!(scene.hit_test(DVec2::new(75.0, 75.0)), Some(bot_id));
    // Point well outside hits nothing.
    assert_eq!(scene.hit_test(DVec2::new(500.0, 500.0)), None);
}

// ---- spatial-index parity ------------------------------------------------

/// Tiny deterministic xorshift PRNG — keeps the parity tests reproducible
/// without pulling a `rand` dependency into the doc crate.
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    /// Uniform f64 in `[lo, hi)`.
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        let u = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        lo + u * (hi - lo)
    }
    fn chance(&mut self, p: f64) -> bool {
        self.range(0.0, 1.0) < p
    }
}

/// Build a randomized scene: a mix of root rects and small groups holding
/// rects, with varied positions, sizes, z-indices, transforms, and some
/// hidden nodes. Returns the scene.
fn random_scene(seed: u64, n: usize) -> Scene {
    let mut rng = Rng::new(seed);
    let mut scene = Scene::new();
    let mut groups: Vec<NodeId> = Vec::new();
    for i in 0..n {
        // Occasionally make a group so we exercise nesting + group exclusion.
        if rng.chance(0.15) {
            let mut g = group_node();
            g.index = IndexKey::from_raw(i as f64);
            if rng.chance(0.3) {
                g.flags |= crate::node::NodeFlags::HIDDEN;
            }
            let id = g.id;
            scene.insert(g).unwrap();
            groups.push(id);
            continue;
        }
        let x = rng.range(-200.0, 200.0);
        let y = rng.range(-200.0, 200.0);
        let w = rng.range(1.0, 60.0);
        let h = rng.range(1.0, 60.0);
        let mut r = rect_node(0.0, 0.0, w, h);
        r.transform = Transform2D::translation(x, y);
        r.index = IndexKey::from_raw(i as f64);
        // Maybe parent under an existing group.
        if !groups.is_empty() && rng.chance(0.4) {
            let gi = (rng.next_u64() as usize) % groups.len();
            r.parent = Some(groups[gi]);
        }
        if rng.chance(0.1) {
            r.flags |= crate::node::NodeFlags::HIDDEN;
        }
        scene.insert(r).unwrap();
    }
    scene
}

#[test]
fn indexed_hit_test_matches_brute_force_over_random_scenes() {
    for seed in 1..=40u64 {
        let scene = random_scene(seed, 300);
        let mut rng = Rng::new(seed.wrapping_mul(2_654_435_761));
        for _ in 0..200 {
            let p = DVec2::new(rng.range(-220.0, 220.0), rng.range(-220.0, 220.0));
            let indexed = scene.hit_test(p);
            let brute = scene.hit_test_brute(p);
            assert_eq!(
                indexed, brute,
                "hit_test parity failed at seed {seed}, point {p:?}"
            );
        }
    }
}

#[test]
fn indexed_hit_test_matches_brute_on_exact_node_corners() {
    // Probe at the exact min/max corners and centers of every node's world
    // bounds — the boundary cases most likely to expose an off-by-one in
    // the grid bucketing or the inclusive contains test.
    for seed in 1..=20u64 {
        let scene = random_scene(seed, 200);
        let ids: Vec<NodeId> = scene.nodes.keys().copied().collect();
        for &id in &ids {
            if let Some(b) = scene.world_bounds(id) {
                let probes = [
                    DVec2::new(b.min_x, b.min_y),
                    DVec2::new(b.max_x, b.max_y),
                    b.center(),
                    DVec2::new(b.min_x, b.max_y),
                    DVec2::new(b.max_x, b.min_y),
                ];
                for p in probes {
                    assert_eq!(
                        scene.hit_test(p),
                        scene.hit_test_brute(p),
                        "corner-probe parity failed at seed {seed}, id {id}, point {p:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn index_rebuilds_after_transform_edit() {
    // Moving a node via get_mut must invalidate the index so a subsequent
    // hit-test reflects the new position, not the stale one.
    let mut scene = Scene::new();
    let mut r = rect_node(0.0, 0.0, 10.0, 10.0);
    r.transform = Transform2D::translation(0.0, 0.0);
    let id = r.id;
    scene.insert(r).unwrap();
    // Warm the index at the original location.
    assert_eq!(scene.hit_test(DVec2::new(5.0, 5.0)), Some(id));
    assert_eq!(scene.hit_test(DVec2::new(105.0, 5.0)), None);
    // Drag it +100x.
    scene.get_mut(id).unwrap().transform = Transform2D::translation(100.0, 0.0);
    // Old spot is now empty; new spot hits — index must have rebuilt.
    assert_eq!(scene.hit_test(DVec2::new(5.0, 5.0)), None);
    assert_eq!(scene.hit_test(DVec2::new(105.0, 5.0)), Some(id));
}

#[test]
fn index_rebuilds_after_insert_and_remove() {
    let mut scene = Scene::new();
    let a = rect_node(0.0, 0.0, 10.0, 10.0);
    let a_id = a.id;
    scene.insert(a).unwrap();
    assert_eq!(scene.hit_test(DVec2::new(5.0, 5.0)), Some(a_id));
    // Insert a higher-z node on top — index rebuild must surface it.
    let mut b = rect_node(0.0, 0.0, 10.0, 10.0);
    b.index = IndexKey::after(scene.get(a_id).unwrap().index);
    let b_id = b.id;
    scene.insert(b).unwrap();
    assert_eq!(scene.hit_test(DVec2::new(5.0, 5.0)), Some(b_id));
    // Remove the top node — falls back to the one beneath.
    scene.remove(b_id).unwrap();
    assert_eq!(scene.hit_test(DVec2::new(5.0, 5.0)), Some(a_id));
}

/// A large synthetic scene (~44k leaf nodes spread over a wide field, like
/// the Spectrum file) exercising the index at scale: build it, run many
/// point hit-tests and marquee queries both brute-force and indexed,
/// confirm the answers match exactly, and print the timings. Run with
/// `cargo test -p fanta-doc spatial_index_benchmark -- --nocapture` to see
/// the before/after numbers.
#[test]
#[ignore = "benchmark-style scale test; run explicitly when profiling spatial index performance"]
fn spatial_index_benchmark() {
    use std::time::Instant;

    const N: usize = 44_000;
    // Lay nodes out on a grid so they don't all overlap (mirrors a real
    // page where most points hit at most a few nodes).
    let cols = (N as f64).sqrt().ceil() as usize;
    let mut scene = Scene::new();
    let mut rng = Rng::new(0xDEAD_BEEF);
    for i in 0..N {
        let gx = (i % cols) as f64 * 30.0;
        let gy = (i / cols) as f64 * 30.0;
        let w = rng.range(8.0, 24.0);
        let h = rng.range(8.0, 24.0);
        let mut r = rect_node(0.0, 0.0, w, h);
        r.transform = Transform2D::translation(gx, gy);
        r.index = IndexKey::from_raw(i as f64);
        scene.insert(r).unwrap();
    }
    let world_w = cols as f64 * 30.0;

    // Random query points across the laid-out field.
    let mut qrng = Rng::new(0x1234_5678);
    let points: Vec<DVec2> = (0..2_000)
        .map(|_| {
            DVec2::new(
                qrng.range(-50.0, world_w + 50.0),
                qrng.range(-50.0, world_w + 50.0),
            )
        })
        .collect();

    // --- Point hit-test: brute force (cold reference). ---
    let t0 = Instant::now();
    let brute: Vec<Option<NodeId>> = points.iter().map(|&p| scene.hit_test_brute(p)).collect();
    let brute_dur = t0.elapsed();

    // --- Point hit-test: indexed. First call builds the index. ---
    let t1 = Instant::now();
    let indexed: Vec<Option<NodeId>> = points.iter().map(|&p| scene.hit_test(p)).collect();
    let indexed_dur = t1.elapsed();

    assert_eq!(
        brute, indexed,
        "indexed point hit-test must match brute force"
    );

    // --- Marquee: a handful of rectangles of varying size. ---
    let rects: Vec<Bounds> = (0..16)
        .map(|_| {
            let x = qrng.range(0.0, world_w);
            let y = qrng.range(0.0, world_w);
            let w = qrng.range(50.0, 1_000.0);
            let h = qrng.range(50.0, 1_000.0);
            Bounds::from_xywh(x, y, w, h)
        })
        .collect();

    // Brute marquee (intersect mode), matching `rect_query_where`'s filter.
    let brute_marquee = |rect: Bounds| -> Vec<NodeId> {
        let mut hits: Vec<(IndexKey, NodeId)> = Vec::new();
        // Stable order: gather then sort by node index (== paint rank here,
        // since this scene is flat with ascending indices).
        for n in scene.nodes.values() {
            if matches!(n.data, NodeData::Group(_)) {
                continue;
            }
            if let Some(bb) = scene.world_bounds(n.id) {
                if bb.intersects(&rect) {
                    hits.push((n.index, n.id));
                }
            }
        }
        hits.sort_by_key(|&(idx, _)| idx);
        hits.into_iter().map(|(_, id)| id).collect()
    };

    let t2 = Instant::now();
    let mut brute_marquee_counts = 0usize;
    let brute_marquee_results: Vec<Vec<NodeId>> = rects
        .iter()
        .map(|&r| {
            let v = brute_marquee(r);
            brute_marquee_counts += v.len();
            v
        })
        .collect();
    let brute_marquee_dur = t2.elapsed();

    let t3 = Instant::now();
    let indexed_marquee_results: Vec<Vec<NodeId>> = rects
        .iter()
        .map(|&r| scene.rect_query_where(r, |_, bb| bb.intersects(&r)))
        .collect();
    let indexed_marquee_dur = t3.elapsed();

    assert_eq!(
        brute_marquee_results, indexed_marquee_results,
        "indexed marquee must match brute force"
    );

    println!("\n=== spatial index benchmark ({N} nodes) ===");
    println!("point hit-tests ({} queries):", points.len());
    println!("  brute force : {brute_dur:?}");
    println!("  indexed     : {indexed_dur:?} (includes first-query index build)");
    if indexed_dur.as_secs_f64() > 0.0 {
        println!(
            "  speedup     : {:.1}x",
            brute_dur.as_secs_f64() / indexed_dur.as_secs_f64()
        );
    }
    println!(
        "marquee queries ({} rects, {} total hits):",
        rects.len(),
        brute_marquee_counts
    );
    println!("  brute force : {brute_marquee_dur:?}");
    println!("  indexed     : {indexed_marquee_dur:?}");
    if indexed_marquee_dur.as_secs_f64() > 0.0 {
        println!(
            "  speedup     : {:.1}x",
            brute_marquee_dur.as_secs_f64() / indexed_marquee_dur.as_secs_f64()
        );
    }
    println!("=========================================\n");
}

#[test]
fn hidden_subtree_is_never_hit_via_index() {
    let mut scene = Scene::new();
    let mut g = group_node();
    g.flags |= crate::node::NodeFlags::HIDDEN;
    let g_id = g.id;
    scene.insert(g).unwrap();
    let mut child = rect_node(0.0, 0.0, 10.0, 10.0);
    child.parent = Some(g_id);
    scene.insert(child).unwrap();
    // Both indexed and brute agree: nothing hits under a hidden group.
    assert_eq!(scene.hit_test(DVec2::new(5.0, 5.0)), None);
    assert_eq!(scene.hit_test_brute(DVec2::new(5.0, 5.0)), None);
}

#[test]
fn world_transform_composes_through_ancestors() {
    let mut scene = Scene::new();
    let mut g = group_node();
    g.transform = Transform2D::translation(100.0, 0.0);
    let g_id = g.id;
    scene.insert(g).unwrap();
    let mut r = rect_node(0.0, 0.0, 10.0, 10.0);
    r.parent = Some(g_id);
    r.transform = Transform2D::translation(20.0, 30.0);
    let r_id = r.id;
    scene.insert(r).unwrap();

    let wt = scene.world_transform(r_id).unwrap();
    let p = wt.transform_point(DVec2::ZERO);
    // Group translates +100x; rect translates +20x +30y; net (120, 30).
    assert!((p - DVec2::new(120.0, 30.0)).length() < 1e-9);
}

#[test]
fn world_transform_of_root_is_its_local_transform() {
    let mut scene = Scene::new();
    let mut r = rect_node(0.0, 0.0, 10.0, 10.0);
    r.transform = Transform2D::translation(7.0, -3.0);
    let r_id = r.id;
    scene.insert(r).unwrap();
    let wt = scene.world_transform(r_id).unwrap();
    let p = wt.transform_point(DVec2::ZERO);
    assert!((p - DVec2::new(7.0, -3.0)).length() < 1e-9);
}

#[test]
fn world_transform_missing_node_is_none() {
    let scene = Scene::new();
    assert!(scene.world_transform(NodeId::new()).is_none());
}

#[test]
fn world_transform_deep_chain_composes_correctly() {
    // 50 nested groups, each translating +1x +2y. The leaf rect adds a
    // final +5x +7y. Net translation: (50 + 5, 100 + 7) = (55, 107).
    const DEPTH: usize = 50;
    let mut scene = Scene::new();
    let mut parent: Option<NodeId> = None;
    for _ in 0..DEPTH {
        let mut g = group_node();
        g.parent = parent;
        g.transform = Transform2D::translation(1.0, 2.0);
        let id = g.id;
        scene.insert(g).unwrap();
        parent = Some(id);
    }
    let mut leaf = rect_node(0.0, 0.0, 10.0, 10.0);
    leaf.parent = parent;
    leaf.transform = Transform2D::translation(5.0, 7.0);
    let leaf_id = leaf.id;
    scene.insert(leaf).unwrap();

    let wt = scene.world_transform(leaf_id).unwrap();
    let p = wt.transform_point(DVec2::ZERO);
    let expected = DVec2::new(DEPTH as f64 + 5.0, 2.0 * DEPTH as f64 + 7.0);
    assert!(
        (p - expected).length() < 1e-9,
        "deep chain composed to {p:?}, expected {expected:?}"
    );
}

#[test]
fn world_transform_repeated_calls_are_stable_with_cache() {
    // Calling twice must return the identical value: the second call hits
    // the memo cache, and a warm read must match a cold compute.
    let mut scene = Scene::new();
    let mut g = group_node();
    g.transform = Transform2D::translation(100.0, 0.0);
    let g_id = g.id;
    scene.insert(g).unwrap();
    let mut r = rect_node(0.0, 0.0, 10.0, 10.0);
    r.parent = Some(g_id);
    r.transform = Transform2D::translation(20.0, 30.0);
    let r_id = r.id;
    scene.insert(r).unwrap();

    let first = scene.world_transform(r_id).unwrap();
    let second = scene.world_transform(r_id).unwrap();
    assert_eq!(first, second);
    let p = second.transform_point(DVec2::ZERO);
    assert!((p - DVec2::new(120.0, 30.0)).length() < 1e-9);
}

#[test]
fn get_mut_transform_change_invalidates_world_cache() {
    // Proves the invalidation contract: mutating `transform` through the
    // opaque `get_mut` and re-querying must return the NEW world transform,
    // not the cached pre-edit value.
    let mut scene = Scene::new();
    let mut g = group_node();
    g.transform = Transform2D::translation(100.0, 0.0);
    let g_id = g.id;
    scene.insert(g).unwrap();
    let mut r = rect_node(0.0, 0.0, 10.0, 10.0);
    r.parent = Some(g_id);
    r.transform = Transform2D::translation(20.0, 30.0);
    let r_id = r.id;
    scene.insert(r).unwrap();

    // Warm the cache for the leaf.
    let before = scene.world_transform(r_id).unwrap();
    assert!((before.transform_point(DVec2::ZERO) - DVec2::new(120.0, 30.0)).length() < 1e-9);

    // Drag the leaf: change its local transform via get_mut.
    scene.get_mut(r_id).unwrap().transform = Transform2D::translation(0.0, 0.0);
    let after_leaf = scene.world_transform(r_id).unwrap();
    // Group still +100x; leaf now identity → (100, 0), NOT the stale (120, 30).
    assert!(
        (after_leaf.transform_point(DVec2::ZERO) - DVec2::new(100.0, 0.0)).length() < 1e-9,
        "leaf world transform was not invalidated after get_mut edit"
    );

    // Move the *ancestor*: a change to the group must propagate to the leaf.
    scene.get_mut(g_id).unwrap().transform = Transform2D::translation(0.0, 50.0);
    let after_ancestor = scene.world_transform(r_id).unwrap();
    assert!(
        (after_ancestor.transform_point(DVec2::ZERO) - DVec2::new(0.0, 50.0)).length() < 1e-9,
        "leaf world transform did not reflect an ancestor's get_mut edit"
    );
}

#[test]
fn local_bounds_warm_read_matches_cold_and_unions_children() {
    // A group's local bounds is the union of its children's transformed
    // local bounds, and a warm (memoized) read must equal the cold compute.
    let mut scene = Scene::new();
    let g = group_node();
    let g_id = g.id;
    scene.insert(g).unwrap();

    // Child A: rect path [0,0,10,10], identity transform.
    let mut a = rect_node(0.0, 0.0, 10.0, 10.0);
    a.parent = Some(g_id);
    scene.insert(a).unwrap();
    // Child B: same rect, pushed to [50,50,60,60] by its own local transform.
    let mut b = rect_node(0.0, 0.0, 10.0, 10.0);
    b.parent = Some(g_id);
    b.transform = Transform2D::translation(50.0, 50.0);
    scene.insert(b).unwrap();

    let cold = scene.local_bounds(g_id).unwrap();
    let warm = scene.local_bounds(g_id).unwrap();
    assert_eq!(
        cold, warm,
        "warm cache read must equal cold compute exactly"
    );
    // Union spans [0,0]..[60,60]; rough bounds may only ever grow the box,
    // so use inclusive comparisons.
    assert!(
        cold.min_x <= 1e-6 && cold.min_y <= 1e-6,
        "union lower corner"
    );
    assert!(
        cold.max_x >= 60.0 - 1e-6 && cold.max_y >= 60.0 - 1e-6,
        "union upper corner must reach the far child"
    );
}

#[test]
fn non_clipping_group_box_does_not_hide_overflowing_children_from_bounds() {
    let mut scene = Scene::new();
    let group = CanvasNode::new(NodeData::Group(GroupNode {
        local_size: Some([40.0, 30.0]),
        ..Default::default()
    }));
    let group_id = group.id;
    scene.insert(group).unwrap();

    let mut child = rect_node(0.0, 0.0, 20.0, 10.0);
    child.parent = Some(group_id);
    child.transform = Transform2D::translation(70.0, -15.0);
    scene.insert(child).unwrap();

    assert_eq!(
        scene.local_bounds(group_id),
        Some(Bounds::from_xywh(0.0, -15.0, 90.0, 45.0)),
        "the explicit box sizes the group but must not cull non-clipped overflow"
    );
}

#[test]
fn clipping_group_bounds_remain_the_clip_box() {
    let mut scene = Scene::new();
    let group = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([40.0, 30.0]),
        ..Default::default()
    }));
    let group_id = group.id;
    scene.insert(group).unwrap();

    let mut child = rect_node(0.0, 0.0, 20.0, 10.0);
    child.parent = Some(group_id);
    child.transform = Transform2D::translation(70.0, -15.0);
    scene.insert(child).unwrap();

    assert_eq!(
        scene.local_bounds(group_id),
        Some(Bounds::from_xywh(0.0, 0.0, 40.0, 30.0))
    );
}

#[test]
fn frame_with_clipping_disabled_unions_its_box_and_overflowing_children() {
    let mut scene = Scene::new();
    let mut group = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([40.0, 30.0]),
        ..Default::default()
    }));
    group.meta = serde_json::json!({ "clip_content": false });
    let group_id = group.id;
    scene.insert(group).unwrap();

    let mut child = rect_node(0.0, 0.0, 20.0, 10.0);
    child.parent = Some(group_id);
    child.transform = Transform2D::translation(70.0, -15.0);
    scene.insert(child).unwrap();

    assert_eq!(
        scene.local_bounds(group_id),
        Some(Bounds::from_xywh(0.0, -15.0, 90.0, 45.0))
    );
    assert_eq!(
        scene.hit_test(DVec2::new(75.0, -10.0)),
        scene.children_of(Some(group_id)).first().copied(),
        "visible overflow must remain hittable outside the authored frame box"
    );
}

#[test]
fn local_bounds_invalidated_after_child_moves() {
    // The property the render-cull perf fix relies on: moving a child via
    // the opaque `get_mut` must invalidate the parent group's cached local
    // bounds, so the next cull test sees the NEW union, not a stale one.
    let mut scene = Scene::new();
    let g = group_node();
    let g_id = g.id;
    scene.insert(g).unwrap();
    let mut r = rect_node(0.0, 0.0, 10.0, 10.0);
    r.parent = Some(g_id);
    let r_id = r.id;
    scene.insert(r).unwrap();

    // Warm the cache: group bounds == child bounds ≈ [0,0,10,10].
    let before = scene.local_bounds(g_id).unwrap();
    assert!(before.max_x <= 10.0 + 1e-6, "warm group bounds before move");

    // Drag the child far away through get_mut (the transient-drag path).
    scene.get_mut(r_id).unwrap().transform = Transform2D::translation(100.0, 100.0);
    let after = scene.local_bounds(g_id).unwrap();
    assert!(
        after.max_x >= 100.0,
        "group local bounds did not follow a child moved via get_mut — stale cache"
    );
}

#[test]
fn set_parent_invalidates_world_transform() {
    // Reparenting changes the ancestor chain, so a previously-cached world
    // transform must be recomputed against the new parent.
    let mut scene = Scene::new();
    let mut g_a = group_node();
    g_a.transform = Transform2D::translation(100.0, 0.0);
    let a_id = g_a.id;
    scene.insert(g_a).unwrap();
    let mut g_b = group_node();
    g_b.transform = Transform2D::translation(0.0, 200.0);
    let b_id = g_b.id;
    scene.insert(g_b).unwrap();

    let mut r = rect_node(0.0, 0.0, 10.0, 10.0);
    r.parent = Some(a_id);
    r.transform = Transform2D::IDENTITY;
    let r_id = r.id;
    scene.insert(r).unwrap();

    // Under A: (100, 0). Warm the cache.
    let under_a = scene.world_transform(r_id).unwrap();
    assert!((under_a.transform_point(DVec2::ZERO) - DVec2::new(100.0, 0.0)).length() < 1e-9);

    // Reparent under B: now (0, 200).
    scene.set_parent(r_id, Some(b_id), IndexKey::FIRST).unwrap();
    let under_b = scene.world_transform(r_id).unwrap();
    assert!(
        (under_b.transform_point(DVec2::ZERO) - DVec2::new(0.0, 200.0)).length() < 1e-9,
        "world transform was not invalidated after reparent"
    );
}

#[test]
fn wide_scene_world_bounds_for_all_siblings() {
    // 5,000 sibling rects, each translated to a distinct column. Every
    // world_bounds must compute and land where its transform places it.
    const N: usize = 5_000;
    let mut scene = Scene::new();
    let mut ids = Vec::with_capacity(N);
    for i in 0..N {
        let mut r = rect_node(0.0, 0.0, 10.0, 10.0);
        r.transform = Transform2D::translation(i as f64 * 20.0, 0.0);
        r.index = IndexKey::from_raw(i as f64);
        ids.push(r.id);
        scene.insert(r).unwrap();
    }
    // All bounds resolve; spot-check that each is offset to its column.
    for (i, &id) in ids.iter().enumerate() {
        let b = scene.world_bounds(id).expect("every rect has world bounds");
        assert!((b.min_x - i as f64 * 20.0).abs() < 1e-9);
        assert!((b.width() - 10.0).abs() < 1e-9);
    }
}

#[test]
fn deep_chain_world_bounds_matches_world_transform() {
    // world_bounds must agree with applying world_transform to local_bounds
    // even through a deep chain — guards the cache feeding world_bounds.
    const DEPTH: usize = 50;
    let mut scene = Scene::new();
    let mut parent: Option<NodeId> = None;
    for _ in 0..DEPTH {
        let mut g = group_node();
        g.parent = parent;
        g.transform = Transform2D::translation(3.0, 0.0);
        let id = g.id;
        scene.insert(g).unwrap();
        parent = Some(id);
    }
    let mut leaf = rect_node(0.0, 0.0, 10.0, 10.0);
    leaf.parent = parent;
    let leaf_id = leaf.id;
    scene.insert(leaf).unwrap();

    let wb = scene.world_bounds(leaf_id).unwrap();
    // Local rect [0,10]x[0,10] shifted +3x per level: min_x = 150, width 10.
    assert!((wb.min_x - 3.0 * DEPTH as f64).abs() < 1e-9);
    assert!((wb.width() - 10.0).abs() < 1e-9);
    assert!((wb.min_y - 0.0).abs() < 1e-9);
}

#[test]
fn validate_succeeds_after_normal_use() {
    let mut scene = Scene::new();
    let g = group_node();
    let g_id = g.id;
    scene.insert(g).unwrap();
    let mut r = rect_node(0.0, 0.0, 10.0, 10.0);
    r.parent = Some(g_id);
    scene.insert(r).unwrap();
    scene.validate().unwrap();
}

#[test]
fn rebuild_child_index_recovers_after_skip() {
    let mut scene = Scene::new();
    let g = group_node();
    let g_id = g.id;
    scene.insert(g).unwrap();
    let mut r = rect_node(0.0, 0.0, 10.0, 10.0);
    r.parent = Some(g_id);
    scene.insert(r).unwrap();

    let json = serde_json::to_string(&scene).unwrap();
    let mut deserialized: Scene = serde_json::from_str(&json).unwrap();
    // child_index was skipped; it's empty until we rebuild.
    assert!(deserialized.children_of(Some(g_id)).is_empty());
    deserialized.rebuild_child_index();
    assert_eq!(deserialized.children_of(Some(g_id)).len(), 1);
    deserialized.validate().unwrap();
}

#[test]
fn revision_bumps_on_mutation_and_holds_across_reads() {
    let mut scene = Scene::new();
    let r0 = scene.revision();
    let node = rect_node(0.0, 0.0, 10.0, 10.0);
    let id = node.id;
    scene.insert(node).unwrap();
    let r1 = scene.revision();
    assert_ne!(r0, r1, "insert bumps the revision");
    // Pure reads — world bounds, hit test — must NOT bump it: equal
    // revisions are the memoization key for retained render surfaces.
    let _ = scene.world_bounds(id);
    let _ = scene.hit_test(glam::DVec2::new(5.0, 5.0));
    assert_eq!(scene.revision(), r1, "reads keep the revision stable");
    // A transform write through get_mut bumps it again.
    if let Some(n) = scene.get_mut(id) {
        n.transform = Transform2D::translation(50.0, 0.0);
    }
    assert_ne!(scene.revision(), r1, "get_mut bumps the revision");
}

#[test]
fn instance_id_is_unique_across_new_default_clone_and_deserialize() {
    // Every construction path must mint a fresh process-unique instance id:
    // cross-scene memo consumers (the renderer's revision-keyed caches) rely
    // on equal `(instance_id, revision)` pairs guaranteeing identical content,
    // which only holds if no two scene instances ever share an id.
    let a = Scene::new();
    let b = Scene::default();
    assert_ne!(a.instance_id(), b.instance_id(), "new vs default");
    assert_ne!(a.instance_id(), 0, "0 is the reserved 'no scene' sentinel");

    let mut original = Scene::new();
    original.insert(rect_node(0.0, 0.0, 10.0, 10.0)).unwrap();
    let cloned = original.clone();
    assert_ne!(
        original.instance_id(),
        cloned.instance_id(),
        "a clone diverges independently, so it must not alias the original's \
         (instance_id, revision) key space"
    );
    assert_eq!(
        original.revision(),
        cloned.revision(),
        "revision itself still carries over — only the identity is re-minted"
    );

    let json = serde_json::to_string(&original).unwrap();
    let deserialized: Scene = serde_json::from_str(&json).unwrap();
    assert_ne!(
        original.instance_id(),
        deserialized.instance_id(),
        "a reparse is a new instance — its restarted revision counter must \
         not collide with the source scene's"
    );
    assert_ne!(deserialized.instance_id(), 0);
}
