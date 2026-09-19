//! Business economy establishment, deterministic cycle planning, and atomic ledger settlement.

mod lifecycle;

pub(crate) use lifecycle::validate_acquisition_restart;
pub use lifecycle::{
    ValidatedBusinessEconomyStatusChange, validate_resume_business_economy,
    validate_suspend_business_economy,
};

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{
    BusinessCycleId, BusinessId, FinancialAccountId, IdExhaustionError, IdKind, OrganizationId,
};
use crate::core::state::AppState;
use crate::core::time::{SimDuration, SimTime};
use crate::core::version::{
    VersionCapacityError, ensure_version_can_advance, ensure_version_can_advance_by,
};
use crate::economy::{
    BusinessCycleRecord, BusinessEconomyDraft, BusinessOperatingStatus, OperatingCapitalFloor,
    build_business_economy_record,
};
use crate::finance::finance_system::{
    FinanceError, ValidatedFinancialAccountOpenings, ValidatedLedgerTransaction,
    validate_record_business_transaction,
};
use crate::finance::{
    AccountKind, FinancialOwner, LedgerPosting, LedgerTransactionDraft, Money,
    helpers::format_money_cents,
};
use crate::intelligence::intelligence_system::{
    IntelligenceError, ValidatedInformation, validate_record_information,
};
use crate::intelligence::{
    InformationDraft, InformationSourceKind, KnowledgeHolder, Reliability, Specificity,
};
use crate::registry::{BusinessEconomicsDefinition, Registry};
use crate::world::{BusinessOwner, NeighborhoodProfile};
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct BusinessCycleSnapshot {
    business: BusinessId,
    expected_business_version: u32,
    owner: BusinessOwner,
    expected_economy_version: u32,
    occurred_at: SimTime,
    /// `None` means this cycle settled successfully but its next authored recurrence lies beyond
    /// the finite simulation clock. The economy remains operational; there is simply no further
    /// representable settlement instant to schedule.
    next_cycle_at: Option<SimTime>,
    /// Set when this losing settlement reaches the authored consecutive-loss threshold:
    /// commit suspends the economy instead of leaving the next cycle scheduled.
    suspends_after_settlement: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BusinessCycleEconomics {
    gross_revenue: Money,
    operating_cost: Money,
    net_cash: Money,
    variance_basis_points: i16,
    disrupted: bool,
    attention: AttentionClass,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BusinessCycleAccounts {
    operating_account: FinancialAccountId,
    settlement_account: FinancialAccountId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BusinessCyclePlan {
    snapshot: BusinessCycleSnapshot,
    economics: BusinessCycleEconomics,
    accounts: BusinessCycleAccounts,
}

pub fn decide_business_cycle(
    registry: &Registry,
    state: &AppState,
    business: BusinessId,
    variance_basis_points: i16,
) -> Result<BusinessCyclePlan, BusinessEconomyError> {
    let business_record = validate_business(state, business)?;
    let economy = state
        .economy
        .get_business_economy(business)
        .ok_or(BusinessEconomyError::MissingBusinessEconomy(business))?;
    if economy.status() != BusinessOperatingStatus::Active {
        return Err(BusinessEconomyError::EconomyNotActive(business));
    }
    let Some(due_at) = economy.next_cycle_at() else {
        return Err(BusinessEconomyError::SimulationTimeOverflow);
    };
    if state.now() < due_at {
        return Err(BusinessEconomyError::CycleNotDue { business, due_at });
    }
    validate_accounts(
        state,
        business,
        economy.operating_account(),
        economy.settlement_account(),
        Some(business),
    )?;
    let definition = registry.get_business(business_record.kind());
    let economics = definition.economics();
    let neighborhood = state
        .world
        .get_neighborhood(business_record.neighborhood())
        .ok_or(BusinessEconomyError::MissingBusinessNeighborhood(business))?;
    let profile = neighborhood.profile();
    // Freeze the disruption fact into this cycle. The economy retains only the current horizon,
    // so historical cycle validation must never infer past disruption from a later sabotage hit.
    let disrupted = economy.is_disrupted(state.now());
    let (gross_revenue, operating_cost, net_cash) = resolve_cycle_financials(
        business,
        economics,
        profile,
        registry.business_disruption().gross_basis_points(),
        disrupted,
        variance_basis_points,
    )?;
    // A losing cycle or a sabotage-degraded gross is always accountant-worthy: chronic
    // silent losses and invisible sabotage are exactly what the owner must see before the
    // authored suspension threshold stops the bleeding.
    let attention = if net_cash < Money::ZERO
        || disrupted
        || i32::from(variance_basis_points).unsigned_abs()
            >= u32::from(economics.notable_variance_basis_points())
    {
        AttentionClass::Notable
    } else {
        AttentionClass::Routine
    };
    let trailing_losing_cycles =
        count_trailing_losing_cycles(state, business, economics.losing_cycles_before_suspension());
    // A losing settlement that reaches the authored consecutive-loss threshold suspends the
    // economy: the domain owner acts on the negative result instead of scheduling another
    // identical loss. Resume stays a manual canonical decision.
    let suspends_after_settlement = net_cash < Money::ZERO
        && trailing_losing_cycles + 1 >= u32::from(economics.losing_cycles_before_suspension());
    // Settling work that is due now must not fail only because its *next* recurrence lies past
    // the finite clock. `None` is persisted as an exhausted recurrence and registry-aware
    // validation proves that the authored cadence really does overflow from this settlement.
    let next_cycle_at = state.now().checked_add(economics.cycle());
    Ok(BusinessCyclePlan {
        snapshot: BusinessCycleSnapshot {
            business,
            expected_business_version: business_record.version(),
            owner: business_record.owner(),
            expected_economy_version: economy.version(),
            occurred_at: state.now(),
            // Re-anchor to the actual settlement instant (mirroring enterprise cycles): if a
            // business settles late (e.g. after a multi-minute advance), the next cycle starts
            // from now rather than from the stale due time, so missed cycles do not resolve as a
            // rapid one-per-minute backlog when work resumes.
            next_cycle_at,
            suspends_after_settlement,
        },
        economics: BusinessCycleEconomics {
            gross_revenue,
            operating_cost,
            net_cash,
            variance_basis_points,
            disrupted,
            attention,
        },
        accounts: BusinessCycleAccounts {
            operating_account: economy.operating_account(),
            settlement_account: economy.settlement_account(),
        },
    })
}

/// Consecutive most-recent settled cycles whose net cash was negative, capped at `limit` so
/// the scan stays bounded regardless of how much history a long-lived business accumulates.
/// Cycles settled before the economy's loss-streak anchor predate its current grace window
/// (a resumed business starts counting fresh) and do not extend the streak.
fn count_trailing_losing_cycles(state: &AppState, business: BusinessId, limit: u8) -> u32 {
    let anchor = state
        .economy
        .get_business_economy(business)
        .and_then(|economy| economy.loss_streak_anchor());
    crate::finance::helpers::count_trailing_losing_cycles(
        state
            .economy
            .cycles_for(business)
            .rev()
            .take(usize::from(limit)),
        |cycle| cycle.occurred_at(),
        |cycle| cycle.net_cash(),
        anchor,
        limit,
    )
}

pub struct ValidatedBusinessCycle {
    plan: BusinessCyclePlan,
    ledger: Option<ValidatedLedgerTransaction>,
    information: Option<ValidatedInformation>,
}

impl ValidatedBusinessCycle {
    pub fn commit(self, state: &mut AppState) -> Result<BusinessCycleId, BusinessEconomyError> {
        let mut budget = Vec::new();
        if self.ledger.is_some() {
            budget.push((IdKind::LedgerTransaction, 1));
        }
        if self.information.is_some() {
            budget.push((IdKind::Information, 1));
        }
        budget.push((IdKind::BusinessCycle, 1));
        state.ids.reserve_many(&budget)?;
        let business = validate_business(state, self.plan.snapshot.business)?;
        if business.version() != self.plan.snapshot.expected_business_version {
            return Err(BusinessEconomyError::StaleBusiness {
                business: self.plan.snapshot.business,
                expected: self.plan.snapshot.expected_business_version,
                found: business.version(),
            });
        }
        let economy = state
            .economy
            .get_business_economy(self.plan.snapshot.business)
            .ok_or(BusinessEconomyError::MissingBusinessEconomy(
                self.plan.snapshot.business,
            ))?;
        if economy.version() != self.plan.snapshot.expected_economy_version {
            return Err(BusinessEconomyError::StaleEconomy {
                business: self.plan.snapshot.business,
                expected: self.plan.snapshot.expected_economy_version,
                found: economy.version(),
            });
        }
        ensure_version_can_advance_by(
            economy.version(),
            1 + u32::from(self.plan.snapshot.suspends_after_settlement),
            "business economy",
        )?;
        if economy.status() != BusinessOperatingStatus::Active {
            return Err(BusinessEconomyError::EconomyNotActive(
                self.plan.snapshot.business,
            ));
        }
        crate::core::time::ensure_time_current(state.now(), self.plan.snapshot.occurred_at)
            .map_err(|(expected, found)| BusinessEconomyError::StaleCycleTime {
                expected,
                found,
            })?;
        validate_accounts(
            state,
            self.plan.snapshot.business,
            self.plan.accounts.operating_account,
            self.plan.accounts.settlement_account,
            Some(self.plan.snapshot.business),
        )?;
        let transaction = match self.ledger {
            Some(ledger) => Some(ledger.commit(state)?),
            None => None,
        };
        let information = self.information.map(|information| {
            information
                .commit(state)
                .expect("business-cycle information ID was preflighted before mutation")
        });
        let cycle = state
            .ids
            .next_business_cycle()
            .expect("business-cycle ID was preflighted before settlement mutation");
        state.economy.apply_cycle(
            BusinessCycleRecord {
                id: cycle,
                context: super::BusinessCycleContext {
                    business: self.plan.snapshot.business,
                    business_version: self.plan.snapshot.expected_business_version,
                    owner: self.plan.snapshot.owner,
                    occurred_at: self.plan.snapshot.occurred_at,
                },
                financials: super::BusinessCycleFinancials {
                    gross_revenue: self.plan.economics.gross_revenue,
                    operating_cost: self.plan.economics.operating_cost,
                    net_cash: self.plan.economics.net_cash,
                    variance_basis_points: self.plan.economics.variance_basis_points,
                    disrupted: self.plan.economics.disrupted,
                },
                artifacts: super::BusinessCycleArtifacts {
                    attention: self.plan.economics.attention,
                    transaction,
                    information,
                },
            },
            self.plan.snapshot.next_cycle_at,
        );
        if self.plan.snapshot.suspends_after_settlement {
            // Domain-owner consequence for chronic losses: suspend instead of scheduling
            // another identical loss. Resumption is a manual canonical decision.
            state.economy.set_status(
                self.plan.snapshot.business,
                BusinessOperatingStatus::Suspended,
                None,
                None,
                false,
                None,
            );
        }
        Ok(cycle)
    }
}

pub fn validate_business_cycle_plan(
    state: &AppState,
    plan: BusinessCyclePlan,
) -> Result<ValidatedBusinessCycle, BusinessEconomyError> {
    let business = validate_business(state, plan.snapshot.business)?;
    if business.version() != plan.snapshot.expected_business_version {
        return Err(BusinessEconomyError::StaleBusiness {
            business: plan.snapshot.business,
            expected: plan.snapshot.expected_business_version,
            found: business.version(),
        });
    }
    let economy = state
        .economy
        .get_business_economy(plan.snapshot.business)
        .ok_or(BusinessEconomyError::MissingBusinessEconomy(
            plan.snapshot.business,
        ))?;
    if economy.version() != plan.snapshot.expected_economy_version {
        return Err(BusinessEconomyError::StaleEconomy {
            business: plan.snapshot.business,
            expected: plan.snapshot.expected_economy_version,
            found: economy.version(),
        });
    }
    ensure_version_can_advance_by(
        economy.version(),
        1 + u32::from(plan.snapshot.suspends_after_settlement),
        "business economy",
    )?;
    if economy.status() != BusinessOperatingStatus::Active {
        return Err(BusinessEconomyError::EconomyNotActive(
            plan.snapshot.business,
        ));
    }
    crate::core::time::ensure_time_current(state.now(), plan.snapshot.occurred_at)
        .map_err(|(expected, found)| BusinessEconomyError::StaleCycleTime { expected, found })?;
    validate_accounts(
        state,
        plan.snapshot.business,
        plan.accounts.operating_account,
        plan.accounts.settlement_account,
        Some(plan.snapshot.business),
    )?;
    // A balanced settlement moves no money, and the ledger rejects zero-value postings, so
    // net-zero cycles record their modeled gross/cost financials without a ledger transaction
    // (see `core::invariants::business` for the matching validity rule).
    let ledger = if plan.economics.net_cash == Money::ZERO {
        None
    } else {
        let postings = crate::finance::helpers::build_settlement_postings(
            plan.accounts.operating_account,
            plan.accounts.settlement_account,
            plan.economics.net_cash,
        )
        .ok_or(BusinessEconomyError::ArithmeticOverflow(
            plan.snapshot.business,
        ))?;
        Some(validate_record_business_transaction(
            state,
            LedgerTransactionDraft {
                occurred_at: plan.snapshot.occurred_at,
                memo: format!(
                    "Routine legitimate business settlement for {}",
                    plan.snapshot.business
                ),
                postings: postings.to_vec(),
                authorization: None,
            },
        )?)
    };
    let information = match (
        plan.economics.attention,
        resolve_accounting_holder(plan.snapshot.owner),
    ) {
        (AttentionClass::Notable, Some(holder)) => Some(validate_record_information(
            state,
            InformationDraft {
                holder,
                source_kind: InformationSourceKind::Accountant,
                topic: crate::intelligence::InformationTopic::FinancialPerformance,
                source_entity: None,
                subject: EntityRef::Business(plan.snapshot.business),
                observed_at: plan.snapshot.occurred_at,
                reliability: Reliability::DirectAccess,
                specificity: Specificity::Precise,
                summary: format!(
                    "Business cycle reported gross {}, operating cost {}, net cash {}, and {}.{}",
                    crate::finance::helpers::format_money_cents(
                        plan.economics.gross_revenue.cents()
                    ),
                    crate::finance::helpers::format_money_cents(
                        plan.economics.operating_cost.cents()
                    ),
                    crate::finance::helpers::format_money_cents(plan.economics.net_cash.cents()),
                    crate::finance::helpers::describe_gross_variance(
                        plan.economics.variance_basis_points
                    ),
                    if plan.snapshot.suspends_after_settlement {
                        " Repeated losses have suspended operations pending a manual resumption."
                            .to_owned()
                    } else {
                        String::new()
                    }
                ),
            },
        )?),
        (AttentionClass::Routine, _) | (AttentionClass::Notable, None) => None,
        (AttentionClass::Exception | AttentionClass::Crisis, _) => {
            unreachable!("business cycles only produce routine or notable attention")
        }
    };
    Ok(ValidatedBusinessCycle {
        plan,
        ledger,
        information,
    })
}

pub(crate) fn find_due_businesses(state: &AppState) -> Vec<BusinessId> {
    state.economy.due_at_or_before(state.now())
}

pub(crate) fn resolve_business_gross_potential(
    registry: &Registry,
    state: &AppState,
    business: BusinessId,
) -> Result<Money, BusinessEconomyError> {
    let business_record = validate_business(state, business)?;
    let neighborhood = state
        .world
        .get_neighborhood(business_record.neighborhood())
        .ok_or(BusinessEconomyError::MissingBusinessNeighborhood(business))?;
    resolve_gross_before_variance(
        business,
        registry.get_business(business_record.kind()).economics(),
        neighborhood.profile(),
    )
}

/// Earning power after any active sabotage-disruption horizon — the same degraded gross a
/// cycle settles on. Laundering plausibility reads this so a degraded front can only hide
/// volume its visibly reduced books could still plausibly explain.
pub(crate) fn resolve_business_current_gross(
    registry: &Registry,
    state: &AppState,
    business: BusinessId,
) -> Result<Money, BusinessEconomyError> {
    let normal = resolve_business_gross_potential(registry, state, business)?;
    let disrupted = state
        .economy
        .get_business_economy(business)
        .ok_or(BusinessEconomyError::MissingBusinessEconomy(business))?
        .is_disrupted(state.now());
    if disrupted {
        return resolve_disrupted_gross(
            business,
            normal,
            registry.business_disruption().gross_basis_points(),
        );
    }
    Ok(normal)
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

fn resolve_accounting_holder(owner: BusinessOwner) -> Option<KnowledgeHolder> {
    match owner {
        BusinessOwner::Independent => None,
        BusinessOwner::Organization(id) => Some(KnowledgeHolder::Organization(id)),
        BusinessOwner::Character(id) => Some(KnowledgeHolder::Character(id)),
    }
}

fn resolve_gross_before_variance(
    business: BusinessId,
    economics: &BusinessEconomicsDefinition,
    profile: NeighborhoodProfile,
) -> Result<Money, BusinessEconomyError> {
    let wealth = weighted_rating(
        business,
        economics.wealth_revenue_per_point(),
        profile.economy.wealth.value(),
    )?;
    let commerce = weighted_rating(
        business,
        economics.commerce_revenue_per_point(),
        profile.economy.commercial_activity.value(),
    )?;
    economics
        .base_gross()
        .checked_add(wealth)
        .and_then(|gross| gross.checked_add(commerce))
        .ok_or(BusinessEconomyError::ArithmeticOverflow(business))
}

/// One owner for the authored business-cycle arithmetic used by both live settlement and
/// persistence re-derivation. `disrupted` is supplied explicitly so historical validation uses
/// the cycle's frozen sabotage fact rather than the economy's mutable current disruption horizon.
fn resolve_cycle_financials(
    business: BusinessId,
    economics: &BusinessEconomicsDefinition,
    profile: NeighborhoodProfile,
    disruption_gross_basis_points: u32,
    disrupted: bool,
    variance_basis_points: i16,
) -> Result<(Money, Money, Money), BusinessEconomyError> {
    let variance_limit = economics.gross_variance_basis_points();
    if i32::from(variance_basis_points).unsigned_abs() > u32::from(variance_limit) {
        return Err(BusinessEconomyError::VarianceOutOfRange {
            basis_points: variance_basis_points,
            limit: variance_limit,
        });
    }
    let normal_gross = resolve_gross_before_variance(business, economics, profile)?;
    let gross_before_variance = if disrupted {
        resolve_disrupted_gross(business, normal_gross, disruption_gross_basis_points)?
    } else {
        normal_gross
    };
    let gross_revenue =
        resolve_basis_point_variance(business, gross_before_variance, variance_basis_points)?;
    let police_cost = weighted_rating(
        business,
        economics.police_cost_per_point(),
        profile.institutions.police_presence.value(),
    )?;
    let operating_cost = economics
        .base_operating_cost()
        .checked_add(police_cost)
        .ok_or(BusinessEconomyError::ArithmeticOverflow(business))?;
    let net_cash = gross_revenue
        .checked_sub(operating_cost)
        .ok_or(BusinessEconomyError::ArithmeticOverflow(business))?;
    Ok((gross_revenue, operating_cost, net_cash))
}

/// Re-derives persisted business-cycle financials from immutable business/district authorship,
/// the live registry, and the cycle's frozen historical disruption/variance inputs.
pub(crate) fn resolve_historical_business_cycle_financials(
    registry: &Registry,
    state: &AppState,
    cycle: &super::BusinessCycleRecord,
) -> Result<(Money, Money, Money), BusinessEconomyError> {
    let business = validate_business(state, cycle.business())?;
    let neighborhood = state
        .world
        .get_neighborhood(business.neighborhood())
        .ok_or(BusinessEconomyError::MissingBusinessNeighborhood(
            cycle.business(),
        ))?;
    resolve_cycle_financials(
        cycle.business(),
        registry.get_business(business.kind()).economics(),
        neighborhood.profile(),
        registry.business_disruption().gross_basis_points(),
        cycle.disrupted(),
        cycle.variance_basis_points(),
    )
}

/// Authored sabotage damage: disrupted cycles earn the authored fraction of normal gross,
/// rounded with the crate's shared symmetric convention.
fn resolve_disrupted_gross(
    business: BusinessId,
    normal_gross: Money,
    gross_basis_points: u32,
) -> Result<Money, BusinessEconomyError> {
    crate::finance::helpers::apply_basis_point_multiplier(normal_gross, gross_basis_points)
        .ok_or(BusinessEconomyError::ArithmeticOverflow(business))
}

fn weighted_rating(
    business: BusinessId,
    per_point: Money,
    rating: u8,
) -> Result<Money, BusinessEconomyError> {
    crate::finance::helpers::weighted_rating(per_point, rating)
        .ok_or(BusinessEconomyError::ArithmeticOverflow(business))
}

fn resolve_basis_point_variance(
    business: BusinessId,
    amount: Money,
    basis_points: i16,
) -> Result<Money, BusinessEconomyError> {
    crate::finance::helpers::resolve_basis_point_variance(amount, basis_points)
        .ok_or(BusinessEconomyError::ArithmeticOverflow(business))
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
        .unwrap_or(SimTime::from_minutes(u64::MAX))
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

/// An owner's draw on a business it owns: legitimate till cash becomes spendable
/// organization wealth.
///
/// Settlement credits the business's own operating account, so without a canonical
/// withdrawal legitimate profits (and absorbed laundering fees) would strand in
/// `Business`-owned accounts no organization spend path can reach. The sweep is that
/// path: it moves till cash into an organization-owned accounted-funds account through
/// the balanced ledger, gated on real till liquidity. It is a withdrawal, not a
/// conversion — dirty street cash still cannot buy legitimacy except through laundering.
#[derive(Clone, Debug)]
pub struct BusinessProfitSweepDraft {
    pub organization: OrganizationId,
    pub business: BusinessId,
    pub destination: FinancialAccountId,
    pub amount: Money,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum BusinessProfitSweepError {
    #[error("profit sweep amount must be positive")]
    NonPositiveAmount,
    #[error("organization {0} does not exist")]
    MissingOrganization(OrganizationId),
    #[error("business {0} does not exist")]
    MissingBusiness(BusinessId),
    #[error("business {0} is not owned by the requesting organization")]
    ForeignBusiness(BusinessId),
    #[error("business {0} has no operating economy to sweep till cash from")]
    MissingBusinessEconomy(BusinessId),
    #[error(
        "business {business} till account {account} is missing or not legitimate operating funds"
    )]
    CorruptTillAccount {
        business: BusinessId,
        account: FinancialAccountId,
    },
    #[error("destination account {0} does not exist")]
    MissingDestinationAccount(FinancialAccountId),
    #[error("destination account {account} is not owned by organization {organization}")]
    DestinationOwnerMismatch {
        account: FinancialAccountId,
        organization: OrganizationId,
    },
    #[error(
        "destination account {0} must hold accounted funds: till cash enters the books as legitimate wealth"
    )]
    InvalidDestinationKind(FinancialAccountId),
    #[error(
        "business {business} till holds {available_cents} cents and cannot sweep {requested_cents}"
    )]
    InsufficientTillCash {
        business: BusinessId,
        available_cents: i64,
        requested_cents: i64,
    },
    #[error(
        "business {business} retained-capital basis belongs to ownership version {basis_version}, but current ownership is version {current_version}"
    )]
    StaleCapitalBasis {
        business: BusinessId,
        basis_version: u32,
        current_version: u32,
    },
    #[error(
        "business {business} has only {available_cents} cents of earned surplus above retained capital and cannot sweep {requested_cents}"
    )]
    InsufficientEarnedSurplus {
        business: BusinessId,
        available_cents: i64,
        requested_cents: i64,
    },
    #[error("business {business} ownership changed after sweep validation")]
    StaleBusiness {
        business: BusinessId,
        expected: u32,
        found: u32,
    },
    #[error("business {business} economy changed after sweep validation")]
    StaleEconomy {
        business: BusinessId,
        expected: u32,
        found: u32,
    },
    #[error(transparent)]
    Finance(#[from] FinanceError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
    #[error(transparent)]
    VersionCapacity(#[from] VersionCapacityError),
}

pub struct ValidatedBusinessProfitSweep {
    transaction: ValidatedLedgerTransaction,
    business: BusinessId,
    organization: OrganizationId,
    expected_business_version: u32,
    expected_economy_version: u32,
}

impl std::fmt::Debug for ValidatedBusinessProfitSweep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidatedBusinessProfitSweep")
            .field("business", &self.business)
            .field("organization", &self.organization)
            .field("expected_business_version", &self.expected_business_version)
            .field("expected_economy_version", &self.expected_economy_version)
            .finish_non_exhaustive()
    }
}

