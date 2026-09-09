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

/// Where a stroke sits relative to the shape's geometric edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StrokeAlignment {
    Inside,
    Center,
    Outside,
}

/// Horizontal text alignment within the text box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextAlignment {
    Left,
    Center,
    Right,
    Justify,
}

/// The flow direction of an auto-layout frame; `none` turns auto layout off
/// and leaves the children where the last solve put them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LayoutDirection {
    Horizontal,
    Vertical,
    None,
}

/// Cross-axis alignment of auto-layout children (CSS `align-items`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CrossAxisAlignment {
    Start,
    Center,
    End,
    /// Children stretch to the frame's cross-axis size.
    Stretch,
    Baseline,
}

/// Main-axis distribution of auto-layout children (CSS `justify-content`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MainAxisAlignment {
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceEvenly,
}

/// A relative z-order move within the node's current parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NamedLayerPosition {
    /// Topmost among its siblings (painted last).
    Front,
    /// Bottommost among its siblings (painted first).
    Back,
    /// One step up.
    Forward,
    /// One step down.
    Backward,
}

/// Where `set_index` puts a node among its siblings: a named relative move
/// (`"front"`, `"back"`, `"forward"`, `"backward"`) or an absolute slot
/// (`{"index": n}`, 0 = bottom).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum LayerPosition {
    Named(NamedLayerPosition),
    Absolute { index: usize },
}

/// The edge or centre line `align` snaps nodes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AlignEdge {
    Left,
    CenterX,
    Right,
    Top,
    CenterY,
    Bottom,
}

/// The axis `distribute` spaces nodes along.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DistributeAxis {
    Horizontal,
    Vertical,
}

