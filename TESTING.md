# Testing

This document owns test selection, gameplay-harness evidence, and local verification. State ownership belongs to [`ARCHITECTURE.md`](ARCHITECTURE.md); implemented scope belongs to [`STATUS.md`](STATUS.md).

## Test selection

Use the narrowest proof that covers the changed behavior:

| Change | First proof | Broader lane |
| --- | --- | --- |
| One library behavior | `cargo test-focused <filter>` | `cargo test-fast` |
| Library implementation | `cargo check-fast` or a focused test | `.\scripts\verify.cmd -Fast` |
| Gameplay harness or adapter | `cargo check-harness` or a focused harness mode | `.\scripts\verify.cmd -Fast -Harness` |
| Persistence, invariants, or cross-domain behavior | Focused owner test plus continuation/load evidence | `.\scripts\verify.cmd` |

Tests prove observable production behavior: calculations, state transitions, transactions, invariants, serialization, deterministic continuation, and failure paths.

- Exercise canonical system operations rather than private helpers or test-only mutation shortcuts.
- Match typed error variants and relevant fields rather than rendered error text.
- For atomic rejection, assert that authoritative state is unchanged.
- Use explicit seeds and stable ordering for deterministic tests; do not search for a passing seed.
- Keep content-count, CRUD, and generator-smoke tests only when they protect a real contract.

Ordinary tests live with their owning module under `#[cfg(test)]`. Name tests after behavior. Use `make_test_*` for local fixtures and the established idempotent `*_for_test` pattern when a shared production registry must be extended. The invariant soak is stress evidence; it does not replace focused behavioral tests.

## Local completion gate

Run the repository-owned gate before handoff:

```text
`.\scripts\verify.cmd`
```

The gate runs, in order:

1. `cargo fmt --check`
2. strict Clippy for the library and `gameplay_harness`
3. `cargo test --locked --all-targets --quiet`
4. the exact ignored test `tests::smoke_mode_covers_canonical_paths`

The smoke stage is selected fail-closed: [`scripts/verify.ps1`](scripts/verify.ps1) lists ignored harness tests and requires exactly that contract before running it. `.\scripts\verify.ps1 -SelfTest` runs the selection check alone. The `.cmd` file is an execution-policy-safe wrapper. `tests/documentation_contracts.rs` protects the authority set, local links, concrete routes, Cargo aliases, and published schema/content revisions.

Verification is local. Do not create or depend on GitHub Actions workflows or hosted runners.

## Gameplay-harness evidence

[`examples/gameplay_harness.rs`](examples/gameplay_harness.rs) evaluates bounded deterministic policy treatments through production paths.

| Mode | Evidence |
| --- | --- |
| `smoke` | Fast canonical strategy and legal-foundation contract. |
| `full` | Narrative, matched-seed, property/legal, opportunity, and bounded sensitivity evidence. |

RUSH, PRESS, and RECON use the same seed-selected authored fixture and authored-content-derived scenario timeline. The acting policy may use only organization/player-visible information, persisted reports/outcomes, and surfaced decision requests. Hidden investigation and evidence state is audit-only. Missing acting information or canonical rejection fails the run; missing events remain observed absence rather than being converted into a verdict.

`--samples` varies the simulation/world seed and bounded policy timing offsets and is bounded to 1..=64. Matched branches use the same seed, fixture, and timeline. Per-run events and `RunMetrics` are raw evidence beneath aggregate diagnostics; aggregate output is not a universal game-quality score or a human-UX verdict. Persisted evaluation artifacts must retain per-run seeds and raw metrics beneath derived findings.

Structural and registry-aware validation runs at setup and consequential observation boundaries. Routine minutes are not revalidated on every tick. The harness also checks the narrative defection-surveillance and second-wind contracts described by its production scenarios; changes to those contracts require updating this section and the focused harness tests together.

## Completion checklist

Before handoff:

- run the narrowest focused proof;
- run the applicable fast lane;
- run `.\scripts\verify.cmd`;
- run `cargo soak` or `cargo harness-full --samples 8` when the changed contract requires that evidence;
- review `git diff --check`, generated output, and the final worktree.

When a test, harness mode, alias, or verification rule changes, update this document and the owning script or command definition in the same change.
