//! Confidential-source relationships and provenance-preserving disclosures into legal cases.

use crate::core::entity::EntityRef;
use crate::core::id::{
    CharacterId, InformantDisclosureId, InformantId, InformationId, InvestigationId, OrganizationId,
};
use crate::core::state::AppState;
use crate::intelligence::{KnowledgeHolder, Reliability, Specificity};
use crate::legal::{
    Admissibility, EvidenceAssessment, EvidenceConnection, EvidenceIdentity, EvidenceKind,
    EvidenceRecord, EvidenceReliability, EvidenceStrength, InformantDisclosureDraft,
    InformantDisclosureRecord, InformantDraft, InformantRecord, InformantStatus,
    InvestigationStatus,
};
use crate::world::{Lifecycle, OrganizationKind};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum InformantError {
    #[error("character {0} does not exist")]
    MissingCharacter(CharacterId),
    #[error("handler organization {0} does not exist")]
    MissingHandler(OrganizationId),
    #[error("organization {0} cannot handle confidential informants")]
    InvalidHandlerKind(OrganizationId),
    #[error("character {0} is not active")]
    InactiveCharacter(CharacterId),
    #[error("handler organization {0} is not active")]
    InactiveHandler(OrganizationId),
    #[error("character {character} belongs to handler organization {handler}")]
    CharacterBelongsToHandler {
        character: CharacterId,
        handler: OrganizationId,
    },
    #[error(
        "character {character} already has active informant relationship {informant} with handler {handler}"
    )]
    AlreadyActive {
        character: CharacterId,
        handler: OrganizationId,
        informant: InformantId,
    },
    #[error("informant relationship {0} does not exist")]
    MissingInformant(InformantId),
    #[error("informant relationship {0} is not active")]
    InactiveInformant(InformantId),
    #[error("investigation {0} does not exist")]
    MissingInvestigation(InvestigationId),
    #[error("investigation {0} is not active")]
    InactiveInvestigation(InvestigationId),
    #[error(
        "informant {informant} is handled by {handler}, which does not own investigation {investigation}"
    )]
    HandlerInvestigationMismatch {
        informant: InformantId,
        handler: OrganizationId,
        investigation: InvestigationId,
    },
    #[error("information record {0} does not exist")]
    MissingInformation(InformationId),
    #[error("information {information} is not personally held by informant character {character}")]
    InformationNotHeldByInformant {
        information: InformationId,
        character: CharacterId,
    },
    #[error(
        "information {information} already has disclosure {disclosure} in investigation {investigation}"
    )]
    DuplicateDisclosure {
        investigation: InvestigationId,
        information: InformationId,
        disclosure: InformantDisclosureId,
    },
    #[error(
        "character {character} changed after informant validation; expected version {expected}, found {found}"
    )]
    StaleCharacter {
        character: CharacterId,
        expected: u32,
        found: u32,
    },
    #[error(
        "informant {informant} changed after validation; expected version {expected}, found {found}"
    )]
    StaleInformant {
        informant: InformantId,
        expected: u32,
        found: u32,
    },
    #[error(
        "investigation {investigation} changed after disclosure validation; expected version {expected}, found {found}"
    )]
    StaleInvestigation {
        investigation: InvestigationId,
        expected: u32,
        found: u32,
    },
}

#[derive(Debug)]
pub struct ValidatedInformantEstablishment {
    draft: InformantDraft,
    expected_character_version: u32,
}

impl ValidatedInformantEstablishment {
    pub fn commit(self, state: &mut AppState) -> Result<InformantId, InformantError> {
        let character = state
            .world
            .get_character(self.draft.character)
            .ok_or(InformantError::MissingCharacter(self.draft.character))?;
        if character.version() != self.expected_character_version {
            return Err(InformantError::StaleCharacter {
                character: self.draft.character,
                expected: self.expected_character_version,
                found: character.version(),
            });
        }
        validate_establishment_dependencies(state, self.draft)?;
        let id = state.ids.next_informant();
        state.legal.insert_informant(InformantRecord {
            id,
            character: self.draft.character,
            handler: self.draft.handler,
            status: InformantStatus::Active,
            established_at: state.now(),
            terminated_at: None,
            version: 1,
        });
        Ok(id)
    }
}

