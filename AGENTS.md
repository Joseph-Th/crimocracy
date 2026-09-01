# Agent Guide — Crimocracy Cockpit

**BCA policy:** advisory

This is the single cockpit for any agent driving this system. It compresses every
authority into one scannable path so you can orient, locate the canonical mutation,
and verify with the least expenditure of resources. Ownership contracts are in
[`ARCHITECTURE.md`](ARCHITECTURE.md); scope is in [`STATUS.md`](STATUS.md);
evidence rules are in [`TESTING.md`](TESTING.md); intent is in
[`GAME_DESIGN.md`](GAME_DESIGN.md); commands are in [`README.md`](README.md).

---

## 1. At a glance — 30 seconds

```text
Registry (immutable, build_registry)  ─┐
AppState  (mutable, 15 substates + 5 RNG streams + clocks) ─┤─► run_tick (1 min, 14 phases, deterministic)
Harness   (evaluation surface, smoke/full, player-visible only) ─┘
```

- **You mutate through one owner per field** via `validate_* → Validated*::commit` or
  `decide_* → apply_*`. Never construct a `*Record{}` directly.
- **One tick = one minute** (`core::simulation::run_tick` at `src/core/simulation.rs`).
  Speed is an adapter concern — call it more often, don't change its semantics.
- **Determinism** = `Registry + AppState + ordered inputs + state-owned RNG`. No wall
  clock, no hash iteration, no ambient entropy.
- **Cheapest proof:** `cargo check-fast` (0.06s warm) → `cargo test-focused <filter>` (0.11s warm)
  → `.\scripts\verify.cmd -Fast` (0.7s warm) → `.\scripts\verify.cmd` (2-3s warm) only for
  persistence/invariant/cross-domain work.

---

## 2. System tower — the single mental model

The codebase is a tower of linked abstractions, not a bag of modules. Every layer
depends only on layers below it.

```text
Layer 4 — Orchestrating domains (transact across many owners)
  operations  legal  enterprises  economy
       \        |        |         /
        \       |        |        /
Layer 3 — Mid domains (cross-reference, own mutations)
  finance  delegation  reputation  decisions  contacts  opportunities  recruitment
              \         |            |           |           |            |
Layer 2 — Leaf / low-dependency domains (no cross-domain writes)
  world  social  intelligence  history  reports
              \    |        |         |       /
Layer 1 — Immutable authoring
  registry  ◄──  content::build_registry  (CURRENT_CONTENT_REVISION = 35)
               \
Layer 0 — Foundations (everyone depends on these)
  core::{id, time, entity, attention, state, simulation, persistence, invariants}
```

**Dependency DAG (no cycles):**

```text
core/id,time,attention,entity ──► every domain
world ──► social, intelligence, delegation(policy), economy, enterprises, operations(rating)
finance ──► delegation, enterprises, economy, operations, world/contacts via ledger
delegation ──► recruitment, enterprises (MandateAuthority)
intelligence ──► operations, contacts, recruitment, legal, reports
social ──► recruitment
legal ──► operations(police_response, investigation) ◄──► enterprises(vice inquiries)
registry/content ──► everything reads it; nothing writes it after build_registry
AppState (state.rs) owns all 15 substates; simulation.rs orchestrates all
```

**File inventory:** `src/lib.rs` enumerates all 20 modules — start there.

### AppState ownership cockpit — 15 substates

