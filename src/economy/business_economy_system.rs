//! Business operating lifecycle, deterministic cycle planning, and atomic ledger settlement.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{BusinessCycleId, BusinessId, FinancialAccountId, IdExhaustionError, IdKind};
use crate::core::state::AppState;
use crate::core::time::{SimDuration, SimTime};
use crate::economy::{
    BusinessCycleRecord, BusinessEconomyDraft, BusinessOperatingStatus,
    build_business_economy_record,
};
use crate::finance::finance_system::{
    FinanceError, ValidatedFinancialAccountOpenings, ValidatedLedgerTransaction,
    validate_record_transaction,
};
use crate::finance::{AccountKind, FinancialOwner, LedgerTransactionDraft, Money};
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
}

pub struct ValidatedBusinessEconomyEstablishment {
    draft: BusinessEconomyDraft,
    cycle_duration: SimDuration,
}

impl ValidatedBusinessEconomyEstablishment {
    pub fn commit(self, state: &mut AppState) -> Result<BusinessId, BusinessEconomyError> {
        validate_business(state, self.draft.business)?;
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
        let business = self.draft.business;
        let established_at = state.now();
        let next_cycle_at = established_at + self.cycle_duration;
        state.economy.insert(build_business_economy_record(
            self.draft,
            established_at,
            next_cycle_at,
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
    Ok(ValidatedBusinessEconomyEstablishment {
        draft,
        cycle_duration,
    })
}

/// Business-economy establishment validated against accounts that a composing operation has
/// planned but not yet opened. The token deliberately does not own the opening plan because the
/// same plan is consumed by the ledger transaction that capitalizes the new operating account.
pub(crate) struct ValidatedComposedBusinessEconomyEstablishment {
    draft: BusinessEconomyDraft,
    cycle_duration: SimDuration,
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
        let business = self.draft.business;
        let established_at = state.now();
        let next_cycle_at = established_at + self.cycle_duration;
        state.economy.insert(build_business_economy_record(
            self.draft,
            established_at,
            next_cycle_at,
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
    validate_business(state, draft.business)?;
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
    Ok(ValidatedComposedBusinessEconomyEstablishment {
        draft,
        cycle_duration,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BusinessCycleSnapshot {
    business: BusinessId,
    expected_business_version: u32,
    owner: BusinessOwner,
    expected_economy_version: u32,
    occurred_at: SimTime,
    next_cycle_at: SimTime,
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

impl BusinessCyclePlan {
    // Test-only drill-down: production consumers read the committed cycle records, not the
    // intermediate plan, so these accessors exist solely for focused assertions.
    #[cfg(test)]
    pub fn gross_revenue(&self) -> Money {
        self.economics.gross_revenue
    }
    #[cfg(test)]
    pub fn operating_cost(&self) -> Money {
        self.economics.operating_cost
    }
    #[cfg(test)]
    pub fn net_cash(&self) -> Money {
        self.economics.net_cash
    }
    #[cfg(test)]
    pub fn attention(&self) -> AttentionClass {
        self.economics.attention
    }
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
    let due_at = economy
        .next_cycle_at()
        .expect("active business economy must carry a scheduled next cycle");
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
    let variance_limit = economics.gross_variance_basis_points();
    if i32::from(variance_basis_points).unsigned_abs() > u32::from(variance_limit) {
        return Err(BusinessEconomyError::VarianceOutOfRange {
            basis_points: variance_basis_points,
            limit: variance_limit,
        });
    }
    let neighborhood = state
        .world
        .get_neighborhood(business_record.neighborhood())
        .ok_or(BusinessEconomyError::MissingBusinessNeighborhood(business))?;
    let profile = neighborhood.profile();
    let gross_before_variance = resolve_gross_before_variance(business, economics, profile)?;
    // Sabotage damage degrades earning power for the authored horizon; costs keep running.
    let gross_before_variance = if economy.is_disrupted(state.now()) {
        resolve_disrupted_gross(
            business,
            gross_before_variance,
            registry.business_disruption().gross_basis_points(),
        )?
    } else {
        gross_before_variance
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
    // A losing cycle is always accountant-worthy: chronic silent losses are exactly what the
    // owner must see before the authored suspension threshold stops the bleeding.
    let attention = if net_cash < Money::ZERO
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
            next_cycle_at: state.now() + economics.cycle(),
            suspends_after_settlement,
        },
        economics: BusinessCycleEconomics {
            gross_revenue,
            operating_cost,
            net_cash,
            variance_basis_points,
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
    let newest_first: Vec<_> = state
        .economy
        .cycles_for(business)
        .rev()
        .take(usize::from(limit))
        .collect();
    crate::finance::helpers::count_trailing_losing_cycles(
        &newest_first,
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
        if economy.status() != BusinessOperatingStatus::Active {
            return Err(BusinessEconomyError::EconomyNotActive(
                self.plan.snapshot.business,
            ));
        }
        if state.now() != self.plan.snapshot.occurred_at {
            return Err(BusinessEconomyError::StaleCycleTime {
                expected: self.plan.snapshot.occurred_at,
                found: state.now(),
            });
        }
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
    if economy.status() != BusinessOperatingStatus::Active {
        return Err(BusinessEconomyError::EconomyNotActive(
            plan.snapshot.business,
        ));
    }
    if state.now() != plan.snapshot.occurred_at {
        return Err(BusinessEconomyError::StaleCycleTime {
            expected: plan.snapshot.occurred_at,
            found: state.now(),
        });
    }
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
        Some(validate_record_transaction(
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
        let next_cycle_at = self.cycle_duration.map(|duration| state.now() + duration);
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
    Ok(ValidatedBusinessEconomyStatusChange {
        business,
        expected_version: economy.version(),
        change: BusinessEconomyStatusChange::Resume,
        cycle_duration: Some(cycle_duration),
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

/// Authored sabotage damage: disrupted cycles earn the authored fraction of normal gross,
/// rounded with the crate's shared symmetric convention.
fn resolve_disrupted_gross(
    business: BusinessId,
    normal_gross: Money,
    gross_basis_points: u32,
) -> Result<Money, BusinessEconomyError> {
    crate::finance::helpers::resolve_basis_point_share(normal_gross, gross_basis_points)
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

/// Canonical sabotage-damage mutation: extends the target's disruption horizon through an
/// validated-then-committed plan so repeated attacks push the horizon later and staleness
/// is re-checked at commit.
pub struct ValidatedBusinessDisruption {
    business: BusinessId,
    expected_economy_version: u32,
    disrupted_through: SimTime,
    /// The instant the horizon was measured from. Commit rejects a token held across a
    /// clock advance, mirroring the cycle path's time-staleness convention.
    expected_now: SimTime,
}

impl ValidatedBusinessDisruption {
    pub(crate) fn ensure_current(&self, state: &AppState) -> Result<(), BusinessEconomyError> {
        if state.now() != self.expected_now {
            return Err(BusinessEconomyError::StaleDisruptionTime {
                business: self.business,
                expected: self.expected_now,
                found: state.now(),
            });
        }
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
        Ok(())
    }

    pub fn commit(self, state: &mut AppState) -> Result<(), BusinessEconomyError> {
        self.ensure_current(state)?;
        state
            .economy
            .apply_disruption(self.business, self.disrupted_through);
        Ok(())
    }
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
    let disrupted_through = state.now() + registry.business_disruption().duration();
    Ok(ValidatedBusinessDisruption {
        business,
        expected_economy_version: economy.version(),
        disrupted_through,
        expected_now: state.now(),
    })
}

#[cfg(test)]
mod tests;
