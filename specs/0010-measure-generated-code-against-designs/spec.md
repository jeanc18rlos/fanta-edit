---
status: accepted
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
- Platform-native toolkits (SwiftUI, Compose, Flutter). The targets are the
  ones R12 names.
- Interaction and accessibility testing. Accessibility findings on designs
  extend [spec 0009](../0009-check-repair-and-snapshot-project-files/spec.md)'s
  checker; interaction tests come with real-browser prototypes.
- Scoring on a phone or simulator. React Native output is scored through
  Expo's web build (R12).

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
  through four input conditions with the same model and the same prompt
  apart from the input: a screenshot; the node JSON another design tool
  gives an agent for the same design (Pencil, read through its MCP server,
  if Q4 proves feasible); the Fanta project files; the Fanta project files
  plus Fanta's MCP tools. The models are the ones Fanta's agent already
  offers. Each design and condition runs three times, because model output
  varies.
- **R7.** The design set is split before any run into a tuning set and a
  held-out set. Prompts may be adjusted against the tuning set only; the
  reported result is from the held-out set.
- **R8.** A run records the model id, the date, the prompts, and both scores
  per design, condition and run, in a file committed with the result.
- **R9.** The hypothesis holds when, on the held-out set and the React
  target, the Fanta-files condition is at least 20% better than the better
  of the two baselines on both the pixel score and the style score (founder,
  2026-11-12). The threshold is fixed here, before the first run. The
  condition with MCP tools and the other targets are reported beside it and
  do not change the verdict: they answer how much the tools add and whether
  the result carries to mobile.
- **R10.** If R9 holds, the design set, the prompts and the results are
  published with spec 0009's conformance fixtures. If it does not, the
  result lists the properties with the largest error in the Fanta condition,
  as input to a format spec.
- **R12.** Targets, in this order (founder, 2026-11-12): React for the web;
  React Native with Expo for mobile, scored through Expo's web build, with
  the result labelled as a browser rendering of the mobile code; Bevy's UI
  as the Rust target for games (founder, 2026-11-12), a later extension with the pixel score only, since it has no
  computed styles to read. The first reported run needs React; each further
  target is its own task.
- **R13.** The design set is drawn from the most used free Figma Community
  files (founder, 2026-11-12), several screens from each, mixing forms,
  dashboards, marketing pages and mobile screens. Each file's licence is
  checked and recorded; free Community files default to CC BY 4.0, which
  allows redistribution with attribution, and the published set carries an
  attribution file. A file under other terms is left out.
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
  four conditions, three runs each, over the committed design set, with
  the split recorded.
- **A6 (R9, R10).** The result file states whether R9 holds, and when it
  does not, lists the worst properties.
- **A8 (R12).** The result file has a React section, and an Expo section
  marked as scored on the web build.
- **A9 (R13).** The committed set has an attribution file naming each
  source file, its author and its licence, and a test fails when a design
  has no entry.
- **A7 (R11).** In the example repository, a change to one token's value in
  code only makes the CI job fail with that token named.

## Open questions

- **Q1.** ~~Which models.~~ **Resolved by the founder, 2026-11-12:** the
  models Fanta's agent already offers, and a condition that also gives the
  model Fanta's MCP tools (R6).
- **Q2.** ~~Which target.~~ **Resolved by the founder, 2026-11-12:** React
  for the web, React Native with Expo for mobile, possibly a Rust UI for
  games (R12).
- **Q3.** ~~The design set.~~ **Resolved by the founder, 2026-11-12:** the
  most used Community files (R13). Six were downloaded the same day from
  the Community's own popularity ranking for free UI kits, all listed as
  CC BY 4.0: Obra Studio's shadcn ui kit (components and forms), CRM UI Kit
  for SaaS Dashboards and DashStack (dashboards), Vectorfair's finance and
  food-delivery kits (mobile), and Framer's AI Startup Website kit
  (marketing). They are held outside the repository with an attribution
  file until `design.md` says how large fixtures are stored: the six total
  242 MB. Choosing the 20 screens and the tuning split is a task.
- **Q4.** The JSON baseline. The founder suggested Pencil (2026-11-12). Open
  until `design.md` shows the same designs can be brought into Pencil and
  read back through its MCP server; if not, the fallback is a neutral node
  tree written here, which is the weaker comparison.
- **Q5.** ~~Where the browser half lives.~~ **Resolved by the founder,
  2026-11-12:** a script under `web/` that drives a headless browser and
  calls the engine binary; the engine crates gain no browser dependency.
- **Q6.** The mapping attribute. Recommendation: the node `id`s Fanta's
  emitted files already carry, and a `data-fanta-id` attribute where they do
  not reach. To confirm in `design.md` against the emitted HTML.
- **Q7.** ~~Which Rust UI.~~ **Resolved by the founder, 2026-11-12:** Bevy's
  UI (R12).

## Changes after acceptance

- None.
