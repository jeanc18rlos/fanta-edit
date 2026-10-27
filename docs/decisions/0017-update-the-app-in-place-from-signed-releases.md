---
status: accepted
date: 2026-10-19
recorded: 2026-10-27
decision-makers: Jean Rojas
tags: [distribution, security]
---

# Update the app in place from signed releases

## Context

Direct distribution ([0012](0012-distribute-a-notarized-dmg-with-web-billing.md))
left updates to Fanta. The 1.0 alpha shipped with automatic updates off and
manual replacement steps.

## Decision

Developer ID builds check the release feed every hour, download a new DMG in
the background and offer **Restart to Update**. A download is installed only
when both checks pass, and is deleted otherwise:

1. an ed25519 signature by the release key over
   `fanta-update-v1:{asset}:{version}:{sha256}`, from the feed or from
   `{url}.sig` beside the DMG, so an older signed build cannot be served as a
   newer one;
2. `codesign --verify --deep --strict` with a requirement pinned to the
   running app's Apple Team ID.

A build without a release public key never polls. App Store builds keep
updating through the App Store. `auto_update` defaults to `true`.

## Consequences

- Good: users stay current without manual steps, and a compromised download
  host cannot push an unsigned or downgraded build.
- Bad: the release signing key is a critical secret. The backend's release
  feed is a contract, and because GitHub's `releases/latest` ignores
  prereleases, a release users should receive must be a full release.

## Confirmation

The verification tests in `crates/auto_update/src/verify.rs`.

## References

- #138 (2026-10-19), [LAUNCH.md](../alpha/LAUNCH.md)
