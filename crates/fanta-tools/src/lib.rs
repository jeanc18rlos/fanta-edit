//! Tool state machines for Fantaisa.
//!
//! Select, marquee, pen, rect, ellipse, line, text, hand. Each tool consumes
//! pointer events and emits doc operations through `fanta-doc`'s typed surface,
//! so AI and human edits share the same code path.
//!
//! ## Why this crate exists
//!
//! Tool gestures (drag-to-marquee, drag-to-move, click-drag-to-create-shape,
//! drag-to-pan) are state machines: their behavior depends on the sequence of
//! events seen so far, not on any one event in isolation. Centralizing those
//! state machines here means the eventual UI shell in `fanta-app` is a thin
//! event-forwarding layer rather than a place where tool logic accumulates by
//! accretion. The shell instantiates a tool, threads each input event through
//! its [`Tool::handle_event`] method, and renders the [`ToolResponse`] hints.
//!
//! ## Layout
//!
//! - [`event`] — pointer / keyboard event types and modifier flags.
//! - [`context`] — [`ToolContext`], the shared bag every tool reads.
//! - [`tool`] — the [`Tool`] trait, [`ToolResponse`], and overlay descriptions.
//! - [`select`] — the default select / marquee / move tool.
//! - [`hand`] — the pan tool.
//! - [`rect`], [`ellipse`], [`line`], [`polygon`], [`star`] — shape-creation
//!   tools, one per primitive.
//!
//! [`Tool`]: crate::tool::Tool
//! [`Tool::handle_event`]: crate::tool::Tool::handle_event
//! [`ToolResponse`]: crate::tool::ToolResponse
//! [`ToolContext`]: crate::context::ToolContext

#![forbid(unsafe_code)]

pub mod context;
pub mod ellipse;
pub mod event;
pub mod frame;
pub mod hand;
pub mod ink;
pub mod line;
pub mod node_edit;
pub mod node_math;
pub mod path_select;
pub mod pen;
pub mod pencil;
pub mod polygon;
pub mod rect;
pub mod scale;
pub mod section;
pub mod select;
pub mod slice;
pub mod star;
pub mod text;
pub mod text_path;
pub mod tool;

// Flat re-exports for ergonomic call sites in `fanta-app`.
pub use context::ToolContext;
pub use ellipse::EllipseTool;
pub use event::{Button, KeyEvent, LogicalKey, ModifierKeys, PointerEvent, ToolEvent};
pub use frame::FrameTool;
pub use hand::HandTool;
pub use line::LineTool;
pub use node_edit::NodeEditTool;
pub use path_select::PathSelectTool;
pub use pen::PenTool;
pub use pencil::PencilTool;
pub use polygon::PolygonTool;
pub use rect::RectTool;
pub use scale::ScaleTool;
pub use section::SectionTool;
pub use select::SelectTool;
pub use slice::SliceTool;
pub use star::StarTool;
pub use text::TextTool;
pub use text_path::TextPathTool;
pub use tool::{
    CursorHint, MovingSelection, SnapGuide, SnapGuideAxis, Tool, ToolOverlay, ToolResponse,
    bounds_from_corners,
};
