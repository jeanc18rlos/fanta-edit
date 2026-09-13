//! Line and arrow creation tool.
//!
//! A line is two clicks (press → release). The committed node is a
//! [`VectorNode`] with an open path (no `Close`) and a single solid stroke.
//!
//! ## Modifiers
//!
//! - **Shift** snaps the angle to the nearest 45° step — Figma / Sketch / SVG
//!   editors all do this.
//! - **Alt** has no canonical meaning for a single line in Figma; we leave it
//!   reserved (no behavior) so future variants can pick it up without breaking
//!   existing reflexes.
//! - **Escape** mid-drag aborts.

use crate::context::ToolContext;
use crate::event::{Button, KeyEvent, LogicalKey, ModifierKeys, PointerEvent, ToolEvent};
use crate::tool::{CursorHint, SnapGuide, Tool, ToolOverlay, ToolResponse};
use fanta_canvas::SnapResult;
use fanta_doc::{CanvasNode, Fill, NodeData, Operation, PathData, Stroke, Transform2D, VectorNode};
use glam::DVec2;
use smallvec::SmallVec;
use std::f64::consts::TAU;

/// Snap a (dx, dy) vector to the nearest multiple of 45° (8 directions).
/// Preserves the magnitude — only the angle is quantized.
fn snap_to_45(dx: f64, dy: f64) -> (f64, f64) {
    let mag = dx.hypot(dy);
    if mag == 0.0 || !mag.is_finite() {
        return (dx, dy);
    }
    let angle = dy.atan2(dx);
    let step = TAU / 8.0;
    let quantized = (angle / step).round() * step;
    (quantized.cos() * mag, quantized.sin() * mag)
}

/// In-flight draft state. World-space.
#[derive(Debug, Clone, Copy)]
struct Draft {
    start: DVec2,
    end: DVec2,
}

struct LineGeometry {
    origin: DVec2,
    delta: DVec2,
    arrowhead: Option<[DVec2; 2]>,
}

impl LineGeometry {
    fn new(start: DVec2, end: DVec2, arrow: bool) -> Option<Self> {
        let delta = end - start;
        let length = delta.x.hypot(delta.y);
        if !start.is_finite() || !end.is_finite() || !length.is_finite() || length == 0.0 {
            return None;
        }
        let arrowhead = if arrow {
            let direction = delta / length;
            let head_length = 12.0_f64.min(length * 0.5);
            if head_length == 0.0 {
                return None;
            }
            let base = delta - direction * head_length;
            let offset = DVec2::new(-direction.y, direction.x) * (head_length * 0.5);
            let wings = [base + offset, base - offset];
            if wings
                .iter()
                .any(|wing| !wing.is_finite() || !(start + *wing).is_finite())
            {
                return None;
            }
            Some(wings)
        } else {
            None
        };
        Some(Self {
            origin: start,
            delta,
            arrowhead,
        })
    }

    fn path(&self) -> PathData {
        let mut path = PathData::new();
        path.move_to(0.0, 0.0).line_to(self.delta.x, self.delta.y);
        if let Some([first, second]) = self.arrowhead {
            path.move_to(first.x, first.y)
                .line_to(self.delta.x, self.delta.y)
                .line_to(second.x, second.y);
        }
        path
    }

    fn overlays(&self) -> impl Iterator<Item = ToolOverlay> {
        let end = self.origin + self.delta;
        let mut segments = SmallVec::<[(DVec2, DVec2); 3]>::new();
        segments.push((self.origin, end));
        if let Some([first, second]) = self.arrowhead {
            segments.push((self.origin + first, end));
            segments.push((end, self.origin + second));
        }
        segments
            .into_iter()
            .map(|(start, end)| ToolOverlay::PreviewLine {
                world_start: start.to_array(),
                world_end: end.to_array(),
            })
    }
}

fn snap_pointer(ctx: &ToolContext, screen: [f64; 2]) -> Option<SnapResult> {
    let world = ctx.screen_to_world(DVec2::from(screen));
    if !world.is_finite() {
        return None;
    }
    let snapped = ctx.snap.snap_point(world, &ctx.doc.scene, &[]);
    snapped.world.is_finite().then_some(snapped)
}

