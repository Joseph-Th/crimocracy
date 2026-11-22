//! Transfer a live racket's authority without replacing its financial or legal identity.

use super::*;

#[derive(Debug)]
pub struct ValidatedEnterpriseReassignment {
    enterprise: EnterpriseId,
    expected_version: u32,
    at: SimTime,
    authority: ResolvedMandateAuthority,
}

pub fn validate_reassign_enterprise(
    state: &AppState,
    enterprise: EnterpriseId,
    authority: MandateAuthority,
) -> Result<ValidatedEnterpriseReassignment, EnterpriseError> {
    let record = state
        .enterprises
        .get_enterprise(enterprise)
        .ok_or(EnterpriseError::MissingEnterprise(enterprise))?;
    if record.status() == EnterpriseStatus::Retired {
        return Err(EnterpriseError::EnterpriseRetired(enterprise));
    }
    if record.authority() == authority {
        return Err(EnterpriseError::EnterpriseAuthorityUnchanged(enterprise));
    }
    ensure_version_can_advance(record.version(), "enterprise")?;
    if record.status() == EnterpriseStatus::Active {
        // Reassignment may not exhaust the version capacity of a scheduled live racket.
        ensure_version_can_advance(record.version() + 1, "enterprise")?;
        if record.next_cycle_at().is_some_and(|due| due <= state.now()) {
            return Err(EnterpriseError::EnterpriseReassignmentCycleDue(enterprise));
        }
        ensure_manager_capacity(
            state,
            record.organization(),
            authority.manager,
            ManagedJob {
                kind: record.kind(),
                location: record.location(),
                associates: record.associates(),
                supporting_businesses: record.supporting_businesses().len(),
            },
            Some(enterprise),
        )?;
    }
    validate_enterprise_environment(
        state,
        record.organization(),
        authority,
        record.location(),
        record.supporting_businesses(),
    )?;
    Ok(ValidatedEnterpriseReassignment {
        enterprise,
        expected_version: record.version(),
        at: state.now(),
        authority: resolve_mandate_authority(state, authority)?,
    })
}

impl ValidatedEnterpriseReassignment {
    pub fn commit(self, state: &mut AppState) -> Result<(), EnterpriseError> {
        if state.now() != self.at {
            return Err(EnterpriseError::StaleCycleTime {
                expected: self.at,
                found: state.now(),
            });
        }
        let record = state
            .enterprises
            .get_enterprise(self.enterprise)
            .ok_or(EnterpriseError::MissingEnterprise(self.enterprise))?;
        if record.version() != self.expected_version {
            return Err(EnterpriseError::StaleEnterprise {
                enterprise: self.enterprise,
                expected: self.expected_version,
                found: record.version(),
            });
        }
        ensure_mandate_authority_current(state, self.authority)?;
        validate_reassign_enterprise(state, self.enterprise, self.authority.authority())?;
        state
            .enterprises
            .reassign_authority(self.enterprise, self.authority.authority(), self.at);
        Ok(())
    }
}