impl ValidatedBusinessProfitSweep {
    pub fn commit(self, state: &mut AppState) -> Result<(), BusinessProfitSweepError> {
        let business_record = state
            .world
            .get_business(self.business)
            .ok_or(BusinessProfitSweepError::MissingBusiness(self.business))?;
        if business_record.version() != self.expected_business_version {
            return Err(BusinessProfitSweepError::StaleBusiness {
                business: self.business,
                expected: self.expected_business_version,
                found: business_record.version(),
            });
        }
        if business_record.owner() != crate::world::BusinessOwner::Organization(self.organization) {
            return Err(BusinessProfitSweepError::ForeignBusiness(self.business));
        }
        // The till account is resolved from live economy state, not carried in the token:
        // a cycle settling between validation and commit must not redirect the withdrawal
        // into a stale account. The ledger leg below re-pins both account versions, so a
        // concurrent settlement rejects as a stale account rather than double-spending.
        let economy = state.economy.get_business_economy(self.business).ok_or(
            BusinessProfitSweepError::MissingBusinessEconomy(self.business),
        )?;
        if economy.version() != self.expected_economy_version {
            return Err(BusinessProfitSweepError::StaleEconomy {
                business: self.business,
                expected: self.expected_economy_version,
                found: economy.version(),
            });
        }
        self.transaction.commit(state)?;
        Ok(())
    }
}

