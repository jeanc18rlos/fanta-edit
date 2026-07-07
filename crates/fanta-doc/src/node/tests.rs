//! Unit tests for the [`CanvasNode`] wrapper, the [`NodeData`] sum type, and
//! cross-cutting serde round-trips that span the variant submodules.
//!
//! A child module of [`crate::node`] (`use super::*`), kept in its own file so
//! `node/mod.rs` stays a thin manifest.

use super::*;
use crate::color::Color;
use crate::id::{
    ComponentId, ComponentPropId, LinkId, ModeId, ReactionId, VariableCollectionId, VariableId,
    WorkflowNodeId,
};
use crate::style::Stroke;
use crate::value::VarValue;
use smallvec::SmallVec;

#[test]
fn new_node_has_variant_appropriate_default_name() {
    let n = CanvasNode::new(NodeData::Group(GroupNode::default()));
    assert_eq!(n.name, "Group");
    let v = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        10.0,
        10.0,
        Color::BLACK,
    )));
    assert_eq!(v.name, "Shape");
    let t = CanvasNode::new(NodeData::Text(TextNode::new("hi", 100.0, 20.0)));
    assert_eq!(t.name, "Text");
}

#[test]
fn mask_defaults_are_skipped_and_round_trip() {
    // A fresh node is not a mask and defaults to ALPHA — both skipped from
    // serialization so old (mask-free) docs round-trip byte-identical.
    let plain = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        1.0,
        1.0,
        Color::BLACK,
    )));
    assert!(!plain.is_mask);
    assert_eq!(plain.mask_type, MaskType::Alpha);
    let j = serde_json::to_value(&plain).unwrap();
    assert!(
        j.get("is_mask").is_none(),
        "default is_mask must be skipped"
    );
    assert!(
        j.get("mask_type").is_none(),
        "default mask_type must be skipped"
    );

    // A luminance mask serializes both fields and round-trips.
    let mut mask = plain.clone();
    mask.is_mask = true;
    mask.mask_type = MaskType::Luminance;
    let j = serde_json::to_value(&mask).unwrap();
    assert_eq!(j["is_mask"], true);
    assert_eq!(j["mask_type"], "luminance");
    let back: CanvasNode = serde_json::from_value(j).unwrap();
    assert!(back.is_mask);
    assert_eq!(back.mask_type, MaskType::Luminance);

    // An ALPHA mask serializes the flag but skips the default type.
    let mut alpha = plain;
    alpha.is_mask = true;
    let j = serde_json::to_value(&alpha).unwrap();
    assert_eq!(j["is_mask"], true);
    assert!(
        j.get("mask_type").is_none(),
        "ALPHA (default) mask_type stays skipped"
    );
    let back: CanvasNode = serde_json::from_value(j).unwrap();
    assert!(back.is_mask);
    assert_eq!(back.mask_type, MaskType::Alpha);
}

#[test]
fn text_node_round_trips_and_tags_as_text() {
    let mut tn = TextNode::new("Hello, Fantaisa", 240.0, 48.0);
    tn.align = TextAlign::Center;
    tn.style.font_family = "Inter".into();
    tn.style.size_px = 24.0;
    tn.style.weight = 700;
    tn.style.color = Color::rgb(0x11, 0x22, 0x33);
    let node = CanvasNode::new(NodeData::Text(tn.clone()));

    let j = serde_json::to_value(&node).unwrap();
    assert_eq!(
        j["type"], "text",
        "TEXT must serialize with the snake_case tag"
    );
    assert_eq!(j["align"], "center");

    let back: CanvasNode = serde_json::from_value(j).unwrap();
    match back.data {
        NodeData::Text(t) => {
            assert_eq!(t, tn, "text node must survive a JSON round-trip");
            assert_eq!(t.content, "Hello, Fantaisa");
            assert_eq!(t.style.weight, 700);
        }
        _ => panic!("variant mismatch"),
    }
}

#[test]
fn vertical_align_defaults_top_and_round_trips() {
    // Default is Top, and an absent `vertical_align` (old docs) deserializes
    // to Top — back-compat. Non-Top survives a JSON round-trip.
    assert_eq!(VAlign::default(), VAlign::Top);
    let mut tn = TextNode::new("Centered", 100.0, 40.0);
    tn.vertical_align = VAlign::Center;
    let j = serde_json::to_value(&tn).unwrap();
    assert_eq!(j["vertical_align"], "center", "snake_case enum tag");
    let back: TextNode = serde_json::from_value(j).unwrap();
    assert_eq!(back.vertical_align, VAlign::Center);

    // A serialized text node WITHOUT the field still loads (defaults to Top).
    let mut bare = serde_json::to_value(TextNode::new("Old", 10.0, 10.0)).unwrap();
    bare.as_object_mut().unwrap().remove("vertical_align");
    let loaded: TextNode = serde_json::from_value(bare).unwrap();
    assert_eq!(loaded.vertical_align, VAlign::Top);
}

