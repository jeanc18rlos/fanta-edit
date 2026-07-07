//! Component-instance render tests: a one-rect master renders through an
//! instance, text/derived overrides apply, dangling instances draw the faint
//! outline, and expansions are memoized across frames.
use super::*;
use fanta_doc::{Doc, Operation, VectorNode};

// -----------------------------------------------------------------------
// Component instances (expand_instance → transient subtree)
// -----------------------------------------------------------------------

use fanta_doc::{ComponentDef, ComponentId, ComponentLibrary, InstanceNode, VariableRegistry};

/// Build a one-rect component master in `doc`'s scene and register it in a
/// fresh library. The master is a single `rect_solid` of `color` at local
/// `[0,0,w,h]`, living at world `(mx, my)` (so it can be placed far from the
/// instance to prove the instance draws from its *own* transform, not the
/// master's location). Returns `(library, component id)`.
fn one_rect_master(
    doc: &mut Doc,
    w: f64,
    h: f64,
    color: Color,
    mx: f64,
    my: f64,
) -> (ComponentLibrary, ComponentId) {
    let mut master = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0, 0.0, w, h, color,
    )));
    master.transform = Transform2D::translation(mx, my);
    let root = master.id;
    doc.apply(Operation::create_node(master)).unwrap();

    let comp = ComponentId::new();
    let mut lib = ComponentLibrary::new();
    lib.defs.insert(comp, ComponentDef::new(comp, root, "Rect"));
    (lib, comp)
}

#[test]
fn instance_of_one_rect_master_renders_that_rect() {
    // A 20x20 red master, plus an instance of it centred at the origin. With
    // the component library passed in, the instance must paint the master's
    // red rect at the surface centre — not the F0 placeholder box.
    let mut doc = Doc::new();
    // Master parked far off-screen so its own node never paints into the
    // visible area; only the expansion (drawn under the instance) can.
    let (lib, comp) = one_rect_master(&mut doc, 20.0, 20.0, Color::rgb(255, 0, 0), 10_000.0, 0.0);

    let mut inst = CanvasNode::new(NodeData::Instance(InstanceNode {
        component: comp,
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [20.0, 20.0],
    }));
    // Centre the instance's [0,0,20,20] box on the world origin.
    inst.transform = Transform2D::translation(-10.0, -10.0);
    doc.apply(Operation::create_node(inst)).unwrap();

    let registry = VariableRegistry::new();
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
    assert!(
        c[0] > 200 && c[1] < 40 && c[2] < 40 && c[3] > 200,
        "instance should paint the master's red rect at centre, got {c:?}"
    );
}

#[test]
fn instance_text_override_renders_the_overridden_glyphs() {
    // A master holding a text child; the instance overrides that text. The
    // override is applied during expansion, so glyphs must paint (the
    // expanded transient subtree is walked and drawn).
    use fanta_doc::{GroupNode, Override, TextNode};
    use smallvec::smallvec;

    let mut doc = Doc::new();
    // Master: a frame with a text child.
    let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([120.0, 40.0]),
        background: None,
        explicit_modes: Default::default(),
        ..Default::default()
    }));
    frame.transform = Transform2D::translation(10_000.0, 0.0);
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).unwrap();
    let mut label = CanvasNode::new(NodeData::Text({
        let mut t = TextNode::new("X", 120.0, 40.0);
        t.style.size_px = 30.0;
        t
    }));
    label.parent = Some(frame_id);
    let label_id = label.id;
    doc.apply(Operation::create_node(label)).unwrap();

    let comp = ComponentId::new();
    let mut lib = ComponentLibrary::new();
    lib.defs
        .insert(comp, ComponentDef::new(comp, frame_id, "Label"));

    let mut inst = CanvasNode::new(NodeData::Instance(InstanceNode {
        component: comp,
        overrides: vec![Override {
            target_path: smallvec![label_id],
            target_prop: fanta_doc::BoundProp::TextContent,
            value: fanta_doc::OverrideValue::Text {
                value: "Hello".into(),
            },
        }],
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [120.0, 40.0],
    }));
    inst.transform = Transform2D::translation(-50.0, -10.0);
    doc.apply(Operation::create_node(inst)).unwrap();

    let registry = VariableRegistry::new();
    let inputs = RenderInputs {
        components: &lib,
        variables: &registry,
        active_modes: &BTreeMap::new(),
        mode_generation: 0,
        playback: None,
        dark_ui: false,
    };

    let mut r = RasterRenderer::new(160, 64).unwrap();
    let metrics = r.render_with(&doc.scene, &doc.viewport, &inputs);
    assert!(metrics.nodes_drawn >= 1, "expanded text should draw");
    let buf = r.copy_rgba();
    assert!(
        opaque_pixel_count(&buf) > 0,
        "the overridden 'Hello' glyphs must light up pixels"
    );
}

