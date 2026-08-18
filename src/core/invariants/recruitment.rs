//! Release-safe structural validation for the recruitment subsystem.

use crate::core::entity::EntityRef;
use crate::core::id::{CharacterId, OrganizationId};
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::decisions::{DecisionContext, DecisionResponse, DecisionStatus};
use crate::history::HistoryEventKind;
use crate::intelligence::{
    InformationSourceKind, InformationTopic, KnowledgeHolder, Reliability, Specificity,
};
use crate::recruitment::recruitment_system::{
    calculate_recruitment_factors_from_context, calculate_recruitment_margin,
    classify_recruitment_outcome, select_perceived_legal_pressure_at, RecruitmentFactorContext,
};
use crate::recruitment::{RecruitmentAuthority, RecruitmentOutcome, RecruitmentPolicySource};
use crate::registry::Registry;
use crate::world::{ApprovalPolicy, OrganizationKind};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn validate_recruitment(state: &AppState) -> Result<(), StateValidationError> {
    let mut previous_attempt_by_pair: BTreeMap<
        (CharacterId, OrganizationId),
        crate::core::time::SimTime,
    > = BTreeMap::new();
    let mut recruitment_history_events = BTreeSet::new();
    let mut recruitment_outcome_information = BTreeSet::new();
    for attempt in state.recruitment.attempts() {
        let candidate = state.world.get_character(attempt.candidate()).ok_or(
            StateValidationError::InvalidRecruitmentAttempt {
                attempt: attempt.id(),
            },
        )?;
        let recruiter = state.world.get_character(attempt.recruiter()).ok_or(
            StateValidationError::InvalidRecruitmentAttempt {
                attempt: attempt.id(),
            },
        )?;
        let target = state
            .world
            .get_organization(attempt.target_organization())
            .ok_or(StateValidationError::InvalidRecruitmentAttempt {
                attempt: attempt.id(),
            })?;
        if candidate.id() == recruiter.id()
            || target.kind() != OrganizationKind::Criminal
            || attempt.occurred_at() > state.now()
            || attempt.previous_organization() == Some(attempt.target_organization())
            || attempt
                .previous_supervisor()
                .is_some_and(|supervisor| state.world.get_character(supervisor).is_none())
            || (attempt.previous_supervisor().is_some()
                && attempt.previous_organization().is_none())
        {
            return Err(StateValidationError::InvalidRecruitmentAttempt {
                attempt: attempt.id(),
            });
        }
        if let Some(previous_organization) = attempt.previous_organization() {
            let previous = state.world.get_organization(previous_organization).ok_or(
                StateValidationError::InvalidRecruitmentAttempt {
                    attempt: attempt.id(),
                },
            )?;
            if previous.kind() != OrganizationKind::Criminal {
                return Err(StateValidationError::InvalidRecruitmentAttempt {
                    attempt: attempt.id(),
                });
            }
        }

        let recruiter_relationship = attempt.recruiter_relationship();
        if recruiter_relationship.from() != attempt.candidate()
            || recruiter_relationship.to() != attempt.recruiter()
            || recruiter_relationship.dimensions().is_none()
            || recruiter_relationship.version().is_none()
            || recruiter_relationship.version() == Some(0)
        {
            return Err(StateValidationError::InvalidRecruitmentAttempt {
                attempt: attempt.id(),
            });
        }
        match (
            attempt.previous_supervisor(),
            attempt.incumbent_relationship(),
        ) {
            (None, None) => {}
            (Some(supervisor), Some(snapshot)) => {
                let snapshot_shape_is_valid = match (snapshot.dimensions(), snapshot.version()) {
                    (Some(_), Some(version)) => version > 0,
                    (None, None) => true,
                    (Some(_), None) | (None, Some(_)) => false,
                };
                if snapshot.from() != attempt.candidate()
                    || snapshot.to() != supervisor
                    || !snapshot_shape_is_valid
                {
                    return Err(StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    });
                }
            }
            (None, Some(_)) | (Some(_), None) => {
                return Err(StateValidationError::InvalidRecruitmentAttempt {
                    attempt: attempt.id(),
                });
            }
        }

        match attempt.authority() {
            RecruitmentAuthority::ExecutiveApproval => {}
            RecruitmentAuthority::ApprovedDecision {
                decision,
                mandate,
                manager,
                scope,
                mandate_version,
                manager_version,
                policy,
                policy_source,
            } => {
                let mandate_record = state.delegation.get_mandate(mandate).ok_or(
                    StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    },
                )?;
                let decision_record = state.decisions.get_decision(decision).ok_or(
                    StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    },
                )?;
                let approval_context = match decision_record.context() {
                    DecisionContext::RecruitmentApproval(context) => context,
                    DecisionContext::OperationException { .. } => {
                        return Err(StateValidationError::InvalidRecruitmentAttempt {
                            attempt: attempt.id(),
                        });
                    }
                };
                let approval_resolution = decision_record.resolution().ok_or(
                    StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    },
                )?;
                if manager != attempt.recruiter()
                    || mandate_record.manager() != manager
                    || mandate_record.organization() != attempt.target_organization()
                    || scope
                        != crate::delegation::ResponsibilityScope::Function(
                            crate::delegation::ResponsibilityFunction::Personnel,
                        )
                    || mandate_version == 0
                    || mandate_version > mandate_record.version()
                    || manager_version == 0
                    || manager_version > recruiter.version()
                    || policy != ApprovalPolicy::RequireApproval
                    || decision_record.status() != DecisionStatus::Resolved
                    || decision_record.requester() != attempt.recruiter()
                    || decision_record.recipient() != attempt.target_organization()
                    || approval_resolution.response() != DecisionResponse::Approve
                    || approval_resolution.resolved_at() != attempt.occurred_at()
                    || approval_context.target_organization() != attempt.target_organization()
                    || approval_context.recruiter() != attempt.recruiter()
                    || approval_context.candidate() != attempt.candidate()
                    || approval_context.approach() != attempt.approach()
                    || approval_context.authority().authority().mandate != mandate
                    || approval_context.authority().authority().manager != manager
                    || approval_context.authority().authority().scope != scope
                    || approval_context.authority().mandate_version() != mandate_version
                    || approval_context.authority().manager_version() != manager_version
                    || approval_context.authority().policy_source() != policy_source
                    || state
                        .recruitment
                        .attempt_for_approval_decision(decision)
                        .map(|record| record.id())
                        != Some(attempt.id())
                {
                    return Err(StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    });
                }
                if mandate_version == mandate_record.version()
                    && !mandate_record.scopes().contains(
                        &crate::delegation::ResponsibilityScope::Function(
                            crate::delegation::ResponsibilityFunction::Personnel,
                        ),
                    )
                {
                    return Err(StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    });
                }
                let valid_policy_source = match policy_source {
                    RecruitmentPolicySource::Organization(organization) => {
                        organization == attempt.target_organization()
                    }
                    RecruitmentPolicySource::Mandate(source_mandate) => source_mandate == mandate,
                };
                if !valid_policy_source {
                    return Err(StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    });
                }
            }
            RecruitmentAuthority::Delegated {
                mandate,
                manager,
                scope,
                mandate_version,
                manager_version,
                policy,
                policy_source,
            } => {
                let mandate_record = state.delegation.get_mandate(mandate).ok_or(
                    StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    },
                )?;
                if manager != attempt.recruiter()
                    || mandate_record.manager() != manager
                    || mandate_record.organization() != attempt.target_organization()
                    || scope
                        != crate::delegation::ResponsibilityScope::Function(
                            crate::delegation::ResponsibilityFunction::Personnel,
                        )
                    || mandate_version == 0
                    || mandate_version > mandate_record.version()
                    || manager_version == 0
                    || manager_version > recruiter.version()
                    || policy != ApprovalPolicy::Delegated
                {
                    return Err(StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    });
                }
                if mandate_version == mandate_record.version()
                    && !mandate_record.scopes().contains(
                        &crate::delegation::ResponsibilityScope::Function(
                            crate::delegation::ResponsibilityFunction::Personnel,
                        ),
                    )
                {
                    return Err(StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    });
                }
                let valid_policy_source = match policy_source {
                    RecruitmentPolicySource::Organization(organization) => {
                        organization == attempt.target_organization()
                    }
                    RecruitmentPolicySource::Mandate(source_mandate) => source_mandate == mandate,
                };
                if !valid_policy_source {
                    return Err(StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    });
                }
            }
        }

        if let Some(information_id) = attempt.pressure_information() {
            let information = state.intelligence.get_information(information_id).ok_or(
                StateValidationError::InvalidRecruitmentAttempt {
                    attempt: attempt.id(),
                },
            )?;
            if information.holder() != KnowledgeHolder::Character(attempt.candidate())
                || information.topic() != InformationTopic::PoliceActivity
                || information.subject() != EntityRef::Character(attempt.candidate())
                || information.recorded_at() > attempt.occurred_at()
                || information.observed_at() > attempt.occurred_at()
            {
                return Err(StateValidationError::InvalidRecruitmentAttempt {
                    attempt: attempt.id(),
                });
            }
        }

        let outcome_information = state
            .intelligence
            .get_information(attempt.outcome_information())
            .ok_or(StateValidationError::InvalidRecruitmentAttempt {
                attempt: attempt.id(),
            })?;
        if !recruitment_outcome_information.insert(attempt.outcome_information())
            || outcome_information.holder()
                != KnowledgeHolder::Organization(attempt.target_organization())
            || outcome_information.source_kind() != InformationSourceKind::AfterAction
            || outcome_information.topic() != InformationTopic::Personnel
            || outcome_information.source_entity()
                != Some(EntityRef::Character(attempt.recruiter()))
            || outcome_information.subject() != EntityRef::Character(attempt.candidate())
            || outcome_information.observed_at() != attempt.occurred_at()
            || outcome_information.recorded_at() != attempt.occurred_at()
            || outcome_information.reliability() != Reliability::DirectAccess
            || outcome_information.specificity() != Specificity::Precise
            || !outcome_information.derived_from().is_empty()
            || outcome_information.summary().trim().is_empty()
        {
            return Err(StateValidationError::InvalidRecruitmentAttempt {
                attempt: attempt.id(),
            });
        }

        let factors = attempt.factors();
        if factors.recruiter_influence() > 100
            || factors.drive_alignment() > 100
            || factors.relationship_support() > 100
            || factors.incumbent_attachment() > 100
            || factors.incumbent_resentment() > 100
            || factors.perceived_legal_pressure() > 100
            || attempt.outcome() != classify_recruitment_outcome(attempt.margin())
        {
            return Err(StateValidationError::InvalidRecruitmentAttempt {
                attempt: attempt.id(),
            });
        }

        let pair = (attempt.candidate(), attempt.target_organization());
        if let Some(previous_time) = previous_attempt_by_pair.insert(pair, attempt.occurred_at()) {
            if attempt.occurred_at() < previous_time {
                return Err(StateValidationError::InvalidRecruitmentAttempt {
                    attempt: attempt.id(),
                });
            }
        }

        match attempt.outcome() {
            RecruitmentOutcome::Accepted => {
                let history_id = attempt.history_event().ok_or(
                    StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    },
                )?;
                if !recruitment_history_events.insert(history_id) {
                    return Err(StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    });
                }
                let history = state.history.get_event(history_id).ok_or(
                    StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    },
                )?;
                if history.kind() != HistoryEventKind::Recruitment
                    || history.occurred_at() != attempt.occurred_at()
                    || !history
                        .entities()
                        .contains(&EntityRef::Character(attempt.candidate()))
                    || !history
                        .entities()
                        .contains(&EntityRef::Character(attempt.recruiter()))
                    || history
                        .entities()
                        .contains(&EntityRef::Organization(attempt.target_organization()))
                        == attempt.previous_organization().is_some()
                {
                    return Err(StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    });
                }
                // The membership consequence of an accepted attempt is part of the persisted
                // outcome: the candidate must now belong to the target organization under the
                // recruiter.
                let candidate = state.world.get_character(attempt.candidate()).ok_or(
                    StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    },
                )?;
                if candidate.organization() != Some(attempt.target_organization())
                    || candidate.supervisor() != Some(attempt.recruiter())
                {
                    return Err(StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    });
                }
            }
            RecruitmentOutcome::Refused => {
                if attempt.history_event().is_some() {
                    return Err(StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    });
                }
                // A refused attempt must leave membership unchanged: the candidate remains in
                // their pre-attempt organization (or stays independent).
                let candidate = state.world.get_character(attempt.candidate()).ok_or(
                    StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    },
                )?;
                if candidate.organization() != attempt.previous_organization() {
                    return Err(StateValidationError::InvalidRecruitmentAttempt {
                        attempt: attempt.id(),
                    });
                }
            }
        }
    }
    Ok(())
}

