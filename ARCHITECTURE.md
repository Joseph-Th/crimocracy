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
- **Operating storefronts** — `BusinessEconomyRecord::is_operating` distinguishes a live scheduled economy from an active-status record with no future cycle. Market competition, commercial income forecasts, territorial defense and coercion, civic work and campaign infrastructure, political standing, sabotage and laundering capacity use this distinction. Ownership, appraisal and historical settlement evidence remain independent of current operating capacity.
- **Adapters** — own filesystem, process, network, and UI effects. Core systems receive explicit data and return explicit outcomes.

Optional campaign conclusions live in `core::campaign_victory`, which derives eligibility from continuous business title history, positive settled business cycles, established institutional contacts, enterprise age and cycle history, available accounted-funds balances, current staff payroll, and the read-only world territory influence resolver. Political and underboss claims require productive presence in three quarters of districts; political storefronts require 1,080 days of continuous title, political sponsorship requires a currently usable contact established at least 180 days ago, and underboss rackets require 90 days of operation. A rolling 30-day window of actual net settlements (including negative cycles) qualifies each mature business or racket with at least 20 positive cycles, seven-day settlement freshness, and district income sufficient to cover one registry-authored crew wage. Political districts require three established profitable storefronts across two different kinds; their mature local net must also meet the one-member wage floor. Political eligibility requires a named command roster of four headquarters members plus two independent local delegates per required district. The political and finance delegates must each have an active Neighborhood mandate for that district plus the respective function scope, Delegated or Broad autonomy, and no active detention; the organizational executive cannot serve as a district delegate. The private `core/campaign_victory/political_staffing.rs` owner jointly assigns the two distinct roles to complete district teams, maximizing electoral-ready districts first and total fully staffed districts second in planning views. At ballot certification, actual vote-winning electoral-ready districts receive the highest priority; a losing ward cannot consume a shared director needed to certify a feasible winning coalition. The shared private `core/campaign_victory/district_matching.rs` solver uses relaxed augmenting-path matching to bound each priority prefix in the exact search; broad mandates and dual-role candidates cannot be reused across posts. The private `core/campaign_victory/commercial_standing.rs` owner derives commercial income and maturity from title periods and settled books, and combines the resulting district standings with the political staffing allocation for both victory validation and read-only district views. Live director candidates are reported for every public ward, regardless of economic maturity or title ownership; only economically qualified wards consume counted two-person teams. The aggregate and district assessment is computed once for elections and combined progress views; `text_play/victory_play.rs` formats the results without embedding this read model in command dispatch. Permanent specialist offices commit their supervisor to a single district: the politics owner checks this at hire and commit, pauses unavailable or conflicting contracts without wages or output, and exposes contracted and currently payable daily salaries. Mature legitimate or criminal income from the relevant victory route must separately cover the entire 30-day named payroll obligation, without substituting laundering or held cash. All other recent net losses are deducted by enterprise or settled business title period, including suspended/retired enterprises and sold businesses; positive net from ineligible ventures does not substitute for mature income, including long-held active titles that lack sufficient recent profitable settlements or a fresh settlement. Underboss districts require a strict **mature** racket majority over all rival active rackets combined, at least 30 historically proven workers across two mature racket kinds and more proven workers than local rivals combined, and three named defenders from the actual territory-conflict force allocation. Mature citywide rackets and proven workers must exceed all active rivals combined. The same districts need current machine influence (200) and institutional support (100), read from the political owner, without requiring an electoral majority. Underboss eligibility additionally requires a scaled available named command roster (at least 24, otherwise six headquarters personnel plus six per district required for three-quarters territorial control), an available executive, and two separately managed mature active rackets per qualifying district. The private `core/campaign_victory/command_staffing.rs` owner supplies the current independent captain candidates to the shared exact two-post district matcher. Only undetained non-executive captains with Delegated or Broad autonomy qualify; reducing discretion removes command credit without erasing earned associate or income history. A manager cannot simultaneously command two districts. The senior-command witness must fit two distinct direct-report captains into complete, civically backed districts in a feasible coalition of the required size; if the unconstrained maximum assignment does not prove this, the same matcher tests alternative assignments with two required report-bearing districts prioritized. The current captain must have personally commanded 20 profitable cycles within the most recent 30 days, checked against the racket's persisted historical authority timeline; a same-day reassignment does not inherit the predecessor's standing. Proven associate strength is the largest staffing complement supported by at least 20 such settlements, capped by current staffing. New hires do not erase established strength, but the increased complement must earn its own operating proof; layoffs cannot count absent workers. Recent net income, operating maturity, and proven staffing are derived in one settlement-history traversal. The entire expanded named roster still counts against payroll; unnamed operational workers are aggregate racket staffing included in operating expenses, not individually tracked tenure. The private `core/campaign_victory/underboss.rs` owner derives eligible senior supervisors from actual direct reporting lines and the enterprise workload resolver, then requires the joint command-staffing witness, institutional reach from active usable 90-day contacts across three civic sectors and two handlers, and the additional 90-day operating buffer from current canonical enterprise financial projections at maximum modeled heat. The latter is added to 180 days of named payroll and 90 days of all retained campaign specialist contracts (including paused contracts) and evaluated against net available liquid organization balances, not storefront tills or clearing. The projections use an explicit conservative pressure input, not hidden live cases. These read-only aggregate views use existing persisted ownership, enterprise, relationship, contact, politics and settlement records, plus reconstructed indexes, without modifying save format. Claims are explicit player operations, not autonomous ticks. The private `core/campaign_victory/retirement.rs` owner uses the finance owner's balanced ledger and planned personal-account openings. The held settlement binds each payee and amount to the current named roster and sole executive; membership or leadership changes stale the payment even when eligibility and aggregate funding remain sufficient. The terminal history validator reconciles the exact organization debits and every member's personal accounted-funds credit against that roster, the fixed nest egg and registry-authored severance. Every outcome is recorded through the history and report owners. The private `core/campaign_victory/certification_report.rs` owner derives historical election identity and winning ward votes from the persisted ballot, not current district readiness; current financial, coalition and sponsor details are explicitly certification-time observations. Held claims regenerate their final report after eligibility and settlement freshness checks and before any state mutation. A `CampaignVictory` event and report have closed, release-safe provenance checks, and the history owner's `serde(skip)` victory index rebuilds on load. `run_tick` checks for a recorded conclusion before modifying the clock. The derived index is reconstructed only from current-schema history events on restore; no old-save migration is supported. A text-play read-only victory progress view never discloses private rival counts or locations.

**Mayoral elections are a separate persistent political domain, not a threshold award.** `politics::PoliticsState` owns each criminal organization's three distinct district constituencies (machine, grassroots voters, institutional support), specialist contracts to real named political delegates, local initiative cooldowns, cumulative actual campaign expenditures, registration for the next eligible ballot and append-only certified electoral tallies. `politics::political_system` validates campaign authority and delegates canonical payments to its private `expenditure.rs` owner. Political expense postings drain one or several actual accounted-funds accounts atomically into a dedicated `Settlement` account owned by a real municipal Political organization, not the criminal organization's treasury. The finance owner protects each attached clearing pocket from non-campaign postings, including held ledger tokens, without restricting unrelated municipal settlement accounts. Release-safe validation requires a distinct clearing account per campaign and reconciles its positive credits, exclusively funded by that campaign's own accounted-funds debits, with both its ledger balance and lifetime spending counter. Rejections preserve state and account versions. Nomination and victory progress share the world owner's sole available executive resolver; a detained second root still represents divided command.

The day-boundary political phase follows commercial settlements and criminal payroll. It decays machine and civic support separately, pays specialists only when funded and backed by undetained, active Political/Neighborhood delegates resolved through their indexed mandate. Multiple specialist contracts can share a manager within one ward; hiring, held-order commit and current-save validation forbid assigning that manager across wards of the same employer. Output and salary need only check current managerial eligibility for these structurally valid contracts. Specialist effects use genuine capability multipliers using SocialAccess, Negotiation or Management skills. Local events require actual active owned civic facilities or a 30-day labor contact: public meetings (customer access/meeting space), neighborhood services (soup kitchen, laundry, pharmacy), labor coalitions (union function or labor contact), and local press (printing function/news service). Each event costs clean funds and has a seven-day cooldown per district and type. Both autonomous commercial acquisitions and rival political investment require a sole undetained executive, resolved by `world::world_system::sole_available_executive`; leaderless or ambiguous reporting roots cannot commission new treasury spending. Rivals spend accounted funds on deterministic weekly machine campaigns only where they have active commercial holdings, prioritizing the strongest live player challenge not already defended by rival machine influence. Weekly spending targets a real player political challenge or, at high public prominence, the player's three-or-more operating businesses in the same ward. Rivals aim for a bounded defensive influence threshold, stop paying once adequately defended, and do not burn cash in unrelated wards. Weekly investment capacity rises from 100 to 220 influence points as public prominence grows, but is capped again by actual spendable accounted funds after reserving seven days of named-member payroll across all accessible criminal treasury accounts. An underfunded rival buys whatever whole influence points it can still afford rather than doing nothing. Equal challenges use stable district-ID tie-breaking; no free influence or fictional funds enter. No criminal organization receives automatic grassroots or institutional legitimacy. A district's 90-day sustained-majority clock resets whenever it loses the simultaneous 800 machine, 300 grassroots, 300 institutional or strict 3:2 rival advantage requirement. The owner consumes no RNG and does not run per-minute electoral scans.

