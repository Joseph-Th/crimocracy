# Current Status

Scope authority: what the foundation implements and what it explicitly excludes.
Ownership is in [`ARCHITECTURE.md`](ARCHITECTURE.md); verification is in
[`TESTING.md`](TESTING.md); intent is in [`GAME_DESIGN.md`](GAME_DESIGN.md);
agent routing is in [`AGENTS.md`](AGENTS.md).

> **Agent quick-find:** the foundation is 11+1 domains (see table below) on the
> `Registry → AppState (15 substates) → run_tick (14 phases)` tower described in
> `AGENTS.md:§2` and `ARCHITECTURE.md`. To add capability, consult
> `AGENTS.md:§10` accretion guide. Version facts are at the bottom — keep them
> in sync with `src/core/state.rs` and `src/content/mod.rs`.

## Runtime foundation

- `AppState` owns serializable campaign state, simulation time, typed ID counters, attention settings, and independent deterministic RNG streams for operation, investigation, business, enterprise, and recruitment work.
- `Registry` owns immutable Rust-authored definitions. Runtime records and generated values belong to `AppState` and its domain owners.
- Save/load validates envelope, schema, authored content revision, registry references, cross-references, indexes, and ID high-water marks before accepting state. Compatibility is current-version only; no implicit migration or defaulting.
- [`core::simulation::run_tick`](src/core/simulation.rs) is the canonical one-minute pipeline: it processes due work in stable order and returns `TickOutcome`.
- `validate_state` provides release-safe structural checks; debug boundaries additionally run the full invariant validator.

## Implemented domains