/// One edit to the open design document. A batch of ops is applied in order
/// as a single undoable transaction: if any op fails, the whole batch rolls
/// back. Every `x`/`y` is a world (canvas) coordinate, y grows downward, and
/// every `id` is an exact node id from `design_state` / `batch_get` (never a
/// layer name).
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
    /// Place an image on the canvas as a bitmap layer. The image bytes are
    /// ingested as a project asset (written to `assets/images/` on save) and
    /// the node references it by asset id.
    CreateImage {
        /// The image content: a `data:image/...;base64,...` URI or raw base64
        /// of the encoded bytes (PNG/JPEG/WebP/GIF). `http(s)` URLs are NOT
        /// fetched here — the `place_generation` tool downloads a URL and
        /// places it in one step.
        source: String,
        /// Id of the parent frame/group. Omit to place on the active page.
        #[serde(default)]
        parent: Option<String>,
        /// Layer name. Omit for the default ("Image").
        #[serde(default)]
        name: Option<String>,
        x: f64,
        y: f64,
        /// Placed width in canvas units. Omit both to use the image's natural
        /// pixel size; give one and the other scales to preserve aspect.
        #[serde(default)]
        width: Option<f64>,
        #[serde(default)]
        height: Option<f64>,
        /// JSON stored in the node's metadata (e.g. AI generation provenance
        /// `{prompt, model, generation_id}`).
        #[serde(default)]
        meta: Option<serde_json::Value>,
    },
    /// Place an instance of an existing component. The instance takes the
    /// master's size and follows later edits to the master.
    CreateInstance {
        /// The component: its id, or its name when exactly one component has
        /// that name (`design_state` lists both).
        component: String,
        /// World x of the instance's top-left corner.
        x: f64,
        /// World y of the instance's top-left corner.
        y: f64,
        /// Id of the parent frame/group. Omit to place on the active page.
        #[serde(default)]
        parent: Option<String>,
        /// Layer name. Omit to use the component's name.
        #[serde(default)]
        name: Option<String>,
    },
    /// Update properties of an existing node. Only the provided fields change.
    SetProps {
        id: String,
        #[serde(default)]
        name: Option<String>,
        /// New world x of the node's bounding-box origin (rotation/scale
        /// preserved).
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
        /// Solid fill hex color; replaces the first fill (glyph color for text,
        /// background for frames).
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
    /// Set the single outline stroke of a shape or frame. Creates the stroke
    /// when the node has none (defaulting to 1px black); `width: 0` removes it.
    SetStroke {
        id: String,
        /// Stroke color as `#RRGGBB` or `#RRGGBBAA`.
        #[serde(default)]
        color: Option<String>,
        /// Stroke width in px; `0` removes the stroke.
        #[serde(default)]
        width: Option<f64>,
        /// Where the stroke sits relative to the edge (default `center`).
        #[serde(default)]
        align: Option<StrokeAlignment>,
    },
    /// Set (or remove) the node's drop shadow. Edits the first drop shadow in
    /// place, or adds one; `remove: true` deletes every drop shadow.
    SetShadow {
        id: String,
        /// Shadow color as `#RRGGBB` or `#RRGGBBAA` (default `#00000040`).
        #[serde(default)]
        color: Option<String>,
        /// Horizontal offset in px (default 0).
        #[serde(default)]
        x: Option<f64>,
        /// Vertical offset in px, positive = down (default 2).
        #[serde(default)]
        y: Option<f64>,
        /// Blur radius in px (default 4).
        #[serde(default)]
        blur: Option<f64>,
        /// Spread in px (default 0).
        #[serde(default)]
        spread: Option<f64>,
        /// Remove all drop shadows instead of setting one.
        #[serde(default)]
        remove: Option<bool>,
    },
    /// Change the typography of a text node. Only the provided fields change;
    /// they apply to the whole text (rich-text runs included).
    SetTextStyle {
        id: String,
        /// Font family name, e.g. `"Inter"`.
        #[serde(default)]
        font_family: Option<String>,
        /// OpenType weight 100–900 (400 regular, 500 medium, 600 semibold,
        /// 700 bold).
        #[serde(default)]
        font_weight: Option<u16>,
        /// Font size in px.
        #[serde(default)]
        font_size: Option<f64>,
        /// Line height as a multiple of the font size (1.0 = single; UI text
        /// is usually 1.2–1.5).
        #[serde(default)]
        line_height: Option<f64>,
        /// Extra letter spacing in px (negative tightens).
        #[serde(default)]
        letter_spacing: Option<f64>,
        /// Horizontal alignment within the text box.
        #[serde(default)]
        align: Option<TextAlignment>,
        /// Glyph color as `#RRGGBB` or `#RRGGBBAA`.
        #[serde(default)]
        color: Option<String>,
    },
    /// Turn a frame or group into an auto-layout (flex) container, change its
    /// layout rules, or (`direction: "none"`) turn auto layout off. Children
    /// are positioned by the layout solver; do not also set their `x`/`y`.
    SetAutoLayout {
        id: String,
        direction: LayoutDirection,
        /// Gap between children along the flow direction, in px.
        #[serde(default)]
        gap: Option<f64>,
        /// Padding in px: `[all]`, `[vertical, horizontal]`, or
        /// `[top, right, bottom, left]`.
        #[serde(default)]
        padding: Option<Vec<f64>>,
        /// Cross-axis alignment of the children.
        #[serde(default)]
        align_items: Option<CrossAxisAlignment>,
        /// Main-axis distribution of the children.
        #[serde(default)]
        justify: Option<MainAxisAlignment>,
    },
    /// Change a node's z-order among its current siblings.
    SetIndex { id: String, position: LayerPosition },
    /// Rotate a node to an absolute angle in degrees about the centre of its
    /// own box (positive = clockwise on the y-down canvas; 0 = upright).
    Rotate { id: String, degrees: f64 },
    /// Align nodes to an edge or centre line of their combined bounds (two or
    /// more ids), or a single node to its parent frame's bounds.
    Align { ids: Vec<String>, edge: AlignEdge },
    /// Space three or more nodes evenly along one axis, keeping the outermost
    /// two where they are.
    Distribute {
        ids: Vec<String>,
        axis: DistributeAxis,
    },
    /// Wrap nodes in a new plain group (no clipping, no background). The group
    /// is created where the topmost member lives; members from other parents
    /// move into it keeping their world position. Returns the group id as
    /// `created`.
    Group {
        ids: Vec<String>,
        /// Group name (default "Group").
        #[serde(default)]
        name: Option<String>,
    },
    /// Wrap nodes in a new frame sized to their combined bounds (Figma "Frame
    /// selection"): a clipping container without a background. Returns the
    /// frame id as `created`.
    FrameSelection {
        ids: Vec<String>,
        /// Frame name (default "Frame").
        #[serde(default)]
        name: Option<String>,
    },
    /// Dissolve a group or frame: its children move to its parent at the same
    /// z-slot, keeping their world positions, and the empty container is
    /// deleted. Reports the freed child ids as `children`.
    Ungroup { id: String },
    /// Deep-copy a node (and its subtree) next to the original, one slot above
    /// it, shifted by `dx`/`dy` world units (default 0). Returns the copy's id
    /// as `created`.
    Duplicate {
        id: String,
        #[serde(default)]
        dx: Option<f64>,
        #[serde(default)]
        dy: Option<f64>,
    },
    /// Promote a frame or group into a component master (it stays in place on
    /// the canvas, like Figma's "Create component"). Reports the new
    /// `component` id; place copies with `create_instance`.
    CreateComponent { id: String },
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

/// Cap on one serialized JSON state result, shared by every surface that hands
/// node listings to a model (the native `design_state` tool and the MCP
/// `batch_get`). A model cannot use megabytes of JSON, and a client that drops
/// the response leaves the agent with nothing at all — so a query whose answer
/// is this big is refused with instructions for narrowing it, never cut.
pub const MAX_JSON_RESPONSE_BYTES: usize = 256 * 1024;

/// Direct children one page listing returns when the caller gives no `limit`.
/// An imported `.fig` page can hold thousands of top-level nodes, and listing
/// all of them at once overruns [`MAX_JSON_RESPONSE_BYTES`] — which used to
/// leave an agent with no way to enumerate the page at all, since ids are
/// discovered by listing. A windowed default always answers; a caller who
/// wants more asks for it explicitly and may be refused for size.
pub const DEFAULT_CHILD_LIMIT: usize = 200;

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
    /// How many of the listed node's direct children to skip before returning
    /// any (default 0). Applies to the TOP-LEVEL listed node only — deeper
    /// levels are governed by `depth`, never by `offset`/`limit`. Ignored when
    /// fetching by `ids`.
    #[serde(default)]
    pub offset: Option<usize>,
    /// How many of the listed node's direct children to return, starting at
    /// `offset` (default 200). Applies to the TOP-LEVEL listed node only —
    /// deeper levels are governed by `depth`. The result reports
    /// `child_count`, `children_offset`, `children_limit` and `more_children`
    /// so a further call can continue where this one stopped. Ignored when
    /// fetching by `ids`.
    #[serde(default)]
    pub limit: Option<usize>,
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
    fn apply(&self, ops: Vec<DesignOp>, label: String, cx: &mut App) -> Result<serde_json::Value>;

    /// Render a PNG of a page or node region.
    fn screenshot(&self, target: ScreenshotTarget, cx: &mut App) -> Task<Result<Vec<u8>>>;

    /// List the project's FNX source files, or return one file's text.
    fn read_source(&self, path: Option<String>, cx: &mut App) -> Result<serde_json::Value>;

    /// A free `{x, y}` top-left for a `width` x `height` box on a page
    /// (default the active page): to the right of, or below, everything the
    /// page already holds, with a margin.
    fn find_empty_space(
        &self,
        width: f64,
        height: f64,
        page: Option<usize>,
        cx: &mut App,
    ) -> Result<serde_json::Value>;
}

