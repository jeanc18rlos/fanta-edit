//! Prototype-playback runtime for the Fanta engine.
//!
//! A [`PresentSession`] plays an imported Figma prototype: it dispatches pointer
//! / key / timer input to the [`Reaction`](fanta_doc::Reaction)s authored on
//! nodes, executes their [`Action`](fanta_doc::Action)s (navigate between
//! frames, open overlays, set variables), and composites
//! [`Transition`](fanta_doc::Transition)s by rendering the outgoing and incoming
//! frames and tweening between them, and samples reaction-bound motion clips as
//! an independent post-action property-animation channel.
//!
//! The session is **ephemeral state over an immutable [`Doc`]**: it never calls
//! `Doc::apply`. Navigation, variable overrides, and overlays all live on the
//! session; the document is the design and is left byte-identical. Rendering
//! reuses the [`RasterRenderer`] and the doc's viewport/hit-test math verbatim.

#![forbid(unsafe_code)]

mod overlay;
mod smart_animate;
mod transition;

use std::collections::{BTreeMap, HashMap};

use fanta_canvas::viewport::{fit_bounds, screen_to_world};
use fanta_doc::{
    Action, AnimationClipId, ComponentId, ComponentPropKind, Doc, ModeId, MotionEvaluation,
    NodeData, NodeFlags, NodeId, OverlaySettings, PrototypeAnimation, ReactionId, Scene,
    ScrollBehavior, ScrollDirection, Transform2D, Transition, TransitionStyle, Trigger, VarValue,
    VariableCollectionId, VariableId, VariableRegistry, Viewport,
};
use fanta_render::{RasterRenderer, RenderError, RenderInputs, RenderMetrics};
use glam::DVec2;

use crate::overlay::{SCRIM_DIM, ScreenRect, apply_scrim, composite_over, place_overlay};
use crate::smart_animate::SmartAnimate;
use crate::transition::{
    ActiveTransition, composite, composite_overlay, directional_offsets, ease, overlay_offset,
};

/// A present surface fits each frame edge-to-edge (no padding), like Figma's
/// present mode.
const FIT_PADDING: f64 = 0.0;
const TIME_EPSILON_SECONDS: f64 = 1e-9;
const MAX_TIMER_EVENTS_PER_TICK: usize = 4096;

#[derive(Debug, thiserror::Error)]
pub enum PresentError {
    /// No start frame: `start` was `None` and the doc has neither a flow start
    /// nor any page.
    #[error("prototype has no start frame (no flow start and no pages)")]
    NoStartFrame,
    /// The requested start frame is not in the scene.
    #[error("start frame is not in the scene")]
    StartNotFound,
    /// The render surface could not be allocated.
    #[error(transparent)]
    Render(#[from] RenderError),
}

/// A pointer gesture, in the host's terms. The session maps these to the
/// [`Trigger`] kinds the prototype authored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerEvent {
    /// The pointer moved (drives `Hover`).
    Move,
    /// A press-release in place (drives `Click`).
    Click,
    /// A drag began (drives `Drag`).
    DragStart,
    /// The primary pointer went down (drives `WhilePressing`).
    Down,
    /// The primary pointer was released and ends `WhilePressing` state.
    Up,
    /// The pointer left the presentation surface.
    Leave,
}

/// A key transition. Only `Down` fires `Trigger::Key`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEvent {
    Down,
    Up,
}

/// What the host should do after handing the session an event.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PresentResponse {
    /// The view changed — the host should re-render.
    pub needs_redraw: bool,
    /// The frame or overlay stack changed.
    pub navigated: bool,
    /// A frame transition or bound property animation is in flight — the host
    /// should keep calling [`tick`].
    ///
    /// [`tick`]: PresentSession::tick
    pub animating: bool,
    /// `Close` at the base level — the host should exit present mode.
    pub exited: bool,
    /// Media time or playing state changed (for video/audio UI update).
    pub media_updated: bool,
    /// A `WhilePressing` reaction consumed the press. Hosts should not synthesize
    /// a Click for the same gesture after pointer-up.
    pub suppress_click: bool,
}

/// A saved navigation step for [`Action::Back`].
#[derive(Debug, Clone)]
struct NavEntry {
    frame: NodeId,
    viewport: Viewport,
    /// Snapshot of scroll offsets at the time we left this frame (so Back can
    /// restore scroll position inside scrollable containers).
    scroll_snapshot: HashMap<NodeId, [f64; 2]>,
}

/// A pending [`Trigger::AfterDelay`] reaction: fires its actions once
/// `remaining` seconds elapse. `root` is the layer (frame or overlay) it
/// belongs to, so the timer is cancelled when that layer goes away.
#[derive(Debug, Clone)]
struct PendingTimer {
    root: NodeId,
    source: NodeId,
    reaction: ReactionId,
    /// The reaction's full action sequence (primary first), executed in order
    /// under the same stop-on-frame-change rule as pointer dispatch.
    actions: Vec<Action>,
    transition: Option<Transition>,
    animation: Option<PrototypeAnimation>,
    remaining: f64,
}

/// One open overlay on the stack (topmost last).
#[derive(Debug, Clone)]
struct OverlayEntry {
    root: NodeId,
    settings: OverlaySettings,
    /// The viewport that renders the overlay frame at the base zoom, placed per
    /// `settings.position`.
    viewport: Viewport,
    /// The overlay's rect on the present surface — decides click-outside.
    screen_rect: ScreenRect,
}

#[derive(Debug, Clone, Copy)]
struct TransitionClock {
    style: TransitionStyle,
    easing: fanta_doc::Easing,
    duration: f64,
    elapsed: f64,
}

impl TransitionClock {
    fn from_transition(transition: Option<&Transition>) -> Option<Self> {
        let transition = transition?;
        if matches!(transition.style, TransitionStyle::Instant) || transition.duration_ms == 0 {
            return None;
        }
        Some(Self {
            style: transition.style,
            easing: transition.easing,
            duration: f64::from(transition.duration_ms) / 1000.0,
            elapsed: 0.0,
        })
    }

    fn progress(self) -> f64 {
        if self.duration <= 0.0 {
            return 1.0;
        }
        (self.elapsed / self.duration).clamp(0.0, 1.0)
    }

    fn done(self) -> bool {
        self.elapsed + TIME_EPSILON_SECONDS >= self.duration
    }
}

#[derive(Debug, Clone, Copy)]
struct ActiveOverlayTransition {
    root: NodeId,
    clock: TransitionClock,
}

#[derive(Debug, Clone)]
struct ActiveOverlayExit {
    entry: OverlayEntry,
    clock: TransitionClock,
    start_visible: f64,
}

struct ActiveVariantTransition {
    layers: Vec<VariantLayerTransition>,
    clock: TransitionClock,
}

struct VariantLayerTransition {
    instance: NodeId,
    viewport: Viewport,
    from_scene: Scene,
    from_root: NodeId,
    to_scene: Scene,
    to_root: NodeId,
    smart_animate: Option<SmartAnimate>,
    target_instance: fanta_doc::CanvasNode,
    target_parent_world: Transform2D,
    target_master_opacity: f32,
}

#[derive(Debug, Clone, Copy)]
struct ActivePrototypeAnimation {
    clip: AnimationClipId,
    delay: f64,
    elapsed: f64,
}

impl ActivePrototypeAnimation {
    fn playhead_ms(self, doc: &Doc) -> Option<u32> {
        if self.elapsed + TIME_EPSILON_SECONDS < self.delay {
            return None;
        }
        let clip = doc.motion.clip(self.clip)?;
        let elapsed = ((self.elapsed - self.delay) * 1000.0)
            .round()
            .clamp(0.0, f64::from(u32::MAX)) as u32;
        Some(elapsed.min(clip.duration_ms))
    }

    fn is_running(self, doc: &Doc) -> bool {
        let Some(clip) = doc.motion.clip(self.clip) else {
            return false;
        };
        self.elapsed + TIME_EPSILON_SECONDS < self.delay + f64::from(clip.duration_ms) / 1000.0
    }
}

/// A reaction resolved by hit-test bubbling, cloned out of the scene so the
/// borrow is released before execution mutates the session. The `Vec<Action>`
/// is the reaction's full sequence (primary action first, then any
/// `extra_actions`) — PR-5 multi-action interactions execute every entry in
/// one gesture.
type MatchedReaction = (
    NodeId,
    ReactionId,
    Vec<Action>,
    Option<Transition>,
    Option<PrototypeAnimation>,
);

/// An eased scroll-offset interpolation (PR-6 `ScrollAnimate` on a ScrollTo):
/// instead of jumping, the container's offset is tweened from `from` to `to`
/// over `duration`, each sample routed through the clamped setter so the
/// animation can never overshoot the content bounds mid-flight.
#[derive(Debug, Clone, Copy)]
struct ActiveScrollAnimation {
    scrollable: NodeId,
    from: [f64; 2],
    to: [f64; 2],
    easing: fanta_doc::Easing,
    /// Total duration in seconds (always > 0 — a zero-duration scroll-animate
    /// is a plain jump and never becomes an `ActiveScrollAnimation`).
    duration: f64,
    elapsed: f64,
}

impl ActiveScrollAnimation {
    fn progress(self) -> f64 {
        if self.duration <= 0.0 {
            return 1.0;
        }
        (self.elapsed / self.duration).clamp(0.0, 1.0)
    }

    fn done(self) -> bool {
        self.elapsed + TIME_EPSILON_SECONDS >= self.duration
    }
}

#[derive(Debug, Clone)]
struct ActivePointerReaction {
    source: NodeId,
    reaction: ReactionId,
    variant_restore: Option<InstanceVariantRestore>,
}

#[derive(Debug, Clone)]
struct InstanceVariantRestore {
    instance: NodeId,
    previous: Option<(ComponentId, String)>,
    applied: (ComponentId, String),
}

/// Plays a prototype over an immutable [`Doc`].
pub struct PresentSession {
    doc: Doc,
    screen_size: DVec2,

    /// The current base frame's root node.
    current: NodeId,
    /// The current frame's viewport (fit to the frame; panned by `ScrollTo`).
    viewport: Viewport,
    /// Navigation history for `Back`.
    back_stack: Vec<NavEntry>,
    /// Open overlays, topmost last. Empty in the common case.
    overlays: Vec<OverlayEntry>,

    /// Session-local variable state (a clone of `doc.variables`), mutated by
    /// `SetVariable` so playback never touches the design. Bumped through
    /// `RenderInputs` on every render.
    variables: VariableRegistry,
    active_modes: BTreeMap<VariableCollectionId, ModeId>,
    mode_generation: u64,

    /// Seconds since the session started (advanced by [`tick`](Self::tick)).
    clock: f64,
    /// The navigation transition currently animating, if any. While set,
    /// [`present_rgba`](Self::present_rgba) composites the two endpoint frames.
    active_transition: Option<ActiveTransition>,
    /// The prepared smart-animate plan for the active transition, when its style
    /// is [`TransitionStyle::SmartAnimate`] — a scratch scene tweened per tick.
    smart_animate: Option<SmartAnimate>,
    /// The newest overlay entering over the already-composited presentation.
    active_overlay_transition: Option<ActiveOverlayTransition>,
    /// A removed overlay retained only as a visual layer while its close
    /// transition runs. Input and timers switch to the revealed layer at once.
    active_overlay_exit: Option<ActiveOverlayExit>,
    /// A component-variant state swap rendered between stable before/after
    /// override snapshots. This is separate from frame Smart Animate because
    /// instances resolve their component masters lazily during rendering.
    active_variant_transition: Option<ActiveVariantTransition>,
    /// Property-animation clip bound to the latest fired reaction. Completed
    /// clips remain here at their final playhead so the last sample is held.
    active_prototype_animation: Option<ActivePrototypeAnimation>,
    /// Pending `AfterDelay` reactions on the active layers, counted down by
    /// [`tick`](Self::tick).
    timers: Vec<PendingTimer>,

    /// Global media playback time in seconds (for VideoNode/AudioNode clips).
    /// Advanced by tick when playing. Hosts can seek via set_media_time (global)
    /// or set_node_media_time for per-clip independent control.
    media_time: f64,
    media_playing: bool,

    /// Per-node overrides for media time (for independent per-clip seek/scrub
    /// of specific Video/Audio nodes). Falls back to global when absent.
    node_media_times: HashMap<NodeId, f64>,

    /// Scroll offsets for scrollable containers (key = the scrollable Group's NodeId).
    /// Value is [scroll_x, scroll_y] (in the scrollable's local space) to apply
    /// when viewing/rending/hit-testing content inside it. Set by ScrollTo when
    /// target is descendant of a `scrollable` frame (enables prototype scrolling
    /// inside containers without always panning the outer viewport).
    scroll_offsets: HashMap<NodeId, [f64; 2]>,

