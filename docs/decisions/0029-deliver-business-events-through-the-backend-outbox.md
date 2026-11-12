---
status: proposed
date: 2026-11-07
decision-makers: Jean Rojas
tags: [operations, backend]
---

# Deliver business events through the backend outbox

## Context

[Spec 0011](../../specs/0011-internal-operations-platform/spec.md) sends
business facts to Slack, Linear, Attio, Discord and email: signups,
payments, refunds, failed webhooks, tickets and replies. The facts live in
v2's Postgres. A notification must never block or fail the request that
caused it (R2), and Slack and Linear messages may carry only a name and an
email address (R29).

v2 already has a transactional outbox (fanta-backend
`v2/src/fanta_v2/outbox.py`). A job is a row written in the same transaction
as the change, delivered by a handler registered with `@handler(kind)` and
drained every minute by a Temporal schedule. It retries six times with
backoff and is then marked `failed`. GitHub and CRM work already uses it.

## Decision

Proposed: every business event becomes an outbox job written in the
transaction that changes the fact, and one handler per destination delivers
it. A single catalog in the backend says which event goes where and which
fields it may carry. Events between two vendors (GitHub and Linear, Vercel
and Slack, Sentry and Slack) use the vendors' own integrations and never
pass through our code.

## Options considered

1. **The backend outbox** (proposed). Retries, dead letters and the
   transactional guarantee already exist. Routing is code that is reviewed
   and tested like any other, and personal data never leaves the backend
   except through the catalog's field list.
2. **PostHog's destinations.** Events already reach PostHog, but only as
   analytics after the fact, and the catalog forbids emails there, so alerts
   and tickets would lack the customer. Routing would live in PostHog's
   interface, unreviewed, and destinations are metered.
3. **Zapier or Make.** No code, but small free plans, our credentials in a
   third party, and routing that nobody reviews.
4. **Self-hosted n8n.** Free, but another service with a database for one
   person to run.
5. **Direct calls from request handlers.** The least code, but a slow or
   failing Slack or Linear call would slow or fail the signup or purchase
   that triggered it, which breaks R2.

## Consequences

- Good: R2 holds by construction, failed deliveries are visible in one
  table, and the founder console can show integration health by reading it.
- Bad: each destination's client is ours to keep working: a Slack incoming
  webhook, Linear's GraphQL API, Discord's REST API and Resend. Each is a
  few dozen lines. Delivery is at least once, so handlers must be
  idempotent.
- Follow-up: spec 0011's design and tasks.

## Confirmation

Tests show that a failing delivery leaves the business transaction
committed and the job retrying, and that the catalog drops fields it does
not list. Review rejects a business notification sent directly from a
request handler.

## References

- [Spec 0011](../../specs/0011-internal-operations-platform/spec.md) and
  its [design](../../specs/0011-internal-operations-platform/design.md)
- [ADR 0028](0028-choose-the-tools-that-run-the-business.md)
- fanta-backend `v2/src/fanta_v2/outbox.py`, `v2/src/fanta_v2/core/crm.py`
