---
status: proposed
date: 2026-10-27
decision-makers: Jean Rojas
tags: [repository, governance]
---

# Organize the repositories under one GitHub organization

## Context

Fanta's repositories are spread over three owners:

- `jeanc18rlos` (personal): `fanta-edit` (public), `fanta-docs` (public),
  `Fanta-JS` (public), `fanta-backend`, `fantaisa-landing`, `fanta-admin` and
  `fanta-dashboard` (private).
- `squidred-dev`: `fanta-ui` (public), the stale engine carve-out
  `fantaisa-engine`, `fanta-platform`, and two empty private placeholders
  named `fanta-engine` and `fanta-editor`.
- `fantaisa-ai` (display name "Fanta", created 2026-10-19, Free plan): on
  2026-10-27 it received private copies of `fanta-ui`, `fanta-edit` and
  `fanta-backend`. They are separate repositories with the same commits, not
  transfers, so they carry no issues, pull requests, releases or redirects.

A personal account owns the main product, and two organizations hold
look-alike names. Release URLs on the download page, the update feed, the
Fanta-JS engine pin and the docs all point at `jeanc18rlos/fanta-edit`.

## Decision

Proposed: one organization, `fantaisa-ai`, owns every product repository.

| Repository | Visibility | Comes from |
| --- | --- | --- |
| `fanta-ui` | public | transfer `squidred-dev/fanta-ui` |
| `fanta-editor` | public | transfer `jeanc18rlos/fanta-edit`, then rename |
| `fanta-engine` | public | new, extracted from the editor ([0024](0024-move-the-engine-into-its-own-repository.md)) |
| `fanta-sdk` | public | transfer `jeanc18rlos/Fanta-JS`, then rename; Rust, Python and Node packages join the JavaScript ones |
| `fanta-docs` | public | transfer `jeanc18rlos/fanta-docs` |
| `fanta-landing` | private | transfer `jeanc18rlos/fantaisa-landing`, then rename |
| `fanta-backend` | private | transfer `jeanc18rlos/fanta-backend` |
| `fanta-admin` | private | transfer `jeanc18rlos/fanta-admin` |
| `fanta-app` | private | transfer `jeanc18rlos/fanta-dashboard` (the app at app.fantaisa.net, whose package is already named `fanta-app`), then rename |

Move repositories by GitHub transfer, not by copying. A transfer keeps
issues, pull requests, wiki, stars, watchers, secrets, webhooks and deploy
keys, and redirects old web and git URLs. A transfer fails when the target
already has a repository of that name, so the three copies made on
2026-10-27 must be deleted or renamed first.

Organization-wide:

- A `fantaisa-ai/.github` repository holds the shared pull request template,
  issue templates, `CONTRIBUTING`, `SECURITY` and the process guide's
  summary, so every repository follows the same spec and decision flow.
- Each repository keeps its own `docs/decisions`. A decision that spans
  repositories lives where the contract lives (cloud contracts in the engine,
  billing in the backend), and the others link to it.
- Agents open pull requests with their own identity (a GitHub App), so the
  founder can be the required reviewer. GitHub does not let authors approve
  their own pull requests, and today agents act as the founder.

## Options considered

1. **One organization, transfers** (proposed).
2. **Keep the current owners.** Ownership stays scattered, and a personal
   account remains the single owner of the product.
3. **Copy into the organization** (what happened on 2026-10-27). Loses
   issues, pull requests, releases, stars and every redirect, and breaks the
   release URLs that the download page and update feed use.

## Consequences

- Good: one place to find, permission and govern everything; a clean public
  face; shared templates.
- Bad: URLs change. Git and web URLs redirect after a transfer, but the
  redirect disappears if anyone creates a repository at the old name, and
  GitHub Pages is not redirected. Release asset redirects, the download page
  and the update feed must be checked after the editor moves. On the Free
  plan, private repositories get no branch protection or rulesets, and
  private Actions minutes are limited (macOS minutes count ten times); the
  four private repositories need GitHub Team for merge rules.

## Confirmation

Spec [0003](../../specs/0003-github-organization/spec.md) lists a check for
each move: redirects, secrets, releases, pins and published links.

## References

- [GitHub: transferring a repository](https://docs.github.com/en/repositories/creating-and-managing-repositories/transferring-a-repository)
- Repository listings taken with `gh repo list` on 2026-10-27
