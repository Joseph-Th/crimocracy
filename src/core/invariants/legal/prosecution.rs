//! Prosecution-case validation: referral artifacts, resolutions, and office exclusivity.

//! Release-safe structural validation for the legal subsystems plus persisted reports and history.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::intelligence::{
    InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability, Specificity,
};
use crate::legal::prosecution_system::write_resolution_summary;
use crate::legal::{ProsecutionCaseResolution, ProsecutionCaseStatus};
use crate::reports::ReportKind;
use crate::world::{CapabilityKind, OrganizationKind};
use std::collections::BTreeSet;

pub(super) fn validate_prosecution_cases(state: &AppState) -> Result<(), StateValidationError> {
    let mut seen_referrals = BTreeSet::new();
    let mut seen_information = BTreeSet::new();
    let mut seen_reports = BTreeSet::new();
    // Reused render target for persisted resolution summaries in this release-safe pass.
    let mut text_scratch = String::new();
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

        // The resolution report's entity set is compared element-wise (both statuses below)
        // so per-record validation never rebuilds this set per case.
        let expected_entities_contains = |entities: &BTreeSet<EntityRef>| {
            entities.len() == 5
                && entities.contains(&EntityRef::Character(case.defendant()))
                && entities.contains(&EntityRef::Organization(case.source_authority()))
                && entities.contains(&EntityRef::Organization(case.prosecutor_office()))
                && entities.contains(&EntityRef::Character(case.lead_prosecutor()))
                && entities.contains(&EntityRef::Investigation(case.source_investigation()))
        };

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
                // The enclosing match arm guarantees a resolved status; map it to the same
                // resolution variant the commit path rendered from.
                let (expected_title, resolution) = match case.status() {
                    ProsecutionCaseStatus::Declined => {
                        ("Prosecution declined", ProsecutionCaseResolution::Declined)
                    }
                    ProsecutionCaseStatus::Closed => (
                        "Prosecution review closed",
                        ProsecutionCaseResolution::Closed,
                    ),
                    ProsecutionCaseStatus::Reviewing => {
                        unreachable!("resolved prosecution cases are never under review")
                    }
                };
                text_scratch.clear();
                write_resolution_summary(
                    &mut text_scratch,
                    resolution,
                    office.name(),
                    defendant.name(),
                    lead.name(),
                )
                .expect("String buffer writes are infallible");
                let expected_summary = text_scratch.as_str();
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
                    || !expected_entities_contains(&report.entries()[0].entities)
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
                || !expected_entities_contains(&report.entries()[0].entities)
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
