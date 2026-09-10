//! The polymorphic [`CanvasNode`] — every kind of thing that lives on a Fanta
//! canvas, dispatched through a single sum type.
//!
//! ## Why a single enum
//!
//! Closed-schema canvases (Figma, Miro) can never become polymorphic. Open-
//! schema canvases (this one, Krea) can always become Figma. The choice has to
//! land on day one — see ARCHITECTURE.md §3.
//!
//! ## Layout
//!
//! Each node has a wrapper carrying the metadata every variant needs
//! (identity, hierarchy, transform, opacity, effects, flags) and a [`NodeData`]
//! payload carrying the variant-specific data. Putting effects/opacity on the
//! wrapper rather than on each variant means the compositing pipeline is
//! uniform — `fanta-render` reads them without matching on the variant.
//!
//! ## Module map
//!
//! This module owns the [`CanvasNode`] wrapper, the [`NodeData`] sum type, and
//! the wrapper-level [`NodeFlags`] / [`MaskType`]. The variant payloads and the
//! auxiliary families split into focused submodules, all re-exported flat below
//! so `crate::node::Foo` paths are unchanged:
//!
//! - [`variants`] — the concrete payloads ([`GroupNode`], [`VectorNode`],
//!   [`TextNode`], [`TextPathNode`], media, [`NodeGraphNode`], [`Model3dNode`], [`AiArtifactNode`],
//!   [`EmbedNode`], …).
//! - [`layout`] — the auto-layout ("stack"/flexbox) model ([`AutoLayout`],
//!   [`LayoutChild`], and their enums).
//! - [`instance`] — component instances ([`InstanceNode`], [`Override`],
//!   [`DerivedOverride`], …).
//! - [`prototype`] — prototype interactions ([`Reaction`], [`Trigger`],
//!   [`Action`], [`Transition`], overlays).
//!
//! ## Lessons from prior art
//!
//! - **tldraw** — opaque `meta` field per node for user extension; `name`
//!   field for the layers panel; `parent` is a single id field, children are
//!   derived from the [`Scene`] index.
//! - **Figma** — distinct `Group` and `Frame` (frame = group with background +
//!   clip). We start with `Group` and treat backgrounds as a child vector node;
//!   a dedicated `Frame` variant can land later without breaking docs.
//! - **ComfyUI / Invoke** — node graphs as a doc-level structure; ports and
//!   typed links. We model that as the [`NodeGraph`] payload of a
//!   [`NodeData::NodeGraph`] canvas node.
//! - **Krea** — AI artifacts as first-class re-rollable nodes with lineage,
//!   not transient renders behind a chat sidebar. Reflected in
//!   [`AiArtifactNode`].
//! - **OpenPencil #2** — canonical JSON shape for AI to read/write directly;
//!   informs the `#[serde(tag = "type")]` choice on [`NodeData`].
//!
//! [`Scene`]: crate::scene::Scene

pub mod fields;
pub mod instance;
pub mod layout;
pub mod prototype;
pub mod variants;

pub use instance::{
    DerivedOverride, DerivedText, InstanceNode, Override, OverridePath, OverrideValue,
};
pub use layout::{AutoLayout, AxisSizing, CounterAlign, LayoutChild, LayoutMode, PrimaryAlign};
pub use prototype::{
    Action, Direction, Easing, OverlayPosition, OverlaySettings, PrototypeAnimation, Reaction,
    Transition, TransitionStyle, Trigger, spring_progress,
};
pub use variants::{
    AiArtifactNode, AudioNode, BitmapNode, BooleanNode, BooleanOp, Camera3d, ConstraintH,
    ConstraintV, Constraints, EmbedNode, FontVariation, GenerationStatus, GroupNode, Link,
    Model3dNode, NodeGraph, NodeGraphNode, ParametricShape, ScrollBehavior, ScrollDirection,
    TextAlign, TextAutoResize, TextNode, TextPathAlignment, TextPathDirection, TextPathNode,
    TextPathSide, TextPathStart, TextStyle, TextStyleRun, VAlign, VectorNode, VideoNode,
    WorkflowNode,
};

use crate::binding::BoundProp;
use crate::id::{NodeId, VariableId};
use crate::index::IndexKey;
use crate::style::{BlendMode, Blur, Shadow, Stroke, UnitInterval};
use crate::transform::{Bounds, Transform2D};
use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::collections::BTreeMap;

