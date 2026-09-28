use anyhow::{Context as _, Result, ensure};
use context_server::{ContextServerCommand, ContextServerId};
use fanta_gpui::settings::{
    SettingsMcpAction, SettingsMcpConnection, SettingsMcpDraft, SettingsMcpScope,
    SettingsMcpServerView, SettingsMcpStatus,
};
use fs::Fs;
use gpui::{App, Entity, ReadGlobal as _, SharedString, Task, Window};
use project::{
    context_server_store::{ContextServerStatus, ContextServerStore},
    project_settings::ContextServerSettings,
};
use settings::{SettingsStore, update_settings_file_with_completion};
use std::{path::PathBuf, sync::Arc};
use workspace::Workspace;

const HIDDEN_ARGUMENTS: &str = "[arguments configured]";

pub(super) fn fanta_live_mcp_enabled(cx: &App) -> bool {
    SettingsStore::global(cx)
        .merged_settings()
        .fanta_live_mcp
        .as_ref()
        .and_then(|settings| settings.enabled)
        .unwrap_or(true)
}

pub(super) fn set_fanta_live_mcp_enabled(enabled: bool, cx: &mut App) -> Task<Result<()>> {
    let fs = workspace::AppState::global(cx).fs.clone();
    let completion = update_settings_file_with_completion(fs, cx, move |settings, _| {
        settings
            .fanta_live_mcp
            .get_or_insert_with(Default::default)
            .enabled = Some(enabled);
    });
    cx.spawn(async move |_| {
        completion.await.context("Settings write was cancelled")??;
        Ok(())
    })
}

fn scope_for_server(id: &str, workspace: &Entity<Workspace>, cx: &App) -> SettingsMcpScope {
    let project = workspace.read(cx).project().clone();
    let project = project.read(cx);
    let Some(worktree) = project.visible_worktrees(cx).next() else {
        return SettingsMcpScope::Global;
    };
    let worktree_id = worktree.read(cx).id();
    if SettingsStore::global(cx)
        .local_settings(worktree_id)
        .any(|(path, settings)| {
            path.as_unix_str().is_empty() && settings.context_servers.contains_key(id)
        })
    {
        SettingsMcpScope::Workspace
    } else {
        SettingsMcpScope::Global
    }
}

fn display_url(url: &str) -> SharedString {
    match url::Url::parse(url) {
        Ok(mut parsed) => {
            parsed.set_query(None);
            parsed.set_fragment(None);
            if parsed.set_username("").is_err() || parsed.set_password(None).is_err() {
                return "Invalid URL; open settings.json to repair".into();
            }
            parsed.to_string().into()
        }
        Err(_) => "Invalid URL; open settings.json to repair".into(),
    }
}

pub(super) fn mcp_server_views(
    workspace: &Entity<Workspace>,
    cx: &App,
) -> Vec<SettingsMcpServerView> {
    let store = workspace.read(cx).project().read(cx).context_server_store();
    let store = store.read(cx);
    store
        .server_ids()
        .iter()
        .filter_map(|id| {
            if id.0.as_ref() == "fanta" || store.is_extension_provided(id, cx) {
                return None;
            }
            let settings = store.settings_for_server(id)?;
            let (connection, enabled) = match settings {
                ContextServerSettings::Stdio {
                    command, enabled, ..
                } => (
                    SettingsMcpConnection::Local {
                        command: command.path.display().to_string().into(),
                        args: if command.args.is_empty() {
                            Vec::new()
                        } else {
                            vec![HIDDEN_ARGUMENTS.into()]
                        },
                    },
                    *enabled,
                ),
                ContextServerSettings::Http { url, enabled, .. } => (
                    SettingsMcpConnection::Http {
                        url: display_url(url),
                    },
                    *enabled,
                ),
                ContextServerSettings::Extension { .. } => return None,
            };
            let (status, status_detail) = match store.status_for_server(id) {
                Some(ContextServerStatus::Running) => (SettingsMcpStatus::Running, None),
                Some(ContextServerStatus::Starting | ContextServerStatus::Authenticating) => {
                    (SettingsMcpStatus::Connecting, None)
                }
                Some(ContextServerStatus::AuthRequired) => (
                    SettingsMcpStatus::AuthenticationRequired,
                    Some("Sign in to connect this server.".into()),
                ),
                Some(ContextServerStatus::ClientSecretRequired { .. }) => (
                    SettingsMcpStatus::AuthenticationRequired,
                    Some(
                        "A client secret is required. Configure this server in advanced settings."
                            .into(),
                    ),
                ),
                Some(ContextServerStatus::Error(_)) => (
                    SettingsMcpStatus::Error,
                    Some("The server could not connect. Retry or inspect the app log.".into()),
                ),
                Some(ContextServerStatus::Stopped) | None => (SettingsMcpStatus::Stopped, None),
            };
            Some(SettingsMcpServerView {
                id: id.0.to_string().into(),
                name: id.0.to_string().into(),
                description: match settings {
                    ContextServerSettings::Stdio { .. } => "Local MCP server".into(),
                    ContextServerSettings::Http { .. } => "Remote MCP server".into(),
                    ContextServerSettings::Extension { .. } => unreachable!(),
                },
                connection,
                status,
                enabled,
                scope: scope_for_server(&id.0, workspace, cx),
                tool_count: None,
                status_detail,
            })
        })
        .collect()
}

