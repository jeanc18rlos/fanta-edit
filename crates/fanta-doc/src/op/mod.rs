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
//! component library, the variable registry, the active-mode map, or the flow
//! start node — all of which live on [`crate::doc::Doc`], not the scene. Rather
//! than give every op a `&mut Doc` (and pull rendering-adjacent concerns into
//! this crate's core), `apply`/`revert` take a single [`OpCtx`] bundling exactly
//! the mutable doc slices an op may need. Scene-only ops simply ignore the rest.
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
        ComponentId, ComponentPropId, ModeId, NodeId, ReactionId, VariableCollectionId, VariableId,
    };
    use crate::node::{
        Action, CanvasNode, Easing, GroupNode, InstanceNode, NodeData, Reaction, Transition,
        TransitionStyle, Trigger, VectorNode,
    };
    use crate::scene::Scene;
    use crate::transform::Transform2D;
    use crate::value::{VarValue, VariableType};
    use crate::variables::{Mode, Variable, VariableCollection, VariableRegistry};
    use std::collections::BTreeMap;

    /// A minimal owner of the doc slices an [`OpCtx`] borrows, for unit tests.
    struct TestDoc {
        scene: Scene,
        components: ComponentLibrary,
        variables: VariableRegistry,
        active_modes: BTreeMap<VariableCollectionId, ModeId>,
        flow_start: Option<NodeId>,
    }
    impl TestDoc {
        fn new() -> Self {
            Self {
                scene: Scene::new(),
                components: ComponentLibrary::new(),
                variables: VariableRegistry::new(),
                active_modes: BTreeMap::new(),
                flow_start: None,
            }
        }
        fn ctx(&mut self) -> OpCtx<'_> {
            OpCtx {
                scene: &mut self.scene,
                components: &mut self.components,
                variables: &mut self.variables,
                active_modes: &mut self.active_modes,
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
            transition: None,
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
            transition: None,
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
            transition: Some(Transition {
                style: TransitionStyle::Dissolve,
                duration_ms: 250,
                easing: Easing::EaseOut,
            }),
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
}
