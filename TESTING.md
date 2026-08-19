# Testing

Owns test selection, harness evidence, and local verification. Ownership is in [`ARCHITECTURE.md`](ARCHITECTURE.md); scope is in [`STATUS.md`](STATUS.md).

## Test selection

Use the narrowest proof that covers the change:

| Change | First proof | Broader lane |
| --- | --- | --- |
| One library behavior | `cargo test-focused <filter>` | `cargo test-fast` |
| Library implementation | `cargo check-fast` or focused test | `.\scripts\verify.cmd -Fast` |
| Harness filter | `cargo harness -- --mode smoke --strategy rush` or `cargo harness-press` | `.\scripts\verify.cmd -Fast -Harness` |
| Persistence, invariants, or cross-domain behavior | Focused owner test plus load/continuation evidence | `.\scripts\verify.cmd` (full gate) |

Tests prove observable production behavior: calculations, transitions, transactions, invariants, serialization, deterministic continuation, and failure paths.

- Exercise canonical system operations, not private helpers or test-only mutation shortcuts.
- Assert typed error variants and relevant fields, not rendered text.
- For atomic rejection, assert authoritative state is unchanged.
- Use explicit seeds and stable ordering. Do not hunt for a passing seed.
- Keep content-count, CRUD, or smokes only when they protect a real contract.

Ordinary tests live with their owning module under `#[cfg(test)]`. Name tests after behavior. Use `make_test_*` for local fixtures and the idempotent `*_for_test` pattern when extending the shared production registry. The invariant soak is stress evidence, not a replacement for focused behavioral tests.

## Local verification

Verification is local and for solo iteration. Hosted CI and GitHub Actions are not authorities.

### Fast lanes (use while coding)

| Need | Command | Warm |
| --- | --- | --- |
| Type-check library | `cargo check-fast` | ~0.9s |
| Type-check harness | `cargo check-harness` | ~1s |
| Lib tests (no soak) | `cargo test-fast` | ~0.7s |
| One test / module | `cargo test-focused <filter>` | ~0.5s |
| Harness smoke, one strategy | `cargo harness-rush` / `-press` / `-recon` | ~1s |
| Fast lane (fmt + lib) | `.\scripts\verify.cmd -Fast` | ~1-2s |
| Fast harness lane | `.\scripts\verify.cmd -Fast -Harness` | ~2s |
| Filtered fast lane | `.\scripts\verify.cmd -Fast -Filter <pattern>` | ~0.5-1s |

`cargo check-fast` and `cargo test-focused` are the inner loop. `.\scripts\verify.cmd -Fast` is the next lane and still avoids soak, clippy, and harness compilation on a warm build.

### Completion gate (run before push)

```text
.\scripts\verify.cmd
.\scripts\verify.cmd -Jobs 2
```

The gate runs fail-fast in order:

1. `cargo fmt --check`
2. `cargo clippy --locked --lib --example gameplay_harness -- -D warnings`
3. `cargo test --locked --all-targets --quiet`
4. Exact ignored test `tests::smoke_mode_covers_canonical_paths` (selected fail-closed)

[`scripts/verify.ps1`](scripts/verify.ps1) is the owner; [`scripts/verify.cmd`](scripts/verify.cmd) is the wrapper. The smoke stage requires exactly one selectable ignored test; `.\scripts\verify.ps1 -SelfTest` checks that selection. [`tests/documentation_contracts.rs`](tests/documentation_contracts.rs) protects the authority set, local links, concrete routes, Cargo aliases, and published schema/content revisions.

The gate is not the inner loop. Iterate with a fast lane and run the gate once before handoff. Use `-Jobs N` to cap parallelism, `-NoClippy` or `-NoFmt` only when those stages are known to pass.

Cargo profiles use `debug = "line-tables-only"`, `incremental = true`, and `codegen-units = 256` so warm builds reuse artifacts while keeping useful panic locations. The verify script reports per-stage timing and a total, and hides `cargo --quiet` output on success while showing full output on failure (or with `-Verbose`).

## Gameplay-harness evidence

[`examples/gameplay_harness.rs`](examples/gameplay_harness.rs) evaluates bounded deterministic policy treatments through production paths.

| Mode | Evidence |
| --- | --- |
| `smoke` | Fast canonical strategy and legal-foundation contract |
| `full` | Narrative, matched-seed, property/legal, opportunity, and bounded sensitivity evidence |

RUSH, PRESS, and RECON use the same seed-selected authored fixture and scenario timeline. Acting policy may use only organization/player-visible information, persisted reports/outcomes, and surfaced decision requests. Hidden investigation and evidence state is audit-only. Missing acting information or canonical rejection fails the run; missing events are observed absence.

`--samples` varies simulation/world seed and bounded timing offsets, range 1..=64. Matched branches share seed, fixture, and timeline. Per-run events and `RunMetrics` are raw evidence beneath aggregates; aggregates are not universal quality scores. Persisted artifacts retain per-run seeds and raw metrics. `full` mode writes per-run JSON to `--artifact-dir` (default `target/harness/`) with a `summary-<seed>.json`.

Structural and registry-aware validation runs at setup and observation boundaries, not on every tick. The harness checks the narrative defection-surveillance, second-wind, opportunity-prioritization, and organizational-capacity contracts defined by its scenarios; changing those contracts requires updating this section and the focused harness tests together. The capacity probe proves overlapping assignments are rejected atomically and become available after the prior operation reaches a terminal state.

`cargo harness` runs smoke by default. For explicit comparison:

```text
cargo harness-full --samples 8
cargo harness-full -- --samples 8 --artifact-dir target/my-run
```

## Completion checklist

Before handoff:

- Run the narrowest focused proof.
- Run the applicable fast lane (`cargo test-fast` or `.\scripts\verify.cmd -Fast`).
- Run `.\scripts\verify.cmd`.
- Run `cargo soak` or `cargo harness-full --samples 8` when the changed contract requires that evidence.
- Review `git diff --check`, generated output, and the final worktree.

When a test, harness mode, alias, or verification rule changes, update this document and the owning script or command definition in the same change.
