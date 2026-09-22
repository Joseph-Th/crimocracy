# Architecture

Owns ownership, mutation, determinism, persistence, and invariant contracts.
Scope is in [`STATUS.md`](STATUS.md); verification is in [`TESTING.md`](TESTING.md);
intent is in [`GAME_DESIGN.md`](GAME_DESIGN.md); the agent cockpit is in
[`AGENTS.md`](AGENTS.md) — start there for orientation, then come here for
contracts.

## System tower

The codebase is organized as one tower of ownership and orchestration responsibilities.
The layer diagram is not a literal Rust import graph: lower-level owners expose canonical
state APIs, while cross-domain systems coordinate through those APIs and orchestrating domains
may depend on peer domains for explicitly modeled integrations. Mutation ownership still flows
through one canonical owner per consequential field.

```text
                  ┌─────────────────────────────────────────────┐
                  │  Layer 4 — Orchestrating domains            │
                  │  operations · legal · enterprises · economy  │
                  │  (transact across many owners atomically)   │
                  ├─────────────────────────────────────────────┤
                  │  Layer 3 — Mid domains                      │
                  │  finance · delegation · reputation          │
                  │  decisions · contacts · opportunities       │
                  │  recruitment  (cross-ref, own mutations)    │
                  ├─────────────────────────────────────────────┤
                  │  Layer 2 — Foundational domain owners       │
                  │  world · social · intelligence · history    │
                  │  reports  (low-dependency state)            │
                  ├─────────────────────────────────────────────┤
                  │  Layer 1 — Immutable authoring              │
                  │  registry ◄── content::build_registry       │
                  │  content::CURRENT_CONTENT_REVISION          │
                  ├─────────────────────────────────────────────┤
                  │  Layer 0 — Foundations                      │
                  │  core::{id,time,entity,attention,           │
                  │         state,simulation,persistence,       │
                  │         invariants}                         │
                  └─────────────────────────────────────────────┘
                          AppState owns all domain state
                          Registry is read-only after build
                          run_tick (1 min) orchestrates all layers
```

**Major dependency relationships and intentional peer integrations:**

```text
core/id,time,attention,entity ──► every domain
world ──► social, intelligence, delegation(policy), economy, enterprises, operations(rating)
finance ──► delegation, enterprises, economy, operations, world/contacts via ledger
delegation ──► recruitment, enterprises (MandateAuthority)
intelligence ──► operations, contacts, recruitment, legal, reports
social ──► recruitment
legal ◄──► operations (police_response, investigation origination)
legal ◄──► enterprises (vice inquiries)
registry/content ──► everything reads it; nothing writes it after build_registry
AppState (state.rs) owns all; simulation.rs orchestrates all
```

This relationship map is intentionally not a DAG. The `legal`/`operations` and
`legal`/`enterprises` pairs exchange read-only context and validated orchestration calls where
crime creates legal consequences and legal pressure affects criminal activity. They do not share
mutation ownership: each consequential record remains private to its owning domain, and
cross-domain commits still pass through canonical owner APIs.

File inventory: `src/lib.rs` is the authoritative top-level module list. `src/core/state.rs` is the
cross-domain aggregate — any cross-domain question starts by finding which `AppState` field
owns it. Each `src/*/mod.rs` `//!` header names the canonical mutation path.

## Program model

- **Registry** — immutable authored definitions and validated lookup tables.
- **AppState** — serializable mutable campaign state. Typed IDs, clocks, and 4 state-owned RNG streams.
- **Records** — typed identity, lifecycle, references, and version state.
- **Systems** — validate requests, derive decisions, commit authoritative mutation, preserve invariants.
- **Indexes and projections** — derived views maintained from authoritative records. Never independent truth.
- **Adapters** — own filesystem, process, network, and UI effects. Core systems receive explicit data and return explicit outcomes.

Static definitions describe what may exist. Runtime records describe what does exist.
Mutable progress does not live in the registry. Future-affecting generated state is
persisted, not reconstructed.

## Runtime flow — the authoritative tick

`AppState::new(seed)` + `content::build_registry()` are the only constructors.
After that, every minute advances through one contractual pipeline:

