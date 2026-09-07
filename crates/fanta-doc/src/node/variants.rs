//! The concrete variant payloads carried by a [`NodeData`](crate::node::NodeData):
//! containers ([`GroupNode`]), shapes ([`VectorNode`]), text
//! ([`TextNode`]/[`TextStyle`]), media ([`BitmapNode`], [`VideoNode`],
//! [`AudioNode`]), embedded graphs ([`NodeGraphNode`]), 3D ([`Model3dNode`]), AI
//! ([`AiArtifactNode`]), and the forward-compat [`EmbedNode`].

use crate::color::Color;
use crate::id::{AssetId, LinkId, ModeId, VariableCollectionId, WorkflowNodeId};
use crate::node::layout::AutoLayout;
use crate::path::PathData;
use crate::serde_util::is_false;
use crate::style::{Fill, Stroke};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::collections::BTreeMap;

/// A pure container. Children are linked back via their `parent` field.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GroupNode {
    /// Explicit box for a non-clipping organizational group. Frames keep their
    /// box in `clip_size`; this separate extent lets the editor resize a plain
    /// group without introducing clipping or baking scale into the group's
    /// transform (which would stretch every descendant).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_size: Option<[f64; 2]>,
    /// When `Some`, restricts rendering of descendants to this rect. None =
    /// no clipping (Figma frames vs groups, modeled with the same variant).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip_size: Option<[f64; 2]>,
    /// Optional background paint (turns this into a Figma-style frame).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<Fill>,
    /// Additional frame/page background paints stacked above `background`.
    /// Figma frames can carry multiple fills; keeping the first in
    /// `background` preserves the older common path while this field keeps the
    /// rest of the paint stack without turning them into editable child layers.
    #[serde(default, skip_serializing_if = "SmallVec::is_empty")]
    pub background_fills: SmallVec<[Fill; 1]>,
    /// Per-frame theme pin: which mode is forced for a given variable
    /// collection within this subtree. The nearest ancestor with an entry for
    /// a collection wins during mode resolution (see
    /// [`crate::resolve::resolve_effective_mode`]) — that's how a single frame
    /// can render "Dark" while the rest of the doc is "Light". Empty ⇒ no pin,
    /// so old files round-trip byte-identical.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub explicit_modes: BTreeMap<VariableCollectionId, ModeId>,

    /// Basic support for scrollable containers (used by prototype ScrollTo).
    /// When true, children can be scrolled within the clip_size (if present).
    /// Superseded by [`scroll_direction`](Self::scroll_direction) — kept (and
    /// still always serialized) so pre-existing docs round-trip byte-identical
    /// and older readers keep working. Read through
    /// [`effective_scroll_direction`](Self::effective_scroll_direction), never
    /// directly.
    #[serde(default)]
    pub scrollable: bool,
    /// Which axes prototype scrolling moves content along (Figma
    /// `scrollDirection` / REST `overflowDirection`). `None` (absent) ⇒ fall
    /// back to the legacy [`scrollable`](Self::scrollable) bool — so old docs
    /// round-trip byte-identical while imports carry the real authored axes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll_direction: Option<ScrollDirection>,
    /// Authored initial scroll offset `[x, y]` (Figma `scrollOffset`): the
    /// content starts pre-scrolled by this much when the frame is presented.
    /// Additive — absent on old docs, skipped when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll_offset: Option<[f64; 2]>,
    /// Auto-layout (Figma flexbox "stack") configuration when this frame is an
    /// auto-layout container. `None` for plain groups/frames whose children sit
    /// at fixed positions. A future layout pass (Stage 2) reads this to flow,
    /// space, pad, align, and (re)size the frame's direct children — see
    /// [`AutoLayout`]. Absent ⇒ `None` ⇒ old files round-trip byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_layout: Option<AutoLayout>,
    /// Stacked frame strokes (the frame's border), bottom-first. A Figma FRAME /
    /// SECTION can carry a border just like a shape; it sits on the frame's
    /// `clip_size` box, rounded to match the frame's corners. Empty ⇒ no border,
    /// so old files round-trip byte-identical. Parallel to [`VectorNode::strokes`].
    #[serde(default, skip_serializing_if = "SmallVec::is_empty")]
    pub strokes: SmallVec<[Stroke; 1]>,
    /// Uniform corner radius for the frame's box (background + border rounding).
    /// Mirrors [`VectorNode::corner_radius`]: `None` ⇒ square frame, so old files
    /// round-trip byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<f64>,
    /// Independent per-corner radii `[top-left, top-right, bottom-right, bottom-left]`
    /// for the frame's box. When `Some`, takes precedence over `corner_radius`
    /// (mixed-corner cards/panels). Mirrors [`VectorNode::corner_radii`]; absent
    /// ⇒ old files round-trip byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_radii: Option<[f64; 4]>,
    /// Corner smoothing 0..=1 (Figma's "squircle" amount): `0` = a plain circular
    /// rounded corner, higher values blend toward a continuous-curvature
    /// superellipse. `0` ⇒ old files round-trip byte-identical.
    #[serde(default, skip_serializing_if = "is_zero_smoothing")]
    pub corner_smoothing: f32,
}

