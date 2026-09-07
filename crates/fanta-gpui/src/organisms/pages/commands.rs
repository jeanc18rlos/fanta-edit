use gpui::{App, KeyBinding, actions};

actions!(
    fanta_pages,
    [
        TogglePagesPanel,
        FindInPages,
        AddPage,
        ToggleSearchSettings,
        ClosePagesSearch,
        ConfirmPagesTextEntry,
        PreviousSearchResult,
        NextSearchResult,
        ReplaceCurrentResult,
        ReplaceAllResults,
        OpenPageContextMenu,
        FocusPreviousPage,
        FocusNextPage,
        FocusFirstPage,
        FocusLastPage
    ]
);

pub(crate) const PAGES_PANEL_KEY_CONTEXT: &str = "FantaPagesPanel";
pub(crate) const PAGES_TEXT_ENTRY_KEY_CONTEXT: &str = "FantaPagesPanelTextEntry";

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-f", FindInPages, Some(PAGES_PANEL_KEY_CONTEXT)),
        KeyBinding::new(
            "shift-secondary-d",
            PreviousSearchResult,
            Some(PAGES_PANEL_KEY_CONTEXT),
        ),
        KeyBinding::new(
            "shift-secondary-f",
            NextSearchResult,
            Some(PAGES_PANEL_KEY_CONTEXT),
        ),
        KeyBinding::new(
            "ctrl-enter",
            OpenPageContextMenu,
            Some(PAGES_PANEL_KEY_CONTEXT),
        ),
        KeyBinding::new("escape", ClosePagesSearch, Some(PAGES_PANEL_KEY_CONTEXT)),
        // The focused single-line Input handles Enter by emitting PressEnter
        // and then propagating the action; this binding consumes the
        // propagated keystroke so the unhandled fall-through cannot type the
        // keystroke's "\n" key_char into the single-line field.
        KeyBinding::new(
            "enter",
            ConfirmPagesTextEntry,
            Some(PAGES_TEXT_ENTRY_KEY_CONTEXT),
        ),
        KeyBinding::new("up", FocusPreviousPage, Some(PAGES_PANEL_KEY_CONTEXT)),
        KeyBinding::new("down", FocusNextPage, Some(PAGES_PANEL_KEY_CONTEXT)),
        KeyBinding::new("home", FocusFirstPage, Some(PAGES_PANEL_KEY_CONTEXT)),
        KeyBinding::new("end", FocusLastPage, Some(PAGES_PANEL_KEY_CONTEXT)),
    ]);
}
