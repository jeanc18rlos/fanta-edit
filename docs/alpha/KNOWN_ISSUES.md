# Known issues, Fanta 0.1.0-alpha.1

Written from the code, the alpha rehearsal, and the focused validation record;
not from wishful thinking. Every entry is something a tester can hit on purpose.
Where a claim comes from reading source rather than from running the app, it
says so. Reconciled against desktop commit `17de5eb`; the release-facing status
matrix is [`CAPABILITY_INVENTORY.md`](CAPABILITY_INVENTORY.md).

**A release binary has now been built from this tree and driven.** On
2026-09-09, `cargo build --release -p zed` produced `target/release/fanta`,
reporting `Fanta: v0.1.0-alpha.1+stable.8f43236…`, and it was driven over its
own live MCP server on macOS 26.6.2, aarch64, 36 GiB. Two documents were
opened: the 9.6 MB, 29,301-node, 3-page `basic.fig` the previous rehearsal used,
and a 128 MB, 40,141-node, 31-page community UI kit ("UI3: Figma's UI Kit") that
no build of this project had ever opened before. `script/smoke-mcp` passed
**18 of 18, exit 0** against that binary. Roughly forty MCP operations were
issued across six launches, including twelve of the fourteen design ops that
were new on this branch — all but `set_index` and `frame_selection`. So the
entries below about launch time, memory, the tool list,
the design ops and the log are measurements of **this** tree, and the ones that
used to say "not re-measured" have been replaced by the numbers rather than
softened.

One caveat on "this tree": the binary the bulk of the measurements come from
reports `stable.8f43236`, and the tree moved after it was built. The
`offset`/`limit` pagination this change adds to `batch_get` was written in
response to the refusal that run found, so a **second** release binary was
built afterwards and driven to check it; that pagination result is the only
measurement here taken against the later binary, and it says so where it
appears.

**The full manual smoke pass has not been completed against one artifact.**
Focused native fixtures have clicked through Scale and Path Select behavior,
generation workspaces, durable generation recovery, and other bounded flows.
Menus, the complete toolbar, image paste, the DMG/Gatekeeper path, and several
other pointer-only surfaces still need the exact-artifact pass. The evidence and
its limits are in [`RELEASE_VALIDATION.md`](RELEASE_VALIDATION.md).

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
  app instead. A separate `9d92b30` QA bundle with its own bundle identifier
  and `--user-data-dir` nevertheless displaced the already-running stable
  Fanta process during native validation. The prior untouched bundle/profile
  was restarted after the pass, but parallel stable-channel QA instances must
  not be treated as isolated.
- **`fanta` with no arguments restores your last session, so you may never see
  the welcome page again.** The rehearsal hit this: after a project had been
  opened once, a bare launch reopened it rather than showing
  *"Your design is code."* and its **Connect Claude Code / Codex** entry. If you
  are looking for the Copy command / Copy Codex config buttons, use
  **File > New Window**. The welcome page has never been confirmed on screen at
  all — see [`REHEARSAL.md`](REHEARSAL.md) row 1 — so its rendered layout is a
  source reading only.

## Canvas and tools

- **Unfinished roadmap faces either decline explicitly or stay hidden.** Direct
  Image/Video placement, Arrow, Annotation, Measure, Dev mode/tools, arbitrary
  toolbar file attachment and voice, auto-keyframe recording, page duplication,
  and page links are not implemented. Their visible
  handlers raise *"&lt;control&gt; is not available in the Fanta alpha yet."* or an
  equally specific notice. Motion Path is hidden and unimplemented. Text on
  Path, the bounded canvas-selection attachment, and Motion time comments are
  implemented only in the current source line and have no native-artifact
  evidence. Scale and Path Select are implemented and have focused native
  history/reopen evidence; Image, Video, Vector, Masks, and Remove Background
  open their dedicated generation workspaces.
- **Ask AI and text-oriented commands prepare Agent drafts, not automatic
  edits.** A typed Ask AI prompt and the Replace Content, Rewrite Text,
  Translate Text, and Rename Layers templates open a fresh Agent Panel draft
  with page and selection context. Nothing is sent until the user reviews and
  submits it. Generate Design opens the Design workspace, whose explicit
  “Prepare unsent Agent brief” action uses the registered `design_state`,
  `design_edit`, and `design_screenshot` tools without requiring a generation
  account. It remains a draft handoff rather than the agreed end-to-end design
  generation, progress, review, acceptance, and Undo workflow.
