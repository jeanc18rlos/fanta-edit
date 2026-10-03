use anthropic::completion::{AnthropicEventMapper, AnthropicPromptCacheMode, into_anthropic};
use anthropic::{AnthropicError, AnthropicModelMode};
use anyhow::{Context as _, Result, ensure};
use client::{Client, ClientSettings};
use credentials_provider::CredentialsProvider;
use futures::{AsyncReadExt as _, FutureExt, StreamExt, future::BoxFuture, stream::BoxStream};
use gpui::{Action as _, App, AppContext, AsyncApp, Context, Entity, SharedString, Task, Window};
use http_client::{AsyncBody, CustomHeaders, HttpClient, Method, Request};
use language_model::{
    AuthenticateError, IconOrSvg, LanguageModel, LanguageModelCompletionError,
    LanguageModelCompletionEvent, LanguageModelCostInfo, LanguageModelId, LanguageModelName,
    LanguageModelProvider, LanguageModelProviderId, LanguageModelProviderName,
    LanguageModelProviderState, LanguageModelRequest, LanguageModelToolChoice,
    ProviderSettingsView, RateLimiter, SubPageProviderSettings,
};
use serde_json::Value;
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
    catalog: Entity<CatalogState>,
}

#[derive(Clone)]
struct CatalogModel {
    available: AvailableModel,
    cost: Option<LanguageModelCostInfo>,
    adaptive_thinking: bool,
    reasoning_efforts: Vec<anthropic::Effort>,
    default_reasoning_effort: Option<anthropic::Effort>,
    supports_disabling_thinking: bool,
    forced_tool_choice: bool,
}

impl From<AvailableModel> for CatalogModel {
    fn from(available: AvailableModel) -> Self {
        Self {
            available,
            cost: None,
            adaptive_thinking: false,
            reasoning_efforts: Vec::new(),
            default_reasoning_effort: None,
            supports_disabling_thinking: true,
            forced_tool_choice: true,
        }
    }
}

#[derive(Default)]
struct CatalogState {
    models: Option<Vec<CatalogModel>>,
    revision: u64,
    fetch_task: Option<Task<()>>,
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

fn available_model_to_anthropic_model(record: &CatalogModel) -> anthropic::Model {
    let available = &record.available;
    let mode = if record.adaptive_thinking {
        AnthropicModelMode::AdaptiveThinking
    } else {
        match available.mode.unwrap_or_default() {
            settings::ModelMode::Default => AnthropicModelMode::Default,
            settings::ModelMode::Thinking { budget_tokens } => {
                AnthropicModelMode::Thinking { budget_tokens }
            }
        }
    };
    let supports_thinking = !matches!(mode, AnthropicModelMode::Default);
    let supports_adaptive_thinking = matches!(mode, AnthropicModelMode::AdaptiveThinking);

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
        supports_adaptive_thinking,
        supports_images: available.capabilities.images,
        supports_speed: false,
        supports_compaction: false,
        supported_effort_levels: record.reasoning_efforts.clone(),
        tool_override: available.tool_override.clone(),
        extra_beta_headers: available.extra_beta_headers.clone(),
    }
}