    /// Variant overrides set during prototype playback via UpdateVariant actions.
    /// Keys are the component set or def id; value is the selected variant name.
    /// This makes prototype-driven component variant switching "live" without
    /// mutating the source Doc (matches Figma prototype behavior).
    /// Hosts may consult variant_overrides() when choosing masters or building
    /// custom RenderInputs for present-mode rendering of instances.
    variant_overrides: HashMap<ComponentId, String>,
    /// Variant changes authored on an instance affect that instance only. The
    /// component-keyed map above remains the explicit host-level/global API.
    instance_variant_overrides: HashMap<NodeId, (ComponentId, String)>,

    hovered_reaction: Option<ActivePointerReaction>,
    pressed_reaction: Option<ActivePointerReaction>,
    suppress_next_click: bool,

    /// The ancestor chain of the node currently under the pointer (hit node
    /// first, walking up to the root), updated on every Move. This is the
    /// hover-tracking state that knows the previously-hovered set: a node
    /// present in the old chain but absent from the new one just had the
    /// pointer LEAVE its bounds — the edge [`Trigger::MouseLeave`] fires on.
    hover_chain: Vec<NodeId>,

    /// An in-flight eased scroll (`ScrollAnimate` on a ScrollTo), advanced by
    /// [`tick`](Self::tick) alongside the other transition clocks.
    active_scroll_animation: Option<ActiveScrollAnimation>,

    pending_url: Option<String>,

    renderer: RasterRenderer,
}

impl PresentSession {
    /// Start a session. `start` names the first frame; `None` falls back to the
    /// doc's flow start, then its active page. `screen_size` is the present
    /// surface size in logical pixels.
    pub fn new(doc: &Doc, start: Option<NodeId>, screen_size: DVec2) -> Result<Self, PresentError> {
        let screen_size = valid_screen_size(screen_size);
        let current = start
            .or_else(|| doc.flow_start())
            // No authored flow: Figma's player opens the page's FIRST TOP-LEVEL
            // FRAME, never the page itself (presenting the page node would fit
            // every frame at once). Fall back to the bare page only when it has
            // no frame children at all.
            .or_else(|| {
                let page = doc.active_page()?;
                doc.scene
                    .children_of(Some(page))
                    .iter()
                    .copied()
                    .find(|child| {
                        doc.scene.get(*child).is_some_and(|node| {
                            matches!(node.data, NodeData::Group(_))
                                && !node.flags.contains(NodeFlags::HIDDEN)
                        })
                    })
                    .or(Some(page))
            })
            .ok_or(PresentError::NoStartFrame)?;
        if !doc.scene.contains(current) {
            return Err(PresentError::StartNotFound);
        }
        let width = (screen_size.x.max(1.0)) as u32;
        let height = (screen_size.y.max(1.0)) as u32;
        let mut renderer = RasterRenderer::new(width, height)?;
        // Honor the authored presentation device (Figma `prototypeDevice`):
        // the surround/chrome color paints the letterbox area around the
        // fitted frame. The device box itself is available to hosts via
        // [`presentation`](Self::presentation) for real chrome/letterboxing.
        if let Some(config) = &doc.presentation
            && let Some(frame_color) = config.frame_color
        {
            renderer.background = frame_color;
        }
        let viewport = frame_viewport(&doc.scene, current, screen_size);
        let mut session = Self {
            doc: doc.clone(),
            screen_size,
            current,
            viewport,
            back_stack: Vec::new(),
            overlays: Vec::new(),
            variables: doc.variables.clone(),
            active_modes: doc.active_modes.clone(),
            mode_generation: 0,
            clock: 0.0,
            active_transition: None,
            smart_animate: None,
            active_overlay_transition: None,
            active_overlay_exit: None,
            active_variant_transition: None,
            active_prototype_animation: None,
            timers: Vec::new(),
            media_time: 0.0,
            media_playing: false,
            node_media_times: HashMap::new(),
            scroll_offsets: HashMap::new(),
            variant_overrides: HashMap::new(),
            instance_variant_overrides: HashMap::new(),
            hovered_reaction: None,
            pressed_reaction: None,
            suppress_next_click: false,
            hover_chain: Vec::new(),
            active_scroll_animation: None,
            pending_url: None,
            renderer,
        };
        session.seed_initial_scroll_offsets(current);
        session.schedule_timers(current);
        Ok(session)
    }

    /// Install an asset resolver so image fills / bitmaps render (delegates to
    /// the internal renderer). Optional — vector-only prototypes need none.
    pub fn set_asset_resolver(
        &mut self,
        resolver: std::sync::Arc<dyn fanta_render::AssetResolver>,
    ) {
        self.renderer.set_asset_resolver(resolver);
    }

    /// The current base frame.
    pub fn current_frame(&self) -> NodeId {
        self.current
    }

    /// The authored presentation device (Figma `prototypeDevice`), when the
    /// document carries one — hosts letterbox frames into
    /// `device_size` and paint device chrome from it.
    pub fn presentation(&self) -> Option<&fanta_doc::PresentationConfig> {
        self.doc.presentation.as_ref()
    }

    /// The document's named prototype flows (entry points for a flow picker).
    /// Empty for single-flow docs — fall back to
    /// [`Doc::flow_start`](fanta_doc::Doc::flow_start).
    pub fn flows(&self) -> &[fanta_doc::Flow] {
        &self.doc.flows
    }

    /// Restart the presentation at a specific flow's start frame.
    pub fn start_flow(&mut self, flow: usize) -> bool {
        let Some(start) = self.doc.flows.get(flow).map(|f| f.start) else {
            return false;
        };
        if !self.doc.scene.contains(start) {
            return false;
        }
        self.back_stack.clear();
        self.enter_frame(start);
        true
    }

    /// Immutable document snapshot that this presentation is playing.
    pub fn document(&self) -> &Doc {
        &self.doc
    }

    /// Restart playback at `start` while retaining the renderer and its asset /
    /// instance caches. All ephemeral prototype state is reset, matching a new
    /// session without cloning the document again.
    pub fn restart_at(&mut self, start: NodeId) -> bool {
        if !self.doc.scene.contains(start) {
            return false;
        }
        self.current = start;
        self.viewport = frame_viewport(&self.doc.scene, start, self.screen_size);
        self.back_stack.clear();
        self.overlays.clear();
        self.variables = self.doc.variables.clone();
        self.active_modes = self.doc.active_modes.clone();
        self.mode_generation = self.mode_generation.wrapping_add(1);
        self.clock = 0.0;
        self.active_transition = None;
        self.smart_animate = None;
        self.active_overlay_transition = None;
        self.active_overlay_exit = None;
        self.active_variant_transition = None;
        self.active_prototype_animation = None;
        self.timers.clear();
        self.media_time = 0.0;
        self.media_playing = false;
        self.node_media_times.clear();
        self.scroll_offsets.clear();
        self.variant_overrides.clear();
        self.instance_variant_overrides.clear();
        self.hovered_reaction = None;
        self.pressed_reaction = None;
        self.suppress_next_click = false;
        self.hover_chain.clear();
        self.active_scroll_animation = None;
        self.pending_url = None;
        self.schedule_timers(start);
        true
    }

    /// Resize the logical presentation surface without discarding navigation,
    /// timer, variable, media, or component-variant state.
    pub fn resize(&mut self, screen_size: DVec2) -> Result<(), PresentError> {
        self.resize_with_scale(screen_size, self.renderer.display_scale)
    }

    /// Resize the logical surface and set its logical-to-physical display
    /// scale, preserving the active prototype state.
    pub fn resize_with_scale(
        &mut self,
        screen_size: DVec2,
        display_scale: f64,
    ) -> Result<(), PresentError> {
        let screen_size = valid_screen_size(screen_size);
        let display_scale = valid_display_scale(display_scale);
        let width = (screen_size.x * display_scale).round().max(1.0) as u32;
        let height = (screen_size.y * display_scale).round().max(1.0) as u32;
        if self.screen_size == screen_size
            && self.renderer.width() == width
            && self.renderer.height() == height
            && self.renderer.display_scale == display_scale
        {
            return Ok(());
        }
        let old_screen_size = self.screen_size;
        let logical_size_changed = old_screen_size != screen_size;
        self.renderer.resize(width, height)?;
        self.renderer.display_scale = display_scale;
        if !logical_size_changed {
            return Ok(());
        }
        self.viewport = rebase_viewport(
            &self.doc.scene,
            self.current,
            self.viewport,
            old_screen_size,
            screen_size,
        );
        for entry in &mut self.back_stack {
            entry.viewport = rebase_viewport(
                &self.doc.scene,
                entry.frame,
                entry.viewport,
                old_screen_size,
                screen_size,
            );
        }
        if let Some(active) = &mut self.active_transition {
            active.from_viewport = rebase_viewport(
                &self.doc.scene,
                active.from_frame,
                active.from_viewport,
                old_screen_size,
                screen_size,
            );
            active.to_viewport = rebase_viewport(
                &self.doc.scene,
                active.to_frame,
                active.to_viewport,
                old_screen_size,
                screen_size,
            );
        }
        self.screen_size = screen_size;

        let mut base_zoom = self.viewport.zoom;
        for overlay in &mut self.overlays {
            let Some(bounds) = self.doc.scene.world_bounds(overlay.root) else {
                continue;
            };
            let (viewport, screen_rect) =
                place_overlay(base_zoom, bounds, overlay.settings.position, screen_size);
            overlay.viewport = viewport;
            overlay.screen_rect = screen_rect;
            base_zoom = viewport.zoom;
        }
        if let Some(exit) = &mut self.active_overlay_exit
            && let Some(bounds) = self.doc.scene.world_bounds(exit.entry.root)
        {
            let (viewport, screen_rect) =
                place_overlay(base_zoom, bounds, exit.entry.settings.position, screen_size);
            exit.entry.viewport = viewport;
            exit.entry.screen_rect = screen_rect;
        }
        Ok(())
    }

    /// Physical pixel dimensions of [`present_rgba`](Self::present_rgba).
    pub fn pixel_size(&self) -> (u32, u32) {
        (self.renderer.width(), self.renderer.height())
    }

    /// Take the URL requested by the latest `OpenLink` action.
    pub fn take_open_url(&mut self) -> Option<String> {
        self.pending_url.take()
    }

    /// Handle a pointer event at `point` (logical screen px).
    pub fn handle_pointer(&mut self, point: DVec2, event: PointerEvent) -> PresentResponse {
        if event == PointerEvent::Leave {
            self.suppress_next_click = false;
            // Leaving the surface exits every hovered node at once: fire their
            // MouseLeave reactions before reverting the transient hover/press
            // effects, mirroring the leave-then-restore order of a Move that
            // exits a node's bounds.
            let left = self.fire_mouse_leave(None);
            return merge_responses(
                left,
                merge_responses(
                    self.restore_hovered_reaction(),
                    self.restore_pressed_reaction(),
                ),
            );
        }
        let hit = self.hit_screen(point);
        // A click that misses the topmost overlay either dismisses it (if it
        // opted into click-outside) or is swallowed — it never falls through to
        // the frame beneath.
        if event == PointerEvent::Click && hit.is_none() {
            if let Some(overlay) = self.overlays.last() {
                if overlay.settings.close_on_click_outside && !overlay.screen_rect.contains(point) {
                    if let Some(closed) = self.overlays.pop() {
                        self.cancel_timers(closed.root);
                        if self
                            .active_overlay_transition
                            .is_some_and(|active| active.root == closed.root)
                        {
                            self.active_overlay_transition = None;
                        }
                        self.active_overlay_exit = None;
                        self.active_prototype_animation = None;
                    }
                    let restored = merge_responses(
                        self.restore_hovered_reaction(),
                        self.restore_pressed_reaction(),
                    );
                    return PresentResponse {
                        needs_redraw: true,
                        navigated: true,
                        media_updated: false,
                        ..restored
                    };
                }
                return PresentResponse::default();
            }
        }
        match event {
            PointerEvent::Move => self.update_hover(hit),
            PointerEvent::Down => {
                let reaction = self.matching_reaction(hit, TriggerKind::WhilePressing);
                let has_reaction = reaction.is_some();
                let consumed = reaction.as_ref().is_some_and(|(_, _, actions, _, _)| {
                    actions.iter().any(down_action_suppresses_click)
                });
                let mut restored = self.restore_pressed_reaction();
                if has_reaction {
                    restored = merge_responses(restored, self.restore_hovered_reaction());
                }
                self.pressed_reaction = reaction
                    .as_ref()
                    .map(|reaction| self.active_pointer_reaction(reaction));
                self.suppress_next_click = consumed;
                let mut response = merge_responses(restored, self.execute_matching(reaction));
                response.suppress_click = consumed;
                response
            }
            PointerEvent::Up => {
                let restored = self.restore_pressed_reaction();
                if self.suppress_next_click {
                    restored
                } else {
                    merge_responses(restored, self.update_hover(hit))
                }
            }
            PointerEvent::Click => {
                if std::mem::take(&mut self.suppress_next_click) {
                    PresentResponse::default()
                } else {
                    let restored = self.restore_hovered_reaction();
                    merge_responses(restored, self.dispatch(hit, TriggerKind::Click))
                }
            }
            PointerEvent::DragStart => {
                self.suppress_next_click = false;
                let restored = self.restore_hovered_reaction();
                merge_responses(restored, self.dispatch(hit, TriggerKind::Drag))
            }
            PointerEvent::Leave => PresentResponse::default(),
        }
    }

