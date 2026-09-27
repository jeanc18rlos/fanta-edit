use anyhow::{Context as _, Result, bail};
use chrono::{Duration, NaiveDate, Utc};
use client::{Client, ClientSettings};
use futures::AsyncReadExt as _;
use gpui::{Context, Task};
use http_client::{AsyncBody, HttpClient as _, Method, Request};
use serde::Deserialize;
use settings::Settings as _;
use ui::prelude::*;

const DASHBOARD_URL: &str = "https://app.fantaisa.net/open";
const APPLE_PURCHASE_HISTORY_URL: &str = "https://reportaproblem.apple.com/";
const APPLE_SUBSCRIPTIONS_URL: &str = "https://apps.apple.com/account/subscriptions";

#[derive(Clone, Deserialize)]
struct Organization {
    id: String,
    name: String,
    role: String,
    is_personal: bool,
}

#[derive(Clone, Deserialize)]
struct Plan {
    display_name: String,
    monthly_credits: i64,
    grant_cadence: String,
}

#[derive(Clone, Deserialize)]
struct Credits {
    available: i64,
}

#[derive(Clone, Deserialize)]
struct UsageSummary {
    credits: i64,
    requests: i64,
}

#[derive(Clone, Deserialize)]
struct Subscription {
    source: String,
    status: String,
    billing_cycle: String,
    current_period_end: Option<String>,
    cancel_at_period_end: bool,
}

#[cfg(not(feature = "mac_app_store"))]
#[derive(Clone, Deserialize)]
struct BillingActions {
    can_start_checkout: bool,
    #[serde(default)]
    available_monthly_plan_ids: Vec<String>,
    can_buy_credits: bool,
    can_open_portal: bool,
    can_change_plan: bool,
}

#[derive(Clone, Deserialize)]
struct AccountOverview {
    organization: Organization,
    plan: Plan,
    credits: Credits,
    usage_30d: UsageSummary,
    subscription: Option<Subscription>,
    #[cfg(not(feature = "mac_app_store"))]
    billing_actions: BillingActions,
}

#[derive(Deserialize)]
struct UsageDay {
    day: String,
    billed_credits: i64,
    requests: i64,
}

#[derive(Deserialize)]
struct UsageResponse {
    days: Vec<UsageDay>,
}

#[derive(Clone)]
struct DailyCredits {
    day: NaiveDate,
    credits: i64,
}

#[derive(Clone, Default)]
struct UsageBreakdown {
    seven_days: UsageSummary,
    ninety_days: UsageSummary,
    daily: Vec<DailyCredits>,
}

impl Default for UsageSummary {
    fn default() -> Self {
        Self {
            credits: 0,
            requests: 0,
        }
    }
}

#[derive(Clone)]
struct AccountData {
    overview: AccountOverview,
    usage: Result<UsageBreakdown, String>,
}

enum AccountState {
    SignedOut,
    Loading,
    Error(String),
    Ready(AccountData),
}

pub(super) struct FantaAccount {
    state: AccountState,
    request_task: Option<Task<()>>,
    #[cfg(not(feature = "mac_app_store"))]
    _action_task: Option<Task<()>>,
    action_error: Option<String>,
    action_pending: bool,
}

