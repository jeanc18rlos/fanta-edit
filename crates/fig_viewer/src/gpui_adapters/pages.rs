//! PagesPanel adapter: builds the panel's read models from the document,
//! maps its typed intents onto the existing page operations, and owns the
//! host-side search index (the engine has no page-search concept — G20).

use std::collections::HashMap;

use fanta_doc::{NodeData, NodeId};
use fanta_gpui::pages::{
    PagesPanel, PagesPanelElementCount, PagesPanelElementKind, PagesPanelItem,
    PagesPanelSearchRequest, PagesPanelSearchResult, PagesPanelSearchResults,
    PagesPanelSearchScope,
};
use gpui::{Entity, SharedString, Subscription};

use crate::document::FigDocument;

/// Host-side handle for one panel row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PageRef {
    /// Index into `FigDocument::pages` (the unfiltered list).
    pub index: usize,
    pub root: Option<NodeId>,
}

/// One search hit: where it lives and which field matched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SearchHit {
    pub node: NodeId,
    /// Index into `FigDocument::pages` for page switching on select.
    pub page_index: usize,
    pub field: HitField,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HitField {
    Name,
    TextContent,
}

pub(crate) struct PagesAdapter {
    pub panel: Entity<PagesPanel>,
    pub id_map: HashMap<SharedString, PageRef>,
    pub result_map: HashMap<SharedString, SearchHit>,
    /// Result ids in display order, for prev/next navigation.
    pub result_order: Vec<SharedString>,
    /// The last search the panel asked for, re-run on every document echo
    /// while the search UI is open.
    pub last_search: Option<PagesPanelSearchRequest>,
    pub _subscription: Subscription,
}

/// Stable, opaque panel id for a page row.
pub(crate) fn page_id(root: Option<NodeId>, index: usize) -> SharedString {
    match root {
        Some(root) => SharedString::from(root.to_string()),
        None => SharedString::from(format!("synthetic:{index}")),
    }
}

/// Read model + id map from the document's visible pages, names read live
/// from the scene like the native section does.
pub(crate) fn pages_view_data(
    document: &FigDocument,
) -> (Vec<PagesPanelItem>, HashMap<SharedString, PageRef>) {
    let mut items = Vec::new();
    let mut id_map = HashMap::new();
    for (index, page) in document.pages.iter().enumerate() {
        if page.hidden {
            continue;
        }
        let name = page
            .root
            .and_then(|root| document.doc.scene.get(root))
            .map(|node| SharedString::from(node.name.clone()))
            .unwrap_or_else(|| page.name.clone());
        let id = page_id(page.root, index);
        id_map.insert(
            id.clone(),
            PageRef {
                index,
                root: page.root,
            },
        );
        items.push(PagesPanelItem { id, title: name });
    }
    (items, id_map)
}

/// The 8-way fold from engine node data to the panel's searchable kinds.
/// Component masters can't be told apart from their node data alone, so the
/// caller passes the set of component roots.
pub(crate) fn element_kind(
    id: NodeId,
    data: &NodeData,
    component_roots: &std::collections::HashSet<NodeId>,
) -> PagesPanelElementKind {
    match data {
        NodeData::Text(_) => PagesPanelElementKind::Text,
        NodeData::Instance(_) => PagesPanelElementKind::Instance,
        NodeData::Group(_) if component_roots.contains(&id) => PagesPanelElementKind::Component,
        NodeData::Group(_) => PagesPanelElementKind::FrameGroup,
        NodeData::Bitmap(_) | NodeData::Video(_) => PagesPanelElementKind::Image,
        NodeData::Vector(_) | NodeData::Boolean(_) => PagesPanelElementKind::Shape,
        NodeData::Audio(_)
        | NodeData::NodeGraph(_)
        | NodeData::Model3d(_)
        | NodeData::AiArtifact(_)
        | NodeData::Embed(_) => PagesPanelElementKind::Other,
    }
}

fn matches_query(haystack: &str, query: &str, match_case: bool, whole_words: bool) -> bool {
    if query.is_empty() {
        return false;
    }
    let (haystack_cmp, query_cmp);
    let (haystack_ref, query_ref) = if match_case {
        (haystack, query)
    } else {
        haystack_cmp = haystack.to_lowercase();
        query_cmp = query.to_lowercase();
        (haystack_cmp.as_str(), query_cmp.as_str())
    };
    if !whole_words {
        return haystack_ref.contains(query_ref);
    }
    let mut start = 0;
    while let Some(pos) = haystack_ref[start..].find(query_ref) {
        let begin = start + pos;
        let end = begin + query_ref.len();
        let boundary_before = begin == 0
            || !haystack_ref[..begin]
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric);
        let boundary_after = end == haystack_ref.len()
            || !haystack_ref[end..]
                .chars()
                .next()
                .is_some_and(char::is_alphanumeric);
        if boundary_before && boundary_after {
            return true;
        }
        start = end;
    }
    false
}

