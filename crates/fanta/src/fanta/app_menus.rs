use gpui::{Menu, MenuItem, OsAction};

pub fn app_menus() -> Vec<Menu> {
    use fanta_actions::Quit;

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
        MenuItem::action("File Tree", fanta_actions::project_panel::ToggleFocus),
        MenuItem::action("Git Changes", git_ui::git_panel::ToggleFocus),
        MenuItem::separator(),
        MenuItem::action("Toggle Agent Panel", fanta_actions::assistant::Toggle),
        MenuItem::action("Toggle Threads Rail", workspace::ToggleWorkspaceSidebar),
        MenuItem::separator(),
        MenuItem::action("Command Palette...", fanta_actions::command_palette::Toggle),
    ];

    vec![
        Menu {
            name: "Fanta".into(),
            disabled: false,
            items: vec![
                MenuItem::action("About Fanta", fanta_actions::About),
                MenuItem::action("Check for Updates…", auto_update::Check),
                #[cfg(feature = "mac_app_store")]
                MenuItem::action("Credits & Billing…", fanta_actions::OpenAccountSettings),
                MenuItem::separator(),
                MenuItem::submenu(Menu::new("Settings").items([
                    MenuItem::action("Open Settings…", fanta_actions::OpenSettings),
                    #[cfg(feature = "mac_app_store")]
                    MenuItem::action("App Access…", workspace::welcome::OpenAppAccess),
                    MenuItem::separator(),
                    MenuItem::action("Open Settings File", super::OpenSettingsFile),
                    MenuItem::action("Open Keymap File", fanta_actions::OpenKeymapFile),
                    MenuItem::separator(),
                    MenuItem::action(
                        "Select Theme...",
                        fanta_actions::theme_selector::Toggle::default(),
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
                MenuItem::action("New Design...", fanta_actions::fanta::NewDesign),
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
                MenuItem::action("Open Recent…", fanta_actions::OpenRecent::default()),
                MenuItem::action(
                    "Show Project in File Manager",
                    fanta_actions::fanta::RevealProjectInFileManager,
                ),
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
            name: "Design".into(),
            disabled: false,
            items: vec![
                MenuItem::action("AI Review…", fig_viewer::RunAiReview),
                MenuItem::action("Ultrareview…", fig_viewer::RunUltraReview),
                MenuItem::separator(),
                MenuItem::action("Export Motion JSON…", fig_viewer::ExportMotionJson),
                MenuItem::action(
                    "Export Figma Motion JSON…",
                    fig_viewer::ExportFigmaMotionJson,
                ),
                MenuItem::action("Export Claude Motion Stage…", fig_viewer::ExportMotionStage),
                MenuItem::action("Export MP4 Video…", fig_viewer::ExportMotionVideo),
                MenuItem::action("Export GIF…", fig_viewer::ExportMotionGif),
                MenuItem::action("Export PNG Frames…", fig_viewer::ExportMotionFrames),
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
                MenuItem::action("View Dependency Licenses", fanta_actions::OpenLicenses),
                MenuItem::separator(),
                MenuItem::action(
                    "Documentation",
                    super::OpenBrowser {
                        url: "https://fantaisa.net/docs".into(),
                    },
                ),
                #[cfg(not(feature = "mac_app_store"))]
                MenuItem::separator(),
                #[cfg(not(feature = "mac_app_store"))]
                MenuItem::action(
                    "Connect Claude Code / Codex…",
                    fanta_actions::fanta::ConnectExternalAgent,
                ),
            ],
        },
    ]
}
