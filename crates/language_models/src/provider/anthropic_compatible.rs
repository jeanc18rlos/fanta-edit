use anthropic::completion::{AnthropicEventMapper, AnthropicPromptCacheMode, into_anthropic};
use anthropic::{AnthropicError, AnthropicModelMode};
use anyhow::Result;
use client::{Client, ClientSettings};
use credentials_provider::CredentialsProvider;
use futures::{FutureExt, StreamExt, future::BoxFuture, stream::BoxStream};
use gpui::{App, AppContext, AsyncApp, Context, Entity, SharedString, Task, Window};
use http_client::{CustomHeaders, HttpClient};
use language_model::{
    AuthenticateError, IconOrSvg, LanguageModel, LanguageModelCompletionError,
    LanguageModelCompletionEvent, LanguageModelId, LanguageModelName, LanguageModelProvider,
    LanguageModelProviderId, LanguageModelProviderName, LanguageModelProviderState,
    LanguageModelRequest, LanguageModelToolChoice, ProviderSettingsView, RateLimiter,
    SubPageProviderSettings,
};
use settings::Settings;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use ui::prelude::*;
use util::ResultExt;

use crate::provider::api_compatible::{
    ApiCompatibleProviderConfigurationView, ApiCompatibleProviderSettings,
    ApiCompatibleProviderState,
};

pub use settings::AnthropicCompatibleAvailableModel as AvailableModel;
pub use settings::AnthropicCompatibleModelCapabilities as ModelCapabilities;

const API_KEY_PLACEHOLDER: &str = "sk-ant-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";

#[derive(Default, Clone, Debug, PartialEq)]
pub struct AnthropicCompatibleSettings {
    pub api_url: String,
    pub available_models: Vec<AvailableModel>,
    pub custom_headers: CustomHeaders,
}

pub struct AnthropicCompatibleLanguageModelProvider {
    id: LanguageModelProviderId,
    name: LanguageModelProviderName,
    http_client: Arc<dyn HttpClient>,
    client: Arc<Client>,
    state: Entity<State>,
}

/// The signed-in account's access token, iff this provider targets the
/// account server itself (the managed Fanta provider). Signing in is then all
/// the configuration the provider needs — no separate API key. Providers
/// pointing anywhere else never see the account token.
fn account_token_for(client: &Arc<Client>, api_url: &str, cx: &App) -> Option<Arc<str>> {
    is_account_provider(api_url, &ClientSettings::get_global(cx).server_url)
        .then(|| client.account_access_token())
        .flatten()
}

fn is_account_provider(api_url: &str, server_url: &str) -> bool {
    !api_url.is_empty() && api_url.trim_end_matches('/') == server_url.trim_end_matches('/')
}

struct ProviderCredentials(Arc<dyn CredentialsProvider>);

impl ProviderCredentials {
    fn storage_url(url: &str, cx: &AsyncApp) -> String {
        cx.update(|cx| {
            if is_account_provider(url, &ClientSettings::get_global(cx).server_url) {
                // Account credentials use the backend URL itself. A separate
                // slot prevents caching them as API keys or overwriting sign-in.
                format!("{}/ai-api-key", url.trim_end_matches('/'))
            } else {
                url.to_string()
            }
        })
    }
}

impl CredentialsProvider for ProviderCredentials {
    fn read_credentials<'a>(
        &'a self,
        url: &'a str,
        cx: &'a AsyncApp,
    ) -> Pin<Box<dyn Future<Output = Result<Option<(String, Vec<u8>)>>> + 'a>> {
        Box::pin(async move {
            self.0
                .read_credentials(&Self::storage_url(url, cx), cx)
                .await
        })
    }

    fn write_credentials<'a>(
        &'a self,
        url: &'a str,
        username: &'a str,
        password: &'a [u8],
        cx: &'a AsyncApp,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
        Box::pin(async move {
            self.0
                .write_credentials(&Self::storage_url(url, cx), username, password, cx)
                .await
        })
    }

    fn delete_credentials<'a>(
        &'a self,
        url: &'a str,
        cx: &'a AsyncApp,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
        Box::pin(async move {
            self.0
                .delete_credentials(&Self::storage_url(url, cx), cx)
                .await
        })
    }
}

impl ApiCompatibleProviderSettings for AnthropicCompatibleSettings {
    fn api_url(&self) -> &str {
        &self.api_url
    }
}

