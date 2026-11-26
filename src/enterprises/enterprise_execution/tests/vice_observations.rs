//! End-to-end vice surveillance through actual settlement, custody, and referral.
use super::*;
use crate::core::time::DAY_DURATION;
use crate::legal::investigation_system::{InvestigationError, apply_enterprise_observations};
use crate::legal::prosecution_system::member_prosecution_under_review;
use crate::world::world_system::designate_player_organization;

#[test]
fn established_public_business_network_draws_more_vice_scrutiny_without_fabricating_evidence() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    let EnterpriseLocation::Neighborhood(district) = fixture.location else {
        unreachable!()
    };
    insert_district_police(&registry, &mut fixture, "Prominence Vice Bureau", district);
    let base_chance = registry
        .get_enterprise(EnterpriseKind::Protection)
        .economics()
        .enforcement_attention_basis_points_per_exposure_source();
    let mut quiet = fixture.state.clone();
    quiet.advance_clock(DAY_DURATION);
    let quiet_plan = decide_enterprise_cycle(
        &registry,
        &quiet,
        enterprise,
        EnterpriseCycleRandomness::new(0, base_chance),
    )
    .unwrap();
    assert!(quiet_plan.enforcement_incident.is_none());

    // The low public footprint tiers do not multiply vice risk; a genuinely
    // city-significant collection of storefronts still raises inquiry odds.
    for index in 0..13 {
        let owner = fixture.organization;
        insert_support_business(
            &registry,
            &mut fixture,
            &format!("Public Chain Store {index}"),
            BusinessKind::Retail,
            BTreeSet::from([
                BusinessFunction::CustomerAccess,
                BusinessFunction::ProfessionalRecords,
            ]),
            BusinessOwner::Organization(owner),
        );
    }
    assert!(
        crate::world::organization_pressure::public_prominence_tier(
            &fixture.state,
            fixture.organization
        ) >= 3
    );
    fixture.state.advance_clock(DAY_DURATION);
    let exposed = decide_enterprise_cycle(
        &registry,
        &fixture.state,
        enterprise,
        EnterpriseCycleRandomness::new(0, base_chance),
    )
    .unwrap();
    assert!(exposed.enforcement_incident.is_some());
    assert_eq!(
        exposed.economics.operating_cost, quiet_plan.economics.operating_cost,
        "public prominence affects detection, not an invented racket operating cost"
    );
    assert_eq!(exposed.economics.investigation_heat, Money::ZERO);
    validate_enterprise_cycle_plan(&fixture.state, exposed)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    validate_state_against_registry(&registry, &fixture.state).unwrap();
    restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();
}

#[test]
fn foreign_case_raises_discovery_risk_but_not_own_racket_cost() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture();
    let enterprise = establish_protection(&registry, &mut fixture);
    let EnterpriseLocation::Neighborhood(district) = fixture.location else {
        unreachable!()
    };
    let police = insert_district_police(&registry, &mut fixture, "Vice Bureau", district);
    let rival = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Foreign Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .unwrap();
    let operative = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Foreign Scout".to_owned(),
            organization: Some(rival),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::Surveillance, rating(90))]),
            drives: BTreeMap::new(),
            traits: BTreeSet::new(),
        },
    )
    .unwrap();
    let threshold = registry
        .get_enterprise(EnterpriseKind::Protection)
        .economics()
        .enforcement_attention_basis_points_per_exposure_source();
    // Same explicit roll: quiet district misses; a rival case adds one exposure source.
    let mut quiet = fixture.state.clone();
    quiet.advance_clock(DAY_DURATION);
    let baseline = decide_enterprise_cycle(
        &registry,
        &quiet,
        enterprise,
        EnterpriseCycleRandomness::new(0, threshold),
    )
    .unwrap();
    assert!(baseline.enforcement_incident.is_none());
    open_originated_pressure_case(
        &registry,
        &mut fixture,
        police,
        PressureCaseOrigin {
            organization: rival,
            manager: operative,
        },
        "Foreign surveillance exposure",
        EntityRef::Neighborhood(district),
    );
    let mut foreign_only = fixture.state.clone();
    foreign_only.advance_clock(DAY_DURATION);
    let foreign = decide_enterprise_cycle(
        &registry,
        &foreign_only,
        enterprise,
        EnterpriseCycleRandomness::new(0, threshold),
    )
    .unwrap();
    assert!(foreign.enforcement_incident.is_some());
    assert_eq!(
        foreign.economics.operating_cost,
        baseline.economics.operating_cost
    );
    assert_eq!(foreign.economics.investigation_heat, Money::ZERO);
    // The same amount of direct casework is economically different.
    let own = fixture.organization;
    let own_manager = fixture.authority.manager;
    open_originated_pressure_case(
        &registry,
        &mut fixture,
        police,
        PressureCaseOrigin {
            organization: own,
            manager: own_manager,
        },
        "Own surveillance exposure",
        EntityRef::Neighborhood(district),
    );
    fixture.state.advance_clock(DAY_DURATION);
    let attributable = decide_enterprise_cycle(
        &registry,
        &fixture.state,
        enterprise,
        EnterpriseCycleRandomness::new(0, threshold),
    )
    .unwrap();
    assert_eq!(
        attributable.economics.investigation_heat,
        registry
            .get_enterprise(EnterpriseKind::Protection)
            .economics()
            .heat_surcharge_per_active_case()
    );
    validate_enterprise_cycle_plan(&fixture.state, attributable)
        .unwrap()
        .commit(&mut fixture.state)
        .unwrap();
    restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();
}

