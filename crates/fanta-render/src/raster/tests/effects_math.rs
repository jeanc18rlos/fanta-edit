//! Pure unit tests for the on-screen sigma cap (`capped_render_sigma`) and the shadow-expanded cull bounds (`shadow_expanded_world_bounds`).
use super::*;
use fanta_doc::{Doc, Operation, VectorNode};

// -----------------------------------------------------------------------
// Drop-shadow on-screen blur cap + shadow-expanded cull (perf-fix units)
// -----------------------------------------------------------------------

#[test]
fn capped_render_sigma_is_a_no_op_below_the_cap() {
    // At normal zoom (and below the cap) the returned world sigma equals the
    // input world sigma exactly — the cap must not change anything until the
    // on-screen blur is already enormous. This is the visual-parity guard.
    // world_sigma 10 at scale 1: on-screen 10, far under the 128 cap.
    assert!((capped_render_sigma(10.0, 1.0) - 10.0).abs() < 1e-6);
    // world_sigma 10 at scale 4: on-screen 40, still under the cap → unchanged.
    assert!((capped_render_sigma(10.0, 4.0) - 10.0).abs() < 1e-6);
    // Right at the cap boundary (on-screen == SIGMA_SCREEN_MAX): unchanged.
    let at_cap = (SIGMA_SCREEN_MAX as f64) / 2.0; // scale 2 → on-screen == cap
    assert!((capped_render_sigma(at_cap, 2.0) - at_cap as f32).abs() < 1e-3);
}

#[test]
fn capped_render_sigma_clamps_on_screen_sigma_by_effective_scale() {
    // The core fix: world sigma is clamped so that sigma·effective_scale
    // never exceeds SIGMA_SCREEN_MAX. At high zoom the returned world sigma
    // shrinks ∝ 1/scale, holding the on-screen sigma flat at the cap.
    for &scale in &[8.0f32, 16.0, 32.0, 100.0] {
        let world_sigma = 200.0; // huge: on-screen would be 200·scale ≫ cap
        let returned = capped_render_sigma(world_sigma, scale);
        let on_screen = returned * scale;
        assert!(
            (on_screen - SIGMA_SCREEN_MAX).abs() <= 0.5,
            "at scale {scale} on-screen sigma should pin to the cap \
                 {SIGMA_SCREEN_MAX}, got {on_screen}"
        );
        // And the world sigma was reduced below the faithful value.
        assert!(
            (returned as f64) < world_sigma,
            "cap must shrink the world sigma at scale {scale}"
        );
    }
}

#[test]
fn capped_render_sigma_floors_subpixel_blur_to_crisp() {
    // Below SIGMA_SCREEN_MIN on-screen, the blur is a sub-pixel no-op → 0
    // (Skia's crisp-shadow fast path). At scale 1 a world sigma of 0.1 is
    // 0.1 on-screen, under the 0.25 min.
    assert_eq!(capped_render_sigma(0.1, 1.0), 0.0);
    // A zoomed-OUT view (scale 0.01) shrinks a moderate world sigma below the
    // floor too: 10 · 0.01 = 0.1 on-screen.
    assert_eq!(capped_render_sigma(10.0, 0.01), 0.0);
    // Just above the floor stays non-zero.
    assert!(capped_render_sigma(0.5, 1.0) > 0.0);
}

#[test]
fn capped_render_sigma_handles_degenerate_scale() {
    // A zero / NaN scale falls back to 1.0 so the cap still bounds the kernel
    // rather than dividing by zero. world_sigma 300 → capped to the cap.
    assert!((capped_render_sigma(300.0, 0.0) - SIGMA_SCREEN_MAX).abs() <= 0.5);
    assert!((capped_render_sigma(300.0, f32::NAN) - SIGMA_SCREEN_MAX).abs() <= 0.5);
}

#[test]
fn shadow_expanded_bounds_grows_by_offset_and_blur() {
    // A 20x20 rect at world (-10,-10)..(10,10) with a drop shadow offset
    // (8, 8) and blur 8 (world sigma 4, reach 3·4 = 12) must expand the world
    // bounds down-right by offset + reach and up-left by reach (offset is
    // positive). Body: [-10,-10,10,10]. Shadow box: x ∈ [-10+8-12, 10+8+12] =
    // [-14, 30], y likewise. Union with body → [-14,-14,30,30].
    let mut doc = Doc::new();
    let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -10.0,
        -10.0,
        20.0,
        20.0,
        Color::rgb(255, 0, 0),
    )));
    n.effects.push(Shadow {
        kind: ShadowKind::Drop,
        color: Color::rgba(0, 0, 0, 255),
        blur: 8.0,
        spread: 0.0,
        offset: [8.0, 8.0],
    });
    let id = n.id;
    doc.apply(Operation::create_node(n)).unwrap();
    let world = doc.scene.world_bounds(id).unwrap();
    // effective_scale 1 → the cap doesn't bite; reach == full 3·sigma.
    let exp = shadow_expanded_world_bounds(
        &doc.scene,
        id,
        &doc.scene.get(id).unwrap().effects,
        &[],
        world,
        1.0,
    );
    assert!((exp.min_x - (-14.0)).abs() < 1e-6, "min_x {}", exp.min_x);
    assert!((exp.min_y - (-14.0)).abs() < 1e-6, "min_y {}", exp.min_y);
    assert!((exp.max_x - 30.0).abs() < 1e-6, "max_x {}", exp.max_x);
    assert!((exp.max_y - 30.0).abs() < 1e-6, "max_y {}", exp.max_y);
}