impl FantaAccount {
    pub(super) fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            state: AccountState::Loading,
            request_task: None,
            #[cfg(not(feature = "mac_app_store"))]
            _action_task: None,
            action_error: None,
            action_pending: false,
        }
    }

    pub(super) fn refresh(&mut self, cx: &mut Context<Self>) {
        let client = Client::global(cx);
        let Some(token) = client.account_access_token() else {
            self.request_task = None;
            self.state = AccountState::SignedOut;
            cx.notify();
            return;
        };
        let server_url = ClientSettings::get_global(cx).server_url.clone();
        self.state = AccountState::Loading;
        self.action_error = None;
        cx.notify();
        self.request_task = Some(cx.spawn(async move |this, cx| {
            let result = fetch_account(&client, &token, &server_url).await;
            let current_token = client.account_access_token();
            if let Err(error) = this.update(cx, |this, cx| {
                this.state = if current_token.as_deref() != Some(token.as_ref()) {
                    AccountState::SignedOut
                } else {
                    match result {
                        Ok(data) => AccountState::Ready(data),
                        Err(error) => AccountState::Error(format!("{error:#}")),
                    }
                };
                cx.notify();
            }) {
                log::debug!("Fanta account view closed during refresh: {error}");
            }
        }));
    }

    fn open_dashboard(&mut self, section: &'static str, cx: &mut Context<Self>) {
        let AccountState::Ready(data) = &self.state else {
            return;
        };
        let Ok(organization_id) = uuid::Uuid::parse_str(&data.overview.organization.id) else {
            log::error!("Fanta returned an invalid organization id");
            self.action_error = Some(
                "Fanta returned an invalid organization. Refresh your account and retry.".into(),
            );
            cx.notify();
            return;
        };
        let mut url = match url::Url::parse(DASHBOARD_URL) {
            Ok(url) => url,
            Err(error) => {
                log::error!("Invalid Fanta dashboard URL: {error}");
                self.action_error =
                    Some("The Fanta dashboard link is unavailable. Contact support.".into());
                cx.notify();
                return;
            }
        };
        url.query_pairs_mut()
            .append_pair("org_id", &organization_id.to_string())
            .append_pair("section", section);
        cx.open_url(url.as_str());
    }

    #[cfg(not(feature = "mac_app_store"))]
    fn start_billing_action(
        &mut self,
        path: &'static str,
        body: Option<serde_json::Value>,
        cx: &mut Context<Self>,
    ) {
        if self.action_pending {
            return;
        }
        let client = Client::global(cx);
        let Some(token) = client.account_access_token() else {
            self.state = AccountState::SignedOut;
            cx.notify();
            return;
        };
        let server_url = ClientSettings::get_global(cx).server_url.clone();
        self.action_pending = true;
        self.action_error = None;
        cx.notify();
        self._action_task = Some(cx.spawn(async move |this, cx| {
            let result = fetch_billing_url(&client, &token, &server_url, path, body).await;
            let current_token = client.account_access_token();
            if let Err(error) = this.update(cx, |this, cx| {
                this.action_pending = false;
                if current_token.as_deref() != Some(token.as_ref()) {
                    this.state = AccountState::SignedOut;
                    this.action_error = Some(
                        "Your Fanta sign-in changed. Sign in again before managing billing.".into(),
                    );
                } else {
                    match result {
                        Ok(url) => cx.open_url(&url),
                        Err(error) => this.action_error = Some(format!("{error:#}")),
                    }
                }
                cx.notify();
            }) {
                log::debug!("Fanta account view closed during billing action: {error}");
            }
        }));
    }

    fn section_title(title: &'static str, description: &'static str) -> impl IntoElement {
        v_flex()
            .gap_1()
            .child(Label::new(title).size(LabelSize::Large))
            .child(
                Label::new(description)
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
    }

    fn account_card(cx: &App) -> Div {
        v_flex()
            .w_full()
            .gap_3()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().colors().border_variant)
            .bg(cx.theme().colors().surface_background)
    }

    fn usage_line(label: &'static str, summary: &UsageSummary) -> impl IntoElement {
        h_flex()
            .w_full()
            .justify_between()
            .gap_3()
            .child(Label::new(label).color(Color::Muted))
            .child(Label::new(format!(
                "{} credits · {} requests",
                summary.credits, summary.requests
            )))
    }

    fn daily_usage_chart(daily: &[DailyCredits], cx: &App) -> impl IntoElement {
        let maximum = daily
            .iter()
            .map(|day| day.credits)
            .max()
            .unwrap_or(0)
            .max(1);
        v_flex()
            .w_full()
            .gap_2()
            .child(
                Label::new("Daily spend · last 7 days")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .children(daily.iter().map(|day| {
                let fraction = (day.credits.max(0) as f32 / maximum as f32).clamp(0.0, 1.0);
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        Label::new(day.day.format("%a").to_string())
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h_2()
                            .rounded_full()
                            .bg(cx.theme().colors().border_variant)
                            .child(
                                div()
                                    .h_full()
                                    .w(relative(fraction))
                                    .rounded_full()
                                    .bg(cx.theme().colors().text_accent),
                            ),
                    )
                    .child(Label::new(day.credits.to_string()).size(LabelSize::Small))
            }))
    }

    fn render_ready(&self, data: &AccountData, cx: &mut Context<Self>) -> impl IntoElement {
        let overview = &data.overview;
        let cadence = match overview.plan.grant_cadence.as_str() {
            "one_time" => format!("{} one-time credits", overview.plan.monthly_credits),
            "monthly" => format!("{} credits granted monthly", overview.plan.monthly_credits),
            other => format!("{} credits · {other}", overview.plan.monthly_credits),
        };
        let available = overview.credits.available;
        let subscription_description = match &overview.subscription {
            Some(subscription) => {
                let provider = if subscription.source == "apple" {
                    "Apple"
                } else {
                    "Polar"
                };
                let renewal = if subscription.cancel_at_period_end {
                    "Ends"
                } else {
                    "Renews"
                };
                match &subscription.current_period_end {
                    Some(end) => format!(
                        "{} {} · {} · {renewal} {}",
                        provider,
                        subscription.status,
                        subscription.billing_cycle,
                        end.get(..10).unwrap_or(end)
                    ),
                    None => format!(
                        "{} {} · {}",
                        provider, subscription.status, subscription.billing_cycle
                    ),
                }
            }
            None => "No active subscription".to_string(),
        };

        let mut billing = Self::account_card(cx)
            .child(Self::section_title(
                "Billing",
                "Manage your plan and see receipts through your payment provider.",
            ))
            .child(Label::new(subscription_description));

        #[cfg(feature = "mac_app_store")]
        {
            if !overview.organization.is_personal {
                billing = billing.child(Label::new("Apple purchases belong to your personal organization. Sign in with its editor key before buying or restoring.").color(Color::Muted));
            } else if overview
                .subscription
                .as_ref()
                .is_some_and(|subscription| subscription.source == "polar")
            {
                billing = billing.child(Label::new("This organization has a Polar subscription. Manage it on the web before purchasing through Apple.").color(Color::Muted));
            } else {
                billing = billing.child(
                    Button::new("apple-billing-store", "Buy or restore with Apple")
                        .on_click(|_, _, cx| super::app_store_billing::open(cx)),
                );
            }
            billing = billing
                .child(
                    Button::new("apple-subscriptions", "Manage Apple subscriptions")
                        .on_click(|_, _, cx| cx.open_url(APPLE_SUBSCRIPTIONS_URL)),
                )
                .child(
                    Button::new("apple-history", "Apple purchase history")
                        .on_click(|_, _, cx| cx.open_url(APPLE_PURCHASE_HISTORY_URL)),
                );
        }

        #[cfg(not(feature = "mac_app_store"))]
        {
            if overview
                .subscription
                .as_ref()
                .is_some_and(|subscription| subscription.source == "apple")
            {
                billing = billing
                    .child(
                        Button::new("apple-subscriptions", "Manage Apple subscriptions")
                            .on_click(|_, _, cx| cx.open_url(APPLE_SUBSCRIPTIONS_URL)),
                    )
                    .child(
                        Button::new("apple-history", "Apple purchase history")
                            .on_click(|_, _, cx| cx.open_url(APPLE_PURCHASE_HISTORY_URL)),
                    );
            }
            if overview.billing_actions.can_open_portal {
                billing = billing.child(
                    Button::new("polar-portal", "Invoices, receipts and payment").on_click(
                        cx.listener(|this, _, _, cx| {
                            this.start_billing_action("/v1/billing/portal", None, cx)
                        }),
                    ),
                );
            }
            if overview.billing_actions.can_change_plan {
                billing = billing
                    .child(
                        Label::new(
                            "Polar shows any proration before you confirm a monthly plan change.",
                        )
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                    )
                    .child(
                        Button::new("polar-change-plan", "Change monthly plan").on_click(
                            cx.listener(|this, _, _, cx| {
                                this.start_billing_action("/v1/billing/portal", None, cx)
                            }),
                        ),
                    );
            }
            if overview.billing_actions.can_start_checkout {
                if overview
                    .billing_actions
                    .available_monthly_plan_ids
                    .iter()
                    .any(|plan| plan == "pro")
                {
                    billing = billing.child(
                        Button::new("upgrade-pro", "Upgrade to Pro monthly").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.start_billing_action(
                                    "/v1/billing/checkout",
                                    Some(serde_json::json!({"plan":"pro","cycle":"monthly"})),
                                    cx,
                                )
                            },
                        )),
                    );
                }
                if overview
                    .billing_actions
                    .available_monthly_plan_ids
                    .iter()
                    .any(|plan| plan == "team")
                {
                    billing = billing.child(
                        Button::new("upgrade-team", "Upgrade to Team monthly").on_click(
                            cx.listener(|this, _, _, cx| {
                                this.start_billing_action(
                                    "/v1/billing/checkout",
                                    Some(serde_json::json!({"plan":"team","cycle":"monthly"})),
                                    cx,
                                )
                            }),
                        ),
                    );
                }
            }
            if overview.billing_actions.can_buy_credits {
                billing = billing.child(Button::new("buy-credits", "Buy 500 credits").on_click(
                    cx.listener(|this, _, _, cx| {
                        this.start_billing_action(
                            "/v1/billing/credits",
                            Some(serde_json::json!({"pack":"small"})),
                            cx,
                        )
                    }),
                ));
            }
            if !overview.billing_actions.can_start_checkout
                && !overview.billing_actions.can_buy_credits
                && !overview.billing_actions.can_open_portal
            {
                let message = if overview.organization.role == "member" {
                    "Only organization owners and admins can manage billing. Ask one of them for help."
                } else {
                    "Purchases are not available for this organization right now."
                };
                billing = billing.child(
                    Label::new(message)
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                );
            }
        }

        if self.action_pending {
            billing = billing.child(Label::new("Opening secure billing…").color(Color::Muted));
        }
        if let Some(error) = &self.action_error {
            billing = billing.child(Label::new(error.clone()).color(Color::Muted));
        }

        let mut usage = Self::account_card(cx).child(Self::section_title(
            "Usage",
            "Credits spent across this organization. Available credits are a pooled balance.",
        ));
        match &data.usage {
            Ok(breakdown) => {
                usage = usage
                    .child(Self::usage_line("Last 7 days", &breakdown.seven_days))
                    .child(Self::usage_line("Last 30 days", &overview.usage_30d))
                    .child(Self::usage_line("Last 90 days", &breakdown.ninety_days))
                    .child(Self::daily_usage_chart(&breakdown.daily, cx));
            }
            Err(error) => {
                usage = usage
                    .child(Self::usage_line("Last 30 days", &overview.usage_30d))
                    .child(
                        Label::new(format!("7 and 90-day usage could not load: {error}"))
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    );
            }
        }

        v_flex()
            .w_full()
            .gap_4()
            .child(Self::account_card(cx)
                .child(Self::section_title("Credits available", "Your current balance, including grants and purchases."))
                .child(Label::new(available.max(0).to_string()).size(LabelSize::Custom(rems(2.5))))
                .when(available < 0, |card| card.child(Label::new(format!("{} credits to recover after refunds or adjustments", available.unsigned_abs())).size(LabelSize::Small).color(Color::Muted)))
                .child(Label::new(format!("{} · {cadence}", overview.plan.display_name)).color(Color::Muted)))
            .child(usage
                .child(Button::new("usage-details", "Detailed usage on the web")
                    .on_click(cx.listener(|this, _, _, cx| this.open_dashboard("usage", cx)))))
            .child(billing
                .child(Button::new("billing-dashboard", "Billing on the web")
                    .on_click(cx.listener(|this, _, _, cx| this.open_dashboard("billing", cx)))))
            .child(Self::account_card(cx)
                .child(Self::section_title("Current organization", "The editor key is linked to this organization."))
                .child(Label::new(format!(
                    "{} · {} · {}",
                    overview.organization.name,
                    overview.organization.role,
                    if overview.organization.is_personal { "Personal" } else { "Shared" }
                )))
                .child(Label::new("Switch organizations and manage members on the web. Sign in again to use the selected organization's editor key.").size(LabelSize::Small).color(Color::Muted))
                .child(Button::new("organization-dashboard", "Organization on the web")
                    .on_click(cx.listener(|this, _, _, cx| this.open_dashboard("organization", cx)))))
            .child(Button::new("refresh-account", "Refresh account")
                .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))))
    }
}

