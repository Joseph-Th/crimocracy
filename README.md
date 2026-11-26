# Crimocracy

Deterministic Rust simulation foundation for a systemic crime-organization strategy game. The player acts through people, information, plans, policies, relationships, delegated authority, enterprises, and institutions.

## Start here

Use each document for one job. [`AGENTS.md`](AGENTS.md) owns agent execution rules and routing; [`STATUS.md`](STATUS.md) owns implemented scope and exclusions; [`ARCHITECTURE.md`](ARCHITECTURE.md) owns state ownership, mutation, determinism, persistence, invariants, and tick order; [`TESTING.md`](TESTING.md) owns verification and text play evidence; [`GAME_DESIGN.md`](GAME_DESIGN.md) owns product intent; [`CITY_VIEW.md`](CITY_VIEW.md) describes the read-only procedural city projection for future GUI adapters.

`content::build_registry()` builds immutable authored definitions. `AppState` owns serializable campaign state and deterministic runtime state. [`core::simulation::run_tick`](src/core/simulation.rs) advances exactly one simulated minute through the contractual phase order in [`ARCHITECTURE.md`](ARCHITECTURE.md), returning a typed error without mutation when the finite simulation clock has no successor minute.

On each day boundary, non-player criminal owners and players who enable daily treasury distributions sweep legitimate business earnings above each storefront's protected working-capital floor into accounted funds before payroll. The finance owner commits a single balanced, preflighted ledger batch, filling existing accounted-funds receiving capacity before opening a new destination. Private proprietors follow the same receiving rule for their own earned dividends. Players using manual treasury distributions can withdraw earned surplus explicitly; neither path treats laundering fees as earned business profit. Legitimate businesses employ their own ordinary staff, so ownership dividends are not arbitrarily capped by the number of named criminals. Criminal operations are different: their actual assigned captains must handle the ongoing burden of operating sites, workers, supporting fronts and geographically separate districts.

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
production simulation. Enter `help` for the full command reference. `advance 30d`
uses the normal attention rules and can pause before the requested end.
`advance 30d uninterrupted` explicitly suppresses optional event pauses for
long-term planning, but **does not skip simulation minutes** and still stops at
pending decisions or a terminal campaign outcome. It reports the actual elapsed
minutes and a digest of deferred alerts, including player-crew departures after
wage shortfalls; normal advancement pauses for an enabled exception when members
walk away. The `brief` command shows gross full-day payroll runway based on
presently accessible wages, without assuming new income or other expenditures.
Inspect `reports`, `people`, `counsel`, and `recovery` after advancing. A commercial empire without adequate local rackets or capable available enforcers now attracts **rival territorial expansion and protection demands**, even if it never ran a racket in those wards. Use `turf` to see active owned storefronts, locally stationed racket associates and the share of trained enforcers allocated across your organization's districts; rival workforce details remain unknown without reconnaissance. When a gang demands payment, inspect its offer with `decisions`, then use `decide <id> approve` to transfer real clean organization funds, or `decide <id> reject` to refuse. Its 30-day price is 5% of normal monthly storefront gross in that ward, bounded at $50–$15,000. Payment halts that gang's existing and newly planned reconnaissance/sabotage against your businesses in the ward until coverage expires, but does not protect against unrelated gangs. Refusal allows renewed coercive pressure, including actual scouting and disruption. A profitable legal holding company is not automatically a defensible criminal organization. Even the two original storefronts may be exposed if a stronger gang moves in. Operation scheduling delays and deadlines remain
limited to seven days.

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

## Named people across the city

A generated city gives individually simulated characters to private business proprietors, municipal politicians, police detectives and officers, prosecutors, lawyers, merchant and labor representatives, alongside criminal personnel. Civilian owners hold actual business titles, accounts and relationships, receive personally earned dividends from shop accounts above protected working-capital floors, and can reject or accept player offers to buy their properties. Their reservation prices reflect their careers, financial livelihood and personal traits. Some properties remain explicitly listed for immediate public acquisition; no direct listing silently turns into a hidden seller.

Every daily boundary assigns named civic characters a real duty or time off. Shopkeepers attend an active business, police cover existing precinct patrols or lead assigned cases, and institutional officials work in their own organizations. Career experience, fatigue and professional standing persist in saves, affect detective allocation, and alter the strength of a named mayoral incumbent and candidate at election time. Detention interrupts work. The game simulates this named professional population rather than all residents or per-minute walking routes.

