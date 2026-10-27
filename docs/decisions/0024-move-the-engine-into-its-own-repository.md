---
status: proposed
date: 2026-10-27
decision-makers: Jean Rojas
supersedes: [0008]
tags: [engine, repository]
---

# Move the engine into its own repository

## Context

This would be the third attempt at the engine boundary:

- 2026-07-10: the editor built the engine from a sibling checkout by path
  ([0006](0006-build-the-engine-from-a-sibling-checkout.md)). Nobody else
  could build it, and the public repository lacked the binary's source.
- 2026-09-08: the engine was vendored back
  ([0008](0008-vendor-the-engine-into-this-repository.md)). That works, but
  the engine can now only be reached by cloning the editor and its 128 Zed
  crates.

The engine has three consumers outside the desktop app, and each pays for
that: the cloud worker is a crate inside the editor repository; Fanta-JS
checks out the whole editor to build the WebAssembly runtime; and the planned
`fanta-sdk` (Rust, Python, Node and browser) needs a small engine with
versions of its own. A public SDK may also need a different licence from the
GPL editor ([0026](0026-choose-the-engine-and-sdk-licence.md)).

A dry run on 2026-10-27 (local only, nothing pushed) showed the move is
mechanical:

- `git filter-repo` on the engine paths kept 274 of the 800 commits, in a
  5.9 MB history against the editor's 46 MB, in under a second.
- The extracted crates resolve as their own 15-crate workspace with 320
  packages (the app links about 1,400), with no GPUI-family or Zed crate.
- The seven crates without Skia passed `cargo check` there in 10.8 seconds,
  with no warnings.

## Decision

Proposed:

1. Create `fanta-engine` in the organization
   ([0025](0025-organize-the-repositories-under-one-github-organization.md))
   by a history-preserving extraction of the engine group in
   [`layers.toml`](../architecture/layers.toml): the 15 crates,
   `vendor/skia-bindings-0.153.3`, the Skia source preparation (action and
   script), `fixtures/ink` and the engine docs (the cloud contract, the
   project schema and the parity matrix).
2. The editor depends on the engine crates by exact git revision in
   `[workspace.dependencies]`, and patches `skia-bindings` to the same
   revision. Never by sibling path. The in-tree copies are deleted in the
   same pull request that switches to the pin, so there is never a second
   copy to drift.
3. Engine changes land in the engine repository first. Developing both
   together uses a local `cargo --config` path patch, never committed (the
   [fanta-ui workflow](0011-consume-gpui-and-shared-ui-from-fanta-ui.md)).
4. The engine repository runs its own CI on Linux and macOS: the engine,
   import and source-order stages, the runtime build and the Skia source
   compatibility check. The editor keeps the editor stages and the render
   parity test that compares the editor with the headless engine, which now
   also guards the pin.
5. The cloud worker image and the SDK packages build from the engine
   repository. `fig_viewer` stays in the editor: it depends on 98 Zed crates.
   Its GPUI-free logic moves into the engine over time
   ([0023](0023-enforce-layered-crate-boundaries.md)).
6. Keep the July and August history reachable. It is the
   `feature/artifact-ir-workspace-session` branch of
   `squidred-dev/fantaisa-engine` (109 commits, ending at `9370fa2`), pushed
   on 2026-10-27. Copy it into the new repository as an archive branch, link
   it from the README, then archive `squidred-dev/fantaisa-engine`.

## Options considered

1. **A separate repository consumed by git revision, with the in-tree copy
   deleted** (proposed).
2. **Keep vendoring.** Every outside consumer keeps cloning the editor, and
   the licence cannot differ cleanly.
3. **Sibling path dependencies.** Failed before (0006).
4. **A one-way mirror of the in-tree engine.** No paired pull requests, but
   the public repository becomes read-only: issues and contributions cannot
   flow back, and two-way syncing breaks.
5. **Publish to crates.io from the start.** Clean versions for SDK users, but
   every editor change that needs an engine change waits for a release, and
   publishing a patched Skia binding that builds from source is its own
   project. A later step, once outside users exist.

## Consequences

- Good: SDKs and the cloud depend on a 15-crate repository. Engine CI runs
  without Zed. The engine can carry its own licence, versions, changelog and
  outside contributors. The boundary becomes a published API.
- Bad: a change that crosses the boundary takes two merges and a pin bump.
  The CE.SDK work (#160 to #166) would have needed about six of them. Build
  settings (lints, profiles, the Skia opt-level) must be kept in step by
  hand in two workspaces.
- Follow-up: rename the cloud worker crate (today `fanta-engine`) so the name
  can belong to the repository and an optional facade crate; recommended
  `fanta-worker`. Its deployment in `fanta-backend` changes with it.

## Confirmation

The engine repository's CI, the editor's render parity test against the
pinned engine, and the layer check (0023) until the move happens.

## References

- Spec [0002](../../specs/0002-fanta-engine-repository/spec.md)
- Commits `e929627ef1` (2026-07-10) and `5c15d4bdb0` (2026-09-08), #132
