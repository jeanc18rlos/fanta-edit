//! Grid auto layout (Figma `stackMode == GRID`, CSS grid).
//!
//! A [`LayoutMode::Grid`] frame places its flow children in the cells of a
//! column × row grid whose tracks live on the frame's [`GroupNode::grid`].
//!
//! - **Placement.** A child's [`LayoutChild::grid`] cell is used as authored
//!   (clamped to the columns; rows are added as needed). Children without one
//!   take the first free cell, row by row, like CSS `grid-auto-flow: row`.
//!   Rows the children need beyond the authored ones are added as Hug tracks
//!   (Figma `gridAutoTracks: ROWS`).
//! - **Track sizing.** Fixed tracks keep their size; Hug tracks take their
//!   largest single-track child; Flex tracks share what the others leave by
//!   their `fr`. On a Hug frame axis there is nothing left to share, so Flex
//!   tracks hug instead (as FILL does in the flex pass). Children spanning
//!   several tracks do not size them.
//! - **Children** align inside their cell area per [`GridCell`]'s
//!   `horizontal`/`vertical`; an unrotated child with `grow > 0` fills the
//!   area's width and one with `align_self: stretch` its height.
//! - **Hug.** Width (the frame's primary axis) and height (counter) hug to the
//!   tracks, gaps and padding, within the frame's min/max size.
//!
//! [`GroupNode::grid`]: crate::node::GroupNode::grid

use super::*;
use crate::node::{GridAlign, GridCell, GridLayout, GridTrack};

/// Every in-flow child of the grid frame `frame_id` with the cell the layout
/// pass puts it in — its own [`GridCell`] or the auto-placed one. Empty when
/// the frame is not a grid.
pub fn grid_cells<T: LayoutTree>(tree: &T, frame_id: NodeId) -> Vec<(NodeId, GridCell)> {
    let Some(al) = auto_layout_of(tree, frame_id).filter(|al| al.mode == LayoutMode::Grid) else {
        return Vec::new();
    };
    let grid = match tree.node(frame_id).map(|node| &node.data) {
        Some(NodeData::Group(group)) => group.grid.clone().unwrap_or_default(),
        _ => return Vec::new(),
    };
    let children = tree.children(frame_id);
    let infos = gather_child_infos(tree, &al, &children);
    let flow = collect_flow_indices(&infos);
    let mut rows = grid.rows;
    let cells = place_cells(tree, &infos, &flow, grid.columns.len().max(1), &mut rows);
    let track = |index: usize| u16::try_from(index).unwrap_or(u16::MAX);
    flow.iter()
        .zip(cells)
        .map(|(&index, placed)| {
            let cell = GridCell {
                column: track(placed.column),
                row: track(placed.row),
                column_span: track(placed.column_span),
                row_span: track(placed.row_span),
                horizontal: placed.horizontal,
                vertical: placed.vertical,
            };
            (infos[index].id, cell)
        })
        .collect()
}

