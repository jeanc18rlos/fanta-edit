//! Mask tests (alpha + luminance): a mask child clips its following siblings
//! to the mask shape, resets at the next mask sibling, and an icon-swap
//! override changes the instance cache key.
use super::*;
use fanta_doc::{Doc, Operation, VectorNode};

// -----------------------------------------------------------------------
// Masks (alpha + luminance)
//
// Coordinate map (100×100 surface, origin-centred viewport, zoom 1,
// display_scale 1): world→screen adds 50. A group at the world origin holds
// children whose world rects therefore land at screen `+50`.
// -----------------------------------------------------------------------

/// Append a child rect to `parent` at the given world `rect` (`[x,y,w,h]`)
/// with `color`, flagged as a mask (`mask`) or not, at z-index `z` so the
/// children render in the order they are added. Returns the child id.
fn add_child(
    doc: &mut Doc,
    parent: NodeId,
    z: f64,
    rect: [f64; 4],
    color: Color,
    mask: Option<MaskType>,
) -> NodeId {
    let [x, y, w, h] = rect;
    let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(x, y, w, h, color)));
    n.parent = Some(parent);
    n.index = fanta_doc::IndexKey::from_raw(z);
    if let Some(mt) = mask {
        n.is_mask = true;
        n.mask_type = mt;
    }
    let id = n.id;
    doc.apply(Operation::create_node(n)).unwrap();
    id
}

/// A group at the world origin to hold mask test children.
fn mask_group(doc: &mut Doc) -> NodeId {
    let g = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode::default()));
    let id = g.id;
    doc.apply(Operation::create_node(g)).unwrap();
    id
}

#[test]
fn alpha_mask_clips_its_following_sibling_to_the_mask_shape() {
    // z0: ALPHA mask, white, world [0,0,20,20] → screen [50,70]×[50,70].
    // z1: masked red rect, world [0,0,40,40] → screen [50,90]×[50,90].
    // The red rect must show ONLY where the mask covers it.
    let mut doc = Doc::new();
    let g = mask_group(&mut doc);
    add_child(
        &mut doc,
        g,
        1.0,
        [0.0, 0.0, 20.0, 20.0],
        Color::WHITE,
        Some(MaskType::Alpha),
    );
    add_child(
        &mut doc,
        g,
        2.0,
        [0.0, 0.0, 40.0, 40.0],
        Color::rgb(255, 0, 0),
        None,
    );

    let mut r = RasterRenderer::new(100, 100).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    // Inside the mask shape AND the red rect → red shows.
    let inside = rgba_at(&buf, 100, 60, 60);
    assert!(
        inside[0] > 200 && inside[1] < 40 && inside[2] < 40 && inside[3] > 200,
        "pixel inside the mask should be red, got {inside:?}"
    );
    // Inside the red rect but OUTSIDE the mask shape → clipped to transparent.
    let outside = rgba_at(&buf, 100, 80, 80);
    assert_eq!(
        outside[3], 0,
        "pixel outside the mask shape must be clipped away, got {outside:?}"
    );
}

#[test]
fn mask_node_itself_is_not_painted_as_normal_content() {
    // A lone ALPHA mask with NO following siblings paints nothing — the mask
    // shape only drives the mask, it is not drawn itself.
    let mut doc = Doc::new();
    let g = mask_group(&mut doc);
    add_child(
        &mut doc,
        g,
        1.0,
        [0.0, 0.0, 20.0, 20.0],
        Color::WHITE,
        Some(MaskType::Alpha),
    );

    let mut r = RasterRenderer::new(100, 100).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();
    assert!(
        buf.iter().all(|&b| b == 0),
        "a mask with no following siblings should paint nothing"
    );
}

