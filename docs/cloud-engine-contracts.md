# Cloud engine contracts

The interfaces between the Fanta app, the v2 API (fanta-backend) and the cloud
engine worker (`crates/fanta-engine`), frozen on 2026-10-21 for cloud AI Review,
Ultra review and cloud generation. Both repositories keep a copy of this page.
Change a contract only together with the code on both sides, and update both
copies in the same pair of pull requests.

```
app ──► v2 API (/v2/reviews, /v2/design-generations) ──► job driver ──► engine POST /v1/run
                                                              │               │
                                                              │               └─► GitHub: commit tarball (read only)
                                                              └─► GitHub: one branch, one commit, one pull request
```

The engine is stateless. It reads a project from a GitHub commit plus an
optional overlay of unpushed files, runs steps on one loaded document and
answers. It never writes to GitHub; v2 does.

## Engine HTTP API

### Authentication

Both checks apply to every route except `GET /healthz`.

- Cloud Run IAM: only v2's service account holds `roles/run.invoker`. v2 sends
  an ID token from the metadata server whose audience is the service URL.
- `X-Fanta-Internal: t=<unix seconds>,v1=<hex>`, where `v1` is
  `HMAC-SHA256(ENGINE_HMAC_SECRET, "{t}.{hex sha256(raw body)}")` in lowercase
  hex. The engine rejects a missing or malformed header, a mismatch, or
  `|now − t| > 300 s` with `401`. This is the scheme of
  `fanta-backend/gpu/common/fanta_gpu_common/auth.py`.

### `GET /healthz`

```json
{"version": "0.1.0", "git_sha": "116d0cb136…", "schema_version": 4,
 "capabilities": ["step:inspect", "step:render"]}
```

`schema_version` is the newest project schema the engine reads.
`capabilities` lists `step:<name>` for each step this build runs; v2 checks it
before it sends a step. The list also carries
`fanta_design_ops::DESIGN_OP_CAPABILITIES`, the ops and query options `apply`
accepts.

### `POST /v1/run`

```json
{
  "request_id": "rev_01J…",
  "deadline_ms": 120000,
  "source": {
    "kind": "github",
    "repo": "acme/site",
    "sha": "0123456789abcdef0123456789abcdef01234567",
    "path": "design",
    "token": "ghs_…",
    "overlay": {"files": [
      {"path": "pages/home/page.fnx", "content_b64": "…"},
      {"path": "pages/old/page.fnx", "deleted": true}
    ]}
  },
  "steps": [
    {"step": "inspect"},
    {"step": "project", "page": {"index": 0}},
    {"step": "render", "targets": [{"id": "home", "page": {"index": 0}}]}
  ]
}
```

| Field | Rule |
| --- | --- |
| `request_id` | Echoed in the response and logs. |
| `deadline_ms` | 1 to 280000. A step that would start after the deadline is reported as `deadline_exceeded` and the run stops. |
| `source.kind` | `github`, or `dir` (`{"kind": "dir", "dir": "/path"}`, development only, refused in production). |
| `source.repo`, `sha` | `owner/name` and a full 40-character commit SHA. |
| `source.path` | The project folder inside the repository, `/`-separated; `""` for the root. |
| `source.token` | A read-only installation token scoped to `repo`. Never logged. |
| `source.overlay.files` | Each entry has `path` (relative to the project folder) and either `content_b64` or `deleted: true`. At most 8 MiB decoded in total. |
| `steps` | At least one. Unknown steps or fields are refused with `400`. |

The engine reads the source this way:
1. It checks access with `GET /repos/{repo}/commits/{sha}`.
2. It downloads the commit tarball and extracts only `path/`. Links, absolute
   paths, `..` components, and archives over 512 MiB extracted or 20,000 files
   are refused.
3. It lays the overlay over the extracted folder.
4. It keeps the three most recent checkouts, keyed by `repo@sha:path`, and
   copies a checkout before a run writes to it.

#### Steps

