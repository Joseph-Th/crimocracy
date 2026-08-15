//! Geographic legal authority assignments and deterministic incident intake routing.

use crate::core::id::{NeighborhoodId, OrganizationId, PatrolDeploymentId};
use crate::core::state::AppState;
use crate::legal::{JurisdictionDraft, JurisdictionRecord};
use crate::world::{Lifecycle, OrganizationKind};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum JurisdictionError {
    #[error("organization {0} does not exist")]
    MissingOrganization(OrganizationId),
    #[error("organization {0} cannot hold law-enforcement jurisdiction")]
    InvalidAuthorityKind(OrganizationId),
    #[error("organization {0} is not active")]
    InactiveAuthority(OrganizationId),
    #[error("jurisdiction must contain at least one neighborhood")]
    EmptyJurisdiction,
    #[error("neighborhood {0} does not exist or is not active")]
    MissingNeighborhood(NeighborhoodId),
    #[error(
        "organization {organization} cannot remove neighborhood {neighborhood} from jurisdiction while patrol deployment {deployment} is active"
    )]
    ActivePatrolDeployment {
        organization: OrganizationId,
        neighborhood: NeighborhoodId,
        deployment: PatrolDeploymentId,
    },
    #[error(
        "jurisdiction for organization {organization} changed after validation; expected version {expected:?}, found {found:?}"
    )]
    StaleJurisdiction {
        organization: OrganizationId,
        expected: Option<u32>,
        found: Option<u32>,
    },
}

#[derive(Debug)]
pub struct ValidatedJurisdiction {
    draft: JurisdictionDraft,
    expected_version: Option<u32>,
}

impl ValidatedJurisdiction {
    pub fn commit(self, state: &mut AppState) -> Result<OrganizationId, JurisdictionError> {
        let found_version = state
            .legal
            .get_jurisdiction(self.draft.organization)
            .map(JurisdictionRecord::version);
        if found_version != self.expected_version {
            return Err(JurisdictionError::StaleJurisdiction {
                organization: self.draft.organization,
                expected: self.expected_version,
                found: found_version,
            });
        }
        validate_jurisdiction_dependencies(state, &self.draft)?;
        let version = self
            .expected_version
            .unwrap_or(0)
            .checked_add(1)
            .expect("jurisdiction version counter exhausted");
        let organization = self.draft.organization;
        state.legal.set_jurisdiction(JurisdictionRecord {
            organization,
            neighborhoods: self.draft.neighborhoods,
            case_intake_priority: self.draft.case_intake_priority,
            version,
        });
        Ok(organization)
    }
}

pub fn validate_set_jurisdiction(
    state: &AppState,
    draft: JurisdictionDraft,
) -> Result<ValidatedJurisdiction, JurisdictionError> {
    validate_jurisdiction_dependencies(state, &draft)?;
    let expected_version = state
        .legal
        .get_jurisdiction(draft.organization)
        .map(JurisdictionRecord::version);
    Ok(ValidatedJurisdiction {
        draft,
        expected_version,
    })
}

pub fn resolve_case_intake_authority(
    state: &AppState,
    neighborhood: NeighborhoodId,
) -> Option<OrganizationId> {
    state
        .legal
        .jurisdictions_for_neighborhood(neighborhood)
        .filter(|jurisdiction| {
            state
                .world
                .get_organization(jurisdiction.organization())
                .is_some_and(|organization| {
                    organization.lifecycle() == Lifecycle::Active
                        && matches!(
                            organization.kind(),
                            OrganizationKind::LawEnforcement | OrganizationKind::LegalAuthority
                        )
                })
        })
        .fold(None, |best, jurisdiction| match best {
            None => Some(jurisdiction),
            Some(current)
                if jurisdiction.case_intake_priority().value()
                    > current.case_intake_priority().value()
                    || (jurisdiction.case_intake_priority() == current.case_intake_priority()
                        && jurisdiction.organization() < current.organization()) =>
            {
                Some(jurisdiction)
            }
            Some(current) => Some(current),
        })
        .map(JurisdictionRecord::organization)
}

