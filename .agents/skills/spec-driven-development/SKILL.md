---
name: spec-driven-development
description: Find, write and follow Fanta's specs and decision records (ADRs). Use before starting a feature, a contract, file-format or API change, a new or moved crate, work that spans repositories or needs more than one pull request; when asked to plan, spec, design or record a decision; and before changing docs/architecture/layers.toml.
---

# Specs and decisions in Fanta

The [process guide](../../../docs/process/spec-driven-development.md)
explains the why. This is the checklist.

## Before writing code

1. Read [`specs/README.md`](../../../specs/README.md) and
   [`docs/decisions/README.md`](../../../docs/decisions/README.md). Open every
   spec and accepted ADR that touches the area: `grep -ril <area> specs docs/decisions`.
2. Decide whether the change needs a spec. It does when it adds or changes
   something users see or rely on; changes a file format, schema, wire
   contract, public API, MCP tool or design op; adds, removes or moves a
   crate; spans repositories; touches release, billing, security or privacy;
   or needs more than one pull request.
   - **An accepted spec covers it:** implement the next open task in its
     `tasks.md`, one task per pull request.
   - **It needs a spec and none exists:** run `script/architecture new spec
     "Title"`, fill `spec.md` (problem with evidence, goals, non-goals,
     numbered requirements, acceptance criteria, open questions), leave it
     `draft`, open the pull request with the spec only, and stop for the
     founder's review. Implement in the same pull request only when the
     founder says so.
   - **It is below the threshold:** go ahead, and explain why in the pull
     request's "Spec and decisions" section.
3. Choosing between options with lasting consequences, or changing
   `docs/architecture/layers.toml`, needs a decision record: run
   `script/architecture new adr "Title"` and leave it `proposed`.

## While implementing

- Stay inside the task. If the plan has to change, edit `design.md` or
  `tasks.md` in the same pull request and say what changed and why.
- Name requirements and acceptance criteria (`R2`, `A3`) in test names or the
  pull request, so the reviewer can trace them.
- Never mark a spec or an ADR accepted yourself. Never edit the decision of
  an accepted ADR: write a new record that supersedes it.
- When `script/architecture check` refuses a dependency, fix the code: move
  it inward or depend on an inner layer. Add a seam only with an ADR the
  founder accepts.

## Before opening the pull request

- Run `script/architecture verify`. It needs no build and takes seconds.
- After changing crate dependencies or adding a crate, run
  `script/architecture map` and commit the regenerated map. A new crate
  outside the shell needs a layer in `layers.toml` and a `description`.
- After adding or changing a record, run `script/architecture index`.
- Tick the task in `tasks.md` and link the pull request.
- Fill "Spec and decisions" in the description, for example
  `Spec: specs/0002-fanta-engine-repository (T6)` and
  `Decisions: adds 0027 (proposed)`.
- Fix the docs the change made wrong, in the same pull request.

## Recording a past decision

When a past decision shaped the code and has no record, write it as
`accepted` with `date` set to when it was made and `recorded` to today. Cite
the commits and pull requests, say plainly when the reasoning was never
written down, and point it out in the pull request so the founder can
confirm it.
