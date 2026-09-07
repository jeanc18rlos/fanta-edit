//! Cross-axis alignment: `CounterAlign` (Center/End/Stretch, per-child
//! `align_self`) and the Baseline->end-pin oracle parity mapping.

use super::*;

// ---------------------------------------------------------------------------
// CounterAlign (incl. Stretch)
// ---------------------------------------------------------------------------

#[test]
fn counter_align_center_and_end_offset_cross_axis() {
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        counter_align: CounterAlign::Center,
        ..Default::default()
    };
    let f = t.push(frame(100.0, 50.0, al));
    let a = t.push(rect_child(f, 20.0, 10.0)); // cross height 10 in 50 → center at 20
    solve_auto_layout(&mut t, f, &mut no_measure);
    approx(placed_origin(&t, a), [0.0, 20.0]);

    let mut t2 = VecTree::new();
    let al2 = AutoLayout {
        mode: LayoutMode::Horizontal,
        counter_align: CounterAlign::End,
        ..Default::default()
    };
    let f2 = t2.push(frame(100.0, 50.0, al2));
    let b = t2.push(rect_child(f2, 20.0, 10.0)); // end → 50-10 = 40
    solve_auto_layout(&mut t2, f2, &mut no_measure);
    approx(placed_origin(&t2, b), [0.0, 40.0]);
}

#[test]
fn oversized_counter_children_clamp_to_cross_start() {
    for align in [
        CounterAlign::Center,
        CounterAlign::End,
        CounterAlign::Baseline,
    ] {
        let mut tree = VecTree::new();
        let layout = AutoLayout {
            mode: LayoutMode::Horizontal,
            counter_align: align,
            ..Default::default()
        };
        let frame = tree.push(frame(100.0, 20.0, layout));
        let child = tree.push(rect_child(frame, 30.0, 40.0));

        solve_auto_layout(&mut tree, frame, &mut no_measure);

        approx(placed_origin(&tree, child), [0.0, 0.0]);
    }
}

#[test]
fn transformed_offset_child_bounds_align_to_cross_start_center_and_end() {
    for (alignment, expected_min_y) in [
        (CounterAlign::Start, 7.0),
        (CounterAlign::Center, 13.0),
        (CounterAlign::End, 19.0),
    ] {
        let mut tree = VecTree::new();
        let auto_layout = AutoLayout {
            mode: LayoutMode::Horizontal,
            padding: [7.0, 0.0, 11.0, 5.0],
            counter_align: alignment,
            ..Default::default()
        };
        let frame_id = tree.push(frame(120.0, 60.0, auto_layout));
        let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            40.0,
            70.0,
            30.0,
            10.0,
            Color::BLACK,
        )));
        child.parent = Some(frame_id);
        // Rotate the offset path 90 degrees. Its transformed AABB is 10×30 and
        // its minimum is not the transformed local-box origin/pivot.
        child.transform = Transform2D::from_components([0.0, 1.0, -1.0, 0.0, 999.0, 999.0]);
        let child_id = tree.push(child);

        solve_auto_layout(&mut tree, frame_id, &mut no_measure);

        let node = tree.get(child_id);
        let local = super::size::local_box(node);
        let local_bounds = crate::transform::Bounds::from_xywh(
            local.origin[0],
            local.origin[1],
            local.size[0],
            local.size[1],
        );
        let placed = local_bounds.transformed(&node.transform);
        assert!(
            (placed.min_x - 5.0).abs() < EPS,
            "{alignment:?}: {placed:?}"
        );
        assert!(
            (placed.min_y - expected_min_y).abs() < EPS,
            "{alignment:?}: {placed:?}"
        );
        assert!(
            (placed.max_y - (expected_min_y + 30.0)).abs() < EPS,
            "{alignment:?}: {placed:?}"
        );
    }
}

