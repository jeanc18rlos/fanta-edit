---
status: draft
date: 2026-11-12
owner: Jean Rojas
decisions: []
---

# Evaluate the product agent on design tasks

## Problem

Fanta's agent edits designs through the design tools (`design_state`,
`design_edit`, `design_screenshot`, `design_comments`, `design_system`) and
the live MCP server. Whether it does that well is not measured. A change to
a tool description, to the project guide seeded into new projects, to a
skill or to the default model can make the agent better or worse, and
today the only way to find out is to try it by hand.

What exists on `main` at 9716f162 (2026-11-12):

- **An eval harness, inherited from Zed, for code tools only.** The
  `unit-eval` feature of the `agent` crate gates
  `crates/agent/src/tools/evals/`, which holds evals for `edit_file`,
  `write_file` and `terminal`. They run through `eval_utils` (repeated
  runs, a pass ratio, an outcome per run) and `script/run-unit-evals`. No
  eval covers a design tool, and no workflow runs the script.
- **A deterministic end-to-end check with no model.** `script/smoke-mcp`
  drives the running app over MCP and asserts on the result. It proves the
  tools work, not that an agent uses them well.
- **Tests of each design tool** with a fake surface. They prove the tool's
  contract, not the agent's behaviour.

The founder decided on 2026-11-12 to work on both sides of the AI harness:
guardrails for the coding agents that build Fanta (spec 0012, drafted
alongside this one), and evals for the product agent, as a draft spec
first. This is that draft. It does not build anything.

## Goals

- A change that affects the agent's behaviour on design tasks can be
  compared with `main` by running one command.
- A result says what the agent did to the document, in a form that can be
  checked without a second model.
- The same task set runs against the in-app agent and against an outside
  agent that speaks MCP.

## Non-goals

- Measuring generated code against a design. That is
  [spec 0010](../0010-measure-generated-code-against-designs/spec.md): it
  goes from a design to code; this spec goes from a request to a design.
- A public benchmark or a leaderboard.
- Training or fine-tuning a model.
- Running evals on every pull request. They cost money and are not
  deterministic; a gate needs evidence from this spec first.
- Evaluating the coding agents that work on this repository.
- Building the harness. This spec stops at the founder's review.

## Requirements

- **R1.** An eval case is a starting project, a request in plain language
  and a list of assertions. Cases live in the repository as files.
- **R2.** Assertions on the resulting document come first and must be
  deterministic: they read the document after the run (nodes, properties,
  hierarchy, tokens, components) and the project check's diagnostics
  ([spec 0009](../0009-check-repair-and-snapshot-project-files/spec.md)),
  and need no model. Every case must have at least one.
- **R3.** A model-judged assertion (for example on a screenshot) may be
  added to a case only beside deterministic ones, and must be reported
  separately, so a judge's opinion never hides or replaces a measured
  failure.
- **R4.** Every run must record the model, the tool calls with their
  arguments and results, the turn count, the token use and the wall-clock
  time, so a regression can be traced to a step.
- **R5.** A case runs several times, and the result is a pass ratio with
  the number of runs, never a single pass or fail.
- **R6.** The harness must reuse what exists before adding anything:
  `eval_utils` for repetition and outcomes, the `unit-eval` feature and
  `crates/agent/src/tools/evals/` for the in-app agent, the headless engine
  (`fanta-headless`, `fanta-mcp`) for runs with no window, and
  `script/smoke-mcp`'s MCP client for an outside agent.
- **R7.** Evals must run without the desktop app where the case allows it,
  so they can run on Linux and in CI.
- **R8.** A run must never write to a real project. Each run works on a
  copy of its starting project.
- **R9.** The first task set must cover: creating a layout from a
  description; editing a named layer; using an existing component and an
  existing token instead of new values; a multi-page change; and a request
  the agent should refuse or ask about.

## Acceptance criteria

- **A1 (R1, R2, R5).** One command runs the task set against a named model
  and prints, per case, the pass ratio of the deterministic assertions.
- **A2 (R2).** With the model replaced by a scripted sequence of tool
  calls, every case passes or fails the same way on every run.
- **A3 (R3).** A case whose deterministic assertions fail is reported as
  failed whatever its model-judged score.
- **A4 (R4).** The record of a failed run is enough to replay its tool
  calls against the starting project and reach the same document.
- **A5 (R7, R8).** The task set runs on Linux with no display, and the
  starting projects are byte-identical afterwards.
- **A6 (R6).** The design names what was reused and justifies each new
  piece.

## Open questions

- **Q1.** Which agent is measured first: the in-app agent (the `agent`
  crate with the design tools), or an outside agent over `fanta-mcp`? They
  share the tools but not the system prompt or the loop.
- **Q2.** Which models, and what budget per run of the task set?
- **Q3.** Where do results live: a file in the repository per run, an
  artifact of a nightly job, or the backend?
- **Q4.** Is `eval_utils` kept? It is inherited, GPL-licensed and its only
  users are the three code-tool evals. If the harness depends on it, it
  stays in the editor repository and cannot move to the engine.
- **Q5.** Are the three inherited code-tool evals kept, run or deleted?
  Nothing runs them today.
- **Q6.** Does the task set share starting projects with spec 0010's
  benchmark designs, or use small fixtures of its own?
- **Q7.** Who writes the expected documents for R9's cases: the founder, or
  an agent whose proposals the founder reviews?

## Changes after acceptance

- None.
