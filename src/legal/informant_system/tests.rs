//! Focused tests for detainee informant recruitment and disclosure handling.

use super::*;
use crate::build_registry;
use crate::core::invariants::{validate_invariants, validate_state};
use crate::core::persistence::{build_save, restore_save, SaveEnvelope};
use crate::core::time::SimDuration;
use crate::intelligence::intelligence_system::validate_record_information;
use crate::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, Reliability, Specificity,
};
use crate::legal::investigation_system::{
    validate_add_evidence, validate_open_investigation, validate_transition_investigation,
    InvestigationError, InvestigationTransition,
};
use crate::legal::ArrestDraft;
use crate::legal::{EvidenceDraft, InvestigationDraft};
use crate::world::world_system::{
    insert_character, insert_organization, validate_reassign_character, WorldError,
};
use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind};
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
            .expect_err("an active source must be terminated before joining its handler"),
        WorldError::ActiveInformantHandlerAssignment {
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
        .get_informant_disclosure(disclosure)
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
            .evidence_of_kind(EvidenceKind::InformantStatement)
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
            .evidence_of_kind(EvidenceKind::InformantStatement)
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
fn informant_relationship_is_versioned_and_save_round_trip_preserves_history() {
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

    // The active relationship is exclusive: a second establishment for the same pair is
    // rejected while the first one lives.
    assert!(matches!(
        validate_establish_informant(
            &fixture.state,
            InformantDraft {
                character: fixture.member,
                handler: fixture.police,
            },
        ),
        Err(InformantError::AlreadyActive { .. })
    ));
    assert_eq!(
        fixture
            .state
            .legal()
            .get_informant(informant)
            .expect("active relationship should persist")
            .status(),
        InformantStatus::Active
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
            .status(),
        InformantStatus::Active
    );
    assert_eq!(
        restored
            .legal()
            .get_informant_disclosure(disclosure)
            .expect("disclosure should survive save")
            .source_information(),
        information
    );
    validate_invariants(&restored);
}

#[test]
fn recruitment_skips_a_detainee_already_informing_for_the_handler() {
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
        &fixture.state,
        ArrestDraft {
            character: fixture.member,
            investigation: case,
            evidence: BTreeSet::from([evidence]),
        },
    )
    .expect("custody arrest should validate")
    .commit(&mut fixture.state)
    .expect("custody arrest should commit");

    fixture.state.advance_clock(SimDuration::from_minutes(
        build_registry()
            .legal()
            .informant_decision_delay()
            .as_minutes() as u32,
    ));
    let recruited = apply_detainee_informant_recruitment(&build_registry(), &mut fixture.state)
        .expect("recruitment pass should resolve without aborting the tick");
    assert!(recruited.is_empty());
    assert_eq!(
        fixture
            .state
            .legal()
            .informants()
            .filter(|informant| informant.status() == InformantStatus::Active)
            .count(),
        1
    );
    validate_state(&fixture.state).expect("post-pass state should validate");
    validate_invariants(&fixture.state);
}

#[test]
fn per_tick_scan_indexes_track_lifecycle_transitions() {
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
    let arrest = crate::legal::arrest_system::validate_arrest(
        &fixture.state,
        ArrestDraft {
            character: fixture.member,
            investigation: case,
            evidence: BTreeSet::from([evidence]),
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

    // An established informant enters the active-informant scan surface.
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
            .active_informants()
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

    // The informant relationship outlives the custody that produced it: an active source
    // stays on the scan surface until a modeled handler decision ends it.
    assert_eq!(
        fixture
            .state
            .legal()
            .active_informants()
            .map(|record| record.id())
            .collect::<Vec<_>>(),
        vec![informant]
    );
    validate_state(&fixture.state).expect("post-transition state should validate");
    validate_invariants(&fixture.state);
}
