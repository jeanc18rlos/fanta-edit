//! Brute-force pre-index oracles and the parity tests that assert the
//! spatial-index-accelerated queries agree with them over randomized scenes.

use super::*;
// ---- brute-force oracles for parity testing -----------------------------
// The production functions delegate to the scene's spatial index; these
// recursive walks are the pre-index reference. Parity tests assert the two
// agree over randomized scenes.

fn brute_deep(scene: &Scene, id: NodeId, p: DVec2, precision: HitPrecision, acc: &mut Vec<NodeId>) {
    let Some(node) = scene.get(id) else { return };
    // A hidden or locked node — and its whole subtree — is not a point-hit
    // target, mirroring `hit_test`/`hit_test_deep`'s `locked_by_flags` (which
    // rejects a node with a locked ancestor) and the marquee oracle above.
    if node.flags.intersects(NodeFlags::HIDDEN | NodeFlags::LOCKED) {
        return;
    }
    let Some(bounds) = scene.world_bounds(id) else {
        return;
    };
    if !bounds.contains_point(p) {
        return;
    }
    for &child in scene.children_of(Some(id)).iter().rev() {
        brute_deep(scene, child, p, precision, acc);
    }
    // Plain groups never self-catch; frame surfaces (clip/background) do —
    // kept in parity with `Scene::deep_hits_where`/`topmost_hit_where`.
    if let NodeData::Group(g) = &node.data {
        if !g.is_frame_surface() {
            return;
        }
    }
    match (precision, &node.data) {
        (HitPrecision::Path, NodeData::Vector(v)) => {
            let local = world_to_local(scene, node, p);
            if point_in_path(&v.path, local) {
                acc.push(id);
            }
        }
        _ => acc.push(id),
    }
}

fn brute_deep_all(scene: &Scene, p: DVec2, precision: HitPrecision) -> Vec<NodeId> {
    let mut acc = Vec::new();
    for &root in scene.roots().iter().rev() {
        brute_deep(scene, root, p, precision, &mut acc);
    }
    acc
}

fn brute_within(scene: &Scene, id: NodeId, rect: Bounds, mode: MarqueeMode, acc: &mut Vec<NodeId>) {
    let Some(node) = scene.get(id) else { return };
    if node.flags.intersects(NodeFlags::HIDDEN | NodeFlags::LOCKED) {
        return;
    }
    if matches!(node.data, NodeData::Group(_)) {
        for &c in scene.children_of(Some(id)) {
            brute_within(scene, c, rect, mode, acc);
        }
        return;
    }
    if let Some(bb) = scene.world_bounds(id) {
        let included = match mode {
            MarqueeMode::Contains => {
                bb.min_x >= rect.min_x
                    && bb.min_y >= rect.min_y
                    && bb.max_x <= rect.max_x
                    && bb.max_y <= rect.max_y
            }
            MarqueeMode::Intersects => bb.intersects(&rect),
        };
        if included {
            acc.push(id);
        }
    }
    for &c in scene.children_of(Some(id)) {
        brute_within(scene, c, rect, mode, acc);
    }
}

fn brute_within_all(scene: &Scene, rect: Bounds, mode: MarqueeMode) -> Vec<NodeId> {
    let mut acc = Vec::new();
    for &root in scene.roots() {
        brute_within(scene, root, rect, mode, &mut acc);
    }
    acc
}

/// Tiny deterministic xorshift PRNG (no `rand` dependency).
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
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        let u = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        lo + u * (hi - lo)
    }
    fn chance(&mut self, p: f64) -> bool {
        self.range(0.0, 1.0) < p
    }
}

fn random_doc(seed: u64, n: usize) -> Doc {
    let mut rng = Rng::new(seed);
    let mut doc = Doc::new();
    let mut groups: Vec<NodeId> = Vec::new();
    for i in 0..n {
        if rng.chance(0.15) {
            let mut g = CanvasNode::new(NodeData::Group(GroupNode::default()));
            g.index = IndexKey::from_raw(i as f64);
            if rng.chance(0.25) {
                g.flags |= NodeFlags::HIDDEN;
            }
            if rng.chance(0.15) {
                g.flags |= NodeFlags::LOCKED;
            }
            let id = g.id;
            doc.apply(Operation::create_node(g)).unwrap();
            groups.push(id);
            continue;
        }
        let x = rng.range(-200.0, 200.0);
        let y = rng.range(-200.0, 200.0);
        let w = rng.range(1.0, 50.0);
        let h = rng.range(1.0, 50.0);
        let mut r = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            w,
            h,
            Color::WHITE,
        )));
        r.transform = Transform2D::translation(x, y);
        r.index = IndexKey::from_raw(i as f64);
        if !groups.is_empty() && rng.chance(0.4) {
            let gi = (rng.next_u64() as usize) % groups.len();
            r.parent = Some(groups[gi]);
        }
        if rng.chance(0.1) {
            r.flags |= NodeFlags::HIDDEN;
        }
        if rng.chance(0.1) {
            r.flags |= NodeFlags::LOCKED;
        }
        doc.apply(Operation::create_node(r)).unwrap();
    }
    doc
}

#[test]
fn indexed_topmost_matches_brute_force() {
    for seed in 1..=30u64 {
        let doc = random_doc(seed, 250);
        let mut rng = Rng::new(seed.wrapping_mul(0x9E37_79B9));
        for precision in [HitPrecision::Bounds, HitPrecision::Path] {
            for _ in 0..150 {
                let p = DVec2::new(rng.range(-220.0, 220.0), rng.range(-220.0, 220.0));
                let indexed = hit_test(&doc.scene, p, precision, None);
                // Brute oracle for topmost = first of the deep list.
                let brute = brute_deep_all(&doc.scene, p, precision).first().copied();
                assert_eq!(
                    indexed, brute,
                    "topmost parity seed {seed} p {p:?} {precision:?}"
                );
            }
        }
    }
}

#[test]
fn indexed_deep_matches_brute_force() {
    for seed in 1..=30u64 {
        let doc = random_doc(seed, 250);
        let mut rng = Rng::new(seed.wrapping_mul(0x85EB_CA6B));
        for precision in [HitPrecision::Bounds, HitPrecision::Path] {
            for _ in 0..120 {
                let p = DVec2::new(rng.range(-220.0, 220.0), rng.range(-220.0, 220.0));
                let indexed: Vec<NodeId> = hit_test_deep(&doc.scene, p, precision, None)
                    .into_iter()
                    .collect();
                let brute = brute_deep_all(&doc.scene, p, precision);
                assert_eq!(
                    indexed, brute,
                    "deep parity seed {seed} p {p:?} {precision:?}"
                );
            }
        }
    }
}

#[test]
fn indexed_marquee_matches_brute_force() {
    for seed in 1..=30u64 {
        let doc = random_doc(seed, 250);
        let mut rng = Rng::new(seed.wrapping_mul(0xC2B2_AE35));
        for mode in [MarqueeMode::Contains, MarqueeMode::Intersects] {
            for _ in 0..120 {
                let x0 = rng.range(-220.0, 220.0);
                let y0 = rng.range(-220.0, 220.0);
                let w = rng.range(1.0, 300.0);
                let h = rng.range(1.0, 300.0);
                let rect = Bounds::from_xywh(x0, y0, w, h);
                let indexed: Vec<NodeId> = hit_test_within(&doc.scene, rect, mode, None)
                    .into_iter()
                    .collect();
                let brute = brute_within_all(&doc.scene, rect, mode);
                assert_eq!(
                    indexed, brute,
                    "marquee parity seed {seed} rect {rect:?} {mode:?}"
                );
            }
        }
    }
}
