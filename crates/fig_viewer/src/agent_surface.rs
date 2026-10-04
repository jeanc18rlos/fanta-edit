//! The fig_viewer implementation of [`design_surface::DesignSurface`]: lets
//! the agent's native design tools read, edit, and screenshot the most
//! recently opened or focused canvas. Edits go through the same document
//! seam as user input (one history transaction per batch, rolled back on
//! failure), so agent work is undoable like any canvas gesture.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use base64::Engine as _;
use design_surface::{
    AlignContent, AlignEdge, CellAlignment, CrossAxisAlignment, DEFAULT_CHILD_LIMIT,
    DesignComponentPropertyKind, DesignEffect, DesignImageFit, DesignNodeType, DesignOp,
    DesignPaint, DesignShadow, DesignShape, DesignStrokeCap, DesignStrokeJoin, DesignSurface,
    DesignSystemQuery, DesignVariableType, DistributeAxis, GridTrackSpec, HorizontalConstraint,
    LayerPosition, LayoutDirection, LayoutSizing, MainAxisAlignment, NamedLayerPosition,
    NodeDetail, NodeQuery, ScreenshotTarget, StrokeAlignment, TextAlignment, TextSizing,
    VariableBindingProperty, VerticalConstraint,
};
use fanta_doc::{
    AssetId, AutoLayout, AxisSizing, BitmapNode, BoundProp, Bounds, CanvasNode, Color, ComponentId,
    CounterAlign, Doc, Fill, GridAlign, GridLayout, GridTrack, GroupNode, ImageFitMode, IndexKey,
    InstanceNode, LayoutMode, Mode, ModeId, ModeScope, NodeData, NodeFlags, NodeId, Operation,
    PathData, PrimaryAlign, ProjectAsset, ProjectAssetKind, Shadow, ShadowKind, Stroke,
    StrokeAlign, StrokeCap, StrokeJoin, TextAlign, TextAutoResize, TextNode, Transform2D,
    UnitInterval, VarValue, Variable, VariableCollection, VariableCollectionId, VariableId,
    VariableType, VectorNode, Viewport,
};
use fanta_render::{AssetResolver, RasterRenderer, visual_world_bounds};
use gpui::{App, AppContext as _, Entity, Global, Task, WeakEntity};
use serde_json::{Value, json};
use std::sync::Arc;
use util::ResultExt as _;

use crate::clipboard::{create_operations, duplicate_operations};
use crate::document::{
    AssetStores, DocChange, FigDocument, FigItem, FigItemEvent, FigPage, MAX_IMAGE_SOURCE_BYTES,
    PreparedImage, SaveKind, page_bounds,
};
use crate::export::render_inputs;
use crate::properties_ops::{
    DEFAULT_FILL_COLOR, blurs_operations, create_component_operations, default_shadow,
    effects_operations, parse_color, replace_data_operation, resize_operations,
    rotation_operations, set_corner_radius, stroke_list_mut,
};
use crate::structure::{frame_selection_operations, group_operations, ungroup_operations};

/// The most recently opened or focused canvas item, shared between the
/// registered provider and the [`FigView`](crate::FigView) instances that
/// update it.
struct ActiveDesignItem(Rc<RefCell<Option<WeakEntity<FigItem>>>>);

impl Global for ActiveDesignItem {}

pub(crate) fn init(cx: &mut App) {
    let active = Rc::new(RefCell::new(None));
    cx.set_global(ActiveDesignItem(active.clone()));
    design_surface::register(Rc::new(FigDesignSurface { active }), cx);
}

/// Record `item` as the canvas the agent's design tools target. Called when a
/// [`FigView`](crate::FigView) is created or focused, so the tools follow the
/// user's attention.
pub(crate) fn set_active_item(item: WeakEntity<FigItem>, cx: &mut App) {
    if let Some(state) = cx.try_global::<ActiveDesignItem>() {
        *state.0.borrow_mut() = Some(item);
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
            let outcome = apply_batch(doc, &mut assets, ops, label);
            (Ok(outcome.value), outcome.change)
        })
        .unwrap_or_else(|| Err(anyhow!("the document is no longer available")))
    })
}