impl Render for FantaAccount {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match &self.state {
            AccountState::SignedOut => Self::account_card(cx)
                .child(Self::section_title(
                    "Sign in to Fanta",
                    "Open the Create panel and sign in to see your plan, credits and organization.",
                ))
                .into_any_element(),
            AccountState::Loading => Self::account_card(cx)
                .child(Self::section_title(
                    "Loading your account",
                    "Checking your credits, plan and recent usage…",
                ))
                .into_any_element(),
            AccountState::Error(error) => Self::account_card(cx)
                .child(Self::section_title(
                    "Account unavailable",
                    "Check your connection or sign in again in the Create panel.",
                ))
                .child(
                    Label::new(error.clone())
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                )
                .child(
                    Button::new("retry-account", "Retry")
                        .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                )
                .into_any_element(),
            AccountState::Ready(data) => self.render_ready(data, cx).into_any_element(),
        }
    }
}

async fn fetch_account(client: &Client, token: &str, server_url: &str) -> Result<AccountData> {
    let overview: AccountOverview =
        fetch_json(client, token, server_url, "/v1/account/overview", None).await?;
    let today = Utc::now().date_naive();
    let from = format!("{}T00:00:00Z", today - Duration::days(89));
    let usage_path = format!("/v1/usage?from={from}");
    let usage =
        match fetch_json::<UsageResponse>(client, token, server_url, &usage_path, None).await {
            Ok(usage) => Ok(summarize_usage(usage, today)),
            Err(error) => Err(format!("{error:#}")),
        };
    Ok(AccountData { overview, usage })
}

