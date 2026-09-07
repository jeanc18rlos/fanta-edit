//! Operations — the command-pattern building blocks for every doc mutation.
//!
//! ## Why
//!
//! Every edit — whether from a tool, an AI agent, a collab peer, or a script —
//! goes through the same typed surface. That gets us:
//!
//! - **Undo/redo for free.** Each op carries the inverse data on its struct.
//! - **A single audit trail.** Logging ops at one chokepoint surfaces what
//!   actually changed, regardless of who changed it.
//! - **AI is not special.** The agent surface in `fanta-ai` emits these same
//!   ops; they go through the same `apply` path the UI uses.
//!
//! ## The apply boundary — [`OpCtx`]
//!
//! Most ops touch only the [`Scene`]. But design-system ops (define a component,
//! create a variable, bind a property, set the active mode, …) touch the
//! component library, the variable registry, the active-mode map, motion clips,
//! or the flow start node — all of which live on [`crate::doc::Doc`], not the
//! scene. Rather than give every op a `&mut Doc` (and pull rendering-adjacent
//! concerns into this crate's core), `apply`/`revert` take a single [`OpCtx`]
//! bundling exactly the mutable doc slices an op may need. Scene-only ops simply
//! ignore the rest.
//!
//! `History::begin`/`commit`/`abort` deliberately keep taking `&mut Scene` (they
//! are transaction markers — they never replay ops), so tool and app call sites
//! that open/close transactions are unchanged. Only op *replay*
//! (`History::apply`/`undo`/`redo`) threads an [`OpCtx`], built by
//! `Doc::apply`/`undo`/`redo` from `&mut self`'s fields.
//!
//! ## Composability
//!
//! Multiple ops can be grouped into a [`Transaction`] (defined in
//! [`crate::history`]) which applies/reverts them atomically.
//!
//! ## Module map
//!
//! Thin manifest: [`ctx`] owns the [`OpCtx`] apply boundary and [`ModeScope`];
//! [`operation`] owns the [`Operation`] sum type and its construction/metadata
//! helpers; [`apply`] owns the `apply`/`revert` behaviour and the instance/mode
//! mutation helpers. All public items are re-exported here so `crate::op::Foo`
//! paths are unchanged.
//!
//! [`Scene`]: crate::scene::Scene
//! [`Transaction`]: crate::history::Transaction

mod apply;
mod ctx;
mod operation;

