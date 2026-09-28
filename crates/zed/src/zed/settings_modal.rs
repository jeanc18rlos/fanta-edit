use client::{Client, ClientSettings, TelemetrySettings};
use fanta_gpui::{
    billing::{
        BillingAction, BillingActionState, BillingActions, BillingLoadState, BillingUsageRange,
        BillingViewData,
    },
    settings::{
        SETTINGS_SCREEN_MIN_HEIGHT, SETTINGS_SCREEN_MIN_WIDTH, SettingsAccountSummary,
        SettingsAction, SettingsChoice, SettingsControl, SettingsGroup, SettingsItem, SettingsPage,
        SettingsPageData, SettingsScreen, SettingsViewData,
    },
};
use fs::Fs;
use futures::StreamExt as _;
use gpui::{
    Action, App, AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Size, StatefulInteractiveElement as _,
    Styled as _, Subscription, Task, TitlebarOptions, WeakEntity, Window, WindowBounds,
    WindowHandle, WindowKind, WindowOptions, div, point, px,
};
use project::context_server_store::ServerStatusChangedEvent;
use settings::{
    AutosaveSetting, FontSize, RelativeLineNumbers, RestoreOnStartupBehavior, Settings,
    SettingsContent, SettingsStore, update_settings_file_with_completion,
};
use theme::ActiveTheme as _;
use theme_settings::ThemeSettings;
use util::ResultExt as _;
use workspace::{MultiWorkspace, Workspace, WorkspaceSettings};

use super::{settings_billing, settings_mcp};

const PRIVACY_POLICY_URL: &str = "https://www.fantaisa.net/privacy";
const TERMS_OF_USE_URL: &str = "https://www.fantaisa.net/terms";

fn legal_document_url(id: &str) -> Option<&'static str> {
    match id {
        "privacy_policy" => Some(PRIVACY_POLICY_URL),
        "terms_of_use" => Some(TERMS_OF_USE_URL),
        _ => None,
    }
}

fn legal_settings_group() -> SettingsGroup {
    SettingsGroup::new(
        "Legal",
        vec![
            action(
                "terms_of_use",
                "Terms of Use",
                "Read the terms for Fanta subscriptions and services.",
                "Open",
            ),
            action(
                "privacy_policy",
                "Privacy Policy",
                "Read how Fanta handles account and application data.",
                "Open",
            ),
        ],
    )
}

pub(super) struct SettingsWindow {
    screen: Entity<SettingsScreen>,
    data: SettingsViewData,
    workspace: WeakEntity<Workspace>,
    source_window: Option<WindowHandle<MultiWorkspace>>,
    usage_range: BillingUsageRange,
    billing_request: u64,
    billing_action_pending: bool,
    billing_unlock_after_refresh: bool,
    _subscriptions: Vec<Subscription>,
    server_subscription: Option<Subscription>,
    _auth_task: Task<()>,
}

fn item(
    id: &'static str,
    title: &'static str,
    description: &'static str,
    control: SettingsControl,
) -> SettingsItem {
    SettingsItem::new(id, title, description, control)
}

fn toggle(
    id: &'static str,
    title: &'static str,
    description: &'static str,
    value: bool,
) -> SettingsItem {
    item(id, title, description, SettingsControl::Toggle(value))
}

fn choice(
    id: &'static str,
    title: &'static str,
    description: &'static str,
    selected: impl Into<SharedString>,
    options: &[(&'static str, &'static str)],
) -> SettingsItem {
    item(
        id,
        title,
        description,
        SettingsControl::Choice {
            selected: selected.into(),
            options: options
                .iter()
                .map(|(value, label)| SettingsChoice::new(*value, *label))
                .collect(),
        },
    )
}

fn action(
    id: &'static str,
    title: &'static str,
    description: &'static str,
    label: &'static str,
) -> SettingsItem {
    item(
        id,
        title,
        description,
        SettingsControl::Action {
            label: label.into(),
        },
    )
}

