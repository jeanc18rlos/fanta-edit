//! Convert a `.fig` file into an on-disk `fanta-project` directory using the
//! same pipeline as the editor's "Create Fanta Project" button, then read the
//! tree back to verify the round trip.
//!
//! Usage: `cargo run -p fig_viewer --example fig_to_project -- <file.fig> [out_dir]`

use std::{env, fs, path::PathBuf};

use anyhow::{Context as _, Result, anyhow};
use fanta_fig_interop::{fig_to_doc, read_fig};

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let fig_path = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("missing .fig path"))?;
    let out_dir = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| fig_path.with_extension(""));

    let bytes = fs::read(&fig_path).with_context(|| format!("reading {}", fig_path.display()))?;
    let fig = read_fig(&bytes).context("parsing .fig")?;
    let (doc, report, assets) = fig_to_doc(&fig).context("mapping .fig to Fanta document")?;
    let assets = assets.into_iter().collect();

    fanta_format::scaffold_project_tree(&out_dir)
        .with_context(|| format!("scaffolding {}", out_dir.display()))?;
    fanta_format::write_project_tree(&out_dir, &doc, &assets)
        .with_context(|| format!("writing {}", out_dir.display()))?;

    let (reloaded, reloaded_assets) =
        fanta_format::read_project_tree(&out_dir).context("re-reading project tree")?;

    println!("project written to {}", out_dir.display());
    println!(
        "mapped {} nodes, {} pages, {} components, {} assets",
        report.mapped,
        doc.pages().len(),
        doc.components.defs.len(),
        assets.len(),
    );
    println!(
        "round trip: {} pages, {} assets re-read",
        reloaded.pages().len(),
        reloaded_assets.len(),
    );
    if reloaded.pages().len() != doc.pages().len() || reloaded_assets.len() != assets.len() {
        return Err(anyhow!("round trip mismatch"));
    }
    Ok(())
}
