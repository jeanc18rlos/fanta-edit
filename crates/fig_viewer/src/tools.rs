//! Bridges GPUI input to the `fanta-tools` state machines.
//!
//! The shell keeps one boxed [`Tool`] alive per view, translates GPUI mouse
//! and key events into [`ToolEvent`]s, and stores the returned render hints
//! ([`ToolOverlay`]s and a [`CursorHint`]) for the canvas element to paint.

use fanta_doc::{Color, Doc, Viewport};
use fanta_tools::{
    Button, CursorHint, EllipseTool, FrameTool, HandTool, KeyEvent, LineTool, LogicalKey,
    ModifierKeys, NodeEditTool, PathSelectTool, PenTool, PencilTool, PointerEvent, PolygonTool,
    RectTool, ScaleTool, SectionTool, SelectTool, SliceTool, StarTool, TextPathTool, TextTool,
    Tool, ToolContext, ToolEvent, ToolOverlay, ToolResponse,
};
use glam::DVec2;
use gpui::{CursorStyle, Modifiers, MouseButton};
use ui::IconName;

/// Fill (and text color) assigned to newly drawn shapes/text until the shell
/// grows a full color palette UI. Black default so newly added elements land
/// with visible/opaque color (not low-contrast gray or "without color").
const NEW_SHAPE_FILL: Color = Color::BLACK;

/// The floating toolbar's tools, grouped Figma-style. Each group renders as one
/// button showing the group's active/last-used tool plus a caret that opens a
/// dropdown of the group's members; the zoom cluster is rendered separately.
/// Order: navigation, layout, shape, line/vector, text.
pub const TOOLBAR_GROUPS: [&[ToolKind]; 6] = [
    &[
        ToolKind::Select,
        ToolKind::PathSelect,
        ToolKind::Hand,
        ToolKind::Scale,
    ],
    &[ToolKind::Frame, ToolKind::Section, ToolKind::Slice],
    &[
        ToolKind::Rect,
        ToolKind::Ellipse,
        ToolKind::Line,
        ToolKind::Polygon,
        ToolKind::Star,
    ],
    &[ToolKind::NodeEdit, ToolKind::Pencil, ToolKind::Pen],
    &[ToolKind::Text, ToolKind::TextPath],
    &[ToolKind::Comment],
];

/// The default face (button icon) for each toolbar group before the user picks
/// a member — each group's first tool. The view stores a mutable copy and
/// updates it as tools are activated so a group keeps showing its last choice.
pub fn initial_group_faces() -> Vec<ToolKind> {
    TOOLBAR_GROUPS
        .iter()
        .filter_map(|g| g.first().copied())
        .collect()
}