| # | Field `state.*` | Owns | Canonical mutation | File |
|---|---|---|---|---|
| 1 | `world` | orgs, characters, neighborhoods, businesses, designation, payroll | `world_system`, `payroll_execution`, `territory_influence` (read-only) | `src/world/` |
| 2 | `social` | directional relationships | `relationship_system` | `src/social/` |
| 3 | `intelligence` | provenance-bearing information | `intelligence_system` | `src/intelligence/` |
| 4 | `reports` | player-facing reports & briefs | `report_system` | `src/reports/` |
| 5 | `history` | entity-linked campaign events | `history_system` | `src/history/` |
| 6 | `finance` | accounts, balanced ledger, laundering | `finance_system` (`validate_launder_funds`) | `src/finance/` |
| 7 | `operations` | plans, execution, property/cash/property disposition | `operation_system`, `operation_execution`, `operation_economics` | `src/operations/` |
| 8 | `opportunities` | provenance-backed opportunities | `opportunity_system` | `src/opportunities/` |
| 9 | `decisions` | typed decision requests | `decision_system` | `src/decisions/` |
| 10 | `delegation` | mandates & responsibility scopes | `delegation_system` | `src/delegation/` |
| 11 | `economy` | business economies, sabotage, suspension | `business_economy_system`, `business_acquisition` | `src/economy/` |
| 12 | `enterprises` | rackets, cycles, vice-attention, expansion | `enterprise_execution`, `autonomous_expansion` | `src/enterprises/` |
| 13 | `legal` | jurisdictions, patrols, investigations, evidence, arrests, custody, representation, prosecution, witnesses, informants | `jurisdiction_system`, `patrol_system`, `investigation_system`, `arrest_system`, … via `legal_state` | `src/legal/` |
| 14 | `contacts` | institutional contacts & disclosures | `contact_system` | `src/contacts/` |
| 15 | `recruitment` | recruitment, cooldowns, approvals, scoring | `recruitment_system`, `scoring` | `src/recruitment/` |
| — | `reputation` | per-audience standing (Fear/Reliability/Competence/Treachery) | `reputation_system::apply_reputation_delta` (single score path) | `src/reputation/` |

Persistence envelope: `src/core/persistence.rs` (`SaveEnvelope { format_version:1, content_revision, state }`);
ID allocator: `src/core/id.rs` (31 kinds, `IdCounters`, `reserve` before multi-record commits).

---

## 3. Canonical operations — quick reference

Every consequential mutation follows one of two shapes. Treat the `//!` header of
each `src/*/mod.rs` as contractual — its second line names the owning system.

### Pattern A — Validate then commit (fallible multi-record)

```text
validate_*(&state, …) -> Result<Validated*, TypedError>
Validated*::commit(self, &mut state) -> Result<Outcome, TypedError>
```

Validation resolves references, permissions, lifecycle, ownership, capacity, ranges,
and arithmetic **before** any mutation. Commit rechecks freshness (`SimTime` staleness
via `ensure_time_current`) and preserves indexes atomically. Rejected operations leave
authoritative state unchanged.

**Where to find them:**

| Domain | Validate entry point | Commit via | Typed error |
|---|---|---|---|
| world | `world_system::validate_insert_*` `validate_designate_player_organization` | `Validated*::commit` | `WorldError` |
| finance | `finance_system::validate_record_transaction` `validate_launder_funds` `validate_insert_account` | `commit` | `FinanceError` |
| operations | `operation_system::validate_authorize_operation` `operation_execution::validate_operation_resolution_plan` `property_disposition::validate_dispose_property` | `commit` | `OperationError` |
| economy | `business_acquisition::validate_acquire_business` `business_economy_system::validate_*` | `commit` | `BusinessError` |
| legal | `investigation_system::validate_open_investigation` `arrest_system::validate_arrest` `legal_representation_system::validate_retain_legal_representation` | `commit` | `LegalError` |
| contacts | `contact_system::validate_establish_contact` `validate_contact_disclosure` | `commit` | `ContactError` |
| delegation | `delegation_system::validate_assign_mandate` `validate_revise_mandate` | `commit` | `DelegationError` |
| recruitment | `recruitment_system::validate_recruitment_attempt` | `commit` | `RecruitmentError` |

### Pattern B — Decide then apply (read-only derivation)

```text
decide_*(&state, …) -> Plan / Outcome / Delta    // read-only, may take &mut RNG
apply_*(&mut state, plan)                        // single-owner, preserves invariants
```

Decision reads broader state than it mutates. Randomness is explicitly supplied
(`&mut ChaCha8Rng`) and drawn via `core::simulation::draw_index`.

**Examples:** `decide_operation_resolution` → `validate_operation_resolution_plan → commit`;
`decide_business_cycle` → `validate_business_cycle_plan → commit`;
`decide_enterprise_cycle` → `validate_enterprise_cycle_plan → commit`;
`decide_executive_brief` → `validate_executive_brief_plan → commit`.

