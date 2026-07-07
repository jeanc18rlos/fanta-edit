//! Instances of components — the placed [`InstanceNode`], its sparse
//! [`Override`]s, and Figma's baked [`DerivedOverride`] render data.
//!
//! An instance stores only the *delta* from its master; the drawable subtree is
//! produced on demand by [`crate::resolve::expand_instance`].

use crate::binding::BoundProp;
use crate::color::Color;
use crate::id::{ComponentId, ComponentPropId, NodeId};
use crate::path::PathData;
use crate::style::{Fill, Stroke};
use crate::transform::Transform2D;
use crate::value::VarValue;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::collections::BTreeMap;

/// A placed instance of a component master.
///
/// The instance stores only the *delta* from the master: which exposed props it
/// sets ([`prop_values`]), and which specific descendant properties it overrides
/// ([`overrides`]). The actual drawable subtree is produced on demand by
/// [`crate::resolve::expand_instance`], so editing the master automatically
/// reflows every instance (the master `rev` invalidates render memo).
///
/// [`prop_values`]: InstanceNode::prop_values
/// [`overrides`]: InstanceNode::overrides
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstanceNode {
    /// The component this is an instance of. A dangling id (master deleted) is
    /// tolerated — resolution yields an empty expansion, `validate` warns.
    pub component: ComponentId,
    /// Per-descendant property overrides, addressed by [`OverridePath`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<Override>,
    /// Values for the component's exposed props (text, bool, variant, swap),
    /// keyed by [`ComponentPropId`]. Unset props use the def's default.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub prop_values: BTreeMap<ComponentPropId, VarValue>,
    /// Figma's **baked, fully-resolved per-descendant render data** for this
    /// placement (`derivedSymbolData`, Kiwi field 125). Figma pre-computes the
    /// exact transform/size/geometry/text-layout each descendant should render
    /// with *for this instance* and stores it in the `.fig`; faithful importers
    /// APPLY it instead of re-deriving from the light master. Empty for
    /// hand-authored instances and old files — [`expand_instance`] then falls
    /// back to the master values, so this is fully back-compatible.
    ///
    /// [`expand_instance`]: crate::resolve::expand_instance
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub derived: Vec<DerivedOverride>,
    /// The instance's own box size. Independent of the master's size so an
    /// instance can be resized; used for bounds and hit-testing.
    pub local_size: [f64; 2],
}

/// One entry of an instance's baked `derivedSymbolData`: the fully-resolved
/// render values Figma computed for a single descendant of the expanded subtree.
///
/// Addressed by the same def-local [`OverridePath`] the sparse [`Override`]s use
/// (the terminal master-descendant node, root excluded). Every payload field is
/// optional — a real `.fig` entry populates only the subset that actually changed
/// from the master (size+transform for a moved child, `path_data` for a
/// re-resolved vector, font fields for a text run). [`expand_instance`]
/// overwrites only the present fields onto the matched clone and leaves the rest
/// at the master's value.
///
/// [`expand_instance`]: crate::resolve::expand_instance
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DerivedOverride {
    /// Def-local path of the target descendant (master ids, root-first, root
    /// excluded) — exactly what [`crate::resolve::ExpandedNode`]'s `def_path`
    /// carries.
    pub path: OverridePath,
    /// Resolved local transform for the target. Replaces the clone's
    /// `transform` (Figma bakes the per-instance position/scale here).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<Transform2D>,
    /// Resolved local box size `[w, h]`. Updates the target's `local_size`
    /// (text/bitmap/instance) where it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<[f64; 2]>,
    /// Resolved fill paint stack. Present only when Figma baked per-instance
    /// fills (rare — most theme fills flow through variable/mode resolution);
    /// applied to a vector target when populated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fills: Option<SmallVec<[Fill; 1]>>,
    /// Resolved fill geometry (decoded from the entry's `fillGeometry` command
    /// blobs). Replaces a vector target's `path`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_data: Option<PathData>,
    /// Resolved stroke geometry (decoded from `strokeGeometry`). Carried for
    /// completeness; the renderer strokes from width, so this is informational.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_path: Option<PathData>,
    /// Resolved stroke weight. Updates the target's first stroke width.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_weight: Option<f64>,
    /// Resolved baked text layout for a text target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<DerivedText>,
}

/// The resolved text fields Figma bakes for a text descendant of an instance
/// (the scalar parts of an entry's text layout the doc model can apply). Each is
/// optional; present fields overwrite the master text node's
/// [`TextStyle`](crate::node::TextStyle) / content. The full glyph run
/// (`derivedTextData.glyphs`) is not modelled in the doc layer — we re-shape from
/// `content` + style — but the resolved size / spacing keep instance text laid
/// out as Figma resolved it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DerivedText {
    /// Resolved character content, when baked. Replaces the text node's
    /// `content`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Resolved font size in px.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    /// Resolved line height as a unitless multiple of font size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_height: Option<f64>,
    /// Resolved extra letter spacing in px.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub letter_spacing: Option<f64>,
    /// Resolved glyph fill color for this placement. Figma bakes the per-instance
    /// text color into the derived entry's `fillPaints` (first solid paint) — the
    /// reason a label on a Dark page must render in its real (light) color rather
    /// than the light master's near-black default. Replaces
    /// [`TextStyle::color`](crate::node::TextStyle::color).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    /// Resolved OpenType numeric weight (100–900) for this placement, derived from
    /// the entry's `derivedTextData`/`fontName` style. Replaces
    /// [`TextStyle::weight`](crate::node::TextStyle::weight).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<u16>,
    /// Resolved primary font family for this placement, from the entry's
    /// `derivedTextData`/`fontName`. Replaces
    /// [`TextStyle::font_family`](crate::node::TextStyle::font_family).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
}

/// A def-local path to a descendant of a component master, root-first.
///
/// `[]` (or `[root]`) addresses the master root itself; `[root, child, …]`
/// walks down. `SmallVec<[NodeId; 2]>` because override targets are usually one
/// or two levels deep — Figma's own overrides are overwhelmingly shallow.
pub type OverridePath = SmallVec<[NodeId; 2]>;

/// One instance override: at `target_path` (a def-local node path), set
/// `target_prop` to `value`. `target_prop` is the same [`BoundProp`] address
/// variable bindings use, so the two systems share a property vocabulary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Override {
    pub target_path: OverridePath,
    pub target_prop: BoundProp,
    pub value: OverrideValue,
}

/// The new value an [`Override`] installs. Distinct from [`VarValue`] because
/// some overrides carry structured payloads (a whole fill stack, an
/// instance-swap target) that a scalar value can't express.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OverrideValue {
    /// Replace text content.
    Text { value: String },
    /// Replace the whole fill stack.
    Fills { fills: SmallVec<[Fill; 1]> },
    /// Replace the whole stroke stack. Empty means "explicitly unstroked".
    Strokes { strokes: SmallVec<[Stroke; 1]> },
    /// Show / hide the targeted descendant.
    Visible { value: bool },
    /// Swap a nested instance for a different component.
    SwapInstance { component: ComponentId },
    /// Generic JSON field write — the escape hatch for anything the typed arms
    /// above don't cover yet. Carried opaquely so the override set can grow
    /// without a migration.
    Field { value: serde_json::Value },
}