impl Draft {
    fn endpoints(&self, modifiers: ModifierKeys) -> (DVec2, DVec2) {
        if modifiers.contains(ModifierKeys::SHIFT) {
            let dx = self.end.x - self.start.x;
            let dy = self.end.y - self.start.y;
            let (qx, qy) = snap_to_45(dx, dy);
            (self.start, self.start + DVec2::new(qx, qy))
        } else {
            (self.start, self.end)
        }
    }
}

/// State machine for the line tool.
#[derive(Debug, Default)]
pub struct LineTool {
    draft: Option<Draft>,
    arrow: bool,
}

impl LineTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn arrow() -> Self {
        Self {
            arrow: true,
            ..Self::default()
        }
    }

    pub fn is_drafting(&self) -> bool {
        self.draft.is_some()
    }
}

impl Tool for LineTool {
    fn name(&self) -> &'static str {
        if self.arrow { "arrow" } else { "line" }
    }

    fn handle_event(&mut self, ctx: &mut ToolContext, event: ToolEvent) -> ToolResponse {
        match event {
            ToolEvent::Pointer(p) => self.handle_pointer(ctx, p),
            ToolEvent::Key(k) => self.handle_key(k),
        }
    }

    fn activate(&mut self, _ctx: &mut ToolContext) {
        self.draft = None;
    }

    fn deactivate(&mut self, _ctx: &mut ToolContext) {
        self.draft = None;
    }
}

impl LineTool {
    fn handle_pointer(&mut self, ctx: &mut ToolContext, p: PointerEvent) -> ToolResponse {
        match p {
            PointerEvent::Press {
                screen,
                button: Button::Primary,
                ..
            } => {
                let Some(snap) = snap_pointer(ctx, screen) else {
                    self.draft = None;
                    return ToolResponse::cursor(CursorHint::Crosshair);
                };
                let start = snap.world;
                self.draft = Some(Draft { start, end: start });
                let mut response = ToolResponse::cursor(CursorHint::Crosshair);
                for o in SnapGuide::from_snap_result(&snap) {
                    response.overlays.push(o);
                }
                response
            }
            PointerEvent::Move { screen, modifiers } => {
                let Some(mut draft) = self.draft else {
                    return ToolResponse::cursor(CursorHint::Crosshair);
                };
                let Some(snap) = snap_pointer(ctx, screen) else {
                    self.draft = None;
                    return ToolResponse::cursor(CursorHint::Crosshair);
                };
                draft.end = snap.world;
                self.draft = Some(draft);
                let (a, b) = draft.endpoints(modifiers);
                let mut response = ToolResponse::cursor(CursorHint::Crosshair);
                if let Some(geometry) = LineGeometry::new(a, b, self.arrow) {
                    response.overlays.extend(geometry.overlays());
                }
                for o in SnapGuide::from_snap_result(&snap) {
                    response.overlays.push(o);
                }
                response
            }
            PointerEvent::Release {
                screen,
                button: Button::Primary,
                modifiers,
            } => {
                let Some(mut draft) = self.draft.take() else {
                    return ToolResponse::cursor(CursorHint::Default);
                };
                let Some(snap) = snap_pointer(ctx, screen) else {
                    return ToolResponse::exit().with_cursor(CursorHint::Default);
                };
                draft.end = snap.world;
                let (a, b) = draft.endpoints(modifiers);
                if let Some(geometry) = LineGeometry::new(a, b, self.arrow) {
                    let mut strokes: SmallVec<[Stroke; 1]> = SmallVec::new();
                    // A line is a stroke, so the active "shape fill" color drives
                    // its stroke paint — cycling the palette recolors new lines.
                    strokes.push(Stroke {
                        paint: Fill::solid(ctx.new_shape_fill),
                        width: 2.0,
                        cap: Default::default(),
                        join: Default::default(),
                        miter_limit: 4.0,
                        dash: Vec::new(),
                        align: Default::default(),
                        per_side: None,
                    });
                    let mut node = CanvasNode::new(NodeData::Vector(VectorNode {
                        path: geometry.path(),
                        fills: SmallVec::new(),
                        strokes,
                        corner_radius: None,
                        corner_radii: None,
                        corner_smoothing: 0.0,
                        local_size: None,
                        parametric: None,
                    }));
                    if self.arrow {
                        node.name = "Arrow".to_owned();
                    }
                    node.transform = Transform2D::translation(a.x, a.y);
                    ctx.place_new_node_on_active_page(&mut node);
                    let id = node.id;
                    if let Err(e) = ctx.doc.apply(Operation::create_node(node)) {
                        tracing::warn!(target: "fanta-tools.line", "create failed: {e}");
                    } else {
                        ctx.doc.selection.select_only(id);
                    }
                }
                let mut response = ToolResponse::exit().with_cursor(CursorHint::Default);
                for o in SnapGuide::from_snap_result(&snap) {
                    response.overlays.push(o);
                }
                response
            }
            _ => ToolResponse::empty(),
        }
    }

