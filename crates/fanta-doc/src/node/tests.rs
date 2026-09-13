//! Unit tests for the [`CanvasNode`] wrapper, the [`NodeData`] sum type, and
//! cross-cutting serde round-trips that span the variant submodules.
//!
//! A child module of [`crate::node`] (`use super::*`), kept in its own file so
//! `node/mod.rs` stays a thin manifest.

use super::*;
use crate::Doc;
use crate::color::Color;
use crate::id::{
    AnimationClipId, ComponentId, ComponentPropId, LinkId, ModeId, ReactionId,
    VariableCollectionId, VariableId, WorkflowNodeId,
};
use crate::op::Operation;
use crate::style::{Fill, Stroke};
use crate::transform::Transform2D;
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
    let text_path = CanvasNode::new(NodeData::TextPath(TextPathNode::new(
        crate::PathData::rect(0.0, 0.0, 10.0, 10.0),
        "hi",
    )));
    assert_eq!(text_path.name, "Text on Path");
}

#[test]
fn font_variations_pack_tags_and_round_trip() {
    // A 4-char tag packs big-endian; short tags pad with spaces.
    assert_eq!(FontVariation::new("wght", 350.0).axis_tag(), 0x7767_6874);
    assert_eq!(FontVariation::new("wd", 75.0).axis_tag(), 0x7764_2020);

    // Empty variations are skipped (old docs byte-identical); a set survives a
    // JSON round-trip.
    let plain = TextNode::new("Hi", 100.0, 20.0);
    assert!(plain.style.font_variations.is_empty());
    let s = serde_json::to_string(&plain.style).unwrap();
    assert!(
        !s.contains("font_variations"),
        "empty variations skipped: {s}"
    );

    let mut style = TextStyle::default();
    style.font_variations = vec![
        FontVariation::new("wght", 350.0),
        FontVariation::new("wdth", 87.5),
    ];
    let j = serde_json::to_value(&style).unwrap();
    assert_eq!(j["font_variations"][0]["axis"], "wght");
    assert_eq!(j["font_variations"][1]["value"], 87.5);
    let back: TextStyle = serde_json::from_value(j).unwrap();
    assert_eq!(back.font_variations, style.font_variations);
}

