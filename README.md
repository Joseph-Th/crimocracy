# Crimocracy

Deterministic Rust simulation foundation for a systemic crime-organization strategy game. The player acts through people, information, plans, policies, relationships, delegated authority, enterprises, and institutions.

## Agent cockpit — 60s quick start

You are the agent driving this system. The cockpit is [`AGENTS.md`](AGENTS.md) — it
compresses every authority into one scannable path. This page orients you; that
page routes you to the cheapest correct proof for any change.

```text
Registry (immutable, build_registry)  ─┐
AppState  (15 substates + 5 RNG streams + clocks) ─┤─► run_tick (1 min, 14 phases)
Harness   (smoke/full, player-visible only)        ─┘
```

```powershell
cargo check-fast          # 0.4s  — does it compile?
cargo test-focused social # 0.5s  — did one behavior change?
.\scripts\verify.cmd -Fast # 1s   — iteration gate (fmt + lib --skip soak)
.\scripts\verify.cmd       # 5-10s — broad gate (only for persistence/invariant/cross-domain)
cargo harness-rush         # 0.15s — does one strategy still narrate correctly?
cargo harness-full --samples 8  # 5s — full calibration + artifacts in target/harness-runs/
```

- Mutate only through the owning system's `validate_* → commit` or `decide_* → apply_*`.
  Never construct a `*Record{}` literal — `AGENTS.md:§3` lists every canonical entry point.
- `run_tick` (`src/core/simulation.rs`) is the only authoritative minute. Speed is
  an adapter concern — call it more often, don't change its semantics.
- The tower is `core → registry/content → world/social/intelligence/history/reports → finance/delegation/reputation/… → operations/legal/enterprises/economy`. See `AGENTS.md:§2` for the diagram.

## Reading order

| Need | Read | Time |
|---|---|---|
| Agent routing and shortest proof | [`AGENTS.md`](AGENTS.md) (cockpit) | 2 min |
| What exists and what is excluded | [`STATUS.md`](STATUS.md) | 1 min |
| State ownership, mutation, determinism, persistence | [`ARCHITECTURE.md`](ARCHITECTURE.md) | 5 min |
| How behavior is proved | [`TESTING.md`](TESTING.md) | 3 min |
| Player experience and product intent | [`GAME_DESIGN.md`](GAME_DESIGN.md) | as needed |

The owning `src/` module and its focused tests are the authority for executable
behavior. Design intent does not prove implementation and status does not define
intent. If authorities conflict, repair the owning document and implementation.

## Architecture shape — the tower

```
Layer 4  operations · legal · enterprises · economy     (orchestrating, cross-owner transactions)
Layer 3  finance · delegation · reputation · decisions · contacts · opportunities · recruitment
Layer 2  world · social · intelligence · history · reports   (leaf, no cross-domain writes)
Layer 1  registry ◄── content::build_registry           (immutable after build)
Layer 0  core::{id,time,entity,attention,state,simulation,persistence,invariants}
```

- `Registry` — immutable authored definitions and validated lookup tables.
- `AppState` — serializable mutable campaign state, typed IDs, clocks, and 5 independent `ChaCha8Rng` streams.
- Domain systems — validate requests, commit mutations, and maintain owned indexes and records.

Canonical execution boundary is [`core::simulation::run_tick`](src/core/simulation.rs). It advances one simulated minute and resolves due work in deterministic order (14 phases — see `ARCHITECTURE.md` for the sequence). Adapters, the harness, tests, and reports observe or request through the same production systems.

Full ownership map, phase diagram, RNG streams, and persistence envelope are in [`ARCHITECTURE.md`](ARCHITECTURE.md); the quick-ref table of `validate_*`/`decide_*` entry points is in [`AGENTS.md:§3`](AGENTS.md#3-canonical-operations--quick-reference).

## Local verification — cheapest lane that proves the change

Fast iteration uses the cheapest lane that proves the change. Routine completion uses the smallest scripted lane selected by the changed surface; the full gate is reserved for contracts that require its broader harness/Clippy coverage or for an explicit broad checkpoint. Lane ownership is in [`TESTING.md`](TESTING.md); the gate is owned by [`scripts/verify.ps1`](scripts/verify.ps1) and wrapped by [`scripts/verify.cmd`](scripts/verify.cmd).

```
Did you touch persistence, invariants, cross-domain, or verify infra?
  YES → .\scripts\verify.cmd                (broad gate, 5-10s)
  NO  → Did you touch harness?
          YES → .\scripts\verify.cmd -Fast -Harness  (1-2s)
          NO  → cargo test-focused <filter>  (0.5s)  or  .\scripts\verify.cmd -Fast  (1s)
```

| Need | Command | Warm |
|---|---|---|
| Type-check library | `cargo check-fast` | ~0.4s |
| Type-check harness | `cargo check-harness` | ~1s |
| One focused test | `cargo test-focused <filter>` | ~0.5s |
| Fast lib tests (no soak) | `cargo test-fast` | ~0.7s |
| Auto-rerun on save | `.\scripts\watch.cmd [-Filter <pattern> \| -Harness \| -Check]` | per-run warm cost of the chosen lane |
| Harness smoke, one strategy | `cargo harness-rush` / `cargo harness-press` / `cargo harness-recon` | ~0.15s |
| Fast lane (fmt + lib) | `.\scripts\verify.cmd -Fast` | ~1-2s |
| Broad local gate | `.\scripts\verify.cmd` | ~5-10s |

The broad gate runs `cargo fmt --check`, lib+integration tests, the exact ignored harness smoke contract selected fail-closed, one full-mode harness run (`--samples 1` on `[profile.harness]`, covering the narrative arcs, probes, and cross-branch contracts smoke skips), and strict Clippy for `lib` + `gameplay_harness`. Do not run it after a passing fast lane merely for reassurance. Verification is local; hosted runners are not authorities. When optimized compilation can change behavior, also run `cargo test-release`.

Full lane table, decision tree, and harness mode matrix are in [`TESTING.md`](TESTING.md); the cockpit summary is in [`AGENTS.md:§5`](AGENTS.md#5-verification-lanes--which-command-when).

## Gameplay harness — bounded evaluation surface

Bounded deterministic evaluation surface, not a human-play test. All harness commands execute on `[profile.harness]` (dev semantics, `opt-level = 1`), so warm runs are ~10x faster than dev-profile execution and never disturb library iteration caches. Acting policy uses only player-visible information and surfaced decisions; `[DEV AUDIT]` is diagnostic only.

```text
cargo harness                                        # smoke: 3 strategies + legal foundation
cargo harness -- --mode smoke --strategy press        # focused smoke: one branch
cargo harness-rush / -press / -recon                 # aliases for focused smoke
cargo harness-full --samples 8                        # full: rotation + probes + batches
cargo harness-full -- --samples 8 --artifact-dir target/my-run
```

Smoke covers canonical strategies (1 campaign day); full mode compares all strategies on matched seeds, rotates across `NARRATIVE_SEED_ROTATION=3` fixture variations (Clockwork/Crowded/Quiet), runs 4 probes + sensitivity batches, and writes per-run `target/harness-runs/*.json` artifacts that preserve seeds and raw metrics. See [`TESTING.md`](TESTING.md) for modes and evidence rules; the cockpit cheat sheet is in [`AGENTS.md:§8`](AGENTS.md#8-harness--bounded-evaluation-surface-not-a-playtest).
