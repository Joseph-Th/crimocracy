//! Prosecution-case validation: referral artifacts, resolutions, and office exclusivity.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::intelligence::{
    InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability, Specificity,
};
use crate::legal::prosecution_system::{
    evidence_concerns_defendant, prosecution_referral_summary, write_resolution_summary,
};
use crate::legal::{
    ArrestRecord, ArrestStatus, InvestigationRecord, ProsecutionCaseRecord,
    ProsecutionCaseResolution, ProsecutionCaseStatus, ProsecutionReferralRecord,
};
use crate::reports::ReportKind;
use crate::world::{CapabilityKind, CharacterRecord, OrganizationKind, OrganizationRecord};
use std::collections::BTreeSet;

#[derive(Default)]
struct SeenProsecutionArtifacts {
    referrals: BTreeSet<crate::core::id::ProsecutionReferralId>,
    information: BTreeSet<crate::core::id::InformationId>,
    reports: BTreeSet<crate::core::id::ReportId>,
    text: String,
}

struct ProsecutionCaseRefs<'a> {
    arrest: &'a ArrestRecord,
    investigation: &'a InvestigationRecord,
    source_authority: &'a OrganizationRecord,
    office: &'a OrganizationRecord,
    defendant: &'a CharacterRecord,
}

pub(super) fn validate_prosecution_cases(state: &AppState) -> Result<(), StateValidationError> {
    let mut seen = SeenProsecutionArtifacts::default();
    for case in state.legal.prosecution_cases() {
        validate_case(state, case, &mut seen)?;
    }
    for referral in state.legal.prosecution_referrals() {
        if !seen.referrals.contains(&referral.id()) {
            return Err(StateValidationError::InvalidProsecutionReferral {
                referral: referral.id(),
            });
        }
    }
    Ok(())
}

fn validate_case(
    state: &AppState,
    case: &ProsecutionCaseRecord,
    seen: &mut SeenProsecutionArtifacts,
) -> Result<(), StateValidationError> {
    let refs = resolve_case_references(state, case)?;
    validate_case_base(state, case, &refs)?;
    validate_case_lifecycle(state, case, &refs, seen)?;
    validate_case_referrals(state, case, &refs, seen)
}

fn resolve_case_references<'a>(
    state: &'a AppState,
    case: &ProsecutionCaseRecord,
) -> Result<ProsecutionCaseRefs<'a>, StateValidationError> {
    let invalid = || invalid_case(case);
    Ok(ProsecutionCaseRefs {
        arrest: state.legal.get_arrest(case.arrest()).ok_or_else(invalid)?,
        investigation: state
            .legal
            .get_investigation(case.source_investigation())
            .ok_or_else(invalid)?,
        source_authority: state
            .world
            .get_organization(case.source_authority())
            .ok_or_else(invalid)?,
        office: state
            .world
            .get_organization(case.prosecutor_office())
            .ok_or_else(invalid)?,
        defendant: state
            .world
            .get_character(case.defendant())
            .ok_or_else(invalid)?,
    })
}

fn validate_case_base(
    state: &AppState,
    case: &ProsecutionCaseRecord,
    refs: &ProsecutionCaseRefs<'_>,
) -> Result<(), StateValidationError> {
    if case.opened_at() > state.now()
        || case.opened_at() < refs.arrest.arrested_at()
        || case.version() == 0
        || case.referrals().is_empty()
        || !case.referrals().contains(&case.initial_referral())
        || refs.arrest.character() != case.defendant()
        || refs.arrest.investigation() != case.source_investigation()
        || refs.arrest.authority() != case.source_authority()
        || refs.investigation.owner() != case.source_authority()
        || refs.source_authority.kind() != OrganizationKind::LawEnforcement
        || refs.office.kind() != OrganizationKind::Prosecutor
        || case.evidence().is_empty()
        || !refs.arrest.evidence().is_subset(case.evidence())
    {
        return Err(invalid_case(case));
    }
    Ok(())
}

fn validate_case_lifecycle(
    state: &AppState,
    case: &ProsecutionCaseRecord,
    refs: &ProsecutionCaseRefs<'_>,
    seen: &mut SeenProsecutionArtifacts,
) -> Result<(), StateValidationError> {
    match case.status() {
        ProsecutionCaseStatus::Reviewing => validate_reviewing_case(state, case),
        ProsecutionCaseStatus::Declined | ProsecutionCaseStatus::Closed => {
            validate_resolved_case(state, case, refs, seen)
        }
    }
}

