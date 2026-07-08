//! The select tool — Figma's default cursor.
//!
//! ## What it does
//!
//! 1. **Click on a node** → replace the selection with that node.
//! 2. **Shift / Cmd / Ctrl click** → toggle membership in the selection.
//! 3. **Click empty space + drag** → marquee. The dragged rectangle accumulates
//!    nodes via [`fanta_canvas::hit_test_within`]; release commits a selection
//!    replacement. Alt held during drag switches the marquee mode from
//!    `Contains` (default — fully inside) to `Intersects` (any overlap).
//! 4. **Click a selected node + drag** → move the entire selection. Snap
//!    candidates are collected **once** at press (the non-dragged neighbors
//!    are stationary for the gesture) and reused on every frame, so a long
//!    drag no longer walks the whole scene per mouse-move. The drag itself is
//!    a **transient preview**: each frame writes transforms directly into the
//!    scene via `scene.get_mut` (no history ops). On release a single
//!    [`Transaction`] records one `SetTransform { old, new }` per node, so
//!    undo is exactly one step and the history holds one op per node — not the
//!    thousands a long drag would otherwise accumulate. On Esc/abort the
//!    press-time transforms are restored directly (no transaction).
//! 5. **Arrow keys** → nudge by 1 world unit (10 with Shift) — Figma's "tap to
//!    nudge, shift-tap for big nudge" convention. Wrapped in a transaction so
//!    a sequence of nudges still undoes as one step? No — Figma debounces by
//!    coalescing within a few hundred ms; we ship a single-op transaction per
//!    key and revisit if it feels coarse.
//! 6. **Escape** → clear the selection.
//!
//! Mirrors the select-tool finite state machine from tldraw and OpenPencil #1.
//!
//! ## Layout
//!
//! The state machine is split across cohesive submodules, all hanging off the
//! one [`SelectTool`] type:
//!
//! - [`state`] — the [`SelectTool`] struct, the interaction-phase enum, the
//!   per-gesture captured-state structs, and tuning constants.
//! - [`dispatch`] — the [`Tool`] impl, pointer/key fan-out, press hit-testing,
//!   release commit dispatch, and arrow-key nudging.
//! - [`mv`] — the move gesture and the per-frame `on_move` state-machine
//!   dispatcher (marquee/move promotion + resize/rotate forwarding).
//! - [`reparent`] — drag-to-reparent (drop into a container / pop out of a frame).
//! - [`resize`] — the corner/edge-handle resize gesture.
//! - [`rotate`] — the rotation-zone gesture.
//!
//! [`Transaction`]: fanta_doc::Transaction
//! [`Tool`]: crate::tool::Tool

mod dispatch;
mod mv;
mod reparent;
mod resize;
mod rotate;
mod state;

#[cfg(test)]
mod tests;

pub use state::SelectTool;