#[test]
fn mask_resets_at_the_next_mask_sibling() {
    // z0: ALPHA mask A, white, world [0,0,20,20]  → screen [50,70]².
    // z1: red rect (masked by A), world [0,0,40,40].
    // z2: ALPHA mask B, white, world [40,40,20,20] → screen [90,110]² (off the
    //     surface bottom-right, but its bbox is [90,110] so screen pixel
    //     (95,95) is inside it).
    // z3: blue rect (masked by B), world [40,40,40,40] → screen [90,130]².
    // The red run is masked by A (not B); the blue run is masked by B (not A).
    // Proof: a blue pixel at screen (95,95) shows (inside B + blue), while a
    // red pixel at (80,80) is clipped (outside A) — they don't bleed into each
    // other's run.
    let mut doc = Doc::new();
    let g = mask_group(&mut doc);
    add_child(
        &mut doc,
        g,
        1.0,
        [0.0, 0.0, 20.0, 20.0],
        Color::WHITE,
        Some(MaskType::Alpha),
    );
    add_child(
        &mut doc,
        g,
        2.0,
        [0.0, 0.0, 40.0, 40.0],
        Color::rgb(255, 0, 0),
        None,
    );
    add_child(
        &mut doc,
        g,
        3.0,
        [40.0, 40.0, 20.0, 20.0],
        Color::WHITE,
        Some(MaskType::Alpha),
    );
    add_child(
        &mut doc,
        g,
        4.0,
        [40.0, 40.0, 40.0, 40.0],
        Color::rgb(0, 0, 255),
        None,
    );

    let mut r = RasterRenderer::new(100, 100).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    // Red run: inside mask A → red; outside mask A but inside red rect → clip.
    let red_in = rgba_at(&buf, 100, 60, 60);
    assert!(
        red_in[0] > 200 && red_in[2] < 40,
        "red run inside mask A should be red, got {red_in:?}"
    );
    let red_clipped = rgba_at(&buf, 100, 80, 80);
    // (80,80) is inside the red rect's extent and inside mask B's run's blue
    // rect too — but mask A does not cover it and mask B's blue rect starts at
    // screen 90, so this pixel must be the RED rect clipped away by A → nothing
    // (B's blue starts at 90). Assert it is NOT blue (the B run didn't leak
    // into A's run) and not opaque red.
    assert!(
        red_clipped[3] == 0,
        "pixel at (80,80) should be clipped (outside A, before B's blue), got {red_clipped:?}"
    );

    // Blue run: inside mask B (screen [90,100] visible corner) → blue shows.
    let blue_in = rgba_at(&buf, 100, 95, 95);
    assert!(
        blue_in[2] > 200 && blue_in[0] < 40,
        "blue run inside mask B should be blue, got {blue_in:?}"
    );
}

#[test]
fn non_mask_sibling_before_a_mask_is_unaffected() {
    // z0: a plain (non-mask) green rect, world [-40,-40,30,30] → screen
    //     [10,40]×[10,40]; nothing masks it (no preceding mask), so it paints
    //     in full. z1: ALPHA mask, z2: red masked rect (a separate run).
    let mut doc = Doc::new();
    let g = mask_group(&mut doc);
    add_child(
        &mut doc,
        g,
        1.0,
        [-40.0, -40.0, 30.0, 30.0],
        Color::rgb(0, 200, 0),
        None,
    );
    add_child(
        &mut doc,
        g,
        2.0,
        [0.0, 0.0, 20.0, 20.0],
        Color::WHITE,
        Some(MaskType::Alpha),
    );
    add_child(
        &mut doc,
        g,
        3.0,
        [0.0, 0.0, 40.0, 40.0],
        Color::rgb(255, 0, 0),
        None,
    );

    let mut r = RasterRenderer::new(100, 100).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    // The green rect (screen ~[10,40]) is fully painted — unaffected by the
    // later mask. Sample its centre at screen (25,25).
    let green = rgba_at(&buf, 100, 25, 25);
    assert!(
        green[1] > 150 && green[0] < 60 && green[3] > 200,
        "non-mask sibling before the mask must paint in full, got {green:?}"
    );
    // And the masked red run still clips outside the mask.
    let clipped = rgba_at(&buf, 100, 85, 85);
    assert_eq!(
        clipped[3], 0,
        "masked red must clip outside the mask, got {clipped:?}"
    );
}

