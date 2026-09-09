//! Versioned persistence envelope; serialization adapters remain outside the simulation core.

use crate::core::invariants::{
    StateValidationError, validate_state, validate_state_against_registry,
};
use crate::core::state::{AppState, CURRENT_STATE_SCHEMA_VERSION};
use crate::registry::Registry;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const CURRENT_SAVE_FORMAT_VERSION: u16 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveEnvelope {
    format_version: u16,
    content_revision: u32,
    state: AppState,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum SaveError {
    #[error("cannot save invalid application state: {0}")]
    InvalidState(#[from] StateValidationError),
}

pub fn build_save(registry: &Registry, state: &AppState) -> Result<SaveEnvelope, SaveError> {
    validate_state(state)?;
    validate_state_against_registry(registry, state)?;
    Ok(SaveEnvelope {
        format_version: CURRENT_SAVE_FORMAT_VERSION,
        content_revision: registry.content_revision(),
        state: state.clone(),
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum LoadError {
    #[error("unsupported save format version {found}; expected {expected}")]
    UnsupportedFormat { found: u16, expected: u16 },
    #[error("unsupported state schema version {found}; expected {expected}")]
    UnsupportedStateSchema { found: u16, expected: u16 },
    #[error("save content revision {found} does not match loaded registry revision {expected}")]
    ContentRevisionMismatch { found: u32, expected: u32 },
    #[error("save authoritative records cannot rebuild deterministic derived indexes")]
    InvalidDerivedIndexRebuild,
    #[error("save contains invalid application state: {0}")]
    InvalidState(#[source] StateValidationError),
}

pub fn restore_save(registry: &Registry, envelope: SaveEnvelope) -> Result<AppState, LoadError> {
    if envelope.format_version != CURRENT_SAVE_FORMAT_VERSION {
        return Err(LoadError::UnsupportedFormat {
            found: envelope.format_version,
            expected: CURRENT_SAVE_FORMAT_VERSION,
        });
    }
    if envelope.state.state_schema_version() != CURRENT_STATE_SCHEMA_VERSION {
        return Err(LoadError::UnsupportedStateSchema {
            found: envelope.state.state_schema_version(),
            expected: CURRENT_STATE_SCHEMA_VERSION,
        });
    }
    if envelope.content_revision != registry.content_revision() {
        return Err(LoadError::ContentRevisionMismatch {
            found: envelope.content_revision,
            expected: registry.content_revision(),
        });
    }
    let mut state = envelope.state;
    if !state.rebuild_derived_indexes_after_restore() {
        return Err(LoadError::InvalidDerivedIndexRebuild);
    }
    validate_state(&state).map_err(LoadError::InvalidState)?;
    validate_state_against_registry(registry, &state).map_err(LoadError::InvalidState)?;
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::attention::AttentionClass;
    use crate::core::entity::EntityRef;
    use crate::core::id::{CharacterId, HistoryEventId};
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
    use crate::reports::report_system::validate_record_report;
    use crate::reports::{ReportDraft, ReportEntry, ReportKind};
    use crate::social::relationship_system::validate_set_relationship;
    use crate::social::{RelationshipDimensions, SocialState};
    use crate::world::world_system::{insert_character, insert_organization};
    use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind};
    use serde::Serialize;
    use std::collections::{BTreeMap, BTreeSet};

    struct PersistenceFixture {
        registry: Registry,
        state: AppState,
        organization: crate::core::id::OrganizationId,
        first: CharacterId,
        second: CharacterId,
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
                    occurred_at: fixture.state.now(),
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
    fn restore_rejects_empty_information_summary() {
        let mut fixture = fixture();
        let summary = "Persistence-only intelligence";
        let information = validate_record_information(
            &fixture.state,
            InformationDraft {
                holder: KnowledgeHolder::Organization(fixture.organization),
                source_kind: InformationSourceKind::DirectObservation,
                topic: InformationTopic::General,
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
            LoadError::InvalidState(StateValidationError::EmptyReportEntrySummary {
                report,
                entry: 0,
            })
        );
    }

    #[test]
    fn restore_rejects_empty_history_summary_and_entity_set() {
        let mut fixture = fixture();
        let summary = "Persistence history artifact";
        let event = validate_record_event(
            &fixture.state,
            HistoryEventDraft {
                occurred_at: fixture.state.now(),
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
}
