//! Fanta's headless MCP server (spec 0008 R7/T7): the fourteen agent tools
//! the app's live server speaks, served over newline-delimited JSON-RPC on
//! stdio from a project directory alone — so Claude Code and Codex can read,
//! edit, render and annotate a design without the Fanta app running.
//!
//! The tool vocabulary is shared by construction: both servers deserialize
//! the argument structs in `fanta-design-ops`, and each tool's description is
//! that struct's doc comment. Edits go through [`HeadlessProject`], the same
//! engine the editor and the cloud workers save with, and every mutating tool
//! saves the project before it answers, so the change is on disk (and in git)
//! when the tool result arrives.
//!
//! Usage: `fanta-mcp <project-dir>`, or `fanta-mcp --frames <project-dir>
//! --out <file.png>` to watch the project and keep a page rendered as a PNG
//! for terminal panes (spec 0008 T4; see `frames.rs`).

mod frames;

use std::io::{BufRead as _, Read as _, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use base64::Engine as _;
use fanta_design_ops::{
    AgentActivity, BatchDesignArgs, BatchGetArgs, CommentDraft, DEFAULT_CHILD_LIMIT,
    DESIGN_GUIDELINES, DesignAssetRequest, DesignSystemQuery, GetEditorStateArgs,
    GetGuidelinesArgs, GetScreenshotArgs, ImportImageArgs, ListCommentsArgs,
    MAX_JSON_RESPONSE_BYTES, NodeDetail, ReadFnxSourceArgs, ReplyCommentArgs,
    ValidateFnxSourceArgs, narrowing_hint, read_project_design_spec, slice_source,
};
use fanta_headless::apply::parse_node_id;
use fanta_headless::assets::{MAX_IMAGE_SOURCE_BYTES, decode_image_source};
use fanta_headless::{HeadlessProject, PageRef, RenderTarget, comments, pages, scene, sources, style};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The protocol revisions this server can answer in; an unknown request is
/// answered with the newest one, as the app's live server does.
const PROTOCOL_VERSIONS: [&str; 4] = ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];
const LATEST_PROTOCOL_VERSION: &str = "2025-11-25";

/// Place a new canvas comment as an agent: a review finding, a question or a
/// note. Anchor it to a layer with `node` (prefixed id; `offset` in the node's
/// local units, its centre by default) so the pin follows the layer, or give a
/// `world` position. Review findings carry `severity` (info, minor, major,
/// blocker), `category` (layout, typography, color, accessibility,
/// consistency, content, interaction, motion, other), the run's
/// `review_run_id`, and optionally a `suggested_fix` (`summary` plus the
/// content-editing batch_design `ops` that would address it, excluding select
/// and set_viewport) the user can apply.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
struct AddCommentArgs {
    #[serde(flatten)]
    draft: CommentDraft,
}

/// Report each visible editing milestone with its page, node, coordinates,
/// source_path and workspace (canvas, variables or code). Choose a stable
/// mage name and agent_id so the user can follow you across files and tabs.
/// Reuse that identity in batch_design.activity for streamed canvas changes.
/// Set active false when finished.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
struct ReportActivityArgs {
    #[serde(flatten)]
    activity: AgentActivity,
}

const USAGE: &str = "usage: fanta-mcp <project-dir>\n       fanta-mcp --frames <project-dir> --out <file.png> \
     [--page <index>] [--max-dimension <pixels>] [--interval-ms <ms>] [--once]";

