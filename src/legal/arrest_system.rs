//! Evidence-backed arrest and custody lifecycle transactions.

use crate::core::entity::EntityRef;
use crate::core::id::{
    ArrestId, CharacterId, EvidenceId, IdExhaustionError, InvestigationId, InvestigationWorkId,
    OperationId, OrganizationId,
};
use crate::core::state::AppState;
use crate::legal::{
    ArrestDraft, ArrestRecord, ArrestStatus, InvestigationStatus, InvestigationWorkStatus,
};
use crate::operations::OperationStatus;
use crate::world::{Lifecycle, OrganizationKind};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ArrestError {
    #[error("character {0} does not exist")]
    MissingCharacter(CharacterId),
    #[error("character {0} is not active and cannot enter a new custody record")]
    InactiveCharacter(CharacterId),
    #[error("arrest evidence {evidence} is too weak for custody: strength {strength:?}, reliability {reliability:?}")]
    InsufficientEvidence {
        evidence: EvidenceId,
        strength: crate::legal::EvidenceStrength,
        reliability: crate::legal::EvidenceReliability,
    },
    #[error("investigation {0} does not exist")]
    MissingInvestigation(InvestigationId),
    #[error("investigation {0} is not active")]
    InactiveInvestigation(InvestigationId),
    #[error("investigation owner {0} is not an active law-enforcement authority")]
    InvalidAuthority(OrganizationId),
    #[error("character {character} is not a subject of investigation {investigation}")]
    CharacterNotSubject {
        character: CharacterId,
        investigation: InvestigationId,
    },
    #[error("arrest must cite at least one evidence record")]
    NoEvidence,
    #[error("evidence {0} does not exist")]
    MissingEvidence(EvidenceId),
    #[error("evidence {evidence} does not belong to investigation {investigation}")]
    EvidenceInvestigationMismatch {
        evidence: EvidenceId,
        investigation: InvestigationId,
    },
    #[error("evidence {evidence} is not held by arresting authority {authority}")]
    EvidenceCustodianMismatch {
        evidence: EvidenceId,
        authority: OrganizationId,
    },
    #[error("evidence {evidence} does not identify character {character} as its subject")]
    EvidenceSubjectMismatch {
        evidence: EvidenceId,
        character: CharacterId,
    },
    #[error("character {character} is already detained under arrest {arrest}")]
    AlreadyDetained {
        character: CharacterId,
        arrest: ArrestId,
    },
    #[error("character {character} is assigned to active operation {operation}")]
    ActiveOperationResponsibility {
        character: CharacterId,
        operation: OperationId,
    },
    #[error("character {character} owns scheduled investigation work {work}")]
    ScheduledInvestigationWork {
        character: CharacterId,
        work: InvestigationWorkId,
    },
    #[error(
        "investigation {investigation} changed after arrest validation; expected version {expected}, found {found}"
    )]
    StaleInvestigation {
        investigation: InvestigationId,
        expected: u32,
        found: u32,
    },
    #[error(
        "character {character} changed after arrest validation; expected version {expected}, found {found}"
    )]
    StaleCharacter {
        character: CharacterId,
        expected: u32,
        found: u32,
    },
    #[error("arrest {0} does not exist")]
    MissingArrest(ArrestId),
    #[error("arrest {0} is not an active detention")]
    NotDetained(ArrestId),
    #[error("arrest {arrest} changed after release validation; expected version {expected}, found {found}")]
    StaleArrest {
        arrest: ArrestId,
        expected: u32,
        found: u32,
    },
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

#[derive(Debug)]
pub struct ValidatedArrest {
    draft: ArrestDraft,
    authority: OrganizationId,
    expected_investigation_version: u32,
    expected_character_version: u32,
}

