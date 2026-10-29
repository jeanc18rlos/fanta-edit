---
status: superseded
date: 2026-09-24
recorded: 2026-10-27
decision-makers: Jean Rojas
superseded-by: 0012
tags: [distribution, billing]
---

# Ship through the Mac App Store

## Context

Fanta needed a first public release by the end of September, with the
Shipaton deadline on 2026-09-30. The Mac App Store offered distribution,
payments and trust.

## Decision

Build a sandboxed App Store variant behind the `mac_app_store` feature, sell
subscriptions through RevenueCat with Apple offer codes, guard dynamic code
execution in the sandbox, and submit the build for review.

## Consequences

- The sandbox blocked MCP servers and local agents, which are central to the
  product.
- The App Store record was available only in the United States, and review
  timing was outside Fanta's control.
- The `mac_app_store` code paths remain, tested by the
  `mac-app-store-runtime` job in `check.yml`.
- Superseded on 2026-09-30 ([0012](0012-distribute-a-notarized-dmg-with-web-billing.md)).

## Confirmation

None; see 0012.

## References

- Commit `404f7b74ea` "Prepare Fanta for Mac App Store billing and sandbox release" (2026-09-24), #10, #11, #23
- [MAC_APP_STORE.md](../alpha/MAC_APP_STORE.md), kept as history
