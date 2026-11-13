//! `fanta-engine serve`: the HTTP API of `docs/cloud-engine-contracts.md`.
//!
//! Each connection carries one request and is then closed, so an unread body
//! goes with the socket instead of being drained. Headers and bodies have read
//! deadlines, the signature header is checked before any body is read, and
//! connections beyond the limit are closed as they arrive. Runs go to blocking
//! threads, each with its own temp folder, and a panic fails only its request.

use std::{
    convert::Infallible,
    net::TcpListener,
    panic::AssertUnwindSafe,
    path::Path,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use bytes::Bytes;
use hmac::{Hmac, Mac as _};
use http_body_util::{BodyExt as _, Full, LengthLimitError, Limited};
use hyper::{
    Method, Request, Response, StatusCode, body::Incoming, header, server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use tokio::sync::Semaphore;

use crate::{
    EngineError, EngineInfo, ErrorCode, RunRequest, RunResponse, Source,
    source::{self, Github},
};

/// The largest request body the engine reads.
pub const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;
/// How far a signature's timestamp may be from the engine's clock.
pub const MAX_CLOCK_SKEW_SECONDS: u64 = 300;
/// Runs one instance takes at once; more answer `429 busy`.
pub const MAX_CONCURRENT_RUNS: usize = 2;
/// Connections one instance holds at once, `/healthz` included. Beyond this a
/// new connection is closed unanswered, since answering means reading it.
///
/// A caller with a well-formed, fresh but forged signature header can hold
/// a connection until the body timeout, because the HMAC covers the body.
/// That is acceptable only because Cloud Run admits nobody but v2's service
/// account (`--no-allow-unauthenticated`, `roles/run.invoker`) and routes at
/// most 2 requests to an instance. Before exposing the engine any other way,
/// sign the timestamp and `Content-Length` in the header as well, which is a
/// contract change made together with the backend.
pub const MAX_OPEN_CONNECTIONS: usize = 8;
/// How long a client may take to send its request headers.
pub const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a client may take to send its request body.
pub const BODY_TIMEOUT: Duration = Duration::from_secs(30);
/// The longest one connection lives: the longest run plus its download.
/// Cloud Run ends requests at 300 s anyway.
pub const CONNECTION_TIMEOUT: Duration = Duration::from_secs(320);

pub struct Config {
    /// `ENGINE_HMAC_SECRET`, shared with the v2 API.
    pub secret: Vec<u8>,
    pub github: Github,
}

struct State {
    config: Config,
    runs: Arc<Semaphore>,
}

/// Answers requests on `listener` until the process ends.
pub fn serve(listener: TcpListener, config: Config) -> std::io::Result<()> {
    listener.set_nonblocking(true)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let listener = {
        let _context = runtime.enter();
        tokio::net::TcpListener::from_std(listener)?
    };
    let state = Arc::new(State {
        config,
        runs: Arc::new(Semaphore::new(MAX_CONCURRENT_RUNS)),
    });
    runtime.block_on(accept(listener, state));
    Ok(())
}

async fn accept(listener: tokio::net::TcpListener, state: Arc<State>) {
    let connections = Arc::new(Semaphore::new(MAX_OPEN_CONNECTIONS));
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(error) => {
                eprintln!("accepting a connection failed: {error}");
                continue;
            }
        };
        let Ok(permit) = connections.clone().try_acquire_owned() else {
            drop(stream);
            continue;
        };
        let state = state.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let service = service_fn(move |request| {
                let state = state.clone();
                async move { Ok::<_, Infallible>(handle(request, state).await) }
            });
            let connection = http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(HEADER_TIMEOUT)
                .keep_alive(false)
                .serve_connection(TokioIo::new(stream), service);
            match tokio::time::timeout(CONNECTION_TIMEOUT, connection).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => eprintln!("a connection failed: {error}"),
                Err(_) => eprintln!("closed a connection after {CONNECTION_TIMEOUT:?}"),
            }
        });
    }
}

