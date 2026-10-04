//! Variant sets the way Figma has them: one frame holding every variant's
//! master, laid out as a grid of their property values, with each variant
//! named after those values (`"Variant=Primary, State=Hover"`).
//!
//! Each edit is planned against a history-free copy of the document and
//! returned as the operations that make it, so the agent ops and the editor's
//! own commands produce the same structure and one undo step.

use std::collections::BTreeMap;

use anyhow::{Context as _, Result, bail};
use fanta_doc::{
    AutoLayout, AxisSizing, CanvasNode, Color, ComponentId, ComponentSet, ComponentSetMembership,
    Doc, Fill, GridCell, GridLayout, GridTrack, GroupNode, LayoutChild, LayoutMode, NodeData,
    NodeId, Operation, Stroke, StrokeAlign, Transform2D, VariantAxis, parse_variant_name,
    variant_name,
};

/// Figma's component-set border.
const SET_BORDER: Color = Color::rgba(0x97, 0x47, 0xFF, 0xFF);
const SET_PADDING: f64 = 20.0;
const SET_GAP: f64 = 20.0;
/// The value a member takes on an axis its name doesn't mention.
const DEFAULT_VALUE: &str = "Default";
/// The single axis of a set whose members aren't named `Axis=Value`.
const PLAIN_AXIS: &str = "Variant";

/// Operations applied to a scratch copy as they are planned, so each step
/// sees the result of the previous ones.
struct Plan {
    doc: Doc,
    operations: Vec<Operation>,
}

impl Plan {
    fn new(doc: &Doc) -> Self {
        Self {
            doc: doc.clone_for_persist(),
            operations: Vec::new(),
        }
    }

    fn apply(&mut self, operation: Operation) -> Result<()> {
        self.doc
            .apply(operation.clone())
            .context("planning the variant set")?;
        self.operations.push(operation);
        Ok(())
    }

    fn apply_all(&mut self, operations: Vec<Operation>) -> Result<()> {
        operations
            .into_iter()
            .try_for_each(|operation| self.apply(operation))
    }
}

/// The operations of a variant-set edit, plus what it made.
pub(crate) struct SetEdit {
    pub(crate) operations: Vec<Operation>,
    pub(crate) set: ComponentId,
    pub(crate) frame: Option<NodeId>,
}

/// Combine standalone component masters (by root node) into a new variant
/// set inside one frame. Members named `Axis=Value, …` become a set with those
/// axes; otherwise each name is a value of one `Variant` axis.
pub(crate) fn combine_variants(doc: &Doc, roots: &[NodeId], name: Option<&str>) -> Result<SetEdit> {
    let members = standalone_masters(doc, roots)?;
    if members.len() < 2 {
        bail!("combine_variants needs at least two standalone component masters");
    }
    let names: Vec<String> = members
        .iter()
        .map(|member| def_name(doc, *member))
        .collect();
    let parsed: Option<Vec<_>> = names.iter().map(|name| parse_variant_name(name)).collect();
    let (axes, values) = match parsed {
        Some(parsed) => axes_from_names(&[], &parsed),
        None => {
            let values = names
                .iter()
                .map(|name| BTreeMap::from([(PLAIN_AXIS.to_owned(), name.clone())]))
                .collect();
            let axis = VariantAxis {
                name: PLAIN_AXIS.to_owned(),
                values: names.clone(),
            };
            (vec![axis], values)
        }
    };
    ensure_distinct(&axes, &values)?;
    let set_name = match name {
        Some(name) => name.trim().to_owned(),
        None if axes.len() == 1 && axes[0].name == PLAIN_AXIS => names[0].clone(),
        None => "Component set".to_owned(),
    };
    if set_name.is_empty() {
        bail!("the set needs a name");
    }

    let mut plan = Plan::new(doc);
    let set = ComponentSet {
        id: ComponentId::new(),
        name: set_name,
        axes,
        members: members.clone(),
        default_variant: members[0],
        root: None,
    };
    let set_id = set.id;
    plan.apply(Operation::DefineComponentSet { set: Box::new(set) })?;
    for (member, axis_values) in members.iter().zip(values) {
        set_membership(&mut plan, *member, set_id, axis_values)?;
    }
    let frame = arrange_in_plan(&mut plan, set_id)?;
    Ok(SetEdit {
        operations: plan.operations,
        set: set_id,
        frame: Some(frame),
    })
}

