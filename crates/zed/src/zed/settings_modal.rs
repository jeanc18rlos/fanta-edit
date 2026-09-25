use fs::Fs;
use gpui::{
    Action, App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, Subscription,
    WeakEntity,
};
use settings::{
    AutosaveSetting, RelativeLineNumbers, Settings, SettingsStore, update_settings_file,
};
use ui::prelude::*;
use workspace::{ModalView, Workspace, WorkspaceSettings};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SettingsPage {
    General,
    Appearance,
    Editor,
    Panels,
    Account,
}

impl SettingsPage {
    const ALL: [Self; 5] = [
        Self::General,
        Self::Appearance,
        Self::Editor,
        Self::Panels,
        Self::Account,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Editor => "Code Editor",
            Self::Panels => "Panels",
            Self::Account => "AI & Billing",
        }
    }

    pub(super) fn from_path(path: &str) -> Self {
        let path = path.to_ascii_lowercase();
        if path.contains("theme") || path.contains("appearance") || path.contains("font") {
            Self::Appearance
        } else if path.contains("editor") || path.contains("language") || path.contains("keymap") {
            Self::Editor
        } else if path.contains("panel") || path.contains("dock") || path.contains("sidebar") {
            Self::Panels
        } else if path.contains("agent")
            || path.contains("ai")
            || path.contains("account")
            || path.contains("billing")
        {
            Self::Account
        } else {
            Self::General
        }
    }
}

#[derive(Clone, Copy)]
enum SettingToggle {
    Autosave,
    ConfirmQuit,
    SystemPathPrompts,
    CursorBlink,
    RelativeLineNumbers,
}

#[derive(Clone, Copy)]
enum DesignPanelAction {
    Layers,
    Inspector,
}

pub(super) struct SettingsModal {
    focus_handle: FocusHandle,
    page: SettingsPage,
    workspace: WeakEntity<Workspace>,
    _settings_subscription: Subscription,
}