| Domain | Capability | Owner and entry point |
| --- | --- | --- |
| World | Organizations, characters, neighborhoods, businesses, institutional profiles, membership and supervision, player designation, daily payroll (available cash is shared evenly to the cent across active members; underpaid members gain resentment and the player organization receives a shortfall report); read-only per-district territory-influence summaries over enterprise records | `src/world/` via `world_system`, `payroll_execution`, `territory_influence` |
| Organizational policies | Behavioral standing policies consumed by recruitment autonomy and automatic legal-support retention | `src/world/` settings via `delegation_system` overrides and `legal_representation_system` execution |
| Social | Directional relationships with source and target indexes | `src/social/` via `relationship_system` |
| Intelligence | Provenance-bearing information with typed topics, holder indexes, transfers, lineage | `src/intelligence/` via `intelligence_system` |
| Reports and history | Player-facing reports, executive briefs, financial reports, Notable standing reports when the player organization's own street reputation shifts, entity-linked campaign events | `src/reports/` and `src/history/` |
| Finance | Typed accounts, allocator-neutral planned account openings for composite transactions, and balanced multi-account ledger | `src/finance/` via `finance_system` |
| Operations | Semantic plans (objectives, approaches, roles, participant reservations, intelligence, timing, contingencies) resolving deterministically through exposure, surveillance, and police response; after-action records; debrief-derived district police knowledge from pre-entry police-arrival aborts; recent-take depletion on repeated targets; venue-sensitive property disposition with proceeds held as cash; custody release through extraction; sabotage-driven business disruption | `src/operations/` via `operation_system` and `operation_execution` |
| Opportunities | Information-backed discovery with an open/dismissed/expired/converted lifecycle | `src/opportunities/` via `opportunity_system` |
| Decisions | Durable typed requests with recipient/context indexes, versioned resolution, attention classes | `src/decisions/` via `decision_system` |
| Delegation | Organization-owned mandates with responsibility scopes, policy overrides, budget authority, revision, revocation, dependency checks | `src/delegation/` via `delegation_system` |
| Recruitment | Relationship-gated recruitment, defection, cooldowns, executive approval, delegated autonomy (RequireApproval managers raise durable exception approvals; non-player organizations resolve their own queue in-pass), one exclusive route per organization-candidate pair across all channels, canonical membership reassignment, refused-approach loyalty reporting to the candidate's organization, underworld competence reputation as an authored scoring term | `src/recruitment/` via `recruitment_system` and `scoring` |
| Enterprises | Routine criminal enterprises (protection, gambling, alcohol distribution, bookmaking, loan sharking, fencing) with authored economics, venue-function requirements version-pinned at establishment and resumption, one racket of a kind per location including suspended ones, manager authority, scheduled cycles settling balanced; active district casework adds a street-heat surcharge reported on appearance or change (an unchanged surcharge settles as routine cost) and rolls per-cycle vice attention that can open an originated inquiry on the racket itself with organization-held legal knowledge; chronic net loss suspends at the authored threshold, counted since the latest resumption; financial reporting; daily delegated expansion for non-player organizations gated by police-fear posture with influence-aware district consolidation | `src/enterprises/` via `enterprise_execution`, `autonomous_expansion`, and reporting modules |
| Legitimate economy | Business operating economies with ownership transfer/history, scheduled cycles, authored economics including the per-kind acquisition price, accounting information, comparative reporting, sabotage disruption horizons with degraded earning power, chronic-loss suspension counted since the latest resumption; organizations buy independently owned businesses outright at that price in accounted funds, capitalizing acquired books, opening the first operating economy when none exists, reopening suspended books; acquisitions surface as Notable financial reports for the buyer | `src/economy/` via `business_economy_system`, `business_acquisition`, and reporting modules |
| Money laundering | Street-cash-to-accounted-funds conversion through owned cash-intensive fronts with an authored fee split to the front's operating account; per-cycle plausibility budget capped at an authored fraction of the front's current gross potential (sabotage-degraded books shrink it), so splitting one sum across transfers cannot exceed it; commit pins both the front's business ownership/version and its economy version, and the source must hold the laundered cash. Accounted wealth is the only money that can buy a legitimate business, so laundering throughput gates legitimate expansion | `src/finance/` via `finance_system::validate_launder_funds`; purchases via `economy::business_acquisition` |
| Reputation | Contextual per-audience organizational standing across fear/reliability/competence/treachery; sparse records created at baseline on first touch and erased when decay returns them to baseline; deterministic operation-consequence producers (success competence, exposure fear, violent-approach business fear; clamped scores are not reported as movement) atomically pair player-visible shifts with their Standing report; police fear from an owned racket drawing a vice inquiry throttles delegated expansion while it decays; daily baseline decay; underworld competence feeds recruitment scoring | `src/reputation/` via `reputation_system` (`apply_reputation_delta` is the single score mutation path) |
| Legal institutions | Jurisdictions, patrol deployments, timed police response; investigations originated by operation exposure or enterprise vice attention (only originated cases decay cold; shelving releases investigators and a later incident sharing the shelf's subject matter resumes the existing file rather than opening a parallel case); evidence graphs; single-seat staffing with investigator-held case-activity knowledge refreshed atomically on every lifecycle transition and lead assignment; detective work; witness interviews producing named testimony; autonomous evidence-threshold arrests counting only independent evidence toward the two-item bar; detainee informant recruitment and disclosures; representation (automatic-policy retention consults organization policy under the defendant's supervisor's mandate override and concludes after custody ends; explicitly commanded retention persists); prosecution referral/review whose final terminal review releases the originating detention when no other prosecutor office still reviews that arrest | `src/legal/` named system modules; invariant validation split per aggregate under `src/core/invariants/legal/` |
| Institutional contacts | Person-mediated Police, Legal, Political, Press, Labor, and Professional channels with provenance-preserving disclosure and a read-only pending-disclosure offer surface | `src/contacts/` via `contact_system` |

## Cross-cutting guarantees

- One owning system and one canonical path per consequential mutation.
- Fallible multi-record operations validate references, ownership, lifecycle, permissions, capacity, arithmetic, and versions before commit.
- Rejected operations preserve authoritative state except for explicitly modeled diagnostics or audit records.
- Derived indexes and summaries are maintained only from authoritative records and checked for consistency.
- Result-affecting randomness uses serialized state-owned streams; ordered work uses explicit stable ordering.
- Player-visible information is separate from hidden world truth. Reports point to information records; adapters and harness diagnostics do not grant omniscience.
- Filesystem, process, network, and UI effects remain outside core systems.
- The harness exercises production paths with authored fixtures and deterministic policy comparisons. It does not prove human comprehension or interface quality.

## Explicit exclusions

The foundation does not model:

- detailed city geography, transport topology, population simulation, broad institution networks, supply chains, inventory quantities, negotiated or dynamic acquisition pricing, market competition, or dynamic citywide supply and demand;
- broader employment, wages, family ties, secrets, injuries, general character needs, specialist affiliations, or universal autonomous behavior;
- delegated role staffing, broad resource competition beyond overlapping character assignments, diplomacy, territory strategy, multi-step rival planning, or general campaign-pressure generation;
- durable receivable/payable obligations for unpaid AI-organization payroll: a shortfall models its consequence through crew resentment (and, for the player organization, a shortfall report), not as carried debt;
- reusable equipment and vehicles, tactical movement, combat control, pursuit, dispatch capacity, casualties, injuries, asset damage, or condition-specific tactical consequences;
- autonomous investigative lead generation beyond modeled evidence review and case-graph work, case merging, charging, bail, trial, conviction, acquittal, court procedure, corruption, political procedure, press behavior, or labor institutions;
- a universal approval flag, generic legal-pressure meter, or generic mission-card generator. Modeled approvals, legal pressure, and opportunities remain typed and domain-owned.

These are scope boundaries, not evidence for unmodeled design goals.

## Version facts — single source of truth for persistence compat

These two numbers gate `core::persistence::restore_save` (`src/core/persistence.rs`):
mismatched saves are rejected. Keep them in sync with the owners.

The current authored content revision is 35.

The current in-memory state schema version is 66.

The compiled operation vocabulary contains only objectives, constraints, and contingencies with corresponding execution inputs and outcomes, and the policy vocabulary contains only settings with a consuming system. Unsupported tactical or governance axes are not represented as inert plan fields.
