# Testing

Owns test selection, harness evidence, and local verification. Ownership is in
[`ARCHITECTURE.md`](ARCHITECTURE.md); scope is in [`STATUS.md`](STATUS.md);
cockpit routing is in [`AGENTS.md`](AGENTS.md).

## Test selection — narrowest proof first

```
Which change did you make?
  │
  ├─ Syntax / type error?
  │   Fastest: cargo check-fast
  │            .\scripts\verify.cmd -Check   (includes fmt)
  │
  ├─ One library behavior (single module, single system)
  │   Focused: cargo test-focused <filter>
  │   Complete: .\scripts\verify.cmd -Fast
  │
  ├─ Library implementation (no harness surface touched)
  │   Focused: cargo check-fast  or  cargo test-focused <filter>
  │   Complete: .\scripts\verify.cmd -Fast
  │
  ├─ Harness surface (examples/gameplay_harness/*.rs)
  │   Focused: cargo harness-rush
  │   Complete: .\scripts\verify.cmd -Fast -Harness
  │
  └─ Persistence, invariants, cross-domain, or verification infra
      Focused: owning module's focused test (e.g. cargo test-focused finance) plus a save/restore round-trip
      Complete: .\scripts\verify.cmd
```

| Change | Focused feedback | Completion lane |
|---|---|---|
| Syntax / types | `cargo check-fast` | `.\scripts\verify.cmd -Check` |
| One library behavior | `cargo test-focused <filter>` | `.\scripts\verify.cmd -Fast` |
| Library implementation | `cargo check-fast` or focused test | `.\scripts\verify.cmd -Fast` |
| Harness filter | `cargo harness-rush` | `.\scripts\verify.cmd -Fast -Harness` |
| Persistence, invariants, or cross-domain | Focused owner test | `.\scripts\verify.cmd` (broad gate) |

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

## Test rules

Tests prove observable production behavior: calculations, transitions, transactions,
invariants, serialization, deterministic continuation, and failure paths.

- Exercise canonical system operations, not private helpers or test-only mutation shortcuts.
- Assert typed error variants and relevant fields, not rendered text.
- For atomic rejection, assert authoritative state is unchanged.
- Use explicit seeds and stable ordering. Do not hunt for a passing seed.
- Keep content-count, CRUD, or smokes only when they protect a real contract.
- Persistence tests distinguish authoritative bytes from derived runtime indexes: save bytes omit
  owner-maintained lookup/scheduling projections, and restore must rebuild them before indexed reads.

Ordinary tests live with their owning module under `#[cfg(test)]`, named after
behavior. Use `make_test_*` for local fixtures and the idempotent `*_for_test`
pattern when extending the shared production registry. Soak-class tests carry the
substring `soak` and are excluded from fast lanes with `--skip soak`, so renames
cannot silently un-exclude them; the invariant soak is stress evidence, not a
replacement for focused behavioral tests.

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
| Type-check lib | `cargo check-fast` | `src/` compiles |
| Type-check all | `cargo check-all` | lib + harness compile |
| Type-check harness | `cargo check-harness` | example adapter compiles |
| Lib tests (no soak) | `cargo test-fast` | all library tests except soak-class tests |
| One test / module | `cargo test-focused <filter>` | owning module's `#[cfg(test)]` |
| One domain | `cargo test-legal` / `test-finance` / `test-world` … | sugar over `test-focused <domain>` |
| Auto-rerun on save | `.\scripts\watch.cmd` (`-Filter`, `-Harness`, `-Check`) | reruns the selected local lane |
| Harness smoke, one strategy | `cargo harness-rush` / `-press` / `-recon` | one strategy branch |
| Full-mode batch | `cargo harness-full --samples 8` | all strategies, matched seeds, artifacts |
| Check lane | `.\scripts\verify.cmd -Check` | fmt + type-check |
| Fast lane (fmt + lib) | `.\scripts\verify.cmd -Fast` | iteration gate |
| Fast harness lane | `.\scripts\verify.cmd -Fast -Harness` | smoke contract only |
| Filtered fast lane | `.\scripts\verify.cmd -Fast -Filter <pat>` | focused tests + fmt |
| Soak only | `cargo soak` | mixed-state invariant stress |

`cargo check-fast` is the absolute fastest; `cargo test-focused` is the inner loop
for behavior; `.\scripts\verify.cmd -Fast` is the iteration gate. The full gate
is reserved for persistence/invariant/cross-domain work.

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

