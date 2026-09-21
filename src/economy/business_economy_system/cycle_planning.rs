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
use crate::core::version::ensure_version_can_advance_by;
use crate::economy::{BusinessCycleRecord, BusinessOperatingStatus};
use crate::enterprises::{EnterpriseLocation, EnterpriseStatus};
use crate::finance::finance_system::{
    ValidatedLedgerTransaction, validate_record_business_transaction,
};
use crate::finance::{LedgerTransactionDraft, Money};
use crate::intelligence::intelligence_system::{ValidatedInformation, validate_record_information};
use crate::intelligence::{
    InformationDraft, InformationSourceKind, KnowledgeHolder, Reliability, Specificity,
};
use crate::registry::{BusinessEconomicsDefinition, Registry};
use crate::world::{BusinessOwner, NeighborhoodProfile};

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
    /// Whether this settlement reaches the authored consecutive-loss threshold. At that point
    /// active enterprise infrastructure may require the otherwise-closing front to remain open.
    loss_threshold_reached: bool,
    /// Deterministic active enterprise dependency observed when the loss threshold was reached.
    /// This is pinned because enterprise lifecycle changes independently from business versions.
    blocking_enterprise: Option<EnterpriseId>,
}

impl BusinessCycleSnapshot {
    fn suspends_after_settlement(&self) -> bool {
        self.loss_threshold_reached && self.blocking_enterprise.is_none()
    }
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
    // A losing settlement that reaches the authored consecutive-loss threshold suspends the
    // economy unless an active racket currently depends on this business as live infrastructure.
    // In that case the front stays open and keeps realizing its legitimate losses until the
    // racket is suspended or retired; silently closing it would invalidate active enterprise
    // requirements across domain ownership boundaries.
    let loss_threshold_reached = net_cash < Money::ZERO
        && trailing_losing_cycles + 1 >= u32::from(economics.losing_cycles_before_suspension());
    let blocking_enterprise = loss_threshold_reached
        .then(|| active_enterprise_dependency(state, business))
        .flatten();
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
            loss_threshold_reached,
            blocking_enterprise,
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
        .min()
}

fn validate_business_cycle_enterprise_dependency(
    state: &AppState,
    snapshot: &BusinessCycleSnapshot,
) -> Result<(), BusinessEconomyError> {
    if !snapshot.loss_threshold_reached {
        return Ok(());
    }
    let found = active_enterprise_dependency(state, snapshot.business);
    if found != snapshot.blocking_enterprise {
        return Err(BusinessEconomyError::StaleEnterpriseDependency {
            business: snapshot.business,
            expected: snapshot.blocking_enterprise,
            found,
        });
    }
    Ok(())
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
            1 + u32::from(self.plan.snapshot.suspends_after_settlement()),
            "business economy",
        )?;
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
        );
        if self.plan.snapshot.suspends_after_settlement() {
            // Domain-owner consequence for chronic losses: suspend instead of scheduling
            // another identical loss. Any later restart still uses the canonical resume token;
            // non-player books may exercise that token through daily autonomous maintenance.
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
        1 + u32::from(plan.snapshot.suspends_after_settlement()),
        "business economy",
    )?;
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
                    if plan.snapshot.suspends_after_settlement() {
                        " Repeated losses have suspended operations until the business is resumed."
                            .to_owned()
                    } else if let Some(enterprise) = plan.snapshot.blocking_enterprise {
                        format!(
                            " Repeated losses would normally suspend operations, but active enterprise {enterprise} requires this business to remain open."
                        )
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
