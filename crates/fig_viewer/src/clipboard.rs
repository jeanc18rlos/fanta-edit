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

pub(crate) fn duplicate_page_operations(doc: &Doc, root: NodeId) -> Result<Vec<Operation>> {
    let index = doc
        .pages()
        .iter()
        .position(|id| *id == root)
        .context("page no longer exists")?;
    let clipboard = CanvasClipboard::capture_roots(doc, vec![root]).context("page is empty")?;
    let mut pasted =
        clipboard.instantiate_offset(doc, (0.0, 0.0), ClipboardPlacement::Duplicate)?;
    let copy = *pasted.roots.first().context("missing duplicated page")?;
    if let Some(node) = pasted.nodes.iter_mut().find(|node| node.id == copy) {
        node.name = format!("{} Copy", node.name);
    }
    let mut operations = clone_component_operations(doc, &clipboard, &mut pasted)?;
    let old = doc.pages().to_vec();
    let mut new = old.clone();
    new.insert(index + 1, copy);
    operations.push(Operation::SetPageRegistry {
        old_pages: old,
        new_pages: new,
        old_active_page: doc.active_page(),
        new_active_page: doc.active_page(),
    });
    Ok(operations)
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

pub(crate) fn editable_selection_roots(doc: &Doc) -> Vec<NodeId> {
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
        for action in std::iter::once(&mut reaction.action).chain(&mut reaction.extra_actions) {
            match action {
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

    fn prototype_reference_actions(
        frame: NodeId,
        scroll_target: NodeId,
        external_target: NodeId,
    ) -> Vec<Action> {
        let overlay = fanta_doc::OverlaySettings {
            position: fanta_doc::OverlayPosition::Manual {
                offset: [12.0, 34.0],
            },
            background_dim: true,
            close_on_click_outside: true,
        };
        vec![
            Action::Navigate { to: frame },
            Action::OpenOverlay {
                frame,
                overlay: overlay.clone(),
            },
            Action::ScrollTo {
                target: scroll_target,
            },
            Action::Navigate {
                to: external_target,
            },
            Action::OpenOverlay {
                frame: external_target,
                overlay,
            },
            Action::ScrollTo {
                target: external_target,
            },
        ]
    }

    fn prototype_subtree_doc() -> (Doc, NodeId, NodeId, NodeId) {
        let (mut doc, frame, trigger, scroll_target) = subtree_doc();
        let external_page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let external_page_id = external_page.id;
        doc.apply(Operation::create_node(external_page))
            .expect("external page");
        doc.add_page(external_page_id);
        let mut external = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([100.0, 100.0]),
            ..Default::default()
        }));
        external.parent = Some(external_page_id);
        let external_target = external.id;
        doc.apply(Operation::create_node(external))
            .expect("external target");
        let actions = prototype_reference_actions(frame, scroll_target, external_target);
        let mut extra_actions = actions.clone();
        extra_actions.extend([
            Action::Back,
            Action::Close,
            Action::OpenLink {
                url: "https://example.com/unchanged".into(),
            },
            Action::SetVariable {
                variable: fanta_doc::VariableId::new(),
                value: fanta_doc::VarValue::Boolean { value: true },
            },
            Action::UpdateVariant {
                component: ComponentId::new(),
                variant: "Pressed".into(),
            },
        ]);
        doc.scene.get_mut(trigger).expect("trigger").reactions = actions
            .into_iter()
            .map(|action| fanta_doc::Reaction {
                id: fanta_doc::ReactionId::new(),
                trigger: fanta_doc::Trigger::Click,
                action,
                extra_actions: extra_actions.clone(),
                transition: Some(fanta_doc::Transition {
                    style: fanta_doc::TransitionStyle::Dissolve,
                    duration_ms: 175,
                    easing: fanta_doc::Easing::EaseIn,
                }),
                animation: None,
            })
            .collect();
        doc.history = Default::default();
        (doc, frame, trigger, external_target)
    }

    fn assert_copied_prototype_reactions(
        doc: &Doc,
        original_trigger: NodeId,
        copied_frame: NodeId,
        external_target: NodeId,
    ) {
        let children = doc.scene.children_of(Some(copied_frame));
        let copied_trigger = children
            .iter()
            .filter_map(|id| doc.scene.get(*id))
            .find(|node| matches!(node.data, NodeData::Text(_)))
            .expect("copied text trigger");
        let copied_scroll_target = children
            .iter()
            .filter_map(|id| doc.scene.get(*id))
            .find(|node| matches!(node.data, NodeData::Vector(_)))
            .expect("copied scroll target");
        let source = doc.scene.get(original_trigger).expect("source trigger");
        let expected_actions =
            prototype_reference_actions(copied_frame, copied_scroll_target.id, external_target);
        assert_eq!(source.reactions.len(), expected_actions.len());
        assert_eq!(copied_trigger.reactions.len(), source.reactions.len());
        for ((source, copied), expected_action) in source
            .reactions
            .iter()
            .zip(&copied_trigger.reactions)
            .zip(&expected_actions)
        {
            let mut expected = source.clone();
            expected.action = expected_action.clone();
            expected.extra_actions = expected_actions.clone();
            expected.extra_actions.extend(
                source
                    .extra_actions
                    .iter()
                    .skip(expected_actions.len())
                    .cloned(),
            );
            assert_eq!(copied, &expected);
        }
    }

    #[test]
    fn paste_and_subtree_duplicate_remap_all_prototype_actions_and_undo() {
        for placement in [ClipboardPlacement::Paste, ClipboardPlacement::Duplicate] {
            let (mut doc, frame, trigger, external_target) = prototype_subtree_doc();
            doc.selection.select_only(frame);
            let original_scene = serde_json::to_value(&doc.scene).expect("original scene");
            let original_pages = doc.pages().to_vec();
            let original_active_page = doc.active_page();
            let source_trigger = doc.scene.get(trigger).expect("source trigger").clone();
            let external = doc.scene.get(external_target).expect("external").clone();
            let pasted = match placement {
                ClipboardPlacement::Paste => CanvasClipboard::capture(&doc)
                    .expect("capture")
                    .instantiate(&doc, 16.0, placement)
                    .expect("paste"),
                ClipboardPlacement::Duplicate => {
                    duplicate_operations(&doc, &[frame], (16.0, 16.0)).expect("duplicate")
                }
            };
            let copy = *pasted.roots.first().expect("copied frame");
            assert!(
                apply_transaction(&mut doc, "Copy subtree", create_operations(&pasted))
                    .expect("apply copy")
            );
            assert_copied_prototype_reactions(&doc, trigger, copy, external_target);
            assert_eq!(doc.scene.get(trigger), Some(&source_trigger));
            assert_eq!(doc.scene.get(external_target), Some(&external));
            assert_eq!(doc.history.undo_depth(), 1);
            let copied_scene = serde_json::to_value(&doc.scene).expect("copied scene");
            assert!(doc.undo().expect("undo copy"));
            assert_eq!(
                serde_json::to_value(&doc.scene).expect("undone scene"),
                original_scene
            );
            assert_eq!(doc.pages(), original_pages);
            assert_eq!(doc.active_page(), original_active_page);
            assert_eq!(doc.history.undo_depth(), 0);
            assert!(doc.redo().expect("redo copy"));
            assert_eq!(
                serde_json::to_value(&doc.scene).expect("redone scene"),
                copied_scene
            );
        }
    }

    #[test]
    fn page_duplicate_remaps_all_prototype_actions_and_undo() {
        let (mut doc, frame, trigger, external_target) = prototype_subtree_doc();
        let page = doc.active_page().expect("source page");
        let original_scene = serde_json::to_value(&doc.scene).expect("original scene");
        let original_pages = doc.pages().to_vec();
        let original_trigger = doc.scene.get(trigger).expect("source trigger").clone();
        let external = doc.scene.get(external_target).expect("external").clone();
        let operations = duplicate_page_operations(&doc, page).expect("duplicate page");
        assert!(apply_transaction(&mut doc, "Duplicate page", operations).expect("apply copy"));
        let copied_page = *doc.pages().get(1).expect("copied page");
        let copied_frame = *doc
            .scene
            .children_of(Some(copied_page))
            .first()
            .expect("copied frame");
        assert_ne!(copied_frame, frame);
        assert_copied_prototype_reactions(&doc, trigger, copied_frame, external_target);
        assert_eq!(doc.scene.get(trigger), Some(&original_trigger));
        assert_eq!(doc.scene.get(external_target), Some(&external));
        assert_eq!(doc.history.undo_depth(), 1);
        let copied_scene = serde_json::to_value(&doc.scene).expect("copied scene");
        let copied_pages = doc.pages().to_vec();
        assert!(doc.undo().expect("undo page copy"));
        assert_eq!(
            serde_json::to_value(&doc.scene).expect("undone scene"),
            original_scene
        );
        assert_eq!(doc.pages(), original_pages);
        assert_eq!(doc.active_page(), Some(page));
        assert_eq!(doc.history.undo_depth(), 0);
        assert!(doc.redo().expect("redo page copy"));
        assert_eq!(
            serde_json::to_value(&doc.scene).expect("redone scene"),
            copied_scene
        );
        assert_eq!(doc.pages(), copied_pages);
        assert_eq!(doc.active_page(), Some(page));
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
    fn duplicating_a_variant_set_frame_makes_a_set_framed_by_the_copy() {
        let mut doc = Doc::new();
        let page = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode::default()));
        let page_id = page.id;
        doc.apply(Operation::create_node(page)).expect("page");
        doc.add_page(page_id);
        doc.set_active_page(Some(page_id));
        let mut roots = Vec::new();
        for name in ["Size=S", "Size=L"] {
            let mut master = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode {
                clip_size: Some([40.0, 20.0]),
                ..Default::default()
            }));
            master.name = name.into();
            master.parent = Some(page_id);
            master.index = doc.scene.next_child_index(Some(page_id));
            let root = master.id;
            doc.apply(Operation::create_node(master)).expect("master");
            let component = fanta_doc::ComponentId::new();
            doc.apply(Operation::DefineComponent {
                def: Box::new(fanta_doc::ComponentDef::new(component, root, name)),
            })
            .expect("component");
            roots.push(root);
        }
        let edit = crate::variant_sets::combine_variants(&doc, &roots, Some("Chip")).expect("set");
        for operation in edit.operations {
            doc.apply(operation).expect("combine");
        }
        let frame = edit.frame.expect("frame");

        for operation in duplicate_layer_operations(&doc, frame).expect("duplicate") {
            doc.apply(operation).expect("apply duplicate");
        }
        let copy = doc
            .components
            .sets
            .values()
            .find(|set| set.id != edit.set)
            .expect("the copied set");
        let copied_frame = copy.root.expect("the copied set has a frame");
        assert_ne!(copied_frame, frame);
        let copied_roots: Vec<_> = copy
            .members
            .iter()
            .map(|member| doc.components.def(*member).expect("copied variant").root)
            .collect();
        assert_eq!(
            doc.scene.children_of(Some(copied_frame)),
            copied_roots.as_slice()
        );
        assert_eq!(doc.components.sets[&edit.set].root, Some(frame));
    }

    fn component_variant_actions(
        set: ComponentId,
        member: ComponentId,
        external: ComponentId,
        member_first: bool,
    ) -> Vec<Action> {
        let (first, second) = if member_first {
            (member, set)
        } else {
            (set, member)
        };
        vec![
            Action::UpdateVariant {
                component: first,
                variant: "Hover".into(),
            },
            Action::OpenLink {
                url: "https://example.com/unchanged".into(),
            },
            Action::UpdateVariant {
                component: second,
                variant: "Default".into(),
            },
            Action::UpdateVariant {
                component: external,
                variant: "External".into(),
            },
            Action::Close,
        ]
    }

    fn component_variant_reference_doc()
    -> (Doc, NodeId, NodeId, ComponentId, ComponentId, ComponentId) {
        let mut doc = Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "Component page".into();
        let page_id = page.id;
        doc.apply(Operation::create_node(page)).expect("page");
        doc.add_page(page_id);
        doc.set_active_page(Some(page_id));
        let mut section = CanvasNode::new(NodeData::Group(GroupNode::default()));
        section.name = "Component section".into();
        section.parent = Some(page_id);
        let section_id = section.id;
        doc.apply(Operation::create_node(section)).expect("section");
        let mut frame = CanvasNode::new(NodeData::Group(GroupNode::default()));
        frame.name = "Variant set".into();
        frame.parent = Some(section_id);
        let frame_id = frame.id;
        doc.apply(Operation::create_node(frame)).expect("set frame");
        let set = ComponentId::new();
        let member = ComponentId::new();
        let hover = ComponentId::new();
        let external = ComponentId::new();
        let mut external_master = CanvasNode::new(NodeData::Group(GroupNode::default()));
        external_master.name = "External master".into();
        let external_root = external_master.id;
        doc.apply(Operation::create_node(external_master))
            .expect("external master");
        doc.apply(Operation::DefineComponent {
            def: Box::new(ComponentDef::new(external, external_root, "External")),
        })
        .expect("external definition");
        for (component, state) in [(member, "Default"), (hover, "Hover")] {
            let mut master = CanvasNode::new(NodeData::Group(GroupNode {
                clip_size: Some([40.0, 20.0]),
                ..Default::default()
            }));
            master.name = format!("State={state}");
            master.parent = Some(frame_id);
            master.index = doc.scene.next_child_index(Some(frame_id));
            let actions = component_variant_actions(set, member, external, false);
            master.reactions.push(fanta_doc::Reaction {
                id: fanta_doc::ReactionId::new(),
                trigger: fanta_doc::Trigger::Click,
                action: actions[0].clone(),
                extra_actions: actions[1..].to_vec(),
                transition: None,
                animation: None,
            });
            let root = master.id;
            doc.apply(Operation::create_node(master))
                .expect("variant master");
            let mut definition = ComponentDef::new(component, root, format!("State={state}"));
            definition.variant_of = Some(fanta_doc::ComponentSetMembership {
                set,
                axis_values: [("State".into(), state.into())].into_iter().collect(),
            });
            doc.apply(Operation::DefineComponent {
                def: Box::new(definition),
            })
            .expect("member definition");
        }
        doc.apply(Operation::DefineComponentSet {
            set: Box::new(fanta_doc::ComponentSet {
                id: set,
                name: "Button".into(),
                axes: vec![fanta_doc::VariantAxis {
                    name: "State".into(),
                    values: vec!["Default".into(), "Hover".into()],
                }],
                members: vec![member, hover],
                default_variant: member,
                root: Some(frame_id),
            }),
        })
        .expect("component set");
        let mut external_nested = CanvasNode::new(NodeData::Instance(fanta_doc::InstanceNode {
            component: member,
            overrides: Vec::new(),
            prop_values: Default::default(),
            derived: Vec::new(),
            local_size: [40.0, 20.0],
        }));
        external_nested.name = "External override target".into();
        external_nested.parent = Some(external_root);
        let external_nested_id = external_nested.id;
        doc.apply(Operation::create_node(external_nested))
            .expect("external nested instance");
        for (name, component) in [
            ("Set placement", set),
            ("Member placement", member),
            ("External placement", external),
        ] {
            let mut instance = CanvasNode::new(NodeData::Instance(fanta_doc::InstanceNode {
                component,
                overrides: Vec::new(),
                prop_values: Default::default(),
                derived: Vec::new(),
                local_size: [40.0, 20.0],
            }));
            if component == external {
                let NodeData::Instance(placement) = &mut instance.data else {
                    panic!("fixture instance")
                };
                placement.overrides.push(fanta_doc::Override {
                    target_path: [external_nested_id].into_iter().collect(),
                    target_prop: fanta_doc::BoundProp::Visible,
                    value: fanta_doc::OverrideValue::SwapInstance { component: set },
                });
            }
            instance.name = name.into();
            instance.parent = Some(section_id);
            instance.index = doc.scene.next_child_index(Some(section_id));
            let actions = component_variant_actions(set, member, external, true);
            instance.reactions.push(fanta_doc::Reaction {
                id: fanta_doc::ReactionId::new(),
                trigger: fanta_doc::Trigger::Hover,
                action: actions[0].clone(),
                extra_actions: actions[1..].to_vec(),
                transition: None,
                animation: None,
            });
            doc.apply(Operation::create_node(instance))
                .expect("placement");
        }
        let container = ComponentId::new();
        let mut master = CanvasNode::new(NodeData::Group(GroupNode::default()));
        master.name = "Swap container".into();
        master.parent = Some(section_id);
        master.index = doc.scene.next_child_index(Some(section_id));
        let master_id = master.id;
        doc.apply(Operation::create_node(master))
            .expect("swap master");
        let mut overrides = Vec::new();
        for (index, component) in [set, member, external].into_iter().enumerate() {
            let mut nested = CanvasNode::new(NodeData::Instance(fanta_doc::InstanceNode {
                component: external,
                overrides: Vec::new(),
                prop_values: Default::default(),
                derived: Vec::new(),
                local_size: [40.0, 20.0],
            }));
            nested.name = format!("Nested swap {index}");
            nested.parent = Some(master_id);
            nested.index = doc.scene.next_child_index(Some(master_id));
            overrides.push(fanta_doc::Override {
                target_path: [nested.id].into_iter().collect(),
                target_prop: fanta_doc::BoundProp::Visible,
                value: fanta_doc::OverrideValue::SwapInstance { component },
            });
            doc.apply(Operation::create_node(nested))
                .expect("nested instance");
        }
        doc.apply(Operation::DefineComponent {
            def: Box::new(ComponentDef::new(container, master_id, "Swap container")),
        })
        .expect("swap container definition");
        let mut placement = CanvasNode::new(NodeData::Instance(fanta_doc::InstanceNode {
            component: container,
            overrides,
            prop_values: Default::default(),
            derived: Vec::new(),
            local_size: [120.0, 20.0],
        }));
        placement.name = "Swap placement".into();
        placement.parent = Some(section_id);
        placement.index = doc.scene.next_child_index(Some(section_id));
        doc.apply(Operation::create_node(placement))
            .expect("swap placement");
        doc.selection.select_only(section_id);
        doc.history = Default::default();
        (doc, page_id, section_id, set, member, external)
    }

    fn component_copy_state(doc: &Doc) -> serde_json::Value {
        serde_json::json!({
            "scene": doc.scene, "components": doc.components,
            "pages": doc.pages(), "active_page": doc.active_page(), "selection": doc.selection,
        })
    }

    #[test]
    fn component_aware_copy_remaps_variant_actions_and_set_instances_with_undo() {
        for duplicate_page in [true, false] {
            let (mut doc, page, section, set, member, external) = component_variant_reference_doc();
            let before = component_copy_state(&doc);
            let source = doc.clone();
            let operations = if duplicate_page {
                duplicate_page_operations(&doc, page).expect("duplicate page")
            } else {
                duplicate_layer_operations(&doc, section).expect("duplicate component section")
            };
            assert!(
                apply_transaction(&mut doc, "Duplicate component content", operations)
                    .expect("apply duplicate")
            );
            assert_eq!(
                doc.components.defs.len(),
                source.components.defs.len() * 2 - 1,
                "the external component dependency is not cloned"
            );
            assert_eq!(doc.components.sets.len(), 2);
            let copied_set = doc
                .components
                .sets
                .values()
                .find(|candidate| candidate.id != set)
                .expect("copied set");
            let copied_member = copied_set.default_variant;
            let copied_root = if duplicate_page {
                doc.pages()[1]
            } else {
                *doc.scene
                    .children_of(Some(page))
                    .iter()
                    .find(|id| **id != section)
                    .expect("copied section")
            };
            let copy_nodes: Vec<_> = doc
                .scene
                .descendants_of(copied_root)
                .filter_map(|id| doc.scene.get(id))
                .collect();
            for source_id in source.scene.descendants_of(section) {
                let original = source.scene.get(source_id).expect("original node");
                assert_eq!(
                    doc.scene.get(source_id),
                    Some(original),
                    "copy must not retarget the original"
                );
                if original.reactions.is_empty() {
                    continue;
                }
                let copy = copy_nodes
                    .iter()
                    .find(|node| node.name == original.name)
                    .expect("copied interactive node");
                let expected = component_variant_actions(
                    copied_set.id,
                    copied_member,
                    external,
                    matches!(&original.data, NodeData::Instance(_)),
                );
                let mut expected_reaction = original.reactions[0].clone();
                expected_reaction.action = expected[0].clone();
                expected_reaction.extra_actions = expected[1..].to_vec();
                assert_eq!(
                    copy.reactions.as_slice(),
                    &[expected_reaction],
                    "{}",
                    original.name
                );
                if let NodeData::Instance(instance) = &copy.data {
                    let expected_component = match original.name.as_str() {
                        "Set placement" => copied_set.id,
                        "Member placement" => copied_member,
                        "External placement" => external,
                        _ => panic!("unexpected fixture instance"),
                    };
                    assert_eq!(instance.component, expected_component, "{}", original.name);
                    if original.name == "External placement" {
                        let NodeData::Instance(original_instance) = &original.data else {
                            panic!("fixture instance")
                        };
                        let mut expected_override = original_instance.overrides[0].clone();
                        expected_override.value = fanta_doc::OverrideValue::SwapInstance {
                            component: copied_set.id,
                        };
                        assert_eq!(
                            instance.overrides,
                            vec![expected_override],
                            "the external master path is retained while the copied swap destination changes"
                        );
                    }
                }
            }
            let copied_container = doc
                .components
                .defs
                .values()
                .find(|definition| {
                    !source.components.defs.contains_key(&definition.id)
                        && definition.name == "Swap container"
                })
                .expect("copied swap container");
            let copy = copy_nodes
                .iter()
                .find(|node| node.name == "Swap placement")
                .expect("copied swap placement");
            let NodeData::Instance(instance) = &copy.data else {
                panic!("fixture instance")
            };
            assert_eq!(instance.component, copied_container.id);
            assert_eq!(instance.overrides.len(), 3);
            for (index, component) in [copied_set.id, copied_member, external]
                .into_iter()
                .enumerate()
            {
                let nested = copy_nodes
                    .iter()
                    .find(|node| node.name == format!("Nested swap {index}"))
                    .expect("copied override target");
                assert_eq!(
                    instance.overrides[index].target_path.as_slice(),
                    &[nested.id]
                );
                assert_eq!(
                    instance.overrides[index].value,
                    fanta_doc::OverrideValue::SwapInstance { component }
                );
            }
            assert_eq!(
                doc.components.defs.get(&member),
                source.components.defs.get(&member)
            );
            assert_eq!(
                doc.components.defs.get(&external),
                source.components.defs.get(&external)
            );
            let after = component_copy_state(&doc);
            assert!(doc.undo().expect("undo duplicate"));
            assert_eq!(
                component_copy_state(&doc),
                before,
                "one Undo restores all content and references"
            );
            assert!(!doc.undo().expect("no second duplicate transaction"));
            assert!(doc.redo().expect("redo duplicate"));
            assert_eq!(
                component_copy_state(&doc),
                after,
                "Redo restores the same copied IDs and references"
            );
        }
    }

    #[test]
    fn component_aware_copy_keeps_uncopied_variant_sets_and_external_targets() {
        let (mut doc, _, _, set, member, external) = component_variant_reference_doc();
        let master = doc.components.def(member).expect("original member").root;
        doc.selection.select_only(master);
        let before = component_copy_state(&doc);
        let operations = duplicate_layer_operations(&doc, master).expect("copy only one variant");
        assert!(apply_transaction(&mut doc, "Duplicate variant", operations).expect("apply"));
        assert_eq!(
            doc.components.sets.len(),
            1,
            "a partial variant set is not copied"
        );
        let copied_member = doc
            .components
            .defs
            .values()
            .find(|definition| definition.id != member && definition.name == "State=Default")
            .expect("copied member");
        assert!(copied_member.variant_of.is_none());
        let copy = doc.scene.get(copied_member.root).expect("copied master");
        assert_eq!(
            copy.reactions[0].actions().cloned().collect::<Vec<_>>(),
            component_variant_actions(set, copied_member.id, external, false)
        );
        let after = component_copy_state(&doc);
        assert!(doc.undo().expect("undo"));
        assert_eq!(component_copy_state(&doc), before);
        assert!(doc.redo().expect("redo"));
        assert_eq!(component_copy_state(&doc), after);
    }

    #[test]
    fn page_duplicate_preserves_component_masters_and_local_instances() {
        let mut doc = Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode::default()));
        page.name = "Components".into();
        let root = page.id;
        doc.apply(Operation::create_node(page)).expect("page");
        doc.add_page(root);
        let mut master = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode::default()));
        master.parent = Some(root);
        let master_id = master.id;
        doc.apply(Operation::create_node(master)).expect("master");
        let component = fanta_doc::ComponentId::new();
        doc.apply(Operation::DefineComponent {
            def: Box::new(fanta_doc::ComponentDef::new(component, master_id, "Button")),
        })
        .expect("definition");
        let mut instance = CanvasNode::new(NodeData::Instance(fanta_doc::InstanceNode {
            component,
            overrides: Vec::new(),
            prop_values: Default::default(),
            derived: Vec::new(),
            local_size: [20.0, 20.0],
        }));
        instance.parent = Some(root);
        doc.apply(Operation::create_node(instance))
            .expect("instance");
        let operations = duplicate_page_operations(&doc, root).expect("duplicate ops");
        apply_transaction(&mut doc, "Duplicate page", operations).expect("apply");
        let copy = doc.pages()[1];
        let copied_component = doc
            .components
            .defs
            .values()
            .find(|definition| definition.id != component)
            .expect("copied component");
        assert_eq!(
            doc.scene
                .get(copied_component.root)
                .expect("copied master")
                .parent,
            Some(copy)
        );
        let instance_component = doc
            .scene
            .children_of(Some(copy))
            .iter()
            .filter_map(|id| match &doc.scene.get(*id)?.data {
                NodeData::Instance(instance) => Some(instance.component),
                _ => None,
            })
            .next()
            .expect("copied instance");
        assert_eq!(instance_component, copied_component.id);
        doc.undo().expect("undo page copy");
        assert_eq!(doc.pages(), &[root]);
        assert_eq!(doc.components.defs.len(), 1);
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