/// Add standalone masters (by root node) named `Axis=Value, …` to an existing
/// set. New axes and values extend the set; a member without a value on an
/// axis takes `Default`. The set's frame is re-laid out around them.
pub(crate) fn add_variants(doc: &Doc, set_id: ComponentId, roots: &[NodeId]) -> Result<SetEdit> {
    let set = doc
        .components
        .sets
        .get(&set_id)
        .context("no such variant set")?
        .clone();
    let added = standalone_masters(doc, roots)?;
    if added.is_empty() {
        bail!("add_variants needs at least one standalone component master");
    }
    let parsed = added
        .iter()
        .map(|member| {
            let name = def_name(doc, *member);
            parse_variant_name(&name).with_context(|| {
                format!("name the master \"{name}\" after its values, like \"Variant=Primary, State=Hover\"")
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let existing: Vec<BTreeMap<String, String>> = set
        .members
        .iter()
        .map(|member| membership_values(doc, *member))
        .collect();
    let (axes, added_values) = axes_from_names(&set.axes, &parsed);
    // Existing members take the default on any axis the new ones introduced.
    let existing_values: Vec<_> = existing
        .into_iter()
        .map(|mut values| {
            for axis in &axes {
                values
                    .entry(axis.name.clone())
                    .or_insert_with(|| DEFAULT_VALUE.to_owned());
            }
            values
        })
        .collect();
    let axes = with_default_values(axes, &existing_values);
    let all_values: Vec<_> = existing_values
        .iter()
        .chain(&added_values)
        .cloned()
        .collect();
    ensure_distinct(&axes, &all_values)?;

    let mut plan = Plan::new(doc);
    let mut members = set.members.clone();
    members.extend(&added);
    let new_set = ComponentSet {
        axes,
        members: members.clone(),
        ..set.clone()
    };
    plan.apply(Operation::SetComponentSet {
        id: set_id,
        old: Box::new(set),
        new: Box::new(new_set),
    })?;
    for (member, values) in members.iter().zip(all_values) {
        set_membership(&mut plan, *member, set_id, values)?;
    }
    let frame = arrange_in_plan(&mut plan, set_id)?;
    Ok(SetEdit {
        operations: plan.operations,
        set: set_id,
        frame: Some(frame),
    })
}

/// Take one variant out of its set. It stays a standalone component, placed
/// beside the set's frame. Unused axis values are dropped, and a set left
/// empty is deleted with its frame.
pub(crate) fn remove_variant(doc: &Doc, member: ComponentId) -> Result<SetEdit> {
    let def = doc
        .components
        .def(member)
        .context("no such component")?
        .clone();
    let membership = def
        .variant_of
        .clone()
        .context("the component is not in a variant set")?;
    let set = doc
        .components
        .sets
        .get(&membership.set)
        .context("the component's variant set is missing")?
        .clone();

    let mut plan = Plan::new(doc);
    plan.apply(Operation::SetVariantMembership {
        id: member,
        old: Some(membership),
        new: None,
    })?;
    if let Some(frame) = set.root.filter(|frame| plan.doc.scene.contains(*frame)) {
        move_out_of_frame(&mut plan, def.root, frame)?;
    }
    let members: Vec<ComponentId> = set
        .members
        .iter()
        .copied()
        .filter(|id| *id != member)
        .collect();
    if members.is_empty() {
        plan.apply(Operation::DeleteComponentSet {
            id: set.id,
            set: Box::new(set.clone()),
        })?;
        if let Some(frame) = set.root.filter(|frame| plan.doc.scene.contains(*frame))
            && plan.doc.scene.children_of(Some(frame)).is_empty()
        {
            let snapshot = vec![plan.doc.scene.get(frame).context("set frame")?.clone()];
            plan.apply(Operation::DeleteSubtree { snapshot })?;
        }
        return Ok(SetEdit {
            operations: plan.operations,
            set: set.id,
            frame: None,
        });
    }
    let remaining: Vec<_> = members
        .iter()
        .map(|id| membership_values(&plan.doc, *id))
        .collect();
    let axes = set
        .axes
        .iter()
        .map(|axis| VariantAxis {
            name: axis.name.clone(),
            values: axis
                .values
                .iter()
                .filter(|value| {
                    remaining
                        .iter()
                        .any(|values| values.get(&axis.name) == Some(value))
                })
                .cloned()
                .collect(),
        })
        .filter(|axis| !axis.values.is_empty())
        .collect();
    let new_set = ComponentSet {
        axes,
        default_variant: if set.default_variant == member {
            members[0]
        } else {
            set.default_variant
        },
        members,
        ..set.clone()
    };
    plan.apply(Operation::SetComponentSet {
        id: set.id,
        old: Box::new(set.clone()),
        new: Box::new(new_set),
    })?;
    let frame = arrange_in_plan(&mut plan, set.id)?;
    Ok(SetEdit {
        operations: plan.operations,
        set: set.id,
        frame: Some(frame),
    })
}

/// Put every member of `set_id` in the set's frame (making one if it has
/// none), name each after its values, and lay them out as a grid: one column
/// per value of the last axis, one row per combination of the others.
pub(crate) fn arrange_variants(doc: &Doc, set_id: ComponentId) -> Result<SetEdit> {
    let mut plan = Plan::new(doc);
    let frame = arrange_in_plan(&mut plan, set_id)?;
    Ok(SetEdit {
        operations: plan.operations,
        set: set_id,
        frame: Some(frame),
    })
}

/// Rename a component or a variant set, keeping its master (or frame) layer
/// name in step. Names must stay unique: pages name components.
pub(crate) fn rename_component(doc: &Doc, id: ComponentId, name: &str) -> Result<Vec<Operation>> {
    let name = name.trim();
    if name.is_empty() {
        bail!("the name is empty");
    }
    let taken = doc
        .components
        .defs
        .values()
        .filter(|def| def.id != id)
        .map(|def| def.name.as_str())
        .chain(
            doc.components
                .sets
                .values()
                .filter(|set| set.id != id)
                .map(|set| set.name.as_str()),
        )
        .any(|other| other == name);
    if taken {
        bail!("another component is already named \"{name}\"");
    }
    let mut operations = Vec::new();
    let layer = if let Some(def) = doc.components.def(id) {
        operations.push(Operation::SetComponentName {
            id,
            old: def.name.clone(),
            new: name.to_owned(),
        });
        Some(def.root)
    } else if let Some(set) = doc.components.sets.get(&id) {
        operations.push(Operation::SetComponentSet {
            id,
            old: Box::new(set.clone()),
            new: Box::new(ComponentSet {
                name: name.to_owned(),
                ..set.clone()
            }),
        });
        set.root
    } else {
        bail!("no such component");
    };
    if let Some(node) = layer.and_then(|layer| doc.scene.get(layer))
        && node.name != name
    {
        operations.push(Operation::SetName {
            id: node.id,
            old: node.name.clone(),
            new: name.to_owned(),
        });
    }
    Ok(operations)
}

fn arrange_in_plan(plan: &mut Plan, set_id: ComponentId) -> Result<NodeId> {
    let set = plan
        .doc
        .components
        .sets
        .get(&set_id)
        .context("no such variant set")?
        .clone();
    let members: Vec<(ComponentId, NodeId, BTreeMap<String, String>)> = set
        .members
        .iter()
        .filter_map(|member| {
            let def = plan.doc.components.def(*member)?;
            Some((*member, def.root, membership_values(&plan.doc, *member)))
        })
        .collect();
    if members.is_empty() {
        bail!("the variant set has no members");
    }

    // A variant named after its values ("Variant=Primary") is renamed to all
    // of them in the set's axis order ("Variant=Primary, State=Default").
    // Plain names ("Default") are the user's and stay.
    for (member, root, values) in &members {
        let name = def_name(&plan.doc, *member);
        if parse_variant_name(&name).is_none() {
            continue;
        }
        let canonical = variant_name(&set.axes, values);
        if !canonical.is_empty() && name != canonical {
            let operations = rename_component(&plan.doc, *member, &canonical)?;
            plan.apply_all(operations)?;
        } else if let Some(node) = plan.doc.scene.get(*root)
            && !canonical.is_empty()
            && node.name != canonical
        {
            let operation = Operation::SetName {
                id: *root,
                old: node.name.clone(),
                new: canonical,
            };
            plan.apply(operation)?;
        }
    }

    let (columns, cells) = grid_cells(&set.axes, members.iter().map(|(_, _, values)| values));
    let frame = match set.root.filter(|frame| plan.doc.scene.contains(*frame)) {
        Some(frame) => frame,
        None => {
            let frame = create_set_frame(plan, &set, members.iter().map(|(_, root, _)| *root))?;
            let new_set = ComponentSet {
                root: Some(frame),
                ..set.clone()
            };
            plan.apply(Operation::SetComponentSet {
                id: set_id,
                old: Box::new(set.clone()),
                new: Box::new(new_set),
            })?;
            frame
        }
    };
    let grid = GridLayout {
        columns: vec![GridTrack::Hug; columns],
        rows: Vec::new(),
        column_gap: SET_GAP,
        row_gap: SET_GAP,
    };
    let operations = crate::properties_ops::replace_data_operation(&plan.doc, frame, |data| {
        if let NodeData::Group(group) = data {
            let layout = group.auto_layout.get_or_insert_with(AutoLayout::default);
            layout.mode = LayoutMode::Grid;
            layout.primary_sizing = AxisSizing::Hug;
            layout.counter_sizing = AxisSizing::Hug;
            layout.padding = [SET_PADDING; 4];
            group.grid = Some(grid);
        }
    });
    plan.apply_all(operations)?;

    for ((_, root, _), cell) in members.iter().zip(cells) {
        let node = plan.doc.scene.get(*root).context("variant master")?.clone();
        if node.parent != Some(frame) {
            let index = plan.doc.scene.next_child_index(Some(frame));
            plan.apply(Operation::Reparent {
                id: *root,
                old_parent: node.parent,
                old_index: node.index,
                new_parent: Some(frame),
                new_index: index,
            })?;
        }
        let node = plan.doc.scene.get(*root).context("variant master")?.clone();
        let mut child = node.layout_child.unwrap_or_default();
        child.absolute = false;
        child.grid = Some(cell);
        if node.layout_child != Some(child) {
            plan.apply(Operation::SetLayoutChild {
                id: *root,
                old: node.layout_child,
                new: Some(child),
            })?;
        }
    }
    Ok(frame)
}

/// A new frame for `set`, in the first member's parent, at the top-left of
/// the members' bounds.
fn create_set_frame(
    plan: &mut Plan,
    set: &ComponentSet,
    roots: impl Iterator<Item = NodeId>,
) -> Result<NodeId> {
    let roots: Vec<NodeId> = roots.collect();
    let parent = plan
        .doc
        .scene
        .get(roots[0])
        .context("variant master")?
        .parent;
    let origin = roots
        .iter()
        .filter_map(|root| plan.doc.scene.world_bounds(*root))
        .fold(None::<[f64; 2]>, |origin, bounds| {
            Some(match origin {
                Some([x, y]) => [x.min(bounds.min_x), y.min(bounds.min_y)],
                None => [bounds.min_x, bounds.min_y],
            })
        })
        .unwrap_or_default();
    let parent_world = parent
        .and_then(|parent| plan.doc.scene.world_transform(parent))
        .unwrap_or(Transform2D::IDENTITY);
    let mut stroke = Stroke::solid(SET_BORDER, 1.0);
    stroke.dash = vec![10.0, 5.0];
    stroke.align = StrokeAlign::Inside;
    let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([100.0, 100.0]),
        corner_radius: Some(5.0),
        strokes: [stroke].into_iter().collect(),
        background: None::<Fill>,
        ..Default::default()
    }));
    frame.name = set.name.clone();
    frame.parent = parent;
    frame.index = plan.doc.scene.next_child_index(parent);
    frame.transform = Transform2D::translation(origin[0] - SET_PADDING, origin[1] - SET_PADDING)
        .then(&parent_world.inverse());
    let id = frame.id;
    plan.apply(Operation::create_node(frame))?;
    Ok(id)
}

/// Move a former member out of the set frame, beside it, keeping its look.
fn move_out_of_frame(plan: &mut Plan, root: NodeId, frame: NodeId) -> Result<()> {
    let node = plan.doc.scene.get(root).context("variant master")?.clone();
    if node.parent != Some(frame) {
        return Ok(());
    }
    let frame_node = plan.doc.scene.get(frame).context("set frame")?.clone();
    let bounds = plan.doc.scene.world_bounds(frame);
    let new_parent = frame_node.parent;
    let index = plan.doc.scene.next_child_index(new_parent);
    plan.apply(Operation::Reparent {
        id: root,
        old_parent: node.parent,
        old_index: node.index,
        new_parent,
        new_index: index,
    })?;
    if let Some(bounds) = bounds {
        let parent_world = new_parent
            .and_then(|parent| plan.doc.scene.world_transform(parent))
            .unwrap_or(Transform2D::IDENTITY);
        let world = Transform2D::translation(bounds.max_x + 40.0, bounds.min_y);
        let current = plan
            .doc
            .scene
            .get(root)
            .context("variant master")?
            .transform;
        plan.apply(Operation::SetTransform {
            id: root,
            old: current,
            new: world.then(&parent_world.inverse()),
        })?;
    }
    if let Some(child) = node.layout_child.filter(|child| child.grid.is_some()) {
        let cleared = LayoutChild {
            grid: None,
            ..child
        };
        plan.apply(Operation::SetLayoutChild {
            id: root,
            old: Some(child),
            new: (!cleared.is_trivial()).then_some(cleared),
        })?;
    }
    Ok(())
}

/// One column per value of the last axis, one row per combination of the
/// others (in axis order, the first axis varying slowest).
fn grid_cells<'a>(
    axes: &[VariantAxis],
    members: impl Iterator<Item = &'a BTreeMap<String, String>>,
) -> (usize, Vec<GridCell>) {
    let Some((last, rest)) = axes.split_last() else {
        let cells: Vec<GridCell> = members
            .enumerate()
            .map(|(index, _)| GridCell {
                column: index as u16,
                ..Default::default()
            })
            .collect();
        return (cells.len().max(1), cells);
    };
    let position = |axis: &VariantAxis, values: &BTreeMap<String, String>| {
        values
            .get(&axis.name)
            .and_then(|value| axis.values.iter().position(|known| known == value))
            .unwrap_or(0)
    };
    let cells = members
        .map(|values| {
            let row = rest.iter().fold(0usize, |row, axis| {
                row * axis.values.len().max(1) + position(axis, values)
            });
            GridCell {
                column: position(last, values) as u16,
                row: row as u16,
                ..Default::default()
            }
        })
        .collect();
    (last.values.len().max(1), cells)
}

