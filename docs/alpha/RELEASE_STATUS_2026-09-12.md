# Fanta release status — September 12, 2026

Fanta has working editor foundations, a connected production backend and AI Gateway chat, verified waitlist capture, and functioning GitHub builds. **It is not yet ready for a paid customer release.** The remaining work includes completing the agreed creative tools, validating them in the native app, activating production media workers and billing, and delivering a signed/notarized installer.

This is a dated handoff snapshot. GitHub, checkout state, the maintenance schedule, and public backend health were checked on September 12. Earlier native, account, and analytics results below come from retained release evidence; they were not repeated during this reporting session. No implementation, deployment, payment, or release was performed in this session.

Use [SOL_ULTRA_HANDOFF.md](SOL_ULTRA_HANDOFF.md) for the ordered continuation plan. Prefer newer exact-source evidence over historical paragraphs in the launch documents.

**Subsequent review correction:** [SOL_REVIEW_2026-09-12.md](SOL_REVIEW_2026-09-12.md) reproduces the lint failure, records 736 passing viewer tests and one failure, and confirms a numeric-preview defect. It also finds that all three retained toolbar screenshots show ChatGPT rather than Fanta. The visual milestone observations below are historical reported results, not independently substantiated visual acceptance. Fresh Fanta captures are required; other build/file evidence is separate.

## Source and deployment inventory

