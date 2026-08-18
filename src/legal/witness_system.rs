//! Case-witness registration, cooperation, and named testimony transactions; anonymous testimony remains ordinary incident evidence.

use crate::core::entity::{is_entity_present, EntityRef};
use crate::core::id::{
    CaseWitnessId, CharacterId, EvidenceId, IdExhaustionError, IdKind, InvestigationId,
    WitnessStatementId,
};
use crate::core::state::AppState;
use crate::legal::{
    Admissibility, CaseWitnessDraft, CaseWitnessRecord, EvidenceAssessment, EvidenceConnection,
    EvidenceIdentity, EvidenceKind, EvidenceRecord, EvidenceReliability, EvidenceStrength,
    InvestigationStatus, WitnessCooperation, WitnessStatementDraft, WitnessStatementRecord,
};
use crate::world::Lifecycle;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum WitnessError {
    #[error("investigation {0} does not exist")]
    MissingInvestigation(InvestigationId),
    #[error("investigation {0} is not active")]
    InactiveInvestigation(InvestigationId),
    #[error("character {0} does not exist")]
    MissingCharacter(CharacterId),
    #[error("character {0} is not active and cannot be registered as a current witness")]
    InactiveWitness(CharacterId),
    #[error("character {witness} is already registered as case witness {existing} for investigation {investigation}")]
    DuplicateCaseWitness {
        investigation: InvestigationId,
        witness: CharacterId,
        existing: CaseWitnessId,
    },
    #[error("case witness {0} does not exist")]
    MissingCaseWitness(CaseWitnessId),
    #[error("witness statement summary must not be empty")]
    EmptyStatement,
    #[error("witness statement references missing entity {0:?}")]
    MissingEntity(EntityRef),
    #[error("case witness {witness} already has cooperation state {cooperation:?}")]
    CooperationUnchanged {
        witness: CaseWitnessId,
        cooperation: WitnessCooperation,
    },
    #[error("investigation {investigation} changed after witness validation; expected version {expected}, found {found}")]
    StaleInvestigation {
        investigation: InvestigationId,
        expected: u32,
        found: u32,
    },
    #[error("character {witness} changed after witness validation; expected version {expected}, found {found}")]
    StaleWitnessCharacter {
        witness: CharacterId,
        expected: u32,
        found: u32,
    },
    #[error("case witness {witness} changed after validation; expected version {expected}, found {found}")]
    StaleCaseWitness {
        witness: CaseWitnessId,
        expected: u32,
        found: u32,
    },
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

#[derive(Debug)]
pub struct ValidatedCaseWitnessRegistration {
    draft: CaseWitnessDraft,
    expected_investigation_version: u32,
    expected_character_version: u32,
}

impl ValidatedCaseWitnessRegistration {
    pub fn commit(self, state: &mut AppState) -> Result<CaseWitnessId, WitnessError> {
        validate_registration_snapshot(
            state,
            self.draft,
            self.expected_investigation_version,
            self.expected_character_version,
        )?;
        let id = state.ids.next_case_witness()?;
        state.legal.insert_case_witness(CaseWitnessRecord {
            id,
            investigation: self.draft.investigation,
            witness: self.draft.witness,
            cooperation: self.draft.cooperation,
            registered_at: state.now(),
            statements: Default::default(),
            version: 1,
        });
        Ok(id)
    }
}

pub fn validate_register_case_witness(
    state: &AppState,
    draft: CaseWitnessDraft,
) -> Result<ValidatedCaseWitnessRegistration, WitnessError> {
    validate_registration_dependencies(state, draft)?;
    let investigation = state
        .legal
        .get_investigation(draft.investigation)
        .expect("validated investigation must still exist");
    let witness = state
        .world
        .get_character(draft.witness)
        .expect("validated witness character must still exist");
    Ok(ValidatedCaseWitnessRegistration {
        draft,
        expected_investigation_version: investigation.version(),
        expected_character_version: witness.version(),
    })
}

fn validate_registration_snapshot(
    state: &AppState,
    draft: CaseWitnessDraft,
    expected_investigation_version: u32,
    expected_character_version: u32,
) -> Result<(), WitnessError> {
    let investigation = state
        .legal
        .get_investigation(draft.investigation)
        .ok_or(WitnessError::MissingInvestigation(draft.investigation))?;
    if investigation.version() != expected_investigation_version {
        return Err(WitnessError::StaleInvestigation {
            investigation: draft.investigation,
            expected: expected_investigation_version,
            found: investigation.version(),
        });
    }
    let witness = state
        .world
        .get_character(draft.witness)
        .ok_or(WitnessError::MissingCharacter(draft.witness))?;
    if witness.version() != expected_character_version {
        return Err(WitnessError::StaleWitnessCharacter {
            witness: draft.witness,
            expected: expected_character_version,
            found: witness.version(),
        });
    }
    validate_registration_dependencies(state, draft)
}

fn validate_registration_dependencies(
    state: &AppState,
    draft: CaseWitnessDraft,
) -> Result<(), WitnessError> {
    let investigation = state
        .legal
        .get_investigation(draft.investigation)
        .ok_or(WitnessError::MissingInvestigation(draft.investigation))?;
    if investigation.status() != InvestigationStatus::Active {
        return Err(WitnessError::InactiveInvestigation(draft.investigation));
    }
    let witness = state
        .world
        .get_character(draft.witness)
        .ok_or(WitnessError::MissingCharacter(draft.witness))?;
    if witness.lifecycle() != Lifecycle::Active {
        return Err(WitnessError::InactiveWitness(draft.witness));
    }
    if let Some(existing) = state
        .legal
        .case_witness_for(draft.investigation, draft.witness)
    {
        return Err(WitnessError::DuplicateCaseWitness {
            investigation: draft.investigation,
            witness: draft.witness,
            existing: existing.id(),
        });
    }
    Ok(())
}

#[derive(Debug)]
pub struct ValidatedWitnessCooperation {
    case_witness: CaseWitnessId,
    cooperation: WitnessCooperation,
    expected_witness_version: u32,
    expected_investigation_version: u32,
}

impl ValidatedWitnessCooperation {
    pub fn commit(self, state: &mut AppState) -> Result<(), WitnessError> {
        let witness = validate_witness_mutation_snapshot(
            state,
            self.case_witness,
            self.expected_witness_version,
            self.expected_investigation_version,
        )?;
        validate_current_witness_character(state, witness.witness())?;
        if witness.cooperation() == self.cooperation {
            return Err(WitnessError::CooperationUnchanged {
                witness: self.case_witness,
                cooperation: self.cooperation,
            });
        }
        state
            .legal
            .set_witness_cooperation(self.case_witness, self.cooperation);
        Ok(())
    }
}

pub fn validate_set_witness_cooperation(
    state: &AppState,
    case_witness: CaseWitnessId,
    cooperation: WitnessCooperation,
) -> Result<ValidatedWitnessCooperation, WitnessError> {
    let witness = validate_case_witness_for_active_case(state, case_witness)?;
    validate_current_witness_character(state, witness.witness())?;
    if witness.cooperation() == cooperation {
        return Err(WitnessError::CooperationUnchanged {
            witness: case_witness,
            cooperation,
        });
    }
    let investigation = state
        .legal
        .get_investigation(witness.investigation())
        .expect("validated case witness investigation must exist");
    Ok(ValidatedWitnessCooperation {
        case_witness,
        cooperation,
        expected_witness_version: witness.version(),
        expected_investigation_version: investigation.version(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WitnessStatementOutcome {
    pub statement: WitnessStatementId,
    pub evidence: EvidenceId,
}

#[derive(Debug)]
pub struct ValidatedWitnessStatement {
    draft: WitnessStatementDraft,
    expected_witness_version: u32,
    expected_investigation_version: u32,
}

impl ValidatedWitnessStatement {
    pub fn commit(self, state: &mut AppState) -> Result<WitnessStatementOutcome, WitnessError> {
        state
            .ids
            .reserve_many(&[(IdKind::WitnessStatement, 1), (IdKind::Evidence, 1)])?;
        let (investigation_id, witness_id) = {
            let case_witness = validate_witness_mutation_snapshot(
                state,
                self.draft.case_witness,
                self.expected_witness_version,
                self.expected_investigation_version,
            )?;
            validate_statement_dependencies(state, case_witness, &self.draft)?;
            (case_witness.investigation(), case_witness.witness())
        };

        let statement = state.ids.next_witness_statement()?;
        let evidence = state.ids.next_evidence()?;
        let investigation = state
            .legal
            .get_investigation(investigation_id)
            .expect("validated witness investigation must exist");
        let recorded_at = state.now();
        state.legal.insert_evidence(
            EvidenceRecord {
                identity: EvidenceIdentity {
                    id: evidence,
                    investigation: investigation_id,
                    custodian: investigation.owner(),
                },
                connection: EvidenceConnection {
                    subject: self.draft.subject,
                    origin: self.draft.origin,
                    source: Some(EntityRef::Character(witness_id)),
                    derived_from: Default::default(),
                },
                assessment: EvidenceAssessment {
                    kind: EvidenceKind::WitnessTestimony,
                    strength: witness_strength(self.draft.confidence),
                    reliability: witness_reliability(self.draft.confidence),
                    admissibility: Admissibility::Unknown,
                },
                discovered_at: recorded_at,
            },
            recorded_at,
        );
        state
            .legal
            .insert_witness_statement(WitnessStatementRecord {
                id: statement,
                case_witness: self.draft.case_witness,
                subject: self.draft.subject,
                origin: self.draft.origin,
                confidence: self.draft.confidence,
                summary: self.draft.summary,
                evidence,
                recorded_at,
            });
        Ok(WitnessStatementOutcome {
            statement,
            evidence,
        })
    }
}

pub fn validate_record_witness_statement(
    state: &AppState,
    draft: WitnessStatementDraft,
) -> Result<ValidatedWitnessStatement, WitnessError> {
    let case_witness = validate_case_witness_for_active_case(state, draft.case_witness)?;
    validate_statement_dependencies(state, case_witness, &draft)?;
    let investigation = state
        .legal
        .get_investigation(case_witness.investigation())
        .expect("validated witness investigation must exist");
    Ok(ValidatedWitnessStatement {
        draft,
        expected_witness_version: case_witness.version(),
        expected_investigation_version: investigation.version(),
    })
}

fn validate_witness_mutation_snapshot(
    state: &AppState,
    case_witness_id: CaseWitnessId,
    expected_witness_version: u32,
    expected_investigation_version: u32,
) -> Result<&CaseWitnessRecord, WitnessError> {
    let case_witness = state
        .legal
        .get_case_witness(case_witness_id)
        .ok_or(WitnessError::MissingCaseWitness(case_witness_id))?;
    if case_witness.version() != expected_witness_version {
        return Err(WitnessError::StaleCaseWitness {
            witness: case_witness_id,
            expected: expected_witness_version,
            found: case_witness.version(),
        });
    }
    let investigation = state
        .legal
        .get_investigation(case_witness.investigation())
        .ok_or(WitnessError::MissingInvestigation(
            case_witness.investigation(),
        ))?;
    if investigation.version() != expected_investigation_version {
        return Err(WitnessError::StaleInvestigation {
            investigation: investigation.id(),
            expected: expected_investigation_version,
            found: investigation.version(),
        });
    }
    if investigation.status() != InvestigationStatus::Active {
        return Err(WitnessError::InactiveInvestigation(investigation.id()));
    }
    Ok(case_witness)
}

fn validate_case_witness_for_active_case(
    state: &AppState,
    case_witness: CaseWitnessId,
) -> Result<&CaseWitnessRecord, WitnessError> {
    let witness = state
        .legal
        .get_case_witness(case_witness)
        .ok_or(WitnessError::MissingCaseWitness(case_witness))?;
    let investigation = state
        .legal
        .get_investigation(witness.investigation())
        .ok_or(WitnessError::MissingInvestigation(witness.investigation()))?;
    if investigation.status() != InvestigationStatus::Active {
        return Err(WitnessError::InactiveInvestigation(investigation.id()));
    }
    Ok(witness)
}

fn validate_statement_dependencies(
    state: &AppState,
    case_witness: &CaseWitnessRecord,
    draft: &WitnessStatementDraft,
) -> Result<(), WitnessError> {
    if draft.summary.trim().is_empty() {
        return Err(WitnessError::EmptyStatement);
    }
    if !is_entity_present(state, draft.subject) {
        return Err(WitnessError::MissingEntity(draft.subject));
    }
    if let Some(origin) = draft.origin {
        if !is_entity_present(state, origin) {
            return Err(WitnessError::MissingEntity(origin));
        }
    }
    validate_current_witness_character(state, case_witness.witness())?;
    Ok(())
}

fn validate_current_witness_character(
    state: &AppState,
    witness: CharacterId,
) -> Result<(), WitnessError> {
    let record = state
        .world
        .get_character(witness)
        .ok_or(WitnessError::MissingCharacter(witness))?;
    if record.lifecycle() != Lifecycle::Active {
        return Err(WitnessError::InactiveWitness(witness));
    }
    Ok(())
}

pub(crate) fn witness_strength(confidence: crate::world::Rating) -> EvidenceStrength {
    match confidence.value() {
        0..=34 => EvidenceStrength::Weak,
        35..=59 => EvidenceStrength::Corroborating,
        60..=84 => EvidenceStrength::Strong,
        85..=100 => EvidenceStrength::Direct,
        _ => unreachable!(),
    }
}

pub(crate) fn witness_reliability(confidence: crate::world::Rating) -> EvidenceReliability {
    match confidence.value() {
        0..=24 => EvidenceReliability::Questionable,
        25..=49 => EvidenceReliability::Mixed,
        50..=79 => EvidenceReliability::Credible,
        80..=100 => EvidenceReliability::HighlyReliable,
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::invariants::{
        validate_invariants, validate_state, validate_state_against_registry,
    };
    use crate::core::persistence::{build_save, restore_save};
    use crate::legal::investigation_system::{
        validate_add_evidence, validate_open_investigation, validate_transition_investigation,
        InvestigationTransition,
    };
    use crate::legal::{EvidenceDraft, InvestigationDraft, WitnessStatementDraft};
    use crate::world::world_system::{insert_character, insert_organization};
    use crate::world::{
        AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind, Rating,
    };
    use std::collections::{BTreeMap, BTreeSet};

    struct WitnessFixture {
        state: AppState,
        police: crate::core::id::OrganizationId,
        criminal: crate::core::id::OrganizationId,
        investigation: InvestigationId,
        witness: CharacterId,
        subject: CharacterId,
    }

    fn rating(value: u8) -> Rating {
        Rating::try_new(value).expect("test rating must be valid")
    }

    fn make_fixture() -> WitnessFixture {
        let registry = build_registry();
        let mut state = AppState::new(0x7117_E551);
        let police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Witness Bureau".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("police fixture should validate");
        let criminal = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Witness Crew".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("criminal fixture should validate");
        let witness = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Daniel Mercer".to_owned(),
                organization: None,
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("witness fixture should validate");
        let subject = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Frank Dello".to_owned(),
                organization: Some(criminal),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("subject fixture should validate");
        let investigation = validate_open_investigation(
            &state,
            InvestigationDraft {
                owner: police,
                title: "Witness identification inquiry".to_owned(),
                subjects: BTreeSet::from([EntityRef::Character(subject)]),
            },
        )
        .expect("investigation fixture should validate")
        .commit(&mut state)
        .expect("investigation fixture should commit");
        WitnessFixture {
            state,
            police,
            criminal,
            investigation,
            witness,
            subject,
        }
    }

    #[test]
    fn named_witness_statement_creates_source_bearing_testimony_and_survives_save() {
        let registry = build_registry();
        let mut fixture = make_fixture();
        let case_witness = validate_register_case_witness(
            &fixture.state,
            CaseWitnessDraft {
                investigation: fixture.investigation,
                witness: fixture.witness,
                cooperation: WitnessCooperation::Cooperative,
            },
        )
        .expect("case witness registration should validate")
        .commit(&mut fixture.state)
        .expect("case witness registration should commit");
        let outcome = validate_record_witness_statement(
            &fixture.state,
            WitnessStatementDraft {
                case_witness,
                subject: EntityRef::Character(fixture.subject),
                origin: Some(EntityRef::Organization(fixture.criminal)),
                confidence: rating(88),
                summary:
                    "Mercer identifies Frank Dello as the man he saw leaving the crew's garage."
                        .to_owned(),
            },
        )
        .expect("named witness statement should validate")
        .commit(&mut fixture.state)
        .expect("named witness statement should commit");

        let statement = fixture
            .state
            .legal()
            .get_witness_statement(outcome.statement)
            .expect("statement should exist");
        assert_eq!(statement.case_witness(), case_witness);
        assert_eq!(statement.evidence(), outcome.evidence);
        assert_eq!(statement.confidence(), rating(88));
        let evidence = fixture
            .state
            .legal()
            .get_evidence(outcome.evidence)
            .expect("statement evidence should exist");
        assert_eq!(evidence.kind(), EvidenceKind::WitnessTestimony);
        assert_eq!(evidence.strength(), EvidenceStrength::Direct);
        assert_eq!(evidence.reliability(), EvidenceReliability::HighlyReliable);
        assert_eq!(evidence.admissibility(), Admissibility::Unknown);
        assert_eq!(evidence.subject(), EntityRef::Character(fixture.subject));
        assert_eq!(
            evidence.origin(),
            Some(EntityRef::Organization(fixture.criminal))
        );
        assert_eq!(
            evidence.source(),
            Some(EntityRef::Character(fixture.witness))
        );
        assert_eq!(
            fixture
                .state
                .legal()
                .evidence_from_source(EntityRef::Character(fixture.witness))
                .map(|record| record.id())
                .collect::<Vec<_>>(),
            vec![outcome.evidence]
        );
        assert_eq!(
            fixture
                .state
                .legal()
                .witness_statement_for_evidence(outcome.evidence)
                .map(|record| record.id()),
            Some(outcome.statement)
        );
        assert_eq!(
            fixture
                .state
                .legal()
                .statements_for_case_witness(case_witness)
                .map(|record| record.id())
                .collect::<Vec<_>>(),
            vec![outcome.statement]
        );

        let mut restored = restore_save(
            &registry,
            build_save(&registry, &fixture.state).expect("named testimony state should save"),
        )
        .expect("named testimony state should restore");
        let restored_evidence = restored
            .legal()
            .get_evidence(outcome.evidence)
            .expect("restored witness evidence should exist");
        assert_eq!(
            restored_evidence.source(),
            Some(EntityRef::Character(fixture.witness))
        );
        assert_eq!(
            restored
                .legal()
                .witness_statement_for_evidence(outcome.evidence)
                .map(|record| record.id()),
            Some(outcome.statement)
        );

        let second_witness = insert_character(
            &registry,
            &mut restored,
            CharacterDraft {
                name: "Nora Bell".to_owned(),
                organization: None,
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("post-restore witness fixture should validate");
        let second_case_witness = validate_register_case_witness(
            &restored,
            CaseWitnessDraft {
                investigation: fixture.investigation,
                witness: second_witness,
                cooperation: WitnessCooperation::Reluctant,
            },
        )
        .expect("post-restore witness registration should validate")
        .commit(&mut restored)
        .expect("post-restore witness registration should allocate a fresh ID");
        let second_statement = validate_record_witness_statement(
            &restored,
            WitnessStatementDraft {
                case_witness: second_case_witness,
                subject: EntityRef::Character(fixture.subject),
                origin: None,
                confidence: rating(61),
                summary: "Bell separately places Dello near the garage that evening.".to_owned(),
            },
        )
        .expect("post-restore testimony should validate")
        .commit(&mut restored)
        .expect("post-restore testimony should allocate fresh statement and evidence IDs");
        assert!(second_case_witness.raw() > case_witness.raw());
        assert!(second_statement.statement.raw() > outcome.statement.raw());
        assert!(second_statement.evidence.raw() > outcome.evidence.raw());
        validate_state(&restored).expect("restored testimony state should be structurally valid");
        validate_state_against_registry(&registry, &restored)
            .expect("restored testimony state should remain registry-valid");
        validate_invariants(&restored);
    }

    #[test]
    fn witness_registration_and_cooperation_tokens_reject_case_and_statement_changes() {
        let mut fixture = make_fixture();
        let stale_registration = validate_register_case_witness(
            &fixture.state,
            CaseWitnessDraft {
                investigation: fixture.investigation,
                witness: fixture.witness,
                cooperation: WitnessCooperation::Reluctant,
            },
        )
        .expect("registration should initially validate");
        validate_add_evidence(
            &fixture.state,
            EvidenceDraft {
                investigation: fixture.investigation,
                custodian: fixture.police,
                subject: EntityRef::Character(fixture.subject),
                origin: None,
                kind: EvidenceKind::Document,
                strength: EvidenceStrength::Weak,
                reliability: EvidenceReliability::Mixed,
                admissibility: Admissibility::Unknown,
                discovered_at: fixture.state.now(),
            },
        )
        .expect("case mutation should validate")
        .commit(&mut fixture.state)
        .expect("case mutation should commit");
        assert!(matches!(
            stale_registration.commit(&mut fixture.state),
            Err(WitnessError::StaleInvestigation { .. })
        ));

        let case_witness = validate_register_case_witness(
            &fixture.state,
            CaseWitnessDraft {
                investigation: fixture.investigation,
                witness: fixture.witness,
                cooperation: WitnessCooperation::Reluctant,
            },
        )
        .expect("fresh registration should validate")
        .commit(&mut fixture.state)
        .expect("fresh registration should commit");
        assert_eq!(
            validate_register_case_witness(
                &fixture.state,
                CaseWitnessDraft {
                    investigation: fixture.investigation,
                    witness: fixture.witness,
                    cooperation: WitnessCooperation::Cooperative,
                },
            )
            .expect_err("same character cannot be registered twice on one case"),
            WitnessError::DuplicateCaseWitness {
                investigation: fixture.investigation,
                witness: fixture.witness,
                existing: case_witness,
            }
        );

        let stale_cooperation = validate_set_witness_cooperation(
            &fixture.state,
            case_witness,
            WitnessCooperation::Cooperative,
        )
        .expect("cooperation change should initially validate");
        validate_record_witness_statement(
            &fixture.state,
            WitnessStatementDraft {
                case_witness,
                subject: EntityRef::Character(fixture.subject),
                origin: None,
                confidence: rating(55),
                summary: "Mercer says he is fairly sure Dello was present.".to_owned(),
            },
        )
        .expect("statement should validate")
        .commit(&mut fixture.state)
        .expect("statement should commit");
        assert!(matches!(
            stale_cooperation.commit(&mut fixture.state),
            Err(WitnessError::StaleCaseWitness { .. })
        ));
        validate_set_witness_cooperation(
            &fixture.state,
            case_witness,
            WitnessCooperation::Cooperative,
        )
        .expect("fresh cooperation token should validate")
        .commit(&mut fixture.state)
        .expect("fresh cooperation change should commit");
        assert_eq!(
            fixture
                .state
                .legal()
                .get_case_witness(case_witness)
                .expect("case witness should exist")
                .cooperation(),
            WitnessCooperation::Cooperative
        );
        validate_state(&fixture.state).expect("versioned witness state should remain valid");
    }

    #[test]
    fn suspended_case_preserves_testimony_but_rejects_new_witness_activity() {
        let registry = build_registry();
        let mut fixture = make_fixture();
        let case_witness = validate_register_case_witness(
            &fixture.state,
            CaseWitnessDraft {
                investigation: fixture.investigation,
                witness: fixture.witness,
                cooperation: WitnessCooperation::Cooperative,
            },
        )
        .expect("registration should validate")
        .commit(&mut fixture.state)
        .expect("registration should commit");
        let historical = validate_record_witness_statement(
            &fixture.state,
            WitnessStatementDraft {
                case_witness,
                subject: EntityRef::Character(fixture.subject),
                origin: None,
                confidence: rating(72),
                summary: "Mercer identifies Dello from the alley encounter.".to_owned(),
            },
        )
        .expect("historical statement should validate")
        .commit(&mut fixture.state)
        .expect("historical statement should commit");
        validate_transition_investigation(
            &fixture.state,
            fixture.investigation,
            InvestigationTransition::Suspend,
        )
        .expect("case suspension should validate")
        .commit(&mut fixture.state)
        .expect("case suspension should commit");

        let statement_error = match validate_record_witness_statement(
            &fixture.state,
            WitnessStatementDraft {
                case_witness,
                subject: EntityRef::Character(fixture.subject),
                origin: None,
                confidence: rating(90),
                summary: "Mercer offers a second identification.".to_owned(),
            },
        ) {
            Ok(_) => panic!("suspended case must reject new witness statements"),
            Err(error) => error,
        };
        assert_eq!(
            statement_error,
            WitnessError::InactiveInvestigation(fixture.investigation)
        );
        assert_eq!(
            validate_set_witness_cooperation(
                &fixture.state,
                case_witness,
                WitnessCooperation::Hostile,
            )
            .expect_err("suspended case must reject cooperation mutation"),
            WitnessError::InactiveInvestigation(fixture.investigation)
        );
        assert!(fixture
            .state
            .legal()
            .get_witness_statement(historical.statement)
            .is_some());
        assert!(fixture
            .state
            .legal()
            .get_evidence(historical.evidence)
            .is_some());

        let restored = restore_save(
            &registry,
            build_save(&registry, &fixture.state)
                .expect("suspended case with testimony should save"),
        )
        .expect("suspended case with testimony should restore");
        assert!(restored
            .legal()
            .get_witness_statement(historical.statement)
            .is_some());
        validate_state(&restored).expect("historical testimony should survive suspension");
        validate_invariants(&restored);
    }

    #[test]
    fn anonymous_witness_testimony_remains_valid_without_named_source() {
        let registry = build_registry();
        let mut fixture = make_fixture();
        let evidence = validate_add_evidence(
            &fixture.state,
            EvidenceDraft {
                investigation: fixture.investigation,
                custodian: fixture.police,
                subject: EntityRef::Character(fixture.subject),
                origin: Some(EntityRef::Organization(fixture.criminal)),
                kind: EvidenceKind::WitnessTestimony,
                strength: EvidenceStrength::Corroborating,
                reliability: EvidenceReliability::Credible,
                admissibility: Admissibility::Unknown,
                discovered_at: fixture.state.now(),
            },
        )
        .expect("anonymous testimony should remain valid evidence")
        .commit(&mut fixture.state)
        .expect("anonymous testimony should commit");
        let record = fixture
            .state
            .legal()
            .get_evidence(evidence)
            .expect("anonymous testimony should exist");
        assert_eq!(record.kind(), EvidenceKind::WitnessTestimony);
        assert_eq!(record.source(), None);
        assert!(fixture
            .state
            .legal()
            .witness_statement_for_evidence(evidence)
            .is_none());
        validate_state(&fixture.state)
            .expect("anonymous testimony should remain structurally valid");
        validate_state_against_registry(&registry, &fixture.state)
            .expect("anonymous testimony should remain registry-valid");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn witness_confidence_maps_to_deterministic_evidence_bands() {
        for (confidence, strength) in [
            (0, EvidenceStrength::Weak),
            (34, EvidenceStrength::Weak),
            (35, EvidenceStrength::Corroborating),
            (59, EvidenceStrength::Corroborating),
            (60, EvidenceStrength::Strong),
            (84, EvidenceStrength::Strong),
            (85, EvidenceStrength::Direct),
            (100, EvidenceStrength::Direct),
        ] {
            assert_eq!(witness_strength(rating(confidence)), strength);
        }
        for (confidence, reliability) in [
            (0, EvidenceReliability::Questionable),
            (24, EvidenceReliability::Questionable),
            (25, EvidenceReliability::Mixed),
            (49, EvidenceReliability::Mixed),
            (50, EvidenceReliability::Credible),
            (79, EvidenceReliability::Credible),
            (80, EvidenceReliability::HighlyReliable),
            (100, EvidenceReliability::HighlyReliable),
        ] {
            assert_eq!(witness_reliability(rating(confidence)), reliability);
        }
    }
}
