# Design: Internal operations platform

## Approach

Most of the platform is configuration: vendor accounts, their own
integrations and a handbook. Code is written only where no integration
exists, and almost all of it goes into fanta-backend v2 as two modules on
the existing outbox ([ADR 0029](../../docs/decisions/0029-deliver-business-events-through-the-backend-outbox.md)):

- `v2/src/fanta_v2/ops.py`: the alert catalog, the Slack handler, the
  morning digest and the budget feed.
- `v2/src/fanta_v2/support.py`: ticket intake, Linear, replies and Attio
  notes.

Elsewhere the changes are small: one workflow in fanta-edit, a consent
prompt and an event sender in the editor, Sentry and PostHog set-up in the
web apps, and one page in the founder console. Once-a-day jobs use the
pattern of `sweep_crm_nightly` (`core/crm.py`): a tick sweeper that keeps its
last run in `system_settings`.

### Phase 0: accounts, wiring and disclosure (R4, R6, R18, R21, R22, R26, R28)

The founder works through a checklist in the handbook
(`procedures/setup.md`). Agents prepare each step and verify it afterwards.

1. **Privacy page first** (fantaisa-landing `src/app/privacy/page.tsx`): it
   names Slack, Linear, Attio, Sentry, Better Stack, Discord, PostHog and
   Resend, and what each receives.
2. **Slack:** a free workspace with five channels: `#ops-action` (needs
   you), `#ops-revenue`, `#ops-growth`, `#ops-support` and `#ops-digest`.
   One custom app, "Fanta Ops", gives an incoming-webhook URL per channel.
   Vendor apps: GitHub, Vercel, Linear, Sentry and Better Stack, six of the
   free plan's ten.
3. **Linear:** join the workspace that the engineering program recreates,
   and add the Support team with a Triage state and `source:*` labels.
   Connect GitHub for pull request links and close-on-merge (R18), and sync
   GitHub issues from public repositories into Support's triage.
4. **Better Stack:** seven monitors (both APIs' health routes, fantaisa.net,
   app, admin, docs and the release feed) and two heartbeats (the minute tick
   and the outbox drain), alerting `#ops-action`.
5. **Sentry:** projects for backend v2, backend v1, the web apps and the
   desktop app.
6. **Polar backstop:** a second webhook endpoint in Slack format for
   `order.paid` and `order.refunded`, posting to `#ops-revenue`. It still
   reports payments when our backend is down.
7. **Attio:** an API key limited to the objects the sync writes, stored in
   Secret Manager and mounted on Cloud Run as `ATTIO_API_KEY`. The founder
   runs the `gcloud run services update` command. `sync-vercel-env.sh` must
   not be used for this: it copies v1's values.
8. **Handbook:** the private `fanta-handbook` repository (layout under
   "Interfaces and data"), with `gitleaks` in a pre-commit hook and in a push
   workflow (R22), and `credentials.yaml` with a daily workflow that posts
   to `#ops-action` 14 days before any expiry (R26). The register lives in
   one place, the handbook, where you edit it in Obsidian.

### Phase 1: alerts, the digest and the CRM (R1–R3, R5, R12, R13, R27)

- **Alerts.** `ops.alert(conn, kind, **facts)` enqueues an `ops.slack` job
  in the caller's transaction. The handler looks the kind up in `CATALOG`
  (channel, message template, allowed fields, runbook slug), drops every
  fact that is not allowed (R29), and posts to that channel's webhook. A
  channel whose `SLACK_WEBHOOK_*` variable is unset is skipped, which keeps
  tests and dev quiet. Each message links to
  `fanta-handbook/runbooks/<slug>.md` (R5). The hooks, each one line:

  | Kind | Channel | Hook |
  | --- | --- | --- |
  | `signup` | growth | `core/accounts.py` `ensure_user_with_personal_org`, new users only |
  | `purchase`, `first_purchase` | revenue | `compat/billing_routes.py`, `order.paid` |
  | `subscription_started`, `subscription_cancelled`, `refund` | revenue | the same handler's subscription and `order.refunded` branches |
  | `payment_webhook_failed` | action | `_polar_signature_valid` false, or the handler raising, in a transaction of its own |
  | `credit_grant` | action | `compat/admin_routes.py` `/internal/admin/credits/grant` |
  | `job_failed` | action | a sweeper over newly `failed` outbox rows, with its cursor in `system_settings` |
  | `budget_threshold` | action | the budget route below |

  v1 gets only `payment_webhook_failed` (Q7): a direct post from its Polar
  route's failure branch, a few lines. It is a fallback: a failed post is
  logged and never changes the route's response.
