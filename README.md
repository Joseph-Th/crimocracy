# Crimocracy

Crimocracy is a deterministic Rust simulation foundation for a systemic crime-organization strategy game. The player acts through people, information, plans, policies, relationships, delegated authority, enterprises, and institutions rather than directly scripting every simulated action.

## Start here

1. Read `AGENTS.md` before changing code. It is the concise repository execution card.
2. Read `STATUS.md` for the foundation that is currently implemented.
3. Read `ARCHITECTURE.md` for ownership, canonical mutation, determinism, persistence, and invariants.
4. Read `TESTING.md` for test and gameplay-evidence rules.
5. Read `GAME_DESIGN.md` only when product intent or player-facing design criteria are relevant.

Do not use `GAME_DESIGN.md` as proof that a feature is implemented. Do not use `STATUS.md` as a substitute for product intent.

## Current architecture

The implemented foundation uses explicit serializable `AppState`, immutable Rust-authored registry definitions, typed persistent IDs, domain-owned state, canonical system functions, deterministic state-owned RNG streams, and invariant validation.

Consequential behavior belongs to owning systems. UI, adapters, tests, examples, and reports must not become alternate mutation paths.

See `STATUS.md` for current domain coverage, `ARCHITECTURE.md` for technical contracts, and `TESTING.md` for verification and gameplay-evidence contracts.

## Documentation authority

| Question | Authority |
| --- | --- |
| How should repository work proceed? | `AGENTS.md` |
| What is the implemented ownership and execution model? | `ARCHITECTURE.md` |
| What capability currently exists or is excluded? | `STATUS.md` |
| How are tests and gameplay evidence selected? | `TESTING.md` |
| What player experience and product behavior are intended? | `GAME_DESIGN.md` |
| What behavior is executable now? | Owning source module and tests |

If these current authorities disagree about the same subject, treat the disagreement as a defect. Resolve it in the owning document and implementation rather than selecting the convenient description.

## Verification

Use the smallest focused test while editing. The one-command local gate runs the whole fast
completion contract with clear per-stage output and timing:

```text
.\scripts\verify.ps1
```

It runs the four canonical steps below in order, stops at the first failing stage, and exits
non-zero on failure. Build parallelism is cargo-autodetected; pass `-Jobs N` to cap it when you want
to leave the machine quieter. The same four steps, run raw, are:

```text
cargo fmt --check
cargo clippy --locked --lib --example gameplay_harness -- -D warnings
cargo test --locked --all-targets --quiet
cargo test --locked --example gameplay_harness tests::smoke_mode_covers_canonical_paths -- --ignored --exact --nocapture
```

The local completion gate above is the authoritative routine verification path. It compiles every
target through the all-target test pass, marks the controlled smoke contract as an explicit ignored
test, and runs it immediately afterward with focused output. Clippy intentionally covers production
library code and the gameplay harness; the all-target test pass owns test-target compilation so the
gate does not build the same test targets twice. No hosted runner or GitHub Actions workflow is part
of verification. The routine gate does not run the long-form narrative batch or an optimized build
on every change.
Use the smoke harness for normal iteration. Running the example without a mode is also a fast
smoke run; full calibration is always explicit:

```text
cargo run --locked --quiet --example gameplay_harness -- --mode smoke
```

The repository also provides Cargo aliases for the short loops:

```text
cargo check-fast
cargo lint-fast
cargo test-fast
cargo soak
cargo harness-smoke
cargo harness-rush
cargo harness-press
cargo harness-recon
cargo harness -- --mode smoke --strategy press
```

`cargo test-fast` skips only the named invariant soak; `cargo soak` runs that deliberate stress
check explicitly, while the normal `cargo test` gate still includes it. Focused `--strategy` smoke
runs skip the independent legal-foundation probe; the default smoke run and completion gate still execute it.

When iterating on one policy branch, focus the smoke run without changing the default completion contract:

```text
cargo run --locked --quiet --example gameplay_harness -- --mode smoke --strategy press
```

When a change can differ under optimized compilation, especially assertions, indexing, arithmetic, or persistence-sensitive runtime behavior, also run:

```text
cargo test --release --locked
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
is the local fast path. Each seed selects a small authored fixture variation, shared by all
strategy branches for that seed, so batches exercise more than one fixed venue and patrol rhythm.
The harness validates structural and registry-aware state after setup and at observation boundaries.
Narrative sessions observe two simulated days of routine ticks; the press branch's player-run
defector watch starts after its cold-case wait and can finish a short way past that boundary.
Batch sensitivity runs observe one day so repeated routine ticks do not dominate the evidence or runtime.