1. `cargo fmt --check`
2. `cargo test --locked --lib --tests --quiet`
3. Harness unit tests (`cargo test --locked --quiet --example gameplay_harness --lib`): the example's own options-parsing and financial-branch contract tests, which stage 2 never compiles
4. Exact ignored test `tests::smoke_mode_covers_canonical_paths` (selected fail-closed — `verify.ps1 -SelfTest` validates the count must be exactly 1)
5. Gameplay-harness full mode, one sample (`--mode full --samples 1`): narrative arcs, probes, and cross-branch contracts that smoke skips
6. `cargo clippy --locked --lib --example gameplay_harness -- -D warnings`

[`scripts/verify.ps1`](scripts/verify.ps1) owns the gate; [`scripts/verify.cmd`](scripts/verify.cmd) wraps it. The smoke stage requires exactly one selectable ignored test; `.\scripts\verify.ps1 -SelfTest` checks that selection. [`tests/documentation_contracts.rs`](tests/documentation_contracts.rs) protects the authority set, local links, concrete routes, Cargo aliases, and published schema/content revisions.

**When to run what:**

- Ordinary library work completes with `.\scripts\verify.cmd -Fast`; harness work with `.\scripts\verify.cmd -Fast -Harness`.
- Run the broad gate only when persistence, invariants, cross-domain behavior, verification infrastructure, or another changed contract requires its wider harness/Clippy coverage, or for an explicit broad checkpoint. Never rerun it after a passing fast lane merely for reassurance.
- Run `cargo soak` or `cargo harness-full --samples 8` only when the changed contract requires that evidence.
- When optimized compilation could change behavior, also run `cargo test-release`.

Gate flags: `-Check` (type-check only) | `-Fast` (skip soak/harness-full/clippy) | `-Harness` (smoke only, requires `-Fast`) | `-Filter <pat>` (one module, implies `-Fast`) | `-Jobs N` (cap parallelism) | `-NoClippy` / `-NoFmt` (skip known-passing) | `-Verbose` / `-Detail` (show cargo output on success). Build profiles are tuned for this crate; alternatives and machine-specific notes live in [`Cargo.toml`](Cargo.toml) and [`.cargo/config.toml`](.cargo/config.toml).

## Gameplay-harness evidence — bounded evaluation surface

[`examples/gameplay_harness/main.rs`](examples/gameplay_harness/main.rs) evaluates bounded deterministic policy treatments through production paths. It is an evaluation surface, not a human-play test: it proves systemic behavior, not interface quality or comprehension.

### Modes

| Mode | Command | Evidence |
|---|---|---|
| `smoke` (default) | `cargo harness` | Canonical strategies + legal-foundation chain; sessions observe the whole first campaign day so recruitment counters carry real rival-attempt evidence |
| focused smoke | `cargo harness-rush` / `-press` / `-recon` | One strategy branch only |
| `full` | `cargo harness-full --samples 8` | Narrative strategy arcs, probes, matched world/policy seed channels, scenario sensitivity, artifacts |

Commands: `cargo harness` runs smoke by default; `cargo harness-full --samples 8` runs explicit comparison; append `--artifact-dir target/my-run` to relocate artifacts. `cargo harness -- --mode smoke --strategy press` selects one branch.

### Information boundary

RUSH, PRESS, and RECON act on the same world-selected authored fixture and the same explicit policy-selected timeline. Acting policy may use only organization/player-visible information: persisted reports and outcomes plus surfaced decision requests.

- Operation after-actions contain only crew-observed operational/exposure facts; they never reveal that institutional intake opened a case or which authority owns it.
- PRESS learns initial case activity only by querying its standing police contact through the canonical disclosure path, after visible exposure gives leadership a reason to ask.
- Hidden investigation, evidence, rival-report, and world-audit state stays in explicitly labeled diagnostic artifact fields and contract metrics; it never feeds decisions.
- Missing acting information or a canonical rejection fails the run; missing events are observed absence.
- `FullNarrative` and `FullQuiet` execute the same full policy arc and must produce identical `RunMetrics`; presentation cannot change gameplay.