fn catalog_reasoning(capabilities: &Value, id: &str) -> (Vec<anthropic::Effort>, bool) {
    use anthropic::Effort;

    let options = capabilities["reasoning_options"].as_array();
    let declared_efforts = capabilities["effort_levels"]
        .as_array()
        .or_else(|| capabilities["supported_effort_levels"].as_array())
        .or_else(|| capabilities["reasoning_efforts"].as_array())
        .or_else(|| {
            options?.iter().find_map(|option| {
                (option["type"].as_str() == Some("effort"))
                    .then(|| option["values"].as_array())
                    .flatten()
            })
        });
    let efforts = if let Some(values) = declared_efforts {
        values
            .iter()
            .filter_map(Value::as_str)
            .filter_map(|value| value.parse::<Effort>().ok())
            .fold(Vec::new(), |mut efforts, effort| {
                if !efforts.contains(&effort) {
                    efforts.push(effort);
                }
                efforts
            })
    } else {
        // These aliases are verified against the gateway's reasoning_options;
        // older account catalogs omit their reasoning metadata.
        let model_id = id.strip_prefix("openai/").unwrap_or(id);
        let model_id = model_id.strip_suffix("-fast").unwrap_or(model_id);
        match model_id {
            "gpt-6-sol" | "gpt-6-luna" | "gpt-5.6-luna" => {
                vec![
                    Effort::None,
                    Effort::Low,
                    Effort::Medium,
                    Effort::High,
                    Effort::XHigh,
                    Effort::Max,
                ]
            }
            "gpt-5.5" | "gpt-5.4" | "gpt-5.4-mini" | "gpt-5.4-nano" | "gpt-5.4-pro"
            | "gpt-5.3-codex" | "gpt-5.2" | "gpt-5.1-thinking" => {
                vec![
                    Effort::None,
                    Effort::Minimal,
                    Effort::Low,
                    Effort::Medium,
                    Effort::High,
                    Effort::XHigh,
                ]
            }
            "gpt-5.6-sol" | "gpt-5.6-terra" => vec![
                Effort::None,
                Effort::Minimal,
                Effort::Low,
                Effort::Medium,
                Effort::High,
                Effort::XHigh,
                Effort::Max,
            ],
            "gpt-6-astra" | "gpt-6.1-sol" => vec![
                Effort::Low,
                Effort::Medium,
                Effort::High,
                Effort::XHigh,
                Effort::Max,
            ],
            "gpt-5.5-pro" | "gpt-5.2-pro" => vec![Effort::Medium, Effort::High, Effort::XHigh],
            "gpt-5.1-codex" | "gpt-5.1-codex-max" | "gpt-5.1-codex-mini" | "gpt-5.2-codex" => {
                vec![Effort::None, Effort::Low, Effort::Medium, Effort::High]
            }
            "gpt-5" | "gpt-5-mini" | "gpt-5-nano" => {
                vec![Effort::Minimal, Effort::Low, Effort::Medium, Effort::High]
            }
            "gpt-5-pro" => vec![Effort::High],
            "gpt-5-codex" | "o1" | "o3" | "o3-mini" | "o3-pro" | "o4-mini" => {
                vec![Effort::Low, Effort::Medium, Effort::High]
            }
            "claude-opus-5-5" | "anthropic/claude-opus-5.5" => {
                vec![
                    Effort::Low,
                    Effort::Medium,
                    Effort::High,
                    Effort::XHigh,
                    Effort::Max,
                ]
            }
            _ if capabilities["reasoning_mode"].as_str() == Some("adaptive") => {
                vec![Effort::Low, Effort::Medium, Effort::High, Effort::XHigh]
            }
            _ => Vec::new(),
        }
    };
    let supports_toggle = capabilities["supports_disabling_thinking"]
        .as_bool()
        .unwrap_or_else(|| {
            efforts.contains(&Effort::None)
                || options.is_some_and(|options| {
                    options
                        .iter()
                        .any(|option| option["type"].as_str() == Some("toggle"))
                })
        });
    (efforts, supports_toggle)
}

fn into_compatible_request(
    mut request: LanguageModelRequest,
    model: &anthropic::Model,
    default_reasoning_effort: Option<anthropic::Effort>,
    supports_disabling_thinking: bool,
    forced_tool_choice: bool,
    cache_mode: AnthropicPromptCacheMode,
) -> anthropic::Request {
    if model.supports_adaptive_thinking && !supports_disabling_thinking {
        request.thinking_allowed = true;
    }
    let thinking_allowed = request.thinking_allowed;
    if !model
        .supported_effort_levels
        .iter()
        .any(|effort| Some(effort.value()) == request.thinking_effort.as_deref())
    {
        request.thinking_effort = default_reasoning_effort.map(|effort| effort.value().to_owned());
    }
    if !forced_tool_choice && request.tool_choice == Some(LanguageModelToolChoice::Any) {
        request.tool_choice = Some(LanguageModelToolChoice::Auto);
    }
    let has_tools = !request.tools.is_empty();
    let request_id = model.request_id(has_tools).to_string();
    let mut request = into_anthropic(
        request,
        request_id,
        model.default_temperature,
        model.max_output_tokens,
        model.mode.clone(),
        cache_mode,
    );
    if !model.supports_speed {
        request.speed = None;
    }
    if model.supports_adaptive_thinking && supports_disabling_thinking && !thinking_allowed {
        request.output_config = Some(anthropic::OutputConfig {
            effort: Some(anthropic::Effort::None),
        });
    }
    request
}

