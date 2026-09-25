use anyhow::{Context as _, Result, bail};
use client::{Client, ClientSettings};
use fanta_revenuecat::{Product, RevenueCat};
use futures::AsyncReadExt as _;
use gpui::{App, AsyncWindowContext, PromptLevel, TaskExt as _};
use http_client::{AsyncBody, HttpClient as _, Method, Request};
use serde_json::Value;
use settings::Settings as _;
use std::sync::Arc;
use workspace::with_active_or_new_workspace;

const PRO_MONTHLY: &str = "dev.fanta.Fanta.pro.monthly";
const CREDITS_500: &str = "dev.fanta.Fanta.credits.500";
const TERMS_URL: &str = "https://www.fantaisa.net/terms";
const PRIVACY_URL: &str = "https://www.fantaisa.net/privacy";
const APPLE_SUBSCRIPTIONS_URL: &str = "https://apps.apple.com/account/subscriptions";
const REVENUECAT_PUBLIC_API_KEY: &str = match option_env!("FANTA_REVENUECAT_PUBLIC_API_KEY") {
    Some(key) => key,
    None => "",
};

pub(super) fn open(cx: &mut App) {
    with_active_or_new_workspace(cx, |_, window, cx| {
        let client = Client::global(cx);
        let token = client.account_access_token();
        let server_url = ClientSettings::get_global(cx).server_url.clone();
        cx.spawn_in(window, async move |_, cx| {
            if let Err(error) = show_store(client, token, server_url, cx).await {
                show_message(cx, "Credits & billing", &format!("{error:#}")).await?;
            }
            anyhow::Ok(())
        })
        .detach_and_log_err(cx);
    });
}

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