    /// Handle a key press. `key` is the opaque Figma key name matched against
    /// `Trigger::Key { keys }`.
    pub fn handle_key(&mut self, key: &str, event: KeyEvent) -> PresentResponse {
        if event != KeyEvent::Down {
            return PresentResponse::default();
        }
        // A key trigger isn't tied to a hit point: any node on the active layer
        // carrying a matching Key reaction fires. Scan the layer's subtree.
        let layer_root = self.layer_root();
        let mut fired = None;
        for id in self.doc.scene.descendants_of(layer_root) {
            if let Some(node) = self.doc.scene.get(id) {
                if let Some(rx) = node
                    .reactions
                    .iter()
                    .find(|r| matches!(&r.trigger, Trigger::Key { keys } if keys.iter().any(|candidate| candidate.eq_ignore_ascii_case(key))))
                {
                    fired = Some((
                        id,
                        rx.actions().cloned().collect::<Vec<_>>(),
                        rx.transition.clone(),
                        rx.animation.clone(),
                    ));
                    break;
                }
            }
        }
        match fired {
            Some((source, actions, transition, animation)) => {
                self.execute_reaction(source, &actions, transition.as_ref(), animation.as_ref())
            }
            None => PresentResponse::default(),
        }
    }

    /// Advance time by `dt` seconds. Progresses frame/overlay/variant
    /// transitions and the active reaction-bound property clip while firing
    /// `AfterDelay` reactions at their exact chronological boundaries.
    pub fn tick(&mut self, dt: f64) -> PresentResponse {
        let mut remaining = dt.max(0.0);
        let mut response = PresentResponse::default();
        let mut fired_events = 0usize;

        loop {
            let next_due = self
                .timers
                .iter()
                .map(|timer| timer.remaining.max(0.0))
                .min_by(f64::total_cmp);
            let Some(next_due) = next_due else {
                response = merge_responses(response, self.advance_continuous(remaining));
                break;
            };

            if next_due > remaining + TIME_EPSILON_SECONDS {
                response = merge_responses(response, self.advance_continuous(remaining));
                for timer in &mut self.timers {
                    timer.remaining -= remaining;
                }
                break;
            }

            let step = next_due.min(remaining);
            response = merge_responses(response, self.advance_continuous(step));
            for timer in &mut self.timers {
                timer.remaining -= step;
            }
            remaining = (remaining - step).max(0.0);

            let mut due = Vec::new();
            self.timers.retain(|timer| {
                if timer.remaining <= TIME_EPSILON_SECONDS {
                    due.push(timer.clone());
                    false
                } else {
                    true
                }
            });
            due.sort_by_key(|timer| (timer.root, timer.source, timer.reaction));

            if due.is_empty() {
                if remaining <= 0.0 {
                    break;
                }
                continue;
            }

            for timer in due {
                if fired_events >= MAX_TIMER_EVENTS_PER_TICK {
                    self.timers.push(timer);
                    continue;
                }
                fired_events += 1;
                if timer.root != self.current
                    && !self
                        .overlays
                        .iter()
                        .any(|overlay| overlay.root == timer.root)
                {
                    continue;
                }
                let fired = self.execute_reaction(
                    timer.source,
                    &timer.actions,
                    timer.transition.as_ref(),
                    timer.animation.as_ref(),
                );
                response = merge_responses(response, fired);
            }

            if fired_events >= MAX_TIMER_EVENTS_PER_TICK {
                response = merge_responses(response, self.advance_continuous(remaining));
                for timer in &mut self.timers {
                    timer.remaining -= remaining;
                }
                break;
            }
        }

        response.animating = self.is_animating();
        response
    }

    fn advance_continuous(&mut self, dt: f64) -> PresentResponse {
        self.clock += dt;
        let mut media_changed = false;
        if self.media_playing {
            self.media_time += dt;
            for t in self.node_media_times.values_mut() {
                *t += dt;
            }
            media_changed = true;
        }

        let mut response = PresentResponse {
            media_updated: media_changed,
            ..PresentResponse::default()
        };

        let navigation_response = match &mut self.active_transition {
            Some(active) => {
                active.elapsed += dt;
                if active.done() {
                    self.active_transition = None;
                    self.smart_animate = None;
                    PresentResponse {
                        needs_redraw: true,
                        ..PresentResponse::default()
                    }
                } else {
                    PresentResponse {
                        needs_redraw: true,
                        animating: true,
                        ..PresentResponse::default()
                    }
                }
            }
            None => PresentResponse::default(),
        };
        response = merge_responses(response, navigation_response);

        let overlay_response = match &mut self.active_overlay_transition {
            Some(active) => {
                active.clock.elapsed += dt;
                if active.clock.done() {
                    self.active_overlay_transition = None;
                    PresentResponse {
                        needs_redraw: true,
                        ..PresentResponse::default()
                    }
                } else {
                    PresentResponse {
                        needs_redraw: true,
                        animating: true,
                        ..PresentResponse::default()
                    }
                }
            }
            None => PresentResponse::default(),
        };
        response = merge_responses(response, overlay_response);

        let overlay_exit_response = match &mut self.active_overlay_exit {
            Some(active) => {
                active.clock.elapsed += dt;
                if active.clock.done() {
                    self.active_overlay_exit = None;
                    PresentResponse {
                        needs_redraw: true,
                        ..PresentResponse::default()
                    }
                } else {
                    PresentResponse {
                        needs_redraw: true,
                        animating: true,
                        ..PresentResponse::default()
                    }
                }
            }
            None => PresentResponse::default(),
        };
        response = merge_responses(response, overlay_exit_response);

        let variant_response = match &mut self.active_variant_transition {
            Some(active) => {
                active.clock.elapsed += dt;
                if active.clock.done() {
                    self.active_variant_transition = None;
                    PresentResponse {
                        needs_redraw: true,
                        ..PresentResponse::default()
                    }
                } else {
                    PresentResponse {
                        needs_redraw: true,
                        animating: true,
                        ..PresentResponse::default()
                    }
                }
            }
            None => PresentResponse::default(),
        };
        response = merge_responses(response, variant_response);

        let animation_response = match &mut self.active_prototype_animation {
            Some(active) => {
                let before = active.playhead_ms(&self.doc);
                active.elapsed += dt;
                let after = active.playhead_ms(&self.doc);
                PresentResponse {
                    needs_redraw: before != after,
                    animating: active.is_running(&self.doc),
                    ..PresentResponse::default()
                }
            }
            None => PresentResponse::default(),
        };
        response = merge_responses(response, animation_response);

        // ScrollAnimate: interpolate the container's offset through the
        // CLAMPED setter each step — the animation target was clamped when the
        // action fired, but content bounds can shift mid-flight (variables,
        // variant swaps), so every sample re-clamps rather than trusting the
        // endpoints.
        let scroll_response = match &mut self.active_scroll_animation {
            Some(active) => {
                active.elapsed += dt;
                let animation = *active;
                let finished = animation.done();
                if finished {
                    self.active_scroll_animation = None;
                }
                let eased = ease(animation.easing, animation.progress());
                let offset = [
                    animation.from[0] + (animation.to[0] - animation.from[0]) * eased,
                    animation.from[1] + (animation.to[1] - animation.from[1]) * eased,
                ];
                self.set_scroll_offset(animation.scrollable, offset);
                PresentResponse {
                    needs_redraw: true,
                    animating: !finished,
                    ..PresentResponse::default()
                }
            }
            None => PresentResponse::default(),
        };
        response = merge_responses(response, scroll_response);

        response
    }

    fn is_animating(&self) -> bool {
        self.active_transition.is_some()
            || self.active_overlay_transition.is_some()
            || self.active_overlay_exit.is_some()
            || self.active_variant_transition.is_some()
            || self.active_scroll_animation.is_some()
            || self
                .active_prototype_animation
                .is_some_and(|active| active.is_running(&self.doc))
    }

    /// Render the current view and read back straight-alpha RGBA8 (headless /
    /// PNG path). Composites, in order: the base layer (an in-flight transition's
    /// two frames, else the current frame), then — for each open overlay — a
    /// backdrop scrim (if it dims) and the overlay frame on top.
    pub fn present_rgba(&mut self) -> Vec<u8> {
        if let Some(mut active) = self.active_variant_transition.take() {
            let eased = ease(active.clock.easing, active.clock.progress());
            let width = self.renderer.width() as usize;
            let height = self.renderer.height() as usize;
            let hidden: Vec<NodeId> = active.layers.iter().map(|layer| layer.instance).collect();
            let mut pixels = self.render_surface_hiding(&hidden);
            for layer in &mut active.layers {
                let target_instance = layer.target_instance.clone();
                let target_parent_world = layer.target_parent_world;
                let target_master_opacity = layer.target_master_opacity;
                let animated = if matches!(active.clock.style, TransitionStyle::SmartAnimate)
                    && layer.smart_animate.is_some()
                {
                    if let Some(smart) = &mut layer.smart_animate {
                        smart.apply(eased);
                        let mut scene = smart.scene.clone();
                        self.apply_destination_motion_to_materialized_root(
                            &mut scene,
                            smart.root,
                            &target_instance,
                            target_parent_world,
                            target_master_opacity,
                        );
                        self.render_scene_root_rgba(&scene, smart.root, layer.viewport, false)
                    } else {
                        Vec::new()
                    }
                } else {
                    let outgoing = self.render_scene_root_rgba(
                        &layer.from_scene,
                        layer.from_root,
                        layer.viewport,
                        false,
                    );
                    let mut incoming_scene = layer.to_scene.clone();
                    self.apply_destination_motion_to_materialized_root(
                        &mut incoming_scene,
                        layer.to_root,
                        &target_instance,
                        target_parent_world,
                        target_master_opacity,
                    );
                    let incoming = self.render_scene_root_rgba(
                        &incoming_scene,
                        layer.to_root,
                        layer.viewport,
                        false,
                    );
                    composite(
                        active.clock.style,
                        eased,
                        width,
                        height,
                        &outgoing,
                        &incoming,
                    )
                };
                composite_over(&mut pixels, &animated);
            }
            self.active_variant_transition = Some(active);
            return pixels;
        }

        let mut buffer = self.base_rgba();
        // Clone the (small) overlay stack so the render borrow of `self` is free.
        let overlays: Vec<OverlayEntry> = self.overlays.clone();
        for (index, entry) in overlays.iter().enumerate() {
            let over = self.render_frame_rgba(entry.root, entry.viewport);
            let entering = self
                .active_overlay_transition
                .filter(|active| active.root == entry.root && index + 1 == overlays.len());
            if let Some(active) = entering {
                let eased = ease(active.clock.easing, active.clock.progress());
                if entry.settings.background_dim {
                    apply_scrim(&mut buffer, SCRIM_DIM * eased);
                }
                buffer = composite_overlay(
                    active.clock.style,
                    eased,
                    self.renderer.width() as usize,
                    self.renderer.height() as usize,
                    &buffer,
                    &over,
                );
            } else {
                if entry.settings.background_dim {
                    apply_scrim(&mut buffer, SCRIM_DIM);
                }
                composite_over(&mut buffer, &over);
            }
        }
        if let Some(exit) = self.active_overlay_exit.clone() {
            let visible =
                exit.start_visible * (1.0 - ease(exit.clock.easing, exit.clock.progress()));
            if exit.entry.settings.background_dim {
                apply_scrim(&mut buffer, SCRIM_DIM * visible);
            }
            let overlay =
                self.render_frame_rgba_with_motion(exit.entry.root, exit.entry.viewport, false);
            buffer = composite_overlay(
                exit.clock.style,
                visible,
                self.renderer.width() as usize,
                self.renderer.height() as usize,
                &buffer,
                &overlay,
            );
        }
        buffer
    }