pub fn validate_sweep_business_profits(
    state: &AppState,
    draft: BusinessProfitSweepDraft,
) -> Result<ValidatedBusinessProfitSweep, BusinessProfitSweepError> {
    if draft.amount <= Money::ZERO {
        return Err(BusinessProfitSweepError::NonPositiveAmount);
    }
    let context = resolve_business_profit_sweep_context(state, &draft)?;
    validate_business_profit_sweep_destination(state, &draft)?;
    validate_business_profit_sweep_capacity(
        &draft,
        context.operating_balance,
        context.capital_floor,
    )?;
    let transaction = validate_record_business_transaction(
        state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: format!(
                "Owner withdrawal of {} from {} till",
                format_money_cents(draft.amount.cents()),
                context.business_name,
            ),
            postings: vec![
                LedgerPosting {
                    account: context.operating_account,
                    amount: draft
                        .amount
                        .checked_neg()
                        .expect("positive sweep amount must negate"),
                },
                LedgerPosting {
                    account: draft.destination,
                    amount: draft.amount,
                },
            ],
            authorization: None,
        },
    )?;
    Ok(ValidatedBusinessProfitSweep {
        transaction,
        business: draft.business,
        organization: draft.organization,
        expected_business_version: context.business_version,
        expected_economy_version: context.economy_version,
    })
}