fn preference_pages(cx: &App, host_workspace: Option<&Entity<Workspace>>) -> Vec<SettingsPageData> {
    let workspace = WorkspaceSettings::get_global(cx);
    let editor = editor::EditorSettings::get_global(cx);
    let theme = ThemeSettings::get_global(cx);
    let telemetry = TelemetrySettings::get_global(cx);

    let startup = match workspace.restore_on_startup {
        RestoreOnStartupBehavior::EmptyTab => "empty_tab",
        RestoreOnStartupBehavior::LastWorkspace => "last_workspace",
        RestoreOnStartupBehavior::LastSession => "last_session",
        RestoreOnStartupBehavior::Launchpad => "launchpad",
    };
    let ui_size = format!("{}", f32::from(theme.ui_font_size(cx)).round() as i32);
    let buffer_size = format!("{}", f32::from(theme.buffer_font_size(cx)).round() as i32);

    vec![
        SettingsPageData::new(
            SettingsPage::General,
            vec![SettingsGroup::new(
                "Startup & files",
                vec![
                    choice(
                        "restore_on_startup",
                        "On launch",
                        "Choose what Fanta opens on launch.",
                        startup,
                        &[
                            ("last_session", "Last session"),
                            ("last_workspace", "Last workspace"),
                            ("empty_tab", "Empty tab"),
                            ("launchpad", "Welcome"),
                        ],
                    ),
                    toggle(
                        "autosave",
                        "Autosave",
                        "Save edited files when focus moves away.",
                        !matches!(workspace.autosave, AutosaveSetting::Off),
                    ),
                    toggle(
                        "confirm_quit",
                        "Confirm before quitting",
                        "Ask before closing Fanta.",
                        workspace.confirm_quit,
                    ),
                    toggle(
                        "native_dialogs",
                        "Use system file dialogs",
                        "Use native Open and Save dialogs.",
                        workspace.use_system_path_prompts,
                    ),
                ],
            )],
        ),
        SettingsPageData::new(
            SettingsPage::Appearance,
            vec![SettingsGroup::new(
                "Interface",
                vec![
                    action(
                        "theme",
                        "Color theme",
                        "Choose a light, dark, or system theme.",
                        "Choose…",
                    ),
                    action(
                        "icon_theme",
                        "Icon theme",
                        "Choose the icons used in panels and source files.",
                        "Choose…",
                    ),
                    choice(
                        "ui_font_size",
                        "Interface text size",
                        "Adjust text and controls throughout Fanta.",
                        ui_size,
                        &[
                            ("14", "14 pt"),
                            ("16", "16 pt"),
                            ("18", "18 pt"),
                            ("20", "20 pt"),
                        ],
                    ),
                ],
            )],
        ),
        SettingsPageData::new(
            SettingsPage::Canvas,
            vec![SettingsGroup::new(
                "Design workspace",
                vec![
                    action(
                        "layers",
                        "Layers",
                        "Show or hide pages and layers beside the active canvas.",
                        "Toggle",
                    ),
                    action(
                        "inspector",
                        "Inspector",
                        "Show or hide design properties beside the active canvas.",
                        "Toggle",
                    ),
                    action(
                        "files",
                        "Files",
                        "Browse project files and FNX source.",
                        "Toggle",
                    ),
                    action(
                        "git",
                        "Git",
                        "Inspect and commit project changes.",
                        "Toggle",
                    ),
                    action(
                        "threads",
                        "Threads",
                        "Show or hide the thread rail.",
                        "Toggle",
                    ),
                ],
            )],
        ),
        SettingsPageData::new(
            SettingsPage::Editor,
            vec![
                SettingsGroup::new(
                    "Source editor",
                    vec![
                        toggle(
                            "cursor_blink",
                            "Blinking cursor",
                            "Blink the caret in source files.",
                            editor.cursor_blink,
                        ),
                        toggle(
                            "relative_line_numbers",
                            "Relative line numbers",
                            "Show line distances in the source editor.",
                            editor.relative_line_numbers != RelativeLineNumbers::Disabled,
                        ),
                        toggle(
                            "line_numbers",
                            "Line numbers",
                            "Show line numbers beside source files.",
                            editor.gutter.line_numbers,
                        ),
                        choice(
                            "buffer_font_size",
                            "Source text size",
                            "Set the font size for code and FNX source.",
                            buffer_size,
                            &[
                                ("12", "12 pt"),
                                ("14", "14 pt"),
                                ("16", "16 pt"),
                                ("18", "18 pt"),
                            ],
                        ),
                    ],
                ),
                SettingsGroup::new(
                    "Keyboard",
                    vec![action(
                        "keymap",
                        "Keyboard shortcuts",
                        "Review and customize command bindings.",
                        "Open keymap",
                    )],
                ),
            ],
        ),
        SettingsPageData::new(
            SettingsPage::AiModels,
            vec![SettingsGroup::new(
                "Agent & models",
                vec![
                    action(
                        "agent_panel",
                        "Agent",
                        "Open the assistant to choose models and manage conversations.",
                        "Open panel",
                    ),
                    action(
                        "configure_providers",
                        "Model providers",
                        "Configure your provider settings and credentials.",
                        "Open settings file",
                    ),
                ],
            )],
        ),
        SettingsPageData::new(
            SettingsPage::McpTools,
            vec![
                SettingsGroup::new(
                    "Agent connections",
                    vec![
                        toggle(
                            "fanta_hosted_tools",
                            "Fanta hosted tools",
                            "Connect the assistant to Fanta generation and design tools.",
                            host_workspace.is_some_and(|workspace| {
                                settings_mcp::fanta_hosted_tools_enabled(workspace, cx)
                            }),
                        ),
                        action(
                            "manage_extension_tools",
                            "Extension tools",
                            "Manage tools installed by extensions in the agent panel.",
                            "Open panel",
                        ),
                    ],
                ),
                SettingsGroup::new(
                    "External agents",
                    vec![toggle(
                        "fanta_live_mcp",
                        "Live canvas MCP server",
                        "Allow local MCP clients to inspect and edit the focused canvas.",
                        settings_mcp::fanta_live_mcp_enabled(cx),
                    )],
                ),
            ],
        ),
        SettingsPageData::new(
            SettingsPage::Account,
            vec![
                SettingsGroup::new(
                    "Fanta account",
                    vec![
                        action(
                            "workspace_details",
                            "Workspace",
                            "Manage members and workspace preferences in the dashboard.",
                            "Manage",
                        ),
                        action(
                            "api_keys",
                            "API keys",
                            "Create and manage keys for Fanta API access.",
                            "Open keys",
                        ),
                        action(
                            "account_security",
                            "Account security",
                            "Review your sign-in and account access.",
                            "Manage",
                        ),
                        #[cfg(feature = "mac_app_store")]
                        action(
                            "delete_account",
                            "Delete Fanta account",
                            "Permanently delete your account and personal cloud data.",
                            "Delete…",
                        ),
                    ],
                ),
                legal_settings_group(),
            ],
        ),
        SettingsPageData::new(
            SettingsPage::Privacy,
            vec![SettingsGroup::new(
                "Diagnostics & data",
                vec![
                    toggle(
                        "diagnostics",
                        "Share crash diagnostics",
                        "Send crash information to improve stability.",
                        telemetry.diagnostics,
                    ),
                    toggle(
                        "metrics",
                        "Share anonymous usage metrics",
                        "Send usage measurements without document contents.",
                        telemetry.metrics,
                    ),
                    action(
                        "privacy_policy",
                        "Privacy Policy",
                        "Read how Fanta handles account and application data.",
                        "Open",
                    ),
                    action(
                        "terms_of_use",
                        "Terms of Use",
                        "Read the terms for Fanta subscriptions and services.",
                        "Open",
                    ),
                ],
            )],
        ),
    ]
}