#[test]
fn corner_radii_round_trips_and_is_back_compat() {
    // Independent per-corner radii survive a JSON round-trip, and a vector
    // serialized WITHOUT `corner_radii` (old docs) loads with `None`.
    let mut v = VectorNode::rect_solid(0.0, 0.0, 40.0, 20.0, Color::BLACK);
    v.corner_radii = Some([1.0, 2.0, 3.0, 4.0]);
    let j = serde_json::to_value(&v).unwrap();
    assert_eq!(j["corner_radii"], serde_json::json!([1.0, 2.0, 3.0, 4.0]));
    let back: VectorNode = serde_json::from_value(j).unwrap();
    assert_eq!(back.corner_radii, Some([1.0, 2.0, 3.0, 4.0]));

    // Absent field => None, and a None field is skipped from the output.
    let plain = VectorNode::rect_solid(0.0, 0.0, 10.0, 10.0, Color::WHITE);
    let s = serde_json::to_string(&plain).unwrap();
    assert!(
        !s.contains("corner_radii"),
        "None is skipped for byte-stable round-trip"
    );
    let loaded: VectorNode = serde_json::from_str(&s).unwrap();
    assert_eq!(loaded.corner_radii, None);
}

#[test]
fn text_style_default_matches_body_text() {
    // Mirrors fanta_text::TextStyle::default so conversion at the render
    // boundary is a no-op for the common case.
    let s = TextStyle::default();
    assert_eq!(s.font_family, "Inter");
    assert_eq!(s.size_px, 16.0);
    assert_eq!(s.weight, 400);
    assert!(!s.italic);
    assert!(!s.underline);
    assert!(!s.strikethrough);
    assert_eq!(s.color, Color::BLACK);
    assert_eq!(s.line_height, 1.2);
}

#[test]
fn node_data_tag_is_snake_case_in_json() {
    let n = CanvasNode::new(NodeData::AiArtifact(AiArtifactNode {
        local_size: [256.0, 256.0],
        prompt: "a cat".into(),
        model: "flux-pro".into(),
        params: serde_json::json!({"steps": 30}),
        inputs: vec![],
        lineage_parent: None,
        output: None,
        status: GenerationStatus::Pending,
        seed: None,
    }));
    let j = serde_json::to_value(&n).unwrap();
    assert_eq!(j["type"], "ai_artifact");
    assert_eq!(j["prompt"], "a cat");
}

#[test]
fn flags_are_empty_by_default_and_serialize_compact() {
    let n = CanvasNode::new(NodeData::Group(GroupNode::default()));
    assert_eq!(n.flags, NodeFlags::empty());
    let j = serde_json::to_string(&n).unwrap();
    // Empty flags are skipped from the output (skip_serializing_if).
    assert!(!j.contains("\"flags\""));
}

#[test]
fn meta_round_trips_through_json_when_set() {
    let mut n = CanvasNode::new(NodeData::Group(GroupNode::default()));
    n.meta = serde_json::json!({"plugin_key": "plugin_value"});
    let j = serde_json::to_string(&n).unwrap();
    let back: CanvasNode = serde_json::from_str(&j).unwrap();
    assert_eq!(back.meta["plugin_key"], "plugin_value");
}

#[test]
fn ai_artifact_round_trips_lineage() {
    let parent_id = NodeId::new();
    let n = CanvasNode::new(NodeData::AiArtifact(AiArtifactNode {
        local_size: [512.0, 512.0],
        prompt: "rolling hills".into(),
        model: "sdxl".into(),
        params: serde_json::json!({}),
        inputs: vec![],
        lineage_parent: Some(parent_id),
        output: None,
        status: GenerationStatus::Done,
        seed: Some(42),
    }));
    let j = serde_json::to_string(&n).unwrap();
    let back: CanvasNode = serde_json::from_str(&j).unwrap();
    match back.data {
        NodeData::AiArtifact(a) => {
            assert_eq!(a.lineage_parent, Some(parent_id));
            assert_eq!(a.seed, Some(42));
            assert_eq!(a.status, GenerationStatus::Done);
        }
        _ => panic!("variant mismatch"),
    }
}

