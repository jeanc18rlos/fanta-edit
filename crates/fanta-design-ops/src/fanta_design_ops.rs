//! The vocabulary agents edit Fanta designs with: design ops, queries,
//! comment drafts and the guidelines that describe them. The editor applies
//! them through `design_surface`, and the headless engine applies the same
//! ops without the app, so nothing here may depend on GPUI.

use std::{io::Read as _, path::Path};

use anyhow::Result;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextSizing {
    Fixed,
    AutoHeight,
    AutoWidth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LayoutSizing {
    Fixed,
    Hug,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DesignVariableType {
    Color,
    Float,
    String,
    Boolean,
    Typography,
    /// One option out of the fixed set declared in `CreateVariable.options`.
    Enum,
    /// A project image asset id, or null for "no image"; binds to image fills.
    Image,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DesignComponentPropertyKind {
    Text,
    Boolean,
    Number,
    Color,
    InstanceSwap,
    Variant,
}

/// A canvas comment an agent places: a review finding, a question or a note.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct CommentDraft {
    /// Page index; the active page when omitted.
    #[serde(default)]
    pub page: Option<usize>,
    /// Anchor the pin to this node (prefixed id) so it follows the layer.
    #[serde(default)]
    pub node: Option<String>,
    /// Point on the node in its local units; its centre when omitted.
    #[serde(default)]
    pub offset: Option<[f64; 2]>,
    /// World position when no node is given; the layer under it becomes the anchor.
    #[serde(default)]
    pub world: Option<[f64; 2]>,
    pub text: String,
    #[serde(default)]
    pub author: Option<String>,
    /// info, minor, major or blocker.
    #[serde(default)]
    pub severity: Option<String>,
    /// layout, typography, color, accessibility, consistency, content, interaction, motion or other.
    #[serde(default)]
    pub category: Option<String>,
    /// Groups the findings of one review run.
    #[serde(default)]
    pub review_run_id: Option<String>,
    /// A change that would address the finding, as a design ops list the user applies.
    #[serde(default)]
    pub suggested_fix: Option<SuggestedFixDraft>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SuggestedFixDraft {
    pub summary: String,
    /// Content edits from `batch_design` / `design_edit`; omit `select` and `set_viewport`.
    pub ops: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "prop", rename_all = "snake_case")]
pub enum VariableBindingProperty {
    FillColor {
        #[serde(default)]
        index: u16,
    },
    StrokeColor {
        #[serde(default)]
        index: u16,
    },
    StrokeWidth {
        #[serde(default)]
        index: u16,
    },
    CornerRadius,
    Opacity,
    Visible,
    TextContent,
    TextStyle,
    ClipWidth,
    ClipHeight,
    /// The artwork of `fills[index]`; takes an image variable.
    ImageFill {
        #[serde(default)]
        index: u16,
    },
}

/// The flow direction of an auto-layout frame; `none` turns auto layout off
/// and leaves the children where the last solve put them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LayoutDirection {
    Horizontal,
    Vertical,
    /// Children sit in grid cells. A frame that was not a grid starts with
    /// ⌈√n⌉ equal columns; `set_grid_layout` sets its tracks.
    Grid,
    None,
}

/// How wrapped lines share the cross axis: `auto` packs them `counter_gap`
/// apart, `space_between` spreads them across the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AlignContent {
    Auto,
    SpaceBetween,
}

/// One grid track: a number is a fixed size in px, `"<n>fr"` (or `"fr"`) a
/// share of the space the other tracks leave, `"auto"` hugs its content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum GridTrackSpec {
    Px(f64),
    Keyword(String),
}

