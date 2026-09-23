//! Knowledge validation and recording; sibling intelligence state never infers hidden truth for callers.

use crate::core::entity::{EntityRef, is_entity_present};
use crate::core::id::{
    ArrestId, CharacterId, IdExhaustionError, IdKind, InformationId, OrganizationId,
};
use crate::core::state::AppState;
use crate::intelligence::{
    InformationDraft, InformationRecord, InformationSignal, InformationSourceKind,
    InformationTopic, InformationTransferDraft, KnowledgeHolder,
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
    #[error("arrest {0} does not exist")]
    MissingArrest(ArrestId),
    #[error("entity {0:?} does not exist")]
    MissingEntity(EntityRef),
    #[error("observation time cannot be later than the current simulation time")]
    ObservationInFuture,
    #[error(
        "information signal {signal:?} is incompatible with topic {topic:?} and subject {subject:?}"
    )]
    InvalidSignal {
        signal: InformationSignal,
        topic: InformationTopic,
        subject: EntityRef,
    },
    #[error("internal-report information must be created through the transfer system")]
    InternalReportRequiresTransfer,
    #[error(
        "contact-derived information source kind {0:?} must be created through contact disclosure"
    )]
    ContactSourceRequiresDisclosure(InformationSourceKind),
    #[error(
        "system-authored information source kind {0:?} must be created through its owning system"
    )]
    SystemSourceRequiresOwner(InformationSourceKind),
    #[error("internal-report information must retain provenance and a source entity")]
    InternalReportMissingProvenance,
    #[error("internal-report source entity does not match its sole source information holder")]
    InternalReportSourceMismatch,
    #[error(
        "information source kind {0:?} cannot be created as an institutional-contact derivation"
    )]
    InvalidContactSourceKind(InformationSourceKind),
    #[error("information source kind {0:?} cannot be created through the system-authored path")]
    InvalidSystemSourceKind(InformationSourceKind),
    #[error(
        "institutional-contact source information {information} is not personally held by character {contact}"
    )]
    InvalidContactInformationSource {
        information: InformationId,
        contact: CharacterId,
    },
    #[error("source and recipient knowledge holders are identical")]
    SameHolder,
    #[error(
        "information {source_information} was already transferred to {recipient:?} as information {existing}"
    )]
    DuplicateTransfer {
        source_information: InformationId,
        recipient: KnowledgeHolder,
        existing: InformationId,
    },
    #[error("knowledge cannot be transferred internally from {source_holder:?} to {recipient:?}")]
    TransferNotPermitted {
        source_holder: KnowledgeHolder,
        recipient: KnowledgeHolder,
    },
    #[error(
        "character {character} changed after transfer validation; expected version {expected}, found {found}"
    )]
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
    if !source_kind.is_contact_derivation() {
        return Err(IntelligenceError::InvalidContactSourceKind(source_kind));
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
        reliability: super::downgraded_reliability_for_contact_derivation(
            source_record.reliability(),
        ),
        specificity: super::downgraded_specificity_for_contact_derivation(
            source_record.specificity(),
        ),
        summary: source_record.summary().to_owned(),
    };
    validate_information_draft(state, &draft)?;
    if let Some(signal) = source_record.signal() {
        validate_information_signal(state, &draft, signal)?;
    }
    Ok(ValidatedInformation {
        draft,
        signal: source_record.signal().cloned(),
        derived_from: BTreeSet::from([source]),
    })
}

pub struct ValidatedInformation {
    draft: InformationDraft,
    signal: Option<InformationSignal>,
    derived_from: BTreeSet<InformationId>,
}

/// Read-only identity/ownership projection for an information record that a validated composite
/// operation will commit before a dependent artifact. The information owner derives both fields
/// from its validated token so downstream systems never invent knowledge ownership for a future
/// record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PlannedInformationSource {
    id: InformationId,
    holder: KnowledgeHolder,
}

impl PlannedInformationSource {
    pub(crate) fn id(self) -> InformationId {
        self.id
    }

    pub(crate) fn holder(self) -> KnowledgeHolder {
        self.holder
    }
}

impl ValidatedInformation {
    pub(crate) fn planned_source(&self, state: &AppState) -> PlannedInformationSource {
        PlannedInformationSource {
            id: InformationId::from_raw(state.ids.next_raw(IdKind::Information)),
            holder: self.draft.holder,
        }
    }

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
                signal: self.signal,
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
    validate_direct_recording_source_kind(draft.source_kind)?;
    validate_information_draft(state, &draft)?;
    Ok(ValidatedInformation {
        draft,
        signal: None,
        derived_from: BTreeSet::new(),
    })
}

