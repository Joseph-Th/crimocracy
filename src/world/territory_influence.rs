//! Read-only per-district economic-influence aggregation over canonical world, enterprise
//! records; no mutation paths live here.
//!
//! Territory answers "who holds sway in this district" from record-backed economic presence:
//! active enterprises operated at district locations, regardless of host ownership. Every
//! figure is derived from authoritative records at resolution time; nothing about influence
//! is persisted.
//!
//! The consumer is simulation-side delegated expansion (`autonomous_expansion` resolves each
//! district's unique strict leader before consolidating). This summary is not a player
//! information feed: surfacing it verbatim would bypass the provenance-bearing intelligence
//! model.

use crate::core::id::{HistoryEventId, IdExhaustionError, IdKind, NeighborhoodId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::{DAY_MINUTES, SimTime, is_day_boundary};
use crate::enterprises::{EnterpriseLocation, EnterpriseStatus};
use crate::finance::finance_system::{
    FinanceError, ValidatedLedgerTransaction, validate_open_accounts, validate_record_transaction,
    validate_record_transaction_with_openings,
};
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crate::history::HistoryEventKind;
use crate::history::history_system::{
    HistoryError, ValidatedHistoryEvent, validate_territory_standstill_event,
};
use crate::world::OrganizationKind;
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum TerritoryInfluenceError {
    #[error("neighborhood {0} does not exist")]
    MissingNeighborhood(NeighborhoodId),
}

/// An agreed district standstill preserves existing rackets but bars both sides
/// from new rackets and offensive business operations here for thirty days.
pub const STANDSTILL_DAYS: u64 = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerritoryStandstill {
    pub history: HistoryEventId,
    pub district: NeighborhoodId,
    pub counterparty: OrganizationId,
    pub expires_at: SimTime,
}

pub fn active_standstill(
    state: &AppState,
    organization: OrganizationId,
    district: NeighborhoodId,
) -> Option<TerritoryStandstill> {
    state
        .history()
        .events()
        .filter_map(|event| {
            let HistoryEventKind::TerritoryStandstill {
                district: held,
                initiator,
                counterparty,
            } = event.kind()
            else {
                return None;
            };
            if held != district || (initiator != organization && counterparty != organization) {
                return None;
            }
            let end = event
                .occurred_at()
                .as_minutes()
                .checked_add(STANDSTILL_DAYS * DAY_MINUTES)?;
            (state.now().as_minutes() < end).then_some(TerritoryStandstill {
                history: event.id(),
                district,
                counterparty: if initiator == organization {
                    counterparty
                } else {
                    initiator
                },
                expires_at: SimTime::from_minutes(end),
            })
        })
        .last()
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum TerritoryNegotiationError {
    #[error("district {0} does not exist")]
    MissingDistrict(NeighborhoodId),
    #[error("both sides must be different criminal organizations")]
    InvalidParties,
    #[error("both gangs need an active racket in the disputed district to negotiate")]
    NoContestedPresence,
    #[error("rival refuses: its local position is too strong to accept a standstill")]
    RivalRefused,
    #[error(
        "rival requires {required_cents} cents tribute for this standstill; offered {offered_cents} cents"
    )]
    TributeRequired {
        required_cents: i64,
        offered_cents: i64,
    },
    #[error("tribute cannot be negative")]
    InvalidTribute,
    #[error("the initiating gang lacks a street-cash account that can fund the tribute")]
    InsufficientTributeFunds,
    #[error("an active territorial standstill already governs one side in this district")]
    ExistingStandstill,
    #[error("the simulation clock cannot represent a thirty-day agreement")]
    TimeExhausted,
    #[error("negotiation was validated for {expected:?}, but the clock now reads {found:?}")]
    StaleTime { expected: SimTime, found: SimTime },
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error(transparent)]
    Finance(#[from] FinanceError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
}

