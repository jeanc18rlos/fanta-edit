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
    /// Answer `GET /healthz` and `POST /v1/run` on `0.0.0.0:$PORT` (8080 by
    /// default). Needs `ENGINE_HMAC_SECRET`; `FANTA_ENGINE_GITHUB_API`
    /// overrides `https://api.github.com`.
    Serve,
}

fn main() -> Result<()> {
    match Arguments::parse().command {
        Command::Run { request } => {
            let bytes = std::fs::read(&request)
                .with_context(|| format!("reading {}", request.display()))?;
            let request: RunRequest = serde_json::from_slice(&bytes)
                .with_context(|| format!("parsing {}", request.display()))?;
            let work_dir = tempfile::tempdir()?;
            let project = work_dir.path().join("project");
            match &request.source {
                Source::Dir { dir } => {
                    std::fs::create_dir(&project)?;
                    copy_project(dir, &project)?;
                }
                Source::Github {
                    repo,
                    sha,
                    path,
                    token,
                    overlay,
                } => {
                    fanta_engine::validate(&request)?;
                    let github = fanta_engine::source::Github::new(
                        std::env::var("FANTA_ENGINE_GITHUB_API")
                            .unwrap_or_else(|_| "https://api.github.com".into()),
                    );
                    github.fetch(repo, sha, path, token, &project)?;
                    if let Some(overlay) = overlay {
                        fanta_engine::source::apply_overlay(overlay, &project)?;
                    }
                }
            }
            match fanta_engine::server::run_isolated(&request, &project) {
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
        Command::Serve => {
            let secret = std::env::var("ENGINE_HMAC_SECRET").unwrap_or_default();
            if secret.is_empty() {
                bail!("ENGINE_HMAC_SECRET must be set: every run request is signed with it");
            }
            let port = std::env::var("PORT").unwrap_or_else(|_| "8080".into());
            let github = std::env::var("FANTA_ENGINE_GITHUB_API")
                .unwrap_or_else(|_| "https://api.github.com".into());
            let server = tiny_http::Server::http(format!("0.0.0.0:{port}"))
                .map_err(|error| anyhow::anyhow!("listening on port {port}: {error}"))?;
            eprintln!("fanta-engine listening on port {port}");
            fanta_engine::server::serve(
                server,
                fanta_engine::server::Config {
                    secret: secret.into_bytes(),
                    github: fanta_engine::source::Github::new(github),
                },
            );
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
