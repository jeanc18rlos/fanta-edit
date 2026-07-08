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
pub(crate) const PROJECT_VERSION: u32 = 2;

pub(crate) const DOC_DIR: &str = "doc";
pub(crate) const PAGES_DIR: &str = "pages";
pub(crate) const COMPONENTS_DIR: &str = "components";
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
/// Page header file: `{ name?, order }`.
pub(crate) const PAGE_JSON: &str = "page.json";
/// Component master file: the `ComponentDef` JSON.
pub(crate) const DEF_JSON: &str = "def.json";
/// Component-set registry (small, rarely edited concurrently).
pub(crate) const SETS_JSON: &str = "sets.json";
/// Pseudo-page directory for orphan nodes — nodes whose parent chain reaches
/// neither a page root nor a component root. Underscore-prefixed so it can
/// never collide with an id-named page directory (`n_…`).
pub(crate) const LOOSE_DIR: &str = "_loose";

pub(crate) const METADATA_JSON: &str = "metadata.json";
pub(crate) const VARIABLES_JSON: &str = "variables.json";
pub(crate) const ACTIVE_MODES_JSON: &str = "active_modes.json";
pub(crate) const FLOW_START_JSON: &str = "flow_start.json";

pub(crate) const GITIGNORE_NAME: &str = ".gitignore";
/// Derived/transient directories and OS noise stay out of history.
pub(crate) const GITIGNORE: &str = "previews/\nexports/\n.DS_Store\n";

/// Agent-guide file at the project root. Zed's agent auto-loads it as a rules
/// file (`RULES_FILE_NAMES` in `prompt_store`), so seeding it teaches any AI
/// agent working the project how the format round-trips.
pub(crate) const AGENTS_MD_NAME: &str = "AGENTS.md";

/// The seed contents of [`AGENTS_MD_NAME`]. Written only when absent (unlike
/// the always-regenerated `.gitignore`) so a user's edits are never clobbered.
/// The guide is intentionally practical and format-accurate — it is derived
/// from the real `.fnx` projection, not invented.
pub(crate) const AGENTS_MD: &str = r#"# Working on this Fanta design project

This directory is a Fanta *design project*: a Figma-class design stored as
editable source files. **The design is the source of truth.** Editing these
files edits the design. When this project is open in the editor, saving your
edits to the `.fnx` files live-reloads the canvas, and edits made on the canvas
save back to these same files. So working here as a text agent *is* the
design-to-canvas loop — no export step.

## Directory layout

```
fanta.json                       # project manifest (format tag, versions, ids) — do not hand-edit
doc/
  metadata.json                  # title + timestamps
  variables.json                 # design variables / tokens and their modes
  active_modes.json              # which mode is active per variable collection
  flow_start.json                # prototype start page (or null)
pages/<NodeId>/
  page.json                      # { "name": ..., "order": N }
  page.fnx                       # the page's node tree as readable source  <- edit this
  page.ids.json                  # id + sibling-order sidecar for page.fnx  — do not hand-edit
components/<ComponentId>/
  def.json                       # component definition (id, root, name)
  master.fnx                     # the component master's tree as source    <- edit this
  master.ids.json                # id sidecar for master.fnx                — do not hand-edit
components/sets.json             # component-set (variant) registry
assets/<family>/<AssetId>.<ext>  # shared binary assets, one folder per family:
                                 #   images/ video/ audio/ models/ svg/ fonts/ other/
previews/  exports/              # generated output — git-ignored, never an input
```

Directory and file names under `pages/` and `components/` are node/component
ids (`n_…`, `c_…`, `a_…`). Ids are the truth; the folder names are just a
projection of them.

## The `.fnx` language

A `.fnx` file is the readable, JSX/TSX-style projection of one page or component
subtree. It opens with `// @generated fanta source …` — treat it as generated
source you may edit, not as a file to reformat. Each scene node is one JSX
element:

- The **tag** is the node kind: `Frame` (group/frame), `Vector` (shape),
  `Text`, `Image`, `Video`, `Audio`, `Model3D`, `NodeGraph`, `AiArtifact`,
  `Instance` (a component instance), `Embed`.
- **Attributes** are the node's fields, verbatim. String attributes render as
  `name="Header"`; everything else renders inside braces, e.g. `opacity={1.0}`,
  `x={280.0}`, `corner_radius={2.0}`. Nesting is by hierarchy: an element's
  children are its child nodes, in render (z) order.
- **Colors** render as hex literals inside the braces: `#RRGGBB`, or `#RRGGBBAA`
  when not fully opaque (e.g. `#FFFFFF1A`).

A short real snippet:

```jsx
<Frame background={{"kind": "solid", "color": #444444}} blend_mode="normal"
       corner_radius={2.0} name="Header" opacity={1.0} x={-734.0} y={-491.0}>
  <Text align="left" content="Little Lemon" name="Title" opacity={1.0}
        style={{"font_family": "Inter", "size_px": 64.0, "weight": 700, "color": #1E1E1E}}
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
change `clip_size` / `local_size` context and the app to resize. A node with
rotation, scale, or skew shows a raw `transform={[a, b, c, d, tx, ty]}` array
instead of `x`/`y` — leave that verbatim unless you mean to change the matrix.

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
  Text `style.color`.
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
- `previews/` and `exports/` are generated; they are never read back. To
  regenerate or preview, save from the editor (or reopen the project).

## Selection context

There is no live "current selection" file to read. When the user wants you to
act on a specific node or frame ("the selected frame", "this button"), ask them
to paste its **layer name** (the `name="…"` attribute) — or the page name — and
find it in the relevant `page.fnx` / `master.fnx`. Names are not guaranteed
unique, so confirm the match (parent frame, position, or surrounding text) if
several elements share a name.
"#;

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
    write_json_file(
        &dir.join(FANTA_JSON),
        &serde_json::to_value(ProjectManifest::new_empty())?,
    )
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

/// Write `value` pretty-printed with a trailing newline, creating parent
/// directories. Pretty + newline keeps the files diff- and `cat`-friendly and
/// is part of the byte-determinism contract.
pub(crate) fn write_json_file(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    fs::write(path, text)?;
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
    fn json_key_is_bare_ulid_and_round_trips() {
        let id = fanta_doc::NodeId::new();
        let key = json_key(&id).unwrap();
        assert_eq!(key.len(), 26, "bare ULID, no prefix");
        assert_eq!(id.to_string(), format!("n_{key}"));
        let back: fanta_doc::NodeId = id_from_key(&key).unwrap();
        assert_eq!(back, id);
    }
}