async fn handle(request: Request<Incoming>, state: Arc<State>) -> Response<Full<Bytes>> {
    let started = Instant::now();
    // Requests carry a token, so log only the route.
    let route = format!("{} {}", request.method(), request.uri().path());
    let answer = match (request.method(), request.uri().path()) {
        (&Method::GET, "/healthz") => json(&EngineInfo::current()),
        (&Method::POST, "/v1/run") => match run(request, state, started).await {
            Ok(response) => json(&response),
            Err(error) => Err(error),
        },
        _ => Err(EngineError::new(ErrorCode::NotFound, "no such route")),
    };
    let (status, body) = answer.unwrap_or_else(|error| {
        (
            error.error.code.status(),
            serde_json::to_vec(&error).unwrap_or_default(),
        )
    });
    eprintln!("{route} {status} {} ms", started.elapsed().as_millis());
    let mut response = Response::new(Full::new(Bytes::from(body)));
    *response.status_mut() =
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/json"),
    );
    response
}

fn json(value: &impl Serialize) -> Result<(u16, Vec<u8>), EngineError> {
    serde_json::to_vec(value)
        .map(|body| (200, body))
        .map_err(|error| EngineError::new(ErrorCode::Internal, error.to_string()))
}

async fn run(
    request: Request<Incoming>,
    state: Arc<State>,
    started: Instant,
) -> Result<RunResponse, EngineError> {
    let unauthorized = || {
        EngineError::new(
            ErrorCode::Unauthorized,
            "missing, malformed, stale or wrong X-Fanta-Internal",
        )
    };
    let too_large = || {
        EngineError::new(
            ErrorCode::PayloadTooLarge,
            format!("the request body is over {MAX_BODY_BYTES} bytes"),
        )
    };
    // Everything that needs no body is checked first, so a request that can't
    // be ours costs no read and no buffer.
    let now = unix_now();
    let signature = request
        .headers()
        .get("x-fanta-internal")
        .and_then(|value| value.to_str().ok())
        .and_then(parse_signature)
        .filter(|(timestamp, _)| now.abs_diff(*timestamp) <= MAX_CLOCK_SKEW_SECONDS)
        .ok_or_else(unauthorized)?;
    if request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > MAX_BODY_BYTES as u64)
    {
        return Err(too_large());
    }
    let body = Limited::new(request.into_body(), MAX_BODY_BYTES).collect();
    let body = match tokio::time::timeout(BODY_TIMEOUT, body).await {
        Err(_) => {
            return Err(EngineError::new(
                ErrorCode::BadRequest,
                format!("the request body took over {BODY_TIMEOUT:?}"),
            ));
        }
        Ok(Err(error)) if error.downcast_ref::<LengthLimitError>().is_some() => {
            return Err(too_large());
        }
        Ok(Err(error)) => {
            return Err(EngineError::new(ErrorCode::BadRequest, error.to_string()));
        }
        Ok(Ok(collected)) => collected.to_bytes(),
    };
    if !signature_matches(&state.config.secret, &signature, &body) {
        return Err(unauthorized());
    }
    let run_request: RunRequest = serde_json::from_slice(&body)
        .map_err(|error| EngineError::new(ErrorCode::BadRequest, error.to_string()))?;
    crate::validate(&run_request)?;
    if !matches!(run_request.source, Source::Github { .. }) {
        return Err(EngineError::new(
            ErrorCode::BadRequest,
            "dir sources are for `fanta-engine run` only",
        ));
    }
    let permit = state.runs.clone().try_acquire_owned().map_err(|_| {
        EngineError::new(
            ErrorCode::Busy,
            format!("the instance is already running {MAX_CONCURRENT_RUNS} requests"),
        )
    })?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        fetch_and_run(&state.config, run_request, started)
    })
    .await
    .unwrap_or_else(|error| {
        Err(EngineError::new(
            ErrorCode::Internal,
            format!("the run's thread failed: {error}"),
        ))
    })
}