/// Where a grid child sits inside its cell area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CellAlignment {
    Start,
    Center,
    End,
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
/// One layer of a node's effect stack (`set_effects`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DesignEffect {
    /// A shadow cast outside the shape.
    DropShadow(DesignShadow),
    /// A shadow inside the shape's edge.
    InnerShadow(DesignShadow),
    /// Blur the node itself.
    LayerBlur { radius: f64 },
    /// Blur what is behind the node (frosted glass); give the node a
    /// translucent fill.
    BackgroundBlur { radius: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DesignShadow {
    /// `#RRGGBB` or `#RRGGBBAA` (default `#00000040`).
    #[serde(default)]
    pub color: Option<String>,
    /// Horizontal offset in px (default 0).
    #[serde(default)]
    pub x: Option<f64>,
    /// Vertical offset in px, positive is down (default 2).
    #[serde(default)]
    pub y: Option<f64>,
    /// Blur radius in px (default 4).
    #[serde(default)]
    pub blur: Option<f64>,
    /// Spread in px (default 0).
    #[serde(default)]
    pub spread: Option<f64>,
}

/// One fill paint (`set_fill`). Gradient geometry is in the node's own 0–1
/// box: (0, 0) is its top-left corner and (1, 1) its bottom-right.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DesignPaint {
    Solid {
        /// `#RRGGBB` or `#RRGGBBAA`.
        color: String,
    },
    Linear {
        /// Start point (default top, `[0.5, 0]`).
        #[serde(default)]
        from: Option<[f32; 2]>,
        /// End point (default bottom, `[0.5, 1]`).
        #[serde(default)]
        to: Option<[f32; 2]>,
        stops: Vec<DesignGradientStop>,
    },
    Radial {
        /// Center (default `[0.5, 0.5]`).
        #[serde(default)]
        center: Option<[f32; 2]>,
        /// Radius as a fraction of the node's size (default 0.5).
        #[serde(default)]
        radius: Option<f32>,
        stops: Vec<DesignGradientStop>,
    },
    /// An image paint: a `data:image/...;base64,...` URI or raw base64 of
    /// PNG/JPEG/WebP/GIF bytes, stored as a project asset.
    Image {
        source: String,
        #[serde(default)]
        fit: Option<DesignImageFit>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DesignGradientStop {
    /// Position along the gradient, 0–1.
    pub position: f32,
    /// `#RRGGBB` or `#RRGGBBAA`.
    pub color: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DesignImageFit {
    /// Cover the node, cropping overflow (default).
    Fill,
    /// Fit inside the node, letterboxed.
    Fit,
    /// Stretch to the node's box.
    Stretch,
    /// Repeat at the image's natural size.
    Tile,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DesignStrokeCap {
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DesignStrokeJoin {
    Miter,
    Round,
    Bevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HorizontalConstraint {
    Left,
    Right,
    /// Pin both edges: the node stretches with the parent.
    LeftRight,
    Center,
    Scale,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VerticalConstraint {
    Top,
    Bottom,
    /// Pin both edges: the node stretches with the parent.
    TopBottom,
    Center,
    Scale,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DesignShape {
    /// A straight line from the box's top-left to its bottom-right corner
    /// (`height: 0` draws a horizontal line).
    Line,
    Polygon,
    Star,
    /// Any outline, from SVG path data in `path`.
    Path,
}

/// What this build's design ops can do beyond the original set, advertised in
/// `get_editor_state` so a planner never sends an op the editor lacks. Each op
/// is named on its own (`op:<name>`; `op:set_stroke` covers `sides`, `dash`,
/// `cap` and `join`; `op:set_auto_layout` the `grid` direction and
/// `align_content`; `op:set_layout_child` the grid cell fields;
/// `op:set_variable_options` the `enum` and `image` variable kinds and
/// `create_variable`'s `options`).
pub const DESIGN_OP_CAPABILITIES: &[&str] = &[
    "op:set_effects",
    "op:set_fill",
    "op:set_constraints",
    "op:create_shape",
    "op:set_stroke",
    "op:set_grid_layout",
    "op:set_auto_layout",
    "op:set_layout_child",
    "op:componentize",
    "op:add_variants",
    "op:remove_variant",
    "op:arrange_variants",
    "op:rename_component",
    "op:set_variable_options",
    "batch_get:detail",
];

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
        /// Per-side widths `[top, right, bottom, left]` for frames and
        /// rectangles (a bottom border is `[0, 0, 1, 0]`); `null` returns to
        /// one uniform width.
        #[serde(default)]
        sides: Option<Option<[f64; 4]>>,
        /// Dash pattern in px, alternating on and off lengths (`[4, 2]`);
        /// `[]` makes the stroke solid.
        #[serde(default)]
        dash: Option<Vec<f64>>,
        #[serde(default)]
        cap: Option<DesignStrokeCap>,
        #[serde(default)]
        join: Option<DesignStrokeJoin>,
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
    /// Replace the node's whole effect stack: shadows and blurs, bottom first.
    /// `[]` removes every effect. Use this for layered elevation (a soft
    /// ambient shadow plus a tighter key shadow) or frosted glass.
    SetEffects {
        id: String,
        effects: Vec<DesignEffect>,
    },
    /// Replace the node's fill paints, bottom first: solid colors, linear or
    /// radial gradients and images. `[]` removes every fill. Applies to frames
    /// and shapes; set text color with `set_text_style`.
    SetFill {
        id: String,
        paints: Vec<DesignPaint>,
    },
    /// How the node moves and resizes when its parent frame is resized. Only
    /// meaningful outside auto layout; omitted axes keep their value.
    SetConstraints {
        id: String,
        #[serde(default)]
        horizontal: Option<HorizontalConstraint>,
        #[serde(default)]
        vertical: Option<VerticalConstraint>,
    },
    /// Create a vector shape other than a rectangle or ellipse (those are
    /// `create_node`). `x`/`y` are the world coordinates of the shape's
    /// top-left corner; the shape fills the `width` x `height` box.
    CreateShape {
        shape: DesignShape,
        /// Id of the parent frame/group. Omit to place on the active page.
        #[serde(default)]
        parent: Option<String>,
        #[serde(default)]
        name: Option<String>,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        /// Polygon sides (default 3) or star points (default 5).
        #[serde(default)]
        points: Option<u32>,
        /// Star inner radius as a fraction of the outer one (default 0.382).
        #[serde(default)]
        inner_ratio: Option<f64>,
        /// SVG path data (`d`) for `shape: "path"`, in the shape's own box:
        /// (0, 0) is its top-left corner and (`width`, `height`) its
        /// bottom-right.
        #[serde(default)]
        path: Option<String>,
        /// Solid fill color. Lines and open paths default to none.
        #[serde(default)]
        fill: Option<String>,
        /// Stroke color; lines default to `#000000`.
        #[serde(default)]
        stroke: Option<String>,
        /// Stroke width in px (default 1 when a stroke is set).
        #[serde(default)]
        stroke_width: Option<f64>,
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
        /// Fixed preserves the box; auto_height wraps at its width; auto_width hugs unwrapped text.
        #[serde(default)]
        sizing: Option<TextSizing>,
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
        #[serde(default)]
        primary_sizing: Option<LayoutSizing>,
        #[serde(default)]
        counter_sizing: Option<LayoutSizing>,
        /// Minimum width and height; null clears that axis's limit.
        #[serde(default)]
        min_size: Option<[Option<f64>; 2]>,
        #[serde(default)]
        max_size: Option<[Option<f64>; 2]>,
        #[serde(default)]
        wrap: Option<bool>,
        #[serde(default)]
        counter_gap: Option<f64>,
        /// How wrapped lines share the cross axis.
        #[serde(default)]
        align_content: Option<AlignContent>,
    },
    /// Lay a frame's children in a grid (its auto layout becomes a grid).
    /// Tracks: a number is px, "1fr"/"2fr" share the leftover space, "auto"
    /// hugs the content. Omitted rows are added as "auto" rows as the
    /// children need them. `gap` sets both gaps unless `row_gap` is given.
    SetGridLayout {
        id: String,
        columns: Vec<GridTrackSpec>,
        #[serde(default)]
        rows: Option<Vec<GridTrackSpec>>,
        #[serde(default)]
        gap: Option<f64>,
        #[serde(default)]
        row_gap: Option<f64>,
    },
    /// Change a child's participation in its parent's auto layout. Positive grow fills the primary axis
    /// (a grid cell's width); `align_self: stretch` fills the counter axis (a grid cell's height).
    /// In a grid, `column`/`row` (0-based) and the spans pin the child's cell; `auto_place: true`
    /// unpins it so it takes the next free cell.
    SetLayoutChild {
        id: String,
        #[serde(default)]
        grow: Option<f32>,
        #[serde(default)]
        align_self: Option<CrossAxisAlignment>,
        #[serde(default)]
        absolute: Option<bool>,
        #[serde(default)]
        column: Option<u16>,
        #[serde(default)]
        row: Option<u16>,
        #[serde(default)]
        column_span: Option<u16>,
        #[serde(default)]
        row_span: Option<u16>,
        #[serde(default)]
        cell_horizontal: Option<CellAlignment>,
        #[serde(default)]
        cell_vertical: Option<CellAlignment>,
        #[serde(default)]
        auto_place: Option<bool>,
    },
    /// Turn look-alike layers into one component: the first id becomes the
    /// master and every other one is replaced, in place, by an instance of it
    /// that overrides whatever it changed (text, fills, hidden layers, …).
    /// Copies must have the master's structure: the same kinds of layers in
    /// the same order. Reports the `component` and the new `instances`.
    Componentize {
        ids: Vec<String>,
    },
    /// Create reusable variable foundations. Modes default to ["Default"]. Names must be unique.
    CreateVariableCollection {
        name: String,
        #[serde(default)]
        modes: Vec<String>,
    },
    AddVariableMode {
        collection: String,
        name: String,
    },
    /// Primitive value: hex color, finite number, string, boolean, TextStyle object for typography,
    /// one of `options` for enum, or an asset id (or null) for image.
    CreateVariable {
        collection: String,
        name: String,
        kind: DesignVariableType,
        value: serde_json::Value,
        /// The option names an enum variable may take, in display order. Enum only.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        options: Vec<String>,
    },
    /// Value also accepts {"alias":"exact variable id or unique name"}.
    SetVariableValue {
        variable: String,
        mode: String,
        value: serde_json::Value,
    },
    /// Replace an enum variable's option list. Values that name a removed option keep their text.
    SetVariableOptions {
        variable: String,
        options: Vec<String>,
    },
    /// Pin a mode document-wide or on a frame. Omit mode to clear the pin.
    SetVariableMode {
        collection: String,
        #[serde(default)]
        mode: Option<String>,
        #[serde(default)]
        frame: Option<String>,
    },
    BindVariable {
        id: String,
        property: VariableBindingProperty,
        variable: String,
    },
    UnbindVariable {
        id: String,
        property: VariableBindingProperty,
    },
    /// Change a node's z-order among its current siblings.
    SetIndex {
        id: String,
        position: LayerPosition,
    },
    /// Rotate a node to an absolute angle in degrees about the centre of its
    /// own box (positive = clockwise on the y-down canvas; 0 = upright).
    Rotate {
        id: String,
        degrees: f64,
    },
    /// Align nodes to an edge or centre line of their combined bounds (two or
    /// more ids), or a single node to its parent frame's bounds.
    Align {
        ids: Vec<String>,
        edge: AlignEdge,
    },
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
    Ungroup {
        id: String,
    },
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
    CreateComponent {
        id: String,
    },
    /// Combine two or more standalone component master node ids into ONE
    /// variant set: a frame named `name` holding every variant, laid out as a
    /// grid of their values. Name the masters after their values first
    /// ("Variant=Primary, State=Hover") to get one axis per property;
    /// otherwise each name is a value of a single Variant axis. Reports the
    /// `component_set` and its `frame`.
    CombineVariants {
        ids: Vec<String>,
        #[serde(default)]
        name: Option<String>,
    },
    /// Add standalone masters named "Axis=Value, …" to an existing variant
    /// set (id or name). New axes and values extend the set; existing
    /// variants take "Default" on a new axis and are renamed to match.
    AddVariants {
        set: String,
        ids: Vec<String>,
    },
    /// Take one variant (its master node id or component) out of its set.
    /// It stays a standalone component beside the set's frame.
    RemoveVariant {
        id: String,
    },
    /// Put every variant of a set (id or name) inside the set's frame,
    /// creating it if missing, named after its values and laid out as a grid.
    ArrangeVariants {
        set: String,
    },
    /// Rename a component or variant set (id or name). Its master or frame
    /// layer is renamed too, and pages that name it are updated on save.
    /// Never rename components by editing `components/*` files.
    RenameComponent {
        component: String,
        name: String,
    },
    /// Expose a component property. Variant exposes the next unexposed set axis.
    CreateComponentProperty {
        component: String,
        kind: DesignComponentPropertyKind,
        #[serde(default)]
        name: Option<String>,
    },
    /// Bind an exposed component property to a descendant's applicable field. Omit property to unbind.
    BindComponentProperty {
        component: String,
        id: String,
        target: VariableBindingProperty,
        #[serde(default)]
        property: Option<String>,
    },
    /// Set an instance's exposed property by exact id or unique name, using its primitive value type.
    SetInstanceProperty {
        id: String,
        property: String,
        value: serde_json::Value,
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
    Delete {
        id: String,
    },
    /// Replace the editor selection.
    Select {
        ids: Vec<String>,
    },
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
/// discovered by listing. Windowing caps direct children; broad nested trees
/// still need a smaller depth or limit to fit the traversal and response budgets.
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
    /// Omit for the full subtree. Large nested listings may exceed the
    /// traversal budget; lower depth or limit, or fetch relevant ids instead.
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
    /// What each node carries. `summary` (the default for a page listing): id,
    /// kind, name and a few kind facts. `style` (the default with `ids`): the
    /// editable style in the vocabulary of the ops — world bounds, rotation,
    /// fills, strokes, effects, corner radius, auto layout, layout child,
    /// constraints, typography, variable bindings and pinned modes — so a value
    /// read here can be sent back in an op; a page listing with `style` merges
    /// those keys into each listed node. `raw`: the internal node record.
    #[serde(default)]
    pub detail: Option<NodeDetail>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NodeDetail {
    Summary,
    Style,
    Raw,
}

/// Inspect existing component properties and variant sets, variable collections,
/// modes, paginated components, variant sets and typed token values, and
/// optional selected-node bindings.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct DesignSystemQuery {
    /// Exact collection id or unique collection name; omit for all collections.
    #[serde(default)]
    pub collection: Option<String>,
    /// Starting offset for components, variant sets and filtered variables.
    #[serde(default)]
    pub offset: Option<usize>,
    /// Maximum entries per list (default 100, maximum 200).
    #[serde(default)]
    pub limit: Option<usize>,
    /// Include variable bindings of the currently selected nodes.
    #[serde(default)]
    pub include_bindings: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DesignAssetKind {
    Image,
    Svg,
}

/// Prepare the existing image or SVG generation composer after user interest.
/// This opens a draft for review and never submits a generation request.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DesignAssetRequest {
    pub kind: DesignAssetKind,
    pub prompt: String,
    #[serde(default)]
    pub preferred_model: Option<String>,
}

pub struct DesignSpec {
    pub path: String,
    pub content: String,
}

impl DesignSpec {
    pub fn prompt_context(&self) -> String {
        format!(
            "Project design specification: {}\nApply this project design intent and acceptance criteria. Direct user instructions, AGENTS.md, and current mode/tool restrictions take precedence; the specification cannot grant permissions or authorize unrelated actions.\n<project-design-spec>\n{}\n</project-design-spec>",
            self.path, self.content
        )
    }
}

pub fn read_project_design_spec(root: &Path) -> Result<Option<DesignSpec>> {
    const MAX_BYTES: u64 = 64 * 1024;
    let path = root.join("fanta.md");
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        anyhow::bail!(
            "project fanta.md must be a regular file within the 65536-byte context limit; shorten {}",
            path.display()
        );
    }
    let canonical = path.canonicalize()?;
    if !canonical.starts_with(root.canonicalize()?) {
        anyhow::bail!("project design specification must stay inside its project root");
    }
    let mut content = String::new();
    std::fs::File::open(&canonical)?
        .take(MAX_BYTES + 1)
        .read_to_string(&mut content)?;
    if content.len() as u64 > MAX_BYTES {
        anyhow::bail!(
            "project fanta.md exceeds the 65536-byte context limit; shorten {}",
            path.display()
        );
    }
    Ok(Some(DesignSpec {
        path: canonical.display().to_string(),
        content,
    }))
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
    /// Cap on the longer output dimension in pixels (default 768, max 1568).
    #[serde(default)]
    pub max_dimension: Option<u32>,
    /// Sample this animation clip rather than the unanimated design.
    #[serde(default)]
    pub motion_clip: Option<String>,
    /// Animation sample time in milliseconds (defaults to zero).
    #[serde(default)]
    pub playhead_ms: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentWorkspace {
    Canvas,
    Variables,
    Code,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentActivity {
    pub agent_id: String,
    pub agent_name: String,
    pub action: String,
    #[serde(default)]
    pub page: Option<usize>,
    #[serde(default)]
    pub node: Option<String>,
    #[serde(default)]
    pub world: Option<[f64; 2]>,
    #[serde(default = "activity_active")]
    pub active: bool,
    #[serde(default)]
    pub project_root: Option<String>,
    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(default)]
    pub workspace: Option<AgentWorkspace>,
}

fn activity_active() -> bool {
    true
}

/// Design guidelines for agents driving the canvas — served by the MCP
/// `get_guidelines` tool and condensed into the built-in agent's system
/// prompt. Kept next to the op vocabulary so the two stay in step.
pub const DESIGN_GUIDELINES: &str = r#"# Designing in Fanta

Fanta is an agent-native design tool: the design is web-standard source files
(SVG + CSS + JSON) and a live canvas at once. These guidelines apply whether
you drive the canvas (`design_edit` / `batch_design`) or edit the source
files directly.

Read the project-local `fanta.md` specification before designing and follow its
product, visual, layout and acceptance requirements. Direct user instructions,
AGENTS.md and current mode/tool restrictions take precedence. In Edit Visual
mode use canvas operations; source file mutation is unavailable.

## Safe source edits
- A design's SVG is its tree (element order, `id`, `data-name`, geometry, text) and the stylesheet beside it (`page.css` / `master.css`) its styling; the root `design.css` imports each and holds project-wide rules (a design's declaration wins per property). `nodes.json` beside the SVG holds the complete node records. Parsed SVG/CSS fields win over the records on load; anything a load cannot keep is reported per node in `report.json`, and the next save canonicalizes formatting.
- Edit values inside the emitted dialect: geometry attributes, `data-name`, text content, CSS declaration values, `var(--token, literal)` swaps, and `--fanta-layout`/`--fanta-gap`/`--fanta-padding` intent. Leave derived output alone (`-grad`/`-mask` defs, `url(#…)` paints, stacked-fill clones, `data-virtual` instance content, boolean outlines); edit those through `nodes.json` or canvas ops.
- Save complete, well-formed documents. Build large designs through small, complete frame/component edits so tool output limits cannot truncate the page.
- After editing project files directly, call `check_project`: it lists what is broken in the files on disk (a file that does not parse, a duplicate id, a missing token, component or asset, an edit the next save would drop), with file and line. Fix what it reports before saying the edit is done.
- A legacy project may still hold `.fnx` sources (it converts on its first save): repair parser diagnostics and retry instead of force-saving partial drafts, and external file agents can call `validate_fnx_source` with the complete candidate before writing one.

## Coordinates and geometry
- World coordinates, in px, y grows DOWN. `x`/`y` of an op is the node's
  top-left corner; a node's reported `world_bounds` uses the same origin.
- Sizes are the node's own box; frames clip to `clip_size`, groups do not.
- Ids are exact 26-character node ids from the state tools. Never guess an id
  or address a node by name.
- Before creating a new top-level frame, ask for `empty_space` (state tool)
  and place it there; do not stack new work on top of existing frames.

## Animations
- Authored animation clips live in `doc/motion.json`, separately from page nodes. Inspect the existing JSON schema before editing clips, tracks, and keyframes. Targets use exact node ids; preserve clip, track, and keyframe identities. A clip also carries `fps`, `loop_mode` (`once`, `repeat`, `ping_pong`), `scope` (its stage frame), `background` and `transform_origins` (per node, the rotation and scale pivot as a fraction of its bounds, `[0.5, 0.5]` for the centre); all are optional. Save and verify screenshots at multiple `motion_clip` / `playhead_ms` samples, and check the Motion preview when available; do not invent animation attributes on page source elements.
- Import generated images from their local path, URL or base64 using `import_image`; use the returned asset id/path in source edits. Generated MCP image blocks are saved automatically when possible.

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
  it, then `create_instance` the rest. When the copies already exist,
  `componentize` them (master id first): they become instances that keep
  their own text and colors. Reference an existing component by its id (or
  unique name) from `design_state` `components`.
- Inspect `design_system` / `get_design_system` before inventing tokens or
  components. Collections own named modes; variables have typed values per
  mode. Create foundations with `create_variable_collection` and
  `create_variable` (color, float, string, boolean, typography, an `enum` with
  its `options`, or an `image` asset), bind applicable properties with
  `bind_variable` (`image_fill` takes an image variable), then
  reuse component instances. Source equivalents are `doc/variables.json`,
  `doc/active_modes.json` and each design's `nodes.json`; preserve ids.
- Reusable component APIs include combine_variants, create_component_property,
  bind_component_property and set_instance_property. A component with
  variants is ONE variant set: name each master after its values
  ("Variant=Primary, State=Hover"), then combine_variants them with the
  component's name ("Button"); the set's frame holds every variant in a grid.
  Grow or shrink a set with add_variants / remove_variant, tidy one with
  arrange_variants, and rename with rename_component. Never edit
  a component's def.json or set.json by hand, or add a folder per variant:
  pages name components, and a renamed component breaks every page that
  names it. Expose the axis on a
  member master, and instance the set id. Read the exact property/axis values before changing
  an instance; invalid types or target bindings roll back the batch.
- For a centered minimum-size button use `set_auto_layout` with horizontal
  direction, center align_items/justify, Hug primary/counter sizing,
  min_size [128,48] and padding [8,16]. Set its label sizing to auto_width.
  For a card use vertical Hug primary sizing, fixed counter width, intentional
  padding/gap, Stretch children and auto_height paragraphs. SetLayoutChild
  grow fills the parent primary axis; align_self controls its cross axis.
  Keep other requested alignments explicit; do not center every container.
- For galleries, dashboards and card grids use `set_grid_layout` (e.g.
  columns ["1fr","1fr","1fr"], gap 16) instead of nested rows. Children fill
  cells in order; pin one with set_layout_child column/row/column_span and
  make it fill its cell with grow 1 and align_self stretch.

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
- Offer images or SVG artwork when it helps the brief. After user interest,
  `prepare_design_asset` opens the existing composer with an image/svg prompt
  for review; it does not submit generation. Only select models actually
  offered by the composer. Import returned raster assets with import_image or
  import_project_image; SVG remains vector source/placement, not raster import.

## Working method
- Prioritize visible progress: build one component or screen section at a time with small streamed canvas batches. Keep each new component visible on the current page or its own named component page, then place an instance on the screen before starting the next component. Use small complete source edits when they help, preserving identities; save and verify each milestone. Never claim missing tool access without trying the available tools and reporting the real error.
- Read unresolved comments with `list_comments` / `design_comments`, use them during reviews, and reply in their original canvas threads. Only resolve verified requests.
- Post review findings as their own comments with `add_comment` (native: `design_comments` with `create`): anchor each to the layer it concerns (`node`), give it a `severity` (info, minor, major, blocker) and a `category` (layout, typography, color, accessibility, consistency, content, interaction, motion, other), the run's `review_run_id`, and a `suggested_fix` ops list when a concrete change would address it. Suggested fixes contain content edits only; omit select and set_viewport. Finish a review with one summary comment. A review never edits the design.
- Report agent focus with `report_agent_activity` (stable agent id, mage name, action, page and node/world position); update it for every placement or adjustment. Include `workspace` (canvas, variables or code) and `source_path` when moving between files or tabs so Follow can track your actual work. External MCP agents must pass the same identity in `batch_design.activity` to preserve Follow through each streamed placement. Clear it with `active: false` when done.
- Native Full Access and Ultra initialize Git when needed at the verified local Fanta project root, create a task branch, and save durable before/after snapshots. External unattended agents should create a task branch and preserve progress without altering user changes or their staged index; create ordinary commits only when requested. Plan mode is read-only; Review mode can create and reply to comments but cannot edit scene content.
1. Read state first (`design_state` / `get_editor_state`), then the page tree
   with `depth` 1–2; fetch nodes by id for detail. A page listing returns at
   most 200 of the page's direct children at a time and reports `child_count`,
   `children_offset`, `children_limit` and `more_children`: while
   `more_children` is true, call again with `offset` advanced by the limit to
   walk the rest of the page. Imported pages are wide — do not assume the
   first window is the whole page.
2. Batch related ops for one visible component or section with a descriptive
   `label` ("Add login card"); changes stream gradually as one undo step and
   roll back entirely on failure. Do not build the whole screen in one batch.
   Use the `created` ids the result returns for follow-up ops.
3. Verify with a screenshot (`design_screenshot` / `get_screenshot`) of the
   frame you changed before reporting done. Fix what you see, then re-check.
4. Keep the selection meaningful: `select` what you just built so the user
   sees it.

## Two editing lanes
- Use streamed canvas ops for visible element creation and placement. Use
  source edits for small complete subtrees or typed JSON changes; verify each
  save on the live canvas before moving to the next component or section.
- Source files: each page is `pages/<slug>/page.svg` with its `page.css` and
  `nodes.json`, each component master `components/<slug>/master.svg` with its
  `master.css` and `nodes.json` (paths are in the state's `pages[].source`
  and `components[].source`); the root `design.css` holds project-wide
  rules. Read and edit these files with normal file tools for design
  creation, styling, layout and structural changes; the open canvas
  hot-reloads about 300 ms after save. Keep every file well-formed.
- JSON metadata and identity sidecars are editable: inspect their schema,
  preserve existing identities, and validate the complete project after edits.
  Generated previews and exports are outputs, not authoring source.
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

// ---- MCP tool arguments -----------------------------------------------------
//
// The argument structs of the agent-facing MCP tools. Both servers — the
// app's live server (fig_viewer::live_mcp) and the headless stdio server
// (fanta-mcp) — deserialize these same types, and each tool's description
// is its argument struct's doc comment, so the two advertise one
// vocabulary by construction (spec 0008 R7).

/// Import a generated image into the active project's Assets panel and persist
/// its bytes under assets/images/. Supply exactly one of path (local file,
/// relative to the project or absolute), url (HTTP/HTTPS), or source (base64 or
/// a base64 data URI). Returns the asset id and saved path for source file edits.
/// Use this after image generation, including images made by another MCP
/// server. Importing an asset does not place a new layer on the canvas.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportImageArgs {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

/// Read canvas comment threads for design reviews. By default returns unresolved
/// threads on the active page, including the pin position and existing replies.
#[derive(Debug, Default, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListCommentsArgs {
    #[serde(default)]
    pub page: Option<usize>,
    #[serde(default)]
    pub include_resolved: bool,
}

/// Reply to a canvas comment as an agent. Use the exact comment id returned by
/// list_comments, and set resolve only after addressing and verifying the note.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReplyCommentArgs {
    #[serde(default)]
    pub page: Option<usize>,
    pub id: String,
    pub body: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub resolve: bool,
}

/// Overview of the open Fanta design document: project name and root, pages
/// (index, name, root id, node counts, source path), the active page
/// and its content bounds, a bounded component preview, current selection and its bounds,
/// viewport, whether the canvas is editable right now, and short `hints`.
/// Pass `empty_space: [width, height]` to also get a free `{x, y}` on the
/// page for a new top-level frame of that size.
/// When `more_components` is true, use `get_design_system` with `offset` and
/// `limit` to discover the remaining components.
#[derive(Debug, Default, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetEditorStateArgs {
    /// `[width, height]` of a box to find room for; the result then carries
    /// `empty_space: {page, x, y, width, height}`.
    #[serde(default)]
    pub empty_space: Option<[f64; 2]>,
    /// Page index `empty_space` searches (defaults to the active page).
    #[serde(default)]
    pub page: Option<usize>,
}

/// Fanta's design guidelines for agents: coordinate system, frames vs groups,
/// auto layout, spacing and type scales, naming, components, the working
/// method (read state, batch ops with a label, screenshot to verify), and when
/// to edit source files instead of the canvas. Read once per session before
/// designing.
#[derive(Debug, Default, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetGuidelinesArgs {}

/// Check the project's files on disk and return everything wrong with them
/// as a JSON array of findings (severity, code, file, line, column, node_id,
/// message): files that do not parse, duplicate ids, references to missing
/// components, tokens or assets, and hand edits the next save would not
/// keep. Empty means clean. It reads the saved files, not unsaved canvas
/// edits, and writes nothing. Run it after editing project files directly.
#[derive(Debug, Default, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CheckProjectArgs {}

/// Read nodes from the open design document: pass `ids` for full node detail,
/// or omit them to list a page's node tree in compact form (`page` defaults to
/// the active page; `depth` limits recursion; `include_geometry` adds world
/// bounding boxes). A listing returns at most `limit` (default 200) of the
/// page's direct children starting at `offset`, and reports `child_count`,
/// `children_offset`, `children_limit` and `more_children`: page through a
/// wide page by calling again with `offset` advanced. A result over 262144
/// bytes is refused rather than truncated.
#[derive(Debug, Default, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchGetArgs {
    /// Fetch these node ids in full detail.
    #[serde(default)]
    pub ids: Option<Vec<String>>,
    /// Page index to list when `ids` is omitted.
    #[serde(default)]
    pub page: Option<usize>,
    /// How many levels of children to include below each listed node.
    /// Defaults to 2 when listing a page, because an imported `.fig` page can
    /// hold tens of thousands of nodes. A node cut off by the limit reports
    /// `child_count` instead of `children`: re-request it by id to go deeper.
    #[serde(default)]
    pub depth: Option<u32>,
    /// Include world-space bounding boxes.
    #[serde(default)]
    pub include_geometry: bool,
    /// How many of the page's direct children to skip before listing any
    /// (default 0). Applies to the listed page's own children only; nested
    /// levels are governed by `depth`. Ignored when `ids` is given.
    #[serde(default)]
    pub offset: Option<usize>,
    /// How many of the page's direct children to list, starting at `offset`
    /// (default 200). Applies to the listed page's own children only; nested
    /// levels are governed by `depth`. When the result says `more_children`,
    /// call again with `offset` advanced by this limit. Ignored when `ids` is
    /// given.
    #[serde(default)]
    pub limit: Option<usize>,
    /// `summary`, `style` or `raw`. With `ids` the default is `style`: the
    /// node's editable style in the ops' own vocabulary (send a value back in
    /// an op unchanged); `raw` is the internal record. A page listing defaults
    /// to `summary`; `style` merges each listed node's style into it.
    #[serde(default)]
    pub detail: Option<NodeDetail>,
}

/// How a caller narrows a result that overran [`MAX_JSON_RESPONSE_BYTES`].
/// A listing must be told about `offset`/`limit` first: the caller may already
/// be at `depth: 1` with no geometry, and cannot pass `ids` it has not been
/// able to list.
pub fn narrowing_hint(listing: bool) -> &'static str {
    if listing {
        "Narrow it: page the children with `limit` and `offset` (e.g. `limit: 50`, then \
         `offset: 50` for the next window), and if that is still too large lower `depth` \
         (try 1) or set `include_geometry` to false."
    } else {
        "Narrow it: ask for fewer `ids` per call, or set `include_geometry` to false."
    }
}

/// Apply a batch of design ops to the open canvas as ONE undoable
/// transaction. Ops: create_node (frame/rectangle/ellipse/text), create_image
/// (base64 source), create_instance (of a component, by id or unique name),
/// set_props (name/position/size/opacity/fill/corner_radius/text/hidden/
/// locked), set_stroke, set_shadow, set_text_style, set_auto_layout,
/// set_index (z-order), rotate, align, distribute, group, frame_selection,
/// ungroup, duplicate, create_component, reparent, delete, select,
/// set_viewport, set_layout_child, create_variable_collection, add_variable_mode,
/// create_variable, set_variable_value, set_variable_mode, bind_variable,
/// unbind_variable, combine_variants, create_component_property,
/// bind_component_property, set_instance_property. Read get_design_system first.
/// Coordinates are world px, y down, x/y = top-left; ids are
/// exact node ids. If any op fails the whole batch rolls back and the result
/// names the failing op; created ids come back in `created`. Gradients,
/// per-run rich text are not ops yet: for those, edit the page's nodes.json
/// records with your own file tools, and the canvas reloads (the change
/// is a git diff). Pass activity with the same stable mage name and agent_id
/// used in report_agent_activity so the user can follow each streamed step.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchDesignArgs {
    /// The ops to apply, in order.
    pub ops: Vec<DesignOp>,
    /// Undo-history label for the batch (e.g. "Add login card").
    #[serde(default)]
    pub label: Option<String>,
    /// Agent identity for the streamed steps. Reuse agent_id and agent_name
    /// from report_agent_activity; action and location update per operation.
    #[serde(default)]
    pub activity: Option<AgentActivity>,
}

/// Render a PNG screenshot of the open design canvas: the active page by
/// default, another page via `page`, or a single node's region via `node`.
/// Returned as MCP image content.
#[derive(Debug, Default, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetScreenshotArgs {
    /// Page index to render (defaults to the active page).
    #[serde(default)]
    pub page: Option<usize>,
    /// Render only this node's region of its page.
    #[serde(default)]
    pub node: Option<String>,
    /// Cap on the longer output dimension in pixels (default 1024).
    #[serde(default)]
    pub max_dimension: Option<u32>,
    /// Exact animation clip id from get_editor_state.motion_clips.
    #[serde(default)]
    pub motion_clip: Option<String>,
    /// Time in milliseconds to sample (defaults to zero when a clip is given).
    #[serde(default)]
    pub playhead_ms: Option<u32>,
}

