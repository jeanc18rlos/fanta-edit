---
status: accepted
date: 2026-09-30
recorded: 2026-10-27
decision-makers: Jean Rojas
supersedes: [0010]
tags: [distribution, billing]
---

# Distribute a notarized DMG with web billing

## Context

The App Store build ([0010](0010-ship-through-the-mac-app-store.md)) was in
expedited review on 2026-09-30, available in the United States only, and its
sandbox blocked MCP servers and local agents. The founder decided to ship to
everyone, worldwide, and to stop waiting on review.

## Decision

Distribute Fanta as a Developer ID-signed, notarized and stapled DMG, built
and signed only in CI by the "Build macOS release" workflow
(`.github/workflows/release-macos.yml`). Tag releases
`alpha-<version>-<YYYYMMDD>`, because the repository inherits Zed's `v1.0.0`
tags. Sell credits through Polar web checkout instead of RevenueCat (#125);
RevenueCat was removed in #139. Keep the App Store code paths behind
`mac_app_store`. Shipaton was dropped.

## Options considered

1. **Direct DMG with web billing** (chosen).
2. **Wait for the App Store.** US-only, sandboxed, and on Apple's schedule.

## Consequences

- Good: Fanta installs in every country, agents and MCP run unsandboxed, and
  the GPL source is published next to the binary.
- Bad: Fanta owns updates ([0017](0017-update-the-app-in-place-from-signed-releases.md)),
  signing secrets live in CI, and the download page on fantaisa.net hard-codes
  each release's asset URLs, so every release must update it.

## Confirmation

The `Protect release tags` ruleset guards `alpha-*` tags. `script/verify-macos-dmg`
and the release workflow check the signed artifact.

## References

- Founder decision recorded in session notes on 2026-09-30
- #125 (2026-10-15), #139 (2026-10-19), [PUBLIC_ALPHA.md](../alpha/PUBLIC_ALPHA.md)