/// A boolean-operation container. Its geometry is the fold of its child
/// *operands* under [`op`](BooleanNode::op), painted with this node's own
/// `fills` / `strokes`. The operands are ordinary child nodes in the scene (like
/// a [`GroupNode`]); the renderer materializes each child's outline to a path
/// and folds them with path-ops. This is what a Figma UNION / SUBTRACT /
/// INTERSECT / EXCLUDE node imports to.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BooleanNode {
    /// Which set operation folds the operands.
    #[serde(default)]
    pub op: BooleanOp,
    /// Stacked fills painted on the folded geometry, bottom-first. Parallel to
    /// [`VectorNode::fills`]. Empty ⇒ unfilled outline.
    #[serde(default, skip_serializing_if = "SmallVec::is_empty")]
    pub fills: SmallVec<[Fill; 1]>,
    /// Stacked strokes painted on the folded geometry, bottom-first. Parallel to
    /// [`VectorNode::strokes`]. Empty ⇒ unstroked.
    #[serde(default, skip_serializing_if = "SmallVec::is_empty")]
    pub strokes: SmallVec<[Stroke; 1]>,
}

/// Horizontal constraint (Figma style) for children of non-auto-layout parents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintH {
    #[default]
    Left,
    Right,
    LeftRight,
    Center,
    Scale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintV {
    #[default]
    Top,
    Bottom,
    TopBottom,
    Center,
    Scale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Constraints {
    pub horizontal: ConstraintH,
    pub vertical: ConstraintV,
}

/// The four boolean set operations (Figma's UNION / SUBTRACT / INTERSECT /
/// EXCLUDE). [`Subtract`](BooleanOp::Subtract) is the first operand minus the
/// union of the rest; [`Exclude`](BooleanOp::Exclude) is the symmetric
/// difference (XOR).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BooleanOp {
    #[default]
    Union,
    Subtract,
    Intersect,
    Exclude,
}

fn is_zero_smoothing(v: &f32) -> bool {
    *v == 0.0
}

/// Which axes prototype scrolling moves a frame's content along — Figma's
/// `scrollDirection` (REST: `overflowDirection`). Content only moves when the
/// frame clips and its content overflows the clip on an allowed axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrollDirection {
    /// No prototype scrolling (Figma's default for every frame).
    None,
    Horizontal,
    Vertical,
    Both,
}

impl ScrollDirection {
    pub fn allows_x(self) -> bool {
        matches!(self, Self::Horizontal | Self::Both)
    }

    pub fn allows_y(self) -> bool {
        matches!(self, Self::Vertical | Self::Both)
    }
}

