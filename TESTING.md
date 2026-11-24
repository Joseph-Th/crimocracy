# Testing

Owns test selection, text play evidence, and local verification. Ownership is in
[`ARCHITECTURE.md`](ARCHITECTURE.md); scope is in [`STATUS.md`](STATUS.md);
cockpit routing is in [`AGENTS.md`](AGENTS.md).

## Test selection — narrowest proof first

| Change | Focused feedback | Completion lane |
|---|---|---|
| Documentation/routes only | `.\scripts\check-docs.cmd` (compile-free) | same command |
| Syntax / types | `cargo check-fast` (no linking) | `.\scripts\verify.cmd -Check` (adds fmt) |
| One library behavior | `cargo test-focused <filter>` | `.\scripts\verify.cmd -Fast -Filter <filter>` |
| Library implementation | `cargo check-fast` or focused test | `.\scripts\verify.cmd -Fast` |
| Text play client | `cargo test-harness text_play` and `cargo text-play --seed 12` (use `cargo text-play-debug` for per-minute debug invariant checks) | `.\scripts\verify.cmd -Harness` plus a real text-play session |
| Persistence, invariants, or cross-domain | Owning module's focused test plus a save/restore round-trip | `.\scripts\verify.cmd` (broad gate) |
| Mixed-state stress | `cargo soak` (always explicit, never part of a gate) | same command |

Managerial span regression evidence lives in `src/enterprises/enterprise_execution/tests/staffing.rs`: workload is derived from the actual manager's capability, active rackets, headcount, geographic spread, and active immediate reporting branches; extra field workers cannot increase personal command capacity. Idle hires do not enlarge existing captains' or executives' authority, while actual reassignments under skilled captains or an underboss free capacity. Two individually viable same-day NPC expansions cannot bypass a shared superior's combined capacity, which is projected before batch commit. Changing real reporting lines can create day-boundary overextension, preferentially suspending newer rackets in the smaller branch through the canonical lifecycle. Custody suspends actual managed rackets, and save/restore preserves the resulting lifecycle. `finance::finance_system::tests::budget_authorization_stales_when_an_active_manager_changes_supervisor` verifies active-mandate intramural restructuring and stale held finance authorization, while the original cross-organization active-mandate release guard remains in the world owner. Other owners' tests cover multi-district territorial, espionage and victory scenarios with capable managers rather than constructing implausible unskilled one-person empires. Legitimate owner withdrawals retain their capital-floor and ledger invariants without any fixed per-person dollar allowance.

The columns are not a required sequence. Use focused feedback while iterating or
isolating a failure; go directly to the completion lane once it compiles and
exercises the same owner coverage. Never rerun the broad gate after a passing
fast lane "for reassurance".

## Rust agent diagnostics

Optional Rust diagnostics may be used before selecting or strengthening the normal proof:

[`scripts/rust-diagnostics.ps1`](scripts/rust-diagnostics.ps1) (or its `.cmd` wrapper) is the
preferred entry point for repeated agent use because it pins the bounded project defaults below
and gives each mutation execution a unique ignored output directory. The underlying Cargo
commands remain useful when a task needs a flag the wrapper intentionally does not generalize.

- `cargo modules structure --lib --no-fns --no-traits --no-types --max-depth 4` is a bounded ownership map when the cockpit/architecture route still leaves the responsible module unclear. Prefer a crate-qualified focus such as `--focus-on crimocracy::<module>`. For unlinked-file checks use `cargo modules orphans --lib --cfg-test`; this repository keeps many tests in split `tests.rs` modules, so omitting `--cfg-test` produces false orphan reports. Do not gate on global cycle or orphan counts.
- `cargo mutants --list --file <owner.rs>` is the cheap first step when a consequential state transition, invariant, or persistence test may not actually distinguish wrong behavior. Execute only a narrow file/function/line selection when the result can change the test design. Prefer passing the owning test substring after `--` when one exists; that makes hangs and misses attributable to the contract under review instead of unrelated suites. The wrapper enforces a narrow regex, defaults to two jobs, and creates a unique `target/agent-output/mutants/<task>/mutants.out`; checked-in [`.cargo/mutants.toml`](.cargo/mutants.toml) keeps Cargo locked. Investigate missed or timed-out mutants individually. If a missed mutation is unreachable for every valid `AppState` because canonical construction and release-safe invariants already make the altered branch equivalent, record that reasoning instead of manufacturing invalid private state solely to kill it. Mutation score is not a completion target.
- `cargo expand --lib <module::item>` is an inspection aid for a derive/proc macro whose generated code is material to the task. The wrapper bounds expansion output to 200 lines by default and accepts `-Filter <regex> -Context N -MaxLines N` for a relevant generated region. If selecting a type shows only its declaration, expand the containing module and filter for the type or generated impl there. Expanded text is debugging evidence, not source or a compilation contract; call raw `cargo expand` only when an intentional full dump is actually needed.

