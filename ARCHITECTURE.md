# Architecture

This document owns Crimocracy's implemented ownership, mutation, determinism, persistence, and invariant contracts. Use [`STATUS.md`](STATUS.md) for scope, [`TESTING.md`](TESTING.md) for proof, and [`GAME_DESIGN.md`](GAME_DESIGN.md) for product intent.

## Program model

Crimocracy uses an explicit Registry / AppState / Record / System model.

- **Registry** owns immutable definitions and validated lookup tables.
- **AppState** owns generated mutable state required for continued execution.
- **Records** own typed identity, local durable values, lifecycle, references, and version state.
- **Systems** validate requests, derive decisions, commit authoritative mutation, and preserve invariants.
- **Indexes and projections** are derived views maintained from authoritative records; they are never independent truth.
- **Adapters** own filesystem, process, network, UI, and other external effects. Core systems receive explicit data and return explicit outcomes.

Static definitions describe what may exist. Runtime records describe what does exist. Mutable progress must not live in registry definitions, and generated future-affecting state must not be reconstructed plausibly after load when exact continuation requires persistence.

## Runtime flow

The normal execution path is:

1. `content::build_registry` constructs validated immutable definitions.
2. `AppState::new(seed)` creates serializable runtime state and state-owned RNG streams.
3. A domain system validates a request or derives a read-only decision from the registry and state.
4. The validated operation commits through its owning system, preserving records and indexes atomically.
5. `core::simulation::run_tick` advances one simulated minute and resolves due work in stable order.
6. Reports, `TickOutcome`, read-only projections, or adapter responses expose consequences without exposing hidden state as player knowledge.
7. Persistence serializes the state envelope; load validates it before the state becomes trusted runtime state.

Simulation speed is an adapter concern. Calling the tick more or less often changes elapsed wall time, not the semantics of one canonical simulated minute.

## Source map

Every `src/` subsystem owns its records and canonical mutation paths and is validated by `src/core/invariants/`. The top-level execution boundary is `core::simulation::run_tick`, driven from `AppState` and the immutable `Registry`.

| Module | Owns | Canonical mutation |
|---|---|---|
| `core/` | Deterministic `SimTime`/`SimDuration`, typed persistent IDs, entity refs, attention classes, serializable `AppState`, persistence envelope, top-level tick, invariant validation | `core::simulation` runs the deterministic tick pipeline; `core::state` is the single owner of generated state |
| `registry/` | Immutable authored definitions and validated lookup tables (`typedef`-safe `Registry`) | Read-only after construction; `content::build_registry` assembles it |
| `content/` | Code-owned authored definitions for the startup registry | `build_registry` |
| `world/` | Organizations, characters, neighborhoods, businesses, institutional profiles, organization designation | `world_system` owns insertion and designation mutations |
| `social/` | Directional character relationships with source/target indexes | `relationship_system` is the sole relationship mutation path |
| `intelligence/` | Provenance-bearing information records and holder/topic indexes | `intelligence_system` records knowledge and executes canonical transfers |
| `reports/` | Player-facing reports and synthesized briefs/financial reports | `report_system` validates and inserts; synthesis modules build artifacts |
| `history/` | Durable entity-linked campaign events | `history_system` owns insertion/indexing |
| `finance/` | Typed monetary accounts and balanced ledger | `finance_system` owns every financial mutation, including multi-account transactions |
| `operations/` | Semantic operation plans, execution records, participant availability reservations, surveillance/police/property integrations | `operation_system` authorizes and commits lifecycle; `operation_execution` resolves outcomes deterministically |
| `opportunities/` | Provenance-backed operation opportunities with lifecycle | `opportunity_system` owns discovery and lifecycle transitions |
| `decisions/` | Durable typed decision records and pending indexes | `decision_system` owns request resolution |
| `delegation/` | Persistent manager mandates and responsibility indexes | `delegation_system` owns assignment, revision, revocation, policy resolution |
| `enterprises/` | Routine criminal enterprises and cycle history | `enterprise_execution` owns lifecycle and settlement; `enterprise_reporting` aggregates read-only |
| `economy/` | Legitimate business operating economies and cycle history | `business_economy_system` owns establishment and cycle settlement; `business_reporting` aggregates |
| `legal/` | Jurisdictions, patrol deployments, investigations/evidence, arrests/custody, representation, prosecution, witnesses, informants, police response | Named legal system modules (`jurisdiction_system`, `patrol_system`, `investigation_system`, `arrest_system`, ...) mutate through their own transactions; `legal_state` keeps records and indexes synchronized |
| `contacts/` | Institutional contacts and provenance-preserving disclosures | `contact_system` owns establishment, termination, disclosure |
| `recruitment/` | Relationship-gated recruitment, cooldowns, approvals, membership changes | `recruitment_system` owns candidate discovery, decisions, and membership |

Adapters, the gameplay harness (`examples/gameplay_harness.rs`), and verification tooling (`scripts/verify.cmd` / `scripts/verify.ps1`) live outside `src/` and must go through the canonical production paths above.

## Canonical operations

Every consequential operation class has one production path. UI, examples, tests, importers, migrations, and administrative tools use the same semantics rather than creating parallel implementations.

### Decide then apply

Use when a decision reads broader state than it mutates:

```text
decide_*(&state, ...) -> Plan / Outcome / Delta
apply_*(&mut state, plan)
```

