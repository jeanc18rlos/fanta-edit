# Public Release - PII & History Cleanup

**WARNING**: Releasing this fork publicly requires removing or anonymizing personal traces.

## Findings (as of 2026-07-13 audit - final sweep)

### Git History (immutable without rewrite)
- 56+ commits authored exclusively by: `Jean Rojas <jeanc16rlos@gmail.com>`
- Every commit, tag, reflog carries the real name + email.
- Some historical patches/diffs contained `/Users/jeanrojas/fanta-edit*` and worktree paths.
- GitHub username `jeanc18rlos` appeared in docs and FORK.md (cleaned in working tree).

### Current Source Tree (clean)
Full recursive grep (names, emails, personal home paths) excluding `target/` and `.claude/worktrees/`:
- No `jeanc*` or `jeanrojas` identifiers remain in any committed or tracked source.
- Main docs (`README.md`, `FORK.md`, binnacle, live-mcp docs) normalized to placeholders.
- `api.fantaisa.net` references updated to `api.fanta.dev`.
- `.claude/launch.json` (untracked) had a path to another personal project — normalized.

The background full-tree scan only surfaced hits inside `.claude/worktrees/` (stale local clones) and the intentional documentation inside this `RELEASE_PII_CLEANUP.md` file.

Remaining considerations:
- If you change GitHub org/username, update the "This fork" link.
- The sibling `fanta-engine-migration` repo likely has the same author history — audit it too.
- Before release: `rm -rf .claude/worktrees` (they are local).

## Recommended History Rewrite (destructive - do on a fresh clone or backup)

Install git-filter-repo (best tool):

```sh
# macOS
brew install git-filter-repo
# or pipx install git-filter-repo
```

Then (from a clone of your public-facing branch):

```sh
cd fanta-edit

# 1. Create a mailmap to map the old identity (optional if you want to keep credit under a new name)
cat > .mailmap << 'EOF'
Your Public Name <public@example.com> Jean Rojas <jeanc16rlos@gmail.com>
EOF

# 2. Rewrite history (removes the old author from all commits)
git filter-repo --mailmap .mailmap --force

# 3. (Optional) Also expunge any remaining personal strings from all blobs
# git filter-repo --replace-text <(echo '/Users/jeanrojas==> /Users/developer') --force

# 4. Review the result carefully (git log --all --oneline | head)
# 5. Force push the cleaned history (this will rewrite public history)
git push --force-with-lease origin main
```

**Consequences of rewrite**:
- All commit SHAs change.
- Anyone who forked/cloned the old history will have to rebase.
- GitHub PRs, issues references in commits may break.
- Do this *before* the first public announcement if possible.

Alternative (lighter): Just leave real name (common in open source) and only clean strings + update your GitHub profile/org.

## Other pre-release PII / leak checks performed
- No obvious API keys, tokens, passwords in fanta crates.
- Live MCP uses private per-user Unix domain socket in temp dir (good).
- No committed .env or secrets (gitignore covers .env*).
- Engine path dependencies point outside the tree — audit the sibling repo identically.
- Docs no longer hardcode personal machine paths.

## Backend / Infrastructure
- `api.fantaisa.net` was replaced because it appeared to be a personal domain.
- Update all references, DNS, certs, and the actual hosted MCP server before enabling features that talk to it.
- Align with the disabled-services binnacle (own your domain, privacy policy, etc.).

Run this checklist again after any history rewrite and before tagging a public release.
