# Release validation — 2026-09-10

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
Both checks also passed at Git-commit-fix source `bccc5fd`, verified at 21:34 UTC:
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
completed successfully at 22:22 UTC. Its downloaded checksum, embedded source
revision, and strict signatures passed. It precedes the Git commit-buffer and
memory corrections below. The `bccc5fd` installer has also completed and passed
downloaded checksums, embedded revision, strict ad-hoc signatures, and bundled
Git init/commit/push/fetch with an empty PATH. That package was not launched;
its read-only disk image was unmounted normally. Evidence is in
`/tmp/fanta-release-qa-20260909/github-bccc5fd/`.
Both checks passed at documentation head `95bbef4` and callback/list memory-fix
source `96d366a`. The queued `96d366a` installer was superseded; the
[4a8f3d9 installer](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34419050153)
completed successfully. Both hosted checks at `20d7108` now pass, including
the Save As/font, generation recovery/contract, and sidebar-selection corrections:
[push checks](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34423710801) and
[pull-request checks](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34423713177).
Its [installer](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34423710706)
is building. The subsequent Path Selection changes still need hosted checks and
a matching installer. Running installers have not been canceled or restarted.
The last local native build passed in
**5m45s**, and Save As passed copy/adoption, original-file preservation,
autosave recovery, and rejection without opening the destination tab. The
font correction at `8ab306a` has completed local allocation review below.
Hosted checks through `20d7108` pass. Path Selection still needs hosted checks,
and exact installer validation remains pending.
No signed/notarized installer is validated.
An earlier native release build passed in **5m30s**, including the page-switch
inspector correction. Candidate app `dev.fanta.CandidateQA` (PID 93583)
reopened the saved large design and passed the selection checks below. The
preceding 7m19s build (PID 90909) passed fresh-import rendering, complete
bundled Git checks, and the measured large-design lifecycle. Earlier
observations are identified separately.
Native QA used the original checkout, including the user’s separate staged
edits; hosted CI validates the isolated PR branch. Retired QA sessions have
been closed. Cleaning the retired secondary Cargo target recovered about
49.24 GiB of disk space; the current local target and user-active installer
were preserved. Daily housekeeping is active at 04:00 local time, with Sunday
`cargo clean` deferred whenever builds, tests, or native validation are active.
The latest process check found only the user-active app and its helper, with
no zombie processes. The retired disk-image mounts were unmounted normally;
downloaded images and untouched app copies remain available.