// =============================================================================
// Wrapper
// =============================================================================

/// One node in the polymorphic scene graph.
///
/// The wrapper carries everything the renderer and the doc machinery need
/// without matching on the variant. Variant-specific data is on [`NodeData`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanvasNode {
    /// Stable identity. Unique across the whole document.
    pub id: NodeId,

    /// Parent node id, or `None` for root-level children. Children are derived
    /// by [`Scene`] from this field — there is no `children: Vec<NodeId>`,
    /// because storing the inverse relationship would double-write on every
    /// reparent and would be CRDT-hostile.
    ///
    /// [`Scene`]: crate::scene::Scene
    pub parent: Option<NodeId>,

    /// Fractional index for z-order within the parent. Higher = on top.
    pub index: IndexKey,

    /// Display name, shown in the layers panel. Defaults to a variant-appropriate
    /// placeholder ("Rectangle", "Image", "AI") on creation.
    pub name: String,

    /// Local transform relative to the parent. Identity if untouched.
    #[serde(default)]
    pub transform: Transform2D,

    /// Figma-style constraints for responsive behavior when parent resizes
    /// (non-auto-layout children). When present, the transform is interpreted
    /// relative to these rules rather than being fully baked. Default (None)
    /// preserves current "baked" behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraints: Option<Constraints>,

    /// Multiplied into the variant's own paint opacity at render. A
    /// [`UnitInterval`], so it is always in `0.0..=1.0` by construction.
    #[serde(default)]
    pub opacity: UnitInterval,

    /// Compositing blend mode. Default is `Normal` (regular alpha-over).
    #[serde(default)]
    pub blend_mode: BlendMode,

    /// Node-level effects (shadows). Applied to the composited node, not to
    /// individual paints. Most nodes have zero effects; SmallVec keeps the
    /// common case cache-friendly.
    #[serde(default, skip_serializing_if = "SmallVec::is_empty")]
    pub effects: SmallVec<[Shadow; 0]>,

    /// Node-level **blur** effects ([`Blur`]) — a LAYER blur Gaussian-blurs the
    /// node's own composited layer (and subtree); a BACKGROUND blur blurs the
    /// backdrop visible through the node's silhouette. Kept as its own list
    /// rather than folded into [`effects`](Self::effects) because a blur has no
    /// color/offset/spread — only a radius — so overloading [`Shadow`] would be
    /// a worse fit and would ripple the shadow shape. Additive: absent on every
    /// pre-existing doc, skipped from JSON when empty, so old files round-trip
    /// byte-identical. Most nodes carry none; `SmallVec<[_; 0]>` keeps the empty
    /// case heap-free.
    #[serde(default, skip_serializing_if = "SmallVec::is_empty")]
    pub blurs: SmallVec<[Blur; 0]>,

    /// Bit-packed flags — locked, hidden, isolated blend.
    #[serde(default, skip_serializing_if = "NodeFlags::is_empty")]
    pub flags: NodeFlags,

    /// `true` ⇒ this node is a **mask** for its FOLLOWING SIBLINGS within the same
    /// parent (Figma `mask` / `isMask`): those siblings are clipped/masked by this
    /// node's shape, until the next mask sibling or the end of the parent's
    /// children. The mask node itself is not painted as normal content — only its
    /// shape (alpha coverage, or luminance for [`MaskType::Luminance`]) drives the
    /// mask. A bool flag (rather than a [`NodeData`] variant) keeps the ripple
    /// small: any node kind can be a mask. Absent ⇒ `false` ⇒ old files round-trip
    /// byte-identical.
    #[serde(default, skip_serializing_if = "crate::serde_util::is_false")]
    pub is_mask: bool,

    /// How this node masks its following siblings when [`is_mask`](Self::is_mask)
    /// is set. [`MaskType::Alpha`] (the default — mask by the mask shape's alpha
    /// coverage) or [`MaskType::Luminance`] (mask by the mask's painted
    /// luminance). Figma's `VECTOR`/`OUTLINE` mask type is treated as alpha of the
    /// vector shape. Ignored when `is_mask` is `false`; skipped from serialization
    /// when it is the default ([`MaskType::Alpha`]) so old files round-trip
    /// byte-identical.
    #[serde(default, skip_serializing_if = "MaskType::is_default")]
    pub mask_type: MaskType,

    /// How this node behaves when an ancestor frame scrolls in a prototype
    /// (Figma per-child `scrollBehavior`): moves with content (the default),
    /// stays fixed, or sticks at the container edge. On the wrapper — not
    /// [`GroupNode`] — because any node kind can be a fixed header or sticky
    /// row. Default is skipped, so old files round-trip byte-identical.
    #[serde(default, skip_serializing_if = "ScrollBehavior::is_default")]
    pub scroll_behavior: ScrollBehavior,

    /// Variant-specific data.
    #[serde(flatten)]
    pub data: NodeData,

    /// Opaque user-extension data. Plugins, AI agents, and external tools stash
    /// extra fields here without colliding with the core schema. Mirrors
    /// tldraw's `meta`. Use for **comments, annotations, measurements**, etc.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub meta: serde_json::Value,

    /// Variable bindings: which properties of this node are driven by a
    /// design-system variable. Keyed by [`BoundProp`] (the same address type
    /// instance overrides use). Resolution substitutes the variable's value for
    /// the effective mode at render/export time. Absent ⇒ empty ⇒ no bindings,
    /// so old files round-trip byte-identical.
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        with = "crate::binding::map_as_seq"
    )]
    pub bindings: BTreeMap<BoundProp, VariableId>,

    /// Prototype interactions originating from this node (click → navigate,
    /// after-delay → open overlay, …). Empty for the vast majority of nodes;
    /// `Vec` preserves authoring order. Absent ⇒ empty in old files.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<Reaction>,

    /// How this node participates as a child of an auto-layout parent (grow,
    /// absolute positioning, per-child align-self). `None` for nodes that are
    /// not auto-layout children, or that take all the parent's defaults — so
    /// most nodes carry nothing and old files round-trip byte-identical. The
    /// parent's [`AutoLayout`] lives on its [`GroupNode`]; this is the per-child
    /// side a Stage-2 layout pass reads alongside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout_child: Option<LayoutChild>,
}

