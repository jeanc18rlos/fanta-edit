//! Fanta's Figma canvas: a workspace item that renders `.fig` files and
//! editable Fanta projects, plus the design (layers) and properties dock
//! panels and the floating canvas toolbar.

mod agent_surface;
mod canvas;
mod clipboard;
mod code_workspace;
mod color_picker;
mod comments;
mod comments_panel;
mod comments_ui;
mod component_properties;
mod design_panel;
mod document;
mod editor_session;
mod export;
mod generation_journal;
mod generation_workspace;
mod generation_media;
#[cfg(feature = "fanta-gpui-ui")]
mod gpui_adapters;
mod inspector_components;
mod inspector_widgets;
mod instance_text;
mod live_mcp;
mod mode_overrides;
mod motion_edit;
mod motion_panel;
mod new_design;
mod panel_settings;
mod properties_ops;
mod properties_panel;
mod properties_render;
mod properties_snapshot;
mod prototype_panel;
mod prototype_player;
mod structure;
mod text_edit;
#[cfg(feature = "fanta-gpui-ui")]
mod theme_bridge;
mod timeline;
mod tools;
mod variable_binding;
mod variables_workspace;
mod view;
mod view_text;
mod workspace_hooks;

use gpui::{App, AsyncWindowContext, Entity, WeakEntity};
use workspace::{Panel, Workspace};

pub use code_workspace::{CodeWorkspaceFile, FantaCodeWorkspace};
pub use design_panel::FantaDesignPanel;
pub use document::{DocChange, FigDocument, FigItem, FigItemEvent, FigPage, ScopeRequester};
pub use editor_session::{
    EditorMode, EditorModeTabs, EditorSession, EditorSessionEvent, EditorWorkspace,
    EditorWorkspaceTabs,
};
pub use inspector_components::{InspectorMessage, InspectorPropertyRow, InspectorSectionHeader};
pub use motion_panel::FantaMotionPanel;
pub use panel_settings::{FantaDesignPanelSettings, FantaPropertiesPanelSettings};
pub use properties_panel::FantaPropertiesPanel;
pub use prototype_panel::FantaPrototypePanel;
pub use timeline::{
    TimelineEditPhase, TimelineEvent, TimelineKeyframeSelection, TimelineKeyframeViewModel,
    TimelineProperty, TimelineShell, TimelineTrackViewModel, TimelineViewModel,
};
pub use tools::ToolKind;
pub use variables_workspace::FantaVariablesWorkspace;
pub use view::{
    FigView, FigViewEvent, FitToView, ResetZoom, SelectAll, ToggleInspectorSidebar,
    ToggleLayersSidebar, ZoomIn, ZoomOut, ZoomToSelection,
};

pub fn init(cx: &mut App) {
    #[cfg(feature = "fanta-gpui-ui")]
    {
        gpui_component::init(cx);
        fanta_gpui::init(cx);
        theme_bridge::init(cx);
    }
    agent_surface::init(cx);
    live_mcp::init(cx);
    workspace::register_project_item::<FigView>(cx);
    workspace_hooks::init(cx);
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
