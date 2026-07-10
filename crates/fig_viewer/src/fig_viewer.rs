//! Fanta's Figma canvas: a workspace item that renders `.fig` files and
//! editable Fanta projects, plus the design (layers) and properties dock
//! panels and the floating canvas toolbar.

mod canvas;
mod color_picker;
mod comments;
mod comments_ui;
mod design_panel;
mod document;
mod inspector_widgets;
mod instance_text;
mod panel_settings;
mod properties_ops;
mod properties_panel;
mod properties_render;
mod properties_snapshot;
mod text_edit;
mod tools;
mod view;
mod view_text;

use gpui::{App, AsyncWindowContext, Entity, WeakEntity};
use workspace::{Panel, Workspace};

pub use design_panel::FantaDesignPanel;
pub use document::{DocChange, FigDocument, FigItem, FigItemEvent, FigPage};
pub use panel_settings::{FantaDesignPanelSettings, FantaPropertiesPanelSettings};
pub use properties_panel::FantaPropertiesPanel;
pub use tools::ToolKind;
pub use view::{FigView, FigViewEvent};

pub fn init(cx: &mut App) {
    workspace::register_project_item::<FigView>(cx);
}

pub async fn add_workspace_panels(
    workspace: WeakEntity<Workspace>,
    cx: AsyncWindowContext,
) -> anyhow::Result<()> {
    add_panel_when_ready(
        "Fanta design panel",
        FantaDesignPanel::load(workspace.clone(), cx.clone()),
        workspace.clone(),
        cx.clone(),
    )
    .await;
    add_panel_when_ready(
        "Fanta properties panel",
        FantaPropertiesPanel::load(workspace.clone(), cx.clone()),
        workspace,
        cx,
    )
    .await;

    Ok(())
}

async fn add_panel_when_ready<P: Panel>(
    label: &'static str,
    panel_task: impl std::future::Future<Output = anyhow::Result<Entity<P>>> + 'static,
    workspace: WeakEntity<Workspace>,
    mut cx: AsyncWindowContext,
) {
    match panel_task.await {
        Ok(panel) => {
            if let Err(error) = workspace.update_in(&mut cx, |workspace, window, cx| {
                workspace.add_panel(panel, window, cx);
            }) {
                log::error!("adding {label} failed: {error:#}");
            }
        }
        Err(error) => {
            log::error!("loading {label} failed: {error:#}");
        }
    }
}

/// Whether `FANTA_PERF=1` diagnostics are enabled for this process.
pub(crate) fn perf_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("FANTA_PERF").is_some())
}

/// Log a hot-path duration when perf diagnostics are on and the cost is
/// non-trivial. Run the app with `FANTA_PERF=1` to see where frames go.
pub(crate) fn report_slow(label: &str, started: std::time::Instant) {
    if perf_enabled() {
        let elapsed = started.elapsed();
        if elapsed.as_micros() >= 1_000 {
            log::warn!("fanta perf: {label} took {elapsed:?}");
        }
    }
}