impl CanvasNode {
    /// Construct a new node with a fresh id, identity transform, full opacity,
    /// and no parent. The caller is expected to attach it to a [`Scene`].
    ///
    /// [`Scene`]: crate::scene::Scene
    pub fn new(data: NodeData) -> Self {
        let name = data.default_name().to_owned();
        Self {
            id: NodeId::new(),
            parent: None,
            index: IndexKey::FIRST,
            name,
            transform: Transform2D::IDENTITY,
            opacity: UnitInterval::ONE,
            blend_mode: BlendMode::Normal,
            effects: SmallVec::new(),
            blurs: SmallVec::new(),
            flags: NodeFlags::empty(),
            is_mask: false,
            mask_type: MaskType::Alpha,
            scroll_behavior: ScrollBehavior::Scrolls,
            data,
            meta: serde_json::Value::Null,
            constraints: None,
            bindings: BTreeMap::new(),
            reactions: Vec::new(),
            layout_child: None,
        }
    }

    /// Apply Figma constraints to adjust a child's transform when its parent frame resizes.
    /// This makes the constraints model "live" for responsive behavior (prod readiness).
    /// Call this from tools on parent resize ops.
    pub fn apply_constraints(
        &self,
        old_parent_size: [f64; 2],
        new_parent_size: [f64; 2],
    ) -> Option<Transform2D> {
        let c = self.constraints.as_ref()?;
        if old_parent_size[0] <= 0.0 || old_parent_size[1] <= 0.0 {
            return None;
        }
        let child_size = match &self.data {
            NodeData::Group(g) => g.clip_size.or(g.local_size).unwrap_or([100.0, 100.0]),
            NodeData::Vector(v) => v.local_size.unwrap_or_else(|| {
                v.path
                    .rough_bounds()
                    .map(|b| [b.width(), b.height()])
                    .unwrap_or([50.0, 50.0])
            }),
            NodeData::TextPath(text_path) => text_path
                .path
                .rough_bounds()
                .map(|bounds| [bounds.width(), bounds.height()])
                .unwrap_or([50.0, 50.0]),
            _ => [50.0, 50.0],
        };
        let comps = self.transform.to_components();
        // to_components layout (SVG matrix order): [a, b, c, d, tx, ty]
        let ox = comps[4];
        let oy = comps[5];
        // Preserve linear part (a,b,c,d) unless applying Scale constraints.
        let mut lin = [comps[0], comps[1], comps[2], comps[3]];

        let nx = match c.horizontal {
            ConstraintH::Left => ox,
            ConstraintH::Right => new_parent_size[0] - (old_parent_size[0] - ox),
            ConstraintH::LeftRight => {
                let right = old_parent_size[0] - (ox + child_size[0]);
                ox + (new_parent_size[0] - old_parent_size[0]) - right
            }
            ConstraintH::Center => ox + (new_parent_size[0] - old_parent_size[0]) * 0.5,
            ConstraintH::Scale => ox * (new_parent_size[0] / old_parent_size[0]),
        };

        let ny = match c.vertical {
            ConstraintV::Top => oy,
            ConstraintV::Bottom => new_parent_size[1] - (old_parent_size[1] - oy),
            ConstraintV::TopBottom => {
                let bottom = old_parent_size[1] - (oy + child_size[1]);
                oy + (new_parent_size[1] - old_parent_size[1]) - bottom
            }
            ConstraintV::Center => oy + (new_parent_size[1] - old_parent_size[1]) * 0.5,
            ConstraintV::Scale => oy * (new_parent_size[1] / old_parent_size[1]),
        };

        if matches!(c.horizontal, ConstraintH::Scale) {
            let fx = new_parent_size[0] / old_parent_size[0];
            lin[0] *= fx;
            lin[1] *= fx;
        }
        if matches!(c.vertical, ConstraintV::Scale) {
            let fy = new_parent_size[1] / old_parent_size[1];
            lin[2] *= fy;
            lin[3] *= fy;
        }

        Some(Transform2D::from_components([
            lin[0], lin[1], lin[2], lin[3], nx, ny,
        ]))
    }

