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
 5  core::simulation::run_tick        one simulated minute in stable contractual order
 6  TickOutcome + reports/projections player-visible consequences, no hidden-state leak
 7  build_save / restore_save         envelope {format_version, content_revision, state}
```

Tick cadence is an adapter concern. Calling `run_tick` faster or slower changes
wall time, not the semantics of one canonical minute.

### run_tick — contractual order (`src/core/simulation.rs`)

Phase order is the coupling contract. Comments at `src/core/simulation.rs`
explain each “runs after X so Y is visible” dependency. Reordering breaks
determinism and harness contracts.

```text
 1  apply_due_custody_releases              hard arrest-custody boundary before same-minute consumers
 2  apply_opportunity_expiry                durable lifecycle report before remaining same-minute consumers
 3  run_operations_phase                    police arrivals → starts → deadline cleanup → resolution
 4    ├─ apply_due_police_response_arrivals (exposure → decisions; must precede new starts)
 5    ├─ find_due_authorized → Begin or deadline-missed
 6    ├─ find_due_with_missed_deadlines → abort via decision when present
 7    └─ find_due_in_progress → decide+validate+commit per operation (RNG: operation stream)
 8  apply_autonomous_investigator_staffing  single-seat staffing, lead-investigator knowledge
 9  apply_evidence_review_scheduling        next unattempted reviewable evidence on active staffed cases
10  apply_witness_interview_scheduling      after reviews so same-minute witness is interviewable
11  run_investigation_work_phase            resolve due work (RNG: investigation stream)
12  apply_autonomous_evidence_arrests       active LawEnforcement cases: authored independent-evidence threshold → custody + responsibility preemption
13  apply_autonomous_prosecution_staffing   refill prosecution seats released by custody
14  apply_automatic_legal_support           conclude boundary releases; retain before a due detainee decision
15  apply_detainee_informant_recruitment    one decision after a delay; active counsel lowers authored flip chance
16  apply_informant_disclosures             holder-knowledge → handler cases
17  apply_cold_case_decay                   originated cases only, authored inactivity window, no RNG
18  run_business_cycle_phase                per due business (RNG: business stream)
19  run_enterprise_cycle_phase              per due enterprise (RNG: enterprise stream, 2 draws unconditionally)
20  apply_daily_payroll  →  apply_reputation_phase
    ──► apply_due_autonomous_recruitment  sees current resentment + decayed/current competence
    ──► apply_due_autonomous_enterprises  reads current police-fear posture
    ──► synthesize_executive_brief        sees every report/decision made this minute, last
    ──► validate_invariants               structural + registry re-derivation
