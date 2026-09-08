//! Path constants, the `fanta.json` project manifest, scaffolding, and the
//! small shared JSON/file helpers the read/write projections both use.
//!
//! Everything here is about the *shape* of a project directory (spec 09 §A.2);
//! the actual doc projection lives in [`super::write`] / [`super::read`].

use crate::error::{FormatError, Result};
use fanta_doc::{Doc, DocId, SCHEMA_VERSION};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// Manifest file at the root of every project directory.
pub(crate) const FANTA_JSON: &str = "fanta.json";
/// Value of [`ProjectManifest::format`] — the tag that makes a directory a
/// Fantaisa project.
pub(crate) const FORMAT_TAG: &str = "fanta-project";
/// Current project *layout* version (independent of the doc `schema_version`).
///
/// v2 replaced one-JSON-file-per-node (`<design>/nodes/<id>.json`) with one
/// readable `.fnx` source file + an `.ids` sidecar per page/component. v1 trees
/// still read (the loader falls back to `nodes/`) and upgrade to v2 on the next
/// save (the full-overwrite write drops the old `nodes/` dirs).
///
/// v3 renamed the design directories from raw ids (`pages/n_<ULID>/`) to
/// human-readable slugs of the design's name (`pages/home-screen/`); identity
/// moved into the JSON headers (`page.json` gained `"id"`, `def.json` already
/// carried the `ComponentDef` id). v2 trees still read (the loader falls back
/// to parsing the directory name as the id) and upgrade to v3 dirs on the next
/// full-overwrite save.
///
/// **v4** marks name-based references in `.fnx` sources (`component="Button"`,
/// `$Collection/Name` token paths): the printer only emits names inside a
/// v4-tagged project, so a v3-era build never encounters source it cannot
/// resolve — its manifest gate refuses v4 outright instead of half-loading.
/// v3 trees still read; the next full-overwrite save upgrades them.
pub(crate) const PROJECT_VERSION: u32 = 4;

pub(crate) const DOC_DIR: &str = "doc";
pub(crate) const PAGES_DIR: &str = "pages";
pub(crate) const COMPONENTS_DIR: &str = "components";
/// Layout v4 multi-kind roots (session architecture).
pub(crate) const GRAPHICS_DIR: &str = "graphics";
pub(crate) const PROTOTYPES_DIR: &str = "prototypes";
pub(crate) const MOTION_DIR: &str = "motion";
pub(crate) const AUDIO_DIR: &str = "audio";
/// Optional workspace source (synthesized when missing).
pub(crate) const WORKSPACE_FNX: &str = "workspace.fnx";
pub(crate) const ASSETS_DIR: &str = "assets";
pub(crate) const PREVIEWS_DIR: &str = "previews";
pub(crate) const EXPORTS_DIR: &str = "exports";
/// Per-design node directory (`pages/<id>/nodes/`, `components/<id>/nodes/`).
/// v1-only; v2 reads it as a fallback but never writes it.
pub(crate) const NODES_DIR: &str = "nodes";
/// v2 per-page design source + its id/index sidecar.
pub(crate) const PAGE_FNX: &str = "page.fnx";
pub(crate) const PAGE_IDS: &str = "page.ids.json";
/// v2 per-component master source + its id/index sidecar.
pub(crate) const MASTER_FNX: &str = "master.fnx";
pub(crate) const MASTER_IDS: &str = "master.ids.json";
/// Page header file: `{ id, name?, order }`. The `id` (added in layout v3,
/// display form `n_…`) is the page's identity of record; v2 trees encoded it
/// in the directory name instead.
pub(crate) const PAGE_JSON: &str = "page.json";
/// Component master file: the `ComponentDef` JSON.
pub(crate) const DEF_JSON: &str = "def.json";
/// Component-set registry (small, rarely edited concurrently).
pub(crate) const SETS_JSON: &str = "sets.json";
/// Graphics design header (layout v4); source names live in `fanta_fnx::artifact_file_names`.
pub(crate) const GRAPHICS_JSON: &str = "graphics.json";
/// Pseudo-page directory for orphan nodes — nodes whose parent chain reaches
/// neither a page root nor a component root. Underscore-prefixed so it can
/// never collide with a page directory: v3 slugs contain only `[a-z0-9-]` and
/// v2 id names (`n_…`) never start with an underscore.
pub(crate) const LOOSE_DIR: &str = "_loose";

pub(crate) const METADATA_JSON: &str = "metadata.json";
pub(crate) const VARIABLES_JSON: &str = "variables.json";
pub(crate) const ACTIVE_MODES_JSON: &str = "active_modes.json";
pub(crate) const MOTION_JSON: &str = "motion.json";
pub(crate) const FLOW_START_JSON: &str = "flow_start.json";

pub(crate) const GITIGNORE_NAME: &str = ".gitignore";
/// Derived/transient directories and OS noise stay out of history.
pub(crate) const GITIGNORE: &str = "previews/\nexports/\n.DS_Store\n";

/// Agent-guide file at the project root. Zed's agent auto-loads it as a rules
/// file (`RULES_FILE_NAMES` in `prompt_store`), so seeding it teaches any AI
/// agent working the project how the format round-trips.
pub(crate) const AGENTS_MD_NAME: &str = "AGENTS.md";

/// Project-local declarations consumed by TypeScript-compatible editor
/// tooling when an `.fnx` source is open.
pub(crate) const FNX_TYPES_NAME: &str = "fnx.d.ts";

