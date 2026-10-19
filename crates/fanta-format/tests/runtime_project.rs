#![cfg(feature = "native")]

use fanta_doc::{
    Action, AnimationClip, AnimationClipId, AnimationTrack, AnimationTrackId, AssetId, BitmapNode,
    BoundProp, CanvasNode, Color, ComponentDef, ComponentId, ComponentSet, ComponentSetMembership,
    Doc, DocId, Easing, Fill, GroupNode, ImageFitMode, IndexKey, InstanceNode, Keyframe,
    KeyframeId, Mode, ModeId, MotionProperty, MotionTarget, NodeData, NodeId, PathData,
    PrototypeAnimation, Reaction, ReactionId, ResolvedVarValue, StyleAssignment, StyleDeclaration,
    StyleId, StyleProperty, StyleRule, TextNode, TextPathNode, TextStyle, TextStyleRun,
    Transform2D, Transition, TransitionStyle, Trigger, VarValue, Variable, VariableCollection,
    VariableCollectionId, VariableId, VariableType, VideoNode,
};
use fanta_format::runtime::{RuntimeProgram, SourceBundle};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;

fn collect_files(
    directory: &Path,
    root: &Path,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_files(&path, root, files)?;
        } else {
            let relative = path.strip_prefix(root).map_err(std::io::Error::other)?;
            files.insert(
                relative.to_string_lossy().replace('\\', "/"),
                std::fs::read(path)?,
            );
        }
    }
    Ok(())
}

fn node(document: &mut Doc, identity: u128, parent: Option<NodeId>, data: NodeData) -> NodeId {
    let mut node = CanvasNode::new(data);
    node.id = NodeId::from_u128(identity);
    node.parent = parent;
    node.index = IndexKey::from_raw(identity as f64);
    let id = node.id;
    document.scene.insert(node).expect("fixture node");
    id
}

