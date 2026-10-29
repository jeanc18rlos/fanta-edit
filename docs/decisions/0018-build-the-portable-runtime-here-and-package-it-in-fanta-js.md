---
status: accepted
date: 2026-10-19
recorded: 2026-10-27
decision-makers: Jean Rojas
tags: [runtime, repository, sdk]
---

# Build the portable runtime here and package it in Fanta-JS

## Context

Designs needed to play outside the app, in browsers and in Node, with the
same results as the editor. The engine lived only in this repository
([0008](0008-vendor-the-engine-into-this-repository.md)).

## Decision

Add the `fanta-runtime` crate: a native `Runtime`, a C JSON bridge and the
`fanta-runtime-wasm` binary that Emscripten compiles. The portable compiler
(`fanta-format::runtime`: source bundle to runtime program to deterministic
bundle) stays beside it. The JavaScript packages, the WebAssembly build
scripts and the browser CI live in `jeanc18rlos/Fanta-JS`, which builds from
the fanta-edit commit pinned in its `fanta-edit.rev`. The `fanta-runtime` and
`fanta-sdk` npm packages 0.1.0 were published on 2026-10-19. A native and
WebAssembly parity fixture (`playback_fixture`) keeps results identical.

## Options considered

1. **Rust runtime here, JavaScript packaging in Fanta-JS** (chosen). The pull
   request's reason: "The Rust engines stay here because the desktop app
   shares them."
2. **Everything in one repository.** JavaScript tooling and browser CI inside
   the editor's Rust workspace.

## Consequences

- Good: one engine for the editor and the runtime, with tested parity.
- Bad: Fanta-JS checks out the entire editor to build the runtime. Schema 4
  shipped to the desktop app as a side effect of the runtime work. The
  WebAssembly build measured 13.5 MB (3.4 MB brotli) on 2026-10-21, and the
  runtime is GPL-3.0-only, which blocks embedding in closed-source apps
  ([0026](0026-choose-the-engine-and-sdk-licence.md)).

## Confirmation

The `fanta-runtime` workflow tests the runtime and the engine crates it uses
and builds the `playback_fixture` example, and the parity fixture runs in
Fanta-JS CI.

## References

- #132 (2026-10-19), `jeanc18rlos/Fanta-JS` PR #2
- [0024](0024-move-the-engine-into-its-own-repository.md) proposes building the runtime from the engine repository instead