fn set_membership(
    plan: &mut Plan,
    member: ComponentId,
    set: ComponentId,
    axis_values: BTreeMap<String, String>,
) -> Result<()> {
    let old = plan
        .doc
        .components
        .def(member)
        .context("variant member")?
        .variant_of
        .clone();
    let new = Some(ComponentSetMembership { set, axis_values });
    if old != new {
        plan.apply(Operation::SetVariantMembership {
            id: member,
            old,
            new,
        })?;
    }
    Ok(())
}

/// Axes in first-seen order (after `existing`), each with its values in
/// first-seen order, and each member's value per axis (`Default` where its
/// name skips one).
fn axes_from_names(
    existing: &[VariantAxis],
    names: &[Vec<(String, String)>],
) -> (Vec<VariantAxis>, Vec<BTreeMap<String, String>>) {
    let mut axes = existing.to_vec();
    for pairs in names {
        for (axis, value) in pairs {
            let index = match axes.iter().position(|known| &known.name == axis) {
                Some(index) => index,
                None => {
                    axes.push(VariantAxis {
                        name: axis.clone(),
                        values: Vec::new(),
                    });
                    axes.len() - 1
                }
            };
            if !axes[index].values.contains(value) {
                axes[index].values.push(value.clone());
            }
        }
    }
    let values: Vec<BTreeMap<String, String>> = names
        .iter()
        .map(|pairs| {
            axes.iter()
                .map(|axis| {
                    let value = pairs
                        .iter()
                        .find(|(name, _)| name == &axis.name)
                        .map_or(DEFAULT_VALUE, |(_, value)| value.as_str());
                    (axis.name.clone(), value.to_owned())
                })
                .collect()
        })
        .collect();
    (with_default_values(axes, &values), values)
}