pub type State = ApiCompatibleProviderState<AnthropicCompatibleSettings>;

fn available_model_to_anthropic_model(available: &AvailableModel) -> anthropic::Model {
    let mode = match available.mode.unwrap_or_default() {
        settings::ModelMode::Default => AnthropicModelMode::Default,
        settings::ModelMode::Thinking { budget_tokens } => {
            AnthropicModelMode::Thinking { budget_tokens }
        }
    };
    let supports_thinking = matches!(mode, AnthropicModelMode::Thinking { .. });

    anthropic::Model {
        display_name: available
            .display_name
            .clone()
            .unwrap_or_else(|| available.name.clone()),
        id: available.name.clone(),
        max_input_tokens: available.max_tokens,
        max_output_tokens: available.max_output_tokens.unwrap_or(4_096),
        default_temperature: available.default_temperature.unwrap_or(1.0),
        mode,
        supports_thinking,
        supports_adaptive_thinking: false,
        supports_images: available.capabilities.images,
        supports_speed: false,
        supports_compaction: false,
        supported_effort_levels: Vec::new(),
        tool_override: available.tool_override.clone(),
        extra_beta_headers: available.extra_beta_headers.clone(),
    }
}

impl AnthropicCompatibleLanguageModelProvider {
    pub fn new(
        id: Arc<str>,
        client: Arc<Client>,
        credentials_provider: Arc<dyn CredentialsProvider>,
        cx: &mut App,
    ) -> Self {
        let state = State::new(
            id.clone(),
            Arc::new(ProviderCredentials(credentials_provider)),
            |id, cx| {
                crate::AllLanguageModelSettings::get_global(cx)
                    .anthropic_compatible
                    .get(id)
            },
            cx,
        );

        // Sign-in/out changes whether the account token authenticates the
        // managed provider; poke the observable state so the registry and the
        // agent panel re-evaluate authentication without an app restart.
        let mut status = client.status();
        cx.spawn({
            let state = state.downgrade();
            async move |cx| {
                while status.next().await.is_some() {
                    if state.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            }
        })
        .detach();

        Self {
            id: id.clone().into(),
            name: id.into(),
            http_client: client.http_client(),
            client,
            state,
        }
    }

    fn create_language_model(&self, model: AvailableModel) -> Arc<dyn LanguageModel> {
        let capabilities = model.capabilities.clone();
        // Compatible providers may not support Anthropic's automatic prompt
        // caching; only request explicit (legacy) cache breakpoints when the
        // user has opted in via the `prompt_caching` capability.
        let cache_mode = if capabilities.prompt_caching {
            AnthropicPromptCacheMode::Legacy
        } else {
            AnthropicPromptCacheMode::Disabled
        };
        let model = available_model_to_anthropic_model(&model);

        Arc::new(AnthropicCompatibleLanguageModel {
            id: LanguageModelId::from(model.id.clone()),
            provider_id: self.id.clone(),
            provider_name: self.name.clone(),
            model,
            capabilities,
            cache_mode,
            state: self.state.clone(),
            http_client: self.http_client.clone(),
            client: self.client.clone(),
            request_limiter: RateLimiter::new(4),
        })
    }
}

impl LanguageModelProviderState for AnthropicCompatibleLanguageModelProvider {
    type ObservableEntity = State;

    fn observable_entity(&self) -> Option<Entity<Self::ObservableEntity>> {
        Some(self.state.clone())
    }
}

impl LanguageModelProvider for AnthropicCompatibleLanguageModelProvider {
    fn id(&self) -> LanguageModelProviderId {
        self.id.clone()
    }

    fn name(&self) -> LanguageModelProviderName {
        self.name.clone()
    }

    fn icon(&self) -> IconOrSvg {
        IconOrSvg::Icon(IconName::AiAnthropicCompat)
    }

    fn default_model(&self, cx: &App) -> Option<Arc<dyn LanguageModel>> {
        self.state
            .read(cx)
            .settings
            .available_models
            .first()
            .map(|model| self.create_language_model(model.clone()))
    }

    fn default_fast_model(&self, _cx: &App) -> Option<Arc<dyn LanguageModel>> {
        None
    }

    fn provided_models(&self, cx: &App) -> Vec<Arc<dyn LanguageModel>> {
        self.state
            .read(cx)
            .settings
            .available_models
            .iter()
            .map(|model| self.create_language_model(model.clone()))
            .collect()
    }

