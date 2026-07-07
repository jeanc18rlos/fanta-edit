//! The polymorphic [`CanvasNode`] — every kind of thing that lives on a Fantaisa
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
//!   [`TextNode`], media, [`NodeGraphNode`], [`Model3dNode`], [`AiArtifactNode`],
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

mod common;
pub mod instance;
pub mod layout;
pub mod prototype;
pub mod variants;

pub use instance::{
    DerivedOverride, DerivedText, InstanceNode, Override, OverridePath, OverrideValue,
};
pub use layout::{AutoLayout, AxisSizing, CounterAlign, LayoutChild, LayoutMode, PrimaryAlign};
pub use prototype::{
    Action, Direction, Easing, OverlayPosition, OverlaySettings, Reaction, Transition,
    TransitionStyle, Trigger,
};
pub use variants::{
    AiArtifactNode, AudioNode, BitmapNode, Camera3d, EmbedNode, GenerationStatus, GroupNode, Link,
    Model3dNode, NodeGraph, NodeGraphNode, TextAlign, TextAutoResize, TextNode, TextStyle,
    TextStyleRun, VAlign, VectorNode, VideoNode, WorkflowNode,
};

use crate::binding::BoundProp;
use crate::id::{NodeId, VariableId};
use crate::index::IndexKey;
use crate::style::{BlendMode, Blur, Shadow};
use crate::transform::Transform2D;
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

    /// 0.0..=1.0. Multiplied into the variant's own paint opacity at render.
    #[serde(default = "default_opacity")]
    pub opacity: f32,

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
    #[serde(default, skip_serializing_if = "common::is_false")]
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

    /// Variant-specific data.
    #[serde(flatten)]
    pub data: NodeData,

    /// Opaque user-extension data. Plugins, AI agents, and external tools stash
    /// extra fields here without colliding with the core schema. Mirrors
    /// tldraw's `meta`.
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

fn default_opacity() -> f32 {
    1.0
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
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            effects: SmallVec::new(),
            blurs: SmallVec::new(),
            flags: NodeFlags::empty(),
            is_mask: false,
            mask_type: MaskType::Alpha,
            data,
            meta: serde_json::Value::Null,
            bindings: BTreeMap::new(),
            reactions: Vec::new(),
            layout_child: None,
        }
    }

    /// Convenience: is this node a group (contains children, draws nothing of
    /// its own)?
    pub fn is_group(&self) -> bool {
        matches!(self.data, NodeData::Group(_))
    }

    /// Whether the node accepts children. Groups only — an [`InstanceNode`]'s
    /// children are *virtual* (produced by [`crate::resolve::expand_instance`]),
    /// so it explicitly does NOT accept real scene children. Later: frames.
    pub fn can_have_children(&self) -> bool {
        self.is_group()
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
            Self::Bitmap(_) => "Image",
            Self::Video(_) => "Video",
            Self::Audio(_) => "Audio",
            Self::NodeGraph(_) => "Node Graph",
            Self::Model3d(_) => "3D",
            Self::AiArtifact(_) => "AI",
            Self::Instance(_) => "Instance",
            Self::Embed(_) => "Embed",
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