```text
 1  content::build_registry          validated immutable registry (content::CURRENT_CONTENT_REVISION)
 2  AppState::new(seed)              serializable state, 4 ChaCha8Rng streams, SimTime::ZERO
 3  validate_* / decide_*            domain system validates or derives read-only plan
 4  Validated*::commit / apply_*      owning system commits atomically, preserves indexes
 5  core::simulation::run_tick        preflight finite clock, then one minute in stable contractual order
 6  Result<TickOutcome, TickError>    typed terminal-clock rejection or player-visible consequences
 7  build_save / restore_save         envelope {format_version, content_revision, state}
```

Tick cadence is an adapter concern. Calling `run_tick` faster or slower changes
wall time, not the semantics of one canonical minute. Clock exhaustion is checked before
the first mutation, so the terminal representable minute is a valid state with no successor tick.

### run_tick — contractual order (`src/core/simulation.rs`)

Phase order is the coupling contract. Comments at `src/core/simulation.rs`
explain each “runs after X so Y is visible” dependency. Reordering breaks
determinism and harness contracts.

```text
 0  checked next minute                    reject ClockExhausted before mutation
 1  apply_due_custody_releases             hard arrest-custody boundary before same-minute consumers
 2  apply_opportunity_expiry               durable lifecycle report before remaining same-minute consumers
 3  run_operations_phase                   police arrivals → starts → deadline cleanup → resolution
 4    ├─ apply_due_police_response_arrivals (exposure → decisions; must precede new starts)
 5    ├─ find_due_authorized → Begin or deadline-missed
 6    ├─ find_due_with_missed_deadlines → abort via decision when present
 7    └─ find_due_in_progress → decide+validate+commit per operation (RNG: operation stream)
 8  apply_autonomous_investigator_staffing single-seat staffing, lead-investigator knowledge
 9  apply_evidence_review_scheduling       next unattempted reviewable evidence on active staffed cases
10  apply_witness_interview_scheduling     after reviews so same-minute witness is interviewable
11  run_investigation_work_phase           resolve due work (RNG: investigation stream)
12  apply_autonomous_evidence_arrests      active LawEnforcement cases: authored independent-evidence threshold → custody + responsibility preemption
13  apply_autonomous_prosecution_staffing  refill prosecution seats released by custody
14  apply_automatic_legal_support          conclude boundary releases; retain before a due detainee decision
15  apply_detainee_informant_recruitment   one decision after a delay; active counsel lowers authored flip chance
16  apply_informant_disclosures            holder-knowledge → handler cases
17  apply_cold_case_decay                  originated cases only, authored inactivity window, no RNG
18  run_business_cycle_phase               per due business (RNG: business stream)
19  apply_due_autonomous_business_lifecycle
                                                suspended non-player books reopen only when current zero-variance economics recover
20  run_enterprise_cycle_phase             per due enterprise (RNG: enterprise stream, 2 draws unconditionally)
21  apply_daily_payroll  →  apply_reputation_phase
    ──► apply_due_autonomous_recruitment  sees current resentment + decayed/current competence
    ──► apply_due_autonomous_enterprise_lifecycle
          suspended NPC rackets: viable reopen; stale frozen authority/assets retire
    ──► apply_due_autonomous_enterprises_excluding
          new growth; mandates that reopened a racket already spent this daily action
    ──► synthesize_executive_brief        sees every report/decision made this minute, last
    ──► validate_invariants               structural + registry re-derivation
```

New autonomous work must slot explicitly here with a rationale comment.

## Source map — one owner per field

Every `src/` subsystem owns its records and canonical mutation paths. Invariants
are validated by `src/core/invariants/`. The top-level tick is
`core::simulation::run_tick`, driven from `AppState` and `Registry`.

