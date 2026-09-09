//! Canonical business-economy status transitions.
//!
//! Suspension and resumption are isolated from cycle settlement so lifecycle validation and
//! freshness checks remain a focused owner shared by direct commands and acquisition composition.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BusinessEconomyStatusChange {
    Suspend,
    Resume,
}

pub struct ValidatedBusinessEconomyStatusChange {
    business: BusinessId,
    expected_version: u32,
    change: BusinessEconomyStatusChange,
    cycle_duration: Option<SimDuration>,
}

impl ValidatedBusinessEconomyStatusChange {
    pub fn commit(self, state: &mut AppState) -> Result<(), BusinessEconomyError> {
        if state.world.get_business(self.business).is_none() {
            return Err(BusinessEconomyError::MissingBusiness(self.business));
        }
        let economy = state
            .economy
            .get_business_economy(self.business)
            .ok_or(BusinessEconomyError::MissingBusinessEconomy(self.business))?;
        if economy.version() != self.expected_version {
            return Err(BusinessEconomyError::StaleEconomy {
                business: self.business,
                expected: self.expected_version,
                found: economy.version(),
            });
        }
        ensure_version_can_advance(economy.version(), "business economy")?;
        if self.change == BusinessEconomyStatusChange::Resume {
            validate_business(state, self.business)?;
            validate_accounts(
                state,
                self.business,
                economy.operating_account(),
                economy.settlement_account(),
                Some(self.business),
            )?;
        }
        let status = match self.change {
            BusinessEconomyStatusChange::Suspend => BusinessOperatingStatus::Suspended,
            BusinessEconomyStatusChange::Resume => BusinessOperatingStatus::Active,
        };
        let next_cycle_at = self
            .cycle_duration
            .map(|duration| {
                state
                    .now()
                    .checked_add(duration)
                    .ok_or(BusinessEconomyError::SimulationTimeOverflow)
            })
            .transpose()?;
        // Resuming restarts the chronic-loss grace window at the actual resume instant.
        let loss_streak_anchor =
            (self.change == BusinessEconomyStatusChange::Resume).then_some(state.now());
        state
            .economy
            .set_status(self.business, status, next_cycle_at, loss_streak_anchor);
        Ok(())
    }
}

pub fn validate_suspend_business_economy(
    state: &AppState,
    business: BusinessId,
) -> Result<ValidatedBusinessEconomyStatusChange, BusinessEconomyError> {
    let economy = state
        .economy
        .get_business_economy(business)
        .ok_or(BusinessEconomyError::MissingBusinessEconomy(business))?;
    match economy.status() {
        BusinessOperatingStatus::Active => {}
        BusinessOperatingStatus::Suspended => {
            return Err(BusinessEconomyError::EconomyNotActive(business));
        }
    }
    ensure_version_can_advance(economy.version(), "business economy")?;
    Ok(ValidatedBusinessEconomyStatusChange {
        business,
        expected_version: economy.version(),
        change: BusinessEconomyStatusChange::Suspend,
        cycle_duration: None,
    })
}

pub fn validate_resume_business_economy(
    registry: &Registry,
    state: &AppState,
    business: BusinessId,
) -> Result<ValidatedBusinessEconomyStatusChange, BusinessEconomyError> {
    let business_record = validate_business(state, business)?;
    let cycle_duration = registry
        .get_business(business_record.kind())
        .economics()
        .cycle();
    validate_resume_with_cycle_duration(state, business, cycle_duration)
}

/// Acquisition composition hook: validates a suspended economy before the acquisition mutates
/// ownership or money. The returned canonical status token can then commit after those controlled
/// mutations without introducing a new validation path.
pub(crate) fn validate_acquisition_resume(
    state: &AppState,
    business: BusinessId,
    cycle_duration: SimDuration,
) -> Result<ValidatedBusinessEconomyStatusChange, BusinessEconomyError> {
    validate_resume_with_cycle_duration(state, business, cycle_duration)
}

fn validate_resume_with_cycle_duration(
    state: &AppState,
    business: BusinessId,
    cycle_duration: SimDuration,
) -> Result<ValidatedBusinessEconomyStatusChange, BusinessEconomyError> {
    let _business_record = validate_business(state, business)?;
    let economy = state
        .economy
        .get_business_economy(business)
        .ok_or(BusinessEconomyError::MissingBusinessEconomy(business))?;
    match economy.status() {
        BusinessOperatingStatus::Active => {
            return Err(BusinessEconomyError::EconomyNotSuspended(business));
        }
        BusinessOperatingStatus::Suspended => {}
    }
    validate_accounts(
        state,
        business,
        economy.operating_account(),
        economy.settlement_account(),
        Some(business),
    )?;
    ensure_version_can_advance(economy.version(), "business economy")?;
    state
        .now()
        .checked_add(cycle_duration)
        .ok_or(BusinessEconomyError::SimulationTimeOverflow)?;
    Ok(ValidatedBusinessEconomyStatusChange {
        business,
        expected_version: economy.version(),
        change: BusinessEconomyStatusChange::Resume,
        cycle_duration: Some(cycle_duration),
    })
}
