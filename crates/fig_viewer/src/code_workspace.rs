use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use editor::Editor;
use fanta_doc::NodeId;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, IntoElement, Render, SharedString,
    Subscription, Task, Window, div, px,
};
use language::{Buffer, BufferEvent};
use project::Project;
use ui::prelude::*;

use crate::document::{FigItem, FigItemEvent};

const SOURCE_VALIDATION_DEBOUNCE: Duration = Duration::from_millis(220);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CodeWorkspaceFile {
    #[default]
    Fnx,
    Json,
}

impl CodeWorkspaceFile {
    fn label(self) -> &'static str {
        match self {
            Self::Fnx => "FNX",
            Self::Json => "JSON",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Fnx => IconName::FileCode,
            Self::Json => IconName::FileGeneric,
        }
    }
}

pub struct FantaCodeWorkspace {
    item: Entity<FigItem>,
    project: Entity<Project>,
    focus_handle: FocusHandle,
    selected_file: CodeWorkspaceFile,
    requested_page: Option<NodeId>,
    fnx_path: Option<PathBuf>,
    json_path: Option<PathBuf>,
    fnx_buffer: Option<Entity<Buffer>>,
    json_buffer: Option<Entity<Buffer>>,
    fnx_editor: Option<Entity<Editor>>,
    json_editor: Option<Entity<Editor>>,
    loading_fnx: bool,
    loading_json: bool,
    error_message: Option<SharedString>,
    validation_message: Option<SharedString>,
    fnx_load_task: Option<Task<()>>,
    json_load_task: Option<Task<()>>,
    validation_task: Option<Task<()>>,
    fnx_subscription: Option<Subscription>,
    _item_subscription: Subscription,
}

impl FantaCodeWorkspace {
    pub fn new(
        item: Entity<FigItem>,
        project: Entity<Project>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let item_subscription = cx.subscribe_in(
            &item,
            window,
            |this: &mut Self, _, event: &FigItemEvent, window, cx| {
                if matches!(
                    event,
                    FigItemEvent::StateChanged | FigItemEvent::SelectionChanged
                ) {
                    let active_page = this.item.read(cx).doc().and_then(|doc| doc.active_page());
                    if active_page != this.requested_page {
                        this.refresh_page(active_page, window, cx);
                    } else if matches!(event, FigItemEvent::StateChanged)
                        && !this.source_is_dirty(cx)
                    {
                        this.refresh_from_item(window, cx);
                    }
                }
                cx.notify();
            },
        );
        let requested_page = item.read(cx).doc().and_then(|doc| doc.active_page());
        let mut workspace = Self {
            item,
            project,
            focus_handle: cx.focus_handle(),
            selected_file: CodeWorkspaceFile::Fnx,
            requested_page,
            fnx_path: None,
            json_path: None,
            fnx_buffer: None,
            json_buffer: None,
            fnx_editor: None,
            json_editor: None,
            loading_fnx: false,
            loading_json: false,
            error_message: None,
            validation_message: None,
            fnx_load_task: None,
            json_load_task: None,
            validation_task: None,
            fnx_subscription: None,
            _item_subscription: item_subscription,
        };
        workspace.refresh_from_item(window, cx);
        workspace
    }

    pub fn refresh_page(
        &mut self,
        page: Option<NodeId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if page != self.requested_page && self.source_is_dirty(cx) {
            self.error_message =
                Some("Save or discard the current FNX edit before switching pages.".into());
            cx.notify();
            return;
        }
        self.requested_page = page;
        self.refresh_from_item(window, cx);
    }

    pub fn selected_file(&self) -> CodeWorkspaceFile {
        self.selected_file
    }

    pub fn validation_error(&self) -> Option<&str> {
        self.error_message.as_deref()
    }

    pub(crate) fn source_is_dirty(&self, cx: &App) -> bool {
        self.fnx_buffer
            .as_ref()
            .is_some_and(|buffer| buffer.read(cx).is_dirty())
    }