#[test]
fn luminance_mask_reveals_under_bright_and_hides_under_dark() {
    // Two side-by-side mask shapes drive a LUMINANCE mask differently:
    // bright (white) reveals, dark (near-black) hides. We use ONE luminance
    // mask whose left half is bright and right half is dark by stacking two
    // mask rects? Simpler: one white luminance mask reveals the red sibling
    // (like alpha), and a separate run with a near-black luminance mask hides
    // its sibling. Compare the two.
    //
    // Run 1 (bright luminance mask): white mask world [0,0,20,20], red sibling
    // world [0,0,20,20] → revealed (luma(white) ≈ 1).
    let mut bright = Doc::new();
    let bg = mask_group(&mut bright);
    add_child(
        &mut bright,
        bg,
        1.0,
        [0.0, 0.0, 20.0, 20.0],
        Color::WHITE,
        Some(MaskType::Luminance),
    );
    add_child(
        &mut bright,
        bg,
        2.0,
        [0.0, 0.0, 20.0, 20.0],
        Color::rgb(255, 0, 0),
        None,
    );
    let mut rb = RasterRenderer::new(100, 100).unwrap();
    rb.render(&bright.scene, &bright.viewport);
    let buf_b = rb.copy_rgba();
    let revealed = rgba_at(&buf_b, 100, 60, 60);
    assert!(
        revealed[0] > 150 && revealed[3] > 150,
        "bright luminance mask should reveal the red sibling, got {revealed:?}"
    );

    // Run 2 (dark luminance mask): near-black mask, same red sibling → hidden
    // (luma(black) ≈ 0 ⇒ DstIn keeps almost nothing).
    let mut dark = Doc::new();
    let dg = mask_group(&mut dark);
    add_child(
        &mut dark,
        dg,
        1.0,
        [0.0, 0.0, 20.0, 20.0],
        Color::rgb(2, 2, 2),
        Some(MaskType::Luminance),
    );
    add_child(
        &mut dark,
        dg,
        2.0,
        [0.0, 0.0, 20.0, 20.0],
        Color::rgb(255, 0, 0),
        None,
    );
    let mut rd = RasterRenderer::new(100, 100).unwrap();
    rd.render(&dark.scene, &dark.viewport);
    let buf_d = rd.copy_rgba();
    let hidden = rgba_at(&buf_d, 100, 60, 60);
    assert!(
        hidden[3] < 40,
        "dark luminance mask should hide the red sibling, got {hidden:?}"
    );
}

#[test]
fn alpha_mask_differs_from_luminance_for_a_dark_mask_shape() {
    // The SAME near-black mask shape masks a red sibling: as an ALPHA mask it
    // REVEALS (alpha of an opaque dark rect is 1), as a LUMINANCE mask it
    // HIDES (luma ≈ 0). This proves the two mask types take different paths.
    let build = |mt: MaskType| -> Vec<u8> {
        let mut doc = Doc::new();
        let g = mask_group(&mut doc);
        add_child(
            &mut doc,
            g,
            1.0,
            [0.0, 0.0, 20.0, 20.0],
            Color::rgb(3, 3, 3),
            Some(mt),
        );
        add_child(
            &mut doc,
            g,
            2.0,
            [0.0, 0.0, 20.0, 20.0],
            Color::rgb(255, 0, 0),
            None,
        );
        let mut r = RasterRenderer::new(100, 100).unwrap();
        r.render(&doc.scene, &doc.viewport);
        r.copy_rgba()
    };
    let alpha = rgba_at(&build(MaskType::Alpha), 100, 60, 60);
    let luma = rgba_at(&build(MaskType::Luminance), 100, 60, 60);
    assert!(
        alpha[3] > 150,
        "alpha mask of an opaque (dark) shape reveals, got {alpha:?}"
    );
    assert!(
        luma[3] < 40,
        "luminance mask of a dark shape hides, got {luma:?}"
    );
}

