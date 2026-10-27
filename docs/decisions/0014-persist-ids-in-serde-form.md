---
status: accepted
date: 2026-10-07
recorded: 2026-10-27
decision-makers: Jean Rojas
tags: [format, contracts]
---

# Persist ids in serde form and prefix them only at the edges

## Context

Every id is a ULID with two spellings. Its serde form is the bare ULID; its
`Display` and `FromStr` forms add a type prefix (`n_`, `c_`, `a_`, …). Code
that built persisted keys with `to_string()` wrote prefixed keys that readers
could not match. Four pull requests hit this: #85, #102, #110 and #128.

## Decision

Persisted maps (scene nodes, component maps, sidecars) key ids by their serde
form. Derive those keys through serde, with `serde_json::to_value(id)` or
`json_key` and `id_from_key` in `fanta-format`, never with `to_string()` or
`from_str`. File names, logs, agents, MCP input and the design API use the
prefixed form, and convert only where project files are read or written.

One known exception: `pages/<slug>/page.json` stores the page id in the
prefixed form (`"id": "n_…"`), as the layout notes in
`crates/fanta-format/src/project/layout.rs` document.

## Consequences

- Good: one rule that explains every id seen in files and in tools.
- Bad: the two spellings remain a trap for new code, and the `page.json`
  exception has to be remembered.

## Confirmation

The root rules list this trap with its four pull requests, and
`crates/fanta-api/AGENTS.md` repeats it for bindings. Format round-trip tests
catch a mismatched key when the affected path is covered.

## References

- #85, #102, #110, #128; recorded in `.rules` by #137 (2026-10-15)
