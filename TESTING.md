# Testing

This document owns Crimocracy's test organization, gameplay-harness evidence contract, and local verification procedure. Architecture belongs in `ARCHITECTURE.md`; implemented scope belongs in `STATUS.md`.

## Edit loop

Use the narrowest test that proves the changed behavior. The repository provides short Cargo aliases such as `cargo test-fast`, focused harness modes, and the local completion gate:

```text
.\scripts\verify.ps1
```

The gate runs formatting, strict production/harness Clippy, all-target tests, and the controlled smoke harness contract. `README.md` owns the current raw command expansion and operator examples; this document owns test semantics.

Repository verification is local. GitHub Actions and hosted runners are not verification authorities.

## Test contract

Tests should prove observable production behavior: calculations, state transitions, transactions, invariants, serialization boundaries, deterministic continuation, and failure paths.

- Prefer testing through canonical system operations instead of private helpers.
- Match typed error variants and relevant fields rather than rendered error text.
- Rejected operations assert unchanged authoritative state when atomic rejection is the contract.
- Deterministic tests use explicit seeds and stable ordering; ordinary tests do not search for a passing seed.
- A test that only proves an otherwise-unused production helper can be called does not establish production reachability.
- Content-count, trivial CRUD, or generator-smoke tests should remain only when they protect a real contract.

Co-locate ordinary tests with the owner under `#[cfg(test)]`. Name tests after behavior without a redundant `test_` prefix. Test-only fixtures that mutate a shared production registry use the established idempotent `*_for_test` pattern; throwaway local fixtures use `make_test_*`.

The deterministic invariant soak remains an explicit stress lane rather than a substitute for focused behavioral tests.

## Gameplay harness evidence

`examples/gameplay_harness.rs` is a controlled simulation/evaluation surface, not an alternate source of simulation rules and not evidence about human comprehension, enjoyment, or interface quality.

- RUSH, PRESS, and RECON are deterministic policy treatments within the same seed-selected authored fixture.
- The acting policy may use only organization/player-visible information, persisted reports/outcomes, and surfaced decision requests.
- Hidden investigation or evidence state is developer-audit-only and must not feed action selection.
- `--mode smoke` is the fast canonical strategy/legal-foundation contract; `--mode full` adds the broader narrative, matched-seed batch, property/legal, opportunity, and bounded sensitivity evidence.
- `--samples` varies the `AppState` simulation/world seed; there is no separate stochastic behavior seed. Matched strategy branches use the same selected fixture variation.
- Structural and registry-aware validation occurs at setup and observation boundaries.
- Per-run events and `RunMetrics` are raw evidence beneath aggregates. Aggregate output is diagnostic, not a universal game-quality score or durable research archive.
- Missing acting information or canonical validation rejection fails the controlled run explicitly. Missing events remain observed absence rather than being forced into a positive or negative verdict.
- Any future persisted evaluation artifact retains per-run seeds and raw metrics beneath derived findings.

## Completion

Before committing a normal change:

- run the narrowest focused test during iteration;
- run `.\scripts\verify.ps1`;
- run the soak or full harness only when the changed contract requires that evidence;
- keep `cargo check`/Clippy warning-free without broad suppressions;
- confirm generated output is not staged;
- review `git diff --check` and the final working tree.

When a test, harness, alias, or verification rule changes, update this document and the owning command/script in the same change.
