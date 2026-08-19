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

Verification is local and optimized for solo iteration. Use the cheapest lane that proves your change while editing, then the full gate once before push.

| Need | Command | Warm time |
| --- | --- | --- |
| Type-check library | `cargo check-fast` | ~0.9s |
| Type-check harness | `cargo check-harness` | ~1s |
| One focused test | `cargo test-focused <filter>` | ~0.5s |
| Fast lib tests (no soak) | `cargo test-fast` | ~0.7s |
| Harness smoke, one strategy | `cargo harness-rush` / `cargo harness-press` / `cargo harness-recon` | ~1s |
| Fast lane (fmt + lib tests) | `.\scripts\verify.cmd -Fast` | ~1-2s |
| Harness fast lane | `.\scripts\verify.cmd -Fast -Harness` | ~2s |
| Filtered fast lane | `.\scripts\verify.cmd -Fast -Filter <pattern>` | ~0.5-1s |
| Deterministic soak | `cargo soak` | ~5s |
| Harness smoke contract | `cargo harness-smoke` | ~1s |
| Full local gate | `.\scripts\verify.cmd` | ~4-8s warm |
| Full gate, cap parallelism | `.\scripts\verify.cmd -Jobs 2` | — |

Edits should be proved with `cargo check-fast` / `cargo test-focused`; save the full gate for the handoff. See [`TESTING.md`](TESTING.md) for lane ownership and harness mode semantics.

The completion gate is owned by [`scripts/verify.ps1`](scripts/verify.ps1) and wrapped by [`scripts/verify.cmd`](scripts/verify.cmd). It runs `cargo fmt --check`, strict Clippy for `lib` + `gameplay_harness`, `cargo test --all-targets`, and the exact ignored harness smoke contract selected fail-closed. Verification is local; hosted runners and GitHub Actions are not project authorities.

When optimized compilation can change the relevant behavior, also run `cargo test --release --locked`.

## Gameplay harness

The harness is a bounded deterministic evaluation surface, not a human-play or UX test. Smoke is the normal iteration path:

```text
cargo harness
cargo harness -- --mode smoke --strategy press
cargo harness-rush
```

Full comparison is explicit, bounded, and artifact-persisting:

```text
cargo harness-full --samples 8
cargo harness-full -- --samples 8 --artifact-dir target/my-run
```

Smoke accepts `all`, `rush`, `press`, or `recon`; full mode compares all strategies on matched seeds and writes per-run `target/harness/*.json` artifacts that preserve seeds and raw metrics beneath aggregate diagnostics. The acting policy uses player-visible information and surfaced decisions. Developer-audit diagnostics do not feed action selection. See [`TESTING.md`](TESTING.md) for evidence rules and mode semantics.