Use these at the point of uncertainty, not as a ritual preflight: `Modules` when ownership remains
ambiguous after the architecture route, `MutantsList`/`Mutants` when a consequential focused test
may not distinguish nearby wrong behavior, and `Expand` when generated code is itself material to
the edit. A useful diagnostic changes where you read, what you test, or what you implement.

None of these replaces `.\scripts\verify.cmd` or a focused behavioral proof. Mutation execution stays in its isolated copy; do not use `--in-place` on a dirty/concurrent repository to bypass scratch-copy problems, and do not reuse another run's output directory.

Territory owner tests cover symmetric standstills, genuine balanced-ledger
street-cash tribute and insufficient funds, third-party clashes despite an
unrelated bilateral peace, independent postwar accords for disjoint gangs in
one district, and persisted history validation after those outcomes. They also cover
a failed assault that shuts down
the attacking racket, a strong autonomous rival displacing a player holding,
an actual crisis report, multi-day cooldown and suspension expiry, and
deterministic save/restore. A post-clash agreement must remain negotiable after
the loser is disabled, and the treaty must not forbid reopening a preexisting
racket when the seven-day clash penalty ends. The generated text-play tests
`generated_rival_clash_then_postwar_negotiation_uses_actual_player_orders`
and `generated_rival_turf_agreement_requires_sighting_and_blocks_both_types_of_escalation`
exercise player-only discovery gates, preview immutability, recruitment of
local force, direct conflict, treaty acceptance and signed-treaty restrictions
through the actual production operations.
The generated treaty test pre-authorizes a delayed hostile sabotage,
signs a standstill before it becomes due, and advances through the canonical
minute/day tick to prove the sabotage aborts and NPC expansion does not
plan a prohibited racket.
The tests `autonomous_gangs_can_negotiate_a_real_postwar_standstill`
and `extra_defenders_can_repulse_an_optimistic_rival_ai_and_deter_repeat_raids`
verify NPC-led reconciliation, defensive surprises caused by extra hires,
and tactical memory after a failed autonomous challenge.

## Adversarial pressure checks

`operations::operation_system::autonomous::tests` exercises mandate and police-fear constraints, a real rival reconnaissance that acquires usable knowledge, deterministic continuation through save/restore, then a canonical strike and disrupted player-owned business settlement. `detained_enterprise_manager_suspends_cycles_until_owner_resumes` verifies custody invalidates previously planned cycles and closes the detainee's enterprise until lawful recovery; release alone does not settle missed cycles. The rival pressure test compares matched business RNG branches to prove a successful strike improves the attacker’s own realized earnings while harming the target. Held cycle tokens reject stale local market capacity without mutation. `autonomous_prosecution_*` tests cover bounded intake, defendant-specific supplements, own-member notifications, release with live review, screening, and save/restore continuation. `vice_inquiry_observes_distinct_operating_cycles_then_detains_and_refers_manager` follows weak initial exposure through staffed observations of separate actual cycles, independent custody, racket suspension and prosecutorial intake; matches save/restore continuation and a branch that suspends the racket after the first observation, and rejects duplicate evidence. `foreign_case_raises_discovery_risk_but_not_own_racket_cost` distinguishes ambient discovery probability from attributable operating surcharges using the same authored exposure threshold and canonical originated-case intake. `business_acquisition::tests::strategic_investment` proves expansion to a previously unowned district, voluntary purchases when public listings end, player property protection, preservation of the seller's earning base, held-offer rejection when a replacement disappears, and reinvestment of actual sale proceeds without a prior foothold. `court_hearing_distinguishes_conviction_from_acquittal_and_collects_only_personal_funds` compares a screened case with and without actual retained counsel, verifies adjudication after preparation and release, checks personal debt against actual ledger collection and excludes concealed and employer funds, and verifies deterministic save/load. `later_review_cannot_impose_second_fine_from_already_adjudicated_evidence` verifies old evidence cannot support another judicial disposition and leaves the first sentence intact. `last_player_member_defection_ends_campaign_after_restore` proves that accepted rival recruitment of the final player member produces terminal campaign state across save/restore and rejects subsequent ticks without mutation. A player-owned public political machine now creates another valid rival sabotage motive even where the attacker has no shared retail sector, and the tests compare the quiet and threatened branches with identical saved state. Political rival tests verify that spending stops when sufficiently defended, preserves a real week of payroll, purchases affordable partial influence increments, and focuses actual challengers rather than draining clean capital in unrelated districts. The generated-player brief test verifies the public prominence warning without changing state. These proofs establish concrete pressure mechanisms, not late-game balance.