/// The index of the toolbar group that contains `kind`, if any.
pub fn group_index_of(kind: ToolKind) -> Option<usize> {
    TOOLBAR_GROUPS.iter().position(|g| g.contains(&kind))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolKind {
    Select,
    PathSelect,
    NodeEdit,
    Hand,
    Scale,
    Rect,
    Ellipse,
    Line,
    Polygon,
    Star,
    Pen,
    Pencil,
    Frame,
    Section,
    Slice,
    Text,
    TextPath,
    Comment,
}

impl ToolKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Select => "Move",
            Self::PathSelect => "Path Selection",
            Self::NodeEdit => "Edit Path",
            Self::Hand => "Hand",
            Self::Scale => "Scale",
            Self::Rect => "Rectangle",
            Self::Ellipse => "Ellipse",
            Self::Line => "Line",
            Self::Polygon => "Polygon",
            Self::Star => "Star",
            Self::Pen => "Pen",
            Self::Pencil => "Pencil",
            Self::Frame => "Frame",
            Self::Section => "Section",
            Self::Slice => "Slice",
            Self::Text => "Text",
            Self::TextPath => "Text on Path",
            Self::Comment => "Comment",
        }
    }

    pub fn icon(self) -> IconName {
        match self {
            Self::Select => IconName::ToolSelect,
            Self::PathSelect => IconName::ToolPathSelect,
            Self::NodeEdit => IconName::ToolNodeEdit,
            Self::Hand => IconName::ToolHand,
            Self::Scale => IconName::ToolScale,
            Self::Rect => IconName::ToolRect,
            Self::Ellipse => IconName::ToolEllipse,
            Self::Line => IconName::ToolLine,
            Self::Polygon => IconName::ToolPolygon,
            Self::Star => IconName::ToolStar,
            Self::Pen => IconName::ToolPen,
            Self::Pencil => IconName::ToolPencil,
            Self::Frame => IconName::ToolFrame,
            Self::Section => IconName::ToolSection,
            Self::Slice => IconName::ToolSlice,
            Self::Text => IconName::ToolText,
            Self::TextPath => IconName::ToolTextPath,
            Self::Comment => IconName::Chat,
        }
    }

    /// Placeholder tools that appear in the toolbar but have no behavior yet
    /// (Scale, direct path-selection, text-on-path). Rendered with a "soon" hint
    /// so it's clear they're not wired up.
    pub fn is_stub(self) -> bool {
        matches!(self, Self::Scale | Self::PathSelect | Self::TextPath)
    }

    fn build(self) -> Box<dyn Tool> {
        match self {
            Self::Select => Box::new(SelectTool::new()),
            Self::PathSelect => Box::new(PathSelectTool::new()),
            Self::NodeEdit => Box::new(NodeEditTool::new()),
            Self::Hand => Box::new(HandTool::new()),
            Self::Scale => Box::new(ScaleTool::new()),
            Self::Rect => Box::new(RectTool::new()),
            Self::Ellipse => Box::new(EllipseTool::new()),
            Self::Line => Box::new(LineTool::new()),
            Self::Polygon => Box::new(PolygonTool::new()),
            Self::Star => Box::new(StarTool::new()),
            Self::Pen => Box::new(PenTool::new()),
            Self::Pencil => Box::new(PencilTool::new()),
            Self::Frame => Box::new(FrameTool::new()),
            Self::Section => Box::new(SectionTool::new()),
            Self::Slice => Box::new(SliceTool::new()),
            Self::Text => Box::new(TextTool::new()),
            Self::TextPath => Box::new(TextPathTool::new()),
            // Comment mode is handled by the shell (click-to-pin); the select
            // tool backs it so unconsumed events stay harmless.
            Self::Comment => Box::new(SelectTool::new()),
        }
    }
}

/// The per-view tool shell: the live tool instance plus the render hints from
/// the most recent event.
pub struct ToolShell {
    kind: ToolKind,
    tool: Box<dyn Tool>,
    pub overlays: Vec<ToolOverlay>,
    pub cursor: Option<CursorHint>,
}

impl ToolShell {
    pub fn new() -> Self {
        Self {
            kind: ToolKind::Select,
            tool: ToolKind::Select.build(),
            overlays: Vec::new(),
            cursor: None,
        }
    }

    pub fn kind(&self) -> ToolKind {
        self.kind
    }

    /// Switch tools, letting the old tool abort any in-flight gesture and the
    /// new one reset its state.
    pub fn activate(&mut self, kind: ToolKind, ctx: &mut ToolContext) {
        if self.kind == kind {
            return;
        }
        self.tool.deactivate(ctx);
        self.kind = kind;
        self.tool = kind.build();
        self.tool.activate(ctx);
        self.overlays.clear();
        self.cursor = None;
    }

    /// Switch tools when the canvas has not produced bounds/viewport yet.
    ///
    /// Toolbar state should still update immediately during initial layout or
    /// after a document reload; the next real tool event will be delivered with
    /// a full [`ToolContext`].
    pub fn activate_without_context(&mut self, kind: ToolKind) {
        if self.kind == kind {
            return;
        }
        self.kind = kind;
        self.tool = kind.build();
        self.overlays.clear();
        self.cursor = None;
    }

    pub fn cancel_and_activate(&mut self, kind: ToolKind, ctx: &mut ToolContext) {
        self.tool.deactivate(ctx);
        self.kind = kind;
        self.tool = kind.build();
        self.tool.activate(ctx);
        self.overlays.clear();
        self.cursor = None;
    }

