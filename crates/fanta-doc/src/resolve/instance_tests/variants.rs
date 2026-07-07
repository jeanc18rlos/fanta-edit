//! Variant-set selection: default-variant fallback, exact single-axis match,
//! dangling-default empties, and two-axis exact / partial-best-match /
//! unrelated-selection behavior.

use super::*;

/// Build a two-variant component SET in `scene`: a "Size" axis with members
/// "Size=Small" (a text child "S") and "Size=Large" (a text child "L"). The
/// `Variant { axis: "Size" }` prop is exposed on each member with the given
/// `axis_prop` id so an instance can select a variant. Returns the library
/// plus the set id and both member component ids.
fn variant_set(
    scene: &mut Scene,
) -> (
    ComponentLibrary,
    ComponentId,
    ComponentId,
    ComponentId,
    crate::id::ComponentPropId,
) {
    use crate::component::{
        ComponentPropDef, ComponentPropKind, ComponentSet, ComponentSetMembership, VariantAxis,
    };
    let mut lib = ComponentLibrary::new();
    let axis_prop = crate::id::ComponentPropId::new();

    let mut mk = |label: &str, value: &str| -> ComponentId {
        let mut root = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([40.0, 40.0]),
            background: None,
            explicit_modes: Default::default(),
            ..Default::default()
        }));
        root.name = format!("Size={value}");
        let root_id = root.id;
        scene.insert(root).unwrap();
        let mut child = CanvasNode::new(NodeData::Text(TextNode::new(label, 30.0, 20.0)));
        child.parent = Some(root_id);
        scene.insert(child).unwrap();

        let cid = ComponentId::new();
        let mut def = ComponentDef::new(cid, root_id, format!("Size={value}"));
        def.variant_of = Some(ComponentSetMembership {
            set: ComponentId::from_u128(0), // filled after the set id exists
            axis_values: BTreeMap::from([("Size".to_owned(), value.to_owned())]),
        });
        def.props.push(ComponentPropDef {
            id: axis_prop,
            name: "Size".into(),
            kind: ComponentPropKind::Variant {
                axis: "Size".into(),
            },
            formatter: Default::default(),
            default: VarValue::String {
                value: value.to_owned(),
            },
            bindings: Vec::new(),
        });
        lib.defs.insert(cid, def);
        cid
    };

    let small = mk("S", "Small");
    let large = mk("L", "Large");

    let set_id = ComponentId::new();
    // Point each member's membership at the real set id.
    for m in [small, large] {
        if let Some(d) = lib.defs.get_mut(&m) {
            if let Some(vm) = d.variant_of.as_mut() {
                vm.set = set_id;
            }
        }
    }
    lib.sets.insert(
        set_id,
        ComponentSet {
            id: set_id,
            name: "Button".into(),
            axes: vec![VariantAxis {
                name: "Size".into(),
                values: vec!["Small".into(), "Large".into()],
            }],
            members: vec![small, large],
            default_variant: small,
        },
    );
    (lib, set_id, small, large, axis_prop)
}

#[test]
fn expand_set_instance_uses_default_variant() {
    let mut scene = Scene::new();
    let (lib, set_id, _small, _large, _axis) = variant_set(&mut scene);
    // An instance pointing at the SET id (no variant selection) expands to
    // the default member (Small → child text "S").
    let inst = InstanceNode {
        component: set_id,
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [40.0, 40.0],
    };
    let expanded = expand_instance(&scene, &lib, &inst);
    assert_eq!(expanded.len(), 2, "set instance expands the default member");
    let text = expanded.iter().find_map(|e| match &e.node.data {
        NodeData::Text(t) => Some(t.content.clone()),
        _ => None,
    });
    assert_eq!(
        text.as_deref(),
        Some("S"),
        "default variant is the small member"
    );
}

