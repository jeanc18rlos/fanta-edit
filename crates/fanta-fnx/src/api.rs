//! High-level codec: a page/component subtree's node values ⇄ a `.fnx` source
//! string + an `.ids` sidecar. This is the seam `fanta-format` plugs into,
//! replacing one-JSON-file-per-node with one readable `.fnx` (+ sidecar) per
//! page/component.

use crate::convert::{FnxError, nodes_from_tree, tree_from_nodes};
use crate::model::{FnxElement, IdEntry};
use crate::parse::{parse_doc, parse_doc_with};
use crate::print::{print_doc, print_doc_with};
use crate::refs::RefTable;
use serde_json::Value;
use std::collections::HashMap;

/// The companion sidecar for one `.fnx` file: the root's external parent id
/// (so the subtree re-links into the wider scene) and the pre-order id/index of
/// every element (the identity + sibling order the readable source omits).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FnxSidecar {
    /// The id this subtree's root node's `parent` points at, or `None` for a
    /// true scene root (a page). Omitted when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_parent: Option<String>,
    /// One entry per element, in the same pre-order the `.fnx` is written/read.
    pub ids: Vec<IdEntry>,
}

/// Encode one single-root subtree (a slice of node JSON values) into `.fnx`
/// source + its sidecar. `fn_name` is the cosmetic function name.
pub fn encode_subtree(nodes: &[Value], fn_name: &str) -> Result<(String, FnxSidecar), FnxError> {
    let tree = tree_from_nodes(nodes)?;
    let text = print_doc(fn_name, &tree.root);
    Ok((
        text,
        FnxSidecar {
            root_parent: tree.root_parent,
            ids: tree.sidecar,
        },
    ))
}

/// [`encode_subtree`] with a name-emission context: component/variable ids in
/// the printed source re-sugar into names when the table opted into
/// `emit_names` (see [`crate::refs`]). The sidecar is reference-free either
/// way — ids in it are NODE ids, which never sugar.
pub fn encode_subtree_with(
    nodes: &[Value],
    fn_name: &str,
    refs: &RefTable,
) -> Result<(String, FnxSidecar), FnxError> {
    let tree = tree_from_nodes(nodes)?;
    let text = print_doc_with(fn_name, &tree.root, refs);
    Ok((
        text,
        FnxSidecar {
            root_parent: tree.root_parent,
            ids: tree.sidecar,
        },
    ))
}

/// Decode a `.fnx` source string + its sidecar back into the flat node values.
pub fn decode_subtree(text: &str, sidecar: &FnxSidecar) -> Result<Vec<Value>, FnxError> {
    let root = parse_doc(text)?;
    nodes_from_tree(&root, &sidecar.ids, sidecar.root_parent.as_deref())
}

/// [`decode_subtree`] with a name-resolution context: `component="Button"` and
/// `$Collection/Name` binding paths resolve into canonical ULIDs before the
/// node values are rebuilt, so no name ever reaches a node map or the doc
/// model (see [`crate::refs`]).
pub fn decode_subtree_with(
    text: &str,
    sidecar: &FnxSidecar,
    refs: &RefTable,
) -> Result<Vec<Value>, FnxError> {
    let root = parse_doc_with(text, refs)?;
    nodes_from_tree(&root, &sidecar.ids, sidecar.root_parent.as_deref())
}

