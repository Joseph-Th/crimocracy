//! Abstract judicial disposition after a preparation window. Review, arrest and
//! conviction remain distinct. The court applies a higher proof standard and
//! admits no exhibit from a previously adjudicated period of the same file.

use super::{
    ProsecutionError, ValidatedProsecutionCaseResolution, validate_prosecution_case_resolution,
};
use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{ArrestId, IdKind, LegalRepresentationId, OrganizationId, ProsecutionCaseId};
use crate::core::state::AppState;
use crate::core::time::{DAY_MINUTES, SimTime};
use crate::finance::Money;
use crate::legal::{
    JudicialJudgment, ProsecutionCaseRecord, ProsecutionCaseResolution, ProsecutionCaseStatus,
};
use crate::registry::Registry;
use crate::reports::report_system::{ValidatedReport, validate_record_report};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::world::{CapabilityKind, OrganizationKind};
use std::collections::BTreeSet;

const PREPARATION_DAYS: u64 = 14;
const FINE_WAGE_DAYS: i64 = 60;

/// Screening uses a lower institutional threshold than a verdict. After an earlier
/// adjudication, old primary exhibits may remain in the file but cannot reopen its
/// adjudicated period. New derivatives of those exhibits are old facts as well.
pub(super) fn screened_case(
    registry: &Registry,
    state: &AppState,
    case: &ProsecutionCaseRecord,
    at: SimTime,
) -> bool {
    let cutoff = prior_judgment(state, case, at);
    let evidence = case
        .evidence()
        .iter()
        .copied()
        .filter(|id| {
            let Some(e) = state.legal().get_evidence(*id) else {
                return false;
            };
            if e.admissibility() != crate::legal::Admissibility::Admissible {
                return false;
            }
            cutoff.is_none_or(|cutoff| {
                let root = match e.derived_from().len() {
                    0 => Some(e),
                    1 => e
                        .derived_from()
                        .iter()
                        .next()
                        .and_then(|id| state.legal().get_evidence(*id)),
                    _ => None,
                };
                root.is_some_and(|root| root.discovered_at() > cutoff)
            })
        })
        .collect();
    crate::legal::arrest_system::arrest_evidence_meets_threshold(
        state,
        &evidence,
        registry.legal().minimum_arrest_qualifying_evidence(),
    )
}

pub(super) fn judgment_due(state: &AppState, case: &ProsecutionCaseRecord) -> bool {
    case.status() == ProsecutionCaseStatus::Reviewing
        && state
            .now()
            .as_minutes()
            .saturating_sub(case.opened_at().as_minutes())
            >= PREPARATION_DAYS * DAY_MINUTES
        && state
            .legal()
            .active_arrest_for_character(case.defendant())
            .is_none()
}

fn prior_judgment(state: &AppState, case: &ProsecutionCaseRecord, at: SimTime) -> Option<SimTime> {
    state
        .legal()
        .prosecution_cases()
        .filter(|prior| {
            prior.id() != case.id()
                && prior.defendant() == case.defendant()
                && prior.source_investigation() == case.source_investigation()
                && prior.judgment().is_some()
        })
        .filter_map(|prior| {
            prior
                .resolved_at()
                .filter(|time| (*time, prior.id()) < (at, case.id()))
        })
        .max()
}

fn defense_skill(state: &AppState, defense: Option<LegalRepresentationId>) -> u8 {
    defense
        .and_then(|id| state.legal().get_legal_representation(id))
        .and_then(|record| state.world().get_character(record.counsel()))
        .and_then(|person| person.capability(CapabilityKind::LegalKnowledge))
        .map_or(0, |rating| rating.value())
}