#[test]
fn expand_set_instance_selects_matching_variant() {
    let mut scene = Scene::new();
    let (lib, set_id, _small, _large, axis_prop) = variant_set(&mut scene);
    // Select the "Large" variant via the variant prop.
    let inst = InstanceNode {
        component: set_id,
        overrides: Vec::new(),
        prop_values: BTreeMap::from([(
            axis_prop,
            VarValue::String {
                value: "Large".into(),
            },
        )]),
        derived: Vec::new(),
        local_size: [40.0, 40.0],
    };
    let expanded = expand_instance(&scene, &lib, &inst);
    let text = expanded.iter().find_map(|e| match &e.node.data {
        NodeData::Text(t) => Some(t.content.clone()),
        _ => None,
    });
    assert_eq!(
        text.as_deref(),
        Some("L"),
        "variant selection picks the large member"
    );
}

#[test]
fn expand_set_with_dangling_default_yields_empty() {
    // A degenerate set whose default_variant is not a real def resolves to
    // nothing rather than panicking.
    use crate::component::ComponentSet;
    let scene = Scene::new();
    let mut lib = ComponentLibrary::new();
    let set_id = ComponentId::new();
    let phantom = ComponentId::new();
    lib.sets.insert(
        set_id,
        ComponentSet {
            id: set_id,
            name: "Empty".into(),
            axes: Vec::new(),
            members: Vec::new(),
            default_variant: phantom,
        },
    );
    let inst = InstanceNode {
        component: set_id,
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [10.0, 10.0],
    };
    assert!(expand_instance(&scene, &lib, &inst).is_empty());
}

/// Build a 2-axis variant set (Size ∈ {S, M}, State ∈ {Default, Hover}) with all
/// four members, each member's text being `"{size}/{state}"`. Returns the lib,
/// set id, the Size + State variant prop ids, and the four member ids keyed by
/// (size, state).
#[allow(clippy::type_complexity)]
fn two_axis_set(
    scene: &mut Scene,
) -> (
    ComponentLibrary,
    ComponentId,
    crate::id::ComponentPropId,
    crate::id::ComponentPropId,
    std::collections::BTreeMap<(&'static str, &'static str), ComponentId>,
) {
    use crate::component::{
        ComponentPropDef, ComponentPropKind, ComponentSet, ComponentSetMembership, VariantAxis,
    };
    let mut lib = ComponentLibrary::new();
    let size_prop = crate::id::ComponentPropId::new();
    let state_prop = crate::id::ComponentPropId::new();
    let set_id = ComponentId::new();
    let mut members: std::collections::BTreeMap<(&'static str, &'static str), ComponentId> =
        Default::default();

    for (size, state) in [
        ("S", "Default"),
        ("S", "Hover"),
        ("M", "Default"),
        ("M", "Hover"),
    ] {
        let mut root = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([40.0, 40.0]),
            ..Default::default()
        }));
        root.name = format!("Size={size}, State={state}");
        let root_name = root.name.clone();
        let root_id = root.id;
        scene.insert(root).unwrap();
        let mut child = CanvasNode::new(NodeData::Text(TextNode::new(
            format!("{size}/{state}"),
            30.0,
            20.0,
        )));
        child.parent = Some(root_id);
        scene.insert(child).unwrap();

        let cid = ComponentId::new();
        let mut def = ComponentDef::new(cid, root_id, root_name);
        def.variant_of = Some(ComponentSetMembership {
            set: set_id,
            axis_values: BTreeMap::from([
                ("Size".to_owned(), size.to_owned()),
                ("State".to_owned(), state.to_owned()),
            ]),
        });
        def.props.push(ComponentPropDef {
            id: size_prop,
            name: "Size".into(),
            kind: ComponentPropKind::Variant {
                axis: "Size".into(),
            },
            formatter: Default::default(),
            default: VarValue::String { value: "S".into() },
            bindings: Vec::new(),
        });
        def.props.push(ComponentPropDef {
            id: state_prop,
            name: "State".into(),
            kind: ComponentPropKind::Variant {
                axis: "State".into(),
            },
            formatter: Default::default(),
            default: VarValue::String {
                value: "Default".into(),
            },
            bindings: Vec::new(),
        });
        lib.defs.insert(cid, def);
        members.insert((size, state), cid);
    }

    lib.sets.insert(
        set_id,
        ComponentSet {
            id: set_id,
            name: "Btn".into(),
            axes: vec![
                VariantAxis {
                    name: "Size".into(),
                    values: vec!["S".into(), "M".into()],
                },
                VariantAxis {
                    name: "State".into(),
                    values: vec!["Default".into(), "Hover".into()],
                },
            ],
            members: members.values().copied().collect(),
            default_variant: members[&("S", "Default")],
        },
    );
    (lib, set_id, size_prop, state_prop, members)
}