```

New autonomous work must slot explicitly here with a rationale comment.

## Source map — one owner per field

Every `src/` subsystem owns its records and canonical mutation paths. Invariants
are validated by `src/core/invariants/`. The top-level tick is
`core::simulation::run_tick`, driven from `AppState` and `Registry`.

| Module | Owns | Canonical mutation | Key file:line |
|---|---|---|---|
| `core/` | `SimTime`/`SimDuration`, typed persistent IDs (`IdCounters`), entity refs (`EntityRef`), attention classes, `AppState`, persistence envelope, tick pipeline, invariant validation | `core::simulation` runs the tick; `core::state` owns generated state; `core::invariants::validate_state` | `src/core/state.rs`, `src/core/simulation.rs`, `src/core/invariants/mod.rs` |
| `registry/` | Immutable authored definitions and validated lookups, including one shared information reliability/specificity quality mapping consumed across domains | `RegistryBuilder` owns registration/completeness; focused authoring validators check domain contracts before insertion; read-only after `content::build_registry` | `src/registry/mod.rs`, `src/registry/builder.rs`, `src/registry/operation_validation.rs` |
| `content/` | Code-owned authored definitions for the registry | `build_registry` | `src/content/mod.rs` owns `CURRENT_CONTENT_REVISION` |
| `world/` | Organizations, characters, neighborhoods, businesses, institutional profiles, designation, versioned per-policy organization storage, daily payroll; read-only territory-influence aggregation | `world_system` (insertion, designation, world-owned versioned policy write used by governance orchestration); `payroll_execution` (daily wage pass through canonical ledger, relationship, and report paths); `territory_influence` (read-only district summaries, never an omniscience feed) | `src/world/world_system.rs`, `src/world/payroll_execution.rs` |
| `social/` | Directional character relationships with source/target indexes | `relationship_system` only; requires active endpoints | `src/social/relationship_system.rs` |
| `intelligence/` | Provenance-bearing information, holder/topic indexes, lineage | `intelligence_system` (record, transfer) | `src/intelligence/intelligence_system.rs` |
| `reports/` | Player-facing reports, briefs, financial reports; bounded executive-brief sources rank by attention then newest report chronology, while pending decisions remain oldest-first within an attention class | `report_system` plus `executive_brief` synthesis | `src/reports/report_system.rs`, `src/reports/executive_brief.rs` |
| `history/` | Durable entity-linked campaign events | `history_system` | `src/history/history_system.rs` |
| `finance/` | Typed accounts, allocator-neutral planned account openings, balanced ledger, criminal-organization laundering transfers through owned cash-intensive fronts with a nonzero authored fee; delegated budgets authorize outflow only from their designated funding account and never provide implicit credit beyond that account's actual balance | `finance_system` (all financial mutations, including `validate_launder_funds`) | `src/finance/finance_system.rs` |
| `operations/` | Criminal-organization operation plans, execution records, participant reservations, abort causality/artifacts, surveillance/police/property integrations, runtime objective viability, take economics; direct authorization enforces the same Criminal sponsorship boundary as opportunity discovery and rejects a direct character objective target serving on its own crew; delayed admission preserves chronological reservation priority when an older due operation grows into a later booking's originally disjoint window; effective crew ability uses authored role-versus-leadership weights rather than a hidden resolution constant; mutable objective viability is checked at due-start admission so work whose objective is already impossible receives a durable pre-start `ObjectiveUnavailable` abort, while changes after execution begins remain historical resolution blockers; after-actions report only crew-observed operational/exposure facts and never convert hidden institutional case intake/routing into organization knowledge; foreign administrative surveillance targets (operations, investigations, enterprises) require organization-held information naming the exact record, while an organization intrinsically knows its own corresponding records, so raw IDs cannot probe hidden state; typed achieved-surveillance patrol signals carry every observed recurring window using conservative containing boundaries at the authored observation granularity; post-entry police-arrival leadership exceptions become durable decisions for the player organization, while non-player organizations resolve the same exception autonomously by authority abort rather than waiting indefinitely for player input; repeat-target proceeds replenish continuously over the authored recovery window; witness pressure treats police custody as target unavailability, so an already-detained witness cannot be authorized for field intimidation and an in-flight detention removes the cooperation effect, while patrol/exposure routing remains anchored to the latest pressureable foreign witness case instead of expanding through unrelated witness or organization assets, with only one durable case used as the in-flight fallback | `operation_system` (authorization/start/scheduling), `operation_intelligence` (shared planning-value/freshness scoring used by discovery, authorization, persistence validation, and resolution), `operation_abort` (authority/deadline/opportunity/objective/decision/police/detention abort lifecycle and artifacts), `operation_objective` (mutable execution-time objective blockers), `operation_execution` (deterministic resolution orchestration, with `resolution_factors`, `resolution_effects`, `incident_intake`, and `narrative` child modules), and `operation_economics` (proceeds and replenishment) | `src/operations/operation_system.rs`, `src/operations/operation_intelligence.rs`, `src/operations/operation_abort.rs`, `src/operations/operation_objective.rs`, `src/operations/operation_execution.rs`, `src/operations/operation_execution/resolution_factors.rs`, `src/operations/operation_execution/resolution_effects.rs`, `src/operations/operation_execution/incident_intake.rs`, `src/operations/operation_execution/narrative.rs` |
| `opportunities/` | Provenance-backed opportunities with lifecycle | `opportunity_system` | `src/opportunities/opportunity_system.rs` |
| `decisions/` | Durable typed decision records and pending indexes; recruitment approvals snapshot the exact mandate/manager/policy-source revisions and are cancelled when their frozen mandate authority or organization-sourced effective policy is permanently superseded | `decision_system` owns request, resolution, and cancellation lifecycles; governance transitions compose its validated cancellation token instead of mutating decision state directly | `src/decisions/decision_system.rs` |
| `delegation/` | Organization-owned mandates and responsibility indexes; organization-policy governance over settings stored by `world` | `delegation_system` owns assignment/revision/revocation, policy resolution, and the canonical organization-policy command; mandate or organization-policy changes compose decision-owned cancellation for approvals they permanently supersede | `src/delegation/delegation_system.rs` |
| `enterprises/` | Routine criminal enterprises and cycle history; Active → Suspended → Active/Retired lifecycle, with Retired terminal history releasing the kind/location slot; per-cycle vice-attention rolls convert sustained district casework into an originated inquiry on the racket through canonical incident intake; delegated daily expansion for non-player organizations through canonical establishment, using only working capital not already reserved for active or earlier same-pass rackets and ranking viable positive-net candidates from the same zero-variance gross/cost composition production settlement uses, but supplying only district pressure inferred from the organization's own recent settled racket history rather than hidden live investigations; observed pressure expires at the legal cold-case horizon while actual settlement continues to read authoritative legal truth; explicit territorial/business scopes rank before the broad `Function(Enterprise)` fallback within one organization's planning, while cross-organization phase contention gives existing district leadership priority over internal mandate specificity and then compares projected economics | `enterprise_execution` (lifecycle/settlement and canonical financial projection), `autonomous_expansion` (daily delegated expansion), `enterprise_reporting` (read-only) | `src/enterprises/enterprise_execution.rs` |
| `economy/` | Legitimate business economies, cycle history, sabotage disruption horizons with exact authored-minute duration, chronic-loss suspension, and current-window laundering provenance linking the exact ledger transactions behind the front's running plausibility total; settlement and explicit operating-window restarts clear that total and provenance together; acquisition of independently owned businesses at the authored kind price paid by aggregating organization-owned accounted-funds accounts, with exact source debits owned by the ledger and purchase consideration credited to the acquired business's non-liquid seller-settlement counterparty rather than its operating cash | `business_economy_system` (establishment/settlement/disruption/suspension/restart), `business_acquisition` (canonical purchase composing ownership transfer, first economy establishment or existing-book restart, and payment), `business_reporting` (read-only) | `src/economy/business_acquisition.rs` |
| `legal/` | Jurisdictions, patrols, timed police response, investigations/evidence/arrests/custody/representation/prosecution/witnesses/informants; case origination is a typed entity link (operation exposure or enterprise vice attention) and only originated cases decay cold; incident continuation resumes the best matching shelf and preserves validated subject matter without storing a parallel visibility-recipient list; authority surveillance derives case relevance from the durable origin organization and creates player knowledge only through the surveillance result; investigation records persist declared incident subject matter separately from their effective tracked subject set, and invariants re-derive the latter exactly as declared subjects plus subjects promoted by actionable evidence; scarce autonomous detective staffing prioritizes developed active files by actionable evidence count, strongest actionable assessment, evidence breadth, and recent institutional activity, with case ID only as the final exact tie-break; current detective/prosecutor staffing is conflict-free with the source case's actionable subjects and named factual witnesses, and actionable evidence that newly implicates a staffed detective or prosecutor atomically recuses that role rather than suppressing the evidence; detective recusal also cancels conflicting pending work with durable evidence provenance; autonomous prosecutor staffing balances active reviewing caseload before LegalKnowledge after conflict filtering; named witness testimony is single-statement per case-witness registration, can concern only case-connected entities, and its confidence/cooperation evidence mapping is authored in the legal registry; interview testimony follows actionable character evidence and ranks it by strength then reliability instead of letting non-actionable evidence steer a statement; staffed detectives hold typed first-hand witness-identity knowledge whether the witness or lead was established first, allowing ordinary contact disclosure to expose witness-pressure opportunities without reading hidden case state; witness registrations remain historical if later evidence promotes that character into the same case's arrest-eligible subject set, but future witness actions stop and any pending interview is cancelled through the work lifecycle with the causal evidence ID; direct statements likewise cancel redundant pending interviews while work-produced statements exempt their originating interview; external witness-cooperation changes and custody-forced detective work cancellation/lead release invalidate dependent plans or staffing without resetting the institution's cold-case inactivity clock; direct and autonomous custody share one authored corroboration bar over independent non-weak/non-questionable evidence sources not known inadmissible, with derived evidence adding no source and repeated witness/informant records from one named source counting once; autonomous custody scans active LawEnforcement-owned investigations, prefers the file with more independent qualifying sources and then more Strong/Direct independent sources when several files qualify one character, and after release requires qualifying evidence from that release minute or later before the same case can detain that person on a later minute; non-police LegalAuthority files remain investigative only; arrest custody has an authored maximum because bail/trial/sentence custody and court procedure are outside the current foundation, while prosecution referral/review can continue after release; prosecution referrals accept only source-authority evidence whose subject is the case defendant; automatic legal support resolves before the detainee informant decision so active counsel can reduce the authored cooperation chance; direct and organization-policy automatic defense retention can aggregate sponsor liquid accounts, while mandate-sourced automatic and explicitly delegated retention use the mandate's Legal scope and budget account | Named modules (`jurisdiction_system`, `patrol_system`, `investigation_system`, `arrest_system`, `legal_representation_system`, `prosecution_system`, …) via `legal_state`; `legal_state.rs` owns durable records and index reconstruction, `legal_state/queries.rs` owns read-only indexed observation, and `legal_state/mutations.rs` owns record mutation plus synchronized index maintenance; arrest composes validated responsibility preemption; investigation and prosecution staffing remain current assignments while historical action actors stay on durable artifacts; retainer payment allocation and delegated budget usage are historical ledger truth, not duplicated representation state | `src/legal/legal_state.rs`, `src/legal/legal_state/queries.rs`, `src/legal/legal_state/mutations.rs` |
| `contacts/` | Institutional contacts and provenance-preserving disclosures; pending disclosure offers rank factual freshness by observation time, using recording time and ID only as deterministic tie-breakers so late forwarding cannot make stale facts outrank newer observations | `contact_system` (establishment, termination, disclosure; `find_pending_disclosure_sources` read-only offer surface) | `src/contacts/contact_system.rs` |
| `recruitment/` | Relationship-gated recruitment, cooldowns, approvals, membership changes; autonomous cross-organization prospect contention is relationship-priority rather than mandate-ID priority, and a pending approval exclusively owns that organization/candidate route so generic world reassignment cannot bypass it while the exact approved decision can complete it | `recruitment_system` owns canonical recruitment transactions; `autonomous_recruitment` owns the deterministic daily delegated policy pass; `world_system` owns the guarded membership mutation; `scoring` owns factor/margin arithmetic shared by decide paths and invariant re-derivation | `src/recruitment/recruitment_system.rs`, `src/recruitment/autonomous_recruitment.rs`, `src/recruitment/scoring.rs`, `src/world/world_system.rs` |
| `reputation/` | Contextual per-audience organizational standing with per-dimension movement freshness and age-gated baseline decay; fed by operation consequences and enterprise vice inquiries, consumed by recruitment scoring and expansion posture; player shifts surface atomically with Standing reports, and reported deltas are the actual bounded score movement after clamping rather than the larger requested change | `reputation_system` (`apply_reputation_delta` is the single score mutation path; consequence composition and decay are tick passes) | `src/reputation/reputation_system.rs` |

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
On rejection authoritative state is unchanged unless the contract explicitly records
a diagnostic (e.g. a `HistoryEvent`).

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
| `enterprise_rng` | `simulation.enterprise_rng` | enterprise gross variance + vice-attention roll (both **unconditionally** per cycle) | `draw_basis_point_variance` + `draw_index(10_000)` |

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
   previously disjoint authorized booking is later reached by an unresolved decision pause.
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
