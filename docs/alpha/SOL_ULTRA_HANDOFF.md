# Fanta continuation plan for GPT-5.6 Sol Ultra

Prepared September 12, 2026. Read [the status report](RELEASE_STATUS_2026-09-12.md) first. This document is an execution handoff, not evidence that the remaining work is complete.

Read [the subsequent review](SOL_REVIEW_2026-09-12.md) before resuming Priority 0. It supplies exact repros for the redundant-clone lint failure, toolbar test failure and numeric return-to-original preview defect, plus a required correction to the toolbar screenshot evidence.

## Model and task setup

Use **GPT-5.6 Sol**, model identifier `gpt-5.6-sol`, with **Ultra** selected in Codex. Resume **“Complete Fanta creative feature set”** so its implementation context and four uncommitted Motion files stay together. Its task ID is `01a08b72-a328-7f50-a59a-d73bd64d4f93`.

The Codex composer model switcher selects the model and effort. If Ultra is absent from the slider, the official documentation points to Settings → Configuration → Ultra in model picker slider. This is a Codex setting; do not infer that `ultra` is a valid public API `reasoning.effort` value. Sources: [Codex models and effort](https://learn.chatgpt.com/docs/models?surface=app), [GPT-5.6 Sol API model](https://developers.openai.com/api/docs/models/gpt-5.6-sol).

This reporting session did not switch any task's model or dispatch new work. Follow the next session's delegation rules; requesting Ultra does not remove restrictions on spawning agents or conflicting writes.

## Objective and fixed scope

Finish and release the Apple Silicon Mac app as soon as the agreed product and release gates pass. Carry each feature through implementation, meaningful tests, native interaction, persistence/recovery, and final-artifact validation. Continue independent work when an owner-controlled external step is pending.

Preserve the prior product decisions:

- Complete the current creative feature set before release. Retain the Zed workspace structure and refine Fanta's own GPUI surfaces.
- Keep the compact floating toolbar at the **bottom of the canvas**, above the timeline/video controls. Improve spacing, typography and integration.
- Finish Image, Video, Vector, Design and Masks, including the agreed segmentation, composition and enhancement workflows.
- Deliver design motion and clip editing. Broader 3D, music, voice conversion and a full multitrack audio/video editor are outside this release. Utility voice dictation remains part of the existing toolbar scope.
- Treat the pricing catalog as a proposal awaiting an owner decision.
- **No paid production AI tests yet.** Use deterministic local fixtures and provider sandboxes. Prepare a small costed live matrix for later authorization.

Freeze the named capability checklist at the first checkpoint. For hidden roadmap functionality such as Motion Path, explicitly record whether it was part of the agreed release contract; do not silently expand the release into every backend alias or use a hidden control to imply completion. Visible in-scope controls must work. A temporary unavailable notice is honest but does not complete an agreed feature.

## Work ownership and preservation

Desktop implementation: `/tmp/fanta-edit-release-20260909`, branch `codex-release-backend-gateway`. Backend: `/tmp/fanta-backend-release-20260909`, branch `codex/launch-billing-integration`. Continue the existing draft PRs; inspect their current heads before writing.

The original `/Users/jeanrojas/fanta-edit` has protected work. Its staged binary-diff SHA-256 at this checkpoint is `f450d6256b31380030251ac176be0a3cc029de2240555067794acf1fd57df2e8`. Preserve the stage and all unrelated edits. Do not blanket-stage, reset, stash away, or overwrite them. These two handoff documents were added in the original workspace and should be carried into a later documentation commit deliberately.

The isolated desktop checkout is also dirty: four Motion files belong to the continuation task. Review and complete that patch. Do not restart it from the published head. Do not work in a second checkout against the same files while that task is active.

Use `/Users/jeanrojas/fanta-edit/target` as the shared Cargo target with one build/test owner. Check activity before reusing or cleaning it. Preserve source manifests and distinguish exact committed builds from builds containing protected user changes.

## Priority 0 — finish the interrupted auto-keyframe slice

Start here, before adding further UI work:

1. Record desktop/backend HEADs, staged/unstaged status, active task/process ownership and free disk. Read the current capability inventory and the last Motion changes.
2. Reproduce the failed scoped Clippy gate and retain the full diagnostic. The previous exit code was 101; do not invent its cause or weaken a lint just to pass.
3. Fix the actual issue. Preserve the no-op precision fix and the preview/commit/persistence boundaries.
4. Run the focused Motion cases, then the full affected viewer/shared-UI suites and scoped lint. Investigate regressions before proceeding.
5. Review the four-file diff for stale entity updates, cancellation/preview leaks, autosave while transient state is active, value quantization, context changes and duplicate history entries.
6. Update capability/validation documents and commit the coherent slice only after its source gates pass. Push to the existing PR and verify the exact new SHA.

Known starting commands, run sequentially from the desktop release checkout after checking build ownership:

```sh
git status --short
git diff --check
CARGO_TARGET_DIR=/Users/jeanrojas/fanta-edit/target CARGO_INCREMENTAL=0 ./script/clippy -p fanta-gpui -p fig_viewer
CARGO_TARGET_DIR=/Users/jeanrojas/fanta-edit/target CARGO_INCREMENTAL=0 cargo test --locked -p fig_viewer --features fanta-gpui-ui motion_auto_key_ -- --nocapture
```

Then run the full viewer suite with the product's feature configuration. The prior task used `--features fanta-gpui-ui`; compare with the current CI before changing the test surface. Use the repository's GPUI testing skill for scheduler/lifecycle regressions and its executor timers. Never replace `./script/clippy` with plain `cargo clippy`.

**Exit:** no unresolved lint/test failure in the new slice; one undo step per committed numeric edit; no keyframe from focus alone; transient preview never persists; cancel and context boundaries restore/disarm correctly; published evidence identifies the exact commit.

## Priority 1 — native validation and a bounded UI refinement pass

Build one coherent candidate containing TextPath, attachments, Motion keyframes, time comments and auto-keyframe. Reuse one QA bundle/profile serially. Distinct bundle IDs/profiles did not prevent stable-channel instances from displacing each other in prior QA; verify single-instance behavior before launch and coordinate access to the user's app. Close completed QA normally and verify process disappearance without an app/AX read that may relaunch it.

Drive the real app through:

- TextPath create/type/caret/selection, supported complex and bidirectional text, inspector edits, undo/cancel and save/reopen. Expose direction with a document-backed typed action; replace API-debug start controls with understandable placement controls. Keep unsupported paint/effect cases explicit and preserve imported data.
- Selection attachment into an unsent Agent draft, bounded snapshot identity, no-selection context picker, rapid invocation, root/scope changes and user review before submission.
- Keyframes/presets at the actual playhead, easing/track edits, timed comment placement/navigation, deleted clips and shortened durations, existing draft preservation, auto-keyframe toggles and transient editing.
- Save/Save As/autosave/reopen throughout. Compare expected document edits and untouched media bytes.

Refine shared styling in existing components: normal body weight, restrained section emphasis, subtle single borders, consistent spacing, theme-derived foreground/background/selection/focus colors, predictable icons and hit areas. Respect the user's Zed font family and scale. Keep the toolbar compact, with contextual flyouts and an explicit AI composer action, anchored to the usable canvas area as panels open or close.

Make two deliberate visual passes: first shared chrome/typography/layout, then interactions and states. After those pass, additional visual changes must fix a named acceptance defect. This bounds polish without cutting the agreed features.

Check light and dark themes, narrow and wide windows, supported UI scales, keyboard navigation, focus restoration, disabled controls, busy/error/empty states, long labels, and hit isolation. Capture before/after screenshots of the same fixtures. Success notices, errors and cancellations must be visible in the surface where the action began.

**Exit:** the current creative slice is natively usable and survives reopen; toolbar/inspector overlap, clipping, accidental canvas clicks and unreadable text are resolved; remaining feature gaps are explicit checklist items.

## Priority 2 — close the remaining editor and motion workflows

Complete vertical slices in this order, validating each in the same way:

| Slice | Required behavior | Acceptance |
| --- | --- | --- |
| Export and direct media placement | Visible PNG/JPEG/SVG/PDF format, destination and scale controls; Arrow and direct image/video placement wired to document operations | Cancel/error/success feedback; output opens with expected size/content; originals and selection remain correct; undo/reopen for insertion |
| Dev/inspect/measure/annotation | Useful read-only inspection and measurements, persisted annotations and actionable readiness findings for existing nodes | Accurate selection/root identity; no accidental document changes from inspection; annotation history/reopen; no placeholder success notices |
| Utility voice input | Permission, recording indicator, stop/cancel, transcription into an editable unsent draft and recovery from denial/failure | No automatic send; discarded recording stops work and is not retained unnecessarily; provider cost boundaries honored |
| Motion authoring | Clips/tracks, keyframes, easing, presets, auto-keyframe and timed comments form one coherent workflow | Preview/scrub/play/loop agree; timeline and inspector synchronize; history/cancel/reopen preserve authored values |
| Video clip editing | Non-destructive trim, proposed 0.25×–2× rates, source-relative timing, poster updates and original audio/media preservation | Absolute source-time representation stays consistent; excluded end frames never appear; rapid seeks and replay pass; unsupported imported values remain intact |
| Motion/video export | MP4 clip trimming with original audio and design-animation export for the agreed scope | Exported duration, timing, orientation, pixels and expected audio verified; progress/cancel/failure cleanup work |

Do not build a separate second timeline or copy an old flyout that contradicts `TimelineShell`. Reuse the shared property catalog and existing document/history operations. Keep prototype/variables/code/Git regression coverage as these workflows integrate.

**Exit:** every in-scope editor/motion command has an implementation, automated evidence where meaningful, and a native pass; no silent no-op or unexplained data change remains.

## Priority 3 — production-ready AI integration

Keep managed chat routed through the backend AI Gateway. Obtain model/capability/credit data from the backend. Preserve explicit user-selected BYOK/custom-server preferences. Do not embed gateway/provider secrets or route a managed request directly from the desktop to bypass the account ledger. GPU media workers need their supported backend job route; routing chat through AI Gateway does not itself activate media generation.

Complete:

1. Image create/edit/inpaint/outpaint, composition and enhancement; multiple named masks with visibility, per-region prompts and clear progress/result review. The existing `fanta-diffusion` frontend is a UX reference, not authorization to copy its local SAM2 transport into production.
2. Vector generation/trace with validation and editable placement, including source identity and output sanitization.
3. Video text/image input, chaining and agreed interpolation/enhancement paths with actual backend capability constraints.
4. Design generation beyond the current brief: explicit submit, progress, inspectable design changes, accept/reject, one undoable acceptance, and safe recovery. Validate design identity before applying node edits.
5. Authentication and account changes: normal sign-in, persisted credentials, restart, logout/revocation, expired session, account/organization switch, insufficient credits, offline recovery and actionable errors.
6. Durable jobs: exact immutable inputs and request identity, no automatic POST on reopen, GET-first recovery, same-key replay only when explicitly allowed, concurrent distinct-job reservations, cancellation, expired result URLs and storage failures. Resolve post-claim/no-output recovery rather than declaring it covered by unclaimed-job tests.

Prepare worker/model readiness per capability: endpoint, supported model/weights, license status, health, storage, request/response compatibility and deployment SHA. Historical gaps included segmentation/SVG endpoints and SAM3/StarVector/Klein availability. Verify current state; public `families: []` and CPU tests are insufficient live proof.

**Exit before paid QA:** deterministic client/backend/worker contracts and native fixtures pass; production configuration is reviewable; a small live test matrix names models, counts, expected debits, maximum spend and stop conditions. Request the required live-test authorization only when that matrix is concrete. Run authorized live QA once, preserving request/ledger evidence without secrets.

## Priority 4 — billing and deployment readiness

Use the existing pricing proposal. Revalidate current provider/model fees from primary sources, present a reconciled catalog, and obtain the owner's catalog/currency decision before configuring live products. Do not re-ask for decisions already made in later task history.

Before activation:

- Match immutable environment-specific product IDs, interval, amount/currency, checkout/order identity and paid evidence to server-owned grants. Fix metadata-preferred subscription mapping and pack verification gaps identified in the proposal. Never trust a client-provided credit quantity.
- Make the displayed currency, Free grant frequency, plan/pack quantities, seat handling and renewal/cancellation behavior agree across app, API and Polar. Implement or remove unsupported marketing promises for API access/priority scheduling. Preserve existing customers deliberately.
- Validate migration history and apply `0020` then `0021` through the documented additive migration path. Deploy a validated compatible backend and workers. Avoid production reseeding or destructive rollback.
- Use the provider sandbox for checkout, portal, duplicate/out-of-order webhook delivery, paid-term creation, renewal, failed payment, cancellation, downgrade, refund/dispute handling and concurrent grants. Subscribe to both `order.created` and `order.paid` plus required subscription events.
- Keep annual checkout disabled until monthly installment scheduling, authenticated tick handling, missed runs, duplicate ticks, cancellation and rollover are verified.
- Record the deployment revision, non-secret configuration presence, migration result, smoke evidence, rollback target and controls to disable checkout or a failing AI capability.

**Exit:** one consistent approved offer, correct sandbox financial/entitlement outcomes, production configuration ready, and controlled final verification completed within existing authorization. Do not charge customers merely to test checkout.

## Priority 5 — stability, exact installer and release

Run the meaningful regression suite for the final source once changes settle. Expand testing when failures or code changes justify it; avoid repeatedly rebuilding unchanged source.

Required final journeys:

- Clean install/launch → sign in → create/import → edit/undo/redo → save/reopen → review/stage/commit/push using bundled Git.
- Each of the five AI workspaces through submit, progress, result review, accept/place/save, restart/recovery and clear failure/cancel paths.
- Prototype, variables, comments, TextPath, Motion and video export on saved fixtures.
- Paid account/credits/portal behavior from verified sandbox and authorized production checks; no duplicate credits or debit without the promised outcome.
- Large-document editing and repeated open/close/video cycles. Use the prepared matched-playback experiment under `inline-video-validation/native-video-memory`; compare no-playback and playback with the same fixture and instrumentation. Attribute retained memory before changing ownership code. Record limitations; no leak-free or Figma-performance claims without matching evidence.

Complete owner-required Apple account/terms steps and notarization credentials through allowed access. Prepare everything else while waiting. Use the existing `release-macos.yml` workflow in distribution mode against the final commit. A green push DMG with notarization skipped is not the release artifact.

Verify signature chain, notarization/stapling, Gatekeeper, embedded revision/version, checksums, app/CLI architecture, bundled Git and URL handlers. Install the downloaded artifact on a clean Mac or appropriate clean environment; record the actual test environment and remaining second-machine limitations. Preserve symbols, logs, checksums and the previous known-good artifact. Enable automatic updates only after their delivery/verification/rollback path is tested.

Resolve the desktop PR's non-main base deliberately. Review final titles/descriptions and complete required checks against the actual intended release history. Avoid blanket cherry-picks of the original dirty workspace. Publish/tag the exact validated artifact within the user's release authorization once gates pass; do not add a redundant general permission step. If an external approval block remains, report it specifically with the ready artifact and remaining action.

**Release gate:** zero unresolved data-loss, payment/account-integrity, startup or core-workflow blockers; every agreed capability passes its acceptance checks; exact distributed artifact is signed/notarized and exercised; rollback and customer-facing support/known-issues information are ready.

## Leads and launch preparation

Use the recovered US PostHog project `410640`. Preserve the working signup flow and its canonical `waitlist_joined` event. Prepare the funnel from landing visit → waitlist signup → download → first saved design → first successful AI result → checkout → paid activation, with identifiers/deduplication and consent/privacy behavior appropriate to the existing product. Exclude synthetic QA contacts and internal activity.

Update the landing page with accurate screenshots, a short real workflow demonstration, clear platform/download instructions, the approved offer, and one clear primary action. Test form validation, retry/duplicate behavior, accessibility, mobile layout and attribution. Draft announcement/email/social copy and a measurable campaign plan; sending messages to people requires explicit authorization. Do not create another PostHog account because of the earlier sign-in problem: access was recovered.

**Exit:** signup capture and activation events are verified, release assets/copy match shipped behavior, and an outreach package is ready. Report acquisition/conversion results only after actual measurements.

## Iteration rhythm and reporting

For each slice: inspect current state → implement a coherent change → run focused checks → resolve failures → run affected gates → native QA → update evidence/docs → commit/push → verify exact-source CI. Keep one primary desktop writer and one local build owner. Batch independent read-only work; separate services can progress without contending for the desktop app or Cargo target when permitted by the session's delegation rules.

At each meaningful checkpoint, report:

- What now works and why it matters.
- Exact commit or uncommitted files, checks run and native/artifact evidence.
- What is implemented, tested, published for review, deployed and released—these are different states.
- Remaining blocker, dependency/owner, and the next concrete action.

Update `CAPABILITY_INVENTORY.md`, `RELEASE_VALIDATION.md`, `KNOWN_ISSUES.md`, `SMOKE.md` and `LAUNCH.md` when a change makes them inaccurate. Rewrite the current checkpoint instead of accumulating contradictory headlines. Preserve detailed historical evidence separately. Follow Rust/GPUI rules and PR hygiene, including the final `Release Notes:` section; do not edit `.rules` during feature work.

Continue the existing daily housekeeping automation without creating a duplicate. Retire finished agent QA promptly; preserve the user's app. Sunday/low-space/deferred Cargo clean runs only when idle and never as a zombie-process remedy. Save a compact continuation checkpoint before context exhaustion: next failing command, edited paths, exact heads, evidence locations, pending owner actions and process ownership.

## Ready-to-paste continuation instruction

> Continue Fanta in this existing task using GPT-5.6 Sol with Ultra. Read `/Users/jeanrojas/fanta-edit/docs/alpha/RELEASE_STATUS_2026-09-12.md` and `/Users/jeanrojas/fanta-edit/docs/alpha/SOL_ULTRA_HANDOFF.md`, then refresh the actual checkout and CI state. Resume the four uncommitted Motion auto-keyframe files in `/tmp/fanta-edit-release-20260909`; the last scoped Clippy command failed after two focused tests passed. Diagnose that failure first, finish the source gate, and validate the accumulated TextPath/attachments/Motion work natively. Continue the ordered plan through the agreed creative tools, UI refinement, backend/media readiness, billing, final installer and release. Preserve the original staged work, reuse the current PRs and one build target, keep housekeeping active, and distinguish implementation from verified production behavior. Do not run paid production AI tests before the required bounded test budget is authorized. Prepare concrete owner-dependent decisions while continuing independent work. Keep working through failures and report exact evidence and the remaining release blockers at each checkpoint.
