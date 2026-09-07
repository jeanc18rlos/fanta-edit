use gpui::{
    Action, App, Global, KeyBinding, KeyContext, Keymap, Keystroke, KeystrokeEvent, Window,
};
use gpui_component::input::{
    Backspace, Copy, Cut, Delete, DeleteToBeginningOfLine, DeleteToEndOfLine, DeleteToNextWordEnd,
    DeleteToPreviousWordStart, Enter, Escape, MoveEnd, MoveHome, MoveLeft, MoveRight, MoveToEnd,
    MoveToNextWord, MoveToPreviousWord, MoveToStart, Paste, Redo, SelectAll, SelectToEnd,
    SelectToEndOfLine, SelectToNextWordEnd, SelectToPreviousWordStart, SelectToStart,
    SelectToStartOfLine, Undo,
};

use crate::{
    design::{CancelDesignInteraction, DESIGN_PANEL_KEY_CONTEXT},
    layers::{CloseLayersOverlay, LAYERS_PANEL_KEY_CONTEXT},
    pages::{ClosePagesSearch, PAGES_PANEL_KEY_CONTEXT},
    toolbar::{
        CloseToolbarOverlay, NextToolbarCommand, PreviousToolbarCommand,
        TOOLBAR_TEXT_ENTRY_KEY_CONTEXT,
    },
};

const INPUT_KEY_CONTEXT: &str = "Input";

struct TextInputFallbackInstalled;

