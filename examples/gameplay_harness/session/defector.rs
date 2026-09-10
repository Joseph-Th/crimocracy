//! Player-visible defector tracing and win-back policy for full harness sessions.

use super::*;

/// Player-earned counter-intelligence after an accepted defection. The organization watches every
/// known rival through canonical surveillance to confirm where the departed member resurfaces.
pub(super) fn run_defector_trail(
    scenario: &mut Scenario,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    let Some(defector) = metrics.defector else {
        return Ok(());
    };
    let defector_name = scenario
        .state
        .world()
        .get_character(defector)
        .expect("departed character must persist")
        .name()
        .to_owned();
    let player_name = scenario
        .state
        .world()
        .get_organization(scenario.player)
        .expect("player organization must persist")
        .name()
        .to_owned();
    if narrative {
        let departed_at = metrics
            .defection_minute
            .map(|minute| format!(" at minute {minute}"))
            .unwrap_or_default();
        println!(
            "[DECIDE]  {player_name} knows {defector_name} left{departed_at}. Watch the district's known rivals for where a defector resurfaces."
        );
    }
    let known_rivals = [scenario.rival, scenario.second_rival];
    let start_index = bounded_policy_choice(scenario.seeds.policy, 0x0DEF, 2) as usize;
    let watch_order = [known_rivals[start_index], known_rivals[1 - start_index]];
    if narrative {
        let first_watched = scenario
            .state
            .world()
            .get_organization(watch_order[0])
            .expect("first watched rival must persist")
            .name()
            .to_owned();
        println!("[DECIDE]  Start the watch with {first_watched}.");
    }
    let mut resurfaced_at: Option<OrganizationId> = None;
    for rival in watch_order {
        let rival_name = scenario
            .state
            .world()
            .get_organization(rival)
            .expect("known rival must persist")
            .name()
            .to_owned();
        let title = format!("{rival_name} personnel watch");
        let scheduled_for = scenario.state.now() + SimDuration::from_minutes(30);
        let operation = authorize_surveillance_target(
            scenario,
            EntityRef::Organization(rival),
            &title,
            scheduled_for,
        )?;
        run_until_operation_terminal(scenario, operation, narrative, metrics)?;
        let resolution = scenario
            .state
            .operations()
            .get_operation(operation)
            .expect("personnel watch must persist")
            .resolution()
            .expect("completed personnel watch must have a resolution");
        let found = resolution
            .discovered_information()
            .iter()
            .any(|information| {
                scenario
                    .state
                    .intelligence()
                    .get_information(*information)
                    .is_some_and(|record| {
                        record.topic() == InformationTopic::Personnel
                            && record.subject() == EntityRef::Organization(rival)
                            && matches!(
                                record.signal(),
                                Some(InformationSignal::PersonnelPresence { characters })
                                    if characters.contains(&defector)
                            )
                    })
            });
        if found {
            resurfaced_at = Some(rival);
        }
        if narrative {
            for information in resolution.discovered_information() {
                let record = scenario
                    .state
                    .intelligence()
                    .get_information(*information)
                    .expect("personnel-watch information must persist");
                if record.topic() == InformationTopic::Personnel
                    && record.subject() == EntityRef::Organization(rival)
                {
                    println!(
                        "[LEARN]   {:?} / {:?}: {}",
                        record.reliability(),
                        record.specificity(),
                        record.summary()
                    );
                }
            }
            if found {
                println!(
                    "[VERIFY DEFECTOR] {defector_name} now appears among {rival_name}'s recurring personnel."
                );
            } else {
                println!(
                    "[VERIFY DEFECTOR] {rival_name}'s watch shows {defector_name} is not working there."
                );
            }
        }
    }
    metrics.defector_trail_confirmed = Some(resurfaced_at.is_some());
    if narrative {
        match resurfaced_at {
            Some(rival) => {
                let rival_name = scenario
                    .state
                    .world()
                    .get_organization(rival)
                    .expect("confirmed rival must persist")
                    .name()
                    .to_owned();
                println!(
                    "[VERIFY DEFECTOR] {defector_name} resurfaces among {rival_name}'s personnel. {player_name} confirmed through its own surveillance where its former member landed."
                );
            }
            None => println!(
                "[VERIFY DEFECTOR] None of the watched rivals showed {defector_name}; the personnel watch did not directly confirm where the member landed."
            ),
        }
    }
    Ok(())
}