impl SettingsModal {
    pub(super) fn new(
        page: SettingsPage,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            page,
            workspace,
            _settings_subscription: cx.observe_global::<SettingsStore>(|_, cx| cx.notify()),
        }
    }

    pub(super) fn select_page(&mut self, page: SettingsPage, cx: &mut Context<Self>) {
        self.page = page;
        cx.notify();
    }

    pub(super) fn open(
        workspace: &mut Workspace,
        page: SettingsPage,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        if let Some(modal) = workspace.active_modal::<Self>(cx) {
            modal.update(cx, |modal, cx| modal.select_page(page, cx));
        } else {
            let workspace_handle = cx.entity().downgrade();
            workspace.toggle_modal(window, cx, move |_, cx| {
                Self::new(page, workspace_handle, cx)
            });
        }
    }

    fn setting_row(
        title: &'static str,
        description: &'static str,
        control: impl IntoElement,
    ) -> impl IntoElement {
        h_flex()
            .w_full()
            .gap_4()
            .justify_between()
            .py_3()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(Label::new(title))
                    .child(
                        Label::new(description)
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            )
            .child(control)
    }

    fn action_row<A: Action + Clone + 'static>(
        title: &'static str,
        description: &'static str,
        button_label: &'static str,
        action: A,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        Self::setting_row(
            title,
            description,
            Button::new(title, button_label).on_click(cx.listener(move |_, _, window, cx| {
                cx.emit(DismissEvent);
                let action = action.clone();
                window.defer(cx, move |window, cx| {
                    window.dispatch_action(Box::new(action), cx);
                });
            })),
        )
    }

    fn design_action_row(
        &self,
        title: &'static str,
        description: &'static str,
        action: DesignPanelAction,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let workspace = self.workspace.clone();
        Self::setting_row(
            title,
            description,
            Button::new(title, "Toggle").on_click(cx.listener(move |_, _, window, cx| {
                cx.emit(DismissEvent);
                let workspace = workspace.clone();
                let action = action.clone();
                window.defer(cx, move |window, cx| {
                    let design_tab = workspace.read_with(cx, |workspace, cx| {
                        let preferred_view = workspace
                            .active_item_as::<fig_viewer::FigView>(cx)
                            .or_else(|| {
                                workspace.recent_active_item_by_type::<fig_viewer::FigView>(cx)
                            })
                            .map(|view| view.entity_id());
                        let design_tabs: Vec<_> = workspace
                            .panes()
                            .iter()
                            .flat_map(|pane| {
                                pane.read(cx)
                                    .items()
                                    .enumerate()
                                    .filter_map(|(index, item)| {
                                        item.downcast::<fig_viewer::FigView>()
                                            .map(|view| (pane.clone(), index, view))
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .collect();
                        design_tabs
                            .iter()
                            .find(|(_, _, view)| Some(view.entity_id()) == preferred_view)
                            .cloned()
                            .or_else(|| design_tabs.into_iter().next())
                    });
                    if let Ok(Some((pane, index, view))) = design_tab {
                        pane.update(cx, |pane, cx| {
                            pane.activate_item(index, true, true, window, cx)
                        });
                        view.update(cx, |view, cx| match action {
                            DesignPanelAction::Layers => view.toggle_layers_sidebar(
                                &fig_viewer::ToggleLayersSidebar,
                                window,
                                cx,
                            ),
                            DesignPanelAction::Inspector => view.toggle_inspector_sidebar(
                                &fig_viewer::ToggleInspectorSidebar,
                                window,
                                cx,
                            ),
                        });
                    }
                });
            })),
        )
    }

    fn toggle_row(
        title: &'static str,
        description: &'static str,
        current: bool,
        setting: SettingToggle,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        Self::setting_row(
            title,
            description,
            Button::new(title, if current { "On" } else { "Off" })
                .toggle_state(current)
                .on_click(cx.listener(move |_, _, _, cx| {
                    let fs = <dyn Fs>::global(cx);
                    update_settings_file(fs, cx, move |settings, _| match setting {
                        SettingToggle::Autosave => {
                            settings.workspace.autosave = Some(if current {
                                AutosaveSetting::Off
                            } else {
                                AutosaveSetting::OnFocusChange
                            });
                        }
                        SettingToggle::ConfirmQuit => {
                            settings.workspace.confirm_quit = Some(!current);
                        }
                        SettingToggle::SystemPathPrompts => {
                            settings.workspace.use_system_path_prompts = Some(!current);
                        }
                        SettingToggle::CursorBlink => {
                            settings.editor.cursor_blink = Some(!current);
                        }
                        SettingToggle::RelativeLineNumbers => {
                            settings.editor.relative_line_numbers = Some(if current {
                                RelativeLineNumbers::Disabled
                            } else {
                                RelativeLineNumbers::Enabled
                            });
                        }
                    });
                })),
        )
    }

    fn page_contents(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut contents = v_flex().w_full().gap_1();
        match self.page {
            SettingsPage::General => {
                let settings = WorkspaceSettings::get_global(cx);
                let autosave = !matches!(settings.autosave, AutosaveSetting::Off);
                let confirm_quit = settings.confirm_quit;
                let system_path_prompts = settings.use_system_path_prompts;
                contents = contents
                    .child(Self::toggle_row(
                        "Autosave",
                        "Save edited files when focus moves away.",
                        autosave,
                        SettingToggle::Autosave,
                        cx,
                    ))
                    .child(Self::toggle_row(
                        "Confirm before quitting",
                        "Ask before closing Fanta.",
                        confirm_quit,
                        SettingToggle::ConfirmQuit,
                        cx,
                    ))
                    .child(Self::toggle_row(
                        "Native file dialogs",
                        "Use macOS Open and Save dialogs.",
                        system_path_prompts,
                        SettingToggle::SystemPathPrompts,
                        cx,
                    ))
                    .child(Self::action_row(
                        "Advanced settings",
                        "Edit settings.json for options not shown here.",
                        "Open File",
                        zed_actions::OpenSettingsFile,
                        cx,
                    ));
            }
            SettingsPage::Appearance => {
                contents = contents
                    .child(Self::action_row(
                        "Theme",
                        "Choose a light or dark canvas and app theme.",
                        "Choose…",
                        zed_actions::theme_selector::Toggle::default(),
                        cx,
                    ))
                    .child(Self::action_row(
                        "Icon theme",
                        "Choose the icons used in panels and source files.",
                        "Choose…",
                        zed_actions::icon_theme_selector::Toggle::default(),
                        cx,
                    ))
                    .child(Self::action_row(
                        "Larger interface text",
                        "Increase text size across the app.",
                        "Increase",
                        zed_actions::IncreaseUiFontSize { persist: true },
                        cx,
                    ))
                    .child(Self::action_row(
                        "Smaller interface text",
                        "Decrease text size across the app.",
                        "Decrease",
                        zed_actions::DecreaseUiFontSize { persist: true },
                        cx,
                    ))
                    .child(Self::action_row(
                        "Reset interface text",
                        "Return to the default interface text size.",
                        "Reset",
                        zed_actions::ResetUiFontSize { persist: true },
                        cx,
                    ));
            }
            SettingsPage::Editor => {
                let settings = editor::EditorSettings::get_global(cx);
                let cursor_blink = settings.cursor_blink;
                let relative_line_numbers =
                    settings.relative_line_numbers != RelativeLineNumbers::Disabled;
                contents = contents
                    .child(Self::toggle_row(
                        "Blinking cursor",
                        "Blink the caret in the FNX source editor.",
                        cursor_blink,
                        SettingToggle::CursorBlink,
                        cx,
                    ))
                    .child(Self::toggle_row(
                        "Relative line numbers",
                        "Show distances from the current line in source files.",
                        relative_line_numbers,
                        SettingToggle::RelativeLineNumbers,
                        cx,
                    ))
                    .child(Self::action_row(
                        "Larger source text",
                        "Increase the font size in the FNX source editor.",
                        "Increase",
                        zed_actions::IncreaseBufferFontSize { persist: true },
                        cx,
                    ))
                    .child(Self::action_row(
                        "Smaller source text",
                        "Decrease the font size in the FNX source editor.",
                        "Decrease",
                        zed_actions::DecreaseBufferFontSize { persist: true },
                        cx,
                    ))
                    .child(Self::action_row(
                        "Key bindings",
                        "Edit keyboard shortcuts in keymap.json.",
                        "Open File",
                        zed_actions::OpenKeymapFile,
                        cx,
                    ));
            }
            SettingsPage::Panels => {
                contents = contents
                    .child(self.design_action_row(
                        "Layers",
                        "Show or hide pages and layers beside the canvas.",
                        DesignPanelAction::Layers,
                        cx,
                    ))
                    .child(self.design_action_row(
                        "Inspector",
                        "Show or hide design properties beside the canvas.",
                        DesignPanelAction::Inspector,
                        cx,
                    ))
                    .child(Self::action_row(
                        "Files",
                        "Browse the project files and FNX source.",
                        "Toggle",
                        zed_actions::project_panel::Toggle,
                        cx,
                    ))
                    .child(Self::action_row(
                        "Git",
                        "Inspect and commit changes to the design project.",
                        "Toggle",
                        git_ui::git_panel::Toggle,
                        cx,
                    ))
                    .child(Self::action_row(
                        "Threads",
                        "Show the thread rail for design work.",
                        "Toggle",
                        workspace::ToggleWorkspaceSidebar,
                        cx,
                    ));
            }
            SettingsPage::Account => {
                contents = contents.child(Self::action_row(
                    "Agent",
                    "Open the agent panel for canvas edits and design generation.",
                    "Open Panel",
                    zed_actions::assistant::Toggle,
                    cx,
                ));
                #[cfg(feature = "mac_app_store")]
                {
                    contents = contents.child(Self::action_row(
                        "Credits & billing",
                        "Manage Fanta credits and restore Apple purchases.",
                        "Open…",
                        zed_actions::OpenAccountSettings,
                        cx,
                    ));
                }
                #[cfg(not(feature = "mac_app_store"))]
                {
                    contents = contents.child(Self::action_row(
                        "Account",
                        "Open Fanta account settings in your browser.",
                        "Open…",
                        zed_actions::OpenAccountSettings,
                        cx,
                    ));
                }
                contents = contents.child(Self::action_row(
                    "Advanced AI settings",
                    "Configure agent, MCP and model settings in settings.json.",
                    "Open File",
                    zed_actions::OpenSettingsFile,
                    cx,
                ));
            }
        }
        contents
    }
}

impl Focusable for SettingsModal {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<DismissEvent> for SettingsModal {}
impl ModalView for SettingsModal {
    fn fade_out_background(&self) -> bool {
        true
    }
}

impl Render for SettingsModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let border_color = cx.theme().colors().border_variant;
        v_flex()
            .id("fanta-settings")
            .key_context("FantaSettings")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|_, _: &menu::Cancel, _, cx| cx.emit(DismissEvent)))
            .elevation_3(cx)
            .w(rems(58.))
            .h(rems(38.))
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(border_color)
                    .child(Label::new("Fanta Settings").size(LabelSize::Large))
                    .child(
                        Button::new("close-settings", "Close")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .flex_1()
                    .overflow_hidden()
                    .items_start()
                    .child(
                        v_flex()
                            .w(rems(14.))
                            .h_full()
                            .p_3()
                            .gap_1()
                            .border_r_1()
                            .border_color(border_color)
                            .children(SettingsPage::ALL.into_iter().map(|page| {
                                Button::new(page.label(), page.label())
                                    .full_width()
                                    .toggle_state(self.page == page)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.select_page(page, cx);
                                    }))
                            })),
                    )
                    .child(
                        v_flex()
                            .id("fanta-settings-content")
                            .flex_1()
                            .h_full()
                            .min_w_0()
                            .overflow_y_scroll()
                            .p_4()
                            .gap_3()
                            .child(Label::new(self.page.label()).size(LabelSize::Large))
                            .child(self.page_contents(cx)),
                    ),
            )
    }
}
