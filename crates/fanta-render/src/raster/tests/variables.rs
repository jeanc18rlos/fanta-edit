//! Variable-resolution-overlay tests: a bound fill follows the active mode
//! (dark vs default) onto a scratch copy without mutating the scene.
use super::*;
use fanta_doc::{
    ComponentLibrary, Doc, GroupNode, ModeId, Operation, Transform2D, Variable, VariableCollection,
    VariableCollectionId, VariableId, VariableRegistry, VectorNode,
};

// -----------------------------------------------------------------------
// Variable-resolution overlay (bound fill follows the active mode)
// -----------------------------------------------------------------------

/// A registry with one "Theme" collection (Light default, Dark) and a `bg`
/// color variable: white in Light, blue in Dark. Returns the registry, the
/// collection id, the Dark mode id, and the variable id.
fn theme_registry_white_blue() -> (VariableRegistry, VariableCollectionId, ModeId, VariableId) {
    use fanta_doc::{Mode, VarValue, VariableType};

    let coll = VariableCollectionId::new();
    let light = ModeId::new();
    let dark = ModeId::new();
    let bg = VariableId::new();
    let mut reg = VariableRegistry::new();
    reg.collections.insert(
        coll,
        VariableCollection {
            id: coll,
            name: "Theme".into(),
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
            variable_order: vec![bg],
        },
    );
    reg.variables.insert(
        bg,
        Variable {
            id: bg,
            collection: coll,
            name: "bg".into(),
            ty: VariableType::Color,
            values_by_mode: BTreeMap::from([
                (
                    light,
                    VarValue::Color {
                        value: Color::WHITE,
                    },
                ),
                (
                    dark,
                    VarValue::Color {
                        value: Color::rgb(0, 0, 255),
                    },
                ),
            ]),
            scopes: Vec::new(),
        },
    );
    (reg, coll, dark, bg)
}

#[test]
fn bound_fill_paints_dark_color_when_active_mode_is_dark() {
    // A 20x20 rect whose fill[0] is bound to the Theme `bg` variable. With
    // the document active mode set to Dark, the overlay must substitute the
    // Dark value (blue) before painting — without mutating the scene node
    // (whose literal stays white).
    let (registry, coll, dark, bg) = theme_registry_white_blue();

    let mut doc = Doc::new();
    let mut rect = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -10.0,
        -10.0,
        20.0,
        20.0,
        Color::WHITE, // the literal; the binding overrides it at paint time
    )));
    rect.bindings
        .insert(fanta_doc::BoundProp::FillColor { index: 0 }, bg);
    let rect_id = rect.id;
    doc.apply(Operation::create_node(rect)).unwrap();

    let lib = ComponentLibrary::new();
    let active = BTreeMap::from([(coll, dark)]);
    let inputs = RenderInputs {
        components: &lib,
        variables: &registry,
        active_modes: &active,
        mode_generation: 1,
        playback: None,
        dark_ui: false,
    };

    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render_with(&doc.scene, &doc.viewport, &inputs);
    let buf = r.copy_rgba();
    let c = rgba_at(&buf, 64, 32, 32);
    assert!(
        c[2] > 200 && c[0] < 40 && c[1] < 40,
        "Dark mode must paint the bound blue fill, got {c:?}"
    );

    // The live scene node is unchanged: its literal fill is still white.
    match &doc.scene.get(rect_id).unwrap().data {
        NodeData::Vector(v) => match v.fills.first() {
            Some(Fill::Solid { color }) => {
                assert_eq!(
                    *color,
                    Color::WHITE,
                    "the scene node must not be mutated by rendering"
                )
            }
            other => panic!("expected a solid fill, got {other:?}"),
        },
        other => panic!("expected a vector, got {other:?}"),
    }
}

#[test]
fn bound_fill_paints_default_mode_color_when_no_active_mode() {
    // With no active mode selected, the overlay resolves the collection
    // default (Light → white). Sampling the centre, the rect must be white
    // (high on all channels), confirming the default-mode path.
    let (registry, _coll, _dark, bg) = theme_registry_white_blue();

    let mut doc = Doc::new();
    let mut rect = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -10.0,
        -10.0,
        20.0,
        20.0,
        Color::rgb(255, 0, 0), // a literal that differs from BOTH modes
    )));
    rect.bindings
        .insert(fanta_doc::BoundProp::FillColor { index: 0 }, bg);
    doc.apply(Operation::create_node(rect)).unwrap();

    let lib = ComponentLibrary::new();
    let inputs = RenderInputs {
        components: &lib,
        variables: &registry,
        active_modes: &BTreeMap::new(),
        mode_generation: 0,
        playback: None,
        dark_ui: false,
    };

    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render_with(&doc.scene, &doc.viewport, &inputs);
    let buf = r.copy_rgba();
    let c = rgba_at(&buf, 64, 32, 32);
    // White, not the red literal and not the blue Dark value.
    assert!(
        c[0] > 200 && c[1] > 200 && c[2] > 200,
        "default (Light) mode must paint white, got {c:?}"
    );
}

#[test]
fn bound_frame_background_paints_dark_color_when_active_mode_is_dark() {
    // Frame backgrounds import as `GroupNode::background`, not `VectorNode`
    // fills. A paint-level fill binding on a frame must still resolve before
    // the frame background is drawn.
    let (registry, coll, dark, bg) = theme_registry_white_blue();

    let mut doc = Doc::new();
    let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([20.0, 20.0]),
        background: Some(Fill::solid(Color::WHITE)),
        ..Default::default()
    }));
    frame.transform = Transform2D::translation(-10.0, -10.0);
    frame
        .bindings
        .insert(fanta_doc::BoundProp::FillColor { index: 0 }, bg);
    doc.apply(Operation::create_node(frame)).unwrap();

    let lib = ComponentLibrary::new();
    let active = BTreeMap::from([(coll, dark)]);
    let inputs = RenderInputs {
        components: &lib,
        variables: &registry,
        active_modes: &active,
        mode_generation: 1,
        playback: None,
        dark_ui: false,
    };

    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render_with(&doc.scene, &doc.viewport, &inputs);
    let buf = r.copy_rgba();
    let c = rgba_at(&buf, 64, 32, 32);
    assert!(
        c[2] > 200 && c[0] < 40 && c[1] < 40,
        "Dark mode must paint the bound blue frame background, got {c:?}"
    );
}