fn advance_day(registry: &Registry, state: &mut AppState) -> crate::core::simulation::TickOutcome {
    let mut last = run_tick(registry, state);
    for _ in 1..DAY_DURATION.as_minutes() {
        last = run_tick(registry, state);
    }
    last
}

#[test]
fn vice_inquiry_observes_distinct_operating_cycles_then_detains_and_refers_manager() {
    let registry = build_registry();
    let mut fixture = make_test_enterprise_fixture_with_inputs(
        OrganizationKind::Criminal,
        AutonomyLevel::Delegated,
        NeighborhoodProfile {
            economy: NeighborhoodEconomyProfile {
                wealth: rating(100),
                commercial_activity: rating(100),
                illicit_demand: rating(100),
            },
            institutions: NeighborhoodInstitutionProfile {
                police_presence: rating(100),
            },
        },
        Some(rating(100)),
    );
    designate_player_organization(&mut fixture.state, fixture.organization).unwrap();
    let enterprise = establish_protection(&registry, &mut fixture);
    let EnterpriseLocation::Neighborhood(district) = fixture.location else {
        unreachable!()
    };
    let police = insert_district_police(&registry, &mut fixture, "Vice Bureau", district);
    let manager = fixture.authority.manager;

    // Force only the initial exposure roll through the production settlement transaction.
    fixture.state.advance_clock(DAY_DURATION);
    let first = validate_enterprise_cycle_plan(
        &fixture.state,
        decide_enterprise_cycle(
            &registry,
            &fixture.state,
            enterprise,
            EnterpriseCycleRandomness::new(0, 0),
        )
        .unwrap(),
    )
    .unwrap()
    .commit(&mut fixture.state)
    .unwrap();
    let case = find_racket_investigation(&fixture, police, enterprise)
        .unwrap()
        .id();
    assert!(
        apply_enterprise_observations(&mut fixture.state, &[first])
            .unwrap()
            .is_empty()
    );
    let draft = |state: &AppState| EvidenceDraft {
        investigation: case,
        custodian: police,
        subject: EntityRef::Character(manager),
        origin: Some(EntityRef::Enterprise(enterprise)),
        kind: EvidenceKind::Surveillance,
        strength: EvidenceStrength::Strong,
        reliability: EvidenceReliability::Credible,
        admissibility: Admissibility::Admissible,
        discovered_at: state.now(),
    };
    assert!(
        matches!(validate_add_evidence(&fixture.state, draft(&fixture.state)),
        Err(InvestigationError::InvalidEnterpriseObservation(id)) if id == enterprise)
    );

    let mut no_detective = fixture.state.clone();
    no_detective.advance_clock(SimDuration::from_minutes(DAY_DURATION.as_minutes() - 1));
    let unstaffed = run_tick(&registry, &mut no_detective);
    assert_eq!(unstaffed.enterprise_cycles.len(), 1);
    assert!(unstaffed.enterprise_observations.is_empty());

    insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Vice Detective".to_owned(),
            organization: Some(police),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::Investigation, rating(100))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();
    let office = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "District Prosecutor".to_owned(),
            kind: OrganizationKind::Prosecutor,
        },
    )
    .unwrap();
    insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Prosecutor".to_owned(),
            organization: Some(office),
            supervisor: None,
            autonomy: AutonomyLevel::Broad,
            capabilities: BTreeMap::from([(CapabilityKind::LegalKnowledge, rating(80))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .unwrap();

    let day = advance_day(&registry, &mut fixture.state);
    assert_eq!(day.enterprise_observations.len(), 1);
    let observation = fixture
        .state
        .legal()
        .get_evidence(day.enterprise_observations[0])
        .unwrap();
    assert_eq!(observation.subject(), EntityRef::Character(manager));
    assert_eq!(
        observation.origin(),
        Some(EntityRef::Enterprise(enterprise))
    );
    assert_eq!(observation.discovered_at(), fixture.state.now());
    assert!(
        fixture
            .state
            .legal()
            .active_arrest_for_character(manager)
            .is_none()
    );
    let before = bincode::serialize(&fixture.state).unwrap();
    assert!(
        matches!(validate_add_evidence(&fixture.state, draft(&fixture.state)),
        Err(InvestigationError::InvalidEnterpriseObservation(id)) if id == enterprise)
    );
    assert_eq!(bincode::serialize(&fixture.state).unwrap(), before);

    // Stopping after one observation prevents a new independent operating event, even
    // while the original file and its forensic derivative survive.
    let mut stopped = fixture.state.clone();
    validate_suspend_enterprise(&stopped, enterprise)
        .unwrap()
        .commit(&mut stopped)
        .unwrap();
    let quiet = advance_day(&registry, &mut stopped);
    assert!(quiet.enterprise_observations.is_empty());
    assert!(
        stopped
            .legal()
            .active_arrest_for_character(manager)
            .is_none()
    );

    let mut restored =
        restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();
    let second = advance_day(&registry, &mut fixture.state);
    let replay = advance_day(&registry, &mut restored);
    assert_eq!(second, replay);
    assert_eq!(second.enterprise_observations.len(), 1);
    assert_ne!(day.enterprise_observations, second.enterprise_observations);
    assert_eq!(
        bincode::serialize(&fixture.state).unwrap(),
        bincode::serialize(&restored).unwrap()
    );
    let next = run_tick(&registry, &mut fixture.state);
    assert_eq!(next, run_tick(&registry, &mut restored));
    assert_eq!(next.evidence_arrests.len(), 1);
    assert_eq!(
        next.evidence_arrests[0].organization,
        Some(fixture.organization)
    );
    let arrest = next.evidence_arrests[0].arrest;
    assert_eq!(
        fixture
            .state
            .legal()
            .get_arrest(arrest)
            .unwrap()
            .character(),
        manager
    );
    assert_eq!(
        fixture
            .state
            .enterprises()
            .get_enterprise(enterprise)
            .unwrap()
            .status(),
        EnterpriseStatus::Suspended
    );

    // An old vice file cannot recycle daily observations of the very same
    // racket into an automatic second arrest when the manager resumes trading.
    // Test beyond the former 60-day cooldown with genuine, distinct new
    // observations, not forged arrest evidence or a hand-edited case graph.
    let mut repeated_vice = fixture.state.clone();
    repeated_vice.advance_clock(SimDuration::ONE_MINUTE);
    crate::legal::arrest_system::validate_release_arrest(&repeated_vice, arrest)
        .unwrap()
        .commit(&mut repeated_vice)
        .unwrap();
    validate_resume_enterprise(&registry, &repeated_vice, enterprise)
        .unwrap()
        .commit(&mut repeated_vice)
        .unwrap();
    let mut subsequent_observations = 0;
    for _ in 0..5 {
        let tick = advance_day(&registry, &mut repeated_vice);
        subsequent_observations += tick.enterprise_observations.len();
        if subsequent_observations >= 2 {
            break;
        }
    }
    assert!(
        subsequent_observations >= 2,
        "the police must have fresh surveillance of operating crime after release"
    );
    // Crossing the former mechanical 60-day boundary without a new operation
    // cannot turn two fresh observations of the old racket into new custody.
    repeated_vice.advance_clock(SimDuration::from_minutes(61 * 1_440));
    assert!(
        crate::legal::arrest_system::apply_autonomous_evidence_arrests(
            &registry,
            &mut repeated_vice,
        )
        .unwrap()
        .is_empty(),
        "observing the same already-cited racket must not create clockwork repeat custody"
    );
    let mut restored_repeated_vice =
        restore_save(&registry, build_save(&registry, &repeated_vice).unwrap()).unwrap();
    assert!(
        crate::legal::arrest_system::apply_autonomous_evidence_arrests(
            &registry,
            &mut restored_repeated_vice,
        )
        .unwrap()
        .is_empty(),
        "save/load must retain the repeat-vice arrest rule"
    );

    // A later, unrelated police matter must not erase the earlier vice history.
    // Otherwise alternating case files makes the same old racket arrestable again
    // as soon as the general cooldown expires.
    let unrelated = validate_open_investigation(
        &repeated_vice,
        InvestigationDraft {
            owner: police,
            title: "Unrelated property inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(manager)]),
        },
    )
    .unwrap()
    .commit(&mut repeated_vice)
    .unwrap();
    let mut evidence = BTreeSet::new();
    for kind in [EvidenceKind::Document, EvidenceKind::FinancialRecord] {
        evidence.insert(
            validate_add_evidence(
                &repeated_vice,
                EvidenceDraft {
                    investigation: unrelated,
                    custodian: police,
                    subject: EntityRef::Character(manager),
                    origin: None,
                    kind,
                    strength: EvidenceStrength::Strong,
                    reliability: EvidenceReliability::HighlyReliable,
                    admissibility: Admissibility::Admissible,
                    discovered_at: repeated_vice.now(),
                },
            )
            .unwrap()
            .commit(&mut repeated_vice)
            .unwrap(),
        );
    }
    let intervening = crate::legal::arrest_system::apply_autonomous_evidence_arrests(
        &registry,
        &mut repeated_vice,
    )
    .unwrap();
    assert_eq!(intervening.len(), 1);
    assert_eq!(
        repeated_vice
            .legal()
            .get_arrest(intervening[0])
            .unwrap()
            .investigation(),
        unrelated,
        "the original vice history must not immunize the manager from a distinct matter"
    );
    repeated_vice.advance_clock(SimDuration::ONE_MINUTE);
    crate::legal::arrest_system::validate_release_arrest(&repeated_vice, intervening[0])
        .unwrap()
        .commit(&mut repeated_vice)
        .unwrap();
    repeated_vice.advance_clock(SimDuration::from_minutes(61 * 1_440));
    let mut replay =
        restore_save(&registry, build_save(&registry, &repeated_vice).unwrap()).unwrap();
    let before = bincode::serialize(&repeated_vice).unwrap();
    for state in [&mut repeated_vice, &mut replay] {
        assert!(
            crate::legal::arrest_system::apply_autonomous_evidence_arrests(&registry, state)
                .unwrap()
                .is_empty(),
            "intervening unrelated custody must not re-enable repeat arrests for old vice"
        );
        assert_eq!(bincode::serialize(state).unwrap(), before);
    }

    // Daily prosecutorial intake is downstream of actual custody, never manufactured here.
    fixture
        .state
        .advance_clock(SimDuration::from_minutes(DAY_DURATION.as_minutes() - 2));
    let referral = run_tick(&registry, &mut fixture.state);
    assert_eq!(referral.opened_prosecution_cases.len(), 1);
    assert!(member_prosecution_under_review(
        &fixture.state,
        fixture.organization,
        arrest
    ));
    restore_save(&registry, build_save(&registry, &fixture.state).unwrap()).unwrap();
}
