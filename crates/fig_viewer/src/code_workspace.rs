use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use buffer_diff::BufferDiff;
use editor::Editor;
use fanta_doc::NodeId;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, IntoElement, Render, SharedString,
    Subscription, Task, Window, div, px,
};
use language::{Buffer, BufferEvent};
use project::{
    Project,
    lsp_store::{FormatTrigger, LspFormatTarget},
};
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
    canvas_diff: Option<Entity<BufferDiff>>,
    canvas_diff_task: Option<Task<()>>,
    previous_fnx_diff: Option<Entity<BufferDiff>>,
    canvas_diff_visible: bool,
    source_save_in_progress: bool,
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
                if matches!(event, FigItemEvent::StateChanged) {
                    if this.has_local_canvas_source_conflict(cx) && this.canvas_diff_visible {
                        this.show_canvas_diff(cx);
                    } else if !this.has_local_canvas_source_conflict(cx) {
                        this.hide_canvas_diff(cx);
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
            canvas_diff: None,
            canvas_diff_task: None,
            previous_fnx_diff: None,
            canvas_diff_visible: false,
            source_save_in_progress: false,
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

    pub(crate) fn has_source_conflict(&self, cx: &App) -> bool {
        let item = self.item.read(cx);
        item.has_conflict() || (item.is_dirty() && self.source_is_dirty(cx))
    }

    fn has_local_canvas_source_conflict(&self, cx: &App) -> bool {
        let item = self.item.read(cx);
        is_exclusively_local_canvas_source_conflict(
            item.has_conflict(),
            item.is_dirty(),
            self.source_is_dirty(cx),
        )
    }

    pub(crate) fn discard_source_edit(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        if self.source_save_in_progress {
            return Task::ready(Err(anyhow!(
                "wait for the current FNX save to finish before discarding it"
            )));
        }
        let Some(buffer) = self.fnx_buffer.clone() else {
            self.item
                .update(cx, |item, cx| item.set_source_edit_locked(false, cx));
            return Task::ready(Ok(()));
        };
        if !buffer.read(cx).is_dirty() {
            self.item
                .update(cx, |item, cx| item.set_source_edit_locked(false, cx));
            self.validation_task = None;
            self.validation_message = None;
            self.error_message = None;
            cx.notify();
            return Task::ready(Ok(()));
        }

        self.validation_task = None;
        self.validation_message = Some("Discarding FNX changes…".into());
        self.error_message = None;
        let reload = self.project.update(cx, |project, cx| {
            project.reload_buffers(std::iter::once(buffer.clone()).collect(), false, cx)
        });
        let item = self.item.clone();
        cx.spawn(async move |this, cx| {
            reload.await.context("reloading FNX from disk")?;
            let source_is_dirty = buffer.read_with(cx, |buffer, _| buffer.is_dirty());
            item.update(cx, |item, cx| {
                item.set_source_edit_locked(source_is_dirty, cx)
            });
            this.update(cx, |this, cx| {
                this.validation_message = None;
                if source_is_dirty {
                    this.error_message =
                        Some("FNX changed again while its edits were being discarded.".into());
                } else {
                    this.error_message = None;
                }
                cx.notify();
            })?;
            if source_is_dirty {
                Err(anyhow!("FNX changed while its edits were being discarded"))
            } else {
                Ok(())
            }
        })
    }

    fn current_canvas_source(&self, cx: &App) -> Result<String> {
        let item = self.item.read(cx);
        let document = item.doc().context("the canvas document is unavailable")?;
        let root = self
            .requested_page
            .or_else(|| document.active_page())
            .context("the current page is unavailable")?;
        let root_node = document
            .scene
            .get(root)
            .context("the current page root is unavailable")?;
        let component_roots: HashSet<_> = document
            .components
            .defs
            .values()
            .map(|component| component.root)
            .collect();
        // Mirror the project writer's bucketing: a node belongs to the design
        // of its NEAREST component-root ancestor-or-self, or to its page when
        // it has none. A page encoding therefore excludes every master
        // subtree, and a component-scope encoding (root is a master) keeps
        // exactly its own subtree minus any nested other master.
        let nearest_component_root = |node: NodeId| -> Option<NodeId> {
            if component_roots.contains(&node) {
                return Some(node);
            }
            document
                .scene
                .ancestors_of(node)
                .map(|ancestor| ancestor.id)
                .find(|id| component_roots.contains(id))
        };
        let scope_component_root = component_roots.contains(&root).then_some(root);
        let nodes = document
            .scene
            .descendants_of(root)
            .filter(|node| nearest_component_root(*node) == scope_component_root)
            .filter_map(|node| document.scene.get(node))
            .map(serde_json::to_value)
            .collect::<serde_json::Result<Vec<_>>>()
            .context("serializing the canvas page")?;
        let function_name = root_node.name.trim();
        let function_name = if function_name.is_empty() {
            if scope_component_root.is_some() {
                "Component"
            } else {
                "Page"
            }
        } else {
            function_name
        };
        let (source, _) = fanta_fnx::encode_subtree(&nodes, function_name)
            .map_err(|error| anyhow!("encoding the canvas page as FNX: {error}"))?;
        Ok(source)
    }

    fn show_canvas_diff(&mut self, cx: &mut Context<Self>) {
        if !self.has_local_canvas_source_conflict(cx) {
            self.hide_canvas_diff(cx);
            return;
        }
        let Some(buffer) = self.fnx_buffer.clone() else {
            self.error_message = Some("The FNX buffer is unavailable.".into());
            cx.notify();
            return;
        };
        let Some(editor) = self.fnx_editor.clone() else {
            self.error_message = Some("The FNX editor is unavailable.".into());
            cx.notify();
            return;
        };
        let source = match self.current_canvas_source(cx) {
            Ok(source) => source,
            Err(error) => {
                self.error_message =
                    Some(format!("Could not compare with canvas: {error:#}").into());
                cx.notify();
                return;
            }
        };
        let buffer_snapshot = buffer.read(cx).text_snapshot();
        let diff = self
            .canvas_diff
            .clone()
            .unwrap_or_else(|| cx.new(|cx| BufferDiff::new(&buffer_snapshot, None, None, cx)));
        if !self.canvas_diff_visible {
            let multi_buffer = editor.read(cx).buffer().clone();
            self.previous_fnx_diff = multi_buffer
                .read(cx)
                .diff_for(buffer_snapshot.remote_id())
                .filter(|existing| existing.entity_id() != diff.entity_id());
            multi_buffer.update(cx, |multi_buffer, cx| {
                multi_buffer.add_diff(diff.clone(), cx);
                multi_buffer.set_all_diff_hunks_expanded(cx);
            });
            self.canvas_diff_visible = true;
        }
        self.canvas_diff = Some(diff.clone());
        self.canvas_diff_task = Some(diff.update(cx, |diff, cx| {
            diff.set_base_text(Some(Arc::from(source)), buffer_snapshot, cx)
        }));
        cx.notify();
    }

    fn hide_canvas_diff(&mut self, cx: &mut Context<Self>) {
        if !self.canvas_diff_visible {
            return;
        }
        let Some(buffer) = self.fnx_buffer.clone() else {
            self.canvas_diff_visible = false;
            self.previous_fnx_diff = None;
            return;
        };
        let Some(editor) = self.fnx_editor.clone() else {
            self.canvas_diff_visible = false;
            self.previous_fnx_diff = None;
            return;
        };
        let multi_buffer = editor.read(cx).buffer().clone();
        if let Some(previous) = self.previous_fnx_diff.take() {
            multi_buffer.update(cx, |multi_buffer, cx| {
                multi_buffer.add_diff(previous, cx);
            });
        } else if let Some(diff) = self.canvas_diff.clone() {
            let buffer_snapshot = buffer.read(cx).text_snapshot();
            self.canvas_diff_task =
                Some(diff.update(cx, |diff, cx| diff.set_base_text(None, buffer_snapshot, cx)));
        }
        self.canvas_diff_visible = false;
        cx.notify();
    }

    /// Persist the dirty FNX buffer through the format layer's validated,
    /// atomic source-edit path. The returned document is installed only after
    /// that write succeeds, and the ordinary project-buffer save then records
    /// the exact saved buffer version so Zed's dirty state and the canvas lock
    /// converge with disk.
    pub(crate) fn save_source_edit(&mut self, cx: &mut Context<Self>) -> Option<Task<Result<()>>> {
        self.save_source_edit_impl(true, cx)
    }

    /// Persist the dirty FNX buffer without the pre-save format pass — the
    /// auto-persist path for edits that validated cleanly (an agent rewriting
    /// the file, or live typing). Skipping the reformat keeps the author's
    /// cursor and text untouched; the manual save still formats.
    fn persist_validated_source_edit(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<Task<Result<()>>> {
        self.save_source_edit_impl(false, cx)
    }

    fn save_source_edit_impl(
        &mut self,
        format: bool,
        cx: &mut Context<Self>,
    ) -> Option<Task<Result<()>>> {
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
        if !self
            .item
            .update(cx, |item, _| item.try_begin_source_edit_pipeline())
        {
            return Some(Task::ready(Err(anyhow!(
                "another FNX save or reconciliation is already in progress"
            ))));
        }

        self.validation_task = None;
        self.error_message = None;
        self.validation_message = Some("Saving FNX…".into());
        self.source_save_in_progress = true;
        let item = self.item.clone();
        let project = self.project.clone();
        let format_source = (format && !cfg!(test)).then(|| {
            project.update(cx, |project, cx| {
                project.format(
                    std::iter::once(buffer.clone()).collect(),
                    LspFormatTarget::Buffers,
                    false,
                    FormatTrigger::Manual,
                    cx,
                )
            })
        });

        Some(cx.spawn(async move |this, cx| {
            if let Some(format_source) = format_source
                && let Err(error) = format_source.await
            {
                item.update(cx, |item, _| item.finish_source_edit_pipeline());
                this.update(cx, |this, cx| {
                    this.source_save_in_progress = false;
                    this.validation_message = None;
                    this.error_message =
                        Some(format!("Could not format FNX before saving: {error:#}").into());
                    cx.notify();
                })?;
                return Err(error).context("formatting the FNX source edit");
            }
            let (source, version) =
                buffer.read_with(cx, |buffer, _| (buffer.text(), buffer.version()));
            let apply_path = source_path.clone();
            let source_edit = cx
                .background_spawn(async move {
                    fanta_format::apply_project_source_edit_with_diagnostics(
                        &project_root,
                        &apply_path,
                        &source,
                    )
                })
                .await;
            let (source_edit, source_diagnostics) = match source_edit {
                Ok(source_edit) => source_edit,
                Err(error) => {
                    item.update(cx, |item, _| item.finish_source_edit_pipeline());
                    this.update(cx, |this, cx| {
                        this.source_save_in_progress = false;
                        this.validation_message = None;
                        this.error_message = Some(format!("Could not save FNX: {error}").into());
                        cx.notify();
                    })?;
                    return Err(error).context("applying the FNX source edit");
                }
            };
            // Authoring warnings (typo'd/unknown attributes with a
            // did-you-mean) never block the save — surface them where the
            // author is looking instead of letting a typo pass silently.
            if let Some(summary) = summarize_source_diagnostics(&source_diagnostics) {
                this.update(cx, |this, cx| {
                    this.validation_message = Some(summary.into());
                    cx.notify();
                })?;
            }

            let buffer_unchanged = buffer.read_with(cx, |buffer, _| buffer.version() == version);
            if !buffer_unchanged {
                // The validated snapshot is now safely on disk, but a newer
                // edit owns the live preview and must remain dirty/locked.
                item.update(cx, |item, _| item.finish_source_edit_pipeline());
                this.update(cx, |this, cx| {
                    this.source_save_in_progress = false;
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
                item.update(cx, |item, _| item.finish_source_edit_pipeline());
                this.update(cx, |this, cx| {
                    this.source_save_in_progress = false;
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
            let reload_json = match this.update(cx, |this, cx| this.reload_json_buffer(cx)) {
                Ok(reload_json) => reload_json,
                Err(error) => {
                    item.update(cx, |item, _| item.finish_source_edit_pipeline());
                    return Err(error).context("refreshing JSON after saving FNX");
                }
            };
            if let Some(reload_json) = reload_json
                && let Err(error) = reload_json.await
            {
                item.update(cx, |item, _| item.finish_source_edit_pipeline());
                this.update(cx, |this, cx| {
                    this.source_save_in_progress = false;
                    this.validation_message = None;
                    this.error_message = Some(
                        format!("FNX was saved, but JSON could not refresh: {error:#}").into(),
                    );
                    cx.notify();
                })?;
                return Ok(());
            }
            item.update(cx, |item, _| item.finish_source_edit_pipeline());
            this.update(cx, |this, cx| {
                this.source_save_in_progress = false;
                this.validation_message = None;
                this.error_message = None;
                cx.notify();
            })?;
            Ok(())
        }))
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
                self.fnx_buffer = None;
                self.json_buffer = None;
                self.fnx_editor = None;
                self.json_editor = None;
                self.fnx_subscription = None;
                self.validation_task = None;
                self.canvas_diff = None;
                self.canvas_diff_task = None;
                self.previous_fnx_diff = None;
                self.canvas_diff_visible = false;
                self.source_save_in_progress = false;
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
        self.canvas_diff = None;
        self.canvas_diff_task = None;
        self.previous_fnx_diff = None;
        self.canvas_diff_visible = false;
        self.source_save_in_progress = false;
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
        self.canvas_diff = None;
        self.canvas_diff_task = None;
        self.previous_fnx_diff = None;
        self.canvas_diff_visible = false;
        self.loading_fnx = true;
        self.error_message = None;
        let open_task = self
            .project
            .update(cx, |project, cx| project.open_local_buffer(&path, cx));
        let project = self.project.clone();
        self.fnx_load_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = match open_task.await {
                Ok(buffer) if cfg!(test) => Ok(buffer),
                Ok(buffer) => {
                    let format = project.update(cx, |project, cx| {
                        project.format(
                            std::iter::once(buffer.clone()).collect(),
                            LspFormatTarget::Buffers,
                            false,
                            FormatTrigger::Manual,
                            cx,
                        )
                    });
                    match format.await {
                        Ok(_) if buffer.read_with(cx, |buffer, _| buffer.is_dirty()) => {
                            match project
                                .update(cx, |project, cx| project.save_buffer(buffer.clone(), cx))
                                .await
                            {
                                Ok(_) => Ok(buffer),
                                Err(error) => Err(error.context("saving formatted FNX source")),
                            }
                        }
                        Ok(_) => Ok(buffer),
                        Err(error) => Err(error.context("formatting FNX source")),
                    }
                }
                Err(error) => Err(error),
            };
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
        let migration_error = crate::fnx_editor::upgrade_legacy_fnx_buffer(&buffer, cx).err();
        if let Some(error) = &migration_error {
            self.error_message =
                Some(format!("Could not prepare the legacy FNX source: {error:#}").into());
        }
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
                this.handle_fnx_buffer_event(buffer.clone(), event, window, cx);
            },
        ));
        self.fnx_buffer = Some(buffer);
        self.fnx_editor = Some(editor);
        if source_is_dirty {
            self.schedule_source_validation(cx);
        } else if migration_error.is_none() {
            self.error_message = None;
        }
    }

    fn handle_fnx_buffer_event(
        &mut self,
        buffer: Entity<Buffer>,
        event: &BufferEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            BufferEvent::Edited { .. } => {
                let source_is_dirty = buffer.read(cx).is_dirty();
                self.item.update(cx, |item, cx| {
                    item.set_source_edit_locked(source_is_dirty, cx)
                });
                if self.canvas_diff_visible {
                    self.show_canvas_diff(cx);
                }
                self.schedule_source_validation(cx);
            }
            BufferEvent::Reloaded => {
                let source_is_dirty = buffer.read(cx).is_dirty();
                self.item.update(cx, |item, cx| {
                    item.set_source_edit_locked(source_is_dirty, cx)
                });
                if source_is_dirty {
                    if self.canvas_diff_visible {
                        self.show_canvas_diff(cx);
                    }
                    self.schedule_source_validation(cx);
                } else {
                    self.hide_canvas_diff(cx);
                    self.validation_task = None;
                    self.validation_message = None;
                    self.error_message = None;
                    cx.notify();
                }
            }
            BufferEvent::Saved => {
                if self.source_save_in_progress {
                    let source_is_dirty = buffer.read(cx).is_dirty();
                    self.item.update(cx, |item, cx| {
                        item.set_source_edit_locked(source_is_dirty, cx)
                    });
                } else {
                    self.validation_task = None;
                    let owns_reconciliation = self
                        .item
                        .update(cx, |item, _| item.try_begin_source_edit_pipeline());
                    if owns_reconciliation {
                        self.item
                            .update(cx, |item, cx| item.set_source_edit_locked(true, cx));
                        self.reconcile_saved_source(cx);
                    }
                }
                let active_page = self.item.read(cx).doc().and_then(|doc| doc.active_page());
                if active_page != self.requested_page {
                    self.refresh_page(active_page, window, cx);
                }
            }
            _ => {}
        }
    }

    fn reconcile_saved_source(&mut self, cx: &mut Context<Self>) {
        self.validation_task = None;
        if self.item.read(cx).is_dirty() {
            self.item
                .update(cx, |item, _| item.finish_source_edit_pipeline());
            self.validation_message = None;
            self.error_message = Some(
                "FNX was saved outside the canvas while canvas changes were unsaved. Reload or save the canvas before reconciling the source."
                    .into(),
            );
            cx.notify();
            return;
        }
        let Some(project_root) = self.item.read(cx).project_root().map(Path::to_path_buf) else {
            self.item
                .update(cx, |item, _| item.finish_source_edit_pipeline());
            self.validation_message = None;
            self.error_message = Some("The FNX project root is unavailable.".into());
            cx.notify();
            return;
        };
        let Some(source_path) = self.fnx_path.clone() else {
            self.item
                .update(cx, |item, _| item.finish_source_edit_pipeline());
            self.validation_message = None;
            self.error_message = Some("The saved FNX source path is unavailable.".into());
            cx.notify();
            return;
        };
        let Some(buffer) = self.fnx_buffer.clone() else {
            self.item
                .update(cx, |item, _| item.finish_source_edit_pipeline());
            self.validation_message = None;
            self.error_message = Some("The saved FNX buffer is unavailable.".into());
            cx.notify();
            return;
        };
        let (source, version) = {
            let buffer = buffer.read(cx);
            (buffer.text(), buffer.version())
        };

        self.error_message = None;
        self.validation_message = Some("Reconciling saved FNX…".into());
        let item = self.item.clone();
        cx.spawn(async move |this, cx| {
            let apply_path = source_path.clone();
            let reconciliation = cx
                .background_spawn(async move {
                    fanta_format::apply_project_source_edit_with_diagnostics(
                        &project_root,
                        &apply_path,
                        &source,
                    )
                })
                .await;

            let (source_edit, source_diagnostics) = match reconciliation {
                Ok(source_edit) => source_edit,
                Err(error) => {
                    item.update(cx, |item, _| item.finish_source_edit_pipeline());
                    if let Err(update_error) = this.update(cx, |this, cx| {
                        this.validation_message = None;
                        this.error_message = Some(
                            format!(
                                "Saved FNX could not be reconciled; the canvas is keeping its last valid state: {error}"
                            )
                            .into(),
                        );
                        cx.notify();
                    }) {
                        log::debug!(
                            "dropping saved FNX reconciliation for closed workspace: {update_error:#}"
                        );
                    }
                    return;
                }
            };
            // Non-blocking authoring warnings (unknown attributes with a
            // did-you-mean) — surfaced, never a reason to reject the save.
            if let Some(summary) = summarize_source_diagnostics(&source_diagnostics)
                && let Err(update_error) = this.update(cx, |this, cx| {
                    this.validation_message = Some(summary.into());
                    cx.notify();
                })
            {
                log::debug!(
                    "dropping saved FNX diagnostics for closed workspace: {update_error:#}"
                );
            }

            let source_is_current = match this.read_with(cx, |this, cx| {
                this.fnx_path.as_ref() == Some(&source_path)
                    && buffer.read(cx).version() == version
                    && !buffer.read(cx).is_dirty()
            }) {
                Ok(source_is_current) => source_is_current,
                Err(error) => {
                    item.update(cx, |item, _| item.finish_source_edit_pipeline());
                    log::debug!(
                        "dropping saved FNX reconciliation for closed workspace: {error:#}"
                    );
                    return;
                }
            };
            if !source_is_current {
                item.update(cx, |item, _| item.finish_source_edit_pipeline());
                if let Err(error) = this.update(cx, |this, cx| {
                    if this.validation_message.as_deref() == Some("Reconciling saved FNX…") {
                        this.validation_message = None;
                    }
                    cx.notify();
                }) {
                    log::debug!(
                        "dropping saved FNX reconciliation for closed workspace: {error:#}"
                    );
                }
                return;
            }

            item.update(cx, |item, cx| {
                item.adopt_saved_source_edit(source_edit, cx);
                item.set_source_edit_locked(false, cx);
                item.finish_source_edit_pipeline();
            });
            let reload_json = match this.update(cx, |this, cx| {
                this.validation_message = None;
                this.error_message = None;
                let reload_json = this.reload_json_buffer(cx);
                cx.notify();
                reload_json
            }) {
                Ok(reload_json) => reload_json,
                Err(error) => {
                    log::debug!(
                        "dropping saved FNX reconciliation for closed workspace: {error:#}"
                    );
                    return;
                }
            };

            if let Some(reload_json) = reload_json
                && let Err(error) = reload_json.await
                && let Err(update_error) = this.update(cx, |this, cx| {
                    this.error_message = Some(
                        format!("FNX was reconciled, but JSON could not refresh: {error:#}").into(),
                    );
                    cx.notify();
                })
            {
                log::debug!(
                    "dropping JSON refresh error for closed workspace: {update_error:#}"
                );
            }
        })
        .detach();
        cx.notify();
    }

    fn reload_json_buffer(&mut self, cx: &mut Context<Self>) -> Option<Task<Result<()>>> {
        let buffer = self.json_buffer.clone()?;
        if buffer.read(cx).is_dirty() {
            return Some(Task::ready(Err(anyhow!(
                "the JSON buffer has unsaved changes"
            ))));
        }
        let reload = self.project.update(cx, |project, cx| {
            project.reload_buffers(std::iter::once(buffer).collect(), false, cx)
        });
        Some(cx.spawn(async move |_, _| {
            reload
                .await
                .context("reloading the generated JSON buffer")?;
            Ok(())
        }))
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
        let (source, version) = {
            let buffer = buffer.read(cx);
            (buffer.text(), buffer.version())
        };
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
                        if buffer.read(cx).version() != version {
                            return;
                        }
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
                        let workspace = cx.weak_entity();
                        let expected_version = version.clone();
                        cx.defer(move |cx| {
                            if buffer.read(cx).version() != expected_version {
                                return;
                            }
                            let source_is_dirty = buffer.read(cx).is_dirty();
                            item.update(cx, |item, cx| {
                                item.adopt_source_edit(source_edit, cx);
                                item.set_source_edit_locked(source_is_dirty, cx);
                            });
                            // A cleanly validated source edit persists right
                            // away: leaving the buffer dirty kept the canvas
                            // LOCKED after an agent rewrote the file — the
                            // user couldn't edit, and the save flow fought
                            // them. Persisting closes the code→canvas loop
                            // (canvas unlocks via the buffer save) and the
                            // lock now only survives for INVALID source.
                            if source_is_dirty
                                && let Err(error) = workspace.update(cx, |workspace, cx| {
                                    if let Some(save) = workspace.persist_validated_source_edit(cx)
                                    {
                                        save.detach_and_log_err(cx);
                                    }
                                })
                            {
                                log::debug!(
                                    "dropping FNX auto-persist for closed workspace: {error:#}"
                                );
                            }
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

fn is_exclusively_local_canvas_source_conflict(
    has_project_conflict: bool,
    canvas_is_dirty: bool,
    source_is_dirty: bool,
) -> bool {
    !has_project_conflict && canvas_is_dirty && source_is_dirty
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
        let has_conflict = self.has_source_conflict(cx);
        let has_local_canvas_source_conflict = self.has_local_canvas_source_conflict(cx);
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
                            div().flex_1().min_w_0().overflow_hidden().child(
                                Label::new(message)
                                    .size(LabelSize::Small)
                                    .color(color)
                                    .single_line(),
                            ),
                        )
                    }),
            )
            .when(has_conflict, |this| {
                let conflict_message = if has_local_canvas_source_conflict {
                    "Canvas and FNX source changed independently. Compare them before choosing Overwrite or Discard."
                } else {
                    "External file edits conflict with unsaved canvas edits on the same values and could not be auto-merged. Choose Overwrite to keep the canvas edits or Discard to reload the project."
                };
                this.child(
                    h_flex()
                        .flex_none()
                        .min_h(px(38.0))
                        .px_3()
                        .gap_2()
                        .border_b_1()
                        .border_color(Color::Warning.color(cx))
                        .bg(cx.theme().status().warning_background)
                        .child(
                            div().flex_1().min_w_0().child(
                                Label::new(conflict_message).size(LabelSize::Small),
                            ),
                        )
                        .when(has_local_canvas_source_conflict, |bar| {
                            bar.child(
                                Button::new("fanta-code-compare-canvas", "Compare with canvas")
                                    .size(ButtonSize::Compact)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_canvas_diff(cx)
                                    })),
                            )
                        }),
                )
            })
            .child(div().flex_1().min_h_0().child(self.render_body(cx)))
    }
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
            crate::fnx_editor::init(cx);
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

    /// Drive the executor until the auto-persist that follows a valid FNX
    /// validation has landed (its save path crosses real-fs operations that
    /// a single `run_until_parked` can outpace).
    fn wait_for_source_persist(item: &Entity<FigItem>, cx: &mut TestAppContext) {
        for _ in 0..200 {
            let unlocked = item.read_with(cx, |item, _| !item.source_edit_locked());
            if unlocked {
                return;
            }
            cx.executor().advance_clock(Duration::from_millis(50));
            cx.run_until_parked();
        }
        panic!("FNX auto-persist did not complete");
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

    /// Resolve a page's source path in a REAL on-disk project — the slug
    /// layout means paths can't be derived from ids.
    fn page_source_path(project_root: &Path, page: NodeId) -> PathBuf {
        fanta_format::locate_page_source(project_root, page).expect("page source on disk")
    }

    fn page_json_path(project_root: &Path, page: NodeId) -> PathBuf {
        page_source_path(project_root, page).with_file_name("page.json")
    }

    #[test]
    fn active_page_compare_is_hidden_when_an_external_project_conflict_coexists() {
        assert!(is_exclusively_local_canvas_source_conflict(
            false, true, true
        ));
        assert!(!is_exclusively_local_canvas_source_conflict(
            true, true, true
        ));
        assert!(!is_exclusively_local_canvas_source_conflict(
            false, true, false
        ));
    }

    #[gpui::test]
    async fn clean_fnx_reload_does_not_transiently_lock_the_canvas(cx: &mut TestAppContext) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        write_project(temporary.path());
        let (_project, item, workspace) = open_code_workspace(temporary.path(), cx).await;
        let buffer = workspace
            .read_with(cx, |workspace, _| {
                workspace.fnx_buffer.clone().expect("FNX buffer")
            })
            .expect("read code workspace");
        buffer.read_with(cx, |buffer, _| assert!(!buffer.is_dirty()));
        item.update(cx, |item, cx| item.set_source_edit_locked(true, cx));

        workspace
            .update(cx, |workspace, window, cx| {
                workspace.handle_fnx_buffer_event(buffer, &BufferEvent::Reloaded, window, cx);
            })
            .expect("handle clean reload");

        item.read_with(cx, |item, _| assert!(!item.source_edit_locked()));
        workspace
            .read_with(cx, |workspace, _| {
                assert!(workspace.validation_task.is_none());
                assert!(workspace.validation_message.is_none());
            })
            .expect("read settled workspace");
    }

    #[gpui::test]
    async fn canvas_diff_requires_local_divergence_and_uses_current_canvas_fnx_as_its_base(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        let page = write_project(temporary.path());
        let source = std::fs::read_to_string(page_source_path(temporary.path(), page))
            .expect("read generated FNX source");
        let (_project, item, workspace) = open_code_workspace(temporary.path(), cx).await;
        item.update(cx, |item, cx| {
            item.apply(
                Operation::SetName {
                    id: page,
                    old: "Original".into(),
                    new: "Canvas edit".into(),
                },
                cx,
            )
        })
        .expect("edit canvas");

        workspace
            .update(cx, |workspace, _window, cx| workspace.show_canvas_diff(cx))
            .expect("ignore canvas-only comparison");
        workspace
            .read_with(cx, |workspace, _| assert!(!workspace.canvas_diff_visible))
            .expect("read hidden canvas diff");

        replace_workspace_source(
            workspace,
            source.replace("name=\"Original\"", "name=\"FNX edit\""),
            cx,
        );

        workspace
            .update(cx, |workspace, _window, cx| workspace.show_canvas_diff(cx))
            .expect("show canvas diff");
        cx.run_until_parked();
        let diff = workspace
            .read_with(cx, |workspace, _| {
                assert!(workspace.canvas_diff_visible);
                workspace.canvas_diff.clone().expect("canvas diff")
            })
            .expect("read code workspace");
        diff.read_with(cx, |diff, cx| {
            let base = diff.base_text_string(cx).expect("canvas FNX base text");
            assert!(base.contains("name=\"Canvas edit\""));
            assert!(base.contains("@jsxRuntime classic"));
        });

        workspace
            .update(cx, |workspace, _window, cx| workspace.hide_canvas_diff(cx))
            .expect("hide canvas diff");
        cx.run_until_parked();
        workspace
            .read_with(cx, |workspace, _| assert!(!workspace.canvas_diff_visible))
            .expect("read hidden canvas diff");
    }

    #[gpui::test]
    async fn discarding_fnx_then_reloading_restores_disk_without_a_lock_error(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        let page = write_project(temporary.path());
        let source_path = page_source_path(temporary.path(), page);
        let original_source =
            std::fs::read_to_string(&source_path).expect("read generated FNX source");
        let (_project, item, workspace) = open_code_workspace(temporary.path(), cx).await;
        // A VALID edit would auto-persist and leave nothing to discard, so
        // the discard path is exercised by what still locks: invalid source.
        replace_workspace_source(workspace, "<Frame".to_owned(), cx);
        item.read_with(cx, |item, _| assert!(item.source_edit_locked()));

        let discard = workspace
            .update(cx, |workspace, _window, cx| {
                workspace.discard_source_edit(cx)
            })
            .expect("start source discard");
        discard.await.expect("discard source edit");
        let reload = item.update(cx, |item, cx| item.reload_from_disk(cx));
        reload.await.expect("reload canvas from disk");
        cx.run_until_parked();

        assert_eq!(page_name(&item, page, cx), "Original");
        item.read_with(cx, |item, _| {
            assert!(!item.source_edit_locked());
            assert!(!item.has_conflict());
        });
        workspace
            .read_with(cx, |workspace, cx| {
                assert!(!workspace.source_is_dirty(cx));
                assert!(workspace.validation_error().is_none());
            })
            .expect("read reconciled workspace");
        assert_eq!(
            std::fs::read_to_string(source_path).expect("source remains unchanged"),
            original_source
        );
    }

    #[gpui::test]
    async fn simultaneous_canvas_and_fnx_edits_are_reported_as_a_conflict(cx: &mut TestAppContext) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        let page = write_project(temporary.path());
        let source_path = page_source_path(temporary.path(), page);
        let original_source =
            std::fs::read_to_string(&source_path).expect("read generated FNX source");
        let (_project, item, workspace) = open_code_workspace(temporary.path(), cx).await;

        item.update(cx, |item, cx| {
            item.apply(
                Operation::SetName {
                    id: page,
                    old: "Original".into(),
                    new: "Canvas edit".into(),
                },
                cx,
            )
        })
        .expect("canvas edit");
        replace_workspace_source(
            workspace,
            original_source.replace("name=\"Original\"", "name=\"FNX edit\""),
            cx,
        );

        workspace
            .read_with(cx, |workspace, cx| {
                assert!(workspace.source_is_dirty(cx));
                assert!(workspace.has_source_conflict(cx));
            })
            .expect("read source conflict");
        assert_eq!(page_name(&item, page, cx), "Canvas edit");
    }

    #[gpui::test]
    async fn ordinary_editor_migrates_shared_fnx_and_reconciles_its_save(cx: &mut TestAppContext) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        let page = write_project(temporary.path());
        let source_path = page_source_path(temporary.path(), page);
        let json_path = page_json_path(temporary.path(), page);
        let generated_source =
            std::fs::read_to_string(&source_path).expect("read generated FNX source");
        let legacy_source = remove_editor_prelude(&generated_source);
        assert_ne!(legacy_source, generated_source);
        std::fs::write(&source_path, &legacy_source).expect("write legacy FNX source");
        let editor_types_path = temporary.path().join("fnx.d.ts");
        if editor_types_path.exists() {
            std::fs::remove_file(&editor_types_path).expect("remove editor types");
        }
        let formatter_settings_path = temporary.path().join(".prettierrc.json");
        if formatter_settings_path.exists() {
            std::fs::remove_file(&formatter_settings_path).expect("remove formatter settings");
        }

        let project = open_test_project(temporary.path(), cx).await;
        let open_buffer = project.update(cx, |project, cx| {
            project.open_local_buffer(&source_path, cx)
        });
        let buffer = open_buffer.await.expect("open ordinary FNX buffer");
        let ordinary_project = project.clone();
        let ordinary_buffer = buffer.clone();
        let ordinary_editor = cx.add_window(move |window, cx| {
            Editor::for_buffer(ordinary_buffer, Some(ordinary_project), window, cx)
        });
        cx.run_until_parked();

        let canonical_source = buffer.read_with(cx, |buffer, _| {
            assert!(buffer.is_dirty());
            buffer.text()
        });
        assert!(canonical_source.contains("@jsxRuntime classic"));
        assert!(canonical_source.contains("from \"../../fnx\";"));
        assert_eq!(
            std::fs::read_to_string(&source_path).expect("source stays implicit-write free"),
            legacy_source
        );
        assert!(temporary.path().join("fnx.d.ts").is_file());
        assert!(temporary.path().join(".prettierrc.json").is_file());

        let (item, workspace) = open_code_workspace_for_project(project.clone(), cx).await;
        let shared_buffer = workspace
            .read_with(cx, |workspace, _| {
                workspace.fnx_buffer.clone().expect("workspace FNX buffer")
            })
            .expect("read code workspace");
        assert_eq!(shared_buffer.entity_id(), buffer.entity_id());
        cx.executor()
            .advance_clock(SOURCE_VALIDATION_DEBOUNCE + Duration::from_millis(1));
        cx.run_until_parked();
        assert_eq!(page_name(&item, page, cx), "Original");
        item.read_with(cx, |item, _| assert!(item.source_edit_locked()));

        let changed_source = canonical_source.replace("name=\"Original\"", "name=\"External\"");
        assert_ne!(changed_source, canonical_source);
        ordinary_editor
            .update(cx, |editor, window, cx| {
                editor.set_text(changed_source.clone(), window, cx)
            })
            .expect("edit ordinary FNX editor");
        cx.executor()
            .advance_clock(SOURCE_VALIDATION_DEBOUNCE + Duration::from_millis(1));
        cx.run_until_parked();
        assert_eq!(page_name(&item, page, cx), "External");

        let json_before = workspace
            .read_with(cx, |workspace, cx| {
                workspace
                    .json_buffer
                    .as_ref()
                    .expect("JSON buffer")
                    .read(cx)
                    .text()
            })
            .expect("read JSON buffer");
        assert!(json_before.contains("Original"));
        std::fs::write(
            &json_path,
            "{\n  \"name\": \"Disk JSON Refresh\",\n  \"order\": 0\n}\n",
        )
        .expect("change JSON on disk");

        project
            .update(cx, |project, cx| project.save_buffer(buffer.clone(), cx))
            .await
            .expect("ordinary editor save");
        cx.run_until_parked();

        assert_eq!(
            std::fs::read_to_string(&source_path).expect("read saved FNX source"),
            changed_source
        );
        assert_eq!(page_name(&item, page, cx), "External");
        item.read_with(cx, |item, _| {
            assert!(!item.source_edit_locked());
            assert!(item.is_editable());
        });
        buffer.read_with(cx, |buffer, _| assert!(!buffer.is_dirty()));
        workspace
            .read_with(cx, |workspace, cx| {
                assert!(workspace.validation_error().is_none());
                assert!(
                    workspace
                        .json_buffer
                        .as_ref()
                        .expect("JSON buffer")
                        .read(cx)
                        .text()
                        .contains("Disk JSON Refresh")
                );
            })
            .expect("read reconciled workspace");
    }

    #[gpui::test]
    async fn ordinary_editor_invalid_save_keeps_canvas_locked_with_an_error(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        let page = write_project(temporary.path());
        let source_path = page_source_path(temporary.path(), page);
        let (project, item, workspace) = open_code_workspace(temporary.path(), cx).await;
        let buffer = workspace
            .read_with(cx, |workspace, _| {
                workspace.fnx_buffer.clone().expect("FNX buffer")
            })
            .expect("read code workspace");

        workspace
            .update(cx, |workspace, window, cx| {
                workspace
                    .fnx_editor
                    .as_ref()
                    .expect("FNX editor")
                    .update(cx, |editor, cx| editor.set_text("<Frame>", window, cx));
            })
            .expect("edit invalid FNX");
        project
            .update(cx, |project, cx| project.save_buffer(buffer.clone(), cx))
            .await
            .expect("ordinary editor writes its buffer");
        cx.run_until_parked();

        assert_eq!(
            std::fs::read_to_string(&source_path).expect("read invalid saved source"),
            "<Frame>"
        );
        assert_eq!(page_name(&item, page, cx), "Original");
        item.read_with(cx, |item, _| {
            assert!(item.source_edit_locked());
            assert!(!item.is_editable());
        });
        workspace
            .read_with(cx, |workspace, _| {
                assert!(workspace.validation_error().is_some_and(|message| {
                    message.starts_with("Saved FNX could not be reconciled")
                }));
            })
            .expect("read invalid-save error");
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
        replace_workspace_source(workspace, changed_source.clone(), cx);
        assert_eq!(page_name(&item, page, cx), "Changed");
        // A valid edit auto-persists: the canvas must NOT stay locked (an
        // agent rewriting the file used to freeze the editor), and disk holds
        // the edit without a manual save.
        wait_for_source_persist(&item, cx);
        item.read_with(cx, |item, _| {
            assert!(!item.source_edit_locked());
            assert!(item.is_editable());
        });
        assert_eq!(
            std::fs::read_to_string(&source_path).expect("auto-persisted source"),
            changed_source
        );
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.selection.select_only(page);
                ((), crate::document::DocChange::Selection)
            });
        });
        item.read_with(cx, |item, _| {
            assert_eq!(item.doc().expect("document").selection.as_slice(), &[page]);
        });
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
        wait_for_source_persist(&item, cx);
        // Validation already persisted the edit; a manual save has nothing
        // left to do.
        workspace
            .update(cx, |workspace, _window, cx| {
                assert!(workspace.save_source_edit(cx).is_none());
            })
            .expect("update code workspace");
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

/// One status-line summary of the engine's authoring warnings (unknown
/// attributes with did-you-mean suggestions). `None` when there are none, so
/// callers can skip the update entirely.
fn summarize_source_diagnostics(diagnostics: &[fanta_format::SourceDiagnostic]) -> Option<String> {
    if diagnostics.is_empty() {
        return None;
    }
    let shown: Vec<&str> = diagnostics
        .iter()
        .take(2)
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();
    let extra = diagnostics.len().saturating_sub(shown.len());
    let mut summary = format!(
        "Saved with {} warning{}: {}",
        diagnostics.len(),
        if diagnostics.len() == 1 { "" } else { "s" },
        shown.join(" · ")
    );
    if extra > 0 {
        summary.push_str(&format!(" (+{extra} more)"));
    }
    Some(summary)
}
