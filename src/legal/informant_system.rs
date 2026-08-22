//! Confidential-source relationships and provenance-preserving disclosures into legal cases.

use crate::core::entity::EntityRef;
use crate::core::id::{
    CharacterId, IdExhaustionError, IdKind, InformantDisclosureId, InformantId, InformationId,
    InvestigationId, OperationId, OrganizationId,
};
use crate::core::state::AppState;
use crate::intelligence::{KnowledgeHolder, Reliability, Specificity};
use crate::legal::{
    Admissibility, ArrestStatus, EvidenceAssessment, EvidenceConnection, EvidenceIdentity,
    EvidenceKind, EvidenceRecord, EvidenceReliability, EvidenceStrength, InformantDisclosureDraft,
    InformantDisclosureRecord, InformantDraft, InformantRecord, InformantStatus,
    InvestigationStatus,
};
use crate::world::{Lifecycle, OrganizationKind};
use std::collections::{BTreeMap, BTreeSet};
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
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
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
        let id = state.ids.next_informant()?;
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
        state
            .ids
            .reserve_many(&[(IdKind::Evidence, 1), (IdKind::InformantDisclosure, 1)])?;
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

        let evidence_id = state.ids.next_evidence()?;
        let disclosure_id = state.ids.next_informant_disclosure()?;
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
            .insert_informant_disclosure(evidence, disclosure, disclosed_at);
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

/// A detained member gets exactly one recruitment decision, one authored cadence after the
/// arrest. No extra per-arrest state is needed: the decision instant is a pure function of
/// `arrested_at`, and the single draw consumes the state-owned investigation stream. The
/// equality below relies on the canonical tick advancing exactly one simulated minute per
/// call (`core::simulation::run_tick`); no adapter may fast-forward across minutes.
pub(crate) const RECRUITMENT_DECISION_OFFSET_MINUTES: u64 = 1_440;
/// Base flip chance in percent; fear of prison (the character's Safety drive) adds up to
/// 50 points on top.
const BASE_FLIP_CHANCE_PERCENT: u32 = 25;

/// Runs the police institution's detainee-to-informant pipeline: exactly one recruitment
/// draw per detained criminal member, one cadence window after their arrest. Members with
/// something personal to hide behind stay quiet; scared ones talk.
pub fn apply_detainee_informant_recruitment(
    state: &mut AppState,
) -> Result<Vec<InformantId>, InformantError> {
    use crate::world::OrganizationKind as OrgKind;

    let now = state.now();
    let candidates: Vec<(CharacterId, OrganizationId)> = state
        .legal
        .arrests()
        .filter(|arrest| arrest.status() == ArrestStatus::Detained)
        .filter_map(|arrest| {
            let handler = arrest.authority();
            let character = arrest.character();
            let record = state.world.get_character(character)?;
            // Only members of criminal organizations have an organization to inform on,
            // and only while they still belong to one.
            let org = record.organization()?;
            let org_record = state.world.get_organization(org)?;
            if org_record.kind() != OrgKind::Criminal {
                return None;
            }
            if org == handler {
                return None;
            }
            // An informant already working this handler keeps that arrangement; a second
            // establishment would be rejected as a duplicate, so no new decision is drawn.
            if state
                .legal
                .active_informant_for(character, handler)
                .is_some()
            {
                return None;
            }
            let minutes_in_custody = now
                .as_minutes()
                .saturating_sub(arrest.arrested_at().as_minutes());
            (minutes_in_custody == RECRUITMENT_DECISION_OFFSET_MINUTES)
                .then_some((character, handler))
        })
        .collect();

    let mut recruited = Vec::new();
    for (character, handler) in candidates {
        let safety = state
            .world
            .get_character(character)
            .and_then(|record| record.drive(crate::world::DriveKind::Safety))
            .map(|rating| u32::from(rating.value()))
            .unwrap_or(0);
        let chance = BASE_FLIP_CHANCE_PERCENT + safety / 2;
        let roll = {
            let rng = state.investigation_rng_mut();
            crate::core::simulation::draw_index(rng, 100)
                .expect("percentile draw over a nonempty 1..=100 range cannot fail")
        };
        if roll as u32 >= chance {
            continue;
        }
        let informant = validate_establish_informant(state, InformantDraft { character, handler })?
            .commit(state)?;
        recruited.push(informant);
    }
    Ok(recruited)
}

/// Active informants disclose what they personally know into their handler's active cases:
/// each piece of personally-held information whose subject matches a case's origin operation
/// is disclosed at most once (the disclosure index rejects duplicates). This is what makes an
/// informant more than a flag: their knowledge becomes InformantStatement evidence.
pub fn apply_informant_disclosures(
    state: &mut AppState,
) -> Result<Vec<InformantDisclosureId>, InformantError> {
    // Active cases owned by each handler, keyed by their origin operation. Built once per
    // pass in investigation-id order so the smallest matching case id wins deterministically.
    let mut cases_by_handler_origin: BTreeMap<
        OrganizationId,
        BTreeMap<OperationId, InvestigationId>,
    > = BTreeMap::new();
    for investigation in state.legal.investigations() {
        if investigation.status() != InvestigationStatus::Active {
            continue;
        }
        if let Some(origin) = investigation.origin_operation() {
            cases_by_handler_origin
                .entry(investigation.owner())
                .or_default()
                .entry(origin)
                .or_insert(investigation.id());
        }
    }

    let candidates: Vec<(InformantId, InformationId, InvestigationId)> = state
        .legal
        .informants()
        .filter(|informant| informant.status() == InformantStatus::Active)
        .flat_map(|informant| {
            let handler = informant.handler();
            let character = informant.character();
            let mut pairs = Vec::new();
            for information in state
                .intelligence
                .information_for_holder(KnowledgeHolder::Character(character))
            {
                let EntityRef::Operation(operation) = information.subject() else {
                    continue;
                };
                if let Some(&investigation) = cases_by_handler_origin
                    .get(&handler)
                    .and_then(|cases| cases.get(&operation))
                {
                    // Skip knowledge already traded into this case; the disclosure index is
                    // the authority on what has been disclosed.
                    let already_disclosed = state
                        .legal
                        .informant_disclosure_for_case_information(investigation, information.id())
                        .is_some();
                    if !already_disclosed {
                        pairs.push((informant.id(), information.id(), investigation));
                    }
                }
            }
            pairs
        })
        .collect();

    let mut disclosures = Vec::new();
    for (informant, information, investigation) in candidates {
        let disclosure = validate_record_informant_disclosure(
            state,
            InformantDisclosureDraft {
                informant,
                investigation,
                source_information: information,
            },
        )?
        .commit(state)?;
        disclosures.push(disclosure);
    }
    Ok(disclosures)
}

#[cfg(test)]
mod tests;
