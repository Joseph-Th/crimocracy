//! Persistence round-trip, corruption, and current-schema rejection tests.

use super::*;
use crate::build_registry;
use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{CharacterId, DecisionRequestId, HistoryEventId, InformationId};
use crate::decisions::decision_system::validate_request_recruitment_approval;
use crate::decisions::{
    DecisionCancellation, DecisionContext, DecisionRequestRecord, DecisionResolution,
    DecisionResponse, DecisionState, DecisionStatus, RecruitmentApprovalRequestDraft,
};
use crate::delegation::delegation_system::validate_assign_mandate;
use crate::delegation::{
    MandateAuthority, MandateDraft, ResponsibilityFunction, ResponsibilityScope,
};
use crate::finance::finance_system::{insert_account, validate_record_transaction};
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crate::history::history_system::validate_record_event;
use crate::history::{HistoryEventDraft, HistoryEventKind, HistoryState};
use crate::intelligence::intelligence_system::validate_record_information;
use crate::intelligence::{
    InformationDraft, InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability,
    Specificity,
};
use crate::intelligence::{InformationSignal, IntelligenceState};
use crate::recruitment::RecruitmentApproach;
use crate::reports::report_system::validate_record_report;
use crate::reports::{ReportDraft, ReportEntry, ReportKind};
use crate::social::relationship_system::validate_set_relationship;
use crate::social::{RelationshipDimensions, RelationshipLevel, SocialState};
use crate::world::world_system::{insert_character, insert_organization};
use crate::world::{
    ApprovalPolicy, AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind, PolicyKind,
    PolicySetting,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

struct PersistenceFixture {
    registry: Registry,
    state: AppState,
    organization: crate::core::id::OrganizationId,
    first: CharacterId,
    second: CharacterId,
}

#[derive(Clone, Debug, Serialize)]
enum DecisionLifecycleWire {
    Pending,
    Resolved(DecisionResolution),
    Cancelled(DecisionCancellation),
}

#[derive(Clone, Debug, Serialize)]
struct DecisionRequestRecordWire {
    id: DecisionRequestId,
    recipient: crate::core::id::OrganizationId,
    requester: CharacterId,
    context: DecisionContext,
    attention: AttentionClass,
    summary: String,
    requested_at: crate::core::time::SimTime,
    options: BTreeSet<DecisionResponse>,
    lifecycle: DecisionLifecycleWire,
    version: u32,
}

#[derive(Clone, Debug, Serialize)]
struct DecisionStateWire {
    records: BTreeMap<DecisionRequestId, DecisionRequestRecordWire>,
}

fn decision_record_wire(record: &DecisionRequestRecord) -> DecisionRequestRecordWire {
    let lifecycle = match record.status() {
        DecisionStatus::Pending => DecisionLifecycleWire::Pending,
        DecisionStatus::Resolved => DecisionLifecycleWire::Resolved(
            record
                .resolution()
                .expect("resolved decision must carry a resolution"),
        ),
        DecisionStatus::Cancelled => DecisionLifecycleWire::Cancelled(
            record
                .cancellation()
                .expect("cancelled decision must carry a cancellation"),
        ),
    };
    DecisionRequestRecordWire {
        id: record.id(),
        recipient: record.recipient(),
        requester: record.requester(),
        context: record.context(),
        attention: record.attention(),
        summary: record.summary().to_owned(),
        requested_at: record.requested_at(),
        options: record.options().clone(),
        lifecycle,
        version: record.version(),
    }
}

fn decision_state_wire(state: &DecisionState) -> DecisionStateWire {
    DecisionStateWire {
        records: state
            .decisions()
            .map(|record| (record.id(), decision_record_wire(record)))
            .collect(),
    }
}

fn fixture() -> PersistenceFixture {
    let registry = build_registry();
    let mut state = AppState::new(0x5A9E_1933);
    let organization = insert_organization(
        &registry,
        &mut state,
        OrganizationDraft {
            name: "Persistence Index Test Organization".to_owned(),
            kind: OrganizationKind::Criminal,
        },
    )
    .expect("organization fixture should validate");
    let insert_fixture_character = |state: &mut AppState, name: &str| {
        insert_character(
            state,
            CharacterDraft {
                name: name.to_owned(),
                organization: Some(organization),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("character fixture should validate")
    };
    let first = insert_fixture_character(&mut state, "Persistence Index Test Character");
    let second = insert_fixture_character(&mut state, "Persistence Peer Character");
    PersistenceFixture {
        registry,
        state,
        organization,
        first,
        second,
    }
}

fn blank_serialized_text(
    envelope: SaveEnvelope,
    text: &str,
    expected_occurrences: usize,
) -> SaveEnvelope {
    assert!(!text.is_empty());
    assert!(text.is_ascii(), "fixture text must be byte-stable ASCII");
    let needle = text.as_bytes();
    let mut bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
    let mut cursor = 0_usize;
    let mut replaced = 0_usize;
    while cursor + needle.len() <= bytes.len() {
        let Some(relative) = bytes[cursor..]
            .windows(needle.len())
            .position(|window| window == needle)
        else {
            break;
        };
        let start = cursor + relative;
        bytes[start..start + needle.len()].fill(b' ');
        cursor = start + needle.len();
        replaced += 1;
    }
    assert_eq!(
        replaced, expected_occurrences,
        "test must blank exactly the intended persisted text copies"
    );
    bincode::deserialize(&bytes).expect("equal-length text corruption must remain decodable")
}

fn replace_serialized_substate<T: Serialize, U: Serialize>(
    envelope: SaveEnvelope,
    original: &T,
    replacement: &U,
) -> SaveEnvelope {
    let original_bytes = bincode::serialize(original).expect("substate should serialize");
    let replacement_bytes =
        bincode::serialize(replacement).expect("replacement substate should serialize");
    assert_eq!(
        replacement_bytes.len(),
        original_bytes.len(),
        "same-layout corruption must retain the serialized substate length"
    );
    let mut envelope_bytes = bincode::serialize(&envelope).expect("save should serialize");
    let matches: Vec<_> = envelope_bytes
        .windows(original_bytes.len())
        .enumerate()
        .filter_map(|(index, window)| (window == original_bytes).then_some(index))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "serialized substate must appear exactly once in the save envelope"
    );
    let start = matches[0];
    envelope_bytes[start..start + replacement_bytes.len()].copy_from_slice(&replacement_bytes);
    bincode::deserialize(&envelope_bytes)
        .expect("same-length substate corruption must remain decodable")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct RelationshipKeyWire {
    from: CharacterId,
    to: CharacterId,
}

#[derive(Clone, Copy, Debug, Serialize)]
struct RelationshipRecordWire {
    from: CharacterId,
    to: CharacterId,
    dimensions: RelationshipDimensions,
    version: u32,
}

#[derive(Clone, Debug, Serialize)]
struct SocialStateWire {
    relationships: BTreeMap<RelationshipKeyWire, RelationshipRecordWire>,
}

fn social_state_wire(state: &SocialState) -> SocialStateWire {
    SocialStateWire {
        relationships: state
            .relationships()
            .map(|record| {
                (
                    RelationshipKeyWire {
                        from: record.from(),
                        to: record.to(),
                    },
                    RelationshipRecordWire {
                        from: record.from(),
                        to: record.to(),
                        dimensions: record.dimensions(),
                        version: record.version(),
                    },
                )
            })
            .collect(),
    }
}

#[derive(Clone, Debug, Serialize)]
struct HistoryEventRecordWire {
    id: HistoryEventId,
    occurred_at: crate::core::time::SimTime,
    kind: HistoryEventKind,
    summary: String,
    entities: BTreeSet<EntityRef>,
}

#[derive(Clone, Debug, Serialize)]
struct HistoryStateWire {
    records: BTreeMap<HistoryEventId, HistoryEventRecordWire>,
}

fn history_state_wire(state: &HistoryState) -> HistoryStateWire {
    HistoryStateWire {
        records: state
            .events()
            .map(|event| {
                (
                    event.id(),
                    HistoryEventRecordWire {
                        id: event.id(),
                        occurred_at: event.occurred_at(),
                        kind: event.kind(),
                        summary: event.summary().to_owned(),
                        entities: event.entities().clone(),
                    },
                )
            })
            .collect(),
    }
}

#[derive(Clone, Debug, Serialize)]
struct InformationSourceWire {
    holder: KnowledgeHolder,
    source_kind: InformationSourceKind,
}

#[derive(Clone, Debug, Serialize)]
struct InformationSubjectWire {
    topic: InformationTopic,
    source_entity: Option<EntityRef>,
    subject: EntityRef,
}

#[derive(Clone, Debug, Serialize)]
struct InformationChronologyWire {
    observed_at: crate::core::time::SimTime,
    recorded_at: crate::core::time::SimTime,
}

#[derive(Clone, Debug, Serialize)]
struct InformationAssessmentWire {
    reliability: Reliability,
    specificity: Specificity,
    signal: Option<InformationSignal>,
    derived_from: BTreeSet<InformationId>,
    summary: String,
}

#[derive(Clone, Debug, Serialize)]
struct InformationRecordWire {
    id: InformationId,
    source: InformationSourceWire,
    subject: InformationSubjectWire,
    chronology: InformationChronologyWire,
    assessment: InformationAssessmentWire,
}

#[derive(Clone, Debug, Serialize)]
struct IntelligenceStateWire {
    records: BTreeMap<InformationId, InformationRecordWire>,
}

fn intelligence_state_wire(state: &IntelligenceState) -> IntelligenceStateWire {
    IntelligenceStateWire {
        records: state
            .information()
            .map(|record| {
                (
                    record.id(),
                    InformationRecordWire {
                        id: record.id(),
                        source: InformationSourceWire {
                            holder: record.holder(),
                            source_kind: record.source_kind(),
                        },
                        subject: InformationSubjectWire {
                            topic: record.topic(),
                            source_entity: record.source_entity(),
                            subject: record.subject(),
                        },
                        chronology: InformationChronologyWire {
                            observed_at: record.observed_at(),
                            recorded_at: record.recorded_at(),
                        },
                        assessment: InformationAssessmentWire {
                            reliability: record.reliability(),
                            specificity: record.specificity(),
                            signal: record.signal().cloned(),
                            derived_from: record.derived_from().clone(),
                            summary: record.summary().to_owned(),
                        },
                    },
                )
            })
            .collect(),
    }
}

#[test]
fn save_bytes_omit_derived_indexes_and_restore_rebuilds_them() {
    let PersistenceFixture {
        registry,
        state,
        organization,
        first,
        second,
    } = fixture();
    assert_eq!(
        state
            .world()
            .characters_in_organization(organization)
            .map(|record| record.id())
            .collect::<Vec<_>>(),
        vec![first, second]
    );

    let envelope = build_save(&registry, &state).expect("valid state should save");
    let bytes = bincode::serialize(&envelope).expect("save should serialize");
    let decoded: SaveEnvelope = bincode::deserialize(&bytes).expect("save should decode");
    assert_eq!(
        decoded
            .state
            .world()
            .get_character(first)
            .map(|record| record.id()),
        Some(first),
        "authoritative records must persist"
    );
    assert_eq!(
        decoded
            .state
            .world()
            .characters_in_organization(organization)
            .count(),
        0,
        "derived organization membership index must not be serialized"
    );

    let restored = restore_save(&registry, decoded).expect("restore should rebuild indexes");
    assert_eq!(
        restored
            .world()
            .characters_in_organization(organization)
            .map(|record| record.id())
            .collect::<Vec<_>>(),
        vec![first, second]
    );
}

#[test]
fn restore_rejects_authoritative_map_key_and_embedded_id_divergence() {
    let mut fixture = fixture();
    let record_event = |fixture: &mut PersistenceFixture, summary: &str| {
        validate_record_event(
            &fixture.state,
            HistoryEventDraft {
                kind: HistoryEventKind::Recruitment,
                summary: summary.to_owned(),
                entities: BTreeSet::from([EntityRef::Organization(fixture.organization)]),
            },
        )
        .expect("history identity fixture should validate")
        .commit(&mut fixture.state)
        .expect("history identity fixture should commit")
    };
    let first = record_event(&mut fixture, "First identity event");
    let second = record_event(&mut fixture, "Second identity event");
    let mut replacement = history_state_wire(fixture.state.history());
    replacement
        .records
        .get_mut(&first)
        .expect("first history record should exist")
        .id = second;
    replacement
        .records
        .get_mut(&second)
        .expect("second history record should exist")
        .id = first;

    let error = restore_save(
        &fixture.registry,
        replace_serialized_substate(
            build_save(&fixture.registry, &fixture.state)
                .expect("valid identity state should save before corruption"),
            fixture.state.history(),
            &replacement,
        ),
    )
    .expect_err("map keys and embedded record IDs must agree at restore");
    assert_eq!(
        error,
        LoadError::InvalidState(StateValidationError::IndexInconsistency {
            subsystem: "history",
        })
    );
}

#[test]
fn restore_rejects_history_chronology_that_rewinds_in_id_order() {
    let mut fixture = fixture();
    fixture
        .state
        .advance_clock(crate::core::time::SimDuration::from_minutes(5));
    let first = validate_record_event(
        &fixture.state,
        HistoryEventDraft {
            kind: HistoryEventKind::Recruitment,
            summary: "First chronological history event".to_owned(),
            entities: BTreeSet::from([EntityRef::Organization(fixture.organization)]),
        },
    )
    .expect("first chronological event should validate")
    .commit(&mut fixture.state)
    .expect("first chronological event should commit");

    fixture
        .state
        .advance_clock(crate::core::time::SimDuration::from_minutes(5));
    let second = validate_record_event(
        &fixture.state,
        HistoryEventDraft {
            kind: HistoryEventKind::Recruitment,
            summary: "Second chronological history event".to_owned(),
            entities: BTreeSet::from([EntityRef::Organization(fixture.organization)]),
        },
    )
    .expect("second chronological event should validate")
    .commit(&mut fixture.state)
    .expect("second chronological event should commit");

    let mut replacement = history_state_wire(fixture.state.history());
    assert!(
        replacement.records[&first].occurred_at < replacement.records[&second].occurred_at,
        "fixture history must begin in chronological ID order"
    );
    replacement
        .records
        .get_mut(&second)
        .expect("second history record should exist")
        .occurred_at = crate::core::time::SimTime::from_minutes(4);

    let error = restore_save(
        &fixture.registry,
        replace_serialized_substate(
            build_save(&fixture.registry, &fixture.state)
                .expect("valid chronological state should save before corruption"),
            fixture.state.history(),
            &replacement,
        ),
    )
    .expect_err("history time may not rewind as monotone IDs advance");
    assert_eq!(
        error,
        LoadError::InvalidState(StateValidationError::IndexInconsistency {
            subsystem: "history",
        })
    );
}

#[test]
fn restore_rejects_decision_request_time_that_rewinds_in_id_order() {
    let mut fixture = fixture();
    let make_candidate = |state: &mut AppState, name: &str| {
        insert_character(
            state,
            CharacterDraft {
                name: name.to_owned(),
                organization: None,
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("decision chronology candidate should validate")
    };
    let first_candidate = make_candidate(&mut fixture.state, "First Decision Candidate");
    let second_candidate = make_candidate(&mut fixture.state, "Second Decision Candidate");
    let relationship_level =
        RelationshipLevel::try_new(50).expect("fixture relationship level should validate");
    let neutral_level =
        RelationshipLevel::try_new(0).expect("fixture neutral relationship level should validate");
    for candidate in [first_candidate, second_candidate] {
        validate_set_relationship(
            &fixture.state,
            candidate,
            fixture.first,
            RelationshipDimensions {
                trust: relationship_level,
                respect: relationship_level,
                fear: neutral_level,
                affection: relationship_level,
                dependence: neutral_level,
                resentment: neutral_level,
                debt: neutral_level,
            },
        )
        .expect("decision chronology relationship should validate")
        .commit(&mut fixture.state)
        .expect("decision chronology relationship should commit");
    }
    let scope = ResponsibilityScope::Function(ResponsibilityFunction::Personnel);
    let mandate = validate_assign_mandate(
        &fixture.state,
        MandateDraft {
            organization: fixture.organization,
            manager: fixture.first,
            scopes: BTreeSet::from([scope]),
            standing_orders: BTreeMap::from([(
                PolicyKind::IndependentRecruitment,
                PolicySetting::IndependentRecruitment(ApprovalPolicy::RequireApproval),
            )]),
            budget: None,
        },
    )
    .expect("decision chronology mandate should validate")
    .commit(&mut fixture.state)
    .expect("decision chronology mandate should commit");
    let request = |fixture: &mut PersistenceFixture,
                   candidate: CharacterId,
                   summary: &str|
     -> DecisionRequestId {
        validate_request_recruitment_approval(
            &fixture.registry,
            &fixture.state,
            RecruitmentApprovalRequestDraft {
                authority: MandateAuthority {
                    mandate,
                    manager: fixture.first,
                    scope,
                },
                target_organization: fixture.organization,
                recruiter: fixture.first,
                candidate,
                approach: RecruitmentApproach::PersonalAppeal,
                attention: AttentionClass::Exception,
                summary: summary.to_owned(),
            },
        )
        .expect("decision chronology request should validate")
        .commit(&mut fixture.state)
        .expect("decision chronology request should commit")
        .decision
    };

    fixture
        .state
        .advance_clock(crate::core::time::SimDuration::from_minutes(5));
    let first = request(
        &mut fixture,
        first_candidate,
        "Approve first recruitment outreach.",
    );
    fixture
        .state
        .advance_clock(crate::core::time::SimDuration::from_minutes(5));
    let second = request(
        &mut fixture,
        second_candidate,
        "Approve second recruitment outreach.",
    );

    let mut replacement = decision_state_wire(fixture.state.decisions());
    assert!(
        replacement.records[&first].requested_at < replacement.records[&second].requested_at,
        "fixture decisions must begin in chronological ID order"
    );
    replacement
        .records
        .get_mut(&second)
        .expect("second decision should exist")
        .requested_at = crate::core::time::SimTime::from_minutes(4);

    let error = restore_save(
        &fixture.registry,
        replace_serialized_substate(
            build_save(&fixture.registry, &fixture.state)
                .expect("valid decision chronology should save before corruption"),
            fixture.state.decisions(),
            &replacement,
        ),
    )
    .expect_err("decision request time may not rewind as monotone IDs advance");
    assert_eq!(
        error,
        LoadError::InvalidState(StateValidationError::InvalidDecisionChronology {
            decision: second,
        })
    );
}

#[test]
fn restore_rejects_world_names_the_canonical_mutators_cannot_create() {
    let fixture = fixture();
    let name = "Persistence Index Test Organization";
    let error = restore_save(
        &fixture.registry,
        blank_serialized_text(
            build_save(&fixture.registry, &fixture.state)
                .expect("valid world state should save before corruption"),
            name,
            1,
        ),
    )
    .expect_err("blank organization name must fail the real restore boundary");
    assert_eq!(
        error,
        LoadError::InvalidState(StateValidationError::EmptyEntityName {
            entity: EntityRef::Organization(fixture.organization),
        })
    );
}

#[test]
fn restore_rejects_ledger_shape_the_canonical_transaction_path_cannot_create() {
    let mut fixture = fixture();
    let source = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::Settlement,
        },
    )
    .expect("source account fixture should validate");
    let destination = insert_account(
        &mut fixture.state,
        FinancialAccountDraft {
            owner: FinancialOwner::Organization(fixture.organization),
            kind: AccountKind::AccountedFunds,
        },
    )
    .expect("destination account fixture should validate");
    let memo = "Persistence ledger transfer";
    let transaction = validate_record_transaction(
        &fixture.state,
        LedgerTransactionDraft {
            occurred_at: fixture.state.now(),
            memo: memo.to_owned(),
            postings: vec![
                LedgerPosting {
                    account: source,
                    amount: Money::from_cents(-100),
                },
                LedgerPosting {
                    account: destination,
                    amount: Money::from_cents(100),
                },
            ],
            authorization: None,
        },
    )
    .expect("ledger fixture should validate")
    .commit(&mut fixture.state)
    .expect("ledger fixture should commit");
    let error = restore_save(
        &fixture.registry,
        blank_serialized_text(
            build_save(&fixture.registry, &fixture.state)
                .expect("valid finance state should save before corruption"),
            memo,
            1,
        ),
    )
    .expect_err("blank ledger memo must fail the real restore boundary");
    assert_eq!(
        error,
        LoadError::InvalidState(StateValidationError::InvalidLedgerTransaction { transaction })
    );
}

#[test]
fn restore_rejects_relationship_shapes_the_canonical_mutator_cannot_create() {
    let mut fixture = fixture();
    validate_set_relationship(
        &fixture.state,
        fixture.first,
        fixture.second,
        RelationshipDimensions::zero(),
    )
    .expect("relationship fixture should validate")
    .commit(&mut fixture.state)
    .expect("relationship should commit");
    let original_wire = social_state_wire(fixture.state.social());
    assert_eq!(
        bincode::serialize(&original_wire).expect("social wire should serialize"),
        bincode::serialize(fixture.state.social()).expect("social state should serialize"),
        "wire mirror must match the production SocialState layout"
    );
    let original_record = fixture
        .state
        .social()
        .get_relationship(fixture.first, fixture.second)
        .expect("relationship fixture should persist");

    let zero_version = SocialStateWire {
        relationships: BTreeMap::from([(
            RelationshipKeyWire {
                from: fixture.first,
                to: fixture.second,
            },
            RelationshipRecordWire {
                from: fixture.first,
                to: fixture.second,
                dimensions: original_record.dimensions(),
                version: 0,
            },
        )]),
    };
    let error = restore_save(
        &fixture.registry,
        replace_serialized_substate(
            build_save(&fixture.registry, &fixture.state)
                .expect("valid relationship state should save"),
            fixture.state.social(),
            &zero_version,
        ),
    )
    .expect_err("zero-version relationship must fail the real restore boundary");
    assert!(matches!(
        error,
        LoadError::InvalidState(StateValidationError::InvalidRelationship { from, to })
            if from == fixture.first && to == fixture.second
    ));

    let self_relationship = SocialStateWire {
        relationships: BTreeMap::from([(
            RelationshipKeyWire {
                from: fixture.first,
                to: fixture.first,
            },
            RelationshipRecordWire {
                from: fixture.first,
                to: fixture.first,
                dimensions: original_record.dimensions(),
                version: 1,
            },
        )]),
    };
    let error = restore_save(
        &fixture.registry,
        replace_serialized_substate(
            build_save(&fixture.registry, &fixture.state)
                .expect("valid relationship state should save"),
            fixture.state.social(),
            &self_relationship,
        ),
    )
    .expect_err("self-relationship must fail the real restore boundary");
    assert!(matches!(
        error,
        LoadError::InvalidState(StateValidationError::InvalidRelationship { from, to })
            if from == fixture.first && to == fixture.first
    ));
}

#[test]
fn restore_rejects_information_recording_time_that_rewinds_in_id_order() {
    let mut fixture = fixture();
    fixture
        .state
        .advance_clock(crate::core::time::SimDuration::from_minutes(5));
    let record = |state: &AppState, summary: &str| {
        validate_record_information(
            state,
            InformationDraft {
                holder: KnowledgeHolder::Organization(fixture.organization),
                source_kind: InformationSourceKind::DirectObservation,
                topic: InformationTopic::Personnel,
                source_entity: None,
                subject: EntityRef::Organization(fixture.organization),
                observed_at: crate::core::time::SimTime::ZERO,
                reliability: Reliability::DirectAccess,
                specificity: Specificity::Precise,
                summary: summary.to_owned(),
            },
        )
        .expect("chronology fixture information should validate")
    };
    let first = record(&fixture.state, "First acquired fact")
        .commit(&mut fixture.state)
        .expect("first information should commit");

    fixture
        .state
        .advance_clock(crate::core::time::SimDuration::from_minutes(5));
    let second = record(&fixture.state, "Second acquired fact")
        .commit(&mut fixture.state)
        .expect("second information should commit");

    let mut replacement = intelligence_state_wire(fixture.state.intelligence());
    assert!(
        replacement.records[&first].chronology.recorded_at
            < replacement.records[&second].chronology.recorded_at,
        "fixture information must begin in chronological ID order"
    );
    replacement
        .records
        .get_mut(&second)
        .expect("second information record should exist")
        .chronology
        .recorded_at = crate::core::time::SimTime::from_minutes(4);

    let error = restore_save(
        &fixture.registry,
        replace_serialized_substate(
            build_save(&fixture.registry, &fixture.state)
                .expect("valid information chronology should save before corruption"),
            fixture.state.intelligence(),
            &replacement,
        ),
    )
    .expect_err("information acquisition time may not rewind as monotone IDs advance");
    assert_eq!(
        error,
        LoadError::InvalidState(StateValidationError::InvalidInformationChronology {
            information: second,
        })
    );
}

#[test]
fn restore_rejects_empty_information_summary() {
    let mut fixture = fixture();
    let summary = "Persistence-only intelligence";
    let information = validate_record_information(
        &fixture.state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(fixture.organization),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::Personnel,
            source_entity: None,
            subject: EntityRef::Organization(fixture.organization),
            observed_at: fixture.state.now(),
            reliability: Reliability::DirectAccess,
            specificity: Specificity::Precise,
            summary: summary.to_owned(),
        },
    )
    .expect("information fixture should validate")
    .commit(&mut fixture.state)
    .expect("information fixture should commit");
    let envelope = build_save(&fixture.registry, &fixture.state)
        .expect("valid information state should save before corruption");
    let error = restore_save(
        &fixture.registry,
        blank_serialized_text(envelope, summary, 1),
    )
    .expect_err("empty information text must fail the real restore boundary");
    assert_eq!(
        error,
        LoadError::InvalidState(StateValidationError::EmptyInformationSummary { information })
    );
}

#[test]
fn restore_rejects_contact_source_kind_without_disclosure_provenance() {
    let mut fixture = fixture();
    let information = validate_record_information(
        &fixture.state,
        InformationDraft {
            holder: KnowledgeHolder::Organization(fixture.organization),
            source_kind: InformationSourceKind::DirectObservation,
            topic: InformationTopic::Personnel,
            source_entity: None,
            subject: EntityRef::Organization(fixture.organization),
            observed_at: fixture.state.now(),
            reliability: Reliability::GenerallyReliable,
            specificity: Specificity::Specific,
            summary: "Persistence contact-provenance fixture".to_owned(),
        },
    )
    .expect("ordinary information fixture should validate")
    .commit(&mut fixture.state)
    .expect("ordinary information fixture should commit");
    let mut replacement = intelligence_state_wire(fixture.state.intelligence());
    replacement
        .records
        .get_mut(&information)
        .expect("information fixture should be present in wire mirror")
        .source
        .source_kind = InformationSourceKind::PoliceContact;

    let error = restore_save(
        &fixture.registry,
        replace_serialized_substate(
            build_save(&fixture.registry, &fixture.state)
                .expect("valid information state should save before provenance corruption"),
            fixture.state.intelligence(),
            &replacement,
        ),
    )
    .expect_err("contact-derived source kind without a disclosure lineage must fail restore");
    assert_eq!(
        error,
        LoadError::InvalidState(StateValidationError::InvalidInformationProvenance {
            information,
            source_information: information,
        })
    );
}

#[test]
fn restore_rejects_system_source_kind_without_authoritative_owner() {
    for source_kind in [
        InformationSourceKind::Accounting,
        InformationSourceKind::Surveillance,
        InformationSourceKind::AfterAction,
    ] {
        let mut fixture = fixture();
        let information = validate_record_information(
            &fixture.state,
            InformationDraft {
                holder: KnowledgeHolder::Organization(fixture.organization),
                source_kind: InformationSourceKind::DirectObservation,
                topic: InformationTopic::Personnel,
                source_entity: None,
                subject: EntityRef::Organization(fixture.organization),
                observed_at: fixture.state.now(),
                reliability: Reliability::GenerallyReliable,
                specificity: Specificity::Specific,
                summary: "Persistence system-provenance fixture".to_owned(),
            },
        )
        .expect("ordinary information fixture should validate")
        .commit(&mut fixture.state)
        .expect("ordinary information fixture should commit");
        let mut replacement = intelligence_state_wire(fixture.state.intelligence());
        replacement
            .records
            .get_mut(&information)
            .expect("information fixture should be present in wire mirror")
            .source
            .source_kind = source_kind;

        let error = restore_save(
            &fixture.registry,
            replace_serialized_substate(
                build_save(&fixture.registry, &fixture.state)
                    .expect("valid information state should save before provenance corruption"),
                fixture.state.intelligence(),
                &replacement,
            ),
        )
        .expect_err("system-authored source kind without an owning artifact must fail restore");
        assert_eq!(
            error,
            LoadError::InvalidState(StateValidationError::UnownedSystemInformation { information })
        );
    }
}

#[test]
fn restore_rejects_empty_report_title_and_entry_summary() {
    let mut fixture = fixture();
    let title = "Persistence financial title";
    let summary = "Persistence financial summary";
    let report = validate_record_report(
        &fixture.state,
        ReportDraft {
            recipient: fixture.organization,
            kind: ReportKind::Financial,
            title: title.to_owned(),
            entries: vec![ReportEntry {
                attention: AttentionClass::Routine,
                summary: summary.to_owned(),
                sources: Vec::new(),
                entities: BTreeSet::from([EntityRef::Organization(fixture.organization)]),
                decision: None,
            }],
        },
    )
    .expect("report fixture should validate")
    .commit(&mut fixture.state)
    .expect("report fixture should commit");

    let envelope = build_save(&fixture.registry, &fixture.state)
        .expect("valid report state should save before title corruption");
    let error = restore_save(&fixture.registry, blank_serialized_text(envelope, title, 1))
        .expect_err("empty report title must fail the real restore boundary");
    assert_eq!(
        error,
        LoadError::InvalidState(StateValidationError::EmptyReportTitle { report })
    );

    let envelope = build_save(&fixture.registry, &fixture.state)
        .expect("valid report state should save before entry corruption");
    let error = restore_save(
        &fixture.registry,
        blank_serialized_text(envelope, summary, 1),
    )
    .expect_err("empty report entry must fail the real restore boundary");
    assert_eq!(
        error,
        LoadError::InvalidState(StateValidationError::EmptyReportEntrySummary { report, entry: 0 })
    );
}

#[test]
fn restore_rejects_empty_history_summary_and_entity_set() {
    let mut fixture = fixture();
    let summary = "Persistence history artifact";
    let event = validate_record_event(
        &fixture.state,
        HistoryEventDraft {
            kind: HistoryEventKind::Recruitment,
            summary: summary.to_owned(),
            entities: BTreeSet::from([EntityRef::Organization(fixture.organization)]),
        },
    )
    .expect("history fixture should validate")
    .commit(&mut fixture.state)
    .expect("history fixture should commit");

    let envelope = build_save(&fixture.registry, &fixture.state)
        .expect("valid history state should save before summary corruption");
    let error = restore_save(
        &fixture.registry,
        blank_serialized_text(envelope, summary, 1),
    )
    .expect_err("empty history summary must fail the real restore boundary");
    assert_eq!(
        error,
        LoadError::InvalidState(StateValidationError::EmptyHistorySummary { event })
    );

    let original_wire = history_state_wire(fixture.state.history());
    assert_eq!(
        bincode::serialize(&original_wire).expect("history wire should serialize"),
        bincode::serialize(fixture.state.history()).expect("history state should serialize"),
        "wire mirror must match the production HistoryState layout"
    );
    let original_len = bincode::serialize(fixture.state.history())
        .expect("history state should serialize")
        .len();
    let mut replacement = HistoryStateWire {
        records: BTreeMap::from([(
            event,
            HistoryEventRecordWire {
                id: event,
                occurred_at: fixture.state.now(),
                kind: HistoryEventKind::Recruitment,
                summary: summary.to_owned(),
                entities: BTreeSet::new(),
            },
        )]),
    };
    while bincode::serialize(&replacement)
        .expect("replacement history should serialize")
        .len()
        < original_len
    {
        replacement
            .records
            .get_mut(&event)
            .expect("replacement event should exist")
            .summary
            .push('X');
    }
    let error = restore_save(
        &fixture.registry,
        replace_serialized_substate(
            build_save(&fixture.registry, &fixture.state)
                .expect("valid history state should save before entity corruption"),
            fixture.state.history(),
            &replacement,
        ),
    )
    .expect_err("entityless history must fail the real restore boundary");
    assert_eq!(
        error,
        LoadError::InvalidState(StateValidationError::HistoryEventHasNoEntities { event })
    );
}
