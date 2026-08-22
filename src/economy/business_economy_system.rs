//! Business operating lifecycle, deterministic cycle planning, and atomic ledger settlement.

use crate::core::attention::AttentionClass;
use crate::core::entity::EntityRef;
use crate::core::id::{BusinessCycleId, BusinessId, FinancialAccountId, IdExhaustionError, IdKind};
use crate::core::state::AppState;
use crate::core::time::{SimDuration, SimTime};
use crate::economy::{
    build_business_economy_record, BusinessCycleRecord, BusinessEconomyDraft,
    BusinessOperatingStatus,
};
use crate::finance::finance_system::{
    validate_record_transaction, FinanceError, ValidatedLedgerTransaction,
};
use crate::finance::{AccountKind, FinancialOwner, LedgerTransactionDraft, Money};
use crate::intelligence::intelligence_system::{
    validate_record_information, IntelligenceError, ValidatedInformation,
};
use crate::intelligence::{
    InformationDraft, InformationSourceKind, KnowledgeHolder, Reliability, Specificity,
};
use crate::registry::{BusinessEconomicsDefinition, Registry};
use crate::world::{BusinessOwner, Lifecycle, NeighborhoodProfile};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum BusinessEconomyError {
    #[error("business {0} does not exist")]
    MissingBusiness(BusinessId),
    #[error("business {0} has no operating economy record")]
    MissingBusinessEconomy(BusinessId),
    #[error("business {0} references a missing neighborhood")]
    MissingBusinessNeighborhood(BusinessId),
    #[error("business {0} is not active")]
    InactiveBusiness(BusinessId),
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
    #[error("business {business} economy changed after validation; expected version {expected}, found {found}")]
    StaleEconomy {
        business: BusinessId,
        expected: u32,
        found: u32,
    },
    #[error("business {business} ownership changed after cycle planning; expected version {expected}, found {found}")]
    StaleBusiness {
        business: BusinessId,
        expected: u32,
        found: u32,
    },
    #[error(
        "business cycle plan was resolved at {expected:?}, but simulation time is now {found:?}"
    )]
    StaleCycleTime { expected: SimTime, found: SimTime },
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct BusinessCycleSnapshot {
    business: BusinessId,
    expected_business_version: u32,
    owner: BusinessOwner,
    expected_economy_version: u32,
    occurred_at: SimTime,
    next_cycle_at: SimTime,
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
    pub fn gross_revenue(&self) -> Money {
        self.economics.gross_revenue
    }
    pub fn operating_cost(&self) -> Money {
        self.economics.operating_cost
    }
    pub fn net_cash(&self) -> Money {
        self.economics.net_cash
    }
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
        .ok_or(BusinessEconomyError::EconomyNotActive(business))?;
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
    if neighborhood.lifecycle() != Lifecycle::Active {
        return Err(BusinessEconomyError::InactiveBusiness(business));
    }
    let profile = neighborhood.profile();
    let gross_before_variance = resolve_gross_before_variance(business, economics, profile)?;
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
    let attention = if i32::from(variance_basis_points).unsigned_abs()
        >= u32::from(economics.notable_variance_basis_points())
    {
        AttentionClass::Notable
    } else {
        AttentionClass::Routine
    };
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
        let information = match self.information {
            Some(information) => Some(information.commit(state)?),
            None => None,
        };
        let cycle = state.ids.next_business_cycle()?;
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
        accounting_holder(plan.snapshot.owner),
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
                    "Business cycle reported gross {}, operating cost {}, net cash {}, and {}.",
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
        state
            .economy
            .set_status(self.business, status, next_cycle_at);
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
            return Err(BusinessEconomyError::EconomyNotActive(business))
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
    let economy = state
        .economy
        .get_business_economy(business)
        .ok_or(BusinessEconomyError::MissingBusinessEconomy(business))?;
    match economy.status() {
        BusinessOperatingStatus::Active => {
            return Err(BusinessEconomyError::EconomyNotSuspended(business))
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
    let cycle_duration = registry
        .get_business(business_record.kind())
        .economics()
        .cycle();
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
    if neighborhood.lifecycle() != Lifecycle::Active {
        return Err(BusinessEconomyError::InactiveBusiness(business));
    }
    resolve_gross_before_variance(
        business,
        registry.get_business(business_record.kind()).economics(),
        neighborhood.profile(),
    )
}

fn validate_business(
    state: &AppState,
    business: BusinessId,
) -> Result<&crate::world::BusinessRecord, BusinessEconomyError> {
    let business_record = state
        .world
        .get_business(business)
        .ok_or(BusinessEconomyError::MissingBusiness(business))?;
    if business_record.lifecycle() != Lifecycle::Active {
        return Err(BusinessEconomyError::InactiveBusiness(business));
    }
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
    if let Some(existing) = state.economy.get_by_settlement_account(settlement_account) {
        if Some(existing.business()) != current_business {
            return Err(BusinessEconomyError::SettlementAccountInUse {
                account: settlement_account,
                business: existing.business(),
            });
        }
    }
    Ok(())
}

fn accounting_holder(owner: BusinessOwner) -> Option<KnowledgeHolder> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::invariants::validate_invariants;
    use crate::core::persistence::{build_save, restore_save, SaveEnvelope};
    use crate::core::simulation::run_tick;
    use crate::economy::business_reporting::{
        resolve_business_financial_summary, resolve_organization_business_financial_summary,
    };
    use crate::economy::BusinessEconomyDraft;
    use crate::finance::finance_system::insert_account;
    use crate::finance::{FinancialAccountDraft, FinancialOwner};
    use crate::reports::organization_financial_report::validate_organization_financial_report;
    use crate::reports::ReportKind;
    use crate::world::world_system::{
        insert_business, insert_neighborhood, insert_organization,
        validate_transfer_business_ownership,
    };
    use crate::world::{
        BusinessDraft, BusinessFunction, BusinessKind, BusinessOwner, NeighborhoodDraft,
        NeighborhoodEconomyProfile, NeighborhoodInstitutionProfile, NeighborhoodProfile,
        OrganizationDraft, OrganizationKind, Rating,
    };
    use std::collections::BTreeSet;

    struct BusinessEconomyFixture {
        state: AppState,
        business: BusinessId,
        organization: crate::core::id::OrganizationId,
        operating: FinancialAccountId,
        settlement: FinancialAccountId,
    }

    fn rating(value: u8) -> Rating {
        Rating::try_new(value).expect("fixture rating must be valid")
    }

    fn make_business_economy_fixture() -> BusinessEconomyFixture {
        let registry = build_registry();
        let mut state = AppState::new(0xB051_1932);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Legitimate Holdings".to_owned(),
                kind: OrganizationKind::Commercial,
            },
        )
        .expect("organization fixture should validate");
        let neighborhood = insert_neighborhood(
            &mut state,
            NeighborhoodDraft {
                name: "Commercial Ward".to_owned(),
                profile: NeighborhoodProfile {
                    economy: NeighborhoodEconomyProfile {
                        wealth: rating(60),
                        commercial_activity: rating(70),
                        illicit_demand: rating(30),
                    },
                    institutions: NeighborhoodInstitutionProfile {
                        police_presence: rating(55),
                        political_influence: rating(50),
                        social_cohesion: rating(60),
                        visible_violence_tolerance: rating(20),
                    },
                },
            },
        )
        .expect("neighborhood fixture should validate");
        let business = insert_business(
            &registry,
            &mut state,
            BusinessDraft {
                name: "Market Street Grocer".to_owned(),
                kind: BusinessKind::Retail,
                functions: BTreeSet::from([
                    BusinessFunction::CashIntensive,
                    BusinessFunction::CustomerAccess,
                    BusinessFunction::MeetingSpace,
                ]),
                neighborhood,
                owner: BusinessOwner::Organization(organization),
            },
        )
        .expect("business fixture should validate");
        let operating = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Business(business),
                kind: AccountKind::LegitimateOperating,
            },
        )
        .expect("operating account should validate");
        let settlement = insert_account(
            &mut state,
            FinancialAccountDraft {
                owner: FinancialOwner::Business(business),
                kind: AccountKind::Settlement,
            },
        )
        .expect("settlement account should validate");
        BusinessEconomyFixture {
            state,
            business,
            organization,
            operating,
            settlement,
        }
    }

    fn establish_business_economy(registry: &Registry, fixture: &mut BusinessEconomyFixture) {
        validate_establish_business_economy(
            registry,
            &fixture.state,
            BusinessEconomyDraft {
                business: fixture.business,
                operating_account: fixture.operating,
                settlement_account: fixture.settlement,
            },
        )
        .expect("business economy fixture should validate")
        .commit(&mut fixture.state)
        .expect("business economy fixture should commit");
    }

    #[test]
    fn routine_business_cycle_records_causal_economics_and_balanced_settlement() {
        let registry = build_registry();
        let mut fixture = make_business_economy_fixture();
        establish_business_economy(&registry, &mut fixture);
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));

        let plan = decide_business_cycle(&registry, &fixture.state, fixture.business, 0)
            .expect("due business cycle should resolve");
        // Assert derived invariants rather than hard-coded cents so content tuning does not
        // spuriously break the contract: cost is authored base, net is gross-cost, attention
        // follows notable threshold, and settlement is the ledger mirror.
        let business_kind = fixture
            .state
            .world()
            .get_business(fixture.business)
            .expect("fixture business should exist")
            .kind();
        let economics = registry.get_business(business_kind).economics();
        let business = fixture
            .state
            .world()
            .get_business(fixture.business)
            .expect("fixture business should exist");
        let neighborhood = fixture
            .state
            .world()
            .get_neighborhood(business.neighborhood())
            .expect("business neighborhood should exist");
        let expected_police = crate::finance::helpers::weighted_rating(
            economics.police_cost_per_point(),
            neighborhood.profile().institutions.police_presence.value(),
        )
        .expect("police cost should not overflow");
        let expected_cost = economics
            .base_operating_cost()
            .checked_add(expected_police)
            .expect("business cost should not overflow");
        assert_eq!(plan.operating_cost(), expected_cost);
        assert_eq!(
            plan.net_cash(),
            plan.gross_revenue()
                .checked_sub(plan.operating_cost())
                .expect("net cash should be gross - cost")
        );
        assert_eq!(plan.attention(), AttentionClass::Routine);
        assert!(plan.gross_revenue().cents() >= economics.base_gross().cents());

        let cycle = validate_business_cycle_plan(&fixture.state, plan)
            .expect("business cycle plan should validate")
            .commit(&mut fixture.state)
            .expect("business cycle should commit");
        let cycle = fixture
            .state
            .economy()
            .get_cycle(cycle)
            .expect("business cycle should persist");
        assert!(cycle.transaction().is_some());
        assert!(cycle.information().is_none());
        let operating_balance = fixture
            .state
            .finance()
            .get_account(fixture.operating)
            .expect("operating account should exist")
            .balance();
        let settlement_balance = fixture
            .state
            .finance()
            .get_account(fixture.settlement)
            .expect("settlement account should exist")
            .balance();
        assert_eq!(operating_balance, cycle.net_cash());
        assert_eq!(
            settlement_balance,
            Money::from_cents(-operating_balance.cents())
        );
        validate_invariants(&fixture.state);
    }

    #[test]
    fn ownership_change_invalidates_prevalidated_business_cycle_atomically() {
        let registry = build_registry();
        let mut fixture = make_business_economy_fixture();
        establish_business_economy(&registry, &mut fixture);
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));
        let plan = decide_business_cycle(&registry, &fixture.state, fixture.business, 900)
            .expect("due business cycle should resolve");
        let validated = validate_business_cycle_plan(&fixture.state, plan)
            .expect("business cycle should validate before ownership changes");
        let successor = insert_organization(
            &registry,
            &mut fixture.state,
            OrganizationDraft {
                name: "Successor Holdings".to_owned(),
                kind: OrganizationKind::Commercial,
            },
        )
        .expect("successor organization should validate");
        validate_transfer_business_ownership(
            &fixture.state,
            fixture.business,
            BusinessOwner::Organization(successor),
        )
        .expect("business ownership change should validate")
        .commit(&mut fixture.state)
        .expect("business ownership change should commit");

        let error = validated
            .commit(&mut fixture.state)
            .expect_err("ownership change must invalidate a prevalidated cycle");
        assert_eq!(
            error,
            BusinessEconomyError::StaleBusiness {
                business: fixture.business,
                expected: 1,
                found: 2,
            }
        );
        assert_eq!(
            fixture.state.economy().cycles_for(fixture.business).count(),
            0
        );
        assert_eq!(
            fixture
                .state
                .finance()
                .get_account(fixture.operating)
                .expect("operating account should exist")
                .balance(),
            Money::ZERO
        );
        assert_eq!(
            fixture
                .state
                .finance()
                .get_account(fixture.settlement)
                .expect("settlement account should exist")
                .balance(),
            Money::ZERO
        );
        validate_invariants(&fixture.state);
    }

    #[test]
    fn transferred_business_cycles_remain_attributed_to_the_owner_at_commit() {
        let registry = build_registry();
        let mut fixture = make_business_economy_fixture();
        establish_business_economy(&registry, &mut fixture);
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));
        let first_cycle = decide_business_cycle(&registry, &fixture.state, fixture.business, 900)
            .expect("first due business cycle should resolve");
        let first_cycle = validate_business_cycle_plan(&fixture.state, first_cycle)
            .expect("first business cycle should validate")
            .commit(&mut fixture.state)
            .expect("first business cycle should commit");
        let first_cycle_record = fixture
            .state
            .economy()
            .get_cycle(first_cycle)
            .expect("first business cycle should persist");
        assert_eq!(
            first_cycle_record.owner(),
            BusinessOwner::Organization(fixture.organization)
        );
        assert_eq!(first_cycle_record.business_version(), 1);

        let successor = insert_organization(
            &registry,
            &mut fixture.state,
            OrganizationDraft {
                name: "Acquiring Company".to_owned(),
                kind: OrganizationKind::Commercial,
            },
        )
        .expect("acquiring organization should validate");
        validate_transfer_business_ownership(
            &fixture.state,
            fixture.business,
            BusinessOwner::Organization(successor),
        )
        .expect("same-minute ownership transfer should validate")
        .commit(&mut fixture.state)
        .expect("same-minute ownership transfer should commit");

        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));
        let second_cycle = decide_business_cycle(&registry, &fixture.state, fixture.business, 900)
            .expect("second due business cycle should resolve");
        let second_cycle = validate_business_cycle_plan(&fixture.state, second_cycle)
            .expect("second business cycle should validate")
            .commit(&mut fixture.state)
            .expect("second business cycle should commit");
        let second_cycle_record = fixture
            .state
            .economy()
            .get_cycle(second_cycle)
            .expect("second business cycle should persist");
        assert_eq!(
            second_cycle_record.owner(),
            BusinessOwner::Organization(successor)
        );
        assert_eq!(second_cycle_record.business_version(), 2);

        let original_summary = resolve_organization_business_financial_summary(
            &fixture.state,
            fixture.organization,
            SimTime::ZERO,
            fixture.state.now(),
        )
        .expect("original owner summary should preserve historical attribution");
        let successor_summary = resolve_organization_business_financial_summary(
            &fixture.state,
            successor,
            SimTime::ZERO,
            fixture.state.now(),
        )
        .expect("successor summary should include only post-transfer cycles");
        assert_eq!(original_summary.totals.business_count, 1);
        assert_eq!(original_summary.totals.cycle_count, 1);
        assert_eq!(original_summary.totals.notable_cycle_count, 1);
        assert_eq!(successor_summary.totals.business_count, 1);
        assert_eq!(successor_summary.totals.cycle_count, 1);
        assert_eq!(successor_summary.totals.notable_cycle_count, 1);

        let original_report = validate_organization_financial_report(
            &fixture.state,
            fixture.organization,
            SimTime::ZERO,
            fixture.state.now(),
        )
        .expect("original owner report should retain its notable historical cycle")
        .commit(&mut fixture.state)
        .expect("original owner report should commit");
        let successor_report = validate_organization_financial_report(
            &fixture.state,
            successor,
            SimTime::ZERO,
            fixture.state.now(),
        )
        .expect("successor report should include its notable post-transfer cycle")
        .commit(&mut fixture.state)
        .expect("successor report should commit");
        assert_eq!(
            fixture
                .state
                .reports()
                .get_report(original_report)
                .expect("original owner report should persist")
                .entries()
                .len(),
            2
        );
        assert_eq!(
            fixture
                .state
                .reports()
                .get_report(successor_report)
                .expect("successor report should persist")
                .entries()
                .len(),
            2
        );
        validate_invariants(&fixture.state);
    }

    #[test]
    fn establishment_and_resume_schedule_from_commit_time() {
        let registry = build_registry();
        let mut fixture = make_business_economy_fixture();
        let establishment = validate_establish_business_economy(
            &registry,
            &fixture.state,
            BusinessEconomyDraft {
                business: fixture.business,
                operating_account: fixture.operating,
                settlement_account: fixture.settlement,
            },
        )
        .expect("business economy should validate before delayed commit");
        fixture.state.advance_clock(SimDuration::from_minutes(60));
        establishment
            .commit(&mut fixture.state)
            .expect("delayed establishment should commit");
        let economy = fixture
            .state
            .economy()
            .get_business_economy(fixture.business)
            .expect("business economy should exist");
        assert_eq!(economy.established_at(), SimTime::from_minutes(60));
        assert_eq!(economy.next_cycle_at(), Some(SimTime::from_minutes(1_500)));

        validate_suspend_business_economy(&fixture.state, fixture.business)
            .expect("business economy should suspend")
            .commit(&mut fixture.state)
            .expect("business suspension should commit");
        let resume = validate_resume_business_economy(&registry, &fixture.state, fixture.business)
            .expect("business economy should validate for resume");
        fixture.state.advance_clock(SimDuration::from_minutes(30));
        resume
            .commit(&mut fixture.state)
            .expect("delayed business resume should commit");
        let economy = fixture
            .state
            .economy()
            .get_business_economy(fixture.business)
            .expect("business economy should still exist");
        assert_eq!(economy.next_cycle_at(), Some(SimTime::from_minutes(1_530)));
        validate_invariants(&fixture.state);
    }

    #[test]
    fn notable_owned_business_cycle_creates_accounting_information_for_owner() {
        let registry = build_registry();
        let mut fixture = make_business_economy_fixture();
        establish_business_economy(&registry, &mut fixture);
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));

        let plan = decide_business_cycle(&registry, &fixture.state, fixture.business, 900)
            .expect("material business variance should resolve");
        assert_eq!(plan.attention(), AttentionClass::Notable);
        let cycle = validate_business_cycle_plan(&fixture.state, plan)
            .expect("material business cycle should validate")
            .commit(&mut fixture.state)
            .expect("material business cycle should commit");
        let cycle = fixture
            .state
            .economy()
            .get_cycle(cycle)
            .expect("cycle should persist");
        let information = cycle
            .information()
            .expect("owned notable business cycle should create accounting information");
        let information = fixture
            .state
            .intelligence()
            .get_information(information)
            .expect("accounting information should persist");
        assert_eq!(
            information.holder(),
            KnowledgeHolder::Organization(fixture.organization)
        );
        assert_eq!(information.source_kind(), InformationSourceKind::Accountant);
        assert_eq!(information.subject(), EntityRef::Business(fixture.business));

        let business_summary = resolve_business_financial_summary(
            &fixture.state,
            fixture.business,
            SimTime::ZERO,
            fixture.state.now(),
        )
        .expect("business financial summary should resolve");
        let organization_summary = resolve_organization_business_financial_summary(
            &fixture.state,
            fixture.organization,
            SimTime::ZERO,
            fixture.state.now(),
        )
        .expect("organization business summary should resolve");
        assert_eq!(business_summary.totals, organization_summary.totals);
        assert_eq!(business_summary.totals.notable_cycle_count, 1);

        let report = validate_organization_financial_report(
            &fixture.state,
            fixture.organization,
            SimTime::ZERO,
            fixture.state.now(),
        )
        .expect("organization financial report should synthesize legitimate business history")
        .commit(&mut fixture.state)
        .expect("organization financial report should commit");
        let report = fixture
            .state
            .reports()
            .get_report(report)
            .expect("organization financial report should persist");
        assert_eq!(report.kind(), ReportKind::Financial);
        assert_eq!(report.entries().len(), 2);
        assert_eq!(report.entries()[0].attention, AttentionClass::Routine);
        assert_eq!(report.entries()[1].attention, AttentionClass::Notable);
        assert_eq!(report.entries()[1].sources.len(), 1);
        assert!(report.entries()[1]
            .entities
            .contains(&EntityRef::Business(fixture.business)));
        validate_invariants(&fixture.state);
    }

    #[test]
    fn business_economy_is_unique_and_suspension_removes_due_work() {
        let registry = build_registry();
        let mut fixture = make_business_economy_fixture();
        establish_business_economy(&registry, &mut fixture);

        let duplicate = match validate_establish_business_economy(
            &registry,
            &fixture.state,
            BusinessEconomyDraft {
                business: fixture.business,
                operating_account: fixture.operating,
                settlement_account: fixture.settlement,
            },
        ) {
            Ok(_) => panic!("one business must not have multiple operating economy records"),
            Err(error) => error,
        };
        assert_eq!(
            duplicate,
            BusinessEconomyError::ExistingBusinessEconomy(fixture.business)
        );

        validate_suspend_business_economy(&fixture.state, fixture.business)
            .expect("active business economy should suspend")
            .commit(&mut fixture.state)
            .expect("business suspension should commit");
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_440));
        assert!(find_due_businesses(&fixture.state).is_empty());
        validate_invariants(&fixture.state);
    }

    #[test]
    fn save_round_trip_preserves_business_schedule_and_deterministic_tick_resolution() {
        let registry = build_registry();
        let mut fixture = make_business_economy_fixture();
        establish_business_economy(&registry, &mut fixture);
        fixture
            .state
            .advance_clock(SimDuration::from_minutes(1_439));
        let successor = insert_organization(
            &registry,
            &mut fixture.state,
            OrganizationDraft {
                name: "Saved Successor Holdings".to_owned(),
                kind: OrganizationKind::Commercial,
            },
        )
        .expect("saved successor organization should validate");
        validate_transfer_business_ownership(
            &fixture.state,
            fixture.business,
            BusinessOwner::Organization(successor),
        )
        .expect("pre-save ownership transfer should validate")
        .commit(&mut fixture.state)
        .expect("pre-save ownership transfer should commit");

        let envelope = build_save(&registry, &fixture.state)
            .expect("business economy state should build a valid save");
        let bytes = bincode::serialize(&envelope).expect("save envelope should serialize");
        let decoded: SaveEnvelope =
            bincode::deserialize(&bytes).expect("save envelope should deserialize");
        let mut restored =
            restore_save(&registry, decoded).expect("business economy save should restore");

        let original = run_tick(&registry, &mut fixture.state);
        let continued = run_tick(&registry, &mut restored);
        assert_eq!(original, continued);
        assert_eq!(original.business_cycles.len(), 1);
        let original_cycle = fixture
            .state
            .economy()
            .get_cycle(original.business_cycles[0])
            .expect("original cycle should exist");
        let restored_cycle = restored
            .economy()
            .get_cycle(continued.business_cycles[0])
            .expect("restored cycle should exist");
        assert_eq!(
            original_cycle.owner(),
            BusinessOwner::Organization(successor)
        );
        assert_eq!(
            restored_cycle.owner(),
            BusinessOwner::Organization(successor)
        );
        assert_eq!(original_cycle.business_version(), 2);
        assert_eq!(restored_cycle.business_version(), 2);
        assert_eq!(
            restored
                .world()
                .business_ownership_history(fixture.business)
                .count(),
            2
        );
        assert_eq!(
            original_cycle.gross_revenue(),
            restored_cycle.gross_revenue()
        );
        assert_eq!(original_cycle.net_cash(), restored_cycle.net_cash());
        assert_eq!(
            fixture
                .state
                .finance()
                .get_account(fixture.operating)
                .expect("original operating account should exist")
                .balance(),
            restored
                .finance()
                .get_account(fixture.operating)
                .expect("restored operating account should exist")
                .balance()
        );
        validate_invariants(&fixture.state);
        validate_invariants(&restored);
    }
}