/// Add `Default` to an axis's values where some member takes it.
fn with_default_values(
    mut axes: Vec<VariantAxis>,
    members: &[BTreeMap<String, String>],
) -> Vec<VariantAxis> {
    for axis in &mut axes {
        let uses_default = members
            .iter()
            .any(|values| values.get(&axis.name).map(String::as_str) == Some(DEFAULT_VALUE));
        if uses_default && !axis.values.iter().any(|value| value == DEFAULT_VALUE) {
            axis.values.insert(0, DEFAULT_VALUE.to_owned());
        }
    }
    axes
}

fn ensure_distinct(axes: &[VariantAxis], members: &[BTreeMap<String, String>]) -> Result<()> {
    for (index, values) in members.iter().enumerate() {
        if members[..index].contains(values) {
            bail!(
                "two variants would both be \"{}\"; give each a different combination of values",
                variant_name(axes, values)
            );
        }
    }
    Ok(())
}

/// The standalone component masters behind `roots`, in order.
fn standalone_masters(doc: &Doc, roots: &[NodeId]) -> Result<Vec<ComponentId>> {
    roots
        .iter()
        .map(|root| {
            let def = doc
                .components
                .defs
                .values()
                .find(|def| def.root == *root)
                .with_context(|| format!("node {root} is not a component master"))?;
            if def.variant_of.is_some() {
                bail!("\"{}\" is already in a variant set", def.name);
            }
            Ok(def.id)
        })
        .collect()
}