pub fn resolve_police_response_authority(
    state: &AppState,
    neighborhood: NeighborhoodId,
) -> Option<OrganizationId> {
    state
        .legal
        .jurisdictions_for_neighborhood(neighborhood)
        .filter(|jurisdiction| {
            state
                .world
                .get_organization(jurisdiction.organization())
                .is_some_and(|organization| {
                    organization.lifecycle() == Lifecycle::Active
                        && organization.kind() == OrganizationKind::LawEnforcement
                })
        })
        .fold(None, |best, jurisdiction| match best {
            None => Some(jurisdiction),
            Some(current)
                if jurisdiction.case_intake_priority().value()
                    > current.case_intake_priority().value()
                    || (jurisdiction.case_intake_priority() == current.case_intake_priority()
                        && jurisdiction.organization() < current.organization()) =>
            {
                Some(jurisdiction)
            }
            Some(current) => Some(current),
        })
        .map(JurisdictionRecord::organization)
}

fn validate_jurisdiction_dependencies(
    state: &AppState,
    draft: &JurisdictionDraft,
) -> Result<(), JurisdictionError> {
    let organization = state
        .world
        .get_organization(draft.organization)
        .ok_or(JurisdictionError::MissingOrganization(draft.organization))?;
    match organization.kind() {
        OrganizationKind::LawEnforcement | OrganizationKind::LegalAuthority => {}
        OrganizationKind::Criminal
        | OrganizationKind::Political
        | OrganizationKind::Press
        | OrganizationKind::Labor
        | OrganizationKind::Civic
        | OrganizationKind::Commercial => {
            return Err(JurisdictionError::InvalidAuthorityKind(draft.organization));
        }
    }
    if organization.lifecycle() != Lifecycle::Active {
        return Err(JurisdictionError::InactiveAuthority(draft.organization));
    }
    if draft.neighborhoods.is_empty() {
        return Err(JurisdictionError::EmptyJurisdiction);
    }
    for neighborhood in &draft.neighborhoods {
        if !state
            .world
            .get_neighborhood(*neighborhood)
            .is_some_and(|record| record.lifecycle() == Lifecycle::Active)
        {
            return Err(JurisdictionError::MissingNeighborhood(*neighborhood));
        }
    }
    if let Some(current) = state.legal.get_jurisdiction(draft.organization) {
        for neighborhood in current.neighborhoods().difference(&draft.neighborhoods) {
            if let Some(deployment) = state
                .legal
                .active_patrol_for(draft.organization, *neighborhood)
            {
                return Err(JurisdictionError::ActivePatrolDeployment {
                    organization: draft.organization,
                    neighborhood: *neighborhood,
                    deployment: deployment.id(),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::invariants::{validate_invariants, validate_state};
    use crate::legal::JurisdictionDraft;
    use crate::world::world_system::{insert_neighborhood, insert_organization};
    use crate::world::{
        NeighborhoodDraft, NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile,
        NeighborhoodProfile, OrganizationDraft, Rating,
    };
    use std::collections::BTreeSet;

    fn make_fixture() -> (AppState, NeighborhoodId, OrganizationId, OrganizationId) {
        let registry = build_registry();
        let mut state = AppState::new(0x1A57_1933);
        let neighborhood = insert_neighborhood(
            &mut state,
            NeighborhoodDraft {
                name: "Jurisdiction Test Ward".to_owned(),
                profile: NeighborhoodProfile {
                    economy: NeighborhoodEconomyProfile {
                        wealth: Rating::try_new(50).expect("fixture rating should validate"),
                        commercial_activity: Rating::try_new(50)
                            .expect("fixture rating should validate"),
                        illicit_demand: Rating::try_new(50)
                            .expect("fixture rating should validate"),
                    },
                    institutions: NeighborhoodInstitutionProfile {
                        police_presence: Rating::try_new(60)
                            .expect("fixture rating should validate"),
                        political_influence: Rating::try_new(50)
                            .expect("fixture rating should validate"),
                        social_cohesion: Rating::try_new(50)
                            .expect("fixture rating should validate"),
                        visible_violence_tolerance: Rating::try_new(40)
                            .expect("fixture rating should validate"),
                    },
                },
            },
        )
        .expect("neighborhood fixture should validate");
        let first = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "First Precinct".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("first legal authority should validate");
        let second = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Second Precinct".to_owned(),
                kind: OrganizationKind::LawEnforcement,
            },
        )
        .expect("second legal authority should validate");
        (state, neighborhood, first, second)
    }

    #[test]
    fn case_intake_uses_priority_then_stable_organization_id() {
        let (mut state, neighborhood, first, second) = make_fixture();
        for (organization, priority) in [(first, 70), (second, 85)] {
            validate_set_jurisdiction(
                &state,
                JurisdictionDraft {
                    organization,
                    neighborhoods: BTreeSet::from([neighborhood]),
                    case_intake_priority: Rating::try_new(priority)
                        .expect("fixture priority should validate"),
                },
            )
            .expect("jurisdiction fixture should validate")
            .commit(&mut state)
            .expect("jurisdiction fixture should commit");
        }
        assert_eq!(
            resolve_case_intake_authority(&state, neighborhood),
            Some(second)
        );

        validate_set_jurisdiction(
            &state,
            JurisdictionDraft {
                organization: first,
                neighborhoods: BTreeSet::from([neighborhood]),
                case_intake_priority: Rating::try_new(85)
                    .expect("fixture priority should validate"),
            },
        )
        .expect("priority update should validate")
        .commit(&mut state)
        .expect("priority update should commit");
        assert!(
            first < second,
            "fixture IDs should be allocated in stable order"
        );
        assert_eq!(
            resolve_case_intake_authority(&state, neighborhood),
            Some(first)
        );
        validate_state(&state).expect("jurisdiction state should remain structurally valid");
        validate_invariants(&state);
    }

    #[test]
    fn stale_jurisdiction_token_cannot_overwrite_newer_assignment() {
        let (mut state, neighborhood, first, _second) = make_fixture();
        let draft = || JurisdictionDraft {
            organization: first,
            neighborhoods: BTreeSet::from([neighborhood]),
            case_intake_priority: Rating::try_new(70).expect("fixture priority should validate"),
        };
        let stale = validate_set_jurisdiction(&state, draft())
            .expect("initial jurisdiction token should validate");
        validate_set_jurisdiction(&state, draft())
            .expect("concurrent jurisdiction token should validate")
            .commit(&mut state)
            .expect("newer jurisdiction token should commit");

        let error = stale
            .commit(&mut state)
            .expect_err("stale jurisdiction token must be rejected");
        assert_eq!(
            error,
            JurisdictionError::StaleJurisdiction {
                organization: first,
                expected: None,
                found: Some(1),
            }
        );
        validate_invariants(&state);
    }

    #[test]
    fn criminal_organization_cannot_receive_legal_jurisdiction() {
        let registry = build_registry();
        let (mut state, neighborhood, _first, _second) = make_fixture();
        let criminal = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Not A Precinct".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("criminal organization fixture should validate");
        let error = validate_set_jurisdiction(
            &state,
            JurisdictionDraft {
                organization: criminal,
                neighborhoods: BTreeSet::from([neighborhood]),
                case_intake_priority: Rating::try_new(50)
                    .expect("fixture priority should validate"),
            },
        )
        .expect_err("criminal organization must not own legal jurisdiction");
        assert_eq!(error, JurisdictionError::InvalidAuthorityKind(criminal));
        validate_invariants(&state);
    }
}
