# Crimocracy

Deterministic Rust simulation foundation for a systemic crime-organization strategy game. The player acts through people, information, plans, policies, relationships, delegated authority, enterprises, and institutions.

## Reading order

| Need | Read |
| --- | --- |
| Repository rules and change routing | [`AGENTS.md`](AGENTS.md) |
| What exists and what is excluded | [`STATUS.md`](STATUS.md) |
| State ownership and mutation contracts | [`ARCHITECTURE.md`](ARCHITECTURE.md) |
| How behavior is proved | [`TESTING.md`](TESTING.md) |
| Player experience and product intent | [`GAME_DESIGN.md`](GAME_DESIGN.md) |

The owning `src/` module and its focused tests are the authority for executable behavior. Design intent does not prove implementation and status does not define intent. If authorities conflict, repair the owning document and implementation.

## Architecture shape

Three layers:

- `Registry` — immutable authored definitions and validated lookup tables.
- `AppState` — serializable mutable campaign state, typed IDs, clocks, and state-owned RNG streams.
- Domain systems — validate requests, commit mutations, and maintain owned indexes and records.

Canonical execution boundary is [`core::simulation::run_tick`](src/core/simulation.rs). It advances one simulated minute and resolves due work in deterministic order. Adapters, the harness, tests, and reports observe or request through the same production systems.

Source ownership and mutation patterns are in [`ARCHITECTURE.md`](ARCHITECTURE.md). Domain list and exclusions are in [`STATUS.md`](STATUS.md).

## Local verification

Fast iteration uses the cheapest lane that proves the change. Routine completion uses the smallest scripted lane selected by the changed surface; the full gate is reserved for contracts that require its broader harness/Clippy coverage or for an explicit broad checkpoint. Lane ownership is in [`TESTING.md`](TESTING.md); the gate is owned by [`scripts/verify.ps1`](scripts/verify.ps1) and wrapped by [`scripts/verify.cmd`](scripts/verify.cmd).

| Need | Command | Warm |
| --- | --- | --- |
| Type-check library | `cargo check-fast` | ~0.4s |
| Type-check harness | `cargo check-harness` | ~1s |
| One focused test | `cargo test-focused <filter>` | ~0.5s |
| Fast lib tests (no soak) | `cargo test-fast` | ~0.7s |
| Auto-rerun on save | `.\scripts\watch.cmd [-Filter <pattern> \| -Harness \| -Check]` | per-run warm cost of the chosen lane |
| Harness smoke, one strategy | `cargo harness-rush` / `cargo harness-press` / `cargo harness-recon` | ~0.15s |
| Fast lane (fmt + lib) | `.\scripts\verify.cmd -Fast` | ~1-2s |
| Broad local gate | `.\scripts\verify.cmd` | ~5-10s |

The broad gate runs `cargo fmt --check`, lib+integration tests, the exact ignored harness smoke contract selected fail-closed, one full-mode harness run (`--samples 1` on `[profile.harness]`, covering the narrative arcs, probes, and cross-branch contracts smoke skips), and strict Clippy for `lib` + `gameplay_harness`. Do not run it after a passing fast lane merely for reassurance. Verification is local; hosted runners are not authorities. When optimized compilation can change behavior, also run `cargo test-release`.

## Gameplay harness

Bounded deterministic evaluation surface, not a human-play test. All harness commands execute on `[profile.harness]` (dev semantics, `opt-level = 1`), so warm runs are ~10x faster than dev-profile execution and never disturb library iteration caches.

```text
cargo harness
cargo harness -- --mode smoke --strategy press
cargo harness-rush
cargo harness-full --samples 8
cargo harness-full -- --samples 8 --artifact-dir target/my-run
```

Smoke covers canonical strategies; full mode compares all strategies on matched seeds and writes per-run `target/harness-runs/*.json` artifacts that preserve seeds and raw metrics. Acting policy uses only player-visible information and surfaced decisions. See [`TESTING.md`](TESTING.md) for modes and evidence rules.