pub fn validate_establish_informant(
    state: &AppState,
    draft: InformantDraft,
) -> Result<ValidatedInformantEstablishment, InformantError> {
    validate_establishment_dependencies(state, draft)?;
    let character = state
        .world
        .get_character(draft.character)
        .expect("validated informant character must exist");
    Ok(ValidatedInformantEstablishment {
        draft,
        expected_character_version: character.version(),
    })
}

fn validate_establishment_dependencies(
    state: &AppState,
    draft: InformantDraft,
) -> Result<(), InformantError> {
    let character = state
        .world
        .get_character(draft.character)
        .ok_or(InformantError::MissingCharacter(draft.character))?;
    if character.lifecycle() != Lifecycle::Active {
        return Err(InformantError::InactiveCharacter(draft.character));
    }
    validate_handler(state, draft.handler)?;
    if character.organization() == Some(draft.handler) {
        return Err(InformantError::CharacterBelongsToHandler {
            character: draft.character,
            handler: draft.handler,
        });
    }
    if let Some(existing) = state
        .legal
        .active_informant_for(draft.character, draft.handler)
    {
        return Err(InformantError::AlreadyActive {
            character: draft.character,
            handler: draft.handler,
            informant: existing.id(),
        });
    }
    Ok(())
}

fn validate_handler(state: &AppState, handler: OrganizationId) -> Result<(), InformantError> {
    let organization = state
        .world
        .get_organization(handler)
        .ok_or(InformantError::MissingHandler(handler))?;
    if !matches!(
        organization.kind(),
        OrganizationKind::LawEnforcement | OrganizationKind::LegalAuthority
    ) {
        return Err(InformantError::InvalidHandlerKind(handler));
    }
    if organization.lifecycle() != Lifecycle::Active {
        return Err(InformantError::InactiveHandler(handler));
    }
    Ok(())
}

#[derive(Debug)]
pub struct ValidatedInformantTermination {
    informant: InformantId,
    expected_version: u32,
}

impl ValidatedInformantTermination {
    pub fn commit(self, state: &mut AppState) -> Result<(), InformantError> {
        let informant = state
            .legal
            .get_informant(self.informant)
            .ok_or(InformantError::MissingInformant(self.informant))?;
        if informant.version() != self.expected_version {
            return Err(InformantError::StaleInformant {
                informant: self.informant,
                expected: self.expected_version,
                found: informant.version(),
            });
        }
        if informant.status() != InformantStatus::Active {
            return Err(InformantError::InactiveInformant(self.informant));
        }
        state.legal.terminate_informant(self.informant, state.now());
        Ok(())
    }
}

pub fn validate_terminate_informant(
    state: &AppState,
    informant: InformantId,
) -> Result<ValidatedInformantTermination, InformantError> {
    let record = state
        .legal
        .get_informant(informant)
        .ok_or(InformantError::MissingInformant(informant))?;
    if record.status() != InformantStatus::Active {
        return Err(InformantError::InactiveInformant(informant));
    }
    Ok(ValidatedInformantTermination {
        informant,
        expected_version: record.version(),
    })
}

#[derive(Debug)]
pub struct ValidatedInformantDisclosure {
    draft: InformantDisclosureDraft,
    expected_informant_version: u32,
    expected_investigation_version: u32,
}

