//! Headless SVG target smoke tests.
//!
//! The SVG path must exercise the same renderer walk as PNG/GPU targets and
//! remain deterministic enough for source-oriented harness diffs.

use fanta_doc::{CanvasNode, Color, Doc, NodeData, Operation, UnitInterval, VectorNode, Viewport};
use fanta_render::{RasterRenderer, RenderInputs, SvgRenderOptions};

fn scene() -> Doc {
    let mut doc = Doc::new();
    let rectangle = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        100.0,
        60.0,
        Color::rgb(0x31, 0x7A, 0xF5),
    )));
    doc.apply(Operation::create_node(rectangle))
        .expect("insert rectangle through operation");
    doc
}

fn render(doc: &Doc) -> fanta_render::SvgRenderOutput {
    let viewport = Viewport {
        center: [50.0, 30.0],
        zoom: 1.0,
    };
    let mut renderer = RasterRenderer::new(100, 60).expect("renderer");
    renderer.render_page_svg_with(
        &doc.scene,
        &viewport,
        None,
        &RenderInputs::for_doc(doc),
        SvgRenderOptions::default(),
    )
}

#[test]
fn svg_target_emits_valid_vector_output_and_metrics() {
    let doc = scene();
    let output = render(&doc);
    let svg = std::str::from_utf8(&output.svg).expect("SVG is UTF-8");

    assert!(svg.contains("<svg"));
    assert!(svg.contains("</svg>"));
    assert!(
        svg.contains("<path") || svg.contains("<rect"),
        "expected vector geometry in {svg}"
    );
    assert_eq!(output.metrics.nodes_visited, 1);
    assert_eq!(output.metrics.nodes_drawn, 1);
}

#[test]
fn svg_target_is_byte_deterministic_for_the_same_scene() {
    let doc = scene();
    let first = render(&doc);
    let second = render(&doc);
    assert_eq!(first.svg, second.svg);
}

#[test]
fn svg_target_keeps_save_layer_geometry_visible_for_inspection() {
    let mut doc = scene();
    let rectangle = doc.scene.roots()[0];
    doc.apply(Operation::SetOpacity {
        id: rectangle,
        old: UnitInterval::ONE,
        new: UnitInterval::new(0.5),
    })
    .expect("set opacity through operation");

    let output = render(&doc);
    let svg = std::str::from_utf8(&output.svg).expect("SVG is UTF-8");
    assert!(
        svg.contains("<path") || svg.contains("<rect"),
        "the SVG inspection target must bypass unsupported save-layers instead of dropping content"
    );
}
