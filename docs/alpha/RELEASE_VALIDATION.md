# Release validation — 2026-09-09

**The customer release is not yet validated.** This records completed checks
and the remaining work for the Apple Silicon alpha. Configuration and launch
instructions are in [LAUNCH.md](LAUNCH.md).

## GitHub and local checks

The desktop [push CI run](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34380329072)
and [pull request CI run](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34380334599)
both passed at `5181599`.
A later full native release build including the six-tool whitelist passed in
5m27s. The subsequent sidebar ownership and generation-interface fixes have
passed their focused tests; a final native build and fresh hosted checks are
pending. Those later fixes have not yet been retested in the running app.

The backend [CI run](https://github.com/jeanc18rlos/fanta-backend/actions/runs/34380169196)
passed at pull request head `9d05908`, with **365 TypeScript tests**, **73
isolated GPU tests**, and type checking. These checks do not establish deployed
GPU readiness or successful production AI requests.

Local validation results:

| Check | Result |
| --- | --- |
| `cargo test --locked --offline -p fanta-doc --lib` | 333 passed; 1 explicit benchmark ignored |
| `cargo test --locked --offline -p fanta-format` | 181 passed, including integration tests and doctests; 2 ignored |
| Focused Rust formatting and `git diff --check` | Passed |
| `.github/workflows/check.yml` with actionlint 1.7.12 | Passed; now includes `cargo test --locked -p fanta-doc -p fanta-format` |
| `cargo build --locked --release -p zed --bin fanta` | Latest build passed in 5m27s, including the six-tool whitelist; before the sidebar ownership fix |
| Browser sign-in callback recovery | 2 passed; valid encrypted callbacks, invalid callback rejection, 10-minute deadline, retry and cancellation |
| Document lifecycle suite | 30 passed; bundled Git resolution, long-save watcher suppression, overlapping/failed/canceled saves, and entity release |
| Native toolbar interaction suite | 32 passed, including pointer focus, popup dismissal, input editing, and keyboard navigation |
| Toolbar input with the shipped canvas keymap | 1 passed; typing and clicking in search preserve the query, Escape restores shortcuts |
| Native generation/media suite | 27 passed, including actual toolbar activation, catalog authorization, prompt-to-SVG, visible prompt rendering, accurate source clicks, and scrollable inpainting controls |
| Sidebar suite with ownership fix | 131 passed, 8 failed; baseline was 128 passed with the same 8 failures. Three new tests passed; the suite is not fully green. |

The local CI workflow now includes the focused sidebar ownership regressions
so future runs cover them without adding the eight known baseline failures.
This workflow change is not evidence that those baseline failures are fixed.

New tests cover snapshot isolation through node/hierarchy edits and unchanged
JSON serialization. A GPUI test also covers closing the visible assistant
while the center document has focus, reopening it, and restoring document
focus. It passed locally (1 test; 0.37s) with:

```sh
cargo test --locked --offline -p agent_ui --lib test_toggle_closes_visible_agent_panel_when_center_pane_has_focus
```

Backend coverage includes early idempotent recovery, Gateway planner routing,
worker source fixes, and safe validation-error formatting. Production
deployment and authenticated end-to-end verification remain pending.

## Measured snapshot improvement

The [scene benchmark](../../crates/fanta-doc/examples/scene_snapshot.rs) ran on
macOS 26.6.2, Apple Silicon, 36 GiB memory, Rust 1.95.0, release profile.
It creates 10,000 vector nodes with 256 cubic segments each, retains three
snapshots, and performs 12 cycles of moving one node, inserting a rectangle,
and replacing one snapshot. The new snapshot exists before the old one drops.

Baseline: owned-node scene implementation, graph blob
`59566b7662e74e689486bc414322b54db5110f14`. After: the current scene implementation
shares unchanged nodes and copies only edited nodes. The benchmark workload
and profile were the same, with three separate process runs per implementation.

| Trial | Peak RSS before → after, MiB | Median snapshot time before → after, ms |
| --- | --- | --- |
| 1 | 1006.297 → 302.672 | 9.624 → 0.051 |
| 2 | 1006.156 → 302.703 | 8.967 → 0.055 |
| 3 | 1006.156 → 302.656 | 8.825 → 0.050 |

Peak process memory fell about **70% in this workload**. Snapshot timing
measures cloning, excluding disposal of the replaced snapshot. Median edit
time was 0.008 ms before and 0.003–0.005 ms after. These measurements do not
establish whole-app memory, frame rate, leak freedom, or superiority to Figma.

Reproduce each implementation with the same example source, running the
executable three times after building:

```sh
cargo build --locked --release -p fanta-doc --example scene_snapshot
/usr/bin/time -l target/release/examples/scene_snapshot
```

## UI and large-document baseline

The existing release binary opened New Design in an isolated QA profile.
The visible baseline also exposed a canvas squeezed to about 120 px by the
sidebars, a Commit menu action that did nothing, and an assistant toggle that
focused the panel instead of closing it. Source fixes adjust layout, mount
GitPanel, and close the active assistant regardless of focus.

The rebuilt QA app, using a fresh profile, visibly made Fanta account sign-in
the primary onboarding choice and gave the canvas more room. A newly created
empty design visibly starts at 100% zoom. View → Toggle
Agent Panel closed the visible panel and expanded the canvas. The saved green
shape in the synthetic `Canvas Flow` project reopened correctly.

File → Review Changes → Stage All → Commit created `Save initial canvas design`
through the app. The review's Publish button pushed it to the private
[validation repository](https://github.com/jeanc18rlos/fanta-release-validation).
GitHub and local history both report commit
`2bb85c6a7c4500f34f9a830713a259333007933a`; the working tree is clean and tracks
`origin/main`. This repository contains only the synthetic design, not the
private import fixture.

The app command palette opens the native Image workspace with Image, Video,
Vector, Design, and Masks modes. The toolbar focus/keymap, ten-minute sign-in
callback, catalog authorization, and prompt-to-SVG fixes are included in the
latest native build. The production sign-in page previously identified its
Clerk instance as Development mode; production account configuration still
needs verification. An earlier native sign-in succeeded with the existing
Google session. The final production account/catalog check is currently
blocked by a macOS Keychain approval that requires manual interaction; the
automation tool cannot approve it.

In a separate local QA profile, the rebuilt native app signed in to a
loopback fixture and loaded its authenticated, visibly QA-labeled catalog.
Image submission, automatic polling, a gallery with two results, PNG saving,
placement on the canvas, and saving the design passed. Mask output and local
background removal also passed. The prompt-to-editable-SVG fixture passed
preview and Add to design; the expanded tree showed `QA-label`,
`QA-editable-shapes`, and `QA-background`. Shift-2 framed the result correctly,
and Cmd-S persisted the editable layers in `page.fnx`.

Video submission returned HTTP 202, automatic polling completed, and Play
video opened the one-second MP4 in QuickTime. Clicking Play advanced the
position to 0.409 seconds. Add to design and save persisted the MP4 layer.
File verification found the saved PNG was 4,771 bytes with a SHA-256 identical
to the fixture; the design contained the 4,771-byte PNG and 5,921-byte MP4
assets, with the named layers persisted. The fixture also accepted the saved
PNG through its upload-reuse path and completed image tracing with an SVG
preview. Design mode prepared the requested brief in the assistant without
sending it; it did not generate a complete design. After tracing, counters
recorded four generated jobs, five polls, one Messages request, one reused
upload, thirteen authenticated requests, and zero authentication rejections.

These checks use deterministic local fixtures and a fake account; they do
not exercise real GPU inference, AI Gateway generation, or billing. Subsequent
rendered tests reproduced invisible prompt text (the editor measured 0×0) and
source-click bounds 200 px below the preview. Both are fixed in source, with
separate prompt drafts for each mode and an inpainting action that scrolls
into view at 1000×768. The full generation suite now has 27 passing tests.
Native verification of these later fixes and production checks remain pending.

A QA copy of `/Users/jeanrojas/Desktop/basic.fig` (9,589,921 bytes) was imported
through Open; `get_editor_state` reported 29,301 nodes. The converted editable
project was saved at `/tmp/fanta-release-qa-20260909/import-baseline/basic`.
Through the UI, one rectangle's fill changed from `#D9D9D9` to `#22C55E`;
Undo restored gray and Redo restored green. Cmd-S persisted the green value,
verified in `page.fnx` at line 21. File → Close Project closed the project.

The rebuilt app also imported a fresh copy at
`/tmp/fanta-release-qa-20260909/import-after/basic`, changed the same fill,
passed visible Undo/Redo, saved the green value to `page.fnx:21`, and closed
the project. These are single observations with different UI layout and
account state, not a controlled benchmark:

| Checkpoint | Original RSS, KiB | Rebuilt RSS, KiB |
| --- | ---: | ---: |
| Before import | 189,408 | 226,704 |
| After import | 3,540,848 | 3,387,200 |
| After first edit and save | 4,066,192 | 4,930,144 |
| After closing the project | 3,892,704 | 4,858,416 |

The synthetic snapshot improvement does not translate to a demonstrated
whole-app memory improvement here. A source race was found: the one-second
self-write suppression could expire while a long save was still writing,
starting a redundant full reload. The fix holds suppression through
foreground and background write completion and is covered by the 30-test
document lifecycle suite.

The latest memory guard run (PID 76062) showed RSS falling to about 180 MiB
after Close Project. That did **not** demonstrate document memory release:
`vmmap` still reported about 2.1 GiB physical footprint, 13.2 million
allocations, roughly 1.5 GiB of live allocation bytes, and about 1.6 GiB
swapped. The low RSS reflected compression/swapping, not released ownership.
Earlier lower-RSS or heap-census observations must not be treated as evidence
that the close-project retention was resolved.

A strong `active_entry` reference in the sidebar was found retaining the
document. A regression using the real Close Project path and a `WeakEntity`
failed before the fix, then passed with the panel absent and with an empty
panel. The full fixed sidebar suite reports 131 passes and the same 8 failures
as the baseline's 128 passes; the three new tests pass. The source fix is
frozen but has not been rebuilt or retested in the native app. Repeat
allocation/footprint measurements after close, reopen, and sustained editing
before drawing a whole-app memory conclusion.

The earlier [release rehearsal](REHEARSAL.md) measured the 128 MB, 40,141-node,
31-page UI kit opening in 48.5–54.1 seconds. It settled at 1952–1960 MiB, then
rose from 1960 to 5788 MiB after one rectangle and 25 seconds for autosave.
Later samples partly receded; these observations do not demonstrate an
unbounded leak. Repeat the same fixture and timing after rebuilding, followed
by repeated edits, undo/redo, page changes, saves, close/reopen, and idle checks.

## Remaining release requirements

| Goal | Evidence and remaining verification |
| --- | --- |
| Backend and AI Gateway | Backend head `9d05908` is green in CI. Apply the migration and deploy reviewed changes; verify authenticated streaming, debit, errors, and sign-out against production. Final native account/catalog verification awaits manual Keychain approval. |
| Charge customers | Billing fixes and backend tests exist. Pricing/product/currency selection and production Polar configuration remain blocked. Checkout, webhook retry/cancellation/renewal, exactly-once credits, and billing portal remain unverified. No customer was charged. |
| Native generation | 27 targeted tests passed. Local fixture sign-in/catalog, image polling/gallery/save/place, masks/background removal, editable SVG preview/place/save, and MP4 poll/play/place/save passed, with saved assets/layers verified. Recheck prompt, point-selection, and scrolling fixes in the final native build, then verify real media requests in production. Retry state/history lasts only for the tab lifetime; video playback uses the system player and canvas cards have no poster yet. |
| GPU service | Backend CI tests passed. Production HMAC access remains blocked; deployed GPU availability and successful end-to-end generation are not established. |
| Design and Git UI | Synthetic creation/edit/save/reopen and app-driven review/stage/commit/push passed. Import/edit/undo/redo/save passed on the baseline fixture; repeat that import workflow on the final build. |
| Performance and UI quality | Build and retest the sidebar ownership fix in the native app, then repeat full-app large-file and sustained-memory measurements. Low post-close RSS alone is not release evidence. Review layouts and interaction feedback. A Figma comparison needs the same fixture, hardware, operations, and measurement method. |
| Installer | No valid local signing identity was found. Configure Developer ID/notarization secrets, build a signed DMG, verify notarization, and install/launch on a clean Mac. |
| Leads | US PostHog project 410640 and the existing landing page are accessible. Validate direct visit-to-signup reporting and durable contact handling; no improved conversion rate or outreach result is established. |

Do not publish a release or enable customer checkout on the strength of the
synthetic benchmark, public health checks, or the earlier green CI runs alone.
