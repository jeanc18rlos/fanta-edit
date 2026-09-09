# Known issues, Fanta 0.1.0-alpha.1

Written from the code and from the alpha rehearsal, not from wishful thinking.
Every entry is something a tester can hit on purpose. Where a claim comes from
reading source rather than from running the app, it says so. Re-checked entry by
entry against the working tree of `perf/large-documents-ai-alignment` — the
uncommitted work on top of `16309b2` — by grepping for the thing each one
claims; entries that work closed have been deleted or rewritten rather than left
standing, and entries that only a human at the keyboard could confirm are
labelled as source readings.

A release build was made from `16309b2` and driven afterwards, so the entries
about the shipped CLI, the MCP loop and memory are measurements rather than
readings — **of that build**. No binary has been built from this tree, and this
tree changed five things those measurements ran through: the autosave write
path (`Doc::clone_for_persist`, `fanta_format::ProjectWriteCache`), image
decoding (`fanta_render::asset::LazyAssetResolver`), how a drag reaches the
render thread (`ScenePatch` in `crates/fig_viewer/src/canvas.rs`), the layers
tree (`layers_tree` built over the expanded set only) and the `.fig` import
pipeline (`crates/fanta-fig-interop`). Every number below that touches one of
those was measured before the change and has **not** been re-measured; each
such entry says so. Everything reachable only by a mouse click is still a source
reading: nobody has clicked a menu in any build of this tree.

[`REHEARSAL.md`](REHEARSAL.md) is the committed record of what was and was not
actually driven. Nothing here claims more than that record supports.

## First launch

- **The DMG is ad-hoc signed**, so macOS refuses to open it until you go through
  System Settings > Privacy & Security > **Open Anyway**, or run
  `xattr -dr com.apple.quarantine /Applications/Fanta.app`. The full sequence is
  in [`RELEASE_NOTES.md`](RELEASE_NOTES.md). This is not a bug you can work
  around by re-downloading.
- **A folder that is not a design project still raises a Restricted Mode
  prompt.** Design projects normally do not: `auto_trust_design_projects`
  (`crates/zed/src/zed.rs:453`) trusts a lone `.fig`/`.fnx` file, and trusts a
  directory only when **all** of these hold — it has `fanta.json` at its root,
  it has no `.zed` directory, and `git_repository_cannot_execute_anything`
  finds no `.git` at all, **or** a `.git` that is a genuine directory with no
  hooks beyond git's own `.sample` files and a config every line of which is on
  a short **allowlist** of keys known to be inert (`git_setting_is_inert`:
  a handful of `core`, `remote`, `branch`, `pull`, `push`, `fetch`, `gc`,
  `init`, `user` and `submodule` keys, with `remote.url` additionally rejected
  when it starts `ext::`). It is an allowlist rather than a list of dangerous
  keys because the dangerous set is not even bounded by the file — `include.path`
  and `includeIf` pull settings in from anywhere else on disk. So an ordinary
  config key that nobody thought to list, an include directive, or any line that
  is not a plain `key = value` all mean "not understood", and the user is asked.
  Every other uncertainty — an unreadable config, a `.git` file or symlink
  pointing elsewhere, an unexpected hook — falls back to asking too.
  Open anything else — a plain git checkout, a downloads folder — and you get
  the inherited blocking *"Unrecognized Project"* dialog, in Zed's wording,
  which **cannot be dismissed with Escape**; the title bar then reads
  *Restricted Mode* for the rest of the session.
  `"trust_all_worktrees"` is deliberately still `false`
  (`assets/settings/default.json:2565`): a worktree's own `.zed/settings.json`
  can name a context-server command that this build will spawn, and trusting a
  worktree also un-suppresses git hooks, `diff.external` and credential helpers.
  Flipping it globally would have been a real security regression, so the guard
  is narrow on purpose.
- **A design project you received as a zip or a copied folder can still prompt,
  and so can one whose git config you customised.** That is the guard above
  doing its job, not a bug: a copied `.git` carries its own hooks and config,
  and those run commands. A plain `git init` or `git clone` config is entirely
  on the allowlist, so the common cases are silent — but a config key the
  allowlist does not name, however harmless, is treated as "not understood" and
  you get the prompt. If a shared project prompts, either answer the prompt
  yourself or delete the copied `.git` and re-`git init` it.