fn parse_frame_options(arguments: &[String]) -> Result<frames::FrameOptions> {
    let mut root = None;
    let mut out = None;
    let mut page = None;
    let mut max_dimension = 1024;
    let mut interval_ms = 500;
    let mut once = false;
    let mut words = arguments.iter();
    while let Some(word) = words.next() {
        let mut value = |name: &str| words.next().with_context(|| format!("{name} needs a value"));
        match word.as_str() {
            "--frames" => {}
            "--once" => once = true,
            "--out" => out = Some(PathBuf::from(value("--out")?)),
            "--page" => page = Some(value("--page")?.parse().context("--page takes an index")?),
            "--max-dimension" => {
                max_dimension = value("--max-dimension")?
                    .parse()
                    .context("--max-dimension takes pixels")?;
            }
            "--interval-ms" => {
                interval_ms = value("--interval-ms")?
                    .parse()
                    .context("--interval-ms takes milliseconds")?;
            }
            positional if !positional.starts_with("--") && root.is_none() => {
                root = Some(PathBuf::from(positional));
            }
            unknown => bail!("unknown argument {unknown}"),
        }
    }
    Ok(frames::FrameOptions {
        root: root.context("a project directory is required")?,
        out: out.context("--out is required")?,
        page,
        max_dimension,
        interval: std::time::Duration::from_millis(interval_ms),
        once,
    })
}

fn main() {
    let all: Vec<String> = std::env::args().skip(1).collect();
    if all.iter().any(|argument| argument == "--frames") {
        let options = match parse_frame_options(&all) {
            Ok(options) => options,
            Err(error) => {
                eprintln!("fanta-mcp: {error:#}\n{USAGE}");
                std::process::exit(2);
            }
        };
        if let Err(error) = frames::run(&options) {
            eprintln!("fanta-mcp --frames: {error:#}");
            std::process::exit(1);
        }
        return;
    }
    let root = match (all.first(), all.get(1)) {
        (Some(root), None) if root != "--help" => PathBuf::from(root),
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };
    let mut server = match Server::open(&root) {
        Ok(server) => server,
        Err(error) => {
            eprintln!("fanta-mcp: opening {}: {error:#}", root.display());
            std::process::exit(1);
        }
    };
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                eprintln!("fanta-mcp: reading stdin: {error:#}");
                break;
            }
        };
        if let Some(response) = server.handle_line(&line) {
            let written = serde_json::to_string(&response)
                .context("serializing a response")
                .and_then(|text| {
                    stdout.write_all(text.as_bytes())?;
                    stdout.write_all(b"\n")?;
                    stdout.flush()?;
                    Ok(())
                });
            if let Err(error) = written {
                eprintln!("fanta-mcp: writing stdout: {error:#}");
                break;
            }
        }
    }
}

struct Server {
    project: HeadlessProject,
    root: PathBuf,
}

impl Server {
    fn open(root: &Path) -> Result<Self> {
        Ok(Self {
            project: HeadlessProject::open(root)?,
            root: root.to_path_buf(),
        })
    }