The decision phase is read-only apart from explicitly supplied deterministic randomness. Apply consumes the result in the same pipeline.

### Validate then commit

Use for fallible multi-resource operations:

```text
validate_*(&state, ...) -> Validated*
Validated*::commit(self, &mut state)
```

Validation resolves every relevant reference, permission, lifecycle, range, ownership, capacity, and arithmetic precondition before the first consequential mutation. Commit consumes the validated value. When staleness can invalidate authorization, commit rechecks the relevant owner/revision before mutation.

Single-owner operations may mutate directly when every return path preserves that owner's invariants and derived indexes.

## Data ownership

- A consequential field is private to the smallest owner that can keep it coherent.
- Collections that must agree are private fields of one owner and change through atomic owner methods such as insert, remove, move, or reassign.
- Cross-owner coordination belongs in a system or higher orchestration boundary; one owner does not patch another owner's private state.
- Importers, migrations, tests, and administrative tooling do not bypass owner methods.
- Durable references use explicit typed identity where the project controls the vocabulary. User-facing text is not identity.

## Determinism

Authoritative behavior is determined by immutable registry definitions, serialized runtime state, ordered explicit inputs, state-owned random streams, and any explicitly modeled external snapshot.

- All result-affecting randomness comes from state-owned or explicitly injected deterministic RNG.
- Order-sensitive work uses ordered collections or explicit sorting with complete stable tie-breakers.
- Wall-clock time, filesystem iteration, hash iteration, thread scheduling, UI timing, and ambient entropy are not simulation inputs.
- Parallel implementation may change throughput, not authoritative semantics.
- Top-level scheduled execution order remains visible in one orchestration surface; load-bearing ordering is documented at that boundary.

## Persistence

Save/load preserves every value required for supported continuation, including IDs, relationships, lifecycle state, counters, generated definitions, random state, and active durable work.

- Cross-references and complete invariants are validated before loaded state becomes trusted runtime state.
- Derived indexes may be rebuilt only from authoritative persisted records and must agree with those records after reconstruction.
- Missing future-affecting values are not silently invented merely to make old data load.
- The current compatibility policy is owned by `STATUS.md`.
- Core systems do not perform implicit filesystem IO.

## Runtime invariants

The state model preserves at least these invariant classes:

1. Required registry and runtime references resolve.
2. Records appear exactly where required in derived indexes.
3. Exclusive ownership is represented once.
4. Lifecycle state agrees with active, scheduled, and indexed membership.
5. Multi-record operations commit completely or not at all.
6. Generated IDs, handles, events, and durable outcomes always have an owner and valid location.
7. Deterministic selection uses stable ordering and tie-breaking.
8. A character cannot be assigned to overlapping non-terminal operations; terminal operations release the participant.
9. Static definitions contain no mutable runtime state.
10. Save/load preserves all future-affecting state.
11. Derived counters, summaries, caches, and projections agree with source records.
12. External effects cross explicit adapter boundaries.
13. Rejected operations preserve authoritative state except for explicitly modeled diagnostics or audit records.

Maintain `validate_invariants(state)` for cheap structural checks and add the relevant assertion when adding a new invariant. The deterministic soak exercises mixed state under invariant validation; `TESTING.md` owns how it is run.

## API and representation rules

- Prefer explicit structs and project-owned enums over string-keyed behavior registries for closed vocabularies.
- Match project-owned enums exhaustively; wildcard handling is for genuinely open or third-party vocabularies.
- Map closed project-owned records explicitly enough that adding a field cannot silently disappear from another shape.
- Use typed error enums for new fallible domain operations; error variants identify the failed precondition without requiring text parsing.
- Pass the narrowest context a phase needs. Read-only phases receive immutable access; mutation phases receive only the owner access they require.
- Public surface is intentional. Do not make production helpers public solely for tests.

## Naming and modules

Use one vocabulary across subsystems:

| Purpose | Form |
|---|---|
| keyed lookup | `get_*` |
| conditional scan | `find_*` |
| final derivation | `resolve_*` |
| plain accessor | noun form such as `status()` |
| plain construction | `new()` |
| aggregate assembly | `build_*` |
| authored definition registration | `register_*` |
| runtime insertion/removal | `insert_*`, `remove_*` |
| read-only decision | `decide_*` returning a plan/outcome/delta |
| checked command | `validate_*` returning `Validated*` when appropriate |
| resolved mutation | `apply_*` or consuming `commit` |

Predicates use `is_`, `has_`, or `can_`. New production `create_*`, `make_*`, `execute_*`, `perform_*`, or `attempt_*` names should not duplicate an established role.

Multi-file subsystem suffixes use established roles such as `_execution`, `_integration`, `_loader`, `_ui`, and `_adapter`. Every `src/` file begins with a concise `//!` purpose statement; multi-file subsystems also explain the relationship to siblings where it is not obvious.

Comments explain constraints, ordering, safety, invariants, or non-obvious intent. They do not narrate implementation history or retain commented-out code.

## Dead code and replacement

Treat dead code by ownership, not by warning suppression.

- Production behavior that should execute must be wired into the canonical path.
- Test-only fixtures and helpers belong under test configuration.
- Obsolete behavior is deleted together with tests/documentation that describe it.
- Do not add fake production call sites, broad dead-code allowances, public shims, or test-only production APIs merely to silence warnings.

One implementation owns each concern unless an active external compatibility contract explicitly requires otherwise.