impl ValidatedArrest {
    pub fn commit(self, state: &mut AppState) -> Result<ArrestId, ArrestError> {
        let investigation = state
            .legal
            .get_investigation(self.draft.investigation)
            .ok_or(ArrestError::MissingInvestigation(self.draft.investigation))?;
        if investigation.version() != self.expected_investigation_version {
            return Err(ArrestError::StaleInvestigation {
                investigation: self.draft.investigation,
                expected: self.expected_investigation_version,
                found: investigation.version(),
            });
        }
        let character = state
            .world
            .get_character(self.draft.character)
            .ok_or(ArrestError::MissingCharacter(self.draft.character))?;
        if character.version() != self.expected_character_version {
            return Err(ArrestError::StaleCharacter {
                character: self.draft.character,
                expected: self.expected_character_version,
                found: character.version(),
            });
        }
        let authority = validate_arrest_dependencies(state, &self.draft)?;
        debug_assert_eq!(authority, self.authority);

        let id = state.ids.next_arrest()?;
        state.legal.insert_arrest(ArrestRecord {
            id,
            character: self.draft.character,
            authority,
            investigation: self.draft.investigation,
            evidence: self.draft.evidence,
            arrested_at: state.now(),
            released_at: None,
            status: ArrestStatus::Detained,
            version: 1,
        });
        Ok(id)
    }
}

pub fn validate_arrest(
    state: &AppState,
    draft: ArrestDraft,
) -> Result<ValidatedArrest, ArrestError> {
    let authority = validate_arrest_dependencies(state, &draft)?;
    let investigation = state
        .legal
        .get_investigation(draft.investigation)
        .expect("validated investigation must exist");
    let character = state
        .world
        .get_character(draft.character)
        .expect("validated arrest character must exist");
    Ok(ValidatedArrest {
        draft,
        authority,
        expected_investigation_version: investigation.version(),
        expected_character_version: character.version(),
    })
}

fn validate_arrest_dependencies(
    state: &AppState,
    draft: &ArrestDraft,
) -> Result<OrganizationId, ArrestError> {
    let character = state
        .world
        .get_character(draft.character)
        .ok_or(ArrestError::MissingCharacter(draft.character))?;
    if character.lifecycle() != Lifecycle::Active {
        return Err(ArrestError::InactiveCharacter(draft.character));
    }
    if let Some(existing) = state.legal.active_arrest_for_character(draft.character) {
        return Err(ArrestError::AlreadyDetained {
            character: draft.character,
            arrest: existing.id(),
        });
    }

    let investigation = state
        .legal
        .get_investigation(draft.investigation)
        .ok_or(ArrestError::MissingInvestigation(draft.investigation))?;
    if investigation.status() != InvestigationStatus::Active {
        return Err(ArrestError::InactiveInvestigation(draft.investigation));
    }
    if !investigation
        .subjects()
        .contains(&EntityRef::Character(draft.character))
    {
        return Err(ArrestError::CharacterNotSubject {
            character: draft.character,
            investigation: draft.investigation,
        });
    }
    let authority = investigation.owner();
    let authority_record = state
        .world
        .get_organization(authority)
        .ok_or(ArrestError::InvalidAuthority(authority))?;
    if authority_record.kind() != OrganizationKind::LawEnforcement
        || authority_record.lifecycle() != Lifecycle::Active
    {
        return Err(ArrestError::InvalidAuthority(authority));
    }

    if draft.evidence.is_empty() {
        return Err(ArrestError::NoEvidence);
    }
    for evidence_id in &draft.evidence {
        let evidence = state
            .legal
            .get_evidence(*evidence_id)
            .ok_or(ArrestError::MissingEvidence(*evidence_id))?;
        if evidence.investigation() != draft.investigation {
            return Err(ArrestError::EvidenceInvestigationMismatch {
                evidence: *evidence_id,
                investigation: draft.investigation,
            });
        }
        if evidence.custodian() != authority {
            return Err(ArrestError::EvidenceCustodianMismatch {
                evidence: *evidence_id,
                authority,
            });
        }
        if evidence.subject() != EntityRef::Character(draft.character) {
            return Err(ArrestError::EvidenceSubjectMismatch {
                evidence: *evidence_id,
                character: draft.character,
            });
        }
        if evidence.strength() == crate::legal::EvidenceStrength::Weak {
            return Err(ArrestError::InsufficientEvidence {
                evidence: *evidence_id,
                strength: evidence.strength(),
                reliability: evidence.reliability(),
            });
        }
    }

    validate_character_can_enter_custody(state, draft.character)?;
    Ok(authority)
}

