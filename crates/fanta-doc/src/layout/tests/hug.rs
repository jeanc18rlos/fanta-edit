//! Hug sizing: counter-axis hug to max child + padding, and the family of
//! primary-justification no-ops on a HUG primary axis (incl. nested grids).

use super::*;

// ---------------------------------------------------------------------------
// Hug counter sizing, bottom-up
// ---------------------------------------------------------------------------

#[test]
fn hug_counter_sizes_frame_to_max_child_plus_padding() {
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        padding: [3.0, 0.0, 4.0, 0.0], // cross pads 3 + 4
        counter_sizing: AxisSizing::Hug,
        ..Default::default()
    };
    let f = t.push(frame(200.0, 999.0, al)); // counter (height) is junk; hug recomputes
    t.push(rect_child(f, 20.0, 10.0));
    t.push(rect_child(f, 20.0, 25.0)); // tallest = 25

    solve_auto_layout(&mut t, f, &mut no_measure);

    // hug counter = max(10,25) + 3 + 4 = 32.
    approx(placed_size(&t, f), [200.0, 32.0]);
}

#[test]
fn hug_counter_center_aligns_relative_to_hugged_size_not_authored() {
    // A hug-counter frame's authored counter extent (999) is junk; a Center
    // child must center within the HUGGED inner size (max child), not the junk.
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        counter_align: CounterAlign::Center,
        counter_sizing: AxisSizing::Hug,
        ..Default::default()
    };
    let f = t.push(frame(200.0, 999.0, al));
    let tall = t.push(rect_child(f, 20.0, 30.0)); // tallest → hug cross = 30
    let short = t.push(rect_child(f, 20.0, 10.0)); // centered in 30 → y = 10

    solve_auto_layout(&mut t, f, &mut no_measure);

    // Frame hugs counter to 30 (no padding); tall pins at 0, short centers at 10.
    approx(placed_size(&t, f), [200.0, 30.0]);
    approx(placed_origin(&t, tall), [0.0, 0.0]);
    approx(placed_origin(&t, short), [20.0, 10.0]);
}

#[test]
fn hug_primary_sizes_frame_to_packed_run_plus_padding() {
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        spacing: 5.0,
        padding: [0.0, 8.0, 0.0, 6.0], // primary pads left 6 + right 8
        primary_sizing: AxisSizing::Hug,
        ..Default::default()
    };
    let f = t.push(frame(999.0, 50.0, al));
    t.push(rect_child(f, 20.0, 10.0));
    t.push(rect_child(f, 30.0, 10.0));

    solve_auto_layout(&mut t, f, &mut no_measure);

    // packed = 20+30+5 = 55; hug primary = 55 + 6 + 8 = 69.
    approx(placed_size(&t, f), [69.0, 50.0]);
}

#[test]
fn min_width_holds_a_hug_frame_above_its_content() {
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        primary_sizing: AxisSizing::Hug,
        // A "min-width button": content is small, but the frame must not shrink
        // below 120 wide (max caps the tall axis, unused here).
        min_size: [Some(120.0), None],
        ..Default::default()
    };
    let f = t.push(frame(0.0, 40.0, al));
    t.push(rect_child(f, 30.0, 10.0));

    solve_auto_layout(&mut t, f, &mut no_measure);

    // Content hug would be 30, but min-width holds it at 120.
    approx(placed_size(&t, f), [120.0, 40.0]);
}

#[test]
fn max_height_caps_a_hug_frame_below_its_content() {
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Vertical,
        counter_sizing: AxisSizing::Hug, // width hugs
        primary_sizing: AxisSizing::Hug, // height hugs the stacked children
        max_size: [None, Some(50.0)],
        ..Default::default()
    };
    let f = t.push(frame(0.0, 0.0, al));
    t.push(rect_child(f, 20.0, 40.0));
    t.push(rect_child(f, 20.0, 40.0)); // content height 80 > cap 50

    solve_auto_layout(&mut t, f, &mut no_measure);

    // Content-hug height would be 80; max caps it at 50.
    let size = placed_size(&t, f);
    assert!(
        (size[1] - 50.0).abs() < 1e-4,
        "max-height should cap the hug at 50, got {}",
        size[1]
    );
}

#[test]
fn invalid_and_inverted_hug_limits_are_normalized() {
    let mut tree = VecTree::new();
    let layout = AutoLayout {
        mode: LayoutMode::Horizontal,
        primary_sizing: AxisSizing::Hug,
        counter_sizing: AxisSizing::Hug,
        min_size: [Some(80.0), Some(f64::NAN)],
        max_size: [Some(40.0), Some(f64::INFINITY)],
        ..Default::default()
    };
    let frame = tree.push(frame(0.0, 0.0, layout));
    tree.push(rect_child(frame, 20.0, 30.0));

    solve_auto_layout(&mut tree, frame, &mut no_measure);

    // Minimum wins over an inverted maximum. Non-finite limits are ignored.
    approx(placed_size(&tree, frame), [80.0, 30.0]);
}