    /// Feed one event through the active tool and record its render hints.
    /// Returns the response so the caller can react to `wants_exit`.
    pub fn handle_event(&mut self, ctx: &mut ToolContext, event: ToolEvent) -> ToolResponse {
        let response = self.tool.handle_event(ctx, event);
        self.overlays = response.overlays.to_vec();
        if response.cursor.is_some() {
            self.cursor = response.cursor;
        }
        response
    }

    pub fn cursor_style(&self, dragging_canvas: bool) -> CursorStyle {
        if dragging_canvas {
            return CursorStyle::ClosedHand;
        }
        match self.cursor {
            Some(CursorHint::Crosshair) => CursorStyle::Crosshair,
            Some(CursorHint::Grab) => CursorStyle::OpenHand,
            Some(CursorHint::Grabbing) => CursorStyle::ClosedHand,
            Some(CursorHint::Text) => CursorStyle::IBeam,
            Some(CursorHint::Move) | Some(CursorHint::Default) => CursorStyle::Arrow,
            None => match self.kind {
                ToolKind::Hand => CursorStyle::OpenHand,
                ToolKind::Select => CursorStyle::Arrow,
                ToolKind::Text => CursorStyle::IBeam,
                _ => CursorStyle::Crosshair,
            },
        }
    }
}

/// Build the shared context every tool event needs. `viewport` is the view's
/// local copy; the caller writes it back after the event since tools like the
/// hand mutate it.
pub fn tool_context<'a>(
    doc: &'a mut Doc,
    viewport: &'a mut Viewport,
    screen_size: DVec2,
) -> ToolContext<'a> {
    let snap = fanta_canvas::SnapEngine {
        zoom: viewport.zoom,
        ..Default::default()
    };
    ToolContext::new(doc, viewport, snap, screen_size).with_new_shape_fill(NEW_SHAPE_FILL)
}

pub fn modifier_keys(modifiers: Modifiers) -> ModifierKeys {
    let mut keys = ModifierKeys::empty();
    if modifiers.shift {
        keys |= ModifierKeys::SHIFT;
    }
    if modifiers.alt {
        keys |= ModifierKeys::ALT;
    }
    if modifiers.control {
        keys |= ModifierKeys::CTRL;
    }
    if modifiers.platform {
        keys |= ModifierKeys::META;
    }
    keys
}

pub fn pointer_button(button: MouseButton) -> Option<Button> {
    match button {
        MouseButton::Left => Some(Button::Primary),
        MouseButton::Right => Some(Button::Secondary),
        MouseButton::Middle => Some(Button::Middle),
        _ => None,
    }
}

pub fn press_event(
    screen: DVec2,
    button: Button,
    modifiers: Modifiers,
    click_count: usize,
) -> ToolEvent {
    ToolEvent::Pointer(PointerEvent::Press {
        screen: [screen.x, screen.y],
        button,
        modifiers: modifier_keys(modifiers),
        count: click_count.min(u8::MAX as usize) as u8,
    })
}

pub fn move_event(screen: DVec2, modifiers: Modifiers) -> ToolEvent {
    ToolEvent::Pointer(PointerEvent::Move {
        screen: [screen.x, screen.y],
        modifiers: modifier_keys(modifiers),
    })
}

pub fn release_event(screen: DVec2, button: Button, modifiers: Modifiers) -> ToolEvent {
    ToolEvent::Pointer(PointerEvent::Release {
        screen: [screen.x, screen.y],
        button,
        modifiers: modifier_keys(modifiers),
    })
}

