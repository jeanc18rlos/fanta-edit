//! Components, variants, instances, and component sets.

use super::*;

// =============================================================================
// Task 2: Components & variants
// =============================================================================

#[test]
fn symbol_on_canvas_maps_to_component_def_and_stays_on_page() {
    // A SYMBOL (main component) with a child rect becomes a ComponentDef, but
    // when it is visible on a Figma page it must stay there. Figma renders
    // page-level component masters beside frames, so the read-only viewer should
    // not move them away.
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".into())),
                ("name", KiwiValue::String("Page 1".to_owned())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("Button".to_owned())),
                ("size", vector(120.0, 40.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("parentIndex", parent_index(0, 2)),
                ("type", KiwiValue::Enum("RECTANGLE".into())),
                ("size", vector(120.0, 40.0)),
                (
                    "fillPaints",
                    KiwiValue::array(vec![solid_paint(0.0, 0.0, 1.0, 1.0)]),
                ),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.components, 1, "one ComponentDef built");
    assert_eq!(doc.components.defs.len(), 1);

    let def = doc.components.defs.values().next().unwrap();
    assert_eq!(def.name, "Button");
    // The master root keeps its child rect.
    let kids = doc.scene.children_of(Some(def.root));
    assert_eq!(kids.len(), 1);
    assert!(matches!(
        doc.scene.get(kids[0]).unwrap().data,
        NodeData::Vector(_)
    ));

    let design_page = doc
        .pages()
        .iter()
        .copied()
        .find(|p| doc.page_name(*p) == Some("Page 1"))
        .expect("design page exists");
    assert_eq!(doc.scene.get(def.root).unwrap().parent, Some(design_page));
    assert_eq!(
        report.masters_kept_in_place, 1,
        "visible page-level master was not relocated"
    );
    assert_eq!(doc.active_page(), Some(design_page));
    doc.scene.validate().unwrap();
}

#[test]
fn symbol_embedded_in_design_frame_renders_in_place() {
    // Regression for the Action-Bar card's missing "Variants" example bars: a
    // SYMBOL (component master) that is NESTED inside a design FRAME (not a child
    // of a page root) is a documentation example meant to render IN PLACE — Figma
    // shows component masters at their canvas location. Such a master must NOT be
    // relocated to the hidden Components page (which empties the frame), yet it
    // must still become a ComponentDef so instances expand.
    //
    // Page 1 → "Card" FRAME → "Example" SYMBOL → child rect.
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 0)),
                ("type", KiwiValue::Enum("DOCUMENT".into())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("parentIndex", parent_index(0, 0)),
                ("type", KiwiValue::Enum("CANVAS".into())),
                ("name", KiwiValue::String("Page 1".to_owned())),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("FRAME".into())),
                ("name", KiwiValue::String("Card".to_owned())),
                ("size", vector(300.0, 200.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("parentIndex", parent_index(0, 2)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("Example".to_owned())),
                ("size", vector(120.0, 40.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 4)),
                ("parentIndex", parent_index(0, 3)),
                ("type", KiwiValue::Enum("RECTANGLE".into())),
                ("size", vector(120.0, 40.0)),
                (
                    "fillPaints",
                    KiwiValue::array(vec![solid_paint(0.0, 0.0, 1.0, 1.0)]),
                ),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    // Still a real component definition.
    assert_eq!(
        report.components, 1,
        "the embedded master still defines a component"
    );
    assert_eq!(
        report.masters_kept_in_place, 1,
        "the embedded master was not relocated"
    );

    let def = doc.components.defs.values().next().unwrap();
    assert_eq!(def.name, "Example");

    // The design page and the "Card" frame.
    let design_page = doc
        .pages()
        .iter()
        .copied()
        .find(|p| doc.page_name(*p) == Some("Page 1"))
        .expect("design page exists");
    let card = doc
        .scene
        .descendants_of(design_page)
        .find(|id| {
            doc.scene
                .get(*id)
                .map(|n| n.name == "Card")
                .unwrap_or(false)
        })
        .expect("Card frame present");

    // The master root is STILL a child of the design "Card" frame — it renders
    // in place — not relocated under the hidden Components page.
    assert_eq!(
        doc.scene.get(def.root).unwrap().parent,
        Some(card),
        "embedded master must stay inside its design frame"
    );
    let comp_page = doc
        .pages()
        .iter()
        .copied()
        .find(|p| doc.page_name(*p) == Some("Components"));
    if let Some(cp) = comp_page {
        assert!(
            !doc.scene.ancestors_of(def.root).any(|a| a.id == cp),
            "embedded master must NOT live under the Components page"
        );
    }
    // And its content survived (so the frame is not empty).
    assert_eq!(doc.scene.children_of(Some(def.root)).len(), 1);
    doc.scene.validate().unwrap();
}

#[test]
fn instance_maps_to_instance_node_resolving_component() {
    // A SYMBOL plus an INSTANCE referencing it: the instance becomes a
    // NodeData::Instance whose `component` resolves to the symbol's ComponentId.
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("Button".to_owned())),
                ("size", vector(120.0, 40.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("type", KiwiValue::Enum("INSTANCE".into())),
                ("name", KiwiValue::String("Button instance".to_owned())),
                ("size", vector(120.0, 40.0)),
                ("symbolData", symbol_data(0, 1)),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.instances, 1);
    assert_eq!(report.components, 1);

    let inst_id = doc
        .scene
        .roots()
        .iter()
        .flat_map(|r| doc.scene.descendants_of(*r))
        .find(|id| doc.scene.get(*id).unwrap().name == "Button instance")
        .expect("instance present");
    let comp_id = doc.components.defs.keys().copied().next().unwrap();
    match &doc.scene.get(inst_id).unwrap().data {
        NodeData::Instance(i) => {
            assert_eq!(
                i.component, comp_id,
                "instance resolves to the symbol's def"
            );
            assert_eq!(i.local_size, [120.0, 40.0]);
        }
        other => panic!("INSTANCE should map to NodeData::Instance, got {other:?}"),
    }
    doc.scene.validate().unwrap();
}

#[test]
fn instance_children_are_dropped_as_virtual_subtree() {
    // An INSTANCE's child nodes are virtual (reproduced by expand_instance), so
    // they must not be stored — the instance has no real scene children.
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("Card".to_owned())),
                ("size", vector(100.0, 100.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("type", KiwiValue::Enum("INSTANCE".into())),
                ("name", KiwiValue::String("Card instance".to_owned())),
                ("size", vector(100.0, 100.0)),
                ("symbolData", symbol_data(0, 1)),
            ],
        ),
        // A rectangle nested under the instance — must be dropped.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("parentIndex", parent_index(0, 2)),
                ("type", KiwiValue::Enum("RECTANGLE".into())),
                ("size", vector(20.0, 20.0)),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(
        report.instance_children_dropped, 1,
        "the nested rect is dropped"
    );

    let inst_id = doc
        .scene
        .roots()
        .iter()
        .flat_map(|r| doc.scene.descendants_of(*r))
        .find(|id| doc.scene.get(*id).unwrap().name == "Card instance")
        .expect("instance present");
    // Instance has no real children (its subtree is virtual).
    assert_eq!(doc.scene.children_of(Some(inst_id)).len(), 0);
    doc.scene.validate().unwrap();
}