The latest backend [CI run](https://github.com/jeanc18rlos/fanta-backend/actions/runs/34426211299)
passed at pull request head `a2bb2bb`, with **396 TypeScript tests**, **73
isolated GPU tests**, type checking, and Docker. This source is deployed.
Authenticated production checks below establish the default Sonnet stream,
credit metering, mock-model rejection without billing, and key revocation.
They do not establish deployed GPU readiness or final native authentication.

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
| `cargo test --locked --offline -p fanta-doc --lib` | 336 passed; 1 explicit benchmark ignored, including conservative curve bounds |
| `cargo test --locked --offline -p fanta-tools --lib` | 258 passed, including 16 Path Selection regressions and existing node-edit tools |
| `cargo test --locked --offline -p fanta-format` | 181 passed, including integration tests and doctests; 2 ignored |
| Focused Rust formatting and `git diff --check` | Passed |
| `.github/workflows/check.yml` with actionlint 1.7.12 | Passed; now includes `cargo test --locked -p fanta-doc -p fanta-format` |
| `cargo build --locked --release -p zed --bin fanta` | Final Save As/font build passed in 5m45s; native copy/adoption, data protection, rejection without destination-tab activation, autosave recovery, and reopen passed |
| Browser sign-in callback recovery | 2 passed; valid encrypted callbacks, invalid callback rejection, 10-minute deadline, retry and cancellation |
| Document suite | 36 passed, including Save As regressions; hidden active-page recovery with background/shape pixel assertions, bundled Git resolution, long-save watcher suppression, overlapping/failed/canceled saves, and entity release |
| Production language registration | 4 passed, including named Git Commit lookup and COMMIT_EDITMSG file recognition without a parser or language server |
| Bundled Git transport | 2 Rust regressions passed, including clone/push/fetch/pull without system Git and with a bad inherited `GIT_EXEC_PATH`; replay against actual Dugite passed with clean repository integrity checks |
| Inspector integration suite | 34 passed; real page switching clears off-page selection, preserves selection on the current page, updates the inspector, and leaves the old shape unchanged |
| Native toolbar interaction suite | 32 passed, including pointer focus, popup dismissal, input editing, and keyboard navigation |
| Native toolbar adapter suite | 15 passed, including Path Selection drag/undo/redo and the shipped canvas keymap; typing and clicking in search preserve the query, Escape restores shortcuts |
| Native generation/media suite | 37 passed, including backend string-seed submission/poll/replay responses, integer segmentation clicks, toolbar activation, catalog authorization, prompt-to-SVG, and scrollable inpainting controls |
| Sidebar suite with ownership, startup, and selection fixes | 137 passed, 6 existing failures. Three new selection regressions and the previously failing historical-thread selection test pass. The initial baseline was 128 passes and 8 failures; the suite is not fully green. |

The local CI workflow includes the focused sidebar ownership regressions.
Six baseline failures remain; their exclusion from the focused check is not
evidence that they are fixed. The current CI also runs selection-remapping
regressions and the historical-thread activation check.

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
build is now in progress after `d0bc2d7` completed. The exact corrected installer still requires
validation.

The app command palette opens the native Image workspace with Image, Video,
Vector, Design, and Masks modes. The toolbar focus/keymap, ten-minute sign-in
callback, catalog authorization, and prompt-to-SVG fixes are included in the
latest native build. The production sign-in page previously identified its
Clerk instance as Development mode; production account configuration still
needs verification. An earlier native sign-in succeeded with the existing
Google session. The later production account/catalog check encountered a
macOS Keychain approval that requires manual interaction; the automation
tool cannot approve it. That QA process has since been closed. A fresh
production sign-in and authenticated native verification remain required.

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
This soak did not identify ownership; the subsequent stack-attribution work
below found and corrected two specific leaks. The soak supports document release for
this workflow, not universal leak freedom, sustained editing stability, or
performance superiority to Figma.

Raw snapshots, parsed metrics, native close timestamps, fixture hashes, and
the content-excluded memory graph are in
`/tmp/fanta-release-qa-20260909/memory-soak-bccc5fd/`.

## Targeted native memory fixes — September 9, 22:29 UTC

A separate allocation-stack session on the small synthetic design identified
one leaked Open dialog callback/capture pair per invocation (112 bytes) and
a ThreadView scroll-handler cycle retaining its own ListState and SumTree
(1,488 bytes in the instrumented scan). The callback was passed by value
through Objective-C FFI, losing Rust's release; the scroll handler captured
a strong clone of its own list state.

The fix borrows copied callbacks at seven macOS registration sites and reads
the list state through a weak ThreadView in the deferred scroll callback.
A standalone real-Foundation regression harness failed both capture-release
checks with the old ownership and passed both after the fix, including
executed and discarded callbacks. The full native release build passed in
10m27s. Its QA bundle SHA-256 is
`e62282f7ae238d30a3c0994c1412ba76f5f1d1a0dea4946af8261f343e355437`.

The fixed app (PID 18114) used a new stateless profile and allocation-stack
logging. After a warm-up, two project open/render/Close Project cycles and
two Open cancellations completed normally. New Design's native Save dialog
was cancelled once and then used to create a separate blank QA design.
Presenting that design produced the expected native no-frames alert, which
was dismissed before closing the project. The original fixture's 15 files
remained byte-for-byte unchanged, and the app quit normally with exit 0.

Independent content-excluded offline scans found neither targeted retain
cycle in any after-fix snapshot, including the final Save/alert snapshot.
No leaked Rust Open, Save, or native-alert callback signature was found.
The final scanner still reports 316 allocations / 20,320 bytes, including
startup AppIntents/XPC and semver findings plus font/accessibility findings.
An AppKit alert completion frame appears in an accessibility allocation
stack; this is not the traced Rust callback cycle and remains unresolved.
These small findings do not establish overall memory stability or explain
the larger residual from the earlier large-design soak. Screen-capture and
URL-handler registration changes share the tested ownership pattern but
were not exercised through their native permission flows.

This is local candidate validation with preserved user worktree edits,
not exact-installer validation or a performance comparison. Evidence,
fixture hashes, native event timestamps, and the independent review are in
`/tmp/fanta-release-qa-20260909/memory-fix-native-validation/`.
The before-fix traces are in
`/tmp/fanta-release-qa-20260909/leak-attribution-bccc5fd/`.

## Save As automated and native validation — September 10

Save As on an existing design was a silent no-op. The correction copies the
current document and assets to a valid destination, updates shared views and
project entries, and preserves the original. Rejected destinations do not
overwrite existing content, and autosave resumes on the source design. Native
and custom pickers now receive an initial directory beside the original
project; unrelated picker defaults remain unchanged.

The `fig_viewer` `save_as_` filter now passes **9 tests**, including the
correction that validates an occupied destination before opening its worktree.
That new regression also passed **20 scheduler seeds**. An earlier run of the
other GPUI cases passed its own 20-seed sweep; the three ordinary Rust tests
are not scheduler tests. The full `open_path_prompt` suite
passed **9 tests**, with one existing Windows test ignored. The document suite
passed **36 tests**, and the existing autosave regression passed separately. CI includes both
`fig_viewer --lib save_as_` and `open_path_prompt --lib save_as_` filters.

Evidence is in `/tmp/fanta-release-qa-20260909/save-as-validation/`:
`focused-tests-5-prevalidation.log`, `prevalidation-seed-sweep.log`,
`save-as-seed-sweep.log`, `all-picker-tests.log`,
`document-regressions.log`, and `autosave-regression.log`, with the updated
`verification-status.json`.

The first combined native release build passed in **5m29s**. In the isolated QA
profile, Save As suggested a sibling `Original Copy` destination. Cancel
created no copy and preserved the original. Successful Save As adopted the
copy; a subsequent edit/save and close/reopen preserved its changed geometry.
All **15 original file hashes** remained unchanged, and reopening the original
showed its unchanged rectangle. Selecting an occupied destination produced a
visible rejection; a later edit to the source copy autosaved after the error.

That first run exposed unwanted destination-tab activation after rejection.
The correction validates the destination before opening its worktree and
passed the nine-test suite and new 20-seed regression above. The final native
build then passed in **5m45s**. Rejecting the occupied `Original` destination
left exactly one `Original Copy` tab after dismissing the alert, with its path
unchanged. Changing the copy's height from 144 to 160 autosaved without Cmd-S;
all **15 original hashes** remained unchanged. A valid Save As to
`Verified Copy` adopted that path, and Close Project/reopen restored the
**208×160** design in both canvas and inspector. The QA process quit normally
with exit code 0.

Evidence is in `native-prevalidation-events.json`,
`native-prevalidation-integrity.json`, and `build-provenance-prevalidation.json`;
`native-events.json`, `native-fixture-integrity.json`, and `build-provenance.json`
retain the earlier observations. Both local builds include preserved user
changes and are not exact GitHub installers.
**Hosted checks through `20d7108` pass; exact-installer verification remains pending.**

## Font ownership reproduction — September 10

A standalone CoreText harness on macOS 26.6.2 arm64 reproduced the descriptor
array ownership sequence in pinned `zed-font-kit` / `core-text 21.0.0`.
Adding the extra retain used by the wrapper left **16 arrays / 1,280 bytes**
after 16 queries and **128 arrays / 10,240 bytes** after 128 queries. Matched
Create-ownership controls left zero scanner leaks, as did the zero-operation
baseline. Each process first performed four balanced warm-up operations.
All processes exited normally.

The allocation stack matches the native font-array finding, but the harness
queries one system family and reproduces the ownership operations directly;
it is not the full Rust dependency or app. Its 80-byte arrays also differ
from the native 320-byte array. Raw retain counts alone were not used as
proof. The evidence supports correcting the ownership wrapper, without
establishing supported-OS behavior or whole-app leak freedom.

Source, logs, graphs, and reproduction instructions are in
`/tmp/fanta-release-qa-20260909/coretext-ownership-repro/README.md` and
`report.json`. The full macOS `text_system::tests` suite subsequently passed
**6 tests**, including face ordering, missing-family behavior, and virtual
family/cached selection, in `save-as-validation/font-tests.log`. Two additional
`open_type::tests` passed after the feature-array ownership correction:
retained tag/value ownership and a usable font after applying features,
including glyph lookup and valid metrics. These tests **do not verify feature
shaping**, such as ligature substitution. Their log is
`save-as-validation/font-feature-tests.log`.

CI runs both `text_system::tests` and `open_type::tests` with
`cargo test --locked -p gpui_macos --features font-kit --lib`.

The independent review of both saved native snapshots from the **5m29s**
build found neither the prior CoreText matching-descriptor array signature
nor the previously fixed chooser/save/alert callback and ThreadView/ListState
signatures among scanner roots. The final closed-project snapshot still
flags **314 blocks / 19,936 bytes across 20 roots**, largely framework XPC and
accessibility graphs plus one app-version allocation; their ownership remains
unresolved. A framework alert-completion stack remains distinct from the
former Rust callback cycle.

The first snapshot was captured after opening a design and starting Save As,
so it is not a clean startup baseline. The earlier app run used different
operations; its raw totals are not a controlled before/after comparison.
These findings do not establish leak freedom, feature shaping, supported-OS
coverage, or long-duration stability. The snapshots also precede the separate
Save As focus correction. Evidence is in
`save-as-validation/independent-memory-review.md` and `.json`.
**Exact installer validation remains pending.**

## Generation network recovery — September 10

Generation API requests now have a 125-second deadline covering the response
headers and body, accommodating the backend's 120-second submission limit.
Media uploads and downloads have a separate five-minute deadline. An uncertain
submission retains its original request and idempotency key so Retry same
request can recover the existing job. Failed downloads preserve the job,
history, output, and existing destination file. Successful save/play/place
retries clear the previous transfer error.

The generation filter passed **33 tests**. Six new stalled-header and
stalled-body regressions passed **20 scheduler seeds each**, including same-key
submission retry, download/save retry, and refusing to complete an unconfirmed
upload. These tests use fake HTTP responses and GPUI's clock; they do not prove
production inference or billing. Evidence is in
`/tmp/fanta-release-qa-20260909/generation-timeout-validation/`.
Final hosted checks and exact-installer verification remain required.

## Live waitlist correction — September 10

Landing commit `4e6c9c9` fixes silent address truncation, false confirmation
from malformed success responses, and confirmation of an address edited during
submission. All **15 offline tests**, TypeScript, focused lint, and the production
build passed. Both hosted checks passed (`34419688177` and `34419692662`).
Deployment `dpl_7SD2uTp6M2W37XABjH7qm2ynSGnU` is live on
`https://www.fantaisa.net/`. Promotion exceeded the CLI observation deadline;
the subsequent promotion status and production deployment lookup confirmed
success. No duplicate promotion was started. The protected candidate was not
browser-verified; public browser verification followed promotion.

The live form rejected an overlong address, disabled editing while submitting,
and confirmed the exact reserved test address
`release-qa-20260910-001@fanta.invalid`. The test uses campaign
`signup_validation_20260910` and source `release_qa`; it is not a customer lead.
Three invalid API cases returned HTTP 400. Only PostHog is listed as the
configured production sink. A fresh Google sign-in to the recovered US project
then retrieved the saved contact with its exact email, `waitlisted=true`,
placement, and campaign attributes. Its `waitlist_joined` event has matching
submission ID `dabc9eba-4c5c-4914-8db1-3d850df95790` and timestamp
`2026-09-10T00:11:31.546Z`; one canonical signup event appears in the loaded
person history. Basic signup-to-stored-contact verification passed. Evidence
is in `/tmp/fanta-release-qa-20260909/waitlist-live-verification.json` and
`waitlist-live-invalid-cases.json`. Exclude this synthetic record from customer
lead counts. No conversion improvement is established, and saved dashboard
configuration was not changed.

## Backend account contracts — September 10

Backend `6fae7a8` corrects two reproduced defects. The desktop account endpoint
now selects the current subscription using the same rule as billing, instead
of returning dates from an older canceled row. Dashboard sessions now select
the signed-in user's membership role; a member can no longer inherit the first
owner row's billing or API-key administration permissions.

All **369 tests across 42 files**, type checking, and
[GitHub CI](https://github.com/jeanc18rlos/fanta-backend/actions/runs/34421837511)
passed, including the hosted GPU suite and Docker build. The committed source
passed the pinned Next.js 16.2.10 Turbopack production build and generated type
validation on Vercel. Candidate and post-promotion public checks passed all
nine cases. Deployment `dpl_7snZZNo5pfUkroT4axaGNPQZT9ee` is live at
`https://api.fantaisa.net`; no new migration or payment configuration was used.
A fresh browser sign-in after promotion displayed Pro, Owner, and 389 credits,
with checkout explicitly unconfigured. The member-denial tests used isolated
database fixtures; no production membership was changed.

Supplemental local Webpack validation exposed pre-existing generated page and
route-wrapper type incompatibilities; default hosted Turbopack and Docker
builds passed. No Webpack workaround, configuration weakening, or unrelated
Gallery change was retained. The multi-organization selection gap identified in this deployment is
addressed by the subsequent `2ef667b` correction below. Evidence is in
`/tmp/fanta-release-qa-20260909/account-contract-verification.json`.

## Native generation wire compatibility — September 10

The backend returns generation seeds as text, while the native response type
previously required an integer. Completed submission, polling, and same-key
replay responses could therefore fail decoding after successful generation.
The desktop now preserves text seeds exactly, including values beyond numeric
precision limits, while continuing to accept numeric, null, and omitted seeds.

Segmentation preview clicks previously sent fractional source coordinates to
workers that require integer pixel indices. Clicks now map to their containing
source pixel, and requests serialize integer coordinates. Off-grid clicks,
right/bottom edges, negative/outside clicks, and invalid coordinates are covered.

The new and extended cases failed **8 checks before the correction**; all
**37 generation checks now pass**. Two new submission/poll tests also passed
20 scheduler iterations each. These use backend-shaped HTTP fixtures and the
GPUI clock, not production inference. Evidence is in
`/tmp/fanta-release-qa-20260909/generation-contract-validation/`.

## Sidebar selection identity — September 10

When an empty draft disappeared during thread activation, selection retained
its old row index. The highlight could move to another thread and the next
Enter could activate it. Selection now follows the surviving thread, terminal,
or project identity through list rebuilds. Removing the selected item falls
back to the nearest remaining row; an absent selection stays absent.

Three new regressions cover real draft disappearance and Enter activation,
selected-row deletion, and a project-header shift with changed layout. They
failed against the prior implementation and now pass. The existing historical
thread selection failure also passes. One new header fixture initially assumed
the wrong group order; it was corrected to assert actual ordering and a real
one-row shift, and then reproduced the prior bug independently. Repeated
execution also exposed shared test-database state between scheduler iterations;
the header fixture now uses a separate database each time. The four selection
tests passed **20 scheduler iterations each** with all assertions retained.

The full suite now reports **137 passes and 6 baseline failures**, improved
from 133 passes and 7 failures before this correction. The remaining failures
involve draft/focus/async fixture expectations and are preserved for follow-up;
they have not been declared fixed. Evidence is in
`/tmp/fanta-release-qa-20260909/sidebar-selection-validation/`.

## Production device API verification — September 10

Normal browser-confirmed device authorization passed against live backend
`6fae7a8` at 01:11 UTC. A temporary credential was held only in process memory;
it was never written to a file or read from the user's Keychain. The account
resolved to the existing Pro owner with 389 credits and a 75-model catalog.

One streaming request to the native default `claude-sonnet-5` returned HTTP 200,
the exact requested `FANTA_GATEWAY_OK` text, and a final `message_stop` event.
The response reported `anthropic/claude-sonnet-5`, with 32 input and 16 output
tokens. Request `req_5e8rah7sXUwPbN7g` consumed one existing credit: the credits
API changed from 389 to 388, and the usage API and browser dashboard both
showed one request and one credit spent. No checkout or payment was performed.

The temporary key was revoked immediately after verification; a subsequent
account request returned 401. This proves the live device-authorization,
catalog, default-model stream, metering, and revocation API path. It does not
prove the final native application's sign-in, Keychain persistence, media
generation, or all models. Evidence is in
`/tmp/fanta-release-qa-20260909/production-device-verification.json`.

## Organization selection and removed-member access — September 10

Backend `2ef667b` aligns dashboard, browser API, device, and native sign-ins on
the user's active membership, falling back to their oldest valid organization
and then organization ID for ties. A stale or foreign active pointer cannot
grant access. Existing API keys retain their original organization; a new
native sign-in is required after choosing another organization.

Keys now require a current membership in their owning organization. Previously,
removing a member left their API keys able to spend that organization's credits.
Derived search tokens also revalidate the originating key's ID, user, org,
scope, expiry, revocation, and agent budget, so a signed token no longer retains
access after its key or membership is invalidated.

Four organization regressions reproduced the wrong account selection; ten new
authorization checks failed before the access correction, while the valid-member
control passed. All **384 tests across 42 files** and ordinary type checking now
pass. A native fixture's expected Team enum was corrected to the existing
`zed_business` contract; the baseline also demonstrated the wrong organization.
The prior local supplemental Webpack generated output was archived outside the
source tree before type checking, with compiler settings unchanged.

[GitHub CI](https://github.com/jeanc18rlos/fanta-backend/actions/runs/34424686656),
including Docker and GPU tests, and the Vercel production build passed.
Deployment `dpl_Cj8VnhhFguMmjU2qFe9yVpEYKTbp` was promoted; all nine public checks
passed before and after promotion. The signed-in billing page shows Pro, Owner,
and 388 credits, with checkout unconfigured. No migration, new credit grant,
or billing configuration was used. Evidence
is in `/tmp/fanta-release-qa-20260909/account-org-validation/`.

## Production mock-model billing guard — September 10

Backend `a2bb2bb` blocks synthetic inference in production, including model
aliases, organization overrides, REST/MCP discovery, direct chat routers, and
workflow dispatch. Offline test mocks still work. Existing completed generation
retrieval remains available; pending compose jobs that require synthetic workers
stay unavailable rather than fabricating output. No model seed or database
mutation was used to hide the defect.

The tests-only baseline reproduced 11 failures, with one existing-generation
recovery control passing. All **396 backend tests** and type checking pass.
[GitHub CI](https://github.com/jeanc18rlos/fanta-backend/actions/runs/34426211299),
Docker/GPU checks, the production build, and nine public checks before and after
promotion passed. Current deployment is `dpl_7YZNhCekZv7wGpXgZFrivLsB2FTT`.

A fresh browser-confirmed device API check at 01:48 UTC verified 73 callable
models in both REST and MCP, with no mock entries. Buffered and streaming
`mock-chat` requests and `mock-image` returned 400; the credit balance remained
388. A single valid default Sonnet stream then returned the exact requested
text and completion event, reported 32 input/16 output tokens, and debited one
credit (388 → 387). The usage API recorded one billed request in that check's
window; the browser dashboard showed the two cumulative Sonnet checks and two
credits spent. The temporary key was revoked and the next account call returned
401. Credentials stayed in process memory. This is API evidence consistent
with the configured Gateway route, not an independent network-hop trace or
proof of the final native app, all models, real media jobs, or customer checkout.
Evidence: `/tmp/fanta-release-qa-20260909/production-mock-boundary/`.

## Path Selection and shared curve bounds — September 10

Path Selection now selects existing anchors, handles, or segment endpoints,
with transformed dragging, marquee/Shift selection, and atomic undo/redo or
cancellation. Selecting a segment preserves the authored curve rather than
inserting a point. Acquisition uses the scene's existing spatial index and a
screen-space tolerance, respects hidden/locked ancestors, clipping overrides,
and page scope, and excludes invalid transforms and boolean children. The
native toolbar now activates this tool. Scale and Text on Path remain placeholders.

Two original curve regressions exposed endpoint-only broad-phase bounds that
excluded the visible body of a curve. Shared `PathData::rough_bounds` now
includes finite control points as well as endpoints, using the Bézier convex
hull. This is conservative rather than a tight extrema calculation: shared
selection, group bounds, and sizing/layout extents may increase for curves.
Straight paths retain their extents. No private Path Selection cache or extra
whole-scene traversal was introduced.

The original tests-only baseline compiled with 13 failures and two controls
passing; the native toolbar regression failed at Select versus PathSelect.
The first implementation left two curve tests failing (255 passes). The bounds
revision's tests-only baseline then reproduced two document failures (334
passes, one ignored benchmark). Its transformed scene/index regression passed
as a control before and after. One added clipping fixture initially failed to
compile because its JSON metadata used the wrong API; after that fixture-only
correction it reproduced the expected selection failure before the runtime fix.
All these logs remain preserved.

Final results: **336 document tests passed** with one explicit benchmark ignored;
**258 editing-tool tests passed**, including all 16 Path Selection cases; and
**15 native toolbar tests passed**. The new toolbar-to-canvas drag/undo/redo
regression passed **20 scheduler iterations**. Focused Rust formatting,
actionlint, and diff checks passed. GitHub now includes the full editing-tool
suite and the new native regression. These are automated GPUI tests; no new
standalone app was opened for this batch. The next exact installer still needs
visual curve/selection/layout checks. Evidence and guarded before/final source
hashes: `/tmp/fanta-release-qa-20260909/path-selection-validation/`.

## Remaining release requirements

| Goal | Evidence and remaining verification |
| --- | --- |
| Backend and AI Gateway | Backend head `a2bb2bb` is green in CI and deployed, with all 396 tests passing. Account selection, removed-member/key access, and production mock-model billing guards are included. The earlier additive migration remains verified. Nine public checks passed; unauthenticated chat returned 401 and billing CORS preflight returned 204. Browser-confirmed device API sign-in, catalog discovery, default Sonnet streaming, one-credit debit and key revocation passed in production. Repeat these checks through the final native installer; Keychain persistence and native account-error handling remain unverified. The earlier native QA session was closed. |
| Charge customers | The billing page still displays an explicit checkout-not-configured notice and no purchase or manage buttons. The latest API/dashboard check shows Pro/Owner and 387 credits after two one-credit AI verification requests. Pricing/product/currency selection and production Polar configuration remain blocked. Checkout, webhook retry/cancellation/renewal, exactly-once credits, and billing portal remain unverified. No purchase or customer charge was made. |
| Native generation | 37 generation tests passed, including string-seed submission/poll/replay and integer segmentation coordinates. Six network-timeout regressions passed 20 scheduler seeds each; the two new submission/poll checks also passed 20 each. Requests and transfers now time out without losing their recovery state. Local fixture sign-in/catalog, image polling/gallery/save/place, masks/background removal, editable SVG preview/place/save, and MP4 poll/play/place/save passed, with saved assets/layers verified. Final native checks also passed visible prompts, per-mode drafts, source-point selection, mask-to-inpaint source restoration, scrolling, and inpaint completion. Verify real media requests in production. Retry state/history lasts only for the tab lifetime; video playback uses the system player and canvas cards have no poster yet. |
| Additional canvas tools | Path Selection is implemented and passes 258 tool tests, 336 document tests, 15 native toolbar tests, and a 20-iteration native regression. Shared curve bounds are now conservative; verify curved selection/layout visually in the final installer. Scale and Text on Path remain explicit placeholders. |
| GPU service | Backend CI tests passed. Production HMAC access remains blocked; deployed GPU availability and successful end-to-end generation are not established. |
| Design and Git UI | Synthetic creation/edit/save/reopen and app-driven review/stage/commit/push passed. Final native import/edit/undo/redo/save and a second reopen/edit/save/close passed for the 29,301-node fixture. The candidate now renders a fresh 128 MB UI-kit import and Grid/Icons page changes; a visible green fill edit, Undo/Redo, and saved source passed. Complete bundled Git verification passed. The final native inspector check passed, including same-page selection preservation, clearing on page changes, and unchanged saved source. The Save As correction passed 9 automated regressions, a 20-seed destination-prevalidation regression, an earlier GPUI sweep, and the custom-picker suite. Native cancel, sibling default, copy adoption/edit/reopen, original-file preservation, occupied-destination rejection, and autosave recovery passed. The final 5m45s build also passed rejection with one unchanged source tab, post-error autosave, and valid Save As/reopen of a 208×160 copy. Hosted checks through `20d7108` pass; exact-installer verification remains pending. |
| Performance and UI quality | The repeated native lifecycle above released document-sized allocations in one warm-up plus four measured cycles. After the final five-minute idle, live malloc was 33.3 MiB and physical footprint 218.9 MiB; a 6.3 MiB residual above warm-up remains unattributed. The offline leak scanner flagged 340 allocations totaling 23,120 bytes. Earlier editing/UI-kit checks also released document-sized allocations. Two specific callback/list retain cycles were subsequently fixed and absent from repeated native after-fix scans. The previously observed descriptor-array scanner signature is absent after the later font correction; its final native snapshot still flags 314 blocks / 19,936 bytes across 20 roots. Other scanner findings remain unresolved. No leak-free or Figma-performance claim is established. Test sustained edits under controlled conditions. |
| Installer | Developer ID Application certificate `ML3GCBU926` for team `SP6J7Q6M3J` was issued/downloaded, with private-key match and G2 certificate chain verified; it expires 2031-09-10. GitHub secret names `MACOS_CERTIFICATE` and `MACOS_CERTIFICATE_PASSWORD` were verified after setting them at 17:57 UTC. No local keychain import was performed. The App Store Connect API terms modal awaits explicit user approval before notarization-key generation. Then build a signed/notarized DMG and install/launch it on a clean Mac. |
| Leads | Landing head `4e6c9c9` is live as `dpl_7SD2uTp6M2W37XABjH7qm2ynSGnU`; both hosted checks passed. All 15 offline tests, type checks, lint, and the production build passed. The live form rejected an overlong address, disabled edits while submitting, and confirmed the exact test address. Its stored contact and canonical `waitlist_joined` event were retrieved in US PostHog project 410640 with matching submission ID and campaign attribution. Basic signup-to-stored-contact verification passed using one reserved-domain QA address. Exclude that record from customer counts; no conversion improvement or completed outreach campaign is established. |

Do not publish a release or enable customer checkout on the strength of the
synthetic benchmark, public health checks, or the earlier green CI runs alone.