    /// The base layer beneath any overlays: the transition composite while one
    /// animates, otherwise the current frame rendered directly.
    fn base_rgba(&mut self) -> Vec<u8> {
        let Some(active) = self.active_transition else {
            let _ = self.render_current();
            return self.renderer.copy_rgba();
        };
        let eased = ease(active.easing, active.progress());

        // Smart animate: tween the scratch scene and render it directly (no
        // two-raster composite). Fields are borrowed disjointly so the scratch
        // scene, the render inputs, and the renderer can be touched at once.
        if matches!(active.style, TransitionStyle::SmartAnimate) && self.smart_animate.is_some() {
            let smart_scene = self.smart_animate.as_mut().map(|smart| {
                smart.apply(eased);
                (smart.scene.clone(), smart.root)
            });
            if let Some((mut scene, root)) = smart_scene {
                Self::apply_scroll_offsets_to_scene(&self.scroll_offsets, &mut scene);
                Self::apply_variant_overrides_to_scene(
                    &self.variant_overrides,
                    &self.instance_variant_overrides,
                    &self.doc.components,
                    &mut scene,
                    root,
                );
                let motion = self.prototype_motion_evaluation();
                let inputs = RenderInputs {
                    components: &self.doc.components,
                    variables: &self.variables,
                    active_modes: &self.active_modes,
                    mode_generation: self.mode_generation,
                    motion: motion.as_ref(),
                    playback: None,
                    dark_ui: false,
                };
                self.renderer
                    .render_page_with(&scene, &active.to_viewport, Some(root), &inputs);
                return self.renderer.copy_rgba();
            }
        }

        // Every other style composites the two endpoint frames.
        let outgoing =
            self.render_frame_rgba_with_motion(active.from_frame, active.from_viewport, false);
        let incoming = self.render_frame_rgba(active.to_frame, active.to_viewport);
        let width = self.renderer.width() as usize;
        let height = self.renderer.height() as usize;
        composite(active.style, eased, width, height, &outgoing, &incoming)
    }

    /// Render one frame through `viewport` onto the owned surface and read back
    /// straight-alpha RGBA8. Used to produce the two layers a transition blends.
    fn render_frame_rgba(&mut self, frame: NodeId, viewport: Viewport) -> Vec<u8> {
        self.render_frame_rgba_with_motion(frame, viewport, true)
    }

    fn render_frame_rgba_with_motion(
        &mut self,
        frame: NodeId,
        viewport: Viewport,
        include_motion: bool,
    ) -> Vec<u8> {
        // Built inline for the same borrow-splitting reason as `render_current`.
        let motion = include_motion
            .then(|| self.prototype_motion_evaluation())
            .flatten();
        let inputs = RenderInputs {
            components: &self.doc.components,
            variables: &self.variables,
            active_modes: &self.active_modes,
            mode_generation: self.mode_generation,
            motion: motion.as_ref(),
            playback: None,
            dark_ui: false,
        };
        if let Some(patched) = self.build_runtime_scene(frame) {
            self.renderer
                .render_page_with(&patched, &viewport, Some(frame), &inputs);
        } else {
            self.renderer
                .render_page_with(&self.doc.scene, &viewport, Some(frame), &inputs);
        }
        self.renderer.copy_rgba()
    }

    fn render_surface_hiding(&mut self, hidden: &[NodeId]) -> Vec<u8> {
        let mut buffer = self.render_frame_rgba_hiding(self.current, self.viewport, hidden);
        let overlays = self.overlays.clone();
        for entry in overlays {
            if entry.settings.background_dim {
                apply_scrim(&mut buffer, SCRIM_DIM);
            }
            let overlay = self.render_frame_rgba_hiding(entry.root, entry.viewport, hidden);
            composite_over(&mut buffer, &overlay);
        }
        buffer
    }

    fn render_frame_rgba_hiding(
        &mut self,
        frame: NodeId,
        viewport: Viewport,
        hidden: &[NodeId],
    ) -> Vec<u8> {
        let mut scene = self
            .build_runtime_scene(frame)
            .unwrap_or_else(|| self.doc.scene.clone());
        for hidden in hidden {
            if descends_from(&scene, *hidden, frame)
                && let Some(node) = scene.get_mut(*hidden)
            {
                node.flags.insert(NodeFlags::HIDDEN);
            }
        }
        let motion = self.prototype_motion_evaluation_excluding(hidden);
        let inputs = RenderInputs {
            components: &self.doc.components,
            variables: &self.variables,
            active_modes: &self.active_modes,
            mode_generation: self.mode_generation,
            motion: motion.as_ref(),
            playback: None,
            dark_ui: false,
        };
        self.renderer
            .render_page_with(&scene, &viewport, Some(frame), &inputs);
        self.renderer.copy_rgba()
    }

    fn render_scene_root_rgba(
        &mut self,
        scene: &Scene,
        root: NodeId,
        viewport: Viewport,
        include_motion: bool,
    ) -> Vec<u8> {
        let motion = include_motion
            .then(|| self.prototype_motion_evaluation())
            .flatten();
        let inputs = RenderInputs {
            components: &self.doc.components,
            variables: &self.variables,
            active_modes: &self.active_modes,
            mode_generation: self.mode_generation,
            motion: motion.as_ref(),
            playback: None,
            dark_ui: false,
        };
        self.renderer
            .render_page_with(scene, &viewport, Some(root), &inputs);
        self.renderer.copy_rgba()
    }

    /// Render the current view onto the owned surface, returning metrics.
    pub fn render_current(&mut self) -> RenderMetrics {
        // Built inline (not via a `&self` helper) so the borrow checker sees the
        // input borrows (`doc`/`variables`/`active_modes`) are disjoint from the
        // mutable `renderer` borrow.
        let playback = self.build_media_playback();
        let motion = self.prototype_motion_evaluation();
        let inputs = RenderInputs {
            components: &self.doc.components,
            variables: &self.variables,
            active_modes: &self.active_modes,
            mode_generation: self.mode_generation,
            motion: motion.as_ref(),
            playback: Some(&playback),
            dark_ui: false,
        };
        if let Some(patched) = self.build_runtime_scene(self.current) {
            self.renderer
                .render_page_with(&patched, &self.viewport, Some(self.current), &inputs)
        } else {
            self.renderer.render_page_with(
                &self.doc.scene,
                &self.viewport,
                Some(self.current),
                &inputs,
            )
        }
    }

    /// Set or seek the global media playback time (seconds).
    pub fn set_media_time(&mut self, t: f64) {
        self.media_time = t.max(0.0);
    }

    /// Start/resume media playback (advances on tick, feeds VideoNodes).
    pub fn play_media(&mut self) {
        self.media_playing = true;
    }

    /// Pause media playback.
    pub fn pause_media(&mut self) {
        self.media_playing = false;
    }

    /// Current media time (for testing / hosts).
    pub fn media_time(&self) -> f64 {
        self.media_time
    }

    /// Set/seek the media time for a specific Video/Audio node (per-clip control).
    /// Subsequent global tick/play will advance it from this value (unless set again).
    /// Falls back to global media_time when not explicitly set for the node.
    pub fn set_node_media_time(&mut self, id: NodeId, t: f64) {
        self.node_media_times.insert(id, t.max(0.0));
    }

    /// Clear any per-node media time override for `id`; future queries will use the global.
    pub fn clear_node_media_time(&mut self, id: NodeId) {
        self.node_media_times.remove(&id);
    }

    /// Set or update a component variant override (from prototype UpdateVariant).
    /// The variant name is the authored value (e.g. "Hover", "Large").
    pub fn set_variant_override(&mut self, component: ComponentId, variant: String) {
        self.variant_overrides.insert(component, variant);
    }

    /// Current variant overrides active in this prototype session.
    pub fn variant_overrides(&self) -> &HashMap<ComponentId, String> {
        &self.variant_overrides
    }

    /// Clear a specific variant override (revert to design default for playback).
    pub fn clear_variant_override(&mut self, component: ComponentId) {
        self.variant_overrides.remove(&component);
    }

    fn prototype_motion_evaluation(&self) -> Option<MotionEvaluation> {
        let active = self.active_prototype_animation?;
        let playhead_ms = active.playhead_ms(&self.doc)?;
        self.doc.motion.evaluate(active.clip, playhead_ms)
    }

    fn prototype_motion_evaluation_excluding(
        &self,
        excluded: &[NodeId],
    ) -> Option<MotionEvaluation> {
        let mut evaluation = self.prototype_motion_evaluation()?;
        evaluation
            .overrides
            .retain(|target, _| !excluded.contains(&target.node));
        Some(evaluation)
    }

    fn apply_destination_motion_to_materialized_root(
        &self,
        scene: &mut Scene,
        root: NodeId,
        target_instance: &fanta_doc::CanvasNode,
        target_parent_world: Transform2D,
        target_master_opacity: f32,
    ) {
        let Some(evaluation) = self.prototype_motion_evaluation() else {
            return;
        };
        let evaluated = evaluation.apply_to_node(target_instance);
        let Some(root) = scene.get_mut(root) else {
            return;
        };
        root.transform = evaluated.transform.then(&target_parent_world);
        root.opacity =
            fanta_doc::UnitInterval::new(target_master_opacity * evaluated.opacity.get());
    }

    /// Apply prototype variant overrides to any InstanceNodes in the given
    /// scene subtree. Takes the pieces explicitly so it can be used from
    /// contexts holding a &mut on a scratch scene (e.g. inside SmartAnimate
    /// render) without &self borrow conflicts.
    #[cfg_attr(test, allow(dead_code))] // exercised via public render paths and direct tests
    pub(crate) fn apply_variant_overrides_to_scene(
        overrides: &HashMap<ComponentId, String>,
        instance_overrides: &HashMap<NodeId, (ComponentId, String)>,
        components: &fanta_doc::ComponentLibrary,
        scene: &mut Scene,
        root: NodeId,
    ) {
        if overrides.is_empty() && instance_overrides.is_empty() {
            return;
        }
        let ids: Vec<NodeId> = scene.descendants_of(root).collect();
        for id in ids {
            if let Some(node) = scene.get_mut(id) {
                if let NodeData::Instance(inst) = &mut node.data {
                    apply_variant_override_to_instance(
                        overrides,
                        instance_overrides.get(&id),
                        components,
                        inst,
                    );
                }
            }
        }
    }

    fn build_runtime_scene(&self, layer_root: NodeId) -> Option<Scene> {
        self.build_runtime_scene_with_variant_state(
            layer_root,
            &self.variant_overrides,
            &self.instance_variant_overrides,
        )
    }

    fn build_runtime_scene_with_variant_state(
        &self,
        layer_root: NodeId,
        global: &HashMap<ComponentId, String>,
        instances: &HashMap<NodeId, (ComponentId, String)>,
    ) -> Option<Scene> {
        if self.scroll_offsets.is_empty() && global.is_empty() && instances.is_empty() {
            return None;
        }
        let mut scene = self.doc.scene.clone();
        Self::apply_scroll_offsets_to_scene(&self.scroll_offsets, &mut scene);
        Self::apply_variant_overrides_to_scene(
            global,
            instances,
            &self.doc.components,
            &mut scene,
            layer_root,
        );
        Some(scene)
    }

    fn apply_scroll_offsets_to_scene(offsets: &HashMap<NodeId, [f64; 2]>, scene: &mut Scene) {
        for (scrollable, offset) in offsets {
            if offset[0].abs() <= f64::EPSILON && offset[1].abs() <= f64::EPSILON {
                continue;
            }
            let children = scene.children_of(Some(*scrollable)).to_vec();
            for child in children {
                let Some(node) = scene.get_mut(child) else {
                    continue;
                };
                let effective = match node.scroll_behavior {
                    // Moves with content — the full offset.
                    ScrollBehavior::Scrolls => *offset,
                    // Pinned: a fixed header/tab-bar ignores the scroll
                    // entirely (Figma renders it in place above the moving
                    // content).
                    ScrollBehavior::Fixed => continue,
                    // Sticky: scrolls until its leading edge reaches the
                    // frame's, then pins — consume only the part of the
                    // offset that hasn't pushed the node past origin.
                    // (Negative overscroll behaves like plain content.)
                    ScrollBehavior::Sticky => {
                        let components = node.transform.to_components();
                        let origin = [components[4], components[5]];
                        [
                            sticky_component(offset[0], origin[0]),
                            sticky_component(offset[1], origin[1]),
                        ]
                    }
                };
                if effective[0].abs() <= f64::EPSILON && effective[1].abs() <= f64::EPSILON {
                    continue;
                }
                let translation = Transform2D::translation(-effective[0], -effective[1]);
                node.transform = node.transform.then(&translation);
            }
        }
    }

    /// Current media time for a specific node (per-clip), or global if no override set.
    pub fn node_media_time(&self, id: NodeId) -> f64 {
        *self.node_media_times.get(&id).unwrap_or(&self.media_time)
    }

    /// Set a scroll offset for a scrollable container (used internally by ScrollTo
    /// and available to hosts for custom prototype scroll control). The offset is
    /// clamped to the container's authored scroll axes and content overflow —
    /// content can never scroll past its far edge or before its start.
    pub fn set_scroll_offset(&mut self, scrollable: NodeId, offset: [f64; 2]) {
        let clamped = self.clamped_scroll_offset(scrollable, offset);
        self.scroll_offsets.insert(scrollable, clamped);
    }

