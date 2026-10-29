---
status: accepted
date: 2026-07-07
recorded: 2026-10-27
decision-makers: Jean Rojas
tags: [platform, licensing]
---

# Build Fanta on a fork of Zed

## Context

The first Fanta app (`jeanc18rlos/fanta`, "Fantaisa") was a native Rust
canvas with a winit and egui shell. A memo in that repository dated
2026-06-27 (`docs/ui-shell-decision-memo.md`) kept egui for the shell and
deferred GPUI until a spike proved it, while calling GPUI the right candidate
for a more polished shell. Ten days later Fanta started over on Zed, which is
built on GPUI and already has a workspace, docks, a project tree, Git, settings
and an agent panel. Beyond these notes, the reasons for the switch were not
written down.

## Decision

Use Zed 1.11.0 as the application shell. The baseline commit `8b08b0607b`
(4,175 files) imports it unchanged. Fanta's canvas is a Zed workspace item
with its own panels (`crates/fig_viewer`, first commit `3c74753cda`). The
engine crates are ported from the first app with their APIs intact. When
touching inherited code, prefer seams that stay compatible with upstream over
rewrites.

## Options considered

1. **Fork Zed** (chosen). A mature GPUI shell, workspace, project model, Git
   integration and agent tooling from the first day.
2. **Keep the egui shell.** It worked, but its polish was the open problem the
   June memo described.
3. **Build a new GPUI shell.** The memo's spike path; slower to a product.

## Consequences

- Good: Fanta shipped a public alpha on 2026-10-12 with a shell it did not
  have to build. The agent panel, MCP support and Git came with it.
- Bad: the app is GPL-3.0-or-later like Zed, so distributed binaries need
  their corresponding source published. Zed's code makes up the 128-crate
  shell layer, the app links about 1,400 packages, and builds are slow.
  Hosted services had to be switched off or redirected
  ([binnacle](../fanta/disabled-services-binnacle.md)), branding had to be
  removed (#135, 2026-10-15), and the inherited `v1.0.0` tag forced the
  `alpha-<version>-<date>` release tags.
- Follow-up: keep the engine independent of the shell
  ([0003](0003-keep-skia-out-of-the-document-model.md),
  [0023](0023-enforce-layered-crate-boundaries.md)). Of the 237 inherited
  crates, 111 have since been removed.

## Confirmation

`NOTICE.md` and `LICENSE-GPL` record the licence. The shell layer in
[`layers.toml`](../architecture/layers.toml) keeps inherited crates apart from
Fanta's.

## References

- Commits `8b08b0607b` "Initialize Fanta Edit baseline" and `3c74753cda` "Add Fanta fig viewer foundation", both 2026-07-07
- `jeanc18rlos/fanta`: `docs/ui-shell-decision-memo.md` (2026-06-27)
- [NOTICE.md](../../NOTICE.md), #135
