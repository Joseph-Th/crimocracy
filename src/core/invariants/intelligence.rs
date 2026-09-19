//! Release-safe structural validation for persisted intelligence and provenance.

use crate::core::entity::{EntityRef, is_entity_present};
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::intelligence::{
    InformationRecord, InformationSignal, InformationSourceKind, KnowledgeHolder,
    downgraded_reliability_for_contact_derivation, downgraded_specificity_for_contact_derivation,
};

pub(super) fn validate_intelligence(state: &AppState) -> Result<(), StateValidationError> {
    let mut previous_recorded_at = None;
    for information in state.intelligence.information() {
        validate_information(state, information)?;
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
    } else if !information.derived_from().is_empty() {
        validate_contact_derived_provenance(state, information)?;
    }
    validate_information_lineage(state, information)
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
    let valid_contact_kind = matches!(
        information.source_kind(),
        InformationSourceKind::PoliceContact
            | InformationSourceKind::Lawyer
            | InformationSourceKind::PoliticalContact
            | InformationSourceKind::ProfessionalContact
            | InformationSourceKind::Press
    );
    if !valid_contact_kind
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