    /// One line in, at most one line out: a JSON-RPC 2.0 object per line, a
    /// `-32700` framed error with `id: null` for an unparseable line, and no
    /// answer at all for notifications (requests without an `id`).
    fn handle_line(&mut self, line: &str) -> Option<Value> {
        if line.trim().is_empty() {
            return None;
        }
        let request: Value = match serde_json::from_str(line) {
            Ok(request) => request,
            Err(error) => {
                return Some(error_response(
                    Value::Null,
                    -32700,
                    &format!("parsing the request: {error}"),
                ));
            }
        };
        let id = request.get("id").filter(|id| !id.is_null())?.clone();
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let params = request.get("params").cloned().unwrap_or(Value::Null);
        let response = match method {
            "initialize" => Ok(initialize_response(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tool_entries() })),
            "tools/call" => self.call_tool(&params),
            _ => {
                return Some(error_response(
                    id,
                    -32601,
                    &format!("method {method} is not supported"),
                ));
            }
        };
        Some(match response {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(error) => error_response(id, -32603, &format!("{error:#}")),
        })
    }

    fn call_tool(&mut self, params: &Value) -> Result<Value> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .context("tools/call requires a tool name")?;
        let arguments = match params.get("arguments") {
            None | Some(Value::Null) => json!({}),
            Some(arguments) => arguments.clone(),
        };
        let content = match name {
            "get_editor_state" => self.get_editor_state(parse(arguments)?)?,
            "get_design_system" => self.get_design_system(parse(arguments)?)?,
            "prepare_design_asset" => {
                let _request: DesignAssetRequest = parse(arguments)?;
                bail!(
                    "the asset composer is part of the Fanta app; connect to a running Fanta \
                     over its live MCP socket, or import a finished image with import_image"
                );
            }
            "batch_get" => self.batch_get(parse(arguments)?)?,
            "batch_design" => self.batch_design(parse(arguments)?)?,
            "get_screenshot" => self.get_screenshot(parse(arguments)?)?,
            "read_fnx_source" => self.read_fnx_source(parse(arguments)?)?,
            "validate_fnx_source" => self.validate_fnx_source(parse(arguments)?)?,
            "get_guidelines" => {
                let _arguments: GetGuidelinesArgs = parse(arguments)?;
                self.get_guidelines()?
            }
            "import_image" => self.import_image(parse(arguments)?)?,
            "list_comments" => self.list_comments(parse(arguments)?)?,
            "reply_comment" => self.reply_comment(parse(arguments)?)?,
            "add_comment" => self.add_comment(parse(arguments)?)?,
            "report_agent_activity" => {
                let _arguments: ReportActivityArgs = parse(arguments)?;
                // There is no app UI to stream milestones to; acknowledging
                // keeps agent loops written for the live server working.
                vec![text_content(&json!({ "reported": true }))?]
            }
            _ => bail!("tool {name} does not exist on this server"),
        };
        Ok(json!({ "content": content }))
    }

    fn page_root(&self, page: Option<usize>) -> Result<(usize, fanta_doc::NodeId)> {
        let index = match page {
            Some(index) => index,
            None => self.project.active_page_index()?,
        };
        let root = self.project.resolve_page(&PageRef::Index { index })?;
        Ok((index, root))
    }

    fn get_editor_state(&self, arguments: GetEditorStateArgs) -> Result<Vec<Value>> {
        let mut state = self.project.inspect()?;
        if let Some(object) = state.as_object_mut() {
            object.insert(
                "project_root".into(),
                json!(self.root.display().to_string()),
            );
            if let Some([width, height]) = arguments.empty_space {
                if !(width.is_finite() && width > 0.0 && height.is_finite() && height > 0.0) {
                    bail!("width and height must be positive");
                }
                let (index, root) = self.page_root(arguments.page)?;
                let spot =
                    scene::empty_space(self.project.document().doc(), root, width, height);
                object.insert(
                    "empty_space".into(),
                    json!({
                        "page": index,
                        "x": spot.x,
                        "y": spot.y,
                        "width": width,
                        "height": height,
                    }),
                );
            }
        }
        Ok(vec![text_content(&state)?])
    }

    fn get_guidelines(&self) -> Result<Vec<Value>> {
        let mut text = DESIGN_GUIDELINES.to_string();
        if let Some(spec) = read_project_design_spec(&self.root)? {
            text.push_str(&format!("\n\n{}", spec.prompt_context()));
        }
        Ok(vec![json!({ "type": "text", "text": text })])
    }

    fn get_design_system(&self, query: DesignSystemQuery) -> Result<Vec<Value>> {
        let value = scene::design_system_json(
            self.project.document().doc(),
            Some(&self.root),
            query,
            |design| self.project.indexed_design_source(design),
        )?;
        bounded_text_content(
            &value,
            "Lower limit; use offset for subsequent component, variant-set and variable pages. \
             Choose a collection to filter variables.",
        )
    }

    fn batch_get(&self, arguments: BatchGetArgs) -> Result<Vec<Value>> {
        let doc = self.project.document().doc();
        if let Some(ids) = &arguments.ids {
            let detail = arguments.detail.unwrap_or(NodeDetail::Style);
            let mut nodes = Vec::with_capacity(ids.len());
            for raw in ids {
                let id = parse_node_id(raw)?;
                let node = doc
                    .scene
                    .get(id)
                    .with_context(|| format!("node {raw} does not exist"))?;
                let mut value = match detail {
                    NodeDetail::Raw => serde_json::to_value(node)?,
                    NodeDetail::Style => style::node_style(doc, id),
                    NodeDetail::Summary => {
                        scene::node_summary(doc, id, Some(0), arguments.include_geometry)
                    }
                };
                if let Some(object) = value.as_object_mut() {
                    object.insert(
                        "children".into(),
                        json!(
                            doc.scene
                                .children_of(Some(id))
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                        ),
                    );
                    if arguments.include_geometry {
                        object.insert("world_bounds".into(), scene::world_bounds_json(doc, id));
                    }
                }
                nodes.push(value);
            }
            return bounded_text_content(&json!({ "nodes": nodes }), narrowing_hint(false));
        }

        let (page_index, root) = self.page_root(arguments.page)?;
        // An unbounded page listing walks the whole scene; on a real imported
        // `.fig` that is tens of thousands of nodes.
        let depth = arguments.depth.or(Some(2));
        scene::check_page_listing_budget(
            doc,
            root,
            depth,
            arguments.offset.unwrap_or(0),
            arguments.limit.unwrap_or(DEFAULT_CHILD_LIMIT),
        )?;
        let mut listing = scene::paginated_node_summary(
            doc,
            root,
            depth,
            arguments.include_geometry,
            arguments.offset.unwrap_or(0),
            arguments.limit.unwrap_or(DEFAULT_CHILD_LIMIT),
        );
        match arguments.detail {
            Some(NodeDetail::Style) => style::attach_styles(doc, &mut listing),
            Some(NodeDetail::Raw) => {
                bail!("`detail: \"raw\"` returns whole node records; fetch them by `ids`")
            }
            Some(NodeDetail::Summary) | None => {}
        }
        let (name, _hidden) = pages::page_label(doc, Some(root), page_index);
        bounded_text_content(
            &json!({
                "page": page_index,
                "name": scene::truncate_summary_string(&name, scene::SUMMARY_LABEL_CHARS),
                "root": listing,
            }),
            narrowing_hint(true),
        )
    }

    fn batch_design(&mut self, arguments: BatchDesignArgs) -> Result<Vec<Value>> {
        let label = arguments.label.unwrap_or_else(|| "MCP edit".to_string());
        let result = self.project.apply(None, &label, &arguments.ops, true)?;
        self.project.settle_layout()?;
        self.project.save()?;
        Ok(vec![text_content(&result)?])
    }

    fn get_screenshot(&mut self, arguments: GetScreenshotArgs) -> Result<Vec<Value>> {
        let (index, _root) = self.page_root(arguments.page)?;
        let target = RenderTarget {
            id: "screenshot".to_string(),
            page: PageRef::Index { index },
            node: arguments.node,
            max_dimension: arguments.max_dimension.unwrap_or(1024),
            motion_clip: arguments.motion_clip,
            playhead_ms: arguments.playhead_ms,
        };
        let image = self.project.render(&target)?;
        Ok(vec![json!({
            "type": "image",
            "data": base64::engine::general_purpose::STANDARD.encode(&image.png),
            "mimeType": "image/png",
        })])
    }

    /// `relative` resolved inside the project, refusing escapes — the same
    /// guard the live server's source reads apply.
    fn source_path(&self, relative: &str) -> Result<PathBuf> {
        let canonical_root = self
            .root
            .canonicalize()
            .with_context(|| format!("resolving {}", self.root.display()))?;
        let canonical = self
            .root
            .join(relative)
            .canonicalize()
            .with_context(|| format!("{relative} does not exist in the project"))?;
        if !canonical.starts_with(&canonical_root) {
            bail!("{relative} escapes the project root");
        }
        Ok(canonical)
    }

    fn read_fnx_source(&self, arguments: ReadFnxSourceArgs) -> Result<Vec<Value>> {
        let Some(path) = arguments.path else {
            return bounded_text_content(
                &json!({
                    "root": self.root.display().to_string(),
                    "files": sources::list_source_files(&self.root),
                }),
                "This project has more source files than fit in one result; read them by name \
                 from `pages/` and `components/` instead.",
            );
        };
        let text = std::fs::read_to_string(self.source_path(&path)?)
            .with_context(|| format!("reading {path}"))?;
        let slice = slice_source(&text, arguments.offset, arguments.limit, arguments.max_bytes);
        Ok(vec![text_content(&slice.into_response(&path))?])
    }

    fn validate_fnx_source(&self, arguments: ValidateFnxSourceArgs) -> Result<Vec<Value>> {
        let validation = if Path::new(&arguments.path)
            .extension()
            .is_none_or(|extension| extension != "fnx" && extension != "json")
        {
            json!({ "applicable": false })
        } else {
            let source = match arguments.source {
                Some(source) => source,
                None => std::fs::read_to_string(self.source_path(&arguments.path)?)
                    .context("the requested FNX source text is unavailable")?,
            };
            sources::validate_source_candidate(
                Some(&self.root),
                Path::new(&arguments.path),
                &source,
            )?
        };
        bounded_text_content(&validation, "Additional FNX diagnostics omitted.")
    }

    fn import_image(&mut self, arguments: ImportImageArgs) -> Result<Vec<Value>> {
        if [
            arguments.path.is_some(),
            arguments.url.is_some(),
            arguments.source.is_some(),
        ]
        .into_iter()
        .filter(|provided| *provided)
        .count()
            != 1
        {
            bail!("supply exactly one of path, url or source");
        }
        let bytes = if let Some(path) = arguments.path {
            let path = PathBuf::from(path);
            let path = if path.is_absolute() {
                path
            } else {
                self.root.join(path)
            };
            let file = std::fs::File::open(&path)
                .with_context(|| format!("opening image {}", path.display()))?;
            if !file.metadata()?.is_file() {
                bail!("the image path must be a regular file");
            }
            let mut bytes = Vec::new();
            file.take(MAX_IMAGE_SOURCE_BYTES as u64 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > MAX_IMAGE_SOURCE_BYTES {
                bail!("the image exceeds the project asset limit");
            }
            bytes
        } else if arguments.url.is_some() {
            bail!(
                "downloading images over HTTP needs the Fanta app; pass the file as `path` or \
                 its bytes as base64 `source` instead"
            );
        } else {
            let source = arguments.source.context("the image source is missing")?;
            decode_image_source(&source)?
        };
        let name = arguments.name.unwrap_or_else(|| "Generated image".into());
        let name: String = name.trim().chars().take(80).collect();
        let (asset, natural_size, _ingested) =
            self.project.document_mut().ingest_image(bytes)?;
        let document = self.project.document_mut();
        document
            .doc_mut()
            .asset_library
            .entry(asset)
            .or_insert_with(|| fanta_doc::ProjectAsset {
                name: if name.is_empty() {
                    "Generated image".into()
                } else {
                    name
                },
                kind: fanta_doc::ProjectAssetKind::Image,
            });
        let format = fanta_format::MediaRegistry::with_builtins()
            .sniff(
                document
                    .raw_assets()
                    .get(&asset)
                    .context("the imported image bytes are unavailable")?,
            )
            .context("the imported image has an unsupported asset format")?;
        let relative = format!("assets/{}/{}.{}", format.family, asset, format.extension);
        self.project.save()?;
        Ok(vec![text_content(&json!({
            "asset_id": asset.to_string(),
            "path": relative,
            "width": natural_size[0],
            "height": natural_size[1],
            "absolute_path": self.root.join(&relative).display().to_string(),
            "persisted": true,
        }))?])
    }

    fn list_comments(&self, arguments: ListCommentsArgs) -> Result<Vec<Value>> {
        let (page_index, root) = self.page_root(arguments.page)?;
        let doc = self.project.document().doc();
        let comments = comments::read_comments(doc, root)
            .into_iter()
            .filter(|comment| arguments.include_resolved || !comment.resolved)
            .map(|comment| {
                let resolved_world = comments::resolve_world(doc, &comment);
                let mut value = serde_json::to_value(&comment)?;
                if let Some(object) = value.as_object_mut() {
                    object.insert("resolved_world".into(), json!(resolved_world));
                    if let Some(anchor) = comment.anchor {
                        object.insert(
                            "anchor".into(),
                            json!({ "node": anchor.node.to_string(), "offset": anchor.offset }),
                        );
                    }
                }
                Ok(value)
            })
            .collect::<Result<Vec<_>>>()?;
        bounded_text_content(
            &json!({ "page": page_index, "comments": comments }),
            "Read one page's comments at a time.",
        )
    }

    fn reply_comment(&mut self, arguments: ReplyCommentArgs) -> Result<Vec<Value>> {
        let (page_index, root) = self.page_root(arguments.page)?;
        let author = arguments
            .author
            .unwrap_or_else(|| "Fanta Agent".to_string());
        let operation = comments::agent_reply_comment_op(
            self.project.document().doc(),
            root,
            &arguments.id,
            &arguments.body,
            &author,
            arguments.resolve,
        )?;
        self.project
            .document_mut()
            .transaction("Reply to comment", vec![operation])?;
        self.project.save()?;
        Ok(vec![text_content(&json!({
            "page": page_index,
            "comment_id": arguments.id,
            "replied": true,
            "resolved": arguments.resolve,
        }))?])
    }

    fn add_comment(&mut self, arguments: AddCommentArgs) -> Result<Vec<Value>> {
        let (page_index, root) = self.page_root(arguments.draft.page)?;
        let mut results = self.project.add_comments("Add comment", &[arguments.draft])?;
        let id = results
            .pop()
            .context("the comment draft produced no result")??;
        let world = comments::read_comments(self.project.document().doc(), root)
            .into_iter()
            .find(|comment| comment.id == id)
            .map(|comment| comment.world);
        self.project.save()?;
        Ok(vec![text_content(&json!({
            "page": page_index,
            "comment_id": id,
            "created": true,
            "world": world,
        }))?])
    }
}

