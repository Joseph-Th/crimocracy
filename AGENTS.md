# Agent Guide

This file is the execution card for repository work. Detailed ownership and mutation rules live in [ARCHITECTURE.md](ARCHITECTURE.md); test and gameplay-harness rules live in [TESTING.md](TESTING.md).

## Start here

1. Read [`../AGENTS.md`](../AGENTS.md) and the applicable portfolio standards.
2. Inspect Git status and preserve unrelated work.
3. Read [STATUS.md](STATUS.md) for implemented capability and explicit exclusions.
4. Use [README.md](README.md) and [ARCHITECTURE.md](ARCHITECTURE.md) to find the owning subsystem and canonical operation.
5. Read the owner module's `//!` contract and focused tests before editing.
6. Read [TESTING.md](TESTING.md) before changing tests, harness behavior, persistence, or verification tooling.
7. Use [GAME_DESIGN.md](GAME_DESIGN.md) only for product intent, never as proof of implementation.

If ownership, current scope, or the proving test is not discoverable, repair the owning documentation as part of the change.

## Authority map

| Question | Authority |
|---|---|
| How should repository work proceed? | `AGENTS.md` |
| Who owns state and how may it change? | `ARCHITECTURE.md` |
| What is implemented or intentionally absent? | `STATUS.md` |
| How are tests and gameplay evidence selected and interpreted? | `TESTING.md` |
| What player experience is intended? | `GAME_DESIGN.md` |
| How do I run the project and local gate? | `README.md` |
| What behavior is executable now? | Owning source module and tests |

Resolve contradictions in the owning contract instead of choosing the convenient description. Git history is historical evidence, not current authority.

## Non-negotiable change rules

- Consequential state has one owner and changes through the canonical production path described in `ARCHITECTURE.md`.
- Do not add mutation shortcuts for tests, examples, adapters, importers, migrations, or administrative tools.
- Validate fallible multi-record work before mutation; rejected operations preserve authoritative state unless the contract explicitly owns a failure record.
- Preserve deterministic ordering and state-owned randomness. Do not introduce result-affecting wall time, filesystem order, thread scheduling, or ambient entropy.
- New future-affecting runtime state must be serializable and covered by invariant/load/continuation checks where applicable.
- Project-owned enum handling is exhaustive; consequential fields remain private to their owner; new fallible operations use typed errors.
- External effects stay behind explicit adapter boundaries.
- Delete superseded internal paths instead of preserving history as production compatibility.
- Keep current documentation forward-facing. Do not add implementation diaries, incident narratives, or stale compatibility descriptions.
- Repository verification is local. Do not create or depend on GitHub Actions workflows.

## Change route

Use the source map in [ARCHITECTURE.md](ARCHITECTURE.md). For every change, identify:

1. the owner and canonical operation;
2. affected invariants, indexes, persistence, and observation boundaries;
3. the narrowest focused test;
4. the broader completion lane in [TESTING.md](TESTING.md);
5. the one document that owns any changed contract.

## Completion

Run the applicable focused checks while iterating, then the local completion gate documented in [TESTING.md](TESTING.md). Before commit, confirm that ownership, deterministic behavior, persistence, invariants, adapters, tests, and documentation remain coherent and that the diff contains no unrelated or generated files.
