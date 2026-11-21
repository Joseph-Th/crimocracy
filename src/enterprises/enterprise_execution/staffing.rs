//! Canonical rank-and-file enterprise staffing adjustments.

use super::*;

/// A named captain can direct only so many independently operated rackets and
/// rank-and-file associates at once. This is an organizational constraint,
/// not a secret NPC modifier. It applies equally to autonomous and player moves.
pub const MAX_RACKETS_PER_MANAGER: u16 = 6;
pub const MAX_ASSOCIATES_PER_MANAGER: u16 = 24;

pub(crate) fn ensure_manager_capacity(
    state: &AppState,
    organization: OrganizationId,
    manager: crate::core::id::CharacterId,
    additional_associates: u16,
    excluding: Option<EnterpriseId>,
) -> Result<(), EnterpriseError> {
    let mut rackets = 1_u16;
    let mut associates = u32::from(additional_associates);
    for active in state
        .enterprises()
        .active_for_organization(organization)
        .filter(|record| record.manager() == manager && Some(record.id()) != excluding)
    {
        rackets = rackets.saturating_add(1);
        associates += u32::from(active.associates());
    }
    if rackets > MAX_RACKETS_PER_MANAGER || associates > u32::from(MAX_ASSOCIATES_PER_MANAGER) {
        return Err(EnterpriseError::ManagerCapacityExceeded {
            manager,
            rackets,
            associates,
            max_rackets: MAX_RACKETS_PER_MANAGER,
            max_associates: MAX_ASSOCIATES_PER_MANAGER,
        });
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub struct ValidatedEnterpriseStaffing {
    enterprise: EnterpriseId,
    expected_version: u32,
    at: SimTime,
    associates: u16,
}

pub fn validate_staff_enterprise(
    state: &AppState,
    enterprise: EnterpriseId,
    associates: u16,
) -> Result<ValidatedEnterpriseStaffing, EnterpriseError> {
    let record = state
        .enterprises()
        .get_enterprise(enterprise)
        .ok_or(EnterpriseError::MissingEnterprise(enterprise))?;
    if record.status() == EnterpriseStatus::Retired {
        return Err(EnterpriseError::EnterpriseRetired(enterprise));
    }
    let minimum = record.kind().baseline_associates();
    let maximum = record.kind().max_associates();
    if !(minimum..=maximum).contains(&associates) {
        return Err(EnterpriseError::InvalidAssociateCount {
            enterprise,
            minimum,
            maximum,
        });
    }
    if record.associates() == associates {
        return Err(EnterpriseError::EnterpriseStaffingUnchanged(enterprise));
    }
    ensure_version_can_advance(record.version(), "enterprise")?;
    if record.status() == EnterpriseStatus::Active {
        ensure_version_can_advance(record.version() + 1, "enterprise")?;
        if record.next_cycle_at().is_some_and(|due| due <= state.now()) {
            return Err(EnterpriseError::EnterpriseStaffingCycleDue(enterprise));
        }
        ensure_manager_capacity(
            state,
            record.organization(),
            record.manager(),
            associates,
            Some(enterprise),
        )?;
    }
    Ok(ValidatedEnterpriseStaffing {
        enterprise,
        expected_version: record.version(),
        at: state.now(),
        associates,
    })
}

impl ValidatedEnterpriseStaffing {
    pub fn commit(self, state: &mut AppState) -> Result<(), EnterpriseError> {
        if state.now() != self.at {
            return Err(EnterpriseError::StaleCycleTime {
                expected: self.at,
                found: state.now(),
            });
        }
        let record = state
            .enterprises()
            .get_enterprise(self.enterprise)
            .ok_or(EnterpriseError::MissingEnterprise(self.enterprise))?;
        if record.version() != self.expected_version {
            return Err(EnterpriseError::StaleEnterprise {
                enterprise: self.enterprise,
                expected: self.expected_version,
                found: record.version(),
            });
        }
        validate_staff_enterprise(state, self.enterprise, self.associates)?;
        state
            .enterprises
            .set_associates(self.enterprise, self.associates, self.at);
        Ok(())
    }
}