pub(super) fn fanta_hosted_tools_enabled(workspace: &Entity<Workspace>, cx: &App) -> bool {
    let store = workspace.read(cx).project().read(cx).context_server_store();
    store
        .read(cx)
        .settings_for_server(&ContextServerId("fanta".into()))
        .is_some_and(ContextServerSettings::enabled)
}

pub(super) fn set_fanta_hosted_tools_enabled(
    workspace: Entity<Workspace>,
    enabled: bool,
    cx: &mut App,
) -> Task<Result<()>> {
    let store = workspace.read(cx).project().read(cx).context_server_store();
    let current = store
        .read(cx)
        .settings_for_server(&ContextServerId("fanta".into()))
        .cloned();
    let scope = scope_for_server("fanta", &workspace, cx);
    let project_path = if scope == SettingsMcpScope::Workspace {
        Some(local_settings_path(&workspace, cx))
    } else {
        None
    };
    let fs = workspace.read(cx).app_state().fs.clone();
    cx.spawn(async move |cx| {
        let mut current = current.context("Fanta hosted tools are unavailable")?;
        current.set_enabled(enabled);
        match scope {
            SettingsMcpScope::Global => {
                write_user_settings(
                    fs,
                    Some("fanta".into()),
                    Some(("fanta".into(), current)),
                    cx,
                )
                .await
            }
            SettingsMcpScope::Workspace => {
                write_project_settings(
                    project_path
                        .transpose()?
                        .context("Workspace path unavailable")?,
                    fs,
                    Some("fanta".into()),
                    Some(("fanta".into(), current)),
                    cx,
                )
                .await
            }
        }
    })
}

fn local_settings_path(workspace: &Entity<Workspace>, cx: &App) -> Result<PathBuf> {
    let project = workspace.read(cx).project().clone();
    let project = project.read(cx);
    let worktree = project
        .visible_worktrees(cx)
        .next()
        .context("Open a local workspace to save workspace MCP settings")?;
    let worktree = worktree.read(cx);
    ensure!(
        worktree.is_local(),
        "Workspace settings require a local project"
    );
    let root = worktree
        .root_dir()
        .context("Open a project folder to save workspace MCP settings")?;
    Ok(root.join(paths::local_settings_file_relative_path().as_std_path()))
}

async fn write_project_settings(
    path: PathBuf,
    fs: Arc<dyn Fs>,
    old_id: Option<String>,
    new_entry: Option<(String, ContextServerSettings)>,
    cx: &mut gpui::AsyncApp,
) -> Result<()> {
    let old_text = if fs.is_file(&path).await {
        fs.load(&path).await?
    } else {
        "{}\n".to_string()
    };
    let new_text = cx.update(|cx| {
        SettingsStore::global(cx).new_text_for_update(old_text, |settings| {
            if let Some(old_id) = &old_id {
                settings.project.context_servers.remove(old_id.as_str());
            }
            if let Some((id, entry)) = new_entry {
                settings
                    .project
                    .context_servers
                    .insert(id.into(), entry.into());
            }
        })
    })?;
    if let Some(parent) = path.parent() {
        fs.create_dir(parent).await?;
    }
    let target = if fs.is_file(&path).await {
        fs.canonicalize(&path).await?
    } else {
        path
    };
    fs.atomic_write(target, new_text).await?;
    Ok(())
}

