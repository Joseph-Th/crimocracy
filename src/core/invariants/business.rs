//! Release-safe structural validation for legitimate business economies.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::LedgerTransactionId;
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::economy::{BusinessCycleRecord, BusinessEconomyRecord, BusinessOperatingStatus};
use crate::finance::{AccountKind, FinancialOwner, Money};
use crate::intelligence::{InformationSourceKind, KnowledgeHolder, Reliability, Specificity};
use crate::registry::Registry;
use crate::world::{BusinessFunction, BusinessOwner, OrganizationKind};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn validate_business_economies(state: &AppState) -> Result<(), StateValidationError> {
    let mut used_transactions: BTreeSet<LedgerTransactionId> = state
        .enterprises
        .cycles()
        .filter_map(|cycle| cycle.transaction())
        .collect();
    for economy in state.economy.business_economies() {
        validate_business_economy_record(state, economy, &mut used_transactions)?;
    }

    let mut previous_cycle_at = BTreeMap::new();
    for cycle in state.economy.cycles() {
        validate_business_cycle(state, cycle, &mut previous_cycle_at, &mut used_transactions)?;
    }
    Ok(())
}

fn validate_business_economy_record(
    state: &AppState,
    economy: &BusinessEconomyRecord,
    used_transactions: &mut BTreeSet<LedgerTransactionId>,
) -> Result<(), StateValidationError> {
    if economy.version() == 0 {
        return Err(invalid_economy(economy));
    }
    state
        .world
        .get_business(economy.business())
        .ok_or_else(|| invalid_economy(economy))?;
    validate_business_economy_accounts(state, economy)?;
    validate_business_economy_schedule(state, economy)?;
    if economy.laundered_this_cycle().cents() < 0
        || !validate_current_laundering_window(state, economy, used_transactions)?
    {
        return Err(invalid_economy(economy));
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct LaunderingTransactionAmounts {
    amount: Money,
    accounted: Money,
    fee: Money,
}

fn validate_current_laundering_window(
    state: &AppState,
    economy: &BusinessEconomyRecord,
    used_transactions: &mut BTreeSet<LedgerTransactionId>,
) -> Result<bool, StateValidationError> {
    let transactions = economy.laundering_transactions_this_cycle();
    if transactions.is_empty() != (economy.laundered_this_cycle() == Money::ZERO) {
        return Ok(false);
    }
    if transactions.is_empty() {
        return Ok(true);
    }
    let business = state
        .world
        .get_business(economy.business())
        .ok_or_else(|| invalid_economy(economy))?;
    let BusinessOwner::Organization(organization) = business.owner() else {
        return Ok(false);
    };
    let window_start = [
        Some(economy.established_at()),
        economy.last_cycle_at(),
        economy.loss_streak_anchor(),
    ]
    .into_iter()
    .flatten()
    .max()
    .expect("business economy always has an establishment instant");
    let mut total = Money::ZERO;
    for transaction_id in transactions {
        if !used_transactions.insert(*transaction_id) {
            return Ok(false);
        }
        let transaction = state
            .finance
            .get_transaction(*transaction_id)
            .ok_or_else(|| invalid_economy(economy))?;
        if transaction.occurred_at() < window_start
            || transaction.occurred_at() > state.now()
            || transaction.budget_usage().is_some()
        {
            return Ok(false);
        }
        let Some(amounts) =
            laundering_transaction_amounts(state, economy, organization, transaction)
        else {
            return Ok(false);
        };
        total = total
            .checked_add(amounts.amount)
            .ok_or_else(|| invalid_economy(economy))?;
    }
    Ok(total == economy.laundered_this_cycle())
}

fn laundering_transaction_amounts(
    state: &AppState,
    economy: &BusinessEconomyRecord,
    organization: crate::core::id::OrganizationId,
    transaction: &crate::finance::LedgerTransactionRecord,
) -> Option<LaunderingTransactionAmounts> {
    if transaction.postings().len() != 3 {
        return None;
    }
    let mut amount = None;
    let mut accounted = None;
    let mut fee = None;
    for posting in transaction.postings() {
        let account = state.finance.get_account(posting.account)?;
        if posting.amount < Money::ZERO
            && account.owner() == FinancialOwner::Organization(organization)
            && account.kind() == AccountKind::StreetCash
            && amount.is_none()
        {
            amount = posting.amount.checked_neg();
        } else if posting.amount > Money::ZERO
            && account.owner() == FinancialOwner::Organization(organization)
            && account.kind() == AccountKind::AccountedFunds
            && accounted.is_none()
        {
            accounted = Some(posting.amount);
        } else if posting.amount > Money::ZERO
            && posting.account == economy.operating_account()
            && account.owner() == FinancialOwner::Business(economy.business())
            && account.kind() == AccountKind::LegitimateOperating
            && fee.is_none()
        {
            fee = Some(posting.amount);
        } else {
            return None;
        }
    }
    let amount = amount?;
    let accounted = accounted?;
    let fee = fee?;
    (accounted.checked_add(fee) == Some(amount)).then_some(LaunderingTransactionAmounts {
        amount,
        accounted,
        fee,
    })
}

fn validate_business_economy_accounts(
    state: &AppState,
    economy: &BusinessEconomyRecord,
) -> Result<(), StateValidationError> {
    let invalid = || StateValidationError::InvalidBusinessEconomyAccounts {
        business: economy.business(),
    };
    let operating = state
        .finance
        .get_account(economy.operating_account())
        .ok_or_else(invalid)?;
    let settlement = state
        .finance
        .get_account(economy.settlement_account())
        .ok_or_else(invalid)?;
    if operating.owner() != FinancialOwner::Business(economy.business())
        || settlement.owner() != FinancialOwner::Business(economy.business())
        || operating.kind() != AccountKind::LegitimateOperating
        || settlement.kind() != AccountKind::Settlement
        || economy.operating_account() == economy.settlement_account()
    {
        return Err(invalid());
    }
    Ok(())
}

fn validate_business_economy_schedule(
    state: &AppState,
    economy: &BusinessEconomyRecord,
) -> Result<(), StateValidationError> {
    let invalid = || StateValidationError::InvalidBusinessEconomySchedule {
        business: economy.business(),
    };
    if economy.established_at() > state.now()
        || economy
            .last_cycle_at()
            .is_some_and(|last_cycle| last_cycle > state.now())
        || economy
            .loss_streak_anchor()
            .is_some_and(|anchor| anchor < economy.established_at() || anchor > state.now())
    {
        return Err(invalid());
    }
    // Settlement order is sequential-ID order, so the newest cycle is a direct index lookup.
    if state
        .economy
        .latest_cycle(economy.business())
        .map(|cycle| cycle.occurred_at())
        != economy.last_cycle_at()
    {
        return Err(invalid());
    }
    match economy.status() {
        BusinessOperatingStatus::Active => match economy.next_cycle_at() {
            Some(next_cycle_at) => {
                if next_cycle_at <= economy.established_at()
                    || economy
                        .last_cycle_at()
                        .is_some_and(|last_cycle| next_cycle_at <= last_cycle)
                {
                    return Err(invalid());
                }
            }
            None if economy.last_cycle_at().is_some() => {}
            None => return Err(invalid()),
        },
        BusinessOperatingStatus::Suspended if economy.next_cycle_at().is_some() => {
            return Err(invalid());
        }
        BusinessOperatingStatus::Suspended => {}
    }
    Ok(())
}

fn validate_business_cycle(
    state: &AppState,
    cycle: &BusinessCycleRecord,
    previous_cycle_at: &mut BTreeMap<crate::core::id::BusinessId, crate::core::time::SimTime>,
    used_transactions: &mut BTreeSet<LedgerTransactionId>,
) -> Result<(), StateValidationError> {
    let invalid = || invalid_cycle(cycle);
    let economy = state
        .economy
        .get_business_economy(cycle.business())
        .ok_or_else(invalid)?;
    let business = state
        .world
        .get_business(cycle.business())
        .ok_or_else(invalid)?;
    let ownership = state
        .world
        .get_business_ownership_change_for_version(cycle.business(), cycle.business_version())
        .ok_or_else(invalid)?;
    let prior_cycle_at = previous_cycle_at.insert(cycle.business(), cycle.occurred_at());
    if cycle.occurred_at() <= economy.established_at()
        || cycle.occurred_at() > state.now()
        || prior_cycle_at.is_some_and(|prior| cycle.occurred_at() <= prior)
        || cycle.business_version() == 0
        || cycle.business_version() > business.version()
        || ownership.new_owner() != cycle.owner()
        || ownership.changed_at() > cycle.occurred_at()
        || cycle.gross_revenue().cents() < 0
        || cycle.operating_cost().cents() < 0
        || cycle.gross_revenue().checked_sub(cycle.operating_cost()) != Some(cycle.net_cash())
    {
        return Err(invalid());
    }
    validate_business_cycle_information(state, cycle)?;
    validate_business_cycle_transaction(state, economy, cycle, used_transactions)
}

fn validate_business_cycle_information(
    state: &AppState,
    cycle: &BusinessCycleRecord,
) -> Result<(), StateValidationError> {
    let expected_holder = match cycle.owner() {
        BusinessOwner::Independent => None,
        BusinessOwner::Organization(id) => Some(KnowledgeHolder::Organization(id)),
        BusinessOwner::Character(id) => Some(KnowledgeHolder::Character(id)),
    };
    match (cycle.attention(), expected_holder, cycle.information()) {
        (AttentionClass::Routine, _, None) | (AttentionClass::Notable, None, None) => Ok(()),
        (AttentionClass::Notable, Some(holder), Some(information_id)) => {
            let information = state
                .intelligence
                .get_information(information_id)
                .ok_or_else(|| invalid_cycle(cycle))?;
            if information.holder() != holder
                || information.source_kind() != InformationSourceKind::Accountant
                || information.source_entity().is_some()
                || information.subject() != EntityRef::Business(cycle.business())
                || information.observed_at() != cycle.occurred_at()
                || information.recorded_at() != cycle.occurred_at()
                || information.reliability() != Reliability::DirectAccess
                || information.specificity() != Specificity::Precise
            {
                return Err(invalid_cycle(cycle));
            }
            Ok(())
        }
        (AttentionClass::Routine, _, Some(_))
        | (AttentionClass::Notable, None, Some(_))
        | (AttentionClass::Notable, Some(_), None)
        | (AttentionClass::Exception | AttentionClass::Crisis, _, _) => Err(invalid_cycle(cycle)),
    }
}

fn validate_business_cycle_transaction(
    state: &AppState,
    economy: &BusinessEconomyRecord,
    cycle: &BusinessCycleRecord,
    used_transactions: &mut BTreeSet<LedgerTransactionId>,
) -> Result<(), StateValidationError> {
    match (cycle.net_cash() == Money::ZERO, cycle.transaction()) {
        (true, None) => Ok(()),
        (false, Some(transaction_id)) => {
            if !used_transactions.insert(transaction_id) {
                return Err(invalid_cycle(cycle));
            }
            let transaction = state
                .finance
                .get_transaction(transaction_id)
                .ok_or_else(|| invalid_cycle(cycle))?;
            let settlement_cents = cycle
                .net_cash()
                .cents()
                .checked_neg()
                .ok_or_else(|| invalid_cycle(cycle))?;
            let has_operating = transaction.postings().iter().any(|posting| {
                posting.account == economy.operating_account() && posting.amount == cycle.net_cash()
            });
            let has_settlement = transaction.postings().iter().any(|posting| {
                posting.account == economy.settlement_account()
                    && posting.amount == Money::from_cents(settlement_cents)
            });
            if transaction.occurred_at() != cycle.occurred_at()
                || transaction.postings().len() != 2
                || !has_operating
                || !has_settlement
            {
                return Err(invalid_cycle(cycle));
            }
            Ok(())
        }
        (true, Some(_)) | (false, None) => Err(invalid_cycle(cycle)),
    }
}

fn invalid_economy(economy: &BusinessEconomyRecord) -> StateValidationError {
    StateValidationError::InvalidBusinessEconomy {
        business: economy.business(),
    }
}

fn invalid_cycle(cycle: &BusinessCycleRecord) -> StateValidationError {
    StateValidationError::InvalidBusinessCycle { cycle: cycle.id() }
}

pub(super) fn validate_businesses_against_registry(
    registry: &Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    validate_business_cycles_against_registry(registry, state)?;
    validate_business_economies_against_registry(registry, state)
}

fn validate_business_cycles_against_registry(
    registry: &Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    for cycle in state.economy.cycles() {
        let business = state
            .world
            .get_business(cycle.business())
            .ok_or(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() })?;
        let economics = registry.get_business(business.kind()).economics();
        let (expected_gross, expected_cost, expected_net) =
            crate::economy::business_economy_system::resolve_historical_business_cycle_financials(
                registry, state, cycle,
            )
            .map_err(|_| StateValidationError::InvalidBusinessCycle { cycle: cycle.id() })?;
        if cycle.gross_revenue() != expected_gross
            || cycle.operating_cost() != expected_cost
            || cycle.net_cash() != expected_net
        {
            return Err(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() });
        }
        let variance = i32::from(cycle.variance_basis_points()).unsigned_abs();
        let expected_attention = if variance >= u32::from(economics.notable_variance_basis_points())
            || cycle.net_cash() < Money::ZERO
            || cycle.disrupted()
        {
            AttentionClass::Notable
        } else {
            AttentionClass::Routine
        };
        if cycle.attention() != expected_attention {
            return Err(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() });
        }
    }
    Ok(())
}

