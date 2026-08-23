//! Release-safe structural validation for the legal subsystems plus persisted reports and history.

use crate::contacts::ContactStatus;
use crate::core::attention::AttentionClass;
use crate::core::entity::{is_entity_present, EntityRef};
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::delegation::{ResponsibilityFunction, ResponsibilityScope};
use crate::finance::{AccountKind, FinancialOwner, Money};
use crate::intelligence::{
    InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability, Specificity,
};
use crate::legal::informant_system::{informant_reliability, informant_strength};
use crate::legal::investigation_work_execution::{
    is_reviewable_evidence_kind, resolve_improved_evidence_reliability,
    source_evidence_forms_simple_path,
};
use crate::legal::patrol_system::is_canonical_patrol_schedule;
use crate::legal::witness_system::{resolve_witness_reliability, resolve_witness_strength};
use crate::legal::{
    Admissibility, ArrestStatus, EvidenceKind, InformantStatus, InvestigationStatus,
    InvestigationWorkFocus, InvestigationWorkKind, InvestigationWorkOutcome,
    InvestigationWorkStatus, LegalRepresentationStatus, PatrolDeploymentStatus,
    PoliceResponseStatus, ProsecutionCaseStatus, WitnessCooperation,
};
use crate::operations::OperationStatus;
use crate::reports::ReportKind;
use crate::world::{CapabilityKind, OrganizationKind};
use std::collections::BTreeSet;

