//! `fanta --mcp-stdio`: a stdio bridge onto the running app's live MCP server.
//!
//! Claude Code, Codex and every other MCP client we care about speak
//! newline-delimited JSON-RPC over stdin/stdout. The live server
//! (`fig_viewer::live_mcp`) listens on a Unix socket in a per-launch temp
//! directory (an AF_UNIX socket on Windows) and advertises the path in
//! `<data_dir>/fanta_live_mcp.json` as `{"socket": "…", "pid": N}`. This
//! process reads that file and copies bytes both ways, so a static MCP client
//! config can point at the binary and still reach whatever socket the current
//! launch happens to be using.
//!
//! The recorded pid is checked before the socket path is trusted because a
//! crash can leave the discovery file behind.

use std::fs;
use std::io::{self, Write as _};
use std::thread;
use std::time::{Duration, Instant};

use net::UnixStream;

const NOT_RUNNING: &str =
    "Fanta is not running, or its live MCP server is off (settings: fanta_live_mcp.enabled)";

/// Runs the bridge to completion. Returns the process exit code.
pub fn run() -> i32 {
    let stream = match connect_with_retry() {
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

fn connect_with_retry() -> Option<UnixStream> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(stream) = connect() {
            return Some(stream);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(50));
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
#[cfg(unix)]
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

/// Windows has no `kill(pid, 0)`. A crashed app leaves its socket file with no
/// listener, so the `connect` that follows fails and the stale file is ignored.
#[cfg(windows)]
fn process_is_alive(pid: i64) -> bool {
    pid > 0
}

fn pump(stream: UnixStream) -> io::Result<()> {
    let (from_server, mut to_server) = split(stream)?;

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
        let closed = copied.and_then(|()| to_server.finish());
        if let Err(error) = closed {
            eprintln!("fanta --mcp-stdio: writing to the live MCP socket: {error}");
        }
    });

    // Returning here means the app closed the connection, which ends the
    // bridge whether or not stdin is still open.
    forward(from_server, io::stdout().lock())
}

/// Copies responses to the client as they arrive.
///
/// Not `io::copy`: on Linux it moves bytes from a socket to a pipe with
/// `splice`, which keeps the pipe locked while it waits for the next message.
/// A client reading that pipe then gets no response until the stream ends.
fn forward(mut from_server: impl io::Read, mut to_client: impl io::Write) -> io::Result<()> {
    let mut buffer = [0u8; 8192];
    loop {
        let read = match from_server.read(&mut buffer) {
            Ok(0) => return to_client.flush(),
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        let Some(response) = buffer.get(..read) else {
            return Err(io::Error::other("the live MCP socket read past its buffer"));
        };
        to_client.write_all(response)?;
        to_client.flush()?;
    }
}

/// The request direction of the connection. `finish` half-closes it so the
/// server reads the end of the request stream while responses keep flowing.
#[cfg(unix)]
struct RequestWriter(UnixStream);

#[cfg(unix)]
fn split(stream: UnixStream) -> io::Result<(UnixStream, RequestWriter)> {
    let writer = stream.try_clone()?;
    Ok((stream, RequestWriter(writer)))
}

#[cfg(unix)]
impl RequestWriter {
    fn finish(&self) -> io::Result<()> {
        self.0.shutdown(std::net::Shutdown::Write)
    }
}

#[cfg(unix)]
impl io::Write for RequestWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// The request direction of the connection. `finish` half-closes it so the
/// server reads the end of the request stream while responses keep flowing.
#[cfg(windows)]
struct RequestWriter {
    writer: net::OwnedWriteHalf,
    /// The socket both halves share; `writer` keeps it open while this exists.
    socket: std::os::windows::io::RawSocket,
}

#[cfg(windows)]
fn split(stream: UnixStream) -> io::Result<(net::OwnedReadHalf, RequestWriter)> {
    use std::os::windows::io::{AsRawSocket as _, AsSocket as _};

    let socket = stream.as_socket().as_raw_socket();
    let (reader, writer) = stream.into_split();
    Ok((reader, RequestWriter { writer, socket }))
}

#[cfg(windows)]
impl RequestWriter {
    fn finish(&self) -> io::Result<()> {
        use windows::Win32::Networking::WinSock::{
            SD_SEND, SOCKET, SOCKET_ERROR, WSAGetLastError, shutdown,
        };

        // SAFETY: `writer` shares ownership of the socket, so the handle is
        // still open, and `shutdown` only ends its send direction.
        let result = unsafe { shutdown(SOCKET(self.socket as usize), SD_SEND) };
        if result == SOCKET_ERROR {
            // SAFETY: reads the calling thread's most recent WinSock error.
            return Err(io::Error::from_raw_os_error(unsafe { WSAGetLastError() }.0));
        }
        Ok(())
    }
}

#[cfg(windows)]
impl io::Write for RequestWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.writer.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::forward;
    use std::io::{Read as _, Write as _};
    use std::os::unix::net::UnixStream;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn a_response_reaches_the_client_while_the_stream_stays_open() {
        let (mut app, bridge) = UnixStream::pair().expect("socket pair");
        let (mut client, to_client) = std::io::pipe().expect("pipe");
        let forwarding = thread::spawn(move || forward(bridge, to_client));

        app.write_all(b"{\"id\":1}\n").expect("the app writes a response");

        // Read on a thread so that a response held back fails the test instead of hanging it.
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut response = [0u8; 9];
            let read = client.read_exact(&mut response).map(|()| response);
            sender.send(read).expect("the test is still waiting");
        });
        let response = receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("the response arrives while the app keeps the stream open")
            .expect("the client reads the response");
        assert_eq!(&response, b"{\"id\":1}\n");

        drop(app);
        forwarding
            .join()
            .expect("the forwarding thread ends")
            .expect("forwarding ends cleanly when the app closes the stream");
    }
}
