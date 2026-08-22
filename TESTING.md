# Testing

Owns test selection, harness evidence, and local verification. Ownership is in [`ARCHITECTURE.md`](ARCHITECTURE.md); scope is in [`STATUS.md`](STATUS.md).

## Test selection

Use the narrowest proof that covers the change:

| Change | Focused feedback | Completion lane |
| --- | --- | --- |
| One library behavior | `cargo test-focused <filter>` | `.\scripts\verify.cmd -Fast` |
| Library implementation | `cargo check-fast` or focused test | `.\scripts\verify.cmd -Fast` |
| Harness filter | `cargo harness -- --mode smoke --strategy rush` or `cargo harness-press` | `.\scripts\verify.cmd -Fast -Harness` |
| Persistence, invariants, or cross-domain behavior | Focused owner test or load/continuation diagnosis | `.\scripts\verify.cmd` (broad gate) |

The two columns are not a required sequence. Use focused feedback while the implementation is moving or when isolating a failure; when the change is ready for its completion lane, go directly to that lane if it already compiles and exercises the same owner coverage.

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
| Auto-rerun on save | `.\scripts\watch.cmd` (`-Filter <pattern>`, `-Harness`, `-Check`) | per-run warm cost of the chosen lane |
| Harness smoke, one strategy | `cargo harness-rush` / `-press` / `-recon` | ~1s |
| Fast lane (fmt + lib) | `.\scripts\verify.cmd -Fast` | ~0.7-1.5s |
| Fast harness lane | `.\scripts\verify.cmd -Fast -Harness` | ~1-2s |
| Filtered fast lane | `.\scripts\verify.cmd -Fast -Filter <pattern>` | ~0.5-1s |

`cargo check-fast` and `cargo test-focused` are the inner loop. `scripts/watch.ps1` is the hands-free form of that loop: it reruns one focused lane on every save and never builds more than that lane. `.\scripts\verify.cmd -Fast` is the next lane and still avoids soak, clippy, and harness compilation on a warm build.

### Broad completion gate

```text
.\scripts\verify.cmd
.\scripts\verify.cmd -Jobs 2
```

The gate runs fail-fast in order:

1. `cargo fmt --check`
2. `cargo test --locked --lib --tests --quiet`
3. Harness unit tests (`cargo test --locked --quiet --example gameplay_harness --lib`): the example's own options-parsing and financial-branch contract tests, which the lib+integration stage never compiles
4. Exact ignored test `tests::smoke_mode_covers_canonical_paths` (selected fail-closed, `--quiet` + captured output)
5. Gameplay-harness full mode with one sample (`--mode full --samples 1`): exercises the narrative arcs, probes, and cross-branch contracts that smoke mode skips, in seconds
6. `cargo clippy --locked --lib --example gameplay_harness -- -D warnings`

Tests run before clippy so the hot test artifacts stay reused; clippy is last so lint failure still preserves test signal.

[`scripts/verify.ps1`](scripts/verify.ps1) is the owner; [`scripts/verify.cmd`](scripts/verify.cmd) is the wrapper. The smoke stage requires exactly one selectable ignored test; `.\scripts\verify.ps1 -SelfTest` checks that selection. [`tests/documentation_contracts.rs`](tests/documentation_contracts.rs) protects the authority set, local links, concrete routes, Cargo aliases, and published schema/content revisions.

The broad gate is not the routine finishing step. Ordinary library work completes with `.\scripts\verify.cmd -Fast`; harness work uses `.\scripts\verify.cmd -Fast -Harness`. Run the broad gate when persistence, invariants, cross-domain behavior, verification infrastructure, or another changed contract requires its wider harness/Clippy coverage, or for an explicit broad checkpoint. Do not run a fast lane and then the broad lane solely for reassurance. Use `-Jobs N` to cap parallelism, `-NoClippy` or `-NoFmt` only when those stages are known to pass. Pass `-Verbose` (alias for `-Detail`) to show cargo output on success.