`--world-seed` controls fixture variation and the simulation RNG streams. `--policy-seed` independently controls evaluation-owned timing and bounded policy choices. `--samples N` (range 1..=64) is the built-in scenario-sensitivity sweep: it varies only the world/simulation seed while holding the policy seed fixed. Matched branches share both seed channels, fixture, and policy timeline. Policy sensitivity is tested by holding `--world-seed` fixed and varying `--policy-seed`; the two axes must never be conflated. Per-run events and `RunMetrics` are raw evidence beneath aggregates; aggregates are not quality scores.

- `full` mode writes batch per-run JSON to `--artifact-dir` (default `target/harness-runs/`), all rotated full-session runs to its `narrative/` subdirectory (never overwriting same-seed batches), plus `summary-w<world>-p<policy>.json`. Each run artifact records both seed channels and separates `player_visible` evidence from `diagnostic` hidden-state evidence.
- Artifact write errors fail the run rather than silently dropping evidence; contract coverage verifies rotation retention, batch isolation, and error propagation. Financial artifacts retain every term of the accounted-funds reconciliation.
- Structural validation runs at setup and observation boundaries, not every tick.

### Organic variation — not one replayed story

The harness must not replay one exact story, but variation axes remain attributable:

- Full mode rotates its narrative comparison across `NARRATIVE_SEED_ROTATION` adjacent world seeds while holding policy seed fixed, covering every authored fixture variation (economy profile, police presence, patrol windows, target names, till kind). Every set validates deterministic contracts plus any stochastic consequence that actually occurs, but player follow-up is required only when player-visible evidence crossed the acting-policy boundary: a hidden trace-level institutional case remains diagnostic evidence and must not manufacture a contact query or counter-surveillance action. Stochastic reachability contracts are evaluated across sufficiently broad world-seed batches rather than forced in every sample. The deep metrics and experience readout run on the primary world seed, while other sets print compact summaries.
- Authored-content-derived timing: scenario anchors come from authored operation durations, recruitment cadence, and the cold-case window; the terminal-wait guard's slack equals the longest authored operation duration, so it tracks content instead of a constant.
- Policy-seed-derived variation inside fixed branch identities: the witness-pressure delay used only when no actionable patrol pattern is known, and which rival the defector watch visits first. The win-back pitch is not policy-seed variation: after surveillance confirms the defector's destination, leadership makes one PersonalAppeal grounded in the already-authored boss-member relationship. The harness does not inspect latent candidate drives or traits to choose that pitch; production recruitment scoring still uses the candidate's private willingness factors to determine whether the appeal succeeds.
- World-seed-authored fixture axes stay live: the racket till is street cash on even world seeds and concealed cash on odd world seeds, and the PRESS arc adapts to what its books actually hold (see the wealth-gate contract).

### Contracts — changing any requires updating this section + the harness tests

`Contract` below means a pinned forward-facing assertion over player-visible behavior.

Narrative arcs (one shared fixture per comparison; each session closes with an organization view built from player-visible state):

