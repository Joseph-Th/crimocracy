//! Scheduled detective pattern analysis; this sibling system derives new case evidence only from evidence already owned by the investigation.

use crate::core::entity::EntityRef;
use crate::core::id::{ArrestId, CharacterId, EvidenceId, InvestigationId, InvestigationWorkId};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::legal::case_graph::resolve_evidence_path;
use crate::legal::{
    Admissibility, EvidenceAssessment, EvidenceConnection, EvidenceIdentity, EvidenceKind,
    EvidenceRecord, EvidenceReliability, EvidenceStrength, InvestigationStatus,
    InvestigationWorkDraft, InvestigationWorkFactors, InvestigationWorkFocus,
    InvestigationWorkIdentity, InvestigationWorkKind, InvestigationWorkOutcome,
    InvestigationWorkRecord, InvestigationWorkResolution, InvestigationWorkRuntime,
    InvestigationWorkStatus,
};
use crate::registry::{InvestigationWorkDefinition, Registry};
use crate::world::{CapabilityKind, Lifecycle, Rating};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum InvestigationWorkError {
    #[error("investigation {0} does not exist")]
    MissingInvestigation(InvestigationId),
    #[error("investigation {0} is not active")]
    InactiveInvestigation(InvestigationId),
    #[error("investigator {0} does not exist")]
    MissingInvestigator(CharacterId),
    #[error("investigator {investigator} is not assigned to investigation {investigation}")]
    InvestigatorNotAssigned {
        investigation: InvestigationId,
        investigator: CharacterId,
    },
    #[error("investigator {0} is not active")]
    InactiveInvestigator(CharacterId),
    #[error("investigator {investigator} is detained under arrest {arrest}")]
    DetainedInvestigator {
        investigator: CharacterId,
        arrest: ArrestId,
    },
    #[error("investigator {0} has no Investigation capability")]
    MissingInvestigationCapability(CharacterId),
    #[error("investigation work focus must connect two distinct entities")]
    InvalidFocus,
    #[error("investigation {investigation} has no evidence path between {from:?} and {to:?}")]
    NoEvidencePath {
        investigation: InvestigationId,
        from: EntityRef,
        to: EntityRef,
    },
    #[error(
        "investigation {investigation} already has direct evidence between {from:?} and {to:?}"
    )]
    DirectEvidenceAlreadyExists {
        investigation: InvestigationId,
        from: EntityRef,
        to: EntityRef,
    },
    #[error("evidence {evidence} has already been reviewed as evidence {derived}")]
    EvidenceAlreadyReviewed {
        evidence: EvidenceId,
        derived: EvidenceId,
    },
    #[error("scheduled investigation work {work} already covers this case focus")]
    DuplicateScheduledWork { work: InvestigationWorkId },
    #[error("investigation evidence path is too large to persist as one work item")]
    SourceEvidenceCountOverflow,
    #[error("investigation {investigation} changed after work validation; expected version {expected}, found {found}")]
    StaleInvestigation {
        investigation: InvestigationId,
        expected: u32,
        found: u32,
    },
    #[error("investigator {investigator} changed after work validation; expected version {expected}, found {found}")]
    StaleInvestigator {
        investigator: CharacterId,
        expected: u32,
        found: u32,
    },
    #[error("investigation work {0} does not exist")]
    MissingWork(InvestigationWorkId),
    #[error("investigation work {0} is not scheduled")]
    WorkNotScheduled(InvestigationWorkId),
    #[error("investigation work {work} is not due until {due_at:?}")]
    WorkNotDue {
        work: InvestigationWorkId,
        due_at: SimTime,
    },
    #[error("investigation work {work} changed after resolution planning; expected version {expected}, found {found}")]
    StaleWork {
        work: InvestigationWorkId,
        expected: u32,
        found: u32,
    },
    #[error("investigation work resolution was planned at {expected:?}, but simulation time is now {found:?}")]
    StaleResolutionTime { expected: SimTime, found: SimTime },
    #[error("investigation work variance {variance} exceeds authored limit {limit}")]
    VarianceOutOfRange { variance: i8, limit: u8 },
    #[error("investigation work source evidence {0} no longer belongs to the case")]
    InvalidSourceEvidence(EvidenceId),
}

#[derive(Debug)]
pub struct ValidatedInvestigationWorkSchedule {
    draft: InvestigationWorkDraft,
    source_evidence: BTreeSet<EvidenceId>,
    expected_investigation_version: u32,
    expected_investigator_version: u32,
    duration_minutes: u32,
}

impl ValidatedInvestigationWorkSchedule {
    pub fn commit(
        self,
        state: &mut AppState,
    ) -> Result<InvestigationWorkId, InvestigationWorkError> {
        let investigation = state
            .legal
            .get_investigation(self.draft.investigation)
            .ok_or(InvestigationWorkError::MissingInvestigation(
                self.draft.investigation,
            ))?;
        if investigation.version() != self.expected_investigation_version {
            return Err(InvestigationWorkError::StaleInvestigation {
                investigation: self.draft.investigation,
                expected: self.expected_investigation_version,
                found: investigation.version(),
            });
        }
        let investigator = state.world.get_character(self.draft.investigator).ok_or(
            InvestigationWorkError::MissingInvestigator(self.draft.investigator),
        )?;
        if investigator.version() != self.expected_investigator_version {
            return Err(InvestigationWorkError::StaleInvestigator {
                investigator: self.draft.investigator,
                expected: self.expected_investigator_version,
                found: investigator.version(),
            });
        }
        validate_case_and_investigator(state, self.draft.investigation, self.draft.investigator)?;
        validate_no_duplicate_work(state, self.draft)?;
        let current_sources = resolve_work_sources(state, self.draft)?;
        if current_sources != self.source_evidence {
            return Err(InvestigationWorkError::StaleInvestigation {
                investigation: self.draft.investigation,
                expected: self.expected_investigation_version,
                found: investigation.version(),
            });
        }

        let id = state.ids.next_investigation_work();
        let scheduled_at = state.now();
        let due_at =
            scheduled_at + crate::core::time::SimDuration::from_minutes(self.duration_minutes);
        state
            .legal
            .insert_investigation_work(InvestigationWorkRecord {
                identity: InvestigationWorkIdentity {
                    id,
                    investigation: self.draft.investigation,
                    investigator: self.draft.investigator,
                    kind: self.draft.kind,
                    focus: self.draft.focus,
                },
                source_evidence: self.source_evidence,
                runtime: InvestigationWorkRuntime {
                    scheduled_at,
                    due_at,
                    status: InvestigationWorkStatus::Scheduled,
                    resolution: None,
                    version: 1,
                },
            });
        Ok(id)
    }
}

