//! The seam between the agent's design tools and the design editor.
//!
//! `crates/agent` must not depend on the editor (`fig_viewer`/`workspace`), so
//! the editor registers a [`DesignSurface`] provider in a GPUI global and the
//! agent tools resolve it per call. Ops and queries are typed DTOs so tool
//! input schemas stay stable even if the editor's document model evolves.
//! They live in `fanta_design_ops`, which the headless engine shares, and are
//! re-exported here.

use std::{path::PathBuf, rc::Rc};

use anyhow::Result;
use gpui::{App, AppContext as _, Context, Entity, Global, Task};

pub use fanta_design_ops::*;

pub fn agent_color(agent_id: &str) -> gpui::Hsla {
    let identity = agent_id.bytes().fold(0u32, |identity, byte| {
        identity.wrapping_mul(31).wrapping_add(u32::from(byte))
    });
    gpui::rgb(match identity % 4 {
        0 => 0x387bff,
        1 => 0xd83bea,
        2 => 0x008b68,
        _ => 0xc65b06,
    })
    .into()
}

#[derive(Default)]
pub struct AgentActivityState {
    activities: Vec<(AgentActivity, std::time::Instant, Option<[f64; 2]>, u64)>,
    followed_agent: Option<String>,
    revision: u64,
    expiration_tasks: std::collections::BTreeMap<String, Task<()>>,
}

impl AgentActivityState {
    pub fn record(&mut self, activity: AgentActivity, cx: &mut Context<Self>) {
        self.expiration_tasks.remove(&activity.agent_id);
        let previous_world = self.activities.iter().find_map(|(existing, _, _, _)| {
            (existing.agent_id == activity.agent_id
                && existing.project_root == activity.project_root
                && existing.page == activity.page)
                .then_some(existing.world)
                .flatten()
        });
        self.activities.retain(|(existing, recorded, _, _)| {
            existing.agent_id != activity.agent_id && recorded.elapsed().as_secs() < 30
        });
        self.expiration_tasks.retain(|agent_id, _| {
            self.activities
                .iter()
                .any(|(activity, _, _, _)| &activity.agent_id == agent_id)
        });
        if activity.active {
            self.revision = self.revision.wrapping_add(1);
            let recorded = std::time::Instant::now();
            self.schedule_expiration(activity.agent_id.clone(), recorded, cx);
            self.activities
                .push((activity, recorded, previous_world, self.revision));
        }
        cx.notify();
    }

    fn schedule_expiration(
        &mut self,
        agent_id: String,
        recorded: std::time::Instant,
        cx: &mut Context<Self>,
    ) {
        let task = cx.spawn({
            let agent_id = agent_id.clone();
            async move |state, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(30))
                    .await;
                if let Some(state) = state.upgrade() {
                    state.update(cx, |state, cx| {
                        if state
                            .activities
                            .iter()
                            .any(|(activity, last_recorded, _, _)| {
                                activity.agent_id == agent_id && *last_recorded == recorded
                            })
                        {
                            state.stop(&agent_id, cx);
                        }
                    });
                }
            }
        });
        self.expiration_tasks.insert(agent_id, task);
    }

    pub fn activities(&self) -> Vec<AgentActivity> {
        self.activities
            .iter()
            .filter(|(_, recorded, _, _)| recorded.elapsed().as_secs() < 30)
            .map(|(activity, _, _, _)| activity.clone())
            .collect()
    }

    pub fn source_activity(&self, source: &std::path::Path) -> Option<&AgentActivity> {
        self.activities
            .iter()
            .rev()
            .find_map(|(activity, recorded, _, _)| {
                (recorded.elapsed().as_secs() < 30
                    && activity.source_path.as_deref().map(std::path::Path::new) == Some(source))
                .then_some(activity)
            })
    }

    pub fn cursor_motion(&self, agent_id: &str) -> Option<(Option<[f64; 2]>, u64)> {
        self.activities
            .iter()
            .find_map(|(activity, _, previous_world, revision)| {
                (activity.agent_id == agent_id).then_some((*previous_world, *revision))
            })
    }

    pub fn followed_agent(&self) -> Option<&str> {
        self.followed_agent.as_deref()
    }

    pub fn refresh(&mut self, agent_id: &str, cx: &mut Context<Self>) {
        if let Some((_, recorded, _, _)) = self
            .activities
            .iter_mut()
            .find(|(activity, _, _, _)| activity.agent_id == agent_id)
        {
            *recorded = std::time::Instant::now();
            let recorded = *recorded;
            self.schedule_expiration(agent_id.to_string(), recorded, cx);
            cx.notify();
        }
    }

    pub fn stop(&mut self, agent_id: &str, cx: &mut Context<Self>) {
        self.expiration_tasks.remove(agent_id);
        self.activities
            .retain(|(activity, _, _, _)| activity.agent_id != agent_id);
        cx.notify();
    }

    pub fn follow(&mut self, agent: Option<String>, cx: &mut Context<Self>) {
        self.followed_agent = agent;
        cx.notify();
    }
}

