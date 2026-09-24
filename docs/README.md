# Fanta docs

Fanta's documentation is plain Markdown in this directory. There is no doc build
step: read the files here, or on GitHub.

- [`alpha/`](./alpha) — what ships in the alpha: [`SMOKE.md`](./alpha/SMOKE.md)
  (the manual pass before a build goes out), [`RELEASE_NOTES.md`](./alpha/RELEASE_NOTES.md)
  and [`KNOWN_ISSUES.md`](./alpha/KNOWN_ISSUES.md).
- [`alpha/LAUNCH.md`](./alpha/LAUNCH.md) — backend, billing, analytics, and
  GitHub installer requirements for the first customer release.
- [`alpha/MAC_APP_STORE.md`](./alpha/MAC_APP_STORE.md) — Apple signing, RevenueCat,
  App Review, and Shipaton release checklist.
- [`alpha/RELEASE_VALIDATION.md`](./alpha/RELEASE_VALIDATION.md) — measured
  performance, completed checks, and remaining release verification.
- [`alpha/RELEASE_STATUS_2026-09-12.md`](./alpha/RELEASE_STATUS_2026-09-12.md) —
  dated release report; [`SOL_ULTRA_HANDOFF.md`](./alpha/SOL_ULTRA_HANDOFF.md)
  provides the ordered continuation plan and ready-to-paste instructions.
- [`alpha/SOL_REVIEW_2026-09-12.md`](./alpha/SOL_REVIEW_2026-09-12.md) —
  review findings, reproduced failures, completed Sol work, and remaining gaps.
- [`fanta/`](./fanta) — notes on the app itself, including
  [`project-schema.md`](./fanta/project-schema.md) for the editable project tree and
  [`code-first-audit.md`](./fanta/code-first-audit.md) for the architecture review, plus
  [`disabled-services-binnacle.md`](./fanta/disabled-services-binnacle.md),
  the record of which inherited Zed services are switched off and why.
- [`live-mcp-v2.md`](./live-mcp-v2.md) and
  [`live-mcp-v2-implementation.md`](./live-mcp-v2-implementation.md) — the live
  MCP server agents connect to.

Zed's mdBook (`docs/src`, `docs/theme`, `book.toml`) was removed: none of it
described Fanta.
