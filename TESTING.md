# Testing

Owns test selection, harness evidence, and local verification. Ownership is in
[`ARCHITECTURE.md`](ARCHITECTURE.md); scope is in [`STATUS.md`](STATUS.md);
cockpit routing is in [`AGENTS.md`](AGENTS.md).

## Test selection — narrowest proof first

```
Which change did you make?
  │
  ├─ Syntax / type error?
  │   Fastest: cargo check-fast              (~0.06s warm / 6s after edit)
  │            .\scripts\verify.cmd -Check   (~0.7s warm, includes fmt)
  │
  ├─ One library behavior (single module, single system)
  │   Focused: cargo test-focused <filter>         (~0.11s warm / 6-12s after edit)
  │   Complete: .\scripts\verify.cmd -Fast         (~0.7s warm)
  │
  ├─ Library implementation (no harness surface touched)
  │   Focused: cargo check-fast  or  cargo test-focused <filter>
  │   Complete: .\scripts\verify.cmd -Fast
  │
  ├─ Harness surface (examples/gameplay_harness/*.rs)
  │   Focused: cargo harness-rush  (~0.15s warm, incremental cache)
  │   Complete: .\scripts\verify.cmd -Fast -Harness
  │
  └─ Persistence, invariants, cross-domain, or verification infra
      Focused: owning module's focused test or load/continuation diagnosis
      Complete: .\scripts\verify.cmd               (~2-3s warm / 15-20s after edit)
```

| Change | Focused feedback | Completion lane |
|---|---|---|
| Syntax / types | `cargo check-fast` | `.\scripts\verify.cmd -Check` |
| One library behavior | `cargo test-focused <filter>` | `.\scripts\verify.cmd -Fast` |
| Library implementation | `cargo check-fast` or focused test | `.\scripts\verify.cmd -Fast` |
| Harness filter | `cargo harness-rush` | `.\scripts\verify.cmd -Fast -Harness` |
| Persistence, invariants, or cross-domain | Focused owner test | `.\scripts\verify.cmd` (broad gate) |

The columns are not a required sequence. Use focused feedback while iterating or
isolating a failure; go directly to the completion lane once it compiles and
exercises the same owner coverage. Never rerun the broad gate after a passing
fast lane "for reassurance".

## Test rules

Tests prove observable production behavior: calculations, transitions, transactions,
invariants, serialization, deterministic continuation, and failure paths.

- Exercise canonical system operations, not private helpers or test-only mutation shortcuts.
- Assert typed error variants and relevant fields, not rendered text.
- For atomic rejection, assert authoritative state is unchanged.
- Use explicit seeds and stable ordering. Do not hunt for a passing seed.
- Keep content-count, CRUD, or smokes only when they protect a real contract.

Ordinary tests live with their owning module under `#[cfg(test)]`, named after
behavior. Use `make_test_*` for local fixtures and the idempotent `*_for_test`
pattern when extending the shared production registry. Soak-class tests carry the
substring `soak` and are excluded from fast lanes with `--skip soak`, so renames
cannot silently un-exclude them; the invariant soak is stress evidence, not a
replacement for focused behavioral tests.

### Accretion checklist for a new test

- [ ] Calls the owner's `validate_* → commit` or `decide_* → apply_*`, not a private helper.
- [ ] Asserts the typed `Error` variant (e.g. `FinanceError::InsufficientFunds`) + fields on failure, not a string.
- [ ] On rejection, clones `state` before and `assert_eq!(state, before)` after.
- [ ] Uses `AppState::new(explicit_seed)` and `BTreeMap`/`BTreeSet` ordering — no `HashMap` iteration.
- [ ] Named `fn <behavior>_when_<condition>()`, not `fn test_crud()`.

## Local verification — solo, in this repo, no hosted CI

### Fast lanes — inner loop

