//! Check that a project folder parses, without writing anything:
//!
//! ```sh
//! cargo run -p fanta-format --example check_project -- <project-dir>
//! ```
//!
//! Prints the error the editor would show ("invalid project tree: …") or a
//! one-line summary of what loaded.

fn main() -> anyhow::Result<()> {
    let root = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: check_project <project-dir>"))?;
    let (doc, assets) = fanta_format::read_project_tree(std::path::Path::new(&root))?;
    println!(
        "ok: {} pages, {} nodes, {} components in {} sets, {} assets",
        doc.pages().len(),
        doc.scene.len(),
        doc.components.defs.len(),
        doc.components.sets.len(),
        assets.len()
    );
    Ok(())
}
