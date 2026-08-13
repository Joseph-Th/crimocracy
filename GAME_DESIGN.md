# GAME DESIGN DOCUMENT

## Untitled Crime Organization Simulation

**Document status:** Core design specification  
**Genre:** Systemic crime-organization strategy / management simulation  
**Primary mode:** Single-player  
**Platform assumptions:** Desktop PC, mouse and keyboard first  
**Technical implementation:** Rust, intentionally outside the scope of this document  

---

# 1. High Concept

The player builds and runs a criminal organization inside a persistent simulated city. The game is not primarily about personally committing crimes, manually moving individual gangsters, or optimizing production chains. It is about making consequential decisions through people, information, plans, policies, relationships, and institutions.

The city continues to function whether or not the player is looking at it. Businesses operate, residents move, police patrol, investigators build cases, rival organizations recruit and expand, politicians pursue careers, newspapers shape public attention, and individual characters form loyalties, grudges, debts, and ambitions.

The player's job is to understand this world well enough to exploit it.

The central design goal is:

> **High simulation complexity, moderate-to-high decision complexity, low control complexity, and carefully managed information complexity.**

The player should feel like the head of an organization, not its dispatcher, accountant, pathfinder, or omniscient god.

---

# 2. Design Thesis

Earlier crime simulations often contained excellent systems but required the player to operate those systems at the same level of detail as the simulation itself. Detailed plans required detailed movement instructions. Large organizations required large quantities of repeated orders. Deep investigations were represented by opaque meters or unexplained failures. Expanding an empire often expanded the player's workload faster than their authority.

This game separates four kinds of complexity:

- **Simulation complexity:** May be very high.
- **Decision complexity:** Should be meaningful and strategically demanding.
- **Control complexity:** Must remain low enough that the player expresses intent rather than procedure.
- **Information complexity:** Must be filtered through the player's organization so that uncertainty is meaningful rather than confusing.

The simulation may track thousands of facts. The player should only need to act on the few that matter.

Every major system must primarily produce at least one of three things:

1. A meaningful decision.
2. Useful information.
3. A consequence that changes future decisions.

If a repeated interaction produces none of these, it should be automated, delegated, summarized, or removed.

---

# 3. Player Fantasy

The player fantasy is not "be a gangster."

It is:

> **Build an organization that can do things you personally could never do, while remaining powerful enough to control it and informed enough to survive its consequences.**

The player begins as a small operator with a few trusted people, little political protection, limited capital, and poor information. Their early decisions are personal because the organization is small. As the organization grows, the player's unit of control changes.

Early game questions:

- Who can I trust to collect this debt?
- Which store is vulnerable to protection?
- Can I afford a lawyer if this goes wrong?
- Is this informant reliable?
- Which fence will buy the stolen goods?

Mid-game questions:

- Which lieutenant should control the waterfront?
- Should we tolerate a rival's gambling operation in exchange for access to their union contacts?
- Is a profitable crew attracting too much police attention?
- Which legitimate company can absorb illicit cash without becoming an obvious investigative target?
- Is a suspicious subordinate stealing from us, informing, or simply incompetent?

Late-game questions:

- Should the organization become quieter and more legitimate or escalate against the state?
- Which political institutions must be influenced rather than merely bribed?
- Can the player remove a powerful lieutenant without splitting the organization?
- Is the city's changing economy making a core racket obsolete?
- Can an active conspiracy investigation be disrupted without revealing how much the organization knows?

Progression must therefore change the *kind* of decision the player makes, not merely increase numbers.

---

# 4. Core Design Pillars

## 4.1 Intent Over Procedure

The player specifies goals, constraints, priorities, roles, and contingencies. Subordinates determine routine implementation.

The player should say:

> Break into Bellmore Jewelry after closing. Avoid casualties. Eddie handles alarms. Maria drives. Abort if police arrive before the safe is open.

The player should not say:

> Eddie walks six meters north, waits four seconds, opens door B, walks to tile 48, crouches, then waits for Maria.

Detailed simulation happens below the level of direct control.

## 4.2 Information Is a Resource

The world is not fully revealed. The player knows what their people know, plus whatever can be reasonably inferred.

Reconnaissance, surveillance, bribery, informants, accountants, lawyers, political contacts, compromised police, newspapers, and direct observation all produce different kinds and qualities of information.

The player should rarely receive perfect numerical certainty about hidden systems.

## 4.3 People Are the Organization

Every important capability is embodied in characters and relationships.

A business may matter because of the owner. A neighborhood may matter because of the ward boss. A trucking firm may matter because its dispatcher is loyal to one of the player's lieutenants. A corrupt detective may be useful but also dangerous because another officer suspects him.

Characters are not interchangeable stat bundles.

## 4.4 Consequences Persist

Crimes create more than money and a generic heat value.

They can create witnesses, evidence, rumors, debts, injuries, grudges, fear, political pressure, market shortages, suspicious transactions, missing personnel, damaged businesses, press attention, and investigative connections between previously separate events.

The city remembers through its people and institutions.

## 4.5 Power Has Multiple Forms

There is no single power score.

The organization can accumulate:

- Cash.
- Legitimate wealth.
- Illicit income.
- Territory.
- Political influence.
- Legal protection.
- Police intelligence.
- Social legitimacy.
- Fear.
- Loyalty.
- Access to specialists.
- Control over supply and distribution.
- Compromising information.
- Institutional influence.

These forms of power can reinforce one another but are not freely interchangeable.

## 4.6 Scaling Means Delegation

Growth must reduce routine control rather than multiply it.

The larger the organization becomes, the more the player governs through:

- Lieutenants.
- Standing policies.
- Budgets.
- Territorial responsibilities.
- Operating rules.
- Exception handling.

A successful late-game organization should be capable of running large portions of itself.

## 4.7 The State Is an Institution, Not a Meter

Police, prosecutors, courts, politicians, regulators, tax authorities, and journalists have their own structures, limitations, jurisdictions, priorities, and internal politics.

There is no universal "heat" variable that represents all law-enforcement attention.

The player can be obscure to patrol police while highly interesting to a tax investigator, protected by one politician while targeted by another, and suspected of a crime without the state possessing admissible evidence.

---

# 5. Explicit Non-Goals

The following are intentionally outside the core fantasy:

- Manual real-time control of individual combatants.
- Repetitive RTS-style unit selection.
- XCOM-style tactical combat as the primary resolution system.
- Repeated delivery-route optimization.
- Manual bookkeeping for every individual business.
- A universal crime "heat" meter.
- Fixed mission solutions requiring one prescribed sequence.
- A city map covered in dozens of interchangeable resource nodes.
- Large quantities of +5%, +10%, +15% upgrade progression.
- Minigames for every criminal activity.
- Omniscient visibility into AI intentions, police knowledge, or exact success chances.
- Hidden mechanics that fail without giving the player an understandable reason.
- Expansion that creates more repetitive clicks per minute.