#[test]
fn two_axis_variant_selection_picks_exact_member() {
    let mut scene = Scene::new();
    let (lib, set_id, size_prop, state_prop, _members) = two_axis_set(&mut scene);
    // Select Size=M, State=Hover → the "M/Hover" member.
    let inst = InstanceNode {
        component: set_id,
        overrides: Vec::new(),
        prop_values: BTreeMap::from([
            (size_prop, VarValue::String { value: "M".into() }),
            (
                state_prop,
                VarValue::String {
                    value: "Hover".into(),
                },
            ),
        ]),
        derived: Vec::new(),
        local_size: [40.0, 40.0],
    };
    let expanded = expand_instance(&scene, &lib, &inst);
    let text = expanded.iter().find_map(|e| match &e.node.data {
        NodeData::Text(t) => Some(t.content.clone()),
        _ => None,
    });
    assert_eq!(text.as_deref(), Some("M/Hover"));
}

#[test]
fn partial_variant_selection_falls_back_to_best_match() {
    // A set with no member matching EVERY selected axis: select Size=M but ask
    // for a State the set lacks ("Pressed"). The closest member (agreeing on
    // Size=M, the one axis that does match) must be chosen — Figma renders the
    // nearest variant, never nothing. Both M members agree on Size; `find` order
    // (members insertion order) breaks the tie to the first M member.
    let mut scene = Scene::new();
    let (lib, set_id, size_prop, state_prop, _members) = two_axis_set(&mut scene);
    let inst = InstanceNode {
        component: set_id,
        overrides: Vec::new(),
        prop_values: BTreeMap::from([
            (size_prop, VarValue::String { value: "M".into() }),
            (
                state_prop,
                VarValue::String {
                    value: "Pressed".into(), // no member has this state
                },
            ),
        ]),
        derived: Vec::new(),
        local_size: [40.0, 40.0],
    };
    let expanded = expand_instance(&scene, &lib, &inst);
    let text = expanded
        .iter()
        .find_map(|e| match &e.node.data {
            NodeData::Text(t) => Some(t.content.clone()),
            _ => None,
        })
        .expect("a closest variant must expand, not nothing");
    // The chosen member must be an "M/*" member (Size axis agreed), not the
    // default S/Default — proving best-partial-match beats the blind default.
    assert!(
        text.starts_with("M/"),
        "best-partial-match should pick an M-size variant, got {text:?}"
    );
}

#[test]
fn unrelated_variant_selection_uses_default() {
    // A selection whose axis VALUES match no member on any axis falls back to the
    // set default (not a random partial match), because no axis agrees.
    let mut scene = Scene::new();
    let (lib, set_id, size_prop, _state_prop, members) = two_axis_set(&mut scene);
    let inst = InstanceNode {
        component: set_id,
        overrides: Vec::new(),
        prop_values: BTreeMap::from([(
            size_prop,
            VarValue::String {
                value: "XXL".into(), // no member has this size
            },
        )]),
        derived: Vec::new(),
        local_size: [40.0, 40.0],
    };
    let expanded = expand_instance(&scene, &lib, &inst);
    let text = expanded.iter().find_map(|e| match &e.node.data {
        NodeData::Text(t) => Some(t.content.clone()),
        _ => None,
    });
    // default_variant is ("S","Default").
    assert_eq!(text.as_deref(), Some("S/Default"));
    let _ = members;
}
