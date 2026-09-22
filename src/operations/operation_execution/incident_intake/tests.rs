//! Operation incident-intake integration tests for witnesses, evidence, and custody boundaries.

use super::*;
use crate::build_registry;
use crate::core::state::AppState;
use crate::core::time::SimDuration;
use crate::legal::arrest_system::validate_arrest;
use crate::legal::investigation_system::{validate_add_evidence, validate_open_investigation};
use crate::legal::{
    Admissibility, ArrestDraft, EvidenceDraft, EvidenceKind, EvidenceReliability, EvidenceStrength,
    InvestigationDraft,
};
use crate::operations::operation_system::validate_authorize_operation;
use crate::operations::{
    OperationApproach, OperationDraft, OperationExposureFactors, OperationExposureLevel,
    OperationKind, OperationObjective, RoleKind,
};
use crate::world::world_system::{
    insert_business, insert_character, insert_neighborhood, insert_organization,
};
use crate::world::{
    AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner, CapabilityKind,
    CharacterDraft, NeighborhoodDraft, NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile,
    NeighborhoodProfile, OrganizationDraft, OrganizationKind, Rating,
};
use std::collections::{BTreeMap, BTreeSet};

fn rating(value: u8) -> Rating {
    Rating::try_new(value).expect("test rating should validate")
}

#[test]
fn detained_business_owner_is_not_manufactured_as_on_scene_witness() {
    let registry = build_registry();
    let mut state = AppState::new(0x1AC1_DE17);
    let crew = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Witness Availability Crew".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("crew should validate");
    let police = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Witness Availability Bureau".to_owned(),
            kind: OrganizationKind::LawEnforcement,
        },
    )
    .expect("police should validate");
    let neighborhood = insert_neighborhood(
        &mut state,
        NeighborhoodDraft {
            name: "Witness Availability Ward".to_owned(),
            profile: NeighborhoodProfile {
                economy: NeighborhoodEconomyProfile {
                    wealth: rating(50),
                    commercial_activity: rating(50),
                    illicit_demand: rating(50),
                },
                institutions: NeighborhoodInstitutionProfile {
                    police_presence: rating(50),
                },
            },
        },
    )
    .expect("neighborhood should validate");
    let owner = insert_character(
        &mut state,
        CharacterDraft {
            name: "Detained Shopkeeper".to_owned(),
            organization: None,
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("owner should validate");
    let business = insert_business(
        &registry,
        &mut state,
        BusinessDraft {
            name: "Detained Owner Store".to_owned(),
            kind: BusinessKind::Retail,
            functions: BTreeSet::from([BusinessFunction::CustomerAccess]),
            neighborhood,
            owner: BusinessOwner::Character(owner),
        },
    )
    .expect("business should validate");
    let leader = insert_character(
        &mut state,
        CharacterDraft {
            name: "Collection Leader".to_owned(),
            organization: Some(crew),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::from([(CapabilityKind::Management, rating(80))]),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("leader should validate");
    let operation = validate_authorize_operation(
        &registry,
        &state,
        OperationDraft {
            title: "Witness availability collection".to_owned(),
            kind: OperationKind::Intimidation,
            responsible_organization: crew,
            leader,
            objective: OperationObjective::ObtainCash {
                target: EntityRef::Business(business),
            },
            approach: OperationApproach::Intimidating,
            roles: BTreeMap::from([(RoleKind::Coordinator, leader)]),
            intelligence: BTreeSet::new(),
            constraints: Vec::new(),
            contingencies: Vec::new(),
            scheduled_for: state.now() + SimDuration::ONE_MINUTE,
        },
    )
    .expect("operation should validate")
    .commit(&mut state)
    .expect("operation should commit");
    let exposure = OperationExposurePlan {
        level: OperationExposureLevel::Witnessed,
        score: 50,
        factors: OperationExposureFactors {
            stealth_average: rating(0),
            target_police_presence: Some(rating(50)),
            police_response_arrived: false,
            approach_adjustment: 0,
            intelligence_mitigation: 0,
            variance: 0,
        },
        neighborhood: Some(neighborhood),
        identified_character: None,
    };
    let operation_record = state
        .operations()
        .get_operation(operation)
        .expect("operation should persist");
    let execution = registry
        .get_operation(OperationKind::Intimidation)
        .execution();
    assert_eq!(
        resolve_incident_witness(
            execution,
            &state,
            operation_record,
            &exposure,
            Some(rating(50))
        )
        .map(|witness| witness.character),
        Some(owner),
        "an available character-owner remains the modeled on-scene witness"
    );

    let investigation = validate_open_investigation(
        &state,
        InvestigationDraft {
            owner: police,
            title: "Shopkeeper custody".to_owned(),
            subjects: BTreeSet::from([EntityRef::Character(owner)]),
        },
    )
    .expect("custody investigation should validate")
    .commit(&mut state)
    .expect("custody investigation should commit");
    let strong = validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(owner),
            origin: None,
            kind: EvidenceKind::Document,
            strength: EvidenceStrength::Strong,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )
    .expect("strong custody evidence should validate")
    .commit(&mut state)
    .expect("strong custody evidence should commit");
    let corroborating = validate_add_evidence(
        &state,
        EvidenceDraft {
            investigation,
            custodian: police,
            subject: EntityRef::Character(owner),
            origin: None,
            kind: EvidenceKind::KnownAssociation,
            strength: EvidenceStrength::Corroborating,
            reliability: EvidenceReliability::HighlyReliable,
            admissibility: Admissibility::Admissible,
            discovered_at: state.now(),
        },
    )
    .expect("corroborating custody evidence should validate")
    .commit(&mut state)
    .expect("corroborating custody evidence should commit");
    validate_arrest(
        &registry,
        &state,
        ArrestDraft {
            character: owner,
            investigation,
            evidence: BTreeSet::from([strong, corroborating]),
        },
    )
    .expect("evidence-backed owner detention should validate")
    .commit(&mut state)
    .expect("owner detention should commit");

    let operation_record = state
        .operations()
        .get_operation(operation)
        .expect("operation should persist after unrelated custody");
    assert!(
        resolve_incident_witness(
            execution,
            &state,
            operation_record,
            &exposure,
            Some(rating(50))
        )
        .is_none(),
        "a detained owner cannot be the operation's on-scene eyewitness"
    );
}