fn fetch_and_run(
    config: &Config,
    mut run_request: RunRequest,
    started: Instant,
) -> Result<RunResponse, EngineError> {
    let Source::Github {
        repo,
        sha,
        path,
        token,
        overlay,
    } = &run_request.source
    else {
        return Err(EngineError::new(
            ErrorCode::BadRequest,
            "dir sources are for `fanta-engine run` only",
        ));
    };
    let work = tempfile::tempdir()
        .map_err(|error| EngineError::new(ErrorCode::Internal, error.to_string()))?;
    let project = work.path().join("project");
    // shortcut: every run downloads its commit; the contract's three-checkout
    // cache is skipped. Add it when cold downloads miss a tool's deadline.
    config.github.fetch(repo, sha, path, token, &project)?;
    if let Some(overlay) = overlay {
        source::apply_overlay(overlay, &project)?;
    }
    let sha = sha.clone();
    let deadline = Duration::from_millis(run_request.deadline_ms);
    let Some(remaining) = deadline
        .checked_sub(started.elapsed())
        .filter(|left| !left.is_zero())
    else {
        return Err(EngineError::new(
            ErrorCode::DeadlineExceeded,
            "the deadline passed while the source downloaded",
        ));
    };
    run_request.deadline_ms = remaining.as_millis().max(1) as u64;
    let mut response = run_isolated(&run_request, &project)?;
    response.source.sha = Some(sha);
    Ok(response)
}

/// Runs the steps, turning a panic into a 500 for this request alone.
pub fn run_isolated(request: &RunRequest, project: &Path) -> Result<RunResponse, EngineError> {
    std::panic::catch_unwind(AssertUnwindSafe(|| crate::run_in(request, project))).unwrap_or_else(
        |_| {
            Err(EngineError::new(
                ErrorCode::Internal,
                "the run panicked; the engine logged it",
            ))
        },
    )
}

/// Checks `t=<unix seconds>,v1=<hex>`, where `v1` is
/// `HMAC-SHA256(secret, "{t}.{hex sha256(body)}")`, in constant time. The
/// v2 API signs with `fanta_gpu_common.auth.sign`.
pub fn verify_signature(secret: &[u8], header: &str, body: &[u8], now: u64) -> bool {
    parse_signature(header)
        .filter(|(timestamp, _)| now.abs_diff(*timestamp) <= MAX_CLOCK_SKEW_SECONDS)
        .is_some_and(|signature| signature_matches(secret, &signature, body))
}

/// The header's timestamp and signature bytes, when it is well formed.
fn parse_signature(header: &str) -> Option<(u64, Vec<u8>)> {
    let (mut timestamp, mut signature) = (None, None);
    for part in header.split(',') {
        match part.split_once('=') {
            Some(("t", value)) => timestamp = value.parse::<u64>().ok(),
            Some(("v1", value)) => signature = hex::decode(value).ok(),
            _ => return None,
        }
    }
    timestamp.zip(signature)
}

