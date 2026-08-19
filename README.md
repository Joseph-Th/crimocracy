# Crimocracy

Crimocracy is a deterministic Rust simulation foundation for a systemic crime-organization strategy game. The player acts through people, information, plans, policies, relationships, delegated authority, enterprises, and institutions.

## Cold-start route

Read only the document that answers the question you have:

1. [`AGENTS.md`](AGENTS.md) — repository rules, change routing, and completion requirements.
2. [`STATUS.md`](STATUS.md) — implemented capability and current exclusions.
3. [`ARCHITECTURE.md`](ARCHITECTURE.md) — state ownership, mutation paths, determinism, persistence, and invariants.
4. [`TESTING.md`](TESTING.md) — focused tests, harness evidence, and verification semantics.
5. [`GAME_DESIGN.md`](GAME_DESIGN.md) — player experience and product intent.

The source module and its focused tests are the authority for executable behavior. Design intent does not prove implementation, and status does not define product intent.

## Authority map

| Question | Authority |
| --- | --- |
| How should repository work proceed? | [`AGENTS.md`](AGENTS.md) |
| Who owns state and how may it change? | [`ARCHITECTURE.md`](ARCHITECTURE.md) |
| What exists or is excluded? | [`STATUS.md`](STATUS.md) |
| How is behavior proved? | [`TESTING.md`](TESTING.md) |
| What experience is intended? | [`GAME_DESIGN.md`](GAME_DESIGN.md) |
| What behavior is executable? | Owning `src/` module and focused tests |

If two authorities disagree about the same contract, repair the owning document and implementation. Do not resolve the conflict by choosing the more convenient wording.

## Implemented shape

The library has three important layers:

- `Registry` contains immutable, Rust-authored definitions.
- `AppState` contains serializable mutable campaign state, typed IDs, clocks, and state-owned random streams.
- Domain systems validate requests, commit mutations, and maintain their owned indexes and records.

The canonical execution boundary is [`core::simulation::run_tick`](src/core/simulation.rs). It advances one simulated minute and processes due work in a deterministic order. External adapters, the gameplay harness, tests, and reports observe or request behavior through the same production systems; they do not create alternate mutation paths.

The source ownership map and mutation patterns are in [`ARCHITECTURE.md`](ARCHITECTURE.md). The current domain list and exclusions are in [`STATUS.md`](STATUS.md).

## Local verification

Use the narrowest lane that proves the change, then run the completion gate before handoff.

| Need | Command |
| --- | --- |
| Focus one library test | `cargo test-focused <filter>` |
| Check library code | `cargo check-fast` |
| Check the gameplay harness | `cargo check-harness` |
| Run fast library tests | `cargo test-fast` |
| Run the deterministic invariant soak | `cargo soak` |
| Run the canonical harness smoke | `cargo harness-smoke` |
| Run the full local gate | `.\scripts\verify.cmd` |

On Windows PowerShell, use `.\scripts\verify.cmd`; the script accepts `-Jobs N` to cap Cargo parallelism. `.\scripts\verify.cmd -Fast` is the short library lane, and `.\scripts\verify.cmd -Fast -Harness` is the short harness lane. These omit the broad Clippy/all-target work that belongs to the completion gate.

The completion gate is owned by [`scripts/verify.ps1`](scripts/verify.ps1) and wrapped by [`scripts/verify.cmd`](scripts/verify.cmd). It runs formatting, strict library/harness Clippy, all-target tests, and the exact ignored harness smoke contract. Verification is local; hosted runners and GitHub Actions are not project authorities.

When optimized compilation can change the relevant behavior, also run `cargo test --release --locked`.

## Gameplay harness

The harness is a bounded deterministic evaluation surface, not a human-play or UX test. Smoke is the normal iteration path:

```text
cargo harness
cargo harness -- --mode smoke --strategy press
```

Full comparison is explicit and bounded:

```text
cargo harness-full --samples 8
```

Smoke accepts `all`, `rush`, `press`, or `recon`; full mode compares all strategies on matched seeds. The acting policy uses player-visible information and surfaced decisions. Developer-audit diagnostics do not feed action selection. See [`TESTING.md`](TESTING.md) for evidence rules and mode semantics.