impl Global for TextInputFallbackInstalled {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InputAction {
    Backspace,
    Delete,
    DeleteToBeginningOfLine,
    DeleteToEndOfLine,
    DeleteToPreviousWordStart,
    DeleteToNextWordEnd,
    MoveLeft,
    MoveRight,
    MoveHome,
    MoveEnd,
    MoveToPreviousWord,
    MoveToNextWord,
    SelectLeft,
    SelectRight,
    SelectAll,
    SelectToStartOfLine,
    SelectToEndOfLine,
    SelectToPreviousWordStart,
    SelectToNextWordEnd,
    MoveToStart,
    MoveToEnd,
    SelectToStart,
    SelectToEnd,
    Enter(bool),
    Escape,
    Copy,
    Cut,
    Paste,
    Undo,
    Redo,
    PreviousToolbarCommand,
    NextToolbarCommand,
    CloseToolbarOverlay,
    ClosePagesSearch,
    CancelDesignInteraction,
    CloseLayersOverlay,
}

impl InputAction {
    fn boxed(self, cx: &App) -> Box<dyn Action> {
        match self {
            Self::Backspace => Box::new(Backspace),
            Self::Delete => Box::new(Delete),
            Self::DeleteToBeginningOfLine => Box::new(DeleteToBeginningOfLine),
            Self::DeleteToEndOfLine => Box::new(DeleteToEndOfLine),
            Self::DeleteToPreviousWordStart => Box::new(DeleteToPreviousWordStart),
            Self::DeleteToNextWordEnd => Box::new(DeleteToNextWordEnd),
            Self::MoveLeft => Box::new(MoveLeft),
            Self::MoveRight => Box::new(MoveRight),
            Self::MoveHome => Box::new(MoveHome),
            Self::MoveEnd => Box::new(MoveEnd),
            Self::MoveToPreviousWord => Box::new(MoveToPreviousWord),
            Self::MoveToNextWord => Box::new(MoveToNextWord),
            // gpui-component keeps these shared UI actions crate-private, but
            // registers them for dynamic construction like its other actions.
            // Fail loudly if that contract changes instead of silently losing
            // selection deletion again.
            Self::SelectLeft => cx
                .build_action("ui::SelectLeft", None)
                .expect("gpui-component must register ui::SelectLeft"),
            Self::SelectRight => cx
                .build_action("ui::SelectRight", None)
                .expect("gpui-component must register ui::SelectRight"),
            Self::SelectAll => Box::new(SelectAll),
            Self::SelectToStartOfLine => Box::new(SelectToStartOfLine),
            Self::SelectToEndOfLine => Box::new(SelectToEndOfLine),
            Self::SelectToPreviousWordStart => Box::new(SelectToPreviousWordStart),
            Self::SelectToNextWordEnd => Box::new(SelectToNextWordEnd),
            Self::MoveToStart => Box::new(MoveToStart),
            Self::MoveToEnd => Box::new(MoveToEnd),
            Self::SelectToStart => Box::new(SelectToStart),
            Self::SelectToEnd => Box::new(SelectToEnd),
            Self::Enter(secondary) => Box::new(Enter { secondary }),
            Self::Escape => Box::new(Escape),
            Self::Copy => Box::new(Copy),
            Self::Cut => Box::new(Cut),
            Self::Paste => Box::new(Paste),
            Self::Undo => Box::new(Undo),
            Self::Redo => Box::new(Redo),
            Self::PreviousToolbarCommand => Box::new(PreviousToolbarCommand),
            Self::NextToolbarCommand => Box::new(NextToolbarCommand),
            Self::CloseToolbarOverlay => Box::new(CloseToolbarOverlay),
            Self::ClosePagesSearch => Box::new(ClosePagesSearch),
            Self::CancelDesignInteraction => Box::new(CancelDesignInteraction),
            Self::CloseLayersOverlay => Box::new(CloseLayersOverlay),
        }
    }
}

pub(crate) fn init(cx: &mut App) {
    if cx.has_global::<TextInputFallbackInstalled>() {
        return;
    }
    cx.set_global(TextInputFallbackInstalled);

    cx.intercept_keystrokes(|event, window, cx| {
        if !is_fanta_text_input(event) {
            return;
        }
        let actions = fallback_actions(event);
        if actions.is_empty() {
            return;
        }
        let actions = actions
            .into_iter()
            .map(|action| action.boxed(cx))
            .collect::<Vec<_>>();
        if action_binding_is_available(event, &actions, window, cx) {
            return;
        }

        clear_pending_input(window, cx);
        for action in actions {
            window.dispatch_action(action, cx);
        }
        cx.stop_propagation();
    })
    .detach();
}

fn is_fanta_text_input(event: &KeystrokeEvent) -> bool {
    let has_input = event
        .context_stack
        .iter()
        .any(|context| context.contains(INPUT_KEY_CONTEXT));
    let has_fanta_surface = event.context_stack.iter().any(|context| {
        context.contains(TOOLBAR_TEXT_ENTRY_KEY_CONTEXT)
            || context.contains(PAGES_PANEL_KEY_CONTEXT)
            || context.contains(DESIGN_PANEL_KEY_CONTEXT)
            || context.contains(LAYERS_PANEL_KEY_CONTEXT)
    });
    has_input && has_fanta_surface
}

fn action_binding_is_available(
    event: &KeystrokeEvent,
    actions: &[Box<dyn Action>],
    window: &Window,
    cx: &App,
) -> bool {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let input_depth = event
        .context_stack
        .iter()
        .enumerate()
        .filter(|(_, context)| context.contains(INPUT_KEY_CONTEXT))
        .map(|(index, _)| index + 1)
        .max()
        .unwrap_or(usize::MAX);

    if let Some(pending) = window.pending_input_keystrokes() {
        let mut combined = pending.to_vec();
        combined.push(event.keystroke.clone());
        match binding_resolution(
            &keymap,
            &combined,
            &event.context_stack,
            input_depth,
            actions,
        ) {
            BindingResolution::Available => return true,
            BindingResolution::Blocked => return false,
            BindingResolution::Missing => {}
        }
    }

    matches!(
        binding_resolution(
            &keymap,
            std::slice::from_ref(&event.keystroke),
            &event.context_stack,
            input_depth,
            actions,
        ),
        BindingResolution::Available
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BindingResolution {
    Available,
    Blocked,
    Missing,
}

fn binding_resolution(
    keymap: &Keymap,
    input: &[Keystroke],
    context_stack: &[KeyContext],
    input_depth: usize,
    actions: &[Box<dyn Action>],
) -> BindingResolution {
    let (bindings, pending) = keymap.bindings_for_input(input, context_stack);
    if pending {
        let pending_binding = keymap
            .possible_next_bindings_for_input(input, context_stack)
            .into_iter()
            .next();
        return if pending_binding
            .as_ref()
            .is_some_and(|binding| binding_targets_input(binding, context_stack, input_depth))
        {
            BindingResolution::Available
        } else {
            BindingResolution::Blocked
        };
    }

    let Some(binding) = bindings.first() else {
        return BindingResolution::Missing;
    };
    if actions
        .iter()
        .any(|action| binding.action().partial_eq(action.as_ref()))
        || binding_targets_input(binding, context_stack, input_depth)
    {
        BindingResolution::Available
    } else {
        BindingResolution::Blocked
    }
}

fn binding_targets_input(
    binding: &KeyBinding,
    context_stack: &[KeyContext],
    input_depth: usize,
) -> bool {
    binding
        .predicate()
        .and_then(|predicate| predicate.depth_of(context_stack))
        .is_some_and(|depth| depth >= input_depth)
}

fn clear_pending_input(window: &mut Window, cx: &mut App) {
    if !window.has_pending_keystrokes() {
        return;
    }

    // GPUI intentionally exposes pending input read-only. Moving focus away
    // and back synchronously is its public way to discard a chord without a
    // frame in between, so the focused Input and its selection stay intact.
    if let Some(focus) = window.focused(cx) {
        window.blur();
        window.focus(&focus, cx);
    }
}

fn fallback_actions(event: &KeystrokeEvent) -> Vec<InputAction> {
    let mut actions = Vec::with_capacity(2);
    if let Some(action) = input_action(&event.keystroke) {
        actions.push(action);
    }

    let unmodified = !event.keystroke.modifiers.modified();
    let in_toolbar = event
        .context_stack
        .iter()
        .any(|context| context.contains(TOOLBAR_TEXT_ENTRY_KEY_CONTEXT));
    let in_pages = event
        .context_stack
        .iter()
        .any(|context| context.contains(PAGES_PANEL_KEY_CONTEXT));
    let in_design = event
        .context_stack
        .iter()
        .any(|context| context.contains(DESIGN_PANEL_KEY_CONTEXT));
    let in_layers = event
        .context_stack
        .iter()
        .any(|context| context.contains(LAYERS_PANEL_KEY_CONTEXT));

    if in_toolbar && unmodified {
        match event.keystroke.key.as_str() {
            "up" => actions.push(InputAction::PreviousToolbarCommand),
            "down" => actions.push(InputAction::NextToolbarCommand),
            "escape" => actions.push(InputAction::CloseToolbarOverlay),
            _ => {}
        }
    } else if in_pages && unmodified && event.keystroke.key == "escape" {
        actions.push(InputAction::ClosePagesSearch);
    } else if in_design && unmodified && event.keystroke.key == "escape" {
        actions.push(InputAction::CancelDesignInteraction);
    } else if in_layers && unmodified && event.keystroke.key == "escape" {
        actions.push(InputAction::CloseLayersOverlay);
    }

    actions
}

fn input_action(keystroke: &Keystroke) -> Option<InputAction> {
    let modifiers = keystroke.modifiers;
    if modifiers.function {
        return None;
    }

    if !modifiers.modified() {
        return match keystroke.key.as_str() {
            "backspace" => Some(InputAction::Backspace),
            "delete" => Some(InputAction::Delete),
            "enter" => Some(InputAction::Enter(false)),
            "escape" => Some(InputAction::Escape),
            "left" => Some(InputAction::MoveLeft),
            "right" => Some(InputAction::MoveRight),
            "home" => Some(InputAction::MoveHome),
            "end" => Some(InputAction::MoveEnd),
            _ => None,
        };
    }

    if modifiers.shift && !modifiers.control && !modifiers.alt && !modifiers.platform {
        return match keystroke.key.as_str() {
            "left" => Some(InputAction::SelectLeft),
            "right" => Some(InputAction::SelectRight),
            "home" => Some(InputAction::SelectToStartOfLine),
            "end" => Some(InputAction::SelectToEndOfLine),
            _ => None,
        };
    }

    #[cfg(target_os = "macos")]
    {
        if modifiers.platform && !modifiers.control && !modifiers.alt && !modifiers.shift {
            return match keystroke.key.as_str() {
                "backspace" => Some(InputAction::DeleteToBeginningOfLine),
                "delete" => Some(InputAction::DeleteToEndOfLine),
                "left" => Some(InputAction::MoveHome),
                "right" => Some(InputAction::MoveEnd),
                "up" => Some(InputAction::MoveToStart),
                "down" => Some(InputAction::MoveToEnd),
                "a" => Some(InputAction::SelectAll),
                "c" => Some(InputAction::Copy),
                "x" => Some(InputAction::Cut),
                "v" => Some(InputAction::Paste),
                "z" => Some(InputAction::Undo),
                "enter" => Some(InputAction::Enter(true)),
                _ => None,
            };
        }
        if modifiers.alt && !modifiers.control && !modifiers.platform && !modifiers.shift {
            return match keystroke.key.as_str() {
                "backspace" => Some(InputAction::DeleteToPreviousWordStart),
                "delete" => Some(InputAction::DeleteToNextWordEnd),
                "left" => Some(InputAction::MoveToPreviousWord),
                "right" => Some(InputAction::MoveToNextWord),
                _ => None,
            };
        }
        if modifiers.platform && modifiers.shift && !modifiers.control && !modifiers.alt {
            return match keystroke.key.as_str() {
                "left" => Some(InputAction::SelectToStartOfLine),
                "right" => Some(InputAction::SelectToEndOfLine),
                "up" => Some(InputAction::SelectToStart),
                "down" => Some(InputAction::SelectToEnd),
                "z" => Some(InputAction::Redo),
                _ => None,
            };
        }
        if modifiers.alt && modifiers.shift && !modifiers.control && !modifiers.platform {
            return match keystroke.key.as_str() {
                "left" => Some(InputAction::SelectToPreviousWordStart),
                "right" => Some(InputAction::SelectToNextWordEnd),
                _ => None,
            };
        }
        if modifiers.control && !modifiers.alt && !modifiers.platform {
            return match (modifiers.shift, keystroke.key.as_str()) {
                (false, "a") => Some(InputAction::MoveHome),
                (false, "e") => Some(InputAction::MoveEnd),
                (true, "a") => Some(InputAction::SelectToStartOfLine),
                (true, "e") => Some(InputAction::SelectToEndOfLine),
                _ => None,
            };
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        if modifiers.control && !modifiers.alt && !modifiers.platform && !modifiers.shift {
            return match keystroke.key.as_str() {
                "backspace" => Some(InputAction::DeleteToPreviousWordStart),
                "delete" => Some(InputAction::DeleteToNextWordEnd),
                "left" => Some(InputAction::MoveToPreviousWord),
                "right" => Some(InputAction::MoveToNextWord),
                "a" => Some(InputAction::SelectAll),
                "c" => Some(InputAction::Copy),
                "x" => Some(InputAction::Cut),
                "v" => Some(InputAction::Paste),
                "z" => Some(InputAction::Undo),
                "y" => Some(InputAction::Redo),
                "enter" => Some(InputAction::Enter(true)),
                _ => None,
            };
        }
        if modifiers.control && modifiers.shift && !modifiers.alt && !modifiers.platform {
            return match keystroke.key.as_str() {
                "left" => Some(InputAction::SelectToPreviousWordStart),
                "right" => Some(InputAction::SelectToNextWordEnd),
                _ => None,
            };
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(source: &str) -> Option<InputAction> {
        input_action(&Keystroke::parse(source).expect("valid test keystroke"))
    }

    #[test]
    fn maps_unmodified_deletion_keys_only() {
        assert_eq!(action("backspace"), Some(InputAction::Backspace));
        assert_eq!(action("delete"), Some(InputAction::Delete));
        assert_eq!(action("shift-backspace"), None);
        assert_eq!(action("a"), None);
    }

    #[test]
    fn maps_basic_navigation_and_selection_keys() {
        assert_eq!(action("left"), Some(InputAction::MoveLeft));
        assert_eq!(action("right"), Some(InputAction::MoveRight));
        assert_eq!(action("home"), Some(InputAction::MoveHome));
        assert_eq!(action("end"), Some(InputAction::MoveEnd));
        assert_eq!(action("shift-left"), Some(InputAction::SelectLeft));
        assert_eq!(action("shift-right"), Some(InputAction::SelectRight));
        assert_eq!(action("shift-home"), Some(InputAction::SelectToStartOfLine));
        assert_eq!(action("shift-end"), Some(InputAction::SelectToEndOfLine));
        assert_eq!(action("enter"), Some(InputAction::Enter(false)));
        assert_eq!(action("escape"), Some(InputAction::Escape));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn maps_macos_word_and_line_deletion_keys() {
        assert_eq!(
            action("cmd-backspace"),
            Some(InputAction::DeleteToBeginningOfLine)
        );
        assert_eq!(action("cmd-delete"), Some(InputAction::DeleteToEndOfLine));
        assert_eq!(
            action("alt-backspace"),
            Some(InputAction::DeleteToPreviousWordStart)
        );
        assert_eq!(action("alt-delete"), Some(InputAction::DeleteToNextWordEnd));
        assert_eq!(action("cmd-left"), Some(InputAction::MoveHome));
        assert_eq!(action("cmd-right"), Some(InputAction::MoveEnd));
        assert_eq!(action("cmd-a"), Some(InputAction::SelectAll));
        assert_eq!(action("cmd-c"), Some(InputAction::Copy));
        assert_eq!(action("cmd-x"), Some(InputAction::Cut));
        assert_eq!(action("cmd-v"), Some(InputAction::Paste));
        assert_eq!(action("cmd-z"), Some(InputAction::Undo));
        assert_eq!(action("cmd-shift-z"), Some(InputAction::Redo));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn maps_non_macos_word_deletion_keys() {
        assert_eq!(
            action("ctrl-backspace"),
            Some(InputAction::DeleteToPreviousWordStart)
        );
        assert_eq!(
            action("ctrl-delete"),
            Some(InputAction::DeleteToNextWordEnd)
        );
        assert_eq!(action("ctrl-left"), Some(InputAction::MoveToPreviousWord));
        assert_eq!(action("ctrl-right"), Some(InputAction::MoveToNextWord));
        assert_eq!(action("ctrl-a"), Some(InputAction::SelectAll));
        assert_eq!(action("ctrl-c"), Some(InputAction::Copy));
        assert_eq!(action("ctrl-x"), Some(InputAction::Cut));
        assert_eq!(action("ctrl-v"), Some(InputAction::Paste));
        assert_eq!(action("ctrl-z"), Some(InputAction::Undo));
        assert_eq!(action("ctrl-y"), Some(InputAction::Redo));
    }
}
