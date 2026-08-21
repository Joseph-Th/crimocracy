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
    if report.kind() != ReportKind::Financial || report.title() != "Property disposition" {
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
        let prior = crate::operations::operation_execution::unliquidated_property_clause(
            proceeds.estimated_value().cents(),
        );
        if !refreshed.summary.contains(&prior) {
            continue;
        }
        let current = crate::operations::operation_execution::liquidated_property_clause(
            proceeds.estimated_value().cents(),
            venue.name(),
            disposition.realized_value().cents(),
        );
        refreshed.summary = refreshed.summary.replace(&prior, &current);
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
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::invariants::{validate_invariants, validate_state};
    use crate::core::persistence::{build_save, restore_save, SaveEnvelope};
    use crate::core::simulation::run_tick;
    use crate::decisions::decision_system::{
        validate_request_recruitment_approval, validate_resolve_decision,
    };
    use crate::decisions::{DecisionResponse, RecruitmentApprovalRequestDraft};
    use crate::delegation::delegation_system::validate_assign_mandate;
    use crate::delegation::{
        MandateAuthority, MandateDraft, ResponsibilityFunction, ResponsibilityScope,
    };
    use crate::recruitment::RecruitmentApproach;
    use crate::reports::report_system::validate_record_report;
    use crate::social::relationship_system::validate_set_relationship;
    use crate::social::{RelationshipDimensions, RelationshipLevel};
    use crate::world::world_system::{
        designate_player_organization, insert_character, insert_organization,
    };
    use crate::world::{
        AutonomyLevel, CapabilityKind, CharacterDraft, OrganizationDraft, OrganizationKind, Rating,
    };
    use std::collections::{BTreeMap, BTreeSet};

    struct BriefFixture {
        registry: Registry,
        state: AppState,
        organization: OrganizationId,
        recruiter: crate::core::id::CharacterId,
        candidate: crate::core::id::CharacterId,
        mandate: crate::core::id::MandateId,
    }

    fn rating(value: u8) -> Rating {
        Rating::try_new(value).expect("fixture rating must be valid")
    }

    fn level(value: u8) -> RelationshipLevel {
        RelationshipLevel::try_new(value).expect("fixture relationship level must be valid")
    }

    fn make_test_brief_fixture() -> BriefFixture {
        let registry = build_registry();
        let mut state = AppState::new(0xB21E_1933);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Executive Brief Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("player organization fixture should validate");
        designate_player_organization(&mut state, organization)
            .expect("player organization designation should validate");
        let recruiter = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Personnel Manager".to_owned(),
                organization: Some(organization),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::from([(CapabilityKind::Negotiation, rating(75))]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("recruiter fixture should validate");
        let candidate = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Independent Candidate".to_owned(),
                organization: None,
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("candidate fixture should validate");
        validate_set_relationship(
            &state,
            candidate,
            recruiter,
            RelationshipDimensions {
                trust: level(60),
                respect: level(65),
                fear: level(5),
                affection: level(30),
                dependence: level(10),
                resentment: level(0),
                debt: level(5),
            },
        )
        .expect("recruitment relationship fixture should validate")
        .commit(&mut state);
        let mandate = validate_assign_mandate(
            &registry,
            &state,
            MandateDraft {
                organization,
                manager: recruiter,
                scopes: BTreeSet::from([ResponsibilityScope::Function(
                    ResponsibilityFunction::Personnel,
                )]),
                standing_orders: BTreeMap::new(),
                budget: None,
            },
        )
        .expect("personnel mandate fixture should validate")
        .commit(&mut state)
        .expect("personnel mandate fixture should commit");
        BriefFixture {
            registry,
            state,
            organization,
            recruiter,
            candidate,
            mandate,
        }
    }

    fn request_recruitment_approval(fixture: &mut BriefFixture) -> DecisionRequestId {
        validate_request_recruitment_approval(
            &fixture.registry,
            &fixture.state,
            RecruitmentApprovalRequestDraft {
                authority: MandateAuthority {
                    mandate: fixture.mandate,
                    manager: fixture.recruiter,
                    scope: ResponsibilityScope::Function(ResponsibilityFunction::Personnel),
                },
                target_organization: fixture.organization,
                recruiter: fixture.recruiter,
                candidate: fixture.candidate,
                approach: RecruitmentApproach::PersonalAppeal,
                attention: AttentionClass::Exception,
                summary: "Personnel manager requests approval for a recruitment approach."
                    .to_owned(),
            },
        )
        .expect("approval request fixture should validate")
        .commit(&mut fixture.state)
        .expect("approval request fixture should commit")
        .decision
    }

    fn record_report(
        state: &mut AppState,
        recipient: OrganizationId,
        title: &str,
        entries: Vec<ReportEntry>,
    ) -> ReportId {
        validate_record_report(
            state,
            ReportDraft {
                recipient,
                kind: ReportKind::PoliceIntelligence,
                title: title.to_owned(),
                entries,
            },
        )
        .expect("source report fixture should validate")
        .commit(state)
        .expect("source report fixture should commit")
    }

    fn entry(attention: AttentionClass, summary: &str) -> ReportEntry {
        ReportEntry {
            attention,
            summary: summary.to_owned(),
            sources: Vec::new(),
            entities: BTreeSet::new(),
            decision: None,
        }
    }

    #[test]
    fn synthesis_prioritizes_pending_decisions_filters_routine_and_deduplicates_sources() {
        let mut fixture = make_test_brief_fixture();
        let decision = request_recruitment_approval(&mut fixture);
        record_report(
            &mut fixture.state,
            fixture.organization,
            "First source report",
            vec![
                entry(
                    AttentionClass::Routine,
                    "Routine collection completed normally.",
                ),
                entry(
                    AttentionClass::Notable,
                    "Detectives increased questioning near the docks.",
                ),
            ],
        );
        record_report(
            &mut fixture.state,
            fixture.organization,
            "Duplicate source report",
            vec![entry(
                AttentionClass::Notable,
                "Detectives increased questioning near the docks.",
            )],
        );
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));

        let plan = decide_executive_brief(&fixture.registry, &fixture.state, fixture.organization)
            .expect("daily brief should synthesize current executive information");
        assert_eq!(plan.entries().len(), 2);
        assert_eq!(plan.entries()[0].attention, AttentionClass::Exception);
        assert_eq!(plan.entries()[0].decision, Some(decision));
        assert!(plan.entries()[0]
            .entities
            .contains(&EntityRef::DecisionRequest(decision)));
        assert_eq!(plan.entries()[1].attention, AttentionClass::Notable);
        assert_eq!(
            plan.entries()[1].summary,
            "Detectives increased questioning near the docks."
        );

        let report = validate_executive_brief_plan(&fixture.state, plan)
            .expect("fresh executive brief should validate")
            .commit(&mut fixture.state)
            .expect("validated executive brief should commit");
        assert_eq!(
            fixture
                .state
                .reports()
                .get_report(report)
                .expect("executive brief should persist")
                .kind(),
            ReportKind::ExecutiveBrief
        );
        assert_eq!(
            decide_executive_brief(&fixture.registry, &fixture.state, fixture.organization)
                .expect_err("a second brief at the same boundary must be rejected"),
            ExecutiveBriefError::AlreadyGenerated { report }
        );
        validate_state(&fixture.state).expect("executive brief state should validate");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn stale_plan_rejects_pending_decision_changes_without_partial_mutation() {
        let mut fixture = make_test_brief_fixture();
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));
        let plan = decide_executive_brief(&fixture.registry, &fixture.state, fixture.organization)
            .expect("empty daily brief should initially plan against no pending decisions");
        let decision = request_recruitment_approval(&mut fixture);

        let error = match validate_executive_brief_plan(&fixture.state, plan) {
            Ok(_) => panic!("new pending executive work must stale the older synthesis plan"),
            Err(error) => error,
        };
        assert_eq!(error, ExecutiveBriefError::StalePendingDecisions);
        assert_eq!(
            fixture
                .state
                .decisions()
                .pending_for_recipient(fixture.organization)
                .map(DecisionRequestRecord::id)
                .collect::<Vec<_>>(),
            vec![decision]
        );
        assert!(fixture
            .state
            .reports()
            .latest_for_kind(fixture.organization, ReportKind::ExecutiveBrief)
            .is_none());
        validate_invariants(&fixture.state);
    }

    #[test]
    fn stale_plan_rejects_report_window_changes_without_partial_mutation() {
        let mut fixture = make_test_brief_fixture();
        record_report(
            &mut fixture.state,
            fixture.organization,
            "Initial report",
            vec![entry(AttentionClass::Notable, "Initial notable item.")],
        );
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));
        let plan = decide_executive_brief(&fixture.registry, &fixture.state, fixture.organization)
            .expect("brief plan should validate before a new source arrives");
        record_report(
            &mut fixture.state,
            fixture.organization,
            "Late report",
            vec![entry(AttentionClass::Notable, "Late notable item.")],
        );

        let error = match validate_executive_brief_plan(&fixture.state, plan) {
            Ok(_) => panic!("new source reports must stale the older synthesis plan"),
            Err(error) => error,
        };
        assert_eq!(error, ExecutiveBriefError::StaleReportWindow);
        assert!(fixture
            .state
            .reports()
            .latest_for_kind(fixture.organization, ReportKind::ExecutiveBrief)
            .is_none());
        validate_invariants(&fixture.state);
    }

    #[test]
    fn source_entry_limit_preserves_priority_and_discloses_overflow() {
        let mut fixture = make_test_brief_fixture();
        let mut entries = vec![
            entry(AttentionClass::Crisis, "Immediate crisis A."),
            entry(AttentionClass::Crisis, "Immediate crisis B."),
        ];
        entries.extend((0..8).map(|index| {
            entry(
                AttentionClass::Notable,
                &format!("Notable source item {index}."),
            )
        }));
        record_report(
            &mut fixture.state,
            fixture.organization,
            "Dense source report",
            entries,
        );
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));

        let plan = decide_executive_brief(&fixture.registry, &fixture.state, fixture.organization)
            .expect("dense source set should still produce a bounded brief");
        assert_eq!(plan.entries().len(), 9);
        assert_eq!(plan.entries()[0].attention, AttentionClass::Crisis);
        assert_eq!(plan.entries()[1].attention, AttentionClass::Crisis);
        assert!(plan.entries()[8].summary.contains("2 additional items"));
        validate_invariants(&fixture.state);
    }

    #[test]
    fn resolved_decision_report_is_not_resurfaced_as_current_executive_work() {
        let mut fixture = make_test_brief_fixture();
        let decision = request_recruitment_approval(&mut fixture);
        record_report(
            &mut fixture.state,
            fixture.organization,
            "Decision source report",
            vec![ReportEntry {
                attention: AttentionClass::Exception,
                summary: "Recruitment approval is waiting for leadership.".to_owned(),
                sources: Vec::new(),
                entities: BTreeSet::from([EntityRef::DecisionRequest(decision)]),
                decision: Some(decision),
            }],
        );
        validate_resolve_decision(
            &fixture.registry,
            &fixture.state,
            decision,
            fixture.organization,
            DecisionResponse::Reject,
        )
        .expect("decision rejection should validate")
        .commit(&mut fixture.state)
        .expect("decision rejection should commit");
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));

        let plan = decide_executive_brief(&fixture.registry, &fixture.state, fixture.organization)
            .expect("resolved decision history should not block the daily brief");
        assert_eq!(plan.entries().len(), 1);
        assert_eq!(plan.entries()[0].attention, AttentionClass::Routine);
        assert!(plan.entries()[0].decision.is_none());
        validate_invariants(&fixture.state);
    }

    #[test]
    fn next_brief_reads_only_reports_created_after_the_previous_brief() {
        let mut fixture = make_test_brief_fixture();
        record_report(
            &mut fixture.state,
            fixture.organization,
            "Day one source",
            vec![entry(AttentionClass::Notable, "Day one notable item.")],
        );
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));
        let first_plan =
            decide_executive_brief(&fixture.registry, &fixture.state, fixture.organization)
                .expect("first daily brief should plan");
        assert_eq!(first_plan.entries()[0].summary, "Day one notable item.");
        validate_executive_brief_plan(&fixture.state, first_plan)
            .expect("first daily brief should validate")
            .commit(&mut fixture.state)
            .expect("first daily brief should commit");

        record_report(
            &mut fixture.state,
            fixture.organization,
            "Day two source",
            vec![entry(AttentionClass::Notable, "Day two notable item.")],
        );
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));
        let second_plan =
            decide_executive_brief(&fixture.registry, &fixture.state, fixture.organization)
                .expect("second daily brief should plan from the prior brief cursor");
        assert_eq!(second_plan.entries().len(), 1);
        assert_eq!(second_plan.entries()[0].summary, "Day two notable item.");
        assert!(!second_plan
            .entries()
            .iter()
            .any(|entry| entry.summary == "Day one notable item."));
        validate_invariants(&fixture.state);
    }

    #[test]
    fn daily_tick_generation_is_deterministic_across_save_round_trip() {
        let mut fixture = make_test_brief_fixture();
        record_report(
            &mut fixture.state,
            fixture.organization,
            "Daily source report",
            vec![entry(
                AttentionClass::Notable,
                "A source report is waiting for the next executive cycle.",
            )],
        );
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_439));
        let envelope = build_save(&fixture.registry, &fixture.state)
            .expect("pre-brief state should build a valid save");
        let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
        let decoded: SaveEnvelope =
            bincode::deserialize(&bytes).expect("save envelope should deserialize");
        let mut restored =
            restore_save(&fixture.registry, decoded).expect("pre-brief save should restore");

        let original = run_tick(&fixture.registry, &mut fixture.state);
        let continued = run_tick(&fixture.registry, &mut restored);
        assert_eq!(original, continued);
        let report = original
            .executive_brief
            .expect("daily boundary should generate an executive brief");
        assert_eq!(original.now, SimTime::from_minutes(1_440));
        assert_eq!(
            fixture
                .state
                .reports()
                .get_report(report)
                .expect("original executive brief should persist")
                .entries()[0]
                .summary,
            restored
                .reports()
                .get_report(report)
                .expect("restored executive brief should persist")
                .entries()[0]
                .summary
        );
        validate_state(&fixture.state).expect("original post-brief state should validate");
        validate_state(&restored).expect("restored post-brief state should validate");
        validate_invariants(&fixture.state);
        validate_invariants(&restored);
    }
}
