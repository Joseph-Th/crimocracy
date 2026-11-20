# Crimocracy

Deterministic Rust simulation foundation for a systemic crime-organization strategy game. The player acts through people, information, plans, policies, relationships, delegated authority, enterprises, and institutions.

## Start here

Use each document for one job. [`AGENTS.md`](AGENTS.md) owns agent execution rules and routing; [`STATUS.md`](STATUS.md) owns implemented scope and exclusions; [`ARCHITECTURE.md`](ARCHITECTURE.md) owns state ownership, mutation, determinism, persistence, invariants, and tick order; [`TESTING.md`](TESTING.md) owns verification and text play evidence; [`GAME_DESIGN.md`](GAME_DESIGN.md) owns product intent; [`CITY_VIEW.md`](CITY_VIEW.md) describes the read-only procedural city projection for future GUI adapters.

`content::build_registry()` builds immutable authored definitions. `AppState` owns serializable campaign state and deterministic runtime state. [`core::simulation::run_tick`](src/core/simulation.rs) advances exactly one simulated minute through the contractual phase order in [`ARCHITECTURE.md`](ARCHITECTURE.md), returning a typed error without mutation when the finite simulation clock has no successor minute.

On each day boundary, non-player criminal owners and players who enable daily treasury distributions sweep legitimate business earnings above each storefront's protected working-capital floor into accounted funds before payroll. The finance owner commits a single balanced, preflighted ledger batch, opening a destination account only when necessary. Players using manual treasury distributions can withdraw earned surplus explicitly; neither path treats laundering fees as earned business profit.

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
cargo text-play --generate --seed 12 --city-size small
cargo text-play --generate --seed 1931 --city-size medium
cargo text-play --generate --city-size large
```

New procedurally generated campaigns use a fixed world seed and a selectable
city size:

| Size | Districts | Businesses | Criminal rivals | Police precincts |
| --- | ---: | ---: | ---: | ---: |
| Small | 4 | 48 | 2 | 1 |
| Medium (default) | 8 | 144 | 3 | 2 |
| Large | 16 | 384 | 5 | 4 |

Each generated city has distinct district names and correlated economic/policing
profiles, named civilian and institutional characters, staffed police and rival
organizations, law enforcement jurisdictions and patrols, operating storefronts
across all 25 business types, real business and criminal ledger accounts,
delegation mandates, active rival and player rackets, starting crew relationships,
approachable recruitment prospects, and two actionable burglary leads. Generation
uses deterministic local randomness for construction; gameplay then follows the
same authoritative minute ticks, validated operations, finances and persistence
as authored campaigns. No rival ownership or investigation details are revealed
just because a generated storefront appears in the public directory.

The seeded city can also be projected into the existing interactive WebGL2 map:

```powershell
cargo run --locked --example city_gui_preview -- --generate --seed 12 --city-size medium
python scripts/generated_city_browser_smoke.py
```

Open `target/agent-output/city_preview/index.html` to inspect generated
district streets, lots and registered business locations. The map remains a
read-only visualization. Road geometry and travel routes are not yet a
simulation-backed transport or movement network. `cargo text-play` uses an
optimized, overflow-checked play profile. Use `cargo text-play-debug` when
investigating full per-minute debug invariant checks; both use the same
canonical tick and validated save format.

Without `--generate`, the historical authored opening remains the default.
The authored city has public storefronts, several owned businesses,
an established racket, connected recruitment prospects, and two burglary leads.
For example, the following commands work together in a new `--seed 12` session:

```text
brief
prospects
recruit 7 appeal
scout 5 char-5
advance 3h
discover 5
burgle 3 char-4 char-3 ask
operations
accounts
save campaign_01
```

Burglary and general operation orders share police contingencies: `press` continues under exposure without requesting a decision, `ask` requests leadership intervention, and `abort` automatically withdraws before entry or requests a decision after entry. Burglary defaults to `abort`; general operation plans default to `press`.

Organizational expansion and economic decisions use the same player interface:
`directory` lists public establishments; `market` lists currently purchasable
independent establishments with their prices and advertised business functions,
without exposing rival ownership or private books. `buy` spends accounted funds on
independent businesses. When the listings are exhausted, `offer <business-id> <cents>`
submits a binding negotiated purchase offer to another organization's owner. An
owner may refuse; sufficient consideration must compensate its asset value and
foregone operating income, and it will not sell infrastructure used by active or
suspended rackets. A staffed rival also protects its productive payroll base if
there is no affordable replacement, and keeps a newly acquired profitable active
storefront for at least ninety days. Refusal alone does not reveal the reason.
Rivals invest across the city and submit voluntary offers themselves, but cannot
purchase your properties without your consent. Accepted offers pay the actual seller, transfer the existing
books, and restart the acquired operating cycle. `launder` converts street cash within an owned front's
real capacity, `businesses` shows owned infrastructure and earned owner-withdrawal headroom, `treasury daily` authorizes daily profit distributions before payroll (protected working capital stays in the storefront), `treasury manual` restores explicit withdrawals, and
`racket-kinds` lists enterprise requirements. `open-account`, `move`,
and `start-racket` let leadership fund and establish more enterprises with
actual mandates and existing books. Ordinary `move` transfers only between
accounts of the same spendable money kind, never dirty cash directly to clean
funds. `operation`, `dispose` and `deposit` handle held proceeds through
production reports and ledger transactions.

`start-racket` accepts either an owned `business-N` venue or a public
`district-N` for rackets that do not require a physical host. Supply any
number of distinct owned support-business IDs after the two account IDs;
distributed numbers and slot-machine rackets need multiple actual business
functions across that network. `racket-kinds` lists the required
functions. Add `manager=char-N` to choose an eligible manager explicitly; a
cross-district network requires an organization-wide enterprise scope.
`preview-racket` takes the same arguments, validates without establishing the
racket, and shows the chosen manager, conditional costs and earnings, and the
case-pressure threshold for nonpositive expected earnings. Both `preview-racket` and `enterprises` warn when assigning several active rackets to one manager would expose them to a simultaneous suspension on detention. `business business-N suspend` pauses a player-owned storefront
only after its active racket dependencies are suspended;
`business business-N resume` restarts its real operating cycle without resetting the
current laundering window. Rackets must be resumed separately. `reassign-racket <id> <member-id>` transfers an existing racket to an eligible manager whose mandate covers its whole network, retaining the accounts, historical settlements, and current operating status. Use `racket <id> resume` afterward if it was suspended. An active racket with a due settlement must settle or suspend before reassignment.

For financial review, `ledger [count]` lists recent transactions touching
organization-owned accounts, and `transaction transaction-N` shows only
the player's own postings, excluding the counterparties' private books.
`business-cycles business-N [count]` summarizes actual storefront
settlements earned under player ownership;
`racket-cycles enterprise-N [count]` reports gross, operating costs, net cash, enforcement costs, and the manager responsible for each historical settlement.
All histories are read-only and survive save/load.

Leadership decisions include `policies`, `policy`, `mandates`, `delegate`, and
`mandate` for organization-level recruitment and legal-support rules, managerial
territory/functions, standing orders, budgets, and revocation. `districts` and
`directory` expose public district IDs needed for territorial delegation.
`operation-kinds` lists the authored operation vocabulary, required crew skills,
supported approaches, and relevant intelligence topics. `plan <kind> <target-id>`
authorizes operations against eligible business targets. Surveillance and
infiltration additionally accept public `district-N` and `business-N` targets,
or identified `char-N`, `org-N`, `operation-N`, `investigation-N`,
and `enterprise-N` records. Successful infiltration of a business can reveal its owner and latest operating results; infiltration of a criminal or commercial organization reveals actual leadership and mandates, and enterprise infiltration can reveal its supporting businesses and latest settlement. Ordinary surveillance does not expose these internal records. `known-targets` lists identified institutional and
administrative targets; it never searches hidden case or rival rosters.
`known-people` includes current crew, observed people, contacts, and recruitment
prospects. The `prospects` view identifies familiar vocational skills without
revealing private motives or acceptance odds. Managers adapt their approaches after
refusals rather than repeating an unchanged pitch every cooldown. A recruiting
player organization's outcomes generate sourced reports; approving a pitch displays
the actual acceptance or refusal, not merely the approval resolution. `plan` also accepts `char-N` for witness pressure and extraction
when the organization has learned typed, episode-specific legal status. Own-member
release is visible in `known-people` and prevents authorizing another rescue against
that concluded custody episode.

All `plan` orders accept optional `approach=...`, `leader=char-N`,
`Role=char-N`, `police=press|ask|abort`, `delay=2h`, `deadline=6h`,
and `need=PoliceActivity` options. Delays and deadlines are measured from the
current simulated minute and capped at seven days. `police=press` is the default:
the crew continues under any response penalties. `police=ask` requests a
leadership decision; `police=abort` adds a pre-entry escape when the operation
has an actual authored entry stage, then asks leadership on later arrivals.
`need` requires usable organization-held intelligence of an applicable topic
before authorization. `preview` accepts the same options as `plan` and validates
the same order without committing it; it lists the crew, planned start, and
best usable observation per applicable topic at that start, including missing topics.
The production owner validates every constraint and crew
booking. Automatic staffing excludes detained and overlapping participants and solves the whole crew assignment to maximize
trained capability, with stable ID-based tie-breaking and no duplicate
participants; explicit `Role=char-N` permits knowingly untrained people.
`cancel operation-N` withdraws an authorized or running operation through
the canonical abort path, files its report, and releases personnel; pending
leadership decisions must instead be resolved with `decide`. Burglary uses
`burgle` to consume actual leads through their conversion lifecycle.
`discover business-N` selects the newest usable, operation-relevant
organization-held observation for that business, then validates the opening.

Institutional relationships and legal defense are player-managed.
`connections` lists known outside acquaintances and eligible handlers;
`connect <handler-id> <known-person-id>` establishes a real institutional
channel only when the modeled social relationship supports it. `contacts`
lists established channels and outstanding disclosures;
`contact <contact-id> [ask|ask-all|end]` requests one or up to 64 available
disclosures from the explicitly selected channel, or terminates it. Disclosures
carry their original observation minutes. Use `contacts` to identify the channel.
The authored city includes a known lawyer at Cedar Street Legal, but players
must form that connection.

Personal field observations do not automatically become organization knowledge.
`debriefs` lists the count of unreported notes held by current, non-detained
members. `debrief char-N` transfers the member's next new observation through
the production intelligence owner, retaining its original observation time,
typed facts, source, and lineage; repeat to receive additional notes.
`intelligence [count] [page] [target-id]` browses held observations by descending
observation time, rather than disclosure time. The default page is 20 observations;
page size is 1–100 and an optional public or identified target filters the list.
`intel information-N` inspects a specific organization-held observation,
including its typed signal and provenance, but cannot read an outside contact's
or member's still-private source just by guessing an ID. `case-brief` collects
up to 20 recent organization-held case-activity observations with original dates,
reliability and provenance IDs; it is not a live census of investigations.

A working police contact may disclose an investigator's actual case-witness
registration following a criminal incident. After that disclosure,
`known-people` identifies the witness, and `plan witnesspressure char-N` can
use the learned episode if the production legal and operation checks permit.
An institution-level surveillance report about case activity is not itself
evidence that a specific person is a witness.

`counsel` displays known custody matters involving current members and their
actual representation. `retain <arrest-id> <legal-contact-id> <fee-cents>`
hires counsel through a live legal contact and the production ledger.
`end-counsel <representation-id> <concluded|replace|withdraw>` closes
representation only when its legal constraints allow it. Alternatively,
`policy legal automatic` enables automatic retention when a legal contact
and funding exist. `advance` interrupts on arrests and releases of members
so manual counsel decisions are not skipped. It also pauses on newly suspended
own businesses or rackets, the first losing cycle in an own racket's current
losing streak (including after reopening), and payroll shortfalls by default.
Use `attention exception off` for uninterrupted routine management; actual
pending decisions and own custody changes still interrupt. `brief` displays
current daily payroll coverage alongside withdrawable earned surplus.

Games may use seeded procedural cities through `--generate` or one of the
original authored test openings. The `--opening` option chooses among `night-trap`,
`late-patrol`, `fleeting-window`, `veteran-crew`, `thin-crew`, and `green-scout`;
these are authored alternative opening conditions, not procedural seeds.
Each unseeded start chooses a fresh replayable world seed;
the opening banner prints it. Saves use production `build_save` / `restore_save`
inside `target/text-play/<slot>.sav`. Scripts are ordinary command-per-line
transcripts and reject invalid actions rather than silently correcting them.
Command verbs are ASCII case-insensitive; arguments retain their own syntax.
Loading reads the saved campaign directly, without rebuilding the procedural
city or authored fixture. The text client shows the player's records, available public
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
