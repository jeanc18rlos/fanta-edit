//! `fanta-engine serve`: the HTTP API of `docs/cloud-engine-contracts.md`.
//! Every run gets its own temp folder and runs on its own thread, and a panic
//! fails only that request.

use std::{
    io::Read as _,
    panic::AssertUnwindSafe,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use hmac::{Hmac, Mac as _};
use sha2::{Digest as _, Sha256};

use crate::{
    EngineError, EngineInfo, ErrorCode, RunRequest, RunResponse, Source,
    source::{self, Github},
};

/// The largest request body the engine reads.
pub const MAX_BODY_BYTES: u64 = 16 * 1024 * 1024;
/// How far a signature's timestamp may be from the engine's clock.
pub const MAX_CLOCK_SKEW_SECONDS: u64 = 300;
/// Runs one instance takes at once; more answer `429 busy`.
pub const MAX_CONCURRENT_RUNS: usize = 2;

pub struct Config {
    /// `ENGINE_HMAC_SECRET`, shared with the v2 API.
    pub secret: Vec<u8>,
    pub github: Github,
}

/// Answers requests until the server stops.
pub fn serve(server: tiny_http::Server, config: Config) {
    let config = Arc::new(config);
    let in_flight = Arc::new(AtomicUsize::new(0));
    for request in server.incoming_requests() {
        let config = config.clone();
        let in_flight = in_flight.clone();
        std::thread::spawn(move || handle(request, &config, &in_flight));
    }
}

fn handle(mut request: tiny_http::Request, config: &Config, in_flight: &AtomicUsize) {
    let started = Instant::now();
    let route = format!("{} {}", request.method(), request.url());
    let answer = match (request.method(), request.url()) {
        (tiny_http::Method::Get, "/healthz") => serde_json::to_vec(&EngineInfo::current())
            .map(|body| (200, body))
            .map_err(|error| EngineError::new(ErrorCode::Internal, error.to_string())),
        (tiny_http::Method::Post, "/v1/run") => run(&mut request, config, in_flight, started)
            .and_then(|response| {
                serde_json::to_vec(&response)
                    .map(|body| (200, body))
                    .map_err(|error| EngineError::new(ErrorCode::Internal, error.to_string()))
            }),
        _ => Err(EngineError::new(ErrorCode::NotFound, "no such route")),
    };
    let (status, body) = answer.unwrap_or_else(|error| {
        (
            error.error.code.status(),
            serde_json::to_vec(&error).unwrap_or_default(),
        )
    });
    // Cloud Run collects stderr. Requests carry a token, so log only the route.
    eprintln!(
        "{route} {status} {} ms",
        started.elapsed().as_millis()
    );
    let mut response = tiny_http::Response::from_data(body).with_status_code(status);
    if let Ok(content_type) = tiny_http::Header::from_bytes("Content-Type", "application/json") {
        response.add_header(content_type);
    }
    if let Err(error) = request.respond(response) {
        eprintln!("{route}: answering failed: {error}");
    }
}

fn run(
    request: &mut tiny_http::Request,
    config: &Config,
    in_flight: &AtomicUsize,
    started: Instant,
) -> Result<RunResponse, EngineError> {
    let mut body = Vec::new();
    request
        .as_reader()
        .take(MAX_BODY_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|error| EngineError::new(ErrorCode::BadRequest, error.to_string()))?;
    if body.len() as u64 > MAX_BODY_BYTES {
        return Err(EngineError::new(
            ErrorCode::PayloadTooLarge,
            format!("the request body is over {MAX_BODY_BYTES} bytes"),
        ));
    }
    let signature = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("X-Fanta-Internal"))
        .map(|header| header.value.as_str())
        .unwrap_or_default();
    if !verify_signature(&config.secret, signature, &body, unix_now()) {
        return Err(EngineError::new(
            ErrorCode::Unauthorized,
            "missing, malformed, stale or wrong X-Fanta-Internal",
        ));
    }
    let mut run_request: RunRequest = serde_json::from_slice(&body)
        .map_err(|error| EngineError::new(ErrorCode::BadRequest, error.to_string()))?;
    crate::validate(&run_request)?;
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
    let _slot = RunSlot::take(in_flight)?;
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
    let Some(remaining) = deadline.checked_sub(started.elapsed()).filter(|left| !left.is_zero())
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

/// Holds one of the instance's run slots until dropped.
struct RunSlot<'a>(&'a AtomicUsize);