async fn show_store(
    client: Arc<Client>,
    token: Option<Arc<str>>,
    server_url: String,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    let token = token.context("Sign in to Fanta in the Create panel before purchasing credits.")?;
    if REVENUECAT_PUBLIC_API_KEY.is_empty() {
        bail!("App Store purchases are not configured in this build.");
    }

    let user = fetch_backend_user(&client, &token, &server_url).await?;
    let revenuecat = RevenueCat::get_or_configure(REVENUECAT_PUBLIC_API_KEY, &user.id)
        .await
        .context("Could not connect to the App Store")?;
    let products = match revenuecat.products(&[PRO_MONTHLY, CREDITS_500]).await {
        Ok(products) => products,
        Err(error) => {
            log::warn!("Could not load App Store products: {error}");
            Vec::new()
        }
    };
    let pro = products
        .iter()
        .find(|product| product.identifier == PRO_MONTHLY);
    let credits = products
        .iter()
        .find(|product| product.identifier == CREDITS_500);
    let mut actions = Vec::new();
    if let Some(product) = pro {
        actions.push((
            format!("Pro monthly — {}", product.localized_price),
            Some(PRO_MONTHLY),
        ));
    }
    if let Some(product) = credits {
        actions.push((
            format!("500 credits — {}", product.localized_price),
            Some(CREDITS_500),
        ));
    }
    actions.push(("Redeem offer code (macOS 15+)".to_string(), None));
    actions.push(("Sync Apple purchases".to_string(), None));
    actions.push(("Restore purchases".to_string(), None));
    actions.push(("Terms of Use".to_string(), None));
    actions.push(("Privacy Policy".to_string(), None));
    actions.push(("Cancel".to_string(), None));
    let labels: Vec<&str> = actions.iter().map(|(label, _)| label.as_str()).collect();
    let description = purchase_description(pro, credits);
    let response = cx.update(|window, cx| {
        window.prompt(
            PromptLevel::Info,
            "Credits & billing",
            Some(&description),
            &labels,
            cx,
        )
    })?;
    let selected = response.await?;
    let Some((label, product_id)) = actions.get(selected) else {
        return Ok(());
    };
    if label == "Cancel" {
        return Ok(());
    }
    if label == "Terms of Use" {
        cx.update(|_, cx| cx.open_url(TERMS_URL))?;
        return Ok(());
    }
    if label == "Privacy Policy" {
        cx.update(|_, cx| cx.open_url(PRIVACY_URL))?;
        return Ok(());
    }

    let current_token = cx.update(|_, cx| Client::global(cx).account_access_token())?;
    if current_token.as_deref() != Some(token.as_ref()) {
        bail!("Your Fanta account changed. Reopen this screen and try again.");
    }

    if label == "Redeem offer code (macOS 15+)" {
        match revenuecat.redeem_offer_code().await {
            Ok(_) => {
                let current_token = cx.update(|_, cx| Client::global(cx).account_access_token())?;
                if current_token.as_deref() != Some(token.as_ref()) {
                    bail!(
                        "Your Fanta account changed while Apple's offer code sheet was open. Sign in to the original account to check any purchase."
                    );
                }
                show_message(
                    cx,
                    "Apple purchases checked",
                    "If you redeemed a code, your Fanta balance will update after Apple and Fanta process the purchase. Reopen Credits & billing to sync again if needed.",
                )
                .await?;
            }
            Err(fanta_revenuecat::RevenueCatError::Cancelled) => {}
            Err(error) => return Err(error).context("Could not redeem or sync the offer code"),
        }
        return Ok(());
    }
    if label == "Sync Apple purchases" {
        revenuecat
            .sync_purchases()
            .await
            .context("Could not sync Apple purchases")?;
        let current_token = cx.update(|_, cx| Client::global(cx).account_access_token())?;
        if current_token.as_deref() != Some(token.as_ref()) {
            bail!(
                "Your Fanta account changed while Apple purchases were being checked. Sign in to the original account to see its credits."
            );
        }
        show_message(
            cx,
            "Apple purchases checked",
            "If you redeemed a code in the App Store, your Fanta balance will update after the purchase is processed.",
        )
        .await?;
        return Ok(());
    }

    if let Some(product_id) = product_id {
        match revenuecat.purchase(product_id).await {
            Ok(_) => {
                let current_token = cx.update(|_, cx| Client::global(cx).account_access_token())?;
                let message = if current_token.as_deref() == Some(token.as_ref()) {
                    "Apple confirmed your purchase. Your Fanta credits will appear after the receipt is processed. If they do not appear, use Restore purchases."
                } else {
                    "Apple confirmed your purchase for the Fanta account used at checkout. Sign in to that account to see its credits."
                };
                show_message(cx, "Purchase confirmed", message).await?;
            }
            Err(fanta_revenuecat::RevenueCatError::Cancelled) => {}
            Err(error) => return Err(error).context("The App Store purchase did not complete"),
        }
    } else {
        revenuecat
            .restore()
            .await
            .context("Could not restore purchases")?;
        show_message(
            cx,
            "Purchases restored",
            "Apple purchases were restored. Your Fanta balance will update after processing.",
        )
        .await?;
    }
    Ok(())
}

fn purchase_description(pro: Option<&Product>, credits: Option<&Product>) -> String {
    let mut lines = Vec::new();
    if pro.is_none() && credits.is_none() {
        lines.push("The App Store products are unavailable right now. You can still redeem an offer code or sync an earlier purchase.".into());
    }
    if let Some(product) = pro {
        lines.push(format!(
            "Pro: 3,000 AI credits every month for {}. Automatically renews monthly until canceled in your Apple account.",
            product.localized_price
        ));
    }
    if let Some(product) = credits {
        lines.push(format!(
            "Credit pack: 500 AI credits for {}. One-time purchase; credits are consumed when you generate.",
            product.localized_price
        ));
    }
    lines.push(
        "Payment is handled by Apple. Purchases are linked to your signed-in Fanta account.".into(),
    );
    lines.push(
        "Redeem offer codes inside Fanta while signed in to the account that should receive them. If you redeemed in the App Store, choose Sync Apple purchases."
            .into(),
    );
    lines.join("\n\n")
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
