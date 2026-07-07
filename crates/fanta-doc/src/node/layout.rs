//! Auto-layout (Figma "stack" / flexbox) model — Stage 1: model + import only.
//!
//! The per-frame [`AutoLayout`] config lives on a frame-like
//! [`GroupNode`](crate::node::GroupNode); the per-child [`LayoutChild`] side
//! lives on the [`CanvasNode`](crate::node::CanvasNode) wrapper because any node
//! variant can be an auto-layout child. A Stage-2 layout pass
//! ([`crate::layout`]) consumes both.

use crate::node::common::{is_false, is_zero_f32};
use serde::{Deserialize, Serialize};

/// Primary (main-axis) flow direction of an [`AutoLayout`] container.
///
/// Maps from Figma's `stackMode` (`HORIZONTAL`/`VERTICAL`; `NONE` means the
/// frame is *not* auto-layout, so it carries no [`AutoLayout`] at all, and
/// `GRID` is not yet modelled — a grid frame imports without auto-layout).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutMode {
    /// Children flow left→right along the x axis (CSS `flex-direction: row`).
    #[default]
    Horizontal,
    /// Children flow top→bottom along the y axis (CSS `flex-direction: column`).
    Vertical,
}

/// How an [`AutoLayout`] axis sizes itself: a fixed extent, or hug-to-content.
///
/// Maps from Figma's `stackPrimarySizing` / `stackCounterSizing` (`StackSize`):
/// `FIXED` → [`AxisSizing::Fixed`]; `RESIZE_TO_FIT` /
/// `RESIZE_TO_FIT_WITH_IMPLICIT_SIZE` → [`AxisSizing::Hug`]. A HUG axis is the
/// one whose extent the Stage-2 layout pass must *compute* from the children
/// (unless Figma baked an explicit size into the frame's own NodeChange, which
/// for this fixture it does — see the diagnosis).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AxisSizing {
    /// The axis has an authored, fixed extent. The default.
    #[default]
    Fixed,
    /// The axis shrinks/grows to fit its content (Figma "Hug contents").
    Hug,
}

/// Alignment/distribution of children along the PRIMARY (main) axis.
///
/// Maps from Figma's `stackPrimaryAlignItems` (falling back to the legacy
/// `stackJustify`) — `StackJustify`. `SPACE_EVENLY` collapses to
/// [`PrimaryAlign::SpaceBetween`] (same as OpenPencil's `mapJustify`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrimaryAlign {
    /// Pack children at the start of the axis (CSS `justify-content: flex-start`).
    #[default]
    Start,
    /// Center children along the axis.
    Center,
    /// Pack children at the end of the axis.
    End,
    /// Distribute free space between children (CSS `space-between`).
    SpaceBetween,
}

/// Alignment of children along the COUNTER (cross) axis.
///
/// Maps from Figma's `stackCounterAlignItems` (falling back to the legacy
/// `stackCounterAlign`) — `StackAlign`/`StackCounterAlign`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CounterAlign {
    /// Align children to the start of the cross axis (CSS `align-items: flex-start`).
    #[default]
    Start,
    /// Center children on the cross axis.
    Center,
    /// Align children to the end of the cross axis.
    End,
    /// Stretch children to fill the cross axis (CSS `align-items: stretch`).
    Stretch,
    /// Align children's text baselines (Figma `BASELINE`).
    Baseline,
}

