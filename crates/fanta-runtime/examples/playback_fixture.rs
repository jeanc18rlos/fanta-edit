use std::error::Error;
use std::io::{self, Read, Write};

use fanta_format::runtime::SourceBundle;
use fanta_runtime::Runtime;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct PlaybackFixture {
    source: SourceBundle,
    commands: Vec<PlaybackCommand>,
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
    let player = runtime.create_player(program, 320.0, 180.0, None)?;
    let mut results = Vec::with_capacity(fixture.commands.len());
    for command in fixture.commands {
        results.push(runtime.command(player, &command.command, command.arguments)?);
    }
    let mut output = io::stdout().lock();
    serde_json::to_writer(&mut output, &results)?;
    writeln!(output)?;
    Ok(())
}