Cargo profiles are tuned by measurement for this machine and crate (`debug = false`, `incremental = false`, `codegen-units = 32`); see the profile notes in [`Cargo.toml`](Cargo.toml) for the alternatives that lost. Panic messages keep file/line through the `Location` API without debuginfo; only RUST_BACKTRACE source lines degrade. A small library edit costs roughly ~7-16s to re-check and ~23-31s to rebuild+link tests (machine-load dependent); unchanged sources re-run in under a second. The verify script reports per-stage timing and a total, hides `cargo --quiet` output on success while showing full output on failure (or with `-Verbose` / `-Detail`), and pins `CARGO_INCREMENTAL=0`. If rebuilds ever feel pathological on Windows, exclude the repository `target\` directory from Defender real-time scanning.

When optimized compilation could change behavior, run `cargo test-release` (documented in the README); it is not part of any routine gate.

## Gameplay-harness evidence

[`examples/gameplay_harness/main.rs`](examples/gameplay_harness/main.rs) evaluates bounded deterministic policy treatments through production paths.

| Mode | Evidence |
| --- | --- |
| `smoke` | Fast canonical strategy and legal-foundation contract |
| `full` | Narrative, matched-seed, property/legal, opportunity, and bounded sensitivity evidence |

RUSH, PRESS, and RECON use the same seed-selected authored fixture and scenario timeline. Acting policy may use only organization/player-visible information, persisted reports/outcomes, and surfaced decision requests. Hidden investigation and evidence state is audit-only. Missing acting information or canonical rejection fails the run; missing events are observed absence.

`--samples` varies simulation/world seed and bounded timing offsets, range 1..=64. Matched branches share seed, fixture, and timeline. Per-run events and `RunMetrics` are raw evidence beneath aggregates; aggregates are not universal quality scores. Persisted artifacts retain per-run seeds and raw metrics. `full` mode writes per-run JSON to `--artifact-dir` (default `target/harness/`) with a `summary-<seed>.json`.

Structural and registry-aware validation runs at setup and observation boundaries, not on every tick. The harness checks the narrative defection-surveillance, second-wind, opportunity-prioritization, organizational-capacity, repeat-take depletion, district-diversification, and cross-branch financial contracts defined by its scenarios; changing those contracts requires updating this section and the focused harness tests together. Casing carries authored risk: a surveillance operation can draw trace-level exposure and open a case exactly like a burglary, so branch-heating evidence uses a session-wide staffed-case signal rather than the burglary's resolution record, and refused rival poaching pitches surface as player-visible loyalty reports counted as poach warnings. The capacity probe proves overlapping assignments are rejected atomically and become available after the prior operation reaches a terminal state. The repeat-take probe proves a successful take depletes the same target inside the production recency window and recovers after it. The narrative PRESS arc proves that standing-down time becomes governance: after the matched observation window closes, it revises its lieutenant's mandate to cover two districts, capitalizes a second-district float from idle street cash through a canonical ledger transfer, establishes a second gambling enterprise there, and ends the session with positive surcharge-free harbor earnings that an unheated branch never needs. The narrative RUSH arc proves failure teaches: a pre-entry police-arrival abort leaves the organization debrief-derived PoliceActivity knowledge as an abort artifact, and the rebuilt crew's second-score plan must carry that district-scoped record. Cross-branch financial evidence is window-honest: each branch snapshots cumulative finances at the shared campaign-day boundary before its arc extends (the PRESS arc deliberately waits out the authored cold-case window), and the contract asserts legitimate-business isolation, identical enterprise economics across unheated branches, and that an investigation-active branch never out-earns an unheated one over the same window. Narrative readouts quote the matched-window snapshot, not raw cumulative totals, when comparing branch economics.

`cargo harness` runs smoke by default. For explicit comparison:

```text
cargo harness-full --samples 8
cargo harness-full -- --samples 8 --artifact-dir target/my-run
```

## Completion checklist

Before handoff:

- Use the narrowest focused proof beforehand only when it provided useful iteration or failure-isolation feedback.
- Run exactly the smallest scripted completion lane that covers the changed surface: normally `.\scripts\verify.cmd -Fast`, or `.\scripts\verify.cmd -Fast -Harness` for harness work. Do not rerun an overlapping focused proof immediately before that lane merely because both commands are documented.
- Run `.\scripts\verify.cmd` only when the changed contract requires the broad gate.
- Run `cargo soak` or `cargo harness-full --samples 8` when the changed contract requires that evidence.
- Review `git diff --check`, generated output, and the final worktree.

When a test, harness mode, alias, or verification rule changes, update this document and the owning script or command definition in the same change.