Use `civics` for public profession counts, `civics district-N` for people in a ward, and `civic-person char-N` for one person's registered business ownership and civilian professional history. Own criminal members instead show their actual completed crew operations, achieved objectives, current racket management and personally maintained institutional channels; the civilian workday count is not applicable to their criminal leadership. Confidential investigation assignments, outsider fatigue and personal bank balances remain private.

A city is not just a handful of special NPCs. Each generated district adds **42 more individually persisted people**, including 12 storefront employees with actual workplaces, nine independent criminal freelancers, eight other residents, four additional precinct officers, two politicians, two defense lawyers, a prosecutor, labor and merchant representatives, a journalist and independent counsel. Additional rival gang members also have real organization membership, supervisors, relationships and payroll. A small city now contains more than 200 named people, a medium city more than 400, and a large city more than 800. The largest supported custom cities exceed 1,100. Each individual has their own skills, motives, personality, district home and daily professional history; this does not simulate every anonymous customer or voter as a character.

Use `population` to see the full named census without revealing hidden identities, `freelancers` to find independent criminals personally known to the crew, and `contract char-N char-M <days>` to pay an identified freelancer for 1 to 14 days of supervised work under member `char-M`. The signing fee is assembled from the organization's spendable street-cash accounts, including across multiple pockets when no single account can cover it, and paid to the contractor's own account. The contractor temporarily joins the crew so normal job assignments, wages and custody rules apply, and departs at the signed expiration minute when no active operation or detention prevents release, with the final earned payroll settled first at midnight. Normal `advance` pauses to report an expired player contract; `uninterrupted` advancement counts completed terms in its summary. Afterward they remain available for genuine recruitment through the established personal-relationship system. Neither a contract nor ordinary city directories reveal otherwise unknown criminals.

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

Sandbox campaigns have three **optional, player-claimed endings**. Use `victory` to inspect actual eligibility, then `claim-victory political`, `claim-victory retirement`, or `claim-victory underboss` when qualified. Meeting a threshold does not automatically terminate play. A successful claim creates a durable victory history and report, keeps saves readable, and prevents further simulation ticks.

**Political victory** requires winning an actual municipal election and proving a transition to legitimate business. The organization needs an active political institutional contact held for **180 days**, no active rackets, and **365 days since its last racket settlement**. At least three quarters of city districts must each contain **three active storefronts continuously owned for 1,080 days**, spanning **two distinct business kinds**. Each qualifying storefront needs 20 profitable settlements within the latest 30 days and one settlement within seven days. The district's mature 30-day net income, after applicable portfolio administration, must cover 30 days of one named member's wages; the mature citywide legitimate portfolio must cover **all current named-member payroll** without laundering or criminal income.

Each qualifying political district needs **two different, undetained delegated managers**: one holding its Neighborhood and Political-function scopes, the other its Neighborhood and Finance-function scopes. A manager cannot fill posts in multiple districts, and the executive cannot double as a local delegate. The organization must maintain **four headquarters members plus two independently matched district directors per required district**. This means at least 10 named members in a four-district city and 16 in an eight-district city. Use `delegate <member-id> district-N`, `mandate <mandate-id> add political` or `mandate <mandate-id> add finance`, and `autonomy <member-id> delegated` to staff real responsibilities. Ownership alone, nominal mandates and unstaffed payroll do not qualify. Political and finance posts are assigned jointly to complete teams, prioritizing electoral-ready districts; a dual-qualified person is not automatically consumed by the first role.

**Retirement** ends an actual criminal career, not a passive landlord portfolio. The organization must retain **at least four named members**, have **at least 90 profitable racket settlements** and **$10,000 net settled criminal income over the latest 180 days**, and hold net accounted reserves to fund a **$1,000,000** leader nest egg plus 180 days of current daily wages for every subordinate. Retired and suspended rackets contribute their actual recent settlements, including losses, so a boss may shut down operations before exiting. The actual payout transfers accounted funds to named personal accounts through one balanced ledger transaction. Gross storefront value, street cash and uncollected profits do not substitute for available clean reserves.