fn unavailable_billing(message: &'static str) -> BillingViewData {
    let disabled = |label| BillingActionState::disabled(label, message);
    BillingViewData {
        load_state: BillingLoadState::Error(message.into()),
        actions: BillingActions {
            add_credits: disabled("Add credits"),
            manage_subscription: disabled("Manage subscription"),
            restore_purchases: disabled("Restore purchases"),
            redeem_code: disabled("Redeem code"),
            sync_purchases: disabled("Sync purchases"),
            export_usage: disabled("Export usage"),
            refresh: disabled("Refresh"),
        },
        ..BillingViewData::default()
    }
}

fn disable_busy_billing_actions(billing: &mut BillingViewData) {
    const REASON: &str = "Another billing action is in progress.";
    let disable = |action: &mut BillingActionState| {
        if action.enabled {
            *action = BillingActionState::disabled(action.label.clone(), REASON);
        }
    };
    disable(&mut billing.actions.add_credits);
    disable(&mut billing.actions.manage_subscription);
    disable(&mut billing.actions.restore_purchases);
    disable(&mut billing.actions.sync_purchases);
    disable(&mut billing.actions.redeem_code);
    disable(&mut billing.actions.export_usage);
    for plan in &mut billing.plans {
        disable(&mut plan.action);
    }
    for transaction in &mut billing.transactions {
        if let Some(receipt) = &mut transaction.receipt_action {
            disable(receipt);
        }
    }
}

fn apply_toggle_setting(settings: &mut SettingsContent, id: &str, value: bool) -> bool {
    match id {
        "autosave" => {
            settings.workspace.autosave = Some(if value {
                AutosaveSetting::OnFocusChange
            } else {
                AutosaveSetting::Off
            });
        }
        "confirm_quit" => settings.workspace.confirm_quit = Some(value),
        "native_dialogs" => settings.workspace.use_system_path_prompts = Some(value),
        "cursor_blink" => settings.editor.cursor_blink = Some(value),
        "relative_line_numbers" => {
            settings.editor.relative_line_numbers = Some(if value {
                RelativeLineNumbers::Enabled
            } else {
                RelativeLineNumbers::Disabled
            });
        }
        "line_numbers" => {
            settings
                .editor
                .gutter
                .get_or_insert_with(Default::default)
                .line_numbers = Some(value);
        }
        "diagnostics" => {
            settings
                .telemetry
                .get_or_insert_with(Default::default)
                .diagnostics = Some(value);
        }
        "metrics" => {
            settings
                .telemetry
                .get_or_insert_with(Default::default)
                .metrics = Some(value);
        }
        _ => return false,
    }
    true
}

