//! `grow`/FILL distribution: equal-share assignment of leftover primary
//! space (assign-not-add semantics), collapse, and per-line wrap behaviour.

use super::*;

// ---------------------------------------------------------------------------
// grow / FILL distribution
// ---------------------------------------------------------------------------

#[test]
fn grow_children_share_leftover_primary_space_equally() {
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        spacing: 10.0,
        ..Default::default()
    };
    let f = t.push(frame(200.0, 50.0, al));
    // fixed 30 + grow + grow, spacing 10*2 = 20.
    let fixed = t.push(rect_child(f, 30.0, 10.0));
    let mut g1 = rect_child(f, 10.0, 10.0);
    g1.layout_child = Some(LayoutChild {
        grow: 1.0,
        absolute: false,
        align_self: None,
    });
    let g1 = t.push(g1);
    let mut g2 = rect_child(f, 10.0, 10.0);
    g2.layout_child = Some(LayoutChild {
        grow: 1.0,
        absolute: false,
        align_self: None,
    });
    let g2 = t.push(g2);

    solve_auto_layout(&mut t, f, &mut no_measure);

    // used = 30+10+10 = 50; gaps = 20; leftover = 200-50-20 = 130; each grow += 65.
    approx(placed_size(&t, g1), [75.0, 10.0]);
    approx(placed_size(&t, g2), [75.0, 10.0]);
    // Positions: fixed@0, g1@30+10=40, g2@40+75+10=125.
    approx(placed_origin(&t, fixed), [0.0, 0.0]);
    approx(placed_origin(&t, g1), [40.0, 0.0]);
    approx(placed_origin(&t, g2), [125.0, 0.0]);
}

#[test]
fn grow_children_with_unequal_bases_end_up_the_same_size() {
    // The Yoga/OpenPencil semantics fanta must match: every FILL (grow) child is
    // ASSIGNED the same final primary extent — `(inner - fixed - gaps) / fillCount`
    // — regardless of its authored base. Two grow children with very different
    // bases (10 vs 90) must come out IDENTICAL, not `base + share` (which would
    // preserve a stale 80px difference). This is the bug the assign-not-add fix
    // closes; the equal-base test above can't see it.
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        spacing: 0.0,
        ..Default::default()
    };
    let f = t.push(frame(200.0, 50.0, al));
    let mut g1 = rect_child(f, 10.0, 10.0); // tiny base
    g1.layout_child = Some(LayoutChild {
        grow: 1.0,
        absolute: false,
        align_self: None,
    });
    let g1 = t.push(g1);
    let mut g2 = rect_child(f, 90.0, 10.0); // fat base
    g2.layout_child = Some(LayoutChild {
        grow: 1.0,
        absolute: false,
        align_self: None,
    });
    let g2 = t.push(g2);

    solve_auto_layout(&mut t, f, &mut no_measure);

    // No fixed children, no gaps: each grow child = 200 / 2 = 100 (NOT 60/140).
    approx(placed_size(&t, g1), [100.0, 10.0]);
    approx(placed_size(&t, g2), [100.0, 10.0]);
    // Packed edge-to-edge from the start: g1@0, g2@100.
    approx(placed_origin(&t, g1), [0.0, 0.0]);
    approx(placed_origin(&t, g2), [100.0, 0.0]);
}

#[test]
fn grow_share_counts_only_fixed_children_against_the_frame() {
    // The leftover a FILL child draws from is `inner - FIXED - gaps`, where FIXED
    // excludes the grow children's own (irrelevant) bases. A 30px fixed child +
    // one grow child (junk base 999) in a 200 frame, spacing 10: grow gets
    // 200 - 30 - 10 = 160 — its 999 base must NOT shrink the pool to a negative.
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        spacing: 10.0,
        ..Default::default()
    };
    let f = t.push(frame(200.0, 50.0, al));
    let fixed = t.push(rect_child(f, 30.0, 10.0));
    let mut g = rect_child(f, 999.0, 10.0); // junk base — must be discarded
    g.layout_child = Some(LayoutChild {
        grow: 1.0,
        absolute: false,
        align_self: None,
    });
    let g = t.push(g);

    solve_auto_layout(&mut t, f, &mut no_measure);

    approx(placed_size(&t, g), [160.0, 10.0]);
    approx(placed_origin(&t, fixed), [0.0, 0.0]);
    approx(placed_origin(&t, g), [40.0, 0.0]); // 30 + spacing 10
}

#[test]
fn grow_children_collapse_to_zero_when_fixed_exceeds_frame() {
    // When the fixed children already overflow the frame there is no slack: Yoga/
    // OpenPencil clamp `remainingMain` to 0, so a FILL child is assigned 0 width
    // (it collapses) rather than keeping its authored base.
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        spacing: 0.0,
        ..Default::default()
    };
    let f = t.push(frame(100.0, 50.0, al));
    let fixed = t.push(rect_child(f, 120.0, 10.0)); // already overflows 100
    let mut g = rect_child(f, 40.0, 10.0);
    g.layout_child = Some(LayoutChild {
        grow: 1.0,
        absolute: false,
        align_self: None,
    });
    let g = t.push(g);

    solve_auto_layout(&mut t, f, &mut no_measure);

    // No slack → grow collapses to 0; it sits flush after the (overflowing) fixed.
    approx(placed_size(&t, g), [0.0, 10.0]);
    approx(placed_origin(&t, fixed), [0.0, 0.0]);
    approx(placed_origin(&t, g), [120.0, 0.0]);
}

#[test]
fn wrap_grow_children_with_unequal_bases_match_per_line() {
    // In a wrapping frame, FILL children are still ASSIGNED an equal share of
    // their own line's leftover (after that line's fixed children), discarding
    // their bases. One row, frame inner 200, a 40 fixed + two grow (bases 10/90),
    // spacing 0: each grow = (200 - 40) / 2 = 80 — identical despite the bases.
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        spacing: 0.0,
        counter_spacing: 0.0,
        wrap: true,
        ..Default::default()
    };
    let f = t.push(frame(200.0, 100.0, al)); // wide enough: all on row 1
    let fixed = t.push(rect_child(f, 40.0, 10.0));
    let mut g1 = rect_child(f, 10.0, 10.0);
    g1.layout_child = Some(LayoutChild {
        grow: 1.0,
        absolute: false,
        align_self: None,
    });
    let g1 = t.push(g1);
    let mut g2 = rect_child(f, 90.0, 10.0);
    g2.layout_child = Some(LayoutChild {
        grow: 1.0,
        absolute: false,
        align_self: None,
    });
    let g2 = t.push(g2);

    solve_auto_layout(&mut t, f, &mut no_measure);

    approx(placed_size(&t, g1), [80.0, 10.0]);
    approx(placed_size(&t, g2), [80.0, 10.0]);
    // fixed@0, g1@40, g2@120 — one tight row, all assigned-equal grows.
    approx(placed_origin(&t, fixed), [0.0, 0.0]);
    approx(placed_origin(&t, g1), [40.0, 0.0]);
    approx(placed_origin(&t, g2), [120.0, 0.0]);
}