fn parse<T: serde::de::DeserializeOwned>(arguments: Value) -> Result<T> {
    serde_json::from_value(arguments).context("parsing the tool arguments")
}

fn initialize_response(params: &Value) -> Value {
    let requested = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(LATEST_PROTOCOL_VERSION);
    let protocol_version = if PROTOCOL_VERSIONS.contains(&requested) {
        requested
    } else {
        LATEST_PROTOCOL_VERSION
    };
    json!({
        "protocolVersion": protocol_version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": {
            "name": "fanta-headless",
            "title": "Fanta (headless)",
            "version": env!("CARGO_PKG_VERSION"),
        },
    })
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    })
}

fn text_content(value: &Value) -> Result<Value> {
    Ok(json!({
        "type": "text",
        "text": serde_json::to_string(value).context("serializing the tool result")?,
    }))
}

/// Like [`text_content`], but refuses a result too large to be carried or
/// read, with the exact wording the live server uses.
fn bounded_text_content(value: &Value, narrowing_hint: &str) -> Result<Vec<Value>> {
    let text = serde_json::to_string(value).context("serializing the tool result")?;
    if text.len() > MAX_JSON_RESPONSE_BYTES {
        bail!(
            "this result would be {} bytes, over the {MAX_JSON_RESPONSE_BYTES}-byte response \
             budget, and no part of it was returned. {narrowing_hint}",
            text.len()
        );
    }
    Ok(vec![json!({ "type": "text", "text": text })])
}