### Single-owner direct mutation

Allowed only when every return path preserves that owner's invariants and indexes
(`social::relationship_system::set_relationship`, `reputation_system::apply_reputation_delta`).

**Rule:** adapters, tests, examples, importers, and tools use the same owner methods.
No bypasses. If you are constructing a `*Record` literal, stop — use the owner's
`Draft → validate → commit` path.

---

## 4. Change recipe — 7 concrete steps

1. **Locate the owner.** Find the `AppState` field (`src/core/state.rs`) and the
   `ARCHITECTURE.md` source map. Read the `//!` header of that `src/*/mod.rs`.
2. **Read the focused tests.** Open `src/<owner>/tests.rs` or `#[cfg(test)]` in the
   owning module. They are the executable contract.
3. **Name the canonical operation.** Search for `validate_*` or `decide_*` in the
   owner's `*_system.rs`. Do not invent `create_*`/`make_*`/`execute_*`.
4. **Implement through the owner.** Validate before mutation; commit atomically;
   maintain every derived index (`BTreeMap` + `BTreeSet`) and bump `version` where
   present. Handle the project's enums exhaustively.
5. **Preserve determinism.** Use `BTreeMap`/`BTreeSet` or explicit stable sorting
   with tie-breakers. Draw randomness only from `state.operation_rng_mut()` /
   `investigation_rng_mut()` / `business_rng_mut()` / `enterprise_rng_mut()` /
   `recruitment_rng_mut()` via `draw_index`.
6. **Preserve persistence.** Every future-affecting value must survive `build_save` →
   `restore_save` (`src/core/persistence.rs`). Add `#[derive(Serialize,Deserialize)]`
   and a round-trip test if you add state.
7. **Prove it with the narrowest lane** (see §6), then run the broader completion lane.
   Update the single authority document whose contract you changed.

If any step is not discoverable, repair the owning documentation as part of the change.

---

## 5. Verification lanes — which command when

### Decision tree

```text
Did you touch persistence, invariants, cross-domain behavior, or verification infra?
  ├─ YES → broad gate:  .\scripts\verify.cmd              (2-3s warm)
  └─ NO ─┬─ Did you touch harness surface (examples/gameplay_harness)?
          │   ├─ YES → fast harness lane: .\scripts\verify.cmd -Fast -Harness  (0.7s warm)
          │   └─ NO ─┬─ Single behavior? → cargo test-focused <filter>          (0.11s warm)
          │           └─ Library work?   → .\scripts\verify.cmd -Fast           (0.7s warm)
          │                              cargo test-fast                         (0.11s warm)
          │                              cargo check-fast                        (0.06s warm)
```

**Never rerun the broad gate after a passing fast lane “for reassurance”.**

| Need | Command | Warm (no change) | What it proves |
|---|---|---|---|
| Type-check library | `cargo check-fast` | ~0.06s | `src/` compiles |
| Type-check all | `cargo check-all` | ~0.45s | `src/` + harness surface |
| One focused test | `cargo test-focused <filter>` | ~0.11s | owning module's `#[cfg(test)]` |
| Fast lib tests (no soak) | `cargo test-fast` | ~0.11s | all lib, `--skip soak` (324 tests) |
| Auto-rerun on save | `.\scripts\watch.cmd [-Filter <p> \| -Harness \| -Check]` | per lane | polls 120ms, debounce 300ms, watches `*.rs,*.toml,*.md` |
| Harness smoke, one branch | `cargo harness-rush` / `-press` / `-recon` | ~0.15s | one strategy on `[profile.harness]` |
| Harness smoke, all | `cargo harness` | ~0.5s | all 3 strategies + legal foundation |
| Full comparison batch | `cargo harness-full --samples 8` | ~5s | all strategies, matched seeds, artifacts in `target/harness-runs/` |
| Check lane (fmt + check) | `.\scripts\verify.cmd -Check` | ~0.7s | type-check only, fastest gate |
| Fast lane (fmt + lib) | `.\scripts\verify.cmd -Fast` | ~0.7s | iteration gate |
| Fast harness lane | `.\scripts\verify.cmd -Fast -Harness` | ~0.7s | smoke contract only |
| Broad local gate | `.\scripts\verify.cmd` | ~2-3s | **fmt → lib+integration → harness units → smoke (fail-closed) → full n=1 → clippy** |

