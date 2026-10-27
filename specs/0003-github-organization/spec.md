---
status: draft
date: 2026-10-27
owner: Jean Rojas
decisions: [0025]
repositories: [fanta-ui, fanta-editor, fanta-engine, fanta-sdk, fanta-docs, fanta-landing, fanta-backend, fanta-admin, fanta-app]
---

# One GitHub organization

## Problem

The product's repositories belong to a personal account and two
organizations (ADR 0025). Copies made in `fantaisa-ai` on 2026-10-27 have no
issues, pull requests, releases or redirects. Nothing on `main` requires a
pull request or a green check: its only rules block deletion and force
pushes, and 39 commits reached `main` directly after pull requests began.
Agents act as the founder on GitHub, so the founder cannot be a required
reviewer of their pull requests.

## Goals

- Every product repository lives in `fantaisa-ai` with its history, issues,
  pull requests, releases and secrets.
- Every published link and integration keeps working.
- Public repositories accept changes only through reviewed pull requests with
  green checks.

## Non-goals

- Moving personal or unrelated repositories.
- Changing CI providers or hosting.
- Deciding the paid plan; Q2 lists the trade-off.

## Requirements

- **R1.** Each repository in ADR 0025's table must be in `fantaisa-ai` with
  its issues, pull requests, releases, stars and secrets.
- **R2.** Old web, git and release-download URLs must redirect.
- **R3.** Nothing published may break: the download page, the update feed,
  docs links, the SDK's engine pin, Vercel's Git connections, Cloud Build or
  Cloud Run triggers, the GitHub App that cloud review uses, and webhooks.
- **R4.** Public repositories must have rules on `main`: a pull request is
  required, the Check and Architecture workflows must pass, and deletion and
  force pushes are blocked. Release tags stay protected.
- **R5.** Agents must open pull requests under their own identity, and the
  founder's approval must be required on public repositories.
- **R6.** Shared community files (pull request template, issue templates,
  `CONTRIBUTING`, `SECURITY`) must come from `fantaisa-ai/.github`.
- **R7.** Look-alike and stale repositories must be archived or deleted, each
  by the founder's choice.
- **R8.** Local checkouts and worktrees must point at the new remotes.

## Acceptance criteria

- **A1 (R1).** For each repository, issue, pull request and release counts
  match before and after its move.
- **A2 (R2).** The old `releases/download/alpha-1.0.0-20261012/Fanta-aarch64.dmg`
  URL answers with a redirect to the new location, and `git ls-remote` on
  each old URL works.
- **A3 (R3).** The download page and the update feed serve the latest release
  from the new location; every Vercel project deploys from its moved
  repository; a cloud review run completes.
- **A4 (R4).** A pull request without green checks cannot merge into
  `fantaisa-ai/fanta-editor`, and a direct push to `main` is rejected.
- **A5 (R5).** A pull request opened by the agent identity shows the founder
  as a required reviewer.
- **A6 (R7, R8).** `gh repo list squidred-dev` and `gh repo list jeanc18rlos`
  show no product repository outside the archived ones, and
  `git remote get-url origin` in each local checkout names `fantaisa-ai`.

## Open questions

- **Q1.** Delete the three copies made in `fantaisa-ai` on 2026-10-27, or
  rename them? A transfer fails while a repository with the same name exists.
  Recommended: delete them; nothing in them is missing from the originals.
- **Q2.** Move to GitHub Team so the four private repositories get branch
  rules and more Actions minutes? On the Free plan they get neither branch
  protection nor rulesets.
- **Q3.** Which identity for agents: a GitHub App installed on the
  organization (recommended: short-lived tokens, scoped to repositories) or a
  machine user?
- **Q4.** What happens to `squidred-dev/fanta-platform` and the other
  `squidred-dev` repositories?
- **Q5.** When to move the editor? Recommended: after the engine repository
  exists, at a time with no release in flight.

## Changes after acceptance

- None.
