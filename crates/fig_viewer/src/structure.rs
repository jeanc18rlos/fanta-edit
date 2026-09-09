//! Structural canvas edits — group, frame selection, ungroup, and image
//! placement — built as operation lists. Every builder is pure over the
//! document; the caller wraps the result in one transaction with
//! [`crate::clipboard::apply_transaction`] so each edit is a single undo step.

use std::collections::HashSet;

use anyhow::{Context as _, Result, bail};
use fanta_doc::{
    AssetId, BitmapNode, Bounds, CanvasNode, Doc, GroupNode, ImageFitMode, IndexKey, NodeData,
    NodeId, Operation, Transform2D,
};

#[derive(Debug)]
pub(crate) struct Grouped {
    pub group: NodeId,
    pub operations: Vec<Operation>,
}

#[derive(Debug)]
pub(crate) struct Ungrouped {
    pub children: Vec<NodeId>,
    pub operations: Vec<Operation>,
}

/// Wrap the top-level members of `ids` (any id with a selected ancestor is
/// dropped) in a new plain group. The group is parented where the topmost
/// member lives and takes that member's slot in the z-order; members from
/// other parents are reparented into it with their world transforms kept.
pub(crate) fn group_operations(doc: &Doc, ids: &[NodeId], name: Option<&str>) -> Result<Grouped> {
    wrap_operations(doc, ids, name.unwrap_or("Group"), false)
}

/// Same as [`group_operations`], but the wrapper is a frame: a group that
/// clips to the union of its members' bounds and carries no background.
pub(crate) fn frame_selection_operations(
    doc: &Doc,
    ids: &[NodeId],
    name: Option<&str>,
) -> Result<Grouped> {
    wrap_operations(doc, ids, name.unwrap_or("Frame"), true)
}

fn wrap_operations(doc: &Doc, ids: &[NodeId], name: &str, clip: bool) -> Result<Grouped> {
    let members = top_level_members(doc, ids)?;
    let mut ordered = members
        .iter()
        .map(|id| (paint_path(doc, *id), *id))
        .collect::<Vec<_>>();
    ordered.sort();
    let members = ordered.into_iter().map(|(_, id)| id).collect::<Vec<_>>();
    let topmost = *members.last().context("nothing to group")?;
    let topmost_node = doc
        .scene
        .get(topmost)
        .context("the topmost layer is gone")?;
    let parent = topmost_node.parent;

    let mut union: Option<Bounds> = None;
    for member in &members {
        if let Some(bounds) = doc.scene.world_bounds(*member)
            && bounds.is_finite()
        {
            union = Some(match union {
                Some(current) => current.union(&bounds),
                None => bounds,
            });
        }
    }
    let union = union.context("the selected layers have no bounds to group around")?;

    let parent_world = parent_world_transform(doc, parent)?;
    let group_world = Transform2D::translation(union.min_x, union.min_y);
    let group_local = group_world.then(&parent_world.inverse());
    if !group_local.is_finite() {
        bail!("the parent's transform cannot be inverted");
    }

    // The slot is computed against the siblings that stay behind, so the new
    // key may equal a member's current key; the scene orders equal keys by
    // id and the member leaves this parent in the same transaction, so the
    // group ends up exactly where the topmost member was.
    let member_set = members.iter().copied().collect::<HashSet<_>>();
    let siblings = doc.scene.children_of(parent);
    let topmost_position = siblings
        .iter()
        .position(|id| *id == topmost)
        .context("the topmost layer is not among its parent's children")?;
    let below = siblings[..topmost_position]
        .iter()
        .rev()
        .find(|id| !member_set.contains(id))
        .and_then(|id| doc.scene.get(*id))
        .map(|node| node.index);
    let above = siblings[topmost_position + 1..]
        .iter()
        .find(|id| !member_set.contains(id))
        .and_then(|id| doc.scene.get(*id))
        .map(|node| node.index);

    let size = [union.width(), union.height()];
    let mut group = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: clip.then_some(size),
        ..GroupNode::default()
    }));
    group.name = name.to_owned();
    group.parent = parent;
    group.transform = group_local;
    let group_id = group.id;

    let mut operations = Vec::new();
    match slot_between(below, above) {
        Some(index) => group.index = index,
        None => {
            let mut order = Vec::with_capacity(siblings.len() + 1);
            for sibling in siblings {
                if *sibling == topmost {
                    order.push(Slot::Incoming);
                }
                if !member_set.contains(sibling) {
                    order.push(Slot::Existing(*sibling));
                }
            }
            let renumbered = renumber(doc, &order)?;
            group.index = renumbered
                .incoming
                .into_iter()
                .next()
                .context("the group has no slot among its siblings")?;
            operations.extend(renumbered.operations);
        }
    }
    operations.push(Operation::create_node(group));
    let group_world_inverse = group_world.inverse();
    for (position, member) in members.iter().enumerate() {
        let node = doc.scene.get(*member).context("a selected layer is gone")?;
        let world = doc
            .scene
            .world_transform(*member)
            .context("a selected layer has no world transform")?;
        let new_local = world.then(&group_world_inverse);
        if !new_local.is_finite() {
            bail!("the layer's transform cannot be expressed inside the group");
        }
        operations.push(Operation::Reparent {
            id: *member,
            old_parent: node.parent,
            old_index: node.index,
            new_parent: Some(group_id),
            new_index: IndexKey::from_raw(position as f64 + 1.0),
        });
        if new_local != node.transform {
            operations.push(Operation::SetTransform {
                id: *member,
                old: node.transform,
                new: new_local,
            });
        }
    }
    Ok(Grouped {
        group: group_id,
        operations,
    })
}

