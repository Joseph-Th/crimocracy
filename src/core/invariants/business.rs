//! Release-safe structural validation for legitimate business economies.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::LedgerTransactionId;
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::economy::{BusinessCycleRecord, BusinessEconomyRecord, BusinessOperatingStatus};
use crate::finance::{AccountKind, FinancialOwner, Money};
use crate::intelligence::{InformationSourceKind, KnowledgeHolder, Reliability, Specificity};
use crate::world::BusinessOwner;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn validate_business_economies(state: &AppState) -> Result<(), StateValidationError> {
    for economy in state.economy.business_economies() {
        validate_business_economy_record(state, economy)?;
    }

    let mut previous_cycle_at = BTreeMap::new();
    let mut used_transactions: BTreeSet<LedgerTransactionId> = state
        .enterprises
        .cycles()
        .filter_map(|cycle| cycle.transaction())
        .collect();
    for cycle in state.economy.cycles() {
        validate_business_cycle(state, cycle, &mut previous_cycle_at, &mut used_transactions)?;
    }
    Ok(())
}

fn validate_business_economy_record(
    state: &AppState,
    economy: &BusinessEconomyRecord,
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
    if economy.laundered_this_cycle().cents() < 0 {
        return Err(invalid_economy(economy));
    }
    Ok(())
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
        BusinessOperatingStatus::Active => {
            let next_cycle_at = economy.next_cycle_at().ok_or_else(invalid)?;
            if next_cycle_at <= economy.established_at()
                || economy
                    .last_cycle_at()
                    .is_some_and(|last_cycle| next_cycle_at <= last_cycle)
            {
                return Err(invalid());
            }
        }
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

pub(super) fn validate_business_economies_against_registry(
    registry: &crate::registry::Registry,
    state: &AppState,
) -> Result<(), StateValidationError> {
    for economy in state.economy.business_economies() {
        if let Some(disrupted_through) = economy.disrupted_through() {
            let duration = u64::from(registry.business_disruption().duration().as_minutes());
            let min_horizon = economy
                .established_at()
                .as_minutes()
                .checked_add(duration)
                .ok_or(StateValidationError::InvalidBusinessEconomySchedule {
                    business: economy.business(),
                })?;
            let max_horizon = state.now().as_minutes().checked_add(duration).ok_or(
                StateValidationError::InvalidBusinessEconomySchedule {
                    business: economy.business(),
                },
            )?;
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
        let gross = crate::economy::business_economy_system::resolve_business_current_gross(
            registry,
            state,
            economy.business(),
        )
        .map_err(|_| StateValidationError::InvalidBusinessEconomy {
            business: economy.business(),
        })?;
        let capacity = crate::finance::helpers::resolve_basis_point_share(
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