pub use ctx::{ModeScope, OpCtx};
pub use operation::Operation;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binding::BoundProp;
    use crate::color::Color;
    use crate::component::{ComponentDef, ComponentLibrary};
    use crate::id::{
        AnimationClipId, AnimationTrackId, ComponentId, ComponentPropId, KeyframeId, ModeId,
        NodeId, ReactionId, VariableCollectionId, VariableId,
    };
    use crate::motion::{
        AnimationClip, AnimationTrack, Interpolation, Keyframe, MotionProperty, MotionTarget,
    };
    use crate::node::{
        Action, CanvasNode, Easing, GroupNode, InstanceNode, NodeData, PrototypeAnimation,
        Reaction, Transition, TransitionStyle, Trigger, VectorNode,
    };
    use crate::scene::{Scene, SceneError};
    use crate::style::UnitInterval;
    use crate::transform::Transform2D;
    use crate::value::{ResolvedVarValue, VarValue, VariableType};
    use crate::variables::{Mode, Variable, VariableCollection, VariableRegistry};
    use std::collections::BTreeMap;

    /// A minimal owner of the doc slices an [`OpCtx`] borrows, for unit tests.
    struct TestDoc {
        scene: Scene,
        components: ComponentLibrary,
        variables: VariableRegistry,
        active_modes: BTreeMap<VariableCollectionId, ModeId>,
        motion: crate::motion::MotionLibrary,
        flow_start: Option<NodeId>,
    }
    impl TestDoc {
        fn new() -> Self {
            Self {
                scene: Scene::new(),
                components: ComponentLibrary::new(),
                variables: VariableRegistry::new(),
                active_modes: BTreeMap::new(),
                motion: crate::motion::MotionLibrary::new(),
                flow_start: None,
            }
        }
        fn ctx(&mut self) -> OpCtx<'_> {
            OpCtx {
                scene: &mut self.scene,
                components: &mut self.components,
                variables: &mut self.variables,
                active_modes: &mut self.active_modes,
                motion: &mut self.motion,
                flow_start: &mut self.flow_start,
            }
        }
    }

    fn rect_node() -> CanvasNode {
        CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            10.0,
            10.0,
            Color::WHITE,
        )))
    }

    #[test]
    fn create_apply_revert_round_trips() {
        let mut d = TestDoc::new();
        let node = rect_node();
        let id = node.id;
        let op = Operation::create_node(node);
        op.apply(&mut d.ctx()).unwrap();
        assert!(d.scene.contains(id));
        op.revert(&mut d.ctx()).unwrap();
        assert!(!d.scene.contains(id));
    }

    /// `SetOpacity` carries [`UnitInterval`]s, clamped at construction, so
    /// apply/revert are an ordinary symmetric pick — no clamp branch, and an
    /// out-of-range opacity is simply unconstructable.
    #[test]
    fn set_opacity_is_a_symmetric_pick_over_clamped_values() {
        let mut d = TestDoc::new();
        let node = rect_node();
        let id = node.id;
        d.scene.insert(node).unwrap();
        // Out-of-range inputs clamp when the op is built, not when it runs.
        let op = Operation::SetOpacity {
            id,
            old: UnitInterval::new(1.5), // → 1.0
            new: UnitInterval::new(0.5),
        };
        op.apply(&mut d.ctx()).unwrap();
        assert_eq!(d.scene.get(id).unwrap().opacity, UnitInterval::new(0.5));
        op.revert(&mut d.ctx()).unwrap();
        assert_eq!(d.scene.get(id).unwrap().opacity, UnitInterval::ONE);
    }

    #[test]
    fn set_meta_round_trips() {
        let mut d = TestDoc::new();
        let node = rect_node();
        let id = node.id;
        d.scene.insert(node).unwrap();
        assert!(d.scene.get(id).unwrap().meta.is_null());
        let new = serde_json::json!({ "page_icon": "home" });
        let op = Operation::SetMeta {
            id,
            old: serde_json::Value::Null,
            new: new.clone(),
        };
        op.apply(&mut d.ctx()).unwrap();
        assert_eq!(d.scene.get(id).unwrap().meta, new);
        op.revert(&mut d.ctx()).unwrap();
        assert!(d.scene.get(id).unwrap().meta.is_null());
    }

    #[test]
    fn set_transform_round_trips() {
        let mut d = TestDoc::new();
        let node = rect_node();
        let id = node.id;
        d.scene.insert(node).unwrap();
        let new = Transform2D::translation(50.0, 30.0);
        let op = Operation::SetTransform {
            id,
            old: Transform2D::IDENTITY,
            new,
        };
        op.apply(&mut d.ctx()).unwrap();
        assert_eq!(d.scene.get(id).unwrap().transform, new);
        op.revert(&mut d.ctx()).unwrap();
        assert_eq!(d.scene.get(id).unwrap().transform, Transform2D::IDENTITY);
    }

    #[test]
    fn delete_subtree_restores_descendants() {
        let mut d = TestDoc::new();
        let g = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let g_id = g.id;
        d.scene.insert(g).unwrap();
        let mut r = rect_node();
        r.parent = Some(g_id);
        let r_id = r.id;
        d.scene.insert(r).unwrap();
        let snapshot: Vec<_> = d
            .scene
            .descendants_of(g_id)
            .map(|id| d.scene.get(id).unwrap().clone())
            .collect();
        let op = Operation::DeleteSubtree { snapshot };
        op.apply(&mut d.ctx()).unwrap();
        assert!(!d.scene.contains(g_id) && !d.scene.contains(r_id));
        op.revert(&mut d.ctx()).unwrap();
        assert!(d.scene.contains(g_id) && d.scene.contains(r_id));
    }

    #[test]
    fn deleting_component_content_invalidates_instances_on_delete_and_undo() {
        let mut doc = crate::doc::Doc::new();
        let root = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let root_id = root.id;
        doc.scene.insert(root).unwrap();
        let mut child = rect_node();
        child.parent = Some(root_id);
        let child_id = child.id;
        doc.scene.insert(child).unwrap();
        let component = ComponentId::new();
        doc.components
            .defs
            .insert(component, ComponentDef::new(component, root_id, "Master"));

        let snapshot = doc
            .scene
            .descendants_of(child_id)
            .filter_map(|id| doc.scene.get(id).cloned())
            .collect();
        doc.apply(Operation::DeleteSubtree { snapshot }).unwrap();
        assert_eq!(doc.components.defs[&component].rev, 1);

        assert!(doc.undo().unwrap());
        assert_eq!(doc.components.defs[&component].rev, 2);
        assert!(doc.scene.contains(child_id));
    }

    #[test]
    fn creating_component_content_invalidates_instances_on_create_and_undo() {
        let mut doc = crate::doc::Doc::new();
        let root = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let root_id = root.id;
        doc.scene.insert(root).unwrap();
        let component = ComponentId::new();
        doc.components
            .defs
            .insert(component, ComponentDef::new(component, root_id, "Master"));

        let mut child = rect_node();
        child.parent = Some(root_id);
        let child_id = child.id;
        doc.apply(Operation::create_node(child)).unwrap();
        assert_eq!(doc.components.defs[&component].rev, 1);

        assert!(doc.undo().unwrap());
        assert_eq!(doc.components.defs[&component].rev, 2);
        assert!(!doc.scene.contains(child_id));
    }

    #[test]
    fn reparenting_between_component_masters_invalidates_both_on_apply_and_undo() {
        let mut doc = crate::doc::Doc::new();
        let first_root = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let first_root_id = first_root.id;
        doc.scene.insert(first_root).unwrap();
        let second_root = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let second_root_id = second_root.id;
        doc.scene.insert(second_root).unwrap();

        let mut child = rect_node();
        child.parent = Some(first_root_id);
        let child_id = child.id;
        let child_index = child.index;
        doc.scene.insert(child).unwrap();

        let first_component = ComponentId::new();
        let second_component = ComponentId::new();
        doc.components.defs.insert(
            first_component,
            ComponentDef::new(first_component, first_root_id, "First"),
        );
        doc.components.defs.insert(
            second_component,
            ComponentDef::new(second_component, second_root_id, "Second"),
        );

        doc.apply(Operation::Reparent {
            id: child_id,
            old_parent: Some(first_root_id),
            old_index: child_index,
            new_parent: Some(second_root_id),
            new_index: crate::index::IndexKey::FIRST,
        })
        .unwrap();
        assert_eq!(doc.components.defs[&first_component].rev, 1);
        assert_eq!(doc.components.defs[&second_component].rev, 1);

        assert!(doc.undo().unwrap());
        assert_eq!(doc.components.defs[&first_component].rev, 2);
        assert_eq!(doc.components.defs[&second_component].rev, 2);
        assert_eq!(doc.scene.get(child_id).unwrap().parent, Some(first_root_id));
    }

    #[test]
    fn define_component_round_trips() {
        let mut d = TestDoc::new();
        let cid = ComponentId::from_u128(1);
        let def = ComponentDef::new(cid, NodeId::from_u128(2), "Button");
        let op = Operation::DefineComponent { def: Box::new(def) };
        op.apply(&mut d.ctx()).unwrap();
        assert!(d.components.defs.contains_key(&cid));
        op.revert(&mut d.ctx()).unwrap();
        assert!(!d.components.defs.contains_key(&cid));
    }

    #[test]
    fn set_variant_membership_round_trips() {
        let mut d = TestDoc::new();
        let cid = ComponentId::from_u128(1);
        Operation::DefineComponent {
            def: Box::new(ComponentDef::new(cid, NodeId::from_u128(2), "Button")),
        }
        .apply(&mut d.ctx())
        .unwrap();
        assert!(d.components.defs[&cid].variant_of.is_none());

        let set = ComponentId::from_u128(9);
        let mut axis_values = std::collections::BTreeMap::new();
        axis_values.insert("Variant".to_owned(), "Primary".to_owned());
        let op = Operation::SetVariantMembership {
            id: cid,
            old: None,
            new: Some(crate::component::ComponentSetMembership { set, axis_values }),
        };
        op.apply(&mut d.ctx()).unwrap();
        let m = d.components.defs[&cid].variant_of.clone().unwrap();
        assert_eq!(m.set, set);
        assert_eq!(
            m.axis_values.get("Variant").map(String::as_str),
            Some("Primary")
        );

        op.revert(&mut d.ctx()).unwrap();
        assert!(d.components.defs[&cid].variant_of.is_none());
    }

    #[test]
    fn set_component_set_round_trips() {
        use crate::component::{ComponentSet, VariantAxis};
        let mut d = TestDoc::new();
        let set_id = ComponentId::from_u128(7);
        let m1 = ComponentId::from_u128(1);
        let m2 = ComponentId::from_u128(2);
        let old = ComponentSet {
            id: set_id,
            name: "Button".into(),
            axes: vec![VariantAxis {
                name: "Size".into(),
                values: vec!["S".into(), "M".into()],
            }],
            members: vec![m1, m2],
            default_variant: m1,
        };
        Operation::DefineComponentSet {
            set: Box::new(old.clone()),
        }
        .apply(&mut d.ctx())
        .unwrap();

        let mut new = old.clone();
        new.name = "Btn".into();
        new.axes[0].values.push("L".into());
        new.default_variant = m2;
        let op = Operation::SetComponentSet {
            id: set_id,
            old: Box::new(old),
            new: Box::new(new),
        };
        op.apply(&mut d.ctx()).unwrap();
        assert_eq!(d.components.sets[&set_id].name, "Btn");
        assert_eq!(d.components.sets[&set_id].axes[0].values.len(), 3);
        assert_eq!(d.components.sets[&set_id].default_variant, m2);

        op.revert(&mut d.ctx()).unwrap();
        assert_eq!(d.components.sets[&set_id].name, "Button");
        assert_eq!(d.components.sets[&set_id].axes[0].values.len(), 2);
        assert_eq!(d.components.sets[&set_id].default_variant, m1);
    }

    #[test]
    fn create_variable_and_set_value_round_trip() {
        let mut d = TestDoc::new();
        let cid = VariableCollectionId::from_u128(1);
        let mode = ModeId::from_u128(2);
        let vid = VariableId::from_u128(3);
        Operation::CreateVariableCollection {
            collection: Box::new(VariableCollection {
                id: cid,
                name: "Theme".into(),
                modes: vec![Mode {
                    id: mode,
                    name: "Light".into(),
                }],
                default_mode: mode,
                variable_order: vec![],
            }),
        }
        .apply(&mut d.ctx())
        .unwrap();
        let create_variable = Operation::CreateVariable {
            variable: Box::new(Variable {
                id: vid,
                collection: cid,
                name: "bg".into(),
                ty: VariableType::Color,
                values_by_mode: BTreeMap::new(),
                scopes: vec![],
            }),
        };
        create_variable.apply(&mut d.ctx()).unwrap();
        assert_eq!(d.variables.collections[&cid].variable_order, vec![vid]);

        let set = Operation::SetVariableValue {
            variable: vid,
            mode,
            old: None,
            new: Some(VarValue::Color {
                value: Color::WHITE,
            }),
        };
        set.apply(&mut d.ctx()).unwrap();
        assert_eq!(
            d.variables.variables[&vid].values_by_mode.get(&mode),
            Some(&VarValue::Color {
                value: Color::WHITE
            })
        );
        set.revert(&mut d.ctx()).unwrap();
        assert!(d.variables.variables[&vid].values_by_mode.is_empty());

        create_variable.revert(&mut d.ctx()).unwrap();
        assert!(!d.variables.variables.contains_key(&vid));
        assert!(d.variables.collections[&cid].variable_order.is_empty());
        create_variable.apply(&mut d.ctx()).unwrap();
        assert_eq!(d.variables.collections[&cid].variable_order, vec![vid]);
    }

    #[test]
    fn rename_variable_collection_and_mode_round_trip() {
        let mut d = TestDoc::new();
        let cid = VariableCollectionId::from_u128(1);
        let mode = ModeId::from_u128(2);
        let vid = VariableId::from_u128(3);
        Operation::CreateVariableCollection {
            collection: Box::new(VariableCollection {
                id: cid,
                name: "Theme".into(),
                modes: vec![Mode {
                    id: mode,
                    name: "Light".into(),
                }],
                default_mode: mode,
                variable_order: vec![],
            }),
        }
        .apply(&mut d.ctx())
        .unwrap();
        Operation::CreateVariable {
            variable: Box::new(Variable {
                id: vid,
                collection: cid,
                name: "bg".into(),
                ty: VariableType::Color,
                values_by_mode: BTreeMap::new(),
                scopes: vec![],
            }),
        }
        .apply(&mut d.ctx())
        .unwrap();

        let rv = Operation::RenameVariable {
            id: vid,
            old: "bg".into(),
            new: "surface".into(),
        };
        rv.apply(&mut d.ctx()).unwrap();
        assert_eq!(d.variables.variables[&vid].name, "surface");
        rv.revert(&mut d.ctx()).unwrap();
        assert_eq!(d.variables.variables[&vid].name, "bg");

        let rc = Operation::RenameVariableCollection {
            id: cid,
            old: "Theme".into(),
            new: "Brand".into(),
        };
        rc.apply(&mut d.ctx()).unwrap();
        assert_eq!(d.variables.collections[&cid].name, "Brand");
        rc.revert(&mut d.ctx()).unwrap();
        assert_eq!(d.variables.collections[&cid].name, "Theme");

        let rm = Operation::RenameMode {
            collection: cid,
            mode,
            old: "Light".into(),
            new: "Day".into(),
        };
        rm.apply(&mut d.ctx()).unwrap();
        assert_eq!(d.variables.collections[&cid].modes[0].name, "Day");
        rm.revert(&mut d.ctx()).unwrap();
        assert_eq!(d.variables.collections[&cid].modes[0].name, "Light");
    }

    #[test]
    fn bind_property_round_trips() {
        let mut d = TestDoc::new();
        let node = rect_node();
        let id = node.id;
        d.scene.insert(node).unwrap();
        let vid = VariableId::from_u128(7);
        let op = Operation::BindProperty {
            node: id,
            prop: BoundProp::Opacity,
            old: None,
            new: vid,
        };
        op.apply(&mut d.ctx()).unwrap();
        assert_eq!(d.scene.get(id).unwrap().bindings[&BoundProp::Opacity], vid);
        op.revert(&mut d.ctx()).unwrap();
        assert!(d.scene.get(id).unwrap().bindings.is_empty());
    }

    #[test]
    fn set_active_mode_doc_and_frame_round_trip() {
        let mut d = TestDoc::new();
        let coll = VariableCollectionId::from_u128(1);
        let m = ModeId::from_u128(2);
        let doc_op = Operation::SetActiveMode {
            scope: ModeScope::Doc,
            collection: coll,
            old: None,
            new: Some(m),
        };
        doc_op.apply(&mut d.ctx()).unwrap();
        assert_eq!(d.active_modes.get(&coll), Some(&m));
        doc_op.revert(&mut d.ctx()).unwrap();
        assert!(d.active_modes.is_empty());

        let frame = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let fid = frame.id;
        d.scene.insert(frame).unwrap();
        let frame_op = Operation::SetActiveMode {
            scope: ModeScope::Frame { node: fid },
            collection: coll,
            old: None,
            new: Some(m),
        };
        frame_op.apply(&mut d.ctx()).unwrap();
        match &d.scene.get(fid).unwrap().data {
            NodeData::Group(g) => assert_eq!(g.explicit_modes.get(&coll), Some(&m)),
            _ => panic!(),
        }
        frame_op.revert(&mut d.ctx()).unwrap();
        match &d.scene.get(fid).unwrap().data {
            NodeData::Group(g) => assert!(g.explicit_modes.is_empty()),
            _ => panic!(),
        }
    }

    #[test]
    fn instance_prop_and_swap_round_trip() {
        let mut d = TestDoc::new();
        let inst = CanvasNode::new(NodeData::Instance(InstanceNode {
            component: ComponentId::from_u128(1),
            overrides: vec![],
            prop_values: BTreeMap::new(),
            derived: Vec::new(),
            local_size: [10.0, 10.0],
        }));
        let id = inst.id;
        d.scene.insert(inst).unwrap();

        let prop = ComponentPropId::from_u128(5);
        let set_prop = Operation::SetInstanceProp {
            id,
            prop,
            old: None,
            new: Some(VarValue::Boolean { value: true }),
        };
        set_prop.apply(&mut d.ctx()).unwrap();
        match &d.scene.get(id).unwrap().data {
            NodeData::Instance(i) => {
                assert_eq!(i.prop_values[&prop], VarValue::Boolean { value: true })
            }
            _ => panic!(),
        }
        set_prop.revert(&mut d.ctx()).unwrap();

        let swap = Operation::SwapInstance {
            id,
            old: ComponentId::from_u128(1),
            new: ComponentId::from_u128(2),
        };
        swap.apply(&mut d.ctx()).unwrap();
        match &d.scene.get(id).unwrap().data {
            NodeData::Instance(i) => assert_eq!(i.component, ComponentId::from_u128(2)),
            _ => panic!(),
        }
        swap.revert(&mut d.ctx()).unwrap();
        match &d.scene.get(id).unwrap().data {
            NodeData::Instance(i) => assert_eq!(i.component, ComponentId::from_u128(1)),
            _ => panic!(),
        }
    }

    #[test]
    fn add_remove_reaction_and_flow_start_round_trip() {
        let mut d = TestDoc::new();
        let node = rect_node();
        let id = node.id;
        d.scene.insert(node).unwrap();
        let reaction = Reaction {
            id: ReactionId::from_u128(1),
            trigger: Trigger::Click,
            action: Action::Back,
            extra_actions: Vec::new(),
            transition: None,
            animation: Some(PrototypeAnimation {
                clip: AnimationClipId::from_u128(8),
                delay_ms: 125,
            }),
        };
        let add = Operation::AddReaction { node: id, reaction };
        add.apply(&mut d.ctx()).unwrap();
        assert_eq!(d.scene.get(id).unwrap().reactions.len(), 1);
        add.revert(&mut d.ctx()).unwrap();
        assert!(d.scene.get(id).unwrap().reactions.is_empty());

        let fs = Operation::SetFlowStart {
            old: None,
            new: Some(id),
        };
        fs.apply(&mut d.ctx()).unwrap();
        assert_eq!(d.flow_start, Some(id));
        fs.revert(&mut d.ctx()).unwrap();
        assert_eq!(d.flow_start, None);
    }

    /// Pin the on-disk JSON envelope of [`Operation`]. `Doc.history` and the
    /// session journal persist serialized ops, so the `"op"` tag key, the
    /// snake_case variant names, and the `old`/`new` (and other) field names
    /// are a wire format: a rename here silently breaks reading old documents.
    /// Ids are interpolated (their encoding is pinned in `id.rs`); everything
    /// else is a hard literal on purpose.
    #[test]
    fn operation_json_envelope_is_pinned() {
        use serde_json::json;

        let id = NodeId::from_u128(1);
        let id_j = serde_json::to_value(id).unwrap();

        // Scalar field-write op: tag key, snake_case name, old/new fields.
        // UnitInterval serializes as a bare number, so the wire is unchanged.
        assert_eq!(
            serde_json::to_value(Operation::SetOpacity {
                id,
                old: UnitInterval::ONE,
                new: UnitInterval::new(0.5),
            })
            .unwrap(),
            json!({ "op": "set_opacity", "id": id_j, "old": 1.0, "new": 0.5 }),
        );

        // Option field, absent → null.
        assert_eq!(
            serde_json::to_value(Operation::SetFlowStart {
                old: None,
                new: Some(id),
            })
            .unwrap(),
            json!({ "op": "set_flow_start", "old": null, "new": id_j }),
        );

        // Multi-field structural op — pins the four reparent field names, the
        // exact shape the merged `run` now reads as an (parent, index) tuple.
        let reparent = serde_json::to_value(Operation::Reparent {
            id,
            old_parent: None,
            old_index: crate::index::IndexKey::FIRST,
            new_parent: Some(id),
            new_index: crate::index::IndexKey::FIRST,
        })
        .unwrap();
        assert_eq!(reparent["op"], "reparent");
        for key in ["id", "old_parent", "old_index", "new_parent", "new_index"] {
            assert!(reparent.get(key).is_some(), "missing reparent field {key}");
        }

        // Nested internally-tagged enum: BoundProp carries its own `prop` tag.
        let vid = VariableId::from_u128(7);
        assert_eq!(
            serde_json::to_value(Operation::BindProperty {
                node: id,
                prop: BoundProp::Opacity,
                old: None,
                new: vid,
            })
            .unwrap(),
            json!({
                "op": "bind_property",
                "node": id_j,
                "prop": { "prop": "opacity" },
                "old": null,
                "new": serde_json::to_value(vid).unwrap(),
            }),
        );

        // Nested ModeScope carries its own `scope` tag.
        let coll = VariableCollectionId::from_u128(2);
        let mode = ModeId::from_u128(3);
        assert_eq!(
            serde_json::to_value(Operation::SetActiveMode {
                scope: ModeScope::Doc,
                collection: coll,
                old: None,
                new: Some(mode),
            })
            .unwrap(),
            json!({
                "op": "set_active_mode",
                "scope": { "scope": "doc" },
                "collection": serde_json::to_value(coll).unwrap(),
                "old": null,
                "new": serde_json::to_value(mode).unwrap(),
            }),
        );

        // The tag round-trips back to the right variant on deserialize.
        let bytes = serde_json::to_string(&Operation::SetOpacity {
            id,
            old: UnitInterval::ONE,
            new: UnitInterval::new(0.5),
        })
        .unwrap();
        assert!(matches!(
            serde_json::from_str::<Operation>(&bytes).unwrap(),
            Operation::SetOpacity { new, .. } if new.get() == 0.5
        ));
    }

    #[test]
    fn set_reaction_edits_in_place_round_trip() {
        let mut d = TestDoc::new();
        let node = rect_node();
        let id = node.id;
        d.scene.insert(node).unwrap();
        // Seed a Click → Back reaction.
        let old = Reaction {
            id: ReactionId::from_u128(7),
            trigger: Trigger::Click,
            action: Action::Back,
            extra_actions: Vec::new(),
            transition: None,
            animation: Some(PrototypeAnimation {
                clip: AnimationClipId::from_u128(9),
                delay_ms: 50,
            }),
        };
        Operation::AddReaction {
            node: id,
            reaction: old.clone(),
        }
        .apply(&mut d.ctx())
        .unwrap();

        // Replace it with a Hover → Close + dissolve transition (same id).
        let new = Reaction {
            id: old.id,
            trigger: Trigger::Hover,
            action: Action::Close,
            extra_actions: Vec::new(),
            transition: Some(Transition {
                style: TransitionStyle::Dissolve,
                duration_ms: 250,
                easing: Easing::EaseOut,
            }),
            animation: None,
        };
        let edit = Operation::SetReaction {
            node: id,
            index: 0,
            old: old.clone(),
            new: new.clone(),
        };
        edit.apply(&mut d.ctx()).unwrap();
        assert_eq!(d.scene.get(id).unwrap().reactions[0], new);
        // Revert restores the original reaction exactly.
        edit.revert(&mut d.ctx()).unwrap();
        assert_eq!(d.scene.get(id).unwrap().reactions[0], old);
    }

    #[test]
    fn motion_operations_apply_revert_and_report_their_node() -> Result<(), SceneError> {
        let mut d = TestDoc::new();
        let node = rect_node();
        let node_id = node.id;
        d.scene.insert(node)?;

        let clip_id = AnimationClipId::from_u128(100);
        let create_clip = Operation::CreateAnimationClip {
            clip: Box::new(AnimationClip::new(clip_id, "Entrance", 1_000)),
        };
        create_clip.apply(&mut d.ctx())?;
        assert!(d.motion.clip(clip_id).is_some());

        let track_id = AnimationTrackId::from_u128(101);
        let target = MotionTarget::new(node_id, MotionProperty::PositionX);
        let track = AnimationTrack::new(track_id, target);
        let set_track = Operation::SetAnimationTrack {
            clip: clip_id,
            track: track_id,
            old: None,
            new: Some(Box::new(track)),
        };
        assert_eq!(set_track.primary_target(), Some(node_id));
        assert_eq!(set_track.label(), "Edit Animation Track");
        set_track.apply(&mut d.ctx())?;

        let keyframe_id = KeyframeId::from_u128(102);
        let keyframe = Keyframe {
            id: keyframe_id,
            time_ms: 500,
            value: ResolvedVarValue::Float { value: 42.0 },
            interpolation: Interpolation::Linear,
            easing: Easing::Linear,
        };
        let set_keyframe = Operation::SetKeyframe {
            clip: clip_id,
            track: track_id,
            target,
            keyframe: keyframe_id,
            old: None,
            new: Some(keyframe),
        };
        assert_eq!(set_keyframe.primary_target(), Some(node_id));
        assert_eq!(set_keyframe.label(), "Edit Keyframe");
        set_keyframe.apply(&mut d.ctx())?;
        let sampled = d
            .motion
            .evaluate(clip_id, 500)
            .and_then(|evaluation| evaluation.get(target).cloned());
        assert_eq!(sampled, Some(ResolvedVarValue::Float { value: 42.0 }));

        set_keyframe.revert(&mut d.ctx())?;
        assert!(
            d.motion
                .evaluate(clip_id, 500)
                .is_some_and(|e| e.is_empty())
        );
        set_track.revert(&mut d.ctx())?;
        assert!(
            d.motion
                .clip(clip_id)
                .is_some_and(|clip| clip.tracks.is_empty())
        );

        let rename = Operation::SetAnimationClipName {
            id: clip_id,
            old: "Entrance".into(),
            new: "Reveal".into(),
        };
        rename.apply(&mut d.ctx())?;
        assert_eq!(
            d.motion.clip(clip_id).map(|clip| clip.name.as_str()),
            Some("Reveal")
        );
        rename.revert(&mut d.ctx())?;

        let set_duration = Operation::SetAnimationClipDuration {
            id: clip_id,
            old: 1_000,
            new: 2_000,
        };
        set_duration.apply(&mut d.ctx())?;
        assert_eq!(
            d.motion.clip(clip_id).map(|clip| clip.duration_ms),
            Some(2_000)
        );
        set_duration.revert(&mut d.ctx())?;

        let Some(snapshot) = d.motion.clip(clip_id).cloned() else {
            return Err(SceneError::InvariantViolated(
                "motion fixture clip disappeared".into(),
            ));
        };
        let delete_clip = Operation::DeleteAnimationClip {
            id: clip_id,
            clip: Box::new(snapshot),
        };
        delete_clip.apply(&mut d.ctx())?;
        assert!(d.motion.clip(clip_id).is_none());
        delete_clip.revert(&mut d.ctx())?;
        assert!(d.motion.clip(clip_id).is_some());

        create_clip.revert(&mut d.ctx())?;
        assert!(d.motion.clip(clip_id).is_none());
        Ok(())
    }

    #[test]
    fn keyframe_operation_round_trips_through_json() -> serde_json::Result<()> {
        let target = MotionTarget::new(NodeId::from_u128(1), MotionProperty::Rotation);
        let operation = Operation::SetKeyframe {
            clip: AnimationClipId::from_u128(2),
            track: AnimationTrackId::from_u128(3),
            target,
            keyframe: KeyframeId::from_u128(4),
            old: None,
            new: Some(Keyframe {
                id: KeyframeId::from_u128(4),
                time_ms: 250,
                value: ResolvedVarValue::Float { value: 1.5 },
                interpolation: Interpolation::Linear,
                easing: Easing::EaseOut,
            }),
        };

        let json = serde_json::to_string(&operation)?;
        let restored: Operation = serde_json::from_str(&json)?;
        assert!(matches!(
            restored,
            Operation::SetKeyframe {
                target: restored_target,
                new: Some(Keyframe { time_ms: 250, .. }),
                ..
            } if restored_target == target
        ));
        Ok(())
    }
}