fn mixed_document() -> (Doc, BTreeMap<AssetId, Vec<u8>>) {
    let mut document = Doc::new();
    document.id = DocId::from_u128(7);
    document.metadata.created_at = 0;
    document.metadata.modified_at = 0;
    document.metadata.title = "Runtime mixed fixture".into();
    let collection = VariableCollectionId::from_u128(7000);
    let light = ModeId::from_u128(7001);
    let dark = ModeId::from_u128(7002);
    let opacity = VariableId::from_u128(7003);
    let typography = VariableId::from_u128(7004);
    let foreground = VariableId::from_u128(7005);
    let radius = VariableId::from_u128(7006);
    let style = TextStyle {
        font_family: "Runtime Fixture".into(),
        size_px: 19.0,
        color: Color::BLACK,
        ..TextStyle::default()
    };
    document.variables.collections.insert(
        collection,
        VariableCollection {
            id: collection,
            name: "Fixture".into(),
            modes: vec![
                Mode {
                    id: light,
                    name: "Light".into(),
                },
                Mode {
                    id: dark,
                    name: "Dark".into(),
                },
            ],
            default_mode: light,
            variable_order: vec![opacity, typography, foreground, radius],
        },
    );
    for (id, name, ty, values_by_mode) in [
        (
            radius,
            "Radius",
            VariableType::Float,
            BTreeMap::from([
                (light, VarValue::Float { value: 12.0 }),
                (dark, VarValue::Float { value: 20.0 }),
            ]),
        ),
        (
            opacity,
            "Opacity",
            VariableType::Float,
            BTreeMap::from([
                (light, VarValue::Float { value: 0.85 }),
                (dark, VarValue::Float { value: 0.6 }),
            ]),
        ),
        (
            typography,
            "Typography",
            VariableType::Typography,
            BTreeMap::from([
                (
                    light,
                    VarValue::TextStyle {
                        value: style.clone(),
                    },
                ),
                (
                    dark,
                    VarValue::TextStyle {
                        value: TextStyle {
                            size_px: 23.0,
                            ..style.clone()
                        },
                    },
                ),
            ]),
        ),
        (
            foreground,
            "Foreground",
            VariableType::Color,
            BTreeMap::from([
                (
                    light,
                    VarValue::Color {
                        value: Color::rgb(20, 80, 40),
                    },
                ),
                (
                    dark,
                    VarValue::Color {
                        value: Color::WHITE,
                    },
                ),
            ]),
        ),
    ] {
        document.variables.variables.insert(
            id,
            Variable {
                id,
                collection,
                name: name.into(),
                ty,
                values_by_mode,
                scopes: Vec::new(),
                options: Vec::new(),
            },
        );
    }
    document.active_modes.insert(collection, dark);
    let box_style = StyleId::from_u128(5000);
    let text_style = StyleId::from_u128(5001);
    let color_style = StyleId::from_u128(5002);
    document.styles.rules.insert(
        box_style,
        StyleRule {
            id: box_style,
            declarations: BTreeMap::from([
                (
                    StyleProperty::Fill,
                    StyleDeclaration {
                        value: json!(Color::rgb(238, 221, 204)),
                        binding: None,
                    },
                ),
                (
                    StyleProperty::CornerRadius,
                    StyleDeclaration {
                        value: json!(12),
                        binding: Some(radius),
                    },
                ),
                (
                    StyleProperty::Opacity,
                    StyleDeclaration {
                        value: json!(0.85),
                        binding: Some(opacity),
                    },
                ),
            ]),
        },
    );
    document.styles.rules.insert(
        text_style,
        StyleRule {
            id: text_style,
            declarations: BTreeMap::from([(
                StyleProperty::Typography,
                StyleDeclaration {
                    value: json!(style),
                    binding: Some(typography),
                },
            )]),
        },
    );
    document.styles.rules.insert(
        color_style,
        StyleRule {
            id: color_style,
            declarations: BTreeMap::from([(
                StyleProperty::Color,
                StyleDeclaration {
                    value: json!(Color::rgb(20, 80, 40)),
                    binding: Some(foreground),
                },
            )]),
        },
    );
    let page = node(
        &mut document,
        1000,
        None,
        NodeData::Group(GroupNode::default()),
    );
    document.add_page(page);
    document.active_page = None;
    let first_frame = node(
        &mut document,
        1001,
        Some(page),
        NodeData::Group(GroupNode {
            clip_size: Some([320.0, 180.0]),
            explicit_modes: BTreeMap::from([(collection, light)]),
            ..GroupNode::default()
        }),
    );
    let second_frame = node(
        &mut document,
        1002,
        Some(page),
        NodeData::Group(GroupNode {
            clip_size: Some([320.0, 180.0]),
            ..GroupNode::default()
        }),
    );
    let rectangle = node(
        &mut document,
        1003,
        Some(first_frame),
        NodeData::Vector(fanta_doc::VectorNode::rect_solid(
            0.0,
            0.0,
            120.0,
            40.0,
            Color::WHITE,
        )),
    );
    let rectangle_node = document.scene.get_mut(rectangle).expect("rectangle");
    rectangle_node.style_assignment.style_ids.push(box_style);
    rectangle_node.style_assignment.local_patch.insert(
        StyleProperty::CornerRadius,
        StyleDeclaration {
            value: json!(12),
            binding: None,
        },
    );
    rectangle_node.transform = Transform2D::translation(8.0, 16.0);
    let first_frame_node = document.scene.get_mut(first_frame).expect("frame");
    first_frame_node.style_assignment.style_ids.push(box_style);
    let mut label = TextNode::new("Header Ω", 180.0, 32.0);
    label.style_runs.push(TextStyleRun {
        start: 0,
        end: 6,
        style: TextStyle {
            underline: true,
            ..TextStyle::default()
        },
    });
    let label_id = node(
        &mut document,
        1004,
        Some(first_frame),
        NodeData::Text(label),
    );
    document
        .scene
        .get_mut(label_id)
        .expect("label")
        .style_assignment = StyleAssignment {
        style_ids: vec![text_style, color_style],
        local_patch: BTreeMap::from([(
            StyleProperty::FontSize,
            StyleDeclaration {
                value: json!(24),
                binding: None,
            },
        )]),
        ..StyleAssignment::default()
    };
    let master = node(
        &mut document,
        1020,
        None,
        NodeData::Group(GroupNode {
            clip_size: Some([160.0, 50.0]),
            background: Some(Fill::solid(Color::WHITE)),
            ..GroupNode::default()
        }),
    );
    let path_label = node(
        &mut document,
        1021,
        Some(master),
        NodeData::TextPath(TextPathNode::new(
            PathData::rect(0.0, 0.0, 140.0, 24.0),
            "Path Ω",
        )),
    );
    document
        .scene
        .get_mut(path_label)
        .expect("path label")
        .style_assignment = StyleAssignment {
        style_ids: vec![text_style, color_style],
        ..StyleAssignment::default()
    };
    let component = ComponentId::from_u128(6000);
    let set = ComponentId::from_u128(6001);
    let mut definition = ComponentDef::new(component, master, "Primary");
    definition.variant_of = Some(ComponentSetMembership {
        set,
        axis_values: BTreeMap::new(),
    });
    document.components.defs.insert(component, definition);
    document.components.sets.insert(
        set,
        ComponentSet {
            id: set,
            name: "Shared button".into(),
            axes: Vec::new(),
            members: vec![component],
            default_variant: component,
            root: None,
        },
    );
    node(
        &mut document,
        1005,
        Some(first_frame),
        NodeData::Instance(InstanceNode {
            component: set,
            overrides: Vec::new(),
            prop_values: BTreeMap::new(),
            derived: Vec::new(),
            local_size: [160.0, 50.0],
        }),
    );
    let image = AssetId::from_u128(8000);
    let video = AssetId::from_u128(8001);
    node(
        &mut document,
        1006,
        Some(first_frame),
        NodeData::Bitmap(BitmapNode {
            asset: image,
            natural_size: [2, 2],
            local_size: [20.0, 20.0],
            crop: None,
            fit: ImageFitMode::Fill,
            tint: None,
        }),
    );
    node(
        &mut document,
        1007,
        Some(second_frame),
        NodeData::Video(VideoNode {
            asset: video,
            natural_size: [2, 2],
            local_size: [20.0, 20.0],
            time_range_us: [0, 1_000_000],
            speed: 1.0,
            muted: true,
            volume: 0.0,
            poster_frame_us: None,
            poster: Some(image),
            fit: ImageFitMode::Fit,
        }),
    );
    let clip_id = AnimationClipId::from_u128(9000);
    let track_id = AnimationTrackId::from_u128(9001);
    let mut clip = AnimationClip::new(clip_id, "Pulse", 1000);
    let mut track = AnimationTrack::new(
        track_id,
        MotionTarget::new(rectangle, MotionProperty::bound(BoundProp::Opacity)),
    );
    track.keyframes.insert(
        KeyframeId::from_u128(9002),
        Keyframe::new(
            KeyframeId::from_u128(9002),
            0,
            ResolvedVarValue::Float { value: 0.0 },
        ),
    );
    track.keyframes.insert(
        KeyframeId::from_u128(9003),
        Keyframe::new(
            KeyframeId::from_u128(9003),
            1000,
            ResolvedVarValue::Float { value: 1.0 },
        ),
    );
    clip.tracks.insert(track_id, track);
    document.motion.clips.insert(clip_id, clip);
    document
        .scene
        .get_mut(first_frame)
        .expect("frame")
        .reactions
        .push(Reaction {
            id: ReactionId::from_u128(9100),
            trigger: Trigger::Click,
            action: Action::Navigate { to: second_frame },
            extra_actions: Vec::new(),
            transition: Some(Transition {
                style: TransitionStyle::Dissolve,
                duration_ms: 180,
                easing: Easing::Linear,
            }),
            animation: Some(PrototypeAnimation {
                clip: clip_id,
                delay_ms: 30,
            }),
        });
    let document = Doc::from_json_str(&document.to_json_string().expect("authored fixture"))
        .expect("compiled fixture");
    (
        document,
        BTreeMap::from([(image, vec![1, 2, 3, 4]), (video, vec![5, 6, 7, 8])]),
    )
}