| Need | Command | Warm (no change) | After touching one file | What it proves |
|---|---|---|---|---|
| Type-check lib | `cargo check-fast` | ~0.06s | ~6s | `src/` compiles |
| Type-check all | `cargo check-all` | ~0.45s | ~6s | lib + harness compile |
| Type-check harness | `cargo check-harness` | ~0.4s | ~3s | example adapter compiles |
| Lib tests (no soak) | `cargo test-fast` | ~0.11s | ~12s | 324 lib tests, `--skip soak` |
| One test / module | `cargo test-focused <filter>` | ~0.11s | ~6-12s | owning module's `#[cfg(test)]` |
| Auto-rerun on save | `.\scripts\watch.cmd` (`-Filter`, `-Harness`, `-Check`) | per-run | per-run | polls 120ms, debounce 300ms, watches `*.rs,*.toml,*.md` |
| Harness smoke, one strategy | `cargo harness-rush` / `-press` / `-recon` | ~0.15s | ~10-15s | one branch on `[profile.harness]` |
| Full-mode batch | `cargo harness-full --samples 8` | ~5s | ~15s | all strategies, matched seeds, artifacts |
| Check lane | `.\scripts\verify.cmd -Check` | ~0.7s | ~7s | fmt + type-check |
| Fast lane (fmt + lib) | `.\scripts\verify.cmd -Fast` | ~0.7s | ~13s | iteration gate |
| Fast harness lane | `.\scripts\verify.cmd -Fast -Harness` | ~0.7s | ~10s | smoke contract only |
| Filtered fast lane | `.\scripts\verify.cmd -Fast -Filter <pat>` | ~0.5s | ~6-12s | focused + fmt |
| Soak only | `cargo soak` | ~1s | ~13s | mixed-state invariant stress |

`cargo check-fast` is the absolute fastest; `cargo test-focused` is the inner loop
for behavior; `.\scripts\verify.cmd -Fast` is the iteration gate. The full gate
is reserved for persistence/invariant/cross-domain work.

**Why some lanes are slower after edits:** after touching one lib file, `cargo check`
recompiles that file's crate (~6s). Tests additionally link the test binary
(~12s). These are rustc costs, not script overhead. Warm runs with no changes
are near-instant because cargo's cache is reused. See `Cargo.toml` for the
profile tuning and measured alternatives that lost.

**Harness rebuild cost model (measured, see `Cargo.toml`):**

