//! Staffed vice inquiries observe actual racket activity, rather than merely adding heat.

use super::{InvestigationError, ValidatedEvidence, validate_add_evidence};
use crate::core::entity::EntityRef;
use crate::core::id::{EnterpriseCycleId, EnterpriseId, EvidenceId, IdKind};
use crate::core::state::AppState;
use crate::enterprises::enterprise_execution::{
    is_enterprise_inquiry_evidence, resolve_location_neighborhood,
};
use crate::legal::{
    Admissibility, EvidenceDraft, EvidenceKind, EvidenceReliability, EvidenceStrength,
    InvestigationRecord,
};
use crate::world::CapabilityKind;
use std::collections::BTreeSet;

/// A staffed detective's chance of identifying the manager on an actual observed operating
/// cycle. Local police presence supports fieldwork, but does not substitute for a detective.
/// Both factors are bounded ratings; no campaign age or treasury factor enters the decision.
fn observation_chance(skill: u8, presence: u8) -> usize {
    usize::from(skill) * (100 + usize::from(presence)) / 200
}

/// The existing surveillance vocabulary distinguishes a personal vice observation from
/// the weak enterprise-only intake. The enterprise's manager assignment is immutable.
pub(crate) fn enterprise_observation_origin(
    kind: EvidenceKind,
    subject: EntityRef,
    origin: Option<EntityRef>,
) -> Option<EnterpriseId> {
    match (kind, subject, origin) {
        (EvidenceKind::Surveillance, EntityRef::Character(_), Some(EntityRef::Enterprise(id))) => {
            Some(id)
        }
        _ => None,
    }
}

pub(super) fn validate_observation_draft(
    state: &AppState,
    case: &InvestigationRecord,
    draft: &EvidenceDraft,
) -> Result<(), InvestigationError> {
    let Some(id) = enterprise_observation_origin(draft.kind, draft.subject, draft.origin) else {
        return Ok(());
    };
    let invalid = || InvestigationError::InvalidEnterpriseObservation(id);
    let enterprise = state.enterprises().get_enterprise(id).ok_or_else(invalid)?;
    let cycle = state.enterprises().latest_cycle(id).ok_or_else(invalid)?;
    let lead = case.lead_investigator().ok_or_else(invalid)?;
    if draft.subject != EntityRef::Character(enterprise.manager())
        || draft.discovered_at != state.now()
        || cycle.occurred_at() != state.now()
        || cycle.drew_enforcement_attention()
        || draft.strength != EvidenceStrength::Strong
        || draft.reliability != EvidenceReliability::Credible
        || draft.admissibility != Admissibility::Admissible
        || state.legal.active_arrest_for_character(lead).is_some()
        || state.legal.scheduled_work_for_investigator(lead).is_some()
        || !case.evidence().iter().any(|item| {
            state.legal.get_evidence(*item).is_some_and(|evidence| {
                evidence.discovered_at() < state.now()
                    && is_enterprise_inquiry_evidence(state, case, evidence, id)
            })
        })
        || state.legal.all_evidence().any(|record| {
            record.discovered_at() == state.now()
                && enterprise_observation_origin(record.kind(), record.subject(), record.origin())
                    == Some(id)
        })
    {
        return Err(invalid());
    }
    Ok(())
}

