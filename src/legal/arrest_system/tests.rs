//! Focused tests for evidence-threshold arrest validation, custody, and autonomous conversion.

use super::*;
use crate::build_registry;
use crate::core::invariants::{validate_invariants, validate_state};
use crate::core::persistence::{SaveEnvelope, build_save, restore_save};
use crate::core::time::SimDuration;
use crate::legal::investigation_system::{
    InvestigationError, InvestigationTransition, validate_add_evidence,
    validate_open_investigation, validate_transition_investigation,
};
use crate::legal::{
    Admissibility, EvidenceDraft, EvidenceKind, EvidenceReliability, EvidenceStrength,
    InvestigationDraft,
};
use crate::registry::Registry;
use crate::world::world_system::{
    WorldError, insert_character, insert_organization, validate_reassign_character,
};
use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind};
use std::collections::{BTreeMap, BTreeSet};

struct Fixture {
    registry: Registry,
    state: AppState,
    police: OrganizationId,
    suspect: CharacterId,
    investigation: InvestigationId,
    evidence: EvidenceId,
}

fn fixture() -> Fixture {
    let registry = build_registry();
    let mut state = AppState::new(0xA22E_5701);
    let crew = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Custody Test Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("crew should validate");
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Custody Test Police".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police should validate");
    let suspect = insert_character(
        &mut state,
        CharacterDraft {
            name: "Case Subject".to_owned(),
            organization: Some(crew),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("suspect should validate");
    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Evidence-backed custody test".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(suspect)]),
        },
    )
    .expect("investigation should validate")
    .commit(&mut state)
    .expect("investigation should commit");
    let evidence = add_character_evidence(&mut state, police, investigation, suspect);
    Fixture {
        registry,
        state,
        police,
        suspect,
        investigation,
        evidence,
    }
}

fn add_character_evidence(
    state: &mut AppState,
    police: OrganizationId,
    investigation: InvestigationId,
    suspect: CharacterId,
) -> EvidenceId {
    validate_add_evidence(
        state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(suspect),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )
    .expect("case evidence should validate")
    .commit(state)
    .expect("case evidence should commit")
}

fn arrest_fixture(fixture: &mut Fixture) -> ArrestId {
    validate_arrest(
        &fixture.state,
        ArrestDraft {
            character: fixture.suspect,
            investigation: fixture.investigation,
            evidence: BTreeSet::from([fixture.evidence]),
        },
    )
    .expect("evidence-backed arrest should validate")
    .commit(&mut fixture.state)
    .expect("evidence-backed arrest should commit")
}