struct BusinessProfitSweepContext {
    operating_account: FinancialAccountId,
    operating_balance: Money,
    capital_floor: Money,
    business_name: String,
    business_version: u32,
    economy_version: u32,
}

fn resolve_business_profit_sweep_context(
    state: &AppState,
    draft: &BusinessProfitSweepDraft,
) -> Result<BusinessProfitSweepContext, BusinessProfitSweepError> {
    if state.world.get_organization(draft.organization).is_none() {
        return Err(BusinessProfitSweepError::MissingOrganization(
            draft.organization,
        ));
    }
    let business_record = state
        .world
        .get_business(draft.business)
        .ok_or(BusinessProfitSweepError::MissingBusiness(draft.business))?;
    if business_record.owner() != crate::world::BusinessOwner::Organization(draft.organization) {
        return Err(BusinessProfitSweepError::ForeignBusiness(draft.business));
    }
    let economy = state.economy.get_business_economy(draft.business).ok_or(
        BusinessProfitSweepError::MissingBusinessEconomy(draft.business),
    )?;
    // A suspended till still holds its cash: suspension stops future cycles, it does not
    // confiscate the balance. Withdrawing from it is ordinary closure of the books.
    let operating = state
        .finance
        .get_account(economy.operating_account())
        .ok_or(BusinessProfitSweepError::CorruptTillAccount {
            business: draft.business,
            account: economy.operating_account(),
        })?;
    if operating.owner() != FinancialOwner::Business(draft.business)
        || operating.kind() != AccountKind::LegitimateOperating
    {
        return Err(BusinessProfitSweepError::CorruptTillAccount {
            business: draft.business,
            account: economy.operating_account(),
        });
    }
    if economy.capital_floor_business_version() != business_record.version() {
        return Err(BusinessProfitSweepError::StaleCapitalBasis {
            business: draft.business,
            basis_version: economy.capital_floor_business_version(),
            current_version: business_record.version(),
        });
    }
    Ok(BusinessProfitSweepContext {
        operating_account: economy.operating_account(),
        operating_balance: operating.spendable_balance(),
        capital_floor: economy.operating_capital_floor(),
        business_name: business_record.name().to_owned(),
        business_version: business_record.version(),
        economy_version: economy.version(),
    })
}

