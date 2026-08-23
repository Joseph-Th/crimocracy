//! Knowledge validation and recording; sibling intelligence state never infers hidden truth for callers.

use crate::core::entity::{is_entity_present, EntityRef};
use crate::core::id::{CharacterId, IdExhaustionError, InformationId, OrganizationId};
use crate::core::state::AppState;
use crate::intelligence::{
    InformationDraft, InformationRecord, InformationSourceKind, InformationTransferDraft,
    KnowledgeHolder,
};
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
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
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
    pub fn commit(self, state: &mut AppState) -> Result<InformationId, IntelligenceError> {
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
        let id = state.ids.next_information()?;
        let recorded_at = state.now();
        state.intelligence.insert(InformationRecord {
            id,
            source: super::InformationSource {
                holder,
                source_kind,
            },
            subject: super::InformationSubject {
                topic,
                source_entity,
                subject,
            },
            chronology: super::InformationChronology {
                observed_at,
                recorded_at,
            },
            assessment: super::InformationAssessment {
                reliability,
                specificity,
                derived_from: self.derived_from,
                summary,
            },
        });
        Ok(id)
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
        KnowledgeHolder::Character(id) => {
            if state.world.get_character(id).is_none() {
                return Err(IntelligenceError::MissingCharacter(id));
            }
        }
        KnowledgeHolder::Organization(id) => {
            if state.world.get_organization(id).is_none() {
                return Err(IntelligenceError::MissingOrganization(id));
            }
        }
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
        validate_internal_transfer_information(state, draft, self.source)?.commit(state)
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
    match holder {
        KnowledgeHolder::Character(character) => {
            state
                .world
                .get_character(character)
                .ok_or(IntelligenceError::MissingCharacter(character))?;
        }
        KnowledgeHolder::Organization(organization) => {
            state
                .world
                .get_organization(organization)
                .ok_or(IntelligenceError::MissingOrganization(organization))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