/// Observe only cycles committed this tick. The first exposure opens an inquiry, not a
/// retroactive identification. One idle lead can observe one racket in a minute, and the
/// same operating event cannot be observed twice merely because files overlap.
pub(crate) fn apply_enterprise_observations(
    state: &mut AppState,
    settled: &[EnterpriseCycleId],
) -> Result<Vec<EvidenceId>, InvestigationError> {
    let mut planned: Vec<ValidatedEvidence> = Vec::new();
    let mut used_cases = BTreeSet::new();
    let mut used_enterprises = BTreeSet::new();
    let mut rng = state.investigation_rng_mut().clone();
    for id in settled {
        let cycle = state
            .enterprises()
            .get_cycle(*id)
            .expect("settled cycle exists");
        if cycle.occurred_at() != state.now()
            || cycle.drew_enforcement_attention()
            || !used_enterprises.insert(cycle.enterprise())
        {
            continue;
        }
        let enterprise = state
            .enterprises()
            .get_enterprise(cycle.enterprise())
            .expect("settled enterprise exists");
        let target = EntityRef::Enterprise(enterprise.id());
        let subject = EntityRef::Character(enterprise.manager());
        let district = resolve_location_neighborhood(state, enterprise.location())
            .expect("settled enterprise has district");
        let presence = state
            .world()
            .get_neighborhood(district)
            .expect("district exists")
            .profile()
            .institutions
            .police_presence
            .value();
        let candidate = state
            .legal
            .active_investigations_for_subject(target)
            .filter_map(|case| {
                if used_cases.contains(&case.id()) || case.version() == u32::MAX {
                    return None;
                }
                let investigator = case.lead_investigator()?;
                if state
                    .legal
                    .scheduled_work_for_investigator(investigator)
                    .is_some()
                    || state
                        .legal
                        .active_arrest_for_character(investigator)
                        .is_some()
                {
                    return None;
                }
                let skill = state
                    .world()
                    .get_character(investigator)?
                    .capability(CapabilityKind::Investigation)?
                    .value();
                let has_intake = case.evidence().iter().any(|id| {
                    let evidence = state.legal.get_evidence(*id).expect("case evidence exists");
                    evidence.discovered_at() < state.now()
                        && is_enterprise_inquiry_evidence(state, case, evidence, enterprise.id())
                });
                let already_observed = case.evidence().iter().any(|id| {
                    let evidence = state.legal.get_evidence(*id).expect("case evidence exists");
                    evidence.subject() == subject
                        && evidence.origin() == Some(target)
                        && evidence.discovered_at() == state.now()
                        && evidence.kind() == EvidenceKind::Surveillance
                });
                (has_intake && !already_observed).then_some((
                    case.id(),
                    case.owner(),
                    observation_chance(skill, presence),
                ))
            })
            .max_by_key(|(id, _, chance)| (*chance, std::cmp::Reverse(*id)));
        let Some((case, owner, chance)) = candidate else {
            continue;
        };
        if chance == 0 {
            continue;
        }

        // A distinct observed trading event is independently collected primary evidence, not a
        // forensic derivative or an inference from the organization's balance. The model treats
        // successfully observed field conduct as lawfully obtained, credible surveillance.
        let token = match validate_add_evidence(
            state,
            EvidenceDraft {
                investigation: case,
                custodian: owner,
                subject,
                origin: Some(target),
                kind: EvidenceKind::Surveillance,
                strength: EvidenceStrength::Strong,
                reliability: EvidenceReliability::Credible,
                admissibility: Admissibility::Admissible,
                discovered_at: state.now(),
            },
        ) {
            Ok(token) => token,
            Err(
                InvestigationError::VersionCapacity(_)
                | InvestigationError::InvalidEnterpriseObservation(_),
            ) => continue,
            Err(error) => return Err(error),
        };
        if state.ids.reserve(IdKind::Evidence, 1).is_err() {
            continue;
        }
        used_cases.insert(case);
        if crate::core::simulation::draw_index(&mut rng, 100).expect("nonempty chance range")
            < chance
        {
            planned.push(token);
        }
    }
    if state
        .ids
        .reserve(
            IdKind::Evidence,
            u32::try_from(planned.len()).expect("bounded by cycle IDs"),
        )
        .is_err()
    {
        return Ok(Vec::new());
    }
    *state.investigation_rng_mut() = rng;
    Ok(planned
        .into_iter()
        .map(|token| {
            token
                .commit(state)
                .expect("preflighted distinct vice inquiries remain current")
        })
        .collect())
}
