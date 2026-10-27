---
status: accepted
date: 2026-10-21
recorded: 2026-10-27
decision-makers: Jean Rojas
tags: [cloud, contracts]
---

# Run cloud work in a stateless engine worker behind a frozen contract

## Context

Cloud AI Review, Ultra review and cloud generation are paid features of the
v2 cutover. The backend (`fanta-backend`, TypeScript) owns accounts, billing
and GitHub access; the engine is Rust.

## Decision

The founder's decisions of 2026-10-21:

- Headless work runs as a Linux Cloud Run worker on the Rust engine (the
  `fanta-engine` crate), not in the app.
- Review screenshots are rendered in the cloud.
- The project source is the user's GitHub repository at a commit, read
  through the GitHub App, plus an overlay of unpushed files.

The worker is stateless: it reads a project, runs an ordered list of steps
(`{"step": …}`) on one loaded document and answers. A failed step ends the
run, still inside a 200 response. It never writes to GitHub; the v2 API does.
Only v2's service account may invoke it, and every call also carries an HMAC
header (`X-Fanta-Internal`). The interfaces are frozen in
[cloud-engine-contracts.md](../cloud-engine-contracts.md), which both
repositories keep a copy of, and they change only together with the code on
both sides, in a pair of pull requests.

## Consequences

- Good: one engine for the desktop, the runtime and the cloud. A stateless
  worker scales by instances, and GitHub stays the source of truth.
- Bad: two copies of the contract are kept in sync by hand, and every
  contract change is a pair of pull requests across repositories (#164 with
  `fanta-backend#48`).

## Confirmation

The wire types in `crates/fanta-engine/src/fanta_engine.rs` and their tests,
and review of both copies of the contract in each paired pull request.

## References

- [cloud-engine-contracts.md](../cloud-engine-contracts.md), #155, #164
