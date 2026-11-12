---
status: draft
date: 2026-11-04
owner: Jean Rojas
decisions: [0028, 0029]
repositories: [fanta-backend, fanta-admin, fanta-edit, fantaisa-landing, fanta-dashboard, fanta-handbook]
---

# Internal operations platform

## Problem

The product is live, but the business around it runs on the founder's memory
and inbox. Paid v2 credits went on sale on 2026-11-02, the day the YC W27
application went in, so the first paying customers may arrive before anything
tells the founder they did. One person runs the company with agents, so
whatever we build has to work for one person and leave the routine work to
agents.

What happens today:

- **Failures surface weeks late, and by accident.** v1's Polar webhook
  answered all 30 of its deliveries on 2026-10-13 with 403, so no v1 purchase
  could have been credited. It was found on 2026-11-02, and the fix
  (fanta-backend #51) is still open. On v2, every Agent Panel request from
  anyone but the founder got a 503 until the founder hit it in a live run on
  2026-11-02 (fanta-backend #50). macOS release builds failed once the
  `cargo-bundle` fork they cloned disappeared, which was found at release
  time (#172). The October GCP bill, about €44 for an always-on Cloud SQL
  instance, was found on the invoice. The budget created on 2026-11-02 sends
  email at 50 and 100 percent, and nothing reads its Pub/Sub feed yet.
- **Nobody can see the product being used.** The editor ships with `metrics`
  and `diagnostics` off ([binnacle](../../docs/fanta/disabled-services-binnacle.md)),
  and nothing sends the `editor_*` events that the backend's PostHog catalog
  declares (fanta-backend `v2/src/fanta_v2/events.py`). Server events cover
  what the backend sees: AI requests, agent runs and billing. The landing site
  loads PostHog, Vercel Analytics and optional Google Analytics but never
  identifies anyone, and the dashboard loads no analytics, so a landing visit
  never joins the account it turns into. Nothing tracks errors in the backend
  or the web apps, and no build sets an endpoint for the app's inherited crash
  uploader (`FANTA_MINIDUMP_ENDPOINT`).
- **Customer conversations have no home.** support@fantaisa.net is an
  ImprovMX forward with no queue behind it. The Fanta Community Discord linked
  from the README is not connected to accounts or tickets. fanta-edit has
  GitHub issue templates, but nobody has ever filed an issue. Nothing turns a
  report into a tracked ticket, links it to the customer or tells the
  customer it was fixed.
- **The CRM is wired but probably empty.** v1 and v2 sync people and
  workspaces to Attio through `crm_outbox`, with lifecycle stage, plan,
  balance, 30-day spend, scores, deletes and a nightly resync
  (`core/crm.py`, `intelligence.py`), and the founder console shows the sync
  status. But `v2/deploy/sync-vercel-env.sh` copies `POSTHOG_*` and
  `RESEND_API_KEY` to v2 and not `ATTIO_API_KEY`, so unless someone set it by
  hand, v2 syncs nothing.
- **The founder console still points at v1.** fanta-admin
  (admin.fantaisa.net) calls `NEXT_PUBLIC_API_BASE`, which defaults to
  `api.fantaisa.net`. v2 serves all 26 `/internal/admin/*` routes the console
  uses. Its last feature landed on 2026-07-18, and it shows no costs,
  integration health or support queue.
- **Operating knowledge lives in agent session notes on one Mac and in
  chat.** Runbooks, account details, vendor limits and credential expiry dates
  have no shared home, and credentials expire unnoticed: v1's Polar token
  expires on 2026-11-12.

## Goals

- The founder hears in Slack, within five minutes, about every event that
  needs them: a signup, a payment, a refund, a cancellation, a failed
  webhook, an error spike, an outage, a budget threshold, an expiring
  credential.
- A customer's whole story is in one Attio record: who they are, their plan,
  spend, usage, scores, conversations and tickets.
- Every bug report, support request and piece of feedback becomes one Linear
  issue linked to the customer, whatever channel it came through, and the
  customer hears back through that channel.
- PostHog shows the whole journey, from the landing page through the editor
  to payment, for the users who consent.
- One digest each morning says how the business is doing.
- The company's operating knowledge lives in one handbook that the founder
  and the agents can both read and edit.
- Agents run the routine operations with scoped access. Anything that
  reaches a customer or the public waits for the founder's yes.
- Tool fees stay at $0 a month until there is revenue, and each tool has a
  written trigger for its first upgrade.

## Non-goals

- Building our own CRM, help desk, chat, analytics or wiki. We connect hosted
  tools, and the code we write is glue.
- Live chat or a support widget on the website.
- Marketing automation beyond the lifecycle emails the backend already sends:
  newsletters, campaigns, ad attribution.
- Accounting, tax, payroll, legal and hiring. They get their own spec when
  needed; the handbook can hold their notes meanwhile.
- Changing billing (Polar), sign-in (Clerk) or hosting (GCP, Vercel, Modal).
- Moving repositories ([spec 0003](../0003-github-organization/spec.md)). New
  repositories follow its layout.
- Syncing edits made in Attio or Linear back into the product database.

## Operating map

Each kind of data has one owner. [ADR 0028](../../docs/decisions/0028-choose-the-tools-that-run-the-business.md)
records the choice of tools and their free-plan limits.

| Data | Owner | Written by |
| --- | --- | --- |
| Accounts, workspaces, credits, usage, the ledger | Backend Postgres (v2) | The product |
| Payments, subscriptions, invoices, tax | Polar | Checkout |
| Identity and sign-in | Clerk | Sign-up |
| People, companies, deals, notes | Attio | The backend (facts); the founder (notes, owners, deals) |
| Product behaviour, funnels, replays | PostHog EU | Landing, dashboard, editor (opt-in), backend |
| Errors and crashes | Sentry | Backend, web apps, editor (opt-in) |
| Uptime, heartbeats, status page | Better Stack | Probes, the backend's scheduled jobs |
| Bugs, support tickets, roadmap | Linear: one workspace, the FAN team for engineering and a Support team for customers | Every intake channel, the founder |
| Internal conversation and alerts | Slack | People, the backend, vendor apps |
| Community, help forum, announcements | Discord | Users, release automation |
| Transactional and support email | Resend; ImprovMX forwarding | The backend; the founder |
| Runbooks, procedures, vendor accounts, business notes | Handbook: an Obsidian vault in a private Git repository | The founder, agents |

Engineering specs and decision records stay in the product repositories.

## Requirements

### Alerts and the digest

- **R1.** The backend must post each event in the alert catalog to Slack
  within five minutes. Events that need action go to one channel and
  everything else to others, so the action channel stays quiet. The first
  catalog: signup, first purchase, purchase, subscription started, cancelled
  or refunded, a payment webhook that fails verification or handling, a
  credit grant by an admin, a dead-lettered outbox job, a budget threshold
  and an expiring credential.
- **R2.** A notification must never block or fail the request that caused
  it. It is recorded in the same transaction as the change and delivered
  with retries, so a Slack outage delays alerts but never fails a signup or a
  purchase.
- **R3.** Each morning at 08:00 Madrid time (Q1) the founder must get a
  digest of the last 24 hours and the month to date: signups, activations, purchases
  and refunds, credits spent, AI cost against revenue, cloud spend against
  budget, new error groups, open tickets by age, uptime, and any tool near a
  free-plan limit. Its figures must match the founder console for the same
  window.
- **R4.** Events that start in a vendor reach Slack through that vendor's own
  integration where one exists (GitHub, Vercel, Linear, Polar, Better Stack,
  Sentry), without code of ours.
- **R5.** Every alert must link to a runbook in the handbook that says what
  to check and how to fix it.

### Uptime and errors

- **R6.** External probes must check both APIs (`api.fantaisa.net`,
  `api-v2.fantaisa.net`), the websites (fantaisa.net and its app, admin and
  docs subdomains) and the release feed, and alert Slack within five minutes
  of a failure. Heartbeats must alert when the minute tick or the outbox
  drain stops running.
- **R7.** Errors in the backend (v1 and v2) and the web apps must be grouped
  by release in Sentry. A new group must alert Slack, and a group the founder
  marks as a bug must become one Linear issue.
- **R8.** The desktop app must upload crash reports only after the user opts
  in, with symbols uploaded for each signed release so a crash shows a
  readable stack for that version. The binnacle's conditions for diagnostics
  (ingestion, symbolication, retention, alerting and disclosure) must hold
  before it ships.

### Product analytics

- **R9.** The editor must send the `editor_*` events in the backend's catalog
  only after the user opts in, and never content: no prompts, no design data,
  no file or project names. Off stays the default, and turning metrics on
  must meet the binnacle's conditions.
- **R10.** A person must be one PostHog person across the landing site, the
  dashboard, the backend and the editor, keyed by the backend user id and not
  by email address. Anonymous landing visits join that person at sign-up.
- **R11.** PostHog must hold the funnel from landing visit to download,
  sign-in, first project, first agent turn, first export and first purchase,
  and weekly retention. The handbook keeps the definitions needed
  to rebuild these dashboards.

### CRM

- **R12.** Every user and workspace must exist in Attio within five minutes
  of sign-up, with stage, plan, balance, 30-day spend, last activity and
  scores. The backend writes only the fields it owns and never overwrites the
  notes, owners or deal stages people set in Attio.
- **R13.** Every paying workspace must show its plan, monthly revenue, start
  date and cancellation date in Attio.
- **R14.** A customer's tickets must be visible from their Attio record.

### Support and tickets

- **R15.** Reports sent to support@fantaisa.net, posted in the Discord help
  forum, filed from the in-app report in the editor or the dashboard, or
  opened as GitHub issues on public repositories must each become one Linear
  issue in triage within five minutes, labelled with its source and linked
  to the customer when known. A second report of the same problem attaches
  to the open issue instead of creating another.
- **R16.** The customer must get an acknowledgement, and must hear back in
  the same channel when their issue closes: by email for email and in-app
  reports, as a thread reply on Discord, as a comment on GitHub.
- **R17.** Support email must be answered from support@fantaisa.net, not
  from a personal address.
- **R18.** Pull requests must link to their Linear issue and close it on
  merge.
- **R19.** Open issues must stay under the Linear free plan's cap, which the
  Support team shares with the engineering program's FAN team, and the
  digest must report the count.

### Community

- **R20.** Discord stays the community: a help forum that feeds R15,
  announcements and rules. Publishing a release must post an announcement
  automatically. Staff alerts never go to Discord.

### Handbook

- **R21.** The handbook must be a private Git repository of Markdown that
  opens as an Obsidian vault. It holds the operations map, runbooks, support
  replies, vendor accounts and limits, and procedures. The founder's agents
  must be able to read and edit it, and every change must be in its history.
- **R22.** The handbook must never hold a secret. It says where each
  credential lives (Secret Manager, Vercel, a password manager), and a secret
  scan must reject a push that contains one.

### Founder console

- **R23.** The console must work against v2 and show integration health: the
  last successful delivery to each destination (Slack, Attio, Linear,
  PostHog, Discord), dead-lettered jobs with a retry button, and each
  destination's failures over the last 24 hours.
- **R24.** The console must show this month's spend per vendor (GCP from the
  budget feed, AI providers from the ledger, Modal, Vercel) next to revenue.

### Agents, access and cost

- **R25.** Every system must be reachable by the founder's agents through an
  official MCP server or API. Agent credentials are scoped and read-only by
  default. Anything that messages a customer, posts in public or moves money
  waits for the founder to approve that action.
- **R26.** Every credential must have an owner, a storage location and an
  expiry date recorded in the handbook, and an alert must fire 14 days before
  it expires.
- **R27.** Tool fees must stay at $0 a month until there is revenue. The
  operations map records each tool's free-plan limits, current usage and the
  trigger for its first upgrade, and the digest warns at 80 percent of any
  limit.

### Privacy

- **R28.** The privacy policy must name each new processor and what it
  receives before any data flows to it, and data must stay in the EU where
  the vendor offers it.
- **R29.** Slack messages and Linear issues may carry a customer's name and
  email address and nothing more sensitive: no payment details, prompts or
  design content.
- **R30.** Deleting an account must remove or anonymize the person in Attio,
  PostHog, Linear and Sentry within 30 days.

## Acceptance criteria

- **A1 (R1, R2).** In a test environment, a sandbox purchase posts to the
  revenue channel within five minutes. With the Slack destination pointed at
  a dead endpoint, the purchase still succeeds, the delivery retries, and
  after the last attempt the job is dead-lettered and shows in the console.
- **A2 (R3).** The digest arrives at the set time on three mornings in a
  row, and its signups, purchases and credits match the console's overview
  for the same window.
- **A3 (R4, R5).** The operations map lists every alert with its channel, its
  source (our code or a vendor app) and a runbook link that opens.
- **A4 (R6).** A probe pointed at a failing URL, and a stopped tick
  heartbeat, each alert Slack within five minutes.
- **A5 (R7).** A test exception thrown in v1, v2 and each web app appears in
  Sentry grouped by release, alerts Slack once, and becomes one Linear issue
  when marked as a bug.
- **A6 (R8).** With crash reports off, a forced crash
  (`FANTA_GENERATE_MINIDUMPS`) uploads nothing. After opting in, it appears in
  Sentry with a symbolicated stack for that release.
- **A7 (R9).** With metrics off, a session that opens a project and runs the
  agent sends nothing to the telemetry endpoint. With metrics on, it sends
  `editor_opened`, `editor_project_opened` and `editor_agent_turn`, and no
  property holds a file name, a prompt or design data.
- **A8 (R10, R11).** A new test user who visits the landing page, downloads,
  signs in, opens a project, runs an agent turn and buys credits is one
  PostHog person with every funnel step in order.
- **A9 (R12–R14).** A new sign-up appears in Attio within five minutes with
  stage, plan and balance. A note and an owner set by hand survive the next
  nightly resync. A paying test workspace shows its plan and revenue, and its
  open ticket shows on the record.
- **A10 (R15, R16).** An email to support@, a Discord help post, an in-app
  report and a GitHub issue each create one Linear triage issue with the
  right source and customer. A second report of the same problem attaches to
  that issue. Closing it notifies each reporter in their own channel.
- **A11 (R17).** A reply to a support email reaches the customer from
  support@fantaisa.net and passes SPF and DKIM.
- **A12 (R18).** Merging a pull request that names a Linear issue closes the
  issue.
- **A13 (R19).** The digest shows the open-issue count against the cap.
- **A14 (R20).** Publishing a GitHub release posts in the Discord
  announcements channel.
- **A15 (R21, R22).** The handbook clones and opens as an Obsidian vault, an
  agent session can edit a page and push it, and a push that contains a test
  secret is rejected.
- **A16 (R23, R24).** The console, pointed at v2, shows a last delivery time
  for each destination, retries a dead-lettered job, and shows month-to-date
  spend per vendor next to revenue.
- **A17 (R25).** The operations map lists each agent credential with its
  scope, and a read-only agent token can neither create a Linear issue nor
  send an email.
- **A18 (R26).** A credential recorded as expiring within 14 days appears in
  Slack the same day.
- **A19 (R27).** The operations map shows each tool's limits, usage and
  upgrade trigger, and the month's invoices for internal tools total $0.
- **A20 (R28, R29).** The privacy page names every processor in the
  operations map, and a week's sample of Slack alerts and Linear issues holds
  no customer field beyond name and email.
- **A21 (R30).** Deleting a test account removes or anonymizes its person in
  Attio, PostHog, Linear and Sentry within 30 days.

## Phases

The phases set the order. Once this spec is accepted, `design.md` and
`tasks.md` break each phase into pull requests. R28–R30 bind every phase: a
phase that sends personal data to a tool first names the tool in the privacy
policy, keeps to R29 in what it sends, and deletes the person there when the
account is deleted.

0. **Accounts, wiring and disclosure.** Mostly the founder's clicks, with a
   checklist, starting with the privacy page: the Slack workspace and
   channels, Linear connected to GitHub, the Attio key on v2, Better Stack
   probes, Sentry projects, the vendors' Slack apps, and the handbook
   repository with the operations map and the credentials register. Covers
   R4, R6, R18, R21, R22 and R26, and starts R27.
1. **Alerts, the digest and the CRM** (fanta-backend): the alert catalog
   through the outbox, the budget feed, the morning digest with its
   free-plan warnings, and the Attio gaps. Covers R1–R3, R5, R12, R13 and
   R27.
2. **Support and tickets** (fanta-backend, fanta-edit, fanta-dashboard):
   intake from each channel into Linear, acknowledgements and replies, Attio
   links, sending as support@ and Discord release posts. Covers R14–R17, R19
   and R20.
3. **Analytics and errors** (fanta-edit, fantaisa-landing, fanta-dashboard,
   fanta-backend): consent, editor events, one identity, the funnel, error
   tracking and crash reports. Covers R7–R11.
4. **Console and agents** (fanta-admin, fanta-backend): the console on v2
   with integration health and costs, and scoped agent access. Covers
   R23–R25.

## Open questions

On 2026-11-07 the founder said "go forth" and declined to answer one by one,
so each recommendation below is the default. Change any of them in review.

- **Q1.** The digest arrives at 08:00 Madrid time, every day.
- **Q2.** Probes start now; the public status page opens at the first paying
  customer.
- **Q3.** Sentry tracks errors and crashes. The app's inherited uploader
  already posts native minidumps, which Sentry symbolicates.
- **Q4.** The editor asks once, at first launch, whether to share analytics
  and crash reports. Both stay off unless accepted and can be changed later
  in settings.
- **Q5.** One Linear workspace, shared with the AI-native engineering
  program, which recreates it: its FAN team holds engineering work and
  roadmap, and a Support team holds customer tickets. A ticket that turns out
  to be a bug moves to FAN at triage.
- **Q6.** Support email is answered in Gmail, sending as support@fantaisa.net
  through Resend's SMTP with a sending-only key. ImprovMX keeps forwarding
  incoming mail.
- **Q7.** v1 gets payment-webhook failure alerts and a probe; everything else
  runs on v2.
- **Q8.** Discord roles tied to Fanta accounts wait until people pay.
- **Q9.** After phase 2, an agent runs a daily routine: it triages new Linear
  issues, drafts support replies for the founder's approval and writes a
  weekly metrics note in the handbook. It drafts only, never sends or posts.
- **Q10.** The handbook is a private `fanta-handbook` repository under
  `jeanc18rlos`, moving with the others if spec 0003's move happens.

## Changes after acceptance

- None.
