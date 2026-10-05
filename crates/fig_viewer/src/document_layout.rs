use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use fanta_doc::{
    BoundProp, CanvasNode, Color, Doc, ModeId, NodeData, NodeFlags, NodeId, Operation, Scene,
    TextAutoResize, TextNode, VariableCollectionId, VariableRegistry,
};

pub(super) struct LayoutState {
    scene: Scene,
    source_instance: u64,
    revision: u64,
    variables: VariableRegistry,
    active_modes: BTreeMap<VariableCollectionId, ModeId>,
    pub(super) history_generation: u64,
}

impl LayoutState {
    pub(super) fn new(document: &Doc) -> Self {
        Self {
            scene: document.scene.clone(),
            source_instance: document.scene.instance_id(),
            revision: document.scene.revision(),
            variables: document.variables.clone(),
            active_modes: document.active_modes.clone(),
            history_generation: document.history.edit_generation(),
        }
    }

    pub(super) fn changed_roots(&self, document: &Doc) -> BTreeSet<NodeId> {
        let mut candidates = BTreeSet::new();
        if self.source_instance == document.scene.instance_id()
            && let Some(delta) = document.scene.changes_since(self.revision)
        {
            candidates.extend(delta.nodes);
            candidates.extend(delta.transforms);
        } else {
            for scene in [&self.scene, &document.scene] {
                for root in scene.roots() {
                    candidates.extend(scene.descendants_of(*root));
                }
            }
        }
        let modes_changed = self.variables != document.variables
            || self.active_modes != document.active_modes
            || candidates
                .iter()
                .any(|id| match (self.scene.get(*id), document.scene.get(*id)) {
                    (Some(old), Some(new)) => match (&old.data, &new.data) {
                        (NodeData::Group(old), NodeData::Group(new)) => {
                            old.explicit_modes != new.explicit_modes
                        }
                        _ => false,
                    },
                    _ => false,
                });
        if modes_changed {
            for root in document.scene.roots() {
                candidates.extend(document.scene.descendants_of(*root).filter(|id| {
                    document.scene.get(*id).is_some_and(|node| {
                        node.bindings.keys().any(|property| {
                            matches!(property, BoundProp::TextStyle | BoundProp::TextContent)
                        })
                    })
                }));
            }
        }
        let mut roots = BTreeSet::new();
        for id in candidates {
            if !modes_changed && self.scene.shares_node(&document.scene, id) {
                continue;
            }
            let old = self.scene.get(id);
            let new = document.scene.get(id);
            let structural = match (old, new) {
                (Some(old), Some(new)) => {
                    old.parent != new.parent
                        || old.index != new.index
                        || old.layout_child != new.layout_child
                        || old.flags.contains(NodeFlags::HIDDEN)
                            != new.flags.contains(NodeFlags::HIDDEN)
                }
                _ => true,
            };
            let geometry_changed = match (old, new) {
                (Some(old), Some(new)) => !layout_data_equal(
                    old,
                    new,
                    &self.scene,
                    &document.scene,
                    &self.variables,
                    &document.variables,
                    &self.active_modes,
                    &document.active_modes,
                ),
                _ => true,
            };
            let transform_changed =
                old.map(|node| node.transform) != new.map(|node| node.transform);
            if structural || geometry_changed {
                if let Some(node) = new {
                    add_own_root(node, &mut roots);
                }
            }
            if structural || geometry_changed || transform_changed {
                for (scene, node) in [(&self.scene, old), (&document.scene, new)] {
                    if let Some(node) = node {
                        add_parent_roots(scene, node, &mut roots);
                    }
                }
            }
        }
        roots.retain(|root| document.scene.contains(*root));
        minimal_roots(&document.scene, roots)
    }

