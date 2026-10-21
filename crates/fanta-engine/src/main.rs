use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::{Parser, Subcommand};
use fanta_engine::{EngineInfo, RunRequest, Source};

/// Fanta's cloud engine: runs design steps on a project without the app.
#[derive(Parser)]
#[command(name = "fanta-engine")]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run one request file, as `POST /v1/run` would, and print the response.
    Run {
        #[arg(long)]
        request: PathBuf,
    },
    /// Print what `GET /healthz` answers.
    Healthz,
}

fn main() -> Result<()> {
    match Arguments::parse().command {
        Command::Run { request } => {
            let bytes = std::fs::read(&request)
                .with_context(|| format!("reading {}", request.display()))?;
            let request: RunRequest = serde_json::from_slice(&bytes)
                .with_context(|| format!("parsing {}", request.display()))?;
            let work_dir = tempfile::tempdir()?;
            match &request.source {
                Source::Dir { dir } => copy_project(dir, work_dir.path())?,
                Source::Github { .. } => bail!("reading GitHub sources is not implemented yet"),
            }
            match fanta_engine::run_in(&request, work_dir.path()) {
                Ok(response) => println!("{}", serde_json::to_string_pretty(&response)?),
                Err(error) => {
                    println!("{}", serde_json::to_string_pretty(&error)?);
                    std::process::exit(1);
                }
            }
        }
        Command::Healthz => {
            println!("{}", serde_json::to_string_pretty(&EngineInfo::current())?);
        }
    }
    Ok(())
}

/// Copy a project folder for a run to work on, leaving hidden entries such as
/// `.git` behind. Links are refused so a run can only read inside the folder.
fn copy_project(source: &Path, destination: &Path) -> Result<()> {
    for entry in
        std::fs::read_dir(source).with_context(|| format!("reading {}", source.display()))?
    {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let file_type = entry.file_type()?;
        let target = destination.join(entry.file_name());
        if file_type.is_symlink() {
            bail!(
                "{} is a link; project folders may not contain links",
                entry.path().display()
            );
        } else if file_type.is_dir() {
            std::fs::create_dir(&target)?;
            copy_project(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)
                .with_context(|| format!("copying {}", entry.path().display()))?;
        }
    }
    Ok(())
}
