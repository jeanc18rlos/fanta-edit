# Release validation — 2026-09-09

**The customer release is not yet validated.** This records completed checks
and the remaining work for the Apple Silicon alpha. Configuration and launch
instructions are in [LAUNCH.md](LAUNCH.md).

## GitHub and local checks

Desktop push and pull request CI passed at `5181599` and `b0523fe`; the latter
runs are [34384719415](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34384719415)
and [34384726796](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34384726796).
Both desktop CI runs passed at inspector-fix source `d0bc2d7`:
[push checks](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34392882554)
and [pull-request checks](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34392886767).
Both checks also passed at current source `bccc5fd`, verified at 21:34 UTC:
[push checks](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34404096365)
and [pull-request checks](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34404101640).
The first unsigned `0a2ae52` installer was uploaded by
[34390500529](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34390500529)
and downloaded for package verification. The published SHA-256 matched, as
did deep/strict ad-hoc signature checks and isolated bundled-Git checks. An
unchanged copy launched, created a design, drew a visible 176×120 rectangle,
and saved `page.fnx`. Fanta Sonnet and the Fanta sign-in prompt were visible.
This used a stateless QA profile and does not establish session persistence.
No production credentials or paid calls were used. It lacks the final
inspector-selection fix; the matching `d0bc2d7`
[installer build](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34392882537)
is now running. Both artifacts precede the subsequent Git commit-buffer
registration correction below. No signed/notarized installer is validated.
The latest native release build passed in **5m30s**, including the page-switch
inspector correction. Candidate app `dev.fanta.CandidateQA` (PID 93583)
reopened the saved large design and passed the selection checks below. The
preceding 7m19s build (PID 90909) passed fresh-import rendering, complete
bundled Git checks, and the measured large-design lifecycle. Earlier
observations are identified separately.
Native QA used the original checkout, including the user’s separate staged
edits; hosted CI validates the isolated PR branch.

