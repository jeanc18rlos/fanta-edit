# fig_viewer rules

The Fanta canvas app. `document.rs` holds `FigItem` (load, save, disk sync), `view.rs` holds `FigView` (tools, actions, mounting), `canvas.rs` the Metal canvas, plus the panels, code workspace and the agent surface (`agent_surface.rs`, `live_mcp.rs`). Before changing agent tools or the live MCP server, also read `crates/design_surface/AGENTS.md`.

PR numbers point at the change that found each trap (`gh pr view <n>`).

## Re-entrancy

- Inside a `Workspace::register_action` handler or a `workspace.update`/`update_in` closure, show notices with `show_workspace_canvas_notice(workspace, ..)`, never `show_canvas_notice`. The latter updates the window's workspace again and GPUI panics on the double lease. (#131)
- Tool and MCP connection methods can run inside a `FigView` or `FigItem` update. Defer work that reads the same entity to a spawned foreground task: code before the first `.await` still runs while the caller holds the lease. (#26)

## Rendering and performance

- GPUI re-renders every visible view on every window redraw, regardless of subscriptions. Panel `render` must be O(visible content): build anything O(document) (layer trees, page, component and asset snapshots, layer-action availability) in event handlers and cache it. Build document-wide lookups once per projection, never per row, and run reference scans only when a command executes. (#40)
- Panels subscribe to the item's `FigItemEvent` stream. Do not drive panel renders from `cx.observe(&fig_view)`: the view notifies on every pan and pointer move. Heavy listeners ignore `FigItemEvent::EditedTransient`, which fires every drag-preview frame. An observer that cannot be avoided stays O(1) and notifies only on change (see `set_inspecting`).
- `dispatch_tool_event` notifies only when viewport, overlays or cursor changed. Keep tool paths from calling `cx.notify()` unconditionally.
- Every keystroke in a canvas text field (comment composer, annotation note) redraws FigView and every visible panel. Paint repeated overlay items (comment pins, badges) with `paint_quad` and skip the ones outside the canvas bounds: a `PathBuilder` path per pin is tessellated on the CPU every frame, and 200 pins cost about 17 ms a frame in a debug build. Render long panel lists through `gpui::list`.
- A camera move supersedes visible canvas rendering; it never cancels it. Use `CameraTicket::superseded` only for speculative maintenance and `CameraTicket::cancelled` (content, page, size or scale changes) for visible renders. Cancelling on camera moves starved pans to 0.2 frames/s. (#129)
- Never allocate a presentation buffer per frame. GPUI imports painted surfaces through a CoreVideo texture cache that keeps each IOSurface alive about a second, so per-frame buffers pin ~60 frames (~1.5 GB) while panning. Take buffers from `PresentationPool`; `PresentationTracker` recycles them. (#130)
- Keep Graphite's `ContextOptions::fExecutor` (the vendored `MultiListFIFOThreadPool` shim). Without it, pipelines compile inline in `Recorder::snap` and stall frames for 200–1300 ms.
- A transient preview write (`get_mut` on the scene) does not bump `scene.revision()`, which keys the canvas frame cache. Call `invalidate_canvas_cache()` after preview writes.

## Documents, saving and disk sync

- Never derive design paths from ids: page and component directories are name slugs. Use `fanta_format::locate_page_source`, `locate_master_source` and `page_scope_of_source`. For agent metadata use `indexed_design_source`, because the `locate_*` functions scan directories. (#126)
- Async work that adopts a disk snapshot (reload, merge, discard) captures `sync_epoch` and resyncs if it advanced. Otherwise a snapshot read before a save silently reverts the saved content.
- Capture saved document and asset baselines only after a queued save receives its write lease. Persisted clones get a fresh `Scene` instance, so when the baseline advances, compare shared-node ownership instead of trusting a live revision journal. (#126)
- One `FigItem` can back several views, and only one view owns a pending preview. Grouped saves and closes commit view-local input before moving focus, and process the view that owns pending input before its siblings. (#123)
- When restoring shared-document canvas tabs concurrently, derive each view's entry and scope from its own saved path immediately before construction. The global pending view descriptor can belong to another open by the time a load finishes. (#41)
- Opening a native panel (file picker, save dialog) deactivates the window, and `workspace_deactivated` runs `finish_document_edits`, which can discard invalid drafts. Refuse while any edit session is pending, not only while `content_preview_active()` is true. (#4)
- Remove component-bearing content with `layer_context_ops::delete_layers`. `clipboard::delete_operations` only removes scene nodes, which breaks surviving component references. Test Delete and Paste-to-replace against the post-removal component registry. (#61)
- Infer vector viewports only at the `.fig` decoding boundary. An omitted `local_size` in a native project is valid authored state. (#62)
- After Save, refreshing embedded source buffers keeps the transaction a no-op reload returns (it can be the previous authored one), and merges canonical layout changes into the triggering source edit so one Undo restores both. (#54)
- Derive macOS video duration from native playback preparation. Raw MP4 track duration can include samples an edit list excludes. (#4)

## Input, focus and keymaps

- Canvas keymap blocks use contexts like `FigViewer && !Editor && !Input`. Toolbar search and inspector fields are `gpui_component::Input`, so excluding only `Editor` fires single-key shortcuts while the user types. (#1)
- A control that can refuse a mode or workspace change because a draft is pending must keep the editor's focus on mouse-down until the change is admitted (see `draft_preserving_toolbar_focus_scope`). A clickable child inherits its ancestor's focus transfer, so a check in `on_click` runs after blur has already committed the draft. (#7)
- A canvas press that opens an overlay editor (comment composer, annotation note) must focus it and call `window.prevent_default()`. The press keeps bubbling to FigView's tracked-focus element, which takes focus back: typing then runs canvas shortcuts, and Backspace deletes the selected layers.
- Canvas popovers are painted with `deferred(..)`. Sibling order inside `fig-container` does not lift them above ancestor workspace tabs, and they must stay inside the canvas content mask. Check new popovers against the floating toolbar and the workspace tabs. (#3)

## Variables

- The Variables workspace maps `VariableType` onto the external `fanta_gpui` `VariableKind` (Color, Number, String, Boolean) in `screen_variable_kind`, and skips every variable that maps to `None`. A new kind must map onto one of those four or it vanishes from the tab, as Typography does today; adding a glyph or editor means a `fanta-ui` change. (#140)

## Exports

- The default build's Design-mode inspector is fanta-gpui's `DesignPanel`, whose export formats are fanta-ui's `DesignExportFormat` (PNG, JPG, SVG, PDF). `EXPORT_FORMATS` reaches only the legacy inspector (`FANTA_GPUI_DESIGN=0`). A new export format therefore needs a Design menu action, or a `fanta-ui` change, before the shipped app can use it. (#150)

## Tests

- Mount the real UI. Sidebar regressions mount the sidebar owned by `FigView` inside a workspace and click its visible control. Keyboard regressions mount `MultiWorkspace` with the shipped keymap, because a bound action can consume a key before a raw listener sees it. (#6, #7)
- The `gpui-test` skill covers focus, drag and popover-menu test patterns.
- Simulate platform shortcuts with `secondary-`, which is Command on macOS and Control elsewhere; a `cmd-` keystroke does nothing on Windows. Build a `RelPath` from a native path with `RelPath::new(path, PathStyle::local())`: `rel_path(path.to_str())` keeps Windows backslashes inside one component. (#149)
- Wait for real-file work with a condition, not `run_until_parked`. On Windows `RealFs::metadata` reads the file id on smol's blocking threads, outside the test scheduler, so opening a buffer or refreshing a worktree entry finishes after `run_until_parked` returns; on macOS it finishes inside it. In the code workspace tests, call `run_until_sources_open` after anything that opens source buffers: showing the workspace, switching pages, saving a renamed or new page. To reproduce on a Mac, add a `smol::unblock` sleep to `RealFs::metadata`. (#151)
- Build test paths one component at a time: `join("Original").join("fanta.json")`. On Windows `join("Original/fanta.json")` keeps a `/`, a worktree created from it keeps that spelling, and session restore, which looks saved sessions up by the text of canonicalized roots, misses the session. (#151)
- The crate must compile and test off macOS: the Windows workflow checks the app and runs this crate's tests. Gate tests that use `std::os::unix` (permission bits, symlinks) with `#[cfg(unix)]`, together with any helper only they use, and give every new `#[cfg(target_os = "macos")]` path a non-macOS branch. Use a crate from the macOS-only dependency table only inside macOS-gated code: audio placement called `rodio` ungated and broke the Windows build. (#149)
- Stage `editor` of `script/verify-fanta-release` runs `cargo test --locked -p fig_viewer --lib --example drag_bench`. CI also requires specific tests from this crate by name (see the root rules).
