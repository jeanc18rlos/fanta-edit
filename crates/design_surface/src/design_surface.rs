//! The seam between the agent's design tools and the design editor.
//!
//! `crates/agent` must not depend on the editor (`fig_viewer`/`workspace`), so
//! the editor registers a [`DesignSurface`] provider in a GPUI global and the
//! agent tools resolve it per call. Ops and queries are typed DTOs so tool
//! input schemas stay stable even if the editor's document model evolves.

use std::rc::Rc;

use anyhow::Result;
use gpui::{App, Global, Task};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The kind of node a `create_node` op produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DesignNodeType {
    /// A clipping container with a background (a Figma-style frame).
    Frame,
    Rectangle,
    Ellipse,
    Text,
}

/// One edit to the open design document. A batch of ops is applied in order
/// as a single undoable transaction: if any op fails, the whole batch rolls
/// back.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum DesignOp {
    /// Create a new node. `x`/`y` are world (canvas) coordinates of the node's
    /// top-left corner.
    CreateNode {
        node_type: DesignNodeType,
        /// Id of the parent frame/group. Omit to place on the active page.
        #[serde(default)]
        parent: Option<String>,
        /// Layer name. Omit for a type-appropriate default.
        #[serde(default)]
        name: Option<String>,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        /// Solid fill as a hex color (`#RRGGBB` or `#RRGGBBAA`). For text this
        /// is the glyph color.
        #[serde(default)]
        fill: Option<String>,
        /// Text content (text nodes only).
        #[serde(default)]
        text: Option<String>,
        /// Font size in px (text nodes only, default 16).
        #[serde(default)]
        font_size: Option<f32>,
    },
    /// Update properties of an existing node. Only the provided fields change.
    SetProps {
        id: String,
        #[serde(default)]
        name: Option<String>,
        /// New world x of the node's origin (rotation/scale preserved).
        #[serde(default)]
        x: Option<f64>,
        #[serde(default)]
        y: Option<f64>,
        #[serde(default)]
        width: Option<f64>,
        #[serde(default)]
        height: Option<f64>,
        /// Node opacity in `0.0..=1.0`.
        #[serde(default)]
        opacity: Option<f32>,
        /// Solid fill hex color; replaces the first fill (glyph color for text).
        #[serde(default)]
        fill: Option<String>,
        /// Uniform corner radius (rectangles and frames).
        #[serde(default)]
        corner_radius: Option<f64>,
        /// Replace the text content (text nodes only).
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        hidden: Option<bool>,
        #[serde(default)]
        locked: Option<bool>,
    },
    /// Move a node under a new parent, preserving its world position.
    Reparent {
        id: String,
        /// New parent frame/group id. Omit to move to the active page root.
        #[serde(default)]
        parent: Option<String>,
        /// Position among the new siblings (0 = bottom). Omit to append on top.
        #[serde(default)]
        index: Option<usize>,
    },
    /// Delete a node and its whole subtree.
    Delete { id: String },
    /// Replace the editor selection.
    Select { ids: Vec<String> },
    /// Set the persisted document viewport.
    SetViewport {
        #[serde(default)]
        center: Option<[f64; 2]>,
        #[serde(default)]
        zoom: Option<f64>,
    },
}

/// Which nodes `DesignSurface::get_nodes` returns.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct NodeQuery {
    /// Fetch these node ids in full detail. When omitted, the query lists the
    /// node tree of a page in compact form instead.
    #[serde(default)]
    pub ids: Option<Vec<String>>,
    /// Page index to list (defaults to the active page).
    #[serde(default)]
    pub page: Option<usize>,
    /// How many levels of children to include below each listed node.
    /// Omit for the full subtree.
    #[serde(default)]
    pub depth: Option<u32>,
    /// Include world-space bounding boxes.
    #[serde(default)]
    pub include_geometry: bool,
}

/// What `DesignSurface::screenshot` renders.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ScreenshotTarget {
    /// Page index to render (defaults to the active page).
    #[serde(default)]
    pub page: Option<usize>,
    /// Render only this node's region of its page.
    #[serde(default)]
    pub node: Option<String>,
    /// Cap on the longer output dimension in pixels (default 1024).
    #[serde(default)]
    pub max_dimension: Option<u32>,
}

/// The editor-side provider driving the open design document. All methods
/// operate on the most recently focused design canvas.
pub trait DesignSurface: 'static {
    /// Project/document overview: pages, active page, selection, viewport,
    /// node counts, and editability.
    fn state(&self, cx: &mut App) -> Result<serde_json::Value>;

    /// Node detail (by id) or a compact page tree listing.
    fn get_nodes(&self, query: NodeQuery, cx: &mut App) -> Result<serde_json::Value>;

    /// Apply a batch of ops as one undoable transaction. Returns created node
    /// ids and per-op status; on failure the batch is rolled back.
    fn apply(&self, ops: Vec<DesignOp>, label: String, cx: &mut App)
    -> Result<serde_json::Value>;

    /// Render a PNG of a page or node region.
    fn screenshot(&self, target: ScreenshotTarget, cx: &mut App) -> Task<Result<Vec<u8>>>;

    /// List the project's FNX source files, or return one file's text.
    fn read_source(&self, path: Option<String>, cx: &mut App) -> Result<serde_json::Value>;
}

#[derive(Default)]
struct DesignSurfaceRegistry {
    provider: Option<Rc<dyn DesignSurface>>,
}

impl Global for DesignSurfaceRegistry {}

/// Install `provider` as the process-wide design surface. Called once from the
/// design editor's `init`.
pub fn register(provider: Rc<dyn DesignSurface>, cx: &mut App) {
    cx.set_global(DesignSurfaceRegistry {
        provider: Some(provider),
    });
}

/// The registered design surface, if the editor has installed one.
pub fn active(cx: &App) -> Option<Rc<dyn DesignSurface>> {
    cx.try_global::<DesignSurfaceRegistry>()?.provider.clone()
}
