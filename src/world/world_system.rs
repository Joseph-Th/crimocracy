//! Canonical world mutation systems; sibling `world` types remain passive records and indexes.

use crate::core::id::{
    ArrestId, BusinessId, BusinessOwnershipChangeId, CharacterId, ContactId, EnterpriseId,
    IdExhaustionError, IdKind, InformantId, InvestigationId, MandateId, NeighborhoodId,
    OperationId, OrganizationId, ProsecutionCaseId,
};
use crate::core::state::AppState;
use crate::enterprises::{EnterpriseLocation, EnterpriseStatus};
use crate::legal::ProsecutionCaseStatus;
use crate::operations::OperationStatus;
use crate::registry::Registry;
use crate::world::{
    BusinessDraft, BusinessOwner, BusinessOwnershipChangeRecord, BusinessRecord,
    CharacterCapabilities, CharacterDisposition, CharacterDraft, CharacterIdentity,
    CharacterMembership, CharacterRecord, CharacterRuntime, Lifecycle, NeighborhoodDraft,
    NeighborhoodRecord, OrganizationDraft, OrganizationKind, OrganizationRecord, PolicySetting,
};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum WorldError {
    #[error("name must not be empty")]
    EmptyName,
    #[error("organization {0} does not exist")]
    MissingOrganization(OrganizationId),
    #[error("organization {0} is not active")]
    InactiveOrganization(OrganizationId),
    #[error("character {0} does not exist")]
    MissingCharacter(CharacterId),
    #[error("character {0} is not active")]
    InactiveCharacter(CharacterId),
    #[error("neighborhood {0} does not exist")]
    MissingNeighborhood(NeighborhoodId),
    #[error("neighborhood {0} is not active")]
    InactiveNeighborhood(NeighborhoodId),
    #[error("business {0} does not exist")]
    MissingBusiness(BusinessId),
    #[error("business {0} is not active")]
    InactiveBusiness(BusinessId),
    #[error("business {business} is already owned by {owner:?}")]
    BusinessOwnershipUnchanged {
        business: BusinessId,
        owner: BusinessOwner,
    },
    #[error(
        "business {business} changed after validation; expected version {expected}, found {found}"
    )]
    StaleBusiness {
        business: BusinessId,
        expected: u32,
        found: u32,
    },
    #[error("business {business} supports active enterprise {enterprise} for organization {organization}")]
    ActiveEnterpriseSupport {
        business: BusinessId,
        enterprise: EnterpriseId,
        organization: OrganizationId,
    },
    #[error("business {business} is the hosted venue of active enterprise {enterprise} for organization {organization}")]
    ActiveEnterpriseHost {
        business: BusinessId,
        enterprise: EnterpriseId,
        organization: OrganizationId,
    },
    #[error("supervisor {supervisor} does not belong to requested organization {organization:?}")]
    SupervisorOrganizationMismatch {
        supervisor: CharacterId,
        organization: Option<OrganizationId>,
    },
    #[error("character {character} cannot have a supervisor without belonging to an organization")]
    SupervisorWithoutOrganization { character: CharacterId },
    #[error("character {character} cannot supervise itself")]
    SelfSupervision { character: CharacterId },
    #[error("reassignment would create a supervision cycle involving character {character}")]
    SupervisionCycle { character: CharacterId },
    #[error("character {character} is assigned to active operation {operation}")]
    ActiveOperationAssignment {
        character: CharacterId,
        operation: OperationId,
    },
    #[error("character {character} owns active mandate {mandate}")]
    ActiveMandateAssignment {
        character: CharacterId,
        mandate: MandateId,
    },
    #[error("character {character} is assigned to active investigation {investigation}")]
    ActiveInvestigationAssignment {
        character: CharacterId,
        investigation: InvestigationId,
    },
    #[error("character {character} is detained under arrest {arrest}")]
    ActiveArrestAssignment {
        character: CharacterId,
        arrest: ArrestId,
    },
    #[error("character {character} is lead prosecutor for open prosecution case {case}")]
    ActiveProsecutionAssignment {
        character: CharacterId,
        case: ProsecutionCaseId,
    },
    #[error("supervisor {supervisor} is detained under arrest {arrest}")]
    DetainedSupervisor {
        supervisor: CharacterId,
        arrest: ArrestId,
    },
    #[error("supervisor {0} is not active")]
    InactiveSupervisor(CharacterId),
    #[error(
        "character {character} is active informant {informant} for target handler organization {handler}"
    )]
    ActiveInformantHandlerAssignment {
        character: CharacterId,
        handler: OrganizationId,
        informant: InformantId,
    },
    #[error("character {character} handles active institutional contact {contact}")]
    ActiveInstitutionalContactHandler {
        character: CharacterId,
        contact: ContactId,
    },
    #[error("character {character} is active institutional contact {contact}")]
    ActiveInstitutionalContactAssignment {
        character: CharacterId,
        contact: ContactId,
    },
    #[error("character {character} still supervises direct report {direct_report}")]
    DirectReportAssignment {
        character: CharacterId,
        direct_report: CharacterId,
    },
    #[error("character {character} changed after validation; expected version {expected}, found {found}")]
    StaleCharacter {
        character: CharacterId,
        expected: u32,
        found: u32,
    },
    #[error(
        "organization {0} is not a criminal organization and cannot be the player organization"
    )]
    InvalidPlayerOrganization(OrganizationId),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