/// How a node behaves while an ancestor frame scrolls — Figma's per-child
/// `scrollBehavior`. Lives on [`CanvasNode`](crate::node::CanvasNode) (any
/// node kind can be fixed/sticky), default [`Scrolls`](Self::Scrolls) so old
/// docs round-trip byte-identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrollBehavior {
    /// Moves with the scrolled content. The default.
    #[default]
    Scrolls,
    /// Stays put while content scrolls under it (Figma "Fixed" — headers,
    /// tab bars, FABs).
    Fixed,
    /// Scrolls until it reaches the container's leading edge, then pins
    /// (Figma "Sticky").
    Sticky,
}

impl ScrollBehavior {
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Scrolls)
    }
}

impl GroupNode {
    /// The scroll axes this frame actually honors: the authored
    /// [`scroll_direction`](Self::scroll_direction) when present, else the
    /// legacy [`scrollable`](Self::scrollable) bool widened to
    /// [`ScrollDirection::Both`]. Callers (present runtime, hit-testing)
    /// read THIS, never the raw fields.
    pub fn effective_scroll_direction(&self) -> ScrollDirection {
        self.scroll_direction.unwrap_or(if self.scrollable {
            ScrollDirection::Both
        } else {
            ScrollDirection::None
        })
    }

    /// Whether this group is a *frame surface* — a real backing rect (a clip
    /// box and/or a background paint) that should catch clicks on its body,
    /// like a Figma FRAME. A plain organizational group (no clip, no
    /// background) is click-through: selection falls to whichever child sits
    /// under the cursor. Hit-testing uses this to let frames be clicked and
    /// dragged by their body while keeping bare groups transparent to picks.
    pub fn is_frame_surface(&self) -> bool {
        self.clip_size.is_some() || self.background.is_some() || !self.background_fills.is_empty()
    }
}

/// A parametric shape descriptor: a few scrubable parameters that regenerate
/// [`VectorNode::path`], so a designer can change a star's point count or an
/// arc's sweep *non-destructively* (Figma's `arcData` / star / polygon
/// properties). The `path` is always kept as the tessellated result the
/// renderer draws; this descriptor lets the editor rebuild it via
/// [`to_path`](ParametricShape::to_path). `None` ⇒ a plain authored path (the
/// overwhelmingly common case), skipped from serialization so old docs
/// round-trip byte-identical.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum ParametricShape {
    /// Ellipse arc / pie / donut. Angles in radians (0 = +x, clockwise);
    /// `inner_ratio` in 0..=1 (`0` = solid pie, `> 0` = a ring/donut).
    Arc {
        start_rad: f64,
        sweep_rad: f64,
        inner_ratio: f64,
    },
    /// Regular star: `points` tips, notches at `inner_ratio` of the radii.
    Star { points: u32, inner_ratio: f64 },
    /// Regular convex polygon with `points` sides.
    Polygon { points: u32 },
}

impl ParametricShape {
    /// Regenerate the path this descriptor produces, in the box `[0, 0, w, h]`.
    pub fn to_path(&self, w: f64, h: f64) -> PathData {
        match *self {
            Self::Arc {
                start_rad,
                sweep_rad,
                inner_ratio,
            } => PathData::ellipse_arc(w * 0.5, h * 0.5, start_rad, sweep_rad, inner_ratio),
            Self::Star {
                points,
                inner_ratio,
            } => PathData::star(w, h, points, inner_ratio),
            Self::Polygon { points } => PathData::polygon(w, h, points),
        }
    }
}