#[test]
fn dangling_instance_draws_a_faint_outline_not_a_blue_block() {
    // An instance whose component id is unknown (no master) must NOT paint
    // the old heavy translucent-blue block over the design. Instead it draws
    // a faint 1px gray dashed outline of its box: the interior stays empty
    // (transparent) and only the box edges carry a few pixels. Also a no-panic
    // guard. The 20x20 box at world (-10,-10)..(10,10) maps to surface pixels
    // ~22..42 on the 64x64 centered surface.
    let mut doc = Doc::new();
    let mut inst = CanvasNode::new(NodeData::Instance(InstanceNode {
        component: ComponentId::new(), // not in the (empty) library
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [20.0, 20.0],
    }));
    inst.transform = Transform2D::translation(-10.0, -10.0);
    doc.apply(Operation::create_node(inst)).unwrap();

    let lib = ComponentLibrary::new();
    let registry = VariableRegistry::new();
    let inputs = RenderInputs {
        components: &lib,
        variables: &registry,
        active_modes: &BTreeMap::new(),
        mode_generation: 0,
        playback: None,
        dark_ui: false,
    };

    let mut r = RasterRenderer::new(64, 64).unwrap();
    let metrics = r.render_with(&doc.scene, &doc.viewport, &inputs);
    assert!(metrics.nodes_drawn >= 1, "the outline counts as a draw");
    let buf = r.copy_rgba();
    // Interior (box centre): must be empty — no filled blue block. The old
    // bug painted rgba(120,170,255,90) here; the fix paints nothing inside.
    let centre = rgba_at(&buf, 64, 32, 32);
    assert_eq!(
        centre[3], 0,
        "instance interior must be empty (no blue block), got {centre:?}"
    );
    // The dashed outline lives on the box edge (top edge ≈ surface row 22).
    // Scan a band of rows around the top edge for any dashed-stroke pixel.
    let edge_lit = (20u32..25).any(|row| (16u32..48).any(|x| rgba_at(&buf, 64, x, row)[3] > 0));
    assert!(
        edge_lit,
        "expected a faint dashed outline along the box edge"
    );
}

#[test]
fn legacy_render_draws_instance_outline_without_a_library() {
    // The back-compatible `render` (no RenderInputs) has no component
    // library, so an instance is unresolved and falls back to the faint
    // outline (not the old blue block). Regression guard for the empty-context
    // wrapper.
    let mut doc = Doc::new();
    let mut inst = CanvasNode::new(NodeData::Instance(InstanceNode {
        component: ComponentId::new(),
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [20.0, 20.0],
    }));
    inst.transform = Transform2D::translation(-10.0, -10.0);
    doc.apply(Operation::create_node(inst)).unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    let metrics = r.render(&doc.scene, &doc.viewport);
    assert!(metrics.nodes_drawn >= 1);
    let buf = r.copy_rgba();
    let centre = rgba_at(&buf, 64, 32, 32);
    assert_eq!(
        centre[3], 0,
        "no blue block under the legacy path, got {centre:?}"
    );
    let edge_lit = (20u32..25).any(|row| (16u32..48).any(|x| rgba_at(&buf, 64, x, row)[3] > 0));
    assert!(edge_lit, "faint outline visible under the legacy path");
}