#[test]
fn counter_align_stretch_fills_counter_inner_size() {
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        padding: [6.0, 0.0, 9.0, 0.0], // top 6, bottom 9 → inner cross = 50-15 = 35
        counter_align: CounterAlign::Stretch,
        ..Default::default()
    };
    let f = t.push(frame(100.0, 50.0, al));
    let a = t.push(rect_child(f, 20.0, 10.0));
    solve_auto_layout(&mut t, f, &mut no_measure);

    // Stretched child height = inner cross = 35; positioned at counter pad-lo = 6.
    approx(placed_size(&t, a), [20.0, 35.0]);
    approx(placed_origin(&t, a), [0.0, 6.0]);
}

#[test]
fn stretched_nested_autolayout_child_reflows_at_new_width() {
    // Regression: a narrow horizontal row (Hug width, SpaceBetween) nested in a
    // wider vertical frame that Stretches it must redistribute its OWN children
    // across the stretched width — not leave them packed where the narrow Hug
    // solve put them. This is the "wide instance of a narrow component master"
    // bug (a 720-wide composer instance of a 400-wide master left its send button
    // bunched at the left with dead space on the right).
    let mut t = VecTree::new();

    // Outer: vertical, fixed 400 wide, stretches its children on the cross axis.
    let outer_al = AutoLayout {
        mode: LayoutMode::Vertical,
        counter_align: CounterAlign::Stretch,
        primary_sizing: AxisSizing::Fixed,
        counter_sizing: AxisSizing::Fixed,
        ..Default::default()
    };
    let outer = t.push(frame(400.0, 80.0, outer_al));

    // Row: horizontal, Hug width, SpaceBetween. It would hug to 150 on its own,
    // but the outer frame stretches it to 400.
    let row_al = AutoLayout {
        mode: LayoutMode::Horizontal,
        primary_align: PrimaryAlign::SpaceBetween,
        primary_sizing: AxisSizing::Hug,
        counter_sizing: AxisSizing::Hug,
        ..Default::default()
    };
    let mut row_node = frame(150.0, 32.0, row_al);
    row_node.parent = Some(outer);
    let row = t.push(row_node);

    // Two children in the row: left 100, right 50.
    let left = t.push(rect_child(row, 100.0, 32.0));
    let right = t.push(rect_child(row, 50.0, 32.0));

    solve_auto_layout(&mut t, outer, &mut no_measure);

    // The row was stretched to the outer inner width (400)...
    approx(placed_size(&t, row), [400.0, 32.0]);
    // ...and SpaceBetween redistributed its children across that width: left
    // pinned at x=0, right pushed to 400-50=350 (in the row's local space).
    approx(placed_origin(&t, left), [0.0, 0.0]);
    approx(placed_origin(&t, right), [350.0, 0.0]);
}

#[test]
fn per_child_align_self_overrides_frame_counter_align() {
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        counter_align: CounterAlign::Start,
        ..Default::default()
    };
    let f = t.push(frame(100.0, 50.0, al));
    let a = t.push(rect_child(f, 20.0, 10.0)); // inherits Start → y 0
    let mut b_node = rect_child(f, 20.0, 10.0);
    b_node.layout_child = Some(LayoutChild {
        grow: 0.0,
        absolute: false,
        align_self: Some(CounterAlign::End), // overrides → y 40
    });
    let b = t.push(b_node);

    solve_auto_layout(&mut t, f, &mut no_measure);
    approx(placed_origin(&t, a), [0.0, 0.0]);
    approx(placed_origin(&t, b), [20.0, 40.0]);
}

// ---------------------------------------------------------------------------
// CounterAlign::Baseline → end-pinned (oracle parity)
// ---------------------------------------------------------------------------
// The OpenPencil/Yoga oracle has no font-metrics pipeline, so its layout engine
// normalizes BASELINE counter-alignment to END (`normalizeAlignItems` in
// pen-core/src/layout/engine.ts). Bottom-pinning a "big number + small unit" row
// is visually indistinguishable from true baseline alignment, and the old
// Start-pin was wrong. These lock fanta to the same mapping so a `.fig` importing
// `stackCounterAlignItems: "BASELINE"` lays out identically.