pub(crate) fn validate_record_system_information(
    state: &AppState,
    draft: InformationDraft,
) -> Result<ValidatedInformation, IntelligenceError> {
    validate_system_recording_source_kind(draft.source_kind)?;
    validate_information_draft(state, &draft)?;
    Ok(ValidatedInformation {
        draft,
        signal: None,
        derived_from: BTreeSet::new(),
    })
}

pub(crate) fn validate_record_information_with_signal(
    state: &AppState,
    draft: InformationDraft,
    signal: InformationSignal,
) -> Result<ValidatedInformation, IntelligenceError> {
    validate_direct_recording_source_kind(draft.source_kind)?;
    validate_information_draft(state, &draft)?;
    validate_information_signal(state, &draft, &signal)?;
    Ok(ValidatedInformation {
        draft,
        signal: Some(signal),
        derived_from: BTreeSet::new(),
    })
}

/// Canonical composition hook for case-witness registration. The legal owner validates the
/// registration before calling this helper, but the witness record does not exist yet because
/// information allocation must be preflighted before the first authoritative mutation.
///
/// Keep this narrower than the generic typed-information path: only a first-hand witness fact
/// observed at the current instant can use the planned-registration exception. Restore and every
/// later consumer still require the persisted registration through the ordinary semantic check.
pub(crate) fn validate_record_planned_case_witness_information(
    state: &AppState,
    draft: InformationDraft,
    investigation: crate::core::id::InvestigationId,
    witness: crate::core::id::CharacterId,
) -> Result<ValidatedInformation, IntelligenceError> {
    validate_direct_recording_source_kind(draft.source_kind)?;
    validate_information_draft(state, &draft)?;
    let signal = InformationSignal::LegalPersonStatus(
        crate::intelligence::LegalPersonStatusSignal::CaseWitness { investigation },
    );
    if draft.source_kind != InformationSourceKind::DirectObservation
        || draft.subject != EntityRef::Character(witness)
        || draft.observed_at != state.now()
        || !signal.is_compatible(draft.topic, draft.subject)
        || !is_entity_present(state, EntityRef::Investigation(investigation))
    {
        return Err(IntelligenceError::InvalidSignal {
            signal,
            topic: draft.topic,
            subject: draft.subject,
        });
    }
    Ok(ValidatedInformation {
        draft,
        signal: Some(signal),
        derived_from: BTreeSet::new(),
    })
}

pub(crate) fn validate_record_system_information_with_signal(
    state: &AppState,
    draft: InformationDraft,
    signal: InformationSignal,
) -> Result<ValidatedInformation, IntelligenceError> {
    validate_system_recording_source_kind(draft.source_kind)?;
    validate_information_draft(state, &draft)?;
    validate_information_signal(state, &draft, &signal)?;
    Ok(ValidatedInformation {
        draft,
        signal: Some(signal),
        derived_from: BTreeSet::new(),
    })
}

fn validate_direct_recording_source_kind(
    source_kind: InformationSourceKind,
) -> Result<(), IntelligenceError> {
    if source_kind == InformationSourceKind::InternalReport {
        return Err(IntelligenceError::InternalReportRequiresTransfer);
    }
    if source_kind.is_contact_derivation() {
        return Err(IntelligenceError::ContactSourceRequiresDisclosure(
            source_kind,
        ));
    }
    match source_kind {
        InformationSourceKind::DirectObservation | InformationSourceKind::StreetRumor => Ok(()),
        InformationSourceKind::Accounting
        | InformationSourceKind::Surveillance
        | InformationSourceKind::AcquiredRecords
        | InformationSourceKind::AfterAction => {
            Err(IntelligenceError::SystemSourceRequiresOwner(source_kind))
        }
        InformationSourceKind::PoliceContact
        | InformationSourceKind::PoliticalContact
        | InformationSourceKind::ProfessionalContact
        | InformationSourceKind::LegalContact
        | InformationSourceKind::InternalReport => {
            unreachable!("derived source kinds are rejected before direct-source matching")
        }
    }
}