pub(super) fn page_from_path(path: &str) -> SettingsPage {
    let path = path.to_ascii_lowercase();
    if path.contains("mcp") || path.contains("context_server") || path.contains("tool") {
        SettingsPage::McpTools
    } else if path.contains("billing") || path.contains("credit") || path.contains("plan") {
        SettingsPage::Billing
    } else if path.contains("usage") {
        SettingsPage::Usage
    } else if path.contains("theme") || path.contains("appearance") || path.contains("ui_font") {
        SettingsPage::Appearance
    } else if path.contains("editor") || path.contains("language") || path.contains("keymap") {
        SettingsPage::Editor
    } else if path.contains("canvas") || path.contains("panel") || path.contains("dock") {
        SettingsPage::Canvas
    } else if path.contains("agent") || path.contains("model") || path.contains("ai") {
        SettingsPage::AiModels
    } else if path.contains("privacy") || path.contains("telemetry") {
        SettingsPage::Privacy
    } else if path.contains("account") || path.contains("workspace") {
        SettingsPage::Account
    } else {
        SettingsPage::General
    }
}

impl SettingsWindow {
    pub(super) fn open(
        page: SettingsPage,
        _workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        let workspace = cx.entity().downgrade();
        let source_window = window.window_handle().downcast::<MultiWorkspace>();
        // This action runs inside `Workspace::update`. Constructing the settings
        // window immediately would read that same workspace while it is still
        // mutably borrowed, which panics in the native menu callback.
        cx.defer(move |cx| {
            if let Some(existing) = cx
                .windows()
                .into_iter()
                .find_map(|handle| handle.downcast::<Self>())
            {
                existing
                    .update(cx, |settings, window, cx| {
                        settings.workspace = workspace;
                        settings.source_window = source_window;
                        settings.rebind_server_subscription(cx);
                        settings.refresh_local(cx);
                        settings.select_page(page, window, cx);
                        window.activate_window();
                        settings.focus_handle(cx).focus(window, cx);
                    })
                    .log_err();
                return;
            }

            let window_size = Size {
                width: px(1120.),
                height: px(780.),
            };
            let window_bounds = WindowBounds::centered(window_size, cx);
            let window_background = cx.theme().window_background_appearance();
            cx.open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("Fanta Settings".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(point(px(9.), px(9.))),
                    }),
                    window_bounds: Some(window_bounds),
                    window_min_size: Some(Size {
                        width: px(SETTINGS_SCREEN_MIN_WIDTH),
                        height: px(SETTINGS_SCREEN_MIN_HEIGHT.max(640.)),
                    }),
                    kind: WindowKind::Normal,
                    is_movable: true,
                    window_background,
                    ..Default::default()
                },
                move |window, cx| {
                    let settings =
                        cx.new(|cx| Self::new(page, workspace, source_window, window, cx));
                    window.activate_window();
                    settings.read(cx).focus_handle(cx).focus(window, cx);
                    settings.update(cx, |settings, cx| {
                        settings.load_billing(window, cx);
                    });
                    settings
                },
            )
            .log_err();
        });
    }

    fn new(
        page: SettingsPage,
        workspace: WeakEntity<Workspace>,
        source_window: Option<WindowHandle<MultiWorkspace>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mcp_servers = workspace
            .upgrade()
            .map(|workspace| settings_mcp::mcp_server_views(&workspace, cx))
            .unwrap_or_default();
        let data = SettingsViewData {
            selected_page: page,
            scope: "User".into(),
            settings_file_available: true,
            pages: preference_pages(cx, workspace.upgrade().as_ref()),
            mcp_servers,
            account: None,
            usage: None,
            billing: Some(BillingViewData {
                load_state: BillingLoadState::Loading,
                ..BillingViewData::default()
            }),
            notice: None,
        };
        let screen = cx.new(|cx| SettingsScreen::new("fanta-settings", data.clone(), window, cx));
        let subscriptions = vec![
            cx.subscribe_in(
                &screen,
                window,
                |this, _, event: &SettingsAction, window, cx| {
                    this.handle_action(event.clone(), window, cx);
                },
            ),
            cx.observe_global::<SettingsStore>(|this, cx| this.refresh_local(cx)),
        ];
        let server_subscription = workspace.upgrade().map(|workspace| {
            let server_store = workspace.read(cx).project().read(cx).context_server_store();
            cx.subscribe(
                &server_store,
                |this, _, _: &ServerStatusChangedEvent, cx| this.refresh_local(cx),
            )
        });
        let client = Client::global(cx);
        let auth_task = cx.spawn_in(window, async move |this, cx| {
            let mut status = client.status();
            let mut previous_token = client.account_access_token();
            while status.next().await.is_some() {
                let token = client.account_access_token();
                if token == previous_token {
                    continue;
                }
                previous_token = token;
                if this
                    .update_in(cx, |this, window, cx| this.load_billing(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            screen,
            data,
            workspace,
            source_window,
            usage_range: BillingUsageRange::Days30,
            billing_request: 0,
            billing_action_pending: false,
            billing_unlock_after_refresh: false,
            _subscriptions: subscriptions,
            server_subscription,
            _auth_task: auth_task,
        }
    }

    fn rebind_server_subscription(&mut self, cx: &mut Context<Self>) {
        self.server_subscription = self.workspace.upgrade().map(|workspace| {
            let server_store = workspace.read(cx).project().read(cx).context_server_store();
            cx.subscribe(
                &server_store,
                |this, _, _: &ServerStatusChangedEvent, cx| this.refresh_local(cx),
            )
        });
    }

    fn sync_screen(&mut self, cx: &mut Context<Self>) {
        let mut data = self.data.clone();
        #[cfg(feature = "mac_app_store")]
        if data.selected_page == SettingsPage::McpTools {
            const LOCAL_MCP_NOTICE: &str = "This Mac App Store build cannot run local MCP commands. Use a remote HTTP server here, or use a non-Store build for local commands.";
            data.notice = Some(match data.notice.take() {
                Some(notice) => format!("{LOCAL_MCP_NOTICE}\n\n{notice}").into(),
                None => LOCAL_MCP_NOTICE.into(),
            });
        }
        if self.billing_action_pending {
            if let Some(billing) = &mut data.billing {
                disable_busy_billing_actions(billing);
            }
        }
        self.screen
            .update(cx, |screen, cx| screen.set_view_data(data, cx));
        cx.notify();
    }

    fn refresh_local(&mut self, cx: &mut Context<Self>) {
        self.data.pages = preference_pages(cx, self.workspace.upgrade().as_ref());
        self.data.mcp_servers = self
            .workspace
            .upgrade()
            .map(|workspace| settings_mcp::mcp_server_views(&workspace, cx))
            .unwrap_or_default();
        self.sync_screen(cx);
    }

    fn select_page(&mut self, page: SettingsPage, window: &mut Window, cx: &mut Context<Self>) {
        self.data.selected_page = page;
        self.sync_screen(cx);
        if matches!(
            page,
            SettingsPage::Account
                | SettingsPage::Billing
                | SettingsPage::Usage
                | SettingsPage::Plans
                | SettingsPage::Activity
        ) {
            self.load_billing(window, cx);
        }
    }

    fn notice(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.data.notice = Some(message.into());
        self.sync_screen(cx);
    }

    fn load_billing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.billing_request = self.billing_request.wrapping_add(1);
        let request = self.billing_request;
        let client = Client::global(cx);
        let Some(token) = client.account_access_token() else {
            if self.billing_unlock_after_refresh {
                self.billing_action_pending = false;
                self.billing_unlock_after_refresh = false;
            }
            self.data.account = None;
            self.data.usage = None;
            self.data.billing = Some(unavailable_billing(
                "Sign in to Fanta to see credits, usage, and purchases.",
            ));
            self.sync_screen(cx);
            return;
        };
        let server_url = ClientSettings::get_global(cx).server_url.clone();
        let range = self.usage_range;
        self.data.billing = Some(BillingViewData {
            load_state: BillingLoadState::Loading,
            ..self.data.billing.take().unwrap_or_default()
        });
        self.sync_screen(cx);
        cx.spawn_in(window, async move |this, cx| {
            let (account, billing) = futures::join!(
                settings_billing::load_account_summary(
                    client.clone(),
                    token.clone(),
                    server_url.clone()
                ),
                settings_billing::load_billing_snapshot(
                    client.clone(),
                    token.clone(),
                    server_url,
                    range
                )
            );
            this.update(cx, |this, cx| {
                if this.billing_request != request
                    || Client::global(cx).account_access_token().as_deref() != Some(token.as_ref())
                {
                    return;
                }
                match account {
                    Ok(account) => this.data.account = Some(account),
                    Err(error) => {
                        this.data.account = Some(SettingsAccountSummary {
                            display_name: "Fanta account".into(),
                            email: format!("Account details unavailable: {error:#}").into(),
                            plan: "—".into(),
                            workspace: "—".into(),
                        });
                    }
                }
                match billing {
                    Ok(billing) => {
                        this.data.usage = Some(fanta_gpui::settings::SettingsUsageSummary {
                            available: format!("{} credits", billing.balance.available_label)
                                .into(),
                            used: billing.usage.total_label.clone(),
                            period: range.label().into(),
                        });
                        this.data.billing = Some(billing);
                    }
                    Err(error) => {
                        this.data.usage = None;
                        let mut billing = this.data.billing.take().unwrap_or_default();
                        billing.load_state = BillingLoadState::Error(format!("{error:#}").into());
                        this.data.billing = Some(billing);
                    }
                }
                if this.billing_unlock_after_refresh {
                    this.billing_action_pending = false;
                    this.billing_unlock_after_refresh = false;
                }
                this.sync_screen(cx);
            })
            .log_err();
        })
        .detach();
    }

    fn perform_billing_action(
        &mut self,
        action: BillingAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.billing_action_pending {
            self.notice("Wait for the current billing action to finish.", cx);
            return;
        }
        let client = Client::global(cx);
        let Some(token) = client.account_access_token() else {
            self.notice("Sign in to Fanta before managing purchases.", cx);
            return;
        };
        let server_url = ClientSettings::get_global(cx).server_url.clone();
        let range = self.usage_range;
        self.billing_action_pending = true;
        self.notice("Working with your Fanta account…", cx);
        cx.spawn_in(window, async move |this, cx| {
            let result = settings_billing::perform_billing_action(
                action, client, token, server_url, range, cx,
            )
            .await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(settings_billing::BillingActionOutcome::Refresh { notice }) => {
                    if let Some(notice) = notice {
                        this.notice(notice, cx);
                    }
                    this.billing_unlock_after_refresh = true;
                    this.load_billing(window, cx);
                }
                Ok(settings_billing::BillingActionOutcome::NoRefresh { notice }) => {
                    this.billing_action_pending = false;
                    if let Some(notice) = notice {
                        this.notice(notice, cx);
                    } else {
                        this.data.notice = None;
                        this.sync_screen(cx);
                    }
                }
                Err(error) => {
                    this.billing_action_pending = false;
                    this.notice(format!("{error:#}"), cx);
                }
            })
            .log_err();
        })
        .detach();
    }

    fn persist_toggle(&mut self, id: SharedString, value: bool, cx: &mut Context<Self>) {
        if matches!(id.as_ref(), "fanta_hosted_tools" | "fanta_live_mcp") {
            let task = if id.as_ref() == "fanta_hosted_tools" {
                let Some(workspace) = self.workspace.upgrade() else {
                    self.notice("Open a workspace to manage Fanta hosted tools.", cx);
                    return;
                };
                settings_mcp::set_fanta_hosted_tools_enabled(workspace, value, cx)
            } else {
                settings_mcp::set_fanta_live_mcp_enabled(value, cx)
            };
            cx.spawn(async move |this, cx| {
                let result = task.await;
                this.update(cx, |this, cx| {
                    if let Err(error) = result {
                        this.notice(format!("Could not update MCP tools: {error:#}"), cx);
                    }
                    this.refresh_local(cx);
                })
                .log_err();
            })
            .detach();
            return;
        }
        if !matches!(
            id.as_ref(),
            "autosave"
                | "confirm_quit"
                | "native_dialogs"
                | "cursor_blink"
                | "relative_line_numbers"
                | "line_numbers"
                | "diagnostics"
                | "metrics"
        ) {
            self.notice("This preference is unavailable in this build.", cx);
            return;
        }
        let receiver =
            update_settings_file_with_completion(<dyn Fs>::global(cx), cx, move |settings, _| {
                apply_toggle_setting(settings, &id, value);
            });
        self.finish_preference_write(receiver, cx);
    }

    fn persist_choice(&mut self, id: SharedString, value: SharedString, cx: &mut Context<Self>) {
        let font_size = match id.as_ref() {
            "ui_font_size" | "buffer_font_size" => {
                let Ok(size) = value.parse::<f32>() else {
                    self.notice("The selected font size is invalid.", cx);
                    return;
                };
                if !(6.0..=100.0).contains(&size) {
                    self.notice("The selected font size is outside the supported range.", cx);
                    return;
                }
                Some(size)
            }
            "restore_on_startup" => None,
            _ => {
                self.notice("This preference is unavailable in this build.", cx);
                return;
            }
        };
        let receiver =
            update_settings_file_with_completion(<dyn Fs>::global(cx), cx, move |settings, _| {
                match id.as_ref() {
                    "restore_on_startup" => {
                        settings.workspace.restore_on_startup = Some(match value.as_ref() {
                            "empty_tab" => RestoreOnStartupBehavior::EmptyTab,
                            "last_workspace" => RestoreOnStartupBehavior::LastWorkspace,
                            "launchpad" => RestoreOnStartupBehavior::Launchpad,
                            _ => RestoreOnStartupBehavior::LastSession,
                        });
                    }
                    "ui_font_size" => settings.theme.ui_font_size = font_size.map(FontSize),
                    "buffer_font_size" => settings.theme.buffer_font_size = font_size.map(FontSize),
                    _ => {}
                }
            });
        self.finish_preference_write(receiver, cx);
    }

    fn finish_preference_write(
        &mut self,
        receiver: futures::channel::oneshot::Receiver<anyhow::Result<()>>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let result = receiver.await;
            this.update(cx, |this, cx| match result {
                Ok(Ok(())) => this.refresh_local(cx),
                Ok(Err(error)) => this.notice(format!("Could not save setting: {error:#}"), cx),
                Err(_) => this.notice("The settings write was interrupted.", cx),
            })
            .log_err();
        })
        .detach();
    }

    fn dispatch_to_workspace(&mut self, action: Box<dyn Action>, cx: &mut Context<Self>) {
        let Some(window) = self.source_window.as_ref() else {
            self.notice("Open a Fanta workspace to use this action.", cx);
            return;
        };
        if let Err(error) = window.update(cx, |_, window, cx| {
            window.activate_window();
            window.dispatch_action(action, cx);
        }) {
            self.notice(format!("Could not open workspace action: {error:#}"), cx);
        }
    }

    fn handle_action_request(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(url) = legal_document_url(id) {
            cx.open_url(url);
            return;
        }
        match id {
            "settings_json" | "configure_providers" => {
                self.dispatch_to_workspace(Box::new(zed_actions::OpenSettingsFile), cx)
            }
            "keymap" => self.dispatch_to_workspace(Box::new(zed_actions::OpenKeymapFile), cx),
            "theme" => self.dispatch_to_workspace(
                Box::new(zed_actions::theme_selector::Toggle::default()),
                cx,
            ),
            "icon_theme" => self.dispatch_to_workspace(
                Box::new(zed_actions::icon_theme_selector::Toggle::default()),
                cx,
            ),
            "layers" => self.dispatch_to_workspace(Box::new(fig_viewer::ToggleLayersSidebar), cx),
            "inspector" => {
                self.dispatch_to_workspace(Box::new(fig_viewer::ToggleInspectorSidebar), cx)
            }
            "files" => self.dispatch_to_workspace(Box::new(zed_actions::project_panel::Toggle), cx),
            "git" => self.dispatch_to_workspace(Box::new(git_ui::git_panel::Toggle), cx),
            "threads" => {
                self.dispatch_to_workspace(Box::new(workspace::ToggleWorkspaceSidebar), cx)
            }
            "agent_panel" | "manage_extension_tools" => {
                self.dispatch_to_workspace(Box::new(zed_actions::assistant::Toggle), cx)
            }
            "workspace_details" => cx.open_url("https://app.fantaisa.net/organization"),
            "api_keys" => cx.open_url("https://app.fantaisa.net/keys"),
            "account_security" => cx.open_url("https://app.fantaisa.net/settings"),
            #[cfg(feature = "mac_app_store")]
            "delete_account" => super::app_store_billing::request_account_deletion(cx),
            _ => self.notice("This settings action is unavailable.", cx),
        }
    }

    fn handle_action(
        &mut self,
        action: SettingsAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            SettingsAction::PageSelected(page) => self.select_page(page, window, cx),
            SettingsAction::ToggleRequested { id, value } => self.persist_toggle(id, value, cx),
            SettingsAction::ChoiceRequested { id, value } => self.persist_choice(id, value, cx),
            SettingsAction::ActionRequested { id } => self.handle_action_request(&id, cx),
            SettingsAction::BillingActionRequested(action) => match action {
                BillingAction::TabSelected(tab) => {
                    let page = match tab {
                        fanta_gpui::billing::BillingTab::Overview => SettingsPage::Billing,
                        fanta_gpui::billing::BillingTab::Usage => SettingsPage::Usage,
                        fanta_gpui::billing::BillingTab::Plans => SettingsPage::Plans,
                        fanta_gpui::billing::BillingTab::History => SettingsPage::Activity,
                    };
                    self.select_page(page, window, cx);
                }
                BillingAction::UsageRangeSelected(range) => {
                    self.usage_range = range;
                    self.load_billing(window, cx);
                }
                BillingAction::RefreshRequested => self.load_billing(window, cx),
                action => self.perform_billing_action(action, window, cx),
            },
            SettingsAction::SignInRequested => {
                let client = Client::global(cx);
                cx.spawn_in(window, async move |this, cx| {
                    let result = client.sign_in_with_optional_connect(false, cx).await;
                    this.update_in(cx, |this, _, cx| match result {
                        Ok(()) => {
                            // The client status watcher loads account and billing after sign-in.
                            this.data.notice = None;
                            this.sync_screen(cx);
                        }
                        Err(error) => this.notice(format!("Could not sign in: {error:#}"), cx),
                    })
                    .log_err();
                })
                .detach();
            }
            SettingsAction::WorkspaceRequested => {
                cx.open_url("https://app.fantaisa.net/organization")
            }
            SettingsAction::ApiKeysRequested => cx.open_url("https://app.fantaisa.net/keys"),
            SettingsAction::McpSaveRequested { server_id, draft } => {
                let Some(workspace) = self.workspace.upgrade() else {
                    self.screen.update(cx, |screen, cx| {
                        screen.finish_mcp_save(Err("Open a workspace to configure MCP.".into()), cx)
                    });
                    return;
                };
                let task = settings_mcp::save_mcp_server(workspace, server_id, draft, cx);
                cx.spawn(async move |this, cx| {
                    let result = task.await.map_err(|error| format!("{error:#}").into());
                    this.update(cx, |this, cx| {
                        this.screen
                            .update(cx, |screen, cx| screen.finish_mcp_save(result, cx));
                        this.refresh_local(cx);
                    })
                    .log_err();
                })
                .detach();
            }
            SettingsAction::McpActionRequested { server_id, action } => {
                let Some(workspace) = self.workspace.upgrade() else {
                    self.notice("Open a workspace to manage MCP servers.", cx);
                    return;
                };
                let task =
                    settings_mcp::handle_mcp_action(workspace, server_id, action, window, cx);
                cx.spawn(async move |this, cx| {
                    let result = task.await;
                    this.update(cx, |this, cx| {
                        if let Err(error) = result {
                            this.notice(format!("Could not update MCP server: {error:#}"), cx);
                        }
                        this.refresh_local(cx);
                    })
                    .log_err();
                })
                .detach();
            }
        }
    }
}