fn validate_business_economies_against_registry(
    registry: &Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    for economy in state.economy.business_economies() {
        if economy.status() == BusinessOperatingStatus::Active {
            let business = state
                .world
                .get_business(economy.business())
                .ok_or_else(|| invalid_economy(economy))?;
            let cycle = registry.get_business(business.kind()).economics().cycle();
            let schedule_base = [
                Some(economy.established_at()),
                economy.last_cycle_at(),
                economy.loss_streak_anchor(),
            ]
            .into_iter()
            .flatten()
            .max()
            .expect("established business economy always has a schedule base");
            let expected_next = schedule_base.checked_add(cycle);
            if economy.next_cycle_at() != expected_next {
                return Err(StateValidationError::InvalidBusinessEconomySchedule {
                    business: economy.business(),
                });
            }
        }
        if let Some(disrupted_through) = economy.disrupted_through() {
            let duration = registry.business_disruption().duration();
            // The mutation owner defines disruption endpoints. Reuse it here so restore
            // validation cannot drift from the inclusive N-minute interval convention or its
            // finite-clock clamping behavior.
            let min_horizon =
                crate::economy::business_economy_system::resolve_business_disruption_horizon(
                    economy.established_at(),
                    duration,
                )
                .as_minutes();
            let max_horizon =
                crate::economy::business_economy_system::resolve_business_disruption_horizon(
                    state.now(),
                    duration,
                )
                .as_minutes();
            let horizon = disrupted_through.as_minutes();
            if horizon < min_horizon || horizon > max_horizon {
                return Err(StateValidationError::InvalidBusinessEconomySchedule {
                    business: economy.business(),
                });
            }
        }
        let laundered = economy.laundered_this_cycle();
        if laundered == crate::finance::Money::ZERO {
            continue;
        }
        let business = state
            .world
            .get_business(economy.business())
            .ok_or_else(|| invalid_economy(economy))?;
        let BusinessOwner::Organization(organization) = business.owner() else {
            return Err(invalid_economy(economy));
        };
        let organization = state
            .world
            .get_organization(organization)
            .ok_or_else(|| invalid_economy(economy))?;
        if organization.kind() != OrganizationKind::Criminal
            || !business
                .functions()
                .contains(&BusinessFunction::CashIntensive)
        {
            return Err(invalid_economy(economy));
        }
        for transaction_id in economy.laundering_transactions_this_cycle() {
            let transaction = state
                .finance
                .get_transaction(*transaction_id)
                .ok_or_else(|| invalid_economy(economy))?;
            let amounts =
                laundering_transaction_amounts(state, economy, organization.id(), transaction)
                    .ok_or_else(|| invalid_economy(economy))?;
            let expected_fee = crate::finance::helpers::apply_basis_point_multiplier(
                amounts.amount,
                registry.laundering().fee_basis_points(),
            )
            .ok_or_else(|| invalid_economy(economy))?;
            let expected_accounted = amounts
                .amount
                .checked_sub(expected_fee)
                .ok_or_else(|| invalid_economy(economy))?;
            if expected_fee <= crate::finance::Money::ZERO
                || expected_accounted <= crate::finance::Money::ZERO
                || amounts.fee != expected_fee
                || amounts.accounted != expected_accounted
            {
                return Err(invalid_economy(economy));
            }
        }
        // Restore can prove the immutable normal-gross ceiling for this operating window, but
        // not whether each historical laundering transfer happened before or after a later
        // sabotage hit because disruption start history is not duplicated on the economy
        // record. Live laundering always enforces the stricter current (possibly disrupted)
        // capacity at the transfer instant. Rechecking today's degraded capacity here would
        // retroactively invalidate money that was legitimately laundered before later damage.
        let gross = crate::economy::business_economy_system::resolve_business_gross_potential(
            registry,
            state,
            economy.business(),
        )
        .map_err(|_| StateValidationError::InvalidBusinessEconomy {
            business: economy.business(),
        })?;
        let capacity = crate::finance::helpers::apply_basis_point_multiplier(
            gross,
            registry.laundering().plausibility_gross_basis_points(),
        )
        .ok_or(StateValidationError::InvalidBusinessEconomy {
            business: economy.business(),
        })?;
        if laundered > capacity {
            return Err(StateValidationError::InvalidBusinessEconomy {
                business: economy.business(),
            });
        }
    }
    Ok(())
}
