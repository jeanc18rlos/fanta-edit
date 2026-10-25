//! The fig_viewer implementation of [`design_surface::DesignSurface`]: lets
//! the agent's native design tools read, edit, and screenshot the most
//! recently opened or focused canvas. Edits go through the same document
//! seam as user input (one history transaction per batch, rolled back on
//! failure), so agent work is undoable like any canvas gesture.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use design_surface::{
    DEFAULT_CHILD_LIMIT, DesignOp, DesignSurface, DesignSystemQuery, NodeDetail, NodeQuery,
    ScreenshotTarget,
};
use fanta_doc::{
    AssetId, Bounds, CanvasNode, ComponentId, Doc, ModeId, NodeId, ProjectAsset, ProjectAssetKind,
    VariableCollectionId, Viewport,
};
use fanta_headless::apply::{
    BatchEffect, BatchProgress, apply_batch, design_operation_is_presence,
    design_operation_is_variable, design_operation_node, parse_node_id,
};
#[cfg(test)]
use fanta_headless::scene::PAGE_LISTING_NODE_BUDGET;
use fanta_headless::scene::{
    COMPONENT_OVERVIEW_LIMIT, bounds_json, check_page_listing_budget, components_json,
    content_bounds, design_system_json, empty_space, paginated_node_summary, relative_source,
    union_bounds,
};
pub(crate) use fanta_headless::scene::{
    SUMMARY_LABEL_CHARS, node_summary, truncate_summary_string, world_bounds_json,
};
use fanta_render::AssetResolver;
use gpui::{App, AppContext as _, Entity, Global, Task, WeakEntity};
use serde_json::{Value, json};
use std::sync::Arc;
use util::ResultExt as _;

use crate::document::{
    DocChange, FigDocument, FigItem, FigItemEvent, FigPage, PreparedImage, SaveKind,
};

/// The most recently opened or focused canvas item, shared between the
/// registered provider and the [`FigView`](crate::FigView) instances that
/// update it.
struct ActiveDesignItem(
    Rc<RefCell<Option<WeakEntity<FigItem>>>>,
    Cell<Option<gpui::EntityId>>,
);

impl Global for ActiveDesignItem {}

pub(crate) fn init(cx: &mut App) {
    let active = Rc::new(RefCell::new(None));
    cx.set_global(ActiveDesignItem(active.clone(), Cell::new(None)));
    design_surface::register(Rc::new(FigDesignSurface { active }), cx);
}

/// Record `item` as the canvas the agent's design tools target. Called when a
/// [`FigView`](crate::FigView) is created or focused, so the tools follow the
/// user's attention.
pub(crate) fn set_active_item(item: WeakEntity<FigItem>, cx: &mut App) {
    if let Some(state) = cx.try_global::<ActiveDesignItem>() {
        let mut active = state.0.borrow_mut();
        let same_live_item = active
            .as_ref()
            .and_then(WeakEntity::upgrade)
            .zip(item.upgrade())
            .is_some_and(|(active, item)| active.entity_id() == item.entity_id());
        *active = Some(item);
        if !same_live_item {
            state.1.set(None);
        }
    }
}

pub(crate) fn set_active_item_for_view(
    item: WeakEntity<FigItem>,
    view: gpui::EntityId,
    cx: &mut App,
) {
    if let Some(state) = cx.try_global::<ActiveDesignItem>() {
        *state.0.borrow_mut() = Some(item);
        state.1.set(Some(view));
    }
}

pub(crate) fn active_view_for_item(item: gpui::EntityId, cx: &App) -> Option<gpui::EntityId> {
    let state = cx.try_global::<ActiveDesignItem>()?;
    let active = state.0.borrow().as_ref()?.upgrade()?;
    (active.entity_id() == item)
        .then_some(state.1.get())
        .flatten()
}

pub(crate) fn clear_active_view(view: gpui::EntityId, cx: &mut App) {
    if let Some(state) = cx.try_global::<ActiveDesignItem>()
        && state.1.get() == Some(view)
    {
        state.1.set(None);
    }
}

/// Apply `ops` to `item`'s document as one undoable batch, exactly as the
/// `batch_design` tool does. The result reports `applied` and, on a failure,
/// the failing op (the batch is then rolled back).
pub(crate) fn apply_ops_to_item(
    item: &Entity<FigItem>,
    ops: &[DesignOp],
    label: &str,
    cx: &mut App,
) -> Result<Value> {
    if ops.is_empty() {
        bail!("the ops list is empty");
    }
    item.update(cx, |item, cx| {
        if !item.is_editable() {
            if item.source_edit_locked() {
                bail!(
                    "the FNX source has unsaved edits; save or discard them before editing the canvas"
                );
            }
            bail!("the design document is still loading");
        }
        item.with_document(cx, |document| {
            let (doc, mut assets) = document.doc_and_assets();
            let default_container = doc.active_page();
            let outcome = apply_batch(doc, &mut assets, ops, label, default_container);
            (Ok(outcome.value), doc_change(outcome.effect))
        })
        .unwrap_or_else(|| Err(anyhow!("the document is no longer available")))
    })
}

pub(crate) fn apply_comment_suggested_fix(
    item: &Entity<FigItem>,
    page: NodeId,
    comment_id: &str,
    cx: &mut App,
) -> Result<()> {
    let operations = {
        let document = item
            .read(cx)
            .document()
            .context("the design is still loading")?;
        let comment = crate::comments::read_comments(&document.doc, page)
            .into_iter()
            .find(|comment| comment.id == comment_id)
            .context("the comment no longer exists")?;
        let fix = comment
            .suggested_fix
            .context("the comment has no suggested fix")?;
        fix.ops
            .into_iter()
            .map(serde_json::from_value::<DesignOp>)
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    anyhow::ensure!(
        !operations.iter().any(design_operation_is_presence),
        "suggested fixes must change design content; selection and viewport operations cannot be undone"
    );
    let result = apply_ops_to_item(item, &operations, "Apply suggested fix", cx)?;
    anyhow::ensure!(
        result["applied"] == true,
        "{}",
        result
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("the suggested fix could not be applied")
    );
    Ok(())
}

/// The active page's top-level layers (up to `limit`, `depth` levels deep) in
/// the style projection with world bounds: what `batch_get` returns for a page
/// listing with `detail: "style"`, and what the v2 agent sessions read.
pub(crate) fn page_scene(item: &FigItem, depth: u32, limit: usize) -> Result<Vec<Value>> {
    let document = ready_document(item)?;
    let page = &document.pages[resolve_page_index(document, None)?];
    let root = page.root.context("the page has no root node")?;
    fanta_headless::scene::page_scene(&document.doc, root, depth, limit)
}

struct FigDesignSurface {
    active: Rc<RefCell<Option<WeakEntity<FigItem>>>>,
}

impl FigDesignSurface {
    fn item(&self) -> Result<Entity<FigItem>> {
        self.active
            .borrow()
            .as_ref()
            .and_then(WeakEntity::upgrade)
            .context("no design canvas is open; open a .fig file or Fanta project first")
    }
}

fn project_source_path(root: &Path, source: &Path) -> Result<PathBuf> {
    // Windows resolves `..` before it looks a path up, so `missing/../x.fnx` names
    // `x.fnx` there but fails on Unix. Refusing `..` gives agents one answer everywhere.
    if source
        .components()
        .any(|component| component == std::path::Component::ParentDir)
    {
        bail!("agent source focus must have a project-contained path");
    }
    let project_root = root
        .canonicalize()
        .context("resolving the design project")?;
    let requested = if source.is_absolute() {
        source.to_path_buf()
    } else {
        root.join(source)
    };
    let mut ancestor = requested.as_path();
    let mut missing = Vec::new();
    let mut resolved = loop {
        match ancestor.canonicalize() {
            Ok(resolved) => break resolved,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match std::fs::symlink_metadata(ancestor) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        bail!("agent source focus contains an unresolved symbolic link");
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error).context("reading agent source focus"),
                }
                let Some(std::path::Component::Normal(name)) = ancestor.components().next_back()
                else {
                    bail!("agent source focus must have a project-contained path");
                };
                missing.push(name.to_os_string());
                ancestor = ancestor
                    .parent()
                    .context("agent source focus has no parent")?;
            }
            Err(error) => return Err(error).context("resolving agent source focus"),
        }
    };
    if !missing.is_empty() && !resolved.is_dir() {
        bail!("agent source focus parent must be a directory");
    }
    for name in missing.into_iter().rev() {
        resolved.push(name);
    }
    if !resolved.starts_with(project_root) {
        bail!("agent source focus must be inside the design project");
    }
    Ok(resolved)
}

impl DesignSurface for FigDesignSurface {
    fn project_root(&self, cx: &mut App) -> Result<Option<PathBuf>> {
        Ok(self.item()?.read(cx).project_root().map(Path::to_path_buf))
    }

    fn prepare_asset(
        &self,
        request: design_surface::DesignAssetRequest,
        cx: &mut App,
    ) -> Result<Value> {
        if request.prompt.trim().is_empty() || request.prompt.len() > 32 * 1024 {
            bail!("the generation prompt must contain 1–32768 bytes");
        }
        if request
            .preferred_model
            .as_ref()
            .is_some_and(|model| model.trim().is_empty() || model.len() > 512)
        {
            bail!("preferred_model must contain 1–512 bytes when provided");
        }
        let item = self.item()?;
        let window = cx.active_window().context("no design window is active")?;
        let kind = request.kind;
        window.update(cx, |_, window, cx| {
            crate::generation_workspace::open_prepared_asset(request, item.downgrade(), window, cx)
        })??;
        Ok(json!({"prepared": true, "submitted": false, "kind": kind,
            "message": "The generation composer is prefilled. The user can review the catalog model and submit there; no generation was submitted."}))
    }

    fn read_design_spec(&self, cx: &mut App) -> Result<Option<design_surface::DesignSpec>> {
        let Some(item) = self.active.borrow().as_ref().and_then(WeakEntity::upgrade) else {
            return Ok(None);
        };
        let item = item.read(cx);
        match item.project_root() {
            Some(root) => design_surface::read_project_design_spec(root),
            None => Ok(None),
        }
    }
    fn validate_source_edit(
        &self,
        path: String,
        source: String,
        cx: &mut App,
    ) -> Task<Result<Value>> {
        if Path::new(&path)
            .extension()
            .is_none_or(|extension| extension != "fnx" && extension != "json")
        {
            return Task::ready(Ok(json!({ "applicable": false })));
        }
        let root = match self.item() {
            Ok(item) => item.read(cx).project_root().map(Path::to_path_buf),
            Err(_) => None,
        };
        cx.background_spawn(async move {
            validate_source_candidate(root.as_deref(), Path::new(&path), &source)
        })
    }

    fn import_image(&self, bytes: Vec<u8>, name: String, cx: &mut App) -> Task<Result<Value>> {
        let item = match self.item() {
            Ok(item) => item,
            Err(error) => return Task::ready(Err(error)),
        };
        let prepared = cx.background_spawn(async move {
            let format = fanta_format::MediaRegistry::with_builtins()
                .sniff(&bytes)
                .context("the image has an unsupported project asset format")?;
            if format.family != "images" {
                bail!("the result is not a supported raster image");
            }
            PreparedImage::new(bytes)
        });
        cx.spawn(async move |cx| {
            let prepared = prepared.await?;
            let mut result = item.update(cx, |item, cx| {
                if !item.is_editable() {
                    bail!("save or discard FNX source edits and wait for the canvas to load before importing images");
                }
                item.with_document(cx, |document| {
                    let result = import_prepared_image(document, prepared, &name);
                    let change = if result.is_ok() { DocChange::Content } else { DocChange::None };
                    (result, change)
                }).context("the design document is still loading")?
            })?;
            let save = item.update(cx, |item, cx| item.save(SaveKind::Explicit, cx));
            save.await.context("the image is in the asset panel, but saving the project failed")?;
            let root = item.read_with(cx, |item, _| item.project_root().map(Path::to_path_buf))
                .context("the project has no saved asset directory")?;
            let relative = result.get("path").and_then(Value::as_str)
                .context("the imported image has no asset path")?;
            let path = root.join(relative);
            result["absolute_path"] = json!(path.display().to_string());
            result["persisted"] = json!(true);
            Ok(result)
        })
    }

