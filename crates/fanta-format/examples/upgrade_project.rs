//! Bring a project folder up to the current layout now:
//!
//! ```sh
//! cargo run -p fanta-format --example upgrade_project -- <project-dir>
//! ```
//!
//! Close the project in the editor first. A project at the current layout is
//! left untouched; otherwise every file moves to where the current layout
//! keeps it (layout v5: each component set's variants into the set's folder).

fn main() -> anyhow::Result<()> {
    let root = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: upgrade_project <project-dir>"))?;
    let upgrade = fanta_format::upgrade_project(std::path::Path::new(&root))?;
    if upgrade.from == upgrade.to {
        println!("already at layout v{}; nothing to do", upgrade.to);
    } else {
        println!("upgraded layout v{} -> v{}", upgrade.from, upgrade.to);
    }
    Ok(())
}