fn validate_reviewing_case(
    state: &AppState,
    case: &ProsecutionCaseRecord,
) -> Result<(), StateValidationError> {
    let assigned = case.assigned_prosecutor().and_then(|prosecutor| {
        state
            .world
            .get_character(prosecutor)
            .map(|record| (prosecutor, record))
    });
    if case.resolved_at().is_some()
        || case.resolution_information().is_some()
        || case.resolution_report().is_some()
        || case.resolution_prosecutor().is_some()
        || assigned.is_some_and(|(prosecutor, lead)| {
            prosecutor == case.defendant()
                || state
                    .legal
                    .case_witness_for(case.source_investigation(), prosecutor)
                    .is_some()
                || refs_source_investigation_contains_prosecutor(state, case, prosecutor)
                || lead.organization() != Some(case.prosecutor_office())
                || lead.capability(CapabilityKind::LegalKnowledge).is_none()
                || state
                    .legal
                    .active_arrest_for_character(prosecutor)
                    .is_some()
        })
        || (case.assigned_prosecutor().is_some() && assigned.is_none())
        || state
            .legal
            .open_prosecution_case_for(case.arrest(), case.prosecutor_office())
            .is_none_or(|open| open.id() != case.id())
    {
        return Err(invalid_case(case));
    }
    Ok(())
}

fn refs_source_investigation_contains_prosecutor(
    state: &AppState,
    case: &ProsecutionCaseRecord,
    prosecutor: crate::core::id::CharacterId,
) -> bool {
    state
        .legal
        .get_investigation(case.source_investigation())
        .is_some_and(|investigation| {
            investigation
                .subjects()
                .contains(&EntityRef::Character(prosecutor))
        })
}

fn validate_resolved_case(
    state: &AppState,
    case: &ProsecutionCaseRecord,
    refs: &ProsecutionCaseRefs<'_>,
    seen: &mut SeenProsecutionArtifacts,
) -> Result<(), StateValidationError> {
    let invalid = || invalid_case(case);
    if case.assigned_prosecutor().is_some() {
        return Err(invalid());
    }
    let resolution_prosecutor = case.resolution_prosecutor().ok_or_else(invalid)?;
    let resolved_at = case.resolved_at().ok_or_else(invalid)?;
    let lead = state
        .world
        .get_character(resolution_prosecutor)
        .ok_or_else(invalid)?;
    if resolution_prosecutor == case.defendant()
        || state
            .legal
            .case_witness_for(case.source_investigation(), resolution_prosecutor)
            .is_some_and(|witness| witness.registered_at() < resolved_at)
        || lead.capability(CapabilityKind::LegalKnowledge).is_none()
    {
        return Err(invalid());
    }
    let information_id = case.resolution_information().ok_or_else(invalid)?;
    let report_id = case.resolution_report().ok_or_else(invalid)?;
    let information = state
        .intelligence
        .get_information(information_id)
        .ok_or_else(invalid)?;
    let report = state.reports.get_report(report_id).ok_or_else(invalid)?;
    let (expected_title, resolution) = resolved_case_rendering(case.status());
    seen.text.clear();
    write_resolution_summary(
        &mut seen.text,
        resolution,
        refs.office.name(),
        refs.defendant.name(),
        lead.name(),
    )
    .expect("String buffer writes are infallible");
    let Some(entry) = report.entries().first() else {
        return Err(invalid());
    };
    if resolved_at < case.opened_at()
        || resolved_at > state.now()
        || state
            .legal
            .open_prosecution_case_for(case.arrest(), case.prosecutor_office())
            .is_some_and(|open| open.id() == case.id())
        || !seen.information.insert(information_id)
        || information.holder() != KnowledgeHolder::Organization(case.prosecutor_office())
        || information.source_kind() != InformationSourceKind::AfterAction
        || information.topic() != InformationTopic::LegalActivity
        || information.source_entity() != Some(EntityRef::Character(resolution_prosecutor))
        || information.subject() != EntityRef::Character(case.defendant())
        || information.observed_at() != resolved_at
        || information.recorded_at() != resolved_at
        || information.reliability() != Reliability::DirectAccess
        || information.specificity() != Specificity::Precise
        || !information.derived_from().is_empty()
        || information.summary() != seen.text
        || !seen.reports.insert(report_id)
        || report.recipient() != case.prosecutor_office()
        || report.kind() != ReportKind::Legal
        || report.title() != expected_title
        || report.generated_at() != resolved_at
        || report.entries().len() != 1
        || entry.attention != AttentionClass::Notable
        || entry.summary != information.summary()
        || !entry.sources.is_empty()
        || entry.decision.is_some()
        || !case_entities_are_valid(case, &entry.entities, resolution_prosecutor)
        || (refs.arrest.status() == ArrestStatus::Detained
            && !state
                .legal
                .has_open_prosecution_case_for_arrest(case.arrest()))
    {
        return Err(invalid());
    }
    Ok(())
}

fn resolved_case_rendering(
    status: ProsecutionCaseStatus,
) -> (&'static str, ProsecutionCaseResolution) {
    match status {
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
    }
}

