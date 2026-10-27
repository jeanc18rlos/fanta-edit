# Design: One GitHub organization

## Approach

A transfer keeps issues, pull requests, wiki, stars, watchers, secrets,
webhooks and deploy keys, and redirects old web and git URLs. It fails when
the target has a repository with the same name. GitHub does not redirect
Pages sites, and creating a repository at an old location deletes its
redirects for good. The REST transfer call can rename in the same step, which
avoids a chain of redirects:

```bash
gh api repos/jeanc18rlos/fanta-edit/transfer -f new_owner=fantaisa-ai -f new_name=fanta-editor
```

Each move is one task with the same three phases.

1. **Inventory** before the move: issue, pull request and release counts;
   `gh secret list` and `gh variable list` names; webhooks, deploy keys and
   environments; GitHub App installations; the Vercel or Cloud Build project
   that deploys from it; every file in every repository that hard-codes the
   old URL (on 2026-10-27: 44 files in the editor name
   `jeanc18rlos/fanta-edit` and 9 name `squidred-dev/fanta-ui`).
2. **Move**: the transfer call, with the rename where the table asks for one.
3. **Check** against the inventory, then a cleanup pull request in each
   repository that replaced old URLs, keeping URLs that are test data.

### Order

| Step | Move | What to check afterwards |
| --- | --- | --- |
| 0 | Delete or rename the three 2026-10-27 copies (Q1) | Nothing depends on them |
| 1 | `squidred-dev/fanta-ui` → `fanta-ui` | The editor still fetches its 41 pinned packages; then a pull request updates the git URL in `Cargo.toml` and the lockfile's `source` lines |
| 2 | `fanta-docs`, `fanta-admin` | Vercel deployments of the docs and the admin |
| 3 | `fantaisa-landing` → `fanta-landing`, `fanta-dashboard` → `fanta-app` | Vercel deployments; fantaisa.net/download |
| 4 | `fanta-backend` | Vercel and Cloud Run deployments, the release feed, the GitHub App used for cloud review, webhooks |
| 5 | `Fanta-JS` → `fanta-sdk` | npm publishing, the engine pin, the browser CI |
| 6 | `fanta-edit` → `fanta-editor` | A2 and A3: release redirects, the download page, the update feed, the SDK pin, docs links; macOS signing secrets |
| 7 | Create `fanta-engine` | Spec 0002 |
| 8 | `fantaisa-ai/.github` | Templates show up in a new pull request in each repository |
| 9 | Rules and the agent identity | A4 and A5 |
| 10 | Archive or delete stale repositories | A6 |

### Rules for public repositories

A repository ruleset on `main`: require a pull request with one approving
review from the founder; require the `Check` and `Architecture` workflows;
block deletion and force pushes; allow merge commits (the history uses them).
Keep the existing tag ruleset for `alpha-*`. Required review only works once
agents open pull requests under their own identity (Q3).

## Interfaces and data

None.

## Alternatives

ADR 0025 compares transfers with copies and with staying put.

## Risks

- **A redirect is lost** because someone creates a repository at an old name.
  Never reuse an old name.
- **An integration silently stops** (a Vercel project, a webhook, a GitHub
  App). The inventory lists each one and the check step tests it.
- **Required checks block urgent fixes.** The founder keeps the ability to
  bypass rules as an organization owner; every bypass is visible in the
  ruleset's history.

## Verification

Each task records its inventory and the result of its checks in its pull
request or issue.
