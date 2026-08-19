# Current Status

This document is the scope authority for the implemented Crimocracy foundation. It lists supported capability and explicit exclusions. [`ARCHITECTURE.md`](ARCHITECTURE.md) owns technical contracts, [`TESTING.md`](TESTING.md) owns verification, and [`GAME_DESIGN.md`](GAME_DESIGN.md) owns product intent.

## Runtime foundation

- `AppState` owns serializable campaign state, simulation time, typed ID counters, attention settings, and independent deterministic RNG streams for generic, operation, investigation, business, and enterprise work.
- `Registry` contains immutable Rust-authored definitions. Runtime records and generated values belong to `AppState` and its domain owners.
- Save/load validates the envelope, state schema, authored content revision, registry references, cross-references, indexes, and ID high-water marks before accepting state. Compatibility is current-version only; there is no implicit migration or defaulting path.
- [`core::simulation::run_tick`](src/core/simulation.rs) is the canonical one-minute pipeline. It processes due work in stable order and returns a structured `TickOutcome`.
- `validate_state` provides release-safe structural checks. Debug simulation boundaries additionally run the full invariant validator.

## Implemented domains

| Domain | Implemented surface | Owner and entry point |
| --- | --- | --- |
| World | Organizations, characters, neighborhoods, businesses, institutional profiles, membership, supervision, and designation | `src/world/`; `world_system` |
| Social | Directional relationships with source and target indexes | `src/social/`; `relationship_system` |
| Intelligence | Provenance-bearing information, typed topics, holder indexes, transfers, and lineage | `src/intelligence/`; `intelligence_system` |
| Reports and history | Player-facing reports, executive briefs, financial reports, and entity-linked campaign events | `src/reports/`, `src/history/` |
| Finance | Typed accounts and balanced multi-account ledger transactions | `src/finance/`; `finance_system` |
| Operations | Semantic plans, objectives, approaches, roles, participant availability reservations, intelligence, timing, contingencies, deterministic outcomes, exposure, surveillance, police response, after-action records, and property disposition | `src/operations/`; `operation_system` and `operation_execution` |
| Opportunities | Information-backed discovery, open/dismissed/expired/converted lifecycle, provenance, and operation conversion | `src/opportunities/`; `opportunity_system` |
| Decisions | Durable typed requests, recipient/context indexes, versioned resolution, and attention classes | `src/decisions/`; `decision_system` |
| Delegation | Organization-owned mandates, responsibility scopes, policy overrides, budget authority, revision, revocation, and dependency checks | `src/delegation/`; `delegation_system` |
| Recruitment | Relationship-gated personnel recruitment, defection, cooldowns, executive approval, delegated autonomy, and canonical membership reassignment | `src/recruitment/`; `recruitment_system` |
| Enterprises | Routine criminal enterprises, authored economics, manager authority, scheduled cycles, balanced settlement, and financial reporting | `src/enterprises/`; `enterprise_execution` and reporting modules |
| Legitimate economy | Business operating economies, ownership transfer/history, scheduled cycles, authored economics, accounting information, and comparative reporting | `src/economy/`; `business_economy_system` and reporting modules |
| Legal institutions | Jurisdictions, patrol deployments, timed police response, investigations, evidence graphs, staffing, detective work, cold cases, arrests/custody, representation, prosecution referral, witnesses, and informants | `src/legal/`; named legal system modules |
| Institutional contacts | Person-mediated Police, Legal, Political, Press, Labor, and Professional information channels with provenance-preserving disclosure | `src/contacts/`; `contact_system` |

## Cross-cutting guarantees

- Consequential mutation has one owning system and one canonical production path.
- Fallible multi-record operations validate references, ownership, lifecycle, permissions, capacity, arithmetic, and versions before commit.
- Rejected operations preserve authoritative state except for explicitly modeled diagnostics or audit records.
- Derived indexes and summaries are rebuilt or updated only from authoritative records and are checked for consistency.
- Result-affecting randomness uses serialized state-owned streams. Ordered work uses explicit stable ordering and tie-breakers.
- Player-visible information is separate from hidden world truth. Reports point to information records; adapters and harness diagnostics do not grant omniscience.
- External filesystem, process, network, and UI effects remain outside core systems.
- The gameplay harness exercises production paths with authored fixtures and deterministic policy comparisons. It is not evidence of human comprehension, enjoyment, or interface quality.

## Explicit exclusions

The current foundation does not model:

- detailed city geography, transport topology, population simulation, broad institution networks, supply chains, inventory quantities, acquisition pricing, market competition, or dynamic citywide supply and demand;
- broader employment, wages, family ties, secrets, injuries, general character needs, specialist affiliations, or universal autonomous behavior;
- delegated role staffing, broad resource competition beyond overlapping character assignments, diplomacy, territory strategy, multi-step rival planning, or general campaign-pressure generation;
- reusable equipment and vehicles, tactical movement, combat control, pursuit, dispatch capacity, casualties, injuries, asset damage, or condition-specific tactical consequences;
- autonomous investigative lead generation beyond the modeled evidence review and case-graph work, case merging, charging, bail, trial, conviction, acquittal, court procedure, corruption, political procedure, press behavior, or labor institutions;
- a universal approval flag, generic legal-pressure meter, or generic mission-card generator. Modeled approvals, legal pressure, and opportunities remain typed and domain-owned.

These exclusions are scope boundaries, not implementation evidence for unmodeled design goals.

## Version facts

The current authored content revision is 16.

The current in-memory state schema version is 41.

The compiled operation vocabulary contains only constraints and contingencies with corresponding execution inputs and outcomes. Unsupported tactical axes are not represented as inert plan fields.