/// One personal re-approach through canonical executive recruitment after the player's own trail
/// confirms a defector. A refusal can leak the recruiter to the rival through production rules.
pub(super) fn run_win_back_attempt(
    scenario: &mut Scenario,
    narrative: bool,
    metrics: &mut RunMetrics,
) -> Result<(), Box<dyn Error>> {
    let Some(defector) = metrics.defector else {
        return Ok(());
    };
    let defector_name = scenario
        .state
        .world()
        .get_character(defector)
        .expect("departed character must persist")
        .name()
        .to_owned();
    let boss_name = scenario
        .state
        .world()
        .get_character(scenario.boss)
        .expect("boss must persist")
        .name()
        .to_owned();
    let player_name = scenario
        .state
        .world()
        .get_organization(scenario.player)
        .expect("player organization must persist")
        .name()
        .to_owned();
    let rival_name = scenario
        .state
        .world()
        .get_organization(scenario.rival)
        .expect("rival organization must persist")
        .name()
        .to_owned();
    let approach = match bounded_policy_choice(scenario.seeds.policy, 0x5EED, 2) {
        0 => RecruitmentApproach::PersonalAppeal,
        _ => RecruitmentApproach::FinancialOpportunity,
    };
    if narrative {
        println!(
            "[DECIDE]  {boss_name} makes one {approach:?} pitch to {defector_name}: come home to {player_name}."
        );
    }
    let attempt = validate_recruitment_attempt(
        scenario.registry,
        &scenario.state,
        RecruitmentDraft {
            target_organization: scenario.player,
            recruiter: scenario.boss,
            candidate: defector,
            approach,
        },
    )?
    .commit(&mut scenario.state)?;
    let record = scenario
        .state
        .recruitment()
        .get_attempt(attempt)
        .expect("committed win-back attempt must be queryable");
    let accepted = record.outcome() == RecruitmentOutcome::Accepted;
    metrics.win_back_attempted = true;
    metrics.win_back_accepted = Some(accepted);
    metrics.win_back_margin = Some(record.margin());
    if narrative {
        println!(
            "[NARRATION] The {:?} pitch resolves against {boss_name}'s old bond, {defector_name}'s fresh attachment to {}, and ordinary membership resistance.",
            record.approach(),
            rival_name,
        );
    }
    if accepted {
        let membership = scenario
            .state
            .world()
            .get_character(defector)
            .expect("defector must persist")
            .organization();
        debug_assert_eq!(
            membership,
            Some(scenario.player),
            "an accepted win-back must move membership back through the canonical reassignment"
        );
        if narrative {
            println!(
                "[WIN BACK]  {defector_name} came home to {player_name}. Membership moved through the production reassignment path; the crew that left in fear is whole again - and both organizations now know exactly how much his loyalty is worth."
            );
        }
        return Ok(());
    }
    let leaked = record
        .member_report()
        .and_then(|report| scenario.state.reports().get_report(report))
        .is_some_and(|report| {
            report.recipient() == scenario.rival
                && report.kind() == ReportKind::AfterAction
                && report.entries().len() == 1
                && report.entries()[0]
                    .entities
                    .contains(&EntityRef::Character(scenario.boss))
        });
    metrics.win_back_refusal_leaked_to_rival = Some(leaked);
    if narrative {
        println!(
            "[WIN BACK]  {defector_name} stayed with {rival_name}. The re-approach failed and membership did not move."
        );
    }
    Ok(())
}
