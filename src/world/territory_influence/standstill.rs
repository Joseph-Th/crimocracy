//! Canonical territorial standstill negotiation, tribute and daily peace decisions.
//!
//! Agreement history remains the authority for restrictions and expiry. The
//! derived economic footprint supplies the current negotiation terms.

use super::resolve_neighborhood_influence;
use crate::core::id::{
    HistoryEventId, IdExhaustionError, IdKind, LedgerTransactionId, NeighborhoodId, OrganizationId,
};
use crate::core::state::AppState;
use crate::core::time::{DAY_MINUTES, SimTime, is_day_boundary};
use crate::core::version::VersionCapacityError;
use crate::decisions::decision_system::{
    ValidatedProtectionStandstillCancellation, validate_cancel_protection_for_standstill,
};
use crate::enterprises::EnterpriseStatus;
use crate::finance::finance_system::{
    FinanceError, ValidatedLedgerTransaction, validate_open_accounts, validate_record_transaction,
    validate_record_transaction_with_openings,
};
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crate::history::HistoryEventKind;
use crate::history::history_system::{HistoryError, validate_territory_standstill_event};
use crate::world::OrganizationKind;
use crate::world::world_system::sole_available_executive;
use std::collections::BTreeMap;
use thiserror::Error;

/// An agreed district standstill preserves existing rackets but bars both sides
/// from new rackets and offensive business operations here for thirty days.
pub const STANDSTILL_DAYS: u64 = 30;

