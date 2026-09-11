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
    fn insert(&mut self, report: ReportRecord) {
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
    use crate::core::invariants::StateValidationError;
    use crate::core::persistence::{SaveEnvelope, build_save, restore_save};
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
    use crate::reports::report_system::validate_record_report;
    use crate::social::relationship_system::validate_set_relationship;
    use crate::social::{RelationshipDimensions, RelationshipLevel};
    use crate::world::world_system::{insert_character, insert_organization};
    use crate::world::{
        ApprovalPolicy, AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind,
        PolicyKind, PolicySetting,
    };
    use serde::Serialize;
    use std::collections::{BTreeMap, BTreeSet};

    #[derive(Clone, Serialize)]
    struct ReportRecordWire {
        id: ReportId,
        recipient: OrganizationId,
        kind: ReportKind,
        title: String,
        generated_at: SimTime,
        entries: Vec<ReportEntry>,
    }

    fn report_wire(record: &ReportRecord) -> ReportRecordWire {
        ReportRecordWire {
            id: record.id(),
            recipient: record.recipient(),
            kind: record.kind(),
            title: record.title().to_owned(),
            generated_at: record.generated_at(),
            entries: record.entries().to_vec(),
        }
    }

    fn replace_serialized_report(
        envelope: SaveEnvelope,
        original: &ReportRecord,
        replacement: &ReportRecordWire,
    ) -> SaveEnvelope {
        let original_bytes = bincode::serialize(original).expect("report should serialize");
        let mirror = report_wire(original);
        assert_eq!(
            bincode::serialize(&mirror).expect("report mirror should serialize"),
            original_bytes,
            "wire mirror must match production report persistence layout exactly"
        );
        let replacement_bytes =
            bincode::serialize(replacement).expect("replacement report should serialize");
        assert_eq!(
            replacement_bytes.len(),
            original_bytes.len(),
            "same-layout report corruption must preserve serialized length"
        );
        let mut envelope_bytes =
            bincode::serialize(&envelope).expect("save envelope should serialize");
        let matches: Vec<_> = envelope_bytes
            .windows(original_bytes.len())
            .enumerate()
            .filter_map(|(index, window)| (window == original_bytes).then_some(index))
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "serialized report must appear exactly once in the save envelope"
        );
        let start = matches[0];
        envelope_bytes[start..start + replacement_bytes.len()].copy_from_slice(&replacement_bytes);
        bincode::deserialize(&envelope_bytes)
            .expect("same-layout report corruption must remain decodable")
    }

    fn record_empty_report(
        state: &mut AppState,
        recipient: OrganizationId,
        title: &str,
    ) -> ReportId {
        validate_record_report(
            state,
            ReportDraft {
                recipient,
                kind: ReportKind::Financial,
                title: title.to_owned(),
                entries: Vec::new(),
            },
        )
        .expect("fixture report should validate")
        .commit(state)
        .expect("fixture report should commit")
    }

    #[test]
    fn restore_rejects_report_time_rewind_after_equal_minute_reports() {
        let registry = build_registry();
        let mut state = AppState::new(0x0A11_CE01);
        let recipient = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Chronology Recipient".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("recipient should validate");
        state.advance_clock(SimDuration::from_minutes(10));
        record_empty_report(&mut state, recipient, "First report");
        record_empty_report(&mut state, recipient, "Second report");
        state.advance_clock(SimDuration::ONE_MINUTE);
        let third = record_empty_report(&mut state, recipient, "Third report");
        let original = state
            .reports()
            .get_report(third)
            .expect("third report should persist");
        let mut corrupted = report_wire(original);
        corrupted.generated_at = SimTime::from_minutes(9);
        let envelope = replace_serialized_report(
            build_save(&registry, &state).expect("valid report chronology should save"),
            original,
            &corrupted,
        );

        assert!(matches!(
            restore_save(&registry, envelope),
            Err(crate::core::persistence::LoadError::InvalidState(
                StateValidationError::IndexInconsistency {
                    subsystem: "reports"
                }
            ))
        ));
    }

    #[test]
    fn restore_rejects_report_time_rewind_across_recipients() {
        let registry = build_registry();
        let mut state = AppState::new(0x0A11_CE02);
        let first_recipient = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "First Chronology Recipient".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("first recipient should validate");
        let second_recipient = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Second Chronology Recipient".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("second recipient should validate");
        state.advance_clock(SimDuration::from_minutes(10));
        record_empty_report(&mut state, first_recipient, "First recipient report");
        state.advance_clock(SimDuration::ONE_MINUTE);
        let second = record_empty_report(&mut state, second_recipient, "Second recipient report");
        let original = state
            .reports()
            .get_report(second)
            .expect("second recipient report should persist");
        let mut corrupted = report_wire(original);
        corrupted.generated_at = SimTime::from_minutes(9);
        let envelope = replace_serialized_report(
            build_save(&registry, &state).expect("valid cross-recipient chronology should save"),
            original,
            &corrupted,
        );

        assert!(matches!(
            restore_save(&registry, envelope),
            Err(crate::core::persistence::LoadError::InvalidState(
                StateValidationError::IndexInconsistency {
                    subsystem: "reports"
                }
            ))
        ));
    }

    #[test]
    fn restore_rejects_report_citing_information_recorded_later() {
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
        let report = validate_record_report(
            &state,
            ReportDraft {
                recipient,
                kind: ReportKind::Financial,
                title: "Information chronology report".to_owned(),
                entries: vec![ReportEntry {
                    attention: AttentionClass::Notable,
                    summary: "The report cites current information.".to_owned(),
                    sources: vec![information],
                    entities: BTreeSet::from([EntityRef::Organization(recipient)]),
                    decision: None,
                }],
            },
        )
        .expect("current information should support a report")
        .commit(&mut state)
        .expect("current report should commit");
        let original = state
            .reports()
            .get_report(report)
            .expect("report should persist");
        let mut corrupted = report_wire(original);
        corrupted.generated_at = SimTime::from_minutes(5);
        let envelope = replace_serialized_report(
            build_save(&registry, &state).expect("valid information chronology should save"),
            original,
            &corrupted,
        );

        assert!(matches!(
            restore_save(&registry, envelope),
            Err(crate::core::persistence::LoadError::InvalidState(
                StateValidationError::ReportInformationUnavailable {
                    report: invalid_report,
                    information: invalid_information,
                }
            )) if invalid_report == report && invalid_information == information
        ));
    }

    #[test]
    fn restore_rejects_report_citing_decision_requested_later() {
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
        let report = validate_record_report(
            &state,
            ReportDraft {
                recipient,
                kind: ReportKind::Financial,
                title: "Decision chronology report".to_owned(),
                entries: vec![ReportEntry {
                    attention: AttentionClass::Exception,
                    summary: "This report cites the current decision.".to_owned(),
                    sources: Vec::new(),
                    entities: BTreeSet::from([EntityRef::DecisionRequest(decision)]),
                    decision: Some(decision),
                }],
            },
        )
        .expect("current decision should support a report")
        .commit(&mut state)
        .expect("current decision report should commit");
        let original = state
            .reports()
            .get_report(report)
            .expect("decision report should persist");
        let mut corrupted = report_wire(original);
        corrupted.generated_at = SimTime::from_minutes(5);
        let envelope = replace_serialized_report(
            build_save(&registry, &state).expect("valid decision chronology should save"),
            original,
            &corrupted,
        );

        assert!(matches!(
            restore_save(&registry, envelope),
            Err(crate::core::persistence::LoadError::InvalidState(
                StateValidationError::ReportDecisionUnavailableAtGeneration {
                    report: invalid_report,
                    decision: invalid_decision,
                }
            )) if invalid_report == report && invalid_decision == decision
        ));
    }
}
