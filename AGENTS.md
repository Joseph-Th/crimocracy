# Agent Guide

Execution rules for this repository. Ownership and mutation contracts are in [`ARCHITECTURE.md`](ARCHITECTURE.md); verification and harness rules are in [`TESTING.md`](TESTING.md); implemented scope is in [`STATUS.md`](STATUS.md).

## Cold start

For a cold agent or new checkout:

1. If this repository lives inside a portfolio workspace, read the workspace `../AGENTS.md` and applicable portfolio standards first.
2. Read this file for repository execution rules.
3. Read [`STATUS.md`](STATUS.md) for what exists and what is explicitly excluded.
4. Read [`README.md`](README.md) for orientation and commands.
5. Read [`ARCHITECTURE.md`](ARCHITECTURE.md) to find state owners and canonical mutation paths.
6. Read the `//!` header of the owning `src/` module and its focused tests before editing.
7. Read [`TESTING.md`](TESTING.md) before changing tests, persistence, or harness behavior.
8. Read [`GAME_DESIGN.md`](GAME_DESIGN.md) only for product-intent questions.

To watch the implemented game behave end to end, run `cargo harness-full --samples 8`: it plays matched strategy branches through production paths and narrates the causal story from player-visible state only. Evidence rules are in [`TESTING.md`](TESTING.md).

## Authority map

| Question | Authority |
| --- | --- |
| Repository and collaboration rules | Workspace `AGENTS.md` (if present) |
| Project execution rules | This file |
| State ownership and mutation | [`ARCHITECTURE.md`](ARCHITECTURE.md) |
| Implemented scope and exclusions | [`STATUS.md`](STATUS.md) |
| Tests and harness evidence | [`TESTING.md`](TESTING.md) |
| Product intent | [`GAME_DESIGN.md`](GAME_DESIGN.md) |
| Commands and local gate | [`README.md`](README.md) and [`TESTING.md`](TESTING.md) |
| Executable behavior | Owning `src/` module and its focused tests |

If authorities conflict, the owning contract and implementation win. Do not keep stale wording.

## Non-negotiable rules

- One owner per consequential state field; mutate only through the canonical production path.
- Tests, examples, adapters, importers, and tools use owner methods. No bypasses or mutation shortcuts.
- Validate fallible multi-record operations before mutation. Rejected operations leave authoritative state unchanged unless the contract explicitly records a failure.
- Keep ordering and randomness deterministic: ordered collections or explicit stable sorting with tie-breakers, state-owned RNG only.
- Persist every future-affecting runtime value; cover with invariant, load, and continuation checks where applicable.
- Handle project-owned enums exhaustively; use typed error enums for new fallible operations.
- Keep external effects (filesystem, network, UI, process) behind explicit adapter boundaries.
- Delete superseded paths. Do not keep historical shims.
- Keep documentation concise, current, and forward-facing. No implementation diaries.
- Verification is local. Do not add or depend on GitHub Actions.

## Change route

For every change, identify:

1. Owner and canonical operation (`ARCHITECTURE.md` source map).
2. Invariants, indexes, persistence, and observation boundaries affected.
3. Narrowest focused proof (`TESTING.md` test selection).
4. Broader completion lane (`TESTING.md` verification).
5. One authority document for any changed contract.

If any of the above is not discoverable, repair the owning documentation as part of the change.

## Completion

Use focused checks while editing when they shorten feedback or isolate a failure. For completion, run the smallest lane that covers the changed surface; if the implementation is already ready for that lane, go directly to it instead of forcing a focused build first:

- `cargo check-fast` / `cargo test-focused <filter>` while editing
- `.\scripts\verify.cmd -Fast` for ordinary library work
- `.\scripts\verify.cmd -Fast -Harness` when the harness surface changes
- `.\scripts\verify.cmd` only for persistence/invariant/cross-domain work, verification infrastructure, or an explicit broad checkpoint

Before handoff, confirm ownership, determinism, persistence, invariants, adapters, tests, documentation, and worktree scope remain coherent.
