//! Release-safe structural validation for the business economy and enterprise subsystems.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::LedgerTransactionId;
use crate::core::invariants::StateValidationError;
use crate::core::state::AppState;
use crate::delegation::{MandateStatus, ResponsibilityFunction, ResponsibilityScope};
use crate::economy::BusinessOperatingStatus;
use crate::enterprises::{EnterpriseLocation, EnterpriseStatus};
use crate::finance::{AccountKind, FinancialOwner, Money};
use crate::intelligence::{InformationSourceKind, KnowledgeHolder, Reliability, Specificity};
use crate::world::BusinessOwner;
use std::collections::BTreeSet;

pub(super) fn validate_business_economies(state: &AppState) -> Result<(), StateValidationError> {
    for economy in state.economy.business_economies() {
        let _ = state.world.get_business(economy.business()).ok_or(
            StateValidationError::InvalidBusinessEconomy {
                business: economy.business(),
            },
        )?;
        let operating = state
            .finance
            .get_account(economy.operating_account())
            .ok_or(StateValidationError::InvalidBusinessEconomyAccounts {
                business: economy.business(),
            })?;
        let settlement = state
            .finance
            .get_account(economy.settlement_account())
            .ok_or(StateValidationError::InvalidBusinessEconomyAccounts {
                business: economy.business(),
            })?;
        if operating.owner() != FinancialOwner::Business(economy.business())
            || settlement.owner() != FinancialOwner::Business(economy.business())
            || operating.kind() != AccountKind::LegitimateOperating
            || settlement.kind() != AccountKind::Settlement
            || economy.operating_account() == economy.settlement_account()
        {
            return Err(StateValidationError::InvalidBusinessEconomyAccounts {
                business: economy.business(),
            });
        }
        if economy.established_at() > state.now()
            || economy
                .last_cycle_at()
                .is_some_and(|last_cycle| last_cycle > state.now())
        {
            return Err(StateValidationError::InvalidBusinessEconomySchedule {
                business: economy.business(),
            });
        }
        // Settlement order is sequential-ID order, so the newest cycle is a direct index
        // lookup; scanning the whole history per economy would grow with campaign length.
        let latest_cycle_at = state
            .economy
            .latest_cycle(economy.business())
            .map(|cycle| cycle.occurred_at());
        if latest_cycle_at != economy.last_cycle_at() {
            return Err(StateValidationError::InvalidBusinessEconomySchedule {
                business: economy.business(),
            });
        }
        match economy.status() {
            BusinessOperatingStatus::Active => {
                let next_cycle_at = economy.next_cycle_at().ok_or(
                    StateValidationError::InvalidBusinessEconomySchedule {
                        business: economy.business(),
                    },
                )?;
                if next_cycle_at <= economy.established_at()
                    || economy
                        .last_cycle_at()
                        .is_some_and(|last_cycle| next_cycle_at <= last_cycle)
                {
                    return Err(StateValidationError::InvalidBusinessEconomySchedule {
                        business: economy.business(),
                    });
                }
            }
            BusinessOperatingStatus::Suspended => {
                if economy.next_cycle_at().is_some() {
                    return Err(StateValidationError::InvalidBusinessEconomySchedule {
                        business: economy.business(),
                    });
                }
            }
        }
    }

    let mut used_transactions: BTreeSet<LedgerTransactionId> = state
        .enterprises
        .cycles()
        .filter_map(|cycle| cycle.transaction())
        .collect();
    for cycle in state.economy.cycles() {
        let economy = state
            .economy
            .get_business_economy(cycle.business())
            .ok_or(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() })?;
        let business = state
            .world
            .get_business(cycle.business())
            .ok_or(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() })?;
        let ownership = state
            .world
            .get_business_ownership_change_for_version(cycle.business(), cycle.business_version())
            .ok_or(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() })?;
        if cycle.occurred_at() < economy.established_at()
            || cycle.occurred_at() > state.now()
            || cycle.business_version() == 0
            || cycle.business_version() > business.version()
            || ownership.new_owner() != cycle.owner()
            || ownership.changed_at() > cycle.occurred_at()
            || cycle.gross_revenue().cents() < 0
            || cycle.operating_cost().cents() < 0
            || cycle.gross_revenue().checked_sub(cycle.operating_cost()) != Some(cycle.net_cash())
        {
            return Err(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() });
        }
        let expected_holder = match cycle.owner() {
            BusinessOwner::Independent => None,
            BusinessOwner::Organization(id) => Some(KnowledgeHolder::Organization(id)),
            BusinessOwner::Character(id) => Some(KnowledgeHolder::Character(id)),
        };
        match (cycle.attention(), expected_holder, cycle.information()) {
            (AttentionClass::Routine, _, None) | (AttentionClass::Notable, None, None) => {}
            (AttentionClass::Notable, Some(holder), Some(information_id)) => {
                let information = state
                    .intelligence
                    .get_information(information_id)
                    .ok_or(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() })?;
                if information.holder() != holder
                    || information.source_kind() != InformationSourceKind::Accountant
                    || information.source_entity().is_some()
                    || information.subject() != EntityRef::Business(cycle.business())
                    || information.observed_at() != cycle.occurred_at()
                    || information.recorded_at() != cycle.occurred_at()
                    || information.reliability() != Reliability::DirectAccess
                    || information.specificity() != Specificity::Precise
                {
                    return Err(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() });
                }
            }
            (AttentionClass::Routine, _, Some(_))
            | (AttentionClass::Notable, None, Some(_))
            | (AttentionClass::Notable, Some(_), None)
            | (AttentionClass::Exception | AttentionClass::Crisis, _, _) => {
                return Err(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() })
            }
        }
        match (cycle.net_cash() == Money::ZERO, cycle.transaction()) {
            (true, None) => {}
            (false, Some(transaction_id)) => {
                if !used_transactions.insert(transaction_id) {
                    return Err(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() });
                }
                let transaction = state
                    .finance
                    .get_transaction(transaction_id)
                    .ok_or(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() })?;
                let settlement_cents = cycle
                    .net_cash()
                    .cents()
                    .checked_neg()
                    .ok_or(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() })?;
                let has_operating = transaction.postings().iter().any(|posting| {
                    posting.account == economy.operating_account()
                        && posting.amount == cycle.net_cash()
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
                    return Err(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() });
                }
            }
            (true, Some(_)) | (false, None) => {
                return Err(StateValidationError::InvalidBusinessCycle { cycle: cycle.id() })
            }
        }
    }
    Ok(())
}