/// Dissolve every group in `groups` as ONE transaction's worth of operations.
///
/// Each group is planned against a scratch document that already carries the
/// earlier groups' edits, because [`ungroup_operations`] is not isolated to the
/// group it dissolves: when the wrapper's key gap is too narrow it renumbers
/// the whole parent, rewriting every staying sibling's key. Two sibling groups
/// planned from the same pre-edit document would then disagree about that
/// parent's z-order — colliding keys that reshuffle a bystander layer, or a
/// `SetIndex` naming a wrapper the first plan already deleted, which fails and
/// aborts the whole transaction. Ungroup is not a hot path, so one clone per
/// invocation is fine.
pub(crate) fn ungroup_many_operations(
    doc: &Doc,
    groups: &[NodeId],
) -> Result<(Vec<Operation>, Vec<NodeId>)> {
    let mut operations = Vec::new();
    let mut children = Vec::new();
    let mut scratch = doc.clone();
    for group in groups {
        let ungrouped = ungroup_operations(&scratch, *group)?;
        for operation in &ungrouped.operations {
            scratch.apply(operation.clone())?;
        }
        operations.extend(ungrouped.operations);
        children.extend(ungrouped.children);
    }
    Ok((operations, children))
}

/// Dissolve one group or frame: its children move to the group's parent in
/// the group's slot with their world transforms kept, then the empty wrapper
/// is deleted.
pub(crate) fn ungroup_operations(doc: &Doc, group: NodeId) -> Result<Ungrouped> {
    let node = doc.scene.get(group).context("the layer is gone")?;
    if doc.pages().contains(&group) {
        bail!("a page cannot be ungrouped");
    }
    if doc.is_component_root(group) {
        bail!("a component master cannot be ungrouped; detach or delete the component instead");
    }
    match &node.data {
        NodeData::Group(_) => {}
        NodeData::Instance(_) => bail!("an instance cannot be ungrouped; detach it first"),
        _ => bail!("\"{}\" is not a group or frame", node.name),
    }

    let parent = node.parent;
    let parent_world = parent_world_transform(doc, parent)?;
    let parent_world_inverse = parent_world.inverse();
    let children = doc.scene.children_of(Some(group)).to_vec();

    // Children land strictly between the group and its next sibling so their
    // keys cannot collide with the group's own while it still exists; once
    // the wrapper is deleted they occupy exactly its slot. When that gap is
    // too narrow to hold them, the parent is renumbered with the wrapper's
    // slot widened to one key per child instead.
    let siblings = doc.scene.children_of(parent);
    let above = siblings
        .iter()
        .skip_while(|id| **id != group)
        .nth(1)
        .and_then(|id| doc.scene.get(*id))
        .map(|node| node.index);
    let mut operations = Vec::new();
    let keys = match keys_between(node.index, above, children.len()) {
        Some(keys) => keys,
        None => {
            let mut order = Vec::with_capacity(siblings.len() + children.len());
            for sibling in siblings {
                if *sibling == group {
                    order.extend(std::iter::repeat_n(Slot::Incoming, children.len()));
                } else {
                    order.push(Slot::Existing(*sibling));
                }
            }
            let renumbered = renumber(doc, &order)?;
            operations.extend(renumbered.operations);
            renumbered.incoming
        }
    };

    for (child, new_index) in children.iter().zip(keys) {
        let child_node = doc.scene.get(*child).context("a child layer is gone")?;
        let world = doc
            .scene
            .world_transform(*child)
            .context("a child layer has no world transform")?;
        let new_local = world.then(&parent_world_inverse);
        if !new_local.is_finite() {
            bail!("a child's transform cannot be expressed outside the group");
        }
        operations.push(Operation::Reparent {
            id: *child,
            old_parent: Some(group),
            old_index: child_node.index,
            new_parent: parent,
            new_index,
        });
        if new_local != child_node.transform {
            operations.push(Operation::SetTransform {
                id: *child,
                old: child_node.transform,
                new: new_local,
            });
        }
    }
    operations.push(Operation::DeleteSubtree {
        snapshot: vec![node.clone()],
    });
    Ok(Ungrouped {
        children,
        operations,
    })
}