fn validate_standstill_terms(
    state: &AppState,
    initiator: OrganizationId,
    counterparty: OrganizationId,
    district: NeighborhoodId,
    tribute: Money,
) -> Result<(), TerritoryNegotiationError> {
    if tribute < Money::ZERO {
        return Err(TerritoryNegotiationError::InvalidTribute);
    }
    if state.world().get_neighborhood(district).is_none() {
        return Err(TerritoryNegotiationError::MissingDistrict(district));
    }
    if initiator == counterparty
        || [initiator, counterparty].into_iter().any(|id| {
            state
                .world()
                .get_organization(id)
                .is_none_or(|organization| organization.kind() != OrganizationKind::Criminal)
        })
    {
        return Err(TerritoryNegotiationError::InvalidParties);
    }
    if active_standstill(state, initiator, district).is_some()
        || active_standstill(state, counterparty, district).is_some()
    {
        return Err(TerritoryNegotiationError::ExistingStandstill);
    }
    state
        .now()
        .as_minutes()
        .checked_add(STANDSTILL_DAYS * DAY_MINUTES)
        .ok_or(TerritoryNegotiationError::TimeExhausted)?;
    let summary = resolve_neighborhood_influence(state, district)
        .map_err(|_| TerritoryNegotiationError::MissingDistrict(district))?;
    let count = |organization| {
        let active = summary
            .standings
            .iter()
            .find(|standing| standing.organization == organization)
            .map_or(0, |standing| standing.active_enterprises);
        if active != 0 {
            return active;
        }
        // A gang driven underground within the last seven days is still party
        // to this dispute. Peace cannot require restarting a racket that the
        // other side just closed, or the strongest faction can avoid talks by
        // disabling the last rival front.
        u32::from(
            state
                .enterprises()
                .enterprises_for_organization(organization)
                .any(|racket| {
                    racket.status() == EnterpriseStatus::Suspended
                        && crate::world::territory_conflict::district_of_racket(
                            state,
                            racket.location(),
                        ) == Some(district)
                        && crate::world::territory_conflict::suppression_ends_at(state, racket.id())
                            .is_some()
                }),
        )
    };
    let own = count(initiator);
    let rival = count(counterparty);
    if own == 0 || rival == 0 {
        return Err(TerritoryNegotiationError::NoContestedPresence);
    }
    // Equal rivals and rivals facing stronger encroachment can settle.
    // A dominant rival will not freeze an overwhelming advantage for free.
    let required_cents = i64::from(rival.saturating_sub(own.saturating_add(1))) * 25_000;
    if tribute.cents() < required_cents {
        return Err(TerritoryNegotiationError::TributeRequired {
            required_cents,
            offered_cents: tribute.cents(),
        });
    }
    Ok(())
}

pub struct ValidatedTerritoryStandstill {
    initiator: OrganizationId,
    counterparty: OrganizationId,
    district: NeighborhoodId,
    validated_at: SimTime,
    history: ValidatedHistoryEvent,
    tribute: Money,
    payment: Option<ValidatedLedgerTransaction>,
}

impl ValidatedTerritoryStandstill {
    pub fn commit(self, state: &mut AppState) -> Result<HistoryEventId, TerritoryNegotiationError> {
        if state.now() != self.validated_at {
            return Err(TerritoryNegotiationError::StaleTime {
                expected: self.validated_at,
                found: state.now(),
            });
        }
        validate_standstill_terms(
            state,
            self.initiator,
            self.counterparty,
            self.district,
            self.tribute,
        )?;
        if let Some(payment) = &self.payment {
            payment.ensure_current(state)?;
        }
        let mut budget = vec![(IdKind::HistoryEvent, 1)];
        if let Some(payment) = &self.payment {
            budget.extend(payment.id_budget());
        }
        state.ids.reserve_many(&budget)?;
        let history = self.history.commit(state)?;
        if let Some(payment) = self.payment {
            payment.commit_preflighted(state);
        }
        Ok(history)
    }
}

pub fn validate_negotiate_standstill(
    state: &AppState,
    initiator: OrganizationId,
    counterparty: OrganizationId,
    district: NeighborhoodId,
) -> Result<ValidatedTerritoryStandstill, TerritoryNegotiationError> {
    validate_negotiate_standstill_with_tribute(
        state,
        initiator,
        counterparty,
        district,
        Money::ZERO,
    )
}

