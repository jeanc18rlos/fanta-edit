//! Tier-1 E2E: deterministic golden-image render.
//!
//! This is the always-on renderer guard. It builds a small but *representative*
//! scene entirely in code — a themed frame background, a text node, a vector
//! with per-corner radii + a stroke, and a component instance — renders it
//! through the real [`RasterRenderer`] path (`render_page_with`, the same entry
//! `fanta-app` uses), and compares the output pixel-for-pixel against a
//! committed golden PNG with a small per-pixel tolerance.
//!
//! Why a golden image and not pixel pokes: a golden catches *any* regression in
//! the composited result — fill resolution, stroke geometry, corner rounding,
//! instance expansion, text layout — in one assertion, through refactors, with
//! no external 21 MB `.fig`. The targeted pixel-poke tests in `raster.rs` stay
//! as fast unit checks; this is the integration backstop.
//!
//! ## Running / regenerating
//! - `cargo test -p fanta-render --test golden_render` — compares against the
//!   committed golden.
//! - `FANTA_REGEN_GOLDEN=1 cargo test -p fanta-render --test golden_render` —
//!   (re)writes `tests/golden/tier1_scene.png` from the current output. Do this
//!   only after an intentional rendering change; eyeball the PNG before
//!   committing it.
//! - On a mismatch the test writes `target/golden_diff_tier1.png` (red where
//!   pixels differ) and fails with the mismatch percentage.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use fanta_doc::id::AssetId;
use fanta_doc::{BooleanNode, BooleanOp};
use fanta_doc::{
    CanvasNode, Color, ComponentDef, ComponentId, ComponentLibrary, Doc, Fill, FontVariation,
    GroupNode, ImageAdjust, ImageFitMode, InstanceNode, NodeData, Operation, Stroke, TextNode,
    Transform2D, VariableRegistry, VectorNode, Viewport,
};
use fanta_render::{InMemoryAssetResolver, RasterRenderer, RenderInputs};
use smallvec::smallvec;
use std::sync::Arc;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 200;

/// Per-channel absolute difference allowed before a pixel counts as "different".
/// Skia's anti-aliasing and the prebuilt-binaries' rasterizer can vary a hair
/// across platforms/versions on edge pixels; 8/255 absorbs that without hiding
/// a real color regression (a wrong fill differs by tens-to-hundreds).
const CHANNEL_TOL: u8 = 8;

/// Fraction of pixels allowed to exceed `CHANNEL_TOL` before the test fails.
/// 0.5% covers sub-pixel AA seams along the rounded corners / stroke / glyph
/// edges (a few hundred pixels of a 64k-pixel image) while still red-flagging a
/// real change (a recolored frame is thousands of pixels).
const MISMATCH_BUDGET: f64 = 0.005;

