//! Focused tests for information recording, transfer, lineage, and holder indexes.

use super::*;
use crate::build_registry;
use crate::core::attention::AttentionClass;
use crate::core::invariants::{validate_invariants, validate_state};
use crate::core::persistence::{build_save, restore_save, SaveEnvelope};
use crate::reports::report_system::{validate_record_report, ReportError};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::world::world_system::{
    insert_character, insert_organization, validate_reassign_character,
};
use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind};

fn make_transfer_fixture() -> (
    crate::registry::Registry,
    AppState,
    OrganizationId,
    CharacterId,
) {
    let registry = build_registry();
    let mut state = AppState::new(0x1F0A_1933);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Information Test Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("organization fixture should validate");
    let character = insert_character(
        &mut state,
        CharacterDraft {
            name: "Information Courier".to_owned(),
            organization: Some(organization),
            supervisor: None,
            autonomy: AutonomyLevel::Delegated,
            capabilities: BTreeMap::new(),
            traits: BTreeSet::new(),
            drives: BTreeMap::new(),
        },
    )
    .expect("character fixture should validate");
    (registry, state, organization, character)
}

fn record_character_information(
    state: &mut AppState,
    character: CharacterId,
    organization: OrganizationId,
) -> InformationId {
    validate_record_information(
        state,
        InformationDraft {
            holder: KnowledgeHolder::Character(character),
            source_kind: InformationSourceKind::DirectObservation,
            topic: crate::intelligence::InformationTopic::TargetSecurity,
            source_entity: None,
            subject: EntityRef::Organization(organization),
            observed_at: state.now(),
            reliability: crate::intelligence::Reliability::DirectAccess,
            specificity: crate::intelligence::Specificity::Precise,
            summary: "A member directly observed information relevant to leadership.".to_owned(),
        },
    )
    .expect("character information fixture should validate")
    .commit(state)
    .expect("character information fixture should commit")
}

#[test]
fn explicit_transfer_creates_stable_organization_knowledge_and_provenance() {
    let (registry, mut state, organization, character) = make_transfer_fixture();
    let source = record_character_information(&mut state, character, organization);

    let direct_report_error = match validate_record_report(
        &state,
        ReportDraft {
            recipient: organization,
            kind: ReportKind::Legal,
            title: "Unreported member knowledge".to_owned(),
            entries: vec![ReportEntry {
                attention: AttentionClass::Notable,
                summary: "Leadership cannot cite knowledge that has not been reported upward."
                    .to_owned(),
                sources: vec![source],
                entities: BTreeSet::from([EntityRef::Character(character)]),
                decision: None,
            }],
        },
    ) {
        Ok(_) => panic!("organization report must reject character-only knowledge"),
        Err(error) => error,
    };
    assert_eq!(
        direct_report_error,
        ReportError::InformationUnavailable {
            information: source,
            recipient: organization,
        }
    );

    let transferred = validate_information_transfer(
        &state,
        InformationTransferDraft {
            source,
            recipient: KnowledgeHolder::Organization(organization),
        },
    )
    .expect("member-to-organization transfer should validate")
    .commit(&mut state)
    .expect("validated information transfer should commit");
    let transferred_record = state
        .intelligence()
        .get_information(transferred)
        .expect("transferred information should persist");
    assert_eq!(
        transferred_record.holder(),
        KnowledgeHolder::Organization(organization)
    );
    assert_eq!(
        transferred_record.source_kind(),
        InformationSourceKind::InternalReport
    );
    assert_eq!(
        transferred_record.topic(),
        crate::intelligence::InformationTopic::TargetSecurity
    );
    assert_eq!(
        transferred_record.source_entity(),
        Some(EntityRef::Character(character))
    );
    assert_eq!(transferred_record.derived_from(), &BTreeSet::from([source]));
    assert_eq!(
        state
            .intelligence()
            .information_derived_from(source)
            .map(InformationRecord::id)
            .collect::<Vec<_>>(),
        vec![transferred]
    );
    assert_eq!(
        state
            .intelligence()
            .information_for_holder_by_topic(
                KnowledgeHolder::Organization(organization),
                crate::intelligence::InformationTopic::TargetSecurity,
            )
            .map(InformationRecord::id)
            .collect::<Vec<_>>(),
        vec![transferred]
    );

    let report = validate_record_report(
        &state,
        ReportDraft {
            recipient: organization,
            kind: ReportKind::Legal,
            title: "Reported member knowledge".to_owned(),
            entries: vec![ReportEntry {
                attention: AttentionClass::Notable,
                summary: "Leadership now possesses a provenance-bearing internal report."
                    .to_owned(),
                sources: vec![transferred],
                entities: BTreeSet::from([EntityRef::Character(character)]),
                decision: None,
            }],
        },
    )
    .expect("organization-held transfer should be reportable")
    .commit(&mut state)
    .expect("organization-held transfer report should commit");

    validate_reassign_character(&state, character, None, None)
        .expect("character should be able to leave after reporting information")
        .commit(&mut state)
        .expect("character reassignment should commit");
    validate_state(&state).expect("historical organization report must survive membership change");
    validate_invariants(&state);

    let envelope =
        build_save(&registry, &state).expect("provenance-bearing organization report should save");
    let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let decoded: SaveEnvelope =
        bincode::deserialize(&bytes).expect("save envelope should deserialize");
    let restored = restore_save(&registry, decoded).expect("provenance save should restore");
    assert!(restored.reports().get_report(report).is_some());
    assert_eq!(
        restored
            .intelligence()
            .information_derived_from(source)
            .map(InformationRecord::id)
            .collect::<Vec<_>>(),
        vec![transferred]
    );
    validate_invariants(&restored);
}