#[test]
fn two_consecutive_masks_clip_content_to_their_intersection() {
    // Figma intersects consecutive masks: content under TWO stacked masks shows
    // only where BOTH masks cover.
    //   z0: ALPHA mask A, white, world [0,0,40,40]   → screen [50,90]².
    //   z1: ALPHA mask B, white, world [20,20,40,40] → screen [70,110]².
    //   z2: red content, world [0,0,60,60]           → screen [50,110]².
    // Intersection(A,B) in screen space = [70,90]². Content shows only there.
    let mut doc = Doc::new();
    let g = mask_group(&mut doc);
    add_child(
        &mut doc,
        g,
        1.0,
        [0.0, 0.0, 40.0, 40.0],
        Color::WHITE,
        Some(MaskType::Alpha),
    );
    add_child(
        &mut doc,
        g,
        2.0,
        [20.0, 20.0, 40.0, 40.0],
        Color::WHITE,
        Some(MaskType::Alpha),
    );
    add_child(
        &mut doc,
        g,
        3.0,
        [0.0, 0.0, 60.0, 60.0],
        Color::rgb(255, 0, 0),
        None,
    );

    let mut r = RasterRenderer::new(100, 100).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    // Inside BOTH masks (the intersection [70,90]²) → red shows.
    let inside_both = rgba_at(&buf, 100, 78, 78);
    assert!(
        inside_both[0] > 200 && inside_both[1] < 40 && inside_both[2] < 40 && inside_both[3] > 200,
        "pixel inside BOTH masks should be red, got {inside_both:?}"
    );
    // Inside A but OUTSIDE B (B starts at screen 70) → clipped by the
    // intersection. Pre-fix this pixel would still be red (A alone masked it).
    let in_a_not_b = rgba_at(&buf, 100, 60, 60);
    assert_eq!(
        in_a_not_b[3], 0,
        "pixel inside A but outside B must be clipped by the intersection, got {in_a_not_b:?}"
    );
    // Inside B but OUTSIDE A (A ends at screen 90) → clipped by the
    // intersection. Pre-fix B would start a fresh run and reveal this pixel.
    let in_b_not_a = rgba_at(&buf, 100, 95, 95);
    assert_eq!(
        in_b_not_a[3], 0,
        "pixel inside B but outside A must be clipped by the intersection, got {in_b_not_a:?}"
    );
}

