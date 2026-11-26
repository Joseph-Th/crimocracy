//! Final retirement settlement: canonical payee roster, ledger planning, held-order freshness,
//! and terminal settlement provenance.

use super::{
    RETIREMENT_MIN_KEY_PERSONNEL, RETIREMENT_NEST_EGG, RETIREMENT_SEVERANCE_DAYS, VictoryError,
};
use crate::core::id::{CharacterId, IdKind, LedgerTransactionId, OrganizationId};
use crate::core::state::AppState;
use crate::core::time::SimTime;
use crate::finance::finance_system::{
    ValidatedLedgerTransaction, validate_open_accounts, validate_record_transaction,
    validate_record_transaction_with_openings,
};
use crate::finance::{
    AccountKind, FinancialAccountDraft, FinancialOwner, LedgerPosting, LedgerTransactionDraft,
    Money,
};
use crate::registry::Registry;
use std::collections::BTreeMap;

const SETTLEMENT_MEMO: &str =
    "Final campaign retirement settlement: leader nest egg and crew severance";

pub(super) fn retirement_members(
    registry: &Registry,
    state: &AppState,
    organization: OrganizationId,
) -> Result<(Vec<(CharacterId, Money)>, Money), VictoryError> {
    let severance = registry
        .upkeep()
        .per_member_daily()
        .checked_mul(RETIREMENT_SEVERANCE_DAYS)
        .ok_or(VictoryError::Arithmetic)?;
    retirement_payees(state, organization, severance)
}

fn retirement_payees(
    state: &AppState,
    organization: OrganizationId,
    severance: Money,
) -> Result<(Vec<(CharacterId, Money)>, Money), VictoryError> {
    let members: Vec<_> = state.world().permanent_members(organization).collect();
    let leaders: Vec<_> = members
        .iter()
        .filter(|member| member.supervisor().is_none())
        .map(|member| member.id())
        .collect();
    if leaders.len() != 1 || severance <= Money::ZERO {
        return Err(VictoryError::RetirementNotReady);
    }
    let mut payees = Vec::with_capacity(members.len());
    for member in members {
        let amount = if member.id() == leaders[0] {
            RETIREMENT_NEST_EGG
        } else {
            severance
        };
        payees.push((member.id(), amount));
    }
    let required = RETIREMENT_NEST_EGG
        .checked_add(
            severance
                .checked_mul(i64::try_from(payees.len() - 1).map_err(|_| VictoryError::Arithmetic)?)
                .ok_or(VictoryError::Arithmetic)?,
        )
        .ok_or(VictoryError::Arithmetic)?;
    Ok((payees, required))
}

/// The terminal campaign retains its final roster. Verify the actual payment,
/// not just its label and the existence of one debit and one credit. Structural
/// validation checks the complete roster, uniform positive severance and the
/// fixed nest egg; registry-relative validation additionally checks the wage.
pub(crate) fn retirement_settlement_matches(
    state: &AppState,
    organization: OrganizationId,
    transaction: LedgerTransactionId,
    at: SimTime,
    registry: Option<&Registry>,
) -> bool {
    // Retirement terminates simulation: an outstanding paid term cannot be
    // left waiting for a future release that will never occur.
    if state
        .world()
        .characters_in_organization(organization)
        .any(|person| {
            state
                .world()
                .active_freelance_contract(person.id())
                .is_some()
        })
    {
        return false;
    }
    let Some(record) = state.finance().get_transaction(transaction) else {
        return false;
    };
    if record.occurred_at() != at
        || record.memo() != SETTLEMENT_MEMO
        || record.budget_usage().is_some()
    {
        return false;
    }
    let mut debited = 0_i128;
    let mut paid = BTreeMap::new();
    for posting in record.postings() {
        let Some(account) = state.finance().get_account(posting.account) else {
            return false;
        };
        if account.kind() != AccountKind::AccountedFunds {
            return false;
        }
        match account.owner() {
            FinancialOwner::Organization(owner)
                if owner == organization && posting.amount < Money::ZERO =>
            {
                debited -= i128::from(posting.amount.cents());
            }
            FinancialOwner::Character(person) if posting.amount > Money::ZERO => {
                if paid.insert(person, posting.amount).is_some() {
                    return false;
                }
            }
            FinancialOwner::Organization(_)
            | FinancialOwner::Character(_)
            | FinancialOwner::Business(_) => return false,
        }
    }
    let expected = if let Some(registry) = registry {
        retirement_members(registry, state, organization)
    } else {
        let Some(severance) = state
            .world()
            .permanent_members(organization)
            .filter(|member| member.supervisor().is_some())
            .find_map(|member| paid.get(&member.id()).copied())
        else {
            return false;
        };
        retirement_payees(state, organization, severance)
    };
    let Ok((payees, total)) = expected else {
        return false;
    };
    payees.len() >= RETIREMENT_MIN_KEY_PERSONNEL
        && paid.len() == payees.len()
        && debited == i128::from(total.cents())
        && payees
            .iter()
            .all(|(person, amount)| paid.get(person) == Some(amount))
}