    /// Screen-space variant of [`scroll_by`](Self::scroll_by) — the shape a
    /// host's wheel handler already has (the same coordinates
    /// [`handle_pointer`](Self::handle_pointer) takes). Converts through the
    /// current frame's viewport and delegates.
    pub fn scroll_by_screen(&mut self, screen_point: DVec2, delta: [f64; 2]) -> Option<NodeId> {
        let world = screen_to_world(screen_point, &self.viewport, self.screen_size);
        self.scroll_by([world.x, world.y], delta)
    }

    /// Scroll the topmost scrollable container under `world_point` by `delta`
    /// (the wheel/trackpad entry point a host forwards). Walks up from the hit
    /// node to the nearest ancestor whose authored axes accept any of the
    /// delta. Returns the container scrolled, or `None` when nothing under the
    /// point scrolls — the host can then fall back to panning its own surface.
    pub fn scroll_by(&mut self, world_point: [f64; 2], delta: [f64; 2]) -> Option<NodeId> {
        let hit = self
            .doc
            .scene
            .hit_test(DVec2::new(world_point[0], world_point[1]))?;
        let mut cursor = Some(hit);
        while let Some(id) = cursor {
            let node = self.doc.scene.get(id)?;
            if let NodeData::Group(group) = &node.data {
                let direction = group.effective_scroll_direction();
                if (direction.allows_x() && delta[0] != 0.0)
                    || (direction.allows_y() && delta[1] != 0.0)
                {
                    let current = self.scroll_offset(id);
                    self.set_scroll_offset(id, [current[0] + delta[0], current[1] + delta[1]]);
                    return Some(id);
                }
            }
            cursor = node.parent;
        }
        None
    }

    /// Clamp a proposed offset to `scrollable`'s authored axes and content
    /// overflow. Axes the container doesn't scroll zero out; allowed axes clamp
    /// to `[0, content_extent - viewport_extent]` (the Figma model: content
    /// starts flush and can scroll until its far edge meets the viewport's).
    fn clamped_scroll_offset(&self, scrollable: NodeId, offset: [f64; 2]) -> [f64; 2] {
        let Some(node) = self.doc.scene.get(scrollable) else {
            return [0.0, 0.0];
        };
        let NodeData::Group(group) = &node.data else {
            return [0.0, 0.0];
        };
        let direction = group.effective_scroll_direction();
        if direction == ScrollDirection::None {
            return [0.0, 0.0];
        }

        // Viewport box: the frame's own clip box (falling back to its declared
        // size). Children coordinates are relative to the frame origin.
        let viewport = group.clip_size.or(group.local_size);
        // Content extent: union of the children's boxes in frame-local space.
        // World AABBs mapped through the frame's inverse world transform —
        // exact for the axis-aligned common case, conservative under rotation.
        let Some(world) = self.doc.scene.world_transform(scrollable) else {
            return [0.0, 0.0];
        };
        let inverse = world.inverse();
        let mut content_max = [f64::MIN, f64::MIN];
        let mut any = false;
        for child in self.doc.scene.children_of(Some(scrollable)).to_vec() {
            let Some(bounds) = self.doc.scene.world_bounds(child) else {
                continue;
            };
            for corner in [
                [bounds.min_x, bounds.min_y],
                [bounds.max_x, bounds.min_y],
                [bounds.min_x, bounds.max_y],
                [bounds.max_x, bounds.max_y],
            ] {
                let local = inverse.transform_point(DVec2::new(corner[0], corner[1]));
                content_max[0] = content_max[0].max(local.x);
                content_max[1] = content_max[1].max(local.y);
                any = true;
            }
        }
        if !any {
            return [0.0, 0.0];
        }
        let max_scroll = match viewport {
            Some([w, h]) => [(content_max[0] - w).max(0.0), (content_max[1] - h).max(0.0)],
            // No declared box (plain scrollable group): allow the raw offset on
            // permitted axes — there is no viewport edge to clamp against.
            None => [f64::MAX, f64::MAX],
        };
        [
            if direction.allows_x() {
                offset[0].clamp(0.0, max_scroll[0])
            } else {
                0.0
            },
            if direction.allows_y() {
                offset[1].clamp(0.0, max_scroll[1])
            } else {
                0.0
            },
        ]
    }

    /// Get current scroll offset for a scrollable (or [0,0] if none).
    pub fn scroll_offset(&self, scrollable: NodeId) -> [f64; 2] {
        self.scroll_offsets
            .get(&scrollable)
            .copied()
            .unwrap_or([0.0, 0.0])
    }

    /// Clear scroll offset for a container (e.g. on frame enter or reset).
    pub fn clear_scroll_offset(&mut self, scrollable: NodeId) {
        self.scroll_offsets.remove(&scrollable);
    }

    /// Build per-node MediaPlayback for video/audio nodes based on current media_time.
    /// When not playing the map is empty (no progress updates).
    fn build_media_playback(
        &self,
    ) -> std::collections::HashMap<NodeId, fanta_render::MediaPlayback> {
        use fanta_doc::NodeData;
        let mut map = std::collections::HashMap::new();
        if !self.media_playing {
            return map;
        }
        for id in self.doc.scene.descendants_of(self.current) {
            if let Some(node) = self.doc.scene.get(id) {
                if let NodeData::Video(v) = &node.data {
                    let t = self.node_media_time(id);
                    let start = v.time_range_us[0] as f64 / 1_000_000.0;
                    let end = v.time_range_us[1] as f64 / 1_000_000.0;
                    let local_t = ((t * v.speed as f64) + start).clamp(start, end);
                    let progress = if end > start {
                        ((local_t - start) / (end - start)).clamp(0.0, 1.0) as f32
                    } else {
                        0.0
                    };
                    map.insert(
                        id,
                        fanta_render::MediaPlayback {
                            progress,
                            frame: None,
                            decoded_frame: None,
                        },
                    );
                } else if let NodeData::Audio(a) = &node.data {
                    let t = self.node_media_time(id);
                    let start = a.time_range_us[0] as f64 / 1_000_000.0;
                    let end = a.time_range_us[1] as f64 / 1_000_000.0;
                    let local_t = (t + start).clamp(start, end); // speed default 1.0 for audio
                    let progress = if end > start {
                        ((local_t - start) / (end - start)).clamp(0.0, 1.0) as f32
                    } else {
                        0.0
                    };
                    map.insert(
                        id,
                        fanta_render::MediaPlayback {
                            progress,
                            frame: None,
                            decoded_frame: None,
                        },
                    );
                }
            }
        }
        map
    }

    /// Current media playback map for all Video/Audio nodes under the active
    /// scene. Empty when paused/stopped. This gives hosts per-clip progress
    /// (0..1 within the node's time_range) driven by the (global or per-node
    /// override) media clock. Use set_node_media_time for independent per-clip seek.
    pub fn media_playback(&self) -> std::collections::HashMap<NodeId, fanta_render::MediaPlayback> {
        self.build_media_playback()
    }

    /// The root of the layer that currently receives input: the topmost overlay
    /// if any, else the current base frame.
    fn layer_root(&self) -> NodeId {
        self.overlays.last().map(|o| o.root).unwrap_or(self.current)
    }

    /// The viewport of the layer that currently receives input — the topmost
    /// overlay's placement viewport, else the base frame's.
    fn active_viewport(&self) -> Viewport {
        self.overlays
            .last()
            .map(|o| o.viewport)
            .unwrap_or(self.viewport)
    }

    fn hit_screen(&mut self, point: DVec2) -> Option<NodeId> {
        if let Some(active) = self.active_transition {
            return self.hit_navigation_transition(point, active);
        }
        if self.active_variant_transition.is_some() {
            return self.hit_variant_transition(point);
        }
        if let Some(active) = self.active_overlay_transition {
            let eased = ease(active.clock.easing, active.clock.progress());
            if matches!(
                active.clock.style,
                TransitionStyle::Dissolve | TransitionStyle::SmartAnimate
            ) && eased <= TIME_EPSILON_SECONDS
            {
                return None;
            }
            let offset = overlay_offset(
                active.clock.style,
                eased,
                self.screen_size.x,
                self.screen_size.y,
            );
            let mapped = point - DVec2::new(offset.0, offset.1);
            return self.hit_layer_at_screen(active.root, self.active_viewport(), mapped, true);
        }
        self.hit_layer_at_screen(self.layer_root(), self.active_viewport(), point, true)
    }

    fn hit_navigation_transition(
        &mut self,
        point: DVec2,
        active: ActiveTransition,
    ) -> Option<NodeId> {
        let eased = ease(active.easing, active.progress());
        if matches!(active.style, TransitionStyle::SmartAnimate)
            && let Some((mut scene, root)) = self.smart_animate.as_mut().map(|smart| {
                smart.apply(eased);
                (smart.scene.clone(), smart.root)
            })
        {
            Self::apply_scroll_offsets_to_scene(&self.scroll_offsets, &mut scene);
            Self::apply_variant_overrides_to_scene(
                &self.variant_overrides,
                &self.instance_variant_overrides,
                &self.doc.components,
                &mut scene,
                root,
            );
            Self::apply_motion_to_hit_scene(&mut scene, self.prototype_motion_evaluation());
            let world = screen_to_world(point, &active.to_viewport, self.screen_size);
            return scene.topmost_hit_where(world, |id| descends_from(&scene, id, root));
        }

        match active.style {
            TransitionStyle::SlideIn { direction }
            | TransitionStyle::Push { direction }
            | TransitionStyle::MoveIn { direction } => {
                let (outgoing, incoming) = directional_offsets(
                    active.style,
                    direction,
                    eased,
                    self.screen_size.x,
                    self.screen_size.y,
                );
                let incoming_point = point - DVec2::new(incoming.0, incoming.1);
                self.hit_layer_at_screen(active.to_frame, active.to_viewport, incoming_point, true)
                    .or_else(|| {
                        let outgoing_point = point - DVec2::new(outgoing.0, outgoing.1);
                        self.hit_layer_at_screen(
                            active.from_frame,
                            active.from_viewport,
                            outgoing_point,
                            false,
                        )
                    })
            }
            // Out styles mirror the probe order too: the OUTGOING frame is
            // the moving top layer, so it wins hits where it still covers the
            // point; the revealed incoming frame catches the rest.
            TransitionStyle::SlideOut { direction } | TransitionStyle::MoveOut { direction } => {
                let (outgoing, incoming) = directional_offsets(
                    active.style,
                    direction,
                    eased,
                    self.screen_size.x,
                    self.screen_size.y,
                );
                let outgoing_point = point - DVec2::new(outgoing.0, outgoing.1);
                self.hit_layer_at_screen(
                    active.from_frame,
                    active.from_viewport,
                    outgoing_point,
                    false,
                )
                .or_else(|| {
                    let incoming_point = point - DVec2::new(incoming.0, incoming.1);
                    self.hit_layer_at_screen(
                        active.to_frame,
                        active.to_viewport,
                        incoming_point,
                        true,
                    )
                })
            }
            TransitionStyle::Dissolve
            | TransitionStyle::SmartAnimate
            | TransitionStyle::ScrollAnimate => {
                let incoming = (eased > TIME_EPSILON_SECONDS).then(|| {
                    self.hit_layer_at_screen(active.to_frame, active.to_viewport, point, true)
                });
                incoming.flatten().or_else(|| {
                    (eased < 1.0 - TIME_EPSILON_SECONDS)
                        .then(|| {
                            self.hit_layer_at_screen(
                                active.from_frame,
                                active.from_viewport,
                                point,
                                false,
                            )
                        })
                        .flatten()
                })
            }
            TransitionStyle::Instant => {
                self.hit_layer_at_screen(active.to_frame, active.to_viewport, point, true)
            }
        }
    }

    fn hit_variant_transition(&self, point: DVec2) -> Option<NodeId> {
        let active = self.active_variant_transition.as_ref()?;
        let eased = ease(active.clock.easing, active.clock.progress());
        let hidden: Vec<NodeId> = active.layers.iter().map(|layer| layer.instance).collect();
        for layer in active.layers.iter().rev() {
            let hit = match active.clock.style {
                TransitionStyle::SlideIn { direction }
                | TransitionStyle::Push { direction }
                | TransitionStyle::MoveIn { direction } => {
                    let (outgoing, incoming) = directional_offsets(
                        active.clock.style,
                        direction,
                        eased,
                        self.screen_size.x,
                        self.screen_size.y,
                    );
                    let incoming_point = point - DVec2::new(incoming.0, incoming.1);
                    self.hit_layer_at_screen(layer.instance, layer.viewport, incoming_point, true)
                        .or_else(|| {
                            let outgoing_point = point - DVec2::new(outgoing.0, outgoing.1);
                            self.hit_layer_at_screen(
                                layer.instance,
                                layer.viewport,
                                outgoing_point,
                                false,
                            )
                        })
                }
                _ => self.hit_layer_at_screen(layer.instance, layer.viewport, point, true),
            };
            if hit.is_some() {
                return hit;
            }
        }
        self.hit_layer_at_screen_excluding(
            self.layer_root(),
            self.active_viewport(),
            point,
            true,
            &hidden,
        )
    }