// ---------------------------------------------------------------------------
// Primary justification is a no-op on a HUG primary axis
// ---------------------------------------------------------------------------
// On a HUG primary axis the frame has no fixed extent, so Figma leaves no free
// space to justify into: Center/End/SpaceBetween all degenerate to Start. The
// frame's authored primary extent (999 below) is junk and must NOT leak into the
// alignment offset — that was the bug these guard.

#[test]
fn hug_primary_center_collapses_to_start() {
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        spacing: 5.0,
        primary_align: PrimaryAlign::Center,
        primary_sizing: AxisSizing::Hug, // junk authored width (999) must not center
        ..Default::default()
    };
    let f = t.push(frame(999.0, 50.0, al));
    let a = t.push(rect_child(f, 20.0, 10.0));
    let b = t.push(rect_child(f, 30.0, 10.0));

    solve_auto_layout(&mut t, f, &mut no_measure);

    // Packs from the start (no free space): a@0, b@25 (20 + spacing 5).
    approx(placed_origin(&t, a), [0.0, 0.0]);
    approx(placed_origin(&t, b), [25.0, 0.0]);
    // Frame hugged to its packed run: 20 + 30 + 5 = 55.
    approx(placed_size(&t, f), [55.0, 50.0]);
}

#[test]
fn hug_primary_end_collapses_to_start() {
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        spacing: 0.0,
        primary_align: PrimaryAlign::End,
        primary_sizing: AxisSizing::Hug,
        ..Default::default()
    };
    let f = t.push(frame(999.0, 50.0, al));
    let a = t.push(rect_child(f, 20.0, 10.0));
    let b = t.push(rect_child(f, 30.0, 10.0));

    solve_auto_layout(&mut t, f, &mut no_measure);

    // End collapses to Start on a hug primary: a@0, b@20.
    approx(placed_origin(&t, a), [0.0, 0.0]);
    approx(placed_origin(&t, b), [20.0, 0.0]);
    approx(placed_size(&t, f), [50.0, 50.0]);
}

#[test]
fn hug_primary_space_between_collapses_to_packed_with_spacing() {
    // SpaceBetween on a hug primary has no free space to spread, so it behaves
    // like Start *with* the authored spacing (the frame just hugs the run).
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        spacing: 8.0,
        primary_align: PrimaryAlign::SpaceBetween,
        primary_sizing: AxisSizing::Hug,
        ..Default::default()
    };
    let f = t.push(frame(999.0, 50.0, al));
    let a = t.push(rect_child(f, 20.0, 10.0));
    let b = t.push(rect_child(f, 20.0, 10.0));
    let c = t.push(rect_child(f, 20.0, 10.0));

    solve_auto_layout(&mut t, f, &mut no_measure);

    // free = packed - content = 0, so the gap is the authored spacing (8):
    // a@0, b@28, c@56.
    approx(placed_origin(&t, a), [0.0, 0.0]);
    approx(placed_origin(&t, b), [28.0, 0.0]);
    approx(placed_origin(&t, c), [56.0, 0.0]);
    // Frame hugs: 3*20 + 2*8 = 76.
    approx(placed_size(&t, f), [76.0, 50.0]);
}

#[test]
fn hug_primary_grow_is_noop_keeps_child_base_size() {
    // FILL (grow) needs a fixed primary extent to fill into. On a hug primary
    // there is none, so Figma treats grow as a no-op: the child keeps its base
    // size and the frame hugs the packed run — it must NOT balloon to the junk
    // authored extent (999).
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        spacing: 0.0,
        primary_sizing: AxisSizing::Hug,
        ..Default::default()
    };
    let f = t.push(frame(999.0, 50.0, al));
    let fixed = t.push(rect_child(f, 30.0, 10.0));
    let mut g = rect_child(f, 40.0, 10.0);
    g.layout_child = Some(LayoutChild {
        grow: 1.0,
        absolute: false,
        align_self: None,
    });
    let g = t.push(g);

    solve_auto_layout(&mut t, f, &mut no_measure);

    // grow ignored: child keeps 40; positions are fixed@0, g@30.
    approx(placed_size(&t, g), [40.0, 10.0]);
    approx(placed_origin(&t, fixed), [0.0, 0.0]);
    approx(placed_origin(&t, g), [30.0, 0.0]);
    // Frame hugged to 30 + 40 = 70 (not 999).
    approx(placed_size(&t, f), [70.0, 50.0]);
}

