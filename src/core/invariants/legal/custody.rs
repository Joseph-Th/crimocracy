//! Custody-cluster validation: detentions, legal representation, and confidential sources.

//! Release-safe structural validation for the legal subsystems plus persisted reports and history.

use crate::contacts::ContactStatus;
use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{CharacterId, EvidenceId};
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::delegation::{ResponsibilityFunction, ResponsibilityScope};
use crate::finance::{AccountKind, FinancialOwner, Money};
use crate::intelligence::{
    InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability, Specificity,
};
use crate::legal::informant_system::{informant_reliability, informant_strength};
use crate::legal::{
    Admissibility, ArrestStatus, EvidenceKind, InformantStatus, InvestigationStatus,
    InvestigationWorkStatus, LegalRepresentationStatus,
};
use crate::operations::ACTIVE_ASSIGNMENT_STATUSES;
use crate::reports::ReportKind;
use crate::world::{CapabilityKind, OrganizationKind};
use std::collections::BTreeSet;

pub(super) fn validate_arrests(state: &AppState) -> Result<(), StateValidationError> {
    // Characters bound to any non-terminal operation, computed once for the whole arrest
    // pass: the detained-arm check below only needs membership, so rescanning the live
    // operation set per detained arrest would be quadratic in custody volume for no
    // extra coverage.
    let booked_characters: BTreeSet<CharacterId> = ACTIVE_ASSIGNMENT_STATUSES
        .into_iter()
        .flat_map(|status| state.operations.operations_with_status(status))
        .flat_map(|operation| operation.participants())
        .collect();

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
                    })
            })
        {
            return Err(StateValidationError::InvalidArrest {
                arrest: arrest.id(),
            });
        }
        match arrest.status() {
            ArrestStatus::Detained => {
                // Only live statuses can hold the character; scanning completed operation
                // history here would grow unbounded with campaign length.
                let active_operation = booked_characters.contains(&arrest.character());
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

pub(super) fn validate_legal_representations(state: &AppState) -> Result<(), StateValidationError> {
    let mut payments = BTreeSet::new();
    let mut information_ids = BTreeSet::new();
    let mut report_ids = BTreeSet::new();
    for representation in state.legal.legal_representations() {
        let invalid = || StateValidationError::InvalidLegalRepresentation {
            representation: representation.id(),
        };
        let arrest = state
            .legal
            .get_arrest(representation.arrest())
            .ok_or_else(invalid)?;
        let _ = state
            .world
            .get_character(representation.defendant())
            .ok_or_else(invalid)?;
        let sponsor = state
            .world
            .get_organization(representation.sponsor())
            .ok_or_else(invalid)?;
        let counsel = state
            .world
            .get_character(representation.counsel())
            .ok_or_else(invalid)?;
        let firm = state
            .world
            .get_organization(representation.counsel_institution())
            .ok_or_else(invalid)?;
        let contact = state
            .contacts
            .get_contact(representation.contact())
            .ok_or_else(invalid)?;
        let payer = state
            .finance
            .get_account(representation.payer_account())
            .ok_or_else(invalid)?;
        let provider = state
            .finance
            .get_account(representation.provider_account())
            .ok_or_else(invalid)?;
        let payment = state
            .finance
            .get_transaction(representation.payment())
            .ok_or_else(invalid)?;
        let retained_information = state
            .intelligence
            .get_information(representation.information())
            .ok_or_else(invalid)?;
        let retained_report = state
            .reports
            .get_report(representation.report())
            .ok_or_else(invalid)?;

        let Some(outflow) = representation
            .fee()
            .cents()
            .checked_neg()
            .map(Money::from_cents)
        else {
            return Err(invalid());
        };
        let has_payer_posting = payment.postings().iter().any(|posting| {
            posting.account == representation.payer_account() && posting.amount == outflow
        });
        let has_provider_posting = payment.postings().iter().any(|posting| {
            posting.account == representation.provider_account()
                && posting.amount == representation.fee()
        });
        let authority_is_valid = match (representation.authorization(), payment.budget_usage()) {
            (None, None) => true,
            (Some(authority), Some(usage)) => {
                authority.scope == ResponsibilityScope::Function(ResponsibilityFunction::Legal)
                    && usage.mandate() == authority.mandate
                    && usage.manager() == authority.manager
                    && usage.scope() == authority.scope
                    && usage.funding_account() == representation.payer_account()
                    && usage.amount() == representation.fee()
            }
            (None, Some(_)) | (Some(_), None) => false,
        };
        let retained_report_is_valid = retained_report.recipient() == representation.sponsor()
            && retained_report.kind() == ReportKind::Legal
            && retained_report.title() == "Legal representation retained"
            && retained_report.generated_at() == representation.retained_at()
            && retained_report.entries().len() == 1
            && retained_report.entries()[0].attention == AttentionClass::Notable
            && retained_report.entries()[0].summary == retained_information.summary()
            && retained_report.entries()[0].sources.is_empty()
            && retained_report.entries()[0].decision.is_none()
            // Compared element-wise so per-record validation never rebuilds the entity set.
            && {
                let entities = &retained_report.entries()[0].entities;
                entities.len() == 4
                    && entities.contains(&EntityRef::Character(representation.defendant()))
                    && entities.contains(&EntityRef::Character(representation.counsel()))
                    && entities
                        .contains(&EntityRef::Organization(representation.counsel_institution()))
                    && entities.contains(&EntityRef::Investigation(arrest.investigation()))
            };

        if arrest.character() != representation.defendant()
            || sponsor.kind() != OrganizationKind::Criminal
            || firm.kind() != OrganizationKind::LegalServices
            || contact.sponsor() != representation.sponsor()
            || contact.contact() != representation.counsel()
            || contact.institution() != representation.counsel_institution()
            || contact.kind() != crate::contacts::ContactKind::Legal
            || counsel.capability(CapabilityKind::LegalKnowledge).is_none()
            || representation.fee() <= Money::ZERO
            || representation.retained_at() > state.now()
            || representation.version() == 0
            || payer.owner() != FinancialOwner::Organization(representation.sponsor())
            || !matches!(
                payer.kind(),
                AccountKind::StreetCash
                    | AccountKind::ConcealedCash
                    | AccountKind::AccountedFunds
                    | AccountKind::LegitimateOperating
            )
            || provider.owner()
                != FinancialOwner::Organization(representation.counsel_institution())
            || provider.kind() != AccountKind::LegitimateOperating
            || payment.occurred_at() != representation.retained_at()
            || payment.postings().len() != 2
            || !has_payer_posting
            || !has_provider_posting
            || !authority_is_valid
            || retained_information.holder()
                != KnowledgeHolder::Organization(representation.sponsor())
            || retained_information.source_kind() != InformationSourceKind::AfterAction
            || retained_information.topic() != InformationTopic::LegalActivity
            || retained_information.source_entity()
                != Some(EntityRef::Character(representation.counsel()))
            || retained_information.subject() != EntityRef::Character(representation.defendant())
            || retained_information.observed_at() != representation.retained_at()
            || retained_information.recorded_at() != representation.retained_at()
            || retained_information.reliability() != Reliability::DirectAccess
            || retained_information.specificity() != Specificity::Precise
            || !retained_information.derived_from().is_empty()
            || retained_information.summary().trim().is_empty()
            || !retained_report_is_valid
            || !payments.insert(representation.payment())
            || !information_ids.insert(representation.information())
            || !report_ids.insert(representation.report())
        {
            return Err(invalid());
        }

        match representation.status() {
            LegalRepresentationStatus::Active => {
                if representation.version() != 1
                    || representation.ended_at().is_some()
                    || representation.end_reason().is_some()
                    || representation.ended_information().is_some()
                    || representation.ended_report().is_some()
                    || contact.status() != ContactStatus::Active
                    || counsel.organization() != Some(representation.counsel_institution())
                    || state
                        .legal
                        .active_representation_for_arrest(representation.arrest())
                        .is_none_or(|active| active.id() != representation.id())
                {
                    return Err(invalid());
                }
            }
            LegalRepresentationStatus::Ended => {
                let ended_at = representation.ended_at().ok_or_else(invalid)?;
                let ended_information_id =
                    representation.ended_information().ok_or_else(invalid)?;
                let ended_report_id = representation.ended_report().ok_or_else(invalid)?;
                let ended_information = state
                    .intelligence
                    .get_information(ended_information_id)
                    .ok_or_else(invalid)?;
                let ended_report = state
                    .reports
                    .get_report(ended_report_id)
                    .ok_or_else(invalid)?;
                // Safe first-entry extraction: an empty entry list rejects here instead of
                // panicking, and the exact single-entry contract is enforced below.
                let Some(ended_entry) = ended_report.entries().first() else {
                    return Err(invalid());
                };
                let ended_entities = &ended_entry.entities;
                if representation.version() != 2
                    || ended_at < representation.retained_at()
                    || ended_at > state.now()
                    || representation.end_reason().is_none()
                    || state
                        .legal
                        .active_representation_for_arrest(representation.arrest())
                        .is_some_and(|active| active.id() == representation.id())
                    || ended_information.holder()
                        != KnowledgeHolder::Organization(representation.sponsor())
                    || ended_information.source_kind() != InformationSourceKind::AfterAction
                    || ended_information.topic() != InformationTopic::LegalActivity
                    || ended_information.source_entity()
                        != Some(EntityRef::Character(representation.counsel()))
                    || ended_information.subject()
                        != EntityRef::Character(representation.defendant())
                    || ended_information.observed_at() != ended_at
                    || ended_information.recorded_at() != ended_at
                    || ended_information.reliability() != Reliability::DirectAccess
                    || ended_information.specificity() != Specificity::Precise
                    || !ended_information.derived_from().is_empty()
                    || ended_information.summary().trim().is_empty()
                    || ended_report.recipient() != representation.sponsor()
                    || ended_report.kind() != ReportKind::Legal
                    || ended_report.title() != "Legal representation ended"
                    || ended_report.generated_at() != ended_at
                    || ended_report.entries().len() != 1
                    || ended_report.entries()[0].attention != AttentionClass::Notable
                    || ended_report.entries()[0].summary != ended_information.summary()
                    || !ended_report.entries()[0].sources.is_empty()
                    || ended_report.entries()[0].decision.is_some()
                    || ended_entities.len() != 3
                    || !ended_entities.contains(&EntityRef::Character(representation.defendant()))
                    || !ended_entities.contains(&EntityRef::Character(representation.counsel()))
                    || !ended_entities.contains(&EntityRef::Organization(
                        representation.counsel_institution(),
                    ))
                    || !information_ids.insert(ended_information_id)
                    || !report_ids.insert(ended_report_id)
                {
                    return Err(invalid());
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
            || informant.version() == 0
        {
            return Err(StateValidationError::InvalidInformant {
                informant: informant.id(),
            });
        }
        if informant.status() != InformantStatus::Active
            || character.organization() == Some(informant.handler())
        {
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
