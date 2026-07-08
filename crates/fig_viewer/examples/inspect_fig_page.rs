//! Dumps the node tree of one page of a .fig file, to a limited depth.
//!
//! usage: cargo run -p fig_viewer --example inspect_fig_page -- \
//!     <file.fig> <page-name-fragment> [--depth <n>] [--no-solve-layout]

use std::{env, fs, path::PathBuf};

use anyhow::{Context as _, Result, anyhow};
use fanta_doc::{Doc, NodeData, NodeFlags, NodeId};
use fanta_fig_interop::{fig_to_doc, read_fig};
use fanta_render::solve_scene_layout;

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let fig_path = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("missing .fig path"))?;
    let page_filter = args.next().ok_or_else(|| anyhow!("missing page name"))?;

    let mut max_depth = 3usize;
    let mut solve_layout = true;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--depth" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow!("--depth needs a value"))?;
                max_depth = value.parse().context("parsing --depth")?;
            }
            "--no-solve-layout" => solve_layout = false,
            other => return Err(anyhow!("unknown argument {other:?}")),
        }
    }

    let bytes = fs::read(&fig_path).with_context(|| format!("reading {}", fig_path.display()))?;
    let fig = read_fig(&bytes).context("parsing .fig")?;
    let (mut doc, _report, _assets) = fig_to_doc(&fig).context("mapping .fig")?;

    let page = find_page(&doc, &page_filter)?;
    if solve_layout {
        solve_scene_layout(&mut doc.scene, page);
    }
    dump(&doc, page, 0, max_depth);
    Ok(())
}

fn find_page(doc: &Doc, filter: &str) -> Result<NodeId> {
    let filter = filter.to_lowercase();
    doc.pages()
        .iter()
        .find(|page| {
            doc.scene
                .get(**page)
                .is_some_and(|node| node.name.to_lowercase().contains(&filter))
        })
        .copied()
        .ok_or_else(|| anyhow!("no page matching {filter:?}"))
}

fn dump(doc: &Doc, node_id: NodeId, depth: usize, max_depth: usize) {
    let Some(node) = doc.scene.get(node_id) else {
        return;
    };
    let kind = match &node.data {
        NodeData::Group(group) => {
            let auto = group
                .auto_layout
                .map(|auto_layout| format!(" auto={:?}", auto_layout.mode))
                .unwrap_or_default();
            format!("group{auto}")
        }
        NodeData::Vector(_) => "vector".to_string(),
        NodeData::Text(text) => format!("text {:?}", truncate(&text.content, 30)),
        NodeData::Instance(instance) => {
            let component = doc
                .components
                .def(instance.component)
                .map(|definition| definition.name.as_str())
                .unwrap_or("<missing>");
            format!(
                "instance of {:?} derived={}",
                component,
                instance.derived.len()
            )
        }
        other => format!("{other:?}").chars().take(20).collect(),
    };
    let world = doc
        .scene
        .world_bounds(node_id)
        .map(|bounds| {
            format!(
                " world=[{:.0},{:.0} {:.0}x{:.0}]",
                bounds.min_x,
                bounds.min_y,
                bounds.width(),
                bounds.height()
            )
        })
        .unwrap_or_else(|| " world=<none>".to_string());
    let hidden = if node.flags.contains(NodeFlags::HIDDEN) {
        " HIDDEN"
    } else {
        ""
    };
    let mask = if node.is_mask { " MASK" } else { "" };
    let child_count = doc.scene.children_of(Some(node_id)).len();
    println!(
        "{}{:?} [{kind}]{world}{hidden}{mask} children={child_count}",
        "  ".repeat(depth),
        node.name,
    );
    if depth >= max_depth {
        return;
    }
    for child in doc.scene.children_of(Some(node_id)) {
        dump(doc, *child, depth + 1, max_depth);
    }
}

fn truncate(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        value.to_string()
    } else {
        let prefix: String = value.chars().take(limit).collect();
        format!("{prefix}…")
    }
}