The backend [CI run](https://github.com/jeanc18rlos/fanta-backend/actions/runs/34380169196)
passed at pull request head `9d05908`, with **365 TypeScript tests**, **73
isolated GPU tests**, and type checking. These checks do not establish deployed
GPU readiness or successful production AI requests.

Production migration `0018_subscription_event_order` was applied at 20:13 UTC
on September 9 after all 18 preceding migration timestamps and SQL hashes
matched the committed files. The resulting 19-entry history and nullable
subscription-event timestamp were verified; no seed was run. Backend
`9d05908` was built from committed source as a separate production candidate,
passed nine public smoke checks, and was promoted to `https://api.fantaisa.net`.
Those checks passed again on the live domain. Account, upgrade, and trial links
previously returned 404 and now redirect to dashboard/billing. Database reads,
missing-credential rejection, and malformed sign-in handling passed. These
checks do not authenticate a customer or incur AI/payment charges.

Local validation results:

| Check | Result |
| --- | --- |
| `cargo test --locked --offline -p fanta-doc --lib` | 333 passed; 1 explicit benchmark ignored |
| `cargo test --locked --offline -p fanta-format` | 181 passed, including integration tests and doctests; 2 ignored |
| Focused Rust formatting and `git diff --check` | Passed |
| `.github/workflows/check.yml` with actionlint 1.7.12 | Passed; now includes `cargo test --locked -p fanta-doc -p fanta-format` |
| `cargo build --locked --release -p zed --bin fanta` | Latest build passed in 5m30s; native inspector navigation passed, following the 7m19s build’s rendering and complete bundled Git checks |
| Browser sign-in callback recovery | 2 passed; valid encrypted callbacks, invalid callback rejection, 10-minute deadline, retry and cancellation |
| Document suite | 31 passed; hidden active-page recovery with background/shape pixel assertions, bundled Git resolution, long-save watcher suppression, overlapping/failed/canceled saves, and entity release |
| Production language registration | 4 passed, including named Git Commit lookup and COMMIT_EDITMSG file recognition without a parser or language server |
| Bundled Git transport | 2 Rust regressions passed, including clone/push/fetch/pull without system Git and with a bad inherited `GIT_EXEC_PATH`; replay against actual Dugite passed with clean repository integrity checks |
| Inspector integration suite | 34 passed; real page switching clears off-page selection, preserves selection on the current page, updates the inspector, and leaves the old shape unchanged |
| Native toolbar interaction suite | 32 passed, including pointer focus, popup dismissal, input editing, and keyboard navigation |
| Toolbar input with the shipped canvas keymap | 1 passed; typing and clicking in search preserve the query, Escape restores shortcuts |
| Native generation/media suite | 27 passed, including actual toolbar activation, catalog authorization, prompt-to-SVG, visible prompt rendering, accurate source clicks, and scrollable inpainting controls |
| Sidebar suite with ownership and startup fixes | 133 passed, 7 failed; baseline was 128 passed with 8 failures. Four new regressions and the previously failing selection invariant (20 generated cases) pass; the suite is not fully green. |

The local CI workflow includes the focused sidebar ownership regressions.
Seven unrelated baseline failures remain; their exclusion from the focused
check is not evidence that they are fixed.

New tests cover snapshot isolation through node/hierarchy edits and unchanged
JSON serialization. A GPUI test also covers closing the visible assistant
while the center document has focus, reopening it, and restoring document
focus. It passed locally (1 test; 0.37s) with:

```sh
cargo test --locked --offline -p agent_ui --lib test_toggle_closes_visible_agent_panel_when_center_pane_has_focus
```

Backend coverage includes early idempotent recovery, Gateway planner routing,
worker source fixes, and safe validation-error formatting. The backend is
deployed; authenticated end-to-end verification remains pending.

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

The subsequent packaging fix includes the complete Dugite Git 2.53.0
archive, with its published SHA-256 and nested ad-hoc signatures verified.
Checks with an empty `PATH` passed public HTTPS access and Git LFS/Credential
Manager startup. Two Rust transport regressions passed; replay against actual
Dugite covered clone, push, fetch, and pull with no system Git and a deliberately
invalid inherited `GIT_EXEC_PATH`. Repository integrity checks were clean.
The rebuilt 410 MB candidate bundle passed deep, strict ad-hoc signature
verification and public HTTPS access with an empty `PATH`. Developer ID signing,
notarization, and a clean-Mac installer check remain pending. The release
workflow requests unsigned test artifacts for app code, assets, Cargo/build
scripts, and packaging changes on `codex-release-backend-gateway`. Docs-only
changes do not rebuild; these branch runs do not create customer or draft
releases.

The exact GitHub artifact logged `language not found` while initializing
its Git commit editor. Production language registration omitted Git Commit,
so loading the repository buffer, commit template and draft subscriptions
stopped early. The panel still had a placeholder editor; the logs alone do
not prove basic commits failed. The correction registers the existing Git
Commit file configuration without a parser or language server. Its regression
and the three existing language tests pass, and GitHub CI now runs this suite.
The corrected native build passed in 4m23s. In a stateless synthetic QA
project, the commit template loaded visibly, an edited draft survived closing
and reopening the commit dialog, and the native Stage All/Commit flow created
`ff7bf4a3a0b00e6ae525cee0fe4b88007ea10922` with 15 design files and a clean
working tree. No `language not found` error occurred in that session. No
production credentials or paid calls were used. Current source `bccc5fd` is
pushed; both GitHub checks passed, verified at 21:34 UTC, and its installer
build is queued behind `d0bc2d7`. The exact corrected installer still requires
validation.

The app command palette opens the native Image workspace with Image, Video,
Vector, Design, and Masks modes. The toolbar focus/keymap, ten-minute sign-in
callback, catalog authorization, and prompt-to-SVG fixes are included in the
latest native build. The production sign-in page previously identified its
Clerk instance as Development mode; production account configuration still
needs verification. An earlier native sign-in succeeded with the existing
Google session. The final production account/catalog check is currently
blocked by a macOS Keychain approval that requires manual interaction; the
automation tool cannot approve it. That approval remains pending for the
production-account app process (PID 76062).

In a separate local QA profile, the rebuilt native app signed in to a
loopback fixture and loaded its authenticated, visibly QA-labeled catalog.
The final fake-account profile required a unique `credentials_url` to keep
its keychain namespace separate. An earlier fake profile reused a namespace
and encountered a Keychain approval; no real credential was read or approval
bypassed.
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
source-click bounds 200 px below the preview. The fixes passed **27 generation
tests** and were verified in the final native app: typed prompt text was
visible; independent Image and Masks drafts survived switching away and back;
the whole source image fit in its preview; clicking it added one visible green
point. Mask submission returned HTTP 202 and completed through automatic
polling. Edit this mask restored the original source and Image prompt.
At a 1089×768 window size, scrolling exposed Generate and the inpaint request
completed with the mint-colored fixture result. Production checks remain
pending.

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

These earlier RSS observations did not demonstrate a whole-app memory
improvement. A source race was found: the one-second
self-write suppression could expire while a long save was still writing,
starting a redundant full reload. The fix holds suppression through
foreground and background write completion and is covered by the 31-test
document suite.

An earlier memory guard run (PID 76062) showed RSS falling to about 180 MiB
after Close Project. That did **not** demonstrate document memory release:
`vmmap` still reported about 2.1 GiB physical footprint, 13.2 million
allocations, roughly 1.5 GiB of live allocation bytes, and about 1.6 GiB
swapped. The low RSS reflected compression/swapping, not released ownership.
Earlier lower-RSS or heap-census observations must not be treated as evidence
that the close-project retention was resolved.

A strong `active_entry` reference in the sidebar was found retaining the
document. A regression using the real Close Project path and a `WeakEntity`
failed before the fix, then passed with the panel absent and with an empty
panel. The initial ownership fix reported 131 passes and the baseline's same 8
failures. A subsequent one-line synchronization of an already mounted agent
panel fixed the selection invariant (20 generated cases), bringing the suite
to 133 passes and 7 remaining failures. Four new regressions pass. The ownership
and startup fixes are included in the final native build.

In the final app process (PID 82910), a fresh import of `basic.fig` again
produced 29,301 nodes. Changing the fill to green, Undo to gray, Redo to green,
and Cmd-S all passed; the saved green value was verified in `page.fnx:21`.
Closing the project then released the document's live allocations in this
observed case:

| Final-build checkpoint | Physical footprint | Default malloc allocation count | Default malloc allocated bytes | Default malloc fragmentation |
| --- | ---: | ---: | ---: | ---: |
| Before import | 330.7M | 89,621 | 44.9M | 19.0M |
| After edit and save | 2.3G | 13,217,457 | 1.6G | 0K |
| After first Close Project | 526.5M | 109,818 | 61.3M | 89.6M |
| After first close and idle | 526.5M | 109,789 | 61.1M | 89.7M |
| After second Close Project | 959.0M | 117,891 | 70.2M | 308.0M |
| After second close and idle | 475.1M | 115,873 | 63.5M | 32.8M |

Values use `vmmap`'s reported M/G units and its `DefaultMallocZone` row, not
the all-zone totals. The source snapshots are
`/tmp/fanta-release-qa-20260909/import-final-{before,edited,closed,closed-idle,reopened,second-close,second-close-idle}.vmmap`;
timestamps and RSS samples are in `import-final-events.jsonl` in that directory.
RSS remained 3,261,456 KiB after close while empty malloc regions were retained,
so it was not a useful stand-alone measure of live document ownership. The
fall in allocated bytes and allocation count demonstrates release in this
case, unlike the earlier compression-only result.

The same final app process then reopened the saved project. The green edited
rectangle persisted. Default malloc grew to 13,220,424 allocations and about
1.5G live bytes while the document was open. A Right/Left edit, Save, and a
second Close Project completed; live allocations returned to 117,891 and
70.2M. Document-sized ownership was therefore released in both observed
cycles.

Immediately after the second close, physical footprint was 959.0M and RSS
was 4,519,232 KiB. The roughly
432.5M footprint increase was largely accounted for by 218.4M additional
default-malloc fragmentation and 201.6M of empty large allocations. IOSurface
(247.5M) and GPU allocations (28.1M) were unchanged between the close samples.
An idle sample at 20:22:46.110 +0200, 541.9 seconds after the second close,
without trimming or other intervention, showed footprint falling to 475.1M, with 115,873 live allocations, 63.5M allocated bytes, and
32.8M fragmentation in the default malloc zone. IOSurface and GPU allocations
remained unchanged. Automatic allocator reclamation reduced the temporary
post-close growth sometime within that sampling interval. No explicit
allocator-trimming change was warranted or implemented. These results do not establish leak
freedom, stable overall memory across sustained use, or performance relative
to Figma.

The earlier final app (PID 82910) opened a fresh private UI-kit copy of
**128,246,530 bytes**.
Canvas labels appeared within **16.2 seconds**, but this was not a usable full
render: the importer left `Internal Only Canvas` active with a hidden root, so
fills and newly created shapes were invisible. A green rectangle was saved to
`page.fnx:8871` despite its invisible canvas fill. Reopening the converted
project rendered Grid and Icons with their backgrounds and colored fills.
The loader now replaces a hidden active page with its visible default while
preserving hidden library content. Its regression failed before the fix and
now passes active-root and background/green-shape pixel checks as part of the
31-test document suite. The candidate native verification below now confirms
fresh import and page switching after this fix.

| UI-kit checkpoint | Default malloc allocation count | Default malloc allocated bytes |
| --- | ---: | ---: |
| After opening | 1,441,606 | 663.1M |
| After edit and save | 1,571,923 | 654.2M |
| After Close Project | 137,137 | 75.6M |

The immediate post-close footprint was 961.2M. These snapshots are
`/tmp/fanta-release-qa-20260909/import-ui-kit-{opened,edited,closed}.vmmap`.
They show document-sized allocation release for this observation, without
establishing sustained overall memory stability or full rendering correctness.

The rebuilt candidate (PID 90909) then opened another fresh 128 MB UI-kit
copy. Actual white Icons content was visible at the first observation
**14.544 seconds after Open**. Grid rendered immediately on selection, and
switching between Grid and Icons rendered both pages. A new Grid rectangle
changed from gray to `#22C55E` through the picker; Undo visibly restored gray
and Redo restored green. Save persisted the edit in `pages/grid/page.fnx:308`.
A later 5m30s build (PID 93583) fixed selection retained across page changes.
Native verification reopened the saved green shape, selected it, and clicked
the same Grid page: selection and the Rectangle inspector remained. Switching
to Icons cleared selection and showed the Page inspector; returning to Grid
kept the green shape intact and unselected. The saved page’s SHA-256 was
unchanged throughout navigation. All 34 inspector integration tests passed;
the new regression failed before the fix. Evidence is in
`/tmp/fanta-release-qa-20260909/candidate-page-selection-native.json`.

| Candidate UI-kit checkpoint | Physical footprint | Default malloc allocation count | Default malloc allocated bytes |
| --- | ---: | ---: | ---: |
| Before import | 122.9M | 68,917 | 19.5M |
| After opening | 808.7M | 666,370 | 507.5M |
| After edit and save | 999.6M | 684,136 | 514.6M |
| After Close Project | 724.0M | 106,995 | 39.1M |
| After close and idle | 301.0M | 106,869 | 38.9M |

Snapshots are
`/tmp/fanta-release-qa-20260909/candidate-ui-kit-{before,opened,edited,closed,closed-idle}.vmmap`.
The drop in live allocation bytes and count confirms document-sized heap
release in this cycle. An idle sample about 397 seconds later showed physical
footprint dropping naturally from 724.0M to 301.0M. This observation does not
establish sustained overall memory stability. The 14.544-second observation
is an upper bound from a single UI observation, not a controlled import
benchmark or a comparison with Figma.

The earlier [release rehearsal](REHEARSAL.md) measured the 128 MB, 40,141-node,
31-page UI kit opening in 48.5–54.1 seconds. It settled at 1952–1960 MiB, then
rose from 1960 to 5788 MiB after one rectangle and 25 seconds for autosave.
Later samples partly receded; these observations do not demonstrate an
unbounded leak. Neither the earlier 16.2-second label appearance nor the
candidate’s single 14.544-second rendered observation is a controlled comparison with that
rehearsal. Repeat sustained edits, page changes, close/reopen, and idle checks
under the same conditions.

## Repeated native document lifecycle

A later stateless native session (PID 10469, binary SHA-256
`ab43d5661495cb847e8f31c6e1f6a8e9dcd801d0101984c50b221be9dece3758`)
opened the same converted 29,301-node design once for warm-up and four more
times. Each open visibly rendered the same page, and File > Close Project
returned to the empty project. There were no edits, saves, process restarts,
or allocator-trimming calls during this sequence. The local binary includes
`bccc5fd` changes and the separately preserved user edits; this is not a test
of the exact GitHub installer or persistent workspace restoration.

Every open sample had about 1.5 GiB of live default-malloc allocations and
13.2 million allocations. The table shows samples approximately 60 seconds
after each close. Sizes use binary units and are rounded by `vmmap`.

| Close checkpoint | Live default-malloc bytes | Allocation count | Physical footprint |
| --- | ---: | ---: | ---: |
| Warm-up | 27.0 MiB | 96,091 | 1,843.2 MiB |
| Cycle 1 | 28.7 MiB | 98,278 | 215.8 MiB |
| Cycle 2 | 30.5 MiB | 100,962 | 265.8 MiB |
| Cycle 3 | 38.5 MiB | 106,680 | 265.7 MiB |
| Cycle 4 | 33.3 MiB | 105,686 | 218.9 MiB |
| Cycle 4 after 300 seconds | 33.3 MiB | 105,163 | 218.9 MiB |

Document-sized allocations were released on every close, with no swapped
malloc bytes in the close samples. The later decrease makes the residual
non-monotonic, but the final sample still has 6.3 MiB and 9,072 allocations
above the warm-up close. That residual is not attributed to a specific owner.
Physical footprint recovered while resident pages remained high; these
metrics must not be treated as interchangeable.

After all timed samples, a memory graph was captured with allocation contents
excluded. Offline Apple's `leaks` analysis reported **340 allocations totaling
23,120 bytes**, including NSXPCConnection cycles and anonymous cycles. There
is no pre-soak leak graph or allocation-stack history for attribution. This
finding does not explain the larger residual and is not a clean leak scan.
No speculative memory fix was applied. The run supports document release for
this workflow, not universal leak freedom, sustained editing stability, or
performance superiority to Figma.

Raw snapshots, parsed metrics, native close timestamps, fixture hashes, and
the content-excluded memory graph are in
`/tmp/fanta-release-qa-20260909/memory-soak-bccc5fd/`.

## Remaining release requirements

| Goal | Evidence and remaining verification |
| --- | --- |
| Backend and AI Gateway | Backend head `9d05908` is green in CI and deployed after its verified additive migration. Nine public checks passed; unauthenticated chat returned 401 and billing CORS preflight returned 204. Authenticated native streaming, debit, errors, and sign-out remain unverified against production. Final native account/catalog verification awaits manual Keychain approval. |
| Charge customers | The existing authenticated owner session displayed Pro and 389 credits after migration, with an explicit checkout-not-configured notice and no purchase or manage buttons. Pricing/product/currency selection and production Polar configuration remain blocked. Checkout, webhook retry/cancellation/renewal, exactly-once credits, and billing portal remain unverified. No purchase or customer charge was made. |
| Native generation | 27 targeted tests passed. Local fixture sign-in/catalog, image polling/gallery/save/place, masks/background removal, editable SVG preview/place/save, and MP4 poll/play/place/save passed, with saved assets/layers verified. Final native checks also passed visible prompts, per-mode drafts, source-point selection, mask-to-inpaint source restoration, scrolling, and inpaint completion. Verify real media requests in production. Retry state/history lasts only for the tab lifetime; video playback uses the system player and canvas cards have no poster yet. |
| GPU service | Backend CI tests passed. Production HMAC access remains blocked; deployed GPU availability and successful end-to-end generation are not established. |
| Design and Git UI | Synthetic creation/edit/save/reopen and app-driven review/stage/commit/push passed. Final native import/edit/undo/redo/save and a second reopen/edit/save/close passed for the 29,301-node fixture. The candidate now renders a fresh 128 MB UI-kit import and Grid/Icons page changes; a visible green fill edit, Undo/Redo, and saved source passed. Complete bundled Git verification passed. The final native inspector check passed, including same-page selection preservation, clearing on page changes, and unchanged saved source. |
| Performance and UI quality | The repeated native lifecycle above released document-sized allocations in one warm-up plus four measured cycles. After the final five-minute idle, live malloc was 33.3 MiB and physical footprint 218.9 MiB; a 6.3 MiB residual above warm-up remains unattributed. The offline leak scanner flagged 340 allocations totaling 23,120 bytes. Earlier editing/UI-kit checks also released document-sized allocations. No leak-free or Figma-performance claim is established; attribute the scanner findings and test sustained edits under controlled conditions. |
| Installer | Developer ID Application certificate `ML3GCBU926` for team `SP6J7Q6M3J` was issued/downloaded, with private-key match and G2 certificate chain verified; it expires 2031-09-10. GitHub secret names `MACOS_CERTIFICATE` and `MACOS_CERTIFICATE_PASSWORD` were verified after setting them at 17:57 UTC. No local keychain import was performed. The App Store Connect API terms modal awaits explicit user approval before notarization-key generation. Then build a signed/notarized DMG and install/launch it on a clean Mac. |
| Leads | Landing runtime `388e70d` is live as `dpl_HQURSm5XizjgZvvWUtPExb39tbd1`. Workflow head `e61e547` passed push/PR CI runs `34406236688`/`34406236734`. Seven analytics/navigation tests, nine initial-HTML checks, and desktop/mobile direct-hash and CTA checks passed with the complete waitlist form server-rendered. US PostHog project 410640 is accessible. No real lead was submitted; visit-to-signup reporting and durable contact capture still need end-to-end verification. No conversion improvement is established. |

Do not publish a release or enable customer checkout on the strength of the
synthetic benchmark, public health checks, or the earlier green CI runs alone.