pub fn insert_organization(
    registry: &Registry,
    state: &mut AppState,
    draft: OrganizationDraft,
) -> Result<OrganizationId, WorldError> {
    if draft.name.trim().is_empty() {
        return Err(WorldError::EmptyName);
    }
    let id = state.ids.next_organization()?;
    let policies = registry.default_policies();
    state.world.insert_organization(OrganizationRecord {
        id,
        name: draft.name,
        kind: draft.kind,
        lifecycle: Lifecycle::Active,
        policies,
    });
    Ok(id)
}

pub fn designate_player_organization(
    state: &mut AppState,
    organization: OrganizationId,
) -> Result<(), WorldError> {
    let record = state
        .world
        .get_organization(organization)
        .ok_or(WorldError::MissingOrganization(organization))?;
    if record.lifecycle() != Lifecycle::Active {
        return Err(WorldError::InactiveOrganization(organization));
    }
    if record.kind() != OrganizationKind::Criminal {
        return Err(WorldError::InvalidPlayerOrganization(organization));
    }
    state.set_player_organization(organization);
    Ok(())
}

pub fn insert_neighborhood(
    state: &mut AppState,
    draft: NeighborhoodDraft,
) -> Result<NeighborhoodId, WorldError> {
    if draft.name.trim().is_empty() {
        return Err(WorldError::EmptyName);
    }
    let id = state.ids.next_neighborhood()?;
    state.world.insert_neighborhood(NeighborhoodRecord {
        id,
        name: draft.name,
        profile: draft.profile,
        lifecycle: Lifecycle::Active,
    });
    Ok(id)
}

pub fn insert_character(
    registry: &Registry,
    state: &mut AppState,
    draft: CharacterDraft,
) -> Result<CharacterId, WorldError> {
    if draft.name.trim().is_empty() {
        return Err(WorldError::EmptyName);
    }
    validate_membership(state, draft.organization, draft.supervisor)?;
    for kind in draft.capabilities.keys() {
        registry.get_capability(*kind);
    }
    for kind in &draft.traits {
        registry.get_trait(*kind);
    }
    for kind in draft.drives.keys() {
        registry.get_drive(*kind);
    }

    let id = state.ids.next_character()?;
    state.world.insert_character(CharacterRecord {
        identity: CharacterIdentity {
            id,
            name: draft.name,
        },
        membership: CharacterMembership {
            organization: draft.organization,
            supervisor: draft.supervisor,
            autonomy: draft.autonomy,
        },
        capabilities: CharacterCapabilities {
            ratings: draft.capabilities,
        },
        disposition: CharacterDisposition {
            traits: draft.traits,
            drives: draft.drives,
        },
        runtime: CharacterRuntime {
            lifecycle: Lifecycle::Active,
            version: 1,
        },
    });
    Ok(id)
}

#[derive(Debug)]
pub struct ValidatedCharacterReassignment {
    character: CharacterId,
    organization: Option<OrganizationId>,
    supervisor: Option<CharacterId>,
    expected_version: u32,
}

impl ValidatedCharacterReassignment {
    pub fn commit(self, state: &mut AppState) -> Result<(), WorldError> {
        let record = state
            .world
            .get_character(self.character)
            .ok_or(WorldError::MissingCharacter(self.character))?;
        if record.version() != self.expected_version {
            return Err(WorldError::StaleCharacter {
                character: self.character,
                expected: self.expected_version,
                found: record.version(),
            });
        }
        validate_reassignment_preconditions(
            state,
            self.character,
            self.organization,
            self.supervisor,
        )?;
        state
            .world
            .reassign_character(self.character, self.organization, self.supervisor);
        Ok(())
    }
}

pub fn validate_reassign_character(
    state: &AppState,
    character: CharacterId,
    organization: Option<OrganizationId>,
    supervisor: Option<CharacterId>,
) -> Result<ValidatedCharacterReassignment, WorldError> {
    let record = state
        .world
        .get_character(character)
        .ok_or(WorldError::MissingCharacter(character))?;
    validate_reassignment_preconditions(state, character, organization, supervisor)?;

    Ok(ValidatedCharacterReassignment {
        character,
        organization,
        supervisor,
        expected_version: record.version(),
    })
}