#[test]
fn three_consecutive_masks_intersect() {
    // Extend the intersection to THREE masks to prove the compounding is not
    // hard-coded to two — and that ALL three masks (not just the last) clip.
    //   z0: mask A, world [0,0,40,40]   → screen [50,90]².
    //   z1: mask B, world [10,10,40,40] → screen [60,100]².
    //   z2: mask C, world [20,20,40,40] → screen [70,110]².
    //   z3: red content, world [0,0,60,60] → screen [50,110]².
    // Intersection(A,B,C) in screen space = [70,90]².
    let mut doc = Doc::new();
    let g = mask_group(&mut doc);
    for (z, rect) in [
        (1.0, [0.0, 0.0, 40.0, 40.0]),
        (2.0, [10.0, 10.0, 40.0, 40.0]),
        (3.0, [20.0, 20.0, 40.0, 40.0]),
    ] {
        add_child(&mut doc, g, z, rect, Color::WHITE, Some(MaskType::Alpha));
    }
    add_child(
        &mut doc,
        g,
        4.0,
        [0.0, 0.0, 60.0, 60.0],
        Color::rgb(255, 0, 0),
        None,
    );

    let mut r = RasterRenderer::new(100, 100).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    // Inside all three (intersection [70,90]²) → red.
    let inside_all = rgba_at(&buf, 100, 78, 78);
    assert!(
        inside_all[0] > 200 && inside_all[2] < 40 && inside_all[3] > 200,
        "pixel inside all three masks should be red, got {inside_all:?}"
    );
    // Inside the LAST mask C ([70,110]²) AND B, but OUTSIDE the FIRST mask A
    // (A ends at screen 90): (95,95). The true intersection clips it — and this
    // proves masks earlier in the run still clip, not just the last one. (Pre-
    // fix, only C masked the run, so C would have revealed red here.)
    let outside_a = rgba_at(&buf, 100, 95, 95);
    assert_eq!(
        outside_a[3], 0,
        "pixel outside the FIRST mask must still be clipped by the intersection, got {outside_a:?}"
    );
    // Inside A and C but OUTSIDE the MIDDLE mask B ([60,100]²): a point with
    // x in A∩C but y above B. B's top is screen 60; (78,55) has y=55<60 → out of
    // B, while x=78 ∈ A∩C and y=55 ∈ A([50,90]). Intersection clips it.
    let outside_b = rgba_at(&buf, 100, 78, 55);
    assert_eq!(
        outside_b[3], 0,
        "pixel outside the MIDDLE mask must still be clipped by the intersection, got {outside_b:?}"
    );
}

#[test]
fn single_mask_intersection_path_is_unchanged() {
    // A run of exactly one mask must behave identically to the single-mask path:
    // content shows where the mask covers, clipped elsewhere. (Guards the
    // intersection refactor against regressing the common single-mask case.)
    let mut doc = Doc::new();
    let g = mask_group(&mut doc);
    add_child(
        &mut doc,
        g,
        1.0,
        [0.0, 0.0, 20.0, 20.0],
        Color::WHITE,
        Some(MaskType::Alpha),
    );
    add_child(
        &mut doc,
        g,
        2.0,
        [0.0, 0.0, 40.0, 40.0],
        Color::rgb(255, 0, 0),
        None,
    );

    let mut r = RasterRenderer::new(100, 100).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    let inside = rgba_at(&buf, 100, 60, 60);
    assert!(
        inside[0] > 200 && inside[1] < 40 && inside[2] < 40 && inside[3] > 200,
        "single-mask: pixel inside the mask should be red, got {inside:?}"
    );
    let outside = rgba_at(&buf, 100, 80, 80);
    assert_eq!(
        outside[3], 0,
        "single-mask: pixel outside the mask must be clipped, got {outside:?}"
    );
}