pub(crate) const FNX_TYPES: &str = include_str!("fnx.d.ts");

pub(crate) const FNX_PRETTIER_NAME: &str = ".prettierrc.json";
pub(crate) const FNX_PRETTIER: &str = include_str!("fnx.prettierrc.json");

/// The seed contents of [`AGENTS_MD_NAME`]. Written only when absent (unlike
/// the always-regenerated `.gitignore`) so a user's edits are never clobbered.
/// The guide is intentionally practical and format-accurate — it is derived
/// from the real `.fnx` projection, not invented.
pub(crate) const AGENTS_MD: &str = r##"# Working on this Fanta design project

This directory is a Fanta *design project*: a Figma-class design stored as
editable source files. **The design is the source of truth.** Editing these
files edits the design. When this project is open in the editor, saving your
edits to the `.fnx` files reloads the canvas about 300 ms later, and edits made
on the canvas save back to these same files. So working here as a text agent
*is* the design-to-canvas loop — no export step.

If Fanta is running with this project open you also have a more direct route: a
live MCP server that reads and edits the focused canvas. Jump to *Driving the
open canvas over MCP* below — prefer it when the app is up, and fall back to
editing the files by hand when it is not.

## Directory layout

```
fanta.json                       # project manifest (format tag, versions, ids) — do not hand-edit
fnx.d.ts                         # project-local FNX tag and property types for editor tooling
.prettierrc.json                 # FNX-safe Prettier defaults; user-editable
doc/
  metadata.json                  # title + timestamps
  variables.json                 # design variables / tokens and their modes
  active_modes.json              # which mode is active per variable collection
  motion.json                    # animation clips, tracks, and keyframes
  flow_start.json                # prototype start page (or null)
pages/<page-slug>/
  page.json                      # { "id": "n_…", "name": ..., "order": N }
  page.fnx                       # the page's node tree as readable source  <- edit this
  page.ids.json                  # id + sibling-order sidecar for page.fnx  — do not hand-edit
components/<component-slug>/
  def.json                       # component definition (id, root, name)
  master.fnx                     # the component master's tree as source    <- edit this
  master.ids.json                # id sidecar for master.fnx                — do not hand-edit
components/sets.json             # component-set (variant) registry
assets/<family>/<AssetId>.<ext>  # shared binary assets, one folder per family:
                                 #   images/ video/ audio/ models/ svg/ fonts/ other/
previews/  exports/              # generated output — git-ignored, never an input
```

Directory names under `pages/` and `components/` are human-readable slugs of
the design's name (two pages named the same get `-2`, `-3` suffixes). The
design's *identity* lives in the JSON header next to the source: `page.json`'s
`"id"` and `def.json`'s `id`. Do not hand-edit ids, and renaming the folder is
not how you rename a page — edit the root element's `name` attribute and the
app re-derives the folder name on the next save.

## The `.fnx` language

A `.fnx` file is the readable, JSX/TSX-style projection of one page or component
subtree. It opens with `// @generated fanta source …` — treat it as generated
source you may edit, not as a file to reformat. Each scene node is one JSX
element:

- The **tag** is the node kind: `Frame` (group/frame), `Vector` (shape),
  `Text`, `Image`, `Video`, `Audio`, `Model3D`, `NodeGraph`, `AiArtifact`,
  `Instance` (a component instance), `Embed`. Two **authoring shorthands**
  also parse: `<Rect width={100} height={50} …/>` and
  `<Ellipse width={64} height={64} …/>` — each is sugar for a `<Vector>` with
  the equivalent generated `path`, and simple shapes print back in the sugar
  form. Give a sugar tag `width`/`height` (required) plus any normal vector
  attributes (`fills`, `strokes`, `corner_radius`, `x`, `y`, …); do not give
  it an explicit `path`.
- **Attributes** are the node's fields, verbatim. String attributes render as
  `name="Header"`; everything else renders inside braces, e.g. `opacity={1.0}`,
  `x={280.0}`, `corner_radius={2.0}`. Nesting is by hierarchy: an element's
  children are its child nodes, in render (z) order.
- **Colors** render through `fnxColor("#RRGGBB")`, or an eight-digit hex when
  not fully opaque (e.g. `fnxColor("#FFFFFF1A")`).
- **Components by name**: `<Instance component="Button" />` references the
  component named "Button" (a bare 26-character id is also always accepted).
  If two components share a name, the id form is required — the app prints
  ids for ambiguous names.
- **Design tokens by path**: variable bindings accept `$Collection/Name`
  paths and a readable map form:
  `bindings={{"fill_color": "$Theme/bg", "text_style": "$Type/Heading"}}`.
  The tokens themselves are defined in `doc/variables.json`.
- A typo in an attribute name, component name, or token path fails with a
  `line:column` error and a suggestion — a `.fnx` that loads has resolved
  every reference.

A short real snippet:

```jsx
<Frame background={{"kind": "solid", "color": fnxColor("#444444")}} blend_mode="normal"
       corner_radius={2.0} name="Header" opacity={1.0} x={-734.0} y={-491.0}>
  <Text align="left" content="Little Lemon" name="Title" opacity={1.0}
        style={{"font_family": "Inter", "size_px": 64.0, "weight": 700, "color": fnxColor("#1E1E1E")}}
        x={275.0} y={146.0} />
</Frame>
```

