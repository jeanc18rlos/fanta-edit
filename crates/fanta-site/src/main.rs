use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use clap::Parser;
use fanta_site::{FontSource, RuntimeAssets, SiteOptions};

/// Export a Fanta project as a static website: one read-only page per design
/// page, ready for any static host.
#[derive(Parser)]
#[command(name = "fanta-site")]
struct Arguments {
    /// The Fanta project directory to export.
    project: PathBuf,
    /// Directory to write the site into; created when missing. Re-exporting
    /// into a previous export replaces its pages and keeps other files.
    output: PathBuf,
    /// Built @fanta/runtime package directory, containing dist/, src/ and site/.
    #[arg(long)]
    runtime: PathBuf,
    /// Directory of font files that may be published with the site (repeatable).
    #[arg(long = "font-dir")]
    font_directories: Vec<PathBuf>,
    /// Render FAMILY with REPLACEMENT's faces, as FAMILY=REPLACEMENT (repeatable).
    #[arg(long = "substitute")]
    substitutions: Vec<String>,
    /// Never download fonts; families with no local face use a bundled substitute.
    #[arg(long)]
    offline: bool,
    /// Site title; defaults to the document title.
    #[arg(long)]
    title: Option<String>,
}

fn main() -> Result<()> {
    let arguments = Arguments::parse();
    let mut substitutions = BTreeMap::new();
    for substitution in &arguments.substitutions {
        let Some((family, replacement)) = substitution.split_once('=') else {
            bail!("--substitute expects FAMILY=REPLACEMENT, got {substitution:?}");
        };
        substitutions.insert(family.trim().to_owned(), replacement.trim().to_owned());
    }
    let assets = RuntimeAssets::read(&arguments.runtime).with_context(|| {
        format!(
            "--runtime must point at a built @fanta/runtime package from Fanta-JS (looked in {})",
            arguments.runtime.display()
        )
    })?;
    let options = SiteOptions {
        title: arguments.title,
        font_directories: arguments.font_directories,
        substitutions,
        download_fonts: !arguments.offline,
    };
    let report =
        fanta_site::export_project(&arguments.project, &arguments.output, &assets, &options)?;

    println!(
        "Exported {:?} to {}",
        report.title,
        arguments.output.display()
    );
    for page in &report.pages {
        println!("  page  {}/  {}", page.slug, page.name);
    }
    for face in &report.fonts {
        let style = if face.italic { " italic" } else { "" };
        let source = match &face.source {
            FontSource::File { path } => format!("file {}", path.display()),
            FontSource::Bundled => "bundled".to_owned(),
            FontSource::Downloaded => "downloaded".to_owned(),
            FontSource::Substituted { family } => format!("substituted with {family}"),
        };
        println!("  font  {} {}{}: {source}", face.family, face.weight, style);
    }
    for face in &report.fonts {
        if let FontSource::Substituted { family } = &face.source {
            eprintln!(
                "warning: {} {} renders with {family}; add its licensed files with --font-dir to keep the original typeface",
                face.family, face.weight
            );
        }
    }
    println!(
        "Runtime bundle: {} KiB. Upload the whole directory to any static host.",
        report.bundle_bytes.div_ceil(1024)
    );
    Ok(())
}
