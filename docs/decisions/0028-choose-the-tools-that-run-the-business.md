---
status: proposed
date: 2026-11-04
decision-makers: Jean Rojas
tags: [operations, vendors]
---

# Choose the tools that run the business

## Context

[Spec 0009](../../specs/0009-internal-operations-platform/spec.md) asks for
alerts, a CRM, tickets, product analytics, error tracking, uptime checks, a
community and a handbook. Some of this exists. The backend syncs Attio, sends
PostHog events to the EU cloud and sends mail through Resend; Polar takes
payments; Clerk signs people in; ImprovMX forwards support@fantaisa.net; and a
community Discord server exists. The rest has no tool at all.

There is no revenue yet, so every tool has to start on a free plan. One
person runs the company and agents do much of the work, so every tool also
needs an API or an MCP server.

On 2026-11-04 the founder chose Slack for the team and alerts with Discord
for the community, Linear as the tracker, free plans until there is revenue,
and an Obsidian vault in Git as the wiki.

## Decision

Proposed: run the business on these tools, each on its free plan and each
the owner of one kind of data.

| Need | Tool | Free plan, as listed on 2026-11-04 | First upgrade when |
| --- | --- | --- | --- |
| Team chat and alerts | Slack | 90 days of visible history, 10 apps | A second person joins, or 10 apps are not enough |
| Community, help forum, announcements | Discord, the existing server | Free | — |
| Bugs, support tickets, roadmap | Linear, one workspace shared with the AI-native engineering program (its FAN team plus a Support team) | 250 issues across both teams, 2 teams, unlimited members; API, webhooks, MCP and triage included; Slack and email intake need Business | Open issues pass 200, or a third team is needed |
| CRM | Attio (kept) | 3 seats, 50,000 records, API on every plan | A fourth seat |
| Product analytics | PostHog EU (kept) | 1M events and 5,000 recordings a month | 80% of an allowance |
| Errors and crashes | Sentry | 1 user, 5,000 errors a month | A second engineer, or errors dropped at the cap |
| Uptime, heartbeats, status page | Better Stack | 10 monitors, 10 heartbeats, 1 status page, Slack and email alerts | An eleventh monitor |
| Handbook and wiki | An Obsidian vault in a private Git repository | Obsidian is free for work since 2025-02-20; Git sync replaces the paid Sync | A colleague who needs a web editor |
| Email | Resend (kept) and ImprovMX forwarding (kept) | Resend: 3,000 emails a month and 100 a day, received mail included; SMTP and inbound webhooks | 80% of either limit |

The limits come from the vendors' pricing pages and from secondary sources
where those pages say nothing. Check the vendor's page before relying on
one.

These rules hold for every tool:

- The backend is the source of product facts and pushes them outward. People
  own relationship data in Attio and work in Linear. Nothing syncs back into
  the product database.
- Where a vendor integration connects two tools (GitHub and Linear, Vercel
  and Slack, Sentry and Linear), we use it. Our code covers only what no
  integration does.
- Staff alerts go to Slack and never to Discord.
- No tool gets a paid plan until its trigger fires and the founder approves
  the spend.

## Options considered

1. **Hosted free plans, one owner per kind of data** (proposed). The least
   code and the least to operate. Every tool has an API, most have an MCP
   server, and the cost is a set of limits to watch.
2. **Discord for everything.** One app, and free, but fewer vendor
   integrations post to Discord, and there is no Slack Connect for future
   enterprise customers. The founder chose Slack plus Discord on 2026-11-04.
3. **GitHub Issues and Projects as the tracker.** Free and next to the code,
   but it has no triage inbox and no link to the customer, and private
   support tickets do not belong in public repositories. The founder chose
   Linear on 2026-11-04.
4. **Self-hosted open-source tools**: Twenty for the CRM, Plane for tickets,
   Mattermost for chat, Docmost or Outline for the wiki, GlitchTip for
   errors. No tool fees, but one person would host, upgrade, back up and
   secure each of them, and most need a database of their own while we are
   trying to shrink the Cloud SQL bill.
5. **Paid suites** (HubSpot, Intercom, Notion). Quicker to start, but they
   cost money before there is revenue. HubSpot's free CRM came closest, and
   Attio is already integrated.
6. **PostHog for errors as well.** One vendor fewer and a larger free
   allowance (100,000 exceptions a month), but the app's crash path uploads
   native minidumps, which Sentry symbolicates. Spec 0009's Q3 took Sentry
   as the default on 2026-11-07.

## Consequences

- Good: most connections are configuration, not code. Agents can operate
  every tool, and the bill stays at $0 while there is no revenue.
- Bad: there are nine vendors, each with an account, limits and credentials
  to watch. Slack hides messages older than 90 days, so the source of truth
  stays in Postgres and PostHog. Linear's 250-issue cap is the tightest
  limit, and its pricing page does not say whether archived issues count, so
  expect to need Basic (about $10 per user a month, billed yearly) by the
  first month with revenue. Linear's Slack and email intake need its
  Business plan, so intake goes through the backend and the Linear API. Each
  new processor has to appear in the privacy policy before data reaches it.
- Follow-up: the phases of spec 0009, and
  [ADR 0029](0029-deliver-business-events-through-the-backend-outbox.md) on
  how the backend delivers events to these tools.

## Confirmation

The handbook's operations map lists each tool with the data it owns, where
its credentials live, its limits, current usage and upgrade trigger. The
morning digest flags any tool past 80 percent of a limit. Adding a tool or
moving one to a paid plan needs a record that supersedes this one.

## References

- [Spec 0009: Internal operations platform](../../specs/0009-internal-operations-platform/spec.md)
- fanta-backend `v2/src/fanta_v2/events.py` (PostHog catalog),
  `v2/src/fanta_v2/core/crm.py` (Attio sync), `v2/src/fanta_v2/outbox.py`
- fanta-admin `README.md` (the founder console)
- [Disabled services binnacle](../fanta/disabled-services-binnacle.md)
  (telemetry and crash reporting conditions)
- Pricing pages: [Linear](https://linear.app/pricing),
  [Slack free plan](https://slack.com/help/articles/115002422943-Slack-Free-plan),
  [Attio](https://attio.com/pricing), [PostHog](https://posthog.com/pricing),
  [Better Stack](https://betterstack.com/pricing),
  [Resend limits](https://resend.com/docs/knowledge-base/account-quotas-and-limits),
  [Obsidian licence](https://obsidian.md/license)
