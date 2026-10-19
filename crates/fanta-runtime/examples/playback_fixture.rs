use std::error::Error;
use std::io::{self, Read, Write};

use fanta_doc::NodeId;
use fanta_format::runtime::SourceBundle;
use fanta_present::FitMode;
use fanta_runtime::Runtime;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct PlaybackFixture {
    source: SourceBundle,
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
    let mut runtime = Runtime::default();
    let program = runtime.compile_source(&fixture.source)?;
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
    serde_json::to_writer(&mut output, &results)?;
    writeln!(output)?;
    Ok(())
}