pub fn validate_schedule_investigation_work(
    registry: &Registry,
    state: &AppState,
    draft: InvestigationWorkDraft,
) -> Result<ValidatedInvestigationWorkSchedule, InvestigationWorkError> {
    validate_case_and_investigator(state, draft.investigation, draft.investigator)?;
    validate_no_duplicate_work(state, draft)?;
    let source_evidence = resolve_work_sources(state, draft)?;
    let investigation = state
        .legal
        .get_investigation(draft.investigation)
        .expect("validated investigation must still exist");
    let investigator = state
        .world
        .get_character(draft.investigator)
        .expect("validated investigator must still exist");
    let duration = registry.get_investigation_work(draft.kind).duration();
    Ok(ValidatedInvestigationWorkSchedule {
        draft,
        source_evidence,
        expected_investigation_version: investigation.version(),
        expected_investigator_version: investigator.version(),
        duration_minutes: duration.as_minutes(),
    })
}

fn resolve_work_sources(
    state: &AppState,
    draft: InvestigationWorkDraft,
) -> Result<BTreeSet<EvidenceId>, InvestigationWorkError> {
    match draft.kind {
        InvestigationWorkKind::PatternAnalysis => {
            if !matches!(
                draft.focus,
                crate::legal::InvestigationWorkFocus::EntityConnection { .. }
            ) || draft.focus.from() == draft.focus.to()
            {
                return Err(InvestigationWorkError::InvalidFocus);
            }
            resolve_pattern_sources(state, draft)
        }
        InvestigationWorkKind::EvidenceReview => resolve_review_source(state, draft),
    }
}

fn resolve_review_source(
    state: &AppState,
    draft: InvestigationWorkDraft,
) -> Result<BTreeSet<EvidenceId>, InvestigationWorkError> {
    let evidence_id = draft
        .focus
        .evidence_id()
        .ok_or(InvestigationWorkError::InvalidFocus)?;
    let evidence = state
        .legal
        .get_evidence(evidence_id)
        .ok_or(InvestigationWorkError::InvalidSourceEvidence(evidence_id))?;
    if evidence.investigation() != draft.investigation {
        return Err(InvestigationWorkError::InvalidSourceEvidence(evidence_id));
    }
    if !is_reviewable_evidence_kind(evidence.kind()) {
        return Err(InvestigationWorkError::InvalidFocus);
    }
    if let Some(derived) = state
        .legal
        .derived_evidence_from(evidence_id)
        .find(|derived| derived.kind() == EvidenceKind::ForensicAnalysis)
    {
        return Err(InvestigationWorkError::EvidenceAlreadyReviewed {
            evidence: evidence_id,
            derived: derived.id(),
        });
    }
    Ok(BTreeSet::from([evidence_id]))
}

fn validate_case_and_investigator(
    state: &AppState,
    investigation_id: InvestigationId,
    investigator_id: CharacterId,
) -> Result<(), InvestigationWorkError> {
    let investigation = state.legal.get_investigation(investigation_id).ok_or(
        InvestigationWorkError::MissingInvestigation(investigation_id),
    )?;
    if investigation.status() != InvestigationStatus::Active {
        return Err(InvestigationWorkError::InactiveInvestigation(
            investigation_id,
        ));
    }
    let investigator = state
        .world
        .get_character(investigator_id)
        .ok_or(InvestigationWorkError::MissingInvestigator(investigator_id))?;
    if investigation.investigator_role(investigator_id).is_none() {
        return Err(InvestigationWorkError::InvestigatorNotAssigned {
            investigation: investigation_id,
            investigator: investigator_id,
        });
    }
    if investigator.lifecycle() != Lifecycle::Active {
        return Err(InvestigationWorkError::InactiveInvestigator(
            investigator_id,
        ));
    }
    if let Some(arrest) = state.legal.active_arrest_for_character(investigator_id) {
        return Err(InvestigationWorkError::DetainedInvestigator {
            investigator: investigator_id,
            arrest: arrest.id(),
        });
    }
    if investigator.organization() != Some(investigation.owner()) {
        return Err(InvestigationWorkError::InvestigatorNotAssigned {
            investigation: investigation_id,
            investigator: investigator_id,
        });
    }
    if investigator
        .capability(CapabilityKind::Investigation)
        .is_none()
    {
        return Err(InvestigationWorkError::MissingInvestigationCapability(
            investigator_id,
        ));
    }
    Ok(())
}

fn validate_no_duplicate_work(
    state: &AppState,
    draft: InvestigationWorkDraft,
) -> Result<(), InvestigationWorkError> {
    if let Some(work) =
        state
            .legal
            .scheduled_work_for_focus(draft.investigation, draft.kind, draft.focus)
    {
        return Err(InvestigationWorkError::DuplicateScheduledWork { work: work.id() });
    }
    Ok(())
}

