//! Business economy establishment, deterministic cycle planning, autonomous non-player recovery,
//! and atomic ledger settlement.

mod autonomous_lifecycle;
mod cycle_planning;

#[cfg(test)]
use cycle_planning::resolve_gross_before_variance;
pub use cycle_planning::{
    BusinessCyclePlan, ValidatedBusinessCycle, decide_business_cycle, validate_business_cycle_plan,
};
use cycle_planning::{active_enterprise_dependency, resolve_cycle_financials};
pub(crate) use cycle_planning::{
    find_due_businesses, resolve_business_current_gross, resolve_business_gross_potential,
    resolve_historical_business_cycle_financials,
};
mod lifecycle;
mod profit_sweep;

pub(crate) use autonomous_lifecycle::apply_due_autonomous_business_lifecycle;
pub(crate) use lifecycle::validate_acquisition_restart;
pub use lifecycle::{
    ValidatedBusinessEconomyStatusChange, validate_resume_business_economy,
    validate_suspend_business_economy,
};
pub use profit_sweep::{
    BusinessProfitSweepDraft, BusinessProfitSweepError, ValidatedBusinessProfitSweep,
    validate_sweep_business_profits,
};

#[cfg(test)]
use crate::core::attention::AttentionClass;
#[cfg(test)]
use crate::core::entity::EntityRef;
#[cfg(test)]
use crate::core::id::BusinessCycleId;
use crate::core::id::{BusinessId, EnterpriseId, FinancialAccountId, IdExhaustionError};
use crate::core::state::AppState;
use crate::core::time::{SimDuration, SimTime};
use crate::core::version::{VersionCapacityError, ensure_version_can_advance};
use crate::economy::{
    BusinessEconomyDraft, BusinessOperatingStatus, OperatingCapitalFloor,
    build_business_economy_record,
};
use crate::enterprises::enterprise_execution::EnterpriseError;
use crate::finance::finance_system::{FinanceError, ValidatedFinancialAccountOpenings};
use crate::finance::{AccountKind, FinancialOwner, Money};
use crate::intelligence::intelligence_system::IntelligenceError;
#[cfg(test)]
use crate::intelligence::{InformationSourceKind, KnowledgeHolder};
use crate::registry::Registry;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum BusinessEconomyError {
    #[error("business {0} does not exist")]
    MissingBusiness(BusinessId),
    #[error("business {0} has no operating economy record")]
    MissingBusinessEconomy(BusinessId),
    #[error("business {0} references a missing neighborhood")]
    MissingBusinessNeighborhood(BusinessId),
    #[error("business {0} already has an operating economy record")]
    ExistingBusinessEconomy(BusinessId),
    #[error("financial account {0} does not exist")]
    MissingAccount(FinancialAccountId),
    #[error("business economy account {account} is not owned by business {business}")]
    AccountOwnerMismatch {
        business: BusinessId,
        account: FinancialAccountId,
    },
    #[error("business operating account {0} must be legitimate operating funds")]
    InvalidOperatingAccountKind(FinancialAccountId),
    #[error("business settlement account {0} must be a settlement account")]
    InvalidSettlementAccountKind(FinancialAccountId),
    #[error("settlement account {account} is already assigned to business {business}")]
    SettlementAccountInUse {
        account: FinancialAccountId,
        business: BusinessId,
    },
    #[error("business {0} operating economy is not active")]
    EconomyNotActive(BusinessId),
    #[error("business {0} operating economy is not suspended")]
    EconomyNotSuspended(BusinessId),
    #[error("business {business} is required by active enterprise {enterprise}")]
    ActiveEnterpriseDependency {
        business: BusinessId,
        enterprise: EnterpriseId,
    },
    #[error(
        "business {business} enterprise dependency changed after cycle planning; expected {expected:?}, found {found:?}"
    )]
    StaleEnterpriseDependency {
        business: BusinessId,
        expected: std::collections::BTreeSet<EnterpriseId>,
        found: std::collections::BTreeSet<EnterpriseId>,
    },
    #[error("business {business} is not due for a cycle until {due_at:?}")]
    CycleNotDue {
        business: BusinessId,
        due_at: SimTime,
    },
    #[error("business cycle variance {basis_points} basis points exceeds authored limit {limit}")]
    VarianceOutOfRange { basis_points: i16, limit: u16 },
    #[error("business economics overflowed while resolving business {0}")]
    ArithmeticOverflow(BusinessId),
    #[error("business economy scheduling exceeds the representable simulation clock")]
    SimulationTimeOverflow,
    #[error(
        "business {business} economy changed after validation; expected version {expected}, found {found}"
    )]
    StaleEconomy {
        business: BusinessId,
        expected: u32,
        found: u32,
    },
    #[error(
        "business {business} ownership changed after cycle planning; expected version {expected}, found {found}"
    )]
    StaleBusiness {
        business: BusinessId,
        expected: u32,
        found: u32,
    },
    #[error(
        "business operating account {account} changed after capital validation; expected version {expected}, found {found}"
    )]
    StaleOperatingAccount {
        account: FinancialAccountId,
        expected: u32,
        found: u32,
    },
    #[error(
        "business cycle plan was resolved at {expected:?}, but simulation time is now {found:?}"
    )]
    StaleCycleTime { expected: SimTime, found: SimTime },
    #[error(
        "business disruption for business {business} was validated at {expected:?}, but simulation time is now {found:?}"
    )]
    StaleDisruptionTime {
        business: BusinessId,
        expected: SimTime,
        found: SimTime,
    },
    #[error(transparent)]
    Finance(#[from] FinanceError),
    #[error(transparent)]
    Intelligence(#[from] IntelligenceError),
    #[error(transparent)]
    Enterprise(#[from] EnterpriseError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
    #[error(transparent)]
    VersionCapacity(#[from] VersionCapacityError),
}

pub struct ValidatedBusinessEconomyEstablishment {
    draft: BusinessEconomyDraft,
    cycle_duration: SimDuration,
    capital_floor: OperatingCapitalSnapshot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OperatingCapitalSnapshot {
    amount: Money,
    account_version: u32,
    business_version: u32,
}

impl ValidatedBusinessEconomyEstablishment {
    pub fn commit(self, state: &mut AppState) -> Result<BusinessId, BusinessEconomyError> {
        let business_record = validate_business(state, self.draft.business)?;
        if business_record.version() != self.capital_floor.business_version {
            return Err(BusinessEconomyError::StaleBusiness {
                business: self.draft.business,
                expected: self.capital_floor.business_version,
                found: business_record.version(),
            });
        }
        if state
            .economy
            .get_business_economy(self.draft.business)
            .is_some()
        {
            return Err(BusinessEconomyError::ExistingBusinessEconomy(
                self.draft.business,
            ));
        }
        validate_accounts(
            state,
            self.draft.business,
            self.draft.operating_account,
            self.draft.settlement_account,
            None,
        )?;
        let operating = state
            .finance
            .get_account(self.draft.operating_account)
            .expect("validated operating account must still exist");
        if operating.version() != self.capital_floor.account_version {
            return Err(BusinessEconomyError::StaleOperatingAccount {
                account: self.draft.operating_account,
                expected: self.capital_floor.account_version,
                found: operating.version(),
            });
        }
        let business = self.draft.business;
        let established_at = state.now();
        let next_cycle_at = established_at
            .checked_add(self.cycle_duration)
            .ok_or(BusinessEconomyError::SimulationTimeOverflow)?;
        state.economy.insert(build_business_economy_record(
            self.draft,
            established_at,
            next_cycle_at,
            OperatingCapitalFloor {
                amount: self.capital_floor.amount,
                account_version: self.capital_floor.account_version,
                business_version: self.capital_floor.business_version,
                set_at: established_at,
            },
        ));
        Ok(business)
    }
}

pub fn validate_establish_business_economy(
    registry: &Registry,
    state: &AppState,
    draft: BusinessEconomyDraft,
) -> Result<ValidatedBusinessEconomyEstablishment, BusinessEconomyError> {
    let business = validate_business(state, draft.business)?;
    if state.economy.get_business_economy(draft.business).is_some() {
        return Err(BusinessEconomyError::ExistingBusinessEconomy(
            draft.business,
        ));
    }
    validate_accounts(
        state,
        draft.business,
        draft.operating_account,
        draft.settlement_account,
        None,
    )?;
    let cycle_duration = registry.get_business(business.kind()).economics().cycle();
    state
        .now()
        .checked_add(cycle_duration)
        .ok_or(BusinessEconomyError::SimulationTimeOverflow)?;
    let capital_floor =
        resolve_operating_capital_snapshot(state, business, draft.operating_account)?;
    Ok(ValidatedBusinessEconomyEstablishment {
        draft,
        cycle_duration,
        capital_floor,
    })
}

/// Business-economy establishment validated against accounts that a composing operation has
/// planned but not yet opened. The token deliberately does not own the opening plan because the
/// same plan is consumed by the ledger transaction that capitalizes the new operating account.
pub(crate) struct ValidatedComposedBusinessEconomyEstablishment {
    draft: BusinessEconomyDraft,
    cycle_duration: SimDuration,
    resulting_business_version: u32,
}

impl ValidatedComposedBusinessEconomyEstablishment {
    /// Commit after the composing operation has opened the previously validated accounts. No
    /// fallible work remains here; the acquisition path owns dependency revalidation and ID
    /// preflight before its first mutation.
    pub(crate) fn commit_after_preflight(self, state: &mut AppState) -> BusinessId {
        debug_assert!(
            state.world.get_business(self.draft.business).is_some(),
            "prevalidated composed economy must retain its business"
        );
        debug_assert!(
            state
                .economy
                .get_business_economy(self.draft.business)
                .is_none(),
            "prevalidated composed economy must remain unique"
        );
        debug_assert!(
            validate_accounts(
                state,
                self.draft.business,
                self.draft.operating_account,
                self.draft.settlement_account,
                None,
            )
            .is_ok(),
            "prevalidated composed economy accounts must match the opened plan"
        );
        let business_version = state
            .world
            .get_business(self.draft.business)
            .expect("prevalidated composed economy must retain its business")
            .version();
        debug_assert_eq!(business_version, self.resulting_business_version);
        let operating_version = state
            .finance
            .get_account(self.draft.operating_account)
            .expect("prevalidated composed operating account must be open")
            .version();
        debug_assert_eq!(operating_version, 1);
        let business = self.draft.business;
        let established_at = state.now();
        let next_cycle_at = established_at
            .checked_add(self.cycle_duration)
            .expect("composed business-economy schedule was preflighted before mutation");
        state.economy.insert(build_business_economy_record(
            self.draft,
            established_at,
            next_cycle_at,
            OperatingCapitalFloor {
                amount: Money::ZERO,
                account_version: 1,
                business_version: self.resulting_business_version,
                set_at: established_at,
            },
        ));
        business
    }
}

pub(crate) fn validate_composed_business_economy_establishment(
    state: &AppState,
    draft: BusinessEconomyDraft,
    cycle_duration: SimDuration,
    openings: &ValidatedFinancialAccountOpenings,
) -> Result<ValidatedComposedBusinessEconomyEstablishment, BusinessEconomyError> {
    let business = validate_business(state, draft.business)?;
    if state.economy.get_business_economy(draft.business).is_some() {
        return Err(BusinessEconomyError::ExistingBusinessEconomy(
            draft.business,
        ));
    }
    openings.ensure_current(state)?;
    if !openings.account_matches(
        draft.operating_account,
        FinancialOwner::Business(draft.business),
        AccountKind::LegitimateOperating,
    ) {
        return Err(BusinessEconomyError::InvalidOperatingAccountKind(
            draft.operating_account,
        ));
    }
    if !openings.account_matches(
        draft.settlement_account,
        FinancialOwner::Business(draft.business),
        AccountKind::Settlement,
    ) {
        return Err(BusinessEconomyError::InvalidSettlementAccountKind(
            draft.settlement_account,
        ));
    }
    state
        .now()
        .checked_add(cycle_duration)
        .ok_or(BusinessEconomyError::SimulationTimeOverflow)?;
    ensure_version_can_advance(business.version(), "business")?;
    Ok(ValidatedComposedBusinessEconomyEstablishment {
        draft,
        cycle_duration,
        resulting_business_version: business
            .version()
            .checked_add(1)
            .expect("business version capacity was preflighted"),
    })
}

fn resolve_operating_capital_snapshot(
    state: &AppState,
    business: &crate::world::BusinessRecord,
    operating_account: FinancialAccountId,
) -> Result<OperatingCapitalSnapshot, BusinessEconomyError> {
    let operating = state
        .finance
        .get_account(operating_account)
        .ok_or(BusinessEconomyError::MissingAccount(operating_account))?;
    Ok(OperatingCapitalSnapshot {
        amount: operating.spendable_balance(),
        account_version: operating.version(),
        business_version: business.version(),
    })
}

fn validate_business(
    state: &AppState,
    business: BusinessId,
) -> Result<&crate::world::BusinessRecord, BusinessEconomyError> {
    let business_record = state
        .world
        .get_business(business)
        .ok_or(BusinessEconomyError::MissingBusiness(business))?;
    Ok(business_record)
}

fn validate_accounts(
    state: &AppState,
    business: BusinessId,
    operating_account: FinancialAccountId,
    settlement_account: FinancialAccountId,
    current_business: Option<BusinessId>,
) -> Result<(), BusinessEconomyError> {
    let operating = state
        .finance
        .get_account(operating_account)
        .ok_or(BusinessEconomyError::MissingAccount(operating_account))?;
    let settlement = state
        .finance
        .get_account(settlement_account)
        .ok_or(BusinessEconomyError::MissingAccount(settlement_account))?;
    for account in [operating, settlement] {
        if account.owner() != FinancialOwner::Business(business) {
            return Err(BusinessEconomyError::AccountOwnerMismatch {
                business,
                account: account.id(),
            });
        }
    }
    if operating.kind() != AccountKind::LegitimateOperating {
        return Err(BusinessEconomyError::InvalidOperatingAccountKind(
            operating_account,
        ));
    }
    if settlement.kind() != AccountKind::Settlement {
        return Err(BusinessEconomyError::InvalidSettlementAccountKind(
            settlement_account,
        ));
    }
    if let Some(existing) = state.economy.get_by_settlement_account(settlement_account)
        && Some(existing.business()) != current_business
    {
        return Err(BusinessEconomyError::SettlementAccountInUse {
            account: settlement_account,
            business: existing.business(),
        });
    }
    Ok(())
}

/// Applies the economy-owned plausibility-budget update for a laundering transaction after the
/// finance system has revalidated the business-economy version and committed the corresponding
/// ledger movement. Keeping this write behind the economy system preserves one mutation owner
/// without introducing a fallible step after money has moved.
pub(crate) fn apply_laundering_capacity_preflighted(
    state: &mut AppState,
    business: BusinessId,
    transaction: crate::core::id::LedgerTransactionId,
    total: Money,
) {
    state
        .economy
        .set_laundered_this_cycle(business, transaction, total);
}

/// Canonical sabotage-damage mutation: extends the target's disruption horizon through an
/// validated-then-committed plan so repeated attacks push the horizon later and staleness
/// is re-checked at commit.
pub struct ValidatedBusinessDisruption {
    business: BusinessId,
    expected_economy_version: u32,
    disrupted_through: SimTime,
    /// A second successful sabotage at the same instant is still a real operation outcome, but
    /// it does not change the already-equal damage horizon. Keep the token successful without
    /// manufacturing freshness churn in the economy owner.
    changes_horizon: bool,
    /// The instant the horizon was measured from. Commit rejects a token held across a
    /// clock advance, mirroring the cycle path's time-staleness convention.
    expected_now: SimTime,
}

impl ValidatedBusinessDisruption {
    pub(crate) fn ensure_current(&self, state: &AppState) -> Result<(), BusinessEconomyError> {
        crate::core::time::ensure_time_current(state.now(), self.expected_now).map_err(
            |(expected, found)| BusinessEconomyError::StaleDisruptionTime {
                business: self.business,
                expected,
                found,
            },
        )?;
        let economy = state
            .economy
            .get_business_economy(self.business)
            .ok_or(BusinessEconomyError::MissingBusinessEconomy(self.business))?;
        if economy.version() != self.expected_economy_version {
            return Err(BusinessEconomyError::StaleEconomy {
                business: self.business,
                expected: self.expected_economy_version,
                found: economy.version(),
            });
        }
        if self.changes_horizon {
            ensure_version_can_advance(economy.version(), "business economy")?;
        }
        Ok(())
    }

    pub fn commit(self, state: &mut AppState) -> Result<(), BusinessEconomyError> {
        self.ensure_current(state)?;
        if self.changes_horizon {
            state
                .economy
                .apply_disruption(self.business, self.disrupted_through);
        }
        Ok(())
    }
}

pub(crate) fn resolve_business_disruption_horizon(now: SimTime, duration: SimDuration) -> SimTime {
    let last_affected_minutes = duration
        .as_minutes()
        .checked_sub(1)
        .expect("registry-validated business disruption duration must be positive");
    now.checked_add(SimDuration::from_minutes(last_affected_minutes))
        .unwrap_or(SimTime::MAX)
}

pub fn validate_disrupt_business_economy(
    registry: &Registry,
    state: &AppState,
    business: BusinessId,
) -> Result<ValidatedBusinessDisruption, BusinessEconomyError> {
    let economy = state
        .economy
        .get_business_economy(business)
        .ok_or(BusinessEconomyError::MissingBusinessEconomy(business))?;
    if economy.status() != BusinessOperatingStatus::Active {
        return Err(BusinessEconomyError::EconomyNotActive(business));
    }
    // `disrupted_through` is inclusive, so an N-minute effect beginning now ends on minute N-1.
    // Using `now + duration` would make every sabotage one minute too long and can degrade an
    // extra full business cycle when the erroneous endpoint lands exactly on a settlement time.
    // If the inclusive endpoint lies beyond the finite simulation clock, clamp it to the last
    // representable minute instead of rejecting an otherwise successful sabotage.
    let disrupted_through =
        resolve_business_disruption_horizon(state.now(), registry.business_disruption().duration());
    let changes_horizon = economy
        .disrupted_through()
        .is_none_or(|current| disrupted_through > current);
    if changes_horizon {
        ensure_version_can_advance(economy.version(), "business economy")?;
    }
    Ok(ValidatedBusinessDisruption {
        business,
        expected_economy_version: economy.version(),
        disrupted_through,
        changes_horizon,
        expected_now: state.now(),
    })
}

#[cfg(test)]
mod tests;