**City underboss** requires a mature, diversified organization controlling at least three quarters of districts. Rackets need **90 days of established operation**, 20 profitable settlements in the latest 30 days, a settlement within seven days, and mature local 30-day net covering 30 days of one member's wages. Each qualifying district needs a strict mature racket majority, **30 proven associates across at least two racket kinds** exceeding the combined rival local workforce, **three available named defenders**, and two independent, undetained non-executive captains with Delegated or Broad autonomy and 20 personally directed profitable recent settlements. Citywide, mature rackets and proven associates must exceed all rivals combined. A manager's previous results do not transfer with a reassigned racket, nor may the same captain occupy two qualifying district posts. The same defended and independently commanded districts need **200 machine influence and 100 institutional support** each; see `campaign` for civic standing. The organization needs **24 available named members, or six headquarters members plus six per required district, whichever is greater**, an available executive, and a delegated senior commander with Management 70+ directly supervising two selected captains in different districts of the same feasible, fully backed coalition. At least three of the legal, political, labor and professional contact sectors must have active usable channels held 90 days, with at least two handlers. No election or police contact is required. Mature realized racket income must cover the full named payroll, and liquid reserves must cover **180 days of named wages plus 90 days of current racket costs at maximum modeled heat and retained campaign specialist salaries**; storefront tills, clearing balances and projected future earnings are excluded. Recent net losses from young, suspended, retired or transferred ventures reduce route earnings until they leave the 30-day window; positive earnings from ineligible ventures do not count toward mature standing.

Each racket has baseline associates already included in operating costs. `staff-racket <enterprise-id> <associate-count>` adjusts up to eight additional workers; they cost 1,500 cents each per daily cycle and change demand-dependent revenue. Workload derives from the operating sites, supporting storefronts, associates, distinct districts and actual subordinate reporting branches. Captains' Management skills and superior reporting chains govern sustainable capacity. Extra hires never increase their command capacity. Use `supervise <member-id> <supervisor-id>` to reorganize reporting lines. Unsupported work suspends rackets through canonical lifecycle maintenance. District police pressure adds at most one recurring street-heat surcharge per racket, although every additional investigation still creates evidence and legal exposure. New inquiry risk scales with actual cases up to two case sources and with significant rather than minor changes in the organization's public and criminal footprint. Routine post-release arrests by the same authority have a 60-day minimum interval. Reobserving a racket already cited in the prior police case cannot itself create clockwork renewed custody, even after 60 days. Fresh evidence of a different racket or newly completed criminal operation remains actionable. The `brief`, `enterprises`, `racket-outlook` and `victory` views distinguish these costs and responsibilities. A permanent specialist office requires a distinct district supervisor, even if that person holds multiple neighborhood mandates. The `campaign` view distinguishes contracted salaries from the currently payable subset and identifies unavailable offices; dismiss an office's contracts before assigning its supervisor elsewhere.

The `victory` view identifies mature, profitable districts, eligible independently matched political and financial directors, headcount, earned operating income, territorial workforce and required reserves. `political district-N` shows actual title ages, the **1,080-day** maturation countdown, 30-day owned settlements, current blockers and live eligible political and finance directors even in wards with no mature or owned storefronts. Only economically qualified wards consume counted teams; rival private records remain hidden. A successful mayoral claim reports the elected leader, established districts, responsible managers and political sponsor; unsupported older save schemas fail closed.

## Mayoral campaigns, civic coalitions and actual elections

Municipal elections recur every **180 simulated days**. File the organization's sole undetained executive through `nominate-mayor char-N` at least **30 days** before the ballot, backed by the **180-day political contact** and a **2,000,000-cent ($20,000) filing fee**. Missing the deadline or losing an election leaves the next scheduled contest available.

In addition to the established commercial and management coalition and the one-year legitimate transition, the campaign must have spent **35,000,000 accounted cents ($350,000)** and retain at least **15,000,000 cents ($150,000)** in net clean reserves at the ballot. Each qualifying district must maintain **800/1000 machine, 300/1000 grassroots and 300/1000 institutional support** and a machine advantage greater than **3:2** over the strongest rival for **90 consecutive days**. Falling below any condition restarts the district's majority tenure.