#[test]
fn boolean_node_round_trips_and_tags_as_boolean() {
    // A boolean node carries its op + inherited paint, tags as "boolean", is a
    // child-accepting container, and survives a JSON round-trip.
    let mut b = BooleanNode {
        op: BooleanOp::Subtract,
        ..BooleanNode::default()
    };
    b.fills.push(Fill::solid(Color::rgb(1, 2, 3)));
    let node = CanvasNode::new(NodeData::Boolean(b));
    assert_eq!(node.name, "Boolean");
    assert_eq!(node.data.kind_tag(), "boolean");
    assert!(node.can_have_children(), "boolean accepts operand children");
    assert!(node.data.strokes().is_some(), "boolean exposes strokes");

    let j = serde_json::to_value(&node).unwrap();
    assert_eq!(j["type"], "boolean");
    assert_eq!(j["op"], "subtract");
    let back: CanvasNode = serde_json::from_value(j).unwrap();
    assert_eq!(back, node);

    // A default (Union, no paint) boolean stays compact: op is present but the
    // empty fill/stroke vectors are skipped.
    let plain = CanvasNode::new(NodeData::Boolean(BooleanNode::default()));
    let s = serde_json::to_string(&plain).unwrap();
    assert!(!s.contains("fills"), "empty fills skipped: {s}");
    assert!(!s.contains("strokes"), "empty strokes skipped: {s}");
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
fn text_path_start_validates_without_panicking_and_rejects_invalid_json() {
    let start = TextPathStart::new(3, 0.25).expect("valid normalized start");
    assert_eq!(start.segment(), 3);
    assert_eq!(start.position(), 0.25);
    assert_eq!(start.with_segment(8).segment(), 8);
    assert_eq!(
        start
            .with_position(1.0)
            .expect("inclusive endpoint")
            .position(),
        1.0
    );
    assert!(TextPathStart::new(0, -0.01).is_none());
    assert!(TextPathStart::new(0, 1.01).is_none());
    assert!(TextPathStart::new(0, f64::NAN).is_none());
    assert!(start.with_position(f64::INFINITY).is_none());

    assert!(serde_json::from_str::<TextPathStart>(r#"{"segment":0,"position":-0.1}"#).is_err());
    assert!(serde_json::from_str::<TextPathStart>(r#"{"segment":0,"position":1.1}"#).is_err());
    let defaulted: TextPathStart = serde_json::from_str("{}").expect("missing fields default");
    assert_eq!(defaulted, TextPathStart::DEFAULT);
}

#[test]
fn text_path_node_round_trips_with_geometry_text_and_placement() {
    let mut path = crate::PathData::new();
    path.move_to(1.0, 2.0)
        .quad_to(4.0, 8.0, 10.0, 2.0)
        .cubic_to(12.0, -1.0, 16.0, -1.0, 18.0, 2.0)
        .close();
    let mut text_path = TextPathNode::new(path, "Path text");
    text_path.style.font_family = "Test Sans".to_owned();
    text_path.style.size_px = 22.0;
    text_path.style.weight = 600;
    text_path.style_runs.push(TextStyleRun {
        start: 5,
        end: 9,
        style: TextStyle {
            italic: true,
            color: Color::rgb(0x11, 0x22, 0x33),
            ..TextStyle::default()
        },
    });
    text_path.start = TextPathStart::new(1, 0.375).expect("valid start");
    text_path.alignment = TextPathAlignment::Center;
    text_path.direction = TextPathDirection::Reverse;
    text_path.side = TextPathSide::Flipped;

    let node = CanvasNode::new(NodeData::TextPath(text_path.clone()));
    let value = serde_json::to_value(&node).expect("TextPath serializes");
    assert_eq!(value["type"], "text_path");
    assert_eq!(value["start"]["segment"], 1);
    assert_eq!(value["start"]["position"], 0.375);
    assert_eq!(value["alignment"], "center");
    assert_eq!(value["direction"], "reverse");
    assert_eq!(value["side"], "flipped");

    let restored: CanvasNode = serde_json::from_value(value).expect("TextPath deserializes");
    assert_eq!(restored.data, NodeData::TextPath(text_path));
}

#[test]
fn text_path_defaults_are_omitted_and_restore_canonically() {
    let text_path = TextPathNode::new(crate::PathData::rect(0.0, 0.0, 20.0, 10.0), "Defaults");
    let value = serde_json::to_value(&text_path).expect("TextPath serializes");
    for key in ["start", "alignment", "direction", "side"] {
        assert!(value.get(key).is_none(), "default `{key}` stays omitted");
    }

    let restored: TextPathNode = serde_json::from_value(value).expect("defaults deserialize");
    assert_eq!(restored.start, TextPathStart::DEFAULT);
    assert_eq!(restored.alignment, TextPathAlignment::Start);
    assert_eq!(restored.direction, TextPathDirection::Forward);
    assert_eq!(restored.side, TextPathSide::Default);
}

#[test]
fn text_path_local_bounds_conservatively_include_shaped_ink_band() {
    let mut path = crate::PathData::new();
    path.move_to(0.0, 0.0).line_to(100.0, 0.0);
    let mut text_path = TextPathNode::new(path, "Baseline");
    text_path.style.size_px = 20.0;

    let bounds = NodeData::TextPath(text_path)
        .local_bounds()
        .expect("text path bounds");
    assert_eq!(bounds.min_x, -80.0);
    assert_eq!(bounds.max_x, 180.0);
    assert_eq!(bounds.min_y, -80.0);
    assert_eq!(bounds.max_y, 80.0);
}

#[test]
fn replacing_vector_with_text_path_keeps_identity_and_undo_redo() {
    let mut doc = Doc::new();
    let vector = VectorNode {
        path: crate::PathData::rect(0.0, 0.0, 40.0, 20.0),
        ..VectorNode::default()
    };
    let path = vector.path.clone();
    let mut node = CanvasNode::new(NodeData::Vector(vector));
    node.name = "Baseline".to_owned();
    node.transform = crate::Transform2D::translation(12.0, 24.0);
    node.meta = serde_json::json!({"source": "selected-vector"});
    let id = node.id;
    doc.apply(Operation::create_node(node))
        .expect("create vector");

    let wrapper_before = doc.scene.get(id).expect("vector exists").clone();

    let old = doc.scene.get(id).expect("vector exists").data.clone();
    let text_path = TextPathNode::new(path, "Same identity");
    doc.apply(Operation::ReplaceData {
        id,
        old: Box::new(old.clone()),
        new: Box::new(NodeData::TextPath(text_path.clone())),
    })
    .expect("replace vector data");
    assert!(matches!(
        doc.scene.get(id).map(|node| &node.data),
        Some(NodeData::TextPath(current)) if current == &text_path
    ));
    let replaced = doc.scene.get(id).expect("same node after replace");
    assert_eq!(replaced.id, wrapper_before.id);
    assert_eq!(replaced.name, wrapper_before.name);
    assert_eq!(replaced.transform, wrapper_before.transform);
    assert_eq!(replaced.meta, wrapper_before.meta);

    assert!(doc.undo().expect("undo replace"));
    assert_eq!(&doc.scene.get(id).expect("same node after undo").data, &old);
    assert!(doc.redo().expect("redo replace"));
    assert_eq!(
        &doc.scene.get(id).expect("same node after redo").data,
        &NodeData::TextPath(text_path)
    );
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
        extra_actions: Vec::new(),
        transition: Some(Transition {
            style: TransitionStyle::Dissolve,
            duration_ms: 200,
            easing: Easing::EaseOut,
        }),
        animation: Some(PrototypeAnimation {
            clip: AnimationClipId::from_u128(12),
            delay_ms: 75,
        }),
    });
    let encoded = serde_json::to_string(&n).unwrap();
    assert!(encoded.contains("\"animation\""));
    assert!(encoded.contains("\"delay_ms\":75"));
    let back: CanvasNode = serde_json::from_str(&encoded).unwrap();
    assert_eq!(back.bindings, n.bindings);
    assert_eq!(back.reactions, n.reactions);

    let legacy: Reaction = serde_json::from_value(serde_json::json!({
        "id": ReactionId::from_u128(10),
        "trigger": { "kind": "click" },
        "action": { "kind": "back" }
    }))
    .unwrap();
    assert!(legacy.transition.is_none());
    assert!(legacy.animation.is_none());
    assert!(legacy.extra_actions.is_empty());
}

/// PR-5 serde pin: `extra_actions` is additive. A reaction serialized BEFORE
/// the field existed must re-serialize to the exact same bytes after a load —
/// the guarantee that shipping multi-action support does not rewrite (or even
/// re-noise) documents that never used it.
#[test]
fn reaction_without_extra_actions_round_trips_byte_identical() {
    // The exact JSON envelope a pre-multi-action document stores (field order
    // is declaration order; `serde_json` preserves it). The id is interpolated
    // because its encoding is pinned separately in `id.rs`.
    let pinned = format!(
        concat!(
            "{{\"id\":{id},",
            "\"trigger\":{{\"kind\":\"click\"}},",
            "\"action\":{{\"kind\":\"back\"}},",
            "\"transition\":{{\"style\":{{\"kind\":\"dissolve\"}},",
            "\"duration_ms\":200,\"easing\":\"ease_out\"}}}}"
        ),
        id = serde_json::to_string(&ReactionId::from_u128(4)).unwrap()
    );
    let parsed: Reaction = serde_json::from_str(&pinned).unwrap();
    assert!(parsed.extra_actions.is_empty());
    assert_eq!(
        serde_json::to_string(&parsed).unwrap(),
        pinned,
        "a reaction with no extra actions must serialize byte-identically to the old form"
    );
}

/// PR-5: a reaction that DOES carry extra actions round-trips them, and
/// `actions()` yields the primary first, extras after, in authored order.
#[test]
fn reaction_with_extra_actions_round_trips() {
    let reaction = Reaction {
        id: ReactionId::from_u128(11),
        trigger: Trigger::Click,
        action: Action::SetVariable {
            variable: VariableId::from_u128(21),
            value: VarValue::Boolean { value: true },
        },
        extra_actions: vec![
            Action::Navigate {
                to: NodeId::from_u128(3),
            },
            Action::OpenLink {
                url: "https://example.com".into(),
            },
        ],
        transition: None,
        animation: None,
    };
    let encoded = serde_json::to_string(&reaction).unwrap();
    assert!(encoded.contains("\"extra_actions\""));
    let back: Reaction = serde_json::from_str(&encoded).unwrap();
    assert_eq!(back, reaction);
    let order: Vec<&Action> = back.actions().collect();
    assert_eq!(order.len(), 3);
    assert!(matches!(order[0], Action::SetVariable { .. }));
    assert!(matches!(order[2], Action::OpenLink { .. }));
}

/// PR-6/PR-7: the new transition styles and the spring easing serialize under
/// their own tags and round-trip; existing variants are untouched.
#[test]
fn spring_easing_and_out_transitions_round_trip() {
    let transition = Transition {
        style: TransitionStyle::SlideOut {
            direction: Direction::Left,
        },
        duration_ms: 300,
        easing: Easing::Spring {
            mass: 1.0,
            stiffness: 600.0,
            damping: 15.0,
        },
    };
    let j = serde_json::to_value(&transition).unwrap();
    assert_eq!(j["style"]["kind"], "slide_out");
    assert_eq!(j["easing"]["spring"]["stiffness"], 600.0);
    let back: Transition = serde_json::from_value(j).unwrap();
    assert_eq!(back, transition);

    for style in [
        TransitionStyle::MoveOut {
            direction: Direction::Down,
        },
        TransitionStyle::ScrollAnimate,
    ] {
        let j = serde_json::to_value(style).unwrap();
        let back: TransitionStyle = serde_json::from_value(j).unwrap();
        assert_eq!(back, style);
    }

    for trigger in [
        Trigger::MouseEnter,
        Trigger::MouseLeave,
        Trigger::WhileHovering,
    ] {
        let j = serde_json::to_value(&trigger).unwrap();
        let back: Trigger = serde_json::from_value(j).unwrap();
        assert_eq!(back, trigger);
    }
}

/// The damped-spring sampler is the one shared easing primitive behind both
/// motion clips and present transitions — pin its physical behavior per
/// regime: exact endpoints, under-damped overshoot, over-damped monotonicity.
#[test]
fn spring_progress_regimes_behave_physically() {
    // Endpoints are exact for every preset.
    for (m, k, c) in [
        (1.0, 100.0, 15.0), // gentle (under-damped, mild)
        (1.0, 300.0, 20.0), // quick (under-damped)
        (1.0, 600.0, 15.0), // bouncy (under-damped, strong)
        (1.0, 80.0, 20.0),  // slow (over-damped)
    ] {
        assert_eq!(spring_progress(m, k, c, 0.0), 0.0);
        assert_eq!(spring_progress(m, k, c, 1.0), 1.0);
        // Settled within tolerance just before the end.
        assert!((spring_progress(m, k, c, 0.999) - 1.0).abs() < 1e-3);
    }

    // Bouncy overshoots past 1.0 somewhere mid-flight.
    let bouncy_peak = (1..100)
        .map(|i| spring_progress(1.0, 600.0, 15.0, f64::from(i) / 100.0))
        .fold(f64::MIN, f64::max);
    assert!(
        bouncy_peak > 1.0,
        "bouncy must overshoot, peak {bouncy_peak}"
    );

    // Gentle barely overshoots; slow (over-damped) never does.
    for i in 0..=100 {
        let t = f64::from(i) / 100.0;
        assert!(
            spring_progress(1.0, 100.0, 15.0, t) <= 1.05,
            "gentle at {t}"
        );
        assert!(
            spring_progress(1.0, 80.0, 20.0, t) <= 1.0 + 1e-9,
            "slow at {t}"
        );
    }

    // Degenerate parameters fall back to linear (still monotonic, no NaN).
    assert_eq!(spring_progress(0.0, 100.0, 15.0, 0.5), 0.5);
    assert_eq!(spring_progress(f32::NAN, 100.0, 15.0, 0.25), 0.25);
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
        counter_auto_spacing: true,
        padding: [10.0, 12.0, 10.0, 12.0],
        primary_align: PrimaryAlign::SpaceBetween,
        counter_align: CounterAlign::Stretch,
        primary_sizing: AxisSizing::Hug,
        counter_sizing: AxisSizing::Fixed,
        wrap: true,
        flow_reverse: true,
        child_layout: false,
        reverse_z: true,
        min_size: [Some(50.0), None],
        max_size: [None, Some(300.0)],
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
    assert_eq!(loaded.local_size, None);
    assert!(loaded.strokes.is_empty());
    assert_eq!(loaded.corner_radius, None);
    assert_eq!(loaded.corner_radii, None);

    let sized_plain = GroupNode {
        local_size: Some([120.0, 80.0]),
        ..Default::default()
    };
    let encoded = serde_json::to_string(&sized_plain).unwrap();
    assert!(encoded.contains("local_size"));
    assert_eq!(
        serde_json::from_str::<GroupNode>(&encoded).unwrap(),
        sized_plain
    );
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

#[test]
fn text_truncation_and_paragraph_fields_round_trip_and_default_off() {
    // New fields are skipped at their defaults (byte-stable with old docs) and
    // absent fields load as the defaults.
    let plain = TextNode::new("x", 1.0, 1.0);
    let s = serde_json::to_string(&plain).unwrap();
    for key in [
        "max_lines",
        "truncate",
        "paragraph_spacing",
        "paragraph_indent",
    ] {
        assert!(!s.contains(key), "{key} skipped at default: {s}");
    }
    let loaded: TextNode = serde_json::from_str(&s).unwrap();
    assert_eq!(loaded.max_lines, None);
    assert!(!loaded.truncate);
    assert_eq!(loaded.paragraph_spacing, 0.0);
    assert_eq!(loaded.paragraph_indent, 0.0);

    let mut clamped = TextNode::new("long label", 100.0, 30.0);
    clamped.max_lines = Some(2);
    clamped.truncate = true;
    clamped.paragraph_spacing = 8.0;
    clamped.paragraph_indent = 12.0;
    let j = serde_json::to_string(&clamped).unwrap();
    let back: TextNode = serde_json::from_str(&j).unwrap();
    assert_eq!(back, clamped);
}

#[test]
fn line_height_auto_percent_round_trips_and_defaults_none() {
    let mut style = TextStyle::default();
    let s = serde_json::to_string(&style).unwrap();
    assert!(!s.contains("line_height_auto_percent"), "skipped at None");
    let loaded: TextStyle = serde_json::from_str(&s).unwrap();
    assert_eq!(loaded.line_height_auto_percent, None);

    style.line_height_auto_percent = Some(100.0);
    let j = serde_json::to_string(&style).unwrap();
    let back: TextStyle = serde_json::from_str(&j).unwrap();
    assert_eq!(back.line_height_auto_percent, Some(100.0));
}

#[test]
fn vector_corner_smoothing_round_trips_and_defaults_zero() {
    let mut v = VectorNode::rect_solid(0.0, 0.0, 10.0, 10.0, crate::Color::BLACK);
    let s = serde_json::to_string(&v).unwrap();
    assert!(!s.contains("corner_smoothing"), "skipped at 0.0");
    let loaded: VectorNode = serde_json::from_str(&s).unwrap();
    assert_eq!(loaded.corner_smoothing, 0.0);

    v.corner_smoothing = 0.6;
    let j = serde_json::to_string(&v).unwrap();
    let back: VectorNode = serde_json::from_str(&j).unwrap();
    assert!((back.corner_smoothing - 0.6).abs() < 1e-6);
}

#[test]
fn parametric_shapes_regenerate_their_paths() {
    // Arc: a 90° pie in a 100×100 box tessellates to a closed contour.
    let arc = ParametricShape::Arc {
        start_rad: 0.0,
        sweep_rad: std::f64::consts::FRAC_PI_2,
        inner_ratio: 0.0,
    };
    let p = arc.to_path(100.0, 100.0);
    assert!(!p.segments.is_empty());
    assert_eq!(*p.segments.last().unwrap(), crate::path::PathSegment::Close);

    // Star: a 5-point star has 2·5 = 10 vertices → a Move + 9 Lines + Close,
    // even-odd wound so the overlapping tips fill.
    let star = ParametricShape::Star {
        points: 5,
        inner_ratio: 0.4,
    };
    let sp = star.to_path(80.0, 80.0);
    assert_eq!(
        sp.segments
            .iter()
            .filter(|s| s.end_point().is_some())
            .count(),
        10,
        "5-point star has 10 ring vertices"
    );
    assert_eq!(sp.fill_rule, crate::path::FillRule::EvenOdd);

    // Polygon: a hexagon has 6 vertices.
    let hex = ParametricShape::Polygon { points: 6 };
    let hp = hex.to_path(60.0, 60.0);
    assert_eq!(
        hp.segments
            .iter()
            .filter(|s| s.end_point().is_some())
            .count(),
        6
    );
    // Regeneration is deterministic — scrubbing to the same params reproduces it.
    assert_eq!(
        hp,
        ParametricShape::Polygon { points: 6 }.to_path(60.0, 60.0)
    );
}

#[test]
fn vector_parametric_field_round_trips_and_is_skipped_when_none() {
    // Absent → not on the wire, so pre-parametric docs round-trip byte-identical.
    let plain = VectorNode::rect_solid(0.0, 0.0, 10.0, 10.0, Color::WHITE);
    let s = serde_json::to_string(&plain).unwrap();
    assert!(!s.contains("parametric"), "None skipped: {s}");
    assert_eq!(serde_json::from_str::<VectorNode>(&s).unwrap(), plain);

    // Present → round-trips with a self-describing `shape` tag.
    let mut star = plain;
    star.parametric = Some(ParametricShape::Star {
        points: 6,
        inner_ratio: 0.5,
    });
    let s = serde_json::to_string(&star).unwrap();
    assert!(s.contains("\"shape\":\"star\""), "shape tag present: {s}");
    assert_eq!(serde_json::from_str::<VectorNode>(&s).unwrap(), star);
}

#[test]
fn constraints_apply_on_parent_resize_via_doc_helper() {
    // Build a minimal doc with a parent group and a constrained child.
    let mut doc = Doc::new();
    let mut parent = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([200.0, 100.0]),
        ..Default::default()
    }));
    parent.name = "Frame".into();
    let parent_id = parent.id;
    doc.apply(Operation::create_node(parent)).unwrap();
    doc.add_page(parent_id);
    doc.set_active_page(Some(parent_id));

    // Child positioned at (10, 10), sized conceptually 50x20, with Right constraint.
    let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        50.0,
        20.0,
        Color::BLACK,
    )));
    child.transform = Transform2D::translation(10.0, 10.0);
    child.constraints = Some(Constraints {
        horizontal: ConstraintH::Right,
        vertical: ConstraintV::Top,
    });
    child.name = "RightConstrained".into();
    child.parent = Some(parent_id);
    child.index = doc.scene.next_child_index(Some(parent_id));
    let child_id = child.id;
    doc.apply(Operation::create_node(child)).unwrap();

    // Simulate parent width growing from 200 -> 300.
    // For Right constraint the child's x should shift right by +100.
    let adjusted = doc
        .apply_constraints_for_parent_resize(parent_id, [200.0, 100.0], [300.0, 100.0])
        .expect("apply ok");
    assert_eq!(adjusted, 1, "one child should be adjusted");

    let updated = doc.scene.get(child_id).expect("child exists");
    let comps = updated.transform.to_components();
    // tx/ty at [4],[5] after fix; original ox=10 -> nx=110 for Right
    assert!(
        (comps[4] - 110.0).abs() < 1e-9,
        "right constraint x: got {}",
        comps[4]
    );
    assert!((comps[5] - 10.0).abs() < 1e-9, "top y unchanged");

    // Direct node method sanity
    let before = Transform2D::translation(10.0, 10.0);
    let mut c2 = CanvasNode::new(NodeData::Group(GroupNode::default()));
    c2.transform = before;
    c2.constraints = Some(Constraints {
        horizontal: ConstraintH::Left,
        vertical: ConstraintV::Top,
    });
    let tx = c2
        .apply_constraints([100.0, 50.0], [150.0, 50.0])
        .expect("has constraints");
    let c2x = tx.to_components()[4];
    assert!((c2x - 10.0).abs() < 1e-9, "Left stays put");

    // LeftRight using rough_bounds() for Vector without explicit local_size
    let mut v3 = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        5.0,
        5.0,
        40.0,
        10.0,
        Color::BLACK,
    )));
    v3.transform = Transform2D::translation(10.0, 10.0);
    // deliberately no local_size to exercise rough_bounds fallback
    v3.constraints = Some(Constraints {
        horizontal: ConstraintH::LeftRight,
        vertical: ConstraintV::Top,
    });
    let tx3 = v3
        .apply_constraints([100.0, 50.0], [200.0, 50.0])
        .expect("has constraints");
    let c3x = tx3.to_components()[4];
    // right margin = 100 - (10 + 40) = 50; new_x = 10 + 100 - 50 = 60
    assert!(
        (c3x - 60.0).abs() < 1e-9,
        "LeftRight x via rough_bounds: got {}",
        c3x
    );
}
