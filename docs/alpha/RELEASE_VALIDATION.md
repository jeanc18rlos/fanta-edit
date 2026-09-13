# Release validation — 2026-09-12

**The customer release is not yet validated.** This records completed checks
and the remaining work for the Apple Silicon alpha. Configuration and launch
instructions are in [LAUNCH.md](LAUNCH.md).

| Area | Current status |
| --- | --- |
| Backend and AI | `0d3753a` remains live. Earlier managed Sonnet streaming, metering and account checks passed. Latest source `9510f61` passes all four CI jobs, including 537 backend, 153 CPU worker and 17 real PostgreSQL cases; it is not deployed. Native production authentication, worker activation and real media remain unverified. |
| Customer payments | Paid-term gating and annual monthly allocations pass real database races. Checkout remains disabled pending plan/currency decisions, migrations, deployment and provider sandbox validation. |
| Editor and video | Both `17de5eb` hosted checks pass. Exact first-milestone commit `9d92b30` passes 646 viewer tests, 591 shared GPUI tests, three architecture checks, 11 media library tests, 11 native playback cases, and the package-scoped release Clippy gate. Its 7m14s release build was packaged under an isolated local QA identity. The three retained `9d92b30` toolbar screenshots are invalid because they show ChatGPT rather than Fanta, so that revision's Design/Motion/narrow layout claim remains unsubstantiated. A separate exact-`12e2e4c` local QA build supplies inspected Fanta captures for the current Design, Motion, and 1025-point narrow layouts. Other recorded `9d92b30` interaction checks covered toolbar Actions, signed-out Design AI drafting, Fill/Stroke visibility, Fill Undo, Linear gradient, and successful Export. Earlier trim/edit/save/reopen, subsecond counters, and one-click replay after end scrubbing also passed natively. Neither local bundle is the final signed/notarized artifact. |
| Current editor work | Validated source/runtime candidate `12e2e4c` includes the previously recorded Text on Path, active-root canvas attachment, contextual Motion keyframing, and Motion time-comment work. It adds guarded Auto key editing through Motion's At playhead fields for Position X/Y, Rotation, Scale X/Y, Opacity, and Fill color: edits preview live, commit as one reversible keyframe operation, and become autosave-eligible only after commit. Its source gate passes 737/737 viewer tests and package-scoped `./script/clippy -p fanta-gpui -p fig_viewer`, including Cargo Machete. The exact local release build passed the bounded toolbar and Auto key smoke below; Text on Path, attachments, time comments, the other Auto key properties, and light/font-scale variants still lack `12e2e4c` native coverage. No hosted artifact validates this combined slice. Direct-canvas auto-keying, Motion Path, Dev mode, and Voice input remain unavailable. |
| Leads | PostHog US access recovered; live waitlist storage/attribution verified. Exclude the synthetic signup. No outreach or conversion improvement is claimed. |
| Distribution | The exact `17de5eb` hosted Apple Silicon installer completed successfully at 13:52 UTC. Build/package, bundle and checksum verification, installer upload, and crash-symbol upload passed. Notarization and draft-release publication were skipped because the push build had no distribution credentials. Download/launch, notarization, Gatekeeper, clean-Mac validation, and automatic updates remain open. |
| Performance and cleanup | Daily 04:00 Madrid housekeeping is active. The owned trim QA sessions closed normally. Native closed-state memory retained about 5.5 MiB above warmup; attribution and matched controls remain open. |

The hosted installer was downloaded after completion. Its published SHA-256,
image CRC, embedded `17de5eb` revision, strict ad-hoc signature, arm64 app/CLI,
and bundled Git 2.53.0 passed non-launch inspection. The image was detached
normally and the user's existing Fanta processes remained unchanged. Evidence:
`/tmp/fanta-release-qa-20260909/github-17de5eb-hosted-installer/verification.json`.

## Text on Path direction and placement — September 13

The current source adds explicit Forward/Reverse controls and a Start offset
percentage measured along the current path. It keeps the existing raw segment
API available to other component hosts but no longer exposes it in Fanta's
inspector. Offset changes preserve path geometry, content, style runs,
alignment, direction, and side; direction changes preserve offset and side.
Disconnected paths cannot be switched with this control.

Preview, cancel, commit, and Undo use the existing content-preview boundary.
Returning to the rounded initial percentage restores the exact stored segment
position before commit; accepting that original value produces no history.
The source gate passes all 739 viewer tests, 596 shared UI tests, three
architecture checks, and scoped `./script/clippy -p fanta-gpui -p fig_viewer`
including Cargo Machete. Logs are under
`/tmp/fanta-release-qa-20260913/text-path-controls`. Native validation is pending;
this is not a release-artifact claim.


## Exact `9d92b30` toolbar milestone — September 10

> **Evidence correction — September 12.** The retained
> `native-design-toolbar.png`, `native-motion-toolbar.png`, and
> `native-toolbar-narrow.png` files all show ChatGPT rather than Fanta. They
> remain preserved for audit but are rejected as Fanta visual evidence. Fresh,
> immediately inspected captures from exact runtime candidate `12e2e4c`
> validate the current Design, Motion, and narrow-window layouts at
> `/tmp/fanta-release-qa-20260912/toolbar-correction-12e2e4c/verification.json`.

`CARGO_TARGET_DIR=/Users/jeanrojas/fanta-edit/target cargo build --offline
--locked --release -p zed` completed in 7m14s with embedded revision
`9d92b30d437ad7ce3cfa72b9843284d26cdce83f`. The unsigned build output had
SHA-256 `1df79b5bc5a1d06135a2a49b427d6b5bc6fae5193c7c4672e00167f5aa7292c2`.
The local QA bundle used identifier `dev.fanta.ToolbarMilestoneQA`, a fresh
`--user-data-dir`, and an ad-hoc signature that passed strict/deep verification;
its post-sign main-executable SHA-256 is
`63c86ec977baaa2b71bf9fcce06165f95217614dc5cae495e83d4d604f21bdc3`.
It reused the prior QA shell's unchanged CLI, Git, icons, and plist structure,
so it is native smoke evidence rather than an installer candidate.

The app created and saved a local project and recorded these interactions:

- The reported Design/Motion/narrow layout observation is unsubstantiated for
  exact revision `9d92b30` because its captures show the wrong app. The later
  exact-`12e2e4c` pass validates the current layout only: Design has one compact
  canvas-centered row, Motion has its contextual row above the timeline, and a
  1025-point window hides toolbar zoom while retaining Ask AI and centering in
  the available canvas.
- Selecting Ellipse from the shape flyout armed the tool without creating an
  extra node. Toolbar Actions returned Group/Ungroup, Frame Selection, Replace
  Content, Rewrite Text, Translate Text, and Rename Layers.
- A signed-out Ask AI suggestion populated an unsent local draft. No AI request
  was submitted and no production inference was used.
- Fill and Stroke visibility changed the selected rectangle. Undo restored the
  Fill. Switching Fill to Linear produced a finite two-stop gradient.
- Toolbar Export preserved sidebar state, showed a visible success notice, and
  wrote `Shape@2x.png`: a valid 418×354 RGBA PNG with SHA-256
  `8427c595a3682c42d5fb8c03823d0b195cb93cdb0c4d95f3fee4e1d60fd9539e`.

The QA app quit normally with exit code zero. Its bundle, isolated
profile/project, exported PNG, and rejected screenshots remain under
`/tmp/fanta-release-qa-20260909/toolbar-milestone-9d92b30`. The three image
files must not be cited as Fanta evidence; the corrected captures and their
hashes are under
`/tmp/fanta-release-qa-20260912/toolbar-correction-12e2e4c`.
Launching this stable-channel QA binary displaced the previously running Fanta
process even with a distinct bundle ID and profile. The candidate was therefore
closed after the pass and the prior untouched
`github-d0bc2d7/untouched/Fanta.app` was restarted with its original profile.
Do not run two stable-channel Fanta QA binaries in parallel.

## Current Text on Path, canvas context, Motion keyframing, Auto key, and time comments — September 12

Validated source/runtime candidate
`12e2e4c0c86230bf8555071acb76fdf985377d20` includes source-only candidate
`895b6b734e11253c3320918f60492cd1f0758a41`, which replaces the former Text on
Path placeholder with a conservative command: exactly one visible, unlocked
vector is replaced in place, preserving its node identity and wrapper metadata,
then opened for inline editing. The command accepts an unpainted path or one
normal solid fill or stroke and rejects rounded, clipped, degenerate, multiply
painted, gradient/image-painted, or blended vectors without changing the
document.
Text content, base style, and style runs edit through one commit while path,
start, alignment, direction, and side are preserved. Rendering places shaped
clusters along the path and exposes shared cluster-derived curved caret, hit,
selection-quad, containment, and tight visual-bounds geometry. Pointer affinity
and Left/Right/Home/End navigation follow visual bidirectional order. Caret
positions inside one shaping cluster containing multiple graphemes use equal
subdivisions rather than exact glyph-internal positions. Glyphless source
intervals are retained when Skia exposes cluster geometry; otherwise rendering
and edit geometry fail closed rather than inventing a caret interval.
Complex-script offsets, ligatures, bidirectional clusters, trailing whitespace,
and empty-text carets have passing source regression coverage.