    fn is_authenticated(&self, cx: &App) -> bool {
        if self.state.read(cx).is_authenticated() {
            return true;
        }
        let api_url = self.state.read(cx).settings.api_url.clone();
        account_token_for(&self.client, &api_url, cx).is_some()
    }

    fn authenticate(&self, cx: &mut App) -> Task<Result<(), AuthenticateError>> {
        // Always run the credential load so an explicit key (provider UI or
        // env var) is available to requests, where it wins over the account
        // token. For the managed (account-backed) provider a MISSING key is
        // not a failure — signing in already authenticates it — so the
        // account token only masks missing credentials, never storage errors.
        let inner = self.state.update(cx, |state, cx| state.authenticate(cx));
        let api_url = self.state.read(cx).settings.api_url.clone();
        if account_token_for(&self.client, &api_url, cx).is_none() {
            return inner;
        }
        cx.spawn(async move |_cx| match inner.await {
            Ok(()) | Err(AuthenticateError::CredentialsNotFound) => Ok(()),
            Err(error) => Err(error),
        })
    }

    fn settings_view(&self, cx: &mut App) -> Option<ProviderSettingsView> {
        let state = self.state.clone();
        if is_account_provider(
            &state.read(cx).settings.api_url,
            &ClientSettings::get_global(cx).server_url,
        ) {
            let client = self.client.clone();
            return Some(ProviderSettingsView::SubPage(SubPageProviderSettings::new(
                move |_window, cx| {
                    cx.new(|cx| AccountConfigurationView::new(state.clone(), client.clone(), cx))
                        .into()
                },
            )));
        }
        Some(ProviderSettingsView::SubPage(SubPageProviderSettings::new(
            move |window, cx| {
                cx.new(|cx| {
                    ApiCompatibleProviderConfigurationView::new(
                        state.clone(),
                        "Anthropic",
                        API_KEY_PLACEHOLDER,
                        window,
                        cx,
                    )
                })
                .into()
            },
        )))
    }

    fn set_api_key(&self, api_key: Option<String>, cx: &mut App) -> Task<Result<()>> {
        self.state
            .update(cx, |state, cx| state.set_api_key(api_key, cx))
    }
}

struct AccountConfigurationView {
    state: Entity<State>,
    client: Arc<Client>,
    sign_in_task: Option<Task<()>>,
    sign_in_error: Option<SharedString>,
}

impl AccountConfigurationView {
    fn new(state: Entity<State>, client: Arc<Client>, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        Self {
            state,
            client,
            sign_in_task: None,
            sign_in_error: None,
        }
    }

    fn sign_in(&mut self, cx: &mut Context<Self>) {
        if self.sign_in_task.is_some() {
            return;
        }
        self.sign_in_error = None;
        self.sign_in_task = Some(cx.spawn({
            let client = self.client.clone();
            async move |this, cx| {
                let result = client.sign_in_with_optional_connect(true, cx).await;
                this.update(cx, |this, cx| {
                    this.sign_in_task = None;
                    this.sign_in_error = result.err().map(|error| error.to_string().into());
                    cx.notify();
                })
                .log_err();
            }
        }));
        cx.notify();
    }
}

impl Render for AccountConfigurationView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let signed_in =
            account_token_for(&self.client, &self.state.read(cx).settings.api_url, cx).is_some();
        let has_api_key = self.state.read(cx).is_authenticated();
        v_flex()
            .gap_4()
            .child(Label::new(if signed_in {
                "Your Fanta account is connected. AI usage is charged to your Fanta credits."
            } else if has_api_key {
                "A Fanta API key is configured. Sign in to manage your account and billing."
            } else {
                "Sign in to Fanta to use AI with your account credits."
            }))
            .when(!signed_in, |this| {
                this.child(
                    Button::new("fanta-sign-in", "Sign in to Fanta")
                        .style(ButtonStyle::Filled)
                        .disabled(self.sign_in_task.is_some())
                        .on_click(cx.listener(|this, _, _, cx| this.sign_in(cx))),
                )
            })
            .when_some(self.sign_in_error.clone(), |this, error| {
                this.child(Label::new(error).color(Color::Error))
            })
            .child(
                Button::new("fanta-manage-account", "Manage account and billing")
                    .on_click(|_, _, cx| cx.open_url(&client::zed_urls::account_url(cx))),
            )
    }
}