Combat, logistics, bookkeeping, and tactical details can exist in the simulation, but they should support organizational decisions rather than replace them.

---

# 6. Setting

The default design assumes a fictional American industrial metropolis during the late Prohibition era and its immediate aftermath, approximately 1929-1935.

This period supports the required systems particularly well:

- Alcohol prohibition creates large illicit markets.
- Organized crime can plausibly intersect with unions, transport, gambling, prostitution, extortion, smuggling, and legitimate businesses.
- Police and municipal corruption can be local and personal.
- Federal institutions can become increasingly relevant as the organization grows.
- Newspapers are powerful information actors.
- Banking, cash businesses, and accounting create laundering opportunities without requiring modern digital systems.
- The end of Prohibition can force strategic adaptation rather than allowing one racket to remain optimal forever.

The city is fictional so systems can be tuned for gameplay without claiming strict historical simulation.

The city should feel specific, however. Neighborhoods need economic identities, ethnic and political histories, transport patterns, institutions, social networks, and different relationships to law enforcement.

---

# 7. The City as a Persistent Simulation

The city is the primary game board, but it is not merely territory divided into capturable regions.

It contains:

- Residents and workers.
- Businesses.
- Buildings.
- Streets and transit routes.
- Police precincts.
- Courts.
- Municipal offices.
- Newspapers.
- Banks.
- Unions.
- Social clubs.
- Churches and civic organizations.
- Criminal groups.
- Illicit markets.
- Warehouses and transportation infrastructure.
- Neighborhood political structures.

Each neighborhood has characteristics such as:

- Population density.
- Wealth.
- Commercial activity.
- Police presence.
- Political influence.
- Social cohesion.
- Existing criminal penetration.
- Tolerance for visible violence.
- Demand for illegal goods and services.
- Important local institutions.

These are not simple bonuses. They affect opportunities and consequences.

A wealthy shopping district may offer valuable robbery targets but produce witnesses, press attention, and rapid police response. A dock district may provide smuggling opportunities and union leverage but contain entrenched rival interests. A poor neighborhood may tolerate gambling but react severely to indiscriminate violence against residents.

---

# 8. Core Gameplay Loop

The game runs through a repeating strategic cycle:

## 8.1 Observe

The player receives information from the organization and the city:

- Daily reports.
- Financial summaries.
- Rumors.
- Surveillance.
- Police contacts.
- Newspapers.
- Business performance.
- Messages from subordinates.
- Rival activity.
- Legal notices.
- Investigation updates.

## 8.2 Interpret

The player identifies problems and opportunities.

Examples:

- A rival's lieutenant has become financially vulnerable.
- A police patrol pattern leaves a warehouse exposed twice a week.
- One crew's income is rising faster than its reported activity.
- A legitimate trucking company is losing money but provides strategic access to the docks.
- A prosecutor is connecting two previously separate robberies.

## 8.3 Decide

The player changes plans, policies, relationships, assignments, budgets, or operations.

## 8.4 Delegate

Orders enter the organization through responsible characters.

## 8.5 Resolve

Characters act according to instructions, competence, loyalty, available information, local circumstances, and unexpected events.

## 8.6 Consequence

The world changes.

The important result is not whether a die roll succeeded. It is how the outcome changes future possibilities.

Then the cycle begins again with better, worse, or more complicated information.

---

# 9. Time Structure

The game uses continuous simulated time with aggressive pausing and time compression.

The player can pause freely in single-player.

Recommended speeds:

- Paused.
- Normal.
- Fast.
- Very fast.

The game automatically pauses, or optionally auto-pauses, for high-priority exceptions and crises.

The player should not need to watch routine travel, normal collections, or ordinary business operations in real time.

Time matters strategically because:

- Businesses have schedules.
- Police patrols vary.
- Characters sleep, work, socialize, and travel.
- Court dates occur.
- Debts come due.
- Deliveries arrive.
- Political events occur.
- Investigations progress.
- Injuries heal.
- Memories and public attention decay unevenly.
- Opportunities can expire.

Planning therefore includes timing without becoming a scheduling spreadsheet.

---

# 10. Player Attention Model

The game must explicitly classify information by the amount of player attention it deserves.

## Routine

Handled automatically and visible only in summaries unless inspected.

Examples:

- Normal protection collections.
- Expected business revenue.
- Routine payroll.
- Ordinary low-risk deliveries.

## Notable

Does not interrupt the player but appears prominently in the next report.

Examples:

- A collection was lower than expected.
- A subordinate got into a minor fight.
- A police patrol increased near one business.
- A supplier raised prices.

## Exception

A subordinate cannot safely resolve the situation under current policy and requests a decision.

Examples:

- A protected business refuses payment.
- A planned burglary encounters an unexpected security system.
- A lieutenant wants permission to retaliate against a rival.

## Crisis

Immediate high-impact event that justifies interruption.

Examples:

- A key lieutenant is arrested.
- Police are raiding a major property.
- A rival attack threatens important personnel.
- A witness is unexpectedly attempting to leave the city.

The player must be able to customize which classes pause the game.

---

# 11. Organization Structure

The organization is hierarchical.

A basic hierarchy may contain:

- Boss: the player.
- Underboss or equivalent senior executive.
- Lieutenants or captains.
- Crew leaders.
- Soldiers and associates.
- Specialists.
- Non-criminal employees.

The exact historical naming matters less than the functional hierarchy.

Each manager has:

- People they directly control.
- Geographic or functional responsibility.
- A budget.
- Standing orders.
- Autonomy level.
- Loyalty and ambitions.
- Management competence.
- Personal relationships.

The player should not normally assign individual low-level workers after the early game.

A lieutenant might receive:

> Maintain gambling operations in South Ward. Keep police exposure low. Do not initiate violence without approval. Weekly discretionary budget: $2,500.

The lieutenant then makes routine staffing and operational choices.

This creates meaningful managerial selection: a capable but aggressive lieutenant behaves differently from a cautious, loyal, mediocre one.

---

# 12. Character Model

Important characters are persistent agents.

Characters have five broad layers.

## 12.1 Capabilities

Examples:

- Violence.
- Intimidation.
- Stealth.
- Burglary.
- Driving.
- Surveillance.
- Investigation.
- Accounting.
- Negotiation.
- Management.
- Political influence.
- Legal knowledge.
- Social access.

Capabilities should use broad qualitative bands in normal UI rather than false precision.

For example:

- Poor.
- Competent.
- Skilled.
- Excellent.
- Exceptional.

