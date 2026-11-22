//! Fixed-term freelance crew contracts. A signed freelancer joins an actual
//! reporting line for a bounded period so the normal operation, custody,
//! relationship and wage rules apply. Expiration uses the canonical world
//! reassignment guard: a booked or detained contractor is not silently deleted.
//! The signing fee is paid from real street cash to the named person's account.

use crate::core::id::{CharacterId, IdExhaustionError, LedgerTransactionId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::{DAY_MINUTES, SimTime, ensure_time_current, is_day_boundary};
use crate::finance::finance_system::{
    FinanceError, ValidatedLedgerTransaction, validate_open_accounts, validate_record_transaction,
    validate_record_transaction_with_openings,
};
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crate::world::world_system::{
    ValidatedCharacterReassignment, WorldError, validate_reassign_character,
};
use crate::world::{CapabilityKind, IndependentVocation, OrganizationKind, WorldState};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FreelanceContractRecord {
    freelancer: CharacterId,
    sponsor: OrganizationId,
    manager: CharacterId,
    signed_at: SimTime,
    expires_at: SimTime,
    ended_at: Option<SimTime>,
    fee_cents: i64,
    transaction: LedgerTransactionId,
}
impl FreelanceContractRecord {
    pub fn freelancer(&self) -> CharacterId {
        self.freelancer
    }
    pub fn sponsor(&self) -> OrganizationId {
        self.sponsor
    }
    pub fn manager(&self) -> CharacterId {
        self.manager
    }
    pub fn signed_at(&self) -> SimTime {
        self.signed_at
    }
    pub fn expires_at(&self) -> SimTime {
        self.expires_at
    }
    pub fn ended_at(&self) -> Option<SimTime> {
        self.ended_at
    }
    pub fn fee_cents(&self) -> i64 {
        self.fee_cents
    }
    pub fn transaction(&self) -> LedgerTransactionId {
        self.transaction
    }
}

impl WorldState {
    pub fn freelance_contracts(&self) -> impl Iterator<Item = &FreelanceContractRecord> {
        self.freelance_contracts
            .values()
            .flat_map(|records| records.iter())
    }
    pub fn active_freelance_contract(
        &self,
        character: CharacterId,
    ) -> Option<&FreelanceContractRecord> {
        self.freelance_contracts
            .get(&character)?
            .last()
            .filter(|record| record.ended_at.is_none())
    }
    pub(super) fn record_freelance_contract(&mut self, record: FreelanceContractRecord) {
        self.freelance_contracts
            .entry(record.freelancer)
            .or_default()
            .push(record);
    }
    pub(super) fn end_freelance_contract(&mut self, character: CharacterId, now: SimTime) {
        self.freelance_contracts
            .get_mut(&character)
            .and_then(|records| records.last_mut())
            .expect("validated active freelance contract exists")
            .ended_at = Some(now);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum FreelanceError {
    #[error("character {0} is not an available unaffiliated freelance criminal")]
    Unavailable(CharacterId),
    #[error("criminal organization {0} is not eligible to contract staff")]
    InvalidSponsor(OrganizationId),
    #[error("a freelance contract must last 1 through 14 days")]
    InvalidDuration,
    #[error("contractor has no established personal relationship with organization leader")]
    NoRelationship,
    #[error("freelancer already has an active contract")]
    AlreadyContracted,
    #[error("organization has insufficient street cash for {required_cents}-cent signing fee")]
    Unfunded { required_cents: i64 },
    #[error("contract was validated at {expected:?} but current time is {found:?}")]
    StaleTime { expected: SimTime, found: SimTime },
    #[error("freelance contract would exceed representable simulation clock or history capacity")]
    Capacity,
    #[error(transparent)]
    World(#[from] WorldError),
    #[error(transparent)]
    IdExhaustion(#[from] IdExhaustionError),
    #[error(transparent)]
    Finance(#[from] FinanceError),
}

pub struct ValidatedFreelanceContract {
    payment: ValidatedLedgerTransaction,
    membership: ValidatedCharacterReassignment,
    record: FreelanceContractRecord,
}

impl ValidatedFreelanceContract {
    pub fn fee_cents(&self) -> i64 {
        self.record.fee_cents
    }
    pub fn commit(self, state: &mut AppState) -> Result<(), FreelanceError> {
        ensure_time_current(state.now(), self.record.signed_at)
            .map_err(|(expected, found)| FreelanceError::StaleTime { expected, found })?;
        ensure_eligible(
            state,
            self.record.sponsor,
            self.record.freelancer,
            self.record.manager,
        )?;
        self.membership.ensure_current(state)?;
        self.payment.ensure_current(state)?;
        state.ids.reserve_many(&self.payment.id_budget())?;
        let transaction = self.payment.commit_preflighted(state);
        debug_assert_eq!(transaction, self.record.transaction);
        self.membership.commit_preflighted(state);
        state.world.record_freelance_contract(self.record);
        Ok(())
    }
}

fn ensure_eligible(
    state: &AppState,
    sponsor: OrganizationId,
    freelancer: CharacterId,
    manager: CharacterId,
) -> Result<(), FreelanceError> {
    if state
        .world()
        .get_organization(sponsor)
        .is_none_or(|organization| organization.kind() != OrganizationKind::Criminal)
    {
        return Err(FreelanceError::InvalidSponsor(sponsor));
    }
    let candidate = state
        .world()
        .get_character(freelancer)
        .filter(|person| {
            person.organization().is_none()
                && person.life().vocation() == IndependentVocation::FreelanceCriminal
                && state
                    .legal()
                    .active_arrest_for_character(person.id())
                    .is_none()
        })
        .ok_or(FreelanceError::Unavailable(freelancer))?;
    let leader = state
        .world()
        .get_character(manager)
        .filter(|person| {
            person.organization() == Some(sponsor)
                && state
                    .legal()
                    .active_arrest_for_character(person.id())
                    .is_none()
        })
        .ok_or(FreelanceError::Unavailable(manager))?;
    let _ = (candidate, leader);
    if state
        .world()
        .active_freelance_contract(freelancer)
        .is_some()
    {
        return Err(FreelanceError::AlreadyContracted);
    }
    if !state
        .social()
        .get_relationship(freelancer, manager)
        .is_some_and(|relationship| relationship.dimensions().trust.value() >= 30)
    {
        return Err(FreelanceError::NoRelationship);
    }
    Ok(())
}

pub fn validate_hire_freelancer(
    state: &AppState,
    sponsor: OrganizationId,
    freelancer: CharacterId,
    manager: CharacterId,
    days: u8,
) -> Result<ValidatedFreelanceContract, FreelanceError> {
    if !(1..=14).contains(&days) {
        return Err(FreelanceError::InvalidDuration);
    }
    ensure_eligible(state, sponsor, freelancer, manager)?;
    let person = state
        .world()
        .get_character(freelancer)
        .expect("qualified freelance character");
    let max_skill = [
        CapabilityKind::Burglary,
        CapabilityKind::Violence,
        CapabilityKind::Stealth,
        CapabilityKind::Driving,
        CapabilityKind::Surveillance,
    ]
    .into_iter()
    .filter_map(|kind| person.capability(kind))
    .map(|score| i64::from(score.value()))
    .max()
    .unwrap_or(0);
    let fee_cents = 10_000_i64 + i64::from(days) * (3_000 + max_skill * 50);
    let funding = state
        .finance()
        .accounts_for(FinancialOwner::Organization(sponsor))
        .filter(|account| {
            account.kind() == AccountKind::StreetCash
                && account.available_payment_balance().cents() >= fee_cents
        })
        .max_by_key(|account| {
            (
                account.available_payment_balance().cents(),
                std::cmp::Reverse(account.id()),
            )
        })
        .map(|account| account.id())
        .ok_or(FreelanceError::Unfunded {
            required_cents: fee_cents,
        })?;
    let recipient = state
        .finance()
        .accounts_for(FinancialOwner::Character(freelancer))
        .find(|account| {
            account.kind() == AccountKind::StreetCash
                && account.can_receive_liquid_payment(Money::from_cents(fee_cents))
        })
        .map(|account| account.id());
    let openings = if recipient.is_none() {
        Some(validate_open_accounts(
            state,
            vec![FinancialAccountDraft {
                owner: FinancialOwner::Character(freelancer),
                kind: AccountKind::StreetCash,
            }],
        )?)
    } else {
        None
    };
    let recipient = recipient.unwrap_or_else(|| {
        openings
            .as_ref()
            .expect("payment account opening planned")
            .account_id(0)
            .expect("account ID predicted")
    });
    let draft = LedgerTransactionDraft {
        occurred_at: state.now(),
        memo: format!("Fixed-term freelance services of {}", person.name()),
        postings: vec![
            LedgerPosting {
                account: funding,
                amount: Money::from_cents(-fee_cents),
            },
            LedgerPosting {
                account: recipient,
                amount: Money::from_cents(fee_cents),
            },
        ],
        authorization: None,
    };
    let payment = match openings {
        Some(openings) => validate_record_transaction_with_openings(state, openings, draft)?,
        None => validate_record_transaction(state, draft)?,
    };
    let membership = validate_reassign_character(state, freelancer, Some(sponsor), Some(manager))?;
    let expires_at = state
        .now()
        .checked_add(crate::core::time::SimDuration::from_minutes(
            u32::from(days) * DAY_MINUTES as u32,
        ))
        .ok_or(FreelanceError::Capacity)?;
    let count = state
        .world()
        .freelance_contracts
        .get(&freelancer)
        .map_or(0, Vec::len);
    if count >= u32::MAX as usize {
        return Err(FreelanceError::Capacity);
    }
    let transaction = crate::core::id::LedgerTransactionId::from_raw(
        state
            .ids
            .next_raw(crate::core::id::IdKind::LedgerTransaction),
    );
    Ok(ValidatedFreelanceContract {
        payment,
        membership,
        record: FreelanceContractRecord {
            freelancer,
            sponsor,
            manager,
            signed_at: state.now(),
            expires_at,
            ended_at: None,
            fee_cents,
            transaction,
        },
    })
}

/// Contract workers leave after their agreed term. If a case, police custody
/// or a scheduled operation still binds them, expiry is deferred, not forced.
pub fn apply_due_freelance_expirations(state: &mut AppState) -> usize {
    if !is_day_boundary(state.now()) {
        return 0;
    }
    let due: Vec<_> = state
        .world()
        .freelance_contracts()
        .filter(|record| record.ended_at.is_none() && record.expires_at <= state.now())
        .map(|record| (record.freelancer, record.sponsor))
        .collect();
    let mut ended = 0;
    for (freelancer, sponsor) in due {
        let org = state
            .world()
            .get_character(freelancer)
            .and_then(|person| person.organization());
        if org == Some(sponsor) {
            let Ok(release) = validate_reassign_character(state, freelancer, None, None) else {
                continue;
            };
            if release.commit(state).is_err() {
                continue;
            }
        }
        if state
            .world()
            .active_freelance_contract(freelancer)
            .is_some()
        {
            state.world.end_freelance_contract(freelancer, state.now());
        }
        ended += 1;
    }
    ended
}

pub(crate) fn valid_freelance_contracts(state: &AppState) -> bool {
    state
        .world()
        .freelance_contracts
        .iter()
        .all(|(&freelancer, history)| {
            !history.is_empty()
                && history.len() <= u32::MAX as usize
                && history.iter().enumerate().all(|(index, record)| {
                    let person = state.world().get_character(freelancer);
                    let sponsor = state.world().get_organization(record.sponsor);
                    let leader = state.world().get_character(record.manager);
                    let transaction = state.finance().get_transaction(record.transaction);
                    record.freelancer == freelancer
                        && person.is_some_and(|person| {
                            person.life().vocation() == IndependentVocation::FreelanceCriminal
                        })
                        && sponsor.is_some_and(|record| record.kind() == OrganizationKind::Criminal)
                        && leader.is_some()
                        && record.signed_at < record.expires_at
                        && record.signed_at <= state.now()
                        && record.fee_cents > 0
                        && record
                            .ended_at
                            .is_none_or(|end| end >= record.signed_at && end <= state.now())
                        && (index + 1 == history.len() || record.ended_at.is_some())
                        && (index == 0
                            || history[index - 1]
                                .ended_at
                                .is_some_and(|end| end <= record.signed_at))
                        && (record.ended_at.is_some()
                            || person.is_some_and(|person| {
                                person.organization() == Some(record.sponsor)
                            }))
                        && transaction.is_some_and(|transaction| {
                            transaction.occurred_at() == record.signed_at
                                && transaction.postings().len() == 2
                                && transaction.postings().iter().any(|posting| {
                                    posting.amount.cents() == record.fee_cents
                                        && state.finance().get_account(posting.account).is_some_and(
                                            |account| {
                                                account.owner()
                                                    == FinancialOwner::Character(freelancer)
                                            },
                                        )
                                })
                                && transaction.postings().iter().any(|posting| {
                                    posting.amount.cents() == -record.fee_cents
                                        && state.finance().get_account(posting.account).is_some_and(
                                            |account| {
                                                account.owner()
                                                    == FinancialOwner::Organization(record.sponsor)
                                                    && account.kind() == AccountKind::StreetCash
                                            },
                                        )
                                })
                        })
                })
        })
}
