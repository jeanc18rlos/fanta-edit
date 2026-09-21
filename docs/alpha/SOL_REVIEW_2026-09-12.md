# Review of Sol's Fanta work — September 12, 2026

Sol made substantial progress on the editor, but the current working tree is **not ready to publish or release**. This review reproduced two failing source checks, confirmed a live-preview defect with an additional regression probe, and found that the retained toolbar screenshots show the wrong application.

## Scope and current state

Reviewed the 21 commits from `17de5eb` through `fa81bdb` and the four uncommitted Motion files, with deeper inspection of the new auto-keyframe interaction/persistence paths, toolbar integration, selected Agent attachment changes, and release evidence. This was a focused review, not an exhaustive audit of all 103 changed files and roughly 18,000 added lines in the committed range.

The known desktop and backend release heads and the uncommitted Motion diff still match the preceding handoff snapshot. No newer work was found in those locations or their latest GitHub runs. Work in an unreported folder or branch is outside this assessment.

| Item | Checked state |
| --- | --- |
| Desktop | `/tmp/fanta-edit-release-20260909`, `codex-release-backend-gateway`, `fa81bdbd50f763ab40ca190764c1964d1b54024f`, four uncommitted Motion files |
| Backend | `/tmp/fanta-backend-release-20260909`, `codex/launch-billing-integration`, `9510f61c906f46bc119dfd408985b9d6e19da17a`, clean |
| GitHub | Both desktop checks and DMG build at the committed head succeed. Backend's four CI jobs succeed. Both PRs remain drafts. These checks exclude the local Motion patch. |
| Public backend health | At 15:32 UTC, `/v1/status` returned `status: ok`, `families: []`; this does not establish functioning production media workers. |

## Findings

### 1. P2 — restoring the original numeric text leaves a stale canvas preview

Location: [motion_panel.rs:350](/tmp/fanta-edit-release-20260909/crates/fig_viewer/src/motion_panel.rs:350).

The `BufferEdited` handler previews only while the current text differs from `numeric_initial_text`. When the user returns the field to its original display text, it sets the edited flag to false but does not restore the original preview value.

An additional GPUI regression reproduced this sequence: the exact original X is `1.234567`, displayed as `1.23`; type `42`, then replace it with `1.23`. The final displayed text matches the original, but the evaluated canvas X is **1.2**, the last intermediate prefix preview. The original value is restored only when the edit closes. No persisted-data-loss claim is made; the defect is disagreement between the field and live preview.

Fix the return-to-original branch by restoring the session's exact original state while keeping the edit lifecycle coherent. Do not simply parse `1.23` and replace the higher-precision original. Add coverage for original → changed → original before Enter/blur, and preserve the existing no-op focus and autosave guards.

Evidence: [probe log](/tmp/fanta-sol-review-20260912/return-to-initial-probe.log), [reproduction patch](/tmp/fanta-sol-review-20260912/return-to-initial-probe.patch). The probe was temporary; `view.rs` was restored byte-for-byte afterward.

### 2. P2 — the full viewer suite fails in the newly extended toolbar test

Location: [toolbar.rs:2034](/tmp/fanta-edit-release-20260909/crates/fig_viewer/src/gpui_adapters/toolbar.rs:2034).

The current suite returns **736 passed, 1 failed**. `option_models_reach_the_panel_and_pushes_are_diff_guarded` fails at “a controlled toggle must wait for the host echo.” It also fails when run alone at scheduler seed 0, so this is not merely a full-suite ordering observation.

The assertion expects the toolbar to remain false after an update which has already delivered an accepted true value. This failure by itself does not prove an optimistic-state product bug. Correct the test boundary so it distinguishes component intent emission from host acceptance/echo, retaining coverage for rejected requests, accepted state and exactly one options push. Do not just delete the contract check.

Reproduce:

```sh
SEED=0 CARGO_TARGET_DIR=/Users/jeanrojas/fanta-edit/target CARGO_INCREMENTAL=0 cargo test --locked -p fig_viewer --features fanta-gpui-ui option_models_reach_the_panel_and_pushes_are_diff_guarded -- --nocapture
```

Evidence: [full suite](/tmp/fanta-sol-review-20260912/viewer-tests.log), [isolated failure](/tmp/fanta-sol-review-20260912/toolbar-regression.log).

### 3. P2 — required Clippy gate fails on a redundant clone

Location: [motion_edit.rs:987](/tmp/fanta-edit-release-20260909/crates/fig_viewer/src/motion_edit.rs:987).

The new test clones `exact` when inserting it into the fixture even though that value is not used afterward. The required scoped command fails with `clippy::redundant_clone` under denied warnings:

```sh
CARGO_TARGET_DIR=/Users/jeanrojas/fanta-edit/target CARGO_INCREMENTAL=0 ./script/clippy -p fanta-gpui -p fig_viewer
```

Move the value instead, then rerun the complete scoped script. This is a small test-code correction, but until it passes the patch has not met the repository's validation requirement. Evidence: [Clippy log](/tmp/fanta-sol-review-20260912/clippy.log).

### 4. Release evidence issue — all three toolbar screenshots show ChatGPT

The following actual files were opened and inspected:

- [Design screenshot](/tmp/fanta-release-qa-20260909/toolbar-milestone-9d92b30/native-design-toolbar.png)
- [Motion screenshot](/tmp/fanta-release-qa-20260909/toolbar-milestone-9d92b30/native-motion-toolbar.png)
- [Narrow-window screenshot](/tmp/fanta-release-qa-20260909/toolbar-milestone-9d92b30/native-toolbar-narrow.png)