impl<'a> RunSlot<'a> {
    fn take(in_flight: &'a AtomicUsize) -> Result<Self, EngineError> {
        if in_flight.fetch_add(1, Ordering::SeqCst) >= MAX_CONCURRENT_RUNS {
            in_flight.fetch_sub(1, Ordering::SeqCst);
            return Err(EngineError::new(
                ErrorCode::Busy,
                format!("the instance is already running {MAX_CONCURRENT_RUNS} requests"),
            ));
        }
        Ok(Self(in_flight))
    }
}

impl Drop for RunSlot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Checks `t=<unix seconds>,v1=<hex>`, where `v1` is
/// `HMAC-SHA256(secret, "{t}.{hex sha256(body)}")`, in constant time. The
/// v2 API signs with `fanta_gpu_common.auth.sign`.
pub fn verify_signature(secret: &[u8], header: &str, body: &[u8], now: u64) -> bool {
    let (mut timestamp, mut signature) = (None, None);
    for part in header.split(',') {
        match part.split_once('=') {
            Some(("t", value)) => timestamp = value.parse::<u64>().ok(),
            Some(("v1", value)) => signature = hex::decode(value).ok(),
            _ => return false,
        }
    }
    let (Some(timestamp), Some(signature)) = (timestamp, signature) else {
        return false;
    };
    if now.abs_diff(timestamp) > MAX_CLOCK_SKEW_SECONDS {
        return false;
    }
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret) else {
        return false;
    };
    mac.update(format!("{timestamp}.{}", hex::encode(Sha256::digest(body))).as_bytes());
    mac.verify_slice(&signature).is_ok()
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

    fn post(address: &str, body: &[u8], signature: Option<String>) -> (u16, serde_json::Value) {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        let mut request = agent.post(format!("{address}/v1/run"));
        if let Some(signature) = signature {
            request = request.header("X-Fanta-Internal", signature);
        }
        let mut response = request.send(body).expect("engine answers");
        let status = response.status().as_u16();
        let text = response.body_mut().read_to_string().expect("body");
        (status, serde_json::from_str(&text).expect("json body"))
    }

    #[test]
    fn a_signed_github_run_reads_the_commit_and_answers_its_steps() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        let project = tempfile::tempdir().expect("project");
        fanta_format::write_project_tree(project.path(), &fanta_doc::Doc::new(), &Default::default())
            .expect("write project");
        let github = fake_github(project.path(), sha);
        let server = tiny_http::Server::http("127.0.0.1:0").expect("engine");
        let address = format!("http://{}", server.server_addr());
        std::thread::spawn(move || {
            serve(
                server,
                Config {
                    secret: SECRET.to_vec(),
                    github: Github::new(github),
                },
            )
        });
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
        let (status, response) = post(&address, &body, Some(sign(&body, unix_now())));
        assert_eq!(status, 200, "{response}");
        assert_eq!(response["source"]["sha"], sha);
        assert_eq!(response["results"][0]["ok"], true, "{response}");

        let (status, response) = post(&address, &body, None);
        assert_eq!((status, &response["error"]["code"]), (401, &json!("unauthorized")));
        let (status, _) = post(&address, &body, Some(sign(&body, unix_now() - 600)));
        assert_eq!(status, 401);

        let denied = request("ghs_other", "design");
        let (status, response) = post(&address, &denied, Some(sign(&denied, unix_now())));
        assert_eq!((status, &response["error"]["code"]), (403, &json!("forbidden")));

        let missing = request("ghs_test", "elsewhere");
        let (status, response) = post(&address, &missing, Some(sign(&missing, unix_now())));
        assert_eq!((status, &response["error"]["code"]), (404, &json!("not_found")));

        let local = serde_json::to_vec(&json!({
            "request_id": "r2", "deadline_ms": 1000,
            "source": {"kind": "dir", "dir": "/"},
            "steps": [{"step": "inspect"}]
        }))
        .expect("request");
        let (status, response) = post(&address, &local, Some(sign(&local, unix_now())));
        assert_eq!((status, &response["error"]["code"]), (400, &json!("bad_request")));
    }

    #[test]
    fn runs_beyond_the_instance_limit_are_busy() {
        let in_flight = AtomicUsize::new(0);
        let first = RunSlot::take(&in_flight).expect("first slot");
        let _second = RunSlot::take(&in_flight).expect("second slot");
        assert_eq!(
            RunSlot::take(&in_flight).err().map(|error| error.error.code),
            Some(ErrorCode::Busy)
        );
        drop(first);
        RunSlot::take(&in_flight).expect("a freed slot");
    }
}
