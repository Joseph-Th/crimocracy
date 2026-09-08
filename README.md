# Crimocracy

Deterministic Rust simulation foundation for a systemic crime-organization strategy game. The player acts through people, information, plans, policies, relationships, delegated authority, enterprises, and institutions.

## Start here

Use each document for one job. [`AGENTS.md`](AGENTS.md) owns agent execution rules and routing; [`STATUS.md`](STATUS.md) owns implemented scope and exclusions; [`ARCHITECTURE.md`](ARCHITECTURE.md) owns state ownership, mutation, determinism, persistence, invariants, and tick order; [`TESTING.md`](TESTING.md) owns verification and harness evidence; [`GAME_DESIGN.md`](GAME_DESIGN.md) owns product intent.

`content::build_registry()` builds immutable authored definitions. `AppState` owns serializable campaign state and deterministic runtime state. [`core::simulation::run_tick`](src/core/simulation.rs) advances exactly one simulated minute through the contractual phase order in [`ARCHITECTURE.md`](ARCHITECTURE.md).

Consequential mutation goes through the owning system's `validate_* → commit` or `decide_* → validate_* → commit` path. Tests, examples, adapters, and tools use those same production paths. Do not construct authoritative `*Record` values or patch owner-private state as a shortcut.

## Reading order

| Need | Read |
|---|---|
| Agent routing and execution rules | [`AGENTS.md`](AGENTS.md) |
| What exists and what is excluded | [`STATUS.md`](STATUS.md) |
| State ownership, mutation, determinism, persistence | [`ARCHITECTURE.md`](ARCHITECTURE.md) |
| How behavior is proved | [`TESTING.md`](TESTING.md) |
| Player experience and product intent | [`GAME_DESIGN.md`](GAME_DESIGN.md) |

The owning `src/` module and its focused tests are the authority for executable
behavior. Design intent does not prove implementation and status does not define
intent. If authorities conflict, repair the owning document and implementation.

## Local commands

```powershell
cargo check-fast
cargo test-focused <filter>
.\scripts\verify.cmd -Check
.\scripts\verify.cmd -Fast
.\scripts\verify.cmd -Fast -Harness
.\scripts\verify.cmd
cargo harness
cargo harness-rush
cargo harness-press
cargo harness-recon
cargo harness-full --samples 8
```

[`TESTING.md`](TESTING.md) owns which lane completes each class of change and what each harness mode proves. [`scripts/verify.ps1`](scripts/verify.ps1) owns the verification gate; [`scripts/verify.cmd`](scripts/verify.cmd) is its wrapper. Verification is local and does not depend on hosted CI.

Build-profile tuning and measured performance notes live with Cargo configuration rather than in the current contract documents.