#[cfg(feature = "fanta-gpui-ui")]
pub(crate) fn paste_to_replace_operations(
    doc: &Doc,
    id: NodeId,
    payload: &CanvasClipboard,
) -> Result<Vec<Operation>> {
    let targets = crate::layer_context_ops::targets(doc, id);
    let mut scratch = doc.clone();
    let mut operations = Vec::new();
    for id in targets {
        anyhow::ensure!(
            crate::layer_context_ops::editable(&scratch, id),
            "A selected layer is locked"
        );
        let target = scratch
            .scene
            .get(id)
            .context("Missing replacement target")?
            .clone();
        let bounds = scratch
            .scene
            .world_bounds(id)
            .context("The replacement target has no bounds")?;
        scratch.selection.replace_with([id]);
        let mut pasted = payload.instantiate(&scratch, 0., ClipboardPlacement::Paste)?;
        let mut preview = scratch.clone();
        for operation in create_operations(&pasted) {
            preview.apply(operation)?;
        }
        let source = pasted
            .roots
            .iter()
            .filter_map(|id| preview.scene.world_bounds(*id))
            .reduce(|a, b| a.union(&b))
            .context("The clipboard has no geometry")?;
        let offset =
            Transform2D::translation(bounds.min_x - source.min_x, bounds.min_y - source.min_y);
        let parent = target
            .parent
            .and_then(|id| scratch.scene.world_transform(id))
            .unwrap_or(Transform2D::IDENTITY);
        let next = scratch
            .scene
            .children_of(target.parent)
            .iter()
            .filter_map(|id| scratch.scene.get(*id))
            .find(|node| node.index > target.index)
            .map(|node| node.index);
        let mut index = target.index;
        for node in &mut pasted.nodes {
            if !pasted.roots.contains(&node.id) {
                continue;
            }
            node.transform = preview
                .scene
                .world_transform(node.id)
                .context("Missing clipboard transform")?
                .then(&offset)
                .then(&parent.inverse());
            anyhow::ensure!(
                node.transform.is_finite(),
                "The target parent transform is singular"
            );
            node.parent = target.parent;
            node.index = index;
            index = if let Some(next) = next {
                anyhow::ensure!(
                    !IndexKey::near_precision_limit(index, next),
                    "The stacking order needs rebalancing"
                );
                IndexKey::between(index, next)
            } else {
                IndexKey::after(index)
            };
        }
        let mut edits = crate::layer_context_ops::delete_layers(&scratch, id)?;
        edits.extend(clone_component_operations(&scratch, payload, &mut pasted)?);
        for operation in &edits {
            scratch.apply(operation.clone())?;
        }
        operations.extend(edits);
    }
    Ok(operations)
}

