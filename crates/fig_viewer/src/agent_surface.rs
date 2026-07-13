//! The fig_viewer implementation of [`design_surface::DesignSurface`]: lets
//! the agent's native design tools read, edit, and screenshot the most
//! recently opened or focused canvas. Edits go through the same document
//! seam as user input (one history transaction per batch, rolled back on
//! failure), so agent work is undoable like any canvas gesture.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use anyhow::{Context as _, Result, anyhow, bail};
use base64::Engine as _;
use design_surface::{DesignNodeType, DesignOp, DesignSurface, NodeQuery, ScreenshotTarget};
use fanta_doc::{
    AssetId, BitmapNode, CanvasNode, Color, Doc, Fill, GroupNode, ImageFitMode, IndexKey, NodeData,
    NodeFlags, NodeId, Operation, PathData, TextNode, Transform2D, UnitInterval, VectorNode,
    Viewport,
};
use fanta_render::{AssetResolver, RasterRenderer, visual_world_bounds};
use gpui::{App, AppContext as _, Entity, Global, Task, WeakEntity};
use serde_json::{Value, json};
use std::sync::Arc;

use crate::document::{AssetStores, DocChange, FigDocument, FigItem, page_bounds};
use crate::export::render_inputs;
use crate::properties_ops::{
    DEFAULT_FILL_COLOR, parse_color, replace_data_operation, resize_operations, set_corner_radius,
};

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

impl DesignSurface for FigDesignSurface {
    fn state(&self, cx: &mut App) -> Result<Value> {
        let item = self.item()?;
        let item = item.read(cx);
        let document = ready_document(item)?;
        let doc = &document.doc;
        let active_page = doc.active_page();
        let pages: Vec<Value> = document
            .pages
            .iter()
            .enumerate()
            .map(|(index, page)| {
                json!({
                    "index": index,
                    "name": page.name.as_ref(),
                    "root": page.root.map(|id| id.to_string()),
                    "hidden": page.hidden,
                    "active": page.root.is_some() && page.root == active_page,
                    "nodes": page
                        .root
                        .map(|root| doc.scene.descendants_of(root).count().saturating_sub(1))
                        .unwrap_or(0),
                })
            })
            .collect();
        Ok(json!({
            "project": item.title().as_ref(),
            "project_root": item.project_root().map(|root| root.display().to_string()),
            "is_editable": item.is_editable(),
            "source_edit_locked": item.source_edit_locked(),
            "dirty": item.is_dirty(),
            "total_nodes": doc.scene.len(),
            "pages": pages,
            "selection": selection_ids(doc),
            "viewport": { "center": doc.viewport.center, "zoom": doc.viewport.zoom },
        }))
    }