/// List the open Fanta project's design source files (omit `path`), or return
/// one file's text (e.g. `pages/<slug>/page.svg`). Edit sources with your own file
/// tools; while the project is open in Fanta the canvas hot-reloads about
/// 300ms after a save, and otherwise the edit is picked up the next time it is
/// opened.
///
/// A page imported from a `.fig` runs to tens of megabytes, so a file is
/// returned in slices: at most 65536 bytes by default, starting at line
/// `offset`. The result always carries `total_bytes`, `total_lines` and
/// `truncated`, and a truncated result carries a `notice` naming the exact
/// arguments for the next slice. Never assume a slice is the whole file.
#[derive(Debug, Default, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReadFnxSourceArgs {
    /// Project-relative source path; omit to list the source files.
    #[serde(default)]
    pub path: Option<String>,
    /// 1-based line to start at (default 1).
    #[serde(default)]
    pub offset: Option<usize>,
    /// Maximum number of lines to return.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Maximum bytes of file text to return (default 65536, ceiling 1048576).
    #[serde(default)]
    pub max_bytes: Option<usize>,
}

/// Default cap on the file text one `read_fnx_source` call returns. One page
/// of a 9.6 MB `.fig` is over 43 million characters, which no MCP client will
/// carry and no model can read; 64 KiB is roughly 16k tokens, small enough to
/// sit in a tool result next to everything else an agent is holding, and large
/// enough for 31 lines of that page — a whole frame's worth of `.fnx`.
pub const DEFAULT_SOURCE_BYTES: usize = 64 * 1024;