All show the ChatGPT/Codex conversation window rather than Fanta. Their SHA-256 hashes match the entries in the retained `verification.json`, so these are the exact files attached to the checkpoint.

[RELEASE_VALIDATION.md:39](/tmp/fanta-edit-release-20260909/docs/alpha/RELEASE_VALIDATION.md:39) records visual observations of centering, timeline placement and responsive collapse and cites those screenshots later in the section. The screenshots cannot substantiate those observations. This does **not** invalidate independent build, document, export-file or accessibility-interaction evidence, nor prove that the interactions never occurred. It does mean the visual acceptance claims need fresh Fanta captures and inspection before relying on them.

Repeat visual QA on a clearly identified candidate, verify the captured pixels immediately, and save the actual Fanta window, revision and viewport context. Preserve the old evidence and mark the correction rather than overwriting its history. I have added a caveat to my preceding report, which had relied on the earlier record.

## What Sol completed

| Work | What exists now | Verification boundary |
| --- | --- | --- |
| Toolbar and commands | More editing commands exposed; compact bottom toolbar; responsive layout and explicit AI entry; export progress/results relayed to the canvas | Source and automated evidence exist. Retained visual screenshots need correction. |
| Paint inspector | Fill/stroke visibility, supported gradients and blend edits, undoable operations, explicit rejection of unsupported payloads | Included in source/automated checks; final native matrix remains incomplete. |
| Text on Path | Document representation, migration/persistence, vector conversion, curved shaping/rendering, editing geometry and bounded inspector support | Implemented and covered by source tests; final native and installer validation still required. |
| Agent attachments | Immutable bounded selected-node snapshots inserted into an unsent draft, with document/root identity and request framing | Source and focused tests exist; native attachment workflow remains unverified. |
| Motion | Contextual property keyframes, entrance presets, removal of the duplicate flyout, clip-anchored time comments with history/navigation | Committed. Final native validation remains open. |
| Auto-keyframe | Transient numeric/color property edits, one-step commit, autosave boundary and context gating | Uncommitted. Its two dedicated GPUI tests pass in the current full run, but the defects/check failures above remain. This is Motion-panel editing; do not infer automatic keyframing of every canvas manipulation. |
| Design AI | Clear signed-out flow that prepares an unsent Agent brief | This is not a complete generate/review/accept design workflow. |
| Release planning | Capability inventory, pricing proposal and updated launch/validation notes | Useful documentation; pricing is not approved or activated. |

The unchanged backend, existing Gateway route, recovered PostHog access and earlier video/Git work are continuing foundations; they should not be counted as newly deployed results from this Sol slice.

## Still missing, in priority order

1. **Finish the current patch:** correct the numeric-preview defect and the two failing checks, rerun the affected suite, update evidence, commit/push, and inspect exact-source CI.
2. **Native product validation and UI refinement:** actual Fanta screenshots; light/dark and font-scale coverage; TextPath, attachments, keyframes, timed comments and auto-keyframe; save/reopen and cancel/Undo throughout. TextPath direction and user-friendly placement controls remain incomplete.
3. **Complete agreed editor workflows:** Dev/inspect/measure/annotation, direct Arrow/media actions, voice input, visible export configuration, agreed video-rate/clip-editing and motion/video export. Keep the bounded scope from the handoff.
4. **Finish AI workflows and production readiness:** complete Design acceptance and multi-mask/composition/enhancement flows; configure and validate workers/model availability/storage; verify native account persistence and end-to-end inference. Keep paid production QA within the user's pending budget authorization.
5. **Billing:** approve/reconcile pricing and currency; resolve product/payment matching and entitlement gaps; apply pending migrations and deploy the validated candidate; pass provider sandbox lifecycle tests. Source inspection reconfirms metadata-preferred subscription mapping at [billing.ts:85](/tmp/fanta-backend-release-20260909/lib/billing.ts:85) and pack fulfillment without matching stored checkout/amount/currency at [billing.ts:158](/tmp/fanta-backend-release-20260909/lib/billing.ts:158). These are existing launch-hardening gaps, not a demonstrated live exploit. Checkout remains unvalidated for customers.
6. **Distribution and stability:** Developer ID signing, notarization/Gatekeeper, clean-Mac exact-installer tests, retained-memory attribution, rollback and release publication. Successful ad-hoc DMG packaging does not complete this step.
7. **Leads:** preserve the verified waitlist and recovered PostHog project; finish activation/payment funnel measurement, accurate launch assets and approved outreach. No measured conversion improvement is established.

## Review verification and preservation

New checks performed: scoped Clippy; full 737-test viewer suite; isolated seed-0 toolbar reproduction; one added GPUI preview probe; GitHub status and public health; inspection of all three toolbar images and their hashes. No native Fanta session, paid inference, payment, deployment or release was initiated.

No implementation fix was applied. The temporary probe was removed and the four-file desktop diff restored to SHA-256 `8839e570be3268e94de1483e76357aaf2795df0c1dfd57aae3faf691f8ae3db7`. The original staged diff remains `f450d6256b31380030251ac176be0a3cc029de2240555067794acf1fd57df2e8`. Backend source was unchanged. Review logs and the reproduction patch are retained under `/tmp/fanta-sol-review-20260912`.