**Escalating institutional and rival resistance is derived, not a hidden difficulty timer.** `world::organization_pressure::public_prominence_tier` reads registered business ownership, the breadth of commercial ownership, and publicly visible machine influence in every campaign ward, including wards without owned property. It returns a bounded 0–6 tier without persisting a duplicate notoriety meter or revealing foreign criminal books. Vice inquiry exposure in `enterprises::enterprise_execution::cycle_planning` starts at one authored source, adds up to two observed district cases, one source for each three prominence tiers, and one source for each four additional active rackets (up to two portfolio sources). This only changes the chance of the normal police case-intake roll on a real racket settlement: an enforceable jurisdiction, investigation evidence, and ordinary custody/prosecution rules remain required. The read-only racket establishment and existing-enterprise outlooks share the same exposure formula and clearly distinguish no-case risk from last observed district case pressure. Rival `operations::operation_system::autonomous` planners still need their own local active commercial stake, competent permitted crew, either real forecast market gain or a visible substantial player political machine in the same district, and usable intelligence, but favor prominent publicly registered owners and may continue up to four successful strikes in seven days against a tier-three-or-higher owner instead of two. Political motive allows rival pressure against a legitimately diversified mayoral coalition even without direct sector competition, but only while the rival has an active commercial stake in that ward; inactive targets and independent owners receive no such political targeting. Independent predatory motives still use the territorial-force rules. The player brief discloses their derived prominence tier and its basic consequences. These inputs are reconstructed from authoritative state across save/restore and consume no additional RNG draws.

**Protection coercion is an actual player decision and ledger consequence.** `decisions::decision_system::protection` compares the threatening gang's actual local racket associates against `world::territory_conflict::local_defensive_forces`: operating racket associates plus a finite named enforcer roster distributed deterministically across the defending organization's active commercial and racket districts. That roster cannot protect every district at full strength, and an enforcer currently executing or paused inside an operation does not defend commercial holdings until that operation terminates. Future-authorized operations alone do not remove present defense. A shared read-only district defense projection exposes active racket and operating shop counts, actual associates, and allocated named enforcers separately. Protection, rival operation planning and expansion, and the player turf view consume this allocation once per defender rather than reconstructing it per ward. Underboss qualification uses the same allocation algorithm with only permanent members; fixed-term guards can contribute to immediate defense but not durable command qualification. `enterprises::autonomous_expansion` gives exposed criminal-owned commercial holdings priority for rival racket expansion whether the owner previously operated a racket or has always relied exclusively on legal shops, but each expansion still requires a viable delegated manager, working capital, legal location and positive economics. Once an actual gang has staffed a local racket, its named manager may demand a 30-day payment from a weaker commercial owner. The durable `DecisionContext::ProtectionDemand` identifies the gang, ward and representative active storefront; one offer per district may be pending and only one is proposed citywide each day. The amount is 5% of publicly observable monthly gross, clamped to $50–$15,000. Approval transfers real accounted funds to the rival's street cash through a balanced validated ledger transaction before resolving the decision; refusal needs no money. Approval rechecks the live local force, managerial availability and standstill rules used to generate offers, plus ownership and remaining operating commerce. A vanished threat rejects approval before any ledger mutation, but refusal remains available. The decision owner indexes the latest demand by ward and by ward/gang from immutable request time and ID, rebuilding both projections from saved records. The latest demand per district controls offer timing, while the latest demand for a specific gang controls its own paid ceasefire and refused-demand retaliation against the player-owned businesses in that ward, never against uninvolved owners. A pending negotiation blocks rival attacks for one simulated day only; 30 days of paid coverage block both new rival surveillance/sabotage authorization and already-authorized operation start/resolution, using a typed persistent `OperationObjectiveBlocker::ProtectionAgreement` so a scheduled raid cannot bypass payment. Rival tactics resume if the player refuses or coverage expires. The player's `turf` view reveals only own local workforce and storefronts, not foreign staff or private rival holdings. Release-safe decision invariants verify every settled protection payment against account-indexed, minute-exact ledger postings, and saves retain the contract chronology. Police institutions, rivals not party to the agreement and legitimate-business operating systems retain their normal autonomy.

Every 180-day municipal boundary is a scheduled public election, provided the ballot fits within the finite simulation clock. The stored election day is an elapsed-day ordinal. The player calendar starts on day 1 at minute zero; `SimTime` owns the display conversion for ballot dates, notices and actionable deadlines (the first ballot occurs on day 181 at 00:00). The next calendar election and earliest ballot open to a new filing are distinct read-only projections; `next_mayoral_nomination_day` owns the inclusive 30-day filing cutoff and finite clock check, shared by validation, held-order commit, restoration and both campaign views. A paid filing must target that exact first eligible ballot. Nomination rejects an exhausted filing schedule before any money moves. A nominated player leader must have filed at least 30 days prior with an established political council sponsor, an eligible executive and a paid 2,000,000-cent filing fee. Political sponsorship, labor initiatives relying on contacts and underboss institutional reach use the same live contact availability and relationship predicate; dormant contacts remain in history without granting current access. Weekly rival campaign purchases require an available organizational executive. Daily campaign maintenance and scheduled elections are implemented in separate private modules of the political system. At the ballot, `core::campaign_victory::mayoral_ballot_shortfalls` determines and explains the mature business, independent delegate, profitable-payroll and multi-district qualification rules, alongside a 35,000,000-cent campaign expenditure and 15,000,000-cent remaining clean reserve. The election owner tallies the filed candidate, distinct incumbent political opposition and rival influence for each district; a nominee who subsequently joins a council cannot also be their own incumbent opponent, and losing current executive authority prevents qualification. Local machine, grassroots and institutional support, current voter fear from the existing Residents reputation audience, active local rackets, and the city's police/commerce profile affect the public vote count. Ward ballots apportion integer votes proportionally by largest remainders, with stronger support followed by fixed ballot order breaking rounding ties; a zero-support contender never receives fabricated leftover votes. Wards count only if locally qualified, their 90-day majority has survived, and they win the actual ballot against both competitors. A victory requires three quarters of districts. Results are persisted and surfaced through an ordinary Standing report and the `election` view. Defeat notices distinguish missing citywide qualifications (with actual shortfalls at the ballot) from insufficient qualifying ward victories; the election view distinguishes popular pluralities that fail local qualification from lost votes. A losing nominee may file for another election; a winning candidate alone may invoke `claim-victory political`. Campaign history validation links every political victory event to a real preceding winning ballot. The save schema advances; older layouts fail closed under the current-version-only policy. The read-only text client shows only its own political standing and coarse live rival pressure, revealing public ward votes after the official ballot.

**Named civilians and institutional employees share the canonical person model.** `world::CharacterRecord` holds persistent personal career standing, accumulated workdays, fatigue and the last chosen actual duty. `world::character_life` derives occupation from a person's current property title or organization membership; it does not persist a duplicative role label. Daily work is chosen deterministically from personally owned storefronts that are active or have settled within the preceding simulation day (prioritizing troubled assets using the latest fresh settlement under the current title and after any operating-period restart; a completed loss-making day still counts if settlement suspended the storefront before character work runs, but later closed days do not replay it; an operating shop without a fresh settlement still receives a management workday but no profit or loss adjustment to the proprietor's standing or fatigue), assigned civilian workplaces (using the same open-or-settled workday rule, with ordinary employee standing and fatigue rather than owner profit consequences), actual staffed investigations, a precinct's active patrol deployments, public political, commercial, labor and legal offices, or time off. Weekly rest is staggered by stable character identity rather than closing every police and municipal shift on one shared day; detention interrupts work. The simulation closes this personal workday after legitimate settlements and payroll, before wage-related departures and midnight freelance releases, so membership ending at the boundary cannot earn independent work for the completed paid day. Political campaigning and elections see the updated personal condition and current post-departure roster. Career movement does **not** increment the world character authority version; unrelated held mandate and case tokens remain valid. The release-safe validator checks bounded fatigue and standing, chronology, the agreement between accumulated workdays and the last duty, workday capacity under the same staggered rest calendar, and historical workplace and institutional-kind references; save/restore retains full personal state. The legal investigation owner's staffed-detective ranking reads professional standing and fatigue without bypassing canonical case validation. The mayoral-election owner reads a real undetained council member's and nominee's accumulated skills/standing to calculate incumbent and challenger votes; results persist the incumbent's character identity.

**Actual individual shopkeepers have personal economic agency.** The generated city assigns a stable named person to a substantial share of private storefronts, with a real character-owned business title, personal legitimate bank account, and relations with merchant institutions. A separate generation stream avoids perturbing historic business geography and preserves an actual pool of vacant public listings, so political and economic expansion remain possible. `economy::business_economy_system::personal_owner_dividends` automatically distributes only earned surplus above each business's operating-capital floor, via a balanced preflighted multi-owner ledger posting into the owner's personal accounted funds, after the existing organization sweep and before payroll. Private property offers use the existing `business_acquisition` composition, live seller choices and title-version validation. `business_acquisition::seller_strategy` owns both organizational and personal seller choices. A real proprietor's personal reservation value considers expected business earnings, compensation for the last profitable operating storefront based on its live forecast income, traits, work fatigue and professional standing; closed and loss-making alternatives cannot replace that income; acceptance credits that person's own accounted account, atomically with the title change and economy restart. This does not expose owner-private cash or internal cases to the buyer. The derived proprietor profession takes precedence over a fallback vocation while a person owns any storefront; if all their shops are suspended, no unrelated civilian shift is credited. `civics` and `civic-person` read public records, displaying redacted civic activity and real registered titles, not hidden rival criminal rosters. Public titles do not establish criminal affiliation; an unaffiliated proprietor and a proprietor with undisclosed criminal membership receive the same neutral affiliation label. The model simulates individual named civic participants, not every anonymous resident or every employee in a storefront. Operation venue resolution uses recorded home districts only for characters without a personal business or organizational geographic footprint; custody and pinned witness-case venues retain their existing precedence. Per-query organizational footprints are reused, and enterprise-backed venues use `EnterpriseLocation::neighborhood`.