#[test]
fn vertical_hug_primary_center_collapses_to_start() {
    // Same fix on the vertical primary axis (y): junk authored height (888) must
    // not leak into a Center offset.
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Vertical,
        spacing: 4.0,
        primary_align: PrimaryAlign::Center,
        primary_sizing: AxisSizing::Hug,
        ..Default::default()
    };
    let f = t.push(frame(100.0, 888.0, al));
    let a = t.push(rect_child(f, 20.0, 10.0));
    let b = t.push(rect_child(f, 20.0, 30.0));

    solve_auto_layout(&mut t, f, &mut no_measure);

    // Packs from the top: a@y=0, b@y=14 (10 + spacing 4).
    approx(placed_origin(&t, a), [0.0, 0.0]);
    approx(placed_origin(&t, b), [0.0, 14.0]);
    // Frame hugs its primary (height): 10 + 30 + 4 = 44.
    approx(placed_size(&t, f), [100.0, 44.0]);
}

#[test]
fn nested_hug_resolves_inner_before_outer_bottom_up() {
    // Outer V frame (hug counter) contains an inner H frame (hug both). The inner
    // frame must be hug-sized from ITS children before the outer reads its width.
    let mut t = VecTree::new();
    let outer_al = AutoLayout {
        mode: LayoutMode::Vertical,
        counter_sizing: AxisSizing::Hug, // counter = x (width) → hug to inner frame width
        ..Default::default()
    };
    let outer = t.push(frame(999.0, 200.0, outer_al));

    let inner_al = AutoLayout {
        mode: LayoutMode::Horizontal,
        spacing: 4.0,
        primary_sizing: AxisSizing::Hug,
        counter_sizing: AxisSizing::Hug,
        ..Default::default()
    };
    let mut inner = frame(999.0, 999.0, inner_al);
    inner.parent = Some(outer);
    let inner = t.push(inner);
    t.push(rect_child(inner, 15.0, 12.0));
    t.push(rect_child(inner, 25.0, 8.0));

    solve_auto_layout(&mut t, outer, &mut no_measure);

    // Inner hug: primary = 15+25+4 = 44; counter = max(12,8) = 12.
    approx(placed_size(&t, inner), [44.0, 12.0]);
    // Outer hug counter (width) = inner width 44 (no padding) → outer width 44.
    assert!((placed_size(&t, outer)[0] - 44.0).abs() < EPS);
}

#[test]
fn nested_h_of_v_grid_positions_leaves_at_grid_coordinates() {
    // The Action-Button matrix shape: a HORIZONTAL "Columns" frame whose children
    // are VERTICAL "Rows" columns, each holding several fixed cells. Every leaf
    // cell must land at its true (column, row) WORLD coordinate — the regression
    // guard for "the grid renders empty / cells stacked at the origin". A single
    // bottom-up solve over the root must place all leaves; nothing collapses to
    // (0,0) and the two axes compose cleanly across the two auto-layout levels.
    //
    // Geometry: Columns is Horizontal, spacing 20, no padding; each Rows column is
    // Vertical, spacing 10, no padding, 50 wide, holding three 50x30 cells. So
    // column k starts at x = k*(50+20) = k*70, and row r within a column starts at
    // y = r*(30+10) = r*40.
    let mut t = VecTree::new();
    let columns_al = AutoLayout {
        mode: LayoutMode::Horizontal,
        spacing: 20.0,
        counter_sizing: AxisSizing::Hug, // height hugs the tallest column
        ..Default::default()
    };
    let columns = t.push(frame(999.0, 999.0, columns_al));

    const NCOLS: usize = 5; // five button-state columns (default/hover/down/focus/disabled)
    const NROWS: usize = 3;
    // Each leaf, tagged with its (column, row) so we can assert its grid position.
    let mut leaves: Vec<(usize, usize, NodeId)> = Vec::new();
    let mut first_col: Option<NodeId> = None;
    for c in 0..NCOLS {
        let col_al = AutoLayout {
            mode: LayoutMode::Vertical,
            spacing: 10.0,
            counter_sizing: AxisSizing::Hug, // width hugs the widest cell (50)
            primary_sizing: AxisSizing::Hug, // height hugs the packed run of cells
            ..Default::default()
        };
        let mut col = frame(50.0, 999.0, col_al);
        col.parent = Some(columns);
        let col = t.push(col);
        first_col.get_or_insert(col);
        for r in 0..NROWS {
            leaves.push((c, r, t.push(rect_child(col, 50.0, 30.0))));
        }
    }

    // ONE solve over the grid root must lay out the whole matrix bottom-up.
    solve_auto_layout(&mut t, columns, &mut no_measure);

    // Every leaf lands at its true world (column, row) coordinate — none stacked
    // at the origin, none overlapping.
    for (c, r, id) in &leaves {
        let want = [*c as f64 * 70.0, *r as f64 * 40.0];
        approx(world_origin(&t, *id), want);
    }
    // The Hug columns sized to their content: width = widest cell (50), height =
    // packed run of three 50x30 cells with spacing 10 = 30*3 + 10*2 = 110.
    approx(placed_size(&t, first_col.unwrap()), [50.0, 110.0]);
}
