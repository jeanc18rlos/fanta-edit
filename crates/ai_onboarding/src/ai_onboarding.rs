mod agent_api_keys_onboarding;
mod agent_panel_onboarding_card;
mod agent_panel_onboarding_content;
mod edit_prediction_onboarding_content;
mod plan_definitions;

pub use agent_api_keys_onboarding::{ApiKeysWithProviders, ApiKeysWithoutProviders};
pub use agent_panel_onboarding_card::AgentPanelOnboardingCard;
pub use agent_panel_onboarding_content::AgentPanelOnboarding;
use cloud_api_types::Plan;
pub use edit_prediction_onboarding_content::EditPredictionOnboarding;
pub use plan_definitions::PlanDefinitions;

use std::sync::Arc;

use client::{Client, UserStore};
use gpui::{AnyElement, Entity, IntoElement, ParentElement, TaskExt};
use ui::{RegisterComponent, Tooltip, prelude::*};

#[derive(PartialEq)]
pub enum SignInStatus {
    SignedIn,
    SigningIn,
    SignedOut,
}

impl From<client::Status> for SignInStatus {
    fn from(status: client::Status) -> Self {
        if status.is_signing_in() {
            Self::SigningIn
        } else if status.is_signed_out() {
            Self::SignedOut
        } else {
            Self::SignedIn
        }
    }
}

#[derive(RegisterComponent, IntoElement)]
pub struct ZedAiOnboarding {
    pub sign_in_status: SignInStatus,
    pub plan: Option<Plan>,
    pub account_too_young: bool,
    pub continue_with_zed_ai: Arc<dyn Fn(&mut Window, &mut App)>,
    pub sign_in: Arc<dyn Fn(&mut Window, &mut App)>,
    pub dismiss_onboarding: Option<Arc<dyn Fn(&mut Window, &mut App)>>,
}

impl ZedAiOnboarding {
    pub fn new(
        client: Arc<Client>,
        user_store: &Entity<UserStore>,
        continue_with_zed_ai: Arc<dyn Fn(&mut Window, &mut App)>,
        cx: &mut App,
    ) -> Self {
        let store = user_store.read(cx);
        let status = *client.status().borrow();

        Self {
            sign_in_status: status.into(),
            plan: store.plan(),
            account_too_young: store.account_too_young(),
            continue_with_zed_ai,
            sign_in: Arc::new(move |_window, cx| {
                cx.spawn({
                    let client = client.clone();
                    async move |cx| client.sign_in_with_optional_connect(true, cx).await
                })
                .detach_and_log_err(cx);
            }),
            dismiss_onboarding: None,
        }
    }

    pub fn with_dismiss(
        mut self,
        dismiss_callback: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.dismiss_onboarding = Some(Arc::new(dismiss_callback));
        self
    }

    fn render_dismiss_button(&self) -> Option<AnyElement> {
        self.dismiss_onboarding.as_ref().map(|dismiss_callback| {
            let callback = dismiss_callback.clone();

            h_flex()
                .absolute()
                .top_0()
                .right_0()
                .child(
                    IconButton::new("dismiss_onboarding", IconName::Close)
                        .icon_size(IconSize::Small)
                        .tooltip(Tooltip::text("Dismiss"))
                        .on_click(move |_, window, cx| {
                            telemetry::event!("Banner Dismissed", source = "AI Onboarding",);
                            callback(window, cx)
                        }),
                )
                .into_any_element()
        })
    }

    fn render_model_setup(&self) -> AnyElement {
        let (headline, description, button_label, callback) = match self.sign_in_status {
            SignInStatus::SignedOut => (
                "Create with Fanta AI",
                "Sign in to use your Fanta plan and credits for design assistance and generation.",
                "Sign in to Fanta",
                self.sign_in.clone(),
            ),
            SignInStatus::SigningIn => (
                "Connect your Fanta account",
                "Finish signing in in your browser to return to your design.",
                "Signing in…",
                self.sign_in.clone(),
            ),
            SignInStatus::SignedIn => (
                "Your Fanta account",
                "Use your Fanta plan and credits to work with AI in this project.",
                "Continue with Fanta",
                self.continue_with_zed_ai.clone(),
            ),
        };
        v_flex()
            .w_full()
            .relative()
            .gap_1()
            .child(Headline::new(headline))
            .child(Label::new(description).color(Color::Muted).mb_2())
            .child(
                Button::new("connect-fanta-account", button_label)
                    .full_width()
                    .style(ButtonStyle::Filled)
                    .disabled(self.sign_in_status == SignInStatus::SigningIn)
                    .on_click(move |_, window, cx| callback(window, cx)),
            )
            .children(self.render_dismiss_button())
            .into_any_element()
    }
}

impl RenderOnce for ZedAiOnboarding {
    fn render(self, _window: &mut ui::Window, _cx: &mut App) -> impl IntoElement {
        self.render_model_setup()
    }
}

impl Component for ZedAiOnboarding {
    fn scope() -> ComponentScope {
        ComponentScope::Onboarding
    }

    fn name() -> &'static str {
        "Agent New User Onboarding"
    }

    fn description() -> &'static str {
        "Connect a Fanta account to use managed AI with a subscription and credits."
    }

    fn preview(_window: &mut Window, _cx: &mut App) -> AnyElement {
        div()
            .w_full()
            .min_w_40()
            .max_w(px(1100.))
            .child(
                AgentPanelOnboardingCard::new().child(
                    ZedAiOnboarding {
                        sign_in_status: SignInStatus::SignedOut,
                        plan: None,
                        account_too_young: false,
                        continue_with_zed_ai: Arc::new(|_, _| {}),
                        sign_in: Arc::new(|_, _| {}),
                        dismiss_onboarding: None,
                    }
                    .into_any_element(),
                ),
            )
            .into_any_element()
    }
}