fn validate_reassignment_preconditions(
    state: &AppState,
    character: CharacterId,
    organization: Option<OrganizationId>,
    supervisor: Option<CharacterId>,
) -> Result<(), WorldError> {
    let record = state
        .world
        .get_character(character)
        .ok_or(WorldError::MissingCharacter(character))?;
    if record.lifecycle() != Lifecycle::Active {
        return Err(WorldError::InactiveCharacter(character));
    }
    if supervisor == Some(character) {
        return Err(WorldError::SelfSupervision { character });
    }
    validate_membership(state, organization, supervisor)?;

    // A detained character cannot be given a new supervisor or organization regardless of whether
    // the organization changes: custody blocks new supervisory assignments per the custody contract.
    if let Some(arrest) = state.legal.active_arrest_for_character(character) {
        return Err(WorldError::ActiveArrestAssignment {
            character,
            arrest: arrest.id(),
        });
    }

    let organization_changed = organization != record.organization();
    let supervisor_changed = supervisor != record.supervisor();
    if organization_changed {
        if let Some(case) = state
            .legal
            .prosecution_cases_for_lead(character)
            .find(|case| case.status() == ProsecutionCaseStatus::Reviewing)
        {
            return Err(WorldError::ActiveProsecutionAssignment {
                character,
                case: case.id(),
            });
        }
        if let Some(contact) = state.contacts.active_contacts_for_handler(character).next() {
            return Err(WorldError::ActiveInstitutionalContactHandler {
                character,
                contact: contact.id(),
            });
        }
        if let Some(contact) = state
            .contacts
            .active_contacts_for_character(character)
            .next()
        {
            return Err(WorldError::ActiveInstitutionalContactAssignment {
                character,
                contact: contact.id(),
            });
        }
        if let Some(handler) = organization {
            if let Some(informant) = state.legal.active_informant_for(character, handler) {
                return Err(WorldError::ActiveInformantHandlerAssignment {
                    character,
                    handler,
                    informant: informant.id(),
                });
            }
        }
    }
    if organization_changed || supervisor_changed {
        if let Some(mandate) = state.delegation.active_for_manager(character) {
            return Err(WorldError::ActiveMandateAssignment {
                character,
                mandate: mandate.id(),
            });
        }
        if let Some(investigation) = state.legal.active_investigation_for_investigator(character) {
            return Err(WorldError::ActiveInvestigationAssignment {
                character,
                investigation: investigation.id(),
            });
        }
        for operation in state.operations.operations() {
            match operation.status() {
                OperationStatus::Authorized
                | OperationStatus::InProgress
                | OperationStatus::AwaitingDecision => {
                    if operation.leader() == character
                        || operation
                            .roles()
                            .values()
                            .any(|participant| *participant == character)
                    {
                        return Err(WorldError::ActiveOperationAssignment {
                            character,
                            operation: operation.id(),
                        });
                    }
                }
                OperationStatus::Completed | OperationStatus::Aborted => {}
            }
        }
        if organization_changed {
            if let Some(direct_report) = state.world.direct_reports(character).next() {
                return Err(WorldError::DirectReportAssignment {
                    character,
                    direct_report: direct_report.id(),
                });
            }
        }
    }

    let mut cursor = supervisor;
    while let Some(current) = cursor {
        if current == character {
            return Err(WorldError::SupervisionCycle { character });
        }
        cursor = state
            .world
            .get_character(current)
            .and_then(|record| record.supervisor());
    }
    Ok(())
}

pub fn insert_business(
    registry: &Registry,
    state: &mut AppState,
    draft: BusinessDraft,
) -> Result<BusinessId, WorldError> {
    if draft.name.trim().is_empty() {
        return Err(WorldError::EmptyName);
    }
    let neighborhood = state
        .world
        .get_neighborhood(draft.neighborhood)
        .ok_or(WorldError::MissingNeighborhood(draft.neighborhood))?;
    if neighborhood.lifecycle() != Lifecycle::Active {
        return Err(WorldError::InactiveNeighborhood(draft.neighborhood));
    }
    registry.get_business(draft.kind);
    validate_business_owner(state, draft.owner)?;
    state
        .ids
        .reserve_many(&[(IdKind::Business, 1), (IdKind::BusinessOwnershipChange, 1)])?;

    let id = state.ids.next_business()?;
    let ownership_change = state.ids.next_business_ownership_change()?;
    let changed_at = state.now();
    let BusinessDraft {
        name,
        kind,
        functions,
        neighborhood,
        owner,
    } = draft;
    state.world.insert_business(
        BusinessRecord {
            id,
            name,
            kind,
            functions,
            neighborhood,
            owner,
            lifecycle: Lifecycle::Active,
            version: 1,
        },
        BusinessOwnershipChangeRecord {
            id: ownership_change,
            business: id,
            previous_owner: None,
            new_owner: owner,
            changed_at,
            resulting_business_version: 1,
        },
    );
    Ok(id)
}