- **Failure teaches (RUSH)** — when police actually arrive before entry, the standing abort contingency must fire, leave debrief-derived district PoliceActivity knowledge, and carry that knowledge into the second-score plan. Because the defector arc ends with a canonical win-back that restores the original burglar, RUSH must not hire a redundant replacement afterwards; the win-back-first-then-replacement ordering (replacement only while the defector stays away) is contract, with either whole crew permitted to work the rebuilt plan. Covered batches require this authored path to be reachable across the sampled fixture variations; an individual seed with no pre-entry arrival is observed absence, not a fabricated contract failure.
- **Consequence arc (PRESS)** — witnessed/identifying crew exposure prompts a standing police-contact query; only a canonical contact disclosure can establish that the precinct has an active case, and that player-held case fact then seeds a precinct heat-check surveillance. Standing down stops new street jobs, not the home racket: the narrative explicitly retains its earnings and vice risk. Governance continues through legitimate front-profit withdrawals and, for street tills, surplus laundering: a two-district mandate revision, float capitalization through a canonical ledger transfer, and a second-district enterprise when the books permit. Concealed reserves cannot be laundered directly but do not block a purchase funded by legitimate earnings, with daily contact polling until the channel itself carries the shelved read either way. A shelved read resolves the named burglary file only; it is never narrated as a district-wide all-clear while racket surcharges or manager vice warnings remain live. Investigator-held case knowledge is typed to the originated operation/enterprise, so polling the burglary case cannot accidentally consume a newer vice-inquiry status from the same precinct; the contact's knowledge is production state (`legal::case_knowledge`), never fixture-authored.
- **Casing discipline (RECON)** — opening and second-score scouts share one player-visible assessment: no observed exposure or an explicit shelved contact read permits further planning; active or inconclusive reads and aborted scouts cause stand-down without fabricating a burglary. Opening stand-down remains distinct from a failed or aborted burglary in metrics/artifacts and carries through the reopened score. Scout aborts have no resolution to unwrap. Contracts and readouts accept uncertainty-driven restraint rather than demanding that hidden intake opened a case.
- **Own-heat loop (RECON)** — the second scout reuses the newest organization-held typed patrol observation for its district, with the policy clock as an earliest readiness time and a 60-minute uncertainty buffer around patrol intervals. The selected patrol report is attached to the scout's canonical plan as well as used to select its schedule; contract and artifact evidence retain both attachment and production-scored topic coverage. The narrative and artifact retain the observation age and actual canonical scout schedule; a pinned contract protects the primary seed's clean second score rather than forcing exposure for coverage. Known windows protect looking as well as taking, without promising safety. The trigger for a case query is player-visible crew exposure, not hidden intake: when fresh casing resolves with any observed exposure, the branch checks that operation's case through its standing institutional contact before authorizing another burglary. A confirmed active case or an inconclusive channel read makes RECON stand down and let the reopened score expire; only an explicit shelved read (or a casing run with no observed exposure) permits the patrol-safe burglary. A session whose casing reported no exposure must not fabricate a read.
- **Witness chain and counter-play** — the target is character-owned, so witnessed/identifying exposure names the owner as on-scene witness through canonical intake; institutional interviews can convert his account into testimony; PRESS answers with exactly one WitnessPressure operation. When organization-held typed patrol information exists it is binding and must yield a safe pre-interview window; if it does not, the treatment fails rather than discarding known risk. Only when no actionable patrol pattern is known may a bounded policy-seed delay be used, disclosed as treatment timing rather than invented patrol knowledge. Two honest terminal shapes: registered cooperation degrades, or a police response forces a disciplined abort that leaves no second case. Failed pressure without degradation fails the run. Identifying exposures can escalate to autonomous member arrests (`player_member_arrests`); acting policy never reads case internals.
- **Personnel loop both ways** — after a departure, the player-facing personnel report reveals only that the member left; canonical surveillance must then confirm where the defector landed before leadership makes exactly one executive win-back resolved by production recruitment scoring. On refusal, the rival's production loyalty report naming our recruiter remains diagnostic/contract evidence and is not narrated back to the player without a channel that reveals it. Sessions without a departure attempt nothing.
- **Recovery governance** — successful win-back is followed by an explicit canonical reassignment to the returned member's trusted original lieutenant. Recruitment itself still assigns the recruiter as supervisor; recovery does not silently grant loyalty, immunity, or stronger relationships. Aborted defector watches are inconclusive and do not panic or authorize a win-back without player-held typed personnel confirmation.
- **Second wind** — all branches see the reopened second-score opportunity; RUSH works it with the whole crew (win-back-restored, or a replacement only while the defector stays away), PRESS lets it lapse as the price of standing down, and RECON re-recons before deciding whether to work it or abandon it because the casing itself reported exposure the contact could not affirmatively clear. When opening casing already caused stand-down, RECON lets the reopened score expire without another scout.
- **Standing feedback** — witnessed operations and successful non-surveillance operations surface Notable `Standing` reports; surveillance rewards intelligence, not public competence, but retains exposure consequences. A racket drawing a racket inquiry raises police fear the same way. Closing qualitative bands describe distance from the authored baseline without exaggerating small shifts; boundary and canonical-delta tests cover the readout.
- **Routine continuity** — legitimate-front economics continue identically while leadership handles exceptions.

Probes:

