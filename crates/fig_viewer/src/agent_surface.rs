//! The fig_viewer implementation of [`design_surface::DesignSurface`]: lets
//! the agent's native design tools read, edit, and screenshot the most
//! recently opened or focused canvas. Edits go through the same document
//! seam as user input (one history transaction per batch, rolled back on
//! failure), so agent work is undoable like any canvas gesture.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::{Context as _, Result, anyhow, bail};
use base64::Engine as _;
use design_surface::{
    AlignEdge, CrossAxisAlignment, DEFAULT_CHILD_LIMIT, DesignNodeType, DesignOp, DesignSurface,
    DistributeAxis, LayerPosition, LayoutDirection, MainAxisAlignment, NamedLayerPosition,
    NodeQuery, ScreenshotTarget, StrokeAlignment, TextAlignment,
};
use fanta_doc::{
    AssetId, AutoLayout, BitmapNode, Bounds, CanvasNode, Color, ComponentId, CounterAlign, Doc,
    Fill, GroupNode, ImageFitMode, IndexKey, InstanceNode, LayoutMode, NodeData, NodeFlags, NodeId,
    Operation, PathData, PrimaryAlign, ShadowKind, Stroke, StrokeAlign, TextAlign, TextNode,
    TextStyle, Transform2D, UnitInterval, VectorNode, Viewport,
};
use fanta_render::{AssetResolver, RasterRenderer, visual_world_bounds};
use gpui::{App, AppContext as _, Entity, Global, Task, WeakEntity};
use serde_json::{Value, json};
use std::sync::Arc;

use crate::clipboard::{create_operations, duplicate_operations};
use crate::document::{
    AssetStores, DocChange, FigDocument, FigItem, FigPage, MAX_IMAGE_SOURCE_BYTES, page_bounds,
};
use crate::export::render_inputs;
use crate::properties_ops::{
    DEFAULT_FILL_COLOR, create_component_operations, default_shadow, effects_operations,
    parse_color, replace_data_operation, resize_operations, rotation_operations, set_corner_radius,
    stroke_list_mut,
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
        let project_root = item.project_root();
        let selection: Vec<NodeId> = doc.selection.iter().copied().collect();
        Ok(json!({
            "project": item.title().as_ref(),
            "project_root": project_root.map(|root| root.display().to_string()),
            "is_editable": item.is_editable(),
            "source_edit_locked": item.source_edit_locked(),
            "dirty": item.is_dirty(),
            "total_nodes": doc.scene.len(),
            "pages": pages_json(&document.pages, doc, project_root),
            "active_page_bounds": bounds_json(content_bounds(doc, doc.active_page())),
            "components": components_json(doc, project_root),
            "selection": selection_ids(doc),
            "selection_bounds": bounds_json(union_bounds(doc, &selection)),
            "viewport": { "center": doc.viewport.center, "zoom": doc.viewport.zoom },
            "hints": STATE_HINTS,
        }))
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
            "root": paginated_node_summary(
                doc,
                root,
                query.depth,
                query.include_geometry,
                query.offset.unwrap_or(0),
                query.limit.unwrap_or(DEFAULT_CHILD_LIMIT),
            ),
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
                "name": page.name.as_ref(),
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
                "name": def.name,
                "root": def.root.to_string(),
                "source": relative_source(project_root, source),
            })
        })
        .collect()
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

/// What every state read tells the model up front, because the mistakes
/// these prevent (guessed ids, y-up math, unverified results) are the common
/// ones.
const STATE_HINTS: [&str; 5] = [
    "Coordinates are world px with y growing downward; x/y of an op is the node's top-left corner.",
    "Ids are exact node ids from this state or a page listing; never guess or use layer names.",
    "Ask for empty_space before creating a new top-level frame so it does not land on existing work.",
    "Batch related ops into one design_edit/batch_design call with a descriptive label; it is one undo step.",
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

fn summarize_text(content: &str, style: &TextStyle, object: &mut serde_json::Map<String, Value>) {
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
    let mut created: Vec<String> = Vec::new();
    let mut statuses: Vec<Value> = Vec::new();
    let mut ingested_assets: Vec<AssetId> = Vec::new();
    let mut removed_nodes: HashSet<NodeId> = HashSet::new();
    let mut content_changed = false;
    let mut selection_changed = false;
    let mut failure: Option<(usize, String)> = None;

    doc.history.begin(label, &mut doc.scene);
    for (index, op) in ops.iter().enumerate() {
        match apply_one(doc, assets, &mut ingested_assets, &mut removed_nodes, op) {
            Ok(Applied::Content {
                created: id,
                detail,
            }) => {
                content_changed = true;
                let mut status = json!({ "index": index, "status": "ok" });
                if let Some(id) = id {
                    status["created"] = json!(id);
                    created.push(id);
                }
                if let (Some(Value::Object(detail)), Some(status)) =
                    (detail, status.as_object_mut())
                {
                    status.extend(detail);
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
            if doc
                .selection
                .iter()
                .any(|selected| removed_nodes.contains(selected))
            {
                let surviving: Vec<NodeId> = doc
                    .selection
                    .iter()
                    .copied()
                    .filter(|selected| !removed_nodes.contains(selected))
                    .collect();
                doc.selection.replace_with(surviving);
            }
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
            if let Err(abort_error) = doc.abort_transaction() {
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

/// Apply one op. Assets it ingests go into `ingested_assets` and nodes it
/// removes from the scene into `removed_nodes`, both for [`apply_batch`] to
/// settle once the whole batch has committed or rolled back.
fn apply_one(
    doc: &mut Doc,
    assets: &mut AssetStores<'_>,
    ingested_assets: &mut Vec<AssetId>,
    removed_nodes: &mut HashSet<NodeId>,
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
            let parent = resolve_container(doc, parent.as_deref())?;
            let component_id = resolve_component(doc, component)?;
            let def = doc
                .components
                .def(component_id)
                .with_context(|| format!("component {component} does not exist"))?;
            if Some(def.root) == parent
                || parent.is_some_and(|parent| {
                    doc.scene
                        .ancestors_of(parent)
                        .any(|ancestor| ancestor.id == def.root)
                })
            {
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
                }),
            )
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
        } => {
            let id = parse_node_id(id)?;
            let node = existing_node(doc, id)?;
            if !matches!(node.data, NodeData::Group(_)) {
                bail!(
                    "auto layout applies to frames and groups, not a {} node",
                    node.data.kind_tag()
                );
            }
            if let Some(gap) = gap
                && !(gap.is_finite() && *gap >= 0.0)
            {
                bail!("gap must be non-negative");
            }
            let padding = padding.as_deref().map(parse_padding).transpose()?;
            apply_all(
                doc,
                replace_data_operation(doc, id, |data| {
                    let NodeData::Group(group) = data else {
                        return;
                    };
                    let mode = match direction {
                        LayoutDirection::Horizontal => LayoutMode::Horizontal,
                        LayoutDirection::Vertical => LayoutMode::Vertical,
                        LayoutDirection::None => {
                            group.auto_layout = None;
                            return;
                        }
                    };
                    let layout = group.auto_layout.get_or_insert_with(AutoLayout::default);
                    layout.mode = mode;
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
        && doc.components.def(id).is_some()
    {
        return Ok(id);
    }
    let mut matches = doc
        .components
        .defs
        .values()
        .filter(|def| def.name == reference)
        .map(|def| def.id);
    let first = matches
        .next()
        .with_context(|| format!("no component is named or identified by `{reference}`"))?;
    if matches.next().is_some() {
        bail!("several components are named `{reference}`; use the component id");
    }
    Ok(first)
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
}
