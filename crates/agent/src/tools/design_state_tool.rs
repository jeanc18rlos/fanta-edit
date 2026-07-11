use std::sync::Arc;

use crate::{AgentTool, ToolCallEventStream, ToolInput};
use agent_client_protocol::schema::v1 as acp;
use anyhow::{Context as _, Result};
use design_surface::NodeQuery;
use gpui::{App, SharedString, Task};
use language_model::LanguageModelToolResultContent;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn tool_content_err(e: impl std::fmt::Display) -> LanguageModelToolResultContent {
    LanguageModelToolResultContent::from(e.to_string())
}

/// Read the state of the open design canvas (a `.fig` file or Fanta project).
///
/// With no arguments this returns the document overview: pages, the active
/// page, selection, viewport, and whether the canvas is editable. Pass `page`
/// to list that page's node tree in compact form, or `nodes` to fetch specific
/// nodes in full detail. Read state before editing so node ids and geometry
/// are current.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DesignStateToolInput {
    /// Node ids to fetch in full detail.
    #[serde(default)]
    pub nodes: Option<Vec<String>>,
    /// Page index to list as a compact node tree (defaults to the active page
    /// when only `depth` or `include_geometry` is given).
    #[serde(default)]
    pub page: Option<usize>,
    /// How many levels of children to include below each listed node.
    /// Omit for the full subtree.
    #[serde(default)]
    pub depth: Option<u32>,
    /// Include world-space bounding boxes (default true).
    #[serde(default = "default_true")]
    pub include_geometry: bool,
}

fn default_true() -> bool {
    true
}

pub struct DesignStateTool;

impl AgentTool for DesignStateTool {
    type Input = DesignStateToolInput;
    type Output = LanguageModelToolResultContent;

    const NAME: &'static str = "design_state";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Read
    }

    fn initial_title(
        &self,
        input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        match input {
            Ok(input) if input.nodes.is_some() => "Read design nodes".into(),
            Ok(input) if input.page.is_some() => "List design page".into(),
            _ => "Read design state".into(),
        }
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(tool_content_err)?;
            let value = cx
                .update(|cx| {
                    let surface = design_surface::active(cx).context(
                        "no design canvas is available; ask the user to open a .fig file or Fanta project",
                    )?;
                    if input.nodes.is_some() || input.page.is_some() {
                        surface.get_nodes(
                            NodeQuery {
                                ids: input.nodes.clone(),
                                page: input.page,
                                depth: input.depth,
                                include_geometry: input.include_geometry,
                            },
                            cx,
                        )
                    } else {
                        surface.state(cx)
                    }
                })
                .map_err(tool_content_err)?;
            let text = serde_json::to_string(&value).map_err(tool_content_err)?;
            Ok(text.into())
        })
    }
}
