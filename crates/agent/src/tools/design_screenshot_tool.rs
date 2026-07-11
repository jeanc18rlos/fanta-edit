use std::sync::Arc;

use crate::{AgentTool, ToolCallEventStream, ToolInput};
use agent_client_protocol::schema::v1 as acp;
use anyhow::{Context as _, Result};
use base64::Engine as _;
use design_surface::ScreenshotTarget;
use gpui::{App, SharedString, Task};
use language_model::{LanguageModelImage, LanguageModelToolResultContent};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn tool_content_err(e: impl std::fmt::Display) -> LanguageModelToolResultContent {
    LanguageModelToolResultContent::from(e.to_string())
}

/// Render a PNG screenshot of the open design canvas so you can see what a
/// page or node actually looks like.
///
/// Defaults to the active page; pass `node` to render just that node's region,
/// or `page` for another page. Screenshot after `design_edit` batches to
/// verify the result visually instead of trusting coordinates.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DesignScreenshotToolInput {
    /// Page index to render (defaults to the active page).
    #[serde(default)]
    pub page: Option<usize>,
    /// Render only this node's region of its page.
    #[serde(default)]
    pub node: Option<String>,
    /// Cap on the longer output dimension in pixels (default 1024).
    #[serde(default)]
    pub max_dimension: Option<u32>,
}

pub struct DesignScreenshotTool;

impl AgentTool for DesignScreenshotTool {
    type Input = DesignScreenshotToolInput;
    type Output = LanguageModelToolResultContent;

    const NAME: &'static str = "design_screenshot";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Read
    }

    fn initial_title(
        &self,
        input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        match input {
            Ok(input) if input.node.is_some() => "Screenshot design node".into(),
            _ => "Screenshot design page".into(),
        }
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(tool_content_err)?;
            let render = cx
                .update(|cx| {
                    let surface = design_surface::active(cx).context(
                        "no design canvas is available; ask the user to open a .fig file or Fanta project",
                    )?;
                    anyhow::Ok(surface.screenshot(
                        ScreenshotTarget {
                            page: input.page,
                            node: input.node.clone(),
                            max_dimension: input.max_dimension,
                        },
                        cx,
                    ))
                })
                .map_err(tool_content_err)?;
            let png = render.await.map_err(tool_content_err)?;
            let image = LanguageModelImage {
                source: base64::engine::general_purpose::STANDARD
                    .encode(&png)
                    .into(),
            };
            emit_image(&image, &event_stream);
            Ok(image.into())
        })
    }

    fn replay(
        &self,
        _input: Self::Input,
        output: Self::Output,
        event_stream: ToolCallEventStream,
        _cx: &mut App,
    ) -> Result<()> {
        if let LanguageModelToolResultContent::Image(image) = &output {
            emit_image(image, &event_stream);
        }
        Ok(())
    }
}

/// Show the rendered screenshot inline in the thread.
fn emit_image(image: &LanguageModelImage, event_stream: &ToolCallEventStream) {
    event_stream.update_fields(acp::ToolCallUpdateFields::new().content(vec![
        acp::ToolCallContent::Content(acp::Content::new(acp::ContentBlock::Image(
            acp::ImageContent::new(image.source.clone(), "image/png"),
        ))),
    ]));
}