impl ValidatedInformantDisclosure {
    pub fn commit(self, state: &mut AppState) -> Result<InformantDisclosureId, InformantError> {
        let informant = state
            .legal
            .get_informant(self.draft.informant)
            .ok_or(InformantError::MissingInformant(self.draft.informant))?;
        if informant.version() != self.expected_informant_version {
            return Err(InformantError::StaleInformant {
                informant: self.draft.informant,
                expected: self.expected_informant_version,
                found: informant.version(),
            });
        }
        let investigation = state
            .legal
            .get_investigation(self.draft.investigation)
            .ok_or(InformantError::MissingInvestigation(
                self.draft.investigation,
            ))?;
        if investigation.version() != self.expected_investigation_version {
            return Err(InformantError::StaleInvestigation {
                investigation: self.draft.investigation,
                expected: self.expected_investigation_version,
                found: investigation.version(),
            });
        }
        validate_disclosure_dependencies(state, self.draft)?;

        let informant = state
            .legal
            .get_informant(self.draft.informant)
            .expect("validated informant must still exist");
        let handler = informant.handler();
        let character = informant.character();
        let information = state
            .intelligence
            .get_information(self.draft.source_information)
            .expect("validated source information must still exist");
        let subject = information.subject();
        let strength = informant_strength(information.specificity());
        let reliability = informant_reliability(information.reliability());
        let disclosed_at = state.now();

        let evidence_id = state.ids.next_evidence();
        let disclosure_id = state.ids.next_informant_disclosure();
        let evidence = EvidenceRecord {
            identity: EvidenceIdentity {
                id: evidence_id,
                investigation: self.draft.investigation,
                custodian: handler,
            },
            connection: EvidenceConnection {
                subject,
                origin: None,
                source: Some(EntityRef::Character(character)),
                derived_from: BTreeSet::new(),
            },
            assessment: EvidenceAssessment {
                kind: EvidenceKind::InformantStatement,
                strength,
                reliability,
                admissibility: Admissibility::Unknown,
            },
            discovered_at: disclosed_at,
        };
        let disclosure = InformantDisclosureRecord {
            id: disclosure_id,
            informant: self.draft.informant,
            investigation: self.draft.investigation,
            source_information: self.draft.source_information,
            evidence: evidence_id,
            disclosed_at,
        };
        state
            .legal
            .insert_informant_disclosure(evidence, disclosure);
        Ok(disclosure_id)
    }
}

pub fn validate_record_informant_disclosure(
    state: &AppState,
    draft: InformantDisclosureDraft,
) -> Result<ValidatedInformantDisclosure, InformantError> {
    validate_disclosure_dependencies(state, draft)?;
    let informant = state
        .legal
        .get_informant(draft.informant)
        .expect("validated informant must exist");
    let investigation = state
        .legal
        .get_investigation(draft.investigation)
        .expect("validated investigation must exist");
    Ok(ValidatedInformantDisclosure {
        draft,
        expected_informant_version: informant.version(),
        expected_investigation_version: investigation.version(),
    })
}

fn validate_disclosure_dependencies(
    state: &AppState,
    draft: InformantDisclosureDraft,
) -> Result<(), InformantError> {
    let informant = state
        .legal
        .get_informant(draft.informant)
        .ok_or(InformantError::MissingInformant(draft.informant))?;
    if informant.status() != InformantStatus::Active {
        return Err(InformantError::InactiveInformant(draft.informant));
    }
    let character = state
        .world
        .get_character(informant.character())
        .ok_or(InformantError::MissingCharacter(informant.character()))?;
    if character.lifecycle() != Lifecycle::Active {
        return Err(InformantError::InactiveCharacter(informant.character()));
    }
    validate_handler(state, informant.handler())?;

    let investigation = state
        .legal
        .get_investigation(draft.investigation)
        .ok_or(InformantError::MissingInvestigation(draft.investigation))?;
    if investigation.status() != InvestigationStatus::Active {
        return Err(InformantError::InactiveInvestigation(draft.investigation));
    }
    if investigation.owner() != informant.handler() {
        return Err(InformantError::HandlerInvestigationMismatch {
            informant: draft.informant,
            handler: informant.handler(),
            investigation: draft.investigation,
        });
    }

    let information = state
        .intelligence
        .get_information(draft.source_information)
        .ok_or(InformantError::MissingInformation(draft.source_information))?;
    if information.holder() != KnowledgeHolder::Character(informant.character()) {
        return Err(InformantError::InformationNotHeldByInformant {
            information: draft.source_information,
            character: informant.character(),
        });
    }
    if let Some(existing) = state
        .legal
        .informant_disclosure_for_case_information(draft.investigation, draft.source_information)
    {
        return Err(InformantError::DuplicateDisclosure {
            investigation: draft.investigation,
            information: draft.source_information,
            disclosure: existing.id(),
        });
    }
    Ok(())
}

