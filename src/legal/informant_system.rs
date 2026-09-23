//! Confidential-source relationships and provenance-preserving disclosures into legal cases.

use crate::core::entity::EntityRef;
use crate::core::id::{
    CharacterId, IdExhaustionError, IdKind, InformantDisclosureId, InformantId, InformationId,
    InvestigationId, OrganizationId,
};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::core::version::VersionCapacityError;
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
        self.ensure_current(state)?;
        Ok(self.commit_preflighted(state))
    }

    fn ensure_current(&self, state: &AppState) -> Result<(), InformantError> {
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

        let information = state
            .intelligence
            .get_information(self.draft.source_information)
            .expect("validated source information must still exist");
        ensure_informant_disclosure_case_capacity(state, self.draft, information)?;
        let subject = information.subject();
        let strength = informant_strength(information.specificity());
        let reliability = informant_reliability(information.reliability());
        crate::legal::investigation_system::ensure_evidence_prosecution_recusal_capacity(
            state,
            self.draft.investigation,
            subject,
            strength,
            reliability,
            Admissibility::Unknown,
        )?;
        Ok(())
    }

    fn commit_preflighted(self, state: &mut AppState) -> InformantDisclosureId {
        let informant = state
            .legal
            .get_informant(self.draft.informant)
            .expect("preflighted informant must still exist");
        let handler = informant.handler();
        let character = informant.character();
        let information = state
            .intelligence
            .get_information(self.draft.source_information)
            .expect("preflighted source information must still exist");
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
        disclosure_id
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
    let information = state
        .intelligence
        .get_information(draft.source_information)
        .expect("validated source information must exist");
    ensure_informant_disclosure_case_capacity(state, draft, information)?;
    crate::legal::investigation_system::ensure_evidence_prosecution_recusal_capacity(
        state,
        draft.investigation,
        information.subject(),
        informant_strength(information.specificity()),
        informant_reliability(information.reliability()),
        Admissibility::Unknown,
    )?;
    Ok(ValidatedInformantDisclosure {
        draft,
        expected_investigation_version: investigation.version(),
    })
}

fn ensure_informant_disclosure_case_capacity(
    state: &AppState,
    draft: InformantDisclosureDraft,
    information: &crate::intelligence::InformationRecord,
) -> Result<(), VersionCapacityError> {
    let strength = informant_strength(information.specificity());
    let reliability = informant_reliability(information.reliability());
    crate::legal::investigation_system::ensure_external_evidence_case_capacity(
        state,
        draft.investigation,
        information.subject(),
        strength,
        reliability,
        Admissibility::Unknown,
    )
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
    // Exact equality is safe because the canonical pipeline advances exactly one minute per
    // tick and this pass runs every tick. Select that custody cohort directly from the derived
    // chronology index instead of rescanning every live detainee to rediscover the same fact.
    // A batched or skipped pass would still need a persisted decided-marker instead.
    let due_arrests: Vec<_> = now
        .as_minutes()
        .checked_sub(u64::from(decision_delay))
        .map(SimTime::from_minutes)
        .into_iter()
        .flat_map(|arrested_at| state.legal.detained_arrests_arrested_at(arrested_at))
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
        let safety = record
            .drive(crate::world::DriveKind::Safety)
            .map(|rating| u32::from(rating.value()))
            .unwrap_or(0);
        candidates.push((arrest, character, handler, safety));
    }

    let mut planned = Vec::with_capacity(candidates.len());
    let mut advanced_rng = state.investigation_rng_mut().clone();
    for (arrest, character, handler, safety) in candidates {
        let chance = resolve_informant_flip_chance(
            registry.legal(),
            safety,
            state
                .legal
                .active_representation_for_arrest(arrest)
                .is_some(),
        );
        // Validate the entire due cohort before publishing any draw or relationship. Candidates
        // are distinct detained characters, so one successful establishment cannot invalidate
        // another candidate's character/handler snapshot in this same pass.
        let validated = validate_establish_informant(state, InformantDraft { character, handler })?;
        let roll = crate::core::simulation::draw_index(&mut advanced_rng, 100)
            .expect("percentile draw over the nonempty 0..100 index range cannot fail");
        planned.push((validated, informant_flip_succeeds(roll, chance)));
    }

    // Reserve only the relationships the frozen draw sequence will actually create. This keeps
    // near-exhaustion behavior exact rather than pessimistically requiring one ID per candidate,
    // while preventing a later successful flip from leaving earlier recruits and RNG draws
    // committed behind an allocator error.
    let successful = u32::try_from(planned.iter().filter(|(_, success)| *success).count())
        .expect("detained candidate count must fit the informant ID space");
    if state.ids.reserve(IdKind::Informant, successful).is_err() {
        // The draw sequence is still speculative here. At the finite relationship-ID rail,
        // publish neither recruits nor RNG progress rather than panicking the canonical tick.
        return Ok(Vec::new());
    }

    let mut recruited = Vec::with_capacity(successful as usize);
    for (validated, success) in planned {
        if !success {
            continue;
        }
        recruited.push(
            validated
                .commit(state)
                .expect("prevalidated informant establishment and ID budget must remain current"),
        );
    }
    *state.investigation_rng_mut() = advanced_rng;
    Ok(recruited)
}