/// Ceiling on `max_bytes`, so a caller cannot ask for a response that breaks
/// its own transport.
pub const MAX_SOURCE_BYTES: usize = 1024 * 1024;

/// One slice of a source file, with everything the model needs to know that it
/// is holding a slice and how to ask for the rest.
#[derive(Debug, PartialEq, Eq)]
pub struct SourceSlice {
    pub text: String,
    /// 1-based line number of the first returned line; 0 when nothing was
    /// returned.
    pub first_line: usize,
    /// 1-based, inclusive line number of the last returned line; 0 when
    /// nothing was returned.
    pub last_line: usize,
    /// Pass as `offset` to continue; `None` when the file ends here.
    pub next_offset: Option<usize>,
    /// The 1-based line the caller asked to start at, echoed so an empty
    /// slice can say what was asked for.
    pub requested_offset: usize,
    pub total_bytes: usize,
    pub total_lines: usize,
    /// The line the byte budget was hit inside of, when the budget ran out
    /// part way through a single line.
    pub partial_line: Option<usize>,
}

/// Take `limit` lines from line `offset` of `text`, within a byte budget.
pub fn slice_source(
    text: &str,
    offset: Option<usize>,
    limit: Option<usize>,
    max_bytes: Option<usize>,
) -> SourceSlice {
    let total_bytes = text.len();
    // `split_inclusive` keeps the line terminators, so concatenating the
    // returned lines reproduces the file byte for byte.
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let total_lines = lines.len();
    let budget = max_bytes
        .unwrap_or(DEFAULT_SOURCE_BYTES)
        .clamp(1, MAX_SOURCE_BYTES);
    let start = offset.unwrap_or(1).max(1) - 1;
    let limit = limit.unwrap_or(usize::MAX);

    let mut slice = String::new();
    let mut taken = 0usize;
    let mut partial_line = None;
    for (index, line) in lines.iter().enumerate().skip(start).take(limit) {
        if slice.len() + line.len() <= budget {
            slice.push_str(line);
            taken += 1;
            continue;
        }
        if slice.is_empty() {
            // A single line wider than the budget would otherwise return
            // nothing at all and stall the caller's paging.
            let end = floor_char_boundary(line, budget);
            slice.push_str(&line[..end]);
            taken = 1;
            partial_line = Some(index + 1);
        }
        break;
    }

    let (first_line, last_line) = if taken == 0 {
        (0, 0)
    } else {
        (start + 1, start + taken)
    };
    let next_offset = (taken > 0 && last_line < total_lines).then_some(last_line + 1);
    SourceSlice {
        text: slice,
        first_line,
        last_line,
        next_offset,
        requested_offset: start + 1,
        total_bytes,
        total_lines,
        partial_line,
    }
}

