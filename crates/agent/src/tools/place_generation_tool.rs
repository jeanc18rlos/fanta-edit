use std::sync::Arc;

use agent_client_protocol::schema::v1 as acp;
use anyhow::{Context as _, Result, bail};
use base64::Engine as _;
use design_surface::DesignOp;
use futures::{AsyncReadExt as _, FutureExt as _};
use gpui::{App, AppContext as _, Task};
use http_client::{AsyncBody, HttpClientWithUrl};
use language_model::{LanguageModelImage, LanguageModelToolResultContent};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use ui::SharedString;

use crate::sandboxing::{NetworkRequest, SandboxRequest};
use crate::{AgentTool, ToolCallEventStream, ToolInput};

/// Import a generated image into the active Fanta project's asset library
/// without creating a canvas layer. Supply exactly one of url, path, or source
/// (base64 or a base64 data URI). The saved asset_id and path can be referenced
/// from FNX file edits. Relative paths resolve from the active Fanta project.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ImportProjectImageToolInput {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

pub struct ImportProjectImageTool {
    http_client: Arc<HttpClientWithUrl>,
}

impl ImportProjectImageTool {
    pub fn new(http_client: Arc<HttpClientWithUrl>) -> Self {
        Self { http_client }
    }
}

impl AgentTool for ImportProjectImageTool {
    type Input = ImportProjectImageToolInput;
    type Output = LanguageModelToolResultContent;
    const NAME: &'static str = "import_project_image";
    fn kind() -> acp::ToolKind {
        acp::ToolKind::Edit
    }
    fn allow_in_restricted_mode() -> bool {
        false
    }
    fn initial_title(
        &self,
        input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        input
            .ok()
            .and_then(|input| input.name)
            .map(|name| format!("Import project image: {name}"))
            .unwrap_or_else(|| "Import project image".into())
            .into()
    }
    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(tool_content_err)?;
            if [input.url.is_some(), input.path.is_some(), input.source.is_some()].into_iter().filter(|provided| *provided).count() != 1 {
                return Err(tool_content_err("supply exactly one of url, path or source"));
            }
            let (surface, state) = cx.update(|cx| {
                let surface = design_surface::active(cx).context("open a Fanta project before importing images")?;
                let state = surface.state(cx)?;
                Ok::<_, anyhow::Error>((surface, state))
            }).map_err(tool_content_err)?;
            let document_id = state.get("document_id").cloned();
            let unsandboxed = cx.update(|cx| event_stream.unsandboxed_access_granted(cx));
            let bytes = if let Some(url) = input.url {
                if !unsandboxed {
                    let host = host_pattern_for_url(&url).map_err(tool_content_err)?;
                    let authorization = cx.update(|cx| event_stream.authorize_sandbox(SandboxRequest {
                        network: NetworkRequest::Hosts(vec![host]), ..Default::default()
                    }, String::new(), cx));
                    authorization.await.map_err(tool_content_err)?;
                }
                let download = cx.background_spawn({
                    let http_client = self.http_client.clone();
                    async move { download_image(http_client, &url).await }
                });
                futures::select! {
                    result = download.fuse() => result.map_err(tool_content_err)?,
                    _ = event_stream.cancelled_by_user().fuse() => return Err(tool_content_err("Image import cancelled by user")),
                }
            } else if let Some(path) = input.path {
                let path = std::path::PathBuf::from(path);
                let root = state.get("project_root").and_then(serde_json::Value::as_str).map(std::path::PathBuf::from);
                let path = if path.is_absolute() { path } else {
                    root.as_ref().context("relative image paths require a saved Fanta project; use an absolute path")
                        .map_err(tool_content_err)?.join(path)
                };
                let canonical = cx.background_spawn(async move { path.canonicalize().context("the image file cannot be opened") }).await.map_err(tool_content_err)?;
                let root = cx.background_spawn(async move {
                    root.map(|root| root.canonicalize()).transpose().context("the project directory cannot be resolved")
                }).await.map_err(tool_content_err)?;
                if !unsandboxed && !root.as_ref().is_some_and(|root| canonical.starts_with(root)) {
                    let authorization = cx.update(|cx| event_stream.authorize_always_prompt(
                        format!("Import image outside the project: {}", canonical.display()),
                        crate::ToolPermissionContext::symlink_target(Self::NAME, vec![canonical.display().to_string()]), cx,
                    ));
                    authorization.await.map_err(tool_content_err)?;
                }
                let read = cx.background_spawn(async move {
                    use std::io::Read as _;
                    let file = std::fs::File::open(canonical).context("opening the generated image")?;
                    anyhow::ensure!(file.metadata()?.is_file(), "the image path is not a regular file");
                    let mut bytes = Vec::new();
                    file.take(MAX_DOWNLOAD_BYTES as u64 + 1).read_to_end(&mut bytes)?;
                    anyhow::ensure!(bytes.len() <= MAX_DOWNLOAD_BYTES, "the image exceeds the project asset limit");
                    Ok::<_, anyhow::Error>(bytes)
                });
                futures::select! {
                    result = read.fuse() => result.map_err(tool_content_err)?,
                    _ = event_stream.cancelled_by_user().fuse() => return Err(tool_content_err("Image import cancelled by user")),
                }
            } else {
                let source = input.source.context("the image source is missing").map_err(tool_content_err)?;
                cx.background_spawn(async move { decode_import_source(&source) }).await.map_err(tool_content_err)?
            };
            let name = input.name.unwrap_or_else(|| "Generated image".into());
            let import = cx.update(|cx| {
                let current_state = surface.state(cx)?;
                anyhow::ensure!(current_state.get("document_id") == document_id.as_ref(), "the active design changed during the import; select the intended project and retry");
                Ok::<_, anyhow::Error>(surface.import_image(bytes, name, cx))
            }).map_err(tool_content_err)?;
            let result = import.await.map_err(tool_content_err)?;
            let text = serde_json::to_string(&result).map_err(tool_content_err)?;
            event_stream.update_fields(acp::ToolCallUpdateFields::new().content(vec![
                acp::ToolCallContent::Content(acp::Content::new(text.clone())),
            ]));
            Ok(text.into())
        })
    }
}

