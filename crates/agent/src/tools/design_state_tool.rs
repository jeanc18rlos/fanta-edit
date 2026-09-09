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
/// With no arguments this returns the document overview: pages (with each
/// page's `.fnx` source path), the active page and its content bounds,
/// components (id, name, source), selection and its bounds, viewport, whether
/// the canvas is editable, and short hints. Pass `page` to list that page's
/// node tree in compact form (each node: id, kind, name, plus a frame's size
/// and auto-layout mode, a text's content and font, a shape's fill, an
/// instance's component), or `nodes` to fetch specific nodes in full detail.
/// Pass `empty_space: [width, height]` to get a free `{x, y}` for a new
/// top-level frame of that size. Read state before editing so node ids and
/// geometry are current; imported pages can hold tens of thousands of nodes,
/// so list with a small `depth` and go deeper by id. A page listing returns at
/// most `limit` (default 200) of the page's direct children starting at
/// `offset`, and reports `child_count`, `children_offset`, `children_limit`
/// and `more_children`; while `more_children` is true, call again with
/// `offset` advanced to see the rest of the page.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DesignStateToolInput {
    /// Node ids to fetch in full detail.
    #[serde(default)]
    pub nodes: Option<Vec<String>>,
    /// Page index to list as a compact node tree (defaults to the active page
    /// when only `depth` or `include_geometry` is given).
    #[serde(default)]
    pub page: Option<usize>,
    /// How many levels of children to include below each listed node. A page
    /// listing defaults to 2 (an imported page can hold tens of thousands of
    /// nodes); fetching by `nodes` defaults to the full subtree. A node cut
    /// off by the limit reports `child_count` instead of `children`.
    #[serde(default)]
    pub depth: Option<u32>,
    /// Include world-space bounding boxes (default true).
    #[serde(default = "default_true")]
    pub include_geometry: bool,
    /// How many of the listed page's direct children to skip before listing
    /// any (default 0). Applies to the page's own children only; nested levels
    /// are governed by `depth`. Ignored when fetching `nodes`.
    #[serde(default)]
    pub offset: Option<usize>,
    /// How many of the listed page's direct children to return, starting at
    /// `offset` (default 200). Applies to the page's own children only; nested
    /// levels are governed by `depth`. When the result says `more_children`,
    /// call again with `offset` advanced by this limit. Ignored when fetching
    /// `nodes`.
    #[serde(default)]
    pub limit: Option<usize>,
    /// `[width, height]` of a box to find a free spot for on the page
    /// (`page`, or the active page). Returns the overview plus
    /// `empty_space: {page, x, y, width, height}`.
    #[serde(default)]
    pub empty_space: Option<[f64; 2]>,
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
            Ok(input) if input.empty_space.is_some() => "Find empty canvas space".into(),
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
            let listing =
                input.empty_space.is_none() && input.nodes.is_none() && input.page.is_some();
            let value = cx
                .update(|cx| {
                    let surface = design_surface::active(cx).context(
                        "no design canvas is available; ask the user to open a .fig file or Fanta project",
                    )?;
                    if let Some([width, height]) = input.empty_space {
                        let mut state = surface.state(cx)?;
                        let spot = surface.find_empty_space(width, height, input.page, cx)?;
                        if let Some(state) = state.as_object_mut() {
                            state.insert("empty_space".into(), spot);
                        }
                        Ok(state)
                    } else if input.nodes.is_some() || input.page.is_some() {
                        surface.get_nodes(
                            NodeQuery {
                                ids: input.nodes.clone(),
                                page: input.page,
                                depth: input.depth.or(if listing { Some(2) } else { None }),
                                include_geometry: input.include_geometry,
                                offset: input.offset,
                                limit: input.limit,
                            },
                            cx,
                        )
                    } else {
                        surface.state(cx)
                    }
                })
                .map_err(tool_content_err)?;
            let text = serde_json::to_string(&value).map_err(tool_content_err)?;
            check_response_size(&text, listing).map_err(tool_content_err)?;
            Ok(text.into())
        })
    }
}

/// Refuse a result the model cannot use — megabytes of JSON for a big page —
/// with instructions for asking a smaller question, the same cap the MCP
/// `batch_get` twin enforces. A refused listing is told about `offset`/`limit`
/// first: it may already be at `depth: 1`, and it cannot name the `nodes` ids
/// that only a listing could have shown it.
fn check_response_size(text: &str, listing: bool) -> Result<()> {
    if text.len() > design_surface::MAX_JSON_RESPONSE_BYTES {
        let hint = if listing {
            "Narrow it: page the children with `limit` and `offset` (e.g. `limit: 50`, then \
             `offset: 50` for the next window), and if that is still too large lower `depth` \
             (try 1) or set `include_geometry` to false."
        } else {
            "Narrow it: ask for fewer `nodes` per call, or set `include_geometry` to false."
        };
        anyhow::bail!(
            "this result would be {} bytes, over the {}-byte response cap. {hint}",
            text.len(),
            design_surface::MAX_JSON_RESPONSE_BYTES
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_results_are_refused_with_a_narrowing_hint() {
        assert!(check_response_size("{}", true).is_ok());
        let oversized = "n".repeat(design_surface::MAX_JSON_RESPONSE_BYTES + 1);
        let error = check_response_size(&oversized, false).expect_err("over the cap");
        assert!(error.to_string().contains("`nodes`"), "{error}");
    }

    /// A refused listing must name pagination: the caller that hit this was
    /// already at the narrowest depth and had no ids to ask by.
    #[test]
    fn a_refused_listing_names_offset_and_limit() {
        let oversized = "n".repeat(design_surface::MAX_JSON_RESPONSE_BYTES + 1);
        let error = check_response_size(&oversized, true).expect_err("over the cap");
        let message = error.to_string();
        assert!(message.contains("`limit`"), "{message}");
        assert!(message.contains("`offset`"), "{message}");
    }
}