#[test]
fn counter_align_baseline_bottom_pins_like_end() {
    // Horizontal row, counter axis = height. A short child must bottom-pin to the
    // cross-end (same as CounterAlign::End), NOT top-align at y=0.
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        counter_align: CounterAlign::Baseline,
        ..Default::default()
    };
    let f = t.push(frame(100.0, 50.0, al));
    let big = t.push(rect_child(f, 20.0, 40.0)); // tall: end → 50-40 = 10
    let small = t.push(rect_child(f, 20.0, 10.0)); // short: end → 50-10 = 40

    solve_auto_layout(&mut t, f, &mut no_measure);

    // Both bottom-pinned: bottoms at y=50. big@10, small@40.
    approx(placed_origin(&t, big), [0.0, 10.0]);
    approx(placed_origin(&t, small), [20.0, 40.0]);
}

#[test]
fn counter_align_baseline_matches_end_vertical_with_padding() {
    // Vertical stack, counter axis = width. Baseline must equal End here too, and
    // respect the counter padding (left/right) start offset.
    let mut t_base = VecTree::new();
    let al_base = AutoLayout {
        mode: LayoutMode::Vertical,
        padding: [0.0, 5.0, 0.0, 7.0], // counter pads: left 7, right 5 → inner = 100-12 = 88
        counter_align: CounterAlign::Baseline,
        ..Default::default()
    };
    let fb = t_base.push(frame(100.0, 200.0, al_base));
    let cb = t_base.push(rect_child(fb, 30.0, 20.0));
    solve_auto_layout(&mut t_base, fb, &mut no_measure);

    let mut t_end = VecTree::new();
    let al_end = AutoLayout {
        mode: LayoutMode::Vertical,
        padding: [0.0, 5.0, 0.0, 7.0],
        counter_align: CounterAlign::End,
        ..Default::default()
    };
    let fe = t_end.push(frame(100.0, 200.0, al_end));
    let ce = t_end.push(rect_child(fe, 30.0, 20.0));
    solve_auto_layout(&mut t_end, fe, &mut no_measure);

    // Baseline lays the child out byte-for-byte like End: x = pad_lo(7) + (88-30) = 65.
    approx(placed_origin(&t_base, cb), [65.0, 0.0]);
    approx(placed_origin(&t_base, cb), placed_origin(&t_end, ce));
}

#[test]
fn per_child_align_self_baseline_overrides_to_end() {
    // A per-child `align_self: Baseline` override must end-pin that child while a
    // sibling inheriting the frame's Start stays at the cross-start.
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        counter_align: CounterAlign::Start,
        ..Default::default()
    };
    let f = t.push(frame(100.0, 50.0, al));
    let a = t.push(rect_child(f, 20.0, 10.0)); // inherits Start → y 0
    let mut b_node = rect_child(f, 20.0, 10.0);
    b_node.layout_child = Some(LayoutChild {
        grow: 0.0,
        absolute: false,
        align_self: Some(CounterAlign::Baseline), // overrides → end → y 40
    });
    let b = t.push(b_node);

    solve_auto_layout(&mut t, f, &mut no_measure);
    approx(placed_origin(&t, a), [0.0, 0.0]);
    approx(placed_origin(&t, b), [20.0, 40.0]);
}

#[test]
fn wrap_counter_align_baseline_pins_to_line_end() {
    // In a wrapping frame, Baseline must pin a child to the END of its own line's
    // thickness (same as End), not the line's start. Two children of differing
    // heights on one row: each bottom-aligns within the row's thickness.
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        spacing: 10.0,
        counter_spacing: 0.0,
        wrap: true,
        counter_align: CounterAlign::Baseline,
        ..Default::default()
    };
    let f = t.push(frame(200.0, 100.0, al)); // wide enough: both on row 1
    let tall = t.push(rect_child(f, 40.0, 30.0)); // line thickness = 30
    let short = t.push(rect_child(f, 40.0, 10.0)); // end → 30-10 = 20

    solve_auto_layout(&mut t, f, &mut no_measure);

    // Row 1, thickness 30. tall pins at line-end → y 0; short → y 20.
    approx(placed_origin(&t, tall), [0.0, 0.0]);
    approx(placed_origin(&t, short), [50.0, 20.0]);
}