**District-scale named population and freelance employment.** `city_generation::population::populate_city` appends 42 named individuals per district and extra rival gang staff after creating the original opening. A dedicated seeded generation stream and expanded collision-safe name pool preserve existing world geometry, business identity, original crew, accounts and opportunity identifiers while producing unique `CharacterId` people at the largest supported 24-district custom size. `PopulationCity` and `PopulationRole` group typed creation inputs; each character is created through `world_system::insert_character`, with varied drives and traits, then receives an actual validated `IndependentVocation` and home district through `character_life::validate_set_civic_occupation`. Commercial layout, playable opening properties and proprietor creation are isolated in `city_generation::businesses`. Civilian employees visit distinct existing ward workplaces before repeating a workplace at any supported density; local beat police belong to the actual precinct with a named supervisor. The separately generated ward officers and political delegates receive validated local home districts, so available officers perform their ordinary patrol duties locally. Party officials and legal professionals join real municipal organizations, and freelance criminals remain unaffiliated with capability-specific trades and social contacts. No new shadow census or separate non-character entities are created.

`world::freelance_contracts` owns a persistent per-person append-only term history (sponsor, manager, signing instant, fixed expiration, actual fee, payment transaction and actual end). `validate_hire_freelancer` requires a free, non-detained person whose current derived profession is FreelanceCriminal (not a proprietor retaining that fallback vocation), a genuine trusting social edge to a named criminal crew manager, a 1–14 day term and enough actual spendable street cash. The composite validator preflights the world reassignment and the finance-owned ledger posting, opening a personal receiving street-cash account only when needed. Commit first completes the balanced payment, then joins the freelancer to their manager's organization for existing operation participation and payroll. Due expirations use a reconstructed minute-keyed schedule rather than scanning historical contracts: non-midnight releases precede new operations, with a second expiry pass after operation completion, abort or evidence arrest so workers freed during that minute leave immediately. Midnight releases follow the final earned daily payroll and completed workday. Both use the world owner's reassignment guard; a booked or detained person remains scheduled for retry each minute until safely releasable. Active contractors remain supervised and cannot hold standing mandates, institutional channels, or direct reports. Operation authorization rejects bookings extending past the earliest signed expiration and adds that expiration as a canonical completion deadline unless an earlier deadline already applies; crew availability uses the same booking projection. Delayed or paused operations consequently use ordinary deadline cleanup rather than binding a contractor indefinitely. Any other legally permitted membership change concludes the contract at the actual reassignment instant. Archived contracts and provenance remain in save/restore and release-safe validation: signed terms must be whole days within the same hiring bounds, the fee must match the shared formula using immutable capabilities, the contracting manager must be distinct from the freelancer, and each contract must reference its own balanced non-delegated signing payment. Active contracts prevent contradictory civic-vocation changes; after a canonical contract end, a different occupation does not invalidate its historical payment and term. `TickOutcome` carries the IDs of actually expired workers; text advancement checks the archived sponsor before pausing or counting term endings, without exposing rival contracts. Text play reveals a freelance contact only through a real active personal relationship with a player's crew member and the same contact and membership eligibility checks as contract validation, never via a global hidden-character list. Temporary contractors cannot be advertised as hiring supervisors. This is fixed-term employment with existing operational safeguards, not an omniscient instant-summoning action or a duplicate operation-participant system.

**Racket command capacity is operational workload, not an empire-size counter.** `enterprises::enterprise_execution::staffing` derives a captain's live obligations from assigned active rackets: one recurring task per field associate, one per independently operated racket, one per supporting business, and one additional coordination task for each district beyond the first. The captain's authored Management rating (0–100) maps to task capacity at a tunable three rating points per task. A single kind-specific baseline operation grants a corresponding personal-run allowance; hiring optional additional workers never increases that allowance. Command workload also propagates to real supervisors through `CharacterRecord::supervisor()`: each superior supervises one duty per distinct immediate **active operating** subordinate branch (two for Tight/Guided subordinates, one for Delegated/Broad), not every employee farther down the tree. A skilled underboss can absorb several independent crews, while an unskilled executive cannot command unlimited captains directly. Idle new recruits cannot pad supervision. A detained captain has zero live command capacity. The workload quote is recomputed from owned enterprise assignments, current reporting hierarchy, autonomy, membership and custody state, not stored as a redundant counter. Establishment, staffing, succession, recovery and NPC expansion check the full upstream reporting chain. `world::world_system::validate_set_character_supervisor` permits a member holding live mandates, investigations and operations to change their reporting line *inside the same organization*, without releasing the role, while character-version changes invalidate outstanding authorization tokens; cross-organization release guards are unchanged. NPC same-minute autonomous enterprise cohort planning projects the entire selected job set through the same canonical workload resolver, rejecting jointly incompatible candidate branches before the preflighted all-or-none establishment commit. After payroll departures and recruitment, the daily governance pass checks active captains and every ancestor supervisor in deepest-first reporting order, with stable character-ID ties. It reconciles subordinate overload before testing a superior's remaining workload, so an already doomed branch does not cause an unnecessary healthy sibling closure. It suspends excess rackets through validated enterprise lifecycle operations, shedding direct excess work first or the newest racket in a smallest excess reporting branch. It deliberately does not make a dynamic workload ceiling a release-safe persistence invariant: a current-schema campaign can temporarily exceed the dynamic workload ceiling after a staffing or command change, until normal governance applies. Arrest processing already shuts dependent racket work; this pass does not attempt a redundant arrest transition. `TickOutcome` exposes genuine overstretch suspensions for an actionable text-client pause. Normal business dividends remain governed by capital retention and balanced accounting, not an arbitrary limit proportional to named criminal personnel: ordinary commercial employees operate those stores. This keeps criminal-command staffing pressure separate from legal property ownership.

Territorial influence is a read-only economic projection in
`world::territory_influence`: active hosted and district-hosted rackets establish
district presence; ties have no leader. `world::territory_conflict` owns direct
confrontations between gangs already operating in the same district. Their
current rank-and-file staffing sets local strength, with a two-associate defender
advantage. The attacker's strongest local active racket risks suspension if the
challenge fails; a successful challenge suspends the identified rival racket.
The defeated racket cannot reopen for seven days through either manual or NPC
enterprise lifecycle paths. Retirement does not erase this deadline: the
territory owner derives suppression from the losing organization, kind,
location and persisted clash history for replacement establishment, NPC
expansion, and historical validation. Other kinds, locations and organizations
remain eligible under their ordinary constraints. Each attacker has a
seven-day cooldown per district. A non-player gang with enough local strength may autonomously contest
another gang at the day boundary. It creates a crisis report when the player's
operation is displaced, without revealing unrelated private rival information.
One outcome suspends one racket through the canonical enterprise owner.

The private `world::territory_influence::standstill` owner validates bilateral 30-day standstills and owns their history-backed lifecycle, separate from the read-only economic influence resolver. Competing
active rackets or a racket displaced by a recent territorial clash establish
standing to negotiate. A stronger counterparty may require street-cash tribute,
graduated by its local racket advantage; any offered tribute is one
balanced, preflighted inter-organization ledger payment drawn from the paying
gang's combined spendable street-cash pockets, never invented cash.
The history owner commits a typed `TerritoryStandstill` or `TerritoryClash`
event. A standstill stores the actual negotiated tribute and optional ledger receipt,
bound at commit after checking current accounts and allocator capacity. Save/load
validation checks the exact payment minute, parties, street-cash postings and amount,
and prohibits receipt reuse; free agreements must have no receipt. The player's turf
view shows paid or received tribute and its receipt while the agreement is active.
The restriction and expiry are derived from typed durable history with
no redundant persistent territory map; history validates parties, chronology,
typed entity linkage and clash cooldown across save/restore. The enterprise
owner checks a standstill when validating and committing new rackets, but not
when restarting an existing one. The operations owner forbids new offensive
foreign-business orders; investigation and normal commercial acquisition remain
allowed. Previously authorized hostile operations that have not yet started
abort through the canonical objective-unavailable path with a typed territorial
blocker, durable history and report; already-in-progress missions cannot achieve
their prohibited business objective. NPC expansion candidate planning excludes
treaty districts before constructing its projected commit cohort, matching
the enterprise validator and preventing a prohibited planned establishment.
The text client requires discovered rival rackets before contesting or
negotiating and does not expose the rival's secret staff count. Rival autonomous
planners inherit the same canonical restrictions.
The truce prevents direct clashes only between its two named parties; neither
party gains immunity from unrelated gangs, and either may confront a third gang.
The district's freeze on signatories' new rackets and hostile business orders
still applies regardless of the target. An organization can hold only one
active standstill per district until the agreement expires.

Autonomous gang-war planning uses its own active manpower but estimates
opponent force from the visible baseline crew sizes of their local racket
types. Each NPC ranks its current local position at its turn in stable organization
order; a committed suspension immediately updates the daily force projection, so
later NPCs cannot act on manpower already lost or an obsolete rival footprint.
Secretly hired additional defenders can repel an otherwise confident attack.
An NPC that was repulsed avoids another challenge in that district
for 30 days. After real NPC-on-NPC fighting, the defeated gang may negotiate
a standstill at the next daily boundary, paying a required concession from
its actual street-cash account when it can afford it. Player organizations
are never automatically bound by an NPC-signed treaty.
The autonomous peace pass considers the newest clash per rival pair, not only
one clash per district. A renewed clash restarts the full-day waiting period,
even when an earlier fight would otherwise qualify for negotiation. Unrelated
gangs can settle their independent feuds on the same day.

