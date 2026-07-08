//! Input event types feeding the tool state machines.
//!
//! ## Why a dedicated event layer
//!
//! Pointer events, keyboard events, and modifier state arrive from three
//! different OS-level sources (winit / GPUI / IME). Each tool needs to react to
//! all three in lockstep — a shift-key press in the middle of a drag changes
//! the constraint, a modifier release at pointer-up changes which selection
//! semantics fire. Normalizing them into a single [`ToolEvent`] sum type means
//! the tool implementations have one match arm shape to reason about, which
//! makes their state machines testable without a window or a runtime.
//!
//! Mirrors the input model used by tldraw and OpenPencil #1 — the difference
//! is that we never carry raw winit/Dom event references through this layer,
//! only the small struct shapes below. That keeps `fanta-tools` self-contained
//! and decoupled from the eventual GPUI shell in `fanta-app`.

use bitflags::bitflags;
use glam::DVec2;
use serde::{Deserialize, Serialize};

bitflags! {
    /// Bit-packed modifier keyboard state for the current event.
    ///
    /// Stored on every [`PointerEvent`] so a tool can decide its constraint
    /// behavior without consulting a separate "current keyboard state" cache.
    /// Mirrors how Figma / Sketch / tldraw model modifier keys: a snapshot per
    /// event, not a side-channel observable.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
    #[serde(transparent)]
    pub struct ModifierKeys: u32 {
        /// Shift — constrain to axis / aspect, extend selection.
        const SHIFT = 1 << 0;
        /// Alt / Option — duplicate, draw-from-center.
        const ALT = 1 << 1;
        /// Control — context menu, secondary modifier.
        const CTRL = 1 << 2;
        /// Meta (Cmd on macOS, Win on Windows) — primary modifier on macOS.
        const META = 1 << 3;
    }
}

impl ModifierKeys {
    /// Whether the conventional "extend selection" modifier is held. We treat
    /// either Shift or Meta as extend so the tool behaves correctly on macOS
    /// (Cmd-click) and Windows/Linux (Ctrl/Shift-click) without forcing the
    /// shell to normalize. Specific call sites can still test [`SHIFT`] alone
    /// when they want strict shift-click semantics.
    ///
    /// [`SHIFT`]: ModifierKeys::SHIFT
    pub fn extend_selection(self) -> bool {
        self.intersects(Self::SHIFT | Self::META | Self::CTRL)
    }
}

/// Which mouse button generated a pointer event. Touch / pen taps map to
/// `Primary`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Button {
    /// Left mouse button. Drives every editing tool.
    Primary,
    /// Right mouse button. Reserved for context menus (handled by the shell,
    /// not by the tools themselves).
    Secondary,
    /// Middle mouse button. Conventionally pans; the shell may translate this
    /// into a [`PointerEvent::Press`] with `button = Primary` while the
    /// [`crate::hand::HandTool`] is active.
    Middle,
}

/// One pointer input event delivered to the active tool.
///
/// Coordinates are screen-space (pixels, with origin at the top-left of the
/// canvas viewport rectangle). Tools translate to world coordinates through
/// [`crate::context::ToolContext::screen_to_world`] when they need world-space
/// math — keeping the source-of-truth screen coords here avoids accumulated
/// f32 precision loss in the chain of pan/zoom transforms.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PointerEvent {
    /// Pointer button went down.
    Press {
        screen: [f64; 2],
        button: Button,
        modifiers: ModifierKeys,
        /// Click-sequence count: 1 = single, 2 = double (3+ reserved). The
        /// shell derives this from press timing + distance; tools that don't
        /// care treat every press the same (a double-click is still a normal
        /// press for shape tools). The select tool reads it for double-click
        /// "drill into a frame". Defaults to 1 for older serialized events.
        #[serde(default = "default_click_count")]
        count: u8,
    },
    /// Pointer moved. Fires both during a drag (button held) and during a
    /// hover (no button held). Tools that care about the distinction track it
    /// internally via the most recent [`Press`].
    ///
    /// [`Press`]: PointerEvent::Press
    Move {
        screen: [f64; 2],
        modifiers: ModifierKeys,
    },
    /// Pointer button released.
    Release {
        screen: [f64; 2],
        button: Button,
        modifiers: ModifierKeys,
    },
    /// Scroll wheel / trackpad scroll. The two-axis delta is in screen pixels.
    /// Scroll is fed through the tool layer (rather than directly to the
    /// viewport) so a tool can intercept it — the pen tool, for example, might
    /// want to ignore stray scroll mid-stroke.
    Scroll {
        screen: [f64; 2],
        delta: [f64; 2],
        modifiers: ModifierKeys,
    },
}

/// Serde default for [`PointerEvent::Press::count`]: a press with no recorded
/// count is a single click.
fn default_click_count() -> u8 {
    1
}