**Watch your lanes:** `tests/documentation_contracts.rs` guards alias names, doc links,
and `STATUS.md ↔ CURRENT_STATE_SCHEMA_VERSION (66) / CURRENT_CONTENT_REVISION (35)`
agreement — they run in stage 2/3.

---

## 6. Determinism · persistence · invariants — checklist

Before handoff, every change that touches state must satisfy all three:

- [ ] **Determinism.** No `HashMap`/`HashSet` iteration, no `SystemTime`, no filesystem
  iteration, no thread scheduling as input. Ordered collections or explicit stable
  sort + tie-breakers. All result-affecting randomness from the 5 state-owned
  `ChaCha8Rng` streams (`src/core/state.rs`, `src/core/simulation.rs`).

  | Stream | Used for | Draw helper |
  |---|---|---|
  | `operation_rng` | operation execution & exposure variance | `draw_signed_variance(limit)` |
  | `investigation_rng` | investigation-work variance | `draw_signed_variance(limit)` |
  | `business_rng` | business cycle gross variance | `draw_basis_point_variance(limit)` |
  | `enterprise_rng` | enterprise gross variance + vice-attention roll (both unconditionally per cycle) | `draw_basis_point_variance` + `draw_index(bound=10000)` |
  | `recruitment_rng` | recruitment scoring variance | `draw_index` |

- [ ] **Persistence.** Every future-affecting value is serialized. Save/load path:
  `build_save(registry, state)` validates `validate_state` + `validate_state_against_registry`
  before cloning into `SaveEnvelope`; `restore_save` validates format, schema
  (`CURRENT_STATE_SCHEMA_VERSION`), content revision, indexes, and high-water marks
  (`src/core/persistence.rs`). Adding a field? Derive `Serialize/Deserialize` and
  add a round-trip test. Derived indexes are never persisted — rebuild from records.

- [ ] **Invariants.** `validate_state(state)` checks ID allocators (no 0, next > max),
  index consistency (every domain `has_consistent_indexes()`), existence of typed
  references, lifecycle agreement, and future timestamps. `validate_state_against_registry`
  re-derives authored-content-dependent values (margins, exposure, proceeds, schedules).
  `run_tick` calls `validate_invariants` at `src/core/simulation.rs` after every
  14-phase minute. The soak (`cargo soak` / `--skip soak`) exercises mixed state
  under full invariant validation.

---

## 7. Runtime flow — the 14-phase tick

`core::simulation::run_tick` (`src/core/simulation.rs`) is the **only**
authoritative minute. Phase order is contractual — comments explain “runs after X so…”.

```text
 1  apply_opportunity_expiry               (durable lifecycle report before consumers)
 2  run_operations_phase                   (start → deadline aborts → police arrivals → resolution)
 3  apply_autonomous_investigator_staffing
 4  apply_initial_evidence_reviews
 5  apply_witness_interview_scheduling
 6  run_investigation_work_phase           (resolve due work with pre-drawn variance)
 7  apply_autonomous_evidence_arrests      (threshold: 2 independent evidence items)
 8  apply_detainee_informant_recruitment   (single decision at now-1440)
 9  apply_informant_disclosures
10  apply_automatic_legal_support           (policy → retention via canonical path)
11  apply_cold_case_decay                  (only originated cases; window = 10080 min)
12  run_business_cycle_phase               (gross variance per due economy)
13  run_enterprise_cycle_phase             (gross variance + vice roll unconditionally)
14  apply_daily_payroll  →  apply_due_autonomous_recruitment  →  apply_due_autonomous_enterprises
    ──► apply_reputation_phase (decay first, then operation + vice consequences)
    ──► synthesize_executive_brief (sees everything above, last)
    ──► validate_invariants
```

New autonomous work must slot explicitly here with a rationale comment matching the
existing ones at `src/core/simulation.rs`.

---

## 8. Harness — bounded evaluation surface (not a playtest)

