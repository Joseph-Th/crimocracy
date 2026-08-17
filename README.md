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

Use the smallest focused test while editing. The fast Rust completion gate is:

```text
cargo fmt --check
cargo clippy --locked --all-targets -j 2 -- -D warnings
cargo test --locked --all-targets --quiet -j 2
cargo test --locked --example gameplay_harness tests::smoke_mode_covers_canonical_paths -- --ignored --exact --nocapture
```

The GitHub Actions gate runs those checks in one cached job, compiles every target through the
all-target test pass, marks the controlled smoke contract as an explicit ignored test, and runs it
immediately afterward with focused output. Superseded runs on the same branch are cancelled. CI
disables incremental artifacts and test debug symbols to keep clean-run linking and cache uploads
small. It does not run the long-form narrative batch or an optimized build on every change.
Use the smoke harness for normal iteration. Running the example without a mode is also a fast
smoke run; full calibration is always explicit:

```text
cargo run --locked --quiet --example gameplay_harness -- --mode smoke
```

When iterating on one policy branch, focus the smoke run without changing the default CI contract:

```text
cargo run --locked --quiet --example gameplay_harness -- --mode smoke --strategy press
```

When a change can differ under optimized compilation, especially assertions, indexing, arithmetic, or persistence-sensitive runtime behavior, also run:

```text
cargo test --release --locked -j 2
```

The explicit gameplay/integration lane is the controlled/calibration harness:

```text
cargo run --locked --example gameplay_harness -- --mode full --samples 8
```

`--samples` must be between 1 and 64. Full mode defaults to three samples, the minimum that
exercises all three authored fixture variations; use a larger explicit count for deeper sensitivity
evidence. Smoke accepts `--strategy all|rush|press|recon`; full mode always runs every strategy for
matched comparison. `--seed` accepts a hexadecimal value and keeps matched
strategy branches on the same simulation seed. The harness uses synthetic authored scenarios
through production mutation paths, keeps player-visible policy inputs separate from `[DEV AUDIT]`
diagnostics, and provides bounded deterministic strategy/sensitivity evidence rather than a
natural-play or human-UX verdict. Full mode is deliberately more verbose and expensive; smoke mode
is the CI/local fast path. Each seed selects a small authored fixture variation, shared by all
strategy branches for that seed, so batches exercise more than one fixed venue and patrol rhythm.
The harness validates structural and registry-aware state after setup and at observation boundaries.
Narrative sessions observe two simulated days; batch sensitivity runs observe one day so repeated
routine ticks do not dominate the evidence or runtime.