`decisions::decision_system::protection::tests` creates truly operating player-owned shops in a generated rival ward, advances the canonical daily tick to a specific gang's protection demand, rejects unaffordable approval with no mutation, funds and approves through the general balanced ledger, verifies exact payer loss and gang receipt, proves expiration and renewal after 30 days, and confirms that unanswered requests do not provide indefinite free protection, rejects payment and confirms retaliation becomes available, and verifies that genuinely available enforcers deter demands before money moves. Separate regressions verify that finite named muscle is allocated across active wards rather than duplicated at every business, that large portfolios cannot buy token-price protection, and that a *previously authorized* rival scouting operation aborts at its actual production start under a newly pending protection demand, deterministically across save/restore. `enterprises::autonomous_expansion::tests::legitimate_only_new_districts_invite_actual_rival_territorial_expansion` begins with newly established commercial holdings in wards where the player has never operated a racket, proves rival planning prioritizes the opportunity and actually establishes a new racketing presence after a full canonical day, and checks the resulting protection decision and deterministic replay. `text_play::generation_tests::generated_turf_explains_own_commercial_exposure_without_exposing_rival_staff` verifies the player's read-only defense and storefront counts. Release-safe decision invariants verify real credited postings for every resolved paid contract, not an honor-system boolean.

## Test rules

Tests prove observable production behavior: calculations, transitions, transactions,
invariants, serialization, deterministic continuation, and failure paths.

- Exercise canonical system operations, not private helpers or test-only mutation shortcuts.
- Assert typed error variants and relevant fields, not rendered text.
- For atomic rejection, assert authoritative state is unchanged.
- Use explicit seeds and stable ordering. Do not hunt for a passing seed.
- Keep content-count, CRUD, or smokes only when they protect a real contract.
- Keep assertions non-brittle: pin counts only for authored bounds (brief entry caps,
  overflow arithmetic), derive expected display prose from the test's own production
  values (e.g. format the asserted cents through the production money formatter)
  instead of duplicating literals, and assert a coarse marker plus the typed record
  rather than a full sentence when the sentence is not the contract. Formatter
  output itself is pinned only in the formatter's own tests; consumers assert the
  typed signal and at most one coarse marker. Content values (prices, margins)
  are pinned only where the registry test is the deliberate content contract;
  behavior tests derive from the registry instead of duplicating its numbers.
- Persistence tests distinguish authoritative bytes from derived runtime indexes: save bytes omit
  owner-maintained lookup/scheduling projections, and restore must rebuild them before indexed reads.

Ordinary tests live with their owning module under `#[cfg(test)]`, named after
behavior. Use `make_test_*` for local fixtures and the idempotent `*_for_test`
pattern when extending the shared production registry. Soak-class tests carry the
substring `soak`.
cannot silently drop deep coverage. These deeper tiers are evidence, not prerequisites for unrelated edits.

### Accretion checklist for a new test

- [ ] Calls the owner's `validate_* → commit` or `decide_* → apply_*`, not a private helper.
- [ ] Asserts the typed `Error` variant (e.g. `FinanceError::InsufficientFunds`) + fields on failure, not a string.
- [ ] On rejection, clones `state` before and `assert_eq!(state, before)` after.
- [ ] Uses `AppState::new(explicit_seed)` and `BTreeMap`/`BTreeSet` ordering — no `HashMap` iteration.
- [ ] Named `fn <behavior>_when_<condition>()`, not `fn test_crud()`.

## Local verification — solo, in this repo, no hosted CI

### Fast lanes — inner loop

| Need | Command | What it proves |
|---|---|---|
| Docs/routes only | `.\scripts\check-docs.cmd` | authorities, links, routes, aliases, published versions; no Rust build |
| Type-check lib | `cargo check-fast` | `src/` compiles |
| Type-check all | `cargo check-all` | lib + text play client compile |
| Type-check client | `cargo check-harness` | text play example compiles |
| Lib tests (no soak) | `cargo test-fast` | all library tests except soak-class tests |
| One test / module | `cargo test-focused <filter>` | owning module's `#[cfg(test)]` |
| Client contracts | `cargo test-harness` | text play command and session tests |
| One client behavior | `cargo test-harness <filter>` | matching client tests only (check the pass count; a typo matches zero) |
| Auto-rerun on save | `.\scripts\watch.cmd` (`-Filter`, `-Harness`, or both) | lib check by default; lib focused or client focused on demand |
| Check lane | `.\scripts\verify.cmd -Check` | fmt (changed files) + lib type-check |
| Fast lane (fmt + lib) | `.\scripts\verify.cmd -Fast` | library completion gate; fmt checks changed files only |
| Text play lane | `.\scripts\verify.cmd -Harness` | fmt + text-play client command contracts |
| Filtered fast lane | `.\scripts\verify.cmd -Fast -Filter <pat>` | fail-closed focused lib tests (soak excluded unless the filter contains `soak`) + fmt (changed) |
| Filtered text play lane | `.\scripts\verify.cmd -Harness -Filter <pat>` | fail-closed focused client tests + fmt (changed) |
| Soak only | `cargo soak` | mixed-state invariant stress |

