//! Prototype interactions — triggers, actions, transitions, and overlays.
//!
//! One [`Reaction`] is a [`Trigger`] firing an [`Action`], optionally with a
//! [`Transition`]. Reactions live on [`CanvasNode::reactions`].
//!
//! [`CanvasNode::reactions`]: crate::node::CanvasNode::reactions

use crate::id::{NodeId, ReactionId, VariableId};
use crate::value::VarValue;
use serde::{Deserialize, Serialize};

/// One prototype interaction: a [`Trigger`] that fires an [`Action`], optionally
/// with a [`Transition`]. Lives on [`CanvasNode::reactions`].
///
/// [`CanvasNode::reactions`]: crate::node::CanvasNode::reactions
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reaction {
    pub id: ReactionId,
    pub trigger: Trigger,
    pub action: Action,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<Transition>,
}

/// What user input fires a [`Reaction`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    /// Pointer click / tap.
    Click,
    /// Pointer drag.
    Drag,
    /// Pointer hover (enter).
    Hover,
    /// Fires automatically after `delay_ms` on entering the frame.
    AfterDelay { delay_ms: u32 },
    /// One of the named keys is pressed. Key names are opaque strings.
    Key { keys: Vec<String> },
}

/// What a [`Reaction`] does when its [`Trigger`] fires.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// Navigate to another frame. A stale `to` (frame deleted) is tolerated —
    /// present-mode hit-testing simply does nothing; `validate` warns.
    Navigate { to: NodeId },
    /// Go back to the previously-navigated frame.
    Back,
    /// Close the current overlay (or exit present mode at the top level).
    Close,
    /// Open `overlay` on top of `frame`, positioned per `overlay`'s settings.
    OpenOverlay {
        frame: NodeId,
        overlay: OverlaySettings,
    },
    /// Scroll the named target into view.
    ScrollTo { target: NodeId },
    /// Set a design-system variable to a value (the typed-value reuse point).
    SetVariable {
        variable: VariableId,
        value: VarValue,
    },
}

/// Animation applied while a navigation/overlay action plays out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    pub style: TransitionStyle,
    pub duration_ms: u32,
    #[serde(default)]
    pub easing: Easing,
}

/// How a [`Transition`] animates the frame swap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TransitionStyle {
    /// Immediate cut, no animation. The default.
    #[default]
    Instant,
    /// Cross-dissolve.
    Dissolve,
    /// New frame slides in from `direction`.
    SlideIn { direction: Direction },
    /// New frame pushes the old out from `direction`.
    Push { direction: Direction },
    /// New frame moves in over the old from `direction`.
    MoveIn { direction: Direction },
}

/// Cardinal direction for directional [`TransitionStyle`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    #[default]
    Left,
    Right,
    Up,
    Down,
}

/// Timing curve for a [`Transition`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Easing {
    Linear,
    EaseIn,
    EaseOut,
    #[default]
    EaseInOut,
}

/// Placement and behavior of an overlay opened by [`Action::OpenOverlay`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverlaySettings {
    pub position: OverlayPosition,
    /// Dim the backdrop behind the overlay.
    #[serde(default)]
    pub background_dim: bool,
    /// Clicking outside the overlay closes it.
    #[serde(default)]
    pub close_on_click_outside: bool,
}

/// Where an overlay sits relative to the viewport / trigger.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OverlayPosition {
    Center,
    /// Anchored relative to the triggering element, offset by `offset`.
    Manual {
        offset: [f64; 2],
    },
    TopLeft,
    TopCenter,
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}