    fn comments(&self, page: Option<usize>, include_resolved: bool, cx: &mut App) -> Result<Value> {
        let item = self.item()?;
        let document = ready_document(item.read(cx))?;
        let page_index = resolve_page_index(document, page)?;
        let root = document
            .pages
            .get(page_index)
            .and_then(|page| page.root)
            .context("the page has no root node")?;
        let comments = crate::comments::read_comments(&document.doc, root)
            .into_iter()
            .filter(|comment| include_resolved || !comment.resolved)
            .map(|comment| {
                let resolved_world = crate::comments::resolve_world(&document.doc, &comment);
                let mut value = serde_json::to_value(&comment)?;
                if let Some(object) = value.as_object_mut() {
                    object.insert("resolved_world".into(), json!(resolved_world));
                    if let Some(anchor) = comment.anchor {
                        object.insert(
                            "anchor".into(),
                            json!({"node": anchor.node.to_string(), "offset": anchor.offset}),
                        );
                    }
                }
                Ok(value)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(json!({ "page": page_index, "comments": comments }))
    }

    fn add_comment(&self, draft: design_surface::CommentDraft, cx: &mut App) -> Result<Value> {
        let item = self.item()?;
        item.update(cx, |item, cx| {
            if !item.is_editable() {
                bail!("save or discard source edits and wait for the canvas to load before adding comments");
            }
            item.with_document(cx, |document| {
                let result = (|| {
                    let page_index = resolve_page_index(document, draft.page)?;
                    let root = document.pages.get(page_index).and_then(|page| page.root)
                        .context("the page has no root node")?;
                    let (id, operation) = crate::comments::agent_add_comment_op(&document.doc, root, &draft)?;
                    document.doc.apply(operation)?;
                    let world = crate::comments::read_comments(&document.doc, root)
                        .into_iter()
                        .find(|comment| comment.id == id)
                        .map(|comment| comment.world);
                    Ok(json!({ "page": page_index, "comment_id": id, "created": true, "world": world }))
                })();
                let change = if result.is_ok() { DocChange::Content } else { DocChange::None };
                (result, change)
            }).context("the document is still loading")?
        })
    }

    fn reply_comment(
        &self,
        page: Option<usize>,
        id: String,
        body: String,
        author: String,
        resolve: bool,
        cx: &mut App,
    ) -> Result<Value> {
        let item = self.item()?;
        item.update(cx, |item, cx| {
            if !item.is_editable() {
                bail!("save or discard source edits and wait for the canvas to load before replying to comments");
            }
            item.with_document(cx, |document| {
                let result = (|| {
                    let page_index = resolve_page_index(document, page)?;
                    let root = document.pages.get(page_index).and_then(|page| page.root)
                        .context("the page has no root node")?;
                    let operation = crate::comments::agent_reply_comment_op(
                        &document.doc, root, &id, &body, &author, resolve,
                    )?;
                    document.doc.apply(operation)?;
                    Ok(json!({ "page": page_index, "comment_id": id, "replied": true, "resolved": resolve }))
                })();
                let change = if result.is_ok() { DocChange::Content } else { DocChange::None };
                (result, change)
            }).context("the document is still loading")?
        })
    }

    fn report_source_activity(
        &self,
        path: String,
        mut activity: design_surface::AgentActivity,
        cx: &mut App,
    ) -> Result<Value> {
        {
            let item = self.item()?;
            let item = item.read(cx);
            let document = ready_document(item)?;
            let Some(root) = item.project_root() else {
                return Ok(json!({ "reported": false }));
            };
            let source = project_source_path(root, Path::new(&path))?;
            let source_page = document.pages.iter().enumerate().find_map(|(index, page)| {
                let page_root = page.root?;
                let page_source = document
                    .doc
                    .components
                    .defs
                    .values()
                    .find(|definition| definition.root == page_root)
                    .and_then(|definition| fanta_format::locate_master_source(root, definition.id))
                    .or_else(|| fanta_format::locate_page_source(root, page_root))?;
                (page_source.canonicalize().ok().as_ref() == Some(&source)).then_some(index)
            });
            activity.source_path = Some(source.display().to_string());
            activity.workspace.get_or_insert_with(|| {
                match source.file_name().and_then(|name| name.to_str()) {
                    Some("variables.json" | "active_modes.json") => {
                        design_surface::AgentWorkspace::Variables
                    }
                    _ if source
                        .extension()
                        .is_some_and(|extension| extension == "fnx") =>
                    {
                        design_surface::AgentWorkspace::Canvas
                    }
                    _ => design_surface::AgentWorkspace::Code,
                }
            });
            if let Some(page_index) = source_page {
                activity.page = Some(page_index);
                activity.node = None;
                let page_root = document.pages.get(page_index).and_then(|page| page.root);
                activity.world = content_bounds(&document.doc, page_root)
                    .filter(Bounds::is_finite)
                    .map(|bounds| {
                        let center = bounds.center();
                        [center.x, center.y]
                    });
            }
        }
        self.report_activity(activity, cx)
    }

    fn report_activity(
        &self,
        mut activity: design_surface::AgentActivity,
        cx: &mut App,
    ) -> Result<Value> {
        if activity.agent_id.trim().is_empty()
            || activity.agent_name.trim().is_empty()
            || activity.action.trim().is_empty()
        {
            bail!("agent_id, agent_name and action must not be empty");
        }
        if activity
            .world
            .is_some_and(|world| !world.into_iter().all(f64::is_finite))
        {
            bail!("activity coordinates must be finite");
        }
        let item = self.item()?;
        let item = item.read(cx);
        let document = ready_document(item)?;
        let unscoped_source =
            activity.source_path.is_some() && activity.page.is_none() && activity.node.is_none();
        let page_index = if let Some(raw) = &activity.node {
            let id = parse_node_id(raw)?;
            let node_page = document
                .page_index_of_node(id)
                .with_context(|| format!("node {raw} is not on a design page"))?;
            if let Some(page) = activity.page {
                let page = resolve_page_index(document, Some(page))?;
                if page != node_page {
                    bail!("node {raw} does not belong to page {page}");
                }
            }
            node_page
        } else {
            resolve_page_index(document, activity.page)?
        };
        activity.page = (!unscoped_source).then_some(page_index);
        if let Some(path) = &activity.source_path {
            let root = item
                .project_root()
                .context("agent source focus requires a design project")?;
            let requested = project_source_path(root, Path::new(path))?;
            activity.source_path = Some(requested.display().to_string());
        }
        activity.project_root = item.project_root().map(|root| root.display().to_string());
        if let Some(raw) = &activity.node {
            let id = parse_node_id(raw)?;
            let bounds = document
                .doc
                .scene
                .world_bounds(id)
                .with_context(|| format!("node {raw} does not exist or has no bounds"))?;
            if activity.world.is_none() {
                let center = bounds.center();
                activity.world = Some([center.x, center.y]);
            }
        }
        if activity.world.is_none() && !unscoped_source {
            let selected = document
                .doc
                .selection
                .iter()
                .copied()
                .filter(|id| document.page_index_of_node(*id) == Some(page_index))
                .collect::<Vec<_>>();
            let bounds = union_bounds(&document.doc, &selected).or_else(|| {
                let root = document.pages.get(page_index).and_then(|page| page.root);
                content_bounds(&document.doc, root)
            });
            activity.world = bounds.filter(Bounds::is_finite).map(|bounds| {
                let center = bounds.center();
                [center.x, center.y]
            });
        }
        design_surface::activity_state(cx).update(cx, |state, cx| state.record(activity, cx));
        Ok(json!({ "reported": true }))
    }

    fn state(&self, cx: &mut App) -> Result<Value> {
        let item = self.item()?;
        let document_id = item.entity_id().as_u64();
        let item = item.read(cx);
        let document = ready_document(item)?;
        let doc = &document.doc;
        let project_root = item.project_root();
        let selection: Vec<NodeId> = doc.selection.iter().copied().collect();
        let motion_source =
            project_root.map(|root| root.join("doc/motion.json").display().to_string());
        let motion_clips = doc
            .motion
            .clips
            .values()
            .map(|clip| {
                json!({
                    "id": clip.id.to_string(), "name": clip.name, "duration_ms": clip.duration_ms,
                    "track_count": clip.tracks.len(),
                    "fps": clip.settings.fps, "loop_mode": clip.settings.loop_mode,
                    "scope": clip.settings.scope.map(|scope| scope.to_string()),
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "document_id": document_id,
            "project": truncate_summary_string(item.title().as_ref(), SUMMARY_LABEL_CHARS),
            "project_root": project_root.map(|root| root.display().to_string()),
            "design_spec": project_root.filter(|root| root.join("fanta.md").is_file()).map(|root| root.join("fanta.md").display().to_string()),
            "is_editable": item.is_editable(),
            "source_edit_locked": item.source_edit_locked(),
            "dirty": item.is_dirty(),
            "total_nodes": doc.scene.len(),
            "pages": pages_json(&document.pages, doc, project_root, |design| item.indexed_design_source(design)),
            "page_count": doc.pages().len(),
            "editor_canvas_count": document.pages.len(),
            "active_page_index": resolve_page_index(document, None)?,
            "active_root": doc.active_page().map(|root| root.to_string()),
            "active_page_bounds": bounds_json(content_bounds(doc, doc.active_page())),
            "components": components_json(doc, project_root, 0, COMPONENT_OVERVIEW_LIMIT, |design| item.indexed_design_source(design)),
            "component_count": doc.components.defs.len(),
            "components_offset": 0,
            "components_limit": COMPONENT_OVERVIEW_LIMIT,
            "more_components": doc.components.defs.len() > COMPONENT_OVERVIEW_LIMIT,
            "components_truncated": doc.components.defs.len() > COMPONENT_OVERVIEW_LIMIT,
            "component_discovery": "The component overview is bounded. Read design_system/get_design_system with limit and offset to discover the remaining components and variant sets.",
            "variable_collections": doc.variables.collections.len(),
            "variables": doc.variables.variables.len(),
            "design_system_sources": ["doc/variables.json", "doc/active_modes.json", "components/<set>/set.json"],
            "motion_source": motion_source,
            "motion_clips": motion_clips,
            "selection": selection_ids(doc),
            "selection_bounds": bounds_json(union_bounds(doc, &selection)),
            "viewport": { "center": doc.viewport.center, "zoom": doc.viewport.zoom },
            "hints": STATE_HINTS,
            "capabilities": design_surface::DESIGN_OP_CAPABILITIES,
        }))
    }

    fn design_system(&self, query: DesignSystemQuery, cx: &mut App) -> Result<Value> {
        let item = self.item()?;
        let item = item.read(cx);
        let document = ready_document(item)?;
        design_system_json(&document.doc, item.project_root(), query, |design| {
            item.indexed_design_source(design)
        })
    }

    fn find_empty_space(
        &self,
        width: f64,
        height: f64,
        page: Option<usize>,
        cx: &mut App,
    ) -> Result<Value> {
        if !(width.is_finite() && width > 0.0 && height.is_finite() && height > 0.0) {
            bail!("width and height must be positive");
        }
        let item = self.item()?;
        let item = item.read(cx);
        let document = ready_document(item)?;
        let page_index = resolve_page_index(document, page)?;
        let root = document.pages[page_index]
            .root
            .context("the page has no root node")?;
        let spot = empty_space(&document.doc, root, width, height);
        Ok(json!({
            "page": page_index,
            "x": spot.x,
            "y": spot.y,
            "width": width,
            "height": height,
        }))
    }

    fn get_nodes(&self, query: NodeQuery, cx: &mut App) -> Result<Value> {
        let item = self.item()?;
        let item = item.read(cx);
        let document = ready_document(item)?;
        let doc = &document.doc;

        if let Some(ids) = &query.ids {
            let detail = query.detail.unwrap_or(NodeDetail::Style);
            let mut nodes = Vec::with_capacity(ids.len());
            for raw in ids {
                let id = parse_node_id(raw)?;
                let node = doc
                    .scene
                    .get(id)
                    .with_context(|| format!("node {raw} does not exist"))?;
                let mut value = match detail {
                    NodeDetail::Raw => serde_json::to_value(node)?,
                    NodeDetail::Style => crate::agent_style::node_style(doc, id),
                    NodeDetail::Summary => node_summary(doc, id, Some(0), query.include_geometry),
                };
                if let Some(object) = value.as_object_mut() {
                    object.insert(
                        "children".into(),
                        json!(
                            doc.scene
                                .children_of(Some(id))
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                        ),
                    );
                    if query.include_geometry {
                        object.insert("world_bounds".into(), world_bounds_json(doc, id));
                    }
                }
                nodes.push(value);
            }
            return Ok(json!({ "nodes": nodes }));
        }

        let page_index = resolve_page_index(document, query.page)?;
        let page = &document.pages[page_index];
        let root = page.root.context("the page has no root node")?;
        check_page_listing_budget(
            doc,
            root,
            query.depth,
            query.offset.unwrap_or(0),
            query.limit.unwrap_or(DEFAULT_CHILD_LIMIT),
        )?;
        let mut listing = paginated_node_summary(
            doc,
            root,
            query.depth,
            query.include_geometry,
            query.offset.unwrap_or(0),
            query.limit.unwrap_or(DEFAULT_CHILD_LIMIT),
        );
        match query.detail {
            Some(NodeDetail::Style) => crate::agent_style::attach_styles(doc, &mut listing),
            Some(NodeDetail::Raw) => {
                bail!("`detail: \"raw\"` returns whole node records; fetch them by `ids`")
            }
            Some(NodeDetail::Summary) | None => {}
        }
        Ok(json!({
            "page": page_index,
            "name": truncate_summary_string(page.name.as_ref(), SUMMARY_LABEL_CHARS),
            "root": listing,
        }))
    }

    fn apply(&self, ops: Vec<DesignOp>, label: String, cx: &mut App) -> Result<Value> {
        apply_ops_to_item(&self.item()?, &ops, &label, cx)
    }

    fn apply_streamed(
        &self,
        ops: Vec<DesignOp>,
        label: String,
        activity: Option<design_surface::AgentActivity>,
        cx: &mut App,
    ) -> Task<Result<Value>> {
        let item = match self.item() {
            Ok(item) => item,
            Err(error) => return Task::ready(Err(error)),
        };
        if ops.is_empty() {
            return Task::ready(Err(anyhow!("the ops list is empty")));
        }
        if ops.iter().all(design_operation_is_presence) {
            return Task::ready(apply_ops_to_item(&item, &ops, &label, cx));
        }
        let default_container = item.read(cx).doc().and_then(Doc::active_page);
        let batch = cx.new(|cx| {
            cx.on_release(|batch: &mut StreamedDesignBatch, cx| batch.cancel(cx))
                .detach();
            StreamedDesignBatch {
                item: item.downgrade(),
                owner: cx.entity_id(),
                progress: BatchProgress::new(default_container),
                original_selection: Vec::new(),
                original_viewport: Viewport::default(),
                pending: false,
            }
        });
        let activity = activity.unwrap_or_else(|| design_surface::AgentActivity {
            agent_id: "external-designer".into(),
            agent_name: "Cornelius".into(),
            action: label.clone(),
            page: None,
            node: None,
            world: None,
            active: true,
            project_root: None,
            source_path: None,
            workspace: Some(design_surface::AgentWorkspace::Canvas),
        });
        cx.spawn(async move |cx| {
            loop {
                let started = batch.update(cx, |batch, cx| {
                    item.update(cx, |item, cx| {
                        if !item.is_editable() {
                            bail!(
                                "save or discard the current source edit before editing the design"
                            );
                        }
                        if item.content_preview_active() {
                            return Ok(false);
                        }
                        if !item.can_preview_for_owner(batch.owner) {
                            bail!("finish saving the project before editing the design");
                        }
                        item.with_document_for_preview_owner(batch.owner, cx, |document| {
                            batch.original_selection =
                                document.doc.selection.iter().copied().collect();
                            batch.original_viewport = document.doc.viewport;
                            document.doc.history.begin(&label, &mut document.doc.scene);
                            batch.progress.mark_history_started();
                            batch.pending = true;
                            (true, DocChange::ContentPreview)
                        })
                        .context("the design document is still loading")
                    })
                })?;
                if started {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(90))
                    .await;
            }

            for (index, operation) in ops.iter().enumerate() {
                let next_activity = batch.update(cx, |batch, cx| {
                    item.update(cx, |item, cx| {
                        let project_root =
                            item.project_root().map(|root| root.display().to_string());
                        let mut next_activity = item
                            .with_document_for_preview_owner(batch.owner, cx, |document| {
                                {
                                    let (doc, mut assets) = document.doc_and_assets();
                                    batch.progress.apply(doc, &mut assets, index, operation);
                                }
                                let target = batch
                                    .progress
                                    .last_created()
                                    .or_else(|| design_operation_node(&document.doc, operation));
                                if let Some(root) = document.doc.active_page() {
                                    document.solved_pages.remove(&root);
                                    document.ensure_root_solved(root);
                                }
                                if let Some(target) = target {
                                    let root = document
                                        .doc
                                        .scene
                                        .ancestors_of(target)
                                        .last()
                                        .map_or(target, |node| node.id);
                                    document.solved_pages.remove(&root);
                                    document.ensure_root_solved(root);
                                }
                                if design_operation_is_variable(operation) {
                                    document.mark_variables_changed();
                                }
                                let mut activity = activity.clone();
                                activity.action =
                                    format!("{} · {} of {}", label, index + 1, ops.len());
                                activity.active = true;
                                activity.source_path = None;
                                activity.workspace =
                                    Some(if design_operation_is_variable(operation) {
                                        design_surface::AgentWorkspace::Variables
                                    } else {
                                        design_surface::AgentWorkspace::Canvas
                                    });
                                activity.project_root = project_root;
                                activity.node = target.map(|node| node.to_string());
                                activity.page =
                                    target.and_then(|node| document.page_index_of_node(node));
                                activity.world = target
                                    .and_then(|node| document.doc.scene.world_bounds(node))
                                    .filter(Bounds::is_finite)
                                    .map(|bounds| {
                                        let center = bounds.center();
                                        [center.x, center.y]
                                    });
                                (activity, DocChange::ContentPreview)
                            })
                            .context("the design document became unavailable")?;
                        let document = ready_document(item)?;
                        next_activity.page = next_activity
                            .node
                            .as_deref()
                            .and_then(|node| node.parse::<NodeId>().ok())
                            .and_then(|node| document.page_index_of_node(node));
                        // Agent previews can introduce layers and variables;
                        // panels that skip drag previews must project these steps.
                        cx.emit(FigItemEvent::Edited);
                        Ok::<_, anyhow::Error>(next_activity)
                    })
                })?;
                cx.update(|cx| {
                    design_surface::activity_state(cx)
                        .update(cx, |state, cx| state.record(next_activity, cx));
                });
                if batch.read_with(cx, |batch, _| batch.progress.failed()) {
                    break;
                }
                // Yield a painted frame for each operation without splitting the undo transaction.
                cx.background_executor()
                    .timer(Duration::from_millis(90))
                    .await;
            }
            batch.update(cx, |batch, cx| {
                item.update(cx, |item, cx| {
                    let success = !batch.progress.failed();
                    let outcome = item
                        .with_document_for_owner(batch.owner, cx, |document| {
                            let outcome = {
                                let (doc, mut assets) = document.doc_and_assets();
                                batch.progress.finish(doc, &mut assets, ops.len())
                            };
                            if !success {
                                refresh_streamed_layout(document);
                            }
                            let change = if success {
                                doc_change(outcome.effect)
                            } else {
                                DocChange::ContentPreview
                            };
                            (outcome.value, change)
                        })
                        .context("the design document became unavailable")?;
                    batch.pending = false;
                    item.finish_content_preview(batch.owner, success, cx);
                    Ok(outcome)
                })
            })
        })
    }

    fn screenshot(&self, target: ScreenshotTarget, cx: &mut App) -> Task<Result<Vec<u8>>> {
        let prepared = self
            .item()
            .and_then(|item| item.update(cx, |item, cx| prepare_screenshot(item, &target, cx)));
        let (doc, asset_resolver, page_root, node) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => return Task::ready(Err(error)),
        };
        cx.background_spawn(async move {
            render_surface_screenshot(&doc, asset_resolver, page_root, node, target)
        })
    }

    fn read_source(&self, path: Option<String>, cx: &mut App) -> Result<Value> {
        let item = self.item()?;
        let item = item.read(cx);
        let root = item.project_root().context(
            "the document has no on-disk Fanta project yet; save the canvas once to materialize one",
        )?;
        match path {
            None => Ok(json!({
                "root": root.display().to_string(),
                "files": list_source_files(root),
            })),
            Some(relative) => {
                let requested = root.join(&relative);
                let canonical_root = root
                    .canonicalize()
                    .with_context(|| format!("resolving {}", root.display()))?;
                let canonical = requested
                    .canonicalize()
                    .with_context(|| format!("{relative} does not exist in the project"))?;
                if !canonical.starts_with(&canonical_root) {
                    bail!("{relative} escapes the project root");
                }
                let text = std::fs::read_to_string(&canonical)
                    .with_context(|| format!("reading {relative}"))?;
                Ok(json!({ "path": relative, "text": text }))
            }
        }
    }
}

fn validate_source_candidate(
    active_root: Option<&Path>,
    source_path: &Path,
    source: &str,
) -> Result<Value> {
    let path = if source_path.is_absolute() {
        match source_path.canonicalize() {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => source_path.to_path_buf(),
            Err(error) => return Err(error).context("resolving the agent source candidate"),
        }
    } else {
        source_path.to_path_buf()
    };
    let mut inferred_root = None;
    if path.is_absolute() {
        for ancestor in path.parent().into_iter().flat_map(Path::ancestors) {
            if ancestor.join("fanta.json").try_exists()? {
                inferred_root = Some(ancestor);
                break;
            }
        }
    }
    let Some(root) = inferred_root.or(active_root) else {
        return Ok(json!({ "applicable": false }));
    };
    validate_agent_source_edit(root, &path, source)
}

fn validate_agent_source_edit(
    project_root: &Path,
    source_path: &Path,
    source: &str,
) -> Result<Value> {
    use std::path::Component;
    let canonical_root = project_root
        .canonicalize()
        .with_context(|| format!("resolving Fanta project {}", project_root.display()))?;
    let requested = if source_path.is_absolute() {
        source_path
            .strip_prefix(project_root)
            .map(|relative| canonical_root.join(relative))
            .unwrap_or_else(|_| source_path.to_path_buf())
    } else {
        canonical_root.join(source_path)
    };
    let source_path = match requested.canonicalize() {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => requested,
        Err(error) => return Err(error).context("resolving the design source candidate"),
    };
    let Ok(relative) = source_path.strip_prefix(&canonical_root) else {
        return Ok(json!({ "applicable": false }));
    };
    let components = relative.components().collect::<Vec<_>>();
    if validate_managed_json(relative, source)
        .with_context(|| format!("Invalid managed JSON in {}", source_path.display()))?
    {
        let scope = if source_path.is_file() {
            fanta_format::validate_project_json_edit(&canonical_root, &source_path, source)
                .context("the JSON candidate cannot be materialized; the existing source has not been overwritten")?;
            "project"
        } else {
            "typed_json"
        };
        return Ok(
            json!({"applicable":true,"validated":true,"scope":scope,"path":source_path.display().to_string(),"diagnostics":[]}),
        );
    }
    let managed = matches!(components.as_slice(), [
        Component::Normal(directory), Component::Normal(_), Component::Normal(file)
    ] if (*directory == "pages" && *file == "page.fnx")
        || (*directory == "components" && *file == "master.fnx"))
        // A variant's master, inside its component set's folder.
        || matches!(components.as_slice(), [
            Component::Normal(directory), Component::Normal(_), Component::Normal(_),
            Component::Normal(file)
        ] if *directory == "components" && *file == "master.fnx");
    if !managed {
        return Ok(json!({ "applicable": false }));
    }
    fanta_fnx::parse_doc(source).with_context(|| format!(
        "Invalid FNX in {}. FNX attributes require quoted JSON object keys and complete closing tags; repair the candidate before saving",
        source_path.display()
    ))?;
    let (scope, diagnostics) = if source_path.is_file() {
        let (_, diagnostics) = fanta_format::validate_project_source_edit_with_diagnostics(
            &canonical_root, &source_path, source,
        ).context("the FNX candidate cannot be materialized; the existing source has not been overwritten")?;
        ("project", serde_json::to_value(diagnostics)?)
    } else {
        ("syntax", json!([]))
    };
    Ok(
        json!({ "applicable": true, "validated": true, "scope": scope,
        "path": source_path.display().to_string(), "diagnostics": diagnostics }),
    )
}

fn validate_typed_json<T: serde::de::DeserializeOwned>(source: &str) -> Result<()> {
    let value: Value = serde_json::from_str(source)?;
    if !value.is_object() && !value.is_null() {
        bail!("this managed JSON source requires an object");
    }
    serde_json::from_value::<T>(value)?;
    Ok(())
}

fn validate_json_value<T: serde::de::DeserializeOwned>(source: &str) -> Result<()> {
    serde_json::from_str::<T>(source)?;
    Ok(())
}

fn validate_managed_json(relative: &Path, source: &str) -> Result<bool> {
    use std::collections::BTreeMap;
    use std::path::Component;

    let components = relative.components().collect::<Vec<_>>();
    match components.as_slice() {
        [Component::Normal(file)] if *file == "fanta.json" => {
            validate_typed_json::<fanta_format::ProjectManifest>(source)?;
            let manifest: fanta_format::ProjectManifest = serde_json::from_str(source)?;
            if manifest.format != "fanta-project" {
                bail!("the project manifest requires the fanta-project format tag");
            }
            manifest.project_id.parse::<fanta_doc::DocId>()?;
        }
        [Component::Normal(directory), Component::Normal(file)] if *directory == "doc" => {
            match file.to_str() {
                Some("metadata.json") => validate_typed_json::<fanta_doc::DocMetadata>(source)?,
                Some("asset_library.json") => {
                    validate_typed_json::<BTreeMap<AssetId, ProjectAsset>>(source)?
                }
                Some("variables.json") => {
                    validate_typed_json::<fanta_doc::VariableRegistry>(source)?
                }
                Some("active_modes.json") => {
                    validate_typed_json::<BTreeMap<VariableCollectionId, ModeId>>(source)?
                }
                Some("motion.json") => validate_typed_json::<fanta_doc::MotionLibrary>(source)?,
                Some("flow_start.json") => validate_json_value::<Option<NodeId>>(source)?,
                Some("flows.json") => validate_json_value::<Vec<fanta_doc::Flow>>(source)?,
                Some("presentation.json") => {
                    validate_typed_json::<Option<fanta_doc::PresentationConfig>>(source)?
                }
                _ => return Ok(false),
            }
        }
        [Component::Normal(directory), Component::Normal(file)]
            if *directory == "components" && *file == "sets.json" =>
        {
            validate_typed_json::<BTreeMap<ComponentId, fanta_doc::ComponentSet>>(source)?;
        }
        [
            Component::Normal(directory),
            Component::Normal(_),
            Component::Normal(file),
        ] if *directory == "components" && *file == "set.json" => {
            validate_typed_json::<fanta_doc::ComponentSet>(source)?;
        }
        [
            Component::Normal(directory),
            Component::Normal(_),
            Component::Normal(_),
            Component::Normal(file),
        ] if *directory == "components" && *file == "def.json" => {
            validate_typed_json::<fanta_doc::ComponentDef>(source)?;
        }
        [Component::Normal(directory), Component::Normal(file)]
            if *directory == "assets" && *file == "index.json" =>
        {
            let index: Value = serde_json::from_str(source)?;
            if index.get("version").and_then(Value::as_u64) != Some(1) {
                bail!("the asset index requires version 1");
            }
            for (name, record) in index
                .get("assets")
                .and_then(Value::as_object)
                .context("the asset index requires an assets object")?
            {
                name.parse::<AssetId>()?;
                record
                    .get("size")
                    .and_then(Value::as_u64)
                    .context("asset size must be an unsigned integer")?;
                let digest = record
                    .get("sha256")
                    .and_then(Value::as_str)
                    .context("asset SHA-256 must be a string")?;
                if digest.len() != 64
                    || !digest
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                {
                    bail!("asset {name} requires a lowercase SHA-256 digest");
                }
            }
        }
        [
            Component::Normal(directory),
            Component::Normal(_),
            Component::Normal(file),
        ] if *directory == "pages" && *file == "page.json" => {
            let header: Value = serde_json::from_str(source)?;
            header
                .get("order")
                .and_then(Value::as_u64)
                .context("page order must be an unsigned integer")?;
            if let Some(id) = header.get("id") {
                id.as_str()
                    .context("page identity must be a string")?
                    .parse::<NodeId>()?;
            }
            if let Some(name) = header.get("name") {
                name.as_str().context("page name must be a string")?;
            }
        }
        [
            Component::Normal(directory),
            Component::Normal(_),
            Component::Normal(file),
        ] if *directory == "components" && *file == "def.json" => {
            validate_typed_json::<fanta_doc::ComponentDef>(source)?;
        }
        [
            Component::Normal(directory),
            Component::Normal(_),
            Component::Normal(file),
        ]
        | [
            Component::Normal(directory),
            Component::Normal(_),
            Component::Normal(_),
            Component::Normal(file),
        ] if (*directory == "pages" && *file == "page.ids.json")
            || (*directory == "components" && *file == "master.ids.json") =>
        {
            validate_typed_json::<fanta_fnx::FnxSidecar>(source)?;
            let sidecar: fanta_fnx::FnxSidecar = serde_json::from_str(source)?;
            for entry in sidecar.ids {
                serde_json::from_value::<NodeId>(json!(entry.id))?;
                if !entry.index.is_null() && !entry.index.is_number() {
                    bail!("sidecar sibling order must be a number");
                }
            }
            if let Some(parent) = sidecar.root_parent {
                serde_json::from_value::<NodeId>(json!(parent))?;
            }
        }
        [
            Component::Normal(directory),
            Component::Normal(_),
            Component::Normal(nodes),
            Component::Normal(file),
        ] if (*directory == "pages" || *directory == "components")
            && *nodes == "nodes"
            && Path::new(file)
                .extension()
                .is_some_and(|extension| extension == "json") =>
        {
            validate_typed_json::<CanvasNode>(source)?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn import_prepared_image(
    document: &mut FigDocument,
    prepared: PreparedImage,
    name: &str,
) -> Result<Value> {
    let (asset, natural_size, _) = document
        .doc_and_assets()
        .1
        .add_prepared_image_tracked(prepared)?;
    let name = name.trim().chars().take(80).collect::<String>();
    document
        .doc
        .asset_library
        .entry(asset)
        .or_insert_with(|| ProjectAsset {
            name: if name.is_empty() {
                "Generated image".into()
            } else {
                name
            },
            kind: ProjectAssetKind::Image,
        });
    let bytes = document
        .raw_assets
        .get(&asset)
        .context("the imported image bytes are unavailable")?;
    let format = fanta_format::MediaRegistry::with_builtins()
        .sniff(bytes)
        .context("the imported image has an unsupported asset format")?;
    Ok(json!({
        "asset_id": asset.to_string(), "path": format!("assets/{}/{}.{}", format.family, asset, format.extension),
        "width": natural_size[0], "height": natural_size[1], "in_asset_panel": true,
    }))
}

fn ready_document(item: &FigItem) -> Result<&FigDocument> {
    item.document()
        .context("the design document is still loading")
}

fn selection_ids(doc: &Doc) -> Vec<String> {
    doc.selection.iter().map(ToString::to_string).collect()
}

/// One entry per page, each naming the `.fnx` file that page IS — the whole
/// point of a Fanta project is that an agent edits the source, so the state
/// an agent reads has to say which file to open.
///
fn pages_json(
    pages: &[FigPage],
    doc: &Doc,
    project_root: Option<&Path>,
    source: impl Fn(fanta_format::ScopedDesign) -> Option<PathBuf>,
) -> Vec<Value> {
    let active_page = doc.active_page();
    let authored_pages = doc.pages().iter().copied().collect::<HashSet<_>>();
    pages
        .iter()
        .enumerate()
        .filter(|(_, page)| {
            page.root
                .is_none_or(|root| authored_pages.contains(&root) || Some(root) == active_page)
        })
        .map(|(index, page)| {
            let source = page.root.and_then(|root| {
                let design = if authored_pages.contains(&root) {
                    Some(fanta_format::ScopedDesign::Page(root))
                } else {
                    doc.components
                        .defs
                        .values()
                        .find(|definition| definition.root == root)
                        .map(|definition| fanta_format::ScopedDesign::Component(definition.id))
                };
                design.and_then(&source)
            });
            json!({
                "index": index,
                "name": truncate_summary_string(page.name.as_ref(), SUMMARY_LABEL_CHARS),
                "root": page.root.map(|id| id.to_string()),
                "hidden": page.hidden,
                "active": page.root.is_some() && page.root == active_page,
                "nodes": page
                    .root
                    .map(|root| doc.scene.descendants_of(root).count().saturating_sub(1))
                    .unwrap_or(0),
                "source": relative_source(project_root, source),
            })
        })
        .collect()
}

/// What every state read tells the model up front, because the mistakes
/// these prevent (guessed ids, y-up math, unverified results) are the common
/// ones.
const STATE_HINTS: [&str; 8] = [
    "Read each page/component source under project_root and edit the .fnx files with file tools; preserve stable ids and save before inspecting the reloaded canvas.",
    "Coordinates are world px with y growing downward; x/y of an op is the node's top-left corner.",
    "Ids are exact node ids from this state or a page listing; never guess or use layer names.",
    "Ask for empty_space before creating a new top-level frame so it does not land on existing work.",
    "When using design_edit/batch_design as a fallback, batch related ops with a descriptive label; it is one undo step.",
    "Author animation clips in motion_source (doc/motion.json), preserving clip/track/keyframe ids; verify with screenshot motion_clip and playhead_ms samples.",
    "Import generated image files into project assets with import_project_image (native) or import_image (MCP); use the returned asset id/path when authoring the design.",
    "Verify substantive edits with a screenshot of the changed frame before reporting done.",
];

/// Explicit page indices error when out of range; `None` falls back to the
/// active page like the canvas does.
fn resolve_page_index(document: &FigDocument, page: Option<usize>) -> Result<usize> {
    if let Some(page) = page {
        if page >= document.pages.len() {
            bail!(
                "page index {page} is out of range (the document has {} pages)",
                document.pages.len()
            );
        }
        return Ok(page);
    }
    if let Some(active_root) = document.doc.active_page()
        && let Some(index) = document
            .pages
            .iter()
            .position(|page| page.root == Some(active_root))
    {
        return Ok(index);
    }
    document
        .page_index(None)
        .context("the document has no pages")
}

fn render_surface_screenshot(
    doc: &Doc,
    asset_resolver: Option<Arc<dyn AssetResolver>>,
    page_root: Option<NodeId>,
    node: Option<NodeId>,
    target: ScreenshotTarget,
) -> Result<Vec<u8>> {
    fanta_headless::render::render_screenshot(doc, asset_resolver, page_root, node, target)
        .map(|screenshot| screenshot.png)
}

#[allow(clippy::type_complexity)]
fn prepare_screenshot(
    item: &mut FigItem,
    target: &ScreenshotTarget,
    cx: &mut gpui::Context<FigItem>,
) -> Result<(
    Doc,
    Option<Arc<dyn AssetResolver>>,
    Option<NodeId>,
    Option<NodeId>,
)> {
    let node = target.node.as_deref().map(parse_node_id).transpose()?;
    let page_index = {
        let document = ready_document(item)?;
        match node {
            Some(node) => {
                if doc_get(document, node).is_none() {
                    bail!(
                        "node {} does not exist",
                        target.node.as_deref().unwrap_or("")
                    );
                }
                document
                    .page_index_of_node(node)
                    .context("the node is not on any page")?
            }
            None => resolve_page_index(document, target.page)?,
        }
    };
    // Pages other than the active one may not have their layout solved yet;
    // solve before cloning so the render sees final geometry.
    item.with_document(cx, |document| {
        document.ensure_page_solved(page_index);
        ((), DocChange::None)
    });
    let document = ready_document(item)?;
    Ok((
        document.doc.clone_for_persist(),
        document.asset_resolver.clone(),
        document.pages[page_index].root,
        node,
    ))
}

fn doc_get(document: &FigDocument, id: NodeId) -> Option<&CanvasNode> {
    document.doc.scene.get(id)
}

fn list_source_files(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    for source in [
        "fanta.json",
        "fanta.md",
        "doc/motion.json",
        "doc/variables.json",
        "doc/active_modes.json",
        "components/sets.json",
    ] {
        if root.join(source).is_file() {
            files.push(source.into());
        }
    }
    for (directory, file_name) in [
        ("pages", "page.fnx"),
        ("components", "master.fnx"),
        ("motion", "motion.json"),
    ] {
        let Ok(entries) = std::fs::read_dir(root.join(directory)) else {
            continue;
        };
        let mut found: Vec<String> = entries
            .flatten()
            .flat_map(|entry| {
                let folder = entry.file_name().to_string_lossy().into_owned();
                let mut sources = Vec::new();
                if entry.path().join(file_name).is_file() {
                    sources.push(format!("{directory}/{folder}/{file_name}"));
                }
                // A component set's folder: its set.json and its variants.
                if directory == "components" && entry.path().join("set.json").is_file() {
                    sources.push(format!("{directory}/{folder}/set.json"));
                    let mut variants: Vec<String> = std::fs::read_dir(entry.path())
                        .into_iter()
                        .flatten()
                        .flatten()
                        .filter(|variant| variant.path().join(file_name).is_file())
                        .map(|variant| {
                            format!(
                                "{directory}/{folder}/{}/{file_name}",
                                variant.file_name().to_string_lossy()
                            )
                        })
                        .collect();
                    variants.sort();
                    sources.append(&mut variants);
                }
                sources
            })
            .collect();
        found.sort();
        files.append(&mut found);
    }
    files
}

// ---- batch application ------------------------------------------------------

struct StreamedDesignBatch {
    item: WeakEntity<FigItem>,
    owner: gpui::EntityId,
    progress: BatchProgress,
    original_selection: Vec<NodeId>,
    original_viewport: Viewport,
    pending: bool,
}

impl StreamedDesignBatch {
    fn cancel(&mut self, cx: &mut App) {
        if !self.pending {
            return;
        }
        self.item
            .update(cx, |item, cx| {
                item.with_document_for_owner(self.owner, cx, |document| {
                    {
                        let (doc, mut assets) = document.doc_and_assets();
                        if let Err(error) = doc.abort_transaction() {
                            log::error!(
                                "rolling back an interrupted agent design edit failed: {error}"
                            );
                        }
                        for asset in self.progress.take_ingested_assets() {
                            assets.remove(asset);
                        }
                        doc.selection
                            .replace_with(self.original_selection.iter().copied());
                        doc.viewport = self.original_viewport;
                    }
                    refresh_streamed_layout(document);
                    ((), DocChange::ContentPreview)
                });
                item.finish_content_preview(self.owner, false, cx);
            })
            .log_err();
        self.pending = false;
    }
}

fn doc_change(effect: BatchEffect) -> DocChange {
    match effect {
        BatchEffect::None => DocChange::None,
        BatchEffect::Selection => DocChange::Selection,
        BatchEffect::Content => DocChange::Content,
    }
}

fn refresh_streamed_layout(document: &mut FigDocument) {
    document.refresh_layout_baseline();
    document.mark_variables_changed();
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use fanta_doc::{
        BitmapNode, Color, GroupNode, ImageFitMode, NodeData, Operation, Transform2D, VectorNode,
    };
    use fanta_headless::apply::BatchOutcome;

    #[test]
    fn agent_source_focus_accepts_new_files_inside_the_project() -> Result<()> {
        let project = tempfile::tempdir()?;
        let relative = Path::new("components/Card/master.fnx");
        assert_eq!(
            project_source_path(project.path(), relative)?,
            project.path().canonicalize()?.join(relative),
        );
        let escaped = project.path().join("../outside-fanta-project/new.fnx");
        assert!(project_source_path(project.path(), &escaped).is_err());
        assert!(project_source_path(project.path(), Path::new("missing/../other.fnx")).is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn agent_source_focus_resolves_links_before_accepting_new_files() -> Result<()> {
        let project = tempfile::tempdir()?;
        let outside = tempfile::tempdir()?;
        let outside_link = project.path().join("Outside");
        std::os::unix::fs::symlink(outside.path(), &outside_link)?;
        assert!(project_source_path(project.path(), &outside_link.join("new.fnx")).is_err());
        let dangling_link = project.path().join("Unresolved");
        std::os::unix::fs::symlink(outside.path().join("missing"), &dangling_link)?;
        assert!(project_source_path(project.path(), &dangling_link.join("new.fnx")).is_err());
        let components = project.path().join("components");
        std::fs::create_dir(&components)?;
        let internal_link = project.path().join("Library");
        std::os::unix::fs::symlink(&components, &internal_link)?;
        assert_eq!(
            project_source_path(project.path(), &internal_link.join("Card/master.fnx"))?,
            components.canonicalize()?.join("Card/master.fnx"),
        );
        Ok(())
    }

    fn ops(value: Value) -> Vec<DesignOp> {
        serde_json::from_value(value).expect("ops JSON matches the DesignOp schema")
    }

    fn doc_with_page() -> (Doc, NodeId) {
        let mut doc = Doc::new();
        let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let page_id = page.id;
        doc.apply(Operation::create_node(page)).unwrap();
        doc.add_page(page_id);
        doc.set_active_page(Some(page_id));
        doc.history = Default::default();
        (doc, page_id)
    }

    fn created_id(outcome: &BatchOutcome, index: usize) -> NodeId {
        outcome.value["created"][index]
            .as_str()
            .expect("created id present")
            .parse()
            .expect("created id parses")
    }

    /// Run a batch against throwaway asset stores (for tests that don't
    /// place images).
    fn run_batch(doc: &mut Doc, ops: &[DesignOp], label: &str) -> BatchOutcome {
        let mut stores = crate::document::TestAssetStores::default();
        let default_container = doc.active_page();
        apply_batch(doc, &mut stores.stores(), ops, label, default_container)
    }

    async fn streamed_design_fixture(
        cx: &mut gpui::TestAppContext,
    ) -> (Entity<FigItem>, FigDesignSurface, Entity<project::Project>) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
        });
        let project = project::Project::test(fs::FakeFs::new(cx.executor()), [], cx).await;
        let (doc, _) = doc_with_page();
        let item = crate::document::ready_item_for_test(
            &project,
            PathBuf::from("/tmp/Design.fig"),
            doc,
            cx,
        );
        let surface = FigDesignSurface {
            active: Rc::new(RefCell::new(Some(item.downgrade()))),
        };
        (item, surface, project)
    }

    #[gpui::test]
    async fn add_comment_tool_creates_anchored_agent_comment_and_fix_undo(
        cx: &mut gpui::TestAppContext,
    ) {
        let (item, surface, _project) = streamed_design_fixture(cx).await;
        let page = item.read_with(cx, |item, _| {
            item.doc().expect("doc").active_page().expect("page")
        });
        let created = cx
            .update(|cx| {
                surface.add_comment(design_surface::CommentDraft {
            node: Some(page.to_string()), text: "Rename this frame".into(),
            severity: Some("minor".into()), category: Some("content".into()),
            review_run_id: Some("test-review".into()),
            suggested_fix: Some(design_surface::SuggestedFixDraft {
                summary: "Give the frame a clear name".into(),
                ops: vec![json!({"op":"set_props", "id":page.to_string(), "name":"Reviewed page"})],
            }), ..Default::default()
        }, cx)
            })
            .expect("create comment");
        let comment_id = created["comment_id"].as_str().expect("comment id");
        let comments = cx
            .update(|cx| surface.comments(None, false, cx))
            .expect("comments");
        assert_eq!(comments["comments"][0]["anchor"]["node"], page.to_string());
        assert_eq!(comments["comments"][0]["review_run_id"], "test-review");
        assert_eq!(comments["comments"][0]["resolved_world"], json!([0.0, 0.0]));
        let undo_depth =
            item.read_with(cx, |item, _| item.doc().expect("doc").history.undo_depth());
        cx.update(|cx| apply_comment_suggested_fix(&item, page, comment_id, cx))
            .expect("apply fix");
        item.update(cx, |item, cx| {
            item.with_document(cx, |document| {
                assert_eq!(
                    document.doc.scene.get(page).expect("page").name,
                    "Reviewed page"
                );
                assert_eq!(document.doc.history.undo_depth(), undo_depth + 1);
                assert!(document.doc.undo().expect("undo"));
                assert_ne!(
                    document.doc.scene.get(page).expect("page").name,
                    "Reviewed page"
                );
                assert_eq!(crate::comments::read_comments(&document.doc, page).len(), 1);
                assert!(document.doc.redo().expect("redo"));
                assert_eq!(
                    document.doc.scene.get(page).expect("page").name,
                    "Reviewed page"
                );
                ((), DocChange::Content)
            })
        });
    }

    #[gpui::test]
    async fn item_only_activation_preserves_requester_only_for_the_same_live_document(
        cx: &mut gpui::TestAppContext,
    ) {
        let (item, _, project) = streamed_design_fixture(cx).await;
        cx.update(init);
        let requester = cx.new(|_| ());
        cx.update(|cx| {
            set_active_item_for_view(item.downgrade(), requester.entity_id(), cx);
            set_active_item(item.downgrade(), cx);
            assert_eq!(
                active_view_for_item(item.entity_id(), cx),
                Some(requester.entity_id())
            );
        });
        let (document, _) = doc_with_page();
        let other = crate::document::ready_item_for_test(
            &project,
            PathBuf::from("/tmp/Other.fig"),
            document,
            cx,
        );
        let other_weak = other.downgrade();
        cx.update(|cx| {
            set_active_item(other.downgrade(), cx);
            assert_eq!(active_view_for_item(item.entity_id(), cx), None);
            assert_eq!(active_view_for_item(other.entity_id(), cx), None);
            set_active_item_for_view(other.downgrade(), requester.entity_id(), cx);
        });
        drop(other);
        cx.update(|cx| {
            set_active_item(other_weak, cx);
            assert_eq!(cx.global::<ActiveDesignItem>().1.get(), None);
        });
    }

    #[gpui::test]
    async fn inspecting_component_source_reports_its_component_scope(
        cx: &mut gpui::TestAppContext,
    ) {
        async {
            let (_, _, project) = streamed_design_fixture(cx).await;
            let (mut doc, page) = doc_with_page();
            let mut master = CanvasNode::new(NodeData::Group(GroupNode::default()));
            master.name = "Card".into();
            master.parent = Some(page);
            let master_root = master.id;
            doc.apply(Operation::create_node(master))?;
            let component = fanta_doc::ComponentId::new();
            doc.apply(Operation::DefineComponent {
                def: Box::new(fanta_doc::ComponentDef::new(component, master_root, "Card")),
            })?;
            let directory = tempfile::tempdir()?;
            fanta_format::write_project_tree(directory.path(), &doc, &Default::default())?;
            let source = fanta_format::locate_master_source(directory.path(), component)
                .context("component source")?;
            let item = crate::document::ready_item_with_root_for_test(
                &project,
                directory.path().join("fanta.json"),
                Some(directory.path().to_path_buf()),
                doc,
                cx,
            );
            let component_page = item
                .read_with(cx, |item, _| {
                    item.document()
                        .and_then(|document| document.page_index_of_node(master_root))
                })
                .context("component scope")?;
            let surface = FigDesignSurface {
                active: Rc::new(RefCell::new(Some(item.downgrade()))),
            };
            cx.update(|cx| {
                surface.report_source_activity(
                    source.display().to_string(),
                    design_surface::AgentActivity {
                        agent_id: "reader".into(),
                        agent_name: "Morgana".into(),
                        action: "Inspecting source".into(),
                        page: None,
                        node: None,
                        world: None,
                        active: true,
                        project_root: None,
                        source_path: None,
                        workspace: Some(design_surface::AgentWorkspace::Code),
                    },
                    cx,
                )
            })?;
            let state = cx.update(design_surface::activity_state);
            state.read_with(cx, |state, _| {
                let activity = state
                    .activities()
                    .into_iter()
                    .find(|activity| activity.agent_id == "reader")
                    .context("component inspection activity")?;
                assert_eq!(activity.page, Some(component_page));
                assert_eq!(
                    activity.workspace,
                    Some(design_surface::AgentWorkspace::Code)
                );
                assert_eq!(
                    activity.source_path,
                    Some(source.canonicalize()?.display().to_string())
                );
                Ok::<_, anyhow::Error>(())
            })?;
            Ok::<_, anyhow::Error>(())
        }
        .await
        .expect("component source inspection should identify its component scope");
    }

    #[gpui::test]
    async fn streamed_presence_does_not_start_a_content_preview(cx: &mut gpui::TestAppContext) {
        let (item, surface, _) = streamed_design_fixture(cx).await;
        let baseline = item.read_with(cx, |item, _| {
            let document = item.document().expect("ready document");
            (
                document.render_generation(),
                document.doc.scene.revision(),
                document.variables_generation(),
            )
        });
        let result = cx
            .update(|cx| {
                surface.apply_streamed(
                    ops(json!([
                        {"op":"set_viewport","center":[100.0,200.0],"zoom":0.5},
                        {"op":"select","ids":[]}
                    ])),
                    "Navigate".into(),
                    None,
                    cx,
                )
            })
            .await
            .expect("presence request");
        assert_eq!(result["applied"], true);
        cx.run_until_parked();
        item.read_with(cx, |item, _| {
            let document = item.document().expect("ready document");
            assert_eq!(document.doc.viewport.center, [100.0, 200.0]);
            assert_eq!(document.doc.viewport.zoom, 0.5);
            assert_eq!(document.doc.history.undo_depth(), 0);
            assert_eq!(
                (
                    document.render_generation(),
                    document.doc.scene.revision(),
                    document.variables_generation(),
                ),
                baseline
            );
            assert!(!item.content_preview_active());
            assert!(!item.is_dirty());
        });
    }

    #[gpui::test]
    async fn streamed_design_paints_each_node_and_commits_one_undo_step(
        cx: &mut gpui::TestAppContext,
    ) {
        async {
            let (item, surface, _) = streamed_design_fixture(cx).await;
            let operations = ops(json!([
                {"op":"create_node","node_type":"rectangle","name":"First","x":20,"y":30,"width":100,"height":40},
                {"op":"create_node","node_type":"rectangle","name":"Second","x":180,"y":90,"width":80,"height":40}
            ]));
            let task =
                cx.update(|cx| surface.apply_streamed(operations, "Build cards".into(), None, cx));
            cx.run_until_parked();
            item.read_with(cx, |item, _| {
                let doc = item.doc().expect("ready document");
                assert_eq!(
                    doc.scene.len(),
                    2,
                    "the first node is visible before the second"
                );
                assert!(item.content_preview_active());
                assert_eq!(doc.history.undo_depth(), 0);
            });
            let state = cx.update(design_surface::activity_state);
            let first = state
                .read_with(cx, |state, _| state.activities().first().cloned())
                .context("first activity")?;
            assert_eq!(first.world, Some([70.0, 50.0]));
            cx.executor().advance_clock(Duration::from_millis(90));
            cx.run_until_parked();
            item.read_with(cx, |item, _| {
                assert_eq!(item.doc().expect("document").scene.len(), 3)
            });
            let second = state
                .read_with(cx, |state, _| state.activities().first().cloned())
                .context("second activity")?;
            assert_eq!(second.world, Some([220.0, 110.0]));
            assert_ne!(first.node, second.node);
            cx.executor().advance_clock(Duration::from_millis(90));
            assert_eq!(task.await?["applied"], true);
            item.update(cx, |item, cx| {
                assert!(!item.content_preview_active());
                assert_eq!(item.doc().context("document")?.history.undo_depth(), 1);
                assert!(item.undo(cx)?);
                assert_eq!(item.doc().context("document")?.scene.len(), 1);
                Ok::<_, anyhow::Error>(())
            })?;
            Ok::<_, anyhow::Error>(())
        }
        .await
        .expect("streamed design and undo checks must complete");
    }

    #[gpui::test]
    async fn streamed_design_keeps_its_destination_when_the_viewed_scope_changes(
        cx: &mut gpui::TestAppContext,
    ) {
        async {
            let (item, surface, _) = streamed_design_fixture(cx).await;
            let viewed_page = CanvasNode::new(NodeData::Group(GroupNode::default()));
            let viewed_root = viewed_page.id;
            let destination = item.update(cx, |item, cx| {
                item.with_document(cx, |document| {
                    let result = (|| {
                        let destination = document.doc.active_page().context("original page")?;
                        document.doc.apply(Operation::create_node(viewed_page))?;
                        document.doc.add_page(viewed_root);
                        document.doc.set_active_page(Some(destination));
                        Ok::<_, anyhow::Error>(destination)
                    })();
                    (result, DocChange::Content)
                })
            });
            let destination = destination.context("ready document")??;
            let operations = ops(json!([
                {"op":"create_node","node_type":"rectangle","name":"First","x":20,"y":30,"width":100,"height":40},
                {"op":"create_node","node_type":"rectangle","name":"Second","x":180,"y":90,"width":80,"height":40},
                {"op":"create_node","node_type":"rectangle","parent":viewed_root.to_string(),"name":"Explicit","x":20,"y":30,"width":80,"height":40}
            ]));
            let task = cx.update(|cx| {
                surface.apply_streamed(operations, "Build across pages".into(), None, cx)
            });
            cx.run_until_parked();
            item.update(cx, |item, cx| {
                item.request_scope(
                    crate::document::FigScope::Page(viewed_root),
                    crate::document::ScopeRequester::Open,
                    cx,
                );
            });
            cx.executor().advance_clock(Duration::from_millis(90));
            cx.run_until_parked();
            item.read_with(cx, |item, _| {
                let doc = item.doc().expect("ready document");
                assert_eq!(doc.active_page(), Some(viewed_root));
                assert_eq!(doc.scene.children_of(Some(destination)).len(), 2);
                assert!(doc.scene.children_of(Some(viewed_root)).is_empty());
            });
            cx.executor().advance_clock(Duration::from_millis(90));
            cx.run_until_parked();
            cx.executor().advance_clock(Duration::from_millis(90));
            assert_eq!(task.await?["applied"], true);
            item.read_with(cx, |item, _| {
                let doc = item.doc().expect("ready document");
                assert_eq!(doc.active_page(), Some(viewed_root));
                assert_eq!(doc.scene.children_of(Some(destination)).len(), 2);
                assert_eq!(doc.scene.children_of(Some(viewed_root)).len(), 1);
            });
            Ok::<_, anyhow::Error>(())
        }
        .await
        .expect("viewing another page must not retarget a streamed batch");
    }

    #[gpui::test]
    async fn streamed_component_is_followable_before_the_batch_commits(
        cx: &mut gpui::TestAppContext,
    ) {
        async {
            let (item, surface, project) = streamed_design_fixture(cx).await;
            let mut master = CanvasNode::new(NodeData::Group(GroupNode {
                clip_size: Some([100.0, 40.0]),
                ..GroupNode::default()
            }));
            master.name = "Card".into();
            let master_root = master.id;
            let page = item
                .update(cx, |item, cx| {
                    item.with_document(cx, |document| {
                        let result = (|| {
                            let page = document.doc.active_page().context("original page")?;
                            master.parent = Some(page);
                            document.doc.apply(Operation::create_node(master))?;
                            Ok::<_, anyhow::Error>(page)
                        })();
                        (result, DocChange::Content)
                    })
                })
                .context("ready document")??;
            let window = cx.add_window(|_, _| gpui::Empty);
            let view = window.update(cx, |_, window, cx| {
                cx.new(|cx| crate::view::FigView::new(item.clone(), project, window, cx))
            })?;
            view.update(cx, |view, cx| view.select_page(0, cx));
            let registry_changes = Rc::new(std::cell::Cell::new(0));
            let _subscription = cx.update(|cx| {
                let registry_changes = registry_changes.clone();
                cx.subscribe(&item, move |_, event, _| {
                    if matches!(event, FigItemEvent::PageRegistryChanged) {
                        registry_changes.set(registry_changes.get() + 1);
                    }
                })
            });
            let state = cx.update(design_surface::activity_state);
            state.update(cx, |state, cx| {
                state.follow(Some("external-designer".into()), cx)
            });
            let initial_undo_depth = item.read_with(cx, |item, _| {
                item.doc().expect("ready document").history.undo_depth()
            });
            let operations = ops(json!([
                {"op":"create_component","id":master_root.to_string()},
                {"op":"set_props","id":master_root.to_string(),"width":160}
            ]));
            let task =
                cx.update(|cx| surface.apply_streamed(operations, "Build Card".into(), None, cx));
            cx.run_until_parked();
            let component_page = item.read_with(cx, |item, _| {
                assert!(item.content_preview_active());
                let document = item.document().expect("ready document");
                assert_eq!(document.doc.pages(), &[page]);
                assert_eq!(document.doc.active_page(), Some(master_root));
                assert_eq!(document.doc.history.undo_depth(), initial_undo_depth);
                let index = document
                    .page_index_of_node(master_root)
                    .expect("component scope");
                let component = document.pages.get(index).expect("registered component");
                assert_eq!(component.name.as_ref(), "Card");
                assert!(component.hidden);
                index
            });
            assert_eq!(
                view.read_with(cx, |view, _| view.selected_page_index()),
                Some(component_page)
            );
            assert_eq!(
                state.read_with(cx, |state, _| state
                    .activities()
                    .first()
                    .and_then(|activity| activity.page)),
                Some(component_page)
            );
            assert_eq!(registry_changes.get(), 1);
            cx.executor().advance_clock(Duration::from_millis(90));
            cx.run_until_parked();
            assert!(item.read_with(cx, |item, _| item.content_preview_active()));
            assert_eq!(registry_changes.get(), 1);
            cx.executor().advance_clock(Duration::from_millis(90));
            assert_eq!(task.await?["applied"], true);
            assert_eq!(registry_changes.get(), 1);
            assert_eq!(
                item.read_with(cx, |item, _| item
                    .doc()
                    .expect("ready document")
                    .history
                    .undo_depth()),
                initial_undo_depth + 1
            );
            assert!(item.update(cx, |item, cx| item.undo(cx))?);
            cx.run_until_parked();
            item.read_with(cx, |item, _| {
                let document = item.document().expect("ready document");
                assert!(!document.doc.is_component_root(master_root));
                assert_eq!(document.doc.active_page(), Some(page));
                assert_eq!(document.doc.history.undo_depth(), initial_undo_depth);
                assert_eq!(document.pages.len(), 1);
                assert_eq!(document.page_index_of_node(master_root), Some(0));
            });
            assert_eq!(
                view.read_with(cx, |view, _| view.selected_page_index()),
                Some(0)
            );
            assert_eq!(registry_changes.get(), 2);
            Ok::<_, anyhow::Error>(())
        }
        .await
        .expect("new component scopes must be visible during streamed construction");
    }

    #[gpui::test]
    async fn cancelling_streamed_design_rolls_back_the_preview(cx: &mut gpui::TestAppContext) {
        let (item, surface, _) = streamed_design_fixture(cx).await;
        let operations = ops(json!([
            {"op":"create_node","node_type":"rectangle","x":0,"y":0,"width":10,"height":10},
            {"op":"create_node","node_type":"rectangle","x":20,"y":20,"width":10,"height":10}
        ]));
        let task =
            cx.update(|cx| surface.apply_streamed(operations, "Cancelled edit".into(), None, cx));
        cx.run_until_parked();
        assert!(item.read_with(cx, |item, _| item.content_preview_active()));
        drop(task);
        cx.run_until_parked();
        // Cancellation releases the batch outside App updates; its rollback
        // observer runs when the next update flushes dropped entities.
        cx.update(|_| {});
        item.read_with(cx, |item, _| {
            assert!(!item.content_preview_active());
            assert!(!item.is_dirty());
            let doc = item.doc().expect("ready document");
            assert_eq!(doc.scene.len(), 1);
            assert_eq!(doc.history.undo_depth(), 0);
        });
    }

    /// A tiny valid PNG (2x1, opaque) as base64.
    fn tiny_png_base64() -> String {
        let mut png = Vec::new();
        let image = image::RgbaImage::from_pixel(2, 1, image::Rgba([255, 0, 0, 255]));
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .expect("encoding the fixture PNG");
        base64::engine::general_purpose::STANDARD.encode(&png)
    }

    #[test]
    fn agent_source_validation_uses_the_target_project_when_focus_changes() -> Result<()> {
        let (doc, page) = doc_with_page();
        let target_project = tempfile::tempdir()?;
        let focused_project = tempfile::tempdir()?;
        for root in [target_project.path(), focused_project.path()] {
            fanta_format::write_project_tree(root, &doc, &Default::default())?;
        }
        let path = fanta_format::locate_page_source(target_project.path(), page)
            .context("target source")?;
        let source = std::fs::read_to_string(&path)?;
        for active_root in [None, Some(focused_project.path())] {
            let valid = validate_source_candidate(active_root, &path, &source)?;
            assert_eq!(valid["validated"], true);
            assert_eq!(valid["scope"], "project");
            assert_eq!(valid["path"], path.canonicalize()?.display().to_string());
            let error = validate_source_candidate(active_root, &path, "<Group><Text")
                .expect_err("unfocused target source still requires validation");
            assert!(format!("{error:#}").contains("Invalid FNX"));
            assert_eq!(std::fs::read_to_string(&path)?, source);
        }
        Ok(())
    }

    #[test]
    fn agent_source_validation_rejects_malformed_candidates_without_writing() -> Result<()> {
        let (mut doc, page) = doc_with_page();
        doc.scene.get_mut(page).context("page")?.name = "Original".into();
        let directory = tempfile::tempdir()?;
        fanta_format::write_project_tree(directory.path(), &doc, &Default::default())?;
        let path =
            fanta_format::locate_page_source(directory.path(), page).context("page source")?;
        let original = std::fs::read_to_string(&path)?;
        let changed = original.replace("name=\"Original\"", "name=\"Changed\"");
        assert_ne!(changed, original);
        let valid = validate_agent_source_edit(directory.path(), &path, &changed)?;
        assert_eq!(valid["validated"], true);
        assert_eq!(valid["scope"], "project");
        for candidate in [
            r##"<Group background={{kind: "solid", color: fnxColor("#FFFFFF")}}/>"##,
            r#"<Group name="Changed"><Text content="Interrupted""#,
        ] {
            let error = validate_agent_source_edit(directory.path(), &path, candidate)
                .expect_err("malformed FNX must fail before saving");
            assert!(format!("{error:#}").contains("Invalid FNX"));
            assert_eq!(std::fs::read_to_string(&path)?, original);
        }
        let (reopened, _) = fanta_format::read_project_tree(directory.path())?;
        assert_eq!(
            reopened.scene.get(page).context("saved page")?.name,
            "Original"
        );
        let ignored = validate_agent_source_edit(
            directory.path(),
            &directory.path().join("README.md"),
            "draft",
        )?;
        assert_eq!(ignored["applicable"], false);
        let new_path = directory.path().join("pages/new/page.fnx");
        let new_source = validate_agent_source_edit(directory.path(), &new_path, &changed)?;
        assert_eq!(new_source["scope"], "syntax");
        assert!(!new_path.exists());
        Ok(())
    }

    #[test]
    fn agent_source_validation_checks_typed_design_system_json_without_active_canvas() -> Result<()>
    {
        let (mut doc, _) = doc_with_page();
        // A variant set, so the project has a `components/chip/set.json`.
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame", "name": "Size=S",
                 "x": 0.0, "y": 0.0, "width": 40.0, "height": 20.0},
                {"op": "create_node", "node_type": "frame", "name": "Size=L",
                 "x": 60.0, "y": 0.0, "width": 80.0, "height": 20.0},
            ])),
            "Frames",
        );
        let (small, large) = (created_id(&outcome, 0), created_id(&outcome, 1));
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_component", "id": small.to_string()},
                {"op": "create_component", "id": large.to_string()},
                {"op": "combine_variants", "ids": [small.to_string(), large.to_string()], "name": "Chip"},
            ])),
            "Chip",
        );
        assert_applied(&outcome);
        let directory = tempfile::tempdir()?;
        fanta_format::write_project_tree(directory.path(), &doc, &Default::default())?;
        for relative in [
            "doc/variables.json",
            "doc/active_modes.json",
            "components/chip/set.json",
            "components/chip/s/def.json",
        ] {
            let path = directory.path().join(relative);
            let original = std::fs::read_to_string(&path)?;
            let valid = validate_source_candidate(None, &path, &original)?;
            assert_eq!(valid["applicable"], true);
            assert_eq!(valid["scope"], "project");
            for invalid in ["{\"unfinished\":", "[]"] {
                assert!(
                    validate_source_candidate(None, &path, invalid).is_err(),
                    "{relative} accepted invalid source {invalid:?}"
                );
                assert_eq!(std::fs::read_to_string(&path)?, original);
            }
        }
        Ok(())
    }

    #[gpui::test]
    async fn agent_json_preflight_runs_without_an_active_canvas(cx: &mut gpui::TestAppContext) {
        let result: Result<()> = async {
            let (doc, page) = doc_with_page();
            let directory = tempfile::tempdir()?;
            fanta_format::write_project_tree(directory.path(), &doc, &Default::default())?;
            let path = fanta_format::locate_page_source(directory.path(), page)
                .context("page source")?
                .with_file_name("page.json");
            let original = std::fs::read_to_string(&path)?;
            let surface = FigDesignSurface {
                active: Rc::new(RefCell::new(None)),
            };
            let validate = cx.update(|cx| {
                surface.validate_source_edit(path.display().to_string(), original.clone(), cx)
            });
            let valid = validate.await?;
            assert_eq!(valid["applicable"], true);
            assert_eq!(valid["scope"], "project");
            let mut header: Value = serde_json::from_str(&original)?;
            header["id"] = json!(NodeId::new().to_string());
            let validate = cx.update(|cx| {
                surface.validate_source_edit(path.display().to_string(), header.to_string(), cx)
            });
            let error = validate
                .await
                .expect_err("valid JSON cannot rebind the page");
            assert!(format!("{error:#}").contains("differs from FNX root"));
            assert_eq!(std::fs::read_to_string(path)?, original);
            Ok(())
        }
        .await;
        result.expect("validate managed JSON before saving without an active canvas");
    }

    #[test]
    fn agent_json_validation_targets_the_file_project_and_allows_new_metadata() -> Result<()> {
        let (doc, page) = doc_with_page();
        let directory = tempfile::tempdir()?;
        let focused_project = tempfile::tempdir()?;
        for root in [directory.path(), focused_project.path()] {
            fanta_format::write_project_tree(root, &doc, &Default::default())?;
        }
        let existing = directory.path().join("doc/metadata.json");
        let original = std::fs::read_to_string(&existing)?;
        for active_root in [None, Some(focused_project.path())] {
            let valid = validate_source_candidate(active_root, &existing, &original)?;
            assert_eq!(valid["scope"], "project");
            assert_eq!(
                valid["path"],
                existing.canonicalize()?.display().to_string()
            );
        }
        let definition = serde_json::to_string(&fanta_doc::ComponentDef::new(
            ComponentId::new(),
            NodeId::new(),
            "New component",
        ))?;
        for (relative, source) in [
            ("components/new/def.json", definition),
            (
                "pages/new/page.json",
                json!({"id":page.to_string(),"order":1}).to_string(),
            ),
            ("doc/flows.json", "[]".into()),
            ("doc/presentation.json", "null".into()),
        ] {
            let path = directory.path().join(relative);
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
            let valid = validate_source_candidate(None, &path, &source)?;
            assert_eq!(valid["applicable"], true);
            assert_eq!(valid["scope"], "typed_json");
            assert!(validate_source_candidate(None, &path, "{\"unfinished\":").is_err());
            assert!(!path.exists());
        }
        let unmanaged = directory.path().join("settings.json");
        assert_eq!(
            validate_source_candidate(None, &unmanaged, "custom settings")?["applicable"],
            false
        );
        Ok(())
    }

    #[test]
    fn imported_tool_images_persist_as_named_assets_without_canvas_layers() -> Result<()> {
        let (doc, _) = doc_with_page();
        let node_count = doc.scene.len();
        let mut document = FigDocument::from_doc(doc, Default::default());
        let bytes = base64::engine::general_purpose::STANDARD.decode(tiny_png_base64())?;
        let result = import_prepared_image(
            &mut document,
            PreparedImage::new(bytes.clone())?,
            "Hero photo",
        )?;
        let asset: AssetId = result["asset_id"].as_str().context("asset id")?.parse()?;
        assert_eq!(document.doc.scene.len(), node_count);
        assert_eq!(
            document
                .doc
                .asset_library
                .get(&asset)
                .context("asset metadata")?
                .name,
            "Hero photo"
        );
        assert!(document.gpui_images.contains_key(&asset));
        let repeated = import_prepared_image(
            &mut document,
            PreparedImage::new(bytes.clone())?,
            "Same result",
        )?;
        assert_eq!(result["asset_id"], repeated["asset_id"]);
        assert_eq!(document.raw_assets.len(), 1);
        let directory = tempfile::tempdir()?;
        fanta_format::write_project_tree(directory.path(), &document.doc, &document.raw_assets)?;
        let relative = result["path"].as_str().context("asset path")?;
        assert_eq!(std::fs::read(directory.path().join(relative))?, bytes);
        let (reopened, raw_assets) = fanta_format::read_project_tree(directory.path())?;
        assert_eq!(
            reopened
                .asset_library
                .get(&asset)
                .context("saved asset metadata")?
                .name,
            "Hero photo"
        );
        assert_eq!(raw_assets.get(&asset), Some(&bytes));
        Ok(())
    }

    #[test]
    fn animation_screenshots_sample_pixels_and_bounds_without_changing_source() -> Result<()> {
        use fanta_doc::{
            AnimationClip, AnimationClipId, AnimationTrack, AnimationTrackId, Keyframe, KeyframeId,
            MotionProperty, MotionTarget, ResolvedVarValue,
        };
        let (mut doc, page_id) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "rectangle", "x": 0.0, "y": 0.0,
                 "width": 20.0, "height": 20.0, "fill": "#0000FF"},
                {"op": "create_node", "node_type": "rectangle", "x": 30.0, "y": 0.0,
                 "width": 20.0, "height": 20.0, "fill": "#FF0000"},
            ])),
            "Animation fixture",
        );
        let moving = outcome.value["created"]
            .as_array()
            .and_then(|created| created.get(1))
            .and_then(Value::as_str)
            .context("moving node id")?
            .parse::<NodeId>()?;
        let authored_transform = doc.scene.get(moving).context("moving node")?.transform;
        let clip_id = AnimationClipId::from_u128(1);
        let mut clip = AnimationClip::new(clip_id, "Slide", 1000);
        let mut track = AnimationTrack::new(
            AnimationTrackId::from_u128(2),
            MotionTarget::new(moving, MotionProperty::PositionX),
        );
        for (id, time, value) in [(3, 0, 30.0), (4, 1000, 70.0)] {
            let keyframe = Keyframe::new(
                KeyframeId::from_u128(id),
                time,
                ResolvedVarValue::Float { value },
            );
            track.keyframes.insert(keyframe.id, keyframe);
        }
        clip.tracks.insert(track.id, track);
        doc.motion.clips.insert(clip_id, clip);
        let target = |clip, time| ScreenshotTarget {
            page: None,
            node: None,
            max_dimension: Some(1024),
            motion_clip: Some(clip),
            playhead_ms: Some(time),
        };
        let initial = image::load_from_memory(&render_surface_screenshot(
            &doc,
            None,
            Some(page_id),
            None,
            target(clip_id.to_string(), 0),
        )?)?
        .to_rgba8();
        let final_frame = image::load_from_memory(&render_surface_screenshot(
            &doc,
            None,
            Some(page_id),
            None,
            target(clip_id.to_string(), 1000),
        )?)?
        .to_rgba8();
        assert!(final_frame.width() > initial.width());
        assert_eq!(initial.height(), final_frame.height());
        assert_ne!(initial.as_raw(), final_frame.as_raw());
        assert_eq!(
            doc.scene
                .get(moving)
                .context("authored moving node")?
                .transform,
            authored_transform
        );
        let error = render_surface_screenshot(
            &doc,
            None,
            Some(page_id),
            None,
            target(AnimationClipId::from_u128(99).to_string(), 0),
        )
        .expect_err("unknown clips must be reported");
        assert!(error.to_string().contains("does not exist"));
        Ok(())
    }

    /// An agent that cannot tell which file backs a page cannot edit the
    /// source, so the state has to name it — relative to the project, the way
    /// a file tool wants it.
    #[test]
    fn page_state_names_the_fnx_source_on_disk() {
        let (doc, page_id) = doc_with_page();
        let temporary = tempfile::tempdir().expect("temporary project");
        fanta_format::write_project_tree(
            temporary.path(),
            &doc,
            &std::collections::BTreeMap::new(),
        )
        .expect("write project tree");
        let pages = vec![FigPage {
            root: Some(page_id),
            name: "Page 1".into(),
            bounds: fanta_doc::Bounds::ZERO,
            hidden: false,
        }];

        let saved_source = fanta_format::locate_page_source(temporary.path(), page_id);
        let located = pages_json(&pages, &doc, Some(temporary.path()), |design| {
            (design == fanta_format::ScopedDesign::Page(page_id))
                .then(|| saved_source.clone())
                .flatten()
        });
        let source = located[0]["source"]
            .as_str()
            .expect("the page names its source");
        assert!(source.ends_with("page.fnx"), "{source}");
        assert!(source.starts_with("pages/"), "{source}");

        // A `.fig` import with no project on disk has no source to name.
        assert_eq!(
            pages_json(&pages, &doc, None, |_| None)[0]["source"],
            Value::Null
        );
    }

    #[test]
    fn page_overview_omits_inactive_component_canvases_and_keeps_active_source() -> Result<()> {
        use fanta_doc::ComponentDef;
        use fanta_format::ScopedDesign;

        let mut doc = Doc::new();
        let mut pages = Vec::new();
        for index in 0..23 {
            let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
            page.name = format!("Page {index}");
            let root = page.id;
            doc.scene.insert(page)?;
            doc.add_page(root);
            pages.push(FigPage {
                root: Some(root),
                name: format!("Page {index}").into(),
                bounds: Bounds::ZERO,
                hidden: false,
            });
        }
        for index in 0..9_012 {
            let master = CanvasNode::new(NodeData::Group(GroupNode::default()));
            let root = master.id;
            doc.scene.insert(master)?;
            let id = ComponentId::from_u128(index + 1);
            doc.components
                .defs
                .insert(id, ComponentDef::new(id, root, format!("Master {index}")));
            pages.push(FigPage {
                root: Some(root),
                name: format!("Master {index}").into(),
                bounds: Bounds::ZERO,
                hidden: true,
            });
        }
        let project = Path::new("/project");
        let calls = std::cell::Cell::new(0);
        let source = |design| {
            calls.set(calls.get() + 1);
            Some(match design {
                ScopedDesign::Page(id) => project.join(format!("pages/{id}/page.fnx")),
                ScopedDesign::Component(id) => project.join(format!("components/{id}/master.fnx")),
            })
        };
        let overview = pages_json(&pages, &doc, Some(project), source);
        assert_eq!(overview.len(), 23);
        assert_eq!(calls.get(), 23);
        for (index, page) in overview.iter().enumerate() {
            assert_eq!(page["index"], index);
        }

        let active_index = 23 + 4_500;
        let active_root = pages
            .get(active_index)
            .and_then(|page| page.root)
            .context("master")?;
        doc.set_active_page(Some(active_root));
        calls.set(0);
        let active_overview = pages_json(&pages, &doc, Some(project), source);
        assert_eq!(active_overview.len(), 24);
        assert_eq!(calls.get(), 24);
        let master = active_overview.last().context("active master entry")?;
        assert_eq!(master["index"], active_index);
        assert_eq!(master["root"], active_root.to_string());
        assert_eq!(master["active"], true);
        assert_eq!(
            master["source"],
            format!("components/{}/master.fnx", ComponentId::from_u128(4_501))
        );
        Ok(())
    }

    fn assert_applied(outcome: &BatchOutcome) {
        assert_eq!(outcome.value["applied"], json!(true), "{}", outcome.value);
    }

    #[gpui::test]
    async fn planner_scene_shallows_broad_frames_without_hiding_their_child_count(
        cx: &mut gpui::TestAppContext,
    ) {
        async {
            let (item, surface, _) = streamed_design_fixture(cx).await;
            let frame = item.update(cx, |item, cx| {
                item.with_document(cx, |document| {
                    let result = (|| {
                        let page = document.doc.active_page().context("active page")?;
                        let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
                            clip_size: Some([128.0, 48.0]),
                            ..Default::default()
                        }));
                        frame.parent = Some(page);
                        let frame_id = frame.id;
                        document.doc.scene.insert(frame)?;
                        for _ in 0..PAGE_LISTING_NODE_BUDGET {
                            let mut child = CanvasNode::new(NodeData::Group(GroupNode::default()));
                            child.parent = Some(frame_id);
                            child.index = document.doc.scene.next_child_index(Some(frame_id));
                            document.doc.scene.insert(child)?;
                        }
                        Ok::<_, anyhow::Error>(frame_id)
                    })();
                    (result, DocChange::Content)
                })
                .context("document")?
            })?;
            let scene = item.read_with(cx, |item, _| page_scene(item, u32::MAX, 50))?;
            assert_eq!(scene.len(), 1);
            let listed = scene.first().context("frame summary")?;
            assert_eq!(listed["id"], frame.to_string());
            assert_eq!(listed["child_count"], PAGE_LISTING_NODE_BUDGET);
            assert!(listed.get("children").is_none());
            let explicit = cx.update(|cx| {
                surface.get_nodes(
                    NodeQuery {
                        depth: Some(2),
                        limit: Some(1),
                        ..Default::default()
                    },
                    cx,
                )
            });
            assert!(
                explicit.is_err(),
                "an explicit over-budget query remains an error"
            );
            Ok::<_, anyhow::Error>(())
        }
        .await
        .expect("the planner can read dense imported pages safely");
    }

    #[gpui::test]
    async fn default_agent_queries_follow_authored_and_component_canvas_focus(
        cx: &mut gpui::TestAppContext,
    ) {
        async {
            let (_, _, project) = streamed_design_fixture(cx).await;
            let (mut doc, first) = doc_with_page();
            let mut second = CanvasNode::new(NodeData::Group(GroupNode::default()));
            second.name = "Second page".into();
            let second_root = second.id;
            doc.scene.insert(second)?;
            doc.add_page(second_root);
            let mut master = CanvasNode::new(NodeData::Group(GroupNode::default()));
            master.name = "Button".into();
            let master_root = master.id;
            doc.scene.insert(master)?;
            let component = ComponentId::new();
            doc.components.defs.insert(
                component,
                fanta_doc::ComponentDef::new(component, master_root, "Button"),
            );
            for (root, name, width, color) in [
                (first, "First content", 200.0, Color::WHITE),
                (second_root, "Second content", 100.0, Color::BLACK),
                (master_root, "Button content", 80.0, Color::WHITE),
            ] {
                let mut node = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
                    0.0, 0.0, width, 40.0, color,
                )));
                node.parent = Some(root);
                node.name = name.into();
                doc.scene.insert(node)?;
            }
            doc.set_active_page(Some(first));
            let item = crate::document::ready_item_for_test(
                &project,
                PathBuf::from("/tmp/Focus.fig"),
                doc,
                cx,
            );
            let surface = FigDesignSurface {
                active: Rc::new(RefCell::new(Some(item.downgrade()))),
            };
            assert_eq!(
                item.read_with(cx, |item, _| item
                    .document()
                    .and_then(|document| document.page_index(None))),
                Some(0)
            );
            for (root, name) in [
                (second_root, "Second content"),
                (master_root, "Button content"),
            ] {
                item.update(cx, |item, cx| {
                    item.with_document(cx, |document| {
                        document.doc.set_active_page(Some(root));
                        ((), DocChange::Selection)
                    })
                });
                let index = item
                    .read_with(cx, |item, _| {
                        item.document().and_then(|document| {
                            document
                                .pages
                                .iter()
                                .position(|page| page.root == Some(root))
                        })
                    })
                    .context("focused canvas index")?;
                let state = cx.update(|cx| surface.state(cx))?;
                assert_eq!(state["active_page_index"], index);
                assert_eq!(state["active_root"], root.to_string());
                assert!(
                    state["pages"]
                        .as_array()
                        .context("page overview")?
                        .iter()
                        .any(|page| page["root"] == root.to_string() && page["active"] == true)
                );
                let nodes = cx.update(|cx| surface.get_nodes(NodeQuery::default(), cx))?;
                assert_eq!(nodes["root"]["id"], root.to_string());
                assert_eq!(nodes["page"], index);
                let scene = item.read_with(cx, |item, _| page_scene(item, 8, 50))?;
                assert_eq!(scene.first().context("focused scene")?["name"], name);
                let (_, _, screenshot_root, _) = item.update(cx, |item, cx| {
                    prepare_screenshot(item, &ScreenshotTarget::default(), cx)
                })?;
                assert_eq!(screenshot_root, Some(root));
                let explicit = cx.update(|cx| {
                    surface.get_nodes(
                        NodeQuery {
                            page: Some(0),
                            ..Default::default()
                        },
                        cx,
                    )
                })?;
                assert_eq!(explicit["root"]["id"], first.to_string());
            }
            item.update(cx, |item, cx| {
                item.with_document(cx, |document| {
                    document.doc.set_active_page(None);
                    ((), DocChange::Selection)
                })
            });
            let fallback = cx.update(|cx| surface.get_nodes(NodeQuery::default(), cx))?;
            assert_eq!(fallback["root"]["id"], first.to_string());
            Ok::<_, anyhow::Error>(())
        }
        .await
        .expect("agent reads and screenshots follow the focused canvas");
    }

    #[gpui::test]
    async fn agent_screenshot_excludes_undo_payloads_and_preserves_pixels(
        cx: &mut gpui::TestAppContext,
    ) {
        async {
            let (item, _, _) = streamed_design_fixture(cx).await;
            let result = cx.update(|cx| {
                apply_ops_to_item(
                    &item,
                    &ops(json!([
                        {"op":"create_node","node_type":"rectangle","x":0,"y":0,
                         "width":128,"height":48,"fill":"#2255EE"}
                    ])),
                    "Create export button",
                    cx,
                )
            })?;
            let node = result["created"][0]
                .as_str()
                .context("created node")?
                .parse::<NodeId>()?;
            item.update(cx, |item, cx| {
                item.with_document(cx, |document| {
                    document.doc.selection.replace_with([node]);
                    ((), DocChange::Selection)
                })
            });
            let target = ScreenshotTarget {
                max_dimension: Some(64),
                ..Default::default()
            };
            let original = item.read_with(cx, |item, _| {
                let doc = item.doc().context("document")?;
                render_surface_screenshot(doc, None, doc.active_page(), None, target.clone())
            })?;
            let (snapshot, assets, root, selected_node) =
                item.update(cx, |item, cx| prepare_screenshot(item, &target, cx))?;
            assert_eq!(snapshot.history.undo_depth(), 0);
            assert_eq!(snapshot.history.redo_depth(), 0);
            assert!(snapshot.selection.is_empty());
            assert_eq!(
                render_surface_screenshot(&snapshot, assets, root, selected_node, target)?,
                original
            );
            item.read_with(cx, |item, _| {
                let doc = item.doc().expect("live document");
                assert_eq!(doc.history.undo_depth(), 1);
                assert_eq!(
                    doc.selection.iter().copied().collect::<Vec<_>>(),
                    vec![node]
                );
            });
            Ok::<_, anyhow::Error>(())
        }
        .await
        .expect("agent screenshots preserve rendering without copying undo state");
    }

    /// The cloud engine renders through `fanta_headless`. A project it opens
    /// must render exactly as the editor's screenshot tool renders the same
    /// files after the editor opens them.
    /// The engine applies a batch through the editor's applier, so the same
    /// ops leave both documents drawing the same pixels.
    #[test]
    fn headless_engine_applies_ops_like_the_editor() -> Result<()> {
        let mut doc = Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "Home".into();
        let page_id = page.id;
        doc.apply(Operation::create_node(page))?;
        doc.add_page(page_id);
        let directory = tempfile::tempdir()?;
        fanta_format::write_project_tree(directory.path(), &doc, &Default::default())?;
        let batch = ops(json!([
            {"op": "create_node", "node_type": "frame", "name": "Row",
             "x": 20, "y": 20, "width": 160, "height": 60},
            {"op": "create_image", "source": tiny_png_base64(), "x": 40, "y": 100, "width": 30},
            {"op": "create_node", "node_type": "text", "text": "Shared",
             "x": 200, "y": 20, "width": 120, "height": 30},
        ]));

        let mut editor = crate::document::load_project_document(directory.path())?;
        let editor_result = {
            let (doc, mut assets) = editor.doc_and_assets();
            let default_container = doc.active_page();
            apply_batch(doc, &mut assets, &batch, "Draft", default_container).value
        };
        editor.resolve_after_edit(None);
        let mut engine = fanta_headless::HeadlessProject::open(directory.path())?;
        let engine_result = engine.apply(None, "Draft", &batch, true)?;
        assert_eq!(editor_result["applied"], json!(true), "{editor_result}");
        assert_eq!(engine_result["applied"], json!(true), "{engine_result}");

        let editor_png = render_surface_screenshot(
            &editor.doc,
            editor.asset_resolver.clone(),
            Some(page_id),
            None,
            ScreenshotTarget {
                max_dimension: Some(400),
                ..Default::default()
            },
        )?;
        let engine_png = engine
            .render(&fanta_headless::RenderTarget {
                id: "page".into(),
                page: fanta_headless::PageRef::Index { index: 0 },
                node: None,
                max_dimension: 400,
                motion_clip: None,
                playhead_ms: None,
            })?
            .png;
        assert!(
            editor_png == engine_png,
            "the engine and the editor applied the batch differently"
        );
        Ok(())
    }

    #[test]
    fn headless_engine_renders_match_editor_screenshots() -> Result<()> {
        use fanta_doc::{TextNode, color::Color};
        let mut doc = Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "Home".into();
        let page_id = page.id;
        doc.apply(Operation::create_node(page))?;
        doc.add_page(page_id);
        let mut card = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            160.0,
            90.0,
            Color::BLACK,
        )));
        card.parent = Some(page_id);
        card.transform = Transform2D::translation(20.0, 10.0);
        let card_id = card.id;
        doc.apply(Operation::create_node(card))?;
        let mut label = CanvasNode::new(NodeData::Text(TextNode::new("Review me", 140.0, 24.0)));
        label.parent = Some(page_id);
        label.transform = Transform2D::translation(30.0, 40.0);
        doc.apply(Operation::create_node(label))?;
        let mut pixels = Vec::new();
        image::RgbaImage::from_pixel(8, 8, image::Rgba([220, 40, 40, 255])).write_to(
            &mut std::io::Cursor::new(&mut pixels),
            image::ImageFormat::Png,
        )?;
        let asset = fanta_format::asset_id_for_bytes(&pixels);
        let mut photo = CanvasNode::new(NodeData::Bitmap(BitmapNode {
            asset,
            natural_size: [8, 8],
            local_size: [40.0, 40.0],
            crop: None,
            fit: ImageFitMode::Fill,
            tint: None,
        }));
        photo.parent = Some(page_id);
        photo.transform = Transform2D::translation(130.0, 60.0);
        doc.apply(Operation::create_node(photo))?;
        let directory = tempfile::tempdir()?;
        fanta_format::write_project_tree(
            directory.path(),
            &doc,
            &std::collections::BTreeMap::from([(asset, pixels)]),
        )?;

        let editor = crate::document::load_project_document(directory.path())?;
        let mut engine = fanta_headless::HeadlessProject::open(directory.path())?;
        for node in [None, Some(card_id)] {
            let editor_png = render_surface_screenshot(
                &editor.doc,
                editor.asset_resolver.clone(),
                Some(page_id),
                node,
                ScreenshotTarget {
                    max_dimension: Some(400),
                    ..Default::default()
                },
            )?;
            let engine_png = engine
                .render(&fanta_headless::RenderTarget {
                    id: "page".into(),
                    page: fanta_headless::PageRef::Index { index: 0 },
                    node: node.map(|node| node.to_string()),
                    max_dimension: 400,
                    motion_clip: None,
                    playhead_ms: None,
                })?
                .png;
            assert!(
                editor_png == engine_png,
                "the engine and the editor render {node:?} differently"
            );
        }
        Ok(())
    }
}