struct AgentActivityRegistry(Entity<AgentActivityState>);

impl Global for AgentActivityRegistry {}

pub fn activity_state(cx: &mut App) -> Entity<AgentActivityState> {
    if let Some(registry) = cx.try_global::<AgentActivityRegistry>() {
        return registry.0.clone();
    }
    let state = cx.new(|_| AgentActivityState::default());
    cx.set_global(AgentActivityRegistry(state.clone()));
    state
}

pub fn existing_activity_state(cx: &App) -> Option<Entity<AgentActivityState>> {
    cx.try_global::<AgentActivityRegistry>()
        .map(|registry| registry.0.clone())
}

#[derive(Clone, Debug)]
pub struct LiveMcpCommand {
    pub executable: String,
    pub args: Vec<String>,
}

impl LiveMcpCommand {
    pub fn claude_code_command(&self) -> String {
        let arguments = std::iter::once(self.executable.as_str())
            .chain(self.args.iter().map(String::as_str))
            .map(shell_string)
            .collect::<Vec<_>>()
            .join(" ");
        format!("claude mcp add --scope project --transport stdio fanta -- {arguments}")
    }

    pub fn codex_config(&self) -> String {
        let arguments = self
            .args
            .iter()
            .map(|argument| toml_string(argument))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "[mcp_servers.fanta]\ncommand = {}\nargs = [{arguments}]\n",
            toml_string(&self.executable)
        )
    }
}

fn shell_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn toml_string(value: &str) -> String {
    let mut result = String::from("\"");
    for character in value.chars() {
        match character {
            '\\' => result.push_str("\\\\"),
            '"' => result.push_str("\\\""),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            character if character.is_control() => {
                result.push_str(&format!("\\u{:04X}", u32::from(character)))
            }
            character => result.push(character),
        }
    }
    result.push('"');
    result
}

#[derive(Default)]
struct LiveMcpCommandRegistry(Option<LiveMcpCommand>);

impl Global for LiveMcpCommandRegistry {}

pub fn set_live_mcp_command(command: Option<LiveMcpCommand>, cx: &mut App) {
    cx.set_global(LiveMcpCommandRegistry(command));
}

pub fn live_mcp_command(cx: &App) -> Option<LiveMcpCommand> {
    cx.try_global::<LiveMcpCommandRegistry>()?.0.clone()
}

pub fn observe_live_mcp_command<T: 'static>(cx: &mut Context<T>) -> gpui::Subscription {
    cx.observe_global::<LiveMcpCommandRegistry>(|_, cx| cx.notify())
}

