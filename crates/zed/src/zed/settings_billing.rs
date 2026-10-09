use anyhow::{Context as _, Result, bail, ensure};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use client::Client;
use fanta_gpui::{
    billing::{
        BillingAction, BillingActionState, BillingActions, BillingBalance, BillingLoadState,
        BillingPlan, BillingProvider, BillingSubscription, BillingTransaction, BillingUsage,
        BillingUsageApiKey, BillingUsageCategory, BillingUsageKind, BillingUsagePoint,
        BillingUsageRange, BillingViewData, BillingWorkspace,
    },
    settings::SettingsAccountSummary,
};
use futures::AsyncReadExt as _;
use gpui::{AppContext as _, AsyncWindowContext, SharedString};
use http_client::{AsyncBody, HttpClient as _, Method, Request};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

#[cfg(feature = "mac_app_store")]
use fanta_revenuecat::{CustomerInfo, IntroductoryTrial, Product, RevenueCat, RevenueCatError};

const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;
#[cfg(feature = "mac_app_store")]
const PRO_MONTHLY: &str = "dev.fanta.Fanta.pro.monthly";
#[cfg(feature = "mac_app_store")]
const CREDITS_500: &str = "dev.fanta.Fanta.credits.500";
const APPLE_SUBSCRIPTIONS_URL: &str = "https://apps.apple.com/account/subscriptions";
#[cfg(feature = "mac_app_store")]
const REVENUECAT_PUBLIC_API_KEY: &str = match option_env!("FANTA_REVENUECAT_PUBLIC_API_KEY") {
    Some(key) => key,
    None => "",
};

#[derive(Debug)]
pub(super) enum BillingActionOutcome {
    Refresh { notice: Option<String> },
    NoRefresh { notice: Option<String> },
}

#[derive(Deserialize)]
struct MeResponse {
    user: MeUser,
    org: MeOrg,
    plan: String,
    role: String,
    monthly_credits: i64,
}

#[derive(Deserialize)]
struct MeUser {
    #[cfg(feature = "mac_app_store")]
    id: String,
    email: Option<String>,
    name: Option<String>,
}

#[derive(Deserialize)]
struct MeOrg {
    id: String,
    name: String,
    #[cfg(feature = "mac_app_store")]
    is_personal: Option<bool>,
}

#[derive(Deserialize)]
struct SubscriptionResponse {
    plan: String,
    active: bool,
    subscription: Option<Subscription>,
}

#[derive(Deserialize)]
struct Subscription {
    source: String,
    status: String,
    billing_cycle: String,
    current_period_end: Option<String>,
    cancel_at_period_end: bool,
}

#[derive(Deserialize)]
struct CreditsResponse {
    balance: i64,
    recent: Vec<CreditEvent>,
}

#[derive(Deserialize)]
struct CreditEvent {
    delta: i64,
    reason: String,
    note: Option<String>,
    created_at: String,
}

#[derive(Deserialize)]
struct UsageResponse {
    days: Vec<UsageRow>,
}

#[derive(Deserialize)]
struct UsageRow {
    day: String,
    model: String,
    requests: i64,
    billed_credits: i64,
}

#[derive(Deserialize)]
struct KeyUsageResponse {
    by_key: Vec<KeyUsage>,
}

#[derive(Deserialize)]
struct KeyUsage {
    api_key_id: Option<String>,
    requests: i64,
    billed_credits: i64,
}

#[derive(Deserialize)]
struct KeysResponse {
    keys: Vec<KeySummary>,
}

#[derive(Deserialize)]
struct KeySummary {
    id: String,
    name: String,
    prefix: String,
}

#[derive(Deserialize)]
struct PlansResponse {
    plans: Vec<PlanSummary>,
}

#[derive(Deserialize)]
struct PlanSummary {
    id: String,
    display_name: String,
    description: Option<String>,
    monthly_credits: i64,
    #[cfg(not(feature = "mac_app_store"))]
    price_monthly_cents: i64,
    features: Value,
}

fn ensure_account(client: &Client, token: &str) -> Result<()> {
    ensure!(
        client.account_access_token().as_deref() == Some(token),
        "Your Fanta account changed. Reopen Settings and try again."
    );
    Ok(())
}

async fn api_json(
    client: &Client,
    token: &str,
    server_url: &str,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value> {
    ensure_account(client, token)?;
    let mut request = Request::builder()
        .method(method)
        .uri(format!("{}{path}", server_url.trim_end_matches('/')))
        .header("Accept", "application/json")
        .header("Authorization", format!("Bearer {token}"));
    let body = if let Some(body) = body {
        request = request.header("Content-Type", "application/json");
        AsyncBody::from(serde_json::to_vec(&body)?)
    } else {
        AsyncBody::empty()
    };
    let request = request.body(body)?;
    let mut response = client
        .http_client()
        .send(request)
        .await
        .with_context(|| format!("Could not reach Fanta for {path}"))?;
    let status = response.status();
    let mut bytes = Vec::new();
    response
        .body_mut()
        .take(MAX_RESPONSE_BYTES)
        .read_to_end(&mut bytes)
        .await?;
    ensure_account(client, token)?;
    let value: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("Fanta returned an unreadable response for {path}"))?;
    if !status.is_success() {
        let message = value["error"]["message"]
            .as_str()
            .or_else(|| value["error"].as_str())
            .or_else(|| value["message"].as_str())
            .unwrap_or("Request failed");
        if status.as_u16() == 401 {
            bail!("Your Fanta sign-in expired. Sign in again and retry.");
        }
        bail!("{message} (HTTP {status})");
    }
    Ok(value)
}

async fn get<T: for<'de> Deserialize<'de>>(
    client: &Client,
    token: &str,
    server_url: &str,
    path: &str,
) -> Result<T> {
    serde_json::from_value(api_json(client, token, server_url, Method::GET, path, None).await?)
        .with_context(|| format!("Fanta returned invalid billing data for {path}"))
}