/// A bitmap layer for an already-ingested image asset, sized to `size` with
/// its top-left corner at world `(x, y)` inside `parent` (the active page
/// when `None`).
pub(crate) fn image_layer_node(
    doc: &Doc,
    asset: AssetId,
    natural_size: [u32; 2],
    size: [f64; 2],
    parent: Option<NodeId>,
    x: f64,
    y: f64,
    name: Option<&str>,
) -> Result<CanvasNode> {
    if !(size[0].is_finite() && size[0] > 0.0 && size[1].is_finite() && size[1] > 0.0) {
        bail!("the image size must be positive");
    }
    if !(x.is_finite() && y.is_finite()) {
        bail!("the image position must be finite");
    }
    let parent = match parent {
        Some(parent) => {
            let parent_node = doc.scene.get(parent).context("the parent layer is gone")?;
            if !parent_node.can_have_children() {
                bail!("\"{}\" cannot contain layers", parent_node.name);
            }
            Some(parent)
        }
        None => Some(
            doc.active_page()
                .context("the document has no active page to place the image on")?,
        ),
    };
    let parent_world = parent_world_transform(doc, parent)?;
    let mut node = CanvasNode::new(NodeData::Bitmap(BitmapNode {
        asset,
        natural_size,
        local_size: size,
        crop: None,
        fit: ImageFitMode::Fill,
        tint: None,
    }));
    if let Some(name) = name {
        node.name = name.to_owned();
    }
    node.parent = parent;
    node.index = doc.scene.next_child_index(parent);
    node.transform = Transform2D::translation(x, y).then(&parent_world.inverse());
    if !node.transform.is_finite() {
        bail!("the parent's transform cannot be inverted");
    }
    Ok(node)
}

/// The members of `ids` that survive grouping: present in the scene, not a
/// page or component master, and without another member above them.
fn top_level_members(doc: &Doc, ids: &[NodeId]) -> Result<Vec<NodeId>> {
    if ids.is_empty() {
        bail!("select at least one layer");
    }
    let requested = ids.iter().copied().collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    let mut members = Vec::new();
    for id in ids {
        if !seen.insert(*id) {
            continue;
        }
        let node = doc.scene.get(*id).context("a selected layer is gone")?;
        if doc.pages().contains(id) {
            bail!("a page cannot be grouped");
        }
        if doc.is_component_root(*id) {
            bail!(
                "\"{}\" is a component master and cannot be grouped",
                node.name
            );
        }
        if doc
            .scene
            .ancestors_of(*id)
            .any(|ancestor| requested.contains(&ancestor.id))
        {
            continue;
        }
        members.push(*id);
    }
    if members.is_empty() {
        bail!("select at least one layer");
    }
    Ok(members)
}

/// Position of `id` in paint order as the chain of sibling positions from the
/// root down; lexicographic order on these paths is the scene's paint order
/// for nodes that are not each other's ancestors.
fn paint_path(doc: &Doc, id: NodeId) -> Vec<usize> {
    let mut chain = vec![id];
    chain.extend(doc.scene.ancestors_of(id).map(|ancestor| ancestor.id));
    chain.reverse();
    let mut parent = None;
    let mut path = Vec::with_capacity(chain.len());
    for node in chain {
        let position = doc
            .scene
            .children_of(parent)
            .iter()
            .position(|sibling| *sibling == node)
            .unwrap_or(0);
        path.push(position);
        parent = Some(node);
    }
    path
}

fn parent_world_transform(doc: &Doc, parent: Option<NodeId>) -> Result<Transform2D> {
    let world = match parent {
        Some(parent) => doc
            .scene
            .world_transform(parent)
            .context("the parent layer has no world transform")?,
        None => Transform2D::IDENTITY,
    };
    let [a, b, c, d, _, _] = world.to_components();
    let determinant = a * d - b * c;
    if !world.is_finite() || !determinant.is_finite() || determinant.abs() <= f64::EPSILON {
        bail!("the parent's transform cannot be inverted");
    }
    Ok(world)
}

/// A key between two neighbouring siblings' keys; `None` when the gap is too
/// narrow for f64 to hold another key (or the keys are equal), in which case
/// the caller renumbers the parent with [`renumber`].
fn slot_between(below: Option<IndexKey>, above: Option<IndexKey>) -> Option<IndexKey> {
    match (below, above) {
        (Some(below), Some(above)) => (below < above
            && !IndexKey::near_precision_limit(below, above))
        .then(|| IndexKey::between(below, above)),
        (Some(below), None) => Some(IndexKey::after(below)),
        (None, Some(above)) => Some(IndexKey::before(above)),
        (None, None) => Some(IndexKey::FIRST),
    }
}

/// `count` ascending keys strictly greater than `lower` and, when present,
/// strictly less than `upper`; `None` when the gap is too narrow for f64 to
/// hold that many distinct keys.
fn keys_between(lower: IndexKey, upper: Option<IndexKey>, count: usize) -> Option<Vec<IndexKey>> {
    let step = match upper {
        Some(upper) => {
            if upper <= lower || IndexKey::near_precision_limit(lower, upper) {
                return None;
            }
            (upper.raw() - lower.raw()) / (count as f64 + 1.0)
        }
        None => 1.0,
    };
    let keys: Vec<IndexKey> = (1..=count)
        .map(|position| IndexKey::from_raw(lower.raw() + step * position as f64))
        .collect();
    let mut previous = lower;
    for key in &keys {
        if *key <= previous || upper.is_some_and(|upper| *key >= upper) {
            return None;
        }
        previous = *key;
    }
    Some(keys)
}

/// One entry of a parent's z-order after a structural edit: a sibling that
/// stays, or a node the edit brings in.
#[derive(Clone, Copy)]
enum Slot {
    Existing(NodeId),
    Incoming,
}

