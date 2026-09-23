//! Canonical business-economy status transitions.
//!
//! Suspension and resumption are isolated from cycle settlement so lifecycle validation and
//! freshness checks remain a focused owner shared by direct commands and acquisition composition.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BusinessEconomyStatusChange {
    Suspend,
    Resume,
    Restart,
}

pub struct ValidatedBusinessEconomyStatusChange {
    business: BusinessId,
    expected_version: u32,
    change: BusinessEconomyStatusChange,
    cycle_duration: Option<SimDuration>,
    restart_capital_floor: Option<OperatingCapitalFloor>,
}

impl ValidatedBusinessEconomyStatusChange {
    pub fn commit(self, state: &mut AppState) -> Result<(), BusinessEconomyError> {
        let business = state
            .world
            .get_business(self.business)
            .ok_or(BusinessEconomyError::MissingBusiness(self.business))?;
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
        if self.change == BusinessEconomyStatusChange::Suspend
            && let Some(enterprise) = super::active_enterprise_dependency(state, self.business)
        {
            return Err(BusinessEconomyError::ActiveEnterpriseDependency {
                business: self.business,
                enterprise,
            });
        }
        ensure_version_can_advance(economy.version(), "business economy")?;
        if matches!(
            self.change,
            BusinessEconomyStatusChange::Resume | BusinessEconomyStatusChange::Restart
        ) {
            validate_business(state, self.business)?;
            validate_accounts(
                state,
                self.business,
                economy.operating_account(),
                economy.settlement_account(),
                Some(self.business),
            )?;
            validate_account_posting_headroom(state, economy.operating_account())?;
            validate_account_posting_headroom(state, economy.settlement_account())?;
        }
        if let Some(capital_floor) = self.restart_capital_floor {
            if business.version() != capital_floor.business_version {
                return Err(BusinessEconomyError::StaleBusiness {
                    business: self.business,
                    expected: capital_floor.business_version,
                    found: business.version(),
                });
            }
            let operating = state
                .finance
                .get_account(economy.operating_account())
                .expect("restart account validation proved the operating account exists");
            if operating.version() != capital_floor.account_version {
                return Err(BusinessEconomyError::StaleOperatingAccount {
                    account: economy.operating_account(),
                    expected: capital_floor.account_version,
                    found: operating.version(),
                });
            }
        }
        let status = match self.change {
            BusinessEconomyStatusChange::Suspend => BusinessOperatingStatus::Suspended,
            BusinessEconomyStatusChange::Resume | BusinessEconomyStatusChange::Restart => {
                BusinessOperatingStatus::Active
            }
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
        // Resuming or restarting under a new owner begins a fresh chronic-loss grace window at
        // the actual lifecycle instant. The new owner does not inherit the seller's loss streak.
        let loss_streak_anchor = matches!(
            self.change,
            BusinessEconomyStatusChange::Resume | BusinessEconomyStatusChange::Restart
        )
        .then_some(state.now());
        let reset_laundering_window = match self.change {
            BusinessEconomyStatusChange::Suspend | BusinessEconomyStatusChange::Resume => false,
            BusinessEconomyStatusChange::Restart => true,
        };
        state.economy.set_status(
            self.business,
            status,
            next_cycle_at,
            loss_streak_anchor,
            reset_laundering_window,
            self.restart_capital_floor,
        );
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
    if let Some(enterprise) = super::active_enterprise_dependency(state, business) {
        return Err(BusinessEconomyError::ActiveEnterpriseDependency {
            business,
            enterprise,
        });
    }
    ensure_version_can_advance(economy.version(), "business economy")?;
    Ok(ValidatedBusinessEconomyStatusChange {
        business,
        expected_version: economy.version(),
        change: BusinessEconomyStatusChange::Suspend,
        cycle_duration: None,
        restart_capital_floor: None,
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

/// Acquisition composition hook: every existing economy restarts its cycle when title changes.
/// This prevents a buyer from acquiring just before the seller's scheduled settlement and
/// receiving an entire pre-purchase cycle. Suspended books also become active through the same
/// canonical lifecycle owner. The token commits only after acquisition's controlled mutations.
pub(crate) fn validate_acquisition_restart(
    state: &AppState,
    business: BusinessId,
    cycle_duration: SimDuration,
) -> Result<ValidatedBusinessEconomyStatusChange, BusinessEconomyError> {
    let business_record = validate_business(state, business)?;
    let economy = state
        .economy
        .get_business_economy(business)
        .ok_or(BusinessEconomyError::MissingBusinessEconomy(business))?;
    validate_accounts(
        state,
        business,
        economy.operating_account(),
        economy.settlement_account(),
        Some(business),
    )?;
    validate_account_posting_headroom(state, economy.operating_account())?;
    validate_account_posting_headroom(state, economy.settlement_account())?;
    ensure_version_can_advance(economy.version(), "business economy")?;
    ensure_version_can_advance(business_record.version(), "business")?;
    let operating = state
        .finance
        .get_account(economy.operating_account())
        .expect("account validation proved the operating account exists");
    let cycle_duration = schedulable_cycle_duration(economy.version(), cycle_duration);
    if let Some(cycle_duration) = cycle_duration {
        state
            .now()
            .checked_add(cycle_duration)
            .ok_or(BusinessEconomyError::SimulationTimeOverflow)?;
    }
    Ok(ValidatedBusinessEconomyStatusChange {
        business,
        expected_version: economy.version(),
        change: BusinessEconomyStatusChange::Restart,
        cycle_duration,
        restart_capital_floor: Some(OperatingCapitalFloor {
            amount: operating.spendable_balance(),
            account_version: operating.version(),
            business_version: business_record
                .version()
                .checked_add(1)
                .expect("business version capacity was preflighted"),
            set_at: state.now(),
        }),
    })
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
    validate_account_posting_headroom(state, economy.operating_account())?;
    validate_account_posting_headroom(state, economy.settlement_account())?;
    ensure_version_can_advance(economy.version(), "business economy")?;
    let cycle_duration = schedulable_cycle_duration(economy.version(), cycle_duration);
    if let Some(cycle_duration) = cycle_duration {
        state
            .now()
            .checked_add(cycle_duration)
            .ok_or(BusinessEconomyError::SimulationTimeOverflow)?;
    }
    Ok(ValidatedBusinessEconomyStatusChange {
        business,
        expected_version: economy.version(),
        change: BusinessEconomyStatusChange::Resume,
        cycle_duration,
        restart_capital_floor: None,
    })
}

/// Returning to Active consumes one economy version immediately. If that transition consumes the
/// final representable version, the economy remains operational as terminal state but must not
/// schedule another cycle that can never advance its version.
fn schedulable_cycle_duration(
    current_version: u32,
    cycle_duration: SimDuration,
) -> Option<SimDuration> {
    (current_version < u32::MAX - 1).then_some(cycle_duration)
}