/// Bring a sidecar back into alignment after readable FNX source was edited.
///
/// When the entries carry structural fingerprints (tag + `name` +
/// `parent_index`), old entries are aligned to the edited source's pre-order
/// elements in three passes, so a mid-tree insert, delete, replacement,
/// rename, reorder, or reparent no longer rebinds every following element to
/// its predecessor's identity:
///
/// 1. an order-preserving match on the full fingerprint (tag + name + the
///    parent's fingerprint) — a longest common subsequence when the quadratic
///    table is affordable, otherwise a patience-diff anchor alignment on
///    fingerprints unique to both sides, so large designs never degrade to
///    the pre-fingerprint positional pairing;
/// 2. leftover old entries and new elements whose exact tag + name fingerprint
///    is unique on both sides pair up regardless of order, so a pure z-reorder
///    of distinctive elements keeps their ids;
/// 3. a tag-only order-preserving match over what remains, so a renamed or
///    reparented element in place keeps its id rather than retiring it.
///
/// Only then do unmatched new elements receive ids from `next_id` and
/// unmatched old entries retire. Entries without a fingerprint (legacy
/// sidecars) match any element, and a sidecar with no fingerprints at all
/// keeps the historical purely positional pairing. When the structure changed,
/// sibling indices are normalized to the source order so the decoded scene
/// cannot reorder newly inserted elements because of duplicate indices.
pub fn reconcile_sidecar(
    text: &str,
    sidecar: &FnxSidecar,
    mut next_id: impl FnMut() -> String,
) -> Result<FnxSidecar, FnxError> {
    let root = parse_doc(text)?;
    let root_index = sidecar
        .ids
        .first()
        .map(|entry| entry.index.clone())
        .filter(|index| !index.is_null())
        .unwrap_or_else(|| Value::from(1.0));
    let elements = flatten_elements(&root, root_index);

    // Pure attribute edit: same shape, every fingerprint still in place. The
    // root pair keeps its id even when its tag or name changed — the file's
    // root element IS the page/component this file stores (its id is
    // referenced externally) — but a root TAG change is a type change, which
    // must fall through to the rebuild below so the persisted fingerprint and
    // the sibling indices stay truthful. The shape check compares each
    // element's parent position against the persisted `parent_index`, so a
    // reparent that happens to preserve pre-order still falls through and gets
    // its indices normalized.
    let root_tag_unchanged = match (sidecar.ids.first(), elements.first()) {
        (Some(entry), Some(element)) => {
            entry.tag.is_none() || entry.tag.as_deref() == Some(element.element.tag.as_str())
        }
        _ => true,
    };
    if root_tag_unchanged
        && elements.len() == sidecar.ids.len()
        && sidecar
            .ids
            .iter()
            .zip(&elements)
            .skip(1)
            .all(|(entry, element)| {
                fingerprint_matches(entry, element.element)
                    && (entry.parent_index.is_none()
                        || entry.parent_index
                            == element.parent.and_then(|parent| u32::try_from(parent).ok()))
            })
    {
        return Ok(sidecar.clone());
    }

    let has_fingerprints = sidecar.ids.iter().any(|entry| entry.tag.is_some());
    let assignments = if has_fingerprints {
        align_entries(&sidecar.ids, &elements)
    } else {
        positional_assignments(sidecar.ids.len(), elements.len())
    };

    let mut ids = Vec::with_capacity(elements.len());
    for (element, assignment) in elements.iter().zip(&assignments) {
        let id = match assignment {
            Some(existing) => sidecar.ids[*existing].id.clone(),
            None => next_id(),
        };
        ids.push(IdEntry {
            id,
            index: element.index.clone(),
            tag: Some(element.element.tag.clone()),
            name: element_name(element.element).map(str::to_owned),
            parent_index: element.parent.and_then(|parent| u32::try_from(parent).ok()),
        });
    }
    Ok(FnxSidecar {
        root_parent: sidecar.root_parent.clone(),
        ids,
    })
}

struct FlatElement<'a> {
    element: &'a FnxElement,
    /// The normalized index this element receives if the sidecar is rebuilt:
    /// the root keeps the persisted root index, children take 1-based source
    /// order.
    index: Value,
    /// The pre-order position of this element's parent (`None` for the root).
    parent: Option<usize>,
}

fn flatten_elements(root: &FnxElement, root_index: Value) -> Vec<FlatElement<'_>> {
    let mut elements = Vec::new();
    push_element(root, root_index, None, &mut elements);
    elements
}

fn push_element<'a>(
    element: &'a FnxElement,
    index: Value,
    parent: Option<usize>,
    elements: &mut Vec<FlatElement<'a>>,
) {
    let position = elements.len();
    elements.push(FlatElement {
        element,
        index,
        parent,
    });
    for (child_position, child) in element.children.iter().enumerate() {
        push_element(
            child,
            Value::from(child_position as u64 + 1),
            Some(position),
            elements,
        );
    }
}

fn element_name(element: &FnxElement) -> Option<&str> {
    element.attrs.get("name").and_then(Value::as_str)
}

