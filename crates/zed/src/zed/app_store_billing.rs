use anyhow::{Context as _, Result, bail};
use client::{Client, ClientSettings};
use futures::AsyncReadExt as _;
use gpui::{App, AsyncWindowContext, PromptLevel, TaskExt as _};
use http_client::{AsyncBody, HttpClient as _, Method, Request};
use serde_json::Value;
use settings::Settings as _;
use std::sync::Arc;
use workspace::with_active_or_new_workspace;

const APPLE_SUBSCRIPTIONS_URL: &str = "https://apps.apple.com/account/subscriptions";
pub(super) fn request_account_deletion(cx: &mut App) {
    with_active_or_new_workspace(cx, |_, window, cx| {
        let client = Client::global(cx);
        let token = client.account_access_token();
        let server_url = ClientSettings::get_global(cx).server_url.clone();
        cx.spawn_in(window, async move |_, cx| {
            if let Err(error) = confirm_and_delete_account(client, token, server_url, cx).await {
                show_message(cx, "Delete Fanta account", &format!("{error:#}")).await?;
            }
            anyhow::Ok(())
        })
        .detach_and_log_err(cx);
    });
}

async fn confirm_and_delete_account(
    client: Arc<Client>,
    token: Option<Arc<str>>,
    server_url: String,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    let token =
        token.context("Sign in to Fanta in the Create panel before deleting your account.")?;
    let user = fetch_backend_user(&client, &token, &server_url).await?;
    let account = user.email.as_deref().unwrap_or(&user.id);
    let description = format!(
        "Deleting {account} permanently removes your Fanta account, personal cloud designs and unused credits, and ends your access to shared organizations. This does not cancel an Apple subscription: Apple can keep charging you after your Fanta access ends. Cancel any Fanta subscription in your Apple Account first."
    );
    let response = cx.update(|window, cx| {
        window.prompt(
            PromptLevel::Critical,
            "Delete Fanta account?",
            Some(&description),
            &["Keep account", "Manage subscriptions", "Delete account"],
            cx,
        )
    })?;
    match response.await? {
        0 => return Ok(()),
        1 => {
            cx.update(|_, cx| cx.open_url(APPLE_SUBSCRIPTIONS_URL))?;
            return Ok(());
        }
        2 => {}
        _ => bail!("The account deletion choice was not recognized."),
    }

    let current_token = cx.update(|_, cx| Client::global(cx).account_access_token())?;
    if current_token.as_deref() != Some(token.as_ref()) {
        bail!("Your Fanta account changed. Open Settings and try again.");
    }

    let request = Request::builder()
        .method(Method::DELETE)
        .uri(format!("{}/v1/me", server_url.trim_end_matches('/')))
        .header("Accept", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .body(AsyncBody::empty())?;
    let response = client.http_client().send(request).await.context(
        "Could not confirm account deletion. Check your connection, then sign in and check your account before retrying",
    )?;
    let status = response.status();
    if !status.is_success() {
        if status.as_u16() == 401 {
            bail!("Your Fanta sign-in expired. Sign in again and retry account deletion.");
        }
        bail!(
            "Fanta could not delete your account (HTTP {status}). Please retry or contact support."
        );
    }

    client.request_sign_out();
    show_message(
        cx,
        "Fanta account deleted",
        "Your Fanta account was deleted. If you had an Apple subscription, cancel it in your Apple Account to stop future billing.",
    )
    .await?;
    Ok(())
}

struct BackendUser {
    id: String,
    email: Option<String>,
}

async fn fetch_backend_user(client: &Client, token: &str, server_url: &str) -> Result<BackendUser> {
    let request = Request::builder()
        .method(Method::GET)
        .uri(format!("{}/v1/me", server_url.trim_end_matches('/')))
        .header("Accept", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .body(AsyncBody::empty())?;
    let mut response = client
        .http_client()
        .send(request)
        .await
        .context("Could not reach your Fanta account")?;
    let status = response.status();
    let mut body = Vec::new();
    response
        .body_mut()
        .take(1024 * 1024)
        .read_to_end(&mut body)
        .await?;
    if !status.is_success() {
        bail!("Could not verify your Fanta account (HTTP {status}). Sign in again and retry.");
    }
    let profile: Value =
        serde_json::from_slice(&body).context("Your Fanta account response was invalid")?;
    let user_id = profile["user"]["id"]
        .as_str()
        .context("Your Fanta account identity is unavailable")?;
    uuid::Uuid::parse_str(user_id).context("Your Fanta account identity is invalid")?;
    if client.account_access_token().as_deref() != Some(token) {
        bail!("Your Fanta account changed. Open Credits & billing again.");
    }
    Ok(BackendUser {
        id: user_id.to_string(),
        email: profile["user"]["email"].as_str().map(str::to_string),
    })
}

async fn show_message(cx: &mut AsyncWindowContext, title: &str, message: &str) -> Result<()> {
    let response = cx
        .update(|window, cx| window.prompt(PromptLevel::Info, title, Some(message), &["OK"], cx))?;
    response.await?;
    Ok(())
}