fn validate_legal_representations(state: &AppState) -> Result<(), StateValidationError> {
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
        let expected_retained_entities = BTreeSet::from([
            EntityRef::Character(representation.defendant()),
            EntityRef::Character(representation.counsel()),
            EntityRef::Organization(representation.counsel_institution()),
            EntityRef::Investigation(arrest.investigation()),
        ]);
        let retained_report_is_valid = retained_report.recipient() == representation.sponsor()
            && retained_report.kind() == ReportKind::Legal
            && retained_report.title() == "Legal representation retained"
            && retained_report.generated_at() == representation.retained_at()
            && retained_report.entries().len() == 1
            && retained_report.entries()[0].attention == AttentionClass::Notable
            && retained_report.entries()[0].summary == retained_information.summary()
            && retained_report.entries()[0].sources.is_empty()
            && retained_report.entries()[0].decision.is_none()
            && retained_report.entries()[0].entities == expected_retained_entities;

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
                let expected_ended_entities = BTreeSet::from([
                    EntityRef::Character(representation.defendant()),
                    EntityRef::Character(representation.counsel()),
                    EntityRef::Organization(representation.counsel_institution()),
                ]);
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
                    || ended_report.entries()[0].entities != expected_ended_entities
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

fn validate_prosecution_cases(state: &AppState) -> Result<(), StateValidationError> {
    let mut seen_referrals = BTreeSet::new();
    let mut seen_information = BTreeSet::new();
    let mut seen_reports = BTreeSet::new();
    for case in state.legal.prosecution_cases() {
        let invalid_case = || StateValidationError::InvalidProsecutionCase { case: case.id() };
        let arrest = state
            .legal
            .get_arrest(case.arrest())
            .ok_or_else(invalid_case)?;
        let investigation = state
            .legal
            .get_investigation(case.source_investigation())
            .ok_or_else(invalid_case)?;
        let source_authority = state
            .world
            .get_organization(case.source_authority())
            .ok_or_else(invalid_case)?;
        let office = state
            .world
            .get_organization(case.prosecutor_office())
            .ok_or_else(invalid_case)?;
        let lead = state
            .world
            .get_character(case.lead_prosecutor())
            .ok_or_else(invalid_case)?;
        let defendant = state
            .world
            .get_character(case.defendant())
            .ok_or_else(invalid_case)?;
        let referral_version = u32::try_from(case.referrals().len()).map_err(|_| invalid_case())?;
        let expected_version = match case.status() {
            ProsecutionCaseStatus::Reviewing => referral_version,
            ProsecutionCaseStatus::Declined | ProsecutionCaseStatus::Closed => {
                referral_version.checked_add(1).ok_or_else(invalid_case)?
            }
        };
        if case.opened_at() > state.now()
            || case.version() != expected_version
            || case.referrals().is_empty()
            || !case.referrals().contains(&case.initial_referral())
            || arrest.character() != case.defendant()
            || arrest.investigation() != case.source_investigation()
            || arrest.authority() != case.source_authority()
            || investigation.owner() != case.source_authority()
            || source_authority.kind() != OrganizationKind::LawEnforcement
            || office.kind() != OrganizationKind::Prosecutor
            || lead.capability(CapabilityKind::LegalKnowledge).is_none()
            || case.evidence().is_empty()
            || !arrest.evidence().is_subset(case.evidence())
        {
            return Err(invalid_case());
        }

        let expected_entities = BTreeSet::from([
            EntityRef::Character(case.defendant()),
            EntityRef::Organization(case.source_authority()),
            EntityRef::Organization(case.prosecutor_office()),
            EntityRef::Character(case.lead_prosecutor()),
            EntityRef::Investigation(case.source_investigation()),
        ]);

        match case.status() {
            ProsecutionCaseStatus::Reviewing => {
                if case.resolved_at().is_some()
                    || case.resolution_information().is_some()
                    || case.resolution_report().is_some()
                    || lead.organization() != Some(case.prosecutor_office())
                    || state
                        .legal
                        .open_prosecution_case_for(case.arrest(), case.prosecutor_office())
                        .is_none_or(|open| open.id() != case.id())
                {
                    return Err(invalid_case());
                }
            }
            ProsecutionCaseStatus::Declined | ProsecutionCaseStatus::Closed => {
                let resolved_at = case.resolved_at().ok_or_else(invalid_case)?;
                let information_id = case.resolution_information().ok_or_else(invalid_case)?;
                let report_id = case.resolution_report().ok_or_else(invalid_case)?;
                let information = state
                    .intelligence
                    .get_information(information_id)
                    .ok_or_else(invalid_case)?;
                let report = state
                    .reports
                    .get_report(report_id)
                    .ok_or_else(invalid_case)?;
                let (expected_title, expected_summary) =
                    if case.status() == ProsecutionCaseStatus::Declined {
                        (
                            "Prosecution declined",
                            format!(
                                "{} declined prosecution of {} after review by {}.",
                                office.name(),
                                defendant.name(),
                                lead.name()
                            ),
                        )
                    } else {
                        (
                            "Prosecution review closed",
                            format!(
                                "{} closed its prosecution review of {} after review by {}.",
                                office.name(),
                                defendant.name(),
                                lead.name()
                            ),
                        )
                    };
                if resolved_at < case.opened_at()
                    || resolved_at > state.now()
                    || state
                        .legal
                        .open_prosecution_case_for(case.arrest(), case.prosecutor_office())
                        .is_some_and(|open| open.id() == case.id())
                    || !seen_information.insert(information_id)
                    || information.holder()
                        != KnowledgeHolder::Organization(case.prosecutor_office())
                    || information.source_kind() != InformationSourceKind::AfterAction
                    || information.topic() != InformationTopic::LegalActivity
                    || information.source_entity()
                        != Some(EntityRef::Character(case.lead_prosecutor()))
                    || information.subject() != EntityRef::Character(case.defendant())
                    || information.observed_at() != resolved_at
                    || information.recorded_at() != resolved_at
                    || information.reliability() != Reliability::DirectAccess
                    || information.specificity() != Specificity::Precise
                    || !information.derived_from().is_empty()
                    || information.summary() != expected_summary
                    || !seen_reports.insert(report_id)
                    || report.recipient() != case.prosecutor_office()
                    || report.kind() != ReportKind::Legal
                    || report.title() != expected_title
                    || report.generated_at() != resolved_at
                    || report.entries().len() != 1
                    || report.entries()[0].attention != AttentionClass::Notable
                    || report.entries()[0].summary != information.summary()
                    || !report.entries()[0].sources.is_empty()
                    || report.entries()[0].decision.is_some()
                    || report.entries()[0].entities != expected_entities
                {
                    return Err(invalid_case());
                }
            }
        }
        let mut referred_evidence = BTreeSet::new();
        for referral_id in case.referrals() {
            let invalid_referral = || StateValidationError::InvalidProsecutionReferral {
                referral: *referral_id,
            };
            let referral = state
                .legal
                .get_prosecution_referral(*referral_id)
                .ok_or_else(invalid_referral)?;
            let information = state
                .intelligence
                .get_information(referral.information())
                .ok_or_else(invalid_referral)?;
            let report = state
                .reports
                .get_report(referral.report())
                .ok_or_else(invalid_referral)?;
            let is_initial = referral.id() == case.initial_referral();
            let expected_title = if is_initial {
                "Prosecution case referral"
            } else {
                "Prosecution evidence supplement"
            };
            if !seen_referrals.insert(referral.id())
                || referral.prosecution_case() != case.id()
                || referral.source_investigation() != case.source_investigation()
                || referral.source_authority() != case.source_authority()
                || referral.prosecutor_office() != case.prosecutor_office()
                || referral.evidence().is_empty()
                || referral.referred_at() < case.opened_at()
                || referral.referred_at() > state.now()
                || case
                    .resolved_at()
                    .is_some_and(|resolved_at| referral.referred_at() > resolved_at)
                || (is_initial && referral.referred_at() != case.opened_at())
                || referral.evidence().iter().any(|evidence_id| {
                    state
                        .legal
                        .get_evidence(*evidence_id)
                        .is_none_or(|evidence| {
                            evidence.investigation() != case.source_investigation()
                                || evidence.custodian() != case.source_authority()
                                || evidence.discovered_at() > referral.referred_at()
                        })
                        || !referred_evidence.insert(*evidence_id)
                })
                || !seen_information.insert(referral.information())
                || information.holder() != KnowledgeHolder::Organization(case.prosecutor_office())
                || information.source_kind() != InformationSourceKind::AfterAction
                || information.topic() != InformationTopic::LegalActivity
                || information.source_entity()
                    != Some(EntityRef::Organization(case.source_authority()))
                || information.subject() != EntityRef::Character(case.defendant())
                || information.observed_at() != referral.referred_at()
                || information.recorded_at() != referral.referred_at()
                || information.reliability() != Reliability::DirectAccess
                || information.specificity() != Specificity::Precise
                || !information.derived_from().is_empty()
                || information.summary().trim().is_empty()
                || !seen_reports.insert(referral.report())
                || report.recipient() != case.prosecutor_office()
                || report.kind() != ReportKind::Legal
                || report.title() != expected_title
                || report.generated_at() != referral.referred_at()
                || report.entries().len() != 1
                || report.entries()[0].attention != AttentionClass::Notable
                || report.entries()[0].summary != information.summary()
                || !report.entries()[0].sources.is_empty()
                || report.entries()[0].decision.is_some()
                || report.entries()[0].entities != expected_entities
            {
                return Err(invalid_referral());
            }
        }
        if referred_evidence != *case.evidence() {
            return Err(invalid_case());
        }
    }
    for referral in state.legal.prosecution_referrals() {
        if !seen_referrals.contains(&referral.id()) {
            return Err(StateValidationError::InvalidProsecutionReferral {
                referral: referral.id(),
            });
        }
    }
    Ok(())
}

pub(super) fn validate_legal_subsystems(state: &AppState) -> Result<(), StateValidationError> {
    for jurisdiction in state.legal.jurisdictions() {
        let organization = state
            .world
            .get_organization(jurisdiction.organization())
            .ok_or(StateValidationError::InvalidLegalJurisdiction {
                organization: jurisdiction.organization(),
            })?;
        if !matches!(
            organization.kind(),
            OrganizationKind::LawEnforcement | OrganizationKind::LegalAuthority
        ) || jurisdiction.neighborhoods().is_empty()
            || jurisdiction.version() == 0
            || jurisdiction
                .neighborhoods()
                .iter()
                .any(|neighborhood| state.world.get_neighborhood(*neighborhood).is_none())
        {
            return Err(StateValidationError::InvalidLegalJurisdiction {
                organization: jurisdiction.organization(),
            });
        }
    }

    for response in state.legal.police_responses() {
        let authority = state.world.get_organization(response.authority()).ok_or(
            StateValidationError::InvalidPoliceResponse {
                response: response.id(),
            },
        )?;
        if authority.kind() != OrganizationKind::LawEnforcement
            || state
                .world
                .get_neighborhood(response.neighborhood())
                .is_none()
            || response.version() == 0
            || response.dispatched_at() >= response.arrival_due_at()
            || response.dispatched_at() > state.now()
        {
            return Err(StateValidationError::InvalidPoliceResponse {
                response: response.id(),
            });
        }
        let operation = state
            .operations
            .get_operation(response.source_operation())
            .ok_or(StateValidationError::InvalidPoliceResponse {
                response: response.id(),
            })?;
        let jurisdiction = state.legal.get_jurisdiction(response.authority()).ok_or(
            StateValidationError::InvalidPoliceResponse {
                response: response.id(),
            },
        )?;
        if operation.police_response() != Some(response.id())
            || operation.started_at() != Some(response.dispatched_at())
            || response.jurisdiction_version() == 0
            || response.jurisdiction_version() > jurisdiction.version()
        {
            return Err(StateValidationError::InvalidPoliceResponse {
                response: response.id(),
            });
        }
        if let Some(patrol) = response.patrol() {
            let deployment = state
                .legal
                .get_patrol_deployment(patrol.deployment())
                .ok_or(StateValidationError::InvalidPoliceResponse {
                    response: response.id(),
                })?;
            if patrol.version() == 0
                || patrol.version() > deployment.version()
                || deployment.organization() != response.authority()
                || deployment.neighborhood() != response.neighborhood()
            {
                return Err(StateValidationError::InvalidPoliceResponse {
                    response: response.id(),
                });
            }
        }
        match response.status() {
            PoliceResponseStatus::Dispatched => {
                if response.arrived_at().is_some() || response.version() != 1 {
                    return Err(StateValidationError::InvalidPoliceResponse {
                        response: response.id(),
                    });
                }
            }
            PoliceResponseStatus::Arrived => {
                if response.arrived_at().is_none_or(|arrived_at| {
                    arrived_at < response.arrival_due_at() || arrived_at > state.now()
                }) || response.version() < 2
                {
                    return Err(StateValidationError::InvalidPoliceResponse {
                        response: response.id(),
                    });
                }
            }
        }
    }

    for deployment in state.legal.patrol_deployments() {
        let authority = state
            .world
            .get_organization(deployment.organization())
            .ok_or(StateValidationError::InvalidPatrolDeployment {
                deployment: deployment.id(),
            })?;
        let _ = state
            .world
            .get_neighborhood(deployment.neighborhood())
            .ok_or(StateValidationError::InvalidPatrolDeployment {
                deployment: deployment.id(),
            })?;
        if authority.kind() != OrganizationKind::LawEnforcement
            || deployment.version() == 0
            || deployment.established_at() > deployment.last_changed_at()
            || deployment.last_changed_at() > state.now()
            || !is_canonical_patrol_schedule(deployment.windows())
        {
            return Err(StateValidationError::InvalidPatrolDeployment {
                deployment: deployment.id(),
            });
        }
        match deployment.status() {
            PatrolDeploymentStatus::Active => {
                let jurisdiction = state.legal.get_jurisdiction(deployment.organization());
                if jurisdiction.is_none_or(|record| {
                    !record.neighborhoods().contains(&deployment.neighborhood())
                }) || state
                    .legal
                    .active_patrol_for(deployment.organization(), deployment.neighborhood())
                    .is_none_or(|record| record.id() != deployment.id())
                {
                    return Err(StateValidationError::InvalidPatrolDeployment {
                        deployment: deployment.id(),
                    });
                }
            }
            PatrolDeploymentStatus::Suspended | PatrolDeploymentStatus::Retired => {}
        }
    }

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
                let active_operation = state.operations.operations().any(|operation| {
                    matches!(
                        operation.status(),
                        OperationStatus::Authorized
                            | OperationStatus::InProgress
                            | OperationStatus::AwaitingDecision
                    ) && (operation.leader() == arrest.character()
                        || operation
                            .roles()
                            .values()
                            .any(|participant| *participant == arrest.character()))
                });
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

    validate_legal_representations(state)?;
    validate_prosecution_cases(state)?;

    for investigation in state.legal.investigations() {
        let owner = state.world.get_organization(investigation.owner()).ok_or(
            StateValidationError::MissingEntity {
                context: "investigation owner",
                entity: EntityRef::Organization(investigation.owner()),
            },
        )?;
        if !matches!(
            owner.kind(),
            OrganizationKind::LawEnforcement | OrganizationKind::LegalAuthority
        ) {
            return Err(StateValidationError::MissingEntity {
                context: "investigation owner",
                entity: EntityRef::Organization(investigation.owner()),
            });
        }
        if investigation.opened_at() > state.now() {
            return Err(StateValidationError::FutureTimestamp {
                context: "investigation",
            });
        }
        if investigation.last_activity_at() > state.now()
            || investigation.last_activity_at() < investigation.opened_at()
        {
            return Err(StateValidationError::InvalidInvestigationActivity {
                investigation: investigation.id(),
            });
        }
        let origin = investigation.origin_operation();
        match origin {
            Some(operation) => {
                let operation_record = state.operations.get_operation(operation).ok_or(
                    StateValidationError::InvalidInvestigationActivity {
                        investigation: investigation.id(),
                    },
                )?;
                if investigation.notified_organizations().is_empty()
                    || !investigation
                        .notified_organizations()
                        .contains(&operation_record.responsible_organization())
                {
                    return Err(StateValidationError::InvalidInvestigationActivity {
                        investigation: investigation.id(),
                    });
                }
            }
            None if !investigation.notified_organizations().is_empty() => {
                return Err(StateValidationError::InvalidInvestigationActivity {
                    investigation: investigation.id(),
                });
            }
            None => {}
        }
        for notified in investigation.notified_organizations() {
            let organization = state.world.get_organization(*notified).ok_or(
                StateValidationError::InvalidInvestigationActivity {
                    investigation: investigation.id(),
                },
            )?;
            if !matches!(
                organization.kind(),
                OrganizationKind::Criminal
                    | OrganizationKind::Political
                    | OrganizationKind::Press
                    | OrganizationKind::Labor
                    | OrganizationKind::Civic
                    | OrganizationKind::Commercial
            ) {
                return Err(StateValidationError::InvalidInvestigationActivity {
                    investigation: investigation.id(),
                });
            }
        }
        // Exhaustiveness canary: a new InvestigationStatus must update the status checks above.
        match investigation.status() {
            InvestigationStatus::Active
            | InvestigationStatus::Suspended
            | InvestigationStatus::Closed => {}
        }
        if investigation.version() == 0
            || investigation
                .lead_investigator()
                .is_some_and(|lead| !investigation.assigned_investigators().contains(&lead))
        {
            return Err(StateValidationError::InvalidInvestigationStaffing {
                investigation: investigation.id(),
            });
        }
        for investigator in investigation.assigned_investigators() {
            let character = state.world.get_character(*investigator).ok_or(
                StateValidationError::InvalidInvestigationStaffing {
                    investigation: investigation.id(),
                },
            )?;
            if investigation.status() == InvestigationStatus::Active
                && (character.organization() != Some(investigation.owner())
                    || character
                        .capability(CapabilityKind::Investigation)
                        .is_none())
            {
                return Err(StateValidationError::InvalidInvestigationStaffing {
                    investigation: investigation.id(),
                });
            }
        }
        for subject in investigation.subjects() {
            if !is_entity_present(state, *subject) {
                return Err(StateValidationError::MissingEntity {
                    context: "investigation subject",
                    entity: *subject,
                });
            }
        }
    }

    let mut derived_evidence_from_work = BTreeSet::new();
    for work in state.legal.investigation_work() {
        let investigation = state
            .legal
            .get_investigation(work.investigation())
            .ok_or(StateValidationError::InvalidInvestigationWork { work: work.id() })?;
        let investigator = state
            .world
            .get_character(work.investigator())
            .ok_or(StateValidationError::InvalidInvestigationWork { work: work.id() })?;
        let focus_is_valid = match (work.kind(), work.focus()) {
            (
                InvestigationWorkKind::PatternAnalysis,
                InvestigationWorkFocus::EntityConnection { from, to },
            ) => {
                from < to
                    && is_entity_present(state, from)
                    && is_entity_present(state, to)
                    && source_evidence_forms_simple_path(state, work)
            }
            (InvestigationWorkKind::EvidenceReview, InvestigationWorkFocus::Evidence(source)) => {
                work.source_evidence() == &BTreeSet::from([source])
                    && state.legal.get_evidence(source).is_some_and(|evidence| {
                        evidence.investigation() == work.investigation()
                            && evidence.discovered_at() <= work.scheduled_at()
                            && is_reviewable_evidence_kind(evidence.kind())
                    })
            }
            (
                InvestigationWorkKind::WitnessInterview,
                InvestigationWorkFocus::Witness(case_witness),
            ) => {
                work.source_evidence().is_empty()
                    && state
                        .legal
                        .get_case_witness(case_witness)
                        .is_some_and(|witness| {
                            witness.investigation() == work.investigation()
                                && witness.registered_at() <= work.scheduled_at()
                        })
            }
            (InvestigationWorkKind::PatternAnalysis, InvestigationWorkFocus::Evidence(_))
            | (
                InvestigationWorkKind::EvidenceReview,
                InvestigationWorkFocus::EntityConnection { from: _, to: _ },
            )
            | (InvestigationWorkKind::PatternAnalysis, InvestigationWorkFocus::Witness(_))
            | (InvestigationWorkKind::EvidenceReview, InvestigationWorkFocus::Witness(_))
            | (
                InvestigationWorkKind::WitnessInterview,
                InvestigationWorkFocus::EntityConnection { .. },
            )
            | (InvestigationWorkKind::WitnessInterview, InvestigationWorkFocus::Evidence(_)) => {
                false
            }
        };
        if !focus_is_valid
            || work.scheduled_at() > state.now()
            || work.due_at() <= work.scheduled_at()
            || work.source_evidence().iter().any(|source| {
                state.legal.get_evidence(*source).is_none_or(|evidence| {
                    evidence.investigation() != work.investigation()
                        || evidence.discovered_at() > work.scheduled_at()
                })
            })
        {
            return Err(StateValidationError::InvalidInvestigationWork { work: work.id() });
        }
        match work.status() {
            InvestigationWorkStatus::Scheduled => {
                if work.version() != 1
                    || work.resolution().is_some()
                    || investigation.status() != InvestigationStatus::Active
                    || investigation
                        .investigator_role(work.investigator())
                        .is_none()
                    || investigator.organization() != Some(investigation.owner())
                    || investigator
                        .capability(CapabilityKind::Investigation)
                        .is_none()
                {
                    return Err(StateValidationError::InvalidInvestigationWork { work: work.id() });
                }
            }
            InvestigationWorkStatus::Completed => {
                let resolution = work
                    .resolution()
                    .ok_or(StateValidationError::InvalidInvestigationWork { work: work.id() })?;
                if work.version() != 2
                    || resolution.resolved_at() < work.due_at()
                    || resolution.resolved_at() > state.now()
                {
                    return Err(StateValidationError::InvalidInvestigationWork { work: work.id() });
                }
                match resolution.outcome() {
                    InvestigationWorkOutcome::Connected => {
                        let derived_id = resolution.derived_evidence().ok_or(
                            StateValidationError::InvalidInvestigationWork { work: work.id() },
                        )?;
                        if !derived_evidence_from_work.insert(derived_id) {
                            return Err(StateValidationError::InvalidInvestigationWork {
                                work: work.id(),
                            });
                        }
                        match work.kind() {
                            InvestigationWorkKind::PatternAnalysis => {
                                if resolution.superseded_by().is_some() {
                                    return Err(StateValidationError::InvalidInvestigationWork {
                                        work: work.id(),
                                    });
                                }
                                let derived = state.legal.get_evidence(derived_id).ok_or(
                                    StateValidationError::InvalidInvestigationWork {
                                        work: work.id(),
                                    },
                                )?;
                                if derived.investigation() != work.investigation()
                                    || derived.custodian() != investigation.owner()
                                    || derived.kind() != EvidenceKind::PatternLink
                                    || derived.origin() != Some(work.focus().from())
                                    || derived.subject() != work.focus().to()
                                    || derived.discovered_at() != resolution.resolved_at()
                                    || derived.derived_from() != work.source_evidence()
                                    || work
                                        .source_evidence()
                                        .iter()
                                        .any(|source| *source >= derived_id)
                                {
                                    return Err(StateValidationError::InvalidInvestigationWork {
                                        work: work.id(),
                                    });
                                }
                            }
                            InvestigationWorkKind::WitnessInterview => {
                                if resolution.superseded_by().is_some()
                                    || !work.source_evidence().is_empty()
                                {
                                    return Err(StateValidationError::InvalidInvestigationWork {
                                        work: work.id(),
                                    });
                                }
                                let Some(case_witness) = work.focus().witness_id() else {
                                    return Err(StateValidationError::InvalidInvestigationWork {
                                        work: work.id(),
                                    });
                                };
                                let derived = state.legal.get_evidence(derived_id).ok_or(
                                    StateValidationError::InvalidInvestigationWork {
                                        work: work.id(),
                                    },
                                )?;
                                // The interview's derived evidence is the recorded
                                // testimony; the named statement must exist on the case
                                // witness and point back at the same evidence.
                                let statement_ok = state
                                    .legal
                                    .get_case_witness(case_witness)
                                    .and_then(|witness| {
                                        witness
                                            .statements()
                                            .iter()
                                            .filter_map(|id| state.legal.get_witness_statement(*id))
                                            .find(|statement| statement.evidence() == derived_id)
                                    })
                                    .is_some_and(|statement| {
                                        statement.case_witness() == case_witness
                                            && statement.recorded_at() == resolution.resolved_at()
                                    });
                                if derived.investigation() != work.investigation()
                                    || derived.custodian() != investigation.owner()
                                    || derived.kind() != EvidenceKind::WitnessTestimony
                                    || derived.discovered_at() != resolution.resolved_at()
                                    || !statement_ok
                                {
                                    return Err(StateValidationError::InvalidInvestigationWork {
                                        work: work.id(),
                                    });
                                }
                            }
                            InvestigationWorkKind::EvidenceReview => {
                                return Err(StateValidationError::InvalidInvestigationWork {
                                    work: work.id(),
                                });
                            }
                        }
                    }
                    InvestigationWorkOutcome::Developed => {
                        if work.kind() != InvestigationWorkKind::EvidenceReview
                            || resolution.superseded_by().is_some()
                        {
                            return Err(StateValidationError::InvalidInvestigationWork {
                                work: work.id(),
                            });
                        }
                        let source_id = work.focus().evidence_id().ok_or(
                            StateValidationError::InvalidInvestigationWork { work: work.id() },
                        )?;
                        let source = state.legal.get_evidence(source_id).ok_or(
                            StateValidationError::InvalidInvestigationWork { work: work.id() },
                        )?;
                        let derived_id = resolution.derived_evidence().ok_or(
                            StateValidationError::InvalidInvestigationWork { work: work.id() },
                        )?;
                        if !derived_evidence_from_work.insert(derived_id) {
                            return Err(StateValidationError::InvalidInvestigationWork {
                                work: work.id(),
                            });
                        }
                        let derived = state.legal.get_evidence(derived_id).ok_or(
                            StateValidationError::InvalidInvestigationWork { work: work.id() },
                        )?;
                        if derived.investigation() != work.investigation()
                            || derived.custodian() != investigation.owner()
                            || derived.kind() != EvidenceKind::ForensicAnalysis
                            || derived.subject() != source.subject()
                            || derived.origin() != source.origin()
                            || derived.strength() != source.strength()
                            || derived.reliability()
                                != resolve_improved_evidence_reliability(source.reliability())
                            || derived.admissibility() != source.admissibility()
                            || derived.discovered_at() != resolution.resolved_at()
                            || derived.derived_from() != &BTreeSet::from([source_id])
                            || source_id >= derived_id
                        {
                            return Err(StateValidationError::InvalidInvestigationWork {
                                work: work.id(),
                            });
                        }
                    }
                    InvestigationWorkOutcome::Inconclusive => {
                        if resolution.superseded_by().is_some()
                            || resolution.derived_evidence().is_some()
                        {
                            return Err(StateValidationError::InvalidInvestigationWork {
                                work: work.id(),
                            });
                        }
                    }
                    InvestigationWorkOutcome::Superseded => {
                        if resolution.derived_evidence().is_some() {
                            return Err(StateValidationError::InvalidInvestigationWork {
                                work: work.id(),
                            });
                        }
                        let superseding_id = resolution.superseded_by().ok_or(
                            StateValidationError::InvalidInvestigationWork { work: work.id() },
                        )?;
                        let superseding = state.legal.get_evidence(superseding_id).ok_or(
                            StateValidationError::InvalidInvestigationWork { work: work.id() },
                        )?;
                        let valid_superseding = match (work.kind(), work.focus()) {
                            (
                                InvestigationWorkKind::PatternAnalysis,
                                InvestigationWorkFocus::EntityConnection { from, to },
                            ) => superseding.origin().is_some_and(|origin| {
                                (origin == from && superseding.subject() == to)
                                    || (origin == to && superseding.subject() == from)
                            }),
                            (
                                InvestigationWorkKind::EvidenceReview,
                                InvestigationWorkFocus::Evidence(source),
                            ) => {
                                superseding.kind() == EvidenceKind::ForensicAnalysis
                                    && superseding.derived_from() == &BTreeSet::from([source])
                            }
                            (
                                InvestigationWorkKind::PatternAnalysis,
                                InvestigationWorkFocus::Evidence(_),
                            )
                            | (
                                InvestigationWorkKind::EvidenceReview,
                                InvestigationWorkFocus::EntityConnection { from: _, to: _ },
                            )
                            | (
                                InvestigationWorkKind::PatternAnalysis,
                                InvestigationWorkFocus::Witness(_),
                            )
                            | (
                                InvestigationWorkKind::EvidenceReview,
                                InvestigationWorkFocus::Witness(_),
                            )
                            | (InvestigationWorkKind::WitnessInterview, _) => false,
                        };
                        if superseding.investigation() != work.investigation()
                            || superseding.discovered_at() > resolution.resolved_at()
                            || !valid_superseding
                        {
                            return Err(StateValidationError::InvalidInvestigationWork {
                                work: work.id(),
                            });
                        }
                    }
                }
            }
        }
    }

    for witness in state.legal.case_witnesses() {
        let investigation = state
            .legal
            .get_investigation(witness.investigation())
            .ok_or(StateValidationError::InvalidCaseWitness {
                witness: witness.id(),
            })?;
        if state.world.get_character(witness.witness()).is_none()
            || witness.registered_at() < investigation.opened_at()
            || witness.registered_at() > state.now()
            || witness.version() == 0
        {
            return Err(StateValidationError::InvalidCaseWitness {
                witness: witness.id(),
            });
        }
        // Exhaustiveness canary: a new WitnessCooperation variant must be classified in
        // `discount_band` before persisted statements remain validatable.
        match witness.cooperation() {
            WitnessCooperation::Hostile
            | WitnessCooperation::Reluctant
            | WitnessCooperation::Cooperative => {}
        }
    }

    let mut named_witness_evidence = BTreeSet::new();
    for statement in state.legal.witness_statements() {
        let case_witness = state
            .legal
            .get_case_witness(statement.case_witness())
            .ok_or(StateValidationError::InvalidWitnessStatement {
                statement: statement.id(),
            })?;
        let investigation = state
            .legal
            .get_investigation(case_witness.investigation())
            .ok_or(StateValidationError::InvalidWitnessStatement {
                statement: statement.id(),
            })?;
        if statement.summary().trim().is_empty()
            || statement.recorded_at() < case_witness.registered_at()
            || statement.recorded_at() > state.now()
            || !is_entity_present(state, statement.subject())
            || statement
                .origin()
                .is_some_and(|origin| !is_entity_present(state, origin))
            || !named_witness_evidence.insert(statement.evidence())
        {
            return Err(StateValidationError::InvalidWitnessStatement {
                statement: statement.id(),
            });
        }
        let evidence = state.legal.get_evidence(statement.evidence()).ok_or(
            StateValidationError::InvalidWitnessStatement {
                statement: statement.id(),
            },
        )?;
        if evidence.investigation() != case_witness.investigation()
            || evidence.custodian() != investigation.owner()
            || evidence.subject() != statement.subject()
            || evidence.origin() != statement.origin()
            || evidence.source() != Some(EntityRef::Character(case_witness.witness()))
            || evidence.kind() != EvidenceKind::WitnessTestimony
            || evidence.strength()
                != resolve_witness_strength(statement.confidence(), statement.cooperation())
            || evidence.reliability()
                != resolve_witness_reliability(statement.confidence(), statement.cooperation())
            || evidence.admissibility() != Admissibility::Unknown
            || evidence.discovered_at() != statement.recorded_at()
            || !evidence.derived_from().is_empty()
        {
            return Err(StateValidationError::InvalidWitnessStatement {
                statement: statement.id(),
            });
        }
    }

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
        match informant.status() {
            InformantStatus::Active => {
                if informant.terminated_at().is_some()
                    || character.organization() == Some(informant.handler())
                {
                    return Err(StateValidationError::InvalidInformant {
                        informant: informant.id(),
                    });
                }
            }
            InformantStatus::Terminated => {
                let terminated_at =
                    informant
                        .terminated_at()
                        .ok_or(StateValidationError::InvalidInformant {
                            informant: informant.id(),
                        })?;
                if terminated_at < informant.established_at() || terminated_at > state.now() {
                    return Err(StateValidationError::InvalidInformant {
                        informant: informant.id(),
                    });
                }
            }
        }
    }

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
        let after_termination = informant
            .terminated_at()
            .is_some_and(|terminated_at| disclosure.disclosed_at() > terminated_at);
        if investigation.owner() != informant.handler()
            || information.holder() != KnowledgeHolder::Character(informant.character())
            || information.recorded_at() > disclosure.disclosed_at()
            || disclosure.disclosed_at() < informant.established_at()
            || disclosure.disclosed_at() < investigation.opened_at()
            || disclosure.disclosed_at() > state.now()
            || after_termination
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

    for evidence in state.legal.all_evidence() {
        let investigation = state
            .legal
            .get_investigation(evidence.investigation())
            .ok_or(StateValidationError::MissingEntity {
                context: "evidence investigation",
                entity: EntityRef::Investigation(evidence.investigation()),
            })?;
        if state.world.get_organization(evidence.custodian()).is_none()
            || evidence.custodian() != investigation.owner()
        {
            return Err(StateValidationError::MissingEntity {
                context: "evidence custodian",
                entity: EntityRef::Organization(evidence.custodian()),
            });
        }
        if !is_entity_present(state, evidence.subject()) {
            return Err(StateValidationError::MissingEntity {
                context: "evidence subject",
                entity: evidence.subject(),
            });
        }
        if let Some(origin) = evidence.origin() {
            if !is_entity_present(state, origin) {
                return Err(StateValidationError::MissingEntity {
                    context: "evidence origin",
                    entity: origin,
                });
            }
        }
        if let Some(source) = evidence.source() {
            if !is_entity_present(state, source) {
                return Err(StateValidationError::MissingEntity {
                    context: "evidence source",
                    entity: source,
                });
            }
            let valid_source = matches!(source, EntityRef::Character(_))
                && match evidence.kind() {
                    EvidenceKind::WitnessTestimony => {
                        named_witness_evidence.contains(&evidence.id())
                            && !informant_evidence.contains(&evidence.id())
                    }
                    EvidenceKind::InformantStatement => {
                        informant_evidence.contains(&evidence.id())
                            && !named_witness_evidence.contains(&evidence.id())
                    }
                    EvidenceKind::VehicleDescription
                    | EvidenceKind::Fingerprint
                    | EvidenceKind::RecoveredProperty
                    | EvidenceKind::FinancialRecord
                    | EvidenceKind::Surveillance
                    | EvidenceKind::CommunicationRecord
                    | EvidenceKind::KnownAssociation
                    | EvidenceKind::Document
                    | EvidenceKind::Ballistics
                    | EvidenceKind::PatternLink
                    | EvidenceKind::ForensicAnalysis => false,
                };
            if !valid_source {
                return Err(StateValidationError::InvalidEvidenceProvenance {
                    evidence: evidence.id(),
                });
            }
        } else if named_witness_evidence.contains(&evidence.id())
            || informant_evidence.contains(&evidence.id())
        {
            return Err(StateValidationError::InvalidEvidenceProvenance {
                evidence: evidence.id(),
            });
        }
        if evidence.discovered_at() > state.now() {
            return Err(StateValidationError::FutureTimestamp {
                context: "evidence",
            });
        }
        match evidence.kind() {
            EvidenceKind::PatternLink => {
                if evidence.source().is_some()
                    || evidence.derived_from().len() < 2
                    || !derived_evidence_from_work.contains(&evidence.id())
                {
                    return Err(StateValidationError::InvalidEvidenceProvenance {
                        evidence: evidence.id(),
                    });
                }
            }
            EvidenceKind::ForensicAnalysis => {
                if evidence.source().is_some()
                    || evidence.derived_from().len() != 1
                    || !derived_evidence_from_work.contains(&evidence.id())
                {
                    return Err(StateValidationError::InvalidEvidenceProvenance {
                        evidence: evidence.id(),
                    });
                }
            }
            EvidenceKind::InformantStatement => {
                if !informant_evidence.contains(&evidence.id())
                    || evidence.source().is_none()
                    || !evidence.derived_from().is_empty()
                {
                    return Err(StateValidationError::InvalidEvidenceProvenance {
                        evidence: evidence.id(),
                    });
                }
            }
            EvidenceKind::WitnessTestimony
            | EvidenceKind::VehicleDescription
            | EvidenceKind::Fingerprint
            | EvidenceKind::RecoveredProperty
            | EvidenceKind::FinancialRecord
            | EvidenceKind::Surveillance
            | EvidenceKind::CommunicationRecord
            | EvidenceKind::KnownAssociation
            | EvidenceKind::Document
            | EvidenceKind::Ballistics => {
                if !evidence.derived_from().is_empty() {
                    return Err(StateValidationError::InvalidEvidenceProvenance {
                        evidence: evidence.id(),
                    });
                }
            }
        }
        for source_id in evidence.derived_from() {
            let source = state.legal.get_evidence(*source_id).ok_or(
                StateValidationError::InvalidEvidenceProvenance {
                    evidence: evidence.id(),
                },
            )?;
            if *source_id >= evidence.id()
                || source.investigation() != evidence.investigation()
                || source.discovered_at() > evidence.discovered_at()
            {
                return Err(StateValidationError::InvalidEvidenceProvenance {
                    evidence: evidence.id(),
                });
            }
        }
    }

    for report in state.reports.reports() {
        if state.world.get_organization(report.recipient()).is_none() {
            return Err(StateValidationError::MissingEntity {
                context: "report recipient",
                entity: EntityRef::Organization(report.recipient()),
            });
        }
        if report.generated_at() > state.now() {
            return Err(StateValidationError::FutureTimestamp { context: "report" });
        }
        for entry in report.entries() {
            for information in &entry.sources {
                let information_record = state.intelligence.get_information(*information).ok_or(
                    StateValidationError::MissingReportInformation {
                        report: report.id(),
                        information: *information,
                    },
                )?;
                let is_available = match information_record.holder() {
                    KnowledgeHolder::Organization(organization) => {
                        organization == report.recipient()
                    }
                    KnowledgeHolder::Character(_) => false,
                };
                if !is_available {
                    return Err(StateValidationError::ReportInformationUnavailable {
                        report: report.id(),
                        information: *information,
                    });
                }
            }
            for entity in &entry.entities {
                if !is_entity_present(state, *entity) {
                    return Err(StateValidationError::MissingEntity {
                        context: "report entry",
                        entity: *entity,
                    });
                }
            }
            if let Some(decision) = entry.decision {
                let decision_record = state.decisions.get_decision(decision).ok_or(
                    StateValidationError::MissingReportDecision {
                        report: report.id(),
                        decision,
                    },
                )?;
                if decision_record.recipient() != report.recipient() {
                    return Err(StateValidationError::ReportDecisionRecipientMismatch {
                        report: report.id(),
                        decision,
                    });
                }
            }
        }
    }

    for event in state.history.events() {
        if event.occurred_at() > state.now() {
            return Err(StateValidationError::FutureTimestamp {
                context: "history event",
            });
        }
        for entity in event.entities() {
            if !is_entity_present(state, *entity) {
                return Err(StateValidationError::MissingEntity {
                    context: "history event",
                    entity: *entity,
                });
            }
        }
    }
    Ok(())
}
