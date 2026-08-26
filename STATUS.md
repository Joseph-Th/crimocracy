# Current Status

Scope authority: what the foundation implements and what it explicitly excludes. Ownership is in [`ARCHITECTURE.md`](ARCHITECTURE.md); verification is in [`TESTING.md`](TESTING.md); intent is in [`GAME_DESIGN.md`](GAME_DESIGN.md).

## Runtime foundation

- `AppState` owns serializable campaign state, simulation time, typed ID counters, attention settings, and independent deterministic RNG streams for operation, investigation, business, enterprise, and recruitment work.
- `Registry` owns immutable Rust-authored definitions. Runtime records and generated values belong to `AppState` and its domain owners.
- Save/load validates envelope, schema, authored content revision, registry references, cross-references, indexes, and ID high-water marks before accepting state. Compatibility is current-version only; no implicit migration or defaulting.
- [`core::simulation::run_tick`](src/core/simulation.rs) is the canonical one-minute pipeline. It processes due work in stable order and returns `TickOutcome`.
- `validate_state` provides release-safe structural checks. Debug boundaries additionally run the full invariant validator.

## Implemented domains

| Domain | Capability | Owner and entry point |
| --- | --- | --- |
| World | Organizations, characters, neighborhoods, businesses, institutional profiles, membership, supervision, designation, daily payroll with shortfall resentment and reporting; read-only per-district territory-influence summaries over enterprise records | `src/world/` via `world_system`, `payroll_execution`, and `territory_influence` |
| Organizational policies | Behavioral standing policies consumed by recruitment autonomy and automatic legal-support retention | `src/world/` (settings) via `delegation_system` overrides, `legal_representation_system` execution |
| Social | Directional relationships with source and target indexes | `src/social/` via `relationship_system` |
| Intelligence | Provenance-bearing information, typed topics, holder indexes, transfers, lineage | `src/intelligence/` via `intelligence_system` |
| Reports and history | Player-facing reports (including Notable standing reports when the player organization's own street reputation shifts), executive briefs, financial reports, entity-linked campaign events | `src/reports/` and `src/history/` |
| Finance | Typed accounts and balanced multi-account ledger | `src/finance/` via `finance_system` |
| Operations | Semantic plans, objectives, approaches, roles, participant reservations, intelligence, timing, contingencies, deterministic outcomes, exposure, surveillance, police response, debrief-derived district police-activity knowledge on pre-entry police-arrival aborts, after-action records, recent-take depletion on repeated targets, venue-sensitive property disposition, held-cash deposition into organization accounts, custody release through extraction, sabotage-driven business disruption | `src/operations/` via `operation_system` and `operation_execution` |
| Opportunities | Information-backed discovery and open/dismissed/expired/converted lifecycle | `src/opportunities/` via `opportunity_system` |
| Decisions | Durable typed requests, recipient/context indexes, versioned resolution, attention classes | `src/decisions/` via `decision_system` |
| Delegation | Organization-owned mandates, responsibility scopes, policy overrides, budget authority, revision, revocation, dependency checks | `src/delegation/` via `delegation_system` |
| Recruitment | Relationship-gated recruitment, defection, cooldowns, executive approval, delegated autonomy (RequireApproval managers raise durable Exception approval requests; non-player organizations resolve their own queue in-pass by approving pitches whose pre-resolved assessment lands), one exclusive route per organization-candidate pair across all three channels, canonical membership reassignment, refused-approach loyalty reporting to the candidate's organization, organization competence reputation as an authored scoring term | `src/recruitment/` via `recruitment_system` and `scoring` |
| Enterprises | Routine criminal enterprises (protection, gambling, alcohol distribution, bookmaking, loan sharking, fencing), authored economics, venue-function requirements with version-pinned hosts at establishment and at resumption, one racket of a kind per location (including suspended ones), manager authority, district-heat surcharge from active investigations with player-visible manager reporting on appearance or change (a sustained identical surcharge settles as routine cost visible in financial summaries), scheduled cycles with drawn vice inquiries settling as Notable cycles, balanced settlement with chronic-loss suspension at the authored threshold (the losing-cycle count restarts at every resumption), financial reporting, per-cycle vice-attention rolls that convert sustained district casework into an originated inquiry on the racket itself with organization-held legal knowledge, daily delegated-autonomy expansion for non-player organizations with police-fear posture gating and influence-aware district consolidation preference | `src/enterprises/` via `enterprise_execution`, `autonomous_expansion`, and reporting modules |
| Legitimate economy | Business operating economies, ownership transfer/history, scheduled cycles, authored economics (including the authored per-kind acquisition price), accounting information, comparative reporting, sabotage disruption horizons with degraded earning power, chronic-loss suspension after an authored run of net-losing cycles (counted since the latest resumption); organizations buy independently owned businesses outright at that price, capitalizing the acquired books, opening the first operating economy when none exists, and reopening books a chronic-loss suspension left dormant; acquisitions surface as Notable financial reports for the buyer | `src/economy/` via `business_economy_system`, `business_acquisition`, and reporting modules |
| Money laundering | Street-cash-to-accounted-funds conversion through owned cash-intensive fronts, authored fee split to the front's operating account, per-cycle plausibility budget capped at an authored fraction of the front's current gross potential (sabotage-degraded books shrink it) so splitting one sum across transfers cannot exceed it; commit re-validates against the front's economy version and the source must actually hold the laundered street cash. Accounted wealth is the only money that can buy a legitimate business: the acquisition path rejects street and concealed cash, so laundering throughput gates legitimate expansion | `src/finance/` via `finance_system::validate_launder_funds`; purchases via `economy::business_acquisition` |
| Reputation | Contextual per-audience organizational standing across fear/reliability/competence/treachery, sparse records created at the authored baseline on first touch and erased when decay returns them to baseline, deterministic operation-consequence producers (success competence, exposure fear, violent-approach business fear; scores already clamped at a rail are not reported as movement), police fear from an owned racket drawing a dedicated vice inquiry (throttling delegated expansion while it decays), daily baseline decay, underworld competence as an authored recruitment-scoring term | `src/reputation/` via `reputation_system` |
| Legal institutions | Jurisdictions, patrol deployments, timed police response, investigations (originated by operation exposure or enterprise vice attention; only originated cases decay cold; shelving releases a case's investigators and a later incident sharing the shelf's subject matter resumes the existing file rather than opening a parallel case), evidence graphs, single-seat staffing with investigator-held case-activity knowledge refreshed on every lifecycle transition and lead assignment, detective work, witness interviews and named testimony, cold cases, autonomous evidence-threshold arrests/custody counting only independent evidence (a forensic derivative cannot corroborate its own source into the two-item bar), detainee informant recruitment and disclosures, representation (automatic-policy retention consults organization policy with the defendant's supervisor's mandate override taking precedence, and is swept at custody release; explicitly commanded retention is not), prosecution referral | `src/legal/` via named legal system modules; invariant validation split per aggregate under `src/core/invariants/legal/` |
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

- detailed city geography, transport topology, population simulation, broad institution networks, supply chains, inventory quantities, acquisition pricing, market competition, or dynamic citywide supply and demand;
- broader employment, wages, family ties, secrets, injuries, general character needs, specialist affiliations, or universal autonomous behavior;
- delegated role staffing, broad resource competition beyond overlapping character assignments, diplomacy, territory strategy, multi-step rival planning, or general campaign-pressure generation;
- durable receivable/payable obligations for unpaid AI-organization payroll: a shortfall models its consequence through crew resentment (and, for the player organization, a shortfall report), not as carried debt;
- reusable equipment and vehicles, tactical movement, combat control, pursuit, dispatch capacity, casualties, injuries, asset damage, or condition-specific tactical consequences;
- autonomous investigative lead generation beyond modeled evidence review and case-graph work, case merging, charging, bail, trial, conviction, acquittal, court procedure, corruption, political procedure, press behavior, or labor institutions;
- a universal approval flag, generic legal-pressure meter, or generic mission-card generator. Modeled approvals, legal pressure, and opportunities remain typed and domain-owned.

These are scope boundaries, not evidence for unmodeled design goals.

## Version facts

The current authored content revision is 33.

The current in-memory state schema version is 65.

The compiled operation vocabulary contains only objectives, constraints, and contingencies with corresponding execution inputs and outcomes, and the policy vocabulary contains only settings with a consuming system. Unsupported tactical or governance axes are not represented as inert plan fields.
