---
status: accepted
date: 2026-11-12
decision-makers: Jean Rojas
tags: [licensing, engine, sdk]
---

# Choose the engine and SDK licence

## Context

- The editor is GPL-3.0-or-later because it derives from Zed
  ([0002](0002-build-fanta-on-a-fork-of-zed.md)). That does not change.
- The engine crates are Fanta's own code, ported from the first app. Their
  licence fields disagree: thirteen say `GPL-3.0`, a deprecated SPDX
  identifier that means GPL-3.0-only, and five say `GPL-3.0-or-later`
  (`fanta-design-ops`, `fanta-headless`, `fanta-api`, `fanta-mcp`,
  `fanta-wasm-engine`).
- Fanta-JS's packages, `fanta-runtime` and `fanta-sdk` (0.2.0 in that
  repository), declare GPL-3.0-only. GPL code cannot be embedded in a
  closed-source app, which is
  how runtimes such as Rive's (MIT) are used. The 2026-10-21 runtime review
  called this the biggest non-code blocker.
- In `fanta-ui`, GPUI itself is Apache-2.0 and the rest of the workspace is
  GPL-3.0.

The engine repository ([0024](0024-move-the-engine-into-its-own-repository.md))
and the public SDK make this the moment to choose.

## Decision

Option 2, decided by the founder on 2026-11-12: the engine and the SDK are
Apache-2.0, and the editor stays GPL-3.0-or-later.

- "Engine" is the engine group of
  [`layers.toml`](../architecture/layers.toml) and the files that move with
  it (spec 0002). "SDK" is `fanta-sdk` and the `fanta-runtime` packages.
- Before any licence field changes, check that no engine file was copied
  from Zed, GPL or other third-party sources.
- Counsel confirms the choice before the engine repository becomes public.

## Options considered

1. **Keep GPL-3.0 everywhere.** Simplest, and closed forks are impossible.
   SDK adoption stays limited to GPL projects and to server-side use.
2. **Apache-2.0 for the engine and SDK; the editor stays GPL.** The widest
   adoption, close to how design runtimes are licensed. Competitors may embed
   the engine, and outside contributions need a DCO sign-off.
3. **GPL-3.0 plus a commercial licence** (dual licensing). A revenue path
   from companies that embed the SDK; every outside contribution needs a CLA
   so Fanta can keep offering both licences.
4. **MPL-2.0.** File-level copyleft: apps may embed the engine, and changes to
   engine files must be shared.
5. **A source-available licence that turns into Apache-2.0 after a delay**
   (BSL or FSL). Protects the business for a few years, but is not open source
   during that time.

Recommendation: option 2 if the SDK is meant to spread through developers'
apps, or option 3 if SDK licences are meant to pay. Option 1 gives up most of
the SDK plan.

## Consequences

- Good: closed-source apps may embed the runtime and the SDK, which the
  2026-10-21 review named as the biggest non-code blocker. The engine
  repository starts with one licence in every crate.
- Bad: competitors may embed the engine and keep their changes. Code can
  no longer be copied from the editor or from Zed into the engine: the
  boundary is now a licence boundary as well, and GPUI-free logic that
  leaves `fig_viewer` for the engine (ADR 0024) must be Fanta's own.
- Follow-up: the provenance sweep and the `license = "Apache-2.0"` fields
  (spec 0002 T2); a `LICENSE` file and SPDX identifiers in the engine
  repository (T4); the same change in the Fanta-JS packages (T7); a DCO
  sign-off for outside contributions once the repository is public;
  [NOTICE.md](../../NOTICE.md) updated with the fields. Spec 0009 T8
  publishes the checker under this licence once T2 has merged.
- Until T2 merges, the crates remain GPL as their manifests say.

## Confirmation

`script/check-licenses` and the package metadata of each published crate and
npm package.

## References

- [NOTICE.md](../../NOTICE.md); runtime size and licence review of 2026-10-21 (session notes)