- **Heartbeats.** The tick and the outbox drain ping their Better Stack URL
  after each run when `HEARTBEAT_TICK_URL` and `HEARTBEAT_OUTBOX_URL` are
  set.
- **Budget feed.** The existing topic `fanta-budget-alerts` gets a push
  subscription to `POST /v2/internal/ops/budget`, authenticated with an
  OIDC token from a dedicated service account. The route checks the token's
  audience and email, keeps the latest `costAmount`, `budgetAmount` and
  `currencyCode` in `system_settings['gcp_budget']`, and raises
  `budget_threshold` when `alertThresholdExceeded` passes a threshold it
  has not reported yet.
- **Digest.** The `ops_digest` sweeper runs once Madrid time passes 08:00 if
  `system_settings['ops_digest'].last_day` is not today. It builds its
  figures from `adm.platform_overview` and `adm.revenue_analytics`, the
  functions the console's overview calls, so the two agree (R3). It adds 24
  hour counts, `gcp_budget`, the failed outbox jobs, the open-issue count
  from Linear's API (R19), and usage against the limits stored in
  `system_settings['ops_limits']` (R27). Sentry and Better Stack counts join
  once their read tokens exist.
- **CRM.** The sync already meets R12 once the key is set; a test proves the
  five-minute path. R13 adds `mrr`, `subscription_started_at` and
  `subscription_cancelled_at` to `build_company_payload`, and their
  attributes in Attio.

### Phase 2: support and tickets (R14–R17, R19, R20)

- **One intake function.** Every channel calls
  `support.ingest(source, source_ref, email, name, subject, body)`. It looks
  the customer up by email and records a row in `v2.support_tickets`. A
  report from the same person with the same subject within 14 days becomes a
  comment on their open issue. Anything else becomes a new issue in
  Support's triage, labelled `source:<channel>`, with the customer's name,
  email and plan. Then it enqueues an acknowledgement and `support_ticket`
  in `#ops-support`. Duplicates from different people are marked in Linear
  with "Mark as duplicate", and closing notifies every ticket linked to the
  issue, so the founder clicks once and everyone hears back (R15, R16).
- **Email.** ImprovMX forwards support@ both to the founder's inbox and to
  `support@in.fantaisa.net`, a Resend inbound domain. Resend's
  `email.received` webhook reaches `POST /v2/webhooks/resend`, verified
  with its Svix signature. The founder replies from Gmail, sending as
  support@ (Q6, R17); acknowledgements and closing notes go out through the
  backend's Resend client from support@.
- **Discord.** A bot that can read the help forum and reply in it. The
  `support_discord` sweeper runs every five minutes, lists the guild's
  active threads, ingests new posts in the forum (the thread id is the
  `source_ref`) and replies in the thread.
- **In-app reports.** The dashboard gets a signed-in `/support` page that
  posts to `POST /v2/support/reports`. The editor's Help menu gains "Report
  a problem", which opens that page with the app version and OS filled in.
  One web form serves both apps, and the customer is always known.
- **GitHub.** Linear's issue sync from phase 0 brings issues in, and its
  comment sync answers them. If the free plan lacks issue sync, a GitHub
  webhook calls `ingest` instead.
- **Closing.** Linear's webhook reaches `POST /v2/webhooks/linear`, verified
  with its HMAC signature. When an issue moves to a completed state, every
  linked ticket's reporter is notified in their channel, and the tickets
  close.
- **Attio (R14).** Opening and closing a ticket each add a note to the
  person, with the Linear link.
