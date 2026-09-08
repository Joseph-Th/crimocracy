//! Legal-representation validation: retained payment/artifact contracts and lifecycle closure.

use crate::contacts::{ContactKind, ContactStatus, InstitutionalContactRecord};
use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::delegation::{ResponsibilityFunction, ResponsibilityScope};
use crate::finance::{
    AccountKind, FinancialAccountRecord, FinancialOwner, LedgerTransactionRecord, Money,
};
use crate::intelligence::{
    InformationRecord, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
    Specificity,
};
use crate::legal::legal_representation_system::{
    ended_representation_summary, retained_representation_summary,
};
use crate::legal::{ArrestRecord, LegalRepresentationRecord, LegalRepresentationStatus};
use crate::reports::{ReportKind, ReportRecord};
use crate::world::{CapabilityKind, CharacterRecord, OrganizationKind, OrganizationRecord};
use std::collections::BTreeSet;

#[derive(Default)]
struct SeenRepresentationArtifacts {
    payments: BTreeSet<crate::core::id::LedgerTransactionId>,
    information: BTreeSet<crate::core::id::InformationId>,
    reports: BTreeSet<crate::core::id::ReportId>,
}

struct RepresentationRefs<'a> {
    arrest: &'a ArrestRecord,
    defendant: &'a CharacterRecord,
    sponsor: &'a OrganizationRecord,
    counsel: &'a CharacterRecord,
    firm: &'a OrganizationRecord,
    contact: &'a InstitutionalContactRecord,
    provider: &'a FinancialAccountRecord,
    payment: &'a LedgerTransactionRecord,
    retained_information: &'a InformationRecord,
    retained_report: &'a ReportRecord,
}

pub(super) fn validate_legal_representations(state: &AppState) -> Result<(), StateValidationError> {
    let mut seen = SeenRepresentationArtifacts::default();
    for representation in state.legal.legal_representations() {
        validate_representation(state, representation, &mut seen)?;
    }
    Ok(())
}

fn validate_representation(
    state: &AppState,
    representation: &LegalRepresentationRecord,
    seen: &mut SeenRepresentationArtifacts,
) -> Result<(), StateValidationError> {
    let refs = resolve_references(state, representation)?;
    validate_retained_contract(state, representation, &refs, seen)?;
    match representation.status() {
        LegalRepresentationStatus::Active => {
            validate_active_lifecycle(state, representation, &refs)
        }
        LegalRepresentationStatus::Ended => {
            validate_ended_lifecycle(state, representation, &refs, seen)
        }
    }
}

fn resolve_references<'a>(
    state: &'a AppState,
    representation: &LegalRepresentationRecord,
) -> Result<RepresentationRefs<'a>, StateValidationError> {
    let invalid = || invalid_representation(representation);
    Ok(RepresentationRefs {
        arrest: state
            .legal
            .get_arrest(representation.arrest())
            .ok_or_else(invalid)?,
        defendant: state
            .world
            .get_character(representation.defendant())
            .ok_or_else(invalid)?,
        sponsor: state
            .world
            .get_organization(representation.sponsor())
            .ok_or_else(invalid)?,
        counsel: state
            .world
            .get_character(representation.counsel())
            .ok_or_else(invalid)?,
        firm: state
            .world
            .get_organization(representation.counsel_institution())
            .ok_or_else(invalid)?,
        contact: state
            .contacts
            .get_contact(representation.contact())
            .ok_or_else(invalid)?,
        provider: state
            .finance
            .get_account(representation.provider_account())
            .ok_or_else(invalid)?,
        payment: state
            .finance
            .get_transaction(representation.payment())
            .ok_or_else(invalid)?,
        retained_information: state
            .intelligence
            .get_information(representation.information())
            .ok_or_else(invalid)?,
        retained_report: state
            .reports
            .get_report(representation.report())
            .ok_or_else(invalid)?,
    })
}

fn validate_retained_contract(
    state: &AppState,
    representation: &LegalRepresentationRecord,
    refs: &RepresentationRefs<'_>,
    seen: &mut SeenRepresentationArtifacts,
) -> Result<(), StateValidationError> {
    if !base_relationships_are_valid(state, representation, refs)
        || !retainer_payment_is_valid(state, representation, refs)
        || !retained_artifacts_are_valid(representation, refs)
        || !seen.payments.insert(representation.payment())
        || !seen.information.insert(representation.information())
        || !seen.reports.insert(representation.report())
    {
        return Err(invalid_representation(representation));
    }
    Ok(())
}