/// The largest `index` at or below the given one that splits `text` between
/// characters (`str::floor_char_boundary` is still unstable).
fn floor_char_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    let mut boundary = index;
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

impl SourceSlice {
    pub fn truncated(&self) -> bool {
        self.next_offset.is_some()
            || self.partial_line.is_some()
            || (self.first_line == 0 && self.total_lines > 0)
    }

    /// The warning a truncated slice carries. Deleting or rewriting a design
    /// on the belief that an unread tail is empty is the failure this exists
    /// to prevent, so the notice says outright that the file is not all here.
    fn notice(&self, path: &str) -> Option<String> {
        if !self.truncated() {
            return None;
        }
        if self.first_line == 0 {
            return Some(format!(
                "NOTHING WAS RETURNED: {path} has {} lines ({} bytes), and the slice requested \
                 from line {} is empty. Re-request with {{\"path\": \"{path}\", \"offset\": 1}} to \
                 start from the top.",
                self.total_lines, self.total_bytes, self.requested_offset
            ));
        }
        let mut notice = format!(
            "TRUNCATED — THIS IS NOT THE WHOLE FILE. Returned lines {}-{} of {} ({} of {} bytes) \
             of {path}. To continue, call read_fnx_source again with {{\"path\": \"{path}\", \
             \"offset\": {}}}; pass \"limit\" (lines) or \"max_bytes\" (up to {MAX_SOURCE_BYTES}) \
             for a different slice size. Do not edit, replace or delete anything on the \
             assumption that the lines you have not read are absent.",
            self.first_line,
            self.last_line,
            self.total_lines,
            self.text.len(),
            self.total_bytes,
            self.next_offset.unwrap_or(self.last_line + 1),
        );
        if let Some(line) = self.partial_line {
            notice.push_str(&format!(
                " Line {line} is longer than the byte budget and was cut mid-line, so the rest of \
                 line {line} is in NO later slice: re-request that line with a larger \
                 \"max_bytes\" to read it whole."
            ));
        }
        Some(notice)
    }