impl PointerEvent {
    /// Convenience: screen position of the event, if any.
    pub fn screen(&self) -> DVec2 {
        let [x, y] = match self {
            Self::Press { screen, .. }
            | Self::Move { screen, .. }
            | Self::Release { screen, .. }
            | Self::Scroll { screen, .. } => *screen,
        };
        DVec2::new(x, y)
    }

    /// Convenience: modifier state at the moment the event fired.
    pub fn modifiers(&self) -> ModifierKeys {
        match self {
            Self::Press { modifiers, .. }
            | Self::Move { modifiers, .. }
            | Self::Release { modifiers, .. }
            | Self::Scroll { modifiers, .. } => *modifiers,
        }
    }
}

/// Logical keys the tool layer cares about. Text-entry events are *not*
/// modeled here — those go through `fanta-text`'s IME pipeline.
///
/// Stored as a coarse enum (not a key code) because the tool layer only acts
/// on these specific keys. Adding a new key here is intentional and reviewed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LogicalKey {
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    /// Escape — used by every drag-based tool to abort the in-flight gesture.
    Escape,
    /// Enter / Return — confirms a multi-step gesture (future pen tool).
    Enter,
    /// Delete / Backspace — deletes the selection.
    Delete,
}

/// Keyboard event delivered to the active tool. Press-down semantics; a press
/// fires once per key, the shell handles auto-repeat by re-emitting events.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct KeyEvent {
    pub key: LogicalKey,
    pub modifiers: ModifierKeys,
}

impl KeyEvent {
    /// New press event with no modifiers — handy in tests.
    pub fn press(key: LogicalKey) -> Self {
        Self {
            key,
            modifiers: ModifierKeys::empty(),
        }
    }

    /// New press event with the given modifiers.
    pub fn with_modifiers(key: LogicalKey, modifiers: ModifierKeys) -> Self {
        Self { key, modifiers }
    }
}

/// The single event type a tool receives. Pointer + keyboard are unified so a
/// tool's `handle_event` is one method, not three.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "stream", rename_all = "snake_case")]
pub enum ToolEvent {
    Pointer(PointerEvent),
    Key(KeyEvent),
}

impl From<PointerEvent> for ToolEvent {
    fn from(e: PointerEvent) -> Self {
        Self::Pointer(e)
    }
}

impl From<KeyEvent> for ToolEvent {
    fn from(e: KeyEvent) -> Self {
        Self::Key(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_extend_selection_covers_meta_and_shift() {
        assert!(ModifierKeys::SHIFT.extend_selection());
        assert!(ModifierKeys::META.extend_selection());
        assert!(ModifierKeys::CTRL.extend_selection());
        assert!(!ModifierKeys::ALT.extend_selection());
        assert!(!ModifierKeys::empty().extend_selection());
    }

    #[test]
    fn pointer_event_exposes_screen_for_each_variant() {
        let p = PointerEvent::Press {
            screen: [10.0, 20.0],
            button: Button::Primary,
            modifiers: ModifierKeys::empty(),
            count: 1,
        };
        assert_eq!(p.screen(), DVec2::new(10.0, 20.0));

        let dbl = PointerEvent::Press {
            screen: [10.0, 20.0],
            button: Button::Primary,
            modifiers: ModifierKeys::empty(),
            count: 2,
        };
        assert_eq!(dbl.screen(), DVec2::new(10.0, 20.0));
        assert_eq!(dbl.modifiers(), ModifierKeys::empty());

        let m = PointerEvent::Move {
            screen: [1.0, 2.0],
            modifiers: ModifierKeys::SHIFT,
        };
        assert_eq!(m.modifiers(), ModifierKeys::SHIFT);
    }

    #[test]
    fn tool_event_constructs_via_from() {
        let p = PointerEvent::Move {
            screen: [0.0, 0.0],
            modifiers: ModifierKeys::empty(),
        };
        let ev: ToolEvent = p.into();
        assert!(matches!(ev, ToolEvent::Pointer(_)));

        let k = KeyEvent::press(LogicalKey::Escape);
        let ek: ToolEvent = k.into();
        assert!(matches!(ek, ToolEvent::Key(_)));
    }

    #[test]
    fn modifiers_compose_with_bitwise_ops() {
        let combo = ModifierKeys::SHIFT | ModifierKeys::ALT;
        assert!(combo.contains(ModifierKeys::SHIFT));
        assert!(combo.contains(ModifierKeys::ALT));
        assert!(!combo.contains(ModifierKeys::META));
    }

    #[test]
    fn key_event_press_helper_has_no_modifiers() {
        let k = KeyEvent::press(LogicalKey::Escape);
        assert_eq!(k.modifiers, ModifierKeys::empty());
        let kw = KeyEvent::with_modifiers(LogicalKey::ArrowLeft, ModifierKeys::SHIFT);
        assert_eq!(kw.modifiers, ModifierKeys::SHIFT);
    }
}