Common attributes: `name`, `x`/`y` (position, see below), `opacity`,
`blend_mode`, `background`/`fills`/`strokes` (paints, with colors as hex),
`corner_radius`, `clip_size`, `content` and `style` (Text), `path` (Vector),
`auto_layout` (auto-layout frames), `meta` (source metadata like
`figma_type`/`figma_id`). Component instances carry `component` (the master's
id), `overrides` (per-instance changes such as swapped text or colors),
`prop_values`, and `derived`/`local_size` (solver-computed geometry).

### Coordinates

`x`/`y` are the node's translation in pixels relative to its parent (they are
the pure-translation part of the node's transform). Change them to move a node;
change `clip_size` / `local_size` to resize. You may also write `width={…}`
`height={…}`: on load they fold into `clip_size` on a `Frame` and into
`local_size` on every other sized tag, and an explicit `clip_size`/`local_size`
already on the element wins over the sugar. The one exception is `Vector`, whose
`local_size` is the SVG-viewport *clip* rather than the shape's geometry —
`width`/`height` on a `<Vector>` are ignored, so resize a vector by editing its
`path`, or author it as `<Rect>`/`<Ellipse>` sugar where `width`/`height` do
generate the path. If you spell a size as `width`/`height`, an in-place edit of
that element keeps your spelling; a full reprint of the file emits the canonical
field instead.

Leaving the size off entirely is tolerated, not encouraged: a `Text`, media or
`Instance` element with neither `local_size` nor `width`/`height` is backfilled
on read — media from its intrinsic `natural_size`, text from an estimate of its
style and content, anything else with a 200 x 100 placeholder box — instead of
failing the whole design. Expect the next save to write that guess back as a
real `local_size`.

A node with rotation, scale, or skew shows a raw
`transform={[a, b, c, d, tx, ty]}` array instead of `x`/`y` — leave that
verbatim unless you mean to change the matrix.

### Ids live in the sidecar, not the source

Stable node ids and fractional sibling order are lifted out of the `.fnx` into
the neighboring `page.ids.json` / `master.ids.json` sidecar (in pre-order), so
the source stays readable. **Do not hand-edit the `.ids.json` sidecars or ids
in `fanta.json`.** The round trip stays lossless as long as you leave identity
to the sidecar and edit only the readable source.

## What you can do by editing `.fnx`

Editing the readable source changes the design directly:

- **Rename a layer** — change its `name="…"`.
- **Edit text** — change a `Text` element's `content="…"`, or an instance's
  `text_content` override.
- **Recolor** — change a hex color inside `fills` / `background` / `strokes` /
  Text `style.color`. Colors use the TSX-valid `fnxColor("#RRGGBB")` helper.
- **Move / resize** — change `x` / `y` (and size-related attributes).
- **Reorder** — change an element's position among its siblings.
- **Duplicate / delete** — copy an element (the app assigns a fresh id on the
  next save) or remove it.

## Guardrails

- Keep the JSX well-formed: balanced tags, valid attribute values. A `.fnx`
  that fails to parse will not load.
- Do not hand-edit `page.ids.json`, `master.ids.json`, or the ids in
  `fanta.json` — ids and sibling order are owned by the sidecars.
- Assets are shared **by reference**: they live under `assets/<family>/` and are
  named by content id. Reference them; never paste binary data inline into a
  `.fnx`.
- For **auto-layout** frames (those with an `auto_layout={…}` attribute), the
  app solves child positions from the layout rules — set the auto-layout
  properties and let the solver place children rather than fighting it with
  manual `x`/`y`.
- `previews/` and `exports/` are generated; they are never read back.
- Positioning: `x`/`y` are sugar for a translation-only transform. A node with
  rotation/scale shows a raw `transform={[a,b,c,d,tx,ty]}` instead — writing
  BOTH `x`/`y` and `transform` on one element is an error, not a merge.

## Driving the open canvas over MCP

While the project is open in Fanta, a local MCP server exposes **the design
canvas that is focused right now**. It is on by default; it is turned off with
`"fanta_live_mcp": { "enabled": false }` in settings.

- **Connecting.** In Fanta, run the **Connect External Agent** command (Help
  menu, or the command palette). It copies a ready-to-paste
  `claude mcp add -s user fanta -- /path/to/Fanta --mcp-stdio` to the clipboard,
  and shows the equivalent `[mcp_servers.fanta]` snippet for
  `~/.codex/config.toml`. There is deliberately no `.mcp.json` committed in this
  project: it would have to hard-code an install path, and the user-scope
  command above covers the same ground.
- **How the transport works.** The server listens on a Unix socket in a private
  temp directory and advertises the path in
  `~/Library/Application Support/Fanta/fanta_live_mcp.json` (the platform data
  directory elsewhere) as `{"socket": "…/mcp.sock", "pid": 1234}`.
  `fanta --mcp-stdio` reads that file and bridges stdio to the socket. The file
  is rewritten on every launch and is *not* deleted on quit, so anything reading
  it directly must check that `pid` is still alive before trusting `socket`.

### The five tools

**`get_editor_state`** — takes no arguments. Returns the project name and
`project_root`, every page (`index`, `name`, root node id, node count, and which
one is `active`), the current `selection` ids, the `viewport`, `total_nodes`,
and whether the canvas `is_editable` and is `dirty`. Call it first: the page
indices and node ids it returns are what every other tool takes.