| Component | Verified source state | Meaning |
| --- | --- | --- |
| Original workspace | `/Users/jeanrojas/fanta-edit`, HEAD `d8610f9e3b4d5ed2e9d472e2bf692694adedddd0` | Contains protected staged work and extensive unstaged/untracked work. It is not the latest release checkout. |
| Desktop release checkout | `/tmp/fanta-edit-release-20260909`, branch `codex-release-backend-gateway`, HEAD `fa81bdbd50f763ab40ca190764c1964d1b54024f` | Published branch plus four uncommitted Motion files. |
| Desktop PR | [Prepare Fanta editing, Motion, and AI workflows for alpha release](https://github.com/jeanc18rlos/fanta-edit/pull/1) | Open draft; base is `perf/large-documents-ai-alignment`, not `main`. |
| Backend release checkout | `/tmp/fanta-backend-release-20260909`, branch `codex/launch-billing-integration`, HEAD `9510f61c906f46bc119dfd408985b9d6e19da17a` | Clean checkout; candidate is published for review. |
| Backend PR | [Connect account billing and native AI generation](https://github.com/jeanc18rlos/fanta-backend/pull/1) | Open draft against `main`; current four CI jobs pass. |
| Production backend | `https://api.fantaisa.net` | Last recorded deployment is `0d3753a`, deployment `dpl_CVnuDoWotpCv8v7SG3UyXrbzwvhQ`. The deployment identity was not re-resolved today. Public `/v1/status` returned `ok`, `families: []` at 14:18 UTC. |
| Waitlist | Last recorded landing head `4e6c9c9`, deployment `dpl_7SD2uTp6M2W37XABjH7qm2ynSGnU` | Previously deployed and verified with stored signup and matching analytics event; not rechecked today. |

The existing task **“Complete Fanta creative feature set”** contains the latest implementation history. Resume that task with the requested model setting; avoid starting a competing desktop implementation.

## Completed work and its limits

| Area | Completed and evidenced | Still missing |
| --- | --- | --- |
| Backend connection and AI Gateway | Fanta account credentials connect managed chat and media tools. New chats default to Fanta Claude Sonnet while existing selections are preserved. Managed Claude requests use the backend's Vercel AI Gateway route; the gateway secret remains server-side. Earlier production device-API checks proved streaming, a measured debit, and revocation. | Final native production sign-in, Keychain persistence, account/error recovery, and inference through the release installer. Do not treat an account badge as proof. |
| Backend reliability | Atomic generation submission/completion and billing, request-body binding, organization access/revocation guards, and renewable video links were implemented. Latest candidate adds guarded unclaimed-job recovery, verified paid-term grants, annual monthly allowances, and database encoding fixes. | Candidate `9510f61`, migrations `0020`/`0021`, and compatible worker rollout are not production-validated. Recovery after a permanent worker claim without durable output remains a gap. |
| Desktop AI workspaces | Image, Video, Vector, Design, and Masks surfaces are present. Deterministic native fixtures verified image polling/save/place, editable SVG, mask-to-inpaint handoff, video flows, visible prompts, source points, per-mode drafts, and restart recovery. Accepted and uncertain submissions retain durable identity and exact input. | Real production media inference and workers; multi-mask composition/enhancement completion; end-to-end Design generation/review/acceptance. Design currently prepares an unsent local Agent brief. |
| Core design editing | Native creation, editing, Undo/Redo, save/reopen, path selection, Scale/K, curve bounds, page rename/Code identity, and inspector fixes were exercised. Save As protects the original and rejects occupied destinations. Canonical saves preserve unchanged project bytes and media. | Final installer regression matrix covering the completed feature set, including prototype, variables, export, menus, and Git. |
| Git workflow | App-driven review, stage, commit, and push passed earlier QA. Bundled Git packaging and operation without a system Git installation have tests and retained verification. | Repeat on the final signed installer and a clean user environment. |
| UI refinement | The exact `9d92b30` native milestone verified a compact bottom toolbar centered in the canvas, Motion placement above the timeline, responsive collapse/recentering, flyout event isolation, searchable Actions, explicit Ask AI, paint visibility/Undo, gradient editing, and visible export success. | Complete light/dark, typography/scale, keyboard/focus, narrow-window, inspector, error/empty/loading-state review. Export preset configuration is still absent from the default visible inspector. |
| Text on Path and attachments | Source implements vector-to-text-path authoring, shaped curved text rendering/edit geometry, inspector edits, history/persistence protection, and bounded immutable selection snapshots attached to an unsent Agent draft. | Native and final-artifact checks. TextPath direction lacks a typed control; some typography controls are visibly inapplicable; start controls expose API-debug data. |
| Motion | Contextual seven-property keyframing, entrance presets and timeline synchronization are committed. Time comments capture clip/time, persist with one history entry, navigate by stable identity, and guard existing drafts. | Native validation; finish uncommitted auto-keyframe; remaining clip-editing/export flows. Motion Path is hidden and unimplemented. |
| Video | Native bounded playback, orientation, clipping, seeks, pause/replay, and non-destructive trim with original media/audio preservation were implemented. Right-edge scrubbing now restarts with one Play click; the regression failed before the fix and passed afterward. Native trim/Undo/Redo/reopen and exact saved bytes passed. | Variable playback rates, coherent motion/video export, audible playback and sustained resource checks, and final-artifact validation. |
| Payments | Account/billing links and safeguards are implemented. Latest candidate passes paid-term and real-database contention tests. A detailed pricing/entitlement proposal exists. | Catalog approval, currency consistency, payment-proof hardening identified in the proposal, Polar configuration, migrations/deployment, sandbox checkout/webhook lifecycle/portal verification, then production activation. Checkout remains disabled. No customer charge was made. |
| PostHog and leads | Access to US PostHog project `410640` was recovered. The live waitlist stored a QA contact and matching `waitlist_joined` event with submission ID/campaign attribution. Form validation and serverless analytics flushing were fixed. | Funnel from signup to activation and payment, release-ready landing copy/assets, operational monitoring, and an approved outreach campaign. Exclude the synthetic signup; no growth uplift has been established. |
| GitHub and installer | Automated desktop checks and Apple Silicon DMG packaging/upload work. Latest committed head has successful push and PR checks and a successful DMG build. Developer ID certificate preparation and GitHub signing-secret setup were recorded. | Distribution credentials/notarization steps were skipped in the latest build. Signed/notarized/Gatekeeper/clean-Mac validation and customer release publication remain open. |

## Exact current interruption point

The desktop checkout has **four uncommitted files**, totaling 2,151 insertions and 159 deletions:

- `crates/fig_viewer/src/gpui_adapters/toolbar.rs`
- `crates/fig_viewer/src/motion_edit.rs`
- `crates/fig_viewer/src/motion_panel.rs`
- `crates/fig_viewer/src/view.rs`

These implement Motion auto-keyframe and transient property editing. The prior task fixed a real precision bug: focusing and leaving a rounded numeric field could create a keyframe and quantize the value. The latest two recorded focused tests passed:

- `motion_auto_key_previews_live_commits_once_and_only_then_autosaves`
- `motion_auto_key_is_gated_and_disarms_at_context_boundaries`

The first test checks no-op focus, transient unsaved typing, one commit/Undo step, autosave after commit, and playhead transition. **The next package-scoped `./script/clippy -p fanta-gpui -p fig_viewer` exited 101.** The complete final diagnostic was not available in the bounded retrieved output. Reproduce and diagnose it; do not assume the full working tree is green or publish it as validated. No tests were rerun for this report.

The committed capability inventory still says auto-keyframe is unavailable. That is accurate for the published revision and stale for describing the uncommitted implementation. Update it only after the new slice has passed its gates.

## Verification ledger

| Evidence | Result | Boundary |
| --- | --- | --- |
| [Desktop PR check, `34542571688`](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34542571688) and [push check, `34542567398`](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34542567398) | Success at `fa81bdb` | Does not include the four uncommitted files. The separate diagnostic `native-video` job is dispatch-only; ordinary full checks include the media/native tests. |
| [macOS build, `34542567402`](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34542567402) | Packaging and bundle/checksum verification succeed at `fa81bdb` | Distribution-credential and Apple-notarization checks skipped; draft-release preparation skipped. Build success does not establish distribution readiness. |
| [Backend CI, `34468287843`](https://github.com/jeanc18rlos/fanta-backend/actions/runs/34468287843) | Four jobs succeed: control plane, GPU plane, Docker, real PostgreSQL | Recorded suite: 537 backend, 153 CPU worker and 17 PostgreSQL cases. Does not establish live GPU or payment execution. |
| Committed creative slice | Recorded coherent source gate: 726 viewer tests; 116 text unit tests plus one doctest; 21 focused renderer and five document TextPath tests; 297 tools checks; 598 shared GPUI checks; focused Agent tests and package-scoped Clippy | This historical gate predates uncommitted auto-keyframe work. Focused Agent counts overlap and should not be added into an invented total. |
| Native toolbar milestone | Exact `9d92b30` built in 7m14s and passed the native interactions above | Local isolated QA bundle reused some prior bundle components. It is not the final installer. |
| Video trim/replay | 649 editor, 11 media library and 11 native playback cases; traced and untraced; native one-click replay and byte-preserving Save | Earlier source revision, with original-workspace changes included in some local QA builds. |

## Remaining release blockers

1. **Finish the product contract.** Complete the agreed visible creative features and validate them natively. Major open surfaces include auto-keyframe, Dev/inspect/measure/annotation, Arrow/direct media placement, voice input, export configuration, Design AI acceptance, multi-mask workflows, and agreed motion/video editing. Do not quietly remove them to declare completion.
2. **Prove production AI.** Public health still reports no media families. Establish worker endpoints, model availability/licenses, compatible storage and deployment, final native account flow, and a bounded end-to-end matrix. Earlier HMAC/credential access blocks cannot be bypassed.
3. **Make payments safe and consistent.** Resolve the catalog/currency decision; harden product/payment verification and entitlement behavior; apply ordered migrations; validate sandbox checkout, renewals, cancellation, retries, reversals and the billing portal. Keep annual checkout off until its scheduler and allowance lifecycle pass.
4. **Produce the release artifact.** Complete owner-required Apple steps, sign and notarize the exact release source, verify Gatekeeper and bundled Git, and run clean-user/clean-Mac install, launch, authentication, editing and save checks. Review PR base/release-branch integration before tagging.
5. **Close material stability issues.** Retained memory growth needs attribution. Run controlled no-playback/playback comparisons before proposing a fix. No leak-free or faster-than-Figma claim is established.

## Pricing decision already prepared

The continuation task wrote [PRICING_PROPOSAL.md](/tmp/fanta-edit-release-20260909/docs/alpha/PRICING_PROPOSAL.md). Its **unapproved** recommendation is USD Pro $45/month for 3,000 credits, Team $89/month for 6,000 credits, and packs of 500/$9, 1,500/$24, and 5,000/$79. Free currently grants 500 credits once; do not advertise monthly replenishment. These are existing proposal values, not activated prices or a fresh financial recommendation.

The live legacy Polar Starter/Pro/Max catalog does not match backend grants. The backend seed and dashboard also disagree on currency presentation. Product names are insufficient mapping keys. The proposal further identifies metadata-preferred subscription fulfillment, insufficient pack payment verification, unenforced API/priority distinctions, and Team downgrade handling. Revalidate and resolve these findings before checkout activation. The proposal's fee/model-cost assumptions are dated; refresh primary pricing sources before asking for the final catalog decision.

## System housekeeping

The existing **Fanta daily housekeeping** automation is active at 04:00 Europe/Madrid. It closes only verified retired agent QA sessions, preserves user apps and active work, and runs Cargo cleanup on Sundays, below 40 GiB, or after a deferred clean becomes safe.

The September 12 registry records 227.35 GiB free, no active Fanta QA or local build/test, no Fanta zombies, and preservation of the user app from `github-d0bc2d7/untouched/Fanta.app`. Two Chrome-owned zombies were left to their active browser parent. No cleanup was due. A retired Cargo target previously yielded about 49.24 GiB of recovered space. `cargo clean` removes build files; it does not terminate or reap zombie processes.

Registry: `/Users/jeanrojas/.config/fanta/maintenance.json`. Re-identify processes by executable path and ownership every time; do not use a historical PID as authorization.

## Evidence and continuation references

- [Current capability matrix](/tmp/fanta-edit-release-20260909/docs/alpha/CAPABILITY_INVENTORY.md)
- [Current launch requirements](/tmp/fanta-edit-release-20260909/docs/alpha/LAUNCH.md)
- [Detailed validation history](/tmp/fanta-edit-release-20260909/docs/alpha/RELEASE_VALIDATION.md)
- [Pricing proposal](/tmp/fanta-edit-release-20260909/docs/alpha/PRICING_PROPOSAL.md)
- [Backend billing implementation notes](/tmp/fanta-backend-release-20260909/docs/billing-credits.md)
- [Backend dispatch recovery notes](/tmp/fanta-backend-release-20260909/docs/dispatch-recovery.md)
- Evidence root: `/tmp/fanta-release-qa-20260909`, especially `toolbar-milestone-9d92b30`, `github-17de5eb-hosted-installer`, `video-trim-validation/end-scrub`, and `inline-video-validation/native-video-memory`.

Paths under `/tmp` are local working material and may eventually disappear. Preserve the final evidence manifests, relevant logs, checksums and release artifacts in durable release storage before cleanup; never copy credentials into a repository or report.
