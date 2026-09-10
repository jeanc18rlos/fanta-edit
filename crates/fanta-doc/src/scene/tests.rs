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
fn curve_body_hits_survive_parent_transforms_and_spatial_index_refresh() {
    for cubic in [false, true] {
        let mut scene = Scene::new();
        let mut parent = group_node();
        parent.transform = Transform2D::rotation(0.3).then(&Transform2D::translation(200., 100.));
        let parent_id = parent.id;
        scene.insert(parent).expect("parent");
        let mut path = crate::PathData::new();
        path.move_to(0., 0.);
        let midpoint = if cubic {
            path.cubic_to(0., 60., 100., 60., 100., 0.);
            DVec2::new(50., 45.)
        } else {
            path.quad_to(50., 80., 100., 0.);
            DVec2::new(50., 40.)
        };
        let mut node = CanvasNode::new(NodeData::Vector(VectorNode {
            path,
            ..Default::default()
        }));
        node.parent = Some(parent_id);
        node.transform = Transform2D::scale_xy(2., 0.5);
        let id = node.id;
        scene.insert(node).expect("curve");
        let old_point = scene
            .world_transform(id)
            .expect("transform")
            .transform_point(midpoint);
        for moved in [false, true] {
            if moved {
                scene.get_mut(id).expect("curve").transform =
                    Transform2D::scale_xy(2., 0.5).then(&Transform2D::translation(300., 200.));
                assert_eq!(scene.hit_test(old_point), None);
            }
            let world = scene
                .world_transform(id)
                .expect("transform")
                .transform_point(midpoint);
            assert!(
                scene
                    .world_bounds(id)
                    .expect("curve bounds")
                    .contains_point(world)
            );
            assert!(
                scene
                    .world_bounds(parent_id)
                    .expect("parent bounds")
                    .contains_point(world)
            );
            assert_eq!(scene.hit_test(world), Some(id));
            let probe = Bounds::from_min_max(world - DVec2::splat(1.), world + DVec2::splat(1.));
            assert_eq!(
                scene.rect_query_where(probe, |_, bounds| bounds.intersects(&probe)),
                vec![id]
            );
        }
    }
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

// ---- targeted invalidation + change log ----------------------------------

/// A tree deep enough to have ancestors, descendants, siblings and unrelated
/// roots around one "moved" node:
///
/// ```text
/// root_a (group)          root_b (group)
/// ├── mid (group)         └── b_leaf
/// │   ├── inner (group)   ← the node the tests move / patch
/// │   │   ├── inner_leaf_0
/// │   │   └── inner_leaf_1
/// │   └── mid_leaf
/// └── sibling_leaf
/// ```
struct DeepScene {
    scene: Scene,
    root_a: NodeId,
    mid: NodeId,
    inner: NodeId,
    inner_leaves: Vec<NodeId>,
    mid_leaf: NodeId,
    sibling_leaf: NodeId,
    root_b: NodeId,
    b_leaf: NodeId,
}

impl DeepScene {
    fn build() -> Self {
        let mut scene = Scene::new();
        let mut add = |mut node: CanvasNode, parent: Option<NodeId>, x: f64, y: f64| {
            node.parent = parent;
            node.transform = Transform2D::translation(x, y);
            node.index = scene.next_child_index(parent);
            let id = node.id;
            scene.insert(node).unwrap();
            id
        };
        let root_a = add(group_node(), None, 10.0, 20.0);
        let mid = add(group_node(), Some(root_a), 5.0, 5.0);
        let inner = add(group_node(), Some(mid), 1.0, 2.0);
        let inner_leaves = vec![
            add(rect_node(0.0, 0.0, 10.0, 10.0), Some(inner), 0.0, 0.0),
            add(rect_node(0.0, 0.0, 10.0, 10.0), Some(inner), 30.0, 0.0),
        ];
        let mid_leaf = add(rect_node(0.0, 0.0, 8.0, 8.0), Some(mid), 100.0, 0.0);
        let sibling_leaf = add(rect_node(0.0, 0.0, 6.0, 6.0), Some(root_a), 0.0, 100.0);
        let root_b = add(group_node(), None, 500.0, 500.0);
        let b_leaf = add(rect_node(0.0, 0.0, 20.0, 20.0), Some(root_b), 3.0, 3.0);
        Self {
            scene,
            root_a,
            mid,
            inner,
            inner_leaves,
            mid_leaf,
            sibling_leaf,
            root_b,
            b_leaf,
        }
    }

    fn all_ids(&self) -> Vec<NodeId> {
        let mut ids = vec![
            self.root_a,
            self.mid,
            self.inner,
            self.mid_leaf,
            self.sibling_leaf,
            self.root_b,
            self.b_leaf,
        ];
        ids.extend(self.inner_leaves.iter().copied());
        ids
    }

    /// Fill every cache entry the render/hit-test paths would.
    fn warm(&self) {
        for id in self.all_ids() {
            self.scene.world_transform(id);
            self.scene.world_bounds(id);
        }
        self.scene.hit_test(DVec2::new(-1e9, -1e9));
    }
}

/// A copy of `scene` with every derived cache dropped — the uncached fold the
/// warm scene is checked against.
fn cold_oracle(scene: &Scene) -> Scene {
    let fresh = scene.clone();
    fresh.clear_derived_caches();
    fresh
}

fn assert_geometry_matches_cold_fold(scene: &Scene, ids: &[NodeId]) {
    let oracle = cold_oracle(scene);
    for &id in ids {
        assert_eq!(
            scene.world_transform(id),
            oracle.world_transform(id),
            "world_transform of {id} diverged from the uncached fold"
        );
        assert_eq!(
            scene.world_bounds(id),
            oracle.world_bounds(id),
            "world_bounds of {id} diverged from the uncached walk"
        );
        assert_eq!(
            scene.local_bounds(id),
            oracle.local_bounds(id),
            "local_bounds of {id} diverged from the uncached walk"
        );
    }
}

#[test]
fn set_transform_on_a_deep_subtree_matches_a_fresh_fold_everywhere() {
    let mut deep = DeepScene::build();
    deep.warm();
    deep.scene
        .set_transform(deep.inner, Transform2D::translation(-40.0, 75.0))
        .unwrap();
    assert_geometry_matches_cold_fold(&deep.scene, &deep.all_ids());
    // The moved node really moved: its leaf follows the new local transform.
    let leaf = deep.scene.world_bounds(deep.inner_leaves[0]).unwrap();
    assert!((leaf.min_x - (10.0 + 5.0 - 40.0)).abs() < 1e-9);
    assert!((leaf.min_y - (20.0 + 5.0 + 75.0)).abs() < 1e-9);
    // Hit-testing through the rebuilt index agrees with brute force at the
    // new and old positions.
    for probe in [leaf.center(), DVec2::new(16.0 + 5.0, 27.0 + 5.0)] {
        assert_eq!(deep.scene.hit_test(probe), deep.scene.hit_test_brute(probe));
    }
}

#[test]
fn set_transform_of_a_root_and_of_a_leaf_match_a_fresh_fold() {
    let mut deep = DeepScene::build();
    deep.warm();
    deep.scene
        .set_transform(deep.root_a, Transform2D::translation(0.0, 0.0))
        .unwrap();
    assert_geometry_matches_cold_fold(&deep.scene, &deep.all_ids());
    deep.warm();
    deep.scene
        .set_transform(deep.inner_leaves[1], Transform2D::translation(-30.0, 60.0))
        .unwrap();
    assert_geometry_matches_cold_fold(&deep.scene, &deep.all_ids());
}

#[test]
fn set_transform_keeps_unrelated_cache_entries_and_drops_the_affected_ones() {
    let mut deep = DeepScene::build();
    deep.warm();
    deep.scene
        .set_transform(deep.inner, Transform2D::translation(-40.0, 75.0))
        .unwrap();

    let world = deep.scene.world_cache.borrow();
    let local = deep.scene.local_bounds_cache.borrow();
    // World transforms: the moved node and its descendants are gone…
    for id in std::iter::once(deep.inner).chain(deep.inner_leaves.iter().copied()) {
        assert!(
            !world.contains_key(&id),
            "stale world transform kept for {id}"
        );
    }
    // …while ancestors, siblings and the other root keep theirs.
    for id in [
        deep.root_a,
        deep.mid,
        deep.mid_leaf,
        deep.sibling_leaf,
        deep.root_b,
        deep.b_leaf,
    ] {
        assert!(
            world.contains_key(&id),
            "unrelated world transform dropped for {id}"
        );
    }
    // Local bounds: the ancestors' unions are gone, the node's own bounds and
    // everything else survive.
    for id in [deep.mid, deep.root_a] {
        assert!(
            !local.contains_key(&id),
            "stale local bounds kept for ancestor {id}"
        );
    }
    for id in std::iter::once(deep.inner)
        .chain(deep.inner_leaves.iter().copied())
        .chain([deep.mid_leaf, deep.sibling_leaf, deep.root_b, deep.b_leaf])
    {
        assert!(
            local.contains_key(&id),
            "unrelated local bounds dropped for {id}"
        );
    }
    assert!(
        deep.scene.spatial_index.borrow().is_none(),
        "the spatial index snapshots world AABBs and must be rebuilt lazily"
    );
}

#[test]
fn clone_starts_without_a_spatial_index() {
    let deep = DeepScene::build();
    deep.warm();
    assert!(deep.scene.spatial_index.borrow().is_some());
    let cloned = deep.scene.clone();
    assert!(cloned.spatial_index.borrow().is_none());
    // And still answers hit-tests identically once it rebuilds.
    let probe = deep.scene.world_bounds(deep.b_leaf).unwrap().center();
    assert_eq!(cloned.hit_test(probe), deep.scene.hit_test(probe));
}

#[test]
fn snapshot_stays_unchanged_across_node_and_hierarchy_edits()
-> Result<(), Box<dyn std::error::Error>> {
    let mut deep = DeepScene::build();
    deep.warm();
    let snapshot = deep.scene.clone();
    let expected = serde_json::to_value(&snapshot)?;
    let expected_bounds = snapshot.world_bounds(deep.root_a);

    deep.scene
        .get_mut(deep.mid_leaf)
        .ok_or(SceneError::NotFound(deep.mid_leaf))?
        .name = "Renamed after snapshot".into();
    deep.scene
        .set_transform(deep.inner, Transform2D::translation(500.0, 250.0))?;
    deep.scene.set_parent(
        deep.sibling_leaf,
        Some(deep.root_b),
        deep.scene.next_child_index(Some(deep.root_b)),
    )?;
    deep.scene.set_index(deep.mid, IndexKey::from_raw(500.0))?;
    let mut replacement = deep
        .scene
        .get(deep.b_leaf)
        .ok_or(SceneError::NotFound(deep.b_leaf))?
        .clone();
    replacement.name = "Patched after snapshot".into();
    deep.scene.patch_node(replacement, next_geometry_stamp())?;
    let mut removed = deep.scene.remove(deep.inner)?;
    removed.name = "Removed root is independently owned".into();
    let mut shared_root_removed = deep.scene.remove(deep.root_a)?;
    shared_root_removed.name = "Shared removed root is independently owned".into();
    deep.scene
        .insert_many([rect_node(0.0, 0.0, 20.0, 20.0), group_node()])?;
    deep.scene.insert(rect_node(10.0, 10.0, 30.0, 30.0))?;

    deep.scene.validate()?;
    snapshot.validate()?;
    assert_ne!(serde_json::to_value(&deep.scene)?, expected);
    assert_eq!(serde_json::to_value(&snapshot)?, expected);
    assert_eq!(snapshot.world_bounds(deep.root_a), expected_bounds);
    assert_geometry_matches_cold_fold(&snapshot, &deep.all_ids());
    Ok(())
}

#[test]
fn snapshot_serializes_nodes_without_changing_the_document_shape()
-> Result<(), Box<dyn std::error::Error>> {
    let mut scene = Scene::new();
    let node = rect_node(0.0, 0.0, 10.0, 10.0);
    let id = node.id;
    let expected_node = serde_json::to_value(&node)?;
    scene.insert(node)?;
    let snapshot = scene.clone();
    let serialized = serde_json::to_value(&snapshot)?;
    assert_eq!(
        serialized
            .get("nodes")
            .and_then(|nodes| nodes.get(id.0.to_string())),
        Some(&expected_node)
    );

    let mut restored: Scene = serde_json::from_value(serialized)?;
    restored.rebuild_child_index();
    restored.validate()?;
    assert_eq!(restored.get(id), scene.get(id));
    restored.get_mut(id).ok_or(SceneError::NotFound(id))?.name = "Restored edit".into();
    assert_eq!(snapshot.get(id), scene.get(id));
    assert_ne!(restored.get(id), snapshot.get(id));
    Ok(())
}

#[test]
fn changes_since_the_current_revision_is_empty() {
    let deep = DeepScene::build();
    let delta = deep.scene.changes_since(deep.scene.revision()).unwrap();
    assert!(delta.is_empty());
}

#[test]
fn changes_since_reports_a_contiguous_deduplicated_transform_run() {
    let mut deep = DeepScene::build();
    let start = deep.scene.revision();
    for frame in 0..5 {
        deep.scene
            .set_transform(deep.inner, Transform2D::translation(frame as f64, 0.0))
            .unwrap();
        deep.scene
            .set_transform(deep.b_leaf, Transform2D::translation(0.0, frame as f64))
            .unwrap();
    }
    let delta = deep.scene.changes_since(start).unwrap();
    let mut expected = vec![deep.inner, deep.b_leaf];
    expected.sort_unstable();
    assert_eq!(delta.transforms, expected);
    assert!(delta.nodes.is_empty());
    // A copy that caught up half-way sees only the later frames' nodes.
    let mid_revision = start + 3;
    let later = deep.scene.changes_since(mid_revision).unwrap();
    assert_eq!(later.transforms, expected);
}

#[test]
fn changes_since_folds_a_transform_into_a_node_change() {
    let mut deep = DeepScene::build();
    let start = deep.scene.revision();
    deep.scene
        .set_transform(deep.inner, Transform2D::translation(1.0, 1.0))
        .unwrap();
    deep.scene.get_mut(deep.inner).unwrap().name = "renamed".into();
    deep.scene
        .set_transform(deep.inner, Transform2D::translation(2.0, 2.0))
        .unwrap();
    deep.scene
        .set_transform(deep.mid_leaf, Transform2D::translation(2.0, 2.0))
        .unwrap();
    let delta = deep.scene.changes_since(start).unwrap();
    assert_eq!(delta.nodes, vec![deep.inner]);
    assert_eq!(delta.transforms, vec![deep.mid_leaf]);
}

#[test]
fn changes_since_gives_up_on_structural_and_unknown_edits() {
    let mut deep = DeepScene::build();
    let start = deep.scene.revision();
    deep.scene
        .set_transform(deep.inner, Transform2D::translation(1.0, 1.0))
        .unwrap();
    deep.scene.invalidate_world_cache();
    assert!(
        deep.scene.changes_since(start).is_none(),
        "an untracked edit poisons every delta spanning it"
    );
    // Once past it, deltas resume.
    let after_unknown = deep.scene.revision();
    deep.scene
        .set_transform(deep.inner, Transform2D::translation(3.0, 3.0))
        .unwrap();
    assert_eq!(
        deep.scene.changes_since(after_unknown).unwrap().transforms,
        vec![deep.inner]
    );

    let before_insert = deep.scene.revision();
    let mut extra = rect_node(0.0, 0.0, 1.0, 1.0);
    extra.parent = Some(deep.root_b);
    deep.scene.insert(extra).unwrap();
    assert!(deep.scene.changes_since(before_insert).is_none());

    let before_reparent = deep.scene.revision();
    deep.scene
        .set_parent(deep.mid_leaf, Some(deep.root_b), IndexKey::FIRST)
        .unwrap();
    assert!(deep.scene.changes_since(before_reparent).is_none());

    let before_reorder = deep.scene.revision();
    deep.scene
        .set_index(deep.sibling_leaf, IndexKey::from_raw(99.0))
        .unwrap();
    assert!(deep.scene.changes_since(before_reorder).is_none());

    let before_remove = deep.scene.revision();
    deep.scene.remove(deep.b_leaf).unwrap();
    assert!(deep.scene.changes_since(before_remove).is_none());

    let before_rebuild = deep.scene.revision();
    deep.scene.rebuild_child_index();
    assert!(deep.scene.changes_since(before_rebuild).is_none());
}

#[test]
fn changes_since_gives_up_when_a_touched_node_is_missing() {
    let deep = DeepScene::build();
    let start = deep.scene.revision();
    // No public mutator logs a node it does not hold any more (`get_mut`
    // returns before logging on a miss), so the guard is exercised at the
    // log directly.
    deep.scene.record_change(SceneChange::Node(NodeId::new()));
    assert!(deep.scene.changes_since(start).is_none());
}

#[test]
fn changes_since_gives_up_past_the_log_window() {
    let mut deep = DeepScene::build();
    let start = deep.scene.revision();
    for frame in 0..=SCENE_CHANGE_LOG_CAP {
        deep.scene
            .set_transform(deep.inner, Transform2D::translation(frame as f64, 0.0))
            .unwrap();
    }
    assert!(deep.scene.changes_since(start).is_none());
    // The newest CAP entries are still served.
    let inside_window = deep.scene.revision() - (SCENE_CHANGE_LOG_CAP as u64);
    assert_eq!(
        deep.scene.changes_since(inside_window).unwrap().transforms,
        vec![deep.inner]
    );
    let just_outside = inside_window - 1;
    assert!(deep.scene.changes_since(just_outside).is_none());
}

#[test]
fn changes_since_gives_up_beyond_the_node_cap() {
    let mut scene = Scene::new();
    let ids: Vec<NodeId> = (0..=SCENE_DELTA_MAX_NODES)
        .map(|_| {
            let node = rect_node(0.0, 0.0, 1.0, 1.0);
            let id = node.id;
            scene.insert(node).unwrap();
            id
        })
        .collect();
    let start = scene.revision();
    for &id in &ids[..SCENE_DELTA_MAX_NODES] {
        scene
            .set_transform(id, Transform2D::translation(1.0, 0.0))
            .unwrap();
    }
    assert_eq!(
        scene.changes_since(start).unwrap().transforms.len(),
        SCENE_DELTA_MAX_NODES,
        "exactly the cap is still a delta"
    );
    scene
        .set_transform(
            ids[SCENE_DELTA_MAX_NODES],
            Transform2D::translation(1.0, 0.0),
        )
        .unwrap();
    assert!(scene.changes_since(start).is_none());
}

#[test]
fn patch_node_brings_a_clone_into_parity_with_the_original() {
    let mut deep = DeepScene::build();
    deep.warm();
    let mut copy = deep.scene.clone();
    // Warm the copy too, so stale entries would show if invalidation missed.
    for id in deep.all_ids() {
        copy.world_bounds(id);
    }
    copy.hit_test(DVec2::ZERO);

    let copy_revision = copy.revision();
    {
        let inner = deep.scene.get_mut(deep.inner).unwrap();
        inner.transform = Transform2D::translation(-40.0, 75.0);
        inner.name = "moved".into();
    }
    {
        let leaf = deep.scene.get_mut(deep.b_leaf).unwrap();
        leaf.data = rect_node(0.0, 0.0, 200.0, 200.0).data;
        leaf.flags |= crate::node::NodeFlags::HIDDEN;
    }
    deep.scene
        .set_transform(deep.mid_leaf, Transform2D::translation(7.0, 7.0))
        .unwrap();

    let delta = deep.scene.changes_since(copy_revision).unwrap();
    for id in delta.transforms {
        let transform = deep.scene.get(id).unwrap().transform;
        copy.set_transform(id, transform).unwrap();
    }
    for id in delta.nodes {
        let node = deep.scene.get(id).unwrap().clone();
        copy.patch_node(node, deep.scene.node_stamp(id)).unwrap();
    }

    for id in deep.all_ids() {
        assert_eq!(copy.get(id), deep.scene.get(id), "node {id} differs");
        assert_eq!(copy.world_transform(id), deep.scene.world_transform(id));
        assert_eq!(copy.world_bounds(id), deep.scene.world_bounds(id));
        assert_eq!(
            copy.node_stamp(id),
            deep.scene.node_stamp(id),
            "stamp of {id}"
        );
    }
    assert_geometry_matches_cold_fold(&copy, &deep.all_ids());
    for id in deep.all_ids() {
        if let Some(bounds) = deep.scene.world_bounds(id) {
            let probe = bounds.center();
            assert_eq!(copy.hit_test(probe), deep.scene.hit_test(probe));
            assert_eq!(copy.hit_test(probe), copy.hit_test_brute(probe));
        }
    }
    copy.validate().unwrap();
}

#[test]
fn patch_node_refuses_a_hierarchy_change_and_a_missing_node() {
    let mut deep = DeepScene::build();
    let mut reparented = deep.scene.get(deep.mid_leaf).unwrap().clone();
    reparented.parent = Some(deep.root_b);
    assert!(matches!(
        deep.scene.patch_node(reparented, 1),
        Err(SceneError::InvariantViolated(_))
    ));
    let mut reordered = deep.scene.get(deep.mid_leaf).unwrap().clone();
    reordered.index = IndexKey::from_raw(1234.0);
    assert!(matches!(
        deep.scene.patch_node(reordered, 1),
        Err(SceneError::InvariantViolated(_))
    ));
    assert!(matches!(
        deep.scene.patch_node(rect_node(0.0, 0.0, 1.0, 1.0), 1),
        Err(SceneError::NotFound(_))
    ));
    deep.scene.validate().unwrap();
}

#[test]
fn patch_node_logs_a_node_change_and_targets_its_invalidation() {
    let mut deep = DeepScene::build();
    deep.warm();
    let start = deep.scene.revision();
    let mut replacement = deep.scene.get(deep.inner).unwrap().clone();
    replacement.transform = Transform2D::translation(9.0, 9.0);
    deep.scene.patch_node(replacement, 77).unwrap();
    assert_eq!(deep.scene.node_stamp(deep.inner), 77);
    assert_eq!(
        deep.scene.changes_since(start).unwrap().nodes,
        vec![deep.inner]
    );
    {
        let world = deep.scene.world_cache.borrow();
        let local = deep.scene.local_bounds_cache.borrow();
        assert!(!world.contains_key(&deep.inner));
        assert!(!world.contains_key(&deep.inner_leaves[0]));
        assert!(world.contains_key(&deep.sibling_leaf));
        assert!(!local.contains_key(&deep.inner));
        assert!(!local.contains_key(&deep.mid));
        assert!(local.contains_key(&deep.inner_leaves[0]));
        assert!(local.contains_key(&deep.b_leaf));
    }
    assert_geometry_matches_cold_fold(&deep.scene, &deep.all_ids());
}

#[test]
fn get_mut_of_a_missing_node_leaves_revision_log_and_caches_untouched() {
    let mut deep = DeepScene::build();
    deep.warm();
    let start = deep.scene.revision();
    let warm_world_entries = deep.scene.world_cache.borrow().len();
    let warm_local_entries = deep.scene.local_bounds_cache.borrow().len();
    let missing = rect_node(0.0, 0.0, 1.0, 1.0).id;

    assert!(deep.scene.get_mut(missing).is_none());

    assert_eq!(
        deep.scene.revision(),
        start,
        "a miss must not bump the revision"
    );
    assert_eq!(
        deep.scene.changes_since(start),
        Some(SceneDelta::default()),
        "a miss must not be logged"
    );
    assert_eq!(deep.scene.world_cache.borrow().len(), warm_world_entries);
    assert_eq!(
        deep.scene.local_bounds_cache.borrow().len(),
        warm_local_entries
    );
    assert_eq!(deep.scene.node_stamp(missing), 0);
}

/// A batch whose parents come both before and after their children, with
/// equal and distinct index keys under every parent, some of which already
/// exist in the scene. Returns the nodes in the batch order to feed
/// `insert_many`, plus the same nodes in a parent-first order a sequential
/// `insert` accepts.
fn interleaved_batch(scene: &Scene, existing_group: NodeId) -> (Vec<CanvasNode>, Vec<CanvasNode>) {
    let mut group_a = group_node();
    group_a.index = IndexKey::FIRST;
    let mut group_b = group_node();
    group_b.index = IndexKey::FIRST;
    let mut leaf_a1 = rect_node(0.0, 0.0, 1.0, 1.0);
    leaf_a1.parent = Some(group_a.id);
    leaf_a1.index = IndexKey::from_raw(2.0);
    let mut leaf_a2 = rect_node(0.0, 0.0, 1.0, 1.0);
    leaf_a2.parent = Some(group_a.id);
    leaf_a2.index = IndexKey::from_raw(2.0);
    let mut leaf_a3 = rect_node(0.0, 0.0, 1.0, 1.0);
    leaf_a3.parent = Some(group_a.id);
    leaf_a3.index = IndexKey::from_raw(0.5);
    let mut leaf_b1 = rect_node(0.0, 0.0, 1.0, 1.0);
    leaf_b1.parent = Some(group_b.id);
    leaf_b1.index = IndexKey::FIRST;
    let mut nested = group_node();
    nested.parent = Some(group_b.id);
    nested.index = IndexKey::FIRST;
    let mut nested_leaf = rect_node(0.0, 0.0, 1.0, 1.0);
    nested_leaf.parent = Some(nested.id);
    nested_leaf.index = IndexKey::FIRST;
    // Under the pre-existing group: one key equal to the existing children's
    // and one that sorts between them.
    let existing_key = scene.children_of(Some(existing_group))[0];
    let mut under_existing_equal = rect_node(0.0, 0.0, 1.0, 1.0);
    under_existing_equal.parent = Some(existing_group);
    under_existing_equal.index = scene.get(existing_key).unwrap().index;
    let mut under_existing_between = rect_node(0.0, 0.0, 1.0, 1.0);
    under_existing_between.parent = Some(existing_group);
    under_existing_between.index = IndexKey::from_raw(1.5);
    let mut new_root = rect_node(0.0, 0.0, 1.0, 1.0);
    new_root.index = IndexKey::from_raw(0.0);

    let sequential = vec![
        group_a.clone(),
        group_b.clone(),
        nested.clone(),
        leaf_a1.clone(),
        leaf_a2.clone(),
        leaf_a3.clone(),
        leaf_b1.clone(),
        nested_leaf.clone(),
        under_existing_equal.clone(),
        under_existing_between.clone(),
        new_root.clone(),
    ];
    let batch = vec![
        nested_leaf,
        leaf_a2,
        under_existing_between,
        group_b,
        leaf_a1,
        nested,
        new_root,
        leaf_b1,
        group_a,
        under_existing_equal,
        leaf_a3,
    ];
    (batch, sequential)
}

fn seeded_scene() -> (Scene, NodeId) {
    let mut scene = Scene::new();
    let mut group = group_node();
    group.index = IndexKey::from_raw(3.0);
    let group_id = group.id;
    scene.insert(group).unwrap();
    for _ in 0..3 {
        let mut child = rect_node(0.0, 0.0, 1.0, 1.0);
        child.parent = Some(group_id);
        child.index = IndexKey::FIRST;
        scene.insert(child).unwrap();
    }
    let mut child = rect_node(0.0, 0.0, 1.0, 1.0);
    child.parent = Some(group_id);
    child.index = IndexKey::from_raw(2.0);
    scene.insert(child).unwrap();
    (scene, group_id)
}

#[test]
fn insert_many_matches_sequential_insert_order_byte_for_byte() {
    let (mut batched, existing_group) = seeded_scene();
    let mut sequential = batched.clone();
    let (batch, ordered) = interleaved_batch(&batched, existing_group);
    let expected_ids: Vec<NodeId> = batch.iter().map(|node| node.id).collect();

    let inserted = batched.insert_many(batch).unwrap();
    for node in ordered {
        sequential.insert(node).unwrap();
    }

    assert_eq!(inserted, expected_ids, "ids come back in batch order");
    assert_eq!(batched.len(), sequential.len());
    for id in std::iter::once(None).chain(sequential.nodes.keys().copied().map(Some)) {
        assert_eq!(
            batched.children_of(id),
            sequential.children_of(id),
            "child order under {id:?} diverges from sequential insertion"
        );
    }
    assert_eq!(batched.child_index.len(), sequential.child_index.len());
    batched.validate().unwrap();
    for id in &inserted {
        let batched_node = batched.get(*id).unwrap();
        let sequential_node = sequential.get(*id).unwrap();
        assert_eq!(batched_node.parent, sequential_node.parent);
        assert_eq!(batched_node.index, sequential_node.index);
    }
}

#[test]
fn insert_many_rejections_leave_the_scene_untouched() {
    let (mut scene, existing_group) = seeded_scene();
    let existing_leaf = scene.children_of(Some(existing_group))[0];
    scene.world_bounds(existing_group);
    let len = scene.len();
    let start = scene.revision();
    let warm_world_entries = scene.world_cache.borrow().len();
    let stamps_before: Vec<u64> = scene.nodes.keys().map(|id| scene.node_stamp(*id)).collect();

    let assert_untouched = |scene: &Scene, label: &str| {
        assert_eq!(scene.len(), len, "{label}: node count changed");
        assert_eq!(scene.revision(), start, "{label}: revision changed");
        assert_eq!(
            scene.changes_since(start),
            Some(SceneDelta::default()),
            "{label}: change logged"
        );
        assert_eq!(
            scene.world_cache.borrow().len(),
            warm_world_entries,
            "{label}: caches cleared"
        );
        let stamps_after: Vec<u64> = scene.nodes.keys().map(|id| scene.node_stamp(*id)).collect();
        assert_eq!(stamps_after, stamps_before, "{label}: stamps moved");
        scene.validate().unwrap();
    };

    // Duplicate against the scene, placed after a valid node so the valid
    // node must not slip in before the batch is rejected.
    let duplicate_of_existing = scene.get(existing_leaf).unwrap().clone();
    let fresh = rect_node(0.0, 0.0, 1.0, 1.0);
    let fresh_id = fresh.id;
    assert!(matches!(
        scene.insert_many(vec![fresh, duplicate_of_existing]),
        Err(SceneError::Duplicate(id)) if id == existing_leaf
    ));
    assert!(!scene.contains(fresh_id));
    assert_untouched(&scene, "duplicate against scene");

    let twice = rect_node(0.0, 0.0, 1.0, 1.0);
    let twice_id = twice.id;
    assert!(matches!(
        scene.insert_many(vec![twice.clone(), twice]),
        Err(SceneError::Duplicate(id)) if id == twice_id
    ));
    assert_untouched(&scene, "duplicate within batch");

    let mut orphan = rect_node(0.0, 0.0, 1.0, 1.0);
    let missing_parent = rect_node(0.0, 0.0, 1.0, 1.0).id;
    orphan.parent = Some(missing_parent);
    assert!(matches!(
        scene.insert_many(vec![orphan]),
        Err(SceneError::ParentMissing(id)) if id == missing_parent
    ));
    assert_untouched(&scene, "missing parent");

    let mut under_leaf = rect_node(0.0, 0.0, 1.0, 1.0);
    under_leaf.parent = Some(existing_leaf);
    assert!(matches!(
        scene.insert_many(vec![under_leaf]),
        Err(SceneError::ParentNotContainer(id)) if id == existing_leaf
    ));
    assert_untouched(&scene, "non-container parent in scene");

    let batch_leaf = rect_node(0.0, 0.0, 1.0, 1.0);
    let batch_leaf_id = batch_leaf.id;
    let mut under_batch_leaf = rect_node(0.0, 0.0, 1.0, 1.0);
    under_batch_leaf.parent = Some(batch_leaf_id);
    assert!(matches!(
        scene.insert_many(vec![under_batch_leaf, batch_leaf]),
        Err(SceneError::ParentNotContainer(id)) if id == batch_leaf_id
    ));
    assert_untouched(&scene, "non-container parent in batch");

    let mut ring_a = group_node();
    let mut ring_b = group_node();
    let mut ring_c = group_node();
    ring_a.parent = Some(ring_c.id);
    ring_b.parent = Some(ring_a.id);
    ring_c.parent = Some(ring_b.id);
    let mut hanging = rect_node(0.0, 0.0, 1.0, 1.0);
    hanging.parent = Some(ring_a.id);
    assert!(matches!(
        scene.insert_many(vec![hanging, ring_a, ring_b, ring_c]),
        Err(SceneError::Cycle { .. })
    ));
    assert_untouched(&scene, "cycle within batch");
}

#[test]
fn insert_many_logs_one_structural_change_and_stamps_inserted_nodes_and_parents() {
    let (mut scene, existing_group) = seeded_scene();
    let mut bystander = group_node();
    bystander.index = IndexKey::from_raw(9.0);
    let bystander_id = bystander.id;
    scene.insert(bystander).unwrap();
    scene.world_bounds(existing_group);
    let start = scene.revision();
    let group_stamp_before = scene.node_stamp(existing_group);
    let bystander_stamp_before = scene.node_stamp(bystander_id);

    let mut batch_group = group_node();
    batch_group.index = IndexKey::from_raw(4.0);
    let mut under_batch_group = rect_node(0.0, 0.0, 1.0, 1.0);
    under_batch_group.parent = Some(batch_group.id);
    let mut under_existing = rect_node(0.0, 0.0, 1.0, 1.0);
    under_existing.parent = Some(existing_group);
    under_existing.index = IndexKey::from_raw(5.0);
    let inserted = scene
        .insert_many(vec![under_batch_group, batch_group, under_existing])
        .unwrap();

    assert_eq!(
        scene.revision(),
        start + 1,
        "one revision bump for the batch"
    );
    assert_eq!(
        scene.changes_since(start),
        None,
        "the batch is a structural change a copy cannot patch"
    );
    assert!(
        scene.world_cache.borrow().is_empty(),
        "derived caches cleared"
    );
    for id in &inserted {
        assert!(scene.node_stamp(*id) > 0, "inserted node {id} is stamped");
    }
    assert!(
        scene.node_stamp(existing_group) > group_stamp_before,
        "an existing parent that gained children is stamped"
    );
    assert_eq!(
        scene.node_stamp(bystander_id),
        bystander_stamp_before,
        "an untouched node keeps its stamp"
    );
    scene.validate().unwrap();

    let untouched = scene.revision();
    assert_eq!(scene.insert_many(Vec::new()).unwrap(), Vec::<NodeId>::new());
    assert_eq!(scene.revision(), untouched, "an empty batch is not an edit");
}

#[test]
fn rebuild_child_index_matches_insert_many_order() {
    let (mut scene, existing_group) = seeded_scene();
    let (batch, _) = interleaved_batch(&scene, existing_group);
    scene.insert_many(batch).unwrap();
    let live = scene.child_index.clone();
    scene.rebuild_child_index();
    assert_eq!(scene.child_index, live);
}
