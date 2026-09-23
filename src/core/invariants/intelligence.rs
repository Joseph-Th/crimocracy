//! Release-safe structural validation for persisted intelligence and provenance.

use crate::core::entity::{EntityRef, is_entity_present};
use crate::core::id::InformationId;
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::intelligence::intelligence_system::information_signal_matches_subject_history;
use crate::intelligence::{
    InformationRecord, InformationSignal, InformationSourceKind, KnowledgeHolder,
    downgraded_reliability_for_contact_derivation, downgraded_specificity_for_contact_derivation,
};
use crate::operations::OperationKind;
use std::collections::BTreeSet;

pub(super) fn validate_intelligence(state: &AppState) -> Result<(), StateValidationError> {
    let system_owners = collect_system_information_owners(state);
    let mut previous_recorded_at = None;
    for information in state.intelligence.information() {
        validate_information(state, information, &system_owners)?;
        if previous_recorded_at.is_some_and(|at| information.recorded_at() < at) {
            return Err(StateValidationError::InvalidInformationChronology {
                information: information.id(),
            });
        }
        previous_recorded_at = Some(information.recorded_at());
    }
    Ok(())
}

fn validate_information(
    state: &AppState,
    information: &InformationRecord,
    system_owners: &SystemInformationOwners,
) -> Result<(), StateValidationError> {
    validate_information_references(state, information)?;
    if information.observed_at() > information.recorded_at()
        || information.recorded_at() > state.now()
    {
        return Err(StateValidationError::InvalidInformationChronology {
            information: information.id(),
        });
    }
    if let Some(signal) = information.signal() {
        if !signal.is_compatible(information.topic(), information.subject()) {
            return Err(StateValidationError::InvalidInformationSignal {
                information: information.id(),
            });
        }
        if !information_signal_matches_subject_history(
            state,
            information.subject(),
            information.observed_at(),
            signal,
        ) {
            return Err(StateValidationError::InvalidInformationSignal {
                information: information.id(),
            });
        }
        if let InformationSignal::LegalPersonStatus(
            crate::intelligence::LegalPersonStatusSignal::Detained { arrest },
        ) = signal
            && state.legal.get_arrest(*arrest).is_none()
        {
            return Err(StateValidationError::InvalidInformationSignal {
                information: information.id(),
            });
        }
        for entity in signal.referenced_entities() {
            if !is_entity_present(state, entity) {
                return Err(StateValidationError::MissingEntity {
                    context: "information signal",
                    entity,
                });
            }
        }
    }
    if information.source_kind() == InformationSourceKind::InternalReport {
        validate_internal_report_provenance(state, information)?;
    } else if information.source_kind().is_contact_derivation()
        || !information.derived_from().is_empty()
    {
        validate_contact_derived_provenance(state, information)?;
    } else if matches!(
        information.source_kind(),
        InformationSourceKind::Accounting
            | InformationSourceKind::Surveillance
            | InformationSourceKind::AcquiredRecords
            | InformationSourceKind::AfterAction
    ) && !system_owners.contains(information.source_kind(), information.id())
    {
        return Err(StateValidationError::UnownedSystemInformation {
            information: information.id(),
        });
    }
    validate_information_lineage(state, information)
}

#[derive(Default)]
struct SystemInformationOwners {
    accounting: BTreeSet<InformationId>,
    surveillance: BTreeSet<InformationId>,
    acquired_records: BTreeSet<InformationId>,
    after_action: BTreeSet<InformationId>,
}

impl SystemInformationOwners {
    fn contains(&self, kind: InformationSourceKind, information: InformationId) -> bool {
        match kind {
            InformationSourceKind::Accounting => self.accounting.contains(&information),
            InformationSourceKind::Surveillance => self.surveillance.contains(&information),
            InformationSourceKind::AcquiredRecords => self.acquired_records.contains(&information),
            InformationSourceKind::AfterAction => self.after_action.contains(&information),
            InformationSourceKind::DirectObservation
            | InformationSourceKind::PoliceContact
            | InformationSourceKind::PoliticalContact
            | InformationSourceKind::ProfessionalContact
            | InformationSourceKind::LegalContact
            | InformationSourceKind::StreetRumor
            | InformationSourceKind::InternalReport => false,
        }
    }
}

fn collect_system_information_owners(state: &AppState) -> SystemInformationOwners {
    let mut owners = SystemInformationOwners::default();

    for cycle in state.economy.cycles() {
        if let Some(information) = cycle.information() {
            owners.accounting.insert(information);
        }
    }
    for cycle in state.enterprises.cycles() {
        if let Some(information) = cycle.information() {
            owners.after_action.insert(information);
        }
    }
    for attempt in state.recruitment.attempts() {
        owners.after_action.insert(attempt.outcome_information());
    }
    for representation in state.legal.legal_representations() {
        owners.after_action.insert(representation.information());
        if let Some(information) = representation.ended_information() {
            owners.after_action.insert(information);
        }
    }
    for case in state.legal.prosecution_cases() {
        if let Some(information) = case.resolution_information() {
            owners.after_action.insert(information);
        }
    }
    for referral in state.legal.prosecution_referrals() {
        owners.after_action.insert(referral.information());
    }

    for operation in state.operations.operations() {
        if let Some(disposition) = operation.property_disposition() {
            owners.accounting.insert(disposition.information());
        }
        if let Some(disposition) = operation.cash_disposition() {
            owners.accounting.insert(disposition.information());
        }
        if let Some(abort) = operation.abort_record() {
            let artifacts = abort.artifacts();
            owners.after_action.insert(artifacts.information());
            if let Some(information) = artifacts.police_activity_information() {
                owners.after_action.insert(information);
            }
        }
        let Some(resolution) = operation.resolution() else {
            continue;
        };
        owners
            .after_action
            .insert(resolution.after_action_information());
        if operation.kind() == OperationKind::Surveillance {
            owners
                .surveillance
                .extend(resolution.discovered_information().iter().copied());
        } else if operation.kind() == OperationKind::DocumentTheft {
            owners
                .acquired_records
                .extend(resolution.discovered_information().iter().copied());
        }
        owners
            .after_action
            .extend(resolution.participant_information().values().copied());
    }

    owners
}