The Design inspector identifies TextPath distinctly and permits document-backed
edits for name; font family, Regular/Italic style, weight, size, line height,
pixel letter spacing, start/center/end alignment, and underline/strikethrough;
side/orientation; and, in the September 13 source, Forward/Reverse direction and Start offset
as a percentage along the current path. Its
single synthetic glyph Fill permits RGB and opacity edits only; add, style,
visibility, remove, reorder, and forged non-color paint operations are gated.
Supported typography and color changes update the base style and its runs, and
flipping the side preserves direction. The shared typography UI still shows
disabled non-applicable controls, while the new direction and placement controls still need their native pass.

A completed phased start interaction commits as one undo step. While any
item-level content preview is active, autosave and direct persistence are
blocked, relevant watcher changes remain deferred instead of being adopted by a
merge/reload, and cancel or selection change restores and reprojects the
document. Preview completion re-announces the edit so persistence resumes. The
new persistence regressions pass in the current source gate. Color/bitmap glyphs
can be drawn and use conservative editing bounds, but they do not expose an
exact vector alpha silhouette; path-only inner-shadow and
background-blur effects are therefore skipped instead of approximated with a
rectangle.

When every selected node belongs to the active page or component root, the
toolbar attachment control captures an immutable attach-time JSON snapshot and
inserts it as a named embedded resource in an unsent Agent draft. The resource
includes exact node ids, an omitted count, and stable `document_id`, document
path, project root, `active_root_id`, scope kind, and bounded scope, node-name,
and font descriptions. It is capped at 64 included nodes and 128 KiB.
Mixed-root and off-root selections are rejected rather than labelled with the wrong source,
and the embedded usage contract requires matching the identity against
`design_state` before live ids are used. Rapid attachment requests queue until
the draft is ready. The user must review and submit it; no request is sent
automatically. With no canvas selection, the control opens the existing Agent
Add Context menu. The snapshot is not a live reference. Voice recording,
permission handling, transcription, and cancellation are not part of this
slice, and Dev mode's inspect, measure, annotation, and readiness controls
remain unavailable.

The contextual Motion Keyframe chip now opens the same Position X/Y, Rotation,
Scale X/Y, Opacity, and Fill color catalog as the production timeline. Choosing
a property samples the selected layer's resolved value and adds or replaces its
keyframe at the current playhead in one undoable transaction. Exact label
parsing rejects forged choices, and a content preview owned elsewhere blocks the
operation without changing motion state or history. The former duplicate
primary Motion flyout is absent; Move remains the primary selection face.

In validated runtime candidate `12e2e4c`, Auto key is implemented only for
Motion's At playhead property fields. With one editable selected layer, an
active clip, paused playback, and the Motion canvas open, Position X/Y,
Rotation, Scale X/Y, Opacity, and Fill color preview live and commit as one
reversible keyframe operation. Active previews block persistence; autosave
begins only after commit. Invalid or stale state fails closed, context changes
disarm Auto key, and returning a numeric field to its rounded initial display
restores the exact underlying value and track state without history or
autosave. Moving, resizing, rotating, or otherwise editing a layer directly on
the canvas does not auto-author keyframes.

The current source tree also replaces the Time comment placeholder. Invocation
captures the active page, clip, and exact integer-ms playhead, pauses playback,
and enters page-bound Comment placement. Posting stores the root comment's
motion anchor in one `SetMeta` history operation; one Undo removes it, Redo
restores it, and FNX write/reopen preserves it. Static comments remain visible
in every mode. Timed pins and ordinary overlays appear only in Motion on their
exact active clip. Opening a timed thread from Comments or Properties navigates
by stable page and clip identity, seeks to its time, and pauses without adding
history. If the clip duration later shrinks, navigation clamps to the new end
without rewriting the authored anchor. If the clip is deleted, explicitly
opening the thread keeps it accessible in the Canvas workspace, reports its
unavailable anchor, and does not select or retarget another clip.

Mode, workspace, tool, page, clip, timeline-time, playback, scope, and
source-lock changes clear an unplaced time-comment intent. Time-comment arming,
placement, and thread navigation refuse to replace an existing comment draft or
unsent reply; unrelated mode and tool actions can still intentionally cancel a
placed draft. Preview ownership, source locking, Save As, or a missing clip
refuses an unsafe post without losing the draft. Malformed or future comment
metadata is preserved rather than hiding valid comments or being overwritten by
a later mutation. Direct-canvas auto-keying and Motion Path remain
unimplemented. The exact `12e2e4c` local release build passed a native Position
X Auto key smoke: operator inspection observed `motion.json` unchanged during
preview, returning to the initial value restored the canvas before Enter,
commit wrote one keyframe, one Undo removed it, and the retained final file plus
reopen show the clip without tracks. The native smoke used an integer baseline;
the higher-precision rounded-display case is covered by the automated GPUI
regression. The other current source behavior still needs native and
final-artifact validation.

The current coherent source gate passed:

- all 737/737 `fig_viewer` tests;
- 116 `fanta-text` unit tests and one doctest, with one network test ignored;
- 21 focused `fanta-render` TextPath tests and five focused `fanta-doc`
  TextPath tests;
- 297 `fanta-tools` tests: 290 unit, six end-to-end, and one ink oracle;
- 598 `fanta-gpui` checks: 595 unit and three architecture, with one doctest
  ignored;
- two Agent canvas-selection request-framing checks; three attachment, one
  external-context, and nine queue-filter Agent UI checks; 49 `acp_thread`
  mention checks; and one canvas-selection URI round trip; and
- `12e2e4c` package-scoped `./script/clippy -p fanta-gpui -p fig_viewer`,
  including Cargo Machete. The previously recorded wider package-scoped gate
  remains evidence for the unchanged TextPath and Agent crates.

The separately filtered Agent UI counts can overlap and are not presented as a
summed suite total. An earlier wider Agent UI subset reproduced 14 pre-existing
global-state fixture failures; that historical result is not a current combined
gate. The exact `12e2e4c` local release build, corrected toolbar captures, and
bounded Position X Auto key smoke are retained under
`/tmp/fanta-release-qa-20260912/toolbar-correction-12e2e4c`. There is no hosted
installer, deployment, billing activation, or released artifact for this work.

## Latest trim, database and native checkpoint

A later review found that an exact right-edge scrub lost its logical end
position when converted to the last included microsecond. Play then advanced
only that final instant, requiring another click to replay. A new native
regression failed against `7646e13` with a one-Play restart timeout. The player
now preserves the latest logical end request separately from the bounded
decode time. Play supersedes even a pending end seek with the range start;
a newer lower seek or range change clears the old end intent. The component
forwards the exact endpoint. Existing exclusive-end pixel guards are unchanged.
All 649 editor tests, 11 library cases and 11 native playback cases pass; the
native cases pass both traced and untraced. The regression checks settled and
pending end seeks, full and trimmed clips, advancing decoded frames and newer
seek/range overrides. Independent review found no blocker. The native app
built in 7m21s with unchanged source and protected staged state. Dragging
beyond the right scrubber edge showed 0.800 seconds and Play; one click then
showed Pause at 0.319 seconds, followed by Play at the 0.800-second end.
The included cyan scene, clipping and foreground overlap remained visible.
A normal Save and quit preserved all 21 fixture files exactly. The QA app
exited with code zero. This local binary includes preserved user changes and
is not a notarized installer. Both hosted follow-up checks pass; its exact
hosted installer also passed packaging, bundle/checksum verification, artifact
upload, and crash-symbol upload at 13:52 UTC. Notarization was skipped, and the
artifact has not been used for the clean-install pass. Evidence
is in `video-trim-validation/end-scrub`, including `native-verification.json`
and the four hashed screenshots.

The normal-speed trim candidate stores its range on the existing video node and
keeps the original source asset. It prepares a bounded, oriented preview before
one `ReplaceData` history step commits the range and poster. Cancellation,
timeout, changed inputs, changed video data, hidden/locked targets, deactivation
and removal reject or cancel pending work without partial document changes.

The existing-API authored-trim baseline failed before the change. The initial
engine candidate passed 11 library cases but failed two new native cases after
repeated identical seeks. AVFoundation can withhold a new-buffer notification
for an acquired sample. One precisely keyed validated frame now releases that
seek gate, without weakening the original timestamp/pixel assertions. All ten
native cases pass traced and untraced. Additional nearby forward/backward
positions inside one low-frame-rate sample also pass with unchanged runtime.
The held frame is replaced on new accepted output and dropped with the player.

The first combined trim build passed in 7m49s. Native QA rejected a reversed
range, applied 1.2–2.0 seconds of a three-second clip with the correct cyan
preview, restored the original preview/range with Undo, restored the trim with
Redo, and played/replayed without showing the excluded scene. Saved source
changed only the range/poster attributes, new PNG and modification timestamp;
all original MP4s, other scene values and sidecars remained exact. A normal
restart restored the saved preview and 1.2/2 inputs; a subsequent Save kept all
21 files byte-identical. Both owned QA processes exited normally. This local
binary includes preserved user changes and is not a notarized installer.
The native 800ms trim exposed a whole-second counter displaying zero; the
millisecond display correction passes the full 649-test editor suite. Its
final native build completed in 5m15s; UI observations show 0.000 → 0.321 →
0.800 seconds, followed by a stopped player at the range end. All 21 saved
files remained identical and the final owned QA session exited normally.