- **Release posts (R20).** A new fanta-edit workflow,
  `.github/workflows/announce-release.yml`, runs on `release: published` and
  posts the release name and link to the announcements channel's Discord
  webhook (`DISCORD_ANNOUNCE_WEBHOOK`).

### Phase 3: analytics and errors (R7–R11)

- **Consent (Q4).** On first launch, through the first-open path in
  `crates/fanta/src/fanta.rs`, the editor asks once. The answer sets
  `telemetry.metrics` and `telemetry.diagnostics` in the user's settings.
  The defaults in `assets/settings/default.json` stay off.
- **Editor events (R9).** A small sender posts catalogue events to
  `/v2/telemetry/events` when metrics are on and the user is signed in; the
  route requires a user, so nothing is sent before sign-in. Five calls in
  `fig_viewer` send `editor_opened`, `editor_project_opened`,
  `editor_import_completed`, `editor_agent_turn` and `editor_export`. The
  route already drops properties that are not catalogued. The inherited Zed
  telemetry client stays off, and the binnacle's telemetry row records the
  conditions that now hold.
- **One person (R10, R11).** The backend already uses the user id as the
  PostHog distinct id. The dashboard loads `posthog-js` and calls
  `identify(user id)` after sign-in. Sign-up links on the landing site carry
  the visitor's anonymous id, which the dashboard passes to PostHog's
  `bootstrap`, so earlier visits join the person. The landing page captures
  `download_clicked`. The handbook's `analytics.md` lists the funnel steps
  and the retention definition.
- **Errors (R7).** `sentry-sdk` starts in v2 next to `observability.py` when
  `SENTRY_DSN` is set, and `@sentry/nextjs` runs in v1, the landing site, the
  dashboard and the console. `send_default_pii` stays off and events carry
  only the user id, so Sentry holds no personal data to delete (R30). A
  Sentry alert rule posts new issues to `#ops-action`, and Sentry's Linear
  integration turns an accepted one into an issue.
- **Crashes (R8).** Release builds set `FANTA_MINIDUMP_ENDPOINT` to the
  desktop project's minidump URL. `crates/fanta/src/reliability.rs`
  already uploads only when `diagnostics_enabled()`. The release workflows
  upload debug files with `sentry-cli debug-files upload`, using
  `SENTRY_AUTH_TOKEN` from the `release` environment that fanta-edit #174
  creates.

### Phase 4: the console and agents (R23–R25)

- **Routes** in `compat/admin_routes.py`:
  - `GET /internal/admin/ops/destinations`: for each outbox kind, the last
    `done_at`, and the pending and failed counts over 24 hours.
  - `POST /internal/admin/ops/jobs/{id}/retry`: sets a failed job back to
    pending.
  - `GET /internal/admin/ops/costs`: `gcp_budget`, AI cost from the ledger,
    and Modal and Vercel. Those two are read from their billing APIs if they
    have one, or else entered monthly in `system_settings['ops_costs_manual']`.
- **fanta-admin.** `NEXT_PUBLIC_API_BASE` on Vercel becomes
  `https://api-v2.fantaisa.net`, and an Operations page shows the two
  tables with the console's existing table components.
- **Agent access (R25).** The handbook's `operations/agent-access.md` lists,
  for each system, its MCP server or API, the credential, its scope and
  where it is stored. Agent tokens are read-only. Write access is limited to
  the backend's own keys: Linear's is restricted to the Support team, and
  Attio's to the objects the sync writes.

### Deletion, in every phase (R30)

`core/account.py` `purge_user_account` already queues the Attio delete. It
also queues `posthog.delete_person` (phase 3) and `support.redact` (phase
2). The second replaces the email and name in the person's Linear issues and
deletes their ticket rows. Sentry holds only ids.

## Interfaces and data

- **Outbox kinds:** `ops.slack`, `support.linear`, `support.notify`,
  `crm.note`, `posthog.delete_person`, `support.redact`.
- **Routes:**
  - `POST /v2/internal/ops/budget` (Pub/Sub push, OIDC);
  - `POST /v2/webhooks/resend` (Svix);
  - `POST /v2/webhooks/linear` (HMAC);
  - `POST /v2/support/reports` (signed-in user);
  - the three admin routes above.