#[derive(Debug)]
pub struct ValidatedBusinessOwnershipTransfer {
    business: BusinessId,
    new_owner: BusinessOwner,
    expected_version: u32,
    previous_owner: BusinessOwner,
}

impl ValidatedBusinessOwnershipTransfer {
    pub fn commit(self, state: &mut AppState) -> Result<BusinessOwnershipChangeId, WorldError> {
        let record = validate_transferable_business(state, self.business)?;
        if record.version() != self.expected_version {
            return Err(WorldError::StaleBusiness {
                business: self.business,
                expected: self.expected_version,
                found: record.version(),
            });
        }
        if record.owner() != self.previous_owner {
            return Err(WorldError::StaleBusiness {
                business: self.business,
                expected: self.expected_version,
                found: record.version(),
            });
        }
        validate_business_owner(state, self.new_owner)?;
        validate_business_support_ownership_change(state, self.business, self.new_owner)?;
        if self.new_owner == self.previous_owner {
            return Err(WorldError::BusinessOwnershipUnchanged {
                business: self.business,
                owner: self.new_owner,
            });
        }
        let resulting_business_version = self
            .expected_version
            .checked_add(1)
            .expect("business version counter exhausted");
        let id = state.ids.next_business_ownership_change()?;
        state
            .world
            .transfer_business_ownership(BusinessOwnershipChangeRecord {
                id,
                business: self.business,
                previous_owner: Some(self.previous_owner),
                new_owner: self.new_owner,
                changed_at: state.now(),
                resulting_business_version,
            });
        Ok(id)
    }
}

pub fn validate_transfer_business_ownership(
    state: &AppState,
    business: BusinessId,
    new_owner: BusinessOwner,
) -> Result<ValidatedBusinessOwnershipTransfer, WorldError> {
    let record = validate_transferable_business(state, business)?;
    validate_business_owner(state, new_owner)?;
    validate_business_support_ownership_change(state, business, new_owner)?;
    if record.owner() == new_owner {
        return Err(WorldError::BusinessOwnershipUnchanged {
            business,
            owner: new_owner,
        });
    }
    Ok(ValidatedBusinessOwnershipTransfer {
        business,
        new_owner,
        expected_version: record.version(),
        previous_owner: record.owner(),
    })
}

fn validate_business_support_ownership_change(
    state: &AppState,
    business: BusinessId,
    new_owner: BusinessOwner,
) -> Result<(), WorldError> {
    for enterprise in state
        .enterprises
        .enterprises_supported_by_business(business)
    {
        if enterprise.status() == EnterpriseStatus::Active
            && new_owner != BusinessOwner::Organization(enterprise.organization())
        {
            return Err(WorldError::ActiveEnterpriseSupport {
                business,
                enterprise: enterprise.id(),
                organization: enterprise.organization(),
            });
        }
    }
    // A business that hosts an active racket as its venue is locked the same way a support
    // business is: the racket cannot keep settling at a venue the organization no longer owns.
    for enterprise in state
        .enterprises
        .enterprises_at(EnterpriseLocation::Business(business))
    {
        if enterprise.status() == EnterpriseStatus::Active
            && new_owner != BusinessOwner::Organization(enterprise.organization())
        {
            return Err(WorldError::ActiveEnterpriseHost {
                business,
                enterprise: enterprise.id(),
                organization: enterprise.organization(),
            });
        }
    }
    Ok(())
}

fn validate_transferable_business(
    state: &AppState,
    business: BusinessId,
) -> Result<&BusinessRecord, WorldError> {
    let record = state
        .world
        .get_business(business)
        .ok_or(WorldError::MissingBusiness(business))?;
    if record.lifecycle() != Lifecycle::Active {
        return Err(WorldError::InactiveBusiness(business));
    }
    Ok(record)
}

fn validate_business_owner(state: &AppState, owner: BusinessOwner) -> Result<(), WorldError> {
    match owner {
        BusinessOwner::Independent => Ok(()),
        BusinessOwner::Organization(id) => {
            let organization = state
                .world
                .get_organization(id)
                .ok_or(WorldError::MissingOrganization(id))?;
            if organization.lifecycle() != Lifecycle::Active {
                return Err(WorldError::InactiveOrganization(id));
            }
            Ok(())
        }
        BusinessOwner::Character(id) => {
            let character = state
                .world
                .get_character(id)
                .ok_or(WorldError::MissingCharacter(id))?;
            if character.lifecycle() != Lifecycle::Active {
                return Err(WorldError::InactiveCharacter(id));
            }
            Ok(())
        }
    }
}

