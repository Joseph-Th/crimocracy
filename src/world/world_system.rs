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
    CharacterMembership, CharacterRecord, CharacterRuntime, NeighborhoodDraft, NeighborhoodRecord,
    OrganizationDraft, OrganizationKind, OrganizationRecord, PolicySetting,
};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum WorldError {
    #[error("name must not be empty")]
    EmptyName,
    #[error("organization {0} does not exist")]
    MissingOrganization(OrganizationId),
    #[error("character {0} does not exist")]
    MissingCharacter(CharacterId),
    #[error("neighborhood {0} does not exist")]
    MissingNeighborhood(NeighborhoodId),
    #[error("business {0} does not exist")]
    MissingBusiness(BusinessId),
    #[error("business {business} is already owned by {owner:?}")]
    BusinessOwnershipUnchanged {
        business: BusinessId,
        owner: BusinessOwner,
    },
    #[error(
    "character {character} already has this organization and supervisor; reassignment unchanged"
  )]
    CharacterReassignmentUnchanged { character: CharacterId },
    #[error(
        "business {business} changed after validation; expected version {expected}, found {found}"
    )]
    StaleBusiness {
        business: BusinessId,
        expected: u32,
        found: u32,
    },
    #[error(
        "business {business} is owned by {actual:?}, not the validated previous owner {expected:?}"
    )]
    BusinessOwnerChanged {
        business: BusinessId,
        expected: BusinessOwner,
        actual: BusinessOwner,
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
    #[error("cannot assign supervisor {supervisor} to a character without an organization")]
    SupervisorWithoutOrganization { supervisor: CharacterId },
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
    });
    Ok(id)
}

pub fn insert_character(
    state: &mut AppState,
    draft: CharacterDraft,
) -> Result<CharacterId, WorldError> {
    if draft.name.trim().is_empty() {
        return Err(WorldError::EmptyName);
    }
    validate_membership(state, draft.organization, draft.supervisor)?;

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
        runtime: CharacterRuntime { version: 1 },
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
    if organization == record.organization() && supervisor == record.supervisor() {
        // A no-op reassignment must not commit or bump the version: that would silently
        // invalidate every outstanding validated token pinned to this character.
        return Err(WorldError::CharacterReassignmentUnchanged { character });
    }
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
    let _ = state
        .world
        .get_neighborhood(draft.neighborhood)
        .ok_or(WorldError::MissingNeighborhood(draft.neighborhood))?;
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
            return Err(WorldError::BusinessOwnerChanged {
                business: self.business,
                expected: self.previous_owner,
                actual: record.owner(),
            });
        }
        validate_business_owner(state, self.new_owner)?;
        validate_business_support_ownership_change(state, self.business, self.new_owner)?;
        debug_assert_ne!(
            self.new_owner, self.previous_owner,
            "validation rejects unchanged ownership before a token exists"
        );
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
    Ok(record)
}

fn validate_business_owner(state: &AppState, owner: BusinessOwner) -> Result<(), WorldError> {
    match owner {
        BusinessOwner::Independent => Ok(()),
        BusinessOwner::Organization(id) => {
            state
                .world
                .get_organization(id)
                .ok_or(WorldError::MissingOrganization(id))?;
            Ok(())
        }
        BusinessOwner::Character(id) => {
            state
                .world
                .get_character(id)
                .ok_or(WorldError::MissingCharacter(id))?;
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
    state
        .world
        .get_organization(organization)
        .ok_or(WorldError::MissingOrganization(organization))?;
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
        state
            .world
            .get_organization(organization_id)
            .ok_or(WorldError::MissingOrganization(organization_id))?;
    }
    // A supervision chain is an organization hierarchy; an unassigned character cannot report
    // to a supervisor because no organization would own the relationship.
    if let Some(supervisor_id) = supervisor {
        if organization.is_none() {
            return Err(WorldError::SupervisorWithoutOrganization {
                supervisor: supervisor_id,
            });
        }
        let supervisor_record = state
            .world
            .get_character(supervisor_id)
            .ok_or(WorldError::MissingCharacter(supervisor_id))?;
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
mod tests;
