//! Persisted player-facing reports; specialized synthesis modules build artifacts that `report_system` validates before insertion.

pub mod executive_brief;
pub mod organization_financial_report;
pub mod report_system;

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{DecisionRequestId, IdKeyedBounds, InformationId, OrganizationId, ReportId};
use crate::core::time::SimTime;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Types of player-facing reports the simulation can generate. Only variants
/// actually produced by systems are represented here; unused slots were deleted
/// to preserve exhaustive-match discipline per ARCHITECTURE.md.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReportKind {
    ExecutiveBrief,
    Financial,
    Legal,
    AfterAction,
    Opportunity,
    /// The organization's own street standing shifting after its own operations.
    Standing,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReportEntry {
    pub attention: AttentionClass,
    pub summary: String,
    pub sources: Vec<InformationId>,
    pub entities: BTreeSet<EntityRef>,
    pub decision: Option<DecisionRequestId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReportRecord {
    id: ReportId,
    recipient: OrganizationId,
    kind: ReportKind,
    title: String,
    generated_at: SimTime,
    entries: Vec<ReportEntry>,
}

impl ReportRecord {
    pub fn id(&self) -> ReportId {
        self.id
    }
    pub fn recipient(&self) -> OrganizationId {
        self.recipient
    }
    pub fn kind(&self) -> ReportKind {
        self.kind
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn generated_at(&self) -> SimTime {
        self.generated_at
    }
    pub fn entries(&self) -> &[ReportEntry] {
        &self.entries
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReportState {
    records: BTreeMap<ReportId, ReportRecord>,
    #[serde(skip)]
    by_recipient: BTreeMap<OrganizationId, BTreeSet<ReportId>>,
}

impl ReportState {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) fn rebuild_derived_indexes(&mut self) {
        self.by_recipient.clear();
        for report in self.records.values() {
            self.by_recipient
                .entry(report.recipient())
                .or_default()
                .insert(report.id());
        }
    }
    pub fn get_report(&self, id: ReportId) -> Option<&ReportRecord> {
        self.records.get(&id)
    }
    pub fn reports_for(&self, recipient: OrganizationId) -> impl Iterator<Item = &ReportRecord> {
        self.by_recipient
            .get(&recipient)
            .into_iter()
            .flatten()
            .map(|id| {
                self.records
                    .get(id)
                    .expect("report recipient index must reference a report")
            })
    }
    pub fn reports_for_after(
        &self,
        recipient: OrganizationId,
        after: Option<ReportId>,
    ) -> impl Iterator<Item = &ReportRecord> {
        use std::ops::Bound::{Excluded, Unbounded};

        let lower = after.map_or(Unbounded, Excluded);
        self.by_recipient
            .get(&recipient)
            .into_iter()
            .flat_map(move |ids| ids.range((lower, Unbounded)))
            .map(|id| {
                self.records
                    .get(id)
                    .expect("report recipient index must reference a report")
            })
    }
    pub fn latest_for_kind(
        &self,
        recipient: OrganizationId,
        kind: ReportKind,
    ) -> Option<&ReportRecord> {
        self.by_recipient.get(&recipient).and_then(|ids| {
            ids.iter()
                .rev()
                .map(|id| {
                    self.records
                        .get(id)
                        .expect("report recipient index must reference a report")
                })
                .find(|report| report.kind() == kind)
        })
    }
    pub(crate) fn latest_for_recipient(&self, recipient: OrganizationId) -> Option<&ReportRecord> {
        self.by_recipient
            .get(&recipient)
            .and_then(|ids| ids.last())
            .map(|id| {
                self.records
                    .get(id)
                    .expect("report recipient index must reference a report")
            })
    }
    pub(crate) fn reports(&self) -> impl Iterator<Item = &ReportRecord> {
        self.records.values()
    }
    pub(crate) fn report_id_bounds(&self) -> Option<(u32, u32)> {
        self.records.id_bounds()
    }
    pub(crate) fn insert(&mut self, report: ReportRecord) {
        self.by_recipient
            .entry(report.recipient())
            .or_default()
            .insert(report.id());
        let previous = self.records.insert(report.id(), report);
        debug_assert!(
            previous.is_none(),
            "Index Uniqueness: duplicate report ID inserted"
        );
    }
    pub(crate) fn has_consistent_indexes(&self) -> bool {
        let mut previous_generated_at = None;
        for (stored_id, report) in &self.records {
            if *stored_id != report.id() {
                return false;
            }
            if previous_generated_at.is_some_and(|previous| report.generated_at() < previous) {
                return false;
            }
            if !self
                .by_recipient
                .get(&report.recipient())
                .is_some_and(|ids| ids.contains(&report.id()))
            {
                return false;
            }
            // Report IDs are allocated globally and every canonical report is stamped with the
            // current simulation time. Prove that persisted report chronology never rewinds even
            // across different recipients; recipient-local latest/cursor queries then inherit the
            // same ordering without maintaining a weaker duplicate chronology check.
            previous_generated_at = Some(report.generated_at());
        }
        for (recipient, ids) in &self.by_recipient {
            for id in ids {
                let Some(report) = self.records.get(id) else {
                    return false;
                };
                if report.recipient() != *recipient {
                    return false;
                }
            }
        }
        true
    }
}

pub struct ReportDraft {
    pub recipient: OrganizationId,
    pub kind: ReportKind,
    pub title: String,
    pub entries: Vec<ReportEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::attention::AttentionClass;
    use crate::core::entity::EntityRef;
    use crate::core::invariants::{StateValidationError, validate_state};
    use crate::core::state::AppState;
    use crate::core::time::SimDuration;
    use crate::decisions::RecruitmentApprovalRequestDraft;
    use crate::decisions::decision_system::validate_request_recruitment_approval;
    use crate::delegation::delegation_system::validate_assign_mandate;
    use crate::delegation::{
        MandateAuthority, MandateDraft, ResponsibilityFunction, ResponsibilityScope,
    };
    use crate::intelligence::intelligence_system::validate_record_information;
    use crate::intelligence::{
        InformationDraft, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
        Specificity,
    };
    use crate::recruitment::RecruitmentApproach;
    use crate::social::relationship_system::validate_set_relationship;
    use crate::social::{RelationshipDimensions, RelationshipLevel};
    use crate::world::world_system::{insert_character, insert_organization};
    use crate::world::{
        ApprovalPolicy, AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind,
        PolicyKind, PolicySetting,
    };
    use std::collections::{BTreeMap, BTreeSet};

    fn report(id: u32, recipient: OrganizationId, generated_at: u64) -> ReportRecord {
        ReportRecord {
            id: ReportId::from_raw(id),
            recipient,
            kind: ReportKind::Financial,
            title: format!("Report {id}"),
            generated_at: SimTime::from_minutes(generated_at),
            entries: Vec::new(),
        }
    }

    #[test]
    fn recipient_index_rejects_report_time_rewind_by_id() {
        let recipient = OrganizationId::from_raw(1);
        let mut state = ReportState::new();
        state.insert(report(1, recipient, 10));
        state.insert(report(2, recipient, 10));
        state.insert(report(3, recipient, 11));
        assert!(
            state.has_consistent_indexes(),
            "equal-minute reports and later IDs must preserve canonical chronology"
        );

        state
            .records
            .get_mut(&ReportId::from_raw(3))
            .expect("third report should exist")
            .generated_at = SimTime::from_minutes(9);
        assert!(
            !state.has_consistent_indexes(),
            "an ID-ordered report cursor must reject a persisted timestamp rewind"
        );
    }

    #[test]
    fn report_index_rejects_time_rewind_across_recipients() {
        let first_recipient = OrganizationId::from_raw(1);
        let second_recipient = OrganizationId::from_raw(2);
        let mut state = ReportState::new();
        state.insert(report(1, first_recipient, 10));
        state.insert(report(2, second_recipient, 9));

        assert!(
            !state.has_consistent_indexes(),
            "global report allocation order must not rewind merely because the recipient changed"
        );
    }

    #[test]
    fn state_validation_rejects_report_citing_information_recorded_later() {
        let registry = build_registry();
        let mut state = AppState::new(0x00A1_1D17);
        let recipient = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Chronology Recipient".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("report recipient should validate");
        state.advance_clock(SimDuration::from_minutes(10));
        let information = validate_record_information(
            &state,
            InformationDraft {
                holder: KnowledgeHolder::Organization(recipient),
                source_kind: InformationSourceKind::DirectObservation,
                topic: InformationTopic::General,
                source_entity: None,
                subject: EntityRef::Organization(recipient),
                observed_at: state.now(),
                reliability: Reliability::DirectAccess,
                specificity: Specificity::Precise,
                summary: "This fact did not exist at minute five.".to_owned(),
            },
        )
        .expect("information should validate")
        .commit(&mut state)
        .expect("information should commit");
        let report = state
            .ids
            .next_report()
            .expect("report id should be available");
        state.reports.insert(ReportRecord {
            id: report,
            recipient,
            kind: ReportKind::Financial,
            title: "Impossible early report".to_owned(),
            generated_at: SimTime::from_minutes(5),
            entries: vec![ReportEntry {
                attention: AttentionClass::Notable,
                summary: "The report improperly cites later information.".to_owned(),
                sources: vec![information],
                entities: BTreeSet::from([EntityRef::Organization(recipient)]),
                decision: None,
            }],
        });

        assert_eq!(
            validate_state(&state),
            Err(StateValidationError::ReportInformationUnavailable {
                report,
                information,
            }),
            "restore validation must reject information that was unavailable when the report was generated"
        );
    }

    #[test]
    fn state_validation_rejects_report_citing_decision_requested_later() {
        let registry = build_registry();
        let mut state = AppState::new(0x0DEC_1510);
        let recipient = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Decision Chronology Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("decision recipient should validate");
        let manager = insert_character(
            &mut state,
            CharacterDraft {
                name: "Decision Chronology Manager".to_owned(),
                organization: Some(recipient),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("manager should validate");
        let candidate = insert_character(
            &mut state,
            CharacterDraft {
                name: "Decision Chronology Candidate".to_owned(),
                organization: None,
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("candidate should validate");
        let level = RelationshipLevel::try_new(50).expect("fixture relationship level is valid");
        validate_set_relationship(
            &state,
            candidate,
            manager,
            RelationshipDimensions {
                trust: level,
                respect: level,
                fear: RelationshipLevel::try_new(0).expect("zero relationship level is valid"),
                affection: level,
                dependence: RelationshipLevel::try_new(0)
                    .expect("zero relationship level is valid"),
                resentment: RelationshipLevel::try_new(0)
                    .expect("zero relationship level is valid"),
                debt: RelationshipLevel::try_new(0).expect("zero relationship level is valid"),
            },
        )
        .expect("recruitment relationship should validate")
        .commit(&mut state)
        .expect("recruitment relationship should commit");
        let scope = ResponsibilityScope::Function(ResponsibilityFunction::Personnel);
        let mandate = validate_assign_mandate(
            &state,
            MandateDraft {
                organization: recipient,
                manager,
                scopes: BTreeSet::from([scope]),
                standing_orders: BTreeMap::from([(
                    PolicyKind::IndependentRecruitment,
                    PolicySetting::IndependentRecruitment(ApprovalPolicy::RequireApproval),
                )]),
                budget: None,
            },
        )
        .expect("personnel mandate should validate")
        .commit(&mut state)
        .expect("personnel mandate should commit");

        state.advance_clock(SimDuration::from_minutes(10));
        let decision = validate_request_recruitment_approval(
            &registry,
            &state,
            RecruitmentApprovalRequestDraft {
                authority: MandateAuthority {
                    mandate,
                    manager,
                    scope,
                },
                target_organization: recipient,
                recruiter: manager,
                candidate,
                approach: RecruitmentApproach::PersonalAppeal,
                attention: AttentionClass::Exception,
                summary: "Approve recruitment outreach.".to_owned(),
            },
        )
        .expect("recruitment approval should validate")
        .commit(&mut state)
        .expect("recruitment approval should commit")
        .decision;
        let report = state
            .ids
            .next_report()
            .expect("report id should be available");
        state.reports.insert(ReportRecord {
            id: report,
            recipient,
            kind: ReportKind::Financial,
            title: "Impossible decision report".to_owned(),
            generated_at: SimTime::from_minutes(5),
            entries: vec![ReportEntry {
                attention: AttentionClass::Exception,
                summary: "This report improperly cites a later decision.".to_owned(),
                sources: Vec::new(),
                entities: BTreeSet::from([EntityRef::DecisionRequest(decision)]),
                decision: Some(decision),
            }],
        });

        assert_eq!(
            validate_state(&state),
            Err(StateValidationError::ReportDecisionUnavailableAtGeneration { report, decision }),
            "restore validation must reject a decision that did not exist when the report was generated"
        );
    }
}