#[test]
fn mixed_document_native_source_and_deployment_bundle_preserve_authored_state_exactly() {
    let (document, resources) = mixed_document();
    let directory = tempfile::tempdir().expect("project directory");
    fanta_format::write_project_tree(directory.path(), &document, &resources)
        .expect("save native project");
    let mut source = SourceBundle::default();
    collect_files(directory.path(), directory.path(), &mut source.files).expect("project bytes");
    let compiled = RuntimeProgram::compile(&source).expect("portable compilation");
    assert_eq!(
        compiled
            .document()
            .to_authored_json_value()
            .expect("authored runtime"),
        document
            .to_authored_json_value()
            .expect("original authored")
    );
    assert!(compiled.document().active_page.is_none());
    assert_eq!(compiled.document().pages, vec![NodeId::from_u128(1000)]);
    assert_eq!(compiled.resources(), &resources);
    assert!(
        compiled
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == "poster_only_video")
    );
    let bytes = compiled.to_bundle().expect("deployment bundle");
    let loaded = RuntimeProgram::load_bundle(&bytes).expect("load deployment bundle");
    assert_eq!(loaded.to_bundle().expect("rebundle"), bytes);
    assert_eq!(
        loaded
            .document()
            .to_authored_json_value()
            .expect("loaded authored"),
        document
            .to_authored_json_value()
            .expect("original authored")
    );
    let (opened, opened_resources) =
        fanta_format::read_project_tree(directory.path()).expect("native open");
    assert_eq!(opened_resources, resources);
    assert_eq!(
        opened.to_authored_json_value().expect("native authored"),
        document
            .to_authored_json_value()
            .expect("original authored")
    );
    fanta_format::write_project_tree(directory.path(), &opened, &opened_resources)
        .expect("native save");
    let (reopened, _) = fanta_format::read_project_tree(directory.path()).expect("native reopen");
    assert_eq!(
        reopened
            .to_authored_json_value()
            .expect("reopened authored"),
        document
            .to_authored_json_value()
            .expect("original authored")
    );
}