/// Weaker gangs can buy a ceasefire rather than accepting an otherwise hopeless
/// war. Tribute moves real street cash into a rival's usable treasury through
/// the canonical balanced ledger, and can fund that rival's later operations.
pub fn validate_negotiate_standstill_with_tribute(
    state: &AppState,
    initiator: OrganizationId,
    counterparty: OrganizationId,
    district: NeighborhoodId,
    tribute: Money,
) -> Result<ValidatedTerritoryStandstill, TerritoryNegotiationError> {
    validate_standstill_terms(state, initiator, counterparty, district, tribute)?;
    let payment = if tribute > Money::ZERO {
        let source = state
            .finance()
            .accounts_for(FinancialOwner::Organization(initiator))
            .filter(|account| {
                account.kind() == AccountKind::StreetCash
                    && account.available_payment_balance() >= tribute
            })
            .max_by_key(|account| {
                (
                    account.available_payment_balance(),
                    std::cmp::Reverse(account.id()),
                )
            })
            .ok_or(TerritoryNegotiationError::InsufficientTributeFunds)?
            .id();
        let draft = |destination| LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: format!(
                "Territorial standstill tribute for {district}: {initiator} to {counterparty}"
            ),
            postings: vec![
                LedgerPosting {
                    account: source,
                    amount: Money::from_cents(-tribute.cents()),
                },
                LedgerPosting {
                    account: destination,
                    amount: tribute,
                },
            ],
            authorization: None,
        };
        let existing = state
            .finance()
            .accounts_for(FinancialOwner::Organization(counterparty))
            .filter(|account| {
                account.kind() == AccountKind::StreetCash
                    && account.balance() >= Money::ZERO
                    && account.version() < u32::MAX
                    && account.balance().checked_add(tribute).is_some()
            })
            .map(|account| account.id())
            .next();
        if let Some(destination) = existing {
            Some(validate_record_transaction(state, draft(destination))?)
        } else {
            let openings = validate_open_accounts(
                state,
                vec![FinancialAccountDraft {
                    owner: FinancialOwner::Organization(counterparty),
                    kind: AccountKind::StreetCash,
                }],
            )?;
            let destination = openings
                .account_id(0)
                .expect("validated one-account opening");
            Some(validate_record_transaction_with_openings(
                state,
                openings,
                draft(destination),
            )?)
        }
    } else {
        None
    };
    let history = validate_territory_standstill_event(state, district, initiator, counterparty)?;
    let mut budget = vec![(IdKind::HistoryEvent, 1)];
    if let Some(payment) = &payment {
        budget.extend(payment.id_budget());
    }
    state.ids.reserve_many(&budget)?;
    Ok(ValidatedTerritoryStandstill {
        initiator,
        counterparty,
        district,
        validated_at: state.now(),
        history,
        tribute,
        payment,
    })
}

/// Gangs which actually fought, and were left with a live or recently
/// displaced local claim, may settle the feud on a later daily boundary.
/// The defeated NPC proposes first, including a real treasury payment if
/// the victor has overwhelming remaining strength. Player organizations
/// never get an agreement signed for them: negotiation is their choice.
pub(crate) fn apply_due_autonomous_territory_standstills(
    state: &mut AppState,
) -> Vec<HistoryEventId> {
    if !is_day_boundary(state.now()) {
        return Vec::new();
    }
    let current = state.now().as_minutes();
    let player = state.player_organization();
    let mut recent = BTreeMap::<NeighborhoodId, (OrganizationId, OrganizationId)>::new();
    for event in state.history().events() {
        let HistoryEventKind::TerritoryClash {
            district,
            attacker,
            defender,
            loser,
            ..
        } = event.kind()
        else {
            continue;
        };
        let age = current.saturating_sub(event.occurred_at().as_minutes());
        if !(DAY_MINUTES..=7 * DAY_MINUTES).contains(&age)
            || player == Some(attacker)
            || player == Some(defender)
        {
            continue;
        }
        let defeated = state
            .enterprises()
            .get_enterprise(loser)
            .expect("validated clash loser must still exist")
            .organization();
        let victor = if defeated == attacker {
            defender
        } else {
            attacker
        };
        recent.insert(district, (defeated, victor));
    }
    let mut agreements = Vec::new();
    for (district, (initiator, counterparty)) in recent {
        let plan = match validate_negotiate_standstill(state, initiator, counterparty, district) {
            Ok(plan) => Ok(plan),
            Err(TerritoryNegotiationError::TributeRequired { required_cents, .. }) => {
                validate_negotiate_standstill_with_tribute(
                    state,
                    initiator,
                    counterparty,
                    district,
                    Money::from_cents(required_cents),
                )
            }
            Err(other) => Err(other),
        };
        if let Ok(plan) = plan
            && let Ok(history) = plan.commit(state)
        {
            agreements.push(history);
        }
    }
    agreements
}