| Module | Owns | Canonical mutation | Key file |
|---|---|---|---|
| `core/` | `SimTime`/`SimDuration`, typed persistent IDs (`IdCounters`), entity refs (`EntityRef`), attention classes, `AppState`, persistence envelope, tick pipeline, invariant validation | `core::simulation` runs the tick; `core::state` owns generated state; `core::invariants::validate_state` | `src/core/state.rs`, `src/core/simulation.rs`, `src/core/invariants/mod.rs` |
| `registry/` | Immutable authored definitions and validated lookups, including one shared information reliability/specificity quality mapping consumed across domains | `RegistryBuilder` owns registration/completeness; focused authoring validators check domain contracts before insertion; read-only after `content::build_registry` | `src/registry/mod.rs`, `src/registry/builder.rs`, `src/registry/operation_validation.rs` |
| `content/` | Code-owned authored definitions for the registry | `build_registry` | `src/content/mod.rs` owns `CURRENT_CONTENT_REVISION` |
| `world/` | Organizations, characters, neighborhoods, businesses, institutional profiles, designation, versioned per-policy organization storage, daily payroll; read-only territory-influence aggregation | `world_system` (insertion, designation, world-owned versioned policy write used by governance orchestration); `payroll_execution` (daily wage pass through canonical ledger, relationship, and report paths); `territory_influence` (read-only district summaries, never an omniscience feed) | `src/world/world_system.rs`, `src/world/payroll_execution.rs` |
| `social/` | Directional character relationships with source/target indexes | `relationship_system` only; requires active endpoints | `src/social/relationship_system.rs` |
| `intelligence/` | Provenance-bearing information, holder/topic indexes, lineage, and owner-derived planned source identity for atomic composites that must validate a dependent artifact before the information record is inserted | `intelligence_system` (record, transfer, planned source projection) | `src/intelligence/intelligence_system.rs` |
| `reports/` | Player-facing reports, briefs, financial reports; bounded executive-brief sources rank by attention then newest report chronology, while pending decisions remain oldest-first within an attention class. Composite report validation may cite one intelligence-owned planned source, but report commit remains fail-closed until that exact source exists | `report_system` plus `executive_brief` synthesis | `src/reports/report_system.rs`, `src/reports/executive_brief.rs` |
| `history/` | Durable entity-linked campaign events | `history_system` | `src/history/history_system.rs` |
| `finance/` | Typed accounts, allocator-neutral planned account openings, balanced ledger, canonical basis-point money rounding shared by runtime scaling and registry arithmetic proofs, criminal-organization laundering transfers through owned cash-intensive fronts with a nonzero authored fee; delegated budgets authorize outflow only from their designated funding account and never provide implicit credit beyond that account's actual balance. The generic ledger primitive is the finance owner's trusted composition/setup surface, but it cannot post to a business-owned legitimate operating account once that account is attached to live business books; those tills are mutated only by the economy/laundering owners, and held generic tokens recheck that protection at commit. | `finance_system` remains the public owner/facade for account, ledger, budget, and laundering mutations; `finance_system/laundering.rs` owns laundering-specific validation and composes the parent ledger path with the economy-owned plausibility-capacity update; `helpers` owns shared financial arithmetic | `src/finance/finance_system.rs`, `src/finance/finance_system/laundering.rs`, `src/finance/helpers.rs` |
| `operations/` | Criminal-organization operation plans, execution records, participant reservations, abort causality and artifacts, and surveillance, police-response, property, and take-economics integrations. Authorization, timing, abort lifecycle, objective viability, deterministic resolution, proceeds, and after-action reporting run through the named sibling systems. After-actions carry only crew-observed operational and exposure facts; hidden institutional case intake and routing become organization knowledge only through legal, intelligence, or contact channels. | `operation_system` (authorization/start), `operation_scheduling` (timing, booking, deadline, due-work projections backed by the operation owner's derived active-deadline index), `operation_intelligence` (shared planning-value and freshness scoring), `operation_abort` (abort lifecycle and artifacts), `operation_objective` (execution-time objective blockers), `operation_execution` (resolution orchestration with `resolution_factors`, `resolution_effects`, `incident_intake`, `narrative`), `operation_economics` (proceeds and replenishment), and `surveillance_integration` for target snapshots/observation persistence; `surveillance_integration/observation_text.rs` owns read-only observation prose and typed signals while `surveillance_integration/after_action.rs` owns historical player-facing reconstruction | `src/operations/operation_system.rs`, `src/operations/operation_execution.rs`, `src/operations/surveillance_integration.rs`, `src/operations/surveillance_integration/observation_text.rs`, `src/operations/surveillance_integration/after_action.rs` (full sibling map in `src/operations/mod.rs`) |
| `opportunities/` | Provenance-backed opportunities with lifecycle | `opportunity_system` | `src/opportunities/opportunity_system.rs` |
| `decisions/` | Durable typed decision records and pending indexes; recruitment approvals snapshot the exact mandate/manager/policy-source revisions and are cancelled when their frozen mandate authority or organization-sourced effective policy is permanently superseded | `decision_system` remains the public facade for generic request/resolution orchestration and operation decisions; `decision_system/recruitment_approval.rs` owns recruitment-specific request creation, autonomous resolution composition, obsolete-approval cancellation, and authority/policy freshness. Governance transitions compose its validated cancellation token instead of mutating decision state directly. | `src/decisions/decision_system.rs`, `src/decisions/decision_system/recruitment_approval.rs` |
| `delegation/` | Organization-owned mandates and responsibility indexes; organization-policy governance over settings stored by `world`. A mandate standing order is valid only when the mandate includes the corresponding functional responsibility scope: Personnel for independent recruitment and Legal for associate legal support. | `delegation_system` owns assignment/revision/revocation, policy resolution, and the canonical organization-policy command; mandate or organization-policy changes compose decision-owned cancellation for approvals they permanently supersede | `src/delegation/delegation_system.rs` |
| `enterprises/` | Routine criminal enterprises and cycle history with an Active → Suspended → Active/Retired lifecycle; Retired is terminal history and releases the kind/location slot. Per-cycle enforcement-attention rolls convert sustained district casework into an originated inquiry through canonical incident intake. Daily NPC maintenance reconsiders suspended rackets from the organization's own observable pressure and current working-capital runway: viable positive-net rackets may reopen on a later boundary, temporary heat/funding/economic blockers leave them suspended, while revoked/re-scoped authority, lost required host/support ownership, or immutable Tight/Guided manager autonomy makes an autonomous rival abandon that stale frozen configuration and retire its record so it cannot reserve a slot forever. One mandate may reopen at most one racket per daily pass, and reopening consumes that mandate's expansion action. New delegated expansion then uses canonical establishment against the remaining mandates and working capital. Both behaviors consume shared read-only planning projections for bounded observed district pressure and committed working capital, never hidden live investigations, while settlement reads authoritative legal truth. Notable settlements publish a Financial report citing the manager's own observation. | `enterprise_execution` remains the public owner/facade for lifecycle and atomic settlement; its `cycle_planning` sibling owns the read-only per-cycle decision, economics/heat classification, and staleness snapshot assembly. `autonomous_planning` owns shared bounded-pressure/runway projections, `autonomous_lifecycle` owns daily suspended-racket maintenance, `autonomous_expansion` owns daily delegated growth while its private `candidate_planning` child owns read-only venue/network enumeration and economic ranking, and `enterprise_reporting` is read-only. | `src/enterprises/enterprise_execution.rs`, `src/enterprises/enterprise_execution/cycle_planning.rs`, `src/enterprises/autonomous_planning.rs`, `src/enterprises/autonomous_lifecycle.rs`, `src/enterprises/autonomous_expansion.rs`, `src/enterprises/autonomous_expansion/candidate_planning.rs` |
| `economy/` | Legitimate business economies, cycle history, sabotage/arson disruption horizons of exactly the authored minute duration, and chronic-loss suspension. A daily autonomous pass reconsiders suspended non-player businesses through the canonical resume token: same-minute suspensions remain consequential, currently positive zero-variance books may reopen, structurally losing books stay suspended, and player-organization ownership remains an explicit player decision. Owner profit sweeps may distribute only operating cash above the persisted working-capital floor captured when the economy is established or ownership restarts; same-owner suspension/resumption preserves that snapshot, while acquisition rebases it to the positive operating cash actually inherited at the new ownership revision. The floor is version-pinned to ledger and business history so restore can re-derive it rather than trusting a mutable balance summary. An operating account may receive opening capital before establishment, but once attached to live books its till is protected from generic ledger credits and debits; cycle settlement, laundering, and canonical owner sweeps use the internal business-ledger path. Current-window laundering provenance links the exact ledger transactions behind the front's running plausibility total; settled cycles and acquisition restarts open a fresh window, while same-owner suspension/resumption retains consumed capacity until the next settlement. Acquisition buys independently owned businesses at the authored kind price from organization-owned accounted funds, crediting the acquired business's non-liquid seller-settlement counterparty rather than its operating cash. | `business_economy_system` remains the public owner/facade for establishment, disruption, lifecycle, autonomous recovery, and owner sweeps; `business_economy_system/cycle_planning.rs` owns the complete deterministic business-cycle decide/validate/commit transaction plus shared live/historical financial arithmetic; `business_acquisition` owns canonical purchase composition; `business_reporting` is read-only | `src/economy/business_economy_system.rs`, `src/economy/business_economy_system/cycle_planning.rs`, `src/economy/business_acquisition.rs` |
| `legal/` | Jurisdictions, patrols, timed police response, and origin-typed investigations (operation exposure or enterprise enforcement attention) with evidence, arrests, custody, representation, prosecution, witnesses, and informants. Only originated cases decay cold; live custody under the same file defers decay without aborting the batch, and detention under an unrelated file never clears the case. Staffing is single-seat and conflict-free; actionable evidence newly implicating staff recuses that role with provenance and cancels conflicting pending work. Scheduled detective capacity is a derived live index rather than a scan of lifetime work history. Each reviewable evidence source gets one actual evidence-review attempt across direct and autonomous scheduling; cancellation releases that source for retry because no review occurred, while an inconclusive completed review cannot be rerolled until success. Testimony is single-statement per case-witness registration over case-connected entities only. Direct and autonomous custody share one authored corroboration bar over independent non-weak, non-questionable, admissible sources; renewed custody in the same case requires qualifying evidence from the release minute or later that the prior detention never cited. Arrest custody ends at the authored maximum while prosecution referral and review can continue after release. Automatic legal support resolves before the detainee's one-time informant decision. | Named modules (`jurisdiction_system`, `patrol_system`, `investigation_system`, `arrest_system`, `legal_representation_system`, `prosecution_system`, …) via `legal_state`; `investigation_system/autonomous_staffing.rs` owns detective allocation and `investigation_system/cold_case_decay.rs` owns originated-file inactivity lifecycle, both through the parent investigation owner's canonical mutations; `investigation_work_execution` owns direct scheduling/cancellation, its private `resolution.rs` child owns stochastic resolution/factor validation/evidence persistence, and `investigation_work_execution/autonomous_scheduling.rs` owns the deterministic per-minute scheduling policy and traverses the active-case index once with evidence-review priority; `legal_representation_system/automatic_support.rs` owns deterministic automatic-policy orchestration but composes the parent system's canonical retain/end commands; `legal_state.rs` owns durable records and index reconstruction, `legal_state/queries.rs` owns read-only indexed observation, `legal_state/mutations.rs` owns record mutation plus synchronized index maintenance | `src/legal/investigation_system.rs`, `src/legal/investigation_system/autonomous_staffing.rs`, `src/legal/investigation_system/cold_case_decay.rs`, `src/legal/investigation_work_execution.rs`, `src/legal/investigation_work_execution/resolution.rs`, `src/legal/investigation_work_execution/autonomous_scheduling.rs`, `src/legal/legal_representation_system.rs`, `src/legal/legal_representation_system/automatic_support.rs`, `src/legal/legal_state.rs`, `src/legal/legal_state/queries.rs`, `src/legal/legal_state/mutations.rs` |
| `contacts/` | Institutional contacts and provenance-preserving disclosures; pending disclosure offers rank factual freshness by observation time, using recording time and ID only as deterministic tie-breakers so late forwarding cannot make stale facts outrank newer observations | `contact_system` (establishment, termination, disclosure; `find_pending_disclosure_sources` read-only offer surface) | `src/contacts/contact_system.rs` |
| `recruitment/` | Relationship-gated recruitment, cooldowns, approvals, membership changes; autonomous cross-organization prospect contention is relationship-priority rather than mandate-ID priority, and a pending approval exclusively owns that organization/candidate route so generic world reassignment cannot bypass it while the exact approved decision can complete it | `recruitment_system` owns canonical recruitment transactions; its private `planning.rs` child owns read-only plan construction plus held-plan state/definition revalidation; `autonomous_recruitment` owns the deterministic daily delegated policy pass; `world_system` owns the guarded membership mutation; `scoring` owns factor/margin arithmetic shared by planning and invariant re-derivation | `src/recruitment/recruitment_system.rs`, `src/recruitment/recruitment_system/planning.rs`, `src/recruitment/autonomous_recruitment.rs`, `src/recruitment/scoring.rs`, `src/world/world_system.rs` |
| `reputation/` | Contextual per-audience organizational standing with per-dimension movement freshness and age-gated baseline decay; fed by operation consequences (non-surveillance success competence, exposure police fear for every kind, violent-approach business and resident fear) and enterprise vice inquiries, consumed by recruitment scoring and expansion posture; player shifts surface atomically with Standing reports, and reported deltas are the actual bounded score movement after clamping rather than the larger requested change | `reputation_system` (`apply_reputation_delta` is the single score mutation path; consequence composition and decay are tick passes) | `src/reputation/reputation_system.rs` |

Adapters, the harness at [`examples/gameplay_harness/`](examples/gameplay_harness/main.rs), and verification at [`scripts/verify.ps1`](scripts/verify.ps1) / [`scripts/verify.cmd`](scripts/verify.cmd) live outside `src/` and use the canonical paths above.

## Canonical operations — one production path

One production path per operation class. UI, tests, examples, importers, and tools use the same semantics.

**Decide then apply** — decision reads broader state than it mutates:

```text
decide_*(&state, ...) -> Plan / Outcome / Delta
apply_*(&mut state, plan)
```

Decision is read-only except for explicitly supplied deterministic randomness
(`&mut ChaCha8Rng` via `draw_index`). The source map above routes each domain to its
owning system and module contract.

**Validate then commit** — fallible multi-resource operations:

```text
validate_*(&state, ...) -> Validated*
Validated*::commit(self, &mut state) -> Result<Outcome, TypedError>
```

Validation resolves references, permissions, lifecycle, ownership, capacity, ranges,
and arithmetic before any consequential mutation. Commit consumes the validated
value and rechecks authorization if staleness can invalidate it (`SimTime` freshness).
When a composite artifact must reference a record that the same commit will create, the owning
system predicts that record's next ID without consuming it and exposes only an owner-derived
planned-source token; the outer commit stales if allocator position changes, and the dependent
owner still refuses to commit until the source actually exists. On rejection authoritative state
is unchanged unless the contract explicitly records a diagnostic (e.g. a `HistoryEvent`).

Single-owner operations may mutate directly when every return path preserves that owner's invariants and indexes
(`social::relationship_system::set_relationship`, `reputation_system::apply_reputation_delta`).

## Data ownership

- One private owner per consequential field — the smallest owner that can keep it coherent.
- Collections that must agree are private fields of one owner, changed atomically (`insert`, `remove`, `move`, `reassign`).
- Cross-owner coordination goes through a system or higher orchestration boundary. Owners do not patch each other's private state.
- Importers, migrations, tests, and tools do not bypass owner methods.
- Durable references use typed identity where the project controls the vocabulary. Display text is not identity.
- ID allocation is monotone from 1; `IdCounters::reserve` before any multi-record commit keeps `validate_id_allocators` (`src/core/invariants/mod.rs`) honest.

**Derived-record pattern (all domains follow it):**

```text
records: BTreeMap<Id, Record>              // authoritative truth
derived: BTreeMap<Key, BTreeSet<Id>>       // maintained at every insert/remove
indexed read -> authoritative record        // missing target is invalid state, never filtered away
has_consistent_indexes() -> bool           // checked by validate_state, exhaustive
BTreeMap::insert + debug_assert!(previous.is_none())  // uniqueness guard
```

See `src/finance/mod.rs`, `src/enterprises/mod.rs`, `src/economy/mod.rs`
for canonical examples. Forgetting the derived index makes a record invisible to
`run_tick`; leaving a stale entry leaks revoked/suspended work.

## Determinism

Authoritative behavior is determined by registry definitions, serialized state,
ordered explicit inputs, state-owned RNG, and any explicitly modeled external snapshot.

- Result-affecting randomness comes from state-owned or explicitly injected deterministic RNG only.
- Order-sensitive work uses ordered collections or explicit sorting with complete stable tie-breakers.
- Wall-clock time, filesystem iteration, hash iteration, thread scheduling, UI timing, and ambient entropy are not simulation inputs.
- Parallelism may change throughput, not authoritative semantics.
- Top-level scheduling order is visible in one orchestration surface (`run_tick`).

**RNG streams — 4 independent ChaCha8, never cross-contaminate (`src/core/state.rs`):**

| Stream | Field | Used for | Draw helper (`src/core/simulation.rs`) |
|---|---|---|---|
| `operation_rng` | `simulation.operation_rng` | operation execution & exposure variance | `draw_signed_variance(limit)` → `i8` |
| `investigation_rng` | `simulation.investigation_rng` | investigation-work variance | `draw_signed_variance(limit)` |
| `business_rng` | `simulation.business_rng` | business cycle gross variance (basis points) | `draw_basis_point_variance(limit)` → `i16` |
| `enterprise_rng` | `simulation.enterprise_rng` | enterprise gross variance + enforcement-attention roll (both **unconditionally** per cycle) | `draw_basis_point_variance` + `draw_index(10_000)` |

`draw_index` at `src/core/simulation.rs` uses rejection sampling (`u64::MAX - (u64::MAX % bound)`) — no modulo bias.
Time-indexed `find_due_*` schedulers use `BTreeMap<SimTime, BTreeSet<Id>>` and preserve its natural `(due time, ID)` order. IDs break ties only among work due at the same instant, so deterministic catch-up never lets creation order outrank chronology.

## Persistence

Save/load preserves every value required for continuation: IDs, relationships,
lifecycle, counters, generated definitions, RNG state, and active durable work.

```text
SaveEnvelope { format_version, content_revision, state: AppState(current schema) }
build_save(registry, state)  ─► validate_state + validate_state_against_registry, then clone;
                                serde skips all derived lookup/scheduling indexes
restore_save(registry, envelope) ─► format check → schema check → content revision check
                                    → rebuild derived indexes from authoritative records
                                    → validate_state → validate_state_against_registry → Ok(state)
```

- Cross-references and invariants are validated before loaded state becomes trusted.
- Derived indexes are not part of save truth. They are omitted from serialized state, rebuilt only
  from persisted authoritative records, and must agree under normal invariant validation after reconstruction.
- Restore re-derives cross-record rules that a rebuilt set index cannot prove by itself: operation
  participant booking exclusivity, one active investigation lead seat per investigator, one
  non-retired enterprise kind/location slot, and globally monotone report chronology.
- Missing future-affecting values are not silently defaulted to make old data load.
- Compatibility policy is in [`STATUS.md`](STATUS.md): current-version only, no implicit migration.
- Core systems do not perform implicit filesystem IO (`src/core/persistence.rs` returns data; adapters do IO).

ID high-water marks: `validate_id_allocators` checks `next > max_persisted` per `IdKind`.
Finance re-derivation: `src/core/invariants/finance.rs` walks the ledger once,
dense `Vec<i64>` keyed by `raw()` — balances must agree with derived cents.

## Runtime invariants

1. Required registry and runtime references resolve.
2. Records appear exactly where required in derived indexes.
3. Exclusive ownership is represented once.
4. Lifecycle state agrees with active, scheduled, and indexed membership.
5. Multi-record operations commit completely or not at all.
6. IDs, handles, events, and outcomes have an owner and valid location.
7. Deterministic selection uses stable ordering and tie-breaking.
8. A character cannot hold overlapping non-terminal operation assignments; restore reconstructs
   the same effective booking windows used by runtime admission and permits overlap only when a
   originally-disjoint authorized booking is later reached by an unresolved decision pause.
9. Static definitions contain no mutable runtime state.
10. Save/load preserves all future-affecting state.
11. Derived counters and projections agree with source records.
12. External effects cross explicit adapter boundaries.
13. Rejected operations preserve authoritative state except for explicitly modeled diagnostics.
14. Active investigation lead staffing is single-seat; suspension/closure releases that seat.
15. At most one non-retired enterprise of a kind occupies a location; retirement releases the slot.

Maintain `validate_invariants(state)` for structural checks. The soak exercises mixed state under invariant validation; [`TESTING.md`](TESTING.md) owns how it is run.

The ledger enforces balance and overflow, not solvency: a validated settlement may drive an operating account negative, recording an obligation rather than rejecting the cycle. Domain owners decide suspension or closure consequences; the ledger itself never silently clamps balances. See `src/finance/mod.rs` (`Money(i64 cents)`) and `src/core/invariants/finance.rs` (single-pass re-derivation).

## API and representation

- Prefer explicit structs and project-owned enums over string-keyed registries for closed vocabularies.
- Match project-owned enums exhaustively; wildcards are for open or third-party vocabularies only (`clippy::wildcard_enum_match_arm = deny`).
- Map closed records explicitly so adding a field cannot silently disappear.
- Use typed error enums for new fallible domain operations; variants identify the failed precondition.
- Pass the narrowest context each phase needs. Read-only phases take `&state`; mutation phases take only the owner access required.
- Public surface is intentional. Do not expose helpers solely for tests.

## Naming and modules

Single vocabulary across subsystems:

| Purpose | Form | Example |
|---|---|---|
| keyed lookup | `get_*` | `get_operation`, `get_account`, `get_enterprise` |
| conditional scan | `find_*` | `find_due_authorized_operations`, `find_pending_disclosure_sources` |
| final derivation | `resolve_*` | `resolve_execution_margin`, `resolve_property_liquidation_value` |
| state-owned randomness draw | `draw_*` | `draw_index`, `draw_signed_variance` |
| plain accessor | noun form, e.g. `status()` | `operation.status()`, `record.balance()` |
| construction | `new()` | `AppState::new(seed)`, `WorldState::new()` |
| aggregate assembly | `build_*` | `build_registry`, `build_budget_usage` |
| authored definition registration | `register_*` | `register_operation_kinds`, `register_business_kinds` |
| runtime insertion/removal | `insert_*`, `remove_*` | `insert_account`, `remove_mandate` |
| in-place record update | `set_*` (field/status writes on an owned record) | `set_relationship`, `set_auto_pause` |
| read-only decision | `decide_*` | `decide_operation_resolution`, `decide_business_cycle` |
| checked command | `validate_*` → `Validated*` | `validate_authorize_operation` → `ValidatedOperation::commit` |
| resolved mutation | `apply_*` or consuming `commit` | `apply_daily_payroll`, `Validated*::commit` |

Predicates use `is_`, `has_`, or `can_`. Do not introduce new `create_*`, `make_*`, `execute_*`, `perform_*`, or `attempt_*` when an established role already fits.

Multi-file suffixes use established roles: `_execution`, `_integration`, `_loader`, `_ui`, `_adapter`. Every `src/` file starts with a concise `//!` purpose statement; multi-file subsystems state sibling relationships where not obvious. Comments explain constraints, ordering, safety, invariants, or non-obvious intent — not history or commented-out code.

## Dead code and replacement

- Behavior that should run is wired into the canonical path (`run_tick` or an explicit system call).
- Test-only fixtures and helpers live under `#[cfg(test)]`.
- Obsolete behavior is deleted with its tests and documentation. No historical shims.
- Do not add fake call sites, broad `allow(dead_code)`, public shims, or test-only production APIs to silence warnings.

One implementation owns each concern unless an active external compatibility contract explicitly requires otherwise.
