# Architecture

This document owns Crimocracy's implemented ownership, mutation, determinism, persistence, and invariant contracts. `STATUS.md` owns supported scope; `GAME_DESIGN.md` owns product intent; `TESTING.md` owns verification and gameplay-evidence rules.

## Program model

Crimocracy uses an explicit Registry / AppState / Record / System model.

- **Registry** owns immutable definitions and validated lookup tables.
- **AppState** owns generated mutable state required for continued execution.
- **Records** own typed identity, local durable values, lifecycle, references, and version state.
- **Systems** validate requests, derive decisions, commit authoritative mutation, and preserve invariants.
- **Indexes and projections** are derived views maintained from authoritative records; they are never independent truth.
- **Adapters** own filesystem, process, network, UI, and other external effects. Core systems receive explicit data and return explicit outcomes.

Static definitions describe what may exist. Runtime records describe what does exist. Mutable progress must not live in registry definitions, and generated future-affecting state must not be reconstructed plausibly after load when exact continuation requires persistence.

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
8. Static definitions contain no mutable runtime state.
9. Save/load preserves all future-affecting state.
10. Derived counters, summaries, caches, and projections agree with source records.
11. External effects cross explicit adapter boundaries.
12. Rejected operations preserve authoritative state except for explicitly modeled diagnostics or audit records.

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
