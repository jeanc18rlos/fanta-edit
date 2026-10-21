//! The projections agents read a design through: compact node listings, the
//! style projection attached to them (`crate::style`), the design system, and
//! the free space beside a page's content. The editor's design tools and the
//! cloud engine answer with the same JSON.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, anyhow, bail};
use fanta_design_ops::DesignSystemQuery;
use fanta_doc::{
    Bounds, CanvasNode, Doc, Fill, LayoutMode, NodeData, NodeFlags, NodeId, VariableCollection,
    VariableCollectionId,
};
use serde_json::{Value, json};

/// A page's top-level layers (up to `limit`, `depth` levels deep) in the style
/// projection with world bounds: what `batch_get` returns for a page listing
/// with `detail: "style"`, and what cloud agents read. The depth shrinks until
/// the listing fits the traversal budget.
pub fn page_scene(doc: &Doc, root: NodeId, depth: u32, limit: usize) -> Result<Vec<Value>> {
    let mut depth = depth.min(16);
    loop {
        match check_page_listing_budget(doc, root, Some(depth), 0, limit) {
            Ok(()) => break,
            Err(_) if depth > 1 => depth -= 1,
            Err(error) => return Err(error),
        }
    }
    let mut listing = paginated_node_summary(doc, root, Some(depth), true, 0, limit);
    crate::style::attach_styles(doc, &mut listing);
    Ok(match listing.get_mut("children").map(Value::take) {
        Some(Value::Array(children)) => children,
        _ => Vec::new(),
    })
}

pub const COMPONENT_OVERVIEW_LIMIT: usize = 100;

/// Component masters are not pages, so their sources live in their own list
/// rather than overloading `pages`.
pub fn components_json(
    doc: &Doc,
    project_root: Option<&Path>,
    offset: usize,
    limit: usize,
    source: impl Fn(fanta_format::ScopedDesign) -> Option<PathBuf>,
) -> Vec<Value> {
    doc.components
        .defs
        .values()
        .skip(offset)
        .take(limit)
        .map(|def| {
            let source = source(fanta_format::ScopedDesign::Component(def.id));
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

pub fn design_system_json(
    doc: &Doc,
    project_root: Option<&Path>,
    query: DesignSystemQuery,
    source: impl Fn(fanta_format::ScopedDesign) -> Option<PathBuf>,
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
        "components": components_json(doc, project_root, offset, limit, source),
        "component_count": doc.components.defs.len(),
        "components_offset": offset, "components_limit": limit,
        "more_components": offset.saturating_add(limit) < doc.components.defs.len(),
        "component_sets": doc.components.sets.values().skip(offset).take(limit).collect::<Vec<_>>(),
        "component_set_count": doc.components.sets.len(),
        "component_sets_offset": offset, "component_sets_limit": limit,
        "more_component_sets": offset.saturating_add(limit) < doc.components.sets.len(),
        "selection_bindings": bindings,
        "source_files": ["doc/variables.json", "doc/active_modes.json", "components/<set>/set.json"],
        "hints": ["The same offset and limit page variables, components, and component sets independently; collection filters variables only. Continue while the corresponding more_* flag is true.",
          "Create semantic variables before components, bind properties, then reuse component instances.",
          "Names are exact and must be unique when used instead of ids. Prefer returned ids for follow-up edits.",
          "Variables use per-mode values. set_variable_mode selects a collection mode document-wide or pins a frame."]
    }))
}

pub fn find_collection<'a>(doc: &'a Doc, key: &str) -> Result<&'a VariableCollection> {
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

/// A located source as the project sees it — `pages/<slug>/page.fnx` — which
/// is the form an agent pastes into a file tool. `null` when the document has
/// no project on disk yet (an unsaved `.fig` import).
pub fn relative_source(project_root: Option<&Path>, source: Option<PathBuf>) -> Value {
    let (Some(project_root), Some(source)) = (project_root, source) else {
        return Value::Null;
    };
    match source.strip_prefix(project_root) {
        // Project-relative paths use `/` on every platform, as the project format does.
        Ok(relative) => json!(
            relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        ),
        Err(_) => json!(source.to_string_lossy()),
    }
}

pub fn world_bounds_json(doc: &Doc, id: NodeId) -> Value {
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

pub fn bounds_json(bounds: Option<Bounds>) -> Value {
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
pub fn union_bounds(doc: &Doc, ids: &[NodeId]) -> Option<Bounds> {
    ids.iter()
        .filter_map(|id| doc.scene.world_bounds(*id))
        .filter(Bounds::is_finite)
        .reduce(|union, bounds| union.union(&bounds))
}

/// Union of the top-level content of `page` (`None` for an empty page or a
/// document without pages).
pub fn content_bounds(doc: &Doc, page: Option<NodeId>) -> Option<Bounds> {
    let root = page?;
    union_bounds(doc, doc.scene.children_of(Some(root)))
}

/// Gap kept between existing content and a newly placed frame.
pub const EMPTY_SPACE_MARGIN: f64 = 100.0;

/// A free top-left for a `width` x `height` box on `page`: the origin on an
/// empty page, otherwise the first spot to the right of, then below, the
/// page's content (stepping further out until nothing on the page overlaps).
pub fn empty_space(doc: &Doc, page: NodeId, width: f64, height: f64) -> glam::DVec2 {
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
pub fn page_area_is_empty(doc: &Doc, page: NodeId, rect: Bounds) -> bool {
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

pub const PAGE_LISTING_NODE_BUDGET: usize = 1_000;

// A depth limit does not bound wide imported frames; check with a bounded,
// iterative walk before allocating JSON or asking for every node's geometry.
pub fn check_page_listing_budget(
    doc: &Doc,
    root: NodeId,
    depth: Option<u32>,
    offset: usize,
    limit: usize,
) -> Result<()> {
    if depth == Some(0) {
        return Ok(());
    }
    let children = doc.scene.children_of(Some(root));
    let end = offset.saturating_add(limit).min(children.len());
    let window = children.get(offset..end).unwrap_or_default();
    let error = || {
        anyhow!(
            "the page listing exceeds the {PAGE_LISTING_NODE_BUDGET}-node traversal limit; \
         lower depth, reduce limit with offset to page top-level layers, or fetch relevant nodes by ids"
        )
    };
    if window.len().saturating_add(1) > PAGE_LISTING_NODE_BUDGET {
        return Err(error());
    }
    let child_depth = depth.map(|depth| depth.saturating_sub(1));
    let mut pending = window
        .iter()
        .map(|child| (*child, child_depth))
        .collect::<Vec<_>>();
    let mut visited = 1usize;
    while let Some((node, depth)) = pending.pop() {
        visited += 1;
        if depth == Some(0) {
            continue;
        }
        let children = doc.scene.children_of(Some(node));
        if visited
            .saturating_add(pending.len())
            .saturating_add(children.len())
            > PAGE_LISTING_NODE_BUDGET
        {
            return Err(error());
        }
        let child_depth = depth.map(|depth| depth.saturating_sub(1));
        pending.extend(children.iter().map(|child| (*child, child_depth)));
    }
    Ok(())
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
pub fn paginated_node_summary(
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
pub fn node_summary(doc: &Doc, id: NodeId, depth: Option<u32>, include_geometry: bool) -> Value {
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
pub const SUMMARY_TEXT_CHARS: usize = 120;
pub const SUMMARY_LABEL_CHARS: usize = 160;

pub fn truncate_summary_string(value: &str, max_chars: usize) -> String {
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
