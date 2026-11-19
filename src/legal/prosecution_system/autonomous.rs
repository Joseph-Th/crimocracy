//! Autonomous institutional intake and evidence supplements.
//!
//! Arrest is the existing independently corroborated custody gate; referral does not imply
//! charges or conviction. Prosecutors receive defendant-specific evidence through the same
//! canonical transactions as explicit institutional actions. A daily finite intake budget
//! prevents every historical arrest being processed in one minute.

use super::{
    ProsecutionCaseDraft, ProsecutionError, ProsecutionReferralDraft,
    ValidatedProsecutionCaseOpening, ValidatedProsecutionCaseResolution,
    ValidatedProsecutionReferral, evidence_concerns_defendant, validate_decline_prosecution_case,
    validate_open_prosecution_case, validate_prosecutor, validate_supplement_prosecution_case,
};
use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{
    ArrestId, CharacterId, EvidenceId, IdKind, OrganizationId, ProsecutionCaseId,
    ProsecutionReferralId,
};
use crate::core::state::AppState;
use crate::core::time::is_day_boundary;
use crate::intelligence::intelligence_system::{
    ValidatedInformation, validate_record_information_with_signal,
};
use crate::intelligence::{
    InformationDraft, InformationSignal, InformationSourceKind, InformationTopic, KnowledgeHolder,
    LegalPersonStatusSignal, Reliability, Specificity,
};
use crate::legal::{
    Admissibility, ArrestRecord, ArrestStatus, InvestigationStatus, ProsecutionCaseStatus,
};
use crate::registry::Registry;
use crate::reports::report_system::{ValidatedReport, validate_record_report};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::world::{CapabilityKind, OrganizationKind};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(crate) struct AutonomousProsecutionOutcome {
    pub(crate) opened: Vec<ProsecutionCaseId>,
    pub(crate) supplemented: Vec<ProsecutionReferralId>,
    pub(crate) declined: Vec<ProsecutionCaseId>,
}

struct MemberNotice {
    history: ValidatedInformation,
    report: ValidatedReport,
}

fn member_notice(
    state: &AppState,
    arrest: &ArrestRecord,
) -> Result<Option<MemberNotice>, ProsecutionError> {
    let defendant = state
        .world()
        .get_character(arrest.character())
        .ok_or(ProsecutionError::MissingDefendant(arrest.character()))?;
    let Some(sponsor) = defendant.organization() else {
        return Ok(None);
    };
    if state
        .world()
        .get_organization(sponsor)
        .is_none_or(|org| org.kind() != OrganizationKind::Criminal)
    {
        return Ok(None);
    }
    // A member can tell their employer about their own case and prior detention. Date the
    // detention fact to the arrest, not today: a released person must not be reported detained.
    // The proceedings notice conveys no private evidence, witness list or prosecutor strategy.
    let history = validate_record_information_with_signal(
        state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(sponsor),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::LegalActivity,
            source_entity: Some(EntityRef::Character(defendant.id())),
            subject: EntityRef::Character(defendant.id()),
            observed_at: arrest.arrested_at(),
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary: format!(
                "{} reports their detention under {}.",
                defendant.name(),
                arrest.id()
            ),
        },
        InformationSignal::LegalPersonStatus(LegalPersonStatusSignal::Detained {
            arrest: arrest.id(),
        }),
    )?;
    let report = validate_record_report(
        state,
        ReportDraft {
            recipient: sponsor,
            kind: ReportKind::Legal,
            title: "Prosecutorial review opened".to_owned(),
            entries: vec![ReportEntry {
                attention: AttentionClass::Exception,
                summary: format!(
                    "{}'s arrest matter {} has been referred for prosecutorial review. Counsel may continue after release; review is not a conviction.",
                    defendant.name(),
                    arrest.id()
                ),
                sources: Vec::new(),
                entities: BTreeSet::from([EntityRef::Character(defendant.id())]),
                decision: None,
            }],
        },
    )?;
    Ok(Some(MemberNotice { history, report }))
}

fn decline_notice(
    state: &AppState,
    arrest: &ArrestRecord,
) -> Result<Option<ValidatedReport>, ProsecutionError> {
    let defendant = state
        .world()
        .get_character(arrest.character())
        .ok_or(ProsecutionError::MissingDefendant(arrest.character()))?;
    let Some(sponsor) = defendant.organization() else {
        return Ok(None);
    };
    if state
        .world()
        .get_organization(sponsor)
        .is_none_or(|org| org.kind() != OrganizationKind::Criminal)
    {
        return Ok(None);
    }
    Ok(Some(validate_record_report(
        state,
        ReportDraft {
            recipient: sponsor,
            kind: ReportKind::Legal,
            title: "Prosecution declined".to_owned(),
            entries: vec![ReportEntry {
                attention: AttentionClass::Notable,
                summary: format!(
                    "{}'s arrest matter {} was declined after police casework ended without sufficient admissible, independently corroborated evidence.",
                    defendant.name(),
                    arrest.id()
                ),
                sources: Vec::new(),
                entities: BTreeSet::from([EntityRef::Character(defendant.id())]),
                decision: None,
            }],
        },
    )?))
}

