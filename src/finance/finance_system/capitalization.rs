//! Explicit conversion of an organization's accounted funds into traceable
//! street operating cash. This is not laundering: the movement is one-way
//! through a balanced, permanently recorded organization ledger transaction.

use super::{FinanceError, ValidatedLedgerTransaction, validate_record_transaction};
use crate::core::id::{FinancialAccountId, OrganizationId};
use crate::core::state::AppState;
use crate::finance::{AccountKind, FinancialOwner, LedgerPosting, LedgerTransactionDraft, Money};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreetCapitalizationDraft {
    pub organization: OrganizationId,
    pub accounted_account: FinancialAccountId,
    pub street_account: FinancialAccountId,
    pub amount: Money,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum StreetCapitalizationError {
    #[error("street capitalization must be a positive amount")]
    InvalidAmount,
    #[error("source must be an accounted-funds account owned by the organization")]
    InvalidSource,
    #[error("destination must be a street-cash account owned by the organization")]
    InvalidDestination,
    #[error("source funding has {available} spendable cents, less than the requested {requested}")]
    InsufficientFunds { available: i64, requested: i64 },
    #[error("destination cannot accept a full street-cash receipt or clear its existing deficit")]
    DestinationUnavailable,
    #[error(transparent)]
    Finance(#[from] FinanceError),
}

/// Lose the clean-money designation to fund on-the-street work. The organization
/// must explicitly select both of its accounts; any old street deficit must
/// be cleared in full, and generic same-kind transfers remain restrictive.
/// The returned canonical ledger token rechecks accounts, versions and time at commit.
pub fn validate_capitalize_street_cash(
    state: &AppState,
    draft: StreetCapitalizationDraft,
) -> Result<ValidatedLedgerTransaction, StreetCapitalizationError> {
    if draft.amount <= Money::ZERO {
        return Err(StreetCapitalizationError::InvalidAmount);
    }
    let owner = FinancialOwner::Organization(draft.organization);
    let source = state
        .finance()
        .get_account(draft.accounted_account)
        .ok_or(FinanceError::MissingAccount(draft.accounted_account))?;
    if source.owner() != owner || source.kind() != AccountKind::AccountedFunds {
        return Err(StreetCapitalizationError::InvalidSource);
    }
    validate_capitalization(
        state,
        draft.organization,
        draft.street_account,
        draft.amount,
        vec![(source.available_payment_balance(), source.id())],
        0,
    )
}

/// Autonomous treasury funding uses every spendable clean account belonging to
/// the organization, leaving its required clean reserve across the same pool.
/// Prefer larger sources to minimize postings; account IDs break exact ties.
pub(crate) fn validate_capitalize_street_cash_from_reserves(
    state: &AppState,
    organization: OrganizationId,
    street_account: FinancialAccountId,
    amount: Money,
    payroll_reserve: i128,
) -> Result<ValidatedLedgerTransaction, StreetCapitalizationError> {
    if amount <= Money::ZERO {
        return Err(StreetCapitalizationError::InvalidAmount);
    }
    let mut sources: Vec<_> = state
        .finance()
        .accounts_for(FinancialOwner::Organization(organization))
        .filter(|account| account.kind() == AccountKind::AccountedFunds)
        .map(|account| (account.available_payment_balance(), account.id()))
        .filter(|(available, _)| *available > Money::ZERO)
        .collect();
    sources.sort_unstable_by_key(|&(available, id)| (std::cmp::Reverse(available), id));
    validate_capitalization(
        state,
        organization,
        street_account,
        amount,
        sources,
        payroll_reserve,
    )
}

fn validate_capitalization(
    state: &AppState,
    organization: OrganizationId,
    street_account: FinancialAccountId,
    amount: Money,
    sources: Vec<(Money, FinancialAccountId)>,
    reserve: i128,
) -> Result<ValidatedLedgerTransaction, StreetCapitalizationError> {
    let owner = FinancialOwner::Organization(organization);
    let destination = state
        .finance()
        .get_account(street_account)
        .ok_or(FinanceError::MissingAccount(street_account))?;
    if destination.owner() != owner || destination.kind() != AccountKind::StreetCash {
        return Err(StreetCapitalizationError::InvalidDestination);
    }
    let available = (sources
        .iter()
        .map(|(balance, _)| i128::from(balance.cents()))
        .sum::<i128>()
        - reserve.max(0))
    .max(0);
    if available < i128::from(amount.cents()) {
        return Err(StreetCapitalizationError::InsufficientFunds {
            available: i64::try_from(available).expect("insufficient funds fit money range"),
            requested: amount.cents(),
        });
    }
    // Explicit capitalization may repay debt, but only by clearing the entire
    // deficit in a balanced transfer from the organization's clean account.
    let fully_clears_deficit = destination.balance() < Money::ZERO
        && destination.version() < u32::MAX
        && destination
            .balance()
            .checked_add(amount)
            .is_some_and(|balance| balance >= Money::ZERO);
    if !destination.can_receive_liquid_payment(amount) && !fully_clears_deficit {
        return Err(StreetCapitalizationError::DestinationUnavailable);
    }
    let mut remaining = amount.cents();
    let mut postings = Vec::new();
    for (available, account) in sources {
        let debit = remaining.min(available.cents());
        if debit > 0 {
            postings.push(LedgerPosting {
                account,
                amount: Money::from_cents(-debit),
            });
            remaining -= debit;
        }
        if remaining == 0 {
            break;
        }
    }
    debug_assert_eq!(remaining, 0);
    postings.push(LedgerPosting {
        account: street_account,
        amount,
    });
    Ok(validate_record_transaction(
        state,
        LedgerTransactionDraft {
            occurred_at: state.now(),
            memo: "Treasury capitalization: accounted funds withdrawn for street operations"
                .to_owned(),
            postings,
            authorization: None,
        },
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_registry;
    use crate::core::invariants::{validate_invariants, validate_state_against_registry};
    use crate::core::persistence::{build_save, restore_save};
    use crate::core::time::SimDuration;
    use crate::finance::FinancialAccountDraft;
    use crate::world::world_system::insert_organization;
    use crate::world::{OrganizationDraft, OrganizationKind};

    #[test]
    fn capitalization_is_funded_one_way_and_survives_save_restore() {
        let registry = build_registry();
        let mut state = AppState::new(0x0CA7_17A1);
        let organization = insert_organization(
            &registry,
            &mut state,
            OrganizationDraft {
                name: "Capitalized outfit".into(),
                kind: OrganizationKind::Criminal,
            },
        )
        .unwrap();
        let mut account = |kind| {
            super::super::insert_account(
                &mut state,
                FinancialAccountDraft {
                    owner: FinancialOwner::Organization(organization),
                    kind,
                },
            )
            .unwrap()
        };
        let accounted = account(AccountKind::AccountedFunds);
        let street = account(AccountKind::StreetCash);
        let old_deficit = account(AccountKind::StreetCash);
        let settlement = account(AccountKind::Settlement);
        validate_record_transaction(
            &state,
            LedgerTransactionDraft {
                occurred_at: state.now(),
                memo: "Fixture lawful funds".into(),
                postings: vec![
                    LedgerPosting {
                        account: settlement,
                        amount: Money::from_cents(-20_000),
                    },
                    LedgerPosting {
                        account: accounted,
                        amount: Money::from_cents(20_000),
                    },
                ],
                authorization: None,
            },
        )
        .unwrap()
        .commit(&mut state)
        .unwrap();
        validate_record_transaction(
            &state,
            LedgerTransactionDraft {
                occurred_at: state.now(),
                memo: "Earlier operating deficit".into(),
                postings: vec![
                    LedgerPosting {
                        account: old_deficit,
                        amount: Money::from_cents(-500),
                    },
                    LedgerPosting {
                        account: settlement,
                        amount: Money::from_cents(500),
                    },
                ],
                authorization: None,
            },
        )
        .unwrap()
        .commit(&mut state)
        .unwrap();
        let draft = StreetCapitalizationDraft {
            organization,
            accounted_account: accounted,
            street_account: street,
            amount: Money::from_cents(7_000),
        };
        let before = bincode::serialize(&state).unwrap();
        assert_eq!(
            validate_capitalize_street_cash(
                &state,
                StreetCapitalizationDraft {
                    street_account: old_deficit,
                    amount: Money::from_cents(250),
                    ..draft
                }
            )
            .err(),
            Some(StreetCapitalizationError::DestinationUnavailable)
        );
        assert_eq!(
            validate_capitalize_street_cash(
                &state,
                StreetCapitalizationDraft {
                    amount: Money::from_cents(25_000),
                    ..draft
                }
            )
            .err(),
            Some(StreetCapitalizationError::InsufficientFunds {
                available: 20_000,
                requested: 25_000,
            })
        );
        assert_eq!(bincode::serialize(&state).unwrap(), before);
        assert_eq!(
            validate_capitalize_street_cash_from_reserves(
                &state,
                organization,
                street,
                Money::from_cents(7_000),
                13_001
            )
            .err(),
            Some(StreetCapitalizationError::InsufficientFunds {
                available: 6_999,
                requested: 7_000
            }),
        );
        assert!(
            validate_capitalize_street_cash_from_reserves(
                &state,
                organization,
                street,
                Money::from_cents(7_000),
                13_000
            )
            .is_ok()
        );
        assert_eq!(bincode::serialize(&state).unwrap(), before);
        let token = validate_capitalize_street_cash(&state, draft).unwrap();
        let mut stale = state.clone();
        stale.advance_clock(SimDuration::ONE_MINUTE);
        let stale_before = bincode::serialize(&stale).unwrap();
        assert!(matches!(
            token.commit(&mut stale),
            Err(FinanceError::NonCurrentTransactionTime { .. })
        ));
        assert_eq!(bincode::serialize(&stale).unwrap(), stale_before);
        let mut restored = restore_save(&registry, build_save(&registry, &state).unwrap()).unwrap();
        for live in [&mut state, &mut restored] {
            let token = validate_capitalize_street_cash(live, draft).unwrap();
            let transaction = token.commit(live).unwrap();
            assert_eq!(
                live.finance()
                    .get_account(accounted)
                    .unwrap()
                    .balance()
                    .cents(),
                13_000
            );
            assert_eq!(
                live.finance()
                    .get_account(street)
                    .unwrap()
                    .balance()
                    .cents(),
                7_000
            );
            assert_eq!(
                live.finance()
                    .get_account(old_deficit)
                    .unwrap()
                    .balance()
                    .cents(),
                -500
            );
            assert!(live.finance().get_transaction(transaction).is_some());
            let explicit_repayment = validate_capitalize_street_cash(
                live,
                StreetCapitalizationDraft {
                    street_account: old_deficit,
                    amount: Money::from_cents(1_000),
                    ..draft
                },
            )
            .unwrap();
            explicit_repayment.commit(live).unwrap();
            assert_eq!(
                live.finance()
                    .get_account(old_deficit)
                    .unwrap()
                    .balance()
                    .cents(),
                500
            );
            assert_eq!(
                live.finance()
                    .get_account(accounted)
                    .unwrap()
                    .balance()
                    .cents(),
                12_000
            );
            validate_state_against_registry(&registry, live).unwrap();
            validate_invariants(live);
        }
        assert_eq!(
            bincode::serialize(&state).unwrap(),
            bincode::serialize(&restored).unwrap()
        );
    }
}