    /// Persist the dirty FNX buffer through the format layer's validated,
    /// atomic source-edit path. The returned document is installed only after
    /// that write succeeds, and the ordinary project-buffer save then records
    /// the exact saved buffer version so Zed's dirty state and the canvas lock
    /// converge with disk.
    pub(crate) fn save_source_edit(&mut self, cx: &mut Context<Self>) -> Option<Task<Result<()>>> {
        if !self.source_is_dirty(cx) {
            return None;
        }
        if self.item.read(cx).is_dirty() {
            return Some(Task::ready(Err(anyhow!(
                "save or discard canvas-authored changes before saving the FNX source"
            ))));
        }
        let Some(project_root) = self.item.read(cx).project_root().map(Path::to_path_buf) else {
            return Some(Task::ready(Err(anyhow!(
                "save the canvas once before saving FNX source"
            ))));
        };
        let Some(source_path) = self.fnx_path.clone() else {
            return Some(Task::ready(Err(anyhow!(
                "the FNX source path is unavailable"
            ))));
        };
        let Some(buffer) = self.fnx_buffer.clone() else {
            return Some(Task::ready(Err(anyhow!("the FNX buffer is unavailable"))));
        };

        let (source, version) = {
            let buffer = buffer.read(cx);
            (buffer.text(), buffer.version())
        };
        self.validation_task = None;
        self.error_message = None;
        self.validation_message = Some("Saving FNX…".into());
        self.item
            .update(cx, |item, _| item.begin_source_edit_save());
        let item = self.item.clone();
        let project = self.project.clone();

        Some(cx.spawn(async move |this, cx| {
            let apply_path = source_path.clone();
            let source_edit = cx
                .background_spawn(async move {
                    fanta_format::apply_project_source_edit(&project_root, &apply_path, &source)
                })
                .await;
            let source_edit = match source_edit {
                Ok(source_edit) => source_edit,
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.validation_message = None;
                        this.error_message = Some(format!("Could not save FNX: {error}").into());
                        cx.notify();
                    })?;
                    return Err(error).context("applying the FNX source edit");
                }
            };

            let buffer_unchanged = buffer.read_with(cx, |buffer, _| buffer.version() == version);
            if !buffer_unchanged {
                // The validated snapshot is now safely on disk, but a newer
                // edit owns the live preview and must remain dirty/locked.
                this.update(cx, |this, cx| {
                    if this.validation_message.as_deref() == Some("Saving FNX…") {
                        this.validation_message = None;
                    }
                    cx.notify();
                })?;
                return Ok(());
            }

            item.update(cx, |item, cx| {
                item.adopt_saved_source_edit(source_edit, cx);
            });
            let save_buffer =
                project.update(cx, |project, cx| project.save_buffer(buffer.clone(), cx));
            if let Err(error) = save_buffer.await {
                this.update(cx, |this, cx| {
                    this.validation_message = None;
                    this.error_message = Some(
                        format!(
                            "FNX was written but its editor state could not be saved: {error:#}"
                        )
                        .into(),
                    );
                    cx.notify();
                })?;
                return Err(error).context("recording the saved FNX buffer version");
            }
            this.update(cx, |this, cx| {
                this.validation_message = None;
                this.error_message = None;
                cx.notify();
            })?;
            Ok(())
        }))
    }

    fn refresh_from_item(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (project_root, page) = {
            let item = self.item.read(cx);
            (
                item.project_root().map(Path::to_path_buf),
                self.requested_page
                    .or_else(|| item.doc().and_then(|doc| doc.active_page())),
            )
        };
        let Some(project_root) = project_root else {
            self.clear_editors();
            self.error_message =
                Some("Save the document once to materialize its FNX and JSON source files.".into());
            cx.notify();
            return;
        };

        match page {
            Some(page) => {
                self.open_fnx(page_source_path(&project_root, page), window, cx);
                self.open_json(page_json_path(&project_root, page), window, cx);
            }
            None => {
                self.fnx_path = None;
                self.json_path = None;
                self.fnx_buffer = None;
                self.json_buffer = None;
                self.fnx_editor = None;
                self.json_editor = None;
                self.fnx_subscription = None;
                self.validation_task = None;
                self.error_message = Some("This document has no page source to display.".into());
            }
        }
        cx.notify();
    }

    fn clear_editors(&mut self) {
        self.fnx_path = None;
        self.json_path = None;
        self.fnx_buffer = None;
        self.json_buffer = None;
        self.fnx_editor = None;
        self.json_editor = None;
        self.loading_fnx = false;
        self.loading_json = false;
        self.fnx_load_task = None;
        self.json_load_task = None;
        self.validation_task = None;
        self.fnx_subscription = None;
    }

    fn open_fnx(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.fnx_path.as_ref() == Some(&path) && self.fnx_editor.is_some() {
            return;
        }
        if self.fnx_path.as_ref() != Some(&path)
            && self
                .fnx_buffer
                .as_ref()
                .is_some_and(|buffer| buffer.read(cx).is_dirty())
        {
            self.error_message =
                Some("Save or discard the current FNX edit before switching pages.".into());
            cx.notify();
            return;
        }
        self.fnx_path = Some(path.clone());
        self.fnx_buffer = None;
        self.fnx_editor = None;
        self.fnx_subscription = None;
        self.validation_task = None;
        self.loading_fnx = true;
        self.error_message = None;
        let open_task = self
            .project
            .update(cx, |project, cx| project.open_local_buffer(&path, cx));
        self.fnx_load_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = open_task.await;
            if let Err(error) = this.update_in(cx, |this, window, cx| {
                if this.fnx_path.as_ref() != Some(&path) {
                    return;
                }
                this.loading_fnx = false;
                match result {
                    Ok(buffer) => this.install_fnx_buffer(buffer, window, cx),
                    Err(error) => {
                        this.error_message =
                            Some(format!("Could not open {}: {error:#}", path.display()).into());
                    }
                }
                cx.notify();
            }) {
                log::debug!("dropping FNX editor load for closed workspace: {error:#}");
            }
        }));
    }

    fn install_fnx_buffer(
        &mut self,
        buffer: Entity<Buffer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor =
            cx.new(|cx| Editor::for_buffer(buffer.clone(), Some(self.project.clone()), window, cx));
        let source_is_dirty = buffer.read(cx).is_dirty();
        self.item.update(cx, |item, cx| {
            item.set_source_edit_locked(source_is_dirty, cx)
        });
        self.fnx_subscription = Some(cx.subscribe_in(
            &buffer,
            window,
            |this: &mut Self, buffer, event: &BufferEvent, window, cx| {
                if matches!(
                    event,
                    BufferEvent::Edited { .. } | BufferEvent::Reloaded | BufferEvent::Saved
                ) {
                    let source_is_dirty = buffer.read(cx).is_dirty();
                    this.item.update(cx, |item, cx| {
                        item.set_source_edit_locked(source_is_dirty, cx)
                    });
                }
                if matches!(event, BufferEvent::Edited { .. } | BufferEvent::Reloaded) {
                    this.schedule_source_validation(cx);
                }
                if matches!(event, BufferEvent::Saved) {
                    let active_page = this.item.read(cx).doc().and_then(|doc| doc.active_page());
                    if active_page != this.requested_page {
                        this.refresh_page(active_page, window, cx);
                    }
                }
            },
        ));
        self.fnx_buffer = Some(buffer);
        self.fnx_editor = Some(editor);
        self.error_message = None;
    }

    fn open_json(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.json_path.as_ref() == Some(&path) && self.json_editor.is_some() {
            return;
        }
        self.json_path = Some(path.clone());
        self.json_buffer = None;
        self.json_editor = None;
        self.loading_json = true;
        let open_task = self
            .project
            .update(cx, |project, cx| project.open_local_buffer(&path, cx));
        self.json_load_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = open_task.await;
            if let Err(error) = this.update_in(cx, |this, window, cx| {
                if this.json_path.as_ref() != Some(&path) {
                    return;
                }
                this.loading_json = false;
                match result {
                    Ok(buffer) => {
                        let editor = cx.new(|cx| {
                            let mut editor = Editor::for_buffer(
                                buffer.clone(),
                                Some(this.project.clone()),
                                window,
                                cx,
                            );
                            editor.set_read_only(true);
                            editor
                        });
                        this.json_buffer = Some(buffer);
                        this.json_editor = Some(editor);
                    }
                    Err(error) => {
                        this.error_message =
                            Some(format!("Could not open {}: {error:#}", path.display()).into());
                    }
                }
                cx.notify();
            }) {
                log::debug!("dropping JSON editor load for closed workspace: {error:#}");
            }
        }));
    }

    fn schedule_source_validation(&mut self, cx: &mut Context<Self>) {
        let Some(project_root) = self.item.read(cx).project_root().map(Path::to_path_buf) else {
            return;
        };
        let Some(source_path) = self.fnx_path.clone() else {
            return;
        };
        let Some(buffer) = self.fnx_buffer.clone() else {
            return;
        };
        if self.item.read(cx).is_dirty() {
            self.error_message =
                Some("Save or discard canvas changes before applying FNX source edits.".into());
            cx.notify();
            return;
        }
        let source = buffer.read(cx).text();
        self.validation_message = Some("Checking FNX…".into());
        self.validation_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(SOURCE_VALIDATION_DEBOUNCE)
                .await;
            let validation_path = source_path.clone();
            let validation = cx
                .background_spawn(async move {
                    fanta_format::validate_project_source_edit(
                        &project_root,
                        &validation_path,
                        &source,
                    )
                })
                .await;
            if let Err(error) = this.update(cx, |this, cx| {
                this.validation_message = None;
                if this.fnx_path.as_ref() != Some(&source_path) {
                    return;
                }
                match validation {
                    Ok(source_edit) => {
                        if this.item.read(cx).is_dirty() {
                            this.error_message = Some(
                                "Canvas content changed while FNX was being checked; save or discard it before retrying."
                                    .into(),
                            );
                            cx.notify();
                            return;
                        }
                        this.error_message = None;
                        let item = this.item.clone();
                        cx.defer(move |cx| {
                            item.update(cx, |item, cx| item.adopt_source_edit(source_edit, cx));
                        });
                    }
                    Err(error) => {
                        this.error_message = Some(format!("FNX error: {error}").into());
                    }
                }
                cx.notify();
            }) {
                log::debug!("dropping FNX validation for closed workspace: {error:#}");
            }
        }));
    }

    fn select_file(
        &mut self,
        file: CodeWorkspaceFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected_file = file;
        if let Some(editor) = self.active_editor() {
            editor.focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    fn active_editor(&self) -> Option<&Entity<Editor>> {
        match self.selected_file {
            CodeWorkspaceFile::Fnx => self.fnx_editor.as_ref(),
            CodeWorkspaceFile::Json => self.json_editor.as_ref(),
        }
    }

    fn render_selector(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut selector = h_flex().gap_1();
        for (index, file) in [CodeWorkspaceFile::Fnx, CodeWorkspaceFile::Json]
            .into_iter()
            .enumerate()
        {
            selector = selector.child(
                Button::new(("fanta-code-file", index), file.label())
                    .size(ButtonSize::Compact)
                    .style(ButtonStyle::Subtle)
                    .start_icon(Icon::new(file.icon()).size(IconSize::XSmall))
                    .toggle_state(file == self.selected_file)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_file(file, window, cx);
                    })),
            );
        }
        selector
    }

    fn render_body(&self, cx: &App) -> AnyElement {
        if let Some(editor) = self.active_editor() {
            return div().size_full().child(editor.clone()).into_any_element();
        }
        let loading = match self.selected_file {
            CodeWorkspaceFile::Fnx => self.loading_fnx,
            CodeWorkspaceFile::Json => self.loading_json,
        };
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .child(
                Label::new(if loading {
                    "Opening source…"
                } else {
                    "Source is unavailable"
                })
                .color(Color::Muted),
            )
            .bg(cx.theme().colors().editor_background)
            .into_any_element()
    }
}