- **Table** (additive migration):
  `v2.support_tickets (id, source, source_ref, user_id, email, name, subject,
  linear_issue_id, status, created_at, closed_at)`, with a unique index on
  `(source, source_ref)`.
- **Settings keys** (`system_settings`): `ops_digest`, `gcp_budget`,
  `ops_dead_letter_cursor`, `ops_limits`, `ops_costs_manual`.
- **Environment** (Secret Manager on Cloud Run):
  - `SLACK_WEBHOOK_ACTION`, `_REVENUE`, `_GROWTH`, `_SUPPORT` and `_DIGEST`;
  - `LINEAR_API_KEY`, `LINEAR_SUPPORT_TEAM_ID` and `LINEAR_WEBHOOK_SECRET`;
  - `RESEND_WEBHOOK_SECRET`;
  - `DISCORD_BOT_TOKEN` and `DISCORD_HELP_FORUM_ID`;
  - `SENTRY_DSN`;
  - `HEARTBEAT_TICK_URL` and `HEARTBEAT_OUTBOX_URL`;
  - `BUDGET_PUSH_AUDIENCE` and `BUDGET_PUSH_SERVICE_ACCOUNT`.

  Every variable is optional: without it, its feature is off.
- **Handbook:**

  ```text
  fanta-handbook/
    README.md            start here: the operations map
    operations/          map.md, credentials.yaml, limits.md, agent-access.md
    runbooks/            one page per alert kind
    support/             reply snippets
    vendors/             one page per vendor: account, plan, owner
    procedures/          setup, release day, month close, incidents
    analytics.md         funnel and retention definitions
    .obsidian/           app.json and core plugins; workspace files ignored
    .github/workflows/   secret-scan.yml, credentials-expiry.yml
  ```

## Alternatives

- **PostHog destinations, Zapier, Make, n8n, or calls straight from request
  handlers** for delivery: see ADR 0029.
- **A shared help desk** instead of the Linear and Gmail pair: no free plan
  fits ([ADR 0028](../../docs/decisions/0028-choose-the-tools-that-run-the-business.md)).
- **The credentials register in `system_settings`:** the backend could alert
  without a workflow, but you would maintain it in the console instead of the
  handbook. Two copies would drift, so it lives in the handbook only.
- **Anonymous editor telemetry before sign-in:** more funnel data, but it
  needs an install id and a public route. The funnel starts at sign-in
  instead.

## Risks

- **Linear's 250-issue cap is shared with the FAN team.** The digest shows
  the count, Support closes tickets promptly, and the cap is the upgrade
  trigger.
- **Slack webhook URLs let anyone holding them post.** They live in Secret
  Manager only.
- **Resend's 3,000 emails a month include received mail.** The digest
  tracks the count, and `ops_limits` warns at 80 percent.
- **Sentry's free plan might not post to Slack.** In that case its alert
  email goes to the founder and the digest counts new issues.
- **A bot that loses forum access misses posts.** The sweeper alerts on any
  Discord API error.
- **Public webhook routes are attack surface.** Each verifies a signature or
  an OIDC token and rejects everything else.
- **Backing out:** unsetting a variable switches its feature off. The
  migration is additive.

## Verification

- **Unit tests** in fanta-backend v2, named after the criteria they prove:
  - the catalog drops fields it does not list (A20);
  - a failing `ops.slack` handler leaves the purchase committed and the job
    retrying, then failed (A1);
  - the digest's figures equal `platform_overview` for the same window (A2);
  - the budget route rejects a bad token (A4);
  - ingest attaches the same reporter's repeat report (A10);
  - Resend and Linear signatures are verified (A10);
  - closing an issue notifies every linked ticket (A10);
  - purge queues the PostHog and redaction jobs (A21).
- **Editor:** a GPUI test that the consent answer sets both settings, and
  that no request leaves with metrics off (A7).
- **Founder's checks, recorded in the handbook:** A3, A4, A6, A8, A9, A11,
  A12, A14, A15, A17, A18 and A19, each run once on production with test
  accounts and the Polar sandbox, where they apply.