/// Vector shape — one path with stacked fills and stacked strokes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VectorNode {
    pub path: PathData,
    /// Stacked fills, bottom-first. Empty = unfilled outline.
    #[serde(default, skip_serializing_if = "SmallVec::is_empty")]
    pub fills: SmallVec<[Fill; 1]>,
    /// Stacked strokes, bottom-first. Empty = unstroked fill.
    #[serde(default, skip_serializing_if = "SmallVec::is_empty")]
    pub strokes: SmallVec<[Stroke; 1]>,
    /// Uniform corner radius applied to rectangle-shaped paths only. Stored
    /// here, not baked into `path`, so designers can scrub it non-destructively.
    /// Ignored when the path is not a rectangle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_radius: Option<f64>,
    /// Independent per-corner radii `[top-left, top-right, bottom-right, bottom-left]`
    /// for rectangle-shaped paths (mixed-corner cards, tabs, segmented controls).
    /// When `Some`, it takes precedence over `corner_radius` at render time; when
    /// `None`, the uniform `corner_radius` (if any) is used. Kept separate from
    /// `corner_radius` for back-compat: old docs round-trip byte-identical because
    /// this field is skipped when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_radii: Option<[f64; 4]>,
    /// Corner smoothing 0..=1 (Figma's "squircle" amount) applied together with
    /// `corner_radius`/`corner_radii` on rectangle-shaped paths. Mirrors
    /// [`GroupNode::corner_smoothing`]; `0` ⇒ plain circular corners and is
    /// skipped, so old docs round-trip byte-identical.
    #[serde(default, skip_serializing_if = "is_zero_smoothing")]
    pub corner_smoothing: f32,
    /// The vector's own viewport box `[width, height]` in local coordinates,
    /// analogous to an SVG `viewBox`. When `Some`, rendering is clipped to
    /// `[0, 0, w, h]` so geometry that spills past the box — most commonly a
    /// stroke thickened well beyond the authored size — is cropped instead of
    /// growing the visible shape (SVG viewport semantics). `None` (old docs,
    /// tool-created shapes) means no clip, and the field is skipped on
    /// serialization so those docs round-trip byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_size: Option<[f64; 2]>,
    /// Optional parametric descriptor that regenerates `path` (arc / star /
    /// polygon). `None` (authored paths, most shapes) is skipped, so old docs
    /// round-trip byte-identical; when `Some`, `path` is the cached tessellation
    /// and the editor rebuilds it via [`ParametricShape::to_path`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parametric: Option<ParametricShape>,
}

impl VectorNode {
    /// Filled rectangle with no stroke.
    pub fn rect_solid(x: f64, y: f64, w: f64, h: f64, color: Color) -> Self {
        Self {
            path: PathData::rect(x, y, w, h),
            fills: SmallVec::from_iter([Fill::solid(color)]),
            strokes: SmallVec::new(),
            corner_radius: None,
            corner_radii: None,
            corner_smoothing: 0.0,
            // Authored at (x, y), not the local origin, so a `[0,0,w,h]` viewport
            // clip would misalign. Solid rects never overflow their box anyway.
            local_size: None,
            parametric: None,
        }
    }
}

/// How a [`TextNode`] resizes its box to fit its glyphs (Figma `textAutoResize`).
///
/// This is the property that decides whether a label *wraps* inside a too-narrow
/// box ([`TextAutoResize::None`] / fixed box) or grows its box to the text
/// ([`WidthAndHeight`](TextAutoResize::WidthAndHeight) — Figma "Auto width", the
/// common button-label case) / grows only its height
/// ([`Height`](TextAutoResize::Height) — "Auto height"). A Stage-2 auto-layout
/// pass needs it to size a HUG button frame to its label rather than wrapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAutoResize {
    /// Fixed box: the shaper wraps text to `local_size[0]` and clips to
    /// `local_size[1]`. Figma `NONE`. The default for a plain text node.
    #[default]
    None,
    /// Auto width: the box hugs the text on BOTH axes; lines never wrap (a label
    /// gets exactly as wide as its glyphs). Figma `WIDTH_AND_HEIGHT`.
    WidthAndHeight,
    /// Auto height: width is fixed (wrap to `local_size[0]`), height hugs the
    /// laid-out paragraph. Figma `HEIGHT`.
    Height,
}

/// Horizontal paragraph alignment for a [`TextNode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    /// Lines flush to the left edge of the box. The default.
    #[default]
    Left,
    /// Lines centered within the box.
    Center,
    /// Lines flush to the right edge of the box.
    Right,
    /// Lines stretched to fill the box width (last line left-aligned).
    Justify,
}