    pub(super) fn refresh(&mut self, document: &Doc) -> Result<()> {
        if self.source_instance == document.scene.instance_id()
            && let Some(delta) = document.scene.changes_since(self.revision)
        {
            for id in delta.nodes.into_iter().chain(delta.transforms) {
                if let Some(node) = document.scene.get(id) {
                    self.scene
                        .patch_node(node.clone(), document.scene.node_stamp(id))?;
                }
            }
        } else {
            self.scene = document.scene.clone();
        }
        self.history_generation = document.history.edit_generation();
        self.source_instance = document.scene.instance_id();
        self.revision = document.scene.revision();
        if self.variables != document.variables {
            self.variables = document.variables.clone();
        }
        if self.active_modes != document.active_modes {
            self.active_modes = document.active_modes.clone();
        }
        Ok(())
    }
}

fn is_auto_layout(node: &CanvasNode) -> bool {
    node.data
        .as_group()
        .is_some_and(|group| group.auto_layout.is_some())
}

fn add_own_root(node: &CanvasNode, roots: &mut BTreeSet<NodeId>) {
    if is_auto_layout(node)
        || matches!(&node.data, NodeData::Text(text) if text.auto_resize != TextAutoResize::None)
    {
        roots.insert(node.id);
    }
}

fn add_parent_roots(scene: &Scene, node: &CanvasNode, roots: &mut BTreeSet<NodeId>) {
    let mut child = node;
    let mut parent = child.parent;
    while let Some(id) = parent {
        let Some(node) = scene.get(id) else { break };
        let Some(layout) = node.data.as_group().and_then(|group| group.auto_layout) else {
            break;
        };
        if layout.child_layout && child.layout_child.is_some_and(|layout| layout.absolute) {
            break;
        }
        roots.insert(id);
        if layout.primary_sizing == fanta_doc::AxisSizing::Fixed
            && layout.counter_sizing == fanta_doc::AxisSizing::Fixed
        {
            break;
        }
        child = node;
        parent = node.parent;
    }
}

fn minimal_roots(scene: &Scene, roots: BTreeSet<NodeId>) -> BTreeSet<NodeId> {
    roots
        .iter()
        .copied()
        .filter(|root| {
            !scene
                .ancestors_of(*root)
                .any(|ancestor| roots.contains(&ancestor.id))
        })
        .collect()
}

pub(super) fn pending_roots(document: &Doc) -> BTreeSet<NodeId> {
    let mut roots = BTreeSet::new();
    for id in &document.pending_layout {
        if let Some(node) = document.scene.get(*id) {
            add_own_root(node, &mut roots);
            add_parent_roots(&document.scene, node, &mut roots);
        }
    }
    minimal_roots(&document.scene, roots)
}

fn effective_text(
    node: &CanvasNode,
    scene: &Scene,
    variables: &VariableRegistry,
    active_modes: &BTreeMap<VariableCollectionId, ModeId>,
) -> Option<TextNode> {
    let NodeData::Text(text) = &node.data else {
        return None;
    };
    let mut resolved = node.clone();
    for (property, variable) in &node.bindings {
        if matches!(property, BoundProp::TextStyle | BoundProp::TextContent)
            && let Some(value) =
                fanta_doc::resolve_bound_value(variables, scene, node.id, active_modes, *variable)
        {
            property.apply_resolved(&mut resolved, value);
        }
    }
    let NodeData::Text(mut resolved) = resolved.data else {
        return Some(text.clone());
    };
    for style in std::iter::once(&mut resolved.style)
        .chain(resolved.style_runs.iter_mut().map(|run| &mut run.style))
    {
        style.color = Color::BLACK;
        style.underline = false;
        style.strikethrough = false;
    }
    resolved.vertical_align = fanta_doc::VAlign::Top;
    Some(resolved)
}

