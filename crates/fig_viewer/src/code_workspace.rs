use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use collections::HashSet;
use editor::{Editor, MultiBufferOffset, SelectionEffects, scroll::Autoscroll};
use fanta_doc::NodeId;
use gpui::{
    Animation, AnimationExt as _, AnyElement, App, ClipboardItem, Context, Entity, FocusHandle,
    Focusable, IntoElement, Render, SharedString, Subscription, Task, Window, div,
    pulsating_between, px,
};
use language::{Buffer, Capability};
use project::Project;
use search::{
    BufferSearchBar,
    buffer_search::{Deploy, DivRegistrar},
};
use serde::Deserialize;
use ui::Tooltip;
use ui::prelude::*;

use crate::document::{FigItem, FigItemEvent};
use workspace::{FollowableItem as _, ItemHandle, ToolbarItemView as _};

const EDITABLE_STATUS: &str = "Edit source · save to validate and update the canvas";

/// Every JSX tag the FNX printer can emit: the canonical node tags plus the
/// two authoring-sugar shape tags. Restricting the opening-tag scan to these
/// is what stops an attribute value like `name="a <Button> b"` — arbitrary
/// user text — from being counted as an element.
const FNX_TAGS: &[&str] = &[
    "AiArtifact",
    "Audio",
    "Boolean",
    "Ellipse",
    "Embed",
    "Frame",
    "Image",
    "Instance",
    "Model3D",
    "NodeGraph",
    "Rect",
    "Text",
    "Vector",
    "Video",
];

/// The `.ids.json` sidecar written next to every `.fnx` source. Declared here
/// as a private shape, read with `serde_json`, rather than depending on
/// `fanta-fnx` for one field.
#[derive(Deserialize)]
struct SourceIdSidecar {
    /// One entry per element, in the same pre-order the source is printed in:
    /// entry 0 is the subtree root, which is opening tag #1.
    ids: Vec<SourceIdEntry>,
}

#[derive(Deserialize)]
struct SourceIdEntry {
    id: String,
}