During that QA, the account card changed to connected without a root-authored
sign-in action. The origin and credential storage were not investigated, so the
observation is excluded from controlled production-authentication evidence.

Backend [CI at 9510f61](https://github.com/jeanc18rlos/fanta-backend/actions/runs/34468287843)
passed all four jobs and 17/17 real PostgreSQL cases. Its preceding paid-term
revision passed 15/16 but exposed a Date passed directly to postgres-js during
annual replenishment. The column-aware encoder now fixes it; a preserved
regression failed against the earlier runtime. Concurrent annual tick/payment,
duplicate paid delivery, creation/payment ordering, rollback, four account
purge/accounting barriers and immediate error/draining behavior pass. These
use real database sessions with controlled effects, not live paid inference or
provider checkout. Production remains `0d3753a`; migrations 0020/0021 and
activation remain pending.

The separate closed-project memory sequence retained approximately 5.5 MiB
and 22,514 allocations above warmup after 180 seconds. IOSurface totals stayed
stable. Both saved graphs had 294 scanner-classified allocations/14,544 bytes,
with zero newly classified leaks between them. Most new surviving allocations
are unlabelled non-object storage, and no allocation stacks were recorded.
This does not identify an owner or prove absence of reachable retention.
Matched no-playback controls and stack/retainer attribution remain open.

Evidence under `/tmp/fanta-release-qa-20260909`: `video-trim-validation`,
`annual-credit-validation/github-9510f61`,
`inline-video-validation/native-canonical`, and
`inline-video-validation/native-video-memory`.

The following sections preserve historical observations. Their older failures
and pending states are superseded only where the checkpoint above states a
verified result.

## Native video controls and hosted seek diagnosis — September 10

Actual app QA exposed the floating design toolbar covering the selected-video
controls. The toolbar now belongs to the canvas container, keeping it above
video controls and the motion timeline. A bounds regression failed before the
change, then passed; all **627 editor tests**, the themed layout regression and
the existing toolbar click-through guard pass. The corrected native release
build completed in 4m39s with the protected staged changes unchanged.

Native screenshots show early/late decoded quadrant pixels with correct normal
and 90-degree orientation, scaled parent clipping and opaque foreground overlap.
The corrected control row and seek bar are visible. Playback progressed from
cyan to the magenta end frame; returning from Code view preserved the selected
paused canvas session. A user interaction changed the viewport/selection during
one sequence, so that sequence is excluded from controlled playback evidence.
The explicit Save and normal quit changed four projected file byte sequences:
independent exact-hash reconstruction proves only JSON object-key ordering
changed. Geometry, IDs, metadata values, timestamps and all three MP4s are
unchanged; `.git/HEAD` and `.git/config` were created. Cross-feature canonical
ordering and a subsequent native byte-stable save/reopen check remain open.

Both hosted checks at `66bef60` failed
`queued_obsolete_frames_never_escape_a_completed_seek`, returning display time
200000 after seeks 1500000 -> 200000 -> 2500000 while the player clock was
2500000. The prior assertion ran before pixel sampling, so those failures do not
yet prove stale pixels. No new timing gate or weaker assertion is introduced.
The test now includes all four sampled colors on failure. Optional
`FANTA_VIDEO_SEEK_TRACE=1` emits at most 128 seek/frame records per player without
retaining frames. Local validation passed all nine media library tests and six
native cases both with and without tracing; this does not resolve the hosted
failure. The Check workflow has an explicit manual native-video-only lane,
with a separate concurrency group, to gather hosted evidence without canceling
full checks or installer builds. Its workflow syntax passes actionlint 1.7.12.

Evidence: `inline-video-validation/toolbar-overlap`, `native-fixed`,
`native-fixed/serialization-audit`, `github-failures`, and
`seek-timestamp-diagnostics` under `/tmp/fanta-release-qa-20260909`.
Final production media, audible playback, sustained resource measurements and
trusted installer validation remain unverified.

## Backend video URL deployment — September 10

At **08:13:26 UTC**, `api.fantaisa.net` resolved to deployment
`dpl_CVnuDoWotpCv8v7SG3UyXrbzwvhQ`, exact source
`0d3753aa70f152b62cd3963045f53da8ec74cdbb`. It replaced
`dpl_12FMtF55tpV1pJnyr93nx4pPz6BS` (`830f702`). The tracked-only candidate
was READY before promotion; nine public candidate checks and three generation/
workflow authentication-boundary checks passed. The nine public checks passed
again on the production domain at **08:13:40 UTC**.

The health response was `ok` with an empty family list. This verifies the
public service contract, not live GPU capacity. Source verification passed
449 backend tests, 78 CPU worker tests, type checking and
[backend CI](https://github.com/jeanc18rlos/fanta-backend/actions/runs/34450548076).
No migration, seed, pricing, environment or secret change was required, and
these checks made no inference or payment requests. Older URL-only worker
responses remain compatible. Fresh video links still require the matching-R2
`videogen` worker rollout; historical URL-only results are not repaired.
Evidence: `/tmp/fanta-release-qa-20260909/video-url-validation/production/DEPLOYMENT.md`.

## GitHub and local checks

At **06:09 UTC**, both checks for published recovery head `02a32f6` had passed:
[push checks](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34441705781)
completed at 05:58:11 UTC and
[PR checks](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34441708551)
completed at 06:04:48 UTC. Its
[exact installer](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34441705786)
was pending behind the still-running `12ad6d3` installer. These checks include
`be2b282` durable recovery. Local native full-restart QA subsequently passed at
06:20:39 UTC; exact-installer and production authentication/media checks remain open.

At **05:26 UTC on September 10**, both checks for published desktop head
`12ad6d3219c857090a264997d968cb951e3c1be5` had passed:
[push check](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34438558490)
finished at 05:20:34 UTC and
[PR check](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34438560024)
finished at 05:20:22 UTC. These include the save/media fixes and full viewer CI.
The [exact installer](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34438558598)
was still building and packaging; that step started at 04:59:26 UTC.
The earlier [eca2b86 installer](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34427356619)
completed successfully at 04:57:38 UTC, including bundle checks, checksums and
artifact upload. Its notarization step was skipped. The queued `9ff7a77`
installer was superseded by the later push; no active build was canceled.
The earlier checks below remain historical evidence.

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
completed successfully. The [eca2b86 installer](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34427356619)
was still in progress at that earlier check and has since passed as recorded above. Both hosted checks at `eca2b86` passed:
[pull-request check](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34427359223) and
[push check](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34427356711).
Subsequent commits `ee8e14d` (fast canvas input and anchor deletion), `af4cb0f`
(Scale), `ca56b0a` (Sidebar fixtures and full-suite CI coverage), `16533e6`
(K shortcut), `dcf27d6` (viewport correction), and `849685f` (history-overlay
refresh) have the local results below. Both checks at `9ff7a77` passed:
[PR check](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34434788518) and
[push check](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34434784764). Native
viewport and K checks passed at `dcf27d6`; Undo exposed stale editing handles.
The `849685f` correction passes automated checks and its 4m36s build passed
native Undo/Redo handle alignment and save/reopen. The matching `9ff7a77`
[installer](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34434784877)
was pending at that prepublication check and was later superseded by `12ad6d3`.
Commits `48171ea` (atomic files), `5d3826d` (Save), and `0edf122` (full viewer CI)
are published in `12ad6d3`, whose two hosted checks passed. Its exact-installer
validation remains pending.
Running installers have not been canceled or restarted.

The preceding local native build used `ca56b0a` and passed in **4m41s**.
Fast input, anchor editing, Scale with proportional strokes, Undo/Redo, and
save/reopen passed. It exposed clipping after point edits, as recorded below.
The `dcf27d6` build passed in **6m42s**, followed by native overflow in both
directions, geometry Undo/Redo, save/reopen, and K checks. A stale editing-handle
overlay after Undo led to the `849685f` correction. Its automated checks pass;
the 4m36s build passed native Undo/Redo handle alignment and save/reopen. The
earlier **4m39s** candidate passed curve
rendering/acquisition and authored-curve save but exposed the fast-drag and
anchor-delete defects that the combined candidate subsequently verified.
The preceding **5m45s** build passed Save As copy/adoption, original-file
preservation, autosave recovery, and rejection without opening the destination
tab. The font correction at `8ab306a` has completed local allocation review
below. Exact installer validation remains pending.
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
After the `5d3826d` native run, the process check then found only the
user-active app (PID 21997) and its helper (PID 22010), zero Cargo processes
and zero zombies, with 87.27 GiB free. The retired disk-image mounts were unmounted normally;
downloaded images and untouched app copies remain available.
After the durable-recovery run at 06:19:40 UTC, all three QA sessions were
closed and only the user app/helper remained, with zero Cargo/rustc processes
or zombies and 81.47 GiB free. The local fixture was stopped and port 47842
was independently verified closed. The completed local sign-in tab was closed.

Previous backend runtime `42a4885` was deployed as
`dpl_5484e7wjBBFjX7W2DguXBw5cHay6`, with documentation head `abc3d17`.
Current source `830f702` is live as `dpl_12FMtF55tpV1pJnyr93nx4pPz6BS`;
its verified migration, CI and public checks are recorded in Durable generation
recovery below. All **408 TypeScript tests in 43 files**, type checking, and
[GitHub CI](https://github.com/jeanc18rlos/fanta-backend/actions/runs/34436154574)
passed. All jobs also passed on documentation head `abc3d17` in
[CI run 34436789383](https://github.com/jeanc18rlos/fanta-backend/actions/runs/34436789383).
Nine public checks passed on the candidate and all nine passed on the live domain. This promotion used no new paid calls or migration. Concurrent
generation completion now commits its terminal result and billing once.
The earlier `a2bb2bb` validation included 396 TypeScript tests, 73 isolated
GPU tests, Docker, and the authenticated production checks below for default
Sonnet streaming, metering, mock-model rejection, and key revocation. These
checks do not establish deployed GPU readiness or final native authentication.

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
| `cargo test --locked --offline -p fanta-doc --lib` | Final viewport revision: 337 passed; 1 explicit benchmark ignored, including conservative curve bounds and persisted unclipped-vector state |
| `cargo test --locked --offline -p fanta-tools --lib` | Final overlay revision `849685f`: 280 passed, retaining the 17 Scale cases, 16 Path Selection cases, and viewport/metadata-preservation node-edit regressions |
| `cargo test --locked --offline -p fanta-render --lib viewport` | Final viewport revision: 6 passed, including canvas/export bounds and authored-viewport behavior |
| `cargo test --locked --offline -p fanta-format` | Atomic-write source `48171ea`: 187 passed, including integration tests and doctests; 2 ignored |
| Focused Rust formatting and `git diff --check` | Passed |
| `.github/workflows/check.yml` with actionlint 1.7.12 | Passed; `ca56b0a` adds full Sidebar and native toolbar adapter suites alongside the existing document, format, and editing-tool checks. Hosted checks through `12ad6d3` pass, including the `0edf122` full-viewer consolidation; actionlint also passed. |
| `cargo build --locked --release -p zed --bin fanta` | Preceding viewport build: 6m42s at `dcf27d6`; positive/negative viewport overflow, geometry Undo/Redo, save/reopen, and K passed. Undo exposed a stale editing-handle overlay, corrected in `849685f` with passing automated checks. Overlay build: 4m36s at `849685f`; native Undo/Redo immediately aligns handles with restored geometry, and save/reopen preserves the complete curve and exact saved-source hash. The prior 4m41s `ca56b0a` build passed fast input, anchors and Scale/strokes. Earlier 5m45s Save As/font build passed the lifecycle checks below. Save build: 280.96s at `5d3826d`; native page rename/Code path, move/Undo/Redo, K, explicit Save/reopen and all 15 saved-file hashes passed. Recovery build: 4m46s at `be2b282`; local native recovery passed two full restarts at 06:20:39 UTC, including no automatic generation/status/Messages requests and exact-byte SVG/inpaint restoration. Poster build: 7m11s at `110868d` plus protected user-staged changes; strict ad-hoc signing passed, but local sign-in stalled in Keychain before poster/video actions could be checked. Hosted-installer and production Keychain/media verification remain open. |
| Browser sign-in callback recovery | 2 passed; valid encrypted callbacks, invalid callback rejection, 10-minute deadline, retry and cancellation |
| Document suite | Save source `5d3826d`: 45 passed, retaining Save As, hidden-page rendering, bundled Git, watcher, cancellation, and entity-release checks. All 9 ordinary-Save regressions passed 20 scheduler iterations each. CodeWorkspace: 14 passed, with all 3 new cases passing 20 iterations each. The full viewer suite passed all 577 tests; both corrected autosave fixtures passed 20 iterations each. Native save/reopen smoke also passed on the 280.96-second build; forced failure coverage is automated, and exact-installer validation remains pending. |
| Production language registration | 4 passed, including named Git Commit lookup and COMMIT_EDITMSG file recognition without a parser or language server |
| Bundled Git transport | 2 Rust regressions passed, including clone/push/fetch/pull without system Git and with a bad inherited `GIT_EXEC_PATH`; replay against actual Dugite passed with clean repository integrity checks |
| Inspector integration suite | 34 passed; real page switching clears off-page selection, preserves selection on the current page, updates the inspector, and leaves the old shape unchanged |
| Native toolbar interaction suite | 32 passed, including pointer focus, popup dismissal, input editing, and keyboard navigation |
| Native toolbar adapter suite | Final overlay revision `849685f`: 26 passed, including exact NodeEdit/Path Selection overlays after keyboard and toolbar Undo/Redo without pointer input, plus the earlier drag/delete, Scale, viewport, and search/keymap checks. Both new overlay cases passed 20 scheduler iterations each. The five fast-drag/delete, Scale, and two viewport GPUI regressions also passed 20 each; the ordinary raster pixel guard passed once. |
| Native generation/media suite | Atomic-write source `48171ea`: 7 generation-media and 28 generation-workspace tests passed. Earlier 37-test coverage included wire compatibility, integer segmentation clicks, toolbar activation, catalog authorization, prompt-to-SVG, and scrollable inpainting controls. |
| Sidebar suite with ownership, startup, and selection fixes | Final run: 143 passed, 0 failed in 22.96s. All six corrected fixtures passed 20 scheduler iterations each. Database-fixture isolation and setup corrections retain the behavioral assertions; this batch adds no Sidebar runtime change. Earlier 137-pass/6-failure and 128-pass/8-failure baselines remain below. |

Commit `ca56b0a` adds full Sidebar and native toolbar suites to CI; actionlint
passed. The final local Sidebar run passed all 143 tests. Repeated archive,
terminal, and unarchive fixture failures came from shared database metadata;
isolation corrections retain the original behavioral assertions. All six
corrected fixtures passed 20 scheduler iterations each. This batch changes no
Sidebar runtime behavior. Hosted execution passed in both `9ff7a77` checks.

New tests cover snapshot isolation through node/hierarchy edits and unchanged
JSON serialization. A GPUI test also covers closing the visible assistant
while the center document has focus, reopening it, and restoring document
focus. It passed locally (1 test; 0.37s) with:

```sh
cargo test --locked --offline -p agent_ui --lib test_toggle_closes_visible_agent_panel_when_center_pane_has_focus
```

Backend coverage includes early idempotent recovery, Gateway planner routing,
worker source fixes, and safe validation-error formatting. The backend is
deployed; default Sonnet streaming and metering passed authenticated production
API checks. Final native, production media, and payment verification remain pending.

## Save lifecycle and atomic files — September 10

Desktop `48171ea` publishes generated outputs, assets, and project source
files through temporary files in the destination directory. Failed writes
preserve existing files and remove temporary files; old short asset files
are repaired. The baseline reproduced six failures with two controls passing.
The final 187 format tests (two ignored), seven generation-media tests, and
28 generation-workspace tests passed. Root and independent peer review found
no blocking defect. This is per-file atomic publication, not a transaction
covering the whole project.

Ordinary-Save source `5d3826d` orders writes by snapshot capture, preserves newer
edits and media, rearms autosave, and keeps newer editing sessions active.
Four baseline regressions reproduced the save races; the final nine cases
(including five additional ordering/cancellation controls) and full 45-test
document suite passed. All nine GPUI cases also passed 20 scheduler iterations
each (15.63 seconds of test time). The canceled first-materialization case
deliberately refuses to overwrite a folder whose foreground adoption never
completed; the original source and newer in-memory content remain intact.

CodeWorkspace's separate baseline reproduced two missing/stale source-path
failures, with the unchanged-editor control passing. Its correction passed all
14 CodeWorkspace tests and all three new cases across 20 iterations each.
The earlier broader run passed 575 tests but exposed two autosave fixtures
missing theme initialization. After the fixture-only correction, the full
viewer suite passed **577 tests with no failures** (16.07 seconds of test
time); both affected cases passed 20 iterations each. CI commit `0edf122` runs
the full viewer suite once in place of five overlapping filters; actionlint
passed. The save/media/CI batch is published in `12ad6d3`, and both hosted
checks passed by 05:26 UTC. **Pending:** exact-installer validation.

The `5d3826d` native release build completed in **280.96 seconds**. Page rename
updated the Code source path; rectangle movement and Undo/Redo ended at
x=88, y=348, width=100, height=60. K activated Scale. Explicit Save, quit and
reopen preserved all three complete curves and the renamed Code path. All 15
saved-project file hashes were unchanged after reopen, and the original
source fixture stayed unchanged. Both QA apps quit normally.

This local build includes the user’s preserved staged changes. The native
run did not force concurrent write failures or make production authentication,
AI/media, or payment requests; deterministic tests cover the save failures.
Evidence is in
`/tmp/fanta-release-qa-20260909/save-generation-validation/native/verification.json`.
The final cleanup check found only the user-active app (PID 21997) and helper
(PID 22010), zero Cargo processes, zero zombies, and 87.27 GiB free.
These checks do not establish a customer-ready installer. Logs are under
`/tmp/fanta-release-qa-20260909/save-generation-validation/` and
`/tmp/fanta-release-qa-20260909/media-atomic-save-validation/revision2/`.

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
production credentials or paid calls were used. Source `bccc5fd` was pushed;
both GitHub checks passed, verified at 21:34 UTC. Its installer subsequently
completed and passed downloaded package, revision, signature, and bundled-Git
checks described above. That package was not launched; later sources need
their own exact-installer validation.

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
Later hosted checks through `eca2b86` cover this network correction and passed.
Exact-installer verification remains required.

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

At that stage the full suite reported **137 passes and 6 baseline failures**,
improved from 133 passes and 7 failures. Subsequent fixture corrections retain
the exact behavioral assertions. Repeated archive, terminal, and unarchive
failures were traced to shared test-database metadata and corrected with
fixture isolation. The final full suite passed **143 tests, 0 failures in
22.96s**; all six corrected fixtures also passed **20 scheduler iterations
each**. This fixture batch adds no Sidebar runtime change. Commit `ca56b0a`
adds full Sidebar and toolbar CI coverage and passed actionlint; hosted
execution passed in both `9ff7a77` checks. Evidence is in
`/tmp/fanta-release-qa-20260909/sidebar-selection-validation/` and
`/tmp/fanta-release-qa-20260909/sidebar-followup-validation/`.

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
native toolbar activates this tool. Scale is implemented in the subsequent
`af4cb0f` correction below; Text on Path was still a placeholder at this
historical checkpoint, before the source-only candidate recorded above.

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

Results for that batch: **336 document tests passed** with one explicit benchmark ignored;
**258 editing-tool tests passed**, including all 16 Path Selection cases; and
**15 native toolbar tests passed**. The new toolbar-to-canvas drag/undo/redo
regression passed **20 scheduler iterations**. Focused Rust formatting,
actionlint, and diff checks passed. GitHub now includes the full editing-tool
suite and the new native regression. The subsequent **4m39s** native candidate
passed curve rendering/acquisition and saving without changing the authored
curve. It also exposed the fast-drag and anchor-delete failures below, so this
was not a complete canvas interaction pass. The next exact installer still
needs visual curve/selection/layout checks. Evidence and guarded before/final
source hashes: `/tmp/fanta-release-qa-20260909/path-selection-validation/`.

## Fast canvas input and proportional Scale — September 10

Native `eca2b86` QA showed press/move/release drags leaving both ordinary
selection and Path Selection unchanged, while clicking and ordinary-selection
keyboard nudging worked. Backspace on one selected anchor deleted the entire layer and left
stale point overlays. Commit `ee8e14d` registers canvas window listeners before
the press and checks live gesture state, preserving one move/release path.
Backspace and toolbar Delete now dispatch to NodeEdit/PathSelect; explicit
layer deletion keeps its object-level meaning.

The tests-only baseline reproduced **three fast-drag failures** and **two
anchor-delete failures**. All five now pass, each across **20 scheduler
iterations**. The drag tests dispatch press/move/release without a repaint,
including release outside the canvas; they check exact geometry, one undo
entry, Undo, and no continued drag on hover. The delete tests load the shipped
keymap and check layer preservation, removal of one anchor, updated overlays,
Undo, and explicit layer deletion. Evidence:
`/tmp/fanta-release-qa-20260909/fast-drag-validation/`.

Commit `af4cb0f` implements proportional Scale handles. Geometry, descendant
positions, text and dimensional styles scale together without compounding
selected descendants; placement-only content preserves its source. Visible
variable-bound frame dimensions determine the pivot, and release consumes its
final pointer position even without a matching move. Undo/Redo and cancellation
preserve original geometry, styles, and bindings. Text on Path was still
pending at this historical Scale checkpoint; the later source-only candidate
is recorded above.

Scale's revised tests-only baseline produced **14 engine failures**, with
**three controls passing**, and **one native failure**. At that stage, all
**275 editing-tool tests** and **21 native toolbar adapter tests** passed. The Scale native
regression also passed **20 scheduler iterations**, checking real toolbar
activation, text size/font preview, a single undo entry, Undo/Redo, and Escape.
Evidence: `/tmp/fanta-release-qa-20260909/scale-validation/`.

These automated before/after results preceded the combined native run below,
which verified fast drags, anchor deletion, Scale, and save/reopen but exposed
viewport clipping. Commit `16533e6` subsequently connected the K shortcut. The
full Sidebar suite passed all 143 tests, and all six corrected fixtures passed
20 scheduler iterations each. Full Sidebar and toolbar CI coverage is committed
in `ca56b0a`; both `9ff7a77` hosted checks pass, and an exact installer remains pending.

## Combined native candidate — September 10

The candidate built from `ca56b0a` source in **4m41s** and passed strict ad-hoc
signature checks in the reused QA bundle. Ordinary rectangle dragging and
Undo worked. Path Selection now committed the expected translated cubic
geometry, and deleting one anchor kept the layer as a three-anchor triangle;
Undo/Redo rendered the correct states. Scale activated from the toolbar,
changed a 100×60 rectangle to 198.802×119.281 with a fixed top-left corner,
and passed Undo/Redo. A quadratic curve changed from 200×160 to
300.351×240.281 while its stroke changed proportionally from 4 to 6.007.
Saving and reopening preserved those shapes and the saved page hash; the
other two curve source lines remained exact. Both native QA processes exited
normally, leaving the user's existing app alone.

This was **not a complete native pass**: a point edit retained an imported
vector's original `local_size` viewport, clipping the visible curve to its old
rectangle despite correct geometry and overlays. Clearing the viewport alone
would be undone by document-load backfill for some paths. The later `dcf27d6`
correction and its final automated results are recorded next; they do not
retroactively turn this candidate into a complete native pass.
Evidence: `/tmp/fanta-release-qa-20260909/canvas-tools-native/`.

## Persistent viewport correction — September 10

Commit `dcf27d6` uses `NodeFlags::UNCLIPPED_VECTOR` to record that point editing
removed an imported vector's viewport. This replaces the intermediate approach
that wrote a marker into `CanvasNode.meta`; that approach could overwrite
opaque array/string extension data. The final implementation preserves metadata
objects, arrays, strings, and null without using them for core editing state.

A geometry-changing gesture clears `local_size` and sets the dedicated flag.
Cancellation, tool changes, and returning to the original position restore the
original path, viewport, and flag bit while preserving unrelated flags. Path
and flag changes commit together for Undo/Redo. Load-time viewport backfill
skips flagged vectors. The flag also overrides a remaining explicit viewport
in canvas rendering and visual export bounds; ordinary unflagged SVG viewports
retain their clipping behavior.

The final revision's tests-only baselines reproduced one document failure,
one tool failure with four controls passing, three viewer pixel/save/reopen
failures, and one export-bounds failure. These include exact opaque-metadata
preservation and the mixed flag/remaining-box case. The final six-file source
then passed **337 document tests with one benchmark ignored**, **280 editing-tool
tests**, **6 viewport render tests**, and **24 native toolbar adapter tests**.
Both positive- and negative-coordinate GPUI pixel regressions passed **20
scheduler iterations each**, including actual project save/load, document
initialization, exact metadata/flag preservation, and visible overflow after
reopening. The ordinary raster pixel guard in the same filtered invocation
ran once; it is not a 20-iteration GPUI test.

Evidence and frozen source hashes:
`/tmp/fanta-release-qa-20260909/path-viewport-validation/revision2/`, including
`final-manifest.json`, `after-execution.json`, and the final test logs. Earlier
metadata-candidate results remain historical evidence, not validation of this
final revision.

**Native viewport and K checks passed; the separate overlay correction is verified below.**
The `dcf27d6` build completed in 6m42s. The existing QA bundle was reused and
passed strict ad-hoc signature verification. Actual pointer drags in both
positive and negative directions rendered the complete edited curve beyond
its old viewport. Saved source contains `UNCLIPPED_VECTOR` and no `local_size`.
Undo restores the exact authored quadratic, original viewport, and absent
flag; Redo restores edited geometry. K activates Scale, confirmed by its menu
selection and eight handles. Reopening the negative edit rendered the whole
curve with an unchanged saved-source hash. Opaque array/string preservation
is covered by the repeated GPUI write/read tests; this manual fixture has no
opaque payload.

Undo also exposed a separate UI defect: path anchors/control handles retain
their previous positions until another pointer event, although the restored
geometry and ordinary selection bounds are correct. The `849685f` correction
and its automated results are recorded next; they do not retroactively turn
this candidate into a complete native editing pass. Both QA instances (72094
and 72782) quit normally. At the end of that QA run, only the user's retained
app/helper remained, with no Cargo/rustc or zombie processes
and about 89 GiB free. Evidence is in
`/tmp/fanta-release-qa-20260909/path-viewport-validation/native/verification.json`.

## Path-editing overlays after Undo/Redo — September 10

Commit `849685f` refreshes cached NodeEdit and Path Selection overlays after a
successful Undo/Redo. The tool shell previously kept the final pointer-response
positions even after document history restored the correct curve. A read-only
overlay hook now derives anchors and controls from the restored document without
changing geometry, document/tool selection, gesture state, or history. Other
tools retain their existing behavior; no synthetic pointer event or tool
reactivation is required.

Two GPUI regressions use the real toolbar and shipped keymap, one for NodeEdit
and one for Path Selection. They drag a quadratic anchor, then invoke both
keyboard and toolbar Undo/Redo without any pointer input. Each checks exact
anchor/control world positions, the selected-anchor marker, geometry, viewport,
flags, opaque metadata, document selection, and history depth. Both compiled
and failed at the stale-overlay assertions before the runtime correction.

The final **280 editing-tool tests** and **26 toolbar adapter tests** pass.
Both new GPUI cases also passed **20 scheduler iterations each** in **45.67s**.
The final six source hashes were independently reviewed. Frozen before/after
source and logs are in
`/tmp/fanta-release-qa-20260909/path-overlay-history-validation/`, including
`final-manifest.json`, `verification.json`, `before-tests.log`, and the final
suite/repetition logs.

**Final overlay native validation: PASSED for the recorded scope.**
The `849685f` release build completed in 4m36s and the reused QA bundle passed
strict ad-hoc signature verification. A real Path Selection segment drag
rendered the complete moved curve. Cmd-Z and Cmd-Shift-Z immediately placed
all visible anchors and controls on the corresponding restored geometry,
without intervening pointer input. The saved FNX was byte-identical to the
same drag on `dcf27d6`; reopening rendered the complete curve with the same
saved-source hash. The manual check used keyboard history in Path Selection;
both modes and keyboard/toolbar history are covered by the repeated GPUI tests.

Both QA instances (74384 and 74957) quit normally. The final process check
found only the user's retained Fanta app and its helper, no Cargo/rustc or
zombie processes, and about 88.5 GiB free. Evidence, source guards, and saved
fixtures are under
`/tmp/fanta-release-qa-20260909/path-overlay-history-validation/native/`.
Both `9ff7a77` checks passed for this editing batch. The later published
`12ad6d3` also passed both checks; its exact installer was still building at
05:26 UTC. These checks are not customer-release signoff.

## Durable generation recovery — September 10

**Backend deployment, local automated validation, both hosted desktop checks
at `02a32f6`, and local native full-restart recovery QA passed. Exact-installer
and production Keychain/media verification remain pending.** Desktop source `be2b282` persists a local recovery journal scoped to
the normalized API endpoint and the authenticated `/v1/me` user and organization
IDs. It stores a generation's submitted body and idempotency key before its
POST, preserves uncertain submissions and accepted IDs across reopening, and
uses the same body/key only on explicit Retry. Reopening does not issue an
automatic paid POST; known jobs recover through authenticated status reads.

Per scope, storage is capped at **32 unfinished generations**, including at
most **8 unconfirmed submissions**, plus **12 completed records** and **64 MiB**
of serialized journal data. Unfinished work is retained when completed history
is pruned. Vector history stores completed SVG results only; it does not
recover uncertain `/v1/messages` requests or replay Messages calls.

Companion backend source `830f702` is live as
`dpl_12FMtF55tpV1pJnyr93nx4pPz6BS` and passed **438 tests in 44 files**,
**60 focused tests**, type checking and
[source CI](https://github.com/jeanc18rlos/fanta-backend/actions/runs/34441329043). It binds the request digest, client
key and generation in one transaction, compares the submitted JSON before
caps, rejects non-finite numbers, and rechecks accepted keys before preflight
rejection. Only a confirmed-unreserved first attempt can be discarded using
the explicit response marker; an earlier uncertain attempt remains recoverable.
The marker does not lock out another concurrent submission. Historical null
hashes remain unverified, and reservation does not guarantee provider dispatch
recovery.

Migration `0019_generation_request_hash` was applied at **05:45:37 UTC on
September 10** after the preceding 19 migration timestamps and SQL hashes
matched. The resulting 20-entry history and nullable text column were verified;
an independent check passed at **05:49:25 UTC**. No seed ran and the migration
checks read no customer rows. The
[candidate](https://fanta-backend-lqqciogei-squidreds-projects.vercel.app) passed
all nine public checks, and all nine passed on `https://api.fantaisa.net` after
promotion, without inference. At **05:52:46 UTC**, the production target and
direct `/v4/aliases/api.fantaisa.net` lookup verified the same READY deployment.
The custom-domain check is authoritative even though the deployment's alias
list only shows its project alias. Evidence is under
`/tmp/fanta-release-qa-20260909/generation-submission-validation/production/`.

A production API check completed at **06:09:09 UTC**: device sign-in passed,
the same deliberately invalid generation request returned HTTP 400 with
`invalid_request_error` and `x-fanta-generation-unreserved: true` twice, and
the credit balance stayed **387 → 387**. The temporary key was revoked and
subsequently rejected with HTTP 401. Credentials stayed in process memory;
no inference, checkout or native application was used. Evidence:
`/tmp/fanta-release-qa-20260909/generation-submission-validation/production/live-rejection-verification.json`.

The complete desktop viewer suite passed **596 tests** in **21.51 seconds**.
Initial tab-close tests reproduced two failures, the unmarked-400 baseline
reproduced one failure with a passing explicit-unreserved control, and the
stale signed-out SVG baseline reproduced one failure. All now pass. Additional
coverage proves a persistent write happens before POST, local acceptance-save
failure preserves the exact mask/source/seed request across closing, retry
reuses its body/key, an already-accepted request uses GET, completed SVG reopens
and saves exact bytes without another metered call, and account-key rotation
preserves only the matching verified identity's history. Nine journal tests
cover persistent SQLite reopening, concurrent writes, rollback, corruption,
size limits and preserving unfinished jobs. The acceptance-failure fixture
initially returned an empty successful output and correctly displayed a media
error; its output fixture was corrected without changing runtime behavior.
All ten new recovery cases passed 20 scheduler iterations each (200 executions,
34.03 seconds). Focused formatting, diff checks, and root/independent peer
review passed. The local native build at `be2b282` completed successfully in
**4m46s**. Local native full-restart recovery QA passed at **06:20:39 UTC**.
The preceding automated checks made no production inference or payment calls;
the migration and deployment are recorded separately above.

The same ad-hoc signed bundle and profile were used for sessions 2561 → 4841 →
5661, with two complete app restarts. Neither restart added generation POSTs,
status reads or Messages calls before explicit actions. Accepted image history
resumed through GET only. A deliberately dropped inpaint response left a
recoverable request; explicit Retry reused the exact key, JSON body, mask and
source and returned the same job. The fixture recorded **3 unique jobs,
4 generation POSTs and 1 Messages call**. SVG saved before and after restart
had identical bytes, and the recovered inpaint PNG matched the fixture exactly.
The original orange 512×512 source and white edit mask were restored, and the
saved SVG remained visible after mask recovery.

All three QA apps quit normally; the user app was preserved. The fixture
session 6493 was stopped and its port 47842 listener was confirmed absent.
Prepared fixture/helper hashes remained unchanged. This was local loopback QA
using a newly built ad-hoc signed bundle that includes protected user-staged
changes. It is not hosted-installer, production GPU or production Keychain
proof. Storage-failure injection remains automated coverage, and provider
dispatch-crash recovery remains a separate limitation. Evidence and exact
artifact hashes are in
`/tmp/fanta-release-qa-20260909/generation-journal-validation/native/verification.json`.
Evidence is under `/tmp/fanta-release-qa-20260909/generation-journal-validation/`
and `/tmp/fanta-release-qa-20260909/generation-submission-validation/`.

## Video posters and result ownership — September 10

Runtime `110868d` adds a decoded first-frame preview for generated MP4 results
on macOS. Placing a video stores its original bytes and PNG poster together;
quarter-turn rotations and reflections determine the canvas dimensions.
Prepared video bytes are reused for Save, Play and placement. Switching results
cancels the old preview, and account changes reject delayed preview, save,
player and placement completion. Negative prompts are sent only when the
selected model explicitly supports them; switching models preserves the draft.

The decoder runs on a background executor with a 20-second preview deadline,
100 MiB input limit, 32-megapixel source limit and 1200-pixel poster limit.
External file/network references are disabled. Scale, shear and perspective
track transforms are rejected rather than placed with incorrect dimensions.
The six real H.264 decoder tests passed, covering first-frame colors, bounded
size, rotations, mirroring, corrupt samples, cancellation and temporary-file
release. Initial color failures were independently reproduced in Apple's
decoder and isolated to ambiguous color tags and a tiny H.264 encoding; the
corrected fixtures and unchanged pixel tolerance are documented in
[the fixture notes](../../crates/media/test_fixtures/README.md).

The final editor suite passed **606 tests** in **24.42 seconds**. It includes
poster rendering assets, exact MP4/PNG preservation through Undo/Redo and
save/reopen, failed-placement cleanup, cached-video saving, result-switch
cancellation and save-dialog completion after sign-out. Seven related timing
cases passed **20 scheduler iterations each**. Two negative-prompt tests first
failed against the prior behavior and then passed with the capability guard.
License, formatting and diff checks passed; GitHub checks now include the
native decoder suite. Review found no blocking ownership/integration defect.
These tests used the original checkout with its protected user-staged changes.
At this automated-test checkpoint, before the later native attempt below, no
new app bundle had yet been built or launched. No production generation or
payment was made. Full native UI/installer QA remains pending. Playback
still opens the system player; inline playback, scrubbing and planned video
editing tools remain unfinished. This is not a claim that every production
codec or platform decodes correctly.

At **07:01 UTC**, only user app 21997 and helper 22010 were running, with no
Cargo/rustc or zombie processes and **68.37 GiB free** after testing. Daily
04:00 local housekeeping remains active; it cleans Cargo files on Sundays when
idle, also cleans on a daily check below 40 GiB free, and honors deferred clean
requests. Active work and installers are preserved. The earlier cleanup
recovered about 49.24 GiB. The `12ad6d3` GitHub installer succeeded; the
`02a32f6` installer was still running and was preserved.
Evidence: `/tmp/fanta-release-qa-20260909/video-preview-validation/verification.json`.

### Native build and blocked sign-in — September 10

The `110868d` poster app built successfully in **430.95 seconds (7m11s)**,
finishing at **07:15:40 UTC**. The build includes protected user-staged changes;
its prepared QA bundle passed strict ad-hoc signature verification. This is a
local build result, separate from hosted-installer validation.

Normal local-fixture sign-in reached a Keychain write and stalled in
`SecItemAdd`. SecurityAgent interaction was blocked to automation, and manual
user approval was requested. At the **07:32:34 UTC** verification, the idle QA
app/helper had quit normally and the fixture was stopped with port 47843
closed. Counters recorded one sign-in callback, zero profile/catalog reads,
and zero generation POSTs, status reads or media deliveries. Consequently,
**native poster visuals, video Save/Play/Place and video-project save/reopen
remain unverified for this build**. The separate native playback engine was
subsequently validated as described below; controls/canvas work followed later.

At that earlier checkpoint, backend video URL fix `0d3753a` had passed
**449 backend tests in 44 files**, **78 CPU-only worker tests**, and type
checking. It has since been deployed and publicly verified as recorded above.
Its renewal behavior still requires the `videogen` rollout with matching R2. Historical URL-only results remain unchanged;
results first completed without backend R2 receive no later library-asset
backfill. These tests do not prove live GPU/storage execution.

The subsequent housekeeping check found only the user app **21997** and helper
**22010**, no Cargo or zombie processes, and **60.88 GiB free**. Daily 04:00
Europe/Madrid checks, Sunday idle Cargo cleanup, below-40-GiB cleanup and
deferred-clean handling remain active; active work and installers are preserved.
Evidence: `/tmp/fanta-release-qa-20260909/video-preview-validation/native/`
(`native-build.json`, `verification.json`, `signin-wait-sample.txt`) and
`/tmp/fanta-release-qa-20260909/video-url-validation/verification.json`.

## Native playback engine — September 10

The media crate now provides AVPlayer playback with oriented, bounded BGRA
frames, play/pause, time reporting, latest-target seeking and explicit session
teardown. The player stays on the main UI thread; metadata preparation and a
single source-frame check are cancellable. The source bytes remain local, and
external references are forbidden. Callers must impose loading/seek deadlines,
poll active playback and release replaced frames. Output is bounded to 2048
pixels per side; this is not a bound on all internal decoder memory.

At **07:40:26 UTC**, all **seven library tests** and **five real native playback
cases** passed. The native cases exercise advancing frames, pause, distinct
scene seeks, rapid latest-target seeking, pause during a seek, natural end and
restart, rotations/reflection and colors, damaged input, main-thread rejection,
and repeated close/cancel cycles with temporary-file cleanup. Tests run on the
main run loop and keep audio muted. Native cases took 4.15 seconds including
compilation; library tests took 1.85 seconds including compilation.

The first native run passed four cases and exposed a real corruption problem:
AVPlayer's compositor could produce black and report normal completion for
zeroed encoded samples. An independent native probe reproduced it. Preparation
now requires a small decoded source frame using the existing image-generator
path and the same asset/input lifetime. The unchanged corrupt fixture is
rejected, and all poster tests still pass. This establishes first-frame
validity, not the validity of every later sample. An earlier test-binding
Boolean compilation error was also corrected; both failure logs are retained.

Root and independent ownership/ABI reviews found no blocking issue. License,
Rust formatting, diff and workflow syntax checks passed. GitHub checks include
both native suites. At that checkpoint the engine was not yet connected to
generation controls or the canvas. The integration and subsequent CI finding
are recorded below; audible playback, sustained resource measurements and
final app/installer checks remain pending.
No production media request or payment was made. Evidence:
`/tmp/fanta-release-qa-20260909/video-playback-validation/verification.json`.

## Inline video controls and canvas rendering — September 10

Generated video previews now use a shared native player with Play/Pause,
seeking, time and mute controls. A selected full-length, normal-speed video
layer uses those controls and draws decoded frames through the existing scene
renderer. Clipping, transforms, layer order and effects remain in that path;
existing volatile-video behavior preserves caching for unrelated static layers.
Unsupported authored trim/speed settings show an explicit message.

The GPU path retains the input pixel buffer, CoreVideo texture and borrowed
Skia image through the synchronous flush. The CPU fallback copies validated
padded BGRA rows into owned pixels. Frame/session keys change rendered pixels
without copying or editing the document every frame. Replacing the selection,
source or document closes the old player. Tab/window inactivity pauses it;
removal releases it even if the prior rendered entity is retained. Moving a
tab recreates playback rather than reusing a permanently closed player.
Generation previews reuse their already downloaded bytes and preserve the
account/result guards. Preparation/loading/seek deadlines remain finite.

All **626 editor tests passed** on the combined source. **15 playback-control
GPUI cases** and **two canvas-lifecycle GPUI cases** passed **20 scheduler
iterations each**. The renderer suite passed **254 tests, one ignored**, with
new pixel tests for live frames under transformed clips, foreground layers
and effects. The padded-buffer test verifies owned pixels after the original
buffer is released. A local project with three exact MP4 fixtures passed
parser/save/reopen and a zero-write second save; it has not yet been opened
in the rebuilt native app. These local checks include the protected user-staged
changes; the isolated source awaits fresh GitHub checks.

The first combined compilation needed an explicit mouse-event type. The next
suite exposed a fixture-only cleanup assertion: GPUI queues destruction until
an App update flushes it, while an idle scheduler pump alone need not do so.
The correction drops the last handle within that update and preserves the
exact release-count assertion; fixture assertions no longer poison held
mutexes. The source project fixture also initially assumed a sized page root;
its corrected expectation follows the existing unsized/unclipped page invariant,
while preserving the nested clipping assertions. All initial logs remain.

GitHub [PR checks at `a984699`](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34451427646)
failed when a rapid seek exposed a 2.7-second frame after a 1.5-second target;
the sibling push check passed. Eleven local runs of the expanded native cases
did not reproduce that intermittent native failure. The correction queries
the explicit latest seek target until its first frame, uses item time while
paused, and retains host-time synchronization during ordinary playback.
A future frame beyond the target is not published. Two deterministic tests
cover the stale-clock choice and publication guard without assuming a fixed
frame duration. Earlier presentation timestamps can be valid long samples;
this is not a general variable-frame-rate corruption detector.

The media library passes **nine tests**; all **six real native playback cases**
passed **ten consecutive runs** after the correction. The app and CLI compile
check passed. A new slow-frame control initially assumed the encoded sample's
starting timestamp; AVVideoComposition legitimately returned the requested
display timestamp instead. That test now verifies the known containing sample
interval, no future frame, and the same expected pixels. Existing rapid-seek
assertions were not relaxed. Fresh GitHub verification remains pending.
Native app visuals, audible playback, sustained decoder/GPU memory measurements,
production generation and the final signed/notarized installer remain open.
Evidence: `/tmp/fanta-release-qa-20260909/inline-video-validation/`.

## Remaining release requirements

| Goal | Evidence and remaining verification |
| --- | --- |
| Backend and AI Gateway | Runtime `0d3753a` is live as `dpl_CVnuDoWotpCv8v7SG3UyXrbzwvhQ`; its candidate/live checks and remaining worker limits are recorded above. Earlier runtime `830f702` passed 438 tests in 44 files, 60 focused tests, type checking and source CI (`34441329043`) passing. Migration `0019_generation_request_hash` was applied at 05:45:37 UTC and independently verified at 05:49:25 UTC (exact 19→20 history and nullable text column, no seed or customer-row reads). Nine candidate and nine live public checks passed without inference; direct alias verification confirms `api.fantaisa.net` serves this deployment. Atomic submission/body binding now joins the earlier terminal completion/billing, account selection, access/revocation and mock-model guards. The 06:09:09 UTC production API rejection check passed device sign-in, two 400/unreserved responses, unchanged 387 credits and key revocation followed by 401; it used no inference or native app. Earlier unauthenticated chat returned 401 and billing preflight returned 204. Browser-confirmed device API sign-in, catalogs, default Sonnet streaming, one-credit debit and key revocation passed previously in production. Repeat through the final native installer; Keychain persistence and native account-error handling remain unverified. |
| Charge customers | The billing page still displays an explicit checkout-not-configured notice and no purchase or manage buttons. The latest API/dashboard check shows Pro/Owner and 387 credits after two one-credit AI verification requests. Pricing/product/currency selection and production Polar configuration remain blocked. Checkout, webhook retry/cancellation/renewal, exactly-once credits, and billing portal remain unverified. No purchase or customer charge was made. |
| Native generation | 37 generation tests passed, including string-seed submission/poll/replay and integer segmentation coordinates. Six network-timeout regressions passed 20 scheduler seeds each; the two new submission/poll checks also passed 20 each. Requests and transfers now time out without losing their recovery state. Local fixture sign-in/catalog, image polling/gallery/save/place, masks/background removal, editable SVG preview/place/save, and MP4 poll/play/place/save passed, with saved assets/layers verified. Final native checks also passed visible prompts, per-mode drafts, source-point selection, mask-to-inpaint source restoration, scrolling, and inpaint completion. Verify real media requests in production. The durable-recovery implementation described above passes all 596 viewer tests and persists generation state across reopening. Its `be2b282` native build passed in 4m46s, and both hosted checks at `02a32f6` passed. Local native full-restart QA passed at 06:20:39 UTC: no automatic generation/status/Messages requests, GET-only accepted-job recovery, exact uncertain-request replay and exact SVG/PNG bytes. The hosted installer and production GPU/Keychain remain unverified. The earlier `12ad6d3` has only tab-lifetime recovery. Runtime `110868d` adds video posters and account-safe result completion; the six native decoder tests and 606 editor tests pass as described above. Its 7m11s native build passed, but sign-in stalled in Keychain before poster Save/Play/Place/reopen checks; native app/installer QA remains pending. That build uses the system player. The subsequent inline generation controls and selected-video canvas rendering pass the automated checks above. Native UI/installer validation, audible playback, sustained resource measurements and planned video editing remain unfinished. |
| Additional canvas tools | Path Selection, proportional Scale, and the K shortcut are implemented. The combined `ca56b0a` native candidate passed fast input, anchor editing, Scale/strokes, Undo/Redo, and save/reopen, but exposed viewport clipping. The `dcf27d6` viewport revision passed 337 document tests with one ignored, 280 tools, 6 viewport renders, and 24 toolbar adapters; both viewport GPUI pixel/reopen cases passed 20 iterations each. Its native build passed in 6m42s; viewport overflow, save/reopen, and K passed. Undo exposed stale editing handles. The `849685f` read-only overlay correction now passes 280 tools, 26 toolbar adapters, and both new GPUI cases across 20 iterations each. Its 4m36s build passed native immediate Undo/Redo handle alignment and save/reopen. Both `9ff7a77` hosted checks pass; exact-installer verification remains pending. The current source-only Text on Path implementation is described above; its coherent automated gate passes, while native validation remains pending. |
| GPU service | Backend CI tests passed. Production HMAC access remains blocked; deployed GPU availability and successful end-to-end generation are not established. |
| Design and Git UI | Synthetic creation/edit/save/reopen and app-driven review/stage/commit/push passed. Final native import/edit/undo/redo/save and a second reopen/edit/save/close passed for the 29,301-node fixture. The candidate now renders a fresh 128 MB UI-kit import and Grid/Icons page changes; a visible green fill edit, Undo/Redo, and saved source passed. Complete bundled Git verification passed. The final native inspector check passed, including same-page selection preservation, clearing on page changes, and unchanged saved source. The Save As correction passed 9 automated regressions, a 20-seed destination-prevalidation regression, an earlier GPUI sweep, and the custom-picker suite. Native cancel, sibling default, copy adoption/edit/reopen, original-file preservation, occupied-destination rejection, and autosave recovery passed. The final 5m45s build also passed rejection with one unchanged source tab, post-error autosave, and valid Save As/reopen of a 208×160 copy. The later `5d3826d` native build passed page rename/Code path, move/Undo/Redo, K and explicit Save/reopen with all 15 saved-file hashes unchanged. Hosted checks through published `12ad6d3` pass, including the save batch; exact-installer verification remains pending. |
| Performance and UI quality | The repeated native lifecycle above released document-sized allocations in one warm-up plus four measured cycles. After the final five-minute idle, live malloc was 33.3 MiB and physical footprint 218.9 MiB; a 6.3 MiB residual above warm-up remains unattributed. The offline leak scanner flagged 340 allocations totaling 23,120 bytes. Earlier editing/UI-kit checks also released document-sized allocations. Two specific callback/list retain cycles were subsequently fixed and absent from repeated native after-fix scans. The previously observed descriptor-array scanner signature is absent after the later font correction; its final native snapshot still flags 314 blocks / 19,936 bytes across 20 roots. Other scanner findings remain unresolved. No leak-free or Figma-performance claim is established. Test sustained edits under controlled conditions. |
| Installer | Developer ID Application certificate `ML3GCBU926` for team `SP6J7Q6M3J` was issued/downloaded, with private-key match and G2 certificate chain verified; it expires 2031-09-10. GitHub secret names `MACOS_CERTIFICATE` and `MACOS_CERTIFICATE_PASSWORD` were verified after setting them at 17:57 UTC. No local keychain import was performed. The App Store Connect API terms modal awaits explicit user approval before notarization-key generation. Then build a signed/notarized DMG and install/launch it on a clean Mac. |
| Leads | Landing head `4e6c9c9` is live as `dpl_7SD2uTp6M2W37XABjH7qm2ynSGnU`; both hosted checks passed. All 15 offline tests, type checks, lint, and the production build passed. The live form rejected an overlong address, disabled edits while submitting, and confirmed the exact test address. Its stored contact and canonical `waitlist_joined` event were retrieved in US PostHog project 410640 with matching submission ID and campaign attribution. Basic signup-to-stored-contact verification passed using one reserved-domain QA address. Exclude that record from customer counts; no conversion improvement or completed outreach campaign is established. |

Do not publish a release or enable customer checkout on the strength of the
synthetic benchmark, public health checks, or the earlier green CI runs alone.


## Accepted input recovery, canonical saves and composition-aware seeking — September 10

The local client recovery candidate passes all **639 editor tests** and eight dispatch cases across **20 scheduler iterations**. Eight baseline-compatible regressions failed before the change. Accepted requests retain their exact saved input until a permanent worker claim or terminal result is durably recorded. Explicit retry reads the existing job first; account changes, another window's claim and failed persistence cannot authorize a stale replay. Reopening never POSTs. Legacy records with discarded input cannot reconstruct it, and missing protocol metadata alone grants no retry permission. The companion backend recovery revision `cca83da` is published for review, not deployed; migration 0020 and worker activation remain pending. All four backend PR CI jobs passed, including 501 backend tests, eight real PostgreSQL multi-client contention cases, 153 CPU worker tests and the container build. The PostgreSQL lane checked the PR merge result for cca83da; local PGlite tests alone are not the concurrency evidence. This does not prove live GPU execution or recovery after a permanent claim with no durable output.

The four native Save byte differences were proved to contain only object-key reordering; all design values and original MP4 bytes were preserved. FNX and project JSON now explicitly sort object keys, including the four remaining source-edit/session sidecar writers. Two object-order regressions fail with the app's JSON configuration before the fix; four sidecar regressions fail in both configurations before their fix. GitHub now checks the app's `serde_json/preserve_order` configuration explicitly. A further regression exposed new child indices written as 1 instead of the document’s 1.0; the constructor now matches the document representation while retaining existing fractional indices. All 93 FNX and 147 format tests pass in both JSON configurations, including exact source-edit→ordinary Save bytes. Final native save/reopen on the combined build remains pending. Evidence: `inline-video-validation/native-fixed/{serialization-audit,serialization-fix,sidecar-serialization-fix}` in the release QA directory.

The [b3b983d native diagnostic run](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34460645894) failed with tracing both enabled and disabled. After native seek completion to 2.5 s, its first published buffer carried 0.2 s and the old red frame pixels. This is a real stale-frame failure. The current bounded candidate enables `seekingWaitsForVideoCompositionRendering` on the composed player item; all nine media library tests and six native cases pass locally with and without tracing. Existing timestamp/pixel assertions and variable-frame-rate handling are unchanged. The [a9f320d hosted diagnostic run](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34461694290) now passes all nine library tests and all six native cases with tracing both enabled and disabled. The formerly failing final seek returns the requested 2.5 s frame with correct pixels. This establishes before-fail/after-pass evidence on macOS 26; final combined-build native validation remains pending. Evidence: `inline-video-validation/{seek-timestamp-diagnostics/github-b3b983d,seek-composition-wait-validation}`.