- **Open Project Settings is hidden from the command palette on purpose.**
  `zed::OpenProjectSettingsFile` and `zed_actions::OpenProjectSettings` are in
  `hide_action_types` (`crates/zed/src/main.rs`) because both write a
  `.zed/settings.json` into the open design, which permanently disqualifies it
  from the auto-trust above and leaves you facing the unescapable prompt on
  every later open. There is no in-app way to set project-local settings for a
  design; use the user settings file.
- Launching a second `fanta <dir>` while the app is running prints
  `Fanta is already running` and exits without opening the project
  (`crates/zed/src/main.rs:390`; the single-instance check is live because
  `crates/zed/RELEASE_CHANNEL` is `stable`). Open the folder from the running
  app instead.
- **`fanta` with no arguments restores your last session, so you may never see
  the welcome page again.** The rehearsal hit this: after a project had been
  opened once, a bare launch reopened it rather than showing
  *"Your design is code."* and its **Connect Claude Code / Codex** entry. If you
  are looking for the Copy command / Copy Codex config buttons, use
  **File > New Window**. The welcome page has never been confirmed on screen at
  all — see [`REHEARSAL.md`](REHEARSAL.md) row 1 — so its rendered layout is a
  source reading only.

## Canvas and tools

- **Unwired faces stay visible and tell you so when clicked.** The toolbar and
  the layers/pages panels come from the vendored `fanta-gpui` crate, which
  exposes no host-side API to hide or disable a control — the comment above the
  `tool_kind` match in `view.rs::handle_toolbar_action` records this
  ("the vendored toolbar has no host-side API to hide a tool"). Hiding them would
  have meant patching a vendored crate on the eve of the alpha, so that was
  **deliberately deferred**; instead
  the host declines the action and raises a notice reading
  *"&lt;control&gt; is not available in the Fanta alpha yet."*
  (`view.rs::notify_unavailable`). You will meet it on the **Scale**, **Path
  Selection** and **Text-on-Path** tools (they have a tool object but no
  behaviour, so arming them would silently swallow every drag), on **Dev mode**,
  on the toolbar's **file-attach** and **voice** buttons, on **Remove
  background** and **Generate an image** in the toolbar's AI menu (there is no
  image backend to hand them to — `toolbar_agent_prompt_template` in `view.rs`
  returns `None` for both), on **auto-keyframe recording**, on **"mark ready
  for dev"**, on **time-anchored comments**, on **page duplication** and **page
  links** (`design_panel.rs`, `PagesPanelAction::DuplicateRequested` /
  `CopyLinkRequested`), and on a number of inspector controls.
- **The other five toolbar AI commands open a draft in the Agent Panel; they do
  not run anything themselves.** *Generate a design*, *Replace content*,
  *Rewrite text*, *Translate text* and *Rename layers* each put a prompt
  template into a fresh Agent Panel draft — with blanks such as
  `<describe the screen>` you are meant to fill in — and so does anything typed
  into the toolbar's own AI box (`toolbar_agent_prompt_template` and
  `route_toolbar_agent_prompt`, `view.rs`). The draft is prefixed with the page
  name, the selection count and up to eight selected layers as
  `- <name> (<kind>, id <node id>, <w>x<h> at <x>,<y>)` (`agent_prompt_context`),
  and nothing is sent until you press send: `agent_ui::open_external_prompt_for_review`
  demands the manual send so a toolbar button cannot bypass the panel's review
  boundary. Without a working agent configuration (see Agents) you get a draft
  you cannot send anywhere. Source reading — no toolbar button has been clicked
  in a build of this tree.