Steps run in order on one loaded document. `PageRef` is `{"index": n}` (the
position in the editor's page list, which `inspect` reports and
`CommentDraft.page` uses), `{"id": "n_…"}` (a page root id), or
`{"created": i}` (the page made by this run's `i`-th `create_page`, from 0).

| Step | Fields | Result |
| --- | --- | --- |
| `inspect` | — | `project_id`, `schema_version`, `pages` (index, id, name, hidden, active, source file, up to 200 top-level layers with ids, kinds, names and world bounds, and `frame_count`), `active_page`, `components` (first 100) and `component_count`, `design_system`, `rules` (the project's `fanta.md`, or `null` with `rules_error` when it cannot be read), `motion` clips, prototype `flows` and `fonts` (the families the text names). |
| `create_page` | `name` | `{"page": {"id": "n_…"}}` |
| `apply` | `page?` (`PageRef`, where parentless ops land), `label`, `ops` (`DesignOp[]`, the `batch_design` vocabulary), `commit` (default `true`; `false` checks the batch and rolls it back) | The `batch_design` result: `applied`, per-op status and `created` ids. |
| `add_comments` | `label`, `comments` (`CommentDraft[]`; a draft without `page` goes on the active page) | `{"comments": [{"ok": true, "id": "…"} \| {"ok": false, "error": "…"}]}`, one entry per draft. The accepted drafts land as one undo step. |
| `project` | `page` (`PageRef`), `depth` (default 8), `limit` (default 50) | `{"nodes": […]}`: the page's top-level layers in the style projection of `batch_get` with `detail: "style"`. |
| `render` | `targets`: 1 to 8 of `{id, page, node?, max_dimension (default and maximum 1568), motion_clip?, playhead_ms?}` | `{"images": [{"target_id", "png_b64", "width", "height", "bounds": [x, y, width, height]}]}` |
| `changeset` | `contents` (default `false`), `ops` (default `false`) | Saves the document, then `{"files": [{"path", "status": "added" \| "modified" \| "deleted", "sha256" (hex, `null` when deleted), "content_b64"?}], "ops"?: Operation[]}`. `ops` is the `fanta_doc::Operation` journal the app replays. Every edit stamps `doc/metadata.json` with its time, as the editor does, so that file is usually listed too. |

#### Response

`200` whenever the project loaded, even if a step failed:

```json
{
  "request_id": "rev_01J…",
  "engine": {"version": "0.1.0", "git_sha": "…", "schema_version": 4, "capabilities": […]},
  "source": {"sha": "0123…", "project_id": "d_01J…", "warnings": [],
             "fonts": {"missing": ["Brand Sans"], "substituted": [{"family": "Brand Sans", "used": "Inter"}]}},
  "results": [
    {"step": "inspect", "ok": true, "result": {…}},
    {"step": "render", "ok": false, "error": {"code": "step_failed", "message": "…"}}
  ]
}
```

`results` has one entry per step that ran. A failed step is the last entry,
because the run stops there. Review prompts mention `fonts.missing` and
`fonts.substituted`, since renders use the substitutes.

#### Errors

Any other status answers `{"error": {"code": "…", "message": "…"}}`.

| Status | `code` | When |
| --- | --- | --- |
| 400 | `bad_request` | Malformed JSON, unknown step or field, a limit above. |
| 401 | `unauthorized` | Missing or wrong `X-Fanta-Internal`. |
| 403 | `forbidden` | The token cannot read the repository. |
| 404 | `not_found` | The repository, commit or project folder does not exist. |
| 409 | `schema_too_new` | The project was saved by a newer Fanta than the engine. v2 answers the app with `409 engine_outdated`. |
| 413 | `payload_too_large` | Request body over 16 MiB, overlay over 8 MiB, or source over the archive limits. |
| 422 | `project_load_failed` | The folder is not a Fanta project or does not load. |
| 429 | `busy` | The instance is at its concurrency (2). v2 retries twice. |
| 504 | `deadline_exceeded` | The deadline passed before the project loaded. |
| 500 | `internal` | Anything else. |

Inside a `200`, a step error is `step_failed`, or `deadline_exceeded` when the
deadline passed before the step started.

`fanta-engine run --request request.json` runs the same path on a file and
prints the response; `fanta-engine healthz` prints the health answer.

## Rust facade (`crates/fanta-headless`)

The engine calls the editor's code through this facade. The editor uses the
same functions, so a review applied in the app and a review written by the
engine produce the same document.

```rust
HeadlessProject::open(root) -> Result<HeadlessProject>
    .doc() -> &Doc
    .warnings() -> &[String]
    .font_families() -> BTreeSet<String>
    .inspect() -> Result<Value>
    .resolve_page(&PageRef) -> Result<NodeId>
    .create_page(name) -> Result<NodeId>
    .apply(page, label, &[DesignOp], commit) -> Result<Value>
    .add_comments(label, &[CommentDraft]) -> Result<Vec<Result<String>>>
    .settle_layout() -> Result<()>
    .project_page(page, depth, limit) -> Result<Vec<Value>>
    .render(&RenderTarget) -> Result<RenderedImage>
    .save() -> Result<()>
    .journal() -> &[Operation]
    .loaded_hashes() -> &BTreeMap<String, [u8; 32]>
changeset(loaded_hashes, work_dir) -> Result<Vec<ChangedFile>>
agent_add_comments_op(&Doc, page, &[CommentDraft]) -> (Vec<Result<String>>, Option<Operation>)
```

`DesignOp`, `CommentDraft` and the query types live in `crates/fanta-design-ops`.
Their serde form is the wire form of `apply` and `add_comments`.

## v2 API: reviews

These routes use the `ai` scope and accept a Clerk session or an `fnt_live_`
key.

```
POST /v2/reviews
  {depth: "standard" | "ultra",
   source: {cloud_project_id? | project_ref? | repo + path, ref? | sha?, overlay?},
   scope?: {pages?: [index], frames?: ["n_…"]},
   brief?: string,
   delivery?: {mode: "app" | "cloud_comments" | "pull_request"},
   client?: {app_version, platform}}
  202 {id, status: "queued", estimate: {credits_max}, poll_after_ms}
  402 insufficient_credits
  404 project_not_linked {install_url}
  409 sha_not_on_remote | engine_outdated
  429

GET  /v2/reviews/{id}        → {status, phase, progress, summary, findings[], renders[], delivery, credits, error}
GET  /v2/reviews?project_ref=…&undelivered=true
POST /v2/reviews/{id}/cancel
POST /v2/reviews/{id}/acknowledge
```

`status` is `queued`, `running`, `completed`, `failed` or `canceled`. A
finding is:

```
{id, page: {index, id, name}, node_id: "n_…" | null, world: [x, y],
 category, severity: "info" | "minor" | "major" | "blocker",
 title, message, suggested_fix: {summary, ops: DesignOp[]} | null,
 sources: [string], confidence: 0..1}
```

`category` is one of the app's nine: `layout`, `typography`, `color`,
`accessibility`, `consistency`, `content`, `interaction`, `motion`, `other`.
Lint severities map `error` → `major`, `warning` → `minor`, `info` → `info`.

A finding becomes this `CommentDraft`, which the app's `agent_add_comments_op`
and the engine's `add_comments` both accept:

```json
{"page": <page.index>, "node": <node_id>, "world": <world when node_id is null or gone>,
 "text": "**<title>**\n\n<message>", "author": "Fanta Review",
 "severity": <severity>, "category": <category>, "review_run_id": <review id>,
 "suggested_fix": <suggested_fix>}
```

## v2 API: design generations

```
POST /v2/design-generations
  {brief, source, page_name?, options?, delivery?: "app" | "pull_request" | "both"}
  202 {id, status: "queued", estimate, poll_after_ms}

GET  /v2/design-generations/{id}
  → {status, phase, plan: {tasks[]}, page, preview,
     changeset: {files: [{path, status, sha256, url}], ops_url, ops_format: 1},
     delivery, credits}
POST /v2/design-generations/{id}/cancel
POST /v2/design-generations/{id}/acknowledge
```

`ops_url` serves the `Operation` journal. `ops_format` 1 is the serde form of
`fanta_doc::Operation` at the engine's `schema_version`. The app applies the
journal as one undo step; when that fails, it writes `changeset.files` and
reloads.

## Versioning

- Every app release tag also deploys the engine built from the same commit.
- A project newer than the engine is refused (`409`) rather than read
  partially.
- A new step or op appears in `capabilities` before v2 sends it.
