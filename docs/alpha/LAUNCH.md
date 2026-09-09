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

Production Vercel currently has the AI Gateway, database, and Clerk settings.
It has **no Polar or PostHog settings**. Set these in the existing backend
project, without putting secret values in either repository:

| Service | Configuration |
| --- | --- |
| Polar | `POLAR_SERVER=production`, `POLAR_ACCESS_TOKEN`, `POLAR_WEBHOOK_SECRET`, `POLAR_SUCCESS_URL` |
| Monthly subscriptions | `POLAR_PRODUCT_PRO_MONTHLY`, `POLAR_PRODUCT_TEAM_MONTHLY`; seed the matching product IDs into the plan rows |
| Optional credit packs | `POLAR_CREDIT_PRODUCT_SMALL`, `POLAR_CREDIT_PRODUCT_MEDIUM`, `POLAR_CREDIT_PRODUCT_LARGE` |
| PostHog | Existing project `POSTHOG_API_KEY` and matching region `POSTHOG_HOST` |
| Clerk | Verify the production instance and webhook secret; local environment snapshots contain a test instance and are not authoritative production configuration |

Register Polar's webhook at `https://api.fantaisa.net/webhooks/polar` for
subscription lifecycle events and `order.paid`. Test a sandbox purchase,
webhook redelivery, cancellation, and a renewed subscription before enabling
customer checkout. Confirm the customer receives credits exactly once and can
open the billing portal. No real customer was charged during this work.

Existing catalog amounts are 24/month for Pro (3,000 credits), 40/month for
Team (6,000 credits), and credit packs of 500/1,500/5,000 for 5/12/35. Confirm
currency: the built-in billing page currently displays EUR, while the backend
pricing documentation describes USD. Polar product currency and displayed
prices must agree before checkout is enabled.

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
for testing. Signed builds and version tags require these repository secrets:

- `MACOS_CERTIFICATE`: base64 Developer ID Application `.p12` certificate.
- `MACOS_CERTIFICATE_PASSWORD`: certificate password.
- `APPLE_NOTARIZATION_KEY`: App Store Connect API private key (`.p8` text).
- `APPLE_NOTARIZATION_KEY_ID` and `APPLE_NOTARIZATION_ISSUER_ID`.

`MACOS_SIGNING_IDENTITY` is optional when automatic identity selection works.
`FANTA_CLIENT_CHECKSUM_SEED` is optional telemetry configuration, not an AI key.
None of these GitHub secrets was configured when checked.

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

Confirm the public interest-page URL before publishing a download or campaign.
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