`buy-influence district-N cents` purchases machine influence for **5,000 cents per point**. That purchase does not buy voters or endorsements. `hire-specialist district-N char-N organizer|publicist|strategist` hires staff with signing fees of 150,000, 350,000, or 600,000 cents and daily salaries of 3,000, 8,000, or 16,000 cents. Organizers concentrate on grassroots work, publicists mix both civic constituencies, and strategists build institutional networks. All also maintain some machine influence. Their actual output scales with their assigned delegate's SocialAccess, Negotiation, or Management capability; an idle or detained manager cannot produce influence. `fire-specialist` ends that wage. Every support axis decays daily and unfunded salaries yield no gains.

Campaign operations additionally use `campaign-event district-N char-N meetings|services|labor|press`. **Meetings** require an owned active gathering/customer venue and cost 150,000 cents; **services** require an owned active soup kitchen, pharmacy, or laundry and cost 450,000 cents; **labor** requires local union infrastructure or an established labor contact and costs 350,000 cents; **press** requires an owned printing press or news service and costs 300,000 cents. Each action builds a different mix of grassroots and institutional support, requires a delegated local Political/Neighborhood manager, and has a **seven-day cooldown per district and initiative**. All campaign transactions use real clean funds and move expenditures to the city's political institution; fragmented clean accounts can fund a single approved payment atomically.

The `campaign` dashboard uses the simulation's exact daily erosion rule to show each district's next machine, grassroots and institutional standing losses before specialist output. This makes the effect and maintenance cost of small civic initiatives visible before committing further clean funds.

Each election resolves public ward-by-ward vote totals for the nominee, incumbent political opposition, and rival influence. Voter blocs, the local commercial coalition, maintained majority tenure, citywide campaign financing, **resident fear** from actual criminal reputation, and active neighborhood rackets affect whether a ward is won. `campaign` shows available capital and staffing; `political district-N` shows the local coalition and initiative blockers; `election` gives official ballots and ward totals. Only a certified **winning election** permits `claim-victory political`, which records the elected mayor and terminates the campaign. This simulates ward elections and institutional campaigning, not individual voter identities or post-election governance.

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
suspended rackets. A staffed criminal rival must have one undetained executive to accept an offer; divided command and executive detention prevent consent, even for held offers. Holdings without remaining personnel can still be liquidated. A staffed rival protects its productive payroll base if there is no affordable
replacement and the offered price plus liquid reserves cannot cover a full year
of any post-sale payroll shortfall plus a week's payroll. Recently purchased
profitable storefronts ordinarily stay with the buyer for ninety days, but a
large buyout covering their standard price plus 180 days of forecast profit
can justify selling sooner. Rival executives still offer only the ordinary
90-day appraisal, preventing constant weekly flips. Refusal alone does not
reveal the reason.
Rivals invest across the city and submit voluntary offers themselves, but cannot
purchase your properties without your consent. Before buying a closed storefront,
executives account for the lost custom at their existing local shops and decline
purchases whose restarted operating income would not offset that loss. They also
avoid assets too exhausted to schedule another operating cycle. Accepted offers
pay the actual seller, transfer the existing
books, and restart the acquired operating cycle. `business-outlook <id>` identifies whether a property is independently listed, already yours, or not available for direct purchase without disclosing foreign title, and clearly labels its normal-business forecast as *standalone*, not profit added to your portfolio. `launder` converts street cash within an owned front's
real capacity, `launder <business-id> max` fills the affordable remaining capacity of a chosen front, and `launder-all <street-reserve-cents>` issues a **one-time**, ID-ordered conversion through eligible owned fronts while preserving the requested total spendable street reserve. Each front settles as its own canonical ledger transaction, pooling spendable street accounts as needed; unsuccessful fronts are explicitly reported, and successful earlier postings are not rolled back. Neither command is an unattended laundering policy. `businesses` shows owned infrastructure and earned owner-withdrawal headroom, `treasury daily` authorizes daily profit distributions before payroll (protected working capital stays in the storefront), `treasury manual` restores explicit withdrawals, and
`racket-kinds` lists enterprise requirements. `open-account`, `move`,
`fund-street`, and `start-racket` let leadership fund and establish more
enterprises with actual mandates and existing books. `brief` and `accounts`
show both net treasury values and actual spendable funds, with deficits kept
visible rather than silently netted against another account's purchasing power.
Ordinary `move` transfers only between accounts of the same spendable money
kind, never dirty cash directly to clean funds. `fund-street <accounted-id>
<street-id> <cents>` explicitly withdraws available clean funds into a
nonnegative street-cash operating account. That payment has a balanced,
auditable ledger receipt and cannot be reversed to clean funds without using
an actual laundering front. If an old street account is overdrawn, open a
new `street` account and use its ID instead of silently erasing that liability.
`operation`, `dispose` and `deposit` handle held proceeds through production
reports and ledger transactions. `accounts` distinguishes general settlement
accounts from accounts dedicated to a racket; the latter cannot clear operation
proceeds.