pub(super) fn layout_frame_grid<T: LayoutTree>(
    tree: &mut T,
    frame_id: NodeId,
    al: &AutoLayout,
    grid: &GridLayout,
    children: &[NodeId],
    measure: &mut Measure,
) {
    let mut infos = gather_child_infos(tree, al, children);
    let flow = collect_flow_indices(&infos);
    let frame = frame_box(tree, frame_id);
    let [pad_t, pad_r, pad_b, pad_l] = al.padding;
    let columns: Vec<GridTrack> = if grid.columns.is_empty() {
        vec![GridTrack::Flex { fr: 1.0 }]
    } else {
        grid.columns.clone()
    };
    let mut rows = grid.rows.clone();
    let cells = place_cells(tree, &infos, &flow, columns.len(), &mut rows);
    let column_gap = finite_or_zero(grid.column_gap);
    let row_gap = finite_or_zero(grid.row_gap);

    let hug_width = al.primary_sizing == AxisSizing::Hug;
    let hug_height = al.counter_sizing == AxisSizing::Hug;
    let extents = |axis: usize, infos: &[ChildInfo]| -> Vec<(usize, usize, f64)> {
        flow.iter()
            .zip(&cells)
            .map(|(&index, cell)| {
                let size = transformed_local_box(&infos[index]).size[axis];
                if axis == 0 {
                    (cell.column, cell.column_span, size)
                } else {
                    (cell.row, cell.row_span, size)
                }
            })
            .collect()
    };
    let widths = track_sizes(
        &columns,
        (!hug_width).then(|| frame.size[0] - pad_l - pad_r),
        column_gap,
        &extents(0, &infos),
    );
    let column_starts = offsets(&widths, column_gap, pad_l);

    // Fill widths first: wrapping text then reports its final height, which
    // the row sizes depend on.
    for (&index, cell) in flow.iter().zip(&cells) {
        if fills(tree, &infos[index], Fill::Width) {
            infos[index].bx.size[0] = span(&widths, cell.column, cell.column_span, column_gap);
        }
    }
    refresh_resized_flow_children(tree, &mut infos, &flow, measure);

    let heights = track_sizes(
        &rows,
        (!hug_height).then(|| frame.size[1] - pad_t - pad_b),
        row_gap,
        &extents(1, &infos),
    );
    let row_starts = offsets(&heights, row_gap, pad_t);
    for (&index, cell) in flow.iter().zip(&cells) {
        if fills(tree, &infos[index], Fill::Height) {
            infos[index].bx.size[1] = span(&heights, cell.row, cell.row_span, row_gap);
        }
    }
    refresh_resized_flow_children(tree, &mut infos, &flow, measure);

    for (&index, cell) in flow.iter().zip(&cells) {
        let area = [
            span(&widths, cell.column, cell.column_span, column_gap),
            span(&heights, cell.row, cell.row_span, row_gap),
        ];
        let visible = transformed_local_box(&infos[index]).size;
        let x = column_starts[cell.column] + aligned(cell.horizontal, area[0] - visible[0]);
        let y = row_starts[cell.row] + aligned(cell.vertical, area[1] - visible[1]);
        place_child(&mut infos[index], DVec2::new(x, y));
    }
    write_flow_children(tree, &infos, &flow);

    let mut size = frame.size;
    if hug_width {
        size[0] = constrained_extent(al, 0, total(&widths, column_gap) + pad_l + pad_r);
    }
    if hug_height {
        size[1] = constrained_extent(al, 1, total(&heights, row_gap) + pad_t + pad_b);
    }
    if size != frame.size
        && let Some(node) = tree.node_mut(frame_id)
    {
        set_frame_size(node, size);
    }
}

/// A child's resolved cell: indices into the column and row tracks.
#[derive(Clone, Copy)]
struct Placed {
    column: usize,
    row: usize,
    column_span: usize,
    row_span: usize,
    horizontal: GridAlign,
    vertical: GridAlign,
}

/// Resolve every flow child's cell (explicit first, then auto-placed), adding
/// Hug rows until every child fits.
fn place_cells<T: LayoutTree>(
    tree: &T,
    infos: &[ChildInfo],
    flow: &[usize],
    column_count: usize,
    rows: &mut Vec<GridTrack>,
) -> Vec<Placed> {
    let explicit: Vec<Option<GridCell>> = flow
        .iter()
        .map(|&index| {
            tree.node(infos[index].id)
                .and_then(|node| node.layout_child)
                .and_then(|child| child.grid)
        })
        .collect();
    let mut occupied: Vec<Vec<bool>> = Vec::new();
    let ensure_rows = |occupied: &mut Vec<Vec<bool>>, count: usize| {
        while occupied.len() < count {
            occupied.push(vec![false; column_count]);
        }
    };
    let mut placed = vec![None; flow.len()];
    for (slot, cell) in explicit.iter().enumerate() {
        let Some(cell) = cell else { continue };
        let column = usize::from(cell.column).min(column_count - 1);
        let column_span = usize::from(cell.column_span.max(1)).min(column_count - column);
        let row = usize::from(cell.row);
        let row_span = usize::from(cell.row_span.max(1));
        ensure_rows(&mut occupied, row + row_span);
        for taken in &mut occupied[row..row + row_span] {
            for flag in &mut taken[column..column + column_span] {
                *flag = true;
            }
        }
        placed[slot] = Some(Placed {
            column,
            row,
            column_span,
            row_span,
            horizontal: cell.horizontal,
            vertical: cell.vertical,
        });
    }
    let mut cursor = 0usize;
    for slot in placed.iter_mut().filter(|slot| slot.is_none()) {
        loop {
            let (row, column) = (cursor / column_count, cursor % column_count);
            ensure_rows(&mut occupied, row + 1);
            cursor += 1;
            if !occupied[row][column] {
                occupied[row][column] = true;
                *slot = Some(Placed {
                    column,
                    row,
                    column_span: 1,
                    row_span: 1,
                    horizontal: GridAlign::Start,
                    vertical: GridAlign::Start,
                });
                break;
            }
        }
    }
    while rows.len() < occupied.len() {
        rows.push(GridTrack::Hug);
    }
    placed.into_iter().flatten().collect()
}

