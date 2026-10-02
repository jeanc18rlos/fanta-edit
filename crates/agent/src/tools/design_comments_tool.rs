use std::sync::Arc;

use crate::{AgentTool, ToolCallEventStream, ToolInput};
use agent_client_protocol::schema::v1 as acp;
use agent_settings::builtin_profiles;
use anyhow::{Context as _, Result};
use gpui::{App, SharedString, Task};
use language_model::LanguageModelToolResultContent;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Read canvas comments or reply to a comment using its exact id.
/// In Review mode, read unresolved comments, address each actionable request
/// in the review, and reply with findings. Resolve a comment only after the
/// requested work is verified. Plan mode permits reading comments only.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DesignCommentsToolInput {
    #[serde(default)]
    pub page: Option<usize>,
    #[serde(default)]
    pub include_resolved: bool,
    /// Omit to read comments; set to reply to this comment.
    #[serde(default)]
    pub comment_id: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub resolve: bool,
}

pub struct DesignCommentsTool;

impl AgentTool for DesignCommentsTool {
    type Input = DesignCommentsToolInput;
    type Output = LanguageModelToolResultContent;

    const NAME: &'static str = "design_comments";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Read
    }

    fn initial_title(
        &self,
        input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        match input {
            Ok(input) if input.comment_id.is_some() => "Reply to canvas comment".into(),
            _ => "Read canvas comments".into(),
        }
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(tool_error)?;
            let value = cx
                .update(|cx| {
                    let surface = design_surface::active(cx)
                        .context("open a Fanta project to read or reply to canvas comments")?;
                    event_stream.report_design_activity(
                        "Reviewing canvas comments",
                        input.page,
                        None,
                        None,
                        cx,
                    );
                    if let Some(comment_id) = input.comment_id {
                        if event_stream.profile_id(cx).as_deref() == Some(builtin_profiles::PLAN) {
                            anyhow::bail!("Plan mode permits reading comments, but cannot reply or resolve them");
                        }
                        let body = input.body.context("a comment reply requires a body")?;
                        anyhow::ensure!(!body.trim().is_empty(), "a comment reply cannot be empty");
                        let (_, agent_name) = event_stream.agent_identity(cx);
                        surface.reply_comment(
                            input.page,
                            comment_id,
                            body,
                            agent_name,
                            input.resolve,
                            cx,
                        )
                    } else {
                        anyhow::ensure!(input.body.is_none() && !input.resolve, "set comment_id to reply or resolve a comment");
                        surface.comments(input.page, input.include_resolved, cx)
                    }
                })
                .map_err(tool_error)?;
            Ok(serde_json::to_string(&value).map_err(tool_error)?.into())
        })
    }
}

fn tool_error(error: impl std::fmt::Display) -> LanguageModelToolResultContent {
    error.to_string().into()
}