fn base_relationships_are_valid(
    state: &AppState,
    representation: &LegalRepresentationRecord,
    refs: &RepresentationRefs<'_>,
) -> bool {
    refs.arrest.character() == representation.defendant()
        && refs.sponsor.kind() == OrganizationKind::Criminal
        && refs.firm.kind() == OrganizationKind::LegalServices
        && refs.contact.sponsor() == representation.sponsor()
        && refs.contact.contact() == representation.counsel()
        && refs.contact.institution() == representation.counsel_institution()
        && refs.contact.kind() == ContactKind::Legal
        && refs
            .counsel
            .capability(CapabilityKind::LegalKnowledge)
            .is_some()
        && representation.fee() > Money::ZERO
        && representation.retained_at() <= state.now()
        && representation.retained_at() >= refs.arrest.arrested_at()
        && representation.retained_at() >= refs.contact.established_at()
        && refs
            .contact
            .terminated_at()
            .is_none_or(|terminated_at| terminated_at >= representation.retained_at())
        && representation.version() > 0
        && refs.provider.owner()
            == FinancialOwner::Organization(representation.counsel_institution())
        && refs.provider.kind() == AccountKind::LegitimateOperating
}

fn retainer_payment_is_valid(
    state: &AppState,
    representation: &LegalRepresentationRecord,
    refs: &RepresentationRefs<'_>,
) -> bool {
    if refs.payment.occurred_at() != representation.retained_at()
        || refs.payment.postings().len() < 2
    {
        return false;
    }
    let payer_postings: Vec<_> = refs
        .payment
        .postings()
        .iter()
        .filter(|posting| posting.account != representation.provider_account())
        .collect();
    let payer_accounts_are_valid = !payer_postings.is_empty()
        && payer_postings.iter().all(|posting| {
            posting.amount < Money::ZERO
                && state
                    .finance
                    .get_account(posting.account)
                    .is_some_and(|payer| {
                        payer.owner() == FinancialOwner::Organization(representation.sponsor())
                            && payer.kind().is_liquid()
                    })
        });
    let payer_outflow_cents = payer_postings.iter().try_fold(0_i128, |total, posting| {
        total.checked_add(i128::from(posting.amount.cents()).checked_neg()?)
    });
    let has_provider_posting = refs.payment.postings().iter().any(|posting| {
        posting.account == representation.provider_account()
            && posting.amount == representation.fee()
    });
    payer_accounts_are_valid
        && payer_outflow_cents == Some(i128::from(representation.fee().cents()))
        && has_provider_posting
        && budget_authority_is_valid(representation, refs.payment, &payer_postings)
}

fn budget_authority_is_valid(
    representation: &LegalRepresentationRecord,
    payment: &LedgerTransactionRecord,
    payer_postings: &[&crate::finance::LedgerPosting],
) -> bool {
    match (representation.authorization(), payment.budget_usage()) {
        (None, None) => true,
        (Some(authority), Some(usage)) => {
            authority.scope == ResponsibilityScope::Function(ResponsibilityFunction::Legal)
                && usage.mandate() == authority.mandate
                && usage.manager() == authority.manager
                && usage.scope() == authority.scope
                && usage.amount() == representation.fee()
                && payer_postings.len() == 1
                && payer_postings[0].account == usage.funding_account()
                && payer_postings[0].amount
                    == representation
                        .fee()
                        .checked_neg()
                        .expect("positive legal fee must negate")
        }
        (None, Some(_)) | (Some(_), None) => false,
    }
}

fn retained_artifacts_are_valid(
    representation: &LegalRepresentationRecord,
    refs: &RepresentationRefs<'_>,
) -> bool {
    let information = refs.retained_information;
    let expected_summary = retained_representation_summary(
        refs.sponsor.name(),
        refs.counsel.name(),
        refs.firm.name(),
        refs.defendant.name(),
        representation.fee(),
    );
    let report = refs.retained_report;
    let Some(entry) = report.entries().first() else {
        return false;
    };
    information.holder() == KnowledgeHolder::Organization(representation.sponsor())
        && information.source_kind() == InformationSourceKind::AfterAction
        && information.topic() == InformationTopic::LegalActivity
        && information.source_entity() == Some(EntityRef::Character(representation.counsel()))
        && information.subject() == EntityRef::Character(representation.defendant())
        && information.observed_at() == representation.retained_at()
        && information.recorded_at() == representation.retained_at()
        && information.reliability() == Reliability::DirectAccess
        && information.specificity() == Specificity::Precise
        && information.derived_from().is_empty()
        && information.summary() == expected_summary
        && report.recipient() == representation.sponsor()
        && report.kind() == ReportKind::Legal
        && report.title() == "Legal representation retained"
        && report.generated_at() == representation.retained_at()
        && report.entries().len() == 1
        && entry.attention == AttentionClass::Notable
        && entry.summary == information.summary()
        && entry.sources.is_empty()
        && entry.decision.is_none()
        && retained_entities_are_valid(representation, refs.arrest, &entry.entities)
}