/// One opening tag found in an `.fnx` source: where it starts, and the value
/// of its `name` attribute when it has one (the fallback mapping key).
struct OpeningTag {
    offset: usize,
    name: Option<String>,
}

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
    search_bar: Entity<BufferSearchBar>,
    search_editor: Option<Entity<Editor>>,
    selected_file: CodeWorkspaceFile,
    requested_page: Option<NodeId>,
    /// The root whose source the panes are currently showing — a page root or
    /// a component master root. Recorded so the selection sync can rank a
    /// node within exactly the subtree the FNX file contains.
    source_root: Option<NodeId>,
    /// The node the FNX pane was last scrolled to. `SelectionChanged` is
    /// emitted for hover and tool state as well as for real selection changes,
    /// and this workspace outlives every tab switch, so the sync would
    /// otherwise rescan the buffer while the Canvas tab is showing.
    last_synced_selection: Option<NodeId>,
    fnx_path: Option<PathBuf>,
    json_path: Option<PathBuf>,
    fnx_editor: Option<Entity<Editor>>,
    json_editor: Option<Entity<Editor>>,
    fnx_source_buffer: Option<Entity<Buffer>>,
    json_source_buffer: Option<Entity<Buffer>>,
    fnx_source_observation: Option<Subscription>,
    json_source_observation: Option<Subscription>,
    fnx_source_version: Option<clock::Global>,
    json_source_version: Option<clock::Global>,
    source_save_in_progress: bool,
    source_discard_in_progress: bool,
    development_read_only: bool,
    source_writer: Option<design_surface::AgentActivity>,
    follow_source_writer: bool,
    loading_fnx: bool,
    loading_json: bool,
    error_message: Option<SharedString>,
    fnx_load_task: Option<Task<()>>,
    json_load_task: Option<Task<()>>,
    _item_subscription: Subscription,
    _project_subscription: Subscription,
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
                    if matches!(event, FigItemEvent::SelectionChanged) {
                        this.sync_selection_to_source(window, cx);
                    }
                } else if matches!(event, FigItemEvent::Saved) {
                    // Renaming a page/master moves its source directory;
                    // first materialization creates paths the pane did not
                    // have. Preserve the editor when paths stay the same.
                    if this.saved_source_paths_changed(cx) {
                        this.refresh_from_item(window, cx);
                    } else if !this.source_save_in_progress && !this.source_discard_in_progress {
                        this.reload_saved_sources(window, cx);
                    }
                    this.update_fnx_editability(cx);
                    this.last_synced_selection = None;
                    this.sync_selection_to_source(window, cx);
                }
                cx.notify();
            },
        );
        let project_subscription = cx.subscribe_in(
            &project,
            window,
            |this: &mut Self, _, event: &project::Event, window, cx| {
                if matches!(event, project::Event::AgentLocationChanged) {
                    this.follow_source_location(window, cx);
                }
            },
        );
        let requested_page = item.read(cx).doc().and_then(|doc| doc.active_page());
        let mut workspace = Self {
            item,
            project,
            focus_handle: cx.focus_handle(),
            search_bar: cx.new(|cx| BufferSearchBar::new(None, window, cx)),
            search_editor: None,
            selected_file: CodeWorkspaceFile::Fnx,
            requested_page,
            source_root: None,
            last_synced_selection: None,
            fnx_path: None,
            json_path: None,
            fnx_editor: None,
            json_editor: None,
            fnx_source_buffer: None,
            json_source_buffer: None,
            fnx_source_observation: None,
            json_source_observation: None,
            fnx_source_version: None,
            json_source_version: None,
            source_save_in_progress: false,
            source_discard_in_progress: false,
            development_read_only: false,
            source_writer: None,
            follow_source_writer: false,
            loading_fnx: false,
            loading_json: false,
            error_message: None,
            fnx_load_task: None,
            json_load_task: None,
            _item_subscription: item_subscription,
            _project_subscription: project_subscription,
        };
        workspace.refresh_from_item(window, cx);
        workspace
    }

    pub(crate) fn set_source_writer(
        &mut self,
        writer: Option<design_surface::AgentActivity>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let identity = |writer: &design_surface::AgentActivity| {
            (writer.agent_id.clone(), writer.source_path.clone())
        };
        if self.source_writer.as_ref().map(identity) != writer.as_ref().map(identity) {
            self.follow_source_writer = writer.is_some();
            if writer.is_some() {
                self.selected_file = CodeWorkspaceFile::Fnx;
            }
        }
        self.source_writer = writer;
        self.follow_source_location(window, cx);
        cx.notify();
    }

    fn source_writer_matches_file(&self) -> bool {
        self.source_writer
            .as_ref()
            .and_then(|writer| writer.source_path.as_deref())
            .is_some_and(|path| self.fnx_path.as_deref() == Some(Path::new(path)))
    }

    fn follow_source_location(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.follow_source_writer || !self.source_writer_matches_file() {
            return;
        }
        let Some(location) = self.project.read(cx).agent_location() else {
            return;
        };
        if !self
            .fnx_source_buffer
            .as_ref()
            .is_some_and(|buffer| buffer.entity_id() == location.buffer.entity_id())
        {
            return;
        }
        if let Some(editor) = &self.fnx_editor {
            editor.update(cx, |editor, cx| {
                editor.update_agent_location(location.position, window, cx);
            });
        }
    }

    pub fn refresh_page(
        &mut self,
        page: Option<NodeId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if page != self.requested_page && self.source_is_dirty(cx) {
            self.error_message =
                Some("Save or discard the source edit before switching pages.".into());
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

    pub(crate) fn set_development_read_only(&mut self, read_only: bool, cx: &mut Context<Self>) {
        if self.development_read_only != read_only {
            self.development_read_only = read_only;
            self.update_fnx_editability(cx);
            cx.notify();
        }
    }

    pub(crate) fn source_is_dirty(&self, cx: &App) -> bool {
        self.source_save_in_progress
            || self.source_discard_in_progress
            || [&self.fnx_source_buffer, &self.json_source_buffer]
                .into_iter()
                .flatten()
                .any(|buffer| {
                    let buffer = buffer.read(cx);
                    buffer.is_dirty() || buffer.has_unsaved_edits()
                })
    }

    pub(crate) fn has_source_conflict(&self, cx: &App) -> bool {
        let item = self.item.read(cx);
        item.has_conflict() || (item.is_dirty() && self.source_is_dirty(cx))
    }

    pub(crate) fn save_source_edit(&mut self, cx: &mut Context<Self>) -> Option<Task<Result<()>>> {
        if self.source_save_in_progress || self.source_discard_in_progress {
            return Some(Task::ready(Err(anyhow!(
                "a source operation is already in progress"
            ))));
        }
        let files = match self.selected_file {
            CodeWorkspaceFile::Fnx => [CodeWorkspaceFile::Fnx, CodeWorkspaceFile::Json],
            CodeWorkspaceFile::Json => [CodeWorkspaceFile::Json, CodeWorkspaceFile::Fnx],
        };
        let (file, buffer, source_path) = files.into_iter().find_map(|file| {
            let (buffer, path) = match file {
                CodeWorkspaceFile::Fnx => (&self.fnx_source_buffer, &self.fnx_path),
                CodeWorkspaceFile::Json => (&self.json_source_buffer, &self.json_path),
            };
            buffer
                .as_ref()
                .filter(|buffer| buffer.read(cx).is_dirty())
                .zip(path.as_ref())
                .map(|(buffer, path)| (file, buffer.clone(), path.clone()))
        })?;
        let label = file.label();
        if buffer.read(cx).has_conflict() {
            return Some(Task::ready(Err(anyhow!(
                "source changed on disk; resolve the file conflict before saving"
            ))));
        }
        if self.item.read(cx).is_dirty() {
            return Some(Task::ready(Err(anyhow!(
                "save or discard canvas changes before saving source"
            ))));
        }
        if self.item.read(cx).project_root().is_none() {
            return Some(Task::ready(Err(anyhow!(
                "save the design before editing its source"
            ))));
        }
        let (source, version, capability) = buffer.read_with(cx, |buffer, _| {
            (buffer.text(), buffer.version(), buffer.capability())
        });
        let authored_transaction = buffer.update(cx, |buffer, cx| {
            let transaction = buffer
                .finalize_last_transaction()
                .map(|transaction| transaction.id);
            buffer.set_capability(Capability::Read, cx);
            transaction
        });
        self.source_save_in_progress = true;
        self.error_message = None;
        self.update_fnx_editability(cx);
        cx.notify();
        let project = self.project.clone();
        let item = self.item.clone();
        Some(cx.spawn(async move |this, cx| {
            let save_result: Result<()> = async {
                let source_write = item
                    .update(cx, |item, cx| {
                        item.apply_source_edit(source_path, source, cx)
                    })
                    .await?;
                anyhow::ensure!(
                    buffer.read_with(cx, |buffer, _| buffer.version() == version),
                    "source changed while saving; resolve the newer edit before retrying"
                );
                let reload_buffer = project.update(cx, |project, cx| {
                    project.reload_buffers(std::iter::once(buffer.clone()).collect(), true, cx)
                });
                let reloaded = reload_buffer.await?;
                if let Some(authored) = authored_transaction
                    && let Some(transaction) = reloaded.0.get(&buffer)
                    && transaction.id != authored
                {
                    buffer.update(cx, |buffer, _| {
                        if buffer.get_transaction(authored).is_some() {
                            buffer.merge_transactions(transaction.id, authored);
                        }
                    });
                }
                anyhow::ensure!(
                    buffer.read_with(cx, |buffer, _| !buffer.has_unsaved_edits()),
                    "source changed while refreshing the editor; resolve its file conflict"
                );
                let reload_canvas = item.update(cx, |item, cx| {
                    item.discard_canvas_edits_for_source_resolution(cx)
                });
                reload_canvas.await?;
                let diagnostics = source_write.finish_adoption();
                let all_sources_saved = this.update(cx, |this, cx| {
                    [&this.fnx_source_buffer, &this.json_source_buffer]
                        .into_iter()
                        .flatten()
                        .all(|buffer| {
                            !buffer.read(cx).is_dirty() && !buffer.read(cx).has_unsaved_edits()
                        })
                })?;
                if all_sources_saved
                    && item.read_with(cx, |item, _| item.has_unpersisted_source_layout())
                {
                    let save_layout =
                        item.update(cx, |item, cx| item.save_computed_source_layout(cx));
                    if let Err(error) = save_layout.await {
                        item.update(cx, |item, cx| item.mark_source_layout_unsaved(cx));
                        return Err(error);
                    }
                    let saved_buffers = this.update(cx, |this, cx| {
                        [&this.fnx_source_buffer, &this.json_source_buffer]
                            .into_iter()
                            .flatten()
                            .filter(|current| !current.read(cx).has_unsaved_edits())
                            .cloned()
                            .collect()
                    })?;
                    let reload_buffer = project.update(cx, |project, cx| {
                        project.reload_buffers(saved_buffers, true, cx)
                    });
                    let reloaded = reload_buffer
                        .await
                        .context("refreshing computed source geometry")?;
                    // The canonical geometry belongs to the source edit that caused it.
                    // A no-op reload can return that same transaction, so never forget it.
                    if let Some(authored) = authored_transaction
                        && let Some(transaction) = reloaded.0.get(&buffer)
                        && transaction.id != authored
                    {
                        buffer.update(cx, |buffer, _| {
                            if buffer.get_transaction(authored).is_some() {
                                buffer.merge_transactions(transaction.id, authored);
                            }
                        });
                    }
                }
                if !diagnostics.is_empty() {
                    log::warn!("FNX saved with {} source diagnostics", diagnostics.len());
                }
                Ok(())
            }
            .await;
            buffer.update(cx, |buffer, cx| buffer.set_capability(capability, cx));
            this.update(cx, |this, cx| {
                this.source_save_in_progress = false;
                let dirty = this.source_is_dirty(cx);
                item.update(cx, |item, cx| item.set_source_edit_locked(dirty, cx));
                this.error_message = save_result
                    .as_ref()
                    .err()
                    .map(|error| format!("Could not save {label}: {error:#}").into());
                this.update_fnx_editability(cx);
                cx.notify();
            })?;
            save_result?;
            if let Some(save_remaining) = this.update(cx, |this, cx| this.save_source_edit(cx))? {
                save_remaining.await?;
            }
            Ok(())
        }))
    }

    pub(crate) fn discard_source_edit(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        if self.source_save_in_progress || self.source_discard_in_progress {
            return Task::ready(Err(anyhow!(
                "wait for the source operation to finish before discarding changes"
            )));
        }
        let buffers = [&self.fnx_source_buffer, &self.json_source_buffer]
            .into_iter()
            .flatten()
            .filter(|buffer| buffer.read(cx).is_dirty())
            .cloned()
            .collect::<Vec<_>>();
        if buffers.is_empty() {
            return Task::ready(Ok(()));
        }
        let reload = self.project.update(cx, |project, cx| {
            project.reload_buffers(buffers.iter().cloned().collect(), false, cx)
        });
        let item = self.item.clone();
        cx.spawn(async move |this, cx| {
            reload.await?;
            let still_dirty = this.update(cx, |this, cx| {
                [&this.fnx_source_buffer, &this.json_source_buffer]
                    .into_iter()
                    .flatten()
                    .any(|buffer| {
                        let buffer = buffer.read(cx);
                        buffer.is_dirty() || buffer.has_unsaved_edits()
                    })
            })?;
            item.update(cx, |item, cx| {
                item.set_source_edit_locked(still_dirty, cx);
                if !still_dirty {
                    item.mark_source_layout_unsaved(cx);
                }
            });
            this.update(cx, |this, cx| {
                this.error_message = None;
                cx.notify();
            })?;
            if still_dirty {
                Err(anyhow!("source changed again while discarding its edits"))
            } else {
                Ok(())
            }
        })
    }

    fn discard_current_source_edit(&mut self, cx: &mut Context<Self>) -> Task<Result<()>> {
        if self.source_save_in_progress || self.source_discard_in_progress {
            return Task::ready(Err(anyhow!(
                "wait for the source operation to finish before discarding changes"
            )));
        }
        let source = match self.selected_file {
            CodeWorkspaceFile::Fnx => self.fnx_source_buffer.as_ref(),
            CodeWorkspaceFile::Json => self.json_source_buffer.as_ref(),
        };
        let Some(buffer) = source
            .filter(|buffer| {
                let buffer = buffer.read(cx);
                buffer.is_dirty() || buffer.has_unsaved_edits()
            })
            .cloned()
        else {
            return Task::ready(Ok(()));
        };
        let label = self.selected_file.label();
        let capability = buffer.read(cx).capability();
        buffer.update(cx, |buffer, cx| buffer.set_capability(Capability::Read, cx));
        self.source_discard_in_progress = true;
        self.update_fnx_editability(cx);
        cx.notify();
        let reload = self.project.update(cx, |project, cx| {
            project.reload_buffers(std::iter::once(buffer.clone()).collect(), false, cx)
        });
        cx.spawn(async move |this, cx| {
            let result: Result<()> = async {
                reload.await?;
                anyhow::ensure!(
                    buffer.read_with(cx, |buffer, _| {
                        !buffer.is_dirty() && !buffer.has_unsaved_edits()
                    }),
                    "source changed again while discarding its edits"
                );
                Ok(())
            }
            .await;
            buffer.update(cx, |buffer, cx| buffer.set_capability(capability, cx));
            this.update(cx, |this, cx| {
                this.source_discard_in_progress = false;
                let still_dirty = this.source_is_dirty(cx);
                this.item.update(cx, |item, cx| {
                    item.set_source_edit_locked(still_dirty, cx);
                    if !still_dirty {
                        item.mark_source_layout_unsaved(cx);
                    }
                });
                this.error_message = result
                    .as_ref()
                    .err()
                    .map(|error| format!("Could not discard {label}: {error:#}").into());
                this.update_fnx_editability(cx);
                cx.notify();
            })?;
            result
        })
    }

    fn sync_search_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.active_editor().cloned();
        if editor == self.search_editor {
            return;
        }
        self.search_bar.update(cx, |search_bar, cx| {
            search_bar.set_active_pane_item(
                editor.as_ref().map(|editor| editor as &dyn ItemHandle),
                window,
                cx,
            );
        });
        self.search_editor = editor;
    }

    fn saved_source_paths_changed(&self, cx: &App) -> bool {
        let item = self.item.read(cx);
        let page = self
            .requested_page
            .or_else(|| item.doc().and_then(|doc| doc.active_page()));
        let component = page.and_then(|root| {
            item.doc().and_then(|doc| {
                doc.components
                    .defs
                    .iter()
                    .find(|(_, definition)| definition.root == root)
                    .map(|(id, _)| *id)
            })
        });
        let (fnx_path, json_file) = match (item.project_root(), page, component) {
            (Some(root), Some(_), Some(component)) => (
                fanta_format::locate_master_source(root, component),
                "def.json",
            ),
            (Some(root), Some(page), None) => {
                (fanta_format::locate_page_source(root, page), "page.json")
            }
            _ => (None, "page.json"),
        };
        let json_path = fnx_path.as_ref().map(|path| path.with_file_name(json_file));
        self.fnx_path != fnx_path || self.json_path != json_path
    }

    fn reload_saved_sources(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let buffers: HashSet<_> = [
            self.fnx_source_buffer.as_ref(),
            self.json_source_buffer.as_ref(),
        ]
        .into_iter()
        .flatten()
        .filter(|buffer| !buffer.read(cx).has_unsaved_edits())
        .cloned()
        .collect();
        if buffers.is_empty() {
            return;
        }
        let reload = self
            .project
            .update(cx, |project, cx| project.reload_buffers(buffers, true, cx));
        cx.spawn_in(window, async move |this, cx| {
            let result = reload.await;
            if let Err(error) = this.update_in(cx, |this, _, cx| {
                if let Err(error) = result {
                    this.error_message =
                        Some(format!("Could not refresh saved source: {error:#}").into());
                    cx.notify();
                }
            }) {
                log::debug!("dropping source refresh for closed workspace: {error:#}");
            }
        })
        .detach();
    }

    fn refresh_from_item(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (project_root, page, component) = {
            let item = self.item.read(cx);
            let page = self
                .requested_page
                .or_else(|| item.doc().and_then(|doc| doc.active_page()));
            // The active root may be a component master (a component-scoped
            // view) rather than a listed page; its source is its folder's
            // `master.fnx` (`components/<slug>/`, or a variant's
            // `components/<set>/<variant>/`), with `def.json` as the JSON pane.
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
        if self.source_root != page {
            self.source_root = page;
            self.last_synced_selection = None;
        }
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
                self.clear_editors();
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
        self.fnx_source_buffer = None;
        self.json_source_buffer = None;
        self.fnx_source_observation = None;
        self.json_source_observation = None;
        self.fnx_source_version = None;
        self.json_source_version = None;
        self.loading_fnx = false;
        self.loading_json = false;
        self.fnx_load_task = None;
        self.json_load_task = None;
        self.last_synced_selection = None;
    }

    /// Opening the editor must not format or write the source. Saving validates
    /// the user's edit before the canvas adopts the changed file.
    fn open_fnx(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.fnx_path.as_ref() == Some(&path) && self.fnx_editor.is_some() {
            self.update_fnx_editability(cx);
            return;
        }
        self.fnx_path = Some(path.clone());
        self.fnx_editor = None;
        self.fnx_source_buffer = None;
        self.fnx_source_observation = None;
        self.loading_fnx = true;
        self.fnx_source_version = None;
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
                        this.fnx_source_observation =
                            Some(cx.observe_in(&buffer, window, |this, buffer, window, cx| {
                                this.source_buffer_changed(buffer, window, cx);
                            }));
                        this.fnx_source_buffer = Some(buffer.clone());
                        this.show_fnx_editor(buffer.clone(), window, cx);
                        this.source_buffer_changed(buffer, window, cx);
                        // The buffer arrives long after the selection that
                        // should be revealed in it, so catch up once here.
                        this.last_synced_selection = None;
                        this.sync_selection_to_source(window, cx);
                        this.follow_source_location(window, cx);
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
            self.update_fnx_editability(cx);
            return;
        }
        self.json_path = Some(path.clone());
        self.json_editor = None;
        self.json_source_buffer = None;
        self.json_source_observation = None;
        self.loading_json = true;
        self.json_source_version = None;
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
                        this.json_source_observation =
                            Some(cx.observe_in(&buffer, window, |this, buffer, window, cx| {
                                this.source_buffer_changed(buffer, window, cx);
                            }));
                        this.json_source_buffer = Some(buffer.clone());
                        this.show_json_editor(buffer.clone(), window, cx);
                        this.source_buffer_changed(buffer, window, cx);
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

    fn show_fnx_editor(
        &mut self,
        source_buffer: Entity<Buffer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project = self.project.clone();
        let read_only = self.development_read_only || self.item.read(cx).is_dirty();
        let editor = cx.new(|cx| {
            let mut editor = Editor::for_buffer(source_buffer, Some(project), window, cx);
            editor.set_read_only(read_only);
            editor
        });
        self.fnx_editor = Some(editor);
        self.last_synced_selection = None;
        self.sync_selection_to_source(window, cx);
        cx.notify();
    }

    fn source_buffer_changed(
        &mut self,
        buffer: Entity<Buffer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let version = buffer.read(cx).version();
        let previous_version = if self.fnx_source_buffer.as_ref() == Some(&buffer) {
            self.fnx_source_version.replace(version.clone())
        } else if self.json_source_buffer.as_ref() == Some(&buffer) {
            self.json_source_version.replace(version.clone())
        } else {
            None
        };
        let text_changed = previous_version.is_some_and(|previous| previous != version);
        let dirty = self.source_is_dirty(cx);
        self.item
            .update(cx, |item, cx| item.set_source_edit_locked(dirty, cx));
        if dirty && text_changed {
            self.error_message = None;
        }
        self.update_fnx_editability(cx);
        self.follow_source_location(window, cx);
        cx.notify();
    }

    fn update_fnx_editability(&mut self, cx: &mut Context<Self>) {
        let read_only = self.development_read_only
            || self.item.read(cx).is_dirty()
            || self.source_save_in_progress
            || self.source_discard_in_progress;
        for editor in [&self.fnx_editor, &self.json_editor].into_iter().flatten() {
            if editor.read(cx).read_only(cx) != read_only {
                editor.update(cx, |editor, _| editor.set_read_only(read_only));
            }
        }
    }

    fn show_json_editor(
        &mut self,
        source_buffer: Entity<Buffer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project = self.project.clone();
        let read_only = self.development_read_only || self.item.read(cx).is_dirty();
        let editor = cx.new(|cx| {
            let mut editor = Editor::for_buffer(source_buffer, Some(project), window, cx);
            editor.set_read_only(read_only);
            editor
        });
        self.json_editor = Some(editor);
        cx.notify();
    }

    pub(crate) fn reveal_source_path(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let file = if self.json_path.as_deref() == Some(path) {
            CodeWorkspaceFile::Json
        } else {
            CodeWorkspaceFile::Fnx
        };
        self.select_file(file, window, cx);
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
        self.sync_selection_to_source(window, cx);
        cx.notify();
    }

    fn active_editor(&self) -> Option<&Entity<Editor>> {
        match self.selected_file {
            CodeWorkspaceFile::Fnx => self.fnx_editor.as_ref(),
            CodeWorkspaceFile::Json => self.json_editor.as_ref(),
        }
    }

    fn active_path(&self) -> Option<&PathBuf> {
        match self.selected_file {
            CodeWorkspaceFile::Fnx => self.fnx_path.as_ref(),
            CodeWorkspaceFile::Json => self.json_path.as_ref(),
        }
    }

    /// The path as the project sees it — `pages/<slug>/page.fnx` — which is
    /// also the path an agent is told to edit. Falls back to the bare file
    /// name for a source that somehow sits outside the project root.
    fn project_relative_path(&self, path: &Path, cx: &App) -> SharedString {
        let relative = self
            .item
            .read(cx)
            .project_root()
            .and_then(|root| path.strip_prefix(root).ok());
        match relative {
            Some(relative) => relative.to_string_lossy().into_owned().into(),
            None => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string())
                .into(),
        }
    }

    /// Scroll the FNX pane to the opening tag of the one selected node, so the
    /// Code tab shows the same thing the canvas does.
    ///
    /// Deliberately not reachable from `render`: the scan is O(buffer) and the
    /// sidecar read touches the disk, so both are paid once per *changed*
    /// selection, never once per frame.
    fn sync_selection_to_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_file != CodeWorkspaceFile::Fnx {
            return;
        }
        let (Some(editor), Some(fnx_path)) = (self.fnx_editor.clone(), self.fnx_path.clone())
        else {
            return;
        };
        let selected = self.single_selection(cx);
        if self.last_synced_selection == selected {
            return;
        }
        self.last_synced_selection = selected;
        let Some(selected) = selected else {
            return;
        };

        let source = editor.read(cx).buffer().read(cx).snapshot(cx).text();
        let tags = opening_tags(&source);
        let Some(offset) = sidecar_offset(&fnx_path, selected, &tags)
            .or_else(|| self.named_offset(selected, &tags, cx))
        else {
            return;
        };
        editor.update(cx, |editor, cx| {
            editor.change_selections(
                SelectionEffects::scroll(Autoscroll::center()),
                window,
                cx,
                |selections| {
                    selections
                        .select_ranges([MultiBufferOffset(offset)..MultiBufferOffset(offset)]);
                },
            );
        });
    }

    /// The selected node when exactly one is selected. A multi-selection has
    /// no single opening tag to reveal, so it syncs nothing.
    fn single_selection(&self, cx: &App) -> Option<NodeId> {
        let item = self.item.read(cx);
        let doc = item.doc()?;
        let mut selected = doc.selection.iter();
        match (selected.next(), selected.next()) {
            (Some(id), None) => Some(*id),
            _ => None,
        }
    }

    /// Fallback for a missing or stale sidecar: match on the node's `name`
    /// attribute, disambiguated by the node's rank among the same-named nodes
    /// of this source's subtree (the scene walk is the same pre-order the
    /// source is printed in).
    fn named_offset(&self, id: NodeId, tags: &[OpeningTag], cx: &App) -> Option<usize> {
        let root = self.source_root?;
        let item = self.item.read(cx);
        let doc = item.doc()?;
        let name = doc.scene.get(id)?.name.clone();
        if name.is_empty() {
            return None;
        }
        let rank = doc
            .scene
            .descendants_of(root)
            .filter(|candidate| {
                doc.scene
                    .get(*candidate)
                    .is_some_and(|node| node.name == name)
            })
            .position(|candidate| candidate == id)?;
        tags.iter()
            .filter(|tag| tag.name.as_deref() == Some(name.as_str()))
            .nth(rank)
            .map(|tag| tag.offset)
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

    /// "Which file am I looking at?", answered in the header: the
    /// project-relative path, clickable to copy, with a reveal-in-file-manager
    /// button beside it.
    fn render_path(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let path = self.active_path()?.clone();
        let label = self.project_relative_path(&path, cx);
        let copied = label.clone();
        let reveal = path;
        Some(
            h_flex()
                .flex_none()
                .gap_0p5()
                .min_w_0()
                .child(
                    div()
                        .id("fanta-code-path")
                        .min_w_0()
                        .overflow_hidden()
                        .cursor_pointer()
                        .tooltip(Tooltip::text("Click to copy the path"))
                        .on_click(cx.listener(move |_, _, window, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copied.to_string()));
                            crate::view::show_canvas_notice("Copied path".to_string(), window, cx);
                        }))
                        .child(Label::new(label).size(LabelSize::Small).single_line()),
                )
                .child(
                    IconButton::new("fanta-code-reveal", IconName::FolderOpen)
                        .icon_size(IconSize::XSmall)
                        .tooltip(Tooltip::text("Reveal the source file"))
                        .on_click(move |_, _, cx| cx.reveal_path(&reveal)),
                )
                .into_any_element(),
        )
    }

    fn render_body(&self, cx: &App) -> AnyElement {
        if let Some(editor) = self.active_editor() {
            return div()
                .relative()
                .size_full()
                .child(editor.clone())
                .when(self.source_writer_matches_file(), |element| {
                    element.child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .w_full()
                            .h(px(3.0))
                            .bg(self.source_writer.as_ref().map_or_else(
                                || gpui::rgb(0x387bff).into(),
                                |writer| design_surface::agent_color(&writer.agent_id),
                            ))
                            .with_animation(
                                "live-source-writing",
                                Animation::new(Duration::from_millis(1200))
                                    .repeat()
                                    .with_easing(pulsating_between(0.35, 1.0)),
                                |element, opacity| element.opacity(opacity),
                            ),
                    )
                })
                .into_any_element();
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

/// The byte offset of the opening tag belonging to `id`, taken from the
/// `.ids.json` sidecar beside the source: its entries are in the same
/// pre-order as the opening tags, so entry *k* is tag *k*. A count mismatch
/// means the source was edited since the sidecar was written, so the caller
/// falls back to matching on `name`.
fn sidecar_offset(fnx_path: &Path, id: NodeId, tags: &[OpeningTag]) -> Option<usize> {
    let sidecar_name = match fnx_path.file_name().and_then(OsStr::to_str)? {
        "page.fnx" => "page.ids.json",
        "master.fnx" => "master.ids.json",
        _ => return None,
    };
    let text = std::fs::read_to_string(fnx_path.with_file_name(sidecar_name)).ok()?;
    let sidecar: SourceIdSidecar = serde_json::from_str(&text).ok()?;
    if sidecar.ids.len() != tags.len() {
        return None;
    }
    // The sidecar stores the id exactly as the document JSON does — a bare
    // ULID — which is NOT what `NodeId`'s `Display` prints (it prefixes `n_`).
    let wanted = serde_json::to_value(id).ok()?.as_str()?.to_owned();
    let index = sidecar.ids.iter().position(|entry| entry.id == wanted)?;
    tags.get(index).map(|tag| tag.offset)
}

/// Every opening tag in an `.fnx` source, in document order.
///
/// A naive scan for `<` followed by a capital letter miscounts: attribute
/// values carry arbitrary user text, so a Text node's characters can contain
/// a literal `<Frame>`. This walks tags whole — once inside a tag, quoted
/// values and braced expressions are skipped over — and only accepts names
/// the FNX language actually defines.
fn opening_tags(source: &str) -> Vec<OpeningTag> {
    let bytes = source.as_bytes();
    let mut tags = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'<' {
            index += 1;
            continue;
        }
        let name_start = index + 1;
        let name_end = identifier_end(bytes, name_start);
        // A closing tag (`</Frame>`) has a `/` here, so it never matches.
        if name_end == name_start || !FNX_TAGS.contains(&&source[name_start..name_end]) {
            index += 1;
            continue;
        }
        // The delimiter is what separates `<Text …>` from a longer identifier
        // that merely starts with a tag name.
        match bytes.get(name_end) {
            Some(b' ' | b'\t' | b'\n' | b'\r' | b'/' | b'>') => {}
            _ => {
                index += 1;
                continue;
            }
        }
        let (end, name) = scan_tag(source, name_end);
        tags.push(OpeningTag {
            offset: index,
            name,
        });
        index = end;
    }
    tags
}