Exact underlying values may exist, but the player generally sees what their organization has learned.

## 12.2 Traits

Traits describe behavior rather than passive bonuses.

Examples:

- Cautious.
- Impulsive.
- Greedy.
- Proud.
- Patient.
- Cruel.
- Charismatic.
- Vindictive.
- Secretive.
- Ambitious.
- Loyal to family.
- Easily frightened.

Traits should alter decisions and relationships.

## 12.3 Needs and Interests

Characters want things.

Examples:

- Money.
- Status.
- Safety.
- Respect.
- Revenge.
- Family security.
- Political advancement.
- Independence.
- Ideological causes.

A relationship becomes strategically interesting when the player understands what another person wants.

## 12.4 Relationships

Relationships are directional.

Frank may trust Maria while Maria distrusts Frank.

Relationships can include:

- Trust.
- Respect.
- Fear.
- Affection.
- Dependence.
- Resentment.
- Debt.
- Family ties.
- Shared secrets.
- Rivalries.

The UI should emphasize notable relationships rather than exposing a complete social matrix.

## 12.5 Knowledge

Characters know specific facts.

This is crucial.

A soldier who participated in three murders knows more dangerous information than a bookkeeper who only sees sanitized financial records.

Knowledge creates organizational risk.

A character can become dangerous because they are disloyal, arrested, frightened, financially desperate, or simply too informed.

---

# 13. Loyalty, Fear, and Internal Stability

There is no single loyalty meter that determines whether a character betrays the organization.

A character's behavior should depend on interacting pressures such as:

- Personal relationship with leadership.
- Perceived fairness.
- Fear of consequences.
- Financial satisfaction.
- Ambition.
- Relationships with other members.
- Confidence that the organization is stable.
- Exposure to arrest or prosecution.
- Opportunity to defect.
- Personal grudges.
- Family pressure.

A highly frightened character may obey while secretly resenting the organization.

A respected lieutenant may remain loyal during hardship because their identity and status are tied to the organization.

A greedy but otherwise loyal manager may skim income without wanting to defect.

Internal crime should therefore emerge from motives rather than random "betrayal events."

---

# 14. Recruitment

Recruitment is primarily relational.

People enter the organization through:

- Existing members.
- Family connections.
- Neighborhood networks.
- Prison connections.
- Business relationships.
- Unions.
- Professional contacts.
- Political intermediaries.
- Criminal reputation.

The player should rarely browse an abstract global hiring list.

Recruiting a highly capable person can create new risks because they bring relationships, enemies, habits, and existing exposure.

The organization also recruits non-criminal specialists:

- Lawyers.
- Accountants.
- Doctors.
- Drivers.
- Mechanics.
- Business managers.
- Union organizers.
- Political fixers.

Some knowingly participate in crime. Others deliberately avoid knowing too much.

---

# 15. Information and Uncertainty

Information has:

- Source.
- Subject.
- Age.
- Reliability.
- Specificity.
- Chain of custody or provenance where relevant.

The player does not receive raw truth unless there is a credible reason.

Examples:

> **Informant report:** Bellmore Jewelry receives a high-value shipment every Thursday afternoon. Reliability: generally reliable.

> **Police contact:** Detectives from Central Precinct have asked about a dark Buick seen near two recent robberies. Source has direct access.

> **Street rumor:** Carlo Rosetti may be dissatisfied with his boss. Unconfirmed.

Conflicting information is allowed.

Better intelligence enables better plans, but perfect certainty should remain rare.

---

# 16. Reports as Interface

Much of the game is understood through artifacts generated by the simulation.

Possible report types:

- Daily organization brief.
- Weekly financial report.
- Surveillance report.
- Police intelligence report.
- Newspaper clipping.
- Lawyer memorandum.
- Accountant warning.
- Informant message.
- Intercepted communication.
- Medical report.
- Court notice.
- Business ledger.
- Crew after-action report.

Reports are functional UI, not decorative flavor.

A good report answers:

1. What happened?
2. Why does it matter?
3. How certain are we?
4. Which people, places, businesses, or cases are connected?
5. Does the player need to decide something?

The player can click entities mentioned in a report to inspect them.

---

# 17. Operations

An operation is a deliberate activity that requires coordination beyond routine business.

Examples:

- Burglary.
- Robbery.
- Hijacking.
- Smuggling run.
- Intimidation.
- Kidnapping.
- Surveillance.
- Sabotage.
- Bribery attempt.
- Witness pressure.
- Theft of documents.
- Illegal gambling event.
- Covert transfer of money.
- Rescue or extraction.
- Rival infiltration.

Operations use the same core planning framework rather than becoming separate minigames.

---

# 18. Operation Planning Model

The player builds a plan from semantic components.

## Objective

What outcome is desired?

Examples:

- Steal specific property.
- Obtain cash.
- Frighten a target.
- Gather evidence.
- Destroy equipment.
- Move contraband.
- Remove a person.

## Approach

Examples:

- Covert.
- Deceptive.
- Intimidating.
- Violent.
- Inside assistance.
- Opportunistic.

## Team

The player chooses a responsible leader and may choose critical specialists.

Routine personnel can be selected by the responsible manager.

## Roles

Examples:

- Driver.
- Lookout.
- Entry specialist.
- Safe specialist.
- Muscle.
- Inside contact.
- Coordinator.

## Constraints

Examples:

- Avoid casualties.
- Do not harm employees.
- Avoid firearms.
- Do not leave witnesses who can identify leadership.
- Preserve merchandise.
- Complete before a given time.
- Do not involve anyone associated with another ongoing investigation.

## Contingencies

Examples:

- Abort if police arrive before entry.
- Use force if the target resists.
- Switch to secondary exit if the primary route is blocked.
- Contact a named fixer if detained.

## Intelligence

The planning screen shows what is known and what remains uncertain.

The game should explicitly warn about important unknowns without revealing hidden truth.

Example:

> Alarm system: poorly understood.  
> Night staff: confirmed one guard, possible second guard.  
> Police response: likely fast due to nearby patrol concentration.  
> Fence: confirmed buyer for jewelry, uncertain capacity for high-value stones.

---

# 19. Operation Resolution

Once authorized, the operation proceeds under AI control.

The player does not normally intervene in routine execution.

Characters make local decisions based on:

- The plan.
- Their role.
- Their competence.
- Their personality.
- Their loyalty.
- Their understanding of the situation.
- Standing organizational policies.
- Unexpected circumstances.

The simulation may stop and request player input when an event exceeds the delegated authority established in the plan.

Example:

> 02:11. Eddie reports that the alarm panel does not match the model described by the informant. He estimates he can bypass it, but doing so will take longer and may trigger a silent alarm.

Possible decisions:

- Let Eddie improvise.
- Use destructive entry.
- Abort.
- Call the electrician contact.

The meaningful decision is preserved while mechanical choreography is automated.

---

# 20. After-Action Reports

Every significant operation produces a concise causal summary.

Example:

> **Bellmore Jewelry burglary**  
> Objective: Partial success. Approximately $18,400 in jewelry recovered.  
> Crew: No injuries.  
> Exposure: One witness saw the getaway vehicle.  
> Evidence: Rear door forced; alarm panel damaged; one glove recovered by police.  
> Delay: Safe required eleven minutes longer than expected because intelligence on the model was incorrect.  
> Notable behavior: Eddie continued after the planned abort threshold without contacting leadership.  
> Follow-up: Fence Marcus Vale can absorb roughly half the goods immediately.

The game should not merely report "success chance failed."

The player must be able to understand the causal chain.

---

# 21. Routine Criminal Enterprises

Not every illicit activity should be an operation.

Once established, many activities become ongoing enterprises:

- Protection.
- Gambling.
- Bookmaking.
- Loan sharking.
- Alcohol distribution.
- Smuggling.
- Fencing.
- Prostitution where historically/contextually appropriate.
- Labor racketeering.
- Fraud.

Each racket has:

- Local demand.
- Revenue.
- Operating cost.
- Personnel requirements.
- Dependencies.
- Exposure.
- Social consequences.
- Political consequences.
- Vulnerabilities.

The player establishes and governs the enterprise, then delegates routine operation.

The strategic questions should concern scale, location, management, tolerance for violence, suppliers, protection, and institutional exposure.

---

# 22. Economy

The economy exists independently of the player.

Businesses employ people, buy goods, sell goods, pay rent, borrow money, and fail.

Illegal enterprises depend on the legal economy.

Examples:

- Bootlegging requires supply, transport, storage, distribution, and retail outlets.
- Gambling depends on customers, venues, money handling, and protection.
- Hijacking depends on valuable shipments that are actually moving through the city.
- Extortion depends on businesses worth extorting.

This allows criminal opportunities to emerge from the economy rather than from fixed mission icons.

---

# 23. Cash, Dirty Money, and Legitimate Wealth

Money exists in meaningfully different states.

## Street Cash

Easy to spend informally but dangerous in large quantities.

## Concealed Cash

Hidden reserves with physical security risk.

## Laundered / Accounted Money

Money that can enter legitimate businesses and financial systems with reduced immediate suspicion.

## Legitimate Assets

Businesses, real estate, vehicles, and investments whose ownership may itself create records and exposure.

Money laundering should not be a single percentage conversion mechanic.

It should involve plausible business activity, financial records, accountants, ownership structures, and transaction patterns.

A cash-heavy business can absorb illicit income more easily but may still attract tax or investigative attention if its declared activity becomes implausible.

---

# 24. Legitimate Businesses

Legitimate businesses are multifunctional strategic assets.

A business can provide:

- Genuine profit.
- Employment for associates.
- Cash laundering capacity.
- Vehicles.
- Warehousing.
- Meeting space.
- Access to customers.
- Access to unions.
- Political legitimacy.
- Information.
- Cover identities.
- Distribution infrastructure.

The same business can also become a liability through:

- Financial records.
- Employee witnesses.
- Tax audits.
- Repeated association with known criminals.
- Suspicious losses.
- Physical evidence.

The player should choose businesses because of organizational function, not merely because one building has a +12% laundering statistic.

---

# 25. Territory

Territory is influence, not ownership coloring.

Control over a neighborhood may mean:

- Local businesses generally comply with demands.
- The player has informants and social access.
- Rivals need permission or protection to operate openly.
- Local political figures take the organization seriously.
- Residents know who controls the street.

Territory can be strong in one dimension and weak in another.

The player might dominate gambling in a district without controlling its union. A rival may possess political protection while the player has stronger street enforcement.

Territorial conflict therefore concerns institutions and networks, not merely map tiles.

---

# 26. Reputation

Reputation is contextual.

Different audiences maintain different impressions:

- Criminal underworld.
- Local businesses.
- Residents.
- Politicians.
- Police.
- Labor organizations.
- Wealthy elites.

Important reputation dimensions include:

- Reliability.
- Fear.
- Restraint.
- Wealth.
- Competence.
- Treachery.
- Political legitimacy.

The same action can produce different reputational effects across groups.

A brutal retaliation may increase underworld fear while reducing political willingness to associate with the player.

---

# 27. Rival Organizations

Rival groups operate under the same general simulation rules as the player.

They have:

- Leadership structures.
- Crews.
- Businesses.
- Rackets.
- Political relationships.
- Police exposure.
- Internal factions.
- Strategic preferences.

Rivals should not exist solely to attack the player.

They pursue their own survival and expansion.

Possible relationships include:

- Competition.
- Non-aggression.
- Shared markets.
- Supplier relationships.
- Temporary alliances.
- Territorial agreements.
- Personal friendships between members.
- Personal vendettas despite organizational peace.

This enables unstable underworld politics.

---

# 28. Negotiation and Deals

Many conflicts should be solvable through arrangements rather than combat.

A deal can involve:

- Territory.
- Revenue shares.
- Market access.
- Introductions.
- Protection.
- Information.
- Debt forgiveness.
- Political assistance.
- Personnel exchanges.
- Temporary neutrality.

Deals create obligations and expectations.

Breaking a deal damages reputation differently depending on who knows about it.

There should be no perfect "diplomacy score" predicting acceptance. The player uses known interests, relative power, relationships, and information.

---

# 29. Violence

Violence is effective, fast, and costly.

It can:

- Remove people.
- Intimidate businesses.
- Disrupt rivals.
- Prevent immediate threats.

But it can also create:

- Witnesses.
- Bodies.
- Injured members.
- Police mobilization.
- Public outrage.
- Political pressure.
- Retaliation.
- Internal moral or loyalty effects.
- Press attention.
- Investigative resources.

Violence should become strategically less attractive as the player's legitimate and political interests grow.

A small street gang can survive chaos that a citywide organization with judges, businesses, politicians, and banks tied to it cannot.

---

# 30. Law Enforcement Simulation

Law enforcement is divided into actual institutions.

Possible actors:

- Patrol police.
- Detectives.
- Precinct leadership.
- Vice squads.
- Municipal prosecutors.
- State authorities.
- Federal investigators.
- Tax investigators.
- Courts.
- Grand juries.

Different institutions possess different jurisdiction, evidence, priorities, resources, and information.

A local police captain may be corrupt while a federal investigator is unaffected by that relationship.

---

# 31. Patrol and Police Presence

Police have geographic deployment.

Patrol affects:

- Response time.
- Witness confidence.
- Opportunity for street crime.
- Probability of incidental observation.
- Visibility of violent disputes.

The player can learn approximate patrol patterns through observation and contacts.

Corrupting one officer should affect that officer's behavior, not magically lower global law-enforcement pressure.

Example:

> Officer DeLuca no longer reports gambling activity behind the Fulton Social Club.

That creates a local advantage and potentially a future corruption risk.

---

# 32. Investigations and Evidence

The state builds cases from specific evidence.

Evidence can include:

- Witness testimony.
- Vehicle descriptions.
- Fingerprints.
- Recovered property.
- Financial records.
- Informant statements.
- Surveillance.
- Phone or communication records appropriate to the era.
- Known associations.
- Documents.
- Ballistics.
- Patterns connecting incidents.

Evidence has properties such as:

- Which institution possesses it.
- Which case it belongs to.
- What it supports.
- Reliability.
- Legal admissibility.
- Whether the player knows it exists.

The player should often know that an investigation exists without knowing its exact contents.

---

# 33. Case Graphs

Investigations should function as evolving networks rather than linear progress bars.

Example:

Robbery A produces a vehicle description.

Robbery B produces a witness who recognizes Frank.

Police already know Frank associates with Maria.

Maria's registered garage serviced the vehicle described in Robbery A.

An investigator may connect these facts into a broader conspiracy theory.

This is substantially more interesting than adding +15 heat for each crime.

The player can attempt to disrupt individual links rather than simply paying to reduce a meter.

---

# 34. Player Knowledge of Investigations

The player learns about state activity through:

- Lawyers.
- Corrupt police.
- Court filings.
- Informants.
- Newspaper reporting.
- Surveillance of investigators.
- Arrested members.
- Political contacts.

Information should vary in detail.

Examples:

> Detectives are asking about Frank.

> A grand jury subpoenaed records from the Fulton Garage.

> Our contact believes the district attorney has a witness connecting Frank to the Bellmore burglary, but he does not know who.

This creates counterintelligence gameplay without revealing the entire case graph.

---

# 35. Arrest, Prosecution, and Courts

Arrest is not equivalent to conviction.

A character can be:

- Questioned.
- Detained.
- Charged.
- Released.
- Held on bail.
- Tried.
- Convicted.
- Acquitted.

Legal outcomes depend on:

- Evidence.
- Witnesses.
- Prosecutorial priorities.
- Legal representation.
- Court influence.
- Jury manipulation where applicable.
- Witness intimidation.
- Procedural mistakes.

The player therefore has strategic options after an arrest that do not amount to "pay fine to remove heat."

---

# 36. Informants

Informants are one of the most important systemic threats.

A character may inform because of:

- Fear of prison.
- Financial reward.
- Revenge.
- Personal resentment.
- Pressure against family.
- Existing relationship with police.
- A belief that the organization will abandon them.

The player normally does not receive a clear "informant" icon.

Suspicion emerges through inconsistencies:

- Police seem unusually informed.
- One character repeatedly avoids charges.
- Investigators ask questions they should not know to ask.
- A contact reports unusual meetings.

Counterintelligence must remain uncertain to prevent the optimal strategy from becoming automatic execution of flagged traitors.

---

# 37. Politics

Political influence operates through people and institutions.

Potential targets include:

- Ward leaders.
- Aldermen.
- Mayoral staff.
- Police commissioners.
- Licensing boards.
- Inspectors.
- Judges.

Influence can come from:

- Money.
- Votes.
- Labor support.
- Business relationships.
- Blackmail.
- Friendship.
- Family ties.
- Reciprocal favors.

Political power should become an alternative to raw violence rather than another resource bar.

---

# 38. Press and Public Attention

Newspapers act as information systems and political amplifiers.

Press attention can:

- Increase pressure on police.
- Damage politicians connected to the organization.
- Turn obscure violence into a citywide issue.
- Reveal facts to rivals.
- Affect juror attitudes.
- Increase the value of public legitimacy.

The press does not need perfect information.

A false or incomplete story can still change behavior.

---

# 39. Labor and Unions

Labor organizations provide a major bridge between street power, business, politics, and logistics.

Control or influence over a union can provide:

- Work stoppages.
- Access to facilities.
- Information about shipments.
- Political leverage.
- Employment opportunities.
- Legal revenue.
- Concealment.

Union activity should be relational and institutional rather than represented by a generic building upgrade.

---

# 40. Organizational Policies

The player can establish standing policies so routine situations do not require repeated decisions.

Examples:

- Maximum violence permitted during collections.
- Whether crews may bribe patrol officers without approval.
- When managers may recruit independently.
- How much discretionary spending lieutenants receive.
- Whether failed collections should be retried, escalated, or referred upward.
- Whether operations automatically abort after casualties.
- Whether arrested low-level associates automatically receive legal support.

Policies convert repeated decisions into governance.

The player can override them when desired.

---

# 41. Delegated Autonomy

Each manager can have an autonomy level.

Low autonomy means more exception requests and tighter control.

High autonomy means fewer interruptions but more risk that the manager acts according to their own judgment.

Autonomy should interact with personality and competence.

A highly competent, loyal lieutenant is valuable because the player can safely give them broad authority.

An aggressive lieutenant with too much autonomy may start unnecessary wars.

A cautious lieutenant may allow opportunities to pass.

This makes delegation itself a strategic decision.

---

# 42. Financial Reporting

The player receives layered financial information.

At the highest level:

- Total liquid cash.
- Legitimate income.
- Illicit income.
- Major expenses.
- Debt.
- Suspicious variances.

The player can drill down by:

- Territory.
- Business.
- Lieutenant.
- Racket.

The normal game should never require manually reconciling dozens of line items.

Accountants can detect anomalies such as:

- Skimming.
- Poor management.
- Implausible laundering.
- Unprofitable fronts.

Their conclusions are information, not automatic truth.

---

# 43. Skimming and Internal Theft

Managers can steal from the organization.

Detection depends on:

- Accounting quality.
- Scale of theft.
- Complexity of operations.
- Character competence.
- Internal relationships.

A discrepancy does not automatically reveal the culprit.

The player may respond by:

- Auditing.
- Reassigning responsibility.
- Confronting someone.
- Increasing oversight.
- Secretly investigating.
- Tolerating theft from a strategically valuable lieutenant.

This creates internal politics rather than a binary corruption event.

---

# 44. Progression

Progression occurs primarily through organizational capability.

The player gains access to qualitatively different actions because they acquire:

- Trusted managers.
- Specialists.
- Capital.
- Political relationships.
- Legitimate businesses.
- Supply networks.
- Information sources.
- Territory.
- Reputation.

Progression should not primarily rely on abstract technology trees.