struct Renumbered {
    /// `SetIndex` for every staying sibling whose key changes.
    operations: Vec<Operation>,
    /// The key each [`Slot::Incoming`] takes, in order.
    incoming: Vec<IndexKey>,
}

/// Normalize a parent's z-order to `1.0, 2.0, …` along `order` — the fallback
/// when the gap a new key must land in is too narrow for f64, mirroring the
/// layer panel's drop handling so both paths leave the same keys behind.
fn renumber(doc: &Doc, order: &[Slot]) -> Result<Renumbered> {
    let mut operations = Vec::new();
    let mut incoming = Vec::new();
    for (position, slot) in order.iter().enumerate() {
        let key = IndexKey::from_raw(position as f64 + 1.0);
        match slot {
            Slot::Incoming => incoming.push(key),
            Slot::Existing(id) => {
                let node = doc.scene.get(*id).context("a sibling layer is gone")?;
                if node.index != key {
                    operations.push(Operation::SetIndex {
                        id: *id,
                        old: node.index,
                        new: key,
                    });
                }
            }
        }
    }
    Ok(Renumbered {
        operations,
        incoming,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::apply_transaction;
    use fanta_doc::{Color, ComponentDef, ComponentId, InstanceNode, VectorNode};

    fn page_doc() -> (Doc, NodeId) {
        let mut doc = Doc::new();
        let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let page_id = page.id;
        doc.apply(Operation::create_node(page)).unwrap();
        doc.add_page(page_id);
        doc.set_active_page(Some(page_id));
        doc.history = Default::default();
        (doc, page_id)
    }

    fn insert(doc: &mut Doc, mut node: CanvasNode) -> NodeId {
        node.index = doc.scene.next_child_index(node.parent);
        let id = node.id;
        doc.apply(Operation::create_node(node)).unwrap();
        doc.history = Default::default();
        id
    }

    fn insert_with_index(doc: &mut Doc, mut node: CanvasNode, index: IndexKey) -> NodeId {
        node.index = index;
        let id = node.id;
        doc.apply(Operation::create_node(node)).unwrap();
        doc.history = Default::default();
        id
    }

    fn assert_strictly_increasing_keys(doc: &Doc, parent: Option<NodeId>) {
        let keys: Vec<IndexKey> = doc
            .scene
            .children_of(parent)
            .iter()
            .map(|id| doc.scene.get(*id).unwrap().index)
            .collect();
        for pair in keys.windows(2) {
            assert!(
                pair[0] < pair[1],
                "z-order keys {:?} and {:?} are not strictly increasing",
                pair[0],
                pair[1]
            );
        }
    }

    fn rect(parent: NodeId, x: f64, y: f64, width: f64, height: f64) -> CanvasNode {
        let mut node = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            width,
            height,
            Color::BLACK,
        )));
        node.parent = Some(parent);
        node.transform = Transform2D::translation(x, y);
        node
    }

    fn frame(parent: NodeId, transform: Transform2D, size: [f64; 2]) -> CanvasNode {
        let mut node = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some(size),
            ..GroupNode::default()
        }));
        node.parent = Some(parent);
        node.transform = transform;
        node
    }

    fn assert_transform_close(actual: Transform2D, expected: Transform2D) {
        for (actual, expected) in actual
            .to_components()
            .into_iter()
            .zip(expected.to_components())
        {
            assert!(
                (actual - expected).abs() < 1e-9,
                "transform component {actual} differs from {expected}"
            );
        }
    }

    fn world_bounds(doc: &Doc, id: NodeId) -> Bounds {
        doc.scene.world_bounds(id).expect("node has bounds")
    }

    fn assert_bounds_close(actual: Bounds, expected: Bounds) {
        for (actual, expected) in [
            (actual.min_x, expected.min_x),
            (actual.min_y, expected.min_y),
            (actual.max_x, expected.max_x),
            (actual.max_y, expected.max_y),
        ] {
            assert!(
                (actual - expected).abs() < 1e-9,
                "bounds edge {actual} differs from {expected}"
            );
        }
    }

    #[test]
    fn group_preserves_world_positions_and_z_order_in_one_undo_step() {
        let (mut doc, page) = page_doc();
        let bottom = insert(&mut doc, rect(page, 10.0, 20.0, 30.0, 30.0));
        let middle = insert(&mut doc, rect(page, 100.0, 50.0, 20.0, 20.0));
        let top = insert(&mut doc, rect(page, 200.0, 200.0, 10.0, 10.0));
        let bottom_before = world_bounds(&doc, bottom);
        let top_before = world_bounds(&doc, top);

        let grouped = group_operations(&doc, &[top, bottom], None).unwrap();
        assert!(apply_transaction(&mut doc, "Group", grouped.operations).unwrap());

        let group = doc.scene.get(grouped.group).unwrap();
        assert_eq!(group.name, "Group");
        assert_eq!(group.parent, Some(page));
        assert!(matches!(&group.data, NodeData::Group(inner) if inner.clip_size.is_none()));
        assert_transform_close(group.transform, Transform2D::translation(10.0, 20.0));
        assert_eq!(doc.scene.children_of(Some(grouped.group)), &[bottom, top]);
        assert_eq!(doc.scene.children_of(Some(page)), &[middle, grouped.group]);
        assert_bounds_close(world_bounds(&doc, bottom), bottom_before);
        assert_bounds_close(world_bounds(&doc, top), top_before);
        let union = world_bounds(&doc, grouped.group);
        assert_eq!((union.min_x, union.min_y), (10.0, 20.0));
        assert_eq!((union.max_x, union.max_y), (210.0, 210.0));

        assert_eq!(doc.history.undo_depth(), 1);
        assert!(doc.undo().unwrap());
        assert!(!doc.scene.contains(grouped.group));
        assert_eq!(doc.scene.children_of(Some(page)), &[bottom, middle, top]);
        assert_bounds_close(world_bounds(&doc, bottom), bottom_before);
    }

    #[test]
    fn group_takes_the_topmost_members_slot_and_drops_nested_ids() {
        let (mut doc, page) = page_doc();
        let below = insert(&mut doc, rect(page, 0.0, 0.0, 10.0, 10.0));
        let frame_id = insert(
            &mut doc,
            frame(page, Transform2D::translation(50.0, 50.0), [100.0, 100.0]),
        );
        let inner = insert(&mut doc, rect(frame_id, 5.0, 5.0, 10.0, 10.0));
        let above = insert(&mut doc, rect(page, 300.0, 0.0, 10.0, 10.0));

        let grouped = group_operations(&doc, &[inner, frame_id], Some("Card")).unwrap();
        assert!(apply_transaction(&mut doc, "Group", grouped.operations).unwrap());

        assert_eq!(doc.scene.get(grouped.group).unwrap().name, "Card");
        assert_eq!(
            doc.scene.children_of(Some(page)),
            &[below, grouped.group, above]
        );
        assert_eq!(doc.scene.children_of(Some(grouped.group)), &[frame_id]);
        assert_eq!(doc.scene.get(inner).unwrap().parent, Some(frame_id));
    }

    #[test]
    fn grouping_across_parents_lands_beside_the_topmost_member() {
        let (mut doc, page) = page_doc();
        let frame_id = insert(
            &mut doc,
            frame(
                page,
                Transform2D::scale(2.0).then(&Transform2D::translation(100.0, 100.0)),
                [200.0, 200.0],
            ),
        );
        let nested = insert(&mut doc, rect(frame_id, 10.0, 10.0, 20.0, 20.0));
        let loose = insert(&mut doc, rect(page, 0.0, 0.0, 40.0, 40.0));
        let nested_before = world_bounds(&doc, nested);
        let loose_before = world_bounds(&doc, loose);

        let grouped = group_operations(&doc, &[nested, loose], None).unwrap();
        assert!(apply_transaction(&mut doc, "Group", grouped.operations).unwrap());

        let group = doc.scene.get(grouped.group).unwrap();
        assert_eq!(group.parent, Some(page));
        assert_eq!(
            doc.scene.children_of(Some(page)),
            &[frame_id, grouped.group]
        );
        assert_eq!(doc.scene.children_of(Some(grouped.group)), &[nested, loose]);
        assert_bounds_close(world_bounds(&doc, nested), nested_before);
        assert_bounds_close(world_bounds(&doc, loose), loose_before);
        let union = world_bounds(&doc, grouped.group);
        assert_eq!((union.min_x, union.min_y), (0.0, 0.0));
        assert_eq!((union.max_x, union.max_y), (160.0, 160.0));
    }

    #[test]
    fn grouping_inside_a_scaled_frame_keeps_members_in_place() {
        let (mut doc, page) = page_doc();
        let frame_id = insert(
            &mut doc,
            frame(
                page,
                Transform2D::scale(0.5).then(&Transform2D::translation(-20.0, 40.0)),
                [400.0, 400.0],
            ),
        );
        let first = insert(&mut doc, rect(frame_id, 10.0, 10.0, 20.0, 20.0));
        let second = insert(&mut doc, rect(frame_id, 100.0, 60.0, 20.0, 20.0));
        let first_before = world_bounds(&doc, first);
        let second_before = world_bounds(&doc, second);

        let grouped = group_operations(&doc, &[first, second], None).unwrap();
        assert!(apply_transaction(&mut doc, "Group", grouped.operations).unwrap());

        assert_eq!(doc.scene.get(grouped.group).unwrap().parent, Some(frame_id));
        assert_bounds_close(world_bounds(&doc, first), first_before);
        assert_bounds_close(world_bounds(&doc, second), second_before);
        let group_world = doc.scene.world_transform(grouped.group).unwrap();
        assert_transform_close(
            group_world,
            Transform2D::translation(first_before.min_x, first_before.min_y),
        );
    }

    #[test]
    fn frame_selection_clips_to_the_union_without_a_background() {
        let (mut doc, page) = page_doc();
        let first = insert(&mut doc, rect(page, 10.0, 10.0, 30.0, 10.0));
        let second = insert(&mut doc, rect(page, 20.0, 40.0, 10.0, 30.0));

        let grouped = frame_selection_operations(&doc, &[first, second], None).unwrap();
        assert!(apply_transaction(&mut doc, "Frame", grouped.operations).unwrap());

        let group = doc.scene.get(grouped.group).unwrap();
        assert_eq!(group.name, "Frame");
        let NodeData::Group(inner) = &group.data else {
            panic!("frame selection should create a group node");
        };
        assert_eq!(inner.clip_size, Some([30.0, 60.0]));
        assert!(inner.background.is_none());
        assert_transform_close(group.transform, Transform2D::translation(10.0, 10.0));
    }

    #[test]
    fn grouping_refuses_pages_masters_and_empty_input() {
        let (mut doc, page) = page_doc();
        let error = group_operations(&doc, &[], None).unwrap_err();
        assert!(error.to_string().contains("at least one layer"));

        let error = group_operations(&doc, &[page], None).unwrap_err();
        assert!(error.to_string().contains("page"));

        let master = insert(&mut doc, frame(page, Transform2D::IDENTITY, [100.0, 100.0]));
        let component = ComponentId::new();
        doc.components
            .defs
            .insert(component, ComponentDef::new(component, master, "Button"));
        let error = group_operations(&doc, &[master], None).unwrap_err();
        assert!(error.to_string().contains("component master"));

        let error = group_operations(&doc, &[NodeId::new()], None).unwrap_err();
        assert!(error.to_string().contains("gone"));
    }

    #[test]
    fn ungroup_restores_children_to_the_parent_and_deletes_the_wrapper() {
        let (mut doc, page) = page_doc();
        let below = insert(&mut doc, rect(page, 0.0, 0.0, 10.0, 10.0));
        let frame_id = insert(
            &mut doc,
            frame(
                page,
                Transform2D::scale(2.0).then(&Transform2D::translation(100.0, 50.0)),
                [100.0, 100.0],
            ),
        );
        let first = insert(&mut doc, rect(frame_id, 5.0, 5.0, 10.0, 10.0));
        let second = insert(&mut doc, rect(frame_id, 30.0, 30.0, 10.0, 10.0));
        let above = insert(&mut doc, rect(page, 300.0, 0.0, 10.0, 10.0));
        let first_before = doc.scene.world_transform(first).unwrap();
        let second_before = doc.scene.world_transform(second).unwrap();

        let ungrouped = ungroup_operations(&doc, frame_id).unwrap();
        assert_eq!(ungrouped.children, vec![first, second]);
        assert!(apply_transaction(&mut doc, "Ungroup", ungrouped.operations).unwrap());

        assert!(!doc.scene.contains(frame_id));
        assert_eq!(
            doc.scene.children_of(Some(page)),
            &[below, first, second, above]
        );
        assert_transform_close(doc.scene.world_transform(first).unwrap(), first_before);
        assert_transform_close(doc.scene.world_transform(second).unwrap(), second_before);

        assert_eq!(doc.history.undo_depth(), 1);
        assert!(doc.undo().unwrap());
        assert!(doc.scene.contains(frame_id));
        assert_eq!(doc.scene.children_of(Some(page)), &[below, frame_id, above]);
        assert_eq!(doc.scene.children_of(Some(frame_id)), &[first, second]);
        assert_transform_close(doc.scene.world_transform(first).unwrap(), first_before);
    }

    #[test]
    fn ungroup_of_the_topmost_group_keeps_children_on_top() {
        let (mut doc, page) = page_doc();
        let below = insert(&mut doc, rect(page, 0.0, 0.0, 10.0, 10.0));
        let group_id = insert(
            &mut doc,
            frame(page, Transform2D::translation(10.0, 10.0), [50.0, 50.0]),
        );
        let child = insert(&mut doc, rect(group_id, 1.0, 1.0, 5.0, 5.0));

        let ungrouped = ungroup_operations(&doc, group_id).unwrap();
        assert!(apply_transaction(&mut doc, "Ungroup", ungrouped.operations).unwrap());
        assert_eq!(doc.scene.children_of(Some(page)), &[below, child]);
        assert_transform_close(
            doc.scene.get(child).unwrap().transform,
            Transform2D::translation(11.0, 11.0),
        );
    }

    /// Repeated inserts into the same gap exhaust f64 precision; grouping
    /// there must renumber the parent rather than mint a colliding key (or
    /// trip `IndexKey::between`'s ordering assertion).
    #[test]
    fn grouping_into_an_exhausted_gap_renumbers_the_siblings() {
        let (mut doc, page) = page_doc();
        let below = insert_with_index(
            &mut doc,
            rect(page, 0.0, 0.0, 10.0, 10.0),
            IndexKey::from_raw(1.0),
        );
        let member = insert_with_index(
            &mut doc,
            rect(page, 20.0, 0.0, 10.0, 10.0),
            IndexKey::from_raw(1.0 + f64::EPSILON),
        );
        let above_key = IndexKey::from_raw(1.0 + 2.0 * f64::EPSILON);
        let above = insert_with_index(&mut doc, rect(page, 40.0, 0.0, 10.0, 10.0), above_key);
        assert_eq!(doc.scene.children_of(Some(page)), &[below, member, above]);
        let member_before = world_bounds(&doc, member);

        let grouped = group_operations(&doc, &[member], None).unwrap();
        assert!(apply_transaction(&mut doc, "Group", grouped.operations).unwrap());

        assert_eq!(
            doc.scene.children_of(Some(page)),
            &[below, grouped.group, above]
        );
        assert_strictly_increasing_keys(&doc, Some(page));
        assert_eq!(doc.scene.children_of(Some(grouped.group)), &[member]);
        assert_bounds_close(world_bounds(&doc, member), member_before);

        assert_eq!(doc.history.undo_depth(), 1);
        assert!(doc.undo().unwrap());
        assert_eq!(doc.scene.children_of(Some(page)), &[below, member, above]);
        assert_eq!(doc.scene.get(above).unwrap().index, above_key);
    }

    #[test]
    fn grouping_between_equal_keys_renumbers_instead_of_colliding() {
        let (mut doc, page) = page_doc();
        for position in 0..3 {
            insert_with_index(
                &mut doc,
                rect(page, position as f64 * 20.0, 0.0, 10.0, 10.0),
                IndexKey::FIRST,
            );
        }
        let order_before = doc.scene.children_of(Some(page)).to_vec();
        let member = order_before[1];

        let grouped = group_operations(&doc, &[member], None).unwrap();
        assert!(apply_transaction(&mut doc, "Group", grouped.operations).unwrap());

        assert_eq!(
            doc.scene.children_of(Some(page)),
            &[order_before[0], grouped.group, order_before[2]]
        );
        assert_strictly_increasing_keys(&doc, Some(page));
    }

    #[test]
    fn ungrouping_into_an_exhausted_gap_renumbers_the_siblings() {
        let (mut doc, page) = page_doc();
        let below = insert_with_index(
            &mut doc,
            rect(page, 0.0, 0.0, 10.0, 10.0),
            IndexKey::from_raw(1.0),
        );
        let group_id = insert_with_index(
            &mut doc,
            frame(page, Transform2D::translation(10.0, 10.0), [50.0, 50.0]),
            IndexKey::from_raw(2.0),
        );
        let above_key = IndexKey::from_raw(2.0 + 2.0 * f64::EPSILON);
        let above = insert_with_index(&mut doc, rect(page, 300.0, 0.0, 10.0, 10.0), above_key);
        let children: Vec<NodeId> = (0..3)
            .map(|position| {
                insert(
                    &mut doc,
                    rect(group_id, position as f64 * 5.0, 0.0, 4.0, 4.0),
                )
            })
            .collect();
        let worlds_before: Vec<Transform2D> = children
            .iter()
            .map(|child| doc.scene.world_transform(*child).unwrap())
            .collect();

        let ungrouped = ungroup_operations(&doc, group_id).unwrap();
        assert!(apply_transaction(&mut doc, "Ungroup", ungrouped.operations).unwrap());

        let mut expected = vec![below];
        expected.extend(children.iter().copied());
        expected.push(above);
        assert_eq!(doc.scene.children_of(Some(page)), expected.as_slice());
        assert_strictly_increasing_keys(&doc, Some(page));
        for (child, before) in children.iter().zip(worlds_before) {
            assert_transform_close(doc.scene.world_transform(*child).unwrap(), before);
        }

        assert_eq!(doc.history.undo_depth(), 1);
        assert!(doc.undo().unwrap());
        assert_eq!(doc.scene.children_of(Some(page)), &[below, group_id, above]);
        assert_eq!(doc.scene.children_of(Some(group_id)), children.as_slice());
        assert_eq!(doc.scene.get(above).unwrap().index, above_key);
    }

    /// A gap can clear `near_precision_limit` and still be too narrow for
    /// one distinct key per child; the sequence check has to catch that too.
    #[test]
    fn ungrouping_many_children_into_a_narrow_gap_keeps_keys_distinct() {
        let (mut doc, page) = page_doc();
        let group_id = insert_with_index(
            &mut doc,
            frame(page, Transform2D::IDENTITY, [500.0, 50.0]),
            IndexKey::from_raw(1.0),
        );
        let above = insert_with_index(
            &mut doc,
            rect(page, 600.0, 0.0, 10.0, 10.0),
            IndexKey::from_raw(1.0 + 24.0 * f64::EPSILON),
        );
        let children: Vec<NodeId> = (0..40)
            .map(|position| {
                insert(
                    &mut doc,
                    rect(group_id, position as f64 * 10.0, 0.0, 8.0, 8.0),
                )
            })
            .collect();

        let ungrouped = ungroup_operations(&doc, group_id).unwrap();
        assert!(apply_transaction(&mut doc, "Ungroup", ungrouped.operations).unwrap());

        let mut expected = children;
        expected.push(above);
        assert_eq!(doc.scene.children_of(Some(page)), expected.as_slice());
        assert_strictly_increasing_keys(&doc, Some(page));
    }

    /// Two sibling groups ungrouped in one command, both in gaps narrow enough
    /// to trigger the renumber fallback. Planning the second against the
    /// pre-edit document made the two plans disagree about the parent's
    /// z-order: colliding keys that dragged the bystander layer into the middle
    /// of the stack, or a `SetIndex` on a wrapper the first plan had already
    /// deleted, which fails and aborts the whole transaction.
    #[test]
    fn ungrouping_two_siblings_at_once_keeps_the_z_order_consistent() {
        let (mut doc, page) = page_doc();
        let below = insert_with_index(
            &mut doc,
            rect(page, 0.0, 0.0, 10.0, 10.0),
            IndexKey::from_raw(0.5),
        );
        let first = insert_with_index(
            &mut doc,
            frame(page, Transform2D::translation(10.0, 10.0), [50.0, 50.0]),
            IndexKey::from_raw(1.0),
        );
        let second = insert_with_index(
            &mut doc,
            frame(page, Transform2D::translation(80.0, 10.0), [50.0, 50.0]),
            IndexKey::from_raw(1.0 + 2.0 * f64::EPSILON),
        );
        let bystander = insert_with_index(
            &mut doc,
            rect(page, 300.0, 0.0, 10.0, 10.0),
            IndexKey::from_raw(1.0 + 4.0 * f64::EPSILON),
        );
        let first_children: Vec<NodeId> = (0..3)
            .map(|position| insert(&mut doc, rect(first, position as f64 * 5.0, 0.0, 4.0, 4.0)))
            .collect();
        let second_children: Vec<NodeId> = vec![insert(&mut doc, rect(second, 0.0, 0.0, 4.0, 4.0))];
        let worlds_before: Vec<(NodeId, Transform2D)> = first_children
            .iter()
            .chain(second_children.iter())
            .map(|child| (*child, doc.scene.world_transform(*child).unwrap()))
            .collect();

        let (operations, children) =
            ungroup_many_operations(&doc, &[first, second]).expect("planning both ungroups");
        assert!(apply_transaction(&mut doc, "Ungroup", operations).unwrap());
        assert_eq!(children.len(), first_children.len() + second_children.len());

        let mut expected = vec![below];
        expected.extend(first_children.iter().copied());
        expected.extend(second_children.iter().copied());
        expected.push(bystander);
        assert_eq!(
            doc.scene.children_of(Some(page)),
            expected.as_slice(),
            "both groups' children keep their place and the bystander stays on top"
        );
        assert_strictly_increasing_keys(&doc, Some(page));
        for (child, before) in worlds_before {
            assert_transform_close(doc.scene.world_transform(child).unwrap(), before);
        }
        assert!(!doc.scene.contains(first));
        assert!(!doc.scene.contains(second));
    }

    #[test]
    fn ungroup_refuses_pages_masters_instances_and_leaves() {
        let (mut doc, page) = page_doc();
        assert!(
            ungroup_operations(&doc, page)
                .unwrap_err()
                .to_string()
                .contains("page")
        );

        let leaf = insert(&mut doc, rect(page, 0.0, 0.0, 10.0, 10.0));
        assert!(
            ungroup_operations(&doc, leaf)
                .unwrap_err()
                .to_string()
                .contains("not a group")
        );

        let master = insert(&mut doc, frame(page, Transform2D::IDENTITY, [100.0, 100.0]));
        let component = ComponentId::new();
        doc.components
            .defs
            .insert(component, ComponentDef::new(component, master, "Button"));
        assert!(
            ungroup_operations(&doc, master)
                .unwrap_err()
                .to_string()
                .contains("component master")
        );

        let mut instance = CanvasNode::new(NodeData::Instance(InstanceNode {
            component,
            overrides: Vec::new(),
            prop_values: Default::default(),
            derived: Vec::new(),
            local_size: [100.0, 100.0],
        }));
        instance.parent = Some(page);
        let instance = insert(&mut doc, instance);
        assert!(
            ungroup_operations(&doc, instance)
                .unwrap_err()
                .to_string()
                .contains("instance")
        );
    }

    #[test]
    fn image_layer_node_places_its_top_left_at_the_world_point() {
        let (mut doc, page) = page_doc();
        let frame_id = insert(
            &mut doc,
            frame(
                page,
                Transform2D::scale(2.0).then(&Transform2D::translation(100.0, 100.0)),
                [400.0, 400.0],
            ),
        );
        let asset = AssetId::new();

        let on_page = image_layer_node(
            &doc,
            asset,
            [800, 600],
            [400.0, 300.0],
            None,
            20.0,
            30.0,
            None,
        )
        .unwrap();
        assert_eq!(on_page.parent, Some(page));
        assert_eq!(on_page.name, "Image");
        assert_transform_close(on_page.transform, Transform2D::translation(20.0, 30.0));
        let NodeData::Bitmap(bitmap) = &on_page.data else {
            panic!("expected a bitmap node");
        };
        assert_eq!(bitmap.asset, asset);
        assert_eq!(bitmap.natural_size, [800, 600]);
        assert_eq!(bitmap.local_size, [400.0, 300.0]);

        let in_frame = image_layer_node(
            &doc,
            asset,
            [800, 600],
            [400.0, 300.0],
            Some(frame_id),
            120.0,
            140.0,
            Some("Hero"),
        )
        .unwrap();
        assert_eq!(in_frame.parent, Some(frame_id));
        assert_eq!(in_frame.name, "Hero");
        let world = in_frame
            .transform
            .then(&doc.scene.world_transform(frame_id).unwrap());
        assert_transform_close(world, Transform2D::translation(120.0, 140.0));

        let leaf = insert(&mut doc, rect(page, 0.0, 0.0, 10.0, 10.0));
        assert!(
            image_layer_node(&doc, asset, [8, 8], [8.0, 8.0], Some(leaf), 0.0, 0.0, None).is_err()
        );
        assert!(image_layer_node(&doc, asset, [8, 8], [0.0, 8.0], None, 0.0, 0.0, None).is_err());
    }
}