pub fn key_event(key: LogicalKey, modifiers: Modifiers) -> ToolEvent {
    ToolEvent::Key(KeyEvent::with_modifiers(key, modifier_keys(modifiers)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::NavigationDirection;

    #[test]
    fn pointer_buttons_map_and_navigation_buttons_are_dropped() {
        assert_eq!(pointer_button(MouseButton::Left), Some(Button::Primary));
        assert_eq!(pointer_button(MouseButton::Right), Some(Button::Secondary));
        assert_eq!(pointer_button(MouseButton::Middle), Some(Button::Middle));
        assert_eq!(
            pointer_button(MouseButton::Navigate(NavigationDirection::Back)),
            None
        );
        assert_eq!(
            pointer_button(MouseButton::Navigate(NavigationDirection::Forward)),
            None
        );
    }

    #[test]
    fn each_gpui_modifier_maps_to_its_tool_flag() {
        assert_eq!(modifier_keys(Modifiers::default()), ModifierKeys::empty());
        let cases = [
            (
                Modifiers {
                    shift: true,
                    ..Default::default()
                },
                ModifierKeys::SHIFT,
            ),
            (
                Modifiers {
                    alt: true,
                    ..Default::default()
                },
                ModifierKeys::ALT,
            ),
            (
                Modifiers {
                    control: true,
                    ..Default::default()
                },
                ModifierKeys::CTRL,
            ),
            (
                Modifiers {
                    platform: true,
                    ..Default::default()
                },
                ModifierKeys::META,
            ),
        ];
        for (gpui_modifiers, expected) in cases {
            assert_eq!(modifier_keys(gpui_modifiers), expected);
        }
        let all = Modifiers {
            shift: true,
            alt: true,
            control: true,
            platform: true,
            ..Default::default()
        };
        assert_eq!(
            modifier_keys(all),
            ModifierKeys::SHIFT | ModifierKeys::ALT | ModifierKeys::CTRL | ModifierKeys::META
        );
    }

    #[test]
    fn the_function_modifier_maps_to_no_tool_flag() {
        let function_only = Modifiers {
            function: true,
            ..Default::default()
        };
        assert_eq!(modifier_keys(function_only), ModifierKeys::empty());
    }

    #[test]
    fn press_event_carries_position_button_modifiers_and_count() {
        let shift = Modifiers {
            shift: true,
            ..Default::default()
        };
        let event = press_event(DVec2::new(12.5, -3.0), Button::Primary, shift, 2);
        assert_eq!(
            event,
            ToolEvent::Pointer(PointerEvent::Press {
                screen: [12.5, -3.0],
                button: Button::Primary,
                modifiers: ModifierKeys::SHIFT,
                count: 2,
            })
        );
    }

    #[test]
    fn press_event_saturates_the_click_count_at_u8_max() {
        let event = press_event(DVec2::ZERO, Button::Primary, Modifiers::default(), 300);
        let ToolEvent::Pointer(PointerEvent::Press { count, .. }) = event else {
            panic!("expected a pointer press");
        };
        assert_eq!(count, u8::MAX);
    }

    #[test]
    fn move_release_and_key_events_map_their_fields() {
        let alt = Modifiers {
            alt: true,
            ..Default::default()
        };
        assert_eq!(
            move_event(DVec2::new(1.0, 2.0), alt),
            ToolEvent::Pointer(PointerEvent::Move {
                screen: [1.0, 2.0],
                modifiers: ModifierKeys::ALT,
            })
        );
        assert_eq!(
            release_event(DVec2::new(3.0, 4.0), Button::Secondary, alt),
            ToolEvent::Pointer(PointerEvent::Release {
                screen: [3.0, 4.0],
                button: Button::Secondary,
                modifiers: ModifierKeys::ALT,
            })
        );
        assert_eq!(
            key_event(LogicalKey::Escape, alt),
            ToolEvent::Key(KeyEvent::with_modifiers(
                LogicalKey::Escape,
                ModifierKeys::ALT
            ))
        );
    }

    #[test]
    fn every_toolbar_group_member_resolves_to_its_own_group() {
        for (group_index, group) in TOOLBAR_GROUPS.iter().enumerate() {
            for &kind in *group {
                assert_eq!(
                    group_index_of(kind),
                    Some(group_index),
                    "{kind:?} must resolve to group {group_index}"
                );
            }
        }
        assert_eq!(initial_group_faces().len(), TOOLBAR_GROUPS.len());
    }
}
