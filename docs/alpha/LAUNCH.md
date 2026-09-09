# Fanta launch readiness

Verified on 2026-09-09. First release target: Apple Silicon Mac.

## What is connected

- The production backend is `https://api.fantaisa.net`; `/v1/status` and
  `/v1/plans` respond successfully. This does not prove authenticated AI,
  checkout, or GPU generation works in production.
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
rejection, and the three account/billing redirects. Authenticated AI, GPU
requests, and customer checkout remain unverified.

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
no separate waitlist webhook or email-list service is configured. No test lead
was submitted.

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

## Review and validation

- [Desktop integration and macOS builds](https://github.com/jeanc18rlos/fanta-edit/pull/1) — draft PR based on the existing Fanta branch.
- [Backend billing and analytics](https://github.com/jeanc18rlos/fanta-backend/pull/1) — draft PR based on backend `main`.
- Desktop account/provider checks, 27 native generation tests, 36 document tests, 34 inspector tests, and four new Sidebar regressions passed. The full Sidebar suite has 133 passes and seven existing failures; it is not fully green.
- Save As now passes 9 document/view/picker-hook regressions, including destination validation before opening a worktree; the new regression passed 20 scheduler seeds, following an earlier sweep of the other GPUI cases. The full custom-picker suite passed 9 tests with one existing Windows test ignored; the existing autosave regression also passed. CI now runs both focused Save As filters. The 5m29s native build passed cancel, sibling default, copy adoption/edit/reopen, all 15 original-file hashes, occupied-destination rejection, and autosave recovery. The final 5m45s build also passed rejection with exactly one unchanged source tab, post-error autosave, and valid Save As/reopen of a 208×160 copy; all 15 original hashes remained unchanged. Final hosted checks and exact-installer verification remain pending.
- An isolated CoreText ownership reproducer left one 80-byte array per query with the extra retain (16/128 queries), and zero scanner leaks in matched Create-ownership controls. The subsequent macOS text-system suite passed 6 tests, and 2 feature-ownership/font-usability tests passed; both suites are included in CI. These tests do not verify feature shaping. The 5m29s native build has completed independent allocation review: the prior font-array and callback/list scanner signatures are absent, while the final snapshot still flags 314 blocks / 19,936 bytes across 20 roots. Framework XPC/accessibility findings remain unresolved. Font correction `8ab306a` has local validation; final hosted checks and exact installer validation remain pending. This does not establish whole-app leak freedom or behavior across supported macOS versions.
- 365 backend tests passed across all 42 test files, with type checking and 73 isolated GPU tests passing.
- The production backend is now `9d05908`, after the verified additive migration. Nine public smoke checks passed; the previously broken account, upgrade, and trial URLs now redirect to the dashboard/billing pages. The separate local smoke script passed a mocked account, MCP, streaming, and credit-debit flow.
- [Backend GitHub CI passed](https://github.com/jeanc18rlos/fanta-backend/actions/runs/34380169196) at `9d05908`, including GPU tests and Docker. Desktop [push checks](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34404096365) and [pull-request checks](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34404101640) passed at Git-commit-fix source `bccc5fd`, verified at 21:34 UTC. Documentation head `95bbef4` and callback/list memory-fix source `96d366a` also passed both checks. The `0a2ae52` test installer passed package and native smoke checks. The `d0bc2d7` installer completed at 22:22 UTC and passed downloaded checksum, embedded revision and strict signature checks; the `bccc5fd` installer is building, with the [96d366a installer](https://github.com/jeanc18rlos/fanta-edit/actions/runs/34412799333) pending behind it. The subsequent Save As/font changes still require hosted and native installer validation. The `0a2ae52` installer lacks the final inspector-selection fix; the candidate app has now passed native hidden-page rendering and complete bundled Git checks. The later inspector-selection correction also passed native verification in the 5m30s build. A notarized installer, authenticated production AI/media request, and end-to-end payment still require verification.

See [the current validation record](RELEASE_VALIDATION.md) for later successful
GitHub checks, native generation tests, UI observations, and measured memory
changes. The final local native build passed in 5m45s, including the verified Save As focus correction; the completed font allocation review used the preceding 5m29s build, as described above. The earlier 10m27s build included two targeted memory fixes. Repeated project open/close, chooser cancellation, new-design save, and native alert checks passed; independent scans no longer show the traced callback/list retain cycles. Other scanner findings remain unresolved. Retired QA sessions were closed, and the retired secondary Cargo cache was cleaned while preserving the current target and user-active installer. Retired disk-image mounts were unmounted; downloaded images and app copies remain available. Earlier builds include the 5m30s inspector candidate and 7m19s rendering
and complete-Git candidate. A fresh
128 MB UI-kit import showed rendered Icons content by the first observation
at 14.544 seconds; this is a single upper-bound observation, not a benchmark
or Figma comparison. Grid/Icons switching, a visible fill edit, Undo/Redo, and
saving passed. Closing released document-sized live allocations; sustained
overall memory stability is unproven. The final build also passed saved-design
reopening and inspector selection checks: changing pages clears selection,
clicking the same page preserves it, and saved source remains unchanged. Local fixtures verify
interactions, not production inference or billing.

The review branches contain only these release fixes. Pre-existing local
design, import, GPU, and motion work was preserved.