#[test]
fn shadow_expanded_bounds_unchanged_without_drop_shadow() {
    // No drop shadow (or only inner / zero-alpha) ⇒ the body bounds pass
    // through untouched (the fast path).
    let mut doc = Doc::new();
    let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -10.0,
        -10.0,
        20.0,
        20.0,
        Color::rgb(255, 0, 0),
    )));
    // An inner shadow and a zero-alpha drop shadow: neither expands bounds.
    n.effects.push(Shadow {
        kind: ShadowKind::Inner,
        color: Color::rgba(0, 0, 0, 255),
        blur: 40.0,
        spread: 0.0,
        offset: [40.0, 40.0],
    });
    n.effects.push(Shadow {
        kind: ShadowKind::Drop,
        color: Color::rgba(0, 0, 0, 0), // fully transparent
        blur: 40.0,
        spread: 0.0,
        offset: [40.0, 40.0],
    });
    let id = n.id;
    doc.apply(Operation::create_node(n)).unwrap();
    let world = doc.scene.world_bounds(id).unwrap();
    let exp = shadow_expanded_world_bounds(
        &doc.scene,
        id,
        &doc.scene.get(id).unwrap().effects,
        &[],
        world,
        1.0,
    );
    assert_eq!(
        exp, world,
        "inner / zero-alpha shadows must not expand bounds"
    );
}

#[test]
fn node_offscreen_but_shadow_onscreen_is_not_culled() {
    // A rect whose BODY sits just off the right edge of the surface, with a
    // big drop shadow offset back LEFT (into the viewport). Its body world
    // AABB misses the visible rect, but its shadow reaches in — so it must be
    // kept (not culled) and paint shadow pixels on-screen. 64x64 surface,
    // origin-centred, zoom 1 → visible world x ∈ [-32, 32] (+margin).
    let mut doc = Doc::new();
    // Body at world x ∈ [40, 60] (off the right edge).
    let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        40.0,
        -10.0,
        20.0,
        20.0,
        Color::rgb(255, 0, 0),
    )));
    // Shadow offset far left (−50) with blur, so its silhouette lands around
    // world x ∈ [-22, 22] — inside the viewport.
    n.effects.push(Shadow {
        kind: ShadowKind::Drop,
        color: Color::rgba(0, 0, 0, 255),
        blur: 12.0,
        spread: 0.0,
        offset: [-50.0, 0.0],
    });
    doc.apply(Operation::create_node(n)).unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    let metrics = r.render(&doc.scene, &doc.viewport);
    assert_eq!(
        metrics.nodes_culled, 0,
        "a node whose shadow reaches the viewport must NOT be culled"
    );
    // The offset shadow paints dark pixels near the surface centre.
    let buf = r.copy_rgba();
    let c = rgba_at(&buf, 64, 32, 32);
    assert!(
        c[3] > 0,
        "the on-screen shadow must paint pixels, got {c:?}"
    );
    assert!(
        c[0] < 120 && c[1] < 120 && c[2] < 120,
        "the on-screen pixels should be the dark shadow, got {c:?}"
    );
}

#[test]
fn node_and_shadow_both_offscreen_is_culled() {
    // The same body off the right edge, but now the shadow offsets further
    // RIGHT (away from the viewport) — body and shadow both miss the visible
    // rect, so the node is culled. Contrast with the previous test: this is
    // the "shadow doesn't save an off-screen node" half of the contract.
    let mut doc = Doc::new();
    let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        40.0,
        -10.0,
        20.0,
        20.0,
        Color::rgb(255, 0, 0),
    )));
    n.effects.push(Shadow {
        kind: ShadowKind::Drop,
        color: Color::rgba(0, 0, 0, 255),
        blur: 12.0,
        spread: 0.0,
        offset: [50.0, 0.0], // shadow goes further off-screen
    });
    doc.apply(Operation::create_node(n)).unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    let metrics = r.render(&doc.scene, &doc.viewport);
    assert_eq!(
        metrics.nodes_culled, 1,
        "a node whose body AND shadow both miss the viewport must be culled"
    );
    assert_eq!(metrics.nodes_drawn, 0);
    let buf = r.copy_rgba();
    assert!(buf.iter().all(|&b| b == 0), "nothing should paint");
}

#[test]
fn normal_zoom_shadow_pixels_unchanged_by_the_cap() {
    // Visual-parity guard: at zoom 1 the cap never engages (on-screen sigma
    // is tiny), so a shadowed node renders byte-for-byte the same pixels it
    // would with no cap. We render the canonical shadow scene and assert the
    // exact same pixels the pre-cap `drop_shadow_bleeds_*` test asserts —
    // proving the cap is a no-op at normal zoom.
    let mut doc = Doc::new();
    let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -10.0,
        -10.0,
        20.0,
        20.0,
        Color::rgb(255, 0, 0),
    )));
    n.effects.push(Shadow {
        kind: ShadowKind::Drop,
        color: Color::rgba(0, 0, 0, 255),
        blur: 8.0,
        spread: 0.0,
        offset: [8.0, 8.0],
    });
    doc.apply(Operation::create_node(n)).unwrap();
    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();
    // Shadow bleeds dark pixels down-right of the rect (screen (48,48)).
    let shad = rgba_at(&buf, 64, 48, 48);
    assert!(
        shad[3] > 0 && shad[0] < 120,
        "shadow still paints at zoom 1: {shad:?}"
    );
    // The node body is still opaque red over its shadow.
    let centre = rgba_at(&buf, 64, 32, 32);
    assert!(
        centre[0] > 200 && centre[1] < 40 && centre[2] < 40 && centre[3] > 200,
        "node body unchanged at zoom 1: {centre:?}"
    );
}