#[test]
fn arrest_and_release_are_durable_indexed_lifecycle_records() {
    let mut fixture = fixture();
    let arrest = arrest_fixture(&mut fixture);
    let record = fixture
        .state
        .legal()
        .get_arrest(arrest)
        .expect("arrest should persist");
    assert_eq!(record.status(), ArrestStatus::Detained);
    assert_eq!(record.version(), 1);
    assert_eq!(record.authority(), fixture.police);
    assert_eq!(record.evidence(), &BTreeSet::from([fixture.evidence]));
    assert_eq!(
        fixture
            .state
            .legal()
            .active_arrest_for_character(fixture.suspect)
            .map(|record| record.id()),
        Some(arrest)
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .arrests_for_investigation(fixture.investigation)
            .count(),
        1
    );
    validate_state(&fixture.state).expect("detention state should validate");
    validate_invariants(&fixture.state);

    let envelope = build_save(&fixture.registry, &fixture.state)
        .expect("detention state should build a save envelope");
    let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("save envelope should deserialize");
    let mut restored = restore_save(&fixture.registry, decoded)
        .expect("detention state should restore with indexes intact");
    assert_eq!(
        restored
            .legal()
            .active_arrest_for_character(fixture.suspect)
            .map(|record| record.id()),
        Some(arrest)
    );
    validate_release_arrest(&restored, arrest)
        .expect("restored detention should remain releasable")
        .commit(&mut restored)
        .expect("restored detention release should commit");
    let rearrest = validate_arrest(
        &restored,
        ArrestDraft {
            character: fixture.suspect,
            investigation: fixture.investigation,
            evidence: BTreeSet::from([fixture.evidence]),
        },
    )
    .expect("released restored character should permit a later evidence-backed arrest")
    .commit(&mut restored)
    .expect("later restored arrest should commit with a fresh ID");
    assert_ne!(rearrest, arrest);
    assert_eq!(
        restored
            .legal()
            .arrests_for_character(fixture.suspect)
            .count(),
        2
    );
    assert_eq!(
        restored
            .legal()
            .active_arrest_for_character(fixture.suspect)
            .map(|record| record.id()),
        Some(rearrest)
    );
    validate_state(&restored).expect("restored re-arrest state should validate");
    validate_invariants(&restored);

    fixture.state.advance_clock(SimDuration::from_minutes(45));
    validate_release_arrest(&fixture.state, arrest)
        .expect("active detention should release")
        .commit(&mut fixture.state)
        .expect("release should commit");
    let released = fixture
        .state
        .legal()
        .get_arrest(arrest)
        .expect("released arrest history should persist");
    assert_eq!(released.status(), ArrestStatus::Released);
    assert_eq!(released.version(), 2);
    assert_eq!(released.released_at(), Some(fixture.state.now()));
    assert!(
        fixture
            .state
            .legal()
            .active_arrest_for_character(fixture.suspect)
            .is_none()
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .arrests_for_character(fixture.suspect)
            .count(),
        1
    );
    validate_state(&fixture.state).expect("released custody history should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn arrest_validation_is_case_specific_and_stales_when_case_evidence_changes() {
    let mut fixture = fixture();
    let stale = validate_arrest(
        &fixture.state,
        ArrestDraft {
            character: fixture.suspect,
            investigation: fixture.investigation,
            evidence: BTreeSet::from([fixture.evidence]),
        },
    )
    .expect("initial arrest plan should validate");
    add_character_evidence(
        &mut fixture.state,
        fixture.police,
        fixture.investigation,
        fixture.suspect,
    );
    let error = stale
        .commit(&mut fixture.state)
        .expect_err("case mutation must stale a previously validated arrest");
    assert!(matches!(error, ArrestError::StaleInvestigation { .. }));
    assert!(
        fixture
            .state
            .legal()
            .active_arrest_for_character(fixture.suspect)
            .is_none()
    );

    let second_case = validate_open_investigation(
        &fixture.state,
        InvestigationDraft {
            owner: fixture.police,
            title: "Separate case".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(fixture.suspect)]),
        },
    )
    .expect("second investigation should validate")
    .commit(&mut fixture.state)
    .expect("second investigation should commit");
    let foreign_evidence = add_character_evidence(
        &mut fixture.state,
        fixture.police,
        second_case,
        fixture.suspect,
    );
    let error = validate_arrest(
        &fixture.state,
        ArrestDraft {
            character: fixture.suspect,
            investigation: fixture.investigation,
            evidence: BTreeSet::from([foreign_evidence]),
        },
    )
    .expect_err("evidence from another case must not support this arrest");
    assert_eq!(
        error,
        ArrestError::EvidenceInvestigationMismatch {
            evidence: foreign_evidence,
            investigation: fixture.investigation,
        }
    );
    validate_state(&fixture.state).expect("rejected arrest attempts must preserve valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn active_detention_blocks_case_suspension_and_membership_escape_until_release() {
    let mut fixture = fixture();
    let arrest = arrest_fixture(&mut fixture);

    // Suspension stays blocked while an arrest holds someone in custody; closing remains
    // allowed because a case whose subject is detained is cleared by arrest.
    let transition_error = validate_transition_investigation(
        &fixture.state,
        fixture.investigation,
        InvestigationTransition::Suspend,
    )
    .expect_err("active detention must keep its source case unsuspended");
    assert_eq!(
        transition_error,
        InvestigationError::ActiveArrestBlocksTransition {
            investigation: fixture.investigation,
            arrest,
        }
    );
    validate_transition_investigation(
        &fixture.state,
        fixture.investigation,
        InvestigationTransition::Close,
    )
    .expect("a cleared case must close while its subject is in custody");
    let reassignment_error =
        validate_reassign_character(&fixture.state, fixture.suspect, None, None)
            .expect_err("detained character must not escape custody through reassignment");
    assert_eq!(
        reassignment_error,
        WorldError::ActiveArrestAssignment {
            character: fixture.suspect,
            arrest,
        }
    );

    validate_release_arrest(&fixture.state, arrest)
        .expect("detention should release")
        .commit(&mut fixture.state)
        .expect("release should commit");
    validate_transition_investigation(
        &fixture.state,
        fixture.investigation,
        InvestigationTransition::Close,
    )
    .expect("released custody no longer requires an active source case")
    .commit(&mut fixture.state)
    .expect("case close should commit after release");
    validate_reassign_character(&fixture.state, fixture.suspect, None, None)
        .expect("released character should permit ordinary membership changes")
        .commit(&mut fixture.state)
        .expect("membership change should commit after release");
    validate_state(&fixture.state).expect("post-release lifecycle state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn detention_preserves_formal_supervision_but_blocks_new_supervisory_work() {
    let mut fixture = fixture();
    let crew = fixture
        .state
        .world()
        .get_character(fixture.suspect)
        .and_then(|record| record.organization())
        .expect("suspect fixture should belong to the criminal organization");
    let direct_report = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Existing Direct Report".to_owned(),
            organization: Some(crew),
            supervisor: Some(fixture.suspect),
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("preexisting reporting line should validate");
    let unassigned = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Unassigned Member".to_owned(),
            organization: Some(crew),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("unassigned member should validate");

    let arrest = arrest_fixture(&mut fixture);
    assert_eq!(
        fixture
            .state
            .world()
            .direct_reports(fixture.suspect)
            .map(|record| record.id())
            .collect::<Vec<_>>(),
        vec![direct_report]
    );
    validate_state(&fixture.state)
        .expect("formal reporting lines may persist while a supervisor is detained");
    validate_invariants(&fixture.state);

    let error = validate_reassign_character(
        &fixture.state,
        unassigned,
        Some(crew),
        Some(fixture.suspect),
    )
    .expect_err("detained supervisor must not receive new reporting responsibility");
    assert_eq!(
        error,
        WorldError::DetainedSupervisor {
            supervisor: fixture.suspect,
            arrest,
        }
    );
    assert_eq!(
        fixture
            .state
            .world()
            .get_character(unassigned)
            .expect("rejected reassignment must retain the character")
            .supervisor(),
        None
    );
    validate_state(&fixture.state).expect("rejected supervisory work must preserve valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn active_operation_responsibility_blocks_custody() {
    use crate::operations::operation_system::validate_authorize_operation;
    use crate::operations::{OperationApproach, OperationDraft, OperationKind, OperationObjective};
    use crate::world::world_system::{insert_business, insert_neighborhood};
    use crate::world::{
        BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner, NeighborhoodDraft,
        NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile, NeighborhoodProfile, Rating,
    };

    let mut fixture = fixture();
    let neighborhood = insert_neighborhood(
        &mut fixture.state,
        NeighborhoodDraft {
            name: "Arrest Guard Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: Rating::try_new(50).expect("fixture rating should validate"),
                    commercial_activity: Rating::try_new(50)
                        .expect("fixture rating should validate"),
                    illicit_demand: Rating::try_new(50).expect("fixture rating should validate"),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: Rating::try_new(50).expect("fixture rating should validate"),
                },
            },
        },
    )
    .expect("neighborhood should validate");
    let business = insert_business(
        &fixture.registry,
        &mut fixture.state,
        BusinessDraft {
            name: "Arrest Guard Front".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner: BusinessOwner::Independent,
        },
    )
    .expect("business should validate");
    let operation = validate_authorize_operation(
        &fixture.registry,
        &fixture.state,
        OperationDraft {
            title: "Guarded score".to_owned(),
            kind: OperationKind::Intimidation,
            responsible_organization: fixture
                .state
                .world()
                .get_character(fixture.suspect)
                .and_then(|record| record.organization())
                .expect("suspect should hold membership"),
            leader: fixture.suspect,
            objective: OperationObjective::ObtainCash {
                target: crate::core::entity::EntityRef::Business(business),
            },
            approach: OperationApproach::Intimidating,
            roles: BTreeMap::from([(crate::operations::RoleKind::Coordinator, fixture.suspect)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: crate::core::time::SimTime::ZERO,
        },
    )
    .expect("authorized operation should validate")
    .commit(&mut fixture.state)
    .expect("authorized operation should commit");

    let error = validate_arrest(
        &fixture.state,
        ArrestDraft {
            character: fixture.suspect,
            investigation: fixture.investigation,
            evidence: BTreeSet::from([fixture.evidence]),
        },
    )
    .expect_err("an operation participant must not enter custody");
    assert_eq!(
        error,
        ArrestError::ActiveOperationResponsibility {
            character: fixture.suspect,
            operation,
        }
    );
    assert!(
        fixture
            .state
            .legal()
            .active_arrest_for_character(fixture.suspect)
            .is_none(),
        "rejected arrest must leave authoritative state unchanged"
    );
    validate_state(&fixture.state).expect("state after rejected arrest should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn derived_forensic_evidence_cannot_satisfy_the_autonomous_arrest_bar_alone() {
    use crate::core::entity::EntityRef;
    use crate::core::simulation::run_tick;
    use crate::legal::investigation_system::{
        validate_assign_investigator, validate_incident_intake,
    };
    use crate::legal::investigation_work_execution::validate_schedule_investigation_work;
    use crate::legal::{
        IncidentEvidenceDraft, IncidentIntakeDraft, InvestigationWorkDraft, InvestigationWorkFocus,
        InvestigationWorkKind,
    };
    use crate::operations::operation_system::validate_authorize_operation;
    use crate::operations::{OperationApproach, OperationDraft, OperationKind, OperationObjective};
    use crate::world::world_system::{insert_business, insert_character, insert_neighborhood};
    use crate::world::{
        BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner, CapabilityKind,
        NeighborhoodDraft, NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile,
        NeighborhoodProfile, Rating,
    };

    let mut fixture = fixture();
    let crew = fixture
        .state
        .world()
        .get_character(fixture.suspect)
        .and_then(|record| record.organization())
        .expect("suspect should hold membership");
    let leader = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Unbooked Crew Leader".to_owned(),
            organization: Some(crew),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("leader fixture should validate");
    let detective = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "Forensic Detective".to_owned(),
            organization: Some(fixture.police),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(
                CapabilityKind::Investigation,
                Rating::try_new(90).expect("fixture rating should validate"),
            )]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("detective fixture should validate");
    let neighborhood = insert_neighborhood(
        &mut fixture.state,
        NeighborhoodDraft {
            name: "Corroboration Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: Rating::try_new(50).expect("fixture rating should validate"),
                    commercial_activity: Rating::try_new(50)
                        .expect("fixture rating should validate"),
                    illicit_demand: Rating::try_new(50).expect("fixture rating should validate"),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: Rating::try_new(50).expect("fixture rating should validate"),
                },
            },
        },
    )
    .expect("neighborhood should validate");
    let business = insert_business(
        &fixture.registry,
        &mut fixture.state,
        BusinessDraft {
            name: "Corroboration Front".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([
                BusinessFunction::CashIntensive,
                BusinessFunction::CustomerAccess,
            ]),
            neighborhood,
            owner: BusinessOwner::Independent,
        },
    )
    .expect("business should validate");
    // The suspect holds no operation booking: a different member leads the score.
    let operation = validate_authorize_operation(
        &fixture.registry,
        &fixture.state,
        OperationDraft {
            title: "Someone else's score".to_owned(),
            kind: OperationKind::Intimidation,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::ObtainCash {
                target: EntityRef::Business(business),
            },
            approach: OperationApproach::Intimidating,
            roles: BTreeMap::from([(crate::operations::RoleKind::Coordinator, leader)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: crate::core::time::SimTime::ZERO,
        },
    )
    .expect("operation should authorize")
    .commit(&mut fixture.state)
    .expect("operation should commit");
    // An operation-originated case with exactly ONE independent strong item on the suspect.
    let outcome = validate_incident_intake(
        &fixture.state,
        IncidentIntakeDraft {
            owner: fixture.police,
            title: "Single-source inquiry".to_owned(),
            subjects: BTreeSet::from([
                EntityRef::Operation(operation),
                EntityRef::Character(fixture.suspect),
            ]),
            evidence: vec![IncidentEvidenceDraft {
                subject: EntityRef::Character(fixture.suspect),
                origin: Some(EntityRef::Operation(operation)),
                kind: EvidenceKind::Fingerprint,
                strength: EvidenceStrength::Strong,
                reliability: EvidenceReliability::HighlyReliable,
                admissibility: Admissibility::Admissible,
                discovered_at: fixture.state.now(),
            }],
            origin: Some(EntityRef::Operation(operation)),
            notified_organizations: BTreeSet::from([crew]),
            witness: None,
        },
    )
    .expect("originated case should validate")
    .commit(&mut fixture.state)
    .expect("originated case should commit");
    let case = outcome.investigation;
    let source = *outcome
        .evidence
        .first()
        .expect("intake carries its evidence");
    validate_assign_investigator(&fixture.state, case, detective)
        .expect("case staffing should validate")
        .commit(&mut fixture.state)
        .expect("case staffing should commit");

    // Develop the source: the forensic derivative clones its subject and strength.
    let work = validate_schedule_investigation_work(
        &fixture.registry,
        &fixture.state,
        InvestigationWorkDraft {
            investigation: case,
            investigator: detective,
            kind: InvestigationWorkKind::EvidenceReview,
            focus: InvestigationWorkFocus::evidence(source),
        },
    )
    .expect("reviewable source should schedule")
    .commit(&mut fixture.state)
    .expect("review should commit");
    loop {
        let outcome = run_tick(&fixture.registry, &mut fixture.state);
        if outcome.resolved_investigation_work.contains(&work) {
            break;
        }
    }
    assert!(
        fixture
            .state
            .legal()
            .work_for_investigation(case)
            .any(|entry| entry
                .resolution()
                .is_some_and(|resolution| resolution.derived_evidence().is_some())),
        "the review must have produced a forensic derivative"
    );

    // One independent item plus its own derivative is still one fact: no custody.
    assert!(
        apply_autonomous_evidence_arrests(&mut fixture.state)
            .expect("autonomous arrest pass should resolve")
            .is_empty(),
        "a derived analysis must not corroborate its own source"
    );

    // A second INDEPENDENT strong item completes the corroboration bar and custody follows.
    let second = add_character_evidence(&mut fixture.state, fixture.police, case, fixture.suspect);
    let arrests = apply_autonomous_evidence_arrests(&mut fixture.state)
        .expect("autonomous arrest pass should resolve");
    assert_eq!(arrests.len(), 1);
    let record = fixture
        .state
        .legal()
        .get_arrest(arrests[0])
        .expect("arrest record should persist");
    assert_eq!(record.character(), fixture.suspect);
    assert_eq!(
        record.evidence(),
        &BTreeSet::from([source, second]),
        "only independent items carry the arrest"
    );
    validate_state(&fixture.state).expect("custody state should remain valid");
    validate_invariants(&fixture.state);
}