/// Vertical alignment of the laid-out paragraph block within a [`TextNode`]'s
/// box height (`local_size[1]`). Figma's `textAlignVertical`. Centered text in a
/// button/cell/badge must sit in the middle of the box, not pinned to the top.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VAlign {
    /// Block pinned to the top of the box. The default (matches Figma's `TOP`).
    #[default]
    Top,
    /// Block centered vertically within the box (`CENTER`).
    Center,
    /// Block pinned to the bottom of the box (`BOTTOM`).
    Bottom,
}

/// Uniform character styling for a [`TextNode`], in the doc's Skia-free layer.
///
/// Field-for-field parallel to `fanta_text::TextStyle` on purpose: `fanta-render`
/// (which may link Skia) converts one to the other with a plain struct literal.
/// We keep a separate type here rather than importing the text engine's because
/// `fanta-doc` must not grow a Skia/text-shaping dependency — the doc model is
/// renderer-agnostic by design (crate docs; ARCHITECTURE.md §13). The defaults
/// match `fanta_text::TextStyle::default` (16px Inter, regular, black) so a
/// node created with `..Default::default()` looks identical in both layers.
///
/// Container-level `serde(default)` makes every field optional on load: a
/// hand-authored `.fnx` `style={{"size_px": 24.0, "weight": 700}}` fills the
/// rest from `Self::default()` instead of failing the whole page. Purely a
/// deserialization affordance — serialization is unchanged, so existing docs
/// round-trip byte-identical.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextStyle {
    /// Primary font family (e.g. "Inter"). Resolved against the system font
    /// manager with fallback at layout time, so an unavailable family degrades
    /// gracefully instead of failing.
    pub font_family: String,
    /// Em size in logical pixels (world units at zoom 1).
    pub size_px: f64,
    /// OpenType numeric weight (100–900). 400 = regular, 700 = bold. Stored
    /// numerically so intermediate weights (e.g. 500) survive a round-trip.
    pub weight: u16,
    /// Italic / oblique.
    pub italic: bool,
    /// Draw an underline decoration under this run.
    #[serde(default, skip_serializing_if = "is_false")]
    pub underline: bool,
    /// Draw a line-through decoration through this run.
    #[serde(default, skip_serializing_if = "is_false")]
    pub strikethrough: bool,
    /// Glyph fill color, in the same sRGB space as every other document color.
    pub color: Color,
    /// Extra inter-character spacing in logical pixels; negative tightens.
    pub letter_spacing: f64,
    /// Line height as a multiple of `size_px` (1.0 = single spacing). Unitless
    /// so it scales with the font size, matching CSS `line-height`.
    ///
    /// When [`line_height_auto_percent`](Self::line_height_auto_percent) is
    /// `Some`, this holds only a metric-free *approximation* for consumers
    /// that don't resolve font metrics; the authoritative value is the
    /// percentage of the font's intrinsic line height.
    pub line_height: f64,
    /// Metric-relative line height: `Some(p)` means the authored line height is
    /// `p` percent of the FONT'S INTRINSIC line height (ascent + descent + gap
    /// from the font metrics), not of `size_px`. Figma's "auto" line height is
    /// exactly `Some(100.0)` — Kiwi `lineHeight` PERCENT units. Consumers that
    /// resolve font metrics (the text engine) should prefer this over the
    /// scalar [`line_height`](Self::line_height) approximation when present.
    /// Absent ⇒ `None` ⇒ old docs round-trip byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_height_auto_percent: Option<f64>,
    /// Variable-font axis settings (Figma's variable-font properties): each is a
    /// 4-char OpenType axis tag and its design value, e.g. `wght=350`,
    /// `wdth=75`, `opsz=14`, or a custom axis. Empty ⇒ the face's default
    /// instance; skipped from serialization so old docs round-trip
    /// byte-identical. The text engine applies these as the font's variation
    /// coordinates (not just the numeric `weight`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub font_variations: Vec<FontVariation>,
}

