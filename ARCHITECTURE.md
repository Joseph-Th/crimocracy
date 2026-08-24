# Architecture

Owns ownership, mutation, determinism, persistence, and invariant contracts. Scope is in [`STATUS.md`](STATUS.md); verification is in [`TESTING.md`](TESTING.md); intent is in [`GAME_DESIGN.md`](GAME_DESIGN.md).

## Program model

- **Registry** — immutable authored definitions and validated lookup tables.
- **AppState** — serializable mutable campaign state. Typed IDs, clocks, and state-owned RNG streams.
- **Records** — typed identity, lifecycle, references, and version state.
- **Systems** — validate requests, derive decisions, commit authoritative mutation, preserve invariants.
- **Indexes and projections** — derived views maintained from authoritative records. Never independent truth.
- **Adapters** — own filesystem, process, network, and UI effects. Core systems receive explicit data and return explicit outcomes.

Static definitions describe what may exist. Runtime records describe what does exist. Mutable progress does not live in the registry. Future-affecting generated state is persisted, not reconstructed.

## Runtime flow

1. `content::build_registry` builds the validated immutable registry.
2. `AppState::new(seed)` creates serializable state and state-owned RNG streams.
3. A domain system validates a request or derives a read-only decision from registry and state.
4. The validated operation commits through its owning system, preserving records and indexes atomically.
5. [`core::simulation::run_tick`](src/core/simulation.rs) advances one simulated minute and resolves due work in stable order.
6. `TickOutcome`, reports, and read-only projections expose consequences without leaking hidden state as player knowledge.
7. Persistence serializes the envelope; load validates it before the state becomes trusted runtime state.

Tick cadence is an adapter concern. Calling `run_tick` faster or slower changes wall time, not the semantics of one canonical minute.

## Source map

Every `src/` subsystem owns its records and canonical mutation paths. Invariants are validated by `src/core/invariants/`. The top-level tick is `core::simulation::run_tick`, driven from `AppState` and `Registry`.

| Module | Owns | Canonical mutation |
|---|---|---|
| `core/` | `SimTime`/`SimDuration`, typed persistent IDs, entity refs, attention classes, `AppState`, persistence envelope, tick pipeline, invariant validation | `core::simulation` runs the tick; `core::state` owns generated state |
| `registry/` | Immutable authored definitions and validated lookups | Read-only after `content::build_registry` |
| `content/` | Code-owned authored definitions for the registry | `build_registry` |
| `world/` | Organizations, characters, neighborhoods, businesses, institutional profiles, designation, daily payroll; read-only territory-influence aggregation | `world_system` (insertion, designation); active-record requirements for assignments and ownership; `payroll_execution` (autonomous wage pass: ledger charges via `finance_system`, shortfall resentment via `relationship_system`, shortfall reports via `report_system`); `territory_influence` (read-only district summaries for simulation-side consumers, never a player omniscience feed) |
| `social/` | Directional character relationships with source/target indexes | `relationship_system` only; requires active endpoints |
| `intelligence/` | Provenance-bearing information, holder/topic indexes, lineage | `intelligence_system` (record, transfer) |
| `reports/` | Player-facing reports, briefs, financial reports | `report_system` |
| `history/` | Durable entity-linked campaign events | `history_system` |
| `finance/` | Typed accounts and balanced ledger; laundering transfers | `finance_system` (all financial mutations, including `validate_launder_funds`) |
| `operations/` | Operation plans, execution records, participant reservations, surveillance/police/property integrations, take economics | `operation_system` (lifecycle), `operation_execution` (deterministic resolution), and `operation_economics` (proceeds and depletion) |
| `opportunities/` | Provenance-backed opportunities with lifecycle | `opportunity_system` |
| `decisions/` | Durable typed decision records and pending indexes | `decision_system` |
| `delegation/` | Organization-owned mandates and responsibility indexes | `delegation_system` |
| `enterprises/` | Routine criminal enterprises and cycle history; delegated daily expansion for non-player organizations through canonical establishment | `enterprise_execution` (lifecycle/settlement), `autonomous_expansion` (daily delegated expansion), `enterprise_reporting` (read-only) |
| `economy/` | Legitimate business economies, cycle history, sabotage disruption horizons, chronic-loss suspension | `business_economy_system` (establishment/settlement/disruption/suspension), `business_reporting` (read-only) |
| `legal/` | Jurisdictions, patrols, investigations/evidence, arrests/custody, representation, prosecution, witnesses, informants, police response, investigator-held case knowledge | Named modules (`jurisdiction_system`, `patrol_system`, `investigation_system`, `arrest_system`, …) via `legal_state`; `case_knowledge` records lead-investigator activity knowledge through `intelligence_system` |
| `contacts/` | Institutional contacts and provenance-preserving disclosures | `contact_system` (establishment, termination, disclosure; `find_pending_disclosure_sources` read-only offer surface) |
| `recruitment/` | Relationship-gated recruitment, cooldowns, approvals, membership changes | `recruitment_system` |
| `reputation/` | Contextual per-audience organizational standing with baseline decay; operation consequences feed it, recruitment scoring and expansion posture consume it, player shifts surface as Standing reports | `reputation_system` (`apply_reputation_delta` is the single mutation path; consequences, feedback, and decay are tick passes) |

Adapters, the harness at [`examples/gameplay_harness/`](examples/gameplay_harness/main.rs), and verification at [`scripts/verify.ps1`](scripts/verify.ps1) / [`scripts/verify.cmd`](scripts/verify.cmd) live outside `src/` and use the canonical paths above.

