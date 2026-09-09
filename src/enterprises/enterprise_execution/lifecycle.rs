//! Canonical enterprise status transitions.
//!
//! Lifecycle changes are separated from cycle settlement so suspension, resumption, and terminal
//! retirement retain one focused validate/commit owner without expanding the already substantial
//! settlement module.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EnterpriseStatusChange {
    Suspend,
    Resume,
    Retire,
}

pub struct ValidatedEnterpriseStatusChange {
    enterprise: EnterpriseId,
    expected_version: u32,
    change: EnterpriseStatusChange,
    cycle_duration: Option<SimDuration>,
    authority: Option<ResolvedMandateAuthority>,
    supporting_business_versions: BTreeMap<BusinessId, u32>,
    /// Venue version pinned at validation for a resumption at a business location. A token
    /// held across a venue sale or refit must stale exactly like the cycle path's host pin:
    /// ownership and required functions live on the business record, so its version guards both.
    host_business_version: Option<(BusinessId, u32)>,
}

impl ValidatedEnterpriseStatusChange {
    pub fn commit(self, state: &mut AppState) -> Result<(), EnterpriseError> {
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
        ensure_version_can_advance(record.version(), "enterprise")?;
        if let Some(authority) = self.authority {
            ensure_mandate_authority_current(state, authority)?;
            validate_enterprise_environment(
                state,
                record.organization(),
                record.authority(),
                record.location(),
                record.supporting_businesses(),
            )?;
            validate_supporting_business_versions(state, &self.supporting_business_versions)?;
            if let Some((business_id, expected)) = self.host_business_version {
                let business = state
                    .world
                    .get_business(business_id)
                    .ok_or(EnterpriseError::InvalidLocation(record.location()))?;
                if business.version() != expected {
                    return Err(EnterpriseError::StaleHostBusiness {
                        business: business_id,
                        expected,
                        found: business.version(),
                    });
                }
            }
            validate_supporting_businesses(
                state,
                record.organization(),
                record.location(),
                record.supporting_businesses(),
            )?;
            validate_enterprise_accounts(
                state,
                record.organization(),
                record.cash_account(),
                record.settlement_account(),
                Some(record.id()),
            )?;
        }
        let next_status = match self.change {
            EnterpriseStatusChange::Suspend => EnterpriseStatus::Suspended,
            EnterpriseStatusChange::Resume => EnterpriseStatus::Active,
            EnterpriseStatusChange::Retire => EnterpriseStatus::Retired,
        };
        let next_cycle_at = self
            .cycle_duration
            .map(|duration| {
                state
                    .now()
                    .checked_add(duration)
                    .ok_or(EnterpriseError::SimulationTimeOverflow)
            })
            .transpose()?;
        // Resuming restarts the chronic-loss grace window at the actual resume instant.
        let loss_streak_anchor =
            (self.change == EnterpriseStatusChange::Resume).then_some(state.now());
        state.enterprises.set_status(
            self.enterprise,
            next_status,
            next_cycle_at,
            loss_streak_anchor,
            state.now(),
        );
        Ok(())
    }
}

pub fn validate_suspend_enterprise(
    state: &AppState,
    enterprise: EnterpriseId,
) -> Result<ValidatedEnterpriseStatusChange, EnterpriseError> {
    let record = state
        .enterprises
        .get_enterprise(enterprise)
        .ok_or(EnterpriseError::MissingEnterprise(enterprise))?;
    if record.status() != EnterpriseStatus::Active {
        return Err(match record.status() {
            EnterpriseStatus::Active => unreachable!(),
            EnterpriseStatus::Suspended => EnterpriseError::EnterpriseNotActive(enterprise),
            EnterpriseStatus::Retired => EnterpriseError::EnterpriseRetired(enterprise),
        });
    }
    ensure_version_can_advance(record.version(), "enterprise")?;
    Ok(ValidatedEnterpriseStatusChange {
        enterprise,
        expected_version: record.version(),
        change: EnterpriseStatusChange::Suspend,
        cycle_duration: None,
        authority: None,
        supporting_business_versions: BTreeMap::new(),
        host_business_version: None,
    })
}

pub fn validate_resume_enterprise(
    registry: &Registry,
    state: &AppState,
    enterprise: EnterpriseId,
) -> Result<ValidatedEnterpriseStatusChange, EnterpriseError> {
    let record = state
        .enterprises
        .get_enterprise(enterprise)
        .ok_or(EnterpriseError::MissingEnterprise(enterprise))?;
    match record.status() {
        EnterpriseStatus::Active => {
            return Err(EnterpriseError::EnterpriseNotSuspended(enterprise));
        }
        EnterpriseStatus::Suspended => {}
        EnterpriseStatus::Retired => return Err(EnterpriseError::EnterpriseRetired(enterprise)),
    }
    ensure_version_can_advance(record.version(), "enterprise")?;
    let authority = resolve_mandate_authority(state, record.authority())?;
    validate_enterprise_environment(
        state,
        record.organization(),
        record.authority(),
        record.location(),
        record.supporting_businesses(),
    )?;
    let definition = registry.get_enterprise(record.kind());
    validate_enterprise_business_dependencies(
        definition,
        state,
        record.organization(),
        record.location(),
        record.supporting_businesses(),
    )?;
    validate_enterprise_accounts(
        state,
        record.organization(),
        record.cash_account(),
        record.settlement_account(),
        Some(record.id()),
    )?;
    let cycle_duration = definition.economics().cycle();
    state
        .now()
        .checked_add(cycle_duration)
        .ok_or(EnterpriseError::SimulationTimeOverflow)?;
    let supporting_business_versions =
        snapshot_supporting_business_versions(state, record.supporting_businesses())?;
    let host_business_version = match record.location() {
        EnterpriseLocation::Business(business_id) => {
            let business = state
                .world
                .get_business(business_id)
                .ok_or(EnterpriseError::InvalidLocation(record.location()))?;
            Some((business_id, business.version()))
        }
        EnterpriseLocation::Neighborhood(_) => None,
    };
    Ok(ValidatedEnterpriseStatusChange {
        enterprise,
        expected_version: record.version(),
        change: EnterpriseStatusChange::Resume,
        cycle_duration: Some(cycle_duration),
        authority: Some(authority),
        supporting_business_versions,
        host_business_version,
    })
}

/// Permanently abandons a suspended racket while preserving its historical record and cycles.
/// Retirement is intentionally a separate step from suspension: callers must first release the
/// active schedule/mandate dependency, then explicitly decide that the old operation will never
/// be resumed. A retired record no longer reserves its kind/location slot.
pub fn validate_retire_enterprise(
    state: &AppState,
    enterprise: EnterpriseId,
) -> Result<ValidatedEnterpriseStatusChange, EnterpriseError> {
    let record = state
        .enterprises
        .get_enterprise(enterprise)
        .ok_or(EnterpriseError::MissingEnterprise(enterprise))?;
    match record.status() {
        EnterpriseStatus::Active => {
            return Err(EnterpriseError::EnterpriseNotSuspended(enterprise));
        }
        EnterpriseStatus::Suspended => {}
        EnterpriseStatus::Retired => return Err(EnterpriseError::EnterpriseRetired(enterprise)),
    }
    ensure_version_can_advance(record.version(), "enterprise")?;
    Ok(ValidatedEnterpriseStatusChange {
        enterprise,
        expected_version: record.version(),
        change: EnterpriseStatusChange::Retire,
        cycle_duration: None,
        authority: None,
        supporting_business_versions: BTreeMap::new(),
        host_business_version: None,
    })
}