/// One organization's record-backed footprint inside a single district.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerritoryStanding {
    pub organization: OrganizationId,
    /// Active enterprises operating at district locations, regardless of host ownership.
    pub active_enterprises: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NeighborhoodInfluenceSummary {
    /// Ascending organization order; organizations with no district footprint are absent.
    pub standings: Vec<TerritoryStanding>,
}

impl NeighborhoodInfluenceSummary {
    /// The unique strict leader on active-enterprise count, if any. Contested districts —
    /// ties between two or more organizations — have no leader, because territory is
    /// contested rather than shared by default.
    pub fn economic_leader(&self) -> Option<OrganizationId> {
        unique_economic_leader(
            self.standings
                .iter()
                .map(|standing| (standing.organization, standing.active_enterprises)),
        )
    }
}

/// Shared strict-leader rule for authoritative and projected district influence. Keeping this
/// tie behavior in one owner prevents autonomous same-day projections from drifting away from
/// the read-only territory summary they model.
pub(crate) fn unique_economic_leader(
    standings: impl IntoIterator<Item = (OrganizationId, u32)>,
) -> Option<OrganizationId> {
    let mut best_count = 0;
    let mut leader = None;
    for (organization, count) in standings {
        match count.cmp(&best_count) {
            std::cmp::Ordering::Greater => {
                best_count = count;
                leader = Some(organization);
            }
            std::cmp::Ordering::Equal => leader = None,
            std::cmp::Ordering::Less => {}
        }
    }
    leader
}

/// Resolves who holds sway in a district from canonical records alone. Deterministic:
/// standings ascend by organization id.
pub fn resolve_neighborhood_influence(
    state: &AppState,
    neighborhood: NeighborhoodId,
) -> Result<NeighborhoodInfluenceSummary, TerritoryInfluenceError> {
    if state.world().get_neighborhood(neighborhood).is_none() {
        return Err(TerritoryInfluenceError::MissingNeighborhood(neighborhood));
    }

    // Economic presence: active enterprises whose hosted or district location resolves to
    // this neighborhood. The location index serves district-hosted rackets directly and the
    // neighborhood's own businesses cover venue-hosted ones, without scanning every racket
    // in the city. Ordered scans keep the derivation deterministic.
    let mut enterprises_by_org: BTreeMap<OrganizationId, u32> = BTreeMap::new();
    let neighborhood_enterprises = state
        .enterprises()
        .enterprises_at(EnterpriseLocation::Neighborhood(neighborhood))
        .filter(|record| record.status() == EnterpriseStatus::Active)
        .map(|record| record.organization())
        .chain(
            state
                .world()
                .businesses_in_neighborhood(neighborhood)
                .flat_map(|business| {
                    state
                        .enterprises()
                        .enterprises_at(EnterpriseLocation::Business(business.id()))
                })
                .filter(|record| record.status() == EnterpriseStatus::Active)
                .map(|record| record.organization()),
        );
    for organization in neighborhood_enterprises {
        *enterprises_by_org.entry(organization).or_default() += 1;
    }

    let standings = enterprises_by_org
        .into_iter()
        .map(|(organization, active_enterprises)| TerritoryStanding {
            organization,
            active_enterprises,
        })
        .collect();

    Ok(NeighborhoodInfluenceSummary { standings })
}

#[cfg(test)]
mod tests;
