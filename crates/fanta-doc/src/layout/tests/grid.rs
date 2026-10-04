//! Grid auto layout (`LayoutMode::Grid`): track sizing, placement, alignment,
//! hug.

use super::*;
use crate::node::{GridAlign, GridCell, GridLayout, GridTrack};

fn grid_frame(w: f64, h: f64, al: AutoLayout, grid: GridLayout) -> CanvasNode {
    let mut node = frame(
        w,
        h,
        AutoLayout {
            mode: LayoutMode::Grid,
            ..al
        },
    );
    if let NodeData::Group(group) = &mut node.data {
        group.grid = Some(grid);
    }
    node
}

fn fixed(size: f64) -> GridTrack {
    GridTrack::Fixed { size }
}

fn in_cell(mut node: CanvasNode, cell: GridCell) -> CanvasNode {
    node.layout_child = Some(LayoutChild {
        grid: Some(cell),
        grow: 0.0,
        absolute: false,
        align_self: None,
    });
    node
}

fn cell(column: u16, row: u16) -> GridCell {
    GridCell {
        column,
        row,
        column_span: 1,
        row_span: 1,
        horizontal: GridAlign::Start,
        vertical: GridAlign::Start,
    }
}

#[test]
fn children_fill_fixed_columns_row_by_row() {
    let mut t = VecTree::new();
    let f = t.push(grid_frame(
        400.0,
        400.0,
        AutoLayout {
            padding: [5.0, 0.0, 0.0, 8.0],
            ..Default::default()
        },
        GridLayout {
            columns: vec![fixed(100.0), fixed(50.0)],
            rows: vec![fixed(40.0), fixed(40.0)],
            column_gap: 10.0,
            row_gap: 6.0,
        },
    ));
    let a = t.push(rect_child(f, 20.0, 20.0));
    let b = t.push(rect_child(f, 20.0, 20.0));
    let c = t.push(rect_child(f, 20.0, 20.0));
    solve_auto_layout(&mut t, f, &mut no_measure);
    approx(placed_origin(&t, a), [8.0, 5.0]);
    approx(placed_origin(&t, b), [8.0 + 100.0 + 10.0, 5.0]);
    approx(placed_origin(&t, c), [8.0, 5.0 + 40.0 + 6.0]);
}

#[test]
fn flex_columns_share_the_leftover_by_fr() {
    let mut t = VecTree::new();
    let f = t.push(grid_frame(
        310.0,
        100.0,
        AutoLayout::default(),
        GridLayout {
            columns: vec![GridTrack::Flex { fr: 1.0 }, GridTrack::Flex { fr: 2.0 }],
            rows: vec![fixed(100.0)],
            column_gap: 10.0,
            row_gap: 0.0,
        },
    ));
    let mut wide = rect_child(f, 10.0, 10.0);
    wide.layout_child = Some(LayoutChild {
        grid: Some(cell(1, 0)),
        grow: 1.0,
        absolute: false,
        align_self: Some(CounterAlign::Stretch),
    });
    let wide = t.push(wide);
    solve_auto_layout(&mut t, f, &mut no_measure);
    // Columns 100 and 200 (300 shared 1:2 after the 10 gap); the child fills
    // the second column's width and the row's height.
    approx(placed_origin(&t, wide), [110.0, 0.0]);
    approx(placed_size(&t, wide), [200.0, 100.0]);
}

#[test]
fn hug_rows_take_their_tallest_child_and_extra_rows_are_added() {
    let mut t = VecTree::new();
    let f = t.push(grid_frame(
        200.0,
        999.0,
        AutoLayout::default(),
        GridLayout {
            columns: vec![fixed(100.0), fixed(100.0)],
            rows: vec![GridTrack::Hug],
            column_gap: 0.0,
            row_gap: 4.0,
        },
    ));
    t.push(rect_child(f, 10.0, 30.0));
    t.push(rect_child(f, 10.0, 50.0));
    let third = t.push(rect_child(f, 10.0, 10.0));
    solve_auto_layout(&mut t, f, &mut no_measure);
    // Row 0 hugs to 50; the third child wraps to an added row below it.
    approx(placed_origin(&t, third), [0.0, 54.0]);
}