    fn hit_layer_at_screen(
        &self,
        layer_root: NodeId,
        viewport: Viewport,
        point: DVec2,
        include_motion: bool,
    ) -> Option<NodeId> {
        self.hit_layer_at_screen_excluding(layer_root, viewport, point, include_motion, &[])
    }

    fn hit_layer_at_screen_excluding(
        &self,
        layer_root: NodeId,
        viewport: Viewport,
        point: DVec2,
        include_motion: bool,
        excluded: &[NodeId],
    ) -> Option<NodeId> {
        let world = screen_to_world(point, &viewport, self.screen_size);
        if let Some(scene) = self.build_hit_scene(layer_root, include_motion) {
            scene.topmost_hit_where(world, |id| {
                descends_from(&scene, id, layer_root)
                    && !excluded
                        .iter()
                        .any(|excluded| descends_from(&scene, id, *excluded))
            })
        } else {
            self.doc.scene.topmost_hit_where(world, |id| {
                descends_from(&self.doc.scene, id, layer_root)
                    && !excluded
                        .iter()
                        .any(|excluded| descends_from(&self.doc.scene, id, *excluded))
            })
        }
    }

    fn build_hit_scene(&self, layer_root: NodeId, include_motion: bool) -> Option<Scene> {
        let motion = include_motion
            .then(|| self.prototype_motion_evaluation())
            .flatten();
        let mut scene = match self.build_runtime_scene(layer_root) {
            Some(scene) => scene,
            None if motion.is_some() => self.doc.scene.clone(),
            None => return None,
        };
        Self::apply_motion_to_hit_scene(&mut scene, motion);
        Some(scene)
    }

    fn apply_motion_to_hit_scene(scene: &mut Scene, motion: Option<MotionEvaluation>) {
        if let Some(motion) = motion {
            let mut targets: Vec<NodeId> =
                motion.overrides.keys().map(|target| target.node).collect();
            targets.sort_unstable();
            targets.dedup();
            for target in targets {
                let Some(evaluated) = scene.get(target).map(|node| motion.apply_to_node(node))
                else {
                    continue;
                };
                if let Some(node) = scene.get_mut(target) {
                    *node = evaluated;
                }
            }
        }
    }

    /// Walk up from the hit node, firing the nearest matching reaction (Figma's
    /// trigger bubbling).
    fn dispatch(&mut self, hit: Option<NodeId>, want: TriggerKind) -> PresentResponse {
        let reaction = self.matching_reaction(hit, want);
        self.execute_matching(reaction)
    }

    fn matching_reaction(&self, hit: Option<NodeId>, want: TriggerKind) -> Option<MatchedReaction> {
        let mut node = hit;
        while let Some(id) = node {
            if let Some(cn) = self.doc.scene.get(id) {
                if let Some(rx) = cn
                    .reactions
                    .iter()
                    .find(|r| trigger_matches(&r.trigger, want))
                {
                    return Some((
                        id,
                        rx.id,
                        rx.actions().cloned().collect(),
                        rx.transition.clone(),
                        rx.animation.clone(),
                    ));
                }
            }
            node = self.doc.scene.get(id).and_then(|n| n.parent);
        }
        None
    }

    fn execute_matching(&mut self, reaction: Option<MatchedReaction>) -> PresentResponse {
        let Some((source, _, actions, transition, animation)) = reaction else {
            return PresentResponse::default();
        };
        self.execute_reaction(source, &actions, transition.as_ref(), animation.as_ref())
    }

    fn update_hover(&mut self, hit: Option<NodeId>) -> PresentResponse {
        // MouseLeave first: a pointer that moved from node A into node B has
        // left A before it is hovering B (Figma's edge ordering), and the
        // leave may navigate — in which case the enter side below re-resolves
        // against whatever hit remains valid.
        let left = self.fire_mouse_leave(hit);

        let reaction = self.matching_reaction(hit, TriggerKind::Hover);
        let next = reaction
            .as_ref()
            .map(|(source, reaction, _, _, _)| (*source, *reaction));
        let current = self
            .hovered_reaction
            .as_ref()
            .map(|active| (active.source, active.reaction));
        if current == next {
            return left;
        }
        let restored = self.restore_hovered_reaction();
        self.hovered_reaction = reaction
            .as_ref()
            .map(|reaction| self.active_pointer_reaction(reaction));
        merge_responses(
            left,
            merge_responses(restored, self.execute_matching(reaction)),
        )
    }

    /// Fire [`Trigger::MouseLeave`] reactions on every node the pointer just
    /// exited, then remember the new hover chain. A node "exits" when it was
    /// on the previous Move's ancestor chain (hit node up to its root) but is
    /// absent from the new one — the same containment edge that re-arms the
    /// entry-side hover machinery. Exits fire innermost-first; a leave action
    /// that changes the current frame stops the remaining exits (they belonged
    /// to the frame that was left — the multi-action rule applied across
    /// reactions).
    fn fire_mouse_leave(&mut self, hit: Option<NodeId>) -> PresentResponse {
        let new_chain = self.ancestor_chain(hit);
        if new_chain == self.hover_chain {
            return PresentResponse::default();
        }
        let old_chain = std::mem::replace(&mut self.hover_chain, new_chain);
        let mut fired: Vec<MatchedReaction> = Vec::new();
        for exited in &old_chain {
            if self.hover_chain.contains(exited) {
                continue;
            }
            let Some(node) = self.doc.scene.get(*exited) else {
                continue;
            };
            if let Some(rx) = node
                .reactions
                .iter()
                .find(|r| matches!(r.trigger, Trigger::MouseLeave))
            {
                fired.push((
                    *exited,
                    rx.id,
                    rx.actions().cloned().collect(),
                    rx.transition.clone(),
                    rx.animation.clone(),
                ));
            }
        }
        let mut response = PresentResponse::default();
        for reaction in fired {
            let frame_before = self.current;
            response = merge_responses(response, self.execute_matching(Some(reaction)));
            if response.exited || self.current != frame_before {
                break;
            }
        }
        response
    }

    /// The hit node's ancestor chain (hit first, walking `parent` up), or
    /// empty when nothing is hit. This is what "the pointer is inside node X"
    /// means for enter/leave edges: hovering a child hovers every ancestor.
    fn ancestor_chain(&self, hit: Option<NodeId>) -> Vec<NodeId> {
        let mut chain = Vec::new();
        let mut cursor = hit;
        while let Some(id) = cursor {
            chain.push(id);
            cursor = self.doc.scene.get(id).and_then(|n| n.parent);
        }
        chain
    }

    fn active_pointer_reaction(&self, reaction: &MatchedReaction) -> ActivePointerReaction {
        let (source, reaction_id, actions, _, _) = reaction;
        // The transient hover/press effect that gets reverted on leave/release
        // is the variant swap; the first UpdateVariant in the sequence is the
        // one whose pre-state we must remember.
        let variant_restore = actions.iter().find_map(|action| match action {
            Action::UpdateVariant { component, variant } => self
                .source_instance(*source, *component)
                .map(|instance| InstanceVariantRestore {
                    instance,
                    previous: self.instance_variant_overrides.get(&instance).cloned(),
                    applied: (*component, variant.clone()),
                }),
            _ => None,
        });
        ActivePointerReaction {
            source: *source,
            reaction: *reaction_id,
            variant_restore,
        }
    }

    fn restore_hovered_reaction(&mut self) -> PresentResponse {
        let active = self.hovered_reaction.take();
        self.restore_pointer_reaction(active)
    }

    fn restore_pressed_reaction(&mut self) -> PresentResponse {
        let active = self.pressed_reaction.take();
        self.restore_pointer_reaction(active)
    }

    fn restore_pointer_reaction(
        &mut self,
        active: Option<ActivePointerReaction>,
    ) -> PresentResponse {
        let Some(restore) = active.and_then(|active| active.variant_restore) else {
            return PresentResponse::default();
        };
        if self.instance_variant_overrides.get(&restore.instance) != Some(&restore.applied) {
            return PresentResponse::default();
        }
        match restore.previous {
            Some(previous) => {
                self.instance_variant_overrides
                    .insert(restore.instance, previous);
            }
            None => {
                self.instance_variant_overrides.remove(&restore.instance);
            }
        }
        self.active_variant_transition = None;
        PresentResponse {
            needs_redraw: true,
            ..PresentResponse::default()
        }
    }

    /// Execute a reaction's full action sequence (primary first, then any
    /// `extra_actions`) in one gesture. Each action sees the state its
    /// predecessors produced — a SetVariable-then-Navigate lands on the
    /// destination with the variable already set. THE STOP RULE: an action
    /// that CHANGES the current frame (a Navigate somewhere new, a Back that
    /// pops) ends the sequence, because the remaining actions were authored
    /// against the frame that was just left; likewise a Close that exits
    /// present mode ends it. Non-navigating actions (SetVariable, ScrollTo,
    /// OpenLink, overlay operations) let the sequence continue.
    fn execute_reaction(
        &mut self,
        source: NodeId,
        actions: &[Action],
        transition: Option<&Transition>,
        animation: Option<&PrototypeAnimation>,
    ) -> PresentResponse {
        let mut action_response = PresentResponse::default();
        for action in actions {
            let frame_before = self.current;
            action_response =
                merge_responses(action_response, self.execute(source, action, transition));
            if action_response.exited {
                return action_response;
            }
            if self.current != frame_before {
                break;
            }
        }
        let animation_response = animation.map_or_else(PresentResponse::default, |animation| {
            self.start_prototype_animation(animation)
        });
        merge_responses(action_response, animation_response)
    }

    fn start_prototype_animation(&mut self, animation: &PrototypeAnimation) -> PresentResponse {
        let replaced_active_animation = self.active_prototype_animation.take().is_some();
        if self.doc.motion.clip(animation.clip).is_none() {
            return PresentResponse {
                needs_redraw: replaced_active_animation,
                ..PresentResponse::default()
            };
        }
        let active = ActivePrototypeAnimation {
            clip: animation.clip,
            delay: f64::from(animation.delay_ms) / 1000.0,
            elapsed: 0.0,
        };
        let animating = active.is_running(&self.doc);
        let needs_redraw = replaced_active_animation || animation.delay_ms == 0;
        self.active_prototype_animation = Some(active);
        PresentResponse {
            needs_redraw,
            animating,
            ..PresentResponse::default()
        }
    }

