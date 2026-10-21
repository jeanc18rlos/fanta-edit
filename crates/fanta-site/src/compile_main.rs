use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Parser;
use fanta_site::{FontSource, SiteOptions};

/// Compile a Fanta project into one runtime bundle. `fanta-runtime` loads the
/// bundle with `loadBundle` and has no source compiler, so this is the build
/// step between a saved project and a page that plays it.
#[derive(Parser)]
#[command(name = "fanta-compile")]
struct Arguments {
    /// The Fanta project directory to compile.
    project: PathBuf,
    /// The bundle file to write, conventionally `program.fanta`. Missing parent
    /// directories are created.
    output: PathBuf,
    /// Directory of font files that may be embedded in the bundle (repeatable).
    #[arg(long = "font-dir")]
    font_directories: Vec<PathBuf>,
    /// Render FAMILY with REPLACEMENT's faces, as FAMILY=REPLACEMENT (repeatable).
    #[arg(long = "substitute")]
    substitutions: Vec<String>,
    /// Never download fonts; families with no local face use a bundled substitute.
    #[arg(long)]
    offline: bool,
}

fn main() -> Result<()> {
    let arguments = Arguments::parse();
    let mut substitutions = BTreeMap::new();
    for substitution in &arguments.substitutions {
        let Some((family, replacement)) = fanta_site::parse_substitution(substitution) else {
            bail!("--substitute expects FAMILY=REPLACEMENT, got {substitution:?}");
        };
        substitutions.insert(family, replacement);
    }
    let options = SiteOptions {
        title: None,
        font_directories: arguments.font_directories,
        substitutions,
        download_fonts: !arguments.offline,
    };
    let report = fanta_site::export_bundle(&arguments.project, &arguments.output, &options)?;

    println!(
        "Compiled {:?} to {} ({} KiB)",
        report.title,
        arguments.output.display(),
        report.bundle_bytes.div_ceil(1024)
    );
    for page in &report.pages {
        println!("  page  {}", page.name);
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
    Ok(())
}