- **Street-work leverage** — each full narrative seed runs a no-street-job baseline from the same fixture to the exact shared financial boundary. It leaves the initial opportunity unworked while normal trade, rackets, rivals, and wages continue. `leverage-w<world>-p<policy>.json` preserves own-book net, realized and held property separately, paid/unpaid wages, district surcharges, actual execution crew-minutes, departures, and surfaced decisions for all four treatments. Earned flow excludes opening capital and does not double-count internal laundry fees or owner withdrawals; it is not treasury liquidity or a universal strategy score. Longer PRESS recovery/acquisition totals never enter this comparison.
- **Personnel retention** — a matched continuation clones the exact post-win-back RUSH state and compares leaving the member under his recruiter with explicitly restoring his trusted lieutenant. Both arms stop new street work, pay ordinary wages, and run two authored recruitment cooldowns plus one day; no rival timing or hidden scoring selects actions. Full mode writes `retention-w<world>-p<policy>.json` with the shared window, own-roster outcomes, payroll and timestamped player report provenance. The primary-world contract requires a renewed departure under weak supervision versus two reported refusals under the trusted reporting line; missing initial recovery on other worlds is observed absence. The comparison demonstrates bounded retention, not permanent loyalty.
- **Operating posture** — matched branches play the same initial PRESS burglary, then react to the first organization-held home-racket surcharge report. The shared trigger settlement is excluded from an eight-day keep-open versus suspended comparison of home-racket earnings, paid surcharges, manager-observed vice warnings, front trade, and paid/unpaid wages. Trigger search is bounded to four days after the first due cycle and tolerates custody-delayed settlements; absence is explicitly reported without injecting cases. The primary world must reach the trigger. Canonical resumption occurs outside the comparison and schedules a full new cycle without backlog; its surcharge is retained rather than treating reopening as a clean district. `posture-w<world>-p<policy>.json` retains the trigger report and matched evidence. No manual suspension does not prevent automatic loss suspension, and zero charged settlements does not establish case clearance.
- **Opportunity prioritization** — strongest player-visible source converts; weaker expires with its report; a decoy dismisses through the canonical lifecycle.
- **Rival intelligence** — after a normal daily rival-expansion boundary, watch a known rival organization, select the first player-held Personnel observation with an Enterprise subject, and attach it to one direct follow-up watch. No hidden enterprise enumeration supplies targets; failed/partial discovery records absence rather than inventing a target. The default-world contract requires the complete discovery/follow-up chain.
  - The focused enterprise watch additionally learns local PoliceActivity, with typed patrol intervals only on an achieved watch; broad organization discovery does not reveal patrols at every racket location.
  - `rival-intelligence-w<world>-p<policy>.json` retains exact summaries, quality, typed signals, observation times, source identities, exposure and attached information; artifact IO failures propagate.
  - Existing defector watches also surface their incidental racket sightings in the narrative, closing view and per-run `player_visible.known_rackets`; these are historical, bounded observations, never live portfolio totals.
  - Production contract tests cover bounded active-only discovery, partial/failed withholding, exact-subject authorization, snapshot freshness and persistence. Observations and after-action findings name the observed racket kind so several activities at one venue remain distinguishable without disclosing revenue or cases; a co-located three-kind contract test also preserves those summaries across save/restore.
- **Organizational capacity** — overlapping specialist assignments reject atomically with a typed error and unchanged state; the specialist releases after the prior operation reaches terminal; mandate revision advances version.
- **Repeat-take depletion** — an immediate re-score is reduced by the authored repeat-target rule but already reflects whatever value replenished during the second operation's execution; intermediate elapsed time increases recoverable value monotonically, and a re-score after the authored recovery window returns full value. Probe expectations use production's nearest-cent (half-away-from-zero) rounding; an alternate-world contract test protects fractional-cent recovery.
- **Enforcement-attention conversion** — organic hits are probabilistic (per-cycle authored rates multiplied by active originated district cases), so batches count them (`racket_inquiries_drawn`) and focused `enterprise_execution` tests prove hit, compounding, observable manager warning, institutional case privacy, and post-shelving release; full mode additionally runs a deterministic probe: a clean-district control cycle must draw nothing, then enough parallel district cases opened through canonical incident intake push the rate to certainty, and the next cycle must pay the compounded street surcharge and open a dedicated inquiry on the racket as an active originated case without silently creating organization-held formal case knowledge.
- **Legal foundation** — arrest → paid counsel → custody-preserving prosecution referral → terminal decline → named cooperative witness intimidated through canonical pressure with mandatory cooperation degradation. Focused legal tests additionally prove:
  - custody cannot be evaded by internal operation/work bookings; detained investigators release their case seats; conflicted subjects cannot investigate themselves;
  - scheduled investigator capacity is rebuilt from authoritative work records after restore; completed/cancelled work releases that live slot, and each evidence source receives at most one real review attempt across direct and autonomous scheduling unless the prior work was cancelled before review;
  - detained prosecutors release reviews for deterministic office restaffing without rewriting historical referral actors;
  - autonomous arrest custody ends at the authored maximum without preventing prosecution review from continuing after release, and does not immediately re-arrest the same subject from unchanged case evidence;
  - one named witness or informant source cannot manufacture corroboration through multiple evidence records; named testimony cannot inject an unrelated case target; a case subject cannot enter as its own named witness;
  - later subject promotion preserves historical testimony but cancels pending interview work and blocks further witness actions/pressure;
  - a detained named witness cannot be authorized as a field-pressure target, and detention arising after pressure begins blocks the cooperation mutation;
  - direct testimony cancels a redundant scheduled interview without cancelling the case;
  - inactive originated files shelf even when they retain an at-large identified lead;
  - direct and organization-policy automatic counsel can aggregate sponsor liquidity; automatic counsel is established before the one-time detainee informant decision and materially lowers its authored cooperation chance; mandate-sourced automatic or explicitly delegated counsel remains confined to its Legal-scope mandate budget account and current budget window.