/// The editor-side provider driving the open design document. All methods
/// operate on the most recently focused design canvas.
pub trait DesignSurface: 'static {
    fn prepare_asset(
        &self,
        _request: DesignAssetRequest,
        _cx: &mut App,
    ) -> Result<serde_json::Value> {
        anyhow::bail!("this design surface cannot prepare generation requests")
    }

    fn read_design_spec(&self, _cx: &mut App) -> Result<Option<DesignSpec>> {
        Ok(None)
    }

    /// Return the active saved project's root without constructing its overview.
    /// Prompt preparation calls this before requesting any design metadata.
    fn project_root(&self, _cx: &mut App) -> Result<Option<PathBuf>> {
        Ok(None)
    }

    /// Project/document overview: pages, active page, selection, viewport,
    /// node counts, and editability.
    fn state(&self, cx: &mut App) -> Result<serde_json::Value>;

    /// Node detail (by id) or a compact page tree listing.
    fn get_nodes(&self, query: NodeQuery, cx: &mut App) -> Result<serde_json::Value>;

    fn design_system(&self, _query: DesignSystemQuery, _cx: &mut App) -> Result<serde_json::Value> {
        anyhow::bail!("this design surface cannot inspect a design system")
    }

    /// Apply a batch of ops as one undoable transaction. Returns created node
    /// ids and per-op status; on failure the batch is rolled back.
    fn apply(&self, ops: Vec<DesignOp>, label: String, cx: &mut App) -> Result<serde_json::Value>;

    fn apply_streamed(
        &self,
        ops: Vec<DesignOp>,
        label: String,
        _activity: Option<AgentActivity>,
        cx: &mut App,
    ) -> Task<Result<serde_json::Value>> {
        Task::ready(self.apply(ops, label, cx))
    }

    /// Render a PNG of a page or node region.
    fn screenshot(&self, target: ScreenshotTarget, cx: &mut App) -> Task<Result<Vec<u8>>>;

    /// List the project's FNX source files, or return one file's text.
    fn read_source(&self, path: Option<String>, cx: &mut App) -> Result<serde_json::Value>;

    /// Validate managed FNX and JSON candidates before an agent saves them.
    /// Existing files are validated against the project; new files use their
    /// source schema until their design source pair exists. Validation remains
    /// available while an unsaved draft locks canvas edits.
    fn validate_source_edit(
        &self,
        _path: String,
        _source: String,
        _cx: &mut App,
    ) -> Task<Result<serde_json::Value>> {
        Task::ready(Ok(serde_json::json!({ "applicable": false })))
    }

    fn import_image(
        &self,
        _bytes: Vec<u8>,
        _name: String,
        _cx: &mut App,
    ) -> Task<Result<serde_json::Value>> {
        Task::ready(Err(anyhow::anyhow!(
            "this design surface cannot import images"
        )))
    }

    fn comments(
        &self,
        _page: Option<usize>,
        _include_resolved: bool,
        _cx: &mut App,
    ) -> Result<serde_json::Value> {
        anyhow::bail!("this design surface cannot read comments")
    }

    /// Place a new comment on the canvas as an agent.
    fn add_comment(&self, _draft: CommentDraft, _cx: &mut App) -> Result<serde_json::Value> {
        anyhow::bail!("this design surface cannot add comments")
    }

    fn reply_comment(
        &self,
        _page: Option<usize>,
        _id: String,
        _body: String,
        _author: String,
        _resolve: bool,
        _cx: &mut App,
    ) -> Result<serde_json::Value> {
        anyhow::bail!("this design surface cannot reply to comments")
    }

    fn report_source_activity(
        &self,
        path: String,
        mut activity: AgentActivity,
        cx: &mut App,
    ) -> Result<serde_json::Value> {
        activity.source_path = Some(path);
        self.report_activity(activity, cx)
    }

    fn report_activity(&self, activity: AgentActivity, cx: &mut App) -> Result<serde_json::Value> {
        activity_state(cx).update(cx, |state, cx| state.record(activity, cx));
        Ok(serde_json::json!({"reported": true}))
    }

    /// A free `{x, y}` top-left for a `width` x `height` box on a page
    /// (default the active page): to the right of, or below, everything the
    /// page already holds, with a margin.
    fn find_empty_space(
        &self,
        width: f64,
        height: f64,
        page: Option<usize>,
        cx: &mut App,
    ) -> Result<serde_json::Value>;
}

#[derive(Default)]
struct DesignSurfaceRegistry {
    provider: Option<Rc<dyn DesignSurface>>,
}

impl Global for DesignSurfaceRegistry {}

/// Install `provider` as the process-wide design surface. Called once from the
/// design editor's `init`.
pub fn register(provider: Rc<dyn DesignSurface>, cx: &mut App) {
    cx.set_global(DesignSurfaceRegistry {
        provider: Some(provider),
    });
}