#[test]
fn node_graph_round_trips() {
    let wn1 = WorkflowNodeId::new();
    let wn2 = WorkflowNodeId::new();
    let mut graph = NodeGraph::default();
    graph.nodes.insert(
        wn1,
        WorkflowNode {
            id: wn1,
            kind: "image.load".into(),
            position: [0.0, 0.0],
            params: serde_json::json!({"path": "foo.png"}),
        },
    );
    graph.nodes.insert(
        wn2,
        WorkflowNode {
            id: wn2,
            kind: "image.resize".into(),
            position: [200.0, 0.0],
            params: serde_json::json!({"w": 512, "h": 512}),
        },
    );
    graph.links.push(Link {
        id: LinkId::new(),
        from_node: wn1,
        from_port: "image".into(),
        to_node: wn2,
        to_port: "image".into(),
    });
    graph.output = Some(wn2);

    let n = CanvasNode::new(NodeData::NodeGraph(NodeGraphNode {
        local_size: [400.0, 400.0],
        graph,
        preview: None,
    }));
    let j = serde_json::to_string(&n).unwrap();
    let back: CanvasNode = serde_json::from_str(&j).unwrap();
    match back.data {
        NodeData::NodeGraph(ng) => {
            assert_eq!(ng.graph.nodes.len(), 2);
            assert_eq!(ng.graph.links.len(), 1);
            assert_eq!(ng.graph.output, Some(wn2));
        }
        _ => panic!("variant mismatch"),
    }
}

#[test]
fn instance_node_tags_and_does_not_accept_children() {
    let inst = InstanceNode {
        component: ComponentId::from_u128(1),
        overrides: vec![Override {
            target_path: SmallVec::from_iter([NodeId::from_u128(7)]),
            target_prop: BoundProp::TextContent,
            value: OverrideValue::Text {
                value: "Save".into(),
            },
        }],
        prop_values: BTreeMap::from([(
            ComponentPropId::from_u128(2),
            VarValue::Boolean { value: true },
        )]),
        derived: Vec::new(),
        local_size: [120.0, 40.0],
    };
    let n = CanvasNode::new(NodeData::Instance(inst.clone()));
    assert_eq!(n.name, "Instance");
    // Instances never accept real children — their subtree is virtual.
    assert!(!n.can_have_children());
    let j = serde_json::to_value(&n).unwrap();
    assert_eq!(j["type"], "instance");
    let back: CanvasNode = serde_json::from_value(j).unwrap();
    match back.data {
        NodeData::Instance(i) => assert_eq!(i, inst),
        _ => panic!("variant mismatch"),
    }
}

#[test]
fn bindings_and_reactions_round_trip_and_skip_when_empty() {
    let mut n = CanvasNode::new(NodeData::Group(GroupNode::default()));
    // Empty: neither field appears.
    let j = serde_json::to_string(&n).unwrap();
    assert!(!j.contains("bindings"));
    assert!(!j.contains("reactions"));

    n.bindings
        .insert(BoundProp::Opacity, VariableId::from_u128(5));
    n.reactions.push(Reaction {
        id: ReactionId::from_u128(9),
        trigger: Trigger::Click,
        action: Action::Navigate {
            to: NodeId::from_u128(3),
        },
        transition: Some(Transition {
            style: TransitionStyle::Dissolve,
            duration_ms: 200,
            easing: Easing::EaseOut,
        }),
    });
    let back: CanvasNode = serde_json::from_str(&serde_json::to_string(&n).unwrap()).unwrap();
    assert_eq!(back.bindings, n.bindings);
    assert_eq!(back.reactions, n.reactions);
}

#[test]
fn explicit_modes_round_trip_on_frame() {
    let g = GroupNode {
        clip_size: Some([100.0, 100.0]),
        background: None,
        explicit_modes: BTreeMap::from([(
            VariableCollectionId::from_u128(1),
            ModeId::from_u128(2),
        )]),
        ..Default::default()
    };
    let n = CanvasNode::new(NodeData::Group(g.clone()));
    let back: CanvasNode = serde_json::from_str(&serde_json::to_string(&n).unwrap()).unwrap();
    match back.data {
        NodeData::Group(bg) => assert_eq!(bg, g),
        _ => panic!("variant mismatch"),
    }
}

#[test]
fn auto_layout_round_trips_and_is_back_compat() {
    // A frame carrying a full AutoLayout survives a JSON round-trip with the
    // enums tagged snake_case, and a group serialized WITHOUT `auto_layout`
    // (old docs) loads with `None`.
    let al = AutoLayout {
        mode: LayoutMode::Vertical,
        spacing: 8.0,
        counter_spacing: 4.0,
        padding: [10.0, 12.0, 10.0, 12.0],
        primary_align: PrimaryAlign::SpaceBetween,
        counter_align: CounterAlign::Stretch,
        primary_sizing: AxisSizing::Hug,
        counter_sizing: AxisSizing::Fixed,
        wrap: true,
        flow_reverse: true,
        child_layout: false,
        reverse_z: true,
    };
    let g = GroupNode {
        clip_size: Some([100.0, 200.0]),
        auto_layout: Some(al),
        ..Default::default()
    };
    let j = serde_json::to_value(&g).unwrap();
    assert_eq!(j["auto_layout"]["mode"], "vertical");
    assert_eq!(j["auto_layout"]["primary_align"], "space_between");
    assert_eq!(j["auto_layout"]["counter_align"], "stretch");
    assert_eq!(j["auto_layout"]["primary_sizing"], "hug");
    assert_eq!(j["auto_layout"]["counter_sizing"], "fixed");
    let back: GroupNode = serde_json::from_value(j).unwrap();
    assert_eq!(back.auto_layout, Some(al));

    // Absent field => None, and a None auto_layout is skipped from output.
    let plain = GroupNode::default();
    let s = serde_json::to_string(&plain).unwrap();
    assert!(
        !s.contains("auto_layout"),
        "None is skipped for byte-stable round-trip"
    );
    let loaded: GroupNode = serde_json::from_str(&s).unwrap();
    assert_eq!(loaded.auto_layout, None);
}