async fn write_user_settings(
    fs: Arc<dyn Fs>,
    old_id: Option<String>,
    new_entry: Option<(String, ContextServerSettings)>,
    cx: &mut gpui::AsyncApp,
) -> Result<()> {
    let completion = cx.update(|cx| {
        update_settings_file_with_completion(fs, cx, move |settings, _| {
            if let Some(old_id) = old_id {
                settings.project.context_servers.remove(old_id.as_str());
            }
            if let Some((id, entry)) = new_entry {
                settings
                    .project
                    .context_servers
                    .insert(id.into(), entry.into());
            }
        })
    });
    completion.await.context("Settings write was cancelled")??;
    Ok(())
}

fn connection_settings(
    draft: &SettingsMcpDraft,
    original: Option<&ContextServerSettings>,
) -> Result<ContextServerSettings> {
    Ok(match &draft.connection {
        SettingsMcpConnection::Local { command, args } => {
            ensure!(!command.trim().is_empty(), "Enter a command");
            let (env, timeout, remote, original_args) = match original {
                Some(ContextServerSettings::Stdio {
                    command, remote, ..
                }) => (
                    command.env.clone(),
                    command.timeout,
                    *remote,
                    Some(command.args.clone()),
                ),
                _ => (None, None, false, None),
            };
            let args = if args.len() == 1 && args[0].as_ref() == HIDDEN_ARGUMENTS {
                original_args.context("Re-enter the command arguments")?
            } else {
                args.iter().map(ToString::to_string).collect()
            };
            ContextServerSettings::Stdio {
                enabled: draft.enabled,
                remote,
                command: ContextServerCommand {
                    path: PathBuf::from(command.as_ref()),
                    args,
                    env,
                    timeout,
                },
            }
        }
        SettingsMcpConnection::Http { url } => {
            let parsed = url::Url::parse(url.as_ref()).context("Enter a valid server URL")?;
            ensure!(
                matches!(parsed.scheme(), "http" | "https"),
                "Use HTTP or HTTPS"
            );
            ensure!(
                parsed.username().is_empty() && parsed.password().is_none(),
                "Remove credentials from the URL"
            );
            let (url, headers, timeout, oauth) = match original {
                Some(ContextServerSettings::Http {
                    url: original_url,
                    headers,
                    timeout,
                    oauth,
                    ..
                }) => {
                    let url = if display_url(original_url).as_ref() == url.as_ref() {
                        original_url.clone()
                    } else {
                        url.to_string()
                    };
                    (url, headers.clone(), *timeout, oauth.clone())
                }
                _ => (url.to_string(), Default::default(), None, None),
            };
            ContextServerSettings::Http {
                enabled: draft.enabled,
                url,
                headers,
                timeout,
                oauth,
            }
        }
    })
}

pub(super) fn save_mcp_server(
    workspace: Entity<Workspace>,
    server_id: Option<SharedString>,
    draft: SettingsMcpDraft,
    cx: &mut App,
) -> Task<Result<()>> {
    let fs = workspace.read(cx).app_state().fs.clone();
    let old_id = server_id.map(|id| id.to_string());
    let new_id = draft.name.trim().to_string();
    let store = workspace.read(cx).project().read(cx).context_server_store();
    let (original, old_scope, exists) = {
        let store = store.read(cx);
        let original = old_id.as_ref().and_then(|id| {
            store
                .settings_for_server(&ContextServerId(id.clone().into()))
                .cloned()
        });
        let old_scope = old_id
            .as_ref()
            .map(|id| scope_for_server(id, &workspace, cx));
        let exists = store
            .settings_for_server(&ContextServerId(new_id.clone().into()))
            .is_some();
        (original, old_scope, exists)
    };
    let project_path = match (draft.scope, old_scope) {
        (SettingsMcpScope::Workspace, _) | (_, Some(SettingsMcpScope::Workspace)) => {
            Some(local_settings_path(&workspace, cx))
        }
        _ => None,
    };
    cx.spawn(async move |cx| {
        ensure!(!new_id.is_empty(), "Enter a server name");
        ensure!(
            new_id != "fanta",
            "Fanta hosted tools are configured separately"
        );
        ensure!(
            old_id.as_deref() == Some(new_id.as_str()) || !exists,
            "An MCP server with this name already exists"
        );
        let original = if old_id.is_some() {
            Some(original.context("MCP server no longer exists")?)
        } else {
            None
        };
        ensure!(
            !matches!(original, Some(ContextServerSettings::Extension { .. })),
            "Extension servers are managed in the Agent panel"
        );
        let entry = connection_settings(&draft, original.as_ref())?;
        let project_path = project_path.transpose()?;
        match draft.scope {
            SettingsMcpScope::Global => {
                write_user_settings(
                    fs.clone(),
                    old_id
                        .clone()
                        .filter(|_| old_scope == Some(SettingsMcpScope::Global)),
                    Some((new_id, entry)),
                    cx,
                )
                .await?;
                if old_scope == Some(SettingsMcpScope::Workspace) {
                    write_project_settings(
                        project_path.context("Workspace path unavailable")?,
                        fs,
                        old_id,
                        None,
                        cx,
                    )
                    .await?;
                }
            }
            SettingsMcpScope::Workspace => {
                write_project_settings(
                    project_path.context("Workspace path unavailable")?,
                    fs.clone(),
                    old_id
                        .clone()
                        .filter(|_| old_scope == Some(SettingsMcpScope::Workspace)),
                    Some((new_id, entry)),
                    cx,
                )
                .await?;
                if old_scope == Some(SettingsMcpScope::Global) {
                    write_user_settings(fs, old_id, None, cx).await?;
                }
            }
        }
        Ok(())
    })
}