fn validate_case_referrals(
    state: &AppState,
    case: &ProsecutionCaseRecord,
    refs: &ProsecutionCaseRefs<'_>,
    seen: &mut SeenProsecutionArtifacts,
) -> Result<(), StateValidationError> {
    let mut referred_evidence = BTreeSet::new();
    for referral_id in case.referrals() {
        let referral = state.legal.get_prosecution_referral(*referral_id).ok_or(
            StateValidationError::InvalidProsecutionReferral {
                referral: *referral_id,
            },
        )?;
        validate_referral(state, case, refs, referral, seen, &mut referred_evidence)?;
    }
    if referred_evidence != *case.evidence() {
        return Err(invalid_case(case));
    }
    Ok(())
}

fn validate_referral(
    state: &AppState,
    case: &ProsecutionCaseRecord,
    refs: &ProsecutionCaseRefs<'_>,
    referral: &ProsecutionReferralRecord,
    seen: &mut SeenProsecutionArtifacts,
    referred_evidence: &mut BTreeSet<crate::core::id::EvidenceId>,
) -> Result<(), StateValidationError> {
    let invalid = || StateValidationError::InvalidProsecutionReferral {
        referral: referral.id(),
    };
    let information = state
        .intelligence
        .get_information(referral.information())
        .ok_or_else(invalid)?;
    let report = state
        .reports
        .get_report(referral.report())
        .ok_or_else(invalid)?;
    let prosecutor = state
        .world
        .get_character(referral.prosecutor())
        .ok_or_else(invalid)?;
    let is_initial = referral.id() == case.initial_referral();
    let expected_title = if is_initial {
        "Prosecution case referral"
    } else {
        "Prosecution evidence supplement"
    };
    let expected_summary = prosecution_referral_summary(
        refs.source_authority.name(),
        refs.defendant.name(),
        refs.office.name(),
        referral.evidence().len(),
        is_initial,
    );
    let Some(entry) = report.entries().first() else {
        return Err(invalid());
    };
    if !seen.referrals.insert(referral.id())
        || referral.prosecution_case() != case.id()
        || referral.source_investigation() != case.source_investigation()
        || referral.source_authority() != case.source_authority()
        || referral.prosecutor_office() != case.prosecutor_office()
        || referral.prosecutor() == case.defendant()
        || state
            .legal
            .case_witness_for(case.source_investigation(), referral.prosecutor())
            .is_some_and(|witness| witness.registered_at() < referral.referred_at())
        || prosecutor
            .capability(CapabilityKind::LegalKnowledge)
            .is_none()
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
                        || !evidence_concerns_defendant(evidence, case.defendant())
                        || evidence.discovered_at() > referral.referred_at()
                })
                || !referred_evidence.insert(*evidence_id)
        })
        || !seen.information.insert(referral.information())
        || information.holder() != KnowledgeHolder::Organization(case.prosecutor_office())
        || information.source_kind() != InformationSourceKind::AfterAction
        || information.topic() != InformationTopic::LegalActivity
        || information.source_entity() != Some(EntityRef::Organization(case.source_authority()))
        || information.subject() != EntityRef::Character(case.defendant())
        || information.observed_at() != referral.referred_at()
        || information.recorded_at() != referral.referred_at()
        || information.reliability() != Reliability::DirectAccess
        || information.specificity() != Specificity::Precise
        || !information.derived_from().is_empty()
        || information.summary() != expected_summary
        || !seen.reports.insert(referral.report())
        || report.recipient() != case.prosecutor_office()
        || report.kind() != ReportKind::Legal
        || report.title() != expected_title
        || report.generated_at() != referral.referred_at()
        || report.entries().len() != 1
        || entry.attention != AttentionClass::Notable
        || entry.summary != information.summary()
        || !entry.sources.is_empty()
        || entry.decision.is_some()
        || !case_entities_are_valid(case, &entry.entities, referral.prosecutor())
    {
        return Err(invalid());
    }
    Ok(())
}

fn case_entities_are_valid(
    case: &ProsecutionCaseRecord,
    entities: &BTreeSet<EntityRef>,
    prosecutor: crate::core::id::CharacterId,
) -> bool {
    entities.len() == 5
        && entities.contains(&EntityRef::Character(case.defendant()))
        && entities.contains(&EntityRef::Organization(case.source_authority()))
        && entities.contains(&EntityRef::Organization(case.prosecutor_office()))
        && entities.contains(&EntityRef::Character(prosecutor))
        && entities.contains(&EntityRef::Investigation(case.source_investigation()))
}

fn invalid_case(case: &ProsecutionCaseRecord) -> StateValidationError {
    StateValidationError::InvalidProsecutionCase { case: case.id() }
}
