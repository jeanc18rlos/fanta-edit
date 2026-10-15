---
name: fanta-release
description: Build, verify and publish a Fanta macOS release - the "Build macOS release" workflow, signing and notarization, version tags, the GitHub prerelease, crash symbols, and every place that pins the download. Use when preparing, building, checking or publishing a Fanta release or DMG.
---

# Releasing Fanta for macOS

Fanta ships as a notarized Developer ID DMG for Apple Silicon, built in GitHub Actions. Publishing a release, changing a download page or announcing anything needs the user's explicit go-ahead for that step.

## Build

Workflow "Build macOS release" (`.github/workflows/release-macos.yml`):

- `workflow_dispatch` inputs: `distribution` (`developer-id` or `app-store`), `signed` (sign and notarize; needs the Apple secrets), `build_diagnostics`.
- Notarization is required whenever the workflow runs on a tag, even with `signed=false`. Dispatch unsigned test builds on a branch.
- A pushed tag must equal `v<version>` from `crates/zed/Cargo.toml`, and creates a draft GitHub release.
- Artifacts: `fanta-macos-aarch64-notarized` (or `fanta-macos-aarch64-test-only`) with `Fanta-aarch64.dmg` and `SHA256SUMS.txt`; crash symbols `fanta-symbols-aarch64-<commit>` (`fanta.dwarf`, kept 90 days); DMG verification evidence.

Locally, `./script/bundle-mac aarch64-apple-darwin` builds the DMG and `fanta.dwarf` (`-i` installs it into `/Applications`; `-s` builds the Mac App Store variant instead). Local notarization needs `FANTA_REQUIRE_NOTARIZATION=true` and either a notarytool keychain profile (`APPLE_NOTARIZATION_KEYCHAIN_PROFILE`) or the API key variables. A local bundle does not establish that the public installer works.

## Verify

- `python3 script/verify-macos-dmg <dmg> <app>` (the workflow runs it), `python3 script/test-macos-release`, and stage `build` of `script/verify-fanta-release`.
- Run signature checks (`codesign`, `pkgutil --check-signature`, `stapler`) with access to the login keychain. Inside a sandbox they can falsely report a missing signing certificate.
- Check the downloaded asset with `shasum -a 256 -c SHA256SUMS.txt`.

## Publish a public alpha

Public alphas are GitHub prereleases tagged `alpha-<version>-<YYYYMMDD>`. The first, `alpha-1.0.0-20261012`, targets the commit built by a `workflow_dispatch` run on a branch. Create the prerelease from the verified notarized artifact with `Fanta-aarch64.dmg` and `SHA256SUMS.txt`, targeting the built commit.

Then update every place that pins the release tag, and check that each download link resolves:

- `docs/alpha/PUBLIC_ALPHA.md` in this repository;
- the download page (`jeanc18rlos/fantaisa-landing`, `src/app/download/page.tsx`);
- the docs site (`jeanc18rlos/fanta-docs`, `content/docs/getting-started/install.mdx`).

Keep the release's `fanta-symbols-aarch64-<commit>` artifact reachable for crash symbolication (see the `crash-investigation` skill); it expires after 90 days.
