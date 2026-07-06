# Fanta Edit adoption plan

Fanta Edit is an unofficial fork of Zed. The first milestone is a clean Fanta
baseline: the app should build, launch, store data, identify itself, and make
network/service decisions as Fanta before any design-canvas business logic is
added.

## Product thesis

Fanta Edit should become a universal text editor where design state, code,
agents, git history, and visual canvas interactions share one editable model.
Canvas edits must produce reviewable code changes, and code changes must be able
to update the canvas state. Agent workflows should operate through the same
paths a user or IDE would use, so visual changes remain explainable as source
changes.

## Fork contract

Keep upstream-compatible foundations whenever possible:

- GPUI, editor, workspace, project, git, diagnostics, terminal, settings, and
  extension systems.
- Agent, chat, and ACP infrastructure where it can run without Zed-hosted
  services.
- Zed's license structure and third-party notices.
- A history shape that can continue merging upstream Zed.

Change Fanta-owned product surfaces:

- Product names, bundle identifiers, icons, URL schemes, CLI identity, and
  default paths.
- Hosted service URLs, telemetry, crash upload, auto-update, release, and
  deployment ownership.
- Defaults that would send users to Zed-owned cloud services.
- Documentation, onboarding, and prompts that describe Fanta's canvas/code-sync
  model.

## Phase 1: clean brand baseline

- Finish app identity across macOS, Windows, Linux, Flatpak, Snap, CLI, window
  title, about dialog, notifications, and docs.
- Replace app icons and document icons.
- Decide whether project-local settings stay compatible with `.zed/` or move to
  `.fanta-edit/`.
- Keep internal compatibility aliases where renaming protocols would create a
  large upstream merge burden.

## Phase 2: service safety baseline

- Disable auto-update by default until Fanta owns signed releases.
- Disable telemetry and crash upload by default until Fanta owns privacy docs
  and ingestion endpoints.
- Point `server_url` away from `zed.dev`.
- Avoid a hosted `zed.dev` language-model provider as the default model.
- Disable or gate Zed-owned GitHub Actions, Cloudflare workers, docs deploys,
  release deploys, and collaboration deploys.

Operational details for the disabled services live in
[`docs/fanta/disabled-services-binnacle.md`](./docs/fanta/disabled-services-binnacle.md).

## Phase 3: release baseline

Required resources before public binaries:

- Apple Developer account for macOS signing and notarization.
- Windows signing certificate if Windows releases are shipped.
- Fanta-owned domain and release/update endpoint, or a GitHub Releases based
  update strategy.
- Fanta privacy policy, terms, and subprocessors page if any hosted service is
  enabled.
- Sentry or another crash-reporting project, if crash upload is enabled.
- Release signing secrets stored in CI.

## Phase 4: Fanta feature substrate

Start with one Fanta-owned component rather than many small crates. Likely
entry points to study before implementation:

- `crates/workspace` for panels and panes.
- `crates/editor` for text-buffer edits and selections.
- `crates/project` for project files and worktrees.
- `crates/agent`, `crates/agent_ui`, and `crates/acp_thread` for agent access.
- `crates/svg_preview` and `crates/component_preview` as precedents for
  code-backed visual workflows.

The first feature proof should be a minimal round trip:

1. Load a simple code-backed design document.
2. Render it in a dev-only canvas panel.
3. Apply a canvas edit that writes deterministic source changes.
4. Apply a source edit that updates canvas state.
5. Verify undo, diff, and git output remain understandable.

## Working rules

- Prefer upstream-compatible seams over broad rewrites.
- Do not add Fanta business logic until the app is safe to identify and operate
  as Fanta.
- Do not enable Zed-hosted services by default.
- Keep `.rules` high-signal; propose new rules in PR descriptions first unless
  the team explicitly asks to edit them.
- Use `./script/clippy` instead of `cargo clippy`.