/// A fingerprint-less entry (legacy sidecar) matches any element, so partially
/// upgraded sidecars degrade to positional pairing instead of retiring ids.
fn fingerprint_matches(entry: &IdEntry, element: &FnxElement) -> bool {
    match &entry.tag {
        None => true,
        Some(tag) => *tag == element.tag && entry.name.as_deref() == element_name(element),
    }
}

/// The historical pairing: consume old entries in pre-order, mint for the rest.
fn positional_assignments(existing: usize, elements: usize) -> Vec<Option<usize>> {
    (0..elements)
        .map(|position| (position < existing).then_some(position))
        .collect()
}

/// The parent-context half of the primary fingerprint: the persisted
/// `parent_index` names an entry whose tag + name must match the element's
/// actual parent. The root pair is the same pinned node on both sides, so two
/// direct children of the root always share context, whatever the root's own
/// fingerprint currently says. Entries without a `parent_index` (or whose
/// parent entry predates fingerprints) match any shape, keeping legacy
/// sidecars on their historical behavior.
fn parent_context_matches(
    entries: &[IdEntry],
    elements: &[FlatElement<'_>],
    old: usize,
    new: usize,
) -> bool {
    let Some(parent_index) = entries[old].parent_index else {
        return true;
    };
    let parent_index = parent_index as usize;
    let Some(old_parent) = entries.get(parent_index) else {
        return true;
    };
    let Some(new_parent_position) = elements[new].parent else {
        return true;
    };
    if parent_index == 0 && new_parent_position == 0 {
        return true;
    }
    let Some(old_parent_tag) = old_parent.tag.as_deref() else {
        return true;
    };
    let new_parent = elements[new_parent_position].element;
    old_parent_tag == new_parent.tag && old_parent.name.as_deref() == element_name(new_parent)
}

/// A tag + name fingerprint used as an anchor/uniqueness key.
type Fingerprint = (String, Option<String>);

/// Above this many LCS table cells the quadratic match is replaced by the
/// patience-diff anchor alignment.
const LCS_CELL_CAP: usize = 4_000_000;

/// Recursion guard for the anchor alignment's divide-and-conquer; a segment
/// that still exceeds [`LCS_CELL_CAP`] this deep falls back to the
/// fingerprint-gated positional zip.
const ANCHOR_DEPTH_CAP: u32 = 16;

/// Align the persisted entries to the edited source's pre-order elements. The
/// root pair is pinned unconditionally — it pins only the ID; the rebuilt
/// entry re-fingerprints from the source (see [`reconcile_sidecar`]). Returns
/// one `Option<entry index>` per element; `None` means the element has no
/// surviving identity and a fresh id must be minted.
fn align_entries(entries: &[IdEntry], elements: &[FlatElement<'_>]) -> Vec<Option<usize>> {
    let mut assignments = vec![None; elements.len()];
    if entries.is_empty() || elements.is_empty() {
        return assignments;
    }
    assignments[0] = Some(0);

    // Pass 1: full structural fingerprint (tag + name + parent context),
    // order-preserving.
    let primary_matches = |old: usize, new: usize| {
        fingerprint_matches(&entries[old], elements[new].element)
            && parent_context_matches(entries, elements, old, new)
    };
    let primary_old_key = |old: usize| {
        entries[old]
            .tag
            .clone()
            .map(|tag| (tag, entries[old].name.clone()))
    };
    let primary_new_key = |new: usize| {
        (
            elements[new].element.tag.clone(),
            element_name(elements[new].element).map(str::to_owned),
        )
    };
    let old_all: Vec<usize> = (1..entries.len()).collect();
    let new_all: Vec<usize> = (1..elements.len()).collect();
    align_ranges(
        &old_all,
        &new_all,
        &primary_matches,
        &primary_old_key,
        &primary_new_key,
        0,
        &mut assignments,
    );

    // Pass 2: leftovers whose exact tag + name fingerprint is unique on both
    // sides pair up regardless of order, so a pure z-reorder (which reverses
    // relative order and defeats any subsequence match) keeps ids.
    pair_unique_leftovers(entries, elements, &mut assignments);

    // Pass 3: tag-only, order-preserving — a renamed or reparented element
    // keeps its id instead of retiring it and minting a successor.
    let secondary_matches = |old: usize, new: usize| match entries[old].tag.as_deref() {
        None => true,
        Some(tag) => tag == elements[new].element.tag,
    };
    let secondary_old_key = |old: usize| entries[old].tag.clone().map(|tag| (tag, None));
    let secondary_new_key = |new: usize| (elements[new].element.tag.clone(), None);
    let (old_rest, new_rest) = residuals(entries.len(), &assignments);
    align_ranges(
        &old_rest,
        &new_rest,
        &secondary_matches,
        &secondary_old_key,
        &secondary_new_key,
        0,
        &mut assignments,
    );

    assignments
}

/// The entry positions and element positions still unmatched after the passes
/// run so far, both in pre-order (the root pair is never residual).
fn residuals(entry_count: usize, assignments: &[Option<usize>]) -> (Vec<usize>, Vec<usize>) {
    let mut matched = vec![false; entry_count];
    for &assignment in assignments.iter().flatten() {
        if let Some(slot) = matched.get_mut(assignment) {
            *slot = true;
        }
    }
    let old = (1..entry_count).filter(|&old| !matched[old]).collect();
    let new = (1..assignments.len())
        .filter(|&new| assignments[new].is_none())
        .collect();
    (old, new)
}

/// Pair still-unmatched entries and elements whose exact tag + name
/// fingerprint occurs exactly once among the unmatched on BOTH sides,
/// regardless of order.
fn pair_unique_leftovers(
    entries: &[IdEntry],
    elements: &[FlatElement<'_>],
    assignments: &mut [Option<usize>],
) {
    let (old_rest, new_rest) = residuals(entries.len(), assignments);
    let mut old_unique: HashMap<Fingerprint, (usize, usize)> = HashMap::new();
    for &old in &old_rest {
        if let Some(tag) = entries[old].tag.clone() {
            let slot = old_unique
                .entry((tag, entries[old].name.clone()))
                .or_insert((0, old));
            slot.0 += 1;
            slot.1 = old;
        }
    }
    let mut new_unique: HashMap<Fingerprint, (usize, usize)> = HashMap::new();
    for &new in &new_rest {
        let key = (
            elements[new].element.tag.clone(),
            element_name(elements[new].element).map(str::to_owned),
        );
        let slot = new_unique.entry(key).or_insert((0, new));
        slot.0 += 1;
        slot.1 = new;
    }
    for (key, &(new_count, new)) in &new_unique {
        if new_count != 1 {
            continue;
        }
        if let Some(&(1, old)) = old_unique.get(key) {
            assignments[new] = Some(old);
        }
    }
}

/// Order-preserving alignment of `old` entry positions against `new` element
/// positions: an exact longest common subsequence under `matches` when the
/// quadratic table is affordable, otherwise a patience-diff anchor alignment —
/// fingerprints unique to both sides (per the key functions) anchor via their
/// longest increasing subsequence and the segments between anchors recurse.
/// With no anchors at all (or too deep), the segment degrades to a positional
/// zip gated on `matches`, which for interchangeable fingerprints is exactly
/// the historical positional pairing.
fn align_ranges(
    old: &[usize],
    new: &[usize],
    matches: &impl Fn(usize, usize) -> bool,
    old_key: &impl Fn(usize) -> Option<Fingerprint>,
    new_key: &impl Fn(usize) -> Fingerprint,
    depth: u32,
    assignments: &mut [Option<usize>],
) {
    let rows = old.len();
    let columns = new.len();
    if rows == 0 || columns == 0 {
        return;
    }
    if rows
        .checked_mul(columns)
        .is_some_and(|cells| cells <= LCS_CELL_CAP)
    {
        lcs_align(old, new, matches, assignments);
        return;
    }
    if depth >= ANCHOR_DEPTH_CAP {
        zip_align(old, new, matches, assignments);
        return;
    }

    let mut old_counts: HashMap<Fingerprint, (usize, usize)> = HashMap::new();
    for (position, &entry) in old.iter().enumerate() {
        if let Some(key) = old_key(entry) {
            let slot = old_counts.entry(key).or_insert((0, position));
            slot.0 += 1;
            slot.1 = position;
        }
    }
    let mut new_counts: HashMap<Fingerprint, (usize, usize)> = HashMap::new();
    for (position, &element) in new.iter().enumerate() {
        let slot = new_counts.entry(new_key(element)).or_insert((0, position));
        slot.0 += 1;
        slot.1 = position;
    }
    let mut candidates: Vec<(usize, usize)> = old_counts
        .iter()
        .filter_map(|(key, &(old_count, old_position))| {
            if old_count != 1 {
                return None;
            }
            let &(new_count, new_position) = new_counts.get(key)?;
            (new_count == 1).then_some((old_position, new_position))
        })
        .collect();
    // Each candidate owns its positions on both sides, so sorting on the old
    // position makes the set (and everything downstream) deterministic despite
    // the HashMap iteration above.
    candidates.sort_unstable();
    let chain = longest_increasing_chain(&candidates);
    if chain.is_empty() {
        zip_align(old, new, matches, assignments);
        return;
    }

    let mut old_start = 0;
    let mut new_start = 0;
    for &(anchor_old, anchor_new) in &chain {
        assignments[new[anchor_new]] = Some(old[anchor_old]);
        align_ranges(
            &old[old_start..anchor_old],
            &new[new_start..anchor_new],
            matches,
            old_key,
            new_key,
            depth + 1,
            assignments,
        );
        old_start = anchor_old + 1;
        new_start = anchor_new + 1;
    }
    align_ranges(
        &old[old_start..],
        &new[new_start..],
        matches,
        old_key,
        new_key,
        depth + 1,
        assignments,
    );
}

/// The longest chain of candidate pairs strictly increasing on both sides.
/// `candidates` must be sorted by (distinct) old position; O(n log n) patience
/// piles on the new position.
fn longest_increasing_chain(candidates: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut pile_tops: Vec<usize> = Vec::new();
    let mut predecessors: Vec<Option<usize>> = Vec::with_capacity(candidates.len());
    for (position, &(_, new_position)) in candidates.iter().enumerate() {
        let pile = pile_tops.partition_point(|&top| candidates[top].1 < new_position);
        predecessors.push((pile > 0).then(|| pile_tops[pile - 1]));
        if pile == pile_tops.len() {
            pile_tops.push(position);
        } else {
            pile_tops[pile] = position;
        }
    }
    let mut chain = Vec::new();
    let mut current = pile_tops.last().copied();
    while let Some(position) = current {
        chain.push(candidates[position]);
        current = predecessors[position];
    }
    chain.reverse();
    chain
}

/// Exact longest common subsequence of `old` against `new` under `matches`.
/// `table[i * width + j]` = length of the longest pairing of `old[i..]`
/// against `new[j..]`.
fn lcs_align(
    old: &[usize],
    new: &[usize],
    matches: &impl Fn(usize, usize) -> bool,
    assignments: &mut [Option<usize>],
) {
    let rows = old.len();
    let columns = new.len();
    let width = columns + 1;
    let mut table = vec![0u32; (rows + 1) * width];
    for i in (0..rows).rev() {
        for j in (0..columns).rev() {
            let paired = if matches(old[i], new[j]) {
                table[(i + 1) * width + j + 1] + 1
            } else {
                0
            };
            table[i * width + j] = paired
                .max(table[(i + 1) * width + j])
                .max(table[i * width + j + 1]);
        }
    }
    let mut i = 0;
    let mut j = 0;
    while i < rows && j < columns {
        if matches(old[i], new[j]) && table[i * width + j] == table[(i + 1) * width + j + 1] + 1 {
            assignments[new[j]] = Some(old[i]);
            i += 1;
            j += 1;
        } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
}

/// Positional pairing gated on `matches`: the k-th unmatched old entry meets
/// the k-th unmatched new element, keeping its id only when the fingerprints
/// agree — a mismatched pair mints/retires rather than crossing identities.
fn zip_align(
    old: &[usize],
    new: &[usize],
    matches: &impl Fn(usize, usize) -> bool,
    assignments: &mut [Option<usize>],
) {
    for (&old_position, &new_position) in old.iter().zip(new) {
        if matches(old_position, new_position) {
            assignments[new_position] = Some(old_position);
        }
    }
}