/// One variable-font axis setting: a 4-character OpenType axis tag and its
/// design value. Kept Skia-free (a plain tag + number) so the doc model stays
/// renderer-agnostic; the text engine turns it into font variation coordinates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontVariation {
    /// OpenType axis tag — 4 ASCII chars (`wght`, `wdth`, `slnt`, `opsz`,
    /// `ital`, or a custom axis). Shorter tags pad with spaces, longer truncate.
    pub axis: String,
    /// The axis design value, in that axis's own units (e.g. `100.0..=900.0`
    /// for `wght`).
    pub value: f32,
}

impl FontVariation {
    pub fn new(axis: impl Into<String>, value: f32) -> Self {
        Self {
            axis: axis.into(),
            value,
        }
    }

    /// The axis tag packed big-endian into a `u32` (an OpenType `Tag`), padding
    /// with spaces / truncating to 4 bytes.
    pub fn axis_tag(&self) -> u32 {
        let mut bytes = [b' '; 4];
        for (slot, byte) in bytes.iter_mut().zip(self.axis.bytes()) {
            *slot = byte;
        }
        u32::from_be_bytes(bytes)
    }
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            // Inter — bundled in fanta-text, Figma's UI typeface. Matches
            // `fanta_text::TextStyle::DEFAULT_FAMILY` so a freshly-placed text
            // node renders the same face the layout engine ships.
            font_family: "Inter".to_string(),
            size_px: 16.0,
            weight: 400,
            italic: false,
            underline: false,
            strikethrough: false,
            color: Color::BLACK,
            letter_spacing: 0.0,
            line_height: 1.2,
            line_height_auto_percent: None,
            font_variations: Vec::new(),
        }
    }
}

/// A byte range within a [`TextNode`] that overrides the node's base
/// [`TextStyle`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextStyleRun {
    /// Inclusive UTF-8 byte offset where the run begins.
    pub start: usize,
    /// Exclusive UTF-8 byte offset where the run ends.
    pub end: usize,
    /// Formatting applied to this byte range.
    pub style: TextStyle,
}

/// A run of text laid out inside a box.
///
/// The base [`TextStyle`] applies to the whole string; optional
/// [`TextStyleRun`] entries override specific UTF-8 byte ranges for imported
/// rich text spans.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextNode {
    /// The UTF-8 string to render. May contain `\n` hard line breaks.
    pub content: String,
    /// The local-space box the text lays out into. `local_size[0]` is the wrap
    /// width (the shaper breaks lines to it); `local_size[1]` is the box height,
    /// used for bounds, vertical positioning, and clipping — not for shaping.
    pub local_size: [f64; 2],
    /// Horizontal alignment of lines within the box.
    #[serde(default)]
    pub align: TextAlign,
    /// Vertical alignment of the paragraph block within the box height. Defaults
    /// to `Top` (Figma's default); `Center`/`Bottom` offset the paint origin so
    /// centered button/cell/badge labels sit correctly in their box.
    #[serde(default)]
    pub vertical_align: VAlign,
    /// Uniform character style applied to the whole string.
    #[serde(default)]
    pub style: TextStyle,
    /// Optional rich-text overrides imported from Figma `styleOverrideTable`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub style_runs: Vec<TextStyleRun>,
    /// How the box resizes to its glyphs (Figma `textAutoResize`). `None` (the
    /// default) is a fixed box that wraps; `WidthAndHeight` hugs the text and
    /// never wraps (the auto-width button-label case). Imported from `.fig`;
    /// Stage 2 reads it when laying a label out inside a HUG auto-layout frame.
    /// Absent ⇒ `None` ⇒ old files round-trip byte-identical.
    #[serde(default, skip_serializing_if = "is_text_autoresize_none")]
    pub auto_resize: TextAutoResize,
    /// Maximum number of laid-out lines to show (Figma `maxLines`, the "line
    /// clamp" paired with truncation). `None` ⇒ unlimited. Additive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_lines: Option<u32>,
    /// Truncate overflowing text with an ellipsis (Figma
    /// `textTruncation: ENDING`) — at [`max_lines`](Self::max_lines) when set,
    /// otherwise at the box height. Default `false` is skipped.
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncate: bool,
    /// Extra vertical space in logical pixels inserted between paragraphs
    /// (after each hard newline) — Figma `paragraphSpacing`. `0` ⇒ plain line
    /// spacing, skipped from JSON.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub paragraph_spacing: f64,
    /// First-line indent of each paragraph in logical pixels — Figma
    /// `paragraphIndent`. `0` skipped.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub paragraph_indent: f64,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