/// Design guidelines for agents driving the canvas — served by the MCP
/// `get_guidelines` tool and condensed into the built-in agent's system
/// prompt. Kept next to the op vocabulary so the two stay in step.
pub const DESIGN_GUIDELINES: &str = r#"# Designing in Fanta

Fanta is an agent-native design tool: the design is `.fnx` source files and a
live canvas at once. These guidelines apply whether you drive the canvas
(`design_edit` / `batch_design`) or edit `.fnx` files directly.

## Coordinates and geometry
- World coordinates, in px, y grows DOWN. `x`/`y` of an op is the node's
  top-left corner; a node's reported `world_bounds` uses the same origin.
- Sizes are the node's own box; frames clip to `clip_size`, groups do not.
- Ids are exact 26-character node ids from the state tools. Never guess an id
  or address a node by name.
- Before creating a new top-level frame, ask for `empty_space` (state tool)
  and place it there; do not stack new work on top of existing frames.

## Structure
- Frame = container with a size, clipping, a background, optional auto layout.
  Group = a bare wrapper for moving things together. Screens, cards, list rows
  and buttons are frames; loose decoration is grouped.
- Prefer auto layout for any UI. Set `set_auto_layout` on the container, then
  create children inside it with a size and any `x`/`y` (the container's own
  `x`/`y` is fine); the layout solver repositions them. Use `gap` and
  `padding` in 4/8 px steps (4, 8, 12, 16, 24, 32, 48, 64).