New capabilities should usually have an in-world source.

Example:

The organization becomes capable of sophisticated financial fraud because it recruited an accountant willing to design the scheme, not because the player purchased "Fraud II."

---

# 45. Early Game

The early game is personal and constrained.

The player has:

- Few people.
- Little cash.
- Minimal political protection.
- Narrow geographic reach.
- Weak intelligence.

The player performs more direct assignments because the organization cannot delegate effectively yet.

Early-game crimes should be understandable and local.

Failure should create complications rather than immediate campaign loss.

---

# 46. Mid Game

The organization has multiple crews and meaningful territory.

The player begins shifting from direct operations toward management.

Core problems become:

- Choosing managers.
- Preventing internal instability.
- Balancing visible expansion against law-enforcement attention.
- Maintaining relations with rivals.
- Integrating legitimate businesses.
- Managing multiple investigative threats.

The player should feel that the game has changed because their organization has changed.

---

# 47. Late Game

Late game is institutional.

The player may control substantial criminal markets but face:

- Federal investigations.
- Political reform campaigns.
- Powerful rival coalitions.
- Internal succession problems.
- Economic transitions.
- Public scandals.
- Large legitimate interests vulnerable to exposure.

Late-game strategy concerns what kind of organization the player wants to become.

---

# 48. Dynamic Historical Pressure

The world should change over time.

For the proposed setting, the most important structural event is the end of Prohibition.

An organization built almost entirely around alcohol should face disruption.

Possible responses include:

- Move into gambling.
- Expand labor racketeering.
- Enter legitimate alcohol distribution.
- Use existing transport networks for other contraband.
- Invest in legal businesses.
- Exploit political contacts to shape licensing.

This ensures that success does not mean solving one optimal economy and repeating it forever.

---

# 49. Campaign Objectives

The game should avoid tightly scripted missions as the primary structure.

Campaign pressure should usually specify a *problem*, not a solution.

Examples:

- Raise $50,000 before a debt comes due.
- Prevent a rival from dominating the docks.
- Keep a political ally in office through the next election.
- Break an investigation before it produces an indictment.
- Replace declining bootlegging revenue after repeal.
- Recover from the arrest of a senior lieutenant.

The simulation should provide multiple viable responses.

---

# 50. End States

There should be several forms of success.

Possible strategic conclusions include:

## Underworld Dominance

The player becomes the city's undisputed criminal power.

## Political Capture

The organization gains enough institutional influence that direct criminal control becomes secondary.

## Legitimate Transformation

The player converts enough wealth and power into legitimate businesses and political capital to withdraw from high-risk crime.

## Quiet Hegemony

The organization remains criminal but deliberately reduces visibility, violence, and expansion in exchange for long-term stability.

These should not be simple buttons. They represent different strategic paths requiring different forms of power.

---

# 51. Failure States

Failure should usually be gradual.

Possible collapse mechanisms include:

- Financial insolvency.
- Leadership conviction.
- Internal fragmentation.
- Rival takeover.
- Loss of critical political protection.
- Organization-wide conspiracy prosecution.
- Loss of trusted managers leading to uncontrollable operations.

The player should often be able to survive serious setbacks by shrinking, reorganizing, sacrificing assets, or negotiating.

A disastrous year can create a different game rather than an immediate game-over screen.

---

# 52. Difficulty

Difficulty should primarily alter pressure and forgiveness, not hide more information.

Possible difficulty dimensions:

- Rival competence.
- Police resourcefulness.
- Economic scarcity.
- Loyalty pressures.
- Speed at which institutions react to visible crime.
- Severity of legal consequences.

Higher difficulty should not make the UI less informative.

The player still needs causal feedback to learn.

---

# 53. UI Principles

## 53.1 Explain Causality

After important outcomes, the player must be able to determine:

- What happened.
- What factors contributed.
- What was uncertain.
- What information was missing.
- What choices could plausibly have changed the result.

## 53.2 Hide Precision, Not Meaning

Prefer:

> Police response likely 3-6 minutes.

Over:

> Police response = 41.7% risk.

But never replace useful information with vague mystery.

## 53.3 Progressive Disclosure

Show the most important information first.

Allow the player to drill down when needed.

## 53.4 Entity-Centered Navigation

People, businesses, cases, neighborhoods, and organizations should be clickable wherever they appear.

A player reading a report about a detective should be one action away from that detective's known profile.

## 53.5 Batch Routine Actions

If the player performs the same harmless action repeatedly, the interface should support applying it to a group, creating a standing policy, or delegating it.

## 53.6 No Unexplained Failure

Orders may fail because characters refuse, circumstances change, information was wrong, resources were insufficient, or the assigned manager made a bad decision.

The player should receive the reason when the organization could reasonably know it.

---

# 54. Main Strategic Views

The exact final interface is open, but the game needs conceptual views for:

## City View

Geography, known activities, institutions, businesses, neighborhoods, and operational context.

## Organization View

Hierarchy, responsibilities, personnel, loyalty concerns, and management load.

## Intelligence View

Reports, unresolved questions, known relationships, active surveillance, and information sources.

## Legal / Investigation View

Known cases, arrests, hearings, suspected evidence, legal resources, and law-enforcement actors.

## Financial View

High-level finances, business performance, illicit revenue, suspicious anomalies, and cash position.

## Operations View

Planning, active operations, contingencies, and after-action reports.

These views must connect through shared entities rather than behave like separate games.

---

# 55. Alerts and Summaries

The game should produce a morning or periodic executive brief containing only items that matter at the player's current authority level.

Example:

> **Executive Brief — May 14**  
> South Ward gambling revenue is 18% below recent expectations. Carlo claims police pressure is responsible. Accountant believes collections are also being underreported.  
>  
> Detective Harlan questioned two employees of Fulton Garage yesterday. Our source does not know what he asked.  
>  
> Rosetti organization requested a meeting concerning dock access.  
>  
> Bellmore burglary proceeds have been partially fenced. Remaining stones are too recognizable for Marcus Vale's normal buyers.  
>  
> No immediate decision required on routine operations.

This should be more useful than dozens of real-time notifications.

---

# 56. Emergent Storytelling

The game should generate stories through persistent causality rather than relying primarily on scripted event chains.

A strong emergent sequence might look like this:

1. The player hires a talented but ambitious lieutenant.
2. The lieutenant builds a profitable gambling network.
3. Their growing independence makes them popular with local soldiers.
4. An accountant detects missing revenue.
5. The player investigates quietly rather than confronting them.
6. Police unexpectedly raid one of the lieutenant's properties.
7. The lieutenant believes the player deliberately withheld protection.
8. A rival offers the lieutenant support.
9. The player must decide whether to reconcile, demote, isolate, prosecute internally, or eliminate them.