    fn execute(
        &mut self,
        source: NodeId,
        action: &Action,
        transition: Option<&Transition>,
    ) -> PresentResponse {
        match action {
            Action::Navigate { to } => {
                if !self.doc.scene.contains(*to) {
                    return PresentResponse::default();
                }
                let from_frame = self.current;
                let from_viewport = self.viewport;
                let scroll_snapshot = self.scroll_offsets.clone();
                self.back_stack.push(NavEntry {
                    frame: from_frame,
                    viewport: from_viewport,
                    scroll_snapshot,
                });
                self.enter_frame(*to);
                let animating = self.begin_transition(
                    from_frame,
                    from_viewport,
                    *to,
                    self.viewport,
                    transition,
                );
                PresentResponse {
                    needs_redraw: true,
                    navigated: true,
                    animating,
                    ..PresentResponse::default()
                }
            }
            Action::Back => match self.back_stack.pop() {
                Some(entry) => {
                    let from_frame = self.current;
                    let from_viewport = self.viewport;
                    self.enter_frame(entry.frame);
                    self.viewport = entry.viewport;
                    self.scroll_offsets = entry.scroll_snapshot;
                    let animating = self.begin_transition(
                        from_frame,
                        from_viewport,
                        entry.frame,
                        entry.viewport,
                        transition,
                    );
                    PresentResponse {
                        needs_redraw: true,
                        navigated: true,
                        animating,
                        ..PresentResponse::default()
                    }
                }
                None => PresentResponse::default(),
            },
            Action::Close => {
                if let Some(closed) = self.overlays.pop() {
                    let interrupted_entry = self
                        .active_overlay_transition
                        .filter(|active| active.root == closed.root)
                        .map(|active| {
                            (
                                active.clock.style,
                                ease(active.clock.easing, active.clock.progress()),
                            )
                        });
                    self.active_overlay_transition = None;
                    self.active_variant_transition = None;
                    self.active_prototype_animation = None;
                    self.cancel_timers(closed.root);
                    self.restore_hovered_reaction();
                    self.restore_pressed_reaction();
                    self.active_overlay_exit =
                        TransitionClock::from_transition(transition).map(|mut clock| {
                            let start_visible =
                                interrupted_entry.map_or(1.0, |(style, visible)| {
                                    clock.style = style;
                                    visible
                                });
                            ActiveOverlayExit {
                                entry: closed,
                                clock,
                                start_visible,
                            }
                        });
                    PresentResponse {
                        needs_redraw: true,
                        navigated: true,
                        animating: self.active_overlay_exit.is_some(),
                        ..PresentResponse::default()
                    }
                } else {
                    PresentResponse {
                        exited: true,
                        ..PresentResponse::default()
                    }
                }
            }
            Action::SetVariable { variable, value } => {
                if self.set_variable(source, *variable, value.clone()) {
                    PresentResponse {
                        needs_redraw: true,
                        media_updated: false,
                        ..PresentResponse::default()
                    }
                } else {
                    PresentResponse::default()
                }
            }
            Action::OpenOverlay { frame, overlay } => {
                self.open_overlay(*frame, overlay, transition)
            }
            Action::UpdateVariant { component, variant } => {
                let from_global = self.variant_overrides.clone();
                let from_instances = self.instance_variant_overrides.clone();
                if let Some(instance) = self.source_instance(source, *component) {
                    self.instance_variant_overrides
                        .insert(instance, (*component, variant.clone()));
                } else {
                    self.variant_overrides.insert(*component, variant.clone());
                }
                self.active_transition = None;
                self.smart_animate = None;
                self.active_overlay_transition = None;
                self.active_overlay_exit = None;
                self.active_variant_transition = TransitionClock::from_transition(transition)
                    .and_then(|clock| {
                        let layers = self.prepare_variant_layers(
                            &from_global,
                            &from_instances,
                            &self.variant_overrides,
                            &self.instance_variant_overrides,
                        );
                        (!layers.is_empty()).then_some(ActiveVariantTransition { layers, clock })
                    });
                PresentResponse {
                    needs_redraw: true,
                    animating: self.active_variant_transition.is_some(),
                    ..PresentResponse::default()
                }
            }
            Action::OpenLink { url } => {
                self.pending_url = Some(url.clone());
                PresentResponse {
                    needs_redraw: true,
                    ..PresentResponse::default()
                }
            }
            Action::ScrollTo { target } => {
                if let Some(bounds) = self.doc.scene.world_bounds(*target) {
                    if let Some((scrollable, offset)) = self.scroll_target_offset(*target) {
                        // ScrollAnimate (PR-6): ease the offset there instead
                        // of jumping. The destination is pre-clamped so the
                        // tween's endpoint is the exact offset a jump would
                        // have landed on; each tick re-clamps its samples.
                        if let Some(Transition {
                            style: TransitionStyle::ScrollAnimate,
                            duration_ms,
                            easing,
                        }) = transition
                            && *duration_ms > 0
                        {
                            self.active_scroll_animation = Some(ActiveScrollAnimation {
                                scrollable,
                                from: self.scroll_offset(scrollable),
                                to: self.clamped_scroll_offset(scrollable, offset),
                                easing: *easing,
                                duration: f64::from(*duration_ms) / 1000.0,
                                elapsed: 0.0,
                            });
                            return PresentResponse {
                                needs_redraw: true,
                                navigated: true,
                                animating: true,
                                ..PresentResponse::default()
                            };
                        }
                        // Through the clamped setter: a ScrollTo can never
                        // overshoot the container's content or move a
                        // single-axis container diagonally.
                        self.set_scroll_offset(scrollable, offset);
                        return PresentResponse {
                            needs_redraw: true,
                            navigated: true,
                            media_updated: false,
                            ..PresentResponse::default()
                        };
                    }

                    // Default (no scrollable parent): pan the main viewport.
                    let cx = (bounds.min_x + bounds.max_x) * 0.5;
                    let cy = (bounds.min_y + bounds.max_y) * 0.5;
                    self.viewport.center = [cx, cy];
                    PresentResponse {
                        needs_redraw: true,
                        navigated: true,
                        media_updated: false,
                        ..PresentResponse::default()
                    }
                } else {
                    PresentResponse::default()
                }
            }
        }
    }

    fn scroll_target_offset(&self, target: NodeId) -> Option<(NodeId, [f64; 2])> {
        let mut cursor = self.doc.scene.get(target)?.parent;
        let scrollable = loop {
            let id = cursor?;
            let node = self.doc.scene.get(id)?;
            // The nearest ancestor whose AUTHORED axes scroll (imported files
            // carry the real Figma overflow; the legacy `scrollable` bool
            // widens to Both for programmatic scenes).
            if matches!(&node.data, NodeData::Group(group)
                if group.effective_scroll_direction() != ScrollDirection::None)
            {
                break id;
            }
            cursor = node.parent;
        };
        let target_bounds = self.doc.scene.world_bounds(target)?;
        let target_world = DVec2::new(
            (target_bounds.min_x + target_bounds.max_x) * 0.5,
            (target_bounds.min_y + target_bounds.max_y) * 0.5,
        );
        let scrollable_world = self.doc.scene.world_transform(scrollable)?;
        let components = scrollable_world.to_components();
        let determinant = components[0] * components[3] - components[1] * components[2];
        if !determinant.is_finite() || determinant.abs() <= f64::EPSILON {
            return None;
        }
        let target_local = scrollable_world.inverse().transform_point(target_world);
        let viewport_bounds = self.doc.scene.local_bounds(scrollable)?;
        let viewport_center = DVec2::new(
            (viewport_bounds.min_x + viewport_bounds.max_x) * 0.5,
            (viewport_bounds.min_y + viewport_bounds.max_y) * 0.5,
        );
        let offset = target_local - viewport_center;
        Some((scrollable, [offset.x, offset.y]))
    }

    fn source_instance(&self, mut source: NodeId, component: ComponentId) -> Option<NodeId> {
        loop {
            let node = self.doc.scene.get(source)?;
            if let NodeData::Instance(instance) = &node.data {
                let instance_set =
                    variant_set_for_component(&self.doc.components, instance.component)
                        .unwrap_or(instance.component);
                let action_set =
                    variant_set_for_component(&self.doc.components, component).unwrap_or(component);
                if instance.component == component || instance_set == action_set {
                    return Some(source);
                }
            }
            source = node.parent?;
        }
    }

    fn prepare_variant_layers(
        &self,
        from_global: &HashMap<ComponentId, String>,
        from_instances: &HashMap<NodeId, (ComponentId, String)>,
        to_global: &HashMap<ComponentId, String>,
        to_instances: &HashMap<NodeId, (ComponentId, String)>,
    ) -> Vec<VariantLayerTransition> {
        let layer_root = self.layer_root();
        let viewport = self.active_viewport();
        let mut layers = Vec::new();
        for instance in self.doc.scene.descendants_of(layer_root) {
            if !matches!(
                self.doc.scene.get(instance).map(|node| &node.data),
                Some(NodeData::Instance(_))
            ) {
                continue;
            }
            let Some((from_scene, from_root, from_node, _, _)) = self.materialize_variant_instance(
                layer_root,
                instance,
                from_global,
                from_instances,
            ) else {
                continue;
            };
            let Some((
                to_scene,
                to_root,
                target_instance,
                target_parent_world,
                target_master_opacity,
            )) = self.materialize_variant_instance(layer_root, instance, to_global, to_instances)
            else {
                continue;
            };
            if from_node.data == target_instance.data {
                continue;
            }
            let smart_animate =
                smart_animate::prepare_scenes(&from_scene, from_root, &to_scene, to_root);
            layers.push(VariantLayerTransition {
                instance,
                viewport,
                from_scene,
                from_root,
                to_scene,
                to_root,
                smart_animate,
                target_instance,
                target_parent_world,
                target_master_opacity,
            });
        }
        layers
    }

    fn materialize_variant_instance(
        &self,
        layer_root: NodeId,
        instance_id: NodeId,
        global: &HashMap<ComponentId, String>,
        instances: &HashMap<NodeId, (ComponentId, String)>,
    ) -> Option<(Scene, NodeId, fanta_doc::CanvasNode, Transform2D, f32)> {
        let runtime = self
            .build_runtime_scene_with_variant_state(layer_root, global, instances)
            .unwrap_or_else(|| self.doc.scene.clone());
        let instance_node = runtime.get(instance_id)?.clone();
        let NodeData::Instance(instance) = &instance_node.data else {
            return None;
        };
        let parent_world = instance_node
            .parent
            .and_then(|parent| runtime.world_transform(parent))
            .unwrap_or(Transform2D::IDENTITY);
        let instance_world = instance_node.transform.then(&parent_world);
        let context = fanta_doc::InstanceExpansionContext::new(
            &self.variables,
            &self.active_modes,
            instance_id,
        );
        let mut expanded = fanta_doc::expand_instance_with_context(
            &runtime,
            &self.doc.components,
            instance,
            &context,
        );
        if expanded.is_empty() {
            return None;
        }
        if instance.derived.is_empty() {
            fanta_doc::solve_expanded(&mut expanded, &mut fanta_render::measure_text_node);
        }
        let root = expanded
            .iter()
            .find(|entry| entry.def_path.is_empty())?
            .node
            .id;
        let root_opacity = expanded
            .iter()
            .find(|entry| entry.node.id == root)?
            .node
            .opacity
            .get();
        let mut materialized = runtime;
        for mut entry in expanded {
            if entry.node.id == root {
                entry.node.parent = None;
                entry.node.transform = instance_world;
                entry.node.opacity = fanta_doc::UnitInterval::new(
                    entry.node.opacity.get() * instance_node.opacity.get(),
                );
            }
            if materialized.insert(entry.node).is_err() {
                return None;
            }
        }
        Some((
            materialized,
            root,
            instance_node,
            parent_world,
            root_opacity,
        ))
    }

    /// Push an overlay frame onto the stack, placed per its settings at the
    /// current layer's zoom. A missing or bounds-less overlay frame is a no-op.
    fn open_overlay(
        &mut self,
        frame: NodeId,
        settings: &OverlaySettings,
        transition: Option<&Transition>,
    ) -> PresentResponse {
        if !self.doc.scene.contains(frame) {
            return PresentResponse::default();
        }
        let Some(bounds) = self.doc.scene.world_bounds(frame) else {
            return PresentResponse::default();
        };
        let base_zoom = self.active_viewport().zoom;
        let (viewport, screen_rect) =
            place_overlay(base_zoom, bounds, settings.position, self.screen_size);
        self.restore_hovered_reaction();
        self.restore_pressed_reaction();
        self.overlays.push(OverlayEntry {
            root: frame,
            settings: settings.clone(),
            viewport,
            screen_rect,
        });
        self.schedule_timers(frame);
        self.active_transition = None;
        self.smart_animate = None;
        self.active_variant_transition = None;
        self.active_prototype_animation = None;
        self.active_overlay_exit = None;
        self.active_overlay_transition = TransitionClock::from_transition(transition)
            .map(|clock| ActiveOverlayTransition { root: frame, clock });
        PresentResponse {
            needs_redraw: true,
            navigated: true,
            animating: self.active_overlay_transition.is_some(),
            ..PresentResponse::default()
        }
    }

    /// Override a variable's value for the session (never the doc). Writes into
    /// the collection's currently-effective mode of the session-local registry
    /// clone and bumps `mode_generation` so the render + instance cache pick up
    /// the new value. Returns `false` (a no-op) for an unknown variable or one
    /// whose collection is missing.
    fn set_variable(&mut self, source: NodeId, variable: VariableId, value: VarValue) -> bool {
        let Some((collection_id, variable_type)) = self
            .variables
            .variable(variable)
            .map(|variable| (variable.collection, variable.ty))
        else {
            return false;
        };
        if value
            .variable_type()
            .is_some_and(|value_type| value_type != variable_type)
        {
            return false;
        }
        let Some(collection) = self.variables.collections.get(&collection_id) else {
            return false;
        };
        let mode = fanta_doc::resolve_effective_mode(
            &self.doc.scene,
            source,
            collection,
            &self.active_modes,
        );
        match self.variables.variables.get_mut(&variable) {
            Some(var) => {
                var.values_by_mode.insert(mode, value);
                self.mode_generation = self.mode_generation.wrapping_add(1);
                true
            }
            None => false,
        }
    }

    /// Switch the current base frame and fit its viewport. Resets all pending
    /// timers to the new frame's `AfterDelay` reactions.
    fn enter_frame(&mut self, frame: NodeId) {
        self.restore_hovered_reaction();
        self.restore_pressed_reaction();
        // The hover chain tracked nodes of the frame being left; leave edges
        // must not fire against the new frame's coordinate space.
        self.hover_chain.clear();
        self.current = frame;
        self.viewport = frame_viewport(&self.doc.scene, frame, self.screen_size);
        self.overlays.clear();
        self.active_overlay_transition = None;
        self.active_overlay_exit = None;
        self.active_variant_transition = None;
        self.active_prototype_animation = None;
        self.active_scroll_animation = None;
        self.scroll_offsets.clear();
        self.seed_initial_scroll_offsets(frame);
        self.timers.clear();
        self.schedule_timers(frame);
    }