fn validate_character_can_enter_custody(
    state: &AppState,
    character: CharacterId,
) -> Result<(), ArrestError> {
    if let Some(work) = state
        .legal
        .work_for_investigator(character)
        .find(|work| work.status() == InvestigationWorkStatus::Scheduled)
    {
        return Err(ArrestError::ScheduledInvestigationWork {
            character,
            work: work.id(),
        });
    }
    for operation in state.operations.operations() {
        if matches!(
            operation.status(),
            OperationStatus::Authorized
                | OperationStatus::InProgress
                | OperationStatus::AwaitingDecision
        ) && (operation.leader() == character
            || operation
                .roles()
                .values()
                .any(|participant| *participant == character))
        {
            return Err(ArrestError::ActiveOperationResponsibility {
                character,
                operation: operation.id(),
            });
        }
    }
    Ok(())
}

#[derive(Debug)]
pub struct ValidatedRelease {
    arrest: ArrestId,
    expected_version: u32,
}

impl ValidatedRelease {
    pub fn commit(self, state: &mut AppState) -> Result<(), ArrestError> {
        let record = state
            .legal
            .get_arrest(self.arrest)
            .ok_or(ArrestError::MissingArrest(self.arrest))?;
        if record.version() != self.expected_version {
            return Err(ArrestError::StaleArrest {
                arrest: self.arrest,
                expected: self.expected_version,
                found: record.version(),
            });
        }
        if record.status() != ArrestStatus::Detained {
            return Err(ArrestError::NotDetained(self.arrest));
        }
        state.legal.release_arrest(self.arrest, state.now());
        Ok(())
    }
}

