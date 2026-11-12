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
    /// Export plain web files instead of the runtime site: one SVG per page,
    /// tokens.css, and a dependency-free HTML+JS viewer. Needs no --runtime.
    #[arg(long)]
    plain: bool,
    /// Built @fanta/runtime package directory, containing dist/, src/ and site/.
    #[arg(long, required_unless_present = "plain")]
    runtime: Option<PathBuf>,
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
    if arguments.plain {
        let report =
            fanta_site::web::export_web(&arguments.project, Some(&arguments.output))?;
        println!("Exported plain web files to {}", arguments.output.display());
        for page in &report.pages {
            match &page.error {
                None => println!("  page  {}  {}", page.slug, page.name),
                Some(error) => eprintln!("warning: page {} was not exported: {error}", page.name),
            }
        }
        for skipped in &report.skipped_variables {
            eprintln!(
                "warning: variable {}/{} has no CSS form: {}",
                skipped.collection, skipped.variable, skipped.reason
            );
        }
        for warning in &report.warnings {
            eprintln!("warning: {warning}");
        }
        return Ok(());
    }
    let mut substitutions = BTreeMap::new();
    for substitution in &arguments.substitutions {
        let Some((family, replacement)) = fanta_site::parse_substitution(substitution) else {
            bail!("--substitute expects FAMILY=REPLACEMENT, got {substitution:?}");
        };
        substitutions.insert(family, replacement);
    }
    let runtime = arguments
        .runtime
        .as_deref()
        .context("--runtime is required unless --plain is given")?;
    let assets = RuntimeAssets::read(runtime).with_context(|| {
        format!(
            "--runtime must point at a built @fanta/runtime package from Fanta-JS (looked in {})",
            runtime.display()
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
        println!(
            "  font  {} {}{}: {}",
            face.family, face.weight, style, face.source
        );
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
