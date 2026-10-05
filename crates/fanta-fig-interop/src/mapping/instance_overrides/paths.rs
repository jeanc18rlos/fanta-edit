//! Resolving Figma `guidPath`s into flat def-local `OverridePath`s across
//! nested-instance masters: swap redirects, master roots, and path helpers.

use super::{
    BoundProp, ComponentId, Doc, HashMap, KiwiValue, NodeData, NodeId, Override, OverridePath,
    OverrideValue, guid_key,
};

pub(crate) struct SwapRedirect {
    pub(crate) master_root: NodeId,
    pub(crate) source_order: usize,
}

pub(crate) type InstanceSwapRedirects = HashMap<NodeId, HashMap<String, SwapRedirect>>;

pub(crate) fn insert_swap_redirect(
    redirects: &mut HashMap<String, SwapRedirect>,
    path: String,
    master_root: NodeId,
    source_order: usize,
) {
    if redirects
        .get(&path)
        .is_none_or(|existing| existing.source_order <= source_order)
    {
        redirects.insert(
            path,
            SwapRedirect {
                master_root,
                source_order,
            },
        );
    }
}

pub(crate) struct ResolvedOverrideTarget {
    pub(crate) path: OverridePath,
    pub(crate) instance_master: Option<NodeId>,
}

/// Path-resolution state shared by every instance of one import: the guid of
/// each mapped node (the inverse of `guid_to_node`, built once) and each
/// master's `guid → def-local-path` map, built the first time an override
/// addresses that master. The shared inverse is the point: building it per
/// master meant scanning the whole `guid_to_node` map once per distinct
/// master, which is quadratic over a component-heavy file.
pub(crate) struct MasterPathCache<'a> {
    node_to_guid: HashMap<NodeId, &'a str>,
    per_master: HashMap<NodeId, HashMap<String, OverridePath>>,
}

impl<'a> MasterPathCache<'a> {
    pub(crate) fn new(guid_to_node: &'a HashMap<String, Option<NodeId>>) -> Self {
        let node_to_guid = guid_to_node
            .iter()
            .filter_map(|(guid, id)| id.map(|id| (id, guid.as_str())))
            .collect();
        Self {
            node_to_guid,
            per_master: HashMap::new(),
        }
    }

    pub(crate) fn node_guid(&self, node: NodeId) -> Option<&str> {
        self.node_to_guid.get(&node).copied()
    }

    /// Borrow (building + caching on first use) the `guid → def-local-path` map
    /// for the master rooted at `master_root`. Cached per master so the
    /// cross-master walk and repeated instances of the same master don't
    /// rebuild it.
    pub(crate) fn master_guid_paths(
        &mut self,
        doc: &Doc,
        master_root: NodeId,
    ) -> &HashMap<String, OverridePath> {
        self.per_master
            .entry(master_root)
            .or_insert_with(|| build_master_guid_paths(doc, master_root, &self.node_to_guid))
    }
}

/// Resolve a full Figma `guidPath` (a `KiwiValue` array of guids) into one flat
/// def-local [`OverridePath`] spanning nested-instance masters.
///
/// Each guid is resolved to its def-local path within the *current* master and
/// the segments are concatenated. Between segments, the guid must name a nested
/// `InstanceNode` in the current master; we descend into that instance's
/// component master for the next guid. The first guid is resolved in
/// `master_root`. Returns `None` if any guid fails to resolve (the path crosses a
/// boundary we can't model — tolerated, never fatal) — except the COMMON case
/// where a length-1 path's single guid resolves directly.
///
/// The returned path's segment boundaries are exactly where
/// [`fanta_doc::resolve::expand_instance`] peels matched-instance prefixes, so a
/// length-1 path applies at the top level and a longer path routes onto the
/// nested instance level by level.
pub(crate) fn resolve_full_guid_path(
    doc: &Doc,
    master_root: NodeId,
    guid_to_node: &HashMap<String, Option<NodeId>>,
    guids: &[KiwiValue],
    swap_redirects: &InstanceSwapRedirects,
    instance: NodeId,
    cache: &mut MasterPathCache<'_>,
) -> Option<OverridePath> {
    resolve_override_target(
        doc,
        master_root,
        guid_to_node,
        guids,
        swap_redirects,
        instance,
        cache,
    )
    .map(|target| target.path)
}