/// Build the representative scene. Kept deterministic: fixed sizes, fixed
/// viewport, fixed background — so the golden is reproducible byte-for-byte
/// (modulo the AA tolerance) on every run.
///
/// `image_asset` is used for an image-fill vector exercising ImageAdjust path
/// (and the new cache). The caller is responsible for providing matching pixels
/// via the asset resolver at render time.
fn build_scene(
    image_asset: AssetId,
) -> (
    Doc,
    ComponentLibrary,
    BTreeMap<fanta_doc::VariableCollectionId, fanta_doc::ModeId>,
) {
    let mut doc = Doc::new();

    // A "card" master parked far off-screen (so only its instance paints into
    // the visible area): a blue 60x40 rounded rect.
    let mut master = CanvasNode::new(NodeData::Vector(VectorNode {
        path: fanta_doc::PathData::rect(0.0, 0.0, 60.0, 40.0),
        fills: smallvec![Fill::solid(Color::rgb(40, 90, 220))],
        strokes: smallvec![],
        corner_radius: Some(10.0),
        corner_radii: None,
        corner_smoothing: 0.0,
        local_size: None,
        parametric: None,
    }));
    master.name = "Card".into();
    master.transform = Transform2D::translation(10_000.0, 0.0);
    let master_id = master.id;
    doc.apply(Operation::create_node(master)).unwrap();

    let comp = ComponentId::new();
    let mut lib = ComponentLibrary::new();
    lib.defs
        .insert(comp, ComponentDef::new(comp, master_id, "Card"));

    // The visible page: a clipped frame with a light-gray background, holding a
    // text label, a vector with mixed corner radii + a red stroke, and an
    // instance of the card master.
    let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([300.0, 180.0]),
        background: Some(Fill::solid(Color::rgb(242, 242, 242))),
        explicit_modes: Default::default(),
        ..Default::default()
    }));
    frame.name = "Page".into();
    frame.transform = Transform2D::translation(0.0, 0.0);
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).unwrap();
    doc.add_page(frame_id);
    doc.set_active_page(Some(frame_id));

    // Text label, dark on the light frame. Uses variable font + explicit wght
    // variation to exercise the variable-font shaping + cache path.
    let mut label = CanvasNode::new(NodeData::Text({
        let mut t = TextNode::new("Fantaisa", 200.0, 30.0);
        t.style.size_px = 24.0;
        t.style.weight = 700;
        t.style.color = Color::rgb(20, 20, 20);
        t.style.font_family = "Source Sans 3".to_string(); // bundled variable font
        t.style.font_variations = vec![FontVariation::new("wght", 650.0)]; // non-default wght
        t
    }));
    label.name = "Title".into();
    label.transform = Transform2D::translation(20.0, 16.0);
    label.parent = Some(frame_id);
    doc.apply(Operation::create_node(label)).unwrap();

    // Vector with per-corner radii + a stroke (exercises corner + stroke paths).
    let mut shape = CanvasNode::new(NodeData::Vector(VectorNode {
        path: fanta_doc::PathData::rect(0.0, 0.0, 100.0, 70.0),
        fills: smallvec![Fill::solid(Color::rgb(255, 200, 60))],
        strokes: smallvec![Stroke::solid(Color::rgb(200, 40, 40), 4.0)],
        corner_radius: None,
        corner_radii: Some([0.0, 20.0, 0.0, 20.0]),
        corner_smoothing: 0.0,
        local_size: None,
        parametric: None,
    }));
    shape.name = "Tile".into();
    shape.transform = Transform2D::translation(20.0, 70.0);
    shape.parent = Some(frame_id);
    doc.apply(Operation::create_node(shape)).unwrap();

    // The card instance, placed on the right.
    let mut inst = CanvasNode::new(NodeData::Instance(InstanceNode {
        component: comp,
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [60.0, 40.0],
    }));
    inst.name = "CardInstance".into();
    inst.transform = Transform2D::translation(180.0, 90.0);
    inst.parent = Some(frame_id);
    doc.apply(Operation::create_node(inst)).unwrap();

    // Boolean node exercising the new paint + cache path (union of two rects for visibility).
    let mut boolean = CanvasNode::new(NodeData::Boolean(BooleanNode {
        op: BooleanOp::Union,
        fills: smallvec![Fill::solid(Color::rgb(80, 160, 80))],
        strokes: smallvec![],
    }));
    boolean.name = "BoolUnion".into();
    boolean.transform = Transform2D::translation(20.0, 150.0);
    boolean.parent = Some(frame_id);
    let bool_id = boolean.id;
    doc.apply(Operation::create_node(boolean)).unwrap();

    // Operand A (left rect)
    let mut op_a = CanvasNode::new(NodeData::Vector(VectorNode {
        path: fanta_doc::PathData::rect(0.0, 0.0, 25.0, 20.0),
        fills: smallvec![Fill::solid(Color::rgb(255, 255, 255))],
        strokes: smallvec![],
        ..Default::default()
    }));
    op_a.parent = Some(bool_id);
    op_a.index = doc.scene.next_child_index(Some(bool_id));
    doc.apply(Operation::create_node(op_a)).unwrap();

    // Operand B (overlapping right rect)
    let mut op_b = CanvasNode::new(NodeData::Vector(VectorNode {
        path: fanta_doc::PathData::rect(15.0, 0.0, 25.0, 20.0),
        fills: smallvec![Fill::solid(Color::rgb(255, 255, 255))],
        strokes: smallvec![],
        ..Default::default()
    }));
    op_b.parent = Some(bool_id);
    op_b.index = doc.scene.next_child_index(Some(bool_id));
    doc.apply(Operation::create_node(op_b)).unwrap();

    // Image fill with non-default adjust (exercises tone curve + chroma matrix + cache).
    // The actual pixels are supplied by the caller via resolver for this asset id.
    let mut img_rect = CanvasNode::new(NodeData::Vector(VectorNode {
        path: fanta_doc::PathData::rect(0.0, 0.0, 30.0, 30.0),
        fills: smallvec![Fill::Image {
            asset: image_asset,
            mode: ImageFitMode::Fill,
            opacity: 1.0,
            crop: None,
            adjust: ImageAdjust {
                exposure: 0.15,
                contrast: 0.25,
                saturation: -0.1,
                ..Default::default()
            },
            scale: None,
            rotation: None,
            blend: fanta_doc::style::BlendMode::Normal,
        }],
        strokes: smallvec![],
        ..Default::default()
    }));
    img_rect.name = "AdjustedImage".into();
    img_rect.transform = Transform2D::translation(60.0, 150.0);
    img_rect.parent = Some(frame_id);
    doc.apply(Operation::create_node(img_rect)).unwrap();

    (doc, lib, BTreeMap::new())
}

