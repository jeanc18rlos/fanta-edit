use std::collections::{HashMap, HashSet};

use anyhow::{Context as _, Result, bail};
use fanta_doc::{
    Action, AnimationClipId, AnimationTrack, AnimationTrackId, CanvasNode, Doc, DocId, IndexKey,
    KeyframeId, NodeData, NodeId, Operation, Transform2D,
};
use serde::{Deserialize, Serialize};

const CLIPBOARD_VERSION: u8 = 2;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct CanvasClipboard {
    version: u8,
    source_document: DocId,
    roots: Vec<ClipboardRoot>,
    nodes: Vec<CanvasNode>,
    motion_tracks: Vec<ClipboardMotionTrack>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ClipboardRoot {
    id: NodeId,
    parent: Option<NodeId>,
    world_transform: Transform2D,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ClipboardMotionTrack {
    clip: AnimationClipId,
    track: AnimationTrack,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClipboardPlacement {
    Paste,
    Duplicate,
}

pub(crate) struct PastedNodes {
    pub(crate) nodes: Vec<CanvasNode>,
    pub(crate) roots: Vec<NodeId>,
    motion_tracks: Vec<ClipboardMotionTrack>,
}

impl CanvasClipboard {
    pub(crate) fn capture(doc: &Doc) -> Option<Self> {
        Self::capture_roots(doc, editable_selection_roots(doc))
    }

    /// Capture the subtrees under `roots` (already reduced to top-level
    /// members: no root may be a descendant of another).
    fn capture_roots(doc: &Doc, mut roots: Vec<NodeId>) -> Option<Self> {
        roots.sort_by(|left, right| {
            let left = doc.scene.get(*left);
            let right = doc.scene.get(*right);
            left.map(|node| (node.parent, node.index, node.id))
                .cmp(&right.map(|node| (node.parent, node.index, node.id)))
        });
        if roots.is_empty() {
            return None;
        }

        let root_records = roots
            .iter()
            .filter_map(|id| {
                let node = doc.scene.get(*id)?;
                Some(ClipboardRoot {
                    id: *id,
                    parent: node.parent,
                    world_transform: doc.scene.world_transform(*id)?,
                })
            })
            .collect::<Vec<_>>();
        if root_records.len() != roots.len() {
            return None;
        }

        let nodes = roots
            .iter()
            .flat_map(|root| doc.scene.descendants_of(*root))
            .filter_map(|id| doc.scene.get(id).cloned())
            .collect::<Vec<_>>();
        let captured_ids = nodes
            .iter()
            .map(|node: &CanvasNode| node.id)
            .collect::<HashSet<_>>();
        let motion_tracks = doc
            .motion
            .clips
            .iter()
            .flat_map(|(clip, animation)| {
                animation
                    .tracks
                    .values()
                    .filter(|track| captured_ids.contains(&track.target.node))
                    .cloned()
                    .map(|track| ClipboardMotionTrack { clip: *clip, track })
            })
            .collect();
        Some(Self {
            version: CLIPBOARD_VERSION,
            source_document: doc.id,
            roots: root_records,
            nodes,
            motion_tracks,
        })
    }

    pub(crate) fn display_text(&self) -> String {
        let by_id = self
            .nodes
            .iter()
            .map(|node| (node.id, node))
            .collect::<HashMap<_, _>>();
        self.roots
            .iter()
            .filter_map(|root| by_id.get(&root.id))
            .map(|node| node.name.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub(crate) fn instantiate(
        &self,
        doc: &Doc,
        offset: f64,
        placement: ClipboardPlacement,
    ) -> Result<PastedNodes> {
        self.instantiate_offset(doc, (offset, offset), placement)
    }

    fn instantiate_offset(
        &self,
        doc: &Doc,
        offset: (f64, f64),
        placement: ClipboardPlacement,
    ) -> Result<PastedNodes> {
        if self.version != CLIPBOARD_VERSION {
            bail!("unsupported canvas clipboard version {}", self.version);
        }
        if self.source_document != doc.id {
            bail!(
                "cross-document canvas paste is not supported yet; assets, components, and variables were left unchanged"
            );
        }
        if self.roots.is_empty() || self.nodes.is_empty() {
            bail!("the canvas clipboard is empty");
        }

        let mut remap = HashMap::with_capacity(self.nodes.len());
        for node in &self.nodes {
            if remap.insert(node.id, NodeId::new()).is_some() {
                bail!("the canvas clipboard contains duplicate node ids");
            }
        }
        let roots_by_id = self
            .roots
            .iter()
            .map(|root| (root.id, root))
            .collect::<HashMap<_, _>>();
        let root_ids = roots_by_id.keys().copied().collect::<HashSet<_>>();
        let mut next_indices = HashMap::<Option<NodeId>, IndexKey>::new();
        let duplicate_indices = match placement {
            ClipboardPlacement::Paste => HashMap::new(),
            ClipboardPlacement::Duplicate => duplicate_root_indices(doc, &self.roots)?,
        };
        let paste_parent = (placement == ClipboardPlacement::Paste)
            .then(|| paste_destination_parent(doc, &root_ids))
            .flatten();
        let mut pasted_roots = Vec::with_capacity(self.roots.len());
        let mut pasted_nodes = Vec::with_capacity(self.nodes.len());

        for source in &self.nodes {
            let source_id = source.id;
            let mut node = source.clone();
            let Some(remapped_id) = remap.get(&source_id).copied() else {
                bail!("the canvas clipboard is missing a remapped node id");
            };
            node.id = remapped_id;
            if let Some(parent) = source.parent.and_then(|parent| remap.get(&parent).copied()) {
                node.parent = Some(parent);
            } else if root_ids.contains(&source_id) {
                let Some(root) = roots_by_id.get(&source_id).copied() else {
                    bail!("the canvas clipboard is missing root geometry");
                };
                let (destination_parent, source_world, destination_index) = match placement {
                    ClipboardPlacement::Paste => {
                        let next_index = next_indices
                            .entry(paste_parent)
                            .or_insert_with(|| doc.scene.next_child_index(paste_parent));
                        let index = *next_index;
                        *next_index = IndexKey::after(*next_index);
                        (paste_parent, root.world_transform, index)
                    }
                    ClipboardPlacement::Duplicate => {
                        let source_node = doc.scene.get(source_id).with_context(|| {
                            format!("duplicate source {source_id} no longer exists")
                        })?;
                        if source_node.parent != root.parent {
                            bail!("duplicate source {source_id} changed parent during capture");
                        }
                        let source_world =
                            doc.scene.world_transform(source_id).with_context(|| {
                                format!("duplicate source {source_id} has no world transform")
                            })?;
                        let index =
                            duplicate_indices
                                .get(&source_id)
                                .copied()
                                .with_context(|| {
                                    format!("duplicate source {source_id} has no insertion index")
                                })?;
                        (source_node.parent, source_world, index)
                    }
                };
                node.parent = destination_parent;
                let parent_world = destination_parent
                    .and_then(|parent| doc.scene.world_transform(parent))
                    .unwrap_or(Transform2D::IDENTITY);
                let determinant = parent_world.0.matrix2.determinant();
                if !determinant.is_finite() || determinant.abs() <= f64::EPSILON {
                    bail!("the paste destination has a non-invertible transform");
                }
                node.transform = source_world.then(&parent_world.inverse());
                node.transform = node
                    .transform
                    .then(&Transform2D::translation(offset.0, offset.1));
                node.index = destination_index;
                pasted_roots.push(node.id);
            } else {
                bail!("the canvas clipboard contains a child without its parent");
            }
            remap_node_references(&mut node, &remap);
            pasted_nodes.push(node);
        }

        let mut pasted_motion_tracks = Vec::with_capacity(self.motion_tracks.len());
        for captured in &self.motion_tracks {
            let mut track = captured.track.clone();
            let Some(target) = remap.get(&track.target.node).copied() else {
                bail!("the canvas clipboard contains a motion track outside its node subtree");
            };
            track.id = AnimationTrackId::new();
            track.target.node = target;
            let mut keyframes = std::collections::BTreeMap::new();
            for keyframe in track.keyframes.values() {
                let mut keyframe = keyframe.clone();
                keyframe.id = KeyframeId::new();
                keyframes.insert(keyframe.id, keyframe);
            }
            track.keyframes = keyframes;
            pasted_motion_tracks.push(ClipboardMotionTrack {
                clip: captured.clip,
                track,
            });
        }

        Ok(PastedNodes {
            nodes: pasted_nodes,
            roots: pasted_roots,
            motion_tracks: pasted_motion_tracks,
        })
    }
}

fn paste_destination_parent(doc: &Doc, source_roots: &HashSet<NodeId>) -> Option<NodeId> {
    let selected = doc
        .selection
        .iter()
        .copied()
        .filter(|id| node_is_on_active_page(doc, *id))
        .filter_map(|id| doc.scene.get(id).map(|node| (id, node)))
        .collect::<Vec<_>>();
    if let [(id, node)] = selected.as_slice()
        && !source_roots.contains(id)
        && node.can_have_children()
    {
        return Some(*id);
    }

    let common_parent = selected.first().and_then(|(_, node)| node.parent);
    if selected
        .iter()
        .all(|(_, node)| node.parent == common_parent)
        && common_parent.is_some_and(|parent| compatible_parent(doc, parent))
    {
        return common_parent;
    }

    doc.active_page()
        .filter(|page| compatible_parent(doc, *page))
}

fn compatible_parent(doc: &Doc, id: NodeId) -> bool {
    node_is_on_active_page(doc, id) && doc.scene.get(id).is_some_and(CanvasNode::can_have_children)
}

pub(crate) fn node_is_on_active_page(doc: &Doc, id: NodeId) -> bool {
    let Some(page) = doc.active_page() else {
        return true;
    };
    id == page
        || doc
            .scene
            .ancestors_of(id)
            .any(|ancestor| ancestor.id == page)
}

fn duplicate_root_indices(doc: &Doc, roots: &[ClipboardRoot]) -> Result<HashMap<NodeId, IndexKey>> {
    let mut by_parent = HashMap::<Option<NodeId>, Vec<(NodeId, IndexKey)>>::new();
    for root in roots {
        let source = doc
            .scene
            .get(root.id)
            .with_context(|| format!("duplicate source {} no longer exists", root.id))?;
        by_parent
            .entry(source.parent)
            .or_default()
            .push((source.id, source.index));
    }

    let mut result = HashMap::with_capacity(roots.len());
    for (parent, mut sources) in by_parent {
        sources.sort_by_key(|(_, index)| *index);
        let Some((_, anchor)) = sources.last().copied() else {
            continue;
        };
        let next = doc
            .scene
            .children_of(parent)
            .iter()
            .filter_map(|id| doc.scene.get(*id))
            .map(|node| node.index)
            .find(|index| *index > anchor);
        if let Some(next) = next {
            if IndexKey::near_precision_limit(anchor, next) {
                bail!("the duplicate insertion gap is exhausted; reorder the layer and retry");
            }
            let step = (next.raw() - anchor.raw()) / (sources.len() as f64 + 1.0);
            for (position, (id, _)) in sources.into_iter().enumerate() {
                let index = IndexKey::from_raw(anchor.raw() + step * (position as f64 + 1.0));
                if !(anchor < index && index < next) {
                    bail!("the duplicate insertion gap is exhausted; reorder the layer and retry");
                }
                result.insert(id, index);
            }
        } else {
            let mut index = anchor;
            for (id, _) in sources {
                index = IndexKey::after(index);
                result.insert(id, index);
            }
        }
    }
    Ok(result)
}

/// Deep-copy the subtrees under `roots` (ids nested under another root are
/// dropped, page roots are refused), each copy landing one z-slot above its
/// source, shifted by `offset` world units. The returned [`PastedNodes`] is
/// turned into ops with [`create_operations`]; `roots` names the copies.
pub(crate) fn duplicate_operations(
    doc: &Doc,
    roots: &[NodeId],
    offset: (f64, f64),
) -> Result<PastedNodes> {
    if !(offset.0.is_finite() && offset.1.is_finite()) {
        bail!("the duplicate offset must be finite");
    }
    let requested = roots.iter().copied().collect::<HashSet<_>>();
    let mut top_level = Vec::with_capacity(roots.len());
    for id in roots {
        if !doc.scene.contains(*id) {
            bail!("node {id} does not exist");
        }
        if doc.pages().contains(id) {
            bail!("refusing to duplicate a page root");
        }
        let nested = doc
            .scene
            .ancestors_of(*id)
            .any(|ancestor| requested.contains(&ancestor.id));
        if !nested && !top_level.contains(id) {
            top_level.push(*id);
        }
    }
    let clipboard =
        CanvasClipboard::capture_roots(doc, top_level).context("nothing to duplicate")?;
    clipboard.instantiate_offset(doc, offset, ClipboardPlacement::Duplicate)
}

pub(crate) fn delete_operations(doc: &Doc) -> Vec<Operation> {
    let mut deleted = HashSet::new();
    let mut operations = Vec::new();
    for root in editable_selection_roots(doc) {
        let snapshot = doc
            .scene
            .descendants_of(root)
            .filter_map(|id| doc.scene.get(id).cloned())
            .collect::<Vec<_>>();
        deleted.extend(snapshot.iter().map(|node| node.id));
        if !snapshot.is_empty() {
            operations.push(Operation::DeleteSubtree { snapshot });
        }
    }
    for (clip, animation) in &doc.motion.clips {
        for track in animation.tracks.values() {
            if deleted.contains(&track.target.node) {
                operations.push(Operation::SetAnimationTrack {
                    clip: *clip,
                    track: track.id,
                    old: Some(Box::new(track.clone())),
                    new: None,
                });
            }
        }
    }
    operations
}

pub(crate) fn create_operations(pasted: &PastedNodes) -> Vec<Operation> {
    let mut operations = pasted
        .nodes
        .iter()
        .cloned()
        .map(Operation::create_node)
        .collect::<Vec<_>>();
    operations.extend(
        pasted
            .motion_tracks
            .iter()
            .map(|captured| Operation::SetAnimationTrack {
                clip: captured.clip,
                track: captured.track.id,
                old: None,
                new: Some(Box::new(captured.track.clone())),
            }),
    );
    operations
}

pub(crate) fn apply_transaction(
    doc: &mut Doc,
    label: &str,
    operations: Vec<Operation>,
) -> Result<bool> {
    if operations.is_empty() {
        return Ok(false);
    }
    doc.history.begin(label, &mut doc.scene);
    for operation in operations {
        if let Err(error) = doc.apply(operation) {
            doc.abort_transaction()
                .context("rolling back the canvas edit")?;
            return Err(error).context("applying the canvas edit");
        }
    }
    doc.history.commit(&mut doc.scene);
    Ok(true)
}

fn editable_selection_roots(doc: &Doc) -> Vec<NodeId> {
    let selected = doc.selection.iter().copied().collect::<HashSet<_>>();
    doc.selection
        .iter()
        .copied()
        .filter(|id| Some(*id) != doc.active_page())
        .filter(|id| node_is_on_active_page(doc, *id))
        .filter(|id| doc.scene.get(*id).is_some())
        .filter(|id| {
            !doc.scene
                .ancestors_of(*id)
                .any(|node| selected.contains(&node.id))
        })
        .collect()
}

fn remap_node_references(node: &mut CanvasNode, remap: &HashMap<NodeId, NodeId>) {
    for reaction in &mut node.reactions {
        match &mut reaction.action {
            Action::Navigate { to } => remap_reference(to, remap),
            Action::OpenOverlay { frame, .. } => remap_reference(frame, remap),
            Action::ScrollTo { target } => remap_reference(target, remap),
            Action::Back
            | Action::Close
            | Action::SetVariable { .. }
            | Action::UpdateVariant { .. }
            | Action::OpenLink { .. } => {}
        }
    }
    if let NodeData::AiArtifact(artifact) = &mut node.data {
        for input in &mut artifact.inputs {
            remap_reference(input, remap);
        }
        if let Some(parent) = &mut artifact.lineage_parent {
            remap_reference(parent, remap);
        }
    }
}

fn remap_reference(reference: &mut NodeId, remap: &HashMap<NodeId, NodeId>) {
    if let Some(remapped) = remap.get(reference) {
        *reference = *remapped;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{
        AnimationClip, Color, ComponentDef, ComponentId, GroupNode, MotionProperty, MotionTarget,
        TextNode, VectorNode,
    };

    fn subtree_doc() -> (Doc, NodeId, NodeId, NodeId) {
        let mut doc = Doc::new();
        let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let page_id = page.id;
        doc.apply(Operation::create_node(page)).unwrap();
        doc.add_page(page_id);
        doc.set_active_page(Some(page_id));

        let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([200.0, 120.0]),
            ..Default::default()
        }));
        frame.parent = Some(page_id);
        frame.transform = Transform2D::translation(20.0, 30.0);
        let frame_id = frame.id;
        doc.apply(Operation::create_node(frame)).unwrap();

        let mut first = CanvasNode::new(NodeData::Text(TextNode::new("First", 80.0, 24.0)));
        first.parent = Some(frame_id);
        first.index = IndexKey::from_raw(1.0);
        first.transform = Transform2D::translation(8.0, 12.0);
        let first_id = first.id;
        doc.apply(Operation::create_node(first)).unwrap();

        let mut second = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            20.0,
            20.0,
            Color::WHITE,
        )));
        second.parent = Some(frame_id);
        second.index = IndexKey::from_raw(2.0);
        let second_id = second.id;
        doc.apply(Operation::create_node(second)).unwrap();
        doc.history = Default::default();
        (doc, frame_id, first_id, second_id)
    }

    #[test]
    fn paste_remaps_complete_subtree_and_is_one_undo_step() {
        let (mut doc, frame_id, first_id, second_id) = subtree_doc();
        doc.selection.select_only(frame_id);
        let payload = CanvasClipboard::capture(&doc).expect("capture subtree");
        let pasted = payload
            .instantiate(&doc, 16.0, ClipboardPlacement::Paste)
            .expect("instantiate subtree");
        let pasted_frame = pasted.roots[0];
        let pasted_ids = pasted
            .nodes
            .iter()
            .map(|node| node.id)
            .collect::<HashSet<_>>();
        assert!(!pasted_ids.contains(&frame_id));
        assert!(!pasted_ids.contains(&first_id));
        assert!(!pasted_ids.contains(&second_id));
        let pasted_children = pasted
            .nodes
            .iter()
            .filter(|node| node.parent == Some(pasted_frame))
            .collect::<Vec<_>>();
        assert_eq!(pasted_children.len(), 2);
        assert!(pasted_children[0].index < pasted_children[1].index);
        assert_eq!(
            pasted_children[0].transform,
            doc.scene.get(first_id).unwrap().transform
        );

        assert!(apply_transaction(&mut doc, "Paste", create_operations(&pasted)).unwrap());
        doc.selection.replace_with(pasted.roots.iter().copied());
        assert_eq!(doc.history.undo_depth(), 1);
        assert!(doc.scene.contains(pasted_frame));
        assert!(doc.undo().unwrap());
        assert!(!doc.scene.contains(pasted_frame));
        assert!(doc.redo().unwrap());
        assert!(doc.scene.contains(pasted_frame));
    }

    #[test]
    fn deleting_parent_and_selected_descendant_records_one_subtree_once() {
        let (mut doc, frame_id, first_id, _) = subtree_doc();
        doc.selection.select_only(frame_id);
        doc.selection.add(first_id);
        let operations = delete_operations(&doc);
        assert_eq!(operations.len(), 1);
        assert!(apply_transaction(&mut doc, "Delete", operations).unwrap());
        doc.selection.clear();
        assert_eq!(doc.history.undo_depth(), 1);
        assert!(!doc.scene.contains(frame_id));
        assert!(doc.undo().unwrap());
        assert!(doc.scene.contains(frame_id));
        assert!(doc.scene.contains(first_id));
    }

    #[test]
    fn capture_never_treats_the_active_page_as_editable_content() {
        let (mut doc, _, _, _) = subtree_doc();
        doc.selection.select_only(doc.active_page().unwrap());
        assert!(CanvasClipboard::capture(&doc).is_none());
        assert!(delete_operations(&doc).is_empty());
    }

    #[test]
    fn paste_uses_the_active_page_instead_of_a_source_parent_on_an_old_page() {
        let (mut doc, frame_id, _, _) = subtree_doc();
        doc.selection.select_only(frame_id);
        let payload = CanvasClipboard::capture(&doc).expect("capture frame");

        let second_page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let second_page_id = second_page.id;
        doc.apply(Operation::create_node(second_page)).unwrap();
        doc.add_page(second_page_id);
        doc.set_active_page(Some(second_page_id));

        let pasted = payload
            .instantiate(&doc, 16.0, ClipboardPlacement::Paste)
            .expect("paste onto active page");
        let pasted_root = pasted
            .nodes
            .iter()
            .find(|node| node.id == pasted.roots[0])
            .expect("pasted root");
        assert_eq!(pasted_root.parent, Some(second_page_id));
    }

    #[test]
    fn cross_document_paste_is_rejected_before_dependencies_can_dangle() {
        let (mut source, frame_id, _, _) = subtree_doc();
        source.selection.select_only(frame_id);
        let payload = CanvasClipboard::capture(&source).expect("capture frame");
        let (destination, _, _, _) = subtree_doc();

        let error = match payload.instantiate(&destination, 16.0, ClipboardPlacement::Paste) {
            Ok(_) => panic!("cross-document paste unexpectedly succeeded"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("cross-document"));
    }

    #[test]
    fn duplicate_inserts_a_copy_immediately_above_the_source() {
        let (mut doc, frame_id, first_id, _) = subtree_doc();
        doc.scene.get_mut(first_id).unwrap().is_mask = true;
        let page = doc.active_page().unwrap();
        let mut upper = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            10.0,
            10.0,
            Color::BLACK,
        )));
        upper.parent = Some(page);
        upper.index = IndexKey::from_raw(2.0);
        let upper_id = upper.id;
        doc.apply(Operation::create_node(upper)).unwrap();
        doc.selection.select_only(frame_id);
        let payload = CanvasClipboard::capture(&doc).expect("capture frame");

        let pasted = payload
            .instantiate(&doc, 16.0, ClipboardPlacement::Duplicate)
            .expect("duplicate frame");
        let duplicate = pasted
            .nodes
            .iter()
            .find(|node| node.id == pasted.roots[0])
            .expect("duplicate root");
        let source_index = doc.scene.get(frame_id).unwrap().index;
        let upper_index = doc.scene.get(upper_id).unwrap().index;
        assert!(source_index < duplicate.index && duplicate.index < upper_index);
        let duplicated_children = pasted
            .nodes
            .iter()
            .filter(|node| node.parent == Some(duplicate.id))
            .collect::<Vec<_>>();
        assert_eq!(duplicated_children.len(), 2);
        assert!(duplicated_children[0].is_mask);
        assert!(duplicated_children[0].index < duplicated_children[1].index);
    }

    #[test]
    fn duplicate_preserves_a_selected_mask_and_its_following_sibling_order() {
        let (mut doc, _, first_id, second_id) = subtree_doc();
        doc.scene.get_mut(first_id).unwrap().is_mask = true;
        doc.selection.select_only(first_id);
        doc.selection.add(second_id);
        let payload = CanvasClipboard::capture(&doc).expect("capture mask pair");

        let pasted = payload
            .instantiate(&doc, 16.0, ClipboardPlacement::Duplicate)
            .expect("duplicate mask pair");
        assert_eq!(pasted.roots.len(), 2);
        let first = pasted
            .nodes
            .iter()
            .find(|node| node.id == pasted.roots[0])
            .expect("duplicated mask");
        let second = pasted
            .nodes
            .iter()
            .find(|node| node.id == pasted.roots[1])
            .expect("duplicated masked sibling");
        assert!(first.is_mask);
        assert!(first.index < second.index);
        assert!(doc.scene.get(second_id).unwrap().index < first.index);
    }

    #[test]
    fn duplicate_clones_motion_tracks_with_remapped_targets() {
        let (mut doc, frame_id, first_id, _) = subtree_doc();
        let clip_id = AnimationClipId::new();
        let mut clip = AnimationClip::new(clip_id, "Entrance", 1_000);
        let track_id = AnimationTrackId::new();
        clip.tracks.insert(
            track_id,
            AnimationTrack::new(
                track_id,
                MotionTarget::new(first_id, MotionProperty::PositionX),
            ),
        );
        doc.motion.clips.insert(clip_id, clip);
        doc.selection.select_only(frame_id);
        let payload = CanvasClipboard::capture(&doc).expect("capture animated frame");
        let pasted = payload
            .instantiate(&doc, 16.0, ClipboardPlacement::Duplicate)
            .expect("duplicate animated frame");
        assert_eq!(pasted.motion_tracks.len(), 1);
        let remapped_target = pasted.motion_tracks[0].track.target.node;
        assert_ne!(remapped_target, first_id);
        assert!(pasted.nodes.iter().any(|node| node.id == remapped_target));

        assert!(apply_transaction(&mut doc, "Duplicate", create_operations(&pasted)).unwrap());
        let clip = doc.motion.clip(clip_id).expect("motion clip");
        assert_eq!(clip.tracks.len(), 2);
        assert!(
            clip.tracks
                .values()
                .any(|track| track.target.node == remapped_target)
        );
    }

    #[test]
    fn duplicate_operations_copies_explicit_roots_with_an_offset() {
        let (mut doc, frame_id, first_id, _) = subtree_doc();
        let pasted = duplicate_operations(&doc, &[frame_id, first_id], (10.0, 5.0))
            .expect("duplicate the frame");
        assert_eq!(pasted.roots.len(), 1, "the nested child is not a root");
        let copy = pasted.roots[0];
        assert!(apply_transaction(&mut doc, "Duplicate", create_operations(&pasted)).unwrap());
        let source = doc.scene.world_bounds(frame_id).unwrap();
        let duplicate = doc.scene.world_bounds(copy).unwrap();
        assert_eq!(duplicate.min_x - source.min_x, 10.0);
        assert_eq!(duplicate.min_y - source.min_y, 5.0);
        assert_eq!(doc.scene.children_of(Some(copy)).len(), 2);
        assert!(doc.scene.get(frame_id).unwrap().index < doc.scene.get(copy).unwrap().index);
    }

    #[test]
    fn duplicate_operations_refuses_page_roots_and_missing_nodes() {
        let (doc, _, _, _) = subtree_doc();
        let page = doc.active_page().unwrap();
        assert!(duplicate_operations(&doc, &[page], (0.0, 0.0)).is_err());
        assert!(duplicate_operations(&doc, &[NodeId::new()], (0.0, 0.0)).is_err());
        assert!(duplicate_operations(&doc, &[], (0.0, 0.0)).is_err());
    }

    #[test]
    fn deleting_component_content_bumps_master_revision_on_delete_and_undo() {
        let (mut doc, frame_id, first_id, _) = subtree_doc();
        let component = ComponentId::new();
        doc.components.defs.insert(
            component,
            ComponentDef::new(component, frame_id, "Frame master"),
        );
        doc.selection.select_only(first_id);

        let operations = delete_operations(&doc);
        assert!(apply_transaction(&mut doc, "Delete", operations).unwrap());
        assert_eq!(doc.components.defs[&component].rev, 1);
        assert!(doc.undo().unwrap());
        assert_eq!(doc.components.defs[&component].rev, 2);
    }
}