pub(super) struct ValidatedRetirementSettlement {
    payees: Vec<(CharacterId, Money)>,
    transaction: ValidatedLedgerTransaction,
}

impl ValidatedRetirementSettlement {
    pub(super) fn id_budget(&self) -> Vec<(IdKind, u32)> {
        self.transaction.id_budget()
    }

    pub(super) fn commit_preflighted(self, state: &mut AppState) -> LedgerTransactionId {
        self.transaction.commit_preflighted(state)
    }

    pub(super) fn ensure_current(
        &self,
        registry: &Registry,
        state: &AppState,
        owner: OrganizationId,
    ) -> Result<(), VictoryError> {
        let (current, _) = retirement_members(registry, state, owner)?;
        if self.payees != current {
            return Err(VictoryError::Stale);
        }
        self.transaction.ensure_current(state)?;
        Ok(())
    }
}

pub(super) fn prepare_retirement_payment(
    registry: &Registry,
    state: &AppState,
    owner: OrganizationId,
) -> Result<ValidatedRetirementSettlement, VictoryError> {
    let (payees, required) = retirement_members(registry, state, owner)?;
    let mut postings = crate::finance::helpers::payment_debits(
        state
            .finance()
            .accounts_for(FinancialOwner::Organization(owner))
            .filter(|account| account.kind() == AccountKind::AccountedFunds),
        required,
    )
    .map_err(|_| VictoryError::RetirementNotReady)?;
    let mut missing = Vec::new();
    let mut destinations = Vec::new();
    for &(person, amount) in &payees {
        let destination = state
            .finance()
            .accounts_for(FinancialOwner::Character(person))
            .find(|account| {
                account.kind() == AccountKind::AccountedFunds
                    && account.can_receive_liquid_payment(amount)
            })
            .map(|account| account.id());
        if destination.is_none() {
            missing.push(FinancialAccountDraft {
                owner: FinancialOwner::Character(person),
                kind: AccountKind::AccountedFunds,
            });
        }
        destinations.push((destination, amount));
    }
    let openings = if missing.is_empty() {
        None
    } else {
        Some(validate_open_accounts(state, missing)?)
    };
    let mut opened = 0;
    for (destination, amount) in destinations {
        let id = match destination {
            Some(id) => id,
            None => {
                let id = openings
                    .as_ref()
                    .and_then(|plan| plan.account_id(opened))
                    .ok_or(VictoryError::Arithmetic)?;
                opened += 1;
                id
            }
        };
        postings.push(LedgerPosting {
            account: id,
            amount,
        });
    }
    let draft = LedgerTransactionDraft {
        occurred_at: state.now(),
        memo: SETTLEMENT_MEMO.to_owned(),
        postings,
        authorization: None,
    };
    let transaction = match openings {
        Some(plan) => validate_record_transaction_with_openings(state, plan, draft)?,
        None => validate_record_transaction(state, draft)?,
    };
    Ok(ValidatedRetirementSettlement {
        payees,
        transaction,
    })
}
