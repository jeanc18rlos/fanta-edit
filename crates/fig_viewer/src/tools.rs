//! Bridges GPUI input to the `fanta-tools` state machines.
//!
//! The shell keeps one boxed [`Tool`] alive per view, translates GPUI mouse
//! and key events into [`ToolEvent`]s, and stores the returned render hints
//! ([`ToolOverlay`]s and a [`CursorHint`]) for the canvas element to paint.

use fanta_doc::{Color, Doc, Viewport};
use fanta_tools::{
    Button, CursorHint, EllipseTool, FrameTool, HandTool, KeyEvent, LineTool, LogicalKey,
    ModifierKeys, NodeEditTool, PenTool, PointerEvent, PolygonTool, RectTool, SelectTool, StarTool,
    TextTool, Tool, ToolContext, ToolEvent, ToolOverlay, ToolResponse,
};
use glam::DVec2;
use gpui::{CursorStyle, Modifiers, MouseButton};
use ui::IconName;

/// Fill assigned to newly drawn shapes until the shell grows a color palette.
const NEW_SHAPE_FILL: Color = Color::rgb(0xD9, 0xD9, 0xD9);

/// The floating toolbar's tools, grouped Figma-style: selection, frame,
/// shapes, drawing, pan. Groups render with separators between them.
pub const TOOLBAR_GROUPS: [&[ToolKind]; 5] = [
    &[ToolKind::Select, ToolKind::NodeEdit],
    &[ToolKind::Frame],
    &[
        ToolKind::Rect,
        ToolKind::Ellipse,
        ToolKind::Line,
        ToolKind::Polygon,
        ToolKind::Star,
    ],
    &[ToolKind::Pen, ToolKind::Text],
    &[ToolKind::Hand],
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolKind {
    Select,
    NodeEdit,
    Hand,
    Rect,
    Ellipse,
    Line,
    Polygon,
    Star,
    Pen,
    Frame,
    Text,
}

impl ToolKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Select => "Move",
            Self::NodeEdit => "Edit Path",
            Self::Hand => "Hand",
            Self::Rect => "Rectangle",
            Self::Ellipse => "Ellipse",
            Self::Line => "Line",
            Self::Polygon => "Polygon",
            Self::Star => "Star",
            Self::Pen => "Pen",
            Self::Frame => "Frame",
            Self::Text => "Text",
        }
    }

    pub fn icon(self) -> IconName {
        match self {
            Self::Select => IconName::ToolSelect,
            Self::NodeEdit => IconName::ToolNodeEdit,
            Self::Hand => IconName::ToolHand,
            Self::Rect => IconName::ToolRect,
            Self::Ellipse => IconName::ToolEllipse,
            Self::Line => IconName::ToolLine,
            Self::Polygon => IconName::ToolPolygon,
            Self::Star => IconName::ToolStar,
            Self::Pen => IconName::ToolPen,
            Self::Frame => IconName::ToolFrame,
            Self::Text => IconName::ToolText,
        }
    }

    fn build(self) -> Box<dyn Tool> {
        match self {
            Self::Select => Box::new(SelectTool::new()),
            Self::NodeEdit => Box::new(NodeEditTool::new()),
            Self::Hand => Box::new(HandTool::new()),
            Self::Rect => Box::new(RectTool::new()),
            Self::Ellipse => Box::new(EllipseTool::new()),
            Self::Line => Box::new(LineTool::new()),
            Self::Polygon => Box::new(PolygonTool::new()),
            Self::Star => Box::new(StarTool::new()),
            Self::Pen => Box::new(PenTool::new()),
            Self::Frame => Box::new(FrameTool::new()),
            Self::Text => Box::new(TextTool::new()),
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