/// Render the scene to a straight-alpha RGBA8 buffer with a fixed viewport.
fn render_scene() -> Vec<u8> {
    // Create a small deterministic image for the adjusted fill example.
    // 4x4 RGBA (straight alpha) — a soft gradient-ish red block.
    let px: Vec<u8> = (0..4 * 4 * 4)
        .map(|i| {
            let c = (i % 4) as u8;
            match i % 4 {
                0 => 180 + c * 10, // R
                1 => 60,           // G
                2 => 40,           // B
                _ => 255,          // A
            }
        })
        .collect();
    let decoded = fanta_render::asset::DecodedImage::new(Arc::new(px), 4, 4);
    let image_asset = AssetId::new();
    let mut resolver = InMemoryAssetResolver::new();
    resolver.insert(image_asset, decoded);
    let resolver = Arc::new(resolver);

    let (doc, lib, modes) = build_scene(image_asset);
    let registry = VariableRegistry::new();
    let inputs = RenderInputs {
        components: &lib,
        variables: &registry,
        active_modes: &modes,
        mode_generation: 0,
        motion: None,
        playback: None,
        dark_ui: false,
    };
    // Frame the 300x180 page (origin top-left) in the 320x200 surface with a
    // little margin; centre on (150, 90).
    let viewport = Viewport {
        center: [150.0, 90.0],
        zoom: 1.0,
    };
    let page = doc.active_page();

    let mut r = RasterRenderer::new(WIDTH, HEIGHT).unwrap();
    // Opaque white canvas so the golden has no alpha-dependent platform fuzz.
    r.background = Color::WHITE;
    r.set_asset_resolver(resolver);
    r.render_page_with(&doc.scene, &viewport, page, &inputs);
    r.copy_rgba()
}

fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/tier1_scene.png")
}

/// Decode a PNG file to straight-alpha RGBA8 (`WIDTH*HEIGHT*4` bytes).
fn decode_png(path: &Path) -> Vec<u8> {
    let img = image::open(path)
        .unwrap_or_else(|e| panic!("decode golden {}: {e}", path.display()))
        .to_rgba8();
    assert_eq!(
        (img.width(), img.height()),
        (WIDTH, HEIGHT),
        "golden dimensions changed; regenerate with FANTA_REGEN_GOLDEN=1"
    );
    img.into_raw()
}

/// Write a straight-alpha RGBA8 buffer to a PNG.
fn write_png(path: &Path, buf: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let img = image::RgbaImage::from_raw(WIDTH, HEIGHT, buf.to_vec())
        .expect("buffer matches WIDTH*HEIGHT*4");
    img.save(path)
        .unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}

#[test]
fn tier1_golden_scene_matches() {
    let actual = render_scene();

    let golden = golden_path();
    if std::env::var_os("FANTA_REGEN_GOLDEN").is_some() {
        write_png(&golden, &actual);
        eprintln!("regenerated golden at {}", golden.display());
        return;
    }
    assert!(
        golden.exists(),
        "missing golden {}; create it once with FANTA_REGEN_GOLDEN=1 cargo test -p fanta-render --test golden_render",
        golden.display()
    );
    let expected = decode_png(&golden);
    assert_eq!(actual.len(), expected.len(), "buffer size mismatch");

    let total = (WIDTH * HEIGHT) as usize;
    let mut diff_pixels = 0usize;
    let mut diff_img = vec![0u8; actual.len()];
    for p in 0..total {
        let i = p * 4;
        let differs = (0..4).any(|c| {
            let a = actual[i + c] as i16;
            let e = expected[i + c] as i16;
            (a - e).unsigned_abs() as u8 > CHANNEL_TOL
        });
        if differs {
            diff_pixels += 1;
            diff_img[i] = 255; // mark differing pixels red, opaque
            diff_img[i + 3] = 255;
        } else {
            // Faint gray where it matches, so the diff image is legible.
            diff_img[i] = expected[i];
            diff_img[i + 1] = expected[i + 1];
            diff_img[i + 2] = expected[i + 2];
            diff_img[i + 3] = 60;
        }
    }
    let frac = diff_pixels as f64 / total as f64;
    if frac > MISMATCH_BUDGET {
        let diff_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/golden_diff_tier1.png");
        write_png(&diff_path, &diff_img);
        // Also drop the actual render next to the diff for side-by-side review.
        let actual_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/golden_actual_tier1.png");
        write_png(&actual_path, &actual);
        panic!(
            "golden mismatch: {diff_pixels}/{total} pixels differ ({:.3}% > {:.3}% budget).\n\
             diff written to {}\n actual written to {}\n\
             If this change is intentional, regenerate with \
             FANTA_REGEN_GOLDEN=1 cargo test -p fanta-render --test golden_render",
            frac * 100.0,
            MISMATCH_BUDGET * 100.0,
            diff_path.display(),
            actual_path.display(),
        );
    }
}

#[test]
fn tier1_render_is_deterministic() {
    // Two renders of the same scene must be byte-identical — the precondition
    // for a golden comparison to be stable.
    let a = render_scene();
    let b = render_scene();
    assert_eq!(a, b, "render output must be deterministic across runs");
}