/// Track extents along one axis. `available` is the inner extent to fill, or
/// `None` on a Hug axis (Flex tracks then hug too).
fn track_sizes(
    tracks: &[GridTrack],
    available: Option<f64>,
    gap: f64,
    items: &[(usize, usize, f64)],
) -> Vec<f64> {
    let content = |track: usize| {
        items
            .iter()
            .filter(|(start, span, _)| *start == track && *span == 1)
            .map(|(_, _, extent)| *extent)
            .fold(0.0_f64, f64::max)
    };
    let mut sizes: Vec<f64> = tracks
        .iter()
        .enumerate()
        .map(|(index, track)| match track {
            GridTrack::Fixed { size } => finite_or_zero(*size).max(0.0),
            GridTrack::Hug => content(index),
            GridTrack::Flex { .. } if available.is_none() => content(index),
            GridTrack::Flex { .. } => 0.0,
        })
        .collect();
    if let Some(available) = available {
        let fr_total: f64 = tracks
            .iter()
            .map(|track| match track {
                GridTrack::Flex { fr } => finite_or_zero(*fr).max(0.0),
                _ => 0.0,
            })
            .sum();
        if fr_total > 0.0 {
            let used: f64 = sizes.iter().sum::<f64>() + gap * tracks.len().saturating_sub(1) as f64;
            let leftover = (available - used).max(0.0);
            for (size, track) in sizes.iter_mut().zip(tracks) {
                if let GridTrack::Flex { fr } = track {
                    *size = leftover * finite_or_zero(*fr).max(0.0) / fr_total;
                }
            }
        }
    }
    sizes
}

fn offsets(sizes: &[f64], gap: f64, start: f64) -> Vec<f64> {
    let mut cursor = start;
    sizes
        .iter()
        .map(|size| {
            let offset = cursor;
            cursor += size + gap;
            offset
        })
        .collect()
}

/// The extent of `span` tracks starting at `start`, gaps between them included.
fn span(sizes: &[f64], start: usize, span: usize, gap: f64) -> f64 {
    let end = (start + span).min(sizes.len());
    sizes[start..end].iter().sum::<f64>() + gap * end.saturating_sub(start + 1) as f64
}

fn total(sizes: &[f64], gap: f64) -> f64 {
    span(sizes, 0, sizes.len(), gap)
}

fn aligned(align: GridAlign, free: f64) -> f64 {
    match align {
        GridAlign::Start => 0.0,
        GridAlign::Center => free / 2.0,
        GridAlign::End => free,
    }
}

enum Fill {
    Width,
    Height,
}

/// Whether an unrotated, unscaled child fills its cell area on this axis.
fn fills<T: LayoutTree>(tree: &T, info: &ChildInfo, axis: Fill) -> bool {
    if info.matrix != DMat2::IDENTITY {
        return false;
    }
    let child = tree.node(info.id).and_then(|node| node.layout_child);
    match axis {
        Fill::Width => child.is_some_and(|child| child.grow > 0.0),
        Fill::Height => child.is_some_and(|child| child.align_self == Some(CounterAlign::Stretch)),
    }
}

fn finite_or_zero(value: f64) -> f64 {
    if value.is_finite() { value } else { 0.0 }
}
