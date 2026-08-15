//! Knowledge validation and recording; sibling intelligence state never infers hidden truth for callers.

use crate::core::entity::{is_entity_present, EntityRef};
use crate::core::id::{CharacterId, InformationId, OrganizationId};
use crate::core::state::AppState;
use crate::intelligence::{
    InformationDraft, InformationRecord, InformationSourceKind, InformationTransferDraft,
    KnowledgeHolder,
};
use crate::world::Lifecycle;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum IntelligenceError {
    #[error("information summary must not be empty")]
    EmptySummary,
    #[error("character {0} does not exist")]
    MissingCharacter(CharacterId),
    #[error("organization {0} does not exist")]
    MissingOrganization(OrganizationId),
    #[error("information record {0} does not exist")]
    MissingInformation(InformationId),
    #[error("entity {0:?} does not exist")]
    MissingEntity(EntityRef),
    #[error("observation time cannot be later than the current simulation time")]
    ObservationInFuture,
    #[error("internal-report information must be created through the transfer system")]
    InternalReportRequiresTransfer,
    #[error("internal-report information must retain provenance and a source entity")]
    InternalReportMissingProvenance,
    #[error("internal-report source entity does not match its sole source information holder")]
    InternalReportSourceMismatch,
    #[error(
        "information source kind {0:?} cannot be created as an institutional-contact derivation"
    )]
    InvalidContactSourceKind(InformationSourceKind),
    #[error("institutional-contact source information {information} is not personally held by character {contact}")]
    InvalidContactInformationSource {
        information: InformationId,
        contact: CharacterId,
    },
    #[error("information holder {0:?} is not currently active")]
    InactiveHolder(KnowledgeHolder),
    #[error("source and recipient knowledge holders are identical")]
    SameHolder,
    #[error("knowledge cannot be transferred internally from {source_holder:?} to {recipient:?}")]
    TransferNotPermitted {
        source_holder: KnowledgeHolder,
        recipient: KnowledgeHolder,
    },
    #[error("character {character} changed after transfer validation; expected version {expected}, found {found}")]
    StaleTransferCharacter {
        character: CharacterId,
        expected: u32,
        found: u32,
    },
}

pub(crate) fn validate_contact_information_derivation(
    state: &AppState,
    source: InformationId,
    contact: CharacterId,
    recipient: OrganizationId,
    source_kind: InformationSourceKind,
) -> Result<ValidatedInformation, IntelligenceError> {
    match source_kind {
        InformationSourceKind::PoliceContact
        | InformationSourceKind::Lawyer
        | InformationSourceKind::PoliticalContact
        | InformationSourceKind::ProfessionalContact
        | InformationSourceKind::Press => {}
        InformationSourceKind::DirectObservation
        | InformationSourceKind::Informant
        | InformationSourceKind::Accountant
        | InformationSourceKind::Surveillance
        | InformationSourceKind::StreetRumor
        | InformationSourceKind::Intercept
        | InformationSourceKind::AfterAction
        | InformationSourceKind::InternalReport => {
            return Err(IntelligenceError::InvalidContactSourceKind(source_kind));
        }
    }
    let source_record = state
        .intelligence
        .get_information(source)
        .ok_or(IntelligenceError::MissingInformation(source))?;
    if source_record.holder() != KnowledgeHolder::Character(contact) {
        return Err(IntelligenceError::InvalidContactInformationSource {
            information: source,
            contact,
        });
    }
    let draft = InformationDraft {
        holder: KnowledgeHolder::Organization(recipient),
        source_kind,
        topic: source_record.topic(),
        source_entity: Some(EntityRef::Character(contact)),
        subject: source_record.subject(),
        observed_at: source_record.observed_at(),
        reliability: source_record.reliability(),
        specificity: source_record.specificity(),
        summary: source_record.summary().to_owned(),
    };
    validate_information_draft(state, &draft)?;
    Ok(ValidatedInformation {
        draft,
        derived_from: BTreeSet::from([source]),
    })
}

pub struct ValidatedInformation {
    draft: InformationDraft,
    derived_from: BTreeSet<InformationId>,
}