fn defendant_evidence(
    state: &AppState,
    arrest: &ArrestRecord,
) -> Result<BTreeSet<EvidenceId>, ProsecutionError> {
    let investigation = state
        .legal()
        .get_investigation(arrest.investigation())
        .ok_or(ProsecutionError::MissingInvestigation(
            arrest.investigation(),
        ))?;
    let mut evidence = arrest.evidence().clone();
    for id in investigation.evidence() {
        let record = state
            .legal()
            .get_evidence(*id)
            .ok_or(ProsecutionError::MissingEvidence(*id))?;
        if record.custodian() == arrest.authority()
            && evidence_concerns_defendant(record, arrest.character())
        {
            evidence.insert(*id);
        }
    }
    Ok(evidence)
}

enum Action {
    Open(ValidatedProsecutionCaseOpening, Option<MemberNotice>),
    Supplement(ValidatedProsecutionReferral),
    Decline(
        ProsecutionCaseId,
        ValidatedProsecutionCaseResolution,
        Option<ValidatedReport>,
    ),
}

impl Action {
    fn id_budget(&self) -> Vec<(IdKind, u32)> {
        match self {
            Self::Open(_, notice) => {
                let artifacts = 1 + u32::from(notice.is_some());
                vec![
                    (IdKind::Information, artifacts),
                    (IdKind::Report, artifacts),
                    (IdKind::ProsecutionCase, 1),
                    (IdKind::ProsecutionReferral, 1),
                ]
            }
            Self::Supplement(_) => vec![
                (IdKind::Information, 1),
                (IdKind::Report, 1),
                (IdKind::ProsecutionReferral, 1),
            ],
            Self::Decline(_, _, notice) => vec![
                (IdKind::Information, 1),
                (IdKind::Report, 1 + u32::from(notice.is_some())),
            ],
        }
    }
}

/// Current own-member legal matters are known to the defendant; this projection does not expose
/// a foreign prosecution file or its evidence. The adapter must separately know the arrest.
pub fn member_prosecution_under_review(
    state: &AppState,
    organization: OrganizationId,
    arrest: ArrestId,
) -> bool {
    state.legal().get_arrest(arrest).is_some_and(|record| {
        state
            .world()
            .get_character(record.character())
            .is_some_and(|person| person.organization() == Some(organization))
            && state.legal().has_open_prosecution_case_for_arrest(arrest)
    })
}