`examples/gameplay_harness/` exercises **production paths** through player-visible
information only. `[DEV AUDIT]` is diagnostic, never fed to decisions.

| Mode | Command | What it proves |
|---|---|---|
| `smoke` (default) | `cargo harness` | 3 strategies (`Rush/Press/Recon`) + legal foundation on one seed; 1 campaign day |
| focused smoke | `cargo harness-rush` / `-press` / `-recon` | single strategy branch |
| `full` (calibration) | `cargo harness-full --samples 8` | narrative rotation (`NARRATIVE_SEED_ROTATION=3`, covers Clockwork/Crowded/Quiet), 4 probes, batch sensitivity, artifacts |

Full mode artifacts: `target/harness-runs/*.json` per run + `summary-<seed>.json`.
Matched-seed branches share fixture, timeline, and seed — use `RunMetrics` and
`validate_branch_financial_isolation` (enterprise heat never lets a cased branch
out-earn an unheated one over the shared window). Timing anchors derive from
`content::build_registry()` (operation durations, recruitment cadence, cold window,
longest operation duration as terminal-wait slack), not constants.

**Do not** infer safe operation windows from vague patrol text — use
`choose_safe_start_from_patrol_report` with actionable windows only.

---

## 9. Common pitfalls — fail fast, not silently

| # | Anti-pattern | Why it breaks | Correct path |
|---|---|---|---|
| 1 | Constructing `*Record{}` or patching a private field | Skips validation, indexes, version bumps; `validate_state` fails | Use `*Draft → validate_* → commit` via the owning `*_system` |
| 2 | `HashMap`/`HashSet` or unsorted `Vec` for order-sensitive work | Nondeterministic iteration → flaky `soak` and harness divergence | `BTreeMap`/`BTreeSet` or explicit stable sort + tie-breaker |
| 3 | `rand::thread_rng()` or `SystemTime::now()` | Ambient entropy leaks into authoritative state | `state.<domain>_rng_mut()` + `draw_index` (rejection sampling) |
| 4 | Adding a record but forgetting its schedule/index | Invisible to `run_tick`; leaks onto revoked/suspended work | Follow the 3-part pattern: `BTreeMap::insert` + derived `BTreeSet` + `has_consistent_indexes` (see `src/finance/mod.rs`) |
| 5 | Forgetting `IdCounters::reserve` before multi-record commits | Allocator high-water mark drifts, `validate_id_allocators` fails on next save | Call `state.ids.reserve(…)` before any `validate_*` that may allocate |
| 6 | Consuming RNG conditionally (e.g. vice roll only when vice inquiry) | Branches needing matched determinism diverge | Draw unconditionally per cycle (see `src/core/simulation.rs`) |
| 7 | Writing ledger `balance` directly | Balance is a materialized view; ledger `postings` are the truth | `finance_system::validate_record_transaction` derives balances; audit re-derives via dense `Vec<i64>` at `src/core/invariants/mod.rs` |
| 8 | Using display text as identity | Fragile foreign keys, collisions | Typed IDs (`CharacterId`, `BusinessId`) + `EntityRef` where project controls vocabulary |
| 9 | Silently defaulting a missing future-affecting value on load | Old save loads but loses continuation fidelity | `CURRENT_STATE_SCHEMA_VERSION` / `CURRENT_CONTENT_REVISION` mismatch is `LoadError`; compat is current-version only (`STATUS.md:61`) |
| 10 | `cargo test` without `--locked` or with inherited `CARGO_INCREMENTAL=1` | Defeats `[profile.dev] incremental=false` (measured +30-180s) or `[profile.harness] incremental=true` (revert to 75s) | Use aliases or `verify.ps1` — it pins `CARGO_INCREMENTAL=0` for dev stages and clears it for harness stages |

---

## 10. Accretion guide — adding without fragmenting

**New domain** (`src/<newdomain>/`):

1. Define `NewDomainState` with `records: BTreeMap<NewId, NewRecord>` + derived
   `BTreeMap<Key, BTreeSet<Id>>` indexes. Own the `IdKind::NewKind` counter.
2. Create `newdomain_system.rs` with `validate_* → Validated*::commit` following
   `finance_system` as a template. Keep `has_consistent_indexes()` exhaustive.
