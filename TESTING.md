# Testing

This document owns test selection, gameplay-harness evidence, and local verification. State ownership belongs to [`ARCHITECTURE.md`](ARCHITECTURE.md); implemented scope belongs to [`STATUS.md`](STATUS.md).

## Test selection

Use the narrowest proof that covers the changed behavior:

| Change | First proof | Broader lane |
| --- | --- | --- |
| One library behavior | `cargo test-focused <filter>` | `cargo test-fast` |
| Library implementation | `cargo check-fast` or a focused test | `.\scripts\verify.cmd -Fast` |
| Harness filter | `cargo harness -- --mode smoke --strategy rush` or `cargo harness-press` | `.\scripts\verify.cmd -Fast -Harness` |
| Persistence, invariants, or cross-domain behavior | Focused owner test plus continuation/load evidence | `.\scripts\verify.cmd` (full gate) |

Tests prove observable production behavior: calculations, state transitions, transactions, invariants, serialization, deterministic continuation, and failure paths.

- Exercise canonical system operations rather than private helpers or test-only mutation shortcuts.
- Match typed error variants and relevant fields rather than rendered error text.
- For atomic rejection, assert that authoritative state is unchanged.
- Use explicit seeds and stable ordering for deterministic tests; do not search for a passing seed.
- Keep content-count, CRUD, and generator-smoke tests only when they protect a real contract.

Ordinary tests live with their owning module under `#[cfg(test)]`. Name tests after behavior. Use `make_test_*` for local fixtures and the established idempotent `*_for_test` pattern when a shared production registry must be extended. The invariant soak is stress evidence; it does not replace focused behavioral tests.

## Local verification

Verification is local and optimized for fast solo iteration on a single machine. Hosted CI and GitHub Actions are not authorities.

### Fast iteration lanes (use while coding)

| Need | Command | Typical warm time |
| --- | --- | --- |
| Type-check library only | `cargo check-fast` | ~0.9s |
| Type-check harness | `cargo check-harness` | ~1s |
| Run lib tests (no soak) | `cargo test-fast` | ~0.7s |
| Run one test / module | `cargo test-focused <filter>` | ~0.5s |
| Harness smoke, one strategy | `cargo harness-rush` / `-press` / `-recon` | ~1s |
| Full fast lane (fmt + lib tests) | `.\scripts\verify.cmd -Fast` | ~1-2s |
| Fast harness lane | `.\scripts\verify.cmd -Fast -Harness` | ~2s |
| Filtered fast lane | `.\scripts\verify.cmd -Fast -Filter <pattern>` | ~0.5-1s |

`cargo check-fast` / `cargo test-focused` are the inner-loop commands. `.\scripts\verify.cmd -Fast` is the next lane up and still avoids the soak, clippy, and harness compilation on a warm build.

### Completion gate (run before push / handoff)

```text
.\scripts\verify.cmd
.\scripts\verify.cmd -Jobs 2   # cap cargo parallelism on a hot / quiet machine
```

The gate runs, in order, fail-fast:

1. `cargo fmt --check`
2. `cargo clippy --locked --lib --example gameplay_harness -- -D warnings`
3. `cargo test --locked --all-targets --quiet`
4. the exact ignored test `tests::smoke_mode_covers_canonical_paths` (selected fail-closed)

`scripts/verify.ps1` is the owner; `scripts/verify.cmd` is the execution-policy-safe wrapper. The smoke stage requires exactly one selectable ignored test; `.\scripts\verify.ps1 -SelfTest` checks that selection alone. `tests/documentation_contracts.rs` protects the authority set, local links, concrete routes, Cargo aliases, and published schema/content revisions.

The gate is intentionally not the inner loop. Do not run the full gate after every edit — that is the time sink. Iterate with a fast lane and run the gate once before handoff; add `-Jobs N` when you want the machine quieter, or `-NoClippy` / `-NoFmt` only when you know those stages already pass and want to isolate a test failure.

`cargo` profiles use `debug = "line-tables-only"`, `incremental = true`, and `codegen-units = 256` so warm builds reuse artifacts and keep useful panic locations without paying for full debuginfo. The verify script reports per-stage timing and a total, and hides `cargo --quiet` output on success to stay concise while still showing full output on failure (or with `-Verbose`).

## Gameplay-harness evidence

[`examples/gameplay_harness.rs`](examples/gameplay_harness.rs) evaluates bounded deterministic policy treatments through production paths.

| Mode | Evidence |
| --- | --- |
| `smoke` | Fast canonical strategy and legal-foundation contract. |
| `full` | Narrative, matched-seed, property/legal, opportunity, and bounded sensitivity evidence. |

RUSH, PRESS, and RECON use the same seed-selected authored fixture and authored-content-derived scenario timeline. The acting policy may use only organization/player-visible information, persisted reports/outcomes, and surfaced decision requests. Hidden investigation and evidence state is audit-only. Missing acting information or canonical rejection fails the run; missing events remain observed absence rather than being converted into a verdict.

`--samples` varies the simulation/world seed and bounded policy timing offsets and is bounded to 1..=64. Matched branches use the same seed, fixture, and timeline. Per-run events and `RunMetrics` are raw evidence beneath aggregate diagnostics; aggregate output is not a universal game-quality score or a human-UX verdict. Persisted evaluation artifacts must retain per-run seeds and raw metrics beneath derived findings — `full` mode writes per-run JSON to `--artifact-dir` (default `target/harness/`) together with a `summary-<seed>.json`. Each artifact preserves the seed, profile, strategy, variation, and raw metrics so findings remain reproducible.

Structural and registry-aware validation runs at setup and consequential observation boundaries. Routine minutes are not revalidated on every tick. The harness also checks the narrative defection-surveillance, second-wind, opportunity-prioritization, and organizational-capacity contracts described by its production scenarios; changes to those contracts require updating this section and the focused harness tests together. The capacity probe proves that overlapping assignments are rejected atomically and become available after the prior operation reaches a terminal state.

`--artifact-dir <path>` overrides the default artifact location; `cargo harness-full -- --artifact-dir target/my-run` is useful when comparing runs without clobbering.

## Completion checklist

Before handoff:

- run the narrowest focused proof;
- run the applicable fast lane (`cargo test-fast` or `.\scripts\verify.cmd -Fast`);
- run `.\scripts\verify.cmd`;
- run `cargo soak` or `cargo harness-full --samples 8` when the changed contract requires that evidence;
- review `git diff --check`, generated output, and the final worktree.

When a test, harness mode, alias, or verification rule changes, update this document and the owning script or command definition in the same change.
