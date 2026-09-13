use std::sync::Arc;

use ::settings::{Settings, SettingsStore};
use client::{Client, UserStore};
use collections::HashMap;
use credentials_provider::CredentialsProvider;
use gpui::{App, Context, Entity};
use language_model::{LanguageModelProviderId, LanguageModelRegistry};

pub mod provider;
mod settings;

use crate::provider::anthropic::AnthropicLanguageModelProvider;
use crate::provider::anthropic_compatible::AnthropicCompatibleLanguageModelProvider;
use crate::provider::open_ai::OpenAiLanguageModelProvider;
use crate::provider::open_ai_compatible::OpenAiCompatibleLanguageModelProvider;
pub use crate::settings::*;

pub fn init(user_store: Entity<UserStore>, client: Arc<Client>, cx: &mut App) {
    let credentials_provider = client.credentials_provider();
    let registry = LanguageModelRegistry::global(cx);
    registry.update(cx, |registry, cx| {
        register_language_model_providers(
            registry,
            user_store,
            client.clone(),
            credentials_provider.clone(),
            cx,
        );
    });

    let mut compatible_providers = CompatibleProviders::from_settings(cx);

    registry.update(cx, |registry, cx| {
        register_compatible_providers(
            registry,
            &CompatibleProviders::default(),
            &compatible_providers,
            &client,
            &credentials_provider,
            cx,
        );
    });

    let registry = registry.downgrade();
    cx.observe_global::<SettingsStore>(move |cx| {
        let Some(registry) = registry.upgrade() else {
            return;
        };
        let compatible_providers_new = CompatibleProviders::from_settings(cx);
        if compatible_providers_new != compatible_providers {
            registry.update(cx, |registry, cx| {
                register_compatible_providers(
                    registry,
                    &compatible_providers,
                    &compatible_providers_new,
                    &client,
                    &credentials_provider,
                    cx,
                );
            });
            compatible_providers = compatible_providers_new;
        }
    })
    .detach();
}

#[derive(Default, PartialEq, Eq)]
struct CompatibleProviders(HashMap<Arc<str>, CompatibleProviderKind>);

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum CompatibleProviderKind {
    OpenAi,
    Anthropic,
}

impl CompatibleProviders {
    fn from_settings(cx: &App) -> Self {
        let settings = AllLanguageModelSettings::get_global(cx);
        let mut providers: HashMap<Arc<str>, CompatibleProviderKind> = settings
            .openai_compatible
            .keys()
            .map(|id| (id.clone(), CompatibleProviderKind::OpenAi))
            .collect();
        for id in settings.anthropic_compatible.keys() {
            // The registry has a single provider ID namespace, so a name can
            // only refer to one provider. OpenAI-compatible entries win
            // collisions because they predate Anthropic-compatible ones, so
            // existing configurations keep working.
            if providers.contains_key(id) {
                log::warn!(
                    "ignoring `anthropic_compatible` provider `{id}`: \
                     an `openai_compatible` provider with the same name exists"
                );
            } else {
                providers.insert(id.clone(), CompatibleProviderKind::Anthropic);
            }
        }
        Self(providers)
    }
}

fn register_compatible_providers(
    registry: &mut LanguageModelRegistry,
    old: &CompatibleProviders,
    new: &CompatibleProviders,
    client: &Arc<Client>,
    credentials_provider: &Arc<dyn CredentialsProvider>,
    cx: &mut Context<LanguageModelRegistry>,
) {
    for (provider_id, old_kind) in &old.0 {
        if new.0.get(provider_id) != Some(old_kind) {
            registry.unregister_provider(LanguageModelProviderId::from(provider_id.clone()), cx);
        }
    }

    for (provider_id, kind) in &new.0 {
        if old.0.get(provider_id) != Some(kind) {
            match kind {
                CompatibleProviderKind::OpenAi => registry.register_provider(
                    Arc::new(OpenAiCompatibleLanguageModelProvider::new(
                        provider_id.clone(),
                        client.http_client(),
                        credentials_provider.clone(),
                        cx,
                    )),
                    cx,
                ),
                CompatibleProviderKind::Anthropic => registry.register_provider(
                    Arc::new(AnthropicCompatibleLanguageModelProvider::new(
                        provider_id.clone(),
                        client.clone(),
                        credentials_provider.clone(),
                        cx,
                    )),
                    cx,
                ),
            }
        }
    }
}