**`batch_get`** — `{ ids?, page?, depth?, include_geometry? }`. With `ids`,
returns those nodes in full detail. Without them it lists one page's tree in
compact form, where `page` is a page *index* (default: the active page), `depth`
limits how many levels of children come back, and `include_geometry: true` adds
world-space bounding boxes.

**`batch_design`** — `{ ops, label? }`. Applies `ops` in order as **one undo
step**, named by `label` (default `"MCP edit"`). If any op fails the whole batch
rolls back and the error names the op that failed. Each op is an object tagged
by `"op"`:

- `create_node` — `node_type` (`"frame"`, `"rectangle"`, `"ellipse"` or
  `"text"`), `x`, `y`, `width`, `height` (all required; `x`/`y` are the world
  coordinates of the top-left corner), plus optional `parent` (a frame id; omit
  to place on the active page), `name`, `fill` (`"#RRGGBB"` or `"#RRGGBBAA"`;
  the glyph color for text), `text` and `font_size`.
- `create_image` — `source` (a `data:image/…;base64,…` URI or raw base64 of
  encoded PNG/JPEG/WebP/GIF bytes; `http(s)` URLs are *not* fetched), `x`, `y`,
  plus optional `parent`, `name`, `width`/`height` (give one and the other
  scales to preserve aspect; give neither for the natural pixel size) and `meta`
  (JSON stored in the node's metadata, e.g. generation provenance). The bytes
  become a project asset under `assets/images/` on the next save.
- `set_props` — `id` plus only the fields you are changing: `name`, `x`, `y`,
  `width`, `height`, `opacity` (0.0–1.0), `fill`, `corner_radius`, `text`,
  `hidden`, `locked`.
- `reparent` — `id`, optional `parent` (omit to move to the active page root)
  and optional `index` among the new siblings (0 = bottom; omit to append on
  top). The node keeps its world position.
- `delete` — `id`; removes that node and its whole subtree.
- `select` — `ids`, replacing the editor selection.
- `set_viewport` — optional `center` (`[x, y]`) and `zoom`.

Strokes, gradients, shadows, auto-layout, fonts and components are **not**
`batch_design` properties. For those, read the page's `.fnx`, edit the file with
your normal file tools, and let the canvas reload — that edit is also a reviewable
git diff, which a `batch_design` call is not until the editor saves.

**`get_screenshot`** — `{ page?, node?, max_dimension? }`, all optional. Renders
a PNG of the active page, of another page by index, or of a single node's region,
capped so the longer side is at most `max_dimension` pixels (default 1024). Use
it to check your own work before telling the user it is done.

**`read_fnx_source`** — `{ path? }`. Omit `path` for the project `root` and the
list of its source files; pass a project-relative path (for example
`pages/<page-slug>/page.fnx`) for that file's text. Paths that escape the project
root are rejected. This tool only reads — write with your own file tools. On a
document that has never been saved it fails with *the document has no on-disk
Fanta project yet; save the canvas once to materialize one*.

### When no canvas is open

Every one of these tools fails with *no design canvas is open; open a .fig file
or Fanta project in Fanta first* unless a design canvas is focused in the app.
That is not something to work around — stop and ask the user to open the project
(or, if they cannot, edit the `.fnx` files directly and tell them the canvas will
pick the change up when they next open it).

## Saving, reloading, and committing

The two directions have different latencies, and mixing them up is the usual way
to commit a broken tree:

- **You edit a `.fnx`; the canvas follows.** The editor watches this directory
  and reloads roughly **300 ms** after a burst of external writes settles, so a
  multi-file rewrite lands as one reload rather than several. A parse error
  keeps the old canvas and surfaces the `line:column` error instead.
- **The canvas edits; the files follow.** Canvas edits — including everything
  `batch_design` does — are held in the editor's document until it saves, so
  treat the on-disk tree as *behind* the canvas until you have checked. Cmd-S
  saves immediately; the editor may also save on its own about a second after
  the last edit. Never assume either has happened.

`get_editor_state`'s `dirty` field is how you check: while it is `true` the
editor is still holding changes that have not reached disk, and re-reading the
`.fnx` will show you the state *before* those edits.

Saving rewrites the project tree, which also **regenerates the `.ids.json`
sidecars**. So after any edit — yours or the user's — let the editor save once
*before* you `git add`. Committing while `dirty` is `true` captures sources whose
sidecars, manifest timestamps and asset files do not match them.

## Finding the node the user means

When the user says "the selected frame" or "this button", call
`get_editor_state` and read `selection` — those are the node ids the canvas has
selected right now, and `batch_get` turns them into names, geometry and parents.
If the app is not running there is no live selection to read: ask the user for
the **layer name** (the `name="…"` attribute) or the page name and find it in
the relevant `page.fnx` / `master.fnx`. Names are not unique, so when several
elements share one, confirm the match by its parent frame, position or
surrounding text before editing.
"##;

/// `fanta.json` — the root manifest of a project directory.
///
/// Mirrors the `.fant` [`crate::Manifest`] in spirit (schema version, doc id,
/// app version, timestamps) but for the unzipped, git-native tree. No asset
/// index: assets are discovered by scanning `assets/**`, with the id encoded
/// in each filename — the directory *is* the index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectManifest {
    /// Always [`FORMAT_TAG`]. The presence of this tag is what
    /// [`is_project_dir`] checks.
    pub format: String,
    /// Project *layout* version ([`PROJECT_VERSION`]) — bumped when the
    /// directory shape changes, independently of the doc schema.
    pub version: u32,
    /// [`fanta_doc::DocId`] of the project's document, in display form
    /// (`d_<ULID>`).
    pub project_id: String,
    /// Doc schema version of the projected JSON, gating migrations on read.
    pub schema_version: u32,
    /// Fantaisa build that wrote the tree. Diagnostic only.
    pub app_version: String,
    /// Unix epoch seconds. Sourced from the doc's own metadata (not the wall
    /// clock) so identical input docs project to byte-identical trees.
    pub created_at: i64,
    /// Unix epoch seconds; same determinism rule as `created_at`.
    pub modified_at: i64,
}