- **Group, Frame selection and Ungroup are wired and registered.** `cmd-g`,
  `cmd-alt-g` and `cmd-shift-g` are bound to
  `fig_viewer::GroupSelection` / `FrameSelection` / `UngroupSelection` in both
  the `FigViewer && !Editor` and `FantaDesignPanel && !Editor` contexts
  (`assets/keymaps/default-macos.json`). Their Actions-palette entries route to
  the same handlers, as do the layers context menu entries
  (`design_panel.rs::handle_layers_context_action`,
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
  `structure.rs`. Exact palette search/action tests now cover all three structure
  commands and the four text-AI commands. The same builders were reached over
  MCP on 2026-09-09:
  `batch_design`'s `group` and `ungroup` ops succeeded on both test documents;
  `frame_selection` was not among the operations driven.
- Grid auto layout is removed from the default inspector; horizontal and
  vertical auto layout work.
- **Inspector paint edits now mutate undoably or decline explicitly in the
  current candidate.** Fill and stroke visibility, supported solid payloads,
  finite unbound gradients with an identity transform, and per-paint blend-mode
  changes produce document edits. Pattern, Image, Video, and Shader payloads,
  bound paint payloads, and unsupported or nonidentity-transform gradients
  raise the existing unavailable notice. Exact `9d92b30` native QA verified
  Fill/Stroke visibility, Fill Undo, and a rendered/exported finite Linear
  gradient. Per-paint blends and the remaining unsupported-payload notices
  retain automated rather than native coverage.
- Pass Through works. Linear Burn and Linear Dodge are explicitly unavailable
  because they cannot map to engine blend modes. Shadow blend is read-only
  because the document model has no representation for it.
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
  counted as a render failure. Unit-tested for parity; **nobody has dragged a
  node in this build**, so the only exercise this path has had outside those
  tests is whatever the MCP edits and the `get_screenshot` calls of 2026-09-09
  happened to trigger.

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
  compare cached and cold trees byte for byte. The three-file diff has now been
  taken from this write path too: on 2026-09-09 `script/smoke-mcp` made one node
  over MCP against the release build of this tree, on a freshly materialised
  project with a committed baseline, and `git status` afterwards showed exactly
  `doc/metadata.json`, `pages/page-1/page.fnx` and `pages/page-1/page.ids.json`
  — three files, no `fanta.json`. The edit reached `page.fnx` with no `cmd-s`,
  its mtime moving after 2.5 s.
- **A rectangle gains one attribute the first time a project is reloaded.**
  The loader backfills a vector's SVG viewport, so `<Rect width height />`
  becomes `<Rect width height local_size={[w, h]} />` on the first save after a
  reopen — one line per rectangle, once, and it stays a readable `<Rect>`
  thereafter. (A viewport that genuinely crops the shape has no `<Rect>`
  spelling and still prints as a canonical `<Vector>`; that is correct, not a
  regression.)
- **Export's engine and toolbar feedback work, but preset configuration is not
  visible in the default inspector.** Export needs a written project and can
  produce PNG, JPG, SVG, or PDF under `<project>/exports/`. Toolbar Export calls
  that same engine, preserves the current sidebar state, and mirrors running,
  success, and failure messages to the canvas. The default GPUI inspector still
  removes the legacy Export section, so users cannot configure the existing
  1×/2×/4× presets or formats from the shipped surface. Automated success and
  unsaved-canvas cases pass. Exact `9d92b30` native QA exercised the success
  route and wrote a valid 418×354 `Shape@2x.png`; native failure feedback is
  still open.
- **Auto-keyframe and Motion Path remain unfinished; time comments are
  source-only.** Motion mode can create/select/rename clips, change duration,
  add property tracks and keyframes, move/delete keyframes, edit
  interpolation/easing, apply entrance
  presets, play, loop, scrub, and zoom. The timeline-wide and contextual
  Keyframe menus now share the same seven-property catalog; a contextual choice
  adds or replaces one keyframe at the playhead in one undoable transaction.
  Animation Style applies an undoable preset and synchronizes the timeline, and
  the contradictory primary Motion flyout is removed. The source-only Time
  comment action captures the active page, clip, and exact integer-ms playhead,
  pauses, and arms canvas placement. Posting persists one `SetMeta` history
  operation, so one Undo removes the comment; FNX write/reopen preserves it.
  Static comments remain visible everywhere, but a timed pin is visible only in
  Motion on its exact clip. Thread navigation uses stable page/clip identity,
  pauses at the anchored time, clamps to a shortened clip without rewriting the
  anchor, and keeps an explicitly opened deleted-clip thread accessible without
  retargeting another clip. Mode, workspace, tool, page, clip, timeline-time,
  playback, scope, and source-lock changes clear unplaced intent. Time-comment
  arming, placement, and thread navigation refuse to replace an existing draft
  or unsent reply; unrelated mode and tool actions can still intentionally
  cancel a placed draft. Auto-keyframe still declines explicitly, and Motion
  Path remains hidden with no document implementation. Production uses
  `fig_viewer::TimelineShell`; the reusable `fanta-gpui` timeline tests exercise
  the pseudo editor, not the shipped timeline. None of the contextual keyframing
  or time-comment work has native or final-artifact evidence.
- The Code tab is read-only by design. Edit `.fnx` in your own editor and the
  canvas follows the file. Note that with autosave on, `page.fnx`'s mtime moves
  about a second after any canvas edit — **an unchanged mtime is no longer a
  useful check for anything**, and [`SMOKE.md`](SMOKE.md) has been corrected
  accordingly.
- A design tab is restored on relaunch through its project folder; that was
  verified in the rehearsal. Restoring a `.fig` that never materialised a
  project was **not** verified — if it does not come back, reopen it from
  File > Open Recent.
- **A 128 MB community `.fig` takes about fifty seconds to open. A 9.6 MB one
  takes about two.** Both measured on 2026-09-09 against the release build of
  this tree, timing launch to a canvas that answers `get_editor_state` with a
  node count. Nothing was clicked, so this is time-to-answering-canvas, not
  time-to-a-frame-someone-looked-at. `~/Desktop/basic.fig` (9.6 MB, 29,301
  nodes, 3 pages): **2.1 s and 2.4 s** over two runs. The 128 MB *UI3: Figma's
  UI Kit (Community)* file (40,141 nodes, 31 pages), which no build of this
  project had ever opened before today: **48.5 s, 48.6 s and 54.1 s** over three
  runs. Every *edit* tried against it afterwards applied; the one thing that did
  not work was listing a wide page's children, which the response cap refused —
  see the `batch_get` entry under Agents. The smoke
  checklist's timing row for that file, previously marked never-run, is now
  these numbers. The 30–40 s once recorded for the 9.6 MB file was a **debug**
  build of `16309b2`'s predecessor, so it is not a like-for-like comparison and
  no speedup is claimed from it; treat the release figures as the first ones of
  their kind. This branch did rework the import crate — `byte[]` fields decode
  to one `KiwiValue::Bytes` instead of one value per byte (`kiwi/value.rs`), the
  shared-style pre-pass no longer deep-clones every node change
  (`fields.rs::resolve_style_references` returns `Cow`s), master paths are
  built once per master (`instance_overrides/paths.rs::MasterPathCache`), and
  per-instance side tables are borrowed rather than copied
  (`orchestrator.rs`) — but nothing in these runs attributes a figure to a
  change. Whoever wants the crate's own share of that time has a hook:
  `import_timing_diagnostic` in
  `crates/fanta-fig-interop/src/fig/fixture_tests.rs` is an ignored test that
  prints `read_fig` and `fig_to_doc` timings for the file named by
  `FANTA_FIG_FIXTURE`. It covers the crate, not the canvas.
- **Memory is high, and the first edit still costs gigabytes that only partly
  come back.** Resident size, watched while the release build of this tree was
  driven over MCP on 2026-09-09:
  - 9.6 MB `basic.fig`: **2004 MiB and 2173 MiB** once the canvas was up, on two
    runs. After about fifteen MCP operations including `create_component` and
    `create_instance`: **3677 MiB and 3703 MiB**.
  - 128 MB UI kit: **1952, 1960 and 1958 MiB** once open and settled for 20 s,
    across three runs — no higher than the file thirteen times smaller, which is
    an observation, not something these runs explain.
  - The first edit on that document, isolated (open, settle, create one 10×10
    rectangle, wait 25 s for the debounced autosave to finish): **1960 MiB →
    5788 MiB**, about **3.8 GiB** for one rectangle and its save.
  - That is a high-water mark that partly recedes, not a per-edit leak: a
    following `set_props` left it at 5789 MiB, the next autosave brought it down
    to 5270 MiB, and a second edit-plus-autosave cycle to 5087 MiB.

  Read two things out of this and no more. Opening settles at roughly half what
  the release build of `16309b2` recorded on the same 9.6 MB file (4.3 GiB
  importing, 5.0 GiB reopening the saved project). And the first-edit high-water
  mark is still there — larger in absolute terms on a 128 MB document than the
  2.3 GiB that build's first edit cost on a 9.6 MB one — so the shape is
  unchanged even though the resting figures improved. No cause is claimed for
  either: this branch replaced the save path (`Doc::clone_for_persist` through
  `fanta_format::write_project_tree_cached`) and made image decoding lazy under
  a 1.5 GiB cap (`DEFAULT_DECODED_BUDGET_BYTES`,
  `crates/fanta-render/src/asset.rs`), but nothing in these runs isolates which
  change moved which number.
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
  and draws as missing from then on. Unit-tested in `asset.rs`; the only
  rendering asked of this build is `get_screenshot`, which returned valid PNGs
  of both test documents on 2026-09-09 — no window has been watched, and no
  document with more image pixels than the budget has been paged through.

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
  used to come back as 43,145,940 characters in one block measured **60,719**
  when `script/smoke-mcp` asked the release build of this tree for it on
  2026-09-09. On a document that size, paging through it is still the wrong move
  — use `get_editor_state` and `batch_get` for structure. `batch_get` with no
  `ids` now defaults to `depth: 2` rather than walking the whole scene, and any
  result over 256 KiB (`MAX_JSON_RESPONSE_BYTES`) is refused with a message
  naming its size and how to narrow it. On a wide page that refusal is a dead
  end; see the next entry.
- **On a wide page, `batch_get` cannot list the children at all.** Driven
  against the 128 MB UI kit on 2026-09-09: `batch_get` on the active page with
  `depth: 1` and `include_geometry: false` — already the narrowest listing the
  tool offers — produced **967,017 bytes** and was refused for exceeding the
  262,144-byte cap, while the refusal text told the caller to *lower `depth`
  (try 1)*, which is what it had just done. With no pagination there is no
  argument combination that lists that page's children, so an agent that has not
  been handed node ids some other way cannot get them. The fix — `offset` and
  `limit` on `batch_get`, and a refusal message that names them — lands in this
  same change, and **was** driven against the same document on a second release
  build: the page that refused is *Internal Only Canvas*, whose 8,920 direct
  children now come back in 45 windows of 200, every child exactly once and
  `child_count` agreeing with the total. The seven next-widest pages of that
  document list in a single call each. The unbounded request is still refused,
  but now reads *"page the children with `limit` and `offset`"* before it
  mentions `depth`.
- `get_screenshot` is not bounded the same way: a dense page at a large
  `max_dimension` can produce several MB of base64. The default of 1024 measured
  64,844 characters in the rehearsal and is safe; pass a large `max_dimension`
  only deliberately. Measured on 2026-09-09 against this tree's release build:
  `max_dimension` 1200 on the 9.6 MB document returned a 147,712-byte PNG in
  0.19 s that visibly contained the edits made over MCP, and the same call on
  the 128 MB UI kit returned 52,007 bytes in 0.15 s.
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
  entries. Confirmed by a client on 2026-09-09: the smoke run against this
  tree's release build saw all six names in `tools/list`, and `get_guidelines`
  answered with 4,075 characters.
- **`batch_design` (and the built-in agent's `design_edit`) accept fourteen new
  ops; twelve of them have now been driven, two have not.** The shared
  vocabulary is
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
  (`agent_surface.rs::empty_space`, `EMPTY_SPACE_MARGIN`). On 2026-09-09,
  `set_auto_layout`, `align`, `distribute`, `group`, `ungroup`,
  `set_text_style`, `set_stroke`, `set_shadow`, `rotate`, `duplicate`,
  `create_component` and `create_instance` were all issued over MCP against the
  release build of this tree, on **both** the 9.6 MB document and the 128 MB UI
  kit, and all succeeded; `create_component` wrote a real
  `components/drive-card/master.fnx` to disk through the autosave, and
  `get_editor_state` answered an `empty_space` query with a usable free
  rectangle. Creating a frame plus three children in one `batch_design` took
  0.06 s. **`set_index` and `frame_selection` were not among them** and remain
  source readings backed only by the unit tests at the bottom of
  `agent_surface.rs`, as does every path through the built-in agent's
  `design_edit` — nothing has been driven through the agent panel.
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
- **The CLI no longer says `Zed`, and that is now an observation.** On the
  release binary built from this tree, `fanta --system-specs` prints
  `Fanta: v0.1.0-alpha.1+stable.8f43236…`. The rehearsal recorded that same line
  as `Zed: v0.1.0-alpha.1+stable.6e14229 (Fanta)`; that leak is gone.
- **The two wrong agent-panel strings from alpha.1 are gone from the tree, and
  the panel itself still has not been looked at.** `crates/agent_ui/src/ui/end_trial_upsell.rs`
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
  are unchanged on purpose. Everything in this entry is a grep of the tree: the
  agent panel has never been opened in a build of it.
- **This build never updates itself.** `script/bundle-mac` exports
  `ZED_UPDATE_EXPLANATION='Alpha builds are updated by downloading a new DMG
  from Fanta.'`, so "Check for Updates" shows that as an information prompt
  instead of erroring, and the poll of the Zed release endpoint is suppressed.
  Get new alpha builds by downloading a new DMG.
- **The once-per-launch `language not found` ERROR is gone.** The alpha.1 line
  — `ERROR … agent_ui/src/message_editor.rs:601 language not found`, once per
  launch — did not fire once on 2026-09-09: the agent composer now asks for a
  Markdown grammar only when one is registered
  (`available_language_for_name("Markdown")` in `message_editor.rs`). Across
  every run of the release build that day — three opens of the 9.6 MB file,
  three of the 128 MB UI kit, roughly forty MCP operations, several screenshots
  and many autosaves — `~/Library/Logs/Fanta/Fanta.log` recorded **zero ERROR
  lines, zero panics and zero `language not found` lines**. Every
  `language not found` line the log still holds pre-dates that day.
- A second ERROR can appear under load and is **cosmetic**:
  `gpui_macos::metal_renderer failed to render: scene too large: … retrying
  with larger instance buffer size`. It says what it does — the renderer grows
  its instance buffer and draws the frame. It did not fire on 2026-09-09
  either, which is unsurprising: nothing was dragged, zoomed or scrolled. The
  ERROR lines the log does contain — **13: twelve `language not found` and one
  `scene too large`**, counted in an earlier pass over a 103 KB log of five
  launches — were all written by older binaries, before `16309b2`.
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
  current count. Re-running them was skipped again for the 2026-09-09 pass, in
  which a release binary was built and driven but these two suites were not
  touched. Both crates
  still exist (`crates/sidebar`, `crates/agent_ui`); this branch edited
  `agent_ui` (the removed upsell, the agent display name, the Markdown-grammar
  guard) and added unit tests for the display-name helper, but the suite as a
  whole was not re-run.
- A third suite has one red test: `cargo test -p agent --lib` finishes **681
  passed, 1 failed, 11 ignored**, the failure being
  `tools::write_file_tool::tests::test_streaming_format_on_save`, which panics
  with *"Parking forbidden"* out of `crates/scheduler/src/test_scheduler.rs`.
  Re-run on 2026-09-09 with this branch's code changes stashed, it fails
  identically, so it is pre-existing and sits off every path this branch
  touched. The design-tool tests in the same crate pass.
