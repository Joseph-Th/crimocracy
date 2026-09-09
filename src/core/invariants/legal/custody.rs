//! Custody-cluster validation: detentions and confidential sources.

use crate::core::entity::EntityRef;
use crate::core::id::EvidenceId;
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::intelligence::KnowledgeHolder;
use crate::legal::arrest_system::evidence_qualifies_for_custody;
use crate::legal::informant_system::{
    informant_reliability, informant_strength, information_is_relevant_to_investigation,
};
use crate::legal::{
    Admissibility, ArrestStatus, EvidenceKind, InvestigationStatus, InvestigationWorkStatus,
};
use crate::world::OrganizationKind;
use std::collections::BTreeSet;

pub(super) fn validate_arrests(state: &AppState) -> Result<(), StateValidationError> {
    for arrest in state.legal.arrests() {
        let _ = state.world.get_character(arrest.character()).ok_or(
            StateValidationError::InvalidArrest {
                arrest: arrest.id(),
            },
        )?;
        let authority = state.world.get_organization(arrest.authority()).ok_or(
            StateValidationError::InvalidArrest {
                arrest: arrest.id(),
            },
        )?;
        let investigation = state
            .legal
            .get_investigation(arrest.investigation())
            .ok_or(StateValidationError::InvalidArrest {
                arrest: arrest.id(),
            })?;
        if authority.kind() != OrganizationKind::LawEnforcement
            || investigation.owner() != arrest.authority()
            || !investigation
                .subjects()
                .contains(&EntityRef::Character(arrest.character()))
            || arrest.evidence().is_empty()
            || arrest.arrested_at() > state.now()
            || arrest.version() == 0
            || arrest.evidence().iter().any(|evidence_id| {
                state
                    .legal
                    .get_evidence(*evidence_id)
                    .is_none_or(|evidence| {
                        evidence.investigation() != arrest.investigation()
                            || evidence.custodian() != arrest.authority()
                            || evidence.subject() != EntityRef::Character(arrest.character())
                            || evidence.discovered_at() > arrest.arrested_at()
                            || !evidence_qualifies_for_custody(evidence)
                    })
            })
        {
            return Err(StateValidationError::InvalidArrest {
                arrest: arrest.id(),
            });
        }
        match arrest.status() {
            ArrestStatus::Detained => {
                let active_operation = state
                    .operations
                    .find_active_operation_booking(arrest.character())
                    .is_some();
                if arrest.released_at().is_some()
                    || arrest.version() != 1
                    || !matches!(
                        investigation.status(),
                        // A closed case may still hold its detainee: the case was cleared by
                        // arrest, and custody outlives the institutional casework.
                        InvestigationStatus::Active | InvestigationStatus::Closed
                    )
                    || state
                        .legal
                        .active_arrest_for_character(arrest.character())
                        .is_none_or(|active| active.id() != arrest.id())
                    || state
                        .legal
                        .work_for_investigator(arrest.character())
                        .any(|work| work.status() == InvestigationWorkStatus::Scheduled)
                    || active_operation
                {
                    return Err(StateValidationError::InvalidArrest {
                        arrest: arrest.id(),
                    });
                }
            }
            ArrestStatus::Released => {
                if arrest.version() != 2
                    || arrest.released_at().is_none_or(|released_at| {
                        released_at < arrest.arrested_at() || released_at > state.now()
                    })
                {
                    return Err(StateValidationError::InvalidArrest {
                        arrest: arrest.id(),
                    });
                }
            }
        }
    }

    Ok(())
}

pub(super) fn validate_informants(state: &AppState) -> Result<(), StateValidationError> {
    for informant in state.legal.informants() {
        let character = state.world.get_character(informant.character()).ok_or(
            StateValidationError::InvalidInformant {
                informant: informant.id(),
            },
        )?;
        let handler = state.world.get_organization(informant.handler()).ok_or(
            StateValidationError::InvalidInformant {
                informant: informant.id(),
            },
        )?;
        if !matches!(
            handler.kind(),
            OrganizationKind::LawEnforcement | OrganizationKind::LegalAuthority
        ) || informant.established_at() > state.now()
        {
            return Err(StateValidationError::InvalidInformant {
                informant: informant.id(),
            });
        }
        if character.organization() == Some(informant.handler()) {
            return Err(StateValidationError::InvalidInformant {
                informant: informant.id(),
            });
        }
    }

    Ok(())
}

pub(super) fn validate_informant_disclosures(
    state: &AppState,
) -> Result<BTreeSet<EvidenceId>, StateValidationError> {
    let mut informant_evidence = BTreeSet::new();
    for disclosure in state.legal.informant_disclosures() {
        let informant = state.legal.get_informant(disclosure.informant()).ok_or(
            StateValidationError::InvalidInformantDisclosure {
                disclosure: disclosure.id(),
            },
        )?;
        let investigation = state
            .legal
            .get_investigation(disclosure.investigation())
            .ok_or(StateValidationError::InvalidInformantDisclosure {
                disclosure: disclosure.id(),
            })?;
        let information = state
            .intelligence
            .get_information(disclosure.source_information())
            .ok_or(StateValidationError::InvalidInformantDisclosure {
                disclosure: disclosure.id(),
            })?;
        let evidence = state.legal.get_evidence(disclosure.evidence()).ok_or(
            StateValidationError::InvalidInformantDisclosure {
                disclosure: disclosure.id(),
            },
        )?;
        if investigation.owner() != informant.handler()
            || information.holder() != KnowledgeHolder::Character(informant.character())
            || !information_is_relevant_to_investigation(information, investigation)
            || information.recorded_at() > disclosure.disclosed_at()
            || disclosure.disclosed_at() < informant.established_at()
            || disclosure.disclosed_at() < investigation.opened_at()
            || disclosure.disclosed_at() > state.now()
            || !informant_evidence.insert(disclosure.evidence())
            || evidence.investigation() != disclosure.investigation()
            || evidence.custodian() != informant.handler()
            || evidence.subject() != information.subject()
            || evidence.origin().is_some()
            || evidence.source() != Some(EntityRef::Character(informant.character()))
            || evidence.kind() != EvidenceKind::InformantStatement
            || evidence.strength() != informant_strength(information.specificity())
            || evidence.reliability() != informant_reliability(information.reliability())
            || evidence.admissibility() != Admissibility::Unknown
            || evidence.discovered_at() != disclosure.disclosed_at()
            || !evidence.derived_from().is_empty()
        {
            return Err(StateValidationError::InvalidInformantDisclosure {
                disclosure: disclosure.id(),
            });
        }
    }

    Ok(informant_evidence)
}