    /// Convenience: is this node a group (contains children, draws nothing of
    /// its own)?
    pub fn is_group(&self) -> bool {
        matches!(self.data, NodeData::Group(_))
    }

    /// Whether this node is a boolean-operation container.
    pub fn is_boolean(&self) -> bool {
        matches!(self.data, NodeData::Boolean(_))
    }

    /// Whether the node accepts children. Groups and boolean-operation nodes (the
    /// latter's children are its operands) — an [`InstanceNode`]'s children are
    /// *virtual* (produced by [`crate::resolve::expand_instance`]), so it
    /// explicitly does NOT accept real scene children.
    pub fn can_have_children(&self) -> bool {
        self.is_group() || self.is_boolean()
    }
}

// =============================================================================
// Flags
// =============================================================================

bitflags! {
    /// Per-node boolean state, bit-packed to keep the wrapper cache-friendly.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
    #[serde(transparent)]
    pub struct NodeFlags: u32 {
        /// Locked nodes cannot be selected or edited by user input. AI tools
        /// can still target them by id (so an agent can unlock-then-edit).
        const LOCKED = 1 << 0;
        /// Hidden nodes are skipped by rendering and hit-testing.
        const HIDDEN = 1 << 1;
        /// Forces a compositing group — children are flattened before parent
        /// blend mode applies. Mirrors Figma's "Pass through" toggle inverted.
        const ISOLATED_BLEND = 1 << 2;
        /// Excluded from automatic AI context (when an agent gets "everything
        /// on canvas," this node is omitted). Useful for sensitive layers.
        const EXCLUDE_FROM_AI = 1 << 3;
        /// Edited vector paths may extend beyond an imported viewport. This
        /// overrides `local_size` clipping and prevents legacy viewport backfill.
        const UNCLIPPED_VECTOR = 1 << 4;
    }
}

// =============================================================================
// Masks
// =============================================================================