impl Focusable for SettingsWindow {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.screen.read(cx).focus_handle(cx)
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let legal_prompt = if matches!(
            self.data.selected_page,
            SettingsPage::Billing | SettingsPage::Plans
        ) {
            "Before subscribing, review"
        } else {
            "Fanta legal"
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.background)
            .child(div().flex_1().min_h_0().child(self.screen.clone()))
            .child(
                div()
                    .id("fanta-settings-legal-footer")
                    .h(px(52.))
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(16.))
                    .px(px(24.))
                    .border_t_1()
                    .border_color(colors.border_variant)
                    .text_size(px(14.))
                    .child(div().text_color(colors.text_muted).child(legal_prompt))
                    .child(
                        div()
                            .id("fanta-settings-terms-link")
                            .cursor_pointer()
                            .text_color(colors.text_accent)
                            .hover(|link| link.text_color(colors.text))
                            .on_click(cx.listener(|_, _, _, cx| cx.open_url(TERMS_OF_USE_URL)))
                            .child("Terms of Use"),
                    )
                    .child(
                        div()
                            .id("fanta-settings-privacy-link")
                            .cursor_pointer()
                            .text_color(colors.text_accent)
                            .hover(|link| link.text_color(colors.text))
                            .on_click(cx.listener(|_, _, _, cx| cx.open_url(PRIVACY_POLICY_URL)))
                            .child("Privacy Policy"),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_legal_actions_open_the_public_subscription_documents() {
        let legal = legal_settings_group();
        let links: Vec<_> = legal
            .items
            .iter()
            .map(|item| (item.title.as_ref(), legal_document_url(item.id.as_ref())))
            .collect();
        assert_eq!(
            links,
            vec![
                ("Terms of Use", Some(TERMS_OF_USE_URL)),
                ("Privacy Policy", Some(PRIVACY_POLICY_URL)),
            ]
        );
    }

    #[test]
    fn settings_paths_open_the_relevant_native_destination() {
        assert_eq!(page_from_path("theme.mode"), SettingsPage::Appearance);
        assert_eq!(page_from_path("editor.cursor_blink"), SettingsPage::Editor);
        assert_eq!(page_from_path("agent.mcp.servers"), SettingsPage::McpTools);
        assert_eq!(page_from_path("account.billing"), SettingsPage::Billing);
        assert_eq!(page_from_path("ai.usage"), SettingsPage::Usage);
        assert_eq!(page_from_path("agent.model"), SettingsPage::AiModels);
    }

    #[test]
    fn native_toggle_intents_write_real_settings_fields() {
        let mut content = SettingsContent::default();
        assert!(apply_toggle_setting(&mut content, "autosave", true));
        assert!(matches!(
            content.workspace.autosave,
            Some(AutosaveSetting::OnFocusChange)
        ));
        assert!(apply_toggle_setting(&mut content, "line_numbers", false));
        assert_eq!(
            content
                .editor
                .gutter
                .as_ref()
                .and_then(|gutter| gutter.line_numbers),
            Some(false)
        );
        assert!(apply_toggle_setting(&mut content, "diagnostics", false));
        assert_eq!(
            content
                .telemetry
                .as_ref()
                .and_then(|telemetry| telemetry.diagnostics),
            Some(false)
        );
        assert!(!apply_toggle_setting(&mut content, "unsupported", true));
    }

    #[test]
    fn billing_buttons_disable_during_an_action_without_changing_the_snapshot() {
        let mut billing = BillingViewData::default();
        billing.actions.add_credits = BillingActionState::enabled("Add credits");
        billing.actions.restore_purchases = BillingActionState::enabled("Restore purchases");
        billing.actions.refresh = BillingActionState::enabled("Refresh");
        billing.plans.push(fanta_gpui::billing::BillingPlan {
            id: "pro".into(),
            name: "Pro".into(),
            eyebrow: "".into(),
            description: "".into(),
            price_label: "".into(),
            cadence_label: "".into(),
            credits_label: "".into(),
            features: Vec::new(),
            highlighted: false,
            current: false,
            action: BillingActionState::enabled("Choose Pro"),
        });
        let original = billing.clone();

        disable_busy_billing_actions(&mut billing);

        assert!(!billing.actions.add_credits.enabled);
        assert!(!billing.actions.restore_purchases.enabled);
        assert!(!billing.plans[0].action.enabled);
        assert!(billing.actions.refresh.enabled);
        assert!(original.actions.add_credits.enabled);
        assert!(original.actions.restore_purchases.enabled);
        assert!(original.plans[0].action.enabled);
    }
}
