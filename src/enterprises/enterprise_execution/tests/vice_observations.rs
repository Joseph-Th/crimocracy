//! End-to-end vice surveillance through actual settlement, custody, and referral.
use super::*;
use crate::core::time::DAY_DURATION;
use crate::legal::investigation_system::{InvestigationError, apply_enterprise_observations};
use crate::legal::prosecution_system::member_prosecution_under_review;
use crate::world::world_system::designate_player_organization;

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
    let arrest = next.evidence_arrests[0];
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