fn resolve_pattern_sources(
    state: &AppState,
    draft: InvestigationWorkDraft,
) -> Result<BTreeSet<EvidenceId>, InvestigationWorkError> {
    let path = resolve_evidence_path(
        state,
        draft.investigation,
        draft.focus.from(),
        draft.focus.to(),
    )
    .map_err(|_| InvestigationWorkError::MissingInvestigation(draft.investigation))?
    .ok_or(InvestigationWorkError::NoEvidencePath {
        investigation: draft.investigation,
        from: draft.focus.from(),
        to: draft.focus.to(),
    })?;
    if path.links.len() == 1 {
        return Err(InvestigationWorkError::DirectEvidenceAlreadyExists {
            investigation: draft.investigation,
            from: draft.focus.from(),
            to: draft.focus.to(),
        });
    }
    if path.links.len() > usize::from(u8::MAX) {
        return Err(InvestigationWorkError::SourceEvidenceCountOverflow);
    }
    Ok(path.links.into_iter().map(|link| link.evidence).collect())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvestigationWorkRandomness {
    variance: i8,
}

impl InvestigationWorkRandomness {
    pub fn new(variance: i8) -> Self {
        Self { variance }
    }

    pub fn variance(self) -> i8 {
        self.variance
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvestigationWorkResolutionPlan {
    work: InvestigationWorkId,
    expected_work_version: u32,
    expected_investigator_version: u32,
    resolved_at: SimTime,
    outcome: InvestigationWorkOutcome,
    factors: InvestigationWorkFactors,
    margin: i16,
    superseded_by: Option<EvidenceId>,
}

impl InvestigationWorkResolutionPlan {
    pub fn work(&self) -> InvestigationWorkId {
        self.work
    }

    pub fn outcome(&self) -> InvestigationWorkOutcome {
        self.outcome
    }

    pub fn factors(&self) -> InvestigationWorkFactors {
        self.factors
    }

    pub fn margin(&self) -> i16 {
        self.margin
    }
}

pub fn decide_investigation_work_resolution(
    registry: &Registry,
    state: &AppState,
    work_id: InvestigationWorkId,
    randomness: InvestigationWorkRandomness,
) -> Result<InvestigationWorkResolutionPlan, InvestigationWorkError> {
    let work = validate_due_work(state, work_id)?;
    let definition = registry.get_investigation_work(work.kind());
    if randomness.variance().unsigned_abs() > definition.variance_limit() {
        return Err(InvestigationWorkError::VarianceOutOfRange {
            variance: randomness.variance(),
            limit: definition.variance_limit(),
        });
    }
    let investigator = state
        .world
        .get_character(work.investigator())
        .expect("validated scheduled investigator must exist");
    let (factors, margin) =
        calculate_work_factors_and_margin(definition, state, work, randomness.variance())?;
    let superseded_by = find_superseding_evidence(state, work);
    let outcome = if superseded_by.is_some() {
        InvestigationWorkOutcome::Superseded
    } else if margin >= definition.connected_margin() {
        match work.kind() {
            InvestigationWorkKind::PatternAnalysis => InvestigationWorkOutcome::Connected,
            InvestigationWorkKind::EvidenceReview => InvestigationWorkOutcome::Developed,
        }
    } else {
        InvestigationWorkOutcome::Inconclusive
    };
    Ok(InvestigationWorkResolutionPlan {
        work: work.id(),
        expected_work_version: work.version(),
        expected_investigator_version: investigator.version(),
        resolved_at: state.now(),
        outcome,
        factors,
        margin,
        superseded_by,
    })
}

fn validate_due_work(
    state: &AppState,
    work_id: InvestigationWorkId,
) -> Result<&InvestigationWorkRecord, InvestigationWorkError> {
    let work = state
        .legal
        .get_investigation_work(work_id)
        .ok_or(InvestigationWorkError::MissingWork(work_id))?;
    if work.status() != InvestigationWorkStatus::Scheduled {
        return Err(InvestigationWorkError::WorkNotScheduled(work_id));
    }
    if state.now() < work.due_at() {
        return Err(InvestigationWorkError::WorkNotDue {
            work: work_id,
            due_at: work.due_at(),
        });
    }
    validate_case_and_investigator(state, work.investigation(), work.investigator())?;
    validate_source_evidence(state, work)?;
    Ok(work)
}

fn validate_source_evidence(
    state: &AppState,
    work: &InvestigationWorkRecord,
) -> Result<(), InvestigationWorkError> {
    for evidence_id in work.source_evidence() {
        let evidence = state
            .legal
            .get_evidence(*evidence_id)
            .ok_or(InvestigationWorkError::InvalidSourceEvidence(*evidence_id))?;
        if evidence.investigation() != work.investigation() {
            return Err(InvestigationWorkError::InvalidSourceEvidence(*evidence_id));
        }
    }
    Ok(())
}

pub(crate) fn calculate_work_difficulty(
    definition: &InvestigationWorkDefinition,
    source_evidence_count: u8,
) -> u8 {
    let additional_count = source_evidence_count.saturating_sub(2);
    let additional = u16::from(additional_count)
        .saturating_mul(u16::from(definition.additional_source_difficulty()));
    u8::try_from(
        u16::from(definition.base_difficulty())
            .saturating_add(additional)
            .min(100),
    )
    .expect("clamped investigation difficulty must fit u8")
}

pub(crate) fn calculate_work_factors_and_margin(
    definition: &InvestigationWorkDefinition,
    state: &AppState,
    work: &InvestigationWorkRecord,
    variance: i8,
) -> Result<(InvestigationWorkFactors, i16), InvestigationWorkError> {
    let investigator = state.world.get_character(work.investigator()).ok_or(
        InvestigationWorkError::MissingInvestigator(work.investigator()),
    )?;
    let investigation_capability = investigator
        .capability(CapabilityKind::Investigation)
        .ok_or(InvestigationWorkError::MissingInvestigationCapability(
            work.investigator(),
        ))?;
    let source_support = resolve_source_support(state, work)?;
    let source_evidence_count = u8::try_from(work.source_evidence().len())
        .map_err(|_| InvestigationWorkError::SourceEvidenceCountOverflow)?;
    let difficulty = calculate_work_difficulty(definition, source_evidence_count);
    let support_adjustment =
        i16::from(source_support.value()) * i16::from(definition.source_support_weight()) / 100;
    let margin =
        i16::from(investigation_capability.value()) + support_adjustment + i16::from(variance)
            - i16::from(difficulty);
    Ok((
        InvestigationWorkFactors {
            investigation_capability,
            source_support,
            source_evidence_count,
            difficulty,
            variance,
        },
        margin,
    ))
}

pub(crate) fn resolve_source_support(
    state: &AppState,
    work: &InvestigationWorkRecord,
) -> Result<Rating, InvestigationWorkError> {
    let mut total = 0_u32;
    let mut count = 0_u32;
    for evidence_id in work.source_evidence() {
        let evidence = state
            .legal
            .get_evidence(*evidence_id)
            .ok_or(InvestigationWorkError::InvalidSourceEvidence(*evidence_id))?;
        if evidence.investigation() != work.investigation() {
            return Err(InvestigationWorkError::InvalidSourceEvidence(*evidence_id));
        }
        total = total
            .saturating_add(u32::from(strength_score(evidence.strength())))
            .saturating_add(u32::from(reliability_score(evidence.reliability())))
            .saturating_add(u32::from(admissibility_score(evidence.admissibility())));
        count = count.saturating_add(3);
    }
    let average = total.checked_div(count).unwrap_or(0);
    Ok(
        Rating::try_new(u8::try_from(average).expect("evidence support average must fit u8"))
            .expect("bounded evidence support average must be a valid rating"),
    )
}

fn strength_score(strength: EvidenceStrength) -> u8 {
    match strength {
        EvidenceStrength::Weak => 20,
        EvidenceStrength::Corroborating => 45,
        EvidenceStrength::Strong => 70,
        EvidenceStrength::Direct => 95,
    }
}

fn reliability_score(reliability: EvidenceReliability) -> u8 {
    match reliability {
        EvidenceReliability::Questionable => 15,
        EvidenceReliability::Mixed => 40,
        EvidenceReliability::Credible => 70,
        EvidenceReliability::HighlyReliable => 95,
    }
}

fn admissibility_score(admissibility: Admissibility) -> u8 {
    match admissibility {
        Admissibility::Unknown => 35,
        Admissibility::Inadmissible => 0,
        Admissibility::Disputed => 50,
        Admissibility::Admissible => 90,
    }
}

pub(crate) fn find_superseding_evidence(
    state: &AppState,
    work: &InvestigationWorkRecord,
) -> Option<EvidenceId> {
    let own_derived = work
        .resolution()
        .and_then(|resolution| resolution.derived_evidence());
    if work.kind() == InvestigationWorkKind::EvidenceReview {
        let source = work.focus().evidence_id()?;
        return state
            .legal
            .derived_evidence_from(source)
            .find(|evidence| {
                Some(evidence.id()) != own_derived
                    && evidence.kind() == EvidenceKind::ForensicAnalysis
            })
            .map(|evidence| evidence.id());
    }
    let from = work.focus().from();
    let to = work.focus().to();
    state
        .legal
        .get_investigation(work.investigation())
        .into_iter()
        .flat_map(|investigation| investigation.evidence().iter())
        .filter_map(|id| state.legal.get_evidence(*id))
        .find(|evidence| {
            Some(evidence.id()) != own_derived
                && evidence.origin().is_some_and(|origin| {
                    (origin == from && evidence.subject() == to)
                        || (origin == to && evidence.subject() == from)
                })
        })
        .map(|evidence| evidence.id())
}

pub(crate) fn source_evidence_forms_simple_path(
    state: &AppState,
    work: &InvestigationWorkRecord,
) -> bool {
    if work.source_evidence().len() < 2 {
        return false;
    }
    let mut adjacency: std::collections::BTreeMap<EntityRef, BTreeSet<EntityRef>> =
        std::collections::BTreeMap::new();
    for evidence_id in work.source_evidence() {
        let Some(evidence) = state.legal.get_evidence(*evidence_id) else {
            return false;
        };
        if evidence.investigation() != work.investigation() {
            return false;
        }
        let Some(origin) = evidence.origin() else {
            return false;
        };
        if origin == evidence.subject() {
            return false;
        }
        adjacency
            .entry(origin)
            .or_default()
            .insert(evidence.subject());
        adjacency
            .entry(evidence.subject())
            .or_default()
            .insert(origin);
    }
    if adjacency.len() != work.source_evidence().len().saturating_add(1)
        || adjacency
            .get(&work.focus().from())
            .is_none_or(|neighbors| neighbors.len() != 1)
        || adjacency
            .get(&work.focus().to())
            .is_none_or(|neighbors| neighbors.len() != 1)
    {
        return false;
    }
    if adjacency.iter().any(|(entity, neighbors)| {
        *entity != work.focus().from() && *entity != work.focus().to() && neighbors.len() != 2
    }) {
        return false;
    }
    let mut visited = BTreeSet::from([work.focus().from()]);
    let mut frontier = std::collections::VecDeque::from([work.focus().from()]);
    while let Some(current) = frontier.pop_front() {
        let Some(neighbors) = adjacency.get(&current) else {
            return false;
        };
        for neighbor in neighbors {
            if visited.insert(*neighbor) {
                frontier.push_back(*neighbor);
            }
        }
    }
    visited.len() == adjacency.len() && visited.contains(&work.focus().to())
}

pub struct ValidatedInvestigationWorkResolution {
    plan: InvestigationWorkResolutionPlan,
}

struct DerivedEvidenceDraft {
    investigation: InvestigationId,
    custodian: crate::core::id::OrganizationId,
    subject: EntityRef,
    origin: Option<EntityRef>,
    kind: EvidenceKind,
    strength: EvidenceStrength,
    reliability: EvidenceReliability,
    admissibility: Admissibility,
    derived_from: BTreeSet<EvidenceId>,
}

pub(crate) fn derive_pattern_strength(source_support: Rating) -> EvidenceStrength {
    if source_support.value() >= 75 {
        EvidenceStrength::Strong
    } else {
        EvidenceStrength::Corroborating
    }
}

pub(crate) fn derive_pattern_admissibility(
    state: &AppState,
    work: &InvestigationWorkRecord,
) -> Admissibility {
    if work.source_evidence().iter().all(|id| {
        state
            .legal
            .get_evidence(*id)
            .is_some_and(|evidence| evidence.admissibility() == Admissibility::Admissible)
    }) {
        Admissibility::Admissible
    } else {
        Admissibility::Disputed
    }
}

impl ValidatedInvestigationWorkResolution {
    pub fn commit(
        self,
        state: &mut AppState,
    ) -> Result<InvestigationWorkId, InvestigationWorkError> {
        validate_resolution_snapshot(state, &self.plan)?;
        let derived_evidence_draft = match self.plan.outcome {
            InvestigationWorkOutcome::Connected => {
                let work = state
                    .legal
                    .get_investigation_work(self.plan.work)
                    .expect("validated investigation work must exist");
                debug_assert_eq!(work.kind(), InvestigationWorkKind::PatternAnalysis);
                let strength = derive_pattern_strength(self.plan.factors.source_support());
                let reliability = minimum_source_reliability(state, work)?;
                let admissibility = derive_pattern_admissibility(state, work);
                Some(DerivedEvidenceDraft {
                    investigation: work.investigation(),
                    custodian: state
                        .legal
                        .get_investigation(work.investigation())
                        .expect("validated investigation must exist")
                        .owner(),
                    subject: work.focus().to(),
                    origin: Some(work.focus().from()),
                    kind: EvidenceKind::PatternLink,
                    strength,
                    reliability,
                    admissibility,
                    derived_from: work.source_evidence().clone(),
                })
            }
            InvestigationWorkOutcome::Developed => {
                let work = state
                    .legal
                    .get_investigation_work(self.plan.work)
                    .expect("validated investigation work must exist");
                debug_assert_eq!(work.kind(), InvestigationWorkKind::EvidenceReview);
                let source_id = work
                    .focus()
                    .evidence_id()
                    .expect("evidence review work must focus one evidence record");
                let source = state
                    .legal
                    .get_evidence(source_id)
                    .expect("validated evidence review source must exist");
                Some(DerivedEvidenceDraft {
                    investigation: work.investigation(),
                    custodian: state
                        .legal
                        .get_investigation(work.investigation())
                        .expect("validated investigation must exist")
                        .owner(),
                    subject: source.subject(),
                    origin: source.origin(),
                    kind: EvidenceKind::ForensicAnalysis,
                    strength: source.strength(),
                    reliability: improve_evidence_reliability(source.reliability()),
                    admissibility: source.admissibility(),
                    derived_from: BTreeSet::from([source_id]),
                })
            }
            InvestigationWorkOutcome::Inconclusive | InvestigationWorkOutcome::Superseded => None,
        };
        let derived_evidence = if let Some(draft) = derived_evidence_draft {
            let id = state.ids.next_evidence();
            state.legal.insert_evidence(EvidenceRecord {
                identity: EvidenceIdentity {
                    id,
                    investigation: draft.investigation,
                    custodian: draft.custodian,
                },
                connection: EvidenceConnection {
                    subject: draft.subject,
                    origin: draft.origin,
                    source: None,
                    derived_from: draft.derived_from,
                },
                assessment: EvidenceAssessment {
                    kind: draft.kind,
                    strength: draft.strength,
                    reliability: draft.reliability,
                    admissibility: draft.admissibility,
                },
                discovered_at: self.plan.resolved_at,
            });
            Some(id)
        } else {
            None
        };
        state.legal.complete_investigation_work(
            self.plan.work,
            InvestigationWorkResolution {
                resolved_at: self.plan.resolved_at,
                outcome: self.plan.outcome,
                factors: self.plan.factors,
                margin: self.plan.margin,
                superseded_by: self.plan.superseded_by,
                derived_evidence,
            },
        );
        Ok(self.plan.work)
    }
}

pub(crate) fn improve_evidence_reliability(
    reliability: EvidenceReliability,
) -> EvidenceReliability {
    match reliability {
        EvidenceReliability::Questionable => EvidenceReliability::Mixed,
        EvidenceReliability::Mixed => EvidenceReliability::Credible,
        EvidenceReliability::Credible | EvidenceReliability::HighlyReliable => {
            EvidenceReliability::HighlyReliable
        }
    }
}

pub(crate) fn schedule_initial_evidence_reviews(
    registry: &Registry,
    state: &mut AppState,
    staffed: &[(InvestigationId, CharacterId)],
) -> Result<Vec<InvestigationWorkId>, InvestigationWorkError> {
    let mut scheduled = Vec::new();
    for (investigation_id, investigator) in staffed {
        if state
            .legal
            .work_for_investigation(*investigation_id)
            .next()
            .is_some()
        {
            continue;
        }
        let source = state
            .legal
            .get_investigation(*investigation_id)
            .into_iter()
            .flat_map(|investigation| investigation.evidence().iter())
            .filter_map(|id| state.legal.get_evidence(*id))
            .find(|evidence| is_reviewable_evidence_kind(evidence.kind()))
            .map(|evidence| evidence.id());
        let Some(source) = source else {
            continue;
        };
        let work = validate_schedule_investigation_work(
            registry,
            state,
            InvestigationWorkDraft {
                investigation: *investigation_id,
                investigator: *investigator,
                kind: InvestigationWorkKind::EvidenceReview,
                focus: InvestigationWorkFocus::evidence(source),
            },
        )?
        .commit(state)?;
        scheduled.push(work);
    }
    Ok(scheduled)
}

pub(crate) fn is_reviewable_evidence_kind(kind: EvidenceKind) -> bool {
    matches!(
        kind,
        EvidenceKind::Fingerprint
            | EvidenceKind::RecoveredProperty
            | EvidenceKind::FinancialRecord
            | EvidenceKind::Surveillance
            | EvidenceKind::CommunicationRecord
            | EvidenceKind::Document
            | EvidenceKind::Ballistics
            | EvidenceKind::VehicleDescription
    )
}

pub(crate) fn minimum_source_reliability(
    state: &AppState,
    work: &InvestigationWorkRecord,
) -> Result<EvidenceReliability, InvestigationWorkError> {
    let mut minimum = EvidenceReliability::HighlyReliable;
    for evidence_id in work.source_evidence() {
        let evidence = state
            .legal
            .get_evidence(*evidence_id)
            .ok_or(InvestigationWorkError::InvalidSourceEvidence(*evidence_id))?;
        if reliability_score(evidence.reliability()) < reliability_score(minimum) {
            minimum = evidence.reliability();
        }
    }
    Ok(minimum)
}

pub fn validate_investigation_work_resolution_plan(
    registry: &Registry,
    state: &AppState,
    plan: InvestigationWorkResolutionPlan,
) -> Result<ValidatedInvestigationWorkResolution, InvestigationWorkError> {
    validate_resolution_snapshot(state, &plan)?;
    let work = state
        .legal
        .get_investigation_work(plan.work)
        .expect("validated work must exist");
    let definition = registry.get_investigation_work(work.kind());
    let (expected_factors, expected_margin) =
        calculate_work_factors_and_margin(definition, state, work, plan.factors.variance())?;
    let expected_superseded_by = find_superseding_evidence(state, work);
    let expected_outcome = if expected_superseded_by.is_some() {
        InvestigationWorkOutcome::Superseded
    } else if expected_margin >= definition.connected_margin() {
        match work.kind() {
            InvestigationWorkKind::PatternAnalysis => InvestigationWorkOutcome::Connected,
            InvestigationWorkKind::EvidenceReview => InvestigationWorkOutcome::Developed,
        }
    } else {
        InvestigationWorkOutcome::Inconclusive
    };
    if plan.factors != expected_factors
        || plan.factors.variance().unsigned_abs() > definition.variance_limit()
        || plan.margin != expected_margin
        || plan.outcome != expected_outcome
        || plan.superseded_by != expected_superseded_by
    {
        return Err(InvestigationWorkError::StaleWork {
            work: plan.work,
            expected: plan.expected_work_version,
            found: work.version(),
        });
    }
    Ok(ValidatedInvestigationWorkResolution { plan })
}

fn validate_resolution_snapshot(
    state: &AppState,
    plan: &InvestigationWorkResolutionPlan,
) -> Result<(), InvestigationWorkError> {
    let work = validate_due_work(state, plan.work)?;
    if work.version() != plan.expected_work_version {
        return Err(InvestigationWorkError::StaleWork {
            work: plan.work,
            expected: plan.expected_work_version,
            found: work.version(),
        });
    }
    let investigator = state
        .world
        .get_character(work.investigator())
        .expect("validated investigator must exist");
    if investigator.version() != plan.expected_investigator_version {
        return Err(InvestigationWorkError::StaleInvestigator {
            investigator: investigator.id(),
            expected: plan.expected_investigator_version,
            found: investigator.version(),
        });
    }
    if state.now() != plan.resolved_at {
        return Err(InvestigationWorkError::StaleResolutionTime {
            expected: plan.resolved_at,
            found: state.now(),
        });
    }
    Ok(())
}

pub(crate) fn due_scheduled_investigation_work(state: &AppState) -> Vec<InvestigationWorkId> {
    state.legal.due_investigation_work_at_or_before(state.now())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::invariants::{
        validate_invariants, validate_state, validate_state_against_registry,
    };
    use crate::core::persistence::{build_save, restore_save};
    use crate::core::simulation::run_tick;
    use crate::core::time::SimDuration;
    use crate::legal::case_graph::resolve_evidence_path;
    use crate::legal::investigation_system::{
        validate_add_evidence, validate_assign_investigator, validate_open_investigation,
        validate_remove_investigator, InvestigationError,
    };
    use crate::legal::{
        EvidenceDraft, InvestigationDraft, InvestigationWorkFocus, InvestigatorRole,
    };
    use crate::world::world_system::{insert_character, insert_organization};
    use crate::world::{
        AutonomyLevel, CharacterDraft, OrganizationDraft, OrganizationKind, Rating,
    };
    use std::collections::{BTreeMap, BTreeSet};

    struct WorkFixture {
        state: AppState,
        police: crate::core::id::OrganizationId,
        investigation: InvestigationId,
        investigator: CharacterId,
        first: CharacterId,
        middle: CharacterId,
        target: CharacterId,
        first_evidence: EvidenceId,
        second_evidence: EvidenceId,
    }

    fn rating(value: u8) -> Rating {
        Rating::try_new(value).expect("test rating must be valid")
    }

    fn make_fixture(
        investigator_skill: u8,
        strength: EvidenceStrength,
        reliability: EvidenceReliability,
        admissibility: Admissibility,
    ) -> WorkFixture {
        let registry = build_registry();
        let mut state = AppState::new(0x1A7E_5731);
        let police = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Pattern Bureau".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("police fixture should validate");
        let criminal = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Pattern Crew".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("criminal fixture should validate");
        let investigator = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Detective Harlan".to_owned(),
                organization: Some(police),
                supervisor: None,
                autonomy: AutonomyLevel::Delegated,
                capabilities: BTreeMap::from([(
                    CapabilityKind::Investigation,
                    rating(investigator_skill),
                )]),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("investigator fixture should validate");
        let mut insert_subject = |name: &str| {
            insert_character(
                &registry,
                &mut state,
                CharacterDraft {
                    name: name.to_owned(),
                    organization: Some(criminal),
                    supervisor: None,
                    autonomy: AutonomyLevel::Guided,
                    capabilities: BTreeMap::new(),
                    traits: BTreeSet::new(),
                    drives: BTreeMap::new(),
                },
            )
            .expect("case subject fixture should validate")
        };
        let first = insert_subject("Frank Dello");
        let middle = insert_subject("Maria Vale");
        let target = insert_subject("Fulton Garage Manager");
        let investigation = validate_open_investigation(
            &state,
            InvestigationDraft {
                owner: police,
                title: "Vehicle association inquiry".to_owned(),
                subjects: BTreeSet::from([EntityRef::Character(first)]),
            },
        )
        .expect("investigation fixture should validate")
        .commit(&mut state)
        .expect("investigation fixture should commit");
        validate_assign_investigator(&state, investigation, investigator, InvestigatorRole::Lead)
            .expect("investigator assignment should validate")
            .commit(&mut state)
            .expect("investigator assignment should commit");

        let first_evidence = add_evidence(
            &mut state,
            TestEvidenceDraft {
                investigation,
                police,
                subject: EntityRef::Character(middle),
                origin: EntityRef::Character(first),
                strength,
                reliability,
                admissibility,
            },
        );
        let second_evidence = add_evidence(
            &mut state,
            TestEvidenceDraft {
                investigation,
                police,
                subject: EntityRef::Character(target),
                origin: EntityRef::Character(middle),
                strength,
                reliability,
                admissibility,
            },
        );
        WorkFixture {
            state,
            police,
            investigation,
            investigator,
            first,
            middle,
            target,
            first_evidence,
            second_evidence,
        }
    }

    struct TestEvidenceDraft {
        investigation: InvestigationId,
        police: crate::core::id::OrganizationId,
        subject: EntityRef,
        origin: EntityRef,
        strength: EvidenceStrength,
        reliability: EvidenceReliability,
        admissibility: Admissibility,
    }

    fn add_evidence(state: &mut AppState, draft: TestEvidenceDraft) -> EvidenceId {
        let TestEvidenceDraft {
            investigation,
            police,
            subject,
            origin,
            strength,
            reliability,
            admissibility,
        } = draft;
        validate_add_evidence(
            state,
            EvidenceDraft {
                investigation,
                custodian: police,
                subject,
                origin: Some(origin),
                kind: EvidenceKind::KnownAssociation,
                strength,
                reliability,
                admissibility,
                discovered_at: state.now(),
            },
        )
        .expect("evidence fixture should validate")
        .commit(state)
        .expect("evidence fixture should commit")
    }

    fn work_draft(fixture: &WorkFixture) -> InvestigationWorkDraft {
        InvestigationWorkDraft {
            investigation: fixture.investigation,
            investigator: fixture.investigator,
            kind: InvestigationWorkKind::PatternAnalysis,
            focus: InvestigationWorkFocus::new(
                EntityRef::Character(fixture.first),
                EntityRef::Character(fixture.target),
            ),
        }
    }

    #[test]
    fn pattern_analysis_resolves_to_derived_evidence_with_provenance() {
        let registry = build_registry();
        let mut fixture = make_fixture(
            90,
            EvidenceStrength::Strong,
            EvidenceReliability::Credible,
            Admissibility::Admissible,
        );
        let work =
            validate_schedule_investigation_work(&registry, &fixture.state, work_draft(&fixture))
                .expect("pattern analysis should validate")
                .commit(&mut fixture.state)
                .expect("pattern analysis should schedule");

        let due_at = fixture
            .state
            .legal()
            .get_investigation_work(work)
            .expect("scheduled work should exist")
            .due_at();
        let early_error = decide_investigation_work_resolution(
            &registry,
            &fixture.state,
            work,
            InvestigationWorkRandomness::new(0),
        )
        .expect_err("work must not resolve before its due time");
        assert_eq!(
            early_error,
            InvestigationWorkError::WorkNotDue { work, due_at }
        );

        for _ in 0..359 {
            let outcome = run_tick(&registry, &mut fixture.state);
            assert!(outcome.resolved_investigation_work.is_empty());
        }
        let outcome = run_tick(&registry, &mut fixture.state);
        assert_eq!(outcome.resolved_investigation_work, vec![work]);

        let record = fixture
            .state
            .legal()
            .get_investigation_work(work)
            .expect("completed work should exist");
        assert_eq!(record.status(), InvestigationWorkStatus::Completed);
        let resolution = record.resolution().expect("completed work must resolve");
        assert_eq!(resolution.outcome(), InvestigationWorkOutcome::Connected);
        assert!(resolution.margin() > 0);
        assert_eq!(resolution.superseded_by(), None);
        let derived_id = resolution
            .derived_evidence()
            .expect("connected pattern analysis must create evidence");
        let derived = fixture
            .state
            .legal()
            .get_evidence(derived_id)
            .expect("derived evidence should exist");
        assert_eq!(derived.kind(), EvidenceKind::PatternLink);
        assert_eq!(
            derived.derived_from(),
            &BTreeSet::from([fixture.first_evidence, fixture.second_evidence])
        );
        assert_eq!(derived.origin(), Some(EntityRef::Character(fixture.first)));
        assert_eq!(derived.subject(), EntityRef::Character(fixture.target));
        assert_eq!(
            fixture
                .state
                .legal()
                .derived_evidence_from(fixture.first_evidence)
                .map(|evidence| evidence.id())
                .collect::<Vec<_>>(),
            vec![derived_id]
        );
        let direct_path = resolve_evidence_path(
            &fixture.state,
            fixture.investigation,
            EntityRef::Character(fixture.first),
            EntityRef::Character(fixture.target),
        )
        .expect("case graph should resolve")
        .expect("derived evidence should create a direct case link");
        assert_eq!(direct_path.links.len(), 1);
        assert_eq!(direct_path.links[0].evidence, derived_id);

        validate_remove_investigator(&fixture.state, fixture.investigation, fixture.investigator)
            .expect("completed work should release the investigator dependency")
            .commit(&mut fixture.state)
            .expect("investigator release should commit");
        validate_state(&fixture.state).expect("completed pattern analysis state should be valid");
        validate_state_against_registry(&registry, &fixture.state)
            .expect("completed pattern analysis should match authored definitions");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn evidence_review_develops_case_owned_evidence_without_inventing_subjects() {
        let registry = build_registry();
        let mut fixture = make_fixture(
            90,
            EvidenceStrength::Strong,
            EvidenceReliability::Credible,
            Admissibility::Admissible,
        );
        let fingerprint = validate_add_evidence(
            &fixture.state,
            EvidenceDraft {
                investigation: fixture.investigation,
                custodian: fixture.police,
                subject: EntityRef::Character(fixture.first),
                origin: None,
                kind: EvidenceKind::Fingerprint,
                strength: EvidenceStrength::Corroborating,
                reliability: EvidenceReliability::Mixed,
                admissibility: Admissibility::Unknown,
                discovered_at: fixture.state.now(),
            },
        )
        .expect("fingerprint evidence should validate")
        .commit(&mut fixture.state)
        .expect("fingerprint evidence should commit");
        let subjects_before = fixture
            .state
            .legal()
            .get_investigation(fixture.investigation)
            .expect("investigation should persist")
            .subjects()
            .clone();
        let draft = InvestigationWorkDraft {
            investigation: fixture.investigation,
            investigator: fixture.investigator,
            kind: InvestigationWorkKind::EvidenceReview,
            focus: InvestigationWorkFocus::evidence(fingerprint),
        };
        let work = validate_schedule_investigation_work(&registry, &fixture.state, draft)
            .expect("case-owned fingerprint should support evidence review")
            .commit(&mut fixture.state)
            .expect("evidence review should schedule");
        assert_eq!(
            fixture
                .state
                .legal()
                .get_investigation_work(work)
                .expect("scheduled evidence review should persist")
                .due_at(),
            SimTime::from_minutes(180)
        );

        for _ in 0..179 {
            assert!(run_tick(&registry, &mut fixture.state)
                .resolved_investigation_work
                .is_empty());
        }
        let outcome = run_tick(&registry, &mut fixture.state);
        assert_eq!(outcome.resolved_investigation_work, vec![work]);
        let record = fixture
            .state
            .legal()
            .get_investigation_work(work)
            .expect("completed evidence review should persist");
        let resolution = record
            .resolution()
            .expect("review should have a resolution");
        assert_eq!(resolution.outcome(), InvestigationWorkOutcome::Developed);
        let derived_id = resolution
            .derived_evidence()
            .expect("successful evidence review should derive forensic analysis");
        let source = fixture
            .state
            .legal()
            .get_evidence(fingerprint)
            .expect("source fingerprint should persist");
        let derived = fixture
            .state
            .legal()
            .get_evidence(derived_id)
            .expect("forensic analysis should persist");
        assert_eq!(derived.kind(), EvidenceKind::ForensicAnalysis);
        assert_eq!(derived.subject(), source.subject());
        assert_eq!(derived.origin(), source.origin());
        assert_eq!(derived.strength(), source.strength());
        assert_eq!(derived.reliability(), EvidenceReliability::Credible);
        assert_eq!(derived.admissibility(), source.admissibility());
        assert_eq!(derived.derived_from(), &BTreeSet::from([fingerprint]));
        assert_eq!(
            fixture
                .state
                .legal()
                .get_investigation(fixture.investigation)
                .expect("investigation should persist after review")
                .subjects(),
            &subjects_before
        );
        assert!(matches!(
            validate_schedule_investigation_work(&registry, &fixture.state, draft),
            Err(InvestigationWorkError::EvidenceAlreadyReviewed {
                evidence,
                derived
            }) if evidence == fingerprint && derived == derived_id
        ));
        validate_state(&fixture.state).expect("evidence review state should validate");
        validate_state_against_registry(&registry, &fixture.state)
            .expect("evidence review should remain registry-valid");
        validate_invariants(&fixture.state);
    }

    #[test]
    fn weak_pattern_analysis_is_inconclusive_without_fabricating_evidence() {
        let registry = build_registry();
        let mut fixture = make_fixture(
            5,
            EvidenceStrength::Weak,
            EvidenceReliability::Questionable,
            Admissibility::Inadmissible,
        );
        let work =
            validate_schedule_investigation_work(&registry, &fixture.state, work_draft(&fixture))
                .expect("weak pattern analysis should still be valid work")
                .commit(&mut fixture.state)
                .expect("weak pattern analysis should schedule");
        fixture.state.advance_clock(SimDuration::from_minutes(360));
        let plan = decide_investigation_work_resolution(
            &registry,
            &fixture.state,
            work,
            InvestigationWorkRandomness::new(12),
        )
        .expect("due weak work should resolve a plan");
        assert_eq!(plan.outcome(), InvestigationWorkOutcome::Inconclusive);
        assert!(plan.margin() < 0);
        validate_investigation_work_resolution_plan(&registry, &fixture.state, plan)
            .expect("fresh weak-work plan should validate")
            .commit(&mut fixture.state)
            .expect("inconclusive work should commit");
        let resolution = fixture
            .state
            .legal()
            .get_investigation_work(work)
            .expect("work should exist")
            .resolution()
            .expect("work should be completed");
        assert_eq!(resolution.outcome(), InvestigationWorkOutcome::Inconclusive);
        assert_eq!(resolution.derived_evidence(), None);
        assert_eq!(
            fixture
                .state
                .legal()
                .evidence_of_kind(EvidenceKind::PatternLink)
                .count(),
            0
        );
        validate_state(&fixture.state).expect("inconclusive work state should be valid");
        validate_state_against_registry(&registry, &fixture.state)
            .expect("inconclusive work should match authored definitions");
    }

    #[test]
    fn new_direct_evidence_supersedes_scheduled_pattern_analysis() {
        let registry = build_registry();
        let mut fixture = make_fixture(
            90,
            EvidenceStrength::Strong,
            EvidenceReliability::Credible,
            Admissibility::Admissible,
        );
        let work =
            validate_schedule_investigation_work(&registry, &fixture.state, work_draft(&fixture))
                .expect("pattern analysis should validate")
                .commit(&mut fixture.state)
                .expect("pattern analysis should schedule");
        let direct = add_evidence(
            &mut fixture.state,
            TestEvidenceDraft {
                investigation: fixture.investigation,
                police: fixture.police,
                subject: EntityRef::Character(fixture.target),
                origin: EntityRef::Character(fixture.first),
                strength: EvidenceStrength::Direct,
                reliability: EvidenceReliability::HighlyReliable,
                admissibility: Admissibility::Admissible,
            },
        );
        fixture.state.advance_clock(SimDuration::from_minutes(360));
        let plan = decide_investigation_work_resolution(
            &registry,
            &fixture.state,
            work,
            InvestigationWorkRandomness::new(0),
        )
        .expect("superseded work should still resolve normally");
        assert_eq!(plan.outcome(), InvestigationWorkOutcome::Superseded);
        validate_investigation_work_resolution_plan(&registry, &fixture.state, plan)
            .expect("superseded plan should validate")
            .commit(&mut fixture.state)
            .expect("superseded work should commit without derived evidence");
        let resolution = fixture
            .state
            .legal()
            .get_investigation_work(work)
            .expect("work should exist")
            .resolution()
            .expect("work should resolve");
        assert_eq!(resolution.outcome(), InvestigationWorkOutcome::Superseded);
        assert_eq!(resolution.superseded_by(), Some(direct));
        assert_eq!(resolution.derived_evidence(), None);
        assert_eq!(
            fixture
                .state
                .legal()
                .evidence_of_kind(EvidenceKind::PatternLink)
                .count(),
            0
        );
        validate_state(&fixture.state).expect("superseded work state should be valid");
        validate_state_against_registry(&registry, &fixture.state)
            .expect("superseded work should match authored definitions");
    }

    #[test]
    fn scheduling_is_versioned_deduplicated_and_blocks_investigator_release() {
        let registry = build_registry();
        let mut fixture = make_fixture(
            90,
            EvidenceStrength::Strong,
            EvidenceReliability::Credible,
            Admissibility::Admissible,
        );
        let stale_removal = validate_remove_investigator(
            &fixture.state,
            fixture.investigation,
            fixture.investigator,
        )
        .expect("investigator should initially be releasable");
        let stale_schedule =
            validate_schedule_investigation_work(&registry, &fixture.state, work_draft(&fixture))
                .expect("initial schedule token should validate");
        add_evidence(
            &mut fixture.state,
            TestEvidenceDraft {
                investigation: fixture.investigation,
                police: fixture.police,
                subject: EntityRef::Character(fixture.middle),
                origin: EntityRef::Character(fixture.target),
                strength: EvidenceStrength::Weak,
                reliability: EvidenceReliability::Mixed,
                admissibility: Admissibility::Unknown,
            },
        );
        assert!(matches!(
            stale_schedule.commit(&mut fixture.state),
            Err(InvestigationWorkError::StaleInvestigation { .. })
        ));

        let work =
            validate_schedule_investigation_work(&registry, &fixture.state, work_draft(&fixture))
                .expect("fresh schedule should validate after case change")
                .commit(&mut fixture.state)
                .expect("fresh schedule should commit");
        assert!(matches!(
            stale_removal.commit(&mut fixture.state),
            Err(InvestigationError::StaleInvestigation { .. })
        ));
        assert_eq!(
            validate_remove_investigator(
                &fixture.state,
                fixture.investigation,
                fixture.investigator,
            )
            .expect_err("scheduled work must block investigator release"),
            InvestigationError::ScheduledInvestigationWork {
                investigator: fixture.investigator,
                work,
            }
        );
        let reverse_focus = InvestigationWorkDraft {
            investigation: fixture.investigation,
            investigator: fixture.investigator,
            kind: InvestigationWorkKind::PatternAnalysis,
            focus: InvestigationWorkFocus::new(
                EntityRef::Character(fixture.target),
                EntityRef::Character(fixture.first),
            ),
        };
        assert_eq!(
            validate_schedule_investigation_work(&registry, &fixture.state, reverse_focus)
                .expect_err("reverse focus must canonicalize to the same scheduled work"),
            InvestigationWorkError::DuplicateScheduledWork { work }
        );
        validate_state(&fixture.state).expect("scheduled work dependencies should remain valid");
    }

    #[test]
    fn generic_evidence_path_cannot_forge_pattern_link() {
        let fixture = make_fixture(
            90,
            EvidenceStrength::Strong,
            EvidenceReliability::Credible,
            Admissibility::Admissible,
        );
        let error = match validate_add_evidence(
            &fixture.state,
            EvidenceDraft {
                investigation: fixture.investigation,
                custodian: fixture.police,
                subject: EntityRef::Character(fixture.target),
                origin: Some(EntityRef::Character(fixture.first)),
                kind: EvidenceKind::PatternLink,
                strength: EvidenceStrength::Strong,
                reliability: EvidenceReliability::Credible,
                admissibility: Admissibility::Admissible,
                discovered_at: fixture.state.now(),
            },
        ) {
            Ok(_) => panic!("pattern links must require the canonical analysis pipeline"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            InvestigationError::PatternLinkRequiresInvestigationWork
        );
    }

    #[test]
    fn save_round_trip_preserves_due_work_and_deterministic_resolution() {
        let registry = build_registry();
        let mut fixture = make_fixture(
            90,
            EvidenceStrength::Strong,
            EvidenceReliability::Credible,
            Admissibility::Admissible,
        );
        let work =
            validate_schedule_investigation_work(&registry, &fixture.state, work_draft(&fixture))
                .expect("pattern analysis should validate")
                .commit(&mut fixture.state)
                .expect("pattern analysis should schedule");
        for _ in 0..359 {
            run_tick(&registry, &mut fixture.state);
        }
        let mut restored = restore_save(
            &registry,
            build_save(&registry, &fixture.state).expect("pending work should save"),
        )
        .expect("pending work should restore");
        let original_outcome = run_tick(&registry, &mut fixture.state);
        let restored_outcome = run_tick(&registry, &mut restored);
        assert_eq!(original_outcome, restored_outcome);
        assert_eq!(original_outcome.resolved_investigation_work, vec![work]);

        let original_resolution = fixture
            .state
            .legal()
            .get_investigation_work(work)
            .expect("original work should exist")
            .resolution()
            .expect("original work should resolve")
            .clone();
        let restored_resolution = restored
            .legal()
            .get_investigation_work(work)
            .expect("restored work should exist")
            .resolution()
            .expect("restored work should resolve")
            .clone();
        assert_eq!(original_resolution, restored_resolution);
        let original_derived = original_resolution
            .derived_evidence()
            .expect("strong work should derive evidence");
        let restored_derived = restored_resolution
            .derived_evidence()
            .expect("restored strong work should derive evidence");
        assert_eq!(original_derived, restored_derived);
        assert_eq!(
            fixture
                .state
                .legal()
                .get_evidence(original_derived)
                .expect("original derived evidence should exist")
                .derived_from(),
            restored
                .legal()
                .get_evidence(restored_derived)
                .expect("restored derived evidence should exist")
                .derived_from()
        );

        let second_investigation = validate_open_investigation(
            &restored,
            InvestigationDraft {
                owner: fixture.police,
                title: "Post-restore association inquiry".to_owned(),
                subjects: BTreeSet::from([EntityRef::Character(fixture.first)]),
            },
        )
        .expect("post-restore investigation should validate")
        .commit(&mut restored)
        .expect("post-restore investigation should commit");
        validate_assign_investigator(
            &restored,
            second_investigation,
            fixture.investigator,
            InvestigatorRole::Lead,
        )
        .expect("post-restore investigator assignment should validate")
        .commit(&mut restored)
        .expect("post-restore investigator assignment should commit");
        add_evidence(
            &mut restored,
            TestEvidenceDraft {
                investigation: second_investigation,
                police: fixture.police,
                subject: EntityRef::Character(fixture.middle),
                origin: EntityRef::Character(fixture.first),
                strength: EvidenceStrength::Strong,
                reliability: EvidenceReliability::Credible,
                admissibility: Admissibility::Admissible,
            },
        );
        add_evidence(
            &mut restored,
            TestEvidenceDraft {
                investigation: second_investigation,
                police: fixture.police,
                subject: EntityRef::Character(fixture.target),
                origin: EntityRef::Character(fixture.middle),
                strength: EvidenceStrength::Strong,
                reliability: EvidenceReliability::Credible,
                admissibility: Admissibility::Admissible,
            },
        );
        let second_work = validate_schedule_investigation_work(
            &registry,
            &restored,
            InvestigationWorkDraft {
                investigation: second_investigation,
                investigator: fixture.investigator,
                kind: InvestigationWorkKind::PatternAnalysis,
                focus: InvestigationWorkFocus::new(
                    EntityRef::Character(fixture.first),
                    EntityRef::Character(fixture.target),
                ),
            },
        )
        .expect("post-restore pattern analysis should validate")
        .commit(&mut restored)
        .expect("post-restore pattern analysis should allocate a fresh work ID");
        assert!(second_work.raw() > work.raw());
        validate_state_against_registry(&registry, &restored)
            .expect("restored work should retain authored causal validity");
    }
}