    fn handle_key(&mut self, k: KeyEvent) -> ToolResponse {
        if matches!(k.key, LogicalKey::Escape) && self.draft.is_some() {
            self.draft = None;
            return ToolResponse::cursor(CursorHint::Default);
        }
        ToolResponse::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_canvas::SnapEngine;
    use fanta_doc::{Color, Doc, GroupNode, PathSegment, Viewport};

    fn pe_press(screen: [f64; 2], modifiers: ModifierKeys) -> ToolEvent {
        ToolEvent::Pointer(PointerEvent::Press {
            screen,
            button: Button::Primary,
            modifiers,
            count: 1,
        })
    }
    fn pe_move(screen: [f64; 2], modifiers: ModifierKeys) -> ToolEvent {
        ToolEvent::Pointer(PointerEvent::Move { screen, modifiers })
    }
    fn pe_release(screen: [f64; 2], modifiers: ModifierKeys) -> ToolEvent {
        ToolEvent::Pointer(PointerEvent::Release {
            screen,
            button: Button::Primary,
            modifiers,
        })
    }

    fn ctx_pieces() -> (Doc, Viewport, SnapEngine, DVec2) {
        (
            Doc::new(),
            Viewport::default(),
            SnapEngine {
                zoom: 1.0,
                targets: fanta_canvas::SnapTargets::empty(),
                ..Default::default()
            },
            DVec2::new(800.0, 600.0),
        )
    }

    fn path_lines(path: &PathData, transform: Transform2D) -> Vec<(DVec2, DVec2)> {
        let mut current = None;
        let mut lines = Vec::new();
        for segment in &path.segments {
            match segment {
                PathSegment::Move { to } => {
                    current = Some(transform.transform_point(DVec2::from(*to)));
                }
                PathSegment::Line { to } => {
                    let end = transform.transform_point(DVec2::from(*to));
                    lines.push((current.expect("line follows a move"), end));
                    current = Some(end);
                }
                _ => panic!("line and arrow paths must contain only open straight segments"),
            }
        }
        lines
    }

    fn preview_lines(response: &ToolResponse) -> Vec<(DVec2, DVec2)> {
        response
            .overlays
            .iter()
            .filter_map(|overlay| match overlay {
                ToolOverlay::PreviewLine {
                    world_start,
                    world_end,
                } => Some((DVec2::from(*world_start), DVec2::from(*world_end))),
                _ => None,
            })
            .collect()
    }

    fn assert_lines_match(actual: &[(DVec2, DVec2)], expected: &[(DVec2, DVec2)]) {
        assert_eq!(actual.len(), expected.len());
        for ((start, end), (expected_start, expected_end)) in actual.iter().zip(expected) {
            assert!(start.distance(*expected_start) < 1e-9, "{actual:?}");
            assert!(end.distance(*expected_end) < 1e-9, "{actual:?}");
        }
    }

    #[test]
    fn arrow_head_follows_reverse_and_diagonal_drags_and_caps_short_lengths() {
        for direction in [
            DVec2::X,
            -DVec2::X,
            DVec2::Y,
            -DVec2::Y,
            DVec2::new(1.0, 1.0).normalize(),
            DVec2::new(-1.0, 1.0).normalize(),
        ] {
            for length in [0.01, 1.0, 10.0, 24.0, 100.0] {
                let origin = DVec2::new(20.0, -30.0);
                let geometry = LineGeometry::new(origin, origin + direction * length, true)
                    .expect("valid arrow");
                let [first, second] = geometry.arrowhead.expect("arrowhead");
                let head_length = 12.0_f64.min(length * 0.5);
                for wing in [first, second] {
                    let tip_to_wing = geometry.delta - wing;
                    assert!((tip_to_wing.dot(direction) - head_length).abs() < 1e-9);
                    assert!(wing.dot(direction) > 0.0);
                    assert!(wing.is_finite());
                }
                let normal = DVec2::new(-direction.y, direction.x);
                assert!(((first - geometry.delta).dot(normal) - head_length * 0.5).abs() < 1e-9);
                assert!(((second - geometry.delta).dot(normal) + head_length * 0.5).abs() < 1e-9);
                assert_eq!(geometry.path().segments.len(), 5);
                let preview = ToolResponse {
                    overlays: geometry.overlays().collect(),
                    ..ToolResponse::empty()
                };
                assert_lines_match(
                    &preview_lines(&preview),
                    &path_lines(
                        &geometry.path(),
                        Transform2D::translation(origin.x, origin.y),
                    ),
                );
            }
        }
    }

    #[test]
    fn arrow_geometry_rejects_zero_nonfinite_and_overflowing_lengths() {
        for (start, end) in [
            (DVec2::ZERO, DVec2::ZERO),
            (DVec2::new(f64::NAN, 0.0), DVec2::X),
            (DVec2::ZERO, DVec2::new(f64::INFINITY, 0.0)),
            (DVec2::ZERO, DVec2::new(0.0, f64::NEG_INFINITY)),
            (DVec2::new(-f64::MAX, 0.0), DVec2::new(f64::MAX, 0.0)),
            (DVec2::ZERO, DVec2::splat(f64::MAX)),
        ] {
            assert!(LineGeometry::new(start, end, true).is_none());
        }
        let large = LineGeometry::new(DVec2::ZERO, DVec2::new(1e200, 1e200), true)
            .expect("a finite hypotenuse must not overflow intermediate squared coordinates");
        assert!(
            large
                .path()
                .segments
                .iter()
                .all(|segment| segment.end_point().is_some_and(DVec2::is_finite))
        );
    }

    #[test]
    fn arrow_shift_constrains_all_eight_directions_with_preview_commit_parity() {
        for direction in 0..8 {
            let (mut doc, mut viewport, snap, size) = ctx_pieces();
            let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
            let mut tool = LineTool::arrow();
            let angle = f64::from(direction) * TAU / 8.0;
            let end = [
                400.0 + (angle + 0.1).cos() * 100.0,
                300.0 + (angle + 0.1).sin() * 100.0,
            ];
            tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
            let preview = tool.handle_event(&mut ctx, pe_move(end, ModifierKeys::SHIFT));
            assert_eq!(ctx.doc.scene.len(), 0);
            let response = tool.handle_event(&mut ctx, pe_release(end, ModifierKeys::SHIFT));
            assert!(response.wants_exit);
            assert!(!tool.is_drafting());
            let id = *doc.selection.as_slice().first().expect("selected arrow");
            let NodeData::Vector(vector) = &doc.scene.get(id).expect("arrow").data else {
                panic!("arrow must be a vector")
            };
            let lines = path_lines(
                &vector.path,
                doc.scene.world_transform(id).expect("arrow transform"),
            );
            assert_eq!(lines.len(), 3);
            let (start, tip) = lines.first().expect("shaft");
            assert!(start.length() < 1e-9);
            assert!(tip.distance(DVec2::new(angle.cos(), angle.sin()) * 100.0) < 1e-9);
            assert_lines_match(&preview_lines(&preview), &lines);
        }
    }

    #[test]
    fn arrow_uses_grid_snapping_for_preview_and_commit() {
        let (mut doc, mut viewport, mut snap, size) = ctx_pieces();
        snap.targets = fanta_canvas::SnapTargets::GRID;
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = LineTool::arrow();
        tool.handle_event(&mut ctx, pe_press([401.0, 309.0], ModifierKeys::empty()));
        let preview = tool.handle_event(&mut ctx, pe_move([465.0, 341.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_release([465.0, 341.0], ModifierKeys::empty()));
        let id = *doc.selection.as_slice().first().expect("selected arrow");
        let NodeData::Vector(vector) = &doc.scene.get(id).expect("arrow").data else {
            panic!("arrow must be a vector")
        };
        let lines = path_lines(
            &vector.path,
            doc.scene.world_transform(id).expect("arrow transform"),
        );
        assert_eq!(
            lines.first(),
            Some(&(DVec2::new(0.0, 8.0), DVec2::new(64.0, 40.0)))
        );
        assert_lines_match(&preview_lines(&preview), &lines);
        assert!(
            preview.overlays.len() > 3,
            "snapped drag should retain its guides"
        );
    }

    #[test]
    fn arrow_creation_preserves_world_geometry_on_transformed_page_and_undo_redo() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.transform = Transform2D::scale_xy(2.0, 0.5)
            .then(&Transform2D::rotation(0.35))
            .then(&Transform2D::translation(120.0, -75.0));
        let page_id = page.id;
        doc.apply(Operation::create_node(page))
            .expect("create page");
        doc.add_page(page_id);
        doc.set_active_page(Some(page_id));
        doc.history = Default::default();
        let color = Color::rgba(204, 51, 102, 179);
        let mut ctx =
            ToolContext::new(&mut doc, &mut viewport, snap, size).with_new_shape_fill(color);
        let mut tool = LineTool::arrow();
        assert_eq!(tool.name(), "arrow");
        tool.handle_event(&mut ctx, pe_press([510.0, 420.0], ModifierKeys::empty()));
        let preview = tool.handle_event(&mut ctx, pe_move([430.0, 335.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_release([430.0, 335.0], ModifierKeys::empty()));
        assert_eq!(doc.scene.len(), 2);
        assert_eq!(doc.history.undo_depth(), 1);
        let [id] = doc.selection.as_slice() else {
            panic!("arrow must be the sole selection")
        };
        let created = doc.scene.get(*id).expect("arrow").clone();
        assert_eq!(created.name, "Arrow");
        assert_eq!(created.parent, Some(page_id));
        let NodeData::Vector(vector) = &created.data else {
            panic!("arrow must be a vector")
        };
        assert!(vector.fills.is_empty());
        let [stroke] = vector.strokes.as_slice() else {
            panic!("arrow must have one stroke")
        };
        assert_eq!(stroke.paint, Fill::solid(color));
        assert_eq!(stroke.width, 2.0);
        let lines = path_lines(
            &vector.path,
            doc.scene
                .world_transform(created.id)
                .expect("world transform"),
        );
        assert_lines_match(&preview_lines(&preview), &lines);
        assert_lines_match(
            &lines[..1],
            &[(DVec2::new(110.0, 120.0), DVec2::new(30.0, 35.0))],
        );
        assert!(doc.undo().expect("undo arrow"));
        assert_eq!(doc.scene.len(), 1);
        assert!(doc.scene.get(created.id).is_none());
        assert_eq!(doc.history.undo_depth(), 0);
        assert!(doc.redo().expect("redo arrow"));
        assert_eq!(doc.scene.get(created.id), Some(&created));
        assert_eq!(doc.history.undo_depth(), 1);
    }

    #[test]
    fn arrow_zero_length_cancel_and_nonfinite_events_do_not_create_nodes() {
        for case in ["zero", "escape", "deactivate", "press", "move", "release"] {
            for invalid in [
                [f64::NAN, 300.0],
                [400.0, f64::INFINITY],
                [f64::NEG_INFINITY, 300.0],
            ] {
                let (mut doc, mut viewport, snap, size) = ctx_pieces();
                let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
                let mut tool = LineTool::arrow();
                let start = if case == "press" {
                    invalid
                } else {
                    [400.0, 300.0]
                };
                tool.handle_event(&mut ctx, pe_press(start, ModifierKeys::empty()));
                if case == "move" {
                    let preview =
                        tool.handle_event(&mut ctx, pe_move(invalid, ModifierKeys::empty()));
                    assert!(preview_lines(&preview).is_empty());
                } else if case == "escape" {
                    tool.handle_event(
                        &mut ctx,
                        ToolEvent::Key(KeyEvent::press(LogicalKey::Escape)),
                    );
                } else if case == "deactivate" {
                    tool.deactivate(&mut ctx);
                }
                let end = match case {
                    "zero" => start,
                    "release" => invalid,
                    _ => [500.0, 360.0],
                };
                let response = tool.handle_event(&mut ctx, pe_release(end, ModifierKeys::empty()));
                assert!(preview_lines(&response).is_empty());
                assert!(!tool.is_drafting(), "{case}");
                assert!(doc.scene.is_empty(), "{case}");
                assert!(doc.selection.is_empty(), "{case}");
                assert_eq!(doc.history.undo_depth(), 0, "{case}");
            }
        }
    }

    #[test]
    fn snap_to_45_quantizes_arbitrary_angle() {
        // Vector at ~30° should snap to the nearest multiple of 45° (either
        // 45° or 0°).
        let (x, _) = snap_to_45(10.0, 5.0);
        // The resulting x component must align with one of cos(45°), cos(0°),
        // etc. We just check the magnitude is preserved.
        let m_before = (10.0_f64 * 10.0 + 5.0 * 5.0).sqrt();
        let m_after = (x * x + snap_to_45(10.0, 5.0).1.powi(2)).sqrt();
        assert!((m_before - m_after).abs() < 1e-9);
    }

    #[test]
    fn snap_to_45_no_op_at_zero_length() {
        let (x, y) = snap_to_45(0.0, 0.0);
        assert_eq!(x, 0.0);
        assert_eq!(y, 0.0);
    }

    #[test]
    fn press_does_not_mutate_doc() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = LineTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        assert!(tool.is_drafting());
        assert_eq!(doc.scene.len(), 0);
    }

    #[test]
    fn release_commits_line_node() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = LineTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_move([500.0, 360.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_release([500.0, 360.0], ModifierKeys::empty()));
        assert_eq!(doc.scene.len(), 1);
        let id = doc.scene.roots()[0];
        match &doc.scene.get(id).unwrap().data {
            NodeData::Vector(v) => {
                // Open path: M + L. Two segments, no Close.
                assert_eq!(v.path.segments.len(), 2);
                assert_eq!(v.fills.len(), 0);
                assert_eq!(v.strokes.len(), 1);
            }
            _ => panic!("expected vector node"),
        }
    }

    #[test]
    fn release_signals_wants_exit() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = LineTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        let r = tool.handle_event(&mut ctx, pe_release([420.0, 320.0], ModifierKeys::empty()));
        assert!(r.wants_exit);
    }

    #[test]
    fn shift_snaps_to_axis_aligned_direction() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = LineTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        // Drag almost horizontal: dx=100, dy=5 → with Shift the line should
        // snap to 0° (purely horizontal).
        tool.handle_event(&mut ctx, pe_move([500.0, 305.0], ModifierKeys::SHIFT));
        tool.handle_event(&mut ctx, pe_release([500.0, 305.0], ModifierKeys::SHIFT));
        let id = doc.scene.roots()[0];
        match &doc.scene.get(id).unwrap().data {
            NodeData::Vector(v) => {
                // Second segment's end y should equal start y (0° snap).
                let line_end = v.path.segments.last().unwrap();
                match *line_end {
                    fanta_doc::PathSegment::Line { to } => {
                        // Start is at world (0, 0). End should be on the x-axis.
                        assert!(to[1].abs() < 1e-6, "y not snapped to 0: {}", to[1]);
                    }
                    _ => panic!("expected line segment"),
                }
            }
            _ => panic!("expected vector node"),
        }
    }

    #[test]
    fn escape_aborts_draft() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = LineTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_move([500.0, 360.0], ModifierKeys::empty()));
        tool.handle_event(
            &mut ctx,
            ToolEvent::Key(KeyEvent::press(LogicalKey::Escape)),
        );
        assert_eq!(doc.scene.len(), 0);
        assert!(!tool.is_drafting());
    }

    #[test]
    fn zero_length_release_does_nothing() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = LineTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_release([400.0, 300.0], ModifierKeys::empty()));
        assert_eq!(doc.scene.len(), 0);
    }

    #[test]
    fn preview_overlay_present_during_drag() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = LineTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        let r = tool.handle_event(&mut ctx, pe_move([500.0, 360.0], ModifierKeys::empty()));
        let has_preview = r
            .overlays
            .iter()
            .any(|o| matches!(o, ToolOverlay::PreviewLine { .. }));
        assert!(has_preview);
    }

    #[test]
    fn release_without_press_is_safe() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = LineTool::new();
        let r = tool.handle_event(&mut ctx, pe_release([400.0, 300.0], ModifierKeys::empty()));
        assert!(!r.wants_exit);
        assert_eq!(doc.scene.len(), 0);
    }

    #[test]
    fn deactivate_clears_draft() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = LineTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        assert!(tool.is_drafting());
        tool.deactivate(&mut ctx);
        assert!(!tool.is_drafting());
    }
}