pub(super) fn validate_enterprises(state: &AppState) -> Result<(), StateValidationError> {
    for enterprise in state.enterprises.enterprises() {
        state
            .world
            .get_organization(enterprise.organization())
            .ok_or(StateValidationError::InvalidEnterpriseAuthority {
                enterprise: enterprise.id(),
            })?;
        let authority = enterprise.authority();
        let mandate = state.delegation.get_mandate(authority.mandate).ok_or(
            StateValidationError::InvalidEnterpriseAuthority {
                enterprise: enterprise.id(),
            },
        )?;
        let manager = state.world.get_character(authority.manager).ok_or(
            StateValidationError::InvalidEnterpriseAuthority {
                enterprise: enterprise.id(),
            },
        )?;
        if mandate.organization() != enterprise.organization()
            || mandate.manager() != authority.manager
            || enterprise.manager() != authority.manager
        {
            return Err(StateValidationError::InvalidEnterpriseAuthority {
                enterprise: enterprise.id(),
            });
        }

        let (neighborhood_id, location_is_active) = match enterprise.location() {
            EnterpriseLocation::Neighborhood(id) => {
                state.world.get_neighborhood(id).ok_or(
                    StateValidationError::InvalidEnterpriseLocation {
                        enterprise: enterprise.id(),
                    },
                )?;
                (id, true)
            }
            EnterpriseLocation::Business(id) => {
                let business = state.world.get_business(id).ok_or(
                    StateValidationError::InvalidEnterpriseLocation {
                        enterprise: enterprise.id(),
                    },
                )?;
                state
                    .world
                    .get_neighborhood(business.neighborhood())
                    .ok_or(StateValidationError::InvalidEnterpriseLocation {
                        enterprise: enterprise.id(),
                    })?;
                (business.neighborhood(), true)
            }
        };

        let mut supporting_businesses =
            Vec::with_capacity(enterprise.supporting_businesses().len());
        for business_id in enterprise.supporting_businesses() {
            if matches!(enterprise.location(), EnterpriseLocation::Business(location_id) if location_id == *business_id)
            {
                return Err(StateValidationError::InvalidEnterpriseSupportingBusiness {
                    enterprise: enterprise.id(),
                    business: *business_id,
                });
            }
            let business = state.world.get_business(*business_id).ok_or(
                StateValidationError::InvalidEnterpriseSupportingBusiness {
                    enterprise: enterprise.id(),
                    business: *business_id,
                },
            )?;
            supporting_businesses.push(business);
        }

        let cash = state.finance.get_account(enterprise.cash_account()).ok_or(
            StateValidationError::InvalidEnterpriseAccounts {
                enterprise: enterprise.id(),
            },
        )?;
        let settlement = state
            .finance
            .get_account(enterprise.settlement_account())
            .ok_or(StateValidationError::InvalidEnterpriseAccounts {
                enterprise: enterprise.id(),
            })?;
        let expected_owner = FinancialOwner::Organization(enterprise.organization());
        let cash_kind_is_valid = matches!(
            cash.kind(),
            AccountKind::StreetCash | AccountKind::ConcealedCash
        );
        if cash.owner() != expected_owner
            || settlement.owner() != expected_owner
            || !cash_kind_is_valid
            || settlement.kind() != AccountKind::Settlement
            || enterprise.cash_account() == enterprise.settlement_account()
        {
            return Err(StateValidationError::InvalidEnterpriseAccounts {
                enterprise: enterprise.id(),
            });
        }

        if enterprise.established_at() > state.now()
            || enterprise
                .last_cycle_at()
                .is_some_and(|last_cycle| last_cycle > state.now())
        {
            return Err(StateValidationError::InvalidEnterpriseSchedule {
                enterprise: enterprise.id(),
            });
        }
        // Settlement order is sequential-ID order, so the newest cycle is a direct index
        // lookup; scanning the whole history per enterprise would grow with campaign length.
        let latest_cycle_at = state
            .enterprises
            .latest_cycle(enterprise.id())
            .map(|cycle| cycle.occurred_at());
        if latest_cycle_at != enterprise.last_cycle_at() {
            return Err(StateValidationError::InvalidEnterpriseSchedule {
                enterprise: enterprise.id(),
            });
        }

        match enterprise.status() {
            EnterpriseStatus::Active => {
                let authority_covers_location = match authority.scope {
                    ResponsibilityScope::Function(ResponsibilityFunction::Enterprise) => true,
                    ResponsibilityScope::Function(
                        ResponsibilityFunction::Territory
                        | ResponsibilityFunction::Operations
                        | ResponsibilityFunction::Intelligence
                        | ResponsibilityFunction::Finance
                        | ResponsibilityFunction::Legal
                        | ResponsibilityFunction::Political
                        | ResponsibilityFunction::Personnel,
                    ) => false,
                    ResponsibilityScope::Neighborhood(id) => id == neighborhood_id,
                    ResponsibilityScope::Business(id) => {
                        matches!(enterprise.location(), EnterpriseLocation::Business(location_id) if location_id == id)
                    }
                };
                let next_cycle_at = enterprise.next_cycle_at().ok_or(
                    StateValidationError::InvalidEnterpriseSchedule {
                        enterprise: enterprise.id(),
                    },
                )?;
                if manager.organization() != Some(enterprise.organization())
                    || mandate.status() != MandateStatus::Active
                    || !mandate.scopes().contains(&authority.scope)
                    || !authority_covers_location
                    || !location_is_active
                    || supporting_businesses.iter().any(|business| {
                        business.owner() != BusinessOwner::Organization(enterprise.organization())
                    })
                {
                    return Err(StateValidationError::InvalidEnterpriseAuthority {
                        enterprise: enterprise.id(),
                    });
                }
                if next_cycle_at <= enterprise.established_at()
                    || enterprise
                        .last_cycle_at()
                        .is_some_and(|last_cycle| next_cycle_at <= last_cycle)
                {
                    return Err(StateValidationError::InvalidEnterpriseSchedule {
                        enterprise: enterprise.id(),
                    });
                }
            }
            EnterpriseStatus::Suspended => {
                if enterprise.next_cycle_at().is_some() {
                    return Err(StateValidationError::InvalidEnterpriseSchedule {
                        enterprise: enterprise.id(),
                    });
                }
            }
        }
    }

    let mut used_transactions = BTreeSet::new();
    for cycle in state.enterprises.cycles() {
        let enterprise = state
            .enterprises
            .get_enterprise(cycle.enterprise())
            .ok_or(StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() })?;
        if cycle.occurred_at() < enterprise.established_at()
            || cycle.occurred_at() > state.now()
            || cycle.gross_revenue().cents() < 0
            || cycle.operating_cost().cents() < 0
            || cycle.gross_revenue().checked_sub(cycle.operating_cost()) != Some(cycle.net_cash())
        {
            return Err(StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() });
        }
        match cycle.attention() {
            AttentionClass::Routine => {
                if cycle.information().is_some() {
                    return Err(StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() });
                }
            }
            AttentionClass::Notable => {
                let information_id = cycle
                    .information()
                    .ok_or(StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() })?;
                let information = state
                    .intelligence
                    .get_information(information_id)
                    .ok_or(StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() })?;
                if information.holder() != KnowledgeHolder::Organization(enterprise.organization())
                    || information.source_kind() != InformationSourceKind::AfterAction
                    || information.source_entity()
                        != Some(EntityRef::Character(enterprise.manager()))
                    || information.subject() != EntityRef::Enterprise(enterprise.id())
                    || information.observed_at() != cycle.occurred_at()
                    || information.recorded_at() != cycle.occurred_at()
                    || information.reliability() != Reliability::DirectAccess
                    || information.specificity() != Specificity::Precise
                {
                    return Err(StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() });
                }
            }
            AttentionClass::Exception | AttentionClass::Crisis => {
                return Err(StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() })
            }
        }
        match (cycle.net_cash() == Money::ZERO, cycle.transaction()) {
            (true, None) => {}
            (false, Some(transaction_id)) => {
                if !used_transactions.insert(transaction_id) {
                    return Err(StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() });
                }
                let transaction = state
                    .finance
                    .get_transaction(transaction_id)
                    .ok_or(StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() })?;
                let settlement_cents =
                    cycle.net_cash().cents().checked_neg().ok_or(
                        StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() },
                    )?;
                let has_cash = transaction.postings().iter().any(|posting| {
                    posting.account == enterprise.cash_account()
                        && posting.amount == cycle.net_cash()
                });
                let has_settlement = transaction.postings().iter().any(|posting| {
                    posting.account == enterprise.settlement_account()
                        && posting.amount == Money::from_cents(settlement_cents)
                });
                if transaction.occurred_at() != cycle.occurred_at()
                    || transaction.postings().len() != 2
                    || !has_cash
                    || !has_settlement
                {
                    return Err(StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() });
                }
            }
            (true, Some(_)) | (false, None) => {
                return Err(StateValidationError::InvalidEnterpriseCycle { cycle: cycle.id() })
            }
        }
    }
    Ok(())
}