#[test]
fn native_scoped_sessions_keep_the_shared_style_registry_on_open_and_save() {
    let (document, resources) = mixed_document();
    let directory = tempfile::tempdir().expect("project directory");
    fanta_format::write_project_tree(directory.path(), &document, &resources)
        .expect("save native project");
    let mut workspace =
        fanta_format::WorkspaceSession::open(directory.path()).expect("open native workspace");
    let page = document.pages.first().copied().expect("fixture page");
    let component = document
        .components
        .defs
        .keys()
        .next()
        .copied()
        .expect("fixture component");
    for artifact in [
        fanta_format::ArtifactId::Page(page),
        fanta_format::ArtifactId::Component(component),
    ] {
        workspace
            .open_artifact(artifact.clone())
            .expect("open styled artifact");
        let session = workspace.artifact(&artifact).expect("styled session");
        assert_eq!(session.doc().styles, document.styles);
        let authored = session
            .doc()
            .to_json_string()
            .expect("scoped authored serialization");
        let restored = Doc::from_json_str(&authored).expect("restore scoped styled document");
        assert_eq!(restored.styles, document.styles);
        let styled = std::iter::once(session.root())
            .chain(session.doc().scene.descendants_of(session.root()))
            .filter_map(|id| session.doc().scene.get(id))
            .find(|node| !node.style_assignment.is_empty())
            .map(|node| (node.id, node.name.clone(), node.style_assignment.clone()))
            .expect("styled node in artifact");
        workspace
            .artifact_mut(&artifact)
            .expect("editable session")
            .apply(fanta_doc::Operation::SetName {
                id: styled.0,
                old: styled.1,
                new: "Renamed styled fixture".into(),
            })
            .expect("rename without modifying styles");
        workspace
            .save_artifact(artifact.clone())
            .expect("save styled artifact");
        assert_eq!(
            workspace
                .artifact(&artifact)
                .expect("saved session")
                .doc()
                .scene
                .get(styled.0)
                .expect("saved node")
                .style_assignment,
            styled.2
        );
    }
    let (restored, _) = fanta_format::read_project_tree(directory.path()).expect("native reopen");
    assert_eq!(restored.styles, document.styles);
    for id in document
        .scene
        .roots()
        .iter()
        .copied()
        .flat_map(|root| std::iter::once(root).chain(document.scene.descendants_of(root)))
    {
        let original = document.scene.get(id).expect("original node");
        assert_eq!(
            restored
                .scene
                .get(id)
                .expect("restored node")
                .style_assignment,
            original.style_assignment
        );
    }
}
