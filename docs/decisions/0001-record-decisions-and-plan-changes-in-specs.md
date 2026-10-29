---
status: proposed
date: 2026-10-27
decision-makers: Jean Rojas
tags: [process]
---

# Record decisions in ADRs and plan changes in specs

## Context

Fanta went from an empty fork on 2026-07-07 to a released 2.0 line in under
four months. Until the first pull request on 2026-09-13, all 100 first-parent
commits went straight to `main`. Since then 137 changes arrived as merged pull
requests and 39 more were still pushed directly, such as the 2026-10-02
integration commits.

The reasons behind the system are scattered. Some sit in pull request
descriptions, some in plan files under `~/.claude/plans`, some in session
memory on one machine, and some in dated reports under `docs/alpha/` next to
current guides. Codex writes most pull requests (88% when #137 measured it)
and reads only what the repository holds. Answering "why is it built this
way?" takes archaeology. Nothing stops an agent from changing a contract that
another repository depends on.

The first app's repository (`jeanc18rlos/fanta`, private) tried a heavier
version in June 2026: feature specs of 1,200 to 1,400 lines written by
parallel agents, and a paired JSON and Markdown memory harness. The specs
drifted from the code, and the harness doubled the bookkeeping.

## Decision

- Record each architecturally significant decision as a numbered ADR in
  `docs/decisions/`, one decision per file, with a status in its front matter.
- Plan each change above the threshold in the
  [process guide](../process/spec-driven-development.md) as a folder in
  `specs/` with `spec.md` (what and why), `design.md` (how) and `tasks.md`
  (one pull request per task).
- The founder approves specs and accepts ADRs. Agents write drafts and
  proposals and never accept their own.
- `script/architecture` generates the indexes and the crate map. CI fails when
  a record is invalid or a generated file is stale.
- Reconstruct the decisions already made (0002 to 0022) from the history,
  citing the evidence and saying where the reasoning was never written down.

## Options considered

1. **ADRs and small staged specs in the repository** (chosen). They are
   versioned with the code, reviewed in pull requests and read by every agent.
2. **Keep decisions in pull requests and session notes.** No status and no
   index; invisible to an agent in another session or another tool.
3. **An external wiki or tracker.** Agents cannot read it reliably, and it
   drifts from the code it describes.
4. **GitHub Spec Kit as it ships.** Its `spec.md`, `plan.md` and `tasks.md`
   shape is close to this one, but its CLI and commands would be one more tool
   to keep working for two different agents. The chosen layout keeps a later
   move cheap.
5. **The memory harness from the first app.** Two formats for every record,
   and validators for both.

## Consequences

- Good: decisions are searchable and carry a status. A new agent session
  starts from the indexes instead of guessing. A contract change needs a
  written reason before it lands.
- Bad: a few minutes of writing before each change above the threshold.
  Records can describe intent the code no longer follows: CI checks their
  structure, not their truth, so the weekly pass in the process guide matters.
- Follow-up: point agents at the process from `.rules`, and add branch rules
  on `main` (spec 0003).

## Confirmation

The Architecture workflow runs `script/architecture verify` on every pull
request. The pull request template asks which spec and decisions each change
follows.

## References

- [Process guide](../process/spec-driven-development.md)
- #137, which moved agent rules into the repository ([0016](0016-keep-agent-instructions-in-the-repository.md))