## Canonical operations

One production path per operation class. UI, tests, examples, importers, and tools use the same semantics.

**Decide then apply** — decision reads broader state than it mutates:

```text
decide_*(&state, ...) -> Plan / Outcome / Delta
apply_*(&mut state, plan)
```

Decision is read-only except for explicitly supplied deterministic randomness.

**Validate then commit** — fallible multi-resource operations:

```text
validate_*(&state, ...) -> Validated*
Validated*::commit(self, &mut state)
```

Validation resolves references, permissions, lifecycle, ownership, capacity, ranges, and arithmetic before any consequential mutation. Commit consumes the validated value and rechecks authorization if staleness can invalidate it.

Single-owner operations may mutate directly when every return path preserves that owner's invariants and indexes.

## Data ownership

- One private owner per consequential field — the smallest owner that can keep it coherent.
- Collections that must agree are private fields of one owner, changed atomically (`insert`, `remove`, `move`, `reassign`).
- Cross-owner coordination goes through a system or higher orchestration boundary. Owners do not patch each other's private state.
- Importers, migrations, tests, and tools do not bypass owner methods.
- Durable references use typed identity where the project controls the vocabulary. Display text is not identity.

## Determinism

Authoritative behavior is determined by registry definitions, serialized state, ordered explicit inputs, state-owned RNG, and any explicitly modeled external snapshot.

- Result-affecting randomness comes from state-owned or explicitly injected deterministic RNG only.
- Order-sensitive work uses ordered collections or explicit sorting with complete stable tie-breakers.
- Wall-clock time, filesystem iteration, hash iteration, thread scheduling, UI timing, and ambient entropy are not simulation inputs.
- Parallelism may change throughput, not authoritative semantics.
- Top-level scheduling order is visible in one orchestration surface.

## Persistence

Save/load preserves every value required for continuation: IDs, relationships, lifecycle, counters, generated definitions, RNG state, and active durable work.

- Cross-references and invariants are validated before loaded state becomes trusted.
- Derived indexes are rebuilt only from persisted authoritative records and must agree after reconstruction.
- Missing future-affecting values are not silently defaulted to make old data load.
- Compatibility policy is in [`STATUS.md`](STATUS.md).
- Core systems do not perform implicit filesystem IO.

## Runtime invariants

1. Required registry and runtime references resolve.
2. Records appear exactly where required in derived indexes.
3. Exclusive ownership is represented once.
4. Lifecycle state agrees with active, scheduled, and indexed membership.
5. Multi-record operations commit completely or not at all.
6. IDs, handles, events, and outcomes have an owner and valid location.
7. Deterministic selection uses stable ordering and tie-breaking.
8. A character cannot hold overlapping non-terminal operation assignments; terminal operations release participants.
9. Static definitions contain no mutable runtime state.
10. Save/load preserves all future-affecting state.
11. Derived counters and projections agree with source records.
12. External effects cross explicit adapter boundaries.
13. Rejected operations preserve authoritative state except for explicitly modeled diagnostics.

Maintain `validate_invariants(state)` for structural checks. The soak exercises mixed state under invariant validation; [`TESTING.md`](TESTING.md) owns how it is run.

The ledger enforces balance and overflow, not solvency: a validated settlement may drive an operating account negative, recording an obligation rather than rejecting the cycle. Domain owners decide suspension or closure consequences; the ledger itself never silently clamps balances.

## API and representation

- Prefer explicit structs and project-owned enums over string-keyed registries for closed vocabularies.
- Match project-owned enums exhaustively; wildcards are for open or third-party vocabularies only.
- Map closed records explicitly so adding a field cannot silently disappear.
- Use typed error enums for new fallible domain operations; variants identify the failed precondition.
- Pass the narrowest context each phase needs. Read-only phases take `&state`; mutation phases take only the owner access required.
- Public surface is intentional. Do not expose helpers solely for tests.

## Naming and modules

Single vocabulary across subsystems:

| Purpose | Form |
|---|---|
| keyed lookup | `get_*` |
| conditional scan | `find_*` |
| final derivation | `resolve_*` |
| state-owned randomness draw | `draw_*` |
| plain accessor | noun form, e.g. `status()` |
| construction | `new()` |
| aggregate assembly | `build_*` |
| authored definition registration | `register_*` |
| runtime insertion/removal | `insert_*`, `remove_*` |
| in-place record update | `set_*` (field/status writes on an owned record) |
| read-only decision | `decide_*` |
| checked command | `validate_*` → `Validated*` |
| resolved mutation | `apply_*` or consuming `commit` |

Predicates use `is_`, `has_`, or `can_`. Do not introduce new `create_*`, `make_*`, `execute_*`, `perform_*`, or `attempt_*` when an established role already fits.

Multi-file suffixes use established roles: `_execution`, `_integration`, `_loader`, `_ui`, `_adapter`. Every `src/` file starts with a concise `//!` purpose statement; multi-file subsystems state sibling relationships where not obvious. Comments explain constraints, ordering, safety, invariants, or non-obvious intent — not history or commented-out code.

## Dead code and replacement

- Behavior that should run is wired into the canonical path.
- Test-only fixtures and helpers live under `#[cfg(test)]`.
- Obsolete behavior is deleted with its tests and documentation. No historical shims.
- Do not add fake call sites, broad `allow(dead_code)`, public shims, or test-only production APIs to silence warnings.

One implementation owns each concern unless an active external compatibility contract explicitly requires otherwise.
