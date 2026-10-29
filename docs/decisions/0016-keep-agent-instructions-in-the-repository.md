---
status: accepted
date: 2026-10-15
recorded: 2026-10-27
decision-makers: Jean Rojas
tags: [process, agents]
---

# Keep agent instructions in the repository

## Context

The root `.rules` was still upstream Zed's file. It told agents to run a
twenty-minute unscoped lint and pointed at a missing script. Thirty-five of
the first 134 pull requests proposed rules that never landed, so the same
traps recurred, and durable traps lived only in one machine's session memory.
Codex wrote most pull requests and reads only `AGENTS.md` files and
`.agents/skills`.

## Decision

- The root `.rules`, symlinked as `CLAUDE.md` and `AGENTS.md`, holds
  repository-wide rules and lists every crate rules file, because Codex does
  not load nested files on its own.
- Crate traps live in `crates/<crate>/AGENTS.md` with a `CLAUDE.md` symlink,
  land in the pull request that found them, and cite it.
- Root-level suggestions go in pull request descriptions under "Suggested
  .rules additions"; the `rules-harvest` skill lands them in a dedicated pull
  request.
- Skills live in `.agents/skills/` with symlinks in `.claude/skills/`.
- A PreToolUse hook blocks an unscoped `script/clippy` and builds into the
  main checkout's `target/` from a worktree.

## Consequences

- Good: Claude Code and Codex read the same rules, and traps outlive sessions.
- Bad: rules go stale like any document, so each one must earn its place: the
  "Rules hygiene" section sets the bar.

## Confirmation

The `rules-harvest` skill, `.claude/hooks/guard_bash.py`, and review of every
`AGENTS.md` change.

## References

- #137 (2026-10-15)
