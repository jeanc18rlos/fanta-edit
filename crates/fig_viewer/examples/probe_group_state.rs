//! Prints clip/background state for groups whose name contains a filter, to
//! diagnose fidelity issues (frame clip vs background AA seams).
//!
//! usage: cargo run -p fig_viewer --example probe_group_state -- <file.fig> <name-fragment>

use std::{env, fs, path::PathBuf};

use anyhow::{Context as _, Result, anyhow};
use fanta_doc::NodeData;
use fanta_fig_interop::{fig_to_doc, read_fig};

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let fig_path = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("missing .fig path"))?;
    let filter = args
        .next()
        .ok_or_else(|| anyhow!("missing name fragment"))?
        .to_lowercase();

    let bytes = fs::read(&fig_path).with_context(|| format!("reading {}", fig_path.display()))?;
    let fig = read_fig(&bytes).context("parsing .fig")?;
    let (doc, _report, _assets) = fig_to_doc(&fig).context("mapping .fig")?;

    let mut stack: Vec<fanta_doc::NodeId> = doc.scene.roots().to_vec();
    while let Some(id) = stack.pop() {
        stack.extend(doc.scene.children_of(Some(id)).iter().copied());
        let Some(node) = doc.scene.get(id) else {
            continue;
        };
        if !node.name.to_lowercase().contains(&filter) {
            continue;
        }
        match &node.data {
            NodeData::Group(g) => {
                println!(
                    "{:?} group name={:?} clip_size={:?} clip_content={:?} figma_type={:?} bg={:?} bg_fills={} strokes={} radius={:?}",
                    id,
                    node.name,
                    g.clip_size,
                    node.meta.get("clip_content"),
                    node.meta.get("figma_type"),
                    g.background,
                    g.background_fills.len(),
                    g.strokes.len(),
                    g.corner_radius,
                );
            }
            _ => {
                println!("{:?} {:?} (non-group)", id, node.name);
            }
        }
    }
    Ok(())
}