    fn get_nodes(&self, query: NodeQuery, cx: &mut App) -> Result<Value> {
        let item = self.item()?;
        let item = item.read(cx);
        let document = ready_document(item)?;
        let doc = &document.doc;

        if let Some(ids) = &query.ids {
            let mut nodes = Vec::with_capacity(ids.len());
            for raw in ids {
                let id = parse_node_id(raw)?;
                let node = doc
                    .scene
                    .get(id)
                    .with_context(|| format!("node {raw} does not exist"))?;
                let mut value = serde_json::to_value(node)?;
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
        Ok(json!({
            "page": page_index,
            "name": page.name.as_ref(),
            "root": node_summary(doc, root, query.depth, query.include_geometry),
        }))
    }

    fn apply(&self, ops: Vec<DesignOp>, label: String, cx: &mut App) -> Result<Value> {
        if ops.is_empty() {
            bail!("the ops list is empty");
        }
        let item = self.item()?;
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
                let outcome = apply_batch(doc, &mut assets, &ops, &label);
                (Ok(outcome.value), outcome.change)
            })
            .unwrap_or_else(|| Err(anyhow!("the document is no longer available")))
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
        let max_dimension = f64::from(target.max_dimension.unwrap_or(1024).clamp(16, 4096));
        cx.background_spawn(async move {
            let bounds = match node {
                Some(node) => visual_world_bounds(&doc.scene, node, 0.0)
                    .context("the node has no visible bounds")?,
                None => page_bounds(&doc, page_root),
            };
            if !bounds.is_finite() || bounds.width() <= 0.0 || bounds.height() <= 0.0 {
                bail!("the screenshot target has invalid or empty bounds");
            }
            // Fit within the dimension cap; allow mild upscale so small nodes
            // (icons) stay legible without ballooning the surface.
            let zoom = (max_dimension / bounds.width().max(bounds.height())).min(2.0);
            let width = (bounds.width() * zoom).ceil().max(1.0) as u32;
            let height = (bounds.height() * zoom).ceil().max(1.0) as u32;
            let mut renderer = RasterRenderer::new(width, height).map_err(|error| {
                anyhow!("creating {width}x{height} screenshot surface: {error}")
            })?;
            if let Some(asset_resolver) = asset_resolver {
                renderer.set_asset_resolver(asset_resolver);
            }
            let center = bounds.center();
            let viewport = Viewport {
                center: [center.x, center.y],
                zoom,
            };
            let inputs = render_inputs(&doc);
            renderer.render_page_with(&doc.scene, &viewport, page_root, &inputs);
            renderer
                .encode_png()
                .map_err(|error| anyhow!("encoding screenshot PNG: {error}"))
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

fn world_bounds_json(doc: &Doc, id: NodeId) -> Value {
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

/// A compact node-tree projection for page listings: enough for the model to
/// navigate and target nodes without the full serde payload (fetch specific
/// ids for that).
fn node_summary(doc: &Doc, id: NodeId, depth: Option<u32>, include_geometry: bool) -> Value {
    let Some(node) = doc.scene.get(id) else {
        return Value::Null;
    };
    let mut object = serde_json::Map::new();
    object.insert("id".into(), json!(id.to_string()));
    object.insert("kind".into(), json!(node.data.kind_tag()));
    object.insert("name".into(), json!(node.name));
    if node.flags.contains(NodeFlags::HIDDEN) {
        object.insert("hidden".into(), json!(true));
    }
    if node.flags.contains(NodeFlags::LOCKED) {
        object.insert("locked".into(), json!(true));
    }
    if let NodeData::Text(text) = &node.data {
        object.insert("text".into(), json!(text.content));
    }
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
    if root.join("fanta.json").is_file() {
        files.push("fanta.json".to_string());
    }
    for (directory, file_name) in [("pages", "page.fnx"), ("components", "master.fnx")] {
        let Ok(entries) = std::fs::read_dir(root.join(directory)) else {
            continue;
        };
        let mut found: Vec<String> = entries
            .flatten()
            .filter(|entry| entry.path().join(file_name).is_file())
            .map(|entry| {
                format!(
                    "{directory}/{}/{file_name}",
                    entry.file_name().to_string_lossy()
                )
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
    Content { created: Option<String> },
    Selection,
    Viewport,
    Nothing,
}

/// Apply the batch inside one history transaction: all content ops commit as
/// a single undo step, and any failure rolls the content back (selection and
/// viewport changes are not transactional). Image assets ingested by
/// `create_image` ops are removed again when the batch rolls back.
fn apply_batch(
    doc: &mut Doc,
    assets: &mut AssetStores<'_>,
    ops: &[DesignOp],
    label: &str,
) -> BatchOutcome {
    let mut created: Vec<String> = Vec::new();
    let mut statuses: Vec<Value> = Vec::new();
    let mut ingested_assets: Vec<AssetId> = Vec::new();
    let mut content_changed = false;
    let mut selection_changed = false;
    let mut failure: Option<(usize, String)> = None;

    doc.history.begin(label, &mut doc.scene);
    for (index, op) in ops.iter().enumerate() {
        match apply_one(doc, assets, &mut ingested_assets, op) {
            Ok(Applied::Content { created: id }) => {
                content_changed = true;
                let mut status = json!({ "index": index, "status": "ok" });
                if let Some(id) = id {
                    status["created"] = json!(id);
                    created.push(id);
                }
                statuses.push(status);
            }
            Ok(Applied::Selection) => {
                selection_changed = true;
                statuses.push(json!({ "index": index, "status": "ok" }));
            }
            Ok(Applied::Viewport | Applied::Nothing) => {
                statuses.push(json!({ "index": index, "status": "ok" }));
            }
            Err(error) => {
                failure = Some((index, format!("{error:#}")));
                break;
            }
        }
    }

    match failure {
        None => {
            doc.history.commit(&mut doc.scene);
            BatchOutcome {
                value: json!({ "applied": true, "created": created, "ops": statuses }),
                change: if content_changed {
                    DocChange::Content
                } else if selection_changed {
                    DocChange::Selection
                } else {
                    DocChange::None
                },
            }
        }
        Some((index, mut message)) => {
            if let Err(abort_error) = doc.history.abort(&mut doc.scene) {
                message = format!("{message}; rolling back also failed: {abort_error}");
            }
            for asset in ingested_assets {
                assets.remove(asset);
            }
            statuses.push(json!({ "index": index, "status": "failed", "error": message }));
            for skipped in index + 1..ops.len() {
                statuses.push(json!({ "index": skipped, "status": "skipped" }));
            }
            BatchOutcome {
                value: json!({
                    "applied": false,
                    "error": format!("op {index} failed; the batch was rolled back"),
                    "ops": statuses,
                }),
                change: if selection_changed {
                    DocChange::Selection
                } else {
                    DocChange::None
                },
            }
        }
    }
}

fn apply_one(
    doc: &mut Doc,
    assets: &mut AssetStores<'_>,
    ingested_assets: &mut Vec<AssetId>,
    op: &DesignOp,
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
            let parent = resolve_container(doc, parent.as_deref())?;
            let bytes = decode_image_source(source)?;
            let (asset, natural_size) = assets.add_image(bytes)?;
            // Track before any fallible step so a later failure in this batch
            // rolls the asset back out of the stores too.
            ingested_assets.push(asset);

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
            Ok(Applied::Content {
                created: Some(id.to_string()),
            })
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
            let parent = resolve_container(doc, parent.as_deref())?;
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
            Ok(Applied::Content {
                created: Some(id.to_string()),
            })
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
                Ok(Applied::Content { created: None })
            } else {
                Ok(Applied::Nothing)
            }
        }
        DesignOp::Reparent { id, parent, index } => {
            let id = parse_node_id(id)?;
            let node = doc
                .scene
                .get(id)
                .with_context(|| format!("node {id} does not exist"))?
                .clone();
            let new_parent = resolve_container(doc, parent.as_deref())?;
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
            Ok(Applied::Content { created: None })
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
            let deleted: Vec<NodeId> = snapshot.iter().map(|node| node.id).collect();
            doc.apply(Operation::DeleteSubtree { snapshot })?;
            let surviving: Vec<NodeId> = doc
                .selection
                .iter()
                .copied()
                .filter(|selected| !deleted.contains(selected))
                .collect();
            doc.selection.replace_with(surviving);
            Ok(Applied::Content { created: None })
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

/// Resolve an optional parent id to a container node, defaulting to the
/// active page.
fn resolve_container(doc: &Doc, parent: Option<&str>) -> Result<Option<NodeId>> {
    match parent {
        Some(raw) => {
            let id = parse_node_id(raw)?;
            let node = doc
                .scene
                .get(id)
                .with_context(|| format!("parent {raw} does not exist"))?;
            if !node.can_have_children() {
                bail!(
                    "parent {raw} is a {} node and cannot have children",
                    node.data.kind_tag()
                );
            }
            Ok(Some(id))
        }
        None => Ok(doc.active_page()),
    }
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

/// Cap on a `create_image` payload after base64 decoding. Generous for any
/// generated PNG while keeping a bad tool call from ballooning the document.
const MAX_IMAGE_SOURCE_BYTES: usize = 32 * 1024 * 1024;

/// Decode a `create_image` source: a `data:` URI or raw base64. URLs are
/// deliberately not fetched here — the surface is synchronous and network
/// access belongs to the tool layer (`place_generation`), which downloads and
/// re-issues the op with base64.
fn decode_image_source(source: &str) -> Result<Vec<u8>> {
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
}