    /// Apply each scrollable descendant's authored initial `scroll_offset`
    /// (Figma `scrollOffset`) when a frame is presented, so content authored
    /// pre-scrolled starts pre-scrolled instead of snapping to the top.
    fn seed_initial_scroll_offsets(&mut self, frame: NodeId) {
        let mut seeded = Vec::new();
        for id in self.doc.scene.descendants_of(frame) {
            if let Some(node) = self.doc.scene.get(id)
                && let NodeData::Group(group) = &node.data
                && group.effective_scroll_direction() != ScrollDirection::None
                && let Some(offset) = group.scroll_offset
            {
                seeded.push((id, offset));
            }
        }
        for (id, offset) in seeded {
            let clamped = self.clamped_scroll_offset(id, offset);
            self.scroll_offsets.insert(id, clamped);
        }
    }

    /// Queue the `AfterDelay` reactions found anywhere in `root`'s subtree as
    /// pending timers owned by `root`.
    fn schedule_timers(&mut self, root: NodeId) {
        let mut scheduled = Vec::new();
        for id in self.doc.scene.descendants_of(root) {
            if let Some(node) = self.doc.scene.get(id) {
                for reaction in &node.reactions {
                    if let Trigger::AfterDelay { delay_ms } = &reaction.trigger {
                        scheduled.push(PendingTimer {
                            root,
                            source: id,
                            reaction: reaction.id,
                            actions: reaction.actions().cloned().collect(),
                            transition: reaction.transition.clone(),
                            animation: reaction.animation.clone(),
                            remaining: *delay_ms as f64 / 1000.0,
                        });
                    }
                }
            }
        }
        self.timers.extend(scheduled);
    }

    /// Drop every pending timer owned by `root` (its layer was closed).
    fn cancel_timers(&mut self, root: NodeId) {
        self.timers.retain(|timer| timer.root != root);
    }

    /// Arm a navigation transition, if the reaction carried an animating one.
    /// An `Instant`/zero-duration transition (or no transition) is a cut: it
    /// clears any prior animation and returns `false`. Otherwise the two
    /// endpoint frames are recorded for [`present_rgba`](Self::present_rgba) to
    /// composite and `true` is returned so the host keeps ticking.
    fn begin_transition(
        &mut self,
        from_frame: NodeId,
        from_viewport: Viewport,
        to_frame: NodeId,
        to_viewport: Viewport,
        transition: Option<&Transition>,
    ) -> bool {
        self.active_overlay_transition = None;
        self.active_overlay_exit = None;
        self.active_variant_transition = None;
        match transition {
            Some(Transition {
                style,
                duration_ms,
                easing,
            }) if !matches!(style, TransitionStyle::Instant) && *duration_ms > 0 => {
                self.active_transition = Some(ActiveTransition {
                    from_frame,
                    from_viewport,
                    to_frame,
                    to_viewport,
                    style: *style,
                    easing: *easing,
                    duration: *duration_ms as f64 / 1000.0,
                    elapsed: 0.0,
                });
                // Smart animate needs a scratch scene of matched-layer tweens; the
                // other styles composite two rendered rasters and need no plan.
                self.smart_animate = if matches!(style, TransitionStyle::SmartAnimate) {
                    smart_animate::prepare(&self.doc, from_frame, to_frame)
                } else {
                    None
                };
                true
            }
            _ => {
                self.active_transition = None;
                self.smart_animate = None;
                false
            }
        }
    }
}

/// The trigger family an input event wants, matched against the authored
/// [`Trigger`] regardless of its payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TriggerKind {
    Click,
    Drag,
    Hover,
    WhilePressing,
}

/// Combine two responses (OR every flag). Used when a [`tick`](PresentSession::tick)
/// both advances a transition and fires one or more timers.
fn merge_responses(a: PresentResponse, b: PresentResponse) -> PresentResponse {
    PresentResponse {
        needs_redraw: a.needs_redraw || b.needs_redraw,
        navigated: a.navigated || b.navigated,
        animating: a.animating || b.animating,
        exited: a.exited || b.exited,
        media_updated: a.media_updated || b.media_updated,
        suppress_click: a.suppress_click || b.suppress_click,
    }
}

fn trigger_matches(trigger: &Trigger, want: TriggerKind) -> bool {
    matches!(
        (trigger, want),
        (Trigger::Click, TriggerKind::Click)
            | (Trigger::Drag, TriggerKind::Drag)
            // The three enter-edge triggers share the hover lane: Hover and
            // MouseEnter fire once on entry and re-arm after leaving;
            // WhileHovering additionally has its reversible effect (a variant
            // swap) undone on leave via the same restore machinery that
            // WhilePressing uses on release. MouseLeave deliberately does NOT
            // match here — it fires on the exit edge (see `fire_mouse_leave`),
            // never on entry.
            | (
                Trigger::Hover | Trigger::MouseEnter | Trigger::WhileHovering,
                TriggerKind::Hover
            )
            | (Trigger::WhilePressing, TriggerKind::WhilePressing)
    )
}

fn down_action_suppresses_click(action: &Action) -> bool {
    matches!(
        action,
        Action::Navigate { .. }
            | Action::Back
            | Action::Close
            | Action::OpenOverlay { .. }
            | Action::ScrollTo { .. }
            | Action::OpenLink { .. }
    )
}

/// The scroll a STICKY child actually consumes along one axis: it scrolls with
/// content until its leading edge (`origin`, its local translation) reaches the
/// frame's origin, then pins there. Negative overscroll passes through so the
/// element never detaches downward.
fn sticky_component(offset: f64, origin: f64) -> f64 {
    if offset <= 0.0 {
        offset
    } else {
        offset.min(origin.max(0.0))
    }
}

/// Whether `id` is `root` or descends from it (walks the `parent` chain).
fn descends_from(scene: &Scene, mut id: NodeId, root: NodeId) -> bool {
    loop {
        if id == root {
            return true;
        }
        match scene.get(id).and_then(|n| n.parent) {
            Some(parent) => id = parent,
            None => return false,
        }
    }
}

/// Fit a frame's world bounds to the present surface. A frame with no bounds
/// (empty) falls back to the default viewport so playback never panics.
fn frame_viewport(scene: &Scene, frame: NodeId, screen_size: DVec2) -> Viewport {
    match scene.world_bounds(frame) {
        Some(bounds) => fit_bounds(bounds, screen_size, FIT_PADDING),
        None => Viewport::default(),
    }
}

fn valid_screen_size(screen_size: DVec2) -> DVec2 {
    fn dimension(value: f64) -> f64 {
        if value.is_finite() && value > 0.0 {
            value
        } else {
            1.0
        }
    }
    DVec2::new(dimension(screen_size.x), dimension(screen_size.y))
}

fn valid_display_scale(display_scale: f64) -> f64 {
    if display_scale.is_finite() && display_scale > 0.0 {
        display_scale
    } else {
        1.0
    }
}

fn rebase_viewport(
    scene: &Scene,
    frame: NodeId,
    viewport: Viewport,
    old_screen_size: DVec2,
    new_screen_size: DVec2,
) -> Viewport {
    let old_fit = frame_viewport(scene, frame, old_screen_size);
    let new_fit = frame_viewport(scene, frame, new_screen_size);
    let zoom_factor = if old_fit.zoom.is_finite() && old_fit.zoom.abs() > f64::EPSILON {
        viewport.zoom / old_fit.zoom
    } else {
        1.0
    };
    let center_offset = [
        viewport.center[0] - old_fit.center[0],
        viewport.center[1] - old_fit.center[1],
    ];
    Viewport {
        center: [
            new_fit.center[0] + center_offset[0],
            new_fit.center[1] + center_offset[1],
        ],
        zoom: (new_fit.zoom * zoom_factor).max(f64::EPSILON),
    }
}

fn apply_variant_override_to_instance(
    overrides: &HashMap<ComponentId, String>,
    instance_override: Option<&(ComponentId, String)>,
    components: &fanta_doc::ComponentLibrary,
    inst: &mut fanta_doc::InstanceNode,
) {
    let direct_overrides = instance_override
        .map(|(component, variant)| HashMap::from([(*component, variant.clone())]));
    let selection = direct_overrides
        .as_ref()
        .and_then(|direct| variant_selection_for_instance(direct, components, inst.component))
        .or_else(|| variant_selection_for_instance(overrides, components, inst.component));
    let Some(selection) = selection else {
        return;
    };

    if let Some(member) = selection.member {
        inst.component = member;
    }

    if selection.axis_values.is_empty() {
        if let Some(value) = selection.fallback_value {
            apply_variant_value_to_props(components, inst, &value);
        }
        return;
    }

    apply_variant_axes_to_props(components, inst, &selection.axis_values);
}

#[derive(Debug)]
struct VariantSelection {
    member: Option<ComponentId>,
    axis_values: BTreeMap<String, String>,
    fallback_value: Option<String>,
}

fn variant_selection_for_instance(
    overrides: &HashMap<ComponentId, String>,
    components: &fanta_doc::ComponentLibrary,
    component: ComponentId,
) -> Option<VariantSelection> {
    let set_id = variant_set_for_component(components, component);

    if let Some(value) = overrides.get(&component) {
        if let Some(set_id) = set_id
            && let Some(member) = member_matching_value(components, set_id, value)
        {
            return selection_from_member(components, member);
        }
        return Some(VariantSelection {
            member: None,
            axis_values: BTreeMap::new(),
            fallback_value: Some(value.clone()),
        });
    }

    let set_id = set_id?;
    member_override_in_set(overrides, components, set_id)
        .and_then(|member| selection_from_member(components, member))
        .or_else(|| {
            overrides
                .get(&set_id)
                .and_then(|value| member_matching_value(components, set_id, value))
                .and_then(|member| selection_from_member(components, member))
        })
        .or_else(|| {
            overrides.get(&set_id).map(|value| VariantSelection {
                member: None,
                axis_values: BTreeMap::new(),
                fallback_value: Some(value.clone()),
            })
        })
}

fn variant_set_for_component(
    components: &fanta_doc::ComponentLibrary,
    component: ComponentId,
) -> Option<ComponentId> {
    if components.sets.contains_key(&component) {
        return Some(component);
    }
    components
        .def(component)
        .and_then(|def| def.variant_of.as_ref())
        .map(|membership| membership.set)
}

fn member_override_in_set(
    overrides: &HashMap<ComponentId, String>,
    components: &fanta_doc::ComponentLibrary,
    set_id: ComponentId,
) -> Option<ComponentId> {
    components
        .sets
        .get(&set_id)?
        .members
        .iter()
        .copied()
        .find(|member| overrides.contains_key(member))
}

fn member_matching_value(
    components: &fanta_doc::ComponentLibrary,
    set_id: ComponentId,
    value: &str,
) -> Option<ComponentId> {
    components
        .sets
        .get(&set_id)?
        .members
        .iter()
        .copied()
        .find(|member| {
            let Some(def) = components.def(*member) else {
                return false;
            };
            def.name == value
                || def
                    .variant_of
                    .as_ref()
                    .is_some_and(|membership| membership.axis_values.values().any(|v| v == value))
        })
}

fn selection_from_member(
    components: &fanta_doc::ComponentLibrary,
    member: ComponentId,
) -> Option<VariantSelection> {
    let def = components.def(member)?;
    let axis_values = def.variant_of.as_ref()?.axis_values.clone();
    Some(VariantSelection {
        member: Some(member),
        axis_values,
        fallback_value: None,
    })
}

fn apply_variant_axes_to_props(
    components: &fanta_doc::ComponentLibrary,
    inst: &mut fanta_doc::InstanceNode,
    axis_values: &BTreeMap<String, String>,
) {
    let Some(set_id) = variant_set_for_component(components, inst.component) else {
        return;
    };
    let Some(set) = components.sets.get(&set_id) else {
        return;
    };
    for &member_id in &set.members {
        let Some(def) = components.def(member_id) else {
            continue;
        };
        for prop in &def.props {
            if let ComponentPropKind::Variant { axis } = &prop.kind
                && let Some(value) = axis_values.get(axis)
            {
                inst.prop_values.insert(
                    prop.id,
                    VarValue::String {
                        value: value.clone(),
                    },
                );
            }
        }
    }
}

fn apply_variant_value_to_props(
    components: &fanta_doc::ComponentLibrary,
    inst: &mut fanta_doc::InstanceNode,
    value: &str,
) {
    let Some(set_id) = variant_set_for_component(components, inst.component) else {
        return;
    };
    let Some(set) = components.sets.get(&set_id) else {
        return;
    };
    for &member_id in &set.members {
        let Some(def) = components.def(member_id) else {
            continue;
        };
        for prop in &def.props {
            if matches!(prop.kind, ComponentPropKind::Variant { .. }) {
                inst.prop_values.insert(
                    prop.id,
                    VarValue::String {
                        value: value.to_owned(),
                    },
                );
            }
        }
    }
}

#[cfg(test)]
mod tests;