fn informant_flip_succeeds(roll: usize, chance: u32) -> bool {
    u32::try_from(roll).expect("percentile draw fits u32") < chance
}

fn resolve_informant_flip_chance(
    legal: crate::registry::LegalConfigDefinition,
    safety: u32,
    represented: bool,
) -> u32 {
    let safety_bonus = safety * u32::from(legal.informant_safety_bonus_percent()) / 100;
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
    let mut candidates: Vec<(InformantId, InformationId, InvestigationId)> = Vec::new();
    let handlers: Vec<OrganizationId> = state.legal.informant_handlers().collect();
    for handler in handlers {
        // Build only this handler's live case view. A held fact is institutionally available to
        // every matching active file in the same pass. Serializing that propagation one case per
        // minute would make case ID determine which file gets refreshed before same-minute
        // cold-case decay.
        let mut cases_by_subject: BTreeMap<EntityRef, BTreeSet<InvestigationId>> = BTreeMap::new();
        for investigation in state.legal.active_investigations_for_owner(handler) {
            if let Some(origin) = investigation.origin() {
                cases_by_subject
                    .entry(origin)
                    .or_default()
                    .insert(investigation.id());
            }
            for subject in investigation.subjects() {
                cases_by_subject
                    .entry(*subject)
                    .or_default()
                    .insert(investigation.id());
            }
        }
        for informant in state.legal.informants_for_handler(handler) {
            let holder = KnowledgeHolder::Character(informant.character());
            for (subject, investigations) in &cases_by_subject {
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
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    // Investigation versions are finite, while an active file can legitimately survive all the
    // way to that rail after years of evidence, staffing, witness, and lifecycle revisions. One
    // case must never make the institution's whole disclosure pass fail forever. Preserve the
    // all-or-none guarantee *per case*: if this minute's complete fact cohort would exceed that
    // case's remaining version capacity, skip the case entirely and keep processing other files.
    // Direct disclosure remains fail-closed with VersionCapacity for callers that explicitly ask
    // to mutate such a case.
    let mut required_advances_by_investigation: BTreeMap<InvestigationId, u32> = BTreeMap::new();
    for (_, _, investigation) in &candidates {
        let advances = required_advances_by_investigation
            .entry(*investigation)
            .or_insert(0);
        *advances = advances
            .checked_add(1)
            .ok_or_else(|| VersionCapacityError::new("investigation"))?;
    }
    let blocked_investigations: BTreeSet<_> = required_advances_by_investigation
        .iter()
        .filter_map(|(investigation, required)| {
            let version = state
                .legal
                .get_investigation(*investigation)
                .expect("active disclosure candidate must reference a live investigation")
                .version();
            let headroom =
                crate::legal::investigation_work_execution::scheduled_work_investigation_headroom(
                    state,
                    *investigation,
                    None,
                );
            required
                .checked_add(headroom)
                .is_none_or(|total| total > u32::MAX - version)
                .then_some(*investigation)
        })
        .collect();
    if !blocked_investigations.is_empty() {
        candidates.retain(|(_, _, investigation)| !blocked_investigations.contains(investigation));
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
    }

    // One pass promises same-minute propagation to every matching file. Preflight the whole
    // finite-resource budget before the first disclosure mutates a case so ID or case-version
    // exhaustion cannot publish only a prefix and let stable IDs decide which case received the
    // fact before cold-case processing.
    let candidate_count =
        u32::try_from(candidates.len()).map_err(|_| VersionCapacityError::new("investigation"))?;
    if state
        .ids
        .reserve_many(&[
            (IdKind::Evidence, candidate_count),
            (IdKind::InformantDisclosure, candidate_count),
        ])
        .is_err()
    {
        // Same-minute propagation is an all-or-none promise across the surviving candidate set.
        // An exhausted persistence rail leaves every disclosure pending instead of publishing an
        // ID-ordered prefix before cold-case processing.
        return Ok(Vec::new());
    }
    let mut advances_by_investigation: BTreeMap<InvestigationId, u32> = BTreeMap::new();
    for (informant, information, investigation) in &candidates {
        let draft = InformantDisclosureDraft {
            informant: *informant,
            investigation: *investigation,
            source_information: *information,
        };
        validate_disclosure_dependencies(state, draft)?;
        let advances = advances_by_investigation.entry(*investigation).or_insert(0);
        *advances = advances
            .checked_add(1)
            .ok_or_else(|| VersionCapacityError::new("investigation"))?;
        let information = state
            .intelligence
            .get_information(*information)
            .expect("preflighted disclosure information must exist");
        crate::legal::investigation_system::ensure_evidence_prosecution_recusal_capacity(
            state,
            *investigation,
            information.subject(),
            informant_strength(information.specificity()),
            informant_reliability(information.reliability()),
            Admissibility::Unknown,
        )?;
    }
    for (investigation, advances) in advances_by_investigation {
        crate::legal::investigation_work_execution::ensure_external_investigation_mutation_capacity(
            state,
            investigation,
            advances,
            None,
        )?;
    }

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
        )?;
        // Validation used the post-previous-disclosure state and the whole pass already reserved
        // aggregate ID/version capacity before the first mutation. Nothing can stale this token
        // between validation and its owner mutation.
        let disclosure = disclosure.commit_preflighted(state);
        disclosures.push(disclosure);
    }
    Ok(disclosures)
}

#[cfg(test)]
mod tests;