fn summarize_usage(usage: UsageResponse, today: NaiveDate) -> UsageBreakdown {
    let seven_day_start = today - Duration::days(6);
    let ninety_day_start = today - Duration::days(89);
    let mut summary = UsageBreakdown::default();
    summary.daily = (0..7)
        .map(|offset| DailyCredits {
            day: seven_day_start + Duration::days(offset),
            credits: 0,
        })
        .collect();
    for row in usage.days {
        let Some(date_prefix) = row.day.get(..10) else {
            continue;
        };
        let Ok(day) = NaiveDate::parse_from_str(date_prefix, "%Y-%m-%d") else {
            continue;
        };
        if day < ninety_day_start || day > today {
            continue;
        }
        summary.ninety_days.credits += row.billed_credits;
        summary.ninety_days.requests += row.requests;
        if day >= seven_day_start {
            summary.seven_days.credits += row.billed_credits;
            summary.seven_days.requests += row.requests;
            if let Some(daily) = summary.daily.iter_mut().find(|daily| daily.day == day) {
                daily.credits += row.billed_credits;
            }
        }
    }
    summary
}

#[cfg(not(feature = "mac_app_store"))]
async fn fetch_billing_url(
    client: &Client,
    token: &str,
    server_url: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<String> {
    #[derive(Deserialize)]
    struct BillingUrl {
        url: String,
    }
    let response: BillingUrl = fetch_json(
        client,
        token,
        server_url,
        path,
        Some(body.unwrap_or_else(|| serde_json::json!({}))),
    )
    .await?;
    let url = url::Url::parse(&response.url).context("Fanta returned an invalid billing link")?;
    if url.scheme() != "https" {
        bail!("Fanta returned a billing link without HTTPS.");
    }
    Ok(url.to_string())
}

async fn fetch_json<T: serde::de::DeserializeOwned>(
    client: &Client,
    token: &str,
    server_url: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<T> {
    let method = if body.is_some() {
        Method::POST
    } else {
        Method::GET
    };
    let request_body = match body {
        Some(value) => AsyncBody::from(serde_json::to_vec(&value)?),
        None => AsyncBody::empty(),
    };
    let request = Request::builder()
        .method(method)
        .uri(format!("{}{}", server_url.trim_end_matches('/'), path))
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .body(request_body)?;
    let mut response = client
        .http_client()
        .send(request)
        .await
        .context("Could not reach Fanta. Check your connection and retry")?;
    let status = response.status();
    let mut response_body = Vec::new();
    response
        .body_mut()
        .take(1024 * 1024)
        .read_to_end(&mut response_body)
        .await?;
    if !status.is_success() {
        match status.as_u16() {
            401 => bail!("Your Fanta sign-in expired. Sign in again in the Create panel."),
            403 => {
                bail!("Your organization role cannot perform this action. Ask an owner or admin.")
            }
            404 if path == "/v1/account/overview" => bail!(
                "This Fanta server does not support the account center yet. Update the server and retry."
            ),
            409 => bail!(
                "Another subscription or checkout is already in progress. Check billing before retrying."
            ),
            503 => bail!(
                "Purchases are not available yet. Try again after the billing catalog is released."
            ),
            _ => bail!(
                "Fanta could not complete the request (HTTP {status}). Retry or contact support."
            ),
        }
    }
    serde_json::from_slice(&response_body).context("Fanta returned an invalid account response")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_counts_both_windows_without_treating_a_balance_as_a_quota() -> Result<()> {
        let today = NaiveDate::from_ymd_opt(2026, 9, 27).context("valid test date")?;
        let response = UsageResponse {
            days: vec![
                UsageDay {
                    day: "2026-09-26 00:00:00+00".into(),
                    billed_credits: 25,
                    requests: 2,
                },
                UsageDay {
                    day: "2026-09-26 00:00:00+00".into(),
                    billed_credits: 5,
                    requests: 1,
                },
                UsageDay {
                    day: "2026-08-27 00:00:00+00".into(),
                    billed_credits: 40,
                    requests: 3,
                },
                UsageDay {
                    day: "2026-06-29 00:00:00+00".into(),
                    billed_credits: 100,
                    requests: 4,
                },
            ],
        };
        let summary = summarize_usage(response, today);
        assert_eq!(summary.seven_days.credits, 30);
        assert_eq!(summary.seven_days.requests, 3);
        assert_eq!(summary.ninety_days.credits, 70);
        assert_eq!(summary.ninety_days.requests, 6);
        assert_eq!(summary.daily.len(), 7);
        assert_eq!(
            summary
                .daily
                .iter()
                .find(|daily| daily.day == today - Duration::days(1))
                .map(|daily| daily.credits),
            Some(30)
        );
        Ok(())
    }

    #[test]
    fn overview_accepts_a_negative_balance_and_one_time_free_grant() -> Result<()> {
        let overview: AccountOverview = serde_json::from_value(serde_json::json!({
            "organization": { "id": "7fd42b77-6941-4234-a40f-53d08b7acc1a", "name": "Studio", "role": "owner", "is_personal": true },
            "plan": { "id": "free", "display_name": "Free", "monthly_credits": 500, "grant_cadence": "one_time", "currency": "USD" },
            "credits": { "available": -35 },
            "usage_30d": { "credits": 52, "requests": 9 },
            "subscription": null,
            "billing_actions": { "can_start_checkout": false, "can_buy_credits": false, "can_open_portal": false, "can_change_plan": false },
            "links": { "dashboard": "https://app.fantaisa.net/open?org_id=7fd42b77-6941-4234-a40f-53d08b7acc1a&section=account" }
        }))?;
        assert_eq!(overview.credits.available, -35);
        assert_eq!(overview.plan.grant_cadence, "one_time");
        assert!(overview.organization.is_personal);
        Ok(())
    }
}