#[test]
fn group_stroke_and_corner_radius_round_trip_and_are_back_compat() {
    // A frame carrying a border (stroke) + rounded corners survives a JSON
    // round-trip, and a group serialized WITHOUT the new fields (old docs)
    // loads with empty strokes / None radii — byte-stable back-compat.
    use crate::style::StrokeAlign;
    let mut strokes: SmallVec<[Stroke; 1]> = SmallVec::new();
    let mut s = Stroke::solid(Color::rgb(10, 20, 30), 2.0);
    s.align = StrokeAlign::Inside;
    strokes.push(s);
    let g = GroupNode {
        clip_size: Some([100.0, 60.0]),
        background: Some(crate::style::Fill::solid(Color::rgb(255, 255, 255))),
        strokes: strokes.clone(),
        corner_radius: Some(8.0),
        corner_radii: Some([4.0, 8.0, 8.0, 4.0]),
        ..Default::default()
    };
    let back: GroupNode = serde_json::from_str(&serde_json::to_string(&g).unwrap()).unwrap();
    assert_eq!(back, g);

    // An old GroupNode (no strokes / corner fields) deserializes cleanly.
    let plain = GroupNode::default();
    let out = serde_json::to_string(&plain).unwrap();
    assert!(!out.contains("strokes"), "empty strokes skipped");
    assert!(!out.contains("corner_radius"), "None corner_radius skipped");
    assert!(!out.contains("corner_radii"), "None corner_radii skipped");
    let legacy = r#"{"clip_size":[10.0,10.0]}"#;
    let loaded: GroupNode = serde_json::from_str(legacy).unwrap();
    assert!(loaded.strokes.is_empty());
    assert_eq!(loaded.corner_radius, None);
    assert_eq!(loaded.corner_radii, None);
}

#[test]
fn layout_child_round_trips_on_wrapper_and_skips_when_absent() {
    // A non-trivial LayoutChild (grow + alignSelf) round-trips on the node
    // wrapper; a node without one serializes nothing.
    let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        10.0,
        10.0,
        Color::BLACK,
    )));
    // No layout_child by default => field skipped.
    assert!(!serde_json::to_string(&n).unwrap().contains("layout_child"));

    let lc = LayoutChild {
        grow: 1.0,
        absolute: false,
        align_self: Some(CounterAlign::Center),
    };
    assert!(!lc.is_trivial());
    n.layout_child = Some(lc);
    let j = serde_json::to_value(&n).unwrap();
    assert_eq!(j["layout_child"]["grow"], 1.0);
    assert_eq!(j["layout_child"]["align_self"], "center");
    let back: CanvasNode = serde_json::from_value(j).unwrap();
    assert_eq!(back.layout_child, Some(lc));

    // A trivial LayoutChild reports as such (the importer drops it to None).
    assert!(
        LayoutChild {
            grow: 0.0,
            absolute: false,
            align_self: None
        }
        .is_trivial()
    );
}

#[test]
fn text_auto_resize_round_trips_and_defaults_none() {
    // Default is None and an absent field (old docs) deserializes to None.
    assert_eq!(TextAutoResize::default(), TextAutoResize::None);
    let mut t = TextNode::new("Edit", 40.0, 20.0);
    t.auto_resize = TextAutoResize::WidthAndHeight;
    let j = serde_json::to_value(&t).unwrap();
    assert_eq!(j["auto_resize"], "width_and_height");
    let back: TextNode = serde_json::from_value(j).unwrap();
    assert_eq!(back.auto_resize, TextAutoResize::WidthAndHeight);

    // None is skipped (byte-stable) and a node missing the field loads None.
    let plain = TextNode::new("x", 1.0, 1.0);
    let s = serde_json::to_string(&plain).unwrap();
    assert!(!s.contains("auto_resize"));
    let loaded: TextNode = serde_json::from_str(&s).unwrap();
    assert_eq!(loaded.auto_resize, TextAutoResize::None);
}
