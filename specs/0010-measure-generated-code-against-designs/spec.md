---
status: draft
date: 2026-11-12
owner: Jean Rojas
decisions: [0027]
---

# Measure generated code against designs

## Problem

Fanta stores designs as web files
([ADR 0027](../../docs/decisions/0027-project-designs-onto-plain-web-files.md))
on the belief that a model writes more accurate product code from them than
from a screenshot or from another tool's JSON. The founder said on
2026-11-12 that this is still a hypothesis, and that the runtime, the
libraries and the hosted plans all lean on it. Nothing in the repository
measures it.

A second problem has the same solution. A team that builds from a design
has no check that the shipped code still matches it. The measurement the
hypothesis needs (render code, compare with the design) is that check.

## Goals

- A number that says whether Fanta's files improve generated code, produced
  by one command and reproducible.
- The scorer behind that number is a command teams run in CI to fail a
  change that drifts from the design.
- When the hypothesis fails, the result says which design properties the
  model got wrong, so the format can be fixed.

## Non-goals

- A code generator product, prompts tuned per customer, or model training.
- Pixel-identical output. A design renderer and a browser differ in text
  rasterization; the scorer has a tolerance.
- Native targets (SwiftUI, Compose, Flutter). Web output only.
- Interaction and accessibility testing. Accessibility findings on designs
  extend [spec 0009](../0009-check-repair-and-snapshot-project-files/spec.md)'s
  checker; interaction tests come with real-browser prototypes.
- Giving the model Fanta's MCP tools during the benchmark. The first run
  isolates the file format: every condition gets static input only.

## Requirements

- **R1.** A `compare` command takes a design (project directory plus a page
  or component, by name or prefixed id) and a target (a URL or a local HTML
  file, with an optional CSS selector), and prints a score as JSON. With a
  threshold it exits non-zero when the score is worse.
- **R2.** Pixel score. The design is rendered by the headless engine and
  the target by a headless browser, at the same viewport width and scale,
  with the project's fonts available to both. The comparison is per mapped
  component (R4), with a documented tolerance, and writes a difference image
  for each failing component.
- **R3.** Style score. For each mapped node the target element's computed
  styles are compared with the design's values: colour, font family, size,
  weight and line height, padding and gap, corner radius, and size where the
  design fixes it. The score also reports token reuse: of the design values
  that are bound to tokens, the share the code expresses through the
  corresponding custom property and not a literal.
- **R4.** Mapping between design nodes and target elements is explicit: an
  attribute on the element or a map file. Unmapped design nodes are listed
  in the output and counted; they are never skipped silently.
- **R5.** The same inputs give the same score: the browser version is
  pinned, animations are disabled, and no network resource is fetched.
- **R6.** A benchmark command runs a fixed set of at least 20 designs
  through three input conditions with the same model and the same prompt
  apart from the input: a screenshot; a JSON node tree of the kind other
  design tools export; the Fanta project files. Each design and condition
  runs three times, because model output varies.
- **R7.** The design set is split before any run into a tuning set and a
  held-out set. Prompts may be adjusted against the tuning set only; the
  reported result is from the held-out set.
- **R8.** A run records the model id, the date, the prompts, and both scores
  per design, condition and run, in a file committed with the result.
- **R9.** The hypothesis holds when, on the held-out set, the Fanta
  condition is at least 20% better than the better of the two baselines on
  both the pixel score and the style score (founder, 2026-11-12). The
  threshold is fixed here, before the first run.
- **R10.** If R9 holds, the design set, the prompts and the results are
  published with spec 0009's conformance fixtures. If it does not, the
  result lists the properties with the largest error in the Fanta condition,
  as input to a format spec.
- **R11.** A documented CI example runs `compare` on a repository and fails
  a change that moves the code away from the design.

## Acceptance criteria

- **A1 (R1, R2).** Comparing a design with the page Fanta's own writer emits
  for it scores inside the tolerance. This run sets the tolerance.
- **A2 (R2, R3, R4).** Against copies of that page with one change each (a
  fill colour, an 8 px shift, a font size, a literal replacing a token),
  `compare` fails and names the node and the property.
- **A3 (R4).** A target missing the mapping for one node lists that node as
  unmapped.
- **A4 (R5).** Two runs on the same inputs produce byte-identical JSON.
- **A5 (R6, R7, R8).** The benchmark command writes the result file for
  three conditions, three runs each, over the committed design set, with
  the split recorded.
- **A6 (R9, R10).** The result file states whether R9 holds, and when it
  does not, lists the worst properties.
- **A7 (R11).** In the example repository, a change to one token's value in
  code only makes the CI job fail with that token named.

## Open questions

- **Q1.** Which models. Recommendation: the current Claude model and one
  from another vendor, so the result is not a property of one model.
- **Q2.** Which target the model writes. Recommendation: plain HTML and CSS
  first, since it needs no build step and isolates the format; React next.
- **Q3.** The design set. The repository holds one sample `.fig`
  (`docs/samples/`). The benchmark needs about 20 designs the founder may
  publish. Who supplies them?
- **Q4.** The JSON baseline. The honest competitor input is the JSON another
  tool's API returns for the same file, which requires files the founder
  owns in that tool. The alternative is a neutral node tree written here.
  Which?
- **Q5.** Where the browser half lives. The engine crates must not grow a
  browser dependency; candidates are a script under `web/` that calls the
  engine binary, or a binary that drives an installed browser. To settle in
  `design.md`.
- **Q6.** The mapping attribute's name, and whether Fanta's own emitted HTML
  already carries enough (node `id`s) to serve as the convention.

## Changes after acceptance

- None.