pub(super) async fn load_account_summary(
    client: Arc<Client>,
    token: Arc<str>,
    server_url: String,
) -> Result<SettingsAccountSummary> {
    let me: MeResponse = get(&client, &token, &server_url, "/v1/me").await?;
    Ok(SettingsAccountSummary {
        display_name: me
            .user
            .name
            .unwrap_or_else(|| "Fanta account".into())
            .into(),
        email: me.user.email.unwrap_or_default().into(),
        plan: plan_name(&me.plan).into(),
        workspace: me.org.name.into(),
    })
}

struct UsageWindow {
    range: BillingUsageRange,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
}

impl UsageWindow {
    fn new(range: BillingUsageRange, to: DateTime<Utc>) -> Self {
        Self {
            range,
            from: to - Duration::days(Self::days(range)),
            to,
        }
    }

    fn days(range: BillingUsageRange) -> i64 {
        match range {
            BillingUsageRange::Days7 => 7,
            BillingUsageRange::Days30 => 30,
            BillingUsageRange::Days90 => 90,
        }
    }

    fn path(&self, by_key: bool) -> String {
        let mut path = format!(
            "/v1/usage?from={}&to={}",
            self.from.to_rfc3339_opts(SecondsFormat::Secs, true),
            self.to.to_rfc3339_opts(SecondsFormat::Secs, true)
        );
        if by_key {
            path.push_str("&group_by=key");
        }
        path
    }
}

fn comma(value: i64) -> String {
    let digits = value.unsigned_abs().to_string();
    let mut result = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if value < 0 {
        result.push('-');
    }
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            result.push(',');
        }
        result.push(digit);
    }
    result
}

fn plan_name(id: &str) -> String {
    match id {
        "free" => "Free".into(),
        "pro" => "Pro".into(),
        "team" => "Team".into(),
        _ => id.to_owned(),
    }
}

fn credit_title(reason: &str, delta: i64) -> &'static str {
    match reason {
        "subscription" => "Subscription credits",
        "purchase" => "Credit purchase",
        "promo" => "Promotional credits",
        "refund" => "Credit refund",
        "usage" => "AI usage",
        _ if delta >= 0 => "Credits added",
        _ => "Credits used",
    }
}

fn build_usage(
    window: &UsageWindow,
    rows: Vec<UsageRow>,
    key_rows: Vec<KeyUsage>,
    keys: &[KeySummary],
    balance: i64,
) -> BillingUsage {
    let days = UsageWindow::days(window.range);
    let mut daily = BTreeMap::<String, i64>::new();
    let mut models = BTreeMap::<String, (i64, i64)>::new();
    for row in rows {
        let date = row.day.get(..10).unwrap_or(&row.day).to_owned();
        *daily.entry(date).or_default() += row.billed_credits;
        let item = models.entry(row.model).or_default();
        item.0 += row.billed_credits;
        item.1 += row.requests;
    }
    let total: i64 = daily.values().sum();
    let max_day = daily.values().copied().max().unwrap_or(0).max(1) as f32;
    let trend: Vec<BillingUsagePoint> = if total == 0 {
        Vec::new()
    } else {
        window
            .from
            .date_naive()
            .iter_days()
            .take_while(|date| *date <= window.to.date_naive())
            .map(|date| {
                let date = date.format("%Y-%m-%d").to_string();
                let amount = daily.get(&date).copied().unwrap_or(0);
                BillingUsagePoint {
                    label: date.get(5..).unwrap_or(&date).to_owned().into(),
                    amount_label: format!("{} credits", comma(amount)).into(),
                    fraction: (amount as f32 / max_day).clamp(0., 1.),
                }
            })
            .collect()
    };
    let categories = models
        .into_iter()
        .map(|(model, (credits, requests))| BillingUsageCategory {
            id: model.clone().into(),
            name: model.into(),
            detail: format!("{} requests", comma(requests)).into(),
            amount_label: format!("{} credits", comma(credits)).into(),
            fraction: if total > 0 {
                (credits as f32 / total as f32).clamp(0., 1.)
            } else {
                0.
            },
            kind: BillingUsageKind::Other,
        })
        .collect();
    let api_keys = key_rows
        .into_iter()
        .map(|key| {
            let name = key
                .api_key_id
                .as_deref()
                .and_then(|id| keys.iter().find(|item| item.id == id))
                .map(|item| (item.name.clone(), item.prefix.clone()))
                .unwrap_or_else(|| ("Desktop and other usage".into(), "No named API key".into()));
            BillingUsageApiKey {
                id: key.api_key_id.unwrap_or_default().into(),
                name: name.0.into(),
                detail: format!("{} · {} requests", name.1, comma(key.requests)).into(),
                amount_label: format!("{} credits", comma(key.billed_credits)).into(),
                fraction: if total > 0 {
                    (key.billed_credits as f32 / total as f32).clamp(0., 1.)
                } else {
                    0.
                },
            }
        })
        .collect();
    BillingUsage {
        range: window.range,
        period_label: format!("Last {days} days").into(),
        total_label: format!("{} credits used", comma(total)).into(),
        allowance_label: "Rolling period · first and last UTC days may be partial".into(),
        reset_label: SharedString::default(),
        allowance_fraction: if total > 0 {
            (total as f32 / (total + balance.max(0)) as f32).clamp(0., 1.)
        } else {
            0.
        },
        trend,
        categories,
        api_keys,
        note: "Credits are a shared balance. Grants and top-ups add to it; usage reduces it."
            .into(),
    }
}

#[cfg(feature = "mac_app_store")]
fn apple_pro_price_label(product: &Product) -> String {
    if let Some(trial) = &product.eligible_introductory_trial {
        format!(
            "{} free, then {}",
            trial.duration_label(),
            product.localized_price
        )
    } else {
        product.localized_price.clone()
    }
}