pub fn validate_release_arrest(
    state: &AppState,
    arrest: ArrestId,
) -> Result<ValidatedRelease, ArrestError> {
    let record = state
        .legal
        .get_arrest(arrest)
        .ok_or(ArrestError::MissingArrest(arrest))?;
    if record.status() != ArrestStatus::Detained {
        return Err(ArrestError::NotDetained(arrest));
    }
    Ok(ValidatedRelease {
        arrest,
        expected_version: record.version(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::invariants::{validate_invariants, validate_state};
    use crate::core::persistence::{build_save, restore_save, SaveEnvelope};
    use crate::core::time::SimDuration;
    use crate::legal::investigation_system::{
        validate_add_evidence, validate_open_investigation, validate_transition_investigation,
        InvestigationError, InvestigationTransition,
    };
    use crate::legal::{
        Admissibility, EvidenceDraft, EvidenceKind, EvidenceReliability, EvidenceStrength,
        InvestigationDraft,
    };
    use crate::registry::Registry;
    use crate::world::world_system::{
        insert_character, insert_organization, validate_reassign_character, WorldError,
    };
    use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind};
    use std::collections::{BTreeMap, BTreeSet};

    struct Fixture {
        registry: Registry,
        state: AppState,
        police: OrganizationId,
        suspect: CharacterId,
        investigation: InvestigationId,
        evidence: EvidenceId,
    }

    fn fixture() -> Fixture {
        let registry = build_registry();
        let mut state = AppState::new(0xA22E_5701);
        let crew = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Custody Test Crew".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("crew should validate");
        let police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Custody Test Police".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("police should validate");
        let suspect = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Case Subject".to_owned(),
                organization: Some(crew),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("suspect should validate");
        let investigation = validate_open_investigation(
            &state,
            InvestigationDraft {
                owner: police,
                title: "Evidence-backed custody test".to_owned(),
                subjects: BTreeSet::from([EntityRef::Character(suspect)]),
            },
        )
        .expect("investigation should validate")
        .commit(&mut state)
        .expect("investigation should commit");
        let evidence = add_character_evidence(&mut state, police, investigation, suspect);
        Fixture {
            registry,
            state,
            police,
            suspect,
            investigation,
            evidence,
        }
    }

    fn add_character_evidence(
        state: &mut AppState,
        police: OrganizationId,
        investigation: InvestigationId,
        suspect: CharacterId,
    ) -> EvidenceId {
        validate_add_evidence(
            state,
            EvidenceDraft {
                investigation,
                custodian: police,
                subject: EntityRef::Character(suspect),
                origin: None,
                kind: EvidenceKind::Document,
                strength: EvidenceStrength::Strong,
                reliability: EvidenceReliability::HighlyReliable,
                admissibility: Admissibility::Admissible,
                discovered_at: state.now(),
            },
        )
        .expect("case evidence should validate")
        .commit(state)
        .expect("case evidence should commit")
    }

    fn arrest_fixture(fixture: &mut Fixture) -> ArrestId {
        validate_arrest(
            &fixture.state,
            ArrestDraft {
                character: fixture.suspect,
                investigation: fixture.investigation,
                evidence: BTreeSet::from([fixture.evidence]),
            },
        )
        .expect("evidence-backed arrest should validate")
        .commit(&mut fixture.state)
        .expect("evidence-backed arrest should commit")
    }

    #[test]
    fn arrest_and_release_are_durable_indexed_lifecycle_records() {
        let mut fixture = fixture();
        let arrest = arrest_fixture(&mut fixture);
        let record = fixture
            .state
            .legal()
            .get_arrest(arrest)
            .expect("arrest should persist");
        assert_eq!(record.status(), ArrestStatus::Detained);
        assert_eq!(record.version(), 1);
        assert_eq!(record.authority(), fixture.police);
        assert_eq!(record.evidence(), &BTreeSet::from([fixture.evidence]));
        assert_eq!(
            fixture
                .state
                .legal()
                .active_arrest_for_character(fixture.suspect)
                .map(|record| record.id()),
            Some(arrest)
        );
        assert_eq!(
            fixture
                .state
                .legal()
                .arrests_for_investigation(fixture.investigation)
                .count(),
            1
        );
        validate_state(&fixture.state).expect("detention state should validate");
        validate_invariants(&fixture.state);

        let envelope = build_save(&fixture.registry, &fixture.state)
            .expect("detention state should build a save envelope");
        let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
        let decoded: SaveEnvelope =
            bincode::deserialize(&bytes).expect("save envelope should deserialize");
        let mut restored = restore_save(&fixture.registry, decoded)
            .expect("detention state should restore with indexes intact");
        assert_eq!(
            restored
                .legal()
                .active_arrest_for_character(fixture.suspect)
                .map(|record| record.id()),
            Some(arrest)
        );
        validate_release_arrest(&restored, arrest)
            .expect("restored detention should remain releasable")
            .commit(&mut restored)
            .expect("restored detention release should commit");
        let rearrest = validate_arrest(
            &restored,
            ArrestDraft {
                character: fixture.suspect,
                investigation: fixture.investigation,
                evidence: BTreeSet::from([fixture.evidence]),
            },
        )
        .expect("released restored character should permit a later evidence-backed arrest")
        .commit(&mut restored)
        .expect("later restored arrest should commit with a fresh ID");
        assert_ne!(rearrest, arrest);
        assert_eq!(
            restored
                .legal()
                .arrests_for_character(fixture.suspect)
                .count(),
            2
        );
        assert_eq!(
            restored
                .legal()
                .active_arrest_for_character(fixture.suspect)
                .map(|record| record.id()),
            Some(rearrest)
        );
        validate_state(&restored).expect("restored re-arrest state should validate");
        validate_invariants(&restored);

        fixture.state.advance_clock(SimDuration::from_minutes(45));
        validate_release_arrest(&fixture.state, arrest)
            .expect("active detention should release")
            .commit(&mut fixture.state)
            .expect("release should commit");
        let released = fixture
            .state
            .legal()
            .get_arrest(arrest)
            .expect("released arrest history should persist");
        assert_eq!(released.status(), ArrestStatus::Released);
        assert_eq!(released.version(), 2);
        assert_eq!(released.released_at(), Some(fixture.state.now()));
        assert!(fixture
            .state
            .legal()
            .active_arrest_for_character(fixture.suspect)
            .is_none());
        assert_eq!(
            fixture
                .state
                .legal()
                .arrests_for_character(fixture.suspect)
                .count(),
            1
        );
        validate_state(&fixture.state).expect("released custody history should validate");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn arrest_validation_is_case_specific_and_stales_when_case_evidence_changes() {
        let mut fixture = fixture();
        let stale = validate_arrest(
            &fixture.state,
            ArrestDraft {
                character: fixture.suspect,
                investigation: fixture.investigation,
                evidence: BTreeSet::from([fixture.evidence]),
            },
        )
        .expect("initial arrest plan should validate");
        add_character_evidence(
            &mut fixture.state,
            fixture.police,
            fixture.investigation,
            fixture.suspect,
        );
        let error = stale
            .commit(&mut fixture.state)
            .expect_err("case mutation must stale a previously validated arrest");
        assert!(matches!(error, ArrestError::StaleInvestigation { .. }));
        assert!(fixture
            .state
            .legal()
            .active_arrest_for_character(fixture.suspect)
            .is_none());

        let second_case = validate_open_investigation(
            &fixture.state,
            InvestigationDraft {
                owner: fixture.police,
                title: "Separate case".to_owned(),
                subjects: BTreeSet::from([EntityRef::Character(fixture.suspect)]),
            },
        )
        .expect("second investigation should validate")
        .commit(&mut fixture.state)
        .expect("second investigation should commit");
        let foreign_evidence = add_character_evidence(
            &mut fixture.state,
            fixture.police,
            second_case,
            fixture.suspect,
        );
        let error = validate_arrest(
            &fixture.state,
            ArrestDraft {
                character: fixture.suspect,
                investigation: fixture.investigation,
                evidence: BTreeSet::from([foreign_evidence]),
            },
        )
        .expect_err("evidence from another case must not support this arrest");
        assert_eq!(
            error,
            ArrestError::EvidenceInvestigationMismatch {
                evidence: foreign_evidence,
                investigation: fixture.investigation,
            }
        );
        validate_state(&fixture.state).expect("rejected arrest attempts must preserve valid state");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn active_detention_blocks_case_shutdown_and_membership_escape_until_release() {
        let mut fixture = fixture();
        let arrest = arrest_fixture(&mut fixture);

        let transition_error = validate_transition_investigation(
            &fixture.state,
            fixture.investigation,
            InvestigationTransition::Close,
        )
        .expect_err("active detention must keep its source case open");
        assert_eq!(
            transition_error,
            InvestigationError::ActiveArrestBlocksTransition {
                investigation: fixture.investigation,
                arrest,
            }
        );
        let reassignment_error =
            validate_reassign_character(&fixture.state, fixture.suspect, None, None)
                .expect_err("detained character must not escape custody through reassignment");
        assert_eq!(
            reassignment_error,
            WorldError::ActiveArrestAssignment {
                character: fixture.suspect,
                arrest,
            }
        );

        validate_release_arrest(&fixture.state, arrest)
            .expect("detention should release")
            .commit(&mut fixture.state)
            .expect("release should commit");
        validate_transition_investigation(
            &fixture.state,
            fixture.investigation,
            InvestigationTransition::Close,
        )
        .expect("released custody no longer requires an active source case")
        .commit(&mut fixture.state)
        .expect("case close should commit after release");
        validate_reassign_character(&fixture.state, fixture.suspect, None, None)
            .expect("released character should permit ordinary membership changes")
            .commit(&mut fixture.state)
            .expect("membership change should commit after release");
        validate_state(&fixture.state).expect("post-release lifecycle state should validate");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn detention_preserves_formal_supervision_but_blocks_new_supervisory_work() {
        let mut fixture = fixture();
        let crew = fixture
            .state
            .world()
            .get_character(fixture.suspect)
            .and_then(|record| record.organization())
            .expect("suspect fixture should belong to the criminal organization");
        let direct_report = insert_character(
            &fixture.registry,
            &mut fixture.state,
            CharacterDraft {
                name: "Existing Direct Report".to_owned(),
                organization: Some(crew),
                supervisor: Some(fixture.suspect),
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("preexisting reporting line should validate");
        let unassigned = insert_character(
            &fixture.registry,
            &mut fixture.state,
            CharacterDraft {
                name: "Unassigned Member".to_owned(),
                organization: Some(crew),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("unassigned member should validate");

        let arrest = arrest_fixture(&mut fixture);
        assert_eq!(
            fixture
                .state
                .world()
                .direct_reports(fixture.suspect)
                .map(|record| record.id())
                .collect::<Vec<_>>(),
            vec![direct_report]
        );
        validate_state(&fixture.state)
            .expect("formal reporting lines may persist while a supervisor is detained");
        validate_invariants(&fixture.state);

        let error = validate_reassign_character(
            &fixture.state,
            unassigned,
            Some(crew),
            Some(fixture.suspect),
        )
        .expect_err("detained supervisor must not receive new reporting responsibility");
        assert_eq!(
            error,
            WorldError::DetainedSupervisor {
                supervisor: fixture.suspect,
                arrest,
            }
        );
        assert_eq!(
            fixture
                .state
                .world()
                .get_character(unassigned)
                .expect("rejected reassignment must retain the character")
                .supervisor(),
            None
        );
        validate_state(&fixture.state)
            .expect("rejected supervisory work must preserve valid state");
        validate_invariants(&fixture.state);
    }
}