fn decode_import_source(source: &str) -> Result<Vec<u8>> {
    anyhow::ensure!(
        source.len() <= 90 * 1024 * 1024,
        "the image source exceeds the project asset limit"
    );
    let payload = if let Some(data) = source.trim().strip_prefix("data:") {
        let (header, payload) = data
            .split_once(',')
            .context("the image data URI has no payload")?;
        anyhow::ensure!(
            header.ends_with(";base64"),
            "the image data URI must use base64"
        );
        payload
    } else {
        source
    };
    let compact = payload
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(compact)
        .context("the image source is not valid base64")?;
    anyhow::ensure!(
        bytes.len() <= MAX_DOWNLOAD_BYTES,
        "the image exceeds the project asset limit"
    );
    Ok(bytes)
}

fn tool_content_err(e: impl std::fmt::Display) -> LanguageModelToolResultContent {
    LanguageModelToolResultContent::from(e.to_string())
}

/// Download a generated image by URL and place it on the open design canvas
/// as a bitmap layer, recording its AI provenance (prompt, model,
/// generation id) in the node's metadata.
///
/// Use this after a generation from the `fanta` media tools finishes and
/// `get_generation` reports an asset URL. The image is ingested as a project
/// asset (persisted under `assets/images/` when the project is saved), so the
/// design stays self-contained. `x`/`y` are world (canvas) coordinates of the
/// placed image's top-left corner (y grows down); omit `width`/`height` to
/// keep the image's natural pixel size (give one and the other follows the
/// aspect ratio). Ask `design_state` for `empty_space` first when placing on
/// the page rather than inside a frame, then verify placement with
/// `design_screenshot`. This is the equivalent of a `create_image` op in
/// `design_edit` with the download done for you.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PlaceGenerationToolInput {
    /// URL of the finished generation's image file.
    pub url: String,
    /// World x of the placed image's top-left corner.
    pub x: f64,
    /// World y of the placed image's top-left corner.
    pub y: f64,
    /// Placed width in canvas units (defaults to the natural width).
    #[serde(default)]
    pub width: Option<f64>,
    /// Placed height in canvas units (defaults to the natural height).
    #[serde(default)]
    pub height: Option<f64>,
    /// Id of the parent frame/group. Omit to place on the active page.
    #[serde(default)]
    pub parent: Option<String>,
    /// Layer name (e.g. a short slug of the prompt).
    #[serde(default)]
    pub name: Option<String>,
    /// The prompt that produced the image (recorded as provenance).
    #[serde(default)]
    pub prompt: Option<String>,
    /// The model that produced the image (recorded as provenance).
    #[serde(default)]
    pub model: Option<String>,
    /// The backend generation id (recorded as provenance).
    #[serde(default)]
    pub generation_id: Option<String>,
}