- Nest frames for hierarchy (screen > section > row > control). Keep depth
  under about six levels.
- Name layers by role in Title Case: "Header", "Primary Button", "Card /
  Title". Never leave "Rectangle 12" behind in finished work.
- Repeated elements are components: build one instance right, `create_component`
  it, then `create_instance` the rest. Reference an existing component by its
  id (or unique name) from `design_state` `components`.

## Visual language
- Type scale (px): 12 caption, 14 body-small, 16 body, 20 heading-3, 24
  heading-2, 32 heading-1, 48 display. Line height 1.2 for headings, 1.4–1.5
  for body. Weights: 400 body, 500/600 labels, 700 headings.
- Contrast: body text at least 4.5:1 against its background. Muted text is
  a lighter tint of the text color, not a lighter opacity.
- Corner radius: 4–8 px controls, 12–16 px cards, 999 px pills.
- Shadows are subtle: `#00000014` to `#00000033`, y 2–8, blur 8–24.
- One accent color; neutrals do the rest. Use `#RRGGBBAA` for tints.
- Text boxes: make them wide enough for the content at the font size (about
  0.55 x size per character for Latin text) and 1.4 x size tall per line.

## Working method
1. Read state first (`design_state` / `get_editor_state`), then the page tree
   with `depth` 1–2; fetch nodes by id for detail. A page listing returns at
   most 200 of the page's direct children at a time and reports `child_count`,
   `children_offset`, `children_limit` and `more_children`: while
   `more_children` is true, call again with `offset` advanced by the limit to
   walk the rest of the page. Imported pages are wide — do not assume the
   first window is the whole page.
2. Batch related ops into ONE call with a descriptive `label` ("Add login
   card"); the batch is one undo step and rolls back entirely on failure.
   Use the `created` ids the result returns for follow-up ops.
3. Verify with a screenshot (`design_screenshot` / `get_screenshot`) of the
   frame you changed before reporting done. Fix what you see, then re-check.
4. Keep the selection meaningful: `select` what you just built so the user
   sees it.

## Two editing lanes
- Canvas ops (above): precise, immediate, undoable; the right lane for
  building and adjusting.
- `.fnx` files: each page is `pages/<slug>/page.fnx`, each component master
  `components/<slug>/master.fnx` (paths are in the state's `pages[].source`
  and `components[].source`). Edit the file with normal file tools for bulk
  textual changes (renaming many layers, retyping colors, restructuring a
  whole page); the open canvas hot-reloads about 300 ms after save. Keep the
  JSX well-formed.
- Never touch `*.ids.json` sidecars, `fanta.json`, `previews/` or `exports/`.
- Do not mix lanes on the same nodes in one step: finish canvas edits (they
  autosave) before editing the file, and vice versa.

## Don'ts
- Don't set `x`/`y` on children of an auto-layout frame; change the layout.
- Don't scale text or images by editing `width`/`height` to fake a style
  change; use `set_text_style` or replace the image.
- Don't delete or rewrite what you have not read; page trees are large,
  `child_count` means there is more below, and `more_children` means there is
  more beside what you were given.
"#;

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Child pagination is additive: a caller that sends neither field asks
    /// the same question it always did, and gets the surface's default window.
    #[test]
    fn a_query_without_pagination_fields_still_parses() {
        let query: NodeQuery = serde_json::from_value(serde_json::json!({
            "page": 0,
            "depth": 1,
            "include_geometry": false,
        }))
        .expect("the pre-pagination query shape stays valid");
        assert_eq!(query.offset, None);
        assert_eq!(query.limit, None);
    }
}
