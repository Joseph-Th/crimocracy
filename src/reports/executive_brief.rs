//! Periodic executive-brief synthesis from organization-visible reports and unresolved addressed decisions.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{DecisionRequestId, InformationId, OrganizationId, ReportId};
use crate::core::state::AppState;
use crate::core::time::{SimDuration, SimTime};
use crate::decisions::{DecisionContext, DecisionRequestRecord};
use crate::registry::Registry;
use crate::reports::report_system::{validate_report_draft, ReportError, ValidatedReport};
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use std::cmp::Reverse;
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ExecutiveBriefError {
    #[error("executive brief is not due at {at:?}; authored cadence is {cadence:?}")]
    NotDue { at: SimTime, cadence: SimDuration },
    #[error("executive brief {report} was already generated at the current simulation time")]
    AlreadyGenerated { report: ReportId },
    #[error(
        "executive brief plan was generated at {expected:?}, but simulation time is now {found:?}"
    )]
    StaleTime { expected: SimTime, found: SimTime },
    #[error("executive brief source-report window changed after planning")]
    StaleReportWindow,
    #[error("executive brief pending-decision set changed after planning")]
    StalePendingDecisions,
    #[error(transparent)]
    Report(#[from] ReportError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DecisionDependency {
    id: DecisionRequestId,
    version: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct SourceEntryKey {
    attention: AttentionClass,
    summary: String,
    sources: Vec<InformationId>,
    entities: Vec<EntityRef>,
}

#[derive(Clone, Debug)]
struct SourceCandidate {
    report: ReportId,
    entry_index: usize,
    entry: ReportEntry,
}

#[derive(Clone, Debug)]
pub struct ExecutiveBriefPlan {
    recipient: OrganizationId,
    generated_at: SimTime,
    previous_brief: Option<ReportId>,
    latest_report: Option<ReportId>,
    pending_decisions: Vec<DecisionDependency>,
    entries: Vec<ReportEntry>,
}

impl ExecutiveBriefPlan {
    pub fn recipient(&self) -> OrganizationId {
        self.recipient
    }

    pub fn generated_at(&self) -> SimTime {
        self.generated_at
    }

    pub fn entries(&self) -> &[ReportEntry] {
        &self.entries
    }
}

pub fn is_executive_brief_due(registry: &Registry, at: SimTime) -> bool {
    let cadence = u64::from(registry.executive_brief().cadence().as_minutes());
    at != SimTime::ZERO && at.as_minutes().is_multiple_of(cadence)
}

pub fn decide_executive_brief(
    registry: &Registry,
    state: &AppState,
    recipient: OrganizationId,
) -> Result<ExecutiveBriefPlan, ExecutiveBriefError> {
    let definition = registry.executive_brief();
    if !is_executive_brief_due(registry, state.now()) {
        return Err(ExecutiveBriefError::NotDue {
            at: state.now(),
            cadence: definition.cadence(),
        });
    }

    let previous_brief = state
        .reports()
        .latest_for_kind(recipient, ReportKind::ExecutiveBrief);
    if let Some(report) = previous_brief.filter(|report| report.generated_at() == state.now()) {
        return Err(ExecutiveBriefError::AlreadyGenerated {
            report: report.id(),
        });
    }
    let previous_brief_id = previous_brief.map(|report| report.id());
    let latest_report = state
        .reports()
        .latest_for_recipient(recipient)
        .map(|report| report.id());

    let mut pending: Vec<_> = state.decisions().pending_for_recipient(recipient).collect();
    pending.sort_by_key(|decision| (Reverse(decision.attention()), decision.id()));
    let pending_decisions = pending
        .iter()
        .map(|decision| DecisionDependency {
            id: decision.id(),
            version: decision.version(),
        })
        .collect();

    let mut entries: Vec<ReportEntry> = pending
        .into_iter()
        .map(build_pending_decision_entry)
        .collect();

    let pending_ids: BTreeSet<_> = entries.iter().filter_map(|entry| entry.decision).collect();
    let mut source_candidates = collect_source_candidates(
        state,
        recipient,
        previous_brief_id,
        definition.minimum_source_attention(),
        &pending_ids,
    );
    source_candidates.sort_by_key(|candidate| {
        (
            Reverse(candidate.entry.attention),
            candidate.report,
            candidate.entry_index,
        )
    });
    let source_candidates = deduplicate_source_candidates(source_candidates);
    let max_source_entries = usize::from(definition.max_source_entries());
    let omitted = source_candidates.len().saturating_sub(max_source_entries);
    entries.extend(
        source_candidates
            .into_iter()
            .take(max_source_entries)
            .map(|candidate| candidate.entry),
    );
    if omitted > 0 {
        // The omitted tail may include Exception or Crisis items, so avoid mislabeling them as
        // only "notable": report the count without overclaiming a specific attention class.
        entries.push(ReportEntry {
            attention: AttentionClass::Notable,
            summary: format!("{omitted} additional items remain available in underlying reports."),
            sources: Vec::new(),
            entities: BTreeSet::new(),
            decision: None,
        });
    }
    if entries.is_empty() {
        entries.push(ReportEntry {
            attention: AttentionClass::Routine,
            summary: "No immediate decision or notable exception requires executive attention."
                .to_owned(),
            sources: Vec::new(),
            entities: BTreeSet::new(),
            decision: None,
        });
    }

    Ok(ExecutiveBriefPlan {
        recipient,
        generated_at: state.now(),
        previous_brief: previous_brief_id,
        latest_report,
        pending_decisions,
        entries,
    })
}

fn collect_source_candidates(
    state: &AppState,
    recipient: OrganizationId,
    previous_brief: Option<ReportId>,
    minimum_attention: AttentionClass,
    pending_decisions: &BTreeSet<DecisionRequestId>,
) -> Vec<SourceCandidate> {
    let mut candidates = Vec::new();
    for report in state.reports().reports_for_after(recipient, previous_brief) {
        if report.kind() == ReportKind::ExecutiveBrief {
            continue;
        }
        if disposition_report_is_redundant_in_window(state, report, previous_brief) {
            continue;
        }
        if state
            .opportunities()
            .opportunity_for_report(report.id())
            .is_some_and(|opportunity| {
                report.id() == opportunity.report()
                    && opportunity.status() != crate::opportunities::OpportunityStatus::Open
            })
        {
            continue;
        }
        for (entry_index, entry) in report.entries().iter().enumerate() {
            if entry.attention < minimum_attention {
                continue;
            }
            if let Some(decision) = entry.decision {
                if pending_decisions.contains(&decision) {
                    continue;
                }
                if state
                    .decisions()
                    .get_decision(decision)
                    .is_some_and(|record| {
                        record.status() != crate::decisions::DecisionStatus::Pending
                    })
                {
                    continue;
                }
            }
            let entry = refresh_operation_financial_state(state, report.id(), entry);
            candidates.push(SourceCandidate {
                report: report.id(),
                entry_index,
                entry,
            });
        }
    }
    candidates
}

fn disposition_report_is_redundant_in_window(
    state: &AppState,
    report: &crate::reports::ReportRecord,
    previous_brief: Option<ReportId>,
) -> bool {
    // Structural identity only: a Financial-kind report that some operation's disposition
    // points back to is a property-disposition report. Display titles are not identity.
    if report.kind() != ReportKind::Financial {
        return false;
    }
    report.entries().iter().any(|entry| {
        entry.entities.iter().any(|entity| {
            let EntityRef::Operation(operation_id) = entity else {
                return false;
            };
            state
                .operations()
                .get_operation(*operation_id)
                .is_some_and(|operation| {
                    operation
                        .property_disposition()
                        .is_some_and(|disposition| disposition.report() == report.id())
                        && operation.resolution().is_some_and(|resolution| {
                            previous_brief
                                .is_none_or(|previous| resolution.after_action_report() > previous)
                        })
                })
        })
    })
}

fn refresh_operation_financial_state(
    state: &AppState,
    source_report: ReportId,
    entry: &ReportEntry,
) -> ReportEntry {
    let mut refreshed = entry.clone();
    for entity in &entry.entities {
        let EntityRef::Operation(operation_id) = entity else {
            continue;
        };
        let Some(operation) = state.operations().get_operation(*operation_id) else {
            continue;
        };
        let Some(resolution) = operation.resolution() else {
            continue;
        };
        if resolution.after_action_report() != source_report {
            continue;
        }
        let (Some(proceeds), Some(disposition)) = (
            resolution.property_proceeds(),
            operation.property_disposition(),
        ) else {
            continue;
        };
        let Some(venue) = state.world().get_business(disposition.venue()) else {
            continue;
        };
        // The clause pair is produced by one owner (`operation_economics`), so this refresh
        // matches on exact text rather than parsing. The `contains` guard is also the drift
        // alarm: if either clause's wording changes without the other, the unliquidated
        // clause silently survives into every later brief.
        let prior = crate::operations::operation_economics::unliquidated_property_clause(
            proceeds.estimated_value().cents(),
        );
        if !refreshed.summary.contains(&prior) {
            continue;
        }
        let current = crate::operations::operation_economics::liquidated_property_clause(
            proceeds.estimated_value().cents(),
            venue.name(),
            disposition.realized_value().cents(),
        );
        // Refresh exactly one clause occurrence per operation so two operations sharing an
        // identical estimated value each get their own clause liquidated instead of the
        // second silently skipping.
        if let Some(position) = refreshed.summary.find(&prior) {
            let end = position + prior.len();
            refreshed.summary.replace_range(position..end, &current);
        }
    }
    refreshed
}

fn deduplicate_source_candidates(candidates: Vec<SourceCandidate>) -> Vec<SourceCandidate> {
    let mut seen = BTreeSet::new();
    candidates
        .into_iter()
        .filter(|candidate| seen.insert(source_entry_key(&candidate.entry)))
        .collect()
}

fn source_entry_key(entry: &ReportEntry) -> SourceEntryKey {
    let mut sources = entry.sources.clone();
    sources.sort_unstable();
    sources.dedup();
    SourceEntryKey {
        attention: entry.attention,
        summary: entry.summary.clone(),
        sources,
        entities: entry.entities.iter().copied().collect(),
    }
}

fn build_pending_decision_entry(decision: &DecisionRequestRecord) -> ReportEntry {
    ReportEntry {
        attention: decision.attention(),
        summary: format!("Decision required: {}", decision.summary()),
        sources: Vec::new(),
        entities: decision_entities(decision),
        decision: Some(decision.id()),
    }
}

fn decision_entities(decision: &DecisionRequestRecord) -> BTreeSet<EntityRef> {
    let mut entities = BTreeSet::from([
        EntityRef::Character(decision.requester()),
        EntityRef::DecisionRequest(decision.id()),
    ]);
    match decision.context() {
        DecisionContext::OperationException {
            operation,
            reason: _,
        } => {
            entities.insert(EntityRef::Operation(operation));
        }
        DecisionContext::RecruitmentApproval(context) => {
            entities.insert(EntityRef::Organization(context.target_organization()));
            entities.insert(EntityRef::Character(context.recruiter()));
            entities.insert(EntityRef::Character(context.candidate()));
            entities.insert(EntityRef::Mandate(context.authority().authority().mandate));
        }
    }
    entities
}

pub struct ValidatedExecutiveBrief {
    plan: ExecutiveBriefPlan,
    report: ValidatedReport,
}

impl ValidatedExecutiveBrief {
    pub fn commit(self, state: &mut AppState) -> Result<ReportId, ExecutiveBriefError> {
        validate_plan_dependencies(state, &self.plan)?;
        Ok(self.report.commit(state)?)
    }
}

pub fn validate_executive_brief_plan(
    state: &AppState,
    plan: ExecutiveBriefPlan,
) -> Result<ValidatedExecutiveBrief, ExecutiveBriefError> {
    validate_plan_dependencies(state, &plan)?;
    let report = validate_report_draft(
        state,
        ReportDraft {
            recipient: plan.recipient,
            kind: ReportKind::ExecutiveBrief,
            title: "Executive brief".to_owned(),
            entries: plan.entries.clone(),
        },
    )?;
    Ok(ValidatedExecutiveBrief { plan, report })
}

fn validate_plan_dependencies(
    state: &AppState,
    plan: &ExecutiveBriefPlan,
) -> Result<(), ExecutiveBriefError> {
    if state.now() != plan.generated_at {
        return Err(ExecutiveBriefError::StaleTime {
            expected: plan.generated_at,
            found: state.now(),
        });
    }
    let previous_brief = state
        .reports()
        .latest_for_kind(plan.recipient, ReportKind::ExecutiveBrief)
        .map(|report| report.id());
    let latest_report = state
        .reports()
        .latest_for_recipient(plan.recipient)
        .map(|report| report.id());
    if previous_brief != plan.previous_brief || latest_report != plan.latest_report {
        return Err(ExecutiveBriefError::StaleReportWindow);
    }
    let mut current_pending: Vec<_> = state
        .decisions()
        .pending_for_recipient(plan.recipient)
        .collect();
    current_pending.sort_by_key(|decision| (Reverse(decision.attention()), decision.id()));
    let current_pending: Vec<_> = current_pending
        .into_iter()
        .map(|decision| DecisionDependency {
            id: decision.id(),
            version: decision.version(),
        })
        .collect();
    if current_pending != plan.pending_decisions {
        return Err(ExecutiveBriefError::StalePendingDecisions);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
