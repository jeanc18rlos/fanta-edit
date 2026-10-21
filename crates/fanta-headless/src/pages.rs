//! The page list the editor shows, which page a document opens on, and
//! adding a page.

use std::collections::HashSet;

use fanta_doc::{CanvasNode, Doc, GroupNode, NodeData, NodeId, Operation};

/// Every root the editor lists as a page, in order: the authored pages (one
/// scene-root canvas when there are none), then the component masters that
/// live on roots of their own, which the Pages panel hides.
pub fn editor_page_roots(doc: &Doc) -> Vec<Option<NodeId>> {
    let authored: HashSet<NodeId> = doc.pages().iter().copied().collect();
    let mut roots = if doc.pages().is_empty() {
        vec![None]
    } else {
        doc.pages().iter().copied().map(Some).collect()
    };
    let mut components: Vec<_> = doc
        .components
        .defs
        .values()
        .map(|definition| definition.root)
        .filter(|root| doc.scene.contains(*root) && !authored.contains(root))
        .collect();
    components.sort_unstable();
    components.dedup();
    roots.extend(components.into_iter().map(Some));
    roots
}

/// The name the editor shows for the page at `index` of
/// [`editor_page_roots`], and whether the Pages panel hides it.
pub fn page_label(doc: &Doc, root: Option<NodeId>, index: usize) -> (String, bool) {
    let Some(root) = root else {
        return ("Document".into(), false);
    };
    let name = doc
        .components
        .defs
        .values()
        .find(|definition| definition.root == root)
        .map(|definition| definition.name.as_str())
        .or_else(|| doc.page_name(root))
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Page {}", index + 1));
    (name, page_hidden(doc, root))
}

/// Whether the Pages panel hides the page at `root`: a component master's
/// own root, or a page marked `hidden_page`.
pub fn page_hidden(doc: &Doc, root: NodeId) -> bool {
    !doc.pages().contains(&root)
        || doc
            .scene
            .get(root)
            .and_then(|node| node.meta.get("hidden_page"))
            .and_then(serde_json::Value::as_bool)
            == Some(true)
}

pub fn visible_page_roots(doc: &Doc) -> Vec<NodeId> {
    doc.pages()
        .iter()
        .copied()
        .filter(|page| {
            doc.scene
                .get(*page)
                .and_then(|node| node.meta.get("hidden_page"))
                .and_then(|value| value.as_bool())
                != Some(true)
        })
        .collect()
}

/// The page a document opens on: its active page when that has content,
/// otherwise the visible page with the most.
pub fn default_page_root(doc: &Doc, visible_page_roots: &[NodeId]) -> Option<NodeId> {
    if visible_page_roots.is_empty() {
        return None;
    }

    let active_page = doc
        .active_page()
        .filter(|page| visible_page_roots.contains(page));
    if let Some(active_page) = active_page
        && page_content_area(doc, Some(active_page)) > 1.0
    {
        return Some(active_page);
    }

    visible_page_roots
        .iter()
        .copied()
        .map(|root| (root, page_content_area(doc, Some(root))))
        .max_by(|(_, left_area), (_, right_area)| left_area.total_cmp(right_area))
        .map(|(root, _)| root)
}

pub fn page_content_area(doc: &Doc, page_root: Option<NodeId>) -> f64 {
    crate::render::try_page_bounds(doc, page_root)
        .map(|bounds| bounds.width() * bounds.height())
        .unwrap_or(0.0)
}

/// The operations that add an empty page named `name` after the last authored
/// page and make it the active page. Returns the new page's root.
pub fn add_page_operations(doc: &Doc, name: String) -> (NodeId, Vec<Operation>) {
    let mut node = CanvasNode::new(NodeData::Group(GroupNode::default()));
    node.name = name;
    node.index = doc.scene.next_root_index();
    let root = node.id;
    let old_pages = doc.pages().to_vec();
    let mut new_pages = old_pages.clone();
    new_pages.push(root);
    let registry = Operation::SetPageRegistry {
        old_pages,
        new_pages,
        old_active_page: doc.active_page(),
        new_active_page: Some(root),
    };
    (root, vec![Operation::create_node(node), registry])
}