`cargo check-fast` is the absolute fastest; `cargo test-focused` is the inner loop
for library behavior and `cargo test-harness <filter>` is the inner loop for the
text play client. A single-owner change can complete through the filtered fast
lane without rerunning every library test. A client change can complete through
the filtered text play lane without rerunning the whole client suite. The broad
gate is reserved for contracts that actually cross persistence, invariant,
domain, or verification boundaries. Real player-experience judgement comes from
playing a session, not from a gate.

Build-profile tuning and measured compile-cost observations live with Cargo configuration.
This document owns behavioral proof selection, not machine-specific timing claims.

### Broad completion gate — when cheap lanes are not enough

```text
.\scripts\verify.cmd
.\scripts\verify.cmd -Jobs 2
.\scripts\verify.cmd -Check          # type-check only, fastest gate
.\scripts\verify.cmd -Fast -Filter payroll   # one module
```

Fail-fast stages, in order (see [`scripts/verify.ps1`](scripts/verify.ps1)):

1. Compile-free documentation/route/alias/version contracts (`scripts/check-docs.ps1`; skipped when no doc/version files changed)
2. fmt on changed `.rs` files (full scan only for a clean checkpoint tree or unreadable git state)
3. `cargo test --locked --lib --quiet -- --skip soak` (soak stays explicit via `cargo soak`; filtered lanes also exclude soak unless the filter contains `soak`)
4. Text play client command contracts (`cargo test --example gameplay_harness`; skipped when no `src/`/client files changed)
5. `cargo clippy --locked --lib --example gameplay_harness -- -D warnings`

Skips are fail-closed and printed as `SKIP` lines with reasons: unreadable git
state or a clean checkpoint tree runs every stage.

[`scripts/verify.ps1`](scripts/verify.ps1) owns the broad gate; [`scripts/verify.cmd`](scripts/verify.cmd) wraps it. The broad gate proves the client still drives canonical operations instead of rerunning its whole command suite: client contracts stay in the text play lane so persistence or cross-domain checkpoints do not rebuild and rerun the example test binary. `scripts/check-docs.ps1` replaces the former Rust integration test for authority/link/route/alias/version checks, so documentation-only completion does not compile the crate.

**When to run what:**

- One-owner behavior work may complete with `.\scripts\verify.cmd -Fast -Filter <filter>`; broader library work with `.\scripts\verify.cmd -Fast`; text play client work with `.\scripts\verify.cmd -Harness` (or `-Harness -Filter <pat>` for one behavior).
- Run the broad gate only when persistence, invariants, cross-domain behavior, verification infrastructure, or another changed contract requires its wider client/Clippy coverage, or for an explicit broad checkpoint. It intentionally stops at the client's command contracts; real play observation is specialized evidence, not a tax on unrelated changes. Never rerun it after a passing narrower lane merely for reassurance.
- Run `cargo soak` only when the changed contract requires that evidence.
- When optimized compilation could change behavior, also run `cargo test-release`.

Gate flags: `-Check` (fmt on changed files + lib type-check) | `-Fast` (library tests, soak excluded; fmt on changed files) | `-Harness` (client command contracts; with `-Filter <pat>` runs fail-closed focused client tests) | `-Filter <pat>` (fail-closed focused tests; implies `-Fast` unless `-Harness` is given) | `-Jobs N` (cap parallelism) | `-NoClippy` / `-NoFmt` (skip known-passing) | `-Verbose` / `-Detail` (show captured output on success).

Documentation contracts are intentionally outside Rust's integration-test graph. The checker reads source declarations directly, so Markdown and command-surface changes have a complete compile-free path and the broad gate no longer links a second test binary merely to validate documentation.

## Text play — primary player-experience surface