impl ProjectManifest {
    /// Manifest for an existing doc. Timestamps come from `doc.metadata` so
    /// the projection is a pure function of the doc.
    pub fn for_doc(doc: &Doc) -> Self {
        Self {
            format: FORMAT_TAG.to_owned(),
            version: PROJECT_VERSION,
            project_id: doc.id.to_string(),
            schema_version: doc.schema_version,
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            created_at: doc.metadata.created_at,
            modified_at: doc.metadata.modified_at,
        }
    }

    /// Manifest for a brand-new, empty project (used by
    /// [`scaffold_project_tree`]). Mints a fresh [`DocId`] and stamps the
    /// current time.
    pub fn new_empty() -> Self {
        let now = unix_seconds_now();
        Self {
            format: FORMAT_TAG.to_owned(),
            version: PROJECT_VERSION,
            project_id: DocId::new().to_string(),
            schema_version: SCHEMA_VERSION,
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            created_at: now,
            modified_at: now,
        }
    }
}

/// Create the skeleton of an empty project at `dir`: the standard directories,
/// a `.gitignore`, and a fresh [`ProjectManifest`]. No git operations — repo
/// init is the app layer's job (spec 09 §A.1).
pub fn scaffold_project_tree(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir.join(DOC_DIR))?;
    fs::create_dir_all(dir.join(PAGES_DIR))?;
    fs::create_dir_all(dir.join(COMPONENTS_DIR))?;
    for media in super::media::MEDIA_DIRS {
        fs::create_dir_all(dir.join(ASSETS_DIR).join(media))?;
    }
    fs::create_dir_all(dir.join(PREVIEWS_DIR))?;
    fs::create_dir_all(dir.join(EXPORTS_DIR))?;
    fs::write(dir.join(GITIGNORE_NAME), GITIGNORE)?;
    seed_agents_md(dir)?;
    seed_fnx_types(dir)?;
    seed_fnx_prettier(dir)?;
    write_json_file(
        &dir.join(FANTA_JSON),
        &serde_json::to_value(ProjectManifest::new_empty())?,
    )
}

/// Seed editor support files that are missing from an existing project.
///
/// This validates the project manifest, then creates only the project-local
/// FNX declarations and default Prettier configuration. Existing support
/// files, custom Prettier configurations, the manifest, and all design files
/// are left untouched.
pub fn ensure_project_editor_support(dir: &Path) -> Result<()> {
    read_manifest(dir)?;
    seed_fnx_types(dir)?;
    seed_fnx_prettier(dir)?;
    seed_prettierignore(dir)?;
    // The agent guide is editor support too: it is the de-facto language spec
    // an AI agent reads before touching `.fnx`. Previously only snapshot
    // imports seeded it, so projects written by `write_project_tree` never
    // gained the guide at all.
    seed_agents_md(dir)
}

/// Write the [`AGENTS_MD`] guide to the project root, but only when no
/// `AGENTS.md` already exists. Unlike the regenerated `.gitignore`, this file
/// is a user-ownable seed: once present (whether from an earlier scaffold or
/// hand-authored) it is left untouched so customizations survive re-saves.
fn seed_agents_md(dir: &Path) -> Result<()> {
    let path = dir.join(AGENTS_MD_NAME);
    if !path.exists() {
        fs::write(path, AGENTS_MD)?;
    }
    Ok(())
}

fn seed_fnx_types(dir: &Path) -> Result<()> {
    let path = dir.join(FNX_TYPES_NAME);
    if !path.exists() {
        fs::write(path, FNX_TYPES)?;
    }
    Ok(())
}

/// The engine's printer is the one canonical formatter for `.fnx`: it emits
/// deterministic bytes, and the sidecar/mirror machinery patches that exact
/// text surgically. A host formatter (prettier, editor format-on-save)
/// rewriting generated source creates two competing canonical forms — every
/// open and save then rewrites the whole file, each side sees a foreign edit,
/// and change detection degenerates. Seed a `.prettierignore` so tooling that
/// honors it leaves `.fnx` alone; the `.prettierrc.json` overrides remain for
/// a user who deliberately deletes the ignore.
fn seed_prettierignore(dir: &Path) -> Result<()> {
    const PRETTIERIGNORE_NAME: &str = ".prettierignore";
    const PRETTIERIGNORE: &str =
        "# Generated Fanta design source: the engine printer is its canonical
# formatter; reformatting it fights the surgical source patcher.
*.fnx
";
    let path = dir.join(PRETTIERIGNORE_NAME);
    if !path.exists() {
        fs::write(path, PRETTIERIGNORE)?;
    }
    Ok(())
}