The player-driven text adapter lives in `examples/gameplay_harness/text_play/`
and can start from either the generated authoritative city or a canonical
authored opening without running a scripted evaluator. It owns parsing,
player-knowledge projections,
interactive/scripted session I/O, and save-file publication. Restored sessions
derive their organization and records directly from production persistence
without reconstructing the authored fixture. Saving uses an exclusively created,
per-writer staging file, syncs the complete serialized campaign before publication,
and replaces the slot by rename; a failed publication never deletes the existing
save or another writer's staging data. Its public storefront directory
projects only name, kind, identity, and district through `WorldState` and never
exposes private ownership or law-enforcement state. Commands validate current
player membership, account ownership and available authority before composing
production owner transactions. After victory or dissolution, only inspection, saving
and session exit remain available; even a command that defaults to mutation, such
as contacting a source for a disclosure, is blocked before invoking its owner.
The generic treasury allocation command only
moves same-kind liquid funds: street money cannot become accounted money except
through the canonical laundering path. Explicit accounted-to-street capitalization
is validated and posted by `finance_system::validate_capitalize_street_cash` as
an auditable, one-way ledger transaction. Explicit orders select a single source;
autonomous racket recovery pools spendable clean accounts through the same validator
and preserves a two-day clean payroll reserve. A negative destination balance
must be cleared in full; neither path grants access to another organization's cash.
The adapter only selects its player's accounts, and reports spendable balances
separately from net balances. Read-only leadership summaries aggregate actual
owner-scoped operating cycles over a bounded trailing 30-day window and compare
their daily net with current payroll, without projecting hidden case activity or
counting laundering as operating income. The text client also projects own
suspended rackets through the enterprise owner's conditional outlook and exact
canonical resume validator, and suspended storefronts through the legitimate
business lifecycle validator. Conditional racket forecasts additionally show the
one- and two-extra-case net sensitivity without reading hidden police cases.
It also derives the authored per-cycle baseline police-inquiry chance from the
same enterprise definition as settlement, showing observed-pressure sensitivity
only when the player has dated direct evidence. An existing inquiry suppresses
duplicate intake, and pausing a racket prevents new enterprise inquiry rolls
without erasing earlier casework. The leadership brief reports gross payroll
runway from actual accessible wage-payment balances, explicitly excluding
future revenue and other spending, so a partially funded organization can
recognize its approaching payroll crisis before members depart. Uninterrupted
text advancement counts confirmed player wage-related departures from the
canonical payroll outcome rather than reading former members' now-cleared
organization fields or exposing rival losses.
Players may explicitly select a set of rackets to resume in one prevalidated
order, or name a replacement manager and a selected set of suspended rackets for
atomic reassignment and resumption. The adapter stages that entire order on a
projected state using canonical enterprise and delegation validators and
commits no part of a rejected group. One-time reserve-bounded laundering orders
use the existing finance owner for each storefront, report any partial failure,
and never create a standing or autonomous player move. No recovery or conversion
command infers foreign owner identities or hidden police investigations.
Political district progress is a read-only own-books projection derived from the
same mature-title, recent-settlement, and income calculation as victory
eligibility; it exposes neither hidden rival territory nor independent policy.
Contact-channel reassignment is a contacts-owner validated transaction: the
replacement handler must be available, belong to the sponsor, and maintain a
live social relationship to the same institutional source. The old handler may
be detained. The contact retains its identity, disclosures, legal-representation
references, and established-at time; the current handler's relationship snapshot
and version are refreshed along with the reconstructed handler index. The
current-schema contact lifecycle accepts further version increments before
termination without adding a serialized field. Briefing views deduplicate only
the *displayed/selected* newest unseen subject/topic facts; canonical individual
contact disclosures preserve the original source IDs, chronology, epistemic
downshift and historical backlog. Consequential mutation still belongs
to validators/commits and the one-minute `run_tick`; the save envelope owns
future-affecting state, not the text client.

The generation composer `src/city_generation.rs` builds a new `AppState`
from a caller-supplied seed and bounded district/business/rival budgets.
An independent, platform-stable SplitMix64 stream selects district names,
economic archetypes and institutional presence, unique people, crew capabilities,
land uses, ownership, and starting relationships. It does not consume the
simulation's persisted operation, business, enterprise or investigation RNG
streams. Every entity is inserted or committed through the owning production
system: world organizations, people, neighborhoods and businesses; police
jurisdiction and patrol deployments; legitimate operating economies and
business-ledger accounts; initial street-cash treasury transactions; managerial
delegation mandates; genuine player and rival gambling enterprises; contact and
social records; information sources and first burglary opportunities. The
complete starting state is invariant-checked and has no generator-owned parallel
truth or persistent metadata. Preset sizes produce 4/8/16 districts with
48/144/384 businesses; custom plans are validated before any construction.
The first generated neighborhood is the home label for a saved campaign, while
the persistent world and all simulation state are restored directly from the
normal `SaveEnvelope`. Authored openings continue to build through
`examples/gameplay_harness/scenario.rs` for fixed-situation tests. The builder
returns complete validated `AppState`, including both opening opportunities;
the player adapter does not finish campaign construction.
The read-only `city_view::CityScene` projects existing streets, lots and
modeled business geometry deterministically from the generated `WorldState`;
road strips are visual topology and do not create persisted transport routes,
line of sight or pedestrian movement. Public browser exports filter both
dossier fields and GPU pick IDs through the authorized public directory.

Player governance orders remain in `delegation_system` through validated
policy and mandate operations, including assignment, revision and revocation.
The text operation planner derives role capabilities, supported approaches and
completion timing from immutable registry definitions. Automatic crew selection
uses a bounded maximum-weight role-mask matching over current member skills
and supervisory preference, without inventing trained personnel or assigning
the same person twice. Ties select the lowest IDs in stable role order.
Explicit role assignments remain fixed and can knowingly use untrained members.
The general plan/preview commands also admit optional registry-supported seats; only
required seats are auto-filled, and the operation catalog distinguishes the two.
Automatic roles and leadership exclude a direct person target using the same objective
projection as authorization and save validation; explicit self-target assignments are
rejected, not silently replaced.
Business directories are public; named people are selectable through current
membership, established contacts, recruitment prospects, or personally
identified information held by the organization. Sensitive witness and
detention objectives require the production operation owner's typed legal-person
status proof, never inspection of hidden cases from the player adapter.

Broad reconnaissance target selection lives in `text_play/reconnaissance.rs`.
The player adapter admits deliberately public neighborhoods and business
identities without further discovery. Organizations, people and administrative
records require a known ID from player membership, contact institutions,
recruitment candidate discovery, own operations/enterprises, or explicit
organization-held information; case-witness signals identify the specific
investigation without revealing its live state. The production reconnaissance
validator accepts that typed case reference as well as direct case subjects.
The adapter uses generic labels
for learned rival records, never reads hidden titles or current status to
populate the targeting directory, and defers objective viability to the
operation owner. Surveillance and infiltration observe the target through
`surveillance_integration`, producing first-party intel after the simulated
resolution rather than at authorization. Successful local watches use indexed district and venue enterprises to produce bounded typed enterprise-location and organization-personnel observations; they reveal only sampled active foreign operations. An institutional sightline about
player-originated cases does not confer access to all the authority's cases. Successful infiltration adds target-bounded internal findings from business ownership and settlement records, organization leadership and mandates, or enterprise support and settlement records. These findings participate in the same resolution snapshot, production information creation, and immutable after-action report reconstruction; incomplete infiltration does not acquire internal records.

Current business financial intelligence is scoped to the present ownership period, identified
by the business version frozen on each settled cycle. Both infiltration and document theft
consume the economy owner's indexed latest-cycle-for-version query. A transferred business
therefore has no current books until it settles again, even if title returns to a prior owner;
historical settlements remain intact for their original owners and accounting history.
Financial findings and court notices render money with the finance owner's currency formatter,
not raw diagnostic cent counts.

The planner attaches the registry's supported police contingencies, bounded
relative scheduling, completion deadlines and known-topic requirements to
`OperationDraft`; `operation_system` validates entry-stage support,
execution windows, intelligence freshness and crew reservations. Manual
`cancel` delegates to `operation_system::apply_transition` with its
authority-abort transition and never directly rewrites operation runtime.
The client treats a current recruitment prospect as identified because the
production recruitment owner already established a playable relationship to
that person. Executive prospect discovery and recruitment share a candidate-to-
recruiter map spanning every available unsupervised leader. Multiple eligible
leaders for one candidate resolve to the lowest character ID. Typed legal-person
operation authorization remains stricter than being generally acquainted with
a character.

Internal member observations stay with `KnowledgeHolder::Character` until
leadership issues `debrief`. `text_play/debrief_play.rs` enumerates
only current roster members' pending sources and calls
`intelligence_system::validate_information_transfer` for a single
organization-directed handoff. The resulting `InternalReport` preserves
the original observation, typed semantics, and source lineage. Existing
duplicate-lineage checks prevent repeated reporting; the intelligence owner
rejects ordinary transfers to or from detained characters at validation and
commit. The adapter displays custody-related unavailability. `intel` queries
only the organization's holder index and cannot read a guessed personal or
rival information ID.
Case-witness information begins with the staffed investigator; an actual
institutional contact disclosure conveys its episode-specific typed signal
to the organization, not mere surveillance of the whole authority.

The contact interface derives candidates from organization-held identities,
existing sponsor-owned contact records, and actual social relationships to
current staff. Undisclosed source details remain behind
`contact_system::validate_contact_disclosure`. Institutional establishment
and termination use the contact owner. `legal_play.rs` reads current member
custody and typed member detention knowledge, then retains or ends counsel
through `legal_representation_system`, funding a legal-services operating
account with a balanced ledger transaction. The authored opening contains an
independent legal-services organization, a known lawyer, and two real staff
relationships, not an automatic retainer. Extended text-play advancement
interrupts on own-member arrests and releases from the canonical
`TickOutcome`, rather than inferring unseen investigations. Automatic
legal-support decisions remain in the production tick before detainee informant
decisions. The player `advance` handler lives in `text_play/time_play.rs` and
caps requested ticks at the finite simulation horizon before mutating. The
leadership brief distinguishes active and suspended rackets, excludes retired history,
and shows earned storefront surplus and the persisted treasury instruction. Advances
pause on fresh own-storefront or racket suspensions and payroll shortfalls when
Exception auto-pause is enabled; summary reports do not re-trigger old alerts. The opportunity owner provides an organization-scoped record query;
the client does not reconstruct leads from duplicate lifecycle reports.

