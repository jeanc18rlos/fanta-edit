//! Write a tiny `fanta-project` containing a component master (a frame with a
//! big text label) and two instances of it, so the in-place editing of text
//! INSIDE a component instance can be exercised in the running app.
//!
//! Usage: `cargo run -p fig_viewer --example make_instance_project -- <out_dir>`
//! Then open `<out_dir>/fanta.json` in the app.

use std::{collections::BTreeMap, env, path::PathBuf};

use anyhow::{Result, anyhow};
use fanta_doc::{
    CanvasNode, Color, ComponentDef, ComponentId, Doc, Fill, GroupNode, InstanceNode, NodeData,
    Operation, TextAlign, TextNode, Transform2D, VectorNode,
};

/// A text node with a large, obviously-readable label.
fn big_text(content: &str, width: f64, height: f64, color: Color) -> TextNode {
    let mut text = TextNode::new(content, width, height);
    text.style.size_px = 48.0;
    text.style.color = color;
    text.align = TextAlign::Left;
    text
}

fn main() -> Result<()> {
    let out_dir = env::args()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("usage: make_instance_project <out_dir>"))?;

    let mut doc = Doc::new();

    // ---- page ------------------------------------------------------------
    let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
    let page_id = page.id;
    page.transform = Transform2D::IDENTITY;
    // Seed a comment thread so pin/popover rendering is verifiable on open.
    page.meta = serde_json::json!({
        "comments": [
            {
                "id": "seed-thread",
                "world": [640.0, 180.0],
                "author": "jean",
                "text": "Can we brighten this banner?",
                "created": 1_754_000_000u64,
                "replies": [
                    {
                        "author": "ana",
                        "body": "Trying FFD666 now.",
                        "created": 1_754_003_600u64
                    }
                ]
            },
            {
                "id": "seed-resolved",
                "world": [640.0, 480.0],
                "author": "ana",
                "text": "Old note, done.",
                "created": 1_753_900_000u64,
                "resolved": true
            }
        ]
    });
    doc.apply(Operation::create_node(page))?;
    doc.add_page(page_id);
    doc.set_active_page(Some(page_id));

    // A backdrop rect so the page is obviously present on the canvas.
    let mut backdrop = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        900.0,
        600.0,
        Color::rgb(245, 245, 245),
    )));
    backdrop.parent = Some(page_id);
    backdrop.index = doc.scene.next_child_index(Some(page_id));
    doc.apply(Operation::create_node(backdrop))?;

    // ---- component master: ON the page and visible, the Figma way (the old
    // fanta moved masters off-canvas; that convention was dropped).
    let mut master_root = CanvasNode::new(NodeData::Group(GroupNode {
        background: Some(Fill::solid(Color::rgb(255, 214, 102))),
        clip_size: Some([700.0, 120.0]),
        ..GroupNode::default()
    }));
    let master_root_id = master_root.id;
    master_root.parent = Some(page_id);
    master_root.index = doc.scene.next_child_index(Some(page_id));
    master_root.transform = Transform2D::translation(100.0, 40.0);
    doc.apply(Operation::create_node(master_root))?;

    // The text child, offset inside the master.
    let mut master_text = CanvasNode::new(NodeData::Text(big_text(
        "EDIT ME",
        640.0,
        70.0,
        Color::rgb(20, 20, 20),
    )));
    master_text.parent = Some(master_root_id);
    master_text.index = doc.scene.next_child_index(Some(master_root_id));
    master_text.transform = Transform2D::translation(30.0, 25.0);
    doc.apply(Operation::create_node(master_text))?;

    let component = ComponentId::new();
    doc.components.defs.insert(
        component,
        ComponentDef::new(component, master_root_id, "Banner"),
    );

    // ---- two instances of it on the page, below the master ---------------
    for (i, y) in [220.0_f64, 400.0].into_iter().enumerate() {
        let mut instance = CanvasNode::new(NodeData::Instance(InstanceNode {
            component,
            overrides: Vec::new(),
            prop_values: BTreeMap::new(),
            derived: Vec::new(),
            local_size: [700.0, 120.0],
        }));
        instance.parent = Some(page_id);
        instance.index = doc.scene.next_child_index(Some(page_id));
        instance.transform = Transform2D::translation(100.0, y);
        let id = instance.id;
        doc.apply(Operation::create_node(instance))?;
        println!("instance {i} id={id:?} at world (100, {y}) size 700x120");
    }

    fanta_format::scaffold_project_tree(&out_dir)?;
    fanta_format::write_project_tree(&out_dir, &doc, &BTreeMap::new())?;

    println!("wrote project to {}", out_dir.display());
    println!("open {}/fanta.json in the app", out_dir.display());
    println!(
        "master root id={master_root_id:?} on page at (100, 40); text clone world origin for instance 0 = (130, 245)"
    );
    Ok(())
}