/// Walk from just past an opening tag's name to the `>` that closes it,
/// stepping over quoted attribute values and braced expressions, and pick up
/// the `name` attribute on the way. Returns the offset just past the `>`.
fn scan_tag(source: &str, start: usize) -> (usize, Option<String>) {
    let bytes = source.as_bytes();
    let mut index = start;
    let mut braces = 0usize;
    let mut name = None;
    while index < bytes.len() {
        let byte = bytes[index];
        if braces == 0 && byte == b'>' {
            return (index + 1, name);
        }
        match byte {
            b'{' => {
                braces += 1;
                index += 1;
            }
            b'}' => {
                braces = braces.saturating_sub(1);
                index += 1;
            }
            b'"' | b'\'' => index = quoted_end(bytes, index),
            _ if braces == 0 && is_identifier_start(byte) => {
                let key_end = identifier_end(bytes, index);
                let is_name_attribute = &source[index..key_end] == "name"
                    && bytes.get(key_end) == Some(&b'=')
                    && bytes.get(key_end + 1) == Some(&b'"');
                if is_name_attribute {
                    let value_end = quoted_end(bytes, key_end + 1);
                    if name.is_none() {
                        name = serde_json::from_str::<String>(&source[key_end + 1..value_end]).ok();
                    }
                    index = value_end;
                } else {
                    index = key_end;
                }
            }
            _ => index += 1,
        }
    }
    (bytes.len(), name)
}

/// The offset just past the closing quote of the string starting at `open`
/// (or the end of input for an unterminated one).
fn quoted_end(bytes: &[u8], open: usize) -> usize {
    let quote = bytes[open];
    let mut index = open + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            byte if byte == quote => return index + 1,
            _ => index += 1,
        }
    }
    bytes.len()
}

fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$'
}

/// The offset just past the ASCII identifier starting at `start`, or `start`
/// itself when there is no identifier there.
fn identifier_end(bytes: &[u8], start: usize) -> usize {
    if !bytes.get(start).copied().is_some_and(is_identifier_start) {
        return start;
    }
    let mut index = start + 1;
    while index < bytes.len() && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_') {
        index += 1;
    }
    index
}

impl Focusable for FantaCodeWorkspace {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.active_editor()
            .map(|editor| editor.focus_handle(cx))
            .unwrap_or_else(|| self.focus_handle.clone())
    }
}

