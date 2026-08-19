# Agent Guide

This is the repository execution card. Detailed ownership and mutation rules live in [`ARCHITECTURE.md`](ARCHITECTURE.md); test and gameplay-harness rules live in [`TESTING.md`](TESTING.md).

## Cold start

1. Read the workspace [`../AGENTS.md`](../AGENTS.md) and the applicable portfolio standards.
2. Inspect `git status` and preserve unrelated changes.
3. Read [`STATUS.md`](STATUS.md) for current capability and exclusions.
4. Use [`README.md`](README.md) for orientation and commands.
5. Use [`ARCHITECTURE.md`](ARCHITECTURE.md) to identify the owner and canonical operation.
6. Read the owner module's `//!` contract and focused tests before editing.
7. Read [`TESTING.md`](TESTING.md) before changing tests, persistence, harness behavior, or verification tooling.
8. Read [`GAME_DESIGN.md`](GAME_DESIGN.md) only when product intent is relevant.

This project uses the Universal, Stateful Application, Deterministic System, and Automated Behavior Evaluation portfolio profiles.

## Authority map

| Question | Authority |
| --- | --- |
| Repository and collaboration rules | workspace `AGENTS.md` |
| Project execution rules | this file |
| State ownership and mutation | `ARCHITECTURE.md` |
| Implemented scope | `STATUS.md` |
| Tests and gameplay evidence | `TESTING.md` |
| Product intent | `GAME_DESIGN.md` |
| Commands and local gate | `README.md` |
| Executable behavior | owning source module and focused tests |

Resolve contradictions in the owning contract and implementation. Do not preserve stale wording as compatibility documentation.

## Non-negotiable rules

- Consequential state has one owner and changes through its canonical production path.
- Tests, examples, adapters, importers, migrations, and administrative tools do not bypass owner methods or add mutation shortcuts.
- Validate fallible multi-record work before mutation; rejected operations preserve authoritative state unless the contract explicitly records a failure.
- Keep result-affecting ordering and randomness deterministic and state-owned.
- Persist every new future-affecting runtime value and cover it with invariant, load, and continuation checks where applicable.
- Handle project-owned enums exhaustively; use typed errors for new fallible operations.
- Keep external effects behind explicit adapter boundaries.
- Delete superseded internal paths instead of retaining historical production compatibility shims.
- Keep documentation current, concise, and forward-facing. Do not add implementation diaries or incident narratives.
- Verification is local. Do not create or depend on GitHub Actions workflows.

## Change route

For every change, identify:

1. owner and canonical operation;
2. affected invariants, indexes, persistence, and observation boundaries;
3. narrowest focused proof;
4. broader completion lane from [`TESTING.md`](TESTING.md);
5. one authority document for any changed contract.

If ownership, scope, or the proving test is not discoverable, repair the owning documentation as part of the change.

## Completion

Run focused checks while iterating, then the local completion gate. Before handoff, confirm ownership, deterministic behavior, persistence, invariants, adapters, tests, documentation, and worktree scope remain coherent.
