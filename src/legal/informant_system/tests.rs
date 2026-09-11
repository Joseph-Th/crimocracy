//! Focused tests for detainee informant recruitment and disclosure handling.

use super::*;
use crate::build_registry;
use crate::core::invariants::{validate_invariants, validate_state};
use crate::core::persistence::{SaveEnvelope, build_save, restore_save};
use crate::core::simulation::run_tick;
use crate::core::time::{SimDuration, SimTime};
use crate::intelligence::intelligence_system::validate_record_information;
use crate::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, Reliability, Specificity,
};
use crate::legal::ArrestDraft;
use crate::legal::investigation_system::{
    InvestigationError, InvestigationTransition, validate_add_evidence, validate_incident_intake,
    validate_open_investigation, validate_transition_investigation,
};
use crate::legal::{EvidenceDraft, IncidentEvidenceDraft, IncidentIntakeDraft, InvestigationDraft};
use crate::operations::operation_system::validate_authorize_operation;
use crate::operations::{
    OperationApproach, OperationDraft, OperationKind, OperationObjective, RoleKind,
};
use crate::world::world_system::{
    WorldError, insert_character, insert_organization, validate_reassign_character,
};
use crate::world::{
    AutonomyLevel, CapabilityKind, CharacterDraft, DriveKind, OrganizationDraft, OrganizationKind,
    Rating,
};
use std::collections::{BTreeMap, BTreeSet};

struct Fixture {
    state: AppState,
    police: OrganizationId,
    criminal: OrganizationId,
    member: CharacterId,
    investigation: InvestigationId,
}

fn fixture() -> Fixture {
    let registry = build_registry();
    let mut state = AppState::new(0x1F0A_1934);
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Confidential Source Bureau".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let criminal = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Harbor Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("criminal fixture should validate");
    let member = insert_character(
        &mut state,
        CharacterDraft {
            name: "Leo Trent".to_owned(),
            organization: Some(criminal),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("member fixture should validate");
    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Harbor organization inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Organization(criminal)]),
        },
    )
    .expect("investigation fixture should validate")
    .commit(&mut state)
    .expect("investigation fixture should commit");
    Fixture {
        state,
        police,
        criminal,
        member,
        investigation,
    }
}

fn record_personal_information(fixture: &mut Fixture) -> InformationId {
    validate_record_information(
        &fixture.state,
        InformationDraft {
            holder: KnowledgeHolder::Character(fixture.member),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::Personnel,
            source_entity: None,
            subject: EntityRef::Organization(fixture.criminal),
            observed_at: fixture.state.now(),
            reliability: Reliability::GenerallyReliable,
            specificity: Specificity::Specific,
            summary: "The member directly observed the crew's current personnel structure."
                .to_owned(),
        },
    )
    .expect("personal information should validate")
    .commit(&mut fixture.state)
    .expect("personal information should commit")
}