Cross-cutting contracts:

- **Matched-window financial honesty** — each branch snapshots cumulative finances at the shared campaign-day boundary before its arc extends; the contract asserts identical legitimate income everywhere, identical enterprise economics across unheated branches, and that a branch with a staffed case (casing counts) never out-earns an unheated one over the same window. Readouts quote matched snapshots, not raw totals.
- **Money states** — liquidation proceeds stay dirty street cash until laundered through an owned cash-intensive front; the front's per-cycle plausibility ceiling visibly rejects the over-capacity remainder.
  - Focused contract tests prove delayed and repeated same-owner reopening retain consumed capacity and ledger provenance across save/restore, rejection leaves state unchanged, actual settlement renews capacity, and acquisition discards prior-owner provenance.
  - Accounted funds reconcile exactly as laundering gross plus canonical owner withdrawals minus the front's fee minus acquisition spend minus only those payroll debits that the payroll outcome's canonical ledger transaction actually sourced from accounted-funds accounts. Payroll treats street cash, concealed cash, accounted funds, and legitimate operating funds as organization liquidity while excluding settlement counterparties; it spends street cash before concealed reserves and dirty cash before clean liquidity.
  - Direct and organization-policy automatic legal retainers may aggregate sponsor-owned liquid accounts and use the shared unrestricted-spending order (street cash, then concealed cash, then clean liquidity) rather than account creation order; mandate-sourced automatic and explicitly delegated retainers use only the mandate's budget account, consume its current budget window, reject every secondary outflow, and require the designated account to hold the requested funds rather than treating unused budget authority as credit.
- **Legitimate-wealth gate (primary set)** — on the primary narrative seed the strict chain is required:
  - PRESS keeps a $50 policy reserve while laundering surplus street cash, pays the authored laundering fee as a non-recoverable cost, and withdraws only the owned front's earned legitimate surplus (settled net, bounded by actual till liquidity, excluding opening capital) through `validate_sweep_business_profits` until aggregate organization-owned accounted books cover the venue's authored price.
  - Leadership attempts the purchase once while short (canonical rejection evidence), then buys at the authored kind price through `economy::business_acquisition`: ownership transfer plus first operating economy plus full accounted payment, with the ledger recording deterministic debits across as many accounted-funds accounts as needed and crediting only the acquired business's non-liquid seller-settlement counterparty, surfaced as a Notable report.
  - The second-district racket is established there (ownership being the production prerequisite for hosting), and the new book settles positive cycles before the final financial view.
  - Rotated sets run the same paths on worlds whose economics may stall accumulation; concealed cash cannot be laundered directly, but legitimate front withdrawals can still finance the purchase and concealed reserves can capitalize the racket. Rotated sets require an honest ending: the cooled read confirmed, with any purchase that did occur carrying the full consistent chain.
  - Focused contract tests cover reserve preservation under excess laundry capacity, no double withdrawal or opening-capital drain, concealed-reserve expansion, and canonical purchase rejection without state mutation.
  - Acquisition spends accounted funds only and never returns purchase consideration as buyer-controlled operating liquidity; payroll may draw any organization-owned spendable liquid account, including enterprise-referenced cash and accounted/operating funds, but never settlement counterparties.
