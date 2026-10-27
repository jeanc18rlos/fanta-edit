---
status: accepted
date: 2026-10-24
recorded: 2026-10-27
decision-makers: Jean Rojas
tags: [engine, api, sdk]
---

# Offer a Fanta-native design API, built clean-room

## Context

IMG.LY's CreativeEditor SDK (CE.SDK) is a headless design engine for
templating, automation and export. On 2026-10-24 the founder asked how
Fanta's engine could match it.

## Decision

Build a Fanta-native programmatic API, `fanta-api`'s `DesignSession`, with
CE.SDK's capabilities: node lifecycle, typed properties with reflection,
change events, explicit undo steps, text variables, placeholders, template
scopes and roles, filling templates from data, and batch export. It runs over
`fanta-headless`, and language bindings wrap it.

Work clean-room: use only CE.SDK's public documentation; never decompile its
binaries or copy its shaders, LUTs, fonts, sample code or names. Fanta keeps
its own names, ids and property paths. A compatibility adapter, if one is
ever built, is a separate optional layer.

## Options considered

1. **A Fanta-native API with equivalent capabilities** (chosen).
2. **A CE.SDK-compatible API.** It would copy their names and design, which
   the clean-room rule forbids, and tie Fanta to another product's choices.

## Consequences

- Good: a programmatic surface for SDKs and the cloud, shipped in phases
  (#160 to #166 on 2026-10-25 and 26).
- Bad: its calls and errors are now a public surface. Permission checks are
  easy to miss, and template data is untrusted input; the crate rules cover
  both.

## Confirmation

The rules in [crates/fanta-api/AGENTS.md](../../crates/fanta-api/AGENTS.md),
the exhaustive `Operation` match in `events::node_events`, and the crate's
tests. The parity matrix and roadmap are in
[engine-parity.md](../fanta/engine-parity.md).

## References

- #160, #161, #162, #163, #164, #165, #166
