# Release validation — 2026-09-09

**The customer release is not yet validated.** This records completed checks
and the remaining work for the Apple Silicon alpha. Configuration and launch
instructions are in [LAUNCH.md](LAUNCH.md).

## GitHub and local checks

Both hosted `Check` runs passed at
`c27b7a36f34dc020ee2316f381eb76c26a93dc8a`:
[pull request run](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34369734047)
and [branch run](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34369730134).
They cover app compilation and account/provider/media regressions. They do
**not** cover the subsequent memory, generation workspace, or UI fixes.

For the current copy-on-write scene change, these local checks passed:

| Check | Result |
| --- | --- |
| `cargo test --locked --offline -p fanta-doc --lib` | 333 passed; 1 explicit benchmark ignored |
| `cargo test --locked --offline -p fanta-format` | 181 passed, including integration tests and doctests; 2 ignored |
| Focused Rust formatting and `git diff --check` | Passed |
| `.github/workflows/check.yml` with actionlint 1.7.12 | Passed; now includes `cargo test --locked -p fanta-doc -p fanta-format` |
| `cargo build --locked --release -p zed --bin fanta` | Passed in 8m06s; before the subsequent prompt-to-SVG follow-up |
| Browser sign-in callback recovery | 2 passed; valid encrypted callbacks, invalid callback rejection, 10-minute deadline, retry and cancellation |
| Document lifecycle suite | 30 passed; bundled Git resolution, long-save watcher suppression, overlapping/failed/canceled saves, and entity release |
| Native toolbar interaction suite | 32 passed, including pointer focus, popup dismissal, input editing, and keyboard navigation |
| Toolbar input with the shipped canvas keymap | 1 passed; typing and clicking in search preserve the query, Escape restores shortcuts |

New tests cover snapshot isolation through node/hierarchy edits and unchanged
JSON serialization. A GPUI test also covers closing the visible assistant
while the center document has focus, reopening it, and restoring document
focus. It passed locally (1 test; 0.37s) with:

```sh
cargo test --locked --offline -p agent_ui --lib test_toggle_closes_visible_agent_panel_when_center_pane_has_focus
```

The native generation/media suite passed **23 targeted tests**, including the
total SVG geometry budget and prompt-to-SVG follow-up. The isolated backend release source passed **365 tests in
42 files**, **73 GPU tests**, and type checking after early idempotent recovery,
Gateway planner routing, worker source fixes, and safe validation-error
formatting. Production deployment remains pending.

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
Vector, Design, and Masks modes. A separate AI-toolbar search defect was
reproduced: typing invokes canvas shortcuts and dismisses the search. Its
focus/keymap fix and the prompt-to-SVG follow-up require another build and UI
check. The production sign-in page visibly identifies its Clerk instance as
Development mode; production account configuration needs verification. Native
sign-in succeeded with the existing Google session. A slow first attempt
outlived the 100-second callback listener. The signed-in model catalog also
returned 401 because the client omitted authorization. The callback and
catalog fixes passed focused tests and await the next native app build.

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
whole-app memory improvement here. Retained memory after closing remains a
release concern requiring allocation attribution and repeatable profiling;
these samples alone cannot establish a leak. After further UI activity and
idle time, RSS receded to 2,615,312 KiB. A heap census then reported 132,776
live allocations totaling 48,184,824 bytes, versus roughly 13.2 million
allocations/1.6 GiB in an earlier post-close vmmap sample. This suggests delayed
cleanup and allocator retention need separating from live document ownership.
A source race was found: the one-second self-write suppression could expire
while a long save was still writing, starting a redundant full reload. The
fix holds suppression through foreground and background write completion and
passed four new lifecycle regressions; its whole-app effect needs measurement.
The later physical footprint was 1.8 GiB (peak 3.8 GiB). Reopening the saved
imported project and sustained edit cycles remain pending.

The earlier [release rehearsal](REHEARSAL.md) measured the 128 MB, 40,141-node,
31-page UI kit opening in 48.5–54.1 seconds. It settled at 1952–1960 MiB, then
rose from 1960 to 5788 MiB after one rectangle and 25 seconds for autosave.
Later samples partly receded; these observations do not demonstrate an
unbounded leak. Repeat the same fixture and timing after rebuilding, followed
by repeated edits, undo/redo, page changes, saves, close/reopen, and idle checks.

## Remaining release requirements

| Goal | Evidence and remaining verification |
| --- | --- |
| Backend and AI Gateway | Public health/plans and account/provider tests passed. Apply the backend migration and deploy reviewed changes; verify authenticated streaming, debit, errors, and sign-out against production. |
| Charge customers | Billing fixes and backend tests exist. Production Polar configuration, matching products/currency, checkout, webhook retry/cancellation/renewal, exactly-once credits, and billing portal remain unverified. No customer was charged. |
| Native generation | Source and 23 targeted tests are ready. Verify account registration/sign-in, real media jobs, progress/failure/cancel states, and inserting results into a saved design. Retry state/history lasts only for the tab lifetime; video playback uses the system player and canvas cards have no poster yet. |
| GPU service | Backend CI tests passed; deployed GPU availability and successful end-to-end generation are not established. |
| Design and Git UI | Synthetic creation/edit/save/reopen and app-driven review/stage/commit/push passed. Import/edit/undo/redo/save passed on the baseline fixture; repeat that import workflow on the final build. |
| Performance and UI quality | Repeat full-app large-file and sustained-memory tests. Review usable layouts and interaction feedback. A Figma comparison needs the same fixture, hardware, operations, and measurement method. |
| Installer | No valid local signing identity was found. Configure Developer ID/notarization secrets, build a signed DMG, verify notarization, and install/launch on a clean Mac. |
| Leads | US PostHog project 410640 and the existing landing page are accessible. Validate direct visit-to-signup reporting and durable contact handling; no improved conversion rate or outreach result is established. |

Do not publish a release or enable customer checkout on the strength of the
synthetic benchmark, public health checks, or the earlier green CI runs alone.