- **Group, Frame selection and Ungroup now exist, and nobody has pressed the
  keys yet.** `cmd-g`, `cmd-alt-g` and `cmd-shift-g` are bound to
  `fig_viewer::GroupSelection` / `FrameSelection` / `UngroupSelection` in both
  the `FigViewer && !Editor` and `FantaDesignPanel && !Editor` contexts
  (`assets/keymaps/default-macos.json`), the toolbar's *Group selection* /
  *Ungroup selection* / *Frame selection* commands route to the same handlers
  (`view.rs::handle_toolbar_action`, `ToolbarCommand::Group` and friends), and so
  do the layers context menu's entries (`design_panel.rs::handle_layers_context_action`,
  which acts on the selection when the clicked row is part of it and on that
  row alone otherwise — `view.rs::structure_targets`). The builders live in
  `crates/fig_viewer/src/structure.rs`. What to expect, from reading them:
  the group is created where the topmost member lives and takes its slot in the
  z-order; members from other parents move into it keeping their world
  position; *Frame selection* makes the same wrapper but clipping to the union
  of the members' bounds, with no background; *Ungroup* moves the children into
  the group's slot and deletes the wrapper, and dissolves every top-level group
  in the selection in one step. Each is one undo step. Refusals are shown as a
  canvas notice rather than swallowed (`view.rs::apply_structure_edit`): a page
  or a component master in the selection (*"a page cannot be grouped"*,
  *"… is a component master and cannot be grouped"*), ungrouping an instance
  (*"an instance cannot be ungrouped; detach it first"*), a component master
  (*"… detach or delete the component instead"*), or something that is not a
  group (*"select a group or frame to ungroup"*). Unit-tested in
  `structure.rs`; not driven in the app.
- Grid auto layout is offered in the inspector and the engine ignores it: the
  adapter logs a line and returns no operations at all
  (`gpui_adapters/design.rs::layout_mode_operations`). Horizontal and vertical
  auto layout work.
- Pass Through, Linear Burn and Linear Dodge are listed as blend modes and do
  nothing; the engine has no equivalent, so the adapter emits no operation
  (`gpui_adapters/design.rs`). The other blend modes map straight through.
- Effects are limited to drop shadow, inner shadow, layer blur and background
  blur. Every other effect kind is reported as unsupported in the inspector.
- **Select more than 512 layers and the per-layer outlines disappear.** Above
  `SELECTION_OUTLINE_CAP` (`crates/fig_viewer/src/canvas.rs`),
  `collect_overlay_data` keeps only the union of the selection — one outline,
  the handles and the size badge — and skips the per-node outlines and text
  baselines. `cmd-a` on an imported page is the easy way to hit it. Deliberate
  (thousands of outlines are unreadable and cost a quad each per frame), and a
  source reading.
- **The layers panel only builds the rows it can show.** `layers_tree`
  (`crates/fig_viewer/src/gpui_adapters/layers.rs`) emits a page's top level plus
  the children of *expanded* containers; a collapsed container gets a
  `has_children` hint so it still draws its disclosure arrow, and expanding it
  asks the host for its children (`design_panel.rs::refresh_gpui_layers`,
  keyed on `LayersTreeKey { page_root, render_generation, expansion_generation }`).
  If a container ever shows an arrow but no children after expanding, or a
  reveal-from-canvas lands on a row that is not there, this is the code to
  suspect. Covered by unit and harness tests; not driven on screen.
- **A drag no longer copies the whole scene to the render thread on every
  frame.** The macOS canvas keeps a copy of the scene on its render thread and,
  when the document's change log can prove which nodes moved or changed since
  that copy (`Scene::changes_since`, `crates/fanta-doc/src/scene/graph.rs`;
  `plan_snapshot_sync` in `canvas.rs`), sends only those nodes as a
  `ScenePatch`. Anything the log cannot prove — a structural edit, a component,
  variable or asset change, more than 2,048 touched nodes — falls back to a full
  copy, so correctness never rides on the patch. If applying a patch fails the
  render thread drops its copy, logs `the canvas render thread dropped its scene
  copy: …` at WARN, and the next paint sends the whole scene again; it is not
  counted as a render failure. Unit-tested for parity; no frame of this tree has
  been rendered.

## Editing and files

- **Canvas copy and paste works inside one document only.** A cross-document
  paste is refused with a message — *"cross-document canvas paste is not
  supported yet; assets, components, and variables were left unchanged"*
  (`clipboard.rs:144`).
