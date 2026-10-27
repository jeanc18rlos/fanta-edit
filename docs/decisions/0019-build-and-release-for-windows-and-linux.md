---
status: accepted
date: 2026-10-21
recorded: 2026-10-27
decision-makers: Jean Rojas
tags: [platform, distribution]
---

# Build and release for Windows and Linux

## Context

Fanta shipped for Apple Silicon Macs only. Zed's shell already runs on
Windows and Linux, and the cloud engine runs on Linux. #149 made Fanta build
and pass its tests on Windows in CI on 2026-10-20.

## Decision

On 2026-10-21 the founder decided that Linux and Windows get public downloads
with automatic updates, and that Windows ships unsigned first. Windows is
built and tested by its own CI workflow (`.github/workflows/windows.yml`).

## Consequences

- Good: Fanta reaches users beyond macOS, and the platform code shares one
  shell.
- Bad: three release pipelines to keep working. The Windows workflow is the
  only Windows check, with no cross-check from macOS. An unsigned installer
  draws SmartScreen warnings until it is signed. Platform-specific failures
  (renderer crashes, path and junction behavior) need a Windows machine or CI
  to reproduce.
- Status on 2026-10-27: the Linux release (#154) and the Windows installer
  (#156) are open pull requests.

## Confirmation

The `Windows` workflow on every pull request, and
[windows-development.md](../fanta/windows-development.md) for local builds.

## References

- #149 (2026-10-20), #151; founder decisions recorded in session notes on 2026-10-21
