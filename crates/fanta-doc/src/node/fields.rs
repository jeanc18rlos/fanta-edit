//! The known serde field names of every node kind — the vocabulary the
//! authoring pipeline validates hand-written `.fnx` attributes against.
//!
//! `CanvasNode` flattens its [`NodeData`](super::NodeData) variant, which
//! defeats both `deny_unknown_fields` and `serde_ignored`, so unknown-attribute
//! detection must be a key-set diff. The sets here are derived at first use
//! from **maximal samples** — one node per variant with every optional field
//! populated, serialized through the same serde the load path uses — so the
//! table can never drift from the actual schema *as long as the samples stay
//! maximal*. The `maximal_samples_cover_recent_fields` test pins that contract
//! on the fields most recently added; extend it when adding fields.

use super::{
    AiArtifactNode, AudioNode, AutoLayout, BitmapNode, BooleanBakedGeometry, BooleanNode,
    CanvasNode, Constraints, DerivedOverride, EmbedNode, GenerationStatus, GridCell, GridLayout,
    GroupNode, InstanceNode, LayoutChild, MaskType, Model3dNode, NodeData, NodeGraphNode, Override,
    OverrideValue, ParametricShape, Reaction, ScrollBehavior, ScrollDirection, TextAutoResize,
    TextNode, TextPathAlignment, TextPathDirection, TextPathNode, TextPathSide, TextPathStart,
    TextStyle, TextStyleRun, VectorNode, VideoNode,
};
use crate::binding::BoundProp;
use crate::color::Color;
use crate::id::{AssetId, ComponentId, ComponentPropId, NodeId, VariableId};
use crate::style::{Blur, Fill, Shadow, Stroke};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

/// The serde keys a node of `type_tag` (the `NodeData` serde tag, e.g.
/// `"group"`, `"vector"`, `"text"`) can carry, or `None` for an unknown tag.
/// Keys not in this set are either typos or fields from a future schema —
/// callers should warn (with a suggestion), never error: unknown *future*
/// fields must keep riding through untouched.
pub fn known_fields(type_tag: &str) -> Option<&'static BTreeSet<String>> {
    static TABLE: OnceLock<BTreeMap<String, BTreeSet<String>>> = OnceLock::new();
    TABLE.get_or_init(build_table).get(type_tag)
}

fn build_table() -> BTreeMap<String, BTreeSet<String>> {
    let mut table = BTreeMap::new();
    for data in maximal_variants() {
        let node = maximal_node(data);
        let value = serde_json::to_value(&node).expect("maximal node serializes");
        let tag = value
            .get("type")
            .and_then(serde_json::Value::as_str)
            .expect("NodeData is internally tagged")
            .to_owned();
        let keys = value
            .as_object()
            .expect("CanvasNode serializes as an object")
            .keys()
            .cloned()
            .collect();
        table.insert(tag, keys);
    }
    table
}

/// Wrap a variant payload in a `CanvasNode` with every wrapper-level optional
/// field populated, so every wrapper key serializes.
fn maximal_node(data: NodeData) -> CanvasNode {
    let mut node = CanvasNode::new(data);
    node.parent = Some(crate::id::NodeId::new());
    node.transform = crate::transform::Transform2D::translation(1.0, 2.0);
    node.constraints = Some(Constraints::default());
    node.opacity = crate::style::UnitInterval::new(0.5);
    node.blend_mode = crate::style::BlendMode::Multiply;
    node.effects.push(Shadow {
        kind: crate::style::ShadowKind::Drop,
        color: Color::BLACK,
        blur: 4.0,
        spread: 1.0,
        offset: [0.0, 2.0],
        show_behind_node: true,
    });
    node.blurs.push(Blur::layer(4.0));
    node.flags = super::NodeFlags::LOCKED;
    node.is_mask = true;
    node.mask_type = MaskType::Luminance;
    node.scroll_behavior = ScrollBehavior::Fixed;
    node.meta = serde_json::json!({ "k": true });
    node.bindings
        .insert(BoundProp::Opacity, VariableId::from_u128(7));
    node.reactions.push(Reaction {
        id: crate::id::ReactionId::from_u128(1),
        trigger: super::Trigger::Click,
        action: super::Action::Back,
        extra_actions: Vec::new(),
        transition: None,
        animation: None,
    });
    node.layout_child = Some(LayoutChild {
        grid: Some(GridCell::default()),
        grow: 1.0,
        absolute: true,
        align_self: Some(super::CounterAlign::Center),
    });
    node
}

