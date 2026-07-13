//! Probe: load a fanta project, expand each instance, and report every clone
//! (kind, def_path, reconstructed world transform, world box) plus whether a
//! given world point hits a TEXT clone — mirroring `instance_text::text_target_at`.
//!
//! Usage: `cargo run -p fig_viewer --example probe_instance_text -- <project_dir> [wx] [wy]`

use std::{collections::HashMap, env, path::PathBuf};

use anyhow::{Result, anyhow};
use fanta_doc::{Doc, ExpandedNode, NodeData, NodeId, Transform2D, expand_instance};
use glam::DVec2;

fn clone_world(
    expanded: &[ExpandedNode],
    by_id: &HashMap<NodeId, usize>,
    idx: usize,
    w_inst: Transform2D,
) -> Transform2D {
    let mut chain: Vec<Transform2D> = Vec::new();
    let mut cursor = idx;
    loop {
        let node = &expanded[cursor].node;
        let Some(parent) = node.parent else { break };
        chain.push(node.transform);
        let Some(&p) = by_id.get(&parent) else { break };
        cursor = p;
    }
    let mut world = w_inst;
    for t in chain.iter().rev() {
        world = t.then(&world);
    }
    world
}

fn kind(data: &NodeData) -> &'static str {
    match data {
        NodeData::Group(_) => "Group",
        NodeData::Vector(_) => "Vector",
        NodeData::Text(_) => "Text",
        NodeData::Instance(_) => "Instance",
        _ => "Other",
    }
}

fn main() -> Result<()> {
    let dir = env::args()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("usage: probe_instance_text <project_dir> [wx] [wy]"))?;
    let px: f64 = env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(163.0);
    let py: f64 = env::args()
        .nth(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or(113.0);
    let point = DVec2::new(px, py);

    let (doc, _assets) = fanta_format::read_project_tree(&dir)?;
    let doc: Doc = doc;

    println!(
        "pages={:?} active={:?}",
        doc.pages().len(),
        doc.active_page()
    );
    println!("components: {}", doc.components.defs.len());
    for (id, def) in &doc.components.defs {
        println!(
            "  def {id:?} name={:?} root={:?} root_in_scene={}",
            def.name,
            def.root,
            doc.scene.get(def.root).is_some()
        );
    }

    let instances: Vec<NodeId> = doc
        .scene
        .descendants_of(
            doc.active_page()
                .unwrap_or_else(|| doc.scene.roots().first().copied().expect("a root")),
        )
        .filter(|id| {
            matches!(
                doc.scene.get(*id).map(|n| &n.data),
                Some(NodeData::Instance(_))
            )
        })
        .collect();
    println!("instances under active page: {}", instances.len());

    for inst_id in instances {
        let node = doc.scene.get(inst_id).expect("node");
        let NodeData::Instance(inst) = &node.data else {
            continue;
        };
        let w_inst = doc.scene.world_transform(inst_id).expect("world");
        println!(
            "\n=== instance {inst_id:?} derived={} overrides={} w_inst_origin={:?}",
            inst.derived.len(),
            inst.overrides.len(),
            w_inst.transform_point(DVec2::ZERO)
        );

        let mut expanded = expand_instance(&doc.scene, &doc.components, inst);
        println!("  expand_instance -> {} clones", expanded.len());
        if expanded.is_empty() {
            println!("  !! EMPTY EXPANSION (master missing?)");
            continue;
        }
        if inst.derived.is_empty() {
            fanta_doc::solve_expanded(&mut expanded, &mut fanta_render::measure_text_node);
            println!("  (ran solve_expanded)");
        }

        let by_id: HashMap<NodeId, usize> = expanded
            .iter()
            .enumerate()
            .map(|(i, e)| (e.node.id, i))
            .collect();

        let mut hit = None;
        for (i, e) in expanded.iter().enumerate() {
            let w = clone_world(&expanded, &by_id, i, w_inst);
            let origin = w.transform_point(DVec2::ZERO);
            let size = match &e.node.data {
                NodeData::Text(t) => Some(t.local_size),
                NodeData::Group(g) => g.clip_size,
                _ => None,
            };
            println!(
                "  [{i}] {:<8} def_path_len={} parent={:?} local_t={:?} world_origin={:?} size={:?}",
                kind(&e.node.data),
                e.def_path.len(),
                e.node.parent.is_some(),
                e.node.transform.transform_point(DVec2::ZERO),
                origin,
                size
            );
            if let NodeData::Text(t) = &e.node.data {
                println!("        content={:?} finite={}", t.content, w.is_finite());
                let local = w.inverse().transform_point(point);
                let [tw, th] = t.local_size;
                let inside = local.x >= 0.0 && local.x <= tw && local.y >= 0.0 && local.y <= th;
                println!(
                    "        point {point:?} -> local {local:?} box [0,0,{tw},{th}] inside={inside}"
                );
                if inside {
                    hit = Some(i);
                }
            }
        }
        match hit {
            Some(i) => println!("  => HIT text clone [{i}]"),
            None => println!("  => NO TEXT HIT at {point:?}"),
        }
    }
    Ok(())
}