#[expect(
    clippy::too_many_arguments,
    reason = "compare effective layout inputs against both authoritative document states"
)]
fn layout_data_equal(
    old: &CanvasNode,
    new: &CanvasNode,
    old_scene: &Scene,
    new_scene: &Scene,
    old_variables: &VariableRegistry,
    new_variables: &VariableRegistry,
    old_modes: &BTreeMap<VariableCollectionId, ModeId>,
    new_modes: &BTreeMap<VariableCollectionId, ModeId>,
) -> bool {
    match (&old.data, &new.data) {
        (NodeData::Group(old), NodeData::Group(new)) => {
            old.clip_size == new.clip_size
                && old.local_size == new.local_size
                && old.auto_layout == new.auto_layout
                && old.grid == new.grid
                && (!(old.auto_layout.is_some_and(|layout| layout.include_strokes)
                    || new.auto_layout.is_some_and(|layout| layout.include_strokes))
                    || stroke_layout_inputs(&old.strokes) == stroke_layout_inputs(&new.strokes))
        }
        (NodeData::Text(_), NodeData::Text(_)) => {
            effective_text(old, old_scene, old_variables, old_modes)
                == effective_text(new, new_scene, new_variables, new_modes)
        }
        (old, new) => {
            std::mem::discriminant(old) == std::mem::discriminant(new)
                && old.local_bounds() == new.local_bounds()
        }
    }
}

fn stroke_layout_inputs(strokes: &[fanta_doc::Stroke]) -> Vec<([f64; 4], fanta_doc::StrokeAlign)> {
    strokes
        .iter()
        .map(|stroke| (stroke.per_side.unwrap_or([stroke.width; 4]), stroke.align))
        .collect()
}

pub(super) fn geometry_operations(document: &Doc, roots: &BTreeSet<NodeId>) -> Vec<Operation> {
    let mut operations = Vec::new();
    for root in roots {
        let Some(mut solved) = document.scene.extract_subtree(*root) else {
            continue;
        };
        let ids: Vec<_> = solved.descendants_of(*root).collect();
        for id in &document.pending_layout {
            if solved.get(*id).is_some_and(|node| {
                node.data.as_group().is_some_and(|group| {
                    group.auto_layout.is_some()
                        && group.clip_size.is_none()
                        && group.local_size.is_none()
                })
            }) && let Some(node) = solved.get_mut(*id)
                && let NodeData::Group(group) = &mut node.data
            {
                group.local_size = Some([0.0, 0.0]);
            }
        }
        for id in &ids {
            if let Some(original) = document.scene.get(*id)
                && let Some(text) = effective_text(
                    original,
                    &document.scene,
                    &document.variables,
                    &document.active_modes,
                )
                && let Some(node) = solved.get_mut(*id)
            {
                node.data = NodeData::Text(text);
            }
        }
        fanta_render::solve_scene_layout(&mut solved, *root);
        for id in ids {
            let (Some(original), Some(solved)) = (document.scene.get(id), solved.get(id)) else {
                continue;
            };
            if original.transform != solved.transform {
                operations.push(Operation::SetTransform {
                    id,
                    old: original.transform,
                    new: solved.transform,
                });
            }
            let mut data = original.data.clone();
            match (&mut data, &solved.data) {
                (NodeData::Group(original), NodeData::Group(solved)) => {
                    original.clip_size = solved.clip_size;
                    original.local_size = solved.local_size;
                }
                (NodeData::Vector(original), NodeData::Vector(solved)) => {
                    original.path = solved.path.clone()
                }
                (original, solved) => {
                    if let Some([width, height]) = solved.local_size() {
                        original.set_local_size(width, height);
                    }
                }
            }
            if original.data != data {
                operations.push(Operation::ReplaceData {
                    id,
                    old: Box::new(original.data.clone()),
                    new: Box::new(data),
                });
            }
        }
    }
    operations
}

pub(super) fn apply_without_history(
    document: &mut Doc,
    operations: Vec<Operation>,
) -> Result<BTreeSet<NodeId>> {
    let mut changed = BTreeSet::new();
    for operation in operations {
        match operation {
            Operation::SetTransform { id, new, .. } => {
                document.scene.set_transform(id, new)?;
                changed.insert(id);
            }
            Operation::ReplaceData { id, new, .. } => {
                if let Some(node) = document.scene.get_mut(id) {
                    node.data = *new;
                    changed.insert(id);
                }
            }
            _ => anyhow::bail!("layout produced a non-geometry operation"),
        }
    }
    Ok(changed)
}