pub struct PlaceGenerationTool {
    http_client: Arc<HttpClientWithUrl>,
}

impl PlaceGenerationTool {
    pub fn new(http_client: Arc<HttpClientWithUrl>) -> Self {
        Self { http_client }
    }
}

/// Cap on the downloaded image size; matches the design surface's own ingest
/// limit so the fetch fails fast instead of the placement op.
const MAX_DOWNLOAD_BYTES: usize = 32 * 1024 * 1024;

impl AgentTool for PlaceGenerationTool {
    type Input = PlaceGenerationToolInput;
    type Output = LanguageModelToolResultContent;

    const NAME: &'static str = "place_generation";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Edit
    }

    fn allow_in_restricted_mode() -> bool {
        // Downloads from the network, like `fetch`.
        false
    }

    fn initial_title(
        &self,
        input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        match input {
            Ok(input) => match input.name {
                Some(name) => format!("Place generated image: {name}").into(),
                None => "Place generated image".into(),
            },
            Err(_) => "Place generated image".into(),
        }
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        let http_client = self.http_client.clone();
        cx.spawn(async move |cx| {
            let input = input.recv().await.map_err(tool_content_err)?;

            // The download reuses the per-host network grants shared with the
            // `terminal` and `fetch` tools (unless unsandboxed access already
            // covers everything), so a URL outside the granted hosts prompts
            // the user instead of silently fetching.
            let unsandboxed = cx.update(|cx| event_stream.unsandboxed_access_granted(cx));
            if !unsandboxed {
                let host = host_pattern_for_url(&input.url).map_err(tool_content_err)?;
                let authorize = cx.update(|cx| {
                    let request = SandboxRequest {
                        network: NetworkRequest::Hosts(vec![host]),
                        ..Default::default()
                    };
                    event_stream.authorize_sandbox(request, String::new(), cx)
                });
                futures::select! {
                    result = authorize.fuse() => result.map_err(tool_content_err)?,
                    _ = event_stream.cancelled_by_user().fuse() => {
                        return Err(tool_content_err("Placement cancelled by user"));
                    }
                };
            }

            let download = cx.background_spawn({
                let url = input.url.clone();
                async move { download_image(http_client, &url).await }
            });
            let bytes = futures::select! {
                result = download.fuse() => result.map_err(tool_content_err)?,
                _ = event_stream.cancelled_by_user().fuse() => {
                    return Err(tool_content_err("Placement cancelled by user"));
                }
            };
            let mime = sniff_image_mime(&bytes);
            let base64_bytes = base64::engine::general_purpose::STANDARD.encode(&bytes);

            let mut provenance = serde_json::Map::new();
            if let Some(prompt) = &input.prompt {
                provenance.insert("prompt".into(), json!(prompt));
            }
            if let Some(model) = &input.model {
                provenance.insert("model".into(), json!(model));
            }
            if let Some(generation_id) = &input.generation_id {
                provenance.insert("generation_id".into(), json!(generation_id));
            }
            provenance.insert("url".into(), json!(input.url));

            let op = DesignOp::CreateImage {
                source: base64_bytes,
                parent: input.parent.clone(),
                name: input.name.clone(),
                x: input.x,
                y: input.y,
                width: input.width,
                height: input.height,
                meta: Some(json!({ "generation": provenance })),
            };
            let label = input
                .name
                .clone()
                .map(|name| format!("Place {name}"))
                .unwrap_or_else(|| "Place generated image".to_string());
            let value = cx
                .update(|cx| {
                    let surface = design_surface::active(cx).context(
                        "no design canvas is available; ask the user to open a .fig file or Fanta project",
                    )?;
                    surface.apply(vec![op], label, cx)
                })
                .map_err(tool_content_err)?;

            let applied = value
                .get("applied")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let text: LanguageModelToolResultContent = serde_json::to_string(&value)
                .map_err(tool_content_err)?
                .into();
            if !applied {
                return Err(text);
            }
            // Show the placed image inline in the thread when the format is
            // one the client can render.
            if let Some(mime) = mime {
                let image = LanguageModelImage {
                    source: base64::engine::general_purpose::STANDARD.encode(&bytes).into(),
                };
                event_stream.update_fields(acp::ToolCallUpdateFields::new().content(vec![
                    acp::ToolCallContent::Content(acp::Content::new(acp::ContentBlock::Image(
                        acp::ImageContent::new(image.source, mime),
                    ))),
                ]));
            }
            Ok(text)
        })
    }
}