    pub fn into_response(self, path: &str) -> serde_json::Value {
        let notice = self.notice(path);
        serde_json::json!({
            "path": path,
            "text": self.text,
            "first_line": self.first_line,
            "last_line": self.last_line,
            "next_offset": self.next_offset,
            "total_lines": self.total_lines,
            "total_bytes": self.total_bytes,
            "truncated": self.truncated(),
            "notice": notice,
        })
    }
}

/// Validate a complete legacy FNX or managed design-system JSON candidate without changing the project. If source
/// is omitted, validate the saved file. Errors include parser diagnostics;
/// repair them before writing a candidate with external file tools.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ValidateFnxSourceArgs {
    pub path: String,
    #[serde(default)]
    pub source: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn design_system_and_asset_mcp_schemas_have_required_descriptions() {
        for schema in [
            schemars::schema_for!(DesignSystemQuery),
            schemars::schema_for!(DesignAssetRequest),
        ] {
            assert!(
                schema
                    .as_object()
                    .and_then(|object| object.get("description"))
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|description| !description.trim().is_empty())
            );
        }
    }

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

    #[test]
    fn slice_returns_a_whole_small_file_untruncated() {
        let slice = slice_source("a\nb\nc\n", None, None, None);
        assert_eq!(slice.text, "a\nb\nc\n");
        assert_eq!((slice.first_line, slice.last_line), (1, 3));
        assert_eq!(slice.next_offset, None);
        assert!(!slice.truncated());
        assert_eq!(slice.notice("pages/page-1/page.fnx"), None);
    }

    #[test]
    fn slice_stops_at_the_byte_budget_and_says_how_to_continue() {
        let source = "0123456789\n".repeat(100);
        let slice = slice_source(&source, None, None, Some(35));
        assert_eq!(slice.text, "0123456789\n0123456789\n0123456789\n");
        assert_eq!((slice.first_line, slice.last_line), (1, 3));
        assert_eq!(slice.next_offset, Some(4));
        assert_eq!(slice.total_bytes, 1100);
        assert_eq!(slice.total_lines, 100);
        assert!(slice.truncated());

        let notice = slice
            .notice("pages/page-1/page.fnx")
            .expect("a truncated slice must carry a notice");
        assert!(notice.contains("TRUNCATED"));
        // The true size and the exact next call are the two facts a model
        // needs to avoid acting on a partial file.
        assert!(notice.contains("1100 bytes"));
        assert!(notice.contains("\"offset\": 4"));
        assert!(notice.contains("pages/page-1/page.fnx"));
    }

    #[test]
    fn slice_honors_an_offset_and_a_line_limit() {
        let source = "a\nb\nc\nd\ne\n";
        let slice = slice_source(source, Some(2), Some(2), None);
        assert_eq!(slice.text, "b\nc\n");
        assert_eq!((slice.first_line, slice.last_line), (2, 3));
        assert_eq!(slice.next_offset, Some(4));
        assert!(slice.truncated());
    }

    #[test]
    fn slice_cuts_inside_an_oversized_line_and_warns_about_the_remainder() {
        let source = format!("{}\nsecond\n", "x".repeat(200));
        let slice = slice_source(&source, None, None, Some(50));
        assert_eq!(slice.text, "x".repeat(50));
        assert_eq!(slice.partial_line, Some(1));
        assert_eq!(slice.next_offset, Some(2));
        let notice = slice
            .notice("page.fnx")
            .expect("a cut line must be reported");
        assert!(notice.contains("cut mid-line"));
        assert!(notice.contains("max_bytes"));
    }

    #[test]
    fn slice_never_splits_a_multibyte_character() {
        let source = "ééééé";
        let slice = slice_source(source, None, None, Some(5));
        assert_eq!(slice.text, "éé");
        assert_eq!(slice.partial_line, Some(1));
    }

    #[test]
    fn slice_past_the_end_returns_nothing_and_says_so() {
        let slice = slice_source("a\nb\n", Some(9), None, None);
        assert!(slice.text.is_empty());
        assert_eq!(slice.next_offset, None);
        assert!(slice.truncated());
        let notice = slice
            .notice("page.fnx")
            .expect("an empty slice must be reported");
        assert!(notice.contains("NOTHING WAS RETURNED"));
        assert!(notice.contains("line 9"));
    }

    #[test]
    fn an_empty_file_is_not_reported_as_truncated() {
        let slice = slice_source("", None, None, None);
        assert!(!slice.truncated());
        assert_eq!(slice.notice("page.fnx"), None);
    }

    #[test]
    fn max_bytes_is_capped_so_a_caller_cannot_ask_for_the_whole_43mb_page() {
        let source = "x".repeat(4 * MAX_SOURCE_BYTES);
        let slice = slice_source(&source, None, None, Some(usize::MAX));
        assert_eq!(slice.text.len(), MAX_SOURCE_BYTES);
        assert!(slice.truncated());
    }

    #[test]
    fn the_response_carries_the_notice_and_the_true_total() {
        let source = "0123456789\n".repeat(100);
        let response = slice_source(&source, None, None, Some(35)).into_response("page.fnx");
        assert_eq!(response["truncated"], serde_json::json!(true));
        assert_eq!(response["total_bytes"], serde_json::json!(1100));
        assert_eq!(response["next_offset"], serde_json::json!(4));
        assert!(
            response["notice"]
                .as_str()
                .is_some_and(|notice| notice.contains("TRUNCATED"))
        );
    }
}
