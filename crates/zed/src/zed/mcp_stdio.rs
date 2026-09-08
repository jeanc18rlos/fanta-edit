//! `fanta --mcp-stdio`: a stdio bridge onto the running app's live MCP server.
//!
//! Claude Code, Codex and every other MCP client we care about speak
//! newline-delimited JSON-RPC over stdin/stdout. The live server
//! (`fig_viewer::live_mcp`) listens on a Unix socket in a per-launch temp
//! directory and advertises the path in `<data_dir>/fanta_live_mcp.json` as
//! `{"socket": "…", "pid": N}`. This process reads that file and copies bytes
//! both ways, so a static MCP client config can point at the binary and still
//! reach whatever socket the current launch happens to be using.
//!
//! Nothing removes the discovery file when the app quits, so the recorded pid
//! is checked before the socket path next to it is trusted.

use std::fs;
use std::io::{self, Write as _};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::thread;

const NOT_RUNNING: &str =
    "Fanta is not running, or its live MCP server is off (settings: fanta_live_mcp.enabled)";

/// Runs the bridge to completion. Returns the process exit code.
pub fn run() -> i32 {
    let stream = match connect() {
        Some(stream) => stream,
        None => {
            eprintln!("{NOT_RUNNING}");
            return 2;
        }
    };

    match pump(stream) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("fanta --mcp-stdio: {error}");
            1
        }
    }
}

fn connect() -> Option<UnixStream> {
    let path = paths::data_dir().join("fanta_live_mcp.json");
    let contents = fs::read_to_string(&path).ok()?;
    let discovery: serde_json::Value = serde_json::from_str(&contents).ok()?;
    let socket = discovery.get("socket")?.as_str()?;
    let pid = discovery.get("pid")?.as_i64()?;
    if !process_is_alive(pid) {
        return None;
    }
    UnixStream::connect(socket).ok()
}

/// `kill(pid, 0)` reports whether the process still exists without signalling
/// it. `EPERM` means it exists but belongs to another user, which still counts.
fn process_is_alive(pid: i64) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    if pid <= 0 {
        return false;
    }
    // SAFETY: `kill` with signal 0 performs the permission and existence
    // checks only; it never delivers a signal and touches no memory.
    let result = unsafe { libc::kill(pid, 0) };
    result == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn pump(stream: UnixStream) -> io::Result<()> {
    let mut to_server = stream.try_clone()?;
    let mut from_server = stream;

    // The request side runs on its own thread and is never joined: it spends
    // its life blocked in a `read` on stdin, which nothing can interrupt. When
    // this function returns the process exits and takes the thread with it, so
    // the thread reports its own errors rather than handing them back.
    thread::spawn(move || {
        let copied = {
            let mut stdin = io::stdin().lock();
            io::copy(&mut stdin, &mut to_server).and_then(|_| to_server.flush())
        };
        // Stdin EOF only means the client has no more requests. Half-close so
        // the server sees the end of the request stream and can finish
        // answering, rather than tearing down responses already in flight.
        let closed = copied.and_then(|()| to_server.shutdown(Shutdown::Write));
        if let Err(error) = closed {
            eprintln!("fanta --mcp-stdio: writing to the live MCP socket: {error}");
        }
    });

    // Returning here means the app closed the connection, which ends the
    // bridge whether or not stdin is still open.
    let mut stdout = io::stdout().lock();
    io::copy(&mut from_server, &mut stdout)?;
    stdout.flush()
}