pub(crate) fn resolve_override_target(
    doc: &Doc,
    master_root: NodeId,
    guid_to_node: &HashMap<String, Option<NodeId>>,
    guids: &[KiwiValue],
    swap_redirects: &InstanceSwapRedirects,
    instance: NodeId,
    cache: &mut MasterPathCache<'_>,
) -> Option<ResolvedOverrideTarget> {
    if guids.is_empty() {
        return None;
    }
    let mut current_root = master_root;
    let mut out: OverridePath = OverridePath::new();
    // Outer placement overrides are applied after a nested instance's own
    // overrides during expansion, so they must win when both swap the same slot.
    let mut contexts: Vec<_> = swap_redirects
        .get(&instance)
        .map(|redirects| (redirects, String::new()))
        .into_iter()
        .collect();
    for (i, g) in guids.iter().enumerate() {
        let guid = guid_key(g)?;
        for (_, prefix) in &mut contexts {
            if !prefix.is_empty() {
                prefix.push('>');
            }
            prefix.push_str(&guid);
        }
        // MAIN-vs-PUBLISHED ROOT CASE. Figma roots a `guidPath` at the SYMBOL the
        // instance references: a path segment whose guid is the *current master's
        // own root* addresses the expansion ROOT at this level (def-local path
        // `[]`), not a descendant. `build_master_guid_paths` deliberately EXCLUDES
        // the root (it maps only descendants), so a root-targeted segment isn't in
        // that map and used to resolve to `None` — silently dropping ~14.5k
        // length-1 root overrides/derived per the Spectrum fixture (the instance's
        // own resolved surface fill + baked root size/transform/geometry). We
        // detect it directly: the guid maps to the current master root NodeId.
        // The segment is empty (the root), and the node to descend into for any
        // following guid is the root itself.
        let (seg, last) = if guid_to_node.get(&guid).copied().flatten() == Some(current_root) {
            (OverridePath::new(), current_root)
        } else {
            let seg = cache
                .master_guid_paths(doc, current_root)
                .get(&guid)
                .cloned()?;
            let last = *seg.last()?;
            (seg, last)
        };
        out.extend(seg);
        let node = doc.scene.get(last)?;
        let instance_master = match &node.data {
            NodeData::Instance(nested) => contexts
                .iter()
                .find_map(|(redirects, prefix)| redirects.get(prefix).map(|swap| swap.master_root))
                .or_else(|| master_root_for(doc, nested.component)),
            _ if last == current_root => Some(current_root),
            _ => None,
        };
        if i + 1 == guids.len() {
            return Some(ResolvedOverrideTarget {
                path: out,
                instance_master,
            });
        }
        if !matches!(node.data, NodeData::Instance(_)) {
            return None;
        }
        current_root = instance_master?;
        if let Some(redirects) = swap_redirects.get(&last) {
            contexts.push((redirects, String::new()));
        }
    }
    None
}