- **Payroll** — every session crossing a campaign-day boundary meets payroll through the canonical ledger path; totals are raw evidence in metrics and the financial view.
- **Governed rival world** — the Rosetti organization manages home rackets identically across matched branches unless branch actions create pressure the organization can actually observe (police fear throttles expansion/reopening while it decays, and settled racket heat supplies recent district-pressure knowledge). Delegated lifecycle/growth never reads hidden live investigations and ages its own pressure observation out at the legal cold-case horizon. A chronic-loss suspension survives the boundary that caused it; on later daily boundaries a rival may reopen a suspended racket only when its frozen authority/assets remain valid, its manager still has Delegated/Broad discretion, projected zero-variance net is positive, and its existing cash account still covers one cycle after active-racket reservations. Temporary heat, funding, detention, or bad economics leave it suspended; revoked/re-scoped authority, lost required host/support ownership, or an immutable manager autonomy that cannot perform autonomous governance makes the rival retire that stale frozen configuration and release its slot. One mandate may reopen at most one racket per pass, and reopening consumes that mandate's same-day expansion action. Remaining mandates consider all viable authored kinds, refuse non-positive zero-variance net under observed pressure, preserve explicit neighborhood/business authority priority, honor organization-wide `Function(Enterprise)` as a broad fallback, and choose the strongest current economics inside a tier with stable deterministic tie-breakers; observed audit-only via `rival_home_enterprises`.
- **Manager reports reach leadership** — notable enterprise settlements atomically persist a Financial report citing the manager's organization-held information, so executive briefs surface the same observed economics, changed street surcharge, vice warning, and loss suspension without revealing formal case internals. Each settlement summary names the racket kind, location, and responsible manager; district- and business-hosted contract tests protect attribution and exact source-summary preservation in the brief. Unchanged profitable, low-variance heat settles as routine without another report. Focused contract tests cover brief inclusion without pending decisions, unchanged-heat silence, and report-ID exhaustion rejecting the entire settlement without mutation. Narration quotes only this organization's cycles; live rival racket totals remain diagnostic-only.
- **Casing risk symmetry** — surveillance can draw trace-level exposure and open a case exactly like a burglary, so heating signals are session-wide rather than tied to the burglary's resolution record; achieved surveillance reads the deployment's recurring patrol rhythm into typed intervals (the crew infers shift patterns from the watch rather than only the minutes it stared at one corner, so casing can protect later work planned around that rhythm), with short and wrapped windows conservatively bounded rather than collapsing into all-day coverage; independent civilian witness pressure uses one latest pressureable foreign witness case for patrol/exposure geography instead of occurring nowhere or spanning several simultaneous case districts, with the latest durable foreign registration used only when pressureability disappears in flight; refused poaching pitches surface as player-visible loyalty reports counted as poach warnings.
- **Recruitment route ownership and contention** — delegated managers rank visible prospects by relationship support and choose autonomous pitches from recruiter-owned disposition rather than latent prospect drives or traits; candidate motives affect only the candidate's willingness calculation. When organizations contend for one prospect, the strongest currently actionable relationship gets first access and stable IDs break exact ties only. A pending approval makes that organization/candidate pair unavailable to autonomous recruitment and blocks generic world reassignment into the target organization, while resolving that exact approval remains able to complete the membership change atomically.
- **Player decision authority** — the harness counts and resolves only pending requests addressed to the player organization. Foreign requests (including NPC approvals already resolved during the tick) and terminal reobservations remain untouched; canonical request/resolution contract tests protect ownership, unchanged foreign state, and one-time player attention.
- **Executive brief prioritization** — pending decisions are listed first, then bounded source entries; source caps preserve higher attention first and newest reports within an equal-attention class, so old same-priority items cannot crowd out later developments merely through lower report IDs; an overflow disclosure carries the highest omitted attention class rather than a fixed label; pending decisions remain oldest-first within an attention class by request time.

## Completion checklist

Before handoff:

- [ ] Ran exactly the smallest scripted completion lane that covers the changed surface (see decision tree above); did not rerun an overlapping focused proof immediately before that lane.
- [ ] `git diff --check` is clean; generated output and final worktree reviewed.
- [ ] If a test, harness mode, alias, or verification rule changed, this document and the owning script/command definition were updated in the same change.

When a test, harness mode, alias, or verification rule changes, update this document and the owning script or command definition in the same change.