`start-racket` accepts either an owned `business-N` venue or a public
`district-N` for rackets that do not require a physical host. Supply any
number of distinct owned support-business IDs after the two account IDs;
distributed numbers and slot-machine rackets need multiple actual business
functions across that network. `racket-kinds` lists the required
functions. Add `manager=char-N` to choose an eligible manager explicitly; a
cross-district network requires an organization-wide enterprise scope.
`preview-racket` takes the same arguments, validates without establishing the
racket, and shows the chosen manager, conditional costs and earnings, the
case-pressure threshold for nonpositive expected earnings, and the effects of one or
two additional active district cases at unchanged gross and staffing. Both `preview-racket` and `enterprises` warn when assigning several active rackets to one manager would expose them to a simultaneous suspension on detention. `business business-N suspend` pauses a player-owned storefront
only after its active racket dependencies are suspended;
`business business-N resume` restarts its real operating cycle without resetting the
current laundering window. Suspended storefronts now also appear in `recovery` alongside rackets, including last realized margin and the actual resume blocker. Each racket still requires a player-authorized resumption, but `resume-rackets <id> [id...]` validates a **player-selected** group before making any status change. It does not automatically restart unselected or detained managers' operations. `reassign-racket <id> <member-id>` transfers an existing racket to an eligible manager whose mandate covers its whole network, retaining the accounts, historical settlements, and current operating status. Use `racket <id> resume` afterward if it was suspended, or use `cover-rackets <char-N> <id> [id...]` to explicitly select one eligible replacement manager and atomically reassign and resume the selected suspended rackets. A blocked member, capacity limit, host, or territory condition rejects the entire order without partial changes. An active racket with a due settlement must settle or suspend before reassignment.

For financial review, `ledger [count]` lists recent transactions touching
organization-owned accounts, and `transaction transaction-N` shows only
the player's own postings, excluding the counterparties' private books.
`business-cycles business-N [count]` summarizes actual storefront
settlements earned under player ownership;
`racket-cycles enterprise-N [count]` reports gross, operating costs, net cash, enforcement costs, and the manager responsible for each historical settlement.
All histories are read-only and survive save/load.

Territory is contested by active rackets rather than an independently owned
district token. `turf` lists your active rackets and associates per district,
observed rival rackets, the time and source of their latest location sighting,
and any binding agreements. Knowing an enterprise's identity alone does not
reveal its district or authorize a territorial challenge or negotiation.
Unknown rival holdings and exact staff numbers remain private; reconnaissance
provides the necessary location evidence. `preview-contest enterprise-N` inspects the legal opening and your
available manpower without revealing the opponent's current strength.
`contest enterprise-N` then challenges that rival with your actual local
associates. The defender receives a two-associate positional advantage. The
loser has one racket suspended for seven days and cannot reopen it early, even
by retiring it and establishing a replacement of the same kind at the same
location;
the attacker must also wait seven days before another district challenge.
Stronger rivals autonomously contest overlapping territory and can shut down
player holdings, triggering an actionable crisis report. The resulting lost
enterprise income and district influence are real.
NPC challengers estimate opponents' baseline crews rather than reading their
secret extra hires. When several raids are viable, they prioritize the largest
estimated staffing advantage, then the target with the larger baseline crew;
creation order only breaks equal strategic ties. Reinforcements can reverse the
outcome, and a repelled NPC avoids repeating the same district attack for
30 days. Non-player gangs may negotiate ceasefires with each other after actual
clashes, with treasury-backed tribute when appropriate. They never sign for the player.