fn is_text_autoresize_none(v: &TextAutoResize) -> bool {
    matches!(v, TextAutoResize::None)
}

impl TextNode {
    /// A text node with the given content and box size, styled with defaults
    /// (16px Inter, regular, black, left-aligned). The common "just place
    /// some text" path; callers tweak `style` / `align` afterward.
    pub fn new(content: impl Into<String>, width: f64, height: f64) -> Self {
        Self {
            content: content.into(),
            local_size: [width, height],
            align: TextAlign::default(),
            vertical_align: VAlign::default(),
            style: TextStyle::default(),
            style_runs: Vec::new(),
            auto_resize: TextAutoResize::default(),
            max_lines: None,
            truncate: false,
            paragraph_spacing: 0.0,
            paragraph_indent: 0.0,
        }
    }

    pub fn set_glyph_color(&mut self, color: Color) {
        self.style.color = color;
        for run in &mut self.style_runs {
            run.style.color = color;
        }
    }
}

/// Raster image fitted into a node-local rectangle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BitmapNode {
    pub asset: AssetId,
    /// Natural pixel dimensions of the asset. Useful for crop math without
    /// loading the asset blob.
    pub natural_size: [u32; 2],
    /// Local canvas-space size the image fits into.
    pub local_size: [f64; 2],
    /// Optional crop rectangle in normalized 0..=1 asset space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<[f32; 4]>, // [x, y, w, h] in 0..=1
    /// How to size the (cropped) image into the local rect.
    #[serde(default = "default_fit_mode")]
    pub fit: crate::style::ImageFitMode,
    /// Optional color overlay applied multiplicatively on top of the image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tint: Option<Color>,
}

fn default_fit_mode() -> crate::style::ImageFitMode {
    crate::style::ImageFitMode::Fill
}

/// Video clip. Frame-accurate playback; timeline integration lives in
/// `fanta-video` (phase 3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoNode {
    pub asset: AssetId,
    pub natural_size: [u32; 2],
    pub local_size: [f64; 2],
    /// Active clip window in microseconds (start, end) within the source.
    pub time_range_us: [i64; 2],
    /// Playback speed multiplier. 1.0 = realtime; negative = reverse (phase 3).
    #[serde(default = "default_speed")]
    pub speed: f32,
    #[serde(default)]
    pub muted: bool,
    #[serde(default = "default_volume")]
    pub volume: f32,
    /// Frame to display when the timeline is not actively playing. Defaults
    /// to "first frame of trimmed range".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poster_frame_us: Option<i64>,
    /// A decoded poster image (the extracted frame) the canvas draws for the
    /// node, so a placed video shows real content rather than a placeholder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poster: Option<AssetId>,
    #[serde(default = "default_fit_mode")]
    pub fit: crate::style::ImageFitMode,
}

fn default_speed() -> f32 {
    1.0
}
fn default_volume() -> f32 {
    1.0
}

/// Audio clip displayed as a waveform.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioNode {
    pub asset: AssetId,
    pub local_size: [f64; 2],
    pub time_range_us: [i64; 2],
    #[serde(default = "default_volume")]
    pub volume: f32,
    #[serde(default)]
    pub muted: bool,
    /// Color of the rendered waveform.
    #[serde(default = "Color::default")]
    pub waveform_color: Color,
}

// =============================================================================
// Node graph payload — inspired by ComfyUI / Invoke / Substance Designer
// =============================================================================

/// An embedded workflow graph rendered as a canvas node. The graph editor is
/// a panel that, when opened on this node, treats it as a recursive canvas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeGraphNode {
    pub local_size: [f64; 2],
    pub graph: NodeGraph,
    /// Optional cached preview of the current output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<AssetId>,
}