/// The registered design surface, if the editor has installed one.
pub fn active(cx: &App) -> Option<Rc<dyn DesignSurface>> {
    cx.try_global::<DesignSurfaceRegistry>()?.provider.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn source_activity(agent_id: &str, project_root: &str, source: &str) -> AgentActivity {
        AgentActivity {
            agent_id: agent_id.into(),
            agent_name: "Morgana".into(),
            action: "Inspecting source".into(),
            page: None,
            node: None,
            world: None,
            active: true,
            project_root: Some(project_root.into()),
            source_path: Some(source.into()),
            workspace: Some(AgentWorkspace::Code),
        }
    }

    #[gpui::test]
    fn source_activity_tracks_file_changes_and_stops(cx: &mut gpui::TestAppContext) {
        let state = cx.new(|_| AgentActivityState::default());
        let first = Path::new("/project-a/file.rs");
        let second = Path::new("/project-b/file.rs");
        state.update(cx, |state, cx| {
            state.record(
                source_activity("reader", "/project-a", "/project-a/file.rs"),
                cx,
            );
            assert!(state.source_activity(first).is_some());
            assert!(state.source_activity(second).is_none());
            state.record(
                source_activity("reader", "/project-b", "/project-b/file.rs"),
                cx,
            );
            assert!(state.source_activity(first).is_none());
            assert!(state.source_activity(second).is_some());
            state.record(
                AgentActivity {
                    active: false,
                    ..source_activity("reader", "/project-b", "/project-b/file.rs")
                },
                cx,
            );
            assert!(state.source_activity(second).is_none());
        });
    }

    #[gpui::test]
    fn source_activity_expires_and_notifies_observers_after_a_refresh(
        cx: &mut gpui::TestAppContext,
    ) {
        use std::{cell::Cell, time::Duration};

        cx.executor().allow_parking();
        let state = cx.new(|_| AgentActivityState::default());
        let notifications = Rc::new(Cell::new(0));
        let _subscription = cx.update({
            let notifications = notifications.clone();
            |cx| {
                cx.observe(&state, move |_, _| {
                    notifications.set(notifications.get() + 1)
                })
            }
        });
        state.update(cx, |state, cx| {
            state.record(
                source_activity("reader", "/project-a", "/project-a/file.rs"),
                cx,
            );
        });
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_secs(20));
        cx.run_until_parked();
        state.update(cx, |state, cx| state.refresh("reader", cx));
        cx.run_until_parked();
        let notifications_after_refresh = notifications.get();
        cx.executor().advance_clock(Duration::from_secs(11));
        cx.run_until_parked();
        state.read_with(cx, |state, _| {
            assert!(
                state
                    .source_activity(Path::new("/project-a/file.rs"))
                    .is_some()
            );
        });
        assert_eq!(notifications.get(), notifications_after_refresh);
        cx.executor().advance_clock(Duration::from_secs(19));
        cx.run_until_parked();
        state.read_with(cx, |state, _| {
            assert!(state.activities().is_empty());
            assert!(
                state
                    .source_activity(Path::new("/project-a/file.rs"))
                    .is_none()
            );
        });
        assert!(notifications.get() > notifications_after_refresh);
    }

    #[test]
    fn live_mcp_connections_preserve_instance_paths_and_project_scope() {
        let command = LiveMcpCommand {
            executable: "/Applications/Fanta Dev's.app/Contents/MacOS/fanta".into(),
            args: vec![
                "--mcp-stdio".into(),
                "--user-data-dir".into(),
                "/tmp/Fanta \"instance\"".into(),
            ],
        };
        let claude = command.claude_code_command();
        assert!(claude.starts_with("claude mcp add --scope project --transport stdio fanta -- "));
        assert!(claude.contains("'/Applications/Fanta Dev'\\''s.app/Contents/MacOS/fanta'"));
        assert!(claude.contains("'--user-data-dir' '/tmp/Fanta \"instance\"'"));
        let codex = command.codex_config();
        assert!(codex.contains("command = \"/Applications/Fanta Dev's.app/Contents/MacOS/fanta\""));
        assert!(codex.contains("\"--user-data-dir\", \"/tmp/Fanta \\\"instance\\\"\""));
    }
}