pub(crate) fn clone_component_operations(
    doc: &Doc,
    clipboard: &CanvasClipboard,
    pasted: &mut PastedNodes,
) -> Result<Vec<Operation>> {
    let nodes: HashMap<_, _> = clipboard
        .nodes
        .iter()
        .zip(&pasted.nodes)
        .map(|(source, copy)| (source.id, copy.id))
        .collect();
    let components: HashMap<_, _> = doc
        .components
        .defs
        .values()
        .filter(|definition| nodes.contains_key(&definition.root))
        .map(|definition| (definition.id, fanta_doc::ComponentId::new()))
        .collect();
    let sets: HashMap<_, _> = doc
        .components
        .sets
        .values()
        .filter(|set| {
            !set.members.is_empty()
                && set
                    .members
                    .iter()
                    .all(|member| components.contains_key(member))
        })
        .map(|set| (set.id, fanta_doc::ComponentId::new()))
        .collect();
    let copied_component = |component| {
        components
            .get(&component)
            .or_else(|| sets.get(&component))
            .copied()
    };
    for node in &mut pasted.nodes {
        if let NodeData::Instance(instance) = &mut node.data {
            let copied_master = copied_component(instance.component);
            if let Some(component) = copied_master {
                instance.component = component;
                for derived in &mut instance.derived {
                    for node in &mut derived.path {
                        remap_reference(node, &nodes);
                    }
                }
            }
            for replacement in &mut instance.overrides {
                if copied_master.is_some() {
                    for node in &mut replacement.target_path {
                        remap_reference(node, &nodes);
                    }
                }
                if let fanta_doc::OverrideValue::SwapInstance { component } = &mut replacement.value
                    && let Some(copy) = copied_component(*component)
                {
                    *component = copy;
                }
            }
        }
        for reaction in &mut node.reactions {
            for action in std::iter::once(&mut reaction.action).chain(&mut reaction.extra_actions) {
                if let Action::UpdateVariant { component, .. } = action
                    && let Some(copy) = copied_component(*component)
                {
                    *component = copy;
                }
            }
        }
    }
    let mut operations = create_operations(&pasted);
    for definition in doc.components.defs.values() {
        let Some(id) = components.get(&definition.id) else {
            continue;
        };
        let mut copy = definition.clone();
        copy.id = *id;
        copy.root = *nodes
            .get(&definition.root)
            .context("missing copied component root")?;
        copy.variant_of = definition.variant_of.as_ref().and_then(|membership| {
            sets.get(&membership.set).map(|set| {
                let mut copy = membership.clone();
                copy.set = *set;
                copy
            })
        });
        for property in &mut copy.props {
            for binding in &mut property.bindings {
                for node in &mut binding.path {
                    remap_reference(node, &nodes);
                }
            }
        }
        operations.push(Operation::DefineComponent {
            def: Box::new(copy),
        });
    }
    for set in doc.components.sets.values() {
        let Some(id) = sets.get(&set.id) else {
            continue;
        };
        let mut copy = set.clone();
        copy.id = *id;
        copy.members = set
            .members
            .iter()
            .map(|member| components[member])
            .collect();
        copy.default_variant = *components
            .get(&set.default_variant)
            .context("missing copied default variant")?;
        // The copied set's frame is the copy of its frame, when that was
        // copied too; otherwise the copied variants have no frame.
        copy.root = set.root.and_then(|root| nodes.get(&root).copied());
        operations.push(Operation::DefineComponentSet {
            set: Box::new(copy),
        });
    }
    Ok(operations)
}

#[cfg(feature = "fanta-gpui-ui")]
pub(crate) fn duplicate_layer_operations(doc: &Doc, id: NodeId) -> Result<Vec<Operation>> {
    let targets = crate::layer_context_ops::targets(doc, id);
    for target in &targets {
        anyhow::ensure!(
            crate::layer_context_ops::editable(doc, *target),
            "A selected layer is locked"
        );
    }
    let clipboard = CanvasClipboard::capture_roots(doc, targets).context("Nothing to duplicate")?;
    let mut pasted = clipboard.instantiate(doc, 16., ClipboardPlacement::Duplicate)?;
    clone_component_operations(doc, &clipboard, &mut pasted)
}