Use `cargo text-play` to play through decisions and consequences without an
automated evaluation policy selecting the player's moves. This is a thin
terminal adapter under `examples/gameplay_harness/text_play/`; it can start
from `city_generation::generate_city` or the existing authored `scenario.rs`
fixture, but never runs an automated player strategy or reconstructs either
world during save restoration. Its
`mod.rs` owns session creation, current-version saves, options, and the
player-scoped read-only selection rules; `commands.rs` dispatches interactive
player choices to focused command families. `business_play.rs`,
`racket_play.rs`, and `treasury_play.rs` adapt commercial, enterprise, and
financial commands respectively. `governance.rs` delegates policy/mandate
mutation to the production delegation owner, and `planning.rs` composes
registry-directed operation drafts. `contact_play.rs` owns the player contact interface and
`legal_play.rs` direct-counsel commands; both use the existing canonical
contact and legal systems. `tests.rs`, `governance_tests.rs`, `openings_tests.rs`,
`legal_planning_tests.rs`, and `contact_legal_tests.rs` prove real tick
continuation, legal ledger payments, saving, information boundaries,
authorization, and rejection integrity.
`run_tick` remains the only way to advance simulation time.

Views show public business-directory identities (not ownership), organization-held
intelligence, its own finances, people, businesses and enterprises, delivered
reports, and pending decisions. Public business names do not themselves become
intelligence. The CLI must not read hidden legal cases or rival records to
recommend actions. Role assignments come from current organization members;
no participant/authority IDs from the starting fixture are retained. Entity
selection checks organizational ownership before producing actionable plans.
Opening accounts, same-kind treasury allocation, one-way accounted-to-street
capitalization through `finance_system::validate_capitalize_street_cash`, laundering,
recruitment, business acquisition, enterprise establishment and lifecycle, burglary and
reconnaissance, discovery, disposition, and decision resolution all use canonical
owner paths. Generic money movement is not a laundering or minting shortcut.
The `--opening` option selects one of six deterministic authored alternatives;
`--generate [--city-size small|medium|large] [--seed N]` constructs a fresh
seeded world and does not accept authored opening or policy seed overrides.
`--load` excludes generation/seed/opening overrides and restores its saved campaign.
`operation-kinds` derives role, approach, duration and relevant intelligence
topics from the registry. `plan` uses the current player roster, rejects
automatic assignment of untrained specialists, globally matches the required
roles to distinct trained people for maximum total skill and a modest
supervised-member preference, and accepts explicit crew,
leader, police-response, start-delay, completion-deadline, and required-topic
options through canonical authorization. Foreign business actions require
organization-held target information. Surveillance/infiltration can begin at
public venues and neighborhoods, but non-public organizations, operations,
investigations, people and enterprises must already be identified by sponsor
contacts, recruitment prospects, ownership, or organization-held information.
No typed ID creates knowledge of a hidden record. Witness pressure and
extraction still require actual episode-bound learned legal-person status;
ordinary person sightings or authority-level case-activity sightings are
not equivalent to witness registration or custody evidence.
`legal::investigation_system::tests::unrelated_incident_with_existing_shelf_witness_opens_separate_case`
proves that two separate operation incidents implicating the same suspect and
eyewitness do not reuse an incompatible suspended case; the existing same-origin
registration tests still reject duplicate testimony without mutation. The
read-only intake selection is checked through canonical commit, full invariant
validation and save/restore. Long text-play continuation from the generated
seed 32885517 day-252 campaign also exercises the previously failing autonomous
operation minute without a client-side special case.

`reconnaissance_tests.rs` covers all selectable target forms, foreign
institution/case boundaries, full district patrol observations, recruitment
prospect surveillance, watchable own operation/enterprise records, and a real
police sightline after a criminal incident. `planning_options_tests.rs`
proves bounded schedule arithmetic, deadline and pre-entry contingency gates,
same-state rejection, persistence, and the flow from surveillance-learned
personnel information into a genuine required-intelligence infiltration. The production surveillance integration tests distinguish successful infiltration's internal records from ordinary watches and check the persisted after-action reconstruction, including ownership identity and the institutional privacy boundary.
`cancellation_tests.rs` proves player-only cancellation of scheduled and
in-progress operations via the production abort owner, durable debriefs,
crew-booking release, and restored state equivalence.