/// The auto-layout (flexbox / "stack") configuration of a frame-like
/// [`GroupNode`](crate::node::GroupNode). Field-for-field a faithful capture of
/// Figma's `stack*` node properties, kept renderer-agnostic in the doc layer.
///
/// Stage 1 imports this; a Stage-2 layout pass consumes it to compute child
/// positions/sizes and HUG-axis frame extents. Carrying it explicitly (rather
/// than only the baked `derivedSymbolData`) is what lets the layout pass reflow
/// when a child changes — and what an authoring UI edits.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AutoLayout {
    /// Primary flow direction.
    pub mode: LayoutMode,
    /// Gap between adjacent children along the primary axis, in logical px
    /// (Figma `stackSpacing`). When [`PrimaryAlign::SpaceBetween`] is set, Figma
    /// ignores this and distributes free space instead.
    #[serde(default)]
    pub spacing: f64,
    /// Gap between wrapped rows/columns on the counter axis (Figma
    /// `stackCounterSpacing`); only meaningful when wrapping is enabled.
    #[serde(default)]
    pub counter_spacing: f64,
    /// Inner padding `[top, right, bottom, left]`, in logical px. Resolved from
    /// Figma's `stackPadding*` / `stack{Horizontal,Vertical}Padding` fields with
    /// the same fallback chain OpenPencil uses.
    #[serde(default)]
    pub padding: [f64; 4],
    /// Distribution of children along the primary axis.
    #[serde(default)]
    pub primary_align: PrimaryAlign,
    /// Alignment of children along the counter axis.
    #[serde(default)]
    pub counter_align: CounterAlign,
    /// Primary-axis sizing of the frame itself (Fixed vs Hug-content).
    #[serde(default)]
    pub primary_sizing: AxisSizing,
    /// Counter-axis sizing of the frame itself (Fixed vs Hug-content).
    #[serde(default)]
    pub counter_sizing: AxisSizing,
    /// Whether children wrap onto multiple rows/columns (Figma `stackWrap`).
    #[serde(default)]
    pub wrap: bool,
    /// Whether auto-layout flow should consume the scene children in reverse
    /// order. This is an explicit compatibility escape hatch; native Figma
    /// imports keep it `false` because the `.fig` child order already matches
    /// Figma's visual auto-layout order.
    #[serde(default)]
    pub flow_reverse: bool,
    /// Whether per-child [`LayoutChild`] data (FILL/grow, absolute positioning,
    /// align-self) participates in this frame's flow. OpenPencil only applies
    /// Figma `stackChild*` sizing when the parent has an explicit H/V stack mode;
    /// inferred layouts from gap/padding ignore it.
    #[serde(default = "default_child_layout")]
    pub child_layout: bool,
    /// Whether the last child paints first (Figma `stackReverseZIndex` / "Last
    /// on top" toggle). Informational for Stage 2 paint order.
    #[serde(default)]
    pub reverse_z: bool,
}

const fn default_child_layout() -> bool {
    true
}

impl Default for AutoLayout {
    fn default() -> Self {
        Self {
            mode: LayoutMode::default(),
            spacing: 0.0,
            counter_spacing: 0.0,
            padding: [0.0; 4],
            primary_align: PrimaryAlign::default(),
            counter_align: CounterAlign::default(),
            primary_sizing: AxisSizing::default(),
            counter_sizing: AxisSizing::default(),
            wrap: false,
            flow_reverse: false,
            child_layout: true,
            reverse_z: false,
        }
    }
}

/// Per-child layout participation inside an auto-layout parent. Lives on the
/// [`CanvasNode`](crate::node::CanvasNode) wrapper because any node variant can
/// be an auto-layout child.
///
/// Only present (`Some`) on a child that carries non-default layout data —
/// absent ⇒ the child takes the parent's defaults (auto-positioned, no grow,
/// `alignSelf: AUTO`). Maps from the child NodeChange's `stackChildPrimaryGrow`,
/// `stackPositioning`, and `stackChildAlignSelf`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LayoutChild {
    /// Flex grow factor along the parent's primary axis (Figma
    /// `stackChildPrimaryGrow`). `> 0` ⇒ the child FILLs available primary space
    /// (CSS `flex-grow`); `0` ⇒ it keeps its own primary extent.
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub grow: f32,
    /// `true` ⇒ the child is taken out of the flow and positioned absolutely
    /// (Figma `stackPositioning == ABSOLUTE`); the layout pass leaves its
    /// transform/size untouched. `false` ⇒ normal auto-layout participation.
    #[serde(default, skip_serializing_if = "is_false")]
    pub absolute: bool,
    /// Per-child counter-axis alignment override (Figma `stackChildAlignSelf`);
    /// `None` ⇒ inherit the parent's [`AutoLayout::counter_align`] (Figma `AUTO`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align_self: Option<CounterAlign>,
}

impl LayoutChild {
    /// Whether this carries no non-default data (so it can be dropped to `None`
    /// for a byte-stable round-trip).
    pub fn is_trivial(&self) -> bool {
        self.grow == 0.0 && !self.absolute && self.align_self.is_none()
    }
}