#[test]
fn an_explicit_cell_spans_and_centers() {
    let mut t = VecTree::new();
    let f = t.push(grid_frame(
        300.0,
        100.0,
        AutoLayout::default(),
        GridLayout {
            columns: vec![fixed(100.0), fixed(100.0), fixed(100.0)],
            rows: vec![fixed(100.0)],
            column_gap: 0.0,
            row_gap: 0.0,
        },
    ));
    let auto = t.push(rect_child(f, 20.0, 20.0));
    let spanning = t.push(in_cell(
        rect_child(f, 40.0, 20.0),
        GridCell {
            column_span: 2,
            horizontal: GridAlign::Center,
            vertical: GridAlign::End,
            ..cell(1, 0)
        },
    ));
    solve_auto_layout(&mut t, f, &mut no_measure);
    // The explicit cell is placed first; the auto child takes column 0.
    approx(placed_origin(&t, auto), [0.0, 0.0]);
    // Columns 1-2 span 200: centered 40 starts at 100 + 80; bottom-aligned 20.
    approx(placed_origin(&t, spanning), [180.0, 80.0]);
}

#[test]
fn a_hug_grid_sizes_to_its_tracks_gaps_and_padding() {
    let mut t = VecTree::new();
    let f = t.push(grid_frame(
        999.0,
        999.0,
        AutoLayout {
            padding: [10.0, 10.0, 10.0, 10.0],
            primary_sizing: AxisSizing::Hug,
            counter_sizing: AxisSizing::Hug,
            ..Default::default()
        },
        GridLayout {
            columns: vec![fixed(40.0), GridTrack::Flex { fr: 1.0 }],
            rows: vec![GridTrack::Hug],
            column_gap: 8.0,
            row_gap: 8.0,
        },
    ));
    t.push(rect_child(f, 30.0, 25.0));
    t.push(rect_child(f, 60.0, 15.0));
    solve_auto_layout(&mut t, f, &mut no_measure);
    // Width: 40 + 8 + (flex hugs to 60) + 20 padding; height: 25 + 20.
    approx(placed_size(&t, f), [128.0, 45.0]);
}

#[test]
fn a_grid_filled_by_its_stack_parent_reflows_its_flex_columns() {
    // The grid is solved at its own 100 width first; its parent's FILL then
    // widens it to 400, and the re-flow must run the grid pass (not the flex
    // one) so the second 1fr column starts at 200.
    let mut t = VecTree::new();
    let row = t.push(frame(
        400.0,
        50.0,
        AutoLayout {
            mode: LayoutMode::Horizontal,
            ..Default::default()
        },
    ));
    let mut grid = grid_frame(
        100.0,
        20.0,
        AutoLayout::default(),
        GridLayout {
            columns: vec![GridTrack::Flex { fr: 1.0 }, GridTrack::Flex { fr: 1.0 }],
            rows: vec![fixed(20.0)],
            column_gap: 0.0,
            row_gap: 0.0,
        },
    );
    grid.parent = Some(row);
    grid.layout_child = Some(LayoutChild {
        grow: 1.0,
        ..Default::default()
    });
    let grid = t.push(grid);
    let second = t.push(in_cell(rect_child(grid, 10.0, 10.0), cell(1, 0)));
    solve_auto_layout(&mut t, row, &mut no_measure);
    approx(placed_size(&t, grid), [400.0, 20.0]);
    approx(placed_origin(&t, second), [200.0, 0.0]);
}

#[test]
fn grid_cells_reports_pinned_and_auto_placed_cells() {
    let mut t = VecTree::new();
    let f = t.push(grid_frame(
        200.0,
        100.0,
        AutoLayout::default(),
        GridLayout {
            columns: vec![fixed(50.0), fixed(50.0)],
            ..Default::default()
        },
    ));
    let pinned = t.push(in_cell(rect_child(f, 10.0, 10.0), cell(0, 0)));
    let auto = t.push(rect_child(f, 10.0, 10.0));
    let wrapped = t.push(rect_child(f, 10.0, 10.0));
    let cells = super::super::grid_cells(&t, f);
    let at = |id| {
        let cell = cells.iter().find(|(node, _)| *node == id).expect("cell").1;
        (cell.column, cell.row)
    };
    assert_eq!(at(pinned), (0, 0));
    assert_eq!(at(auto), (1, 0));
    assert_eq!(at(wrapped), (0, 1));
}