impl Render for FantaCodeWorkspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_search_editor(window, cx);
        let error_message = self.error_message.clone().or_else(|| {
            self.item
                .read(cx)
                .source_reload_error()
                .map(SharedString::from)
        });
        let status_color = if error_message.is_some() {
            Color::Error
        } else {
            Color::Muted
        };
        let status = error_message.unwrap_or_else(|| {
            if self.source_discard_in_progress {
                format!("Discarding {} draft…", self.selected_file.label()).into()
            } else if self.source_save_in_progress {
                format!("Saving {}…", self.selected_file.label()).into()
            } else if self.source_writer_matches_file() {
                format!(
                    "{} is writing · {}",
                    self.source_writer
                        .as_ref()
                        .map_or("Agent", |writer| writer.agent_name.as_str()),
                    if self.follow_source_writer {
                        "Following live source"
                    } else {
                        "Follow paused"
                    },
                )
                .into()
            } else if self.item.read(cx).is_dirty() {
                "Save canvas changes before editing source".into()
            } else if self.source_is_dirty(cx) {
                "Unsaved source edits · save to validate and update the canvas".into()
            } else {
                EDITABLE_STATUS.into()
            }
        });
        let selected_dirty = match self.selected_file {
            CodeWorkspaceFile::Fnx => self.fnx_source_buffer.as_ref(),
            CodeWorkspaceFile::Json => self.json_source_buffer.as_ref(),
        }
        .is_some_and(|buffer| buffer.read(cx).is_dirty() || buffer.read(cx).has_unsaved_edits());
        let mut context = gpui::KeyContext::new_with_defaults();
        self.search_bar
            .read(cx)
            .contribute_context(&mut context, cx);
        let mut registrar = DivRegistrar::new(
            |this: &Self, _, _| this.active_editor().map(|_| this.search_bar.clone()),
            cx,
        );
        BufferSearchBar::register(&mut registrar);
        registrar
            .into_div()
            .v_flex()
            .key_context(context)
            .track_focus(&self.focus_handle)
            .size_full()
            // The floating workspace selector must not cover source controls.
            .pt(px(48.))
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
                    .children(self.render_path(cx))
                    .child(
                        div().flex_1().min_w_0().overflow_hidden().child(
                            Label::new(status)
                                .size(LabelSize::Small)
                                .color(status_color)
                                .single_line(),
                        ),
                    )
                    .child(
                        Button::new("fanta-source-find", "Find")
                            .size(ButtonSize::Compact)
                            .disabled(self.active_editor().is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.sync_search_editor(window, cx);
                                this.search_bar.update(cx, |search_bar, cx| {
                                    search_bar.deploy(&Deploy::find(), None, window, cx);
                                });
                                cx.notify();
                            })),
                    )
                    .when(selected_dirty, |element| {
                        element.child(
                            div()
                                .debug_selector(|| "fanta-source-discard-target".to_owned())
                                .child(
                                    Button::new("fanta-source-discard", "Discard draft")
                                        .size(ButtonSize::Compact)
                                        .disabled(
                                            self.source_save_in_progress
                                                || self.source_discard_in_progress,
                                        )
                                        .tooltip(Tooltip::text(
                                            "Discard unsaved changes in the selected source file",
                                        ))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.discard_current_source_edit(cx)
                                                .detach_and_log_err(cx);
                                        })),
                                ),
                        )
                    })
                    .when(self.source_writer_matches_file(), |element| {
                        element.child(
                            Button::new(
                                "follow-live-source",
                                if self.follow_source_writer {
                                    "Pause follow"
                                } else {
                                    "Follow"
                                },
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.follow_source_writer = !this.follow_source_writer;
                                    this.follow_source_location(window, cx);
                                    cx.notify();
                                },
                            )),
                        )
                    }),
            )
            .when(
                !self.search_bar.read(cx).is_dismissed() && self.active_editor().is_some(),
                |element| {
                    element.child(
                        div()
                            .debug_selector(|| "fanta-source-search".to_owned())
                            .flex_none()
                            .child(self.search_bar.clone()),
                    )
                },
            )
            .child(div().flex_1().min_h_0().child(self.render_body(cx)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use editor::ToPoint as _;
    use fanta_doc::{CanvasNode, Doc, GroupNode, IndexKey, NodeData};
    use gpui::TestAppContext;
    use project::{ProjectItem as _, ProjectPath};
    use settings::Settings as _;

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

    /// A page with two named children, so the selection sync has a tag other
    /// than the root to find. Returns the page and its second child.
    fn write_project_with_children(root: &Path) -> (NodeId, NodeId) {
        let mut document = Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "Home".to_owned();
        let page_id = page.id;
        document.scene.insert(page).expect("insert page");
        document.add_page(page_id);

        let mut second_id = None;
        for (name, index) in [
            ("First child", IndexKey::FIRST),
            ("Second child", IndexKey::after(IndexKey::FIRST)),
        ] {
            let mut child = CanvasNode::new(NodeData::Group(GroupNode::default()));
            child.name = name.to_owned();
            child.parent = Some(page_id);
            child.index = index;
            second_id = Some(child.id);
            document.scene.insert(child).expect("insert child");
        }

        fanta_format::write_project_tree(root, &document, &BTreeMap::new())
            .expect("write project tree");
        (page_id, second_id.expect("two children were inserted"))
    }

    /// The row the FNX pane's caret sits on — where the selection sync
    /// scrolled it.
    fn fnx_selection_row(
        workspace: &gpui::WindowHandle<FantaCodeWorkspace>,
        cx: &mut TestAppContext,
    ) -> u32 {
        workspace
            .read_with(cx, |workspace, cx| {
                let editor = workspace.fnx_editor.as_ref().expect("FNX editor").read(cx);
                let snapshot = editor.buffer().read(cx).snapshot(cx);
                editor
                    .selections
                    .newest_anchor()
                    .head()
                    .to_point(&snapshot)
                    .row
            })
            .expect("read code workspace")
    }

    /// The row of the opening tag carrying `name` in the source editor.
    fn source_row_of(source_path: &Path, name: &str) -> u32 {
        let source = std::fs::read_to_string(source_path).expect("read FNX source");
        let needle = format!("name=\"{name}\"");
        let attribute_offset = source
            .find(&needle)
            .expect("the child is named in the source");
        let opening_offset = source[..attribute_offset]
            .rfind('<')
            .expect("the child has an opening tag");
        source[..opening_offset]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count() as u32
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

    #[gpui::test]
    async fn source_follow_uses_the_shared_buffer_and_respects_pause(cx: &mut TestAppContext) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        let (page, child) = write_project_with_children(temporary.path());
        let (project, _, workspace) = open_code_workspace(temporary.path(), cx).await;
        cx.run_until_parked();
        let path = fanta_format::locate_page_source(temporary.path(), page).expect("source");
        let buffer = workspace
            .read_with(cx, |workspace, _| {
                workspace
                    .fnx_source_buffer
                    .clone()
                    .expect("shared source buffer")
            })
            .expect("workspace");
        let position = buffer.read_with(cx, |buffer, _| {
            buffer.anchor_before(language::Point::new(
                source_row_of(&path, "Second child"),
                0,
            ))
        });
        let writer = design_surface::AgentActivity {
            agent_id: "lead".into(),
            agent_name: "Designer".into(),
            action: "Editing source".into(),
            page: Some(0),
            node: Some(child.to_string()),
            world: None,
            active: true,
            project_root: Some(temporary.path().display().to_string()),
            source_path: Some(path.display().to_string()),
            workspace: None,
        };
        workspace
            .update(cx, |workspace, window, cx| {
                workspace.set_source_writer(Some(writer.clone()), window, cx);
            })
            .expect("set writer");
        project.update(cx, |project, cx| {
            project.set_agent_location(
                Some(project::AgentLocation {
                    buffer: buffer.downgrade(),
                    position,
                }),
                cx,
            );
        });
        cx.run_until_parked();
        assert_eq!(
            fnx_selection_row(&workspace, cx),
            source_row_of(&path, "Second child")
        );
        workspace
            .update(cx, |workspace, _, _| workspace.follow_source_writer = false)
            .expect("pause");
        let position = buffer.read_with(cx, |buffer, _| buffer.anchor_before(0));
        project.update(cx, |project, cx| {
            project.set_agent_location(
                Some(project::AgentLocation {
                    buffer: buffer.downgrade(),
                    position,
                }),
                cx,
            );
        });
        cx.run_until_parked();
        assert_eq!(
            fnx_selection_row(&workspace, cx),
            source_row_of(&path, "Second child")
        );
        workspace
            .update(cx, |workspace, window, cx| {
                workspace.set_source_writer(Some(writer), window, cx);
                assert!(
                    !workspace.follow_source_writer,
                    "stream chunks must not reset manual pause"
                );
                workspace.set_source_writer(None, window, cx);
                assert!(!workspace.source_writer_matches_file());
            })
            .expect("stop writer");
    }

    #[gpui::test]
    async fn a_scoped_open_reuses_the_shared_project_item_and_applies_the_scope(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        let page = write_project(temporary.path());
        let (project, item, _workspace) = open_code_workspace(temporary.path(), cx).await;
        cx.update(|cx| {
            project::DisableAiSettings::register(cx);
            workspace::register_project_item::<crate::view::FigView>(cx);
        });
        let window = cx.add_window(|_, _| gpui::Empty);
        let workspace = window
            .update(cx, |_, window, cx| {
                cx.new(|cx| workspace::Workspace::test_new(project.clone(), window, cx))
            })
            .expect("create editor workspace");
        let initial_open = window
            .update(cx, |_, window, cx| {
                workspace.update(cx, |workspace, cx| {
                    workspace.open_abs_path(
                        temporary.path().join("fanta.json"),
                        workspace::OpenOptions {
                            focus: Some(false),
                            ..Default::default()
                        },
                        window,
                        cx,
                    )
                })
            })
            .expect("open manifest tab");
        let initial_tab = initial_open.await.expect("manifest tab");
        cx.run_until_parked();

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
                None,
                "loading a scoped item must preserve the visible document scope"
            );
        });
        let background_open = window
            .update(cx, |_, window, cx| {
                workspace.update(cx, |workspace, cx| {
                    workspace.open_path_preview(scoped_path, None, false, false, false, window, cx)
                })
            })
            .expect("open page tab without activation");
        let background_tab = background_open.await.expect("page tab");
        cx.run_until_parked();
        assert_eq!(
            workspace.read_with(cx, |workspace, cx| workspace
                .active_item(cx)
                .map(|item| item.item_id())),
            Some(initial_tab.item_id()),
            "the manifest stays active while the page tab loads"
        );
        item.read_with(cx, |item, _| {
            assert_eq!(item.doc().and_then(|doc| doc.active_page()), None);
        });
        window
            .update(cx, |_, window, cx| {
                workspace.update(cx, |workspace, cx| {
                    assert!(workspace.activate_item(&*background_tab, true, false, window, cx));
                });
            })
            .expect("activate page tab");
        cx.run_until_parked();
        item.read_with(cx, |item, _| {
            assert_eq!(
                item.doc().and_then(|doc| doc.active_page()),
                Some(page),
                "activating the scoped tab navigates the shared document to its page"
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

    #[gpui::test]
    async fn saved_reconciles_renamed_page_source_paths(cx: &mut TestAppContext) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        let page = write_project(temporary.path());
        let original_path = page_source_path(temporary.path(), page);
        let (_project, item, workspace) = open_code_workspace(temporary.path(), cx).await;
        item.update(cx, |item, cx| {
            item.apply(
                fanta_doc::Operation::SetName {
                    id: page,
                    old: "Original".into(),
                    new: "Renamed page".into(),
                },
                cx,
            )
            .expect("rename page");
        });
        item.update(cx, |item, cx| {
            item.save(crate::document::SaveKind::Auto, cx)
        })
        .await
        .expect("save renamed page");
        cx.run_until_parked();
        let current_path = page_source_path(temporary.path(), page);
        assert_ne!(
            current_path, original_path,
            "renaming moves the source slug"
        );
        assert!(!original_path.exists(), "the old source was pruned");
        workspace
            .read_with(cx, |workspace, cx| {
                assert_eq!(workspace.fnx_path.as_ref(), Some(&current_path));
                assert_eq!(
                    workspace.json_path.as_ref(),
                    Some(&current_path.with_file_name("page.json"))
                );
                let source = workspace
                    .fnx_editor
                    .as_ref()
                    .expect("renamed FNX editor")
                    .read(cx)
                    .buffer()
                    .read(cx)
                    .snapshot(cx)
                    .text();
                assert!(source.contains("name=\"Renamed page\""));
                assert!(workspace.json_editor.is_some());
                assert!(workspace.error_message.is_none());
            })
            .expect("read renamed source workspace");
    }

    #[gpui::test]
    async fn saved_reconciles_first_materialized_source(cx: &mut TestAppContext) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary parent");
        let project = open_test_project(temporary.path(), cx).await;
        let mut document = Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "First page".into();
        let page_id = page.id;
        document.scene.insert(page).expect("insert page");
        document.add_page(page_id);
        let item = crate::document::ready_item_for_test(
            &project,
            temporary.path().join("New design.fig"),
            document,
            cx,
        );
        let workspace = cx.add_window({
            let item = item.clone();
            move |window, cx| FantaCodeWorkspace::new(item, project, window, cx)
        });
        workspace
            .read_with(cx, |workspace, _| {
                assert!(workspace.fnx_editor.is_none());
                assert!(
                    workspace
                        .error_message
                        .as_ref()
                        .is_some_and(|message| { message.contains("Save the document once") })
                );
            })
            .expect("read unmaterialized source workspace");
        // Exercise the Saved event already used by persistence before explicit
        // saves adopt it, without relying on their old StateChanged refresh.
        let root = item
            .update(cx, |item, cx| {
                item.save(crate::document::SaveKind::Auto, cx)
            })
            .await
            .expect("materialize the saved snapshot")
            .expect("new project root");
        cx.run_until_parked();
        workspace
            .read_with(cx, |workspace, _| {
                assert_eq!(workspace.fnx_path, Some(page_source_path(&root, page_id)));
                assert_eq!(workspace.json_path, Some(page_json_path(&root, page_id)));
                assert!(workspace.fnx_editor.is_some());
                assert!(workspace.json_editor.is_some());
                assert!(workspace.error_message.is_none());
            })
            .expect("read materialized source workspace");
    }

    #[gpui::test]
    async fn saved_reconciliation_preserves_editors_when_source_paths_are_unchanged(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        write_project(temporary.path());
        let (_project, item, workspace) = open_code_workspace(temporary.path(), cx).await;
        let editors = workspace
            .update(cx, |workspace, _, _| {
                workspace.selected_file = CodeWorkspaceFile::Json;
                (
                    workspace
                        .fnx_editor
                        .as_ref()
                        .expect("FNX editor")
                        .entity_id(),
                    workspace
                        .json_editor
                        .as_ref()
                        .expect("JSON editor")
                        .entity_id(),
                )
            })
            .expect("select JSON pane");
        item.update(cx, |item, cx| {
            item.save(crate::document::SaveKind::Auto, cx)
        })
        .await
        .expect("save unchanged source paths");
        cx.run_until_parked();
        workspace
            .read_with(cx, |workspace, _| {
                assert_eq!(workspace.selected_file, CodeWorkspaceFile::Json);
                assert_eq!(
                    workspace
                        .fnx_editor
                        .as_ref()
                        .expect("FNX editor")
                        .entity_id(),
                    editors.0
                );
                assert_eq!(
                    workspace
                        .json_editor
                        .as_ref()
                        .expect("JSON editor")
                        .entity_id(),
                    editors.1
                );
            })
            .expect("read preserved editors");
    }

    #[gpui::test]
    async fn saved_canvas_edits_update_the_source_editor(cx: &mut TestAppContext) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        let (_page, child) = write_project_with_children(temporary.path());
        let (_project, item, workspace) = open_code_workspace(temporary.path(), cx).await;
        cx.run_until_parked();

        item.update(cx, |item, cx| {
            item.apply(
                fanta_doc::Operation::SetName {
                    id: child,
                    old: "Second child".into(),
                    new: "Updated child".into(),
                },
                cx,
            )
            .expect("rename child");
        });
        item.update(cx, |item, cx| {
            item.save(crate::document::SaveKind::Auto, cx)
        })
        .await
        .expect("save renamed child");
        cx.run_until_parked();

        workspace
            .read_with(cx, |workspace, cx| {
                let editor = workspace.fnx_editor.as_ref().expect("FNX preview");
                let source = editor.read(cx).buffer().read(cx).snapshot(cx).text();
                assert!(source.contains("name=\"Updated child\""));
            })
            .expect("read updated preview");
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

    #[gpui::test]
    async fn fnx_and_json_source_are_editable(cx: &mut TestAppContext) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        write_project(temporary.path());
        let (_project, _item, workspace) = open_code_workspace(temporary.path(), cx).await;

        workspace
            .read_with(cx, |workspace, cx| {
                let fnx_editor = workspace.fnx_editor.as_ref().expect("FNX editor");
                let json_editor = workspace.json_editor.as_ref().expect("JSON editor");
                assert!(!fnx_editor.read(cx).read_only(cx), "FNX source is editable");
                assert!(
                    !json_editor.read(cx).read_only(cx),
                    "JSON source is editable"
                );
            })
            .expect("read code workspace");
    }

    #[gpui::test]
    async fn saving_fnx_source_updates_the_file_and_canvas(cx: &mut TestAppContext) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        let page = write_project(temporary.path());
        let source_path = page_source_path(temporary.path(), page);
        let original = std::fs::read_to_string(&source_path).expect("read FNX");
        let changed = original.replace("name=\"Original\"", "name=\"Saved edit\"");
        assert_ne!(changed, original);
        let (_project, item, workspace) = open_code_workspace(temporary.path(), cx).await;

        workspace
            .update(cx, |workspace, window, cx| {
                workspace
                    .fnx_editor
                    .as_ref()
                    .expect("FNX editor")
                    .update(cx, |editor, cx| {
                        editor.set_text(changed.clone(), window, cx)
                    });
            })
            .expect("edit FNX");
        cx.run_until_parked();
        item.read_with(cx, |item, _| assert!(item.source_edit_locked()));

        let save = workspace
            .update(cx, |workspace, _, cx| {
                workspace.save_source_edit(cx).expect("dirty FNX source")
            })
            .expect("start FNX save");
        workspace
            .read_with(cx, |workspace, cx| {
                assert!(workspace.source_save_in_progress);
                assert!(workspace.source_is_dirty(cx));
                assert!(
                    workspace
                        .fnx_editor
                        .as_ref()
                        .expect("FNX editor")
                        .read(cx)
                        .read_only(cx)
                );
                assert_eq!(
                    workspace
                        .fnx_source_buffer
                        .as_ref()
                        .expect("FNX buffer")
                        .read(cx)
                        .capability(),
                    Capability::Read
                );
            })
            .expect("read frozen FNX editor");
        save.await.expect("save validated FNX");
        cx.run_until_parked();

        assert_eq!(
            std::fs::read_to_string(&source_path).expect("read saved FNX"),
            changed
        );
        item.read_with(cx, |item, _| {
            assert_eq!(
                item.doc()
                    .and_then(|doc| doc.scene.get(page))
                    .map(|node| node.name.as_str()),
                Some("Saved edit")
            );
            assert!(!item.source_edit_locked());
            assert!(item.is_editable());
        });
        workspace
            .read_with(cx, |workspace, cx| {
                assert!(!workspace.source_is_dirty(cx));
                assert_eq!(
                    workspace
                        .fnx_source_buffer
                        .as_ref()
                        .expect("FNX buffer")
                        .read(cx)
                        .capability(),
                    Capability::ReadWrite
                );
            })
            .expect("read code workspace");
    }

    fn write_layout_project(root: &Path) -> (NodeId, Vec<NodeId>, Doc) {
        let page = write_project(root);
        let (mut document, assets) = fanta_format::read_project_tree(root).expect("read project");
        let mut text_ids = Vec::new();
        for index in 0..2 {
            let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
                clip_size: Some([220.0, 100.0]),
                auto_layout: Some(fanta_doc::AutoLayout::default()),
                ..Default::default()
            }));
            frame.parent = Some(page);
            frame.name = format!("Layout island {index}");
            let parent = frame.id;
            document.scene.insert(frame).expect("frame");
            let mut text =
                fanta_doc::TextNode::new(format!("Original layout label {index}"), 200.0, 18.0);
            text.auto_resize = fanta_doc::TextAutoResize::WidthAndHeight;
            let mut node = CanvasNode::new(NodeData::Text(text));
            node.parent = Some(parent);
            node.transform = fanta_doc::Transform2D::translation(12.0, 36.0);
            text_ids.push(node.id);
            document.scene.insert(node).expect("text");
        }
        fanta_format::write_project_tree(root, &document, &assets).expect("write computed project");
        (page, text_ids, document)
    }

    #[gpui::test]
    async fn loaded_layout_fnx_edit_saves_computed_geometry_and_preserves_other_islands(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        let (page, text_ids, document) = write_layout_project(temporary.path());
        let original_other = document
            .scene
            .get(text_ids[1])
            .expect("untouched text")
            .clone();
        let source_path = page_source_path(temporary.path(), page);
        let original_source = std::fs::read_to_string(&source_path).expect("FNX");
        let changed = original_source.replace(
            "Original layout label 0",
            "An intentionally wider authored source label",
        );
        assert_ne!(changed, original_source);
        let (_project, item, workspace) = open_code_workspace(temporary.path(), cx).await;
        item.read_with(cx, |item, _| {
            assert_eq!(
                item.doc().expect("document").scene.get(text_ids[1]),
                Some(&original_other)
            );
        });
        workspace
            .update(cx, |workspace, window, cx| {
                workspace
                    .fnx_editor
                    .as_ref()
                    .expect("FNX editor")
                    .update(cx, |editor, cx| {
                        editor.set_text(changed, window, cx);
                    });
            })
            .expect("edit source");
        cx.run_until_parked();
        let (arrived, resume) = item.read_with(cx, |item, _| item.pause_next_save_for_test());
        let save = workspace
            .update(cx, |workspace, _, cx| {
                workspace.save_source_edit(cx).expect("source save")
            })
            .expect("start save");
        arrived
            .await
            .expect("computed layout writer reached barrier");
        item.update(cx, |item, cx| {
            assert!(
                item.source_edit_locked(),
                "source Save retains its canvas edit barrier"
            );
            assert!(
                item.apply(
                    fanta_doc::Operation::SetName {
                        id: page,
                        old: "Original".into(),
                        new: "Rejected concurrent edit".into()
                    },
                    cx
                )
                .is_err()
            );
        });
        resume.send(()).expect("resume computed write");
        save.await.expect("save computed source geometry");
        cx.run_until_parked();
        let final_scene = item.read_with(cx, |item, _| {
            let doc = item.doc().expect("document");
            let NodeData::Text(text) = &doc.scene.get(text_ids[0]).expect("edited text").data
            else {
                panic!("text fixture")
            };
            assert_ne!(text.local_size, [200.0, 18.0]);
            assert_eq!(
                doc.scene.get(text_ids[1]),
                Some(&original_other),
                "the other island remains authoritative"
            );
            assert!(!item.source_edit_locked());
            assert!(!item.is_dirty());
            assert!(!item.has_unpersisted_source_layout());
            serde_json::to_value(&doc.scene).expect("canvas scene")
        });
        let (saved, _) =
            fanta_format::read_project_tree(temporary.path()).expect("reopen saved project");
        assert_eq!(
            serde_json::to_value(&saved.scene).expect("saved scene"),
            final_scene,
            "computed coordinates must be persisted by the same explicit FNX Save"
        );
        workspace
            .read_with(cx, |workspace, cx| {
                assert!(!workspace.source_is_dirty(cx));
                let actual = workspace
                    .fnx_source_buffer
                    .as_ref()
                    .expect("source buffer")
                    .read(cx)
                    .text();
                assert_eq!(
                    actual,
                    std::fs::read_to_string(&source_path).expect("saved FNX")
                );
            })
            .expect("clean editor displays persisted geometry");

        let saved_source = std::fs::read_to_string(&source_path).expect("saved source");
        workspace
            .update(cx, |workspace, window, cx| {
                workspace
                    .fnx_editor
                    .as_ref()
                    .expect("editor")
                    .update(cx, |editor, cx| {
                        editor.set_text("<Frame name={ />", window, cx)
                    });
            })
            .expect("invalid edit");
        cx.run_until_parked();
        let invalid = workspace
            .update(cx, |workspace, _, cx| {
                workspace.save_source_edit(cx).expect("dirty source")
            })
            .expect("invalid save");
        assert!(invalid.await.is_err());
        assert_eq!(
            std::fs::read_to_string(&source_path).expect("saved source"),
            saved_source
        );
        item.read_with(cx, |item, _| {
            assert!(item.source_edit_locked());
            assert_eq!(
                serde_json::to_value(&item.doc().expect("document").scene).expect("canvas"),
                final_scene
            );
        });
    }

    #[gpui::test]
    async fn loaded_layout_two_source_drafts_keep_canvas_locked_until_both_save(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        let (page, text_ids, _) = write_layout_project(temporary.path());
        let source_path = page_source_path(temporary.path(), page);
        let original = std::fs::read_to_string(&source_path).expect("FNX");
        let changed = original.replace(
            "Original layout label 0",
            "A changed source label that needs reflow",
        );
        let json_path = page_json_path(temporary.path(), page);
        let header = std::fs::read_to_string(&json_path).expect("header");
        let (_project, item, workspace) = open_code_workspace(temporary.path(), cx).await;
        workspace
            .update(cx, |workspace, window, cx| {
                workspace.selected_file = CodeWorkspaceFile::Fnx;
                workspace
                    .fnx_editor
                    .as_ref()
                    .expect("FNX")
                    .update(cx, |editor, cx| editor.set_text(changed, window, cx));
                workspace
                    .json_editor
                    .as_ref()
                    .expect("JSON")
                    .update(cx, |editor, cx| {
                        editor.set_text("{ invalid header", window, cx)
                    });
            })
            .expect("edit both buffers");
        cx.run_until_parked();
        let lock_violations = std::rc::Rc::new(std::cell::Cell::new(0));
        let json_buffer = workspace
            .read_with(cx, |workspace, _| {
                workspace.json_source_buffer.as_ref().expect("JSON").clone()
            })
            .expect("buffer");
        let _subscription = cx.update(|cx| {
            cx.observe(&item, {
                let violations = lock_violations.clone();
                move |item, cx| {
                    if json_buffer.read(cx).is_dirty() && !item.read(cx).source_edit_locked() {
                        violations.set(violations.get() + 1);
                    }
                }
            })
        });
        let save = workspace
            .update(cx, |workspace, _, cx| {
                workspace.save_source_edit(cx).expect("save both")
            })
            .expect("start save");
        assert!(
            save.await.is_err(),
            "invalid second buffer remains rejected"
        );
        cx.run_until_parked();
        assert_eq!(
            lock_violations.get(),
            0,
            "no unlock while the other source draft is dirty"
        );
        item.read_with(cx, |item, _| {
            assert!(item.source_edit_locked());
            assert!(
                item.has_unpersisted_source_layout(),
                "geometry awaits the final valid source save"
            );
            assert!(
                !item.is_dirty(),
                "a rejected source draft does not manufacture canvas conflict"
            );
        });
        workspace
            .update(cx, |workspace, window, cx| {
                workspace
                    .json_editor
                    .as_ref()
                    .expect("JSON")
                    .update(cx, |editor, cx| {
                        let mut valid: serde_json::Value =
                            serde_json::from_str(&header).expect("header JSON");
                        valid["order"] = serde_json::json!(9);
                        editor.set_text(valid.to_string(), window, cx);
                    });
            })
            .expect("repair header");
        cx.run_until_parked();
        workspace
            .update(cx, |workspace, _, cx| {
                workspace.save_source_edit(cx).expect("retry source")
            })
            .expect("start retry")
            .await
            .expect("both sources and layout save");
        cx.run_until_parked();
        let (saved, _) = fanta_format::read_project_tree(temporary.path()).expect("reopen");
        item.read_with(cx, |item, _| {
            assert!(!item.source_edit_locked());
            assert!(!item.is_dirty());
            assert!(!item.has_unpersisted_source_layout());
            assert_eq!(
                saved.scene.get(text_ids[0]),
                item.doc().expect("document").scene.get(text_ids[0])
            );
        });
    }

    #[gpui::test]
    async fn loaded_layout_failed_geometry_write_retains_retryable_canvas_state(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let directory = tempfile::tempdir().expect("project");
        let (page, _, _) = write_layout_project(directory.path());
        let source_path = page_source_path(directory.path(), page);
        let original = std::fs::read_to_string(&source_path).expect("FNX");
        let (_project, item, workspace) = open_code_workspace(directory.path(), cx).await;
        workspace
            .update(cx, |workspace, window, cx| {
                workspace
                    .fnx_editor
                    .as_ref()
                    .expect("FNX")
                    .update(cx, |editor, cx| {
                        editor.set_text(
                            original.replace(
                                "Original layout label 0",
                                "A source edit that needs a wider layout",
                            ),
                            window,
                            cx,
                        );
                    });
            })
            .expect("edit source");
        cx.run_until_parked();
        let (arrived, resume) = item.read_with(cx, |item, _| item.pause_next_save_for_test());
        let save = workspace
            .update(cx, |workspace, _, cx| {
                workspace.save_source_edit(cx).expect("save source")
            })
            .expect("start source save");
        arrived.await.expect("layout writer paused");
        let manifest = directory.path().join("fanta.json");
        let manifest_bytes = std::fs::read(&manifest).expect("manifest");
        std::fs::remove_file(&manifest).expect("simulate unavailable manifest");
        std::fs::create_dir(&manifest).expect("manifest path cannot be read as a file");
        resume.send(()).expect("resume writer");
        assert!(save.await.is_err(), "computed layout write fails visibly");
        cx.run_until_parked();
        let expected = item.read_with(cx, |item, _| {
            assert!(
                item.is_dirty(),
                "failed derived persistence must offer Save retry"
            );
            assert!(item.has_unpersisted_source_layout());
            assert!(
                !item.source_edit_locked(),
                "the validated source buffer is clean"
            );
            serde_json::to_value(&item.doc().expect("document").scene).expect("retained scene")
        });
        workspace
            .read_with(cx, |workspace, _| {
                assert!(workspace.error_message.is_some())
            })
            .expect("visible error");
        std::fs::remove_dir(&manifest).expect("remove simulated failure");
        std::fs::write(&manifest, manifest_bytes).expect("restore manifest");
        item.update(cx, |item, cx| {
            item.save(crate::document::SaveKind::Explicit, cx)
        })
        .await
        .expect("retry computed persistence");
        let (saved, _) =
            fanta_format::read_project_tree(directory.path()).expect("reopen retried save");
        assert_eq!(
            serde_json::to_value(&saved.scene).expect("saved scene"),
            expected
        );
        item.read_with(cx, |item, _| {
            assert!(!item.is_dirty());
            assert!(!item.has_unpersisted_source_layout());
        });
    }

    #[gpui::test]
    async fn loaded_layout_discarding_invalid_second_source_retains_pending_geometry(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let directory = tempfile::tempdir().expect("project");
        let (page, _, _) = write_layout_project(directory.path());
        let source_path = page_source_path(directory.path(), page);
        let original = std::fs::read_to_string(&source_path).expect("FNX");
        let (_project, item, workspace) = open_code_workspace(directory.path(), cx).await;
        workspace
            .update(cx, |workspace, window, cx| {
                workspace.selected_file = CodeWorkspaceFile::Fnx;
                workspace
                    .fnx_editor
                    .as_ref()
                    .expect("FNX")
                    .update(cx, |editor, cx| {
                        editor.set_text(
                            original.replace(
                                "Original layout label 0",
                                "A validated source label awaiting its geometry write",
                            ),
                            window,
                            cx,
                        );
                    });
                workspace
                    .json_editor
                    .as_ref()
                    .expect("JSON")
                    .update(cx, |editor, cx| {
                        editor.set_text("{ invalid header", window, cx)
                    });
            })
            .expect("edit both source drafts");
        cx.run_until_parked();
        let save = workspace
            .update(cx, |workspace, _, cx| {
                workspace.save_source_edit(cx).expect("source save")
            })
            .expect("start source save");
        assert!(save.await.is_err());
        cx.run_until_parked();
        item.read_with(cx, |item, _| assert!(item.has_unpersisted_source_layout()));
        workspace
            .update(cx, |workspace, _, cx| workspace.discard_source_edit(cx))
            .expect("discard invalid draft")
            .await
            .expect("discard second buffer only");
        cx.run_until_parked();
        let expected = item.read_with(cx, |item, _| {
            assert!(!item.source_edit_locked());
            assert!(
                item.is_dirty(),
                "computed geometry from the validated first source remains unsaved"
            );
            assert!(item.has_unpersisted_source_layout());
            serde_json::to_value(&item.doc().expect("document").scene).expect("retained geometry")
        });
        item.update(cx, |item, cx| {
            item.save(crate::document::SaveKind::Explicit, cx)
        })
        .await
        .expect("persist retained geometry");
        let (saved, _) = fanta_format::read_project_tree(directory.path()).expect("reopen");
        assert_eq!(
            serde_json::to_value(&saved.scene).expect("saved geometry"),
            expected
        );
    }

    #[gpui::test]
    async fn invalid_fnx_edit_keeps_last_saved_file_and_canvas(cx: &mut TestAppContext) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        let page = write_project(temporary.path());
        let source_path = page_source_path(temporary.path(), page);
        let original = std::fs::read_to_string(&source_path).expect("read FNX");
        let (_project, item, workspace) = open_code_workspace(temporary.path(), cx).await;

        workspace
            .update(cx, |workspace, window, cx| {
                workspace
                    .fnx_editor
                    .as_ref()
                    .expect("FNX editor")
                    .update(cx, |editor, cx| {
                        editor.set_text("<Frame name={ />".to_owned(), window, cx)
                    });
            })
            .expect("edit FNX");
        cx.run_until_parked();

        let save = workspace
            .update(cx, |workspace, _, cx| {
                workspace.save_source_edit(cx).expect("dirty FNX source")
            })
            .expect("start FNX save");
        assert!(save.await.is_err());
        cx.run_until_parked();

        assert_eq!(
            std::fs::read_to_string(&source_path).expect("read saved FNX"),
            original
        );
        item.read_with(cx, |item, _| {
            assert_eq!(
                item.doc()
                    .and_then(|doc| doc.scene.get(page))
                    .map(|node| node.name.as_str()),
                Some("Original")
            );
            assert!(item.source_edit_locked());
        });
        workspace
            .read_with(cx, |workspace, cx| {
                assert!(workspace.source_is_dirty(cx));
                assert!(!workspace.source_save_in_progress);
                assert_eq!(
                    workspace
                        .fnx_source_buffer
                        .as_ref()
                        .expect("FNX buffer")
                        .read(cx)
                        .capability(),
                    Capability::ReadWrite
                );
            })
            .expect("read code workspace");
    }

    #[gpui::test]
    async fn json_source_save_keeps_invalid_drafts_out_of_the_project(cx: &mut TestAppContext) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        let page = write_project(temporary.path());
        let source_path = page_json_path(temporary.path(), page);
        let original = std::fs::read_to_string(&source_path).expect("JSON source");
        let (_project, item, workspace) = open_code_workspace(temporary.path(), cx).await;

        workspace
            .update(cx, |workspace, window, cx| {
                workspace
                    .json_editor
                    .as_ref()
                    .expect("JSON editor")
                    .update(cx, |editor, cx| {
                        editor.set_text("{\"order\":".to_owned(), window, cx)
                    });
            })
            .expect("edit JSON draft");
        cx.run_until_parked();
        assert!(item.read_with(cx, |item, _| item.source_edit_locked()));
        let save = workspace
            .update(cx, |workspace, _, cx| {
                workspace.save_source_edit(cx).expect("dirty JSON source")
            })
            .expect("start JSON save");
        assert!(save.await.is_err());
        cx.run_until_parked();
        workspace
            .read_with(cx, |workspace, _| {
                assert!(
                    workspace
                        .error_message
                        .as_deref()
                        .is_some_and(|error| error.contains("Could not save JSON"))
                );
            })
            .expect("JSON validation error stays visible");
        assert_eq!(
            std::fs::read_to_string(&source_path).expect("JSON"),
            original
        );
        item.read_with(cx, |item, _| {
            assert_eq!(item.doc().and_then(|doc| doc.active_page()), Some(page));
            assert!(item.source_edit_locked());
        });

        let mut corrected: serde_json::Value = serde_json::from_str(&original).expect("JSON");
        corrected["order"] = serde_json::json!(9);
        let corrected = corrected.to_string();
        workspace
            .update(cx, |workspace, window, cx| {
                workspace
                    .json_editor
                    .as_ref()
                    .expect("JSON editor")
                    .update(cx, |editor, cx| {
                        editor.set_text(corrected.clone(), window, cx)
                    });
            })
            .expect("correct JSON");
        cx.run_until_parked();
        workspace
            .update(cx, |workspace, _, cx| {
                workspace
                    .save_source_edit(cx)
                    .expect("corrected JSON source")
            })
            .expect("save JSON")
            .await
            .expect("validated JSON save");
        cx.run_until_parked();
        assert_eq!(
            std::fs::read_to_string(source_path).expect("JSON"),
            corrected
        );
        item.read_with(cx, |item, _| {
            assert_eq!(item.doc().and_then(|doc| doc.active_page()), Some(page));
            assert!(!item.source_edit_locked());
            assert!(!item.has_conflict());
        });
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

    /// An external agent edit still reaches the canvas when no local source
    /// edit is pending.
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

    /// "Your design is text" only lands if the text follows the canvas:
    /// selecting a node on the canvas must move the FNX pane's caret to that
    /// node's opening tag. The mapping runs through the `.ids.json` sidecar,
    /// whose entries are in the same pre-order as the tags.
    #[gpui::test]
    async fn selecting_one_node_moves_the_fnx_caret_to_its_opening_tag(cx: &mut TestAppContext) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        let (page, second) = write_project_with_children(temporary.path());
        let source_path = page_source_path(temporary.path(), page);
        let (_project, item, workspace) = open_code_workspace(temporary.path(), cx).await;
        cx.run_until_parked();

        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.selection.select_only(second);
                ((), crate::document::DocChange::Selection)
            });
        });
        cx.run_until_parked();

        assert_eq!(
            fnx_selection_row(&workspace, cx),
            source_row_of(&source_path, "Second child"),
        );
    }

    /// Without the sidecar there is nothing to count against, so the sync
    /// falls back to the node's `name` attribute rather than giving up.
    #[gpui::test]
    async fn the_caret_still_follows_the_selection_without_a_sidecar(cx: &mut TestAppContext) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        let (page, second) = write_project_with_children(temporary.path());
        let source_path = page_source_path(temporary.path(), page);
        let (_project, item, workspace) = open_code_workspace(temporary.path(), cx).await;
        cx.run_until_parked();
        // Removed only after the load: a project with no sidecar cannot be
        // read at all, so this is the "sidecar went stale under us" case.
        std::fs::remove_file(source_path.with_file_name("page.ids.json"))
            .expect("remove the id sidecar");

        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.selection.select_only(second);
                ((), crate::document::DocChange::Selection)
            });
        });
        cx.run_until_parked();

        assert_eq!(
            fnx_selection_row(&workspace, cx),
            source_row_of(&source_path, "Second child"),
        );
    }

    /// The sidecar path is the primary mapping, so pin it directly — the
    /// name-attribute fallback would otherwise hide a broken one. Note the id
    /// form: the sidecar stores the bare ULID the document JSON uses, NOT
    /// `NodeId`'s `Display`, which prefixes `n_`.
    #[test]
    fn the_id_sidecar_maps_a_node_onto_its_opening_tag() {
        let temporary = tempfile::tempdir().expect("temporary project");
        let (page, second) = write_project_with_children(temporary.path());
        let source_path = page_source_path(temporary.path(), page);
        let source = std::fs::read_to_string(&source_path).expect("read FNX source");
        let tags = opening_tags(&source);
        assert_eq!(tags.len(), 3, "the page root plus its two children");
        assert_eq!(
            sidecar_offset(&source_path, page, &tags),
            Some(tags[0].offset),
            "sidecar entry 0 is the root, which is opening tag #1"
        );
        assert_eq!(
            sidecar_offset(&source_path, second, &tags),
            Some(tags[2].offset)
        );
    }

    #[test]
    fn an_opening_tag_scan_ignores_tags_written_inside_attribute_text() {
        let source = concat!(
            "export default function Home() {\n",
            "  return (\n",
            "    <Frame name=\"Home\">\n",
            "      <Text name=\"Copy\" characters=\"press <Rect> to draw\" />\n",
            "      <Rect name=\"Box\" width={10} height={10} />\n",
            "    </Frame>\n",
            "  );\n",
            "}\n",
        );
        let tags = opening_tags(source);
        let names: Vec<Option<&str>> = tags.iter().map(|tag| tag.name.as_deref()).collect();
        assert_eq!(names, [Some("Home"), Some("Copy"), Some("Box")]);
        for tag in &tags {
            assert!(source[tag.offset..].starts_with('<'), "{}", tag.offset);
        }
    }
    fn bind_source_editor_keys(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let bindings = settings::KeymapFile::load_asset_allow_partial_failure(
                "keymaps/default-macos.json",
                cx,
            )
            .expect("default editor keymap");
            cx.bind_keys(bindings);
        });
    }

    #[gpui::test]
    async fn embedded_source_find_keeps_fnx_and_json_unchanged(cx: &mut TestAppContext) {
        init_test(cx);
        bind_source_editor_keys(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        write_project_with_children(temporary.path());
        let (_, item, window) = open_code_workspace(temporary.path(), cx).await;
        let workspace = window.entity(cx).expect("source workspace");
        let before_document = item.read_with(cx, |item, _| {
            serde_json::to_value(item.doc().expect("document")).expect("document snapshot")
        });
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        let cx = &mut visual;
        cx.simulate_resize(gpui::size(px(1100.0), px(700.0)));
        for (file, query, read_only) in [
            (CodeWorkspaceFile::Fnx, "Second child", false),
            (CodeWorkspaceFile::Json, "Home", false),
            (CodeWorkspaceFile::Fnx, "First child", true),
        ] {
            let (editor, before_text) = workspace.update_in(cx, |workspace, window, cx| {
                workspace.set_development_read_only(read_only, cx);
                workspace.select_file(file, window, cx);
                let editor = workspace.active_editor().expect("source editor").clone();
                let text = editor.read(cx).buffer().read(cx).snapshot(cx).text();
                (editor, text)
            });
            cx.run_until_parked();
            cx.simulate_keystrokes("cmd-f cmd-a");
            cx.simulate_input(query);
            cx.run_until_parked();
            assert_eq!(
                editor.read_with(cx, |editor, cx| editor
                    .buffer()
                    .read(cx)
                    .snapshot(cx)
                    .text()),
                before_text,
                "Find input must never become an authored source edit"
            );
            assert!(cx.debug_bounds("fanta-source-search").is_some());
            workspace.read_with(cx, |workspace, cx| {
                let search = workspace.search_bar.read(cx);
                assert_eq!(search.query(cx), query);
                assert!(
                    search.has_active_match(),
                    "query must match the active source"
                );
            });
            workspace.read_with(cx, |workspace, cx| assert!(!workspace.source_is_dirty(cx)));
            item.read_with(cx, |item, _| {
                assert!(!item.source_edit_locked());
                assert_eq!(
                    serde_json::to_value(item.doc().expect("document")).expect("document"),
                    before_document
                );
            });
            cx.simulate_keystrokes("escape");
            cx.run_until_parked();
            editor.update_in(cx, |editor, window, cx| {
                assert!(editor.focus_handle(cx).is_focused(window));
            });
        }
    }

    #[gpui::test]
    async fn embedded_source_find_retargets_a_new_page(cx: &mut TestAppContext) {
        init_test(cx);
        bind_source_editor_keys(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        let page = write_project(temporary.path());
        let (mut doc, assets) =
            fanta_format::read_project_tree(temporary.path()).expect("document");
        let mut other = CanvasNode::new(NodeData::Group(GroupNode::default()));
        other.name = "Other needle".to_owned();
        let other_page = other.id;
        doc.scene.insert(other).expect("other page");
        doc.add_page(other_page);
        fanta_format::write_project_tree(temporary.path(), &doc, &assets).expect("two pages");
        let (_, item, window) = open_code_workspace(temporary.path(), cx).await;
        let workspace = window.entity(cx).expect("workspace");
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        let cx = &mut visual;
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.select_file(CodeWorkspaceFile::Fnx, window, cx);
        });
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("Original");
        cx.run_until_parked();
        assert!(cx.debug_bounds("fanta-source-search").is_some());
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.set_active_page(Some(other_page));
                ((), crate::document::DocChange::Selection)
            });
        });
        cx.run_until_parked();
        let editor = workspace.update_in(cx, |workspace, window, cx| {
            assert_eq!(workspace.requested_page, Some(other_page));
            assert_ne!(workspace.requested_page, Some(page));
            workspace.select_file(CodeWorkspaceFile::Fnx, window, cx);
            workspace.active_editor().expect("other editor").clone()
        });
        let text = editor.read_with(cx, |editor, cx| {
            editor.buffer().read(cx).snapshot(cx).text()
        });
        cx.simulate_keystrokes("cmd-f cmd-a");
        cx.simulate_input("Other needle");
        cx.run_until_parked();
        assert_eq!(
            editor.read_with(cx, |editor, cx| editor
                .buffer()
                .read(cx)
                .snapshot(cx)
                .text()),
            text
        );
        workspace.read_with(cx, |workspace, cx| {
            assert!(!workspace.source_is_dirty(cx));
            let search = workspace.search_bar.read(cx);
            assert_eq!(search.query(cx), "Other needle");
            assert!(
                search.has_active_match(),
                "Find must target the newly loaded page"
            );
        });
    }

    #[gpui::test]
    async fn source_save_preserves_authored_undo_without_geometry(cx: &mut TestAppContext) {
        init_test(cx);
        bind_source_editor_keys(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        let (page, _) = write_project_with_children(temporary.path());
        let path = page_source_path(temporary.path(), page);
        let original = std::fs::read_to_string(&path).expect("FNX");
        let changed = original.replace("Second child", "Renamed child");
        let (_, item, window) = open_code_workspace(temporary.path(), cx).await;
        let workspace = window.entity(cx).expect("workspace");
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        let cx = &mut visual;
        let editor = workspace.update_in(cx, |workspace, window, cx| {
            workspace.select_file(CodeWorkspaceFile::Fnx, window, cx);
            workspace.active_editor().expect("FNX editor").clone()
        });
        cx.simulate_keystrokes("cmd-a");
        cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string(changed.clone())));
        cx.simulate_keystrokes("cmd-v");
        cx.run_until_parked();
        let save = workspace.update(cx, |workspace, cx| {
            workspace.save_source_edit(cx).expect("save")
        });
        save.await.expect("save source");
        cx.run_until_parked();
        assert_eq!(std::fs::read_to_string(&path).expect("saved FNX"), changed);
        item.update(cx, |item, cx| {
            item.save(crate::document::SaveKind::Explicit, cx)
        })
        .await
        .expect("repeat Save with no authored changes");
        cx.run_until_parked();
        workspace.update_in(cx, |workspace, window, cx| {
            assert_eq!(workspace.active_editor(), Some(&editor));
            editor.focus_handle(cx).focus(window, cx);
        });
        cx.simulate_keystrokes("cmd-z");
        cx.run_until_parked();
        assert_eq!(
            editor.read_with(cx, |editor, cx| editor
                .buffer()
                .read(cx)
                .snapshot(cx)
                .text()),
            original
        );
        assert!(workspace.read_with(cx, |workspace, cx| workspace.source_is_dirty(cx)));
        let save = workspace.update(cx, |workspace, cx| {
            workspace.save_source_edit(cx).expect("save Undo")
        });
        save.await.expect("persist Undo");
        cx.run_until_parked();
        assert_eq!(
            std::fs::read_to_string(&path).expect("restored FNX"),
            original
        );
        assert!(!item.read_with(cx, |item, _| item.is_dirty()));
        cx.simulate_keystrokes("cmd-shift-z");
        cx.run_until_parked();
        assert_eq!(
            editor.read_with(cx, |editor, cx| editor
                .buffer()
                .read(cx)
                .snapshot(cx)
                .text()),
            changed
        );
    }

    fn write_spacing_project(root: &Path) -> (NodeId, NodeId) {
        let page = write_project(root);
        let (mut document, assets) = fanta_format::read_project_tree(root).expect("document");
        let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([120.0, 40.0]),
            auto_layout: Some(fanta_doc::AutoLayout {
                spacing: 8.0,
                primary_sizing: fanta_doc::AxisSizing::Fixed,
                counter_sizing: fanta_doc::AxisSizing::Fixed,
                ..Default::default()
            }),
            ..Default::default()
        }));
        frame.parent = Some(page);
        let parent = frame.id;
        document.scene.insert(frame).expect("frame");
        let mut second = None;
        for index in 0..2 {
            let mut child = CanvasNode::new(NodeData::Vector(fanta_doc::VectorNode::rect_solid(
                0.0,
                0.0,
                20.0,
                20.0,
                fanta_doc::Color::BLACK,
            )));
            child.parent = Some(parent);
            child.index = if index == 0 {
                IndexKey::FIRST
            } else {
                IndexKey::after(IndexKey::FIRST)
            };
            child.transform = fanta_doc::Transform2D::translation(f64::from(index) * 28.0, 0.0);
            second = Some(child.id);
            document.scene.insert(child).expect("child");
        }
        fanta_format::write_project_tree(root, &document, &assets).expect("spacing fixture");
        (page, second.expect("two children"))
    }

    #[gpui::test]
    async fn source_save_computed_geometry_undoes_with_the_authored_edit(cx: &mut TestAppContext) {
        init_test(cx);
        bind_source_editor_keys(cx);
        let temporary = tempfile::tempdir().expect("temporary project");
        let (page, child) = write_spacing_project(temporary.path());
        let path = page_source_path(temporary.path(), page);
        let original = std::fs::read_to_string(&path).expect("FNX");
        let changed = original.replacen("\"spacing\": 8.0", "\"spacing\": 16.0", 1);
        assert_ne!(changed, original, "fixture spacing encoding");
        let (baseline, baseline_assets) =
            fanta_format::read_project_tree(temporary.path()).expect("baseline");
        let (_, _, window) = open_code_workspace(temporary.path(), cx).await;
        let workspace = window.entity(cx).expect("workspace");
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        let cx = &mut visual;
        let editor = workspace.update_in(cx, |workspace, window, cx| {
            workspace.select_file(CodeWorkspaceFile::Fnx, window, cx);
            workspace.active_editor().expect("editor").clone()
        });
        cx.simulate_keystrokes("cmd-a");
        cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string(changed.clone())));
        cx.simulate_keystrokes("cmd-v");
        cx.run_until_parked();
        let save = workspace.update(cx, |workspace, cx| {
            workspace.save_source_edit(cx).expect("save")
        });
        save.await.expect("save layout");
        cx.run_until_parked();
        let persisted = std::fs::read_to_string(&path).expect("computed FNX");
        let (saved, _) = fanta_format::read_project_tree(temporary.path()).expect("saved");
        assert_eq!(
            saved
                .scene
                .get(child)
                .expect("child")
                .transform
                .0
                .translation
                .x,
            36.0
        );
        assert_ne!(persisted, changed);
        editor.update_in(cx, |editor, window, cx| {
            editor.focus_handle(cx).focus(window, cx)
        });
        cx.simulate_keystrokes("cmd-z");
        cx.run_until_parked();
        assert_eq!(
            editor.read_with(cx, |editor, cx| editor
                .buffer()
                .read(cx)
                .snapshot(cx)
                .text()),
            original,
            "one Undo must restore the authored spacing and derived position together"
        );
        let save = workspace.update(cx, |workspace, cx| {
            workspace.save_source_edit(cx).expect("save Undo")
        });
        save.await.expect("persist layout Undo");
        cx.run_until_parked();
        let (restored, assets) =
            fanta_format::read_project_tree(temporary.path()).expect("restored");
        assert_eq!(
            serde_json::to_value(&restored.scene).expect("scene"),
            serde_json::to_value(&baseline.scene).expect("baseline")
        );
        assert_eq!(assets, baseline_assets);
        cx.simulate_keystrokes("cmd-shift-z");
        cx.run_until_parked();
        assert_eq!(
            editor.read_with(cx, |editor, cx| editor
                .buffer()
                .read(cx)
                .snapshot(cx)
                .text()),
            persisted
        );
        let save = workspace.update(cx, |workspace, cx| {
            workspace.save_source_edit(cx).expect("save Redo")
        });
        save.await.expect("persist layout Redo");
        cx.run_until_parked();
        let (redone, _) = fanta_format::read_project_tree(temporary.path()).expect("redone");
        assert_eq!(
            serde_json::to_value(&redone.scene).expect("scene"),
            serde_json::to_value(&saved.scene).expect("saved")
        );
    }

    #[gpui::test]
    async fn source_discard_button_discards_only_the_selected_draft(cx: &mut TestAppContext) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("project");
        let page = write_project(temporary.path());
        let path = page_source_path(temporary.path(), page);
        let fnx = std::fs::read_to_string(&path).expect("FNX");
        let json = std::fs::read_to_string(path.with_file_name("page.json")).expect("JSON");
        let (_, item, window) = open_code_workspace(temporary.path(), cx).await;
        let workspace = window.entity(cx).expect("workspace");
        let before = item.read_with(cx, |item, _| {
            serde_json::to_value(item.doc().expect("doc")).expect("document")
        });
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        let cx = &mut visual;
        cx.simulate_resize(gpui::size(px(1600.0), px(800.0)));
        workspace.update_in(cx, |workspace, window, cx| {
            workspace
                .fnx_editor
                .as_ref()
                .expect("FNX")
                .update(cx, |editor, cx| editor.set_text("<invalid FNX", window, cx));
            workspace
                .json_editor
                .as_ref()
                .expect("JSON")
                .update(cx, |editor, cx| {
                    editor.set_text("{ invalid JSON", window, cx)
                });
            workspace.select_file(CodeWorkspaceFile::Json, window, cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear());
        let button = cx
            .debug_bounds("fanta-source-discard-target")
            .expect("discard selected draft control");
        cx.simulate_click(button.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        workspace.read_with(cx, |workspace, cx| {
            assert_eq!(
                workspace
                    .json_source_buffer
                    .as_ref()
                    .expect("JSON")
                    .read(cx)
                    .text(),
                json
            );
            assert_eq!(
                workspace
                    .fnx_source_buffer
                    .as_ref()
                    .expect("FNX")
                    .read(cx)
                    .text(),
                "<invalid FNX"
            );
            assert!(workspace.source_is_dirty(cx));
            assert!(workspace.error_message.is_none());
        });
        item.read_with(cx, |item, _| {
            assert!(item.source_edit_locked());
            assert_eq!(
                serde_json::to_value(item.doc().expect("doc")).expect("doc"),
                before
            );
        });
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.select_file(CodeWorkspaceFile::Fnx, window, cx)
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear());
        let button = cx
            .debug_bounds("fanta-source-discard-target")
            .expect("FNX discard");
        cx.simulate_click(button.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        workspace.read_with(cx, |workspace, cx| {
            assert!(!workspace.source_is_dirty(cx));
            assert_eq!(
                workspace
                    .fnx_source_buffer
                    .as_ref()
                    .expect("FNX")
                    .read(cx)
                    .text(),
                fnx
            );
        });
        item.read_with(cx, |item, _| assert!(!item.source_edit_locked()));
        assert_eq!(std::fs::read_to_string(path).expect("unchanged FNX"), fnx);
    }

    #[gpui::test]
    async fn source_discard_button_retains_validated_layout_after_invalid_json(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("project");
        let (page, child) = write_spacing_project(temporary.path());
        let path = page_source_path(temporary.path(), page);
        let original = std::fs::read_to_string(&path).expect("FNX");
        let (_, baseline_assets) =
            fanta_format::read_project_tree(temporary.path()).expect("baseline");
        let (_, item, window) = open_code_workspace(temporary.path(), cx).await;
        let workspace = window.entity(cx).expect("workspace");
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        let cx = &mut visual;
        cx.simulate_resize(gpui::size(px(1600.0), px(800.0)));
        workspace.update_in(cx, |workspace, window, cx| {
            workspace
                .fnx_editor
                .as_ref()
                .expect("FNX")
                .update(cx, |editor, cx| {
                    editor.set_text(
                        original.replacen("\"spacing\": 8.0", "\"spacing\": 16.0", 1),
                        window,
                        cx,
                    );
                });
            workspace
                .json_editor
                .as_ref()
                .expect("JSON")
                .update(cx, |editor, cx| {
                    editor.set_text("{ invalid JSON", window, cx);
                });
            workspace.select_file(CodeWorkspaceFile::Fnx, window, cx);
        });
        cx.run_until_parked();
        let save = workspace.update(cx, |workspace, cx| {
            workspace.save_source_edit(cx).expect("save both")
        });
        assert!(
            save.await.is_err(),
            "invalid second source must still reject Save"
        );
        cx.run_until_parked();
        let (persisted_before, _) =
            fanta_format::read_project_tree(temporary.path()).expect("validated source");
        assert_eq!(
            persisted_before
                .scene
                .get(child)
                .expect("child")
                .transform
                .0
                .translation
                .x,
            28.0
        );
        let expected = item.read_with(cx, |item, _| {
            assert!(item.source_edit_locked());
            assert!(item.has_unpersisted_source_layout());
            let doc = item.doc().expect("doc");
            assert_eq!(
                doc.scene
                    .get(child)
                    .expect("computed child")
                    .transform
                    .0
                    .translation
                    .x,
                36.0
            );
            serde_json::to_value(doc).expect("computed document")
        });
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.select_file(CodeWorkspaceFile::Json, window, cx)
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear());
        let button = cx
            .debug_bounds("fanta-source-discard-target")
            .expect("discard invalid JSON");
        cx.simulate_click(button.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        item.read_with(cx, |item, _| {
            assert!(!item.source_edit_locked());
            assert!(item.is_dirty());
            assert!(item.has_unpersisted_source_layout());
            assert_eq!(
                serde_json::to_value(item.doc().expect("doc")).expect("doc"),
                expected
            );
        });
        item.update(cx, |item, cx| {
            item.save(crate::document::SaveKind::Explicit, cx)
        })
        .await
        .expect("save retained layout");
        cx.run_until_parked();
        let (saved, assets) = fanta_format::read_project_tree(temporary.path()).expect("reopen");
        let mut actual = serde_json::to_value(&saved).expect("saved document");
        let mut expected_saved = persisted_before;
        expected_saved
            .scene
            .get_mut(child)
            .expect("expected child")
            .transform = fanta_doc::Transform2D::translation(36.0, 0.0);
        let mut expected = serde_json::to_value(&expected_saved).expect("expected saved document");
        actual["metadata"]
            .as_object_mut()
            .expect("metadata")
            .remove("modified_at");
        expected["metadata"]
            .as_object_mut()
            .expect("metadata")
            .remove("modified_at");
        assert_eq!(actual, expected, "only persisted timestamp may change");
        assert_eq!(assets, baseline_assets);
        item.read_with(cx, |item, _| {
            assert!(!item.source_edit_locked());
            assert!(!item.is_dirty());
            assert!(!item.has_unpersisted_source_layout());
        });
    }
    async fn mounted_source_recovery_fixture<'a>(
        root: &Path,
        cx: &'a mut TestAppContext,
    ) -> (
        Entity<crate::view::FigView>,
        Entity<FantaCodeWorkspace>,
        Entity<FigItem>,
        &'a mut gpui::VisualTestContext,
    ) {
        let project = open_test_project(root, cx).await;
        let worktree_id = project.read_with(cx, |project, cx| {
            project
                .worktrees(cx)
                .next()
                .expect("worktree")
                .read(cx)
                .id()
        });
        let item = cx
            .update(|cx| {
                FigItem::try_open(
                    &project,
                    &ProjectPath {
                        worktree_id,
                        path: util::rel_path::rel_path("fanta.json").into(),
                    },
                    cx,
                )
            })
            .expect("project item")
            .await
            .expect("open item");
        cx.run_until_parked();
        cx.update(|cx| {
            #[cfg(feature = "fanta-gpui-ui")]
            {
                gpui_component::init(cx);
                fanta_gpui::init(cx);
                crate::theme_bridge::init(cx);
            }
            cx.bind_keys([gpui::KeyBinding::new(
                "cmd-s",
                workspace::Save { save_intent: None },
                None,
            )]);
        });
        let (multi_workspace, cx) = cx.add_window_view(|window, cx| {
            workspace::MultiWorkspace::test_new(project.clone(), window, cx)
        });
        let workspace = multi_workspace.read_with(cx, |workspace, _| workspace.workspace().clone());
        let view = cx.update(|window, cx| {
            let view = cx.new(|cx| crate::view::FigView::new(item.clone(), project, window, cx));
            workspace.update(cx, |workspace, cx| {
                workspace.add_item_to_active_pane(Box::new(view.clone()), None, true, window, cx);
            });
            window.activate_window();
            view
        });
        cx.simulate_resize(gpui::size(px(1600.0), px(1000.0)));
        cx.run_until_parked();
        item.read_with(cx, |item, _| {
            assert!(item.has_ready_document(), "ready document")
        });
        workspace.read_with(cx, |workspace, cx| {
            assert_eq!(
                workspace.active_item(cx).expect("active item").item_id(),
                view.entity_id()
            )
        });
        let code = view.read_with(cx, |view, _| view.code_workspace_for_test());
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
        let button = cx
            .debug_bounds("fanta-collapsible-tab-fanta-editor-workspace-2")
            .unwrap_or_else(|| {
                panic!(
                    "Code tab; canvas bounds {:?}, inspector {:?}",
                    cx.debug_bounds("fig-container"),
                    cx.debug_bounds("fanta-inspector-sidebar")
                )
            });
        cx.simulate_click(button.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        code.update_in(cx, |code, window, cx| {
            code.select_file(CodeWorkspaceFile::Fnx, window, cx)
        });
        (view, code, item, cx)
    }

    #[cfg(feature = "fanta-gpui-ui")]
    #[gpui::test]
    async fn source_locked_inspector_keeps_geometry_and_layout_readable(cx: &mut TestAppContext) {
        use fanta_gpui::design::{
            DesignPanelAction, DesignPanelProperty, DesignPanelTarget, DesignPanelValue,
        };

        init_test(cx);
        let temporary = tempfile::tempdir().expect("project");
        let (page, child) = write_spacing_project(temporary.path());
        let (mut document, assets) =
            fanta_format::read_project_tree(temporary.path()).expect("fixture");
        let row = document
            .scene
            .get(child)
            .expect("child")
            .parent
            .expect("row");
        let frame = document.scene.get_mut(row).expect("frame");
        frame.name = "Accepted row".into();
        frame.transform = fanta_doc::Transform2D::translation(24.0, 32.0);
        fanta_format::write_project_tree(temporary.path(), &document, &assets).expect("fixture");
        let path = page_source_path(temporary.path(), page);
        let original = std::fs::read_to_string(&path).expect("FNX");
        let draft = original.replacen("Accepted row", "Unsaved draft row", 1);
        assert_ne!(draft, original, "the test must create a real source edit");

        let (view, code, item, cx) = mounted_source_recovery_fixture(temporary.path(), cx).await;
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.selection.replace_with([row]);
                ((), crate::document::DocChange::Selection)
            });
        });
        cx.run_until_parked();
        let panel = view.read_with(cx, |view, _| {
            view.gpui_design
                .as_ref()
                .expect("mounted inspector")
                .panel
                .clone()
        });
        panel.read_with(cx, |panel, _| {
            assert!(panel.inspection_context().permissions().can_edit());
            let node = panel.node();
            assert_eq!(
                (node.x, node.y, node.width, node.height),
                (24.0, 32.0, 120.0, 40.0)
            );
            assert_eq!(node.layout.as_ref().expect("layout").gap, 8.0);
        });
        let before = item.read_with(cx, |item, _| {
            serde_json::to_value(item.doc().expect("document")).expect("accepted document")
        });
        code.update_in(cx, |code, window, cx| {
            code.fnx_editor
                .as_ref()
                .expect("FNX editor")
                .update(cx, |editor, cx| {
                    editor.set_text(draft.clone(), window, cx);
                });
        });
        cx.run_until_parked();
        item.read_with(cx, |item, _| {
            assert!(item.source_edit_locked());
            assert!(!item.is_editable());
        });
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
        let canvas_tab = cx
            .debug_bounds("fanta-collapsible-tab-fanta-editor-workspace-0")
            .expect("Canvas tab");
        cx.simulate_click(canvas_tab.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
        panel.read_with(cx, |panel, _| {
            assert!(!panel.inspection_context().permissions().can_edit());
            let properties = panel
                .viewer_properties_view_data()
                .expect("a dirty source buffer must retain readable accepted properties");
            assert_eq!(
                properties.target,
                DesignPanelTarget::Nodes {
                    node_ids: vec![row.to_string().into()]
                }
            );
            for (section, property, expected) in [
                ("identity", "name", "Accepted row"),
                ("geometry", "x", "24"),
                ("geometry", "y", "32"),
                ("geometry", "width", "120"),
                ("geometry", "height", "40"),
                ("layout", "gap", "8"),
            ] {
                assert_eq!(
                    properties
                        .section(section)
                        .and_then(|section| section.row(property))
                        .expect("read-only row")
                        .displayed_value
                        .as_ref(),
                    expected
                );
            }
        });
        for selector in [
            "fig-gpui-design-viewer-row-geometry-x",
            "fig-gpui-design-viewer-row-layout-gap",
        ] {
            assert!(
                cx.debug_bounds(selector).is_some(),
                "{selector} must be rendered, not only stored"
            );
        }
        panel.update_in(cx, |_, _, cx| {
            cx.emit(DesignPanelAction::PropertyChangeRequested {
                node_id: row.to_string().into(),
                property: DesignPanelProperty::Gap,
                value: DesignPanelValue::Number(99.0),
            });
        });
        item.update(cx, |item, cx| {
            assert!(
                item.apply(
                    fanta_doc::Operation::SetName {
                        id: row,
                        old: "Accepted row".into(),
                        new: "Rejected canvas edit".into(),
                    },
                    cx
                )
                .is_err()
            );
        });
        cx.run_until_parked();
        item.read_with(cx, |item, _| {
            let document = item.doc().expect("document");
            assert_eq!(
                serde_json::to_value(document).expect("locked document"),
                before
            );
            assert_eq!(
                (document.history.undo_depth(), document.history.redo_depth()),
                (0, 0)
            );
        });
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.selection.replace_with([child]);
                ((), crate::document::DocChange::Selection)
            });
        });
        cx.run_until_parked();
        panel.read_with(cx, |panel, _| {
            let properties = panel
                .viewer_properties_view_data()
                .expect("child properties");
            assert_eq!(
                properties.target,
                DesignPanelTarget::Nodes {
                    node_ids: vec![child.to_string().into()]
                }
            );
            let geometry = properties.section("geometry").expect("child geometry");
            assert_eq!(geometry.row("x").expect("X").displayed_value.as_ref(), "52");
            assert_eq!(geometry.row("y").expect("Y").displayed_value.as_ref(), "32");
            assert!(
                properties.section("layout").is_none(),
                "a child must not inherit its parent's Gap"
            );
        });
        let child_selected = item.read_with(cx, |item, _| {
            serde_json::to_value(item.doc().expect("document")).expect("child selection")
        });
        for selection in [vec![], vec![row, child]] {
            item.update(cx, |item, cx| {
                item.with_document(cx, |document| {
                    document.doc.selection.replace_with(selection);
                    ((), crate::document::DocChange::Selection)
                });
            });
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear();
            });
            assert!(
                cx.debug_bounds("fanta-source-locked-inspector-selection")
                    .is_some(),
                "page and mixed selections need a readable selection notice"
            );
            assert!(
                cx.debug_bounds("fig-gpui-design-viewer-row-geometry-x")
                    .is_none()
            );
            panel.read_with(cx, |panel, _| {
                assert!(!panel.inspection_context().permissions().can_edit());
                assert!(panel.viewer_properties_view_data().is_none());
            });
            item.read_with(cx, |item, _| {
                assert!(item.source_edit_locked() && !item.is_editable())
            });
        }
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document.doc.selection.replace_with([child]);
                ((), crate::document::DocChange::Selection)
            });
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
        assert!(
            cx.debug_bounds("fanta-source-locked-inspector-selection")
                .is_none()
        );
        assert!(
            cx.debug_bounds("fig-gpui-design-viewer-row-geometry-x")
                .is_some()
        );
        item.read_with(cx, |item, _| {
            assert_eq!(
                serde_json::to_value(item.doc().expect("document")).expect("restored selection"),
                child_selected
            );
        });
        code.read_with(cx, |code, cx| {
            assert_eq!(
                code.fnx_source_buffer
                    .as_ref()
                    .expect("source")
                    .read(cx)
                    .text(),
                draft
            );
        });
        assert_eq!(std::fs::read_to_string(&path).expect("disk FNX"), original);
        let (disk, disk_assets) =
            fanta_format::read_project_tree(temporary.path()).expect("disk document");
        assert_eq!(
            serde_json::to_value(&disk).expect("disk"),
            serde_json::to_value(&document).expect("fixture")
        );
        assert_eq!(disk_assets, assets);

        code.update(cx, |code, cx| code.discard_source_edit(cx))
            .await
            .expect("discard the owned draft");
        cx.run_until_parked();
        panel.read_with(cx, |panel, _| {
            assert!(panel.inspection_context().permissions().can_edit());
            assert!(panel.viewer_properties_view_data().is_none());
            assert_eq!((panel.node().x, panel.node().y), (52.0, 32.0));
        });
        item.read_with(cx, |item, _| {
            assert!(!item.source_edit_locked());
            assert!(item.is_editable());
            let document = item.doc().expect("document");
            assert_eq!(
                (document.history.undo_depth(), document.history.redo_depth()),
                (0, 0)
            );
        });
    }

    #[gpui::test]
    async fn source_recovery_canvas_tab_restores_save_keyboard_route(cx: &mut TestAppContext) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("project");
        let (page, child) = write_spacing_project(temporary.path());
        let (view, code, item, cx) = mounted_source_recovery_fixture(temporary.path(), cx).await;
        code.update_in(cx, |code, window, cx| {
            assert!(
                code.active_editor()
                    .expect("editor")
                    .focus_handle(cx)
                    .is_focused(window)
            );
        });
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                document
                    .doc
                    .apply(fanta_doc::Operation::SetName {
                        id: child,
                        old: document.doc.scene.get(child).expect("child").name.clone(),
                        new: "Saved from Canvas".into(),
                    })
                    .expect("rename");
                ((), crate::document::DocChange::Content)
            });
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
        let button = cx
            .debug_bounds("fanta-collapsible-tab-fanta-editor-workspace-0")
            .expect("Canvas tab");
        cx.simulate_click(button.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        view.update_in(cx, |view, window, cx| {
            assert!(
                view.focus_handle(cx).is_focused(window),
                "Canvas must own focus after leaving source"
            );
        });
        cx.simulate_keystrokes("cmd-s");
        cx.run_until_parked();
        assert!(!cx.has_pending_prompt());
        let (saved, _) = fanta_format::read_project_tree(temporary.path()).expect("saved document");
        assert_eq!(
            saved.scene.get(child).expect("saved child").name,
            "Saved from Canvas"
        );
        assert_eq!(saved.pages(), &[page]);
        item.read_with(cx, |item, _| assert!(!item.is_dirty()));
    }

    #[gpui::test]
    async fn source_recovery_discard_then_workspace_save_ignores_delayed_owned_events(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let temporary = tempfile::tempdir().expect("project");
        let (page, child) = write_spacing_project(temporary.path());
        let path = page_source_path(temporary.path(), page);
        let original = std::fs::read_to_string(&path).expect("FNX");
        let (view, code, item, cx) = mounted_source_recovery_fixture(temporary.path(), cx).await;
        code.update_in(cx, |code, window, cx| {
            code.fnx_editor
                .as_ref()
                .expect("FNX")
                .update(cx, |editor, cx| {
                    editor.set_text(
                        original.replacen("\"spacing\": 8.0", "\"spacing\": 16.0", 1),
                        window,
                        cx,
                    );
                });
            code.json_editor
                .as_ref()
                .expect("JSON")
                .update(cx, |editor, cx| {
                    editor.set_text("{ invalid JSON", window, cx);
                });
            code.select_file(CodeWorkspaceFile::Fnx, window, cx);
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("cmd-s");
        cx.run_until_parked();
        assert!(code.read_with(cx, |code, _| code.validation_error().is_some()));
        if cx.has_pending_prompt() {
            cx.simulate_prompt_answer("OK");
            cx.run_until_parked();
        }
        let (partial, assets) =
            fanta_format::read_project_tree(temporary.path()).expect("partial source");
        assert_eq!(
            partial
                .scene
                .get(child)
                .expect("disk child")
                .transform
                .0
                .translation
                .x,
            28.0
        );
        item.update(cx, |item, cx| {
            item.queue_watcher_paths_for_test(
                [path.clone(), path.with_file_name("page.ids.json")],
                cx,
            )
        });
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(350));
        cx.run_until_parked();
        item.read_with(cx, |item, _| {
            assert!(item.source_edit_locked());
            assert!(
                !item.has_conflict(),
                "delayed notifications for our validated FNX must not become external conflicts"
            );
            assert_eq!(
                item.doc()
                    .expect("doc")
                    .scene
                    .get(child)
                    .expect("computed child")
                    .transform
                    .0
                    .translation
                    .x,
                36.0
            );
        });
        code.update_in(cx, |code, window, cx| {
            code.select_file(CodeWorkspaceFile::Json, window, cx)
        });
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
        let button = cx
            .debug_bounds("fanta-source-discard-target")
            .expect("Discard JSON");
        cx.simulate_click(button.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(item.read_with(cx, |item, _| item.is_dirty() && !item.source_edit_locked()));
        item.update(cx, |item, cx| {
            item.queue_watcher_paths_for_test(
                [path.clone(), path.with_file_name("page.ids.json")],
                cx,
            )
        });
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(350));
        cx.run_until_parked();
        item.read_with(cx, |item, _| {
            assert!(!item.has_conflict());
            assert!(item.is_dirty() && item.has_unpersisted_source_layout());
            assert_eq!(
                item.doc()
                    .expect("doc")
                    .scene
                    .get(child)
                    .expect("child")
                    .transform
                    .0
                    .translation
                    .x,
                36.0
            );
        });
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
        let button = cx
            .debug_bounds("fanta-collapsible-tab-fanta-editor-workspace-0")
            .expect("Canvas tab");
        cx.simulate_click(button.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        cx.simulate_keystrokes("cmd-s");
        cx.run_until_parked();
        assert!(
            !cx.has_pending_prompt(),
            "owned source writes must not show an Overwrite prompt"
        );
        let (saved, saved_assets) =
            fanta_format::read_project_tree(temporary.path()).expect("saved computed layout");
        let mut actual = serde_json::to_value(&saved).expect("saved");
        let mut expected_doc = partial;
        expected_doc.scene.get_mut(child).expect("child").transform =
            fanta_doc::Transform2D::translation(36.0, 0.0);
        let mut expected = serde_json::to_value(expected_doc).expect("expected");
        for value in [&mut actual, &mut expected] {
            value["metadata"]
                .as_object_mut()
                .expect("metadata")
                .remove("modified_at");
        }
        assert_eq!(
            actual, expected,
            "only the pending child position and timestamp may change"
        );
        assert_eq!(saved_assets, assets);
        item.read_with(cx, |item, _| {
            assert!(!item.is_dirty() && !item.has_conflict())
        });
        view.update_in(cx, |view, window, cx| {
            assert!(view.focus_handle(cx).is_focused(window))
        });
    }

    #[gpui::test]
    async fn source_recovery_external_writes_still_prompt_and_cancel_preserves_disk(
        test_cx: &mut TestAppContext,
    ) {
        init_test(test_cx);
        let mut directories = Vec::new();
        for external_target in ["fnx", "sidecar", "metadata"] {
            let temporary = tempfile::tempdir().expect("project");
            let (page, child) = write_spacing_project(temporary.path());
            let path = page_source_path(temporary.path(), page);
            let original = std::fs::read_to_string(&path).expect("FNX");
            let (_view, code, item, cx) =
                mounted_source_recovery_fixture(temporary.path(), test_cx).await;
            code.update_in(cx, |code, window, cx| {
                code.fnx_editor
                    .as_ref()
                    .expect("FNX")
                    .update(cx, |editor, cx| {
                        editor.set_text(
                            original.replacen("\"spacing\": 8.0", "\"spacing\": 16.0", 1),
                            window,
                            cx,
                        );
                    });
                code.json_editor
                    .as_ref()
                    .expect("JSON")
                    .update(cx, |editor, cx| {
                        editor.set_text("{ invalid JSON", window, cx);
                    });
                code.select_file(CodeWorkspaceFile::Fnx, window, cx);
            });
            cx.run_until_parked();
            cx.simulate_keystrokes("cmd-s");
            cx.run_until_parked();
            assert!(code.read_with(cx, |code, _| code.validation_error().is_some()));
            if cx.has_pending_prompt() {
                cx.simulate_prompt_answer("OK");
                cx.run_until_parked();
            }
            let external_path = match external_target {
                "fnx" => path.clone(),
                "sidecar" => path.with_file_name("page.ids.json"),
                _ => temporary.path().join("doc/metadata.json"),
            };
            let mut external_bytes = std::fs::read(&external_path).expect("external baseline");
            external_bytes.extend_from_slice(b"\n ");
            std::fs::write(&external_path, &external_bytes).expect("genuine external byte change");
            let (before_cancel, assets) =
                fanta_format::read_project_tree(temporary.path()).expect("external project");
            item.update(cx, |item, cx| {
                item.queue_watcher_paths_for_test([external_path.clone()], cx)
            });
            cx.run_until_parked();
            cx.executor().advance_clock(Duration::from_millis(350));
            cx.run_until_parked();
            item.read_with(cx, |item, _| {
                assert!(
                    item.has_conflict(),
                    "external {external_target} must not be classified as our write"
                );
                assert!(item.source_edit_locked());
            });
            code.update_in(cx, |code, window, cx| {
                code.select_file(CodeWorkspaceFile::Json, window, cx)
            });
            cx.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear();
            });
            let button = cx
                .debug_bounds("fanta-source-discard-target")
                .expect("Discard JSON");
            cx.simulate_click(button.center(), gpui::Modifiers::none());
            cx.run_until_parked();
            item.read_with(cx, |item, _| {
                assert!(item.has_conflict() && item.is_dirty() && !item.source_edit_locked());
                assert_eq!(
                    item.doc()
                        .expect("doc")
                        .scene
                        .get(child)
                        .expect("computed child")
                        .transform
                        .0
                        .translation
                        .x,
                    36.0
                );
            });
            cx.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear();
            });
            let button = cx
                .debug_bounds("fanta-collapsible-tab-fanta-editor-workspace-0")
                .expect("Canvas tab");
            cx.simulate_click(button.center(), gpui::Modifiers::none());
            cx.run_until_parked();
            cx.simulate_keystrokes("cmd-s");
            cx.run_until_parked();
            assert!(
                cx.has_pending_prompt(),
                "external {external_target} still requires explicit conflict resolution"
            );
            cx.simulate_prompt_answer("Cancel");
            cx.run_until_parked();
            assert_eq!(
                std::fs::read(&external_path).expect("preserved external file"),
                external_bytes
            );
            let (after_cancel, after_assets) =
                fanta_format::read_project_tree(temporary.path()).expect("project after cancel");
            assert_eq!(
                serde_json::to_value(after_cancel).expect("after"),
                serde_json::to_value(before_cancel).expect("before")
            );
            assert_eq!(after_assets, assets);
            directories.push(temporary);
        }
    }
}
