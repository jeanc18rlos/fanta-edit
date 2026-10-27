# Specs

A spec says what we build, why, and how we will know it works, before
anyone writes the code. The [process guide](../docs/process/spec-driven-development.md)
explains when a change needs one and how specs move from draft to
implemented.

## Layout

| File | Holds | Approved at |
| --- | --- | --- |
| `spec.md` | Problem, goals, non-goals, numbered requirements, acceptance criteria, open questions | Gate 1: the founder accepts the what |
| `design.md` | Approach, interfaces and data, alternatives, risks, verification | Gate 2: the founder accepts the how and its ADRs |
| `tasks.md` | Ordered tasks, one pull request each, ticked as they merge | Gate 3: each task's pull request |

## Writing one

```bash
script/architecture new spec "Fill templates from a CSV file"
```

The command creates the next numbered folder from [`templates/`](templates)
and adds it to the index below.

| Status | Meaning |
| --- | --- |
| `draft` | Written, not yet approved |
| `accepted` | The founder approved the spec and its design; work may start |
| `in-progress` | At least one task has merged |
| `implemented` | Every task merged and every acceptance criterion holds |
| `abandoned` | Dropped; the spec says why |

An accepted spec needs `design.md` and `tasks.md`, and an implemented spec
has no open tasks. `script/architecture index --check` enforces both in CI.

## Index

<!-- specs:start -->
| Spec | Title | Status | Date | Decisions |
| --- | --- | --- | --- | --- |
| [0001](0001-architecture-map-and-boundaries/spec.md) | Architecture map and boundaries | draft | 2026-10-27 | 0001, 0023 |
| [0002](0002-fanta-engine-repository/spec.md) | The fanta-engine repository | draft | 2026-10-27 | 0023, 0024, 0025, 0026 |
| [0003](0003-github-organization/spec.md) | One GitHub organization | draft | 2026-10-27 | 0025 |
<!-- specs:end -->