/// How a node flagged [`CanvasNode::is_mask`] masks its following siblings.
///
/// Maps from Figma's `maskType` (Kiwi `maskType`): `ALPHA` (the default) →
/// [`MaskType::Alpha`]; `LUMINANCE` → [`MaskType::Luminance`]; `VECTOR` /
/// `OUTLINE` are treated as alpha of the vector shape, so they also map to
/// [`MaskType::Alpha`] (the mask shape's coverage is its outline). The renderer
/// composites masked siblings against the mask with `DstIn`: ALPHA uses the
/// mask's painted alpha directly; LUMINANCE first runs the mask's pixels through
/// a luma→alpha color filter so brighter mask pixels reveal more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaskType {
    /// Mask by the mask shape's ALPHA coverage (the default). Also the target for
    /// Figma's `VECTOR`/`OUTLINE` mask type.
    #[default]
    Alpha,
    /// Mask by the mask's painted LUMINANCE — brighter mask pixels reveal more of
    /// the masked siblings, black hides them.
    Luminance,
}

impl MaskType {
    /// Whether this is the default ([`MaskType::Alpha`]) — used by serde to skip
    /// serializing the field for the common case, so old files round-trip
    /// byte-identical.
    pub fn is_default(&self) -> bool {
        matches!(self, MaskType::Alpha)
    }
}

// =============================================================================
// Variant payload
// =============================================================================

/// The variant-specific data carried by a [`CanvasNode`].
///
/// # Adding a node type
///
/// The polymorphic dispatch is centralized, so a new **box-shaped** variant
/// (one that carries a `local_size: [f64; 2]`, like the media variants) touches
/// only:
///
/// 1. `node/variants.rs` — the payload struct (with a `local_size` field).
/// 2. `NodeData` here — the variant, plus an arm in [`default_name`] and
///    [`kind_tag`] (the compiler flags both). [`local_size`], [`set_local_size`],
///    and [`local_bounds`] are the seam every other consumer reads through, so
///    bounds, auto-layout sizing, snapshots, effects-layer bounds, and the
///    render silhouette all follow from those without further edits.
/// 3. `fanta-render`'s `paint_node_content` — the paint arm (the match has no
///    wildcard, so this is compiler-enforced too).
/// 4. `fanta-fnx`'s `TYPE_TAGS` — the `.fnx` tag mapping.
///
/// A variant with geometry outside the common box-shaped representation (like
/// [`Vector`]'s path or [`Group`]'s clipping box) additionally needs arms in
/// [`local_size`] / [`set_local_size`] / [`local_bounds`]; the accessors return
/// the box-shaped default via the catch-all `data` arm, which the special cases
/// override.
///
/// [`default_name`]: NodeData::default_name
/// [`kind_tag`]: NodeData::kind_tag
/// [`local_size`]: NodeData::local_size
/// [`set_local_size`]: NodeData::set_local_size
/// [`local_bounds`]: NodeData::local_bounds
/// [`Vector`]: NodeData::Vector
/// [`Group`]: NodeData::Group
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeData {
    /// Hierarchical container. Draws nothing; children draw their content.
    Group(GroupNode),

    /// Vector shape — path data + fills + strokes. The single most common
    /// node type; covers rectangles, ellipses, freehand strokes, custom paths.
    Vector(VectorNode),

    /// Text — a string laid out in a box with paragraph + character styling.
    /// The doc layer keeps a Skia-free style model ([`TextNode`] / [`TextStyle`]);
    /// `fanta-render` converts it to `fanta-text`'s engine types to shape, wrap,
    /// and draw glyphs. This is what a Figma `TEXT` node imports to.
    Text(TextNode),

    /// Styled text placed along an owned vector baseline.
    TextPath(TextPathNode),

    /// Raster image (PNG/JPG/WebP). Asset stored externally; node references
    /// it by [`AssetId`](crate::id::AssetId) and declares the canvas-space
    /// rectangle to fit into.
    Bitmap(BitmapNode),

    /// Video clip. Frame-accurate, timeline-aware, supports playback inside
    /// the canvas. Phase 3 in the roadmap.
    Video(VideoNode),

    /// Audio asset. Renders as a waveform on canvas; participates in the
    /// timeline alongside video.
    Audio(AudioNode),

    /// Embedded sub-graph. The node's preview is the graph's output; the
    /// editor can open the graph in a panel and treat it as a recursive
    /// canvas of its own.
    NodeGraph(NodeGraphNode),

    /// 3D viewport. Renders a model via the wgpu pipeline into an off-screen
    /// texture, composited as an image fill.
    Model3d(Model3dNode),

    /// AI-generated artifact. Re-rollable, lineage-tracked, parameters
    /// recorded so the generation is reproducible.
    AiArtifact(AiArtifactNode),

    /// An instance of a component. Draws the (lazily expanded) master subtree,
    /// with per-instance prop values and overrides applied. Its children are
    /// *virtual* — produced by [`crate::resolve::expand_instance`], never stored
    /// in the scene — so `can_have_children()` is `false`.
    Instance(InstanceNode),

    /// Boolean-operation container: its child operands are folded under
    /// [`BooleanNode::op`] and the result painted with the node's own fills /
    /// strokes. Accepts real scene children (the operands), like a group.
    Boolean(BooleanNode),

    /// Forward-compat extension point. Plugins and future variants land here
    /// without a breaking change.
    Embed(EmbedNode),
}