pub(crate) fn apply_due_prosecution_referrals(
    registry: &Registry,
    state: &mut AppState,
) -> Result<AutonomousProsecutionOutcome, ProsecutionError> {
    if !is_day_boundary(state.now()) {
        return Ok(AutonomousProsecutionOutcome::default());
    }
    let mut prosecutors = BTreeMap::<CharacterId, (OrganizationId, u8, usize)>::new();
    for office in state
        .world()
        .organizations()
        .filter(|org| org.kind() == OrganizationKind::Prosecutor)
    {
        for person in state.world().characters_in_organization(office.id()) {
            if state
                .legal()
                .active_arrest_for_character(person.id())
                .is_some()
            {
                continue;
            }
            if let Some(skill) = person.capability(CapabilityKind::LegalKnowledge) {
                prosecutors.insert(
                    person.id(),
                    (
                        office.id(),
                        skill.value(),
                        state
                            .legal()
                            .reviewing_prosecution_cases_for_prosecutor(person.id())
                            .count(),
                    ),
                );
            }
        }
    }
    if prosecutors.is_empty() {
        return Ok(AutonomousProsecutionOutcome::default());
    }

    let mut plans = Vec::new();
    let mut used = BTreeSet::new();
    let mut reviewing_subjects = BTreeSet::new();
    // Plan each already-open review first. Supplemental work has priority over new intake.
    for arrest_id in state.legal().arrests_with_open_prosecutions() {
        let arrest = state
            .legal()
            .get_arrest(arrest_id)
            .expect("open review owns its arrest");
        reviewing_subjects.insert((arrest.investigation(), arrest.character()));
        let evidence = defendant_evidence(state, arrest)?;
        for case in state
            .legal()
            .prosecution_cases_for_arrest(arrest_id)
            .filter(|case| case.status() == ProsecutionCaseStatus::Reviewing)
        {
            let Some(lead) = case.assigned_prosecutor() else {
                continue;
            };
            if !prosecutors.contains_key(&lead)
                || used.contains(&lead)
                || case.version() > u32::MAX - 2
            {
                continue;
            }
            let additions: BTreeSet<_> = evidence.difference(case.evidence()).copied().collect();
            let action = if !additions.is_empty() {
                Action::Supplement(validate_supplement_prosecution_case(
                    state,
                    ProsecutionReferralDraft {
                        prosecution_case: case.id(),
                        evidence: additions,
                    },
                )?)
            } else {
                // The case is ready for a conservative screening decision only after release
                // and after police stop active work. Unknown admissibility is not proof.
                // A sufficient referred file stays open for a future adjudication phase;
                // this is NOT a conviction or a way to extend arrest custody.
                let investigation = state
                    .legal()
                    .get_investigation(arrest.investigation())
                    .ok_or(ProsecutionError::MissingInvestigation(
                        arrest.investigation(),
                    ))?;
                if arrest.status() != ArrestStatus::Released
                    || investigation.status() == InvestigationStatus::Active
                {
                    continue;
                }
                let admissible: BTreeSet<_> = case
                    .evidence()
                    .iter()
                    .copied()
                    .filter(|id| {
                        state
                            .legal()
                            .get_evidence(*id)
                            .is_some_and(|e| e.admissibility() == Admissibility::Admissible)
                    })
                    .collect();
                if crate::legal::arrest_system::arrest_evidence_meets_threshold(
                    state,
                    &admissible,
                    registry.legal().minimum_arrest_qualifying_evidence(),
                ) {
                    continue;
                }
                Action::Decline(
                    case.id(),
                    validate_decline_prosecution_case(state, case.id())?,
                    decline_notice(state, arrest)?,
                )
            };
            if state.ids.reserve_many(&action.id_budget()).is_ok() {
                used.insert(lead);
                plans.push(action);
            }
        }
    }

    // Prioritize actual custody, then older referrals; released defendants remain eligible.
    // Prior completed review is not reopened merely because the same arrest still exists.
    let mut pending: Vec<_> = state
        .legal()
        .arrests()
        .filter(|arrest| {
            state
                .legal()
                .prosecution_cases_for_arrest(arrest.id())
                .next()
                .is_none()
        })
        .collect();
    pending.sort_unstable_by_key(|arrest| {
        (
            arrest.status() != ArrestStatus::Detained,
            arrest.arrested_at(),
            arrest.id(),
        )
    });
    for arrest in pending {
        let subject = (arrest.investigation(), arrest.character());
        if reviewing_subjects.contains(&subject) {
            continue;
        }
        let lead = prosecutors
            .iter()
            .filter(|(id, (office, _, _))| {
                !used.contains(id)
                    && validate_prosecutor(
                        state,
                        *office,
                        **id,
                        arrest.character(),
                        arrest.investigation(),
                    )
                    .is_ok()
            })
            .min_by_key(|(id, (_, skill, workload))| (*workload, Reverse(*skill), **id));
        let Some((&lead, &(office, _, _))) = lead else {
            continue;
        };
        let plan = validate_open_prosecution_case(
            state,
            ProsecutionCaseDraft {
                arrest: arrest.id(),
                prosecutor_office: office,
                prosecutor: lead,
                evidence: defendant_evidence(state, arrest)?,
            },
        )?;
        let action = Action::Open(plan, member_notice(state, arrest)?);
        if state.ids.reserve_many(&action.id_budget()).is_ok() {
            used.insert(lead);
            reviewing_subjects.insert(subject);
            plans.push(action);
        }
    }

    let budget: Vec<_> = plans.iter().flat_map(Action::id_budget).collect();
    if state.ids.reserve_many(&budget).is_err() {
        return Ok(AutonomousProsecutionOutcome::default());
    }
    let mut outcome = AutonomousProsecutionOutcome::default();
    for action in plans {
        match action {
            Action::Open(plan, notice) => {
                let id = plan
                    .commit(state)
                    .expect("preflighted independent prosecution intake must commit");
                if let Some(notice) = notice {
                    notice
                        .history
                        .commit(state)
                        .expect("member notice information was preflighted");
                    notice
                        .report
                        .commit(state)
                        .expect("member notice report was preflighted");
                }
                outcome.opened.push(id);
            }
            Action::Supplement(plan) => {
                outcome.supplemented.push(
                    plan.commit(state)
                        .expect("preflighted independent prosecution supplement must commit"),
                );
            }
            Action::Decline(id, plan, notice) => {
                plan.commit(state)
                    .expect("preflighted review disposition must commit");
                if let Some(notice) = notice {
                    notice
                        .commit(state)
                        .expect("member disposition notice was preflighted");
                }
                outcome.declined.push(id);
            }
        }
    }
    Ok(outcome)
}
