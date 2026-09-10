# Fanta launch readiness

Updated on 2026-09-10. First release target: Apple Silicon Mac.

## What is connected

- The production backend is `https://api.fantaisa.net`; `/v1/status` and
  `/v1/plans` respond successfully. Later browser-confirmed device API checks
  verified default Sonnet streaming and credit metering, as recorded below.
  Final native authentication, checkout, and GPU generation remain unverified.
- Fanta account credentials authenticate the managed Messages API. New chats
  default to Fanta Claude Sonnet; existing user-selected models are preserved.
  Managed Claude models go through the backend's Vercel AI Gateway route.
  The gateway key stays on the server.
- Backend media tools use the same account when signed in. Signing out stops
  that connection. Explicit custom server credentials still take precedence.
- The companion backend changes restore `/account` and upgrade links, make
  payment grants atomic and retryable, and fix current-subscription selection.
- Backend analytics now flush within the request lifetime, as required for
  [PostHog in serverless environments](https://posthog.com/docs/libraries/node#short-lived-processes-like-serverless-environments).

Video preview runtime `110868d` passes six real native decoder tests and all
606 editor tests. Generated MP4 results now have first-frame posters, oriented
canvas placement, cached Save/Play bytes and account-change guards. Negative
prompts follow the selected model's capabilities while preserving the draft.
Seven timing cases passed 20 iterations each. The local app build passed,
but native poster and installer QA remain pending after blocked sign-in, as
recorded below. Playback in that build uses the system player; production GPU
verification and planned video editing remain unfinished.
See the video-poster section of [RELEASE_VALIDATION.md](RELEASE_VALIDATION.md).

## Latest video validation — September 10

Backend video URL fix `0d3753a` was reviewed and passes **449 backend tests
across 44 files**, **78 CPU-only worker tests**, and type checking. It is **not
deployed**. URL renewal requires the backend and `videogen` worker with
matching R2 configuration; historical URL-only results remain unchanged.

The local poster app built successfully in **7m11s**, from runtime `110868d`
plus protected user-staged changes, and passed strict ad-hoc signature checks.
Normal local-fixture sign-in then stalled in `SecItemAdd` while writing to
Keychain. Automation could not interact with SecurityAgent, so manual user
approval was requested. The idle QA app and helper subsequently quit normally,
and the fixture stopped. No native poster visual, video Save/Play/Place or
video-project save/reopen result was obtained. A separate native playback
engine now passes seven library tests and five real playback cases, including
corrupt-first-frame rejection and cleanup. Its generation controls and canvas
integration remain unfinished; the app still uses the system player.
Evidence: `/tmp/fanta-release-qa-20260909/video-preview-validation/native/`
(`native-build.json` and `verification.json`).

The subsequent housekeeping check found only user app **21997** and helper
**22010**, zero Cargo/zombie processes and **60.88 GiB free**. Daily **04:00
Europe/Madrid** housekeeping remains scheduled, with Cargo cleanup on Sundays
when idle, below 40 GiB free, or when a deferred cleanup becomes safe. Active
work is preserved; the video fixture's port 47843 is closed.

## Required before accepting payment

Production Vercel has the AI Gateway, database, and Clerk settings. The
existing US PostHog project token and ingestion host are included in the
September 9 backend production deployment. Live analytics delivery still needs
an authenticated event check. Production still has **no Polar settings**.
Complete the configuration in the existing backend project, without putting
secret values in either repository:

The existing Polar account is accessible. Its live catalog currently contains
Starter ($20/month), Pro ($80/month), Max ($200/month), and one-time packs of
100/$5, 500/$20, and 2,000/$60. These **do not match** the backend plan and pack
definitions below. The inspected Starter product uses USD and grants the
legacy “AI Image Editor Access” benefit, with no credit metadata. A launch
catalog choice and credit allowance mapping are pending; do not connect these
product IDs to the current backend grants by name alone.

| Service | Configuration |
| --- | --- |
| Polar | `POLAR_SERVER=production`, `POLAR_ACCESS_TOKEN`, `POLAR_WEBHOOK_SECRET`, `POLAR_SUCCESS_URL` |
| Monthly subscriptions | `POLAR_PRODUCT_PRO_MONTHLY`, `POLAR_PRODUCT_TEAM_MONTHLY`; seed the matching product IDs into the plan rows |
| Optional credit packs | `POLAR_CREDIT_PRODUCT_SMALL`, `POLAR_CREDIT_PRODUCT_MEDIUM`, `POLAR_CREDIT_PRODUCT_LARGE` |
| PostHog | `POSTHOG_API_KEY` and `POSTHOG_HOST=https://us.i.posthog.com` included in the current production deployment; project `410640` matches the live landing page |
| Clerk | Verify the production instance and webhook secret; local environment snapshots contain a test instance and are not authoritative production configuration |

Companion backend migration `0018_subscription_event_order` was applied and
verified in production on September 9 at 20:13 UTC, after all 18 preceding
migration timestamps and hashes matched the committed history. It adds one
nullable timestamp column used to reject delayed subscription events; no seed
was run. Backend `9d05908` was then built from committed source, checked at its
candidate URL, and promoted to `https://api.fantaisa.net`. All nine public
checks passed after promotion, including database reads, missing-credential
rejection, and the three account/billing redirects. At that stage authenticated
AI, GPU requests, and customer checkout were unverified. Later device API
checks below verified default Sonnet streaming and metering; GPU generation
and customer checkout remain unverified.

Register Polar's webhook at `https://api.fantaisa.net/webhooks/polar` for
subscription lifecycle events and `order.paid`. Test a sandbox purchase,
webhook redelivery, cancellation, and a renewed subscription before enabling
customer checkout. Confirm the customer receives credits exactly once and can
open the billing portal. No real customer was charged during this work.

Existing catalog amounts are 24/month for Pro (3,000 credits), 40/month for
Team (6,000 credits), and credit packs of 500/1,500/5,000 for 5/12/35. The current
billing UI and tier documentation use EUR; price fields and the API carry
cents without a currency. USD references describe usage-cost and credit
accounting. Displayed prices and currency must match the chosen Polar products
before checkout is enabled.

Keep yearly product IDs unset for the first release. The current yearly
subscription implementation grants one monthly allocation per annual billing
period; monthly replenishment for annual subscribers is not implemented.

Review the Team allowance before selling it. At the default 1.4 billing
multiplier, 60 USD of fully consumed credits represents roughly 42.86 USD of
upstream cost before payment fees. Existing prices and credit amounts were
preserved, not approved as a sustainable launch offer.

## GitHub builds and installers

At **06:09 UTC**, both checks for published recovery head `02a32f6` had passed:
[push checks](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34441705781)
completed at 05:58:11 UTC and
[PR checks](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34441708551)
completed at 06:04:48 UTC. Its
[exact installer](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34441705786)
was pending behind the still-running `12ad6d3` installer. These checks include
`be2b282` durable recovery. Local native full-restart QA subsequently passed at
06:20:39 UTC; exact-installer and production authentication/media checks remain open.

At **05:26 UTC on September 10**, published desktop head `12ad6d3` had passed
both [push checks](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34438558490)
(05:20:34 UTC) and
[PR checks](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34438560024)
(05:20:22 UTC), including the save/media fixes and full viewer suite.
Its [exact installer](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34438558598)
was still building and packaging. The earlier
[eca2b86 installer](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34427356619)
passed at 04:57:38 UTC, including bundle/checksum checks and upload;
notarization was skipped. These are test installers, and final notarization
and clean-Mac validation remain required.

GitHub Actions is enabled. **The GitHub default `main` branch is still upstream
Zed** (`9064f26` when checked). The Fanta checkout starts at `d8610f9`, published
on `perf/large-documents-ai-alignment`. Merge the release changes into the
Fanta branch and intentionally select the Fanta release branch as the default
before relying on the repository home page or manual release workflow.

`Check` runs on pushes and pull requests. `Build macOS release` produces an
Apple Silicon DMG and checksum as run artifacts. Manual unsigned builds are
for testing. A narrow branch trigger also requests an unsigned packaging
build when app code, assets, Cargo/build scripts, or packaging files change on
`codex-release-backend-gateway`; docs-only changes do not rebuild. These branch
builds create test artifacts, not customer releases or draft releases. Signed builds and version tags require these
repository secrets:

- `MACOS_CERTIFICATE`: base64 Developer ID Application `.p12` certificate.
- `MACOS_CERTIFICATE_PASSWORD`: certificate password.
- `APPLE_NOTARIZATION_KEY`: App Store Connect API private key (`.p8` text).
- `APPLE_NOTARIZATION_KEY_ID` and `APPLE_NOTARIZATION_ISSUER_ID`.

`MACOS_SIGNING_IDENTITY` is optional when automatic identity selection works.
`FANTA_CLIENT_CHECKSUM_SEED` is optional telemetry configuration, not an AI key.
`MACOS_CERTIFICATE` and `MACOS_CERTIFICATE_PASSWORD` were configured and their
names verified on 2026-09-09. Apple issued Developer ID Application certificate
`ML3GCBU926` for team `SP6J7Q6M3J`, expiring 2031-09-10; the private-key match,
G2 certificate chain, and encrypted signing package were verified. The signing
material is backed up outside the repository with restricted file permissions.

Notarization credentials remain pending. App Store Connect requires accepting
its API-use terms before API access can be requested; that agreement is awaiting
the account holder's approval. After access is granted, a Team API key with the
Developer role supports the workflow. Its issuer UUID is separate from the
Developer Team ID. No notarization key has been created or stored yet.

The app now packages the complete Dugite Git 2.53.0 distribution, verified
against its published SHA-256, including helpers, templates, Git LFS, and Git
Credential Manager. The latest 410 MB candidate app passed deep, strict
ad-hoc signature verification and public HTTPS access with an empty `PATH`.
Clone/push/fetch/pull regressions also passed against the packaged Git. This
does not replace Developer ID signing, notarization, or a clean-Mac installer
check.

Packaged alpha builds use manual DMG updates. Automatic update checks are
disabled, and the default menus and command palette do not expose a
**Check for Updates** command. Keep automatic updates disabled until the installer’s mounted
volume name, prerelease comparison, and compiled release-channel selection
are corrected and an end-to-end upgrade passes. On-demand SSH remote-server
downloads are separate; this first Apple Silicon workflow has no verified
SSH remote-server artifact.

A `v*` tag must match the version in `crates/zed/Cargo.toml`. Successful tag
builds prepare a **draft** GitHub release. Publish only after checking the DMG
on a clean Mac: install, launch, sign in, make a design, run an AI edit, save,
close, reopen, and sign out. Keep the separate crash-symbol artifact.

## Check the deployed connection

Run `./script/smoke-backend` for public health and plan checks. Set
`FANTA_API_KEY` through the environment to include authenticated model,
credit-balance, and MCP discovery checks. The script does not print the key or
account history. Use a dedicated test account.

`./script/smoke-backend --chat` additionally sends one metered, 64-token
streaming request and checks its credit debit. This is an explicit paid-call
option; the default run makes no AI call. Pass `--base-url` for a local or
staging backend. A public-only pass is not a release sign-off.

The gateway's public catalog contains the shipped Claude models. The two
older Grok mappings in the backend seed were absent when checked; keep them
out of the release's advertised model list until their routes are verified.
Media generation needs its own successful live job; backend health alone
does not validate GPU availability.

The September 9 infrastructure check found deployed image and video apps, but
the health probe rejected the locally available signing credential. Production
segmentation and SVG endpoint rows still point at localhost, and corresponding
Fanta worker deployments are absent. Segmentation, vector, and compose model
weights are also missing from the inspected worker volume. Source fixes and
mock tests do not resolve this deployment gap. Verify the intended production
signing configuration, deploy the required workers and weights, and update only
their intended endpoint rows before advertising these capabilities.

## Lead capture and launch message

The existing [Fanta Sales Funnel](https://us.posthog.com/project/410640/dashboard/1546459)
is accessible in PostHog's **US region**, project `410640`. The EU login opened
an organization-creation screen; switching to US recovered access to the
existing project without creating an account. Keep `POSTHOG_HOST` set to
`https://us.i.posthog.com` for this project.

The existing six-step landing funnel requires product interaction before
counting a signup. In its September 2–9 window, it shows 13 entrants, 11 engaged
visitors, and zero reaching the product-interaction step. This is not proof
that nobody joined the waitlist: visitors may skip optional steps. Measure a
direct landing-to-`waitlist_joined` funnel alongside this behavioral breakdown.

The public interest page is [fantaisa.net](https://www.fantaisa.net/#waitlist),
served by `jeanc18rlos/fantaisa-landing`. Its production waitlist currently
persists email, attribution, and submission identifiers in this PostHog project;
no separate waitlist webhook or email-list service is configured. The initial
inspection submitted no test lead. The later live waitlist correction verified
one reserved-domain QA signup and its stored contact/event, as recorded below;
exclude that synthetic record from customer counts.

The additional direct signup metric was not saved; dashboard changes await
user approval. The existing dashboard is unchanged.

Keep the first offer specific: an Apple Silicon Mac alpha for designers who
want editable source and agent-assisted canvas changes.

Suggested copy for the existing page:

> **Your design is code.**
>
> Fanta is a Mac design app that saves your canvas as editable, git-tracked
> source. Import a Figma file, draw on the canvas, and work with an AI agent
> in the same project. Review what changed and keep control of the files.
>
> **Join the Mac alpha.** Leave your email to hear when the next build is
> ready. Apple Silicon first; this is an early release with known limitations.

Collect email plus an optional role/use-case field; make the expected follow-up
clear. Use source tags on the same interest-page link for the personal website,
GitHub README, and launch posts. Measure page visits, successful interest
submissions, activated accounts, first successful AI use, and paid conversions.
Backend billing events are `backend_subscription_credited` and
`backend_credit_purchase_completed`; these report committed credit grants,
not authoritative revenue. Use Polar for revenue reconciliation.

Start with a short real demo: import a design, make a canvas edit, ask an agent
for a change, and show the source diff. Prepare posts for the personal site
and relevant design communities, and invite the existing opted-in waitlist
after the installer and payment checks pass. No outreach was sent or published.

## Durable generation recovery

Desktop source `be2b282` persists generation recovery locally,
scoped to the normalized API endpoint plus the verified `/v1/me` user and
organization IDs. It saves the submitted JSON and idempotency key before a
generation POST; explicit Retry reuses that body/key, while reopening issues
no automatic paid POST. Each scope allows 32 unfinished generations, including
up to 8 unconfirmed submissions, plus 12 completed records within a 64 MiB
journal cap. Completed SVG results are retained, but uncertain vector Messages
requests are not recovered and `/v1/messages` is not replayed.

Backend submission source `830f702` is now live as
`dpl_12FMtF55tpV1pJnyr93nx4pPz6BS`. All 438 tests in 44 files, 60 focused tests,
type checking and [source CI](https://github.com/jeanc18rlos/fanta-backend/actions/runs/34441329043)
passed. Migration `0019_generation_request_hash` was applied at 05:45:37 UTC
on September 10, after the preceding 19 history entries matched; the resulting
20-entry history and nullable text column were independently verified at
05:49:25 UTC. No seed ran or customer rows were read by those checks. Nine
candidate and nine live public checks passed without inference. Direct lookup
of `api.fantaisa.net` confirmed this production deployment.

A production API check completed at **06:09:09 UTC**: device sign-in passed,
the same deliberately invalid generation request returned HTTP 400 with
`invalid_request_error` and `x-fanta-generation-unreserved: true` twice, and
the credit balance stayed **387 → 387**. The temporary key was revoked and
subsequently rejected with HTTP 401. Credentials stayed in process memory;
no inference, checkout or native application was used. Evidence:
`/tmp/fanta-release-qa-20260909/generation-submission-validation/production/live-rejection-verification.json`.

The full desktop viewer suite passed all 596 tests, and ten new recovery cases
each passed 20 scheduler iterations. Root and independent peer review passed.
The local native build at `be2b282` completed successfully in **4m46s**.
Both hosted checks at `02a32f6` passed by 06:09 UTC. **Local native full-restart
recovery QA passed at 06:20:39 UTC.** Across two full restarts, opening the same
profile issued no generation POST, status read or Messages call before an
explicit action. Accepted jobs resumed through GET only; a dropped inpaint
response recovered the same job using the exact key, body, mask and source.
The run created three unique jobs with four generation POSTs and one Messages
call. SVG bytes matched before and after restart, the recovered PNG matched
the fixture, and the original orange 512×512 source, white edit mask and SVG
were restored. All three QA app sessions quit normally, and the fixture stopped.

This used the same ad-hoc signed local bundle/profile, including protected
user-staged changes. It does not validate the hosted installer, production GPU
or production Keychain persistence. Exact-installer verification remains
pending. Evidence is in
`/tmp/fanta-release-qa-20260909/generation-journal-validation/native/verification.json`.
See the
[recovery validation record](RELEASE_VALIDATION.md#durable-generation-recovery--september-10)
for the marker, legacy-record and provider-dispatch limits.

## Review and validation

- [Desktop integration and macOS builds](https://github.com/jeanc18rlos/fanta-edit/pull/1) — draft PR based on the existing Fanta branch.
- [Backend billing and analytics](https://github.com/jeanc18rlos/fanta-backend/pull/1) — draft PR based on backend `main`.
- Desktop account/provider checks, earlier 37 native generation tests, 34 inspector tests, and the focused Sidebar regressions passed. The final full Sidebar suite passed all 143 tests; six corrected fixtures each passed 20 scheduler iterations, with their behavioral assertions retained. Generation now accepts backend string seeds and sends integer segmentation coordinates; sidebar selection follows the same item through list updates. Four selection tests passed 20 scheduler iterations each.
- Atomic-file source `48171ea` passed 187 format tests (two ignored), seven generation-media tests and 28 generation-workspace tests after six baseline failures and two passing controls. Root and peer review passed. Failed writes preserve existing outputs and project files; publication is atomic per file, not for the whole project.
- Save source `5d3826d` passed all nine new cases and the full 45-test document suite; all nine GPUI cases passed 20 scheduler iterations each. Four baseline failures proved stale-save cleaning, lost editing sessions and overlapping writes; five additional controls cover ordering and cancellation. Writes now stay in capture order and newer edits/media remain dirty with autosave rearmed. A canceled first materialization refuses an unadopted existing folder rather than overwriting it. CodeWorkspace's source-path fix passed all 14 tests and its three new cases across 20 iterations each, following two baseline failures and one passing unchanged-editor control. The full viewer suite now passes all 577 tests (16.07s), and both corrected autosave fixtures pass 20 iterations each. Local CI commit `0edf122` runs this full suite once instead of five overlapping filters; actionlint passed. The save/media/CI commits `48171ea`, `5d3826d` and `0edf122` are published in `12ad6d3`, whose two hosted checks passed by 05:26 UTC. **Pending:** exact-installer validation.
- The `5d3826d` native build completed in 280.96 seconds. Page rename updated the Code path; move/Undo/Redo ended at x=88, y=348, width=100, height=60; K activated Scale. Explicit Save, quit and reopen preserved all three complete curves and the renamed Code path, with all 15 saved-project hashes and the original fixture unchanged. Both QA apps quit normally. The build includes preserved user-staged changes; native QA did not force write failures or call production authentication, AI/media or payments. The final cleanup check found only user app 21997/helper 22010, no Cargo or zombie processes, and 87.27 GiB free.
- Save As now passes 9 document/view/picker-hook regressions, including destination validation before opening a worktree; the new regression passed 20 scheduler seeds, following an earlier sweep of the other GPUI cases. The full custom-picker suite passed 9 tests with one existing Windows test ignored; the existing autosave regression also passed. CI retains custom-picker coverage and the local `0edf122` change includes Save As in the full viewer suite. The 5m29s native build passed cancel, sibling default, copy adoption/edit/reopen, all 15 original-file hashes, occupied-destination rejection, and autosave recovery. The final 5m45s build also passed rejection with exactly one unchanged source tab, post-error autosave, and valid Save As/reopen of a 208×160 copy; all 15 original hashes remained unchanged. Hosted checks through `f03a26d5` pass; exact-installer verification remains pending.
- An isolated CoreText ownership reproducer left one 80-byte array per query with the extra retain (16/128 queries), and zero scanner leaks in matched Create-ownership controls. The subsequent macOS text-system suite passed 6 tests, and 2 feature-ownership/font-usability tests passed; both suites are included in CI. These tests do not verify feature shaping. The 5m29s native build has completed independent allocation review: the prior font-array and callback/list scanner signatures are absent, while the final snapshot still flags 314 blocks / 19,936 bytes across 20 roots. Framework XPC/accessibility findings remain unresolved. Font correction `8ab306a` has local and hosted validation; exact installer validation remains pending. This does not establish whole-app leak freedom or behavior across supported macOS versions.
- The earlier backend deployment at `9d05908` included the verified additive migration. Nine public smoke checks passed; the previously broken account, upgrade, and trial URLs now redirect to the dashboard/billing pages. The separate local smoke script passed a mocked account, MCP, streaming, and credit-debit flow.
- Backend account corrections at `6fae7a8` passed all 369 tests, type checking, [GitHub CI](https://github.com/jeanc18rlos/fanta-backend/actions/runs/34421837511), and the Vercel production build. Deployment `dpl_7snZZNo5pfUkroT4axaGNPQZT9ee` was promoted; nine public checks passed before and after promotion. A fresh browser sign-in showed Pro/Owner/389 credits and checkout still unconfigured. No new migration, payment configuration, or charge was performed.
- [Backend GitHub CI passed](https://github.com/jeanc18rlos/fanta-backend/actions/runs/34380169196) at `9d05908`, including GPU tests and Docker. Desktop [push checks](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34404096365) and [pull-request checks](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34404101640) passed at Git-commit-fix source `bccc5fd`, verified at 21:34 UTC. Documentation head `95bbef4` and callback/list memory-fix source `96d366a` also passed both checks. The `0a2ae52` test installer passed package and native smoke checks. The `d0bc2d7` installer completed at 22:22 UTC and passed downloaded checksum, embedded revision and strict signature checks; the `bccc5fd` installer has also completed and passed package, signature, revision, and bundled Git checks without being launched. Its disk image was unmounted normally. The `96d366a` queued installer was superseded; [4a8f3d9](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34419050153) completed. The `20d7108` installer (`34423710706`) completed successfully; the `eca2b86` installer (`34427356619`) also passed at 04:57:38 UTC, with notarization skipped. Both hosted checks at `f03a26d5` pass, including Save As/font and generation network recovery. Both hosted checks at `20d7108` pass, including generation-contract and sidebar-selection changes. Both hosted checks at `eca2b86` passed: [pull-request check](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34427359223) and [push check](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34427356711). The subsequent `ee8e14d` fast-input, `af4cb0f` Scale, `ca56b0a` Sidebar-fixture/CI, `16533e6` K-shortcut, `dcf27d6` viewport-correction, and `849685f` history-overlay commits have passed local validation and both `9ff7a77` hosted checks: [PR check](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34434788518) and [push check](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34434784764). The combined `ca56b0a` native candidate passed the interaction checks below but exposed viewport clipping. The `dcf27d6` native build passed in 6m42s and its viewport, save/reopen, and K-shortcut checks passed. Undo exposed stale editing handles; `849685f` corrects the overlay refresh and passes automated checks. Its 4m36s build passed native Undo/Redo handle alignment and save/reopen. At the earlier prepublication check, the `9ff7a77` installer (`34434784877`) was pending and `eca2b86` (`34427356619`) was still building. The former was superseded by the later push and the latter passed. Both checks at published `12ad6d3` now pass; its exact installer remained in progress at 05:26 UTC. Running builds were preserved. The `0a2ae52` installer lacks the final inspector-selection fix; the candidate app has now passed native hidden-page rendering and complete bundled Git checks. The later inspector-selection correction also passed native verification in the 5m30s build. A notarized installer, final native authentication, real production media, and end-to-end payment still require verification.

The subsequent generation-network correction passed 33 tests, including six timeout/recovery regressions across 20 scheduler seeds each. API and media transfers now have deadlines while retaining the original submission key and saved-output recovery. Production inference and final installer verification remain required.

The waitlist correction at `4e6c9c9` is live and passed both hosted checks. Fifteen offline tests and live invalid-address, pending-input, and confirmation checks passed. One reserved-domain QA signup was retrieved in PostHog with its contact properties, canonical event, matching submission ID, and campaign attribution. This verifies basic lead capture; exclude the synthetic record from customer counts. No conversion improvement is established.

See [the current validation record](RELEASE_VALIDATION.md) for later successful
GitHub checks, native generation tests, UI observations, and measured memory
changes. The preceding local native build used `ca56b0a` and passed in 4m41s. Fast input, anchor editing, proportional Scale and strokes, Undo/Redo, and save/reopen passed, but point editing exposed viewport clipping. The `dcf27d6` build passed in 6m42s. Native positive/negative overflow, exact geometry and viewport restoration on Undo/Redo, save/reopen, and the K shortcut passed; Undo exposed a separate stale-handle overlay. The `849685f` refresh correction passes automated checks and the 4m36s native build passed immediate Undo/Redo handle alignment and save/reopen. The earlier 4m39s candidate passed curve rendering/acquisition and authored-curve preservation on save but exposed the fast-drag and anchor-delete defects subsequently corrected. The earlier 5m45s build included the verified Save As focus correction; the completed font allocation review used the preceding 5m29s build, as described above. The earlier 10m27s build included two targeted memory fixes. Repeated project open/close, chooser cancellation, new-design save, and native alert checks passed; independent scans no longer show the traced callback/list retain cycles. Other scanner findings remain unresolved. Retired QA sessions were closed, and the retired secondary Cargo cache was cleaned while preserving the current target and user-active installer. Retired disk-image mounts were unmounted; downloaded images and app copies remain available. Earlier builds include the 5m30s inspector candidate and 7m19s rendering
and complete-Git candidate. A fresh
128 MB UI-kit import showed rendered Icons content by the first observation
at 14.544 seconds; this is a single upper-bound observation, not a benchmark
or Figma comparison. Grid/Icons switching, a visible fill edit, Undo/Redo, and
saving passed. Closing released document-sized live allocations; sustained
overall memory stability is unproven. The earlier 5m30s inspector candidate also passed saved-design
reopening and inspector selection checks: changing pages clears selection,
clicking the same page preserves it, and saved source remains unchanged. Local fixtures verify
interactions, not production inference or billing.

The review branches contain only these release fixes. Pre-existing local
design, import, GPU, and motion work was preserved.

Production device authorization and the default Sonnet stream passed against `6fae7a8`: the expected response completed, one credit was debited (389 → 388), and both API and dashboard usage agreed. The temporary key was revoked and then rejected with 401. This used an API harness; final native sign-in/persistence, media jobs, and payment still need validation. Subsequent organization-selection and removed-member access fixes at `2ef667b` pass 384 tests and type checking; GitHub CI, Docker/GPU checks, and the production build passed. Deployment `dpl_Cj8VnhhFguMmjU2qFe9yVpEYKTbp` passed nine public checks before and after promotion; the signed-in billing page shows Pro/Owner/388 credits and checkout unconfigured.

Earlier backend `a2bb2bb` passed 396 tests, type checking, GitHub CI, Docker/GPU checks, and its production build. REST and MCP production catalogs excluded mock models; three direct mock requests returned 400 with no credit debit. A default Sonnet stream then passed with one credit debited (388 → 387), usage recorded, and the temporary key revoked and rejected. These API checks do not establish final native sign-in, real media generation, all models, or customer payment.

Previous runtime `42a4885` was deployed as `dpl_5484e7wjBBFjX7W2DguXBw5cHay6`, with backend PR documentation head `abc3d17`; current runtime `830f702` is deployed and verified as recorded in Durable generation recovery above. Its concurrent generation completion/billing fix passed all 408 tests in 43 files, type checking and [GitHub CI](https://github.com/jeanc18rlos/fanta-backend/actions/runs/34436154574). The documentation head also passed all jobs in [CI run 34436789383](https://github.com/jeanc18rlos/fanta-backend/actions/runs/34436789383). Nine public checks passed on the candidate and nine on the live domain. No new paid calls or migration were used for this promotion. Final native authentication, production media and customer payment remain unvalidated.

Path Selection edits existing anchors, handles, and segment endpoints. Commit `ee8e14d` fixes drags delivered before repaint and routes Backspace/toolbar Delete to selected anchors. Commit `af4cb0f` implements proportional Scale handles, including nested geometry, text and dimensional styles, with Undo/Redo and cancellation; `16533e6` connects the K shortcut. The fast-input baseline failed all five new checks; Scale's revised baseline failed 14 engine checks and its native check, with three engine controls passing. The five fast-drag/delete regressions and the Scale native regression each passed 20 scheduler iterations.

The old `eca2b86` native candidate rendered/acquired curves and preserved authored curves on save, but quick drags did not move objects and anchor Backspace deleted the whole layer. The combined `ca56b0a` native candidate passed fast input, anchor editing, Scale with proportional stroke changes, Undo/Redo, and save/reopen. It also exposed clipping after point edits.

Commit `dcf27d6` records edited vectors with `NodeFlags::UNCLIPPED_VECTOR`, preserving opaque extension metadata and keeping canvas rendering and export bounds consistent after save/reopen. It supersedes the intermediate metadata-marker approach. The viewport revision passed 337 document tests with one benchmark ignored, 280 editing-tool tests, 6 viewport render tests, and 24 native toolbar adapter tests. Both GPUI pixel/save/reopen regressions passed 20 scheduler iterations each; the ordinary raster pixel guard passed once. These results cover the final flag-based source, not just the earlier candidate.

The `dcf27d6` native viewport and K-shortcut checks passed after the 6m42s build. That candidate restored saved geometry correctly on Undo but left editing handles stale until pointer input. Commit `849685f` refreshes cached path-editing overlays after successful Undo/Redo through a read-only hook, without changing geometry, selection, gesture state, or history. Both NodeEdit and Path Selection regressions failed at exact overlay positions before the fix. The final 280 editing-tool tests and 26 toolbar adapter tests pass, and both new GPUI regressions pass 20 scheduler iterations each. The `849685f` build passed in 4m36s, followed by native immediate Undo/Redo handle alignment and save/reopen with an unchanged source hash. Text on Path remains a placeholder. The full Sidebar suite passed all 143 tests, and six corrected fixtures each passed 20 scheduler iterations. Commit `ca56b0a` adds full Sidebar and toolbar CI coverage and passed actionlint; the fixture corrections add no Sidebar runtime change. Both hosted checks at `9ff7a77` passed for this editing batch, and published `12ad6d3` also passed both checks. The exact `12ad6d3` installer remained in progress at 05:26 UTC; final installer verification is pending.
