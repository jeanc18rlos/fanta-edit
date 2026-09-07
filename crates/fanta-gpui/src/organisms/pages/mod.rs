//! Pages navigation components.

mod commands;
mod model;
mod panel;

pub use commands::{
    AddPage, ClosePagesSearch, ConfirmPagesTextEntry, FindInPages, FocusFirstPage, FocusLastPage,
    FocusNextPage, FocusPreviousPage, NextSearchResult, OpenPageContextMenu, PreviousSearchResult,
    ReplaceAllResults, ReplaceCurrentResult, TogglePagesPanel, ToggleSearchSettings,
};
pub use model::{
    PagesPanelAction, PagesPanelElementCount, PagesPanelElementKind, PagesPanelItem,
    PagesPanelResultDirection, PagesPanelSearchRequest, PagesPanelSearchResult,
    PagesPanelSearchResults, PagesPanelSearchScope,
};
pub use panel::{PAGES_PANEL_MIN_HEIGHT, PAGES_PANEL_MIN_WIDTH, PagesPanel};

pub(crate) use commands::{PAGES_PANEL_KEY_CONTEXT, PAGES_TEXT_ENTRY_KEY_CONTEXT};

pub(crate) fn init(cx: &mut gpui::App) {
    commands::init(cx);
}