fn seed_fnx_prettier(dir: &Path) -> Result<()> {
    let existing_config = [
        ".prettierrc",
        ".prettierrc.json",
        ".prettierrc.json5",
        ".prettierrc.yml",
        ".prettierrc.yaml",
        ".prettierrc.js",
        ".prettierrc.cjs",
        ".prettierrc.mjs",
        "prettier.config.js",
        "prettier.config.cjs",
        "prettier.config.mjs",
        "prettier.config.ts",
        "prettier.config.cts",
        "prettier.config.mts",
    ]
    .into_iter()
    .any(|name| dir.join(name).exists());
    if !existing_config {
        fs::write(dir.join(FNX_PRETTIER_NAME), FNX_PRETTIER)?;
    }
    Ok(())
}

/// Whether `dir` looks like a Fantaisa project: a `fanta.json` exists, parses
/// as JSON, and carries the [`FORMAT_TAG`]. Never errors — any failure is
/// simply "not a project".
pub fn is_project_dir(dir: &Path) -> bool {
    let Ok(text) = fs::read_to_string(dir.join(FANTA_JSON)) else {
        return false;
    };
    serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v.get("format").and_then(Value::as_str).map(String::from))
        .is_some_and(|tag| tag == FORMAT_TAG)
}

/// Read and validate `fanta.json`. A missing, unreadable, or untagged manifest
/// is [`FormatError::NotAProject`]; a newer layout version is
/// [`FormatError::UnsupportedProjectVersion`].
pub(crate) fn read_manifest(dir: &Path) -> Result<ProjectManifest> {
    let not_a_project = || FormatError::NotAProject {
        path: dir.to_path_buf(),
    };
    let text = fs::read_to_string(dir.join(FANTA_JSON)).map_err(|_| not_a_project())?;
    let manifest: ProjectManifest = serde_json::from_str(&text).map_err(|_| not_a_project())?;
    if manifest.format != FORMAT_TAG {
        return Err(not_a_project());
    }
    if manifest.version > PROJECT_VERSION {
        return Err(FormatError::UnsupportedProjectVersion {
            found: manifest.version,
            supported: PROJECT_VERSION,
        });
    }
    Ok(manifest)
}

/// The canonical byte form of a projected JSON file: pretty-printed with a
/// trailing newline. Pretty + newline keeps the files diff- and `cat`-friendly
/// and is part of the byte-determinism contract; every JSON file the writer
/// projects goes through this one function.
pub(crate) fn json_bytes(value: &Value) -> Result<Vec<u8>> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    Ok(text.into_bytes())
}

/// Write `value` in the [`json_bytes`] form, creating parent directories.
pub(crate) fn write_json_file(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, json_bytes(value)?)?;
    Ok(())
}

/// Read a JSON file. A missing file maps to [`FormatError::MissingFile`] with
/// the full path so the error names the culprit.
pub(crate) fn read_json_file(path: &Path) -> Result<Value> {
    let text = fs::read_to_string(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            FormatError::MissingFile {
                name: path.display().to_string(),
            }
        } else {
            FormatError::Io(e)
        }
    })?;
    Ok(serde_json::from_str(&text)?)
}

/// Read a JSON file, substituting `fallback` when the file doesn't exist.
/// Used for the doc singletons whose absence means "default".
pub(crate) fn read_json_or(path: &Path, fallback: Value) -> Result<Value> {
    if path.exists() {
        read_json_file(path)
    } else {
        Ok(fallback)
    }
}

/// Longest slug [`slugify`] produces. Long enough for real page names, short
/// enough that nested project paths stay comfortably under OS path limits.
pub(crate) const MAX_SLUG_LEN: usize = 48;

/// Project a design name onto a directory-safe slug: lowercase, ASCII
/// alphanumerics kept, every other run of characters collapsed to a single
/// `-`, trimmed at both ends, capped at [`MAX_SLUG_LEN`]. A name with nothing
/// usable (empty, all emoji, …) becomes `fallback`.
pub(crate) fn slugify(name: &str, fallback: &str) -> String {
    let mut slug = String::new();
    let mut pending_separator = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            if pending_separator && !slug.is_empty() {
                slug.push('-');
            }
            pending_separator = false;
            slug.push(character.to_ascii_lowercase());
        } else {
            pending_separator = true;
        }
    }
    slug.truncate(MAX_SLUG_LEN);
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        fallback.to_owned()
    } else {
        slug
    }
}

/// The serde JSON form of an id — the bare 26-char ULID string (ids serialize
/// `#[serde(transparent)]`, without the `n_`/`c_` display prefix). This is the
/// key shape used inside `scene.nodes` and the component maps, as opposed to
/// the prefixed `Display` form used for file and directory names.
pub(crate) fn json_key<T: Serialize>(id: &T) -> Result<String> {
    match serde_json::to_value(id)? {
        Value::String(s) => Ok(s),
        other => Err(FormatError::InvalidProjectTree(format!(
            "id did not serialize to a string: {other}"
        ))),
    }
}

/// Inverse of [`json_key`]: parse a bare ULID map key back into a typed id.
pub(crate) fn id_from_key<T: serde::de::DeserializeOwned>(key: &str) -> Result<T> {
    serde_json::from_value(Value::String(key.to_owned()))
        .map_err(|e| FormatError::InvalidProjectTree(format!("invalid id key {key:?}: {e}")))
}

/// Directory entries sorted by path — deterministic iteration for readers.
pub(crate) fn sorted_entries(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        out.push(entry?.path());
    }
    out.sort();
    Ok(out)
}