fn validate_information_references(
    state: &AppState,
    information: &InformationRecord,
) -> Result<(), StateValidationError> {
    if information.summary().trim().is_empty() {
        return Err(StateValidationError::EmptyInformationSummary {
            information: information.id(),
        });
    }
    match information.holder() {
        KnowledgeHolder::Character(id) if state.world.get_character(id).is_none() => {
            return Err(StateValidationError::MissingEntity {
                context: "information holder",
                entity: EntityRef::Character(id),
            });
        }
        KnowledgeHolder::Organization(id) if state.world.get_organization(id).is_none() => {
            return Err(StateValidationError::MissingEntity {
                context: "information holder",
                entity: EntityRef::Organization(id),
            });
        }
        KnowledgeHolder::Character(_) | KnowledgeHolder::Organization(_) => {}
    }
    if !is_entity_present(state, information.subject()) {
        return Err(StateValidationError::MissingEntity {
            context: "information subject",
            entity: information.subject(),
        });
    }
    if let Some(source) = information.source_entity()
        && !is_entity_present(state, source)
    {
        return Err(StateValidationError::MissingEntity {
            context: "information source",
            entity: source,
        });
    }
    Ok(())
}

fn validate_internal_report_provenance(
    state: &AppState,
    information: &InformationRecord,
) -> Result<(), StateValidationError> {
    if information.derived_from().len() != 1 || information.source_entity().is_none() {
        return Err(invalid_provenance(information, information.id()));
    }
    let source = *information
        .derived_from()
        .iter()
        .next()
        .expect("validated internal report must have one provenance record");
    let source_record = state
        .intelligence
        .get_information(source)
        .ok_or_else(|| invalid_provenance(information, source))?;
    if !derived_information_matches_source(information, source_record)
        || information.source_entity() != Some(source_record.holder().entity())
    {
        return Err(invalid_provenance(information, source));
    }
    Ok(())
}

fn validate_contact_derived_provenance(
    state: &AppState,
    information: &InformationRecord,
) -> Result<(), StateValidationError> {
    let source = information
        .derived_from()
        .iter()
        .next()
        .copied()
        .ok_or_else(|| invalid_provenance(information, information.id()))?;
    let source_record = state
        .intelligence
        .get_information(source)
        .ok_or_else(|| invalid_provenance(information, source))?;
    if !information.source_kind().is_contact_derivation()
        || information.derived_from().len() != 1
        || state
            .contacts
            .disclosure_for_information(information.id())
            .is_none()
        || information.source_entity() != Some(source_record.holder().entity())
        || !matches!(source_record.holder(), KnowledgeHolder::Character(_))
        || !derived_contact_information_matches_source(information, source_record)
    {
        return Err(invalid_provenance(information, source));
    }
    Ok(())
}

/// Contact-channel re-derivation: identity, timing, signal, and summary cross the hop
/// untouched, but epistemic grade steps down one rung per
/// [`downgraded_reliability_for_contact_derivation`]. Internal transfers keep the exact
/// grade via [`derived_information_matches_source`]; the two paths must not share a
/// matcher.
fn derived_contact_information_matches_source(
    information: &InformationRecord,
    source: &InformationRecord,
) -> bool {
    information.topic() == source.topic()
        && information.subject() == source.subject()
        && information.observed_at() == source.observed_at()
        && information.reliability()
            == downgraded_reliability_for_contact_derivation(source.reliability())
        && information.specificity()
            == downgraded_specificity_for_contact_derivation(source.specificity())
        && information.signal() == source.signal()
        && information.summary() == source.summary()
}

fn derived_information_matches_source(
    information: &InformationRecord,
    source: &InformationRecord,
) -> bool {
    information.topic() == source.topic()
        && information.subject() == source.subject()
        && information.observed_at() == source.observed_at()
        && information.reliability() == source.reliability()
        && information.specificity() == source.specificity()
        && information.signal() == source.signal()
        && information.summary() == source.summary()
}

fn validate_information_lineage(
    state: &AppState,
    information: &InformationRecord,
) -> Result<(), StateValidationError> {
    for source in information.derived_from() {
        let source_record = state
            .intelligence
            .get_information(*source)
            .ok_or_else(|| invalid_provenance(information, *source))?;
        if *source >= information.id() || source_record.recorded_at() > information.recorded_at() {
            return Err(invalid_provenance(information, *source));
        }
    }
    Ok(())
}

fn invalid_provenance(
    information: &InformationRecord,
    source_information: crate::core::id::InformationId,
) -> StateValidationError {
    StateValidationError::InvalidInformationProvenance {
        information: information.id(),
        source_information,
    }
}
