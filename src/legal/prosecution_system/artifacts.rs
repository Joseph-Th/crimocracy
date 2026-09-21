//! Canonical prosecution information/report artifact construction and rendering.

use super::ProsecutionError;
use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{CharacterId, EvidenceId, InvestigationId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::intelligence::intelligence_system::{ValidatedInformation, validate_record_information};
use crate::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
    Specificity,
};
use crate::legal::{ProsecutionCaseRecord, ProsecutionCaseResolution};
use crate::reports::report_system::{ValidatedReport, validate_record_report};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use std::collections::BTreeSet;

/// Renders the canonical resolution summary text for a terminal prosecution case; one
/// template source shared by the commit path and the invariant pass's scratch-buffer
/// re-render. The report titles are plain literals owned by the match arms below.
pub(crate) fn write_resolution_summary(
    out: &mut impl std::fmt::Write,
    resolution: ProsecutionCaseResolution,
    office_name: &str,
    defendant_name: &str,
    lead_name: &str,
) -> std::fmt::Result {
    match resolution {
        ProsecutionCaseResolution::Declined => write!(
            out,
            "{office_name} declined prosecution of {defendant_name} after review by {lead_name}."
        ),
        ProsecutionCaseResolution::Closed => write!(
            out,
            "{office_name} closed its prosecution review of {defendant_name} after review by {lead_name}."
        ),
    }
}

pub(super) fn validate_resolution_artifacts(
    state: &AppState,
    case: &ProsecutionCaseRecord,
    prosecutor: CharacterId,
    resolution: ProsecutionCaseResolution,
    resolved_at: SimTime,
) -> Result<(ValidatedInformation, ValidatedReport), ProsecutionError> {
    let defendant_name = state
        .world
        .get_character(case.defendant())
        .expect("validated prosecution defendant must exist")
        .name();
    let office_name = state
        .world
        .get_organization(case.prosecutor_office())
        .expect("validated prosecutor office must exist")
        .name();
    let lead_name = state
        .world
        .get_character(prosecutor)
        .expect("validated lead prosecutor must exist")
        .name();
    let title = match resolution {
        ProsecutionCaseResolution::Declined => "Prosecution declined",
        ProsecutionCaseResolution::Closed => "Prosecution review closed",
    };
    let mut summary_buffer = String::new();
    write_resolution_summary(
        &mut summary_buffer,
        resolution,
        office_name,
        defendant_name,
        lead_name,
    )
    .expect("String buffer writes are infallible");
    let summary = summary_buffer;
    let information = validate_record_information(
        state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(case.prosecutor_office()),
            source_kind: InformationSourceKind::AfterAction,
            topic: InformationTopic::LegalActivity,
            source_entity: Some(EntityRef::Character(prosecutor)),
            subject: EntityRef::Character(case.defendant()),
            observed_at: resolved_at,
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary: summary.clone(),
        },
    )?;
    let report = validate_record_report(
        state,
        ReportDraft {
            recipient: case.prosecutor_office(),
            kind: ReportKind::Legal,
            title: title.to_owned(),
            entries: vec![ReportEntry {
                attention: AttentionClass::Notable,
                summary,
                sources: Vec::new(),
                entities: BTreeSet::from([
                    EntityRef::Character(case.defendant()),
                    EntityRef::Organization(case.source_authority()),
                    EntityRef::Organization(case.prosecutor_office()),
                    EntityRef::Character(prosecutor),
                    EntityRef::Investigation(case.source_investigation()),
                ]),
                decision: None,
            }],
        },
    )?;
    Ok((information, report))
}

pub(super) struct ReferralArtifactContext<'a> {
    pub(super) defendant: CharacterId,
    pub(super) source_investigation: InvestigationId,
    pub(super) source_authority: OrganizationId,
    pub(super) prosecutor_office: OrganizationId,
    pub(super) prosecutor: CharacterId,
    pub(super) evidence: &'a BTreeSet<EvidenceId>,
    pub(super) referred_at: SimTime,
    pub(super) initial: bool,
}

pub(crate) fn prosecution_referral_summary(
    source_authority: &str,
    defendant: &str,
    prosecutor_office: &str,
    evidence_count: usize,
    initial: bool,
) -> String {
    if initial {
        format!(
            "{source_authority} referred the arrest matter for {defendant} to {prosecutor_office}, sharing {evidence_count} evidence record(s)."
        )
    } else {
        format!(
            "{source_authority} supplemented the prosecution matter for {defendant} with {evidence_count} additional evidence record(s)."
        )
    }
}

pub(super) fn validate_referral_artifacts(
    state: &AppState,
    context: ReferralArtifactContext<'_>,
) -> Result<(ValidatedInformation, ValidatedReport), ProsecutionError> {
    let ReferralArtifactContext {
        defendant,
        source_investigation,
        source_authority,
        prosecutor_office,
        prosecutor,
        evidence,
        referred_at,
        initial,
    } = context;
    let defendant_name = state
        .world
        .get_character(defendant)
        .expect("validated defendant must exist")
        .name();
    let source_name = state
        .world
        .get_organization(source_authority)
        .expect("validated source authority must exist")
        .name();
    let office_name = state
        .world
        .get_organization(prosecutor_office)
        .expect("validated prosecutor office must exist")
        .name();
    let summary = prosecution_referral_summary(
        source_name,
        defendant_name,
        office_name,
        evidence.len(),
        initial,
    );
    let information = validate_record_information(
        state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(prosecutor_office),
            source_kind: InformationSourceKind::AfterAction,
            topic: InformationTopic::LegalActivity,
            source_entity: Some(EntityRef::Organization(source_authority)),
            subject: EntityRef::Character(defendant),
            observed_at: referred_at,
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary: summary.clone(),
        },
    )?;
    let report = validate_record_report(
        state,
        ReportDraft {
            recipient: prosecutor_office,
            kind: ReportKind::Legal,
            title: if initial {
                "Prosecution case referral".to_owned()
            } else {
                "Prosecution evidence supplement".to_owned()
            },
            entries: vec![ReportEntry {
                attention: AttentionClass::Notable,
                summary,
                sources: Vec::new(),
                entities: BTreeSet::from([
                    EntityRef::Character(defendant),
                    EntityRef::Organization(source_authority),
                    EntityRef::Organization(prosecutor_office),
                    EntityRef::Character(prosecutor),
                    EntityRef::Investigation(source_investigation),
                ]),
                decision: None,
            }],
        },
    )?;
    Ok((information, report))
}