fn register_language_model_providers(
    registry: &mut LanguageModelRegistry,
    // The managed "Fanta" provider is served by the settings-driven
    // `anthropic_compatible` path, not by the zed.dev cloud provider, so nothing here
    // needs the user store. The parameter stays so `init`'s public signature is unchanged.
    _user_store: Entity<UserStore>,
    client: Arc<Client>,
    credentials_provider: Arc<dyn CredentialsProvider>,
    cx: &mut Context<LanguageModelRegistry>,
) {
    registry.register_provider(
        Arc::new(AnthropicLanguageModelProvider::new(
            client.http_client(),
            credentials_provider.clone(),
            cx,
        )),
        cx,
    );
    registry.register_provider(
        Arc::new(OpenAiLanguageModelProvider::new(
            client.http_client(),
            credentials_provider,
            cx,
        )),
        cx,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use clock::FakeSystemClock;
    use feature_flags::FeatureFlagAppExt as _;
    use futures::{AsyncReadExt as _, StreamExt as _};
    use gpui::{AppContext as _, AsyncApp, BorrowAppContext as _, TestAppContext};
    use http_client::FakeHttpClient;
    use language_model::{
        AuthenticateError, IconOrSvg, LanguageModelCompletionEvent, LanguageModelProvider as _,
        LanguageModelRequest, LanguageModelRequestMessage, MessageContent, Role,
    };
    use release_channel::AppVersion;
    use std::future::Future;
    use std::pin::Pin;
    use ui::IconName;

    #[derive(Default)]
    struct FakeCredentialsProvider {
        read_error: Option<&'static str>,
        stored_credentials: Option<(String, String, Vec<u8>)>,
        expected_storage_url: Option<&'static str>,
    }

    impl CredentialsProvider for FakeCredentialsProvider {
        fn read_credentials<'a>(
            &'a self,
            url: &'a str,
            _cx: &'a AsyncApp,
        ) -> Pin<Box<dyn Future<Output = Result<Option<(String, Vec<u8>)>>> + 'a>> {
            Box::pin(async move {
                if let Some(error) = self.read_error {
                    anyhow::bail!(error);
                }
                if let Some(expected_url) = self.expected_storage_url {
                    anyhow::ensure!(url == expected_url, "unexpected credential storage URL");
                }
                Ok(self
                    .stored_credentials
                    .as_ref()
                    .and_then(|(stored_url, username, key)| {
                        (stored_url == url).then(|| (username.clone(), key.clone()))
                    }))
            })
        }

        fn write_credentials<'a>(
            &'a self,
            url: &'a str,
            _username: &'a str,
            _password: &'a [u8],
            _cx: &'a AsyncApp,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
            Box::pin(async move {
                if let Some(expected_url) = self.expected_storage_url {
                    assert_eq!(url, expected_url, "unexpected credential storage URL");
                }
                Ok(())
            })
        }

        fn delete_credentials<'a>(
            &'a self,
            url: &'a str,
            _cx: &'a AsyncApp,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
            Box::pin(async move {
                if let Some(expected_url) = self.expected_storage_url {
                    assert_eq!(url, expected_url, "unexpected credential storage URL");
                }
                Ok(())
            })
        }
    }

    fn init_test(cx: &mut App) -> (Arc<Client>, Arc<dyn CredentialsProvider>) {
        let settings_store = SettingsStore::test(cx);
        cx.set_global(settings_store);
        cx.set_global(db::AppDatabase::test_new());
        let app_version = AppVersion::global(cx);
        release_channel::init_test(app_version, release_channel::ReleaseChannel::Dev, cx);
        gpui_tokio::init(cx);
        cx.update_flags(false, Vec::new());

        let client = Client::new(
            Arc::new(FakeSystemClock::new()),
            FakeHttpClient::with_404_response(),
            cx,
        );
        (client, Arc::new(FakeCredentialsProvider::default()))
    }

    fn update_compatible_provider_settings(
        openai: &[&str],
        anthropic: &[&str],
        cx: &mut App,
    ) -> CompatibleProviders {
        fn section(ids: &[&str]) -> serde_json::Value {
            ids.iter()
                .map(|id| {
                    (
                        id.to_string(),
                        serde_json::json!({
                            "api_url": "https://example.com",
                            "available_models": [],
                        }),
                    )
                })
                .collect::<serde_json::Map<String, serde_json::Value>>()
                .into()
        }

        let content = serde_json::json!({
            "language_models": {
                "openai_compatible": section(openai),
                "anthropic_compatible": section(anthropic),
            }
        })
        .to_string();
        cx.update_global::<SettingsStore, _>(|store, cx| {
            store
                .set_user_settings(&content, cx)
                .expect("failed to parse test settings");
        });
        CompatibleProviders::from_settings(cx)
    }

    fn provider_icons(registry: &LanguageModelRegistry, id: &str) -> Vec<IconOrSvg> {
        registry
            .providers()
            .into_iter()
            .filter(|provider| provider.id().0.as_ref() == id)
            .map(|provider| provider.icon())
            .collect()
    }

    #[gpui::test]
    async fn test_managed_provider_uses_account_token_for_streaming(cx: &mut TestAppContext) {
        let (client, credentials_provider) = cx.update(init_test);
        let provider = cx.update(|cx| {
            AnthropicCompatibleLanguageModelProvider::new(
                "Fanta".into(),
                client.clone(),
                credentials_provider,
                cx,
            )
        });
        assert!(matches!(
            cx.update(|cx| provider.authenticate(cx)).await,
            Err(AuthenticateError::CredentialsNotFound)
        ));

        client.override_authenticate(|_| {
            gpui::Task::ready(Ok(client::Credentials {
                user_id: 1,
                access_token: "fnt_live_account_token".into(),
            }))
        });
        client
            .sign_in(false, &cx.to_async())
            .await
            .expect("test account sign-in failed");
        cx.update(|cx| provider.authenticate(cx))
            .await
            .expect("account sign-in should authenticate managed AI");
        assert!(cx.update(|cx| provider.is_authenticated(cx)));

        client.http_client().as_fake().replace_handler(|_, mut request| async move {
            assert_eq!(request.uri().to_string(), "https://api.fantaisa.net/v1/messages");
            assert_eq!(request.headers().get("x-api-key").and_then(|value| value.to_str().ok()), Some("fnt_live_account_token"));
            let mut body = String::new();
            request.body_mut().read_to_string(&mut body).await?;
            let body: serde_json::Value = serde_json::from_str(&body)?;
            assert_eq!(body["model"], "claude-sonnet-5");
            assert_eq!(body["stream"], true);
            assert_eq!(body["messages"][0]["content"][0]["text"], "Hello");
            Ok(http_client::Response::builder().status(200).body(
                "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Connected\"}}\n\n".into(),
            )?)
        });

        let model = cx
            .update(|cx| provider.default_model(cx))
            .expect("managed provider should have a default model");
        let mut stream = model
            .stream_completion(
                LanguageModelRequest {
                    messages: vec![LanguageModelRequestMessage {
                        role: Role::User,
                        content: vec![MessageContent::Text("Hello".into())],
                        cache: false,
                        reasoning_details: None,
                    }],
                    ..Default::default()
                },
                &cx.to_async(),
            )
            .await
            .expect("managed request should stream through the backend");
        assert!(matches!(
            stream.next().await,
            Some(Ok(LanguageModelCompletionEvent::Text(text))) if text == "Connected"
        ));

        client.sign_out(&cx.to_async()).await;
        assert!(!cx.update(|cx| provider.is_authenticated(cx)));
        assert!(matches!(
            model
                .stream_completion(LanguageModelRequest::default(), &cx.to_async())
                .await,
            Err(language_model::LanguageModelCompletionError::NoApiKey { .. })
        ));
    }

    #[gpui::test]
    async fn test_account_token_does_not_authenticate_external_provider(cx: &mut TestAppContext) {
        let (client, credentials_provider) = cx.update(init_test);
        cx.update(|cx| {
            update_compatible_provider_settings(&[], &["external"], cx);
        });
        client.override_authenticate(|_| {
            gpui::Task::ready(Ok(client::Credentials {
                user_id: 1,
                access_token: "fnt_live_account_token".into(),
            }))
        });
        client
            .sign_in(false, &cx.to_async())
            .await
            .expect("test account sign-in failed");
        let provider = cx.update(|cx| {
            AnthropicCompatibleLanguageModelProvider::new(
                "external".into(),
                client,
                credentials_provider,
                cx,
            )
        });
        assert!(!cx.update(|cx| provider.is_authenticated(cx)));
        assert!(matches!(
            cx.update(|cx| provider.authenticate(cx)).await,
            Err(AuthenticateError::CredentialsNotFound)
        ));
    }

    #[gpui::test]
    async fn test_managed_provider_reports_credential_storage_errors(cx: &mut TestAppContext) {
        let (client, _) = cx.update(init_test);
        client.override_authenticate(|_| {
            gpui::Task::ready(Ok(client::Credentials {
                user_id: 1,
                access_token: "fnt_live_account_token".into(),
            }))
        });
        client
            .sign_in(false, &cx.to_async())
            .await
            .expect("test account sign-in failed");
        let provider = cx.update(|cx| {
            AnthropicCompatibleLanguageModelProvider::new(
                "Fanta".into(),
                client,
                Arc::new(FakeCredentialsProvider {
                    read_error: Some("keychain unavailable"),
                    ..Default::default()
                }),
                cx,
            )
        });
        let result = cx.update(|cx| provider.authenticate(cx)).await;
        assert!(
            matches!(result, Err(AuthenticateError::Other(error)) if error.to_string() == "keychain unavailable")
        );
    }

    #[gpui::test]
    async fn test_managed_provider_does_not_cache_stored_account_credentials(
        cx: &mut TestAppContext,
    ) {
        let (client, _) = cx.update(init_test);
        let provider = cx.update(|cx| {
            AnthropicCompatibleLanguageModelProvider::new(
                "Fanta".into(),
                client.clone(),
                Arc::new(FakeCredentialsProvider {
                    expected_storage_url: Some("https://api.fantaisa.net/ai-api-key"),
                    stored_credentials: Some((
                        "https://api.fantaisa.net".into(),
                        "1".into(),
                        b"fnt_live_stored_account_token".to_vec(),
                    )),
                    ..Default::default()
                }),
                cx,
            )
        });
        assert!(matches!(
            cx.update(|cx| provider.authenticate(cx)).await,
            Err(AuthenticateError::CredentialsNotFound)
        ));
        assert!(!cx.update(|cx| provider.is_authenticated(cx)));
        cx.update(|cx| provider.set_api_key(Some("fnt_live_explicit_key".into()), cx))
            .await
            .expect("explicit keys should use their own credential storage slot");
        cx.update(|cx| provider.set_api_key(None, cx))
            .await
            .expect("resetting a provider key must not delete account credentials");
    }

    #[gpui::test]
    fn test_compatible_provider_id_collision_resolves_when_one_entry_is_removed(cx: &mut App) {
        let (client, credentials_provider) = init_test(cx);
        let registry = cx.new(|_| LanguageModelRegistry::default());

        // The same provider name is configured in both `openai_compatible`
        // and `anthropic_compatible` settings sections; the OpenAI-compatible
        // entry wins the collision.
        let both = update_compatible_provider_settings(&["acme"], &["acme"], cx);
        registry.update(cx, |registry, cx| {
            register_compatible_providers(
                registry,
                &CompatibleProviders::default(),
                &both,
                &client,
                &credentials_provider,
                cx,
            );
        });
        assert_eq!(
            registry.read_with(cx, |registry, _| provider_icons(registry, "acme")),
            vec![IconOrSvg::Icon(IconName::AiOpenAiCompat)],
            "the OpenAI-compatible provider should win the name collision"
        );

        // The user removes the `anthropic_compatible` entry; the remaining
        // `openai_compatible` entry must stay registered.
        let openai_only = update_compatible_provider_settings(&["acme"], &[], cx);
        registry.update(cx, |registry, cx| {
            register_compatible_providers(
                registry,
                &both,
                &openai_only,
                &client,
                &credentials_provider,
                cx,
            );
        });
        assert_eq!(
            registry.read_with(cx, |registry, _| provider_icons(registry, "acme")),
            vec![IconOrSvg::Icon(IconName::AiOpenAiCompat)],
            "the provider registered for `acme` should be the OpenAI-compatible one"
        );
    }

    #[gpui::test]
    fn test_compatible_provider_changes_kind_and_unregisters(cx: &mut App) {
        let (client, credentials_provider) = init_test(cx);
        let registry = cx.new(|_| LanguageModelRegistry::default());

        let both = update_compatible_provider_settings(&["acme"], &["acme"], cx);
        registry.update(cx, |registry, cx| {
            register_compatible_providers(
                registry,
                &CompatibleProviders::default(),
                &both,
                &client,
                &credentials_provider,
                cx,
            );
        });

        // Removing the `openai_compatible` entry hands the name over to the
        // remaining `anthropic_compatible` entry.
        let anthropic_only = update_compatible_provider_settings(&[], &["acme"], cx);
        registry.update(cx, |registry, cx| {
            register_compatible_providers(
                registry,
                &both,
                &anthropic_only,
                &client,
                &credentials_provider,
                cx,
            );
        });
        assert_eq!(
            registry.read_with(cx, |registry, _| provider_icons(registry, "acme")),
            vec![IconOrSvg::Icon(IconName::AiAnthropicCompat)],
            "after removing the openai_compatible entry, the anthropic_compatible provider should be registered"
        );

        // Removing the last entry unregisters the provider entirely.
        let none = update_compatible_provider_settings(&[], &[], cx);
        registry.update(cx, |registry, cx| {
            register_compatible_providers(
                registry,
                &anthropic_only,
                &none,
                &client,
                &credentials_provider,
                cx,
            );
        });
        assert_eq!(
            registry.read_with(cx, |registry, _| provider_icons(registry, "acme")),
            Vec::new(),
            "removing all entries should unregister the provider"
        );
    }
}