fn validate_business_profit_sweep_destination(
    state: &AppState,
    draft: &BusinessProfitSweepDraft,
) -> Result<(), BusinessProfitSweepError> {
    let destination = state.finance.get_account(draft.destination).ok_or(
        BusinessProfitSweepError::MissingDestinationAccount(draft.destination),
    )?;
    if destination.owner() != FinancialOwner::Organization(draft.organization) {
        return Err(BusinessProfitSweepError::DestinationOwnerMismatch {
            account: draft.destination,
            organization: draft.organization,
        });
    }
    if destination.kind() != AccountKind::AccountedFunds {
        return Err(BusinessProfitSweepError::InvalidDestinationKind(
            draft.destination,
        ));
    }
    Ok(())
}

fn validate_business_profit_sweep_capacity(
    draft: &BusinessProfitSweepDraft,
    operating_balance: Money,
    capital_floor: Money,
) -> Result<(), BusinessProfitSweepError> {
    // The sweep spends real till liquidity: unlike cycle settlement (which books an
    // obligation when costs exceed cash), an owner cannot withdraw cash the till never held.
    if operating_balance < draft.amount {
        return Err(BusinessProfitSweepError::InsufficientTillCash {
            business: draft.business,
            available_cents: operating_balance.cents(),
            requested_cents: draft.amount.cents(),
        });
    }
    let available_surplus = operating_balance
        .checked_sub(capital_floor)
        .ok_or(BusinessProfitSweepError::InsufficientEarnedSurplus {
            business: draft.business,
            available_cents: 0,
            requested_cents: draft.amount.cents(),
        })?
        .max(Money::ZERO);
    if available_surplus < draft.amount {
        return Err(BusinessProfitSweepError::InsufficientEarnedSurplus {
            business: draft.business,
            available_cents: available_surplus.cents(),
            requested_cents: draft.amount.cents(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