fn unix_seconds_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn scaffold_creates_skeleton_and_manifest() {
        let dir = tempdir().unwrap();
        scaffold_project_tree(dir.path()).unwrap();
        for sub in [
            DOC_DIR,
            PAGES_DIR,
            COMPONENTS_DIR,
            PREVIEWS_DIR,
            EXPORTS_DIR,
        ] {
            assert!(dir.path().join(sub).is_dir(), "missing {sub}");
        }
        for media in super::super::media::MEDIA_DIRS {
            assert!(dir.path().join(ASSETS_DIR).join(media).is_dir());
        }
        assert_eq!(
            fs::read_to_string(dir.path().join(GITIGNORE_NAME)).unwrap(),
            GITIGNORE
        );
        let manifest = read_manifest(dir.path()).unwrap();
        assert_eq!(manifest.format, FORMAT_TAG);
        assert_eq!(manifest.version, PROJECT_VERSION);
        assert_eq!(manifest.schema_version, fanta_doc::SCHEMA_VERSION);
        assert!(manifest.project_id.starts_with("d_"));
    }

    #[test]
    fn scaffold_seeds_agents_md_but_never_overwrites_a_user_copy() {
        // A fresh project gets the AGENTS.md guide, recognizable by its marker.
        let fresh = tempdir().unwrap();
        scaffold_project_tree(fresh.path()).unwrap();
        let seeded = fs::read_to_string(fresh.path().join(AGENTS_MD_NAME)).unwrap();
        assert!(
            seeded.contains("# Working on this Fanta design project"),
            "seeded AGENTS.md is missing its marker heading"
        );
        assert_eq!(seeded, AGENTS_MD, "fresh scaffold writes the template seed");

        // A pre-existing AGENTS.md is a user document — scaffolding must leave
        // it byte-for-byte intact (this covers re-saves of an existing project,
        // whose save path scaffolds before writing).
        let customized = tempdir().unwrap();
        let custom = "# My own notes\n\nDon't touch this.\n";
        fs::write(customized.path().join(AGENTS_MD_NAME), custom).unwrap();
        scaffold_project_tree(customized.path()).unwrap();
        assert_eq!(
            fs::read_to_string(customized.path().join(AGENTS_MD_NAME)).unwrap(),
            custom,
            "an existing AGENTS.md must be preserved"
        );
    }

    #[test]
    fn scaffold_seeds_fnx_types_but_preserves_project_extensions() {
        let project = tempdir().unwrap();
        scaffold_project_tree(project.path()).unwrap();
        assert!(FNX_TYPES.contains("type FnxFill ="));
        assert!(FNX_TYPES.contains("interface FnxTextStyle"));
        assert!(FNX_TYPES.contains("interface FnxPathData"));
        assert!(FNX_TYPES.contains("export function Text"));
        assert_eq!(
            fs::read_to_string(project.path().join(FNX_TYPES_NAME)).unwrap(),
            FNX_TYPES
        );

        let custom = format!("{FNX_TYPES}\ninterface ProjectTokens {{ brand: FnxColor }}\n");
        fs::write(project.path().join(FNX_TYPES_NAME), &custom).unwrap();
        scaffold_project_tree(project.path()).unwrap();
        assert_eq!(
            fs::read_to_string(project.path().join(FNX_TYPES_NAME)).unwrap(),
            custom
        );
    }

    #[test]
    fn scaffold_seeds_fnx_prettier_defaults_without_overriding_user_config() {
        let project = tempdir().unwrap();
        scaffold_project_tree(project.path()).unwrap();
        assert_eq!(
            fs::read_to_string(project.path().join(FNX_PRETTIER_NAME)).unwrap(),
            FNX_PRETTIER
        );

        let custom_project = tempdir().unwrap();
        let custom = "export default { singleQuote: true };\n";
        fs::write(custom_project.path().join("prettier.config.mjs"), custom).unwrap();
        scaffold_project_tree(custom_project.path()).unwrap();
        assert!(!custom_project.path().join(FNX_PRETTIER_NAME).exists());
        assert_eq!(
            fs::read_to_string(custom_project.path().join("prettier.config.mjs")).unwrap(),
            custom
        );
    }

    #[test]
    fn editor_support_can_be_seeded_without_touching_project_data() {
        let project = tempdir().unwrap();
        scaffold_project_tree(project.path()).unwrap();
        fs::remove_file(project.path().join(FNX_TYPES_NAME)).unwrap();
        fs::remove_file(project.path().join(FNX_PRETTIER_NAME)).unwrap();

        let design_dir = project.path().join(PAGES_DIR).join("existing-page");
        fs::create_dir_all(&design_dir).unwrap();
        let design_path = design_dir.join(PAGE_FNX);
        let design = b"// existing design bytes\n<Frame name=\"Keep me\" />\n";
        fs::write(&design_path, design).unwrap();
        let manifest_before = fs::read(project.path().join(FANTA_JSON)).unwrap();

        ensure_project_editor_support(project.path()).unwrap();

        assert_eq!(
            fs::read(project.path().join(FANTA_JSON)).unwrap(),
            manifest_before
        );
        assert_eq!(fs::read(design_path).unwrap(), design);
        assert_eq!(
            fs::read_to_string(project.path().join(FNX_TYPES_NAME)).unwrap(),
            FNX_TYPES
        );
        assert_eq!(
            fs::read_to_string(project.path().join(FNX_PRETTIER_NAME)).unwrap(),
            FNX_PRETTIER
        );
    }

    #[test]
    fn editor_support_preserves_custom_types_and_prettier_configuration() {
        let project = tempdir().unwrap();
        scaffold_project_tree(project.path()).unwrap();
        fs::remove_file(project.path().join(FNX_PRETTIER_NAME)).unwrap();
        let custom_types = "export {};\ndeclare global { interface ProjectOnly {} }\n";
        let custom_prettier = "export default { printWidth: 72 };\n";
        fs::write(project.path().join(FNX_TYPES_NAME), custom_types).unwrap();
        fs::write(project.path().join("prettier.config.mjs"), custom_prettier).unwrap();

        ensure_project_editor_support(project.path()).unwrap();
        ensure_project_editor_support(project.path()).unwrap();

        assert_eq!(
            fs::read_to_string(project.path().join(FNX_TYPES_NAME)).unwrap(),
            custom_types
        );
        assert_eq!(
            fs::read_to_string(project.path().join("prettier.config.mjs")).unwrap(),
            custom_prettier
        );
        assert!(!project.path().join(FNX_PRETTIER_NAME).exists());
    }

    #[test]
    fn editor_support_rejects_non_projects_without_writing_files() {
        let directory = tempdir().unwrap();
        let error = ensure_project_editor_support(directory.path()).unwrap_err();
        assert!(matches!(error, FormatError::NotAProject { .. }));
        assert!(fs::read_dir(directory.path()).unwrap().next().is_none());
    }

    #[test]
    fn is_project_dir_true_after_scaffold_false_otherwise() {
        let project = tempdir().unwrap();
        scaffold_project_tree(project.path()).unwrap();
        assert!(is_project_dir(project.path()));

        let empty = tempdir().unwrap();
        assert!(!is_project_dir(empty.path()));

        // A fanta.json with the wrong format tag is not a project.
        let wrong = tempdir().unwrap();
        fs::write(wrong.path().join(FANTA_JSON), r#"{"format":"zip"}"#).unwrap();
        assert!(!is_project_dir(wrong.path()));

        // Garbled JSON is not a project either.
        let garbled = tempdir().unwrap();
        fs::write(garbled.path().join(FANTA_JSON), "{nope").unwrap();
        assert!(!is_project_dir(garbled.path()));
    }

    #[test]
    fn newer_project_version_is_rejected() {
        let dir = tempdir().unwrap();
        scaffold_project_tree(dir.path()).unwrap();
        let mut v: Value =
            serde_json::from_str(&fs::read_to_string(dir.path().join(FANTA_JSON)).unwrap())
                .unwrap();
        v["version"] = Value::from(PROJECT_VERSION + 1);
        write_json_file(&dir.path().join(FANTA_JSON), &v).unwrap();
        let err = read_manifest(dir.path()).unwrap_err();
        assert!(matches!(
            err,
            FormatError::UnsupportedProjectVersion { found, supported }
                if found == PROJECT_VERSION + 1 && supported == PROJECT_VERSION
        ));
    }

    #[test]
    fn slugify_keeps_ascii_alphanumerics_and_collapses_the_rest() {
        assert_eq!(slugify("Page 1", "page"), "page-1");
        assert_eq!(slugify("Home Screen", "page"), "home-screen");
        assert_eq!(slugify("  --  Hello,   World!  ", "page"), "hello-world");
        assert_eq!(slugify("UPPER lower 42", "page"), "upper-lower-42");
    }

    #[test]
    fn slugify_drops_non_ascii_and_emoji() {
        assert_eq!(slugify("Página Café", "page"), "p-gina-caf");
        assert_eq!(slugify("☕ Break ☕", "page"), "break");
        assert_eq!(slugify("日本語", "page"), "page");
        assert_eq!(slugify("🎨🎨🎨", "component"), "component");
    }

    #[test]
    fn slugify_empty_and_symbol_only_names_use_the_fallback() {
        assert_eq!(slugify("", "page"), "page");
        assert_eq!(slugify("   ", "page"), "page");
        assert_eq!(slugify("!!!///???", "component"), "component");
    }

    #[test]
    fn slugify_caps_length_without_a_trailing_dash() {
        let long = "a".repeat(80);
        assert_eq!(slugify(&long, "page"), "a".repeat(MAX_SLUG_LEN));
        // Truncation landing on a separator must not leave a trailing dash.
        let boundary = format!("{} {}", "a".repeat(MAX_SLUG_LEN - 1), "b".repeat(20));
        let slug = slugify(&boundary, "page");
        assert_eq!(slug, "a".repeat(MAX_SLUG_LEN - 1));
        assert!(!slug.ends_with('-'));
        assert!(slug.len() <= MAX_SLUG_LEN);
    }

    #[test]
    fn slugify_is_deterministic() {
        for name in ["Page 1", "Página Café", "🎨", ""] {
            assert_eq!(slugify(name, "page"), slugify(name, "page"));
        }
    }

    #[test]
    fn json_key_is_bare_ulid_and_round_trips() {
        let id = fanta_doc::NodeId::new();
        let key = json_key(&id).unwrap();
        assert_eq!(key.len(), 26, "bare ULID, no prefix");
        assert_eq!(id.to_string(), format!("n_{key}"));
        let back: fanta_doc::NodeId = id_from_key(&key).unwrap();
        assert_eq!(back, id);
    }
}