#[test]
fn derived_instance_preserves_baked_positions_instead_of_reflowing() {
    use fanta_doc::{
        AutoLayout, AxisSizing, CounterAlign, DerivedOverride, GroupNode, LayoutMode, PrimaryAlign,
    };
    use smallvec::smallvec;

    let mut doc = Doc::new();
    let mut root = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([100.0, 20.0]),
        background: None,
        auto_layout: Some(AutoLayout {
            mode: LayoutMode::Horizontal,
            spacing: 0.0,
            counter_spacing: 0.0,
            padding: [0.0, 0.0, 0.0, 0.0],
            primary_align: PrimaryAlign::Start,
            counter_align: CounterAlign::Start,
            primary_sizing: AxisSizing::Fixed,
            counter_sizing: AxisSizing::Fixed,
            wrap: false,
            flow_reverse: false,
            child_layout: true,
            reverse_z: false,
        }),
        explicit_modes: Default::default(),
        ..Default::default()
    }));
    root.transform = Transform2D::translation(10_000.0, 0.0);
    let root_id = root.id;
    doc.apply(Operation::create_node(root)).unwrap();

    let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        10.0,
        10.0,
        Color::rgb(255, 0, 0),
    )));
    child.parent = Some(root_id);
    let child_id = child.id;
    doc.apply(Operation::create_node(child)).unwrap();

    let comp = ComponentId::new();
    let mut lib = ComponentLibrary::new();
    lib.defs
        .insert(comp, ComponentDef::new(comp, root_id, "AutoLayoutMaster"));

    let mut inst = CanvasNode::new(NodeData::Instance(InstanceNode {
        component: comp,
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: vec![DerivedOverride {
            path: smallvec![child_id],
            transform: Some(Transform2D::translation(60.0, 5.0)),
            size: None,
            fills: None,
            path_data: None,
            stroke_path: None,
            stroke_weight: None,
            text: None,
        }],
        local_size: [100.0, 20.0],
    }));
    inst.transform = Transform2D::translation(-50.0, -10.0);
    doc.apply(Operation::create_node(inst)).unwrap();

    let registry = VariableRegistry::new();
    let inputs = RenderInputs {
        components: &lib,
        variables: &registry,
        active_modes: &BTreeMap::new(),
        mode_generation: 0,
        playback: None,
        dark_ui: false,
    };

    let mut renderer = RasterRenderer::new(128, 64).unwrap();
    renderer.render_with(&doc.scene, &doc.viewport, &inputs);
    let buffer = renderer.copy_rgba();

    let left = rgba_at(&buffer, 128, 18, 32);
    let right = rgba_at(&buffer, 128, 78, 32);
    assert!(
        left[3] == 0,
        "a solver reflow would place the rect near the left edge, got {left:?}"
    );
    assert!(
        right[0] > 200 && right[1] < 40 && right[2] < 40 && right[3] > 200,
        "derived transform should keep the rect near the right edge, got {right:?}"
    );
}

#[test]
fn instance_expansion_is_memoized_across_frames() {
    // Two renders of the same unchanged instance must reuse one cached
    // expansion (the memo key is stable), so the cache holds exactly one
    // entry after either frame.
    let mut doc = Doc::new();
    let (lib, comp) = one_rect_master(&mut doc, 20.0, 20.0, Color::rgb(0, 0, 255), 10_000.0, 0.0);
    let mut inst = CanvasNode::new(NodeData::Instance(InstanceNode {
        component: comp,
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [20.0, 20.0],
    }));
    inst.transform = Transform2D::translation(-10.0, -10.0);
    doc.apply(Operation::create_node(inst)).unwrap();

    let registry = VariableRegistry::new();
    let inputs = RenderInputs {
        components: &lib,
        variables: &registry,
        active_modes: &BTreeMap::new(),
        mode_generation: 0,
        playback: None,
        dark_ui: false,
    };

    let mut r = RasterRenderer::new(64, 64).unwrap();
    assert_eq!(r.instance_cache_len(), 0, "cache starts empty");
    r.render_with(&doc.scene, &doc.viewport, &inputs);
    assert_eq!(
        r.instance_cache_len(),
        1,
        "first frame caches the expansion"
    );
    r.render_with(&doc.scene, &doc.viewport, &inputs);
    assert_eq!(
        r.instance_cache_len(),
        1,
        "second frame reuses the cached expansion, no new entry"
    );

    r.clear_instance_cache();
    assert_eq!(r.instance_cache_len(), 0, "clear drops the memo");
}