fn assessment(
    registry: &Registry,
    state: &AppState,
    case: &ProsecutionCaseRecord,
    at: SimTime,
    defense: Option<LegalRepresentationId>,
) -> (ProsecutionCaseResolution, JudicialJudgment) {
    let (independent_sources, strong_sources, proof_score) =
        crate::legal::arrest_system::judicial_proof(
            state,
            case.evidence(),
            prior_judgment(state, case, at),
        );
    // A case cannot win on one witness, association, or many weak findings. Skilled
    // representation raises the proof burden by challenging the prosecution's exhibits.
    let threshold = 8 + u64::from(defense_skill(state, defense) / 40);
    let convicted = independent_sources >= 2 && strong_sources >= 2 && proof_score >= threshold;
    let resolution = if convicted {
        ProsecutionCaseResolution::Convicted
    } else {
        ProsecutionCaseResolution::Acquitted
    };
    let fine = if convicted {
        registry
            .upkeep()
            .per_member_daily()
            .checked_mul(FINE_WAGE_DAYS)
            .unwrap_or(Money::from_cents(i64::MAX))
    } else {
        Money::ZERO
    };
    (
        resolution,
        JudicialJudgment {
            defense,
            independent_sources,
            strong_sources,
            proof_score,
            fine,
            payments: BTreeSet::new(),
        },
    )
}

pub(super) struct ValidatedJudgment {
    case: ProsecutionCaseId,
    resolution: ValidatedProsecutionCaseResolution,
    judgment: JudicialJudgment,
    notice: Option<ValidatedReport>,
}

impl ValidatedJudgment {
    pub(super) fn id_budget(&self) -> Vec<(IdKind, u32)> {
        vec![
            (IdKind::Information, 1),
            (IdKind::Report, 1 + u32::from(self.notice.is_some())),
        ]
    }

    pub(super) fn commit(
        self,
        registry: &Registry,
        state: &mut AppState,
    ) -> Result<ProsecutionCaseId, ProsecutionError> {
        let case = state
            .legal()
            .get_prosecution_case(self.case)
            .ok_or(ProsecutionError::MissingProsecutionCase(self.case))?;
        let defense = state
            .legal()
            .active_representation_for_arrest(case.arrest())
            .map(|rep| rep.id());
        if !judgment_due(state, case)
            || !screened_case(registry, state, case, state.now())
            || assessment(registry, state, case, state.now(), defense).1 != self.judgment
        {
            return Err(ProsecutionError::StaleJudgment(self.case));
        }
        state.ids.reserve_many(&self.id_budget())?;
        self.resolution.commit(state)?;
        state.legal.attach_judgment(self.case, self.judgment);
        if let Some(notice) = self.notice {
            notice
                .commit(state)
                .expect("judgment notice ID was preflighted");
        }
        Ok(self.case)
    }
}

pub(super) fn validate_judgment(
    registry: &Registry,
    state: &AppState,
    id: ProsecutionCaseId,
) -> Result<ValidatedJudgment, ProsecutionError> {
    let case = state
        .legal()
        .get_prosecution_case(id)
        .ok_or(ProsecutionError::MissingProsecutionCase(id))?;
    if !judgment_due(state, case) || !screened_case(registry, state, case, state.now()) {
        return Err(ProsecutionError::JudgmentNotDue(id));
    }
    let person = state
        .world()
        .get_character(case.defendant())
        .ok_or(ProsecutionError::MissingDefendant(case.defendant()))?;
    let defense = state
        .legal()
        .active_representation_for_arrest(case.arrest())
        .map(|rep| rep.id());
    let (outcome, judgment) = assessment(registry, state, case, state.now(), defense);
    let resolution = validate_prosecution_case_resolution(state, id, outcome)?;
    let convicted = outcome == ProsecutionCaseResolution::Convicted;
    let notice = person
        .organization()
        .filter(|organization| {
            state
                .world()
                .get_organization(*organization)
                .is_some_and(|record| record.kind() == OrganizationKind::Criminal)
        })
        .map(|recipient| {
            let summary = if convicted {
                format!(
                    "{} was convicted in the matter of {}. The court imposed a personal fine of {}, collectible from the defendant's accessible funds.",
                    person.name(),
                    case.arrest(),
                    crate::finance::helpers::format_money_cents(judgment.fine.cents()),
                )
            } else {
                format!(
                    "{} was acquitted in the matter of {}. The court's proof standard was not met; no fine was imposed.",
                    person.name(), case.arrest(),
                )
            };
            validate_record_report(
                state,
                ReportDraft {
                    recipient,
                    kind: ReportKind::Legal,
                    title: if convicted { "Court conviction" } else { "Court acquittal" }.to_owned(),
                    entries: vec![ReportEntry {
                        attention: AttentionClass::Exception,
                        summary,
                        sources: Vec::new(),
                        entities: BTreeSet::from([EntityRef::Character(person.id())]),
                        decision: None,
                    }],
                },
            )
        })
        .transpose()?;
    Ok(ValidatedJudgment {
        case: id,
        resolution,
        judgment,
        notice,
    })
}