impl ValidatedInformation {
    pub fn commit(self, state: &mut AppState) -> InformationId {
        let InformationDraft {
            holder,
            source_kind,
            topic,
            source_entity,
            subject,
            observed_at,
            reliability,
            specificity,
            summary,
        } = self.draft;
        let id = state.ids.next_information();
        let recorded_at = state.now();
        state.intelligence.insert(InformationRecord {
            id,
            holder,
            source_kind,
            topic,
            source_entity,
            subject,
            observed_at,
            recorded_at,
            reliability,
            specificity,
            derived_from: self.derived_from,
            summary,
        });
        id
    }
}

pub fn validate_record_information(
    state: &AppState,
    draft: InformationDraft,
) -> Result<ValidatedInformation, IntelligenceError> {
    if draft.source_kind == InformationSourceKind::InternalReport {
        return Err(IntelligenceError::InternalReportRequiresTransfer);
    }
    validate_information_draft(state, &draft)?;
    Ok(ValidatedInformation {
        draft,
        derived_from: BTreeSet::new(),
    })
}

fn validate_information_draft(
    state: &AppState,
    draft: &InformationDraft,
) -> Result<(), IntelligenceError> {
    if draft.summary.trim().is_empty() {
        return Err(IntelligenceError::EmptySummary);
    }
    match draft.holder {
        KnowledgeHolder::Character(id) if state.world.get_character(id).is_none() => {
            return Err(IntelligenceError::MissingCharacter(id))
        }
        KnowledgeHolder::Organization(id) if state.world.get_organization(id).is_none() => {
            return Err(IntelligenceError::MissingOrganization(id))
        }
        KnowledgeHolder::Character(_) | KnowledgeHolder::Organization(_) => {}
    }
    if !is_entity_present(state, draft.subject) {
        return Err(IntelligenceError::MissingEntity(draft.subject));
    }
    if let Some(source) = draft.source_entity {
        if !is_entity_present(state, source) {
            return Err(IntelligenceError::MissingEntity(source));
        }
    }
    if draft.observed_at > state.now() {
        return Err(IntelligenceError::ObservationInFuture);
    }
    Ok(())
}

fn validate_internal_transfer_information(
    state: &AppState,
    draft: InformationDraft,
    source: InformationId,
) -> Result<ValidatedInformation, IntelligenceError> {
    if draft.source_kind != InformationSourceKind::InternalReport || draft.source_entity.is_none() {
        return Err(IntelligenceError::InternalReportMissingProvenance);
    }
    validate_information_draft(state, &draft)?;
    let source_record = state
        .intelligence
        .get_information(source)
        .ok_or(IntelligenceError::MissingInformation(source))?;
    if draft.source_entity != Some(source_record.holder().entity()) {
        return Err(IntelligenceError::InternalReportSourceMismatch);
    }
    Ok(ValidatedInformation {
        draft,
        derived_from: BTreeSet::from([source]),
    })
}

pub struct ValidatedInformationTransfer {
    source: InformationId,
    recipient: KnowledgeHolder,
    expected_character_versions: BTreeMap<CharacterId, u32>,
}

impl ValidatedInformationTransfer {
    pub fn commit(self, state: &mut AppState) -> Result<InformationId, IntelligenceError> {
        let source = state
            .intelligence
            .get_information(self.source)
            .ok_or(IntelligenceError::MissingInformation(self.source))?;
        let source_holder = source.holder();
        for (character, expected) in &self.expected_character_versions {
            let record = state
                .world
                .get_character(*character)
                .ok_or(IntelligenceError::MissingCharacter(*character))?;
            if record.version() != *expected {
                return Err(IntelligenceError::StaleTransferCharacter {
                    character: *character,
                    expected: *expected,
                    found: record.version(),
                });
            }
        }
        validate_transfer_relationship(state, source_holder, self.recipient)?;
        let draft = build_transfer_draft(source, self.recipient);
        Ok(validate_internal_transfer_information(state, draft, self.source)?.commit(state))
    }
}