impl NodeData {
    pub fn default_name(&self) -> &'static str {
        match self {
            Self::Group(_) => "Group",
            Self::Vector(_) => "Shape",
            Self::Text(_) => "Text",
            Self::TextPath(_) => "Text on Path",
            Self::Bitmap(_) => "Image",
            Self::Video(_) => "Video",
            Self::Audio(_) => "Audio",
            Self::NodeGraph(_) => "Node Graph",
            Self::Model3d(_) => "3D",
            Self::AiArtifact(_) => "AI",
            Self::Instance(_) => "Instance",
            Self::Boolean(_) => "Boolean",
            Self::Embed(_) => "Embed",
        }
    }

    /// Short stable variant tag. A group with a clip box reads as a `frame`
    /// (Figma's distinction), an unclipped group as `group`.
    pub fn kind_tag(&self) -> &'static str {
        match self {
            Self::Group(g) => {
                if g.clip_size.is_some() {
                    "frame"
                } else {
                    "group"
                }
            }
            Self::Vector(_) => "vector",
            Self::Text(_) => "text",
            Self::TextPath(_) => "text_path",
            Self::Bitmap(_) => "bitmap",
            Self::Video(_) => "video",
            Self::Audio(_) => "audio",
            Self::NodeGraph(_) => "node_graph",
            Self::Model3d(_) => "model3d",
            Self::AiArtifact(_) => "ai_artifact",
            Self::Instance(_) => "instance",
            Self::Boolean(_) => "boolean",
            Self::Embed(_) => "embed",
        }
    }

    /// The `local_size` box of the box-shaped variants. A [`Group`] reports its
    /// non-clipping explicit box when present (frames still use `clip_size`), and
    /// `None` for a content-sized group. [`Vector`] also returns `None` because
    /// [`Vector`] (its box is the path's bounds; `VectorNode.local_size` is an
    /// optional SVG-viewport *clip*, not the content box). A [`TextPath`]
    /// likewise uses its owned baseline instead of a box. Use
    /// [`local_bounds`](Self::local_bounds) for the polymorphic answer.
    ///
    /// [`Group`]: NodeData::Group
    /// [`TextPath`]: NodeData::TextPath
    /// [`Vector`]: NodeData::Vector
    pub fn local_size(&self) -> Option<[f64; 2]> {
        match self {
            Self::Group(g) => g.local_size,
            Self::Text(t) => Some(t.local_size),
            Self::Bitmap(b) => Some(b.local_size),
            Self::Video(v) => Some(v.local_size),
            Self::Audio(a) => Some(a.local_size),
            Self::NodeGraph(n) => Some(n.local_size),
            Self::Model3d(m) => Some(m.local_size),
            Self::AiArtifact(a) => Some(a.local_size),
            Self::Instance(i) => Some(i.local_size),
            Self::Embed(e) => Some(e.local_size),
            // Vector's box is its path and Boolean's is folded operand geometry.
            Self::Vector(_) | Self::TextPath(_) | Self::Boolean(_) => None,
        }
    }

    /// Overwrite the `local_size` box of a box-shaped variant. For a plain group
    /// this writes its non-clipping explicit box; frames continue to use
    /// `clip_size`. Returns `false` for vectors, text paths, and booleans.
    ///
    /// [`Group`]: NodeData::Group
    /// [`Vector`]: NodeData::Vector
    pub fn set_local_size(&mut self, w: f64, h: f64) -> bool {
        match self {
            Self::Group(g) if g.clip_size.is_none() => g.local_size = Some([w, h]),
            Self::Group(_) => return false,
            Self::Text(t) => t.local_size = [w, h],
            Self::Bitmap(b) => b.local_size = [w, h],
            Self::Video(v) => v.local_size = [w, h],
            Self::Audio(a) => a.local_size = [w, h],
            Self::NodeGraph(n) => n.local_size = [w, h],
            Self::Model3d(m) => m.local_size = [w, h],
            Self::AiArtifact(a) => a.local_size = [w, h],
            Self::Instance(i) => i.local_size = [w, h],
            Self::Embed(e) => e.local_size = [w, h],
            Self::Vector(_) | Self::TextPath(_) | Self::Boolean(_) => return false,
        }
        true
    }

    /// Intrinsic local-space bounds, independent of any scene index: a group's
    /// clipping or explicit non-clipping box when set (`None` otherwise — a
    /// sizeless group's content bounds require walking children, which only the
    /// scene can do), a vector's path bounds, a text path's owned baseline
    /// bounds, and the `local_size` box of everything else.
    pub fn local_bounds(&self) -> Option<Bounds> {
        match self {
            Self::Group(g) => g
                .clip_size
                .or(g.local_size)
                .map(|[w, h]| Bounds::from_xywh(0.0, 0.0, w, h)),
            Self::Vector(v) => v.path.rough_bounds(),
            // Phase-one bounds are the owned baseline only. Exact glyph bounds
            // depend on shaping and belong to the renderer integration.
            Self::TextPath(text_path) => text_path.path.rough_bounds(),
            data => data
                .local_size()
                .map(|[w, h]| Bounds::from_xywh(0.0, 0.0, w, h)),
        }
    }

    /// Strokes, for the variants that carry them: a vector's shape strokes or
    /// a frame (group) border.
    pub fn strokes(&self) -> Option<&[Stroke]> {
        match self {
            Self::Vector(v) => Some(&v.strokes),
            Self::Group(g) => Some(&g.strokes),
            Self::Boolean(b) => Some(&b.strokes),
            _ => None,
        }
    }

    /// Mutable access to [`strokes`](Self::strokes).
    pub fn strokes_mut(&mut self) -> Option<&mut SmallVec<[Stroke; 1]>> {
        match self {
            Self::Vector(v) => Some(&mut v.strokes),
            Self::Group(g) => Some(&mut g.strokes),
            Self::Boolean(b) => Some(&mut b.strokes),
            _ => None,
        }
    }

    pub fn as_group(&self) -> Option<&GroupNode> {
        match self {
            Self::Group(g) => Some(g),
            _ => None,
        }
    }

    pub fn as_group_mut(&mut self) -> Option<&mut GroupNode> {
        match self {
            Self::Group(g) => Some(g),
            _ => None,
        }
    }

    pub fn as_vector(&self) -> Option<&VectorNode> {
        match self {
            Self::Vector(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_vector_mut(&mut self) -> Option<&mut VectorNode> {
        match self {
            Self::Vector(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&TextNode> {
        match self {
            Self::Text(t) => Some(t),
            _ => None,
        }
    }

    pub fn as_text_mut(&mut self) -> Option<&mut TextNode> {
        match self {
            Self::Text(t) => Some(t),
            _ => None,
        }
    }

    pub fn as_text_path(&self) -> Option<&TextPathNode> {
        match self {
            Self::TextPath(text_path) => Some(text_path),
            _ => None,
        }
    }

    pub fn as_text_path_mut(&mut self) -> Option<&mut TextPathNode> {
        match self {
            Self::TextPath(text_path) => Some(text_path),
            _ => None,
        }
    }

    pub fn as_instance(&self) -> Option<&InstanceNode> {
        match self {
            Self::Instance(i) => Some(i),
            _ => None,
        }
    }

    pub fn as_instance_mut(&mut self) -> Option<&mut InstanceNode> {
        match self {
            Self::Instance(i) => Some(i),
            _ => None,
        }
    }

    pub fn as_boolean(&self) -> Option<&BooleanNode> {
        match self {
            Self::Boolean(b) => Some(b),
            _ => None,
        }
    }

    pub fn as_boolean_mut(&mut self) -> Option<&mut BooleanNode> {
        match self {
            Self::Boolean(b) => Some(b),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
