# Crimocracy

Crimocracy is a deterministic Rust simulation foundation for a systemic crime-organization strategy game. The player acts through people, information, plans, policies, relationships, delegated authority, enterprises, and institutions rather than directly scripting every simulated action.

## Start here

1. Read `AGENTS.md` before changing code. It is the repository execution and architecture contract.
2. Read `STATUS.md` for the foundation that is currently implemented.
3. Read `GAME_DESIGN.md` for product intent and player-facing design criteria.
4. Route the change to the owning module named by `AGENTS.md` and inspect its focused tests before editing.

Do not use `GAME_DESIGN.md` as proof that a feature is implemented. Do not use `STATUS.md` as a substitute for product intent.

## Current architecture

The implemented foundation uses explicit serializable `AppState`, immutable Rust-authored registry definitions, typed persistent IDs, domain-owned state, canonical system functions, deterministic state-owned RNG streams, and invariant validation.

Consequential behavior belongs to owning systems. UI, adapters, tests, examples, and reports must not become alternate mutation paths.

See `STATUS.md` for the current domain coverage and `AGENTS.md` for the canonical mutation, determinism, ownership, naming, persistence, and test rules.

## Documentation authority

| Question | Authority |
| --- | --- |
| How may the repository be changed? | `AGENTS.md` |
| What architecture and capability currently exist? | `STATUS.md` |
| What player experience and product behavior are intended? | `GAME_DESIGN.md` |
| What behavior is executable now? | Owning source module and tests |

If these current authorities disagree about the same subject, treat the disagreement as a defect. Resolve it in the owning document and implementation rather than selecting the convenient description.

## Verification

Use the smallest focused test while editing. The normal Rust completion checks are:

```text
cargo fmt --check
cargo check --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked -j 2
```

When a change can differ under optimized compilation, especially assertions, indexing, arithmetic, or persistence-sensitive runtime behavior, also run:

```text
cargo test --release --locked -j 2
```

The explicit gameplay/integration lane is the controlled/calibration harness:

```text
cargo run --locked --example gameplay_harness -- --samples 24
```

`--samples` must be between 1 and 64. The harness uses synthetic authored scenarios through production mutation paths, keeps player-visible policy inputs separate from `[DEV AUDIT]` diagnostics, and provides bounded deterministic strategy/sensitivity evidence rather than a natural-play or human-UX verdict.