pub(super) fn validate_recruitment_against_registry(
    registry: &Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    let definition = registry.recruitment();
    let mut previous_attempt_by_pair: BTreeMap<
        (CharacterId, OrganizationId),
        crate::core::time::SimTime,
    > = BTreeMap::new();
    for attempt in state.recruitment.attempts() {
        let candidate = state.world.get_character(attempt.candidate()).ok_or(
            StateValidationError::InvalidRecruitmentAttempt {
                attempt: attempt.id(),
            },
        )?;
        let recruiter = state.world.get_character(attempt.recruiter()).ok_or(
            StateValidationError::InvalidRecruitmentAttempt {
                attempt: attempt.id(),
            },
        )?;
        let (expected_pressure_information, expected_legal_pressure) =
            select_perceived_legal_pressure_at(
                definition,
                state,
                attempt.candidate(),
                attempt.occurred_at(),
            );
        let expected_factors =
            calculate_recruitment_factors_from_context(RecruitmentFactorContext {
                definition,
                candidate,
                recruiter,
                approach: attempt.approach(),
                recruiter_relationship: attempt.recruiter_relationship(),
                incumbent_relationship: attempt.incumbent_relationship(),
                perceived_legal_pressure: expected_legal_pressure,
                had_previous_organization: attempt.previous_organization().is_some(),
            });
        if expected_factors != Some(attempt.factors())
            || attempt.pressure_information() != expected_pressure_information
            || attempt.margin() != calculate_recruitment_margin(definition, attempt.factors())
            || attempt.outcome() != classify_recruitment_outcome(attempt.margin())
        {
            return Err(StateValidationError::InvalidRecruitmentAttempt {
                attempt: attempt.id(),
            });
        }

        let pair = (attempt.candidate(), attempt.target_organization());
        if let Some(previous_time) = previous_attempt_by_pair.insert(pair, attempt.occurred_at()) {
            if attempt.occurred_at() < previous_time + definition.cooldown() {
                return Err(StateValidationError::InvalidRecruitmentAttempt {
                    attempt: attempt.id(),
                });
            }
        }
    }
    Ok(())
}