/// Replace every match of `query` in `text`, honoring `match_case`.
pub(crate) fn replace_matches(
    text: &str,
    query: &str,
    replacement: &str,
    match_case: bool,
) -> String {
    if query.is_empty() {
        return text.to_string();
    }
    if match_case {
        return text.replace(query, replacement);
    }
    let lower = text.to_lowercase();
    let query = query.to_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(pos) = lower[cursor..].find(&query) {
        let begin = cursor + pos;
        out.push_str(&text[cursor..begin]);
        out.push_str(replacement);
        cursor = begin + query.len();
    }
    out.push_str(&text[cursor..]);
    out
}

/// Run a search over the document. Returns the panel read model plus the
/// host-side hit index keyed by result id, in display order.
pub(crate) fn search_pages(
    document: &FigDocument,
    current_page_index: Option<usize>,
    request: &PagesPanelSearchRequest,
) -> (
    PagesPanelSearchResults,
    HashMap<SharedString, SearchHit>,
    Vec<SharedString>,
) {
    let component_roots: std::collections::HashSet<NodeId> = document
        .doc
        .components
        .defs
        .values()
        .map(|def| def.root)
        .collect();
    let kind_filter: Vec<PagesPanelElementKind> = request
        .element_kinds
        .iter()
        .copied()
        .filter(|kind| *kind != PagesPanelElementKind::All)
        .collect();

    let mut items = Vec::new();
    let mut hit_map = HashMap::new();
    let mut order = Vec::new();
    let mut counts: HashMap<PagesPanelElementKind, usize> = HashMap::new();
    let mut total_all_kinds = 0usize;

    let doc = &document.doc;
    for (page_index, page) in document.pages.iter().enumerate() {
        if page.hidden {
            continue;
        }
        if request.scope == PagesPanelSearchScope::CurrentPage
            && current_page_index.is_some_and(|current| current != page_index)
        {
            continue;
        }
        let page_name = page.name.clone();
        let nodes: Vec<NodeId> = match page.root {
            Some(root) => doc
                .scene
                .descendants_of(root)
                .filter(|id| *id != root)
                .collect(),
            None => doc
                .scene
                .roots()
                .iter()
                .flat_map(|root| doc.scene.descendants_of(*root))
                .collect(),
        };
        for id in nodes {
            let Some(node) = doc.scene.get(id) else {
                continue;
            };
            let name_hit = matches_query(
                &node.name,
                &request.query,
                request.match_case,
                request.whole_words,
            );
            let text_hit = match &node.data {
                NodeData::Text(text) => matches_query(
                    &text.content,
                    &request.query,
                    request.match_case,
                    request.whole_words,
                ),
                _ => false,
            };
            if !name_hit && !text_hit {
                continue;
            }
            let kind = element_kind(id, &node.data, &component_roots);
            total_all_kinds += 1;
            *counts.entry(kind).or_default() += 1;
            if !kind_filter.is_empty() && !kind_filter.contains(&kind) {
                continue;
            }
            let result_id = SharedString::from(id.to_string());
            let title = if name_hit || node.name.is_empty() {
                SharedString::from(node.name.clone())
            } else if let NodeData::Text(text) = &node.data {
                SharedString::from(text.content.clone())
            } else {
                SharedString::from(node.name.clone())
            };
            items.push(PagesPanelSearchResult {
                id: result_id.clone(),
                title,
                parent: Some(page_name.clone()),
                kind,
            });
            hit_map.insert(
                result_id.clone(),
                SearchHit {
                    node: id,
                    page_index,
                    field: if name_hit {
                        HitField::Name
                    } else {
                        HitField::TextContent
                    },
                },
            );
            order.push(result_id);
        }
    }

    let mut element_counts = vec![PagesPanelElementCount::new(
        PagesPanelElementKind::All,
        total_all_kinds,
    )];
    for kind in PagesPanelElementKind::FILTER_ORDER {
        if kind == PagesPanelElementKind::All {
            continue;
        }
        element_counts.push(PagesPanelElementCount::new(
            kind,
            counts.get(&kind).copied().unwrap_or(0),
        ));
    }

    let total = items.len();
    (
        PagesPanelSearchResults {
            total,
            items,
            element_counts,
        },
        hit_map,
        order,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_word_matching_respects_boundaries() {
        assert!(matches_query("Primary Button", "button", false, true));
        assert!(!matches_query("Buttons", "button", false, true));
        assert!(matches_query("nav-button-icon", "button", false, true));
        assert!(matches_query("Button", "button", false, true));
        assert!(!matches_query("Button", "button", true, true));
    }

    #[test]
    fn replace_matches_is_case_aware() {
        assert_eq!(
            replace_matches("Hello hello HELLO", "hello", "hi", false),
            "hi hi hi"
        );
        assert_eq!(
            replace_matches("Hello hello", "hello", "hi", true),
            "Hello hi"
        );
        assert_eq!(replace_matches("untouched", "", "x", false), "untouched");
    }
}
