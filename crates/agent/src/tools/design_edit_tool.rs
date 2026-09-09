use std::sync::Arc;

use crate::{AgentTool, ToolCallEventStream, ToolInput};
use agent_client_protocol::schema::v1 as acp;
use anyhow::{Context as _, Result};
use design_surface::DesignOp;
use gpui::{App, SharedString, Task};
use language_model::LanguageModelToolResultContent;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn tool_content_err(e: impl std::fmt::Display) -> LanguageModelToolResultContent {
    LanguageModelToolResultContent::from(e.to_string())
}

/// Edit the open design canvas by applying a batch of ops in order.
///
/// Create: `create_node` (frame, rectangle, ellipse, text), `create_image`
/// (base64 or data: URI source; becomes a project asset + bitmap layer),
/// `create_instance` (of a component by id or unique name), `duplicate`.
/// Style: `set_props` (name, x, y, width, height, opacity, fill,
/// corner_radius, text, hidden, locked), `set_stroke`, `set_shadow`,
/// `set_text_style` (family, weight, size, line height, letter spacing,
/// align, color), `set_auto_layout` (direction, gap, padding, align_items,
/// justify; `none` turns it off). Arrange: `rotate`, `align`, `distribute`,
/// `set_index` (z-order), `reparent`. Structure: `group`, `frame_selection`,
/// `ungroup`, `create_component`, `delete`. Editor: `select`, `set_viewport`.
///
/// The batch is one undoable transaction: if any op fails, everything rolls
/// back and the result reports the failing op so it can be corrected. `x`/`y`
/// are world (canvas) coordinates of a node's top-left corner (y grows down);
/// ids are exact node ids from `design_state`. Created node ids are returned
/// in `created` in creation order (`group`/`frame_selection`/`duplicate`/
/// `create_instance` report their new node there; `ungroup` reports freed
/// `children`; `create_component` reports the `component` id). Verify the
/// result with `design_screenshot` after substantive edits. To place an image
/// from a URL (e.g. a finished AI generation), use `place_generation` instead.
/// Gradients, variables and per-run rich text are not ops: edit the page's
/// `.fnx` source with the file tools when the project is on disk.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DesignEditToolInput {
    /// The ops to apply, in order.
    #[serde(deserialize_with = "crate::tools::deserialize_maybe_stringified")]
    pub ops: Vec<DesignOp>,
    /// Undo-history label for the batch (e.g. "Add login card").
    #[serde(default)]
    pub label: Option<String>,
}

pub struct DesignEditTool;

impl AgentTool for DesignEditTool {
    type Input = DesignEditToolInput;
    type Output = LanguageModelToolResultContent;

    const NAME: &'static str = "design_edit";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Edit
    }

    fn initial_title(
        &self,
        input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        match input {
            Ok(input) => match (input.label, input.ops.len()) {
                (Some(label), _) => format!("Edit design: {label}").into(),
                (None, 1) => "Edit design (1 op)".into(),
                (None, count) => format!("Edit design ({count} ops)").into(),
            },
            Err(_) => "Edit design".into(),
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
            let label = input.label.unwrap_or_else(|| "Agent edit".to_string());
            let value = cx
                .update(|cx| {
                    let surface = design_surface::active(cx).context(
                        "no design canvas is available; ask the user to open a .fig file or Fanta project",
                    )?;
                    surface.apply(input.ops, label, cx)
                })
                .map_err(tool_content_err)?;
            let applied = value
                .get("applied")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let text: LanguageModelToolResultContent = serde_json::to_string(&value)
                .map_err(tool_content_err)?
                .into();
            // A rolled-back batch surfaces as a tool error so the model
            // repairs the failing op; the per-op statuses say which one.
            if applied { Ok(text) } else { Err(text) }
        })
    }
}
