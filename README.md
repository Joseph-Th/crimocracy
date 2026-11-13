# Crimocracy

Deterministic Rust simulation foundation for a systemic crime-organization strategy game. The player acts through people, information, plans, policies, relationships, delegated authority, enterprises, and institutions.

## Start here

Use each document for one job. [`AGENTS.md`](AGENTS.md) owns agent execution rules and routing; [`STATUS.md`](STATUS.md) owns implemented scope and exclusions; [`ARCHITECTURE.md`](ARCHITECTURE.md) owns state ownership, mutation, determinism, persistence, invariants, and tick order; [`TESTING.md`](TESTING.md) owns verification and text play evidence; [`GAME_DESIGN.md`](GAME_DESIGN.md) owns product intent; [`CITY_VIEW.md`](CITY_VIEW.md) describes the read-only procedural city projection for future GUI adapters.

`content::build_registry()` builds immutable authored definitions. `AppState` owns serializable campaign state and deterministic runtime state. [`core::simulation::run_tick`](src/core/simulation.rs) advances exactly one simulated minute through the contractual phase order in [`ARCHITECTURE.md`](ARCHITECTURE.md), returning a typed error without mutation when the finite simulation clock has no successor minute.

On each day boundary, non-player criminal owners distribute legitimate business earnings above each storefront's protected working-capital floor into accounted funds before payroll. The finance owner commits a single balanced, preflighted ledger batch, opening a destination account only when necessary. Player-owned storefronts require an explicit profit-sweep command; neither path treats laundering fees as earned business profit.

Consequential mutation goes through the owning system's `validate_* → commit` or `decide_* → apply_*` path (some owners validate the decided plan before applying it). Tests, examples, adapters, and tools use those same production paths. Do not construct authoritative `*Record` values or patch owner-private state as a shortcut.

## Reading order

| Need | Read |
|---|---|
| Agent routing and execution rules | [`AGENTS.md`](AGENTS.md) |
| What exists and what is excluded | [`STATUS.md`](STATUS.md) |
| State ownership, mutation, determinism, persistence | [`ARCHITECTURE.md`](ARCHITECTURE.md) |
| How behavior is proved | [`TESTING.md`](TESTING.md) |
| Player experience and product intent | [`GAME_DESIGN.md`](GAME_DESIGN.md) |

The owning `src/` module and its focused tests are the authority for executable
behavior. Design intent does not prove implementation and status does not define
intent. If authorities conflict, repair the owning document and implementation.

## Play the simulation in text

`cargo text-play` starts an interactive, line-oriented leadership session over the
production simulation. Enter `help` for the full command reference.

```powershell
cargo text-play
cargo text-play --seed 12
cargo text-play --seed 12 --opening thin-crew
cargo text-play --load campaign_01
cargo text-play --seed 12 --script target/text-play/commands.txt
```

The initial authored city has public storefronts, several owned businesses,
an established racket, connected recruitment prospects, and two burglary leads.
For example, the following commands work together in a new `--seed 12` session:

```text
brief
prospects
recruit 7 appeal
scout 5 char-5
advance 3h
discover 5
burgle 3 char-4 char-3 press
operations
accounts
save campaign_01
```

Organizational expansion and economic decisions use the same player interface:
`directory` lists public establishments, `buy` spends accounted funds on
independent businesses, `launder` converts street cash within an owned front's
real capacity, `businesses` shows earned owner-withdrawal headroom, and
`racket-kinds` lists enterprise requirements. `open-account`, `move`,
and `start-racket` let leadership fund and establish more enterprises with
actual mandates and existing books. Ordinary `move` transfers only between
accounts of the same spendable money kind, never dirty cash directly to clean
funds. `operation`, `dispose` and `deposit` handle held proceeds through
production reports and ledger transactions.

Leadership decisions include `policies`, `policy`, `mandates`, `delegate`, and
`mandate` for organization-level recruitment and legal-support rules, managerial
territory/functions, standing orders, budgets, and revocation. `districts` and
`directory` expose public district IDs needed for territorial delegation.
`operation-kinds` lists the complete authored operation vocabulary with required
crew skills and supported approaches. `plan <kind> <business-id>` accepts
player-driven plans using those definitions; `plan` also accepts `char-N` for
witness pressure and extraction when the organization has learned the relevant
legal-person status. `known-people` lists people the organization has identified,
not every person in the simulation. Automatic staffing requires rated skill for
each required role, while an explicit `Role=char-N` assignment permits the player
to take the risk of an untrained participant. Burglary continues to use `burgle`
so existing known opportunities are converted through their actual lifecycle.

New games currently use the canonical authored city scenario, not a generated
open-world campaign. The `--opening` option chooses among `night-trap`,
`late-patrol`, `fleeting-window`, `veteran-crew`, `thin-crew`, and `green-scout`;
these are authored alternative opening conditions, not new campaign generators.
Each unseeded start chooses a fresh replayable world seed;
the opening banner prints it. Saves use production `build_save` / `restore_save`
inside `target/text-play/<slot>.sav`. Scripts are ordinary command-per-line
transcripts and reject invalid actions rather than silently correcting them.
Loading reads the saved campaign directly, without rebuilding the authored
starting fixture. The text client shows the player's records, available public
directory fields, and organization-held intelligence, not hidden law-enforcement
or rival state. [`TESTING.md`](TESTING.md) distinguishes play from regression evidence.

## Local commands

```powershell
cargo text-play
cargo check-fast
cargo test-focused <filter>
.\scripts\check-docs.cmd
.\scripts\verify.cmd -Check
.\scripts\verify.cmd -Fast -Filter <filter>
.\scripts\verify.cmd -Fast
.\scripts\verify.cmd -Harness
.\scripts\verify.cmd
cargo test-harness
```

Optional Rust diagnostics are deliberately outside the completion lanes:

```powershell
.\scripts\rust-diagnostics.cmd Modules -Focus crimocracy::enterprises
.\scripts\rust-diagnostics.cmd Orphans
.\scripts\rust-diagnostics.cmd MutantsList -File src/enterprises/enterprise_execution/support.rs -Filter validate_enterprise_business_dependencies
.\scripts\rust-diagnostics.cmd Mutants -File src/core/simulation.rs -Filter draw_index -TestFilter draw_index -Name draw-index
.\scripts\rust-diagnostics.cmd Expand -Item core::id -Filter OrganizationId -Context 3 -MaxLines 120
```

[`TESTING.md`](TESTING.md) owns which lane completes each class of change and what a text play session establishes. [`scripts/verify.ps1`](scripts/verify.ps1) owns the verification gate; [`scripts/verify.cmd`](scripts/verify.cmd) is its wrapper. Verification is local and does not depend on hosted CI.

Build-profile tuning and measured performance notes live with Cargo configuration rather than in the current contract documents.