pub(crate) const fn informant_strength(specificity: Specificity) -> EvidenceStrength {
    match specificity {
        Specificity::Vague => EvidenceStrength::Weak,
        Specificity::General => EvidenceStrength::Corroborating,
        Specificity::Specific => EvidenceStrength::Strong,
        Specificity::Precise => EvidenceStrength::Direct,
    }
}

pub(crate) const fn informant_reliability(reliability: Reliability) -> EvidenceReliability {
    match reliability {
        Reliability::Unknown | Reliability::Unreliable => EvidenceReliability::Questionable,
        Reliability::Mixed => EvidenceReliability::Mixed,
        Reliability::GenerallyReliable => EvidenceReliability::Credible,
        Reliability::DirectAccess => EvidenceReliability::HighlyReliable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::invariants::{validate_invariants, validate_state};
    use crate::core::persistence::{build_save, restore_save, SaveEnvelope};
    use crate::intelligence::intelligence_system::validate_record_information;
    use crate::intelligence::{
        InformationDraft, InformationSourceKind, InformationTopic, Reliability, Specificity,
    };
    use crate::legal::investigation_system::{
        validate_add_evidence, validate_open_investigation, InvestigationError,
    };
    use crate::legal::{EvidenceDraft, InvestigationDraft};
    use crate::world::world_system::{
        insert_character, insert_organization, validate_reassign_character, WorldError,
    };
    use crate::world::{AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind};
    use std::collections::{BTreeMap, BTreeSet};

    struct Fixture {
        state: AppState,
        police: OrganizationId,
        criminal: OrganizationId,
        member: CharacterId,
        investigation: InvestigationId,
    }

    fn fixture() -> Fixture {
        let registry = build_registry();
        let mut state = AppState::new(0x1F0A_1934);
        let police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Confidential Source Bureau".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("police fixture should validate");
        let criminal = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Harbor Crew".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("criminal fixture should validate");
        let member = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Leo Trent".to_owned(),
                organization: Some(criminal),
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("member fixture should validate");
        let investigation = validate_open_investigation(
            &state,
            InvestigationDraft {
                owner: police,
                title: "Harbor organization inquiry".to_owned(),
                subjects: BTreeSet::from([EntityRef::Organization(criminal)]),
            },
        )
        .expect("investigation fixture should validate")
        .commit(&mut state)
        .expect("investigation fixture should commit");
        Fixture {
            state,
            police,
            criminal,
            member,
            investigation,
        }
    }

    fn record_personal_information(fixture: &mut Fixture) -> InformationId {
        validate_record_information(
            &fixture.state,
            InformationDraft {
                holder: KnowledgeHolder::Character(fixture.member),
                source_kind: InformationSourceKind::DirectObservation,
                topic: InformationTopic::Personnel,
                source_entity: None,
                subject: EntityRef::Organization(fixture.criminal),
                observed_at: fixture.state.now(),
                reliability: Reliability::GenerallyReliable,
                specificity: Specificity::Specific,
                summary: "The member directly observed the crew's current personnel structure."
                    .to_owned(),
            },
        )
        .expect("personal information should validate")
        .commit(&mut fixture.state)
    }

    #[test]
    fn disclosure_requires_personal_knowledge_and_creates_provenance_evidence() {
        let mut fixture = fixture();
        let informant = validate_establish_informant(
            &fixture.state,
            InformantDraft {
                character: fixture.member,
                handler: fixture.police,
            },
        )
        .expect("informant establishment should validate")
        .commit(&mut fixture.state)
        .expect("informant establishment should commit");
        assert_eq!(
            validate_reassign_character(
                &fixture.state,
                fixture.member,
                Some(fixture.police),
                None,
            )
            .expect_err("an active source must be terminated before joining its handler"),
            WorldError::ActiveInformantHandlerAssignment {
                character: fixture.member,
                handler: fixture.police,
                informant,
            }
        );
        let organization_information = validate_record_information(
            &fixture.state,
            InformationDraft {
                holder: KnowledgeHolder::Organization(fixture.police),
                source_kind: InformationSourceKind::DirectObservation,
                topic: InformationTopic::Personnel,
                source_entity: None,
                subject: EntityRef::Organization(fixture.criminal),
                observed_at: fixture.state.now(),
                reliability: Reliability::GenerallyReliable,
                specificity: Specificity::Specific,
                summary: "The bureau has separate knowledge about the crew's personnel.".to_owned(),
            },
        )
        .expect("organization information should validate")
        .commit(&mut fixture.state);
        assert_eq!(
            validate_record_informant_disclosure(
                &fixture.state,
                InformantDisclosureDraft {
                    informant,
                    investigation: fixture.investigation,
                    source_information: organization_information,
                },
            )
            .expect_err("informants cannot disclose knowledge held only by their handler"),
            InformantError::InformationNotHeldByInformant {
                information: organization_information,
                character: fixture.member,
            }
        );
        let information = record_personal_information(&mut fixture);

        let disclosure = validate_record_informant_disclosure(
            &fixture.state,
            InformantDisclosureDraft {
                informant,
                investigation: fixture.investigation,
                source_information: information,
            },
        )
        .expect("personal informant knowledge should be disclosable")
        .commit(&mut fixture.state)
        .expect("validated disclosure should commit");

        let disclosure_record = fixture
            .state
            .legal()
            .get_informant_disclosure(disclosure)
            .expect("disclosure should persist");
        let evidence = fixture
            .state
            .legal()
            .get_evidence(disclosure_record.evidence())
            .expect("informant evidence should persist");
        assert_eq!(evidence.kind(), EvidenceKind::InformantStatement);
        assert_eq!(evidence.strength(), EvidenceStrength::Strong);
        assert_eq!(evidence.reliability(), EvidenceReliability::Credible);
        assert_eq!(evidence.admissibility(), Admissibility::Unknown);
        assert_eq!(
            evidence.source(),
            Some(EntityRef::Character(fixture.member))
        );
        assert_eq!(
            evidence.subject(),
            EntityRef::Organization(fixture.criminal)
        );
        assert_eq!(disclosure_record.source_information(), information);
        assert_eq!(
            fixture
                .state
                .legal()
                .informant_disclosures_from_information(information)
                .map(|record| record.id())
                .collect::<Vec<_>>(),
            vec![disclosure]
        );
        assert!(matches!(
            validate_record_informant_disclosure(
                &fixture.state,
                InformantDisclosureDraft {
                    informant,
                    investigation: fixture.investigation,
                    source_information: information,
                },
            ),
            Err(InformantError::DuplicateDisclosure {
                disclosure: existing,
                ..
            }) if existing == disclosure
        ));
        validate_state(&fixture.state).expect("canonical disclosure state should validate");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn generic_evidence_path_cannot_forge_informant_statement() {
        let fixture = fixture();
        let error = match validate_add_evidence(
            &fixture.state,
            EvidenceDraft {
                investigation: fixture.investigation,
                custodian: fixture.police,
                subject: EntityRef::Organization(fixture.criminal),
                origin: None,
                kind: EvidenceKind::InformantStatement,
                strength: EvidenceStrength::Strong,
                reliability: EvidenceReliability::Credible,
                admissibility: Admissibility::Unknown,
                discovered_at: fixture.state.now(),
            },
        ) {
            Ok(_) => panic!("generic evidence path must reject informant statements"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            InvestigationError::InformantStatementRequiresDisclosure
        );
        assert_eq!(
            fixture
                .state
                .legal()
                .evidence_of_kind(EvidenceKind::InformantStatement)
                .count(),
            0
        );
        validate_invariants(&fixture.state);
    }

    #[test]
    fn disclosure_token_rejects_case_change_without_partial_mutation() {
        let mut fixture = fixture();
        let informant = validate_establish_informant(
            &fixture.state,
            InformantDraft {
                character: fixture.member,
                handler: fixture.police,
            },
        )
        .expect("informant establishment should validate")
        .commit(&mut fixture.state)
        .expect("informant establishment should commit");
        let information = record_personal_information(&mut fixture);
        let stale = validate_record_informant_disclosure(
            &fixture.state,
            InformantDisclosureDraft {
                informant,
                investigation: fixture.investigation,
                source_information: information,
            },
        )
        .expect("disclosure should initially validate");

        validate_add_evidence(
            &fixture.state,
            EvidenceDraft {
                investigation: fixture.investigation,
                custodian: fixture.police,
                subject: EntityRef::Organization(fixture.criminal),
                origin: None,
                kind: EvidenceKind::Surveillance,
                strength: EvidenceStrength::Weak,
                reliability: EvidenceReliability::Questionable,
                admissibility: Admissibility::Unknown,
                discovered_at: fixture.state.now(),
            },
        )
        .expect("independent case mutation should validate")
        .commit(&mut fixture.state)
        .expect("independent case mutation should commit");

        assert!(matches!(
            stale.commit(&mut fixture.state),
            Err(InformantError::StaleInvestigation { .. })
        ));
        assert_eq!(
            fixture
                .state
                .legal()
                .evidence_of_kind(EvidenceKind::InformantStatement)
                .count(),
            0
        );
        assert_eq!(
            fixture
                .state
                .legal()
                .informant_disclosures_from_information(information)
                .count(),
            0
        );
        validate_state(&fixture.state).expect("stale rejection should leave valid state");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn termination_is_versioned_and_save_round_trip_preserves_history() {
        let registry = build_registry();
        let mut fixture = fixture();
        let informant = validate_establish_informant(
            &fixture.state,
            InformantDraft {
                character: fixture.member,
                handler: fixture.police,
            },
        )
        .expect("informant establishment should validate")
        .commit(&mut fixture.state)
        .expect("informant establishment should commit");
        let information = record_personal_information(&mut fixture);
        let disclosure = validate_record_informant_disclosure(
            &fixture.state,
            InformantDisclosureDraft {
                informant,
                investigation: fixture.investigation,
                source_information: information,
            },
        )
        .expect("disclosure should validate")
        .commit(&mut fixture.state)
        .expect("disclosure should commit");

        let stale_termination = validate_terminate_informant(&fixture.state, informant)
            .expect("termination should validate");
        validate_terminate_informant(&fixture.state, informant)
            .expect("second termination token should validate against same version")
            .commit(&mut fixture.state)
            .expect("first committed termination should succeed");
        assert!(matches!(
            stale_termination.commit(&mut fixture.state),
            Err(InformantError::StaleInformant { .. })
        ));
        assert!(fixture
            .state
            .legal()
            .active_informant_for(fixture.member, fixture.police)
            .is_none());
        assert_eq!(
            fixture
                .state
                .legal()
                .get_informant(informant)
                .expect("historical relationship should persist")
                .status(),
            InformantStatus::Terminated
        );

        let replacement = validate_establish_informant(
            &fixture.state,
            InformantDraft {
                character: fixture.member,
                handler: fixture.police,
            },
        )
        .expect("terminated relationship should permit later re-establishment")
        .commit(&mut fixture.state)
        .expect("replacement relationship should commit");
        assert_ne!(replacement, informant);

        let envelope = build_save(&registry, &fixture.state).expect("informant state should save");
        let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
        let decoded: SaveEnvelope =
            bincode::deserialize(&bytes).expect("save envelope should deserialize");
        let restored = restore_save(&registry, decoded).expect("informant save should restore");
        assert_eq!(
            restored
                .legal()
                .get_informant(informant)
                .expect("terminated relationship should survive save")
                .status(),
            InformantStatus::Terminated
        );
        assert!(restored.legal().get_informant(replacement).is_some());
        assert_eq!(
            restored
                .legal()
                .get_informant_disclosure(disclosure)
                .expect("disclosure should survive save")
                .source_information(),
            information
        );
        validate_invariants(&restored);
    }
}