fn parse_catalog(value: &Value) -> Result<Vec<CatalogModel>> {
    let rows = value["models"]
        .as_array()
        .context("Fanta's model catalog has no models array")?;
    let mut models = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for row in rows {
        if row["kind"].as_str() != Some("chat") {
            continue;
        }
        let Some(id) = row["id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 128)
        else {
            continue;
        };
        if !seen.insert(id) {
            continue;
        }
        let capabilities = &row["capabilities"];
        let context_window = capabilities["context_window"]
            .as_u64()
            .filter(|tokens| (4_096..=2_000_000).contains(tokens))
            .unwrap_or(200_000);
        let max_output_tokens = row["max_output_tokens"]
            .as_u64()
            .filter(|tokens| (1..=128_000).contains(tokens))
            .unwrap_or(4_096);
        let display_name = capabilities["display_name"]
            .as_str()
            .filter(|name| !name.is_empty() && name.len() <= 120)
            .unwrap_or(id);
        let available = AvailableModel {
            name: id.to_owned(),
            display_name: Some(display_name.to_owned()),
            max_tokens: context_window,
            tool_override: None,
            max_output_tokens: Some(max_output_tokens),
            default_temperature: None,
            extra_beta_headers: Vec::new(),
            mode: None,
            capabilities: ModelCapabilities {
                tools: capabilities["tools"].as_bool().unwrap_or(true),
                images: capabilities["images"].as_bool().unwrap_or(false),
                prompt_caching: capabilities["prompt_caching"].as_bool().unwrap_or(false),
            },
        };
        let cost = row["credits_per_mtok_input"]
            .as_u64()
            .zip(row["credits_per_mtok_output"].as_u64())
            .map(|(input_credits_per_1m, output_credits_per_1m)| {
                LanguageModelCostInfo::CreditTokenCost {
                    input_credits_per_1m,
                    output_credits_per_1m,
                }
            });
        let (mut reasoning_efforts, supports_disabling_thinking) =
            catalog_reasoning(capabilities, id);
        reasoning_efforts.retain(|effort| *effort != anthropic::Effort::None);
        let default_reasoning_effort = capabilities["default_effort"]
            .as_str()
            .and_then(|value| value.parse::<anthropic::Effort>().ok())
            .filter(|effort| reasoning_efforts.contains(effort))
            .or_else(|| {
                let default = if id.starts_with("gpt-") || id.starts_with("openai/") {
                    anthropic::Effort::Medium
                } else {
                    anthropic::Effort::High
                };
                reasoning_efforts.contains(&default).then_some(default)
            })
            .or_else(|| reasoning_efforts.first().copied());
        models.push(CatalogModel {
            available,
            cost,
            adaptive_thinking: capabilities["reasoning_mode"].as_str() == Some("adaptive")
                || !reasoning_efforts.is_empty(),
            reasoning_efforts,
            default_reasoning_effort,
            supports_disabling_thinking,
            forced_tool_choice: capabilities["forced_tool_choice"].as_bool().unwrap_or(true),
        });
    }
    Ok(models)
}

async fn fetch_catalog(
    http_client: &dyn HttpClient,
    api_url: &str,
    token: &str,
) -> Result<Vec<CatalogModel>> {
    const MAX_CATALOG_BYTES: u64 = 512 * 1024;
    let request = Request::builder()
        .method(Method::GET)
        .uri(format!("{}/v1/models", api_url.trim_end_matches('/')))
        .header("Accept", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .body(AsyncBody::empty())?;
    let response = http_client.send(request).await?;
    ensure!(
        response.status().is_success(),
        "Fanta catalog returned {}",
        response.status()
    );
    let mut bytes = Vec::new();
    response
        .into_body()
        .take(MAX_CATALOG_BYTES + 1)
        .read_to_end(&mut bytes)
        .await?;
    ensure!(
        bytes.len() as u64 <= MAX_CATALOG_BYTES,
        "Fanta catalog is too large"
    );
    parse_catalog(&serde_json::from_slice(&bytes)?)
}

fn refresh_catalog(
    state: &Entity<State>,
    catalog: &Entity<CatalogState>,
    client: &Arc<Client>,
    http_client: &Arc<dyn HttpClient>,
    cx: &mut App,
) {
    let (api_url, key, managed_account) = state.read_with(cx, |state, cx| {
        let api_url = state.settings.api_url.clone();
        let managed_account =
            is_account_provider(&api_url, &ClientSettings::get_global(cx).server_url);
        let key = state
            .api_key_state
            .key(&api_url)
            .or_else(|| account_token_for(client, &api_url, cx));
        (api_url, key, managed_account)
    });
    let revision = catalog.update(cx, |catalog, _| {
        catalog.revision = catalog.revision.wrapping_add(1);
        catalog.models = None;
        catalog.fetch_task = None;
        catalog.revision
    });
    state.update(cx, |_, cx| cx.notify());
    if !managed_account {
        return;
    }
    let Some(key) = key else {
        return;
    };
    let catalog_for_task = catalog.downgrade();
    let state_for_task = state.downgrade();
    let http_client = http_client.clone();
    let task = cx.spawn(async move |cx| {
        let result = fetch_catalog(http_client.as_ref(), &api_url, &key).await;
        let (Some(catalog_for_task), Some(state_for_task)) =
            (catalog_for_task.upgrade(), state_for_task.upgrade())
        else {
            return;
        };
        match result {
            Ok(models) => {
                let changed = catalog_for_task.update(cx, |catalog, _| {
                    if catalog.revision != revision {
                        return false;
                    }
                    catalog.models = Some(models);
                    true
                });
                if changed {
                    state_for_task.update(cx, |_, cx| cx.notify());
                }
            }
            Err(error) => log::warn!("Fanta model catalog unavailable: {error:#}"),
        }
    });
    catalog.update(cx, |catalog, _| catalog.fetch_task = Some(task));
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

        let catalog = cx.new(|_| CatalogState::default());

        // Sign-in/out changes whether the account token authenticates the
        // managed provider; poke the observable state so the registry and the
        // agent panel re-evaluate authentication without an app restart.
        let mut status = client.status();
        cx.spawn({
            let state = state.downgrade();
            let catalog = catalog.downgrade();
            let client = client.clone();
            let http_client = client.http_client();
            async move |cx| {
                while status.next().await.is_some() {
                    let (Some(state), Some(catalog)) = (state.upgrade(), catalog.upgrade()) else {
                        break;
                    };
                    cx.update(|cx| {
                        state.update(cx, |_, cx| cx.notify());
                        refresh_catalog(&state, &catalog, &client, &http_client, cx);
                    });
                }
            }
        })
        .detach();

        let provider = Self {
            id: id.clone().into(),
            name: id.into(),
            http_client: client.http_client(),
            client,
            state,
            catalog,
        };
        provider.refresh_catalog(cx);
        provider
    }

    fn refresh_catalog(&self, cx: &mut App) {
        refresh_catalog(
            &self.state,
            &self.catalog,
            &self.client,
            &self.http_client,
            cx,
        );
    }

    fn models(&self, cx: &App) -> Vec<CatalogModel> {
        self.catalog.read(cx).models.clone().unwrap_or_else(|| {
            self.state
                .read(cx)
                .settings
                .available_models
                .iter()
                .cloned()
                .map(CatalogModel::from)
                .collect()
        })
    }

    fn create_language_model(&self, record: CatalogModel) -> Arc<dyn LanguageModel> {
        let capabilities = record.available.capabilities.clone();
        // Compatible providers may not support Anthropic's automatic prompt
        // caching; only request explicit (legacy) cache breakpoints when the
        // user has opted in via the `prompt_caching` capability.
        let cache_mode = if capabilities.prompt_caching {
            AnthropicPromptCacheMode::Legacy
        } else {
            AnthropicPromptCacheMode::Disabled
        };
        let model = available_model_to_anthropic_model(&record);

        Arc::new(AnthropicCompatibleLanguageModel {
            id: LanguageModelId::from(model.id.clone()),
            provider_id: self.id.clone(),
            provider_name: self.name.clone(),
            model,
            capabilities,
            cost: record.cost,
            default_reasoning_effort: record.default_reasoning_effort,
            supports_disabling_thinking: record.supports_disabling_thinking,
            forced_tool_choice: record.forced_tool_choice,
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
        let models = self.models(cx);
        models
            .iter()
            .find(|record| record.available.name == "claude-sonnet-5")
            .or_else(|| models.first())
            .cloned()
            .map(|record| self.create_language_model(record))
    }

    fn default_fast_model(&self, cx: &App) -> Option<Arc<dyn LanguageModel>> {
        let models = self.models(cx);
        models
            .iter()
            .find(|record| record.available.name == "gpt-6-luna")
            .or_else(|| {
                models
                    .iter()
                    .find(|record| record.available.name == "claude-haiku-4-5")
            })
            .cloned()
            .map(|record| self.create_language_model(record))
    }

    fn provided_models(&self, cx: &App) -> Vec<Arc<dyn LanguageModel>> {
        self.models(cx)
            .into_iter()
            .map(|model| self.create_language_model(model))
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
        let has_account_token = account_token_for(&self.client, &api_url, cx).is_some();
        let state = self.state.clone();
        let catalog = self.catalog.clone();
        let client = self.client.clone();
        let http_client = self.http_client.clone();
        cx.spawn(async move |cx| {
            let result = inner.await;
            cx.update(|cx| refresh_catalog(&state, &catalog, &client, &http_client, cx));
            match result {
                Ok(()) => Ok(()),
                Err(AuthenticateError::CredentialsNotFound) if has_account_token => Ok(()),
                Err(error) => Err(error),
            }
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
        let inner = self
            .state
            .update(cx, |state, cx| state.set_api_key(api_key, cx));
        let state = self.state.clone();
        let catalog = self.catalog.clone();
        let client = self.client.clone();
        let http_client = self.http_client.clone();
        cx.spawn(async move |cx| {
            let result = inner.await;
            cx.update(|cx| refresh_catalog(&state, &catalog, &client, &http_client, cx));
            result
        })
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
                Button::new("fanta-manage-account", "Manage account and billing").on_click(
                    |_, window, cx| {
                        window.dispatch_action(zed_actions::OpenAccountSettings.boxed_clone(), cx);
                    },
                ),
            )
    }
}

pub struct AnthropicCompatibleLanguageModel {
    id: LanguageModelId,
    provider_id: LanguageModelProviderId,
    provider_name: LanguageModelProviderName,
    model: anthropic::Model,
    capabilities: ModelCapabilities,
    cost: Option<LanguageModelCostInfo>,
    default_reasoning_effort: Option<anthropic::Effort>,
    supports_disabling_thinking: bool,
    forced_tool_choice: bool,
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

        let (api_key, api_url, extra_headers, managed_account) =
            self.state.read_with(cx, |state, cx| {
                let api_url = state.settings.api_url.clone();
                // An explicit API key wins; the signed-in account token covers the
                // managed (account-server) provider so sign-in alone grants AI.
                let api_key = state
                    .api_key_state
                    .key(&api_url)
                    .or_else(|| account_token_for(&self.client, &api_url, cx));
                let managed_account =
                    is_account_provider(&api_url, &ClientSettings::get_global(cx).server_url);
                (
                    api_key,
                    api_url,
                    state.settings.custom_headers.clone(),
                    managed_account,
                )
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

            request.await.map_err(|error| match error {
                AnthropicError::ApiError(api_error)
                    if managed_account
                        && api_error.error_type == "invalid_request_error"
                        && api_error
                            .message
                            .contains("Fanta credit balance is too low") =>
                {
                    LanguageModelCompletionError::PaymentRequired
                }
                error => anthropic::completion_error_from_anthropic(error, provider_name),
            })
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
            LanguageModelToolChoice::Auto => self.capabilities.tools,
            LanguageModelToolChoice::Any => self.capabilities.tools && self.forced_tool_choice,
            LanguageModelToolChoice::None => true,
        }
    }

    fn supports_thinking(&self) -> bool {
        self.model.supports_thinking
    }

    fn supports_disabling_thinking(&self) -> bool {
        self.supports_disabling_thinking
    }

    fn supported_effort_levels(&self) -> Vec<language_model::LanguageModelEffortLevel> {
        self.model
            .supported_effort_levels
            .iter()
            .map(|effort| language_model::LanguageModelEffortLevel {
                name: effort.label().into(),
                value: effort.value().into(),
                is_default: Some(*effort) == self.default_reasoning_effort,
            })
            .collect()
    }

    fn model_cost_info(&self) -> Option<LanguageModelCostInfo> {
        self.cost.clone()
    }

    fn telemetry_id(&self) -> String {
        format!("{}/{}", self.provider_id.0, self.model.id)
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
        let request = into_compatible_request(
            request,
            &self.model,
            self.default_reasoning_effort,
            self.supports_disabling_thinking,
            self.forced_tool_choice,
            self.cache_mode,
        );
        let completion_request = self.stream_completion(request, cx);
        let provider_name = self.provider_name.clone();
        let future = self.request_limiter.stream(async move {
            let response = completion_request.await?;
            Ok(AnthropicEventMapper::new(provider_name).map_stream(response))
        });
        async move { Ok(future.await?.boxed()) }.boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn catalog_model(id: &str, capabilities: Value) -> Result<CatalogModel> {
        parse_catalog(&json!({
            "models": [{ "id": id, "kind": "chat", "capabilities": capabilities }]
        }))?
        .into_iter()
        .next()
        .context("test catalog must contain a model")
    }

    #[test]
    fn gpt_catalog_aliases_expose_their_verified_efforts_and_toggle() -> Result<()> {
        let sol = catalog_model("gpt-6-sol", json!({}))?;
        assert!(sol.adaptive_thinking);
        assert!(sol.supports_disabling_thinking);
        assert_eq!(
            sol.default_reasoning_effort,
            Some(anthropic::Effort::Medium)
        );
        assert_eq!(
            sol.reasoning_efforts
                .iter()
                .map(|effort| effort.value())
                .collect::<Vec<_>>(),
            ["low", "medium", "high", "xhigh", "max"],
        );
        let five = catalog_model("gpt-5.5", json!({}))?;
        assert_eq!(
            five.reasoning_efforts
                .iter()
                .map(|effort| effort.value())
                .collect::<Vec<_>>(),
            ["minimal", "low", "medium", "high", "xhigh"],
        );
        let unknown = catalog_model("unknown-model", json!({}))?;
        assert!(!available_model_to_anthropic_model(&unknown).supports_thinking);
        assert!(unknown.reasoning_efforts.is_empty());
        Ok(())
    }

    #[test]
    fn catalog_metadata_overrides_alias_defaults_and_filters_invalid_efforts() -> Result<()> {
        let model = catalog_model(
            "gpt-6-sol",
            json!({
                "effort_levels": ["low", "high", "ultra-unknown", "high", 17],
                "default_effort": "high",
                "supports_disabling_thinking": false,
            }),
        )?;
        assert_eq!(
            model.reasoning_efforts,
            [anthropic::Effort::Low, anthropic::Effort::High]
        );
        assert_eq!(
            model.default_reasoning_effort,
            Some(anthropic::Effort::High)
        );
        assert!(!model.supports_disabling_thinking);
        let disabled = catalog_model("gpt-6-sol", json!({ "effort_levels": [] }))?;
        assert!(!available_model_to_anthropic_model(&disabled).supports_thinking);
        Ok(())
    }

    #[test]
    fn compatible_requests_forward_gpt_effort_and_explicit_thinking_off() -> Result<()> {
        let record = catalog_model("gpt-6-sol", json!({}))?;
        let model = available_model_to_anthropic_model(&record);
        for (thinking_allowed, selected, expected) in [
            (true, "max", "max"),
            (true, "unsupported", "medium"),
            (false, "high", "none"),
        ] {
            let request = into_compatible_request(
                LanguageModelRequest {
                    thinking_allowed,
                    thinking_effort: Some(selected.to_owned()),
                    ..Default::default()
                },
                &model,
                record.default_reasoning_effort,
                record.supports_disabling_thinking,
                record.forced_tool_choice,
                AnthropicPromptCacheMode::Disabled,
            );
            let serialized = serde_json::to_value(&request)?;
            assert_eq!(serialized["output_config"]["effort"], expected);
            assert_eq!(request.thinking.is_some(), thinking_allowed);
        }
        let record = catalog_model("gpt-5.5", json!({}))?;
        let request = into_compatible_request(
            LanguageModelRequest {
                thinking_allowed: true,
                thinking_effort: Some("minimal".into()),
                ..Default::default()
            },
            &available_model_to_anthropic_model(&record),
            record.default_reasoning_effort,
            record.supports_disabling_thinking,
            true,
            AnthropicPromptCacheMode::Disabled,
        );
        assert_eq!(
            serde_json::to_value(request)?["output_config"]["effort"],
            "minimal"
        );
        Ok(())
    }

    #[test]
    fn compatible_requests_preserve_always_on_claude_reasoning() -> Result<()> {
        let record = catalog_model("claude-opus-5-5", json!({}))?;
        let request = into_compatible_request(
            LanguageModelRequest {
                thinking_allowed: false,
                thinking_effort: Some("max".into()),
                ..Default::default()
            },
            &available_model_to_anthropic_model(&record),
            record.default_reasoning_effort,
            record.supports_disabling_thinking,
            true,
            AnthropicPromptCacheMode::Disabled,
        );
        assert!(request.thinking.is_some());
        assert_eq!(
            serde_json::to_value(request)?["output_config"]["effort"],
            "max"
        );
        Ok(())
    }

    #[test]
    fn catalog_exposes_only_chat_models_with_server_authored_capabilities_and_prices() {
        let catalog = parse_catalog(&json!({
            "models": [
                {
                    "id": "claude-opus-5-5",
                    "kind": "chat",
                    "max_output_tokens": 64000,
                    "credits_per_mtok_input": 5600,
                    "credits_per_mtok_output": 28000,
                    "capabilities": {
                        "display_name": "Claude Opus 5.5",
                        "context_window": 1000000,
                        "tools": true,
                        "images": true,
                        "prompt_caching": true,
                        "reasoning_mode": "adaptive",
                        "forced_tool_choice": false
                    }
                },
                { "id": "claude-opus-5-5", "kind": "chat" },
                { "id": "typesafe-ai/jev", "kind": "evaluation" },
                { "id": "flux-2", "kind": "image" }
            ]
        }))
        .unwrap();
        assert_eq!(catalog.len(), 1);
        let opus = &catalog[0];
        assert_eq!(opus.available.name, "claude-opus-5-5");
        assert_eq!(opus.available.max_tokens, 1_000_000);
        assert_eq!(opus.available.max_output_tokens, Some(64_000));
        assert!(opus.available.capabilities.images);
        assert!(opus.available.capabilities.prompt_caching);
        assert!(!opus.forced_tool_choice);
        assert_eq!(
            opus.cost,
            Some(LanguageModelCostInfo::CreditTokenCost {
                input_credits_per_1m: 5_600,
                output_credits_per_1m: 28_000,
            })
        );
        assert!(matches!(
            available_model_to_anthropic_model(opus).mode,
            AnthropicModelMode::AdaptiveThinking
        ));
    }

    #[test]
    fn malformed_catalog_is_rejected_and_invalid_rows_are_skipped() {
        assert!(parse_catalog(&json!({})).is_err());
        let catalog = parse_catalog(&json!({
            "models": [
                { "kind": "chat" },
                { "id": "", "kind": "chat" },
                { "id": "gpt-6-sol", "kind": "chat", "capabilities": {} }
            ]
        }))
        .unwrap();
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].available.max_tokens, 200_000);
        assert!(catalog[0].available.capabilities.tools);
        assert_eq!(catalog[0].cost, None);
    }
}
