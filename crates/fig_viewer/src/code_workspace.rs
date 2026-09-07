use std::path::{Path, PathBuf};

use anyhow::Result;
use editor::Editor;
use fanta_doc::NodeId;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, IntoElement, Render, SharedString,
    Subscription, Task, Window, div, px,
};
use project::Project;
use ui::prelude::*;

use crate::document::{FigItem, FigItemEvent};

/// The alpha ships a code *viewer*: the canvas follows the file on disk, and
/// the file is authored by the agent or by an ordinary editor — never by this
/// pane. Stated once, where the panes are.
const READ_ONLY_STATUS: &str =
    "Read-only. Edit through the agent or your editor; the canvas follows the file.";

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
    fnx_editor: Option<Entity<Editor>>,
    json_editor: Option<Entity<Editor>>,
    loading_fnx: bool,
    loading_json: bool,
    error_message: Option<SharedString>,
    fnx_load_task: Option<Task<()>>,
    json_load_task: Option<Task<()>>,
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
                    } else if matches!(event, FigItemEvent::StateChanged) {
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
            fnx_editor: None,
            json_editor: None,
            loading_fnx: false,
            loading_json: false,
            error_message: None,
            fnx_load_task: None,
            json_load_task: None,
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
        self.requested_page = page;
        self.refresh_from_item(window, cx);
    }

    pub fn selected_file(&self) -> CodeWorkspaceFile {
        self.selected_file
    }

    pub fn validation_error(&self) -> Option<&str> {
        self.error_message.as_deref()
    }

    /// A read-only pane never authors an unsaved source edit, so the view's
    /// dirty state belongs entirely to the canvas document.
    pub(crate) fn source_is_dirty(&self, _cx: &App) -> bool {
        false
    }

    /// External on-disk edits that could not be merged into unsaved canvas
    /// work are still a real conflict; the item owns that flag.
    pub(crate) fn has_source_conflict(&self, cx: &App) -> bool {
        self.item.read(cx).has_conflict()
    }

    /// Nothing to persist: the pane never holds a source edit, so a save is
    /// entirely the canvas document's business.
    pub(crate) fn save_source_edit(&mut self, _cx: &mut Context<Self>) -> Option<Task<Result<()>>> {
        None
    }

    /// Nothing to discard, for the same reason `save_source_edit` has nothing
    /// to save. Reload paths await this before reloading the canvas.
    pub(crate) fn discard_source_edit(&mut self, _cx: &mut Context<Self>) -> Task<Result<()>> {
        Task::ready(Ok(()))
    }

    fn refresh_from_item(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (project_root, page, component) = {
            let item = self.item.read(cx);
            let page = self
                .requested_page
                .or_else(|| item.doc().and_then(|doc| doc.active_page()));
            // The active root may be a component master (a component-scoped
            // view) rather than a listed page; its source is
            // `components/<id>/master.fnx`, with `def.json` as the JSON pane.
            let component = page.and_then(|root| {
                item.doc().and_then(|doc| {
                    doc.components
                        .defs
                        .iter()
                        .find(|(_, def)| def.root == root)
                        .map(|(id, _)| *id)
                })
            });
            (item.project_root().map(Path::to_path_buf), page, component)
        };
        let Some(project_root) = project_root else {
            self.clear_editors();
            self.error_message =
                Some("Save the document once to materialize its FNX and JSON source files.".into());
            cx.notify();
            return;
        };

        // Design directories are named by human-readable slugs (layout v3)
        // with identity in the JSON headers, so source paths are resolved by
        // scanning the tree — never derived from ids.
        match page {
            Some(_) if component.is_some() => {
                let component = component.expect("checked by the match guard");
                let Some(fnx_path) = fanta_format::locate_master_source(&project_root, component)
                else {
                    self.clear_editors();
                    self.error_message = Some(
                        "This component has no source on disk yet; save the document to materialize it.".into(),
                    );
                    cx.notify();
                    return;
                };
                let def_path = fnx_path.with_file_name("def.json");
                self.open_fnx(fnx_path, window, cx);
                self.open_json(def_path, window, cx);
            }
            Some(page) => {
                let Some(fnx_path) = fanta_format::locate_page_source(&project_root, page) else {
                    self.clear_editors();
                    self.error_message = Some(
                        "This page has no source on disk yet; save the document to materialize it."
                            .into(),
                    );
                    cx.notify();
                    return;
                };
                let json_path = fnx_path.with_file_name("page.json");
                self.open_fnx(fnx_path, window, cx);
                self.open_json(json_path, window, cx);
            }
            None => {
                self.fnx_path = None;
                self.json_path = None;
                self.fnx_editor = None;
                self.json_editor = None;
                self.error_message = Some("This document has no page source to display.".into());
            }
        }
        cx.notify();
    }

    fn clear_editors(&mut self) {
        self.fnx_path = None;
        self.json_path = None;
        self.fnx_editor = None;
        self.json_editor = None;
        self.loading_fnx = false;
        self.loading_json = false;
        self.fnx_load_task = None;
        self.json_load_task = None;
    }

    /// Open the page source for *reading*. Opening a design must never write
    /// to the project: no format pass, no save, no legacy-source rewrite —
    /// the file belongs to whoever is authoring it.
    fn open_fnx(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.fnx_path.as_ref() == Some(&path) && self.fnx_editor.is_some() {
            return;
        }
        self.fnx_path = Some(path.clone());
        self.fnx_editor = None;
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
                    Ok(buffer) => {
                        // `Some(project)` is what gives the buffer its
                        // language and syntax highlighting; the editor itself
                        // refuses edits.
                        let project = this.project.clone();
                        let editor = cx.new(|cx| {
                            let mut editor = Editor::for_buffer(buffer, Some(project), window, cx);
                            editor.set_read_only(true);
                            editor
                        });
                        this.fnx_editor = Some(editor);
                    }
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

    fn open_json(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.json_path.as_ref() == Some(&path) && self.json_editor.is_some() {
            return;
        }
        self.json_path = Some(path.clone());
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
                        let project = this.project.clone();
                        let editor = cx.new(|cx| {
                            let mut editor = Editor::for_buffer(buffer, Some(project), window, cx);
                            editor.set_read_only(true);
                            editor
                        });
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
        let error_message = self.error_message.clone();
        let status_color = if error_message.is_some() {
            Color::Error
        } else {
            Color::Muted
        };
        let status = error_message.unwrap_or_else(|| READ_ONLY_STATUS.into());
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
                    .child(
                        div().flex_1().min_w_0().overflow_hidden().child(
                            Label::new(status)
                                .size(LabelSize::Small)
                                .color(status_color)
                                .single_line(),
                        ),
                    ),
            )
            .child(div().flex_1().min_h_0().child(self.render_body(cx)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use fanta_doc::{CanvasNode, Doc, GroupNode, NodeData};
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

    async fn open_test_project(root: &Path, cx: &mut TestAppContext) -> Entity<Project> {
        let file_system = Arc::new(fs::RealFs::new(None, cx.executor()));
        Project::test(file_system, [root], cx).await
    }

    async fn open_code_workspace_for_project(
        project: Entity<Project>,
        cx: &mut TestAppContext,
    ) -> (Entity<FigItem>, gpui::WindowHandle<FantaCodeWorkspace>) {
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
        (item, workspace)
    }

    async fn open_code_workspace(
        root: &Path,
        cx: &mut TestAppContext,
    ) -> (
        Entity<Project>,
        Entity<FigItem>,
        gpui::WindowHandle<FantaCodeWorkspace>,
    ) {
        let project = open_test_project(root, cx).await;
        let (item, workspace) = open_code_workspace_for_project(project.clone(), cx).await;
        (project, item, workspace)
    }

    /// Opening a project source file (`page.fnx`) while the project is open
    /// must reuse the SAME shared item — no reload, one document — and apply
    /// the scope so the click navigates the existing editor to that page.
    #[gpui::test]
    async fn a_scoped_open_reuses_the_shared_project_item_and_applies_the_scope(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        let page = write_project(temporary.path());
        let (project, item, _workspace) = open_code_workspace(temporary.path(), cx).await;

        // Navigate away from the page so the scoped open has something to do.
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.set_active_page(None);
                ((), crate::document::DocChange::Selection)
            });
        });

        let worktree_id = project.update(cx, |project, cx| {
            project
                .worktrees(cx)
                .next()
                .expect("project worktree")
                .read(cx)
                .id()
        });
        let fnx_abs =
            fanta_format::locate_page_source(temporary.path(), page).expect("page source on disk");
        let fnx_rel = fnx_abs
            .strip_prefix(temporary.path())
            .expect("page source inside project")
            .to_str()
            .expect("utf8 path");
        let scoped_path = ProjectPath {
            worktree_id,
            path: util::rel_path::rel_path(fnx_rel).into(),
        };
        let scoped_item = cx
            .update(|cx| FigItem::try_open(&project, &scoped_path, cx))
            .expect("page.fnx routes to the fig viewer")
            .await
            .expect("open scoped FigItem");
        cx.run_until_parked();

        assert_eq!(
            scoped_item.entity_id(),
            item.entity_id(),
            "scoped opens must share the project's one item"
        );
        item.read_with(cx, |item, _| {
            assert_eq!(
                item.doc().and_then(|doc| doc.active_page()),
                Some(page),
                "the scoped open navigates the shared document to its page"
            );
        });
    }

    /// Every open leaves a descriptor recording WHICH entry the user clicked
    /// (the pane's tab-dedupe key, via the view's `project_entry_ids`
    /// override) plus that path's scope. A scoped open must be keyed by the
    /// clicked file's entry — not the shared item's first-open entry — so it
    /// gets its own tab, while REOPENING the same path repeats the same key
    /// and lands on the existing tab. The item itself stays shared.
    #[gpui::test]
    async fn scoped_opens_carry_their_own_entry_and_scope_for_the_view(cx: &mut TestAppContext) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        let page = write_project(temporary.path());
        let (project, item, _workspace) = open_code_workspace(temporary.path(), cx).await;

        let manifest_descriptor = cx
            .update(crate::document::take_pending_view_descriptor)
            .expect("the manifest open leaves a view descriptor");
        assert_eq!(manifest_descriptor.scope, None);
        assert!(
            manifest_descriptor.entry_id.is_some(),
            "the manifest exists in the worktree"
        );

        let worktree_id = project.update(cx, |project, cx| {
            project
                .worktrees(cx)
                .next()
                .expect("project worktree")
                .read(cx)
                .id()
        });
        let fnx_abs =
            fanta_format::locate_page_source(temporary.path(), page).expect("page source on disk");
        let fnx_rel = fnx_abs
            .strip_prefix(temporary.path())
            .expect("page source inside project")
            .to_str()
            .expect("utf8 path");
        let scoped_path = ProjectPath {
            worktree_id,
            path: util::rel_path::rel_path(fnx_rel).into(),
        };
        let scoped_entry = project.update(cx, |project, cx| {
            project
                .entry_for_path(&scoped_path, cx)
                .map(|entry| entry.id)
        });
        assert!(
            scoped_entry.is_some(),
            "the page source exists in the worktree"
        );

        let scoped_item = cx
            .update(|cx| FigItem::try_open(&project, &scoped_path, cx))
            .expect("page.fnx routes to the fig viewer")
            .await
            .expect("open scoped FigItem");
        cx.run_until_parked();
        assert_eq!(
            scoped_item.entity_id(),
            item.entity_id(),
            "scoped opens must share the project's one item"
        );
        let first_open = cx
            .update(crate::document::take_pending_view_descriptor)
            .expect("the scoped open leaves a view descriptor");
        assert_eq!(first_open.entry_id, scoped_entry);
        assert_eq!(
            first_open.scope,
            Some(crate::document::FigScope::Page(page))
        );
        assert_ne!(
            first_open.entry_id, manifest_descriptor.entry_id,
            "a scoped open must not dedupe onto the manifest's tab"
        );

        let reopened_item = cx
            .update(|cx| FigItem::try_open(&project, &scoped_path, cx))
            .expect("page.fnx routes to the fig viewer")
            .await
            .expect("reopen scoped FigItem");
        cx.run_until_parked();
        let second_open = cx
            .update(crate::document::take_pending_view_descriptor)
            .expect("the reopen leaves a view descriptor");
        assert_eq!(
            reopened_item.entity_id(),
            item.entity_id(),
            "reopening still shares the project's one item"
        );
        assert_eq!(
            second_open, first_open,
            "reopening the same path repeats the same tab key"
        );
    }

    /// A focused tab re-asserts its scope on every focus change; when the
    /// document already shows that root the request must be a cheap no-op —
    /// no `ScopeApplied` — or every tab switch would reset sibling viewports.
    #[gpui::test]
    async fn re_asserting_the_current_scope_emits_no_scope_applied(cx: &mut TestAppContext) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        let page = write_project(temporary.path());
        let (_project, item, _workspace) = open_code_workspace(temporary.path(), cx).await;

        let scope_applied = std::rc::Rc::new(std::cell::Cell::new(0_usize));
        let _subscription = cx.update({
            let scope_applied = scope_applied.clone();
            |cx| {
                cx.subscribe(&item, move |_, event, _| {
                    if matches!(event, crate::document::FigItemEvent::ScopeApplied(..)) {
                        scope_applied.set(scope_applied.get() + 1);
                    }
                })
            }
        });

        item.update(cx, |item, cx| {
            item.request_scope(
                crate::document::FigScope::Page(page),
                crate::document::ScopeRequester::Open,
                cx,
            );
        });
        cx.run_until_parked();
        assert_eq!(
            scope_applied.get(),
            0,
            "re-asserting the already-active root must not emit ScopeApplied"
        );

        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.set_active_page(None);
                ((), crate::document::DocChange::Selection)
            });
        });
        item.update(cx, |item, cx| {
            item.request_scope(
                crate::document::FigScope::Page(page),
                crate::document::ScopeRequester::Open,
                cx,
            );
        });
        cx.run_until_parked();
        assert_eq!(
            scope_applied.get(),
            1,
            "an actual re-target still emits ScopeApplied"
        );
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

    fn remove_editor_prelude(source: &str) -> String {
        let mut legacy = source
            .lines()
            .filter(|line| {
                !line.contains("@jsxRuntime classic")
                    && !line.contains("@jsx fnxElement")
                    && !(line.starts_with("import {") && line.ends_with("from \"../../fnx\";"))
            })
            .collect::<Vec<_>>()
            .join("\n");
        legacy.push('\n');
        legacy
    }

    /// Resolve a page's source path in a REAL on-disk project — the slug
    /// layout means paths can't be derived from ids.
    fn page_source_path(project_root: &Path, page: NodeId) -> PathBuf {
        fanta_format::locate_page_source(project_root, page).expect("page source on disk")
    }

    fn page_json_path(project_root: &Path, page: NodeId) -> PathBuf {
        page_source_path(project_root, page).with_file_name("page.json")
    }

    /// The one singleton buffer behind a pane's editor, for asserting that
    /// opening it left nothing dirty.
    fn editor_buffer_is_dirty(editor: &Entity<Editor>, cx: &App) -> bool {
        editor
            .read(cx)
            .buffer()
            .read(cx)
            .as_singleton()
            .expect("a code pane shows one buffer")
            .read(cx)
            .is_dirty()
    }

    #[test]
    fn source_paths_follow_the_project_layout() {
        // Design dirs are slug-named (layout v3); paths resolve by scanning,
        // not by id. The resolved page source sits under pages/<slug>/page.fnx.
        let temporary = tempfile::tempdir().expect("temporary project");
        let page = write_project(temporary.path());
        let fnx = page_source_path(temporary.path(), page);
        assert!(fnx.ends_with(Path::new("page.fnx")), "{}", fnx.display());
        assert!(
            fnx.parent()
                .and_then(|dir| dir.parent())
                .is_some_and(|pages| pages.ends_with("pages")),
            "{}",
            fnx.display()
        );
        assert_eq!(
            page_json_path(temporary.path(), page),
            fnx.with_file_name("page.json")
        );
    }

    /// Both panes are viewers. Anything the user typed here could only
    /// diverge from the file the canvas actually follows.
    #[gpui::test]
    async fn both_code_editors_are_read_only(cx: &mut TestAppContext) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        write_project(temporary.path());
        let (_project, _item, workspace) = open_code_workspace(temporary.path(), cx).await;

        workspace
            .read_with(cx, |workspace, cx| {
                let fnx_editor = workspace.fnx_editor.as_ref().expect("FNX editor");
                let json_editor = workspace.json_editor.as_ref().expect("JSON editor");
                assert!(
                    fnx_editor.read(cx).read_only(cx),
                    "the FNX pane is a viewer"
                );
                assert!(
                    json_editor.read(cx).read_only(cx),
                    "the JSON pane is a viewer"
                );
            })
            .expect("read code workspace");
    }

    /// Opening a design must be a pure read. The old open path ran a format
    /// pass, saved the reformatted buffer and canonicalized legacy sources —
    /// starting language servers and REWRITING the author's file as a side
    /// effect of merely looking at it.
    #[gpui::test]
    async fn opening_the_code_pane_does_not_rewrite_the_fnx_on_disk(cx: &mut TestAppContext) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        let page = write_project(temporary.path());
        let source_path = page_source_path(temporary.path(), page);
        // A legacy source (no editor prelude) is exactly what the removed
        // upgrade path used to canonicalize and write back.
        let legacy_source = remove_editor_prelude(
            &std::fs::read_to_string(&source_path).expect("read generated FNX source"),
        );
        std::fs::write(&source_path, &legacy_source).expect("write legacy FNX source");
        let editor_types_path = temporary.path().join("fnx.d.ts");
        if editor_types_path.exists() {
            std::fs::remove_file(&editor_types_path).expect("remove seeded editor types");
        }
        let formatter_settings_path = temporary.path().join(".prettierrc.json");
        if formatter_settings_path.exists() {
            std::fs::remove_file(&formatter_settings_path)
                .expect("remove seeded formatter settings");
        }

        let (_project, _item, workspace) = open_code_workspace(temporary.path(), cx).await;
        cx.run_until_parked();

        workspace
            .read_with(cx, |workspace, cx| {
                let fnx_editor = workspace.fnx_editor.as_ref().expect("FNX editor");
                assert!(
                    !editor_buffer_is_dirty(fnx_editor, cx),
                    "opening must not leave an unsaved in-memory rewrite either"
                );
            })
            .expect("read code workspace");
        assert_eq!(
            std::fs::read_to_string(&source_path).expect("read FNX source after opening"),
            legacy_source,
            "opening the code pane must not rewrite the source"
        );
        assert!(
            !editor_types_path.exists() && !formatter_settings_path.exists(),
            "opening the code pane must not seed editor-support files into the project"
        );
    }

    /// The alpha's code→canvas loop runs through the FILE: an agent (or an
    /// ordinary editor) writes `.fnx`, and the canvas picks the change up on
    /// reload. The code pane never locks the canvas while that happens.
    #[gpui::test]
    async fn agent_written_fnx_on_disk_still_reaches_the_canvas(cx: &mut TestAppContext) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        let page = write_project(temporary.path());
        let source_path = page_source_path(temporary.path(), page);
        let original_source =
            std::fs::read_to_string(&source_path).expect("read generated FNX source");
        let (_project, item, workspace) = open_code_workspace(temporary.path(), cx).await;
        assert_eq!(page_name(&item, page, cx), "Original");

        let changed_source = original_source.replace("name=\"Original\"", "name=\"Agent edit\"");
        assert_ne!(changed_source, original_source);
        std::fs::write(&source_path, &changed_source).expect("agent rewrites the FNX source");

        let reload = item.update(cx, |item, cx| item.reload_from_disk(cx));
        reload.await.expect("reload the canvas from disk");
        cx.run_until_parked();

        assert_eq!(page_name(&item, page, cx), "Agent edit");
        item.read_with(cx, |item, _| {
            assert!(
                !item.source_edit_locked(),
                "the read-only code pane never locks the canvas"
            );
            assert!(item.is_editable());
        });
        workspace
            .read_with(cx, |workspace, cx| {
                assert!(!workspace.source_is_dirty(cx));
                assert!(!workspace.has_source_conflict(cx));
                assert!(workspace.validation_error().is_none());
            })
            .expect("read code workspace");
    }
}
