use gpui::{Menu, MenuItem, OsAction};

pub fn app_menus() -> Vec<Menu> {
    use zed_actions::Quit;

    // Zoom here means the canvas, not the text size: this is a design app, and
    // these four match the shortcuts bound in the FigViewer keymap context.
    let view_items = vec![
        MenuItem::action("Zoom In", fig_viewer::ZoomIn),
        MenuItem::action("Zoom Out", fig_viewer::ZoomOut),
        MenuItem::action("Actual Size", fig_viewer::ResetZoom),
        MenuItem::action("Zoom to Fit", fig_viewer::FitToView),
        MenuItem::action("Zoom to Selection", fig_viewer::ZoomToSelection),
        MenuItem::separator(),
        MenuItem::action("Layers Sidebar", fig_viewer::ToggleLayersSidebar),
        MenuItem::action("Inspector Sidebar", fig_viewer::ToggleInspectorSidebar),
        MenuItem::separator(),
        MenuItem::action("Toggle Agent Panel", zed_actions::assistant::Toggle),
        MenuItem::action("Toggle Threads Rail", workspace::ToggleWorkspaceSidebar),
        MenuItem::separator(),
        MenuItem::action("Command Palette...", zed_actions::command_palette::Toggle),
    ];

    vec![
        Menu {
            name: "Fanta".into(),
            disabled: false,
            items: vec![
                MenuItem::action("About Fanta", zed_actions::About),
                MenuItem::separator(),
                MenuItem::submenu(Menu::new("Settings").items([
                    MenuItem::action("Open Settings File", super::OpenSettingsFile),
                    MenuItem::action("Open Keymap File", zed_actions::OpenKeymapFile),
                    MenuItem::separator(),
                    MenuItem::action(
                        "Select Theme...",
                        zed_actions::theme_selector::Toggle::default(),
                    ),
                ])),
                MenuItem::separator(),
                #[cfg(target_os = "macos")]
                MenuItem::os_submenu("Services", gpui::SystemMenuType::Services),
                #[cfg(target_os = "macos")]
                MenuItem::separator(),
                #[cfg(target_os = "macos")]
                MenuItem::action("Hide Fanta", super::Hide),
                #[cfg(target_os = "macos")]
                MenuItem::action("Hide Others", super::HideOthers),
                #[cfg(target_os = "macos")]
                MenuItem::action("Show All", super::ShowAll),
                #[cfg(target_os = "macos")]
                MenuItem::separator(),
                MenuItem::action("Quit Fanta", Quit),
            ],
        },
        Menu {
            name: "File".into(),
            disabled: false,
            items: vec![
                MenuItem::action("New Design...", zed_actions::fanta::NewDesign),
                MenuItem::action("New Window", workspace::NewWindow),
                MenuItem::separator(),
                #[cfg(not(target_os = "macos"))]
                MenuItem::action("Open File...", workspace::OpenFiles),
                MenuItem::action(
                    if cfg!(not(target_os = "macos")) {
                        "Open Folder..."
                    } else {
                        "Open…"
                    },
                    workspace::Open::default(),
                ),
                MenuItem::action("Open Recent…", zed_actions::OpenRecent::default()),
                MenuItem::separator(),
                MenuItem::action("Save", workspace::Save { save_intent: None }),
                MenuItem::action("Save As…", workspace::SaveAs),
                MenuItem::separator(),
                // Declared in git_ui's `actions!(git, [Diff, ..])`, not in the
                // git crate, and registered on every workspace by git_ui::init.
                MenuItem::action("Review Changes", git_ui::project_diff::Diff),
                MenuItem::action("Commit…", git::Commit),
                MenuItem::separator(),
                MenuItem::action(
                    "Close Tab",
                    workspace::CloseActiveItem {
                        save_intent: None,
                        close_pinned: true,
                    },
                ),
                MenuItem::action("Close Project", workspace::CloseProject),
                MenuItem::action("Close Window", workspace::CloseWindow),
            ],
        },
        Menu {
            name: "Edit".into(),
            disabled: false,
            items: vec![
                MenuItem::os_action("Undo", editor::actions::Undo, OsAction::Undo),
                MenuItem::os_action("Redo", editor::actions::Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action("Cut", editor::actions::Cut, OsAction::Cut),
                MenuItem::os_action("Copy", editor::actions::Copy, OsAction::Copy),
                MenuItem::os_action("Paste", editor::actions::Paste, OsAction::Paste),
                MenuItem::separator(),
                MenuItem::os_action(
                    "Select All",
                    editor::actions::SelectAll,
                    OsAction::SelectAll,
                ),
            ],
        },
        Menu {
            name: "View".into(),
            disabled: false,
            items: view_items,
        },
        Menu {
            name: "Window".into(),
            disabled: false,
            items: vec![
                MenuItem::action("Minimize", super::Minimize),
                MenuItem::action("Zoom", super::Zoom),
            ],
        },
        Menu {
            name: "Help".into(),
            disabled: false,
            items: vec![
                MenuItem::action("View Dependency Licenses", zed_actions::OpenLicenses),
                MenuItem::separator(),
                MenuItem::action(
                    "Documentation",
                    super::OpenBrowser {
                        url: "https://fantaisa.net/docs".into(),
                    },
                ),
                MenuItem::separator(),
                MenuItem::action(
                    "Connect Claude Code / Codex…",
                    zed_actions::fanta::ConnectExternalAgent,
                ),
            ],
        },
    ]
}
