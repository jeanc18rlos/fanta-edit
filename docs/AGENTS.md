# Writing docs in this repo

These are plain Markdown files with no build step, no preprocessor and no
table-of-contents file to keep in sync. Add a page by adding a file and linking
it from [`README.md`](./README.md).

- Put alpha-release material in [`alpha/`](./alpha) — anything a tester or a
  release needs: smoke steps, release notes, known issues.
- Put notes about how the app works in [`fanta/`](./fanta).
- Keep pages short and current. A stale page is worse than a missing one: if a
  change makes a page wrong, fix or delete it in the same commit.
- Do not document inherited Zed behaviour that Fanta does not ship. When a
  service is deliberately off, record it in
  [`fanta/disabled-services-binnacle.md`](./fanta/disabled-services-binnacle.md)
  rather than writing a page about it.