fn def_name(doc: &Doc, component: ComponentId) -> String {
    doc.components
        .def(component)
        .map(|def| def.name.clone())
        .unwrap_or_default()
}

fn membership_values(doc: &Doc, component: ComponentId) -> BTreeMap<String, String> {
    doc.components
        .def(component)
        .and_then(|def| def.variant_of.as_ref())
        .map(|membership| membership.axis_values.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{ComponentDef, solve_auto_layout};

    /// A page with one 100×40 master per name, side by side.
    fn doc_with_masters(names: &[&str]) -> (Doc, NodeId, Vec<(ComponentId, NodeId)>) {
        let mut doc = Doc::new();
        let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let page_id = page.id;
        doc.apply(Operation::create_node(page)).unwrap();
        doc.add_page(page_id);
        doc.set_active_page(Some(page_id));
        let mut masters = Vec::new();
        for (index, name) in names.iter().enumerate() {
            let mut root = CanvasNode::new(NodeData::Group(GroupNode {
                clip_size: Some([100.0, 40.0]),
                ..Default::default()
            }));
            root.name = (*name).to_owned();
            root.parent = Some(page_id);
            root.index = doc.scene.next_child_index(Some(page_id));
            root.transform = Transform2D::translation(index as f64 * 150.0, 300.0);
            let root_id = root.id;
            doc.apply(Operation::create_node(root)).unwrap();
            let component = ComponentId::new();
            doc.apply(Operation::DefineComponent {
                def: Box::new(ComponentDef::new(component, root_id, *name)),
            })
            .unwrap();
            masters.push((component, root_id));
        }
        (doc, page_id, masters)
    }

    fn apply(doc: &mut Doc, plan: impl FnOnce(&Doc) -> SetEdit) -> (ComponentId, Option<NodeId>) {
        let edit = plan(doc);
        for operation in edit.operations {
            doc.apply(operation).expect("applying the planned edit");
        }
        (edit.set, edit.frame)
    }

    fn roots(masters: &[(ComponentId, NodeId)]) -> Vec<NodeId> {
        masters.iter().map(|(_, root)| *root).collect()
    }

    fn cell(doc: &Doc, root: NodeId) -> (u16, u16) {
        let cell = doc
            .scene
            .get(root)
            .unwrap()
            .layout_child
            .unwrap()
            .grid
            .unwrap();
        (cell.column, cell.row)
    }

    #[test]
    fn combining_named_variants_makes_one_frame_with_a_property_grid() {
        let (mut doc, page, masters) = doc_with_masters(&[
            "Variant=Primary, State=Default",
            "Variant=Primary, State=Hover",
            "Variant=Secondary, State=Default",
            "Variant=Secondary, State=Hover",
        ]);
        let (set_id, frame) = apply(&mut doc, |doc| {
            combine_variants(doc, &roots(&masters), Some("Button")).unwrap()
        });
        let frame = frame.unwrap();
        let set = &doc.components.sets[&set_id];
        assert_eq!(set.name, "Button");
        assert_eq!(set.root, Some(frame));
        assert_eq!(
            set.axes
                .iter()
                .map(|axis| (axis.name.as_str(), axis.values.clone()))
                .collect::<Vec<_>>(),
            [
                (
                    "Variant",
                    vec!["Primary".to_owned(), "Secondary".to_owned()]
                ),
                ("State", vec!["Default".to_owned(), "Hover".to_owned()]),
            ]
        );
        let frame_node = doc.scene.get(frame).unwrap();
        assert_eq!(frame_node.name, "Button");
        assert_eq!(frame_node.parent, Some(page));
        assert_eq!(
            doc.scene.children_of(Some(frame)),
            roots(&masters).as_slice()
        );
        assert_eq!(
            roots(&masters)
                .into_iter()
                .map(|root| cell(&doc, root))
                .collect::<Vec<_>>(),
            [(0, 0), (1, 0), (0, 1), (1, 1)]
        );

        // The grid lays the variants out without overlap and hugs them.
        solve_auto_layout(&mut doc.scene, frame, &mut |_| (0.0, 0.0));
        let bounds: Vec<_> = roots(&masters)
            .into_iter()
            .map(|root| doc.scene.world_bounds(root).unwrap())
            .collect();
        assert!(bounds[1].min_x >= bounds[0].max_x + SET_GAP - 1e-9);
        assert!(bounds[2].min_y >= bounds[0].max_y + SET_GAP - 1e-9);
        let NodeData::Group(group) = &doc.scene.get(frame).unwrap().data else {
            panic!("frame");
        };
        let expected = 2.0 * 100.0 + SET_GAP + 2.0 * SET_PADDING;
        assert!((group.clip_size.unwrap()[0] - expected).abs() < 1e-9);
    }

    #[test]
    fn plain_names_become_one_variant_axis_and_duplicates_are_refused() {
        let (mut doc, _, masters) = doc_with_masters(&["Small", "Large"]);
        let (set_id, _) = apply(&mut doc, |doc| {
            combine_variants(doc, &roots(&masters), None).unwrap()
        });
        let set = &doc.components.sets[&set_id];
        assert_eq!(set.name, "Small");
        assert_eq!(set.axes[0].name, "Variant");
        assert_eq!(set.axes[0].values, ["Small", "Large"]);

        let (doc, _, masters) = doc_with_masters(&["Size=S", "Size=S"]);
        let error = combine_variants(&doc, &roots(&masters), None)
            .err()
            .unwrap();
        assert!(error.to_string().contains("Size=S"), "{error}");
    }

    #[test]
    fn adding_a_state_axis_renames_the_existing_variants_and_regrids() {
        // The case behind a hand-edited sets.json: a 3-variant Button gets
        // hover states.
        let (mut doc, _, masters) = doc_with_masters(&[
            "Variant=Primary",
            "Variant=Secondary",
            "Variant=Primary, State=Hover",
        ]);
        let (set_id, _) = apply(&mut doc, |doc| {
            combine_variants(doc, &roots(&masters[..2]), Some("Button")).unwrap()
        });
        let (_, frame) = apply(&mut doc, |doc| {
            add_variants(doc, set_id, &[masters[2].1]).unwrap()
        });
        let set = &doc.components.sets[&set_id];
        assert_eq!(set.members.len(), 3);
        assert_eq!(set.axes[1].name, "State");
        assert_eq!(set.axes[1].values, ["Default", "Hover"]);
        let names: Vec<_> = set
            .members
            .iter()
            .map(|member| doc.components.def(*member).unwrap().name.clone())
            .collect();
        assert_eq!(
            names,
            [
                "Variant=Primary, State=Default",
                "Variant=Secondary, State=Default",
                "Variant=Primary, State=Hover",
            ]
        );
        assert_eq!(
            doc.scene.get(masters[0].1).unwrap().name,
            "Variant=Primary, State=Default",
            "the master layer follows the component name"
        );
        assert_eq!(doc.scene.children_of(frame), roots(&masters).as_slice());
        assert_eq!(cell(&doc, masters[2].1), (1, 0));
        assert_eq!(cell(&doc, masters[1].1), (0, 1));
    }

    #[test]
    fn arranging_a_set_without_a_frame_wraps_every_variant_in_one() {
        let (mut doc, page, masters) = doc_with_masters(&[
            "Variant=Primary, State=Default",
            "Variant=Primary, State=Hover",
        ]);
        // A set made outside the editor: members, no frame.
        let set_id = ComponentId::new();
        let set = ComponentSet {
            id: set_id,
            name: "Button".into(),
            axes: vec![
                VariantAxis {
                    name: "Variant".into(),
                    values: vec!["Primary".into()],
                },
                VariantAxis {
                    name: "State".into(),
                    values: vec!["Default".into(), "Hover".into()],
                },
            ],
            members: masters.iter().map(|(component, _)| *component).collect(),
            default_variant: masters[0].0,
            root: None,
        };
        doc.apply(Operation::DefineComponentSet { set: Box::new(set) })
            .unwrap();
        for ((component, _), state) in masters.iter().zip(["Default", "Hover"]) {
            doc.apply(Operation::SetVariantMembership {
                id: *component,
                old: None,
                new: Some(ComponentSetMembership {
                    set: set_id,
                    axis_values: BTreeMap::from([
                        ("Variant".to_owned(), "Primary".to_owned()),
                        ("State".to_owned(), state.to_owned()),
                    ]),
                }),
            })
            .unwrap();
        }
        let (_, frame) = apply(&mut doc, |doc| arrange_variants(doc, set_id).unwrap());
        let frame = frame.unwrap();
        assert_eq!(doc.components.sets[&set_id].root, Some(frame));
        assert_eq!(doc.scene.get(frame).unwrap().parent, Some(page));
        assert_eq!(
            doc.scene.children_of(Some(frame)),
            roots(&masters).as_slice()
        );
        assert_eq!(doc.scene.children_of(Some(page)), [frame]);

        // Arranging again changes nothing.
        assert!(
            arrange_variants(&doc, set_id)
                .unwrap()
                .operations
                .is_empty()
        );
    }

    #[test]
    fn removing_variants_moves_them_out_and_the_last_one_deletes_the_set() {
        let (mut doc, page, masters) = doc_with_masters(&["Size=S", "Size=M", "Size=L"]);
        let (set_id, frame) = apply(&mut doc, |doc| {
            combine_variants(doc, &roots(&masters), Some("Chip")).unwrap()
        });
        let frame = frame.unwrap();
        apply(&mut doc, |doc| remove_variant(doc, masters[1].0).unwrap());
        let set = &doc.components.sets[&set_id];
        assert_eq!(set.axes[0].values, ["S", "L"]);
        assert_eq!(doc.scene.get(masters[1].1).unwrap().parent, Some(page));
        assert!(
            doc.components
                .def(masters[1].0)
                .unwrap()
                .variant_of
                .is_none()
        );
        assert!(
            doc.scene
                .get(masters[1].1)
                .unwrap()
                .layout_child
                .is_none_or(|child| child.grid.is_none())
        );

        apply(&mut doc, |doc| remove_variant(doc, masters[0].0).unwrap());
        apply(&mut doc, |doc| remove_variant(doc, masters[2].0).unwrap());
        assert!(!doc.components.sets.contains_key(&set_id));
        assert!(!doc.scene.contains(frame), "the empty frame goes too");
    }

    #[test]
    fn a_set_frame_with_its_variants_survives_saving_and_reopening_the_project() {
        let (mut doc, page, masters) = doc_with_masters(&[
            "Variant=Primary, State=Default",
            "Variant=Primary, State=Hover",
        ]);
        let (set_id, frame) = apply(&mut doc, |doc| {
            combine_variants(doc, &roots(&masters), Some("Button")).unwrap()
        });
        let frame = frame.unwrap();
        let directory = tempfile::tempdir().unwrap();
        fanta_format::write_project_tree(directory.path(), &doc, &Default::default()).unwrap();
        let (read, _) = fanta_format::read_project_tree(directory.path()).unwrap();
        assert_eq!(read.components.sets[&set_id].root, Some(frame));
        assert_eq!(read.scene.get(frame).unwrap().parent, Some(page));
        assert_eq!(
            read.scene.children_of(Some(frame)),
            roots(&masters).as_slice()
        );
        assert_eq!(cell(&read, masters[1].1), (1, 0));
    }

    #[test]
    fn renaming_keeps_names_unique_and_the_master_layer_in_step() {
        let (mut doc, _, masters) = doc_with_masters(&["Card", "Badge"]);
        assert!(rename_component(&doc, masters[0].0, "Badge").is_err());
        for operation in rename_component(&doc, masters[0].0, "Product card").unwrap() {
            doc.apply(operation).unwrap();
        }
        assert_eq!(
            doc.components.def(masters[0].0).unwrap().name,
            "Product card"
        );
        assert_eq!(doc.scene.get(masters[0].1).unwrap().name, "Product card");
    }
}