fn schema_for<T: schemars::JsonSchema>() -> Value {
    let mut settings = schemars::generate::SchemaSettings::draft07();
    settings.inline_subschemas = true;
    serde_json::to_value(settings.into_generator().root_schema_for::<T>())
        .unwrap_or(Value::Null)
}

fn read_only() -> Value {
    json!({ "readOnlyHint": true, "destructiveHint": false, "openWorldHint": false })
}

fn non_destructive_write() -> Value {
    json!({
        "readOnlyHint": false,
        "destructiveHint": false,
        "idempotentHint": false,
        "openWorldHint": false,
    })
}

/// The advertised tools, in the live server's order, each described by its
/// argument struct's doc comment.
fn tool_entries() -> Vec<Value> {
    fn tool<T: schemars::JsonSchema>(name: &str, annotations: Value) -> Value {
        let schema = schema_for::<T>();
        let description = schema
            .get("description")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        json!({
            "name": name,
            "description": description,
            "inputSchema": schema,
            "annotations": annotations,
        })
    }
    vec![
        tool::<GetEditorStateArgs>("get_editor_state", read_only()),
        tool::<DesignSystemQuery>("get_design_system", read_only()),
        tool::<DesignAssetRequest>("prepare_design_asset", non_destructive_write()),
        tool::<BatchGetArgs>("batch_get", read_only()),
        tool::<BatchDesignArgs>(
            "batch_design",
            json!({
                "readOnlyHint": false,
                "destructiveHint": true,
                "idempotentHint": false,
                "openWorldHint": false,
            }),
        ),
        tool::<GetScreenshotArgs>("get_screenshot", read_only()),
        tool::<ReadFnxSourceArgs>("read_fnx_source", read_only()),
        tool::<ValidateFnxSourceArgs>("validate_fnx_source", read_only()),
        tool::<GetGuidelinesArgs>("get_guidelines", read_only()),
        tool::<ImportImageArgs>(
            "import_image",
            json!({
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": false,
                "openWorldHint": true,
            }),
        ),
        tool::<ListCommentsArgs>("list_comments", read_only()),
        tool::<ReplyCommentArgs>("reply_comment", non_destructive_write()),
        tool::<AddCommentArgs>("add_comment", non_destructive_write()),
        tool::<ReportActivityArgs>("report_agent_activity", non_destructive_write()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_headless::document::HeadlessDocument;
    use std::collections::BTreeMap;

    fn new_project(root: &Path) -> Result<()> {
        let document = HeadlessDocument::empty()?;
        fanta_format::write_project_tree(root, document.doc(), &BTreeMap::new())?;
        Ok(())
    }

    fn request(server: &mut Server, id: u64, method: &str, params: Value) -> Value {
        let line = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let response = server
            .handle_line(&line.to_string())
            .expect("a request with an id gets a response");
        assert_eq!(response["id"], json!(id), "{response}");
        response
    }

    fn call_text(server: &mut Server, id: u64, tool: &str, arguments: Value) -> Value {
        let response = request(
            server,
            id,
            "tools/call",
            json!({ "name": tool, "arguments": arguments }),
        );
        let text = response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("no text content: {response}"));
        serde_json::from_str(text).expect("tool results are JSON")
    }

    /// An agent learns what a tool does only from its description; the
    /// fourteen names and their one-vocabulary descriptions are the live
    /// server's (spec 0008 R7).
    #[test]
    fn every_tool_is_described() {
        let tools = tool_entries();
        assert_eq!(tools.len(), 14);
        let names: Vec<&str> = tools
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name"))
            .collect();
        assert_eq!(
            names,
            [
                "get_editor_state",
                "get_design_system",
                "prepare_design_asset",
                "batch_get",
                "batch_design",
                "get_screenshot",
                "read_fnx_source",
                "validate_fnx_source",
                "get_guidelines",
                "import_image",
                "list_comments",
                "reply_comment",
                "add_comment",
                "report_agent_activity",
            ]
        );
        for tool in &tools {
            let description = tool["description"].as_str().expect("description");
            assert!(
                description.len() >= 20,
                "{} is undescribed: {description:?}",
                tool["name"]
            );
            assert_eq!(tool["inputSchema"]["type"], json!("object"), "{tool}");
        }
    }

    /// The framing contract: parse errors answer `-32700` with `id: null`,
    /// notifications are silently skipped, `ping` returns `{}`, and an
    /// unknown method answers `-32601`.
    #[test]
    fn framing_matches_the_live_server() {
        let directory = tempfile::tempdir().expect("tempdir");
        new_project(directory.path()).expect("new project");
        let mut server = Server::open(directory.path()).expect("open");

        let parse_error = server.handle_line("{not json").expect("parse error");
        assert_eq!(parse_error["error"]["code"], json!(-32700), "{parse_error}");
        assert_eq!(parse_error["id"], Value::Null);

        assert_eq!(
            server.handle_line(
                &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string()
            ),
            None,
            "notifications get no response"
        );
        assert_eq!(server.handle_line("   "), None);

        let pong = request(&mut server, 1, "ping", json!({}));
        assert_eq!(pong["result"], json!({}), "{pong}");

        let unknown = request(&mut server, 2, "resources/list", json!({}));
        assert_eq!(unknown["error"]["code"], json!(-32601), "{unknown}");

        let initialize = request(
            &mut server,
            3,
            "initialize",
            json!({ "protocolVersion": "2025-03-26", "capabilities": {} }),
        );
        assert_eq!(initialize["result"]["protocolVersion"], json!("2025-03-26"));
        assert_eq!(
            initialize["result"]["serverInfo"]["name"],
            json!("fanta-headless")
        );
        let future = request(
            &mut server,
            4,
            "initialize",
            json!({ "protocolVersion": "2031-01-01" }),
        );
        assert_eq!(
            future["result"]["protocolVersion"],
            json!(LATEST_PROTOCOL_VERSION)
        );
    }

    /// The whole loop an agent runs without the app: initialize, list the
    /// fourteen tools, read the editor state, edit with `batch_design`, and
    /// find the edit saved to the project directory.
    #[test]
    fn edits_land_on_disk_without_the_app() {
        let directory = tempfile::tempdir().expect("tempdir");
        new_project(directory.path()).expect("new project");
        let mut server = Server::open(directory.path()).expect("open");

        let tools = request(&mut server, 1, "tools/list", json!({}));
        assert_eq!(
            tools["result"]["tools"].as_array().map(Vec::len),
            Some(14),
            "{tools}"
        );

        let state = call_text(&mut server, 2, "get_editor_state", json!({}));
        assert_eq!(state["pages"].as_array().map(Vec::len), Some(1), "{state}");
        assert_eq!(
            state["project_root"],
            json!(directory.path().display().to_string())
        );

        let applied = call_text(
            &mut server,
            3,
            "batch_design",
            json!({
                "label": "Add panel",
                "ops": [{
                    "op": "create_node", "node_type": "rectangle", "name": "Smoke Panel",
                    "x": 24.0, "y": 32.0, "width": 120.0, "height": 80.0, "fill": "#123456",
                }],
            }),
        );
        assert_eq!(applied["applied"], json!(true), "{applied}");
        let created = applied["created"][0].as_str().expect("created id");

        let listing = call_text(&mut server, 4, "read_fnx_source", json!({}));
        let files: Vec<&str> = listing["files"]
            .as_array()
            .expect("files")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let page_svg = files
            .iter()
            .find(|file| file.ends_with("page.svg"))
            .unwrap_or_else(|| panic!("no page.svg in {files:?}"));
        let on_disk =
            std::fs::read_to_string(directory.path().join(page_svg)).expect("saved page");
        assert!(
            on_disk.contains("Smoke Panel"),
            "the batch_design edit did not reach {page_svg}"
        );

        let nodes = call_text(
            &mut server,
            5,
            "batch_get",
            json!({ "ids": [created], "include_geometry": true }),
        );
        assert_eq!(
            nodes["nodes"][0]["world_bounds"]["width"],
            json!(120.0),
            "{nodes}"
        );

        let escape = request(
            &mut server,
            6,
            "tools/call",
            json!({ "name": "read_fnx_source", "arguments": { "path": "../escape.svg" } }),
        );
        assert_eq!(escape["error"]["code"], json!(-32603), "{escape}");
    }
}
