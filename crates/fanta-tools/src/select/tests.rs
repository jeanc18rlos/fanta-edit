//! Behavioral tests for the select tool's full state machine.
//!
//! Co-located with the `select` module so they can reach its private phase
//! state (`super::state::Phase`, the `SelectTool::phase` field). These exercise
//! the orchestration across every sibling submodule — clicks, marquee, move,
//! resize, rotate, nudge, escape, and drag-to-reparent — split by gesture into
//! cohesive submodules so no single file is unwieldy.

pub(super) use super::state::Phase;
pub(crate) use super::*;
pub(crate) use crate::context::ToolContext;
pub(crate) use crate::event::{
    Button, KeyEvent, LogicalKey, ModifierKeys, PointerEvent, ToolEvent,
};
pub(crate) use crate::tool::{CursorHint, Tool, ToolOverlay};
pub(crate) use fanta_canvas::SnapEngine;
pub(crate) use fanta_doc::{
    Bounds, CanvasNode, Color, Doc, GroupNode, IndexKey, NodeData, NodeId, Operation, Transform2D,
    VectorNode, Viewport,
};
pub(crate) use glam::DVec2;

mod support;
pub(crate) use support::*;

mod clicks;
mod container_select;
mod keys;
mod marquee;
mod move_drag;
mod reparent;
mod resize;
mod rotate;
mod transient;