impl Focusable for FantaCodeWorkspace {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.active_editor()
            .map(|editor| editor.focus_handle(cx))
            .unwrap_or_else(|| self.focus_handle.clone())
    }
}

impl Render for FantaCodeWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self
            .error_message
            .clone()
            .map(|message| (message, Color::Error))
            .or_else(|| {
                self.validation_message
                    .clone()
                    .map(|message| (message, Color::Muted))
            });
        v_flex()
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_hidden()
            .bg(cx.theme().colors().editor_background)
            .child(
                h_flex()
                    .h(px(40.))
                    .flex_none()
                    .px_3()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().colors().border)
                    .child(self.render_selector(cx))
                    .when_some(status, |this, (message, color)| {
                        this.child(
                            Label::new(message)
                                .size(LabelSize::Small)
                                .color(color)
                                .single_line(),
                        )
                    }),
            )
            .child(div().flex_1().min_h_0().child(self.render_body(cx)))
    }
}

fn page_source_path(project_root: &Path, page: NodeId) -> PathBuf {
    project_root
        .join("pages")
        .join(page.to_string())
        .join("page.fnx")
}

fn page_json_path(project_root: &Path, page: NodeId) -> PathBuf {
    project_root
        .join("pages")
        .join(page.to_string())
        .join("page.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use fanta_doc::{CanvasNode, Doc, GroupNode, NodeData, Operation};
    use gpui::TestAppContext;
    use project::{ProjectItem as _, ProjectPath};

    fn init_test(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        cx.update(|cx| {
            zlog::init_test();
            assets::Assets.load_test_fonts(cx);
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            release_channel::init(semver::Version::new(0, 0, 0), cx);
            editor::init(cx);
        });
    }

    fn write_project(root: &Path) -> NodeId {
        let mut document = Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "Original".to_owned();
        let page_id = page.id;
        document.scene.insert(page).expect("insert page");
        document.add_page(page_id);
        fanta_format::write_project_tree(root, &document, &BTreeMap::new())
            .expect("write project tree");
        page_id
    }

    async fn open_code_workspace(
        root: &Path,
        cx: &mut TestAppContext,
    ) -> (
        Entity<Project>,
        Entity<FigItem>,
        gpui::WindowHandle<FantaCodeWorkspace>,
    ) {
        let file_system = Arc::new(fs::RealFs::new(None, cx.executor()));
        let project = Project::test(file_system, [root], cx).await;
        let worktree_id = project.update(cx, |project, cx| {
            project
                .worktrees(cx)
                .next()
                .expect("project worktree")
                .read(cx)
                .id()
        });
        let project_path = ProjectPath {
            worktree_id,
            path: util::rel_path::rel_path("fanta.json").into(),
        };
        let item = cx
            .update(|cx| FigItem::try_open(&project, &project_path, cx))
            .expect("fanta.json is a FigItem")
            .await
            .expect("open FigItem");
        cx.run_until_parked();
        let workspace_item = item.clone();
        let workspace_project = project.clone();
        let workspace = cx.add_window(move |window, cx| {
            FantaCodeWorkspace::new(workspace_item, workspace_project, window, cx)
        });
        cx.run_until_parked();
        (project, item, workspace)
    }

    fn replace_workspace_source(
        workspace: gpui::WindowHandle<FantaCodeWorkspace>,
        source: String,
        cx: &mut TestAppContext,
    ) {
        workspace
            .update(cx, |workspace, window, cx| {
                workspace
                    .fnx_editor
                    .as_ref()
                    .expect("FNX editor")
                    .update(cx, |editor, cx| editor.set_text(source, window, cx));
            })
            .expect("update code workspace");
        cx.executor()
            .advance_clock(SOURCE_VALIDATION_DEBOUNCE + Duration::from_millis(1));
        cx.run_until_parked();
    }

    fn page_name(item: &Entity<FigItem>, page: NodeId, cx: &TestAppContext) -> String {
        item.read_with(cx, |item, _| {
            item.doc()
                .and_then(|document| document.scene.get(page))
                .expect("page node")
                .name
                .clone()
        })
    }

    #[test]
    fn source_paths_follow_the_project_layout() {
        let root = Path::new("/tmp/design");
        let page = NodeId::from_u128(7);
        assert_eq!(
            page_source_path(root, page),
            root.join("pages").join(page.to_string()).join("page.fnx")
        );
        assert_eq!(
            page_json_path(root, page),
            root.join("pages").join(page.to_string()).join("page.json")
        );
    }

    #[gpui::test]
    async fn fnx_edits_regenerate_live_and_invalid_source_keeps_the_last_good_canvas(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        let page = write_project(temporary.path());
        let source_path = page_source_path(temporary.path(), page);
        let original_source =
            std::fs::read_to_string(&source_path).expect("read generated FNX source");
        let (_project, item, workspace) = open_code_workspace(temporary.path(), cx).await;

        let changed_source = original_source.replace("name=\"Original\"", "name=\"Changed\"");
        assert_ne!(changed_source, original_source);
        replace_workspace_source(workspace, changed_source, cx);
        assert_eq!(page_name(&item, page, cx), "Changed");
        item.read_with(cx, |item, _| {
            assert!(item.source_edit_locked());
            assert!(!item.is_editable());
        });
        let blocked_mutation = item.update(cx, |item, cx| {
            item.apply(
                Operation::SetName {
                    id: page,
                    old: "Changed".to_owned(),
                    new: "Canvas overwrite".to_owned(),
                },
                cx,
            )
        });
        assert!(blocked_mutation.is_err());
        assert_eq!(page_name(&item, page, cx), "Changed");
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.selection.select_only(page);
                ((), crate::document::DocChange::Selection)
            });
        });
        item.read_with(cx, |item, _| {
            assert_eq!(item.doc().expect("document").selection.as_slice(), &[page]);
        });
        assert_eq!(
            std::fs::read_to_string(&source_path).expect("source remains unsaved"),
            original_source
        );
        workspace
            .read_with(cx, |workspace, _| {
                assert!(workspace.validation_error().is_none());
            })
            .expect("read code workspace");

        replace_workspace_source(workspace, "<Frame>".to_owned(), cx);
        assert_eq!(page_name(&item, page, cx), "Changed");
        item.read_with(cx, |item, _| assert!(item.source_edit_locked()));
        workspace
            .read_with(cx, |workspace, _| {
                assert!(
                    workspace
                        .validation_error()
                        .is_some_and(|error| error.starts_with("FNX error:"))
                );
            })
            .expect("read code workspace");
    }

    #[gpui::test]
    async fn saving_fnx_applies_the_project_source_edit_and_unlocks_the_canvas(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        let page = write_project(temporary.path());
        let source_path = page_source_path(temporary.path(), page);
        let original_source =
            std::fs::read_to_string(&source_path).expect("read generated FNX source");
        let (_project, item, workspace) = open_code_workspace(temporary.path(), cx).await;

        let changed_source = original_source.replace("name=\"Original\"", "name=\"Saved\"");
        replace_workspace_source(workspace, changed_source.clone(), cx);
        assert_eq!(page_name(&item, page, cx), "Saved");
        item.read_with(cx, |item, _| assert!(item.source_edit_locked()));

        let save_task = workspace
            .update(cx, |workspace, _window, cx| {
                workspace
                    .save_source_edit(cx)
                    .expect("dirty source save task")
            })
            .expect("update code workspace");
        save_task.await.expect("save valid FNX source edit");
        cx.run_until_parked();

        assert_eq!(
            std::fs::read_to_string(&source_path).expect("read saved source"),
            changed_source
        );
        let (persisted, _) =
            fanta_format::read_project_tree(temporary.path()).expect("read converged project tree");
        assert_eq!(
            persisted.scene.get(page).expect("persisted page").name,
            "Saved"
        );
        assert_eq!(page_name(&item, page, cx), "Saved");
        item.read_with(cx, |item, _| {
            assert!(!item.source_edit_locked());
            assert!(item.is_editable());
            assert!(!item.has_conflict());
        });
        workspace
            .read_with(cx, |workspace, cx| {
                assert!(!workspace.source_is_dirty(cx));
                assert!(workspace.validation_error().is_none());
            })
            .expect("read code workspace");
    }
}