/// The actual graph contents — workflow nodes and links between their ports.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NodeGraph {
    /// Workflow nodes keyed by id. `BTreeMap` for deterministic JSON ordering;
    /// graph sizes here are small (hundreds, not millions).
    #[serde(default)]
    pub nodes: BTreeMap<WorkflowNodeId, WorkflowNode>,
    /// Edges between ports. Order preserved because some engines treat it as
    /// declaration order for tie-breaking.
    #[serde(default)]
    pub links: Vec<Link>,
    /// Which workflow node's primary output is the canvas-visible artifact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<WorkflowNodeId>,
}

/// One workflow node inside a [`NodeGraph`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowNode {
    pub id: WorkflowNodeId,
    /// Type identifier (e.g. `"image.generate"`, `"image.resize"`). The
    /// registry that maps these to executable bodies lives in `fanta-nodes`.
    pub kind: String,
    /// Editor position. Doesn't affect execution.
    pub position: [f64; 2],
    /// Type-specific parameters. Validated by `fanta-nodes` against the kind.
    #[serde(default)]
    pub params: serde_json::Value,
}

/// One directed edge between an output port and an input port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub id: LinkId,
    pub from_node: WorkflowNodeId,
    pub from_port: String,
    pub to_node: WorkflowNodeId,
    pub to_port: String,
}

// =============================================================================
// 3D, AI, Embed
// =============================================================================

/// A 3D viewport rendering a model via the wgpu pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Model3dNode {
    pub asset: AssetId,
    pub local_size: [f64; 2],
    /// Camera in spherical coords (azimuth, elevation, distance, target [3]).
    pub camera: Camera3d,
    /// Material/environment overrides as opaque JSON until phase 5 lands.
    #[serde(default)]
    pub overrides: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Camera3d {
    pub azimuth: f32,
    pub elevation: f32,
    pub distance: f32,
    pub target: [f32; 3],
    pub fov_deg: f32,
}

impl Default for Camera3d {
    fn default() -> Self {
        Self {
            azimuth: 0.0,
            elevation: 0.0,
            distance: 5.0,
            target: [0.0; 3],
            fov_deg: 50.0,
        }
    }
}

/// AI-generated artifact, re-rollable and lineage-tracked.
///
/// This is the Krea / Invoke / Midjourney design point: an AI output is not a
/// transient render behind a chat sidebar — it is a doc node with the prompt,
/// model, params, input refs, and lineage parent all recorded. Re-rolling
/// produces a sibling, not a destructive overwrite.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiArtifactNode {
    pub local_size: [f64; 2],
    pub prompt: String,
    /// Model identifier — `"claude-opus-4-7"`, `"flux-pro"`, `"sdxl"`, …
    pub model: String,
    /// Per-model parameters (negative prompt, sampler, steps, cfg, ...).
    #[serde(default)]
    pub params: serde_json::Value,
    /// Other canvas nodes feeding into this one (reference images, masks).
    #[serde(default)]
    pub inputs: Vec<crate::id::NodeId>,
    /// The previous version this was re-rolled from, if any. Forms a DAG.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_parent: Option<crate::id::NodeId>,
    /// Cached output blob. `None` while pending or failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<AssetId>,
    /// Status of the generation request driving this node.
    #[serde(default)]
    pub status: GenerationStatus,
    /// Seed for reproducibility. `None` means "random next time".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationStatus {
    /// Queued or otherwise not yet started.
    #[default]
    Pending,
    /// In-flight. The progress is observable elsewhere (events, not state).
    Running,
    /// Finished successfully; `output` is populated.
    Done,
    /// Finished with an error; `output` is `None`, the error is logged.
    Failed,
}

/// Forward-compat container. A plugin or future variant lands a typed payload
/// here without forcing a schema migration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbedNode {
    pub local_size: [f64; 2],
    /// Variant tag (e.g. `"twitter.embed"`, `"latex"`).
    pub kind: String,
    /// Opaque, type-specific payload.
    #[serde(default)]
    pub payload: serde_json::Value,
}