The finance owner exposes a bounded recent-transaction projection indexed from
the organization's accounts and an ownership-checked direct lookup; a transaction
touching two owned accounts is returned only once. The text adapter never renders
counterparty postings. Business and enterprise settlement history uses indexed
`cycles_for` queries, with current asset ownership checked by the adapter
and pre-acquisition business cycles excluded by their immutable settlement owner.
Recipient report history provides a double-ended ordered iterator so newest-first
UI lists consume only requested entries, not entire campaign report histories.
Player business status changes are validated and committed by
`business_economy_system`; the player cannot bypass an active racket
dependency or reset laundering capacity by suspending a front. District racket
creation resolves delegation authority against a real neighborhood; the
adapter requires the registry's physical-host functions before allowing a
business kind at district-only scale. All other supporting-network, account
and operating-state checks stay with `enterprise_execution`. Racket succession validates the replacement mandate against the complete operating network and commits through the enterprise owner, changing the active mandate index but retaining the original accounts and settlement schedule. Persisted authority history assigns each completed cycle and its legal evidence to the responsible manager at the settlement minute, including transfers after settlement in that same minute.

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
 5  core::simulation::run_tick        preflight finite clock, then one minute in stable contractual order
 6  Result<TickOutcome, TickError>    typed terminal-clock rejection or player-visible consequences
 7  build_save / restore_save         envelope {format_version, content_revision, state}
```

Tick cadence is an adapter concern. Calling `run_tick` faster or slower changes
wall time, not the semantics of one canonical minute. Clock exhaustion is checked before
the first mutation, so the terminal representable minute is a valid state with no successor tick.

### run_tick — contractual order (`src/core/simulation.rs`)

Phase order is the coupling contract. Comments at `src/core/simulation.rs`
explain each “runs after X so Y is visible” dependency. Reordering breaks
determinism and client contracts.

```text
 0  checked next minute                    reject ClockExhausted before mutation
 1  apply_due_custody_releases             hard arrest-custody boundary before same-minute consumers
    ──► apply_due_freelance_expirations    except at midnight; release expired terms before new bookings
 2  apply_opportunity_expiry               durable lifecycle report before remaining same-minute consumers
 3  run_operations_phase                   police arrivals → deadline cleanup → resolution → starts
 4    ├─ apply_due_police_response_arrivals (exposure → decisions; must precede completion/new starts)
 5    ├─ find_due_with_missed_deadlines → abort via decision when present
 6    ├─ find_due_in_progress → decide+validate+commit per operation (RNG: operation stream)
 7    └─ find_due_authorized → Begin or validated deadline/opportunity/objective abort;
        finite abort-artifact capacity blocks only abort candidates, never independent starts
 8  apply_autonomous_investigator_staffing single-seat staffing, lead-investigator knowledge
 9  apply_investigation_work_scheduling    one active-case scan; evidence review first, otherwise witness interview
10  run_investigation_work_phase           resolve due work (RNG: investigation stream)
11  apply_autonomous_evidence_arrests      active LawEnforcement cases: independent-evidence threshold → custody + responsibility preemption + managed enterprise suspension
                                            reconstruct authority/defendant release and cited-racket history for qualifying candidates; 60-day routine cooldown, no recycled vice incident after unrelated custody
12  apply_autonomous_prosecution_staffing  refill prosecution seats released by custody
    ──► apply_due_prosecution_referrals    daily one action per available prosecutor; due hearings first (oldest opening first), supplement verdict-changing evidence before judgment, then one ordinary queue for review work and initial intake (oldest last referral or arrest time, custody breaks ties); own-member notice
13  apply_automatic_legal_support          conclude only after custody, viable queued initial intake, and every open prosecution review end;
                                                retain for custody or open post-release prosecution,
                                                with pending detainee decisions funded first
14  apply_detainee_informant_recruitment   one decision after a delay; active counsel lowers authored flip chance
15  apply_informant_disclosures            holder-knowledge → handler cases
16  apply_cold_case_decay                  originated cases only, authored inactivity window, no RNG
17  run_business_cycle_phase               per due business (RNG: business stream); freeze local market before cohort settlement
18  apply_due_autonomous_business_lifecycle
                                                suspended non-player books reopen only when current zero-variance economics recover
19  run_enterprise_cycle_phase             per due enterprise (RNG: enterprise stream, 2 draws unconditionally)
    ──► apply_enterprise_observations      staffed vice file observes actual later cycle (RNG: investigation stream); personal evidence enters next minute's custody decision
20  apply_due_autonomous_profit_sweeps     earned NPC and opt-in player business surplus → accounted reserves, retaining working capital
    ──► apply_due_personal_owner_dividends  earned private proprietor surplus → personal accounted funds; both distributions share atomic cohort settlement
21  apply_daily_payroll
    ──► apply_due_character_lives         close civic work, experience and rest after settlements and wages, before midnight membership changes
    ──► apply_due_freelance_expirations    at midnight only, after final earned payroll and completed workday; expired terms take precedence over resignation
    ──► apply_payroll_departures
          wage-driven departures suspend live rackets before revoking mandates and releasing membership; suspended player rackets remain recoverable by succession; history gates return; assetless final abandonment can end a player campaign
    ──► apply_due_political_campaigns     after payroll; decay, funded work, rival spending, tenure
    ──► apply_due_fine_collections
          court fines collect only from convicted defendants' accessible personal funds, including wages, using the canonical payment order (street cash before accounted funds, largest balance first within each kind); a derived unpaid-balance index drops settled judgments and is rebuilt from the authoritative judgments and payment ledger
    ──► apply_reputation_phase           decay and current operation/racket consequences
    ──► apply_due_autonomous_recruitment  sees current resentment + decayed/current competence
    ──► apply_due_overstretched_manager_suspensions
          indexed active portfolios only; deeper branches before superiors; reconcile current command workload before expansion
    ──► apply_due_autonomous_acquisitions
          weekly rival citywide listings or negotiated purchases after payroll and recruitment; competing buyers rotate first access by review week instead of permanent organization-ID privilege; new infrastructure available for expansion
    ──► apply_due_autonomous_enterprise_lifecycle
          suspended NPC rackets: viable reopen; stale frozen authority/assets retire
    ──► apply_due_autonomous_enterprises_excluding
          new growth; mandates that reopened a racket already spent this daily action
    ──► apply_due_autonomous_territory_clashes  current local force after expansion
    ──► apply_due_autonomous_territory_standstills  negotiated NPC concessions, never automatic player agreement
    ──► apply_due_protection_demands      real commercial exposure before rival planning
    ──► apply_due_rival_operations        daily mandate-backed commercial reconnaissance/strikes, after economic decisions and reputation
    ──► apply_due_mayoral_elections       exact scheduled boundary; final reputation, roster, properties and racket status
    ──► synthesize_executive_brief        sees every report/decision made this minute, last
    ──► validate_invariants               structural + registry re-derivation
