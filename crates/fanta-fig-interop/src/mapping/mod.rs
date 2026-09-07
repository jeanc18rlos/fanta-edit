//! First-pass `.fig` → Fantaisa [`Doc`] mapping.
//!
//! This is the semantic layer: it walks the dynamic [`KiwiValue`] tree produced
//! by [`crate::fig::read_fig`] and reconstructs a Fantaisa scene graph plus its
//! design-system side-tables (components, variants, variables, prototype flow).
//! It is deliberately *partial* — an unmappable field is skipped + counted,
//! never fatal — but it now recognizes far more than the v0 "everything → Group,
//! VECTOR skipped" pass:
//!
//! | Figma `NodeType`                       | Fantaisa node                          |
//! |----------------------------------------|----------------------------------------|
//! | `CANVAS`                               | [`NodeData::Group`] + registered page  |
//! | `FRAME`/`SECTION`                      | [`NodeData::Group`] (clip set)         |
//! | `GROUP`                                | [`NodeData::Group`]                    |
//! | `SYMBOL` (main component)              | [`NodeData::Group`] master + [`ComponentDef`] under a hidden Components page |
//! | `SYMBOL` w/ `isStateGroup` / `COMPONENT_SET` | [`ComponentSet`] (variant group) |
//! | `INSTANCE`                             | [`NodeData::Instance`] (or Group fallback) |
//! | `RECTANGLE`, `ROUNDED_RECTANGLE`       | [`NodeData::Vector`] rectangle         |
//! | `ELLIPSE`                              | [`NodeData::Vector`] ellipse           |
//! | `VECTOR`/`STAR`/`LINE`/`BOOLEAN_OPERATION`/`REGULAR_POLYGON` | [`NodeData::Vector`] — real path decoded from command blobs, else bbox fallback |
//! | `TEXT`                                 | [`NodeData::Text`] (string + style)    |
//! | `VARIABLE_SET`                         | [`VariableCollection`] (+ modes)       |
//! | `VARIABLE`                             | [`Variable`] (per-mode values)         |
//! | `ANIMATION_PRESET_INSTANCE` / `KEYFRAME_TRACK` / `KEYFRAME` | [`fanta_doc::MotionLibrary`] clips/tracks/keyframes (see `motion.rs`) |
//! | `DOCUMENT`                             | (implicit root; not itself a node)     |
//! | everything else                        | skipped, counted in [`MapReport`]      |
//!
//! Per-node prototype interactions become [`CanvasNode::reactions`]; a node's
//! `variableConsumptionMap` becomes [`CanvasNode::bindings`]; the document's
//! `prototypeStartNodeID` becomes [`Doc::flow_start`].
//!
//! ## How Figma stores a scene (and how we invert it)
//!
//! Figma does not nest nodes; it stores a *flat* `nodeChanges` array. Each node
//! carries a `guid` (a `{sessionID, localID}` pair) and a `parentIndex` holding
//! the parent's `guid` plus a `position` string (a fractional index for
//! z-order — the same idea as our [`IndexKey`]). We do a multi-pass build:
//!
//! 1. First pass: create a Fantaisa node for every recognized change, recording
//!    `guid -> NodeId`, and collect side-table material (component defs/sets,
//!    instance refs, variable collections/variables, bindings, reactions).
//!    Unrecognized types are counted and skipped, but we still record their guid
//!    as "seen" so children that point at them can be re-parented to the nearest
//!    recognized ancestor instead of being dropped.
//! 2. Second pass: resolve each node's parent guid to a `NodeId` and attach it,
//!    walking up the original parent chain to skip over unrecognized intermediate
//!    nodes. A node whose nearest recognized ancestor is an [`NodeData::Instance`]
//!    is dropped (counted) — instance subtrees are *virtual*, produced on demand
//!    by [`fanta_doc::resolve::expand_instance`], so they must not be stored.
//! 3. Page pass: register every `CANVAS` as a page; relocate `SYMBOL` master
//!    subtrees under a hidden "Components" page; default to the largest content
//!    page.
//! 4. Side-table pass: resolve guid references to `NodeId`/`ComponentId` and
//!    populate [`Doc::components`], [`Doc::variables`], per-node bindings,
//!    reactions, and `flow_start`.
//!
//! Field names and enum members are taken from the real Figma `fig.kiwi` schema,
//! verified against the Adobe Spectrum community file.
//!
//! [`IndexKey`]: fanta_doc::index::IndexKey

// The crate-internal imports are re-exported `pub(crate)` so each focused
// submodule can `use super::*` and see both the shared external types and its
// sibling helpers, keeping this module a thin manifest (no logic).
pub(crate) use crate::error::{FigError, FigResult};
pub(crate) use crate::fig::FigDocument;
pub(crate) use crate::kiwi::KiwiValue;
pub(crate) use fanta_doc::NodeId;
pub(crate) use fanta_doc::binding::BoundProp;
pub(crate) use fanta_doc::color::{Color, Gradient, GradientStop};
pub(crate) use fanta_doc::doc::Doc;
pub(crate) use fanta_doc::id::{
    AssetId, ComponentId, ModeId, ReactionId, VariableCollectionId, VariableId,
};
pub(crate) use fanta_doc::index::IndexKey;
pub(crate) use fanta_doc::node::{
    Action, AutoLayout, AxisSizing, CanvasNode, CounterAlign, Easing, FontVariation, GroupNode,
    InstanceNode, LayoutChild, LayoutMode, MaskType, NodeData, NodeFlags, Override, OverridePath,
    OverrideValue, ParametricShape, PrimaryAlign, Reaction, ScrollBehavior, ScrollDirection,
    TextAlign, TextAutoResize, TextNode, TextStyle, TextStyleRun, Transition, TransitionStyle,
    Trigger, VAlign, VectorNode,
};
pub(crate) use fanta_doc::path::PathData;
pub(crate) use fanta_doc::style::{
    BlendMode, Blur, BlurKind, Fill, ImageAdjust, ImageFitMode, Shadow, ShadowKind, Stroke,
    StrokeAlign, StrokeCap, StrokeJoin,
};
pub(crate) use fanta_doc::transform::Transform2D;
pub(crate) use fanta_doc::value::{VarValue, VariableType};
pub(crate) use fanta_doc::variables::{Mode, Variable, VariableCollection};
pub(crate) use std::collections::HashMap;

mod auto_layout;
mod bindings;
mod components;
mod fields;
mod instance_overrides;
mod motion;
mod node_build;
mod orchestrator;
mod reactions;
mod report;
mod stroke;
mod text;
mod variables;
mod vector;

// Re-export every submodule's items crate-locally so siblings resolve each
// other through `use super::*`, and so the public surface below is preserved.
pub(crate) use auto_layout::*;
pub(crate) use bindings::*;
pub(crate) use components::*;
pub(crate) use fields::*;
pub(crate) use instance_overrides::*;
pub(crate) use motion::*;
pub(crate) use node_build::*;
pub(crate) use orchestrator::*;
pub(crate) use reactions::*;
pub(crate) use stroke::*;
pub(crate) use text::*;
pub(crate) use variables::*;
pub(crate) use vector::*;

// The crate's public mapping surface (unchanged external paths).
pub use orchestrator::fig_to_doc;
pub use report::MapReport;

#[cfg(test)]
mod tests;