- **Pasting an image from another app now places it; pasting any other file
  still does nothing and says nothing.** `paste_selected_nodes` (`view.rs`)
  tries a canvas payload first, then `ClipboardEntry::Image` bytes, then
  `ClipboardEntry::ExternalPaths` filtered to `png`, `jpg`, `jpeg`, `webp`,
  `gif`, `bmp`, `tiff` and `tif` (`is_pastable_image_path`); paths with any
  other extension are dropped without a notice. Each image is ingested as a
  project asset (`AssetStores::add_image`; it becomes a file under
  `assets/images/` on the next save) and placed as a bitmap layer on the active
  page, centred on the viewport at its natural size and shrunk to fit 80% of the
  visible area when larger, with a 16 px offset per additional image
  (`paste_image`, `structure::image_layer_node`). Each image is its own
  *"Paste image"* undo step, so one corrupt file does not undo its neighbours;
  a failure raises the notice *"Pasting an image failed: …"* (or *"Pasting an
  image file failed: …"* for an unreadable path) and removes the asset again.
  The result is selected. Source reading: no image has been pasted into a build
  of this tree.
- **A `.fig` with no project folder does not autosave.** Autosave requires a
  project root — `autosave_allowed` checks `item.project_root().is_some()` —
  because the write that creates one scaffolds a whole directory next to the
  file, and a timer should not do that behind your back. In normal use you will
  not meet this: opening a `.fig` **materialises the project directory on open**
  (`document.rs`, the `None =>` arm of the load match), so the editor is
  project-backed from the first frame. You only fall into the in-memory mode
  when that write **fails** — a full disk, a read-only folder — which logs
  `materializing Fanta project at … on open failed` and removes the partial
  directory. If you see a `.fig` whose edits are not reaching disk, check the
  log for that line, then press `cmd-s` to retry the materialisation.
- **Every save touches three files, not one.** A one-node change shows up as
  `M pages/…/page.fnx`, `M pages/…/page.ids.json` and `M doc/metadata.json`.
  The third is the doc's own `modified_at`, which the 3-way merge tie-breaks on,
  so it stays. `fanta.json` no longer carries a per-save timestamp and has
  dropped out of the diff; a project written by an older build loses that line
  from `fanta.json` the first time it is saved — one-time, and it still opens.
  The write path behind this changed on this branch: `FigItem::save`
  (`document.rs`) now persists `Doc::clone_for_persist()` — a copy without the
  undo history and selection, which never reached disk anyway — through
  `fanta_format::write_project_tree_cached`, which fingerprints each page and
  component and reuses the bytes it produced last time when nothing in that
  design changed (`ProjectWriteCache`, `crates/fanta-format/src/project/write.rs`).
  The module contract is that the bytes are exactly what a cold projection
  yields and every file still goes through the same write-if-changed and prune
  steps, and the crate's unit and integration tests (`tests/write_cache.rs`)
  compare cached and cold trees byte for byte. The three-file diff above was
  measured on the previous build; nobody has diffed a save from this one.
- **A rectangle gains one attribute the first time a project is reloaded.**
  The loader backfills a vector's SVG viewport, so `<Rect width height />`
  becomes `<Rect width height local_size={[w, h]} />` on the first save after a
  reopen — one line per rectangle, once, and it stays a readable `<Rect>`
  thereafter. (A viewport that genuinely crops the shape has no `<Rect>`
  spelling and still prints as a canonical `<Vector>`; that is correct, not a
  regression.)
- Export runs in the inspector and needs a project root, so the project has to
  have been written at least once. Files land in `<project>/exports/`
  (`export.rs:260`), in PNG, JPG, SVG or PDF (`ExportFormat`). The toolbar's
  Export command is **not** a second flow: it forces the inspector sidebar open
  and defers to the same `export_selection`
  (`view.rs::export_from_toolbar`), because every failure it can hit — unsaved
  project, unexportable bounds — is reported as inspector feedback and would
  otherwise look like a click that did nothing.
- The Code tab is read-only by design. Edit `.fnx` in your own editor and the
  canvas follows the file. Note that with autosave on, `page.fnx`'s mtime moves
  about a second after any canvas edit — **an unchanged mtime is no longer a
  useful check for anything**, and [`SMOKE.md`](SMOKE.md) has been corrected
  accordingly.
- A design tab is restored on relaunch through its project folder; that was
  verified in the rehearsal. Restoring a `.fig` that never materialised a
  project was **not** verified — if it does not come back, reopen it from
  File > Open Recent.
- **Opening a very large community `.fig` (100 MB and up) is slow, and the
  only number we have is from the previous build.** A 9.6 MB file took roughly
  30–40 s to reach a rendered canvas on a debug build of `16309b2`'s
  predecessor; the 128 MB case in the smoke checklist has never been run. This
  branch reworked the import crate — `byte[]` fields decode to one
  `KiwiValue::Bytes` instead of one value per byte (`kiwi/value.rs`), the
  shared-style pre-pass no longer deep-clones every node change
  (`fields.rs::resolve_style_references` returns `Cow`s), master paths are
  built once per master (`instance_overrides/paths.rs::MasterPathCache`), and
  per-instance side tables are borrowed rather than copied
  (`orchestrator.rs`) — and it changed what happens after import (images are
  no longer decoded up front; see below). None of that has been timed in the
  app. Whoever measures next has a hook: `import_timing_diagnostic` in
  `crates/fanta-fig-interop/src/fig/fixture_tests.rs` is an ignored test that
  prints `read_fig` and `fig_to_doc` timings for the file named by
  `FANTA_FIG_FIXTURE`. It covers the crate, not the canvas.
- **Memory was high on the previous build, and the first edit cost about
  2.3 GiB that was not given back. Those numbers have not been re-measured on
  this tree.** Measured on the release build of `16309b2` with one 9.6 MB,
  29,301-node `.fig`: importing it settled at 4.3 GiB resident, reopening the
  saved project settled at 5.0 GiB, and the first edit-plus-autosave peaked at
  7.6 GiB and stayed near 7.4 GiB. The cost was isolated to the autosave write
  path — a screenshot added 96 MiB and reading the source added 41 MiB — and it
  was a one-time high-water mark rather than a leak: four further edits stayed
  flat. That write path is exactly what this branch replaced (the persisted
  clone drops the undo history — the comment in `FigItem::save` names the undo
  stack's subtree snapshots as the save path's high-water mark — and the
  projection no
  longer builds a whole-document `serde_json::Value`), and the import path now
  keeps image assets encoded until they are drawn, under a 1.5 GiB cap on
  decoded pixels (`DEFAULT_DECODED_BUDGET_BYTES`,
  `crates/fanta-render/src/asset.rs`). So the figures above describe code that
  is gone, and the new code has never been watched in Activity Monitor. Nobody
  has opened the 128 MB community UI kit that the smoke checklist's timing row
  calls for, on either build.
- **The first frame of a page may decode its images.** Images are decoded
  lazily: `LazyAssetResolver::resolve` (`crates/fanta-render/src/asset.rs`)
  decodes an asset the first time the renderer asks for it, on whichever thread
  is rendering. The page a document opens on is prewarmed on the background load
  thread (`FigDocument::prewarm_default_page_assets`, called from both
  `load_fig_document` and `load_project_document`), so its first frame should
  not stall; every other page pays on first draw. Decoded pixels are kept under
  `DEFAULT_DECODED_BUDGET_BYTES` (1.5 GiB) with least-recently-used eviction,
  so on a document with more image pixels than that, scrolling back to an
  evicted image decodes it again; a single image larger than the whole budget is
  kept rather than re-decoded every frame. A corrupt image logs `failed to
  decode embedded .fig image asset …` at WARN **once** — the failure is cached —
  and draws as missing from then on. Unit-tested in `asset.rs`; no page of this
  tree has been drawn.

## Agents

- The agent panel needs a project open before it will chat. Opening a `.fig` or
  a Fanta project satisfies this.
- The managed Fanta provider requires signing in. Without it, set an Anthropic
  API key in the agent panel's settings; that is the alpha's default path.
- **`read_fnx_source` returns a slice, not a whole file.** It stops at 65,536
  bytes by default (`DEFAULT_SOURCE_BYTES`) and reports `total_lines`,
  `total_bytes` and `truncated`, with a `notice` naming the arguments for the
  next slice; `offset`, `limit` and `max_bytes` (ceiling 1 MiB,
  `MAX_SOURCE_BYTES`) ask for a different one. The page of a 9.6 MB `.fig` that
  used to come back as 43,145,940 characters in one block now measures 60,719.
  On a document that size, paging through it is still the wrong move — use
  `get_editor_state` and `batch_get` for structure. `batch_get` with no `ids`
  now defaults to `depth: 2` rather than walking the whole scene, and any result
  over 256 KiB (`MAX_JSON_RESPONSE_BYTES`) is refused with a message naming its
  size and how to narrow it.
- `get_screenshot` is not bounded the same way: a dense page at a large
  `max_dimension` can produce several MB of base64. The default of 1024 measured
  64,844 characters in the rehearsal and is safe; pass a large `max_dimension`
  only deliberately.
- **`tools/list` now returns six tools, and every document that said five is
  a step behind.** `get_guidelines` was added next to `get_editor_state`,
  `batch_get`, `batch_design`, `get_screenshot` and `read_fnx_source`
  (`server.add_tool(…)`, `crates/fig_viewer/src/live_mcp.rs`); it takes no
  arguments and returns `design_surface::DESIGN_GUIDELINES` — coordinates,
  frames versus groups, auto layout, spacing and type scales, naming,
  components, a working method, and when to edit `.fnx` instead. The seeded
  `AGENTS.md` (`crates/fanta-format/src/project/layout.rs`, "The six tools")
  documents all six. `script/smoke-mcp`'s assertion *"tools/list carries the
  six canvas tools"* now requires `get_guidelines` in `REQUIRED_TOOLS` too; it
  checks that the six names are present rather than that the list has six
  entries. No client has listed this tree's tools since the sixth was added.
- **`batch_design` (and the built-in agent's `design_edit`) accept fourteen new
  ops that only unit tests have exercised.** The shared vocabulary is
  `design_surface::DesignOp` (`crates/design_surface/src/design_surface.rs`,
  tagged by `"op"`): `create_instance`, `set_stroke`, `set_shadow`,
  `set_text_style`, `set_auto_layout`, `set_index`, `rotate`, `align`,
  `distribute`, `group`, `frame_selection`, `ungroup`, `duplicate` and
  `create_component` join `create_node`, `create_image`, `set_props`,
  `reparent`, `delete`, `select` and `set_viewport`; each is implemented as
  `fanta_doc` operations in `fig_viewer::agent_surface::apply_one` inside the
  existing single transaction, so a failing op still rolls the whole batch
  back and names its index. `group` / `frame_selection` / `ungroup` reuse the
  same `structure.rs` builders as the keyboard shortcuts above, so they refuse
  the same things. `get_editor_state` also grew: `active_page_bounds`,
  `selection_bounds`, a `hints` list, and an optional `empty_space: [w, h]`
  query answered with a free `{page, x, y, width, height}` to the right of or
  below the page's content with a 100 px margin
  (`agent_surface.rs::empty_space`, `EMPTY_SPACE_MARGIN`). The only op any
  script has ever driven against a running app is `create_node`
  (`script/smoke-mcp`, its one `"op"` literal); everything else here is a source
  reading backed only by the unit tests at the bottom of `agent_surface.rs`.
- **The built-in agent's system prompt is now Fanta's, with a design section
  that appears only when the design tools are available.**
  `crates/agent/src/templates/system_prompt.hbs` opens with *"You are the Fanta
  agent, running inside Fanta"* and renders a `## Design canvas` section — the
  `design_state` → `design_edit` → `design_screenshot` workflow, coordinate
  rules and a condensed copy of the guidelines — under
  `{{#if (contains available_tools 'design_edit')}}`. If the section is missing
  from a thread, the design tools were not registered for it. Unit-tested in
  `templates.rs`; no thread has been run against this tree.
- Nothing removes the MCP discovery file
  (`~/Library/Application Support/Fanta/fanta_live_mcp.json`) when the app is
  killed, so it can name a dead process — the module comment at
  `mcp_stdio.rs:11` says so. The stdio bridge defends itself by checking the
  recorded pid with `kill(pid, 0)` before trusting it, but a third-party client
  reading that file directly will not.

## Version control

- **Without Xcode Command Line Tools, a new project is not a git repository.**
  The first save shells out to `git init -q` with a bare `git` resolved through
  `PATH` (`document.rs::git_init_if_needed`). On a Mac with no developer
  tools, `/usr/bin/git` is the shim that raises the *"The git command requires
  the command line developer tools"* dialog and exits non-zero; the failure is
  then logged as a warning and swallowed on purpose — a save must never fail
  because version control is unavailable. Result: **the design saves, but there
  is no `.git`**, so Review Changes has nothing to show and the headline "every
  canvas edit is a reviewable diff" quietly does not apply. Note this path does
  **not** use the `git` binary `script/bundle-mac` ships inside the app
  (`Contents/MacOS/git`) — that one backs the git *UI*
  (`git::repository`'s `bundled_git_binary_path`), not this `init`. Install the
  tools (`xcode-select --install`) or run `git init` in the project folder
  yourself.
- A project created inside an existing repository is deliberately not given its
  own nested `.git` — `git_init_if_needed` walks the ancestors — so its changes
  show up in the repository you already have.
- **Git-initialisation has only ever been observed on the `.fig` import path.**
  The rehearsal materialised its project by opening a `.fig` and saw `.git`
  appear. `File > New Design…` reaches the same `write_project` and therefore the
  same `git_init_if_needed`, but that menu path has never been driven — it is a
  source reading, not an observation. [`SMOKE.md`](SMOKE.md) row 2 is the row
  that would settle it.

## Network and privacy

- **Telemetry is off in the default settings.** `assets/settings/default.json`
  sets `telemetry.diagnostics`, `telemetry.metrics` and
  `telemetry.anthropic_retention` to `false`. That is a statement about the
  default settings, **not** a claim that nothing leaves the machine — no one has
  audited every outbound path in this build.
- **Font downloads reach GitHub.** A document using a font you do not have
  installed triggers a fetch from `raw.githubusercontent.com` (the
  `google/fonts` repository) and, for Source Sans/Serif Pro, from
  `github.com/adobe-fonts/…/releases/download/…`
  (`crates/fanta-text/src/font_resolver/download.rs`). Faces are cached under
  `~/.cache/fanta/fonts` and not re-fetched.
- **The Node and npm download paths are still compiled in.** `node_runtime` is
  still linked into the binary; it downloads Node from `nodejs.org/dist/…`
  (`node_runtime.rs:656`) and exposes `npm_install_packages`, which reaches
  `registry.npmjs.org` when an external ACP agent is installed. Nothing was
  observed reaching either during the rehearsal, but a strict network allow-list
  has to account for both.
- Because of the entries above, a network test that asserts "nothing from
  github.com" or "nothing from npm" is **wrong**. [`SMOKE.md`](SMOKE.md) step 9
  carries the full allow-list, each host cited to the file that names it.
- One `TCP 127.0.0.1:<port> (LISTEN)` socket stays open for the life of the
  process. The port matches the stable-channel single-instance handshake
  (`crates/zed/src/zed/mac_only_instance.rs`). Over roughly one idle minute the
  rehearsal saw zero bytes in or out and no outbound connections at all; the
  full five-minute idle test and the agent-turn test were never run.

## Leftovers from the Zed fork

- **Palette entries are still prefixed `zed:`.** The prefix is the action
  namespace, deliberately not renamed for the alpha — renaming namespaces breaks
  keymaps and every `zed::` action path in the tree. The filter lists in
  `crates/zed/src/main.rs` were tightened after the alpha.1 count: the `dev`,
  `feedback` and `remote_debug` namespaces are hidden, and so are
  `zed: open zed url`, `zed: open account settings`, `zed: open server settings`,
  `zed: open telemetry log`, `zed: show update notification`,
  `zed: open performance profiler`, `zed: extensions`, `zed: reset database`,
  `zed: debug elements`, `zed: test crash` and `zed: test panic` (the last two
  were only ever handled behind the server-side `PanicFeatureFlag`).
  `zed: about`, `zed: quit`, `zed: open settings` and
  `zed: copy system specs into clipboard` stay visible. This is the largest
  remaining branding leak and it is a deliberate deferral, not an oversight.
  Two caveats: the alpha.1 derivation (**929 of 1,031 actions visible, 44 in
  the `zed` namespace**, from `fanta --dump-all-actions` on a binary built at
  `493e364`) has not been re-derived against the tightened lists, and it is a
  filter-list derivation, not the palette as rendered — nobody has typed into
  the palette in a running build of this tree.
- **Searching the palette for "terminal" returns one entry, and it works.**
  `agent: new terminal thread` (`agent_ui::NewTerminalThread`) survives the
  filter because `terminal_view` is still linked into the app: it is how an
  external ACP agent runs. The `terminal` and `terminal_panel` namespaces are
  otherwise hidden, so nothing else matches. Searches for "vim", "debugger",
  "project panel" and "new file" return **zero** visible actions by the same
  derivation.
- **The two wrong agent-panel strings from alpha.1 are gone from the tree, but
  no running build has been driven since.** `crates/agent_ui/src/ui/end_trial_upsell.rs`
  (*"Upgrade to Fanta Pro"*, *"Your Fanta Pro Trial has expired"* — for a
  subscription that does not exist) was deleted together with its mount in the
  agent panel, and the free-usage-limit callout lost its "Upgrade" button. The
  authentication callout now resolves the built-in agent's id
  (`ZED_AGENT_ID = AgentId::new("Zed Agent")`, `crates/agent/src/agent.rs`)
  through its display label (`agent_display_name` in
  `crates/agent_ui/src/conversation_view.rs`, unit-tested), so
  *"Authenticate to Zed Agent"* can no longer render. The rest of the CLI and
  the agent composer were de-branded in the earlier fix pass — `--user-data-dir`
  documents `~/Library/Application Support/Fanta`, and there is no "Try Zed Pro
  for Free" or "Message the Zed Agent" string left in `crates/agent_ui`.
  Identifiers, the `zed://` URL scheme, `ZED_*` env vars and action namespaces
  are unchanged on purpose.
- **This build never updates itself.** `script/bundle-mac` exports
  `ZED_UPDATE_EXPLANATION='Alpha builds are updated by downloading a new DMG
  from Fanta.'`, so "Check for Updates" shows that as an information prompt
  instead of erroring, and the poll of the Zed release endpoint is suppressed.
  Get new alpha builds by downloading a new DMG.
- The once-per-launch `ERROR … agent_ui/src/message_editor.rs:601 language not
  found` from alpha.1 should no longer fire: the agent composer now asks for a
  Markdown grammar only when one is registered
  (`available_language_for_name("Markdown")` in `message_editor.rs`). Logs
  written by the alpha.1 binary still contain it; a fresh build has not been
  launched to confirm its absence.
- A second ERROR can appear under load and is also **cosmetic**:
  `gpui_macos::metal_renderer failed to render: scene too large: … retrying
  with larger instance buffer size`. It says what it does — the renderer grows
  its instance buffer and draws the frame. Re-counted just now, the current
  `~/Library/Logs/Fanta/Fanta.log` (103 KB, five launches) holds **13 ERROR
  lines: twelve `language not found` and one `scene too large`**, zero
  `didn't find an action` lines, and no `panic` match beyond
  `panic handler registered`. That log was written by the older binary and
  pre-dates `16309b2`.
- Panic backtraces in the shipped build are **bare addresses**, because
  `script/bundle-mac` runs `/usr/bin/strip` over `Contents/MacOS/fanta` (and
  `cli`) before signing. The same script runs `dsymutil --flat` first, so the
  build leaves `target/<triple>/release/fanta.dwarf` next to the DMG — outside
  the `dmg/` staging directory, so it is never packaged and has to be kept by
  hand. Symbolicate with:

  ```
  atos -o fanta.dwarf -arch arm64 -l <load address> <address>
  ```

  For a `-d` (debug) bundle `dsymutil` is skipped entirely and there is no
  `fanta.dwarf` at all.

## Platform

- **Apple Silicon only.** `script/bundle-mac` builds for the host triple and
  this alpha was built on `aarch64-apple-darwin`; no Intel or universal DMG is
  produced.
- **Only one macOS version has been tested: 26.6.2, the build machine's.** The
  compiler deployment target is `10.15.7` (`crates/zed/build.rs`) and the bundle
  sets no `LSMinimumSystemVersion`, so nothing in the build refuses an older
  system — but nothing has been run on one either. Windows and Linux are not
  supported.

## Test suites

- The `sidebar` (8 failures) and `agent_ui` (14 failures plus one hang) test
  suites are **red**. These pre-date the alpha work and sit off every path it
  touched; the `agent_ui` failures were confirmed pre-existing by backtrace in
  an earlier pass. Those counts were **not re-run for this documentation pass**
  — they are carried over from the pass that measured them, and the numbers may
  have drifted. Treat them as "these two suites are known red", not as a
  current count. Re-running them was skipped again for this pass. Both crates
  still exist (`crates/sidebar`, `crates/agent_ui`); this branch edited
  `agent_ui` (the removed upsell, the agent display name, the Markdown-grammar
  guard) and added unit tests for the display-name helper, but the suite as a
  whole was not re-run.
