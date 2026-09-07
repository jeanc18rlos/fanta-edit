//! Prototype interactions — triggers, actions, transitions, and overlays.
//!
//! One [`Reaction`] is a [`Trigger`] firing an [`Action`], optionally with a
//! [`Transition`]. Reactions live on [`CanvasNode::reactions`].
//!
//! [`CanvasNode::reactions`]: crate::node::CanvasNode::reactions

use crate::id::{AnimationClipId, ComponentId, NodeId, ReactionId, VariableId};
use crate::value::VarValue;
use serde::{Deserialize, Serialize};

/// One prototype interaction: a [`Trigger`] that fires an [`Action`], optionally
/// with a frame [`Transition`] and an independent property animation. Lives on
/// [`CanvasNode::reactions`].
///
/// [`CanvasNode::reactions`]: crate::node::CanvasNode::reactions
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reaction {
    pub id: ReactionId,
    pub trigger: Trigger,
    /// The primary (first) action. Kept as a plain field — not folded into a
    /// list — so every existing consumer keeps compiling and docs authored
    /// before multi-action support serialize byte-identically.
    pub action: Action,
    /// Additional actions fired by the same trigger, in authored order after
    /// `action` (Figma interactions carry an `actions` array; "set variable
    /// then navigate" is the canonical pair). Deliberately additive: absent
    /// from the serialized form when empty so pre-existing documents
    /// round-trip unchanged. Consumers should iterate [`Reaction::actions`]
    /// rather than reading `action` alone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_actions: Vec<Action>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<Transition>,
    /// Optional property animation started by this reaction after its action
    /// has executed. This is deliberately independent from `transition`: frame
    /// transitions composite navigation/overlay surfaces, while this binding
    /// samples an authored clip's per-node property tracks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animation: Option<PrototypeAnimation>,
}

impl Reaction {
    /// Every action this reaction fires, primary first, in execution order.
    /// The runtime executes these sequentially in one gesture: each action
    /// sees the state its predecessors produced, and an action that changes
    /// the current frame (a Navigate/Back that lands somewhere new) ends the
    /// sequence — the remaining actions belonged to the frame that was left.
    pub fn actions(&self) -> impl Iterator<Item = &Action> {
        std::iter::once(&self.action).chain(self.extra_actions.iter())
    }
}

/// Bind one authored motion clip to a prototype reaction.
///
/// The action executes immediately when the trigger fires. The clip then
/// samples against the resulting presentation state after `delay_ms`, so a
/// navigation reaction naturally animates nodes in its destination frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrototypeAnimation {
    pub clip: AnimationClipId,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub delay_ms: u32,
}

fn is_zero(value: &u32) -> bool {
    *value == 0
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
    /// Continuous while pointer is pressed on the node (Figma WHILE_PRESSING).
    WhilePressing,
    /// The pointer entered the node's bounds (Figma MOUSE_ENTER / MOUSE_IN).
    /// Behaviorally identical to [`Trigger::Hover`] at runtime — fires once on
    /// entry and re-arms after the pointer leaves — but kept distinct so the
    /// authored trigger survives round-trips.
    MouseEnter,
    /// The pointer left the node's bounds (Figma MOUSE_LEAVE / MOUSE_OUT).
    /// Fires once on the exit edge only — previously these imported as
    /// [`Trigger::Hover`], making leave-triggered closes fire on entry.
    MouseLeave,
    /// Continuous while the pointer hovers the node (Figma WHILE_HOVERING):
    /// fires on entry and its reversible effects (variant swaps) are undone
    /// when the pointer leaves — the hover-side mirror of
    /// [`Trigger::WhilePressing`].
    WhileHovering,
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
    /// Update a component variant (for prototype variant switching).
    UpdateVariant {
        component: ComponentId,
        variant: String, // or more structured
    },
    /// Open external link (for prototype web actions).
    OpenLink { url: String },
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
    /// Interpolate matched layers (by name) between the two frames — position,
    /// size, rotation, and opacity animate rather than cutting/dissolving.
    SmartAnimate,
    /// The OLD frame slides away toward `direction`, revealing the new frame
    /// beneath (which drifts a small parallax into place) — the mirrored
    /// counterpart of [`TransitionStyle::SlideIn`]. `direction` names the edge
    /// the outgoing content exits TOWARD, matching Figma's "Slide out to
    /// left/right/top/bottom" authoring language.
    SlideOut { direction: Direction },
    /// The OLD frame moves away toward `direction` over the stationary new
    /// frame beneath — the mirrored counterpart of
    /// [`TransitionStyle::MoveIn`]. `direction` names the exit edge, as with
    /// [`TransitionStyle::SlideOut`].
    MoveOut { direction: Direction },
    /// Ease the scroll offset to the target instead of jumping (Figma's
    /// "Scroll animate" on a Scroll-to action). On a frame navigation this has
    /// no directional geometry of its own and degrades to a dissolve.
    ScrollAnimate,
}