#[cfg(feature = "mac_app_store")]
fn apple_trial_description(
    trial: &IntroductoryTrial,
    localized_price: &str,
    monthly_credits: i64,
) -> String {
    format!(
        "Access Pro features during your free trial of {}. The trial adds no subscription credits; you can use credits already in your account. After the trial, Apple charges {localized_price} per month and you receive {} AI credits for each paid month. The subscription automatically renews monthly until canceled in your Apple Account. Cancel before the trial ends to avoid the first charge.",
        trial.duration_label(),
        comma(monthly_credits)
    )
}

pub(super) async fn load_billing_snapshot(
    client: Arc<Client>,
    token: Arc<str>,
    server_url: String,
    range: BillingUsageRange,
) -> Result<BillingViewData> {
    let usage_window = UsageWindow::new(range, Utc::now());
    let usage_request_path = usage_window.path(false);
    let key_usage_path = usage_window.path(true);
    let me: MeResponse = get(&client, &token, &server_url, "/v1/me").await?;
    let credits: CreditsResponse = get(&client, &token, &server_url, "/v1/credits").await?;
    let subscription: SubscriptionResponse =
        get(&client, &token, &server_url, "/v1/billing/subscription").await?;
    let usage: UsageResponse = get(&client, &token, &server_url, &usage_request_path).await?;
    let key_usage: KeyUsageResponse = get(&client, &token, &server_url, &key_usage_path).await?;
    let keys: KeysResponse = get(&client, &token, &server_url, "/v1/keys").await?;
    let plans: PlansResponse = get(&client, &token, &server_url, "/v1/plans").await?;
    ensure_account(&client, &token)?;
    let can_manage = me.role == "owner" || me.role == "admin";
    #[cfg(feature = "mac_app_store")]
    let apple_account_ready = me.org.is_personal == Some(true);
    #[cfg(feature = "mac_app_store")]
    let account_note = if apple_account_ready {
        "Apple purchases and grants belong to this personal Fanta workspace."
    } else {
        "Apple purchases belong to your personal Fanta workspace. Sign out and back in to connect it before buying or restoring."
    };
    #[cfg(not(feature = "mac_app_store"))]
    let account_note = "Credits are shared by members of this workspace.";
    #[cfg(feature = "mac_app_store")]
    let apple_store_reason = if !apple_account_ready {
        "Connect your personal Fanta workspace: sign out and back in, then refresh billing."
    } else if REVENUECAT_PUBLIC_API_KEY.is_empty() {
        "App Store purchases are not configured in this build."
    } else {
        "Could not load App Store products. Check your connection and retry."
    };
    let provider = match subscription
        .subscription
        .as_ref()
        .map(|item| item.source.as_str())
    {
        Some("apple") => BillingProvider::Apple,
        Some("polar") => BillingProvider::Web,
        _ => BillingProvider::None,
    };
    #[cfg(feature = "mac_app_store")]
    let apple_products = if !apple_account_ready || REVENUECAT_PUBLIC_API_KEY.is_empty() {
        None
    } else {
        match RevenueCat::get_or_configure(REVENUECAT_PUBLIC_API_KEY, &me.user.id).await {
            Ok(revenuecat) => match revenuecat.products(&[PRO_MONTHLY, CREDITS_500]).await {
                Ok(products) => Some(products),
                Err(error) => {
                    log::warn!("Could not load App Store products: {error}");
                    None
                }
            },
            Err(error) => {
                log::warn!("Could not initialize App Store purchases: {error}");
                None
            }
        }
    };
    let subscription_price = if provider == BillingProvider::Apple {
        #[cfg(feature = "mac_app_store")]
        {
            apple_products
                .as_ref()
                .and_then(|products| {
                    products
                        .iter()
                        .find(|product| product.identifier == PRO_MONTHLY)
                })
                .map(|product| product.localized_price.clone())
                .unwrap_or_default()
        }
        #[cfg(not(feature = "mac_app_store"))]
        {
            String::new()
        }
    } else {
        String::new()
    };
    let period_end = subscription
        .subscription
        .as_ref()
        .and_then(|item| item.current_period_end.as_deref())
        .and_then(|date| date.get(..10))
        .map(|date| {
            if subscription
                .subscription
                .as_ref()
                .is_some_and(|item| item.cancel_at_period_end)
            {
                format!("Access through {date}")
            } else {
                format!("Renews on {date}")
            }
        })
        .unwrap_or_default();
    let subscription_view = BillingSubscription {
        plan_name: plan_name(&subscription.plan).into(),
        status_label: subscription
            .subscription
            .as_ref()
            .map_or_else(
                || {
                    if subscription.active {
                        "Active"
                    } else {
                        "No subscription"
                    }
                    .to_owned()
                },
                |item| plan_name(&item.status),
            )
            .into(),
        price_label: subscription_price.into(),
        cadence_label: subscription
            .subscription
            .as_ref()
            .map_or("", |item| item.billing_cycle.as_str())
            .into(),
        renewal_label: period_end.into(),
        provider,
        provider_note: match provider {
            BillingProvider::Apple => "Billed by Apple. Manage renewals in your Apple Account.",
            BillingProvider::Web => {
                "Billed on the web. Open your billing portal to manage renewal."
            }
            BillingProvider::None => "No paid subscription is active for this workspace.",
        }
        .into(),
    };
    let plans = plans
        .plans
        .into_iter()
        .map(|plan| {
            let current = plan.id == me.plan;
            #[cfg(feature = "mac_app_store")]
            let product = apple_products.as_ref().and_then(|products| {
                products
                    .iter()
                    .find(|product| product.identifier == PRO_MONTHLY)
            });
            #[cfg(feature = "mac_app_store")]
            let price_label = if plan.id == "pro" {
                product.map(apple_pro_price_label).unwrap_or_default()
            } else if plan.id == "free" {
                "Free".into()
            } else {
                String::new()
            };
            #[cfg(not(feature = "mac_app_store"))]
            let price_label = if plan.price_monthly_cents == 0 {
                "Free".into()
            } else {
                format!("${:.2}", plan.price_monthly_cents as f64 / 100.)
            };
            #[cfg(feature = "mac_app_store")]
            let action = if current {
                BillingActionState::disabled("Current plan", "This workspace is on this plan.")
            } else if !apple_account_ready {
                BillingActionState::disabled("Connect personal workspace", apple_store_reason)
            } else if plan.id == "pro" && product.is_some() && can_manage {
                BillingActionState::enabled(
                    if product.is_some_and(|product| product.eligible_introductory_trial.is_some())
                    {
                        "Start free trial"
                    } else {
                        "Choose Pro"
                    },
                )
            } else {
                BillingActionState::disabled(
                    "Unavailable",
                    "This plan cannot be purchased in this App Store build.",
                )
            };
            #[cfg(not(feature = "mac_app_store"))]
            let action = if current {
                BillingActionState::disabled("Current plan", "This workspace is on this plan.")
            } else if can_manage {
                BillingActionState::enabled("Choose plan")
            } else {
                BillingActionState::disabled(
                    "Owner or admin required",
                    "Ask a workspace owner or admin to manage billing.",
                )
            };
            let description = plan.description.unwrap_or_default();
            #[cfg(feature = "mac_app_store")]
            let description = if plan.id == "pro" {
                product
                    .and_then(|product| {
                        product.eligible_introductory_trial.as_ref().map(|trial| {
                            apple_trial_description(
                                trial,
                                &product.localized_price,
                                plan.monthly_credits,
                            )
                        })
                    })
                    .unwrap_or(description)
            } else {
                description
            };
            let credits_label = if plan.id == "free" {
                format!("{} credits on signup", comma(plan.monthly_credits))
            } else {
                format!("{} credits / month", comma(plan.monthly_credits))
            };
            #[cfg(feature = "mac_app_store")]
            let credits_label = if plan.id == "pro"
                && product.is_some_and(|product| product.eligible_introductory_trial.is_some())
            {
                format!("{} credits per paid month", comma(plan.monthly_credits))
            } else {
                credits_label
            };
            BillingPlan {
                id: plan.id.clone().into(),
                name: plan.display_name.into(),
                eyebrow: if current {
                    "YOUR PLAN"
                } else {
                    "AVAILABLE PLAN"
                }
                .into(),
                description: description.into(),
                price_label: price_label.into(),
                cadence_label: if plan.id == "free" { "" } else { "/ month" }.into(),
                credits_label: credits_label.into(),
                features: plan
                    .features
                    .as_object()
                    .map(|features| {
                        features
                            .iter()
                            .filter_map(|(key, value)| {
                                value
                                    .as_bool()
                                    .filter(|enabled| *enabled)
                                    .map(|_| key.replace('_', " ").into())
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                highlighted: plan.id == "pro",
                current,
                action,
            }
        })
        .collect();
    let transactions = credits
        .recent
        .iter()
        .enumerate()
        .map(|(index, event)| BillingTransaction {
            id: format!("ledger-{index}").into(),
            title: credit_title(&event.reason, event.delta).into(),
            detail: event
                .note
                .clone()
                .unwrap_or_else(|| event.reason.clone())
                .into(),
            date_label: event
                .created_at
                .get(..10)
                .unwrap_or(&event.created_at)
                .to_owned()
                .into(),
            credits_label: format!(
                "{}{} credits",
                if event.delta > 0 { "+" } else { "" },
                comma(event.delta)
            )
            .into(),
            amount_label: SharedString::default(),
            status_label: "Recorded".into(),
            receipt_action: None,
        })
        .collect();
    let usage = build_usage(
        &usage_window,
        usage.days,
        key_usage.by_key,
        &keys.keys,
        credits.balance,
    );
    #[cfg(feature = "mac_app_store")]
    let actions = {
        let pack = apple_products.as_ref().and_then(|products| {
            products
                .iter()
                .find(|product| product.identifier == CREDITS_500)
        });
        let store_reason = apple_store_reason;
        BillingActions {
            add_credits: if !apple_account_ready {
                BillingActionState::disabled("Add credits", store_reason)
            } else if can_manage {
                pack.map_or_else(
                    || BillingActionState::disabled("Add credits", store_reason),
                    |item| {
                        BillingActionState::enabled(format!(
                            "Buy 500 for personal workspace · {}",
                            item.localized_price
                        ))
                    },
                )
            } else {
                BillingActionState::disabled("Add credits", "Owner or admin required")
            },
            manage_subscription: match provider {
                BillingProvider::Apple => BillingActionState::enabled("Manage with Apple"),
                BillingProvider::Web if can_manage => BillingActionState::enabled("Manage on web"),
                _ => BillingActionState::disabled(
                    "Manage subscription",
                    "No subscription or permission for this workspace",
                ),
            },
            restore_purchases: if !apple_account_ready || REVENUECAT_PUBLIC_API_KEY.is_empty() {
                BillingActionState::disabled("Restore purchases", store_reason)
            } else {
                BillingActionState::enabled("Restore purchases")
            },
            sync_purchases: if !apple_account_ready || REVENUECAT_PUBLIC_API_KEY.is_empty() {
                BillingActionState::disabled("Sync purchases", store_reason)
            } else {
                BillingActionState::enabled("Sync purchases")
            },
            redeem_code: if !apple_account_ready || REVENUECAT_PUBLIC_API_KEY.is_empty() {
                BillingActionState::disabled("Redeem code", store_reason)
            } else {
                BillingActionState::enabled("Redeem code")
            },
            export_usage: BillingActionState::enabled("Export CSV"),
            refresh: BillingActionState::enabled("Refresh"),
        }
    };
    #[cfg(not(feature = "mac_app_store"))]
    let actions = BillingActions {
        add_credits: if can_manage {
            BillingActionState::enabled("Add credits")
        } else {
            BillingActionState::disabled("Add credits", "Owner or admin required")
        },
        manage_subscription: match provider {
            BillingProvider::Apple => BillingActionState::enabled("Manage with Apple"),
            BillingProvider::Web if can_manage => BillingActionState::enabled("Manage billing"),
            _ => BillingActionState::disabled(
                "Manage billing",
                "No subscription or permission for this workspace",
            ),
        },
        restore_purchases: BillingActionState::disabled(
            "Restore purchases",
            "Apple purchases are available in the Mac App Store build.",
        ),
        sync_purchases: BillingActionState::disabled(
            "Sync purchases",
            "Apple purchases are available in the Mac App Store build.",
        ),
        redeem_code: BillingActionState::disabled(
            "Redeem code",
            "Apple purchases are available in the Mac App Store build.",
        ),
        export_usage: BillingActionState::enabled("Export CSV"),
        refresh: BillingActionState::enabled("Refresh"),
    };
    Ok(BillingViewData {
        workspace: BillingWorkspace {
            id: me.org.id.into(),
            name: me.org.name.into(),
            role_label: plan_name(&me.role).into(),
        },
        load_state: BillingLoadState::Ready,
        balance: BillingBalance {
            available_label: comma(credits.balance).into(),
            available_caption: "credits available".into(),
            included_label: if me.plan == "free" {
                format!("{} once", comma(me.monthly_credits))
            } else {
                format!("{} / month", comma(me.monthly_credits))
            }
            .into(),
            purchased_label: "See activity".into(),
            refresh_label: "Live workspace balance".into(),
            note: account_note.into(),
        },
        subscription: subscription_view,
        plans,
        usage,
        transactions,
        actions,
        ..BillingViewData::default()
    })
}

#[cfg(feature = "mac_app_store")]
async fn apple_client(
    client: &Client,
    token: &str,
    server_url: &str,
) -> Result<(RevenueCat, MeResponse)> {
    ensure!(
        !REVENUECAT_PUBLIC_API_KEY.is_empty(),
        "App Store purchases are not configured in this build."
    );
    let me: MeResponse = get(client, token, server_url, "/v1/me").await?;
    ensure!(
        me.org.is_personal == Some(true),
        "Apple purchases belong to your personal Fanta workspace. Sign out and back in before buying or restoring."
    );
    let revenuecat = RevenueCat::get_or_configure(REVENUECAT_PUBLIC_API_KEY, &me.user.id)
        .await
        .context("Could not connect to the App Store")?;
    ensure_account(client, token)?;
    Ok((revenuecat, me))
}

#[cfg(feature = "mac_app_store")]
fn checked_customer(customer: &CustomerInfo, user_id: &str) -> Result<bool> {
    ensure!(
        customer.app_user_id == user_id,
        "Apple purchases belong to a different Fanta account. Sign in to the original account and retry."
    );
    Ok(!customer.active_entitlements.is_empty()
        || !customer.active_subscriptions.is_empty()
        || !customer.purchased_product_identifiers.is_empty())
}

#[cfg(feature = "mac_app_store")]
fn apple_recovery_notice(
    has_purchases: bool,
    apple_subscription_active: bool,
    backend_apple_subscription_active: bool,
    balance: i64,
) -> String {
    if !has_purchases {
        return format!(
            "No restorable Apple subscription was found. Previously granted credit packs remain in your Fanta ledger. Current workspace balance: {} credits.",
            comma(balance)
        );
    }
    if apple_subscription_active && !backend_apple_subscription_active {
        return format!(
            "Apple recognizes an active subscription. Apple grants apply to your personal Fanta workspace, which may differ from this one; check that workspace after processing. Current workspace balance: {} credits.",
            comma(balance)
        );
    }
    format!(
        "Apple purchase history was checked for this Fanta account. This check does not add credits by itself. Apple grants apply to your personal workspace after processing. Current workspace balance: {} credits.",
        comma(balance)
    )
}

#[cfg(feature = "mac_app_store")]
async fn apple_recovery(
    revenuecat: RevenueCat,
    user_id: &str,
    client: &Client,
    token: &str,
    server_url: &str,
    restore: bool,
) -> Result<BillingActionOutcome> {
    let customer = if restore {
        revenuecat.restore().await
    } else {
        revenuecat.sync_purchases().await
    };
    let customer = match customer {
        Ok(customer) => customer,
        Err(RevenueCatError::Cancelled) => {
            return Ok(BillingActionOutcome::NoRefresh { notice: None });
        }
        Err(error) => {
            return Err(error).context(if restore {
                "Could not restore Apple purchases"
            } else {
                "Could not sync Apple purchases"
            });
        }
    };
    ensure_account(client, token)?;
    let has_purchases = checked_customer(&customer, user_id)?;
    let credits: CreditsResponse = get(client, token, server_url, "/v1/credits").await?;
    let subscription: SubscriptionResponse =
        get(client, token, server_url, "/v1/billing/subscription").await?;
    let backend_apple_active = subscription
        .subscription
        .as_ref()
        .is_some_and(|item| item.source == "apple" && item.status == "active");
    let notice = apple_recovery_notice(
        has_purchases,
        !customer.active_subscriptions.is_empty(),
        backend_apple_active,
        credits.balance,
    );
    Ok(BillingActionOutcome::Refresh {
        notice: Some(notice),
    })
}

async fn export_usage(
    client: &Client,
    token: &str,
    server_url: &str,
    range: BillingUsageRange,
    cx: &mut AsyncWindowContext,
) -> Result<BillingActionOutcome> {
    let window = UsageWindow::new(range, Utc::now());
    let usage: UsageResponse = get(client, token, server_url, &window.path(false)).await?;
    let path = cx.update(|_, cx| {
        cx.prompt_for_new_path(
            &PathBuf::from(paths::home_dir().as_path()),
            Some("fanta-ai-usage.csv"),
        )
    })?;
    let Some(path) = path.await?? else {
        return Ok(BillingActionOutcome::NoRefresh { notice: None });
    };
    ensure_account(client, token)?;
    let mut csv = String::from("day,model,requests,credits\n");
    for row in usage.days {
        let escaped_model = format!("\"{}\"", row.model.replace('"', "\"\""));
        csv.push_str(&format!(
            "{},{},{},{}\n",
            row.day.get(..10).unwrap_or(&row.day),
            escaped_model,
            row.requests,
            row.billed_credits
        ));
    }
    let save_path = path.clone();
    cx.background_spawn(async move { std::fs::write(&save_path, csv) })
        .await
        .context("Could not save usage CSV")?;
    #[cfg(all(target_os = "macos", feature = "mac_app_store"))]
    workspace::remember_user_selected_paths(std::slice::from_ref(&path))
        .context("Could not retain access to the saved CSV")?;
    Ok(BillingActionOutcome::NoRefresh {
        notice: Some(format!("Usage exported to {}", path.display())),
    })
}

pub(super) async fn perform_billing_action(
    action: BillingAction,
    client: Arc<Client>,
    token: Arc<str>,
    server_url: String,
    range: BillingUsageRange,
    cx: &mut AsyncWindowContext,
) -> Result<BillingActionOutcome> {
    ensure_account(&client, &token)?;
    match action {
        BillingAction::TabSelected(_) | BillingAction::UsageRangeSelected(_) => {
            Ok(BillingActionOutcome::NoRefresh { notice: None })
        }
        BillingAction::RefreshRequested => Ok(BillingActionOutcome::Refresh { notice: None }),
        BillingAction::ReceiptRequested { .. } => bail!(
            "Receipts are not available from the Fanta credit ledger. Check the payment provider for your receipt."
        ),
        BillingAction::ExportUsageRequested => {
            export_usage(&client, &token, &server_url, range, cx).await
        }
        BillingAction::ManageSubscriptionRequested => {
            let subscription: SubscriptionResponse =
                get(&client, &token, &server_url, "/v1/billing/subscription").await?;
            if subscription
                .subscription
                .as_ref()
                .is_some_and(|item| item.source == "apple")
            {
                cx.update(|_, cx| cx.open_url(APPLE_SUBSCRIPTIONS_URL))?;
                return Ok(BillingActionOutcome::NoRefresh { notice: None });
            }
            let response = api_json(
                &client,
                &token,
                &server_url,
                Method::POST,
                "/v1/billing/portal",
                None,
            )
            .await?;
            let url = response["url"]
                .as_str()
                .context("Fanta did not return a billing portal URL")?;
            cx.update(|_, cx| cx.open_url(url))?;
            Ok(BillingActionOutcome::NoRefresh { notice: None })
        }
        BillingAction::AddCreditsRequested => {
            #[cfg(feature = "mac_app_store")]
            {
                let (revenuecat, me) = apple_client(&client, &token, &server_url).await?;
                let purchase = match revenuecat.purchase(CREDITS_500).await {
                    Ok(purchase) => purchase,
                    Err(RevenueCatError::Cancelled) => {
                        return Ok(BillingActionOutcome::NoRefresh { notice: None });
                    }
                    Err(error) => {
                        return Err(error)
                            .context("The App Store credit purchase did not complete");
                    }
                };
                ensure_account(&client, &token)?;
                checked_customer(&purchase.customer_info, &me.user.id)?;
                return Ok(BillingActionOutcome::Refresh { notice: Some("Apple confirmed your credit purchase. Fanta will add 500 credits to your personal workspace after the purchase is processed.".into()) });
            }
            #[cfg(not(feature = "mac_app_store"))]
            {
                let response = api_json(
                    &client,
                    &token,
                    &server_url,
                    Method::POST,
                    "/v1/billing/credits",
                    Some(serde_json::json!({"pack":"small"})),
                )
                .await?;
                let url = response["url"]
                    .as_str()
                    .context("Fanta did not return a credit checkout URL")?;
                cx.update(|_, cx| cx.open_url(url))?;
                return Ok(BillingActionOutcome::NoRefresh { notice: Some("Complete your credit purchase in the browser, then refresh your balance here.".into()) });
            }
        }
        BillingAction::PlanRequested { plan_id } => {
            #[cfg(feature = "mac_app_store")]
            {
                ensure!(
                    plan_id.as_ref() == "pro",
                    "This plan is not available for purchase in the App Store build."
                );
                let (revenuecat, me) = apple_client(&client, &token, &server_url).await?;
                let purchase = match revenuecat.purchase(PRO_MONTHLY).await {
                    Ok(purchase) => purchase,
                    Err(RevenueCatError::Cancelled) => {
                        return Ok(BillingActionOutcome::NoRefresh { notice: None });
                    }
                    Err(error) => {
                        return Err(error).context("The App Store subscription did not complete");
                    }
                };
                ensure_account(&client, &token)?;
                checked_customer(&purchase.customer_info, &me.user.id)?;
                return Ok(BillingActionOutcome::Refresh { notice: Some("Apple confirmed Pro. Fanta will update your personal workspace after processing. Free trials add no subscription credits; each paid month adds 3,000 credits.".into()) });
            }
            #[cfg(not(feature = "mac_app_store"))]
            {
                let response = api_json(
                    &client,
                    &token,
                    &server_url,
                    Method::POST,
                    "/v1/billing/checkout",
                    Some(serde_json::json!({"plan":plan_id.as_ref(),"cycle":"monthly"})),
                )
                .await?;
                let url = response["url"]
                    .as_str()
                    .context("Fanta did not return a plan checkout URL")?;
                cx.update(|_, cx| cx.open_url(url))?;
                return Ok(BillingActionOutcome::NoRefresh {
                    notice: Some(
                        "Complete your plan checkout in the browser, then refresh billing here."
                            .into(),
                    ),
                });
            }
        }
        BillingAction::RestorePurchasesRequested | BillingAction::SyncPurchasesRequested => {
            #[cfg(feature = "mac_app_store")]
            {
                let (revenuecat, me) = apple_client(&client, &token, &server_url).await?;
                return apple_recovery(
                    revenuecat,
                    &me.user.id,
                    &client,
                    &token,
                    &server_url,
                    matches!(action, BillingAction::RestorePurchasesRequested),
                )
                .await;
            }
            #[cfg(not(feature = "mac_app_store"))]
            {
                bail!("Apple purchases can only be restored in the Mac App Store build.");
            }
        }
        BillingAction::RedeemCodeRequested => {
            #[cfg(feature = "mac_app_store")]
            {
                let (revenuecat, me) = apple_client(&client, &token, &server_url).await?;
                let customer = match revenuecat.redeem_offer_code().await {
                    Ok(customer) => customer,
                    Err(RevenueCatError::Cancelled) => {
                        return Ok(BillingActionOutcome::NoRefresh { notice: None });
                    }
                    Err(error) => {
                        return Err(error).context("Could not redeem or sync the Apple offer code");
                    }
                };
                ensure_account(&client, &token)?;
                checked_customer(&customer, &me.user.id)?;
                return Ok(BillingActionOutcome::Refresh { notice: Some("Apple checked the offer code. Fanta credits will update after processing if the redemption succeeded.".into()) });
            }
            #[cfg(not(feature = "mac_app_store"))]
            {
                bail!("Apple offer codes are only available in the Mac App Store build.");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[cfg(feature = "mac_app_store")]
    #[test]
    fn eligible_trial_shows_duration_price_and_paid_credit_timing() {
        let trial = IntroductoryTrial {
            duration: 3,
            unit: fanta_revenuecat::TrialUnit::Day,
        };
        let product = Product {
            identifier: PRO_MONTHLY.into(),
            title: "Fanta Pro".into(),
            description: String::new(),
            localized_price: "$44.99".into(),
            currency_code: "USD".into(),
            eligible_introductory_trial: Some(trial.clone()),
        };
        assert_eq!(apple_pro_price_label(&product), "3 days free, then $44.99");
        let description = apple_trial_description(&trial, &product.localized_price, 3000);
        assert!(description.contains("free trial of 3 days"));
        assert!(description.contains("trial adds no subscription credits"));
        assert!(description.contains("3,000 AI credits for each paid month"));
        assert!(description.contains("Cancel before the trial ends"));
        let product = Product {
            eligible_introductory_trial: None,
            ..product
        };
        assert_eq!(apple_pro_price_label(&product), "$44.99");
    }

    #[test]
    fn desktop_api_key_responses_match_billing_contract() -> Result<()> {
        let me: MeResponse = serde_json::from_value(json!({
            "user": {"id": "34bea1ad-ae2c-4f47-b9cb-2fe78cd5e720", "email": "ada@example.com", "name": null},
            "org": {"id": "c55f585d-a9ee-4334-8f6c-8347bf8d6552", "name": "Design Team", "plan": "pro"},
            "plan": "pro", "role": "owner", "features": {}, "monthly_credits": 3000,
            "key": {"prefix": "fnt_live_abcd", "created_at": "2026-09-01T00:00:00Z"}
        }))?;
        assert_eq!(me.org.name, "Design Team");
        assert_eq!(me.user.email.as_deref(), Some("ada@example.com"));
        #[cfg(feature = "mac_app_store")]
        assert_eq!(me.org.is_personal, None);
        let credits: CreditsResponse = serde_json::from_value(json!({
            "balance": 2480,
            "currency": "credits",
            "recent": [{"delta": 500, "reason": "purchase", "note": "Apple purchase 123", "created_at": "2026-09-28T12:00:00Z"}]
        }))?;
        assert_eq!(credits.balance, 2480);
        assert_eq!(credits.recent[0].delta, 500);
        let plans: PlansResponse = serde_json::from_value(json!({
            "plans": [{"id": "free", "display_name": "Free", "description": null,
                       "price_monthly_cents": 0, "monthly_credits": 500, "features": {}}]
        }))?;
        assert_eq!(plans.plans[0].description, None);
        Ok(())
    }

    #[test]
    fn usage_rollup_uses_desktop_daily_model_and_key_shapes() -> Result<()> {
        let usage: UsageResponse = serde_json::from_value(json!({
            "from": "2026-09-01T00:00:00Z", "to": "2026-09-28T00:00:00Z",
            "days": [
                {"day": "2026-09-27 00:00:00+00", "model": "fanta-image-1", "requests": 2,
                 "input_tokens": 0, "output_tokens": 0, "outputs": 2, "billed_credits": 30},
                {"day": "2026-09-27 00:00:00+00", "model": "claude-sonnet", "requests": 3,
                 "input_tokens": 100, "output_tokens": 40, "outputs": 0, "billed_credits": 12}
            ]
        }))?;
        let keys: KeyUsageResponse = serde_json::from_value(json!({
            "from": "2026-09-01T00:00:00Z", "to": "2026-09-28T00:00:00Z",
            "by_key": [{"api_key_id": null, "requests": 5, "billed_credits": 42}]
        }))?;
        let window = UsageWindow::new(BillingUsageRange::Days30, "2026-09-28T00:00:00Z".parse()?);
        let view = build_usage(&window, usage.days, keys.by_key, &[], 458);
        assert_eq!(view.total_label.as_ref(), "42 credits used");
        assert_eq!(view.categories.len(), 2);
        assert_eq!(view.api_keys[0].name.as_ref(), "Desktop and other usage");
        assert!(view.reset_label.is_empty());
        let empty = build_usage(&window, Vec::new(), Vec::new(), &[], 500);
        assert!(empty.trend.is_empty());
        Ok(())
    }

    #[test]
    fn usage_window_shares_exact_bounds_for_daily_and_key_queries() -> Result<()> {
        for (range, from) in [
            (BillingUsageRange::Days7, "2025-03-25T12:34:56Z"),
            (BillingUsageRange::Days30, "2025-03-02T12:34:56Z"),
            (BillingUsageRange::Days90, "2025-01-01T12:34:56Z"),
        ] {
            let window = UsageWindow::new(range, "2025-04-01T12:34:56Z".parse()?);
            let path = format!("/v1/usage?from={from}&to=2025-04-01T12:34:56Z");
            assert_eq!(window.path(false), path);
            assert_eq!(window.path(true), format!("{path}&group_by=key"));
        }
        Ok(())
    }

    #[test]
    fn usage_rollup_includes_partial_boundary_days_and_empty_middle_days() -> Result<()> {
        for (range, days, first_day) in [
            (BillingUsageRange::Days7, 7, "2025-03-25"),
            (BillingUsageRange::Days30, 30, "2025-03-02"),
            (BillingUsageRange::Days90, 90, "2025-01-01"),
        ] {
            let window = UsageWindow::new(range, "2025-04-01T12:34:56Z".parse()?);
            let rows = vec![
                UsageRow {
                    day: format!("{first_day} 00:00:00+00"),
                    model: "example-a".into(),
                    requests: 1,
                    billed_credits: 2,
                },
                UsageRow {
                    day: format!("{first_day} 00:00:00+00"),
                    model: "example-b".into(),
                    requests: 1,
                    billed_credits: 3,
                },
                UsageRow {
                    day: "2025-04-01 00:00:00+00".into(),
                    model: "example-a".into(),
                    requests: 1,
                    billed_credits: 7,
                },
            ];
            let view = build_usage(
                &window,
                rows,
                vec![KeyUsage {
                    api_key_id: None,
                    requests: 3,
                    billed_credits: 12,
                }],
                &[],
                100,
            );
            assert_eq!(view.trend.len(), days + 1);
            assert_eq!(
                view.trend.first().context("first day")?.label.as_ref(),
                first_day.get(5..).context("day label")?
            );
            assert_eq!(
                view.trend.last().context("last day")?.label.as_ref(),
                "04-01"
            );
            assert_eq!(view.total_label.as_ref(), "12 credits used");
            assert_eq!(view.period_label.as_ref(), format!("Last {days} days"));
            let mut amounts = Vec::new();
            for (index, point) in view.trend.iter().enumerate() {
                let expected = if index == 0 {
                    5
                } else if index == days {
                    7
                } else {
                    0
                };
                assert_eq!(point.amount_label.as_ref(), format!("{expected} credits"));
                assert_eq!(point.fraction, expected as f32 / 7.0);
                amounts.push(
                    point
                        .amount_label
                        .strip_suffix(" credits")
                        .context("credit suffix")?
                        .parse::<i64>()?,
                );
            }
            assert_eq!(amounts.iter().sum::<i64>(), 12);
            let category_amounts = view
                .categories
                .iter()
                .map(|category| (category.name.as_ref(), category.amount_label.as_ref()))
                .collect::<Vec<_>>();
            assert_eq!(
                category_amounts,
                vec![("example-a", "9 credits"), ("example-b", "3 credits")]
            );
            assert_eq!(
                view.api_keys
                    .first()
                    .context("usage key")?
                    .amount_label
                    .as_ref(),
                "12 credits"
            );
        }
        Ok(())
    }

    #[test]
    fn usage_rollup_keeps_the_request_window_across_midnight() -> Result<()> {
        for (to, first_day, last_day) in [
            ("2025-03-31T23:59:59Z", "2025-03-24", "03-31"),
            ("2025-04-01T00:00:00Z", "2025-03-25", "04-01"),
        ] {
            let window = UsageWindow::new(BillingUsageRange::Days7, to.parse()?);
            let view = build_usage(
                &window,
                vec![UsageRow {
                    day: format!("{first_day} 00:00:00+00"),
                    model: "example-a".into(),
                    requests: 1,
                    billed_credits: 4,
                }],
                Vec::new(),
                &[],
                100,
            );
            assert_eq!(view.trend.len(), 8);
            assert_eq!(
                view.trend.first().context("first day")?.label.as_ref(),
                first_day.get(5..).context("day label")?
            );
            assert_eq!(
                view.trend
                    .first()
                    .context("first day")?
                    .amount_label
                    .as_ref(),
                "4 credits"
            );
            assert_eq!(
                view.trend.last().context("last day")?.label.as_ref(),
                last_day
            );
            assert_eq!(view.total_label.as_ref(), "4 credits used");
            let empty = build_usage(&window, Vec::new(), Vec::new(), &[], 100);
            assert!(empty.trend.is_empty());
            assert_eq!(empty.total_label.as_ref(), "0 credits used");
        }
        Ok(())
    }

    #[cfg(feature = "mac_app_store")]
    #[test]
    fn restore_notice_does_not_claim_consumables_were_restored() {
        let empty = apple_recovery_notice(false, false, false, 500);
        assert!(empty.contains("No restorable Apple subscription"));
        assert!(empty.contains("credit packs remain in your Fanta ledger"));
        let waiting = apple_recovery_notice(true, true, false, 500);
        assert!(waiting.contains("Apple recognizes an active subscription"));
        assert!(waiting.contains("personal Fanta workspace"));
        assert!(!waiting.contains("restored"));
        let found = apple_recovery_notice(true, true, true, 3500);
        assert!(found.contains("does not add credits by itself"));
    }
}
