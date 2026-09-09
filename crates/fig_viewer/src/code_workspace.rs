use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use anyhow::Result;
use editor::{Editor, MultiBufferOffset, SelectionEffects, scroll::Autoscroll};
use fanta_doc::NodeId;
use gpui::{
    AnyElement, App, ClipboardItem, Context, Entity, FocusHandle, Focusable, IntoElement, Render,
    SharedString, Subscription, Task, Window, div, px,
};
use project::Project;
use serde::Deserialize;
use ui::Tooltip;
use ui::prelude::*;

use crate::document::{FigItem, FigItemEvent};

/// The alpha ships a code *viewer*: the canvas follows the file on disk, and
/// the file is authored by the agent or by an ordinary editor — never by this
/// pane. Stated once, where the panes are; kept short so the path header —
/// the thing that answers "which file is this?" — gets the room.
const READ_ONLY_STATUS: &str = "Read-only — the canvas follows this file.";

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
                    if matches!(event, FigItemEvent::SelectionChanged) {
                        this.sync_selection_to_source(window, cx);
                    }
                } else if matches!(event, FigItemEvent::Saved) {
                    // An autosave rewrote the source and, with it, the id
                    // sidecar the caret was derived from. The buffers are the
                    // project's own and reload themselves, so only the caret
                    // has to be re-derived — deliberately WITHOUT
                    // `refresh_from_item`, which would open editors from
                    // inside an item event and so demand a fully themed app
                    // on a path that is only reached by an autosave.
                    this.last_synced_selection = None;
                    this.sync_selection_to_source(window, cx);
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
            source_root: None,
            last_synced_selection: None,
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
        self.last_synced_selection = None;
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
                        // The buffer arrives long after the selection that
                        // should be revealed in it, so catch up once here.
                        this.last_synced_selection = None;
                        this.sync_selection_to_source(window, cx);
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
                    .children(self.render_path(cx))
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

    use editor::ToPoint as _;
    use fanta_doc::{CanvasNode, Doc, GroupNode, IndexKey, NodeData};
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

    /// The row of the opening tag carrying `name`, read straight from the file
    /// the pane is showing (the printer puts one element per line).
    fn source_row_of(source_path: &Path, name: &str) -> u32 {
        let source = std::fs::read_to_string(source_path).expect("read FNX source");
        let needle = format!("name=\"{name}\"");
        source
            .lines()
            .position(|line| line.contains(&needle))
            .expect("the child is named in the source") as u32
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
}