`debrief_play.rs` offers player orders for transferring a member's
personally held information to the organization, one source per command. The
`debriefs` query reveals only counts of unreported observations for current
staff and excludes detained members. The `debrief` action delegates to
`intelligence_system::validate_information_transfer`, which ensures
same-organization membership, source provenance, typed findings, historical
observation time, and duplicate-lineage exclusion. `intel` displays full
details only for information already held by the player organization. It
does not bypass a source holder's personal knowledge. `debrief_tests.rs`
uses real police-response observations as well as a canonical direct
observation to prove no premature disclosure, source retention, idempotence,
custody rejection, and exact persistence. `legal_disclosure_tests.rs`
proves that an actual police witness record is privately held by the detective
until the modeled institutional contact discloses it, at which point the
player can authorize a witness-pressure plan from the typed episode signal.
`staff_matching_tests.rs` uses overlapping capability matrices to prove
completeness, maximum score, stable ties, manual-role separation, and no
fabricated trained crew. The matcher has at most nine roles and evaluates
512 assignment masks per current member.

`text_play::tests` verifies that owner-distribution previews round overhead per
storefront, that moving wages into an organization-owned operating account does
not reduce reported payable payroll runway, and that the brief survives save/load.
The generated-session regression checks the manual-distribution warning when
storefront earnings are retained during a treasury shortfall, its persistence,
and its removal when daily distribution is explicitly enabled. The rival campaign
reserve regression tests that organization-owned legitimate operating balances
protect actual payroll while making existing accounted funds available for civic
spending, including through save/restore.

Leadership pauses for pending decisions, enabled urgent-report attention, and
terminal outcomes of its own operations, plus player-member arrests and releases
from the actual tick outcome. The world payroll outcome also identifies the exact
members whose missed pay caused them to walk away: normal play pauses on an
enabled exception, and uninterrupted review counts only departures associated
with the player's payroll. The leadership brief derives gross full-day wage
runway from the same spendable organization-owned liquid accounts as payroll,
including organization operating pockets but never protected business-owned tills. `time_play` includes a live five-day unfunded-crew
regression for departure detection, original-organization scoping, and save
validation; `political_play_tests`, `civic_play_tests` and `economy_play_tests`
cover visible campaign erosion, real crew achievements and authored racket
inquiry risk, respectively. This prevents long advances from skipping the
manual-counsel window before one-time informant decisions. Contact candidates
are drawn from identified people with real relationships to current,
non-detained staff; unrevealed contact information stays private until canonical
disclosure. Legal defense targets known custody or learned detention episodes
of current members. A retainer debits sponsor liquidity and credits the provider's
real operating account, with active-representation and contact lifecycle
restrictions enforced by their owners. Automatic legal-support policy acts through
the production legal phase on the arrest minute when contact and funds exist.
The independently employed lawyer in the authored opening does not grant free
representation. Production saves retain these relationships, cases, and payments
and reconstruct their indexes on restoration.

The economy client also exposes manual storefront suspension/resumption and
district-wide enterprises backed by zero or more actual owned support businesses.
A racket with required host functions cannot be started at a district-only
location; all account, delegation, infrastructure and settlement checks remain
canonical. Suspended business books keep their laundering-window history, and
active hosted or supporting rackets lock their own storefront dependencies.
`economy_play_tests.rs` covers multi-support establishments, duplicate
rejection, genuine daily settlement alongside business-hosted rackets, lifecycle
gating, and persistence.

`finance_play.rs` presents a bounded, newest-first journal selected by
the finance owner's per-account chronology indexes. Transfers touching multiple
player accounts appear once, and a guessed rival transaction ID is not enough
to display rival books. The detail view prints only player-owned postings.
`finance_play_tests.rs` proves ownership isolation, same-minute ordering,
non-mutating rejection, real laundering fees, and save/restore stability.
`treasury_capitalization_tests.rs` proves a player's account-scoped conversion,
rejection of wrong-kind, absent, and unfunded accounts without mutation,
actual balance movement, complete ledger visibility, and restored brief parity.
`finance_system/capitalization.rs` additionally tests a stale token, an
existing negative street account, and byte-identical save/restore continuation.
`business_acquisition/tests/strategic_investment.rs` proves mature sellers
can accept fully funded payroll-runway offers with no available replacement,
and recently acquired storefronts still reject ordinary reference bids while
accepting an exceptional 180-day-income buyout. The tests preserve the original
90-day investment protection, seller wage-base constraints, private refusal,
and title/ledger state through save and restoration.
`economy_history.rs` lists historical business and enterprise cycles from
their existing indexed queries, restricted to current player assets and to
business cycles actually earned during player ownership.