```

New autonomous work must slot explicitly here with a rationale comment. The reputation
phase does not build a standing-score projection on quiet non-day-boundary minutes; the
canonical reputation owner still applies age-gated decay at each due day boundary and every
same-minute operation or racket consequence is still preflighted before mutation.
When player Standing-report IDs cannot fit the full same-minute consequence batch, that
batch remains unapplied; independent daily decay and non-player standing consequences
continue instead of freezing the city's impressions for lack of a player report.

Due authorized work is planned as a complete ordered cohort. Every needed pre-start abort
must pass its owner's validation and the combined artifact-ID budget before any abort commits;
no ID-ordered prefix may consume the remaining IDs. An abort whose record has reached a
finite version rail remains authorized without an invalid transition. Other due operations
that can still begin are admitted independently, even when one or more aborts cannot persist
their required artifacts. A blocked operation never bypasses its abort by beginning instead.

Operation begin tokens re-evaluate canonical admission immediately before committing. An
unchanged operation version and simulation minute alone cannot prove that a business still has
the required owner or operating status, a crew member remains available, or the same policing
context still warrants dispatch. The current admissible objective and police-response plan must
be resolved before any response or operation lifecycle state is persisted.

Due legitimate-business and criminal-enterprise settlements also distinguish an individually
blocked financial rail from a corrupt transaction. A business whose operating or settlement
account has insufficient representable balance headroom leaves its cycle due and consumes no
business RNG draw; independent businesses settle in the same pass. Enterprise settlements
apply the same rule per racket while preserving their sequential shared-cash and legal-pressure
semantics. The blocked financial state remains visible and reversible through canonical
ledger or lifecycle commands; unrelated due cycles cannot be held hostage by one saturated
account. Malformed account ownership, invalid dependencies, and other unexpected finance
failures still surface as errors rather than being treated as harmless capacity limits.
When enterprise-cycle IDs are too scarce for every due racket, the scheduler first previews
each racket's own canonical settlement artifacts without consuming RNG or mutating state.
Rackets whose minimum artifact footprint is already unallocatable do not consume a slot in
the mandatory cohort: an unreportable notable loss cannot indefinitely suppress a viable
routine racket. The remaining cohort must fit the cycle-ID allocator in full before any
settlement commits; competing viable rackets never receive an arbitrary ID-ordered prefix.
The ordinary capacity-sufficient path keeps its single sequential economic resolution pass.

Confidential-source reports can enter a police case from several informants who learned the
same original observation through internal handoffs. The informant system counts that
observation once per investigation, not once per person repeating it: direct disclosure
validation and the autonomous same-minute cohort deduplicate by the internal information
root before issuing evidence. Independent direct observations remain separate and may both
be disclosed. Case-scoped indexed reads avoid scanning unrelated historical disclosures;
restore validation rejects duplicate case/root evidence provenance.

## Source map — one owner per field

Every `src/` subsystem owns its records and canonical mutation paths. Invariants
are validated by `src/core/invariants/`. The top-level tick is
`core::simulation::run_tick`, driven from `AppState` and `Registry`. Its root file owns
phase order, reputation, executive synthesis, and domain RNG helpers;
`core/simulation/operations_phase.rs` owns due operation admission/cleanup/resolution and
`core/simulation/scheduled_phases.rs` owns due investigation-work, business, and enterprise
cycle orchestration. Those children compose canonical domain mutations rather than creating
competing state owners.

| Module | Owns | Canonical mutation | Key file |
|---|---|---|---|
| `city_generation/` and `city_view/` | Seeded composition of simulated districts, businesses, institutions, rival crews, capital, rackets and leads, plus deterministic read-only geometry. No parallel generator-owned runtime state or save schema. | Generator composes existing domain `insert_*` and `validate_* → commit` operations; `CityScene` projects `WorldState`. | `src/city_generation.rs`, `src/city_generation/tests.rs`, `src/city_view.rs`, `examples/city_gui_preview.rs` |
| `core/` | `SimTime`/`SimDuration`, typed persistent IDs (`IdCounters`), entity refs (`EntityRef`), attention classes, `AppState`, persistence envelope, tick pipeline, invariant validation; due-operation resolution excludes individually unallocatable mandatory after-actions before the shared all-or-none cohort budget so a permanently blocked operation cannot freeze independent work | `core::simulation` runs the tick; `core::state` owns generated state; `core::invariants::validate_state` | `src/core/state.rs`, `src/core/simulation.rs`, `src/core/simulation/operations_phase.rs`, `src/core/simulation/scheduled_phases.rs`, `src/core/invariants/mod.rs` |
| `registry/` | Immutable authored definitions and validated lookups, including one shared information reliability/specificity quality mapping consumed across domains | `RegistryBuilder` owns registration/completeness; focused authoring validators check domain contracts before insertion; read-only after `content::build_registry` | `src/registry/mod.rs`, `src/registry/builder.rs`, `src/registry/operation_validation.rs` |
| `src/content/` | Code-owned authored definitions for the registry | `build_registry` | `src/content/mod.rs` owns `CURRENT_CONTENT_REVISION` |
| `world/` | Organizations, characters, neighborhoods, businesses, institutional profiles, designation, versioned per-policy organization storage, daily payroll and wage-driven membership departures; read-only territory-influence aggregation | `world_system` (insertion, designation, world-owned versioned policy write used by governance orchestration); `payroll_execution` (daily wage pass through canonical ledger, relationship, and report paths; existing negative personal street-cash accounts are never implicitly garnished as wage destinations); `territory_influence` (read-only district summaries, never an omniscience feed) | `src/world/world_system.rs`, `src/world/payroll_execution.rs`, `src/world/payroll_departures.rs` |
| `social/` | Directional character relationships with source/target indexes | `relationship_system` only; requires active endpoints | `src/social/relationship_system.rs` |
| `intelligence/` | Provenance-bearing information, holder/topic indexes, lineage, and owner-derived planned source identity for atomic composites that must validate a dependent artifact before the information record is inserted. Internal transfers preserve the original observation root across any number of peer/organization hops: a holder may receive a particular root once, and a copy cannot be sent back to the original observer. A separately observed fact or contact disclosure starts a distinct root. The root/recipient index is derived from persisted provenance and rebuilt on restore; release-safe validation rejects duplicate roots per holder. | `intelligence_system` (record, transfer, planned source projection); `IntelligenceState` maintains the derived holder/root indexes | `src/intelligence/intelligence_system.rs`, `src/intelligence/mod.rs` |
| `reports/` | Player-facing reports, briefs, financial reports; automatic executive synthesis checks the authored cadence but only publishes actionable entries or an initial/weekly quiet heartbeat. A derived append-only latest-per-recipient-and-kind index eliminates campaign-length history scans during executive-brief admission and is reconstructed on restore. Bounded executive-brief sources rank by attention then newest report chronology, while pending decisions remain oldest-first within an attention class. Discovery reports are no longer synthesized as active opportunities after their validity deadline, even if report-ID scarcity deferred the opportunity's expiry record. Composite report validation may cite one intelligence-owned planned source, but report commit remains fail-closed until that exact source exists | `report_system` plus `executive_brief` synthesis | `src/reports/report_system.rs`, `src/reports/executive_brief.rs` |
| `history/` | Durable entity-linked campaign events. The history owner maintains reconstructed, non-persistent latest-event indexes for district standstills, attacker cooldowns, defeated rackets, and repulsed attackers. Territory queries use those heads instead of rescanning lifetime history; distinct repulse history survives later victories. All indexes rebuild from events on restore and are release-safe checked against the records. | `history_system` owns validated event commits and `HistoryState` owns the derived indexes | `src/history/mod.rs`, `src/history/history_system.rs` |
| `finance/` | Typed accounts, allocator-neutral planned account openings, balanced ledger, canonical basis-point money rounding shared by runtime scaling and registry arithmetic proofs, criminal-organization laundering transfers through owned cash-intensive fronts with a nonzero authored fee; delegated budgets authorize outflow only from their designated funding account and never provide implicit credit beyond that account's actual balance. The generic ledger primitive is the finance owner's trusted composition/setup surface, but it cannot post to a business-owned legitimate operating account once that account is attached to live business books; those tills are mutated only by the economy/laundering owners, and held generic tokens recheck that protection at commit. | `finance_system` remains the public owner/facade for account, ledger, budget, and laundering mutations; `FinanceState` owns rebuilt per-account transaction chronology used by bounded historical balance reconstruction; `finance_system/laundering.rs` owns laundering-specific validation and composes the parent ledger path with the economy-owned plausibility-capacity update; the capacity query, validation and held-transfer commit share the live market-adjusted gross ceiling, so a same-minute change to a competing storefront cannot authorize excess laundering; `helpers` owns shared financial arithmetic and treasury projections: gross accessible liquidity funds payments and wage reserves, while net reserves deduct deficits and exclude inaccessible positive balances; business acquisition, seller consent, rival campaign spending, and victory qualification share these projections | `src/finance/finance_system.rs`, `src/finance/mod.rs`, `src/finance/finance_system/laundering.rs`, `src/finance/helpers.rs` |
| `operations/` | Criminal-organization operation plans, execution records, participant reservations, abort causality and artifacts, post-resolution information acquisition, police-response, property, and take-economics integrations. Authorization, timing, abort lifecycle, objective viability, deterministic resolution, proceeds, and after-action reporting run through the named sibling systems. After-actions carry only crew-observed operational and exposure facts; hidden institutional case intake and routing become organization knowledge only through legal, intelligence, or contact channels. | `operation_system` (authorization/start; private `authorization_validation` and `start` children isolate plan validation from begin-time admission), `operation_scheduling` (timing, booking, deadline, due-work projections backed by the operation owner's derived active-deadline index), `operation_intelligence` (shared execution-planning value and freshness scoring), `operation_basis_knowledge` (typed episode proof for sensitive legal targets; own extraction outcomes establish known custody end, while hidden foreign release does not retroactively invalidate a learned report), `information_acquisition` (post-resolution dispatch/provenance for surveillance and acquired records), `operation_abort` (abort lifecycle and artifacts), `operation_objective` (execution-time objective blockers), `operation_execution` (resolution planning/orchestration; private `resolution_commit` owns cross-domain artifact validation plus atomic commit alongside `resolution_factors`, `resolution_effects`, `incident_intake`, and `narrative`), and `operation_economics` (proceeds, replenishment, and frozen historical disruption facts for resolved takes so later sabotage, repair, or business-establishment changes cannot reprice them). `surveillance_integration` and `document_theft_integration` own their distinct observation/snapshot semantics beneath `information_acquisition`; `surveillance_integration/observation_text.rs` owns surveillance prose/signals while `surveillance_integration/after_action.rs` owns surveillance historical reconstruction. | `src/operations/operation_system.rs`, `src/operations/operation_basis_knowledge.rs`, `src/operations/information_acquisition.rs`, `src/operations/operation_execution.rs`, `src/operations/operation_execution/resolution_commit.rs`, `src/operations/surveillance_integration.rs`, `src/operations/document_theft_integration.rs` (full sibling map in `src/operations/mod.rs`) |
| `opportunities/` | Provenance-backed opportunities with lifecycle; sensitive witness-pressure conversion requires the operation target's own recorded case knowledge, not case knowledge attached to another contextual target in the opening. Due expiries reserve the complete report batch before lifecycle mutation; an unreportable batch remains deadline-inert and restore-valid only while that full batch still cannot fit the report ID rail. | `opportunity_system` | `src/opportunities/opportunity_system.rs` |
| `decisions/` | Durable typed decision records and pending indexes; recruitment approvals snapshot the exact mandate/manager/policy-source revisions and are cancelled when their frozen mandate authority or organization-sourced effective policy is permanently superseded | `decision_system` remains the public facade for generic request/resolution orchestration and operation decisions; `decision_system/recruitment_approval.rs` owns recruitment-specific request creation, autonomous resolution composition, obsolete-approval cancellation, and authority/policy freshness. Governance transitions compose its validated cancellation token instead of mutating decision state directly. | `src/decisions/decision_system.rs`, `src/decisions/decision_system/recruitment_approval.rs` |
| `delegation/` | Organization-owned mandates and responsibility indexes; organization-policy governance over settings stored by `world`. A mandate standing order is valid only when the mandate includes the corresponding functional responsibility scope: Personnel for independent recruitment and Legal for associate legal support. | `delegation_system` owns assignment/revision/revocation, policy resolution, and the canonical organization-policy command; mandate or organization-policy changes compose decision-owned cancellation for approvals they permanently supersede. Independent recruitment can be delegated to a named roster ceiling, including zero (pause). Recruitment validates the effective limit before planning and again at held-token commit; same-minute cohort planning counts earlier accepted admissions across all authority paths, rather than granting each manager the same unspent slot. Historical recruitment records retain the actual resolved policy and its source; no new parallel roster counter is persisted. | `src/delegation/delegation_system.rs` |
| `enterprises/` | Routine criminal enterprises and cycle history with an Active → Suspended → Active/Retired lifecycle; Retired is terminal history and releases the kind/location slot. Each enterprise records its rank-and-file associate complement and same-minute historical staffing timeline: baseline workers are included in authored fixed operating costs; additional workers generate demand-sensitive income and 1,500 cents of daily wages per worker through canonical settlements and financial projections. Named managers are constrained by capability-scaled supervision workload over active rackets, associates, supporting storefronts, districts, and upstream operating branches; establishment, staffing, succession, resumption, autonomous expansion, and restore validate those actual reporting responsibilities. Staffing changes are versioned, reject stale/due-cycle orders, and historical settlements rederive the actual headcount at the time; current-only save schema is explicit. Underboss victory requires mature profitable personnel and independent captains in the same uncontested districts, with local rank-and-file strength exceeding that of combined rivals. Authored support networks are minimal: every listed support business must satisfy at least one required network function that the remaining host/support set cannot provide, and registry-relative restore validation re-derives the same rule. Per-cycle enforcement-attention rolls use the racket's own standing exposure as one risk source and add an authored increment for every active district case; a clean district can develop its first vice inquiry through canonical incident intake. Daily NPC maintenance reconsiders suspended rackets from the organization's own observable pressure and current working-capital runway: viable positive-net rackets may reopen on a later boundary, temporary heat/funding/economic blockers leave them suspended, while revoked/re-scoped authority, lost required host/support ownership, or Tight/Guided manager autonomy at the maintenance boundary makes an autonomous rival abandon that stale frozen configuration and retire its record so it cannot reserve a slot forever. The retirement decision is independent of temporary resume blockers and precedes the canonical resume validator. One mandate may reopen at most one racket per daily pass, and reopening consumes that mandate's expansion action. The recovery cohort projects all accepted captains' combined command workload, including their shared superiors, before accepting another candidate. Both maintenance and expansion reserve a full operating cycle for existing active rackets at their actual associate count, including additional wages. Recovery capitalization is planned alongside the selected resumptions, with combined ledger-ID capacity checked before mutation and enough destination posting headroom retained for the next cycle. New delegated expansion then plans the exact same-day cohort read-only, projecting working-capital claims, occupied slots, district leadership, first footholds, strongest racket workforce, and settlement-account claims before reserving the cohort's complete enterprise/account ID capacity; canonical establishment commits only after that preflight. Each cohort selection first resolves each organization's preferred mandate candidate by internal authority rank and economics, then compares organizations by district leadership and economics with stable ID tie-breakers. Player-owned operating commercial wards without active player rackets and their distributed defense are snapshotted once per expansion pass; earlier selected rival establishments update that rival's local presence and strongest workforce, so a second mandate cannot claim first-foothold priority in the same ward. Both lifecycle and growth consume bounded planning projections derived from organization-observable pressure rather than hidden live investigations, while settlement reads authoritative legal truth. Notable settlements publish a Financial report citing the manager's own observation. | `EnterpriseLocation::neighborhood` owns shared read-only resolution of venue- or district-backed locations to persisted districts; callers map a missing location to their own typed validation error. `enterprise_state` owns durable storage, derived-index reconstruction, and synchronized internal state mutation. `enterprise_execution` remains the public owner/facade for lifecycle, staffing, succession and atomic settlement; `staffing.rs` owns the authoritative manpower span and staffing-token validator, `economics.rs` owns live and historical associate wage/output arithmetic, and its `cycle_planning` sibling owns the read-only per-cycle decision, economics/heat classification, and staleness snapshot assembly. `autonomous_planning` owns shared bounded-pressure/runway projections, `autonomous_lifecycle` owns daily suspended-racket maintenance and plans its complete retirement/resumption cohort before mutation, `autonomous_expansion` owns daily delegated growth and canonical commit; its private `cohort_planning` child owns exact same-day cohort projection while `candidate_planning` owns read-only venue/network enumeration and economic ranking, and `enterprise_reporting` is read-only, including owner-scoped operating outlooks using the shared observed-pressure projection (dated settlements, never live cases). | `src/enterprises/enterprise_state.rs`, `src/enterprises/enterprise_execution.rs`, `src/enterprises/enterprise_execution/cycle_planning.rs`, `src/enterprises/autonomous_planning.rs`, `src/enterprises/autonomous_lifecycle.rs`, `src/enterprises/autonomous_expansion.rs`, `src/enterprises/autonomous_expansion/cohort_planning.rs`, `src/enterprises/autonomous_expansion/candidate_planning.rs` |
| `economy/` | Legitimate business economies, cycle history, sabotage/arson disruption horizons of exactly the authored minute duration, and chronic-loss suspension. A daily autonomous pass reconsiders suspended non-player businesses through the canonical resume token: same-minute suspensions remain consequential, currently positive zero-variance books may reopen, structurally losing books stay suspended, player-organization ownership remains an explicit player decision, and a business whose next recurring cycle cannot fit inside the finite simulation clock remains suspended rather than failing routine maintenance. The pass validates each financially viable reopening before choosing at most one per competing local sector, ranked by incremental owner daily net after local portfolio displacement and applicable administration charges (stable business-ID ties), then commits all winners in stable ID order. No creation-order preference overrides profitability. Owner profit sweeps may distribute only operating cash above the persisted working-capital floor captured when the economy is established or ownership restarts; at each day boundary NPC criminal owners and opted-in player owners draw that earned surplus as one preflighted balanced transaction after racket settlement and before payroll, reusing combined receiving headroom across existing accounted-funds accounts before opening any new destination and limiting each source by its settlement account's fee capacity. Manual player withdrawals remain the default and use the same validated receipt planner, excluding debt-bearing and unavailable accounts and atomically opening any necessary receipt account; same-owner suspension/resumption preserves that snapshot, while acquisition rebases it to the positive operating cash actually inherited at the new ownership revision. The floor is version-pinned to ledger and business history so restore can re-derive it rather than trusting a mutable balance summary. An operating account may receive opening capital before establishment, but once attached to live books its till is protected from generic ledger credits and debits; cycle settlement, laundering, and canonical owner sweeps use the internal business-ledger path. Current-window laundering provenance links the exact ledger transactions behind the front's running plausibility total; settled cycles and acquisition restarts open a fresh window, while same-owner suspension/resumption retains consumed capacity until the next settlement. Listed acquisition buys independently owned businesses at the authored kind price from organization-owned accounted funds, crediting the acquired business's non-liquid seller-settlement counterparty rather than its operating cash. A negotiated offer extends the same transaction to a non-player organization-owned property with operating books: the seller declines if the price cannot cover standard asset cost plus ninety days of forecast positive net income, if non-retired rackets use the property, or if the staffed criminal seller's productive asset is needed for its wage base without a feasible replacement, or still within its first ninety days after acquisition. Autonomous buyers use public candidate appraisals, buyer-adjusted consolidation prices for listings and negotiated bids, actual accounted reserves, and private live-market forecasts of their own scheduled stores. The purchase must increase whole-portfolio daily distributable earnings after projected administration charges and any local market displacement from reopening the target; positive cycle receipts pay the canonical rounded charge, while losses remain full losses. The owner forecast computes current, same-district purchase and new-district purchase totals in one traversal. Already-active targets cause no new competition, and unschedulable terminal economies are never bought autonomously. Seller consent is evaluated privately using the actual marginal earnings lost on sale, the post-sale administration rate and wage gap, and feasible replacements at post-sale prices and administration rates. All retained-portfolio and replacement projections include the transferred property's restart as a competing business (preserving its existing disruption), even when it was suspended before the sale. Personal proprietors apply the same post-sale market projection when determining whether a remaining productive livelihood exists. The player is excluded as an unsolicited seller. Seller ownership/version and eligibility are rechecked before commit; the accepted price is credited to the actual seller's accounted funds, opening a pocket atomically if needed. The atomic purchase dismisses the buyer’s obsolete single-target foreign-only leads, publishes anonymous market availability changes to the player, and records property-transfer observations for political and commercial staff (labor staff only for union-access venues). Weekly non-player criminal executive investment follows payroll and recruitment before racket maintenance/expansion: one affordable positive-incremental-portfolio purchase per organization in a weekly rotated stable roster order, preserving a week of payroll across all spendable wage-eligible reserves and paying purchase consideration only from accounted funds. Each purchase is independently atomic, uses the same canonical transfer and ledger as a player purchase, and consumes no random draws. | `business_economy_system` remains the public owner/facade for establishment, disruption, lifecycle, autonomous recovery, and owner sweeps; its private `autonomous_profit_sweep.rs` child plans authorized daily owner distributions through the same retained-capital rule and the finance owner's atomic ledger/account-opening transaction; `business_economy_system/cycle_planning.rs` owns the complete deterministic business-cycle decide/validate/commit transaction plus shared live/historical financial arithmetic, and composes canonical enterprise suspension before chronic-loss closure when active rackets depend on the failing front; `business_acquisition` owns canonical purchase composition; its private `autonomous` child owns weekly rival investment selection; `business_reporting` is read-only | `src/economy/business_economy_system.rs`, `src/economy/business_economy_system/cycle_planning.rs`, `src/economy/business_acquisition.rs` |
| `legal/` | Jurisdictions, patrols, timed police response, and origin-typed investigations (operation exposure or enterprise enforcement attention) with evidence, arrests, custody, representation, prosecution, witnesses, and informants. Explicit patrol windows are concentrations layered over a registry-authored residual share of ambient district police presence between windows; learned timing can make a period safer but never establishes zero enforcement, and persisted response presence is re-derived against that authored rule on restore. Incident intake routes distinct identified operation incidents into a compatible active file for the same suspect, originating organization and authority, preserving separate primary evidence and reserved detective-work version headroom; otherwise compatible existing shelf continuation remains available. A different
incident never resumes a suspended case that already registered its eyewitness,
listed that person as a subject, or assigned them incompatible casework; same-
origin duplicate-witness admission still rejects through canonical validation.
Intake pins its validation minute and target file version; held tokens cannot silently shift a case opening or append prior-minute evidence after the clock advances. Only originated cases decay cold; live custody under the same file defers decay without aborting the batch, and detention under an unrelated file never clears the case. Detective staffing is single-seat and conflict-free: active cases are prioritized by evidence, then a deterministic capability-ranked matching pass reassigns flexible cases when necessary to avoid leaving another eligible case unstaffed solely because of subject/witness recusals. The complete match is validated before any seat is filled. Actionable evidence newly implicating staff recuses that role with provenance and cancels conflicting pending work. Scheduled detective capacity, unattempted reviewable evidence, and unstaffed-case evidence priority are rebuilt derived projections rather than recurring scans of lifetime work/evidence history. Each reviewable evidence source gets one actual evidence-review attempt across direct and autonomous scheduling; cancellation releases that source for retry because no review occurred, while an inconclusive completed review cannot be rerolled until success. Autonomous scheduling treats work whose due time cannot fit inside the finite simulation clock as unschedulable, while the direct scheduling command still reports the typed clock-capacity error. Testimony is single-statement per case-witness registration over case-connected entities only. Direct and autonomous custody share one authored corroboration bar over independent non-weak, non-questionable, admissible sources; the autonomous arrest pass evaluates each active case's evidence once for all its character subjects while retaining separate corroboration and citation sets per suspect. Renewed custody in the same case requires a fresh qualifying independent source from the release minute or later; even questionable evidence cited for the prior detention retains its source identity, so later forensic improvement cannot reset custody. Routine autonomous custody additionally waits sixty days after the subject's most recent release by the same police authority, across all investigations, unless qualifying evidence cites a new criminal operation resolved after release. This is a look-back over persisted arrest history and does not change the direct evidence-backed arrest validator. Runtime admission and restore enforce the same-case repeat rule. Arrest custody ends at the authored maximum while prosecution referral and review can continue after release. Automatic legal support resolves before the detainee's one-time informant decision; closing completed automatic matters remains independent of artifact scarcity for new retainers, while the viable new-retainer cohort remains all-or-none. The minute-by-minute informant disclosure pass chooses the smaller indexed side of the handler case-subject/personal-knowledge join, preserving same-minute propagation to every matching case, unrelated-fact exclusion, and per-case uniqueness of originating observations across internal-report copies. Each relevant case's disclosed roots are projected once per planning pass; independent observations remain admissible and duplicate roots are rejected on restore. | Named modules (`jurisdiction_system`, `patrol_system`, `investigation_system`, `arrest_system`, `legal_representation_system`, `prosecution_system`, …) via `legal_state`; `investigation_system/autonomous_staffing.rs` owns detective allocation and `investigation_system/cold_case_decay.rs` owns originated-file inactivity lifecycle, both through the parent investigation owner's canonical mutations; `investigation_work_execution` owns direct scheduling/cancellation, its private `resolution.rs` child owns stochastic resolution/factor validation/evidence persistence, and `investigation_work_execution/autonomous_scheduling.rs` owns the deterministic per-minute scheduling policy over the active-case and unattempted-review indexes; `legal_representation_system/automatic_support.rs` owns deterministic automatic-policy orchestration but composes the parent system's canonical retain/end commands; `legal_state.rs` owns durable records and derived-index reconstruction, `legal_state/queries.rs` owns read-only indexed observation, `legal_state/mutations.rs` owns record mutation plus synchronized index maintenance; `records.rs` is a stable facade over domain-specific record vocabularies in `records/casework.rs`, `records/custody.rs`, `records/representation.rs`, `records/prosecution.rs`, and `records/policing.rs`, which define shapes only and do not become competing state owners | `src/legal/investigation_system.rs`, `src/legal/investigation_system/autonomous_staffing.rs`, `src/legal/investigation_system/cold_case_decay.rs`, `src/legal/investigation_work_execution.rs`, `src/legal/investigation_work_execution/resolution.rs`, `src/legal/investigation_work_execution/autonomous_scheduling.rs`, `src/legal/legal_representation_system.rs`, `src/legal/legal_representation_system/automatic_support.rs`, `src/legal/legal_state.rs`, `src/legal/legal_state/queries.rs`, `src/legal/legal_state/mutations.rs` |
| `contacts/` | Institutional contacts and provenance-preserving disclosures; pending disclosure offers rank factual freshness by observation time, using recording time and ID only as deterministic tie-breakers so late forwarding cannot make stale facts outrank newer observations. Establishment snapshots are historical provenance only: new disclosure or legal-channel use also requires at least one currently sustaining relationship edge, so a socially collapsed contact becomes dormant without cross-writing its lifecycle record. | `contact_system` (establishment, termination, disclosure, current-channel usability; `find_pending_disclosure_sources` read-only offer surface) | `src/contacts/contact_system.rs` |
| `recruitment/` | Relationship-gated recruitment, cooldowns, approvals, membership changes; autonomous cross-organization prospect contention is relationship-priority rather than mandate-ID priority, a pending approval exclusively owns that organization/candidate route so generic world reassignment cannot bypass it while the exact approved decision can complete it, and the canonical candidate-discovery query excludes pending approval routes and prospects whose cooldown endpoints lie beyond the finite simulation clock while an explicit attempt against the latter retains the typed overflow. Autonomous manager contention consumes that already-filtered snapshot and tracks only newly claimed prospects, with no duplicate availability policy. A manager chooses an unused pitch from the authored approaches after a refusal within two full rounds of cooldowns, taking a break after exhausting those approaches unless a relationship revision permits reconsideration. The recruiting player organization receives a Notable sourced outcome report through the same atomic attempt budget as the incumbent departure/refusal notice. NPC approval-required recruitment uses the same sole available executive resolver as property investment and political spending, both during candidate preparation and autonomous decision validation; independently delegated recruitment and explicit player approvals do not require that automatic executive consent. Approval-required autonomous candidates are deferred for the authored recruitment cooldown after leadership rejection, using one read-only decision-history projection per planning pass rather than rescanning all decisions for each prospect; no candidate attempt is invented, and a deliberate executive approach or delegated authorization can override that deferral. The daily planner excludes individually unallocatable pitches or approvals without claiming their prospects or suppressing independent routes, but reserves the complete viable cohort's artifact budget before committing anything so shared ID scarcity cannot favor an arbitrary prefix. | `recruitment_system` owns canonical recruitment transactions; its private `planning.rs` child owns read-only plan construction plus held-plan state/definition revalidation; `autonomous_recruitment` owns the deterministic daily delegated policy pass; `world_system` owns the guarded membership, supervisor and versioned autonomy mutations; `scoring` owns factor/margin arithmetic shared by planning and invariant re-derivation | `src/recruitment/recruitment_system.rs`, `src/recruitment/recruitment_system/planning.rs`, `src/recruitment/autonomous_recruitment.rs`, `src/recruitment/scoring.rs`, `src/world/world_system.rs` |
| `reputation/` | Criminal-organization-only per-audience standing: direct mutation rejects non-criminal organizations, and persistence validation rejects non-criminal reputation records even when keys and contents are internally consistent. Per-dimension movement freshness and age-gated baseline decay; fed by operation consequences (non-surveillance success competence, exposure police fear for every kind, violent-approach business and resident fear) and enterprise vice inquiries, consumed by recruitment scoring and expansion posture; player shifts surface atomically with Standing reports, and reported deltas are the actual bounded score movement after clamping rather than the larger requested change | `reputation_system` (`apply_reputation_delta` is the single score mutation path; consequence composition and decay are tick passes) | `src/reputation/reputation_system.rs` |

The text play client at [`examples/gameplay_harness/`](examples/gameplay_harness/main.rs), and verification at [`scripts/verify.ps1`](scripts/verify.ps1) / [`scripts/verify.cmd`](scripts/verify.cmd), live outside `src/` and use the canonical paths above.

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
Time-bearing legal admissions (detective-work scheduling, case-witness registration, and
informant establishment) pin the minute at validation and reject stale commits rather
than silently assigning a new due date or historical establishment timestamp.
Autonomous arrests still count a primary fact and its forensic derivative as one
corroboration source, but cite the qualifying record with the strongest
strength/reliability assessment, retaining the primary on equal-quality ties.
If those strongest citations alone cannot demonstrate same-case repeat-custody novelty,
automation also cites a fresh qualifying record from a previously uncited independent
source when available, without increasing the independent-source count.
When a composite artifact must reference a record that the same commit will create, the owning
system predicts that record's next ID without consuming it and exposes only an owner-derived
planned-source token; the outer commit stales if allocator position changes, and the dependent
owner still refuses to commit until the source actually exists. On rejection authoritative state
is unchanged unless the contract explicitly records a diagnostic (e.g. a `HistoryEvent`).

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

See `src/finance/mod.rs`, `src/enterprises/enterprise_state.rs`, `src/economy/mod.rs`
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
| `enterprise_rng` | `simulation.enterprise_rng` | enterprise gross variance + enforcement-attention roll (both **unconditionally** per cycle) | `draw_basis_point_variance` + `draw_index(10_000)` |

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
  Authorized operations reserve projected windows, but live work owns its crew until actual
  completion or abort, even after its scheduled resolution time. Structural validation rejects
  multiple live commitments for one participant independently of projected-window overlap.
  Completion deadlines are binding even when artifact capacity blocks overdue cleanup: the
  resolution cohort excludes expired operations, and completed records cannot postdate their
  authored deadline.
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
   originally-disjoint authorized booking is later reached by an unresolved decision pause.
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

Predicates use `is_`, `has_`, or `can_`. Do not introduce new `create_*`, `make_*`, `execute_*`, `perform_*`, or `attempt_*` in production code when an established role already fits. Test-only fixtures under `#[cfg(test)]` keep the `make_test_*` form sanctioned by [`TESTING.md`](TESTING.md).

Multi-file suffixes use established roles: `_execution`, `_integration`, `_loader`, `_ui`, `_adapter`. Every `src/` file starts with a concise `//!` purpose statement; multi-file subsystems state sibling relationships where not obvious. Comments explain constraints, ordering, safety, invariants, or non-obvious intent — not history or commented-out code.

## Dead code and replacement

- Behavior that should run is wired into the canonical path (`run_tick` or an explicit system call).
- Test-only fixtures and helpers live under `#[cfg(test)]`.
- Obsolete behavior is deleted with its tests and documentation. No historical shims.
- Do not add fake call sites, broad `allow(dead_code)`, public shims, or test-only production APIs to silence warnings.

One implementation owns each concern unless an active external compatibility contract explicitly requires otherwise.
