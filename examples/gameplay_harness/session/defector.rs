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
            .map(|minute| format!(" at {}", stamp(minute)))
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
            BTreeSet::new(),
        )?;
        run_until_operation_terminal(scenario, operation, narrative, metrics)?;
        if observe_defector_watch(scenario, operation, rival, defector, narrative) {
            resurfaced_at = Some(rival);
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
                "[VERIFY DEFECTOR] The personnel watches did not confirm where {defector_name} landed; failed or aborted observation does not establish the member's absence."
            ),
        }
    }
    Ok(())
}

/// A terminal watch confirms only typed personnel observations held by the player. An abort
/// supplies no such observation; it does not establish that the defector is absent from the rival.
fn observe_defector_watch(
    scenario: &Scenario,
    operation: OperationId,
    rival: OrganizationId,
    defector: crimocracy::core::id::CharacterId,
    narrative: bool,
) -> bool {
    let watch = scenario
        .state
        .operations()
        .get_operation(operation)
        .expect("personnel watch must persist");
    if watch.status() == OperationStatus::Aborted {
        if narrative {
            println!(
                "[LEARN]   {} was aborted; it did not confirm where the departed member landed.",
                watch.title()
            );
        }
        return false;
    }
    let resolution = watch
        .resolution()
        .expect("completed personnel watch must have a resolution");
    let mut found = false;
    for information in resolution.discovered_information() {
        let record = scenario
            .state
            .intelligence()
            .get_information(*information)
            .expect("personnel-watch information must persist");
        if record.holder() != KnowledgeHolder::Organization(scenario.player) {
            continue;
        }
        if record.topic() == InformationTopic::Personnel
            && record.subject() == EntityRef::Organization(rival)
            && matches!(
                record.signal(),
                Some(InformationSignal::PersonnelPresence { characters })
                    if characters.contains(&defector)
            )
        {
            found = true;
        }
        if narrative
            && ((record.topic() == InformationTopic::Personnel
                && record.subject() == EntityRef::Organization(rival))
                || (record.topic() == InformationTopic::EnterpriseActivity
                    && matches!(record.subject(), EntityRef::Enterprise(_))))
        {
            println!(
                "[LEARN]   {:?} / {:?}: {}",
                record.reliability(),
                record.specificity(),
                record.summary()
            );
        }
    }
    found
}

/// One personal re-approach through canonical executive recruitment after the player's own trail
/// confirms a defector. A refusal can leak the recruiter to the rival through production rules.
///
/// The organization has confirmed where its former member went, but it has no player-held
/// intelligence that reveals the defector's latent drives or traits. Leadership therefore makes
/// the one pitch justified by information it actually possesses: a personal appeal grounded in
/// the defector's established relationship with the boss. Private motives still affect the
/// production recruitment outcome, but never the harness policy that chooses the pitch.
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
    let approach = RecruitmentApproach::PersonalAppeal;
    if narrative {
        println!(
            "[DECIDE]  {boss_name} makes one {approach:?} pitch to {defector_name}: come home to {player_name}. Leadership leans on their established personal bond rather than pretending to know the defector's private motives."
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
            "[NARRATION] Leadership made a {:?} pitch from the relationship it can actually act on. It sees acceptance or refusal, not the scoring margin or the defector's private motives.",
            record.approach(),
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
                "[WIN BACK]  {defector_name} accepted the offer and rejoined {player_name}. This confirms the return, not a guarantee of future loyalty or what the rival knows."
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
        println!("[WIN BACK]  Leadership cannot know what the rival was told about the approach.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crimocracy::operations::operation_system::{OperationTransition, apply_transition};

    #[test]
    fn defector_watch_is_unconfirmed_when_canonical_watch_aborts() {
        let registry = crimocracy::build_registry();
        let mut scenario = build_scenario(
            &registry,
            EvaluationSeeds::defaults(),
            ScenarioProfile::NightTrap,
        )
        .unwrap();
        let rival = scenario.rival;
        let scheduled_for = scenario.state.now() + SimDuration::from_minutes(30);
        let watch = authorize_surveillance_target(
            &mut scenario,
            EntityRef::Organization(rival),
            "Aborted personnel watch",
            scheduled_for,
            BTreeSet::new(),
        )
        .unwrap();
        apply_transition(
            &registry,
            &mut scenario.state,
            watch,
            OperationTransition::Abort,
        )
        .unwrap();
        let record = scenario.state.operations().get_operation(watch).unwrap();
        assert_eq!(record.status(), OperationStatus::Aborted);
        assert!(record.resolution().is_none());
        let before = bincode::serialize(&scenario.state).unwrap();
        let mut metrics = RunMetrics::default();
        run_until_operation_terminal(&mut scenario, watch, false, &mut metrics).unwrap();
        for narrative in [false, true] {
            assert!(!observe_defector_watch(
                &scenario,
                watch,
                rival,
                scenario.burglar,
                narrative,
            ));
        }
        assert!(!metrics.win_back_attempted);
        assert_eq!(bincode::serialize(&scenario.state).unwrap(), before);
    }
}
