//! Primary-axis packing: horizontal/vertical stacking (spacing + padding,
//! reverse scene order, inferred stretch) and `PrimaryAlign` justification.

use super::*;

// ---------------------------------------------------------------------------
// Horizontal / vertical stacking with spacing + padding
// ---------------------------------------------------------------------------

#[test]
fn horizontal_stack_with_spacing_and_padding() {
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        spacing: 10.0,
        padding: [4.0, 0.0, 0.0, 8.0], // top, right, bottom, left
        ..Default::default()
    };
    let f = t.push(frame(400.0, 100.0, al));
    let a = t.push(rect_child(f, 30.0, 20.0));
    let b = t.push(rect_child(f, 40.0, 20.0));
    let c = t.push(rect_child(f, 50.0, 20.0));

    solve_auto_layout(&mut t, f, &mut no_measure);

    // x starts at padding-left (8); each advances by own width + spacing(10).
    approx(placed_origin(&t, a), [8.0, 4.0]);
    approx(placed_origin(&t, b), [8.0 + 30.0 + 10.0, 4.0]);
    approx(placed_origin(&t, c), [48.0 + 40.0 + 10.0, 4.0]);
}

#[test]
fn horizontal_stack_can_flow_reverse_scene_order() {
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        spacing: 10.0,
        padding: [4.0, 0.0, 0.0, 8.0],
        flow_reverse: true,
        ..Default::default()
    };
    let f = t.push(frame(400.0, 100.0, al));
    let a = t.push(rect_child(f, 30.0, 20.0));
    let b = t.push(rect_child(f, 40.0, 20.0));
    let c = t.push(rect_child(f, 50.0, 20.0));

    solve_auto_layout(&mut t, f, &mut no_measure);

    approx(placed_origin(&t, c), [8.0, 4.0]);
    approx(placed_origin(&t, b), [8.0 + 50.0 + 10.0, 4.0]);
    approx(placed_origin(&t, a), [68.0 + 40.0 + 10.0, 4.0]);
}

#[test]
fn inferred_stack_can_ignore_child_stretch() {
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        spacing: 10.0,
        padding: [0.0, 0.0, 0.0, 0.0],
        child_layout: false,
        ..Default::default()
    };
    let f = t.push(frame(200.0, 100.0, al));
    let mut a = rect_child(f, 30.0, 20.0);
    a.layout_child = Some(LayoutChild {
        grow: 0.0,
        absolute: false,
        align_self: Some(CounterAlign::Stretch),
    });
    let a = t.push(a);

    solve_auto_layout(&mut t, f, &mut no_measure);

    approx(placed_size(&t, a), [30.0, 20.0]);
    approx(placed_origin(&t, a), [0.0, 0.0]);
}

#[test]
fn vertical_stack_with_spacing_and_padding() {
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Vertical,
        spacing: 6.0,
        padding: [5.0, 0.0, 0.0, 7.0],
        ..Default::default()
    };
    let f = t.push(frame(200.0, 400.0, al));
    let a = t.push(rect_child(f, 40.0, 20.0));
    let b = t.push(rect_child(f, 40.0, 30.0));

    solve_auto_layout(&mut t, f, &mut no_measure);

    // y starts at padding-top (5); counter (x) at padding-left (7).
    approx(placed_origin(&t, a), [7.0, 5.0]);
    approx(placed_origin(&t, b), [7.0, 5.0 + 20.0 + 6.0]);
}

// ---------------------------------------------------------------------------
// PrimaryAlign
// ---------------------------------------------------------------------------

#[test]
fn primary_align_center_packs_run_in_middle() {
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        spacing: 0.0,
        primary_align: PrimaryAlign::Center,
        ..Default::default()
    };
    let f = t.push(frame(100.0, 50.0, al));
    let a = t.push(rect_child(f, 20.0, 10.0));
    let b = t.push(rect_child(f, 20.0, 10.0));

    solve_auto_layout(&mut t, f, &mut no_measure);

    // packed = 40; inner = 100; offset = (100-40)/2 = 30.
    approx(placed_origin(&t, a), [30.0, 0.0]);
    approx(placed_origin(&t, b), [50.0, 0.0]);
}

#[test]
fn primary_align_center_overflow_clamps_to_start() {
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        primary_align: PrimaryAlign::Center,
        ..Default::default()
    };
    let f = t.push(frame(50.0, 20.0, al));
    let a = t.push(rect_child(f, 40.0, 10.0));
    let b = t.push(rect_child(f, 40.0, 10.0));

    solve_auto_layout(&mut t, f, &mut no_measure);

    approx(placed_origin(&t, a), [0.0, 0.0]);
    approx(placed_origin(&t, b), [40.0, 0.0]);
}

#[test]
fn primary_align_end_packs_run_at_end() {
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        primary_align: PrimaryAlign::End,
        ..Default::default()
    };
    let f = t.push(frame(100.0, 50.0, al));
    let a = t.push(rect_child(f, 20.0, 10.0));
    let b = t.push(rect_child(f, 30.0, 10.0));

    solve_auto_layout(&mut t, f, &mut no_measure);

    // packed = 50; offset = 100-50 = 50; a at 50, b at 70 → ends at 100.
    approx(placed_origin(&t, a), [50.0, 0.0]);
    approx(placed_origin(&t, b), [70.0, 0.0]);
}

#[test]
fn primary_align_space_between_first_at_start_last_at_end() {
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        spacing: 999.0, // must be ignored for SpaceBetween
        primary_align: PrimaryAlign::SpaceBetween,
        ..Default::default()
    };
    let f = t.push(frame(100.0, 50.0, al));
    let a = t.push(rect_child(f, 20.0, 10.0));
    let b = t.push(rect_child(f, 20.0, 10.0));
    let c = t.push(rect_child(f, 20.0, 10.0));

    solve_auto_layout(&mut t, f, &mut no_measure);

    // content = 60; free = 40; gap = 40/2 = 20.
    approx(placed_origin(&t, a), [0.0, 0.0]); // first at start
    approx(placed_origin(&t, b), [40.0, 0.0]); // 0+20+20
    approx(placed_origin(&t, c), [80.0, 0.0]); // last ends at 100
}