`negotiate district-N org-N [tribute=cents]` proposes a bilateral thirty-day
standstill. Both parties must currently operate there, or have a racket recently
displaced in a territorial clash. Near-even rivals can settle without paying;
a rival with a greater racket advantage demands 25,000 cents for each active
racket beyond a one-racket lead. Tribute is paid immediately in real street
cash into the other gang's treasury, which can finance its future expansion.
An accepted standstill protects existing rackets, including a suspended
racket's right to resume after a clash, but stops both parties from opening
additional local rackets or authorizing offensive business operations until
expiry. Reconnaissance and legitimate acquisitions remain possible. These are
non-lethal workforce confrontations and negotiated expansion restrictions, not
a simulation of street-level shootouts, permanent land transfer or elections.
The ceasefire bars clashes between the two signatories, not clashes with an
unrelated third gang. A gang may hold only one such agreement per district at a time.

Leadership decisions include `policies`, `policy`, `mandates`, `delegate`, and
`mandate` for organization-level recruitment and legal-support rules, managerial
territory/functions, standing orders, budgets, and revocation. Use
`policy recruitment cap-N` to delegate recruitment until the organization has
N named members, or `policy recruitment pause` to stop independent pitches and
approval requests while keeping mandates intact. Multiple managers share the
remaining roster space; refusals do not consume a hiring slot. The `policies`
view shows remaining slots and named payroll at the target, while `mandates`
shows effective caps. A Personnel mandate may explicitly override the organization
policy with its own standing order; an executive's direct recruitment order is
also independent of the cap. Lowering a target never dismisses existing members.
`districts` and `directory` expose public district IDs needed for territorial delegation.
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
refusals rather than repeating an unchanged pitch every cooldown; `prospects` explains when every recent pitch has been refused and the manager is waiting for a relationship change or an older refusal to expire. A recruiting
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
`connections` lists known outside acquaintances and eligible permanent handlers, distinguishing established channels, potential channels, noninstitutional acquaintances, and dormant social ties without consulting outsiders' private custody;
`connect <handler-id> <known-person-id>` establishes a real institutional
channel only when the modeled social relationship supports it. `contacts`
lists established channels, dormant relationships, detained handlers, and outstanding disclosures;
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

Routine autonomous custody has a sixty-day cooldown following release by the same police authority. Continuing observations of an already-cited racket cannot restart custody merely because that cooldown expires or an unrelated arrest intervenes. Distinct new racket evidence remains actionable after the cooldown; a newly completed criminal operation can justify earlier custody. The direct arrest evidence gate, pending prosecutions, and ongoing enforcement costs remain intact.

`counsel` displays current-member legal matters and your own active retainers, including those for former members. Another sponsor's retainer is indicated without revealing its management ID, lawyer, or fee. `counsel` lists only currently usable defense channels, not prosecutors or dormant relationships. `retain <arrest-id> <legal-contact-id> <fee-cents>` hires counsel for a current member through a live legal-services contact and the production ledger, using an existing provider operating account that can receive the fee.
`end-counsel <representation-id> <concluded|replace|withdraw>` closes your sponsored representation, including after a member's departure, only when its legal constraints allow it. Alternatively,
`policy legal automatic` enables automatic retention when a legal contact
and funding exist. `advance` interrupts on arrests and releases of members
so manual counsel decisions are not skipped. It also pauses on newly suspended
own businesses or rackets, the first losing cycle in an own racket's current
losing streak (including after reopening), and payroll shortfalls by default.
Use `attention exception off` for uninterrupted routine management; actual
pending decisions and own custody changes still interrupt. `brief` displays
current daily payroll coverage, liquid organization-owned operating funds, and
withdrawable earned surplus. Portfolio-administration estimates apply the actual
per-storefront fee rounding. When manual distributions strand earned cash in a
storefront while wages cannot be funded, the brief explains the withdrawal choice.
Its realized historical income comparison uses settlements from the most recent
30 elapsed days (excluding the oldest boundary) and the present payroll
obligation, not an estimate of past payroll spending. Before day 30 the
average uses the exact elapsed minutes, with a one-day minimum denominator.
Uninterrupted review counts individual new racket loss streaks, even when
several rackets settle in the same minute.

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