pub(super) fn handle_mcp_action(
    workspace: Entity<Workspace>,
    server_id: SharedString,
    action: SettingsMcpAction,
    window: &mut Window,
    cx: &mut App,
) -> Task<Result<()>> {
    let id = ContextServerId(server_id.to_string().into());
    if id.0.as_ref() == "fanta" {
        return Task::ready(Err(anyhow::anyhow!(
            "Fanta hosted tools are configured separately"
        )));
    }
    let store = workspace.read(cx).project().read(cx).context_server_store();
    match action {
        SettingsMcpAction::Authenticate => {
            if matches!(
                store.read(cx).status_for_server(&id),
                Some(ContextServerStatus::ClientSecretRequired { .. })
            ) {
                return agent_ui::configure_existing_context_server(
                    workspace,
                    id.0.to_string(),
                    window,
                    cx,
                );
            }
            return Task::ready(store.update(cx, |store, cx| store.authenticate_server(&id, cx)));
        }
        SettingsMcpAction::Retry => {
            return Task::ready(store.update(cx, |store, cx| store.retry_server(&id, cx)));
        }
        SettingsMcpAction::Remove | SettingsMcpAction::ToggleEnabled(_) => {}
    }
    let current = store.read(cx).settings_for_server(&id).cloned();
    let cleanup_url = if action == SettingsMcpAction::Remove
        && store.read(cx).get_server(&id).is_none()
    {
        let candidate = match current.as_ref() {
            Some(ContextServerSettings::Http { url, .. }) => url::Url::parse(url).ok(),
            _ => None,
        };
        candidate.filter(|candidate| {
            let candidate_key = context_server::oauth::canonical_server_uri(candidate);
            let store = store.read(cx);
            !store.server_ids().iter().any(|other_id| {
                if other_id == &id {
                    return false;
                }
                match store.settings_for_server(other_id) {
                    Some(ContextServerSettings::Http { url, .. }) => {
                        url::Url::parse(url).ok().is_some_and(|other_url| {
                            context_server::oauth::canonical_server_uri(&other_url) == candidate_key
                        })
                    }
                    _ => false,
                }
            })
        })
    } else {
        None
    };
    let scope = scope_for_server(&id.0, &workspace, cx);
    let fs = workspace.read(cx).app_state().fs.clone();
    let project_path = if scope == SettingsMcpScope::Workspace {
        Some(local_settings_path(&workspace, cx))
    } else {
        None
    };
    cx.spawn(async move |cx| {
        let mut current = current.context("MCP server no longer exists")?;
        ensure!(
            !matches!(current, ContextServerSettings::Extension { .. }),
            "Extension servers are managed in the Agent panel"
        );
        let entry = match action {
            SettingsMcpAction::Remove => None,
            SettingsMcpAction::ToggleEnabled(enabled) => {
                current.set_enabled(enabled);
                Some((id.0.to_string(), current))
            }
            SettingsMcpAction::Authenticate | SettingsMcpAction::Retry => unreachable!(),
        };
        let project_path = project_path.transpose()?;
        match scope {
            SettingsMcpScope::Global => {
                write_user_settings(fs, Some(id.0.to_string()), entry, cx).await
            }
            SettingsMcpScope::Workspace => {
                write_project_settings(
                    project_path.context("Workspace path unavailable")?,
                    fs,
                    Some(id.0.to_string()),
                    entry,
                    cx,
                )
                .await
            }
        }?;
        if let Some(url) = cleanup_url {
            ContextServerStore::clear_http_credentials(&url, cx).await?;
        }
        Ok(())
    })
}