fn validate_system_recording_source_kind(
    source_kind: InformationSourceKind,
) -> Result<(), IntelligenceError> {
    match source_kind {
        InformationSourceKind::Accounting
        | InformationSourceKind::Surveillance
        | InformationSourceKind::AcquiredRecords
        | InformationSourceKind::AfterAction => Ok(()),
        InformationSourceKind::DirectObservation
        | InformationSourceKind::StreetRumor
        | InformationSourceKind::PoliceContact
        | InformationSourceKind::PoliticalContact
        | InformationSourceKind::ProfessionalContact
        | InformationSourceKind::LegalContact
        | InformationSourceKind::InternalReport => {
            Err(IntelligenceError::InvalidSystemSourceKind(source_kind))
        }
    }
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
    if let Some(source) = draft.source_entity
        && !is_entity_present(state, source)
    {
        return Err(IntelligenceError::MissingEntity(source));
    }
    if draft.observed_at > state.now() {
        return Err(IntelligenceError::ObservationInFuture);
    }
    Ok(())
}

fn validate_information_signal(
    state: &AppState,
    draft: &InformationDraft,
    signal: &InformationSignal,
) -> Result<(), IntelligenceError> {
    if !signal.is_compatible(draft.topic, draft.subject) {
        return Err(IntelligenceError::InvalidSignal {
            signal: signal.clone(),
            topic: draft.topic,
            subject: draft.subject,
        });
    }
    if let InformationSignal::LegalPersonStatus(
        crate::intelligence::LegalPersonStatusSignal::Detained { arrest },
    ) = signal
        && state.legal.get_arrest(*arrest).is_none()
    {
        return Err(IntelligenceError::MissingArrest(*arrest));
    }
    if !information_signal_matches_subject_history(state, draft.subject, draft.observed_at, signal)
    {
        return Err(IntelligenceError::InvalidSignal {
            signal: signal.clone(),
            topic: draft.topic,
            subject: draft.subject,
        });
    }
    for entity in signal.referenced_entities() {
        if !is_entity_present(state, entity) {
            return Err(IntelligenceError::MissingEntity(entity));
        }
    }
    Ok(())
}

/// Semantic validation for typed facts that carry a durable legal-episode reference.
///
/// Compatibility alone only proves that a legal-person signal names a character. This check
/// additionally proves that the referenced arrest or witness registration actually belongs to
/// that character and already existed when the fact was observed. A release in the same minute
/// remains admissible because cross-domain ordering inside one simulation minute is not persisted.
pub(crate) fn information_signal_matches_subject_history(
    state: &AppState,
    subject: EntityRef,
    observed_at: crate::core::time::SimTime,
    signal: &InformationSignal,
) -> bool {
    match signal {
        InformationSignal::LegalPersonStatus(
            crate::intelligence::LegalPersonStatusSignal::CaseWitness { investigation },
        ) => {
            let EntityRef::Character(character) = subject else {
                return false;
            };
            state
                .legal
                .case_witness_for(*investigation, character)
                .is_some_and(|witness| witness.registered_at() <= observed_at)
        }
        InformationSignal::LegalPersonStatus(
            crate::intelligence::LegalPersonStatusSignal::Detained { arrest },
        ) => {
            let EntityRef::Character(character) = subject else {
                return false;
            };
            state.legal.get_arrest(*arrest).is_some_and(|record| {
                record.character() == character
                    && record.arrested_at() <= observed_at
                    && record
                        .released_at()
                        .is_none_or(|released_at| observed_at <= released_at)
            })
        }
        InformationSignal::CaseActivity(_)
        | InformationSignal::EnterpriseLocation(_)
        | InformationSignal::PersonnelPresence { .. }
        | InformationSignal::PatrolPattern { .. } => true,
    }
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
    if let Some(signal) = source_record.signal() {
        validate_information_signal(state, &draft, signal)?;
    }
    Ok(ValidatedInformation {
        draft,
        signal: source_record.signal().cloned(),
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
        ensure_transfer_not_duplicate(state, self.source, self.recipient)?;
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
    ensure_transfer_not_duplicate(state, draft.source, draft.recipient)?;
    Ok(ValidatedInformationTransfer {
        source: draft.source,
        recipient: draft.recipient,
        expected_character_versions,
    })
}

fn ensure_transfer_not_duplicate(
    state: &AppState,
    source: InformationId,
    recipient: KnowledgeHolder,
) -> Result<(), IntelligenceError> {
    if let Some(existing) = state.intelligence.internal_transfer_for(source, recipient) {
        return Err(IntelligenceError::DuplicateTransfer {
            source_information: source,
            recipient,
            existing: existing.id(),
        });
    }
    Ok(())
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
    validate_holder_exists(state, source)?;
    validate_holder_exists(state, recipient)?;
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

fn validate_holder_exists(
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