async fn download_image(http_client: Arc<HttpClientWithUrl>, url: &str) -> Result<Vec<u8>> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        bail!("the generation URL must be http(s)");
    }
    let mut response = http_client
        .get(url, AsyncBody::default(), true)
        .await
        .with_context(|| format!("requesting {url}"))?;
    if !response.status().is_success() {
        bail!(
            "downloading the image failed with status {}",
            response.status().as_u16()
        );
    }
    let mut body = Vec::new();
    response
        .body_mut()
        .take(MAX_DOWNLOAD_BYTES as u64 + 1)
        .read_to_end(&mut body)
        .await
        .context("reading the image bytes")?;
    if body.len() > MAX_DOWNLOAD_BYTES {
        bail!(
            "the image is {} bytes; the limit is {MAX_DOWNLOAD_BYTES}",
            body.len()
        );
    }
    if sniff_image_mime(&body).is_none() {
        bail!("the URL did not return a PNG/JPEG/WebP/GIF image");
    }
    Ok(body)
}

/// Image mime type from magic bytes, for the small set of formats the canvas
/// (and thread UI) can decode.
fn sniff_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(b"GIF8") {
        Some("image/gif")
    } else {
        None
    }
}

/// Same URL → host-pattern mapping as the `fetch` tool, so grants line up.
fn host_pattern_for_url(url: &str) -> Result<http_proxy::HostPattern> {
    let parsed = url::Url::parse(url).with_context(|| format!("could not parse URL {url:?}"))?;
    let host = parsed
        .host_str()
        .with_context(|| format!("URL {url:?} has no host to authorize network access for"))?;
    http_proxy::HostPattern::parse(host).map_err(|error| match error {
        http_proxy::HostPatternError::IpLiteral(_) => anyhow::anyhow!(
            "cannot download from {host:?}: loopback and IP-literal hosts are only reachable \
             once unsandboxed access has been granted"
        ),
        error => anyhow::anyhow!("cannot authorize network access to {host:?}: {error}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_source_decodes_data_uris_and_reports_invalid_payloads() -> Result<()> {
        let bytes = b"\x89PNG\r\n\x1a\nfixture";
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        assert_eq!(decode_import_source(&encoded)?, bytes);
        assert_eq!(
            decode_import_source(&format!("data:image/png;base64,{encoded}\n"))?,
            bytes
        );
        assert!(decode_import_source("data:image/png,abc").is_err());
        assert!(decode_import_source("data:image/png;base64").is_err());
        assert!(decode_import_source("invalid%").is_err());
        Ok(())
    }

    #[test]
    fn sniffs_the_supported_image_formats() {
        assert_eq!(
            sniff_image_mime(b"\x89PNG\r\n\x1a\n...."),
            Some("image/png")
        );
        assert_eq!(
            sniff_image_mime(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some("image/jpeg")
        );
        assert_eq!(
            sniff_image_mime(b"RIFF\x10\x00\x00\x00WEBPVP8 "),
            Some("image/webp")
        );
        assert_eq!(sniff_image_mime(b"GIF89a.."), Some("image/gif"));
        assert_eq!(sniff_image_mime(b"<svg xmlns='x'/>"), None);
    }
}