Population-scaling and freelance workforce proofs live in `world::population_tests`: small/medium/large/24-ward cities exceed 200/400/800/1,100 persisted named people, respectively, with distinct full names, canonical personal work sites and real institutional assignment, public acquisition listings, deterministic construction and full save/restore. Independent freelance criminals require an actually known crew contact, authentic cash-funded signing payments, a validated temporary organization assignment, and minute-exact indexed canonical release after the last earned payroll. Regressions cover typed, unchanged-state rejection of permanent supervisory roles, mandates, institutional channels and out-of-term bookings, persistence of the contract-derived operation deadline, an active contract's protected vocation, career changes after a completed term, preserved signed history, and deterministic release across save/restore; expired contractors can still be recruited with an ordinary relationship-backed attempt. The text client in `text_play::civic_play_tests` exercises the census, private freelancer discovery, unchanged-state rejection, fixed-term signed employment and sponsor-filtered, player-visible expiry outcomes during normal or uninterrupted advancement. Complete this cross-domain world/social/finance/persistence change through the full verification gate and inspect a real generated text session.

Individual civic-character verification lives in `world::civic_tests` and `text_play::civic_play_tests`: generated private proprietors and public officials retain actual IDs and roles; actual paid personal shop dividends are balanced against retained operating capital; civilian sellers refuse or accept by their own reservation price; ownership transfers change the derived occupation; daily work, identity-staggered weekly rest with continuously staffed police shifts, fatigue, standing, institutional duties, detention and save/restored continuation remain valid through the normal world, finance and economy owners. Client dossiers expose only public role, registered titles and redacted work history. The `city_generation` and `campaign_victory` gates ensure registered private owners do not silently eliminate public listings or break the long-term civic property coalition. The broad completion gate is required because character layout, finance, legal case staffing and elections all changed.

Political-campaign tests live in `politics::tests` and `politics::integration_tests`: real mult-account expenditure with an external municipal recipient, refusal without cash, skill-sensitive paid specialist growth and interrupted salaries, three-axis daily decay, venue/contact-locked civic initiatives, seven-day cooldowns, real council-sponsored nomination, a scheduled losing ballot, an independently qualified winning ballot, repeat candidacies, autonomous rival purchases, release-safe persisted ballot chronology, and save/reload continuation at the election boundary. `core::campaign_victory::tests` proves that mature commercial coalition thresholds no longer award political victory; a candidate must win an actual ward election, hold uncontested support for 30 days, pay substantial real campaign expenses, and retain clean reserve funds. `text_play::political_play_tests` proves the new commands against an actual generated small city, including correct arity, unchanged-state rejection, and read-only campaign/election/ward dossiers. Use the broad completion lane for changes to this cross-domain state and its reports. Electoral vote outcomes are authored by the production simulation, not test-only client policies.

Optional campaign-victory proofs in `core::campaign_victory::tests` exercise insufficient clean funds and unchanged state, accounted personal leader/crew payouts with exact money preservation, stale tokens, a live council contact plus acquired operating businesses across city districts, at least two established profitable commercial kinds per political district, independent active Political/Neighborhood mandates rather than one manager reused across districts, loss and restoration of delegate autonomy, uncontested district influence and relative racket strength, and terminal behavior across save/reload. `cargo test-focused campaign_victory` is the narrow lane. Text-play `victory` and `political district-N` are read-only projections. The district dossier derives title age and the ceil-rounded days remaining, recent actual profitable cycles, and title-version-pinned settlement evidence through the same eligibility helpers as the victory owner; the view does not mutate the save or expose rivals. `claim-victory` is the only ending action and must reject ineligible claims without mutation. The mayoral election's validated final report identifies the elected player leader and includes an organizational summary, its real council sponsor, and one sourced-from-state organizer and locally qualified district entry each. Report invariants admit both existing single-entry campaign reports and bounded multi-entry political reports. A victorious saved state must refuse subsequent ticks with no clock or record mutation.