3. Register the substate in `AppState` (`src/core/state.rs`), wire it into
   `validate_state` (`src/core/invariants/mod.rs`), and allocate an RNG stream
   if result-affecting randomness is needed.
4. Slot any autonomous work into `run_tick`'s phase order with a rationale comment.
5. Add focused tests under `#[cfg(test)]` named after behavior (not “CRUD smoke”).
   Add a `soak`-substring stress test only if it exercises mixed state.
6. Update `ARCHITECTURE.md` source map and this guide's §2/§3 tables.

**New authored kind** (e.g. `OperationKind::Arson`):

1. Add the variant to `operations/mod.rs` / `registry` vocabulary.
2. Author its economics in `content::build_registry()` (`src/content/mod.rs`).
3. Ensure `validate_state_against_registry` re-derives every authored-dependent value.
4. Verify `MATCH` is exhaustive — the project forbids wildcards on owned enums (`clippy::wildcard_enum_match_arm = deny`).

**New test:**

- Exercise the canonical operation, assert the typed `Error` variant + fields, and
  for rejections assert authoritative state is unchanged. Use explicit seeds and
  stable ordering — never hunt for a passing seed.

---

## 11. Authority map & reading order

| Question | Authority |
|---|---|
| Repository and collaboration rules | Workspace `AGENTS.md` (if present) |
| Project execution rules | **This file** (cockpit) |
| State ownership and mutation | [`ARCHITECTURE.md`](ARCHITECTURE.md) |
| Implemented scope and exclusions | [`STATUS.md`](STATUS.md) |
| Tests and harness evidence | [`TESTING.md`](TESTING.md) |
| Product intent | [`GAME_DESIGN.md`](GAME_DESIGN.md) |
| Commands and local gate | [`README.md`](README.md) and [`TESTING.md`](TESTING.md) |
| Executable behavior | Owning `src/` module and its focused tests |

**Cold start — read in order:**

1. If the repo lives in a portfolio workspace, read the workspace `../AGENTS.md` first.
2. This file (§1-§5 for the working loop; §6-§10 as needed).
3. [`STATUS.md`](STATUS.md) — what exists and what is explicitly excluded.
4. [`ARCHITECTURE.md`](ARCHITECTURE.md) — contracts, then the `//!` header of the
   owning `src/` module and its focused tests before editing.
5. [`TESTING.md`](TESTING.md) before changing tests, persistence, or harness behavior.
6. [`GAME_DESIGN.md`](GAME_DESIGN.md) only for product-intent questions.

If authorities conflict, the owning contract and implementation win. Repair stale
wording as part of the change.

## Non-negotiable rules

- One owner per consequential state field; mutate only through the canonical production path.
- Tests, examples, adapters, importers, and tools use owner methods. No bypasses or mutation shortcuts.
- Validate fallible multi-record operations before mutation. Rejected operations leave authoritative state unchanged unless the contract explicitly records a failure.
- Keep ordering and randomness deterministic: ordered collections or explicit stable sorting with tie-breakers, state-owned RNG only.
- Persist every future-affecting runtime value; cover with invariant, load, and continuation checks where applicable.
- Handle project-owned enums exhaustively; use typed error enums for new fallible operations.
- Keep external effects (filesystem, network, UI, process) behind explicit adapter boundaries.
- Delete superseded paths. Do not keep historical shims.
- Keep documentation concise, current, and forward-facing. No implementation diaries.
- Verification is local. Do not add or depend on GitHub Actions.

## Completion

Use focused checks while editing when they shorten feedback or isolate a failure. For completion, run the smallest lane that covers the changed surface; if the implementation is already ready for that lane, go directly to it instead of forcing a focused build first:

- `cargo check-fast` / `cargo test-focused <filter>` while editing
- `.\scripts\verify.cmd -Fast` for ordinary library work
- `.\scripts\verify.cmd -Fast -Harness` when the harness surface changes
- `.\scripts\verify.cmd` only for persistence/invariant/cross-domain work, verification infrastructure, or an explicit broad checkpoint

Before handoff, confirm ownership, determinism, persistence, invariants, adapters, tests, documentation, and worktree scope remain coherent.
