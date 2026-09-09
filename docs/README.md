# Fanta docs

Fanta's documentation is plain Markdown in this directory. There is no doc build
step: read the files here, or on GitHub.

- [`alpha/`](./alpha) — what ships in the alpha: [`SMOKE.md`](./alpha/SMOKE.md)
  (the manual pass before a build goes out), [`RELEASE_NOTES.md`](./alpha/RELEASE_NOTES.md)
  and [`KNOWN_ISSUES.md`](./alpha/KNOWN_ISSUES.md).
- [`alpha/LAUNCH.md`](./alpha/LAUNCH.md) — backend, billing, analytics, and
  GitHub installer requirements for the first customer release.
- [`fanta/`](./fanta) — notes on the app itself, including
  [`disabled-services-binnacle.md`](./fanta/disabled-services-binnacle.md),
  the record of which inherited Zed services are switched off and why.
- [`live-mcp-v2.md`](./live-mcp-v2.md) and
  [`live-mcp-v2-implementation.md`](./live-mcp-v2-implementation.md) — the live
  MCP server agents connect to.

Zed's mdBook (`docs/src`, `docs/theme`, `book.toml`) was removed: none of it
described Fanta.
