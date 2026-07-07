//! Wrapping (`stackWrap`): row/column breaks, counter-hug summing line
//! thicknesses, and a single oversized child keeping its own line.

use super::*;

// ---------------------------------------------------------------------------
// Wrapping (stackWrap)
// ---------------------------------------------------------------------------

#[test]
fn horizontal_wrap_breaks_to_second_row() {
    // A 100-wide frame, spacing 10, three 40-wide children. First two fit
    // (40 + 10 + 40 = 90 <= 100); the third would push to 140 > 100, so it wraps
    // to row 2. Counter spacing 5 separates the rows; row 1 thickness is 20.
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        spacing: 10.0,
        counter_spacing: 5.0,
        wrap: true,
        ..Default::default()
    };
    let f = t.push(frame(100.0, 200.0, al));
    let a = t.push(rect_child(f, 40.0, 20.0));
    let b = t.push(rect_child(f, 40.0, 20.0));
    let c = t.push(rect_child(f, 40.0, 20.0));

    solve_auto_layout(&mut t, f, &mut no_measure);

    // Row 1: a at x=0, b at x=50 (40 + spacing 10), both at y=0.
    approx(placed_origin(&t, a), [0.0, 0.0]);
    approx(placed_origin(&t, b), [50.0, 0.0]);
    // Row 2: c at x=0, y = row1 thickness(20) + counter_spacing(5) = 25.
    approx(placed_origin(&t, c), [0.0, 25.0]);
}

#[test]
fn vertical_wrap_breaks_to_second_column() {
    // Vertical wrap: primary axis is y. A 100-tall frame, no spacing, three
    // 40-tall children: first two fit (80 <= 100), third wraps to column 2.
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Vertical,
        spacing: 0.0,
        counter_spacing: 8.0,
        wrap: true,
        ..Default::default()
    };
    let f = t.push(frame(200.0, 100.0, al));
    let a = t.push(rect_child(f, 30.0, 40.0));
    let b = t.push(rect_child(f, 30.0, 40.0));
    let c = t.push(rect_child(f, 30.0, 40.0));

    solve_auto_layout(&mut t, f, &mut no_measure);

    // Column 1: a at y=0, b at y=40, both at x=0.
    approx(placed_origin(&t, a), [0.0, 0.0]);
    approx(placed_origin(&t, b), [0.0, 40.0]);
    // Column 2: c at y=0, x = col1 thickness(30) + counter_spacing(8) = 38.
    approx(placed_origin(&t, c), [38.0, 0.0]);
}

#[test]
fn wrap_hug_counter_axis_sums_row_thicknesses() {
    // Two rows of children with differing heights; counter-hug should resize the
    // frame's height to row1_thickness + counter_spacing + row2_thickness + pads.
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        spacing: 0.0,
        counter_spacing: 6.0,
        wrap: true,
        counter_sizing: AxisSizing::Hug,
        ..Default::default()
    };
    let f = t.push(frame(80.0, 500.0, al));
    // Each 50 wide → two fit per 80-wide row? 50 + 50 = 100 > 80, so one per row.
    let _a = t.push(rect_child(f, 50.0, 20.0)); // row 1, thickness 20
    let _b = t.push(rect_child(f, 50.0, 30.0)); // row 2, thickness 30

    solve_auto_layout(&mut t, f, &mut no_measure);

    // Counter (height) hugged to 20 + 6 + 30 = 56 (no counter padding here).
    let fh = placed_size(&t, f)[1];
    assert!(
        (fh - 56.0).abs() < EPS,
        "counter-hug height = {fh}, want 56"
    );
}

#[test]
fn wrap_single_oversized_child_keeps_its_own_line() {
    // A child wider than the frame's inner main extent must not be dropped — it
    // occupies its own line, and the following child wraps after it.
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        spacing: 0.0,
        counter_spacing: 0.0,
        wrap: true,
        ..Default::default()
    };
    let f = t.push(frame(60.0, 200.0, al));
    let big = t.push(rect_child(f, 100.0, 20.0)); // wider than 60
    let small = t.push(rect_child(f, 20.0, 20.0));

    solve_auto_layout(&mut t, f, &mut no_measure);

    approx(placed_origin(&t, big), [0.0, 0.0]);
    // `small` wraps below `big` (big's line is full at 100 > 60).
    approx(placed_origin(&t, small), [0.0, 20.0]);
}