pub fn validate_information_transfer(
    state: &AppState,
    draft: InformationTransferDraft,
) -> Result<ValidatedInformationTransfer, IntelligenceError> {
    let source = state
        .intelligence
        .get_information(draft.source)
        .ok_or(IntelligenceError::MissingInformation(draft.source))?;
    let source_holder = source.holder();
    if source_holder == draft.recipient {
        return Err(IntelligenceError::SameHolder);
    }
    let expected_character_versions =
        validate_transfer_relationship(state, source_holder, draft.recipient)?;
    Ok(ValidatedInformationTransfer {
        source: draft.source,
        recipient: draft.recipient,
        expected_character_versions,
    })
}

fn build_transfer_draft(
    source: &InformationRecord,
    recipient: KnowledgeHolder,
) -> InformationDraft {
    InformationDraft {
        holder: recipient,
        source_kind: InformationSourceKind::InternalReport,
        topic: source.topic(),
        source_entity: Some(source.holder().entity()),
        subject: source.subject(),
        observed_at: source.observed_at(),
        reliability: source.reliability(),
        specificity: source.specificity(),
        summary: source.summary().to_owned(),
    }
}

fn validate_transfer_relationship(
    state: &AppState,
    source: KnowledgeHolder,
    recipient: KnowledgeHolder,
) -> Result<BTreeMap<CharacterId, u32>, IntelligenceError> {
    validate_holder_active(state, source)?;
    validate_holder_active(state, recipient)?;
    let permitted = match (source, recipient) {
        (KnowledgeHolder::Character(character), KnowledgeHolder::Organization(organization))
        | (KnowledgeHolder::Organization(organization), KnowledgeHolder::Character(character)) => {
            state
                .world
                .get_character(character)
                .is_some_and(|record| record.organization() == Some(organization))
        }
        (KnowledgeHolder::Character(source), KnowledgeHolder::Character(recipient)) => {
            let source_organization = state
                .world
                .get_character(source)
                .and_then(|record| record.organization());
            source_organization.is_some()
                && source_organization
                    == state
                        .world
                        .get_character(recipient)
                        .and_then(|record| record.organization())
        }
        (KnowledgeHolder::Organization(_), KnowledgeHolder::Organization(_)) => false,
    };
    if !permitted {
        return Err(IntelligenceError::TransferNotPermitted {
            source_holder: source,
            recipient,
        });
    }

    let mut versions = BTreeMap::new();
    for holder in [source, recipient] {
        if let KnowledgeHolder::Character(character) = holder {
            let record = state
                .world
                .get_character(character)
                .ok_or(IntelligenceError::MissingCharacter(character))?;
            versions.insert(character, record.version());
        }
    }
    Ok(versions)
}

fn validate_holder_active(
    state: &AppState,
    holder: KnowledgeHolder,
) -> Result<(), IntelligenceError> {
    let active = match holder {
        KnowledgeHolder::Character(character) => {
            state
                .world
                .get_character(character)
                .ok_or(IntelligenceError::MissingCharacter(character))?
                .lifecycle()
                == Lifecycle::Active
        }
        KnowledgeHolder::Organization(organization) => {
            state
                .world
                .get_organization(organization)
                .ok_or(IntelligenceError::MissingOrganization(organization))?
                .lifecycle()
                == Lifecycle::Active
        }
    };
    if !active {
        return Err(IntelligenceError::InactiveHolder(holder));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
            &registry,
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
                summary: "A member directly observed information relevant to leadership."
                    .to_owned(),
            },
        )
        .expect("character information fixture should validate")
        .commit(state)
    }

    #[test]
    fn explicit_transfer_creates_stable_organization_knowledge_and_provenance() {
        let (registry, mut state, organization, character) = make_transfer_fixture();
        let source = record_character_information(&mut state, character, organization);

        let direct_report_error = match validate_record_report(
            &state,
            ReportDraft {
                recipient: organization,
                kind: ReportKind::ExecutiveBrief,
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
                kind: ReportKind::ExecutiveBrief,
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
        .commit(&mut state);

        validate_reassign_character(&state, character, None, None)
            .expect("character should be able to leave after reporting information")
            .commit(&mut state)
            .expect("character reassignment should commit");
        validate_state(&state)
            .expect("historical organization report must survive membership change");
        validate_invariants(&state);

        let envelope = build_save(&registry, &state)
            .expect("provenance-bearing organization report should save");
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
}
