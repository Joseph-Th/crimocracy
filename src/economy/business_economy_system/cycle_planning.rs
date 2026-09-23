//! Deterministic legitimate-business cycle planning, validation, and atomic settlement.
//!
//! Establishment, lifecycle, laundering capacity, and disruption stay in the parent economy owner.
//! This child owns the complete decide -> validate -> commit cycle transaction behind that facade.

use super::{BusinessEconomyError, validate_accounts, validate_business};
use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{BusinessCycleId, BusinessId, EnterpriseId, FinancialAccountId, IdKind};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::core::version::ensure_version_can_advance;
use crate::economy::{BusinessCycleRecord, BusinessOperatingStatus};
use crate::enterprises::enterprise_execution::{
    EnterpriseError, ValidatedEnterpriseStatusChange, validate_suspend_enterprise,
};
use crate::enterprises::{EnterpriseLocation, EnterpriseStatus};
use crate::finance::finance_system::{
    ValidatedLedgerTransaction, validate_record_business_transaction,
};
use crate::finance::{LedgerTransactionDraft, Money};
use crate::intelligence::intelligence_system::{
    ValidatedInformation, validate_record_system_information,
};
use crate::intelligence::{
    InformationDraft, InformationSourceKind, KnowledgeHolder, Reliability, Specificity,
};
use crate::registry::{BusinessEconomicsDefinition, Registry};
use crate::world::{BusinessOwner, NeighborhoodProfile};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
struct BusinessCycleSnapshot {
    business: BusinessId,
    expected_business_version: u32,
    owner: BusinessOwner,
    expected_economy_version: u32,
    occurred_at: SimTime,
    /// `None` means this cycle settled successfully but no further recurrence is representable:
    /// either the authored cadence lies beyond the finite clock or this settlement consumed the
    /// final economy version. The economy remains operational but unscheduled.
    next_cycle_at: Option<SimTime>,
    /// Whether this settlement reaches the authored consecutive-loss threshold.
    loss_threshold_reached: bool,
    /// Exact active rackets that depend on this business when the loss threshold is reached.
    /// They are suspended through their canonical lifecycle before the front closes. Pinning the
    /// complete set prevents a held cycle plan from missing a newly established dependency.
    dependent_enterprises: BTreeSet<EnterpriseId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct BusinessCycleEconomics {
    pub(super) gross_revenue: Money,
    pub(super) operating_cost: Money,
    pub(super) net_cash: Money,
    pub(super) variance_basis_points: i16,
    pub(super) disrupted: bool,
    pub(super) attention: AttentionClass,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BusinessCycleAccounts {
    operating_account: FinancialAccountId,
    settlement_account: FinancialAccountId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BusinessCyclePlan {
    snapshot: BusinessCycleSnapshot,
    pub(super) economics: BusinessCycleEconomics,
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
    // A losing settlement that reaches the authored consecutive-loss threshold closes the
    // legitimate business. Active rackets using the front are dependencies of that infrastructure,
    // not vetoes over its economic failure, so validation below suspends those rackets atomically
    // before the business economy closes.
    let loss_threshold_reached = net_cash < Money::ZERO
        && trailing_losing_cycles + 1 >= u32::from(economics.losing_cycles_before_suspension());
    let dependent_enterprises = if loss_threshold_reached {
        active_enterprise_dependencies(state, business)
    } else {
        BTreeSet::new()
    };
    // Settling work that is due now must not fail only because its *next* recurrence is
    // unrepresentable. `None` is the existing exhausted-recurrence shape for both finite clock
    // and finite version rails. A non-suspending cycle from MAX-1 consumes the final economy
    // version, so scheduling another settlement would manufacture a future tick that can never
    // commit.
    let next_cycle_at = (economy.version() < u32::MAX - 1)
        .then(|| state.now().checked_add(economics.cycle()))
        .flatten();
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
            loss_threshold_reached,
            dependent_enterprises,
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

pub(super) fn active_enterprise_dependency(
    state: &AppState,
    business: BusinessId,
) -> Option<EnterpriseId> {
    active_enterprise_dependencies(state, business)
        .into_iter()
        .next()
}

fn active_enterprise_dependencies(
    state: &AppState,
    business: BusinessId,
) -> BTreeSet<EnterpriseId> {
    state
        .enterprises()
        .enterprises_supported_by_business(business)
        .chain(
            state
                .enterprises()
                .enterprises_at(EnterpriseLocation::Business(business)),
        )
        .filter(|enterprise| enterprise.status() == EnterpriseStatus::Active)
        .map(|enterprise| enterprise.id())
        .collect()
}

fn validate_business_cycle_enterprise_dependency(
    state: &AppState,
    snapshot: &BusinessCycleSnapshot,
) -> Result<(), BusinessEconomyError> {
    if !snapshot.loss_threshold_reached {
        return Ok(());
    }
    let found = active_enterprise_dependencies(state, snapshot.business);
    if found != snapshot.dependent_enterprises {
        return Err(BusinessEconomyError::StaleEnterpriseDependency {
            business: snapshot.business,
            expected: snapshot.dependent_enterprises.clone(),
            found,
        });
    }
    Ok(())
}

pub struct ValidatedBusinessCycle {
    plan: BusinessCyclePlan,
    ledger: Option<ValidatedLedgerTransaction>,
    information: Option<ValidatedInformation>,
    enterprise_suspensions: Vec<ValidatedEnterpriseStatusChange>,
    suspend_business_after_settlement: bool,
}

impl ValidatedBusinessCycle {
    pub fn commit(self, state: &mut AppState) -> Result<BusinessCycleId, BusinessEconomyError> {
        self.ensure_current(state)?;
        state.ids.reserve_many(&self.id_budget())?;
        Ok(self.commit_preflighted(state))
    }

    pub(crate) fn id_budget(&self) -> Vec<(IdKind, u32)> {
        let mut budget = Vec::new();
        if let Some(ledger) = &self.ledger {
            budget.extend(ledger.id_budget());
        }
        if self.information.is_some() {
            budget.push((IdKind::Information, 1));
        }
        budget.push((IdKind::BusinessCycle, 1));
        budget
    }

    pub(crate) fn ensure_current(&self, state: &AppState) -> Result<(), BusinessEconomyError> {
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
        ensure_version_can_advance(economy.version(), "business economy")?;
        if economy.status() != BusinessOperatingStatus::Active {
            return Err(BusinessEconomyError::EconomyNotActive(
                self.plan.snapshot.business,
            ));
        }
        validate_business_cycle_enterprise_dependency(state, &self.plan.snapshot)?;
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
        if let Some(ledger) = &self.ledger {
            ledger.ensure_current(state)?;
        }
        for suspension in &self.enterprise_suspensions {
            suspension.ensure_suspension_current(state)?;
        }
        Ok(())
    }

    pub(crate) fn commit_preflighted(self, state: &mut AppState) -> BusinessCycleId {
        for suspension in self.enterprise_suspensions {
            suspension.commit_suspension_preflighted(state);
        }
        let transaction = self.ledger.map(|ledger| ledger.commit_preflighted(state));
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
                context: crate::economy::BusinessCycleContext {
                    business: self.plan.snapshot.business,
                    business_version: self.plan.snapshot.expected_business_version,
                    owner: self.plan.snapshot.owner,
                    occurred_at: self.plan.snapshot.occurred_at,
                },
                financials: crate::economy::BusinessCycleFinancials {
                    gross_revenue: self.plan.economics.gross_revenue,
                    operating_cost: self.plan.economics.operating_cost,
                    net_cash: self.plan.economics.net_cash,
                    variance_basis_points: self.plan.economics.variance_basis_points,
                    disrupted: self.plan.economics.disrupted,
                },
                artifacts: crate::economy::BusinessCycleArtifacts {
                    attention: self.plan.economics.attention,
                    transaction,
                    information,
                },
            },
            self.plan.snapshot.next_cycle_at,
            self.suspend_business_after_settlement,
        );
        cycle
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
    ensure_version_can_advance(economy.version(), "business economy")?;
    if economy.status() != BusinessOperatingStatus::Active {
        return Err(BusinessEconomyError::EconomyNotActive(
            plan.snapshot.business,
        ));
    }
    validate_business_cycle_enterprise_dependency(state, &plan.snapshot)?;
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
    let mut enterprise_suspensions = Vec::new();
    let mut suspend_business_after_settlement = plan.snapshot.loss_threshold_reached;
    if suspend_business_after_settlement {
        for enterprise in &plan.snapshot.dependent_enterprises {
            match validate_suspend_enterprise(state, *enterprise) {
                Ok(suspension) => enterprise_suspensions.push(suspension),
                // A terminal enterprise version cannot transition again. Preserve the finite
                // version rail by leaving both sides active rather than making routine business
                // settlement fail forever at this extreme campaign boundary.
                Err(EnterpriseError::VersionCapacity(_)) => {
                    enterprise_suspensions.clear();
                    suspend_business_after_settlement = false;
                    break;
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
    let information = match (
        plan.economics.attention,
        resolve_accounting_holder(plan.snapshot.owner),
    ) {
        (AttentionClass::Notable, Some(holder)) => Some(validate_record_system_information(
            state,
            InformationDraft {
                holder,
                source_kind: InformationSourceKind::Accounting,
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
                    if suspend_business_after_settlement
                        && !plan.snapshot.dependent_enterprises.is_empty()
                    {
                        format!(
                            " Repeated losses have suspended operations and {} dependent criminal enterprise{} until the front can be restored.",
                            plan.snapshot.dependent_enterprises.len(),
                            if plan.snapshot.dependent_enterprises.len() == 1 {
                                ""
                            } else {
                                "s"
                            }
                        )
                    } else if suspend_business_after_settlement {
                        " Repeated losses have suspended operations until the business is resumed."
                            .to_owned()
                    } else if !plan.snapshot.dependent_enterprises.is_empty() {
                        " Repeated losses reached the suspension threshold, but a terminal enterprise lifecycle version prevents a safe dependency shutdown.".to_owned()
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
        enterprise_suspensions,
        suspend_business_after_settlement,
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

fn resolve_accounting_holder(owner: BusinessOwner) -> Option<KnowledgeHolder> {
    match owner {
        BusinessOwner::Independent => None,
        BusinessOwner::Organization(id) => Some(KnowledgeHolder::Organization(id)),
        BusinessOwner::Character(id) => Some(KnowledgeHolder::Character(id)),
    }
}

pub(super) fn resolve_gross_before_variance(
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
pub(super) fn resolve_cycle_financials(
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
    cycle: &crate::economy::BusinessCycleRecord,
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
