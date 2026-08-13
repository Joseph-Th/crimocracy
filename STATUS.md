# Crimocracy Foundation Status

This file records the architectural foundation currently present in the repository. It is not a feature roadmap and does not override `AGENTS.md` or `GAME_DESIGN.md`.

## Current Foundation

- Deterministic, serializable `AppState` owns all mutable campaign state and the state-owned RNG.
- Persistent references use typed `u32` IDs allocated by state-owned counters.
- Immutable authored definitions are assembled by the startup `Registry` from Rust content builders.
- World records cover organizations, characters, neighborhoods, and businesses with synchronized lookup indexes.
- Character organization membership and supervision changes use versioned validated reassignment tokens. Stale tokens cannot overwrite newer hierarchy state, and organization changes are rejected while a character owns active operational or delegated responsibility.
- Directional relationships are persistent records with bounded dimensions and a single mutation system.
- Intelligence is stored as provenance-bearing records with holder, source, subject, observation time, reliability, and specificity.
- Semantic operations model objective, approach, leader, roles, constraints, contingencies, schedule, and lifecycle.
- Authorized operations whose scheduled time arrives begin through the deterministic top-level tick pipeline in stable ID order. The canonical transition path rejects early starts.
- Authority exceptions are durable decision records with requester, recipient, context, attention class, offered responses, timestamps, lifecycle, and version. An operation in `AwaitingDecision` has exactly one pending decision and can leave that state only through decision resolution.
- Attention classes are shared simulation data (`Routine`, `Notable`, `Exception`, `Crisis`). Persistent campaign settings control which classes request adapter-level auto-pause behavior.
- Manager delegation is represented by persistent mandates with organization ownership, one active mandate per manager, geographic and functional responsibility scopes, standing policy overrides, optional budget authority, versioning, historical revocation, and synchronized indexes.
- Manager policy resolution is deterministic: an active mandate standing order overrides the organization policy for that manager; otherwise the organization policy is inherited.
- Investigations and evidence are specific persistent records with case ownership, subjects, custody, admissibility, and synchronized case-graph indexes.
- Reports store player-facing summaries linked to known information, entities, and optional durable decision IDs. Decision-linked reports remain valid after resolution because decision records are historical state.
- Campaign history stores durable entity-linked events for later timelines and campaign recall.
- Finance uses typed monetary accounts and a balanced double-entry-style ledger. Street cash, concealed cash, accounted funds, legitimate operating funds, receivables, payables, and settlement accounts are distinct states rather than one universal cash scalar.
- Multi-account financial transactions validate every posting and resulting balance before a consuming commit token atomically changes any account. Account versions prevent stale validated transactions from overwriting newer balances.
- Mandate budgets are tied to ledger history rather than duplicated spend counters. A budget specifies an organization-owned funding account, a limit, and a daily or weekly period. Authorized transactions persist resolved usage, cumulative limits are checked before mutation, and stale mandate revisions invalidate previously validated spending tokens.
- Save envelopes are versioned, content-revision checked, structurally validated, and preserve RNG continuation.
- The current in-memory state schema version is 6. Decision, delegation, character-version, mandate-budget, and ledger-budget-history state are part of the serialized deterministic continuation boundary.
- `validate_state` provides release-safe structural validation; debug builds additionally run `validate_invariants` at the top-level simulation boundary.
- A deterministic mixed-state soak test now covers scheduled state transitions, pending-decision reporting, authority-linked ledger spending, remaining budget capacity, and invariants over 5,000 ticks. Persistence tests cover RNG continuation, unresolved decision state with linked reports and customized attention settings, and budget history/remaining authority.

## Architectural Boundaries

- Core systems contain no file, network, database, UI, or process IO.
- Registries contain definitions only; generated campaign values remain in `AppState` or owned records.
- Indexes are derived state and are private to the owner that keeps them synchronized.
- Consequential cross-record mutation uses validation before commit. Multi-resource transactions use consuming validated tokens. Tokens that depend on mutable records capture record versions and reject stale commits.
- Player knowledge is represented separately from world truth. Reports reference information records rather than reading hidden state as if it were known.
- There is no universal heat value. Legal pressure is represented by actual investigations and evidence.
- There is no tactical movement or combat-control layer in the core architecture.
- Core predicate naming follows `is_`, `has_`, and `can_`; synchronized-index validation uses `has_consistent_indexes` project-wide.

## Intentionally Not Yet Modeled

The foundation deliberately does not fabricate shallow implementations for systems whose data model needs further design. Major domains still to be introduced through their own records, registries, systems, indexes, invariants, persistence coverage, and behavioral tests include:

- City geography beyond neighborhood identity, transport topology, schedules, and patrol deployment.
- Business economics, employment, supply, demand, ownership transfer, legitimate assets, and illicit enterprises.
- Delegated routine work selection, staffing behavior, responsibility handoff, and manager behavior that acts from mandates without direct player input.
- Character needs, family ties, secrets, injuries, legal status, recruitment networks, and behavior decision systems.
- Operation resolution plans/outcomes, causal after-action generation, equipment, and local circumstances beyond the durable authority-exception workflow.
- Arrest, charging, bail, prosecution, courts, lawyers, informants, corruption, politics, press, labor institutions, and rival strategy.
- Generated opportunities, executive-brief synthesis, campaign pressures, historical transitions, and end-state evaluation.

These omissions are intentional. New systems should be added only when their canonical ownership and transaction boundaries are clear enough to preserve the design law.
