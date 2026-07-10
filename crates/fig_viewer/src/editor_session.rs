use std::rc::Rc;

use gpui::{App, Context, EventEmitter, IntoElement, RenderOnce, Window};
use ui::{Tab, TabBar, TabPosition, prelude::*};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EditorMode {
    #[default]
    Design,
    Prototype,
    Motion,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EditorWorkspace {
    #[default]
    Canvas,
    Variables,
}

impl EditorWorkspace {
    pub const ALL: [Self; 2] = [Self::Canvas, Self::Variables];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Canvas => "Canvas",
            Self::Variables => "Variables",
        }
    }
}

impl EditorMode {
    pub const ALL: [Self; 3] = [Self::Design, Self::Prototype, Self::Motion];

    pub fn label(self) -> &'static str {
        match self {
            Self::Design => "Design",
            Self::Prototype => "Prototype",
            Self::Motion => "Motion",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorSessionEvent {
    ModeChanged(EditorMode),
    WorkspaceChanged(EditorWorkspace),
}

pub struct EditorSession {
    mode: EditorMode,
    workspace: EditorWorkspace,
}

impl EditorSession {
    pub fn new() -> Self {
        Self::with_mode(EditorMode::Design)
    }

    pub fn with_mode(mode: EditorMode) -> Self {
        Self::with_state(mode, EditorWorkspace::Canvas)
    }

    pub fn with_state(mode: EditorMode, workspace: EditorWorkspace) -> Self {
        Self { mode, workspace }
    }

    pub fn mode(&self) -> EditorMode {
        self.mode
    }

    pub fn workspace(&self) -> EditorWorkspace {
        self.workspace
    }

    pub fn set_mode(&mut self, mode: EditorMode, cx: &mut Context<Self>) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        cx.emit(EditorSessionEvent::ModeChanged(mode));
        cx.notify();
    }

    pub fn set_workspace(&mut self, workspace: EditorWorkspace, cx: &mut Context<Self>) {
        if self.workspace == workspace {
            return;
        }
        self.workspace = workspace;
        cx.emit(EditorSessionEvent::WorkspaceChanged(workspace));
        cx.notify();
    }
}

impl Default for EditorSession {
    fn default() -> Self {
        Self::new()
    }
}

impl EventEmitter<EditorSessionEvent> for EditorSession {}

type ModeHandler = Rc<dyn Fn(EditorMode, &mut Window, &mut App)>;
type WorkspaceHandler = Rc<dyn Fn(EditorWorkspace, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct EditorModeTabs {
    selected: EditorMode,
    on_select: ModeHandler,
}

impl EditorModeTabs {
    pub fn new(
        selected: EditorMode,
        on_select: impl Fn(EditorMode, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            selected,
            on_select: Rc::new(on_select),
        }
    }
}

impl RenderOnce for EditorModeTabs {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let selected_index = EditorMode::ALL
            .iter()
            .position(|mode| *mode == self.selected)
            .unwrap_or(0);
        let mut tabs = TabBar::new("fanta-editor-mode-tabs");
        for (index, mode) in EditorMode::ALL.into_iter().enumerate() {
            let position = match index {
                0 => TabPosition::First,
                2 => TabPosition::Last,
                _ => TabPosition::Middle(index.cmp(&selected_index)),
            };
            let on_select = self.on_select.clone();
            tabs = tabs.child(
                Tab::new(("fanta-editor-mode", index))
                    .position(position)
                    .toggle_state(mode == self.selected)
                    .on_click(move |_, window, cx| on_select(mode, window, cx))
                    .child(mode.label()),
            );
        }
        tabs
    }
}

#[derive(IntoElement)]
pub struct EditorWorkspaceTabs {
    selected: EditorWorkspace,
    on_select: WorkspaceHandler,
}

impl EditorWorkspaceTabs {
    pub fn new(
        selected: EditorWorkspace,
        on_select: impl Fn(EditorWorkspace, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            selected,
            on_select: Rc::new(on_select),
        }
    }
}

impl RenderOnce for EditorWorkspaceTabs {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let mut tabs = TabBar::new("fanta-editor-workspace-tabs");
        for (index, workspace) in EditorWorkspace::ALL.into_iter().enumerate() {
            let position = if index == 0 {
                TabPosition::First
            } else {
                TabPosition::Last
            };
            let on_select = self.on_select.clone();
            tabs = tabs.child(
                Tab::new(("fanta-editor-workspace", index))
                    .position(position)
                    .toggle_state(workspace == self.selected)
                    .on_click(move |_, window, cx| on_select(workspace, window, cx))
                    .child(workspace.label()),
            );
        }
        tabs
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use gpui::TestAppContext;

    use super::*;

    #[test]
    fn mode_order_and_labels_are_stable() {
        assert_eq!(
            EditorMode::ALL.map(EditorMode::label),
            ["Design", "Prototype", "Motion"]
        );
        assert_eq!(EditorMode::default(), EditorMode::Design);
        assert_eq!(
            EditorWorkspace::ALL.map(EditorWorkspace::label),
            ["Canvas", "Variables"]
        );
    }

    #[gpui::test]
    fn session_emits_only_real_mode_changes(cx: &mut TestAppContext) {
        let session = cx.new(|_| EditorSession::new());
        let events = Rc::new(RefCell::new(Vec::new()));
        let _subscription = cx.update(|cx| {
            let events = events.clone();
            cx.subscribe(&session, move |_, event, _| {
                events.borrow_mut().push(*event);
            })
        });

        session.update(cx, |session, cx| {
            session.set_mode(EditorMode::Prototype, cx);
            session.set_mode(EditorMode::Prototype, cx);
            session.set_mode(EditorMode::Motion, cx);
        });

        assert_eq!(
            events.borrow().as_slice(),
            [
                EditorSessionEvent::ModeChanged(EditorMode::Prototype),
                EditorSessionEvent::ModeChanged(EditorMode::Motion),
            ]
        );
    }

    #[gpui::test]
    fn workspace_switches_without_changing_the_canvas_mode(cx: &mut TestAppContext) {
        let session = cx.new(|_| EditorSession::with_mode(EditorMode::Motion));
        let events = Rc::new(RefCell::new(Vec::new()));
        let _subscription = cx.update(|cx| {
            let events = events.clone();
            cx.subscribe(&session, move |_, event, _| {
                events.borrow_mut().push(*event);
            })
        });

        session.update(cx, |session, cx| {
            session.set_workspace(EditorWorkspace::Variables, cx);
            session.set_workspace(EditorWorkspace::Variables, cx);
            session.set_workspace(EditorWorkspace::Canvas, cx);
        });

        assert_eq!(
            session.read_with(cx, |session, _| session.mode()),
            EditorMode::Motion
        );
        assert_eq!(
            events.borrow().as_slice(),
            [
                EditorSessionEvent::WorkspaceChanged(EditorWorkspace::Variables),
                EditorSessionEvent::WorkspaceChanged(EditorWorkspace::Canvas),
            ]
        );
    }
}