fn retained_entities_are_valid(
    representation: &LegalRepresentationRecord,
    arrest: &ArrestRecord,
    entities: &BTreeSet<EntityRef>,
) -> bool {
    entities.len() == 4
        && entities.contains(&EntityRef::Character(representation.defendant()))
        && entities.contains(&EntityRef::Character(representation.counsel()))
        && entities.contains(&EntityRef::Organization(
            representation.counsel_institution(),
        ))
        && entities.contains(&EntityRef::Investigation(arrest.investigation()))
}

fn validate_active_lifecycle(
    state: &AppState,
    representation: &LegalRepresentationRecord,
    refs: &RepresentationRefs<'_>,
) -> Result<(), StateValidationError> {
    if representation.version() != 1
        || representation.ended_at().is_some()
        || representation.end_reason().is_some()
        || representation.ended_information().is_some()
        || representation.ended_report().is_some()
        || refs.contact.status() != ContactStatus::Active
        || refs.counsel.organization() != Some(representation.counsel_institution())
        || state
            .legal
            .active_representation_for_arrest(representation.arrest())
            .is_none_or(|active| active.id() != representation.id())
    {
        return Err(invalid_representation(representation));
    }
    Ok(())
}

fn validate_ended_lifecycle(
    state: &AppState,
    representation: &LegalRepresentationRecord,
    refs: &RepresentationRefs<'_>,
    seen: &mut SeenRepresentationArtifacts,
) -> Result<(), StateValidationError> {
    let invalid = || invalid_representation(representation);
    let ended_at = representation.ended_at().ok_or_else(invalid)?;
    let reason = representation.end_reason().ok_or_else(invalid)?;
    let information_id = representation.ended_information().ok_or_else(invalid)?;
    let report_id = representation.ended_report().ok_or_else(invalid)?;
    let information = state
        .intelligence
        .get_information(information_id)
        .ok_or_else(invalid)?;
    let report = state.reports.get_report(report_id).ok_or_else(invalid)?;
    let Some(entry) = report.entries().first() else {
        return Err(invalid());
    };
    let expected_summary =
        ended_representation_summary(refs.counsel.name(), refs.defendant.name(), reason);
    if representation.version() != 2
        || ended_at < representation.retained_at()
        || ended_at > state.now()
        || state
            .legal
            .active_representation_for_arrest(representation.arrest())
            .is_some_and(|active| active.id() == representation.id())
        || !ended_artifacts_are_valid(
            representation,
            ended_at,
            expected_summary,
            information,
            report,
            entry,
        )
        || !seen.information.insert(information_id)
        || !seen.reports.insert(report_id)
    {
        return Err(invalid());
    }
    Ok(())
}

fn ended_artifacts_are_valid(
    representation: &LegalRepresentationRecord,
    ended_at: crate::core::time::SimTime,
    expected_summary: String,
    information: &InformationRecord,
    report: &ReportRecord,
    entry: &crate::reports::ReportEntry,
) -> bool {
    information.holder() == KnowledgeHolder::Organization(representation.sponsor())
        && information.source_kind() == InformationSourceKind::AfterAction
        && information.topic() == InformationTopic::LegalActivity
        && information.source_entity() == Some(EntityRef::Character(representation.counsel()))
        && information.subject() == EntityRef::Character(representation.defendant())
        && information.observed_at() == ended_at
        && information.recorded_at() == ended_at
        && information.reliability() == Reliability::DirectAccess
        && information.specificity() == Specificity::Precise
        && information.derived_from().is_empty()
        && information.summary() == expected_summary
        && report.recipient() == representation.sponsor()
        && report.kind() == ReportKind::Legal
        && report.title() == "Legal representation ended"
        && report.generated_at() == ended_at
        && report.entries().len() == 1
        && entry.attention == AttentionClass::Notable
        && entry.summary == information.summary()
        && entry.sources.is_empty()
        && entry.decision.is_none()
        && ended_entities_are_valid(representation, &entry.entities)
}

fn ended_entities_are_valid(
    representation: &LegalRepresentationRecord,
    entities: &BTreeSet<EntityRef>,
) -> bool {
    entities.len() == 3
        && entities.contains(&EntityRef::Character(representation.defendant()))
        && entities.contains(&EntityRef::Character(representation.counsel()))
        && entities.contains(&EntityRef::Organization(
            representation.counsel_institution(),
        ))
}

fn invalid_representation(representation: &LegalRepresentationRecord) -> StateValidationError {
    StateValidationError::InvalidLegalRepresentation {
        representation: representation.id(),
    }
}
