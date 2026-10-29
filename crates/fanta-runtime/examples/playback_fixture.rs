//! Native playback reference for the Fanta-JS parity tests.
//!
//! Reads one JSON object from stdin with `commands`, optional `options`, and
//! either `source` (a `SourceBundle`, compiled here with the native compiler)
//! or `bundle` (a base64 runtime bundle). Plays the commands and prints
//! `{"bundle": "<base64>", "results": [...]}`: the WebAssembly runtime only
//! loads bundles, so the browser side replays the exact bytes this used.

use std::error::Error;
use std::io::{self, Read, Write};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use fanta_doc::NodeId;
use fanta_format::runtime::SourceBundle;
use fanta_present::FitMode;
use fanta_runtime::Runtime;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct PlaybackFixture {
    #[serde(default)]
    source: Option<SourceBundle>,
    #[serde(default)]
    bundle: Option<String>,
    #[serde(default)]
    options: PlaybackOptions,
    commands: Vec<PlaybackCommand>,
}

#[derive(Deserialize)]
#[serde(default)]
struct PlaybackOptions {
    width: f64,
    height: f64,
    start: Option<NodeId>,
    fit: FitMode,
}

impl Default for PlaybackOptions {
    fn default() -> Self {
        Self {
            width: 320.0,
            height: 180.0,
            start: None,
            fit: FitMode::Contain,
        }
    }
}

#[derive(Deserialize)]
struct PlaybackCommand {
    command: String,
    #[serde(default)]
    arguments: Value,
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut input = Vec::new();
    io::stdin().lock().read_to_end(&mut input)?;
    let fixture: PlaybackFixture = serde_json::from_slice(&input)?;
    let bundle = match (fixture.source, fixture.bundle) {
        (Some(source), None) => source.compile()?.to_bundle()?,
        (None, Some(encoded)) => STANDARD.decode(encoded)?,
        _ => return Err("the fixture needs exactly one of `source` or `bundle`".into()),
    };
    let mut runtime = Runtime::default();
    let program = runtime.load_bundle(&bundle)?;
    let options = fixture.options;
    let player = runtime.create_player_with_fit(
        program,
        options.width,
        options.height,
        options.start,
        options.fit,
    )?;
    let mut results = Vec::with_capacity(fixture.commands.len());
    for command in fixture.commands {
        results.push(runtime.command(player, &command.command, command.arguments)?);
    }
    let mut output = io::stdout().lock();
    serde_json::to_writer(
        &mut output,
        &json!({"bundle": STANDARD.encode(&bundle), "results": results}),
    )?;
    writeln!(output)?;
    Ok(())
}