/// The active page's top-level layers (up to `limit`, `depth` levels deep) in
/// the style projection with world bounds: what `batch_get` returns for a page
/// listing with `detail: "style"`, and what the v2 agent sessions read.
pub(crate) fn page_scene(item: &FigItem, depth: u32, limit: usize) -> Result<Vec<Value>> {
    let document = ready_document(item)?;
    let doc = &document.doc;
    let page = &document.pages[resolve_page_index(document, None)?];
    let root = page.root.context("the page has no root node")?;
    let mut listing = paginated_node_summary(doc, root, Some(depth), true, 0, limit);
    crate::agent_style::attach_styles(doc, &mut listing);
    Ok(match listing.get_mut("children").map(Value::take) {
        Some(Value::Array(children)) => children,
        _ => Vec::new(),
    })
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
            .collect::<Vec<_>>();
        Ok(json!({ "page": page_index, "comments": comments }))
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
            "pages": pages_json(&document.pages, doc, project_root),
            "active_page_bounds": bounds_json(content_bounds(doc, doc.active_page())),
            "components": components_json(doc, project_root),
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
        design_system_json(&document.doc, item.project_root(), query)
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
        let default_container = item.read(cx).doc().and_then(Doc::active_page);
        let batch = cx.new(|cx| {
            cx.on_release(|batch: &mut StreamedDesignBatch, cx| batch.cancel(cx))
                .detach();
            StreamedDesignBatch {
                item: item.downgrade(),
                owner: cx.entity_id(),
                progress: BatchProgress {
                    default_container,
                    ..BatchProgress::default()
                },
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
                                    .statuses
                                    .last()
                                    .and_then(|status| status.get("created"))
                                    .and_then(Value::as_str)
                                    .and_then(|id| id.parse::<NodeId>().ok())
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
                if batch.read_with(cx, |batch, _| batch.progress.failure.is_some()) {
                    break;
                }
                // Yield a painted frame for each operation without splitting the undo transaction.
                cx.background_executor()
                    .timer(Duration::from_millis(90))
                    .await;
            }
            batch.update(cx, |batch, cx| {
                item.update(cx, |item, cx| {
                    let success = batch.progress.failure.is_none();
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
                                outcome.change
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

fn parse_node_id(raw: &str) -> Result<NodeId> {
    raw.parse::<NodeId>()
        .map_err(|_| anyhow!("`{raw}` is not a valid node id"))
}

fn selection_ids(doc: &Doc) -> Vec<String> {
    doc.selection.iter().map(ToString::to_string).collect()
}

/// One entry per page, each naming the `.fnx` file that page IS — the whole
/// point of a Fanta project is that an agent edits the source, so the state
/// an agent reads has to say which file to open.
///
/// Resolving a source path scans the project's `pages/` directory (design
/// directories are slug-named, so paths cannot be derived from ids). That is
/// fine for a tool call and must never happen in a `render`.
fn pages_json(pages: &[FigPage], doc: &Doc, project_root: Option<&Path>) -> Vec<Value> {
    let active_page = doc.active_page();
    pages
        .iter()
        .enumerate()
        .map(|(index, page)| {
            let source = page
                .root
                .zip(project_root)
                .and_then(|(root, project_root)| {
                    fanta_format::locate_page_source(project_root, root)
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

/// Component masters are not pages, so their sources live in their own list
/// rather than overloading `pages`.
fn components_json(doc: &Doc, project_root: Option<&Path>) -> Vec<Value> {
    doc.components
        .defs
        .values()
        .map(|def| {
            let source = project_root
                .and_then(|project_root| fanta_format::locate_master_source(project_root, def.id));
            json!({
                "id": def.id.to_string(),
                "name": truncate_summary_string(&def.name, SUMMARY_LABEL_CHARS),
                "root": def.root.to_string(),
                "source": relative_source(project_root, source),
                "properties": def.props,
                "variant_of": def.variant_of,
            })
        })
        .collect()
}

fn design_system_json(
    doc: &Doc,
    project_root: Option<&Path>,
    query: DesignSystemQuery,
) -> Result<Value> {
    let collection = query
        .collection
        .as_deref()
        .map(|key| find_collection(doc, key).map(|collection| collection.id))
        .transpose()?;
    let variables = doc
        .variables
        .variables
        .values()
        .filter(|variable| collection.is_none_or(|id| variable.collection == id))
        .collect::<Vec<_>>();
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(100).clamp(1, 200);
    let bindings = if query.include_bindings {
        doc.selection.iter().take(200).filter_map(|id| doc.scene.get(*id)).map(|node| {
            json!({"node": node.id.to_string(), "bindings": node.bindings.iter().map(|(property, variable)| json!({"property":property,"variable":variable.to_string()})).collect::<Vec<_>>()})
        }).collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    Ok(json!({
        "collections": doc.variables.collections.values().filter(|current| collection.is_none_or(|id| current.id == id)).collect::<Vec<_>>(),
        "active_modes": doc.active_modes,
        "variables": variables.iter().skip(offset).take(limit).collect::<Vec<_>>(),
        "variable_count": variables.len(), "offset": offset, "limit": limit,
        "more_variables": offset.saturating_add(limit) < variables.len(),
        "components": components_json(doc, project_root),
        "component_sets": doc.components.sets.values().collect::<Vec<_>>(),
        "selection_bindings": bindings,
        "source_files": ["doc/variables.json", "doc/active_modes.json", "components/<set>/set.json"],
        "hints": ["Create semantic variables before components, bind properties, then reuse component instances.",
          "Names are exact and must be unique when used instead of ids. Prefer returned ids for follow-up edits.",
          "Variables use per-mode values. set_variable_mode selects a collection mode document-wide or pins a frame."]
    }))
}

fn required_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() || name.len() > 512 {
        bail!("names must contain 1–512 bytes");
    }
    Ok(name)
}

fn find_collection<'a>(doc: &'a Doc, key: &str) -> Result<&'a VariableCollection> {
    if let Ok(id) = key.parse::<VariableCollectionId>() {
        return doc
            .variables
            .collections
            .get(&id)
            .context("variable collection id does not exist");
    }
    let mut matches = doc
        .variables
        .collections
        .values()
        .filter(|collection| collection.name == key);
    let collection = matches
        .next()
        .context("variable collection name does not exist")?;
    if matches.next().is_some() {
        bail!("ambiguous collection name; use its exact id");
    }
    Ok(collection)
}

fn find_variable<'a>(doc: &'a Doc, key: &str) -> Result<&'a Variable> {
    if let Ok(id) = key.parse::<VariableId>() {
        return doc
            .variables
            .variables
            .get(&id)
            .context("variable id does not exist");
    }
    let mut matches = doc
        .variables
        .variables
        .values()
        .filter(|variable| variable.name == key);
    let variable = matches.next().context("variable name does not exist")?;
    if matches.next().is_some() {
        bail!("ambiguous variable name; use its exact id");
    }
    Ok(variable)
}

fn find_mode(collection: &VariableCollection, key: &str) -> Result<ModeId> {
    if let Ok(id) = key.parse::<ModeId>() {
        if collection.has_mode(id) {
            return Ok(id);
        }
        bail!("mode does not belong to this collection");
    }
    let mut matches = collection.modes.iter().filter(|mode| mode.name == key);
    let mode = matches
        .next()
        .context("mode name does not exist in this collection")?;
    if matches.next().is_some() {
        bail!("ambiguous mode name; use its exact id");
    }
    Ok(mode.id)
}

fn find_component_property<'a>(
    definition: &'a fanta_doc::ComponentDef,
    key: &str,
) -> Result<&'a fanta_doc::ComponentPropDef> {
    if let Ok(id) = key.parse::<fanta_doc::ComponentPropId>() {
        return definition
            .props
            .iter()
            .find(|property| property.id == id)
            .context("property id does not belong to this component");
    }
    let mut matches = definition
        .props
        .iter()
        .filter(|property| property.name == key);
    let property = matches
        .next()
        .context("component property name does not exist")?;
    if matches.next().is_some() {
        bail!("ambiguous component property name; use its exact id");
    }
    Ok(property)
}

fn variable_type(kind: DesignVariableType) -> VariableType {
    match kind {
        DesignVariableType::Color => VariableType::Color,
        DesignVariableType::Float => VariableType::Float,
        DesignVariableType::String => VariableType::String,
        DesignVariableType::Boolean => VariableType::Boolean,
        DesignVariableType::Typography => VariableType::Typography,
    }
}

fn parse_variable_value(doc: &Doc, kind: VariableType, value: &Value) -> Result<VarValue> {
    if let Some(alias) = value.get("alias").and_then(Value::as_str) {
        let target = find_variable(doc, alias)?;
        if target.ty != kind {
            bail!("alias target has a different variable type");
        }
        return Ok(VarValue::Alias {
            variable: target.id,
        });
    }
    Ok(match kind {
        VariableType::Color => VarValue::Color {
            value: parse_fill_color(value.as_str().context("color requires a hex string")?)?,
        },
        VariableType::Float => {
            let value = value
                .as_f64()
                .filter(|value| value.is_finite())
                .context("float requires a finite number")?;
            VarValue::Float { value }
        }
        VariableType::String => VarValue::String {
            value: value.as_str().context("string requires text")?.to_owned(),
        },
        VariableType::Boolean => VarValue::Boolean {
            value: value.as_bool().context("boolean requires true or false")?,
        },
        VariableType::Typography => {
            let style: fanta_doc::TextStyle = serde_json::from_value(value.clone())
                .context("typography requires a TextStyle object")?;
            if style.font_family.trim().is_empty()
                || !(style.size_px.is_finite() && style.size_px > 0.0)
                || !(style.line_height.is_finite() && style.line_height > 0.0)
                || !style.letter_spacing.is_finite()
            {
                bail!("typography requires a font family and positive finite size/line height");
            }
            VarValue::TextStyle { value: style }
        }
    })
}

fn binding_property(property: VariableBindingProperty) -> BoundProp {
    match property {
        VariableBindingProperty::FillColor { index } => BoundProp::FillColor { index },
        VariableBindingProperty::StrokeColor { index } => BoundProp::StrokeColor { index },
        VariableBindingProperty::StrokeWidth { index } => BoundProp::StrokeWidth { index },
        VariableBindingProperty::CornerRadius => BoundProp::CornerRadius,
        VariableBindingProperty::Opacity => BoundProp::Opacity,
        VariableBindingProperty::Visible => BoundProp::Visible,
        VariableBindingProperty::TextContent => BoundProp::TextContent,
        VariableBindingProperty::TextStyle => BoundProp::TextStyle,
        VariableBindingProperty::ClipWidth => BoundProp::ClipWidth,
        VariableBindingProperty::ClipHeight => BoundProp::ClipHeight,
    }
}

fn layout_sizing(sizing: LayoutSizing) -> AxisSizing {
    match sizing {
        LayoutSizing::Fixed => AxisSizing::Fixed,
        LayoutSizing::Hug => AxisSizing::Hug,
    }
}

/// A `set_grid_layout` track: px, `"<n>fr"` (or `"fr"`), `"<n>px"`, or
/// `"auto"`/`"hug"`.
fn grid_track(spec: &GridTrackSpec) -> Result<GridTrack> {
    let fixed = |size: f64| {
        if size.is_finite() && size >= 0.0 {
            Ok(GridTrack::Fixed { size })
        } else {
            Err(anyhow!("a px track must be finite and non-negative"))
        }
    };
    match spec {
        GridTrackSpec::Px(size) => fixed(*size),
        GridTrackSpec::Keyword(word) => {
            let word = word.trim().to_ascii_lowercase();
            if matches!(word.as_str(), "auto" | "hug") {
                return Ok(GridTrack::Hug);
            }
            if let Some(fr) = word.strip_suffix("fr") {
                let fr = match fr.trim() {
                    "" => 1.0,
                    fr => fr
                        .parse::<f64>()
                        .map_err(|_| anyhow!("bad fr track {word:?}"))?,
                };
                if !(fr.is_finite() && fr > 0.0) {
                    bail!("an fr track must be positive");
                }
                return Ok(GridTrack::Flex { fr });
            }
            match word
                .strip_suffix("px")
                .unwrap_or(&word)
                .trim()
                .parse::<f64>()
            {
                Ok(size) => fixed(size),
                Err(_) => {
                    bail!("unknown grid track {word:?}: use a px number, \"<n>fr\" or \"auto\"")
                }
            }
        }
    }
}

fn grid_alignment(alignment: CellAlignment) -> GridAlign {
    match alignment {
        CellAlignment::Start => GridAlign::Start,
        CellAlignment::Center => GridAlign::Center,
        CellAlignment::End => GridAlign::End,
    }
}

fn counter_alignment(alignment: CrossAxisAlignment) -> CounterAlign {
    match alignment {
        CrossAxisAlignment::Start => CounterAlign::Start,
        CrossAxisAlignment::Center => CounterAlign::Center,
        CrossAxisAlignment::End => CounterAlign::End,
        CrossAxisAlignment::Stretch => CounterAlign::Stretch,
        CrossAxisAlignment::Baseline => CounterAlign::Baseline,
    }
}

/// A located source as the project sees it — `pages/<slug>/page.fnx` — which
/// is the form an agent pastes into a file tool. `null` when the document has
/// no project on disk yet (an unsaved `.fig` import).
fn relative_source(project_root: Option<&Path>, source: Option<PathBuf>) -> Value {
    let (Some(project_root), Some(source)) = (project_root, source) else {
        return Value::Null;
    };
    match source.strip_prefix(project_root) {
        Ok(relative) => json!(relative.to_string_lossy()),
        Err(_) => json!(source.to_string_lossy()),
    }
}

pub(crate) fn world_bounds_json(doc: &Doc, id: NodeId) -> Value {
    match doc.scene.world_bounds(id) {
        Some(bounds) => json!({
            "x": bounds.min_x,
            "y": bounds.min_y,
            "width": bounds.width(),
            "height": bounds.height(),
        }),
        None => Value::Null,
    }
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

fn bounds_json(bounds: Option<Bounds>) -> Value {
    match bounds {
        Some(bounds) => json!({
            "x": bounds.min_x,
            "y": bounds.min_y,
            "width": bounds.width(),
            "height": bounds.height(),
        }),
        None => Value::Null,
    }
}

/// Union of the world bounds of `ids` (nodes without bounds are skipped).
fn union_bounds(doc: &Doc, ids: &[NodeId]) -> Option<Bounds> {
    ids.iter()
        .filter_map(|id| doc.scene.world_bounds(*id))
        .filter(Bounds::is_finite)
        .reduce(|union, bounds| union.union(&bounds))
}

/// Union of the top-level content of `page` (`None` for an empty page or a
/// document without pages).
fn content_bounds(doc: &Doc, page: Option<NodeId>) -> Option<Bounds> {
    let root = page?;
    union_bounds(doc, doc.scene.children_of(Some(root)))
}

/// Gap kept between existing content and a newly placed frame.
const EMPTY_SPACE_MARGIN: f64 = 100.0;

/// A free top-left for a `width` x `height` box on `page`: the origin on an
/// empty page, otherwise the first spot to the right of, then below, the
/// page's content (stepping further out until nothing on the page overlaps).
fn empty_space(doc: &Doc, page: NodeId, width: f64, height: f64) -> glam::DVec2 {
    let Some(content) = content_bounds(doc, Some(page)) else {
        return glam::DVec2::ZERO;
    };
    let mut candidates = Vec::new();
    for step in 1..=8 {
        let offset = EMPTY_SPACE_MARGIN * f64::from(step);
        candidates.push(glam::DVec2::new(content.max_x + offset, content.min_y));
        candidates.push(glam::DVec2::new(content.min_x, content.max_y + offset));
    }
    for candidate in &candidates {
        let rect = Bounds::from_xywh(candidate.x, candidate.y, width, height);
        if page_area_is_empty(doc, page, rect) {
            return *candidate;
        }
    }
    // The page's content is wider than eight margins of overlap allows;
    // fall back to the far right, which the union bounds guarantee is free.
    glam::DVec2::new(
        content.max_x + EMPTY_SPACE_MARGIN,
        content.max_y + EMPTY_SPACE_MARGIN,
    )
}

/// Whether nothing on `page` overlaps `rect`. The spatial index answers for
/// leaves across every page, so hits are filtered to this page's subtree;
/// empty frames (groups, which the index never reports) are checked against
/// the page's top-level children directly.
fn page_area_is_empty(doc: &Doc, page: NodeId, rect: Bounds) -> bool {
    let on_page = |id: NodeId| {
        doc.scene
            .ancestors_of(id)
            .any(|ancestor| ancestor.id == page)
    };
    let leaf_hits = doc
        .scene
        .rect_query_where(rect, |id, bounds| bounds.intersects(&rect) && on_page(id));
    if !leaf_hits.is_empty() {
        return false;
    }
    !doc.scene
        .children_of(Some(page))
        .iter()
        .filter_map(|child| doc.scene.world_bounds(*child))
        .any(|bounds| bounds.intersects(&rect))
}

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
    document
        .page_index(None)
        .context("the document has no pages")
}

/// [`node_summary`] with the listed node's direct children windowed to
/// `offset..offset + limit`, reporting the facts a caller needs to continue:
/// `child_count`, `children_offset`, `children_limit` and `more_children`.
///
/// A page of a real imported `.fig` can hold thousands of top-level nodes, so
/// the whole child list does not fit in one response — and listing is the only
/// way to discover node ids, so refusing the whole answer would leave no way
/// in. Only this top level is windowed; the depth-limited subtrees below each
/// windowed child are unchanged.
fn paginated_node_summary(
    doc: &Doc,
    id: NodeId,
    depth: Option<u32>,
    include_geometry: bool,
    offset: usize,
    limit: usize,
) -> Value {
    let mut value = node_summary(doc, id, Some(0), include_geometry);
    let Some(object) = value.as_object_mut() else {
        return value;
    };
    let children = doc.scene.children_of(Some(id));
    object.insert("child_count".into(), json!(children.len()));
    if depth == Some(0) {
        return value;
    }
    let end = offset.saturating_add(limit).min(children.len());
    let window = children.get(offset..end).unwrap_or_default();
    let child_depth = depth.map(|depth| depth - 1);
    object.insert(
        "children".into(),
        Value::Array(
            window
                .iter()
                .map(|child| node_summary(doc, *child, child_depth, include_geometry))
                .collect(),
        ),
    );
    object.insert("children_offset".into(), json!(offset));
    object.insert("children_limit".into(), json!(limit));
    object.insert("more_children".into(), json!(end < children.len()));
    value
}

/// A compact node-tree projection for page listings: enough for the model to
/// navigate and target nodes without the full serde payload (fetch specific
/// ids for that).
pub(crate) fn node_summary(
    doc: &Doc,
    id: NodeId,
    depth: Option<u32>,
    include_geometry: bool,
) -> Value {
    let Some(node) = doc.scene.get(id) else {
        return Value::Null;
    };
    let mut object = serde_json::Map::new();
    object.insert("id".into(), json!(id.to_string()));
    object.insert("kind".into(), json!(node.data.kind_tag()));
    object.insert(
        "name".into(),
        json!(truncate_summary_string(&node.name, SUMMARY_LABEL_CHARS)),
    );
    if node.flags.contains(NodeFlags::HIDDEN) {
        object.insert("hidden".into(), json!(true));
    }
    if node.flags.contains(NodeFlags::LOCKED) {
        object.insert("locked".into(), json!(true));
    }
    summarize_kind(doc, node, &mut object);
    if include_geometry {
        object.insert("world_bounds".into(), world_bounds_json(doc, id));
    }
    let children = doc.scene.children_of(Some(id));
    if !children.is_empty() {
        if depth == Some(0) {
            object.insert("child_count".into(), json!(children.len()));
        } else {
            let child_depth = depth.map(|depth| depth - 1);
            object.insert(
                "children".into(),
                Value::Array(
                    children
                        .iter()
                        .map(|child| node_summary(doc, *child, child_depth, include_geometry))
                        .collect(),
                ),
            );
        }
    }
    Value::Object(object)
}

/// Characters of a text node's content a compact listing carries; longer
/// content is cut there and reported with its full `text_length`, so a page
/// listing stays bounded per node (fetch the node by id for the whole text).
const SUMMARY_TEXT_CHARS: usize = 120;
pub(crate) const SUMMARY_LABEL_CHARS: usize = 160;

pub(crate) fn truncate_summary_string(value: &str, max_chars: usize) -> String {
    let mut characters = value.chars();
    let mut truncated = characters.by_ref().take(max_chars).collect::<String>();
    if characters.next().is_some() {
        truncated.push('…');
    }
    truncated
}

/// Kind-specific facts a model needs to reason about a node without fetching
/// it in full: a frame's size and layout mode, a text's font and (truncated)
/// content, a shape's fill, an instance's component. Only present facts are
/// emitted so large page listings stay compact.
fn summarize_kind(doc: &Doc, node: &CanvasNode, object: &mut serde_json::Map<String, Value>) {
    match &node.data {
        NodeData::Group(group) => {
            if let Some([width, height]) = group.clip_size {
                object.insert("size".into(), json!([width, height]));
            }
            if let Some(layout) = &group.auto_layout {
                let mode = match layout.mode {
                    LayoutMode::Horizontal => "horizontal",
                    LayoutMode::Vertical => "vertical",
                    LayoutMode::Grid => "grid",
                };
                object.insert("auto_layout".into(), json!(mode));
            }
            if let Some(color) = group.background.as_ref().and_then(Fill::solid_color) {
                object.insert("fill".into(), json!(color.to_hex()));
            }
        }
        NodeData::Text(text) => {
            summarize_text(&text.content, &text.style, object);
        }
        NodeData::TextPath(text_path) => {
            summarize_text(&text_path.content, &text_path.style, object);
        }
        NodeData::Vector(vector) => {
            if let Some(color) = vector.fills.first().and_then(Fill::solid_color) {
                object.insert("fill".into(), json!(color.to_hex()));
            }
        }
        NodeData::Boolean(boolean) => {
            if let Some(color) = boolean.fills.first().and_then(Fill::solid_color) {
                object.insert("fill".into(), json!(color.to_hex()));
            }
        }
        NodeData::Instance(instance) => {
            object.insert(
                "component".into(),
                json!(
                    doc.components
                        .def(instance.component)
                        .map(|def| def.name.as_str())
                        .unwrap_or("(missing)")
                ),
            );
        }
        NodeData::Bitmap(_)
        | NodeData::Video(_)
        | NodeData::Audio(_)
        | NodeData::NodeGraph(_)
        | NodeData::Model3d(_)
        | NodeData::AiArtifact(_)
        | NodeData::Embed(_) => {}
    }
}

fn summarize_text(
    content: &str,
    style: &fanta_doc::TextStyle,
    object: &mut serde_json::Map<String, Value>,
) {
    let text_length = content.chars().count();
    if text_length > SUMMARY_TEXT_CHARS {
        let preview: String = content
            .chars()
            .take(SUMMARY_TEXT_CHARS)
            .chain(std::iter::once('…'))
            .collect();
        object.insert("text".into(), json!(preview));
        object.insert("text_length".into(), json!(text_length));
    } else {
        object.insert("text".into(), json!(content));
    }
    object.insert(
        "font".into(),
        json!({
            "family": style.font_family,
            "size": style.size_px,
            "weight": style.weight,
        }),
    );
}

fn render_surface_screenshot(
    doc: &Doc,
    asset_resolver: Option<Arc<dyn AssetResolver>>,
    page_root: Option<NodeId>,
    node: Option<NodeId>,
    target: ScreenshotTarget,
) -> Result<Vec<u8>> {
    // Larger screenshots consume the gateway's request budget without adding
    // detail after the model's image downscaling.
    let max_dimension = f64::from(target.max_dimension.unwrap_or(768).clamp(16, 1568));
    let motion = match target.motion_clip {
        Some(raw) => {
            let clip = raw
                .parse::<fanta_doc::AnimationClipId>()
                .map_err(|_| anyhow!("{raw} is not a valid animation clip id"))?;
            Some(
                doc.motion
                    .evaluate(clip, target.playhead_ms.unwrap_or(0))
                    .with_context(|| format!("animation clip {raw} does not exist"))?,
            )
        }
        None if target.playhead_ms.is_some() => bail!("playhead_ms requires motion_clip"),
        None => None,
    };
    let motion_scene = motion.as_ref().map(|motion| {
        let mut scene = doc.scene.clone();
        let affected = motion
            .overrides
            .keys()
            .map(|target| target.node)
            .collect::<HashSet<_>>();
        for id in affected {
            if let Some(node) = scene.get_mut(id) {
                *node = motion.apply_to_node(node);
            }
        }
        scene
    });
    let bounds_scene = motion_scene.as_ref().unwrap_or(&doc.scene);
    let bounds = match node {
        Some(node) => visual_world_bounds(bounds_scene, node, 0.0)
            .context("the node has no visible bounds")?,
        None if motion_scene.is_some() => page_root
            .and_then(|root| visual_world_bounds(bounds_scene, root, 0.0))
            .unwrap_or_else(|| page_bounds(doc, page_root)),
        None => page_bounds(doc, page_root),
    };
    if !bounds.is_finite() || bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        bail!("the screenshot target has invalid or empty bounds");
    }
    // Fit within the dimension cap; allow mild upscale so small nodes
    // (icons) stay legible without ballooning the surface.
    let zoom = (max_dimension / bounds.width().max(bounds.height())).min(2.0);
    let width = (bounds.width() * zoom).ceil().max(1.0) as u32;
    let height = (bounds.height() * zoom).ceil().max(1.0) as u32;
    let mut renderer = RasterRenderer::new(width, height)
        .map_err(|error| anyhow!("creating {width}x{height} screenshot surface: {error}"))?;
    if let Some(asset_resolver) = asset_resolver {
        renderer.set_asset_resolver(asset_resolver);
    }
    let center = bounds.center();
    let viewport = Viewport {
        center: [center.x, center.y],
        zoom,
    };
    let mut inputs = render_inputs(doc);
    inputs.motion = motion.as_ref();
    renderer.render_page_with(&doc.scene, &viewport, page_root, &inputs);
    renderer
        .encode_png()
        .map_err(|error| anyhow!("encoding screenshot PNG: {error}"))
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
        document.doc.clone(),
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

struct BatchOutcome {
    value: Value,
    change: DocChange,
}

/// What one applied op affected, for dirty/selection tracking and reporting.
enum Applied {
    Content {
        created: Option<String>,
        /// Extra op-specific facts merged into the op's status entry (e.g.
        /// the children an `ungroup` freed).
        detail: Option<Value>,
    },
    Selection,
    Viewport,
    Nothing,
}

impl Applied {
    fn created(id: NodeId) -> Self {
        Self::Content {
            created: Some(id.to_string()),
            detail: None,
        }
    }

    fn changed() -> Self {
        Self::Content {
            created: None,
            detail: None,
        }
    }
}

/// Apply the batch inside one history transaction: all content ops commit as
/// a single undo step, and any failure rolls the content back (selection and
/// viewport changes are not transactional). Image assets ingested by
/// `create_image` ops are removed again when the batch rolls back, and nodes
/// the batch removed leave the selection only once it has committed, so a
/// rolled-back batch leaves the selection exactly as it found it.
fn apply_batch(
    doc: &mut Doc,
    assets: &mut AssetStores<'_>,
    ops: &[DesignOp],
    label: &str,
) -> BatchOutcome {
    let mut progress = BatchProgress {
        default_container: doc.active_page(),
        ..BatchProgress::default()
    };
    doc.history.begin(label, &mut doc.scene);
    for (index, operation) in ops.iter().enumerate() {
        progress.apply(doc, assets, index, operation);
        if progress.failure.is_some() {
            break;
        }
    }
    progress.finish(doc, assets, ops.len())
}

#[derive(Default)]
struct BatchProgress {
    default_container: Option<NodeId>,
    created: Vec<String>,
    statuses: Vec<Value>,
    ingested_assets: Vec<AssetId>,
    removed_nodes: HashSet<NodeId>,
    content_changed: bool,
    selection_changed: bool,
    failure: Option<(usize, String)>,
}

impl BatchProgress {
    fn apply(
        &mut self,
        doc: &mut Doc,
        assets: &mut AssetStores<'_>,
        index: usize,
        operation: &DesignOp,
    ) {
        match apply_one(
            doc,
            assets,
            &mut self.ingested_assets,
            &mut self.removed_nodes,
            operation,
            self.default_container,
        ) {
            Ok(Applied::Content { created, detail }) => {
                self.content_changed = true;
                let mut status = json!({ "index": index, "status": "ok" });
                if let Some(id) = created {
                    status["created"] = json!(id);
                    self.created.push(id);
                }
                if let (Some(Value::Object(detail)), Some(status)) =
                    (detail, status.as_object_mut())
                {
                    status.extend(detail);
                }
                self.statuses.push(status);
            }
            Ok(Applied::Selection) => {
                self.selection_changed = true;
                self.statuses
                    .push(json!({ "index": index, "status": "ok" }));
            }
            Ok(Applied::Viewport | Applied::Nothing) => {
                self.statuses
                    .push(json!({ "index": index, "status": "ok" }));
            }
            Err(error) => self.failure = Some((index, format!("{error:#}"))),
        }
    }

    fn finish(
        &mut self,
        doc: &mut Doc,
        assets: &mut AssetStores<'_>,
        operation_count: usize,
    ) -> BatchOutcome {
        match self.failure.take() {
            None => {
                doc.history.commit(&mut doc.scene);
                if doc
                    .selection
                    .iter()
                    .any(|selected| self.removed_nodes.contains(selected))
                {
                    let surviving: Vec<NodeId> = doc
                        .selection
                        .iter()
                        .copied()
                        .filter(|selected| !self.removed_nodes.contains(selected))
                        .collect();
                    doc.selection.replace_with(surviving);
                }
                BatchOutcome {
                    value: json!({ "applied": true, "created": self.created, "ops": self.statuses }),
                    change: if self.content_changed {
                        DocChange::Content
                    } else if self.selection_changed {
                        DocChange::Selection
                    } else {
                        DocChange::None
                    },
                }
            }
            Some((index, mut message)) => {
                if let Err(abort_error) = doc.abort_transaction() {
                    message = format!("{message}; rolling back also failed: {abort_error}");
                }
                for asset in self.ingested_assets.drain(..) {
                    assets.remove(asset);
                }
                self.statuses
                    .push(json!({ "index": index, "status": "failed", "error": message }));
                for skipped in index + 1..operation_count {
                    self.statuses
                        .push(json!({ "index": skipped, "status": "skipped" }));
                }
                BatchOutcome {
                    value: json!({ "applied": false,
                        "error": format!("op {index} failed; the batch was rolled back"), "ops": self.statuses }),
                    change: if self.selection_changed {
                        DocChange::Selection
                    } else {
                        DocChange::None
                    },
                }
            }
        }
    }
}

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
                        for asset in self.progress.ingested_assets.drain(..) {
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

fn refresh_streamed_layout(document: &mut FigDocument) {
    if let Some(root) = document.doc.active_page() {
        document.solved_pages.remove(&root);
        document.ensure_root_solved(root);
    }
    document.mark_variables_changed();
}

fn design_operation_is_variable(operation: &DesignOp) -> bool {
    matches!(
        operation,
        DesignOp::CreateVariableCollection { .. }
            | DesignOp::AddVariableMode { .. }
            | DesignOp::CreateVariable { .. }
            | DesignOp::SetVariableValue { .. }
            | DesignOp::SetVariableMode { .. }
    )
}

fn design_operation_node(doc: &Doc, operation: &DesignOp) -> Option<NodeId> {
    let id = match operation {
        DesignOp::SetProps { id, .. }
        | DesignOp::SetStroke { id, .. }
        | DesignOp::SetShadow { id, .. }
        | DesignOp::SetTextStyle { id, .. }
        | DesignOp::SetAutoLayout { id, .. }
        | DesignOp::SetLayoutChild { id, .. }
        | DesignOp::BindVariable { id, .. }
        | DesignOp::UnbindVariable { id, .. }
        | DesignOp::SetIndex { id, .. }
        | DesignOp::Rotate { id, .. }
        | DesignOp::Ungroup { id }
        | DesignOp::Duplicate { id, .. }
        | DesignOp::CreateComponent { id }
        | DesignOp::BindComponentProperty { id, .. }
        | DesignOp::SetInstanceProperty { id, .. }
        | DesignOp::Reparent { id, .. }
        | DesignOp::Delete { id } => Some(id),
        DesignOp::Align { ids, .. }
        | DesignOp::Distribute { ids, .. }
        | DesignOp::Group { ids, .. }
        | DesignOp::FrameSelection { ids, .. }
        | DesignOp::CombineVariants { ids, .. }
        | DesignOp::Componentize { ids }
        | DesignOp::Select { ids } => ids.first(),
        DesignOp::CreateComponentProperty { component, .. } => {
            return resolve_component(doc, component)
                .ok()
                .and_then(|component| doc.components.def(component))
                .map(|definition| definition.root);
        }
        _ => None,
    };
    id.and_then(|id| id.parse::<NodeId>().ok())
        .filter(|id| doc.scene.get(*id).is_some())
}

/// Apply one op. Assets it ingests go into `ingested_assets` and nodes it
/// removes from the scene into `removed_nodes`, both for [`apply_batch`] to
/// settle once the whole batch has committed or rolled back.
fn apply_one(
    doc: &mut Doc,
    assets: &mut AssetStores<'_>,
    ingested_assets: &mut Vec<AssetId>,
    removed_nodes: &mut HashSet<NodeId>,
    op: &DesignOp,
    default_container: Option<NodeId>,
) -> Result<Applied> {
    match op {
        DesignOp::CreateImage {
            source,
            parent,
            name,
            x,
            y,
            width,
            height,
            meta,
        } => {
            let parent = resolve_container(doc, parent.as_deref(), default_container)?;
            let bytes = decode_image_source(source)?;
            let (asset, natural_size, inserted) = assets.add_image_tracked(bytes)?;
            // Track before any fallible step so a later failure in this batch
            // rolls the asset back out of the stores too.
            if inserted {
                ingested_assets.push(asset);
            }

            let natural_width = f64::from(natural_size[0].max(1));
            let natural_height = f64::from(natural_size[1].max(1));
            let (width, height) = match (width, height) {
                (Some(width), Some(height)) => (*width, *height),
                (Some(width), None) => (*width, width * natural_height / natural_width),
                (None, Some(height)) => (height * natural_width / natural_height, *height),
                (None, None) => (natural_width, natural_height),
            };
            if !(width.is_finite() && width > 0.0 && height.is_finite() && height > 0.0) {
                bail!("width and height must be positive");
            }

            let mut node = CanvasNode::new(NodeData::Bitmap(BitmapNode {
                asset,
                natural_size,
                local_size: [width, height],
                crop: None,
                fit: ImageFitMode::Fill,
                tint: None,
            }));
            if let Some(name) = name {
                node.name = name.clone();
            }
            let parent_world = parent
                .and_then(|parent| doc.scene.world_transform(parent))
                .unwrap_or(Transform2D::IDENTITY);
            node.parent = parent;
            node.index = doc.scene.next_child_index(parent);
            node.transform = Transform2D::translation(*x, *y).then(&parent_world.inverse());
            let id = node.id;
            doc.apply(Operation::create_node(node))
                .context("creating the image node")?;
            if let Some(meta) = meta
                && !meta.is_null()
            {
                doc.apply(Operation::SetMeta {
                    id,
                    old: Value::Null,
                    new: meta.clone(),
                })
                .context("recording the image node's metadata")?;
            }
            Ok(Applied::created(id))
        }
        DesignOp::CreateNode {
            node_type,
            parent,
            name,
            x,
            y,
            width,
            height,
            fill,
            text,
            font_size,
        } => {
            if !(width.is_finite() && *width > 0.0 && height.is_finite() && *height > 0.0) {
                bail!("width and height must be positive");
            }
            let parent = resolve_container(doc, parent.as_deref(), default_container)?;
            let fill_color = fill.as_deref().map(parse_fill_color).transpose()?;
            let (data, origin) = match node_type {
                DesignNodeType::Frame => (
                    NodeData::Group(GroupNode {
                        clip_size: Some([*width, *height]),
                        background: Some(Fill::solid(fill_color.unwrap_or(Color::WHITE))),
                        ..GroupNode::default()
                    }),
                    (*x, *y),
                ),
                DesignNodeType::Rectangle => (
                    NodeData::Vector(VectorNode::rect_solid(
                        0.0,
                        0.0,
                        *width,
                        *height,
                        fill_color.unwrap_or(DEFAULT_FILL_COLOR),
                    )),
                    (*x, *y),
                ),
                DesignNodeType::Ellipse => {
                    let mut vector = VectorNode {
                        path: PathData::ellipse(0.0, 0.0, width * 0.5, height * 0.5),
                        ..VectorNode::default()
                    };
                    vector
                        .fills
                        .push(Fill::solid(fill_color.unwrap_or(DEFAULT_FILL_COLOR)));
                    // The path is centered on the local origin, so the node
                    // transform points at the ellipse's center.
                    (
                        NodeData::Vector(vector),
                        (x + width * 0.5, y + height * 0.5),
                    )
                }
                DesignNodeType::Text => {
                    let mut text_node = TextNode::new(
                        text.clone().unwrap_or_else(|| "Text".to_string()),
                        *width,
                        *height,
                    );
                    if let Some(size) = font_size {
                        if !(size.is_finite() && *size > 0.0) {
                            bail!("font_size must be positive");
                        }
                        text_node.style.size_px = f64::from(*size);
                    }
                    if let Some(color) = fill_color {
                        text_node.style.color = color;
                    }
                    (NodeData::Text(text_node), (*x, *y))
                }
            };
            let mut node = CanvasNode::new(data);
            if let Some(name) = name {
                node.name = name.clone();
            }
            // Rebase the desired world position into the parent's space, the
            // same way the canvas creation tools place new nodes.
            let parent_world = parent
                .and_then(|parent| doc.scene.world_transform(parent))
                .unwrap_or(Transform2D::IDENTITY);
            node.parent = parent;
            node.index = doc.scene.next_child_index(parent);
            node.transform =
                Transform2D::translation(origin.0, origin.1).then(&parent_world.inverse());
            let id = node.id;
            doc.apply(Operation::create_node(node))
                .context("creating the node")?;
            Ok(Applied::created(id))
        }
        DesignOp::SetProps {
            id,
            name,
            x,
            y,
            width,
            height,
            opacity,
            fill,
            corner_radius,
            text,
            hidden,
            locked,
        } => {
            let id = parse_node_id(id)?;
            let mut applied_any = false;
            let node = |doc: &Doc| {
                doc.scene
                    .get(id)
                    .with_context(|| format!("node {id} does not exist"))
                    .cloned()
            };
            node(doc)?;

            if let Some(name) = name {
                let old = node(doc)?.name;
                if *name != old {
                    doc.apply(Operation::SetName {
                        id,
                        old,
                        new: name.clone(),
                    })?;
                    applied_any = true;
                }
            }
            if let Some(text) = text {
                let NodeData::Text(_) = node(doc)?.data else {
                    bail!("node {id} is not a text node");
                };
                for operation in replace_data_operation(doc, id, |data| {
                    if let NodeData::Text(text_node) = data {
                        text_node.content = text.clone();
                    }
                }) {
                    doc.apply(operation)?;
                    applied_any = true;
                }
            }
            if let Some(fill) = fill {
                let color = parse_fill_color(fill)?;
                if !matches!(
                    node(doc)?.data,
                    NodeData::Vector(_)
                        | NodeData::Group(_)
                        | NodeData::Boolean(_)
                        | NodeData::Text(_)
                ) {
                    bail!("cannot set a fill on a {} node", node(doc)?.data.kind_tag());
                }
                for operation in replace_data_operation(doc, id, |data| set_solid_fill(data, color))
                {
                    doc.apply(operation)?;
                    applied_any = true;
                }
            }
            if let Some(radius) = corner_radius {
                if !(radius.is_finite() && *radius >= 0.0) {
                    bail!("corner_radius must be non-negative");
                }
                if !matches!(node(doc)?.data, NodeData::Vector(_) | NodeData::Group(_)) {
                    bail!(
                        "corner_radius only applies to rectangles and frames, not a {} node",
                        node(doc)?.data.kind_tag()
                    );
                }
                for operation in
                    replace_data_operation(doc, id, |data| set_corner_radius(data, *radius))
                {
                    doc.apply(operation)?;
                    applied_any = true;
                }
            }
            if let Some(opacity) = opacity {
                let old = node(doc)?.opacity;
                let new = UnitInterval::new(*opacity);
                if new != old {
                    doc.apply(Operation::SetOpacity { id, old, new })?;
                    applied_any = true;
                }
            }
            if hidden.is_some() || locked.is_some() {
                let old = node(doc)?.flags;
                let mut new = old;
                if let Some(hidden) = hidden {
                    new.set(NodeFlags::HIDDEN, *hidden);
                }
                if let Some(locked) = locked {
                    new.set(NodeFlags::LOCKED, *locked);
                }
                if new != old {
                    doc.apply(Operation::SetFlags { id, old, new })?;
                    applied_any = true;
                }
            }
            if let Some(width) = width {
                if !(width.is_finite() && *width > 0.0) {
                    bail!("width must be positive");
                }
                let operations = resize_operations(doc, id, *width, true);
                if operations.is_empty() {
                    bail!("node {id} cannot be resized");
                }
                for operation in operations {
                    doc.apply(operation)?;
                    applied_any = true;
                }
            }
            if let Some(height) = height {
                if !(height.is_finite() && *height > 0.0) {
                    bail!("height must be positive");
                }
                let operations = resize_operations(doc, id, *height, false);
                if operations.is_empty() {
                    bail!("node {id} cannot be resized");
                }
                for operation in operations {
                    doc.apply(operation)?;
                    applied_any = true;
                }
            }
            if x.is_some() || y.is_some() {
                // `x`/`y` address the node's world bounds origin (matching
                // `create_node`), so shift the world transform by the delta.
                let bounds = doc
                    .scene
                    .world_bounds(id)
                    .with_context(|| format!("node {id} has no bounds to position"))?;
                let world = doc
                    .scene
                    .world_transform(id)
                    .with_context(|| format!("node {id} has no world transform"))?;
                let delta_x = x.map(|x| x - bounds.min_x).unwrap_or(0.0);
                let delta_y = y.map(|y| y - bounds.min_y).unwrap_or(0.0);
                if !(delta_x.is_finite() && delta_y.is_finite()) {
                    bail!("x and y must be finite");
                }
                if delta_x != 0.0 || delta_y != 0.0 {
                    let current = node(doc)?;
                    let parent_world = current
                        .parent
                        .and_then(|parent| doc.scene.world_transform(parent))
                        .unwrap_or(Transform2D::IDENTITY);
                    let new_local = world
                        .then(&Transform2D::translation(delta_x, delta_y))
                        .then(&parent_world.inverse());
                    doc.apply(Operation::SetTransform {
                        id,
                        old: current.transform,
                        new: new_local,
                    })?;
                    applied_any = true;
                }
            }

            if applied_any {
                Ok(Applied::changed())
            } else {
                Ok(Applied::Nothing)
            }
        }
        DesignOp::CreateInstance {
            component,
            x,
            y,
            parent,
            name,
        } => {
            if !(x.is_finite() && y.is_finite()) {
                bail!("x and y must be finite");
            }
            let parent = resolve_container(doc, parent.as_deref(), default_container)?;
            let component_id = resolve_component(doc, component)?;
            let def = doc
                .components
                .def(component_id)
                .or_else(|| {
                    doc.components
                        .sets
                        .get(&component_id)
                        .and_then(|set| doc.components.def(set.default_variant))
                })
                .with_context(|| format!("component {component} does not exist"))?;
            let masters = doc
                .components
                .sets
                .get(&component_id)
                .map(|set| {
                    set.members
                        .iter()
                        .filter_map(|id| doc.components.def(*id))
                        .map(|definition| definition.root)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|| vec![def.root]);
            if parent.is_some_and(|parent| {
                masters.iter().any(|root| {
                    *root == parent
                        || doc
                            .scene
                            .ancestors_of(parent)
                            .any(|ancestor| ancestor.id == *root)
                })
            }) {
                bail!("cannot place an instance of a component inside its own master");
            }
            let local_size = master_size(doc, def.root).unwrap_or([100.0, 100.0]);
            let mut node = CanvasNode::new(NodeData::Instance(InstanceNode {
                component: component_id,
                overrides: Vec::new(),
                prop_values: Default::default(),
                derived: Vec::new(),
                local_size,
            }));
            node.name = name.clone().unwrap_or_else(|| def.name.clone());
            let parent_world = parent
                .and_then(|parent| doc.scene.world_transform(parent))
                .unwrap_or(Transform2D::IDENTITY);
            node.parent = parent;
            node.index = doc.scene.next_child_index(parent);
            node.transform = Transform2D::translation(*x, *y).then(&parent_world.inverse());
            let id = node.id;
            doc.apply(Operation::CreateInstance {
                node: Box::new(node),
            })
            .context("creating the instance")?;
            Ok(Applied::created(id))
        }
        DesignOp::SetStroke {
            id,
            color,
            width,
            align,
            sides,
            dash,
            cap,
            join,
        } => {
            let id = parse_node_id(id)?;
            let node = existing_node(doc, id)?;
            if !matches!(node.data, NodeData::Vector(_) | NodeData::Group(_)) {
                bail!(
                    "strokes apply to shapes and frames, not a {} node",
                    node.data.kind_tag()
                );
            }
            if let Some(width) = width
                && !(width.is_finite() && *width >= 0.0)
            {
                bail!("the stroke width must be non-negative");
            }
            let color = color.as_deref().map(parse_fill_color).transpose()?;
            let align = align.map(|align| match align {
                StrokeAlignment::Inside => StrokeAlign::Inside,
                StrokeAlignment::Center => StrokeAlign::Center,
                StrokeAlignment::Outside => StrokeAlign::Outside,
            });
            if let Some(Some(sides)) = sides
                && sides.iter().any(|side| !(side.is_finite() && *side >= 0.0))
            {
                bail!("per-side stroke widths must be non-negative");
            }
            if let Some(dash) = dash
                && dash
                    .iter()
                    .any(|length| !(length.is_finite() && *length >= 0.0))
            {
                bail!("dash lengths must be non-negative");
            }
            let cap = cap.map(|cap| match cap {
                DesignStrokeCap::Butt => StrokeCap::Butt,
                DesignStrokeCap::Round => StrokeCap::Round,
                DesignStrokeCap::Square => StrokeCap::Square,
            });
            let join = join.map(|join| match join {
                DesignStrokeJoin::Miter => StrokeJoin::Miter,
                DesignStrokeJoin::Round => StrokeJoin::Round,
                DesignStrokeJoin::Bevel => StrokeJoin::Bevel,
            });
            apply_all(
                doc,
                replace_data_operation(doc, id, |data| {
                    let Some(strokes) = stroke_list_mut(data) else {
                        return;
                    };
                    if *width == Some(0.0) {
                        strokes.clear();
                        return;
                    }
                    if strokes.is_empty() {
                        strokes.push(Stroke::solid(
                            color.unwrap_or(Color::BLACK),
                            width.unwrap_or(1.0),
                        ));
                    }
                    let Some(stroke) = strokes.first_mut() else {
                        return;
                    };
                    if let Some(color) = color {
                        stroke.paint.set_solid_color(color);
                    }
                    if let Some(width) = width {
                        stroke.width = *width;
                    }
                    if let Some(align) = align {
                        stroke.align = align;
                    }
                    if let Some(sides) = sides {
                        stroke.per_side = *sides;
                    }
                    if let Some(dash) = dash {
                        stroke.dash = dash.clone();
                    }
                    if let Some(cap) = cap {
                        stroke.cap = cap;
                    }
                    if let Some(join) = join {
                        stroke.join = join;
                    }
                }),
            )
        }
        DesignOp::SetEffects { id, effects } => {
            let id = parse_node_id(id)?;
            existing_node(doc, id)?;
            let mut shadows = Vec::new();
            let mut blurs = Vec::new();
            for effect in effects {
                match effect {
                    DesignEffect::DropShadow(shadow) => {
                        shadows.push(design_shadow(shadow, ShadowKind::Drop)?)
                    }
                    DesignEffect::InnerShadow(shadow) => {
                        shadows.push(design_shadow(shadow, ShadowKind::Inner)?)
                    }
                    DesignEffect::LayerBlur { radius }
                    | DesignEffect::BackgroundBlur { radius } => {
                        if !(radius.is_finite() && *radius >= 0.0) {
                            bail!("the blur radius must be non-negative");
                        }
                        let kind = if matches!(effect, DesignEffect::LayerBlur { .. }) {
                            fanta_doc::BlurKind::Layer
                        } else {
                            fanta_doc::BlurKind::Background
                        };
                        blurs.push(fanta_doc::Blur {
                            kind,
                            radius: *radius,
                        });
                    }
                }
            }
            let mut operations = effects_operations(doc, id, |effects| {
                *effects = shadows.into_iter().collect();
            });
            operations.extend(blurs_operations(doc, id, |current| {
                *current = blurs.into_iter().collect();
            }));
            apply_all(doc, operations)
        }
        DesignOp::SetFill { id, paints } => {
            let id = parse_node_id(id)?;
            let node = existing_node(doc, id)?;
            if !matches!(
                node.data,
                NodeData::Vector(_) | NodeData::Boolean(_) | NodeData::Group(_)
            ) {
                bail!(
                    "fills apply to frames and shapes, not a {} node (text color is set_text_style)",
                    node.data.kind_tag()
                );
            }
            let mut fills = Vec::with_capacity(paints.len());
            for paint in paints {
                fills.push(design_fill(paint, assets, ingested_assets)?);
            }
            apply_all(
                doc,
                replace_data_operation(doc, id, |data| match data {
                    NodeData::Vector(vector) => vector.fills = fills.into_iter().collect(),
                    NodeData::Boolean(boolean) => boolean.fills = fills.into_iter().collect(),
                    NodeData::Group(group) => {
                        let mut fills = fills.into_iter();
                        group.background = fills.next();
                        group.background_fills = fills.collect();
                    }
                    _ => {}
                }),
            )
        }
        DesignOp::SetConstraints {
            id,
            horizontal,
            vertical,
        } => {
            let id = parse_node_id(id)?;
            let node = existing_node(doc, id)?;
            let old = node.constraints;
            let mut new = old.unwrap_or_default();
            if let Some(horizontal) = horizontal {
                new.horizontal = match horizontal {
                    HorizontalConstraint::Left => fanta_doc::ConstraintH::Left,
                    HorizontalConstraint::Right => fanta_doc::ConstraintH::Right,
                    HorizontalConstraint::LeftRight => fanta_doc::ConstraintH::LeftRight,
                    HorizontalConstraint::Center => fanta_doc::ConstraintH::Center,
                    HorizontalConstraint::Scale => fanta_doc::ConstraintH::Scale,
                };
            }
            if let Some(vertical) = vertical {
                new.vertical = match vertical {
                    VerticalConstraint::Top => fanta_doc::ConstraintV::Top,
                    VerticalConstraint::Bottom => fanta_doc::ConstraintV::Bottom,
                    VerticalConstraint::TopBottom => fanta_doc::ConstraintV::TopBottom,
                    VerticalConstraint::Center => fanta_doc::ConstraintV::Center,
                    VerticalConstraint::Scale => fanta_doc::ConstraintV::Scale,
                };
            }
            apply_all(
                doc,
                vec![Operation::SetConstraints {
                    id,
                    old,
                    new: Some(new),
                }],
            )
        }
        DesignOp::CreateShape {
            shape,
            parent,
            name,
            x,
            y,
            width,
            height,
            points,
            inner_ratio,
            path,
            fill,
            stroke,
            stroke_width,
        } => {
            if ![*x, *y, *width, *height]
                .iter()
                .all(|value| value.is_finite())
                || *width < 0.0
                || *height < 0.0
            {
                bail!("x, y, width and height must be finite and the size non-negative");
            }
            let parent = resolve_container(doc, parent.as_deref(), default_container)?;
            let (path_data, open) = match shape {
                DesignShape::Line => {
                    let mut line = PathData::new();
                    line.move_to(0.0, 0.0);
                    line.line_to(*width, *height);
                    (line, true)
                }
                DesignShape::Polygon => (
                    PathData::polygon(*width, *height, points.unwrap_or(3)),
                    false,
                ),
                DesignShape::Star => (
                    PathData::star(
                        *width,
                        *height,
                        points.unwrap_or(5),
                        inner_ratio.unwrap_or(0.382),
                    ),
                    false,
                ),
                DesignShape::Path => {
                    let d = path
                        .as_deref()
                        .context("shape \"path\" needs SVG path data in `path`")?;
                    let data = PathData::from_svg_d(d)
                        .map_err(|error| anyhow!("invalid SVG path data: {error}"))?;
                    let open = !d.trim_end().to_ascii_lowercase().ends_with('z');
                    (data, open)
                }
            };
            let mut vector = VectorNode {
                path: path_data,
                ..VectorNode::default()
            };
            let fill = fill.as_deref().map(parse_fill_color).transpose()?;
            if let Some(color) = fill.or((!open).then_some(DEFAULT_FILL_COLOR)) {
                vector.fills.push(Fill::solid(color));
            }
            let stroke_color = stroke.as_deref().map(parse_fill_color).transpose()?;
            if let Some(width) = stroke_width
                && !(width.is_finite() && *width >= 0.0)
            {
                bail!("the stroke width must be non-negative");
            }
            if let Some(color) = stroke_color.or(open.then_some(Color::BLACK)) {
                vector
                    .strokes
                    .push(Stroke::solid(color, stroke_width.unwrap_or(1.0)));
            }
            let mut node = CanvasNode::new(NodeData::Vector(vector));
            node.name = name.clone().unwrap_or_else(|| {
                match shape {
                    DesignShape::Line => "Line",
                    DesignShape::Polygon => "Polygon",
                    DesignShape::Star => "Star",
                    DesignShape::Path => "Vector",
                }
                .to_owned()
            });
            let parent_world = parent
                .and_then(|parent| doc.scene.world_transform(parent))
                .unwrap_or(Transform2D::IDENTITY);
            node.parent = parent;
            node.index = doc.scene.next_child_index(parent);
            node.transform = Transform2D::translation(*x, *y).then(&parent_world.inverse());
            let id = node.id;
            doc.apply(Operation::create_node(node))
                .context("creating the shape")?;
            Ok(Applied::created(id))
        }
        DesignOp::SetShadow {
            id,
            color,
            x,
            y,
            blur,
            spread,
            remove,
        } => {
            let id = parse_node_id(id)?;
            existing_node(doc, id)?;
            let color = color.as_deref().map(parse_fill_color).transpose()?;
            for (label, value) in [("blur", blur), ("spread", spread), ("x", x), ("y", y)] {
                if let Some(value) = value
                    && !value.is_finite()
                {
                    bail!("the shadow {label} must be finite");
                }
            }
            if let Some(blur) = blur
                && *blur < 0.0
            {
                bail!("the shadow blur must be non-negative");
            }
            apply_all(
                doc,
                effects_operations(doc, id, |effects| {
                    if *remove == Some(true) {
                        effects.retain(|shadow| shadow.kind != ShadowKind::Drop);
                        return;
                    }
                    if !effects.iter().any(|shadow| shadow.kind == ShadowKind::Drop) {
                        effects.push(default_shadow());
                    }
                    let Some(shadow) = effects
                        .iter_mut()
                        .find(|shadow| shadow.kind == ShadowKind::Drop)
                    else {
                        return;
                    };
                    if let Some(color) = color {
                        shadow.color = color;
                    }
                    if let Some(x) = x {
                        shadow.offset[0] = *x;
                    }
                    if let Some(y) = y {
                        shadow.offset[1] = *y;
                    }
                    if let Some(blur) = blur {
                        shadow.blur = *blur;
                    }
                    if let Some(spread) = spread {
                        shadow.spread = *spread;
                    }
                }),
            )
        }
        DesignOp::SetTextStyle {
            id,
            font_family,
            font_weight,
            font_size,
            line_height,
            letter_spacing,
            align,
            color,
            sizing,
        } => {
            let id = parse_node_id(id)?;
            let node = existing_node(doc, id)?;
            if !matches!(node.data, NodeData::Text(_)) {
                bail!("node {id} is not a text node");
            }
            if let Some(size) = font_size
                && !(size.is_finite() && *size > 0.0)
            {
                bail!("font_size must be positive");
            }
            if let Some(weight) = font_weight
                && !(100..=900).contains(weight)
            {
                bail!("font_weight must be between 100 and 900");
            }
            if let Some(line_height) = line_height
                && !(line_height.is_finite() && *line_height > 0.0)
            {
                bail!("line_height must be a positive multiple of the font size");
            }
            if let Some(spacing) = letter_spacing
                && !spacing.is_finite()
            {
                bail!("letter_spacing must be finite");
            }
            let color = color.as_deref().map(parse_fill_color).transpose()?;
            let align = align.map(|align| match align {
                TextAlignment::Left => TextAlign::Left,
                TextAlignment::Center => TextAlign::Center,
                TextAlignment::Right => TextAlign::Right,
                TextAlignment::Justify => TextAlign::Justify,
            });
            apply_all(
                doc,
                replace_data_operation(doc, id, |data| {
                    let NodeData::Text(text) = data else {
                        return;
                    };
                    // Rich-text runs override the base style, so a whole-node
                    // change has to land on every run too or it would show on
                    // none of the styled characters.
                    let mut styles = vec![&mut text.style];
                    styles.extend(text.style_runs.iter_mut().map(|run| &mut run.style));
                    for style in styles {
                        if let Some(family) = font_family {
                            style.font_family = family.clone();
                        }
                        if let Some(weight) = font_weight {
                            style.weight = *weight;
                        }
                        if let Some(size) = font_size {
                            style.size_px = *size;
                        }
                        if let Some(line_height) = line_height {
                            style.line_height = *line_height;
                            style.line_height_auto_percent = None;
                        }
                        if let Some(spacing) = letter_spacing {
                            style.letter_spacing = *spacing;
                        }
                        if let Some(color) = color {
                            style.color = color;
                        }
                    }
                    if let Some(align) = align {
                        text.align = align;
                    }
                    if let Some(sizing) = sizing {
                        text.auto_resize = match sizing {
                            TextSizing::Fixed => TextAutoResize::None,
                            TextSizing::AutoHeight => TextAutoResize::Height,
                            TextSizing::AutoWidth => TextAutoResize::WidthAndHeight,
                        };
                    }
                }),
            )
        }
        DesignOp::SetAutoLayout {
            id,
            direction,
            gap,
            padding,
            align_items,
            justify,
            primary_sizing,
            counter_sizing,
            min_size,
            max_size,
            wrap,
            counter_gap,
            align_content,
        } => {
            let id = parse_node_id(id)?;
            let node = existing_node(doc, id)?;
            if !matches!(node.data, NodeData::Group(_)) {
                bail!(
                    "auto layout applies to frames and groups, not a {} node",
                    node.data.kind_tag()
                );
            }
            let child_count = doc.scene.children_of(Some(id)).len();
            if let Some(gap) = gap
                && !(gap.is_finite() && *gap >= 0.0)
            {
                bail!("gap must be non-negative");
            }
            let padding = padding.as_deref().map(parse_padding).transpose()?;
            if counter_gap.is_some_and(|gap| !gap.is_finite() || gap < 0.0) {
                bail!("counter_gap must be finite and non-negative");
            }
            for size in [min_size, max_size].into_iter().flatten() {
                if size
                    .iter()
                    .flatten()
                    .any(|value| !value.is_finite() || *value < 0.0)
                {
                    bail!("min_size and max_size must be finite and non-negative");
                }
            }
            apply_all(
                doc,
                replace_data_operation(doc, id, |data| {
                    let NodeData::Group(group) = data else {
                        return;
                    };
                    let mode = match direction {
                        LayoutDirection::Horizontal => LayoutMode::Horizontal,
                        LayoutDirection::Vertical => LayoutMode::Vertical,
                        LayoutDirection::Grid => LayoutMode::Grid,
                        LayoutDirection::None => {
                            group.auto_layout = None;
                            group.grid = None;
                            return;
                        }
                    };
                    let layout = group.auto_layout.get_or_insert_with(AutoLayout::default);
                    layout.mode = mode;
                    if mode == LayoutMode::Grid {
                        // A grid's gaps live on its tracks: `gap` sets both,
                        // `counter_gap` the row gap.
                        let grid = group.grid.get_or_insert_with(|| {
                            GridLayout::for_children(child_count, layout.spacing)
                        });
                        if let Some(gap) = gap {
                            grid.column_gap = *gap;
                            grid.row_gap = *gap;
                        }
                        if let Some(gap) = counter_gap {
                            grid.row_gap = *gap;
                        }
                    } else {
                        group.grid = None;
                    }
                    if let Some(align_content) = align_content {
                        layout.counter_auto_spacing = *align_content == AlignContent::SpaceBetween;
                    }
                    if let Some(sizing) = primary_sizing {
                        layout.primary_sizing = layout_sizing(*sizing);
                    }
                    if let Some(sizing) = counter_sizing {
                        layout.counter_sizing = layout_sizing(*sizing);
                    }
                    if let Some(size) = min_size {
                        layout.min_size = *size;
                    }
                    if let Some(size) = max_size {
                        layout.max_size = *size;
                    }
                    if let Some(wrap) = wrap {
                        layout.wrap = *wrap;
                    }
                    if let Some(gap) = counter_gap {
                        layout.counter_spacing = *gap;
                    }
                    if let Some(gap) = gap {
                        layout.spacing = *gap;
                    }
                    if let Some(padding) = padding {
                        layout.padding = padding;
                    }
                    if let Some(align_items) = align_items {
                        layout.counter_align = match align_items {
                            CrossAxisAlignment::Start => CounterAlign::Start,
                            CrossAxisAlignment::Center => CounterAlign::Center,
                            CrossAxisAlignment::End => CounterAlign::End,
                            CrossAxisAlignment::Stretch => CounterAlign::Stretch,
                            CrossAxisAlignment::Baseline => CounterAlign::Baseline,
                        };
                    }
                    if let Some(justify) = justify {
                        layout.primary_align = match justify {
                            MainAxisAlignment::Start => PrimaryAlign::Start,
                            MainAxisAlignment::Center => PrimaryAlign::Center,
                            MainAxisAlignment::End => PrimaryAlign::End,
                            MainAxisAlignment::SpaceBetween => PrimaryAlign::SpaceBetween,
                            MainAxisAlignment::SpaceEvenly => PrimaryAlign::SpaceEvenly,
                        };
                    }
                }),
            )
        }
        DesignOp::SetGridLayout {
            id,
            columns,
            rows,
            gap,
            row_gap,
        } => {
            let id = parse_node_id(id)?;
            let node = existing_node(doc, id)?;
            if !matches!(node.data, NodeData::Group(_)) {
                bail!(
                    "grid layout applies to frames and groups, not a {} node",
                    node.data.kind_tag()
                );
            }
            if columns.is_empty() {
                bail!("columns needs at least one track");
            }
            let columns = columns.iter().map(grid_track).collect::<Result<Vec<_>>>()?;
            let rows = rows
                .as_ref()
                .map(|rows| rows.iter().map(grid_track).collect::<Result<Vec<_>>>())
                .transpose()?;
            if [gap, row_gap]
                .into_iter()
                .flatten()
                .any(|gap| !gap.is_finite() || *gap < 0.0)
            {
                bail!("gap and row_gap must be finite and non-negative");
            }
            apply_all(
                doc,
                replace_data_operation(doc, id, |data| {
                    let NodeData::Group(group) = data else {
                        return;
                    };
                    group
                        .auto_layout
                        .get_or_insert_with(AutoLayout::default)
                        .mode = LayoutMode::Grid;
                    let grid = group.grid.get_or_insert_with(GridLayout::default);
                    grid.columns = columns;
                    if let Some(rows) = rows {
                        grid.rows = rows;
                    }
                    if let Some(gap) = gap {
                        grid.column_gap = *gap;
                        grid.row_gap = *gap;
                    }
                    if let Some(gap) = row_gap {
                        grid.row_gap = *gap;
                    }
                }),
            )
        }
        DesignOp::SetLayoutChild {
            id,
            grow,
            align_self,
            absolute,
            column,
            row,
            column_span,
            row_span,
            cell_horizontal,
            cell_vertical,
            auto_place,
        } => {
            let id = parse_node_id(id)?;
            let node = existing_node(doc, id)?;
            if grow.is_some_and(|grow| !grow.is_finite() || grow < 0.0) {
                bail!("grow must be finite and non-negative");
            }
            let pins_cell = column.is_some()
                || row.is_some()
                || column_span.is_some()
                || row_span.is_some()
                || cell_horizontal.is_some()
                || cell_vertical.is_some();
            if pins_cell || auto_place.is_some() {
                let parent_is_grid = node
                    .parent
                    .and_then(|parent| doc.scene.get(parent))
                    .is_some_and(|parent| {
                        matches!(&parent.data, NodeData::Group(group)
                            if group.auto_layout.is_some_and(|layout| layout.mode == LayoutMode::Grid))
                    });
                if !parent_is_grid {
                    bail!(
                        "column, row, spans, cell alignment and auto_place apply to children of a grid frame"
                    );
                }
            }
            if pins_cell && *auto_place == Some(true) {
                bail!(
                    "auto_place unpins the cell; send it without column, row, spans or cell alignment"
                );
            }
            if [column_span, row_span]
                .into_iter()
                .flatten()
                .any(|span| *span == 0)
            {
                bail!("column_span and row_span must be at least 1");
            }
            let old = node.layout_child;
            let mut new = old.unwrap_or_default();
            if *auto_place == Some(true) {
                new.grid = None;
            }
            if pins_cell {
                let mut cell = new.grid.unwrap_or_default();
                if let Some(column) = column {
                    cell.column = *column;
                }
                if let Some(row) = row {
                    cell.row = *row;
                }
                if let Some(span) = column_span {
                    cell.column_span = *span;
                }
                if let Some(span) = row_span {
                    cell.row_span = *span;
                }
                if let Some(align) = cell_horizontal {
                    cell.horizontal = grid_alignment(*align);
                }
                if let Some(align) = cell_vertical {
                    cell.vertical = grid_alignment(*align);
                }
                new.grid = Some(cell);
            }
            if let Some(grow) = grow {
                new.grow = *grow;
            }
            if let Some(align) = align_self {
                new.align_self = Some(counter_alignment(*align));
            }
            if let Some(absolute) = absolute {
                new.absolute = *absolute;
            }
            apply_all(
                doc,
                vec![Operation::SetLayoutChild {
                    id,
                    old,
                    new: Some(new),
                }],
            )
        }
        DesignOp::CreateVariableCollection { name, modes } => {
            let name = required_name(name)?;
            if doc
                .variables
                .collections
                .values()
                .any(|collection| collection.name == name)
            {
                bail!("a variable collection named {name:?} already exists");
            }
            let names = if modes.is_empty() {
                vec!["Default".to_owned()]
            } else {
                modes.clone()
            };
            let mut unique = HashSet::new();
            let modes = names
                .iter()
                .map(|name| {
                    let name = required_name(name)?;
                    if !unique.insert(name.to_owned()) {
                        bail!("duplicate mode name {name:?}");
                    }
                    Ok(Mode {
                        id: ModeId::new(),
                        name: name.to_owned(),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let default_mode = modes.first().context("a collection requires a mode")?.id;
            let collection = VariableCollection {
                id: VariableCollectionId::new(),
                name: name.to_owned(),
                modes,
                default_mode,
                variable_order: Vec::new(),
            };
            let detail =
                json!({"collection": collection.id.to_string(), "modes": collection.modes});
            doc.apply(Operation::CreateVariableCollection {
                collection: Box::new(collection),
            })?;
            Ok(Applied::Content {
                created: None,
                detail: Some(detail),
            })
        }
        DesignOp::AddVariableMode { collection, name } => {
            let collection = find_collection(doc, collection)?;
            let name = required_name(name)?;
            if collection.modes.iter().any(|mode| mode.name == name) {
                bail!("mode {name:?} already exists");
            }
            let mode = Mode {
                id: ModeId::new(),
                name: name.to_owned(),
            };
            let detail = json!({"mode": mode.id.to_string()});
            let transaction = doc.add_mode_transaction(collection.id, mode)?;
            for operation in transaction.ops {
                doc.apply(operation)?;
            }
            Ok(Applied::Content {
                created: None,
                detail: Some(detail),
            })
        }
        DesignOp::CreateVariable {
            collection,
            name,
            kind,
            value,
        } => {
            let collection = find_collection(doc, collection)?;
            let name = required_name(name)?;
            if doc
                .variables
                .variables
                .values()
                .any(|variable| variable.collection == collection.id && variable.name == name)
            {
                bail!("variable {name:?} already exists in this collection");
            }
            if collection.modes.is_empty() {
                bail!("add a mode before creating variables");
            }
            let ty = variable_type(*kind);
            let value = parse_variable_value(doc, ty, value)?;
            let variable = Variable {
                id: VariableId::new(),
                collection: collection.id,
                name: name.to_owned(),
                ty,
                values_by_mode: collection
                    .modes
                    .iter()
                    .map(|mode| (mode.id, value.clone()))
                    .collect(),
                scopes: Vec::new(),
            };
            let detail = json!({"variable": variable.id.to_string()});
            doc.apply(Operation::CreateVariable {
                variable: Box::new(variable),
            })?;
            Ok(Applied::Content {
                created: None,
                detail: Some(detail),
            })
        }
        DesignOp::SetVariableValue {
            variable,
            mode,
            value,
        } => {
            let variable = find_variable(doc, variable)?;
            let collection = doc
                .variables
                .collection_of(variable.id)
                .context("variable collection is missing")?;
            let mode = find_mode(collection, mode)?;
            let value = parse_variable_value(doc, variable.ty, value)?;
            let operation = crate::variables_workspace::set_variable_value_operation(
                doc,
                variable.id,
                mode,
                value,
            )
            .map_err(|error| anyhow!(error))?;
            apply_all(doc, operation.into_iter().collect())
        }
        DesignOp::SetVariableMode {
            collection,
            mode,
            frame,
        } => {
            let collection = find_collection(doc, collection)?;
            let new = mode
                .as_deref()
                .map(|mode| find_mode(collection, mode))
                .transpose()?;
            let (scope, old) = if let Some(frame) = frame {
                let id = parse_node_id(frame)?;
                let NodeData::Group(group) = &existing_node(doc, id)?.data else {
                    bail!("mode pins require a frame or group");
                };
                (
                    ModeScope::Frame { node: id },
                    group.explicit_modes.get(&collection.id).copied(),
                )
            } else {
                (
                    ModeScope::Doc,
                    doc.active_modes.get(&collection.id).copied(),
                )
            };
            apply_all(
                doc,
                vec![Operation::SetActiveMode {
                    scope,
                    collection: collection.id,
                    old,
                    new,
                }],
            )
        }
        DesignOp::BindVariable {
            id,
            property,
            variable,
        } => {
            let variable = find_variable(doc, variable)?.id;
            let operation = crate::variable_binding::variable_binding_operation(
                doc,
                parse_node_id(id)?,
                binding_property(*property),
                Some(variable),
            )
            .map_err(|error| anyhow!(error))?;
            apply_all(doc, operation.into_iter().collect())
        }
        DesignOp::UnbindVariable { id, property } => {
            let operation = crate::variable_binding::variable_binding_operation(
                doc,
                parse_node_id(id)?,
                binding_property(*property),
                None,
            )
            .map_err(|error| anyhow!(error))?;
            apply_all(doc, operation.into_iter().collect())
        }
        DesignOp::SetIndex { id, position } => {
            let id = parse_node_id(id)?;
            let node = existing_node(doc, id)?;
            if doc.pages().contains(&id) {
                bail!("page roots cannot be reordered with set_index");
            }
            let Some(new) = layer_position_index(doc, node.parent, id, node.index, *position)?
            else {
                return Ok(Applied::Nothing);
            };
            doc.apply(Operation::SetIndex {
                id,
                old: node.index,
                new,
            })?;
            Ok(Applied::changed())
        }
        DesignOp::Rotate { id, degrees } => {
            let id = parse_node_id(id)?;
            existing_node(doc, id)?;
            if !degrees.is_finite() {
                bail!("degrees must be finite");
            }
            if doc.pages().contains(&id) {
                bail!("page roots cannot be rotated");
            }
            apply_all(doc, rotation_operations(doc, id, *degrees))
        }
        DesignOp::Align { ids, edge } => {
            let ids = parse_content_ids(doc, ids)?;
            let target = match ids.as_slice() {
                [] => bail!("align needs at least one node id"),
                [single] => {
                    let parent = existing_node(doc, *single)?.parent.with_context(|| {
                        format!("node {single} has no parent frame to align to")
                    })?;
                    if doc.pages().contains(&parent) {
                        bail!(
                            "aligning a single node needs a parent frame, or two or more ids to align to each other"
                        );
                    }
                    doc.scene
                        .world_bounds(parent)
                        .with_context(|| format!("parent {parent} has no bounds"))?
                }
                many => union_bounds(doc, many).context("the nodes have no bounds to align")?,
            };
            let operations = match edge {
                AlignEdge::Left => fanta_canvas::align_to_bounds_h(
                    &doc.scene,
                    &ids,
                    target,
                    fanta_canvas::HAlign::Left,
                ),
                AlignEdge::CenterX => fanta_canvas::align_to_bounds_h(
                    &doc.scene,
                    &ids,
                    target,
                    fanta_canvas::HAlign::Center,
                ),
                AlignEdge::Right => fanta_canvas::align_to_bounds_h(
                    &doc.scene,
                    &ids,
                    target,
                    fanta_canvas::HAlign::Right,
                ),
                AlignEdge::Top => fanta_canvas::align_to_bounds_v(
                    &doc.scene,
                    &ids,
                    target,
                    fanta_canvas::VAlign::Top,
                ),
                AlignEdge::CenterY => fanta_canvas::align_to_bounds_v(
                    &doc.scene,
                    &ids,
                    target,
                    fanta_canvas::VAlign::Middle,
                ),
                AlignEdge::Bottom => fanta_canvas::align_to_bounds_v(
                    &doc.scene,
                    &ids,
                    target,
                    fanta_canvas::VAlign::Bottom,
                ),
            };
            apply_all(doc, operations)
        }
        DesignOp::Distribute { ids, axis } => {
            let ids = parse_content_ids(doc, ids)?;
            if ids.len() < 3 {
                bail!("distribute needs at least three node ids");
            }
            let axis = match axis {
                DistributeAxis::Horizontal => fanta_canvas::Axis::X,
                DistributeAxis::Vertical => fanta_canvas::Axis::Y,
            };
            apply_all(doc, fanta_canvas::distribute(&doc.scene, &ids, axis))
        }
        DesignOp::Group { ids, name } => {
            let ids = parse_content_ids(doc, ids)?;
            let grouped = group_operations(doc, &ids, name.as_deref())?;
            for operation in grouped.operations {
                doc.apply(operation)?;
            }
            Ok(Applied::created(grouped.group))
        }
        DesignOp::FrameSelection { ids, name } => {
            let ids = parse_content_ids(doc, ids)?;
            let framed = frame_selection_operations(doc, &ids, name.as_deref())?;
            for operation in framed.operations {
                doc.apply(operation)?;
            }
            Ok(Applied::created(framed.group))
        }
        DesignOp::Ungroup { id } => {
            let id = parse_node_id(id)?;
            existing_node(doc, id)?;
            let ungrouped = ungroup_operations(doc, id)?;
            for operation in ungrouped.operations {
                doc.apply(operation)?;
            }
            removed_nodes.insert(id);
            Ok(Applied::Content {
                created: None,
                detail: Some(json!({
                    "children": ungrouped
                        .children
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>(),
                })),
            })
        }
        DesignOp::Duplicate { id, dx, dy } => {
            let id = parse_node_id(id)?;
            existing_node(doc, id)?;
            let pasted = duplicate_operations(doc, &[id], (dx.unwrap_or(0.0), dy.unwrap_or(0.0)))?;
            let copy = pasted
                .roots
                .first()
                .copied()
                .context("duplicating produced no copy")?;
            for operation in create_operations(&pasted) {
                doc.apply(operation)?;
            }
            Ok(Applied::created(copy))
        }
        DesignOp::CreateComponent { id } => {
            let id = parse_node_id(id)?;
            let node = existing_node(doc, id)?;
            if doc.pages().contains(&id) {
                bail!("a page cannot become a component");
            }
            let operations = create_component_operations(doc, id);
            let Some(Operation::DefineComponent { def }) = operations.first() else {
                if !matches!(node.data, NodeData::Group(_)) {
                    bail!(
                        "only frames and groups can become components, not a {} node",
                        node.data.kind_tag()
                    );
                }
                bail!("node {id} is already a component master");
            };
            let component = def.id;
            for operation in operations {
                doc.apply(operation)?;
            }
            Ok(Applied::Content {
                created: None,
                detail: Some(json!({ "component": component.to_string() })),
            })
        }
        DesignOp::Componentize { ids } => {
            let ids = parse_content_ids(doc, ids)?;
            let Some((&master, copies)) =
                ids.split_first().filter(|(_, copies)| !copies.is_empty())
            else {
                bail!("componentize needs the master's id and at least one copy");
            };
            let related = |a: NodeId, b: NodeId| {
                doc.scene.ancestors_of(a).any(|ancestor| ancestor.id == b)
                    || doc.scene.ancestors_of(b).any(|ancestor| ancestor.id == a)
            };
            for (index, &copy) in copies.iter().enumerate() {
                if related(master, copy)
                    || copies[..index].iter().any(|&other| related(other, copy))
                {
                    bail!(
                        "node {copy} is inside another of the ids; componentize layers side by side"
                    );
                }
                if doc.components.defs.values().any(|def| def.root == copy) {
                    bail!("node {copy} is already a component master");
                }
            }
            // Check every copy before changing anything.
            let plans = copies
                .iter()
                .map(|&copy| {
                    Ok((
                        copy,
                        crate::componentize::overrides_for_copy(doc, master, copy)?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;

            let component = match doc.components.defs.values().find(|def| def.root == master) {
                Some(def) if def.variant_of.is_some() => {
                    bail!("node {master} is a variant; componentize a standalone layer")
                }
                Some(def) => def.id,
                None => {
                    let operations = create_component_operations(doc, master);
                    let Some(Operation::DefineComponent { def }) = operations.first() else {
                        bail!(
                            "only frames and groups can become components, not a {} node",
                            existing_node(doc, master)?.data.kind_tag()
                        );
                    };
                    let component = def.id;
                    for operation in operations {
                        doc.apply(operation)?;
                    }
                    component
                }
            };

            let mut instances = Vec::with_capacity(plans.len());
            for (copy, overrides) in plans {
                let original = existing_node(doc, copy)?.clone();
                let size = master_size(doc, copy).unwrap_or([100.0, 100.0]);
                let snapshot: Vec<CanvasNode> = doc
                    .scene
                    .descendants_of(copy)
                    .filter_map(|descendant| doc.scene.get(descendant).cloned())
                    .collect();
                removed_nodes.extend(snapshot.iter().map(|node| node.id));
                doc.apply(Operation::DeleteSubtree { snapshot })?;

                let mut node = CanvasNode::new(NodeData::Instance(InstanceNode {
                    component,
                    overrides,
                    prop_values: Default::default(),
                    derived: Vec::new(),
                    local_size: size,
                }));
                node.name = original.name;
                node.parent = original.parent;
                node.index = original.index;
                node.transform = original.transform;
                node.layout_child = original.layout_child;
                node.constraints = original.constraints;
                node.flags = original.flags;
                instances.push(node.id.to_string());
                doc.apply(Operation::CreateInstance {
                    node: Box::new(node),
                })
                .context("replacing a copy with an instance")?;
            }
            Ok(Applied::Content {
                created: None,
                detail: Some(json!({
                    "component": component.to_string(),
                    "instances": instances,
                })),
            })
        }
        DesignOp::CombineVariants { ids, name } => {
            let ids = parse_content_ids(doc, ids)?;
            let name = name.as_deref().map(required_name).transpose()?;
            let edit = crate::variant_sets::combine_variants(doc, &ids, name)?;
            apply_variant_edit(doc, edit)
        }
        DesignOp::AddVariants { set, ids } => {
            let set = resolve_variant_set(doc, set)?;
            let ids = parse_content_ids(doc, ids)?;
            let edit = crate::variant_sets::add_variants(doc, set, &ids)?;
            apply_variant_edit(doc, edit)
        }
        DesignOp::RemoveVariant { id } => {
            let component = match parse_node_id(id) {
                Ok(node) => doc
                    .components
                    .defs
                    .values()
                    .find(|def| def.root == node)
                    .map(|def| def.id)
                    .with_context(|| format!("node {node} is not a component master"))?,
                Err(_) => resolve_component(doc, id)?,
            };
            let edit = crate::variant_sets::remove_variant(doc, component)?;
            apply_variant_edit(doc, edit)
        }
        DesignOp::ArrangeVariants { set } => {
            let set = resolve_variant_set(doc, set)?;
            let edit = crate::variant_sets::arrange_variants(doc, set)?;
            apply_variant_edit(doc, edit)
        }
        DesignOp::RenameComponent { component, name } => {
            let component = resolve_component(doc, component)?;
            let operations = crate::variant_sets::rename_component(doc, component, name)?;
            apply_all(doc, operations)
        }
        DesignOp::CreateComponentProperty {
            component,
            kind,
            name,
        } => {
            use crate::component_properties::CreateComponentPropertyKind as Kind;
            let component = resolve_component(doc, component)?;
            let kind = match kind {
                DesignComponentPropertyKind::Text => Kind::Text,
                DesignComponentPropertyKind::Boolean => Kind::Boolean,
                DesignComponentPropertyKind::Number => Kind::Number,
                DesignComponentPropertyKind::Color => Kind::Color,
                DesignComponentPropertyKind::InstanceSwap => Kind::InstanceSwap,
                DesignComponentPropertyKind::Variant => Kind::Variant,
            };
            let mut operations = crate::component_properties::create_component_property_operations(
                doc, component, kind,
            );
            let Some(Operation::SetComponentProps { new, .. }) = operations.first() else {
                bail!("component has no available property/variant axis to expose");
            };
            let property = new.last().context("component property was not created")?.id;
            if let Some(name) = name {
                let name = required_name(name)?;
                for operation in &mut operations {
                    if let Operation::SetComponentProps { new, .. } = operation {
                        if new
                            .iter()
                            .any(|current| current.id != property && current.name == name)
                        {
                            bail!("component property name already exists");
                        }
                        for current in new.iter_mut().filter(|current| current.id == property) {
                            current.name = name.to_owned();
                        }
                    }
                }
            }
            apply_all(doc, operations)?;
            Ok(Applied::Content {
                created: None,
                detail: Some(json!({"property": property.to_string()})),
            })
        }
        DesignOp::BindComponentProperty {
            component,
            id,
            target,
            property,
        } => {
            let component = resolve_component(doc, component)?;
            let id = parse_node_id(id)?;
            let target = binding_property(*target);
            let definition = doc
                .components
                .def(component)
                .context("component master is missing")?;
            let node = existing_node(doc, id)?;
            if id == definition.root
                || !doc
                    .scene
                    .ancestors_of(id)
                    .any(|ancestor| ancestor.id == definition.root)
            {
                bail!("target must be a descendant of the component master");
            }
            if !target.applies_to(&node) {
                bail!("target property does not apply to this node");
            }
            let property = property
                .as_deref()
                .map(|property| find_component_property(definition, property))
                .transpose()?;
            if property.is_some_and(|property| {
                !crate::component_properties::component_property_accepts_binding(
                    &property.kind,
                    target,
                )
            }) {
                bail!("component property has an incompatible type");
            }
            let operations = crate::component_properties::set_component_property_binding_operations(
                doc,
                component,
                id,
                target,
                property.map(|property| property.id),
            );
            apply_all(doc, operations)
        }
        DesignOp::SetInstanceProperty {
            id,
            property,
            value,
        } => {
            let id = parse_node_id(id)?;
            let node = existing_node(doc, id)?;
            let NodeData::Instance(instance) = &node.data else {
                bail!("node is not a component instance");
            };
            let definition = doc
                .components
                .def(instance.component)
                .or_else(|| {
                    doc.components
                        .sets
                        .get(&instance.component)
                        .and_then(|set| doc.components.def(set.default_variant))
                })
                .context("instance component is missing")?;
            let property = find_component_property(definition, property)?;
            let kind = property
                .kind
                .default_value()
                .variable_type()
                .context("component property has no concrete type")?;
            let mut new = parse_variable_value(doc, kind, value)?;
            if let fanta_doc::ComponentPropKind::Variant { axis } = &property.kind {
                let set = definition
                    .variant_of
                    .as_ref()
                    .and_then(|member| doc.components.sets.get(&member.set))
                    .context("variant set is missing")?;
                let allowed = set
                    .axes
                    .iter()
                    .find(|current| current.name == *axis)
                    .context("variant axis is missing")?;
                let Some(selected) = value.as_str() else {
                    bail!("variant requires an exact axis value");
                };
                if !allowed.values.iter().any(|current| current == selected) {
                    bail!("unknown variant value");
                }
            } else if property.kind == fanta_doc::ComponentPropKind::InstanceSwap {
                let component = resolve_component(
                    doc,
                    value
                        .as_str()
                        .context("instance swap requires a component id or unique name")?,
                )?;
                new = VarValue::String {
                    value: component.to_string(),
                };
            }
            let operation = Operation::SetInstanceProp {
                id,
                prop: property.id,
                old: instance.prop_values.get(&property.id).cloned(),
                new: Some(new),
            };
            apply_all(doc, vec![operation])
        }
        DesignOp::Reparent { id, parent, index } => {
            let id = parse_node_id(id)?;
            let node = doc
                .scene
                .get(id)
                .with_context(|| format!("node {id} does not exist"))?
                .clone();
            let new_parent = resolve_container(doc, parent.as_deref(), default_container)?;
            if new_parent == Some(id)
                || new_parent.is_some_and(|parent| {
                    doc.scene
                        .ancestors_of(parent)
                        .any(|ancestor| ancestor.id == id)
                })
            {
                bail!("cannot reparent {id} into its own subtree");
            }
            let world = doc
                .scene
                .world_transform(id)
                .with_context(|| format!("node {id} has no world transform"))?;
            let new_index = sibling_index(doc, new_parent, id, *index)?;
            doc.apply(Operation::Reparent {
                id,
                old_parent: node.parent,
                old_index: node.index,
                new_parent,
                new_index,
            })?;
            // Keep the node visually in place under its new parent.
            let parent_world = new_parent
                .and_then(|parent| doc.scene.world_transform(parent))
                .unwrap_or(Transform2D::IDENTITY);
            let new_local = world.then(&parent_world.inverse());
            let current = doc
                .scene
                .get(id)
                .with_context(|| format!("node {id} disappeared during reparent"))?;
            if current.transform != new_local {
                doc.apply(Operation::SetTransform {
                    id,
                    old: current.transform,
                    new: new_local,
                })?;
            }
            Ok(Applied::changed())
        }
        DesignOp::Delete { id } => {
            let id = parse_node_id(id)?;
            if !doc.scene.contains(id) {
                bail!("node {id} does not exist");
            }
            if doc.pages().contains(&id) {
                bail!("refusing to delete a page root");
            }
            let snapshot: Vec<CanvasNode> = doc
                .scene
                .descendants_of(id)
                .filter_map(|descendant| doc.scene.get(descendant).cloned())
                .collect();
            removed_nodes.extend(snapshot.iter().map(|node| node.id));
            doc.apply(Operation::DeleteSubtree { snapshot })?;
            Ok(Applied::changed())
        }
        DesignOp::Select { ids } => {
            let mut parsed = Vec::with_capacity(ids.len());
            for raw in ids {
                let id = parse_node_id(raw)?;
                if !doc.scene.contains(id) {
                    bail!("node {raw} does not exist");
                }
                parsed.push(id);
            }
            doc.selection.replace_with(parsed);
            Ok(Applied::Selection)
        }
        DesignOp::SetViewport { center, zoom } => {
            if let Some(center) = center {
                if !(center[0].is_finite() && center[1].is_finite()) {
                    bail!("the viewport center must be finite");
                }
                doc.viewport.center = *center;
            }
            if let Some(zoom) = zoom {
                if !(zoom.is_finite() && *zoom > 0.0) {
                    bail!("the viewport zoom must be positive");
                }
                doc.viewport.zoom = zoom.clamp(0.01, 64.0);
            }
            Ok(Applied::Viewport)
        }
    }
}

fn resolve_container(
    doc: &Doc,
    parent: Option<&str>,
    default_container: Option<NodeId>,
) -> Result<Option<NodeId>> {
    // Follow can change the viewed page between streamed operations; viewing
    // must not change the destination captured for this batch.
    let Some(id) = parent.map(parse_node_id).transpose()?.or(default_container) else {
        return Ok(None);
    };
    let node = doc
        .scene
        .get(id)
        .with_context(|| format!("parent {id} does not exist"))?;
    if !node.can_have_children() {
        bail!(
            "parent {id} is a {} node and cannot have children",
            node.data.kind_tag()
        );
    }
    Ok(Some(id))
}

/// The fractional index for inserting at `position` among `parent`'s children
/// (excluding `moving`, which is being repositioned). `None` appends on top.
fn sibling_index(
    doc: &Doc,
    parent: Option<NodeId>,
    moving: NodeId,
    position: Option<usize>,
) -> Result<IndexKey> {
    let Some(position) = position else {
        return Ok(doc.scene.next_child_index(parent));
    };
    let mut keys: Vec<IndexKey> = doc
        .scene
        .children_of(parent)
        .iter()
        .filter(|child| **child != moving)
        .filter_map(|child| doc.scene.get(*child))
        .map(|child| child.index)
        .collect();
    keys.sort_by(|left, right| left.raw().total_cmp(&right.raw()));
    if keys.is_empty() {
        return Ok(doc.scene.next_child_index(parent));
    }
    if position == 0 {
        return Ok(IndexKey::before(keys[0]));
    }
    if position >= keys.len() {
        return Ok(IndexKey::after(keys[keys.len() - 1]));
    }
    let (left, right) = (keys[position - 1], keys[position]);
    if IndexKey::near_precision_limit(left, right) {
        bail!("the insertion gap at position {position} is exhausted; reorder the siblings first");
    }
    Ok(IndexKey::between(left, right))
}

/// The node for `id`, cloned so the borrow does not outlive later mutation.
fn existing_node(doc: &Doc, id: NodeId) -> Result<CanvasNode> {
    doc.scene
        .get(id)
        .cloned()
        .with_context(|| format!("node {id} does not exist"))
}

/// Apply a helper's operation list inside the running transaction; an empty
/// list means the request changed nothing.
fn apply_all(doc: &mut Doc, operations: Vec<Operation>) -> Result<Applied> {
    if operations.is_empty() {
        return Ok(Applied::Nothing);
    }
    for operation in operations {
        doc.apply(operation)?;
    }
    Ok(Applied::changed())
}

/// Parse a list of ids that must all name existing, non-page content nodes.
fn parse_content_ids(doc: &Doc, raw_ids: &[String]) -> Result<Vec<NodeId>> {
    let mut ids = Vec::with_capacity(raw_ids.len());
    for raw in raw_ids {
        let id = parse_node_id(raw)?;
        if !doc.scene.contains(id) {
            bail!("node {raw} does not exist");
        }
        if doc.pages().contains(&id) {
            bail!("{raw} is a page root, not a content node");
        }
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(ids)
}

/// `[all]`, `[vertical, horizontal]` or `[top, right, bottom, left]` into the
/// layout's `[top, right, bottom, left]`.
fn parse_padding(values: &[f64]) -> Result<[f64; 4]> {
    if values
        .iter()
        .any(|value| !(value.is_finite() && *value >= 0.0))
    {
        bail!("padding values must be non-negative");
    }
    match values {
        [all] => Ok([*all; 4]),
        [vertical, horizontal] => Ok([*vertical, *horizontal, *vertical, *horizontal]),
        [top, right, bottom, left] => Ok([*top, *right, *bottom, *left]),
        _ => bail!(
            "padding takes 1, 2 or 4 values ([all], [vertical, horizontal] or [top, right, bottom, left]), not {}",
            values.len()
        ),
    }
}

/// A component by id, or by name when exactly one component carries it.
fn resolve_component(doc: &Doc, reference: &str) -> Result<ComponentId> {
    if let Ok(id) = reference.parse::<ComponentId>()
        && (doc.components.def(id).is_some() || doc.components.sets.contains_key(&id))
    {
        return Ok(id);
    }
    let mut matches = doc
        .components
        .defs
        .values()
        .filter(|def| def.name == reference)
        .map(|def| def.id)
        .chain(
            doc.components
                .sets
                .values()
                .filter(|set| set.name == reference)
                .map(|set| set.id),
        );
    let first = matches
        .next()
        .with_context(|| format!("no component is named or identified by `{reference}`"))?;
    if matches.next().is_some() {
        bail!("several components are named `{reference}`; use the component id");
    }
    Ok(first)
}

/// Apply a variant-set edit and report the set and its frame.
fn apply_variant_edit(doc: &mut Doc, edit: crate::variant_sets::SetEdit) -> Result<Applied> {
    for operation in edit.operations {
        doc.apply(operation)?;
    }
    Ok(Applied::Content {
        created: None,
        detail: Some(json!({
            "component_set": edit.set.to_string(),
            "frame": edit.frame.map(|frame| frame.to_string()),
        })),
    })
}

fn resolve_variant_set(doc: &Doc, reference: &str) -> Result<ComponentId> {
    let id = resolve_component(doc, reference)?;
    if !doc.components.sets.contains_key(&id) {
        bail!("`{reference}` is a component, not a variant set");
    }
    Ok(id)
}

/// The size a new instance takes: the master's frame box, or its content
/// bounds for a plain group.
fn master_size(doc: &Doc, root: NodeId) -> Option<[f64; 2]> {
    let node = doc.scene.get(root)?;
    if let NodeData::Group(group) = &node.data
        && let Some(size) = group.clip_size.or(group.local_size)
    {
        return Some(size);
    }
    let bounds = doc.scene.local_bounds(root)?;
    Some([bounds.width(), bounds.height()])
}

/// The new z-slot for `set_index`, or `None` when the node is already there.
fn layer_position_index(
    doc: &Doc,
    parent: Option<NodeId>,
    id: NodeId,
    current: IndexKey,
    position: LayerPosition,
) -> Result<Option<IndexKey>> {
    let siblings: Vec<IndexKey> = doc
        .scene
        .children_of(parent)
        .iter()
        .filter(|sibling| **sibling != id)
        .filter_map(|sibling| doc.scene.get(*sibling))
        .map(|sibling| sibling.index)
        .collect();
    let above = siblings.iter().copied().filter(|index| *index > current);
    let below = siblings.iter().copied().filter(|index| *index < current);
    let new = match position {
        LayerPosition::Named(NamedLayerPosition::Front) => match siblings.last() {
            Some(top) if *top > current => IndexKey::after(*top),
            _ => return Ok(None),
        },
        LayerPosition::Named(NamedLayerPosition::Back) => match siblings.first() {
            Some(bottom) if *bottom < current => IndexKey::before(*bottom),
            _ => return Ok(None),
        },
        LayerPosition::Named(NamedLayerPosition::Forward) => {
            let mut above = above;
            let Some(next) = above.next() else {
                return Ok(None);
            };
            match above.next() {
                Some(after_next) => {
                    if IndexKey::near_precision_limit(next, after_next) {
                        bail!(
                            "the z-order gap above {id} is exhausted; reorder the siblings first"
                        );
                    }
                    IndexKey::between(next, after_next)
                }
                None => IndexKey::after(next),
            }
        }
        LayerPosition::Named(NamedLayerPosition::Backward) => {
            let mut below: Vec<IndexKey> = below.collect();
            let Some(previous) = below.pop() else {
                return Ok(None);
            };
            match below.pop() {
                Some(before_previous) => {
                    if IndexKey::near_precision_limit(before_previous, previous) {
                        bail!(
                            "the z-order gap below {id} is exhausted; reorder the siblings first"
                        );
                    }
                    IndexKey::between(before_previous, previous)
                }
                None => IndexKey::before(previous),
            }
        }
        LayerPosition::Absolute { index } => {
            let current_position = siblings
                .iter()
                .filter(|sibling| **sibling < current)
                .count();
            let already_on_top = index >= siblings.len() && current_position == siblings.len();
            if index == current_position || already_on_top {
                return Ok(None);
            }
            sibling_index(doc, parent, id, Some(index))?
        }
    };
    Ok(Some(new))
}

/// Decode a `create_image` source: a `data:` URI or raw base64. URLs are
/// deliberately not fetched here — the surface is synchronous and network
/// access belongs to the tool layer (`place_generation`), which downloads and
/// re-issues the op with base64.
pub(crate) fn decode_image_source(source: &str) -> Result<Vec<u8>> {
    let source = source.trim();
    if source.starts_with("http://") || source.starts_with("https://") {
        bail!(
            "create_image does not fetch URLs; use the place_generation tool, or fetch the \
             bytes yourself and pass them as base64"
        );
    }
    let payload = match source.strip_prefix("data:") {
        Some(rest) => {
            let (header, payload) = rest
                .split_once(',')
                .context("the data: URI has no `,` separating the payload")?;
            if !header.ends_with(";base64") {
                bail!("only base64 data: URIs are supported");
            }
            payload
        }
        None => source,
    };
    // Models occasionally hard-wrap long base64 payloads; strip whitespace
    // before decoding so that doesn't fail the op.
    let compact: String = payload.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(compact.as_bytes())
        .context("the image source is not valid base64")?;
    if bytes.len() > MAX_IMAGE_SOURCE_BYTES {
        bail!(
            "the image is {} bytes; the limit is {MAX_IMAGE_SOURCE_BYTES}",
            bytes.len()
        );
    }
    Ok(bytes)
}

fn parse_fill_color(raw: &str) -> Result<Color> {
    parse_color(raw)
        .with_context(|| format!("`{raw}` is not a valid hex color (use #RRGGBB or #RRGGBBAA)"))
}

/// Replace the node's primary paint with a solid color: a vector/boolean's
/// first fill, a frame's background, a text node's glyph color.
fn design_shadow(shadow: &DesignShadow, kind: ShadowKind) -> Result<Shadow> {
    let defaults = default_shadow();
    for (label, value) in [
        ("x", shadow.x),
        ("y", shadow.y),
        ("blur", shadow.blur),
        ("spread", shadow.spread),
    ] {
        if value.is_some_and(|value| !value.is_finite()) {
            bail!("the shadow {label} must be finite");
        }
    }
    if shadow.blur.is_some_and(|blur| blur < 0.0) {
        bail!("the shadow blur must be non-negative");
    }
    Ok(Shadow {
        kind,
        color: match &shadow.color {
            Some(color) => parse_fill_color(color)?,
            None => defaults.color,
        },
        blur: shadow.blur.unwrap_or(defaults.blur),
        spread: shadow.spread.unwrap_or(defaults.spread),
        offset: [
            shadow.x.unwrap_or(defaults.offset[0]),
            shadow.y.unwrap_or(defaults.offset[1]),
        ],
        ..defaults
    })
}

fn design_fill(
    paint: &DesignPaint,
    assets: &mut AssetStores<'_>,
    ingested_assets: &mut Vec<AssetId>,
) -> Result<Fill> {
    let stops =
        |stops: &[design_surface::DesignGradientStop]| -> Result<Vec<fanta_doc::GradientStop>> {
            if stops.len() < 2 {
                bail!("a gradient needs at least two stops");
            }
            stops
                .iter()
                .map(|stop| {
                    if !(stop.position.is_finite() && (0.0..=1.0).contains(&stop.position)) {
                        bail!("gradient stop positions must be between 0 and 1");
                    }
                    Ok(fanta_doc::GradientStop {
                        position: stop.position,
                        color: parse_fill_color(&stop.color)?,
                    })
                })
                .collect()
        };
    let point = |value: Option<[f32; 2]>, default: [f32; 2]| -> Result<[f32; 2]> {
        let point = value.unwrap_or(default);
        if !point.iter().all(|coordinate| coordinate.is_finite()) {
            bail!("gradient points must be finite");
        }
        Ok(point)
    };
    Ok(match paint {
        DesignPaint::Solid { color } => Fill::solid(parse_fill_color(color)?),
        DesignPaint::Linear {
            from,
            to,
            stops: list,
        } => Fill::Gradient {
            gradient: fanta_doc::Gradient::Linear {
                start: point(*from, [0.5, 0.0])?,
                end: point(*to, [0.5, 1.0])?,
                stops: stops(list)?,
            },
            blend: fanta_doc::BlendMode::Normal,
        },
        DesignPaint::Radial {
            center,
            radius,
            stops: list,
        } => {
            let radius = radius.unwrap_or(0.5);
            if !(radius.is_finite() && radius > 0.0) {
                bail!("the gradient radius must be positive");
            }
            Fill::Gradient {
                gradient: fanta_doc::Gradient::Radial {
                    center: point(*center, [0.5, 0.5])?,
                    radius,
                    handles: None,
                    stops: stops(list)?,
                },
                blend: fanta_doc::BlendMode::Normal,
            }
        }
        DesignPaint::Image { source, fit } => {
            let bytes = decode_image_source(source)?;
            let (asset, _natural_size, inserted) = assets.add_image_tracked(bytes)?;
            if inserted {
                ingested_assets.push(asset);
            }
            Fill::Image {
                asset,
                mode: match fit.unwrap_or(DesignImageFit::Fill) {
                    DesignImageFit::Fill => ImageFitMode::Fill,
                    DesignImageFit::Fit => ImageFitMode::Fit,
                    DesignImageFit::Stretch => ImageFitMode::Stretch,
                    DesignImageFit::Tile => ImageFitMode::Tile,
                },
                opacity: 1.0,
                crop: None,
                adjust: Default::default(),
                scale: None,
                rotation: None,
                blend: fanta_doc::BlendMode::Normal,
            }
        }
    })
}

fn set_solid_fill(data: &mut NodeData, color: Color) {
    match data {
        NodeData::Vector(vector) => {
            if let Some(fill) = vector.fills.first_mut() {
                *fill = Fill::solid(color);
            } else {
                vector.fills.push(Fill::solid(color));
            }
        }
        NodeData::Boolean(boolean) => {
            if let Some(fill) = boolean.fills.first_mut() {
                *fill = Fill::solid(color);
            } else {
                boolean.fills.push(Fill::solid(color));
            }
        }
        NodeData::Group(group) => group.background = Some(Fill::solid(color)),
        NodeData::Text(text) => text.set_glyph_color(color),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        apply_batch(doc, &mut stores.stores(), ops, label)
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

    #[test]
    fn agent_design_system_creates_modes_bindings_and_persists_sources() -> Result<()> {
        let (mut doc, _) = doc_with_page();
        let result = run_batch(
            &mut doc,
            &ops(json!([
                {"op":"create_variable_collection","name":"Theme","modes":["Light","Dark"]},
                {"op":"create_variable","collection":"Theme","name":"surface/accent","kind":"color","value":"#2255EE"},
                {"op":"create_node","node_type":"rectangle","x":0,"y":0,"width":128,"height":48,"fill":"#FFFFFF"}
            ])),
            "Create foundations",
        );
        assert_eq!(result.value["applied"], true);
        let node = created_id(&result, 0);
        let result = run_batch(
            &mut doc,
            &ops(json!([
                {"op":"set_variable_value","variable":"surface/accent","mode":"Dark","value":"#88AAFF"},
                {"op":"bind_variable","id":node.to_string(),"property":{"prop":"fill_color"},"variable":"surface/accent"},
                {"op":"set_variable_mode","collection":"Theme","mode":"Dark"},
                {"op":"add_variable_mode","collection":"Theme","name":"Contrast"}
            ])),
            "Bind and theme",
        );
        assert_eq!(result.value["applied"], true);
        let variable = find_variable(&doc, "surface/accent")?;
        let collection = find_collection(&doc, "Theme")?;
        assert_eq!(variable.values_by_mode.len(), 3);
        assert_eq!(
            variable
                .values_by_mode
                .get(&find_mode(collection, "Contrast")?),
            variable
                .values_by_mode
                .get(&find_mode(collection, "Light")?)
        );
        let variable_id = variable.id;
        let resolved = fanta_doc::resolve_bound_value(
            &doc.variables,
            &doc.scene,
            node,
            &doc.active_modes,
            variable_id,
        )
        .context("resolved token")?;
        let mut painted = existing_node(&doc, node)?;
        BoundProp::FillColor { index: 0 }.apply_resolved(&mut painted, resolved);
        let NodeData::Vector(vector) = &painted.data else {
            bail!("expected a vector");
        };
        assert_eq!(
            vector.fills.first(),
            Some(&Fill::solid(parse_fill_color("#88AAFF")?))
        );
        doc.selection.replace_with([node]);
        let state = design_system_json(
            &doc,
            None,
            DesignSystemQuery {
                include_bindings: true,
                ..Default::default()
            },
        )?;
        assert_eq!(state["variable_count"], 1);
        assert_eq!(
            state["selection_bindings"][0]["bindings"][0]["variable"],
            variable_id.to_string()
        );
        let directory = tempfile::tempdir()?;
        fanta_format::write_project_tree(directory.path(), &doc, &Default::default())?;
        let (saved, _) = fanta_format::read_project_tree(directory.path())?;
        assert_eq!(saved.variables, doc.variables);
        assert_eq!(saved.active_modes, doc.active_modes);
        assert_eq!(
            saved.scene.get(node).context("saved node")?.bindings,
            doc.scene.get(node).context("node")?.bindings
        );
        doc.undo()?;
        assert!(doc.active_modes.is_empty());
        assert!(doc.scene.get(node).context("node")?.bindings.is_empty());
        assert_eq!(find_collection(&doc, "Theme")?.modes.len(), 2);
        Ok(())
    }

    #[test]
    fn agent_design_system_rejects_invalid_aliases_and_rolls_back_variables() -> Result<()> {
        let (mut doc, _) = doc_with_page();
        let result = run_batch(
            &mut doc,
            &ops(json!([
                {"op":"create_variable_collection","name":"Theme"},
                {"op":"create_variable","collection":"Theme","name":"radius","kind":"float","value":8},
                {"op":"create_variable","collection":"Theme","name":"accent","kind":"color","value":{"alias":"radius"}}
            ])),
            "Invalid system",
        );
        assert_eq!(result.value["applied"], false);
        assert!(doc.variables.is_empty());
        let result = run_batch(
            &mut doc,
            &ops(json!([
                {"op":"create_variable_collection","name":"Theme"},
                {"op":"create_variable","collection":"Theme","name":"a","kind":"float","value":8},
                {"op":"create_variable","collection":"Theme","name":"b","kind":"float","value":{"alias":"a"}},
                {"op":"set_variable_value","variable":"a","mode":"Default","value":{"alias":"b"}}
            ])),
            "Alias cycle",
        );
        assert_eq!(result.value["applied"], false);
        assert!(doc.variables.is_empty());
        Ok(())
    }

    #[test]
    fn agent_component_properties_and_variant_sets_drive_real_instances() -> Result<()> {
        let (mut doc, page) = doc_with_page();
        let mut masters = Vec::new();
        let mut labels = Vec::new();
        for name in ["Default", "Hover"] {
            let mut master = CanvasNode::new(NodeData::Group(GroupNode {
                clip_size: Some([128.0, 48.0]),
                ..Default::default()
            }));
            master.name = name.into();
            master.parent = Some(page);
            let root = master.id;
            doc.apply(Operation::create_node(master))?;
            let mut label = CanvasNode::new(NodeData::Text(TextNode::new(name, 100.0, 20.0)));
            label.parent = Some(root);
            labels.push(label.id);
            doc.apply(Operation::create_node(label))?;
            masters.push(root);
        }
        let result = run_batch(
            &mut doc,
            &ops(json!([
                {"op":"create_component","id":masters[0].to_string()},
                {"op":"create_component","id":masters[1].to_string()},
                {"op":"combine_variants","ids":masters.iter().map(ToString::to_string).collect::<Vec<_>>(),"name":"Button"},
                {"op":"create_component_property","component":"Default","kind":"text","name":"Label"},
                {"op":"bind_component_property","component":"Default","id":labels[0].to_string(),"target":{"prop":"text_content"},"property":"Label"},
                {"op":"create_component_property","component":"Default","kind":"variant","name":"State"},
                {"op":"create_instance","component":"Button","x":200,"y":100}
            ])),
            "Create reusable button",
        );
        assert_eq!(result.value["applied"], true, "{}", result.value);
        let instance = created_id(&result, 0);
        let result = run_batch(
            &mut doc,
            &ops(json!([
                {"op":"set_instance_property","id":instance.to_string(),"property":"Label","value":"Continue"}
            ])),
            "Customize label",
        );
        assert_eq!(result.value["applied"], true);
        let expanded_text = |doc: &Doc| -> Result<Vec<String>> {
            let NodeData::Instance(instance) = &doc.scene.get(instance).context("instance")?.data
            else {
                bail!("expected instance");
            };
            Ok(
                fanta_doc::expand_instance(&doc.scene, &doc.components, instance)
                    .into_iter()
                    .filter_map(|expanded| match expanded.node.data {
                        NodeData::Text(text) => Some(text.content),
                        _ => None,
                    })
                    .collect(),
            )
        };
        assert_eq!(expanded_text(&doc)?, vec!["Continue"]);
        let result = run_batch(
            &mut doc,
            &ops(json!([
                {"op":"set_instance_property","id":instance.to_string(),"property":"State","value":"Hover"}
            ])),
            "Use hover variant",
        );
        assert_eq!(result.value["applied"], true);
        assert_eq!(expanded_text(&doc)?, vec!["Hover"]);
        let result = run_batch(
            &mut doc,
            &ops(json!([
                {"op":"set_instance_property","id":instance.to_string(),"property":"State","value":"Unknown"}
            ])),
            "Reject unknown variant",
        );
        assert_eq!(result.value["applied"], false);
        assert_eq!(expanded_text(&doc)?, vec!["Hover"]);
        let directory = tempfile::tempdir()?;
        fanta_format::write_project_tree(directory.path(), &doc, &Default::default())?;
        let (saved, _) = fanta_format::read_project_tree(directory.path())?;
        assert_eq!(expanded_text(&saved)?, vec!["Hover"]);
        Ok(())
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

        let located = pages_json(&pages, &doc, Some(temporary.path()));
        let source = located[0]["source"]
            .as_str()
            .expect("the page names its source");
        assert!(source.ends_with("page.fnx"), "{source}");
        assert!(source.starts_with("pages/"), "{source}");

        // A `.fig` import with no project on disk has no source to name.
        assert_eq!(pages_json(&pages, &doc, None)[0]["source"], Value::Null);
    }

    #[test]
    fn create_image_ingests_the_asset_and_places_a_bitmap_node() {
        let (mut doc, page_id) = doc_with_page();
        let mut stores = crate::document::TestAssetStores::default();
        let outcome = apply_batch(
            &mut doc,
            &mut stores.stores(),
            &ops(json!([
                {"op": "create_image", "source": tiny_png_base64(), "name": "Hero",
                 "x": 5.0, "y": 7.0, "width": 200.0,
                 "meta": {"generation": {"prompt": "a hero", "model": "m1", "generation_id": "g1"}}},
            ])),
            "Place image",
        );
        assert_eq!(outcome.value["applied"], json!(true));
        let id = created_id(&outcome, 0);
        let node = doc.scene.get(id).unwrap();
        assert_eq!(node.parent, Some(page_id));
        assert_eq!(node.name, "Hero");
        let NodeData::Bitmap(bitmap) = &node.data else {
            panic!("expected a bitmap node");
        };
        assert_eq!(bitmap.natural_size, [2, 1]);
        // One dimension given: the other follows the 2:1 natural aspect.
        assert_eq!(bitmap.local_size, [200.0, 100.0]);
        assert_eq!(
            node.meta["generation"]["prompt"],
            json!("a hero"),
            "provenance lands in node meta"
        );

        // The asset is in the raw store (for save) and resolvable (for render).
        assert_eq!(stores.raw_assets().len(), 1);
        assert!(stores.raw_assets().contains_key(&bitmap.asset));
        let resolved = stores
            .resolver()
            .expect("resolver present after ingest")
            .resolve(bitmap.asset)
            .expect("the new asset resolves");
        assert_eq!((resolved.width, resolved.height), (2, 1));
    }

    #[test]
    fn failing_batch_rolls_back_ingested_assets() {
        let (mut doc, _) = doc_with_page();
        let mut stores = crate::document::TestAssetStores::default();
        let outcome = apply_batch(
            &mut doc,
            &mut stores.stores(),
            &ops(json!([
                {"op": "create_image", "source": tiny_png_base64(),
                 "x": 0.0, "y": 0.0},
                {"op": "delete", "id": "not-a-node"},
            ])),
            "Broken image batch",
        );
        assert_eq!(outcome.value["applied"], json!(false));
        assert_eq!(
            stores.raw_assets().len(),
            0,
            "the ingested asset was rolled back"
        );
    }

    #[test]
    fn create_image_refuses_urls_and_junk() {
        let (mut doc, _) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_image", "source": "https://example.com/cat.png",
                 "x": 0.0, "y": 0.0},
            ])),
            "URL image",
        );
        assert_eq!(outcome.value["applied"], json!(false));
        let error = outcome.value["ops"][0]["error"].as_str().unwrap();
        assert!(
            error.contains("place_generation"),
            "error steers to the tool: {error}"
        );

        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_image", "source": "bm90IGFuIGltYWdl", // "not an image"
                 "x": 0.0, "y": 0.0},
            ])),
            "Junk image",
        );
        assert_eq!(outcome.value["applied"], json!(false));
    }

    #[test]
    fn create_batch_places_nodes_on_the_active_page_as_one_undo_step() {
        let (mut doc, page_id) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame", "name": "Card",
                 "x": 10.0, "y": 20.0, "width": 200.0, "height": 100.0, "fill": "#FFFFFF"},
                {"op": "create_node", "node_type": "text", "text": "Hello",
                 "x": 26.0, "y": 36.0, "width": 120.0, "height": 24.0, "font_size": 14.0},
            ])),
            "Create card",
        );
        assert_eq!(outcome.value["applied"], json!(true));
        assert_eq!(outcome.change, DocChange::Content);

        let frame = created_id(&outcome, 0);
        let text = created_id(&outcome, 1);
        assert_eq!(doc.scene.get(frame).unwrap().parent, Some(page_id));
        assert_eq!(doc.scene.get(frame).unwrap().name, "Card");
        let bounds = doc.scene.world_bounds(frame).unwrap();
        assert_eq!(
            (bounds.min_x, bounds.min_y, bounds.width(), bounds.height()),
            (10.0, 20.0, 200.0, 100.0)
        );
        let NodeData::Text(text_node) = &doc.scene.get(text).unwrap().data else {
            panic!("expected a text node");
        };
        assert_eq!(text_node.content, "Hello");
        assert_eq!(text_node.style.size_px, 14.0);

        // The whole batch is one undo step.
        assert_eq!(doc.history.undo_depth(), 1);
        assert!(doc.undo().unwrap());
        assert!(!doc.scene.contains(frame));
        assert!(!doc.scene.contains(text));
    }

    #[test]
    fn set_props_addresses_world_bounds_and_restyles_in_place() {
        let (mut doc, _) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "rectangle",
                 "x": 0.0, "y": 0.0, "width": 40.0, "height": 40.0},
            ])),
            "Create",
        );
        let id = created_id(&outcome, 0);

        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_props", "id": id.to_string(), "x": 50.0, "y": 60.0,
                 "fill": "#FF0000", "corner_radius": 8.0, "opacity": 0.5},
            ])),
            "Restyle",
        );
        assert_eq!(outcome.value["applied"], json!(true));
        let bounds = doc.scene.world_bounds(id).unwrap();
        assert_eq!((bounds.min_x, bounds.min_y), (50.0, 60.0));
        let node = doc.scene.get(id).unwrap();
        assert_eq!(node.opacity, UnitInterval::new(0.5));
        let NodeData::Vector(vector) = &node.data else {
            panic!("expected a vector node");
        };
        assert_eq!(vector.corner_radius, Some(8.0));
        assert_eq!(
            vector.fills.first().and_then(Fill::solid_color),
            parse_color("#FF0000")
        );
    }

    #[test]
    fn reparent_preserves_world_position() {
        let (mut doc, page_id) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame",
                 "x": 100.0, "y": 100.0, "width": 300.0, "height": 200.0},
                {"op": "create_node", "node_type": "rectangle",
                 "x": 120.0, "y": 130.0, "width": 40.0, "height": 40.0},
            ])),
            "Create",
        );
        let frame = created_id(&outcome, 0);
        let rect = created_id(&outcome, 1);
        assert_eq!(doc.scene.get(rect).unwrap().parent, Some(page_id));

        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "reparent", "id": rect.to_string(), "parent": frame.to_string()},
            ])),
            "Nest",
        );
        assert_eq!(outcome.value["applied"], json!(true));
        assert_eq!(doc.scene.get(rect).unwrap().parent, Some(frame));
        let bounds = doc.scene.world_bounds(rect).unwrap();
        assert_eq!((bounds.min_x, bounds.min_y), (120.0, 130.0));
    }

    #[test]
    fn failing_op_rolls_back_the_whole_batch() {
        let (mut doc, _) = doc_with_page();
        let nodes_before = doc.scene.len();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "rectangle",
                 "x": 0.0, "y": 0.0, "width": 40.0, "height": 40.0},
                {"op": "delete", "id": "not-a-node"},
            ])),
            "Broken batch",
        );
        assert_eq!(outcome.value["applied"], json!(false));
        assert_eq!(outcome.change, DocChange::None);
        assert_eq!(outcome.value["ops"][1]["status"], json!("failed"));
        assert_eq!(outcome.value["ops"][0]["status"], json!("ok"));
        assert_eq!(
            doc.scene.len(),
            nodes_before,
            "the created node was rolled back"
        );
        assert_eq!(
            doc.history.undo_depth(),
            0,
            "no undo step for a failed batch"
        );
    }

    #[test]
    fn delete_removes_the_subtree_and_prunes_selection() {
        let (mut doc, _) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame",
                 "x": 0.0, "y": 0.0, "width": 100.0, "height": 100.0},
            ])),
            "Create",
        );
        let frame = created_id(&outcome, 0);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "ellipse", "parent": frame.to_string(),
                 "x": 10.0, "y": 10.0, "width": 20.0, "height": 20.0},
            ])),
            "Fill in",
        );
        let ellipse = created_id(&outcome, 0);
        doc.selection.select_only(ellipse);

        let outcome = run_batch(
            &mut doc,
            &ops(json!([{"op": "delete", "id": frame.to_string()}])),
            "Delete",
        );
        assert_eq!(outcome.value["applied"], json!(true));
        assert!(!doc.scene.contains(frame));
        assert!(!doc.scene.contains(ellipse));
        assert!(doc.selection.iter().next().is_none());
    }

    /// Delete and ungroup prune the selection only once the batch commits:
    /// a batch that rolls back restores the nodes, so it must hand back the
    /// selection it started with too, or the caller would lose the user's
    /// selection to an edit that never happened.
    #[test]
    fn a_failed_batch_leaves_the_selection_untouched() {
        let (mut doc, _) = doc_with_page();
        let first = create_rect(&mut doc, 0.0, 0.0, 20.0, 20.0);
        let second = create_rect(&mut doc, 40.0, 0.0, 20.0, 20.0);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([{"op": "group", "ids": [second.to_string()]}])),
            "Group",
        );
        assert_applied(&outcome);
        let group = created_id(&outcome, 0);
        doc.selection.replace_with([first, group]);

        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "delete", "id": first.to_string()},
                {"op": "ungroup", "id": group.to_string()},
                {"op": "delete", "id": "not-a-node"},
            ])),
            "Broken batch",
        );
        assert_eq!(outcome.value["applied"], json!(false));
        assert!(doc.scene.contains(first));
        assert!(doc.scene.contains(group));
        assert_eq!(
            doc.selection.iter().copied().collect::<Vec<_>>(),
            vec![first, group]
        );

        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "delete", "id": first.to_string()},
                {"op": "ungroup", "id": group.to_string()},
            ])),
            "Working batch",
        );
        assert_applied(&outcome);
        assert!(doc.selection.is_empty());
    }
    fn create_rect(doc: &mut Doc, x: f64, y: f64, width: f64, height: f64) -> NodeId {
        let outcome = run_batch(
            doc,
            &ops(json!([
                {"op": "create_node", "node_type": "rectangle",
                 "x": x, "y": y, "width": width, "height": height},
            ])),
            "Create",
        );
        assert_eq!(outcome.value["applied"], json!(true), "{}", outcome.value);
        created_id(&outcome, 0)
    }

    fn assert_applied(outcome: &BatchOutcome) {
        assert_eq!(outcome.value["applied"], json!(true), "{}", outcome.value);
    }

    #[test]
    fn empty_space_is_the_origin_on_an_empty_page_and_beside_content_otherwise() {
        let (mut doc, page_id) = doc_with_page();
        assert_eq!(empty_space(&doc, page_id, 100.0, 50.0), glam::DVec2::ZERO);

        create_rect(&mut doc, 10.0, 20.0, 200.0, 100.0);
        let spot = empty_space(&doc, page_id, 100.0, 50.0);
        assert_eq!((spot.x, spot.y), (210.0 + EMPTY_SPACE_MARGIN, 20.0));
        assert!(page_area_is_empty(
            &doc,
            page_id,
            Bounds::from_xywh(spot.x, spot.y, 100.0, 50.0)
        ));
        assert!(!page_area_is_empty(
            &doc,
            page_id,
            Bounds::from_xywh(0.0, 0.0, 50.0, 50.0)
        ));
    }

    #[test]
    fn empty_space_never_overlaps_existing_top_level_content() {
        let (mut doc, page_id) = doc_with_page();
        create_rect(&mut doc, 0.0, 0.0, 100.0, 100.0);
        // An empty frame is invisible to the leaf index but still occupies
        // the page; the spot must clear it too.
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame",
                 "x": 100.0 + EMPTY_SPACE_MARGIN, "y": 0.0, "width": 300.0, "height": 100.0},
            ])),
            "Frame",
        );
        assert_applied(&outcome);
        let spot = empty_space(&doc, page_id, 50.0, 50.0);
        let rect = Bounds::from_xywh(spot.x, spot.y, 50.0, 50.0);
        assert!(page_area_is_empty(&doc, page_id, rect));
        for child in doc.scene.children_of(Some(page_id)) {
            let bounds = doc.scene.world_bounds(*child).unwrap();
            assert!(!bounds.intersects(&rect), "{rect:?} overlaps {bounds:?}");
        }
    }

    #[test]
    fn state_style_ops_write_stroke_shadow_and_text_style() {
        let (mut doc, _) = doc_with_page();
        let rect = create_rect(&mut doc, 0.0, 0.0, 40.0, 40.0);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "text", "text": "Hi",
                 "x": 0.0, "y": 0.0, "width": 80.0, "height": 20.0},
            ])),
            "Text",
        );
        let text = created_id(&outcome, 0);

        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_stroke", "id": rect.to_string(), "color": "#112233", "width": 3.0,
                 "align": "inside"},
                {"op": "set_shadow", "id": rect.to_string(), "color": "#00000080", "y": 6.0,
                 "blur": 12.0},
                {"op": "set_text_style", "id": text.to_string(), "font_family": "Inter",
                 "font_weight": 600, "font_size": 24.0, "line_height": 1.2,
                 "letter_spacing": -0.5, "align": "center", "color": "#FF0000"},
            ])),
            "Style",
        );
        assert_applied(&outcome);
        let NodeData::Vector(vector) = &doc.scene.get(rect).unwrap().data else {
            panic!("expected a vector");
        };
        assert_eq!(vector.strokes.len(), 1);
        assert_eq!(vector.strokes[0].width, 3.0);
        assert_eq!(vector.strokes[0].align, StrokeAlign::Inside);
        assert_eq!(
            vector.strokes[0].paint.solid_color(),
            parse_color("#112233")
        );
        let effects = &doc.scene.get(rect).unwrap().effects;
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].offset, [0.0, 6.0]);
        assert_eq!(effects[0].blur, 12.0);
        let NodeData::Text(text_node) = &doc.scene.get(text).unwrap().data else {
            panic!("expected text");
        };
        assert_eq!(text_node.style.font_family, "Inter");
        assert_eq!(text_node.style.weight, 600);
        assert_eq!(text_node.style.size_px, 24.0);
        assert_eq!(text_node.style.line_height, 1.2);
        assert_eq!(text_node.style.letter_spacing, -0.5);
        assert_eq!(text_node.align, TextAlign::Center);
        assert_eq!(text_node.style.color, parse_color("#FF0000").unwrap());

        // Width 0 removes the stroke; remove drops the shadow.
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_stroke", "id": rect.to_string(), "width": 0.0},
                {"op": "set_shadow", "id": rect.to_string(), "remove": true},
            ])),
            "Clear",
        );
        assert_applied(&outcome);
        let NodeData::Vector(vector) = &doc.scene.get(rect).unwrap().data else {
            panic!("expected a vector");
        };
        assert!(vector.strokes.is_empty());
        assert!(doc.scene.get(rect).unwrap().effects.is_empty());

        // Strokes on text are refused with the op index named by the batch.
        let outcome = run_batch(
            &mut doc,
            &ops(json!([{"op": "set_stroke", "id": text.to_string(), "width": 1.0}])),
            "Bad stroke",
        );
        assert_eq!(outcome.value["applied"], json!(false));
        assert_eq!(outcome.value["ops"][0]["status"], json!("failed"));
    }

    #[test]
    fn set_auto_layout_configures_and_clears_the_frame_layout() {
        let (mut doc, _) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame",
                 "x": 0.0, "y": 0.0, "width": 300.0, "height": 100.0},
            ])),
            "Frame",
        );
        let frame = created_id(&outcome, 0);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_auto_layout", "id": frame.to_string(), "direction": "vertical",
                 "gap": 8.0, "padding": [16.0, 24.0], "align_items": "stretch",
                 "justify": "space_between"},
            ])),
            "Layout",
        );
        assert_applied(&outcome);
        let NodeData::Group(group) = &doc.scene.get(frame).unwrap().data else {
            panic!("expected a frame");
        };
        let layout = group.auto_layout.expect("auto layout on");
        assert_eq!(layout.mode, LayoutMode::Vertical);
        assert_eq!(layout.spacing, 8.0);
        assert_eq!(layout.padding, [16.0, 24.0, 16.0, 24.0]);
        assert_eq!(layout.counter_align, CounterAlign::Stretch);
        assert_eq!(layout.primary_align, PrimaryAlign::SpaceBetween);

        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_auto_layout", "id": frame.to_string(), "direction": "none"},
            ])),
            "Layout off",
        );
        assert_applied(&outcome);
        let NodeData::Group(group) = &doc.scene.get(frame).unwrap().data else {
            panic!("expected a frame");
        };
        assert!(group.auto_layout.is_none());

        assert!(parse_padding(&[1.0, 2.0, 3.0]).is_err());
        assert_eq!(parse_padding(&[4.0]).unwrap(), [4.0; 4]);
        assert_eq!(
            parse_padding(&[1.0, 2.0, 3.0, 4.0]).unwrap(),
            [1.0, 2.0, 3.0, 4.0]
        );
    }

    #[test]
    fn set_grid_layout_builds_tracks_and_pins_cells() {
        let (mut doc, _) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame",
                 "x": 0.0, "y": 0.0, "width": 300.0, "height": 200.0},
            ])),
            "Frame",
        );
        let frame = created_id(&outcome, 0);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "rectangle", "parent": frame.to_string(),
                 "x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0},
                {"op": "create_node", "node_type": "rectangle", "parent": frame.to_string(),
                 "x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0},
            ])),
            "Cells",
        );
        let second = created_id(&outcome, 1);

        // `direction: grid` on a frame with two children starts with
        // ⌈√2⌉ = 2 equal columns.
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_auto_layout", "id": frame.to_string(), "direction": "grid", "gap": 12.0},
            ])),
            "Grid",
        );
        assert_applied(&outcome);
        let grid = |doc: &Doc| match &doc.scene.get(frame).unwrap().data {
            NodeData::Group(group) => group.grid.clone().expect("grid tracks"),
            _ => panic!("expected a frame"),
        };
        assert_eq!(grid(&doc).columns, vec![GridTrack::Flex { fr: 1.0 }; 2]);
        assert_eq!((grid(&doc).column_gap, grid(&doc).row_gap), (12.0, 12.0));

        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_grid_layout", "id": frame.to_string(),
                 "columns": [120, "2fr", "auto"], "rows": ["1fr"], "gap": 8.0, "row_gap": 4.0},
                {"op": "set_layout_child", "id": second.to_string(), "column": 1, "row": 0,
                 "column_span": 2, "cell_horizontal": "center"},
            ])),
            "Tracks",
        );
        assert_applied(&outcome);
        let tracks = grid(&doc);
        assert_eq!(
            tracks.columns,
            vec![
                GridTrack::Fixed { size: 120.0 },
                GridTrack::Flex { fr: 2.0 },
                GridTrack::Hug,
            ]
        );
        assert_eq!(tracks.rows, vec![GridTrack::Flex { fr: 1.0 }]);
        assert_eq!((tracks.column_gap, tracks.row_gap), (8.0, 4.0));
        let cell = doc
            .scene
            .get(second)
            .unwrap()
            .layout_child
            .unwrap()
            .grid
            .unwrap();
        assert_eq!((cell.column, cell.row, cell.column_span), (1, 0, 2));
        assert_eq!(cell.horizontal, GridAlign::Center);

        let style = crate::agent_style::node_style(&doc, frame);
        assert_eq!(style["auto_layout"]["direction"], json!("grid"));
        assert_eq!(style["grid"]["columns"], json!([120.0, "2fr", "auto"]));
        let child = crate::agent_style::node_style(&doc, second);
        assert_eq!(child["layout_child"]["column_span"], json!(2));

        // Cells only exist inside a grid, and `auto_place` unpins one.
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_layout_child", "id": frame.to_string(), "column": 0},
            ])),
            "Not a grid child",
        );
        assert_eq!(outcome.value["applied"], json!(false));
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_layout_child", "id": second.to_string(), "auto_place": true},
            ])),
            "Unpin",
        );
        assert_applied(&outcome);
        assert!(
            doc.scene
                .get(second)
                .unwrap()
                .layout_child
                .is_none_or(|child| child.grid.is_none())
        );

        for bad in [json!(["wide"]), json!([]), json!(["0fr"]), json!([-4])] {
            let outcome = run_batch(
                &mut doc,
                &ops(json!([
                    {"op": "set_grid_layout", "id": frame.to_string(), "columns": bad},
                ])),
                "Bad tracks",
            );
            assert_eq!(outcome.value["applied"], json!(false), "{bad}");
        }
    }

    #[test]
    fn set_index_moves_a_node_through_its_siblings() {
        let (mut doc, page_id) = doc_with_page();
        let bottom = create_rect(&mut doc, 0.0, 0.0, 10.0, 10.0);
        let middle = create_rect(&mut doc, 0.0, 0.0, 10.0, 10.0);
        let top = create_rect(&mut doc, 0.0, 0.0, 10.0, 10.0);
        let order = |doc: &Doc| doc.scene.children_of(Some(page_id)).to_vec();
        assert_eq!(order(&doc), vec![bottom, middle, top]);

        let run = |doc: &mut Doc, id: NodeId, position: Value| {
            let outcome = run_batch(
                doc,
                &ops(json!([{"op": "set_index", "id": id.to_string(), "position": position}])),
                "Reorder",
            );
            assert_applied(&outcome);
        };
        run(&mut doc, bottom, json!("front"));
        assert_eq!(order(&doc), vec![middle, top, bottom]);
        run(&mut doc, bottom, json!("backward"));
        assert_eq!(order(&doc), vec![middle, bottom, top]);
        run(&mut doc, middle, json!("forward"));
        assert_eq!(order(&doc), vec![bottom, middle, top]);
        run(&mut doc, top, json!("back"));
        assert_eq!(order(&doc), vec![top, bottom, middle]);
        run(&mut doc, top, json!({"index": 1}));
        assert_eq!(order(&doc), vec![bottom, top, middle]);
        // Already at the front: no change, still a successful op.
        run(&mut doc, middle, json!("front"));
        assert_eq!(order(&doc), vec![bottom, top, middle]);
    }

    #[test]
    fn align_and_distribute_move_nodes_in_world_space() {
        let (mut doc, _) = doc_with_page();
        let first = create_rect(&mut doc, 0.0, 0.0, 10.0, 10.0);
        let second = create_rect(&mut doc, 100.0, 50.0, 10.0, 10.0);
        let third = create_rect(&mut doc, 130.0, 90.0, 10.0, 10.0);
        let ids = [first, second, third]
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "align", "ids": ids, "edge": "top"},
                {"op": "distribute", "ids": ids, "axis": "horizontal"},
            ])),
            "Tidy",
        );
        assert_applied(&outcome);
        for id in [first, second, third] {
            assert_eq!(doc.scene.world_bounds(id).unwrap().min_y, 0.0);
        }
        assert_eq!(doc.scene.world_bounds(second).unwrap().min_x, 65.0);

        // A single node aligns to its parent frame.
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame",
                 "x": 200.0, "y": 200.0, "width": 100.0, "height": 100.0},
            ])),
            "Frame",
        );
        let frame = created_id(&outcome, 0);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "reparent", "id": first.to_string(), "parent": frame.to_string()},
                {"op": "align", "ids": [first.to_string()], "edge": "center_x"},
                {"op": "align", "ids": [first.to_string()], "edge": "bottom"},
            ])),
            "Center in frame",
        );
        assert_applied(&outcome);
        let bounds = doc.scene.world_bounds(first).unwrap();
        assert_eq!((bounds.min_x, bounds.max_y), (245.0, 300.0));

        let outcome = run_batch(
            &mut doc,
            &ops(json!([{"op": "distribute", "ids": [second.to_string()], "axis": "vertical"}])),
            "Too few",
        );
        assert_eq!(outcome.value["applied"], json!(false));
    }

    #[test]
    fn rotate_sets_an_absolute_angle_about_the_centre() {
        let (mut doc, _) = doc_with_page();
        let rect = create_rect(&mut doc, 0.0, 0.0, 40.0, 20.0);
        let before = doc.scene.world_bounds(rect).unwrap().center();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([{"op": "rotate", "id": rect.to_string(), "degrees": 90.0}])),
            "Rotate",
        );
        assert_applied(&outcome);
        let after = doc.scene.world_bounds(rect).unwrap();
        assert!((after.center() - before).length() < 1e-9);
        assert!((after.width() - 20.0).abs() < 1e-9 && (after.height() - 40.0).abs() < 1e-9);
        let angle = fanta_canvas::transform_angle(&doc.scene.world_transform(rect).unwrap());
        assert!((angle - 90.0_f64.to_radians()).abs() < 1e-9);
    }

    #[test]
    fn duplicate_returns_the_copy_and_offsets_it() {
        let (mut doc, page_id) = doc_with_page();
        let rect = create_rect(&mut doc, 10.0, 10.0, 40.0, 20.0);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([{"op": "duplicate", "id": rect.to_string(), "dx": 50.0, "dy": 0.0}])),
            "Duplicate",
        );
        assert_applied(&outcome);
        let copy = created_id(&outcome, 0);
        assert_ne!(copy, rect);
        let bounds = doc.scene.world_bounds(copy).unwrap();
        assert_eq!((bounds.min_x, bounds.min_y), (60.0, 10.0));
        assert_eq!(doc.scene.children_of(Some(page_id)), &[rect, copy]);
        assert_eq!(doc.history.undo_depth(), 2);
    }

    #[test]
    fn componentize_replaces_copies_with_instances_that_keep_their_differences() {
        let (mut doc, page_id) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame", "name": "Card",
                 "x": 0.0, "y": 0.0, "width": 100.0, "height": 60.0},
            ])),
            "Frame",
        );
        let card = created_id(&outcome, 0);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "text", "text": "Title", "parent": card.to_string(),
                 "x": 8.0, "y": 8.0, "width": 80.0, "height": 20.0},
                {"op": "create_node", "node_type": "rectangle", "parent": card.to_string(), "fill": "#00ff00",
                 "x": 8.0, "y": 32.0, "width": 20.0, "height": 20.0},
            ])),
            "Contents",
        );
        assert_applied(&outcome);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([{"op": "duplicate", "id": card.to_string(), "dx": 150.0, "dy": 0.0}])),
            "Copy",
        );
        let copy = created_id(&outcome, 0);
        let copy_children = doc.scene.children_of(Some(copy)).to_vec();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_props", "id": copy_children[0].to_string(), "text": "Other"},
                {"op": "set_props", "id": copy_children[1].to_string(), "fill": "#ff0000"},
                {"op": "set_props", "id": copy.to_string(), "opacity": 0.5},
            ])),
            "Edit the copy",
        );
        assert_applied(&outcome);
        let copy_node = doc.scene.get(copy).unwrap().clone();
        let copy_index = doc
            .scene
            .children_of(Some(page_id))
            .iter()
            .position(|id| *id == copy);

        let outcome = run_batch(
            &mut doc,
            &ops(json!([{"op": "componentize", "ids": [card.to_string(), copy.to_string()]}])),
            "Componentize",
        );
        assert_applied(&outcome);
        let detail = &outcome.value["ops"][0];
        let instance: NodeId = detail["instances"][0].as_str().unwrap().parse().unwrap();
        assert!(!doc.scene.contains(copy));
        assert!(
            doc.components
                .defs
                .values()
                .any(|def| def.root == card && def.id.to_string() == detail["component"])
        );
        let node = doc
            .scene
            .get(instance)
            .expect("the instance replaced the copy");
        assert_eq!(node.parent, copy_node.parent);
        assert_eq!(node.transform, copy_node.transform);
        assert_eq!(node.name, copy_node.name);
        assert_eq!(
            doc.scene
                .children_of(Some(page_id))
                .iter()
                .position(|id| *id == instance),
            copy_index,
            "the instance keeps the copy's z-order"
        );

        let NodeData::Instance(data) = &node.data else {
            panic!("expected an instance");
        };
        // Only what the copy changed: its text, its rectangle's fill, its
        // root's opacity.
        let overridden: Vec<_> = data
            .overrides
            .iter()
            .map(|o| match &o.value {
                fanta_doc::OverrideValue::Field { value } => format!("field {value}"),
                fanta_doc::OverrideValue::Text { .. } => "text".to_owned(),
                fanta_doc::OverrideValue::Fills { .. } => "fills".to_owned(),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(
            overridden,
            vec!["field {\"opacity\":0.5}", "text", "fills"],
            "{:?}",
            data.overrides
        );
        let expanded = fanta_doc::expand_instance(&doc.scene, &doc.components, data);
        let text = expanded
            .iter()
            .find_map(|clone| match &clone.node.data {
                NodeData::Text(text) => Some(text.content.clone()),
                _ => None,
            })
            .expect("text clone");
        assert_eq!(text, "Other");
        let fill = expanded
            .iter()
            .find_map(|clone| match &clone.node.data {
                NodeData::Vector(vector) => vector.fills.first().and_then(Fill::solid_color),
                _ => None,
            })
            .expect("rectangle clone");
        assert_eq!(fill.to_hex(), "#FF0000");
        let root = expanded
            .iter()
            .find(|clone| clone.def_path.is_empty())
            .unwrap();
        assert_eq!(root.node.opacity.get(), 0.5);

        // A copy whose structure differs is refused, and nothing changes.
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame", "name": "Odd",
                 "x": 0.0, "y": 200.0, "width": 100.0, "height": 60.0},
            ])),
            "Odd frame",
        );
        let odd = created_id(&outcome, 0);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([{"op": "componentize", "ids": [card.to_string(), odd.to_string()]}])),
            "Mismatch",
        );
        assert_eq!(outcome.value["applied"], json!(false));
        assert!(doc.scene.contains(odd));
    }

    #[test]
    fn a_failed_batch_rolls_back_a_component_definition() {
        let (mut doc, _page_id) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame", "name": "Button",
                 "x": 0.0, "y": 0.0, "width": 120.0, "height": 40.0},
            ])),
            "Frame",
        );
        let frame = created_id(&outcome, 0);
        let defs_before = doc.components.defs.len();

        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_component", "id": frame.to_string()},
                {"op": "delete", "id": NodeId::new().to_string()},
            ])),
            "Componentize then fail",
        );
        assert_eq!(outcome.value["applied"], Value::Bool(false));
        assert_eq!(
            doc.components.defs.len(),
            defs_before,
            "the definition applied before the failure must roll back with the batch"
        );
        assert!(!doc.is_component_root(frame));
    }

    #[test]
    fn create_component_then_create_instance_by_name_and_id() {
        let (mut doc, page_id) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame", "name": "Button",
                 "x": 0.0, "y": 0.0, "width": 120.0, "height": 40.0},
            ])),
            "Frame",
        );
        let frame = created_id(&outcome, 0);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([{"op": "create_component", "id": frame.to_string()}])),
            "Componentize",
        );
        assert_applied(&outcome);
        let component = outcome.value["ops"][0]["component"]
            .as_str()
            .expect("the component id is reported")
            .to_string();
        assert!(doc.is_component_root(frame));

        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_instance", "component": "Button", "x": 200.0, "y": 300.0},
                {"op": "create_instance", "component": component, "x": 400.0, "y": 300.0,
                 "name": "Second"},
            ])),
            "Instances",
        );
        assert_applied(&outcome);
        let instance = created_id(&outcome, 0);
        let node = doc.scene.get(instance).unwrap();
        assert_eq!(node.parent, Some(page_id));
        assert_eq!(node.name, "Button");
        let NodeData::Instance(instance_node) = &node.data else {
            panic!("expected an instance");
        };
        assert_eq!(instance_node.local_size, [120.0, 40.0]);
        let bounds = doc.scene.world_bounds(instance).unwrap();
        assert_eq!((bounds.min_x, bounds.min_y), (200.0, 300.0));
        assert_eq!(
            doc.scene.get(created_id(&outcome, 1)).unwrap().name,
            "Second"
        );

        // Instances cannot nest inside their own master.
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_instance", "component": "Button", "x": 0.0, "y": 0.0,
                 "parent": frame.to_string()},
            ])),
            "Recursive",
        );
        assert_eq!(outcome.value["applied"], json!(false));
        // A second master named the same makes the name ambiguous.
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame", "name": "Button",
                 "x": 0.0, "y": 500.0, "width": 10.0, "height": 10.0},
            ])),
            "Frame",
        );
        let other = created_id(&outcome, 0);
        run_batch(
            &mut doc,
            &ops(json!([{"op": "create_component", "id": other.to_string()}])),
            "Componentize",
        );
        assert!(resolve_component(&doc, "Button").is_err());
    }

    #[test]
    fn group_ungroup_and_frame_selection_report_their_structure() {
        let (mut doc, page_id) = doc_with_page();
        let first = create_rect(&mut doc, 10.0, 10.0, 20.0, 20.0);
        let second = create_rect(&mut doc, 50.0, 30.0, 20.0, 20.0);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "group", "ids": [first.to_string(), second.to_string()], "name": "Pair"},
            ])),
            "Group",
        );
        assert_applied(&outcome);
        let group = created_id(&outcome, 0);
        assert_eq!(doc.scene.get(group).unwrap().name, "Pair");
        assert_eq!(doc.scene.get(first).unwrap().parent, Some(group));
        let bounds = doc.scene.world_bounds(first).unwrap();
        assert_eq!((bounds.min_x, bounds.min_y), (10.0, 10.0));

        let outcome = run_batch(
            &mut doc,
            &ops(json!([{"op": "ungroup", "id": group.to_string()}])),
            "Ungroup",
        );
        assert_applied(&outcome);
        let children = outcome.value["ops"][0]["children"]
            .as_array()
            .expect("freed children are reported");
        assert_eq!(children.len(), 2);
        assert!(!doc.scene.contains(group));
        assert_eq!(doc.scene.get(second).unwrap().parent, Some(page_id));

        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "frame_selection", "ids": [first.to_string(), second.to_string()]},
            ])),
            "Frame",
        );
        assert_applied(&outcome);
        let frame = created_id(&outcome, 0);
        let NodeData::Group(group_node) = &doc.scene.get(frame).unwrap().data else {
            panic!("expected a frame");
        };
        assert_eq!(group_node.clip_size, Some([60.0, 40.0]));
        assert!(group_node.background.is_none());
    }

    #[test]
    fn node_summary_carries_kind_specific_facts() {
        let (mut doc, page_id) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame", "name": "Card",
                 "x": 0.0, "y": 0.0, "width": 200.0, "height": 100.0, "fill": "#FAFAFA"},
                {"op": "create_node", "node_type": "text", "text": "Title",
                 "x": 8.0, "y": 8.0, "width": 100.0, "height": 20.0},
                {"op": "create_node", "node_type": "rectangle",
                 "x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0, "fill": "#123456"},
            ])),
            "Create",
        );
        let frame = created_id(&outcome, 0);
        run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_auto_layout", "id": frame.to_string(), "direction": "horizontal"},
            ])),
            "Layout",
        );
        let summary = node_summary(&doc, page_id, None, false);
        let children = summary["children"].as_array().unwrap();
        assert_eq!(children[0]["size"], json!([200.0, 100.0]));
        assert_eq!(children[0]["auto_layout"], json!("horizontal"));
        assert_eq!(children[0]["fill"], json!("#FAFAFA"));
        assert_eq!(children[1]["text"], json!("Title"));
        assert!(children[1].get("text_length").is_none());
        assert_eq!(children[1]["font"]["size"], json!(16.0));
        assert_eq!(children[2]["fill"], json!("#123456"));
    }

    /// A page listing must stay bounded per node, so long text is cut in the
    /// summary and its full length reported; the whole content is still
    /// there when the node is fetched by id.
    #[test]
    fn node_summary_truncates_long_text_and_reports_its_length() {
        let (mut doc, page_id) = doc_with_page();
        let long_text = "é".repeat(SUMMARY_TEXT_CHARS + 30);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "text", "text": long_text,
                 "x": 0.0, "y": 0.0, "width": 100.0, "height": 20.0},
            ])),
            "Create",
        );
        assert_applied(&outcome);
        let text = created_id(&outcome, 0);

        let summary = node_summary(&doc, page_id, None, false);
        let listed = &summary["children"][0];
        let preview = listed["text"].as_str().expect("text is a string");
        assert_eq!(preview.chars().count(), SUMMARY_TEXT_CHARS + 1);
        assert!(preview.ends_with('…'));
        assert_eq!(listed["text_length"], json!(SUMMARY_TEXT_CHARS + 30));

        let NodeData::Text(node) = &doc.scene.get(text).unwrap().data else {
            panic!("expected a text node");
        };
        assert_eq!(node.content, long_text);
    }

    /// A page with `count` sibling rectangles, and their ids in z-order.
    fn page_with_children(count: usize) -> (Doc, NodeId, Vec<String>) {
        let (mut doc, page_id) = doc_with_page();
        let requests: Vec<Value> = (0..count)
            .map(|index| {
                json!({"op": "create_node", "node_type": "rectangle", "name": format!("Row {index}"),
                       "x": 0.0, "y": index as f64 * 20.0, "width": 10.0, "height": 10.0})
            })
            .collect();
        let outcome = run_batch(&mut doc, &ops(json!(requests)), "Create");
        assert_applied(&outcome);
        let ids = (0..count)
            .map(|index| created_id(&outcome, index).to_string())
            .collect();
        (doc, page_id, ids)
    }

    fn listed_child_ids(summary: &Value) -> Vec<String> {
        summary["children"]
            .as_array()
            .expect("a listing carries a children array")
            .iter()
            .map(|child| child["id"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    /// Listing is the only way to discover node ids, so a page too wide for one
    /// response must still be enumerable: each call returns a window plus the
    /// facts needed to ask for the next one.
    #[test]
    fn a_page_listing_windows_its_children_and_reports_how_to_continue() {
        let (doc, page_id, all_children) = page_with_children(7);

        let first = paginated_node_summary(&doc, page_id, Some(1), false, 0, 3);
        assert_eq!(first["child_count"], json!(7));
        assert_eq!(first["children_offset"], json!(0));
        assert_eq!(first["children_limit"], json!(3));
        assert_eq!(first["more_children"], json!(true));
        let first_window = listed_child_ids(&first);
        assert_eq!(first_window.len(), 3);

        let second = paginated_node_summary(&doc, page_id, Some(1), false, 3, 3);
        assert_eq!(second["children_offset"], json!(3));
        assert_eq!(second["more_children"], json!(true));
        let third = paginated_node_summary(&doc, page_id, Some(1), false, 6, 3);
        assert_eq!(third["more_children"], json!(false));

        let walked: Vec<String> = [
            first_window,
            listed_child_ids(&second),
            listed_child_ids(&third),
        ]
        .concat();
        assert_eq!(walked, all_children, "the windows must tile the child list");
    }

    #[test]
    fn an_offset_past_the_end_lists_nothing_rather_than_failing() {
        let (doc, page_id, _) = page_with_children(3);
        let summary = paginated_node_summary(&doc, page_id, Some(1), false, 99, 200);
        assert_eq!(summary["child_count"], json!(3));
        assert_eq!(summary["children_offset"], json!(99));
        assert_eq!(summary["more_children"], json!(false));
        assert!(listed_child_ids(&summary).is_empty());
    }

    /// Only the listed node's own children are windowed; what hangs below a
    /// listed child is governed by `depth` alone, so a windowed listing never
    /// silently drops part of a subtree it did return.
    #[test]
    fn pagination_applies_to_the_top_level_only() {
        let (mut doc, page_id) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame", "name": "Card",
                 "x": 0.0, "y": 0.0, "width": 200.0, "height": 100.0},
            ])),
            "Frame",
        );
        assert_applied(&outcome);
        let frame = created_id(&outcome, 0);
        let requests: Vec<Value> = (0..5)
            .map(|index| {
                json!({"op": "create_node", "node_type": "rectangle", "parent": frame.to_string(),
                       "x": index as f64, "y": 0.0, "width": 4.0, "height": 4.0})
            })
            .collect();
        assert_applied(&run_batch(&mut doc, &ops(json!(requests)), "Rows"));

        let summary = paginated_node_summary(&doc, page_id, Some(2), false, 0, 1);
        let listed = &summary["children"][0];
        assert_eq!(listed["children"].as_array().map(Vec::len), Some(5));
        assert!(listed.get("more_children").is_none());
        assert!(listed.get("children_limit").is_none());
    }

    /// `depth: 0` asks for counts, not children, so it keeps its old shape.
    #[test]
    fn a_depth_zero_listing_still_reports_only_a_child_count() {
        let (doc, page_id, _) = page_with_children(4);
        let summary = paginated_node_summary(&doc, page_id, Some(0), false, 0, 2);
        assert_eq!(summary["child_count"], json!(4));
        assert!(summary.get("children").is_none());
        assert!(summary.get("more_children").is_none());
    }

    #[test]
    fn set_text_style_rejects_weights_outside_the_opentype_range() {
        let (mut doc, _) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "text", "text": "Hi",
                 "x": 0.0, "y": 0.0, "width": 80.0, "height": 20.0},
            ])),
            "Text",
        );
        let text = created_id(&outcome, 0);

        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_text_style", "id": text.to_string(), "font_weight": 1000},
            ])),
            "Weight",
        );
        assert_eq!(outcome.value["applied"], json!(false));
        let error = outcome.value["ops"][0]["error"]
            .as_str()
            .expect("the failed op carries its error");
        assert!(error.contains("between 100 and 900"), "{error}");

        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_text_style", "id": text.to_string(), "font_weight": 900},
            ])),
            "Weight",
        );
        assert_applied(&outcome);
    }

    /// The `style` projection speaks the ops' vocabulary: a value read from it
    /// can be sent back through its op unchanged.
    #[test]
    fn style_values_round_trip_through_their_ops() {
        let (mut doc, _) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame",
                 "x": 0.0, "y": 0.0, "width": 300.0, "height": 200.0},
            ])),
            "Frame",
        );
        let frame = created_id(&outcome, 0).to_string();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_auto_layout", "id": frame, "direction": "vertical", "gap": 8.0,
                 "padding": [16.0, 24.0], "align_items": "stretch", "justify": "space_between",
                 "wrap": false, "counter_gap": 4.0},
                {"op": "set_effects", "id": frame, "effects": [
                    {"kind": "drop_shadow", "color": "#0000001A", "y": 1.0, "blur": 2.0},
                    {"kind": "drop_shadow", "color": "#00000014", "y": 8.0, "blur": 24.0, "spread": -4.0},
                    {"kind": "background_blur", "radius": 12.0},
                ]},
                {"op": "set_fill", "id": frame, "paints": [
                    {"kind": "linear", "stops": [
                        {"position": 0.0, "color": "#FFFFFF"},
                        {"position": 1.0, "color": "#F4F4F5"},
                    ]},
                    {"kind": "solid", "color": "#FFFFFF80"},
                ]},
                {"op": "set_stroke", "id": frame, "color": "#E4E4E7", "width": 1.0,
                 "sides": [0.0, 0.0, 1.0, 0.0], "dash": [4.0, 2.0], "cap": "round"},
                {"op": "set_constraints", "id": frame, "horizontal": "left_right",
                 "vertical": "top"},
            ])),
            "Style",
        );
        assert_applied(&outcome);
        let id: NodeId = frame.parse().unwrap();
        let style = crate::agent_style::node_style(&doc, id);
        assert_eq!(style["auto_layout"]["direction"], "vertical");
        assert_eq!(
            style["auto_layout"]["padding"],
            json!([16.0, 24.0, 16.0, 24.0])
        );
        assert_eq!(style["effects"].as_array().unwrap().len(), 3);
        assert_eq!(style["fills"][0]["kind"], "linear");
        assert_eq!(style["strokes"][0]["sides"], json!([0.0, 0.0, 1.0, 0.0]));
        assert_eq!(style["constraints"]["horizontal"], "left_right");

        // Write every read value back through its op.
        let mut auto_layout = style["auto_layout"].clone();
        auto_layout["op"] = json!("set_auto_layout");
        auto_layout["id"] = json!(frame);
        let mut stroke = style["strokes"][0].clone();
        stroke["op"] = json!("set_stroke");
        stroke["id"] = json!(frame);
        let mut constraints = style["constraints"].clone();
        constraints["op"] = json!("set_constraints");
        constraints["id"] = json!(frame);
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                auto_layout,
                {"op": "set_effects", "id": frame, "effects": style["effects"]},
                {"op": "set_fill", "id": frame, "paints": style["fills"]},
                stroke,
                constraints,
            ])),
            "Round trip",
        );
        assert_applied(&outcome);
        assert_eq!(crate::agent_style::node_style(&doc, id), style);
    }

    #[test]
    fn a_style_listing_is_one_flat_node_per_entry() {
        let (mut doc, page) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "frame",
                 "x": 0.0, "y": 0.0, "width": 200.0, "height": 100.0, "fill": "#FFFFFF"},
            ])),
            "Frame",
        );
        let frame = created_id(&outcome, 0).to_string();
        assert_applied(&run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_auto_layout", "id": frame, "direction": "horizontal", "gap": 12.0},
                {"op": "set_stroke", "id": frame, "width": 1.0},
            ])),
            "Style",
        ));
        let mut listing = paginated_node_summary(&doc, page, Some(1), true, 0, 10);
        crate::agent_style::attach_styles(&doc, &mut listing);
        let entry = listing["children"]
            .as_array()
            .unwrap()
            .iter()
            .find(|child| child["id"] == json!(frame))
            .expect("the frame is listed");
        assert_eq!(entry["fill"], "#FFFFFF", "the listing's own keys stay");
        assert_eq!(
            entry["auto_layout"]["gap"], 12.0,
            "style replaces the mode string"
        );
        assert_eq!(entry["strokes"][0]["width"], 1.0);
        assert!(entry.get("style").is_none());
    }

    #[test]
    fn create_shape_builds_lines_polygons_stars_and_paths() {
        let (mut doc, _) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_shape", "shape": "line", "x": 0.0, "y": 0.0,
                 "width": 120.0, "height": 0.0},
                {"op": "create_shape", "shape": "polygon", "x": 0.0, "y": 0.0,
                 "width": 60.0, "height": 60.0, "points": 6, "fill": "#22C55E"},
                {"op": "create_shape", "shape": "star", "x": 0.0, "y": 0.0,
                 "width": 60.0, "height": 60.0},
                {"op": "create_shape", "shape": "path", "x": 10.0, "y": 20.0,
                 "width": 24.0, "height": 24.0, "path": "M0 0 L24 12 L0 24 Z"},
            ])),
            "Shapes",
        );
        assert_applied(&outcome);
        let vector = |index| {
            let NodeData::Vector(vector) = doc
                .scene
                .get(created_id(&outcome, index))
                .unwrap()
                .data
                .clone()
            else {
                panic!("expected a vector");
            };
            vector
        };
        let line = vector(0);
        assert!(line.fills.is_empty(), "a line is not filled");
        assert_eq!(line.strokes.len(), 1, "a line is stroked by default");
        let polygon = vector(1);
        assert_eq!(polygon.fills.len(), 1);
        assert!(polygon.strokes.is_empty());
        assert_eq!(
            vector(2).fills.len(),
            1,
            "a closed star gets the default fill"
        );
        assert_eq!(
            vector(3).fills.len(),
            1,
            "a closed path gets the default fill"
        );

        let bad = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_shape", "shape": "path", "x": 0.0, "y": 0.0,
                 "width": 1.0, "height": 1.0, "path": "not svg"},
            ])),
            "Bad path",
        );
        assert!(bad.value["error"].is_string(), "{:?}", bad.value);
    }

    #[test]
    fn new_ops_validate_and_undo() {
        let (mut doc, _) = doc_with_page();
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "create_node", "node_type": "rectangle",
                 "x": 0.0, "y": 0.0, "width": 40.0, "height": 40.0},
            ])),
            "Rect",
        );
        let rect = created_id(&outcome, 0);
        let id = rect.to_string();
        for (label, op) in [
            (
                "one stop",
                json!({"op": "set_fill", "id": id, "paints": [
                {"kind": "linear", "stops": [{"position": 0.0, "color": "#000000"}]}]}),
            ),
            (
                "negative blur",
                json!({"op": "set_effects", "id": id, "effects": [
                {"kind": "layer_blur", "radius": -1.0}]}),
            ),
            (
                "negative side",
                json!({"op": "set_stroke", "id": id, "sides": [1.0, -1.0, 0.0, 0.0]}),
            ),
        ] {
            let outcome = run_batch(&mut doc, &ops(json!([op])), label);
            assert!(
                outcome.value["error"].is_string(),
                "{label}: {:?}",
                outcome.value
            );
        }

        assert!(doc.scene.get(rect).unwrap().constraints.is_none());
        let outcome = run_batch(
            &mut doc,
            &ops(json!([
                {"op": "set_constraints", "id": id, "horizontal": "scale"},
            ])),
            "Constraints",
        );
        assert_applied(&outcome);
        assert_eq!(
            doc.scene.get(rect).unwrap().constraints.unwrap().horizontal,
            fanta_doc::ConstraintH::Scale
        );
        assert!(doc.undo().unwrap());
        assert!(doc.scene.get(rect).unwrap().constraints.is_none());
    }
}
