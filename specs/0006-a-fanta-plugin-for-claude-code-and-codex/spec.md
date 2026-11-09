---
status: draft
date: 2026-11-09
owner: Jean Rojas
decisions: []
repositories: [fanta-editor, fanta-plugin]
---

# A Fanta plugin for Claude Code and Codex

## Problem

Fanta already speaks to agents: the live MCP server exposes 14 tools over a
local socket ([ADR 0007](../../docs/decisions/0007-give-agents-a-live-design-seam-and-mcp-server.md),
[agent-quickstart.md](../../docs/fanta/agent-quickstart.md)). But reaching it
is manual: open the app, open the right project, click **Connect Codex or
Claude Code**, copy a per-project registration command or `config.toml`
block, restart the agent. Every project repeats the dance, and nothing helps
when the app is closed, the wrong project is focused, or versions drift.

[codex-openpencil](https://github.com/ZSeven-W/codex-openpencil) sets the
bar the founder pointed to on 2026-11-09: one prompt installs a plugin that
registers the MCP bridge, manages the editor's lifecycle, bundles setup
diagnostics and a design-workflow skill, and previews the canvas inside the
conversation. Fanta has the harder half — the server and the design ops —
and lacks the packaging.

## Goals

- Installing once per machine, with one command or one prompt, makes every
  project's agent able to reach Fanta — no per-project copying from the app.
- The agent can get from "nothing running" to "connected to this project"
  by itself: attach to the running instance, or launch the installed app on
  the project folder and wait for the server.
- Setup failures diagnose themselves: a bundled skill checks the install,
  the bridge, the discovery file and version compatibility, and says what to
  fix.
- A bundled design-workflow skill makes agents read `get_guidelines` and
  inspect before writing, so the first experience matches
  [agent-quickstart.md](../../docs/fanta/agent-quickstart.md) without the
  user pasting instructions.
- The same plugin behavior ships for Claude Code and for Codex, on macOS,
  Windows and Linux.
- The user can see what the agent did without switching apps: screenshots
  in the conversation everywhere, and a live browser-panel preview where the
  client offers one.

## Non-goals

- New or changed MCP tools. The 14-tool surface stays as it is; gaps it has
  are their own specs.
- Changes to the hosted `api.fantaisa.net` MCP service, billing or
  credentials.
- An embedded editing canvas inside the agent client (openpencil's MCP Apps
  panel). Preview is read-only in this spec.
- Replacing the in-app **Connect** flow; it remains for users who prefer it.

## Requirements

- **R1.** A Claude Code user must be able to install the plugin with one
  marketplace command, and a Codex user with one documented prompt or
  command; both register the MCP server and the bundled skills with no
  per-project steps.
- **R2.** The plugin must connect through the existing stdio bridge and
  discovery file to the running instance, and must keep its failure
  honesty: no running app or disabled server fails visibly, never with a
  fabricated connection.
- **R3.** When no instance is running and the app is installed, the plugin
  must be able to launch it on the agent's project folder and wait for the
  server to accept. It must never download or install the app silently;
  when the app is missing it points at the platform's download
  ([ADR 0019](../../docs/decisions/0019-build-and-release-for-windows-and-linux.md)).
- **R4.** The plugin must bundle at least two skills: setup diagnostics
  (install, bridge, discovery, versions — each check with a fix) and the
  design workflow (guidelines first, inspect before writing, validate
  before saving).
- **R5.** The plugin must declare the app versions it speaks. A mismatch
  reports both versions and the upgrade path instead of failing obscurely.
- **R6.** The local transport must work on all three platforms. The Unix
  socket the bridge uses today needs a Windows answer (named pipe or
  loopback) decided at design time.
- **R7.** The plugin must live in its own repository with its own releases,
  pinned to the app versions it supports, and must be testable against a
  checked-out fanta-edit without a release.
- **R8.** Preview: `get_screenshot` must work through the plugin on all
  platforms; where the client offers a browser panel, the plugin should
  serve the project's web projection
  ([spec 0005](../0005-view-a-project-as-plain-web-files/spec.md)) as a
  live view. The projection is a preview dependency, not a prerequisite:
  screenshots alone satisfy this requirement until spec 0005 ships.

## Acceptance criteria

- **A1 (R1, R2).** On a machine with Fanta installed and a project open,
  installing the plugin and asking the agent for the editor state returns
  the project root and selection, with no manual MCP registration.
- **A2 (R3).** With Fanta installed but closed, the agent reaches a
  connected state from one request, and the launched app shows the agent's
  project.
- **A3 (R2).** With Fanta not running and launch declined, the tools fail
  with a message naming the app as absent — no silent retry loop, no
  pretend connection.
- **A4 (R4).** Breaking the setup three ways (app missing, server disabled,
  stale discovery file) and running the diagnostics skill names each broken
  step and its fix.
- **A5 (R5).** A deliberately mismatched plugin and app report both
  versions and the upgrade instruction.
- **A6 (R6).** A1 passes on macOS, Windows and Linux, in CI or on recorded
  hardware runs.
- **A7 (R1).** The Codex install path reaches A1 on a machine without
  Claude Code.

## Open questions

- **Repository and name.** `fanta-plugin`? One repo for both agent targets
  or one per target? Feeds the organization layout
  ([spec 0003](../0003-github-organization/spec.md)).
- **An app-less mode.** openpencil downloads a headless runtime instead of
  requiring an installed editor. Should the plugin ever drive
  `fanta-headless` plus the web projection with no app installed, or is the
  app required forever? Shapes R3 and the download story.
- **Marketplace submissions.** Do we submit to the Claude Code plugin
  marketplace and Codex's directory at launch, or distribute from our
  repository first?
- **Which client versions are the floor?** Claude Code and Codex plugin
  formats are both young; the design must pin minimum versions.

## Changes after acceptance

- None.