/// Build the per-instance SWAP REDIRECT map for nested override resolution.
///
/// A nested instance can be swapped to a different variant by an
/// `overriddenSymbolID` symbolOverride; the instance's declared `symbolID` still
/// names the original (published) master, but a *content* override whose
/// `guidPath` descends through that instance addresses the SWAPPED master's
/// descendants. So [`resolve_full_guid_path`] must descend into the swapped
/// master, not the declared one. We key each swap by the joined SOURCE guidPath
/// prefix (the `>`-separated guids of the swap entry's own `guidPath` — the path
/// naming the swapped nested instance), mapped to the swapped component's master
/// ROOT [`NodeId`] (via `master_root` — a member def's root, or a set's default
/// member's root). A swap whose target component or root doesn't resolve is
/// skipped (tolerated — the override still routes to the declared master, the
/// pre-existing behavior).
pub(crate) fn build_swap_redirects(
    symbol_overrides: &[KiwiValue],
    symbol_guid_to_component: &HashMap<String, ComponentId>,
    master_root: impl Fn(ComponentId) -> Option<NodeId>,
) -> HashMap<String, SwapRedirect> {
    let mut out = HashMap::new();
    for (source_order, ov) in symbol_overrides.iter().enumerate() {
        let Some(swap_guid) = ov.get("overriddenSymbolID").and_then(guid_key) else {
            continue;
        };
        let Some(&cid) = symbol_guid_to_component.get(&swap_guid) else {
            continue;
        };
        let Some(root) = master_root(cid) else {
            continue;
        };
        let Some(guids) = ov
            .get("guidPath")
            .and_then(|p| p.get("guids"))
            .and_then(KiwiValue::as_array)
        else {
            continue;
        };
        let key: Vec<String> = guids.iter().filter_map(guid_key).collect();
        if key.len() == guids.len() && !key.is_empty() {
            insert_swap_redirect(&mut out, key.join(">"), root, source_order);
        }
    }
    out
}

/// The master-subtree root `NodeId` an instance of `component` expands against:
/// a member def's own root, or — for a component *set* — the default member's
/// root. `None` when the id resolves to neither (dangling).
pub(crate) fn master_root_for(doc: &Doc, component: ComponentId) -> Option<NodeId> {
    if let Some(def) = doc.components.def(component) {
        return Some(def.root);
    }
    let set = doc.components.sets.get(&component)?;
    doc.components.def(set.default_variant).map(|d| d.root)
}

/// Build a `master-descendant-guid → def-local OverridePath` map for the master
/// rooted at `master_root`. The path is the original master `NodeId`s from the
/// root's child down to the descendant (root excluded) — exactly the `def_path`
/// [`fanta_doc::resolve::expand_instance`] records and matches overrides against.
///
/// `node_to_guid` is the inverse of the import's `guid_to_node` map (every
/// mapped node's guid); only this master's subtree is walked.
pub(crate) fn build_master_guid_paths(
    doc: &Doc,
    master_root: NodeId,
    node_to_guid: &HashMap<NodeId, &str>,
) -> HashMap<String, OverridePath> {
    let mut out: HashMap<String, OverridePath> = HashMap::new();
    for id in doc.scene.descendants_of(master_root) {
        if id == master_root {
            continue; // the root addresses the instance itself, not a descendant
        }
        if let Some(guid) = node_to_guid.get(&id) {
            out.insert(
                (*guid).to_owned(),
                def_local_path(&doc.scene, master_root, id),
            );
        }
    }
    out
}

/// The def-local path of `node` within the master rooted at `root`: original
/// master ids from the root's child down to `node` (root excluded). Mirrors
/// `fanta_doc::resolve::def_local_path` (which is private to that crate).
pub(crate) fn def_local_path(
    scene: &fanta_doc::scene::Scene,
    root: NodeId,
    node: NodeId,
) -> OverridePath {
    let mut up: Vec<NodeId> = Vec::new();
    for anc in scene.ancestors_of(node) {
        if anc.id == root {
            break;
        }
        up.push(anc.id);
    }
    up.reverse();
    let mut path: OverridePath = up.into_iter().collect();
    path.push(node);
    path
}

/// A text-content [`Override`] at `path`.
pub(crate) fn text_override(path: OverridePath, value: &str) -> Override {
    Override {
        target_path: path,
        target_prop: BoundProp::TextContent,
        value: OverrideValue::Text {
            value: value.to_owned(),
        },
    }
}