#[test]
fn disclosure_requires_personal_knowledge_and_creates_provenance_evidence() {
    let mut fixture = fixture();
    let informant = validate_establish_informant(
        &fixture.state,
        InformantDraft {
            character: fixture.member,
            handler: fixture.police,
        },
    )
    .expect("informant establishment should validate")
    .commit(&mut fixture.state)
    .expect("informant establishment should commit");
    assert_eq!(
        validate_reassign_character(&fixture.state, fixture.member, Some(fixture.police), None,)
            .expect_err("an informant cannot join the organization handling that relationship"),
        WorldError::InformantHandlerConflict {
            character: fixture.member,
            handler: fixture.police,
            informant,
        }
    );
    let organization_information = validate_record_information(
        &fixture.state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(fixture.police),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::Personnel,
            source_entity: None,
            subject: EntityRef::Organization(fixture.criminal),
            observed_at: fixture.state.now(),
            reliability: Reliability::GenerallyReliable,
            specificity: Specificity::Specific,
            summary: "The bureau has separate knowledge about the crew's personnel.".to_owned(),
        },
    )
    .expect("organization information should validate")
    .commit(&mut fixture.state)
    .expect("organization information should commit");
    assert_eq!(
        validate_record_informant_disclosure(
            &fixture.state,
            InformantDisclosureDraft {
                informant,
                investigation: fixture.investigation,
                source_information: organization_information,
            },
        )
        .expect_err("informants cannot disclose knowledge held only by their handler"),
        InformantError::InformationNotHeldByInformant {
            information: organization_information,
            character: fixture.member,
        }
    );
    let information = record_personal_information(&mut fixture);

    let disclosure = validate_record_informant_disclosure(
        &fixture.state,
        InformantDisclosureDraft {
            informant,
            investigation: fixture.investigation,
            source_information: information,
        },
    )
    .expect("personal informant knowledge should be disclosable")
    .commit(&mut fixture.state)
    .expect("validated disclosure should commit");

    let disclosure_record = fixture
        .state
        .legal()
        .informant_disclosures()
        .find(|record| record.id() == disclosure)
        .expect("disclosure should persist");
    let evidence = fixture
        .state
        .legal()
        .get_evidence(disclosure_record.evidence())
        .expect("informant evidence should persist");
    assert_eq!(evidence.kind(), EvidenceKind::InformantStatement);
    assert_eq!(evidence.strength(), EvidenceStrength::Strong);
    assert_eq!(evidence.reliability(), EvidenceReliability::Credible);
    assert_eq!(evidence.admissibility(), Admissibility::Unknown);
    assert_eq!(
        evidence.source(),
        Some(EntityRef::Character(fixture.member))
    );
    assert_eq!(
        evidence.subject(),
        EntityRef::Organization(fixture.criminal)
    );
    assert_eq!(disclosure_record.source_information(), information);
    assert_eq!(
        fixture
            .state
            .legal()
            .informant_disclosures()
            .filter(|record| record.source_information() == information)
            .map(|record| record.id())
            .collect::<Vec<_>>(),
        vec![disclosure]
    );
    assert!(matches!(
        validate_record_informant_disclosure(
            &fixture.state,
            InformantDisclosureDraft {
                informant,
                investigation: fixture.investigation,
                source_information: information,
            },
        ),
        Err(InformantError::DuplicateDisclosure {
            disclosure: existing,
            ..
        }) if existing == disclosure
    ));
    validate_state(&fixture.state).expect("canonical disclosure state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn disclosure_rejects_personal_information_unrelated_to_the_case() {
    let registry = build_registry();
    let mut fixture = fixture();
    let unrelated = insert_organization(
        &registry,
        &mut fixture.state,
        OrganizationDraft {
            name: "Unrelated Outfit".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("unrelated organization should validate");
    let informant = validate_establish_informant(
        &fixture.state,
        InformantDraft {
            character: fixture.member,
            handler: fixture.police,
        },
    )
    .expect("informant establishment should validate")
    .commit(&mut fixture.state)
    .expect("informant establishment should commit");
    let information = validate_record_information(
        &fixture.state,
        InformationDraft {
            holder: KnowledgeHolder::Character(fixture.member),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::Personnel,
            source_entity: None,
            subject: EntityRef::Organization(unrelated),
            observed_at: fixture.state.now(),
            reliability: Reliability::GenerallyReliable,
            specificity: Specificity::Specific,
            summary: "The source knows unrelated personnel facts.".to_owned(),
        },
    )
    .expect("unrelated personal information should validate")
    .commit(&mut fixture.state)
    .expect("unrelated personal information should commit");

    assert_eq!(
        validate_record_informant_disclosure(
            &fixture.state,
            InformantDisclosureDraft {
                informant,
                investigation: fixture.investigation,
                source_information: information,
            },
        )
        .expect_err("handler ownership must not turn unrelated knowledge into case evidence"),
        InformantError::InformationCaseMismatch {
            information,
            subject: EntityRef::Organization(unrelated),
            investigation: fixture.investigation,
        }
    );
    assert_eq!(fixture.state.legal().informant_disclosures().count(), 0);
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_disclosure_matches_active_case_subjects_not_only_operation_origins() {
    let mut fixture = fixture();
    let informant = validate_establish_informant(
        &fixture.state,
        InformantDraft {
            character: fixture.member,
            handler: fixture.police,
        },
    )
    .expect("informant establishment should validate")
    .commit(&mut fixture.state)
    .expect("informant establishment should commit");
    let information = record_personal_information(&mut fixture);

    let disclosures = apply_informant_disclosures(&mut fixture.state)
        .expect("relevant personal knowledge should flow into an active handler case");
    assert_eq!(disclosures.len(), 1);
    let disclosure = fixture
        .state
        .legal()
        .informant_disclosures()
        .find(|record| record.id() == disclosures[0])
        .expect("autonomous disclosure should persist");
    assert_eq!(disclosure.informant(), informant);
    assert_eq!(disclosure.investigation(), fixture.investigation);
    assert_eq!(disclosure.source_information(), information);
    validate_state(&fixture.state).expect("subject-matched disclosure state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn autonomous_disclosure_reaches_each_matching_case_in_one_pass() {
    let registry = build_registry();
    let mut fixture = fixture();
    let informant = validate_establish_informant(
        &fixture.state,
        InformantDraft {
            character: fixture.member,
            handler: fixture.police,
        },
    )
    .expect("informant establishment should validate")
    .commit(&mut fixture.state)
    .expect("informant establishment should commit");
    let information = record_personal_information(&mut fixture);
    let second_case = validate_open_investigation(
        &fixture.state,
        InvestigationDraft {
            owner: fixture.police,
            title: "Parallel harbor organization inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Organization(fixture.criminal)]),
        },
    )
    .expect("parallel case with the same subject should validate")
    .commit(&mut fixture.state)
    .expect("parallel case should commit");
    assert!(fixture.investigation < second_case);

    let first_pass = apply_informant_disclosures(&mut fixture.state)
        .expect("first disclosure pass should resolve");
    assert_eq!(first_pass.len(), 2);
    let first_pass_cases = first_pass
        .iter()
        .map(|id| {
            let record = fixture
                .state
                .legal()
                .informant_disclosures()
                .find(|record| record.id() == *id)
                .expect("same-pass disclosure should persist");
            assert_eq!(record.informant(), informant);
            assert_eq!(record.source_information(), information);
            record.investigation()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        first_pass_cases,
        BTreeSet::from([fixture.investigation, second_case]),
        "case creation order must not delay the same held fact for another matching active file"
    );

    let envelope = build_save(&registry, &fixture.state)
        .expect("fully propagated informant state should save");
    let bytes = bincode::serialize(&envelope).expect("informant save should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("informant save should deserialize");
    fixture.state = restore_save(&registry, decoded)
        .expect("informant state should restore with disclosure indexes rebuilt");

    assert!(
        apply_informant_disclosures(&mut fixture.state)
            .expect("rebuilt disclosure indexes should preserve exhaustion")
            .is_empty(),
        "one personal fact should reach every relevant active case once, then stop"
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .informant_disclosures()
            .filter(|record| record.source_information() == information)
            .map(|record| record.investigation())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([fixture.investigation, second_case])
    );
    validate_state(&fixture.state).expect("multi-case disclosure state should remain valid");
    validate_invariants(&fixture.state);
}

#[test]
fn informant_disclosure_refreshes_all_matching_originated_cases_before_cold_decay() {
    let registry = build_registry();
    let mut state = AppState::new(0x1F0A_C01D);
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Parallel Case Bureau".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police fixture should validate");
    let criminal = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Parallel Case Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("criminal fixture should validate");
    let source = insert_character(
        &mut state,
        CharacterDraft {
            name: "Parallel Case Source".to_owned(),
            organization: Some(criminal),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("source fixture should validate");
    let make_leader = |state: &mut AppState, name: &str| {
        insert_character(
            state,
            CharacterDraft {
                name: name.to_owned(),
                organization: Some(criminal),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::from([(
                    CapabilityKind::Surveillance,
                    Rating::try_new(60).expect("fixture rating should validate"),
                )]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("operation leader fixture should validate")
    };
    let first_leader = make_leader(&mut state, "First Parallel Observer");
    let second_leader = make_leader(&mut state, "Second Parallel Observer");

    // Offset setup from the day boundary so the regression isolates legal phase ordering.
    state.advance_clock(SimDuration::ONE_MINUTE);
    let informant = validate_establish_informant(
        &state,
        InformantDraft {
            character: source,
            handler: police,
        },
    )
    .expect("informant establishment should validate")
    .commit(&mut state)
    .expect("informant establishment should commit");
    let information = validate_record_information(
        &state,
        InformationDraft {
            holder: KnowledgeHolder::Character(source),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::Personnel,
            source_entity: None,
            subject: EntityRef::Organization(criminal),
            observed_at: state.now(),
            reliability: Reliability::GenerallyReliable,
            specificity: Specificity::Specific,
            summary: "The source knows the crew's current personnel structure.".to_owned(),
        },
    )
    .expect("source information should validate")
    .commit(&mut state)
    .expect("source information should commit");

    let cold_window = registry.legal().cold_case_window();
    let scheduled_for = SimTime::from_minutes(
        state
            .now()
            .as_minutes()
            .checked_add(u64::from(cold_window.as_minutes()))
            .and_then(|minute| minute.checked_add(60))
            .expect("fixture schedule should fit simulation time"),
    );
    let mut cases = Vec::new();
    for (index, leader) in [first_leader, second_leader].into_iter().enumerate() {
        let origin = validate_authorize_operation(
            &registry,
            &state,
            OperationDraft {
                title: format!("Parallel surveillance origin {}", index + 1),
                kind: OperationKind::Surveillance,
                responsible_organization: criminal,
                leader,
                objective: OperationObjective::GatherInformation {
                    target: EntityRef::Organization(criminal),
                },
                approach: OperationApproach::Covert,
                roles: BTreeMap::from([(RoleKind::Surveillance, leader)]),
                intelligence: BTreeSet::new(),
                constraints: Vec::new(),
                contingencies: Vec::new(),
                scheduled_for,
            },
        )
        .expect("future surveillance origin should validate")
        .commit(&mut state)
        .expect("future surveillance origin should commit");
        let case = validate_incident_intake(
            &state,
            IncidentIntakeDraft {
                owner: police,
                title: format!("Parallel originated case {}", index + 1),
                subjects: BTreeSet::from([EntityRef::Organization(criminal)]),
                evidence: vec![IncidentEvidenceDraft {
                    subject: EntityRef::Organization(criminal),
                    origin: Some(EntityRef::Operation(origin)),
                    kind: EvidenceKind::Surveillance,
                    strength: EvidenceStrength::Weak,
                    reliability: EvidenceReliability::Questionable,
                    admissibility: Admissibility::Unknown,
                    discovered_at: state.now(),
                }],
                origin: Some(EntityRef::Operation(origin)),
                witness: None,
            },
        )
        .expect("distinct active incident should open its own originated case")
        .commit(&mut state)
        .expect("originated case should commit")
        .investigation;
        cases.push(case);
    }
    assert_ne!(cases[0], cases[1]);

    state.advance_clock(SimDuration::from_minutes(
        cold_window
            .as_minutes()
            .checked_sub(1)
            .expect("cold-case window must exceed one minute"),
    ));
    let outcome = run_tick(&registry, &mut state);

    assert_eq!(outcome.informant_disclosures.len(), 2);
    assert!(outcome.cold_case_suspensions.is_empty());
    let disclosed_cases = outcome
        .informant_disclosures
        .iter()
        .map(|id| {
            let disclosure = state
                .legal()
                .informant_disclosures()
                .find(|record| record.id() == *id)
                .expect("same-minute disclosure should persist");
            assert_eq!(disclosure.informant(), informant);
            assert_eq!(disclosure.source_information(), information);
            disclosure.investigation()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(disclosed_cases, cases.iter().copied().collect());
    for case in cases {
        let investigation = state
            .legal()
            .get_investigation(case)
            .expect("refreshed case should persist");
        assert_eq!(investigation.status(), InvestigationStatus::Active);
        assert_eq!(investigation.last_activity_at(), state.now());
    }
    validate_state(&state).expect("same-minute disclosure state should validate");
    validate_invariants(&state);
}

#[test]
fn generic_evidence_path_cannot_forge_informant_statement() {
    let fixture = fixture();
    let error = match validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation: fixture.investigation,
            custodian: fixture.police,
            subject: EntityRef::Organization(fixture.criminal),
            origin: None,
            kind: EvidenceKind::InformantStatement,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::Credible,
            admissibility: Admissibility::Unknown,
            discovered_at: fixture.state.now(),
        },
    ) {
        Ok(_) => panic!("generic evidence path must reject informant statements"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        InvestigationError::InformantStatementRequiresDisclosure
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .all_evidence()
            .filter(|record| record.kind() == EvidenceKind::InformantStatement)
            .count(),
        0
    );
    validate_invariants(&fixture.state);
}

#[test]
fn disclosure_token_rejects_case_change_without_partial_mutation() {
    let mut fixture = fixture();
    let informant = validate_establish_informant(
        &fixture.state,
        InformantDraft {
            character: fixture.member,
            handler: fixture.police,
        },
    )
    .expect("informant establishment should validate")
    .commit(&mut fixture.state)
    .expect("informant establishment should commit");
    let information = record_personal_information(&mut fixture);
    let stale = validate_record_informant_disclosure(
        &fixture.state,
        InformantDisclosureDraft {
            informant,
            investigation: fixture.investigation,
            source_information: information,
        },
    )
    .expect("disclosure should initially validate");

    validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation: fixture.investigation,
            custodian: fixture.police,
            subject: EntityRef::Organization(fixture.criminal),
            origin: None,
            kind: EvidenceKind::Surveillance,
            strength: EvidenceStrength::Weak,
            reliability: EvidenceReliability::Questionable,
            admissibility: Admissibility::Unknown,
            discovered_at: fixture.state.now(),
        },
    )
    .expect("independent case mutation should validate")
    .commit(&mut fixture.state)
    .expect("independent case mutation should commit");

    assert!(matches!(
        stale.commit(&mut fixture.state),
        Err(InformantError::StaleInvestigation { .. })
    ));
    assert_eq!(
        fixture
            .state
            .legal()
            .all_evidence()
            .filter(|record| record.kind() == EvidenceKind::InformantStatement)
            .count(),
        0
    );
    assert_eq!(
        fixture
            .state
            .legal()
            .informant_disclosures()
            .filter(|record| record.source_information() == information)
            .count(),
        0
    );
    validate_state(&fixture.state).expect("stale rejection should leave valid state");
    validate_invariants(&fixture.state);
}

#[test]
fn informant_relationship_is_exclusive_and_save_round_trip_preserves_history() {
    let registry = build_registry();
    let mut fixture = fixture();
    let informant = validate_establish_informant(
        &fixture.state,
        InformantDraft {
            character: fixture.member,
            handler: fixture.police,
        },
    )
    .expect("informant establishment should validate")
    .commit(&mut fixture.state)
    .expect("informant establishment should commit");
    let information = record_personal_information(&mut fixture);
    let disclosure = validate_record_informant_disclosure(
        &fixture.state,
        InformantDisclosureDraft {
            informant,
            investigation: fixture.investigation,
            source_information: information,
        },
    )
    .expect("disclosure should validate")
    .commit(&mut fixture.state)
    .expect("disclosure should commit");

    // The relationship is exclusive: a second establishment for the same pair is rejected.
    assert!(matches!(
        validate_establish_informant(
            &fixture.state,
            InformantDraft {
                character: fixture.member,
                handler: fixture.police,
            },
        ),
        Err(InformantError::AlreadyInformant { .. })
    ));
    assert_eq!(
        fixture
            .state
            .legal()
            .get_informant(informant)
            .expect("relationship should persist")
            .id(),
        informant
    );

    let envelope = build_save(&registry, &fixture.state).expect("informant state should save");
    let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("save envelope should deserialize");
    let restored = restore_save(&registry, decoded).expect("informant save should restore");
    assert_eq!(
        restored
            .legal()
            .get_informant(informant)
            .expect("relationship should survive save")
            .id(),
        informant
    );
    assert_eq!(
        restored
            .legal()
            .informant_disclosures()
            .find(|record| record.id() == disclosure)
            .expect("disclosure should survive save")
            .source_information(),
        information
    );
    validate_invariants(&restored);
}

#[test]
fn recruitment_skips_a_detainee_already_informing_for_the_handler() {
    let registry = build_registry();
    let mut fixture = fixture();
    let case = crate::legal::investigation_system::validate_open_investigation(
        &fixture.state,
        InvestigationDraft {
            owner: fixture.police,
            title: "Member custody inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(fixture.member)]),
        },
    )
    .expect("subject case should validate")
    .commit(&mut fixture.state)
    .expect("subject case should commit");
    let evidence = crate::legal::investigation_system::validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation: case,
            custodian: fixture.police,
            subject: EntityRef::Character(fixture.member),
            origin: None,
            kind: EvidenceKind::KnownAssociation,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: fixture.state.now(),
        },
    )
    .expect("case evidence should validate")
    .commit(&mut fixture.state)
    .expect("case evidence should commit");
    let corroborating = validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation: case,
            custodian: fixture.police,
            subject: EntityRef::Character(fixture.member),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: fixture.state.now(),
        },
    )
    .expect("corroborating case evidence should validate")
    .commit(&mut fixture.state)
    .expect("corroborating case evidence should commit");

    // The member already works for this handler from an earlier stint; a re-arrest must
    // not draw a second recruitment decision, which establishment would reject and the
    // tick pipeline would treat as a bug.
    validate_establish_informant(
        &fixture.state,
        InformantDraft {
            character: fixture.member,
            handler: fixture.police,
        },
    )
    .expect("informant establishment should validate")
    .commit(&mut fixture.state)
    .expect("informant establishment should commit");
    crate::legal::arrest_system::validate_arrest(
        &registry,
        &fixture.state,
        ArrestDraft {
            character: fixture.member,
            investigation: case,
            evidence: BTreeSet::from([evidence, corroborating]),
        },
    )
    .expect("custody arrest should validate")
    .commit(&mut fixture.state)
    .expect("custody arrest should commit");

    fixture.state.advance_clock(SimDuration::from_minutes(
        registry.legal().informant_decision_delay().as_minutes(),
    ));
    let recruited = apply_detainee_informant_recruitment(&registry, &mut fixture.state)
        .expect("recruitment pass should resolve without aborting the tick");
    assert!(recruited.is_empty());
    assert_eq!(fixture.state.legal().informants().count(), 1);
    validate_state(&fixture.state).expect("post-pass state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn active_counsel_materially_reduces_detainee_flip_risk() {
    let legal = build_registry().legal();
    assert_eq!(resolve_informant_flip_chance(legal, 100, false), 75);
    assert_eq!(resolve_informant_flip_chance(legal, 100, true), 50);
    assert_eq!(resolve_informant_flip_chance(legal, 0, false), 25);
    assert_eq!(resolve_informant_flip_chance(legal, 0, true), 0);
}

#[test]
fn informant_id_exhaustion_rejects_before_consuming_investigation_rng() {
    let registry = build_registry();
    let mut fixture = fixture();
    let detainee = insert_character(
        &mut fixture.state,
        CharacterDraft {
            name: "High Safety Detainee".to_owned(),
            organization: Some(fixture.criminal),
            supervisor: None,
            autonomy: AutonomyLevel::Guided,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::from([(
                DriveKind::Safety,
                Rating::try_new(100).expect("maximum Safety drive should validate"),
            )]),
        },
    )
    .expect("high-Safety detainee should validate");
    let case = validate_open_investigation(
        &fixture.state,
        InvestigationDraft {
            owner: fixture.police,
            title: "Allocator exhaustion custody inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(detainee)]),
        },
    )
    .expect("subject case should validate")
    .commit(&mut fixture.state)
    .expect("subject case should commit");
    let evidence = validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation: case,
            custodian: fixture.police,
            subject: EntityRef::Character(detainee),
            origin: None,
            kind: EvidenceKind::KnownAssociation,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: fixture.state.now(),
        },
    )
    .expect("case evidence should validate")
    .commit(&mut fixture.state)
    .expect("case evidence should commit");
    let corroborating = validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation: case,
            custodian: fixture.police,
            subject: EntityRef::Character(detainee),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: fixture.state.now(),
        },
    )
    .expect("corroborating case evidence should validate")
    .commit(&mut fixture.state)
    .expect("corroborating case evidence should commit");
    crate::legal::arrest_system::validate_arrest(
        &registry,
        &fixture.state,
        ArrestDraft {
            character: detainee,
            investigation: case,
            evidence: BTreeSet::from([evidence, corroborating]),
        },
    )
    .expect("custody arrest should validate")
    .commit(&mut fixture.state)
    .expect("custody arrest should commit");
    fixture.state.advance_clock(SimDuration::from_minutes(
        registry.legal().informant_decision_delay().as_minutes(),
    ));
    fixture
        .state
        .ids
        .set_next_raw_for_test(IdKind::Informant, u32::MAX);
    let mut untouched = fixture.state.clone();

    let error = apply_detainee_informant_recruitment(&registry, &mut fixture.state)
        .expect_err("a successful flip must surface informant allocator exhaustion");
    assert!(matches!(
        error,
        InformantError::IdExhaustion(IdExhaustionError::Exhausted {
            kind: "informant",
            ..
        })
    ));
    assert_eq!(fixture.state.legal().informants().count(), 0);

    let after_failure =
        crate::core::simulation::draw_index(fixture.state.investigation_rng_mut(), 100)
            .expect("comparison draw should succeed");
    let untouched_draw =
        crate::core::simulation::draw_index(untouched.investigation_rng_mut(), 100)
            .expect("control draw should succeed");
    assert_eq!(
        after_failure, untouched_draw,
        "a rejected informant establishment must not advance the investigation RNG"
    );
}

#[test]
fn per_tick_scan_indexes_track_lifecycle_transitions() {
    let registry = build_registry();
    let mut fixture = fixture();
    let active_cases = |state: &crate::core::state::AppState| -> Vec<InvestigationId> {
        state
            .legal()
            .active_investigations()
            .map(|record| record.id())
            .collect()
    };
    assert_eq!(active_cases(&fixture.state), vec![fixture.investigation]);

    // Custody: an evidence-backed arrest enters the detained scan surface, and its
    // release through the canonical path leaves it.
    let case = validate_open_investigation(
        &fixture.state,
        InvestigationDraft {
            owner: fixture.police,
            title: "Member custody inquiry".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(fixture.member)]),
        },
    )
    .expect("subject case should validate")
    .commit(&mut fixture.state)
    .expect("subject case should commit");
    let evidence = crate::legal::investigation_system::validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation: case,
            custodian: fixture.police,
            subject: EntityRef::Character(fixture.member),
            origin: None,
            kind: EvidenceKind::KnownAssociation,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: fixture.state.now(),
        },
    )
    .expect("case evidence should validate")
    .commit(&mut fixture.state)
    .expect("case evidence should commit");
    let corroborating = validate_add_evidence(
        &fixture.state,
        EvidenceDraft {
            investigation: case,
            custodian: fixture.police,
            subject: EntityRef::Character(fixture.member),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: fixture.state.now(),
        },
    )
    .expect("corroborating case evidence should validate")
    .commit(&mut fixture.state)
    .expect("corroborating case evidence should commit");
    let arrest = crate::legal::arrest_system::validate_arrest(
        &registry,
        &fixture.state,
        ArrestDraft {
            character: fixture.member,
            investigation: case,
            evidence: BTreeSet::from([evidence, corroborating]),
        },
    )
    .expect("custody arrest should validate")
    .commit(&mut fixture.state)
    .expect("custody arrest should commit");
    assert_eq!(
        fixture
            .state
            .legal()
            .detained_arrests()
            .map(|record| record.id())
            .collect::<Vec<_>>(),
        vec![arrest]
    );

    // An established informant enters the disclosure scan surface.
    let informant = validate_establish_informant(
        &fixture.state,
        InformantDraft {
            character: fixture.member,
            handler: fixture.police,
        },
    )
    .expect("informant establishment should validate")
    .commit(&mut fixture.state)
    .expect("informant establishment should commit");
    assert_eq!(
        fixture
            .state
            .legal()
            .informants()
            .map(|record| record.id())
            .collect::<Vec<_>>(),
        vec![informant]
    );
    validate_invariants(&fixture.state);

    // Suspension leaves the active-case scan surface; resume re-enters it.
    validate_transition_investigation(
        &fixture.state,
        fixture.investigation,
        InvestigationTransition::Suspend,
    )
    .expect("suspension should validate")
    .commit(&mut fixture.state)
    .expect("suspension should commit");
    // Only the custody inquiry opened above stays on the active scan surface.
    assert_eq!(active_cases(&fixture.state), vec![case]);
    validate_transition_investigation(
        &fixture.state,
        fixture.investigation,
        InvestigationTransition::Resume,
    )
    .expect("resume should validate")
    .commit(&mut fixture.state)
    .expect("resume should commit");
    assert_eq!(
        active_cases(&fixture.state),
        vec![fixture.investigation, case]
    );

    crate::legal::arrest_system::validate_release_arrest(&fixture.state, arrest)
        .expect("release should validate")
        .commit(&mut fixture.state)
        .expect("release should commit");
    assert!(fixture.state.legal().detained_arrests().next().is_none());

    // The informant relationship outlives the custody that produced it and remains on the
    // disclosure scan surface.
    assert_eq!(
        fixture
            .state
            .legal()
            .informants()
            .map(|record| record.id())
            .collect::<Vec<_>>(),
        vec![informant]
    );
    validate_state(&fixture.state).expect("post-transition state should validate");
    validate_invariants(&fixture.state);
}
