//! Resolving Figma `guidPath`s into flat def-local `OverridePath`s across
//! nested-instance masters: swap redirects, master roots, and path helpers.

use super::{
    BoundProp, ComponentId, Doc, HashMap, KiwiValue, NodeData, NodeId, Override, OverridePath,
    OverrideValue, guid_key,
};

/// Borrow (building + caching on first use) the `guid → def-local-path` map for
/// the master rooted at `master_root`. Cached per master so the cross-master
/// walk and repeated instances of the same master don't rebuild it.
pub(crate) fn master_guid_paths<'c>(
    doc: &Doc,
    master_root: NodeId,
    guid_to_node: &HashMap<String, Option<NodeId>>,
    cache: &'c mut HashMap<NodeId, HashMap<String, OverridePath>>,
) -> &'c HashMap<String, OverridePath> {
    cache
        .entry(master_root)
        .or_insert_with(|| build_master_guid_paths(doc, master_root, guid_to_node))
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
    swap_redirects: &HashMap<String, NodeId>,
    cache: &mut HashMap<NodeId, HashMap<String, OverridePath>>,
) -> Option<OverridePath> {
    if guids.is_empty() {
        return None;
    }
    let mut current_root = master_root;
    let mut out: OverridePath = OverridePath::new();
    // The joined SOURCE guidPath prefix walked so far (guids, '>'-separated),
    // used to look up a sibling swap that re-points the instance we're about to
    // descend into (see `swap_redirects` / the nested duality).
    let mut src_prefix = String::new();
    for (i, g) in guids.iter().enumerate() {
        let guid = guid_key(g)?;
        if i == 0 {
            src_prefix.push_str(&guid);
        } else {
            src_prefix.push('>');
            src_prefix.push_str(&guid);
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
            let seg = {
                let paths = master_guid_paths(doc, current_root, guid_to_node, cache);
                paths.get(&guid).cloned()?
            };
            let last = *seg.last()?;
            (seg, last)
        };
        out.extend(seg);
        // For every guid but the last, descend into the nested instance it names.
        if i + 1 < guids.len() {
            // SWAP REDIRECT (the nested main-vs-published duality): if a sibling
            // `overriddenSymbolID` override swapped THIS nested instance (keyed by
            // the source-guid prefix up to and including it), descend into the
            // SWAPPED master's root — the subsequent segments address the swapped
            // variant's descendants, not the declared symbolID's. Otherwise descend
            // into the instance's declared component master as before.
            let next_root = if let Some(&swapped_root) = swap_redirects.get(&src_prefix) {
                swapped_root
            } else {
                match doc.scene.get(last).map(|n| &n.data) {
                    Some(NodeData::Instance(inst)) => master_root_for(doc, inst.component),
                    _ => None,
                }?
            };
            current_root = next_root;
        }
    }
    Some(out)
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
) -> HashMap<String, NodeId> {
    let mut out: HashMap<String, NodeId> = HashMap::new();
    for ov in symbol_overrides {
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
            out.insert(key.join(">"), root);
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
/// We invert `guid_to_node` (guid → NodeId) restricted to this master's subtree,
/// then compute each descendant's def-local path from the scene.
pub(crate) fn build_master_guid_paths(
    doc: &Doc,
    master_root: NodeId,
    guid_to_node: &HashMap<String, Option<NodeId>>,
) -> HashMap<String, OverridePath> {
    // NodeId → guid for the master subtree (reverse of the relevant slice).
    let subtree: std::collections::HashSet<NodeId> =
        doc.scene.descendants_of(master_root).collect();
    let mut node_to_guid: HashMap<NodeId, String> = HashMap::new();
    for (g, opt) in guid_to_node {
        if let Some(id) = opt {
            if subtree.contains(id) {
                node_to_guid.insert(*id, g.clone());
            }
        }
    }
    let mut out: HashMap<String, OverridePath> = HashMap::new();
    for id in &subtree {
        if *id == master_root {
            continue; // the root addresses the instance itself, not a descendant
        }
        if let Some(g) = node_to_guid.get(id) {
            out.insert(g.clone(), def_local_path(&doc.scene, master_root, *id));
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
