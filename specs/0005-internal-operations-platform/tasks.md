# Tasks: Internal operations platform

One task per pull request, in order. Each task says what it changes, which
requirements it serves and how it is verified. Tick it and link the pull
request when it merges. T2 lands before any task that sends personal data
to a new tool.

## Phase 0: accounts, wiring and disclosure

- [ ] **T1.** New private repository `fanta-handbook`: the vault layout, the
  operations map, `credentials.yaml` seeded with every known expiry,
  `procedures/setup.md` (the phase 0 checklist), the secret-scan and expiry
  workflows (R21, R22, R26). Verify: A15, A18.
- [ ] **T2.** fantaisa-landing: the privacy page names every processor and
  what it receives (R28). Verify: the processor list in A20.
- [ ] **T3.** No pull request: the founder works through T1's checklist.
  That covers Slack and the Fanta Ops app, the vendor apps, Linear's
  Support team and GitHub connection, Better Stack, Sentry's projects, the
  Polar backstop and the Attio key (R4, R6, R18). Verify: A3's vendor rows,
  A4's probes, A12.

## Phase 1: alerts, the digest and the CRM

- [ ] **T4.** fanta-backend: `ops.py` with the catalog and the Slack
  handler; the hooks, the failed-job sweeper and the heartbeats; v1's
  webhook-failure post (R1, R2, R5, R29). Verify: A1, A3, A4's heartbeats.
- [ ] **T5.** fanta-backend: the budget push route and the morning digest
  (R3, R19, R27). Verify: A2, A13, A19's limits.
- [ ] **T6.** fanta-backend: Attio's revenue fields and a test of the
  five-minute path (R12, R13). Verify: the first half of A9.

## Phase 2: support and tickets

- [ ] **T7.** fanta-backend: `support.py` and `v2.support_tickets`; email
  intake through Resend; Linear issues and comments; the Linear webhook and
  closing notes; Attio notes; redaction on purge (R14–R17, R30). Verify:
  A10 for email, A11, and A21 for Linear.
- [ ] **T8.** fanta-backend, fanta-dashboard and fanta-edit: the `/support`
  form and its route, and the editor's "Report a problem" (R15, R16).
  Verify: A10 for in-app reports.
- [ ] **T9.** fanta-backend: the Discord help forum's intake and replies
  (R15, R16, R20). Verify: A10 for Discord.
- [ ] **T10.** fanta-edit: `announce-release.yml` (R20). Verify: A14.

After T9, the daily operations routine (Q9) is set up as a scheduled agent
run that drafts only. It is not a pull request.

## Phase 3: analytics and errors

- [ ] **T11.** fanta-backend (v1 and v2), fantaisa-landing, fanta-dashboard
  and fanta-admin: the Sentry SDKs, the alert rule and the Linear
  integration (R7). Verify: A5.
- [ ] **T12.** fanta-edit: the first-launch consent, the event sender and
  its five calls, and the binnacle's telemetry row (R9). Verify: A7.
- [ ] **T13.** fanta-edit release workflows: the minidump endpoint and
  debug-file uploads (R8). Verify: A6.
- [ ] **T14.** fantaisa-landing, fanta-dashboard and fanta-backend: one
  PostHog person, `download_clicked`, the funnel definitions in the
  handbook, and PostHog deletion on purge (R10, R11, R30). Verify: A8, and
  A21 for PostHog.

## Phase 4: the console and agents

- [ ] **T15.** fanta-backend and fanta-admin: the ops routes, the Operations
  page and the console on v2 (R23, R24). Verify: A16.
- [ ] **T16.** fanta-handbook: the agent access page and read-only tokens
  (R25). Verify: A17.