None of these steps requires a prewritten "betrayal quest."

The simulation creates the conflict because money, relationships, fear, institutional pressure, and ambition intersect.

---

# 57. Content Design Rules

Authored content should add specificity without overriding the simulation.

Good authored content:

- Distinctive businesses.
- Neighborhood histories.
- Character backgrounds.
- Political controversies.
- Newspaper styles.
- Local institutions.
- Special criminal opportunities.
- Major historical transitions.

Bad authored content:

- Missions requiring one exact solution.
- Characters immune to simulation rules because the plot needs them.
- Scripted betrayals regardless of relationship state.
- Forced wars.
- Artificial resource gates unrelated to the world.

The simulation provides structure. Authored content provides identity.

---

# 58. Procedural Generation

Procedural systems should generate situations, not interchangeable filler.

Useful procedural outputs include:

- Character networks.
- Business ownership relationships.
- Financial vulnerabilities.
- Criminal opportunities.
- Rival strategies.
- Investigative hypotheses.
- Rumors.
- Social conflicts.

Generated content should always connect to persistent entities.

A generated opportunity is more interesting when it exists because:

> A warehouse with expensive goods is poorly protected because its owner recently cut costs after losing a major contract.

rather than:

> Random burglary mission spawned.

---

# 59. Anti-Grind Rules

Whenever a mechanic becomes routine, one of four things should happen:

1. It becomes automated.
2. It becomes delegable.
3. It becomes summarized.
4. The decision changes qualitatively.

Examples:

The first protection racket may require selecting collectors and deciding how to respond to refusal.

After establishing dozens of protected businesses, ordinary collection is automatic under a lieutenant.

The player only sees exceptions such as refusal, missing money, police interference, or politically sensitive targets.

The game must repeatedly graduate the player out of solved problems.

---

# 60. Anti-Min game Rule

A criminal activity should not become a separate mechanical genre merely because it is colorful.

Safecracking, surveillance, intimidation, bribery, smuggling, and robbery should use common systems:

- Character capability.
- Information.
- Equipment.
- Relationships.
- Time.
- Risk.
- Planning.
- Consequence.

A unique interface is justified only when it expresses a unique strategic decision, not to simulate manual dexterity.

---

# 61. Anti-Omniscience Rule

The simulation may know exact values. The player usually should not.

The player receives estimates based on information quality.

However, uncertainty must be actionable.

Bad uncertainty:

> Something may happen.

Good uncertainty:

> We have confirmed one night guard. The building manager claims there are two, but our surveillance has never seen the second.

The latter creates a planning decision.

---

# 62. Anti-Micromanagement Rule

The game should constantly ask:

> Is this a decision the head of this organization should still be making at this scale?

If not, move it down the hierarchy.

The player can inspect details but should not be required to operate at low levels merely because the simulation models them.

---

# 63. Anti-Heat-Meter Rule

No single number should summarize the state's response to crime.

Instead track actual consequences:

- Patrol intensity.
- Active investigations.
- Evidence.
- Witnesses.
- Political pressure.
- Press attention.
- Prosecutorial interest.
- Known suspects.
- Institutional corruption.

The player may receive a high-level assessment such as:

> Law-enforcement pressure: rising.

But this is a summary derived from real systems, not the system itself.

---

# 64. Anti-Scripted-Solution Rule

Scenario design should impose constraints and consequences rather than predetermined answers.

Bad objective:

> Rob First National Bank using the sewer entrance.

Good pressure:

> The organization owes $60,000 within six days. Failure will transfer three profitable properties to the lender.

The player can solve the problem through crime, finance, negotiation, liquidation, blackmail, or another emergent method.

---

# 65. Anti-Expansion-Punishment Rule

Expansion may create strategic complexity, but it must not simply create more chores.

A ten-person crew requires direct supervision.

A hundred-person organization requires management systems.

A citywide organization requires institutional strategy.

The player's workload should remain roughly stable while the *importance* and *scope* of decisions increase.

---

# 66. Example Full Gameplay Sequence

The following illustrates how the major systems should connect.

The organization learns from a bartender that Bellmore Jewelry receives imported stones on Thursdays.

The information is recorded as an unverified opportunity.

The player orders low-cost surveillance.

Two days later, surveillance confirms an armored delivery Thursday afternoon and notes one night guard. The crew cannot determine the alarm model.

The player could gather more intelligence, but additional surveillance increases the chance of being noticed and delays the operation.

The player decides current intelligence is sufficient.

A lieutenant proposes Eddie for alarms, Frank for entry, and Maria as driver. The player approves the team but sets an explicit no-casualty constraint because the organization is currently cultivating a reform-minded alderman.

The operation begins Thursday night.

Inside, Eddie discovers the alarm differs from the informant's description. The system requests guidance because continuing will exceed the plan's risk threshold.

The player authorizes improvisation.

The crew succeeds but needs eleven additional minutes.

During that delay, a delivery driver sees the getaway car leaving the alley.

Police receive a dark-Buick description.

The player receives an after-action report noting the witness and incorrect alarm intelligence.

The loot cannot be sold immediately. The preferred fence warns that several stones are identifiable.

The player sends most ordinary jewelry through the fence and stores the distinctive stones.

Three weeks later, another crew uses the same Buick in an unrelated hijacking because its manager is unaware of the Bellmore evidence.

A detective notices that witnesses in both cases described similar vehicles.

The detective checks garages associated with known burglary suspects and eventually asks questions at Fulton Garage, which is partly owned by Maria's brother.

A corrupt officer informs the organization that detectives are asking about Maria.

The player still does not know exactly what evidence they have.

Possible responses include:

- Move Maria out of the city temporarily.
- Replace the vehicle and destroy records.
- Order surveillance on the detective.
- Ask the lawyer to determine whether subpoenas have been issued.
- Pressure the original witness.
- Do nothing, avoiding suspicious overreaction.

Meanwhile, the lieutenant running the burglary crew complains that leadership is restricting profitable activity because of one vehicle description.

The case has now become an organizational and political problem rather than a completed mission.

That is the target experience.

---

# 67. Player Learning

The tutorial must teach systems through causality rather than encyclopedic instruction.

The player should learn one organizational concept at a time:

- Information quality.
- Delegation.
- Evidence.
- Relationships.
- Legitimate fronts.
- Political influence.

Early consequences should be forgiving enough that players can understand mistakes.

The interface should surface why an outcome occurred.

Depth should be discovered through interaction, not hidden in a manual.

---

# 68. Difficulty of Knowledge Versus Difficulty of Control

The game should be difficult because the player lacks certainty, faces conflicting incentives, and cannot satisfy every stakeholder.

It should not be difficult because:

- Commands are buried.
- Reports are unreadable.
- Critical information is omitted.
- Characters silently refuse orders.
- The player must memorize undocumented rules.

The world can be opaque.

The interface cannot.

---

# 69. Desired Emotional Rhythm

The game's rhythm should alternate among:

- Curiosity: What is happening?
- Calculation: What can we exploit?
- Commitment: Authorize the plan.
- Anticipation: Will it work?
- Surprise: Something changed.
- Interpretation: Why?
- Consequence: What does this create?
- Recovery or escalation: What do we do now?

The game should avoid long stretches of pure maintenance.

---

# 70. Strategic Tensions

The strongest decisions should involve tensions rather than obvious upgrades.

Examples:

## Profit vs Exposure

The most profitable operation may create unacceptable evidence.

## Loyalty vs Competence

A trusted manager may be mediocre. A brilliant manager may be dangerous.

## Fear vs Legitimacy

Violence can solve immediate problems while damaging political relationships.

## Centralization vs Autonomy

Tight control reduces unauthorized behavior but overwhelms leadership.

## Growth vs Stability

Expansion increases revenue and influence but exposes the organization to new institutions and internal factions.

## Secrecy vs Coordination

Compartmentalization limits investigative damage but makes operations less efficient.

## Short-Term Cash vs Long-Term Position

A valuable legitimate business may be worth keeping despite poor immediate returns.

---

# 71. Compartmentalization

As the organization grows, the player can control how information flows internally.

A compartmentalized structure means fewer people understand the entire organization.

Benefits:

- Reduced damage from informants.
- Fewer evidentiary links.

Costs:

- Worse coordination.
- More duplication.
- Greater dependence on intermediaries.
- Higher chance that managers act without important context.

This creates an organizational response to investigations more interesting than simply bribing officials.

---

# 72. Succession and Leadership Risk

Senior characters accumulate real power.

A lieutenant controlling several crews, businesses, and neighborhood relationships cannot be replaced without consequence.

Removing them may cause:

- Lost revenue.
- Defections.
- Rival recruitment.
- Operational confusion.
- Exposure of secrets.

Therefore, the player should cultivate redundant relationships and possible successors.

Late-game organizational resilience becomes a strategic capability.

---

# 73. Memory and History

The game should preserve a readable organizational history.

Important past events remain attached to relevant entities:

- Arrests.
- Betrayals.
- Business acquisitions.
- Major operations.
- Political favors.
- Killings.
- Promotions.
- Investigations.

This supports both player comprehension and emergent narrative.

A character's resentment should be understandable because the UI can show the decisions that produced it.

---

# 74. Save-Friendly Complexity

Returning to a campaign after several days should be practical.

The game should provide a "since you last played" or campaign-state summary containing:

- Current strategic problems.
- Active investigations.
- Major relationships.
- Pending operations.
- Financial condition.
- Recent significant events.

Deep simulation must not depend on perfect player memory.

---

# 75. Accessibility of Information

Important information should have more than one presentation when practical.

Examples:

- Geographic overlay.
- Text report.
- Entity profile.
- Timeline.

Color should not be the sole carrier of meaning.

Dense systems need robust filtering and search.

---

# 76. Metrics for Evaluating Features

Every proposed feature should be evaluated with the following questions:

1. What decision does this create?
2. What other system does it interact with?
3. Can the player understand its consequences?
4. Does it still create interesting decisions after ten hours?
5. Does scale turn it into repetitive work?
6. Can routine use be delegated or automated?
7. Does it produce persistent consequences rather than isolated rewards?
8. Does it reinforce the fantasy of running an organization?

A feature that fails several of these questions should probably be removed or redesigned.

---

# 77. Minimum Viable Game Experience

A viable first complete gameplay experience does not need every institution in the final design.

It does need the systems that prove the central thesis.

A minimal but representative game should include:

- One detailed city district or small city.
- Persistent characters with relationships and knowledge.
- One player organization and at least two rival groups.
- Legitimate and illicit businesses.
- Delegated crews.
- Protection, gambling, burglary, and one contraband market.
- Semantic operation planning.
- Patrol police.
- Detective investigations based on specific evidence.
- Arrest and legal representation.
- Informants.
- Reports and executive summaries.
- A basic political contact system.
- Organizational policies and autonomy.

If these systems produce interesting emergent stories, additional rackets, institutions, and historical content can expand the simulation without changing its foundation.

---

# 78. Core Design Test Scenario

One scenario should be used throughout development to test whether the game's systems remain coherent.

**Scenario:** A profitable crew uses violence during an unauthorized collection. A civilian witness identifies one member. Police begin investigating. The crew's lieutenant hides the seriousness of the incident because they fear losing autonomy. A rival learns of the problem and attempts to recruit one frightened associate. The player has a politically sensitive legitimate business in the same district.

A successful design should allow several plausible responses without special scripting:

- Replace or discipline the lieutenant.
- Protect the witness from rival manipulation while avoiding direct intimidation.
- Use legal pressure.
- Learn what police actually know.
- Sacrifice the implicated member.
- Move the crew.
- Negotiate with the rival.
- Temporarily reduce visible activity.
- Exploit political contacts.
- Accept the legal risk and continue operating.

The game should then produce understandable consequences from whichever combination the player chooses.

If the scenario instead collapses into "pay money to reduce heat" or "win tactical battle," the design has failed its central thesis.

---

# 79. Design Identity

The finished game should be recognizable by five characteristics.

First, **the player gives orders at the level of intent** while the simulation handles ordinary execution.

Second, **information is incomplete but structured**, and better intelligence materially changes decisions.

Third, **characters carry relationships, motives, and knowledge**, making people more important than abstract units.

Fourth, **crime creates persistent institutional consequences**, especially investigations built from actual evidence rather than a universal heat meter.

Fifth, **organizational growth changes the player's job**, moving from direct control toward delegation, policy, politics, and institutional strategy.

These qualities are more important than any particular list of crimes, setting details, or amount of simulated content.

---

# 80. Final Design Principle

The game should simulate more than it asks the player to control.

That asymmetry is the foundation of the design.

The simulation exists to produce situations worth thinking about. The interface exists to reveal the parts the player's organization could reasonably understand. Delegation exists to prevent scale from turning complexity into clerical work. Persistent characters and institutions exist so actions accumulate history rather than disappearing when a mission ends.

The player should finish a session remembering decisions and consequences:

- the lieutenant they trusted too long,
- the business they kept alive because it controlled a useful union relationship,
- the robbery that accidentally connected two investigations,
- the detective they underestimated,
- the politician whose protection became a liability,
- the rival they could have killed but instead made indispensable,
- the organization they built and then had to learn how to control.

That is the game.