/// The edge a directional [`TransitionStyle`] enters from.
///
/// This follows Figma's authoring language: [`Direction::Left`] means "from
/// left", so the incoming content starts to the left of the viewport and
/// travels right toward its resting position.
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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Easing {
    Linear,
    EaseIn,
    EaseOut,
    #[default]
    EaseInOut,
    /// Standard CSS cubic-bezier(x1, y1, x2, y2). Values typically in 0..1 but
    /// can go outside for overshoot effects.
    CubicBezier {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
    },
    /// Physically-modeled damped spring (Figma's spring presets and custom
    /// mass/stiffness/damping curves). Sampled by [`spring_progress`]: the
    /// motion is time-normalized so the spring settles at 1.0 by the end of
    /// the transition's duration; an under-damped spring overshoots past 1.0
    /// on the way (the whole point of a bouncy easing).
    Spring {
        mass: f32,
        stiffness: f32,
        damping: f32,
    },
}

/// Closed-form damped-spring step response, sampled at normalized time
/// `t ∈ [0, 1]` and normalized so the spring has settled (within ~1e-4 of 1.0)
/// exactly at `t = 1`.
///
/// WHY closed-form rather than integrating: transitions are progress-driven
/// (`elapsed / duration`), so easing must be a pure `t → position` function —
/// the same shape as the cubic-bezier solver — evaluable at any `t` without
/// history.
///
/// WHY time-normalized: a real spring has its own settling time (seconds); a
/// transition has an authored duration. Figma scales the spring so the motion
/// fits the duration; we do the same by computing the time `T` at which the
/// spring's decay envelope reaches 1e-4 and sampling the physical solution at
/// `t * T`. The envelope target is deliberately below the 1e-3 accuracy we
/// promise at `t = 1` because the oscillatory/second-root factor multiplying
/// it can exceed 1.
///
/// The three damping regimes use the standard unit-step ODE solutions for
/// `m·x″ + c·x′ + k·x = k` with `x(0) = 0, x′(0) = 0`:
/// - under-damped (ζ < 1):  `1 − e^(−ζωt)·(cos(ωd·t) + (ζω/ωd)·sin(ωd·t))`
/// - critically damped:      `1 − e^(−ωt)·(1 + ωt)`
/// - over-damped (ζ > 1):    `1 − (s₂·e^(s₁t) − s₁·e^(s₂t)) / (s₂ − s₁)`
///
/// Degenerate parameters (non-positive mass/stiffness/damping, NaN) fall back
/// to linear progress so a malformed document still animates monotonically.
pub fn spring_progress(mass: f32, stiffness: f32, damping: f32, t: f64) -> f64 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    let m = f64::from(mass);
    let k = f64::from(stiffness);
    let c = f64::from(damping);
    if !m.is_finite() || m <= 0.0 || !k.is_finite() || k <= 0.0 || !c.is_finite() || c <= 0.0 {
        return t;
    }

    // Natural frequency and damping ratio of the (m, k, c) system.
    let omega = (k / m).sqrt();
    let zeta = c / (2.0 * (k * m).sqrt());

    // Settle window: the time for the slowest decay envelope to fall to 1e-4.
    // Under/critical damping decay at rate ζω; over-damping's slow root decays
    // at ω(ζ − sqrt(ζ² − 1)).
    const SETTLE: f64 = 1e-4;
    let ln_target = -SETTLE.ln(); // ln(10^4) ≈ 9.21
    let slow_rate = if zeta < 1.0 {
        zeta * omega
    } else {
        omega * (zeta - (zeta * zeta - 1.0).sqrt()).max(f64::EPSILON)
    };
    if !slow_rate.is_finite() || slow_rate <= 0.0 {
        return t;
    }
    let time = t * (ln_target / slow_rate);

    if zeta < 1.0 - 1e-9 {
        // Under-damped: decaying oscillation about 1.0 — this branch is what
        // produces the characteristic overshoot.
        let omega_d = omega * (1.0 - zeta * zeta).sqrt();
        let envelope = (-zeta * omega * time).exp();
        1.0 - envelope
            * ((omega_d * time).cos() + (zeta * omega / omega_d) * (omega_d * time).sin())
    } else if zeta <= 1.0 + 1e-9 {
        // Critically damped: fastest non-overshooting approach.
        1.0 - (-omega * time).exp() * (1.0 + omega * time)
    } else {
        // Over-damped: two real decaying exponentials, no overshoot.
        let discriminant = (zeta * zeta - 1.0).sqrt();
        let s1 = -omega * (zeta - discriminant); // slow root
        let s2 = -omega * (zeta + discriminant); // fast root
        1.0 - (s2 * (s1 * time).exp() - s1 * (s2 * time).exp()) / (s2 - s1)
    }
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