#[test]
fn a_later_run_after_consecutive_masks_is_independent() {
    // A consecutive-mask run (A∩B) masks red; a LATER single mask C starts a
    // fresh, independent run masking blue. The A∩B intersection must NOT bleed
    // into C's run, and C must NOT clip back into A∩B's run — i.e. the
    // consecutive-mask grouping terminates exactly at the next mask.
    //   z0: mask A, world [0,0,40,40]   → screen [50,90]².
    //   z1: mask B, world [20,20,40,40] → screen [70,110]². A∩B = screen [70,90]².
    //   z2: red content,  world [0,0,60,60].
    //   z3: mask C, world [0,0,20,20]   → screen [50,70]² (own run).
    //   z4: blue content, world [0,0,20,20] → masked by C alone.
    let mut doc = Doc::new();
    let g = mask_group(&mut doc);
    add_child(
        &mut doc,
        g,
        1.0,
        [0.0, 0.0, 40.0, 40.0],
        Color::WHITE,
        Some(MaskType::Alpha),
    );
    add_child(
        &mut doc,
        g,
        2.0,
        [20.0, 20.0, 40.0, 40.0],
        Color::WHITE,
        Some(MaskType::Alpha),
    );
    add_child(
        &mut doc,
        g,
        3.0,
        [0.0, 0.0, 60.0, 60.0],
        Color::rgb(255, 0, 0),
        None,
    );
    add_child(
        &mut doc,
        g,
        4.0,
        [0.0, 0.0, 20.0, 20.0],
        Color::WHITE,
        Some(MaskType::Alpha),
    );
    add_child(
        &mut doc,
        g,
        5.0,
        [0.0, 0.0, 20.0, 20.0],
        Color::rgb(0, 0, 255),
        None,
    );

    let mut r = RasterRenderer::new(100, 100).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    // Red run shows only in A∩B = screen [70,90]². Sample (78,78): inside A∩B,
    // outside C's blue rect ([50,70]²) → pure red from the first run.
    let red_in = rgba_at(&buf, 100, 78, 78);
    assert!(
        red_in[0] > 200 && red_in[2] < 40 && red_in[3] > 200,
        "red run inside A∩B should be red, got {red_in:?}"
    );
    // (85,55): inside A ([50,90]), OUTSIDE B (y=55 ∉ [70,110]), and outside both
    // C ([50,70]²) and blue → red is clipped by the intersection, nothing else
    // covers it. Pre-fix B would have started a fresh run and left red here.
    let red_clipped = rgba_at(&buf, 100, 85, 55);
    assert_eq!(
        red_clipped[3], 0,
        "red inside A but outside B must be clipped by A∩B, got {red_clipped:?}"
    );
    // Blue run is masked by C alone (screen [50,70]²). At (60,60) the red above
    // is clipped (outside B) but C's blue shows — proving C's run is its own,
    // independent of A∩B (which would otherwise also have clipped this pixel).
    let blue_in = rgba_at(&buf, 100, 60, 60);
    assert!(
        blue_in[2] > 200 && blue_in[0] < 40 && blue_in[3] > 200,
        "blue run inside C should be blue (C's run is independent of A∩B), got {blue_in:?}"
    );
}

// -----------------------------------------------------------------------
// Instance memo key: an icon-swap override must change the override-hash so
// two buttons differing ONLY by their icon swap don't collide in the
// per-frame `InstanceCache` (the Action Bar Edit-vs-Copy-vs-Delete bug).
// -----------------------------------------------------------------------
#[test]
fn icon_swap_override_changes_instance_cache_key() {
    use fanta_doc::node::{Override, OverrideValue};
    use fanta_doc::{BoundProp, ComponentId, InstanceNode};
    use smallvec::smallvec;

    let comp = ComponentId::new();
    let icon_a = ComponentId::new();
    let icon_b = ComponentId::new();
    let icon_path: fanta_doc::node::OverridePath = smallvec![NodeId::new()];

    let base = InstanceNode {
        component: comp,
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [24.0, 24.0],
    };
    // Two instances of the SAME master, differing only by which component the
    // nested icon instance is swapped to (Copy vs Delete).
    let swap = |to: ComponentId| {
        let mut i = base.clone();
        i.overrides = vec![Override {
            target_path: icon_path.clone(),
            target_prop: BoundProp::Visible, // unused for a swap value
            value: OverrideValue::SwapInstance { component: to },
        }];
        i
    };
    let inst_copy = swap(icon_a);
    let inst_delete = swap(icon_b);

    // The override-hash (which feeds `InstanceCacheKey.override_hash`) must
    // differ between the two swaps, and both must differ from the no-swap
    // default — otherwise the memo serves one cached expansion for all three.
    let h_default = hash_overrides(&base);
    let h_copy = hash_overrides(&inst_copy);
    let h_delete = hash_overrides(&inst_delete);
    assert_ne!(
        h_copy, h_delete,
        "Copy and Delete icon swaps must hash differently"
    );
    assert_ne!(
        h_copy, h_default,
        "an icon swap must differ from the no-swap default"
    );
    assert_ne!(
        h_delete, h_default,
        "an icon swap must differ from the no-swap default"
    );
}