/// The represented person knows their own judgment. Only the adapter's known member
/// arrest is accepted; the projection carries no police evidence or private court file.
#[derive(Clone, Copy, Debug)]
pub struct MemberJudgment {
    pub status: ProsecutionCaseStatus,
    pub resolved_at: SimTime,
    pub fine: Money,
    pub outstanding: Money,
}

pub fn member_judgments(
    state: &AppState,
    organization: OrganizationId,
    arrest: ArrestId,
) -> Vec<MemberJudgment> {
    if state.legal().get_arrest(arrest).is_none_or(|record| {
        state
            .world()
            .get_character(record.character())
            .is_none_or(|person| person.organization() != Some(organization))
    }) {
        return Vec::new();
    }
    state
        .legal()
        .prosecution_cases_for_arrest(arrest)
        .filter_map(|case| {
            let judgment = case.judgment()?;
            Some(MemberJudgment {
                status: case.status(),
                resolved_at: case.resolved_at()?,
                fine: judgment.fine(),
                outstanding: super::fine_collection::outstanding(state, case)?,
            })
        })
        .collect()
}

pub(crate) fn validate_judgment_history(
    registry: &Registry,
    state: &AppState,
    case: &ProsecutionCaseRecord,
) -> bool {
    let Some(judgment) = case.judgment() else {
        return !matches!(
            case.status(),
            ProsecutionCaseStatus::Convicted | ProsecutionCaseStatus::Acquitted
        );
    };
    let Some(at) = case.resolved_at() else {
        return false;
    };
    if at
        .as_minutes()
        .saturating_sub(case.opened_at().as_minutes())
        < PREPARATION_DAYS * DAY_MINUTES
        || !screened_case(registry, state, case, at)
        || state.legal().arrests().any(|arrest| {
            arrest.character() == case.defendant()
                && arrest.arrested_at() < at
                && arrest.released_at().is_none_or(|release| release > at)
        })
    {
        return false;
    }
    // Representation history is append-only. A retainer spanning the verdict
    // unambiguously must be reflected in the frozen assessment; same-minute
    // boundary ties remain governed by the explicit recorded defense.
    if state.legal().legal_representations().any(|rep| {
        rep.arrest() == case.arrest()
            && rep.retained_at() < at
            && rep.ended_at().is_none_or(|end| end > at)
            && Some(rep.id()) != judgment.defense
    }) {
        return false;
    }
    if let Some(id) = judgment.defense {
        let Some(rep) = state.legal().get_legal_representation(id) else {
            return false;
        };
        if rep.arrest() != case.arrest()
            || rep.defendant() != case.defendant()
            || rep.retained_at() > at
            || rep.ended_at().is_some_and(|end| end < at)
        {
            return false;
        }
    }
    let (expected, mut proof) = assessment(registry, state, case, at, judgment.defense);
    proof.payments.clone_from(&judgment.payments);
    expected.status() == case.status() && &proof == judgment
}
