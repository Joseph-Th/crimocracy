//! Confidential-source relationships and provenance-preserving disclosures into legal cases.

use crate::core::entity::EntityRef;
use crate::core::id::{
    CharacterId, IdExhaustionError, IdKind, InformantDisclosureId, InformantId, InformationId,
    InvestigationId, OrganizationId,
};
use crate::core::state::AppState;
use crate::core::version::{VersionCapacityError, ensure_version_can_advance};
use crate::intelligence::{KnowledgeHolder, Reliability, Specificity};
use crate::legal::{
    Admissibility, EvidenceAssessment, EvidenceConnection, EvidenceIdentity, EvidenceKind,
    EvidenceRecord, EvidenceReliability, EvidenceStrength, InformantDisclosureDraft,
    InformantDisclosureRecord, InformantDraft, InformantRecord, InvestigationStatus,
};
use crate::registry::Registry;
use crate::world::OrganizationKind;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum InformantError {
    #[error("character {0} does not exist")]
    MissingCharacter(CharacterId),
    #[error("character organization {0} does not exist")]
    MissingOrganization(OrganizationId),
    #[error("handler organization {0} does not exist")]
    MissingHandler(OrganizationId),
    #[error("organization {0} cannot handle confidential informants")]
    InvalidHandlerKind(OrganizationId),
    #[error("character {character} belongs to handler organization {handler}")]
    CharacterBelongsToHandler {
        character: CharacterId,
        handler: OrganizationId,
    },
    #[error(
        "character {character} already has informant relationship {informant} with handler {handler}"
    )]
    AlreadyInformant {
        character: CharacterId,
        handler: OrganizationId,
        informant: InformantId,
    },
    #[error("informant relationship {0} does not exist")]
    MissingInformant(InformantId),
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
        "information {information} about {subject:?} is unrelated to investigation {investigation}"
    )]
    InformationCaseMismatch {
        information: InformationId,
        subject: EntityRef,
        investigation: InvestigationId,
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
        "investigation {investigation} changed after disclosure validation; expected version {expected}, found {found}"
    )]
    StaleInvestigation {
        investigation: InvestigationId,
        expected: u32,
        found: u32,
    },
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
    #[error(transparent)]
    VersionCapacity(#[from] VersionCapacityError),
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
            established_at: state.now(),
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
    validate_handler(state, draft.handler)?;
    if character.organization() == Some(draft.handler) {
        return Err(InformantError::CharacterBelongsToHandler {
            character: draft.character,
            handler: draft.handler,
        });
    }
    if let Some(existing) = state.legal.informant_for(draft.character, draft.handler) {
        return Err(InformantError::AlreadyInformant {
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
    Ok(())
}

#[derive(Debug)]
pub struct ValidatedInformantDisclosure {
    draft: InformantDisclosureDraft,
    expected_investigation_version: u32,
}

impl ValidatedInformantDisclosure {
    pub fn commit(self, state: &mut AppState) -> Result<InformantDisclosureId, InformantError> {
        state
            .ids
            .reserve_many(&[(IdKind::Evidence, 1), (IdKind::InformantDisclosure, 1)])?;
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
        ensure_version_can_advance(investigation.version(), "investigation")?;
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

        let evidence_id = state
            .ids
            .next_evidence()
            .expect("informant evidence ID was preflighted before mutation");
        let disclosure_id = state
            .ids
            .next_informant_disclosure()
            .expect("informant-disclosure ID was preflighted before mutation");
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
    let investigation = state
        .legal
        .get_investigation(draft.investigation)
        .expect("validated investigation must exist");
    ensure_version_can_advance(investigation.version(), "investigation")?;
    Ok(ValidatedInformantDisclosure {
        draft,
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
    let _ = state
        .world
        .get_character(informant.character())
        .ok_or(InformantError::MissingCharacter(informant.character()))?;
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
    if !information_is_relevant_to_investigation(information, investigation) {
        return Err(InformantError::InformationCaseMismatch {
            information: draft.source_information,
            subject: information.subject(),
            investigation: draft.investigation,
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

/// A confidential source may contribute only facts that actually belong in the target case.
/// Origin-linked cases accept information about their originating event or enterprise; every
/// case also accepts information about an entity it already tracks as a subject. Keeping this
/// predicate shared by validation, automation, and restore prevents unrelated personal knowledge
/// from being converted into case evidence merely because the same institution owns both facts.
pub(crate) fn information_is_relevant_to_investigation(
    information: &crate::intelligence::InformationRecord,
    investigation: &crate::legal::InvestigationRecord,
) -> bool {
    investigation.origin() == Some(information.subject())
        || investigation.subjects().contains(&information.subject())
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
/// arrest (read from the registry's legal configuration). No extra per-arrest state is
/// needed: the decision instant is a pure function of `arrested_at`, and the single draw
/// consumes the state-owned investigation stream. The equality below relies on the canonical
/// tick advancing exactly one simulated minute per call (`core::simulation::run_tick`); no
/// adapter may fast-forward across minutes.
/// Runs the police institution's detainee-to-informant pipeline: exactly one recruitment
/// draw per detained criminal member, one cadence window after their arrest. Members who are
/// not sufficiently afraid stay quiet; stronger Safety pressure makes cooperation likelier,
/// while active counsel lowers the chance that custodial pressure becomes cooperation.
pub(crate) fn apply_detainee_informant_recruitment(
    registry: &Registry,
    state: &mut AppState,
) -> Result<Vec<InformantId>, InformantError> {
    use crate::world::OrganizationKind as OrgKind;

    let decision_delay = registry.legal().informant_decision_delay().as_minutes();
    let now = state.now();
    // The decision instant is a pure function of `arrested_at`, so the cheap timing gate
    // runs first: a detainee not reaching their decision minute this tick skips every
    // record lookup below. Predicates are pure reads, so evaluating them in this order
    // selects exactly the same candidates.
    let due_arrests: Vec<_> = state
        .legal
        .detained_arrests()
        .filter(|arrest| {
            let minutes_in_custody = now
                .as_minutes()
                .saturating_sub(arrest.arrested_at().as_minutes());
            // Exact equality is safe because the canonical pipeline advances exactly one
            // minute per tick and this pass runs every tick: each detention reaches its
            // decision minute under observation exactly once. A batched or skipped pass
            // would need a persisted decided-marker instead.
            minutes_in_custody == u64::from(decision_delay)
        })
        .map(|arrest| (arrest.id(), arrest.character(), arrest.authority()))
        .collect();

    let mut candidates = Vec::new();
    for (arrest, character, handler) in due_arrests {
        let record = state
            .world
            .get_character(character)
            .ok_or(InformantError::MissingCharacter(character))?;
        // Only members of criminal organizations have an organization to inform on, and only
        // while they still belong to one. Being independent is a legitimate non-candidate;
        // pointing at a missing organization is broken authoritative state and must not erase
        // the detainee's single scheduled decision silently.
        let Some(organization) = record.organization() else {
            continue;
        };
        let organization_record = state
            .world
            .get_organization(organization)
            .ok_or(InformantError::MissingOrganization(organization))?;
        if organization_record.kind() != OrgKind::Criminal || organization == handler {
            continue;
        }
        // An informant already working this handler keeps that arrangement; a second
        // establishment would be rejected as a duplicate, so no new decision is drawn.
        if state.legal.informant_for(character, handler).is_some() {
            continue;
        }
        candidates.push((arrest, character, handler));
    }

    let mut recruited = Vec::new();
    for (arrest, character, handler) in candidates {
        let safety = state
            .world
            .get_character(character)
            .and_then(|record| record.drive(crate::world::DriveKind::Safety))
            .map(|rating| u32::from(rating.value()))
            .unwrap_or(0);
        let chance = resolve_informant_flip_chance(
            registry.legal(),
            safety,
            state
                .legal
                .active_representation_for_arrest(arrest)
                .is_some(),
        );
        // Draw speculatively so an ordinary failed flip still consumes its authored decision
        // draw, while a successful flip only publishes that advanced RNG state after the
        // establishment commits. Allocation or freshness failure therefore rejects without
        // perturbing future investigation randomness.
        let validated = validate_establish_informant(state, InformantDraft { character, handler })?;
        let mut advanced_rng = state.investigation_rng_mut().clone();
        let roll = crate::core::simulation::draw_index(&mut advanced_rng, 100)
            .expect("percentile draw over the nonempty 0..100 index range cannot fail");
        if roll as u32 >= chance {
            *state.investigation_rng_mut() = advanced_rng;
            continue;
        }
        let informant = validated.commit(state)?;
        *state.investigation_rng_mut() = advanced_rng;
        recruited.push(informant);
    }
    Ok(recruited)
}

fn resolve_informant_flip_chance(
    legal: crate::registry::LegalConfigDefinition,
    safety: u32,
    represented: bool,
) -> u32 {
    let safety_bonus =
        safety.saturating_mul(u32::from(legal.informant_safety_bonus_percent())) / 100;
    let chance = u32::from(legal.informant_base_flip_chance_percent()) + safety_bonus;
    if represented {
        chance.saturating_sub(u32::from(legal.represented_informant_reduction_percent()))
    } else {
        chance
    }
}

/// Informants disclose personally held information relevant to their handler's active
/// cases. Relevance uses the same subject/origin predicate as the canonical disclosure validator,
/// so institution-authored cases and enterprise-origin vice inquiries are not arbitrarily excluded.
/// Each case-information pair is disclosed at most once by the disclosure index.
pub(crate) fn apply_informant_disclosures(
    state: &mut AppState,
) -> Result<Vec<InformantDisclosureId>, InformantError> {
    // Disclosures need an informant relationship on one side and an active case on the
    // other. With no informant the handler-to-case view could never be consulted,
    // so quiet custody ticks skip building it entirely.
    if !state.legal.has_informants() {
        return Ok(Vec::new());
    }
    // Active cases owned by each handler, keyed by entities that make information relevant to
    // the case. A held fact is institutionally available to every matching active file in the
    // same pass. Serializing that propagation one case per minute would make case ID determine
    // which file gets refreshed before same-minute cold-case decay.
    let mut cases_by_handler_subject: BTreeMap<
        OrganizationId,
        BTreeMap<EntityRef, BTreeSet<InvestigationId>>,
    > = BTreeMap::new();
    for investigation in state.legal.active_investigations() {
        let cases = cases_by_handler_subject
            .entry(investigation.owner())
            .or_default();
        if let Some(origin) = investigation.origin() {
            cases.entry(origin).or_default().insert(investigation.id());
        }
        for subject in investigation.subjects() {
            cases
                .entry(*subject)
                .or_default()
                .insert(investigation.id());
        }
    }

    let mut candidates: Vec<(InformantId, InformationId, InvestigationId)> = Vec::new();
    for (handler, cases_by_subject) in &cases_by_handler_subject {
        for informant in state.legal.informants_for_handler(*handler) {
            let holder = KnowledgeHolder::Character(informant.character());
            for (subject, investigations) in cases_by_subject {
                for information in state
                    .intelligence
                    .information_for_holder_subject(holder, *subject)
                {
                    for investigation in investigations.iter().copied().filter(|case| {
                        state
                            .legal
                            .informant_disclosure_for_case_information(*case, information.id())
                            .is_none()
                    }) {
                        candidates.push((informant.id(), information.id(), investigation));
                    }
                }
            }
        }
    }
    // Stable commit order is informant id, then information id, then case id. IDs order equal
    // facts only; they no longer decide whether another matching case receives the fact at all.
    candidates.sort_unstable();

    let mut disclosures = Vec::new();
    for (informant, information, investigation) in candidates {
        // Every candidate was derived from current indexes in this pass. A validation or
        // commit failure therefore signals state/allocator drift and must surface rather than
        // silently losing evidence that the handler was due to receive.
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