fn signature_matches(secret: &[u8], (timestamp, signature): &(u64, Vec<u8>), body: &[u8]) -> bool {
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret) else {
        return false;
    };
    mac.update(format!("{timestamp}.{}", hex::encode(Sha256::digest(body))).as_bytes());
    mac.verify_slice(signature).is_ok()
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::tests::{Entry, tarball};
    use serde_json::json;
    use std::io::{Read as _, Write as _};

    const SECRET: &[u8] = b"engine-test-secret";

    fn sign(body: &[u8], timestamp: u64) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(SECRET).expect("key");
        mac.update(format!("{timestamp}.{}", hex::encode(Sha256::digest(body))).as_bytes());
        format!("t={timestamp},v1={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn signatures_follow_the_backends_scheme() {
        // Computed with `fanta_gpu_common.auth.sign(secret, body, t)`.
        let body = br#"{"request_id":"r"}"#;
        let header = "t=1700000000,v1=e975c61657c2a51a2d71b0f73615c9c7fd1e95cd4f7b863b53cdaf30163d7746";
        assert!(verify_signature(SECRET, header, body, 1_700_000_000));
        assert!(verify_signature(SECRET, header, body, 1_700_000_300));
        assert!(!verify_signature(SECRET, header, body, 1_700_000_301));
        assert!(!verify_signature(SECRET, header, br#"{"request_id":"s"}"#, 1_700_000_000));
        assert!(!verify_signature(b"other", header, body, 1_700_000_000));
        for malformed in ["", "t=1700000000", "v1=00", "t=x,v1=00", "t=1,v1=zz", "t=1,v1=00,x=1"] {
            assert!(!verify_signature(SECRET, malformed, body, 1), "{malformed}");
        }
    }

    /// A GitHub stand-in serving one commit of `acme/site` with the project
    /// in `design/`.
    fn fake_github(project: &Path, sha: &'static str) -> String {
        let mut entries = Vec::new();
        for file in walk(project) {
            let relative = file.strip_prefix(project).expect("inside the project");
            entries.push((
                format!("design/{}", relative.to_string_lossy()),
                std::fs::read(&file).expect("project file"),
            ));
        }
        let archive = tarball(
            &entries
                .iter()
                .map(|(path, bytes)| Entry::File(path, bytes))
                .collect::<Vec<_>>(),
        );
        let server = tiny_http::Server::http("127.0.0.1:0").expect("fake GitHub");
        let address = format!("http://{}", server.server_addr());
        std::thread::spawn(move || {
            for request in server.incoming_requests() {
                let authorized = request.headers().iter().any(|header| {
                    header.field.equiv("Authorization") && header.value.as_str() == "Bearer ghs_test"
                });
                let response = match request.url() {
                    _ if !authorized => tiny_http::Response::from_data(Vec::new()).with_status_code(401),
                    url if url == format!("/repos/acme/site/commits/{sha}") => {
                        tiny_http::Response::from_data(b"{}".to_vec())
                    }
                    url if url == format!("/repos/acme/site/tarball/{sha}") => {
                        tiny_http::Response::from_data(archive.clone())
                    }
                    _ => tiny_http::Response::from_data(Vec::new()).with_status_code(404),
                };
                request.respond(response).expect("fake GitHub answers");
            }
        });
        address
    }

    fn walk(directory: &Path) -> Vec<std::path::PathBuf> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(directory).expect("project folder") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                files.extend(walk(&path));
            } else {
                files.push(path);
            }
        }
        files
    }

    /// Starts an engine on a free port and returns `host:port`.
    fn engine(github: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("engine port");
        let host = listener.local_addr().expect("address").to_string();
        std::thread::spawn(move || {
            serve(
                listener,
                Config {
                    secret: SECRET.to_vec(),
                    github: Github::new(github),
                },
            )
        });
        host
    }

    fn post(host: &str, body: &[u8], signature: Option<String>) -> (u16, serde_json::Value) {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        let mut request = agent.post(format!("http://{host}/v1/run"));
        if let Some(signature) = signature {
            request = request.header("X-Fanta-Internal", signature);
        }
        let mut response = request.send(body).expect("engine answers");
        let status = response.status().as_u16();
        let text = response.body_mut().read_to_string().expect("body");
        (status, serde_json::from_str(&text).expect("json body"))
    }

    /// Sends raw request headers and returns the first bytes of the answer,
    /// or what reading them failed with.
    fn raw(host: &str, headers: &str) -> Result<String, std::io::Error> {
        let mut stream = std::net::TcpStream::connect(host)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.write_all(headers.as_bytes())?;
        let mut answer = [0u8; 12];
        let read = stream.read(&mut answer)?;
        Ok(String::from_utf8_lossy(&answer[..read]).into_owned())
    }

    #[test]
    fn a_signed_github_run_reads_the_commit_and_answers_its_steps() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        let project = tempfile::tempdir().expect("project");
        fanta_format::write_project_tree(project.path(), &fanta_doc::Doc::new(), &Default::default())
            .expect("write project");
        let host = engine(fake_github(project.path(), sha));
        let request = |token: &str, path: &str| {
            serde_json::to_vec(&json!({
                "request_id": "r1",
                "deadline_ms": 60000,
                "source": {"kind": "github", "repo": "acme/site", "sha": sha, "path": path,
                    "token": token,
                    "overlay": {"files": [{"path": "notes/agent.md", "content_b64": "aGk="}]}},
                "steps": [{"step": "inspect"}]
            }))
            .expect("request")
        };

        let body = request("ghs_test", "design");
        let (status, response) = post(&host, &body, Some(sign(&body, unix_now())));
        assert_eq!(status, 200, "{response}");
        assert_eq!(response["source"]["sha"], sha);
        assert_eq!(response["results"][0]["ok"], true, "{response}");

        let (status, response) = post(&host, &body, None);
        assert_eq!((status, &response["error"]["code"]), (401, &json!("unauthorized")));
        let (status, _) = post(&host, &body, Some(sign(&body, unix_now() - 600)));
        assert_eq!(status, 401);

        let denied = request("ghs_other", "design");
        let (status, response) = post(&host, &denied, Some(sign(&denied, unix_now())));
        assert_eq!((status, &response["error"]["code"]), (403, &json!("forbidden")));

        let missing = request("ghs_test", "elsewhere");
        let (status, response) = post(&host, &missing, Some(sign(&missing, unix_now())));
        assert_eq!((status, &response["error"]["code"]), (404, &json!("not_found")));

        let local = serde_json::to_vec(&json!({
            "request_id": "r2", "deadline_ms": 1000,
            "source": {"kind": "dir", "dir": "/"},
            "steps": [{"step": "inspect"}]
        }))
        .expect("request");
        let (status, response) = post(&host, &local, Some(sign(&local, unix_now())));
        assert_eq!((status, &response["error"]["code"]), (400, &json!("bad_request")));
    }

    #[test]
    fn requests_that_cant_be_ours_cost_no_body() {
        let host = engine("http://127.0.0.1:9".into());
        // Unsigned, promising a megabyte that never comes: refused at once.
        let unsigned = format!("POST /v1/run HTTP/1.1\r\nHost: {host}\r\nContent-Length: 1000000\r\n\r\n");
        assert_eq!(raw(&host, &unsigned).expect("an answer"), "HTTP/1.1 401");
        // A well-formed, fresh header with a terabyte declared: refused before
        // anything is read or reserved.
        let huge = format!(
            "POST /v1/run HTTP/1.1\r\nHost: {host}\r\nX-Fanta-Internal: t={},v1=00\r\nContent-Length: 1000000000000\r\n\r\n",
            unix_now()
        );
        assert_eq!(raw(&host, &huge).expect("an answer"), "HTTP/1.1 413");
        let healthz = format!("GET /healthz HTTP/1.1\r\nHost: {host}\r\n\r\n");
        assert_eq!(raw(&host, &healthz).expect("still alive"), "HTTP/1.1 200");
    }

    #[test]
    fn connections_beyond_the_limit_are_closed_unanswered() {
        let host = engine("http://127.0.0.1:9".into());
        // Idle connections that never send headers hold every slot until the
        // header timeout.
        let idle: Vec<_> = (0..MAX_OPEN_CONNECTIONS)
            .map(|_| std::net::TcpStream::connect(&host).expect("idle connection"))
            .collect();
        std::thread::sleep(Duration::from_millis(200));
        let healthz = format!("GET /healthz HTTP/1.1\r\nHost: {host}\r\n\r\n");
        match raw(&host, &healthz) {
            Ok(answer) => assert_eq!(answer, "", "an extra connection must be closed unanswered"),
            Err(error) => assert!(
                matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
                ),
                "{error}"
            ),
        }
        drop(idle);
    }
}
