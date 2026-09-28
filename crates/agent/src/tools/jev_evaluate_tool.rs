use std::collections::BTreeMap;
use std::sync::Arc;

use agent_client_protocol::schema::v1 as acp;
use anyhow::{Context as _, Result, ensure};
use client::{Client, ClientSettings};
use futures::AsyncReadExt as _;
use gpui::{App, Task};
use http_client::{AsyncBody, HttpClient as _, Method, Request};
use language_model::LanguageModelToolResultContent;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use settings::Settings as _;
use ui::SharedString;

use crate::{AgentTool, ToolCallEventStream, ToolInput};

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum JevQuestion {
    Boolean {
        instructions: String,
    },
    Choice {
        instructions: String,
        /// Two or more named options and when each should be selected.
        criteria: BTreeMap<String, String>,
    },
    Score {
        instructions: String,
        /// Two to ten ordered descriptions, from lowest to highest.
        criteria: Vec<String>,
    },
}

/// Ask TypeSafe AI Jev for typed decisions about supplied text or structured
/// state. Jev returns probabilities, categories, or rubric scores; it does not
/// write chat replies. Use it to classify a request, choose among explicit
/// next steps, or check a completed result. A successful call costs one Fanta
/// credit minimum. State and questions together are limited to 8 KiB.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct JevEvaluateToolInput {
    /// Text, object, or array to evaluate.
    pub state: Value,
    /// One to eight named boolean, choice, or score questions.
    pub questions: BTreeMap<String, JevQuestion>,
}

pub struct JevEvaluateTool {
    client: Arc<Client>,
}

impl JevEvaluateTool {
    pub fn new(client: Arc<Client>) -> Self {
        Self { client }
    }
}

impl AgentTool for JevEvaluateTool {
    type Input = JevEvaluateToolInput;
    type Output = LanguageModelToolResultContent;

    const NAME: &'static str = "evaluate_with_jev";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Other
    }

    fn initial_title(&self, _input: Result<Self::Input, Value>, _cx: &mut App) -> SharedString {
        "Evaluate with Jev".into()
    }

    fn allow_in_restricted_mode() -> bool {
        false
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        let client = self.client.clone();
        cx.spawn(async move |cx| {
            let input = input
                .recv()
                .await
                .map_err(|error| LanguageModelToolResultContent::from(error.to_string()))?;
            let body = serde_json::to_vec(&input)
                .map_err(|error| LanguageModelToolResultContent::from(error.to_string()))?;
            if body.len() > 8192 {
                return Err("Jev evaluation state and questions exceed 8 KiB.".into());
            }
            let (api_url, token) = cx.update(|cx| {
                (
                    ClientSettings::get_global(cx).server_url.clone(),
                    client.account_access_token(),
                )
            });
            let Some(token) = token else {
                return Err("Sign in to Fanta before using Jev evaluation.".into());
            };
            let result = evaluate(client, &api_url, &token, body).await;
            result
                .map(|value| value.into())
                .map_err(|error| LanguageModelToolResultContent::from(error.to_string()))
        })
    }
}

async fn evaluate(
    client: Arc<Client>,
    api_url: &str,
    token: &str,
    body: Vec<u8>,
) -> Result<String> {
    let request = Request::builder()
        .method(Method::POST)
        .uri(format!("{}/v1/evaluate", api_url.trim_end_matches('/')))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .body(AsyncBody::from(body))?;
    let response = client.http_client().send(request).await?;
    let status = response.status();
    let mut bytes = Vec::new();
    response
        .into_body()
        .take(64 * 1024)
        .read_to_end(&mut bytes)
        .await
        .context("reading Jev evaluation response")?;
    let value: Value = serde_json::from_slice(&bytes).context("invalid Jev evaluation response")?;
    ensure!(
        status.is_success(),
        "Jev evaluation failed with HTTP {status}: {}",
        value["error"]["message"]
            .as_str()
            .unwrap_or("unknown error")
    );
    ensure!(
        value["answers"].is_object(),
        "Jev returned no typed answers"
    );
    Ok(serde_json::to_string(&value)?)
}