/// One payload per variant with every variant-level optional populated.
/// KEEP MAXIMAL: an unpopulated `Option`/collection here silently turns its
/// field into a false-positive "unknown attribute" for authors.
fn maximal_variants() -> Vec<NodeData> {
    let mut group = GroupNode {
        local_size: Some([10.0, 10.0]),
        clip_size: Some([10.0, 10.0]),
        background: Some(Fill::solid(Color::WHITE)),
        scrollable: true,
        scroll_direction: Some(ScrollDirection::Both),
        scroll_offset: Some([0.0, 5.0]),
        auto_layout: Some(AutoLayout {
            reverse_z: true,
            ..Default::default()
        }),
        grid: Some(GridLayout::default()),
        corner_radius: Some(2.0),
        corner_radii: Some([1.0, 2.0, 3.0, 4.0]),
        corner_smoothing: 0.6,
        ..GroupNode::default()
    };
    group.background_fills.push(Fill::solid(Color::BLACK));
    group.strokes.push(Stroke::solid(Color::BLACK, 1.0));
    group.explicit_modes.insert(
        crate::id::VariableCollectionId::from_u128(1),
        crate::id::ModeId::from_u128(2),
    );

    let mut vector = VectorNode::rect_solid(0.0, 0.0, 10.0, 10.0, Color::BLACK);
    vector.strokes.push(Stroke::solid(Color::BLACK, 1.0));
    vector.local_size = Some([10.0, 10.0]);
    vector.corner_radius = Some(2.0);
    vector.corner_radii = Some([1.0, 2.0, 3.0, 4.0]);
    vector.corner_smoothing = 0.6;
    vector.parametric = Some(ParametricShape::Arc {
        start_rad: 0.0,
        sweep_rad: 1.0,
        inner_ratio: 0.5,
    });

    let mut text = TextNode::new("x", 10.0, 10.0);
    text.style = TextStyle::default();
    text.style_runs.push(TextStyleRun {
        start: 0,
        end: 1,
        style: TextStyle::default(),
    });
    text.auto_resize = TextAutoResize::Height;
    text.max_lines = Some(2);
    text.truncate = true;
    text.paragraph_spacing = 4.0;
    text.paragraph_indent = 2.0;

    let mut text_path = TextPathNode::new(
        crate::path::PathData::rect(0.0, 0.0, 10.0, 10.0),
        "path text",
    );
    text_path.style_runs.push(TextStyleRun {
        start: 0,
        end: 4,
        style: TextStyle::default(),
    });
    text_path.start = TextPathStart::DEFAULT.with_segment(2);
    text_path.alignment = TextPathAlignment::End;
    text_path.direction = TextPathDirection::Reverse;
    text_path.side = TextPathSide::Flipped;

    let asset = AssetId::from_u128(1);
    let node_id = NodeId::from_u128(2);
    let bitmap = BitmapNode {
        asset,
        natural_size: [10, 10],
        local_size: [10.0, 10.0],
        crop: Some([0.0, 0.0, 1.0, 1.0]),
        fit: crate::style::ImageFitMode::Fill,
        tint: Some(Color::WHITE),
    };
    let video = VideoNode {
        asset,
        natural_size: [10, 10],
        local_size: [10.0, 10.0],
        time_range_us: [0, 1_000_000],
        speed: 1.0,
        muted: true,
        volume: 0.5,
        poster_frame_us: Some(0),
        poster: Some(asset),
        fit: crate::style::ImageFitMode::Fill,
    };
    let audio = AudioNode {
        asset,
        local_size: [10.0, 10.0],
        time_range_us: [0, 1_000_000],
        volume: 0.5,
        muted: true,
        waveform_color: Color::BLACK,
    };
    let graph = NodeGraphNode {
        local_size: [10.0, 10.0],
        graph: Default::default(),
        preview: Some(asset),
    };
    let model = Model3dNode {
        asset,
        local_size: [10.0, 10.0],
        camera: Default::default(),
        overrides: serde_json::json!({ "material": true }),
    };
    let artifact = AiArtifactNode {
        local_size: [10.0, 10.0],
        prompt: "sample".into(),
        model: "sample".into(),
        params: serde_json::json!({ "quality": 1 }),
        inputs: vec![node_id],
        lineage_parent: Some(node_id),
        output: Some(asset),
        status: GenerationStatus::Done,
        seed: Some(1),
    };
    let instance = InstanceNode {
        component: ComponentId::from_u128(1),
        overrides: vec![Override {
            target_path: [node_id].into_iter().collect(),
            target_prop: BoundProp::Visible,
            value: OverrideValue::Visible { value: false },
        }],
        prop_values: [(
            ComponentPropId::from_u128(1),
            crate::value::VarValue::Boolean { value: true },
        )]
        .into_iter()
        .collect(),
        derived: vec![DerivedOverride {
            path: [node_id].into_iter().collect(),
            transform: None,
            size: None,
            fills: None,
            path_data: None,
            stroke_path: None,
            stroke_weight: None,
            text: None,
        }],
        local_size: [10.0, 10.0],
    };
    let boolean = BooleanNode {
        op: Default::default(),
        fills: [Fill::solid(Color::BLACK)].into_iter().collect(),
        strokes: [Stroke::solid(Color::WHITE, 1.0)].into_iter().collect(),
        baked: Some(BooleanBakedGeometry {
            vector: vector.clone(),
            source: "sample".into(),
            stroke_outline: true,
        }),
    };
    let embed = EmbedNode {
        local_size: [10.0, 10.0],
        kind: "sample".into(),
        payload: serde_json::json!({ "value": 1 }),
    };
    vec![
        NodeData::Group(group),
        NodeData::Vector(vector),
        NodeData::Text(text),
        NodeData::TextPath(text_path),
        NodeData::Bitmap(bitmap),
        NodeData::Video(video),
        NodeData::Audio(audio),
        NodeData::NodeGraph(graph),
        NodeData::Model3d(model),
        NodeData::AiArtifact(artifact),
        NodeData::Instance(instance),
        NodeData::Boolean(boolean),
        NodeData::Embed(embed),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_fields_cover_wrapper_and_recent_variant_fields() {
        let group = known_fields("group").expect("group tag known");
        for key in [
            "id",
            "name",
            "transform",
            "opacity",
            "bindings",
            "reactions",
            "scroll_behavior", // wrapper, recent
            "scroll_direction",
            "scroll_offset", // variant, recent
            "corner_smoothing",
            "auto_layout",
            "explicit_modes",
        ] {
            assert!(group.contains(key), "group table missing `{key}`");
        }
        let vector = known_fields("vector").expect("vector tag known");
        for key in ["path", "fills", "strokes", "corner_radii"] {
            assert!(vector.contains(key), "vector table missing `{key}`");
        }
        let text = known_fields("text").expect("text tag known");
        for key in ["content", "style", "style_runs", "truncate", "max_lines"] {
            assert!(text.contains(key), "text table missing `{key}`");
        }
        let text_path = known_fields("text_path").expect("text_path tag known");
        for key in [
            "path",
            "content",
            "style",
            "style_runs",
            "start",
            "alignment",
            "direction",
            "side",
        ] {
            assert!(text_path.contains(key), "text_path table missing `{key}`");
        }
        // Typos are NOT known.
        assert!(!group.contains("colour"));
        assert!(!vector.contains("radius"));
        assert!(known_fields("no_such_kind").is_none());
    }
    #[test]
    fn maximal_samples_cover_recent_fields() {
        let mut tags = BTreeSet::new();
        for data in maximal_variants() {
            let required: &[&str] = match &data {
                NodeData::Group(_) => &[
                    "local_size",
                    "clip_size",
                    "grid",
                    "background_fills",
                    "explicit_modes",
                    "auto_layout",
                    "corner_radii",
                ],
                NodeData::Vector(_) => &[
                    "parametric",
                    "local_size",
                    "corner_radii",
                    "corner_smoothing",
                ],
                NodeData::Text(_) => &[
                    "auto_resize",
                    "style_runs",
                    "max_lines",
                    "truncate",
                    "paragraph_spacing",
                    "paragraph_indent",
                ],
                NodeData::TextPath(_) => &["style_runs", "start", "alignment", "direction", "side"],
                NodeData::Bitmap(_) => &["crop", "tint", "fit", "asset", "natural_size"],
                NodeData::Video(_) => &[
                    "poster",
                    "poster_frame_us",
                    "fit",
                    "speed",
                    "volume",
                    "muted",
                ],
                NodeData::Audio(_) => &["waveform_color", "volume", "muted", "time_range_us"],
                NodeData::NodeGraph(_) => &["preview", "graph"],
                NodeData::Model3d(_) => &["camera", "overrides"],
                NodeData::AiArtifact(_) => &[
                    "lineage_parent",
                    "output",
                    "seed",
                    "inputs",
                    "params",
                    "status",
                ],
                NodeData::Instance(_) => &[
                    "overrides",
                    "prop_values",
                    "derived",
                    "component",
                    "local_size",
                ],
                NodeData::Boolean(_) => &["op", "fills", "strokes", "baked"],
                NodeData::Embed(_) => &["kind", "payload"],
            };
            let value = serde_json::to_value(maximal_node(data)).expect("sample serializes");
            let tag = value
                .get("type")
                .and_then(serde_json::Value::as_str)
                .expect("type");
            tags.insert(tag.to_owned());
            let fields = known_fields(tag).expect("every sample kind known");
            for name in required.iter().copied().chain([
                "id",
                "parent",
                "index",
                "name",
                "transform",
                "constraints",
                "opacity",
                "blend_mode",
                "effects",
                "blurs",
                "flags",
                "is_mask",
                "mask_type",
                "scroll_behavior",
                "meta",
                "bindings",
                "reactions",
                "layout_child",
            ]) {
                assert!(fields.contains(name), "{tag} sample omitted {name}");
            }
            assert!(
                !fields.contains("future_unknown_field"),
                "{tag} must still reject unknown keys"
            );
        }
        assert_eq!(
            tags,
            [
                "group",
                "vector",
                "text",
                "text_path",
                "bitmap",
                "video",
                "audio",
                "node_graph",
                "model3d",
                "ai_artifact",
                "instance",
                "boolean",
                "embed"
            ]
            .into_iter()
            .map(str::to_owned)
            .collect()
        );
    }
}