Start with `cargo text-play --generate --seed 12 --city-size small`
or `cargo text-play --seed 12` for reproducibility and use `help` for
the available actions. `advance` is the sole time-advance command and accepts
up to 30 days per call. By default it stops at player attention events or the
finite clock, reporting actual minutes advanced instead of rejecting after
partial progress. Explicit `advance 30d uninterrupted` suppresses optional
attention, custody and operation-completion interruptions but still runs every
canonical minute, halts for pending leadership decisions or terminal states,
and summarizes deferred events. Exact-save comparison against 180 individual
production ticks proves the longer interactive command cannot skip simulation.
Operation plan delays and deadlines independently retain their seven-day cap. Enterprise succession tests exercise active and suspended transfers, stale tokens, mandate indexes, historical settlement attribution and exact save restoration. The text client exercises reassignment and preserves the same settlement account and cycle IDs. The management-concentration test checks read-only warnings before investment, after establishment, and after suspension. The leadership brief distinguishes active and suspended rackets, excluding retired
history still visible in `enterprises`. Treasury tests cover explicit daily
distribution, retained capital, payroll funding, disabling the order, and
deterministic save/restore. Contact briefing tests exercise the canonical per-source
disclosure path and observation timestamps. Long advances interrupt fresh
own-business/racket suspensions and payroll exceptions; the client test proves
payroll interruption at the exact boundary, read-only coverage, and persistent
attention preferences. The police-response integration test resolves two complete
identifying operations and verifies one active file, independent evidence, actual
custody, and deterministic continuation from a save between incidents.
Script input is one command per line, with blank lines
and lines beginning `#` ignored. Interactive command rejection prints a
reason without ending the session; a rejected scripted command fails with its
line number. Opportunity lifecycle tests reject replayed expired or converted
openings and delayed old observations, then admit genuinely newer observations
after restoring a save. Excess or malformed command arguments are rejected before any
mutation. Procedural campaigns create real initial rosters, independent
businesses, district profiles and patrol schedules from the world seed;
authored scenario openings remain available to test fixed situations.
Current crew membership and skills determine available orders and recruitment
changes that roster.
The terminal exposes the production surveillance/infiltration target shapes,
but detailed investigation discovery and witness/detention status acquisition
still depend on information produced by the modeled world and contacts;
watching an authority can reveal active casework without revealing its private
case IDs. The city generator does not simulate transportation topology, traffic,
citizen schedules or moving street actors: the presentation road grid is not
authoritative travel state.

## Procedural city generation

`src/city_generation.rs` owns deterministic starting-world composition,
not a competing subsystem. Its independent SplitMix64 construction RNG
chooses unique district and person names, correlated wealth/commerce/demand/
police profiles, staffing, relationships, business types/functions and owners.
Every durable record is created through the same production insertion and
validated commit paths used in normal play. Starter police coverage and
patrol windows, rival organizational autonomy, operating business accounts,
player/rival enterprise accounts, and burglary leads are actually present and
participate in the canonical tick. Construction does not consume the four
independent persisted simulation RNG streams. Generated starting identities
are temporary return values, not a second save schema; the original campaign
envelope saves only authoritative state. Validated custom city budgets are
bounded to 4-24 districts and 8-40 businesses per district.

`src/city_generation/tests.rs` checks exact same-seed serialized state
and visual layout, changed-seed diversity, invariant-safe creation for
24 small-city seeds, all three scale envelopes and the maximum supported
24-district/960-business/8-rival configuration containing all 25 business
kinds, real minute ticks, business books, rival rackets, police coverage, and save/restore
equivalence. `text_play/generation_tests.rs` checks startup flags and
mutual exclusions, world and district identity after actual slot save/load,
player-safe target rejection and real district surveillance, and all CLI sizes.
The generated preview case in `examples/city_gui_preview.rs` checks the
public-dossier names against the generated world and ensures that hidden
ownership, legal or intelligence state is not serialized to the browser.

End-to-end inspection commands:

```powershell
cargo test --locked --lib city_generation
cargo test --locked --example gameplay_harness generation_tests
cargo test --locked --example city_gui_preview generated_campaign_export
cargo text-play --generate --seed 12 --city-size small
cargo run --locked --example city_gui_preview -- --generate --seed 12 --city-size medium
python scripts/generated_city_browser_smoke.py
```

The generated WebGL2 smoke checks the actual headless render, every nonzero
GPU business ID against the public directory, player-safe payload fields,
district navigation, property selection and whole-city overview. Its screenshot
is written under `target/agent-output/city_preview/`. Longer sessions
should use `cargo run --locked --release --example gameplay_harness -- --generate --seed 12`
where practical: debug `run_tick` performs an
expensive full invariant check after each of the canonical minute ticks.

Text play is the primary judge of comprehension, usability, and campaign
pacing. Do not reintroduce a scripted policy that selects the player's moves:
that duplicates the game's rules in the harness and drifts from the shipped
game. Player-behavior judgement comes from reading a real session and its
reports. Text-play command tests are included in the `cargo test-harness`
target. The time-advance contract in `text_play/time_play.rs` exercises the
first loss, continuing losses, and the first loss after a canonical racket
suspension/resumption and save restoration; the client must pause at the new
loss without treating pre-resumption history as part of the current streak.

## Completion checklist

Before handoff:

- [ ] Ran exactly the smallest scripted completion lane that covers the changed surface (see decision tree above); did not rerun an overlapping focused proof immediately before that lane.
- [ ] `git diff --check` is clean; generated output and final worktree reviewed.
- [ ] If a test, client command, alias, or verification rule changed, this document and the owning script/command definition were updated in the same change.

When a test, client command, alias, or verification rule changes, update this document and the owning script or command definition in the same change.