pub(crate) fn standstill_expiry_at(signed_at: SimTime) -> Option<SimTime> {
    signed_at
        .as_minutes()
        .checked_add(STANDSTILL_DAYS * DAY_MINUTES)
        .map(SimTime::from_minutes)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerritoryStandstill {
    pub history: HistoryEventId,
    pub district: NeighborhoodId,
    pub counterparty: OrganizationId,
    pub expires_at: SimTime,
    pub initiator: OrganizationId,
    pub tribute: Money,
    pub settlement: Option<LedgerTransactionId>,
}

pub fn active_standstill(
    state: &AppState,
    organization: OrganizationId,
    district: NeighborhoodId,
) -> Option<TerritoryStandstill> {
    let event = state
        .history()
        .latest_territory_standstill(organization, district)?;
    let HistoryEventKind::TerritoryStandstill {
        initiator,
        counterparty,
        tribute,
        settlement,
        ..
    } = event.kind()
    else {
        unreachable!("territory standstill index contains only agreements");
    };
    let end = standstill_expiry_at(event.occurred_at())?;
    (state.now() < end).then_some(TerritoryStandstill {
        history: event.id(),
        district,
        counterparty: if initiator == organization {
            counterparty
        } else {
            initiator
        },
        expires_at: end,
        initiator,
        tribute,
        settlement,
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum TerritoryNegotiationError {
    #[error("district {0} does not exist")]
    MissingDistrict(NeighborhoodId),
    #[error("both sides must be different criminal organizations")]
    InvalidParties,
    #[error("both gangs require a sole available executive to negotiate a standstill")]
    ExecutiveUnavailable,
    #[error(
        "both gangs need an active or recently displaced racket in the disputed district to negotiate"
    )]
    NoContestedPresence,
    #[error(
        "rival requires {required_cents} cents tribute for this standstill; offered {offered_cents} cents"
    )]
    TributeRequired {
        required_cents: i64,
        offered_cents: i64,
    },
    #[error("tribute cannot be negative")]
    InvalidTribute,
    #[error("the initiating gang has insufficient spendable street cash for the tribute")]
    InsufficientTributeFunds,
    #[error("an active territorial standstill already governs one side in this district")]
    ExistingStandstill,
    #[error("the simulation clock cannot represent a thirty-day agreement")]
    TimeExhausted,
    #[error("negotiation was validated for {expected:?}, but the clock now reads {found:?}")]
    StaleTime { expected: SimTime, found: SimTime },
    #[error("the pending protection offer in district {0} changed after negotiation")]
    ProtectionDemandChanged(NeighborhoodId),
    #[error(transparent)]
    VersionCapacity(#[from] VersionCapacityError),
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
    standstill_expiry_at(state.now()).ok_or(TerritoryNegotiationError::TimeExhausted)?;
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
                        && racket.location().neighborhood(state.world()) == Some(district)
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
    // A local captain can fight, but only unified, available executive
    // authority can bind an entire organization. All negotiation routes
    // (including held player orders) must satisfy the same rule.
    if [initiator, counterparty]
        .into_iter()
        .any(|party| sole_available_executive(state, party).is_none())
    {
        return Err(TerritoryNegotiationError::ExecutiveUnavailable);
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
    tribute: Money,
    payment: Option<ValidatedLedgerTransaction>,
    protection_cancellation: ValidatedProtectionStandstillCancellation,
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
        if !self.protection_cancellation.is_current(state) {
            return Err(TerritoryNegotiationError::ProtectionDemandChanged(
                self.district,
            ));
        }
        if let Some(payment) = &self.payment {
            payment.ensure_current(state)?;
        }
        let mut budget = vec![(IdKind::HistoryEvent, 1)];
        if let Some(payment) = &self.payment {
            budget.extend(payment.id_budget());
        }
        state.ids.reserve_many(&budget)?;
        // The ledger allocator may have advanced since validation because of
        // unrelated same-minute work. Bind the actual next ID only after preflight.
        let settlement = self
            .payment
            .as_ref()
            .map(|_| LedgerTransactionId::from_raw(state.ids.next_raw(IdKind::LedgerTransaction)));
        let history = validate_territory_standstill_event(
            state,
            self.district,
            self.initiator,
            self.counterparty,
            self.tribute,
            settlement,
        )?;
        let paid = self
            .payment
            .map(|payment| payment.commit_preflighted(state));
        debug_assert_eq!(paid, settlement);
        let history = history
            .commit(state)
            .expect("preflighted standstill history must commit");
        self.protection_cancellation
            .commit_preflighted(state, history);
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
    let protection_cancellation =
        validate_cancel_protection_for_standstill(state, initiator, counterparty, district)?;
    let payment = if tribute > Money::ZERO {
        // A ceasefire uses the organization's combined street-cash reserves,
        // not the balance of any particular racket account.
        let mut postings = crate::finance::helpers::payment_debits(
            state
                .finance()
                .accounts_for(FinancialOwner::Organization(initiator))
                .filter(|account| account.kind() == AccountKind::StreetCash),
            tribute,
        )
        .map_err(|_| TerritoryNegotiationError::InsufficientTributeFunds)?;
        let draft = |destination| {
            postings.push(LedgerPosting {
                account: destination,
                amount: tribute,
            });
            LedgerTransactionDraft {
                occurred_at: state.now(),
                memo: tribute_memo(district, initiator, counterparty),
                postings,
                authorization: None,
            }
        };
        let existing = state
            .finance()
            .accounts_for(FinancialOwner::Organization(counterparty))
            .find(|account| {
                account.kind() == AccountKind::StreetCash
                    && account.can_receive_liquid_payment(tribute)
            })
            .map(|account| account.id());
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
    let settlement = payment
        .as_ref()
        .map(|_| LedgerTransactionId::from_raw(state.ids.next_raw(IdKind::LedgerTransaction)));
    validate_territory_standstill_event(
        state,
        district,
        initiator,
        counterparty,
        tribute,
        settlement,
    )?;
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
        tribute,
        payment,
        protection_cancellation,
    })
}

fn tribute_memo(
    district: NeighborhoodId,
    initiator: OrganizationId,
    counterparty: OrganizationId,
) -> String {
    format!("Territorial standstill tribute for {district}: {initiator} to {counterparty}")
}

/// An agreement is valid only when its optional receipt proves the exact
/// negotiated amount, minute, payer, receiver and money kind. Free agreements
/// must not claim a receipt. No current balances enter historical validation.
pub(crate) fn standstill_tribute_matches(
    state: &AppState,
    district: NeighborhoodId,
    initiator: OrganizationId,
    counterparty: OrganizationId,
    tribute: Money,
    settlement: Option<LedgerTransactionId>,
    at: SimTime,
) -> bool {
    if tribute == Money::ZERO {
        return settlement.is_none();
    }
    if tribute < Money::ZERO {
        return false;
    }
    let Some(record) = settlement.and_then(|id| state.finance().get_transaction(id)) else {
        return false;
    };
    if record.occurred_at() != at
        || record.memo() != tribute_memo(district, initiator, counterparty)
        || record.budget_usage().is_some()
    {
        return false;
    }
    let mut debited = 0_i128;
    let mut credits = 0;
    for posting in record.postings() {
        let Some(account) = state.finance().get_account(posting.account) else {
            return false;
        };
        if account.kind() != AccountKind::StreetCash {
            return false;
        }
        match account.owner() {
            FinancialOwner::Organization(owner)
                if owner == initiator && posting.amount < Money::ZERO =>
            {
                debited -= i128::from(posting.amount.cents())
            }
            FinancialOwner::Organization(owner)
                if owner == counterparty && posting.amount == tribute =>
            {
                credits += 1
            }
            FinancialOwner::Organization(_)
            | FinancialOwner::Character(_)
            | FinancialOwner::Business(_) => return false,
        }
    }
    credits == 1 && debited == i128::from(tribute.cents())
}

/// Gangs which actually fought, and were left with a live or recently
/// displaced local claim, may settle the feud on a later daily boundary.
/// The defeated NPC proposes first, including a real treasury payment if
/// the victor has overwhelming remaining strength. The canonical negotiation
/// validator checks both parties' executive authority. Player organizations
/// never get an agreement signed for them: negotiation is their choice.
pub(crate) fn apply_due_autonomous_territory_standstills(
    state: &mut AppState,
) -> Vec<HistoryEventId> {
    if !is_day_boundary(state.now()) {
        return Vec::new();
    }
    let current = state.now().as_minutes();
    let player = state.player_organization();
    let mut recent = BTreeMap::<
        (NeighborhoodId, OrganizationId, OrganizationId),
        Option<(OrganizationId, OrganizationId)>,
    >::new();
    // History IDs follow campaign chronology. Only the recent seven-day tail can produce
    // new peace offers; a reverse walk stops as soon as it reaches older history.
    for event in state.history().events().rev().take_while(|event| {
        current.saturating_sub(event.occurred_at().as_minutes()) <= 7 * DAY_MINUTES
    }) {
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
        if player == Some(attacker) || player == Some(defender) {
            continue;
        }
        // The newest clash governs this pair even when it is too recent for talks.
        // Recording an ineligible latest clash prevents an older fight from
        // bypassing the full-day wait. Other pairs remain independent.
        let pair = (attacker.min(defender), attacker.max(defender));
        recent.entry((district, pair.0, pair.1)).or_insert_with(|| {
            if age < DAY_MINUTES {
                return None;
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
            Some((defeated, victor))
        });
    }
    let mut agreements = Vec::new();
    for ((district, _, _), parties) in recent {
        let Some((initiator, counterparty)) = parties else {
            continue;
        };
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