- All `harness*` aliases run on `[profile.harness]` (`target\harness\`): dev semantics at `opt-level 1`, never disturbing library caches in `target\debug\`.
- Example-only edits recompile in ~2-3s. A library edit pays one optimized lib rebuild: ~10-20s warm (incremental cache) vs ~75s cold.
- Dependencies compile at `opt-level 3` in every dev-derived profile and rebuild only on lockfile changes.
- The scripts pin `CARGO_INCREMENTAL=0` per stage, so an inherited value cannot override the profiles. Dev stages force `0`; harness stages clear it so `[profile.harness] incremental=true` governs.

### Broad completion gate — when cheap lanes are not enough

```text
.\scripts\verify.cmd
.\scripts\verify.cmd -Jobs 2
.\scripts\verify.cmd -Check          # type-check only, fastest gate
.\scripts\verify.cmd -Fast -Filter payroll   # one module
```

Fail-fast stages, in order (see [`scripts/verify.ps1`](scripts/verify.ps1)):

1. `cargo fmt --check`
2. `cargo test --locked --lib --tests --quiet`
3. Harness unit tests (`cargo test --locked --quiet --example gameplay_harness --lib`): the example's own options-parsing and financial-branch contract tests, which stage 2 never compiles
4. Exact ignored test `tests::smoke_mode_covers_canonical_paths` (selected fail-closed — `verify.ps1 -SelfTest` validates the count must be exactly 1)
5. Gameplay-harness full mode, one sample (`--mode full --samples 1`): narrative arcs, probes, and cross-branch contracts that smoke skips
6. `cargo clippy --locked --lib --example gameplay_harness -- -D warnings`

[`scripts/verify.ps1`](scripts/verify.ps1) owns the gate; [`scripts/verify.cmd`](scripts/verify.cmd) wraps it. The smoke stage requires exactly one selectable ignored test; `.\scripts\verify.ps1 -SelfTest` checks that selection. [`tests/documentation_contracts.rs`](tests/documentation_contracts.rs) protects the authority set, local links, concrete routes, Cargo aliases, and published schema/content revisions.

**When to run what:**

- Ordinary library work completes with `.\scripts\verify.cmd -Fast`; harness work with `.\scripts\verify.cmd -Fast -Harness`.
- Run the broad gate only when persistence, invariants, cross-domain behavior, verification infrastructure, or another changed contract requires its wider harness/Clippy coverage, or for an explicit broad checkpoint. Never rerun it after a passing fast lane merely for reassurance.
- Run `cargo soak` or `cargo harness-full --samples 8` only when the changed contract requires that evidence.
- When optimized compilation could change behavior, also run `cargo test-release`.

Gate flags: `-Check` (type-check only) | `-Fast` (skip soak/harness-full/clippy) | `-Harness` (smoke only, requires `-Fast`) | `-Filter <pat>` (one module, implies `-Fast`) | `-Jobs N` (cap parallelism) | `-NoClippy` / `-NoFmt` (skip known-passing) | `-Verbose` / `-Detail` (show cargo output on success). Profiles are tuned by measurement for this machine and crate; alternatives are noted in [`Cargo.toml`](Cargo.toml). If rebuilds feel pathological on Windows, exclude the repository `target\` directory from Defender real-time scanning.

## Gameplay-harness evidence — bounded evaluation surface

[`examples/gameplay_harness/main.rs`](examples/gameplay_harness/main.rs) evaluates bounded deterministic policy treatments through production paths. It is an evaluation surface, not a human-play test: it proves systemic behavior, not interface quality or comprehension.

### Modes

| Mode | Command | Evidence | Cost (warm) |
|---|---|---|---|
| `smoke` (default) | `cargo harness` | Canonical strategies + legal-foundation chain; sessions observe the whole first campaign day so recruitment counters carry real rival-attempt evidence | ~0.5s |
| focused smoke | `cargo harness-rush` / `-press` / `-recon` | One strategy branch only | ~0.15s |
| `full` | `cargo harness-full --samples 8` | Narrative strategy arcs, probes, matched-seed batches, scenario sensitivity, artifacts | ~5s |

Commands: `cargo harness` runs smoke by default; `cargo harness-full --samples 8` runs explicit comparison; append `--artifact-dir target/my-run` to relocate artifacts. `cargo harness -- --mode smoke --strategy press` selects one branch.

### Information boundary

RUSH, PRESS, and RECON act on the same seed-selected authored fixture and timeline. Acting policy may use only organization/player-visible information, persisted reports and outcomes, and surfaced decision requests. Hidden investigation and evidence state is audit-only (`[DEV AUDIT]`). Missing acting information or a canonical rejection fails the run; missing events are observed absence.

`--samples N` (range 1..=64) varies simulation/world seed and bounded timing offsets. Matched branches share seed, fixture, and timeline. Per-run events and `RunMetrics` are raw evidence beneath aggregates; aggregates are not quality scores. `full` mode writes per-run JSON (seeds and raw metrics) to `--artifact-dir` (default `target/harness-runs/`) plus a `summary-<seed>.json`. Structural validation runs at setup and observation boundaries, not every tick.

### Organic variation — not one replayed story

The harness must not replay one exact story, so evaluation-owned choices vary deterministically with the run seed:

- Full mode rotates its narrative comparison across `NARRATIVE_SEED_ROTATION` adjacent seeds, covering every authored fixture variation (economy profile, police presence, patrol windows, target names, till kind). Every set validates the full contract suite; the deep metrics and experience readout run on the primary seed, other sets print compact summaries. Batch samples continue to vary seed per sample.
- Authored-content-derived timing: scenario anchors come from authored operation durations, recruitment cadence, and the cold-case window; the terminal-wait guard's slack equals the longest authored operation duration, so it tracks content instead of a constant.
- Seed-derived policy jitter inside fixed branch identities: the witness-pressure delay after case opening, which rival the defector watch visits first, and which executive approach the win-back uses (`PersonalAppeal` or `FinancialOpportunity`; both outcomes are contract-honest).
- Authored fixture axes stay live: the racket till is street cash on even seeds and concealed cash on odd seeds, and the PRESS arc adapts to what its books actually hold (see the wealth-gate contract).

### Contracts — changing any requires updating this section + the harness tests

Narrative arcs (one shared fixture per comparison; each session closes with an organization view built from player-visible state):

- **Failure teaches (RUSH)** — a pre-entry police-arrival abort leaves debrief-derived district PoliceActivity knowledge, and the rebuilt crew's second-score plan must carry it.
- **Consequence arc (PRESS)** — a surfaced legal report seeds a precinct heat-check surveillance; standing down becomes governance where clean money allows it (street till: two-district mandate revision, float capitalization through a canonical ledger transfer, a second-district enterprise) or pure survival where it does not (concealed till), with daily contact polling until the channel itself carries the shelved read either way. The contact's case knowledge is production investigator state (`legal::case_knowledge`), never fixture-authored.
- **Own-heat loop (RECON)** — when its casing draws a case, the branch reads that case's activity through its standing institutional contact's canonical paths; a session whose casing drew no case must not fabricate a read.
- **Witness chain and counter-play** — the target is character-owned, so witnessed/identifying exposure names the owner as on-scene witness through canonical intake; institutional interviews convert his account into testimony; PRESS answers with exactly one WitnessPressure operation scheduled into the morning lull its crew field report identified. Two honest terminal shapes: registered cooperation degrades, or a police response forces a disciplined abort that leaves no second case. Failed pressure without degradation fails the run. Identifying exposures can escalate to autonomous member arrests (`player_member_arrests`); acting policy never reads case internals.
- **Personnel loop both ways** — after a departure, canonical surveillance confirms where the defector landed; leadership then makes exactly one executive win-back resolved by production recruitment scoring. Refusal surfaces the loyalty report that names our recruiter to the rival. Sessions without a departure attempt nothing.
- **Second wind** — all branches see the reopened second-score opportunity; RUSH works it with the rebuilt crew, RECON re-recons first, PRESS lets it lapse as the price of standing down.
- **Standing feedback** — witnessed or successful operations surface Notable `Standing` reports; a racket drawing a vice inquiry raises police fear the same way.
- **Routine continuity** — legitimate-front economics continue identically while leadership handles exceptions.

Probes:

- **Opportunity prioritization** — strongest player-visible source converts; weaker expires with its report; a decoy dismisses through the canonical lifecycle.
- **Organizational capacity** — overlapping specialist assignments reject atomically with a typed error and unchanged state; the specialist releases after the prior operation reaches terminal; mandate revision advances version.
- **Repeat-take depletion** — an immediate re-score recovers half the take; a rested re-score returns full value.
- **Vice-attention conversion** — organic hits are probabilistic (per-cycle authored rates multiplied by active originated district cases), so batches count them (`vice_inquiries_drawn`) and focused `enterprise_execution` tests prove hit, compounding, organization-facing knowledge, and post-shelving release; full mode additionally runs a deterministic probe: a clean-district control cycle must draw nothing, then enough parallel district cases opened through canonical incident intake push the rate to certainty, and the next cycle must pay the compounded street surcharge, open a dedicated inquiry on the racket as an active originated case, and surface it as organization-held legal knowledge.
- **Legal foundation** — arrest → paid counsel → custody-preserving prosecution referral → terminal decline → named cooperative witness intimidated through canonical pressure with mandatory cooperation degradation.

Cross-cutting contracts:

- **Matched-window financial honesty** — each branch snapshots cumulative finances at the shared campaign-day boundary before its arc extends; the contract asserts identical legitimate income everywhere, identical enterprise economics across unheated branches, and that a branch with a staffed case (casing counts) never out-earns an unheated one over the same window. Readouts quote matched snapshots, not raw totals.
- **Money states** — liquidation proceeds stay dirty street cash until laundered through an owned cash-intensive front; the front's per-cycle plausibility ceiling visibly rejects the over-capacity remainder; accounted funds reconcile exactly as gross minus the front's fee minus acquisition spend.
- **Legitimate-wealth gate (primary set)** — on the primary narrative seed the strict chain is required: PRESS launders the racket's street-cash float day by day until its accounted books cover the venue's authored price, attempts the purchase once while short (canonical rejection evidence), buys at the authored kind price through `economy::business_acquisition` (ownership transfer plus first operating economy plus full accounted payment, surfaced as a Notable report), establishes the second-district racket there (ownership being the production prerequisite for hosting), and the new book settles positive cycles before the final financial view. Rotated sets run the same paths on worlds whose authored economics may honestly stall conversion - a concealed till whose money cannot touch a front's ledgers, or a young racket whose heat-taxed float never accumulates - and require only an honest ending: the cooled read confirmed, with any purchase that did occur carrying the full consistent chain. The family treasury is never swept; wages come from it alone.
- **Payroll** — every session crossing a campaign-day boundary meets payroll through the canonical ledger path; totals are raw evidence in metrics and the financial view.
- **Governed rival world** — the Rosetti organization expands home rackets identically across matched branches unless branch actions heat the shared district (police fear throttles expansion while it decays); observed audit-only via `rival_home_enterprises`.
- **Manager reports are held information** — narration quotes only this organization's notable cycles; a street-heat surcharge is reported when it appears or changes, not on every settled cycle.
- **Casing risk symmetry** — surveillance can draw trace-level exposure and open a case exactly like a burglary, so heating signals are session-wide rather than tied to the burglary's resolution record, and refused poaching pitches surface as player-visible loyalty reports counted as poach warnings.

## Completion checklist

Before handoff:

- [ ] Ran exactly the smallest scripted completion lane that covers the changed surface (see decision tree above); did not rerun an overlapping focused proof immediately before that lane.
- [ ] `git diff --check` is clean; generated output and final worktree reviewed.
- [ ] If a test, harness mode, alias, or verification rule changed, this document and the owning script/command definition were updated in the same change.

When a test, harness mode, alias, or verification rule changes, update this document and the owning script or command definition in the same change.