pub fn set_policy(
    registry: &Registry,
    state: &mut AppState,
    organization: OrganizationId,
    setting: PolicySetting,
) -> Result<(), WorldError> {
    let organization_record = state
        .world
        .get_organization(organization)
        .ok_or(WorldError::MissingOrganization(organization))?;
    if organization_record.lifecycle() != Lifecycle::Active {
        return Err(WorldError::InactiveOrganization(organization));
    }
    registry.get_policy(setting.kind());
    state.world.set_policy(organization, setting);
    Ok(())
}

fn validate_membership(
    state: &AppState,
    organization: Option<OrganizationId>,
    supervisor: Option<CharacterId>,
) -> Result<(), WorldError> {
    if let Some(organization_id) = organization {
        let organization_record = state
            .world
            .get_organization(organization_id)
            .ok_or(WorldError::MissingOrganization(organization_id))?;
        if organization_record.lifecycle() != Lifecycle::Active {
            return Err(WorldError::InactiveOrganization(organization_id));
        }
    }
    // A supervision chain is an organization hierarchy; an unassigned character cannot report
    // to a supervisor because no organization would own the relationship.
    if let Some(supervisor_id) = supervisor {
        if organization.is_none() {
            return Err(WorldError::SupervisorWithoutOrganization {
                character: supervisor_id,
            });
        }
        let supervisor_record = state
            .world
            .get_character(supervisor_id)
            .ok_or(WorldError::MissingCharacter(supervisor_id))?;
        if supervisor_record.lifecycle() != Lifecycle::Active {
            return Err(WorldError::InactiveSupervisor(supervisor_id));
        }
        if let Some(arrest) = state.legal.active_arrest_for_character(supervisor_id) {
            return Err(WorldError::DetainedSupervisor {
                supervisor: supervisor_id,
                arrest: arrest.id(),
            });
        }
        if supervisor_record.organization() != organization {
            return Err(WorldError::SupervisorOrganizationMismatch {
                supervisor: supervisor_id,
                organization,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::invariants::validate_invariants;
    use crate::core::time::{SimDuration, SimTime};
    use crate::delegation::delegation_system::validate_assign_mandate;
    use crate::delegation::{MandateDraft, ResponsibilityFunction, ResponsibilityScope};
    use crate::world::{
        AutonomyLevel, BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner,
        CharacterDraft, NeighborhoodDraft, NeighborhoodEconomyProfile,
        NeighborhoodInstitutionProfile, NeighborhoodProfile, OrganizationDraft, OrganizationKind,
        Rating,
    };
    use std::collections::{BTreeMap, BTreeSet};

    fn make_test_character(
        registry: &Registry,
        state: &mut AppState,
        name: &str,
        organization: OrganizationId,
        supervisor: Option<CharacterId>,
    ) -> CharacterId {
        insert_character(
            registry,
            state,
            CharacterDraft {
                name: name.to_owned(),
                organization: Some(organization),
                supervisor,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("test character should validate")
    }

    fn rating(value: u8) -> Rating {
        Rating::try_new(value).expect("test rating must be valid")
    }

    fn make_test_business(
        registry: &Registry,
        state: &mut AppState,
        owner: BusinessOwner,
    ) -> BusinessId {
        let neighborhood = insert_neighborhood(
            state,
            NeighborhoodDraft {
                name: "Ownership Test Ward".to_owned(),
                profile: NeighborhoodProfile {
                    economy: NeighborhoodEconomyProfile {
                        wealth: rating(50),
                        commercial_activity: rating(60),
                        illicit_demand: rating(30),
                    },
                    institutions: NeighborhoodInstitutionProfile {
                        police_presence: rating(50),
                        political_influence: rating(50),
                        social_cohesion: rating(50),
                        visible_violence_tolerance: rating(20),
                    },
                },
            },
        )
        .expect("test neighborhood should validate");
        insert_business(
            registry,
            state,
            BusinessDraft {
                name: "Ownership Test Business".to_owned(),
                kind: BusinessKind::Retail,
                functions: BTreeSet::from([
                    BusinessFunction::CashIntensive,
                    BusinessFunction::CustomerAccess,
                ]),
                neighborhood,
                owner,
            },
        )
        .expect("test business should validate")
    }

    #[test]
    fn business_ownership_transfer_updates_indexes_and_preserves_versioned_history() {
        let registry = build_registry();
        let mut state = AppState::new(0x0B51_0001);
        let first_owner = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "First Holding Company".to_owned(),
                kind: OrganizationKind::Commercial,
            },
        )
        .expect("first owner should validate");
        let second_owner = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Second Holding Company".to_owned(),
                kind: OrganizationKind::Commercial,
            },
        )
        .expect("second owner should validate");
        let individual_owner = make_test_character(
            &registry,
            &mut state,
            "Individual Proprietor",
            second_owner,
            None,
        );
        let business = make_test_business(
            &registry,
            &mut state,
            BusinessOwner::Organization(first_owner),
        );

        let initial = state
            .world()
            .get_business_ownership_change_for_version(business, 1)
            .expect("initial ownership should be durable");
        assert_eq!(initial.previous_owner(), None);
        assert_eq!(
            initial.new_owner(),
            BusinessOwner::Organization(first_owner)
        );
        assert_eq!(initial.changed_at(), SimTime::ZERO);
        assert_eq!(
            state
                .world()
                .businesses_owned_by_organization(first_owner)
                .count(),
            1
        );

        state.advance_clock(SimDuration::from_minutes(15));
        let transferred = validate_transfer_business_ownership(
            &state,
            business,
            BusinessOwner::Organization(second_owner),
        )
        .expect("ownership transfer should validate")
        .commit(&mut state)
        .expect("ownership transfer should commit");

        let record = state
            .world()
            .get_business(business)
            .expect("business should remain present");
        assert_eq!(record.owner(), BusinessOwner::Organization(second_owner));
        assert_eq!(record.version(), 2);
        assert_eq!(
            state
                .world()
                .businesses_owned_by_organization(first_owner)
                .count(),
            0
        );
        assert_eq!(
            state
                .world()
                .businesses_owned_by_organization(second_owner)
                .count(),
            1
        );
        let change = state
            .world()
            .business_ownership_history(business)
            .find(|record| record.previous_owner().is_some())
            .expect("ownership change should persist");
        assert_eq!(change.id(), transferred);
        assert_eq!(
            change.previous_owner(),
            Some(BusinessOwner::Organization(first_owner))
        );
        assert_eq!(
            change.new_owner(),
            BusinessOwner::Organization(second_owner)
        );
        assert_eq!(change.changed_at(), SimTime::from_minutes(15));
        assert_eq!(change.resulting_business_version(), 2);
        assert_eq!(
            state.world().business_ownership_history(business).count(),
            2
        );
        assert_eq!(
            state.world().business_owner_at(business, SimTime::ZERO),
            Some(BusinessOwner::Organization(first_owner))
        );
        assert_eq!(
            state
                .world()
                .business_owner_at(business, SimTime::from_minutes(15)),
            Some(BusinessOwner::Organization(second_owner))
        );

        state.advance_clock(SimDuration::from_minutes(5));
        validate_transfer_business_ownership(
            &state,
            business,
            BusinessOwner::Character(individual_owner),
        )
        .expect("character ownership transfer should validate")
        .commit(&mut state)
        .expect("character ownership transfer should commit");
        let record = state
            .world()
            .get_business(business)
            .expect("business should remain present after character transfer");
        assert_eq!(record.owner(), BusinessOwner::Character(individual_owner));
        assert_eq!(record.version(), 3);
        assert_eq!(
            state
                .world()
                .businesses_owned_by_organization(second_owner)
                .count(),
            0
        );
        assert_eq!(
            state
                .world()
                .businesses_ever_owned_by_organization(first_owner)
                .count(),
            1
        );
        assert_eq!(
            state
                .world()
                .businesses_ever_owned_by_organization(second_owner)
                .count(),
            1
        );
        assert_eq!(
            state
                .world()
                .businesses_owned_by_character(individual_owner)
                .count(),
            1
        );
        assert_eq!(
            state
                .world()
                .businesses_ever_owned_by_character(individual_owner)
                .count(),
            1
        );
        assert_eq!(
            state.world().business_ownership_history(business).count(),
            3
        );
        assert_eq!(
            state
                .world()
                .business_owner_at(business, SimTime::from_minutes(20)),
            Some(BusinessOwner::Character(individual_owner))
        );
        assert!(state.world().business_was_owned_during(
            business,
            BusinessOwner::Organization(second_owner),
            SimTime::from_minutes(15),
            SimTime::from_minutes(20),
        ));
        assert!(!state.world().business_was_owned_during(
            business,
            BusinessOwner::Organization(first_owner),
            SimTime::from_minutes(15),
            SimTime::from_minutes(20),
        ));
        assert!(!state.world().business_was_owned_during(
            business,
            BusinessOwner::Organization(second_owner),
            SimTime::from_minutes(20),
            SimTime::from_minutes(20),
        ));
        assert!(state.world().business_was_owned_during(
            business,
            BusinessOwner::Character(individual_owner),
            SimTime::from_minutes(20),
            SimTime::from_minutes(20),
        ));
        validate_invariants(&state);
    }

    #[test]
    fn stale_business_ownership_token_cannot_overwrite_newer_title() {
        let registry = build_registry();
        let mut state = AppState::new(0x0B51_0002);
        let first_owner = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Initial Owner".to_owned(),
                kind: OrganizationKind::Commercial,
            },
        )
        .expect("first owner should validate");
        let intended_owner = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Intended Buyer".to_owned(),
                kind: OrganizationKind::Commercial,
            },
        )
        .expect("intended owner should validate");
        let business = make_test_business(
            &registry,
            &mut state,
            BusinessOwner::Organization(first_owner),
        );
        let stale = validate_transfer_business_ownership(
            &state,
            business,
            BusinessOwner::Organization(intended_owner),
        )
        .expect("first transfer should validate");
        validate_transfer_business_ownership(&state, business, BusinessOwner::Independent)
            .expect("newer transfer should validate")
            .commit(&mut state)
            .expect("newer transfer should commit");

        let error = stale
            .commit(&mut state)
            .expect_err("stale transfer must not overwrite newer title");
        assert_eq!(
            error,
            WorldError::StaleBusiness {
                business,
                expected: 1,
                found: 2,
            }
        );
        assert_eq!(
            state
                .world()
                .get_business(business)
                .expect("business should remain present")
                .owner(),
            BusinessOwner::Independent
        );
        assert_eq!(
            state.world().business_ownership_history(business).count(),
            2
        );
        validate_invariants(&state);
    }

    #[test]
    fn reassignment_rejects_supervision_cycle_without_mutation() {
        let registry = build_registry();
        let mut state = AppState::new(7);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Test Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("test organization should validate");
        let boss = make_test_character(&registry, &mut state, "Boss", organization, None);
        let lieutenant = make_test_character(
            &registry,
            &mut state,
            "Lieutenant",
            organization,
            Some(boss),
        );
        let soldier = make_test_character(
            &registry,
            &mut state,
            "Soldier",
            organization,
            Some(lieutenant),
        );

        let error = validate_reassign_character(&state, boss, Some(organization), Some(soldier))
            .expect_err("cycle must be rejected before mutation");
        assert_eq!(error, WorldError::SupervisionCycle { character: boss });
        assert_eq!(
            state
                .world
                .get_character(boss)
                .expect("boss should still exist")
                .supervisor(),
            None
        );
        assert_eq!(state.world.direct_reports(lieutenant).count(), 1);
        validate_invariants(&state);
    }

    #[test]
    fn reassignment_updates_hierarchy_indexes_atomically() {
        let registry = build_registry();
        let mut state = AppState::new(11);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Test Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("test organization should validate");
        let boss = make_test_character(&registry, &mut state, "Boss", organization, None);
        let lieutenant = make_test_character(
            &registry,
            &mut state,
            "Lieutenant",
            organization,
            Some(boss),
        );
        let soldier = make_test_character(
            &registry,
            &mut state,
            "Soldier",
            organization,
            Some(lieutenant),
        );

        validate_reassign_character(&state, soldier, Some(organization), Some(boss))
            .expect("valid reassignment should produce a token")
            .commit(&mut state)
            .expect("validated reassignment should remain current");

        assert_eq!(state.world.direct_reports(lieutenant).count(), 0);
        assert_eq!(state.world.direct_reports(boss).count(), 2);
        validate_invariants(&state);
    }

    #[test]
    fn unassigned_character_cannot_have_organization_supervisor() {
        let registry = build_registry();
        let mut state = AppState::new(13);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Test Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("test organization should validate");
        let supervisor =
            make_test_character(&registry, &mut state, "Supervisor", organization, None);

        let error = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Unassigned".to_owned(),
                organization: None,
                supervisor: Some(supervisor),
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect_err("unassigned character must not enter an organization hierarchy");

        assert_eq!(
            error,
            WorldError::SupervisorWithoutOrganization {
                character: supervisor,
            }
        );
        validate_invariants(&state);
    }

    #[test]
    fn unassigned_character_cannot_have_unassigned_supervisor() {
        let registry = build_registry();
        let mut state = AppState::new(13);
        let supervisor = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Unassigned Supervisor".to_owned(),
                organization: None,
                supervisor: None,
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect("unassigned supervisor fixture should validate");

        let error = insert_character(
            &registry,
            &mut state,
            CharacterDraft {
                name: "Unassigned".to_owned(),
                organization: None,
                supervisor: Some(supervisor),
                autonomy: AutonomyLevel::Guided,
                capabilities: BTreeMap::new(),
                traits: BTreeSet::new(),
                drives: BTreeMap::new(),
            },
        )
        .expect_err("unassigned character must not enter an organization hierarchy");

        assert_eq!(
            error,
            WorldError::SupervisorWithoutOrganization {
                character: supervisor,
            }
        );
        assert_eq!(state.world.direct_reports(supervisor).count(), 0);
        validate_invariants(&state);
    }

    #[test]
    fn stale_reassignment_token_cannot_overwrite_newer_membership() {
        let registry = build_registry();
        let mut state = AppState::new(17);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Test Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("test organization should validate");
        let boss = make_test_character(&registry, &mut state, "Boss", organization, None);
        let first = make_test_character(&registry, &mut state, "First", organization, Some(boss));
        let second = make_test_character(&registry, &mut state, "Second", organization, Some(boss));
        let member =
            make_test_character(&registry, &mut state, "Member", organization, Some(first));

        let stale = validate_reassign_character(&state, member, Some(organization), Some(second))
            .expect("first reassignment should validate");
        let current = validate_reassign_character(&state, member, Some(organization), Some(boss))
            .expect("second reassignment should validate against the same snapshot");
        current
            .commit(&mut state)
            .expect("current reassignment should commit");

        let error = stale
            .commit(&mut state)
            .expect_err("stale reassignment must not overwrite newer membership");
        assert_eq!(
            error,
            WorldError::StaleCharacter {
                character: member,
                expected: 1,
                found: 2,
            }
        );
        assert_eq!(
            state
                .world()
                .get_character(member)
                .expect("member should exist")
                .supervisor(),
            Some(boss)
        );
        validate_invariants(&state);
    }

    #[test]
    fn reassignment_token_revalidates_new_mandate_dependency_at_commit() {
        let registry = build_registry();
        let mut state = AppState::new(23);
        let first_organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "First Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("first organization should validate");
        let second_organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Second Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("second organization should validate");
        let manager =
            make_test_character(&registry, &mut state, "Manager", first_organization, None);
        let reassignment =
            validate_reassign_character(&state, manager, Some(second_organization), None)
                .expect("reassignment should initially validate");
        let mandate = validate_assign_mandate(
            &registry,
            &state,
            MandateDraft {
                organization: first_organization,
                manager,
                scopes: BTreeSet::from([ResponsibilityScope::Function(
                    ResponsibilityFunction::Personnel,
                )]),
                standing_orders: BTreeMap::new(),
                budget: None,
            },
        )
        .expect("mandate should validate after reassignment token creation")
        .commit(&mut state)
        .expect("mandate should commit");

        let error = reassignment
            .commit(&mut state)
            .expect_err("new active mandate must invalidate the older reassignment token");
        assert_eq!(
            error,
            WorldError::ActiveMandateAssignment {
                character: manager,
                mandate,
            }
        );
        assert_eq!(
            state
                .world()
                .get_character(manager)
                .expect("manager should exist")
                .organization(),
            Some(first_organization)
        );
        validate_invariants(&state);
    }

    #[test]
    fn reassignment_token_revalidates_supervisor_membership_at_commit() {
        let registry = build_registry();
        let mut state = AppState::new(29);
        let first_organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "First Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("first organization should validate");
        let second_organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Second Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("second organization should validate");
        let supervisor = make_test_character(
            &registry,
            &mut state,
            "Future Supervisor",
            first_organization,
            None,
        );
        let member = make_test_character(&registry, &mut state, "Member", first_organization, None);
        let member_reassignment =
            validate_reassign_character(&state, member, Some(first_organization), Some(supervisor))
                .expect("member reassignment should initially validate");
        validate_reassign_character(&state, supervisor, Some(second_organization), None)
            .expect("supervisor should be movable before gaining direct reports")
            .commit(&mut state)
            .expect("supervisor move should commit");

        let error = member_reassignment
            .commit(&mut state)
            .expect_err("supervisor organization change must invalidate member token");
        assert_eq!(
            error,
            WorldError::SupervisorOrganizationMismatch {
                supervisor,
                organization: Some(first_organization),
            }
        );
        assert_eq!(
            state
                .world()
                .get_character(member)
                .expect("member should exist")
                .supervisor(),
            None
        );
        validate_invariants(&state);
    }

    #[test]
    fn supervisor_cannot_leave_organization_with_direct_reports() {
        let registry = build_registry();
        let mut state = AppState::new(31);
        let first_organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "First Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("first organization should validate");
        let second_organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Second Organization".to_owned(),
                kind: OrganizationKind::Criminal,
            },
        )
        .expect("second organization should validate");
        let supervisor = make_test_character(
            &registry,
            &mut state,
            "Supervisor",
            first_organization,
            None,
        );
        let direct_report = make_test_character(
            &registry,
            &mut state,
            "Direct Report",
            first_organization,
            Some(supervisor),
        );

        let error =
            validate_reassign_character(&state, supervisor, Some(second_organization), None)
                .expect_err("supervisor must reassign direct reports before leaving organization");
        assert_eq!(
            error,
            WorldError::DirectReportAssignment {
                character: supervisor,
                direct_report,
            }
        );
        validate_invariants(&state);
    }
}