#[test]
fn transfer_token_becomes_stale_after_character_membership_change() {
    let (_registry, mut state, organization, character) = make_transfer_fixture();
    let source = record_character_information(&mut state, character, organization);
    let transfer = validate_information_transfer(
        &state,
        InformationTransferDraft {
            source,
            recipient: KnowledgeHolder::Organization(organization),
        },
    )
    .expect("transfer should validate against current membership");

    validate_reassign_character(&state, character, None, None)
        .expect("membership change should validate")
        .commit(&mut state)
        .expect("membership change should commit");
    let error = transfer
        .commit(&mut state)
        .expect_err("transfer must reject a stale character membership snapshot");
    assert_eq!(
        error,
        IntelligenceError::StaleTransferCharacter {
            character,
            expected: 1,
            found: 2,
        }
    );
    assert_eq!(
        state
            .intelligence()
            .information_derived_from(source)
            .count(),
        0
    );
    validate_invariants(&state);
}

#[test]
fn internal_transfer_rejects_unrelated_organization() {
    let (registry, mut state, organization, character) = make_transfer_fixture();
    let other = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Unrelated Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("second organization fixture should validate");
    let source = record_character_information(&mut state, character, organization);

    let error = match validate_information_transfer(
        &state,
        InformationTransferDraft {
            source,
            recipient: KnowledgeHolder::Organization(other),
        },
    ) {
        Ok(_) => panic!("internal transfer must not cross unrelated organizations"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        IntelligenceError::TransferNotPermitted {
            source_holder: KnowledgeHolder::Character(character),
            recipient: KnowledgeHolder::Organization(other),
        }
    );
    validate_invariants(&state);
}

#[test]
fn generic_information_recording_cannot_forge_internal_transfer() {
    let (_registry, mut state, organization, character) = make_transfer_fixture();
    record_character_information(&mut state, character, organization);

    let internal_report_error = match validate_record_information(
        &state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(organization),
            source_kind: InformationSourceKind::InternalReport,
            topic: crate::intelligence::InformationTopic::General,
            source_entity: Some(EntityRef::Character(character)),
            subject: EntityRef::Organization(organization),
            observed_at: state.now(),
            reliability: crate::intelligence::Reliability::DirectAccess,
            specificity: crate::intelligence::Specificity::Precise,
            summary: "This must use the canonical transfer path.".to_owned(),
        },
    ) {
        Ok(_) => panic!("generic recording must not create internal reports"),
        Err(error) => error,
    };
    assert_eq!(
        internal_report_error,
        IntelligenceError::InternalReportRequiresTransfer
    );
    validate_invariants(&state);
}