#[test]
fn state_group_symbol_maps_to_component_set_with_variant_members() {
    // A state-group SYMBOL with two variant-named COMPONENT children becomes a
    // ComponentSet whose members carry parsed axis values.
    let fig = doc_from(vec![
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("Button".to_owned())),
                ("isStateGroup", KiwiValue::Bool(true)),
                ("size", vector(120.0, 40.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                (
                    "name",
                    KiwiValue::String("State=Default, Size=Large".to_owned()),
                ),
                ("size", vector(120.0, 40.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                (
                    "name",
                    KiwiValue::String("State=Hover, Size=Large".to_owned()),
                ),
                ("size", vector(120.0, 40.0)),
            ],
        ),
    ]);
    let (doc, report, _) = fig_to_doc(&fig).unwrap();
    assert_eq!(report.component_sets, 1);
    assert_eq!(report.components, 2, "two variant component defs");

    let set = doc.components.sets.values().next().unwrap();
    assert_eq!(set.name, "Button");
    assert_eq!(set.members.len(), 2);
    // Axes parsed from variant names: State {Default, Hover}, Size {Large}.
    let state_axis = set
        .axes
        .iter()
        .find(|a| a.name == "State")
        .expect("State axis");
    assert!(state_axis.values.contains(&"Default".to_owned()));
    assert!(state_axis.values.contains(&"Hover".to_owned()));
    let size_axis = set
        .axes
        .iter()
        .find(|a| a.name == "Size")
        .expect("Size axis");
    assert_eq!(size_axis.values, vec!["Large".to_owned()]);

    // Each member def links back to the set with its axis values.
    for &m in &set.members {
        let def = &doc.components.defs[&m];
        let membership = def.variant_of.as_ref().expect("variant membership");
        assert_eq!(membership.set, set.id);
        assert!(membership.axis_values.contains_key("State"));
        assert!(membership.axis_values.contains_key("Size"));
    }
    doc.scene.validate().unwrap();
}

#[test]
fn instance_of_a_component_set_resolves_to_a_member_and_expands() {
    // An INSTANCE whose `symbolData.symbolID` names the *set* (the state-group
    // SYMBOL guid) — not a member variant — must resolve to the set id, and
    // `expand_instance` must then expand the set's default member's content
    // (non-empty), rather than leaving a placeholder / empty expansion. This is
    // the path the renderer takes for an instance of a variant group.
    let fig = doc_from(vec![
        // The state-group SYMBOL (the set master).
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("Button".to_owned())),
                ("isStateGroup", KiwiValue::Bool(true)),
                ("size", vector(120.0, 40.0)),
            ],
        ),
        // One variant member with a RECTANGLE child, so its expansion is
        // non-empty.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 2)),
                ("parentIndex", parent_index(0, 1)),
                ("type", KiwiValue::Enum("SYMBOL".into())),
                ("name", KiwiValue::String("State=Default".to_owned())),
                ("size", vector(120.0, 40.0)),
            ],
        ),
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 3)),
                ("parentIndex", parent_index(0, 2)),
                ("type", KiwiValue::Enum("RECTANGLE".into())),
                ("name", KiwiValue::String("bg".to_owned())),
                ("size", vector(120.0, 40.0)),
                (
                    "fillPaints",
                    KiwiValue::array(vec![solid_paint(0.1, 0.4, 0.9, 1.0)]),
                ),
            ],
        ),
        // An INSTANCE pointing at the SET guid (0,1), not a member.
        o(
            "NodeChange",
            vec![
                ("guid", guid(0, 4)),
                ("type", KiwiValue::Enum("INSTANCE".into())),
                ("name", KiwiValue::String("Button instance".to_owned())),
                ("symbolData", symbol_data(0, 1)),
                ("size", vector(120.0, 40.0)),
            ],
        ),
    ]);
    let (doc, _report, _) = fig_to_doc(&fig).unwrap();

    // The set exists.
    assert_eq!(doc.components.sets.len(), 1);
    let set_id = *doc.components.sets.keys().next().unwrap();

    // The instance's component resolves to the SET id (not a placeholder/0).
    let inst = doc
        .scene
        .roots()
        .iter()
        .flat_map(|r| doc.scene.descendants_of(*r))
        .find_map(|id| match doc.scene.get(id).map(|n| &n.data) {
            Some(fanta_doc::node::NodeData::Instance(i)) => Some(i.clone()),
            _ => None,
        })
        .expect("instance present");
    assert_eq!(
        inst.component, set_id,
        "instance resolves to the component SET id"
    );

    // Expanding it yields the default member's real content (root + bg rect),
    // not an empty expansion.
    let expanded = fanta_doc::resolve::expand_instance(&doc.scene, &doc.components, &inst);
    assert!(
        expanded.len() >= 2,
        "set instance expands to the default member's content, got {} nodes",
        expanded.len()
    );
    doc.scene.validate().unwrap();
}