pub struct AnthropicCompatibleLanguageModel {
    id: LanguageModelId,
    provider_id: LanguageModelProviderId,
    provider_name: LanguageModelProviderName,
    model: anthropic::Model,
    capabilities: ModelCapabilities,
    cache_mode: AnthropicPromptCacheMode,
    state: Entity<State>,
    http_client: Arc<dyn HttpClient>,
    client: Arc<Client>,
    request_limiter: RateLimiter,
}

impl AnthropicCompatibleLanguageModel {
    fn stream_completion(
        &self,
        request: anthropic::Request,
        cx: &AsyncApp,
    ) -> BoxFuture<
        'static,
        Result<
            BoxStream<'static, Result<anthropic::Event, AnthropicError>>,
            LanguageModelCompletionError,
        >,
    > {
        let http_client = self.http_client.clone();
        let provider_name = self.provider_name.clone();

        let (api_key, api_url, extra_headers) = self.state.read_with(cx, |state, cx| {
            let api_url = state.settings.api_url.clone();
            // An explicit API key wins; the signed-in account token covers the
            // managed (account-server) provider so sign-in alone grants AI.
            let api_key = state
                .api_key_state
                .key(&api_url)
                .or_else(|| account_token_for(&self.client, &api_url, cx));
            (api_key, api_url, state.settings.custom_headers.clone())
        });

        let beta_headers = self.model.beta_headers();

        async move {
            let Some(api_key) = api_key else {
                return Err(LanguageModelCompletionError::NoApiKey {
                    provider: provider_name,
                });
            };

            let request = anthropic::stream_completion(
                http_client.as_ref(),
                &api_url,
                &api_key,
                request,
                beta_headers,
                &extra_headers,
            );

            request
                .await
                .map_err(|error| anthropic::completion_error_from_anthropic(error, provider_name))
        }
        .boxed()
    }
}

impl LanguageModel for AnthropicCompatibleLanguageModel {
    fn id(&self) -> LanguageModelId {
        self.id.clone()
    }

    fn name(&self) -> LanguageModelName {
        LanguageModelName::from(self.model.display_name.clone())
    }

    fn provider_id(&self) -> LanguageModelProviderId {
        self.provider_id.clone()
    }

    fn provider_name(&self) -> LanguageModelProviderName {
        self.provider_name.clone()
    }

    fn supports_tools(&self) -> bool {
        self.capabilities.tools
    }

    fn supports_images(&self) -> bool {
        self.capabilities.images
    }

    fn supports_streaming_tools(&self) -> bool {
        self.capabilities.tools
    }

    fn supports_tool_choice(&self, choice: LanguageModelToolChoice) -> bool {
        match choice {
            LanguageModelToolChoice::Auto | LanguageModelToolChoice::Any => self.capabilities.tools,
            LanguageModelToolChoice::None => true,
        }
    }

    fn supports_thinking(&self) -> bool {
        self.model.supports_thinking
    }

    fn telemetry_id(&self) -> String {
        format!("anthropic/{}", self.model.id)
    }

    fn max_token_count(&self) -> u64 {
        self.model.max_input_tokens
    }

    fn max_output_tokens(&self) -> Option<u64> {
        Some(self.model.max_output_tokens)
    }

    fn stream_completion(
        &self,
        request: LanguageModelRequest,
        cx: &AsyncApp,
    ) -> BoxFuture<
        'static,
        Result<
            BoxStream<'static, Result<LanguageModelCompletionEvent, LanguageModelCompletionError>>,
            LanguageModelCompletionError,
        >,
    > {
        let has_tools = !request.tools.is_empty();
        let request_id = self.model.request_id(has_tools).to_string();
        let mut request = into_anthropic(
            request,
            request_id,
            self.model.default_temperature,
            self.model.max_output_tokens,
            self.model.mode.clone(),
            self.cache_mode,
        );
        if !self.model.supports_speed {
            request.speed = None;
        }
        let completion_request = self.stream_completion(request, cx);
        let provider_name = self.provider_name.clone();
        let future = self.request_limiter.stream(async move {
            let response = completion_request.await?;
            Ok(AnthropicEventMapper::new(provider_name).map_stream(response))
        });
        async move { Ok(future.await?.boxed()) }.boxed()
    }
}
